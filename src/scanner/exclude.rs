//! Pure exclusion resolver for vault scanning.
//!
//! Implements the precedence stack from SPEC-026 §REQ-205. The pure entry
//! point is [`classify_entry`], which decides whether a single filesystem
//! entry should be included in the scan. Levels 3–5 (.gitignore,
//! .zetlignore, `--exclude`) are handled by the `ignore` crate's
//! `WalkBuilder` chain at the call site; this module contributes levels
//! 1–2 (hardcoded force-ignores + nested vault + dotdir default).

#![deny(clippy::disallowed_methods)]

use ignore::gitignore::Gitignore;
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct ScanOptions {
    /// Repeatable `--exclude PATTERN` values from the CLI. Gitignore syntax,
    /// evaluated relative to the vault root.
    pub exclude_patterns: Vec<String>,
    /// When true, disables the level-2 dotdir default exclusion. Level-1
    /// force-ignores are unaffected.
    pub include_hidden: bool,
    /// Emit OBS-200 `[zetl] scan: skipped ...` lines on stderr when an
    /// entry is excluded. Wired to the global `--verbose` flag.
    pub verbose: bool,
}

impl ScanOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_exclude_patterns(mut self, patterns: Vec<String>) -> Self {
        self.exclude_patterns = patterns;
        self
    }

    pub fn with_include_hidden(mut self, include_hidden: bool) -> Self {
        self.include_hidden = include_hidden;
        self
    }

    pub fn with_verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExcludeReason {
    /// `.git/`, `.zetl/`, `node_modules/` — never overridable.
    Hardcoded,
    /// Directory contains its own `.zetl/`, marking it as a child vault.
    NestedVault,
    /// Default dotdir exclusion (REQ-200), not overridden by `.zetlignore`.
    Dotdir,
}

