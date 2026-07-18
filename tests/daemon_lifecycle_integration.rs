//! End-to-end lifecycle tests for the `zetld` daemon (SPEC-047 REQ-470/471/490,
//! IMPL-047 T2). Drives the real `zetl` binary: start spawns a detached
//! daemon, status reports machine-readable liveness, stop tears it down, and
//! every verb is idempotent and recovers from a crashed daemon's stale state.
//!
//! Unix-only (the daemon uses a Unix-domain control socket + setsid).

#![cfg(unix)]

use assert_cmd::cargo::cargo_bin_cmd;
use std::path::Path;
use std::time::{Duration, Instant};

/// Run `zetl daemon <args>` against a vault dir, returning (code, stdout).
fn daemon(vault: &Path, args: &[&str]) -> (i32, String) {
    let mut full = vec!["--dir", vault.to_str().unwrap(), "--json", "daemon"];
    full.extend_from_slice(args);
    let out = cargo_bin_cmd!("zetl")
        .args(&full)
        .output()
        .expect("run zetl daemon");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

fn status_state(vault: &Path) -> String {
    let (code, stdout) = daemon(vault, &["status"]);
    assert_eq!(code, 0, "daemon status should exit 0: {stdout}");
    let v: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("status is JSON");
    v["state"].as_str().unwrap_or("<none>").to_string()
}

/// Poll status until it reaches `want` or the deadline (guards against any
/// residual startup race even though `start` already blocks until ready).
fn await_state(vault: &Path, want: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if status_state(vault) == want {
            return;
        }
        assert!(Instant::now() < deadline, "state never became {want}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Always stop the daemon so a failed assertion doesn't leak a process.
struct Daemon<'a>(&'a Path);
impl Drop for Daemon<'_> {
    fn drop(&mut self) {
        let _ = daemon(self.0, &["stop"]);
    }
}

#[test]
fn start_status_stop_cycle() {
    let tmp = tempfile::tempdir().unwrap();
    let vault = tmp.path();
    let _guard = Daemon(vault);

    // TEST-470a: start launches a daemon; status reports it running.
    let (code, out) = daemon(vault, &["start"]);
    assert_eq!(code, 0, "start should succeed: {out}");
    await_state(vault, "running");

    // Running status carries a live pid and uptime (REQ-471 C3).
    let (_c, s) = daemon(vault, &["status"]);
    let v: serde_json::Value = serde_json::from_str(s.trim()).unwrap();
    assert_eq!(v["state"], "running");
    assert!(v["pid"].as_i64().unwrap_or(0) > 0, "running status has a pid: {v}");

    // TEST-490a: the daemon survives the client (`start`/`status`) processes
    // exiting — it is still running here after those commands returned.
    assert_eq!(status_state(vault), "running");

    // Stop tears it down; status then reports not-running.
    let (code, out) = daemon(vault, &["stop"]);
    assert_eq!(code, 0, "stop should succeed: {out}");
    await_state(vault, "not-running");
}

#[test]
fn start_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let vault = tmp.path();
    let _guard = Daemon(vault);

    let (c1, _) = daemon(vault, &["start"]);
    assert_eq!(c1, 0);
    await_state(vault, "running");

    // TEST-471: a second start does not launch a second daemon — it reports
    // the already-running one.
    let (c2, out2) = daemon(vault, &["start"]);
    assert_eq!(c2, 0, "second start should succeed idempotently: {out2}");
    let v: serde_json::Value = serde_json::from_str(out2.trim()).unwrap();
    assert_eq!(v["daemon"], "already-running", "second start: {v}");
    assert_eq!(status_state(vault), "running");
}

#[test]
fn stop_when_not_running_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let vault = tmp.path();
    // TEST-471: stopping a daemon that is not running is a no-op success.
    let (code, out) = daemon(vault, &["stop"]);
    assert_eq!(code, 0, "stop-when-stopped should succeed: {out}");
    let v: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(v["daemon"], "not-running");
    assert_eq!(status_state(vault), "not-running");
}

#[test]
fn status_of_pristine_vault_is_not_running() {
    let tmp = tempfile::tempdir().unwrap();
    assert_eq!(status_state(tmp.path()), "not-running");
}

#[test]
fn daemon_owns_vault_store_and_materialises() {
    let tmp = tempfile::tempdir().unwrap();
    let vault = tmp.path();
    // Seed the vault with Markdown before the daemon starts.
    std::fs::write(vault.join("one.md"), "# One\n\nbody").unwrap();
    std::fs::create_dir_all(vault.join("sub")).unwrap();
    std::fs::write(vault.join("sub/two.md"), "two").unwrap();
    let _guard = Daemon(vault);

    let (code, _) = daemon(vault, &["start"]);
    assert_eq!(code, 0);
    await_state(vault, "running");

    // REQ-470: the daemon bootstrapped and now owns the two notes.
    let (_c, s) = daemon(vault, &["status"]);
    let v: serde_json::Value = serde_json::from_str(s.trim()).unwrap();
    assert_eq!(v["notes"], 2, "daemon owns both markdown notes: {v}");

    // Materialise exports the canonical store back to disk.
    let (code, out) = daemon(vault, &["materialise"]);
    assert_eq!(code, 0, "materialise should succeed: {out}");
    let v: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(v["materialised"], 2);
    // Content survives the round-trip through the Loro store.
    assert_eq!(
        std::fs::read_to_string(vault.join("one.md")).unwrap(),
        "# One\n\nbody\n"
    );
    assert_eq!(std::fs::read_to_string(vault.join("sub/two.md")).unwrap(), "two\n");
}

#[test]
fn recovers_from_crashed_daemon_stale_state() {
    let tmp = tempfile::tempdir().unwrap();
    let vault = tmp.path();
    let _guard = Daemon(vault);

    let (code, _) = daemon(vault, &["start"]);
    assert_eq!(code, 0);
    await_state(vault, "running");

    // Read the daemon pid from its record and SIGKILL it — simulating a crash
    // that leaves the record (and possibly the socket) behind.
    let record: serde_json::Value = serde_json::from_slice(
        &std::fs::read(vault.join(".zetl/zetld.json")).expect("record exists while running"),
    )
    .unwrap();
    let pid = record["pid"].as_i64().unwrap().to_string();
    let killed = std::process::Command::new("kill")
        .args(["-9", &pid])
        .status()
        .expect("kill -9");
    assert!(killed.success(), "kill the daemon pid {pid}");

    // Give the OS a moment to reap, then status must classify the stale record
    // (dead pid) and clean it — never report the dead daemon as running.
    std::thread::sleep(Duration::from_millis(200));
    let state = status_state(vault);
    assert!(
        state == "stale" || state == "not-running",
        "crashed daemon must report stale/not-running, got {state}",
    );

    // TEST-471b: a fresh start recovers from the stale state and launches.
    let (code, out) = daemon(vault, &["start"]);
    assert_eq!(code, 0, "start after crash should recover: {out}");
    await_state(vault, "running");
}
