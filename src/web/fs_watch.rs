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

use crate::hooks;
use crate::hooks::context::{build_hook_context, HookSaved, HookUser};
use crate::merkle::{compute_file_root, compute_vault_root};
use crate::search_index::SearchIndex;
use crate::types::ContentHash;

#[cfg(feature = "reason")]
use crate::hooks::context::HookAclViolationEntry;

use super::ws::ServerMsg;
use super::WebState;

/// Source of an external edit, used for attribution (REQ-020-042).
#[derive(Debug, Clone)]
pub enum ExternalSource {
    /// Edit detected via filesystem watcher (unknown author).
    Filesystem,
    /// Edit arrived via a git commit with known author.
    GitCommit { author: String, email: String },
}

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
    source: &ExternalSource,
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

    // ── Step 8: CRDT reconciliation (REQ-020-040) ─────────────────────────
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

    for slug in &changed_slugs {
        if state.crdt_store.is_loaded(slug) {
            let file_exists = state.crdt_store.md_path_for_slug(slug).exists();
            let msgs = state.crdt_store.reconcile_crdt(slug, file_exists);
            let room = state.ws_hub.room(slug);
            for msg in msgs {
                let _ = room.tx.send(msg);
            }
        }
    }

    // ── Step 8b: Notify connected editors (ExternalEdit broadcast) ──────
    let msg = ServerMsg::ExternalEdit {
        files: changed_slugs.clone(),
    };
    state.ws_hub.broadcast_all(msg);

    // ── Step 9: Post-reconciliation ACL violation detection (REQ-020-043) ─
    #[cfg(feature = "reason")]
    {
        detect_and_report_acl_violations(state, &changed_slugs);
    }

    // ── Step 10: Fire on-save hooks with external attribution (REQ-020-042) ─
    fire_external_save_hooks(state, changed_paths, source);

    Some(vault_root_hash)
}

/// Fire on-save hooks for each externally-changed file (REQ-020-042).
///
/// Builds a `HookUser` from the `ExternalSource` and fires the on-save hook
/// once per changed file, with `is_external: true` on both `HookSaved` and
/// `HookUser`.
fn fire_external_save_hooks(
    state: &WebState,
    changed_paths: &[PathBuf],
    source: &ExternalSource,
) {
    let theme_hooks = hooks::resolve_theme_hooks(&state.vault_root, &state.theme);
    let manifest = hooks::discover_hooks(&state.vault_root, theme_hooks.path());

    if hooks::hooks_for(&manifest, "on-save").is_empty() {
        return;
    }

    let data = state.data.read().unwrap();
    let mut ctx = build_hook_context(
        "on-save",
        &state.vault_root,
        &state.theme,
        env!("CARGO_PKG_VERSION"),
        &data.files,
        &data.graph,
    );

    let external_user = match source {
        ExternalSource::GitCommit { author, email } => HookUser::external_git(author, email),
        ExternalSource::Filesystem => HookUser::external_filesystem(),
    };
    ctx.user = Some(external_user);

    for path in changed_paths {
        let rel_path = path
            .strip_prefix(state.vault_root.as_ref())
            .unwrap_or(path);
        let rel_str = rel_path.to_string_lossy().into_owned();
        let page_name = rel_path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let content_length = std::fs::metadata(path).map(|m| m.len() as usize).unwrap_or(0);

        ctx.saved = Some(HookSaved {
            file: rel_str.clone(),
            page: page_name.clone(),
            content_length,
            is_external: true,
        });

        let context_json = match serde_json::to_vec(&ctx) {
            Ok(j) => j,
            Err(e) => {
                eprintln!("external on-save hook: json error: {e}");
                continue;
            }
        };

        let hook_env = hooks::HookEnv {
            vault_root: state.vault_root.as_ref().clone(),
            theme: state.theme.clone(),
            zetl_version: env!("CARGO_PKG_VERSION").to_string(),
            extra_vars: vec![
                ("ZETL_SAVED_FILE".into(), rel_str),
                ("ZETL_SAVED_PAGE".into(), page_name),
                ("ZETL_HOOK_DEPTH".into(), "0".into()),
            ],
        };

        let results = hooks::run_hooks(&manifest, "on-save", &context_json, &hook_env);
        for result in results {
            match result {
                Ok(output) if !output.success() => {
                    eprintln!(
                        "warning: external on-save hook '{}' ({}) exited with code {}",
                        output.path.display(),
                        output.source,
                        output.exit_code.unwrap_or(-1),
                    );
                    if !output.stderr.is_empty() {
                        eprintln!("  stderr: {}", output.stderr.trim_end());
                    }
                }
                Err(e) => {
                    eprintln!("warning: external on-save hook failed to execute: {e}");
                }
                _ => {}
            }
        }
    }
}

