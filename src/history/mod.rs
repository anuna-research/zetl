//! Temporal graph navigation via jj-lib (SPEC-017).
//!
//! This module is only compiled when the `history` Cargo feature is enabled:
//!
//! ```text
//! cargo build --features history
//! ```
//!
//! When the feature is absent the module does not exist and no jj-lib code is
//! compiled, keeping the default binary identical in size to the pre-SPEC-017
//! build.
//!
//! # Submodules
//!
//! - `jj_backend`  — JjBackend wrapper around jj-lib (task-jj-backend)
//! - `cache`       — HistoricalIndexCache keyed by vault_root_hash (task-historical-index-cache)
//! - `core`        — Pure temporal functions: time-expr parser, delta, timeline
//!                   (task-time-expression-parser, task-history-cli)

pub mod cache;
pub mod core;
pub mod jj_backend;

use std::path::Path;

use jj_backend::VcsBackend as _;

/// Open the jj workspace for temporal queries (REQ-084).
///
/// Unlike [`jj_backend::JjBackend::open_or_init_at_vault_root`], this function
/// **does not initialise** a new workspace. If `.zetl/jj/` is absent it returns
/// an error with code `NO_HISTORY` so the caller can surface a structured
/// diagnostic: "history will be available after the next `zetl index`".
///
/// Use [`auto_snapshot`] / `cmd_index` when you want the init-or-open behaviour.
pub fn open_history(vault_root: &Path) -> anyhow::Result<jj_backend::JjBackend> {
    let jj_dir = vault_root.join(".zetl").join("jj");
    if !jj_dir.exists() {
        anyhow::bail!(
            "NO_HISTORY: No history available. \
             Run `zetl index` to create the first snapshot."
        );
    }
    jj_backend::JjBackend::open_or_init_at_vault_root(vault_root)
}

/// Build the `vault.history` template context object (REQ-085, CON-026, ADR-049).
///
/// Opens the jj workspace, loads the snapshot list, and calls
/// [`core::build_vault_history_context`] to produce a populated
/// [`core::VaultHistoryContext`].
///
/// Returns `None` when history is unavailable (no workspace, no snapshots,
/// or any error encountered). Errors are swallowed so templates always
/// receive either a populated object or `null`.
pub fn build_template_history_context(vault_root: &Path) -> Option<core::VaultHistoryContext> {
    use chrono::Local;

    let backend = open_history(vault_root).ok()?;
    let snapshots = backend.list_changes(10_000).ok()?;
    if snapshots.is_empty() {
        return None;
    }
    let now = Local::now().fixed_offset();
    core::build_vault_history_context(&snapshots, vault_root, now)
        .ok()
        .flatten()
}

/// Build the `page.history` template context object (REQ-086, CON-026, ADR-049).
///
/// Opens the jj workspace, loads the snapshot list and cached indexes, and calls
/// [`core::build_page_history_context`] to produce a populated
/// [`core::PageHistoryContext`].
///
/// Returns `None` when history is unavailable or the page has no recorded history.
/// Errors are swallowed so templates always receive either a populated object or `null`.
pub fn build_template_page_history_context(
    page_name: &str,
    vault_root: &Path,
) -> Option<core::PageHistoryContext> {
    use chrono::Local;
    use std::collections::HashMap;
    use std::path::PathBuf;

    let backend = open_history(vault_root).ok()?;
    let snapshots = backend.list_changes(10_000).ok()?;
    if snapshots.is_empty() {
        return None;
    }

    let index_cache = cache::HistoricalIndexCache::with_default_capacity();
    let files_per_snapshot: Vec<Option<Vec<crate::types::ParsedFile>>> = snapshots
        .iter()
        .map(|snap| {
            let hash = core::extract_vault_root_hash_from_description(&snap.description)?;
            let file_map: HashMap<PathBuf, crate::types::ParsedFile> =
                index_cache.load(vault_root, &hash).ok().flatten()?;
            Some(file_map.into_values().collect())
        })
        .collect();

    let now = Local::now().fixed_offset();
    core::build_page_history_context(page_name, &snapshots, &files_per_snapshot, now)
}

