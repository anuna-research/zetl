//! iroh QUIC transport for the peer sync session (SPEC-047 REQ-483, IMPL-047
//! T12, DESIGN-047 `adr-transport`).
//!
//! **UNREVIEWED** — see the banner in [`super`]. This is the network wiring
//! that carries the roster-gated, MLS-sealed sync frames; it is auth-core
//! surface and MUST NOT be trusted or merged before the human crypto review.
//!
//! Reuse over reinvention (Simplicity Ladder rung 4): the whole transport is
//! [`iroh`] 1.0 — a QUIC endpoint keyed by an Ed25519 [[Endpoint Id]], with
//! authenticated peer identity (the remote's endpoint id is proven by the QUIC
//! handshake, REQ-491) and hole-punching / relay fallback handled upstream. We
//! add only: our ALPN, a dial helper, an accept helper that surfaces the
//! authenticated peer id for the roster gate ([`session::roster_admits`]), and
//! a [`Duplex`] adapter so a bi-stream drives the transport-agnostic
//! [`session`] core unchanged.
//!
//! **The endpoint id is a transport fact, not authorisation.** `accept` returns
//! the peer's authenticated id; the caller MUST resolve it to a DID and pass it
//! through [`session::roster_admits`] before syncing (REQ-482/492). This module
//! deliberately does not gate — it surfaces the identity and leaves the
//! decision to the roster, so a resolved/authenticated address is never
//! mistaken for membership.

use crate::crdt::reconcile::Syncable;
use crate::p2p::session;
use anyhow::Result;
use iroh::endpoint::{presets, Connection, RecvStream, SendStream};
use iroh::{Endpoint, EndpointAddr, SecretKey};
use tokio::io::Join;

/// Application-layer protocol id (REQ-483). Versioned so a future frame-format
/// change is a new ALPN, not a silent renegotiation.
pub const ALPN: &[u8] = b"zetl/sync/1";

/// One QUIC bi-stream presented as a single `AsyncRead + AsyncWrite`, so the
/// [`session`](crate::p2p::session) core runs over it exactly as over the
/// in-memory duplex it is tested against. Built transiently from the stream
/// halves in [`Peer::sync_note`].
pub type Duplex<'a> = Join<&'a mut RecvStream, &'a mut SendStream>;

/// A live connection to one peer: its authenticated endpoint id and the sync
/// bi-stream halves. Prefer [`Peer::finish`] over a plain drop — it does a
/// graceful QUIC stream FIN + drain so the last frame is delivered and
/// acknowledged before teardown (an eager connection close would abort
/// in-flight stream data the peer had not yet consumed).
pub struct Peer {
    /// The peer's endpoint id, authenticated by the QUIC handshake (REQ-491).
    /// A transport fact — feed it through the roster gate before trusting it.
    pub endpoint_id: [u8; 32],
    send: SendStream,
    recv: RecvStream,
    conn: Connection,
}

impl Peer {
    /// Sync one replicated document with this peer over the bi-stream, driving
    /// the transport-agnostic [`session::sync_one`] core (REQ-486).
    pub async fn sync_note<T: Syncable>(&mut self, doc: &T) -> Result<()> {
        let mut duplex = tokio::io::join(&mut self.recv, &mut self.send);
        session::sync_one(&mut duplex, doc).await
    }

    /// Gracefully close: FIN our send half (ordered *after* every frame we
    /// wrote, so the peer receives them all), drain the recv half to the peer's
    /// FIN (so we have everything they sent), then close the connection. This
    /// avoids the teardown race where `Connection::close` aborts stream data the
    /// peer has not yet consumed.
    pub async fn finish(mut self) {
        let _ = self.send.finish();
        // Drain until the peer's FIN (`Ok(None)`) or an error — quinn's inherent
        // `read` yields `Ok(None)` at end-of-stream.
        let mut sink = [0u8; 256];
        while matches!(self.recv.read(&mut sink).await, Ok(Some(_))) {}
        self.conn.close(0u32.into(), b"sync-complete");
    }
}

/// A bound iroh endpoint that dials and accepts sync peers.
pub struct SyncTransport {
    endpoint: Endpoint,
}

impl SyncTransport {
    /// Bind a production endpoint: n0 defaults (DNS discovery + relay fallback),
    /// so peers reachable only by [[Endpoint Id]] still connect.
    pub async fn bind(secret: SecretKey) -> Result<Self> {
        Self::bind_preset(secret, presets::N0).await
    }

