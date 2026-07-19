//! `zetld`'s P2P sync service (SPEC-047 REQ-470/482/483/492, IMPL-047 T12).
//!
//! **UNREVIEWED AUTH-CORE** — see the banner in [`crate::p2p`]. This wires the
//! (unreviewed) transport into the live daemon *on this branch only*; it is
//! gated behind an opt-in keyfile and MUST NOT be trusted, merged, or shipped
//! before the human crypto review. Writing the wiring on-branch is not shipping
//! it — the daemon refuses to start the service unless the operator has
//! explicitly provisioned P2P for the vault.
//!
//! The service binds the vault's iroh QUIC endpoint and runs the roster-gated
//! vault sync ([`crate::p2p::session::sync_vault`]) for every admitted peer.
//! The QUIC handshake authenticates the peer's endpoint id (REQ-491); the
//! *admit set* is the roster gate at the transport boundary (REQ-482/492) — a
//! peer whose endpoint id is not admitted is refused before any vault frame, so
//! a resolved/authenticated address is never mistaken for membership.

use crate::daemon::server::VaultState;
use crate::p2p::transport::{Peer, SyncTransport};
use anyhow::{Context, Result};
use iroh::{EndpointAddr, SecretKey};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

/// The vault state shared between the daemon control loop and this service.
type SharedVault = Arc<Mutex<VaultState>>;

/// Directory holding a vault's P2P provisioning (opt-in): the endpoint secret
/// key and the admit set. Its presence is what enables the service.
pub const P2P_DIR: &str = "p2p";
const KEY_FILE: &str = "endpoint.key";
const ADMIT_FILE: &str = "admitted";

/// The daemon's P2P sync service for one vault.
pub struct P2pService {
    transport: SyncTransport,
    /// Admitted peer endpoint ids — the transport-boundary roster gate
    /// (REQ-482/492).
    //
    // SIMPLIFY (rung 5, ceiling): the admit set is loaded from a provisioned
    // file. The *cryptographic* replacement already exists and is tested —
    // `group::endpoint_owner` resolves a QUIC-authenticated endpoint id to its
    // member DID via the MLS-bound leaf credential (REQ-482/492/497). Wiring it
    // here needs the daemon to hold the live MLS group, which is the durable-MLS
    // -provider slice (elephant REQ-306). Until then this file is the interim
    // shim; the binding it stands in for is no longer provisioned-trust in the
    // library. Traced to ADR-481/482.
    admitted: HashSet<[u8; 32]>,
}

impl P2pService {
    /// Bind the vault's endpoint and load its admit set. Returns `Ok(None)` when
    /// the vault has not been provisioned for P2P (no keyfile) — the daemon then
    /// runs local-only, exactly as before.
    pub async fn open(vault_root: &Path) -> Result<Option<P2pService>> {
        let dir = crate::daemon::runtime_dir(vault_root).join(P2P_DIR);
        let key_path = dir.join(KEY_FILE);
        if !key_path.exists() {
            return Ok(None);
        }
        let secret = read_secret(&key_path)?;
        let admitted = read_admit_set(&dir.join(ADMIT_FILE))?;
        let transport = SyncTransport::bind(secret)
            .await
            .context("bind vault P2P endpoint")?;
        Ok(Some(P2pService {
            transport,
            admitted,
        }))
    }

    /// Build a service from explicit material (tests, and the future
    /// provisioning path).
    pub async fn with(secret: SecretKey, admitted: HashSet<[u8; 32]>) -> Result<P2pService> {
        let transport = SyncTransport::bind(secret).await?;
        Ok(P2pService {
            transport,
            admitted,
        })
    }

    pub fn endpoint_id(&self) -> [u8; 32] {
        self.transport.endpoint_id()
    }

    pub fn addr(&self) -> EndpointAddr {
        self.transport.addr()
    }

    pub fn bound_sockets(&self) -> Vec<std::net::SocketAddr> {
        self.transport.bound_sockets()
    }

    /// The transport-boundary roster gate (REQ-482/492): is this peer admitted?
    fn admits(&self, endpoint_id: &[u8; 32]) -> bool {
        self.admitted.contains(endpoint_id)
    }

