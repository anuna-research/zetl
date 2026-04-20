//! Pandoc filter adapter (SPEC-033 REQ-3303 / CON-3303 / ADR-3302).
//!
//! Implements the [`EcosystemAdapter`] trait for the Pandoc ecosystem.
//! Compiled behind the `ecosystem-pandoc` cargo feature (NFR-3304) so a
//! minimal zetl build can drop this code entirely; the registry entry
//! stays unconditional in [`crate::ecosystems::registry`] so `zetl
//! ecosystem check` can still list pandoc as a known id on a stripped
//! binary.
//!
//! ## Invocation modes (REQ-3303)
//!
//! Each manifest declares `mode = "filter" | "native"` under the
//! `[ecosystem]` / `[options]` sub-table. The adapter's [`Self::invoke_plugin`]
//! branches on the mode:
//!
//! - **`filter`** (default) — spawn the configured `exec` binary, argv =
//!   `[exec, "html"]`, env vars `PANDOC_VERSION` / `PANDOC_API_VERSION` /
//!   `PANDOC_READER_OPTIONS` / `PANDOC_WRITER_OPTIONS` /
//!   `PANDOC_SCRIPT_FILE` set per CON-3303. stdin / stdout is pandoc-types
//!   JSON. Identical to Pandoc's own filter contract — filters cannot
//!   distinguish zetl-run from pandoc-run invocations.
//! - **`native`** — spawn the installed `pandoc` binary with
//!   `--from json --to json` plus the manifest's `args` (typically
//!   `--citeproc`, `--lua-filter=...`, etc.). stdin / stdout stays
//!   pandoc-types JSON so the transform-stage contract holds.
//!
//! ## Mode resolution
//!
//! The legacy `pandoc-citeproc` binary was deprecated and removed after
//! Pandoc 2.11; its replacement is the Pandoc-native `--citeproc` flag.
//! To save users the debug session when a historical manifest still
//! names the old binary, [`resolve_mode`] auto-promotes a manifest whose
//! `exec` resolves to `pandoc-citeproc` into native mode with
//! `--citeproc` appended to args, and surfaces a one-line migration
//! hint as a [`Diagnostic`].
//!
//! ## Probe
//!
//! [`Self::probe`] shells out to `pandoc --version`, parses the binary
//! version, and — best-effort — pipes an empty document through
//! `pandoc --from markdown --to json` to discover the runtime's
//! pandoc-types API version. Both values are cached on the adapter so
//! subsequent [`Self::invoke_plugin`] calls don't re-probe. When the
//! API-version probe fails, [`FALLBACK_API_VERSION`] is used —
//! downstream filters generally accept any recent 1.x API version.

#![cfg(feature = "ecosystem-pandoc")]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ecosystems::adapter::{
    Diagnostic, EcosystemAdapter, HookContext, PluginManifest, PluginResponse, RuntimeStatus,
    StageInput, StageOutput,
};
use crate::ecosystems::detection::{install_hint_for, probe_runtime_dep};
use crate::ecosystems::registry::{Ecosystem, EcosystemEntry};
use crate::hooks::ast::Document;
use crate::hooks::pipeline::Stage;
use crate::hooks::translators::{pandoc::PandocTranslator, AstType, TranslationError, Translator};

/// pandoc-types API version used when the runtime probe can't discover
/// the user's pandoc's actual value (e.g. pandoc is not installed and
/// we're being exercised from a pure unit test). Tracks the translator's
/// compiled default — [`crate::hooks::translators::pandoc`].
pub const FALLBACK_API_VERSION: [u32; 4] = [1, 23, 1, 0];

/// Binary name probed via [`probe_runtime_dep`]. Matches
/// [`Ecosystem::Pandoc`]'s registry entry.
const PANDOC_BINARY: &str = "pandoc";

/// Bounded best-effort deadline for the API-version side-probe. If
/// pandoc is on PATH but hangs (busted wrapper script), we give up
/// quickly and fall back to [`FALLBACK_API_VERSION`] rather than making
/// the probe slow.
const API_VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

// ── Invocation modes ──────────────────────────────────────────────────────

/// Pandoc invocation mode declared per manifest (REQ-3303).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PandocMode {
    /// Traditional filter contract — argv = `[exec, "html"]`, pandoc-types
    /// JSON on stdin/stdout. Default.
    Filter,
    /// Pandoc-native invocation — spawn `pandoc` directly with the
    /// manifest's `args` (typically `--citeproc`, `--lua-filter=...`).
    Native,
}

