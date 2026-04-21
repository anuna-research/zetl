//! Integration tests for the revocation + lifecycle verbs
//! (SPEC-034 REQ-3416 / REQ-3426 / REQ-3427).
//!
//! Covers:
//!
//! - `zetl cap revoke <grant-id>` — flips `revoked=true`; unknown id
//!   exits non-zero; idempotent.
//! - `zetl cap rotate --cohort <id>` — records `salt_rotated` +
//!   `last_rotated` without touching `salt_stable` (REQ-3402 URL
//!   stability contract).
//! - `zetl cap finalise <grant-id>` — sets `bound=true`; idempotent.
//!   `--rotate-grant` rolls the pubkey + reprints the invite URL.
//! - `zetl cap sweep` — marks every past-expires grant revoked;
//!   idempotent.
//! - `zetl cap check` — exits 1 when any grant is expired and not
//!   revoked; exits 0 when the vault is clean; `--public-safety`
//!   is currently a passthrough.
//! - `zetl cap rotate-signing-key` — rewrites
//!   `recipients.toml[vault].signing_pubkey` and prints a new
//!   `ZETL_CAP_SIGNING_KEY` on stdout.

use assert_cmd::cargo::cargo_bin_cmd;
use tempfile::tempdir;

const GOOD_SIGNING_PUBKEY: &str = "ed25519:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const RECIPIENT_A: &str = "age-recipient-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

/// Seed a vault with a recipients.toml + grants.toml containing a mix
/// of active / expired / revoked grants for the tests to probe.
fn seed_vault(mode: &str) -> tempfile::TempDir {
    let dir = tempdir().expect("tempdir");
    let recipients = format!(
        r#"version = 1

[vault]
signing_pubkey = "{GOOD_SIGNING_PUBKEY}"

[[cohort]]
id = "engineering"
name = "Engineering"
mode = "{mode}"
pubkeys = ["{RECIPIENT_A}"]
salt_stable = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
"#
    );
    std::fs::write(dir.path().join("recipients.toml"), recipients).unwrap();
    // Empty grants.toml; tests that need grants write them before running.
    std::fs::write(dir.path().join("grants.toml"), "version = 1\n").unwrap();
    dir
}

fn write_grants(dir: &tempfile::TempDir, body: &str) {
    std::fs::write(dir.path().join("grants.toml"), body).unwrap();
}

fn read_grants(dir: &tempfile::TempDir) -> String {
    std::fs::read_to_string(dir.path().join("grants.toml")).unwrap()
}

fn read_recipients(dir: &tempfile::TempDir) -> String {
    std::fs::read_to_string(dir.path().join("recipients.toml")).unwrap()
}

fn one_active_grant_body() -> String {
    format!(
        r#"version = 1

[[grant]]
id = "g_alice"
cohort = "engineering"
recipient = "{RECIPIENT_A}"
mode = "delegated-url"
bound = false
name = "Alice"
created = "2026-01-01T00:00:00Z"
expires = "2099-01-01T00:00:00Z"
revoked = false
pages = "*"
"#,
    )
}

fn one_expired_grant_body() -> String {
    format!(
        r#"version = 1

[[grant]]
id = "g_stale"
cohort = "engineering"
recipient = "{RECIPIENT_A}"
mode = "delegated-url"
bound = false
name = "Stale"
created = "2020-01-01T00:00:00Z"
expires = "2021-01-01T00:00:00Z"
revoked = false
pages = "*"
"#,
    )
}

// ─── revoke ──────────────────────────────────────────────────────────

#[test]
fn revoke_flips_revoked_flag_and_preserves_file_shape() {
    let dir = seed_vault("delegated-url");
    write_grants(&dir, &one_active_grant_body());

    let out = cargo_bin_cmd!("zetl")
        .args([
            "-d",
            dir.path().to_str().unwrap(),
            "cap",
            "revoke",
            "g_alice",
        ])
        .output()
        .expect("revoke runs");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let body = read_grants(&dir);
    assert!(
        body.contains("revoked = true"),
        "grants.toml did not carry revoked=true:\n{body}",
    );
}

#[test]
fn revoke_unknown_grant_errors() {
    let dir = seed_vault("delegated-url");
    write_grants(&dir, &one_active_grant_body());

    let out = cargo_bin_cmd!("zetl")
        .args([
            "-d",
            dir.path().to_str().unwrap(),
            "cap",
            "revoke",
            "g_does_not_exist",
        ])
        .output()
        .expect("revoke runs");
    assert!(!out.status.success(), "expected non-zero exit");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("not found"),
        "stderr missing not-found diagnostic:\n{stderr}"
    );
}

