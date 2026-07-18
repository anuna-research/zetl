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
pub mod server;

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Control-socket file name inside a vault's `.zetl/` directory. Per-vault
/// (not `$XDG_RUNTIME_DIR`) so one daemon owns one vault and the socket shares
/// the vault's filesystem permissions (CON-470 C1).
pub const SOCKET_NAME: &str = "zetld.sock";
/// Daemon discovery record file name inside `.zetl/`.
pub const RECORD_NAME: &str = "zetld.json";
/// Control-protocol version — bumped when the request/response shape changes.
pub const CONTROL_API_VERSION: u32 = 1;

/// Path to a vault's `.zetl/` runtime directory.
pub fn runtime_dir(vault_root: &Path) -> PathBuf {
    vault_root.join(".zetl")
}
/// Path to a vault's control socket.
pub fn socket_path(vault_root: &Path) -> PathBuf {
    runtime_dir(vault_root).join(SOCKET_NAME)
}
/// Path to a vault's daemon discovery record.
pub fn record_path(vault_root: &Path) -> PathBuf {
    runtime_dir(vault_root).join(RECORD_NAME)
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
    /// Shut the daemon down cleanly.
    Stop,
}

/// A control response (CON-470 error model).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "kebab-case")]
pub enum ControlResponse {
    Pong { api_version: u32 },
    Status(DaemonStatus),
    /// Materialised `count` notes to the vault's Markdown files.
    Materialised { count: u32 },
    /// Acknowledged a mutating request (e.g. `stop`).
    Ok,
    /// Typed error (CON-470: `vault-not-found`, `malformed-request`, …).
    Error { kind: String, message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(serde_json::from_str::<ControlResponse>(&json).unwrap(), resp);
    }
}