impl Default for PandocMode {
    fn default() -> Self {
        PandocMode::Filter
    }
}

impl PandocMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            PandocMode::Filter => "filter",
            PandocMode::Native => "native",
        }
    }
}

impl std::fmt::Display for PandocMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Ecosystem-specific options block read out of
/// [`PluginManifest::options`]. Every field is optional; deserialisation
/// ignores unknown keys so downstream evolution (say, a future
/// `reader_options_json`) is additive.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PandocOptions {
    /// Invocation mode. Absent in options means "inferred" — the final
    /// effective mode is the output of [`resolve_mode`] which may
    /// auto-promote based on the `exec` basename.
    #[serde(default)]
    pub mode: Option<PandocMode>,
    /// Override the argv[1] target format. Filter convention defaults to
    /// `"html"` (matches what `pandoc --to html` would pass when it
    /// spawns the filter natively).
    #[serde(default)]
    pub target_format: Option<String>,
    /// Serialised Pandoc reader options blob — passed through as the
    /// value of the `PANDOC_READER_OPTIONS` env var. Empty string by
    /// default, matching Pandoc's own behaviour when the reader runs
    /// with no user options.
    #[serde(default)]
    pub reader_options_json: Option<String>,
    /// Serialised Pandoc writer options blob for `PANDOC_WRITER_OPTIONS`.
    #[serde(default)]
    pub writer_options_json: Option<String>,
}

impl PandocOptions {
    /// Parse from the free-form [`PluginManifest::options`] JSON value.
    /// Returns defaults on `Value::Null` so manifests without an options
    /// block just get the sane filter-mode fallbacks.
    pub fn from_value(value: &Value) -> Result<Self, serde_json::Error> {
        if value.is_null() {
            Ok(Self::default())
        } else {
            serde_json::from_value(value.clone())
        }
    }
}

/// Outcome of [`resolve_mode`] — the effective mode the adapter will
/// dispatch with, plus any user-visible warnings accumulated during
/// resolution (deprecated-binary hints, etc.).
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedInvocation {
    pub mode: PandocMode,
    /// The executable to spawn. In filter mode this is the manifest
    /// `exec`; in native mode it's the pandoc binary itself.
    pub executable: PathBuf,
    /// Final argv tail (excluding the argv[0] program name). In filter
    /// mode this is `[target_format, ...manifest.args]`; in native mode
    /// it's the set of pandoc flags.
    pub argv: Vec<String>,
    /// Non-fatal adapter-level diagnostics. Most commonly the deprecation
    /// hint emitted when `pandoc-citeproc` is auto-promoted.
    pub diagnostics: Vec<Diagnostic>,
}

/// Resolve a manifest into a concrete pandoc invocation.
///
/// Applies the `mode = filter|native` selection plus the
/// `pandoc-citeproc → native` auto-promotion. The adapter's
/// [`EcosystemAdapter::invoke_plugin`] calls this once per invocation,
/// then dispatches based on the returned mode.
pub fn resolve_mode(
    manifest: &PluginManifest,
    options: &PandocOptions,
    pandoc_binary: &Path,
) -> ResolvedInvocation {
    let target_format = options
        .target_format
        .clone()
        .unwrap_or_else(|| "html".to_string());

    // Explicit user selection always wins — if the manifest says
    // mode="filter" with exec="pandoc-citeproc" we honour that (even
    // though pandoc-citeproc no longer ships with recent pandoc
    // releases), so a custom-installed pandoc-citeproc shim still works.
    let explicit_mode = options.mode;
    let exec_name = manifest
        .executable
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");

    let auto_native = explicit_mode.is_none() && exec_name == "pandoc-citeproc";

    let mode = explicit_mode.unwrap_or_else(|| {
        if auto_native {
            PandocMode::Native
        } else {
            PandocMode::Filter
        }
    });

    let mut diagnostics = Vec::new();
    if auto_native {
        diagnostics.push(Diagnostic {
            severity: "warning".into(),
            message: format!(
                "pandoc-citeproc is deprecated (removed after Pandoc 2.11); \
                 auto-promoting '{}' to mode=native with --citeproc. \
                 Migrate the manifest to `mode = \"native\"` and \
                 `args = [\"--citeproc\"]` to silence this warning.",
                manifest.extension_id
            ),
            page_slug: None,
        });
    }

    let (executable, argv) = match mode {
        PandocMode::Filter => {
            let mut argv = Vec::with_capacity(1 + manifest.args.len());
            argv.push(target_format);
            argv.extend(manifest.args.iter().cloned());
            (manifest.executable.clone(), argv)
        }
        PandocMode::Native => {
            // Native mode routes through the host pandoc with a
            // JSON-in / JSON-out round-trip so the transform-stage
            // contract (foreign AST in, foreign AST out) keeps holding.
            let mut argv: Vec<String> =
                vec!["--from".into(), "json".into(), "--to".into(), "json".into()];
            if auto_native {
                argv.push("--citeproc".into());
            }
            argv.extend(manifest.args.iter().cloned());
            (pandoc_binary.to_path_buf(), argv)
        }
    };

    ResolvedInvocation {
        mode,
        executable,
        argv,
        diagnostics,
    }
}

