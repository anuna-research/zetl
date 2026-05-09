//! Pure outbound external-link extraction.
//!
//! Given the rendered HTML for a vault page plus the vault's base URL,
//! return every external link tuple `(source_page_url, target_url)`. The
//! sender feeds these into [`super::diff`] to compute the build's
//! outbound-POST diff against the persisted idempotency log.

use url::Url;

/// Extract external links. "External" = absolute URL whose origin
/// differs from `vault_base_url`. Relative paths are skipped; mailto:,
/// tel:, javascript:, data:, and same-origin URLs are skipped.
///
/// Result is deterministic and de-duplicated by target URL — two
/// occurrences of the same external target on the same page produce a
/// single tuple.
pub fn extract_external_links(
    rendered_html: &str,
    vault_base_url: &Url,
    source_page_url: &Url,
) -> Vec<(Url, Url)> {
    let mut out: Vec<(Url, Url)> = Vec::new();
    for raw in iter_link_attribute_values(rendered_html) {
        let Some(parsed) = parse_absolute_or_resolve(raw, source_page_url) else {
            continue;
        };
        if !matches!(parsed.scheme(), "http" | "https") {
            continue;
        }
        if same_origin(&parsed, vault_base_url) {
            continue;
        }
        if !out.iter().any(|(_, t)| t == &parsed) {
            out.push((source_page_url.clone(), parsed));
        }
    }
    out
}

fn parse_absolute_or_resolve(raw: &str, base: &Url) -> Option<Url> {
    if let Ok(u) = Url::parse(raw) {
        return Some(u);
    }
    if raw.starts_with('/') || raw.starts_with('.') {
        // Relative path — resolves against base, which means same-origin.
        // Caller filters same-origin so we just join and return.
        return base.join(raw).ok();
    }
    None
}

fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.port_or_known_default() == b.port_or_known_default()
}

/// Walk the byte stream and yield every `href` / `src` attribute value
/// that appears on a link-carrying tag (`a`, `link`, `area`, `img`,
/// `video`, `audio`, `source`, `iframe`).
///
/// We keep this dependency-free and tolerant of malformed input — the
/// goal is "find as many candidate links as possible" rather than
/// "spec-correct HTML5 parser." The downstream URL parser is the real
/// validator.
fn iter_link_attribute_values(html: &str) -> Vec<&str> {
    let bytes = html.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            // Skip <!-- ... --> comments and <script>/<style> blocks.
            if bytes[i..].starts_with(b"<!--") {
                if let Some(end) = find(&bytes[i + 4..], b"-->") {
                    i += 4 + end + 3;
                    continue;
                }
                break;
            }
            let lower_window = bytes[i..(i + 8).min(bytes.len())].to_ascii_lowercase();
            if lower_window.starts_with(b"<script") {
                if let Some(end) = find_ascii_ci(&bytes[i..], b"</script>") {
                    i += end + b"</script>".len();
                    continue;
                }
                break;
            }
            if lower_window.starts_with(b"<style") {
                if let Some(end) = find_ascii_ci(&bytes[i..], b"</style>") {
                    i += end + b"</style>".len();
                    continue;
                }
                break;
            }
            // Tag body up to the next '>'.
            let close = find(&bytes[i..], b">").map(|p| i + p);
            let Some(close) = close else { break };
            let tag_body = &html[i + 1..close];
            if let Some(name) = tag_name(tag_body) {
                if matches!(
                    name.as_str(),
                    "a" | "link" | "area" | "img" | "video" | "audio" | "source" | "iframe"
                ) {
                    for v in iter_attribute_values(tag_body) {
                        out.push(v);
                    }
                }
            }
            i = close + 1;
            continue;
        }
        i += 1;
    }
    out
}

fn tag_name(body: &str) -> Option<String> {
    let trimmed = body.trim_start_matches('/');
    let end = trimmed
        .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
        .unwrap_or(trimmed.len());
    if end == 0 {
        return None;
    }
    Some(trimmed[..end].to_ascii_lowercase())
}

