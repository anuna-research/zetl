//! Pure-core page-selection function per ADR-3802 + REQ-3802.
//!
//! `select` filters a vault snapshot by the chosen [`SelectionRule`] and
//! by a caller-supplied visibility predicate (collab read-ACL / public
//! build / cohort cap), sorts the survivors by `date_published`
//! descending with a stable tiebreak on `id`, then truncates to the
//! configured `max_items`.
//!
//! The pure core knows nothing about ACLs or cohorts — visibility is
//! injected as a closure so the shell can compute the predicate from
//! whatever request-time state it owns. Capability-protected pages
//! (URLs containing a cap token segment) are excluded by simply having
//! the public-visibility predicate return `false`; cohort feeds pass a
//! predicate that returns `true` only for in-cohort slugs.
//!
//! Per REQ-3802 the function is deterministic — same inputs always
//! produce the same output set in the same order.

use crate::feed::types::{FeedItem, SelectionRule};

/// Minimal view of a vault page that selection needs. The shell builds
/// these by walking its scanner output — we do not reach into the
/// concrete `web::context::VaultContext`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageView<'a> {
    pub slug: &'a str,
    pub path: &'a std::path::Path,
    /// Frontmatter `feed: true` for the opt-in selection rule
    /// (REQ-3811's convention).
    pub frontmatter_feed_optin: bool,
    /// Tags from frontmatter.
    pub tags: &'a [String],
    /// True iff the slug is in the SPL-resolved set the shell handed
    /// us. Empty for non-SPL rules.
    pub matches_spl_query: bool,
    /// Pre-built [`FeedItem`]; the projection pipeline (resolve_date,
    /// item_id, license_resolve, ...) has already run.
    pub item: FeedItem,
}

/// Visibility predicate. The pure core treats it as opaque; the shell
/// wires it to the public-build predicate, the per-request collab
/// read-ACL, or the cohort-scoping check as appropriate.
pub type VisibilityPredicate<'a> = dyn Fn(&PageView<'_>) -> bool + 'a;

/// Run the selection pipeline.
pub fn select<'a>(
    pages: &[PageView<'a>],
    rule: &SelectionRule,
    visibility: &VisibilityPredicate<'_>,
    max_items: u32,
) -> Vec<FeedItem> {
    let mut chosen: Vec<&FeedItem> = pages
        .iter()
        .filter(|p| matches_rule(p, rule) && visibility(p))
        .map(|p| &p.item)
        .collect();
    // Sort by date_published descending; stable tiebreak on id.
    chosen.sort_by(|a, b| {
        b.date_published
            .cmp(&a.date_published)
            .then_with(|| a.id.cmp(&b.id))
    });
    chosen
        .into_iter()
        .take(max_items as usize)
        .cloned()
        .collect()
}

