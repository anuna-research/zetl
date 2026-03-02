//! Hook discovery for zetl lifecycle hooks (SPEC-016, REQ-016-001).
//!
//! Scans `.zetl/hooks/` (vault hooks) and the active theme's `hooks/`
//! directory (theme hooks) for executable hook files matching recognised
//! lifecycle points. Non-executable hooks produce a warning with a
//! `chmod +x` hint. Unrecognised filenames are silently ignored.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

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
}
