//! remark adapter (SPEC-033 REQ-3305 / CON-3305 / ADR-3304).
//!
//! Implements the [`EcosystemAdapter`] trait for the remark / unified
//! ecosystem. Compiled behind the `ecosystem-remark` cargo feature
//! (NFR-3304) so a minimal zetl build can drop this code entirely; the
//! registry entry stays unconditional in [`crate::ecosystems::registry`]
//! so `zetl ecosystem check` can still list `remark` on a stripped binary.
//!
//! ## Transport (ADR-3304)
//!
//! remark plugins are in-process ESM JavaScript and there is no reasonable
//! way to execute them from Rust. ADR-3304 resolves this by spawning a
//! long-lived Node.js subprocess running a zetl-provided harness
//! (`_static/zetl-remark-harness.mjs`, embedded at compile time via
//! [`include_str!`]) that loads plugins via dynamic `import()` and
//! exchanges JSON-RPC-like messages on stdin/stdout (CON-3305).
//!
//! The harness is bounced onto disk as a temporary file (the Node runtime
//! only accepts file paths, not stdin-script feeding for ESM) and then
//! spawned with argv `[node, <harness-path>]`. Plugin resolution uses
//! Node's own `node_modules` walk, rooted at
//! [`PluginManifest::extension_id`]-adjacent cwd — the adapter passes the
//! vault root (via [`BuildContext::vault_root`]) as the harness's
//! cwd so `import("remark-gfm")` finds the project's installed copy.
//!
//! ## Isolation modes (REQ-3305)
//!
//! A manifest field `isolation = "shared" | "fresh-context"` controls
//! whether plugin applications share the harness's module cache:
//!
//! - **`shared`** (default): one Node subprocess per zetl build. Plugin
//!   imports are cached; subsequent pages reuse the loaded plugin. This
//!   is the perf path — startup amortises across the whole build.
//! - **`fresh-context`**: a new Node subprocess is spawned per invocation
//!   and shut down afterwards. Isolates plugins from each other (a
//!   plugin-A monkey-patch can't leak to plugin-B).
//!
//! The harness script is identical in both modes — the caller (the
//! adapter's [`Self::invoke_plugin`] branch here) decides which lifetime
//! to give it.
//!
//! ## Protocol (CON-3305)
//!
//! ```text
//! zetl → harness:  {"id":1,"type":"load_plugin","package":"remark-gfm","options":{}}
//! harness → zetl:  {"id":1,"type":"load_result","ok":true,"plugin_id":"rp_1"}
//!
//! zetl → harness:  {"id":2,"type":"apply","plugin_id":"rp_1","ast":{...mdast...}}
//! harness → zetl:  {"id":2,"type":"apply_result","ok":true,"ast":{...mdast...}}
//!
//! zetl → harness:  {"id":3,"type":"shutdown"}
//! harness → zetl:  {"id":3,"type":"shutdown_result","ok":true}
//! harness exits 0
//! ```
//!
//! On startup the harness emits an out-of-band `{"type":"ready", ...}`
//! banner so the spawning adapter can confirm the pipe is live and
//! detect an early `import("unified")` failure before sending requests.

#![cfg(feature = "ecosystem-remark")]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ecosystems::adapter::{
    Diagnostic, EcosystemAdapter, HookContext, PluginManifest, PluginResponse, RuntimeStatus,
    StageInput, StageOutput,
};
use crate::ecosystems::detection::{install_hint_for, probe_runtime_dep};
use crate::ecosystems::registry::{Ecosystem, EcosystemEntry};
use crate::hooks::ast::Document;
use crate::hooks::pipeline::Stage;
use crate::hooks::translators::{mdast::MdastTranslator, AstType, TranslationError, Translator};

/// Default shutdown grace period for a harness subprocess.
const DEFAULT_SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

/// Bounded deadline for the harness's startup `ready` banner. A harness
/// that takes longer than this is treated as broken — either `node`
/// itself is slow to start (uncommon on any modern install) or our
/// harness script failed to parse.
const READY_TIMEOUT: Duration = Duration::from_secs(10);

/// Embedded harness source. Written to a temp file on spawn so Node can
/// load it as an ESM module.
pub const HARNESS_SOURCE: &str = include_str!("../../_static/zetl-remark-harness.mjs");

/// Embedded harness version — matches the banner string the harness
/// script emits. Bumped in lock-step with `HARNESS_SOURCE` changes.
pub const HARNESS_VERSION: &str = "1.0.0";

const NODE_BINARY: &str = "node";

// ── Isolation mode ───────────────────────────────────────────────────────