/// Build a map from backlink source page name (lowercase) → earliest snapshot
/// timestamp where it linked to `target_page` (REQ-089, CON-026).
///
/// Loads the jj snapshot list and per-snapshot cached indexes once, then
/// calls [`core::resolve_backlink_since`] for each source in `backlink_sources`.
///
/// Returns an empty map when history is unavailable, `backlink_sources` is
/// empty, or no cached indexes exist. Errors are swallowed so callers always
/// receive a usable (possibly empty) map.
pub fn build_backlink_since_map(
    target_page: &str,
    backlink_sources: &[String],
    vault_root: &Path,
) -> std::collections::HashMap<String, String> {
    use std::collections::HashMap;
    use std::path::PathBuf;

    if backlink_sources.is_empty() {
        return HashMap::new();
    }

    let backend = match open_history(vault_root) {
        Ok(b) => b,
        Err(_) => return HashMap::new(),
    };

    let snapshots = match backend.list_changes(10_000) {
        Ok(s) if !s.is_empty() => s,
        _ => return HashMap::new(),
    };

    let index_cache = cache::HistoricalIndexCache::with_default_capacity();
    let files_per_snapshot: Vec<Option<Vec<crate::types::ParsedFile>>> = snapshots
        .iter()
        .map(|snap| {
            let hash = core::extract_vault_root_hash_from_description(&snap.description)?;
            let file_map: HashMap<PathBuf, crate::types::ParsedFile> =
                index_cache.load(vault_root, &hash).ok().flatten()?;
            Some(file_map.into_values().collect())
        })
        .collect();

    let mut result = HashMap::new();
    for source in backlink_sources {
        if let Some(ts) =
            core::resolve_backlink_since(source, target_page, &snapshots, &files_per_snapshot)
        {
            result.insert(source.to_lowercase(), ts);
        }
    }
    result
}

/// Build the `history` object for hook context stdin (REQ-090, CON-016-001).
///
/// Returns a JSON object with:
/// - `snapshot_count` — total number of raw snapshots
/// - `oldest` — RFC 3339 timestamp of the oldest snapshot (null if none)
/// - `newest` — RFC 3339 timestamp of the newest snapshot (null if none)
/// - `vault_root_hash` — hash embedded in the most recent snapshot (null if absent)
/// - `previous_vault_root_hash` — hash from the second-most-recent distinct state (null if absent)
/// - `delta` — [`core::GraphDelta`] between the previous and current state (null if unavailable)
///
/// Returns [`serde_json::Value::Null`] when history is unavailable (no workspace,
/// no snapshots, or any error). Errors are swallowed so hooks always receive either
/// a populated object or `null`.
pub fn build_hook_history_context(vault_root: &Path) -> serde_json::Value {
    use cache::HistoricalIndexCache;
    use core::{compute_graph_delta, extract_vault_root_hash_from_description};
    use jj_backend::VcsBackend as _;
    use std::collections::HashMap;
    use std::path::PathBuf;

    let backend = match open_history(vault_root) {
        Ok(b) => b,
        Err(_) => return serde_json::Value::Null,
    };

    let snapshots = match backend.list_changes(10_000) {
        Ok(s) if !s.is_empty() => s,
        _ => return serde_json::Value::Null,
    };

    let snapshot_count = snapshots.len();
    let oldest: serde_json::Value = snapshots
        .last()
        .map(|s| serde_json::Value::String(s.timestamp.to_rfc3339()))
        .unwrap_or(serde_json::Value::Null);
    let newest: serde_json::Value = snapshots
        .first()
        .map(|s| serde_json::Value::String(s.timestamp.to_rfc3339()))
        .unwrap_or(serde_json::Value::Null);

    // Walk snapshots newest-first to find the two most recent distinct
    // vault_root_hash values (skipping duplicates).
    let mut distinct: Vec<(String, usize)> = Vec::new(); // (hash, index in snapshots)
    for (i, snap) in snapshots.iter().enumerate() {
        if let Some(hash) = extract_vault_root_hash_from_description(&snap.description) {
            // Include only when this hash differs from the most recently added one.
            let is_new = distinct.last().map(|(h, _)| h != &hash).unwrap_or(true);
            if is_new {
                distinct.push((hash, i));
            }
        }
        if distinct.len() >= 2 {
            break;
        }
    }

    let vault_root_hash: serde_json::Value = distinct
        .first()
        .map(|(h, _)| serde_json::Value::String(h.clone()))
        .unwrap_or(serde_json::Value::Null);
    let previous_vault_root_hash: serde_json::Value = distinct
        .get(1)
        .map(|(h, _)| serde_json::Value::String(h.clone()))
        .unwrap_or(serde_json::Value::Null);

    // Compute delta between the two most recent distinct states.
    let delta: serde_json::Value = if distinct.len() >= 2 {
        let cache = HistoricalIndexCache::with_default_capacity();

        let load_files = |hash: &str| -> Option<Vec<crate::types::ParsedFile>> {
            let file_map: HashMap<PathBuf, crate::types::ParsedFile> =
                cache.load(vault_root, hash).ok().flatten()?;
            Some(file_map.into_values().collect())
        };

        let curr_files = load_files(&distinct[0].0);
        let prev_files = load_files(&distinct[1].0);

        match (prev_files, curr_files) {
            (Some(prev), Some(curr)) => {
                let d = compute_graph_delta(&prev, &curr);
                serde_json::to_value(d).unwrap_or(serde_json::Value::Null)
            }
            _ => serde_json::Value::Null,
        }
    } else {
        serde_json::Value::Null
    };

    serde_json::json!({
        "snapshot_count": snapshot_count,
        "oldest": oldest,
        "newest": newest,
        "vault_root_hash": vault_root_hash,
        "previous_vault_root_hash": previous_vault_root_hash,
        "delta": delta,
    })
}

