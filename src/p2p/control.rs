//! CBCL network control plane (SPEC-047 ADR-479, REQ-487/488, IMPL-047 T7).
//!
//! **UNREVIEWED** — see the banner in [`super`]. The attestation/signature path
//! is auth-core (Q7/Q11); the recogniser itself is a LangSec boundary.
//!
//! Per the adr-control-proto resolution (SPEC-047 0.21.0), [[CBCL]] governs the
//! **network** control plane (peer ↔ peer); the local daemon socket stays JSON.
//! This module is the single recogniser for network control messages
//! (REQ-487): every `zetl-sync` / `zetl-pair` control frame is parsed by the
//! *one* shared `cbcl-parser` pipeline — a Lean-verified DPDA over a decidable
//! context-free language — so there is no parser-differential surface by
//! construction. Recognition is **fail-closed**: any parse or validation error
//! rejects the whole frame; downstream code only ever sees a typed [`Message`].
//!
//! Scope of this slice: the shared recogniser + typed construction of the core
//! control performatives. Deferred: the full `zetl-sync`/`zetl-pair` dialect
//! definitions with R5 choreography contracts (REQ-488), and the per-message
//! attestation (REQ-487 auth path) — both build on this recogniser.

use anyhow::{bail, Result};
use cbcl_core::message::{Message, Performative};
use cbcl_parser::pipeline::{run_pipeline, PipelineResult};

/// Recognise one network control message against the shared CBCL grammar
/// (REQ-487). Returns the typed [`Message`], or a fail-closed rejection on any
/// parse/validation error — malformed input is never partially acted upon
/// ([[PROTO-001]] Principle 14).
pub fn recognise(input: &str) -> Result<Message> {
    match run_pipeline(input) {
        PipelineResult::Success(msg) => Ok(msg),
        PipelineResult::ParseError(e) => bail!("cbcl control parse error: {e:?}"),
        PipelineResult::ValidationError(e) => bail!("cbcl control validation error: {e:?}"),
        other => bail!("cbcl control message not recognised: {other:?}"),
    }
}

/// The performative of a recognised control message, if it carries one.
pub fn performative(msg: &Message) -> Option<&Performative> {
    msg.performative()
}

#[cfg(test)]
mod tests {
    use super::*;

    // REQ-487: a well-formed core control message is recognised by the shared
    // pipeline into a typed Message.
    #[test]
    fn recognises_core_control_message() {
        // A `tell` to a peer carrying an opaque control payload — the core
        // performative shape the zetl-sync dialect builds on.
        let msg = recognise("(tell @peer \"reconcile\")").expect("valid control message");
        assert!(
            performative(&msg).is_some(),
            "recognised message carries a performative"
        );
    }

    // Principle 14 / REQ-483: malformed input is rejected whole (fail-closed),
    // never partially recognised.
    #[test]
    fn rejects_malformed_input() {
        // Unbalanced parens — a parse error.
        assert!(recognise("(tell @peer \"reconcile\"").is_err());
        // Empty input.
        assert!(recognise("").is_err());
        // Garbage.
        assert!(recognise("not an s-expression at all }{").is_err());
    }

    // The recogniser is total over arbitrary bytes — it never panics, only
    // returns Ok/Err (the DPDA is a decidable recogniser).
    #[test]
    fn recognition_is_total() {
        for input in [
            "",
            "(",
            ")",
            "((((",
            "(tell)",
            "(reply)",
            "\0\0",
            "(unknown-perf @x \"y\")",
        ] {
            let _ = recognise(input); // must not panic
        }
    }
}
