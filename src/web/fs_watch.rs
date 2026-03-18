//! Filesystem watch for external edits (REQ-020-039).
//!
//! Detects changes to vault files that happen outside of zetl's own pipeline
//! (e.g. from text editors, git operations, agents, CI). The detection
//! mechanism uses a `PendingWrites` set: files that zetl is currently writing
//! are registered before the write and cleared after; any FS event for a path
//! NOT in the set is treated as an external edit.
//!
//! The reconciliation pipeline runs debounced (500ms window) and performs:
//! 1. Re-scan changed files (reindex)
//! 2. Recompute vault_root_hash (stop if unchanged)
//! 3. Invalidate ACL cache
//! 4. Rebuild search index
//! 5. Update link graph (swap VaultData)
//! 6. jj snapshot
//! 7. Notify connected editors via WebSocket

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::merkle::{compute_file_root, compute_vault_root};
use crate::search_index::SearchIndex;
use crate::types::ContentHash;

use super::ws::ServerMsg;
use super::WebState;

/// Debounce window for batching filesystem events (REQ-020-039 step 1).
const DEBOUNCE_MS: u64 = 500;

/// Tracks files that zetl is currently writing, so FS events for those
/// paths can be filtered out (they are internal, not external edits).
#[derive(Clone, Default)]
pub struct PendingWrites {
    inner: Arc<Mutex<HashSet<PathBuf>>>,
}

impl PendingWrites {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a path as being written by zetl. Call before `fs::write`.
    pub fn insert(&self, path: &Path) {
        if let Ok(mut set) = self.inner.lock() {
            set.insert(path.to_path_buf());
        }
    }

    /// Clear a path after zetl finishes writing. Call after `fs::write`.
    pub fn remove(&self, path: &Path) {
        if let Ok(mut set) = self.inner.lock() {
            set.remove(path);
        }
    }

    /// Check if a path is currently being written by zetl.
    pub fn contains(&self, path: &Path) -> bool {
        self.inner
            .lock()
            .map(|set| set.contains(path))
            .unwrap_or(false)
    }

    /// Drain all currently pending paths and return them (used in tests).
    #[cfg(test)]
    pub fn drain(&self) -> HashSet<PathBuf> {
        self.inner
            .lock()
            .map(|mut set| set.drain().collect())
            .unwrap_or_default()
    }
}

/// Run the reconciliation pipeline for external edits (REQ-020-039 steps 2–7).
///
/// Returns the new `vault_root_hash` if the vault state changed, or `None` if
/// the hash is unchanged (meaning the edits were no-ops or reverts).
pub fn reconcile_external_edits(
    state: &WebState,
    changed_paths: &[PathBuf],
) -> Option<String> {
    // ── Step 2: Re-scan ──────────────────────────────────────────────────
    let new_data = match super::reindex(&state.vault_root) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("fs-watch: reindex error: {e}");
            return None;
        }
    };

    // ── Step 3: Recompute vault_root_hash ────────────────────────────────
    let file_hashes: Vec<(&Path, ContentHash)> = new_data
        .files
        .iter()
        .map(|f| {
            let root = compute_file_root(&f.merkle_leaves);
            (f.path.as_path(), root)
        })
        .collect();
    let vault_root_bytes = compute_vault_root(&file_hashes);
    let vault_root_hash: String = vault_root_bytes.iter().map(|b| format!("{b:02x}")).collect();

    // Two-tier cache check (SPEC-006): if vault_root_hash unchanged, stop.
    {
        let current_data = state.data.read().unwrap();
        let current_hashes: Vec<(&Path, ContentHash)> = current_data
            .files
            .iter()
            .map(|f| {
                let root = compute_file_root(&f.merkle_leaves);
                (f.path.as_path(), root)
            })
            .collect();
        let current_root_bytes = compute_vault_root(&current_hashes);
        let current_hash: String = current_root_bytes.iter().map(|b| format!("{b:02x}")).collect();
        if current_hash == vault_root_hash {
            return None; // No actual change
        }
    }

    // ── Step 4: Invalidate ACL cache ─────────────────────────────────────
    #[cfg(feature = "reason")]
    {
        if let Ok(mut cache) = state.acl_cache.lock() {
            cache.invalidate_if_changed(Some(&vault_root_hash));
        }
    }

    // ── Step 5: Rebuild search index ─────────────────────────────────────
    if let Err(e) = SearchIndex::build(&state.vault_root, &new_data.files) {
        eprintln!("fs-watch: search index rebuild error: {e}");
    }

    // ── Step 6: Update link graph (swap VaultData) ───────────────────────
    *state.data.write().unwrap() = new_data;

    // ── Step 7: jj snapshot ──────────────────────────────────────────────
    #[cfg(feature = "history")]
    {
        match crate::history::auto_snapshot(&state.vault_root, Some(&vault_root_hash)) {
            Ok(Some(change_id)) => {
                eprintln!("fs-watch: jj snapshot {change_id}");
            }
            Ok(None) => { /* deduplicated */ }
            Err(e) => {
                eprintln!("fs-watch: jj snapshot error: {e}");
            }
        }
    }

    // ── Step 8: Notify connected editors ─────────────────────────────────
    let changed_slugs: Vec<String> = changed_paths
        .iter()
        .filter_map(|p| {
            p.strip_prefix(state.vault_root.as_ref())
                .ok()
                .and_then(|rel| {
                    rel.to_string_lossy()
                        .strip_suffix(".md")
                        .map(|s| s.to_string())
                })
        })
        .collect();

    let msg = ServerMsg::ExternalEdit {
        files: changed_slugs,
    };
    state.ws_hub.broadcast_all(msg);

    Some(vault_root_hash)
}

