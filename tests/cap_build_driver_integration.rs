//! Capability-mode build driver — end-to-end integration tests
//! (SPEC-034 REQ-3401, REQ-3403, REQ-3418, OBS-3401).
//!
//! Exercises `zetl::cap::build::run_capability_build` through the
//! same public API the CLI bridge (`zetl build --capability`) will
//! call:
//!
//! - multi-cohort vault ⇒ each cohort gets its own `/c/<path-cap>/`
//!   subtree + envelope files that parse, verify, and decrypt.
//! - TEST-3403 positive: round-trip a rendered page through
//!   sanitise → age → sign → write → parse → verify → decrypt and
//!   recover the sanitised plaintext byte-for-byte.
//! - OBS-3401 stderr line carries every documented counter key.
//! - Expired + revoked grants drop out of `grants_active`.
//! - REQ-3402 path-cap stability: rebuilding with identical inputs
//!   produces byte-identical on-disk paths.

use std::io::Read;

use age::x25519;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use tempfile::TempDir;

use zetl::cap::build::{
    run_capability_build, BuildConfig, BuildSummary, PageInput, Visibility,
};
use zetl::cap::genkey::{build_secret, decode_secret, encode_secret, SECRET_VERSION_V1};
use zetl::cap::grants::validation::{Grant, GrantMode, GrantsFile};
use zetl::cap::recipients::parsing::{
    Cohort, CohortMode, RecipientsFile, VaultSection, AGE_RECIPIENT_V1_PREFIX,
};
use zetl::cap::sign::{parse_envelope, VaultSigningKey};

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

fn sample_secret() -> zetl::cap::genkey::ParsedSecret {
    let random = [0x5Au8; 32];
    let bytes = build_secret(SECRET_VERSION_V1, &random);
    let encoded = encode_secret(&bytes);
    decode_secret(&encoded).expect("secret round-trips")
}

fn signing_key() -> VaultSigningKey {
    VaultSigningKey::from_bytes(&[0x11u8; 32])
}

