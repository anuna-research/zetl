//! Integration tests for `zetl cap invite` (SPEC-034 REQ-3410 /
//! REQ-3416 / CON-3402 / REQ-3430, TEST-3410).
//!
//! These tests drive the real binary against a temp-dir vault seeded
//! with a minimal `recipients.toml`. They exercise:
//!
//! * Delegated-URL mode: URL emitted to stdout; URL carries `#k=…`;
//!   priv_A never appears in stderr or on disk; pub_A lands in
//!   `recipients.toml`; a `bound=false` grant lands in `grants.toml`.
//! * Hardened `--recipient <pubkey>`: no URL fragment; the supplied
//!   pubkey shows up in recipients.toml and the new grant.
//! * `--via enrol-page`: prints an `/enroll.html?cohort=<id>` URL and
//!   writes nothing to grants.toml / recipients.toml.
//! * `--split-key`: URL fragment uses `#k1=` and `half2` is emitted on
//!   a separate stdout line.
//! * REQ-3410 warning banner is emitted on every non-error run.
//! * Two back-to-back invites produce different priv_A values
//!   (no seeded-RNG wiring in the production path).

use assert_cmd::cargo::cargo_bin_cmd;
use tempfile::tempdir;

/// Deterministic `ZETL_CAP_SECRET` used in all tests: version byte +
/// 32 zero bytes + the correct checksum for that body. Built via the
/// pure-core helper rather than hand-pinned so a future checksum-HMAC
/// key change doesn't silently corrupt the fixture.
fn seeded_secret_b64() -> String {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;
    let bytes = zetl::cap::genkey::build_secret(zetl::cap::genkey::SECRET_VERSION_V1, &[0u8; 32]);
    STANDARD.encode(bytes)
}

/// Seed a vault root with a minimal `recipients.toml`. Returns the
/// directory to use as `-d` (i.e. `ZETL_DIR`) plus the recipients
/// body written, so callers can assert on it post-invite.
fn seed_vault(mode: &str) -> tempfile::TempDir {
    let dir = tempdir().expect("tempdir");
    let recipients_body = format!(
        r#"version = 1

[vault]
signing_pubkey = "ed25519:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"

[[cohort]]
id = "engineering"
name = "Engineering Team"
mode = "{mode}"
pubkeys = []
"#
    );
    std::fs::write(dir.path().join("recipients.toml"), recipients_body).unwrap();
    dir
}

/// Write `.zetl/config.toml` carrying an explicit `[access.split_key]`
/// block. `second_factor` goes through verbatim so a test can assert
/// the operator-hint branch for each transport.
fn enable_split_key(dir: &std::path::Path, second_factor: &str) {
    let config_dir = dir.join(".zetl");
    std::fs::create_dir_all(&config_dir).unwrap();
    let body = format!(
        r#"[access.split_key]
enabled = true
second_factor = "{second_factor}"
"#
    );
    std::fs::write(config_dir.join("config.toml"), body).unwrap();
}