fn iter_attribute_values(tag_body: &str) -> Vec<&str> {
    let bytes = tag_body.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let at_boundary = i == 0 || matches!(bytes[i - 1], b' ' | b'\t' | b'\n' | b'\r' | b'/');
        if !at_boundary {
            i += 1;
            continue;
        }
        let attr_names: &[&[u8]] = &[b"href", b"src"];
        let mut name_len = 0;
        for n in attr_names {
            if bytes[i..].len() >= n.len() && bytes[i..i + n.len()].eq_ignore_ascii_case(n) {
                let after = bytes.get(i + n.len()).copied().unwrap_or(b' ');
                if matches!(after, b'=' | b' ' | b'\t' | b'\n' | b'\r') {
                    name_len = n.len();
                    break;
                }
            }
        }
        if name_len == 0 {
            i += 1;
            continue;
        }
        let mut j = i + name_len;
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
            break;
        }
        let quote = bytes[j];
        let (vstart, vend);
        if quote == b'"' || quote == b'\'' {
            vstart = j + 1;
            vend = find(&bytes[vstart..], &[quote])
                .map(|p| vstart + p)
                .unwrap_or(bytes.len());
            i = vend.saturating_add(1);
        } else {
            vstart = j;
            vend = bytes[vstart..]
                .iter()
                .position(|&b| matches!(b, b' ' | b'>' | b'\t' | b'\n' | b'\r'))
                .map(|p| vstart + p)
                .unwrap_or(bytes.len());
            i = vend;
        }
        if vstart < vend {
            out.push(&tag_body[vstart..vend]);
        }
    }
    out
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn find_ascii_ci(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|w| w.eq_ignore_ascii_case(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn extracts_external_links_only() {
        let html = r#"
            <p>see <a href="https://other.example/post">other</a>
               and <a href="https://my.example/internal">self</a>
               and <a href="/relative">rel</a>
               and <a href="mailto:hi@x.example">mail</a>
            </p>
        "#;
        let result = extract_external_links(
            html,
            &url("https://my.example/"),
            &url("https://my.example/page"),
        );
        let targets: Vec<&str> = result.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(targets, vec!["https://other.example/post"]);
    }

    #[test]
    fn deduplicates_identical_external_targets() {
        let html = r#"
            <a href="https://other.example/post">a</a>
            <a href="https://other.example/post">b</a>
        "#;
        let result = extract_external_links(
            html,
            &url("https://my.example/"),
            &url("https://my.example/page"),
        );
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn skips_javascript_and_tel_schemes() {
        let html = r#"
            <a href="javascript:alert(1)">x</a>
            <a href="tel:+15551234">y</a>
            <a href="data:text/plain,hi">z</a>
        "#;
        let result = extract_external_links(
            html,
            &url("https://my.example/"),
            &url("https://my.example/page"),
        );
        assert!(result.is_empty());
    }

    #[test]
    fn skips_links_inside_script_blocks() {
        let html = r#"
            <p>see <a href="https://other.example/post">a</a></p>
            <script>var u = "<a href='https://other.example/in-script'>x</a>";</script>
        "#;
        let result = extract_external_links(
            html,
            &url("https://my.example/"),
            &url("https://my.example/page"),
        );
        let targets: Vec<&str> = result.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(targets, vec!["https://other.example/post"]);
    }

    #[test]
    fn order_is_document_order() {
        let html = r#"
            <a href="https://b.example/p">b</a>
            <a href="https://a.example/p">a</a>
            <a href="https://c.example/p">c</a>
        "#;
        let result = extract_external_links(
            html,
            &url("https://my.example/"),
            &url("https://my.example/page"),
        );
        let targets: Vec<String> = result.iter().map(|(_, t)| t.to_string()).collect();
        assert_eq!(
            targets,
            vec![
                "https://b.example/p".to_string(),
                "https://a.example/p".to_string(),
                "https://c.example/p".to_string(),
            ]
        );
    }

    #[test]
    fn images_and_media_are_extracted() {
        let html = r#"
            <img src="https://cdn.example/pic.jpg">
            <video src="https://cdn.example/vid.mp4"></video>
        "#;
        let result = extract_external_links(
            html,
            &url("https://my.example/"),
            &url("https://my.example/page"),
        );
        assert_eq!(result.len(), 2);
    }
}