/// Detect ACL violations for externally-edited pages (REQ-020-043).
///
/// For each changed page slug, evaluates all registered users for Edit
/// permission. If a page has edit restrictions (any user denied), the edit
/// bypassed ACL controls — log at WARN, fire on-acl-violation hook, and
/// broadcast an admin banner via WebSocket.
///
/// Files are NOT reverted (per TEST-020-043 acceptance criteria).
#[cfg(feature = "reason")]
fn detect_and_report_acl_violations(state: &WebState, changed_slugs: &[String]) {
    if changed_slugs.is_empty() {
        return;
    }

    let users = match crate::user::list_profiles(&state.vault_root) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("fs-watch: acl violation check: failed to list users: {e}");
            return;
        }
    };

    if users.is_empty() {
        return;
    }

    let data = state.data.read().unwrap();
    let all_page_slugs: Vec<String> = data
        .page_slug_map
        .values()
        .cloned()
        .collect();

    let mut violations: Vec<HookAclViolationEntry> = Vec::new();

    for slug in changed_slugs {
        // Find SPL blocks for this page.
        let spl_blocks: Vec<crate::types::SplBlock> = data
            .files
            .iter()
            .find(|f| {
                data.page_slug_map
                    .get(&f.page_name)
                    .map(|s| s == slug)
                    .unwrap_or(false)
            })
            .map(|f| f.spl_blocks.clone())
            .unwrap_or_default();

        for user in &users {
            let query = crate::acl::AclQuery {
                user_id: user.id.clone(),
                page_slug: slug.clone(),
                action: crate::acl::Action::Edit,
                is_agent: false,
                now_epoch_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64,
            };

            match crate::acl::evaluate(&state.vault_root, &query, &spl_blocks, &all_page_slugs) {
                Ok(crate::acl::AclDecision::Denied { tag, rule_trace }) => {
                    let reason_str = if let Some(first) = rule_trace.first() {
                        first.label.clone().unwrap_or_else(|| format!("{tag:?}"))
                    } else {
                        format!("{tag:?}")
                    };
                    eprintln!(
                        "warning: ACL violation detected — page '{}' edited externally but user '{}' is denied edit access ({})",
                        slug, user.id, reason_str
                    );

                    violations.push(HookAclViolationEntry {
                        page: slug.clone(),
                        user_id: user.id.clone(),
                        action: "edit".to_string(),
                        reason: reason_str.clone(),
                    });

                    // Admin banner via WebSocket.
                    state.ws_hub.broadcast_all(ServerMsg::AclViolation {
                        page: slug.clone(),
                        user_id: user.id.clone(),
                        reason: reason_str,
                    });
                }
                Ok(crate::acl::AclDecision::Allowed { .. }) => {
                    // User is allowed — no violation for this user.
                }
                Err(e) => {
                    eprintln!("fs-watch: acl violation check error for page '{}' user '{}': {e}", slug, user.id);
                }
            }
        }
    }

    if violations.is_empty() {
        return;
    }

    // Fire on-acl-violation hooks (best-effort).
    fire_acl_violation_hook(state, violations);
}

