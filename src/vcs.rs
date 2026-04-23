//! VCS (Version Control System) detection and metadata helpers.
//!
//! Provides optional Git enrichment: when the vault is inside a Git repository
//! ztl MAY attach commit metadata to caches and outputs for informational
//! purposes. This is supplementary — it enriches output but does not alter
//! correctness or cache decisions (NFR-017, §1.6).

use std::path::Path;

/// Read optional Git metadata for the vault directory.
///
/// Returns `(git_commit, git_dirty)` where:
/// - `git_commit` is the HEAD commit hash (40-char hex), or `None` if the
///   vault is not inside a Git repository or the commit cannot be resolved.
/// - `git_dirty` is `true` if the working tree has uncommitted changes,
///   `false` if clean, or `None` if Git is unavailable or the vault is not a
///   Git repository.
///
/// Both fields default to `None` and are purely informational — callers MUST
/// NOT make correctness or cache decisions based on their values.
pub fn get_git_metadata(vault_root: &Path) -> (Option<String>, Option<bool>) {
    let git_commit = read_git_commit(vault_root);
    let git_dirty = if git_commit.is_some() {
        read_git_dirty(vault_root)
    } else {
        None
    };
    (git_commit, git_dirty)
}

/// Attempt to resolve the HEAD commit hash by reading `.git` files directly.
///
/// Does not require the `git` binary. Returns `None` if:
/// - No `.git` directory exists
/// - `.git/HEAD` cannot be read or parsed
/// - The referenced branch ref file cannot be read
fn read_git_commit(vault_root: &Path) -> Option<String> {
    // Walk up to find the .git directory (supports worktrees inside sub-dirs)
    let git_dir = find_git_dir(vault_root)?;

    let head_path = git_dir.join("HEAD");
    let head_content = std::fs::read_to_string(&head_path).ok()?;
    let head_content = head_content.trim();

    if let Some(ref_path) = head_content.strip_prefix("ref: ") {
        // Symbolic ref: "ref: refs/heads/main"
        let ref_file = git_dir.join(ref_path.replace('/', std::path::MAIN_SEPARATOR_STR));
        let commit = std::fs::read_to_string(&ref_file).ok()?;
        let commit = commit.trim();
        if is_hex40(commit) {
            return Some(commit.to_string());
        }
        // Packed refs fallback
        read_packed_ref(&git_dir, ref_path)
    } else if is_hex40(head_content) {
        // Detached HEAD: content is the commit hash directly
        Some(head_content.to_string())
    } else {
        None
    }
}

/// Read a ref from `.git/packed-refs` (used when refs are packed).
fn read_packed_ref(git_dir: &Path, ref_path: &str) -> Option<String> {
    let packed_refs_path = git_dir.join("packed-refs");
    let content = std::fs::read_to_string(&packed_refs_path).ok()?;
    for line in content.lines() {
        if line.starts_with('#') {
            continue;
        }
        let mut parts = line.splitn(2, ' ');
        let hash = parts.next()?;
        let name = parts.next()?.trim();
        if name == ref_path && is_hex40(hash) {
            return Some(hash.to_string());
        }
    }
    None
}

/// Check whether the working tree is dirty by running `git status --porcelain`.
///
/// Requires the `git` binary. Returns `None` if git is not available or the
/// command fails.
fn read_git_dirty(vault_root: &Path) -> Option<bool> {
    let output = std::process::Command::new("git")
        .args(["-C", vault_root.to_str()?, "status", "--porcelain"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = std::str::from_utf8(&output.stdout).ok()?;
    Some(!stdout.trim().is_empty())
}

/// Walk up the directory tree from `start` to find the nearest `.git` directory.
///
/// Returns `None` if no `.git` directory is found before the filesystem root.
fn find_git_dir(start: &Path) -> Option<std::path::PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        let candidate = current.join(".git");
        if candidate.is_dir() {
            return Some(candidate);
        }
        if !current.pop() {
            return None;
        }
    }
}

/// Return `true` if `s` is exactly 40 lowercase hex characters (a SHA-1 hash).
fn is_hex40(s: &str) -> bool {
    s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_hex40() {
        assert!(is_hex40("a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2"));
        assert!(!is_hex40("not-a-hash"));
        assert!(!is_hex40(""));
        assert!(!is_hex40("a1b2")); // too short
    }

    #[test]
    fn test_no_git_dir_returns_none() {
        let dir = tempfile::TempDir::new().unwrap();
        let (commit, dirty) = get_git_metadata(dir.path());
        assert!(commit.is_none(), "no .git → commit should be None");
        assert!(dirty.is_none(), "no .git → dirty should be None");
    }
}
