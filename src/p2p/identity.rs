//! did:crdt member identity (SPEC-047 ADR-481, REQ-497, IMPL-047 T11).
//!
//! **UNREVIEWED AUTH-CORE** — see the banner in [`super`]. Pending Q1/Q2/Q7/Q11.
//!
//! A *member* is a [`did_crdt`] DID whose verification methods enumerate that
//! member's device public keys ([[NodeId]]s). This wraps the sibling
//! `../did-crdt` crate's [`Document`] to give the roster (T10) and MLS (T9)
//! layers a member-identity primitive: create a member from its first device
//! key, resolve to the W3C DID document (the device set), and persist.
//!
//! **Substance limit (did-crdt phase-1):** the upstream crate does not yet wire
//! delta *signing* (`Document::new` returns a genesis delta with an empty
//! signature — "no signing infrastructure in this phase"). So this layer lands
//! the identity *structure*; the security-critical signed-delta authentication
//! that REQ-500 (order-independent DID authorization) needs is completed when
//! did-crdt wires signing, and reviewed under Q11. Do not treat DID deltas as
//! authenticated yet.

use anyhow::{Context, Result};
use base64::Engine as _;
use did_crdt::core::document::Document;
use did_crdt::core::resolve::DidDocument;

/// A member's did:crdt identity document.
pub struct MemberIdentity {
    doc: Document,
}

/// Encode a raw 32-byte device public key as the multibase base64url (`u`
/// prefix, no padding) form did-crdt expects for `public_key_multibase`.
fn multibase_key(pubkey: &[u8; 32]) -> String {
    format!(
        "u{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(pubkey)
    )
}

impl MemberIdentity {
    /// Create a fresh member identity whose genesis verification method is
    /// `device_pubkey` (the member's first device — REQ-497). The DID is
    /// derived from the creation hash, so it is stable and content-addressed.
    pub fn genesis(device_pubkey: &[u8; 32]) -> Result<MemberIdentity> {
        // did-crdt returns (document, genesis-delta); the delta carries the
        // AddVerificationMethod op. Signing is upstream's follow-up (see the
        // module note), so we do not broadcast the delta here.
        let (doc, _genesis_delta) =
            Document::new(&multibase_key(device_pubkey)).context("create did:crdt document")?;
        Ok(MemberIdentity { doc })
    }

    /// The member's DID string (`did:crdt:<hash>`).
    pub fn did(&self) -> String {
        self.doc.did.as_str().to_string()
    }

    /// Resolve to the W3C DID document — the member's current device set
    /// (verification methods), or `None` if the DID is deactivated.
    pub fn resolve(&self) -> Result<Option<DidDocument>> {
        Ok(self.doc.resolve().context("resolve did:crdt")?.did_document)
    }

    /// Serialise the identity document for persistence.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        self.doc.to_bytes().context("serialise did:crdt document")
    }

    /// Reload a persisted identity document.
    pub fn from_bytes(bytes: &[u8]) -> Result<MemberIdentity> {
        Ok(MemberIdentity {
            doc: Document::from_bytes(bytes).context("load did:crdt document")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // REQ-497: a member identity is a did:crdt DID; its resolved document
    // carries the creating device as a verification method.
    #[test]
    fn genesis_creates_resolvable_did() {
        let device = [7u8; 32];
        let member = MemberIdentity::genesis(&device).unwrap();

        let did = member.did();
        assert!(did.starts_with("did:crdt:"), "DID has the crdt method: {did}");

        let resolved = member.resolve().unwrap().expect("not deactivated");
        assert_eq!(resolved.id, did, "resolved document id is the DID");
        assert!(
            !resolved.verification_method.is_empty(),
            "the genesis device is a verification method"
        );
    }

    // The DID is deterministic in the device key (content-addressed creation).
    #[test]
    fn did_is_deterministic_in_the_key() {
        let a = MemberIdentity::genesis(&[1u8; 32]).unwrap().did();
        let b = MemberIdentity::genesis(&[1u8; 32]).unwrap().did();
        let c = MemberIdentity::genesis(&[2u8; 32]).unwrap().did();
        assert_eq!(a, b, "same device key → same DID");
        assert_ne!(a, c, "different device key → different DID");
    }

    // Persistence round-trips the identity document.
    #[test]
    fn persistence_round_trips() {
        let member = MemberIdentity::genesis(&[9u8; 32]).unwrap();
        let bytes = member.to_bytes().unwrap();
        let reloaded = MemberIdentity::from_bytes(&bytes).unwrap();
        assert_eq!(reloaded.did(), member.did());
    }
}
