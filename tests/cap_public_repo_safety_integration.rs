//! Integration tests for the public-repo safety gate (SPEC-034
//! REQ-3423 / ADR-3409, TEST-3423).
//!
//! Covers:
//!
//!   * Explicit `[vault] visibility = "public"` declaration engages
//!     the gate regardless of git origin.
//!   * The git-origin heuristic flags `github.com`, `gitlab.com`,
//!     `codeberg.org`, etc. as public; a self-hosted host stays
//!     private.
//!   * When the gate engages, `decide_grants_source` refuses without
//!     `[access] grants_file_external`.
//!   * When the gate engages AND the external path points inside the
//!     repo, the decision is refused — the safety depends on the
//!     file not being tracked by git.
//!   * The committed-stub schema carries ONLY cohort ids + stable
//!     salts; recipient pubkeys / grant metadata / names never make
//!     it into the committed artefact.

use std::path::{Path, PathBuf};

use zetl::cap::public_repo::{
    build_committed_stub, decide_grants_source, origin_host, parse_committed_stub,
    parse_config_lens, resolve_visibility, serialise_committed_stub, AccessConfig,
    GitHeuristicOpts, GrantsSource, PolicyError, ResolvedVisibility, Visibility, VisibilityDecl,
    VisibilitySource, ZetlConfigLens,
};
use zetl::cap::recipients::parsing::{Cohort, CohortMode, RecipientsFile, VaultSection};

fn sample_recipients() -> RecipientsFile {
    RecipientsFile {
        version: 1,
        vault: VaultSection {
            signing_pubkey: "ed25519:Aabc_-".to_string(),
        },
        cohorts: vec![
            Cohort {
                id: "engineering".to_string(),
                name: Some("Engineering".to_string()),
                mode: CohortMode::DelegatedUrl,
                pubkeys: vec!["age-recipient-v1:AliceKeyBase64Url".to_string()],
                pages: None,
                salt_stable: Some("eng-stable-salt".to_string()),
                salt_rotated: Some("eng-rotated-salt".to_string()),
                last_rotated: Some("2026-03-01T00:00:00Z".to_string()),
            },
            Cohort {
                id: "ops".to_string(),
                name: Some("Ops".to_string()),
                mode: CohortMode::DelegatedUrl,
                pubkeys: vec!["age-recipient-v1:BobKeyBase64Url".to_string()],
                pages: None,
                salt_stable: None,
                salt_rotated: None,
                last_rotated: None,
            },
        ],
    }
}

// ─── TEST-3423 canonical scenarios ────────────────────────────────────

#[test]
fn test_3423_explicit_public_config_refuses_without_external_path() {
    let lens = parse_config_lens(
        r#"
        [vault]
        visibility = "public"
    "#,
    )
    .expect("lens parses");

    let resolved = resolve_visibility(&lens, None, &GitHeuristicOpts::default());
    assert_eq!(resolved.visibility, Visibility::Public);
    assert_eq!(resolved.source, VisibilitySource::ExplicitConfig);

    let err = decide_grants_source(
        &resolved,
        &AccessConfig::default(),
        Path::new("/tmp/wiki"),
        Path::new("/tmp/wiki/.zetl/caps/grants.toml"),
    )
    .unwrap_err();
    assert_eq!(err, PolicyError::MissingExternalPath);
}

#[test]
fn test_3423_github_origin_flags_as_public_and_refuses() {
    let lens = ZetlConfigLens::default();
    let resolved = resolve_visibility(
        &lens,
        Some("git@github.com:acme/wiki.git"),
        &GitHeuristicOpts::default(),
    );
    assert_eq!(resolved.visibility, Visibility::Public);
    assert!(matches!(resolved.source, VisibilitySource::HeuristicHost(ref h) if h == "github.com"));

    let err = decide_grants_source(
        &resolved,
        &AccessConfig::default(),
        Path::new("/tmp/wiki"),
        Path::new("/tmp/wiki/.zetl/caps/grants.toml"),
    )
    .unwrap_err();
    assert_eq!(err, PolicyError::MissingExternalPath);
}

#[test]
fn test_3423_gitlab_codeberg_bitbucket_srht_all_flag_public() {
    let opts = GitHeuristicOpts::default();
    for url in [
        "https://gitlab.com/org/repo.git",
        "git@codeberg.org:org/repo.git",
        "https://bitbucket.org/org/repo",
        "git@git.sr.ht:~user/repo",
    ] {
        let r = resolve_visibility(&ZetlConfigLens::default(), Some(url), &opts);
        assert_eq!(
            r.visibility,
            Visibility::Public,
            "expected {url} to engage the gate"
        );
    }
}

