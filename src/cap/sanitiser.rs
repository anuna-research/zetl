//! HTML sanitiser for capability-mode builds (SPEC-034 REQ-3421).
//!
//! Wraps the `ammonia` crate with an allowlist that mirrors the OWASP
//! HTML-sanitisation recommendations and the operator-visible policy at
//! `tools/sanitiser-config.toml`. The two artefacts are kept in lockstep
//! by `tests/cap_sanitiser_integration.rs::test_config_mirrors_spec` —
//! any drift fails CI.
//!
//! # Policy summary
//!
//! Denied (per REQ-3421, enforced by the allowlist below):
//!
//! - `<script>`, `<iframe>`, `<object>`, `<embed>`, `<template>`, `<math>`,
//!   `<style>`, `<base>`, `<meta>`, `<link>`, `<form>`, `<button>`,
//!   `<input>`, `<frame>`, `<frameset>`, `<noframes>`
//! - All event-handler attributes (`on*`)
//! - `srcdoc`, `formaction`, `ping`, `integrity`, `nonce`, `http-equiv`,
//!   `xmlns`, `is`
//! - `javascript:`, `data:`, `vbscript:`, `file:`, `about:` URIs in any
//!   URL-valued attribute (`href`, `src`, `cite`, `action`, …)
//!
//! Preserved (CommonMark + common GFM output):
//!
//! - All block + inline flow elements the CommonMark renderer emits
//!   (h1–h6, p, blockquote, code/pre, lists, tables, em, strong, etc.)
//! - A conservative set of ARIA attributes on every tag
//! - `href` on `<a>`, `src`/`alt` on `<img>`, etc.
//!
//! Active-content tags (`<script>`, `<style>`, `<iframe>`, `<template>`,
//! `<math>`, and their SVG analogues) are removed **with their text
//! children** via `ammonia::Builder::clean_content_tags`, so
//! `<script>alert(1)</script>` leaves neither markup nor the text
//! `alert(1)` behind.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use ammonia::Builder;

/// Operator-visible copy of the sanitiser policy. Bundled into the binary
/// so `zetl cap check` (and tests) can diff the Rust policy against the
/// authoritative TOML without filesystem access.
pub const CONFIG_TOML: &str = include_str!("../../tools/sanitiser-config.toml");

/// Tags allowed through the sanitiser. Mirrors the `[tags].allow` list
/// in `tools/sanitiser-config.toml`.
pub const ALLOWED_TAGS: &[&str] = &[
    "a",
    "abbr",
    "article",
    "aside",
    "b",
    "bdi",
    "bdo",
    "blockquote",
    "br",
    "caption",
    "cite",
    "code",
    "data",
    "dd",
    "del",
    "details",
    "dfn",
    "div",
    "dl",
    "dt",
    "em",
    "figcaption",
    "figure",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "i",
    "img",
    "ins",
    "kbd",
    "li",
    "main",
    "mark",
    "nav",
    "ol",
    "p",
    "pre",
    "q",
    "rp",
    "rt",
    "ruby",
    "s",
    "samp",
    "section",
    "small",
    "span",
    "strong",
    "sub",
    "summary",
    "sup",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "time",
    "tr",
    "u",
    "ul",
    "var",
    "wbr",
];

/// Tags whose element AND text children are removed. A `<script>` that
/// contained `alert(1)` leaves no trace — not even the bare text. Also
/// covers tags that render as inert containers when stripped without
/// their contents (e.g. `<style>body{}` would leave the CSS text node).
pub const STRIP_CONTENT_TAGS: &[&str] = &[
    "script", "style", "iframe", "object", "embed", "template", "math", "base", "frame",
    "frameset", "noframes", "noscript", "xmp",
];

/// Attributes allowed on every tag. Conservative ARIA + presentation set;
/// `style` is intentionally absent (no `@import` / `expression()` vectors),
/// as are `on*` event handlers.
pub const GENERIC_ATTRIBUTES: &[&str] = &[
    "id",
    "class",
    "title",
    "lang",
    "dir",
    "role",
    "aria-label",
    "aria-hidden",
    "aria-describedby",
    "aria-labelledby",
];

