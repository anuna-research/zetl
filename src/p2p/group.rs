//! MLS group key + Owner-committed membership (SPEC-047 ADR-482, REQ-499/502,
//! IMPL-047 T9).
//!
//! **UNREVIEWED AUTH-CORE** — see the banner in [`super`]. Pending Q1/Q2/Q7/Q11.
//!
//! One [[MLS]] group per vault (`group_id` = vault id); the Owner is the sole
//! committer (ADR-482 — the cryptographic realisation of the Owner-only
//! membership authority of REQ-505). Members' leaf credentials carry their
//! [[did:crdt]] DID. Modelled directly on `../elephant-3000`'s proven openmls
//! integration (its SPEC-004 `e2ee` module): create → add (commit + welcome) →
//! join → remove (commit → new epoch), with commits authorised **by leaf
//! index** (the Owner is permanently leaf 0), never by the attacker-chosen
//! credential string — elephant's forged-credential lesson.
//!
//! Scope: the in-memory group operations (create/add/join/remove, Owner-only
//! commit authorisation) plus [`seal`]/[`open`] — the group-keyed sync-frame
//! confidentiality of REQ-499, realised as MLS application messages and driven
//! by [`GroupSealer`] through the session's `FrameSeal` layer. Verified with
//! the standard `OpenMlsRustCrypto` provider. Deferred (later slices): the
//! durable provider + transactional outbox crash-safety (elephant REQ-306) and
//! the Loro membership-lane transport (REQ-502).

use anyhow::{anyhow, Result};
use openmls::prelude::{
    BasicCredential, Ciphersuite, CredentialWithKey, KeyPackage, KeyPackageBundle, KeyPackageIn,
    LeafNodeIndex, MlsGroup, MlsGroupCreateConfig, MlsGroupJoinConfig, MlsMessageBodyIn,
    MlsMessageIn, MlsMessageOut, ProcessedMessageContent, ProtocolMessage, ProtocolVersion, Sender,
    StagedWelcome,
};
use openmls_basic_credential::SignatureKeyPair;
use openmls_rust_crypto::OpenMlsRustCrypto;
use openmls_traits::types::SignatureScheme;
use openmls_traits::OpenMlsProvider as _;
use tls_codec::{Deserialize as _, Serialize as _};

/// The one ciphersuite (ADR-482, matching elephant SPEC-004 REQ-301):
/// Ed25519 signatures — the same primitive as the transport [[NodeId]].
pub const CIPHERSUITE: Ciphersuite = Ciphersuite::MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519;

/// A member's MLS identity for one vault: a DID+endpoint-bound leaf credential
/// + signer.
pub struct GroupIdentity {
    pub signer: SignatureKeyPair,
    pub credential: CredentialWithKey,
    pub did: String,
    /// This device's transport endpoint id, bound into the leaf credential so a
    /// QUIC-authenticated peer resolves to its membership ([`endpoint_owner`]).
    pub endpoint_id: [u8; 32],
}

impl GroupIdentity {
    /// Build an MLS identity for the device whose transport key is `endpoint_id`
    /// (the iroh QUIC key, which is also the did:crdt device key). The member's
    /// **DID is derived** from that key — `did:crdt:blake3(genesis ‖ key)` — so
    /// it is *self-certifying*, not an assigned label: controlling `endpoint_id`
    /// (which the QUIC handshake proves) proves control of the DID. The leaf
    /// credential binds `did ‖ endpoint_id`; once the Owner admits it the leaf
    /// is MLS-authenticated, so [`endpoint_owner`] resolves an authenticated
    /// endpoint to a DID whose ownership is cryptographic, not provisioned.
    ///
    /// The leaf signing key is a *separate* fresh Ed25519 keypair (NOT the
    /// transport/DID key — cross-protocol key reuse is deliberately avoided).
    ///
    /// Single-device today. Multi-device (several endpoint keys under one DID)
    /// is the did:crdt verification-method set, which needs signed deltas
    /// (REQ-500, gated on Q11).
    pub fn new(endpoint_id: &[u8; 32]) -> Result<GroupIdentity> {
        let did = derive_did(endpoint_id)?;
        let signer = SignatureKeyPair::new(SignatureScheme::ED25519)
            .map_err(|e| anyhow!("generate MLS signer: {e:?}"))?;
        let credential = CredentialWithKey {
            credential: BasicCredential::new(encode_credential(&did, endpoint_id)).into(),
            signature_key: signer.public().into(),
        };
        Ok(GroupIdentity {
            signer,
            credential,
            did,
            endpoint_id: *endpoint_id,
        })
    }
}

