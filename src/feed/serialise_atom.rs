//! Pure Atom 1.0 serialiser per REQ-3801 + REQ-3811 + REQ-3814 +
//! NFR-3805. Emits feed-level metadata, per-entry elements, and the
//! `_zetl:` namespace extension for source metadata. Deterministic.

use crate::feed::types::{FeedConfig, FeedItem, SourceMetadata};

const ZETL_NS: &str = "https://schema.zetl.dev/feed-extensions/v1";

/// Serialise a slice of [`FeedItem`] into an Atom 1.0 XML document.
pub fn serialise_atom(items: &[FeedItem], config: &FeedConfig) -> String {
    let mut out = String::with_capacity(8192);
    out.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    out.push('\n');
    // Atom 1.0 uses XML's `xml:lang` attribute on the root for the
    // human-readable language hint; emit it inline rather than abusing
    // `<rights>` for it.
    let lang_attr = config
        .language
        .as_deref()
        .map(|l| format!(r#" xml:lang="{}""#, escape_attr(l)))
        .unwrap_or_default();
    out.push_str(&format!(
        r#"<feed xmlns="http://www.w3.org/2005/Atom" xmlns:_zetl="{ZETL_NS}"{lang_attr}>"#
    ));
    out.push('\n');
    push_text(&mut out, "  ", "title", &config.title);
    push_text(&mut out, "  ", "subtitle", &config.description);
    let self_url = absolute_url(&config.base_url, &config.paths.atom);
    push_text(&mut out, "  ", "id", &self_url);
    if let Some(latest) = items.iter().map(|i| &i.date_published).max() {
        push_text(&mut out, "  ", "updated", latest);
    }
    out.push_str(&format!(
        r#"  <link rel="alternate" type="text/html" href="{}" />"#,
        escape_attr(&config.base_url)
    ));
    out.push('\n');
    out.push_str(&format!(
        r#"  <link rel="self" type="application/atom+xml" href="{}" />"#,
        escape_attr(&self_url)
    ));
    out.push('\n');
    if let Some(c) = &config.copyright {
        push_text(&mut out, "  ", "rights", c);
    }
    if let Some(a) = &config.default_author {
        push_author(&mut out, "  ", a);
    }
    for item in items {
        out.push_str("  <entry>\n");
        push_text(&mut out, "    ", "id", &item.id);
        push_text(&mut out, "    ", "title", &item.title);
        out.push_str(&format!(
            r#"    <link rel="alternate" type="text/html" href="{}" />"#,
            escape_attr(&item.url)
        ));
        out.push('\n');
        push_text(&mut out, "    ", "published", &item.date_published);
        if let Some(modified) = &item.date_modified {
            push_text(&mut out, "    ", "updated", modified);
        } else {
            push_text(&mut out, "    ", "updated", &item.date_published);
        }
        if let Some(a) = &item.author {
            push_author(&mut out, "    ", a);
        }
        for tag in &item.tags {
            out.push_str(&format!(
                r#"    <category term="{}" />"#,
                escape_attr(tag)
            ));
            out.push('\n');
        }
        if let Some(summary) = &item.summary {
            push_text(&mut out, "    ", "summary", summary);
        }
        if let Some(content) = &item.content_html {
            // REQ-3806: Atom content type=html uses character entity
            // escaping, not CDATA. (CDATA is the RSS path; Atom's
            // strict-parser conformance gate prefers entity-escaped
            // bodies.) escape_text expands &, <, >, leaving the rest
            // of the bytes intact — feed readers re-parse the entities
            // back to their HTML markup.
            out.push_str(r#"    <content type="html">"#);
            out.push_str(&escape_text(content));
            out.push_str("</content>\n");
        }
        push_zetl_metadata(&mut out, "    ", &item.source_metadata);
        out.push_str("  </entry>\n");
    }
    out.push_str("</feed>\n");
    out
}

fn push_author(out: &mut String, indent: &str, a: &crate::feed::types::AuthorRef) {
    out.push_str(indent);
    out.push_str("<author>\n");
    out.push_str(indent);
    out.push_str("  <name>");
    out.push_str(&escape_text(&a.name));
    out.push_str("</name>\n");
    if let Some(e) = &a.email {
        out.push_str(indent);
        out.push_str("  <email>");
        out.push_str(&escape_text(e));
        out.push_str("</email>\n");
    }
    if let Some(u) = &a.url {
        out.push_str(indent);
        out.push_str("  <uri>");
        out.push_str(&escape_text(u));
        out.push_str("</uri>\n");
    }
    out.push_str(indent);
    out.push_str("</author>\n");
}

fn push_zetl_metadata(out: &mut String, indent: &str, m: &SourceMetadata) {
    if m.is_empty() {
        return;
    }
    if let Some(p) = &m.source_path {
        push_text(out, indent, "_zetl:source_path", &p.to_string_lossy());
    }
    if let Some(o) = &m.object_id {
        push_text(out, indent, "_zetl:object_id", o);
    }
    if let Some(h) = &m.content_hash {
        push_text(out, indent, "_zetl:content_hash", h);
    }
    if let Some(p) = &m.prev_content_hash {
        push_text(out, indent, "_zetl:prev_content_hash", p);
    }
    if let Some(s) = m.changelog_seq {
        push_text(out, indent, "_zetl:changelog_seq", &s.to_string());
    }
}

fn push_text(out: &mut String, indent: &str, name: &str, value: &str) {
    out.push_str(indent);
    out.push('<');
    out.push_str(name);
    out.push('>');
    out.push_str(&escape_text(value));
    out.push_str("</");
    out.push_str(name);
    out.push_str(">\n");
}

use crate::feed::xml::{absolute_url, escape_attr, escape_text};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::types::{FeedPaths, OutputFormatSet};

    fn cfg() -> FeedConfig {
        FeedConfig {
            base_url: "https://example.com".to_string(),
            title: "Example".to_string(),
            description: "desc".to_string(),
            max_items: 50,
            formats: OutputFormatSet::default(),
            paths: FeedPaths {
                rss: "/feed.xml".to_string(),
                atom: "/atom.xml".to_string(),
                jsonfeed: "/feed.json".to_string(),
            },
            scope_id: None,
            cohort_id: None,
            default_author: None,
            language: None,
            copyright: None,
        }
    }

    fn item(id: &str) -> FeedItem {
        FeedItem {
            id: format!("tag:example.com,2026:zetl/{id}"),
            title: id.to_string(),
            url: format!("https://example.com/{id}"),
            date_published: "2026-05-08T00:00:00Z".to_string(),
            date_modified: None,
            summary: Some("hi".to_string()),
            content_html: Some("<p>hello</p>".to_string()),
            author: None,
            tags: vec!["meta".to_string()],
            license: None,
            source_metadata: SourceMetadata {
                object_id: Some(id.to_string()),
                content_hash: Some("deadbeef".to_string()),
                ..Default::default()
            },
        }
    }

    #[test]
    fn feed_envelope_present() {
        let s = serialise_atom(&[], &cfg());
        assert!(s.starts_with("<?xml"));
        assert!(s.contains(r#"<feed xmlns="http://www.w3.org/2005/Atom""#));
        assert!(s.contains("</feed>"));
        assert!(s.contains(r#"rel="self""#));
        assert!(s.contains(r#"rel="alternate""#));
    }

    #[test]
    fn entry_emits_required_elements_and_zetl_namespace() {
        let s = serialise_atom(&[item("foo")], &cfg());
        assert!(s.contains("<entry>"));
        assert!(s.contains("<id>tag:example.com,2026:zetl/foo</id>"));
        assert!(s.contains("<published>2026-05-08T00:00:00Z</published>"));
        assert!(s.contains("<updated>2026-05-08T00:00:00Z</updated>"));
        assert!(s.contains("<_zetl:object_id>foo</_zetl:object_id>"));
        assert!(s.contains("<_zetl:content_hash>deadbeef</_zetl:content_hash>"));
        assert!(s.contains(r#"<category term="meta" />"#));
    }

    #[test]
    fn deterministic_byte_identical() {
        let items = vec![item("a"), item("b")];
        assert_eq!(serialise_atom(&items, &cfg()), serialise_atom(&items, &cfg()));
    }

    #[test]
    fn content_uses_entity_escaping_not_cdata() {
        // REQ-3806: Atom content type=html uses character entities;
        // RSS uses CDATA. Confirm no CDATA wrapper in atom output.
        let s = serialise_atom(&[item("foo")], &cfg());
        assert!(!s.contains("<![CDATA["), "atom must not use CDATA, got {s}");
        // The <p>hello</p> body should appear entity-escaped.
        assert!(s.contains("&lt;p&gt;hello&lt;/p&gt;"), "got {s}");
    }

    #[test]
    fn empty_feed_has_no_updated_when_no_items() {
        let s = serialise_atom(&[], &cfg());
        // No <updated> at the feed level when there are no items.
        let updated_count = s.matches("<updated>").count();
        assert_eq!(updated_count, 0);
    }
}
