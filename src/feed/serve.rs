//! Outbound serve-mode handlers per task-serve-outbound (REQ-3801,
//! CON-3802, CON-3803).
//!
//! Effectful shell, but wraps the same pure pipeline as `feed::build`.
//! The actual axum/tower wiring lives in `web::routes`; this module
//! provides the per-format response *builders* so the route handlers
//! reduce to a one-liner.

use crate::feed::build::{
    emit_root_feed, BuildError, DEFAULT_ATOM_PATH, DEFAULT_JSONFEED_PATH, DEFAULT_RSS_PATH,
};
use crate::feed::config::FeedConfigLens;
use crate::feed::select::PageView;
use crate::feed::types::{OutputFormat, SelectionRule};

/// Cache-Control freshness window for unauthenticated feeds.
pub const PUBLIC_CACHE_MAX_AGE_SECS: u32 = 300;

/// Built response shape for a feed handler. The route handler maps
/// these fields onto axum types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedResponse {
    pub status: u16,
    pub content_type: &'static str,
    pub cache_control: String,
    pub body: Vec<u8>,
}

/// Resolve the URL path the operator configured for `format`. Falls
/// back to the per-format default when `[feed.paths]` doesn't override
/// it. Pure: pulls everything out of the lens.
pub fn configured_feed_path(lens: &FeedConfigLens, format: OutputFormat) -> String {
    let default = match format {
        OutputFormat::Rss20 => DEFAULT_RSS_PATH,
        OutputFormat::Atom10 => DEFAULT_ATOM_PATH,
        OutputFormat::JsonFeed11 => DEFAULT_JSONFEED_PATH,
    };
    lens.feed
        .as_ref()
        .and_then(|f| f.paths.as_ref())
        .and_then(|p| match format {
            OutputFormat::Rss20 => p.rss.clone(),
            OutputFormat::Atom10 => p.atom.clone(),
            OutputFormat::JsonFeed11 => p.jsonfeed.clone(),
        })
        .unwrap_or_else(|| default.to_string())
}

/// Reverse of [`configured_feed_path`]: classify a request URL path
/// against the operator's [feed.paths] config (falling back to the
/// per-format defaults). Returns the matching format or `None`. Used
/// by the route dispatcher so requests to a custom path like
/// `/rss.xml` reach the correct format handler when the operator has
/// remapped feed URLs.
pub fn classify_feed_path(lens: &FeedConfigLens, url_path: &str) -> Option<OutputFormat> {
    [
        OutputFormat::Rss20,
        OutputFormat::Atom10,
        OutputFormat::JsonFeed11,
    ]
    .into_iter()
    .find(|&fmt| configured_feed_path(lens, fmt) == url_path)
}

/// Render a feed for serve mode. `format` selects which file from the
/// emission set we return; `is_collab` chooses no-store (collab) vs
/// max-age (public) caching.
pub fn render_feed(
    lens: &FeedConfigLens,
    pages: &[PageView<'_>],
    visibility: &dyn Fn(&PageView<'_>) -> bool,
    rule: &SelectionRule,
    format: OutputFormat,
    is_collab: bool,
) -> Result<FeedResponse, BuildError> {
    let emission = emit_root_feed(lens, pages, visibility, rule)?;
    let target_path = configured_feed_path(lens, format);
    let body = emission
        .files
        .iter()
        .find(|(p, _)| p == &target_path)
        .map(|(_, b)| b.clone())
        .ok_or(BuildError::Disabled)?;
    let cache_control = if is_collab {
        "no-store".to_string()
    } else {
        format!("public, max-age={PUBLIC_CACHE_MAX_AGE_SECS}")
    };
    Ok(FeedResponse {
        status: 200,
        content_type: format.content_type(),
        cache_control,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::config::parse_config;
    use crate::feed::types::{FeedItem, License, SourceMetadata};
    use std::path::Path;

    fn lens() -> FeedConfigLens {
        parse_config(
            r#"
            [feed]
            base_url = "https://example.com"
            title = "T"
            description = "d"
            enable_json = true
        "#,
        )
        .unwrap()
    }

    fn page<'a>() -> Vec<PageView<'a>> {
        let path: &'a Path = Path::new("p.md");
        let tags: &'a [String] = &[];
        vec![PageView {
            slug: "x",
            path,
            frontmatter_feed_optin: true,
            tags,
            matches_spl_query: false,
            item: FeedItem {
                id: "tag:example.com,2026:zetl/x".to_string(),
                title: "x".to_string(),
                url: "https://example.com/x".to_string(),
                date_published: "2026-05-08T00:00:00Z".to_string(),
                date_modified: None,
                summary: None,
                content_html: None,
                author: None,
                tags: Vec::new(),
                license: Some(License::Cc0_1_0),
                source_metadata: SourceMetadata::default(),
            },
        }]
    }

    fn always(_: &PageView<'_>) -> bool {
        true
    }

    #[test]
    fn rss_response_carries_correct_content_type() {
        let resp = render_feed(
            &lens(),
            &page(),
            &always,
            &SelectionRule::FrontmatterOptIn,
            OutputFormat::Rss20,
            false,
        )
        .unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.content_type, "application/rss+xml; charset=utf-8");
        assert_eq!(resp.cache_control, "public, max-age=300");
        assert!(!resp.body.is_empty());
    }

    #[test]
    fn collab_uses_no_store() {
        let resp = render_feed(
            &lens(),
            &page(),
            &always,
            &SelectionRule::FrontmatterOptIn,
            OutputFormat::Atom10,
            true,
        )
        .unwrap();
        assert_eq!(resp.cache_control, "no-store");
    }

    #[test]
    fn jsonfeed_when_enabled() {
        let resp = render_feed(
            &lens(),
            &page(),
            &always,
            &SelectionRule::FrontmatterOptIn,
            OutputFormat::JsonFeed11,
            false,
        )
        .unwrap();
        assert_eq!(resp.content_type, "application/feed+json; charset=utf-8");
    }
}
