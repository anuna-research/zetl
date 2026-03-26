//! Git auto-commit on save (REQ-020-015, CON-020-006).
//!
//! Provides `auto_commit()` which stages a single file and commits it to the
//! vault's git repository with user attribution. A `Mutex<git2::Repository>`
//! serializes concurrent commits so that HEAD always advances linearly.

use std::path::Path;
use std::sync::Mutex;

/// Serialization lock wrapping the git2 repository handle.
///
/// All git operations (add + commit) MUST hold this lock to prevent concurrent
/// HEAD mutations. Stored in [`super::WebState`] behind an `Arc`.
pub type GitCommitLock = Mutex<git2::Repository>;

/// Open the vault's git repository for auto-commit.
///
/// Returns `Some(GitCommitLock)` if a `.git/` directory exists at or above
/// `vault_root`, or `None` if the vault is not inside a git repository.
pub fn open_repo(vault_root: &Path) -> Option<GitCommitLock> {
    match git2::Repository::discover(vault_root) {
        Ok(repo) => Some(Mutex::new(repo)),
        Err(_) => None,
    }
}

/// Commit a single file change to the vault's git repository (CON-020-006).
///
/// - `file_path`: path relative to the vault root (e.g. `"notes/My Page.md"`)
/// - `user_name`: display name for the git author
/// - `user_id`: user identifier (used as `<user_id>@vault` in author email)
/// - `message`: optional custom commit message (from `X-Commit-Message` header);
///   defaults to `"edit: {page_name}"` where page_name is the filename stem.
///
/// Returns the commit OID on success.
pub fn auto_commit(
    repo: &git2::Repository,
    file_path: &Path,
    user_name: &str,
    user_id: &str,
    message: Option<&str>,
) -> Result<git2::Oid, git2::Error> {
    // 1. Stage the file via the index.
    //    Resolve absolute paths to repo-relative (required by git2).
    //    This handles vaults that are subdirectories of the git repo.
    //    Canonicalize both paths to handle macOS /var vs /private/var symlinks.
    let resolved;
    let git_path = if file_path.is_absolute() {
        if let Some(wd) = repo.workdir() {
            let canon_file = std::fs::canonicalize(file_path).unwrap_or_else(|_| file_path.to_path_buf());
            let canon_wd = std::fs::canonicalize(wd).unwrap_or_else(|_| wd.to_path_buf());
            if !canon_file.starts_with(&canon_wd) {
                return Err(git2::Error::from_str(&format!(
                    "file path {:?} is outside the repository working directory {:?}",
                    canon_file, canon_wd
                )));
            }
            resolved = canon_file
                .strip_prefix(&canon_wd)
                .unwrap_or(file_path)
                .to_path_buf();
            &resolved
        } else {
            file_path
        }
    } else {
        file_path
    };
    let mut index = repo.index()?;
    index.add_path(git_path)?;
    index.write()?;
    let tree_oid = index.write_tree()?;
    let tree = repo.find_tree(tree_oid)?;

    // 2. Build author signature: "<display_name> <<user_id>@vault>"
    let email = format!("{user_id}@vault");
    let sig = git2::Signature::now(user_name, &email)?;

    // 3. Resolve commit message.
    let page_name = file_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let default_msg = format!("edit: {page_name}");
    let msg = message.unwrap_or(&default_msg);

    // 4. Resolve parent commit (HEAD). For an initial commit there is no parent.
    let parent = match repo.head() {
        Ok(head_ref) => {
            let commit = head_ref.peel_to_commit()?;
            Some(commit)
        }
        Err(e) if e.code() == git2::ErrorCode::UnbornBranch => None,
        Err(e) => return Err(e),
    };
    let parents: Vec<&git2::Commit> = parent.iter().collect();

    // 5. Create the commit on HEAD.
    repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &parents)
}

/// Run `jj git import` to synchronize jj's view after a git2 commit.
///
/// This is necessary because jj in colocated mode needs to see git commits
/// that were made outside jj (REQ-020-015 step 4, SPEC-017).
///
/// Failures are logged but not propagated — jj import is best-effort.
pub fn jj_git_import(vault_root: &Path) {
    match std::process::Command::new("jj")
        .args(["git", "import"])
        .current_dir(vault_root)
        .output()
    {
        Ok(output) if !output.status.success() => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            eprintln!("warning: jj git import failed: {}", stderr.trim());
        }
        Err(e) => {
            // jj not installed or not in PATH — not fatal.
            if e.kind() != std::io::ErrorKind::NotFound {
                eprintln!("warning: jj git import error: {e}");
            }
        }
        _ => {}
    }
}

