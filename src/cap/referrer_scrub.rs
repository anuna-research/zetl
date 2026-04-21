//! External-link referrer scrubbing for capability-mode builds
//! (SPEC-034 REQ-3413 / OBS-3407).
//!
//! The problem: when a reader clicks an external link from a
//! capability-mode page, the browser would normally send the full URL
//! (including the `/c/<path-cap>/<slug>.html` portion) in the
//! `Referer` header to the destination site. The path-cap is a
//! cohort-scoped secret — leaking it to a third party defeats the
//! purpose of the path-cap scheme.
//!
//! Two defences:
//!
//! 1. **Per-link `rel="noopener noreferrer"`**: this module rewrites
//!    external `<a>` tags during render to carry the rel attribute.
//!    Internal links are left unchanged so operators who rely on
//!    same-site Referer for internal analytics still get it.
//! 2. **Document-wide `<meta name="referrer" content="no-referrer">`**:
//!    emitted in the capability HTML shell (see
//!    [`crate::cap::html_shell`]). Belt-and-braces for any surviving
//!    link we fail to rewrite — and for browsers that honour the meta
//!    tag even when `rel` is missing.
//!
//! # Purity
//!
//! [`scrub_external_link_rels`] is a pure function: given the same
//! input HTML it produces the same output. No filesystem, clock, or
//! RNG access.
//!
//! # Operator opt-out
//!
//! The build driver reads `[access] rel_noreferrer` (default `true`)
//! and only calls this scrubber when the flag is on. Disabling is
//! documented as reducing path-cap privacy — see
//! `docs/capability-security.md`.

use std::sync::OnceLock;

use regex::Regex;

/// The exact `rel` token emitted on external `<a>` tags. `noopener`
/// blocks the opened page from reaching back via `window.opener`;
/// `noreferrer` zeroes the `Referer` header for the outgoing
/// navigation (and also implies `noopener`). Both are pinned so the
/// build output is byte-stable.
pub const REL_TOKEN: &str = "noopener noreferrer";

/// The full attribute fragment pasted before the closing `>`.
const REL_ATTR: &str = " rel=\"noopener noreferrer\"";

/// Matches an opening `<a …>` tag. `[^>]*` is safe because ammonia's
/// output never embeds `>` inside an attribute value (quoted values
/// go through ammonia's escaper) and comments/CDATA are stripped.
/// Case-insensitive so `<A>` from hand-authored HTML survives.
static A_TAG_RE: OnceLock<Regex> = OnceLock::new();

fn a_tag_re() -> &'static Regex {
    A_TAG_RE.get_or_init(|| Regex::new(r"(?i)<a(\s[^>]*)?>").expect("static regex"))
}

/// Extracts the `href="…"` or `href='…'` value from an attribute
/// blob. Returns `None` if no `href` attribute is present.
static HREF_RE: OnceLock<Regex> = OnceLock::new();

fn href_re() -> &'static Regex {
    HREF_RE.get_or_init(|| {
        Regex::new(r#"(?i)\bhref\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#).expect("static regex")
    })
}

/// Extracts an existing `rel="…"` (or `'…'`) value so the scrubber
/// can decide whether to replace or skip. Used for idempotence: a
/// second pass over already-scrubbed HTML leaves the `rel` intact.
static REL_RE: OnceLock<Regex> = OnceLock::new();

fn rel_re() -> &'static Regex {
    REL_RE.get_or_init(|| {
        Regex::new(r#"(?i)\brel\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#).expect("static regex")
    })
}

/// Returns `true` when `href` points off-origin. Anything with an
/// absolute scheme (`https://foo`, `mailto:…`, `tel:…`,
/// `javascript:…`) or a protocol-relative prefix (`//host/…`) counts.
///
/// `javascript:` and `data:` will have been stripped by the sanitiser
/// before this function sees the HTML, but classifying them as
/// external here means a caller who runs the scrubber without a prior
/// sanitise pass still gets a defence-in-depth `rel`.
///
/// Anchor-only references (`#foo`), relative paths (`./foo`,
/// `../foo`, `foo.html`), and root-relative paths (`/foo`) are
/// **internal** and left untouched.
pub fn href_is_external(href: &str) -> bool {
    let trimmed = href.trim_start();
    if trimmed.is_empty() {
        return false;
    }
    if let Some(rest) = trimmed.strip_prefix("//") {
        // Protocol-relative; `//` on its own is malformed but treat
        // as external to stay conservative.
        return !rest.is_empty();
    }
    // Look for a scheme prefix: letters / digits / `+` / `-` / `.`
    // followed by `:`. Per RFC 3986 §3.1 a scheme must start with a
    // letter.
    let mut chars = trimmed.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    let mut scheme_end = 1;
    for c in chars {
        if c == ':' {
            return scheme_end >= 1;
        }
        if c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.') {
            scheme_end += 1;
            continue;
        }
        return false;
    }
    false
}

