//! Hardened-mode reader self-enrolment page end-to-end
//! (SPEC-034 REQ-3404 / REQ-3414 / CON-3409, TEST-3404 + TEST-3414
//! Rust side).
//!
//! Runs `zetl::cap::build::run_capability_build` with an
//! `enroll_integrity` SRI token threaded through `BuildConfig` and
//! asserts:
//!
//! - the static `/enroll.html` lands at `<out_dir>/enroll.html`,
//! - the emitted HTML carries the SRI-tagged
//!   `<script src="/assets/enroll.js" integrity="sha384-…">`, the
//!   CSP meta fallback, and the pinned mount-point id,
//! - emission is gated on `enroll_integrity = Some(_)` — None-
//!   valued configs do NOT write `enroll.html`,
//! - the Rust-side PRF-salt derivation matches the REQ-3414
//!   formula for the exact (origin, cohort-id) strings the
//!   browser will feed to SubtleCrypto,
//! - cross-cohort unlinkability (REQ-3414 / BUG-003): the same
//!   reader enrolling in two cohorts on the same origin derives
//!   two distinct salts, so the two X25519 pubkeys differ.

use std::fs;

use age::x25519;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

use zetl::cap::build::{run_capability_build, BuildConfig, PageInput, Visibility};
use zetl::cap::deploy_headers::CAP_CSP;
use zetl::cap::enrolment::{
    compute_prf_salt, ENROLL_HTML_FILENAME, ENROLL_JS_PATH, ENROLL_MOUNT_ID, PRF_SALT_PREFIX,
};
use zetl::cap::genkey::{
    build_secret, decode_secret, encode_secret, ParsedSecret, SECRET_VERSION_V1,
};
use zetl::cap::grants::validation::{Grant, GrantMode, GrantsFile};
use zetl::cap::recipients::parsing::{
    Cohort, CohortMode, RecipientsFile, VaultSection, AGE_RECIPIENT_V1_PREFIX,
};
use zetl::cap::scoping::access_config::AccessConfig;
use zetl::cap::sign::VaultSigningKey;

const SAMPLE_ENROLL_SRI: &str =
    "sha384-EnrollAAAABBBBCCCCDDDDEEEEFFFFGGGGHHHHIIIIJJJJKKKKLLLLMMMMNNNNOOOOP0=";

fn sample_secret() -> ParsedSecret {
    let random = [0x5Au8; 32];
    let bytes = build_secret(SECRET_VERSION_V1, &random);
    let encoded = encode_secret(&bytes);
    decode_secret(&encoded).expect("secret round-trips")
}

fn signing_key() -> VaultSigningKey {
    VaultSigningKey::from_bytes(&[0x42u8; 32])
}

fn pubkey_from_age1(s: &str) -> [u8; 32] {
    let (hrp, data) = bech32::decode(s).expect("valid bech32");
    assert_eq!(hrp.as_str(), "age");
    let mut pk = [0u8; 32];
    assert_eq!(data.len(), 32);
    pk.copy_from_slice(&data);
    pk
}

fn age_recipient_v1(pk: &[u8; 32]) -> String {
    let b64 = URL_SAFE_NO_PAD.encode(pk);
    format!("{AGE_RECIPIENT_V1_PREFIX}{b64}")
}

fn fresh_identity_pair() -> (x25519::Identity, [u8; 32]) {
    let id = x25519::Identity::generate();
    let pk = pubkey_from_age1(&id.to_public().to_string());
    (id, pk)
}

fn minimal_recipients(cohort_mode: CohortMode, pk: [u8; 32]) -> RecipientsFile {
    let salt = URL_SAFE_NO_PAD.encode(b"stable-salt-cohort-enrol-0123456");
    RecipientsFile {
        version: 1,
        vault: VaultSection {
            signing_pubkey: format!(
                "ed25519:{}",
                URL_SAFE_NO_PAD.encode(signing_key().verifying_key().to_bytes())
            ),
        },
        cohorts: vec![Cohort {
            id: "engineering".to_string(),
            name: Some("Engineering".to_string()),
            mode: cohort_mode,
            pubkeys: vec![age_recipient_v1(&pk)],
            pages: None,
            salt_stable: Some(salt),
            salt_rotated: None,
            last_rotated: None,
        }],
    }
}