#[test]
fn delegated_mode_emits_banner_url_and_writes_grant() {
    let dir = seed_vault("delegated-url");
    let out = cargo_bin_cmd!("zetl")
        .env("ZETL_CAP_SECRET", seeded_secret_b64())
        .env("ZETL_CAP_SITE_URL", "https://wiki.example")
        .args([
            "-d",
            dir.path().to_str().unwrap(),
            "cap",
            "invite",
            "alice",
            "--cohort",
            "engineering",
        ])
        .output()
        .expect("invite runs");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        out.status.success(),
        "exit={:?}\nstdout={stdout}\nstderr={stderr}",
        out.status.code(),
    );

    // Banner hits stdout, verbatim (REQ-3410).
    assert!(
        stdout.contains("SECURITY WARNING"),
        "banner missing from stdout:\n{stdout}",
    );
    assert!(stdout.contains("URL shorteners"));

    // Delegated URL has a fragment key.
    let url_line = stdout
        .lines()
        .find(|l| l.contains("/c/"))
        .expect("expected `/c/` URL line in stdout");
    assert!(
        url_line.contains("#k="),
        "URL missing #k= fragment:\n{url_line}",
    );

    // Grant id echoed to stderr, not stdout (stdout is for the URL).
    assert!(
        stderr.contains("grant id:"),
        "grant id not surfaced to stderr:\n{stderr}",
    );

    // Extract fragment key and assert it does NOT appear on stderr.
    let frag_key = url_line
        .split("#k=")
        .nth(1)
        .expect("fragment key split")
        .trim();
    assert!(
        !stderr.contains(frag_key),
        "priv_A leaked to stderr:\n{stderr}",
    );

    // Verify grants.toml exists + carries exactly one grant with
    // `bound=false`, `revoked=false`, and the right cohort.
    let grants_body =
        std::fs::read_to_string(dir.path().join("grants.toml")).expect("grants.toml written");
    let grants =
        zetl::cap::grants::validation::GrantsFile::from_toml(&grants_body).expect("valid TOML");
    assert_eq!(grants.grants.len(), 1);
    let g = &grants.grants[0];
    assert_eq!(g.cohort, "engineering");
    assert!(!g.bound);
    assert!(!g.revoked);
    assert_eq!(g.name.as_deref(), Some("alice"));
    assert!(
        g.recipient
            .starts_with(zetl::cap::recipients::parsing::AGE_RECIPIENT_V1_PREFIX),
        "grant recipient missing age-recipient-v1 prefix: {:?}",
        g.recipient,
    );

    // priv_A must NOT appear in grants.toml.
    assert!(
        !grants_body.contains(frag_key),
        "priv_A leaked to grants.toml:\n{grants_body}",
    );

    // Recipients file gained exactly one pubkey under the cohort.
    let rec_body =
        std::fs::read_to_string(dir.path().join("recipients.toml")).expect("recipients.toml");
    let rec =
        zetl::cap::recipients::parsing::RecipientsFile::parse(&rec_body).expect("valid recipients");
    assert_eq!(rec.cohorts[0].pubkeys.len(), 1);
    assert_eq!(rec.cohorts[0].pubkeys[0], g.recipient);
    // salt_stable must be populated on first invite so subsequent URLs
    // stay stable across invocations.
    assert!(
        rec.cohorts[0].salt_stable.is_some(),
        "first invite must seed salt_stable on the cohort",
    );

    // priv_A must NOT appear in recipients.toml either.
    assert!(
        !rec_body.contains(frag_key),
        "priv_A leaked to recipients.toml:\n{rec_body}",
    );
}

#[test]
fn two_invites_produce_distinct_priv_a() {
    let dir = seed_vault("delegated-url");
    let out1 = cargo_bin_cmd!("zetl")
        .env("ZETL_CAP_SECRET", seeded_secret_b64())
        .env("ZETL_CAP_SITE_URL", "https://wiki.example")
        .args([
            "-d",
            dir.path().to_str().unwrap(),
            "cap",
            "invite",
            "alice",
            "--cohort",
            "engineering",
        ])
        .output()
        .expect("run");
    let out2 = cargo_bin_cmd!("zetl")
        .env("ZETL_CAP_SECRET", seeded_secret_b64())
        .env("ZETL_CAP_SITE_URL", "https://wiki.example")
        .args([
            "-d",
            dir.path().to_str().unwrap(),
            "cap",
            "invite",
            "bob",
            "--cohort",
            "engineering",
        ])
        .output()
        .expect("run");
    assert!(out1.status.success());
    assert!(out2.status.success());
    let s1 = String::from_utf8_lossy(&out1.stdout);
    let s2 = String::from_utf8_lossy(&out2.stdout);
    let k1 = s1
        .lines()
        .find(|l| l.contains("#k="))
        .and_then(|l| l.split("#k=").nth(1))
        .expect("k1");
    let k2 = s2
        .lines()
        .find(|l| l.contains("#k="))
        .and_then(|l| l.split("#k=").nth(1))
        .expect("k2");
    assert_ne!(k1, k2, "two invites produced the same priv_A");
}

