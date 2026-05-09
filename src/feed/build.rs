//! Outbound build-mode emission per task-build-outbound (REQ-3801,
//! REQ-3808, REQ-3809, OBS-3801, OBS-3802).
//!
//! Effectful shell. Drives the pure-core pipeline (`select` ->
//! `resolve_date` -> `rewrite_links` -> `item_id` -> serialise_*) and
//! returns a list of `(relative_path, bytes)` pairs the caller writes
//! to `dist/`. Decoupled from the existing `web::build` pipeline so the
//! feed shell can be exercised in isolation.

use crate::feed::config::{FeedConfigLens, FeedSection};
use crate::feed::select::PageView;
use crate::feed::serialise_atom::serialise_atom;
use crate::feed::serialise_jsonfeed::serialise_jsonfeed;
use crate::feed::serialise_rss::serialise_rss;
use crate::feed::types::{FeedConfig, FeedPaths, OutputFormatSet, SelectionRule};
use std::time::{Duration, Instant};

/// Default per-format paths under `base_url`.
pub const DEFAULT_RSS_PATH: &str = "/feed.xml";
pub const DEFAULT_ATOM_PATH: &str = "/atom.xml";
pub const DEFAULT_JSONFEED_PATH: &str = "/feed.json";
/// Default channel-level item cap per NFR-3802.
pub const DEFAULT_MAX_ITEMS: u32 = 50;

/// Result of an outbound build pass. Caller writes each entry to disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedEmission {
    /// `(relative-path-under-dist, file-body-bytes)`. Paths are
    /// always slash-prefixed and slash-separated (e.g. `/feed.xml`).
    pub files: Vec<(String, Vec<u8>)>,
    /// Build statistics for OBS-3802.
    pub stats: BuildStats,
    /// Discovery tags ready to splice into the `<head>` of every
    /// published page (REQ-3801 rel=alternate).
    pub discovery_tags: Vec<String>,
}

/// OBS-3802 build counters / histograms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildStats {
    /// Number of items selected (post-visibility, pre-truncation).
    pub items_selected: usize,
    /// Number of items emitted (post `max_items` truncation).
    pub items_emitted: usize,
    /// Wall-clock time of the build pass.
    pub duration: Duration,
    /// Which formats were emitted; check via `stats.formats.rss` /
    /// `stats.formats.atom` / `stats.formats.jsonfeed`.
    pub formats: crate::feed::types::OutputFormatSet,
}

/// Hard-error cases for [`emit_root_feed`]. These produce a non-zero
/// exit code in `zetl build`; downstream observability counters
/// (OBS-3801) record the result label.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BuildError {
    /// REQ-3809 self-publication safety: refuse to emit feed bodies
    /// without an explicitly configured `[feed].base_url`.
    #[error("[feed].base_url not set; refusing to emit feeds (REQ-3809)")]
    MissingBaseUrl,
    /// `[feed]` section absent or `enabled = false`.
    #[error("[feed] section disabled or missing")]
    Disabled,
    /// A required field on `[feed]` is missing (e.g. `title`).
    #[error("[feed].{0} required")]
    MissingField(&'static str),
}

