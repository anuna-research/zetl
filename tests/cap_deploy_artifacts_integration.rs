//! Deploy-artifact emission — end-to-end integration tests for
//! SPEC-034 REQ-3418 / CON-3406 (task-cap-deploy-artifacts).
//!
//! Exercises [`zetl::cap::deploy_artifacts`] both at the pure-renderer
//! layer and under [`zetl::cap::build::run_capability_build`] so the
//! deploy tree (`_gone.map`, `_redirects`, top-level `vercel.json`,
//! per-platform `deploy-*.conf`, optional `<vault>-<cohort>.html`
//! single-file bundle) lands alongside the ciphertext root.

use std::fs;
use std::path::Path;

use age::x25519;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use tempfile::TempDir;

use zetl::cap::build::{run_capability_build, BuildConfig, PageInput, Visibility};
use zetl::cap::deploy_artifacts::{
    render_deploy_nginx, render_gone_map, render_netlify_redirects, render_top_vercel_json,
    GENERATED_MARKER,
};
use zetl::cap::deploy_headers::HeaderSpec;
use zetl::cap::genkey::{build_secret, decode_secret, encode_secret, ParsedSecret, SECRET_VERSION_V1};
use zetl::cap::grants::validation::{Grant, GrantMode, GrantsFile};
use zetl::cap::recipients::parsing::{
    Cohort, CohortMode, RecipientsFile, VaultSection, AGE_RECIPIENT_V1_PREFIX,
};
use zetl::cap::scoping::access_config::{AccessConfig, SingleFileConfig};
use zetl::cap::sign::VaultSigningKey;

const ARTIFACT_FILES: &[(&str, bool)] = &[
    ("_zetl/_gone.map", true),
    ("_zetl/deploy-nginx.conf", true),
    ("_zetl/deploy-caddy.conf", true),
    ("_zetl/deploy-netlify.conf", true),
    ("_zetl/deploy-vercel.conf", true),
    ("_zetl/deploy-cloudflare.conf", true),
    ("_redirects", true),
    ("vercel.json", false),
];

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

fn recipients_with_cohorts(cohorts: Vec<Cohort>) -> RecipientsFile {
    RecipientsFile {
        version: 1,
        vault: VaultSection {
            signing_pubkey: format!(
                "ed25519:{}",
                URL_SAFE_NO_PAD.encode(signing_key().verifying_key().to_bytes())
            ),
        },
        cohorts,
    }
}

fn cohort(id: &str, pk: [u8; 32]) -> Cohort {
    let salt = URL_SAFE_NO_PAD.encode(format!("stable-salt-{id}").as_bytes());
    Cohort {
        id: id.to_string(),
        name: None,
        mode: CohortMode::DelegatedUrl,
        pubkeys: vec![age_recipient_v1(&pk)],
        pages: None,
        salt_stable: Some(salt),
        salt_rotated: None,
        last_rotated: None,
    }
}

fn one_grant(cohort_id: &str) -> GrantsFile {
    GrantsFile {
        version: Some(1),
        grants: vec![Grant {
            id: "g_01".into(),
            cohort: cohort_id.into(),
            recipient: format!("{AGE_RECIPIENT_V1_PREFIX}AA"),
            mode: GrantMode::DelegatedUrl,
            bound: false,
            name: None,
            created: "2026-04-20T00:00:00Z".into(),
            expires: Some("2027-01-01T00:00:00Z".into()),
            revoked: false,
            pages: "*".into(),
        }],
    }
}

fn cfg(out: &Path, access: AccessConfig, tombstones: Vec<String>) -> BuildConfig {
    BuildConfig {
        vault_root: std::path::PathBuf::from("/vault"),
        out_dir: out.to_path_buf(),
        build_epoch: "2026-04-20T12:00:00Z".to_string(),
        now_unix: 1_745_149_200,
        path_cap_bits: 64,
        visibility: Visibility::Private,
        access,
        shim_integrity: None,
        enroll_integrity: None,
        vault_name: "my-wiki".to_string(),
        tombstones,
    }
}

fn page(slug: &str) -> PageInput {
    PageInput {
        slug: slug.into(),
        html: format!("<h1>{slug}</h1>"),
        explicit_cohorts: vec![],
    }
}