/// Plugin-harness isolation strategy declared per manifest (REQ-3305).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IsolationMode {
    /// One harness per zetl process; plugins share the module cache.
    /// The perf-default.
    Shared,
    /// New harness subprocess per invocation; plugins are isolated from
    /// each other. Paranoid mode — pays the Node startup cost per page.
    FreshContext,
}

impl Default for IsolationMode {
    fn default() -> Self {
        IsolationMode::Shared
    }
}

impl IsolationMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            IsolationMode::Shared => "shared",
            IsolationMode::FreshContext => "fresh-context",
        }
    }
}

impl std::fmt::Display for IsolationMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ── Manifest options ─────────────────────────────────────────────────────

/// Ecosystem-specific options block read out of
/// [`PluginManifest::options`]. Every field is optional; deserialisation
/// ignores unknown keys so downstream evolution stays additive.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RemarkOptions {
    /// npm package name to `import()` inside the harness. Optional
    /// because some callers (e.g. the conformance harness) don't
    /// exercise a real plugin — they only want the trait plumbing.
    #[serde(default)]
    pub package: Option<String>,
    /// Plugin options passed verbatim to the unified initialiser.
    #[serde(default)]
    pub plugin_options: Value,
    /// Isolation mode (see [`IsolationMode`]). Defaults to `Shared`.
    #[serde(default)]
    pub isolation: Option<IsolationMode>,
}

impl RemarkOptions {
    pub fn from_value(value: &Value) -> Result<Self, serde_json::Error> {
        if value.is_null() {
            Ok(Self::default())
        } else {
            serde_json::from_value(value.clone())
        }
    }

    pub fn effective_isolation(&self) -> IsolationMode {
        self.isolation.unwrap_or_default()
    }
}

// ── Harness subprocess ───────────────────────────────────────────────────

/// Auto-incrementing id for every message the adapter sends to the
/// harness. Responses echo the id back so a single adapter can multiplex
/// multiple in-flight requests — today we only send one at a time, but
/// the ids are free and they make log grepping easier.
static MESSAGE_ID: AtomicU64 = AtomicU64::new(1);

fn next_id() -> u64 {
    MESSAGE_ID.fetch_add(1, Ordering::Relaxed)
}

/// Handle to a live harness subprocess.
struct Harness {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    /// Stderr-pump handle (kept alive so the thread doesn't leak).
    /// Joined when the harness is dropped.
    stderr_pump: Option<thread::JoinHandle<Vec<u8>>>,
    stderr_sink: Arc<Mutex<Vec<u8>>>,
    /// Path to the temp harness script — kept so we can clean it up
    /// when the harness exits.
    _harness_path: TempHarnessFile,
    /// Reported `unified_available` flag from the ready banner. A
    /// harness whose unified import failed is still usable for ping /
    /// shutdown but any `apply` will error. We surface this up the
    /// stack as a user-facing diagnostic.
    unified_available: bool,
    /// If unified import failed, the harness reports the error string.
    unified_import_error: Option<String>,
    /// Harness version reported by the ready banner.
    reported_harness_version: String,
    /// Node version reported by the ready banner.
    reported_node_version: String,
    /// Plugin id cache: (package, options-serialised) → plugin_id. The
    /// harness allocates a new plugin_id for every successful
    /// load_plugin; we cache it so re-invoking the same plugin on a
    /// later page skips the import() round-trip.
    plugin_ids: std::collections::HashMap<(String, String), String>,
}

/// Spawn a fresh harness subprocess rooted at `cwd`. The harness is
/// written to a file first because Node's ESM loader insists on a real
/// file path, and the file must live *under* `cwd` so Node's node_modules
/// walk can find packages installed at the vault root (Node resolves
/// ESM `import()` relative to the importing file's URL, not the
/// process cwd).
fn spawn_harness(node_path: &Path, cwd: &Path) -> Result<Harness, HarnessError> {
    let harness_path = TempHarnessFile::write(cwd)?;

    let mut cmd = Command::new(node_path);
    cmd.arg(harness_path.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Plugin resolution: Node walks up from the harness cwd looking for
    // node_modules. Rooting at the vault means `import("remark-gfm")`
    // picks up a `<vault>/node_modules/remark-gfm` install.
    if cwd.is_dir() {
        cmd.current_dir(cwd);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(HarnessError::NotFound {
                binary: node_path.display().to_string(),
            });
        }
        Err(e) => return Err(HarnessError::Io(e)),
    };

    let stdin = child.stdin.take().expect("piped stdin");
    let stdout = BufReader::new(child.stdout.take().expect("piped stdout"));
    let stderr_pipe = child.stderr.take().expect("piped stderr");

    let stderr_sink = Arc::new(Mutex::new(Vec::new()));
    let stderr_pump = spawn_stderr_pump(stderr_pipe, Arc::clone(&stderr_sink));

    let mut harness = Harness {
        child,
        stdin,
        stdout,
        stderr_pump: Some(stderr_pump),
        stderr_sink,
        _harness_path: harness_path,
        unified_available: false,
        unified_import_error: None,
        reported_harness_version: String::new(),
        reported_node_version: String::new(),
        plugin_ids: std::collections::HashMap::new(),
    };

    harness.wait_for_ready(READY_TIMEOUT)?;
    Ok(harness)
}

