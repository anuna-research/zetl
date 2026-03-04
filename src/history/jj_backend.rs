//! JjBackend: thin wrapper around jj-lib for temporal graph navigation.
//!
//! Implements the [`VcsBackend`] trait, providing:
//! - Open or init a jj workspace (with optional git colocated backend)
//! - Working-copy snapshot creation (deduplicated by tree hash)
//! - File content reading via `MergedTree`
//! - Change-ID prefix resolution
//! - Revset-based commit traversal
//!
//! # Path layout
//!
//! The caller controls where the jj workspace root lives. For zetl production
//! use (see task-vcs-init), the workspace root will be configured so that jj
//! metadata is stored inside `.zetl/`. For unit tests a temp directory is used.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, anyhow, bail};
use jj_lib::backend::{ChangeId, CommitId, Signature, Timestamp};
use jj_lib::commit::Commit;
use jj_lib::config::{ConfigLayer, ConfigSource, StackedConfig};
use jj_lib::gitignore::GitIgnoreFile;
use jj_lib::matchers::EverythingMatcher;
use jj_lib::merged_tree::MergedTree;
use jj_lib::object_id::{HexPrefix, ObjectId as _, PrefixResolution};
use jj_lib::repo::{ReadonlyRepo, Repo as _, StoreFactories};
use jj_lib::repo_path::RepoPath;
use jj_lib::revset::RevsetExpression;
use jj_lib::settings::UserSettings;
use jj_lib::workspace::{Workspace, default_working_copy_factories};
use jj_lib::working_copy::SnapshotOptions;
use pollster::FutureExt as _;
use tokio::io::AsyncReadExt as _;

// ─── Public trait ────────────────────────────────────────────────────────────

/// Minimal VCS backend trait implemented by [`JjBackend`].
///
/// Future tasks may extend this with additional methods (e.g. diff, at-query).
pub trait VcsBackend {
    /// Snapshot the current working-copy state.
    ///
    /// Returns `Some(change_id_prefix)` when a new snapshot was committed, or
    /// `None` if the vault content is identical to the most recent snapshot
    /// (deduplication by tree hash).
    fn snapshot(&mut self, description: &str) -> anyhow::Result<Option<String>>;

    /// Read the bytes of `vault_path` (relative, forward-slash separated) at
    /// the commit identified by `change_id_prefix`.
    fn read_file_at(&self, change_id_prefix: &str, vault_path: &str) -> anyhow::Result<Vec<u8>>;

    /// Return metadata for the most recent `limit` commits in
    /// reverse-chronological order (newest first).
    fn list_changes(&self, limit: usize) -> anyhow::Result<Vec<ChangeInfo>>;

    /// Resolve a change-ID prefix (z-k alphabet) to its most recent commit's
    /// [`ChangeInfo`].
    fn resolve_change_id(&self, prefix: &str) -> anyhow::Result<ChangeInfo>;
}

// ─── Public data types ───────────────────────────────────────────────────────

/// Metadata about a single jj commit / snapshot.
#[derive(Debug, Clone)]
pub struct ChangeInfo {
    /// Change ID rendered as a reverse-hex (z-k) prefix string (12 chars).
    pub change_id: String,
    /// Commit ID as a hex string (12 chars).
    pub commit_id: String,
    /// Committer timestamp.
    pub timestamp: chrono::DateTime<chrono::FixedOffset>,
    /// Commit description (may be empty).
    pub description: String,
}

// ─── JjBackend ───────────────────────────────────────────────────────────────

/// jj-lib backed VCS layer for zetl temporal navigation.
pub struct JjBackend {
    workspace: Workspace,
    repo: Arc<ReadonlyRepo>,
}

impl JjBackend {
    /// Open an existing jj workspace at `workspace_root`, or initialise a new
    /// one.
    ///
    /// Initialisation strategy:
    /// - If a `.git/` directory is found at or above `workspace_root` →
    ///   colocated git backend (jj shares git's object storage)
    /// - Otherwise → internal git backend (jj manages its own bare git repo
    ///   inside `.jj/repo/store/git/`)
    ///
    /// Returns `Err` if the workspace root does not exist.
    pub fn open_or_init(workspace_root: &Path) -> anyhow::Result<Self> {
        if !workspace_root.exists() {
            bail!(
                "workspace root does not exist: {}",
                workspace_root.display()
            );
        }

        let settings = minimal_user_settings()?;
        let store_factories = StoreFactories::default();
        let wc_factories = default_working_copy_factories();

        // Try to load an existing workspace first.
        if workspace_root.join(".jj").is_dir() {
            let workspace =
                Workspace::load(&settings, workspace_root, &store_factories, &wc_factories)
                    .with_context(|| {
                        format!(
                            "failed to load jj workspace at {}",
                            workspace_root.display()
                        )
                    })?;
            let repo = workspace
                .repo_loader()
                .load_at_head()
                .with_context(|| "failed to load repo at head operation")?;
            return Ok(Self { workspace, repo });
        }

        // No existing workspace — initialise.
        let git_dir = find_git_dir(workspace_root);

        let (workspace, repo) = if git_dir.is_some() {
            // Colocated: use the existing git repo as the backend.
            Workspace::init_colocated_git(&settings, workspace_root).with_context(|| {
                format!(
                    "failed to init colocated jj workspace at {}",
                    workspace_root.display()
                )
            })?
        } else {
            // Standalone: jj manages its own internal git repo.
            Workspace::init_internal_git(&settings, workspace_root).with_context(|| {
                format!(
                    "failed to init jj workspace at {}",
                    workspace_root.display()
                )
            })?
        };

        Ok(Self { workspace, repo })
    }

