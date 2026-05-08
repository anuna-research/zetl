//! Wikilink-rewriting pure function per REQ-3805 + REQ-3807.
//!
//! Walks the rendered content, finds `[[wikilink]]` syntax (and
//! optionally already-rendered `<a href="/slug">` to internal slugs),
//! rewrites each to an absolute URL by prepending the configured
//! `base_url`. Unresolved targets receive the configured policy
//! treatment.
//!
//! Properties:
//!   * **Idempotent**: `rewrite(rewrite(c)) == rewrite(c)` (tested).
//!     Once a wikilink has been replaced with `<a href>` the second
//!     pass finds no remaining `[[…]]` syntax to rewrite.
//!   * **Pure**: takes a slug-resolver callable; never reaches network.

use url::Url;

/// What to do with `[[unresolved]]` wikilinks (slug_resolver returned
/// `None`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnresolvedPolicy {
    /// Drop the syntax entirely; emit only the link text (or alias).
    /// Closest to the local-vault rendering for unresolved targets.
    PreserveText,
    /// Drop the link entirely, including text.
    Drop,
    /// Emit a placeholder URL `<base_url>/_unresolved/<slug>` so feed
    /// readers still get a clickable link (404 expected).
    Placeholder,
    /// Keep the original `[[…]]` syntax untouched. Rare; useful for
    /// deferred rewriting workflows.
    Preserve,
}