#[test]
fn split_key_mode_prints_half2_separately() {
    let dir = seed_vault("delegated-url");
    enable_split_key(dir.path(), "spoken-phrase");
    let out = cargo_bin_cmd!("zetl")
        .env("ZETL_CAP_SECRET", seeded_secret_b64())
        .env("ZETL_CAP_SITE_URL", "https://wiki.example")
        .args([
            "-d",
            dir.path().to_str().unwrap(),
            "cap",
            "invite",
            "carol",
            "--cohort",
            "engineering",
            "--split-key",
        ])
        .output()
        .expect("run");
    assert!(
        out.status.success(),
        "exit={:?}\nstdout={}\nstderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    // URL fragment uses #k1= under split-key.
    assert!(
        stdout.contains("#k1="),
        "split-key URL must use #k1= prefix:\n{stdout}",
    );
    assert!(!stdout.contains("#k="));
    // half2 is printed on its own line.
    assert!(
        stdout.contains("half2 ="),
        "half2 missing from stdout:\n{stdout}",
    );
    // Operator hint reflects the configured second factor.
    assert!(
        stdout.contains("second_factor = \"spoken-phrase\""),
        "expected spoken-phrase hint:\n{stdout}",
    );
}

/// REQ-3430 acceptance: `--split-key` is refused when the config is
/// absent or when `[access.split_key] enabled = false`. Proves the
/// opt-in semantics (default is disabled).
#[test]
fn split_key_mode_refused_without_opt_in() {
    let dir = seed_vault("delegated-url");
    let out = cargo_bin_cmd!("zetl")
        .env("ZETL_CAP_SECRET", seeded_secret_b64())
        .env("ZETL_CAP_SITE_URL", "https://wiki.example")
        .args([
            "-d",
            dir.path().to_str().unwrap(),
            "cap",
            "invite",
            "carol",
            "--cohort",
            "engineering",
            "--split-key",
        ])
        .output()
        .expect("run");
    assert!(
        !out.status.success(),
        "expected refusal without opt-in; stdout={}",
        String::from_utf8_lossy(&out.stdout),
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("REQ-3430") && stderr.contains("access.split_key"),
        "expected opt-in diagnostic in stderr:\n{stderr}",
    );
    // And no grants.toml was written.
    assert!(!dir.path().join("grants.toml").exists());
}

/// TEST-3430 reconstruction: half1 (URL fragment) XOR half2 (side
/// channel) MUST equal the priv_A that matches the pubkey stored in
/// `recipients.toml`. The URL alone does not carry priv_A.
#[test]
fn split_key_halves_reconstruct_priv_a_that_matches_recipient() {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;

    let dir = seed_vault("delegated-url");
    enable_split_key(dir.path(), "qr");
    let out = cargo_bin_cmd!("zetl")
        .env("ZETL_CAP_SECRET", seeded_secret_b64())
        .env("ZETL_CAP_SITE_URL", "https://wiki.example")
        .args([
            "-d",
            dir.path().to_str().unwrap(),
            "cap",
            "invite",
            "carol",
            "--cohort",
            "engineering",
            "--split-key",
        ])
        .output()
        .expect("run");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);

    // Extract half1 from the URL fragment (after `#k1=`).
    let url_line = stdout
        .lines()
        .find(|l| l.contains("#k1="))
        .expect("expected #k1= URL line in stdout");
    let half1_b64 = url_line
        .split("#k1=")
        .nth(1)
        .expect("half1 fragment")
        .trim();
    // Extract half2 from the `half2 = …` stdout line.
    let half2_b64 = stdout
        .lines()
        .find(|l| l.starts_with("half2 ="))
        .and_then(|l| l.split_once('=').map(|(_, v)| v.trim()))
        .expect("half2 line in stdout");
    // QR-mode hint must have been emitted.
    assert!(
        stdout.contains("second_factor = \"qr\""),
        "expected qr hint:\n{stdout}",
    );

    let h1 = URL_SAFE_NO_PAD.decode(half1_b64).expect("half1 base64url");
    let h2 = URL_SAFE_NO_PAD.decode(half2_b64).expect("half2 base64url");
    assert_eq!(h1.len(), 32);
    assert_eq!(h2.len(), 32);
    let mut priv_a = [0u8; 32];
    for i in 0..32 {
        priv_a[i] = h1[i] ^ h2[i];
    }

    // Derive pub_A from the reconstructed priv_A; it must match the
    // `age-recipient-v1:` entry that landed in recipients.toml.
    use x25519_dalek::{PublicKey, StaticSecret};
    let sk = StaticSecret::from(priv_a);
    let pk = PublicKey::from(&sk);
    let expected_recipient = format!(
        "{}{}",
        zetl::cap::recipients::parsing::AGE_RECIPIENT_V1_PREFIX,
        URL_SAFE_NO_PAD.encode(pk.as_bytes()),
    );

    let rec_body = std::fs::read_to_string(dir.path().join("recipients.toml")).unwrap();
    assert!(
        rec_body.contains(&expected_recipient),
        "reconstructed pubkey {expected_recipient:?} missing from recipients.toml:\n{rec_body}",
    );

    // And: neither half alone may be enough. A partially-leaked
    // fragment (half1 only) does NOT carry priv_A — under XOR with a
    // uniform half2 the observed half1 is itself a uniform 32-byte
    // string and carries zero information about priv_A. We prove the
    // negative structurally: half1 is not equal to priv_a (with
    // overwhelming probability under a CSPRNG).
    assert_ne!(
        h1.as_slice(),
        priv_a.as_slice(),
        "half1 must not equal reconstructed priv_A — the whole point of \
         XOR splitting is that each half alone is independent of priv_A",
    );
}

