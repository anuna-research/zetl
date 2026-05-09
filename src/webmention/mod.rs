//! SPEC-039 Webmention support.
//!
//! Module layout follows SPEC-039 §9 Purity Boundary Map:
//!
//! - **Pure core ([`core`]):** [`core::verify`], [`core::extract`],
//!   [`core::discover`], [`core::diff`], [`core::moderate`]. Deterministic
//!   given inputs; no I/O, no clock reads, no `tokio` / `axum` / `git2` /
//!   `std::fs` writes.
//! - **Configuration lens ([`config`]):** parses `[webmention]` from
//!   `.zetl/config.toml`.
//! - **Storage primitives ([`storage`]):** JSONL append + read for the
//!   `.zetl/webmentions/{received,sent,queue}.jsonl` triple per ADR-3904.
//!
//! Effectful shell:
//! - [`persist`] — typed wrappers over [`storage`] for received edges,
//!   the moderation queue, and the sender idempotency log.
//! - [`receive`] — Axum POST /webmention handler + source-fetch +
//!   pipeline.
//! - [`send`] — outbound build/serve sender.
//! - [`graph`] — read-side merge of external mentions into the
//!   backlinks view.
//! - [`cli`] — `zetl webmention` clap argument types.
//!
//! Reused from SPEC-038: [`crate::feed::fetch`] (SSRF/scheme/size
//! primitives + [`crate::feed::fetch::HttpTransport`]).
//!
//! ADR resolutions baked into v1 (per [`plans/IMPL-039-webmention.spl`]):
//! ADR-3901 = all four flows; ADR-3902 = serve-only receive (static via
//! recipe); ADR-3903 = hybrid defeasible-rule moderation with pure-Rust
//! fallback; ADR-3904 = central JSONL storage; ADR-3905 = URL-only
//! identity; ADR-3906 = persisted JSONL idempotency log.

pub mod cli;
pub mod config;
pub mod core;
pub mod graph;
pub mod persist;
pub mod receive;
pub mod send;
pub mod storage;
pub mod transport;
pub mod types;

pub use config::WebmentionConfig;
pub use types::{
    ExternalEdge, IncomingMention, ModerationDecision, ModerationKind, OutboundMention, SentRecord,
    VerifiedMention, WebmentionError,
};
