//! Lifecycle hooks for zetl vault operations (SPEC-016).
//!
//! This module provides:
//! - **Discovery** (REQ-016-001): Scan `.zetl/hooks/` and theme `hooks/`
//!   directories for executable hook files matching recognised lifecycle points.
//! - **Execution** (REQ-016-003): Spawn hooks as child processes, write JSON
//!   context to stdin, set ZETL_* environment variables, capture output, and
//!   report exit codes.
//! - **Context** (CON-016-001): Serialise vault data to the JSON schema
//!   written to hook stdin.

pub mod ast;
pub mod build_context;
pub mod build_data;
pub mod composition;
pub mod context;
pub mod diagnostic;
pub mod failure_scoping;
pub mod observability;
pub mod persistent;
pub mod pipeline;
pub mod template_vars;
pub mod translators;

use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Default timeout for hook execution in seconds.
pub const HOOK_TIMEOUT_SECS: u64 = 30;

/// Recognised hook lifecycle points (REQ-016-002).
pub const HOOK_NAMES: &[&str] = &[
    "pre-build",
    "post-build",
    "post-index",
    "post-check",
    "on-save",
    "pre-serve",
    "on-agent",
    "on-access-request",
    "on-acl-violation",
];

/// Where a discovered hook originated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookSource {
    /// Hook from the active theme's `hooks/` directory.
    Theme,
    /// Hook from `.zetl/hooks/`.
    Vault,
}

impl std::fmt::Display for HookSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HookSource::Theme => write!(f, "theme"),
            HookSource::Vault => write!(f, "vault"),
        }
    }
}

/// A single discovered hook file.
#[derive(Debug, Clone)]
pub struct DiscoveredHook {
    /// Lifecycle point name (e.g. `"post-build"`).
    pub name: String,
    /// Where this hook came from.
    pub source: HookSource,
    /// Absolute path to the hook file.
    pub path: PathBuf,
    /// Whether the file has executable permission (`+x`).
    pub executable: bool,
}

/// Result of scanning all hook directories.
#[derive(Debug, Clone)]
pub struct HookManifest {
    /// All discovered hooks, ordered: theme hooks first, then vault hooks.
    pub hooks: Vec<DiscoveredHook>,
    /// Warnings produced during discovery (e.g. non-executable hooks).
    pub warnings: Vec<String>,
}

/// Discover hooks from theme and vault directories.
///
/// Scans two locations in order (REQ-016-001):
/// 1. **Theme hooks:** `<theme_hooks_dir>/<hook-name>` (if provided)
/// 2. **Vault hooks:** `<vault_root>/.zetl/hooks/<hook-name>`
///
/// Only files whose name matches a recognised lifecycle point are considered.
/// Unrecognised names are silently ignored (REQ-016-002).
///
/// `theme_hooks_dir` is `None` when no theme is active or the theme has no
/// `hooks/` directory on disk. Callers resolve the theme hooks path before
/// calling this function (accounting for bundled vs. disk-installed themes).
pub fn discover_hooks(vault_root: &Path, theme_hooks_dir: Option<&Path>) -> HookManifest {
    discover_hooks_verbose(vault_root, theme_hooks_dir, false)
}

/// Like [`discover_hooks`], but with verbose logging to stderr when `verbose` is true.
///
/// Logs which directories are checked, hooks found, and hooks skipped with reasons.
pub fn discover_hooks_verbose(
    vault_root: &Path,
    theme_hooks_dir: Option<&Path>,
    verbose: bool,
) -> HookManifest {
    let mut hooks = Vec::new();
    let mut warnings = Vec::new();

    // 1. Theme hooks
    if let Some(dir) = theme_hooks_dir {
        if verbose {
            eprintln!("[hooks] checking theme hooks dir: {}", dir.display());
        }
        scan_hooks_dir(dir, HookSource::Theme, &mut hooks, &mut warnings, verbose);
    } else if verbose {
        eprintln!("[hooks] no theme hooks dir configured");
    }

    // 2. Vault hooks
    let vault_hooks_dir = vault_root.join(".zetl").join("hooks");
    if verbose {
        eprintln!(
            "[hooks] checking vault hooks dir: {}",
            vault_hooks_dir.display()
        );
    }
    scan_hooks_dir(
        &vault_hooks_dir,
        HookSource::Vault,
        &mut hooks,
        &mut warnings,
        verbose,
    );

    if verbose {
        let executable_count = hooks.iter().filter(|h| h.executable).count();
        let skipped_count = hooks.iter().filter(|h| !h.executable).count();
        eprintln!("[hooks] discovery complete: {} hook(s) found, {} executable, {} skipped (not executable)",
            hooks.len(), executable_count, skipped_count);
    }

    HookManifest { hooks, warnings }
}

/// Resolved theme hooks directory.
///
/// Wraps an optional path to the hooks directory. When hooks come from a
/// bundled theme, the directory is a temporary extraction that lives as long
/// as this struct.
pub struct ThemeHooksDir {
    path: Option<PathBuf>,
    /// Keeps the temporary directory alive for bundled hook extractions.
    _temp: Option<tempfile::TempDir>,
}

impl ThemeHooksDir {
    /// Returns the path to the hooks directory, if any.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}

/// Resolve the theme hooks directory, checking disk-installed themes first,
/// then falling back to bundled themes.
///
/// Resolution order:
/// 1. **Disk-installed:** `.zetl/themes/<name>/hooks/` — if this directory
///    exists on disk, it is used directly.
/// 2. **Bundled (cached):** If the theme is compiled into the binary and has
///    hook files, they are extracted to `.zetl/cache/hooks/<theme>/` with
///    executable permissions. The cache is refreshed when the zetl binary
///    version changes.
/// 3. **Bundled (temp fallback):** If the persistent cache cannot be written,
///    hooks are extracted to a temporary directory instead.
///
/// The returned [`ThemeHooksDir`] must be kept alive until hook execution is
/// complete (it may own a temporary extraction directory as fallback).
pub fn resolve_theme_hooks(vault_root: &Path, theme: &str) -> ThemeHooksDir {
    resolve_theme_hooks_versioned(vault_root, theme, env!("CARGO_PKG_VERSION"))
}

/// Internal version of [`resolve_theme_hooks`] that accepts an explicit version
/// string (for testability).
fn resolve_theme_hooks_versioned(
    vault_root: &Path,
    theme: &str,
    zetl_version: &str,
) -> ThemeHooksDir {
    // 1. Disk-installed theme hooks
    if let Some(dir) = resolve_theme_hooks_dir(vault_root, theme) {
        return ThemeHooksDir {
            path: Some(dir),
            _temp: None,
        };
    }

    // 2. Bundled theme hooks
    let bundled_hooks = crate::web::engine::bundled_theme_hook_files(theme);
    if bundled_hooks.is_empty() {
        return ThemeHooksDir {
            path: None,
            _temp: None,
        };
    }

    // Try persistent cache first
    match extract_bundled_hooks_cached(vault_root, theme, &bundled_hooks, zetl_version) {
        Ok(path) => ThemeHooksDir {
            path: Some(path),
            _temp: None,
        },
        // Fall back to temp extraction if cache directory is not writable
        Err(_) => match extract_bundled_hooks(&bundled_hooks) {
            Ok((path, temp)) => ThemeHooksDir {
                path: Some(path),
                _temp: Some(temp),
            },
            Err(_) => ThemeHooksDir {
                path: None,
                _temp: None,
            },
        },
    }
}

/// Resolve the theme hooks directory path, if it exists on disk.
///
/// For disk-installed themes: `.zetl/themes/<name>/hooks/`
/// Returns `None` if the directory does not exist.
///
/// Prefer [`resolve_theme_hooks`] which also handles bundled themes.
pub fn resolve_theme_hooks_dir(vault_root: &Path, theme: &str) -> Option<PathBuf> {
    let dir = vault_root
        .join(".zetl")
        .join("themes")
        .join(theme)
        .join("hooks");
    if dir.is_dir() {
        Some(dir)
    } else {
        None
    }
}

/// Discover hooks for a single lifecycle point name.
///
/// Returns hooks matching `hook_name` from the manifest, preserving order
/// (theme first, then vault).
pub fn hooks_for<'a>(manifest: &'a HookManifest, hook_name: &str) -> Vec<&'a DiscoveredHook> {
    manifest
        .hooks
        .iter()
        .filter(|h| h.name == hook_name && h.executable)
        .collect()
}

