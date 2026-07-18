//! SPAKE2 pairing ceremony (SPEC-047 CON-474, REQ-476/479/491, IMPL-047 T8).
//!
//! **UNREVIEWED CRYPTO** — see the banner in [`super`]. Pending Q1/Q2/Q7/Q11.
//!
//! The channel-authentication core of pairing, transport-agnostic (runs over
//! any `AsyncRead + AsyncWrite`, so it is tested over an in-memory duplex and
//! will run over an iroh QUIC bi-stream once that transport lands). It reuses
//! the audited [`crate::cap::pair`] symmetric SPAKE2 + HMAC key-confirmation
//! primitive (SPEC-034): both peers derive a shared key from the spoken
//! phrase, then **mutually authenticate each other's transport public key**
//! (MITM protection, REQ-491) before any vault data flows.
//!
//! Deferred to later M2 slices (heavy deps / gated): the routing/secret phrase
//! split + DHT rendezvous (ADR-473, needs pkarr), sealing the peer into the
//! MLS [[Group Key]] (ADR-482, needs openmls), and the iroh transport. What
//! lands here is the SPAKE2 handshake that authenticates the channel and binds
//! the peer's transport identity.
//!
//! Failure is **opaque** on the wire (REQ-479): every distinct cause (wrong
//! phrase, tampered tag, protocol error) surfaces to the peer as a dropped
//! ceremony; only the local caller learns the specific [`PairError`].

use crate::cap::pair::{PairError, PairMessage, PairPhrase, PairSession};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Maximum pairing frame (SPAKE2 messages, pubkeys, tags are all tiny). Bounds
/// recognition before allocation (REQ-501 spirit) on this pre-auth channel.
const MAX_FRAME: u32 = 4096;

/// Outcome of a failed or completed ceremony. On the wire a failure is a
/// dropped connection; locally the caller sees the cause.
#[derive(Debug)]
pub enum CeremonyError {
    /// I/O on the pairing stream failed (peer dropped, timeout, framing).
    Io(std::io::Error),
    /// A frame exceeded the pre-auth bound.
    Oversized(u32),
    /// The SPAKE2 / key-confirmation step failed — a wrong phrase, a MITM, or
    /// a tampered message. This is the single opaque `auth-failed` cause.
    Auth(PairError),
}

impl std::fmt::Display for CeremonyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CeremonyError::Io(e) => write!(f, "pairing io: {e}"),
            CeremonyError::Oversized(n) => write!(f, "pairing frame too large ({n})"),
            // Never distinguish the auth cause in a peer-visible message.
            CeremonyError::Auth(_) => write!(f, "auth-failed"),
        }
    }
}

impl From<std::io::Error> for CeremonyError {
    fn from(e: std::io::Error) -> Self {
        CeremonyError::Io(e)
    }
}
impl From<PairError> for CeremonyError {
    fn from(e: PairError) -> Self {
        CeremonyError::Auth(e)
    }
}

/// Run the symmetric pairing ceremony over `stream`, authenticating our
/// transport public key and confirming the peer's. On success returns the
/// peer's authenticated 32-byte transport public key — the identity that a
/// later slice admits to the roster. Both peers run this same code.
///
/// Steps: (1) exchange SPAKE2 messages and derive the shared key from the
/// phrase; (2) send our pubkey + its HMAC tag, receive and verify the peer's.
/// Any failure aborts with [`CeremonyError`]; the peer only sees the drop.
pub async fn pair<S>(
    stream: &mut S,
    phrase: &PairPhrase,
    our_pubkey: &[u8; 32],
) -> Result<[u8; 32], CeremonyError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // 1. SPAKE2 exchange (symmetric — no role negotiation).
    let (session, our_msg) = PairSession::start(phrase)?;
    write_frame(stream, &our_msg.0).await?;
    let peer_msg = PairMessage(read_frame(stream).await?);
    let key = session.finish(&peer_msg)?;

    // 2. Mutual transport-pubkey confirmation (REQ-491: binds the channel to
    //    the peer's durable identity, defeating a relay MITM). Send ours first
    //    (both sides do; ordering is symmetric over the duplex/QUIC stream).
    let our_tag = key.authenticate_pubkey(our_pubkey)?;
    write_frame(stream, our_pubkey).await?;
    write_frame(stream, &our_tag).await?;

    let peer_pubkey = read_frame(stream).await?;
    let peer_tag = read_frame(stream).await?;
    key.verify_pubkey(&peer_pubkey, &peer_tag)?;

    let mut out = [0u8; 32];
    if peer_pubkey.len() != 32 {
        return Err(CeremonyError::Auth(PairError::PubkeyLen {
            got: peer_pubkey.len(),
        }));
    }
    out.copy_from_slice(&peer_pubkey);
    Ok(out)
}