    /// Return the workspace root (where vault files live).
    pub fn workspace_root(&self) -> &Path {
        self.workspace.workspace_root()
    }

    /// Return the `MergedTree` for the commit identified by `change_id_prefix`.
    pub fn tree_at_change(&self, change_id_prefix: &str) -> anyhow::Result<MergedTree> {
        let commit = self.commit_for_change_id(change_id_prefix)?;
        commit
            .tree()
            .with_context(|| format!("failed to read tree for change {change_id_prefix}"))
    }

    // ── Internal helpers ─────────────────────────────────────────────────────

    /// Resolve a change-ID prefix (reverse hex) to the corresponding commit.
    fn commit_for_change_id(&self, prefix: &str) -> anyhow::Result<Commit> {
        let hex_prefix = HexPrefix::try_from_reverse_hex(prefix.as_bytes())
            .ok_or_else(|| anyhow!("invalid change ID prefix: {prefix}"))?;

        let commit_ids = match self.repo.resolve_change_id_prefix(&hex_prefix) {
            PrefixResolution::NoMatch => {
                bail!("no commit found for change ID prefix: {prefix}")
            }
            PrefixResolution::AmbiguousMatch => {
                bail!("ambiguous change ID prefix: {prefix}")
            }
            PrefixResolution::SingleMatch(ids) => ids,
        };

        // Return the most recent commit among those with this change ID.
        commit_ids
            .iter()
            .filter_map(|id| self.repo.store().get_commit(id).ok())
            .max_by_key(|c| c.committer().timestamp)
            .ok_or_else(|| anyhow!("could not load commit for change ID prefix: {prefix}"))
    }

    /// Build [`ChangeInfo`] from a jj [`Commit`].
    fn change_info_from_commit(commit: &Commit) -> anyhow::Result<ChangeInfo> {
        let ts = commit
            .committer()
            .timestamp
            .to_datetime()
            .map_err(|_| anyhow!("commit timestamp out of range"))?;
        Ok(ChangeInfo {
            change_id: change_id_prefix(commit.change_id()),
            commit_id: commit_id_prefix(commit.id()),
            timestamp: ts,
            description: commit.description().to_owned(),
        })
    }
}

// ─── VcsBackend impl ─────────────────────────────────────────────────────────

impl VcsBackend for JjBackend {
    fn snapshot(&mut self, description: &str) -> anyhow::Result<Option<String>> {
        let base_ignores = GitIgnoreFile::empty();
        let snap_opts = SnapshotOptions {
            base_ignores,
            fsmonitor_settings: jj_lib::fsmonitor::FsmonitorSettings::None,
            progress: None,
            start_tracking_matcher: &EverythingMatcher,
            max_new_file_size: u64::MAX,
            conflict_marker_style: jj_lib::conflicts::ConflictMarkerStyle::default(),
        };

        // Cache workspace name before taking a mutable borrow.
        let workspace_name = self.workspace.workspace_name().to_owned();

        // Lock the working copy and snapshot the filesystem.
        let mut locked_ws = self
            .workspace
            .start_working_copy_mutation()
            .context("failed to start working copy mutation")?;

        let (new_tree_id, _stats) = locked_ws
            .locked_wc()
            .snapshot(&snap_opts)
            .context("working copy snapshot failed")?;

        // Check whether the tree has changed.
        let prev_tree_id = locked_ws.locked_wc().old_tree_id().clone();
        if new_tree_id == prev_tree_id {
            // Nothing changed — discard the lock without writing.
            drop(locked_ws);
            return Ok(None);
        }

        // Obtain the current working-copy commit.
        let wc_commit_id = self
            .repo
            .view()
            .get_wc_commit_id(&workspace_name)
            .cloned()
            .ok_or_else(|| anyhow!("no working-copy commit found"))?;
        // Create a new commit whose parent IS the current WC commit.
        // This builds a proper linear history chain:
        //   root → initial_wc → snap1 → snap2 → …
        let mut tx = self.repo.start_transaction();

        let now = Signature {
            name: "zetl".to_owned(),
            email: "zetl@localhost".to_owned(),
            timestamp: Timestamp::now(),
        };

        let new_commit = tx
            .repo_mut()
            .new_commit(vec![wc_commit_id], new_tree_id)
            .set_description(description)
            .set_author(now.clone())
            .set_committer(now)
            .write()
            .context("failed to write new commit")?;

        // Update the working-copy pointer.
        tx.repo_mut()
            .set_wc_commit(workspace_name, new_commit.id().clone())
            .context("failed to update working-copy pointer")?;

        let new_repo = tx
            .commit("zetl snapshot")
            .context("failed to commit transaction")?;

        // Finish the working-copy lock.
        locked_ws
            .finish(new_repo.op_id().clone())
            .context("failed to finalise working copy")?;

        self.repo = new_repo;

        let change_id = change_id_prefix(new_commit.change_id());
        Ok(Some(change_id))
    }