#[test]
fn revoke_is_idempotent() {
    let dir = seed_vault("delegated-url");
    write_grants(&dir, &one_active_grant_body());

    for _ in 0..2 {
        let out = cargo_bin_cmd!("zetl")
            .args([
                "-d",
                dir.path().to_str().unwrap(),
                "cap",
                "revoke",
                "g_alice",
            ])
            .output()
            .unwrap();
        assert!(out.status.success());
    }
    let body = read_grants(&dir);
    // Only one `revoked = true` line should exist (one grant).
    assert_eq!(
        body.matches("revoked = true").count(),
        1,
        "idempotent revoke should not duplicate the grant:\n{body}"
    );
}

// ─── rotate --cohort ─────────────────────────────────────────────────

#[test]
fn rotate_records_salt_rotated_and_last_rotated() {
    let dir = seed_vault("delegated-url");
    let before = read_recipients(&dir);
    assert!(!before.contains("salt_rotated"));
    assert!(!before.contains("last_rotated"));
    assert!(before.contains("salt_stable"));

    let out = cargo_bin_cmd!("zetl")
        .args([
            "-d",
            dir.path().to_str().unwrap(),
            "cap",
            "rotate",
            "--cohort",
            "engineering",
        ])
        .output()
        .expect("rotate runs");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let after = read_recipients(&dir);
    assert!(after.contains("salt_rotated"), "after:\n{after}");
    assert!(after.contains("last_rotated"), "after:\n{after}");
    // REQ-3402 / BUG-023 URL stability: salt_stable must remain.
    assert!(
        after.contains(r#"salt_stable = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA""#),
        "salt_stable should survive the rotation:\n{after}"
    );
}

#[test]
fn rotate_unknown_cohort_errors() {
    let dir = seed_vault("delegated-url");

    let out = cargo_bin_cmd!("zetl")
        .args([
            "-d",
            dir.path().to_str().unwrap(),
            "cap",
            "rotate",
            "--cohort",
            "no-such-cohort",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("not found"),
        "stderr missing not-found diagnostic:\n{stderr}"
    );
}

// ─── finalise ─────────────────────────────────────────────────────────

#[test]
fn finalise_sets_bound_true() {
    let dir = seed_vault("delegated-url");
    write_grants(&dir, &one_active_grant_body());

    let out = cargo_bin_cmd!("zetl")
        .args([
            "-d",
            dir.path().to_str().unwrap(),
            "cap",
            "finalise",
            "g_alice",
        ])
        .output()
        .expect("finalise runs");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let body = read_grants(&dir);
    assert!(
        body.contains("bound = true"),
        "grants.toml did not carry bound=true:\n{body}"
    );
}

/// REQ-3426: `--rotate-grant` reissues a fresh `priv_A`, swaps the
/// pubkey in both `recipients.toml` and `grants.toml`, resets
/// `bound=false`, and prints the new invite URL on stdout with the
/// REQ-3410 warning banner.
#[test]
fn finalise_rotate_grant_reissues_pubkey_and_prints_new_url() {
    let dir = seed_vault("delegated-url");
    write_grants(&dir, &one_active_grant_body());

    // Deterministic `ZETL_CAP_SECRET`: the same helper the invite
    // integration tests use, so we don't have to round-trip a real
    // genkey invocation.
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;
    let secret_bytes =
        zetl::cap::genkey::build_secret(zetl::cap::genkey::SECRET_VERSION_V1, &[0u8; 32]);
    let secret_b64 = STANDARD.encode(secret_bytes);

    let out = cargo_bin_cmd!("zetl")
        .env("ZETL_CAP_SECRET", &secret_b64)
        .env("ZETL_CAP_SITE_URL", "https://wiki.example")
        .args([
            "-d",
            dir.path().to_str().unwrap(),
            "cap",
            "finalise",
            "g_alice",
            "--rotate-grant",
        ])
        .output()
        .expect("finalise --rotate-grant runs");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("SECURITY WARNING"),
        "reissue must emit the REQ-3410 banner:\n{stdout}"
    );
    assert!(
        stdout.contains("https://wiki.example/c/"),
        "reissue must print a new delegated URL:\n{stdout}"
    );
    assert!(
        stdout.contains("#k="),
        "reissue URL must carry the priv_A fragment:\n{stdout}"
    );

    let recipients_after = read_recipients(&dir);
    assert!(
        !recipients_after.contains(RECIPIENT_A),
        "old recipient pubkey should have been swapped out of recipients.toml:\n{recipients_after}"
    );

    let grants_after = read_grants(&dir);
    assert!(
        !grants_after.contains(&format!("recipient = \"{RECIPIENT_A}\"")),
        "grant row should carry the NEW recipient:\n{grants_after}"
    );
    // bound must reset to false on rotate-grant (REQ-3426).
    assert!(
        grants_after.contains("bound = false"),
        "bound should reset to false on rotate-grant:\n{grants_after}"
    );
}

#[test]
fn finalise_unknown_grant_errors() {
    let dir = seed_vault("delegated-url");
    write_grants(&dir, &one_active_grant_body());

    let out = cargo_bin_cmd!("zetl")
        .args([
            "-d",
            dir.path().to_str().unwrap(),
            "cap",
            "finalise",
            "g_missing",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
}

// ─── sweep ────────────────────────────────────────────────────────────

#[test]
fn sweep_marks_past_expires_grants_revoked() {
    let dir = seed_vault("delegated-url");
    write_grants(&dir, &one_expired_grant_body());

    let out = cargo_bin_cmd!("zetl")
        .args(["-d", dir.path().to_str().unwrap(), "cap", "sweep"])
        .output()
        .expect("sweep runs");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let body = read_grants(&dir);
    assert!(
        body.contains("revoked = true"),
        "sweep did not mark expired grant revoked:\n{body}"
    );
}

#[test]
fn sweep_is_noop_on_all_active() {
    let dir = seed_vault("delegated-url");
    write_grants(&dir, &one_active_grant_body());

    let out = cargo_bin_cmd!("zetl")
        .args(["-d", dir.path().to_str().unwrap(), "cap", "sweep"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let body = read_grants(&dir);
    assert!(body.contains("revoked = false"), "body:\n{body}");
}

// ─── check ────────────────────────────────────────────────────────────

#[test]
fn check_exits_zero_when_vault_is_clean() {
    let dir = seed_vault("delegated-url");
    write_grants(&dir, &one_active_grant_body());

    let out = cargo_bin_cmd!("zetl")
        .args(["-d", dir.path().to_str().unwrap(), "cap", "check"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn check_exits_one_when_stale_grant_still_active() {
    let dir = seed_vault("delegated-url");
    write_grants(&dir, &one_expired_grant_body());

    let out = cargo_bin_cmd!("zetl")
        .args(["-d", dir.path().to_str().unwrap(), "cap", "check"])
        .output()
        .unwrap();
    assert!(!out.status.success(), "check should fail on stale grant");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("g_stale"),
        "stderr should surface the failing grant id:\n{stderr}"
    );
}

#[test]
fn check_public_safety_flag_passes_today() {
    let dir = seed_vault("delegated-url");
    write_grants(&dir, &one_active_grant_body());

    let out = cargo_bin_cmd!("zetl")
        .args([
            "-d",
            dir.path().to_str().unwrap(),
            "cap",
            "check",
            "--public-safety",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
}

// ─── rotate-signing-key ──────────────────────────────────────────────

#[test]
fn rotate_signing_key_overwrites_pubkey_and_prints_new_private_key() {
    let dir = seed_vault("delegated-url");

    let before = read_recipients(&dir);
    assert!(before.contains(GOOD_SIGNING_PUBKEY));

    let out = cargo_bin_cmd!("zetl")
        .args([
            "-d",
            dir.path().to_str().unwrap(),
            "cap",
            "rotate-signing-key",
        ])
        .output()
        .expect("rotate-signing-key runs");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("export ZETL_CAP_SIGNING_KEY="),
        "stdout should carry the new signing-key export line:\n{stdout}"
    );
    // REQ-3419 banner discipline: the new pubkey also lands on stdout
    // in a human-readable form so operators can paste into their PM
    // entry alongside the private key.
    assert!(
        stdout.contains("ed25519:"),
        "stdout should echo the new ed25519 pubkey:\n{stdout}"
    );

    let after = read_recipients(&dir);
    assert!(
        !after.contains(GOOD_SIGNING_PUBKEY),
        "signing_pubkey should have rotated:\n{after}"
    );
    assert!(
        after.contains("signing_pubkey = \"ed25519:"),
        "recipients.toml should still carry an ed25519: signing pubkey:\n{after}"
    );
}

#[test]
fn rotate_signing_key_is_nondeterministic_across_runs() {
    let dir = seed_vault("delegated-url");

    let run = || {
        let out = cargo_bin_cmd!("zetl")
            .args([
                "-d",
                dir.path().to_str().unwrap(),
                "cap",
                "rotate-signing-key",
            ])
            .output()
            .unwrap();
        assert!(out.status.success());
        read_recipients(&dir)
    };

    let a = run();
    let b = run();
    assert_ne!(a, b, "two successive rotations must produce different keys");
}