/// A single entry in the git log for a specific file.
#[derive(Debug, Clone, serde::Serialize)]
pub struct GitLogEntry {
    /// Abbreviated commit hash (7 chars).
    pub commit_id: String,
    /// Full commit hash (for restore operations).
    pub full_oid: String,
    /// Author display name.
    pub author: String,
    /// Author email.
    pub email: String,
    /// Unix timestamp (seconds since epoch).
    pub timestamp: i64,
    /// Commit message summary (first line).
    pub summary: String,
    /// Full commit message.
    pub message: String,
}

/// Walk git history for a specific file path, returning up to `limit` entries.
///
/// `file_path` is relative to the repository root (e.g. `"notes/My Page.md"`).
/// Returns entries in newest-first order.
pub fn file_log(
    repo: &git2::Repository,
    file_path: &Path,
    limit: usize,
) -> Vec<GitLogEntry> {
    let Ok(mut revwalk) = repo.revwalk() else {
        return Vec::new();
    };
    let _ = revwalk.push_head();
    revwalk.set_sorting(git2::Sort::TIME).ok();

    let file_path_str = file_path.to_string_lossy();
    let mut entries = Vec::new();

    for oid in revwalk.flatten() {
        if entries.len() >= limit {
            break;
        }
        let Ok(commit) = repo.find_commit(oid) else {
            continue;
        };

        // Check if this commit touched the file by diffing with parent.
        let touched = if let Ok(parent) = commit.parent(0) {
            let old_tree = parent.tree().ok();
            let new_tree = commit.tree().ok();
            repo.diff_tree_to_tree(old_tree.as_ref(), new_tree.as_ref(), None)
                .map(|diff| {
                    diff.deltas().any(|d| {
                        d.new_file()
                            .path()
                            .map(|p| p.to_string_lossy() == file_path_str)
                            .unwrap_or(false)
                            || d.old_file()
                                .path()
                                .map(|p| p.to_string_lossy() == file_path_str)
                                .unwrap_or(false)
                    })
                })
                .unwrap_or(false)
        } else {
            // Initial commit — check if the file exists in its tree.
            commit
                .tree()
                .ok()
                .and_then(|t| t.get_path(file_path).ok())
                .is_some()
        };

        if !touched {
            continue;
        }

        let oid_str = oid.to_string();
        entries.push(GitLogEntry {
            commit_id: oid_str[..7.min(oid_str.len())].to_string(),
            full_oid: oid_str,
            author: commit
                .author()
                .name()
                .unwrap_or("unknown")
                .to_string(),
            email: commit
                .author()
                .email()
                .unwrap_or("")
                .to_string(),
            timestamp: commit.time().seconds(),
            summary: commit.summary().unwrap_or("").to_string(),
            message: commit.message().unwrap_or("").to_string(),
        });
    }

    entries
}

/// Retrieve the content of a file at a specific commit.
///
/// Returns `None` if the commit or file cannot be found.
pub fn file_at_commit(
    repo: &git2::Repository,
    commit_oid: &str,
    file_path: &Path,
) -> Option<String> {
    let oid = git2::Oid::from_str(commit_oid).ok()?;
    let commit = repo.find_commit(oid).ok()?;
    let tree = commit.tree().ok()?;
    let entry = tree.get_path(file_path).ok()?;
    let blob = repo.find_blob(entry.id()).ok()?;
    std::str::from_utf8(blob.content()).ok().map(|s| s.to_string())
}