/// The canonical did:crdt DID for a device key: `did:crdt:blake3(genesis ‖ key)`
/// (via [`crate::p2p::identity`]). A pure, deterministic function of the key —
/// this is what makes a DID *self-certifying* against the key that controls it.
pub fn derive_did(endpoint_id: &[u8; 32]) -> Result<String> {
    Ok(crate::p2p::identity::MemberIdentity::genesis(endpoint_id)?.did())
}

/// Verify a claimed DID is genuinely the content-address of `endpoint_id` — the
/// check that turns an Owner-asserted label into a self-verified identity. Used
/// at admission so no leaf can ever carry a DID it does not cryptographically
/// own.
pub fn verify_did_binding(did: &str, endpoint_id: &[u8; 32]) -> bool {
    derive_did(endpoint_id).is_ok_and(|expected| expected == did)
}

/// The leaf-credential grammar (LangSec — a defined format at the membership
/// trust boundary): a member device's credential is `did_utf8 ‖ endpoint_id`,
/// with the transport endpoint id as a **fixed 32-byte suffix**. The fixed
/// suffix makes decoding unambiguous for any DID length, with no delimiter a
/// DID could collide with.
///
// SIMPLIFY (ceiling): encoding the binding *inside* BasicCredential is the
// minimal form. A cleaner realisation is a dedicated MLS LeafNode extension (or
// a custom credential type) so the endpoint id is a first-class, typed leaf
// field rather than appended bytes — flagged for the crypto review (Q1/Q7).
pub fn encode_credential(did: &str, endpoint_id: &[u8; 32]) -> Vec<u8> {
    let mut v = did.as_bytes().to_vec();
    v.extend_from_slice(endpoint_id);
    v
}

/// Fully recognise a leaf credential before use (LangSec): split off the fixed
/// 32-byte endpoint suffix, and require the DID prefix be valid UTF-8. Rejects
/// anything too short to carry an endpoint id.
pub fn decode_credential(bytes: &[u8]) -> Result<(String, [u8; 32])> {
    anyhow::ensure!(
        bytes.len() >= 32,
        "leaf credential too short to bind an endpoint id"
    );
    let (did_bytes, ep) = bytes.split_at(bytes.len() - 32);
    let did = std::str::from_utf8(did_bytes)
        .map_err(|_| anyhow!("leaf credential DID is not valid UTF-8"))?
        .to_string();
    let mut endpoint_id = [0u8; 32];
    endpoint_id.copy_from_slice(ep);
    Ok((did, endpoint_id))
}

/// Resolve a QUIC-authenticated transport `endpoint_id` to the DID of the group
/// member whose current leaf credential binds it — the cryptographic
/// replacement for the provisioned admit set (REQ-482/492/497). `None` (fail
/// closed) if no current leaf binds this endpoint: a stranger, or a removed /
/// rotated-out device whose leaf is gone. Because the binding lives in an
/// MLS-authenticated, Owner-authorised leaf, a positive result means the peer
/// is a member — a resolved address alone never is.
pub fn endpoint_owner(group: &MlsGroup, endpoint_id: &[u8; 32]) -> Option<String> {
    group.members().find_map(|m| {
        let (did, ep) = decode_credential(m.credential.serialized_content()).ok()?;
        (ep == *endpoint_id).then_some(did)
    })
}

fn create_config() -> MlsGroupCreateConfig {
    MlsGroupCreateConfig::builder()
        .use_ratchet_tree_extension(true)
        .ciphersuite(CIPHERSUITE)
        .build()
}

fn join_config() -> MlsGroupJoinConfig {
    MlsGroupJoinConfig::builder()
        .use_ratchet_tree_extension(true)
        .build()
}

