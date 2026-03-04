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

pub mod jj_backend;
