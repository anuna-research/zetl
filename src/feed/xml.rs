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
}
