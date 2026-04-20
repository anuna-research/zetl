//! Integration + property tests for SPEC-034 pure-core modules.
//!
//! Covers:
//!
//! - CON-3401 path-cap derivation is deterministic and separates every
//!   input axis (cohort_secret, cohort_salt_stable, cohort_id, slug,
//!   path_cap_bits). Property-based: any change to any axis changes
//!   the output with overwhelming probability.
//! - CON-3401 URL render/parse round-trips for all three modes.
//! - REQ-3409 / CON-3403 recipients.toml end-to-end parse.
//! - Cross-file invariants: `grants.validate(recipients.cohort_ids())`
//!   catches dangling cohort references.
//! - Scoping index assigns pages to cohorts correctly across
//!   explicit-frontmatter + glob-fallback paths.

use std::collections::BTreeSet;

use proptest::prelude::*;

use zetl::cap::derivation::{
    derive_hardened_identity_seed, derive_path_cap, derive_tofu_wrap_key, DerivationError,
    PATH_CAP_DEFAULT_BITS, PATH_CAP_MAX_BITS, PATH_CAP_MIN_BITS,
};
use zetl::cap::grants::validation::{Grant, GrantMode, GrantsFile, ValidationError};
use zetl::cap::recipients::parsing::{
    Cohort, CohortMode, RecipientsFile, VaultSection, AGE_RECIPIENT_V1_PREFIX,
};
use zetl::cap::scoping::cohort_index::{CohortIndex, CohortScope, PageRef};
use zetl::cap::url_format::{CapUrl, CapUrlMode, ParseError};

const VALID_KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

// ─── CON-3401 derivation properties ───────────────────────────────────

proptest! {
    #[test]
    fn path_cap_deterministic(
        secret in prop::collection::vec(any::<u8>(), 1..64),
        salt in prop::collection::vec(any::<u8>(), 1..32),
        cohort in "[a-z][a-z0-9]{0,15}",
        slug in "[a-z0-9/-]{1,40}",
    ) {
        let a = derive_path_cap(&secret, &salt, &cohort, &slug, PATH_CAP_DEFAULT_BITS).unwrap();
        let b = derive_path_cap(&secret, &salt, &cohort, &slug, PATH_CAP_DEFAULT_BITS).unwrap();
        prop_assert_eq!(a, b);
    }

    #[test]
    fn path_cap_separates_slug(
        secret in prop::collection::vec(any::<u8>(), 1..64),
        salt in prop::collection::vec(any::<u8>(), 1..32),
        cohort in "[a-z][a-z0-9]{0,15}",
        slug_a in "[a-z0-9/-]{1,40}",
        slug_b in "[a-z0-9/-]{1,40}",
    ) {
        prop_assume!(slug_a != slug_b);
        let a = derive_path_cap(&secret, &salt, &cohort, &slug_a, PATH_CAP_DEFAULT_BITS).unwrap();
        let b = derive_path_cap(&secret, &salt, &cohort, &slug_b, PATH_CAP_DEFAULT_BITS).unwrap();
        prop_assert_ne!(a, b);
    }

    #[test]
    fn path_cap_separates_cohort(
        secret in prop::collection::vec(any::<u8>(), 1..64),
        salt in prop::collection::vec(any::<u8>(), 1..32),
        cohort_a in "[a-z][a-z0-9]{0,15}",
        cohort_b in "[a-z][a-z0-9]{0,15}",
        slug in "[a-z0-9/-]{1,40}",
    ) {
        prop_assume!(cohort_a != cohort_b);
        let a = derive_path_cap(&secret, &salt, &cohort_a, &slug, PATH_CAP_DEFAULT_BITS).unwrap();
        let b = derive_path_cap(&secret, &salt, &cohort_b, &slug, PATH_CAP_DEFAULT_BITS).unwrap();
        prop_assert_ne!(a, b);
    }

    #[test]
    fn path_cap_separates_secret(
        secret_a in prop::collection::vec(any::<u8>(), 1..64),
        secret_b in prop::collection::vec(any::<u8>(), 1..64),
        salt in prop::collection::vec(any::<u8>(), 1..32),
        cohort in "[a-z][a-z0-9]{0,15}",
        slug in "[a-z0-9/-]{1,40}",
    ) {
        prop_assume!(secret_a != secret_b);
        let a = derive_path_cap(&secret_a, &salt, &cohort, &slug, PATH_CAP_DEFAULT_BITS).unwrap();
        let b = derive_path_cap(&secret_b, &salt, &cohort, &slug, PATH_CAP_DEFAULT_BITS).unwrap();
        prop_assert_ne!(a, b);
    }

    #[test]
    fn path_cap_separates_stable_salt(
        secret in prop::collection::vec(any::<u8>(), 1..64),
        salt_a in prop::collection::vec(any::<u8>(), 1..32),
        salt_b in prop::collection::vec(any::<u8>(), 1..32),
        cohort in "[a-z][a-z0-9]{0,15}",
        slug in "[a-z0-9/-]{1,40}",
    ) {
        prop_assume!(salt_a != salt_b);
        let a = derive_path_cap(&secret, &salt_a, &cohort, &slug, PATH_CAP_DEFAULT_BITS).unwrap();
        let b = derive_path_cap(&secret, &salt_b, &cohort, &slug, PATH_CAP_DEFAULT_BITS).unwrap();
        prop_assert_ne!(a, b);
    }

    #[test]
    fn path_cap_width_controls_length(
        secret in prop::collection::vec(any::<u8>(), 1..32),
        salt in prop::collection::vec(any::<u8>(), 1..16),
        bits_choice in 0u32..11u32,  // 6 byte-aligned widths in [48, 128]
    ) {
        let bits = 48 + bits_choice * 8; // 48, 56, 64, ..., 128
        let out = derive_path_cap(&secret, &salt, "eng", "a", bits).unwrap();
        // Crockford base32: ceil(bits / 5) chars
        let expected_len = ((bits as usize) + 4) / 5;
        prop_assert_eq!(out.len(), expected_len);
    }

    #[test]
    fn wrap_key_deterministic(
        prf in prop::collection::vec(any::<u8>(), 1..128),
    ) {
        let a = derive_tofu_wrap_key(&prf);
        let b = derive_tofu_wrap_key(&prf);
        prop_assert_eq!(a, b);
    }

    #[test]
    fn wrap_key_distinct_from_hardened_seed(
        prf in prop::collection::vec(any::<u8>(), 1..128),
    ) {
        let w = derive_tofu_wrap_key(&prf);
        let h = derive_hardened_identity_seed(&prf);
        // Different `info` strings must produce different outputs.
        prop_assert_ne!(w, h);
    }
}