/// Core TEST-3418 gate: every artifact in the task description lands
/// in the dist tree with the expected generated-file marker.
#[test]
fn default_build_emits_every_deploy_artifact() {
    let tmp = TempDir::new().unwrap();
    let (_a, pk) = fresh_identity_pair();

    run_capability_build(
        &cfg(tmp.path(), AccessConfig::default(), vec![]),
        &recipients_with_cohorts(vec![cohort("engineering", pk)]),
        &one_grant("engineering"),
        &sample_secret(),
        &signing_key(),
        &[page("welcome")],
    )
    .expect("build");

    for (rel, expect_marker) in ARTIFACT_FILES {
        let path = tmp.path().join(rel);
        assert!(path.is_file(), "missing {}", path.display());
        if *expect_marker {
            let body = fs::read_to_string(&path).unwrap();
            assert!(
                body.contains(GENERATED_MARKER),
                "{rel} missing generator marker; body:\n{body}"
            );
        }
    }
}

/// TEST-3418: the top-level `vercel.json` is valid JSON with the
/// REQ-3407 / CON-3406 Cache-Control value in place.
#[test]
fn top_vercel_json_is_well_formed_and_carries_cache_control() {
    let tmp = TempDir::new().unwrap();
    let (_a, pk) = fresh_identity_pair();

    run_capability_build(
        &cfg(tmp.path(), AccessConfig::default(), vec![]),
        &recipients_with_cohorts(vec![cohort("engineering", pk)]),
        &one_grant("engineering"),
        &sample_secret(),
        &signing_key(),
        &[page("welcome")],
    )
    .unwrap();

    let body = fs::read_to_string(tmp.path().join("vercel.json")).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&body).expect("vercel.json must be valid JSON");
    let headers = parsed["headers"].as_array().unwrap();
    let ct_entry = headers
        .iter()
        .find(|e| e["source"] == "/c/(.*)")
        .expect("missing /c/(.*) header entry");
    let values: Vec<&str> = ct_entry["headers"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|h| h["value"].as_str())
        .collect();
    assert!(values.iter().any(|v| v.contains("max-age=300")));
    // redirects array present, empty by default.
    assert!(parsed["redirects"].as_array().unwrap().is_empty());
}

/// Operator-supplied tombstones propagate into every platform's
/// artifact — nginx map, Netlify `_redirects`, Vercel `redirects`,
/// and the per-platform `deploy-*.conf` copy-paste recipes.
#[test]
fn tombstones_propagate_to_all_platforms() {
    let tmp = TempDir::new().unwrap();
    let (_a, pk) = fresh_identity_pair();
    let tombstones = vec![
        "/c/aabbccdd/gone-one.html".to_string(),
        "/c/eeff0011/gone-two.html".to_string(),
    ];

    run_capability_build(
        &cfg(tmp.path(), AccessConfig::default(), tombstones.clone()),
        &recipients_with_cohorts(vec![cohort("engineering", pk)]),
        &one_grant("engineering"),
        &sample_secret(),
        &signing_key(),
        &[page("welcome")],
    )
    .unwrap();

    let gone = fs::read_to_string(tmp.path().join("_zetl").join("_gone.map")).unwrap();
    let redirects = fs::read_to_string(tmp.path().join("_redirects")).unwrap();
    let vercel: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(tmp.path().join("vercel.json")).unwrap()).unwrap();

    for path in &tombstones {
        assert!(
            gone.contains(&format!("{path} 1;")),
            "gone.map missing {path}:\n{gone}"
        );
        assert!(
            redirects.contains(&format!("{path} 410!")),
            "_redirects missing {path}:\n{redirects}"
        );
        assert!(
            vercel["redirects"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["source"] == *path),
            "vercel.json missing redirect for {path}"
        );
    }
}

