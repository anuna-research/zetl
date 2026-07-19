//! The persistent `zetld` sync daemon (SPEC-047 REQ-470/471/490, CON-470).
//!
//! `zetld` owns a vault's realtime state and P2P sessions so clients attach
//! over a local control channel rather than writing `.zetl/` directly. This
//! module is IMPL-047 **T2** — the daemon lifecycle: a per-vault Unix-domain
//! control socket, an idempotent start/stop/status lifecycle, and stale-state
//! recovery after a crash. The P2P/auth-core control verbs
//! (`invite`/`join`/`peers`/`revoke`) are IMPL-047 M2 and gated on the
//! DESIGN-047 crypto-review; T2 lands the lifecycle subset
//! (`status`/`stop`/`ping`).
//!
//! Purity boundary: this file is the **pure core** — record/status types and
//! the [`classify`] liveness function, all deterministic and I/O-free. The
//! effectful shell ([`server`], [`client`], [`lifecycle`]) performs the socket
//! and process I/O and calls into here.
//
// SIMPLIFY: control messages are length-prefixed JSON, not CBCL. Ceiling: the
// inter-process trust boundary once the daemon serves untrusted callers;
// upgrade path: swap the framing to the `zetl-sync` CBCL dialect (IMPL-047 T7).
// The local plane is same-user loopback, so JSON is defensible here — the same
// call `../elephant-3000` made (its ADR-106); confirmed or revised by the
// DESIGN-047 `adr-control-proto` task (trace: SPEC-047 ADR-479).

pub mod client;
pub mod lifecycle;
pub mod p2p;
pub mod server;

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Daemon discovery record file name inside `.zetl/`.
pub const RECORD_NAME: &str = "zetld.json";
/// Control-protocol version — bumped when the request/response shape changes.
pub const CONTROL_API_VERSION: u32 = 1;

/// Path to a vault's `.zetl/` runtime directory (record + Loro store + P2P
/// provisioning). Vault-local, no length limit.
pub fn runtime_dir(vault_root: &Path) -> PathBuf {
    vault_root.join(".zetl")
}

/// Short, user-private base for control **sockets**. A Unix socket path must fit
/// `SUN_LEN` (~104 bytes on macOS, 108 on Linux), so the socket cannot live
/// under a deep vault path — a vault nested in the user's home would exceed the
/// limit and the daemon would fail to bind. Mirrors `../hark`: prefer
/// `$XDG_RUNTIME_DIR` (Linux user runtime), else the OS temp dir (`$TMPDIR` on
/// macOS is per-user private), under a `zetl` subdir. The discovery *record*
/// stays in [`runtime_dir`] (vault-local, no length limit) so clients still find
/// the daemon by vault path.
fn socket_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        // $XDG_RUNTIME_DIR is already per-user private.
        Some(dir) => PathBuf::from(dir).join("zetl"),
        // The temp-dir fallback may be shared (`/tmp` on Linux sessions
        // without XDG_RUNTIME_DIR), and `ensure_socket_dir` requires the dir
        // to be owned by the current user — a bare `/tmp/zetl` would let
        // whichever user creates it first lock every other user out. Scope
        // the fallback by UID so each user gets their own dir.
        // Safety: geteuid is always successful and has no preconditions.
        None => std::env::temp_dir().join(format!("zetl-{}", unsafe { libc::geteuid() })),
    }
}

/// Path to a vault's control socket: a short hashed name under [`socket_dir`],
/// keyed by the vault's **canonical** path so each vault gets a stable, distinct
/// socket that fits `SUN_LEN` no matter how deep the vault lives. Server and
/// client both call this, so they always agree without consulting the record.
pub fn socket_path(vault_root: &Path) -> PathBuf {
    let canon = std::fs::canonicalize(vault_root).unwrap_or_else(|_| vault_root.to_path_buf());
    let digest = blake3::hash(canon.to_string_lossy().as_bytes());
    socket_dir().join(format!("z-{}.sock", &digest.to_hex()[..16]))
}

/// Create [`socket_dir`] owner-only (0700) and verify it is a non-symlink
/// directory owned by us with no group/other access — so a hostile pre-existing
/// dir (e.g. under a world-writable `/tmp` fallback) is rejected rather than
/// trusted. Mirrors `../hark`'s `create_dir_owner_only` + `ensure_secure_path`.
///
/// This preserves the CON-470 C1 same-user control boundary (0600 socket in a
/// 0700 owner-verified dir) that the previous vault-local socket relied on the
/// vault's own permissions for — the boundary is the socket's perms, not its
/// location.
pub fn ensure_socket_dir() -> std::io::Result<PathBuf> {
    use std::io::{Error, ErrorKind};
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};

    let dir = socket_dir();
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)?;

    let meta = std::fs::symlink_metadata(&dir)?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(Error::new(
            ErrorKind::PermissionDenied,
            "socket dir is not a real directory",
        ));
    }
    // Safety: geteuid is always successful and has no preconditions.
    if meta.uid() != unsafe { libc::geteuid() } {
        return Err(Error::new(
            ErrorKind::PermissionDenied,
            "socket dir is not owned by the current user",
        ));
    }
    if meta.mode() & 0o077 != 0 {
        return Err(Error::new(
            ErrorKind::PermissionDenied,
            "socket dir is group/other accessible",
        ));
    }
    Ok(dir)
}