// ── Adapter ───────────────────────────────────────────────────────────────

/// Pandoc adapter (REQ-3303). See module docs for the full shape.
pub struct PandocAdapter {
    translator: PandocTranslator,
    /// Resolved path to the host `pandoc` binary, populated by
    /// [`Self::probe`]. Defaults to the bare binary name so spawn lookups
    /// fall back to `PATH` if probe hasn't run.
    pandoc_path: PathBuf,
    /// Cached pandoc binary version reported by `pandoc --version`.
    /// Populated by [`Self::probe`]; `None` until then.
    pandoc_version: Option<String>,
    /// Cached pandoc-types API version observed from the host pandoc
    /// (via an empty-document round-trip). Falls back to
    /// [`FALLBACK_API_VERSION`].
    api_version: [u32; 4],
    /// `true` once the host binary has been confirmed available. Used to
    /// short-circuit spawn attempts with a typed error rather than a
    /// low-level Io::NotFound.
    available: bool,
    stages: Vec<Stage>,
}

impl Default for PandocAdapter {
    fn default() -> Self {
        Self {
            translator: PandocTranslator,
            pandoc_path: PathBuf::from(PANDOC_BINARY),
            pandoc_version: None,
            api_version: FALLBACK_API_VERSION,
            available: false,
            stages: vec![Stage::Transform],
        }
    }
}

impl PandocAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Current cached pandoc-types API version. Test-visible so unit
    /// tests can assert the probe has populated it.
    pub fn api_version(&self) -> [u32; 4] {
        self.api_version
    }

    /// Cached pandoc binary version string (as printed by
    /// `pandoc --version`). `None` if the probe hasn't run or failed.
    pub fn pandoc_version(&self) -> Option<&str> {
        self.pandoc_version.as_deref()
    }

    /// Format the cached [`Self::api_version`] as the comma-separated
    /// string Pandoc puts in `PANDOC_API_VERSION` (e.g. `"1,23,1,0"`).
    pub fn api_version_env_string(&self) -> String {
        api_version_env_string(&self.api_version)
    }

    /// Build the environment variable set for a filter-mode spawn
    /// (CON-3303). Exposed separately from [`Self::invoke_plugin`] so
    /// unit tests can assert exactly which variables the adapter sets
    /// without having to spawn a real subprocess.
    pub fn filter_env(
        &self,
        manifest: &PluginManifest,
        target_format: &str,
    ) -> Vec<(String, String)> {
        let version = self
            .pandoc_version
            .clone()
            .unwrap_or_else(|| "unknown".into());
        let reader_opts = default_reader_options_json(target_format);
        let writer_opts = default_writer_options_json(target_format);
        vec![
            ("PANDOC_VERSION".into(), version),
            ("PANDOC_API_VERSION".into(), self.api_version_env_string()),
            ("PANDOC_READER_OPTIONS".into(), reader_opts),
            ("PANDOC_WRITER_OPTIONS".into(), writer_opts),
            (
                "PANDOC_SCRIPT_FILE".into(),
                manifest.executable.display().to_string(),
            ),
        ]
    }
}

impl EcosystemAdapter for PandocAdapter {
    fn id(&self) -> &str {
        "pandoc"
    }

    fn ast_type(&self) -> AstType {
        AstType::PandocExt
    }

