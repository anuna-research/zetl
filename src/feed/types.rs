//! Type lattice consumed by the pure-core projections and serialisers.
//!
//! Every type here is `Debug + Clone + PartialEq` and serde-serialisable
//! so the JSON Feed serialiser can use the same struct without a parallel
//! shape. Dates are RFC 3339 strings (validated at construction); we
//! avoid pulling chrono into the leaf type module so the feature-gated
//! `history` feature stays the only consumer of chrono in the crate.
//!
//! Spec references:
//!
//! - REQ-3801 stable FeedItem
//! - REQ-3803 stable item id (see [`crate::feed::item_id`])
//! - REQ-3814 zetl-namespace source metadata
//! - REQ-3820 republication eligibility
//! - CON-3811 license declaration
//! - CON-3814 retention policy

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A single emittable feed entry. Produced by the pure-core projection
/// pipeline (`feed::select` -> `feed::resolve_date` ->
/// `feed::rewrite_links` -> `feed::item_id` -> ...) and consumed by every
/// serialiser (RSS / Atom / JSON Feed).
///
/// Identity is `id`: stable per REQ-3803 across machines, build hosts,
/// and zetl versions in the same major line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedItem {
    /// Stable item id. Tag URI form per REQ-3803 (see [`super::item_id`]).
    pub id: String,
    /// Human-readable item title.
    pub title: String,
    /// Absolute URL for the item, derived from the configured base_url
    /// and the page slug. Always absolute when present in a feed body
    /// (REQ-3805 + REQ-3807).
    pub url: String,
    /// RFC 3339 first-publication timestamp. Required (REQ-3804).
    pub date_published: String,
    /// RFC 3339 last-modification timestamp. Optional; when present
    /// emitted as `<atom:updated>` / JSON Feed `date_modified`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_modified: Option<String>,
    /// Plain-text or short-HTML summary. Distinct from `content_html`;
    /// always rendered as feed `<description>` / atom `<summary>` /
    /// JSON Feed `summary`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Sanitised HTML body. The serialisers escape with CDATA (RSS) or
    /// `type="html"` (Atom) or string-encode (JSON Feed). The pure core
    /// expects this string to already be sanitiser-safe for SPEC-034
    /// concerns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_html: Option<String>,
    /// Optional author. RSS lacks per-item authors so the channel-level
    /// fallback is used when this is `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<AuthorRef>,
    /// Free-form tags. Render as `<category>` (RSS / Atom) or JSON Feed
    /// `tags` array.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Resolved license, when present in the source. `None` is distinct
    /// from `Some(License::Unknown)`: the former means the projection
    /// did not look; the latter means it looked and found no signal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<License>,
    /// Zetl-namespace metadata for downstream subscribers (REQ-3814).
    /// Omitted from the wire when [`SourceMetadata`] is empty.
    #[serde(default, skip_serializing_if = "SourceMetadata::is_empty")]
    pub source_metadata: SourceMetadata,
}

/// Author reference. RSS uses `<author>` as `email (Name)`, Atom uses
/// `<atom:author>` with `<name>`/`<email>`/`<uri>` children, JSON Feed
/// uses `authors[]` with `name`/`url`/`avatar` keys; this single shape
/// covers all three.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AuthorRef {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Zetl-namespace source metadata carried per item per REQ-3814. Used by
/// downstream subscribers (other zetl vaults) to recover source identity
/// and detect changes via the changelog feed (REQ-3816 / REQ-3817).
///
/// Field naming mirrors the Atom `_zetl:` namespace shape (CON-3815) so
/// the same struct serialises into both XML extension elements and JSON
/// Feed extension keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SourceMetadata {
    /// Vault-relative source path (e.g. `notes/foo.md`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<PathBuf>,
    /// Stable object identity (page slug or anchor + block id).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_id: Option<String>,
    /// Current Merkle / content hash. Hex-encoded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    /// Previous content hash, for changelog `updated`/`moved` events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev_content_hash: Option<String>,
    /// Monotonic publisher-local sequence number (REQ-3817).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changelog_seq: Option<u64>,
}

impl SourceMetadata {
    /// True iff every field is `None`. Used by `serde(skip_serializing_if)`
    /// at the parent `FeedItem` level so a fully-empty SourceMetadata
    /// does not show up in JSON Feed output.
    pub fn is_empty(&self) -> bool {
        self.source_path.is_none()
            && self.object_id.is_none()
            && self.content_hash.is_none()
            && self.prev_content_hash.is_none()
            && self.changelog_seq.is_none()
    }
}