#[test]
fn path_cap_rejects_widths_outside_spec_range() {
    for bits in [0, 8, 40, 47, 49, 129, 256] {
        assert!(matches!(
            derive_path_cap(b"s", b"sa", "eng", "a", bits),
            Err(DerivationError::PathCapWidth { .. })
        ));
    }
}

#[test]
fn path_cap_min_and_max_widths_are_spec_exact() {
    // REQ-3402 + NFR-3401 acceptance: min 48 bits, default 64, max 128.
    assert!(derive_path_cap(b"s", b"sa", "eng", "a", PATH_CAP_MIN_BITS).is_ok());
    assert!(derive_path_cap(b"s", b"sa", "eng", "a", PATH_CAP_DEFAULT_BITS).is_ok());
    assert!(derive_path_cap(b"s", b"sa", "eng", "a", PATH_CAP_MAX_BITS).is_ok());
}

// ─── CON-3401 URL round-trips ────────────────────────────────────────

proptest! {
    #[test]
    fn delegated_url_round_trips(
        path_cap in "[0-9A-HJKMNP-TV-Z]{13}",
        slug in "[a-z0-9][a-z0-9/-]{0,50}",
    ) {
        prop_assume!(!slug.ends_with('/') && !slug.contains("//"));
        let url = CapUrl::render_delegated(
            "https", "wiki.example", &path_cap, &slug, VALID_KEY,
        ).unwrap();
        let parsed = CapUrl::parse(&url).unwrap();
        prop_assert_eq!(parsed.path_cap, path_cap);
        prop_assert_eq!(&parsed.slug, slug.strip_suffix(".html").unwrap_or(&slug));
        match parsed.mode {
            CapUrlMode::Delegated { priv_a_b64url } => {
                prop_assert_eq!(priv_a_b64url, VALID_KEY);
            }
            other => prop_assert!(false, "expected delegated, got {other:?}"),
        }
    }

    #[test]
    fn hardened_url_round_trips(
        path_cap in "[0-9A-HJKMNP-TV-Z]{13}",
        slug in "[a-z0-9][a-z0-9/-]{0,50}",
    ) {
        prop_assume!(!slug.ends_with('/') && !slug.contains("//"));
        let url = CapUrl::render_hardened("https", "wiki.example", &path_cap, &slug).unwrap();
        let parsed = CapUrl::parse(&url).unwrap();
        prop_assert_eq!(parsed.mode, CapUrlMode::Hardened);
    }
}

#[test]
fn canonical_url_shape_matches_con_3401() {
    // CON-3401 example (synthesised): shape must exactly match the
    // spec's ABNF so shims written against other implementations can
    // round-trip against zetl-emitted URLs.
    let url = CapUrl::render_delegated(
        "https", "wiki.example", "1234567890ABC", "team/runbook", VALID_KEY,
    )
    .unwrap();
    assert_eq!(
        url,
        format!("https://wiki.example/c/1234567890ABC/team/runbook.html#k={VALID_KEY}")
    );
}

#[test]
fn url_parser_rejects_non_crockford_path_cap() {
    // Lowercase letters are NOT in the Crockford alphabet (encoder
    // emits uppercase; parsing must enforce the same invariant).
    assert!(matches!(
        CapUrl::parse("https://x/c/abcdefghijklm/a.html"),
        Err(ParseError::PathCapCharset('a'))
    ));
}

