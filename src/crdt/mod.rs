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
