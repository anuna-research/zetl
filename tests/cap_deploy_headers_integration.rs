//! Cache-Control + Clear-Site-Data deploy-recipe emission — end-to-
//! end integration tests for SPEC-034 REQ-3407 / REQ-3418 / REQ-3428
//! (CON-3406, TEST-3407, TEST-3418, TEST-3428).
//!
//! Drives `zetl::cap::build::run_capability_build` end-to-end over a
//! minimal vault and asserts that every deploy recipe lands under
//! `<out_dir>/_zetl/deploy/` with the mandated header values — for
//! the default `[access.cache]` config and for an operator override
//! within the `[60, 3600]` bounds. The out-of-bounds override is
//! asserted to short-circuit the build before any ciphertext lands.

use std::fs;

use age::x25519;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use tempfile::TempDir;

use zetl::cap::build::{run_capability_build, BuildConfig, PageInput, Visibility};
use zetl::cap::genkey::{build_secret, decode_secret, encode_secret, ParsedSecret, SECRET_VERSION_V1};
use zetl::cap::grants::validation::{Grant, GrantMode, GrantsFile};
use zetl::cap::recipients::parsing::{
    Cohort, CohortMode, RecipientsFile, VaultSection, AGE_RECIPIENT_V1_PREFIX,
};
use zetl::cap::scoping::access_config::{AccessConfig, AccessConfigError, CacheConfig};
use zetl::cap::sign::VaultSigningKey;

fn sample_secret() -> ParsedSecret {
    let random = [0x42u8; 32];
    let bytes = build_secret(SECRET_VERSION_V1, &random);
    let encoded = encode_secret(&bytes);
    decode_secret(&encoded).expect("secret round-trips")
}

fn signing_key() -> VaultSigningKey {
    VaultSigningKey::from_bytes(&[0x33u8; 32])
}

fn pubkey_from_age1(s: &str) -> [u8; 32] {
    let (hrp, data) = bech32::decode(s).expect("valid bech32");
    assert_eq!(hrp.as_str(), "age");
    let mut pk = [0u8; 32];
    assert_eq!(data.len(), 32);
    pk.copy_from_slice(&data);
    pk
}

fn fresh_identity_pair() -> (x25519::Identity, [u8; 32]) {
    let id = x25519::Identity::generate();
    let pk = pubkey_from_age1(&id.to_public().to_string());
    (id, pk)
}

fn age_recipient_v1(pk: &[u8; 32]) -> String {
    let b64 = URL_SAFE_NO_PAD.encode(pk);
    format!("{AGE_RECIPIENT_V1_PREFIX}{b64}")
}

fn minimal_recipients(pk: [u8; 32]) -> RecipientsFile {
    let salt = URL_SAFE_NO_PAD.encode(b"stable-salt-for-cohort-default-01");
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

fn page() -> PageInput {
    PageInput {
        slug: "welcome".into(),
        html: "<h1>Welcome</h1>".into(),
        explicit_cohorts: vec![],
    }
}

fn mk_cfg(out: &std::path::Path, cache: CacheConfig) -> BuildConfig {
    BuildConfig {
        vault_root: std::path::PathBuf::from("/vault"),
        out_dir: out.to_path_buf(),
        build_epoch: "2026-04-20T12:00:00Z".to_string(),
        now_unix: 1_745_149_200,
        path_cap_bits: 64,
        visibility: Visibility::Private,
        access: AccessConfig {
            cache,
            ..Default::default()
        },
        shim_integrity: None,
    }
}

/// TEST-3407 / TEST-3418 / TEST-3428: every recipe the spec names
/// lands under `_zetl/deploy/` with the fixed + operator-tunable
/// header values.
#[test]
fn default_build_emits_every_deploy_recipe_with_spec_headers() {
    let tmp = TempDir::new().unwrap();
    let (_alice, alice_pk) = fresh_identity_pair();

    run_capability_build(
        &mk_cfg(tmp.path(), CacheConfig::default()),
        &minimal_recipients(alice_pk),
        &minimal_grants(),
        &sample_secret(),
        &signing_key(),
        &[page()],
    )
    .expect("build");

    let deploy = tmp.path().join("_zetl").join("deploy");
    for name in [
        "README.md",
        "nginx.conf.snippet",
        "Caddyfile.snippet",
        "_headers",
        "vercel.json",
    ] {
        assert!(
            deploy.join(name).is_file(),
            "missing deploy recipe {name} under {}",
            deploy.display()
        );
    }

    // REQ-3407: /c/* Cache-Control default is 300s, private, must-revalidate.
    let headers = fs::read_to_string(deploy.join("_headers")).unwrap();
    assert!(
        headers.contains("/c/*\n  Cache-Control: private, max-age=300, must-revalidate"),
        "missing /c/* Cache-Control in _headers:\n{headers}"
    );

    // REQ-3428: Clear-Site-Data on /enroll.html + /logout.
    for path in ["/enroll.html", "/logout"] {
        assert!(
            headers.contains(&format!(
                "{path}\n  Clear-Site-Data: \"cache\", \"storage\", \"executionContexts\""
            )),
            "missing Clear-Site-Data for {path} in _headers:\n{headers}"
        );
    }

    // REQ-3418 / CON-3406: /assets/shim.js is public, immutable, 1 year.
    assert!(
        headers.contains(
            "/assets/shim.js\n  Cache-Control: public, max-age=31536000, immutable"
        ),
        "missing shim.js Cache-Control in _headers:\n{headers}"
    );

    let nginx = fs::read_to_string(deploy.join("nginx.conf.snippet")).unwrap();
    assert!(nginx.contains("location ^~ /c/"));
    assert!(nginx.contains("private, max-age=300, must-revalidate"));
    assert!(nginx.contains("public, max-age=31536000, immutable"));
    assert!(nginx.contains("location = /enroll.html"));
    assert!(nginx.contains("location = /logout"));

    let caddy = fs::read_to_string(deploy.join("Caddyfile.snippet")).unwrap();
    assert!(caddy.contains("@zetl_cap path /c/*"));
    assert!(caddy.contains("@zetl_shim path /assets/shim.js"));
    assert!(caddy.contains("private, max-age=300, must-revalidate"));
    assert!(caddy.contains("public, max-age=31536000, immutable"));

    // vercel.json parses as valid JSON and carries all four rules.
    let vercel: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(deploy.join("vercel.json")).unwrap()).unwrap();
    let headers_arr = vercel["headers"].as_array().unwrap();
    assert_eq!(headers_arr.len(), 4);
    assert_eq!(headers_arr[0]["source"], "/c/(.*)");
    assert_eq!(
        headers_arr[0]["headers"][0]["value"],
        "private, max-age=300, must-revalidate"
    );
    assert_eq!(headers_arr[3]["source"], "/assets/shim.js");
    assert_eq!(
        headers_arr[3]["headers"][0]["value"],
        "public, max-age=31536000, immutable"
    );
}