async fn write_frame<S: AsyncWrite + Unpin>(stream: &mut S, payload: &[u8]) -> std::io::Result<()> {
    let len = u32::try_from(payload.len())
        .map_err(|_| std::io::Error::other("pairing frame too large"))?;
    stream.write_all(&len.to_be_bytes()).await?;
    stream.write_all(payload).await?;
    stream.flush().await?;
    Ok(())
}

async fn read_frame<S: AsyncRead + Unpin>(stream: &mut S) -> Result<Vec<u8>, CeremonyError> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_FRAME {
        return Err(CeremonyError::Oversized(len));
    }
    let mut buf = vec![0u8; len as usize];
    stream.read_exact(&mut buf).await?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cap::pair::generate_phrase;
    use rand_core::OsRng;

    // Run both symmetric halves of the ceremony over an in-memory duplex.
    async fn run(
        phrase_a: &PairPhrase,
        pk_a: &[u8; 32],
        phrase_b: &PairPhrase,
        pk_b: &[u8; 32],
    ) -> (
        Result<[u8; 32], CeremonyError>,
        Result<[u8; 32], CeremonyError>,
    ) {
        let (mut a, mut b) = tokio::io::duplex(8192);
        let ra = pair(&mut a, phrase_a, pk_a);
        let rb = pair(&mut b, phrase_b, pk_b);
        tokio::join!(ra, rb)
    }

    // HP1: same phrase → both peers succeed and each learns the other's pubkey.
    #[tokio::test]
    async fn matching_phrase_authenticates_both() {
        let phrase = generate_phrase(&mut OsRng);
        let (pk_a, pk_b) = ([1u8; 32], [2u8; 32]);
        let (ra, rb) = run(&phrase, &pk_a, &phrase, &pk_b).await;
        assert_eq!(ra.unwrap(), pk_b, "A learns B's pubkey");
        assert_eq!(rb.unwrap(), pk_a, "B learns A's pubkey");
    }

    // REQ-479: a wrong phrase fails — opaquely (both sides see `auth-failed`).
    #[tokio::test]
    async fn wrong_phrase_fails_opaquely() {
        let a = generate_phrase(&mut OsRng);
        let mut b = generate_phrase(&mut OsRng);
        while b.canonical() == a.canonical() {
            b = generate_phrase(&mut OsRng);
        }
        let (ra, rb) = run(&a, &[1u8; 32], &b, &[2u8; 32]).await;
        assert!(ra.is_err() && rb.is_err(), "both fail on phrase mismatch");
        assert_eq!(ra.unwrap_err().to_string(), "auth-failed");
        assert_eq!(rb.unwrap_err().to_string(), "auth-failed");
    }

    // REQ-491: key confirmation binds the pubkeys — a peer that presents a
    // pubkey it did not authenticate is rejected. We simulate a MITM by having
    // one side authenticate a *different* pubkey than it sends: hand-run the
    // frames with a tampered pubkey.
    #[tokio::test]
    async fn tampered_pubkey_is_rejected() {
        let phrase = generate_phrase(&mut OsRng);
        let (mut a, mut b) = tokio::io::duplex(8192);

        // Honest A.
        let honest = pair(&mut a, &phrase, &[1u8; 32]);

        // Malicious B: complete SPAKE2, then send a pubkey whose tag does not
        // match (tag over a different key) — a relay that swaps the identity.
        let attacker = async {
            let (session, our_msg) = PairSession::start(&phrase).unwrap();
            write_frame(&mut b, &our_msg.0).await.unwrap();
            let peer_msg = PairMessage(read_frame(&mut b).await.unwrap());
            let key = session.finish(&peer_msg).unwrap();
            // Authenticate one key but send a different one (swap attack).
            let tag = key.authenticate_pubkey(&[9u8; 32]).unwrap();
            write_frame(&mut b, &[7u8; 32]).await.unwrap(); // sent pubkey ≠ tagged
            write_frame(&mut b, &tag).await.unwrap();
            // Drain A's confirmation so it doesn't block.
            let _ = read_frame(&mut b).await;
            let _ = read_frame(&mut b).await;
        };

        let (res, _) = tokio::join!(honest, attacker);
        assert!(res.is_err(), "honest peer rejects the swapped pubkey");
        assert_eq!(res.unwrap_err().to_string(), "auth-failed");
    }
}
