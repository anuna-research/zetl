//! Capability-mode feed handling per REQ-3829..REQ-3831 + NFR-3809 +
//! ADR-3811 + CON-3813.
//!
//! Two parallel concerns:
//!
//! 1. **EXCLUSION** — the public visibility predicate produced by
//!    [`public_visibility`] excludes any page whose canonical URL
//!    contains a capability token segment. Used by `feed::build`,
//!    `feed::scoped`, and `feed::changelog` so cap-protected pages
//!    NEVER reach a public surface.
//!
//! 2. **COHORT FEED EMISSION** — for each enabled
//!    `[[capability_cohorts]]`, emit a feed under
//!    `/caps/<cohort.token>/feed.xml` (+ `atom.xml` + optional
//!    `feed.json`) containing only items whose ACL allows the cohort
//!    AND whose slug matches the cohort's `select` glob.
//!
//! Token-leak prevention per REQ-3831: the cohort *token* never
//! escapes its URL segment. Observability uses the operator-readable
//! `cohort.id`. The grep probe for token leakage lives in the
//! trust-boundary test suite (`tests/feed_trust_boundary.rs`).

use crate::feed::build::{build_feed_config, BuildError, FeedEmission};
use crate::feed::config::{CapabilityCohortSection, FeedConfigLens};
use crate::feed::select::PageView;
use crate::feed::serialise_atom::serialise_atom;
use crate::feed::serialise_jsonfeed::serialise_jsonfeed;
use crate::feed::serialise_rss::serialise_rss;
use crate::feed::types::{FeedPaths, OutputFormatSet};
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::time::Instant;

/// URL-segment marker the public visibility predicate looks for. Any
/// page whose slug or path contains `/caps/<token>/` is cap-protected.
const CAPS_SEGMENT: &str = "/caps/";

