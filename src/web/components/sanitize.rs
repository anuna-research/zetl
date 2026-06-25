//! SPEC-049 CON-4902 — content output sanitiser (closed HTML allowlist).
//!
//! Applied to the **untrusted body HTML in isolation** (REQ-4905), *before* it is
//! slotted into a trusted component template — never to the composite output. Because it
//! only ever sees untrusted body HTML, provenance is preserved by construction and it
//! can be a closed default-deny allowlist without gutting trusted templates.
//!
//! Built on `ammonia` (a `html5ever`-backed allowlist sanitiser — one HTML namespace, no
//! second divergent parser). On top of ammonia's allowlist this module adds:
//! - a **scheme allowlist after canonicalisation** that rejects protocol-relative
//!   `//host` / `\\host` and obfuscated schemes (`Java\tscript:`), which ammonia's
//!   built-in scheme check alone would let through as "relative" ([`is_safe_url`]);
//! - a `srcset` sub-grammar validator (each candidate URL validated; unparseable →
//!   dropped);
//! - a **serialise→reparse fixed-point** check helper (re-sanitising output is a no-op —
//!   the mutation-XSS guard);
//! - a best-effort **drop report** (kinds + counts, never the payload — OBS-4901).
//!
//! `is_safe_url` and [`URL_ATTRS`] are the single shared recognisers reused by the
//! `url`-ptype ingestion validation (REQ-4904) and the CON-4904 context lint.

use ammonia::Builder;
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::OnceLock;

/// CON-4902 element allowlist: safe prose/structure. Everything else → inert text
/// (element dropped, its text kept), except the content-stripped tags below.
const ALLOWED_TAGS: &[&str] = &[
    "p", "h1", "h2", "h3", "h4", "h5", "h6", "ul", "ol", "li", "blockquote", "pre", "code",
    "em", "strong", "a", "img", "figure", "figcaption", "table", "thead", "tbody", "tr", "th",
    "td", "hr", "br", "span", "div",
];

/// Hard-forbidden tags whose **content is also removed** (not left as inert text): code
/// (`script`/`style`/`template`/`noscript`) and the mXSS-foreign-content pivots
/// (`svg`/`math`) plus framing/embedding elements.
const STRIP_CONTENT_TAGS: &[&str] = &[
    "script", "style", "template", "noscript", "svg", "math", "iframe", "object", "embed",
    "applet", "head", "title", "textarea",
];

/// Generic attributes allowed on any tag. Deliberately minimal — `style`, `id`, `name`,
/// `is`, `srcdoc`, `xlink:*`, and all `on*` are NOT here and are stripped.
const GENERIC_ATTRIBUTES: &[&str] = &["class", "title", "aria-hidden"];

/// CON-4902 / CON-4904(1) closed `URL_ATTR` set — pinned ONCE and shared verbatim with
/// the context lint. A URL attribute value (or a `url`-typed prop) must pass
/// [`is_safe_url`].
pub const URL_ATTRS: &[&str] = &[
    "href",
    "src",
    "srcset",
    "poster",
    "action",
    "formaction",
    "cite",
    "data",
    "background",
    "ping",
    "usemap",
    "longdesc",
];

/// Allowed URL schemes after canonicalisation (CON-4902).
const ALLOWED_SCHEMES: &[&str] = &["http", "https", "mailto"];

fn per_tag_attributes() -> HashMap<&'static str, HashSet<&'static str>> {
    let mut m: HashMap<&'static str, HashSet<&'static str>> = HashMap::new();
    m.insert("a", ["href"].into_iter().collect());
    m.insert(
        "img",
        ["src", "srcset", "alt", "width", "height"].into_iter().collect(),
    );
    m.insert("td", ["colspan", "rowspan"].into_iter().collect());
    m.insert("th", ["colspan", "rowspan", "scope"].into_iter().collect());
    m.insert("ol", ["start"].into_iter().collect());
    m
}

/// Is `attr` a URL-bearing attribute (CON-4904(1) closed set)?
pub fn is_url_attr(attr: &str) -> bool {
    URL_ATTRS.contains(&attr)
}

/// Canonicalise a URL value for scheme recognition: lowercase and strip every ASCII
/// control character and whitespace (so `Java\tscript:` ≡ `javascript:`).
fn canonicalise(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_ascii_control() && !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase()
}