#[test]
fn hardened_recipient_mode_uses_pre_collected_pubkey() {
    let dir = seed_vault("webauthn-prf");
    let rcpt = format!(
        "{}AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        zetl::cap::recipients::parsing::AGE_RECIPIENT_V1_PREFIX
    );
    let out = cargo_bin_cmd!("zetl")
        .env("ZETL_CAP_SECRET", seeded_secret_b64())
        .env("ZETL_CAP_SITE_URL", "https://wiki.example")
        .args([
            "-d",
            dir.path().to_str().unwrap(),
            "cap",
            "invite",
            "dan",
            "--cohort",
            "engineering",
            "--recipient",
            &rcpt,
        ])
        .output()
        .expect("run");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Hardened URL has no fragment.
    let url_line = stdout
        .lines()
        .find(|l| l.contains("/c/"))
        .expect("url line");
    assert!(
        !url_line.contains('#'),
        "hardened URL must not carry a fragment:\n{url_line}",
    );
    // Grant was written with the supplied recipient verbatim.
    let grants_body = std::fs::read_to_string(dir.path().join("grants.toml")).expect("grants.toml");
    assert!(grants_body.contains(&rcpt));
}

#[test]
fn via_enrol_page_prints_enrolment_url_and_writes_nothing() {
    let dir = seed_vault("webauthn-prf");
    let out = cargo_bin_cmd!("zetl")
        .env("ZETL_CAP_SITE_URL", "https://wiki.example")
        .args([
            "-d",
            dir.path().to_str().unwrap(),
            "cap",
            "invite",
            "eve",
            "--cohort",
            "engineering",
            "--via",
            "enrol-page",
        ])
        .output()
        .expect("run");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("/enroll.html?cohort=engineering"),
        "expected enrol-page URL in stdout:\n{stdout}",
    );
    // Warning banner still printed.
    assert!(stdout.contains("SECURITY WARNING"));
    // grants.toml must NOT have been written — enrol-page flow waits
    // for the reader to return a pubkey first.
    assert!(
        !dir.path().join("grants.toml").exists(),
        "grants.toml must not be written under --via enrol-page",
    );
    // recipients.toml must not have gained any pubkeys either.
    let rec = zetl::cap::recipients::parsing::RecipientsFile::parse(
        &std::fs::read_to_string(dir.path().join("recipients.toml")).unwrap(),
    )
    .unwrap();
    assert!(rec.cohorts[0].pubkeys.is_empty());
}

#[test]
fn unknown_cohort_errors_with_hint() {
    let dir = seed_vault("delegated-url");
    let out = cargo_bin_cmd!("zetl")
        .env("ZETL_CAP_SECRET", seeded_secret_b64())
        .env("ZETL_CAP_SITE_URL", "https://wiki.example")
        .args([
            "-d",
            dir.path().to_str().unwrap(),
            "cap",
            "invite",
            "alice",
            "--cohort",
            "marketing",
        ])
        .output()
        .expect("run");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("cohort") && stderr.contains("marketing"),
        "expected cohort-not-found diagnostic in stderr:\n{stderr}",
    );
}

#[test]
fn missing_secret_env_errors() {
    let dir = seed_vault("delegated-url");
    let out = cargo_bin_cmd!("zetl")
        .env_remove("ZETL_CAP_SECRET")
        .env("ZETL_CAP_SITE_URL", "https://wiki.example")
        .args([
            "-d",
            dir.path().to_str().unwrap(),
            "cap",
            "invite",
            "alice",
            "--cohort",
            "engineering",
        ])
        .output()
        .expect("run");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("ZETL_CAP_SECRET"),
        "expected secret-env diagnostic in stderr:\n{stderr}",
    );
}
