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

use std::ffi::OsString;
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

/// Per-message wire-size cap (SPEC-032 §10): protects against memory
/// pressure from a runaway hook emitting an unbounded JSON line and
/// against deeply-nested untrusted JSON deserialisation. Applies to
/// host → hook writes *and* hook → host reads.
pub const DEFAULT_MAX_MESSAGE_BYTES: usize = 10 * 1024 * 1024;

/// Captured-stderr cap. Older bytes are kept (handshake errors are at
/// the start of stderr); once the cap is hit a `[stderr truncated]`
/// marker is appended once and further bytes are dropped on the floor
/// rather than allocating without bound (SPEC-032 §10 streaming guidance).
pub const DEFAULT_MAX_STDERR_BYTES: usize = 1024 * 1024;

/// Marker appended to captured stderr when the [`DEFAULT_MAX_STDERR_BYTES`]
/// cap has been reached and further data was discarded.
pub const STDERR_TRUNCATED_MARKER: &[u8] = b"\n[stderr truncated]\n";

/// Default allow-list of parent-process environment variables forwarded
/// to a hook child after the parent's full environment is cleared
/// (SPEC-032 §10 redact-env-by-default). Anything outside this list
/// (and outside the explicit `cmd.env(...)` calls the spawning code
/// makes for `ZETL_*`) is invisible to the hook.
///
/// The list is deliberately conservative: enough for an interpreter
/// shebang to find `python3` / `node` (`PATH`), for Python's user
/// site-packages discovery to work (`HOME`), and for text encoding to
/// be predictable (`LANG`, `LC_*`). It does *not* include shell
/// secret-bearing variables (`AWS_*`, `*_TOKEN`, `OPENAI_API_KEY`,
/// `GITHUB_TOKEN`, etc.) — those leak only when a vault author
/// explicitly opts in via [`SecurityPolicy::env_allowlist`].
pub const DEFAULT_ENV_ALLOWLIST: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "LC_MESSAGES",
    "TZ",
    "TMPDIR",
    "TEMP",
    "TMP",
    "TERM",
    "SHELL",
];

/// Security policy applied to a [`PersistentHook`] child at spawn time
/// (SPEC-032 §10).
///
/// The defaults implement the spec's "untrusted code" baseline:
/// - parent environment is redacted; only [`DEFAULT_ENV_ALLOWLIST`]
///   variables (plus anything the spawn caller set explicitly via
///   `cmd.env(...)`) reach the child;
/// - stderr is captured into a [`DEFAULT_MAX_STDERR_BYTES`]-bounded
///   ring, so a chatty hook can't OOM the host;
/// - per-message I/O is capped at [`DEFAULT_MAX_MESSAGE_BYTES`] both
///   directions, so a malicious or buggy hook can't blow up host
///   memory by emitting a single unbounded line.
///
/// Customise via [`Self::with_env_allowlist`] / [`Self::with_extra_env`]
/// when a hook genuinely needs an additional variable (e.g. a Python
/// hook that reads `VIRTUAL_ENV`).
#[derive(Debug, Clone)]
pub struct SecurityPolicy {
    /// Names of parent-process env vars to forward to the child.
    /// Variables not present in the parent environment are silently
    /// skipped — a missing `PATH` is not an error.
    pub env_allowlist: Vec<String>,
    /// Maximum captured stderr bytes — see [`DEFAULT_MAX_STDERR_BYTES`].
    pub max_stderr_bytes: usize,
    /// Maximum bytes per line read from / written to the hook. A line
    /// longer than this returns [`ProtocolError::MessageTooLarge`] and
    /// kills the hook.
    pub max_message_bytes: usize,
}

impl Default for SecurityPolicy {
    fn default() -> Self {
        Self {
            env_allowlist: DEFAULT_ENV_ALLOWLIST.iter().map(|s| (*s).to_string()).collect(),
            max_stderr_bytes: DEFAULT_MAX_STDERR_BYTES,
            max_message_bytes: DEFAULT_MAX_MESSAGE_BYTES,
        }
    }
}