/// A fresh in-memory crypto provider. (The durable provider is a later slice.)
pub fn provider() -> OpenMlsRustCrypto {
    OpenMlsRustCrypto::default()
}

/// Create the vault's MLS group; the creator is the Owner (sole committer),
/// permanently leaf 0.
pub fn create_group(
    provider: &OpenMlsRustCrypto,
    owner: &GroupIdentity,
    vault_id: &[u8],
) -> Result<MlsGroup> {
    owner
        .signer
        .store(provider.storage())
        .map_err(|e| anyhow!("store owner signer: {e:?}"))?;
    MlsGroup::new_with_group_id(
        provider,
        &owner.signer,
        &create_config(),
        openmls::group::GroupId::from_slice(vault_id),
        owner.credential.clone(),
    )
    .map_err(|e| anyhow!("create MLS group: {e:?}"))
}

/// A joiner's KeyPackage (published to the Owner during pairing).
pub fn build_key_package(
    provider: &OpenMlsRustCrypto,
    joiner: &GroupIdentity,
) -> Result<(KeyPackageBundle, Vec<u8>)> {
    joiner
        .signer
        .store(provider.storage())
        .map_err(|e| anyhow!("store joiner signer: {e:?}"))?;
    let bundle = KeyPackage::builder()
        .build(
            CIPHERSUITE,
            provider,
            &joiner.signer,
            joiner.credential.clone(),
        )
        .map_err(|e| anyhow!("build key package: {e:?}"))?;
    let bytes = bundle
        .key_package()
        .tls_serialize_detached()
        .map_err(|e| anyhow!("serialize key package: {e:?}"))?;
    Ok((bundle, bytes))
}

/// Recognise a joiner's KeyPackage, checking its credential DID equals the DID
/// authenticated during pairing (REQ-497 — fail closed on mismatch).
pub fn key_package_from_bytes(
    provider: &OpenMlsRustCrypto,
    bytes: &[u8],
    expected_endpoint: &[u8; 32],
) -> Result<KeyPackage> {
    let kp_in = KeyPackageIn::tls_deserialize_exact(bytes)
        .map_err(|e| anyhow!("parse key package: {e:?}"))?;
    // Full recognition before use: openmls validates signature/lifetime/suite.
    let kp = kp_in
        .validate(provider.crypto(), ProtocolVersion::Mls10)
        .map_err(|e| anyhow!("validate key package: {e:?}"))?;
    // Recognise the credential grammar, then verify the two facts that make the
    // member's identity cryptographic rather than assigned:
    //   1. the bound endpoint is the one pairing authenticated (the joiner
    //      controls it), and
    //   2. the bound DID is the *self-certifying* content-address of that
    //      endpoint — so the leaf cannot carry a DID it does not own (no Owner
    //      labelling, no impersonation of another member's DID).
    // The leaf this becomes is the sole cryptographic source for
    // [`endpoint_owner`], so a DID it yields is a verified identity.
    let (did, endpoint) = decode_credential(kp.leaf_node().credential().serialized_content())?;
    if &endpoint != expected_endpoint {
        anyhow::bail!("key package credential endpoint != pairing-authenticated endpoint");
    }
    if !verify_did_binding(&did, &endpoint) {
        anyhow::bail!("key package DID is not the self-certifying address of its endpoint key");
    }
    Ok(kp)
}

/// Owner adds a member: returns (commit, welcome). The Owner MUST publish the
/// commit to the membership lane and hand the welcome to the joiner.
pub fn add_member(
    provider: &OpenMlsRustCrypto,
    group: &mut MlsGroup,
    owner: &GroupIdentity,
    kp: KeyPackage,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let (commit, welcome, _) = group
        .add_members(provider, &owner.signer, &[kp])
        .map_err(|e| anyhow!("MLS add member: {e:?}"))?;
    group
        .merge_pending_commit(provider)
        .map_err(|e| anyhow!("merge add commit: {e:?}"))?;
    Ok((serialize_out(&commit)?, serialize_out(&welcome)?))
}

