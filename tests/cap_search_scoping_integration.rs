//! Capability-mode search & backlinks scoping (SPEC-034 REQ-3415,
//! TEST-3415).
//!
//! Acceptance, reproduced from the task:
//!
//! - Default build: no search UI emitted (the `access` config defaults
//!   to `search.mode = "off"` and `search_ui_enabled()` stays false).
//! - Backlinks panel scoped to sources that share a cohort with the
//!   target (the `scoping::backlinks` pure-core filter).
//! - Attempt to set `[access.search] mode = "global"` →
//!   `run_capability_build` returns `BuildError::AccessConfig(...)`
//!   with a REQ-3415 diagnostic _before_ any ciphertext lands on
//!   disk.
//! - Same rule for `[access.backlinks] mode = "global"`.
//! - `per-cohort` opt-in validates but still emits no index in v1
//!   (BUG-019: index format deferred).

use std::fs;

use age::x25519;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use tempfile::TempDir;

use ztl::cap::build::{run_capability_build, BuildConfig, BuildError, PageInput, Visibility};
use ztl::cap::genkey::{build_secret, decode_secret, encode_secret, SECRET_VERSION_V1};
use ztl::cap::grants::validation::{Grant, GrantMode, GrantsFile};
use ztl::cap::recipients::parsing::{
    Cohort, CohortMode, RecipientsFile, VaultSection, AGE_RECIPIENT_V1_PREFIX,
};
use ztl::cap::scoping::access_config::{
    AccessConfig, AccessConfigError, BacklinksConfig, BacklinksMode, SearchConfig, SearchMode,
};
use ztl::cap::scoping::backlinks::{
    scope_backlinks_for_target, scope_backlinks_per_cohort, RawBacklink,
};
use ztl::cap::scoping::cohort_index::{CohortIndex, CohortScope, PageRef};
use ztl::cap::sign::VaultSigningKey;

// ─── helpers (trimmed copies of the ones in the build-driver test
// file — duplicated rather than factored out because integration
// tests in Rust do not share a support module unless explicitly set
// up, and the fixture shape is small) ──────────────────────────────

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

fn sample_secret() -> ztl::cap::genkey::ParsedSecret {
    let random = [0x7Eu8; 32];
    let bytes = build_secret(SECRET_VERSION_V1, &random);
    decode_secret(&encode_secret(&bytes)).expect("secret round-trips")
}

fn signing_key() -> VaultSigningKey {
    VaultSigningKey::from_bytes(&[0x22u8; 32])
}

fn mk_cohort(id: &str, pk: &[u8; 32], salt_b64: &str) -> Cohort {
    Cohort {
        id: id.to_string(),
        name: None,
        mode: CohortMode::DelegatedUrl,
        pubkeys: vec![age_recipient_v1(pk)],
        pages: None,
        salt_stable: Some(salt_b64.to_string()),
        salt_rotated: None,
        last_rotated: None,
    }
}

