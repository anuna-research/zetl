//! Git HEAD polling for external commit detection (REQ-020-041).
//!
//! Periodically reads the current HEAD commit hash via `git2`. When HEAD
//! advances (e.g. after a `git pull` or `git merge`), the poller diffs the
//! old tree against the new tree to identify changed `.md` files, then feeds
//! them into the reconciliation pipeline (REQ-020-039 steps 2–8).
//!
//! This catches changes that filesystem watchers may miss (NFS mounts,
//! Docker volume sync, etc.) and provides a reliable fallback.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use super::fs_watch::reconcile_external_edits;
use super::git_commit::GitCommitLock;
use super::WebState;

/// Spawn the git HEAD polling background task (REQ-020-041).
///
/// Polls at the given `interval`. If `interval` is zero, polling is disabled.
/// Returns `None` when disabled or when the vault has no git repository.
pub fn spawn_git_poller(
    state: WebState,
    interval: Duration,
    git_lock: Arc<GitCommitLock>,
) -> Option<tokio::task::JoinHandle<()>> {
    if interval.is_zero() {
        eprintln!("git-poll: disabled (interval=0)");
        return None;
    }

    Some(tokio::spawn(async move {
        // Read initial HEAD so we have a baseline.
        let mut last_head = {
            let repo = git_lock.lock().unwrap();
            read_head_oid(&repo)
        };

        eprintln!(
            "git-poll: watching HEAD every {}s (current: {})",
            interval.as_secs(),
            last_head
                .as_ref()
                .map(|o| o.to_string()[..7].to_string())
                .unwrap_or_else(|| "(none)".into())
        );

        loop {
            tokio::time::sleep(interval).await;

            let (new_head, changed_paths) = {
                let repo = git_lock.lock().unwrap();
                let current = read_head_oid(&repo);

                if current == last_head {
                    continue;
                }

                let paths = diff_trees(&repo, last_head.as_ref(), current.as_ref(), &state.vault_root);
                (current, paths)
            };

            if changed_paths.is_empty() {
                // HEAD changed but no .md files affected — update baseline and move on.
                last_head = new_head;
                continue;
            }

            let short_old = last_head
                .as_ref()
                .map(|o| o.to_string()[..7].to_string())
                .unwrap_or_else(|| "(none)".into());
            let short_new = new_head
                .as_ref()
                .map(|o| o.to_string()[..7].to_string())
                .unwrap_or_else(|| "(none)".into());

            eprintln!(
                "git-poll: HEAD advanced {} → {} ({} file(s) changed)",
                short_old,
                short_new,
                changed_paths.len()
            );

            last_head = new_head;

            // Run reconciliation on a blocking thread (it does I/O-heavy re-indexing).
            let state_clone = state.clone();
            let _ = tokio::task::spawn_blocking(move || {
                reconcile_external_edits(&state_clone, &changed_paths);
            })
            .await;
        }
    }))
}

/// Read the OID of the current HEAD commit, or `None` for an unborn branch.
fn read_head_oid(repo: &git2::Repository) -> Option<git2::Oid> {
    repo.head().ok().and_then(|r| r.target())
}

