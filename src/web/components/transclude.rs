//! SPEC-048 REQ-4818 / CON-4806 / ADR-4810 — Addressed vault transclusion.
//!
//! A read-only, content-addressed read of a *named* page (`page`, `page#heading`, or
//! `page#^block`), distinct from the forbidden *ambient* page context (Threat G). The
//! exposed surface is a **closed allow-list** — rendered HTML plus `title` and
//! publishable frontmatter only; backlinks, edges, and raw frontmatter are unreachable
//! (Threat H). Targets are recognised before resolution; unresolved/draft targets fail
//! closed (`transclude-target-unresolved`). Depth/cycles reuse the REQ-4807 bound.

use super::nesting::check_depth;
use super::{CResult, ComponentError};
use std::collections::BTreeMap;

/// A parsed transclusion target (CON-4806 grammar).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub page: String,
    pub heading: Option<String>,
    pub block_id: Option<String>,
}

/// Parse `page[#heading|#^block]` — the existing Embed wikilink-target grammar.
pub fn parse_target(raw: &str) -> CResult<Target> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(ComponentError::new(
            "transclude-target-unresolved",
            "empty transclude target",
        ));
    }
    // split off alias if present (ignored for transclusion)
    let raw = raw.split('|').next().unwrap_or(raw).trim();
    let (page, heading, block_id) = if let Some((p, rest)) = raw.split_once('#') {
        if let Some(bid) = rest.strip_prefix('^') {
            (p.trim(), None, Some(bid.trim().to_string()))
        } else {
            (p.trim(), Some(rest.trim().to_string()), None)
        }
    } else {
        (raw, None, None)
    };
    if page.is_empty() {
        return Err(ComponentError::new(
            "transclude-target-unresolved",
            "transclude target has no page",
        ));
    }
    Ok(Target {
        page: page.to_string(),
        heading,
        block_id,
    })
}

/// The closed metadata allow-list exposed by a transclusion (CON-4806). Deliberately
/// carries no backlinks / edges / raw frontmatter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TranscludeMeta {
    pub title: String,
    /// Publishable frontmatter fields only.
    pub frontmatter: BTreeMap<String, String>,
}

/// The result of a transclusion: rendered HTML + the allow-listed metadata.
#[derive(Debug, Clone)]
pub struct Transcluded {
    pub html: String,
    pub meta: TranscludeMeta,
}

/// The vault read surface a transclusion needs. The implementor (engine) enforces
/// visibility (draft pages return `None`) and the allow-list (only publishable fields
/// populate [`TranscludeMeta`]).
pub trait VaultLookup {
    /// Render the addressed fragment to HTML, or `None` if the page/fragment is
    /// unresolved or the page is not publishable.
    fn render_fragment(
        &self,
        page: &str,
        heading: Option<&str>,
        block_id: Option<&str>,
    ) -> Option<String>;

    /// Allow-listed metadata for a (publishable) page, or `None`.
    fn metadata(&self, page: &str) -> Option<TranscludeMeta>;
}

/// Resolve a transclusion. `depth` is the current nest depth (REQ-4807 bound).
pub fn transclude(
    raw_target: &str,
    lookup: &dyn VaultLookup,
    depth: usize,
) -> CResult<Transcluded> {
    check_depth(depth)?;
    let target = parse_target(raw_target)?;
    let html = lookup
        .render_fragment(
            &target.page,
            target.heading.as_deref(),
            target.block_id.as_deref(),
        )
        .ok_or_else(|| {
            ComponentError::new(
                "transclude-target-unresolved",
                format!("transclude target `{raw_target}` is unresolved or not publishable"),
            )
        })?;
    let meta = lookup.metadata(&target.page).unwrap_or_default();
    Ok(Transcluded { html, meta })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_page_only() {
        let t = parse_target("handbook").unwrap();
        assert_eq!(t.page, "handbook");
        assert_eq!(t.heading, None);
        assert_eq!(t.block_id, None);
    }

    #[test]
    fn parses_heading_and_block() {
        let h = parse_target("handbook#mission").unwrap();
        assert_eq!(h.page, "handbook");
        assert_eq!(h.heading.as_deref(), Some("mission"));
        let b = parse_target("handbook#^intro").unwrap();
        assert_eq!(b.block_id.as_deref(), Some("intro"));
        assert_eq!(b.heading, None);
    }

    #[test]
    fn empty_target_rejected() {
        assert_eq!(
            parse_target("").unwrap_err().code,
            "transclude-target-unresolved"
        );
    }

    struct Mock;
    impl VaultLookup for Mock {
        fn render_fragment(
            &self,
            page: &str,
            heading: Option<&str>,
            _b: Option<&str>,
        ) -> Option<String> {
            if page == "handbook" {
                Some(match heading {
                    Some("mission") => "<p>Our mission.</p>".to_string(),
                    None => "<h1>Handbook</h1>".to_string(),
                    _ => return None,
                })
            } else {
                None // unknown or draft
            }
        }
        fn metadata(&self, page: &str) -> Option<TranscludeMeta> {
            if page == "handbook" {
                Some(TranscludeMeta {
                    title: "Handbook".to_string(),
                    frontmatter: BTreeMap::new(),
                })
            } else {
                None
            }
        }
    }

    #[test]
    fn renders_section() {
        let r = transclude("handbook#mission", &Mock, 0).unwrap();
        assert_eq!(r.html, "<p>Our mission.</p>");
        assert_eq!(r.meta.title, "Handbook");
    }

    #[test]
    fn unresolved_fails_closed() {
        assert_eq!(
            transclude("ghost", &Mock, 0).unwrap_err().code,
            "transclude-target-unresolved"
        );
        assert_eq!(
            transclude("handbook#nope", &Mock, 0).unwrap_err().code,
            "transclude-target-unresolved"
        );
    }

    #[test]
    fn depth_bound_applies() {
        assert_eq!(
            transclude("handbook", &Mock, 99).unwrap_err().code,
            "component-depth-bound"
        );
    }
}