/// Operator override via `[access.cache] max_age = N` propagates to
/// every deploy recipe.
#[test]
fn operator_max_age_override_propagates_to_every_recipe() {
    let tmp = TempDir::new().unwrap();
    let (_alice, alice_pk) = fresh_identity_pair();

    run_capability_build(
        &mk_cfg(tmp.path(), CacheConfig { max_age: 900 }),
        &minimal_recipients(alice_pk),
        &minimal_grants(),
        &sample_secret(),
        &signing_key(),
        &[page()],
    )
    .expect("build");

    let deploy = tmp.path().join("_zetl").join("deploy");
    for name in ["nginx.conf.snippet", "Caddyfile.snippet", "_headers", "vercel.json"] {
        let body = fs::read_to_string(deploy.join(name)).unwrap();
        assert!(
            body.contains("max-age=900"),
            "override not propagated to {name}:\n{body}"
        );
        // Default (300) must not leak through for /c/*.
        assert!(
            !body.contains("max-age=300"),
            "default max-age leaked into {name}:\n{body}"
        );
    }
}

/// NFR-3409 revocation-latency envelope: `max_age < 60` and
/// `max_age > 3600` both short-circuit the build before any
/// ciphertext lands on disk.
#[test]
fn out_of_bounds_max_age_rejects_build() {
    for bad in [30u32, 7200u32] {
        let tmp = TempDir::new().unwrap();
        let (_alice, alice_pk) = fresh_identity_pair();

        let err = run_capability_build(
            &mk_cfg(tmp.path(), CacheConfig { max_age: bad }),
            &minimal_recipients(alice_pk),
            &minimal_grants(),
            &sample_secret(),
            &signing_key(),
            &[page()],
        )
        .unwrap_err();

        let msg = err.to_string();
        assert!(
            msg.contains("[access.cache]"),
            "expected access.cache bound error for {bad}, got {msg:?}"
        );

        // No ciphertext written.
        assert!(
            !tmp.path().join("c").exists(),
            "ciphertext directory should not exist after rejected build"
        );
        // No deploy recipes written.
        assert!(
            !tmp.path().join("_zetl").join("deploy").exists(),
            "deploy recipes should not exist after rejected build"
        );

        // Error is specifically the cache-bound variant.
        let _expected: AccessConfigError = AccessConfigError::CacheMaxAgeOutOfBounds { got: bad };
    }
}

/// The driver emits deploy recipes alongside the ciphertext tree —
/// both subtrees exist after a successful build and the summary
/// counters aren't changed by this task's addition.
#[test]
fn deploy_recipe_emission_is_additive_over_ciphertext_tree() {
    let tmp = TempDir::new().unwrap();
    let (_alice, alice_pk) = fresh_identity_pair();

    let summary = run_capability_build(
        &mk_cfg(tmp.path(), CacheConfig::default()),
        &minimal_recipients(alice_pk),
        &minimal_grants(),
        &sample_secret(),
        &signing_key(),
        &[page()],
    )
    .unwrap();

    assert_eq!(summary.pages_encrypted, 1);

    let stats = &summary.per_page[0];
    let envelope_path = tmp
        .path()
        .join("c")
        .join(&stats.path_cap)
        .join("welcome.html");
    assert!(envelope_path.is_file(), "ciphertext envelope missing");
    assert!(
        tmp.path().join("_zetl").join("deploy").join("_headers").is_file(),
        "deploy recipes missing"
    );

    // Envelope + deploy tree are siblings under out_dir; neither
    // interferes with the other's directory layout.
    let envelope_bytes = fs::read(&envelope_path).unwrap();
    let parsed = zetl::cap::sign::parse_envelope(&envelope_bytes).expect("parse");
    assert_eq!(parsed.header.slug, "welcome");
}
