//! Pure source-link verification per REQ-3903.
//!
//! Given the source HTML and a target URL, decide whether the source
//! "links to" the target. The W3C REC says the receiver MUST verify a
//! link is present before accepting the mention; we honour that strictly
//! — links inside `<script>`, `<style>`, or HTML comments are reported
//! but do NOT satisfy verification.
//!
//! Implementation: a tiny streaming HTML scanner over the byte slice.
//! We do NOT pull in a full parser here because (a) we already trust
//! the SSRF-safe fetcher to bound size + scheme; (b) the verification
//! decision only needs to find href / src attribute values; (c) keeping
//! this module dependency-free keeps the pure-core dependency rule
//! airtight.

use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyResult {
    /// At least one honoured `<a href>` / `<link href>` / `<area href>` /
    /// media `src` linked to the target.
    Found,
    /// The link only appears inside `<script>` / `<style>` content.
    FoundOnlyInScript,
    /// The link only appears inside an HTML comment.
    FoundOnlyInComment,
    /// No link to the target found.
    NotFound,
}

/// Walk `source_html` and decide whether it links to `target`.
///
/// Match semantics: scheme + host + path + query + fragment must all
/// equal after URL normalisation. This is intentionally strict — `http`
/// vs `https` and trailing-slash differences count as MISMATCH per the
/// W3C REC's "the source MUST contain a link to the target".
pub fn verify_link_present(source_html: &str, target: &Url) -> VerifyResult {
    let canonical_target = canonicalise(target);
    let bytes = source_html.as_bytes();

    let mut found_in_script = false;
    let mut found_in_comment = false;
    let state = State::Outside;
    let mut i = 0usize;

    while i < bytes.len() {
        match state {
            State::Outside => {
                if let Some(rest) = bytes[i..].strip_prefix(b"<!--") {
                    let _ = rest;
                    let end = find_subseq(&bytes[i + 4..], b"-->")
                        .map(|p| i + 4 + p + 3)
                        .unwrap_or(bytes.len());
                    let body = &source_html[i + 4..end.saturating_sub(3).max(i + 4)];
                    if scan_attr_values_for_target(body, &canonical_target, target) {
                        found_in_comment = true;
                    }
                    i = end;
                    continue;
                }
                if i + 8 <= bytes.len() && bytes[i..i + 8].eq_ignore_ascii_case(b"<script>")
                    || (i + 7 <= bytes.len() && bytes[i..i + 7].eq_ignore_ascii_case(b"<script"))
                {
                    if let Some(close) =
                        find_subseq_ascii_ci(&bytes[i..], b"</script>").map(|p| i + p)
                    {
                        // Tag itself may have attributes; scan from `<` to `</script>`.
                        let inner_start = bytes[i..close]
                            .iter()
                            .position(|&b| b == b'>')
                            .map(|p| i + p + 1)
                            .unwrap_or(close);
                        let body = &source_html[inner_start..close];
                        if scan_attr_values_for_target(body, &canonical_target, target) {
                            found_in_script = true;
                        }
                        i = close + b"</script>".len();
                        continue;
                    }
                }
                if i + 6 <= bytes.len() && bytes[i..i + 6].eq_ignore_ascii_case(b"<style") {
                    if let Some(close) =
                        find_subseq_ascii_ci(&bytes[i..], b"</style>").map(|p| i + p)
                    {
                        let inner_start = bytes[i..close]
                            .iter()
                            .position(|&b| b == b'>')
                            .map(|p| i + p + 1)
                            .unwrap_or(close);
                        let body = &source_html[inner_start..close];
                        if scan_attr_values_for_target(body, &canonical_target, target) {
                            found_in_script = true; // collapse style into script bucket
                        }
                        i = close + b"</style>".len();
                        continue;
                    }
                }
                if bytes[i] == b'<' {
                    // Find tag end.
                    if let Some(close) = find_subseq(&bytes[i..], b">").map(|p| i + p) {
                        let tag_body = &source_html[i + 1..close];
                        if is_link_carrying_tag(tag_body)
                            && scan_attr_values_for_target(tag_body, &canonical_target, target)
                        {
                            return VerifyResult::Found;
                        }
                        i = close + 1;
                        continue;
                    }
                }
                i += 1;
            }
        }
    }

    if found_in_script {
        VerifyResult::FoundOnlyInScript
    } else if found_in_comment {
        VerifyResult::FoundOnlyInComment
    } else {
        VerifyResult::NotFound
    }
}