    fn probe(&mut self) -> RuntimeStatus {
        let entry: &EcosystemEntry = Ecosystem::Pandoc.entry();
        let status = probe_runtime_dep(&entry.runtime_dep);
        match &status {
            RuntimeStatus::Available {
                executable,
                version,
            } => {
                self.pandoc_version = Some(version.clone());
                if let Some(path) = executable {
                    self.pandoc_path = path.clone();
                }
                self.available = true;
                // Best-effort API-version side-probe. Failure here does
                // not flip status back to ProbeFailed — the filter
                // contract tolerates mismatched api versions within the
                // 1.x range.
                if let Some(api) = probe_api_version(&self.pandoc_path) {
                    self.api_version = api;
                }
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

        // REQ-3302 stage-scoping contract: unsupported stages surface as
        // a typed error instead of a panic so REQ-3207 can record a
        // failure record and continue.
        if !self.stages.contains(&context.stage) {
            return PluginResponse::error(
                "unsupported_stage",
                format!(
                    "pandoc adapter does not support stage {} (supports: {:?})",
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
                        "pandoc adapter expected a transform-stage input, got {:?}",
                        other.stage()
                    ),
                    start.elapsed(),
                );
            }
        };

        let options = match PandocOptions::from_value(&manifest.options) {
            Ok(o) => o,
            Err(e) => {
                return PluginResponse::error(
                    "manifest_parse",
                    format!("could not parse pandoc options: {e}"),
                    start.elapsed(),
                );
            }
        };

        let invocation = resolve_mode(manifest, &options, &self.pandoc_path);

        let stdin_bytes = match serde_json::to_vec(&foreign) {
            Ok(b) => b,
            Err(e) => {
                return PluginResponse::error(
                    "serialisation",
                    format!("could not serialise pandoc-types JSON: {e}"),
                    start.elapsed(),
                );
            }
        };

        let mut cmd = Command::new(&invocation.executable);
        cmd.args(&invocation.argv)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if invocation.mode == PandocMode::Filter {
            for (k, v) in
                self.filter_env(manifest, options.target_format.as_deref().unwrap_or("html"))
            {
                cmd.env(k, v);
            }
        }

        match spawn_and_exchange(cmd, &stdin_bytes, manifest.timeout) {
            Ok(exchange) => {
                let duration = start.elapsed();
                let parsed: Value = match serde_json::from_slice(&exchange.stdout) {
                    Ok(v) => v,
                    Err(e) => {
                        return PluginResponse::Error {
                            reason: "malformed_output".into(),
                            detail: format!(
                                "pandoc plugin '{}' produced non-JSON stdout: {e}",
                                manifest.extension_id
                            ),
                            stderr: String::from_utf8_lossy(&exchange.stderr).into_owned(),
                            duration,
                        };
                    }
                };
                PluginResponse::Success {
                    output: StageOutput::Transform { foreign: parsed },
                    diagnostics: invocation.diagnostics,
                    stderr: String::from_utf8_lossy(&exchange.stderr).into_owned(),
                    duration,
                }
            }
            Err(e) => {
                let duration = start.elapsed();
                match e {
                    SpawnError::NotFound { binary } => PluginResponse::Error {
                        reason: "runtime_absence".into(),
                        detail: format!(
                            "pandoc plugin '{}' could not be spawned: binary '{}' not found on PATH. {}",
                            manifest.extension_id,
                            binary,
                            install_hint_for(PANDOC_BINARY)
                        ),
                        stderr: String::new(),
                        duration,
                    },
                    SpawnError::Io(err) => PluginResponse::Error {
                        reason: "spawn_failed".into(),
                        detail: format!(
                            "pandoc plugin '{}' spawn failed: {err}",
                            manifest.extension_id
                        ),
                        stderr: String::new(),
                        duration,
                    },
                    SpawnError::Timeout => PluginResponse::Error {
                        reason: "timeout".into(),
                        detail: format!(
                            "pandoc plugin '{}' exceeded timeout of {:?}",
                            manifest.extension_id, manifest.timeout
                        ),
                        stderr: String::new(),
                        duration,
                    },
                    SpawnError::NonZeroExit { code, stderr } => PluginResponse::Error {
                        reason: "non_zero_exit".into(),
                        detail: format!(
                            "pandoc plugin '{}' exited with code {}",
                            manifest.extension_id,
                            code.map(|c| c.to_string()).unwrap_or_else(|| "signal".into())
                        ),
                        stderr: String::from_utf8_lossy(&stderr).into_owned(),
                        duration,
                    },
                }
            }
        }
    }
}

// ── Subprocess exchange ───────────────────────────────────────────────────

struct SpawnOutput {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

enum SpawnError {
    NotFound { binary: String },
    Io(std::io::Error),
    Timeout,
    NonZeroExit { code: Option<i32>, stderr: Vec<u8> },
}

/// Spawn `cmd`, write `stdin_bytes` to its stdin, and collect stdout +
/// stderr with a hard deadline.
///
/// Kept minimal on purpose — Pandoc filters are one-shot by convention
/// (CON-3303 persistent-mode is a per-filter capability zetl will layer
/// in later). The caller's `PluginManifest::timeout` is enforced here by
/// spawning a watchdog thread that kills the child on expiry.
fn spawn_and_exchange(
    mut cmd: Command,
    stdin_bytes: &[u8],
    timeout: Duration,
) -> Result<SpawnOutput, SpawnError> {
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let binary = cmd.get_program().to_string_lossy().into_owned();
            return Err(SpawnError::NotFound { binary });
        }
        Err(e) => return Err(SpawnError::Io(e)),
    };