/// CON-4902 URL recogniser (shared with the `url` ptype, REQ-4904). Accepts `http`,
/// `https`, `mailto`, or a path-absolute / path-relative reference. **Rejects**
/// scheme-relative (`//host`, `\\host` after canonicalisation — resolves off-origin) and
/// any non-allowlisted scheme (`javascript:`, `data:`, `blob:`, `vbscript:`, `file:`, …).
pub fn is_safe_url(value: &str) -> bool {
    let canon = canonicalise(value);
    if canon.is_empty() {
        // empty / whitespace-only → treat as a harmless empty reference
        return true;
    }
    // scheme-relative resolves off-origin → reject
    if canon.starts_with("//") || canon.starts_with("\\\\") || canon.starts_with("/\\")
        || canon.starts_with("\\/")
    {
        return false;
    }
    // detect a scheme: `[a-z][a-z0-9+.-]* :` appearing before any '/', '?', '#'
    if let Some(colon) = canon.find(':') {
        let before = &canon[..colon];
        let has_path_sep = canon[..colon].contains('/')
            || canon[..colon].contains('?')
            || canon[..colon].contains('#');
        let looks_like_scheme = !before.is_empty()
            && before
                .bytes()
                .next()
                .map(|b| b.is_ascii_lowercase())
                .unwrap_or(false)
            && before
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'+' | b'.' | b'-'));
        if !has_path_sep && looks_like_scheme {
            return ALLOWED_SCHEMES.contains(&before);
        }
    }
    // no scheme → a relative reference (path-absolute or path-relative) → accept
    true
}

/// Validate a `srcset` value: comma-separated `url [descriptor]` candidates. Every
/// candidate URL must pass [`is_safe_url`]. Returns the original value if all valid,
/// `None` (drop the whole attribute) if any candidate is unsafe or unparseable.
fn sanitise_srcset(value: &str) -> Option<()> {
    for candidate in value.split(',') {
        let url = candidate.split_whitespace().next().unwrap_or("");
        if url.is_empty() || !is_safe_url(url) {
            return None;
        }
    }
    Some(())
}

/// Build the CON-4902 ammonia policy. Cached for the per-page loop.
fn policy() -> Builder<'static> {
    let tags: HashSet<&'static str> = ALLOWED_TAGS.iter().copied().collect();
    let strip_content: HashSet<&'static str> = STRIP_CONTENT_TAGS.iter().copied().collect();
    let generic: HashSet<&'static str> = GENERIC_ATTRIBUTES.iter().copied().collect();
    let schemes: HashSet<&'static str> = ALLOWED_SCHEMES.iter().copied().collect();

    let mut b = Builder::default();
    b.tags(tags)
        .clean_content_tags(strip_content)
        .generic_attributes(generic)
        .tag_attributes(per_tag_attributes())
        .url_schemes(schemes)
        // Keep path-relative/absolute URLs (validated by the filter); reject nothing
        // here — the attribute_filter is the authoritative URL check.
        .url_relative(ammonia::UrlRelative::PassThrough)
        .link_rel(Some("noopener noreferrer"))
        // Strip HTML comments / PIs (classic mXSS pivots, CON-4902).
        .strip_comments(true)
        .id_prefix(None)
        // Authoritative URL recognition: scheme allowlist after canonicalisation +
        // protocol-relative rejection + srcset sub-grammar. Runs before url_relative.
        .attribute_filter(|_element, attribute, value| {
            if is_url_attr(attribute) {
                if attribute == "srcset" {
                    return sanitise_srcset(value).map(|()| Cow::Borrowed(value));
                }
                if is_safe_url(value) {
                    Some(Cow::Borrowed(value))
                } else {
                    None // drop the unsafe URL attribute
                }
            } else {
                Some(Cow::Borrowed(value))
            }
        });
    b
}

fn cached() -> &'static Builder<'static> {
    static CELL: OnceLock<Builder<'static>> = OnceLock::new();
    CELL.get_or_init(policy)
}

/// A best-effort summary of what the sanitiser removed, for author diagnostics
/// (OBS-4901: kinds + counts, **never** the rejected markup verbatim — that would be a
/// sanitiser-probing oracle).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SanitiseReport {
    pub dropped: BTreeMap<String, usize>,
}

impl SanitiseReport {
    pub fn is_empty(&self) -> bool {
        self.dropped.is_empty()
    }

