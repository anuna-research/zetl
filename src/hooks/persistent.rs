//! Persistent-mode wire protocol for render-pipeline hooks
//! (SPEC-032 CON-3201 / NFR-3207 / REQ-3201).
//!
//! Persistent-mode hooks are long-running child processes that zetl reuses
//! across every page rather than re-spawning per invocation. Communication
//! is line-delimited JSON on stdin/stdout. Stderr is free-form and captured
//! into a diagnostic buffer for surfacing on verbose / failure paths.
//!
//! ## Lifecycle
//!
//! ```text
//!   spawn → handshake → init → run (N pages) → finalise → shutdown
//! ```
//!
//! 1. **Handshake** — zetl reads one line from stdout:
//!    `{"zetl_ast": 1, "hook": "callouts", "version": "1.0.3", "ready": true}`
//!    Incompatible `zetl_ast` major → [`ProtocolError::Handshake`]; the
//!    caller disables the hook and logs `ast_version_mismatch`.
//! 2. **Init** — zetl writes one line: `{"type":"init", ...}`. Hook
//!    responds with a `result`/`error` line. Per-build context
//!    (REQ-3220) is embedded here.
//! 3. **Run** — per page, zetl writes `{"type":"run", ...}` and reads a
//!    single response line. Enforces `deadline_ms` — on timeout the
//!    child is hard-killed and [`ProtocolError::Timeout`] is returned.
//! 4. **Finalise** — at end of build, zetl writes `{"type":"finalise"}`
//!    so the hook can emit final `build_data` / diagnostics.
//! 5. **Shutdown** — zetl writes `{"type":"shutdown"}` and closes stdin.
//!    Hook SHOULD exit within [`DEFAULT_SHUTDOWN_GRACE`]; hard-killed
//!    after. [`PersistentHook::drop`] guarantees zero orphans.
//!
//! The [`ProtocolError`] type distinguishes transport errors (broken
//! pipe, protocol timeout) from hook-level errors (typed `{"type":"error"}`
//! responses) so the caller's REQ-3207 recovery policy can branch on the
//! cause.
//!
//! ## Concurrency
//!
//! Each [`PersistentHook`] owns a child plus two pump threads:
//! - a stdout reader thread that feeds parsed lines into a sync channel,
//!   letting the main thread wait with a bounded timeout instead of
//!   blocking indefinitely on a BufReader;
//! - a stderr drain thread that appends bytes into a shared buffer so the
//!   caller can surface them at any point without deadlocking the child
//!   on a full stderr pipe.
//!
//! The channel / buffer are [`Arc`]-owned; both threads exit cleanly when
//! the child's pipes close. Joins are performed in [`Drop`].
//!
//! ## Budget
//!
//! NFR-3207 caps the zetl-side protocol overhead (serialise → write →
//! read → deserialise) at 10 ms P95 per page; this task's internal target
//! is < 1.5 ms for typical payloads (≤ 500 AST nodes). The implementation
//! avoids per-call thread spawns and uses [`BufWriter`] + [`BufReader`]
//! to minimise syscalls. Verified by `protocol_overhead_per_run_is_fast`
//! in the integration suite.

use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::hooks::ast::AST_VERSION;
use crate::hooks::pipeline::Stage;

/// AST major version zetl currently speaks. Must match the leading digit
/// of [`AST_VERSION`].
pub const ZETL_AST_MAJOR: u32 = 1;

/// How long zetl waits for the hook's startup handshake line. Beyond this
/// the hook is considered dead and the child is killed.
pub const DEFAULT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

/// Grace period between `shutdown` + stdin-close and the hard-kill of
/// the child (CON-3201: "Hook SHOULD exit cleanly within 1 s.").
pub const DEFAULT_SHUTDOWN_GRACE: Duration = Duration::from_secs(1);

/// Fallback per-call deadline used when the caller doesn't override via
/// the `deadline_ms` field in a [`HostMessage::Run`].
pub const DEFAULT_DEADLINE_MS: u64 = 100;

/// First line written by the hook on process start (CON-3201).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookHandshake {
    /// AST major version the hook targets. If this disagrees with
    /// [`ZETL_AST_MAJOR`], zetl disables the hook (REQ-3215).
    pub zetl_ast: u32,
    /// Hook id (usually matches the manifest `extension_id`).
    pub hook: String,
    /// Hook version string — free-form, used for `OBS-3201` logs and
    /// the `zetl hook coverage` report.
    pub version: String,
    /// Must be `true` for zetl to proceed past the handshake.
    pub ready: bool,
}