/// Path to a vault's daemon discovery record (vault-local, no length limit).
pub fn record_path(vault_root: &Path) -> PathBuf {
    runtime_dir(vault_root).join(RECORD_NAME)
}

/// An exclusive per-vault startup lock (`flock(2)`), held for the daemon's
/// lifetime so concurrent `start` races cannot yield two live daemons — the
/// loser fails to acquire instead of unlinking the winner's live socket. The
/// kernel releases the lock automatically when the process (and its fd) dies,
/// so a crashed daemon never wedges the next start.
pub struct StartLock {
    _file: std::fs::File,
}

impl StartLock {
    /// Acquire the lock at `path` (0600, created if absent), non-blocking:
    /// if another process holds it, error out rather than wait.
    pub fn acquire(path: &Path) -> std::io::Result<StartLock> {
        use std::os::unix::fs::OpenOptionsExt as _;
        use std::os::unix::io::AsRawFd as _;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path)?;
        // Safety: flock on an owned, open fd; no memory is passed.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(StartLock { _file: file })
    }
}

/// The daemon discovery record, written 0600 at startup and removed on clean
/// shutdown. A client reads it to find the socket and to detect staleness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonRecord {
    /// Control-protocol version this daemon speaks.
    pub api_version: u32,
    /// OS process id of the running daemon.
    pub pid: i32,
    /// Absolute path to the control socket.
    pub socket: PathBuf,
    /// Unix seconds when the daemon started (monotonic-enough for uptime).
    pub started_at: u64,
    /// `zetl` version string that launched the daemon.
    pub version: String,
}

/// Machine-readable liveness classification (REQ-471 C3). Derived from the
/// record plus two observed booleans, so it is a pure function of its inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveness {
    /// No record present — the daemon is not running.
    NotRunning,
    /// Record present, its pid is alive, and the socket exists.
    Running,
    /// Record present but the pid is dead — a crashed daemon. Recoverable:
    /// the caller may clean the record + socket and start fresh.
    StaleCrashed,
    /// Record present, pid alive, but the socket is missing — an inconsistent
    /// state (partial startup or manual deletion). Treated as recoverable.
    StaleInconsistent,
}

impl Liveness {
    /// Whether a fresh `start` may proceed after cleaning up.
    pub fn is_recoverable_stale(self) -> bool {
        matches!(self, Liveness::StaleCrashed | Liveness::StaleInconsistent)
    }
}

/// Pure liveness classifier (hark/elephant pattern, adapted). `record_present`
/// is whether the discovery record parsed; `pid_alive` whether its pid
/// currently exists; `socket_present` whether the socket file exists. The
/// effectful shell supplies the observations; the decision lives here so it is
/// unit-testable without spawning a process.
pub fn classify(record_present: bool, pid_alive: bool, socket_present: bool) -> Liveness {
    match (record_present, pid_alive, socket_present) {
        (false, _, _) => Liveness::NotRunning,
        (true, false, _) => Liveness::StaleCrashed,
        (true, true, false) => Liveness::StaleInconsistent,
        (true, true, true) => Liveness::Running,
    }
}

/// The status a `status` control call reports, or that `lifecycle::status`
/// synthesises when no daemon is running. Machine-readable (REQ-471 C3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum DaemonStatus {
    /// No live daemon for this vault.
    NotRunning,
    /// A crashed/inconsistent daemon record was found (and, by the time the
    /// caller sees this, cleaned).
    Stale { reason: String },
    /// A live daemon, with its self-reported health.
    Running {
        pid: i32,
        uptime_secs: u64,
        /// Vaults the daemon serves (v1: exactly one; multi-vault is REQ-503).
        vaults: Vec<String>,
        /// Notes the daemon owns in the canonical store (manifest size).
        notes: u32,
        /// Connected P2P peers across served vaults.
        peers: u32,
    },
}

/// A control request over the local channel (CON-470 verbs). T2 implements the
/// lifecycle subset; the auth-core verbs are recognised-but-gated (M2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verb", rename_all = "kebab-case")]
pub enum ControlRequest {
    /// Liveness probe — the daemon replies [`ControlResponse::Pong`].
    Ping,
    /// Report health — the daemon replies [`ControlResponse::Status`].
    Status,
    /// Materialise the canonical store to the vault's Markdown files (ADR-470
    /// export). Replies [`ControlResponse::Materialised`].
    Materialise,
    /// Re-import external Markdown edits into the canonical store (REQ-484
    /// guarded import). Replies [`ControlResponse::Reimported`].
    Reimport,
    /// Shut the daemon down cleanly.
    Stop,
}

