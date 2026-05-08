//! `.zetl/config.toml` lens for SPEC-038. Pure parser: hands the shell
//! a fully-validated configuration tree or a structured error naming
//! the offending key.
//!
//! Recognised sections:
//!
//!   * `[feed]`               — root feed settings (REQ-3801, ADR-3801)
//!   * `[[feed.scopes]]`      — per-scope subscription catalog (REQ-3813)
//!   * `[feed.changelog]`     — AST-backed changelog feed (REQ-3816, REQ-3817)
//!   * `[[capability_cohorts]]` — cohort feeds (REQ-3829, NFR-3809)
//!   * `[[subscriptions]]`    — inbound feed subscriptions (CON-3811, ADR-3803)
//!   * `[wiki].self_license`  — receiving-vault license (REQ-3820)
//!   * `[wiki].is_commercial` — receiving-vault commercial flag (REQ-3820)
//!
//! Unknown top-level keys are accepted (the wider config has other
//! consumers); unknown keys *inside* the SPEC-038 sections are rejected
//! to surface typos.

use crate::feed::types::{License, RepublicationMode, RetentionPolicy, SelectionRule};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Cohort token entropy floor per NFR-3809. 128 bits ≈ 22 bytes of
/// uniformly-random base64url, ≈ 32 hex chars. We accept any encoding
/// of ≥16 raw bytes' worth of entropy.
pub const COHORT_TOKEN_MIN_ENTROPY_BITS: u32 = 128;

/// Word-count bounds shared with [`crate::feed::excerpt`] (NFR-3808).
pub const EXCERPT_WORDS_MIN: u32 = 50;
pub const EXCERPT_WORDS_MAX: u32 = 500;