impl SecurityPolicy {
    /// Replace the env allowlist wholesale. Use this when the hook needs
    /// an entirely different set of variables (e.g. a CI runner where
    /// `HOME` should be hidden but `BUILDKITE_*` is required).
    pub fn with_env_allowlist<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.env_allowlist = names.into_iter().map(Into::into).collect();
        self
    }

    /// Append to the existing env allowlist — convenient for the common
    /// case of the default plus one or two extras.
    pub fn with_extra_env<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.env_allowlist.extend(names.into_iter().map(Into::into));
        self
    }
}

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
    /// Capability probe (SPEC-032 REQ-3216). Sent once per spawned hook
    /// immediately after the handshake, before `init`. Hook SHALL reply
    /// with a single [`HookMessage::ProbeResult`] line declaring the
    /// stages and AST types it supports. Probe failures disable the hook
    /// for the current session with an actionable diagnostic.
    Probe(ProbeMessage),
    /// One-time setup. Sent immediately after a successful probe.
    Init(InitMessage),
    /// Invoke the hook against a single page.
    Run(RunMessage),
    /// End-of-build signal — hook may emit final `build_data` /
    /// diagnostics via a `result` response.
    Finalise(FinaliseMessage),
    /// Graceful teardown. Zetl closes stdin immediately after sending.
    Shutdown(ShutdownMessage),
}

/// Probe-mode request body (SPEC-032 REQ-3216). Carries the active
/// build mode so hooks can decline via `applies_when.modes` when the
/// mode doesn't match (e.g. a build-only hook under `zetl serve`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProbeMessage {
    /// `"build"` or `"serve"` — hooks pattern-match against this to
    /// decide whether to report `ready: true` or `ready: false`.
    /// Absent on `--probe` argv form (where the caller has no build
    /// context yet); hooks MUST assume "any mode" in that case.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
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
    /// Reply to a [`HostMessage::Probe`] — declares the hook's supported
    /// stages, AST type(s), and readiness (SPEC-032 REQ-3216).
    #[serde(rename = "probe_result")]
    ProbeResult(ProbeResult),
    /// Typed failure. The caller's REQ-3207 policy decides whether to
    /// revert the page, continue, or abort.
    Error {
        reason: String,
        #[serde(default)]
        detail: String,
    },
}

/// Capability-probe reply body (SPEC-032 REQ-3216).
///
/// The canonical wire shape is:
/// ```json
/// {
///   "type": "probe_result",
///   "zetl_ast": "1.0",
///   "hook": "callouts",
///   "version": "1.0.3",
///   "stages": ["transform"],
///   "ast_types": ["zetl-ext"],
///   "applies_when": {"modes": ["build","serve"], "themes": null, "formats": ["html"]},
///   "ready": true
/// }
/// ```
///
/// Hooks that decline to run (`ready: false`) may include `reason` for
/// the diagnostic surface. `applies_when` is optional; absent means
/// "applies always".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProbeResult {
    /// AST schema version the hook targets. Free-form string; the
    /// caller does a semver-compatible comparison against
    /// `crate::hooks::ast::AST_VERSION`.
    pub zetl_ast: String,
    /// Hook id — usually the manifest `extension_id`.
    pub hook: String,
    /// Hook version string.
    pub version: String,
    /// Pipeline stages this hook handles. `[]` is a manifest/probe
    /// mismatch and disables the hook.
    #[serde(default)]
    pub stages: Vec<String>,
    /// AST ecosystems this hook can read/emit. Absent → `["zetl-ext"]`
    /// fallback (the historical default before task-capability-probe).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ast_types: Option<Vec<String>>,
    /// Optional filter. Absent → applies-always.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applies_when: Option<AppliesWhen>,
    /// `true` — hook accepts invocation; `false` — hook declines.
    pub ready: bool,
    /// Free-form reason for `ready: false`; surfaced in the diagnostic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Conditional-applicability clause inside a [`ProbeResult`].
///
/// All fields are optional. A `null` / absent field means "no
/// constraint on this dimension". Within a field, the value is a
/// *whitelist* — the hook applies when the current build's value is a
/// member of the list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AppliesWhen {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modes: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub themes: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formats: Option<Vec<String>>,
}

