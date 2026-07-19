//! The `zetld` serve loop (SPEC-047 REQ-470/490). Effectful shell: binds the
//! per-vault Unix control socket, writes the discovery record, and serves
//! framed control requests until a `Stop` arrives. Client disconnects never
//! stop the daemon (REQ-490) — the accept loop simply continues.

use super::{
    record_path, socket_path, ControlRequest, ControlResponse, DaemonRecord, DaemonStatus,
    CONTROL_API_VERSION,
};
use crate::crdt::loro_store::LoroStore;
use crate::crdt::manifest::Manifest;
use crate::crdt::vault_fs;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// The vault state a running daemon owns (REQ-470): the canonical per-note
/// Loro store and the namespace manifest. Shared behind an `Arc<Mutex<..>>` so
/// the control loop and the P2P sync service ([`super::p2p`]) serialise access
/// and never mutate the store concurrently.
pub(crate) struct VaultState {
    pub(crate) store: LoroStore,
    pub(crate) manifest: Manifest,
}

impl VaultState {
    /// Load the vault's manifest + store, bootstrapping from existing Markdown
    /// on first run (an empty manifest means the store has never been built).
    pub(crate) fn open(vault_root: &Path) -> Result<VaultState> {
        let manifest = Manifest::load(vault_root)?;
        let store = LoroStore::open(vault_root);
        if manifest.resolve().is_empty() {
            vault_fs::import_vault(vault_root, &store, &manifest)?;
        }
        Ok(VaultState { store, manifest })
    }

    /// Construct directly from parts (tests / provisioning).
    pub(crate) fn from_parts(store: LoroStore, manifest: Manifest) -> VaultState {
        VaultState { store, manifest }
    }

    pub(crate) fn note_count(&self) -> u32 {
        self.manifest.resolve().len() as u32
    }
}
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Mutex, Notify};
use std::sync::Arc;

/// Hard cap on a single control frame (REQ-501 spirit at the local boundary):
/// recognise the length prefix before allocating, reject anything larger.
const MAX_FRAME: u32 = 1 << 20; // 1 MiB

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Run the daemon serve loop for `vault_root` until `Stop`. Writes the socket
/// and record on entry and removes them on clean exit. Blocks (async) for the
/// daemon's lifetime.
pub async fn run(vault_root: &Path) -> Result<()> {
    let sock = socket_path(vault_root);
    let rec = record_path(vault_root);
    // The socket lives in a short, secured runtime dir (SUN_LEN); the record is
    // vault-local. Create/verify both.
    super::ensure_socket_dir().context("prepare socket runtime dir")?;
    if let Some(rec_dir) = rec.parent() {
        std::fs::create_dir_all(rec_dir)
            .with_context(|| format!("create vault runtime dir {}", rec_dir.display()))?;
    }

    // A leftover socket from a crashed daemon would block bind; the caller
    // (`lifecycle::start`) has already classified staleness and cleaned, but
    // remove defensively so a bind race fails closed rather than aliasing.
    let _ = std::fs::remove_file(&sock);
    let listener =
        UnixListener::bind(&sock).with_context(|| format!("bind control socket {}", sock.display()))?;
    set_0600(&sock)?;

    let started_at = now_unix();
    let record = DaemonRecord {
        api_version: CONTROL_API_VERSION,
        pid: std::process::id() as i32,
        socket: sock.clone(),
        started_at,
        version: env!("CARGO_PKG_VERSION").to_string(),
    };
    write_record_0600(&rec, &record)?;

    let shutdown = Arc::new(Notify::new());
    let vault_label = vault_root
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| vault_root.display().to_string());

    // The daemon becomes the single owner of the vault's canonical state
    // (REQ-470), bootstrapping from Markdown on first run. Shared behind a Mutex
    // with the P2P service so vault mutations serialise.
    let vault = Arc::new(Mutex::new(VaultState::open(vault_root)?));

    // Start the P2P sync service iff the vault has been provisioned for it
    // (opt-in keyfile). UNREVIEWED auth-core — see super::p2p. A local-only
    // vault runs exactly as before.
    let p2p_task = match super::p2p::P2pService::open(vault_root).await {
        Ok(Some(service)) => {
            let vault = vault.clone();
            Some(tokio::spawn(async move { let _ = service.serve(vault).await; }))
        }
        Ok(None) => None,
        Err(e) => {
            // Provisioned but unstartable → refuse P2P, keep serving locally.
            eprintln!("zetld: P2P service disabled: {e:#}");
            None
        }
    };

    let result = accept_loop(
        &listener,
        started_at,
        &vault_label,
        vault_root,
        vault.clone(),
        shutdown.clone(),
    )
    .await;

    // Clean shutdown: stop the P2P listener, drop the control listener, remove
    // socket + record so the next `status` reports NotRunning.
    if let Some(task) = p2p_task {
        task.abort();
    }
    drop(listener);
    let _ = std::fs::remove_file(&sock);
    let _ = std::fs::remove_file(&rec);
    result
}

