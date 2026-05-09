//! SPEC-039 pure core. Deterministic over inputs; no I/O, no clock
//! reads, no `tokio` / `axum` / `git2` / `std::fs` writes.
//!
//! Modules:
//! - [`verify`] — REQ-3903 source-link verification
//! - [`extract`] — outbound external-link extraction
//! - [`discover`] — REQ-3908 endpoint discovery
//! - [`diff`] — REQ-3906 sender idempotency diff
//! - [`moderate`] — REQ-3905 moderation gate

pub mod diff;
pub mod discover;
pub mod extract;
pub mod moderate;
pub mod verify;