/// Generate a unified diff of a file between a commit and its parent.
///
/// Returns the diff as a string, or an empty string if unavailable.
pub fn file_diff_at_commit(
    repo: &git2::Repository,
    commit_oid: &str,
    file_path: &Path,
) -> String {
    let oid = match git2::Oid::from_str(commit_oid) {
        Ok(o) => o,
        Err(_) => return String::new(),
    };
    let commit = match repo.find_commit(oid) {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    let new_tree = match commit.tree() {
        Ok(t) => t,
        Err(_) => return String::new(),
    };
    let old_tree = commit.parent(0).ok().and_then(|p| p.tree().ok());

    let mut opts = git2::DiffOptions::new();
    opts.pathspec(file_path.to_string_lossy().as_ref());

    let diff = match repo.diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), Some(&mut opts)) {
        Ok(d) => d,
        Err(_) => return String::new(),
    };

    let mut output = String::new();
    diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
        let origin = line.origin();
        match origin {
            '+' | '-' | ' ' => output.push(origin),
            _ => {}
        }
        if let Ok(content) = std::str::from_utf8(line.content()) {
            output.push_str(content);
        }
        true
    })
    .ok();

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Create a temporary git repo with an initial commit.
    fn init_test_repo() -> (tempfile::TempDir, git2::Repository) {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();

        // Configure user for the test repo.
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "test").unwrap();
        config.set_str("user.email", "test@test").unwrap();

        // Create an initial commit so HEAD exists.
        let sig = git2::Signature::now("test", "test@test").unwrap();
        let tree_oid = {
            let mut index = repo.index().unwrap();
            fs::write(dir.path().join(".gitkeep"), "").unwrap();
            index
                .add_path(Path::new(".gitkeep"))
                .unwrap();
            index.write().unwrap();
            index.write_tree().unwrap()
        };
        {
            let tree = repo.find_tree(tree_oid).unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
                .unwrap();
        }

        (dir, repo)
    }

    #[test]
    fn auto_commit_creates_commit_with_correct_author() {
        let (dir, repo) = init_test_repo();

        // Write a file and auto-commit it.
        fs::write(dir.path().join("page.md"), "# Hello").unwrap();
        let oid = auto_commit(
            &repo,
            Path::new("page.md"),
            "Alice",
            "alice-abc123",
            None,
        )
        .unwrap();

        let commit = repo.find_commit(oid).unwrap();
        assert_eq!(commit.message().unwrap(), "edit: page");
        assert_eq!(commit.author().name().unwrap(), "Alice");
        assert_eq!(commit.author().email().unwrap(), "alice-abc123@vault");
    }

    #[test]
    fn auto_commit_uses_custom_message() {
        let (dir, repo) = init_test_repo();

        fs::write(dir.path().join("note.md"), "content").unwrap();
        let oid = auto_commit(
            &repo,
            Path::new("note.md"),
            "Bob",
            "bob-xyz",
            Some("fix: typo in note"),
        )
        .unwrap();

        let commit = repo.find_commit(oid).unwrap();
        assert_eq!(commit.message().unwrap(), "fix: typo in note");
    }

    #[test]
    fn auto_commit_advances_head() {
        let (dir, repo) = init_test_repo();

        let head_before = repo.head().unwrap().target().unwrap();

        fs::write(dir.path().join("a.md"), "first").unwrap();
        auto_commit(&repo, Path::new("a.md"), "User", "u1", None).unwrap();

        fs::write(dir.path().join("b.md"), "second").unwrap();
        auto_commit(&repo, Path::new("b.md"), "User", "u1", None).unwrap();

        let head_after = repo.head().unwrap().target().unwrap();
        assert_ne!(head_before, head_after);

        // Verify linear history: HEAD → parent → parent → initial
        let head_commit = repo.find_commit(head_after).unwrap();
        assert_eq!(head_commit.parent_count(), 1);
        let parent = head_commit.parent(0).unwrap();
        assert_eq!(parent.parent_count(), 1);
    }

    #[test]
    fn auto_commit_works_on_unborn_branch() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();

        fs::write(dir.path().join("first.md"), "hello").unwrap();
        let oid = auto_commit(
            &repo,
            Path::new("first.md"),
            "New User",
            "new-user",
            None,
        )
        .unwrap();

        let commit = repo.find_commit(oid).unwrap();
        assert_eq!(commit.parent_count(), 0); // initial commit
        assert_eq!(commit.message().unwrap(), "edit: first");
    }

    #[test]
    fn auto_commit_nested_path() {
        let (dir, repo) = init_test_repo();

        let nested = dir.path().join("sub").join("dir");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("deep.md"), "deep content").unwrap();

        let oid = auto_commit(
            &repo,
            Path::new("sub/dir/deep.md"),
            "User",
            "u1",
            None,
        )
        .unwrap();

        let commit = repo.find_commit(oid).unwrap();
        assert_eq!(commit.message().unwrap(), "edit: deep");
    }

    #[test]
    fn open_repo_returns_none_outside_git() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(open_repo(dir.path()).is_none());
    }

    #[test]
    fn open_repo_returns_some_inside_git() {
        let dir = tempfile::TempDir::new().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        assert!(open_repo(dir.path()).is_some());
    }
}
