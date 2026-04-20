//! Integration tests for the `zetl cap` subcommand skeleton
//! (SPEC-034 REQ-3416, TEST-3416 scaffold).
//!
//! Covers:
//! - `zetl cap --help` lists every verb from REQ-3416.
//! - Each verb's `--help` renders a flags + examples section.
//! - Stubbed verbs exit `CAP_NOT_YET_IMPLEMENTED_EXIT` (2) with a
//!   `not-yet-implemented` diagnostic on stderr.
//! - `--json` / `-f json` emits a parseable JSON error to stderr
//!   while still exiting non-zero.
//! - Unknown verbs fail (clap's default usage-error exit).
//! - `audit-diff` is still wired and not stubbed.

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;

/// Every verb enumerated in SPEC-034 REQ-3416 plus `share` (reserved,
/// per this plan's scope). `audit-diff` has a real implementation;
/// everything else is a stub in this skeleton.
const CAP_VERBS: &[&str] = &[
    "genkey",
    "invite",
    "list",
    "revoke",
    "rotate",
    "finalise",
    "share",
    "check",
    "sweep",
    "pair",
    "audit-diff",
    "rotate-signing-key",
    "emergency-shutdown",
];

const STUB_VERBS: &[(&str, &[&str])] = &[
    ("genkey", &[]),
    ("invite", &["alice", "--cohort", "eng"]),
    ("list", &[]),
    ("revoke", &["grant-id-xyz"]),
    ("rotate", &["--cohort", "eng"]),
    ("finalise", &["grant-id-xyz"]),
    ("share", &["grant-id-xyz"]),
    ("check", &[]),
    ("sweep", &[]),
    ("pair", &[]),
    ("rotate-signing-key", &[]),
    // `emergency-shutdown` is NOT stubbed (SPEC-034 REQ-3431 is landed;
    // see tests/cap_emergency_shutdown_integration.rs).
];

#[test]
fn cap_help_lists_every_verb() {
    let assert = cargo_bin_cmd!("zetl").args(["cap", "--help"]).assert();

    let out = assert.get_output();
    let stdout = String::from_utf8_lossy(&out.stdout);

    for verb in CAP_VERBS {
        assert!(
            stdout.contains(verb),
            "`zetl cap --help` is missing verb `{verb}`:\n{stdout}",
        );
    }
    assert!(out.status.success());
}

#[test]
fn cap_each_verb_has_help() {
    for verb in CAP_VERBS {
        let assert = cargo_bin_cmd!("zetl")
            .args(["cap", verb, "--help"])
            .assert()
            .success();
        let out = assert.get_output();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("Usage:"),
            "`zetl cap {verb} --help` is missing a Usage: block:\n{stdout}",
        );
    }
}

#[test]
fn cap_stub_verbs_exit_not_yet_implemented() {
    for (verb, extra_args) in STUB_VERBS {
        let mut args = vec!["cap", verb];
        args.extend_from_slice(extra_args);

        let assert = cargo_bin_cmd!("zetl")
            .args(&args)
            .assert()
            .code(2)
            .stderr(predicate::str::contains(format!(
                "zetl cap {verb}: not-yet-implemented"
            )));
        let _ = assert;
    }
}

#[test]
fn cap_stub_verbs_emit_json_under_json_flag() {
    let assert = cargo_bin_cmd!("zetl")
        .args(["--json", "cap", "genkey"])
        .assert()
        .code(2);
    let out = assert.get_output();
    let stderr = String::from_utf8_lossy(&out.stderr);

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

#[test]
fn cap_stub_verbs_emit_json_under_format_flag() {
    let assert = cargo_bin_cmd!("zetl")
        .args(["-f", "json", "cap", "sweep"])
        .assert()
        .code(2);
    let out = assert.get_output();
    let stderr = String::from_utf8_lossy(&out.stderr);
    let _: serde_json::Value = serde_json::from_str(stderr.trim())
        .expect("stderr should be parseable JSON under -f json");
}

#[test]
fn cap_unknown_verb_errors() {
    cargo_bin_cmd!("zetl")
        .args(["cap", "definitely-not-a-verb"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unrecognized subcommand").or(
            predicate::str::contains("invalid value").or(predicate::str::contains("error:")),
        ));
}

#[test]
fn cap_audit_diff_not_stubbed() {
    // A bare `zetl cap audit-diff` (no refs, no --corpus) errors out
    // because it requires args — but the error should come from the
    // real handler, not the stub diagnostic.
    let assert = cargo_bin_cmd!("zetl").args(["cap", "audit-diff"]).assert();
    let out = assert.get_output();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("not-yet-implemented"),
        "audit-diff is implemented but emitted the skeleton stub diagnostic: {stderr}",
    );
}