impl Harness {
    /// Block until the harness emits its ready banner or `timeout`
    /// elapses. The banner carries the harness version + unified
    /// availability, which we stash on the struct for diagnostics.
    ///
    /// A single line is read — the harness always emits the ready
    /// banner as its first message; anything else is a protocol
    /// violation.
    fn wait_for_ready(&mut self, timeout: Duration) -> Result<(), HarnessError> {
        let deadline = Instant::now() + timeout;
        let line = self.read_line_with_deadline(deadline)?;
        let msg: Value = serde_json::from_str(&line).map_err(|e| {
            HarnessError::Protocol(format!("ready-banner was not JSON: {e} (line: {line:?})"))
        })?;
        if msg.get("type").and_then(Value::as_str) != Some("ready") {
            return Err(HarnessError::Protocol(format!(
                "expected ready banner first, got: {line}"
            )));
        }
        self.unified_available = msg
            .get("unified_available")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        self.unified_import_error = msg
            .get("unified_import_error")
            .and_then(Value::as_str)
            .map(|s| s.to_string());
        self.reported_harness_version = msg
            .get("harness_version")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        self.reported_node_version = msg
            .get("node_version")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        Ok(())
    }

    /// Read a single JSON-line response from the harness, enforcing the
    /// caller's deadline. Returns the raw line (without trailing \n).
    fn read_line_with_deadline(&mut self, deadline: Instant) -> Result<String, HarnessError> {
        // BufRead::read_line blocks on the pipe. To enforce a deadline
        // we poll in a background thread with a channel and timeout.
        // For the common path (harness responds quickly) the channel
        // round-trip is dominated by the harness's own latency.
        let (tx, rx) = std::sync::mpsc::channel();
        // We can't move self.stdout into the thread without breaking
        // subsequent reads. Instead, we perform the read on the current
        // thread with a non-blocking poll via a helper thread that
        // fires a timeout signal. Simpler: do the read inline and
        // rely on a parallel watchdog to kill the child on timeout —
        // identical strategy to the pandoc adapter (see
        // `src/hooks/persistent.rs`'s `recv_timeout`).
        let watchdog_pid = self.child.id();
        let kill_after = deadline.saturating_duration_since(Instant::now());
        let watchdog = thread::spawn(move || {
            let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
            let _ = tx.send(done_tx);
            match done_rx.recv_timeout(kill_after) {
                Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => false,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    #[cfg(unix)]
                    unsafe {
                        libc_kill(watchdog_pid as i32, 9);
                    }
                    #[cfg(not(unix))]
                    {
                        let _ = watchdog_pid;
                    }
                    true
                }
            }
        });
        // Grab the done-sender the watchdog sent us.
        let done_tx = match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(tx) => tx,
            Err(_) => return Err(HarnessError::Protocol("watchdog setup failed".into())),
        };

        let mut buf = String::new();
        let read_result = self.stdout.read_line(&mut buf);
        let _ = done_tx.send(());
        let timed_out = watchdog.join().unwrap_or(false);

        match read_result {
            Ok(0) => {
                // EOF — peer closed the pipe. Grab stderr to surface
                // anything the harness printed before dying.
                let stderr_tail = self
                    .stderr_sink
                    .lock()
                    .map(|g| String::from_utf8_lossy(&g).into_owned())
                    .unwrap_or_default();
                Err(HarnessError::Closed {
                    stderr: stderr_tail,
                })
            }
            Ok(_) => {
                if timed_out {
                    return Err(HarnessError::ReadyTimeout);
                }
                Ok(buf.trim_end_matches(|c| c == '\n' || c == '\r').to_string())
            }
            Err(e) => Err(HarnessError::Io(e)),
        }
    }

    /// Send a single JSON-line request, read one JSON-line response.
    /// The caller is responsible for matching the `id` field if it
    /// pipelines multiple requests; today every call site is serial so
    /// we rely on the harness's FIFO response ordering.
    fn request(&mut self, payload: &Value, timeout: Duration) -> Result<Value, HarnessError> {
        let line = serde_json::to_string(payload)
            .map_err(|e| HarnessError::Protocol(format!("serialise request: {e}")))?;
        self.stdin
            .write_all(line.as_bytes())
            .map_err(HarnessError::Io)?;
        self.stdin.write_all(b"\n").map_err(HarnessError::Io)?;
        self.stdin.flush().map_err(HarnessError::Io)?;
        let deadline = Instant::now() + timeout;
        let raw = self.read_line_with_deadline(deadline)?;
        serde_json::from_str(&raw)
            .map_err(|e| HarnessError::Protocol(format!("parse response: {e} (line: {raw:?})")))
    }

    /// Gracefully shut down the harness, giving it `grace` to respond
    /// to the `shutdown` message before SIGKILL-ing the process.
    fn shutdown(mut self, grace: Duration) {
        let shutdown_msg = json!({"id": next_id(), "type": "shutdown"});
        let _ = self.request(&shutdown_msg, grace);
        // Whether or not the shutdown round-trip succeeded, reap the
        // child. `try_wait` + SIGKILL fallback is the same pattern as
        // the persistent-hook protocol below.
        let start = Instant::now();
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if start.elapsed() >= grace => {
                    let _ = self.child.kill();
                    break;
                }
                Ok(None) => thread::sleep(Duration::from_millis(10)),
                Err(_) => {
                    let _ = self.child.kill();
                    break;
                }
            }
        }
        let _ = self.child.wait();
        if let Some(handle) = self.stderr_pump.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        // Best-effort teardown if the adapter was dropped mid-use.
        // Re-using `shutdown` is awkward because Drop doesn't consume
        // self; we open-code the child termination instead.
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(handle) = self.stderr_pump.take() {
            let _ = handle.join();
        }
    }
}