/// Diff two commit trees and return the vault-relative paths of changed `.md` files.
///
/// If `old_oid` is `None` (initial state unknown), treats all `.md` files in
/// `new_oid`'s tree as changed. If `new_oid` is `None`, returns empty (HEAD
/// deleted — unusual but not actionable).
fn diff_trees(
    repo: &git2::Repository,
    old_oid: Option<&git2::Oid>,
    new_oid: Option<&git2::Oid>,
    vault_root: &std::path::Path,
) -> Vec<PathBuf> {
    let new_oid = match new_oid {
        Some(o) => o,
        None => return Vec::new(),
    };

    let new_tree = repo
        .find_commit(*new_oid)
        .and_then(|c| c.tree())
        .ok();

    let old_tree = old_oid
        .and_then(|o| repo.find_commit(*o).ok())
        .and_then(|c| c.tree().ok());

    let diff = match repo.diff_tree_to_tree(old_tree.as_ref(), new_tree.as_ref(), None) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("git-poll: diff error: {e}");
            return Vec::new();
        }
    };

    // Resolve the repo workdir to compute absolute paths.
    let workdir = repo
        .workdir()
        .unwrap_or(vault_root);

    let mut paths = Vec::new();
    for delta in diff.deltas() {
        // Check both old and new file paths (handles renames).
        for file in [delta.new_file(), delta.old_file()] {
            if let Some(p) = file.path() {
                let p_str = p.to_string_lossy();
                if p_str.ends_with(".md") {
                    let abs = workdir.join(p);
                    if !paths.contains(&abs) {
                        paths.push(abs);
                    }
                }
            }
        }
    }

    paths
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    /// Create a temporary git repo with an initial commit containing a .md file.
    fn init_test_repo() -> (tempfile::TempDir, git2::Repository) {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();

        let mut config = repo.config().unwrap();
        config.set_str("user.name", "test").unwrap();
        config.set_str("user.email", "test@test").unwrap();

        // Initial commit with a .md file.
        let sig = git2::Signature::now("test", "test@test").unwrap();
        fs::write(dir.path().join("hello.md"), "# Hello").unwrap();
        let tree_oid = {
            let mut index = repo.index().unwrap();
            index.add_path(Path::new("hello.md")).unwrap();
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

    /// Helper: commit a file change and return the new HEAD OID.
    fn commit_file(
        repo: &git2::Repository,
        dir: &Path,
        rel_path: &str,
        content: &str,
    ) -> git2::Oid {
        fs::write(dir.join(rel_path), content).unwrap();
        let sig = git2::Signature::now("test", "test@test").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new(rel_path)).unwrap();
        index.write().unwrap();
        let tree_oid = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "update", &tree, &[&parent])
            .unwrap()
    }

    #[test]
    fn read_head_oid_returns_some_for_repo_with_commits() {
        let (_dir, repo) = init_test_repo();
        assert!(read_head_oid(&repo).is_some());
    }

    #[test]
    fn read_head_oid_returns_none_for_empty_repo() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        assert!(read_head_oid(&repo).is_none());
    }

    #[test]
    fn diff_trees_detects_changed_md_files() {
        let (dir, repo) = init_test_repo();
        let old_head = read_head_oid(&repo).unwrap();

        // Commit a change to hello.md and add a new file.
        fs::write(dir.path().join("hello.md"), "# Hello World").unwrap();
        fs::write(dir.path().join("new.md"), "# New").unwrap();
        let sig = git2::Signature::now("test", "test@test").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("hello.md")).unwrap();
        index.add_path(Path::new("new.md")).unwrap();
        index.write().unwrap();
        let tree_oid = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "update", &tree, &[&parent])
            .unwrap();

        let new_head = read_head_oid(&repo).unwrap();
        let changed = diff_trees(&repo, Some(&old_head), Some(&new_head), dir.path());

        assert_eq!(changed.len(), 2);
        let names: Vec<String> = changed
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"hello.md".to_string()));
        assert!(names.contains(&"new.md".to_string()));
    }

    #[test]
    fn diff_trees_ignores_non_md_files() {
        let (dir, repo) = init_test_repo();
        let old_head = read_head_oid(&repo).unwrap();

        commit_file(&repo, dir.path(), "data.json", r#"{"key": "val"}"#);

        let new_head = read_head_oid(&repo).unwrap();
        let changed = diff_trees(&repo, Some(&old_head), Some(&new_head), dir.path());

        assert!(changed.is_empty());
    }

    #[test]
    fn diff_trees_handles_none_old_oid() {
        let (dir, repo) = init_test_repo();
        let head = read_head_oid(&repo).unwrap();

        // With no old OID, all .md files in the new tree should appear.
        let changed = diff_trees(&repo, None, Some(&head), dir.path());
        assert!(!changed.is_empty());
    }

    #[test]
    fn diff_trees_handles_none_new_oid() {
        let (_dir, repo) = init_test_repo();
        let head = read_head_oid(&repo).unwrap();

        let changed = diff_trees(&repo, Some(&head), None, _dir.path());
        assert!(changed.is_empty());
    }

    #[test]
    fn diff_trees_multiple_commits() {
        let (dir, repo) = init_test_repo();
        let old_head = read_head_oid(&repo).unwrap();

        // Make two commits, only the diff between old and latest HEAD matters.
        commit_file(&repo, dir.path(), "a.md", "# A");
        commit_file(&repo, dir.path(), "b.md", "# B");

        let new_head = read_head_oid(&repo).unwrap();
        let changed = diff_trees(&repo, Some(&old_head), Some(&new_head), dir.path());

        let names: Vec<String> = changed
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"a.md".to_string()));
        assert!(names.contains(&"b.md".to_string()));
    }
}