fn minimal_grants() -> GrantsFile {
    GrantsFile {
        version: Some(1),
        grants: vec![Grant {
            id: "g_enrol_01".into(),
            cohort: "engineering".into(),
            recipient: format!("{AGE_RECIPIENT_V1_PREFIX}AA"),
            mode: GrantMode::DelegatedUrl,
            bound: false,
            name: Some("Alice".into()),
            created: "2026-04-20T00:00:00Z".into(),
            expires: Some("2026-12-31T23:59:59Z".into()),
            revoked: false,
            pages: "*".into(),
        }],
    }
}

fn sample_pages() -> Vec<PageInput> {
    vec![PageInput {
        slug: "welcome".into(),
        html: "<p>hello cohort</p>".into(),
        explicit_cohorts: vec!["engineering".into()],
    }]
}

fn cfg_with_enroll(out: &std::path::Path, enroll_sri: Option<&str>) -> BuildConfig {
    BuildConfig {
        vault_root: std::path::PathBuf::from("/vault"),
        out_dir: out.to_path_buf(),
        build_epoch: "2026-04-20T12:00:00Z".to_string(),
        now_unix: 1_745_149_200,
        path_cap_bits: 64,
        visibility: Visibility::Private,
        access: AccessConfig::default(),
        shim_integrity: None,
        enroll_integrity: enroll_sri.map(str::to_string),
        vault_name: "test-vault".to_string(),
        tombstones: Vec::new(),
    }
}

/// TEST-3404: with `enroll_integrity = Some(_)`, the build emits
/// `<out_dir>/enroll.html` with the SRI-tagged enroll.js script,
/// the CSP meta fallback, and the mount-point id the JS bundle
/// expects.
#[test]
fn enroll_html_emitted_when_integrity_threaded() {
    let tmp = TempDir::new().unwrap();
    let (_id, pk) = fresh_identity_pair();
    let cfg = cfg_with_enroll(tmp.path(), Some(SAMPLE_ENROLL_SRI));

    let summary = run_capability_build(
        &cfg,
        &minimal_recipients(CohortMode::WebauthnPrf, pk),
        &minimal_grants(),
        &sample_secret(),
        &signing_key(),
        &sample_pages(),
    )
    .expect("build succeeds");
    assert_eq!(summary.pages_encrypted, 1);

    let path = tmp.path().join(ENROLL_HTML_FILENAME);
    assert!(path.exists(), "expected emitted file at {path:?}");
    let html = fs::read_to_string(&path).unwrap();

    assert!(html.starts_with("<!DOCTYPE html>"), "got:\n{html}");

    let expected_script = format!(
        "<script defer src=\"{ENROLL_JS_PATH}\" integrity=\"{SAMPLE_ENROLL_SRI}\" \
         crossorigin=\"anonymous\"></script>"
    );
    assert!(
        html.contains(&expected_script),
        "missing SRI-tagged enroll script in:\n{html}"
    );

    let csp_escaped = CAP_CSP.replace('\'', "&#39;");
    let expected_csp =
        format!("<meta http-equiv=\"Content-Security-Policy\" content=\"{csp_escaped}\">");
    assert!(
        html.contains(&expected_csp),
        "missing CSP meta fallback in:\n{html}"
    );

    let expected_mount = format!("<main id=\"{ENROLL_MOUNT_ID}\" data-zetl-enroll>");
    assert!(
        html.contains(&expected_mount),
        "missing mount-point div in:\n{html}"
    );
}