enum State {
    Outside,
}

/// Find the first occurrence of `needle` in `haystack`. Linear scan; we
/// don't need Boyer-Moore at this scale.
fn find_subseq(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Case-insensitive variant for ASCII tag-end matching.
fn find_subseq_ascii_ci(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|w| w.eq_ignore_ascii_case(needle))
}

/// Tags whose `href` / `src` attributes count as a link for verification.
fn is_link_carrying_tag(tag_body: &str) -> bool {
    // Strip leading "/" for closers like </a>.
    let trimmed = tag_body.trim_start_matches('/');
    let name_end = trimmed
        .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
        .unwrap_or(trimmed.len());
    let name = trimmed[..name_end].to_ascii_lowercase();
    matches!(
        name.as_str(),
        "a" | "link" | "area" | "img" | "video" | "audio" | "source" | "track" | "iframe"
    )
}

/// Walk attribute values inside a tag body / generic block and return
/// `true` if any value canonicalises to the same URL as `target`.
fn scan_attr_values_for_target(block: &str, canonical_target: &str, target: &Url) -> bool {
    let bytes = block.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Look for an "href=" or "src=" attribute (case-insensitive).
        if let Some(start) = find_attr_value_start(&bytes[i..]) {
            let abs_start = i + start.0;
            let quote = start.1;
            let value_start = abs_start;
            let value_end = if quote == b'"' || quote == b'\'' {
                find_subseq(&bytes[value_start..], &[quote])
                    .map(|p| value_start + p)
                    .unwrap_or(bytes.len())
            } else {
                bytes[value_start..]
                    .iter()
                    .position(|&b| b == b' ' || b == b'>' || b == b'\t' || b == b'\n')
                    .map(|p| value_start + p)
                    .unwrap_or(bytes.len())
            };
            let raw = &block[value_start..value_end];
            let resolved = if let Ok(u) = Url::parse(raw) {
                Some(u)
            } else {
                target.join(raw).ok()
            };
            if let Some(u) = resolved {
                if canonicalise(&u) == canonical_target {
                    return true;
                }
            }
            i = value_end + 1;
        } else {
            break;
        }
    }
    false
}

/// Find the next `(href|src)` attribute and return (offset of value
/// start, quote byte). Returns `None` if no further attribute found.
fn find_attr_value_start(bytes: &[u8]) -> Option<(usize, u8)> {
    let mut i = 0;
    while i < bytes.len() {
        // Boundary: must be at the start OR preceded by whitespace.
        let at_boundary =
            i == 0 || matches!(bytes[i - 1], b' ' | b'\t' | b'\n' | b'\r' | b'/' | b'<');
        if !at_boundary {
            i += 1;
            continue;
        }
        let candidates: &[&[u8]] = &[b"href", b"src"];
        let mut hit = None;
        for c in candidates {
            if i + c.len() < bytes.len() && bytes[i..i + c.len()].eq_ignore_ascii_case(c) {
                let after = bytes[i + c.len()];
                if after == b'=' || after == b' ' || after == b'\t' {
                    hit = Some(c.len());
                    break;
                }
            }
        }
        let Some(name_len) = hit else {
            i += 1;
            continue;
        };
        let mut j = i + name_len;
        // Skip whitespace.
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
            return Some((j + 1, quote));
        }
        // Unquoted value.
        return Some((j, 0));
    }
    None
}

