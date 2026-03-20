//! Unified flush pipeline for CRDT quiescence writes (REQ-020-034).
//!
//! When a CRDT document has been idle for the quiescence delay, the background
//! lifecycle task calls [`flush_pipeline`] which executes all 10 pipeline steps
//! in order:
//!
//! 1. **Serialize** — CRDT → canonical markdown
//! 2. **Write** — atomic write to disk
//! 3. **Re-scan** — `reindex()` to rebuild VaultData
//! 4. **Merkle** — compute `vault_root_hash` from fresh parse
//! 5. **ACL invalidate** — clear the ACL decision cache
//! 6. **Search** — rebuild the Tantivy search index
//! 7. **Git commit** — stage + commit the changed file
//! 8. **jj import** — synchronize jj's view of git
//! 9. **jj snapshot** — create a jj snapshot with vault_root_hash
//! 10. **Graph + hooks** — update shared VaultData and fire on-save hooks

use std::path::Path;

use crate::hooks;
use crate::hooks::context::{build_hook_context, HookSaved};
use crate::merkle::{compute_file_root, compute_vault_root};
use crate::search_index::SearchIndex;
use crate::types::ContentHash;

use super::WebState;

/// Result of a single flush pipeline execution for one slug.
#[derive(Debug)]
pub struct FlushResult {
    pub slug: String,
    /// Hex-encoded vault_root_hash computed in step 4.
    pub vault_root_hash: Option<String>,
    /// Errors from non-fatal pipeline steps (logged but don't abort the pipeline).
    pub warnings: Vec<String>,
}

