//! `recipients.toml` parsing (SPEC-034 REQ-3409 / CON-3403).
//!
//! The file is the operator-facing record of each cohort's active
//! recipient pubkeys plus the vault's Ed25519 signing pubkey. Pure
//! core: parses and validates; effectful shell reads the file and
//! writes it on `zetl cap invite`.

pub mod parsing;