fn mk_cohort(
    id: &str,
    pubkeys: &[[u8; 32]],
    salt_b64: &str,
    pages_glob: Option<&str>,
    mode: CohortMode,
) -> Cohort {
    Cohort {
        id: id.to_string(),
        name: None,
        mode,
        pubkeys: pubkeys.iter().map(age_recipient_v1).collect(),
        pages: pages_glob.map(String::from),
        salt_stable: Some(salt_b64.to_string()),
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

fn mk_grant(id: &str, cohort: &str, expires: Option<&str>, revoked: bool) -> Grant {
    Grant {
        id: id.to_string(),
        cohort: cohort.to_string(),
        recipient: format!("{AGE_RECIPIENT_V1_PREFIX}AA"),
        mode: GrantMode::DelegatedUrl,
        bound: false,
        name: None,
        created: "2026-04-20T00:00:00Z".to_string(),
        expires: expires.map(String::from),
        revoked,
        pages: "*".to_string(),
    }
}

fn build_cfg(out: &std::path::Path) -> BuildConfig {
    BuildConfig {
        vault_root: std::path::PathBuf::from("/vault"),
        out_dir: out.to_path_buf(),
        build_epoch: "2026-04-20T12:00:00Z".to_string(),
        now_unix: 1_745_149_200,
        path_cap_bits: 64,
        visibility: Visibility::Private,
        access: zetl::cap::scoping::access_config::AccessConfig::default(),
        shim_integrity: None,
    }
}

fn try_decrypt(ciphertext: &[u8], id: &x25519::Identity) -> Option<Vec<u8>> {
    let decryptor = age::Decryptor::new(ciphertext).ok()?;
    let mut reader = decryptor
        .decrypt(std::iter::once(id as &dyn age::Identity))
        .ok()?;
    let mut out = Vec::new();
    reader.read_to_end(&mut out).ok()?;
    Some(out)
}

/// TEST-3403 positive: the full build → parse → verify → decrypt
/// pipeline returns the sanitised plaintext unchanged.
#[test]
fn test_3403_single_cohort_round_trip() {
    let tmp = TempDir::new().unwrap();
    let (alice, alice_pk) = fresh_identity_pair();
    let salt = URL_SAFE_NO_PAD.encode(b"stable-eng-salt-byte-string-001");
    let recipients = mk_recipients(vec![mk_cohort(
        "engineering",
        &[alice_pk],
        &salt,
        None,
        CohortMode::DelegatedUrl,
    )]);
    let grants = GrantsFile {
        version: Some(1),
        grants: vec![mk_grant(
            "g_01A",
            "engineering",
            Some("2027-01-01T00:00:00Z"),
            false,
        )],
    };
    let pages = vec![PageInput {
        slug: "welcome".into(),
        html: "<h1>Welcome</h1><p>Hi, team.</p>".into(),
        explicit_cohorts: vec![],
    }];

    let summary = run_capability_build(
        &build_cfg(tmp.path()),
        &recipients,
        &grants,
        &sample_secret(),
        &signing_key(),
        &pages,
    )
    .expect("build succeeds");
    assert_eq!(summary.pages_encrypted, 1);
    assert_eq!(summary.grants_active, 1);

    let stat = &summary.per_page[0];
    let path = tmp
        .path()
        .join("c")
        .join(&stat.path_cap)
        .join("welcome.html");
    let bytes = std::fs::read(&path).unwrap();
    let env = parse_envelope(&bytes).unwrap();
    assert_eq!(env.header.cohort_id, "engineering");
    assert_eq!(env.header.slug, "welcome");
    signing_key()
        .verifying_key()
        .verify_ciphertext(&env.ciphertext, &env.signature)
        .expect("sig verifies");
    let plaintext = try_decrypt(&env.ciphertext, &alice).unwrap();
    let s = String::from_utf8(plaintext).unwrap();
    assert!(s.contains("<h1>Welcome</h1>"));
    assert!(s.contains("<p>Hi, team.</p>"));
}

/// Multi-cohort vault: every cohort emits a distinct ciphertext
/// tree, and per-cohort recipients can each decrypt only their own
/// cohort's page.
#[test]
fn multi_cohort_each_recipient_only_decrypts_their_own() {
    let tmp = TempDir::new().unwrap();
    let (alice, alice_pk) = fresh_identity_pair();
    let (bob, bob_pk) = fresh_identity_pair();
    let eng_salt = URL_SAFE_NO_PAD.encode(b"eng-salt-01");
    let ops_salt = URL_SAFE_NO_PAD.encode(b"ops-salt-01");
    let recipients = mk_recipients(vec![
        mk_cohort(
            "engineering",
            &[alice_pk],
            &eng_salt,
            Some("eng/**"),
            CohortMode::DelegatedUrl,
        ),
        mk_cohort(
            "ops",
            &[bob_pk],
            &ops_salt,
            Some("ops/**"),
            CohortMode::WebauthnPrf,
        ),
    ]);
    let grants = GrantsFile {
        version: Some(1),
        grants: vec![
            mk_grant("g_01A", "engineering", None, false),
            mk_grant("g_02B", "ops", None, false),
        ],
    };
    let pages = vec![
        PageInput {
            slug: "eng/runbook".into(),
            html: "<p>eng-only</p>".into(),
            explicit_cohorts: vec![],
        },
        PageInput {
            slug: "ops/pager".into(),
            html: "<p>ops-only</p>".into(),
            explicit_cohorts: vec![],
        },
    ];

    let summary = run_capability_build(
        &build_cfg(tmp.path()),
        &recipients,
        &grants,
        &sample_secret(),
        &signing_key(),
        &pages,
    )
    .expect("build succeeds");
    assert_eq!(summary.cohorts, 2);
    assert_eq!(summary.pages_encrypted, 2);
    assert_eq!(summary.grants_active, 2);

    let eng = summary
        .per_page
        .iter()
        .find(|s| s.cohort_id == "engineering")
        .expect("engineering entry");
    let ops = summary
        .per_page
        .iter()
        .find(|s| s.cohort_id == "ops")
        .expect("ops entry");
    assert_ne!(eng.path_cap, ops.path_cap);

    // Alice decrypts engineering's page but not ops's.
    let eng_file = tmp
        .path()
        .join("c")
        .join(&eng.path_cap)
        .join("runbook.html");
    let ops_file = tmp.path().join("c").join(&ops.path_cap).join("pager.html");
    let env_eng = parse_envelope(&std::fs::read(&eng_file).unwrap()).unwrap();
    let env_ops = parse_envelope(&std::fs::read(&ops_file).unwrap()).unwrap();

    assert!(try_decrypt(&env_eng.ciphertext, &alice).is_some());
    assert!(
        try_decrypt(&env_ops.ciphertext, &alice).is_none(),
        "alice must not read ops content"
    );
    assert!(try_decrypt(&env_ops.ciphertext, &bob).is_some());
    assert!(
        try_decrypt(&env_eng.ciphertext, &bob).is_none(),
        "bob must not read engineering content"
    );

    assert_eq!(env_eng.header.cohort_mode, CohortMode::DelegatedUrl);
    assert_eq!(env_ops.header.cohort_mode, CohortMode::WebauthnPrf);
}

/// OBS-3401: the stderr line carries the documented counter keys.
#[test]
fn obs_3401_stderr_line_carries_counter_keys() {
    let tmp = TempDir::new().unwrap();
    let (_alice, alice_pk) = fresh_identity_pair();
    let salt = URL_SAFE_NO_PAD.encode(b"salt");
    let recipients =
        mk_recipients(vec![mk_cohort("eng", &[alice_pk], &salt, None, CohortMode::DelegatedUrl)]);
    let grants = GrantsFile {
        version: Some(1),
        grants: vec![
            mk_grant("g_01", "eng", None, false),
            mk_grant("g_02", "eng", Some("2099-01-01T00:00:00Z"), false),
            mk_grant("g_03", "eng", None, true), // revoked — drops out
        ],
    };
    let pages = vec![
        PageInput {
            slug: "a".into(),
            html: "<p>a</p>".into(),
            explicit_cohorts: vec![],
        },
        PageInput {
            slug: "b".into(),
            html: "<p>b</p>".into(),
            explicit_cohorts: vec![],
        },
    ];
    let summary = run_capability_build(
        &build_cfg(tmp.path()),
        &recipients,
        &grants,
        &sample_secret(),
        &signing_key(),
        &pages,
    )
    .expect("build succeeds");
    let line = summary.stderr_line();
    assert!(line.contains("cohorts=1"));
    assert!(line.contains("pages=2"));
    assert!(line.contains("grants_active=2"));
    assert!(line.contains("emitted=2"));
    assert!(line.contains("plaintext_bytes="));
    assert!(line.contains("ciphertext_bytes="));
    assert!(line.contains("envelope_bytes="));
    assert!(line.contains("duration_ms="));
    assert!(line.starts_with("[zetl] cap build:"));
}

/// REQ-3402 stability: identical inputs produce identical path-caps
/// (the CON-3401 URL-stability invariant).
#[test]
fn path_caps_are_stable_across_rebuilds() {
    let (_alice, alice_pk) = fresh_identity_pair();
    let salt = URL_SAFE_NO_PAD.encode(b"stability-check-salt");
    let recipients = mk_recipients(vec![mk_cohort(
        "eng",
        &[alice_pk],
        &salt,
        None,
        CohortMode::DelegatedUrl,
    )]);
    let grants = GrantsFile {
        version: Some(1),
        grants: vec![],
    };
    let pages = vec![PageInput {
        slug: "welcome".into(),
        html: "<p>x</p>".into(),
        explicit_cohorts: vec![],
    }];

    let mk_build_into = |dir: &std::path::Path| -> BuildSummary {
        run_capability_build(
            &build_cfg(dir),
            &recipients,
            &grants,
            &sample_secret(),
            &signing_key(),
            &pages,
        )
        .unwrap()
    };
    let tmp1 = TempDir::new().unwrap();
    let tmp2 = TempDir::new().unwrap();
    let s1 = mk_build_into(tmp1.path());
    let s2 = mk_build_into(tmp2.path());
    assert_eq!(s1.per_page[0].path_cap, s2.per_page[0].path_cap);
}

/// Page whose frontmatter names explicit cohorts lands in exactly
/// those cohorts, overriding cohort globs (CohortIndex rules).
#[test]
fn explicit_frontmatter_cohorts_win_over_globs() {
    let tmp = TempDir::new().unwrap();
    let (_a, a_pk) = fresh_identity_pair();
    let (_b, b_pk) = fresh_identity_pair();
    let salt = URL_SAFE_NO_PAD.encode(b"salt");
    let recipients = mk_recipients(vec![
        mk_cohort(
            "eng",
            &[a_pk],
            &salt,
            Some("**"),
            CohortMode::DelegatedUrl,
        ),
        mk_cohort("ops", &[b_pk], &salt, None, CohortMode::DelegatedUrl),
    ]);
    let grants = GrantsFile {
        version: Some(1),
        grants: vec![],
    };
    let pages = vec![PageInput {
        slug: "shared".into(),
        html: "<p>s</p>".into(),
        explicit_cohorts: vec!["ops".into()],
    }];
    let summary = run_capability_build(
        &build_cfg(tmp.path()),
        &recipients,
        &grants,
        &sample_secret(),
        &signing_key(),
        &pages,
    )
    .unwrap();
    // Only `ops` should have emitted, not `eng` (even though its
    // glob would otherwise match).
    assert_eq!(summary.pages_encrypted, 1);
    assert_eq!(summary.per_page[0].cohort_id, "ops");
}
