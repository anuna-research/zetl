//! Pure RSS 2.0 serialiser per REQ-3801 + REQ-3806 + NFR-3805.
//!
//! Emits standards-conformant RSS 2.0 with channel-level metadata and
//! per-item entries. CDATA escaping per REQ-3806 for content bodies.
//! Output is deterministic — byte-identical for byte-identical inputs.

use crate::feed::types::{FeedConfig, FeedItem};

/// Serialise a slice of [`FeedItem`] into an RSS 2.0 XML document.
pub fn serialise_rss(items: &[FeedItem], config: &FeedConfig) -> String {
    let mut out = String::with_capacity(8192);
    out.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    out.push('\n');
    out.push_str(r#"<rss version="2.0" xmlns:atom="http://www.w3.org/2005/Atom" xmlns:dc="http://purl.org/dc/elements/1.1/">"#);
    out.push('\n');
    out.push_str("  <channel>\n");
    push_text_element(&mut out, "    ", "title", &config.title);
    push_text_element(&mut out, "    ", "link", &config.base_url);
    push_text_element(&mut out, "    ", "description", &config.description);
    if let Some(lang) = &config.language {
        push_text_element(&mut out, "    ", "language", lang);
    }
    if let Some(c) = &config.copyright {
        push_text_element(&mut out, "    ", "copyright", c);
    }
    // Self-link per outbound REQ-3811 equivalent.
    let self_url = absolute_url(&config.base_url, &config.paths.rss);
    out.push_str(&format!(
        r#"    <atom:link href="{}" rel="self" type="application/rss+xml" />"#,
        escape_attr(&self_url)
    ));
    out.push('\n');
    // lastBuildDate is the most-recent item's date (REQ-3804). Empty
    // feed produces no element.
    if let Some(latest) = items.iter().map(|i| &i.date_published).max() {
        push_text_element(&mut out, "    ", "lastBuildDate", &rfc3339_to_rfc822(latest));
    }
    for item in items {
        out.push_str("    <item>\n");
        push_text_element(&mut out, "      ", "title", &item.title);
        push_text_element(&mut out, "      ", "link", &item.url);
        out.push_str(&format!(
            r#"      <guid isPermaLink="false">{}</guid>"#,
            escape_text(&item.id)
        ));
        out.push('\n');
        push_text_element(
            &mut out,
            "      ",
            "pubDate",
            &rfc3339_to_rfc822(&item.date_published),
        );
        if let Some(author) = &item.author {
            // RSS <author> is `email (Name)` form when email present.
            let val = match (&author.email, author.name.as_str()) {
                (Some(e), n) if !n.is_empty() => format!("{e} ({n})"),
                (Some(e), _) => e.clone(),
                (None, n) => format!("{n} (zetl)"),
            };
            push_text_element(&mut out, "      ", "dc:creator", &val);
        }
        for tag in &item.tags {
            push_text_element(&mut out, "      ", "category", tag);
        }
        // Body: prefer content_html in CDATA, fall back to summary.
        let body = item
            .content_html
            .as_deref()
            .or(item.summary.as_deref())
            .unwrap_or("");
        if !body.is_empty() {
            out.push_str("      <description><![CDATA[");
            out.push_str(&cdata_escape(body));
            out.push_str("]]></description>\n");
        }
        out.push_str("    </item>\n");
    }
    out.push_str("  </channel>\n");
    out.push_str("</rss>\n");
    out
}

fn push_text_element(out: &mut String, indent: &str, name: &str, value: &str) {
    out.push_str(indent);
    out.push('<');
    out.push_str(name);
    out.push('>');
    out.push_str(&escape_text(value));
    out.push_str("</");
    out.push_str(name);
    out.push_str(">\n");
}

use crate::feed::xml::{absolute_url, cdata_escape, escape_attr, escape_text};

/// Convert an RFC 3339 timestamp to RFC 822 (RSS pubDate format).
/// Lexicographic comparison stays valid because we only ever compare
/// the original RFC 3339 strings; the RFC 822 conversion is for display.
pub(crate) fn rfc3339_to_rfc822(rfc3339: &str) -> String {
    // Tiny manual converter; matches the canonical form produced by
    // resolve_date.rs (`YYYY-MM-DDTHH:MM:SS[.fff](Z|±HH:MM)`).
    if rfc3339.len() < 19 {
        return rfc3339.to_string();
    }
    let date = &rfc3339[..10];
    let time = &rfc3339[11..19];
    let tail_start = 19usize;
    // Skip optional fractional seconds.
    let bytes = rfc3339.as_bytes();
    let mut i = tail_start;
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
    let tz = &rfc3339[i..];
    let tz_822 = match tz {
        "Z" | "z" => "GMT".to_string(),
        s if s.len() == 6 => {
            // ±HH:MM -> ±HHMM
            let sign = &s[..1];
            let hh = &s[1..3];
            let mm = &s[4..6];
            format!("{sign}{hh}{mm}")
        }
        other => other.to_string(),
    };
    let parts: Vec<&str> = date.split('-').collect();
    if parts.len() != 3 {
        return rfc3339.to_string();
    }
    let (y, m, d) = (parts[0], parts[1], parts[2]);
    let month_name = month_short(m).unwrap_or(m);
    let dow = zellers_day_of_week(y, m, d).unwrap_or("Sat");
    format!("{dow}, {d} {month_name} {y} {time} {tz_822}")
}

fn month_short(mm: &str) -> Option<&'static str> {
    match mm {
        "01" => Some("Jan"),
        "02" => Some("Feb"),
        "03" => Some("Mar"),
        "04" => Some("Apr"),
        "05" => Some("May"),
        "06" => Some("Jun"),
        "07" => Some("Jul"),
        "08" => Some("Aug"),
        "09" => Some("Sep"),
        "10" => Some("Oct"),
        "11" => Some("Nov"),
        "12" => Some("Dec"),
        _ => None,
    }
}

