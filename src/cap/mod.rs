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
pub mod derivation;
pub mod emergency_shutdown;
pub mod genkey;
pub mod grants;
pub mod pad;
pub mod recipients;
pub mod sanitiser;
pub mod scoping;
pub mod url_format;
