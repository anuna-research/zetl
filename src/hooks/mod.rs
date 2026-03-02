//! Lifecycle hooks for zetl vault operations (SPEC-016).
//!
//! This module provides:
//! - **Discovery** (REQ-016-001): Scan `.zetl/hooks/` and theme `hooks/`
//!   directories for executable hook files matching recognised lifecycle points.
//! - **Execution** (REQ-016-003): Spawn hooks as child processes, write JSON
//!   context to stdin, set ZETL_* environment variables, capture output, and
//!   report exit codes.

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Recognised hook lifecycle points (REQ-016-002).
pub const HOOK_NAMES: &[&str] = &[
    "pre-build",
    "post-build",
    "post-index",
    "post-check",
    "on-save",
    "pre-serve",
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
pub fn discover_hooks(
    vault_root: &Path,
    theme_hooks_dir: Option<&Path>,
) -> HookManifest {
    let mut hooks = Vec::new();
    let mut warnings = Vec::new();

    // 1. Theme hooks
    if let Some(dir) = theme_hooks_dir {
        scan_hooks_dir(dir, HookSource::Theme, &mut hooks, &mut warnings);
    }

    // 2. Vault hooks
    let vault_hooks_dir = vault_root.join(".zetl").join("hooks");
    scan_hooks_dir(&vault_hooks_dir, HookSource::Vault, &mut hooks, &mut warnings);

    HookManifest { hooks, warnings }
}

/// Resolve the theme hooks directory path, if it exists on disk.
///
/// For disk-installed themes: `.zetl/themes/<name>/hooks/`
/// Returns `None` if the directory does not exist.
///
/// Bundled theme hook extraction is handled separately (Phase 3).
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
}

impl HookOutput {
    /// Whether the hook exited successfully (exit code 0).
    pub fn success(&self) -> bool {
        self.exit_code == Some(0)
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
    let start = Instant::now();

    let mut cmd = Command::new(&hook.path);
    cmd.current_dir(&env.vault_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("ZETL_HOOK", &hook.name)
        .env("ZETL_VAULT_ROOT", &env.vault_root)
        .env("ZETL_THEME", &env.theme)
        .env("ZETL_VERSION", &env.zetl_version);

    for (key, val) in &env.extra_vars {
        cmd.env(key, val);
    }

    let mut child = cmd.spawn()?;

    // Write context JSON to stdin, then close the pipe.
    if let Some(mut stdin) = child.stdin.take() {
        // Ignore write errors — the hook may have exited early.
        let _ = stdin.write_all(context_json);
    }

    let output = child.wait_with_output()?;
    let duration = start.elapsed();

    Ok(HookOutput {
        hook_name: hook.name.clone(),
        source: hook.source.clone(),
        path: hook.path.clone(),
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        duration,
    })
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
    hooks_for(manifest, hook_name)
        .into_iter()
        .map(|hook| execute_hook(hook, context_json, env))
        .collect()
}

// ── Internal helpers ─────────────────────────────────────────────────────────

/// Scan a single hooks directory and append discovered hooks.
fn scan_hooks_dir(
    dir: &Path,
    source: HookSource,
    hooks: &mut Vec<DiscoveredHook>,
    warnings: &mut Vec<String>,
) {
    if !dir.is_dir() {
        return;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
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
            continue;
        }

        let executable = is_executable(&path);

        if !executable {
            warnings.push(format!(
                "hook '{}' is not executable: {}\nhint: chmod +x {}",
                file_name,
                path.display(),
                path.display(),
            ));
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
fn is_executable(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(meta) => meta.permissions().mode() & 0o111 != 0,
        Err(_) => false,
    }
}

#[cfg(test)]
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
        let theme_hooks = tmp.path().join(".zetl").join("themes").join("fountain").join("hooks");
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
        let theme_hooks = tmp.path().join(".zetl").join("themes").join("fountain").join("hooks");
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
            &format!(
                "#!/bin/sh\ncat > '{}'\n",
                out_file.display()
            ),
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
        create_script(
            &hooks_dir,
            "on-save",
            "#!/bin/sh\necho 'on-save ran'\n",
        );

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
        create_script(
            &vault_hooks,
            "post-build",
            "#!/bin/sh\necho 'vault'\n",
        );

        // Theme hook
        let theme_hooks = tmp.path().join("theme-hooks");
        fs::create_dir_all(&theme_hooks).unwrap();
        create_script(
            &theme_hooks,
            "post-build",
            "#!/bin/sh\necho 'theme'\n",
        );

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
}