impl AppliesWhen {
    /// `true` when the clause permits the given mode. Missing modes =
    /// "no mode constraint" = always permitted.
    pub fn permits_mode(&self, mode: &str) -> bool {
        match &self.modes {
            None => true,
            Some(list) => list.iter().any(|m| m == mode),
        }
    }

    /// `true` when the clause permits the given theme. Missing themes =
    /// "no theme constraint" = always permitted.
    pub fn permits_theme(&self, theme: &str) -> bool {
        match &self.themes {
            None => true,
            Some(list) => list.iter().any(|t| t == theme),
        }
    }
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
    /// A single host-or-hook message exceeded
    /// [`SecurityPolicy::max_message_bytes`]. Direction is `"send"` for
    /// host → hook writes (caller serialised something too large) and
    /// `"recv"` for hook → host reads (the hook is misbehaving and is
    /// killed). SPEC-032 §10 untrusted-JSON guard.
    MessageTooLarge {
        direction: &'static str,
        size: usize,
        limit: usize,
    },
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
            ProtocolError::MessageTooLarge {
                direction,
                size,
                limit,
            } => write!(
                f,
                "persistent hook {direction} message size {size} exceeded limit {limit}"
            ),
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
    /// Per-message wire-size cap copied from the [`SecurityPolicy`] at
    /// spawn time. Enforced on every host → hook write so a host-side
    /// bug can't smuggle an oversized payload past the receive-side cap.
    max_message_bytes: usize,
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
    /// fractions of a second. Uses the default [`SecurityPolicy`].
    pub fn spawn_with_config(
        command: Command,
        hook_id: impl Into<String>,
        stage: Stage,
        handshake_timeout: Duration,
        shutdown_grace: Duration,
    ) -> Result<Self, ProtocolError> {
        Self::spawn_with_policy(
            command,
            hook_id,
            stage,
            handshake_timeout,
            shutdown_grace,
            SecurityPolicy::default(),
        )
    }

