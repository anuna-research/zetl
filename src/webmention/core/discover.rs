//! Pure endpoint discovery per REQ-3908.
//!
//! Algorithm (W3C REC §3.1.2):
//! 1. Scan response Link headers for `rel=webmention` (header preference);
//!    first match wins. Multiple Link headers and comma-separated values
//!    inside a single header are both supported.
//! 2. If no header match, scan the HTML body for
//!    `<link rel="webmention" href="...">` (preferred) or
//!    `<a rel="webmention" href="...">` (fallback). First match wins.
//! 3. Resolve relative URLs against the fetched URL.
//!
//! Pure: no I/O. Returns `Some(endpoint)` on success, `None` on absence.

use url::Url;

pub fn discover_endpoint(
    headers: &[(String, String)],
    html_body: Option<&str>,
    fetched_url: &Url,
) -> Option<Url> {
    if let Some(u) = discover_in_link_headers(headers, fetched_url) {
        return Some(u);
    }
    if let Some(html) = html_body {
        return discover_in_html(html, fetched_url);
    }
    None
}

fn discover_in_link_headers(headers: &[(String, String)], base: &Url) -> Option<Url> {
    for (name, value) in headers {
        if !name.eq_ignore_ascii_case("link") {
            continue;
        }
        for entry in split_link_header(value) {
            if has_rel_webmention(entry) {
                if let Some(href) = extract_link_target(entry) {
                    if let Some(u) = resolve(href, base) {
                        return Some(u);
                    }
                }
            }
        }
    }
    None
}

/// Split a Link header value into individual `<...>; rel=...` entries.
/// Splits on commas that sit OUTSIDE angle-bracketed URI-Refs and outside
/// quoted parameter values.
fn split_link_header(value: &str) -> Vec<&str> {
    let bytes = value.as_bytes();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut depth_angle = 0i32;
    let mut in_quote = false;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'<' if !in_quote => depth_angle += 1,
            b'>' if !in_quote => depth_angle -= 1,
            b'"' => in_quote = !in_quote,
            b',' if depth_angle <= 0 && !in_quote => {
                out.push(value[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start < bytes.len() {
        out.push(value[start..].trim());
    }
    out
}

fn has_rel_webmention(entry: &str) -> bool {
    // entry like: `<https://endpoint/wm>; rel="webmention author"`.
    let lower = entry.to_ascii_lowercase();
    if !lower.contains("rel=") {
        return false;
    }
    // Walk semicolon-separated params after the URI-Ref.
    let after_url = match entry.split_once('>') {
        Some((_, rest)) => rest,
        None => entry,
    };
    for part in after_url.split(';') {
        let part = part.trim();
        let Some(rest) = part
            .strip_prefix("rel=")
            .or_else(|| part.strip_prefix("Rel="))
        else {
            continue;
        };
        let rest = rest.trim_matches(|c| c == '"' || c == ' ');
        if rest
            .split_ascii_whitespace()
            .any(|tok| tok.eq_ignore_ascii_case("webmention"))
        {
            return true;
        }
    }
    false
}

fn extract_link_target(entry: &str) -> Option<&str> {
    let start = entry.find('<')?;
    let end = entry[start + 1..].find('>')?;
    Some(&entry[start + 1..start + 1 + end])
}

fn resolve(href: &str, base: &Url) -> Option<Url> {
    if let Ok(u) = Url::parse(href) {
        return Some(u);
    }
    base.join(href).ok()
}

fn discover_in_html(html: &str, base: &Url) -> Option<Url> {
    // Scan tags top-down for any rel-webmention occurrence.
    // First, search <link rel=...>, then fall back to <a rel=...>.
    if let Some(u) = scan_html_for_rel_webmention(html, base, true) {
        return Some(u);
    }
    scan_html_for_rel_webmention(html, base, false)
}

/// `prefer_link_tag = true`: only match `<link>`. `false`: match `<link>`
/// or `<a>` — used as the second pass.
fn scan_html_for_rel_webmention(html: &str, base: &Url, link_only: bool) -> Option<Url> {
    let bytes = html.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            if bytes[i..].starts_with(b"<!--") {
                let after = i + 4;
                let end = find(&bytes[after..], b"-->").map(|p| after + p + 3);
                i = end.unwrap_or(bytes.len());
                continue;
            }
            let close = find(&bytes[i..], b">").map(|p| i + p);
            let Some(close) = close else { break };
            let tag_body = &html[i + 1..close];
            let lower = tag_body.to_ascii_lowercase();
            let is_link = lower.starts_with("link") || lower.starts_with("link/");
            let is_anchor = lower.starts_with('a') && {
                // Avoid matching <article>, <abbr>, etc.
                lower
                    .as_bytes()
                    .get(1)
                    .map(|c| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | b'/'))
                    .unwrap_or(false)
            };
            let want = is_link || (!link_only && is_anchor);
            if want && tag_has_rel_webmention(tag_body) {
                if let Some(href) = extract_attr(tag_body, "href") {
                    if let Some(u) = resolve(href, base) {
                        return Some(u);
                    }
                }
            }
            i = close + 1;
            continue;
        }
        i += 1;
    }
    None
}