    /// Accept peers forever, syncing the vault with each admitted one. The
    /// roster gate runs on the handshake-authenticated endpoint id **before**
    /// any stream operation the peer controls, and each admitted peer is
    /// served on its own task (its stream-open bounded by a timeout) — so a
    /// stranger who never opens a stream, or a stalling peer, cannot block
    /// every legitimate peer behind it. A refused or failed peer never stops
    /// the loop (REQ-490 spirit). Vault access still serialises on the shared
    /// mutex, so the store is never mutated concurrently.
    pub(crate) async fn serve(&self, vault: SharedVault) -> Result<()> {
        /// How long an admitted peer may take to open its sync bi-stream.
        const STREAM_OPEN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
        loop {
            let incoming = match self.transport.accept().await {
                Ok(Some(incoming)) => incoming,
                Ok(None) => return Ok(()), // endpoint closed
                Err(_) => continue,
            };
            if !self.admits(&incoming.endpoint_id) {
                // Fail closed: not on the roster → refuse before any vault
                // frame. An immediate close, not a cooperative drain — a
                // stranger gets nothing to stall.
                incoming.refuse();
                continue;
            }
            let vault = vault.clone();
            tokio::spawn(async move {
                if let Ok(peer) = incoming.into_peer(STREAM_OPEN_TIMEOUT).await {
                    let _ = sync_peer(peer, &vault).await;
                }
            });
        }
    }

    /// Dial an admitted peer and sync the vault once.
    //
    // Not yet reachable in production: the dial side arrives with the M2
    // control verbs (`zetl collab …`). Exercised by the tests below.
    #[allow(dead_code)]
    pub(crate) async fn connect_and_sync(
        &self,
        peer_addr: impl Into<EndpointAddr>,
        vault: &SharedVault,
    ) -> Result<()> {
        let peer = self.transport.connect(peer_addr).await?;
        anyhow::ensure!(
            self.admits(&peer.endpoint_id),
            "peer is not admitted (roster gate)"
        );
        sync_peer(peer, vault).await
    }
}

async fn sync_peer(mut peer: Peer, vault: &SharedVault) -> Result<()> {
    let result = {
        let vs = vault.lock().await;
        // SIMPLIFY (rung 5, ceiling): transport-authenticated + roster-gated
        // sync. REQ-499 group-key frame sealing (session::sync_one_sealed via
        // group::GroupSealer — implemented + tested) composes on top once the
        // daemon holds the durable MLS group. Traced to ADR-482.
        peer.sync_vault(&vs.store, &vs.manifest).await
    };
    peer.finish().await;
    result
}

/// The stable Loro actor for this vault's device, when the vault is
/// provisioned for P2P (the endpoint keyfile doubles as the device identity
/// root — one key, three layers: transport, DID, edit attribution). `None`
/// for an unprovisioned vault.
pub(crate) fn device_actor(vault_root: &Path) -> Option<u64> {
    let key_path = crate::daemon::runtime_dir(vault_root)
        .join(P2P_DIR)
        .join(KEY_FILE);
    let secret = read_secret(&key_path).ok()?;
    Some(crate::p2p::identity::DeviceIdentity::from_secret(secret.to_bytes()).loro_peer())
}

fn read_secret(path: &Path) -> Result<SecretKey> {
    let bytes =
        std::fs::read(path).with_context(|| format!("read endpoint key {}", path.display()))?;
    let arr: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("endpoint key must be exactly 32 bytes"))?;
    Ok(SecretKey::from_bytes(&arr))
}

fn read_admit_set(path: &Path) -> Result<HashSet<[u8; 32]>> {
    let mut set = HashSet::new();
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(set); // no admit file yet → empty (fail closed: admits nobody)
    };
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let raw = hex_decode(line).with_context(|| format!("parse admitted id {line:?}"))?;
        let id: [u8; 32] = raw
            .as_slice()
            .try_into()
            .map_err(|_| anyhow::anyhow!("admitted id must be 32 bytes (64 hex): {line:?}"))?;
        set.insert(id);
    }
    Ok(set)
}