/// URL schemes permitted in href/src/cite/action-style attributes.
/// Excludes `javascript:`, `data:`, `vbscript:`, `file:`, `about:`.
pub const ALLOWED_URL_SCHEMES: &[&str] = &["http", "https", "mailto", "tel"];

/// Named attributes the spec explicitly calls out as denied even when
/// they appear on an otherwise-allowed tag. Kept as a const for test
/// coverage; enforcement is structural (they never appear in any
/// per-tag allowlist below).
pub const DENIED_ATTRIBUTES: &[&str] = &[
    "srcdoc",
    "formaction",
    "ping",
    "http-equiv",
    "integrity",
    "nonce",
    "xmlns",
    "is",
];

fn per_tag_attributes() -> HashMap<&'static str, HashSet<&'static str>> {
    let mut map: HashMap<&'static str, HashSet<&'static str>> = HashMap::new();
    // Note: `rel` is intentionally absent — ammonia's `link_rel`
    // forcibly replaces every `<a>` rel with `"noopener noreferrer"`
    // and asserts no per-tag `rel` allowlist entry coexists with it.
    map.insert(
        "a",
        ["href", "hreflang", "target", "type", "download"]
            .into_iter()
            .collect(),
    );
    map.insert(
        "img",
        ["src", "alt", "width", "height", "loading", "decoding"]
            .into_iter()
            .collect(),
    );
    map.insert("blockquote", ["cite"].into_iter().collect());
    map.insert("q", ["cite"].into_iter().collect());
    map.insert("time", ["datetime"].into_iter().collect());
    map.insert(
        "th",
        ["scope", "colspan", "rowspan", "abbr", "headers"]
            .into_iter()
            .collect(),
    );
    map.insert(
        "td",
        ["colspan", "rowspan", "headers"].into_iter().collect(),
    );
    map.insert("col", ["span"].into_iter().collect());
    map.insert("colgroup", ["span"].into_iter().collect());
    map.insert("ol", ["start", "type", "reversed"].into_iter().collect());
    map.insert("li", ["value"].into_iter().collect());
    map.insert("details", ["open"].into_iter().collect());
    map.insert("data", ["value"].into_iter().collect());
    map.insert("code", ["data-lang"].into_iter().collect());
    map.insert("pre", ["data-lang"].into_iter().collect());
    map
}

/// Build a fresh `ammonia::Builder` pre-configured with the REQ-3421
/// policy. Exposed for callers that need to layer additional operator
/// overrides before sanitising, and for tests that introspect the
/// resulting config. Most callers should use [`sanitise`] or
/// [`Sanitiser::default`] instead.
pub fn policy() -> Builder<'static> {
    let tags: HashSet<&'static str> = ALLOWED_TAGS.iter().copied().collect();
    let strip_content: HashSet<&'static str> = STRIP_CONTENT_TAGS.iter().copied().collect();
    let generic: HashSet<&'static str> = GENERIC_ATTRIBUTES.iter().copied().collect();
    let url_schemes: HashSet<&'static str> = ALLOWED_URL_SCHEMES.iter().copied().collect();

    let mut b = Builder::default();
    b.tags(tags)
        .clean_content_tags(strip_content)
        .generic_attributes(generic)
        .tag_attributes(per_tag_attributes())
        .url_schemes(url_schemes)
        // `rel` is not in the <a> allowlist so any author-provided rel
        // value is dropped by the allowlist pass. We deliberately do
        // NOT use ammonia's `link_rel` hook to force rel on every
        // surviving link: REQ-3413 scrubs only *external* links and
        // leaves internal navigation untouched so operators can still
        // rely on same-site Referer for internal analytics. The
        // downstream step in `cap::referrer_scrub` adds the canonical
        // `rel="noopener noreferrer"` to external hrefs only.
        .link_rel(None)
        // No relative URL base: the cap-mode shim rewrites hrefs after
        // sanitisation anyway (CON-3408 step 5).
        .url_relative(ammonia::UrlRelative::PassThrough)
        // Strip HTML comments — historically a smuggling vector and
        // carries no semantic value in CommonMark output.
        .strip_comments(true)
        // Deny everything by default at the tag level (no id-for-all
        // default that would leak CSS surface).
        .id_prefix(None);
    b
}

