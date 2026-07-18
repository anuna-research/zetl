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

/// A node's **device identity** — the single Ed25519 key a user creates *before
/// joining* a vault (SPEC-047 REQ-476/491/497). It is the one root from which
/// every identity a node presents is derived, so a node/user is the same
/// identity across all three layers:
///
///  - **transport** — the key's public half is the iroh QUIC endpoint id
///    (the [[NodeId]] the node is dialled at, authenticated by the handshake);
///  - **membership** — its self-certifying [[did:crdt]] DID,
///    `did:crdt:blake3(genesis ‖ key)` (via [`MemberIdentity`]);
///  - **content** — its stable Loro actor id, which attributes the node's edits
///    (so authorship resolves, through the DID, to the node/user).
///
/// "Create a key before joining" is [`DeviceIdentity::generate`]; the secret is
/// persisted (0600) and never leaves the device. A **node** is one device key;
/// a **user** is the DID, which may (multi-device, once did:crdt signs deltas —
/// REQ-500) span several device keys as verification methods.
pub struct DeviceIdentity {
    secret: [u8; 32],
    endpoint_id: [u8; 32],
}

impl DeviceIdentity {
    /// Create a fresh device key — the step a node performs before it can join.
    pub fn generate() -> DeviceIdentity {
        Self::from_secret(iroh::SecretKey::generate().to_bytes())
    }

    /// Reconstruct from a persisted 32-byte secret.
    pub fn from_secret(secret: [u8; 32]) -> DeviceIdentity {
        let endpoint_id = *iroh::SecretKey::from_bytes(&secret).public().as_bytes();
        DeviceIdentity { secret, endpoint_id }
    }

    /// The transport secret (persist 0600; feeds `iroh::SecretKey`).
    pub fn secret(&self) -> &[u8; 32] {
        &self.secret
    }

    /// The public endpoint id — the QUIC [[NodeId]] *and* the did:crdt device
    /// key. One public key serves transport addressing and content identity.
    pub fn endpoint_id(&self) -> &[u8; 32] {
        &self.endpoint_id
    }

    /// This node's self-certifying DID, derived from the device key.
    pub fn did(&self) -> Result<String> {
        Ok(MemberIdentity::genesis(&self.endpoint_id)?.did())
    }

    /// The stable Loro actor id (PeerID) for this device: deterministic in the
    /// key, so a replica keeps one actor across restarts and its edits attribute
    /// to this device — hence, via [`Self::did`], to this node/user. Masked to
    /// 63 bits (Loro reserves only `u64::MAX`).
    ///
    // NOTE for review: this makes attribution *consistent and meaningful*, not
    // *unforgeable* — Loro ops are not signed, so a dishonest member could set
    // another's actor id. Adversarial per-edit provenance needs op signing
    // (separate design, flagged in the crypto review guide §1a).
    pub fn loro_peer(&self) -> u64 {
        let mut b = [0u8; 8];
        b.copy_from_slice(&self.endpoint_id[..8]);
        u64::from_le_bytes(b) & (u64::MAX >> 1)
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

    // One device key yields one identity across all three layers, and survives a
    // persist/reload of just the secret (the "create a key before joining" step).
    #[test]
    fn device_identity_unifies_the_layers() {
        let dev = DeviceIdentity::generate();

        // endpoint id is the public half of the secret; DID is derived from it.
        assert_eq!(
            dev.did().unwrap(),
            MemberIdentity::genesis(dev.endpoint_id()).unwrap().did()
        );
        assert!(dev.did().unwrap().starts_with("did:crdt:"));
        // Loro actor id is stable in the key and never the reserved value.
        assert_ne!(dev.loro_peer(), u64::MAX);

        // Reconstructing from the persisted secret reproduces every derivation.
        let reloaded = DeviceIdentity::from_secret(*dev.secret());
        assert_eq!(reloaded.endpoint_id(), dev.endpoint_id());
        assert_eq!(reloaded.did().unwrap(), dev.did().unwrap());
        assert_eq!(reloaded.loro_peer(), dev.loro_peer());
    }

    // A note's edits attribute to the device's stable actor id, not a random one.
    #[test]
    fn edits_attribute_to_the_device_actor() {
        use crate::crdt::loro_store::NoteDoc;
        let dev = DeviceIdentity::from_secret([5u8; 32]);
        let mut note = NoteDoc::new();
        note.set_actor(dev.loro_peer()).unwrap();
        note.set_content("edited by this device").unwrap();
        assert_eq!(note.actor(), dev.loro_peer(), "the edit's actor is the device id");
    }
}