/// A control response (CON-470 error model).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "kebab-case")]
pub enum ControlResponse {
    Pong {
        api_version: u32,
    },
    Status(DaemonStatus),
    /// Materialised `count` notes to the vault's Markdown files.
    Materialised {
        count: u32,
    },
    /// Re-imported external edits: `folded` into the store, `staged` to the
    /// conflict area.
    Reimported {
        folded: u32,
        staged: u32,
    },
    /// Acknowledged a mutating request (e.g. `stop`).
    Ok,
    /// Typed error (CON-470: `vault-not-found`, `malformed-request`, …).
    Error {
        kind: String,
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    // Regression: the control socket must NOT live under a (possibly deep) vault
    // path — it must fit SUN_LEN. It lives in the short socket_dir, is stably
    // keyed to the vault, and differs per vault; the record stays vault-local.
    #[test]
    fn socket_path_is_short_and_outside_the_vault() {
        let deep = Path::new("/a/very/deeply/nested/vault/path/that/would/blow/past/the/unix/domain/socket/length/limit/on/most/platforms/vault");
        let sock = socket_path(deep);

        assert!(
            !sock.starts_with(deep),
            "socket must not be under the vault"
        );
        assert!(
            sock.starts_with(socket_dir()),
            "socket lives in the short runtime dir"
        );
        assert!(
            sock.as_os_str().len() < 104,
            "socket path must fit SUN_LEN, got {} bytes: {}",
            sock.as_os_str().len(),
            sock.display()
        );
        // Stable and per-vault.
        assert_eq!(sock, socket_path(deep), "stable for a given vault");
        assert_ne!(
            sock,
            socket_path(Path::new("/other/vault")),
            "distinct per vault"
        );
        // The record, by contrast, is vault-local (no length limit).
        assert!(record_path(deep).starts_with(deep));
    }

    // Startup serialisation: two concurrent starts cannot both hold the
    // per-vault lock, and a dead holder (dropped fd) releases it.
    #[test]
    fn start_lock_is_exclusive_and_released_on_drop() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("z.lock");
        let held = StartLock::acquire(&path).unwrap();
        assert!(
            StartLock::acquire(&path).is_err(),
            "second acquire must fail"
        );
        drop(held);
        assert!(
            StartLock::acquire(&path).is_ok(),
            "kernel releases with the fd"
        );
    }

    // TEST-471: liveness classification is a total function of the three
    // observations, and every crashed/inconsistent state is recoverable.
    #[test]
    fn classify_covers_every_observation() {
        assert_eq!(classify(false, false, false), Liveness::NotRunning);
        assert_eq!(classify(false, true, true), Liveness::NotRunning);
        assert_eq!(classify(true, true, true), Liveness::Running);
        assert_eq!(classify(true, false, true), Liveness::StaleCrashed);
        assert_eq!(classify(true, false, false), Liveness::StaleCrashed);
        assert_eq!(classify(true, true, false), Liveness::StaleInconsistent);
    }

    #[test]
    fn stale_states_are_recoverable_running_is_not() {
        assert!(classify(true, false, true).is_recoverable_stale());
        assert!(classify(true, true, false).is_recoverable_stale());
        assert!(!classify(true, true, true).is_recoverable_stale());
        assert!(!classify(false, false, false).is_recoverable_stale());
    }

    #[test]
    fn record_json_roundtrips() {
        let r = DaemonRecord {
            api_version: CONTROL_API_VERSION,
            pid: 4242,
            socket: PathBuf::from("/vault/.zetl/zetld.sock"),
            started_at: 1_700_000_000,
            version: "0.9.3".into(),
        };
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<DaemonRecord>(&json).unwrap(), r);
    }

    #[test]
    fn status_json_is_tagged_machine_readable() {
        let running = DaemonStatus::Running {
            pid: 7,
            uptime_secs: 12,
            vaults: vec!["notes".into()],
            notes: 5,
            peers: 2,
        };
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&running).unwrap()).unwrap();
        assert_eq!(v["state"], "running");
        assert_eq!(v["pid"], 7);

        let none: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&DaemonStatus::NotRunning).unwrap())
                .unwrap();
        assert_eq!(none["state"], "not-running");
    }

    #[test]
    fn control_request_response_roundtrip() {
        for req in [
            ControlRequest::Ping,
            ControlRequest::Status,
            ControlRequest::Materialise,
            ControlRequest::Reimport,
            ControlRequest::Stop,
        ] {
            let json = serde_json::to_string(&req).unwrap();
            assert_eq!(serde_json::from_str::<ControlRequest>(&json).unwrap(), req);
        }
        let resp = ControlResponse::Error {
            kind: "malformed-request".into(),
            message: "bad frame".into(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert_eq!(
            serde_json::from_str::<ControlResponse>(&json).unwrap(),
            resp
        );
    }
}