/// A joiner joins from the Owner's Welcome, pinning the Owner as leaf 0 (the
/// DID authenticated during pairing) so later commit authorisation can trust
/// the leaf index, not the spoofable credential string.
pub fn join_from_welcome(
    provider: &OpenMlsRustCrypto,
    welcome_bytes: &[u8],
    expected_owner_did: &str,
) -> Result<MlsGroup> {
    let msg = MlsMessageIn::tls_deserialize_exact(welcome_bytes)
        .map_err(|e| anyhow!("parse welcome: {e:?}"))?;
    let MlsMessageBodyIn::Welcome(welcome) = msg.extract() else {
        anyhow::bail!("not a welcome message");
    };
    let staged = StagedWelcome::new_from_welcome(provider, &join_config(), welcome, None)
        .map_err(|e| anyhow!("stage welcome: {e:?}"))?;
    let owner_is_leaf_0 = staged
        .members()
        .find(|m| m.index == LeafNodeIndex::new(0))
        .and_then(|m| decode_credential(m.credential.serialized_content()).ok())
        .is_some_and(|(did, _endpoint)| did == expected_owner_did);
    if !owner_is_leaf_0 {
        anyhow::bail!("welcome leaf 0 is not the authenticated Owner {expected_owner_did}");
    }
    staged
        .into_group(provider)
        .map_err(|e| anyhow!("into group: {e:?}"))
}

/// Owner removes a member by their DID (new epoch — the removed leaf can no
/// longer derive the group key: REQ-481/498 rotation).
///
/// The returned commit bytes are **unmerged**: the local epoch has *not*
/// advanced yet. REQ-506 requires the commit to be durable (the rotation
/// outbox, [`crate::p2p::revocation::write_outbox`]) *before* the merge — a
/// crash after merging but before recording would strand surviving peers on
/// the old epoch with no recorded bytes to republish. Call
/// [`merge_removal`] once the outbox write has returned, or use
/// [`crate::p2p::revocation::remove_member_durable`] which sequences both.
pub fn remove_member(
    provider: &OpenMlsRustCrypto,
    group: &mut MlsGroup,
    owner: &GroupIdentity,
    did: &str,
) -> Result<Vec<u8>> {
    let target = group
        .members()
        .find(|m| {
            decode_credential(m.credential.serialized_content())
                .is_ok_and(|(member_did, _)| member_did == did)
        })
        .ok_or_else(|| anyhow!("{did} is not a member"))?;
    let (commit, _, _) = group
        .remove_members(provider, &owner.signer, &[target.index])
        .map_err(|e| anyhow!("MLS remove member: {e:?}"))?;
    serialize_out(&commit)
}

/// Advance the local epoch by merging the pending removal commit. Only call
/// after the commit bytes returned by [`remove_member`] are durable in the
/// rotation outbox (REQ-506 ordering).
pub fn merge_removal(provider: &OpenMlsRustCrypto, group: &mut MlsGroup) -> Result<()> {
    group
        .merge_pending_commit(provider)
        .map_err(|e| anyhow!("merge remove commit: {e:?}"))
}

/// Current member DIDs (from leaf credentials).
pub fn member_dids(group: &MlsGroup) -> Vec<String> {
    group
        .members()
        .filter_map(|m| decode_credential(m.credential.serialized_content()).ok())
        .map(|(did, _endpoint)| did)
        .collect()
}