/// Build the public visibility predicate that callers feed into
/// `feed::select`. Returns `true` for every page whose URL doesn't
/// contain a `/caps/<token>/` segment.
///
/// The predicate is `Send + Sync + 'static` so the route handlers can
/// stash it in axum state.
pub fn public_visibility() -> impl Fn(&PageView<'_>) -> bool + Send + Sync + 'static {
    |page: &PageView<'_>| !is_cap_protected(page)
}

/// Cohort visibility predicate factory. Returns a closure that allows
/// pages whose slug matches any glob in the cohort's `select` list.
/// The shell composes this with the per-request ACL predicate via
/// boolean AND.
pub fn cohort_visibility(
    cohort: &CapabilityCohortSection,
) -> Result<impl Fn(&PageView<'_>) -> bool + Send + Sync + 'static, BuildError> {
    let mut builder = GlobSetBuilder::new();
    for pattern in &cohort.select {
        let glob = Glob::new(pattern).map_err(|_| BuildError::Disabled)?;
        builder.add(glob);
    }
    let set: GlobSet = builder.build().map_err(|_| BuildError::Disabled)?;
    Ok(move |page: &PageView<'_>| set.is_match(page.slug))
}

fn is_cap_protected(page: &PageView<'_>) -> bool {
    // The slug check accepts either a top-level `caps/<token>/...`
    // page or an arbitrarily-nested `*/caps/<token>/...` page; we
    // require the segment boundary so legitimate slugs like
    // `landscaps/x` or `caps-lock` don't get hidden.
    let slug = page.slug;
    let is_top_level = slug == "caps" || slug.starts_with("caps/");
    let is_nested = slug.contains("/caps/");
    is_top_level || is_nested || page.url_contains_caps()
}

trait UrlContainsCaps {
    fn url_contains_caps(&self) -> bool;
}

impl UrlContainsCaps for PageView<'_> {
    fn url_contains_caps(&self) -> bool {
        self.item.url.contains(CAPS_SEGMENT)
    }
}

/// Emit per-cohort feeds under `/caps/<token>/`. Returns one
/// [`FeedEmission`] per cohort with `feed_enabled = true`.
///
/// `acl_predicate` is the per-cohort ACL check supplied by the shell:
/// `true` iff the cohort's grant allows reading this page. Cohort feeds
/// emit only the intersection of `cohort.select` and the ACL predicate.
pub fn emit_cohort_feeds(
    lens: &FeedConfigLens,
    pages: &[PageView<'_>],
    acl_predicate: &dyn Fn(&str, &PageView<'_>) -> bool,
) -> Result<Vec<FeedEmission>, BuildError> {
    let mut out = Vec::new();
    for cohort in &lens.capability_cohorts {
        if !cohort.feed_enabled {
            continue;
        }
        out.push(emit_one_cohort(lens, cohort, pages, acl_predicate)?);
    }
    Ok(out)
}

fn emit_one_cohort(
    lens: &FeedConfigLens,
    cohort: &CapabilityCohortSection,
    pages: &[PageView<'_>],
    acl_predicate: &dyn Fn(&str, &PageView<'_>) -> bool,
) -> Result<FeedEmission, BuildError> {
    let started = Instant::now();
    let cohort_match = cohort_visibility(cohort)?;
    let visibility = |p: &PageView<'_>| cohort_match(p) && acl_predicate(&cohort.id, p);
    let max_items = lens
        .feed
        .as_ref()
        .and_then(|f| f.max_items)
        .unwrap_or(crate::feed::build::DEFAULT_MAX_ITEMS);

    // Cohort feeds don't require the public `feed: true` opt-in
    // (the cohort's `select` glob is the authoritative selector).
    // We collect every visibility-passing page, then apply the same
    // (date desc, id asc) sort + `max_items` truncation that
    // `select::select` enforces for REQ-3802 determinism.
    let mut chosen: Vec<crate::feed::types::FeedItem> = pages
        .iter()
        .filter(|p| visibility(p))
        .map(|p| p.item.clone())
        .collect();
    chosen.sort_by(|a, b| {
        b.date_published
            .cmp(&a.date_published)
            .then_with(|| a.id.cmp(&b.id))
    });
    chosen.truncate(max_items as usize);

    let token_segment = &cohort.token;
    let atom_path = format!("/caps/{token_segment}/atom.xml");
    let rss_path = format!("/caps/{token_segment}/feed.xml");
    let json_path = format!("/caps/{token_segment}/feed.json");
    let formats = OutputFormatSet {
        rss: true,
        atom: true,
        jsonfeed: lens
            .feed
            .as_ref()
            .and_then(|f| f.enable_json)
            .unwrap_or(false),
    };
    let title = cohort
        .feed_title
        .clone()
        .unwrap_or_else(|| format!("Cohort {}", cohort.id));
    let cfg = build_feed_config(
        lens,
        FeedPaths {
            rss: rss_path.clone(),
            atom: atom_path.clone(),
            jsonfeed: json_path.clone(),
        },
        formats,
        title,
        None,
        Some(cohort.id.clone()),
    )?;

    let mut files = Vec::with_capacity(3);
    files.push((rss_path.clone(), serialise_rss(&chosen, &cfg).into_bytes()));
    files.push((
        atom_path.clone(),
        serialise_atom(&chosen, &cfg).into_bytes(),
    ));
    if formats.jsonfeed {
        files.push((
            json_path.clone(),
            serialise_jsonfeed(&chosen, &cfg).into_bytes(),
        ));
    }
    let stats = crate::feed::build::BuildStats {
        items_selected: pages.iter().filter(|p| visibility(p)).count(),
        items_emitted: chosen.len(),
        duration: started.elapsed(),
        formats,
    };
    Ok(FeedEmission {
        files,
        stats,
        // No discovery tags — cohort feeds are NEVER advertised.
        discovery_tags: Vec::new(),
    })
}

/// REQ-3831 token-leak audit. Returns every byte-position where the
/// supplied token appears in `body` outside the `/caps/<token>/`
/// segment. Used by the trust-boundary test suite. Empty result =
/// no leakage.
pub fn audit_token_leakage(token: &str, body: &str) -> Vec<usize> {
    let mut leaks = Vec::new();
    let url_segment = format!("/caps/{token}/");
    let mut position = 0usize;
    while let Some(idx) = body[position..].find(token) {
        let abs = position + idx;
        let context_start = abs.saturating_sub(7);
        let context_end = (abs + url_segment.len()).min(body.len());
        let context = &body[context_start..context_end];
        if !context.contains(&url_segment) {
            leaks.push(abs);
        }
        position = abs + token.len();
    }
    leaks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::config::parse_config;
    use crate::feed::types::{FeedItem, License, SourceMetadata};
    use std::path::Path;

    fn strong_token() -> String {
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string()
    }

    fn lens_with_cohort() -> FeedConfigLens {
        let body = format!(
            r#"
            [feed]
            base_url = "https://example.com"
            title = "T"
            description = "d"

            [[capability_cohorts]]
            id = "alpha"
            token = "{}"
            select = ["alpha/**"]
            feed_enabled = true
            feed_title = "Alpha Cohort"
            "#,
            strong_token()
        );
        parse_config(&body).unwrap()
    }

    fn page<'a>(slug: &'a str, url: &'a str) -> PageView<'a> {
        let path: &'a Path = Path::new("p.md");
        let tags: &'a [String] = &[];
        PageView {
            slug,
            path,
            frontmatter_feed_optin: true,
            tags,
            matches_spl_query: false,
            item: FeedItem {
                id: format!("tag:example.com,2026:zetl/{slug}"),
                title: slug.to_string(),
                url: url.to_string(),
                date_published: "2026-05-08T00:00:00Z".to_string(),
                date_modified: None,
                summary: None,
                content_html: None,
                author: None,
                tags: Vec::new(),
                license: Some(License::Cc0_1_0),
                source_metadata: SourceMetadata::default(),
            },
        }
    }

    #[test]
    fn public_visibility_excludes_cap_protected_url() {
        let pred = public_visibility();
        let token = "deadbeef".repeat(4);
        let cap_url = format!("https://example.com/caps/{token}/secret");
        assert!(!pred(&page("secret", &cap_url)));
        assert!(pred(&page("public", "https://example.com/public")));
    }

    #[test]
    fn cohort_visibility_filters_by_glob() {
        let lens = lens_with_cohort();
        let cohort = &lens.capability_cohorts[0];
        let pred = cohort_visibility(cohort).unwrap();
        assert!(pred(&page("alpha/x", "https://example.com/alpha/x")));
        assert!(!pred(&page("beta/x", "https://example.com/beta/x")));
    }

    #[test]
    fn emit_cohort_feeds_writes_under_caps_segment() {
        let lens = lens_with_cohort();
        let pages = vec![
            page("alpha/x", "https://example.com/alpha/x"),
            page("alpha/y", "https://example.com/alpha/y"),
            page("beta/z", "https://example.com/beta/z"),
        ];
        let acl = |_cohort_id: &str, _p: &PageView<'_>| true;
        let emissions = emit_cohort_feeds(&lens, &pages, &acl).unwrap();
        assert_eq!(emissions.len(), 1);
        let token = strong_token();
        let paths: Vec<&str> = emissions[0].files.iter().map(|(p, _)| p.as_str()).collect();
        for path in &paths {
            assert!(path.starts_with(&format!("/caps/{token}/")), "got {path}");
        }
        // Items selected = 2 (alpha/x + alpha/y), beta/z excluded.
        assert_eq!(emissions[0].stats.items_emitted, 2);
        // No discovery tags — cohort feeds aren't advertised.
        assert!(emissions[0].discovery_tags.is_empty());
    }

    #[test]
    fn cohort_feed_disabled_when_feed_enabled_false() {
        let body = format!(
            r#"
            [feed]
            base_url = "https://example.com"
            title = "T"

            [[capability_cohorts]]
            id = "private"
            token = "{}"
            select = ["**"]
            feed_enabled = false
            "#,
            strong_token()
        );
        let lens = parse_config(&body).unwrap();
        let pages: Vec<PageView<'_>> = Vec::new();
        let acl = |_cohort_id: &str, _p: &PageView<'_>| true;
        let emissions = emit_cohort_feeds(&lens, &pages, &acl).unwrap();
        assert!(emissions.is_empty());
    }

    #[test]
    fn audit_token_leakage_finds_naked_token() {
        let token = strong_token();
        let body = format!(
            "<a href=\"/caps/{token}/feed.xml\">cohort feed</a>\n\
             leaked: {token} (oops)"
        );
        let leaks = audit_token_leakage(&token, &body);
        // One legitimate occurrence (in URL) + one naked = one leak.
        assert_eq!(leaks.len(), 1);
    }

    #[test]
    fn audit_token_leakage_empty_when_only_in_url() {
        let token = strong_token();
        let body = format!("<a href=\"/caps/{token}/feed.xml\">cohort feed</a>");
        assert!(audit_token_leakage(&token, &body).is_empty());
    }
}