/// Rewrites every external `<a>` tag in `html` to carry
/// `rel="noopener noreferrer"`. Internal links (anchor, relative,
/// or root-relative) are returned byte-identical.
///
/// Idempotent: running the scrubber twice produces the same output
/// as running it once. An existing `rel="…"` that already contains
/// both `noopener` and `noreferrer` (case-insensitive, any order,
/// any additional tokens) is kept as-is.
pub fn scrub_external_link_rels(html: &str) -> String {
    let re = a_tag_re();
    let mut out = String::with_capacity(html.len());
    let mut last_end = 0usize;

    for cap in re.captures_iter(html) {
        let whole = cap.get(0).unwrap();
        out.push_str(&html[last_end..whole.start()]);
        let tag = whole.as_str();
        let attrs = cap.get(1).map(|m| m.as_str()).unwrap_or("");

        let href_val = href_re()
            .captures(attrs)
            .and_then(|c| c.get(1).or(c.get(2)).or(c.get(3)))
            .map(|m| m.as_str());

        let is_external = href_val.map(href_is_external).unwrap_or(false);

        if !is_external {
            out.push_str(tag);
            last_end = whole.end();
            continue;
        }

        // External: ensure rel carries both tokens.
        match rel_re().captures(attrs) {
            Some(rc) => {
                let existing = rc
                    .get(1)
                    .or(rc.get(2))
                    .or(rc.get(3))
                    .map(|m| m.as_str())
                    .unwrap_or("");
                if rel_has_both_tokens(existing) {
                    out.push_str(tag);
                } else {
                    // Rewrite the rel value to canonical form. The
                    // sanitiser strips `rel` today so this branch is
                    // dead in the default pipeline, but it keeps
                    // idempotence well-defined when the scrubber is
                    // applied to hand-crafted HTML.
                    let rewritten = rewrite_rel_attribute(attrs, &rc);
                    out.push_str("<a");
                    out.push_str(&rewritten);
                    out.push('>');
                }
            }
            None => {
                out.push_str("<a");
                out.push_str(attrs);
                out.push_str(REL_ATTR);
                out.push('>');
            }
        }
        last_end = whole.end();
    }
    out.push_str(&html[last_end..]);
    out
}

/// Returns `true` when an existing `rel` value already carries both
/// `noopener` and `noreferrer` tokens (case-insensitive,
/// whitespace-separated).
fn rel_has_both_tokens(value: &str) -> bool {
    let mut has_noopener = false;
    let mut has_noreferrer = false;
    for token in value.split_ascii_whitespace() {
        if token.eq_ignore_ascii_case("noopener") {
            has_noopener = true;
        }
        if token.eq_ignore_ascii_case("noreferrer") {
            has_noreferrer = true;
        }
    }
    has_noopener && has_noreferrer
}