fn mk_recipients(cohorts: Vec<Cohort>) -> RecipientsFile {
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

fn mk_grants() -> GrantsFile {
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

fn build_cfg(out: &std::path::Path, access: AccessConfig) -> BuildConfig {
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
        vault_name: "test-vault".to_string(),
        tombstones: Vec::new(),
    }
}

fn sample_pages() -> Vec<PageInput> {
    vec![PageInput {
        slug: "welcome".into(),
        html: "<p>Hi.</p>".into(),
        explicit_cohorts: vec![],
    }]
}

// ─── tests ─────────────────────────────────────────────────────────

/// Default build: no search UI emitted. The config-level accessor is
/// the single source of truth — a future renderer that _can_ emit a
/// search UI will gate on it, and this test will keep that contract
/// honest.
#[test]
fn test_3415_default_build_emits_no_search_ui() {
    let cfg = AccessConfig::default();
    assert_eq!(cfg.search.mode, SearchMode::Off);
    assert!(!cfg.search_ui_enabled());

    // And the build itself runs to completion at the default setting.
    let tmp = TempDir::new().unwrap();
    let salt = URL_SAFE_NO_PAD.encode(b"eng-salt-default-scoping-01");
    let (_alice, pk) = fresh_identity_pair();
    let recipients = mk_recipients(vec![mk_cohort("engineering", &pk, &salt)]);
    let summary = run_capability_build(
        &build_cfg(tmp.path(), AccessConfig::default()),
        &recipients,
        &mk_grants(),
        &sample_secret(),
        &signing_key(),
        &sample_pages(),
    )
    .expect("default build succeeds");
    assert_eq!(summary.pages_encrypted, 1);

    // No search-index artefact is written anywhere beneath /c/ — the
    // driver currently writes only envelope files, but this assert
    // guards against a future accidental emission that would leak
    // cross-cohort metadata.
    let dist = tmp.path().join("c");
    let mut found_index = false;
    for entry in walkdir(&dist) {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if name.contains("search") || name.contains("index.json") || name.contains("index.js") {
            found_index = true;
        }
    }
    assert!(!found_index, "default build must not emit a search index");
}

/// `[access.search] mode = "global"` fails the build up front with
/// the REQ-3415 diagnostic.  No `/c/` tree is written.
#[test]
fn test_3415_global_search_rejected_at_build_start() {
    let tmp = TempDir::new().unwrap();
    let salt = URL_SAFE_NO_PAD.encode(b"eng-salt-global-reject-01");
    let (_alice, pk) = fresh_identity_pair();
    let recipients = mk_recipients(vec![mk_cohort("engineering", &pk, &salt)]);

    let access = AccessConfig {
        search: SearchConfig {
            mode: SearchMode::Global,
        },
        ..Default::default()
    };
    let err = run_capability_build(
        &build_cfg(tmp.path(), access),
        &recipients,
        &mk_grants(),
        &sample_secret(),
        &signing_key(),
        &sample_pages(),
    )
    .expect_err("global search must be rejected");
    match err {
        BuildError::AccessConfig(AccessConfigError::GlobalSearchRejected) => {}
        other => panic!("expected GlobalSearchRejected, got {other:?}"),
    }

    // The diagnostic string carries the REQ-3415 reference so an
    // operator reading the log can find the spec clause.
    let msg = BuildError::from(AccessConfigError::GlobalSearchRejected).to_string();
    assert!(msg.contains("REQ-3415"), "missing REQ-3415 ref in {msg:?}");
    assert!(msg.contains("global"), "missing 'global' in {msg:?}");

    // And nothing was written.
    assert!(
        !tmp.path().join("c").exists(),
        "rejected build must not have created /c/"
    );
}

/// `[access.backlinks] mode = "global"` is also rejected at build
/// start with the matching REQ-3415 diagnostic.
#[test]
fn test_3415_global_backlinks_rejected_at_build_start() {
    let tmp = TempDir::new().unwrap();
    let salt = URL_SAFE_NO_PAD.encode(b"eng-salt-bl-reject-01");
    let (_alice, pk) = fresh_identity_pair();
    let recipients = mk_recipients(vec![mk_cohort("engineering", &pk, &salt)]);

    let access = AccessConfig {
        backlinks: BacklinksConfig {
            mode: BacklinksMode::Global,
        },
        ..Default::default()
    };
    let err = run_capability_build(
        &build_cfg(tmp.path(), access),
        &recipients,
        &mk_grants(),
        &sample_secret(),
        &signing_key(),
        &sample_pages(),
    )
    .expect_err("global backlinks must be rejected");
    assert!(matches!(
        err,
        BuildError::AccessConfig(AccessConfigError::GlobalBacklinksRejected)
    ));

    let msg = BuildError::from(AccessConfigError::GlobalBacklinksRejected).to_string();
    assert!(msg.contains("REQ-3415"), "missing REQ-3415 ref in {msg:?}");
    assert!(!tmp.path().join("c").exists());
}

/// `[access.search] mode = "per-cohort"` validates (future-compat)
/// but ships with the feature still off in v1 — BUG-019 defers the
/// index format.  This test pins that shipping behaviour so we can
/// tell the day the format lands (at which point this test will
/// flip and signal the migration).
#[test]
fn test_3415_per_cohort_opt_in_validates_but_stays_off_in_v1() {
    let access = AccessConfig {
        search: SearchConfig {
            mode: SearchMode::PerCohort,
        },
        ..Default::default()
    };
    access.validate().expect("per-cohort validates");
    assert!(
        !access.search_ui_enabled(),
        "v1: per-cohort opt-in recorded, but search UI not yet emitted (BUG-019)"
    );

    // The build itself runs to completion.
    let tmp = TempDir::new().unwrap();
    let salt = URL_SAFE_NO_PAD.encode(b"eng-salt-percohort-01");
    let (_alice, pk) = fresh_identity_pair();
    let recipients = mk_recipients(vec![mk_cohort("engineering", &pk, &salt)]);
    let summary = run_capability_build(
        &build_cfg(tmp.path(), access),
        &recipients,
        &mk_grants(),
        &sample_secret(),
        &signing_key(),
        &sample_pages(),
    )
    .expect("per-cohort opt-in build succeeds");
    assert_eq!(summary.pages_encrypted, 1);
}

/// Backlinks filter drops cross-cohort sources and keeps in-cohort
/// ones, both for the single-view form and for the per-cohort form.
/// TEST-3415 acceptance: "Backlinks panel scoped".
#[test]
fn test_3415_backlinks_filter_drops_cross_cohort_sources() {
    // Two cohorts, two sources, one target shared.
    let cohorts = vec![
        CohortScope {
            id: "eng".into(),
            pages_glob: None,
        },
        CohortScope {
            id: "ops".into(),
            pages_glob: None,
        },
    ];
    let pages = vec![
        PageRef {
            slug: "readme".into(),
            explicit_cohorts: vec!["eng".into()],
        },
        PageRef {
            slug: "runbook".into(),
            explicit_cohorts: vec!["ops".into()],
        },
        PageRef {
            slug: "arch".into(),
            explicit_cohorts: vec!["eng".into(), "ops".into()],
        },
    ];
    let ix = CohortIndex::build(&cohorts, &pages).unwrap();
    let raw = vec![
        RawBacklink {
            source_slug: "readme".to_string(),
            payload: "from eng",
        },
        RawBacklink {
            source_slug: "runbook".to_string(),
            payload: "from ops",
        },
    ];

    // Single-view filter: because `arch` is in _both_ cohorts it
    // accepts both backlinks.
    let both = scope_backlinks_for_target("arch", &raw, &ix);
    assert_eq!(both.len(), 2);

    // Per-cohort split: the eng render sees only readme; the ops
    // render sees only runbook.  This is the actual panel shape in
    // capability mode — each cohort's decrypted view shows only
    // its own sources.
    let per = scope_backlinks_per_cohort("arch", &raw, &ix);
    assert_eq!(per.get("eng").unwrap().len(), 1);
    assert_eq!(per.get("eng").unwrap()[0].source_slug, "readme");
    assert_eq!(per.get("ops").unwrap().len(), 1);
    assert_eq!(per.get("ops").unwrap()[0].source_slug, "runbook");
}

/// TOML-level: `[access.search] mode = "global"` parses but fails
/// `validate()`; the build-driver wiring converts it into the same
/// `BuildError::AccessConfig(...)` surface the structured-constructor
/// tests exercise.  This keeps the TOML → error path regression-
/// tested end-to-end.
#[test]
fn test_3415_global_mode_via_toml_is_rejected_equivalently() {
    let src = r#"
        [search]
        mode = "global"
    "#;
    let cfg: AccessConfig = toml::from_str(src).unwrap();
    let err = cfg.validate().unwrap_err();
    assert_eq!(err, AccessConfigError::GlobalSearchRejected);
    // And the build driver surfaces it unchanged.
    let tmp = TempDir::new().unwrap();
    let salt = URL_SAFE_NO_PAD.encode(b"eng-salt-toml-path-01");
    let (_alice, pk) = fresh_identity_pair();
    let recipients = mk_recipients(vec![mk_cohort("engineering", &pk, &salt)]);
    let driver_err = run_capability_build(
        &build_cfg(tmp.path(), cfg),
        &recipients,
        &mk_grants(),
        &sample_secret(),
        &signing_key(),
        &sample_pages(),
    )
    .expect_err("toml-parsed global mode rejected at build");
    assert!(matches!(
        driver_err,
        BuildError::AccessConfig(AccessConfigError::GlobalSearchRejected)
    ));
}

// ─── small walkdir replacement used by the "no search index
// artefact" check.  Kept inline to avoid pulling in `walkdir` as a
// dev-dep when the one call site only needs a shallow recursion. ───

fn walkdir(root: &std::path::Path) -> Vec<std::fs::DirEntry> {
    let mut out = Vec::new();
    let mut stack: Vec<std::path::PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(entry);
            }
        }
    }
    out
}