/// Single-file bundle round-trip: when `[access.single_file] enabled
/// = true`, the build emits `<vault>-<cohort>.html` alongside the
/// ciphertext tree with every envelope inlined as a base64-encoded
/// `<template>` block keyed by slug. Cohort ids with unsafe
/// characters (e.g. `/`) collapse to `_`.
#[test]
fn single_file_bundle_emits_one_file_per_cohort_when_enabled() {
    let tmp = TempDir::new().unwrap();
    let (_a, pk_eng) = fresh_identity_pair();
    let (_b, pk_ops) = fresh_identity_pair();
    let mut access = AccessConfig::default();
    access.single_file = SingleFileConfig { enabled: true };

    run_capability_build(
        &cfg(tmp.path(), access, vec![]),
        &recipients_with_cohorts(vec![
            cohort("engineering", pk_eng),
            cohort("ops-core", pk_ops),
        ]),
        &one_grant("engineering"),
        &sample_secret(),
        &signing_key(),
        &[page("welcome"), page("runbook")],
    )
    .unwrap();

    for name in ["my-wiki-engineering.html", "my-wiki-ops-core.html"] {
        let path = tmp.path().join(name);
        assert!(path.is_file(), "missing bundle {name}");
        let body = fs::read_to_string(&path).unwrap();
        assert!(body.contains("<main data-zetl-capability data-zetl-bundle>"));
        assert!(body.contains("<meta name=\"zetl-cohort\""));
        assert!(body.contains("<meta name=\"zetl-envelope-count\" content=\"2\">"));
        assert!(body.contains("data-slug=\"welcome\""));
        assert!(body.contains("data-slug=\"runbook\""));
    }
}

/// Default-off: with the config untouched the build does NOT leak a
/// single-file bundle into the dist root.
#[test]
fn single_file_bundle_absent_by_default() {
    let tmp = TempDir::new().unwrap();
    let (_a, pk) = fresh_identity_pair();

    run_capability_build(
        &cfg(tmp.path(), AccessConfig::default(), vec![]),
        &recipients_with_cohorts(vec![cohort("engineering", pk)]),
        &one_grant("engineering"),
        &sample_secret(),
        &signing_key(),
        &[page("welcome")],
    )
    .unwrap();

    let html_siblings: Vec<String> = fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.ends_with(".html"))
        .collect();
    assert!(
        html_siblings.is_empty(),
        "no HTML bundle expected next to dist/, found {html_siblings:?}"
    );
}

/// Pure-renderer round-trip: `render_top_vercel_json` is stable for a
/// given (spec, tombstones) pair so byte-for-byte rebuilds stay
/// deterministic across driver invocations (NFR-3306 equivalent —
/// deploy-time artifacts must match Makefile-hash checks).
#[test]
fn pure_renderers_are_stable_across_calls() {
    let spec = HeaderSpec::from_cache_config(&AccessConfig::default().cache);
    let tomb = vec!["/c/ffff/x.html".to_string()];
    assert_eq!(render_gone_map(&tomb), render_gone_map(&tomb));
    assert_eq!(
        render_netlify_redirects(&tomb),
        render_netlify_redirects(&tomb)
    );
    assert_eq!(
        render_top_vercel_json(&spec, &tomb),
        render_top_vercel_json(&spec, &tomb),
    );
    assert_eq!(
        render_deploy_nginx(&spec, &tomb),
        render_deploy_nginx(&spec, &tomb),
    );
}

/// Robots.txt is already covered by `cap_robots_txt_integration.rs`
/// but the deploy-artifact task ties it to the same `Disallow: /c/`
/// + `Disallow: /_zetl/` expectation, so we pin that a capability
/// build leaves the existing web-layer emission untouched.
#[test]
fn robots_txt_preserved_alongside_deploy_artifacts() {
    // A capability build does NOT run `build_static`, so robots.txt is
    // out of scope for this test — we instead assert the deploy tree
    // does not accidentally contain a colliding `robots.txt` of its
    // own that would shadow the web layer's emission.
    let tmp = TempDir::new().unwrap();
    let (_a, pk) = fresh_identity_pair();

    run_capability_build(
        &cfg(tmp.path(), AccessConfig::default(), vec![]),
        &recipients_with_cohorts(vec![cohort("engineering", pk)]),
        &one_grant("engineering"),
        &sample_secret(),
        &signing_key(),
        &[page("welcome")],
    )
    .unwrap();

    // Capability build alone must not write a `robots.txt` — that job
    // belongs to `zetl::web::build::build_static` per REQ-3418 general
    // robots.txt handling (cap_robots_txt_integration.rs).
    assert!(
        !tmp.path().join("robots.txt").exists(),
        "capability build should delegate robots.txt to the web layer"
    );
}