// ── Hook Execution (REQ-016-003, REQ-016-004, REQ-016-005) ──────────────────

/// Environment context passed to hook processes.
///
/// The executor sets `ZETL_HOOK` automatically from the hook's name.
/// Callers provide vault-level variables and any hook-specific extras
/// (e.g. `ZETL_OUT_DIR` for build hooks, `ZETL_SAVED_FILE` for on-save).
pub struct HookEnv {
    /// Absolute path to the vault root (working directory + `ZETL_VAULT_ROOT`).
    pub vault_root: PathBuf,
    /// Active theme name (`ZETL_THEME`). Empty string if no theme.
    pub theme: String,
    /// zetl version string (`ZETL_VERSION`).
    pub zetl_version: String,
    /// Additional hook-specific environment variables.
    pub extra_vars: Vec<(String, String)>,
}

/// Result of executing a single hook process (REQ-016-003).
#[derive(Debug)]
pub struct HookOutput {
    /// Hook name that was executed.
    pub hook_name: String,
    /// Where the hook came from (theme or vault).
    pub source: HookSource,
    /// Absolute path to the hook that was executed.
    pub path: PathBuf,
    /// Process exit code (`None` if killed by signal).
    pub exit_code: Option<i32>,
    /// Captured stdout from the hook.
    pub stdout: String,
    /// Captured stderr from the hook.
    pub stderr: String,
    /// Wall-clock execution duration.
    pub duration: Duration,
    /// Whether the hook was killed due to exceeding the timeout.
    pub timed_out: bool,
}

impl HookOutput {
    /// Whether the hook exited successfully (exit code 0, not timed out).
    pub fn success(&self) -> bool {
        self.exit_code == Some(0) && !self.timed_out
    }
}

/// Execute a single discovered hook (CON-016-002).
///
/// Spawns the hook as a child process with:
/// - Working directory set to `env.vault_root`
/// - `ZETL_HOOK` set to the hook's name
/// - `ZETL_VAULT_ROOT`, `ZETL_THEME`, `ZETL_VERSION` from `env`
/// - Any additional variables from `env.extra_vars`
/// - `context_json` written to stdin (then stdin closed)
///
/// Returns the captured output regardless of exit code.
/// The caller decides whether to abort (pre-hooks) or warn (post-hooks)
/// per REQ-016-004.
pub fn execute_hook(
    hook: &DiscoveredHook,
    context_json: &[u8],
    env: &HookEnv,
) -> Result<HookOutput, std::io::Error> {
    execute_hook_verbose(hook, context_json, env, false)
}

/// Like [`execute_hook`], but with verbose logging to stderr when `verbose` is true.
///
/// Logs hook name, source, path, duration, exit code, and captured stdout/stderr.
/// Enforces a timeout of [`HOOK_TIMEOUT_SECS`] seconds — if the hook exceeds
/// this limit, the process is killed, a warning is logged to stderr, and the
/// result is returned with `timed_out = true` so the parent operation can continue.
pub fn execute_hook_verbose(
    hook: &DiscoveredHook,
    context_json: &[u8],
    env: &HookEnv,
    verbose: bool,
) -> Result<HookOutput, std::io::Error> {
    execute_hook_with_timeout(
        hook,
        context_json,
        env,
        verbose,
        Duration::from_secs(HOOK_TIMEOUT_SECS),
    )
}

