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
pub fn auto_snapshot(vault_root: &Path, vault_root_hash: Option<&str>) -> anyhow::Result<Option<String>> {
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