    fn read_file_at(&self, change_id_prefix: &str, vault_path: &str) -> anyhow::Result<Vec<u8>> {
        let tree = self.tree_at_change(change_id_prefix)?;

        let repo_path = RepoPath::from_internal_string(vault_path)
            .with_context(|| format!("invalid repo path: {vault_path}"))?;

        let value = tree
            .path_value(repo_path)
            .with_context(|| format!("failed to read tree entry for {vault_path}"))?;

        let file_id = value
            .into_resolved()
            .ok()
            .flatten()
            .and_then(|tv| {
                if let jj_lib::backend::TreeValue::File { id, .. } = tv {
                    Some(id)
                } else {
                    None
                }
            })
            .ok_or_else(|| {
                anyhow!("path {vault_path} is not a regular file at change {change_id_prefix}")
            })?;

        // Read from the store (async → sync via pollster).
        let store = tree.store().clone();
        let repo_path_owned = repo_path.to_owned();
        let content: Vec<u8> = async move {
            let mut reader = store
                .read_file(&repo_path_owned, &file_id)
                .await
                .map_err(|e| anyhow!("backend read_file: {e}"))?;
            let mut buf = Vec::new();
            reader
                .read_to_end(&mut buf)
                .await
                .context("reading file content")?;
            Ok::<Vec<u8>, anyhow::Error>(buf)
        }
        .block_on()?;

        Ok(content)
    }

    fn list_changes(&self, limit: usize) -> anyhow::Result<Vec<ChangeInfo>> {
        // Walk all visible ancestors, take the newest `limit` commits.
        // We add +2 to the limit to absorb the root commit and the initial
        // empty WC commit that jj creates automatically on workspace init.
        let expr = RevsetExpression::visible_heads()
            .ancestors()
            .latest(limit + 2);

        let resolved = expr
            .evaluate(self.repo.as_ref())
            .context("failed to evaluate revset")?;

        let root_commit_id = self.repo.store().root_commit_id().clone();

        let mut results = Vec::new();
        for commit_id in resolved.iter() {
            let commit_id = commit_id.context("revset iteration error")?;
            let commit = self
                .repo
                .store()
                .get_commit(&commit_id)
                .context("failed to load commit")?;

            // Skip the root commit (no parents).
            if commit.parent_ids().is_empty() {
                continue;
            }

            // Skip the initial empty WC commit jj creates on workspace init:
            // it has the root commit as its sole parent and is not a zetl
            // snapshot.
            let parents = commit.parent_ids();
            if parents.len() == 1 && parents[0] == root_commit_id {
                continue;
            }

            results.push(Self::change_info_from_commit(&commit)?);
            if results.len() >= limit {
                break;
            }
        }
        Ok(results)
    }

    fn resolve_change_id(&self, prefix: &str) -> anyhow::Result<ChangeInfo> {
        let commit = self.commit_for_change_id(prefix)?;
        Self::change_info_from_commit(&commit)
    }
}

// ─── Utilities ────────────────────────────────────────────────────────────────

/// Build a minimal [`UserSettings`] without reading any config file.
fn minimal_user_settings() -> anyhow::Result<UserSettings> {
    let mut config = StackedConfig::with_defaults();
    let mut layer = ConfigLayer::empty(ConfigSource::User);
    layer
        .set_value("user.name", "zetl")
        .map_err(|e| anyhow!("config set user.name: {e}"))?;
    layer
        .set_value("user.email", "zetl@localhost")
        .map_err(|e| anyhow!("config set user.email: {e}"))?;
    config.add_layer(layer);
    UserSettings::from_config(config).context("failed to build UserSettings")
}