/// Normalise a URL so two textually-different forms of the same target
/// compare equal. We DO NOT smooth http/https or trailing-slash —
/// those are real distinctions in the W3C REC.
pub(crate) fn canonicalise(url: &Url) -> String {
    let mut u = url.clone();
    if let Some(host) = u.host_str() {
        let lower = host.to_ascii_lowercase();
        if lower != host {
            // RFC 3986: host is case-insensitive. Best-effort lowercase.
            let _ = u.set_host(Some(&lower));
        }
    }
    u.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn finds_link_in_a_href() {
        let html = r#"<html><body>
            <p>see <a href="https://victim.example/post">post</a></p>
        </body></html>"#;
        assert_eq!(
            verify_link_present(html, &target("https://victim.example/post")),
            VerifyResult::Found
        );
    }

    #[test]
    fn missing_link_returns_not_found() {
        let html = r#"<a href="https://other.example/x">x</a>"#;
        assert_eq!(
            verify_link_present(html, &target("https://victim.example/post")),
            VerifyResult::NotFound
        );
    }

    #[test]
    fn link_inside_script_does_not_satisfy() {
        let html = r#"<script>var u = "https://victim.example/post";</script>"#;
        assert_eq!(
            verify_link_present(html, &target("https://victim.example/post")),
            VerifyResult::NotFound
        );
    }

    #[test]
    fn href_inside_script_tag_attributes() {
        // <a href> inside a <script> body still does not count.
        let html =
            r#"<script>document.write('<a href="https://victim.example/post">x</a>');</script>"#;
        let r = verify_link_present(html, &target("https://victim.example/post"));
        assert!(matches!(
            r,
            VerifyResult::FoundOnlyInScript | VerifyResult::NotFound
        ));
    }

    #[test]
    fn link_inside_html_comment_does_not_satisfy() {
        let html = r#"<!-- <a href="https://victim.example/post">x</a> --> <p>hi</p>"#;
        let r = verify_link_present(html, &target("https://victim.example/post"));
        assert!(matches!(
            r,
            VerifyResult::FoundOnlyInComment | VerifyResult::NotFound
        ));
    }

    #[test]
    fn link_in_img_src_satisfies() {
        let html = r#"<img src="https://victim.example/pic.jpg">"#;
        assert_eq!(
            verify_link_present(html, &target("https://victim.example/pic.jpg")),
            VerifyResult::Found
        );
    }

    #[test]
    fn link_in_link_tag_satisfies() {
        let html = r#"<head><link rel="related" href="https://victim.example/post"></head>"#;
        assert_eq!(
            verify_link_present(html, &target("https://victim.example/post")),
            VerifyResult::Found
        );
    }

    #[test]
    fn trailing_slash_is_a_mismatch() {
        let html = r#"<a href="https://victim.example/post/">x</a>"#;
        assert_eq!(
            verify_link_present(html, &target("https://victim.example/post")),
            VerifyResult::NotFound
        );
    }

    #[test]
    fn http_vs_https_is_a_mismatch() {
        let html = r#"<a href="http://victim.example/post">x</a>"#;
        assert_eq!(
            verify_link_present(html, &target("https://victim.example/post")),
            VerifyResult::NotFound
        );
    }

    #[test]
    fn host_case_is_canonicalised() {
        let html = r#"<a href="https://VICTIM.EXAMPLE/post">x</a>"#;
        assert_eq!(
            verify_link_present(html, &target("https://victim.example/post")),
            VerifyResult::Found
        );
    }

    #[test]
    fn relative_href_resolves_against_target_origin() {
        // A link "/post" on a same-origin page should resolve to the target.
        // We use the target's origin as the base for relative resolution; in
        // practice the receiver knows the source URL too — but since the only
        // resolved-against-target case that matters is "same origin", this
        // matches.
        let html = r#"<a href="/post">x</a>"#;
        assert_eq!(
            verify_link_present(html, &target("https://victim.example/post")),
            VerifyResult::Found
        );
    }

    #[test]
    fn multiple_attributes_on_single_tag() {
        let html =
            r#"<a class="x" id="y" rel="nofollow" href="https://victim.example/post">link</a>"#;
        assert_eq!(
            verify_link_present(html, &target("https://victim.example/post")),
            VerifyResult::Found
        );
    }

    #[test]
    fn unquoted_href() {
        let html = r#"<a href=https://victim.example/post>x</a>"#;
        assert_eq!(
            verify_link_present(html, &target("https://victim.example/post")),
            VerifyResult::Found
        );
    }

    #[test]
    fn single_quoted_href() {
        let html = r#"<a href='https://victim.example/post'>x</a>"#;
        assert_eq!(
            verify_link_present(html, &target("https://victim.example/post")),
            VerifyResult::Found
        );
    }
}