/// Spawn the filesystem watcher background task (REQ-020-039).
///
/// Watches the vault root for file changes, filters out zetl's own writes
/// via the `PendingWrites` set, debounces events over a 500ms window, and
/// runs the reconciliation pipeline for external edits.
pub fn spawn_fs_watcher(state: WebState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<PathBuf>>();
        let vault_root = state.vault_root.clone();
        let pending_writes = state.pending_writes.clone();

        // Spawn the notify watcher on a blocking thread.
        let _watcher_handle = std::thread::spawn(move || {
            let debounce_duration = Duration::from_millis(DEBOUNCE_MS);
            let mut batch: Vec<PathBuf> = Vec::new();
            let mut last_event: Option<Instant> = None;

            let (notify_tx, notify_rx) = std::sync::mpsc::channel();

            let mut watcher: RecommendedWatcher =
                match Watcher::new(notify_tx, notify::Config::default()) {
                    Ok(w) => w,
                    Err(e) => {
                        eprintln!("fs-watch: failed to create watcher: {e}");
                        return;
                    }
                };

            if let Err(e) = watcher.watch(vault_root.as_ref(), RecursiveMode::Recursive) {
                eprintln!("fs-watch: failed to watch vault root: {e}");
                return;
            }

            eprintln!("fs-watch: watching {}", vault_root.display());

            loop {
                // Wait for events with a timeout for debounce flushing.
                let timeout = if last_event.is_some() {
                    debounce_duration
                } else {
                    Duration::from_secs(60)
                };

                match notify_rx.recv_timeout(timeout) {
                    Ok(Ok(event)) => {
                        if let Some(paths) = filter_event(&event, &vault_root, &pending_writes) {
                            batch.extend(paths);
                            last_event = Some(Instant::now());
                        }
                    }
                    Ok(Err(e)) => {
                        eprintln!("fs-watch: watcher error: {e}");
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        // Debounce window elapsed — flush batch.
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        break;
                    }
                }

                // Flush batch if debounce window has elapsed.
                if let Some(ts) = last_event {
                    if ts.elapsed() >= debounce_duration && !batch.is_empty() {
                        let paths: Vec<PathBuf> = batch.drain(..).collect();
                        // Deduplicate
                        let unique: Vec<PathBuf> = {
                            let mut seen = HashSet::new();
                            paths.into_iter().filter(|p| seen.insert(p.clone())).collect()
                        };
                        let _ = tx.send(unique);
                        last_event = None;
                    }
                }
            }
        });

        // Process debounced batches on the async side.
        while let Some(changed_paths) = rx.recv().await {
            let state_clone = state.clone();
            let _ = tokio::task::spawn_blocking(move || {
                reconcile_external_edits(&state_clone, &changed_paths);
            })
            .await;
        }
    })
}

