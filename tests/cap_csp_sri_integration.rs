//! CSP + SRI shell + deploy-header emission end-to-end
//! (SPEC-034 REQ-3421 / CON-3410 / TEST-3421, BUG-006 resolution).
//!
//! Exercises `zetl::cap::build::run_capability_build` against a
//! minimal vault with an SRI hash threaded through `BuildConfig`
//! and asserts:
//!
//! - the shared HTML shell lands at
//!   `<out_dir>/_zetl/capability-shell.html`,
//! - the shell carries a `<meta http-equiv="Content-Security-Policy">`
//!   with the exact CON-3410 directive,
//! - the shell's shim `<script>` carries the passed-in SRI token,
//!   a `crossorigin="anonymous"` attribute, and targets
//!   `/assets/shim.js`,
//! - the deploy recipes emit the same CSP string as an HTTP header
//!   on `/c/*` and `/enroll.html` — belt-and-braces coverage so a
//!   CDN dropping the header still leaves the meta fallback active.

use std::fs;

use age::x25519;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use tempfile::TempDir;

use zetl::cap::build::{run_capability_build, BuildConfig, PageInput, Visibility};
use zetl::cap::deploy_headers::CAP_CSP;
use zetl::cap::genkey::{
    build_secret, decode_secret, encode_secret, ParsedSecret, SECRET_VERSION_V1,
};
use zetl::cap::grants::validation::{Grant, GrantMode, GrantsFile};
use zetl::cap::html_shell::CAPABILITY_SHELL_FILENAME;
use zetl::cap::recipients::parsing::{
    Cohort, CohortMode, RecipientsFile, VaultSection, AGE_RECIPIENT_V1_PREFIX,
};
use zetl::cap::scoping::access_config::AccessConfig;
use zetl::cap::sign::VaultSigningKey;

const SAMPLE_SRI: &str = "sha384-Zx87eaPeqXyzAAAABBBBCCCCDDDDEEEEFFFFGGGGHHHHIIIIJJJJKKKKLLLL0=";

fn sample_secret() -> ParsedSecret {
    let random = [0xA5u8; 32];
    let bytes = build_secret(SECRET_VERSION_V1, &random);
    let encoded = encode_secret(&bytes);
    decode_secret(&encoded).expect("secret round-trips")
}

fn signing_key() -> VaultSigningKey {
    VaultSigningKey::from_bytes(&[0x77u8; 32])
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

fn minimal_recipients(pk: [u8; 32]) -> RecipientsFile {
    let salt = URL_SAFE_NO_PAD.encode(b"stable-salt-for-cohort-csp-sri-01");
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
            mode: CohortMode::DelegatedUrl,
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
            id: "g_01".to_string(),
            cohort: "engineering".to_string(),
            recipient: format!("{AGE_RECIPIENT_V1_PREFIX}AA"),
            mode: GrantMode::DelegatedUrl,
            bound: false,
            name: None,
            created: "2026-04-20T00:00:00Z".to_string(),
            expires: Some("2027-01-01T00:00:00Z".to_string()),
            revoked: false,
            pages: "*".to_string(),
        }],
    }
}

fn one_page() -> PageInput {
    PageInput {
        slug: "welcome".into(),
        html: "<h1>Welcome</h1>".into(),
        explicit_cohorts: vec![],
    }
}

fn cfg_with_sri(out: &std::path::Path, sri: Option<&str>) -> BuildConfig {
    BuildConfig {
        vault_root: std::path::PathBuf::from("/vault"),
        out_dir: out.to_path_buf(),
        build_epoch: "2026-04-20T12:00:00Z".to_string(),
        now_unix: 1_745_149_200,
        path_cap_bits: 64,
        visibility: Visibility::Private,
        access: AccessConfig::default(),
        shim_integrity: sri.map(str::to_string),
        enroll_integrity: None,
        vault_name: "test-vault".to_string(),
        tombstones: Vec::new(),
    }
}

/// TEST-3421a: shell emitted with CSP meta + SRI-tagged script tag.
#[test]
fn shell_carries_csp_meta_and_sri_script() {
    let tmp = TempDir::new().unwrap();
    let (_alice, alice_pk) = fresh_identity_pair();

    run_capability_build(
        &cfg_with_sri(tmp.path(), Some(SAMPLE_SRI)),
        &minimal_recipients(alice_pk),
        &minimal_grants(),
        &sample_secret(),
        &signing_key(),
        &[one_page()],
    )
    .expect("build");

    let shell = fs::read_to_string(tmp.path().join("_zetl").join(CAPABILITY_SHELL_FILENAME))
        .expect("shell file must exist");

    // CSP meta carries the pinned directive byte-for-byte. The shell
    // escapes `'` → `&#39;`, so compare after the same transform so
    // the assertion is readable.
    let escaped_csp = CAP_CSP.replace('\'', "&#39;");
    let expected_meta =
        format!("<meta http-equiv=\"Content-Security-Policy\" content=\"{escaped_csp}\">");
    assert!(
        shell.contains(&expected_meta),
        "missing CSP meta with pinned directive, got:\n{shell}"
    );

    // Shim script tag is exactly the spec form:
    // `<script src="/assets/shim.js" integrity="sha384-…" crossorigin="anonymous"></script>`
    let expected_script = format!(
        "<script src=\"/assets/shim.js\" integrity=\"{SAMPLE_SRI}\" crossorigin=\"anonymous\"></script>"
    );
    assert!(
        shell.contains(&expected_script),
        "missing SRI-tagged shim script, got:\n{shell}"
    );

    // Shim mount point is present so `cap/shim/render.ts::HOST_SELECTOR`
    // can find its injection sink.
    assert!(shell.contains("<main data-zetl-capability></main>"));
}