    async fn bind_preset(secret: SecretKey, preset: impl presets::Preset) -> Result<Self> {
        let endpoint = Endpoint::builder(preset)
            .secret_key(secret)
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await
            .map_err(|e| anyhow::anyhow!("iroh endpoint bind: {e}"))?;
        Ok(Self { endpoint })
    }

    /// This endpoint's own id (its stable network identity).
    pub fn endpoint_id(&self) -> [u8; 32] {
        *self.endpoint.id().as_bytes()
    }

    /// This endpoint's dialable address (id + direct addrs + relay).
    pub fn addr(&self) -> EndpointAddr {
        self.endpoint.addr()
    }

    /// Locally bound UDP sockets — used to build a direct [`EndpointAddr`] for
    /// loopback (tests) without discovery/relay.
    pub fn bound_sockets(&self) -> Vec<std::net::SocketAddr> {
        self.endpoint.bound_sockets()
    }

    /// Dial a peer and open the sync bi-stream.
    pub async fn connect(&self, peer: impl Into<EndpointAddr>) -> Result<Peer> {
        let conn = self
            .endpoint
            .connect(peer, ALPN)
            .await
            .map_err(|e| anyhow::anyhow!("iroh connect: {e}"))?;
        let endpoint_id = *conn.remote_id().as_bytes();
        let (send, recv) = conn
            .open_bi()
            .await
            .map_err(|e| anyhow::anyhow!("open_bi: {e}"))?;
        Ok(Peer { endpoint_id, send, recv, conn })
    }

    /// Accept one inbound peer: complete the handshake, surface the
    /// authenticated endpoint id (for the roster gate), and open the sync
    /// bi-stream. Returns `None` when the endpoint is closed.
    pub async fn accept(&self) -> Result<Option<Peer>> {
        let Some(incoming) = self.endpoint.accept().await else {
            return Ok(None);
        };
        let conn = incoming
            .await
            .map_err(|e| anyhow::anyhow!("accept handshake: {e}"))?;
        let endpoint_id = *conn.remote_id().as_bytes();
        let (send, recv) = conn
            .accept_bi()
            .await
            .map_err(|e| anyhow::anyhow!("accept_bi: {e}"))?;
        Ok(Some(Peer { endpoint_id, send, recv, conn }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crdt::loro_store::NoteDoc;

    // A hermetic loopback endpoint: Minimal preset (no relay, no DNS), reachable
    // only over its bound 127.0.0.1 socket — so the test never touches the
    // network.
    async fn local_endpoint() -> SyncTransport {
        SyncTransport::bind_preset(SecretKey::generate(), presets::Minimal)
            .await
            .unwrap()
    }

    fn direct_addr(t: &SyncTransport) -> EndpointAddr {
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};
        let id = iroh::EndpointId::from_bytes(&t.endpoint_id()).unwrap();
        // bound_sockets() reports the wildcard bind (0.0.0.0:port), which is not
        // dialable — rewrite each to loopback so the client reaches the server.
        let loopback = t.bound_sockets().into_iter().map(|s| {
            iroh::TransportAddr::Ip(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), s.port()))
        });
        EndpointAddr::from_parts(id, loopback)
    }

    // REQ-483/486: the session core converges over a REAL iroh QUIC bi-stream
    // (not just the in-memory duplex), end-to-end over loopback.
    #[tokio::test]
    async fn sync_converges_over_iroh_quic() {
        let server = local_endpoint().await;
        let client = local_endpoint().await;
        let server_addr = direct_addr(&server);
        let server_id = server.endpoint_id();

        // Server side: accept one peer, sync its note.
        let server_task = tokio::spawn(async move {
            let mut peer = server.accept().await.unwrap().expect("a peer");
            let mut doc = NoteDoc::new();
            doc.set_content("base").unwrap();
            doc.insert(4, " SERVER").unwrap();
            peer.sync_note(&doc).await.unwrap();
            peer.finish().await;
            doc.materialise()
        });

        // Client side: dial, sync a divergent note.
        let mut peer = client.connect(server_addr).await.unwrap();
        // The peer id iroh authenticated must be the server's real id (REQ-491).
        assert_eq!(peer.endpoint_id, server_id);
        let mut doc = NoteDoc::new();
        doc.set_content("base").unwrap();
        doc.insert(0, "CLIENT ").unwrap();
        peer.sync_note(&doc).await.unwrap();
        peer.finish().await;

        let server_doc = server_task.await.unwrap();
        assert_eq!(doc.materialise(), server_doc, "converged over QUIC");
    }
}
