//! `grants.toml` parsing and validation (SPEC-034 REQ-3408 / CON-3402).
//!
//! The schema is the committed, git-tracked record of who has been
//! issued what access. The pure-core validator checks structural
//! invariants (unique ids, referenced cohorts, well-formed recipients,
//! parseable timestamps) without touching the filesystem or clock; the
//! effectful shell (`cap::cli::commands`) is responsible for reading
//! the file, calling `validate`, and reporting diagnostics.

pub mod validation;
