//! Integration tests for the `zetl daemon` and `zetl collab` P2P CLI
//! skeleton (SPEC-047 REQ-489 / TEST-489a / TEST-489b).
//!
//! IMPL-047 T1 — build-time CLI surface conformance, no crypto. Every P2P
//! verb of ADR-480 is reachable and returns the `not-yet-implemented`
//! non-zero exit until its handler lands (auth-core handlers are gated on
//! the DESIGN-047 crypto-review). Mirrors the `zetl cap` skeleton pattern.

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;

/// `zetl daemon` verbs (ADR-480: parallels `zetl serve`).
const DAEMON_VERBS: &[&str] = &["start", "stop", "status"];

/// The P2P verbs added to `zetl collab` by SPEC-047 (ADR-480). `passwd`
/// and `share` predate this spec and are not asserted here.
const COLLAB_P2P_VERBS: &[&str] = &["invite", "join", "peers", "revoke"];

/// Stub verbs and any positional args needed to reach the handler (rather
/// than a clap usage error). All are `not-yet-implemented` in T1.
const DAEMON_STUBS: &[(&str, &[&str])] =
    &[("start", &[]), ("stop", &[]), ("status", &[])];
const COLLAB_STUBS: &[(&str, &[&str])] = &[
    ("invite", &[]),
    ("join", &[]),
    ("peers", &[]),
    ("revoke", &["node-id-xyz"]),
];

#[test]
fn daemon_help_lists_every_verb() {
    let assert = cargo_bin_cmd!("zetl").args(["daemon", "--help"]).assert();
    let out = assert.get_output();
    let stdout = String::from_utf8_lossy(&out.stdout);
    for verb in DAEMON_VERBS {
        assert!(
            stdout.contains(verb),
            "`zetl daemon --help` is missing verb `{verb}`:\n{stdout}",
        );
    }
    assert!(out.status.success());
}

#[test]
fn collab_help_lists_p2p_verbs() {
    let assert = cargo_bin_cmd!("zetl").args(["collab", "--help"]).assert();
    let out = assert.get_output();
    let stdout = String::from_utf8_lossy(&out.stdout);
    for verb in COLLAB_P2P_VERBS {
        assert!(
            stdout.contains(verb),
            "`zetl collab --help` is missing P2P verb `{verb}`:\n{stdout}",
        );
    }
    assert!(out.status.success());
}

#[test]
fn daemon_each_verb_has_help() {
    for verb in DAEMON_VERBS {
        let assert = cargo_bin_cmd!("zetl")
            .args(["daemon", verb, "--help"])
            .assert()
            .success();
        let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
        assert!(
            stdout.contains("Usage:"),
            "`zetl daemon {verb} --help` is missing a Usage: block:\n{stdout}",
        );
    }
}

#[test]
fn collab_each_p2p_verb_has_help() {
    for verb in COLLAB_P2P_VERBS {
        let assert = cargo_bin_cmd!("zetl")
            .args(["collab", verb, "--help"])
            .assert()
            .success();
        let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
        assert!(
            stdout.contains("Usage:"),
            "`zetl collab {verb} --help` is missing a Usage: block:\n{stdout}",
        );
    }
}

#[test]
fn daemon_stub_verbs_exit_not_yet_implemented() {
    for (verb, extra) in DAEMON_STUBS {
        let mut args = vec!["daemon", verb];
        args.extend_from_slice(extra);
        cargo_bin_cmd!("zetl")
            .args(&args)
            .assert()
            .code(2)
            .stderr(predicate::str::contains(format!(
                "zetl daemon {verb}: not-yet-implemented"
            )));
    }
}

#[test]
fn collab_p2p_stub_verbs_exit_not_yet_implemented() {
    for (verb, extra) in COLLAB_STUBS {
        let mut args = vec!["collab", verb];
        args.extend_from_slice(extra);
        cargo_bin_cmd!("zetl")
            .args(&args)
            .assert()
            .code(2)
            .stderr(predicate::str::contains(format!(
                "zetl collab {verb}: not-yet-implemented"
            )));
    }
}

/// TEST-489a: `--json` emits a parseable JSON error and still exits non-zero.
#[test]
fn p2p_stub_emits_json_under_json_flag() {
    let assert = cargo_bin_cmd!("zetl")
        .args(["--json", "collab", "invite"])
        .assert()
        .code(2);
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    let parsed: serde_json::Value =
        serde_json::from_str(stderr.trim()).expect("stderr should be parseable JSON under --json");
    assert_eq!(parsed["code"], 2);
    assert!(
        parsed["error"]
            .as_str()
            .unwrap_or("")
            .contains("not-yet-implemented"),
        "JSON error did not contain not-yet-implemented marker: {parsed}",
    );
}

/// TEST-489a: the `-f json` global flag path also emits parseable JSON.
#[test]
fn p2p_stub_emits_json_under_format_flag() {
    let assert = cargo_bin_cmd!("zetl")
        .args(["-f", "json", "daemon", "status"])
        .assert()
        .code(2);
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    let _: serde_json::Value =
        serde_json::from_str(stderr.trim()).expect("stderr should be parseable JSON under -f json");
}

/// The invite/join verbs accept the global `--vault` selector (REQ-503 /
/// CON-474): it must parse even though the handler is stubbed.
#[test]
fn invite_accepts_vault_selector() {
    cargo_bin_cmd!("zetl")
        .args(["collab", "invite", "--vault", "notes"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("not-yet-implemented"));
}

#[test]
fn p2p_unknown_verb_errors() {
    cargo_bin_cmd!("zetl")
        .args(["daemon", "definitely-not-a-verb"])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("unrecognized subcommand")
                .or(predicate::str::contains("error:")),
        );
}