/// TEST-3404 (negative): when the caller does not thread an
/// integrity token, no `enroll.html` is written. Gating keeps
/// delegated-URL-only builds lean and lets pre-bundle dev builds
/// skip the enrolment surface.
#[test]
fn enroll_html_not_emitted_when_integrity_absent() {
    let tmp = TempDir::new().unwrap();
    let (_id, pk) = fresh_identity_pair();
    let cfg = cfg_with_enroll(tmp.path(), None);

    run_capability_build(
        &cfg,
        &minimal_recipients(CohortMode::DelegatedUrl, pk),
        &minimal_grants(),
        &sample_secret(),
        &signing_key(),
        &sample_pages(),
    )
    .expect("build succeeds");

    let path = tmp.path().join(ENROLL_HTML_FILENAME);
    assert!(
        !path.exists(),
        "enroll.html must not exist when enroll_integrity is None; found {path:?}"
    );
}

/// TEST-3404: emission is idempotent — a second run against the
/// same out_dir overwrites without error and leaves the file
/// byte-identical.
#[test]
fn enroll_html_is_overwritten_idempotently() {
    let tmp = TempDir::new().unwrap();
    let (_id, pk) = fresh_identity_pair();
    let cfg = cfg_with_enroll(tmp.path(), Some(SAMPLE_ENROLL_SRI));

    for _ in 0..2 {
        run_capability_build(
            &cfg,
            &minimal_recipients(CohortMode::WebauthnPrf, pk),
            &minimal_grants(),
            &sample_secret(),
            &signing_key(),
            &sample_pages(),
        )
        .expect("build succeeds");
    }
    let a = fs::read_to_string(tmp.path().join(ENROLL_HTML_FILENAME)).unwrap();

    // One more run, then compare.
    run_capability_build(
        &cfg,
        &minimal_recipients(CohortMode::WebauthnPrf, pk),
        &minimal_grants(),
        &sample_secret(),
        &signing_key(),
        &sample_pages(),
    )
    .expect("build succeeds");
    let b = fs::read_to_string(tmp.path().join(ENROLL_HTML_FILENAME)).unwrap();
    assert_eq!(a, b);
}

/// TEST-3414 Rust cross-check: the salt the browser will compute
/// for a given (origin, cohort-id) pair matches the spec formula
/// byte-for-byte. The browser-side code lives in
/// `src/cap/shim/enroll.ts::computePrfSalt` and feeds these exact
/// bytes to `navigator.credentials.create({extensions: {prf: {
/// eval: {first}}}})`.
#[test]
fn prf_salt_matches_req_3414_formula() {
    let origin = "https://wiki.example.org";
    let cohort = "engineering";
    let mut h = Sha256::new();
    h.update(PRF_SALT_PREFIX.as_bytes());
    h.update(origin.as_bytes());
    h.update(b"/");
    h.update(cohort.as_bytes());
    let expected = h.finalize();

    let got = compute_prf_salt(origin, cohort);
    assert_eq!(&got[..], expected.as_slice());
}

/// TEST-3414: cross-cohort unlinkability — the same reader
/// enrolling on the same origin in two cohorts derives two
/// distinct PRF salts. Since the PRF is a PRF (outputs are
/// salt-distinguishable even from an attacker who holds both
/// ciphertext sets), this gives the spec's BUG-003 resolution:
/// an observer cannot link two cohort-scoped recipients to the
/// same reader.
#[test]
fn prf_salt_cross_cohort_unlinkable() {
    let origin = "https://wiki.example.org";
    let eng = compute_prf_salt(origin, "engineering");
    let ops = compute_prf_salt(origin, "ops");
    assert_ne!(eng, ops);
}

/// TEST-3414: different origins produce different salts. An
/// attacker who enrols readers at their own domain cannot derive
/// a salt that matches the wiki operator's cohort, so the reader
/// cannot be tricked into producing a pubkey that looks like a
/// valid enrolment for the real wiki.
#[test]
fn prf_salt_cross_origin_unlinkable() {
    let cohort = "engineering";
    let real = compute_prf_salt("https://wiki.example.org", cohort);
    let fake = compute_prf_salt("https://attacker.example", cohort);
    assert_ne!(real, fake);
}