/// Per-feed configuration consumed by every serialiser. Built by the
/// shell from `[feed]` and (for scoped feeds) `[[feed.scopes]]` /
/// `[[capability_cohorts]]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedConfig {
    /// Absolute base URL for the published vault, no trailing slash.
    pub base_url: String,
    /// Channel-level title.
    pub title: String,
    /// Channel-level description.
    pub description: String,
    /// Cap on emitted item count after sort (REQ-3801 / NFR-3802).
    pub max_items: u32,
    /// Format flags. RSS + Atom are mandatory; JSON Feed is opt-in
    /// per ADR-3801.
    pub formats: OutputFormatSet,
    /// Per-format absolute URL paths under `base_url`. Used both for
    /// the `<atom:link rel=self>` channel element and for cross-format
    /// link rewriting.
    pub paths: FeedPaths,
    /// Scope id for scoped feeds (REQ-3813). `None` for the root feed.
    pub scope_id: Option<String>,
    /// Capability cohort id for cohort feeds (REQ-3830). `None` for
    /// public feeds. The cohort *token* is never carried in this struct
    /// per REQ-3831; only the operator-readable id is.
    pub cohort_id: Option<String>,
    /// Default per-channel author. Shown when an item lacks its own.
    pub default_author: Option<AuthorRef>,
    /// Channel-level language tag (RFC 5646), e.g. `en-AU`. `None` is
    /// permitted and produces no `<language>` element.
    pub language: Option<String>,
    /// Channel-level copyright / rights string.
    pub copyright: Option<String>,
}

/// Flags toggling per-format emission. RSS 2.0 + Atom 1.0 are always
/// produced; JSON Feed is gated by `[feed].enable_json` per ADR-3801.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputFormatSet {
    pub rss: bool,
    pub atom: bool,
    pub jsonfeed: bool,
}

impl Default for OutputFormatSet {
    /// RSS + Atom on, JSON Feed off — matches SPEC-038's Phase 4 defaults.
    fn default() -> Self {
        Self {
            rss: true,
            atom: true,
            jsonfeed: false,
        }
    }
}

/// Absolute URLs for each format under the same base. Built by the
/// shell from `[feed]` keys; the pure core never invents paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedPaths {
    /// e.g. `/feed.xml`. Always absolute path under `base_url`.
    pub rss: String,
    /// e.g. `/atom.xml`.
    pub atom: String,
    /// e.g. `/feed.json`. Only consulted when `formats.jsonfeed`.
    pub jsonfeed: String,
}

/// Wire-level format identifier. Used as a key into per-format
/// behaviour (e.g. tests/cross_format_equivalence) and in observability
/// labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OutputFormat {
    /// RSS 2.0.
    Rss20,
    /// Atom 1.0 (RFC 4287).
    Atom10,
    /// JSON Feed v1.1.
    JsonFeed11,
}

impl OutputFormat {
    /// Canonical Content-Type for serve-mode responses.
    pub fn content_type(self) -> &'static str {
        match self {
            OutputFormat::Rss20 => "application/rss+xml; charset=utf-8",
            OutputFormat::Atom10 => "application/atom+xml; charset=utf-8",
            OutputFormat::JsonFeed11 => "application/feed+json; charset=utf-8",
        }
    }
}

/// SPDX-style license identifiers recognised by the pure core. The
/// closed set is the load-bearing input to [`crate::feed::republication`]
/// — every variant has an explicit row in REQ-3820's eligibility table.
///
/// The wire form is the SPDX identifier with hyphen-separated form
/// (e.g. `CC-BY-SA-4.0`); see [`License::as_spdx`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum License {
    /// Public-domain dedication.
    Cc0_1_0,
    /// Attribution-only.
    CcBy4_0,
    /// Attribution-only, prior version (some old feeds still declare 3.0).
    CcBy3_0,
    /// Attribution + share-alike. Republication restricted to vaults
    /// declaring a compatible self_license per REQ-3820.
    CcBySa4_0,
    /// Attribution + non-commercial. Republication blocked when the
    /// receiving vault is_commercial per REQ-3820.
    CcByNc4_0,
    /// Attribution + no-derivatives. Excerpt-only is the only legal
    /// republication mode (REQ-3820) because wikilink rewriting modifies
    /// the body.
    CcByNd4_0,
    /// Any other SPDX-known license; carries the identifier verbatim.
    /// Treated as `Unknown` for republication eligibility unless the
    /// operator provides explicit `i_have_permission = true`.
    Other(String),
    /// No recognisable signal in the source. Default-deny per REQ-3820.
    Unknown,
}

impl License {
    /// SPDX identifier, hyphen form. The Display impl is identical so
    /// this string is also what serialises into TOML and Atom rights
    /// elements.
    pub fn as_spdx(&self) -> String {
        match self {
            License::Cc0_1_0 => "CC0-1.0".to_string(),
            License::CcBy4_0 => "CC-BY-4.0".to_string(),
            License::CcBy3_0 => "CC-BY-3.0".to_string(),
            License::CcBySa4_0 => "CC-BY-SA-4.0".to_string(),
            License::CcByNc4_0 => "CC-BY-NC-4.0".to_string(),
            License::CcByNd4_0 => "CC-BY-ND-4.0".to_string(),
            License::Other(id) => id.clone(),
            License::Unknown => "NOASSERTION".to_string(),
        }
    }