    // Write stdin in a scoped thread so a filter that never reads stdin
    // doesn't deadlock us on the write (its stdin will be closed when
    // the handle is dropped).
    let mut stdin = child.stdin.take().expect("piped stdin");
    let stdin_payload = stdin_bytes.to_vec();
    let writer = thread::spawn(move || -> std::io::Result<()> {
        stdin.write_all(&stdin_payload)?;
        stdin.flush()?;
        drop(stdin);
        Ok(())
    });

    // Read stdout on the main thread and stderr on a helper thread to
    // avoid pipe-full deadlocks on chatty filters.
    let mut stdout_pipe = child.stdout.take().expect("piped stdout");
    let mut stderr_pipe = child.stderr.take().expect("piped stderr");
    let (stderr_tx, stderr_rx) = mpsc::channel::<Vec<u8>>();
    let stderr_handle = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr_pipe.read_to_end(&mut buf);
        let _ = stderr_tx.send(buf);
    });

    // Watchdog: kill the child if the overall exchange exceeds timeout.
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let pid = child.id();
    let watchdog = thread::spawn(move || match done_rx.recv_timeout(timeout) {
        Ok(()) | Err(RecvTimeoutError::Disconnected) => false,
        Err(RecvTimeoutError::Timeout) => {
            // Best-effort — on Unix, sending SIGKILL via std is just
            // child.kill() from another thread. We don't have the Child
            // handle here, so we leave the kill to the post-join wait
            // detecting a non-zero exit. To actually kill the process
            // we invoke `kill` via libc::kill on unix. Keep portable
            // fallback: indicate timeout and let the caller rely on
            // the read loop terminating when the child's pipes close.
            #[cfg(unix)]
            unsafe {
                // SIGKILL = 9
                libc_kill(pid as i32, 9);
            }
            true
        }
    });

    let mut stdout = Vec::new();
    let read_result = stdout_pipe.read_to_end(&mut stdout);
    let _ = done_tx.send(());
    let timed_out = watchdog.join().unwrap_or(false);
    let _ = writer.join();
    let stderr = stderr_rx.recv().unwrap_or_default();
    let _ = stderr_handle.join();

    match read_result {
        Ok(_) => {}
        Err(e) => return Err(SpawnError::Io(e)),
    }

    let status = match child.wait() {
        Ok(s) => s,
        Err(e) => return Err(SpawnError::Io(e)),
    };

    if timed_out {
        return Err(SpawnError::Timeout);
    }

    if !status.success() {
        return Err(SpawnError::NonZeroExit {
            code: status.code(),
            stderr,
        });
    }

    Ok(SpawnOutput { stdout, stderr })
}

/// Thin wrapper around `libc::kill` so the non-unix code path compiles
/// without the dep. `signum` is the POSIX signal number.
#[cfg(unix)]
unsafe fn libc_kill(pid: i32, signum: i32) {
    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    kill(pid, signum);
}

// ── API-version side-probe ────────────────────────────────────────────────