/// Temp file owning the harness script for the lifetime of a subprocess.
/// Dropping the `TempHarnessFile` deletes the on-disk file.
struct TempHarnessFile {
    path: PathBuf,
}

impl TempHarnessFile {
    /// Write the harness script to a unique path under `cwd` so Node's
    /// ESM resolver walks up into `<cwd>/node_modules` when the harness
    /// does `import("remark-gfm")`. Falls back to `$TMPDIR` if `cwd`
    /// isn't writable — the fallback breaks plugin resolution but keeps
    /// the spawn error path clean (e.g. probe-only tests that never
    /// call apply).
    fn write(cwd: &Path) -> Result<Self, HarnessError> {
        let unique = format!(
            ".zetl-remark-harness-{}-{}.mjs",
            std::process::id(),
            next_id()
        );
        let primary = cwd.join(&unique);
        match std::fs::File::create(&primary) {
            Ok(mut f) => {
                f.write_all(HARNESS_SOURCE.as_bytes())
                    .map_err(HarnessError::Io)?;
                f.flush().map_err(HarnessError::Io)?;
                return Ok(Self { path: primary });
            }
            Err(_) => {}
        }
        // Fallback: write to TMPDIR. Plugin resolution will fail here,
        // but the spawn path still works (ready-banner, ping, shutdown).
        let mut fallback = std::env::temp_dir();
        fallback.push(&unique);
        let mut f = std::fs::File::create(&fallback).map_err(HarnessError::Io)?;
        f.write_all(HARNESS_SOURCE.as_bytes())
            .map_err(HarnessError::Io)?;
        f.flush().map_err(HarnessError::Io)?;
        Ok(Self { path: fallback })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempHarnessFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Pump the harness's stderr into a shared buffer so the adapter can
/// attach it to error responses. Runs until the pipe closes.
fn spawn_stderr_pump(
    mut stderr: ChildStderr,
    sink: Arc<Mutex<Vec<u8>>>,
) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        use std::io::Read;
        let mut buf = [0u8; 4096];
        let mut collected = Vec::new();
        while let Ok(n) = stderr.read(&mut buf) {
            if n == 0 {
                break;
            }
            collected.extend_from_slice(&buf[..n]);
            if let Ok(mut g) = sink.lock() {
                g.extend_from_slice(&buf[..n]);
            }
        }
        collected
    })
}

/// Typed errors raised by the harness subprocess layer.
#[derive(Debug)]
enum HarnessError {
    /// `node` binary isn't on PATH (or the explicit path didn't resolve).
    NotFound { binary: String },
    /// I/O error during spawn / read / write.
    Io(std::io::Error),
    /// Malformed message or unexpected sequence.
    Protocol(String),
    /// Harness exited or closed its pipes before a response arrived.
    Closed { stderr: String },
    /// Harness didn't emit its ready banner within the deadline.
    ReadyTimeout,
    /// Harness response was `ok: false` for a protocol-level request.
    Rejected(String),
}