/// Core hook executor with configurable timeout.
fn execute_hook_with_timeout(
    hook: &DiscoveredHook,
    context_json: &[u8],
    env: &HookEnv,
    verbose: bool,
    timeout: Duration,
) -> Result<HookOutput, std::io::Error> {
    if verbose {
        eprintln!(
            "[hooks] executing '{}' (source: {}, path: {})",
            hook.name,
            hook.source,
            hook.path.display()
        );
    }

    let start = Instant::now();
    let timeout_secs = timeout.as_secs();

    // Compute hook depth: read current ZETL_HOOK_DEPTH from env (or extra_vars),
    // default to 0, then increment for the child process (REQ-020-020).
    let current_depth: u32 = env
        .extra_vars
        .iter()
        .find(|(k, _)| k == "ZETL_HOOK_DEPTH")
        .and_then(|(_, v)| v.parse().ok())
        .or_else(|| std::env::var("ZETL_HOOK_DEPTH").ok()?.parse().ok())
        .unwrap_or(0);
    let child_depth = current_depth.saturating_add(1);

    let mut cmd = Command::new(&hook.path);
    cmd.current_dir(&env.vault_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("ZETL_HOOK", &hook.name)
        .env("ZETL_VAULT_ROOT", &env.vault_root)
        .env("ZETL_THEME", &env.theme)
        .env("ZETL_VERSION", &env.zetl_version)
        .env("ZETL_HOOK_DEPTH", child_depth.to_string());

    for (key, val) in &env.extra_vars {
        if key == "ZETL_HOOK_DEPTH" {
            continue; // Already set above with incremented value
        }
        cmd.env(key, val);
    }

    // Retry on ETXTBSY ("Text file busy"): when a hook script is written
    // and immediately executed — common in tests and in tools that
    // generate hooks programmatically — Linux can reject the `execve`
    // for up to a few milliseconds because another cargo-test thread's
    // concurrent `fork()` inherited a still-open write fd to the script.
    // See rust-lang/rust#114554.
    let mut child = {
        let mut attempt: u32 = 0;
        loop {
            match cmd.spawn() {
                Ok(c) => break c,
                Err(e) if e.raw_os_error() == Some(26) && attempt < 20 => {
                    attempt += 1;
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => return Err(e),
            }
        }
    };

    // Write context JSON to stdin on a background thread so the timeout
    // polling loop below is not blocked when the hook does not consume stdin
    // (e.g. a large context written to a long-running/sleeping process).
    let stdin_handle = child.stdin.take();
    let context_owned = context_json.to_vec();
    let stdin_thread = std::thread::spawn(move || {
        if let Some(mut stdin) = stdin_handle {
            // Ignore write errors — the hook may have exited early.
            let _ = stdin.write_all(&context_owned);
        }
        // stdin is dropped here, closing the pipe.
    });

    // Take stdout/stderr handles so reader threads can drain them in parallel
    // (prevents pipe-buffer deadlock if the hook produces large output).
    let stdout_handle = child.stdout.take();
    let stderr_handle = child.stderr.take();

    let stdout_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut h) = stdout_handle {
            let _ = h.read_to_end(&mut buf);
        }
        buf
    });
    let stderr_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut h) = stderr_handle {
            let _ = h.read_to_end(&mut buf);
        }
        buf
    });

    // Poll for exit with timeout.
    let poll_interval = Duration::from_millis(100);
    let mut timed_out = false;

    let status = loop {
        match child.try_wait()? {
            Some(status) => break status,
            None => {
                if start.elapsed() >= timeout {
                    timed_out = true;
                    let _ = child.kill();
                    break child.wait()?;
                }
                std::thread::sleep(poll_interval);
            }
        }
    };

    // Join the stdin writer — it will unblock once the child is killed/exited
    // because the pipe's read end is closed.
    let _ = stdin_thread.join();

    let stdout_bytes = stdout_thread.join().unwrap_or_default();
    let stderr_bytes = stderr_thread.join().unwrap_or_default();
    let duration = start.elapsed();

    if timed_out {
        eprintln!(
            "warning: hook '{}' exceeded {}s timeout and was killed (source: {}, path: {})",
            hook.name,
            timeout_secs,
            hook.source,
            hook.path.display()
        );
    }

    let result = HookOutput {
        hook_name: hook.name.clone(),
        source: hook.source.clone(),
        path: hook.path.clone(),
        exit_code: if timed_out { None } else { status.code() },
        stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
        stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
        duration,
        timed_out,
    };

    if verbose {
        let code_str = if result.timed_out {
            "timeout".to_string()
        } else {
            match result.exit_code {
                Some(c) => c.to_string(),
                None => "killed".to_string(),
            }
        };
        eprintln!(
            "[hooks] '{}' finished: exit_code={}, duration={:.1}ms",
            result.hook_name,
            code_str,
            result.duration.as_secs_f64() * 1000.0
        );
        if !result.stdout.is_empty() {
            eprintln!(
                "[hooks] '{}' stdout: {}",
                result.hook_name,
                result.stdout.trim_end()
            );
        }
        if !result.stderr.is_empty() {
            eprintln!(
                "[hooks] '{}' stderr: {}",
                result.hook_name,
                result.stderr.trim_end()
            );
        }
    }

    Ok(result)
}

/// Execute all hooks for a lifecycle point sequentially (REQ-016-003).
///
/// Discovers executable hooks matching `hook_name` (theme first, then vault)
/// and runs them in order. Returns a result for each hook.
pub fn run_hooks(
    manifest: &HookManifest,
    hook_name: &str,
    context_json: &[u8],
    env: &HookEnv,
) -> Vec<Result<HookOutput, std::io::Error>> {
    run_hooks_verbose(manifest, hook_name, context_json, env, false)
}

/// Like [`run_hooks`], but with verbose logging to stderr when `verbose` is true.
pub fn run_hooks_verbose(
    manifest: &HookManifest,
    hook_name: &str,
    context_json: &[u8],
    env: &HookEnv,
    verbose: bool,
) -> Vec<Result<HookOutput, std::io::Error>> {
    let matching = hooks_for(manifest, hook_name);
    if verbose {
        eprintln!(
            "[hooks] running lifecycle point '{}': {} hook(s) matched",
            hook_name,
            matching.len()
        );
    }
    // Pre-hooks are abort-capable: stop after the first failure so later
    // hooks don't execute and mutate state.  Post/on-* hooks are non-fatal
    // and must always run all matches (callers warn-and-continue).
    let fail_fast = hook_name.starts_with("pre-");

    let mut results = Vec::with_capacity(matching.len());
    for hook in &matching {
        let result = execute_hook_verbose(hook, context_json, env, verbose);
        let failed = match &result {
            Ok(output) => !output.success(),
            Err(_) => true,
        };
        results.push(result);
        if fail_fast && failed {
            break;
        }
    }
    results
}

// ── Internal helpers ─────────────────────────────────────────────────────────

/// Extract bundled hook files to a temporary directory with executable permissions.
///
/// Returns `(hooks_dir_path, temp_dir_handle)`. The `TempDir` must be kept alive
/// until hook execution is complete.
fn extract_bundled_hooks(
    hook_files: &[(String, Vec<u8>)],
) -> Result<(PathBuf, tempfile::TempDir), std::io::Error> {
    let temp = tempfile::TempDir::new()?;
    let hooks_dir = temp.path().to_path_buf();

    for (name, content) in hook_files {
        let path = hooks_dir.join(name);
        std::fs::write(&path, content)?;
        // Set executable permission (Unix-only; Windows uses file extensions)
        #[cfg(unix)]
        {
            let mut perms = std::fs::metadata(&path)?.permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&path, perms)?;
        }
    }

    Ok((hooks_dir, temp))
}

/// Extract bundled hook files to a persistent cache directory with version tracking.
///
/// Cache location: `<vault_root>/.zetl/cache/hooks/<theme>/`
/// Version stamp: `<cache_dir>/.zetl_version`
///
/// If the cache directory exists and its version stamp matches `zetl_version`,
/// the existing cache is reused without re-extraction. Otherwise the cache is
/// cleared, hooks are re-written with executable permissions (`0o755`), and a
/// fresh version stamp is written.
fn extract_bundled_hooks_cached(
    vault_root: &Path,
    theme: &str,
    hook_files: &[(String, Vec<u8>)],
    zetl_version: &str,
) -> Result<PathBuf, std::io::Error> {
    let cache_dir = vault_root
        .join(".zetl")
        .join("cache")
        .join("hooks")
        .join(theme);
    let version_file = cache_dir.join(".zetl_version");

    // Check if cache is fresh
    if cache_dir.is_dir() {
        if let Ok(cached_version) = std::fs::read_to_string(&version_file) {
            if cached_version.trim() == zetl_version {
                return Ok(cache_dir);
            }
        }
    }

    // Cache is stale or missing — refresh
    if cache_dir.exists() {
        std::fs::remove_dir_all(&cache_dir)?;
    }
    std::fs::create_dir_all(&cache_dir)?;

    for (name, content) in hook_files {
        let path = cache_dir.join(name);
        std::fs::write(&path, content)?;
        #[cfg(unix)]
        {
            let mut perms = std::fs::metadata(&path)?.permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&path, perms)?;
        }
    }

    // Write version stamp
    std::fs::write(&version_file, zetl_version)?;

    Ok(cache_dir)
}

