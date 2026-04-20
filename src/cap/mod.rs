//! Capability-URL static-distribution mode (SPEC-034).
//!
//! This module tree hosts the Rust portion of the capability-mode build
//! pipeline: key derivation, URL format, grants/recipients schemas, age
//! encryption + Ed25519 signing, HTML sanitisation, deploy-artifact
//! emission, and the `zetl cap` CLI verbs.
//!
//! See `specs/SPEC-034.md` §8 (Purity Boundary Map) for which modules
//! live in the pure core vs the effectful shell.

pub mod age_encrypt;
pub mod audit_diff;
pub mod build;
pub mod deploy_headers;
pub mod derivation;
pub mod emergency_shutdown;
pub mod genkey;
pub mod grants;
pub mod html_shell;
pub mod invite;
pub mod pad;
pub mod pair;
pub mod recipients;
pub mod sanitiser;
pub mod scoping;
pub mod sign;
pub mod url_format;