/// Run `<pandoc> --from markdown --to json` with an empty stdin and
/// extract `pandoc-api-version` from the resulting JSON. Returns `None`
/// on any failure — the caller uses [`FALLBACK_API_VERSION`] in that
/// case.
fn probe_api_version(pandoc_path: &Path) -> Option<[u32; 4]> {
    let mut cmd = Command::new(pandoc_path);
    cmd.args(["--from", "markdown", "--to", "json"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let mut child = cmd.spawn().ok()?;
    let stdin = child.stdin.as_mut()?;
    // Empty-document input — pandoc still emits a Pandoc envelope with
    // api version + empty blocks.
    let _ = stdin.write_all(b"");

    let (tx, rx) = mpsc::channel();
    let mut stdout_pipe = child.stdout.take()?;
    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout_pipe.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });

    let buf = rx.recv_timeout(API_VERSION_PROBE_TIMEOUT).ok()?;
    let _ = child.wait();

    let v: Value = serde_json::from_slice(&buf).ok()?;
    let arr = v.get("pandoc-api-version")?.as_array()?;
    let mut out = FALLBACK_API_VERSION;
    for (i, slot) in out.iter_mut().enumerate() {
        if let Some(n) = arr.get(i).and_then(|x| x.as_u64()) {
            *slot = n as u32;
        }
    }
    Some(out)
}

// ── Helpers ───────────────────────────────────────────────────────────────