/// Build the `history-index.json` payload (REQ-088, CON-026, ADR-049, ADR-050).
///
/// Opens the jj workspace once, loads all snapshots and per-snapshot cached
/// indexes, builds a [`core::VaultHistoryContext`] and per-page
/// [`core::PageHistoryContext`] for each name in `page_names`, then calls
/// [`core::serialize_history_index`] to produce the JSON.
///
/// Returns `None` when history is unavailable (no workspace, no snapshots, or
/// any error). Errors are swallowed so the build pipeline skips the export
/// gracefully.
pub fn build_history_index_json(vault_root: &Path, page_names: &[&str]) -> Option<String> {
    use chrono::Local;
    use std::collections::HashMap;
    use std::path::PathBuf;

    let backend = open_history(vault_root).ok()?;
    let snapshots = backend.list_changes(10_000).ok()?;
    if snapshots.is_empty() {
        return None;
    }

    let now = Local::now().fixed_offset();

    let vault_ctx = core::build_vault_history_context(&snapshots, vault_root, now)
        .ok()
        .flatten()?;

    // Load files per snapshot once, shared across all page context builds.
    let index_cache = cache::HistoricalIndexCache::with_default_capacity();
    let files_per_snapshot: Vec<Option<Vec<crate::types::ParsedFile>>> = snapshots
        .iter()
        .map(|snap| {
            let hash = core::extract_vault_root_hash_from_description(&snap.description)?;
            let file_map: HashMap<PathBuf, crate::types::ParsedFile> =
                index_cache.load(vault_root, &hash).ok().flatten()?;
            Some(file_map.into_values().collect())
        })
        .collect();

    let page_contexts: Vec<(&str, core::PageHistoryContext)> = page_names
        .iter()
        .filter_map(|&name| {
            let ctx = core::build_page_history_context(name, &snapshots, &files_per_snapshot, now)?;
            Some((name, ctx))
        })
        .collect();

    let page_refs: Vec<(&str, &core::PageHistoryContext)> =
        page_contexts.iter().map(|(n, c)| (*n, c)).collect();

    let json_value = core::serialize_history_index(&vault_ctx, &page_refs);
    serde_json::to_string(&json_value).ok()
}

/// Create a jj snapshot after index completion (REQ-076, ADR-048).
///
/// - Opens or initialises the jj workspace at `.zetl/jj/`.
/// - Embeds `vault_root_hash` in the commit description for traceability.
/// - Skips the snapshot when the most recent commit already records the same
///   `vault_root_hash` (fast content-hash deduplication).
/// - The jj backend also deduplicates independently by tree hash, so this
///   function is safe to call even when the caller skips the hash check.
///
/// Returns `Ok(Some(change_id))` when a new snapshot was committed,
/// `Ok(None)` when deduplicated (vault state unchanged), or an error if the
/// jj workspace could not be opened or initialised.
pub fn auto_snapshot(
    vault_root: &Path,
    vault_root_hash: Option<&str>,
) -> anyhow::Result<Option<String>> {
    let mut backend = jj_backend::JjBackend::open_or_init_at_vault_root(vault_root)?;

    let description = match vault_root_hash {
        Some(hash) => format!("zetl-snapshot vault_root_hash={hash}"),
        None => "zetl-snapshot".to_owned(),
    };

    // Fast deduplication: skip if the most recent commit already carries this
    // vault_root_hash (REQ-076, ADR-048).
    if let Some(hash) = vault_root_hash {
        let already_current = backend
            .list_changes(1)
            .ok()
            .and_then(|changes| changes.into_iter().next())
            .map(|c| c.description.contains(hash))
            .unwrap_or(false);

        if already_current {
            return Ok(None);
        }
    }

    backend.snapshot(&description)
}