/// Walk parent directories of `start` looking for a `.git` directory.
fn find_git_dir(start: &Path) -> Option<PathBuf> {
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

/// First 12 characters of a change ID in reverse-hex (z-k alphabet).
fn change_id_prefix(id: &ChangeId) -> String {
    let full = id.reverse_hex();
    full[..full.len().min(12)].to_owned()
}

/// First 12 characters of a commit ID in hex.
fn commit_id_prefix(id: &CommitId) -> String {
    let full = id.hex();
    full[..full.len().min(12)].to_owned()
}

// ─── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn write_file(root: &Path, name: &str, content: &str) {
        fs::write(root.join(name), content).unwrap();
    }

    #[test]
    fn test_open_or_init_creates_workspace() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        write_file(root, "note.md", "# Hello\n[[World]]");

        let backend = JjBackend::open_or_init(root).expect("open_or_init should succeed");
        assert_eq!(
            backend.workspace_root(),
            root.canonicalize().unwrap(),
            "workspace root should match"
        );
        assert!(root.join(".jj").is_dir(), ".jj directory should exist");
    }

    #[test]
    fn test_open_or_init_is_idempotent() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        write_file(root, "a.md", "hello");

        JjBackend::open_or_init(root).expect("first init");
        JjBackend::open_or_init(root).expect("second open should succeed without error");
    }

    #[test]
    fn test_snapshot_returns_change_id() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        write_file(root, "page.md", "# Test page");

        let mut backend = JjBackend::open_or_init(root).unwrap();
        let result = backend.snapshot("test: first snapshot").unwrap();
        assert!(result.is_some(), "first snapshot should produce a change ID");
        let change_id = result.unwrap();
        assert!(!change_id.is_empty(), "change ID should be non-empty");
    }

    #[test]
    fn test_snapshot_deduplicates_unchanged_content() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        write_file(root, "page.md", "# No changes");

        let mut backend = JjBackend::open_or_init(root).unwrap();
        let first = backend.snapshot("first").unwrap();
        assert!(first.is_some(), "first snapshot should produce a change ID");

        let second = backend.snapshot("second: no file change").unwrap();
        assert!(
            second.is_none(),
            "second snapshot with no file changes should return None"
        );
    }

    #[test]
    fn test_snapshot_detects_new_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        write_file(root, "a.md", "first");

        let mut backend = JjBackend::open_or_init(root).unwrap();
        let first = backend.snapshot("snap 1").unwrap();
        assert!(first.is_some());

        write_file(root, "b.md", "second page");
        let second = backend.snapshot("snap 2").unwrap();
        assert!(
            second.is_some(),
            "adding a new file should produce a new snapshot"
        );
    }

    #[test]
    fn test_list_changes_empty_before_snapshot() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        write_file(root, "a.md", "content");

        let backend = JjBackend::open_or_init(root).unwrap();
        let changes = backend.list_changes(10).unwrap();
        // Only the root commit exists, which we filter out.
        assert!(
            changes.is_empty(),
            "expected no user commits before any snapshot"
        );
    }

    #[test]
    fn test_list_changes_after_snapshots() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        write_file(root, "a.md", "first state");

        let mut backend = JjBackend::open_or_init(root).unwrap();
        backend.snapshot("snap 1").unwrap();

        write_file(root, "b.md", "second page");
        backend.snapshot("snap 2").unwrap();

        let changes = backend.list_changes(10).unwrap();
        assert_eq!(changes.len(), 2, "should have 2 snapshots: {changes:?}");
    }

    #[test]
    fn test_resolve_change_id_roundtrip() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        write_file(root, "r.md", "roundtrip");

        let mut backend = JjBackend::open_or_init(root).unwrap();
        let change_id = backend
            .snapshot("roundtrip test")
            .unwrap()
            .expect("should have created a snapshot");

        let info = backend
            .resolve_change_id(&change_id)
            .expect("resolve_change_id should succeed");
        assert_eq!(info.description, "roundtrip test");
        assert!(!info.commit_id.is_empty());
    }

    #[test]
    fn test_read_file_at() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        write_file(root, "note.md", "# Note\nContent here.");

        let mut backend = JjBackend::open_or_init(root).unwrap();
        let change_id = backend
            .snapshot("read file test")
            .unwrap()
            .expect("snapshot required");

        let content = backend
            .read_file_at(&change_id, "note.md")
            .expect("read_file_at should succeed");

        assert_eq!(
            String::from_utf8(content).unwrap(),
            "# Note\nContent here."
        );
    }

    #[test]
    fn test_read_file_at_missing_path_errors() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        write_file(root, "exists.md", "exists");

        let mut backend = JjBackend::open_or_init(root).unwrap();
        let change_id = backend.snapshot("test").unwrap().unwrap();

        let result = backend.read_file_at(&change_id, "does-not-exist.md");
        assert!(result.is_err(), "missing file should error");
    }
}