impl ExcludeReason {
    pub fn as_str(self) -> &'static str {
        match self {
            ExcludeReason::Hardcoded => "hardcoded",
            ExcludeReason::NestedVault => "nested-vault",
            ExcludeReason::Dotdir => "dotdir",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Include,
    Exclude(ExcludeReason),
}

/// Pure entry-point classifier. Decides whether a single filesystem entry
/// should be included in the scan, given pre-loaded `.zetlignore` rules
/// and an injected nested-vault probe.
///
/// The function performs no IO directly; the `is_nested_vault` closure is
/// the only escape hatch and is invoked at most once.
pub fn classify_entry(
    relative_path: &Path,
    basename: &str,
    is_dir: bool,
    opts: &ScanOptions,
    is_nested_vault: impl FnOnce() -> bool,
    zetlignore: Option<&Gitignore>,
) -> Decision {
    // Level 1a: hardcoded directory names — never traverse, regardless of
    // any other configuration. These are listed in the WalkBuilder Override
    // as well, but having them here means the watcher path picks them up
    // without depending on Override semantics.
    if is_dir {
        match basename {
            ".git" | ".zetl" | "node_modules" => {
                return Decision::Exclude(ExcludeReason::Hardcoded);
            }
            _ => {}
        }
    }

    // Level 1b: nested vault. A child directory that contains its own
    // `.zetl/` belongs to a different vault and must not be absorbed.
    if is_dir && is_nested_vault() {
        return Decision::Exclude(ExcludeReason::NestedVault);
    }

    // Level 2: dotdir default. Skip directories whose basename starts with
    // `.`, unless `--include-hidden` is set or `.zetlignore` has a
    // whitelist (`!pattern`) match for this path. Files (including
    // dotfiles) are not excluded by this rule — only directories.
    if !opts.include_hidden && is_dir && basename.starts_with('.') {
        if let Some(gi) = zetlignore {
            if gi
                .matched_path_or_any_parents(relative_path, true)
                .is_whitelist()
            {
                return Decision::Include;
            }
        }
        return Decision::Exclude(ExcludeReason::Dotdir);
    }

    Decision::Include
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn no_nested() -> bool {
        false
    }

    #[test]
    fn force_ignored_dirs_excluded() {
        let opts = ScanOptions::default();
        let r = classify_entry(
            &PathBuf::from(".git"),
            ".git",
            true,
            &opts,
            no_nested,
            None,
        );
        assert_eq!(r, Decision::Exclude(ExcludeReason::Hardcoded));

        let r = classify_entry(
            &PathBuf::from("node_modules"),
            "node_modules",
            true,
            &opts,
            no_nested,
            None,
        );
        assert_eq!(r, Decision::Exclude(ExcludeReason::Hardcoded));

        let r = classify_entry(
            &PathBuf::from(".zetl"),
            ".zetl",
            true,
            &opts,
            no_nested,
            None,
        );
        assert_eq!(r, Decision::Exclude(ExcludeReason::Hardcoded));
    }

    #[test]
    fn force_ignored_dirs_excluded_even_with_include_hidden() {
        // REQ-205: level-1 force-ignores are not overridable.
        let opts = ScanOptions::default().with_include_hidden(true);
        let r = classify_entry(
            &PathBuf::from(".zetl"),
            ".zetl",
            true,
            &opts,
            no_nested,
            None,
        );
        assert_eq!(r, Decision::Exclude(ExcludeReason::Hardcoded));
    }

    #[test]
    fn dotdir_excluded_by_default() {
        let opts = ScanOptions::default();
        let r = classify_entry(
            &PathBuf::from(".claude"),
            ".claude",
            true,
            &opts,
            no_nested,
            None,
        );
        assert_eq!(r, Decision::Exclude(ExcludeReason::Dotdir));
    }

    #[test]
    fn dotfile_at_root_included() {
        // REQ-201: dotfiles are not subject to the dotdir rule.
        let opts = ScanOptions::default();
        let r = classify_entry(
            &PathBuf::from(".hidden-note.md"),
            ".hidden-note.md",
            false,
            &opts,
            no_nested,
            None,
        );
        assert_eq!(r, Decision::Include);
    }

    #[test]
    fn dotdir_walked_when_include_hidden_set() {
        let opts = ScanOptions::default().with_include_hidden(true);
        let r = classify_entry(
            &PathBuf::from(".claude"),
            ".claude",
            true,
            &opts,
            no_nested,
            None,
        );
        assert_eq!(r, Decision::Include);
    }

    #[test]
    fn nested_vault_excluded() {
        let opts = ScanOptions::default();
        let r = classify_entry(
            &PathBuf::from("subvault"),
            "subvault",
            true,
            &opts,
            || true,
            None,
        );
        assert_eq!(r, Decision::Exclude(ExcludeReason::NestedVault));
    }

    #[test]
    fn zetlignore_negation_overrides_dotdir_default() {
        let opts = ScanOptions::default();
        let mut gi = ignore::gitignore::GitignoreBuilder::new("/vault");
        gi.add_line(None, "!.archive/").unwrap();
        let matcher = gi.build().unwrap();

        let r = classify_entry(
            &PathBuf::from(".archive"),
            ".archive",
            true,
            &opts,
            no_nested,
            Some(&matcher),
        );
        assert_eq!(r, Decision::Include);
    }

    #[test]
    fn zetlignore_negation_does_not_override_hardcoded() {
        let opts = ScanOptions::default();
        let mut gi = ignore::gitignore::GitignoreBuilder::new("/vault");
        gi.add_line(None, "!.zetl/").unwrap();
        let matcher = gi.build().unwrap();

        let r = classify_entry(
            &PathBuf::from(".zetl"),
            ".zetl",
            true,
            &opts,
            no_nested,
            Some(&matcher),
        );
        assert_eq!(r, Decision::Exclude(ExcludeReason::Hardcoded));
    }

    #[test]
    fn regular_dir_included() {
        let opts = ScanOptions::default();
        let r = classify_entry(
            &PathBuf::from("notes"),
            "notes",
            true,
            &opts,
            no_nested,
            None,
        );
        assert_eq!(r, Decision::Include);
    }
}