impl std::fmt::Display for HarnessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HarnessError::NotFound { binary } => {
                write!(f, "node binary '{binary}' not found on PATH")
            }
            HarnessError::Io(e) => write!(f, "I/O error: {e}"),
            HarnessError::Protocol(msg) => write!(f, "protocol error: {msg}"),
            HarnessError::Closed { stderr } => {
                if stderr.is_empty() {
                    f.write_str("harness closed its pipes unexpectedly")
                } else {
                    write!(f, "harness closed its pipes unexpectedly; stderr: {stderr}")
                }
            }
            HarnessError::ReadyTimeout => {
                f.write_str("harness did not emit its ready banner in time")
            }
            HarnessError::Rejected(msg) => write!(f, "harness rejected request: {msg}"),
        }
    }
}

// ── Adapter ──────────────────────────────────────────────────────────────

/// remark adapter (REQ-3305). See module docs for the full shape.
pub struct RemarkAdapter {
    translator: MdastTranslator,
    /// Resolved path to the host `node` binary, populated by
    /// [`Self::probe`]. Falls back to the bare binary name so spawn
    /// lookups use PATH if probe hasn't run.
    node_path: PathBuf,
    /// Cached node version reported by `node --version`. `None` until
    /// probe runs or it failed.
    node_version: Option<String>,
    /// `true` once probe confirmed the runtime is available at an
    /// acceptable version.
    available: bool,
    /// Live long-lived harness subprocess used for `isolation=shared`
    /// invocations. Lazily spawned on first transform call.
    shared_harness: Mutex<Option<Harness>>,
    stages: Vec<Stage>,
}

impl Default for RemarkAdapter {
    fn default() -> Self {
        Self {
            translator: MdastTranslator,
            node_path: PathBuf::from(NODE_BINARY),
            node_version: None,
            available: false,
            shared_harness: Mutex::new(None),
            stages: vec![Stage::Transform],
        }
    }
}