/// Request zetl sends to a hook. Self-describing via the `type` tag so
/// JSON-lines parsing doesn't need per-call state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum HostMessage {
    /// One-time setup. Sent immediately after a successful handshake.
    Init(InitMessage),
    /// Invoke the hook against a single page.
    Run(RunMessage),
    /// End-of-build signal — hook may emit final `build_data` /
    /// diagnostics via a `result` response.
    Finalise(FinaliseMessage),
    /// Graceful teardown. Zetl closes stdin immediately after sending.
    Shutdown(ShutdownMessage),
}

/// Per-build context embedded in the init payload (REQ-3220).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitMessage {
    /// Stage this hook is wired to (`"pre-parse"` / `"transform"` /
    /// `"post-render"`).
    pub stage: String,
    /// zetl binary version string (REQ-3215 / REQ-3220 `ZETL_VERSION`).
    pub zetl_version: String,
    /// AST schema version zetl is emitting (REQ-3215 — carries the minor
    /// so hooks declaring a semver range can warn on mismatch).
    pub ast_schema_version: String,
    /// Full REQ-3220 ctx payload. Shape owned by
    /// [`crate::hooks::build_context::BuildContext`].
    #[serde(default)]
    pub ctx: Value,
}

/// Per-page invocation. `payload` shape depends on the stage (Markdown
/// string for `pre-parse`, AST JSON for `transform`, HTML string for
/// `post-render`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunMessage {
    pub page_slug: String,
    #[serde(default)]
    pub frontmatter: Value,
    pub payload: Value,
    /// Advisory deadline — the hook should return within this many
    /// milliseconds; zetl enforces a hard kill on its side if not.
    pub deadline_ms: u64,
}

/// End-of-build prompt. Currently carries no fields but reserves the
/// shape for additive evolution (CON-3201 + NFR-3206).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FinaliseMessage {}

/// Teardown prompt. Separate type so NFR-3206's additive-only rule has
/// a named landing spot for future fields.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ShutdownMessage {}

/// Reply zetl reads from the hook after a [`HostMessage`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum HookMessage {
    /// Successful result of a Run / Init / Finalise exchange.
    Result {
        #[serde(default)]
        payload: Value,
        #[serde(default)]
        diagnostics: Vec<Diagnostic>,
        #[serde(default)]
        template_vars: Value,
        #[serde(default)]
        build_data: Value,
    },
    /// Typed failure. The caller's REQ-3207 policy decides whether to
    /// revert the page, continue, or abort.
    Error {
        reason: String,
        #[serde(default)]
        detail: String,
    },
}

/// A single structured diagnostic emitted by the hook. Surfaced via
/// [`ProtocolError::HookError`] for errors and via the returned
/// [`HookMessage::Result`] for advisories.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub severity: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_slug: Option<String>,
}

/// Errors from the persistent-protocol layer.
#[derive(Debug)]
pub enum ProtocolError {
    Io(std::io::Error),
    Json(serde_json::Error),
    /// The hook closed stdout before responding — usually means it
    /// crashed; check [`PersistentHook::drain_stderr`].
    UnexpectedEof,
    /// The hook did not respond within the deadline. The child has
    /// been hard-killed; the instance is no longer usable.
    Timeout { deadline: Duration },
    /// Handshake was malformed or declared an incompatible AST major.
    Handshake(String),
    /// Hook returned a typed `{"type":"error"}` message.
    HookError { reason: String, detail: String },
}

impl From<std::io::Error> for ProtocolError {
    fn from(e: std::io::Error) -> Self {
        ProtocolError::Io(e)
    }
}

impl From<serde_json::Error> for ProtocolError {
    fn from(e: serde_json::Error) -> Self {
        ProtocolError::Json(e)
    }
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtocolError::Io(e) => write!(f, "persistent hook io error: {e}"),
            ProtocolError::Json(e) => write!(f, "persistent hook json error: {e}"),
            ProtocolError::UnexpectedEof => f.write_str("persistent hook closed stdout unexpectedly"),
            ProtocolError::Timeout { deadline } => {
                write!(f, "persistent hook exceeded deadline of {:?}", deadline)
            }
            ProtocolError::Handshake(msg) => write!(f, "persistent hook handshake failed: {msg}"),
            ProtocolError::HookError { reason, detail } => {
                if detail.is_empty() {
                    write!(f, "hook reported error: {reason}")
                } else {
                    write!(f, "hook reported error: {reason} ({detail})")
                }
            }
        }
    }
}

