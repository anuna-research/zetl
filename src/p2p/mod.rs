//! SPEC-047 P2P realtime sync — the network layer (IMPL-047 M2).
//!
//! ╔══════════════════════════════════════════════════════════════════════╗
//! ║  UNREVIEWED AUTH-CORE / CRYPTO — NOT YET APPROVED FOR MERGE OR USE.    ║
//! ║                                                                        ║
//! ║  This module and its children implement the security-sensitive P2P    ║
//! ║  layer of SPEC-047 (pairing, and — when the heavy deps can be built —  ║
//! ║  MLS group key, roster, did:crdt, sync frames, revocation). Per        ║
//! ║  PROTO-001 §AI Trust Boundaries this is Tier-1 no-go-area code that    ║
//! ║  REQUIRES a human cryptography + auth-core review (SPEC-047 §15,        ║
//! ║  Open Questions Q1/Q2/Q7/Q11) before it is trusted, merged, or         ║
//! ║  shipped. It is implemented here on a branch, verified only            ║
//! ║  *mechanically* (it compiles and its behavioural tests pass) — that    ║
//! ║  is NOT evidence of cryptographic soundness. The DESIGN-047            ║
//! ║  `crypto-review` task remains the gate: do not assume this is secure.  ║
//! ╚══════════════════════════════════════════════════════════════════════╝
//!
//! Reuse over reinvention: pairing composes the audited [`crate::cap::pair`]
//! SPAKE2 primitive (SPEC-034); the group key composes `openmls` (ADR-482);
//! member identity composes `../did-crdt` (ADR-481).
//!
//! Implemented (UNREVIEWED): [`pair`] (T8 SPAKE2 ceremony), [`group`] (T9 MLS
//! Owner-committed membership), [`roster`] (T10 role gate), [`identity`] (T11
//! did:crdt), [`revocation`] (T13 durable rotation). NOT implemented: the iroh
//! QUIC transport (T12 — iroh 0.21 has an upstream compile error; a newer iroh
//! needs a different integration) and the CBCL network control plane (T7).

pub mod group;
pub mod identity;
pub mod pair;
pub mod revocation;
pub mod roster;
