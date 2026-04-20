//! TEST-3413: external-link referrer scrubbing (SPEC-034 REQ-3413 /
//! OBS-3407).
//!
//! End-to-end: feed pages with mixed internal/external `<a>` tags
//! through the capability build driver, decrypt the emitted
//! envelope, and assert:
//!
//! 1. External `<a href="https://…">` carries `rel="noopener
//!    noreferrer"` in the ciphertext plaintext.
//! 2. Internal `<a href="/docs/…">` is returned byte-identical — no
//!    rel attribute added (operator's same-site Referer analytics
//!    are preserved).
//! 3. The capability HTML shell carries `<meta name="referrer"
//!    content="no-referrer">` so the document-wide referrer policy
//!    reaches browsers that don't honour per-link rel.
//! 4. Operator opt-out: setting `[access] rel_noreferrer = false`
//!    disables the per-link rewrite. The meta tag in the shell
//!    remains — opt-out of the rewrite is a documented weakening of
//!    path-cap privacy; the shell's document-default is a separate,
//!    always-on defence.

use std::io::Read;

use age::x25519;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use tempfile::TempDir;

use zetl::cap::build::{run_capability_build, BuildConfig, PageInput, Visibility};
use zetl::cap::genkey::{build_secret, decode_secret, encode_secret, SECRET_VERSION_V1};
use zetl::cap::grants::validation::{Grant, GrantMode, GrantsFile};
use zetl::cap::html_shell::{self, REFERRER_META};
use zetl::cap::recipients::parsing::{
    Cohort, CohortMode, RecipientsFile, VaultSection, AGE_RECIPIENT_V1_PREFIX,
};
use zetl::cap::scoping::access_config::AccessConfig;
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

fn signing_key() -> VaultSigningKey {
    VaultSigningKey::from_bytes(&[0x17u8; 32])
}

fn sample_secret() -> zetl::cap::genkey::ParsedSecret {
    let random = [0xA1u8; 32];
    let bytes = build_secret(SECRET_VERSION_V1, &random);
    let encoded = encode_secret(&bytes);
    decode_secret(&encoded).expect("secret round-trips")
}

fn mk_recipients(cohort_id: &str, pubkeys: &[[u8; 32]], salt_b64: &str) -> RecipientsFile {
    RecipientsFile {
        version: 1,
        vault: VaultSection {
            signing_pubkey: format!(
                "ed25519:{}",
                URL_SAFE_NO_PAD.encode(signing_key().verifying_key().to_bytes())
            ),
        },
        cohorts: vec![Cohort {
            id: cohort_id.to_string(),
            name: None,
            mode: CohortMode::DelegatedUrl,
            pubkeys: pubkeys.iter().map(age_recipient_v1).collect(),
            pages: None,
            salt_stable: Some(salt_b64.to_string()),
            salt_rotated: None,
            last_rotated: None,
        }],
    }
}