/// Format the pandoc-types api version tuple the way Pandoc exposes it
/// in `PANDOC_API_VERSION` (CON-3303 example shows `"1,23,1"` — Pandoc
/// emits every component it knows). Trailing zero components are kept so
/// filters that do a strict version compare don't spuriously disagree.
pub fn api_version_env_string(v: &[u32; 4]) -> String {
    v.iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

/// Minimal default Pandoc reader options blob — Pandoc emits an empty
/// JSON object when invoked without user-provided reader options.
fn default_reader_options_json(_target_format: &str) -> String {
    "{}".to_string()
}

/// Minimal default Pandoc writer options blob — matches what Pandoc
/// emits for a bare `--to html` invocation.
fn default_writer_options_json(_target_format: &str) -> String {
    "{}".to_string()
}

/// Convenience ctor for registry wiring (parity with
/// [`crate::ecosystems::adapter::mock_adapter_ctor`]).
pub fn pandoc_adapter_ctor() -> Box<dyn EcosystemAdapter> {
    Box::new(PandocAdapter::new())
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::build_context::{BuildContext, BuildMode, PageMeta};
    use std::path::PathBuf;
    use std::time::Duration;

    fn sample_manifest(exec: &str, args: Vec<String>) -> PluginManifest {
        PluginManifest {
            extension_id: "test-pandoc".into(),
            executable: PathBuf::from(exec),
            args,
            options: Value::Null,
            timeout: Duration::from_secs(5),
        }
    }

    // ── Options parsing ──────────────────────────────────────────────────

    #[test]
    fn options_default_when_null() {
        let opts = PandocOptions::from_value(&Value::Null).unwrap();
        assert!(opts.mode.is_none());
        assert!(opts.target_format.is_none());
    }

    #[test]
    fn options_round_trip_filter_mode() {
        let v = serde_json::json!({
            "mode": "filter",
            "target_format": "latex",
        });
        let opts = PandocOptions::from_value(&v).unwrap();
        assert_eq!(opts.mode, Some(PandocMode::Filter));
        assert_eq!(opts.target_format.as_deref(), Some("latex"));
    }

    #[test]
    fn options_round_trip_native_mode() {
        let v = serde_json::json!({"mode": "native"});
        let opts = PandocOptions::from_value(&v).unwrap();
        assert_eq!(opts.mode, Some(PandocMode::Native));
    }

    // ── resolve_mode: filter default + pandoc-citeproc auto-promotion ────

    #[test]
    fn resolve_mode_defaults_to_filter() {
        let manifest = sample_manifest("/usr/local/bin/pandoc-crossref", vec![]);
        let invocation = resolve_mode(
            &manifest,
            &PandocOptions::default(),
            Path::new("/bin/pandoc"),
        );
        assert_eq!(invocation.mode, PandocMode::Filter);
        assert_eq!(
            invocation.executable,
            PathBuf::from("/usr/local/bin/pandoc-crossref")
        );
        assert_eq!(invocation.argv, vec!["html".to_string()]);
        assert!(invocation.diagnostics.is_empty());
    }

    #[test]
    fn resolve_mode_auto_promotes_pandoc_citeproc() {
        let manifest = sample_manifest("/usr/local/bin/pandoc-citeproc", vec![]);
        let invocation = resolve_mode(
            &manifest,
            &PandocOptions::default(),
            Path::new("/bin/pandoc"),
        );
        assert_eq!(invocation.mode, PandocMode::Native);
        assert_eq!(invocation.executable, PathBuf::from("/bin/pandoc"));
        assert!(
            invocation.argv.iter().any(|a| a == "--citeproc"),
            "expected --citeproc in argv, got {:?}",
            invocation.argv
        );
        assert!(
            invocation.argv.iter().any(|a| a == "json"),
            "native mode should use --from json --to json, got {:?}",
            invocation.argv
        );
        assert_eq!(invocation.diagnostics.len(), 1);
        assert!(invocation.diagnostics[0]
            .message
            .contains("pandoc-citeproc is deprecated"));
    }

    #[test]
    fn resolve_mode_explicit_filter_disables_auto_promotion() {
        let manifest = sample_manifest("/usr/local/bin/pandoc-citeproc", vec![]);
        let opts = PandocOptions {
            mode: Some(PandocMode::Filter),
            ..Default::default()
        };
        let invocation = resolve_mode(&manifest, &opts, Path::new("/bin/pandoc"));
        assert_eq!(invocation.mode, PandocMode::Filter);
        assert!(
            invocation.diagnostics.is_empty(),
            "explicit mode override should suppress auto-promotion warning"
        );
    }

    #[test]
    fn resolve_mode_native_routes_through_host_pandoc() {
        let manifest = sample_manifest("/irrelevant/pandoc-crossref", vec!["--something".into()]);
        let opts = PandocOptions {
            mode: Some(PandocMode::Native),
            ..Default::default()
        };
        let invocation = resolve_mode(&manifest, &opts, Path::new("/opt/pandoc"));
        assert_eq!(invocation.mode, PandocMode::Native);
        assert_eq!(invocation.executable, PathBuf::from("/opt/pandoc"));
        // --from json --to json prefix is always present so the transform
        // stage JSON-in/JSON-out contract holds.
        assert!(invocation
            .argv
            .windows(2)
            .any(|w| w == ["--from".to_string(), "json".to_string()]));
        assert!(invocation.argv.iter().any(|a| a == "--something"));
    }

    #[test]
    fn resolve_mode_custom_target_format() {
        let manifest = sample_manifest("/bin/filter", vec![]);
        let opts = PandocOptions {
            target_format: Some("latex".into()),
            ..Default::default()
        };
        let invocation = resolve_mode(&manifest, &opts, Path::new("/bin/pandoc"));
        assert_eq!(invocation.mode, PandocMode::Filter);
        assert_eq!(invocation.argv, vec!["latex".to_string()]);
    }

    // ── Env var formatting ───────────────────────────────────────────────

    #[test]
    fn api_version_formats_as_comma_separated() {
        assert_eq!(api_version_env_string(&[1, 23, 1, 0]), "1,23,1,0");
        assert_eq!(api_version_env_string(&[1, 22, 2, 1]), "1,22,2,1");
    }

    #[test]
    fn filter_env_carries_con_3303_variables() {
        let adapter = PandocAdapter {
            pandoc_version: Some("3.1.12.1".into()),
            ..PandocAdapter::default()
        };
        let manifest = sample_manifest("/usr/local/bin/pandoc-crossref", vec![]);
        let env = adapter.filter_env(&manifest, "html");
        let keys: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
        // CON-3303 explicitly lists these five variable names.
        for required in [
            "PANDOC_VERSION",
            "PANDOC_API_VERSION",
            "PANDOC_READER_OPTIONS",
            "PANDOC_WRITER_OPTIONS",
            "PANDOC_SCRIPT_FILE",
        ] {
            assert!(
                keys.contains(&required),
                "filter_env missing CON-3303 variable {required} (got: {keys:?})"
            );
        }
        let lookup = |name: &str| env.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
        assert_eq!(lookup("PANDOC_VERSION"), Some("3.1.12.1"));
        assert_eq!(lookup("PANDOC_API_VERSION"), Some("1,23,1,0"));
        assert_eq!(
            lookup("PANDOC_SCRIPT_FILE"),
            Some("/usr/local/bin/pandoc-crossref")
        );
    }

    #[test]
    fn adapter_id_and_ast_type_match_registry() {
        let adapter = PandocAdapter::default();
        assert_eq!(adapter.id(), "pandoc");
        assert_eq!(adapter.ast_type(), AstType::PandocExt);
        assert_eq!(adapter.supported_stages(), &[Stage::Transform]);
    }

    #[test]
    fn translate_round_trips_through_pandoc_ext() {
        let adapter = PandocAdapter::default();
        let fixtures = crate::ecosystems::default_fixtures();
        for fx in &fixtures {
            let foreign = adapter.translate_to_foreign(&fx.document).unwrap();
            let back = adapter.translate_from_foreign(foreign).unwrap();
            assert_eq!(
                crate::hooks::contract::canonicalise(&back),
                crate::hooks::contract::canonicalise(&fx.document),
                "pandoc translator lost fidelity on fixture '{}'",
                fx.name
            );
        }
    }

    #[test]
    fn invoke_plugin_rejects_non_transform_stage() {
        let mut adapter = PandocAdapter::default();
        let build = BuildContext::new(
            BuildMode::Build,
            PathBuf::from("/tmp/pandoc-test"),
            PageMeta::synthetic("Page", "page"),
        );
        let ctx = HookContext::new(Stage::PreParse, "test-pandoc", &build);
        let manifest = sample_manifest("/bin/cat", vec![]);
        let input = StageInput::PreParse {
            markdown: "body".into(),
        };
        match adapter.invoke_plugin(&manifest, input, &ctx) {
            PluginResponse::Error { reason, .. } => assert_eq!(reason, "unsupported_stage"),
            other => panic!("expected unsupported_stage error, got {other:?}"),
        }
    }

    #[test]
    fn invoke_plugin_reports_not_found_for_bogus_exec() {
        let mut adapter = PandocAdapter::default();
        let build = BuildContext::new(
            BuildMode::Build,
            PathBuf::from("/tmp/pandoc-test"),
            PageMeta::synthetic("Page", "page"),
        );
        let ctx = HookContext::new(Stage::Transform, "test-pandoc", &build);
        let manifest = sample_manifest("/definitely-not-a-real-binary-zetl-pandoc-test", vec![]);
        let input = StageInput::Transform {
            foreign: serde_json::json!({"pandoc-api-version": [1,23,1,0], "meta": {}, "blocks": []}),
        };
        match adapter.invoke_plugin(&manifest, input, &ctx) {
            PluginResponse::Error { reason, detail, .. } => {
                assert_eq!(reason, "runtime_absence");
                assert!(
                    detail.contains("not found")
                        || detail.contains("No such")
                        || detail.contains("pandoc"),
                    "expected NotFound-like detail, got: {detail}"
                );
            }
            other => panic!("expected runtime_absence error, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn invoke_plugin_filter_mode_echoes_through_shim() {
        // A no-op pandoc filter's contract is: read stdin, write stdout
        // unchanged. A tiny shell script that ignores argv and cats
        // stdin stands in for one. If the env/argv plumbing is broken
        // the JSON round-trip through the script will drop bytes and
        // the final parse will fail.
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let shim = tmp.path().join("pandoc-no-op-filter");
        {
            let mut f = std::fs::File::create(&shim).unwrap();
            writeln!(f, "#!/bin/sh").unwrap();
            writeln!(f, "# ignore argv[1..] (target format), identity-echo stdin").unwrap();
            writeln!(f, "cat").unwrap();
        }
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();

        let mut adapter = PandocAdapter::default();
        let build = BuildContext::new(
            BuildMode::Build,
            PathBuf::from("/tmp/pandoc-test"),
            PageMeta::synthetic("Page", "page"),
        );
        let ctx = HookContext::new(Stage::Transform, "test-pandoc", &build);
        let manifest = PluginManifest {
            extension_id: "test-pandoc".into(),
            executable: shim.clone(),
            args: vec![],
            options: serde_json::json!({"mode": "filter", "target_format": "html"}),
            timeout: Duration::from_secs(5),
        };
        let input_json = serde_json::json!({
            "pandoc-api-version": [1,23,1,0],
            "meta": {},
            "blocks": []
        });
        let input = StageInput::Transform {
            foreign: input_json.clone(),
        };
        let response = adapter.invoke_plugin(&manifest, input, &ctx);
        match response {
            PluginResponse::Success { output, .. } => match output {
                StageOutput::Transform { foreign } => {
                    assert_eq!(foreign, input_json);
                }
                other => panic!("expected Transform output, got {other:?}"),
            },
            PluginResponse::Error { reason, detail, .. } => {
                panic!("expected success through identity shim, got {reason}: {detail}")
            }
        }
    }
}
