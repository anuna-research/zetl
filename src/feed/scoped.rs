//! Hugo's scoped subscription catalog + per-scope feed emission per
//! REQ-3813..REQ-3815 + CON-3808.
//!
//! Two outputs:
//!
//! 1. `/.well-known/zetl-subscriptions.json` — public catalog
//!    advertising the source wiki id, root feed URL, scoped feeds, and
//!    selectable selector vocabulary. Cap cohorts are NEVER advertised
//!    here (REQ-3829).
//!
//! 2. Per-scope feed bodies (RSS + Atom + optional JSON Feed) under
//!    each scope's configured `path`.

use crate::feed::build::{build_feed_config, BuildError, FeedEmission};
use crate::feed::config::{parse_selection_rule, FeedConfigLens, FeedScope};
use crate::feed::select::PageView;
use crate::feed::serialise_atom::serialise_atom;
use crate::feed::serialise_jsonfeed::serialise_jsonfeed;
use crate::feed::serialise_rss::serialise_rss;
use crate::feed::types::{FeedPaths, OutputFormatSet, SelectionRule};
use std::collections::HashSet;
use std::time::Instant;

/// Closure handed in by the shell to evaluate an SPL query against the
/// vault and return the matching slug set. Required for any
/// `[[feed.scopes]]` whose `select.spl = "..."` rule is configured;
/// when no resolver is supplied we emit a runtime warning and the
/// scope selects nothing (preserving the prior — broken — behaviour
/// without changing semantics for vaults that don't use SPL scopes).
pub type SplResolver<'a> = dyn Fn(&str) -> HashSet<String> + 'a;

/// Schema version of `/.well-known/zetl-subscriptions.json`.
pub const CATALOG_SCHEMA_VERSION: &str = "1.0";

/// Catalog file path under `dist/`.
pub const CATALOG_PATH: &str = "/.well-known/zetl-subscriptions.json";

/// Wiki identity passed in by the shell (read from `[wiki]` section).
#[derive(Debug, Clone, Default)]
pub struct WikiIdentity {
    pub id: Option<String>,
    pub canonical_repo: Option<String>,
    pub title: Option<String>,
}