fn mk_grants(cohort: &str) -> GrantsFile {
    GrantsFile {
        version: Some(1),
        grants: vec![Grant {
            id: "g_01TEST".to_string(),
            cohort: cohort.to_string(),
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

fn build_cfg(out: &std::path::Path) -> BuildConfig {
    BuildConfig {
        vault_root: std::path::PathBuf::from("/vault"),
        out_dir: out.to_path_buf(),
        build_epoch: "2026-04-20T12:00:00Z".to_string(),
        now_unix: 1_745_149_200,
        path_cap_bits: 64,
        visibility: Visibility::Private,
        access: AccessConfig::default(),
        shim_integrity: None,
        enroll_integrity: None,
    }
}

fn try_decrypt(ciphertext: &[u8], id: &x25519::Identity) -> String {
    let decryptor = age::Decryptor::new(ciphertext).expect("age header parses");
    let mut reader = decryptor
        .decrypt(std::iter::once(id as &dyn age::Identity))
        .expect("decryption succeeds");
    let mut out = String::new();
    reader
        .read_to_string(&mut out)
        .expect("valid utf-8 plaintext");
    out
}

const MIXED_LINKS_PAGE: &str = r##"<h1>Links</h1>
<p>Internal: <a href="/docs/welcome">welcome</a>.</p>
<p>Relative: <a href="other.html">other</a>.</p>
<p>Anchor: <a href="#section">top</a>.</p>
<p>External: <a href="https://example.com/tracker">external</a>.</p>
<p>Mail: <a href="mailto:someone@example.com">mail</a>.</p>"##;

/// TEST-3413 core assertion: decrypt a page with mixed internal and
/// external links. External `<a>` gets `rel="noopener noreferrer"`;
/// every internal link is byte-identical to the sanitiser output.
#[test]
fn test_3413_external_links_get_rel_internal_links_unchanged() {
    let tmp = TempDir::new().unwrap();
    let (alice, alice_pk) = fresh_identity_pair();
    let salt = URL_SAFE_NO_PAD.encode(b"referrer-scrub-salt-001");
    let recipients = mk_recipients("engineering", &[alice_pk], &salt);
    let pages = vec![PageInput {
        slug: "links".into(),
        html: MIXED_LINKS_PAGE.to_string(),
        explicit_cohorts: vec![],
    }];

    let summary = run_capability_build(
        &build_cfg(tmp.path()),
        &recipients,
        &mk_grants("engineering"),
        &sample_secret(),
        &signing_key(),
        &pages,
    )
    .expect("build succeeds");
    assert_eq!(summary.pages_encrypted, 1);

    let stat = &summary.per_page[0];
    let path = tmp.path().join("c").join(&stat.path_cap).join("links.html");
    let bytes = std::fs::read(&path).unwrap();
    let env = parse_envelope(&bytes).unwrap();
    let plaintext = try_decrypt(&env.ciphertext, &alice);

    // External links rewritten with both tokens.
    assert!(
        plaintext.contains("<a href=\"https://example.com/tracker\" rel=\"noopener noreferrer\">"),
        "external https link missing rel, got:\n{plaintext}"
    );
    assert!(
        plaintext.contains("<a href=\"mailto:someone@example.com\" rel=\"noopener noreferrer\">"),
        "external mailto link missing rel, got:\n{plaintext}"
    );

    // Internal links unchanged (no rel attribute anywhere near them).
    assert!(
        plaintext.contains("<a href=\"/docs/welcome\">welcome</a>"),
        "internal root-relative link must not be rewritten, got:\n{plaintext}"
    );
    assert!(
        plaintext.contains("<a href=\"other.html\">other</a>"),
        "internal relative link must not be rewritten, got:\n{plaintext}"
    );
    assert!(
        plaintext.contains("<a href=\"#section\">top</a>"),
        "internal anchor link must not be rewritten, got:\n{plaintext}"
    );

    // Canary: the rewritten external rel must not have bled onto
    // internal links via a greedy substitution.
    let internal_rel_hits = plaintext
        .match_indices("<a href=\"/docs/welcome\" rel=")
        .count()
        + plaintext
            .match_indices("<a href=\"other.html\" rel=")
            .count()
        + plaintext.match_indices("<a href=\"#section\" rel=").count();
    assert_eq!(
        internal_rel_hits, 0,
        "no internal link may carry rel; got:\n{plaintext}"
    );
}

/// TEST-3413 opt-out: `[access] rel_noreferrer = false` skips the
/// per-link rewrite. The shell's `<meta name="referrer">` stays as a
/// document-level default — only the rel rewrite is opt-out-able.
#[test]
fn test_3413_opt_out_skips_per_link_rewrite() {
    let tmp = TempDir::new().unwrap();
    let (alice, alice_pk) = fresh_identity_pair();
    let salt = URL_SAFE_NO_PAD.encode(b"referrer-opt-out-salt-001");
    let recipients = mk_recipients("engineering", &[alice_pk], &salt);
    let pages = vec![PageInput {
        slug: "links".into(),
        html: "<p><a href=\"https://example.com\">x</a></p>".into(),
        explicit_cohorts: vec![],
    }];

    let mut cfg = build_cfg(tmp.path());
    cfg.access.rel_noreferrer = false;
    let summary = run_capability_build(
        &cfg,
        &recipients,
        &mk_grants("engineering"),
        &sample_secret(),
        &signing_key(),
        &pages,
    )
    .expect("opt-out build succeeds");

    let stat = &summary.per_page[0];
    let path = tmp.path().join("c").join(&stat.path_cap).join("links.html");
    let bytes = std::fs::read(&path).unwrap();
    let env = parse_envelope(&bytes).unwrap();
    let plaintext = try_decrypt(&env.ciphertext, &alice);

    assert!(
        !plaintext.contains("rel=\"noopener noreferrer\""),
        "rel_noreferrer=false must skip per-link rewrite, got:\n{plaintext}"
    );
    assert!(
        plaintext.contains("<a href=\"https://example.com\">x</a>"),
        "external href must still render, got:\n{plaintext}"
    );
}

/// TEST-3413 shell canary: the capability HTML shell carries the
/// document-wide `<meta name="referrer" content="no-referrer">` tag
/// so browsers enforce no-referrer even for links we failed to
/// rewrite (or for operator opt-out).
#[test]
fn test_3413_shell_carries_referrer_no_referrer_meta() {
    let tmp = TempDir::new().unwrap();
    let (_alice, alice_pk) = fresh_identity_pair();
    let salt = URL_SAFE_NO_PAD.encode(b"referrer-shell-salt-001");
    let recipients = mk_recipients("engineering", &[alice_pk], &salt);
    let pages = vec![PageInput {
        slug: "welcome".into(),
        html: "<p>x</p>".into(),
        explicit_cohorts: vec![],
    }];

    let mut cfg = build_cfg(tmp.path());
    let sri = "sha384-ReferrerShellCanaryHash000000000000000000000000000000000=";
    cfg.shim_integrity = Some(sri.to_string());
    run_capability_build(
        &cfg,
        &recipients,
        &mk_grants("engineering"),
        &sample_secret(),
        &signing_key(),
        &pages,
    )
    .expect("shell build succeeds");

    let shell_path = tmp
        .path()
        .join("_zetl")
        .join(html_shell::CAPABILITY_SHELL_FILENAME);
    let shell = std::fs::read_to_string(&shell_path).expect("shell emitted");
    assert!(
        shell.contains(REFERRER_META),
        "shell missing referrer meta, got:\n{shell}"
    );
    // Spec-pinned byte shape — CI greps depend on this exact line.
    assert!(shell.contains("<meta name=\"referrer\" content=\"no-referrer\">"));
}

/// TEST-3413 shell stays private even when per-link rewrite is
/// opted out. The opt-out applies to the rel rewrite only; the
/// document-level meta tag is a separate, always-on defence.
#[test]
fn test_3413_shell_meta_present_even_with_opt_out() {
    let tmp = TempDir::new().unwrap();
    let (_alice, alice_pk) = fresh_identity_pair();
    let salt = URL_SAFE_NO_PAD.encode(b"referrer-meta-under-opt-out-001");
    let recipients = mk_recipients("engineering", &[alice_pk], &salt);
    let pages = vec![PageInput {
        slug: "welcome".into(),
        html: "<p>x</p>".into(),
        explicit_cohorts: vec![],
    }];

    let mut cfg = build_cfg(tmp.path());
    cfg.access.rel_noreferrer = false;
    let sri = "sha384-ReferrerShellOptOutCanaryHash0000000000000000000000000=";
    cfg.shim_integrity = Some(sri.to_string());
    run_capability_build(
        &cfg,
        &recipients,
        &mk_grants("engineering"),
        &sample_secret(),
        &signing_key(),
        &pages,
    )
    .expect("build succeeds with opt-out");

    let shell = std::fs::read_to_string(
        tmp.path()
            .join("_zetl")
            .join(html_shell::CAPABILITY_SHELL_FILENAME),
    )
    .unwrap();
    assert!(shell.contains(REFERRER_META));
}
