//! Integration tests for `zetl cap emergency-shutdown`
//! (SPEC-034 REQ-3431, §11.3; BUG-024 resolution).
//!
//! The command is documentation-only: it must print the operator
//! checklist to stdout, exit 0, never modify any files, and surface
//! vault-specific identifiers when they are discoverable on disk
//! (vault name always; cohorts + signing pubkey when a well-formed
//! `recipients.toml` sits at the vault root).

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use std::fs;

/// Minimal well-formed `recipients.toml` for exercising the cohort
/// enumeration path. Mirrors the schema in
/// `src/cap/recipients/parsing.rs`.
const RECIPIENTS_TOML: &str = r#"
version = 1

[vault]
signing_pubkey = "ed25519:ZmFrZXNpZ25pbmdwdWJrZXlmb3J0ZXN0aW5n"

[[cohort]]
id = "engineering"
name = "Engineering Team"
mode = "delegated-url"
pubkeys = [
  "age-recipient-v1:YWJjZGVmZ2hpamtsbW5vcHFyc3R1dnd4eXoxMjM0NTY",
]

[[cohort]]
id = "ops"
mode = "webauthn-prf"
pubkeys = []
"#;

fn fresh_vault(name: &str) -> tempfile::TempDir {
    let td = tempfile::Builder::new()
        .prefix(name)
        .tempdir()
        .expect("tempdir");
    td
}