fn matches_rule(page: &PageView<'_>, rule: &SelectionRule) -> bool {
    match rule {
        SelectionRule::FrontmatterOptIn => page.frontmatter_feed_optin,
        SelectionRule::Folder { path } => page.path.starts_with(path),
        SelectionRule::Tag { tag } => page.tags.iter().any(|t| t == tag),
        SelectionRule::SplQuery { .. } => page.matches_spl_query,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::types::License;
    use std::path::Path;

    fn item(id: &str, date: &str) -> FeedItem {
        FeedItem {
            id: id.to_string(),
            title: id.to_string(),
            url: format!("https://example.com/{id}"),
            date_published: date.to_string(),
            date_modified: None,
            summary: None,
            content_html: None,
            author: None,
            tags: Vec::new(),
            license: Some(License::Cc0_1_0),
            source_metadata: Default::default(),
        }
    }

    fn view<'a>(
        slug: &'a str,
        path: &'a Path,
        feed: bool,
        tags: &'a [String],
        matches_spl: bool,
        item: FeedItem,
    ) -> PageView<'a> {
        PageView {
            slug,
            path,
            frontmatter_feed_optin: feed,
            tags,
            matches_spl_query: matches_spl,
            item,
        }
    }

    fn always_visible(_: &PageView<'_>) -> bool {
        true
    }

    #[test]
    fn frontmatter_opt_in_selects_only_marked_pages() {
        let no_tags: Vec<String> = vec![];
        let p_alpha = Path::new("alpha.md");
        let p_beta = Path::new("beta.md");
        let pages = vec![
            view("alpha", p_alpha, true, &no_tags, false, item("alpha", "2026-05-01T00:00:00Z")),
            view("beta", p_beta, false, &no_tags, false, item("beta", "2026-05-02T00:00:00Z")),
        ];
        let out = select(&pages, &SelectionRule::FrontmatterOptIn, &always_visible, 10);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "alpha");
    }

    #[test]
    fn folder_rule_matches_path_prefix() {
        let no_tags: Vec<String> = vec![];
        let p_a = Path::new("notes/a.md");
        let p_b = Path::new("blog/b.md");
        let pages = vec![
            view("a", p_a, false, &no_tags, false, item("a", "2026-05-01T00:00:00Z")),
            view("b", p_b, false, &no_tags, false, item("b", "2026-05-02T00:00:00Z")),
        ];
        let out = select(
            &pages,
            &SelectionRule::Folder {
                path: "notes".into(),
            },
            &always_visible,
            10,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "a");
    }

    #[test]
    fn tag_rule_filters_by_tag() {
        let p = Path::new("p.md");
        let tags_alpha = vec!["alpha".to_string()];
        let tags_beta = vec!["beta".to_string()];
        let pages = vec![
            view("a", p, false, &tags_alpha, false, item("a", "2026-05-01T00:00:00Z")),
            view("b", p, false, &tags_beta, false, item("b", "2026-05-02T00:00:00Z")),
        ];
        let out = select(
            &pages,
            &SelectionRule::Tag {
                tag: "alpha".to_string(),
            },
            &always_visible,
            10,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "a");
    }

    #[test]
    fn spl_query_uses_pre_resolved_match_flag() {
        let p = Path::new("p.md");
        let no_tags: Vec<String> = vec![];
        let pages = vec![
            view("a", p, false, &no_tags, true, item("a", "2026-05-01T00:00:00Z")),
            view("b", p, false, &no_tags, false, item("b", "2026-05-02T00:00:00Z")),
        ];
        let out = select(
            &pages,
            &SelectionRule::SplQuery {
                query: "...".to_string(),
            },
            &always_visible,
            10,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "a");
    }

    #[test]
    fn sort_by_date_descending_with_id_tiebreak() {
        let p = Path::new("p.md");
        let no_tags: Vec<String> = vec![];
        // Two items at the same timestamp -> id tiebreaks lexicographically.
        let pages = vec![
            view("z", p, true, &no_tags, false, item("z", "2026-05-01T00:00:00Z")),
            view("a", p, true, &no_tags, false, item("a", "2026-05-01T00:00:00Z")),
            view("m", p, true, &no_tags, false, item("m", "2026-06-01T00:00:00Z")),
        ];
        let out = select(&pages, &SelectionRule::FrontmatterOptIn, &always_visible, 10);
        assert_eq!(out.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), vec!["m", "a", "z"]);
    }

    #[test]
    fn max_items_truncates_after_sort() {
        let p = Path::new("p.md");
        let no_tags: Vec<String> = vec![];
        let pages: Vec<_> = (0..10)
            .map(|i| {
                let date = format!("2026-{:02}-01T00:00:00Z", i + 1);
                let id = format!("p{i}");
                view("x", p, true, &no_tags, false, item(&id, &date))
            })
            .collect();
        let out = select(&pages, &SelectionRule::FrontmatterOptIn, &always_visible, 3);
        assert_eq!(out.len(), 3);
        // Newest 3.
        assert_eq!(out[0].id, "p9");
        assert_eq!(out[1].id, "p8");
        assert_eq!(out[2].id, "p7");
    }

    #[test]
    fn visibility_predicate_excludes_pages() {
        let p = Path::new("p.md");
        let no_tags: Vec<String> = vec![];
        let pages = vec![
            view("public", p, true, &no_tags, false, item("public", "2026-05-01T00:00:00Z")),
            view("private", p, true, &no_tags, false, item("private", "2026-05-02T00:00:00Z")),
        ];
        let only_public = |p: &PageView<'_>| p.slug == "public";
        let out = select(&pages, &SelectionRule::FrontmatterOptIn, &only_public, 10);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "public");
    }

    #[test]
    fn deterministic_for_shuffled_input() {
        let p = Path::new("p.md");
        let no_tags: Vec<String> = vec![];
        let order_a = vec![
            view("a", p, true, &no_tags, false, item("a", "2026-05-01T00:00:00Z")),
            view("b", p, true, &no_tags, false, item("b", "2026-05-02T00:00:00Z")),
            view("c", p, true, &no_tags, false, item("c", "2026-05-03T00:00:00Z")),
        ];
        let order_b = vec![
            view("c", p, true, &no_tags, false, item("c", "2026-05-03T00:00:00Z")),
            view("a", p, true, &no_tags, false, item("a", "2026-05-01T00:00:00Z")),
            view("b", p, true, &no_tags, false, item("b", "2026-05-02T00:00:00Z")),
        ];
        let out_a = select(&order_a, &SelectionRule::FrontmatterOptIn, &always_visible, 10);
        let out_b = select(&order_b, &SelectionRule::FrontmatterOptIn, &always_visible, 10);
        assert_eq!(out_a, out_b);
    }
}