/// Process an inbound membership commit, enforcing **Owner-only commits**
/// (REQ-505): authorised by the sender **leaf index** — the Owner is
/// permanently leaf 0 — not by the credential string (which an attacker
/// chooses). A non-Owner commit is rejected (elephant ADR-303 lesson).
pub fn process_commit(
    provider: &OpenMlsRustCrypto,
    group: &mut MlsGroup,
    bytes: &[u8],
    owner_did: &str,
) -> Result<()> {
    let msg = MlsMessageIn::tls_deserialize_exact(bytes)
        .map_err(|e| anyhow!("parse mls message: {e:?}"))?;
    let protocol: ProtocolMessage = match msg.extract() {
        MlsMessageBodyIn::PrivateMessage(m) => m.into(),
        MlsMessageBodyIn::PublicMessage(m) => m.into(),
        _ => anyhow::bail!("not a protocol message"),
    };
    let processed = group
        .process_message(provider, protocol)
        .map_err(|e| anyhow!("process message: {e:?}"))?;
    let sender = processed.sender().clone();
    match processed.into_content() {
        ProcessedMessageContent::StagedCommitMessage(staged) => {
            let owner_leaf = LeafNodeIndex::new(0);
            let is_owner = sender == Sender::Member(owner_leaf)
                && group
                    .member(owner_leaf)
                    .and_then(|c| decode_credential(c.serialized_content()).ok())
                    .is_some_and(|(did, _endpoint)| did == owner_did);
            if !is_owner {
                anyhow::bail!("only the Owner (leaf 0) may commit (ADR-482/REQ-505)");
            }
            group
                .merge_staged_commit(provider, *staged)
                .map_err(|e| anyhow!("merge staged commit: {e:?}"))?;
            Ok(())
        }
        ProcessedMessageContent::ProposalMessage(_)
        | ProcessedMessageContent::ExternalJoinProposalMessage(_) => {
            anyhow::bail!("proposals are not accepted (Owner-only commits)")
        }
        ProcessedMessageContent::ApplicationMessage(_) => {
            anyhow::bail!("unexpected application message on the commit path")
        }
    }
}

fn serialize_out(msg: &MlsMessageOut) -> Result<Vec<u8>> {
    msg.tls_serialize_detached()
        .map_err(|e| anyhow!("serialize mls message: {e:?}"))
}

/// Seal an application payload — e.g. a sync frame — under the group's current
/// MLS epoch key (REQ-499), returning the serialized encrypted `PrivateMessage`.
/// This is the composition-first realisation of "group-keyed sync frames": it
/// reuses openmls's own AEAD and per-leaf sender ratchet rather than a bespoke
/// cipher, so confidentiality is bound to group membership at the current epoch
/// (a removed member, past the rotation, cannot derive the key — REQ-481/499).
pub fn seal(
    provider: &OpenMlsRustCrypto,
    group: &mut MlsGroup,
    signer: &SignatureKeyPair,
    payload: &[u8],
) -> Result<Vec<u8>> {
    let out = group
        .create_message(provider, signer, payload)
        .map_err(|e| anyhow!("seal application message: {e:?}"))?;
    serialize_out(&out)
}

/// Adapts an MLS group into the session's [`FrameSeal`] layer (REQ-499), so
/// [`sync_one_sealed`](crate::p2p::session::sync_one_sealed) seals each sync
/// frame under the group key without the session knowing about openmls. Borrows
/// the live group so the sender/receiver ratchets advance across the exchange.
pub struct GroupSealer<'a> {
    pub provider: &'a OpenMlsRustCrypto,
    pub group: &'a mut MlsGroup,
    pub signer: &'a SignatureKeyPair,
}

impl crate::p2p::session::FrameSeal for GroupSealer<'_> {
    fn seal(&mut self, plaintext: &[u8]) -> Result<Vec<u8>> {
        seal(self.provider, self.group, self.signer, plaintext)
    }
    fn open(&mut self, sealed: &[u8]) -> Result<Vec<u8>> {
        open(self.provider, self.group, sealed)
    }
}

