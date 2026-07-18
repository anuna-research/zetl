//! Roster-gated peer sync session (SPEC-047 REQ-482/486/492, IMPL-047 T12).
//!
//! **UNREVIEWED** — see the banner in [`super`]. Runs only after the roster
//! gate admits the peer.
//!
//! The transport-agnostic core of the peer session: sync a vault's replicated
//! documents over a bidirectional stream by exchanging [[Loro]] op deltas keyed
//! on op-log [[Version Vector]]s (REQ-486). It runs over any
//! `AsyncRead + AsyncWrite` — tested here over an in-memory duplex, and (once
//! the iroh glue lands) over an iroh QUIC bi-stream. The exchange is
//! **symmetric** (both peers run the same code), converging quiescent peers in
//! one round.
//!
//! The [`roster_admits`] gate (REQ-482/492) is applied by the caller *before*
//! any vault frame: a peer whose transport key is not admitted by the vault's
//! roster (T10) is refused before this session runs — never trusting a
//! resolved address as authorisation.
//!
//! Deferred (later T12 slices): sealing each frame under the current MLS
//! [[Group Key]] epoch (REQ-499 — composes T9), and the [[Merkle DAG]] coarse
//! filter that skips exchange for unchanged docs (ADR-478).

use crate::crdt::loro_store::LoroStore;
use crate::crdt::manifest::Manifest;
use crate::crdt::reconcile::Syncable;
use crate::p2p::roster::Roster;
use anyhow::{Context, Result};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Hard per-frame cap (REQ-501 spirit): reject an over-limit length before
/// allocating. Sync deltas are bounded by document size; 64 MiB is generous.
const MAX_FRAME: u32 = 64 << 20;

/// The roster gate (REQ-482/492): is `peer_did` admitted to this vault, so the
/// session may proceed? A resolved address confers nothing; only roster
/// membership does. (The caller resolves the peer's transport key to its DID
/// via the member facts; this predicate is the admission decision.)
pub fn roster_admits(roster: &Roster, peer_did: &str) -> bool {
    roster.contains(peer_did)
}

/// Confidentiality layer for sync frames (REQ-499). Wraps each frame before it
/// hits the wire and unwraps each inbound frame, keeping the sync protocol
/// itself independent of *how* frames are protected. The production sealer is
/// the MLS group key ([`crate::p2p::group::GroupSealer`]); [`Plain`] is the
/// unsealed path used for the transport-agnostic tests and for the QUIC-only
/// (handshake-authenticated but not group-sealed) mode.
pub trait FrameSeal {
    fn seal(&mut self, plaintext: &[u8]) -> Result<Vec<u8>>;
    fn open(&mut self, sealed: &[u8]) -> Result<Vec<u8>>;
}

/// The identity sealer: frames go on the wire as-is. Used where the transport
/// already provides confidentiality, or in tests.
pub struct Plain;

impl FrameSeal for Plain {
    fn seal(&mut self, plaintext: &[u8]) -> Result<Vec<u8>> {
        Ok(plaintext.to_vec())
    }
    fn open(&mut self, sealed: &[u8]) -> Result<Vec<u8>> {
        Ok(sealed.to_vec())
    }
}

/// Symmetrically sync one replicated document over `stream`: exchange version
/// vectors, then the deltas each side lacks (REQ-486). Both peers run this;
/// after it returns, quiescent peers hold identical op state. Frames go on the
/// wire unsealed — see [`sync_one_sealed`] for the group-keyed path (REQ-499).
pub async fn sync_one<S, T>(stream: &mut S, doc: &T) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
    T: Syncable,
{
    sync_one_sealed(stream, doc, &mut Plain).await
}

/// As [`sync_one`], but every frame is sealed under `sealer` before it is sent
/// and opened on receipt (REQ-499). With [`crate::p2p::group::GroupSealer`]
/// this binds the exchange to MLS group membership at the current epoch, so a
/// removed member — past the rotation — can neither read nor forge sync frames.
pub async fn sync_one_sealed<S, T, K>(stream: &mut S, doc: &T, sealer: &mut K) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
    T: Syncable,
    K: FrameSeal,
{
    // 1. Exchange op-log version vectors.
    let my_vv = doc.sync_vv().encode();
    write_frame(stream, &sealer.seal(&my_vv)?).await?;
    let peer_vv_bytes = sealer.open(&read_frame(stream).await?)?;
    let peer_vv = loro::VersionVector::decode(&peer_vv_bytes)
        .map_err(|e| anyhow::anyhow!("decode peer version vector: {e:?}"))?;

    // 2. Export exactly what the peer lacks; import what they send us.
    let my_delta = doc.sync_export(&peer_vv)?;
    write_frame(stream, &sealer.seal(&my_delta)?).await?;
    let peer_delta = sealer.open(&read_frame(stream).await?)?;
    doc.sync_import(&peer_delta)?;
    Ok(())
}

/// Sync a whole vault over `stream`: converge the [`Manifest`] first (so both
/// peers agree which notes exist), then sync every note the merged manifest
/// names, in deterministic order. Persists merged notes. Symmetric — both
/// peers run it and converge (REQ-486 at vault scope).
pub async fn sync_vault<S>(
    stream: &mut S,
    store: &LoroStore,
    manifest: &Manifest,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // Manifest first — both peers then derive the same doc set.
    sync_one(stream, manifest).await.context("sync manifest")?;

    let docs: Vec<_> = manifest.resolve().into_keys().collect();
    for id in docs {
        let note = store.load_or_create(&id)?;
        sync_one(stream, &note).await.with_context(|| format!("sync note {id}"))?;
        store.persist(&id, &note)?;
    }
    Ok(())
}