impl RemarkAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Cached node binary version string. `None` until probe runs.
    pub fn node_version(&self) -> Option<&str> {
        self.node_version.as_deref()
    }

    /// Resolved path to the node binary. Defaults to the bare name
    /// `"node"` until probe resolves it.
    pub fn node_path(&self) -> &Path {
        &self.node_path
    }

    /// Ensure the shared harness is spawned, returning a mutex guard on
    /// the live instance. Fails if node isn't available or the harness
    /// couldn't start.
    fn ensure_shared_harness(
        &self,
        cwd: &Path,
    ) -> Result<std::sync::MutexGuard<'_, Option<Harness>>, HarnessError> {
        let mut guard = self
            .shared_harness
            .lock()
            .map_err(|_| HarnessError::Protocol("shared harness mutex poisoned".into()))?;
        if guard.is_none() {
            let h = spawn_harness(&self.node_path, cwd)?;
            *guard = Some(h);
        }
        Ok(guard)
    }

    /// Public shutdown hook. The build pipeline calls this when the
    /// session ends so the Node subprocess reaps cleanly.
    pub fn shutdown(&self, grace: Duration) {
        if let Ok(mut guard) = self.shared_harness.lock() {
            if let Some(h) = guard.take() {
                h.shutdown(grace);
            }
        }
    }

    /// Load a plugin into the given harness (or look it up in the
    /// harness's cache). Returns the harness-assigned `plugin_id`.
    fn load_plugin(
        harness: &mut Harness,
        package: &str,
        options: &Value,
        timeout: Duration,
    ) -> Result<String, HarnessError> {
        let options_key = serde_json::to_string(options).unwrap_or_default();
        if let Some(existing) = harness
            .plugin_ids
            .get(&(package.to_string(), options_key.clone()))
        {
            return Ok(existing.clone());
        }
        let id = next_id();
        let msg = json!({
            "id": id,
            "type": "load_plugin",
            "package": package,
            "options": options,
        });
        let reply = harness.request(&msg, timeout)?;
        match reply.get("ok").and_then(Value::as_bool) {
            Some(true) => {
                let plugin_id = reply
                    .get("plugin_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| HarnessError::Protocol("load_result missing plugin_id".into()))?
                    .to_string();
                harness
                    .plugin_ids
                    .insert((package.to_string(), options_key), plugin_id.clone());
                Ok(plugin_id)
            }
            _ => {
                let err = reply
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown load_plugin error")
                    .to_string();
                Err(HarnessError::Rejected(err))
            }
        }
    }

    /// Send a single apply request and return the transformed AST.
    fn apply_plugin(
        harness: &mut Harness,
        plugin_id: &str,
        ast: &Value,
        timeout: Duration,
    ) -> Result<Value, HarnessError> {
        let id = next_id();
        let msg = json!({
            "id": id,
            "type": "apply",
            "plugin_id": plugin_id,
            "ast": ast,
        });
        let reply = harness.request(&msg, timeout)?;
        match reply.get("ok").and_then(Value::as_bool) {
            Some(true) => reply
                .get("ast")
                .cloned()
                .ok_or_else(|| HarnessError::Protocol("apply_result missing ast".into())),
            _ => {
                let err = reply
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown apply error")
                    .to_string();
                Err(HarnessError::Rejected(err))
            }
        }
    }

    /// Drive a single transform through either the shared harness or a
    /// freshly-spawned one, depending on `isolation`.
    fn dispatch_transform(
        &self,
        manifest: &PluginManifest,
        options: &RemarkOptions,
        foreign: &Value,
        cwd: &Path,
    ) -> Result<(Value, Vec<Diagnostic>, String), HarnessError> {
        let package = options.package.clone().unwrap_or_else(|| {
            // Fallback: the manifest's extension_id doubles as the
            // package name. This is mostly a convenience for the
            // conformance harness — real manifests always set package.
            manifest.extension_id.clone()
        });
        let isolation = options.effective_isolation();
        let timeout = manifest.timeout;
        let plugin_opts = options.plugin_options.clone();
        let mut diagnostics = Vec::new();

        match isolation {
            IsolationMode::Shared => {
                let mut guard = self.ensure_shared_harness(cwd)?;
                let harness = guard.as_mut().expect("ensured above");
                if !harness.unified_available {
                    return Err(HarnessError::Rejected(format!(
                        "unified is not importable: {}",
                        harness
                            .unified_import_error
                            .as_deref()
                            .unwrap_or("unknown reason")
                    )));
                }
                let plugin_id = Self::load_plugin(harness, &package, &plugin_opts, timeout)?;
                let out = Self::apply_plugin(harness, &plugin_id, foreign, timeout)?;
                let stderr = harness
                    .stderr_sink
                    .lock()
                    .map(|g| String::from_utf8_lossy(&g).into_owned())
                    .unwrap_or_default();
                Ok((out, diagnostics, stderr))
            }
            IsolationMode::FreshContext => {
                let mut harness = spawn_harness(&self.node_path, cwd)?;
                if !harness.unified_available {
                    let err = harness
                        .unified_import_error
                        .clone()
                        .unwrap_or_else(|| "unknown reason".into());
                    harness.shutdown(DEFAULT_SHUTDOWN_GRACE);
                    return Err(HarnessError::Rejected(format!(
                        "unified is not importable: {err}"
                    )));
                }
                diagnostics.push(Diagnostic {
                    severity: "info".into(),
                    message: format!(
                        "remark isolation=fresh-context: spawning dedicated node harness for '{}'",
                        manifest.extension_id
                    ),
                    page_slug: None,
                });
                let plugin_id =
                    match Self::load_plugin(&mut harness, &package, &plugin_opts, timeout) {
                        Ok(id) => id,
                        Err(e) => {
                            let stderr_tail = harness
                                .stderr_sink
                                .lock()
                                .map(|g| String::from_utf8_lossy(&g).into_owned())
                                .unwrap_or_default();
                            harness.shutdown(DEFAULT_SHUTDOWN_GRACE);
                            let _ = stderr_tail;
                            return Err(e);
                        }
                    };
                let out_result = Self::apply_plugin(&mut harness, &plugin_id, foreign, timeout);
                let stderr = harness
                    .stderr_sink
                    .lock()
                    .map(|g| String::from_utf8_lossy(&g).into_owned())
                    .unwrap_or_default();
                harness.shutdown(DEFAULT_SHUTDOWN_GRACE);
                let out = out_result?;
                Ok((out, diagnostics, stderr))
            }
        }
    }
}

impl EcosystemAdapter for RemarkAdapter {
    fn id(&self) -> &str {
        "remark"
    }

    fn ast_type(&self) -> AstType {
        AstType::MdastExt
    }

    fn probe(&mut self) -> RuntimeStatus {
        let entry: &EcosystemEntry = Ecosystem::Remark.entry();
        let status = probe_runtime_dep(&entry.runtime_dep);
        match &status {
            RuntimeStatus::Available {
                executable,
                version,
            } => {
                self.node_version = Some(version.clone());
                if let Some(path) = executable {
                    self.node_path = path.clone();
                }
                self.available = true;
            }
            _ => {
                self.available = false;
            }
        }
        status
    }

    fn supported_stages(&self) -> &[Stage] {
        &self.stages
    }

    fn translate_to_foreign(&self, doc: &Document) -> Result<Value, TranslationError> {
        self.translator.zetl_to_foreign(doc)
    }

    fn translate_from_foreign(&self, foreign: Value) -> Result<Document, TranslationError> {
        self.translator.foreign_to_zetl(foreign)
    }