/// Decode hex over *bytes*, never string slices: a malformed admit-file line
/// containing multibyte UTF-8 (e.g. `aéx`) must yield a parse error, not a
/// char-boundary panic that crashes a provisioned daemon at startup.
fn hex_decode(s: &str) -> Result<Vec<u8>> {
    fn nibble(b: u8) -> Result<u8> {
        match b {
            b'0'..=b'9' => Ok(b - b'0'),
            b'a'..=b'f' => Ok(b - b'a' + 10),
            b'A'..=b'F' => Ok(b - b'A' + 10),
            _ => anyhow::bail!("bad hex byte 0x{b:02x}"),
        }
    }
    let bytes = s.as_bytes();
    anyhow::ensure!(bytes.len().is_multiple_of(2), "odd-length hex");
    bytes
        .chunks_exact(2)
        .map(|pair| Ok(nibble(pair[0])? << 4 | nibble(pair[1])?))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crdt::loro_store::{DocId, LoroStore, NoteDoc};
    use crate::crdt::manifest::Manifest;

    fn vault(dir: &Path) -> SharedVault {
        Arc::new(Mutex::new(VaultState::from_parts(
            LoroStore::open(dir),
            Manifest::new(),
        )))
    }

    // Two daemons, mutually admitted, converge a vault over the wired P2P
    // service (REQ-470/483/486): the transport, roster gate, and vault sync run
    // as the daemon would run them.
    #[tokio::test]
    async fn two_admitted_daemons_sync_a_vault() {
        let ta = tempfile::tempdir().unwrap();
        let tb = tempfile::tempdir().unwrap();
        let va = vault(ta.path());
        let vb = vault(tb.path());

        // Seed divergent notes in each vault.
        {
            let g = va.lock().await;
            let n = DocId::parse(&"a".repeat(32)).unwrap();
            g.manifest.create(&n, "a.md").unwrap();
            let mut d = NoteDoc::new();
            d.set_content("from A").unwrap();
            g.store.persist(&n, &d).unwrap();
        }
        {
            let g = vb.lock().await;
            let n = DocId::parse(&"b".repeat(32)).unwrap();
            g.manifest.create(&n, "b.md").unwrap();
            let mut d = NoteDoc::new();
            d.set_content("from B").unwrap();
            g.store.persist(&n, &d).unwrap();
        }

        let ka = SecretKey::generate();
        let kb = SecretKey::generate();
        // Provisionally: to know each other's endpoint id we bind first.
        let server = P2pService::with(kb, HashSet::new()).await.unwrap();
        let server_id = server.endpoint_id();
        let client = P2pService::with(ka, HashSet::from([server_id]))
            .await
            .unwrap();
        let client_id = client.endpoint_id();
        // Server admits the client.
        let server = P2pService::with_admit(server, HashSet::from([client_id]));

        let server_addr = loopback_addr(&server);
        let server_task = {
            let vb = vb.clone();
            tokio::spawn(async move { server.serve(vb).await })
        };

        client.connect_and_sync(server_addr, &va).await.unwrap();
        server_task.abort();

        // Both vaults now name both notes.
        let ga = va.lock().await;
        let gb = vb.lock().await;
        assert_eq!(ga.manifest.resolve().len(), 2, "A sees both notes");
        assert_eq!(gb.manifest.resolve().len(), 2, "B sees both notes");
    }

    // Fail-closed: a peer that is NOT admitted is refused; the vault is untouched.
    #[tokio::test]
    async fn unadmitted_peer_is_refused() {
        let ta = tempfile::tempdir().unwrap();
        let tb = tempfile::tempdir().unwrap();
        let va = vault(ta.path());
        let vb = vault(tb.path());
        {
            let g = vb.lock().await;
            let n = DocId::parse(&"c".repeat(32)).unwrap();
            g.manifest.create(&n, "s.md").unwrap();
            let mut d = NoteDoc::new();
            d.set_content("private").unwrap();
            g.store.persist(&n, &d).unwrap();
        }

        // Server admits nobody; client is a stranger.
        let server = P2pService::with(SecretKey::generate(), HashSet::new())
            .await
            .unwrap();
        let server_id = server.endpoint_id();
        let client = P2pService::with(SecretKey::generate(), HashSet::from([server_id]))
            .await
            .unwrap();
        let server_addr = loopback_addr(&server);

        let server_task = {
            let vb = vb.clone();
            tokio::spawn(async move { server.serve(vb).await })
        };

        // The client dials; the server refuses (closes) so the client's sync
        // errors out. The stranger learns nothing.
        let _ = client.connect_and_sync(server_addr, &va).await;
        server_task.abort();

        let ga = va.lock().await;
        assert_eq!(
            ga.manifest.resolve().len(),
            0,
            "the stranger received no notes"
        );
    }

    // Regression: a malformed admit line with multibyte UTF-8 has even byte
    // length but must parse-error, not panic on a char boundary.
    #[test]
    fn hex_decode_rejects_multibyte_without_panicking() {
        assert!(hex_decode("aéx").is_err(), "even byte length, not hex");
        assert!(hex_decode("aé").is_err(), "odd byte length");
        assert!(hex_decode("zz").is_err());
        assert_eq!(hex_decode("0aff").unwrap(), vec![0x0a, 0xff]);
    }

    // Test helpers ----------------------------------------------------------

    fn loopback_addr(s: &P2pService) -> EndpointAddr {
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};
        let id = iroh::EndpointId::from_bytes(&s.endpoint_id()).unwrap();
        let addrs = s.bound_sockets().into_iter().map(|a| {
            iroh::TransportAddr::Ip(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), a.port()))
        });
        EndpointAddr::from_parts(id, addrs)
    }

    impl P2pService {
        fn with_admit(mut self, admitted: HashSet<[u8; 32]>) -> P2pService {
            self.admitted = admitted;
            self
        }
    }
}