impl std::error::Error for ProtocolError {}

/// A long-running persistent-mode hook process with owned stdio pumps.
///
/// Construct via [`PersistentHook::spawn`] (reads the handshake before
/// returning). Issue per-page requests via [`Self::run`]; call
/// [`Self::shutdown`] when done. [`Drop`] also performs shutdown so a
/// panicking caller never leaks a child process.
pub struct PersistentHook {
    child: Child,
    stdin: Option<BufWriter<ChildStdin>>,
    stdout_rx: Receiver<std::io::Result<String>>,
    stderr: Arc<Mutex<Vec<u8>>>,
    reader_handle: Option<JoinHandle<()>>,
    stderr_handle: Option<JoinHandle<()>>,
    handshake: HookHandshake,
    hook_id: String,
    stage: Stage,
    shutdown_grace: Duration,
    /// `true` after the first fatal protocol error. All subsequent
    /// operations short-circuit with [`ProtocolError::UnexpectedEof`].
    dead: bool,
}

impl PersistentHook {
    /// Spawn the hook process and wait for its startup handshake.
    ///
    /// `command` is expected to be a fully configured [`Command`] (path,
    /// args, environment). Stdio piping is applied automatically so
    /// callers must not pre-configure `stdin` / `stdout` / `stderr`.
    pub fn spawn(
        command: Command,
        hook_id: impl Into<String>,
        stage: Stage,
    ) -> Result<Self, ProtocolError> {
        Self::spawn_with_config(
            command,
            hook_id,
            stage,
            DEFAULT_HANDSHAKE_TIMEOUT,
            DEFAULT_SHUTDOWN_GRACE,
        )
    }