/// Scan a single hooks directory and append discovered hooks.
fn scan_hooks_dir(
    dir: &Path,
    source: HookSource,
    hooks: &mut Vec<DiscoveredHook>,
    warnings: &mut Vec<String>,
    verbose: bool,
) {
    if !dir.is_dir() {
        if verbose {
            eprintln!("[hooks] directory does not exist: {}", dir.display());
        }
        return;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            if verbose {
                eprintln!("[hooks] cannot read directory {}: {}", dir.display(), e);
            }
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();

        // Skip directories and non-files
        if !path.is_file() {
            continue;
        }

        let file_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(name) => name.to_string(),
            None => continue,
        };

        // Only recognised hook names (REQ-016-002)
        if !HOOK_NAMES.contains(&file_name.as_str()) {
            if verbose {
                eprintln!("[hooks] skipping '{file_name}': not a recognised hook name");
            }
            continue;
        }

        let executable = is_executable(&path);

        if !executable {
            if verbose {
                eprintln!("[hooks] found '{file_name}' ({source}) but not executable — skipping");
            }
            warnings.push(format!(
                "hook '{}' is not executable: {}\nhint: chmod +x {}",
                file_name,
                path.display(),
                path.display(),
            ));
        } else if verbose {
            eprintln!(
                "[hooks] found '{}' (source: {}, path: {})",
                file_name,
                source,
                path.display()
            );
        }

        hooks.push(DiscoveredHook {
            name: file_name,
            source: source.clone(),
            path,
            executable,
        });
    }

    // Sort hooks within this source by name for deterministic order
    let start = hooks.len().saturating_sub(
        hooks
            .iter()
            .rev()
            .take_while(|h| h.source == source)
            .count(),
    );
    hooks[start..].sort_by(|a, b| a.name.cmp(&b.name));
}

/// Check if a file has the executable bit set (Unix).
#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(meta) => meta.permissions().mode() & 0o111 != 0,
        Err(_) => false,
    }
}