/// Reusable sanitiser handle. Cheap to clone; construction compiles the
/// ammonia rules. Prefer the cached [`Sanitiser::default`] in hot paths.
pub struct Sanitiser {
    builder: Builder<'static>,
}

impl Sanitiser {
    /// Build a new sanitiser from [`policy`]. Callers that need to
    /// customise should construct via `Builder::default()` directly and
    /// document their divergence.
    pub fn new() -> Self {
        Self { builder: policy() }
    }

    /// Clean an HTML string. Output is a well-formed HTML fragment with
    /// all disallowed elements, attributes, and URL schemes stripped.
    pub fn clean(&self, html: &str) -> String {
        self.builder.clean(html).to_string()
    }
}

impl Default for Sanitiser {
    fn default() -> Self {
        Self::new()
    }
}

/// Sanitise `html` using a process-wide cached policy.
///
/// The underlying `ammonia::Builder` holds allowlist tables that are
/// invariant for the life of the process; rebuilding per call wastes
/// cycles in the build driver's per-page loop.
pub fn sanitise(html: &str) -> String {
    static CELL: OnceLock<Builder<'static>> = OnceLock::new();
    let builder = CELL.get_or_init(policy);
    builder.clean(html).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_script_tag_and_content() {
        let out = sanitise("<p>ok</p><script>alert(1)</script>");
        assert!(!out.contains("<script"));
        assert!(
            !out.contains("alert(1)"),
            "script content must not survive: {}",
            out
        );
    }

    #[test]
    fn strips_iframe() {
        let out = sanitise("<iframe src=\"https://evil\"></iframe>");
        assert!(!out.contains("<iframe"));
    }

    #[test]
    fn strips_object_embed() {
        let out = sanitise("<object data=\"x\"></object><embed src=\"y\">");
        assert!(!out.contains("<object"));
        assert!(!out.contains("<embed"));
    }

    #[test]
    fn strips_base_and_meta() {
        let out = sanitise("<base href=\"http://evil\"><meta http-equiv=\"refresh\" content=\"0;url=http://evil\">");
        assert!(!out.contains("<base"));
        assert!(!out.contains("<meta"));
        assert!(!out.contains("http-equiv"));
    }

    #[test]
    fn strips_link_preconnect() {
        let out = sanitise("<link rel=\"preconnect\" href=\"https://tracker.example\">");
        assert!(!out.contains("<link"), "rendered: {}", out);
    }

    #[test]
    fn strips_event_handlers() {
        let out = sanitise("<a href=\"https://ok\" onclick=\"x()\" onerror=\"y()\">hi</a>");
        assert!(!out.contains("onclick"));
        assert!(!out.contains("onerror"));
        assert!(
            out.contains("href=\"https://ok\""),
            "href should survive: {}",
            out
        );
    }

    #[test]
    fn strips_javascript_uri() {
        let out = sanitise("<a href=\"javascript:alert(1)\">x</a>");
        assert!(!out.contains("javascript:"), "rendered: {}", out);
    }

    #[test]
    fn strips_data_uri_in_href() {
        let out = sanitise("<a href=\"data:text/html,<script>\">x</a>");
        assert!(!out.contains("data:"));
    }

    #[test]
    fn strips_data_uri_in_img_src() {
        let out = sanitise("<img src=\"data:image/svg+xml,<svg onload=alert(1)>\">");
        assert!(!out.contains("data:"), "rendered: {}", out);
    }

    #[test]
    fn strips_vbscript_uri() {
        let out = sanitise("<a href=\"vbscript:msgbox(1)\">x</a>");
        assert!(!out.contains("vbscript:"));
    }

    #[test]
    fn strips_srcdoc_formaction_ping() {
        let out = sanitise(
            "<iframe srcdoc=\"<script>\"></iframe>\
             <button formaction=\"javascript:\">x</button>\
             <a href=\"https://ok\" ping=\"https://tracker\">p</a>",
        );
        assert!(!out.contains("srcdoc"));
        assert!(!out.contains("formaction"));
        assert!(!out.contains("ping"));
    }

    #[test]
    fn strips_svg_script() {
        let out = sanitise("<svg><script>alert(1)</script></svg>");
        // Whole <svg> is not allowlisted; the <script> inside it is in
        // strip_content_tags so neither its open tag nor the alert text
        // survives.
        assert!(!out.contains("<svg"));
        assert!(!out.contains("<script"));
        assert!(!out.contains("alert(1)"));
    }

    #[test]
    fn strips_math_tag() {
        let out = sanitise("<math><mi>x</mi></math>");
        assert!(!out.contains("<math"));
    }

    #[test]
    fn strips_template() {
        let out = sanitise("<template><script>alert(1)</script></template>");
        assert!(!out.contains("<template"));
        assert!(!out.contains("<script"));
        assert!(!out.contains("alert(1)"));
    }

    #[test]
    fn strips_style_tag_with_at_import() {
        let out = sanitise("<style>@import url('https://evil');</style><p>ok</p>");
        assert!(!out.contains("<style"));
        assert!(!out.contains("@import"));
        assert!(out.contains("<p>ok</p>"));
    }

    #[test]
    fn strips_inline_style() {
        let out = sanitise("<p style=\"background:url(javascript:alert(1))\">x</p>");
        assert!(!out.contains("style="));
        assert!(!out.contains("javascript:"));
    }

    #[test]
    fn preserves_commonmark_output() {
        let html = "<h1>Title</h1>\
                    <p>Body with <em>emphasis</em>, <strong>bold</strong>, \
                    <a href=\"https://example.com\">link</a>, and <code>code</code>.</p>\
                    <blockquote cite=\"https://src\"><p>quoted</p></blockquote>\
                    <pre><code>fn f() {}</code></pre>\
                    <ul><li>one</li><li>two</li></ul>\
                    <ol start=\"3\"><li>three</li></ol>\
                    <table><thead><tr><th scope=\"col\">h</th></tr></thead>\
                    <tbody><tr><td>d</td></tr></tbody></table>\
                    <hr><p>after</p>";
        let out = sanitise(html);
        for frag in [
            "<h1>",
            "<em>",
            "<strong>",
            "<code>",
            "<blockquote",
            "<pre>",
            "<ul>",
            "<li>",
            "<ol ",
            "<table>",
            "<thead>",
            "<th scope=\"col\">",
            "<hr",
            "href=\"https://example.com\"",
        ] {
            assert!(out.contains(frag), "missing {} in: {}", frag, out);
        }
    }

    #[test]
    fn strips_author_rel_so_referrer_scrub_owns_rel_policy() {
        // REQ-3413 is implemented by the downstream `referrer_scrub`
        // module, not by the sanitiser. The sanitiser just has to
        // guarantee that any author-supplied `rel` (e.g.
        // `rel="preconnect"` or `rel="dns-prefetch"`) is stripped so
        // the scrubber sees a clean slate when deciding whether to
        // add `noopener noreferrer` on external links.
        let out = sanitise("<a href=\"https://ok\" rel=\"preconnect\">x</a>");
        assert!(!out.contains("preconnect"), "rendered: {}", out);
        assert!(!out.contains("rel="), "rendered: {}", out);
    }

    #[test]
    fn strips_html_comments() {
        let out = sanitise("<p>before</p><!-- secret --><p>after</p>");
        assert!(!out.contains("<!--"));
        assert!(!out.contains("secret"));
    }

    #[test]
    fn mixed_case_script_stripped() {
        // Adversarial: case-variant tag names. Ammonia normalises case.
        let out = sanitise("<ScRiPt>alert(1)</ScRiPt>");
        assert!(!out.to_lowercase().contains("<script"));
        assert!(!out.contains("alert(1)"));
    }

    #[test]
    fn nested_obfuscated_xss_stripped() {
        // From the OWASP XSS Filter Evasion cheatsheet: nested
        // script-in-comment-in-script pattern.
        let out = sanitise("<SCRIPT>a=/XSS/\nalert(a.source)</SCRIPT>");
        assert!(!out.to_lowercase().contains("<script"));
        assert!(!out.contains("alert"));
    }
}