#[test]
fn test_3423_self_hosted_origin_does_not_engage_gate() {
    let lens = ZetlConfigLens::default();
    let resolved = resolve_visibility(
        &lens,
        Some("git@git.acme.internal:team/wiki.git"),
        &GitHeuristicOpts::default(),
    );
    assert_eq!(resolved.visibility, Visibility::Private);

    // Private: in-repo grants path is returned as the source. The
    // filesystem never needs to have the file; that is for the
    // build driver to validate.
    let decision = decide_grants_source(
        &resolved,
        &AccessConfig::default(),
        Path::new("/tmp/wiki"),
        Path::new("/tmp/wiki/.zetl/caps/grants.toml"),
    )
    .unwrap();
    assert!(matches!(decision, GrantsSource::InRepo { .. }));
}

#[test]
fn test_3423_private_token_override_flips_github_to_private() {
    let lens = ZetlConfigLens::default();
    let opts = GitHeuristicOpts {
        private_tokens: vec![("github.com".to_string(), true)],
        ..Default::default()
    };
    let resolved = resolve_visibility(&lens, Some("git@github.com:acme/wiki.git"), &opts);
    assert_eq!(resolved.visibility, Visibility::Private);
}

#[test]
fn test_3423_public_with_external_path_inside_repo_refuses() {
    let resolved = ResolvedVisibility {
        visibility: Visibility::Public,
        source: VisibilitySource::ExplicitConfig,
    };
    // The external file lives inside the repo tree — precisely the
    // misconfiguration the gate must catch, since committing the
    // file to git defeats the entire safety.
    let access = AccessConfig {
        grants_file_external: Some("/tmp/wiki/secrets/grants.toml".to_string()),
        split_key: None,
    };
    let err = decide_grants_source(
        &resolved,
        &access,
        Path::new("/tmp/wiki"),
        Path::new("/tmp/wiki/.zetl/caps/grants.toml"),
    )
    .unwrap_err();
    assert!(matches!(err, PolicyError::ExternalPathInsideRepo { .. }));
}

#[test]
fn test_3423_public_with_external_path_outside_repo_succeeds() {
    let resolved = ResolvedVisibility {
        visibility: Visibility::Public,
        source: VisibilitySource::ExplicitConfig,
    };
    let external = "/home/op/.config/zetl/acme-wiki/grants.toml";
    let access = AccessConfig {
        grants_file_external: Some(external.to_string()),
        split_key: None,
    };
    let decision = decide_grants_source(
        &resolved,
        &access,
        Path::new("/tmp/wiki"),
        Path::new("/tmp/wiki/.zetl/caps/grants.toml"),
    )
    .unwrap();
    assert_eq!(
        decision,
        GrantsSource::External {
            path: PathBuf::from(external)
        }
    );
}

#[test]
fn test_3423_committed_stub_carries_no_reader_identifying_data() {
    let recipients = sample_recipients();
    let stub = build_committed_stub(&recipients);
    let body = serialise_committed_stub(&stub).expect("stub serialises");

    // Critical negatives: if any of these fire, the stub has leaked
    // the very data the REQ-3423 gate exists to protect.
    for forbidden in [
        "age-recipient-v1",  // recipient pubkey prefix
        "AliceKeyBase64Url", // recipient payload
        "BobKeyBase64Url",   // recipient payload
        "signing_pubkey",    // vault signing key — belongs in recipients.toml only
        "Engineering",       // cohort display name
        "last_rotated",      // audit timestamp
        "eng-rotated-salt",  // rotated salt is operator-private
    ] {
        assert!(
            !body.contains(forbidden),
            "committed stub leaked {forbidden:?} — full body:\n{body}"
        );
    }

    // Positives: the stub must carry cohort ids + stable path-cap
    // salts so URLs stay reproducible across operator machines.
    assert!(
        body.contains("engineering"),
        "stub missing cohort id: {body}"
    );
    assert!(
        body.contains("eng-stable-salt"),
        "stub missing stable salt: {body}"
    );
    assert!(body.contains("ops"), "stub missing ops cohort: {body}");
}

#[test]
fn test_3423_committed_stub_round_trips_via_parser() {
    let stub = build_committed_stub(&sample_recipients());
    let body = serialise_committed_stub(&stub).unwrap();
    let back = parse_committed_stub(&body).unwrap();
    assert_eq!(back, stub);
}

// ─── Surface-level sanity ─────────────────────────────────────────────

#[test]
fn origin_host_parses_github_scp_style() {
    assert_eq!(
        origin_host("git@github.com:acme/wiki.git"),
        Some("github.com")
    );
}

#[test]
fn parse_config_lens_rejects_unknown_visibility_word() {
    let err = parse_config_lens(
        r#"
        [vault]
        visibility = "open"
    "#,
    )
    .unwrap_err();
    assert!(err.0.contains("open") || err.0.contains("variant"));
}

#[test]
fn visibility_decl_from_into_visibility_is_total() {
    assert_eq!(Visibility::from(VisibilityDecl::Public), Visibility::Public);
    assert_eq!(
        Visibility::from(VisibilityDecl::Private),
        Visibility::Private
    );
}