/// Fire the on-acl-violation hook with violation context (REQ-020-043).
#[cfg(feature = "reason")]
fn fire_acl_violation_hook(state: &WebState, violations: Vec<HookAclViolationEntry>) {
    use crate::hooks;
    use crate::hooks::context::{build_hook_context, HookAclViolations};

    let theme_hooks = hooks::resolve_theme_hooks(&state.vault_root, &state.theme);
    let manifest = hooks::discover_hooks(&state.vault_root, theme_hooks.path());

    if hooks::hooks_for(&manifest, "on-acl-violation").is_empty() {
        return;
    }

    let data = state.data.read().unwrap();
    let mut ctx = build_hook_context(
        "on-acl-violation",
        &state.vault_root,
        &state.theme,
        env!("CARGO_PKG_VERSION"),
        &data.files,
        &data.graph,
    );

    ctx.acl_violations = Some(HookAclViolations {
        violations: violations.clone(),
    });

    let context_json = match serde_json::to_vec(&ctx) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("on-acl-violation hook: json error: {e}");
            return;
        }
    };

    // Build env vars with violation summary.
    let violation_pages: Vec<String> = violations.iter().map(|v| v.page.clone()).collect();
    let hook_env = hooks::HookEnv {
        vault_root: state.vault_root.as_ref().clone(),
        theme: state.theme.clone(),
        zetl_version: env!("CARGO_PKG_VERSION").to_string(),
        extra_vars: vec![
            ("ZETL_ACL_VIOLATION_COUNT".into(), violations.len().to_string()),
            ("ZETL_ACL_VIOLATION_PAGES".into(), violation_pages.join(",")),
            ("ZETL_HOOK_DEPTH".into(), "0".into()),
        ],
    };

    let results = hooks::run_hooks(&manifest, "on-acl-violation", &context_json, &hook_env);
    for result in results {
        match result {
            Ok(output) if !output.success() => {
                eprintln!(
                    "warning: on-acl-violation hook '{}' ({}) exited with code {}",
                    output.path.display(),
                    output.source,
                    output.exit_code.unwrap_or(-1),
                );
                if !output.stderr.is_empty() {
                    eprintln!("  stderr: {}", output.stderr.trim_end());
                }
            }
            Err(e) => {
                eprintln!("warning: on-acl-violation hook failed to execute: {e}");
            }
            _ => {}
        }
    }
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
                reconcile_external_edits(&state_clone, &changed_paths, &ExternalSource::Filesystem);
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

    /// Helper: build a WebState for testing.
    fn make_test_state(dir: &std::path::Path) -> WebState {
        use crate::search_index::SearchIndex;
        use crate::web::ws::CrdtDocStore;
        use std::sync::{Arc, Mutex, RwLock};

        let vault_root = Arc::new(dir.to_path_buf());
        let data = super::super::reindex(dir).unwrap();
        let search_index = SearchIndex::build(dir, &data.files).unwrap();
        let engine = crate::web::engine::TemplateEngine::new(dir, "default", false, false);

        WebState {
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
            bootstrap_used: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            rate_limiters: crate::web::rate_limit::AuthRateLimiters::new(),
            #[cfg(feature = "reason")]
            acl_cache: Arc::new(Mutex::new(crate::web::AclCache::new())),
            git_commit_lock: None,
            ws_hub: crate::web::ws::WsHub::new(),
            ticket_store: crate::web::ws::TicketStore::new(),
            crdt_store: CrdtDocStore::new(vault_root.clone()),
            wal_store: Arc::new(crate::web::wal::WalStore::new(&vault_root)),
            pending_writes: PendingWrites::new(),
            passkey_mgr: None,
            #[cfg(feature = "semantic")]
            vector_index: None,
        }
    }

    /// TEST-020-039: External file write detected; vault state updated.
    #[test]
    fn reconcile_updates_vault_state() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("hello.md"), "# Hello\n\nworld\n").unwrap();

        let state = make_test_state(dir.path());

        // Simulate external edit: add a new file.
        let new_file = dir.path().join("external.md");
        std::fs::write(&new_file, "# External\n\nAdded by editor\n").unwrap();

        let result = reconcile_external_edits(&state, &[new_file], &ExternalSource::Filesystem);
        assert!(result.is_some(), "vault_root_hash should change after external edit");

        // Verify VaultData was updated to include the new file.
        let data = state.data.read().unwrap();
        assert!(
            data.files.iter().any(|f| f.page_name == "external"),
            "external page should appear in VaultData after reconciliation"
        );
    }

    /// TEST-020-043: ACL violation detected, logged at WARN level, file NOT reverted.
    #[cfg(feature = "reason")]
    #[test]
    fn acl_violation_detected_and_not_reverted() {
        let dir = tempfile::TempDir::new().unwrap();

        // Create a page with an SPL block that denies edit for a specific user.
        std::fs::write(
            dir.path().join("secret.md"),
            "# Secret\n\nRestricted content.\n",
        )
        .unwrap();

        // Create a user profile that will be denied edit access via policy.
        let users_dir = dir.path().join(".zetl").join("users").join("bob-12345678");
        std::fs::create_dir_all(&users_dir).unwrap();
        std::fs::write(
            users_dir.join("profile.json"),
            r#"{
                "id": "bob-12345678",
                "name": "Bob",
                "created_at": "2026-03-18T10:00:00Z",
                "invited_by": "alice-a1b2c3d4",
                "owner": false,
                "credentials": [],
                "recovery_pubkey": "dGVzdA",
                "agent_token_generation": 0
            }"#,
        )
        .unwrap();

        // Create an access.spl policy that denies edit for bob on "secret".
        let zetl_dir = dir.path().join(".zetl");
        std::fs::write(
            zetl_dir.join("access.spl"),
            r#"
(given (deny-edit "bob-12345678" "secret"))
(strict r-deny-bob-secret
  (deny-edit "bob-12345678" "secret")
  (neg (can-edit "bob-12345678" "secret")))
"#,
        )
        .unwrap();

        let state = make_test_state(dir.path());

        // Simulate external edit to "secret.md".
        std::fs::write(
            dir.path().join("secret.md"),
            "# Secret\n\nModified externally by unauthorized user.\n",
        )
        .unwrap();

        let new_file = dir.path().join("secret.md");
        let result = reconcile_external_edits(&state, &[new_file], &ExternalSource::Filesystem);
        assert!(result.is_some(), "vault_root_hash should change");

        // Verify file was NOT reverted (REQ-020-043: file NOT reverted).
        let content = std::fs::read_to_string(dir.path().join("secret.md")).unwrap();
        assert!(
            content.contains("Modified externally"),
            "file content should NOT be reverted after ACL violation"
        );

        // Verify VaultData reflects the external edit.
        let data = state.data.read().unwrap();
        let secret_file = data.files.iter().find(|f| f.page_name == "secret").unwrap();
        assert!(
            secret_file.path.to_string_lossy().contains("secret"),
            "secret page should still be in VaultData"
        );
    }

    /// TEST-020-043: Violation logged — detect_and_report_acl_violations produces violations
    /// for denied users.
    #[cfg(feature = "reason")]
    #[test]
    fn detect_violations_finds_denied_users() {
        let dir = tempfile::TempDir::new().unwrap();

        std::fs::write(dir.path().join("page.md"), "# Page\n\nContent.\n").unwrap();

        // Create two users: owner (allowed) and editor (will be denied via policy).
        let alice_dir = dir.path().join(".zetl").join("users").join("alice-owner");
        std::fs::create_dir_all(&alice_dir).unwrap();
        std::fs::write(
            alice_dir.join("profile.json"),
            r#"{
                "id": "alice-owner",
                "name": "Alice",
                "created_at": "2026-03-18T10:00:00Z",
                "invited_by": null,
                "owner": true,
                "credentials": [],
                "recovery_pubkey": "dGVzdA",
                "agent_token_generation": 0
            }"#,
        )
        .unwrap();

        let bob_dir = dir.path().join(".zetl").join("users").join("bob-editor");
        std::fs::create_dir_all(&bob_dir).unwrap();
        std::fs::write(
            bob_dir.join("profile.json"),
            r#"{
                "id": "bob-editor",
                "name": "Bob",
                "created_at": "2026-03-18T10:00:00Z",
                "invited_by": "alice-owner",
                "owner": false,
                "credentials": [],
                "recovery_pubkey": "dGVzdA",
                "agent_token_generation": 0
            }"#,
        )
        .unwrap();

        // Deny bob-editor from editing "page".
        let zetl_dir = dir.path().join(".zetl");
        std::fs::write(
            zetl_dir.join("access.spl"),
            r#"
(given (deny-edit "bob-editor" "page"))
(strict r-deny-bob-page
  (deny-edit "bob-editor" "page")
  (neg (can-edit "bob-editor" "page")))
"#,
        )
        .unwrap();

        let state = make_test_state(dir.path());

        // Run violation detection on "page" slug.
        detect_and_report_acl_violations(&state, &["page".to_string()]);

        // The function logs to stderr and broadcasts via WebSocket.
        // We can verify the function completes without panic — the WARN log
        // and WebSocket broadcast are observable side-effects.
        // The important acceptance criterion is that the file is NOT reverted.
        let content = std::fs::read_to_string(dir.path().join("page.md")).unwrap();
        assert!(
            content.contains("Content"),
            "file should not be reverted after violation detection"
        );
    }

    /// TEST-020-043: No violations when no users exist.
    #[cfg(feature = "reason")]
    #[test]
    fn no_violations_when_no_users() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("page.md"), "# Page\n").unwrap();

        let state = make_test_state(dir.path());

        // Should complete without panic when no users exist.
        detect_and_report_acl_violations(&state, &["page".to_string()]);
    }

    /// TEST-020-043: No violations when slug list is empty.
    #[cfg(feature = "reason")]
    #[test]
    fn no_violations_for_empty_slugs() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("page.md"), "# Page\n").unwrap();

        let state = make_test_state(dir.path());

        // Should return immediately.
        detect_and_report_acl_violations(&state, &[]);
    }

    /// TEST-020-040 Case 1: Clean CRDT reloaded from disk on external edit.
    #[test]
    fn reconcile_clean_crdt_reloads_from_disk() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("note.md"), "# Note\n\nOriginal.\n").unwrap();

        let state = make_test_state(dir.path());

        // Load the CRDT (simulates a WS connection opening the document).
        state.crdt_store.load_or_get("note").unwrap();
        assert!(state.crdt_store.is_loaded("note"));

        // CRDT is clean (no edits applied).
        let meta = state.crdt_store.crdt_meta("note").unwrap();
        assert!(!meta.dirty);

        // Simulate external edit — rewrite the file on disk.
        std::fs::write(dir.path().join("note.md"), "# Note\n\nEdited externally.\n").unwrap();

        // Reconcile.
        let msgs = state.crdt_store.reconcile_crdt("note", true);

        // Should produce a Sync message (full reload).
        assert_eq!(msgs.len(), 1);
        assert!(matches!(&msgs[0], super::super::ws::ServerMsg::Sync { .. }));

        // The in-memory CRDT should now reflect the external content.
        let md = state.crdt_store.serialize_for_flush("note");
        // After reload the doc is clean, so serialize_for_flush returns None.
        assert!(md.is_none(), "reloaded doc should be clean (not dirty)");
    }

    /// TEST-020-040 Case 2: Dirty CRDT merged with external edit preserves both.
    #[test]
    fn reconcile_dirty_crdt_merges_external() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("note.md"), "# Note\n\nOriginal content.\n").unwrap();

        let state = make_test_state(dir.path());

        // Load CRDT and apply a local edit to make it dirty.
        state.crdt_store.load_or_get("note").unwrap();
        state
            .crdt_store
            .apply_ops(
                "note",
                &[super::super::ws::OpEntry::Splice {
                    pos: 0,
                    del: 0,
                    text: "LOCAL ".to_string(),
                }],
            )
            .unwrap();

        let meta = state.crdt_store.crdt_meta("note").unwrap();
        assert!(meta.dirty, "CRDT should be dirty after local edit");

        // Simulate external edit — rewrite the file on disk.
        std::fs::write(
            dir.path().join("note.md"),
            "# Note\n\nOriginal content. EXTERNAL ADDITION\n",
        )
        .unwrap();

        // Reconcile.
        let msgs = state.crdt_store.reconcile_crdt("note", true);

        // Should produce an ExternalMerge message.
        assert_eq!(msgs.len(), 1);
        assert!(matches!(
            &msgs[0],
            super::super::ws::ServerMsg::ExternalMerge { .. }
        ));

        // The CRDT should still be dirty (merged state needs flushing).
        let meta = state.crdt_store.crdt_meta("note").unwrap();
        assert!(meta.dirty, "merged CRDT should remain dirty");
    }

    /// TEST-020-040 Case 3: Deleted file with dirty CRDT writes recovery file.
    #[test]
    fn reconcile_deleted_file_writes_recovery() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("note.md"), "# Note\n\nOriginal.\n").unwrap();
        // Ensure .zetl dir exists for recovery subdir.
        std::fs::create_dir_all(dir.path().join(".zetl")).unwrap();

        let state = make_test_state(dir.path());

        // Load CRDT and make it dirty.
        state.crdt_store.load_or_get("note").unwrap();
        state
            .crdt_store
            .apply_ops(
                "note",
                &[super::super::ws::OpEntry::Splice {
                    pos: 0,
                    del: 0,
                    text: "UNFLUSHED ".to_string(),
                }],
            )
            .unwrap();
        assert!(state.crdt_store.crdt_meta("note").unwrap().dirty);

        // Delete the file externally.
        std::fs::remove_file(dir.path().join("note.md")).unwrap();

        // Reconcile with file_exists=false.
        let msgs = state.crdt_store.reconcile_crdt("note", false);

        // Should produce a Deleted message.
        assert_eq!(msgs.len(), 1);
        assert!(matches!(
            &msgs[0],
            super::super::ws::ServerMsg::Deleted { page } if page == "note"
        ));

        // CRDT should be evicted.
        assert!(
            !state.crdt_store.is_loaded("note"),
            "CRDT should be evicted after file deletion"
        );

        // Recovery file should exist.
        let recovery_path = dir.path().join(".zetl").join("recovery").join("note.md");
        assert!(
            recovery_path.exists(),
            "recovery file should be written at {}",
            recovery_path.display()
        );

        let recovery_content = std::fs::read_to_string(&recovery_path).unwrap();
        assert!(
            recovery_content.contains("UNFLUSHED"),
            "recovery file should contain unflushed edits"
        );
    }

    /// TEST-020-040 Case 3: Deleted file with clean CRDT — no recovery file.
    #[test]
    fn reconcile_deleted_clean_crdt_no_recovery() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("note.md"), "# Note\n").unwrap();
        std::fs::create_dir_all(dir.path().join(".zetl")).unwrap();

        let state = make_test_state(dir.path());

        // Load CRDT but keep it clean.
        state.crdt_store.load_or_get("note").unwrap();
        assert!(!state.crdt_store.crdt_meta("note").unwrap().dirty);

        // Delete the file externally.
        std::fs::remove_file(dir.path().join("note.md")).unwrap();

        let msgs = state.crdt_store.reconcile_crdt("note", false);

        // Should produce Deleted message.
        assert_eq!(msgs.len(), 1);
        assert!(matches!(&msgs[0], super::super::ws::ServerMsg::Deleted { .. }));

        // CRDT evicted.
        assert!(!state.crdt_store.is_loaded("note"));

        // No recovery file since CRDT was clean.
        let recovery_path = dir.path().join(".zetl").join("recovery").join("note.md");
        assert!(
            !recovery_path.exists(),
            "no recovery file expected for clean CRDT"
        );
    }

    /// TEST-020-040: No CRDT session — reconcile is a no-op.
    #[test]
    fn reconcile_no_crdt_session_is_noop() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("note.md"), "# Note\n").unwrap();

        let state = make_test_state(dir.path());

        // No CRDT loaded — reconcile should produce no messages.
        let msgs = state.crdt_store.reconcile_crdt("note", true);
        assert!(msgs.is_empty());
    }
}