    fn invoke_plugin(
        &mut self,
        manifest: &PluginManifest,
        input: StageInput,
        context: &HookContext<'_>,
    ) -> PluginResponse {
        let start = Instant::now();

        if !self.stages.contains(&context.stage) {
            return PluginResponse::error(
                "unsupported_stage",
                format!(
                    "remark adapter does not support stage {} (supports: {:?})",
                    context.stage, self.stages
                ),
                start.elapsed(),
            );
        }

        let foreign = match input {
            StageInput::Transform { foreign } => foreign,
            other => {
                return PluginResponse::error(
                    "unsupported_stage",
                    format!(
                        "remark adapter expected a transform-stage input, got {:?}",
                        other.stage()
                    ),
                    start.elapsed(),
                );
            }
        };

        let options = match RemarkOptions::from_value(&manifest.options) {
            Ok(o) => o,
            Err(e) => {
                return PluginResponse::error(
                    "manifest_parse",
                    format!("could not parse remark options: {e}"),
                    start.elapsed(),
                );
            }
        };

        let cwd = context.build.vault_root.as_path();
        match self.dispatch_transform(manifest, &options, &foreign, cwd) {
            Ok((out, diagnostics, stderr)) => PluginResponse::Success {
                output: StageOutput::Transform { foreign: out },
                diagnostics,
                stderr,
                duration: start.elapsed(),
            },
            Err(e) => {
                let duration = start.elapsed();
                harness_error_to_response(e, manifest, duration)
            }
        }
    }
}

fn harness_error_to_response(
    e: HarnessError,
    manifest: &PluginManifest,
    duration: Duration,
) -> PluginResponse {
    match e {
        HarnessError::NotFound { binary: _ } => PluginResponse::Error {
            reason: "runtime_absence".into(),
            detail: format!(
                "remark plugin '{}' could not be invoked: node not found on PATH. {}",
                manifest.extension_id,
                install_hint_for(NODE_BINARY)
            ),
            stderr: String::new(),
            duration,
        },
        HarnessError::Io(err) => PluginResponse::Error {
            reason: "spawn_failed".into(),
            detail: format!(
                "remark plugin '{}' harness I/O failed: {err}",
                manifest.extension_id
            ),
            stderr: String::new(),
            duration,
        },
        HarnessError::Protocol(msg) => PluginResponse::Error {
            reason: "malformed_output".into(),
            detail: format!(
                "remark plugin '{}' harness protocol error: {msg}",
                manifest.extension_id
            ),
            stderr: String::new(),
            duration,
        },
        HarnessError::Closed { stderr } => PluginResponse::Error {
            reason: "crash".into(),
            detail: format!(
                "remark plugin '{}' harness closed its pipes unexpectedly",
                manifest.extension_id
            ),
            stderr,
            duration,
        },
        HarnessError::ReadyTimeout => PluginResponse::Error {
            reason: "timeout".into(),
            detail: format!(
                "remark plugin '{}' harness ready-banner timed out",
                manifest.extension_id
            ),
            stderr: String::new(),
            duration,
        },
        HarnessError::Rejected(msg) => PluginResponse::Error {
            reason: "plugin_error".into(),
            detail: format!("remark plugin '{}' failed: {msg}", manifest.extension_id),
            stderr: String::new(),
            duration,
        },
    }
}

/// Convenience ctor for registry wiring.
pub fn remark_adapter_ctor() -> Box<dyn EcosystemAdapter> {
    Box::new(RemarkAdapter::new())
}

