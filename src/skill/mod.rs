//! `zetl skill` — self-bootstrap a Claude Code skill file.
//!
//! Embeds the canonical `SKILL.md` (kept on disk for editor tooling and
//! preview, slurped at compile time via [`include_str!`]) and installs
//! it on demand to either:
//!
//! - `.claude/skills/zetl/SKILL.md` (project scope, gets committed) — default
//! - `~/.claude/skills/zetl/SKILL.md` (user scope) — with `--user`
//!
//! The version sentinel `__ZETL_VERSION__` is substituted at write time
//! with `env!("CARGO_PKG_VERSION")` so the on-disk skill never drifts
//! away from the binary release.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

const SKILL_TEMPLATE: &str = include_str!("SKILL.md");
const SKILL_NAME: &str = "zetl";
const VERSION_SENTINEL: &str = "__ZETL_VERSION__";

/// Rendered skill content with the version sentinel substituted.
pub fn rendered() -> String {
    SKILL_TEMPLATE.replace(VERSION_SENTINEL, env!("CARGO_PKG_VERSION"))
}

/// Resolve the base directory for a given install scope.
///
/// `user=false` → current working directory (project-local install).
/// `user=true`  → `$HOME` (global install).
pub fn install_base(user: bool) -> Result<PathBuf> {
    if user {
        let home = std::env::var_os("HOME")
            .context("$HOME is unset; cannot resolve --user install path")?;
        Ok(PathBuf::from(home))
    } else {
        std::env::current_dir().context("failed to read current directory")
    }
}

/// Compose the full target path for an install given its base dir.
pub fn install_path_under(base: &Path) -> PathBuf {
    base.join(".claude")
        .join("skills")
        .join(SKILL_NAME)
        .join("SKILL.md")
}

/// Write the embedded skill under `base`, creating parents as needed.
/// Idempotent — overwrites any existing file so a user gets the new
/// skill simply by upgrading the binary and re-running this command.
pub fn install_at(base: &Path) -> Result<PathBuf> {
    let path = install_path_under(base);
    let parent = path.parent().expect("install_path always has a parent");
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create {}", parent.display()))?;
    fs::write(&path, rendered())
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}

/// Write the embedded skill at the install location for `user` scope.
pub fn install(user: bool) -> Result<PathBuf> {
    let base = install_base(user)?;
    install_at(&base)
}

/// CLI handler for `zetl skill init`.
pub fn cmd_skill_init(user: bool) -> Result<()> {
    let path = install(user)?;
    println!(
        "wrote {} (zetl v{})",
        path.display(),
        env!("CARGO_PKG_VERSION")
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_sentinel_is_substituted() {
        let out = rendered();
        assert!(
            !out.contains(VERSION_SENTINEL),
            "rendered SKILL.md still contains the version sentinel"
        );
        assert!(
            out.contains(env!("CARGO_PKG_VERSION")),
            "rendered SKILL.md missing the current crate version"
        );
    }

    #[test]
    fn frontmatter_name_matches_parent_dir() {
        // agent-skills-eval requires `name:` to match the parent dir
        // (which we control via [`SKILL_NAME`]).
        assert!(rendered().contains(&format!("name: {SKILL_NAME}")));
    }

    #[test]
    fn install_writes_under_dot_claude_and_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let first = install_at(tmp.path()).unwrap();
        assert!(first.starts_with(tmp.path().join(".claude/skills/zetl")));
        assert!(first.exists());
        let body = std::fs::read_to_string(&first).unwrap();
        assert!(body.contains("name: zetl"));
        assert!(!body.contains(VERSION_SENTINEL));

        // Second call overwrites the same path without erroring.
        let second = install_at(tmp.path()).unwrap();
        assert_eq!(first, second);
    }
}
