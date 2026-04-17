//! Engine-agnostic CRDT backend trait.
//!
//! [`crate::crdt::diamond::DiamondCrdtDocument`] is the sole implementation
//! after IMPL-029 Phase 7. Consumers in `src/web/ws.rs` hold a
//! `Box<dyn CrdtBackend>` so alternative engines remain swappable without
//! touching call sites.

use std::any::Any;

use anyhow::Result;

use crate::crdt::marks::{Mark, MarkType};

/// The public surface required of a CRDT document backend.
///
/// Returns project-owned types (`Mark`, `MarkType`) so no engine-specific
/// borrowed types leak into the trait.
pub trait CrdtBackend: Send + Sync {
    /// Create an empty CRDT document.
    fn new() -> Result<Self>
    where
        Self: Sized;

    /// Parse markdown text into a new CRDT document.
    fn from_markdown(markdown: &str) -> Result<Self>
    where
        Self: Sized;

    /// Reconstruct a CRDT document from saved bytes.
    fn load(data: &[u8]) -> Result<Self>
    where
        Self: Sized;

    /// Raw text content of the document.
    fn text(&self) -> Result<String>;

    /// All marks on the document's text object.
    fn marks(&self) -> Result<Vec<Mark>>;

    /// Splice text at `pos`: delete `del` chars, then insert `text`.
    fn splice_text(&mut self, pos: usize, del: isize, text: &str) -> Result<()>;

    /// Apply a mark to `[start, end)`.
    fn mark(&mut self, mark_type: &MarkType, start: usize, end: usize) -> Result<()>;

    /// Remove a mark from `[start, end)`.
    fn unmark(&mut self, mark_type: &MarkType, start: usize, end: usize) -> Result<()>;

    /// Serialize CRDT state to canonical markdown.
    fn to_markdown(&self) -> Result<String>;

    /// Save CRDT state to bytes.
    fn save(&mut self) -> Vec<u8>;

    /// Fork the document for concurrent editing. Returns a new, independent
    /// backend with the same content.
    fn fork(&mut self) -> Box<dyn CrdtBackend>;

    /// Merge another backend's changes into this one.
    ///
    /// Backends may only merge with instances of the same concrete type —
    /// implementations downcast via [`CrdtBackend::as_any_mut`] and return
    /// an error on type mismatch.
    fn merge(&mut self, other: &mut dyn CrdtBackend) -> Result<()>;

    /// Downcast hook used by [`CrdtBackend::merge`] to recover the
    /// concrete backend type from a trait object.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}
