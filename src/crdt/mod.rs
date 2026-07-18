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
pub mod loro_store;
pub mod marks;
pub mod marks_doc;

pub use backend::CrdtBackend;

/// The CRDT backend used by the WebSocket editing layer.
///
/// Aliases [`diamond::DiamondCrdtDocument`] — the only backend after
/// IMPL-029 Phase 7 flipped the default and removed the legacy engine.
pub use diamond::DiamondCrdtDocument as WsCrdtBackend;