/// Splices a canonical `rel="noopener noreferrer"` into `attrs`,
/// replacing the span matched by `rc`. Preserves everything else
/// (other attributes, whitespace). Returned string includes the
/// leading whitespace of the original attrs so `<a<result>>` is
/// well-formed.
fn rewrite_rel_attribute(attrs: &str, rc: &regex::Captures<'_>) -> String {
    let whole = rc.get(0).unwrap();
    let before = &attrs[..whole.start()];
    let after = &attrs[whole.end()..];
    format!("{before}rel=\"{REL_TOKEN}\"{after}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_https_gets_rel() {
        let out = scrub_external_link_rels("<a href=\"https://example.com\">x</a>");
        assert!(
            out.contains("rel=\"noopener noreferrer\""),
            "rendered: {out}"
        );
    }

    #[test]
    fn external_http_gets_rel() {
        let out = scrub_external_link_rels("<a href=\"http://example.com\">x</a>");
        assert!(out.contains("rel=\"noopener noreferrer\""));
    }

    #[test]
    fn protocol_relative_gets_rel() {
        let out = scrub_external_link_rels("<a href=\"//evil.example\">x</a>");
        assert!(out.contains("rel=\"noopener noreferrer\""));
    }

    #[test]
    fn mailto_gets_rel() {
        let out = scrub_external_link_rels("<a href=\"mailto:alice@example.com\">x</a>");
        assert!(out.contains("rel=\"noopener noreferrer\""));
    }

    #[test]
    fn tel_gets_rel() {
        let out = scrub_external_link_rels("<a href=\"tel:+15551234\">x</a>");
        assert!(out.contains("rel=\"noopener noreferrer\""));
    }

    #[test]
    fn internal_root_relative_unchanged() {
        let html = "<a href=\"/docs/page\">x</a>";
        assert_eq!(scrub_external_link_rels(html), html);
    }

    #[test]
    fn internal_relative_unchanged() {
        let html = "<a href=\"other.html\">x</a>";
        assert_eq!(scrub_external_link_rels(html), html);
    }

    #[test]
    fn internal_dot_relative_unchanged() {
        let html = "<a href=\"./other\">x</a>";
        assert_eq!(scrub_external_link_rels(html), html);
    }

    #[test]
    fn anchor_only_unchanged() {
        let html = "<a href=\"#section\">x</a>";
        assert_eq!(scrub_external_link_rels(html), html);
    }

    #[test]
    fn empty_href_unchanged() {
        let html = "<a href=\"\">x</a>";
        assert_eq!(scrub_external_link_rels(html), html);
    }

    #[test]
    fn a_without_href_unchanged() {
        let html = "<a name=\"anchor\">x</a>";
        assert_eq!(scrub_external_link_rels(html), html);
    }

    #[test]
    fn preserves_other_attributes() {
        let out = scrub_external_link_rels(
            "<a href=\"https://example.com\" class=\"ext\" title=\"x\">link</a>",
        );
        assert!(out.contains("class=\"ext\""));
        assert!(out.contains("title=\"x\""));
        assert!(out.contains("rel=\"noopener noreferrer\""));
    }

    #[test]
    fn multiple_links_mixed() {
        let html = "<p><a href=\"/a\">int</a> and <a href=\"https://ext\">ext</a></p>";
        let out = scrub_external_link_rels(html);
        // Internal is byte-identical; external gets the rel appended.
        assert!(out.contains("<a href=\"/a\">int</a>"));
        assert!(
            out.contains("<a href=\"https://ext\" rel=\"noopener noreferrer\">ext</a>"),
            "rendered: {out}"
        );
    }

    #[test]
    fn idempotent_on_already_scrubbed_output() {
        let once = scrub_external_link_rels("<a href=\"https://ok\">x</a>");
        let twice = scrub_external_link_rels(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn preserves_case_insensitive_tag() {
        // Defensive: our regex is case-insensitive and must also
        // produce a lowercase <a on the rewrite path.
        let out = scrub_external_link_rels("<A HREF=\"https://ok\">x</A>");
        assert!(out.contains("rel=\"noopener noreferrer\""));
    }

    #[test]
    fn existing_rel_with_both_tokens_is_kept() {
        // Caller-authored HTML with rel already correct — don't
        // double-rewrite.
        let html = "<a href=\"https://ok\" rel=\"noopener noreferrer\">x</a>";
        assert_eq!(scrub_external_link_rels(html), html);
    }

    #[test]
    fn existing_rel_missing_token_is_rewritten() {
        let out = scrub_external_link_rels("<a href=\"https://ok\" rel=\"preconnect\">x</a>");
        assert!(out.contains("rel=\"noopener noreferrer\""));
        assert!(!out.contains("preconnect"), "rendered: {out}");
    }

    #[test]
    fn single_quoted_href_external() {
        let out = scrub_external_link_rels("<a href='https://ok'>x</a>");
        assert!(out.contains("rel=\"noopener noreferrer\""));
    }

    #[test]
    fn href_is_external_classifier() {
        for ext in [
            "https://example.com",
            "http://example.com",
            "mailto:a@b",
            "tel:+1",
            "//other.example/path",
            "ftp://files",
        ] {
            assert!(href_is_external(ext), "{ext} should be external");
        }
        for int in [
            "", "#foo", "/abs", "rel", "./rel", "../rel", "foo.html", "?q=1",
        ] {
            assert!(!href_is_external(int), "{int} should be internal");
        }
    }

    #[test]
    fn non_a_tags_unchanged() {
        let html = "<img src=\"https://ext/i.png\"><p><img></p>";
        assert_eq!(scrub_external_link_rels(html), html);
    }

    #[test]
    fn no_links_at_all_is_identity() {
        let html = "<h1>Title</h1><p>body</p>";
        assert_eq!(scrub_external_link_rels(html), html);
    }
}
