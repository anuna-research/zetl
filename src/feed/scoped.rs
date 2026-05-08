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
use std::time::Instant;

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
pub fn emit_scoped_feeds(
    lens: &FeedConfigLens,
    pages: &[PageView<'_>],
    visibility: &dyn Fn(&PageView<'_>) -> bool,
) -> Result<Vec<FeedEmission>, BuildError> {
    let feed = lens.feed.as_ref().ok_or(BuildError::Disabled)?;
    if feed.scopes.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity(feed.scopes.len());
    for scope in &feed.scopes {
        let emission = emit_scope(lens, scope, pages, visibility)?;
        out.push(emission);
    }
    Ok(out)
}

fn emit_scope(
    lens: &FeedConfigLens,
    scope: &FeedScope,
    pages: &[PageView<'_>],
    visibility: &dyn Fn(&PageView<'_>) -> bool,
) -> Result<FeedEmission, BuildError> {
    let started = Instant::now();
    let rule: SelectionRule =
        parse_selection_rule(&scope.select).map_err(|_| BuildError::Disabled)?;
    let max_items = lens
        .feed
        .as_ref()
        .and_then(|f| f.max_items)
        .unwrap_or(crate::feed::build::DEFAULT_MAX_ITEMS);
    let chosen = crate::feed::select::select(pages, &rule, visibility, max_items);

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
    files.push((atom_path.clone(), serialise_atom(&chosen, &cfg).into_bytes()));
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
        let emissions = emit_scoped_feeds(&lens(), &pages, &always).unwrap();
        assert_eq!(emissions.len(), 2);
    }

    #[test]
    fn scoped_emission_deterministic() {
        let pages: Vec<PageView<'_>> = Vec::new();
        let always = |_: &PageView<'_>| true;
        let a = emit_scoped_feeds(&lens(), &pages, &always).unwrap();
        let b = emit_scoped_feeds(&lens(), &pages, &always).unwrap();
        for (ea, eb) in a.iter().zip(b.iter()) {
            assert_eq!(ea.files, eb.files);
        }
    }
}
