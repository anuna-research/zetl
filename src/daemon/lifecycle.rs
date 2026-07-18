//! Daemon lifecycle orchestration (SPEC-047 REQ-470/471). Effectful shell:
//! `start`/`stop`/`status`, each **idempotent** (REQ-471) and each recovering
//! from a crashed daemon's stale record/socket before acting. The CLI handlers
//! call these synchronously; socket I/O runs on a private current-thread
//! tokio runtime so the rest of the CLI stays sync.

use super::{
    classify, client, record_path, server, socket_path, ControlRequest, ControlResponse,
    DaemonStatus, Liveness,
};
use anyhow::{Context, Result};
use std::path::Path;
use std::time::{Duration, Instant};

/// How long `start` waits for the freshly-spawned daemon to answer a ping.
const START_TIMEOUT: Duration = Duration::from_secs(10);
/// How long `stop` waits for the daemon to remove its socket after `Stop`.
const STOP_TIMEOUT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(50);

/// Outcome of `start` — distinct so the CLI can report idempotent no-ops
/// differently from a real launch (REQ-471).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartOutcome {
    Started { pid: i32 },
    AlreadyRunning { pid: i32 },
    RecoveredStale { pid: i32 },
}

/// Outcome of `stop`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopOutcome {
    Stopped,
    AlreadyStopped,
}

fn rt() -> Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build control runtime")
}

/// Observe current liveness for `vault_root` (pure [`classify`] over three
/// observations).
fn observe(vault_root: &Path) -> (Liveness, Option<i32>) {
    match client::read_record(vault_root) {
        None => (Liveness::NotRunning, None),
        Some(rec) => {
            let alive = client::pid_alive(rec.pid);
            let sock = client::socket_present(vault_root);
            (classify(true, alive, sock), Some(rec.pid))
        }
    }
}

/// Remove a crashed daemon's leftovers so a fresh start can bind cleanly.
fn clean_stale(vault_root: &Path) {
    let _ = std::fs::remove_file(socket_path(vault_root));
    let _ = std::fs::remove_file(record_path(vault_root));
}

/// `zetl daemon status` (REQ-471 C3). Reports a machine-readable status,
/// cleaning stale leftovers as a side effect so repeated calls converge.
pub fn status(vault_root: &Path) -> Result<DaemonStatus> {
    match observe(vault_root) {
        (Liveness::NotRunning, _) => Ok(DaemonStatus::NotRunning),
        (Liveness::StaleCrashed, _) => {
            clean_stale(vault_root);
            Ok(DaemonStatus::Stale {
                reason: "crashed daemon (dead pid) — cleaned".into(),
            })
        }
        (Liveness::StaleInconsistent, _) => {
            clean_stale(vault_root);
            Ok(DaemonStatus::Stale {
                reason: "inconsistent daemon state (missing socket) — cleaned".into(),
            })
        }
        (Liveness::Running, _) => {
            // Ask the live daemon for its self-reported health.
            let resp = rt()?.block_on(client::request(vault_root, &ControlRequest::Status))?;
            match resp {
                ControlResponse::Status(s) => Ok(s),
                other => anyhow::bail!("unexpected control reply to status: {other:?}"),
            }
        }
    }
}

/// `zetl daemon start` (REQ-470/471). Idempotent: a live daemon is reported,
/// not relaunched. A crashed daemon's leftovers are cleaned first.
pub fn start(vault_root: &Path) -> Result<StartOutcome> {
    let (liveness, pid) = observe(vault_root);
    let recovered = match liveness {
        Liveness::Running => {
            return Ok(StartOutcome::AlreadyRunning {
                pid: pid.unwrap_or(0),
            })
        }
        Liveness::StaleCrashed | Liveness::StaleInconsistent => {
            clean_stale(vault_root);
            true
        }
        Liveness::NotRunning => false,
    };

    spawn_detached(vault_root)?;

    // Poll until the daemon answers a ping (its socket + record are live).
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        if let (Liveness::Running, Some(p)) = observe(vault_root) {
            // Confirm it actually answers, not just that files exist.
            if rt()?
                .block_on(client::request(vault_root, &ControlRequest::Ping))
                .is_ok()
            {
                return Ok(if recovered {
                    StartOutcome::RecoveredStale { pid: p }
                } else {
                    StartOutcome::Started { pid: p }
                });
            }
        }
        if Instant::now() >= deadline {
            anyhow::bail!("daemon did not become ready within {START_TIMEOUT:?}");
        }
        std::thread::sleep(POLL);
    }
}

/// `zetl daemon stop` (REQ-471). Idempotent: a daemon that is not running
/// reports [`StopOutcome::AlreadyStopped`].
pub fn stop(vault_root: &Path) -> Result<StopOutcome> {
    match observe(vault_root) {
        (Liveness::NotRunning, _) => Ok(StopOutcome::AlreadyStopped),
        (Liveness::StaleCrashed | Liveness::StaleInconsistent, _) => {
            clean_stale(vault_root);
            Ok(StopOutcome::AlreadyStopped)
        }
        (Liveness::Running, _) => {
            let resp = rt()?.block_on(client::request(vault_root, &ControlRequest::Stop))?;
            if !matches!(resp, ControlResponse::Ok) {
                anyhow::bail!("unexpected control reply to stop: {resp:?}");
            }
            // Wait for the daemon to tear down its socket.
            let deadline = Instant::now() + STOP_TIMEOUT;
            while socket_path(vault_root).exists() {
                if Instant::now() >= deadline {
                    // The daemon acked but did not clean up; force-clean so the
                    // next status is coherent.
                    clean_stale(vault_root);
                    break;
                }
                std::thread::sleep(POLL);
            }
            Ok(StopOutcome::Stopped)
        }
    }
}

/// Run the daemon serve loop in the foreground (the entry the detached child
/// executes, and the target of `zetl daemon start --foreground`).
pub fn run_foreground(vault_root: &Path) -> Result<()> {
    rt()?.block_on(server::run(vault_root))
}

/// `zetl daemon materialise` — ask the running daemon to export its canonical
/// store to the vault's Markdown files. Returns the number of notes written.
/// Errors if no daemon is running (the daemon owns the store).
pub fn materialise(vault_root: &Path) -> Result<u32> {
    match observe(vault_root).0 {
        Liveness::Running => {
            let resp = rt()?.block_on(client::request(vault_root, &ControlRequest::Materialise))?;
            match resp {
                ControlResponse::Materialised { count } => Ok(count),
                ControlResponse::Error { kind, message } => {
                    anyhow::bail!("materialise failed ({kind}): {message}")
                }
                other => anyhow::bail!("unexpected control reply to materialise: {other:?}"),
            }
        }
        _ => anyhow::bail!("no running daemon for this vault — start it with `zetl daemon start`"),
    }
}

/// Spawn `zetl daemon start --foreground --dir <vault>` as a detached,
/// session-leader child so it survives the launching shell and the parent
/// `start` process exiting (REQ-490).
fn spawn_detached(vault_root: &Path) -> Result<()> {
    let exe = std::env::current_exe().context("locate current executable")?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("daemon")
        .arg("start")
        .arg("--foreground")
        .arg("--dir")
        .arg(vault_root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        // Detach from the controlling terminal/process group so a shell exit
        // (SIGHUP) does not take the daemon with it.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    cmd.spawn().context("spawn detached zetld")?;
    Ok(())
}