/// Zeller's congruence for day-of-week — keeps us off chrono.
fn zellers_day_of_week(y: &str, m: &str, d: &str) -> Option<&'static str> {
    let year: i32 = y.parse().ok()?;
    let mut month: i32 = m.parse().ok()?;
    let day: i32 = d.parse().ok()?;
    let mut yr = year;
    if month < 3 {
        month += 12;
        yr -= 1;
    }
    let k = yr % 100;
    let j = yr / 100;
    let h = (day + (13 * (month + 1)) / 5 + k + k / 4 + j / 4 + 5 * j) % 7;
    // Zeller's h: 0=Sat, 1=Sun, 2=Mon, ...
    Some(match h {
        0 => "Sat",
        1 => "Sun",
        2 => "Mon",
        3 => "Tue",
        4 => "Wed",
        5 => "Thu",
        _ => "Fri",
    })
}

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
            language: Some("en".to_string()),
            copyright: None,
        }
    }

    fn item(id: &str, date: &str) -> FeedItem {
        FeedItem {
            id: format!("tag:example.com,2026:zetl/{id}"),
            title: id.to_string(),
            url: format!("https://example.com/{id}"),
            date_published: date.to_string(),
            date_modified: None,
            summary: Some("hi".to_string()),
            content_html: Some("<p>hello</p>".to_string()),
            author: None,
            tags: Vec::new(),
            license: None,
            source_metadata: Default::default(),
        }
    }

    #[test]
    fn empty_feed_has_channel_envelope() {
        let s = serialise_rss(&[], &cfg());
        assert!(s.starts_with("<?xml"));
        assert!(s.contains("<rss"));
        assert!(s.contains("<channel>"));
        assert!(s.contains("</channel>"));
        assert!(s.contains("</rss>"));
        assert!(s.contains("<title>Example</title>"));
        assert!(s.contains("rel=\"self\""));
    }

    #[test]
    fn item_emits_required_fields() {
        let s = serialise_rss(&[item("foo", "2026-05-08T00:00:00Z")], &cfg());
        assert!(s.contains("<title>foo</title>"));
        assert!(s.contains("<link>https://example.com/foo</link>"));
        assert!(s.contains(r#"<guid isPermaLink="false">tag:example.com,2026:zetl/foo</guid>"#));
        assert!(s.contains("<pubDate>"));
        assert!(s.contains("<![CDATA[<p>hello</p>]]>"));
    }

    #[test]
    fn cdata_escape_handles_embedded_close_marker() {
        assert_eq!(cdata_escape("foo]]>bar"), "foo]]]]><![CDATA[>bar");
        assert_eq!(cdata_escape("plain"), "plain");
    }

    #[test]
    fn deterministic_byte_identical_for_same_input() {
        let items = vec![
            item("a", "2026-05-08T00:00:00Z"),
            item("b", "2026-05-09T00:00:00Z"),
        ];
        let a = serialise_rss(&items, &cfg());
        let b = serialise_rss(&items, &cfg());
        assert_eq!(a, b);
    }

    #[test]
    fn rfc3339_to_rfc822_smoke() {
        assert_eq!(
            rfc3339_to_rfc822("2026-05-08T12:00:00Z"),
            "Fri, 08 May 2026 12:00:00 GMT"
        );
        assert_eq!(
            rfc3339_to_rfc822("2026-01-01T00:00:00+10:00"),
            "Thu, 01 Jan 2026 00:00:00 +1000"
        );
    }

    #[test]
    fn xml_special_characters_escaped_in_text() {
        let mut it = item("foo", "2026-05-08T00:00:00Z");
        it.title = "<bad>&\"'".to_string();
        let s = serialise_rss(&[it], &cfg());
        assert!(s.contains("<title>&lt;bad&gt;&amp;\"'</title>"));
    }

    #[test]
    fn emits_lastbuilddate_from_max_item_date() {
        let items = vec![
            item("a", "2026-05-08T00:00:00Z"),
            item("b", "2026-05-09T00:00:00Z"),
        ];
        let s = serialise_rss(&items, &cfg());
        assert!(s.contains("<lastBuildDate>Sat, 09 May 2026"));
    }
}