    /// Parse an SPDX identifier into the closed set; unknown SPDX ids
    /// fall through to `License::Other(...)`. The empty string and
    /// the literal `NOASSERTION` map to `License::Unknown`. See also
    /// [`crate::feed::license_resolve`] for the URL-canonicalising
    /// counterpart used on inbound feeds.
    pub fn from_spdx(s: &str) -> License {
        match s.trim() {
            "" | "NOASSERTION" => License::Unknown,
            "CC0-1.0" => License::Cc0_1_0,
            "CC-BY-4.0" => License::CcBy4_0,
            "CC-BY-3.0" => License::CcBy3_0,
            "CC-BY-SA-4.0" => License::CcBySa4_0,
            "CC-BY-NC-4.0" => License::CcByNc4_0,
            "CC-BY-ND-4.0" => License::CcByNd4_0,
            other => License::Other(other.to_string()),
        }
    }
}

impl std::fmt::Display for License {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.as_spdx())
    }
}

/// Republication decision for an inbound item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepublicationDecision {
    /// What the operator may publish.
    pub mode: RepublicationMode,
    /// Which clause of REQ-3820 fired (and why). Carried for audit and
    /// observability; stable enum so log lines are grep-able.
    pub rationale: RepublicationRationale,
}

/// Three-way republication outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RepublicationMode {
    /// Public republication forbidden. Item still ingested to inbox/
    /// for the operator's local view.
    Deny,
    /// Republish summary + link to source (REQ-3821). The full body
    /// is never rendered on the public surface.
    ExcerptOnly,
    /// Full content republication permitted (with attribution).
    FullAllowed,
}

/// Stable rationale codes naming which row of REQ-3820 produced a
/// [`RepublicationDecision`]. Strings are intentionally short and
/// kebab-case so they appear cleanly in observability labels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RepublicationRationale {
    /// Public domain or CC0 — full reuse always allowed.
    PublicDomain,
    /// CC-BY: full reuse with attribution (operator opted into Full).
    CcByFull,
    /// CC-BY: default downgrade to excerpt mode.
    CcByExcerpt,
    /// CC-BY-SA: full reuse permitted because the receiving vault
    /// declares a compatible self_license.
    CcBySaCompatible,
    /// CC-BY-SA: license incompatibility forces excerpt mode.
    CcBySaIncompatible,
    /// CC-BY-NC: receiving vault is non-commercial, full reuse allowed.
    CcByNcNonCommercial,
    /// CC-BY-NC: receiving vault is_commercial, downgrade to excerpt.
    CcByNcCommercial,
    /// CC-BY-ND forbids derivative works; only excerpt+link is legal.
    CcByNdExcerptOnly,
    /// No license signal AND no explicit operator permission.
    UnknownDefaultDeny,
    /// No license signal but operator asserted `i_have_permission=true`.
    /// Honoured at operator's accepted-risk per ADR-3809.
    UnknownOperatorPermitted,
}

/// Page selection rule for the publication-level feed (root or scoped).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SelectionRule {
    /// Page must opt in via `frontmatter.feed: true` (or equivalent).
    /// The default rule for the public root feed.
    FrontmatterOptIn,
    /// Pages whose vault path lives under the given folder prefix.
    Folder { path: PathBuf },
    /// Pages tagged with the given tag.
    Tag { tag: String },
    /// Pages selected by an SPL query string. The pure core does not
    /// evaluate SPL; the shell-side caller hands a pre-resolved set.
    SplQuery { query: String },
}

/// Retention policy per REQ-3832 + CON-3814. Default is `Forever`
/// (ADR-3812 archive-not-delete leaning).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum RetentionPolicy {
    /// Keep every ingested item indefinitely.
    Forever,
    /// Keep items younger than `seconds`. The pure core deals in
    /// integer seconds rather than `chrono::Duration` so the leaf
    /// types module stays chrono-free.
    Duration { seconds: i64 },
    /// Keep at most `count` newest items per subscription.
    Count { count: usize },
}

