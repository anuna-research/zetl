//! SPEC-038 RSS / Atom / JSON Feed support.
//!
//! Module layout follows SPEC-038 §9 Purity Boundary Map:
//!
//! - **Pure core (this directory's leaf modules):** [`types`], [`item_id`],
//!   [`resolve_date`], [`rewrite_links`], [`license_resolve`],
//!   [`republication`], [`excerpt`], [`select`]. No I/O, no clock reads,
//!   no `tokio` / `axum` / `git2` / `std::fs` writes; deterministic for
//!   identical inputs across machines and zetl versions in the same
//!   major-version line.
//! - **Configuration lens:** [`config`] reads `[feed]`, `[[feed.scopes]]`,
//!   `[feed.changelog]`, `[[capability_cohorts]]`, `[[subscriptions]]`,
//!   and the `[wiki]` extension (`self_license`, `is_commercial`) from
//!   `.zetl/config.toml`. Pure parser; the shell decides where to read
//!   the file from.
//! - **Credentials store:** [`credentials`] reads `.zetl/credentials.toml`
//!   under the mode-0600 invariant of CON-3812 / ADR-3810. Effectful but
//!   side-effect-only at the file boundary; the parsed scheme structs
//!   remain inert data.
//!
//! Effectful shell modules:
//! - [`build`] — outbound build-mode emission (file bodies + stats)
//! - [`serve`] — outbound serve-mode response builders
//! - [`scoped`] — Hugo's scoped subscription catalog + per-scope feeds
//! - [`changelog`] — AST-backed changelog feed events
//! - [`cap_feed`] — capability-cohort feed exclusion + emission
//! - [`fetch`] — SSRF/XXE/decompression-safe inbound fetcher
//!   (transport behind the [`fetch::HttpTransport`] trait)
//! - [`auth`] — inbound Basic/Bearer/QueryParam auth + redirect drop
//! - [`inbound`] — CC-aware republication wiring
//! - [`retention`] / [`forget`] — retention pruning + tombstone-backed
//!   `zetl feed forget`
//! - [`cli`] — `zetl feed` clap argument types
//! - [`observability`] — metric counters + histogram + cardinality bounds

pub mod auth;
pub mod build;
pub mod cap_feed;
pub mod changelog;
pub mod cli;
pub mod config;
pub mod datetime;
pub mod observability;
pub mod credentials;
pub mod excerpt;
pub mod fetch;
pub mod forget;
pub mod inbound;
pub mod item_id;
pub mod license_resolve;
pub mod republication;
pub mod resolve_date;
pub mod retention;
pub mod rewrite_links;
pub mod scoped;
pub mod select;
pub mod serialise_atom;
pub mod serialise_jsonfeed;
pub mod serialise_rss;
pub mod serve;
pub mod types;
pub mod xml;

pub use types::{
    AuthorRef, FeedConfig, FeedItem, License, OutputFormat, RepublicationDecision,
    RepublicationMode, RepublicationRationale, RetentionPolicy, SelectionRule,
    SourceMetadata,
};