    /// Spawn with explicit handshake / shutdown timeouts — primarily
    /// exposed for tests that need to exercise the timeout paths in
    /// fractions of a second.
    pub fn spawn_with_config(
        mut command: Command,
        hook_id: impl Into<String>,
        stage: Stage,
        handshake_timeout: Duration,
        shutdown_grace: Duration,
    ) -> Result<Self, ProtocolError> {
        let hook_id = hook_id.into();

        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdin = BufWriter::new(child.stdin.take().expect("piped stdin"));
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");

        let (tx, stdout_rx) = mpsc::sync_channel::<std::io::Result<String>>(32);
        let reader_handle = thread::Builder::new()
            .name(format!("zetl-hook-{hook_id}-stdout"))
            .spawn(move || pump_stdout(stdout, tx))
            .expect("spawn stdout pump");

        let stderr_buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let stderr_buf2 = Arc::clone(&stderr_buf);
        let stderr_handle = thread::Builder::new()
            .name(format!("zetl-hook-{hook_id}-stderr"))
            .spawn(move || pump_stderr(stderr, stderr_buf2))
            .expect("spawn stderr pump");

        let handshake = match stdout_rx.recv_timeout(handshake_timeout) {
            Ok(Ok(line)) => match serde_json::from_str::<HookHandshake>(&line) {
                Ok(h) => h,
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(ProtocolError::Handshake(format!(
                        "malformed handshake line '{line}': {e}"
                    )));
                }
            },
            Ok(Err(e)) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ProtocolError::Io(e));
            }
            Err(RecvTimeoutError::Timeout) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ProtocolError::Timeout {
                    deadline: handshake_timeout,
                });
            }
            Err(RecvTimeoutError::Disconnected) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ProtocolError::UnexpectedEof);
            }
        };

        if handshake.zetl_ast != ZETL_AST_MAJOR {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ProtocolError::Handshake(format!(
                "ast_version_mismatch: hook declared zetl_ast={}, zetl speaks {}",
                handshake.zetl_ast, ZETL_AST_MAJOR
            )));
        }
        if !handshake.ready {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ProtocolError::Handshake(
                "hook reported ready=false".into(),
            ));
        }

        Ok(Self {
            child,
            stdin: Some(stdin),
            stdout_rx,
            stderr: stderr_buf,
            reader_handle: Some(reader_handle),
            stderr_handle: Some(stderr_handle),
            handshake,
            hook_id,
            stage,
            shutdown_grace,
            dead: false,
        })
    }

    /// Handshake the hook emitted on startup.
    pub fn handshake(&self) -> &HookHandshake {
        &self.handshake
    }

    /// Stable hook id (manifest `extension_id`).
    pub fn hook_id(&self) -> &str {
        &self.hook_id
    }

    /// Stage this hook was registered against.
    pub fn stage(&self) -> Stage {
        self.stage
    }

    /// Whether the instance has been killed by a prior fatal error or
    /// shutdown. Dead instances always return [`ProtocolError::UnexpectedEof`]
    /// or similar from subsequent operations.
    pub fn is_dead(&self) -> bool {
        self.dead
    }

    /// Send an `init` message with per-build context and await the
    /// hook's `result` line.
    pub fn init(
        &mut self,
        ctx: Value,
        deadline_ms: u64,
    ) -> Result<HookMessage, ProtocolError> {
        let msg = HostMessage::Init(InitMessage {
            stage: self.stage.as_str().into(),
            zetl_version: env!("CARGO_PKG_VERSION").to_string(),
            ast_schema_version: AST_VERSION.to_string(),
            ctx,
        });
        self.exchange(&msg, Duration::from_millis(deadline_ms))
    }

    /// Send a `run` message for a single page and await the result.
    ///
    /// `deadline_ms` is both advertised to the hook (inside the request)
    /// and enforced by zetl — on timeout the child is killed and the
    /// instance is marked dead.
    pub fn run(
        &mut self,
        page_slug: impl Into<String>,
        frontmatter: Value,
        payload: Value,
        deadline_ms: u64,
    ) -> Result<HookMessage, ProtocolError> {
        let msg = HostMessage::Run(RunMessage {
            page_slug: page_slug.into(),
            frontmatter,
            payload,
            deadline_ms,
        });
        self.exchange(&msg, Duration::from_millis(deadline_ms))
    }

    /// Send a `finalise` message and await the hook's trailing result.
    pub fn finalise(&mut self, deadline_ms: u64) -> Result<HookMessage, ProtocolError> {
        let msg = HostMessage::Finalise(FinaliseMessage::default());
        self.exchange(&msg, Duration::from_millis(deadline_ms))
    }

    /// Drain any captured stderr bytes as a UTF-8 string. Returns the
    /// empty string if the stderr buffer is empty. Each call empties
    /// the buffer.
    pub fn drain_stderr(&self) -> String {
        let mut buf = self.stderr.lock().unwrap();
        let out = String::from_utf8_lossy(&buf).into_owned();
        buf.clear();
        out
    }

    /// Peek at captured stderr bytes without draining.
    pub fn peek_stderr(&self) -> String {
        let buf = self.stderr.lock().unwrap();
        String::from_utf8_lossy(&buf).into_owned()
    }

    /// Gracefully shut down the hook: send `{"type":"shutdown"}`, close
    /// stdin, and wait up to [`Self::shutdown_grace`] for the child to
    /// exit. Hard-kills after the grace period. Idempotent.
    pub fn shutdown(&mut self) -> Result<(), ProtocolError> {
        if self.dead {
            // Already dead — just make sure the child has been reaped.
            let _ = self.child.wait();
            self.stdin = None;
            return Ok(());
        }
        self.dead = true;

        // Best-effort: tell the hook we're done. Ignore IO errors —
        // the hook may have already exited, in which case the EPIPE is
        // fine and the subsequent wait will reap it.
        let _ = self.send(&HostMessage::Shutdown(ShutdownMessage::default()));

        // Close stdin by dropping the writer. This is the canonical
        // end-of-input signal; combined with the explicit shutdown
        // message it gives well-behaved hooks two independent cues.
        self.stdin = None;

        let start = Instant::now();
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => return Ok(()),
                Ok(None) => {
                    if start.elapsed() >= self.shutdown_grace {
                        break;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(e) => return Err(ProtocolError::Io(e)),
            }
        }

        // Grace exceeded — hard-kill.
        let _ = self.child.kill();
        let _ = self.child.wait();
        Ok(())
    }

    /// Wall-clock duration between sending `msg` and parsing the first
    /// response line. Exposed so callers can attribute per-hook time to
    /// the build stats block (SPEC-032 §9 OBS-3201).
    pub fn exchange_with_timing(
        &mut self,
        msg: &HostMessage,
        deadline: Duration,
    ) -> Result<(HookMessage, Duration), ProtocolError> {
        let start = Instant::now();
        let result = self.exchange(msg, deadline)?;
        Ok((result, start.elapsed()))
    }

    fn exchange(
        &mut self,
        msg: &HostMessage,
        deadline: Duration,
    ) -> Result<HookMessage, ProtocolError> {
        if self.dead {
            return Err(ProtocolError::UnexpectedEof);
        }
        if let Err(e) = self.send(msg) {
            self.dead = true;
            return Err(e);
        }
        match self.recv_response(deadline) {
            Ok(HookMessage::Error { reason, detail }) => {
                // Typed errors do not kill the hook — the caller decides
                // whether to continue. REQ-3207 owns the recovery policy.
                Err(ProtocolError::HookError { reason, detail })
            }
            Ok(other) => Ok(other),
            Err(e) => {
                // Any non-typed error — timeout, broken pipe, malformed
                // line — renders the instance unusable. Kill + mark dead.
                self.dead = true;
                let _ = self.child.kill();
                let _ = self.child.wait();
                Err(e)
            }
        }
    }

    fn send(&mut self, msg: &HostMessage) -> Result<(), ProtocolError> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or(ProtocolError::UnexpectedEof)?;
        let line = serde_json::to_string(msg)?;
        stdin.write_all(line.as_bytes())?;
        stdin.write_all(b"\n")?;
        stdin.flush()?;
        Ok(())
    }

    fn recv_response(&self, deadline: Duration) -> Result<HookMessage, ProtocolError> {
        match self.stdout_rx.recv_timeout(deadline) {
            Ok(Ok(line)) => Ok(serde_json::from_str(&line)?),
            Ok(Err(e)) => Err(ProtocolError::Io(e)),
            Err(RecvTimeoutError::Timeout) => Err(ProtocolError::Timeout { deadline }),
            Err(RecvTimeoutError::Disconnected) => Err(ProtocolError::UnexpectedEof),
        }
    }
}

