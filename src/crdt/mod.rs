//! Peritext-flavoured CRDT engine for collaborative editing (REQ-020-024 …
//! REQ-020-027).
//!
//! The backend is [`diamond::DiamondCrdtDocument`] — a `diamond-types` OpLog
//! for text paired with a sibling marks oplog (see
//! [`marks_doc::MarksDoc`]) that stores Peritext span ops. Inline marks map
//! to markdown syntax via the shared helpers in [`marks`]; block-level
//! structure is preserved as atomic tokens from [`blocks::BlockToken`].
//!
//! Every caller reaches the backend through the [`CrdtBackend`] trait —
//! [`WsCrdtBackend`] is the public type alias used by the WebSocket
//! editing layer and production storage paths.

pub mod backend;
pub mod blocks;
pub mod diamond;
pub mod guarded_import;
pub mod loro_backend;
pub mod loro_store;
pub mod manifest;
pub mod marks;
pub mod reconcile;
pub mod vault_fs;
pub mod witness;
pub mod marks_doc;

pub use backend::CrdtBackend;

/// The CRDT backend used by the WebSocket editing layer.
///
/// Aliases [`loro_backend::LoroCrdtDocument`] — SPEC-047 ADR-470/§9 makes
/// [[Loro]] the one editing engine, replacing diamond-types. The
/// `diamond` module is retained for now (its own unit tests still run) and
/// removed in a follow-up cleanup slice once no path references it.
pub use loro_backend::LoroCrdtDocument as WsCrdtBackend;
