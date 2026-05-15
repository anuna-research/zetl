//! Integration tests for `zetl collab passwd` (SPEC-041 REQ-4108, CON-4106).
//!
//! `add`/`remove` read passwords from a TTY prompt by design (REQ-4108 —
//! never from argv or env). Driving them from an `assert_cmd` test would
//! require a pseudo-TTY harness, which is platform-specific overhead the
//! unit tests in `src/web/auth/password.rs` already cover for the store
//! mutation paths. These integration tests therefore exercise:
//!
//! * `list` — fully non-interactive; smoke-tests the dispatch wiring.
//! * `--help` text — confirms the CLI surface (`Add`/`Remove`/`List`).
//! * `remove` of a nonexistent user — non-zero exit + diagnostic, without
//!   needing TTY input.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;

fn make_vault() -> tempfile::TempDir {
    let tmp = tempfile::TempDir::new().unwrap();
    // `.zetl/collab` is what `passwd add` creates on demand. For a `list`
    // on a fresh vault, we just need the vault directory itself to exist.
    fs::create_dir_all(tmp.path().join(".zetl")).unwrap();
    tmp
}

#[test]
fn passwd_list_empty_vault_succeeds_with_no_output() {
    let tmp = make_vault();
    let output = cargo_bin_cmd!("zetl")
        .args(["--dir", tmp.path().to_str().unwrap(), "collab", "passwd", "list"])
        .output()
        .expect("zetl spawn");
    assert!(
        output.status.success(),
        "list should succeed on a fresh vault (exit={:?}, stderr={})",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stdout.is_empty(),
        "list should print nothing for an empty store, got: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn passwd_remove_unknown_user_fails_with_diagnostic() {
    let tmp = make_vault();
    let output = cargo_bin_cmd!("zetl")
        .args([
            "--dir",
            tmp.path().to_str().unwrap(),
            "collab",
            "passwd",
            "remove",
            "ghost",
        ])
        .output()
        .expect("zetl spawn");
    assert!(
        !output.status.success(),
        "remove of unknown user should exit non-zero"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("ghost") || stderr.contains("no password record"),
        "stderr should name the missing user: {stderr}"
    );
}

#[test]
fn passwd_help_lists_subcommands() {
    let output = cargo_bin_cmd!("zetl")
        .args(["collab", "passwd", "--help"])
        .output()
        .expect("zetl spawn");
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    for sub in ["add", "remove", "list"] {
        assert!(
            help.contains(sub),
            "passwd --help should list {sub:?}; got:\n{help}"
        );
    }
}

#[test]
fn collab_help_lists_passwd() {
    let output = cargo_bin_cmd!("zetl")
        .args(["collab", "--help"])
        .output()
        .expect("zetl spawn");
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(
        help.contains("passwd"),
        "collab --help should list passwd; got:\n{help}"
    );
}

#[test]
fn passwd_list_after_direct_upsert_shows_user() {
    // Drive `upsert` directly (TTY-bypass) to seed the store, then verify
    // `list` surfaces the seeded user without revealing any PHC material.
    let tmp = make_vault();
    fs::create_dir_all(tmp.path().join(".zetl/collab")).unwrap();
    zetl::web::auth::password::upsert(tmp.path(), "alice", "hunter2").unwrap();
    zetl::web::auth::password::upsert(tmp.path(), "bob", "swordfish").unwrap();

    let output = cargo_bin_cmd!("zetl")
        .args(["--dir", tmp.path().to_str().unwrap(), "collab", "passwd", "list"])
        .output()
        .expect("zetl spawn");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("alice"), "expected alice; got: {stdout}");
    assert!(stdout.contains("bob"), "expected bob; got: {stdout}");
    // The list must NEVER print PHC material (CON-4106).
    assert!(
        !stdout.contains("$argon2"),
        "list leaked a PHC marker: {stdout}"
    );
}
