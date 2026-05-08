//! Pure JSON Feed v1.1 serialiser per ADR-3801 + CON-3815 + NFR-3805.
//!
//! Output keys are written in a fixed canonical order (top-level and
//! per-item) so byte-identical inputs produce byte-identical output —
//! `serde_json` alone with default `BTreeMap` ordering would also be
//! deterministic but operator-readability suffers; we use an explicit
//! ordered serializer that matches the JSON Feed spec's example.

use crate::feed::types::{FeedConfig, FeedItem, SourceMetadata};

/// JSON Feed v1.1 version URL — fixed by the spec.
pub const JSONFEED_VERSION: &str = "https://jsonfeed.org/version/1.1";

/// Serialise a slice of [`FeedItem`] into a JSON Feed v1.1 document.
pub fn serialise_jsonfeed(items: &[FeedItem], config: &FeedConfig) -> String {
    let mut buf = String::with_capacity(8192);
    buf.push_str("{\n");
    buf.push_str(r#"  "version": ""#);
    push_json_str(&mut buf, JSONFEED_VERSION);
    buf.push_str("\",\n");
    buf.push_str(r#"  "title": ""#);
    push_json_str(&mut buf, &config.title);
    buf.push_str("\",\n");
    if !config.description.is_empty() {
        buf.push_str(r#"  "description": ""#);
        push_json_str(&mut buf, &config.description);
        buf.push_str("\",\n");
    }
    buf.push_str(r#"  "home_page_url": ""#);
    push_json_str(&mut buf, &config.base_url);
    buf.push_str("\",\n");
    buf.push_str(r#"  "feed_url": ""#);
    let self_url = absolute_url(&config.base_url, &config.paths.jsonfeed);
    push_json_str(&mut buf, &self_url);
    buf.push_str("\",\n");
    if let Some(a) = &config.default_author {
        buf.push_str(r#"  "authors": ["#);
        push_author_object(&mut buf, a);
        buf.push_str("],\n");
    }
    if let Some(lang) = &config.language {
        buf.push_str(r#"  "language": ""#);
        push_json_str(&mut buf, lang);
        buf.push_str("\",\n");
    }
    buf.push_str(r#"  "items": ["#);
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            buf.push(',');
        }
        buf.push_str("\n    ");
        push_item(&mut buf, item);
    }
    if !items.is_empty() {
        buf.push_str("\n  ");
    }
    buf.push_str("]\n");
    buf.push_str("}\n");
    buf
}

fn push_item(buf: &mut String, item: &FeedItem) {
    buf.push('{');
    let mut first = true;
    macro_rules! field {
        ($key:expr, $value:expr) => {{
            if !first {
                buf.push_str(", ");
            }
            buf.push_str("\"");
            buf.push_str($key);
            buf.push_str("\": \"");
            push_json_str(buf, $value);
            buf.push_str("\"");
            first = false;
        }};
    }
    field!("id", &item.id);
    field!("url", &item.url);
    field!("title", &item.title);
    // JSON Feed v1.1 §item: every item MUST include at least one of
    // `content_html` or `content_text`. Prefer the rendered HTML when
    // present, then fall back to the plain-text summary, and finally
    // to an empty `content_text` so the document stays conformant for
    // items that legitimately have no body (e.g. opted-in stubs).
    if let Some(html) = &item.content_html {
        if !first {
            buf.push_str(", ");
        }
        buf.push_str(r#""content_html": ""#);
        push_json_str(buf, html);
        buf.push('"');
        first = false;
    } else if let Some(s) = &item.summary {
        if !first {
            buf.push_str(", ");
        }
        buf.push_str(r#""content_text": ""#);
        push_json_str(buf, s);
        buf.push('"');
        first = false;
    } else {
        if !first {
            buf.push_str(", ");
        }
        buf.push_str(r#""content_text": """#);
        first = false;
    }
    if let Some(s) = &item.summary {
        field!("summary", s);
    }
    field!("date_published", &item.date_published);
    if let Some(m) = &item.date_modified {
        field!("date_modified", m);
    }
    if let Some(a) = &item.author {
        if !first {
            buf.push_str(", ");
        }
        buf.push_str(r#""authors": ["#);
        push_author_object(buf, a);
        buf.push(']');
        first = false;
    }
    if !item.tags.is_empty() {
        if !first {
            buf.push_str(", ");
        }
        buf.push_str(r#""tags": ["#);
        for (i, t) in item.tags.iter().enumerate() {
            if i > 0 {
                buf.push_str(", ");
            }
            buf.push('"');
            push_json_str(buf, t);
            buf.push('"');
        }
        buf.push(']');
        first = false;
    }
    push_zetl_extension(buf, &item.source_metadata, &mut first);
    buf.push('}');
}

fn push_author_object(buf: &mut String, a: &crate::feed::types::AuthorRef) {
    buf.push_str(r#"{"name": ""#);
    push_json_str(buf, &a.name);
    buf.push('"');
    if let Some(u) = &a.url {
        buf.push_str(r#", "url": ""#);
        push_json_str(buf, u);
        buf.push('"');
    }
    buf.push('}');
}

fn push_zetl_extension(buf: &mut String, m: &SourceMetadata, first: &mut bool) {
    if m.is_empty() {
        return;
    }
    if !*first {
        buf.push_str(", ");
    }
    buf.push_str(r#""_zetl": {"#);
    let mut zfirst = true;
    macro_rules! zfield {
        ($key:expr, $value:expr) => {{
            if !zfirst {
                buf.push_str(", ");
            }
            buf.push_str("\"");
            buf.push_str($key);
            buf.push_str("\": \"");
            push_json_str(buf, $value);
            buf.push_str("\"");
            zfirst = false;
        }};
    }
    if let Some(p) = &m.source_path {
        zfield!("source_path", &p.to_string_lossy());
    }
    if let Some(o) = &m.object_id {
        zfield!("object_id", o);
    }
    if let Some(h) = &m.content_hash {
        zfield!("content_hash", h);
    }
    if let Some(p) = &m.prev_content_hash {
        zfield!("prev_content_hash", p);
    }
    if let Some(s) = m.changelog_seq {
        if !zfirst {
            buf.push_str(", ");
        }
        buf.push_str(r#""changelog_seq": "#);
        buf.push_str(&s.to_string());
        // Numeric, no quoting.
        let _ = zfirst;
    }
    buf.push('}');
    *first = false;
}

fn push_json_str(buf: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '"' => buf.push_str("\\\""),
            '\\' => buf.push_str("\\\\"),
            '\n' => buf.push_str("\\n"),
            '\r' => buf.push_str("\\r"),
            '\t' => buf.push_str("\\t"),
            '\u{08}' => buf.push_str("\\b"),
            '\u{0c}' => buf.push_str("\\f"),
            c if (c as u32) < 0x20 => buf.push_str(&format!("\\u{:04x}", c as u32)),
            c => buf.push(c),
        }
    }
}

use crate::feed::xml::absolute_url;

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
                ..Default::default()
            },
        }
    }

    #[test]
    fn output_is_valid_json() {
        let s = serialise_jsonfeed(&[item("foo"), item("bar")], &cfg());
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["version"], JSONFEED_VERSION);
        assert_eq!(v["title"], "Example");
        assert_eq!(v["items"][0]["id"], "tag:example.com,2026:zetl/foo");
    }

    #[test]
    fn deterministic_byte_identical() {
        let items = vec![item("a"), item("b")];
        let a = serialise_jsonfeed(&items, &cfg());
        let b = serialise_jsonfeed(&items, &cfg());
        assert_eq!(a, b);
    }

    #[test]
    fn empty_feed_has_empty_items_array() {
        let s = serialise_jsonfeed(&[], &cfg());
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert!(v["items"].as_array().unwrap().is_empty());
    }

    #[test]
    fn special_chars_in_title_escaped() {
        let mut it = item("foo");
        it.title = "He said \"hi\"\nand left.".to_string();
        let s = serialise_jsonfeed(&[it], &cfg());
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["items"][0]["title"], "He said \"hi\"\nand left.");
    }

    #[test]
    fn zetl_extension_carries_source_metadata() {
        let s = serialise_jsonfeed(&[item("foo")], &cfg());
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["items"][0]["_zetl"]["object_id"], "foo");
    }
}