/// `.zetl/config.toml` lens for the feed module.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct FeedConfigLens {
    #[serde(default)]
    pub feed: Option<FeedSection>,
    #[serde(default)]
    pub capability_cohorts: Vec<CapabilityCohortSection>,
    #[serde(default)]
    pub subscriptions: Vec<SubscriptionSection>,
    #[serde(default)]
    pub wiki: Option<WikiSection>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FeedSection {
    /// Master toggle. Default `true` once any other key is present;
    /// the parser does not assume a default — the shell decides whether
    /// missing-section means "off" or "on".
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Absolute URL of the published vault. Required at emit time
    /// (REQ-3809) but optional at parse time so that `zetl init` can
    /// emit a placeholder section.
    #[serde(default)]
    pub base_url: Option<String>,
    /// Channel-level title.
    #[serde(default)]
    pub title: Option<String>,
    /// Channel-level description.
    #[serde(default)]
    pub description: Option<String>,
    /// Default channel-level author name.
    #[serde(default)]
    pub author: Option<String>,
    /// Default channel-level language tag.
    #[serde(default)]
    pub language: Option<String>,
    /// Channel-level copyright string.
    #[serde(default)]
    pub copyright: Option<String>,
    /// Cap on emitted item count. NFR-3802 default = 50.
    #[serde(default)]
    pub max_items: Option<u32>,
    /// Opt-in JSON Feed v1.1 emission per ADR-3801.
    #[serde(default)]
    pub enable_json: Option<bool>,
    /// Per-format paths under `base_url`.
    #[serde(default)]
    pub paths: Option<FeedPaths>,
    /// Scoped subscription catalog entries (REQ-3813).
    #[serde(default)]
    pub scopes: Vec<FeedScope>,
    /// Changelog feed config.
    #[serde(default)]
    pub changelog: Option<FeedChangelog>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FeedPaths {
    #[serde(default)]
    pub rss: Option<String>,
    #[serde(default)]
    pub atom: Option<String>,
    #[serde(default)]
    pub jsonfeed: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FeedScope {
    pub id: String,
    pub title: String,
    pub path: String,
    /// Wire form of [`SelectionRule`]; parsed by [`parse_selection_rule`].
    pub select: SelectionRuleWire,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FeedChangelog {
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Where the latest changelog feed is emitted.
    pub path: String,
    /// Where sealed archive ranges are emitted (`<NNNNNN-MMMMMM>.xml`).
    pub archive_path: String,
    /// Sealing window per CON-3810. Bounded reasonable [10, 100_000].
    pub archive_size: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CapabilityCohortSection {
    pub id: String,
    /// REQ-3830 cohort-scoping token. Validated to ≥128-bit entropy at
    /// parse time per NFR-3809.
    pub token: String,
    /// Glob list of slugs in the cohort.
    pub select: Vec<String>,
    #[serde(default)]
    pub feed_enabled: bool,
    #[serde(default)]
    pub feed_title: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SubscriptionSection {
    pub id: String,
    /// Source feed URL or filesystem path.
    pub source: String,
    /// Subscriber-side selector for which items get imported. Glob list
    /// or `["*"]` for all.
    #[serde(default)]
    pub select: Vec<String>,
    /// Items matching these globs are excluded from import.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Vault-relative target path for ingested items.
    #[serde(default)]
    pub target: Option<String>,
    /// Mapping mode: `mirror` / `flat` / `dated` (REQ-3815).
    #[serde(default)]
    pub mapping: Option<String>,
    /// Operator override for the source's license declaration (CON-3811).
    #[serde(default)]
    pub license: Option<String>,
    /// Operator opt-in to publish ingested items (REQ-3818).
    #[serde(default)]
    pub republish: bool,
    /// `full` / `excerpt` per REQ-3821.
    #[serde(default)]
    pub republish_mode: Option<String>,
    /// Operator-tuned excerpt budget within [`EXCERPT_WORDS_MIN`,
    /// [`EXCERPT_WORDS_MAX`] (NFR-3808).
    #[serde(default)]
    pub excerpt_words: Option<u32>,
    /// Override key for [`crate::feed::credentials`] lookup. Defaults
    /// to `id`.
    #[serde(default)]
    pub credentials_ref: Option<String>,
    /// Operator assertion overriding "no license = deny" default
    /// (REQ-3820 last row, ADR-3809 accepted-risk).
    #[serde(default)]
    pub i_have_permission: bool,
    /// Wire form of [`RetentionPolicy`].
    #[serde(default)]
    pub retention: Option<String>,
    /// `archive` (default) or `delete` per CON-3814.
    #[serde(default)]
    pub retention_mode: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WikiSection {
    /// Receiving-vault license, SPDX form. Used by the CC-BY-SA
    /// compatibility check in [`crate::feed::republication`].
    #[serde(default)]
    pub self_license: Option<String>,
    /// Whether this vault is commercial. CC-BY-NC sources downgrade to
    /// excerpt mode when this is `true` (REQ-3820).
    #[serde(default)]
    pub is_commercial: Option<bool>,
    /// Other `[wiki]` keys (canonical_repo, etc.) round-trip via this
    /// catch-all; not consumed by SPEC-038.
    #[serde(flatten)]
    pub extras: std::collections::BTreeMap<String, toml::Value>,
}

/// Wire-shape for `select` keys. Allows either:
///   * a string `"frontmatter"` (REQ-3811 opt-in), or
///   * a table `{ folder = "..." | tag = "..." | spl = "..." }`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum SelectionRuleWire {
    /// `select = "frontmatter"`.
    Sentinel(String),
    Detailed(SelectionRuleTable),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SelectionRuleTable {
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub spl: Option<String>,
}

/// Parse the lens from a `.zetl/config.toml` body. Surfaces a
/// structured error naming the offending key.
pub fn parse_config(body: &str) -> Result<FeedConfigLens, FeedConfigError> {
    let lens: FeedConfigLens = toml::from_str(body).map_err(|e| FeedConfigError::Toml(e.to_string()))?;
    validate_lens(&lens)?;
    Ok(lens)
}

/// Validate cross-field invariants that serde alone can't catch.
pub fn validate_lens(lens: &FeedConfigLens) -> Result<(), FeedConfigError> {
    // Cohort token entropy + uniqueness.
    let mut seen_ids = std::collections::BTreeSet::new();
    for c in &lens.capability_cohorts {
        if !seen_ids.insert(c.id.as_str()) {
            return Err(FeedConfigError::DuplicateCohortId(c.id.clone()));
        }
        check_token_entropy(&c.token).map_err(|reason| FeedConfigError::WeakCohortToken {
            cohort_id: c.id.clone(),
            reason,
        })?;
    }

    // Subscription validation: id uniqueness, retention, mode strings,
    // license SPDX, excerpt_words bounds.
    let mut sub_ids = std::collections::BTreeSet::new();
    for s in &lens.subscriptions {
        if !sub_ids.insert(s.id.as_str()) {
            return Err(FeedConfigError::DuplicateSubscriptionId(s.id.clone()));
        }
        if let Some(r) = &s.retention {
            RetentionPolicy::parse(r).map_err(|e| FeedConfigError::InvalidRetention {
                subscription_id: s.id.clone(),
                value: r.clone(),
                reason: e.0,
            })?;
        }
        if let Some(mode) = &s.retention_mode {
            if !matches!(mode.as_str(), "archive" | "delete") {
                return Err(FeedConfigError::InvalidRetentionMode {
                    subscription_id: s.id.clone(),
                    value: mode.clone(),
                });
            }
        }
        if let Some(lic) = &s.license {
            // Trim + validate via known set; License::Other is allowed
            // (operator may use any SPDX id) but the empty string and
            // whitespace-only are not.
            if lic.trim().is_empty() {
                return Err(FeedConfigError::InvalidLicense {
                    subscription_id: s.id.clone(),
                    value: lic.clone(),
                });
            }
        }
        if let Some(mode) = &s.republish_mode {
            if !matches!(mode.as_str(), "full" | "excerpt") {
                return Err(FeedConfigError::InvalidRepublishMode {
                    subscription_id: s.id.clone(),
                    value: mode.clone(),
                });
            }
        }
        if let Some(words) = s.excerpt_words {
            if !(EXCERPT_WORDS_MIN..=EXCERPT_WORDS_MAX).contains(&words) {
                return Err(FeedConfigError::InvalidExcerptWords {
                    subscription_id: s.id.clone(),
                    value: words,
                });
            }
        }
        // No-license-declared + republish=true requires i_have_permission.
        if s.republish && s.license.is_none() && !s.i_have_permission {
            return Err(FeedConfigError::RepublishWithoutPermission {
                subscription_id: s.id.clone(),
            });
        }
    }

    // Changelog archive_size bounds.
    if let Some(feed) = &lens.feed {
        if let Some(c) = &feed.changelog {
            if !(10..=100_000).contains(&c.archive_size) {
                return Err(FeedConfigError::InvalidArchiveSize { value: c.archive_size });
            }
        }
        if let Some(max) = feed.max_items {
            if max == 0 {
                return Err(FeedConfigError::InvalidMaxItems);
            }
        }
        // Scope ids must be unique.
        let mut scope_ids = std::collections::BTreeSet::new();
        for sc in &feed.scopes {
            if !scope_ids.insert(sc.id.as_str()) {
                return Err(FeedConfigError::DuplicateScopeId(sc.id.clone()));
            }
        }
    }
    Ok(())
}

/// Resolve a [`SelectionRuleWire`] into the typed
/// [`crate::feed::types::SelectionRule`]. Surfaced separately so the
/// shell can do this lazily (after frontmatter scan).
pub fn parse_selection_rule(w: &SelectionRuleWire) -> Result<SelectionRule, FeedConfigError> {
    match w {
        SelectionRuleWire::Sentinel(s) if s == "frontmatter" => Ok(SelectionRule::FrontmatterOptIn),
        SelectionRuleWire::Sentinel(other) => Err(FeedConfigError::InvalidSelectSentinel(other.clone())),
        SelectionRuleWire::Detailed(t) => match (t.folder.as_deref(), t.tag.as_deref(), t.spl.as_deref()) {
            (Some(f), None, None) => Ok(SelectionRule::Folder { path: PathBuf::from(f) }),
            (None, Some(tag), None) => Ok(SelectionRule::Tag { tag: tag.to_string() }),
            (None, None, Some(spl)) => Ok(SelectionRule::SplQuery { query: spl.to_string() }),
            _ => Err(FeedConfigError::AmbiguousSelectTable),
        },
    }
}

/// Approximate Shannon-entropy check: count distinct printable
/// characters and compute log2. Crude but the SPEC's NFR-3809 only
/// requires ≥128 bits; tokens that are cryptographically random pass
/// trivially (≥22 char base64url ≈ 132 bits, ≥32 hex chars ≈ 128 bits).
fn check_token_entropy(token: &str) -> Result<(), String> {
    if token.is_empty() {
        return Err("token is empty".to_string());
    }
    let len = token.chars().count() as u32;
    let alphabet: std::collections::BTreeSet<char> = token.chars().collect();
    let alphabet_size = alphabet.len() as f64;
    if alphabet_size < 2.0 {
        return Err("token alphabet is degenerate (single character)".to_string());
    }
    let bits_per_char = alphabet_size.log2();
    let total_bits = bits_per_char * len as f64;
    if (total_bits as u32) < COHORT_TOKEN_MIN_ENTROPY_BITS {
        return Err(format!(
            "token entropy {} bits < required {} (len={}, alphabet={})",
            total_bits as u32,
            COHORT_TOKEN_MIN_ENTROPY_BITS,
            len,
            alphabet_size as u32
        ));
    }
    Ok(())
}

/// Helper: convert an operator-side `"full" | "excerpt"` to the typed
/// [`RepublicationMode`].
pub fn parse_republish_mode(s: Option<&str>) -> RepublicationMode {
    match s {
        Some("full") => RepublicationMode::FullAllowed,
        Some("excerpt") | None => RepublicationMode::ExcerptOnly,
        Some(_) => RepublicationMode::ExcerptOnly,
    }
}

/// Helper: convert an SPDX id from `[wiki].self_license` to the typed
/// [`License`].
pub fn parse_self_license(s: Option<&str>) -> Option<License> {
    s.map(License::from_spdx)
}

/// Cross-field config-validation errors. Flattened so callers can
/// `match` exactly on each REQ-3820/3825/3830 invariant.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FeedConfigError {
    #[error("malformed .zetl/config.toml: {0}")]
    Toml(String),
    #[error("[[capability_cohorts]] '{cohort_id}': {reason}")]
    WeakCohortToken { cohort_id: String, reason: String },
    #[error("[[capability_cohorts]] duplicate id: {0}")]
    DuplicateCohortId(String),
    #[error("[[subscriptions]] duplicate id: {0}")]
    DuplicateSubscriptionId(String),
    #[error("[[subscriptions.{subscription_id}].retention]: {value:?} ({reason})")]
    InvalidRetention {
        subscription_id: String,
        value: String,
        reason: String,
    },
    #[error("[[subscriptions.{subscription_id}].retention_mode]: {value:?} (expected 'archive' or 'delete')")]
    InvalidRetentionMode { subscription_id: String, value: String },
    #[error("[[subscriptions.{subscription_id}].license]: {value:?}")]
    InvalidLicense { subscription_id: String, value: String },
    #[error("[[subscriptions.{subscription_id}].republish_mode]: {value:?} (expected 'full' or 'excerpt')")]
    InvalidRepublishMode { subscription_id: String, value: String },
    #[error("[[subscriptions.{subscription_id}].excerpt_words]: {value} out of [{}, {}]", EXCERPT_WORDS_MIN, EXCERPT_WORDS_MAX)]
    InvalidExcerptWords { subscription_id: String, value: u32 },
    #[error(
        "[[subscriptions.{subscription_id}]] republish=true with no license declared requires i_have_permission=true (REQ-3820)"
    )]
    RepublishWithoutPermission { subscription_id: String },
    #[error("[feed.changelog].archive_size: {value} out of [10, 100000]")]
    InvalidArchiveSize { value: u32 },
    #[error("[feed].max_items must be > 0")]
    InvalidMaxItems,
    #[error("[[feed.scopes]] duplicate id: {0}")]
    DuplicateScopeId(String),
    #[error("unknown select sentinel string: {0:?}")]
    InvalidSelectSentinel(String),
    #[error("select table specifies more than one of folder/tag/spl (or none)")]
    AmbiguousSelectTable,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strong_token() -> String {
        // 64-char hex ≈ 256 bits over a 16-character alphabet.
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string()
    }

    #[test]
    fn empty_body_parses_to_default() {
        let lens = parse_config("").unwrap();
        assert!(lens.feed.is_none());
        assert!(lens.subscriptions.is_empty());
        assert!(lens.capability_cohorts.is_empty());
    }

    #[test]
    fn unknown_top_level_keys_allowed() {
        // The wider config has other consumers (parsers, web, hooks).
        // Our lens must not reject keys it doesn't know about.
        let body = r#"
            [some_other_section]
            x = 1
        "#;
        parse_config(body).unwrap();
    }

    #[test]
    fn unknown_keys_inside_feed_section_rejected() {
        let body = r#"
            [feed]
            base_url = "https://example.com"
            unknown_typo = 1
        "#;
        let err = parse_config(body).unwrap_err();
        match err {
            FeedConfigError::Toml(msg) => assert!(msg.contains("unknown_typo"), "got {msg}"),
            _ => panic!("expected toml parse error"),
        }
    }

    #[test]
    fn cohort_token_entropy_validated() {
        let body = format!(
            r#"
            [[capability_cohorts]]
            id = "weak"
            token = "aaaaaaaaaaaa"
            select = ["**/*.md"]

            [[capability_cohorts]]
            id = "strong"
            token = "{}"
            select = ["**/*.md"]
            "#,
            strong_token()
        );
        let err = parse_config(&body).unwrap_err();
        match err {
            FeedConfigError::WeakCohortToken { cohort_id, .. } => assert_eq!(cohort_id, "weak"),
            other => panic!("expected WeakCohortToken, got {other:?}"),
        }
    }

    #[test]
    fn cohort_id_uniqueness() {
        let body = format!(
            r#"
            [[capability_cohorts]]
            id = "dup"
            token = "{}"
            select = []

            [[capability_cohorts]]
            id = "dup"
            token = "{}"
            select = []
            "#,
            strong_token(),
            strong_token()
        );
        assert!(matches!(
            parse_config(&body),
            Err(FeedConfigError::DuplicateCohortId(_))
        ));
    }

    #[test]
    fn subscription_retention_validated() {
        let body = r#"
            [[subscriptions]]
            id = "s"
            source = "https://example.com/feed"
            retention = "garbage"
        "#;
        assert!(matches!(
            parse_config(body),
            Err(FeedConfigError::InvalidRetention { .. })
        ));
    }

    #[test]
    fn subscription_republish_without_license_requires_permission() {
        let body = r#"
            [[subscriptions]]
            id = "s"
            source = "https://example.com/feed"
            republish = true
        "#;
        let err = parse_config(body).unwrap_err();
        assert!(matches!(err, FeedConfigError::RepublishWithoutPermission { .. }));

        let body_ok = r#"
            [[subscriptions]]
            id = "s"
            source = "https://example.com/feed"
            republish = true
            i_have_permission = true
        "#;
        parse_config(body_ok).unwrap();
    }

    #[test]
    fn subscription_id_uniqueness() {
        let body = r#"
            [[subscriptions]]
            id = "dup"
            source = "x"

            [[subscriptions]]
            id = "dup"
            source = "y"
        "#;
        assert!(matches!(
            parse_config(body),
            Err(FeedConfigError::DuplicateSubscriptionId(_))
        ));
    }

    #[test]
    fn excerpt_words_bounds() {
        let body = r#"
            [[subscriptions]]
            id = "s"
            source = "x"
            excerpt_words = 49
        "#;
        assert!(matches!(
            parse_config(body),
            Err(FeedConfigError::InvalidExcerptWords { .. })
        ));
        let body_ok = r#"
            [[subscriptions]]
            id = "s"
            source = "x"
            excerpt_words = 50
        "#;
        parse_config(body_ok).unwrap();
    }

    #[test]
    fn scope_select_sentinel_parses() {
        let body = r#"
            [feed]
            base_url = "https://example.com"
            [[feed.scopes]]
            id = "blog"
            title = "Blog"
            path = "/blog/feed.xml"
            select = "frontmatter"
        "#;
        let lens = parse_config(body).unwrap();
        let scope = &lens.feed.unwrap().scopes[0];
        assert_eq!(parse_selection_rule(&scope.select).unwrap(), SelectionRule::FrontmatterOptIn);
    }

    #[test]
    fn scope_select_table_parses() {
        let body = r#"
            [feed]
            base_url = "https://example.com"
            [[feed.scopes]]
            id = "notes"
            title = "Notes"
            path = "/notes/feed.xml"
            select.folder = "notes/"
        "#;
        let lens = parse_config(body).unwrap();
        let scope = &lens.feed.unwrap().scopes[0];
        match parse_selection_rule(&scope.select).unwrap() {
            SelectionRule::Folder { path } => assert_eq!(path, PathBuf::from("notes/")),
            other => panic!("expected Folder, got {other:?}"),
        }
    }

    #[test]
    fn changelog_archive_size_bounds() {
        let body = r#"
            [feed]
            base_url = "https://example.com"
            [feed.changelog]
            path = "/changelog.xml"
            archive_path = "/archives"
            archive_size = 5
        "#;
        assert!(matches!(
            parse_config(body),
            Err(FeedConfigError::InvalidArchiveSize { .. })
        ));
    }

    #[test]
    fn wiki_sectionextras_preserved() {
        let body = r#"
            [wiki]
            self_license = "CC-BY-SA-4.0"
            is_commercial = false
            canonical_repo = "https://codeberg.org/anuna/zetl"
            id = "anuna"
        "#;
        let lens = parse_config(body).unwrap();
        let wiki = lens.wiki.unwrap();
        assert_eq!(wiki.self_license.as_deref(), Some("CC-BY-SA-4.0"));
        assert_eq!(wiki.is_commercial, Some(false));
        // Other keys round-trip via the catch-all.
        assert!(wiki.extras.contains_key("canonical_repo"));
        assert!(wiki.extras.contains_key("id"));
    }

    #[test]
    fn parse_self_license_returns_typed_license() {
        assert_eq!(parse_self_license(Some("CC-BY-SA-4.0")), Some(License::CcBySa4_0));
        assert_eq!(parse_self_license(None), None);
    }

    #[test]
    fn full_realistic_config_parses_clean() {
        let body = format!(
            r#"
            [feed]
            base_url = "https://example.com"
            title = "Example Vault"
            description = "Notes from the field"
            max_items = 50
            enable_json = true

            [feed.paths]
            rss = "/feed.xml"
            atom = "/atom.xml"
            jsonfeed = "/feed.json"

            [[feed.scopes]]
            id = "blog"
            title = "Blog"
            path = "/blog/feed.xml"
            select = "frontmatter"

            [feed.changelog]
            path = "/changelog.xml"
            archive_path = "/changelog/archives"
            archive_size = 1000

            [[capability_cohorts]]
            id = "alpha-readers"
            token = "{}"
            select = ["alpha/**"]
            feed_enabled = true
            feed_title = "Alpha Cohort"

            [[subscriptions]]
            id = "upstream"
            source = "https://upstream.example/feed.xml"
            select = ["*"]
            target = "subs/upstream"
            mapping = "mirror"
            license = "CC-BY-SA-4.0"
            republish = true
            republish_mode = "full"
            excerpt_words = 200
            retention = "90d"
            retention_mode = "archive"

            [wiki]
            self_license = "CC-BY-SA-4.0"
            is_commercial = false
            "#,
            strong_token()
        );
        parse_config(&body).unwrap();
    }
}