fn tag_has_rel_webmention(tag_body: &str) -> bool {
    let Some(rel) = extract_attr(tag_body, "rel") else {
        return false;
    };
    rel.split_ascii_whitespace()
        .any(|tok| tok.eq_ignore_ascii_case("webmention"))
}

fn extract_attr<'a>(tag_body: &'a str, name: &str) -> Option<&'a str> {
    let bytes = tag_body.as_bytes();
    let needle = name.as_bytes();
    let mut i = 0;
    while i + needle.len() < bytes.len() {
        let at_boundary = i == 0 || matches!(bytes[i - 1], b' ' | b'\t' | b'\n' | b'\r' | b'/');
        if at_boundary && bytes[i..i + needle.len()].eq_ignore_ascii_case(needle) {
            let after = bytes[i + needle.len()];
            if matches!(after, b'=' | b' ' | b'\t' | b'\n' | b'\r') {
                let mut j = i + needle.len();
                while j < bytes.len() && matches!(bytes[j], b' ' | b'\t' | b'\n' | b'\r') {
                    j += 1;
                }
                if j >= bytes.len() || bytes[j] != b'=' {
                    i = j;
                    continue;
                }
                j += 1;
                while j < bytes.len() && matches!(bytes[j], b' ' | b'\t' | b'\n' | b'\r') {
                    j += 1;
                }
                if j >= bytes.len() {
                    return None;
                }
                let quote = bytes[j];
                if quote == b'"' || quote == b'\'' {
                    let vstart = j + 1;
                    let vend = find(&bytes[vstart..], &[quote])
                        .map(|p| vstart + p)
                        .unwrap_or(bytes.len());
                    return Some(&tag_body[vstart..vend]);
                } else {
                    let vstart = j;
                    let vend = bytes[vstart..]
                        .iter()
                        .position(|&b| matches!(b, b' ' | b'>' | b'\t' | b'\n' | b'\r'))
                        .map(|p| vstart + p)
                        .unwrap_or(bytes.len());
                    return Some(&tag_body[vstart..vend]);
                }
            }
        }
        i += 1;
    }
    None
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("https://target.example/post").unwrap()
    }

    #[test]
    fn header_match_wins_over_html() {
        let headers = vec![(
            "Link".to_string(),
            "<https://target.example/wm>; rel=\"webmention\"".to_string(),
        )];
        let html = Some(r#"<link rel="webmention" href="https://other.example/wm">"#);
        let r = discover_endpoint(&headers, html, &base()).unwrap();
        assert_eq!(r.as_str(), "https://target.example/wm");
    }

    #[test]
    fn html_link_tag_used_when_no_header() {
        let html = Some(r#"<head><link rel="webmention" href="https://target.example/wm"></head>"#);
        let r = discover_endpoint(&[], html, &base()).unwrap();
        assert_eq!(r.as_str(), "https://target.example/wm");
    }

    #[test]
    fn anchor_fallback_when_no_link_tag() {
        let html = Some(r#"<a rel="webmention" href="https://target.example/wm">x</a>"#);
        let r = discover_endpoint(&[], html, &base()).unwrap();
        assert_eq!(r.as_str(), "https://target.example/wm");
    }

    #[test]
    fn relative_href_resolves_against_fetched_url() {
        let html = Some(r#"<link rel="webmention" href="/wm">"#);
        let r = discover_endpoint(&[], html, &base()).unwrap();
        assert_eq!(r.as_str(), "https://target.example/wm");
    }

    #[test]
    fn rel_webmention_inside_compound_rel_value() {
        let headers = vec![(
            "link".to_string(),
            "<https://target.example/wm>; rel=\"author webmention\"".to_string(),
        )];
        let r = discover_endpoint(&headers, None, &base()).unwrap();
        assert_eq!(r.as_str(), "https://target.example/wm");
    }

    #[test]
    fn missing_returns_none() {
        let headers = vec![("Content-Type".to_string(), "text/html".to_string())];
        let html = Some(r#"<p>no webmention here</p>"#);
        assert!(discover_endpoint(&headers, html, &base()).is_none());
    }

    #[test]
    fn first_link_header_wins_when_multiple() {
        let headers = vec![
            (
                "Link".to_string(),
                "<https://first.example/wm>; rel=\"webmention\"".to_string(),
            ),
            (
                "Link".to_string(),
                "<https://second.example/wm>; rel=\"webmention\"".to_string(),
            ),
        ];
        let r = discover_endpoint(&headers, None, &base()).unwrap();
        assert_eq!(r.as_str(), "https://first.example/wm");
    }

    #[test]
    fn comma_separated_link_header() {
        let headers = vec![(
            "Link".to_string(),
            "<https://target.example/feed>; rel=\"alternate\", <https://target.example/wm>; rel=\"webmention\""
                .to_string(),
        )];
        let r = discover_endpoint(&headers, None, &base()).unwrap();
        assert_eq!(r.as_str(), "https://target.example/wm");
    }
}