impl RetentionPolicy {
    /// Parse the operator-facing string form. Accepted shapes:
    ///   * `forever`
    ///   * `<N>d` (days), `<N>w` (weeks), `<N>mo` (months ≈ 30 days),
    ///     `<N>y` (years ≈ 365 days)
    ///   * `last-<N>` (count form per CON-3814)
    pub fn parse(s: &str) -> Result<RetentionPolicy, RetentionParseError> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("forever") {
            return Ok(RetentionPolicy::Forever);
        }
        if let Some(rest) = s.strip_prefix("last-") {
            let n: usize = rest
                .parse()
                .map_err(|_| RetentionParseError(format!("invalid count: {s}")))?;
            return Ok(RetentionPolicy::Count { count: n });
        }
        // Duration: <N><unit>; longest unit suffix first so `mo` wins
        // over `m` if we ever add minutes.
        for (suffix, secs_per_unit) in [
            ("mo", 60 * 60 * 24 * 30),
            ("d", 60 * 60 * 24),
            ("w", 60 * 60 * 24 * 7),
            ("y", 60 * 60 * 24 * 365),
        ] {
            if let Some(num) = s.strip_suffix(suffix) {
                let n: i64 = num
                    .trim()
                    .parse()
                    .map_err(|_| RetentionParseError(format!("invalid duration: {s}")))?;
                return Ok(RetentionPolicy::Duration {
                    seconds: n * secs_per_unit,
                });
            }
        }
        Err(RetentionParseError(format!(
            "unrecognised retention form: {s} (expected `forever`, `<N>d|w|mo|y`, or `last-<N>`)"
        )))
    }
}

/// Parse failure for [`RetentionPolicy::parse`]. Surfaced through
/// [`crate::feed::config`] when a `[[subscriptions]]` entry's
/// `retention` field is malformed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct RetentionParseError(pub String);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn license_round_trip_via_spdx() {
        for lic in [
            License::Cc0_1_0,
            License::CcBy4_0,
            License::CcBy3_0,
            License::CcBySa4_0,
            License::CcByNc4_0,
            License::CcByNd4_0,
        ] {
            let id = lic.as_spdx();
            assert_eq!(License::from_spdx(&id), lic, "round-trip {id}");
        }
    }

    #[test]
    fn license_unknown_canonicalises_via_noassertion() {
        assert_eq!(License::Unknown.as_spdx(), "NOASSERTION");
        assert_eq!(License::from_spdx(""), License::Unknown);
        assert_eq!(License::from_spdx("NOASSERTION"), License::Unknown);
    }

    #[test]
    fn license_other_preserves_unknown_spdx_id() {
        let other = License::from_spdx("MIT");
        assert_eq!(other, License::Other("MIT".to_string()));
        assert_eq!(other.as_spdx(), "MIT");
    }

    #[test]
    fn retention_parses_canonical_forms() {
        assert_eq!(
            RetentionPolicy::parse("forever").unwrap(),
            RetentionPolicy::Forever
        );
        assert_eq!(
            RetentionPolicy::parse("90d").unwrap(),
            RetentionPolicy::Duration {
                seconds: 90 * 24 * 60 * 60
            }
        );
        assert_eq!(
            RetentionPolicy::parse("last-100").unwrap(),
            RetentionPolicy::Count { count: 100 }
        );
        assert_eq!(
            RetentionPolicy::parse("3mo").unwrap(),
            RetentionPolicy::Duration {
                seconds: 3 * 30 * 24 * 60 * 60
            }
        );
    }

    #[test]
    fn retention_rejects_garbage() {
        assert!(RetentionPolicy::parse("eternal").is_err());
        assert!(RetentionPolicy::parse("90 days").is_err());
        assert!(RetentionPolicy::parse("last-").is_err());
    }

    #[test]
    fn output_format_content_types_match_specification() {
        assert_eq!(
            OutputFormat::Rss20.content_type(),
            "application/rss+xml; charset=utf-8"
        );
        assert_eq!(
            OutputFormat::Atom10.content_type(),
            "application/atom+xml; charset=utf-8"
        );
        assert_eq!(
            OutputFormat::JsonFeed11.content_type(),
            "application/feed+json; charset=utf-8"
        );
    }

    #[test]
    fn feed_item_serde_round_trip() {
        let item = FeedItem {
            id: "tag:example.com,2026:zetl/foo".to_string(),
            title: "Foo".to_string(),
            url: "https://example.com/foo".to_string(),
            date_published: "2026-05-08T00:00:00Z".to_string(),
            date_modified: None,
            summary: Some("hi".to_string()),
            content_html: None,
            author: None,
            tags: vec!["meta".to_string()],
            license: Some(License::CcBy4_0),
            source_metadata: SourceMetadata::default(),
        };
        let s = serde_json::to_string(&item).unwrap();
        let back: FeedItem = serde_json::from_str(&s).unwrap();
        assert_eq!(back, item);
    }

    #[test]
    fn source_metadata_is_empty_when_default() {
        let s = SourceMetadata::default();
        assert!(s.is_empty());
        let s2 = SourceMetadata {
            object_id: Some("foo".to_string()),
            ..SourceMetadata::default()
        };
        assert!(!s2.is_empty());
    }
}