/// Emit the root public feed (RSS + Atom + optional JSON Feed) for the
/// supplied page set. Returns the file bodies + stats; the caller is
/// responsible for writing them under the dist/ tree.
///
/// `pages` contains every visible page's [`PageView`]; this function
/// applies the configured selection rule + the supplied `visibility`
/// closure (which the shell will already have set to the public
/// predicate by the time `emit_root_feed` runs).
pub fn emit_root_feed(
    lens: &FeedConfigLens,
    pages: &[PageView<'_>],
    visibility: &dyn Fn(&PageView<'_>) -> bool,
    rule: &SelectionRule,
) -> Result<FeedEmission, BuildError> {
    let started = Instant::now();
    let feed = lens.feed.as_ref().ok_or(BuildError::Disabled)?;
    if matches!(feed.enabled, Some(false)) {
        return Err(BuildError::Disabled);
    }
    let base_url = feed.base_url.as_deref().ok_or(BuildError::MissingBaseUrl)?;
    let title = feed.title.clone().unwrap_or_else(|| "Untitled".to_string());
    let description = feed.description.clone().unwrap_or_default();
    let max_items = feed.max_items.unwrap_or(DEFAULT_MAX_ITEMS);
    let formats = OutputFormatSet {
        rss: true,
        atom: true,
        jsonfeed: feed.enable_json.unwrap_or(false),
    };
    let paths = resolve_paths(feed);

    let chosen = crate::feed::select::select(pages, rule, visibility, max_items);

    let items_selected = pages.iter().filter(|p| visibility(p)).count();
    let items_emitted = chosen.len();
    // RFC 4287 §4.1.1: feeds without per-entry authors MUST carry a
    // feed-level <atom:author>. Populate from `[feed].author` if set,
    // else from `[feed].title` so the emitted Atom is always
    // strict-parser conformant.
    let default_author = feed
        .author
        .as_deref()
        .map(|name| crate::feed::types::AuthorRef {
            name: name.to_string(),
            email: None,
            url: None,
        })
        .or_else(|| {
            Some(crate::feed::types::AuthorRef {
                name: feed.title.clone().unwrap_or_else(|| "Untitled".to_string()),
                email: None,
                url: None,
            })
        });
    let cfg = FeedConfig {
        base_url: base_url.to_string(),
        title,
        description,
        max_items,
        formats,
        paths: paths.clone(),
        scope_id: None,
        cohort_id: None,
        default_author,
        language: feed.language.clone(),
        copyright: feed.copyright.clone(),
    };

    // `select::select` already sorts by (date desc, id asc); no post-sort.

    let mut files = Vec::with_capacity(3);
    files.push((paths.rss.clone(), serialise_rss(&chosen, &cfg).into_bytes()));
    files.push((
        paths.atom.clone(),
        serialise_atom(&chosen, &cfg).into_bytes(),
    ));
    if formats.jsonfeed {
        files.push((
            paths.jsonfeed.clone(),
            serialise_jsonfeed(&chosen, &cfg).into_bytes(),
        ));
    }

    let discovery_tags = discovery_tags(&cfg);
    let stats = BuildStats {
        items_selected,
        items_emitted,
        duration: started.elapsed(),
        formats,
    };
    Ok(FeedEmission {
        files,
        stats,
        discovery_tags,
    })
}

/// Build the rel=alternate Link tags for a feed. Inject into every
/// published page's `<head>` (REQ-3801).
pub fn discovery_tags(cfg: &FeedConfig) -> Vec<String> {
    let mut out = Vec::with_capacity(3);
    let base = cfg.base_url.trim_end_matches('/');
    out.push(format!(
        r#"<link rel="alternate" type="application/rss+xml" title="{title} (RSS)" href="{base}{path}" />"#,
        title = html_attr_escape(&cfg.title),
        path = cfg.paths.rss
    ));
    out.push(format!(
        r#"<link rel="alternate" type="application/atom+xml" title="{title} (Atom)" href="{base}{path}" />"#,
        title = html_attr_escape(&cfg.title),
        path = cfg.paths.atom
    ));
    if cfg.formats.jsonfeed {
        out.push(format!(
            r#"<link rel="alternate" type="application/feed+json" title="{title} (JSON Feed)" href="{base}{path}" />"#,
            title = html_attr_escape(&cfg.title),
            path = cfg.paths.jsonfeed
        ));
    }
    out
}

fn html_attr_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;")
}

fn resolve_paths(feed: &FeedSection) -> FeedPaths {
    let cfg_paths = feed.paths.as_ref();
    FeedPaths {
        rss: cfg_paths
            .and_then(|p| p.rss.clone())
            .unwrap_or_else(|| DEFAULT_RSS_PATH.to_string()),
        atom: cfg_paths
            .and_then(|p| p.atom.clone())
            .unwrap_or_else(|| DEFAULT_ATOM_PATH.to_string()),
        jsonfeed: cfg_paths
            .and_then(|p| p.jsonfeed.clone())
            .unwrap_or_else(|| DEFAULT_JSONFEED_PATH.to_string()),
    }
}

/// Build a [`FeedConfig`] for an arbitrary scoped or cohort feed.
/// Used by the scoped-feed and cap-feed emitters.
pub fn build_feed_config(
    lens: &FeedConfigLens,
    paths: FeedPaths,
    formats: OutputFormatSet,
    title: String,
    scope_id: Option<String>,
    cohort_id: Option<String>,
) -> Result<FeedConfig, BuildError> {
    let feed = lens.feed.as_ref().ok_or(BuildError::Disabled)?;
    let base_url = feed.base_url.as_deref().ok_or(BuildError::MissingBaseUrl)?;
    let description = feed.description.clone().unwrap_or_default();
    let max_items = feed.max_items.unwrap_or(DEFAULT_MAX_ITEMS);
    // RFC 4287 §4.1.1: feeds without per-entry authors MUST carry a
    // feed-level <atom:author>. Scoped + cohort feeds inherit the
    // root's `[feed].author` (or fall back to `[feed].title` and then
    // the scope title) so a scoped Atom output is still strict-parser
    // conformant when its items lack item.author.
    let default_author = Some(crate::feed::types::AuthorRef {
        name: feed
            .author
            .clone()
            .or_else(|| feed.title.clone())
            .unwrap_or_else(|| title.clone()),
        email: None,
        url: None,
    });
    Ok(FeedConfig {
        base_url: base_url.to_string(),
        title,
        description,
        max_items,
        formats,
        paths,
        scope_id,
        cohort_id,
        default_author,
        language: feed.language.clone(),
        copyright: feed.copyright.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::types::{FeedItem, License, SourceMetadata};
    use std::path::Path;

    fn lens_with_feed(json: bool) -> FeedConfigLens {
        let body = format!(
            r#"
            [feed]
            base_url = "https://example.com"
            title = "Test"
            description = "Test feed"
            enable_json = {json}
            max_items = 50
        "#
        );
        crate::feed::config::parse_config(&body).unwrap()
    }

    fn views(n: usize) -> (Vec<PageView<'static>>, Vec<FeedItem>) {
        let mut items = Vec::new();
        let path: &'static Path = Path::new("p.md");
        let no_tags: &'static [String] = &[];
        let mut owned_items = Vec::new();
        for i in 0..n {
            owned_items.push(FeedItem {
                id: format!("tag:example.com,2026:zetl/p{i}"),
                title: format!("p{i}"),
                url: format!("https://example.com/p{i}"),
                date_published: format!("2026-05-{:02}T00:00:00Z", i + 1),
                date_modified: None,
                summary: None,
                content_html: None,
                author: None,
                tags: Vec::new(),
                license: Some(License::Cc0_1_0),
                source_metadata: SourceMetadata::default(),
            });
        }
        for it in &owned_items {
            items.push(PageView {
                slug: Box::leak(it.title.clone().into_boxed_str()),
                path,
                frontmatter_feed_optin: true,
                tags: no_tags,
                matches_spl_query: false,
                item: it.clone(),
            });
        }
        (items, owned_items)
    }

    fn always(_: &PageView<'_>) -> bool {
        true
    }

    #[test]
    fn missing_base_url_errors() {
        let body = r#"
            [feed]
            title = "T"
        "#;
        let lens = crate::feed::config::parse_config(body).unwrap();
        let (pages, _) = views(0);
        let err =
            emit_root_feed(&lens, &pages, &always, &SelectionRule::FrontmatterOptIn).unwrap_err();
        assert_eq!(err, BuildError::MissingBaseUrl);
    }

    #[test]
    fn disabled_feed_errors() {
        let body = r#"
            [feed]
            base_url = "https://example.com"
            enabled = false
        "#;
        let lens = crate::feed::config::parse_config(body).unwrap();
        let (pages, _) = views(0);
        let err =
            emit_root_feed(&lens, &pages, &always, &SelectionRule::FrontmatterOptIn).unwrap_err();
        assert_eq!(err, BuildError::Disabled);
    }

    #[test]
    fn emits_rss_and_atom_by_default() {
        let lens = lens_with_feed(false);
        let (pages, _) = views(3);
        let emission =
            emit_root_feed(&lens, &pages, &always, &SelectionRule::FrontmatterOptIn).unwrap();
        let paths: Vec<&str> = emission.files.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(paths, vec!["/feed.xml", "/atom.xml"]);
        assert!(emission.stats.formats.rss);
        assert!(emission.stats.formats.atom);
        assert!(!emission.stats.formats.jsonfeed);
        assert_eq!(emission.stats.items_emitted, 3);
    }

    #[test]
    fn enable_json_emits_feed_json() {
        let lens = lens_with_feed(true);
        let (pages, _) = views(2);
        let emission =
            emit_root_feed(&lens, &pages, &always, &SelectionRule::FrontmatterOptIn).unwrap();
        let paths: Vec<&str> = emission.files.iter().map(|(p, _)| p.as_str()).collect();
        assert!(paths.contains(&"/feed.json"));
        assert!(emission.stats.formats.jsonfeed);
    }

    #[test]
    fn discovery_tags_present_for_active_formats() {
        let lens = lens_with_feed(true);
        let (pages, _) = views(0);
        let emission =
            emit_root_feed(&lens, &pages, &always, &SelectionRule::FrontmatterOptIn).unwrap();
        assert_eq!(emission.discovery_tags.len(), 3);
        assert!(emission.discovery_tags[0].contains("application/rss+xml"));
        assert!(emission.discovery_tags[1].contains("application/atom+xml"));
        assert!(emission.discovery_tags[2].contains("application/feed+json"));
    }

    #[test]
    fn deterministic_byte_identical_rebuilds() {
        let lens = lens_with_feed(true);
        let (pages, _) = views(5);
        let a = emit_root_feed(&lens, &pages, &always, &SelectionRule::FrontmatterOptIn).unwrap();
        let b = emit_root_feed(&lens, &pages, &always, &SelectionRule::FrontmatterOptIn).unwrap();
        // Bytes match (excluding stats.duration which varies).
        assert_eq!(a.files, b.files);
        assert_eq!(a.discovery_tags, b.discovery_tags);
    }

    #[test]
    fn visibility_predicate_filters_pages() {
        let lens = lens_with_feed(false);
        let (pages, _) = views(5);
        let only_first_three = |p: &PageView<'_>| {
            p.slug.ends_with('0') || p.slug.ends_with('1') || p.slug.ends_with('2')
        };
        let emission = emit_root_feed(
            &lens,
            &pages,
            &only_first_three,
            &SelectionRule::FrontmatterOptIn,
        )
        .unwrap();
        assert_eq!(emission.stats.items_emitted, 3);
    }
}