/// Filter a notify event, returning the relevant markdown paths that
/// represent external edits (not in pending_writes, not hidden dirs).
fn filter_event(
    event: &Event,
    vault_root: &Path,
    pending_writes: &PendingWrites,
) -> Option<Vec<PathBuf>> {
    // Only react to creates, modifies, and removes.
    match event.kind {
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {}
        _ => return None,
    }

    let mut external_paths = Vec::new();

    for path in &event.paths {
        // Skip non-markdown files.
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "md" {
            continue;
        }

        // Skip hidden directories (.zetl, .git, etc.).
        if let Ok(rel) = path.strip_prefix(vault_root) {
            let is_hidden = rel
                .components()
                .any(|c| c.as_os_str().to_string_lossy().starts_with('.'));
            if is_hidden {
                continue;
            }
        }

        // Skip files zetl is currently writing.
        if pending_writes.contains(path) {
            // Clear the pending write now that we've seen the event.
            pending_writes.remove(path);
            continue;
        }

        external_paths.push(path.clone());
    }

    if external_paths.is_empty() {
        None
    } else {
        Some(external_paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_writes_insert_contains_remove() {
        let pw = PendingWrites::new();
        let path = PathBuf::from("/vault/hello.md");

        assert!(!pw.contains(&path));
        pw.insert(&path);
        assert!(pw.contains(&path));
        pw.remove(&path);
        assert!(!pw.contains(&path));
    }

    #[test]
    fn filter_event_skips_pending_write() {
        let vault = PathBuf::from("/vault");
        let pw = PendingWrites::new();
        let path = vault.join("hello.md");

        pw.insert(&path);

        let event = Event {
            kind: EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Content,
            )),
            paths: vec![path.clone()],
            attrs: Default::default(),
        };

        let result = filter_event(&event, &vault, &pw);
        assert!(result.is_none(), "pending write should be filtered out");
        // pending write should be cleared after filtering
        assert!(!pw.contains(&path));
    }

    #[test]
    fn filter_event_passes_external_edit() {
        let vault = PathBuf::from("/vault");
        let pw = PendingWrites::new();
        let path = vault.join("hello.md");

        let event = Event {
            kind: EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Content,
            )),
            paths: vec![path.clone()],
            attrs: Default::default(),
        };

        let result = filter_event(&event, &vault, &pw);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), vec![path]);
    }

    #[test]
    fn filter_event_skips_hidden_dirs() {
        let vault = PathBuf::from("/vault");
        let pw = PendingWrites::new();

        let event = Event {
            kind: EventKind::Create(notify::event::CreateKind::File),
            paths: vec![vault.join(".zetl/data.md"), vault.join(".git/refs.md")],
            attrs: Default::default(),
        };

        let result = filter_event(&event, &vault, &pw);
        assert!(result.is_none(), "hidden dir files should be filtered");
    }

    #[test]
    fn filter_event_skips_non_markdown() {
        let vault = PathBuf::from("/vault");
        let pw = PendingWrites::new();

        let event = Event {
            kind: EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Content,
            )),
            paths: vec![vault.join("image.png"), vault.join("script.js")],
            attrs: Default::default(),
        };

        let result = filter_event(&event, &vault, &pw);
        assert!(result.is_none(), "non-markdown files should be filtered");
    }

    /// TEST-020-039: External file write detected; vault state updated.
    #[test]
    fn reconcile_updates_vault_state() {
        use crate::search_index::SearchIndex;
        use crate::web::ws::CrdtDocStore;
        use std::sync::{Arc, Mutex, RwLock};

        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("hello.md"), "# Hello\n\nworld\n").unwrap();

        let vault_root = Arc::new(dir.path().to_path_buf());
        let data = super::super::reindex(dir.path()).unwrap();
        let search_index = SearchIndex::build(dir.path(), &data.files).unwrap();
        let engine =
            crate::web::engine::TemplateEngine::new(dir.path(), "default", false, false);

        let state = WebState {
            data: Arc::new(RwLock::new(data)),
            vault_root: vault_root.clone(),
            search_index: Arc::new(search_index),
            engine: Arc::new(engine),
            theme: "default".to_string(),
            verbose: false,
            collab: false,
            sessions: crate::web::session::SessionStore::new(),
            recovery_challenges: Arc::new(
                crate::user::recovery::RecoveryChallengeStore::new(),
            ),
            mnemonic_shown: Arc::new(Mutex::new(std::collections::HashSet::new())),
            rate_limiters: crate::web::rate_limit::AuthRateLimiters::new(),
            #[cfg(feature = "reason")]
            acl_cache: Arc::new(Mutex::new(crate::web::AclCache::new())),
            git_commit_lock: None,
            ws_hub: crate::web::ws::WsHub::new(),
            ticket_store: crate::web::ws::TicketStore::new(),
            crdt_store: CrdtDocStore::new(vault_root.clone()),
            wal_store: Arc::new(crate::web::wal::WalStore::new(&vault_root)),
            pending_writes: PendingWrites::new(),
            #[cfg(feature = "semantic")]
            vector_index: None,
        };

        // Simulate external edit: add a new file.
        let new_file = dir.path().join("external.md");
        std::fs::write(&new_file, "# External\n\nAdded by editor\n").unwrap();

        let result = reconcile_external_edits(&state, &[new_file]);
        assert!(result.is_some(), "vault_root_hash should change after external edit");

        // Verify VaultData was updated to include the new file.
        let data = state.data.read().unwrap();
        assert!(
            data.files.iter().any(|f| f.page_name == "external"),
            "external page should appear in VaultData after reconciliation"
        );
    }
}