    /// One-line human summary, e.g. `3 script dropped, 1 on*-attribute dropped`.
    pub fn summary(&self) -> String {
        self.dropped
            .iter()
            .map(|(k, n)| format!("{n} {k} dropped"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn bump(&mut self, kind: &str, n: usize) {
        if n > 0 {
            *self.dropped.entry(kind.to_string()).or_insert(0) += n;
        }
    }
}

/// Count removable constructs in the *input* HTML, as an author-facing estimate of what
/// the allowlist pass will strip. Source-side, so it never echoes the rejected payload.
fn count_drops(input: &str) -> SanitiseReport {
    use regex::Regex;
    static FORBIDDEN: OnceLock<Regex> = OnceLock::new();
    static EVENT_ATTR: OnceLock<Regex> = OnceLock::new();
    let forbidden = FORBIDDEN.get_or_init(|| {
        Regex::new(r"(?i)<\s*(script|style|iframe|object|embed|applet|form|input|button|template|noscript|svg|math|base|meta|link)\b").unwrap()
    });
    let event_attr = EVENT_ATTR.get_or_init(|| Regex::new(r"(?i)\son[a-z]+\s*=").unwrap());

    let mut report = SanitiseReport::default();
    for cap in forbidden.captures_iter(input) {
        let tag = cap[1].to_ascii_lowercase();
        report.bump(&tag, 1);
    }
    report.bump("on*-attribute", event_attr.find_iter(input).count());
    report.bump("comment", input.matches("<!--").count());
    report
}

/// Sanitise an untrusted body HTML fragment (CON-4902). Returns the cleaned HTML and a
/// best-effort drop report.
pub fn sanitise_body(html: &str) -> (String, SanitiseReport) {
    let report = count_drops(html);
    let cleaned = cached().clean(html).to_string();
    (cleaned, report)
}

/// CON-4902 fixed-point invariant helper: `sanitise(sanitise(x)) == sanitise(x)`.
/// Exposed for tests and a debug assertion in the expansion path.
pub fn is_fixed_point(html: &str) -> bool {
    let once = cached().clean(html).to_string();
    let twice = cached().clean(&once).to_string();
    once == twice
}

#[cfg(test)]
mod tests {
    use super::*;

    fn san(s: &str) -> String {
        sanitise_body(s).0
    }

    #[test]
    fn strips_script_tag_and_content() {
        let out = san("<p>ok</p><script>alert(1)</script>");
        assert!(!out.contains("<script"));
        assert!(!out.contains("alert(1)"), "script content gone: {out}");
        assert!(out.contains("<p>ok</p>"));
    }

    #[test]
    fn strips_onerror_attribute() {
        let out = san(r#"<img src="x.png" onerror="alert(1)">"#);
        assert!(!out.contains("onerror"));
        assert!(out.contains("src=\"x.png\""));
    }

    #[test]
    fn drops_javascript_href() {
        let out = san(r#"<a href="javascript:alert(1)">x</a>"#);
        assert!(!out.contains("javascript"));
    }

    #[test]
    fn drops_obfuscated_scheme() {
        // Java\tscript: canonicalises to javascript: → rejected.
        let out = san("<a href=\"Java\tscript:alert(1)\">x</a>");
        assert!(!out.to_ascii_lowercase().contains("script:"));
    }

    #[test]
    fn rejects_protocol_relative_url() {
        assert!(!is_safe_url("//evil.com/x"));
        assert!(!is_safe_url("\\\\evil.com"));
        let out = san(r#"<a href="//evil.com">x</a>"#);
        assert!(!out.contains("evil.com"));
    }

    #[test]
    fn keeps_safe_urls() {
        assert!(is_safe_url("https://example.com/x"));
        assert!(is_safe_url("http://example.com"));
        assert!(is_safe_url("mailto:a@b.com"));
        assert!(is_safe_url("/path/abs"));
        assert!(is_safe_url("relative/path"));
        assert!(is_safe_url("#frag"));
        assert!(is_safe_url("?q=1"));
    }

    #[test]
    fn rejects_dangerous_schemes() {
        for u in ["javascript:x", "data:text/html,x", "blob:x", "vbscript:x", "file:///etc"] {
            assert!(!is_safe_url(u), "{u} must be rejected");
        }
    }

    #[test]
    fn strips_iframe_and_style_and_form() {
        let out = san("<iframe src=x></iframe><style>body{}</style><form><input></form>text");
        assert!(!out.contains("<iframe"));
        assert!(!out.contains("<style"));
        assert!(!out.contains("<form"));
        assert!(!out.contains("<input"));
        assert!(out.contains("text"));
    }

    #[test]
    fn strips_comments() {
        let out = san("<p>a</p><!-- evil --><p>b</p>");
        assert!(!out.contains("<!--"));
    }

    #[test]
    fn srcset_with_bad_candidate_dropped() {
        let out = san(r#"<img src="ok.png" srcset="ok.png 1x, javascript:alert(1) 2x">"#);
        assert!(!out.contains("srcset"), "whole srcset dropped: {out}");
    }

    #[test]
    fn srcset_all_valid_kept() {
        let out = san(r#"<img src="ok.png" srcset="a.png 1x, b.png 2x">"#);
        assert!(out.contains("srcset"));
    }

    #[test]
    fn allowlisted_prose_preserved() {
        let input = "<p>Hello <strong>world</strong> and a <a href=\"/x\">link</a>.</p>";
        let out = san(input);
        assert!(out.contains("<strong>world</strong>"));
        assert!(out.contains("href=\"/x\""));
    }

    #[test]
    fn fixed_point_stable() {
        let vectors = [
            "<p>ok</p><script>x</script>",
            r#"<img src="x" onerror="y">"#,
            "<svg><foreignObject><p>x</p></foreignObject></svg>",
            "<p><!-- c --><a href=\"//e\">x</a></p>",
        ];
        for v in vectors {
            assert!(is_fixed_point(v), "not fixed point: {v}");
            let once = san(v);
            assert_eq!(san(&once), once, "re-sanitise not byte-identical: {v}");
        }
    }

    #[test]
    fn drop_report_counts_kinds() {
        let (_html, report) =
            sanitise_body("<script>a</script><script>b</script><img onerror=x><!-- c -->");
        assert_eq!(report.dropped.get("script"), Some(&2));
        assert_eq!(report.dropped.get("on*-attribute"), Some(&1));
        assert_eq!(report.dropped.get("comment"), Some(&1));
        assert!(report.summary().contains("script dropped"));
    }
}