#[cfg(windows)]
fn is_executable(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => matches!(
            ext.to_ascii_lowercase().as_str(),
            "exe" | "bat" | "cmd" | "com" | "ps1"
        ),
        None => false,
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Helper: create a file and optionally make it executable.
    fn create_hook(dir: &Path, name: &str, executable: bool) {
        let path = dir.join(name);
        fs::write(&path, "#!/bin/sh\necho ok").unwrap();
        if executable {
            let mut perms = fs::metadata(&path).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&path, perms).unwrap();
        } else {
            let mut perms = fs::metadata(&path).unwrap().permissions();
            perms.set_mode(0o644);
            fs::set_permissions(&path, perms).unwrap();
        }
    }

    #[test]
    fn discover_vault_hooks() {
        let tmp = TempDir::new().unwrap();
        let hooks_dir = tmp.path().join(".zetl").join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();

        create_hook(&hooks_dir, "post-build", true);
        create_hook(&hooks_dir, "on-save", true);

        let manifest = discover_hooks(tmp.path(), None);
        assert_eq!(manifest.hooks.len(), 2);
        assert!(manifest.warnings.is_empty());
        assert_eq!(manifest.hooks[0].name, "on-save");
        assert_eq!(manifest.hooks[0].source, HookSource::Vault);
        assert_eq!(manifest.hooks[1].name, "post-build");
        assert_eq!(manifest.hooks[1].source, HookSource::Vault);
    }

    #[test]
    fn warn_non_executable() {
        let tmp = TempDir::new().unwrap();
        let hooks_dir = tmp.path().join(".zetl").join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();

        create_hook(&hooks_dir, "post-build", false);

        let manifest = discover_hooks(tmp.path(), None);
        assert_eq!(manifest.hooks.len(), 1);
        assert!(!manifest.hooks[0].executable);
        assert_eq!(manifest.warnings.len(), 1);
        assert!(manifest.warnings[0].contains("chmod +x"));
        assert!(manifest.warnings[0].contains("post-build"));
    }

    #[test]
    fn ignore_unrecognised_names() {
        let tmp = TempDir::new().unwrap();
        let hooks_dir = tmp.path().join(".zetl").join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();

        create_hook(&hooks_dir, "post-build", true);
        create_hook(&hooks_dir, "README.md", true);
        create_hook(&hooks_dir, "my-custom-thing", true);

        let manifest = discover_hooks(tmp.path(), None);
        assert_eq!(manifest.hooks.len(), 1);
        assert_eq!(manifest.hooks[0].name, "post-build");
    }

    #[test]
    fn no_hooks_directory() {
        let tmp = TempDir::new().unwrap();
        let manifest = discover_hooks(tmp.path(), None);
        assert!(manifest.hooks.is_empty());
        assert!(manifest.warnings.is_empty());
    }

    #[test]
    fn theme_hooks_before_vault_hooks() {
        let tmp = TempDir::new().unwrap();

        // Set up vault hooks
        let vault_hooks = tmp.path().join(".zetl").join("hooks");
        fs::create_dir_all(&vault_hooks).unwrap();
        create_hook(&vault_hooks, "post-build", true);

        // Set up theme hooks
        let theme_hooks = tmp
            .path()
            .join(".zetl")
            .join("themes")
            .join("fountain")
            .join("hooks");
        fs::create_dir_all(&theme_hooks).unwrap();
        create_hook(&theme_hooks, "post-build", true);

        let manifest = discover_hooks(tmp.path(), Some(&theme_hooks));
        assert_eq!(manifest.hooks.len(), 2);
        // Theme hooks come first
        assert_eq!(manifest.hooks[0].source, HookSource::Theme);
        assert_eq!(manifest.hooks[1].source, HookSource::Vault);
    }

    #[test]
    fn hooks_for_filters_by_name() {
        let tmp = TempDir::new().unwrap();
        let hooks_dir = tmp.path().join(".zetl").join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();

        create_hook(&hooks_dir, "post-build", true);
        create_hook(&hooks_dir, "on-save", true);
        create_hook(&hooks_dir, "pre-build", false); // not executable

        let manifest = discover_hooks(tmp.path(), None);
        let post_build = hooks_for(&manifest, "post-build");
        assert_eq!(post_build.len(), 1);
        assert_eq!(post_build[0].name, "post-build");

        // pre-build exists but is not executable, so hooks_for skips it
        let pre_build = hooks_for(&manifest, "pre-build");
        assert_eq!(pre_build.len(), 0);
    }

    #[test]
    fn resolve_theme_hooks_dir_exists() {
        let tmp = TempDir::new().unwrap();
        let theme_hooks = tmp
            .path()
            .join(".zetl")
            .join("themes")
            .join("fountain")
            .join("hooks");
        fs::create_dir_all(&theme_hooks).unwrap();

        let result = resolve_theme_hooks_dir(tmp.path(), "fountain");
        assert_eq!(result, Some(theme_hooks));
    }

    #[test]
    fn resolve_theme_hooks_dir_missing() {
        let tmp = TempDir::new().unwrap();
        let result = resolve_theme_hooks_dir(tmp.path(), "nonexistent");
        assert_eq!(result, None);
    }

    #[test]
    fn all_hook_names_discovered() {
        let tmp = TempDir::new().unwrap();
        let hooks_dir = tmp.path().join(".zetl").join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();

        for name in HOOK_NAMES {
            create_hook(&hooks_dir, name, true);
        }

        let manifest = discover_hooks(tmp.path(), None);
        assert_eq!(manifest.hooks.len(), HOOK_NAMES.len());
        assert!(manifest.warnings.is_empty());
    }

    #[test]
    fn hook_source_display() {
        assert_eq!(HookSource::Theme.to_string(), "theme");
        assert_eq!(HookSource::Vault.to_string(), "vault");
    }

    #[test]
    fn skip_directories_in_hooks_dir() {
        let tmp = TempDir::new().unwrap();
        let hooks_dir = tmp.path().join(".zetl").join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();

        // Create a subdirectory named like a hook
        fs::create_dir_all(hooks_dir.join("post-build")).unwrap();
        // Create an actual hook file
        create_hook(&hooks_dir, "on-save", true);

        let manifest = discover_hooks(tmp.path(), None);
        assert_eq!(manifest.hooks.len(), 1);
        assert_eq!(manifest.hooks[0].name, "on-save");
    }

    // ── Executor tests (REQ-016-003, REQ-016-004, REQ-016-005) ─────────────

    /// Helper: create a hook script with custom content and make it executable.
    fn create_script(dir: &Path, name: &str, script: &str) {
        let path = dir.join(name);
        fs::write(&path, script).unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).unwrap();
    }

    fn test_env(vault_root: &Path) -> HookEnv {
        HookEnv {
            vault_root: vault_root.to_path_buf(),
            theme: "test-theme".to_string(),
            zetl_version: "0.1.0-test".to_string(),
            extra_vars: vec![],
        }
    }

    #[test]
    fn executor_receives_json_on_stdin() {
        let tmp = TempDir::new().unwrap();
        let hooks_dir = tmp.path().join(".zetl").join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();

        // Hook that reads stdin and writes it to a file
        let out_file = tmp.path().join("stdin_capture.json");
        create_script(
            &hooks_dir,
            "post-build",
            &format!("#!/bin/sh\ncat > '{}'\n", out_file.display()),
        );

        let manifest = discover_hooks(tmp.path(), None);
        let hook = &manifest.hooks[0];
        let context = br#"{"hook":"post-build","vault_root":"/tmp"}"#;

        let result = execute_hook(hook, context, &test_env(tmp.path())).unwrap();
        assert!(result.success());

        let captured = fs::read_to_string(&out_file).unwrap();
        assert_eq!(captured, r#"{"hook":"post-build","vault_root":"/tmp"}"#);
    }

    #[test]
    fn executor_sets_working_directory() {
        let tmp = TempDir::new().unwrap();
        let hooks_dir = tmp.path().join(".zetl").join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();

        // Hook that prints pwd
        create_script(&hooks_dir, "post-build", "#!/bin/sh\npwd\n");

        let manifest = discover_hooks(tmp.path(), None);
        let hook = &manifest.hooks[0];

        let result = execute_hook(hook, b"{}", &test_env(tmp.path())).unwrap();
        assert!(result.success());

        // Resolve symlinks for macOS /private/var/... vs /var/...
        let expected = fs::canonicalize(tmp.path()).unwrap();
        let actual = PathBuf::from(result.stdout.trim());
        let actual = fs::canonicalize(&actual).unwrap_or(actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn executor_sets_zetl_env_vars() {
        let tmp = TempDir::new().unwrap();
        let hooks_dir = tmp.path().join(".zetl").join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();

        create_script(
            &hooks_dir,
            "post-build",
            "#!/bin/sh\necho \"HOOK=$ZETL_HOOK\"\necho \"THEME=$ZETL_THEME\"\necho \"VERSION=$ZETL_VERSION\"\n",
        );

        let manifest = discover_hooks(tmp.path(), None);
        let hook = &manifest.hooks[0];

        let result = execute_hook(hook, b"{}", &test_env(tmp.path())).unwrap();
        assert!(result.success());
        assert!(result.stdout.contains("HOOK=post-build"));
        assert!(result.stdout.contains("THEME=test-theme"));
        assert!(result.stdout.contains("VERSION=0.1.0-test"));
    }

    #[test]
    fn executor_sets_vault_root_env() {
        let tmp = TempDir::new().unwrap();
        let hooks_dir = tmp.path().join(".zetl").join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();

        create_script(
            &hooks_dir,
            "post-build",
            "#!/bin/sh\necho \"$ZETL_VAULT_ROOT\"\n",
        );

        let manifest = discover_hooks(tmp.path(), None);
        let hook = &manifest.hooks[0];

        let result = execute_hook(hook, b"{}", &test_env(tmp.path())).unwrap();
        assert!(result.success());

        let expected = fs::canonicalize(tmp.path()).unwrap();
        let actual = PathBuf::from(result.stdout.trim());
        let actual = fs::canonicalize(&actual).unwrap_or(actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn executor_sets_extra_env_vars() {
        let tmp = TempDir::new().unwrap();
        let hooks_dir = tmp.path().join(".zetl").join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();

        create_script(
            &hooks_dir,
            "post-build",
            "#!/bin/sh\necho \"OUTDIR=$ZETL_OUT_DIR\"\n",
        );

        let manifest = discover_hooks(tmp.path(), None);
        let hook = &manifest.hooks[0];

        let mut env = test_env(tmp.path());
        env.extra_vars
            .push(("ZETL_OUT_DIR".to_string(), "/tmp/dist".to_string()));

        let result = execute_hook(hook, b"{}", &env).unwrap();
        assert!(result.success());
        assert!(result.stdout.contains("OUTDIR=/tmp/dist"));
    }

    #[test]
    fn executor_captures_exit_code_zero() {
        let tmp = TempDir::new().unwrap();
        let hooks_dir = tmp.path().join(".zetl").join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();

        create_script(&hooks_dir, "post-build", "#!/bin/sh\nexit 0\n");

        let manifest = discover_hooks(tmp.path(), None);
        let hook = &manifest.hooks[0];

        let result = execute_hook(hook, b"{}", &test_env(tmp.path())).unwrap();
        assert!(result.success());
        assert_eq!(result.exit_code, Some(0));
    }

    #[test]
    fn executor_captures_nonzero_exit() {
        let tmp = TempDir::new().unwrap();
        let hooks_dir = tmp.path().join(".zetl").join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();

        create_script(
            &hooks_dir,
            "post-build",
            "#!/bin/sh\necho 'hook error' >&2\nexit 1\n",
        );

        let manifest = discover_hooks(tmp.path(), None);
        let hook = &manifest.hooks[0];

        let result = execute_hook(hook, b"{}", &test_env(tmp.path())).unwrap();
        assert!(!result.success());
        assert_eq!(result.exit_code, Some(1));
        assert!(result.stderr.contains("hook error"));
    }

    #[test]
    fn executor_captures_stdout_and_stderr() {
        let tmp = TempDir::new().unwrap();
        let hooks_dir = tmp.path().join(".zetl").join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();

        create_script(
            &hooks_dir,
            "post-build",
            "#!/bin/sh\necho 'out message'\necho 'err message' >&2\n",
        );

        let manifest = discover_hooks(tmp.path(), None);
        let hook = &manifest.hooks[0];

        let result = execute_hook(hook, b"{}", &test_env(tmp.path())).unwrap();
        assert!(result.success());
        assert_eq!(result.stdout.trim(), "out message");
        assert_eq!(result.stderr.trim(), "err message");
    }

    #[test]
    fn executor_reports_duration() {
        let tmp = TempDir::new().unwrap();
        let hooks_dir = tmp.path().join(".zetl").join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();

        create_script(&hooks_dir, "post-build", "#!/bin/sh\ntrue\n");

        let manifest = discover_hooks(tmp.path(), None);
        let hook = &manifest.hooks[0];

        let result = execute_hook(hook, b"{}", &test_env(tmp.path())).unwrap();
        // Duration should be non-negative (process spawned and exited)
        assert!(result.duration.as_nanos() > 0);
    }

    #[test]
    fn executor_preserves_hook_metadata() {
        let tmp = TempDir::new().unwrap();
        let hooks_dir = tmp.path().join(".zetl").join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();

        create_script(&hooks_dir, "on-save", "#!/bin/sh\ntrue\n");

        let manifest = discover_hooks(tmp.path(), None);
        let hook = &manifest.hooks[0];

        let result = execute_hook(hook, b"{}", &test_env(tmp.path())).unwrap();
        assert_eq!(result.hook_name, "on-save");
        assert_eq!(result.source, HookSource::Vault);
        assert_eq!(result.path, hooks_dir.join("on-save"));
    }

    #[test]
    fn run_hooks_executes_matching_hooks() {
        let tmp = TempDir::new().unwrap();
        let hooks_dir = tmp.path().join(".zetl").join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();

        create_script(
            &hooks_dir,
            "post-build",
            "#!/bin/sh\necho 'post-build ran'\n",
        );
        create_script(&hooks_dir, "on-save", "#!/bin/sh\necho 'on-save ran'\n");

        let manifest = discover_hooks(tmp.path(), None);
        let results = run_hooks(&manifest, "post-build", b"{}", &test_env(tmp.path()));

        assert_eq!(results.len(), 1);
        let output = results[0].as_ref().unwrap();
        assert_eq!(output.hook_name, "post-build");
        assert!(output.stdout.contains("post-build ran"));
    }

    #[test]
    fn run_hooks_returns_empty_for_no_match() {
        let tmp = TempDir::new().unwrap();
        let hooks_dir = tmp.path().join(".zetl").join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();

        create_script(&hooks_dir, "post-build", "#!/bin/sh\ntrue\n");

        let manifest = discover_hooks(tmp.path(), None);
        let results = run_hooks(&manifest, "pre-serve", b"{}", &test_env(tmp.path()));
        assert!(results.is_empty());
    }

    #[test]
    fn run_hooks_theme_before_vault() {
        let tmp = TempDir::new().unwrap();

        // Vault hook
        let vault_hooks = tmp.path().join(".zetl").join("hooks");
        fs::create_dir_all(&vault_hooks).unwrap();
        create_script(&vault_hooks, "post-build", "#!/bin/sh\necho 'vault'\n");

        // Theme hook
        let theme_hooks = tmp.path().join("theme-hooks");
        fs::create_dir_all(&theme_hooks).unwrap();
        create_script(&theme_hooks, "post-build", "#!/bin/sh\necho 'theme'\n");

        let manifest = discover_hooks(tmp.path(), Some(&theme_hooks));
        let results = run_hooks(&manifest, "post-build", b"{}", &test_env(tmp.path()));

        assert_eq!(results.len(), 2);
        let first = results[0].as_ref().unwrap();
        let second = results[1].as_ref().unwrap();
        assert_eq!(first.source, HookSource::Theme);
        assert_eq!(second.source, HookSource::Vault);
        assert!(first.stdout.contains("theme"));
        assert!(second.stdout.contains("vault"));
    }

    // ── Verbose variant tests ──────────────────────────────────────────────

    #[test]
    fn discover_hooks_verbose_returns_same_results() {
        let tmp = TempDir::new().unwrap();
        let hooks_dir = tmp.path().join(".zetl").join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();

        create_hook(&hooks_dir, "post-build", true);
        create_hook(&hooks_dir, "on-save", true);
        create_hook(&hooks_dir, "pre-build", false); // not executable

        let quiet = discover_hooks(tmp.path(), None);
        let verbose = discover_hooks_verbose(tmp.path(), None, true);

        assert_eq!(quiet.hooks.len(), verbose.hooks.len());
        assert_eq!(quiet.warnings.len(), verbose.warnings.len());
        for (q, v) in quiet.hooks.iter().zip(verbose.hooks.iter()) {
            assert_eq!(q.name, v.name);
            assert_eq!(q.source, v.source);
            assert_eq!(q.executable, v.executable);
        }
    }

    #[test]
    fn discover_hooks_verbose_no_dir() {
        let tmp = TempDir::new().unwrap();
        // Should not panic with verbose=true when directories don't exist
        let manifest = discover_hooks_verbose(tmp.path(), None, true);
        assert!(manifest.hooks.is_empty());
    }

    #[test]
    fn execute_hook_verbose_returns_same_results() {
        let tmp = TempDir::new().unwrap();
        let hooks_dir = tmp.path().join(".zetl").join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();

        create_script(
            &hooks_dir,
            "post-build",
            "#!/bin/sh\necho 'hello'\necho 'err' >&2\nexit 0\n",
        );

        let manifest = discover_hooks(tmp.path(), None);
        let hook = &manifest.hooks[0];

        let result = execute_hook_verbose(hook, b"{}", &test_env(tmp.path()), true).unwrap();
        assert!(result.success());
        assert!(result.stdout.contains("hello"));
        assert!(result.stderr.contains("err"));
    }

    #[test]
    fn run_hooks_verbose_returns_same_results() {
        let tmp = TempDir::new().unwrap();
        let hooks_dir = tmp.path().join(".zetl").join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();

        create_script(&hooks_dir, "post-build", "#!/bin/sh\necho 'ran'\n");

        let manifest = discover_hooks(tmp.path(), None);
        let results =
            run_hooks_verbose(&manifest, "post-build", b"{}", &test_env(tmp.path()), true);
        assert_eq!(results.len(), 1);
        let output = results[0].as_ref().unwrap();
        assert!(output.stdout.contains("ran"));
    }

    // ── ThemeHooksDir / resolve_theme_hooks tests ───────────────────────

    #[test]
    fn resolve_theme_hooks_disk_installed() {
        let tmp = TempDir::new().unwrap();
        let theme_hooks = tmp
            .path()
            .join(".zetl")
            .join("themes")
            .join("my-theme")
            .join("hooks");
        fs::create_dir_all(&theme_hooks).unwrap();
        create_hook(&theme_hooks, "post-build", true);

        let resolved = resolve_theme_hooks(tmp.path(), "my-theme");
        assert!(resolved.path().is_some());
        assert_eq!(resolved.path().unwrap(), theme_hooks);
    }

    #[test]
    fn resolve_theme_hooks_no_theme() {
        let tmp = TempDir::new().unwrap();
        let resolved = resolve_theme_hooks(tmp.path(), "nonexistent");
        assert!(resolved.path().is_none());
    }

    #[test]
    fn resolve_theme_hooks_disk_priority_over_bundled() {
        // When a disk-installed theme has hooks/, it should be preferred.
        let tmp = TempDir::new().unwrap();
        let theme_hooks = tmp
            .path()
            .join(".zetl")
            .join("themes")
            .join("default")
            .join("hooks");
        fs::create_dir_all(&theme_hooks).unwrap();
        create_hook(&theme_hooks, "post-build", true);

        let resolved = resolve_theme_hooks(tmp.path(), "default");
        assert!(resolved.path().is_some());
        assert_eq!(resolved.path().unwrap(), theme_hooks);
    }

    #[test]
    fn resolve_theme_hooks_bundled_no_hooks() {
        // Bundled "default" theme has no hooks/ subdirectory.
        let tmp = TempDir::new().unwrap();
        let resolved = resolve_theme_hooks(tmp.path(), "default");
        assert!(resolved.path().is_none());
    }

    #[test]
    fn extract_bundled_hooks_creates_executable_files() {
        let hook_files = vec![
            ("post-build".to_string(), b"#!/bin/sh\necho ok\n".to_vec()),
            ("on-save".to_string(), b"#!/bin/sh\necho saved\n".to_vec()),
        ];

        let (dir, _temp) = extract_bundled_hooks(&hook_files).unwrap();

        // Both files should exist
        assert!(dir.join("post-build").is_file());
        assert!(dir.join("on-save").is_file());

        // Both should be executable
        assert!(is_executable(&dir.join("post-build")));
        assert!(is_executable(&dir.join("on-save")));

        // Content should match
        let content = fs::read_to_string(dir.join("post-build")).unwrap();
        assert_eq!(content, "#!/bin/sh\necho ok\n");
    }

    #[test]
    fn extracted_bundled_hooks_are_discoverable() {
        let hook_files = vec![("post-build".to_string(), b"#!/bin/sh\necho ok\n".to_vec())];

        let (dir, _temp) = extract_bundled_hooks(&hook_files).unwrap();

        // The extracted directory should work with discover_hooks
        let tmp = TempDir::new().unwrap();
        let manifest = discover_hooks(tmp.path(), Some(&dir));
        assert_eq!(manifest.hooks.len(), 1);
        assert_eq!(manifest.hooks[0].name, "post-build");
        assert_eq!(manifest.hooks[0].source, HookSource::Theme);
        assert!(manifest.hooks[0].executable);
    }

    #[test]
    fn theme_hooks_dir_path_lifetime() {
        // Ensure the TempDir stays alive as long as ThemeHooksDir
        let hook_files = vec![("post-build".to_string(), b"#!/bin/sh\necho ok\n".to_vec())];

        let (dir, temp) = extract_bundled_hooks(&hook_files).unwrap();
        let theme_hooks = ThemeHooksDir {
            path: Some(dir),
            _temp: Some(temp),
        };

        // Path should still be valid
        let path = theme_hooks.path().unwrap();
        assert!(path.join("post-build").is_file());
    }

    // ── Cached extraction tests ─────────────────────────────────────────

    #[test]
    fn extract_cached_creates_hooks_and_version_stamp() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".zetl")).unwrap();

        let hook_files = vec![
            ("post-build".to_string(), b"#!/bin/sh\necho ok\n".to_vec()),
            ("on-save".to_string(), b"#!/bin/sh\necho saved\n".to_vec()),
        ];

        let result = extract_bundled_hooks_cached(tmp.path(), "my-theme", &hook_files, "1.0.0");
        assert!(result.is_ok());

        let cache_dir = result.unwrap();
        assert_eq!(cache_dir, tmp.path().join(".zetl/cache/hooks/my-theme"));
        assert!(cache_dir.join("post-build").is_file());
        assert!(cache_dir.join("on-save").is_file());
        assert!(cache_dir.join(".zetl_version").is_file());

        let stamp = fs::read_to_string(cache_dir.join(".zetl_version")).unwrap();
        assert_eq!(stamp, "1.0.0");

        let content = fs::read_to_string(cache_dir.join("post-build")).unwrap();
        assert_eq!(content, "#!/bin/sh\necho ok\n");
    }

    #[test]
    fn extract_cached_hooks_are_executable() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".zetl")).unwrap();

        let hook_files = vec![("post-build".to_string(), b"#!/bin/sh\necho ok\n".to_vec())];

        let cache_dir =
            extract_bundled_hooks_cached(tmp.path(), "test", &hook_files, "1.0.0").unwrap();

        assert!(is_executable(&cache_dir.join("post-build")));
        let mode = fs::metadata(cache_dir.join("post-build"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o755, 0o755);
    }

    #[test]
    fn extract_cached_reuses_on_same_version() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".zetl")).unwrap();

        let hook_files = vec![("post-build".to_string(), b"#!/bin/sh\necho v1\n".to_vec())];

        // First extraction
        let dir1 = extract_bundled_hooks_cached(tmp.path(), "t", &hook_files, "1.0.0").unwrap();

        // Modify the cached file to detect if it gets overwritten
        let marker_path = dir1.join("post-build");
        fs::write(&marker_path, "#!/bin/sh\necho modified\n").unwrap();

        // Second extraction with same version — should reuse cache
        let dir2 = extract_bundled_hooks_cached(tmp.path(), "t", &hook_files, "1.0.0").unwrap();

        assert_eq!(dir1, dir2);
        // File should still have the modified content (cache was reused)
        let content = fs::read_to_string(&marker_path).unwrap();
        assert_eq!(content, "#!/bin/sh\necho modified\n");
    }

    #[test]
    fn extract_cached_refreshes_on_version_change() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".zetl")).unwrap();

        let hook_files = vec![("post-build".to_string(), b"#!/bin/sh\necho v1\n".to_vec())];

        // First extraction at version 1.0.0
        let dir1 = extract_bundled_hooks_cached(tmp.path(), "t", &hook_files, "1.0.0").unwrap();

        // Modify the file to prove refresh overwrites it
        fs::write(dir1.join("post-build"), "#!/bin/sh\necho stale\n").unwrap();

        // Second extraction at version 2.0.0 — should refresh
        let hook_files_v2 = vec![("post-build".to_string(), b"#!/bin/sh\necho v2\n".to_vec())];
        let dir2 = extract_bundled_hooks_cached(tmp.path(), "t", &hook_files_v2, "2.0.0").unwrap();

        assert_eq!(dir1, dir2);
        let content = fs::read_to_string(dir2.join("post-build")).unwrap();
        assert_eq!(content, "#!/bin/sh\necho v2\n");

        let stamp = fs::read_to_string(dir2.join(".zetl_version")).unwrap();
        assert_eq!(stamp, "2.0.0");
    }

    #[test]
    fn extract_cached_discoverable_by_hook_discovery() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".zetl")).unwrap();

        let hook_files = vec![("post-build".to_string(), b"#!/bin/sh\necho ok\n".to_vec())];

        let cache_dir =
            extract_bundled_hooks_cached(tmp.path(), "mytheme", &hook_files, "1.0.0").unwrap();

        let manifest = discover_hooks(tmp.path(), Some(&cache_dir));
        assert_eq!(manifest.hooks.len(), 1);
        assert_eq!(manifest.hooks[0].name, "post-build");
        assert_eq!(manifest.hooks[0].source, HookSource::Theme);
        assert!(manifest.hooks[0].executable);
    }

    #[test]
    fn resolve_theme_hooks_versioned_uses_cache() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".zetl")).unwrap();

        // "default" bundled theme has no hooks, so use a nonexistent theme
        // that won't have bundled hooks either. We test the flow via
        // extract_bundled_hooks_cached directly.
        // This test verifies that resolve_theme_hooks_versioned prefers
        // disk-installed hooks.
        let theme_hooks = tmp
            .path()
            .join(".zetl")
            .join("themes")
            .join("test-theme")
            .join("hooks");
        fs::create_dir_all(&theme_hooks).unwrap();
        create_hook(&theme_hooks, "post-build", true);

        let resolved = resolve_theme_hooks_versioned(tmp.path(), "test-theme", "1.0.0");
        assert!(resolved.path().is_some());
        assert_eq!(resolved.path().unwrap(), theme_hooks);
    }

    // ── Timeout tests ───────────────────────────────────────────────────

    #[test]
    fn hook_killed_on_timeout() {
        let tmp = TempDir::new().unwrap();
        let hooks_dir = tmp.path().join(".zetl").join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();

        // Hook that sleeps forever
        create_script(&hooks_dir, "post-build", "#!/bin/sh\nsleep 300\n");

        let manifest = discover_hooks(tmp.path(), None);
        let hook = &manifest.hooks[0];

        // Use a 1-second timeout for fast testing
        let result = execute_hook_with_timeout(
            hook,
            b"{}",
            &test_env(tmp.path()),
            false,
            Duration::from_secs(1),
        )
        .unwrap();

        assert!(result.timed_out);
        assert!(!result.success());
        assert!(result.exit_code.is_none()); // killed by signal
        assert!(result.duration >= Duration::from_secs(1));
    }

    #[test]
    fn fast_hook_not_timed_out() {
        let tmp = TempDir::new().unwrap();
        let hooks_dir = tmp.path().join(".zetl").join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();

        create_script(&hooks_dir, "post-build", "#!/bin/sh\necho ok\n");

        let manifest = discover_hooks(tmp.path(), None);
        let hook = &manifest.hooks[0];

        let result = execute_hook_with_timeout(
            hook,
            b"{}",
            &test_env(tmp.path()),
            false,
            Duration::from_secs(5),
        )
        .unwrap();

        assert!(!result.timed_out);
        assert!(result.success());
        assert_eq!(result.exit_code, Some(0));
        assert!(result.stdout.contains("ok"));
    }

    #[test]
    fn default_timeout_constant_is_30() {
        assert_eq!(HOOK_TIMEOUT_SECS, 30);
    }

    // ── Hook depth (REQ-020-020) tests ───────────────────────────────────

    #[test]
    fn hook_receives_zetl_hook_depth_env_var() {
        let tmp = TempDir::new().unwrap();
        let hooks_dir = tmp.path().join(".zetl").join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();

        // Script that prints the ZETL_HOOK_DEPTH env var
        create_script(
            &hooks_dir,
            "post-build",
            "#!/bin/sh\necho \"depth=$ZETL_HOOK_DEPTH\"\n",
        );

        let manifest = discover_hooks(tmp.path(), None);
        let hook = &manifest.hooks[0];

        // No ZETL_HOOK_DEPTH in extra_vars → inherits from process env (default 0),
        // child gets depth 1
        let result = execute_hook(hook, b"{}", &test_env(tmp.path())).unwrap();
        assert!(result.success());
        assert!(
            result.stdout.contains("depth=1"),
            "expected depth=1, got: {}",
            result.stdout.trim()
        );
    }

    #[test]
    fn hook_depth_increments_from_extra_vars() {
        let tmp = TempDir::new().unwrap();
        let hooks_dir = tmp.path().join(".zetl").join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();

        create_script(
            &hooks_dir,
            "on-save",
            "#!/bin/sh\necho \"depth=$ZETL_HOOK_DEPTH\"\n",
        );

        let manifest = discover_hooks(tmp.path(), None);
        let hook = &manifest.hooks[0];

        // Pass ZETL_HOOK_DEPTH=2 in extra_vars → child should get 3
        let env = HookEnv {
            vault_root: tmp.path().to_path_buf(),
            theme: "test".to_string(),
            zetl_version: "0.1.0".to_string(),
            extra_vars: vec![("ZETL_HOOK_DEPTH".into(), "2".into())],
        };

        let result = execute_hook(hook, b"{}", &env).unwrap();
        assert!(result.success());
        assert!(
            result.stdout.contains("depth=3"),
            "expected depth=3, got: {}",
            result.stdout.trim()
        );
    }

    #[test]
    fn hook_depth_starts_at_one_from_zero_extra_var() {
        let tmp = TempDir::new().unwrap();
        let hooks_dir = tmp.path().join(".zetl").join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();

        create_script(
            &hooks_dir,
            "on-save",
            "#!/bin/sh\necho \"depth=$ZETL_HOOK_DEPTH\"\n",
        );

        let manifest = discover_hooks(tmp.path(), None);
        let hook = &manifest.hooks[0];

        // Pass ZETL_HOOK_DEPTH=0 (initial event) → child gets 1
        let env = HookEnv {
            vault_root: tmp.path().to_path_buf(),
            theme: "test".to_string(),
            zetl_version: "0.1.0".to_string(),
            extra_vars: vec![("ZETL_HOOK_DEPTH".into(), "0".into())],
        };

        let result = execute_hook(hook, b"{}", &env).unwrap();
        assert!(result.success());
        assert!(
            result.stdout.contains("depth=1"),
            "expected depth=1, got: {}",
            result.stdout.trim()
        );
    }
}