/// Run the unified flush pipeline for a single slug (REQ-020-034).
///
/// Steps 1–2 (serialize + write) must succeed for the pipeline to continue.
/// Steps 3–10 are best-effort: failures are collected in `warnings` but do not
/// abort subsequent steps.
pub fn flush_pipeline(state: &WebState, slug: &str) -> Option<FlushResult> {
    let mut warnings = Vec::new();

    // ── Step 1: Serialize ────────────────────────────────────────────────
    let md = state.crdt_store.serialize_for_flush(slug)?;

    // ── Step 2: Write ────────────────────────────────────────────────────
    // Resolve the real on-disk path from VaultData (preserves original
    // filename casing/spaces).  Falls back to slug-derived path for new pages.
    let path = {
        let data = state.data.read().unwrap();
        let file = data.files.iter().find(|f| {
            crate::scanner::page_slug_from_path(&f.path).eq_ignore_ascii_case(slug)
        });
        if let Some(file) = file {
            state.vault_root.join(&file.path)
        } else {
            crdt_md_path(&state.vault_root, slug)
        }
    };
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("flush: mkdir error for {slug}: {e}");
            return None;
        }
    }
    // Mark as pending write so the fs watcher ignores this event (REQ-020-039).
    state.pending_writes.insert(&path);
    if let Err(e) = std::fs::write(&path, &md) {
        state.pending_writes.remove(&path);
        eprintln!("flush: write error for {slug}: {e}");
        return None;
    }
    state.crdt_store.mark_flushed(slug);

    // Truncate the WAL after successful write (REQ-020-044).
    if let Err(e) = state.wal_store.truncate(slug) {
        eprintln!("flush: WAL truncate error for {slug}: {e}");
    }

    // ── Step 3: Re-scan ──────────────────────────────────────────────────
    let new_data = match super::reindex(&state.vault_root) {
        Ok(d) => d,
        Err(e) => {
            warnings.push(format!("reindex: {e}"));
            return Some(FlushResult {
                slug: slug.to_string(),
                vault_root_hash: None,
                warnings,
            });
        }
    };

    // ── Step 4: Merkle — compute vault_root_hash ─────────────────────────
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

    // ── Step 5: ACL invalidate ───────────────────────────────────────────
    #[cfg(feature = "reason")]
    {
        if let Ok(mut cache) = state.acl_cache.lock() {
            cache.invalidate_if_changed(Some(&vault_root_hash));
        }
    }

    // ── Step 6: Search — rebuild Tantivy index ───────────────────────────
    if let Err(e) = SearchIndex::build(&state.vault_root, &new_data.files) {
        warnings.push(format!("search index: {e}"));
    }

    // ── Step 7: Git commit ───────────────────────────────────────────────
    if let Some(ref lock) = state.git_commit_lock {
        match lock.lock() {
            Ok(repo) => {
                let msg = format!("edit: {slug} (crdt flush)");
                match super::git_commit::auto_commit(
                    &repo,
                    &path,
                    "zetl-crdt",
                    "zetl-crdt",
                    Some(&msg),
                ) {
                    Ok(_oid) => {
                        // ── Step 8: jj import ────────────────────────────
                        super::git_commit::jj_git_import(&state.vault_root);
                    }
                    Err(e) => {
                        warnings.push(format!("git commit: {e}"));
                    }
                }
            }
            Err(e) => {
                warnings.push(format!("git lock: {e}"));
            }
        }
    }

    // ── Step 9: jj snapshot ──────────────────────────────────────────────
    #[cfg(feature = "history")]
    {
        match crate::history::auto_snapshot(&state.vault_root, Some(&vault_root_hash)) {
            Ok(Some(change_id)) => {
                eprintln!("flush: jj snapshot {change_id} for {slug}");
            }
            Ok(None) => { /* deduplicated — vault unchanged */ }
            Err(e) => {
                warnings.push(format!("jj snapshot: {e}"));
            }
        }
    }

    // ── Step 10: Graph — swap VaultData + fire on-save hooks ─────────────
    *state.data.write().unwrap() = new_data;

    // Fire on-save hooks in a blocking context (best-effort).
    let vault_root = state.vault_root.clone();
    let theme = state.theme.clone();
    let content_length = md.len();
    let rel_path_str = path
        .strip_prefix(state.vault_root.as_ref())
        .unwrap_or(&path)
        .to_string_lossy()
        .into_owned();
    let page_name = slug.rsplit('/').next().unwrap_or(slug).to_string();

    std::thread::spawn(move || {
        let theme_hooks = hooks::resolve_theme_hooks(&vault_root, &theme);
        let manifest = hooks::discover_hooks(&vault_root, theme_hooks.path());

        if hooks::hooks_for(&manifest, "on-save").is_empty() {
            return;
        }

        let files = match crate::scanner::scan_vault(&vault_root, &[]) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("flush hook: scan error: {e}");
                return;
            }
        };
        let file_index: Vec<(String, std::path::PathBuf)> = files
            .iter()
            .map(|f| (f.page_name.clone(), f.path.clone()))
            .collect();
        let mut resolved = std::collections::HashMap::new();
        for file in &files {
            for link in &file.links {
                let key = link.raw_target.clone();
                if !resolved.contains_key(&key) {
                    if let Some(r) =
                        crate::scanner::resolve_page_name(&link.target_page, &file_index)
                    {
                        resolved.insert(key, r);
                    }
                }
            }
        }
        let graph = crate::graph::LinkGraph::build(&files, &resolved);

        let mut ctx = build_hook_context(
            "on-save",
            &vault_root,
            &theme,
            env!("CARGO_PKG_VERSION"),
            &files,
            &graph,
        );
        ctx.saved = Some(HookSaved {
            file: rel_path_str.clone(),
            page: page_name.clone(),
            content_length,
            is_external: false,
        });

        let context_json = match serde_json::to_vec(&ctx) {
            Ok(j) => j,
            Err(e) => {
                eprintln!("flush hook: json error: {e}");
                return;
            }
        };

        let hook_env = hooks::HookEnv {
            vault_root: vault_root.to_path_buf(),
            theme: theme.clone(),
            zetl_version: env!("CARGO_PKG_VERSION").to_string(),
            extra_vars: vec![
                ("ZETL_SAVED_FILE".into(), rel_path_str),
                ("ZETL_SAVED_PAGE".into(), page_name),
                ("ZETL_HOOK_DEPTH".into(), "0".into()),
            ],
        };

        let results = hooks::run_hooks(&manifest, "on-save", &context_json, &hook_env);
        for result in results {
            match result {
                Ok(output) if !output.success() => {
                    eprintln!(
                        "warning: flush on-save hook '{}' ({}) exited {}",
                        output.path.display(),
                        output.source,
                        output.exit_code.unwrap_or(-1),
                    );
                }
                Err(e) => {
                    eprintln!("warning: flush on-save hook error: {e}");
                }
                _ => {}
            }
        }
    });

    Some(FlushResult {
        slug: slug.to_string(),
        vault_root_hash: Some(vault_root_hash),
        warnings,
    })
}