/// Thin wrapper around `libc::kill` so the non-unix code path compiles
/// without the dep.
#[cfg(unix)]
unsafe fn libc_kill(pid: i32, signum: i32) {
    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    kill(pid, signum);
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::build_context::{BuildContext, BuildMode, PageMeta};
    use std::path::PathBuf;
    use std::time::Duration;

    fn sample_manifest(package: Option<&str>) -> PluginManifest {
        let options = match package {
            Some(p) => json!({"package": p}),
            None => Value::Null,
        };
        PluginManifest {
            extension_id: "test-remark".into(),
            executable: PathBuf::from("/nonexistent/node-not-used-for-remark"),
            args: vec![],
            options,
            timeout: Duration::from_secs(5),
        }
    }

    #[test]
    fn options_default_when_null() {
        let opts = RemarkOptions::from_value(&Value::Null).unwrap();
        assert!(opts.package.is_none());
        assert!(opts.isolation.is_none());
        assert_eq!(opts.effective_isolation(), IsolationMode::Shared);
    }

    #[test]
    fn options_parse_shared_isolation() {
        let v = json!({"package": "remark-gfm", "isolation": "shared"});
        let opts = RemarkOptions::from_value(&v).unwrap();
        assert_eq!(opts.package.as_deref(), Some("remark-gfm"));
        assert_eq!(opts.effective_isolation(), IsolationMode::Shared);
    }

    #[test]
    fn options_parse_fresh_context_isolation() {
        let v = json!({"package": "remark-gfm", "isolation": "fresh-context"});
        let opts = RemarkOptions::from_value(&v).unwrap();
        assert_eq!(opts.effective_isolation(), IsolationMode::FreshContext);
    }

    #[test]
    fn options_reject_unknown_isolation_mode() {
        let v = json!({"package": "remark-gfm", "isolation": "nuclear"});
        RemarkOptions::from_value(&v).unwrap_err();
    }

    #[test]
    fn isolation_display_and_as_str_agree() {
        assert_eq!(IsolationMode::Shared.as_str(), "shared");
        assert_eq!(IsolationMode::FreshContext.as_str(), "fresh-context");
        assert_eq!(IsolationMode::Shared.to_string(), "shared");
        assert_eq!(IsolationMode::FreshContext.to_string(), "fresh-context");
    }

    #[test]
    fn adapter_id_and_ast_type_match_registry() {
        let adapter = RemarkAdapter::default();
        assert_eq!(adapter.id(), "remark");
        assert_eq!(adapter.ast_type(), AstType::MdastExt);
        assert_eq!(adapter.supported_stages(), &[Stage::Transform]);
    }

    #[test]
    fn translate_round_trips_through_mdast_ext() {
        let adapter = RemarkAdapter::default();
        let fixtures = crate::ecosystems::default_fixtures();
        for fx in &fixtures {
            let foreign = adapter.translate_to_foreign(&fx.document).unwrap();
            let back = adapter.translate_from_foreign(foreign).unwrap();
            assert_eq!(
                crate::hooks::contract::canonicalise(&back),
                crate::hooks::contract::canonicalise(&fx.document),
                "mdast translator lost fidelity on fixture '{}'",
                fx.name
            );
        }
    }

    #[test]
    fn invoke_plugin_rejects_non_transform_stage() {
        let mut adapter = RemarkAdapter::default();
        let build = BuildContext::new(
            BuildMode::Build,
            PathBuf::from("/tmp/remark-test"),
            PageMeta::synthetic("Page", "page"),
        );
        let ctx = HookContext::new(Stage::PreParse, "test-remark", &build);
        let manifest = sample_manifest(Some("remark-gfm"));
        let input = StageInput::PreParse {
            markdown: "body".into(),
        };
        match adapter.invoke_plugin(&manifest, input, &ctx) {
            PluginResponse::Error { reason, .. } => assert_eq!(reason, "unsupported_stage"),
            other => panic!("expected unsupported_stage error, got {other:?}"),
        }
    }

    #[test]
    fn invoke_plugin_reports_runtime_absence_when_node_missing() {
        // Point the adapter at a node binary that can't exist. Probe
        // hasn't populated node_path; we force it to a bogus path.
        let mut adapter = RemarkAdapter::default();
        adapter.node_path = PathBuf::from("/definitely-not-a-real-node-zetl-test");
        let build = BuildContext::new(
            BuildMode::Build,
            PathBuf::from("/tmp/remark-test"),
            PageMeta::synthetic("Page", "page"),
        );
        let ctx = HookContext::new(Stage::Transform, "test-remark", &build);
        let manifest = sample_manifest(Some("remark-gfm"));
        let input = StageInput::Transform {
            foreign: json!({"type":"root","children":[]}),
        };
        match adapter.invoke_plugin(&manifest, input, &ctx) {
            PluginResponse::Error { reason, detail, .. } => {
                assert_eq!(reason, "runtime_absence");
                assert!(
                    detail.contains("not found") || detail.contains("node"),
                    "expected runtime_absence detail, got: {detail}"
                );
            }
            other => panic!("expected runtime_absence error, got {other:?}"),
        }
    }

    #[test]
    fn harness_source_is_non_empty_and_contains_protocol_types() {
        assert!(!HARNESS_SOURCE.is_empty());
        // Spot-check the embedded source for the protocol wire strings
        // so accidental harness-script edits that drop these get flagged
        // by unit tests, not by live integration runs.
        assert!(HARNESS_SOURCE.contains("load_plugin"));
        assert!(HARNESS_SOURCE.contains("apply_result"));
        assert!(HARNESS_SOURCE.contains("shutdown"));
        assert!(HARNESS_SOURCE.contains("ready"));
    }

    #[test]
    fn harness_version_constant_matches_banner_in_source() {
        let marker = format!("HARNESS_VERSION = '{HARNESS_VERSION}'");
        assert!(
            HARNESS_SOURCE.contains(&marker),
            "HARNESS_VERSION constant drifted from harness script; expected marker '{marker}'"
        );
    }
}