    /// Spawn with a fully customised [`SecurityPolicy`]. Use this when
    /// the default env allowlist or message caps don't fit.
    ///
    /// SPEC-032 §10 defines the parent → child env-leak posture: the
    /// policy's `env_allowlist` is forwarded from `std::env`; the
    /// command's explicitly-set environment (added via `cmd.env(...)`
    /// before spawn) is preserved on top, so callers can pass in
    /// `ZETL_*` vars that aren't in the parent's environment.
    pub fn spawn_with_policy(
        mut command: Command,
        hook_id: impl Into<String>,
        stage: Stage,
        handshake_timeout: Duration,
        shutdown_grace: Duration,
        policy: SecurityPolicy,
    ) -> Result<Self, ProtocolError> {
        let hook_id = hook_id.into();

        // Snapshot the caller's explicit `env(...)` / `env_remove(...)` calls
        // *before* env_clear wipes the inheritance set, so we can re-apply
        // them after installing the allowlist (the caller's intent always
        // wins over the parent-env passthrough).
        let explicit_envs: Vec<(OsString, Option<OsString>)> = command
            .get_envs()
            .map(|(k, v)| (k.to_owned(), v.map(|v| v.to_owned())))
            .collect();

        // SPEC-032 §10: redact-env-by-default. After env_clear the only
        // variables visible to the child are the allowlisted ones we
        // re-add below plus the caller's explicit overrides.
        command.env_clear();
        for var in &policy.env_allowlist {
            if let Ok(val) = std::env::var(var) {
                command.env(var, val);
            }
        }
        for (k, v) in explicit_envs {
            match v {
                Some(val) => {
                    command.env(&k, val);
                }
                None => {
                    command.env_remove(&k);
                }
            }
        }

        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdin = BufWriter::new(child.stdin.take().expect("piped stdin"));
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");

        let (tx, stdout_rx) = mpsc::sync_channel::<std::io::Result<String>>(32);
        let max_message_bytes = policy.max_message_bytes;
        let reader_handle = thread::Builder::new()
            .name(format!("zetl-hook-{hook_id}-stdout"))
            .spawn(move || pump_stdout(stdout, tx, max_message_bytes))
            .expect("spawn stdout pump");

        let stderr_buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let stderr_buf2 = Arc::clone(&stderr_buf);
        let max_stderr_bytes = policy.max_stderr_bytes;
        let stderr_handle = thread::Builder::new()
            .name(format!("zetl-hook-{hook_id}-stderr"))
            .spawn(move || pump_stderr(stderr, stderr_buf2, max_stderr_bytes))
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
            max_message_bytes,
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

    /// Send a `probe` message and await the hook's `probe_result` line
    /// (SPEC-032 REQ-3216). The probe runs once per session, immediately
    /// after handshake; hooks that don't speak probe will time out and
    /// be classified as probe-failed by the caller.
    ///
    /// `mode` is `"build"` / `"serve"` and is echoed into the probe body
    /// so hooks can branch on `applies_when.modes`. Pass `None` when the
    /// probe is run outside a build (e.g. `zetl hook capabilities`).
    pub fn probe(
        &mut self,
        mode: Option<&str>,
        deadline_ms: u64,
    ) -> Result<ProbeResult, ProtocolError> {
        let msg = HostMessage::Probe(ProbeMessage {
            mode: mode.map(|m| m.to_string()),
        });
        match self.exchange(&msg, Duration::from_millis(deadline_ms))? {
            HookMessage::ProbeResult(pr) => Ok(pr),
            HookMessage::Result { .. } => Err(ProtocolError::Handshake(
                "hook returned `result` in response to probe; expected `probe_result`".into(),
            )),
            // `Error` is already converted to ProtocolError::HookError by
            // `exchange` before we get here, so this arm is unreachable —
            // kept for serde-exhaustiveness.
            HookMessage::Error { reason, detail } => {
                Err(ProtocolError::HookError { reason, detail })
            }
        }
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
        // SPEC-032 §10: enforce the wire-size cap on the host side too,
        // so a host-bug-induced runaway payload trips the same diagnostic
        // as a hook-side one.
        if line.len() > self.max_message_bytes {
            return Err(ProtocolError::MessageTooLarge {
                direction: "send",
                size: line.len(),
                limit: self.max_message_bytes,
            });
        }
        stdin.write_all(line.as_bytes())?;
        stdin.write_all(b"\n")?;
        stdin.flush()?;
        Ok(())
    }

    fn recv_response(&self, deadline: Duration) -> Result<HookMessage, ProtocolError> {
        match self.stdout_rx.recv_timeout(deadline) {
            Ok(Ok(line)) => Ok(serde_json::from_str(&line)?),
            Ok(Err(e)) => {
                // The stdout pump tags an oversized line by wrapping the
                // size in an InvalidData error with a `zetl:msg-too-large:`
                // prefix; promote it to the typed protocol variant so
                // callers can branch on policy violation vs. plain I/O.
                if e.kind() == std::io::ErrorKind::InvalidData {
                    let msg = e.to_string();
                    if let Some(rest) = msg.strip_prefix("zetl:msg-too-large:") {
                        if let Some((size, limit)) = rest.split_once(':') {
                            if let (Ok(size), Ok(limit)) =
                                (size.parse::<usize>(), limit.parse::<usize>())
                            {
                                return Err(ProtocolError::MessageTooLarge {
                                    direction: "recv",
                                    size,
                                    limit,
                                });
                            }
                        }
                    }
                }
                Err(ProtocolError::Io(e))
            }
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

fn pump_stdout(
    stdout: ChildStdout,
    tx: mpsc::SyncSender<std::io::Result<String>>,
    max_message_bytes: usize,
) {
    let mut reader = BufReader::new(stdout);
    let mut buf: Vec<u8> = Vec::with_capacity(4096);
    loop {
        buf.clear();
        match read_line_capped(&mut reader, &mut buf, max_message_bytes) {
            Ok(0) => break,
            Ok(_) => {
                if buf.last() == Some(&b'\n') {
                    buf.pop();
                }
                if buf.last() == Some(&b'\r') {
                    buf.pop();
                }
                let line = match std::str::from_utf8(&buf) {
                    Ok(s) => s.to_string(),
                    Err(e) => {
                        let _ = tx.send(Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            e,
                        )));
                        break;
                    }
                };
                if tx.send(Ok(line)).is_err() {
                    break;
                }
            }
            Err(e) => {
                let _ = tx.send(Err(e));
                break;
            }
        }
    }
}

/// Read up to and including the next `\n`, capping the total bytes
/// consumed at `limit`. Returns `Ok(0)` on clean EOF, `Ok(n)` with the
/// line in `buf` (newline retained) on success, or `Err(InvalidData)`
/// tagged with a `zetl:msg-too-large:<size>:<limit>` payload when the
/// line exceeds `limit`. Unlike [`BufRead::read_until`] this never
/// allocates more than `limit + buffer-fill` bytes, so an unbounded
/// hook output can't OOM the host.
fn read_line_capped<R: BufRead>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    limit: usize,
) -> std::io::Result<usize> {
    let mut total = 0usize;
    loop {
        let chunk = match reader.fill_buf() {
            Ok(c) => c,
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if chunk.is_empty() {
            return Ok(total);
        }
        if let Some(nl) = chunk.iter().position(|&b| b == b'\n') {
            let take = nl + 1;
            let new_total = total + take;
            if new_total > limit {
                let consumed = chunk.len();
                reader.consume(consumed);
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("zetl:msg-too-large:{new_total}:{limit}"),
                ));
            }
            buf.extend_from_slice(&chunk[..take]);
            reader.consume(take);
            return Ok(new_total);
        }
        let len = chunk.len();
        let new_total = total + len;
        if new_total > limit {
            reader.consume(len);
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("zetl:msg-too-large:{new_total}:{limit}"),
            ));
        }
        buf.extend_from_slice(chunk);
        reader.consume(len);
        total = new_total;
    }
}