/// Emit `/.well-known/zetl-subscriptions.json` per REQ-3813. Returns
/// `(path, body_bytes)`.
pub fn emit_catalog(
    lens: &FeedConfigLens,
    identity: &WikiIdentity,
) -> Result<(String, Vec<u8>), BuildError> {
    let feed = lens.feed.as_ref().ok_or(BuildError::Disabled)?;
    let base_url = feed
        .base_url
        .as_deref()
        .ok_or(BuildError::MissingBaseUrl)?
        .trim_end_matches('/');

    let mut buf = String::with_capacity(1024);
    buf.push_str("{\n");
    buf.push_str(r#"  "zetl_subscription_catalog": "1.0","#);
    buf.push('\n');
    if let Some(id) = &identity.id {
        buf.push_str(r#"  "wiki_id": ""#);
        push_json(&mut buf, id);
        buf.push_str("\",\n");
    }
    if let Some(repo) = &identity.canonical_repo {
        buf.push_str(r#"  "canonical_repo": ""#);
        push_json(&mut buf, repo);
        buf.push_str("\",\n");
    }
    if let Some(title) = &identity.title {
        buf.push_str(r#"  "title": ""#);
        push_json(&mut buf, title);
        buf.push_str("\",\n");
    }
    let root_path = feed
        .paths
        .as_ref()
        .and_then(|p| p.atom.clone())
        .unwrap_or_else(|| crate::feed::build::DEFAULT_ATOM_PATH.to_string());
    buf.push_str(r#"  "root_feed": ""#);
    buf.push_str(base_url);
    push_json(&mut buf, &root_path);
    buf.push_str("\",\n");
    // Selector vocabulary (stable forms only per REQ-3813 — explicitly
    // NOT anonymous AST positions).
    buf.push_str(r#"  "selector_vocabulary": ["folder-subtree", "folder", "page", "stable-heading-anchor", "explicit-block-id"],"#);
    buf.push('\n');
    buf.push_str(r#"  "scopes": ["#);
    for (i, scope) in feed.scopes.iter().enumerate() {
        if i > 0 {
            buf.push(',');
        }
        buf.push_str("\n    {");
        buf.push_str(r#""id": ""#);
        push_json(&mut buf, &scope.id);
        buf.push_str(r#"", "title": ""#);
        push_json(&mut buf, &scope.title);
        buf.push_str(r#"", "feed_url": ""#);
        buf.push_str(base_url);
        push_json(&mut buf, &scope.path);
        buf.push_str("\"}");
    }
    if !feed.scopes.is_empty() {
        buf.push_str("\n  ");
    }
    buf.push_str("],\n");
    if let Some(c) = &feed.changelog {
        buf.push_str(r#"  "changelog": {"#);
        buf.push_str(r#""path": ""#);
        push_json(&mut buf, &c.path);
        buf.push_str(r#"", "archive_path": ""#);
        push_json(&mut buf, &c.archive_path);
        buf.push_str("\"}\n");
    } else {
        buf.push_str("  \"changelog\": null\n");
    }
    buf.push_str("}\n");
    Ok((CATALOG_PATH.to_string(), buf.into_bytes()))
}

fn push_json(buf: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '"' => buf.push_str("\\\""),
            '\\' => buf.push_str("\\\\"),
            '\n' => buf.push_str("\\n"),
            c if (c as u32) < 0x20 => buf.push_str(&format!("\\u{:04x}", c as u32)),
            c => buf.push(c),
        }
    }
}

/// Emit per-[[feed.scopes]] feeds. Returns one [`FeedEmission`] per
/// scope; the caller writes them into dist/.
///
/// When `spl_resolver` is `None`, scopes whose rule is `select.spl =
/// "..."` will warn and select nothing (preserving the previous —
/// broken — behaviour while keeping the vault build succeeding).
/// Callers that have a search index handy should pass a resolver so
/// SPL scopes actually populate.
pub fn emit_scoped_feeds(
    lens: &FeedConfigLens,
    pages: &[PageView<'_>],
    visibility: &dyn Fn(&PageView<'_>) -> bool,
    spl_resolver: Option<&SplResolver<'_>>,
) -> Result<Vec<FeedEmission>, BuildError> {
    let feed = lens.feed.as_ref().ok_or(BuildError::Disabled)?;
    if feed.scopes.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity(feed.scopes.len());
    for scope in &feed.scopes {
        let emission = emit_scope(lens, scope, pages, visibility, spl_resolver)?;
        out.push(emission);
    }
    Ok(out)
}

fn emit_scope(
    lens: &FeedConfigLens,
    scope: &FeedScope,
    pages: &[PageView<'_>],
    visibility: &dyn Fn(&PageView<'_>) -> bool,
    spl_resolver: Option<&SplResolver<'_>>,
) -> Result<FeedEmission, BuildError> {
    let started = Instant::now();
    let rule: SelectionRule =
        parse_selection_rule(&scope.select).map_err(|_| BuildError::Disabled)?;
    let max_items = lens
        .feed
        .as_ref()
        .and_then(|f| f.max_items)
        .unwrap_or(crate::feed::build::DEFAULT_MAX_ITEMS);
    // For SPL-scoped feeds, evaluate the query upfront and rebuild a
    // per-scope page set with `matches_spl_query` flagged on every slug
    // the resolver returns. The shared `pages` slice always carries
    // `matches_spl_query: false`, so without this rebuild the scope
    // would never select anything (the bug the reviewer flagged).
    let scoped_pages: Option<Vec<PageView<'_>>> = match (&rule, spl_resolver) {
        (SelectionRule::SplQuery { query }, Some(resolver)) => {
            let matched = resolver(query);
            Some(
                pages
                    .iter()
                    .map(|p| {
                        let mut np = p.clone();
                        np.matches_spl_query = matched.contains(p.slug);
                        np
                    })
                    .collect(),
            )
        }
        (SelectionRule::SplQuery { query }, None) => {
            eprintln!(
                "[zetl] feed: scope {scope_id:?} configured with select.spl = {query:?} \
                 but no SPL resolver was supplied to emit_scoped_feeds; this scope will \
                 select zero pages",
                scope_id = scope.id,
            );
            None
        }
        _ => None,
    };
    let pages_for_select: &[PageView<'_>] = scoped_pages.as_deref().unwrap_or(pages);
    let chosen = crate::feed::select::select(pages_for_select, &rule, visibility, max_items);

    // Build per-format paths anchored at the scope's path. The scope
    // is published as its own atom feed at exactly the configured
    // `path`; we synthesise companion RSS + JSON Feed paths by
    // suffix substitution (`.xml` -> default atom, plus `.rss.xml` /
    // `.json` siblings).
    let atom_path = scope.path.clone();
    let rss_path = scope.path.replace(".xml", ".rss.xml");
    let json_path = scope.path.replace(".xml", ".json");
    let formats = OutputFormatSet {
        rss: true,
        atom: true,
        jsonfeed: lens
            .feed
            .as_ref()
            .and_then(|f| f.enable_json)
            .unwrap_or(false),
    };
    let cfg = build_feed_config(
        lens,
        FeedPaths {
            rss: rss_path.clone(),
            atom: atom_path.clone(),
            jsonfeed: json_path.clone(),
        },
        formats,
        scope.title.clone(),
        Some(scope.id.clone()),
        None,
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
        items_selected: pages_for_select.iter().filter(|p| visibility(p)).count(),
        items_emitted: chosen.len(),
        duration: started.elapsed(),
        formats,
    };
    Ok(FeedEmission {
        files,
        stats,
        discovery_tags: crate::feed::build::discovery_tags(&cfg),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::config::parse_config;

    fn lens() -> FeedConfigLens {
        parse_config(
            r#"
            [feed]
            base_url = "https://example.com"
            title = "T"
            description = "d"
            enable_json = true

            [[feed.scopes]]
            id = "blog"
            title = "Blog"
            path = "/blog/feed.xml"
            select = "frontmatter"

            [[feed.scopes]]
            id = "notes"
            title = "Notes"
            path = "/notes/feed.xml"
            select.folder = "notes/"

            [feed.changelog]
            path = "/changelog.xml"
            archive_path = "/archives"
            archive_size = 1000
        "#,
        )
        .unwrap()
    }

    #[test]
    fn catalog_includes_every_scope() {
        let identity = WikiIdentity {
            id: Some("anuna".to_string()),
            ..Default::default()
        };
        let (path, body) = emit_catalog(&lens(), &identity).unwrap();
        assert_eq!(path, "/.well-known/zetl-subscriptions.json");
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["zetl_subscription_catalog"], "1.0");
        assert_eq!(v["wiki_id"], "anuna");
        let scopes = v["scopes"].as_array().unwrap();
        assert_eq!(scopes.len(), 2);
        assert_eq!(scopes[0]["id"], "blog");
        assert_eq!(scopes[1]["id"], "notes");
    }

    #[test]
    fn catalog_excludes_canonical_repo_when_unset() {
        let identity = WikiIdentity {
            id: Some("anuna".to_string()),
            canonical_repo: None,
            title: None,
        };
        let (_, body) = emit_catalog(&lens(), &identity).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(v.get("canonical_repo").is_none());
    }

    #[test]
    fn catalog_emits_changelog_pointer_when_configured() {
        let identity = WikiIdentity::default();
        let (_, body) = emit_catalog(&lens(), &identity).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["changelog"]["path"], "/changelog.xml");
    }

    #[test]
    fn scoped_emit_produces_one_emission_per_scope() {
        let pages: Vec<PageView<'_>> = Vec::new();
        let always = |_: &PageView<'_>| true;
        let emissions = emit_scoped_feeds(&lens(), &pages, &always, None).unwrap();
        assert_eq!(emissions.len(), 2);
    }

    #[test]
    fn spl_resolver_populates_scope_selection() {
        // A second `lens` with a single SPL-scoped feed.
        let lens_spl = parse_config(
            r#"
            [feed]
            base_url = "https://example.com"
            title = "T"
            description = "d"
            [[feed.scopes]]
            id = "blog"
            title = "Blog"
            path = "/blog/feed.xml"
            select.spl = "tag:blog"
        "#,
        )
        .unwrap();
        let path = std::path::PathBuf::from("a.md");
        let path_b = std::path::PathBuf::from("b.md");
        let tags: Vec<String> = vec![];
        let mk = |slug: &str, p: &std::path::Path| crate::feed::select::PageView {
            slug: Box::leak(slug.to_string().into_boxed_str()) as &str,
            path: Box::leak(p.to_path_buf().into_boxed_path()),
            frontmatter_feed_optin: true,
            tags: &tags,
            matches_spl_query: false,
            item: crate::feed::types::FeedItem {
                id: format!("urn:{slug}"),
                title: slug.to_string(),
                url: format!("https://example.com/{slug}"),
                date_published: "2026-05-08T00:00:00Z".to_string(),
                date_modified: None,
                summary: None,
                content_html: None,
                author: None,
                tags: Vec::new(),
                license: Some(crate::feed::types::License::Cc0_1_0),
                source_metadata: Default::default(),
            },
        };
        let pages = vec![mk("a", &path), mk("b", &path_b)];
        let always = |_: &PageView<'_>| true;
        let resolver = |_q: &str| {
            let mut s = std::collections::HashSet::new();
            s.insert("a".to_string());
            s
        };
        let emissions = emit_scoped_feeds(&lens_spl, &pages, &always, Some(&resolver)).unwrap();
        assert_eq!(emissions.len(), 1);
        // Atom body should contain `urn:a` (selected) and not `urn:b`.
        let atom = emissions[0]
            .files
            .iter()
            .find(|(p, _)| p == "/blog/feed.xml")
            .map(|(_, b)| std::str::from_utf8(b).unwrap().to_string())
            .unwrap();
        assert!(atom.contains("urn:a"), "expected urn:a in atom: {atom}");
        assert!(!atom.contains("urn:b"), "did not expect urn:b: {atom}");
    }

    #[test]
    fn scoped_emission_deterministic() {
        let pages: Vec<PageView<'_>> = Vec::new();
        let always = |_: &PageView<'_>| true;
        let a = emit_scoped_feeds(&lens(), &pages, &always, None).unwrap();
        let b = emit_scoped_feeds(&lens(), &pages, &always, None).unwrap();
        for (ea, eb) in a.iter().zip(b.iter()) {
            assert_eq!(ea.files, eb.files);
        }
    }
}