/// TEST-3421b: deploy recipes emit the same CSP directive as an HTTP
/// header — belt-and-braces with the meta fallback.
#[test]
fn deploy_recipes_emit_csp_header_matching_shell_meta() {
    let tmp = TempDir::new().unwrap();
    let (_alice, alice_pk) = fresh_identity_pair();

    run_capability_build(
        &cfg_with_sri(tmp.path(), Some(SAMPLE_SRI)),
        &minimal_recipients(alice_pk),
        &minimal_grants(),
        &sample_secret(),
        &signing_key(),
        &[one_page()],
    )
    .expect("build");

    let deploy = tmp.path().join("_zetl").join("deploy");

    // _headers: /c/* and /enroll.html both carry Content-Security-Policy.
    let headers = fs::read_to_string(deploy.join("_headers")).unwrap();
    assert!(
        headers.contains(&format!("/c/*\n  Cache-Control: private, max-age=300, must-revalidate\n  Content-Security-Policy: {CAP_CSP}")),
        "missing CSP under /c/* in _headers:\n{headers}"
    );
    assert!(
        headers.contains(&format!("/enroll.html\n  Clear-Site-Data: \"cache\", \"storage\", \"executionContexts\"\n  Content-Security-Policy: {CAP_CSP}")),
        "missing CSP under /enroll.html in _headers:\n{headers}"
    );
    // /logout is Clear-Site-Data only; it's not an HTML surface so
    // CSP is neither required nor useful there.
    assert!(
        !headers.contains("/logout\n  Clear-Site-Data: \"cache\", \"storage\", \"executionContexts\"\n  Content-Security-Policy:"),
        "/logout must not carry CSP"
    );

    // nginx: same invariant — the CSP string shows up on /c/ and
    // /enroll.html locations.
    let nginx = fs::read_to_string(deploy.join("nginx.conf.snippet")).unwrap();
    assert!(nginx.contains(&format!(
        "add_header Content-Security-Policy \"{CAP_CSP}\" always;"
    )));

    // Caddy: CSP on the @zetl_cap matcher.
    let caddy = fs::read_to_string(deploy.join("Caddyfile.snippet")).unwrap();
    assert!(caddy.contains(&format!(
        "header @zetl_cap Content-Security-Policy \"{CAP_CSP}\""
    )));

    // vercel.json: CSP stacked under /c/(.*) and /enroll.html.
    let vercel: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(deploy.join("vercel.json")).unwrap()).unwrap();
    let top = vercel["headers"].as_array().unwrap();
    let cap_headers = top[0]["headers"].as_array().unwrap();
    assert_eq!(cap_headers.len(), 2);
    assert_eq!(cap_headers[1]["key"], "Content-Security-Policy");
    assert_eq!(cap_headers[1]["value"], CAP_CSP);
}

/// Dev build without a shim bundle: shell is absent but deploy
/// recipes still emit the CSP header. This mirrors the pre-bundle
/// path Node-less CI jobs take.
#[test]
fn without_shim_integrity_shell_is_absent_but_csp_header_still_emitted() {
    let tmp = TempDir::new().unwrap();
    let (_alice, alice_pk) = fresh_identity_pair();

    run_capability_build(
        &cfg_with_sri(tmp.path(), None),
        &minimal_recipients(alice_pk),
        &minimal_grants(),
        &sample_secret(),
        &signing_key(),
        &[one_page()],
    )
    .expect("build");

    assert!(
        !tmp.path()
            .join("_zetl")
            .join(CAPABILITY_SHELL_FILENAME)
            .exists(),
        "shell must not be emitted without a shim_integrity token"
    );
    let headers = fs::read_to_string(tmp.path().join("_zetl").join("deploy").join("_headers"))
        .expect("_headers still emitted");
    assert!(
        headers.contains(&format!("Content-Security-Policy: {CAP_CSP}")),
        "CSP header should ship even without a bundled shim"
    );
}

/// Determinism: rebuilding with the same SRI produces byte-identical
/// shell HTML.
#[test]
fn shell_is_byte_deterministic_across_rebuilds() {
    let tmp1 = TempDir::new().unwrap();
    let tmp2 = TempDir::new().unwrap();
    let (_alice, alice_pk) = fresh_identity_pair();

    for out in [tmp1.path(), tmp2.path()] {
        run_capability_build(
            &cfg_with_sri(out, Some(SAMPLE_SRI)),
            &minimal_recipients(alice_pk),
            &minimal_grants(),
            &sample_secret(),
            &signing_key(),
            &[one_page()],
        )
        .unwrap();
    }
    let s1 = fs::read_to_string(tmp1.path().join("_zetl").join(CAPABILITY_SHELL_FILENAME)).unwrap();
    let s2 = fs::read_to_string(tmp2.path().join("_zetl").join(CAPABILITY_SHELL_FILENAME)).unwrap();
    assert_eq!(s1, s2);
}