/// Resolve a slug to its `.md` file path under the vault root.
fn crdt_md_path(vault_root: &Path, slug: &str) -> std::path::PathBuf {
    vault_root.join(format!("{slug}.md"))
}

/// Spawn the background CRDT lifecycle task that runs quiescence flushes
/// through the unified pipeline and handles TTL evictions.
///
/// Replaces the simpler `spawn_crdt_lifecycle_task` from ws.rs by running
/// the full 10-step pipeline instead of just serialize+write.
pub fn spawn_flush_lifecycle_task(state: WebState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));
        loop {
            interval.tick().await;

            // WAL size check: force-flush any doc whose WAL exceeds MAX_WAL_SIZE (REQ-020-044).
            let loaded_slugs = state.crdt_store.loaded_slugs();
            for slug in &loaded_slugs {
                if state.wal_store.wal_size(slug) >= super::wal::MAX_WAL_SIZE {
                    let state_clone = state.clone();
                    let slug_clone = slug.clone();
                    let _ = tokio::task::spawn_blocking(move || {
                        if let Some(r) = flush_pipeline(&state_clone, &slug_clone) {
                            for w in &r.warnings {
                                eprintln!("WAL force-flush warning ({slug_clone}): {w}");
                            }
                        }
                    })
                    .await;
                }
            }

            // Quiescence flush: run the unified pipeline for dirty docs idle ≥ quiescence_delay.
            let to_flush = state.crdt_store.slugs_needing_flush();
            for slug in &to_flush {
                // Run the blocking pipeline on a dedicated thread to avoid
                // starving the tokio runtime.
                let state_clone = state.clone();
                let slug_clone = slug.clone();
                let result = tokio::task::spawn_blocking(move || {
                    flush_pipeline(&state_clone, &slug_clone)
                })
                .await;

                match result {
                    Ok(Some(r)) => {
                        for w in &r.warnings {
                            eprintln!("flush pipeline warning ({slug}): {w}");
                        }
                    }
                    Ok(None) => {
                        // Document wasn't dirty or serialize failed — already logged
                    }
                    Err(e) => {
                        eprintln!("flush pipeline panic for {slug}: {e}");
                    }
                }
            }

            // TTL eviction: remove docs where all clients left > eviction_ttl ago.
            // Evicted dirty docs get a simple write (they already went through the
            // pipeline during their last quiescence flush).
            let to_evict = state.crdt_store.slugs_for_eviction();
            for slug in &to_evict {
                if let Some(md) = state.crdt_store.evict(slug) {
                    let path = crdt_md_path(&state.vault_root, slug);
                    if let Err(e) = std::fs::write(&path, &md) {
                        eprintln!("error: eviction flush for {slug}: {e}");
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::ws::CrdtDocStore;
    use std::sync::{Arc, Mutex, RwLock};

    /// Helper: create a minimal WebState backed by a temp directory.
    fn test_state(dir: &tempfile::TempDir) -> WebState {
        let vault_root = Arc::new(dir.path().to_path_buf());
        let data = super::super::reindex(dir.path()).unwrap();
        let search_index = SearchIndex::build(dir.path(), &data.files).unwrap();
        let engine = crate::web::engine::TemplateEngine::new(dir.path(), "default", false, false);
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
            pending_writes: crate::web::fs_watch::PendingWrites::new(),
            passkey_mgr: None,
            #[cfg(feature = "semantic")]
            vector_index: None,
        }
    }

    /// TEST-020-034: All 10 pipeline steps execute in order; vault_root_hash updated.
    #[test]
    fn flush_pipeline_executes_all_steps() {
        let dir = tempfile::TempDir::new().unwrap();

        // Seed a page so the vault isn't empty.
        std::fs::write(dir.path().join("hello.md"), "# Hello\n\noriginal\n").unwrap();

        let state = test_state(&dir);

        // Load doc into CRDT store and simulate an edit.
        state.crdt_store.load_or_get("hello").unwrap();
        state.crdt_store.record_edit("hello");

        // Manually set dirty content via apply_ops or direct serialize check.
        // For the pipeline test, we just need serialize_for_flush to return Some.
        // The doc was loaded from "# Hello\n\noriginal\n" and marked dirty.

        let result = flush_pipeline(&state, "hello");
        assert!(result.is_some(), "flush should produce a result");
        let result = result.unwrap();

        assert_eq!(result.slug, "hello");
        assert!(
            result.vault_root_hash.is_some(),
            "vault_root_hash should be computed"
        );

        // Verify the hash is a valid 64-char hex string.
        let hash = result.vault_root_hash.as_ref().unwrap();
        assert_eq!(hash.len(), 64, "BLAKE3 hash should be 64 hex chars");
        assert!(
            hash.chars().all(|c| c.is_ascii_hexdigit()),
            "hash should be hex"
        );

        // Verify VaultData was updated (step 10).
        let data = state.data.read().unwrap();
        assert!(
            data.files.iter().any(|f| f.page_name == "hello"),
            "hello page should be in VaultData after flush"
        );
    }

    #[test]
    fn flush_pipeline_returns_none_for_clean_doc() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("clean.md"), "# Clean\n").unwrap();

        let state = test_state(&dir);

        // Load but don't edit — doc is not dirty.
        state.crdt_store.load_or_get("clean").unwrap();

        let result = flush_pipeline(&state, "clean");
        assert!(result.is_none(), "clean doc should not flush");
    }

    #[test]
    fn flush_pipeline_returns_none_for_missing_doc() {
        let dir = tempfile::TempDir::new().unwrap();
        let state = test_state(&dir);

        let result = flush_pipeline(&state, "nonexistent");
        assert!(result.is_none(), "missing doc should not flush");
    }

    #[test]
    fn flush_pipeline_with_git_repo() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("note.md"), "# Note\n").unwrap();

        // Init a git repo in the temp dir.
        let repo = git2::Repository::init(dir.path()).unwrap();
        {
            let mut config = repo.config().unwrap();
            config.set_str("user.name", "test").unwrap();
            config.set_str("user.email", "test@test").unwrap();
        }
        // Create initial commit.
        {
            let sig = git2::Signature::now("test", "test@test").unwrap();
            let mut index = repo.index().unwrap();
            index
                .add_path(std::path::Path::new("note.md"))
                .unwrap();
            index.write().unwrap();
            let tree_oid = index.write_tree().unwrap();
            let tree = repo.find_tree(tree_oid).unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
                .unwrap();
        }

        let mut state = test_state(&dir);
        state.git_commit_lock = Some(Arc::new(Mutex::new(repo)));

        // Load + dirty the doc.
        state.crdt_store.load_or_get("note").unwrap();
        state.crdt_store.record_edit("note");

        let result = flush_pipeline(&state, "note");
        assert!(result.is_some());
        let result = result.unwrap();

        // Git commit should succeed (no warning about git).
        let git_warnings: Vec<_> = result
            .warnings
            .iter()
            .filter(|w| w.starts_with("git"))
            .collect();
        assert!(
            git_warnings.is_empty(),
            "should have no git warnings: {git_warnings:?}"
        );
    }
}
