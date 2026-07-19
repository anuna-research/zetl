//! Peritext-flavoured CRDT engine for collaborative editing (REQ-020-024 …
//! REQ-020-027).
//!
//! The editing engine is [`loro_backend::LoroCrdtDocument`] — a [[Loro]]
//! rich-text document (text + expand-aware style marks). Inline marks map to
//! markdown syntax via the shared helpers in [`marks`]; block-level structure
//! is preserved as literal text (atomic tokens from [`blocks::BlockToken`]).
//! [`WsCrdtBackend`] is the public alias used by the WebSocket editing layer
//! and production storage paths.
//!
//! The realtime P2P sync substrate (SPEC-047) lives alongside: per-note store
//! ([`loro_store`]), namespace [`manifest`], [`guarded_import`], anti-entropy
//! [`reconcile`], Merkle [`witness`], and Markdown export ([`vault_fs`]).

pub mod blocks;
pub mod export_state;
pub mod guarded_import;
pub mod loro_backend;
pub mod loro_store;
pub mod manifest;
pub mod marks;
pub mod reconcile;
pub mod vault_fs;
pub mod witness;

/// The CRDT backend used by the WebSocket editing layer.
///
/// Aliases [`loro_backend::LoroCrdtDocument`] — SPEC-047 ADR-470/§9 makes
/// [[Loro]] the one editing engine. The former engine-agnostic
/// `CrdtBackend` trait and the diamond-types engine (`diamond`,
/// `marks_doc`, `backend`) are removed: one engine, no speculative
/// indirection (PROTO-001 Discipline Rules).
pub use loro_backend::LoroCrdtDocument as WsCrdtBackend;

/// Make a directory's entries durable after an atomic tmp+rename: syncing the
/// temporary file alone does not persist the *rename* — a power loss right
/// after can still lose the acknowledged snapshot. Every canonical-store
/// persist path (note snapshots, manifest, rotation outbox) calls this after
/// its rename. No-op on non-Unix (directories cannot be opened for sync).
pub(crate) fn fsync_dir(dir: &std::path::Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use anyhow::Context as _;
        std::fs::File::open(dir)
            .and_then(|d| d.sync_all())
            .with_context(|| format!("fsync dir {}", dir.display()))?;
    }
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}