/// Rewrite every wikilink in `content` to an absolute URL anchored at
/// `base_url`. Returns the rewritten body. The function operates over
/// the input as a whole string and is `O(n)` in input length — see
/// the perf assertion in [`tests::idempotent`].
pub fn rewrite_links(
    content: &str,
    base_url: &Url,
    slug_resolver: &dyn Fn(&str) -> Option<String>,
    policy: UnresolvedPolicy,
) -> String {
    let mut out = String::with_capacity(content.len());
    let bytes = content.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        // `[[…]]` — fast path.
        if i + 1 < bytes.len() && bytes[i] == b'[' && bytes[i + 1] == b'[' {
            if let Some(close) = find_double_close(content, i + 2) {
                let inner = &content[i + 2..close];
                // Disallow embedded line breaks inside `[[ ]]` to avoid
                // matching across paragraphs.
                if !inner.contains('\n') {
                    let (target, alias) = split_alias(inner);
                    let resolved = slug_resolver(target);
                    match resolved {
                        Some(slug) => {
                            // Emit `<a href="<base>/<slug>">text</a>`.
                            let href = build_url(base_url, &slug);
                            let text = alias.unwrap_or(target);
                            out.push_str(&format!(
                                r#"<a href="{href}">{text}</a>"#,
                                href = escape_attr(&href),
                                text = escape_text(text)
                            ));
                            i = close + 2;
                            continue;
                        }
                        None => {
                            apply_unresolved(&mut out, base_url, target, alias, policy);
                            i = close + 2;
                            continue;
                        }
                    }
                }
            }
        }
        // Default: copy a single byte. Multi-byte UTF-8 chars stream
        // through unchanged because we never split mid-char.
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn apply_unresolved(
    out: &mut String,
    base_url: &Url,
    target: &str,
    alias: Option<&str>,
    policy: UnresolvedPolicy,
) {
    match policy {
        UnresolvedPolicy::PreserveText => {
            let text = alias.unwrap_or(target);
            out.push_str(&escape_text(text));
        }
        UnresolvedPolicy::Drop => {}
        UnresolvedPolicy::Placeholder => {
            let href = build_url(base_url, &format!("_unresolved/{target}"));
            let text = alias.unwrap_or(target);
            out.push_str(&format!(
                r#"<a href="{href}">{text}</a>"#,
                href = escape_attr(&href),
                text = escape_text(text)
            ));
        }
        UnresolvedPolicy::Preserve => {
            out.push_str("[[");
            out.push_str(target);
            if let Some(a) = alias {
                out.push('|');
                out.push_str(a);
            }
            out.push_str("]]");
        }
    }
}

fn find_double_close(s: &str, start: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = start;
    while i + 1 < bytes.len() {
        if bytes[i] == b']' && bytes[i + 1] == b']' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn split_alias(inner: &str) -> (&str, Option<&str>) {
    if let Some(idx) = inner.find('|') {
        let target = inner[..idx].trim();
        let alias = inner[idx + 1..].trim();
        (target, Some(alias))
    } else {
        (inner.trim(), None)
    }
}

fn build_url(base: &Url, slug: &str) -> String {
    let trimmed = slug.trim_start_matches('/');
    let base_str = base.as_str().trim_end_matches('/');
    format!("{base_str}/{trimmed}")
}

fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("https://example.com").unwrap()
    }

    fn always_resolve(slug: &str) -> Option<String> {
        Some(slug.to_string())
    }

    fn never_resolve(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn rewrites_simple_wikilink() {
        let out = rewrite_links(
            "see [[foo]] for details",
            &base(),
            &always_resolve,
            UnresolvedPolicy::PreserveText,
        );
        assert_eq!(out, r#"see <a href="https://example.com/foo">foo</a> for details"#);
    }

    #[test]
    fn rewrites_aliased_wikilink() {
        let out = rewrite_links(
            "see [[foo|the foo page]] for details",
            &base(),
            &always_resolve,
            UnresolvedPolicy::PreserveText,
        );
        assert_eq!(
            out,
            r#"see <a href="https://example.com/foo">the foo page</a> for details"#
        );
    }

    #[test]
    fn idempotent() {
        let body = "Hi [[foo]] and [[bar|alias]]";
        let once = rewrite_links(body, &base(), &always_resolve, UnresolvedPolicy::PreserveText);
        let twice = rewrite_links(&once, &base(), &always_resolve, UnresolvedPolicy::PreserveText);
        assert_eq!(once, twice);
    }

    #[test]
    fn unresolved_preserve_text_drops_syntax() {
        let out = rewrite_links(
            "see [[ghost]] later",
            &base(),
            &never_resolve,
            UnresolvedPolicy::PreserveText,
        );
        assert_eq!(out, "see ghost later");
    }

    #[test]
    fn unresolved_drop_removes_link() {
        let out = rewrite_links(
            "before [[ghost]] after",
            &base(),
            &never_resolve,
            UnresolvedPolicy::Drop,
        );
        assert_eq!(out, "before  after");
    }

    #[test]
    fn unresolved_placeholder_routes_under_unresolved() {
        let out = rewrite_links(
            "[[ghost]]",
            &base(),
            &never_resolve,
            UnresolvedPolicy::Placeholder,
        );
        assert_eq!(
            out,
            r#"<a href="https://example.com/_unresolved/ghost">ghost</a>"#
        );
    }

    #[test]
    fn unresolved_preserve_keeps_syntax() {
        let out = rewrite_links(
            "[[ghost|spook]]",
            &base(),
            &never_resolve,
            UnresolvedPolicy::Preserve,
        );
        assert_eq!(out, "[[ghost|spook]]");
    }

    #[test]
    fn line_break_inside_wikilink_not_matched() {
        let out = rewrite_links(
            "before [[foo\nbar]] after",
            &base(),
            &always_resolve,
            UnresolvedPolicy::PreserveText,
        );
        // Unchanged: line break inside `[[ ]]` rejects the match.
        assert_eq!(out, "before [[foo\nbar]] after");
    }

    #[test]
    fn html_special_chars_in_alias_escaped() {
        let out = rewrite_links(
            "[[foo|<bad>&\"]]",
            &base(),
            &always_resolve,
            UnresolvedPolicy::PreserveText,
        );
        assert!(out.contains("&lt;bad&gt;&amp;\""));
    }

    #[test]
    fn unaffected_by_already_rendered_anchors() {
        // Already-rendered <a> stays untouched; idempotence covers
        // re-running rewrite over its own output.
        let body = r#"<a href="/foo">foo</a> and [[bar]]"#;
        let out = rewrite_links(body, &base(), &always_resolve, UnresolvedPolicy::PreserveText);
        assert!(out.contains(r#"<a href="/foo">foo</a>"#));
        assert!(out.contains(r#"<a href="https://example.com/bar">bar</a>"#));
    }

    #[test]
    fn preserves_non_wikilink_brackets() {
        let body = "use [single] brackets and [link](http://x)";
        let out = rewrite_links(body, &base(), &always_resolve, UnresolvedPolicy::PreserveText);
        assert_eq!(out, body);
    }
}