// ─── REQ-3409 recipients.toml end-to-end ─────────────────────────────

#[test]
fn recipients_file_round_trips_with_both_modes() {
    let toml_in = format!(r#"
version = 1

[vault]
signing_pubkey = "ed25519:abcDEF-_"

[[cohort]]
id = "engineering"
name = "Engineering"
mode = "delegated-url"
pubkeys = [
    "{AGE_RECIPIENT_V1_PREFIX}alice123",
    "{AGE_RECIPIENT_V1_PREFIX}bob456_",
]

[[cohort]]
id = "ops"
name = "Operations"
mode = "webauthn-prf"
pubkeys = []
"#);

    let f = RecipientsFile::parse(&toml_in).unwrap();
    assert_eq!(f.cohorts.len(), 2);
    assert_eq!(f.cohorts[0].mode, CohortMode::DelegatedUrl);
    assert_eq!(f.cohorts[1].mode, CohortMode::WebauthnPrf);

    let ids = f.cohort_ids();
    assert!(ids.contains("engineering"));
    assert!(ids.contains("ops"));
}

#[test]
fn recipients_file_rejects_non_age_recipient_v1() {
    // REQ-3409 restricts to age-recipient-v1. Other recipient types
    // are accepted by `age` itself but disallowed here.
    let toml_in = r#"
version = 1

[vault]
signing_pubkey = "ed25519:abc"

[[cohort]]
id = "eng"
mode = "delegated-url"
pubkeys = ["age-scrypt:deadbeef"]
"#;
    assert!(RecipientsFile::parse(toml_in).is_err());
}

// ─── Cross-file invariant: grants.cohort must exist ──────────────────

#[test]
fn grants_references_unknown_cohort_detected() {
    let recipients: RecipientsFile = RecipientsFile {
        version: 1,
        vault: VaultSection {
            signing_pubkey: "ed25519:abc".to_string(),
        },
        cohorts: vec![Cohort {
            id: "engineering".to_string(),
            name: None,
            mode: CohortMode::DelegatedUrl,
            pubkeys: vec![],
            pages: None,
        }],
    };
    recipients.validate().unwrap();

    let grants = GrantsFile {
        version: Some(1),
        grants: vec![Grant {
            id: "g1".to_string(),
            cohort: "marketing".to_string(),   // ← not in recipients.toml
            recipient: format!("{AGE_RECIPIENT_V1_PREFIX}abc123"),
            mode: GrantMode::DelegatedUrl,
            bound: false,
            name: None,
            created: "2026-04-20T00:00:00Z".to_string(),
            expires: None,
            revoked: false,
            pages: "*".to_string(),
        }],
    };

    let err = grants.validate(&recipients.cohort_ids()).unwrap_err();
    assert!(matches!(err, ValidationError::UnknownCohort { .. }));
}

#[test]
fn grants_validation_passes_with_matching_cohort() {
    let mut ids = BTreeSet::new();
    ids.insert("engineering".to_string());
    let grants = GrantsFile {
        version: Some(1),
        grants: vec![Grant {
            id: "g1".to_string(),
            cohort: "engineering".to_string(),
            recipient: format!("{AGE_RECIPIENT_V1_PREFIX}abc123"),
            mode: GrantMode::DelegatedUrl,
            bound: false,
            name: None,
            created: "2026-04-20T00:00:00Z".to_string(),
            expires: Some("2026-10-20T00:00:00Z".to_string()),
            revoked: false,
            pages: "*".to_string(),
        }],
    };
    grants.validate(&ids).unwrap();
}

// ─── Scoping: cohort-to-page assignment ──────────────────────────────

#[test]
fn scoping_frontmatter_trumps_glob() {
    let cohorts = vec![
        CohortScope { id: "eng".to_string(), pages_glob: Some("**".to_string()) },
        CohortScope { id: "ops".to_string(), pages_glob: None },
    ];
    let pages = vec![
        // Opts out of "eng" despite glob by naming only "ops".
        PageRef { slug: "shared/secret".to_string(), explicit_cohorts: vec!["ops".to_string()] },
        // No explicit — falls back to the glob (eng matches, ops is None = all).
        PageRef { slug: "readme".to_string(), explicit_cohorts: vec![] },
    ];
    let ix = CohortIndex::build(&cohorts, &pages).unwrap();
    assert_eq!(ix.cohorts_of("shared/secret"), &["ops".to_string()]);
    assert_eq!(
        ix.cohorts_of("readme"),
        &["eng".to_string(), "ops".to_string()]
    );
}

#[test]
fn scoping_unknown_explicit_cohort_fails() {
    let cohorts = vec![CohortScope { id: "eng".to_string(), pages_glob: None }];
    let pages = vec![PageRef {
        slug: "p".to_string(),
        explicit_cohorts: vec!["ghost".to_string()],
    }];
    assert!(CohortIndex::build(&cohorts, &pages).is_err());
}
