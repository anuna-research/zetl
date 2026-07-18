//! Control-channel client + liveness observation (SPEC-047 CON-470). Effectful
//! shell: connects to a vault's control socket, exchanges one framed
//! request/response, and observes process/socket liveness for [`super::classify`].

use super::server::{read_frame, write_frame};
use super::{record_path, socket_path, ControlRequest, ControlResponse, DaemonRecord};
use anyhow::{Context, Result};
use std::path::Path;
use std::time::Duration;
use tokio::net::UnixStream;

/// Default control round-trip timeout — the local socket is same-host, so a
/// slow reply means a wedged daemon, not latency.
pub const CONTROL_TIMEOUT: Duration = Duration::from_secs(5);

/// Read and parse a vault's daemon record, if present and well-formed.
pub fn read_record(vault_root: &Path) -> Option<DaemonRecord> {
    let bytes = std::fs::read(record_path(vault_root)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Whether a process with `pid` currently exists. Unix: `kill(pid, 0)`.
pub fn pid_alive(pid: i32) -> bool {
    #[cfg(unix)]
    {
        // SIG 0 performs error checking without sending a signal: 0 => exists,
        // EPERM => exists but not ours (still alive), ESRCH => gone.
        let ret = unsafe { libc::kill(pid, 0) };
        if ret == 0 {
            return true;
        }
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

/// Whether a vault's control socket file exists.
pub fn socket_present(vault_root: &Path) -> bool {
    socket_path(vault_root).exists()
}

/// Send one control request to a vault's daemon and await the response.
pub async fn request(vault_root: &Path, req: &ControlRequest) -> Result<ControlResponse> {
    let sock = socket_path(vault_root);
    let mut stream = tokio::time::timeout(CONTROL_TIMEOUT, UnixStream::connect(&sock))
        .await
        .context("control connect timed out")?
        .with_context(|| format!("connect control socket {}", sock.display()))?;
    let bytes = serde_json::to_vec(req).context("serialise control request")?;
    write_frame(&mut stream, &bytes).await?;
    let resp_bytes = tokio::time::timeout(CONTROL_TIMEOUT, read_frame(&mut stream))
        .await
        .context("control reply timed out")??;
    serde_json::from_slice(&resp_bytes).context("parse control response")
}