fn pump_stderr(
    mut stderr: std::process::ChildStderr,
    buf: Arc<Mutex<Vec<u8>>>,
    max_bytes: usize,
) {
    let mut chunk = [0u8; 4096];
    let mut truncated = false;
    loop {
        match stderr.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                if truncated {
                    // Cap reached on a previous read; drop further bytes
                    // on the floor so a chatty hook can't OOM the host.
                    continue;
                }
                let mut g = buf.lock().unwrap();
                let remaining = max_bytes.saturating_sub(g.len());
                let take = remaining.min(n);
                if take > 0 {
                    g.extend_from_slice(&chunk[..take]);
                }
                if take < n {
                    g.extend_from_slice(STDERR_TRUNCATED_MARKER);
                    truncated = true;
                }
            }
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

    // ── SPEC-032 §10 security ──────────────────────────────────────────────

    /// SPEC-032 §10 redact-env-by-default: a parent-set environment
    /// variable that isn't in [`DEFAULT_ENV_ALLOWLIST`] must be invisible
    /// to a freshly-spawned hook. Acceptance criterion for task
    /// security-hooks: `echo $SECRET` from a hook reveals nothing.
    ///
    /// The hook here echoes back `os.environ.get("ZETL_TEST_SECRET", "<unset>")`
    /// in the run-response payload so we can assert against it without
    /// touching the parent process's actual environment from the test
    /// matrix runner's POV.
    #[test]
    fn env_redacted_by_default() {
        require_python!();
        let tmp = TempDir::new().unwrap();
        let hook = write_python_hook(
            tmp.path(),
            "envprobe.py",
            r#"
import json, os, sys
sys.stdout.write('{"zetl_ast":1,"hook":"envprobe","version":"0","ready":true}\n')
sys.stdout.flush()
for line in sys.stdin:
    msg = json.loads(line)
    t = msg.get("type")
    if t == "shutdown":
        break
    secret = os.environ.get("ZETL_TEST_SECRET", "<unset>")
    resp = {"type":"result","payload":{"secret": secret}}
    sys.stdout.write(json.dumps(resp) + "\n")
    sys.stdout.flush()
"#,
        );

        // Set the secret in the parent so the leak path *would* be live
        // were it not for env_clear. Using std::env::set_var is the
        // standard way to surface this to Command::spawn's inheritance.
        std::env::set_var("ZETL_TEST_SECRET", "leaked-via-inherit");

        let mut h = PersistentHook::spawn(
            Command::new(&hook),
            "envprobe",
            Stage::Transform,
        )
        .unwrap();
        let resp = h.run("p", json!({}), json!({}), 1000).unwrap();
        std::env::remove_var("ZETL_TEST_SECRET");

        match resp {
            HookMessage::Result { payload, .. } => {
                assert_eq!(
                    payload["secret"], "<unset>",
                    "env leak: hook saw ZETL_TEST_SECRET despite redact-env-by-default"
                );
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }

    /// Allow-listed essentials (PATH at minimum) must still reach the
    /// child — without PATH, a `#!/usr/bin/env python3` shebang can't
    /// resolve `python3` in the first place. Verify a default-policy
    /// spawn round-trips a hook that depends on PATH for execution.
    #[test]
    fn env_allowlist_passes_path() {
        require_python!();
        let tmp = TempDir::new().unwrap();
        // Hook executes via `#!/usr/bin/env python3` — needs PATH visible.
        let hook = write_python_hook(tmp.path(), "echo.py", ECHO_BODY);
        let mut h = PersistentHook::spawn(
            Command::new(&hook),
            "echo",
            Stage::Transform,
        )
        .unwrap();
        let resp = h.run("p", json!({}), json!({"k": "v"}), 1000).unwrap();
        match resp {
            HookMessage::Result { payload, .. } => {
                assert_eq!(payload, json!({"k": "v"}));
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }

    /// Caller-supplied `cmd.env(...)` overrides survive `env_clear`. The
    /// security policy redacts the *inherited* environment but must not
    /// trample explicit ZETL_* vars that the spawn site set up before
    /// handing over the [`Command`].
    #[test]
    fn explicit_command_env_survives_redaction() {
        require_python!();
        let tmp = TempDir::new().unwrap();
        let hook = write_python_hook(
            tmp.path(),
            "explicit.py",
            r#"
import json, os, sys
sys.stdout.write('{"zetl_ast":1,"hook":"explicit","version":"0","ready":true}\n')
sys.stdout.flush()
for line in sys.stdin:
    msg = json.loads(line)
    if msg.get("type") == "shutdown":
        break
    resp = {"type":"result","payload":{
        "zetl_extension_id": os.environ.get("ZETL_EXTENSION_ID", "<unset>"),
        "zetl_random": os.environ.get("ZETL_TEST_OVERRIDE", "<unset>"),
    }}
    sys.stdout.write(json.dumps(resp) + "\n")
    sys.stdout.flush()
"#,
        );

        let mut cmd = Command::new(&hook);
        cmd.env("ZETL_EXTENSION_ID", "explicit-id")
            .env("ZETL_TEST_OVERRIDE", "explicit-value");

        let mut h =
            PersistentHook::spawn(cmd, "explicit", Stage::Transform).unwrap();
        let resp = h.run("p", json!({}), json!({}), 1000).unwrap();
        match resp {
            HookMessage::Result { payload, .. } => {
                assert_eq!(payload["zetl_extension_id"], "explicit-id");
                assert_eq!(payload["zetl_random"], "explicit-value");
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }

    /// `SecurityPolicy::with_extra_env` lets a vault opt a single
    /// variable back in (the documented escape hatch for hooks that
    /// genuinely need e.g. `VIRTUAL_ENV`). Verify the extra var lands
    /// while a non-allowlisted sibling stays redacted.
    #[test]
    fn extra_env_allowlist_opts_specific_vars_back_in() {
        require_python!();
        let tmp = TempDir::new().unwrap();
        let hook = write_python_hook(
            tmp.path(),
            "extra.py",
            r#"
import json, os, sys
sys.stdout.write('{"zetl_ast":1,"hook":"extra","version":"0","ready":true}\n')
sys.stdout.flush()
for line in sys.stdin:
    msg = json.loads(line)
    if msg.get("type") == "shutdown":
        break
    resp = {"type":"result","payload":{
        "opted_in": os.environ.get("ZETL_TEST_OPT_IN", "<unset>"),
        "still_redacted": os.environ.get("ZETL_TEST_STILL_HIDDEN", "<unset>"),
    }}
    sys.stdout.write(json.dumps(resp) + "\n")
    sys.stdout.flush()
"#,
        );

        std::env::set_var("ZETL_TEST_OPT_IN", "visible");
        std::env::set_var("ZETL_TEST_STILL_HIDDEN", "must-not-leak");

        let policy = SecurityPolicy::default().with_extra_env(["ZETL_TEST_OPT_IN"]);
        let mut h = PersistentHook::spawn_with_policy(
            Command::new(&hook),
            "extra",
            Stage::Transform,
            DEFAULT_HANDSHAKE_TIMEOUT,
            DEFAULT_SHUTDOWN_GRACE,
            policy,
        )
        .unwrap();
        let resp = h.run("p", json!({}), json!({}), 1000).unwrap();
        std::env::remove_var("ZETL_TEST_OPT_IN");
        std::env::remove_var("ZETL_TEST_STILL_HIDDEN");

        match resp {
            HookMessage::Result { payload, .. } => {
                assert_eq!(payload["opted_in"], "visible");
                assert_eq!(payload["still_redacted"], "<unset>");
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }

    /// SPEC-032 §10 stderr buffering: a chatty hook's stderr is bounded
    /// by [`SecurityPolicy::max_stderr_bytes`]. Once the cap is hit,
    /// further bytes are dropped on the floor and a single
    /// [`STDERR_TRUNCATED_MARKER`] is appended so the operator knows the
    /// captured tail is not the whole story.
    #[test]
    fn stderr_capped_with_truncation_marker() {
        require_python!();
        let tmp = TempDir::new().unwrap();
        let hook = write_python_hook(
            tmp.path(),
            "loud.py",
            r#"
import json, sys
sys.stdout.write('{"zetl_ast":1,"hook":"loud","version":"0","ready":true}\n')
sys.stdout.flush()
# Spam well past the cap (4 KiB cap × ~16 chunks).
chunk = "X" * 4096
for _ in range(64):
    sys.stderr.write(chunk)
sys.stderr.flush()
for line in sys.stdin:
    msg = json.loads(line)
    if msg.get("type") == "shutdown":
        break
    sys.stdout.write(json.dumps({"type":"result","payload":{}}) + "\n")
    sys.stdout.flush()
"#,
        );

        // Tiny cap so the test runs fast and asserts hard.
        let policy = SecurityPolicy {
            max_stderr_bytes: 4096,
            ..SecurityPolicy::default()
        };
        let mut h = PersistentHook::spawn_with_policy(
            Command::new(&hook),
            "loud",
            Stage::Transform,
            DEFAULT_HANDSHAKE_TIMEOUT,
            DEFAULT_SHUTDOWN_GRACE,
            policy,
        )
        .unwrap();
        let _ = h.run("p", json!({}), json!({}), 1000).unwrap();
        thread::sleep(Duration::from_millis(100));
        let captured = h.drain_stderr();
        let marker = std::str::from_utf8(STDERR_TRUNCATED_MARKER).unwrap();
        assert!(
            captured.contains(marker),
            "expected truncation marker, got {} bytes",
            captured.len()
        );
        // Cap + marker length is the upper bound; allow some slack for
        // the marker's leading newline that may land mid-chunk.
        assert!(
            captured.len() <= 4096 + STDERR_TRUNCATED_MARKER.len() + 4096,
            "captured {} bytes blew past cap",
            captured.len()
        );
    }

    /// Receive-side message cap: a hook emitting a single line larger
    /// than [`SecurityPolicy::max_message_bytes`] returns the typed
    /// [`ProtocolError::MessageTooLarge`] (not a generic IO error) and
    /// the instance is killed. Critical untrusted-JSON guard.
    #[test]
    fn oversized_response_line_returns_typed_error() {
        require_python!();
        let tmp = TempDir::new().unwrap();
        let hook = write_python_hook(
            tmp.path(),
            "huge.py",
            r#"
import json, sys
sys.stdout.write('{"zetl_ast":1,"hook":"huge","version":"0","ready":true}\n')
sys.stdout.flush()
for line in sys.stdin:
    msg = json.loads(line)
    if msg.get("type") == "shutdown":
        break
    # Emit a line well above the configured cap (4 KiB in the test).
    blob = "x" * (16 * 1024)
    sys.stdout.write('{"type":"result","payload":{"blob":"' + blob + '"}}\n')
    sys.stdout.flush()
"#,
        );

        let policy = SecurityPolicy {
            max_message_bytes: 4096,
            ..SecurityPolicy::default()
        };
        let mut h = PersistentHook::spawn_with_policy(
            Command::new(&hook),
            "huge",
            Stage::Transform,
            DEFAULT_HANDSHAKE_TIMEOUT,
            DEFAULT_SHUTDOWN_GRACE,
            policy,
        )
        .unwrap();
        let err = h.run("p", json!({}), json!({}), 1000).unwrap_err();
        match err {
            ProtocolError::MessageTooLarge {
                direction,
                size,
                limit,
            } => {
                assert_eq!(direction, "recv");
                assert_eq!(limit, 4096);
                assert!(size > limit, "size {size} should exceed limit {limit}");
            }
            other => panic!("expected MessageTooLarge(recv), got {other:?}"),
        }
        // Caller's REQ-3207 path now sees a dead instance.
        assert!(h.is_dead());
    }

    /// Send-side message cap: a host that tries to push a payload above
    /// the cap fails fast with a typed `MessageTooLarge` *before* the
    /// bytes hit the wire. Mirrors the recv-side guard so a host bug
    /// can't force the hook to handle (or get killed by) an oversized
    /// message it didn't ask for.
    #[test]
    fn oversized_send_payload_returns_typed_error() {
        require_python!();
        let tmp = TempDir::new().unwrap();
        let hook = write_python_hook(tmp.path(), "echo.py", ECHO_BODY);

        let policy = SecurityPolicy {
            max_message_bytes: 1024,
            ..SecurityPolicy::default()
        };
        let mut h = PersistentHook::spawn_with_policy(
            Command::new(&hook),
            "echo",
            Stage::Transform,
            DEFAULT_HANDSHAKE_TIMEOUT,
            DEFAULT_SHUTDOWN_GRACE,
            policy,
        )
        .unwrap();
        let big = "x".repeat(8 * 1024);
        let err = h
            .run("p", json!({}), json!({ "blob": big }), 1000)
            .unwrap_err();
        match err {
            ProtocolError::MessageTooLarge {
                direction,
                size,
                limit,
            } => {
                assert_eq!(direction, "send");
                assert_eq!(limit, 1024);
                assert!(size > limit, "size {size} should exceed limit {limit}");
            }
            other => panic!("expected MessageTooLarge(send), got {other:?}"),
        }
        // Send-side cap kills the instance (host-side bug equally fatal).
        assert!(h.is_dead());
    }

    /// Default policy values track the SPEC-032 §10 caps. Guard against
    /// silent regressions in [`DEFAULT_MAX_MESSAGE_BYTES`] /
    /// [`DEFAULT_MAX_STDERR_BYTES`] that would weaken untrusted-input
    /// containment.
    #[test]
    fn default_policy_carries_spec_32_caps() {
        let p = SecurityPolicy::default();
        assert_eq!(p.max_message_bytes, 10 * 1024 * 1024);
        assert_eq!(p.max_stderr_bytes, 1024 * 1024);
        // PATH must always be allowlisted — without it, no shebang resolves.
        assert!(p.env_allowlist.iter().any(|n| n == "PATH"));
        // Common secret-bearing names must NOT be present by default.
        for forbidden in [
            "AWS_SECRET_ACCESS_KEY",
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "GITHUB_TOKEN",
        ] {
            assert!(
                !p.env_allowlist.iter().any(|n| n == forbidden),
                "{forbidden} must not be in default allowlist"
            );
        }
    }
}