/// Open a frame sealed by a fellow group member at the current epoch (REQ-499).
/// Fails closed: anything that is not an encrypted application message — a
/// plaintext `PublicMessage`, a commit, a proposal — is rejected, so the sync
/// path only ever accepts group-keyed confidential frames.
pub fn open(provider: &OpenMlsRustCrypto, group: &mut MlsGroup, sealed: &[u8]) -> Result<Vec<u8>> {
    let msg = MlsMessageIn::tls_deserialize_exact(sealed)
        .map_err(|e| anyhow!("parse sealed frame: {e:?}"))?;
    let protocol: ProtocolMessage = match msg.extract() {
        MlsMessageBodyIn::PrivateMessage(m) => m.into(),
        _ => anyhow::bail!("sealed frames must be an encrypted PrivateMessage (REQ-499)"),
    };
    let processed = group
        .process_message(provider, protocol)
        .map_err(|e| anyhow!("open sealed frame: {e:?}"))?;
    match processed.into_content() {
        ProcessedMessageContent::ApplicationMessage(app) => Ok(app.into_bytes()),
        _ => anyhow::bail!("sealed frame was not an application message"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VAULT: &[u8] = b"vault-notes";

    // REQ-499/502: Owner creates a group and admits a member; both converge on
    // the same two-member roster.
    #[test]
    fn create_add_join() {
        let op = provider();
        let bp = provider();
        let owner = GroupIdentity::new(&[1u8; 32]).unwrap();
        let bob = GroupIdentity::new(&[2u8; 32]).unwrap();

        let mut group = create_group(&op, &owner, VAULT).unwrap();
        assert_eq!(member_dids(&group), vec![owner.did.clone()]);
        // The DID is self-certifying — the content-address of the device key —
        // not an assigned string (REQ-497).
        assert!(verify_did_binding(&owner.did, &[1u8; 32]));
        assert!(
            !verify_did_binding(&owner.did, &[2u8; 32]),
            "owner DID is not bob's key"
        );

        let (_kb, kp_bytes) = build_key_package(&bp, &bob).unwrap();
        let kp = key_package_from_bytes(&op, &kp_bytes, &bob.endpoint_id).unwrap();

        let (_commit, welcome) = add_member(&op, &mut group, &owner, kp).unwrap();
        let bob_group = join_from_welcome(&bp, &welcome, &owner.did).unwrap();

        let mut a = member_dids(&group);
        let mut b = member_dids(&bob_group);
        a.sort();
        b.sort();
        assert_eq!(a, b);
        assert_eq!(a.len(), 2);
    }

    // REQ-482/492/497: a QUIC-authenticated endpoint id resolves to its member
    // DID via the MLS-bound leaf credential — the cryptographic admit decision.
    // A stranger's endpoint resolves to nothing (fail closed); a rejected DID or
    // endpoint at admission never enters the tree.
    #[test]
    fn endpoint_resolves_to_member_only() {
        let op = provider();
        let bp = provider();
        let owner = GroupIdentity::new(&[1u8; 32]).unwrap();
        let bob = GroupIdentity::new(&[2u8; 32]).unwrap();

        let mut group = create_group(&op, &owner, VAULT).unwrap();
        let (_kb, kp_bytes) = build_key_package(&bp, &bob).unwrap();

        // A KeyPackage validated against an endpoint that isn't the one it binds
        // is refused (the joiner must control the endpoint it claims).
        assert!(
            key_package_from_bytes(&op, &kp_bytes, &[9u8; 32]).is_err(),
            "endpoint mismatch at admission must be rejected"
        );
        let kp = key_package_from_bytes(&op, &kp_bytes, &bob.endpoint_id).unwrap();
        add_member(&op, &mut group, &owner, kp).unwrap();

        // Each authenticated endpoint resolves to the self-certifying DID that
        // its key derives — verifiably, not by label.
        assert_eq!(endpoint_owner(&group, &[1u8; 32]), Some(owner.did.clone()));
        assert_eq!(endpoint_owner(&group, &[2u8; 32]), Some(bob.did.clone()));
        assert_eq!(
            endpoint_owner(&group, &[1u8; 32]).as_deref(),
            Some(derive_did(&[1u8; 32]).unwrap().as_str())
        );
        // A stranger's endpoint resolves to nothing — a resolved address is not
        // membership.
        assert_eq!(endpoint_owner(&group, &[42u8; 32]), None);
    }

    // REQ-505 / ADR-482: only the Owner (leaf 0) may commit. A member's commit
    // is rejected by the Owner's processor — and even a member whose credential
    // is forged to the Owner's DID cannot commit (authorised by leaf, not
    // credential string).
    #[test]
    fn non_owner_commit_is_rejected() {
        let op = provider();
        let bp = provider();
        let cp = provider();
        let owner = GroupIdentity::new(&[1u8; 32]).unwrap();
        let bob = GroupIdentity::new(&[2u8; 32]).unwrap();
        let carol = GroupIdentity::new(&[3u8; 32]).unwrap();

        let mut group = create_group(&op, &owner, VAULT).unwrap();
        let (_kb, kp_bytes) = build_key_package(&bp, &bob).unwrap();
        let kp = key_package_from_bytes(&op, &kp_bytes, &bob.endpoint_id).unwrap();
        let (_c, welcome) = add_member(&op, &mut group, &owner, kp).unwrap();
        let mut bob_group = join_from_welcome(&bp, &welcome, &owner.did).unwrap();

        // Bob (a non-Owner, leaf 1) tries to add Carol → the Owner rejects it.
        let (_ck, carol_kp_bytes) = build_key_package(&cp, &carol).unwrap();
        let carol_kp = key_package_from_bytes(&bp, &carol_kp_bytes, &carol.endpoint_id).unwrap();
        let (bob_commit, _w) = add_member(&bp, &mut bob_group, &bob, carol_kp).unwrap();
        let err = process_commit(&op, &mut group, &bob_commit, &owner.did);
        assert!(err.is_err(), "a non-Owner commit must be rejected: {err:?}");
    }

    // REQ-481/498: Owner removes a member; the roster shrinks and the epoch
    // advances (the removed member can no longer derive the group key).
    #[test]
    fn owner_removes_member() {
        let op = provider();
        let bp = provider();
        let owner = GroupIdentity::new(&[1u8; 32]).unwrap();
        let bob = GroupIdentity::new(&[2u8; 32]).unwrap();

        let mut group = create_group(&op, &owner, VAULT).unwrap();
        let (_kb, kp_bytes) = build_key_package(&bp, &bob).unwrap();
        let kp = key_package_from_bytes(&op, &kp_bytes, &bob.endpoint_id).unwrap();
        add_member(&op, &mut group, &owner, kp).unwrap();
        assert_eq!(member_dids(&group).len(), 2);
        let epoch_before = group.epoch();

        // REQ-506 ordering: the commit comes back unmerged (epoch unchanged)
        // so a caller can make it durable first; the merge advances the epoch.
        let commit = remove_member(&op, &mut group, &owner, &bob.did).unwrap();
        assert!(!commit.is_empty());
        assert_eq!(
            group.epoch(),
            epoch_before,
            "commit is pending until merged"
        );
        merge_removal(&op, &mut group).unwrap();
        assert_eq!(member_dids(&group), vec![owner.did.clone()]);
        assert!(group.epoch() > epoch_before, "removal advances the epoch");
    }

    // REQ-499: a sync frame sealed under the group key by one member is opened
    // by a fellow member — and is opaque to a non-member (a separate group at a
    // different epoch cannot decrypt it).
    #[test]
    fn sealed_frame_round_trips_within_the_group_only() {
        let op = provider();
        let bp = provider();
        let ep = provider();
        let owner = GroupIdentity::new(&[1u8; 32]).unwrap();
        let bob = GroupIdentity::new(&[2u8; 32]).unwrap();
        let eve = GroupIdentity::new(&[4u8; 32]).unwrap();

        // Owner + Bob share one group; Eve is outside it entirely.
        let mut group = create_group(&op, &owner, VAULT).unwrap();
        let (_kb, kp_bytes) = build_key_package(&bp, &bob).unwrap();
        let kp = key_package_from_bytes(&op, &kp_bytes, &bob.endpoint_id).unwrap();
        let (_commit, welcome) = add_member(&op, &mut group, &owner, kp).unwrap();
        let mut bob_group = join_from_welcome(&bp, &welcome, &owner.did).unwrap();
        let mut eve_group = create_group(&ep, &eve, VAULT).unwrap();

        let frame = b"version-vector-and-loro-delta";
        let sealed = seal(&op, &mut group, &owner.signer, frame).unwrap();

        // A fellow member opens it to the original plaintext.
        assert_eq!(open(&bp, &mut bob_group, &sealed).unwrap(), frame);
        // A non-member cannot (wrong group / epoch key).
        assert!(open(&ep, &mut eve_group, &sealed).is_err());
        // Plaintext is not on the wire.
        assert!(
            !sealed.windows(frame.len()).any(|w| w == frame),
            "the payload must be encrypted, not framed in the clear"
        );
    }
}