#[test]
fn emergency_shutdown_prints_full_checklist_on_stdout() {
    let vault = fresh_vault("zetl-cap-es-full");
    fs::write(vault.path().join("recipients.toml"), RECIPIENTS_TOML)
        .expect("write recipients.toml");

    let assert = cargo_bin_cmd!("zetl")
        .args([
            "-d",
            vault.path().to_str().unwrap(),
            "cap",
            "emergency-shutdown",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();

    // Header + all four REQ-3431 canonical steps + the re-enrolment
    // step must appear in order.
    for marker in [
        "zetl cap emergency-shutdown",
        "Step 1 — Remove or redirect DNS",
        "Step 2 — CDN: purge /c/* objects",
        "Step 3 — Rotate ZETL_CAP_SECRET + ZETL_CAP_SIGNING_KEY",
        "Step 4 — Announce to readers",
        "Step 5 — Re-enrolment",
    ] {
        assert!(
            stdout.contains(marker),
            "missing marker `{marker}`:\n{stdout}"
        );
    }

    // REQ-3431 explicitly: this command is NOT automated.
    assert!(
        stdout.contains("does NOT modify"),
        "checklist must state it modifies nothing:\n{stdout}",
    );
}

#[test]
fn emergency_shutdown_enumerates_cohorts_and_signing_pubkey() {
    let vault = fresh_vault("zetl-cap-es-cohorts");
    fs::write(vault.path().join("recipients.toml"), RECIPIENTS_TOML)
        .expect("write recipients.toml");

    let assert = cargo_bin_cmd!("zetl")
        .args([
            "-d",
            vault.path().to_str().unwrap(),
            "cap",
            "emergency-shutdown",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();

    assert!(
        stdout.contains("engineering"),
        "cohort id missing:\n{stdout}"
    );
    assert!(
        stdout.contains("Engineering Team"),
        "cohort human label missing:\n{stdout}",
    );
    assert!(stdout.contains("ops"), "second cohort missing:\n{stdout}");
    assert!(
        stdout.contains("ed25519:ZmFrZXNpZ25pbmdwdWJrZXlmb3J0ZXN0aW5n"),
        "on-disk signing pubkey missing from rotation step:\n{stdout}",
    );
}

#[test]
fn emergency_shutdown_degrades_without_recipients_toml() {
    let vault = fresh_vault("zetl-cap-es-bare");

    let assert = cargo_bin_cmd!("zetl")
        .args([
            "-d",
            vault.path().to_str().unwrap(),
            "cap",
            "emergency-shutdown",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();

    // All five steps still render.
    for marker in ["Step 1", "Step 2", "Step 3", "Step 4", "Step 5"] {
        assert!(
            stdout.contains(marker),
            "missing marker `{marker}`:\n{stdout}"
        );
    }
    // Step 4 falls back to the "no cohorts on file" guidance.
    assert!(
        stdout.contains("recipients.toml` was not found"),
        "fallback guidance missing:\n{stdout}",
    );
}

#[test]
fn emergency_shutdown_is_idempotent_on_disk() {
    let vault = fresh_vault("zetl-cap-es-nowrite");
    fs::write(vault.path().join("recipients.toml"), RECIPIENTS_TOML)
        .expect("write recipients.toml");

    let before: Vec<_> = fs::read_dir(vault.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    let before_body = fs::read_to_string(vault.path().join("recipients.toml")).unwrap();

    cargo_bin_cmd!("zetl")
        .args([
            "-d",
            vault.path().to_str().unwrap(),
            "cap",
            "emergency-shutdown",
        ])
        .assert()
        .success();

    let after: Vec<_> = fs::read_dir(vault.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    let after_body = fs::read_to_string(vault.path().join("recipients.toml")).unwrap();

    assert_eq!(before, after, "vault directory listing changed");
    assert_eq!(
        before_body, after_body,
        "recipients.toml contents changed — command must be read-only",
    );
}

#[test]
fn emergency_shutdown_emits_json_under_json_flag() {
    let vault = fresh_vault("zetl-cap-es-json");
    fs::write(vault.path().join("recipients.toml"), RECIPIENTS_TOML)
        .expect("write recipients.toml");

    let assert = cargo_bin_cmd!("zetl")
        .args([
            "-d",
            vault.path().to_str().unwrap(),
            "--json",
            "cap",
            "emergency-shutdown",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();

    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout must be JSON under --json");
    assert_eq!(parsed["command"], "zetl cap emergency-shutdown");
    assert_eq!(parsed["spec"], "SPEC-034 REQ-3431");
    assert_eq!(parsed["automated"], false);
    assert_eq!(parsed["steps"].as_array().unwrap().len(), 5);
    let cohort_ids: Vec<&str> = parsed["cohorts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert!(cohort_ids.contains(&"engineering"));
    assert!(cohort_ids.contains(&"ops"));
}

#[test]
fn emergency_shutdown_surfaces_vault_basename() {
    let vault = fresh_vault("zetl-cap-es-named-vault");

    let assert = cargo_bin_cmd!("zetl")
        .args([
            "-d",
            vault.path().to_str().unwrap(),
            "cap",
            "emergency-shutdown",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();

    let basename = vault
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(
        stdout.contains(&basename),
        "vault basename `{basename}` missing from output:\n{stdout}",
    );
}

#[test]
fn emergency_shutdown_tolerates_malformed_recipients_toml() {
    let vault = fresh_vault("zetl-cap-es-malformed");
    fs::write(
        vault.path().join("recipients.toml"),
        "this is not toml = = =",
    )
    .expect("write bad toml");

    let assert = cargo_bin_cmd!("zetl")
        .args([
            "-d",
            vault.path().to_str().unwrap(),
            "cap",
            "emergency-shutdown",
        ])
        .assert()
        .success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();

    // Parser failure is surfaced to stderr but never blocks the
    // checklist — an incident operator needs the runbook, not a
    // parser error.
    assert!(
        stderr.contains("unparseable"),
        "warning about unparseable recipients.toml missing:\n{stderr}",
    );
    assert!(stdout.contains("Step 1"));
    assert!(stdout.contains("Step 4"));
}

#[test]
fn emergency_shutdown_is_not_stubbed() {
    // Sanity check mirroring `cap_audit_diff_not_stubbed` in the
    // skeleton suite: the command must not hit the
    // `not-yet-implemented` path.
    let vault = fresh_vault("zetl-cap-es-not-stub");
    let assert = cargo_bin_cmd!("zetl")
        .args([
            "-d",
            vault.path().to_str().unwrap(),
            "cap",
            "emergency-shutdown",
        ])
        .assert()
        .success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(
        !stderr.contains("not-yet-implemented"),
        "emergency-shutdown should be a real handler now, not a stub: {stderr}",
    );
    // Also check that no JSON error body leaked to stderr.
    assert!(predicate::str::is_empty().not().eval(&stderr) || stderr.is_empty());
}