impl std::fmt::Debug for PersistentHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PersistentHook")
            .field("hook_id", &self.hook_id)
            .field("stage", &self.stage)
            .field("handshake", &self.handshake)
            .field("dead", &self.dead)
            .field("shutdown_grace", &self.shutdown_grace)
            .finish_non_exhaustive()
    }
}

impl Drop for PersistentHook {
    fn drop(&mut self) {
        if !self.dead {
            let _ = self.shutdown();
        }
        // Extra safety: even if shutdown ran the path below may still
        // find a child that hasn't been reaped yet on some kernels.
        let _ = self.child.try_wait();
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(h) = self.reader_handle.take() {
            let _ = h.join();
        }
        if let Some(h) = self.stderr_handle.take() {
            let _ = h.join();
        }
    }
}

fn pump_stdout(stdout: ChildStdout, tx: mpsc::SyncSender<std::io::Result<String>>) {
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        if tx.send(line).is_err() {
            break;
        }
    }
}

fn pump_stderr(mut stderr: std::process::ChildStderr, buf: Arc<Mutex<Vec<u8>>>) {
    let mut chunk = [0u8; 4096];
    loop {
        match stderr.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => buf.lock().unwrap().extend_from_slice(&chunk[..n]),
            Err(_) => break,
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use serde_json::json;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Write a python fixture hook into `dir/<name>` with executable
    /// permissions. Panics if python3 isn't discoverable — callers
    /// should gate on [`python3_available`] first.
    fn write_python_hook(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        let script = format!("#!/usr/bin/env python3\n{body}");
        std::fs::write(&path, script).unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
        path
    }

    fn python3_available() -> bool {
        Command::new("python3")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Skip-macro for tests that require python3.
    macro_rules! require_python {
        () => {
            if !python3_available() {
                eprintln!("[persistent_protocol] skipping: python3 not available");
                return;
            }
        };
    }

    /// Standard echo hook body: handshake, then for each request echo
    /// the payload back in a `result`. Responds to shutdown by exiting.
    const ECHO_BODY: &str = r#"
import json, sys
sys.stdout.write('{"zetl_ast":1,"hook":"echo","version":"0.1.0","ready":true}\n')
sys.stdout.flush()
for line in sys.stdin:
    try:
        msg = json.loads(line)
    except Exception:
        continue
    t = msg.get("type")
    if t == "shutdown":
        break
    payload = msg.get("payload", {})
    resp = {"type":"result", "payload": payload, "diagnostics": [], "template_vars": {}}
    sys.stdout.write(json.dumps(resp) + "\n")
    sys.stdout.flush()
"#;

    #[test]
    fn handshake_parses_and_is_retained() {
        require_python!();
        let tmp = TempDir::new().unwrap();
        let hook = write_python_hook(tmp.path(), "echo.py", ECHO_BODY);

        let cmd = Command::new(&hook);
        let h = PersistentHook::spawn(cmd, "echo", Stage::Transform).unwrap();
        assert_eq!(h.handshake().zetl_ast, 1);
        assert_eq!(h.handshake().hook, "echo");
        assert_eq!(h.handshake().version, "0.1.0");
        assert!(h.handshake().ready);
        assert_eq!(h.hook_id(), "echo");
        assert_eq!(h.stage(), Stage::Transform);
    }

    #[test]
    fn init_run_finalise_round_trip() {
        require_python!();
        let tmp = TempDir::new().unwrap();
        let hook = write_python_hook(tmp.path(), "echo.py", ECHO_BODY);

        let mut h = PersistentHook::spawn(Command::new(&hook), "echo", Stage::Transform).unwrap();

        // init
        let init = h.init(json!({"theme":"default"}), 1000).unwrap();
        match init {
            HookMessage::Result { .. } => (),
            other => panic!("unexpected init response: {other:?}"),
        }

        // run
        let run = h
            .run("page-a", json!({}), json!({"body":"hello"}), 1000)
            .unwrap();
        match run {
            HookMessage::Result { payload, .. } => {
                assert_eq!(payload, json!({"body":"hello"}));
            }
            other => panic!("unexpected run response: {other:?}"),
        }

        // finalise
        let _ = h.finalise(1000).unwrap();
        assert!(!h.is_dead());
    }

    #[test]
    fn shutdown_exits_within_grace() {
        require_python!();
        let tmp = TempDir::new().unwrap();
        let hook = write_python_hook(tmp.path(), "echo.py", ECHO_BODY);

        let mut h = PersistentHook::spawn(Command::new(&hook), "echo", Stage::Transform).unwrap();
        let start = Instant::now();
        h.shutdown().unwrap();
        let elapsed = start.elapsed();

        // Grace is 1 s; a well-behaved echo hook should exit on stdin-close
        // in milliseconds.
        assert!(
            elapsed < Duration::from_millis(500),
            "shutdown took {elapsed:?}"
        );
        assert!(h.is_dead());

        // Second shutdown is a no-op.
        h.shutdown().unwrap();
    }

    #[test]
    fn handshake_timeout_kills_child() {
        require_python!();
        let tmp = TempDir::new().unwrap();
        // A hook that never emits a handshake line.
        let hook = write_python_hook(
            tmp.path(),
            "silent.py",
            r#"
import time
while True:
    time.sleep(1)
"#,
        );

        let start = Instant::now();
        let err = PersistentHook::spawn_with_config(
            Command::new(&hook),
            "silent",
            Stage::Transform,
            Duration::from_millis(300),
            DEFAULT_SHUTDOWN_GRACE,
        )
        .unwrap_err();
        let elapsed = start.elapsed();

        match err {
            ProtocolError::Timeout { deadline } => {
                assert_eq!(deadline, Duration::from_millis(300));
            }
            other => panic!("expected Timeout, got {other:?}"),
        }
        // Should not block much beyond the handshake timeout.
        assert!(elapsed < Duration::from_millis(1500), "elapsed {elapsed:?}");
    }

    #[test]
    fn ast_version_mismatch_is_rejected() {
        require_python!();
        let tmp = TempDir::new().unwrap();
        let hook = write_python_hook(
            tmp.path(),
            "v2.py",
            r#"
import sys
sys.stdout.write('{"zetl_ast":2,"hook":"future","version":"1.0","ready":true}\n')
sys.stdout.flush()
import time
time.sleep(5)
"#,
        );

        let err =
            PersistentHook::spawn(Command::new(&hook), "future", Stage::Transform).unwrap_err();
        match err {
            ProtocolError::Handshake(msg) => {
                assert!(msg.contains("ast_version_mismatch"), "msg: {msg}");
            }
            other => panic!("expected Handshake error, got {other:?}"),
        }
    }

    #[test]
    fn ready_false_is_rejected() {
        require_python!();
        let tmp = TempDir::new().unwrap();
        let hook = write_python_hook(
            tmp.path(),
            "notready.py",
            r#"
import sys
sys.stdout.write('{"zetl_ast":1,"hook":"x","version":"0","ready":false}\n')
sys.stdout.flush()
import time
time.sleep(5)
"#,
        );

        let err = PersistentHook::spawn(Command::new(&hook), "x", Stage::Transform).unwrap_err();
        match err {
            ProtocolError::Handshake(msg) => {
                assert!(msg.contains("ready=false"), "msg: {msg}");
            }
            other => panic!("expected Handshake error, got {other:?}"),
        }
    }

    #[test]
    fn run_timeout_kills_child_and_marks_dead() {
        require_python!();
        let tmp = TempDir::new().unwrap();
        // Handshake OK, then hangs forever on first input.
        let hook = write_python_hook(
            tmp.path(),
            "hang.py",
            r#"
import json, sys, time
sys.stdout.write('{"zetl_ast":1,"hook":"hang","version":"0","ready":true}\n')
sys.stdout.flush()
for line in sys.stdin:
    time.sleep(60)
"#,
        );

        let mut h =
            PersistentHook::spawn(Command::new(&hook), "hang", Stage::Transform).unwrap();

        let start = Instant::now();
        let err = h
            .run("slug", json!({}), json!({"x":1}), 200)
            .unwrap_err();
        let elapsed = start.elapsed();

        match err {
            ProtocolError::Timeout { deadline } => {
                assert_eq!(deadline, Duration::from_millis(200));
            }
            other => panic!("expected Timeout, got {other:?}"),
        }
        assert!(h.is_dead());
        assert!(elapsed < Duration::from_millis(1500), "elapsed {elapsed:?}");

        // Subsequent calls return UnexpectedEof without hanging.
        let err2 = h.run("slug2", json!({}), json!({}), 200).unwrap_err();
        matches!(err2, ProtocolError::UnexpectedEof);
    }

    #[test]
    fn typed_error_response_is_surfaced() {
        require_python!();
        let tmp = TempDir::new().unwrap();
        let hook = write_python_hook(
            tmp.path(),
            "err.py",
            r#"
import json, sys
sys.stdout.write('{"zetl_ast":1,"hook":"err","version":"0","ready":true}\n')
sys.stdout.flush()
for line in sys.stdin:
    msg = json.loads(line)
    if msg.get("type") == "shutdown":
        break
    sys.stdout.write(json.dumps({"type":"error","reason":"nope","detail":"because"}) + "\n")
    sys.stdout.flush()
"#,
        );

        let mut h = PersistentHook::spawn(Command::new(&hook), "err", Stage::Transform).unwrap();
        let err = h.run("slug", json!({}), json!({}), 1000).unwrap_err();
        match err {
            ProtocolError::HookError { reason, detail } => {
                assert_eq!(reason, "nope");
                assert_eq!(detail, "because");
            }
            other => panic!("expected HookError, got {other:?}"),
        }
        // Typed errors don't kill the instance — the REQ-3207 policy
        // owner decides whether to continue with this hook or drop it.
        assert!(!h.is_dead());
    }

    #[test]
    fn stderr_is_captured_and_drained() {
        require_python!();
        let tmp = TempDir::new().unwrap();
        let hook = write_python_hook(
            tmp.path(),
            "stderr.py",
            r#"
import json, sys
sys.stdout.write('{"zetl_ast":1,"hook":"stderr","version":"0","ready":true}\n')
sys.stdout.flush()
for line in sys.stdin:
    msg = json.loads(line)
    if msg.get("type") == "shutdown":
        break
    print("diagnostic line", file=sys.stderr, flush=True)
    sys.stdout.write(json.dumps({"type":"result","payload":{}}) + "\n")
    sys.stdout.flush()
"#,
        );

        let mut h =
            PersistentHook::spawn(Command::new(&hook), "stderr", Stage::Transform).unwrap();
        let _ = h.run("p", json!({}), json!({}), 1000).unwrap();
        // Give stderr pump a moment.
        thread::sleep(Duration::from_millis(50));
        let captured = h.drain_stderr();
        assert!(captured.contains("diagnostic line"), "captured: {captured:?}");
        // Subsequent drain is empty.
        assert_eq!(h.drain_stderr(), "");
    }

    #[test]
    fn drop_does_not_leak_children() {
        require_python!();
        let tmp = TempDir::new().unwrap();
        // A hook that ignores stdin close and keeps running. Drop must
        // still kill it.
        let hook = write_python_hook(
            tmp.path(),
            "ignore_close.py",
            r#"
import sys, signal, time
sys.stdout.write('{"zetl_ast":1,"hook":"ignore","version":"0","ready":true}\n')
sys.stdout.flush()
# Ignore EOF by spinning on a no-op loop.
signal.signal(signal.SIGPIPE, signal.SIG_IGN)
while True:
    time.sleep(1)
"#,
        );

        let pid = {
            let h = PersistentHook::spawn_with_config(
                Command::new(&hook),
                "ignore",
                Stage::Transform,
                DEFAULT_HANDSHAKE_TIMEOUT,
                Duration::from_millis(100), // short grace so drop completes quickly
            )
            .unwrap();
            h.child.id()
        };

        // After drop, the child should no longer exist. `kill -0 <pid>`
        // returns success iff the process is alive.
        // Give the kernel a moment to reap.
        thread::sleep(Duration::from_millis(200));
        let alive = Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        assert!(!alive, "child pid {pid} still alive after Drop");
    }

    #[test]
    fn exchange_with_timing_records_wall_clock() {
        require_python!();
        let tmp = TempDir::new().unwrap();
        let hook = write_python_hook(tmp.path(), "echo.py", ECHO_BODY);
        let mut h =
            PersistentHook::spawn(Command::new(&hook), "echo", Stage::Transform).unwrap();

        let msg = HostMessage::Run(RunMessage {
            page_slug: "t".into(),
            frontmatter: json!({}),
            payload: json!({"k":"v"}),
            deadline_ms: 1000,
        });
        let (_resp, dur) = h.exchange_with_timing(&msg, Duration::from_secs(1)).unwrap();
        // Just assert it was recorded — absolute floors vary wildly across
        // hosts. Upper bound of 500 ms is generous for a single Python
        // round-trip on any sane CI worker.
        assert!(dur < Duration::from_millis(500), "dur {dur:?}");
    }

    #[test]
    fn handshake_major_constant_matches_ast_version() {
        let major: u32 = AST_VERSION
            .split('.')
            .next()
            .and_then(|s| s.parse().ok())
            .expect("AST_VERSION has a major component");
        assert_eq!(major, ZETL_AST_MAJOR);
    }

    #[test]
    fn host_message_json_is_tagged_lowercase() {
        // Guard against accidental rename on the wire — every variant's
        // discriminator is the lowercase of the ident, which is what
        // REQ-3220 hooks match on.
        let run = HostMessage::Run(RunMessage {
            page_slug: "x".into(),
            frontmatter: json!({}),
            payload: json!({}),
            deadline_ms: 50,
        });
        let wire = serde_json::to_value(&run).unwrap();
        assert_eq!(wire.get("type").and_then(Value::as_str), Some("run"));
        assert_eq!(wire.get("deadline_ms").and_then(Value::as_u64), Some(50));

        for (name, msg) in [
            ("init", HostMessage::Init(InitMessage {
                stage: Stage::Transform.as_str().into(),
                zetl_version: "0.5.0".into(),
                ast_schema_version: AST_VERSION.into(),
                ctx: json!({}),
            })),
            ("finalise", HostMessage::Finalise(FinaliseMessage::default())),
            ("shutdown", HostMessage::Shutdown(ShutdownMessage::default())),
        ] {
            let v = serde_json::to_value(&msg).unwrap();
            assert_eq!(v.get("type").and_then(Value::as_str), Some(name));
        }
    }

    #[test]
    fn hook_message_parses_result_and_error() {
        let r: HookMessage = serde_json::from_str(
            r#"{"type":"result","payload":{"html":"<p>x</p>"}}"#,
        )
        .unwrap();
        match r {
            HookMessage::Result { payload, diagnostics, .. } => {
                assert_eq!(payload, json!({"html":"<p>x</p>"}));
                assert!(diagnostics.is_empty());
            }
            _ => panic!("wrong variant"),
        }

        let e: HookMessage = serde_json::from_str(
            r#"{"type":"error","reason":"r","detail":"d"}"#,
        )
        .unwrap();
        match e {
            HookMessage::Error { reason, detail } => {
                assert_eq!(reason, "r");
                assert_eq!(detail, "d");
            }
            _ => panic!("wrong variant"),
        }
    }
}