async fn write_frame<S: AsyncWrite + Unpin>(stream: &mut S, payload: &[u8]) -> Result<()> {
    let len = u32::try_from(payload.len()).context("sync frame too large")?;
    anyhow::ensure!(len <= MAX_FRAME, "sync frame {len} exceeds cap");
    stream.write_all(&len.to_be_bytes()).await?;
    stream.write_all(payload).await?;
    stream.flush().await?;
    Ok(())
}

async fn read_frame<S: AsyncRead + Unpin>(stream: &mut S) -> Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf);
    anyhow::ensure!(len <= MAX_FRAME, "sync frame length {len} exceeds cap");
    let mut buf = vec![0u8; len as usize];
    stream.read_exact(&mut buf).await?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crdt::loro_store::{DocId, NoteDoc};

    // REQ-486: two peers with divergent notes converge over a stream.
    #[tokio::test]
    async fn sync_one_converges_over_a_stream() {
        let mut a = NoteDoc::new();
        a.set_content("base").unwrap();
        let mut b = NoteDoc::from_snapshot(&a.snapshot().unwrap()).unwrap();
        a.insert(4, " A").unwrap();
        b.insert(0, "B ").unwrap();

        let (mut sa, mut sb) = tokio::io::duplex(1 << 16);
        let (ra, rb) = tokio::join!(sync_one(&mut sa, &a), sync_one(&mut sb, &b));
        ra.unwrap();
        rb.unwrap();
        assert_eq!(a.materialise(), b.materialise(), "converged");
        assert_eq!(a.oplog_vv(), b.oplog_vv());
    }

    // REQ-499: a sync between two MLS group members converges even though every
    // frame is sealed under the group key (and the plaintext delta never
    // appears on the wire).
    #[tokio::test]
    async fn sealed_sync_converges_between_group_members() {
        use crate::p2p::group::{self, GroupIdentity, GroupSealer};

        let op = group::provider();
        let bp = group::provider();
        let owner = GroupIdentity::new("did:crdt:owner", &[1u8; 32]).unwrap();
        let bob = GroupIdentity::new("did:crdt:bob", &[2u8; 32]).unwrap();
        let mut ogroup = group::create_group(&op, &owner, b"vault").unwrap();
        let (_kb, kp) = group::build_key_package(&bp, &bob).unwrap();
        let kp = group::key_package_from_bytes(&op, &kp, &bob.did, &bob.endpoint_id).unwrap();
        let (_c, welcome) = group::add_member(&op, &mut ogroup, &owner, kp).unwrap();
        let mut bgroup = group::join_from_welcome(&bp, &welcome, &owner.did).unwrap();

        let mut a = NoteDoc::new();
        a.set_content("base").unwrap();
        let mut b = NoteDoc::from_snapshot(&a.snapshot().unwrap()).unwrap();
        a.insert(4, " A").unwrap();
        b.insert(0, "B ").unwrap();

        let (mut sa, mut sb) = tokio::io::duplex(1 << 16);
        let mut owner_sealer = GroupSealer { provider: &op, group: &mut ogroup, signer: &owner.signer };
        let mut bob_sealer = GroupSealer { provider: &bp, group: &mut bgroup, signer: &bob.signer };
        let (ra, rb) = tokio::join!(
            sync_one_sealed(&mut sa, &a, &mut owner_sealer),
            sync_one_sealed(&mut sb, &b, &mut bob_sealer),
        );
        ra.unwrap();
        rb.unwrap();
        assert_eq!(a.materialise(), b.materialise(), "converged under the group key");
    }

    // REQ-486 at vault scope: two peers with divergent manifests + notes
    // converge over a stream (manifest first, then notes).
    #[tokio::test]
    async fn sync_vault_converges_over_a_stream() {
        let ta = tempfile::tempdir().unwrap();
        let tb = tempfile::tempdir().unwrap();
        let (sa, sb) = (LoroStore::open(ta.path()), LoroStore::open(tb.path()));
        let (ma, mb) = (Manifest::new(), Manifest::new());

        let n1 = DocId::parse("n1").unwrap();
        let n2 = DocId::parse("n2").unwrap();
        ma.create(&n1, "a.md").unwrap();
        let mut d1 = NoteDoc::new();
        d1.set_content("A note").unwrap();
        sa.persist(&n1, &d1).unwrap();
        mb.create(&n2, "b.md").unwrap();
        let mut d2 = NoteDoc::new();
        d2.set_content("B note").unwrap();
        sb.persist(&n2, &d2).unwrap();

        let (mut da, mut db) = tokio::io::duplex(1 << 16);
        let (ra, rb) = tokio::join!(sync_vault(&mut da, &sa, &ma), sync_vault(&mut db, &sb, &mb));
        ra.unwrap();
        rb.unwrap();

        assert_eq!(ma.resolve(), mb.resolve(), "manifests converge");
        assert_eq!(ma.resolve().len(), 2);
        for id in [&n1, &n2] {
            assert_eq!(
                sa.load_or_create(id).unwrap().materialise(),
                sb.load_or_create(id).unwrap().materialise(),
            );
        }
    }
}