#[allow(clippy::too_many_arguments)]
async fn accept_loop(
    listener: &UnixListener,
    started_at: u64,
    vault_label: &str,
    vault_root: &Path,
    vault: Arc<Mutex<VaultState>>,
    shutdown: Arc<Notify>,
) -> Result<()> {
    loop {
        tokio::select! {
            _ = shutdown.notified() => return Ok(()),
            accepted = listener.accept() => {
                let (stream, _addr) = match accepted {
                    Ok(pair) => pair,
                    // An accept error on one connection must not kill the
                    // daemon (REQ-490); log-and-continue.
                    Err(_) => continue,
                };
                // Handle inline: control traffic is low-volume and strictly
                // ordered per connection. A handler that requests shutdown
                // notifies the loop, which exits on the next select.
                handle_conn(stream, started_at, vault_label, vault_root, &vault, &shutdown).await;
            }
        }
    }
}

async fn handle_conn(
    mut stream: UnixStream,
    started_at: u64,
    vault_label: &str,
    vault_root: &Path,
    vault: &Arc<Mutex<VaultState>>,
    shutdown: &Arc<Notify>,
) {
    let req = match read_frame(&mut stream).await {
        Ok(bytes) => match serde_json::from_slice::<ControlRequest>(&bytes) {
            Ok(req) => req,
            Err(e) => {
                // CON-470 error model: malformed input is fully recognised
                // (rejected) before any action — never partially applied.
                let _ = respond(
                    &mut stream,
                    &ControlResponse::Error {
                        kind: "malformed-request".into(),
                        message: format!("unrecognised control frame: {e}"),
                    },
                )
                .await;
                return;
            }
        },
        Err(_) => return,
    };

    let resp = match req {
        ControlRequest::Ping => ControlResponse::Pong {
            api_version: CONTROL_API_VERSION,
        },
        ControlRequest::Status => {
            let notes = vault.lock().await.note_count();
            ControlResponse::Status(DaemonStatus::Running {
                pid: std::process::id() as i32,
                uptime_secs: now_unix().saturating_sub(started_at),
                vaults: vec![vault_label.to_string()],
                notes,
                peers: 0, // per-peer session accounting is a later T12 slice
            })
        }
        ControlRequest::Materialise => {
            let vault = vault.lock().await;
            match vault_fs::export_vault(vault_root, &vault.manifest, &vault.store) {
                Ok(count) => ControlResponse::Materialised { count: count as u32 },
                Err(e) => ControlResponse::Error {
                    kind: "materialise-failed".into(),
                    message: e.to_string(),
                },
            }
        }
        ControlRequest::Reimport => {
            let vault = vault.lock().await;
            match vault_fs::reimport_vault(vault_root, &vault.store, &vault.manifest) {
                Ok((folded, staged)) => ControlResponse::Reimported {
                    folded: folded as u32,
                    staged: staged as u32,
                },
                Err(e) => ControlResponse::Error {
                    kind: "reimport-failed".into(),
                    message: e.to_string(),
                },
            }
        }
        ControlRequest::Stop => {
            let _ = respond(&mut stream, &ControlResponse::Ok).await;
            shutdown.notify_one();
            return;
        }
    };
    let _ = respond(&mut stream, &resp).await;
}

async fn respond(stream: &mut UnixStream, resp: &ControlResponse) -> Result<()> {
    let bytes = serde_json::to_vec(resp).context("serialise control response")?;
    write_frame(stream, &bytes).await
}

/// Read one `u32-be length ‖ payload` frame, rejecting an over-limit length at
/// the prefix before allocating (bounded recognition).
pub(crate) async fn read_frame(stream: &mut UnixStream) -> Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_FRAME {
        anyhow::bail!("control frame length {len} exceeds {MAX_FRAME}");
    }
    let mut buf = vec![0u8; len as usize];
    stream.read_exact(&mut buf).await?;
    Ok(buf)
}

/// Write one length-prefixed frame.
pub(crate) async fn write_frame(stream: &mut UnixStream, payload: &[u8]) -> Result<()> {
    let len = u32::try_from(payload.len()).context("frame too large")?;
    stream.write_all(&len.to_be_bytes()).await?;
    stream.write_all(payload).await?;
    stream.flush().await?;
    Ok(())
}

fn write_record_0600(path: &Path, record: &DaemonRecord) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(record).context("serialise daemon record")?;
    write_0600(path, &bytes)
}

fn write_0600(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write as _;
    let tmp = path.with_extension("tmp");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp).with_context(|| format!("open {}", tmp.display()))?;
    f.write_all(bytes)?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn set_0600(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("chmod 0600 {}", path.display()))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Convenience for the CLI: the vault root a foreground daemon should serve.
pub fn resolve_vault_root(dir: &str) -> PathBuf {
    PathBuf::from(dir)
}
