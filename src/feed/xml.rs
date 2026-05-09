//! XML/JSON helpers shared across the feed serialisers.
//!
//! Hoisted out of `serialise_rss.rs` / `serialise_atom.rs` /
//! `serialise_jsonfeed.rs` / `rewrite_links.rs`, where they were
//! previously byte-identical copies. Centralised so a security-
//! adjacent change (escape rules, CDATA boundaries, content-hash
//! encoding) can't drift across the three formats.

/// Escape `&`, `<`, `>` for XML text content. RSS 2.0 + Atom 1.0
/// element bodies (other than CDATA-wrapped content) consume the
/// 3-char form; attributes use [`escape_attr`] (which adds `"`).
pub fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Escape `&`, `<`, `>`, `"` for XML attribute values. Same as
/// [`escape_text`] plus the double-quote that delimits attribute
/// strings.
pub fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// CDATA-safe content per REQ-3806. The only sequence forbidden
/// inside a CDATA section is `]]>`; we split it across two CDATA
/// sections so the content survives strict parsers.
pub fn cdata_escape(s: &str) -> String {
    s.replace("]]>", "]]]]><![CDATA[>")
}

/// Join an absolute base URL with a path (`base + "/" + path`),
/// trimming a trailing `/` from `base` so the result has exactly
/// one separator regardless of input shape.
pub fn absolute_url(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    if path.starts_with('/') {
        format!("{base}{path}")
    } else {
        format!("{base}/{path}")
    }
}

/// XML well-formedness check via `quick-xml`'s streaming reader.
/// Walks every event once, verifies that the document parses cleanly
/// (balanced tags, terminated comments / CDATA / processing
/// instructions, valid attributes) and returns the root element name.
///
/// The XXE / DOCTYPE surface is gated upstream by
/// [`crate::feed::fetch::assert_no_xxe`]; this check covers
/// well-formedness only.
pub fn assert_xml_well_formed(s: &str) -> Result<String, String> {
    use quick_xml::events::Event;
    use quick_xml::reader::Reader;

    let mut reader = Reader::from_str(s);
    let config = reader.config_mut();
    config.check_end_names = true;
    config.trim_text(false);
    let mut buf = Vec::new();
    let mut root: Option<String> = None;
    let mut depth: usize = 0;
    loop {
        match reader.read_event_into(&mut buf) {
            Err(e) => {
                return Err(format!("{} at byte {}", e, reader.buffer_position()));
            }
            Ok(Event::Start(e)) => {
                let name = std::str::from_utf8(e.name().as_ref())
                    .map_err(|err| format!("non-UTF8 tag name: {err}"))?
                    .to_string();
                if depth == 0 {
                    if root.is_some() {
                        return Err(format!("second root element <{name}>"));
                    }
                    root = Some(name);
                }
                depth += 1;
            }
            Ok(Event::Empty(e)) => {
                let name = std::str::from_utf8(e.name().as_ref())
                    .map_err(|err| format!("non-UTF8 tag name: {err}"))?
                    .to_string();
                if depth == 0 {
                    if root.is_some() {
                        return Err(format!("second root element <{name}>"));
                    }
                    root = Some(name);
                }
            }
            Ok(Event::End(_)) => {
                depth = depth.saturating_sub(1);
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
        }
        buf.clear();
    }
    root.ok_or_else(|| "no element found".to_string())
}

/// Lowercase-hex encode a byte slice. Used for content-hash
/// stringification (BLAKE3 root hashes, ETag values). Allocates
/// once with the exact capacity needed.
pub fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        write!(out, "{b:02x}").expect("write to String never fails");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_text_three_chars() {
        assert_eq!(escape_text("a < b & c > d"), "a &lt; b &amp; c &gt; d");
        assert_eq!(escape_text("plain"), "plain");
    }

    #[test]
    fn escape_attr_four_chars() {
        assert_eq!(escape_attr(r#"<"&>"#), "&lt;&quot;&amp;&gt;");
    }

    #[test]
    fn cdata_handles_embedded_close() {
        assert_eq!(cdata_escape("foo]]>bar"), "foo]]]]><![CDATA[>bar");
        assert_eq!(cdata_escape("plain"), "plain");
    }

    #[test]
    fn absolute_url_one_separator() {
        assert_eq!(
            absolute_url("https://x.com", "/feed.xml"),
            "https://x.com/feed.xml"
        );
        assert_eq!(
            absolute_url("https://x.com/", "/feed.xml"),
            "https://x.com/feed.xml"
        );
        assert_eq!(
            absolute_url("https://x.com", "feed.xml"),
            "https://x.com/feed.xml"
        );
        assert_eq!(
            absolute_url("https://x.com/", "feed.xml"),
            "https://x.com/feed.xml"
        );
    }

    #[test]
    fn hex_encode_lowercase() {
        assert_eq!(hex_encode(&[0xde, 0xad, 0xbe, 0xef]), "deadbeef");
        assert_eq!(hex_encode(&[]), "");
        assert_eq!(hex_encode(&[0x00, 0x0f, 0xff]), "000fff");
    }

    #[test]
    fn well_formed_accepts_minimal_rss() {
        let body = r#"<?xml version="1.0"?>
            <rss version="2.0"><channel><title>x</title></channel></rss>"#;
        assert_eq!(assert_xml_well_formed(body).unwrap(), "rss");
    }

    #[test]
    fn well_formed_accepts_atom_with_self_closing() {
        let body = r#"<feed xmlns="http://www.w3.org/2005/Atom">
            <title>x</title>
            <link rel="self" href="https://x.example/atom.xml" />
        </feed>"#;
        assert_eq!(assert_xml_well_formed(body).unwrap(), "feed");
    }

    #[test]
    fn well_formed_handles_cdata_and_comments() {
        let body = "<r><!-- c --><x><![CDATA[<not> a tag]]></x></r>";
        assert_eq!(assert_xml_well_formed(body).unwrap(), "r");
    }

    #[test]
    fn well_formed_rejects_arbitrary_text() {
        assert!(assert_xml_well_formed("not an xml document").is_err());
    }

    #[test]
    fn well_formed_rejects_unclosed_tag() {
        assert!(assert_xml_well_formed("<rss><channel></rss>").is_err());
    }

    #[test]
    fn well_formed_rejects_mismatched_tags() {
        assert!(assert_xml_well_formed("<a></b>").is_err());
    }

    #[test]
    fn well_formed_rejects_unterminated_comment() {
        assert!(assert_xml_well_formed("<r><!-- never closed</r>").is_err());
    }

    #[test]
    fn well_formed_rejects_two_root_elements() {
        assert!(assert_xml_well_formed("<a/><b/>").is_err());
    }

    #[test]
    fn well_formed_attribute_with_gt_quoted() {
        let body = r#"<r><x a="1>2"/></r>"#;
        assert_eq!(assert_xml_well_formed(body).unwrap(), "r");
    }
}
