//! Vault-diff malicious-content scanner for capability-mode PR gating.
//!
//! SPEC-034 REQ-3424 + ADR-3410 (BUG-016 resolution). Pure-core module:
//! given two flat sets of `(path, contents)` pairs representing the
//! baseline and new vault states, emit a `Vec<Finding>` describing any
//! suspicious construct introduced in the new set. The effectful shell
//! (`src/main.rs::cmd_cap_audit_diff`) is responsible for enumerating
//! the vault at two git refs; everything here operates on strings.
//!
//! # Heuristics
//!
//! Four detector families match the spec:
//!
//! 1. **Unseen external-origin links** — any URL whose host is not
//!    present in the baseline domain set. The host is compared
//!    case-insensitively; `www.` prefix is normalised away.
//! 2. **Raw HTML surviving the sanitiser** — any HTML block or inline
//!    HTML span in new markdown that survives a pass through
//!    `cap::sanitiser::sanitise` with non-empty output. The author had
//!    to type raw HTML for this to happen, so every surviving fragment
//!    is a reviewer-check point (even if structurally safe). If the
//!    pre-sanitised fragment contains a denylisted construct that the
//!    sanitiser strips, we additionally flag it under
//!    `SanitiserStripped` so the reviewer sees the attack attempt even
//!    though the runtime output is safe.
//! 3. **Dangerous URI schemes** — `javascript:`, `data:`, `vbscript:`,
//!    `file:`, `about:` in any markdown link, image, or autolink. These
//!    rarely appear in innocent prose; the sanitiser strips them, but
//!    the *intent* is a signal we surface.
//! 4. **Dynamically-constructed URIs** — URLs containing template
//!    markers (`{{...}}`, `${...}`, `<%...%>`) indicating the link was
//!    generated at render time rather than typed as a literal. These
//!    bypass the domain-baseline check because the final host is
//!    unknown statically.
//!
//! # Output
//!
//! `scan_diff` returns findings in the deterministic order
//! `(path, line, kind)` to make CI log diffs stable across reruns.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use pulldown_cmark::{CowStr, Event, Options, Parser, Tag};
use url::Url;

/// One reviewer-visible finding. Carries enough context to point an
/// operator at the exact line in the new vault state.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Finding {
    pub kind: FindingKind,
    pub path: PathBuf,
    pub line: usize,
    pub excerpt: String,
}

/// Finding taxonomy. The string variants appear verbatim in CLI output
/// and the corpus fixture `expected.txt` lines — rename with care.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum FindingKind {
    /// External link to a host not seen in the baseline vault.
    UnseenDomain { domain: String, url: String },
    /// Raw HTML fragment in markdown source. `tag` is the first tag
    /// name encountered in the fragment (lowercased) or `"fragment"`.
    RawHtml { tag: String, fragment: String },
    /// Sanitiser stripped the author's markup — they tried to emit
    /// content that the allowlist rejects. Attack-intent signal.
    SanitiserStripped { detail: String, fragment: String },
    /// Markdown URL uses a scheme on the spec denylist
    /// (`javascript`, `data`, `vbscript`, `file`, `about`).
    DangerousScheme { scheme: String, url: String },
    /// URL contains a template expression — the final target cannot
    /// be validated statically.
    DynamicUri { url: String },
}

impl FindingKind {
    /// One-word identifier used by corpus `expected.txt` files.
    pub fn tag(&self) -> &'static str {
        match self {
            FindingKind::UnseenDomain { .. } => "unseen-domain",
            FindingKind::RawHtml { .. } => "raw-html",
            FindingKind::SanitiserStripped { .. } => "sanitiser-stripped",
            FindingKind::DangerousScheme { .. } => "dangerous-scheme",
            FindingKind::DynamicUri { .. } => "dynamic-uri",
        }
    }
}

/// A `(path, contents)` pair as seen at one git ref.
#[derive(Debug, Clone)]
pub struct Page<'a> {
    pub path: &'a Path,
    pub contents: &'a str,
}

/// Schemes whose presence in any URL position is always reported, even
/// if the sanitiser would strip them from rendered output.
const DANGEROUS_SCHEMES: &[&str] = &["javascript", "data", "vbscript", "file", "about"];

/// Schemes treated as "external origins" whose host is baseline-checked.
/// `mailto:` and `tel:` have no host component and are silently allowed.
const EXTERNAL_SCHEMES: &[&str] = &["http", "https"];

/// Walk a set of baseline pages and collect every host appearing in
/// markdown links, images, autolinks, or link-reference definitions.
/// Hosts are lowercased with any leading `www.` trimmed so CI doesn't
/// false-positive on cosmetic redirects.
pub fn collect_domains(pages: &[Page]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for page in pages {
        for (url, _) in extract_urls(page.contents) {
            if let Some(host) = host_of(&url) {
                out.insert(host);
            }
        }
    }
    out
}

/// Scan one page and emit every finding. The `baseline_domains` set
/// gates `UnseenDomain` firings; pass an empty set to treat every
/// external link as new (useful for corpus fixtures that ship only a
/// `new/` side).
pub fn scan_page(page: &Page, baseline_domains: &BTreeSet<String>) -> Vec<Finding> {
    let mut findings = Vec::new();

    for (url, line) in extract_urls(page.contents) {
        classify_url(&url, line, page.path, baseline_domains, &mut findings);
    }

    for (fragment, line) in extract_raw_html(page.contents) {
        classify_html(&fragment, line, page.path, &mut findings);
    }

    findings.sort();
    findings
}

/// Walk the full diff: baseline + new. Emits findings only for pages in
/// `new_pages`; `baseline_pages` are consulted for the domain allowlist.
pub fn scan_diff(baseline_pages: &[Page], new_pages: &[Page]) -> Vec<Finding> {
    let baseline_domains = collect_domains(baseline_pages);
    let mut all: Vec<Finding> = new_pages
        .iter()
        .flat_map(|p| scan_page(p, &baseline_domains))
        .collect();
    all.sort();
    all
}

// ---------------------------------------------------------------------
// URL extraction
// ---------------------------------------------------------------------

fn extract_urls(contents: &str) -> Vec<(String, usize)> {
    let mut urls: Vec<(String, usize)> = Vec::new();

    // pulldown-cmark doesn't expose source offsets per-event without the
    // offset-iterator API, which we use below. Events' byte-offset
    // range lets us compute a 1-based line number.
    let parser = Parser::new_ext(contents, Options::all()).into_offset_iter();
    for (event, range) in parser {
        let line = line_of(contents, range.start);
        match event {
            Event::Start(Tag::Link { dest_url, .. })
            | Event::Start(Tag::Image { dest_url, .. }) => {
                push_url(&mut urls, dest_url, line);
            }
            _ => {}
        }
    }

    // Reference-style link *definitions* and autolinks are handled
    // above by pulldown-cmark (autolinks emit a Link event whose url
    // matches the text). Belt-and-braces: catch bare `scheme:` tokens
    // the author may have pasted that CommonMark doesn't treat as a
    // link (e.g. inside a code span we still want to see the intent).
    for (line_no, raw) in contents.lines().enumerate() {
        for token in
            raw.split(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '(' | ')' | '[' | ']'))
        {
            if let Some(colon) = token.find(':') {
                if colon == 0 {
                    continue;
                }
                let scheme = &token[..colon].to_ascii_lowercase();
                if DANGEROUS_SCHEMES.contains(&scheme.as_str())
                    || EXTERNAL_SCHEMES.contains(&scheme.as_str())
                {
                    // Strip trailing punctuation that's commonly tacked
                    // onto pasted URLs in prose.
                    let trimmed = token.trim_end_matches(|c: char| {
                        matches!(c, '.' | ',' | ';' | ':' | '!' | '?' | '"' | '\'' | ')')
                    });
                    if !trimmed.is_empty() {
                        push_url(&mut urls, CowStr::Borrowed(trimmed), line_no + 1);
                    }
                }
            }
        }
    }

    // De-duplicate while preserving first-seen line.
    urls.sort();
    urls.dedup();
    urls
}

fn push_url<'a>(out: &mut Vec<(String, usize)>, url: CowStr<'a>, line: usize) {
    let s = url.to_string();
    if s.is_empty() {
        return;
    }
    out.push((s, line));
}

fn host_of(url: &str) -> Option<String> {
    match Url::parse(url) {
        Ok(u) => {
            if !EXTERNAL_SCHEMES.contains(&u.scheme()) {
                return None;
            }
            u.host_str().map(|h| {
                let lower = h.to_ascii_lowercase();
                lower
                    .strip_prefix("www.")
                    .map(|s| s.to_owned())
                    .unwrap_or(lower)
            })
        }
        Err(_) => None,
    }
}

fn classify_url(
    url: &str,
    line: usize,
    path: &Path,
    baseline_domains: &BTreeSet<String>,
    out: &mut Vec<Finding>,
) {
    // Dynamic URI detection comes before scheme parsing — template
    // markers break Url::parse in interesting ways and we want the
    // finding either way.
    if looks_dynamic(url) {
        out.push(Finding {
            kind: FindingKind::DynamicUri {
                url: url.to_string(),
            },
            path: path.to_path_buf(),
            line,
            excerpt: truncate(url, 200),
        });
        return;
    }

    // Scheme extraction up front: Url::parse is strict about RFC
    // conformance and will reject `javascript:alert(1)` on missing
    // authority in some builds. Fall back to the bare scheme prefix.
    let scheme = url
        .split_once(':')
        .map(|(s, _)| s.to_ascii_lowercase())
        .unwrap_or_default();

    if DANGEROUS_SCHEMES.contains(&scheme.as_str()) {
        out.push(Finding {
            kind: FindingKind::DangerousScheme {
                scheme,
                url: url.to_string(),
            },
            path: path.to_path_buf(),
            line,
            excerpt: truncate(url, 200),
        });
        return;
    }

    if EXTERNAL_SCHEMES.contains(&scheme.as_str()) {
        if let Some(host) = host_of(url) {
            if !baseline_domains.contains(&host) {
                out.push(Finding {
                    kind: FindingKind::UnseenDomain {
                        domain: host,
                        url: url.to_string(),
                    },
                    path: path.to_path_buf(),
                    line,
                    excerpt: truncate(url, 200),
                });
            }
        }
    }
}

fn looks_dynamic(url: &str) -> bool {
    url.contains("{{")
        || url.contains("}}")
        || url.contains("${")
        || url.contains("<%")
        || url.contains("%>")
}

// ---------------------------------------------------------------------
// Raw HTML extraction
// ---------------------------------------------------------------------

fn extract_raw_html(contents: &str) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let mut seen: BTreeSet<(String, usize)> = BTreeSet::new();

    let parser = Parser::new_ext(contents, Options::all()).into_offset_iter();
    for (event, range) in parser {
        let line = line_of(contents, range.start);
        match event {
            Event::Html(s) | Event::InlineHtml(s) => {
                let trimmed = s.trim();
                if !trimmed.is_empty() && seen.insert((trimmed.to_string(), line)) {
                    out.push((trimmed.to_string(), line));
                }
            }
            _ => {}
        }
    }

    // Fallback: pulldown-cmark's HTML-block grammar rejects some
    // malformed-but-dangerous constructs (`<svg/onload=...>` — slash
    // inside the tag name). Scan the raw source for any line
    // containing a `<known-bad-tag` prefix and fold each match into
    // the finding stream. Keeps the adversarial coverage honest.
    for (idx, raw) in contents.lines().enumerate() {
        for needle in ACTIVE_TAG_NEEDLES {
            if let Some(pos) = raw.to_ascii_lowercase().find(needle) {
                // Extract from `<` through end-of-line or matching `>`.
                let start = raw[..pos].rfind('<').unwrap_or(pos);
                let tail = &raw[start..];
                let end = tail.find('>').map(|e| e + 1).unwrap_or(tail.len());
                let fragment = tail[..end].trim().to_string();
                if fragment.is_empty() {
                    continue;
                }
                let line = idx + 1;
                if seen.insert((fragment.clone(), line)) {
                    out.push((fragment, line));
                }
            }
        }
    }

    out
}

/// Lower-cased `<tag` prefixes whose appearance on any line is treated
/// as raw-HTML evidence even when pulldown-cmark's HTML-block grammar
/// refuses to tokenise the construct. Kept tight: only tags that are
/// either on the sanitiser strip-content list or carry active-content
/// semantics.
const ACTIVE_TAG_NEEDLES: &[&str] = &[
    "<script",
    "<iframe",
    "<object",
    "<embed",
    "<svg",
    "<math",
    "<base ",
    "<meta ",
    "<link ",
    "<style",
    "<form ",
    "<button",
    "<frame",
    "<frameset",
    "<noscript",
    "<xmp",
];

fn classify_html(fragment: &str, line: usize, path: &Path, out: &mut Vec<Finding>) {
    let sanitised = crate::cap::sanitiser::sanitise(fragment);
    let stripped = normalised(fragment) != normalised(&sanitised);
    let tag = first_tag_name(fragment).unwrap_or_else(|| "fragment".to_string());

    if stripped {
        out.push(Finding {
            kind: FindingKind::SanitiserStripped {
                detail: tag.clone(),
                fragment: truncate(fragment, 200),
            },
            path: path.to_path_buf(),
            line,
            excerpt: truncate(fragment, 200),
        });
    }

    // Always surface raw HTML as a reviewer check. Even sanitiser-safe
    // HTML in prose is a review trigger because it's unusual in a
    // typical markdown vault.
    if !sanitised.trim().is_empty() {
        out.push(Finding {
            kind: FindingKind::RawHtml {
                tag,
                fragment: truncate(fragment, 200),
            },
            path: path.to_path_buf(),
            line,
            excerpt: truncate(fragment, 200),
        });
    }
}

fn normalised(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn first_tag_name(fragment: &str) -> Option<String> {
    // Find the first `<tag` — works for open, self-closing, and
    // closing tags; we care about the name only.
    let after_lt = fragment.find('<')?;
    let rest = &fragment[after_lt + 1..];
    let rest = rest.strip_prefix('/').unwrap_or(rest);
    let name_end = rest
        .find(|c: char| c.is_whitespace() || matches!(c, '>' | '/'))
        .unwrap_or(rest.len());
    let name = rest[..name_end].to_ascii_lowercase();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    // Respect UTF-8 boundaries.
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = s[..end].to_string();
    out.push('…');
    out
}

fn line_of(contents: &str, byte_offset: usize) -> usize {
    // 1-based line number of `byte_offset` in `contents`. Used for CLI
    // output, so off-by-one matters to reviewers.
    if byte_offset >= contents.len() {
        return contents.lines().count().max(1);
    }
    contents[..byte_offset]
        .bytes()
        .filter(|&b| b == b'\n')
        .count()
        + 1
}

/// Render findings into a stable, reviewer-readable report. One line
/// per finding, sorted by `(path, line, kind)`. The header reports the
/// total and a per-kind count; the corpus runner asserts against these
/// strings so the surface is kept byte-stable.
pub fn format_report(findings: &[Finding]) -> String {
    let mut out = String::new();
    if findings.is_empty() {
        out.push_str("[zetl cap audit-diff] no findings\n");
        return out;
    }
    out.push_str(&format!(
        "[zetl cap audit-diff] {} finding(s)\n",
        findings.len()
    ));
    for f in findings {
        out.push_str(&format!(
            "  {}:{} {} {}\n",
            f.path.display(),
            f.line,
            f.kind.tag(),
            format_detail(&f.kind),
        ));
    }
    out
}

fn format_detail(kind: &FindingKind) -> String {
    match kind {
        FindingKind::UnseenDomain { domain, url } => {
            format!("{domain} ({})", truncate(url, 80))
        }
        FindingKind::RawHtml { tag, .. } => format!("<{tag}>"),
        FindingKind::SanitiserStripped { detail, .. } => format!("<{detail}>"),
        FindingKind::DangerousScheme { scheme, url } => {
            format!("{scheme}: {}", truncate(url, 80))
        }
        FindingKind::DynamicUri { url } => truncate(url, 80),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page<'a>(path: &'a str, contents: &'a str) -> Page<'a> {
        Page {
            path: Path::new(path),
            contents,
        }
    }

    #[test]
    fn collects_domains_from_links() {
        let p = page(
            "a.md",
            "See [site](https://example.com/x) and <https://other.org>.",
        );
        let d = collect_domains(&[p]);
        assert!(d.contains("example.com"), "{d:?}");
        assert!(d.contains("other.org"), "{d:?}");
    }

    #[test]
    fn flags_unseen_domain() {
        let baseline = [page("a.md", "See [ok](https://example.com/)")];
        let new = [page("b.md", "Read [news](https://evil.test/page)")];
        let findings = scan_diff(&baseline, &new);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].kind.tag(), "unseen-domain");
    }

    #[test]
    fn does_not_flag_known_domain() {
        let baseline = [page("a.md", "See [ok](https://example.com/)")];
        let new = [page("b.md", "Read [more](https://www.example.com/other)")];
        let findings = scan_diff(&baseline, &new);
        assert!(
            findings.iter().all(|f| f.kind.tag() != "unseen-domain"),
            "www. prefix should normalise: {findings:?}",
        );
    }

    #[test]
    fn flags_javascript_scheme() {
        let new = [page("b.md", "Click [here](javascript:alert(1))")];
        let findings = scan_diff(&[], &new);
        let kinds: Vec<_> = findings.iter().map(|f| f.kind.tag()).collect();
        assert!(kinds.contains(&"dangerous-scheme"), "{kinds:?}");
    }

    #[test]
    fn flags_data_uri() {
        let new = [page("b.md", "![x](data:text/html,<script>)")];
        let findings = scan_diff(&[], &new);
        assert!(findings.iter().any(|f| f.kind.tag() == "dangerous-scheme"));
    }

    #[test]
    fn flags_raw_html_block() {
        let new = [page("b.md", "<div class=hi>hi</div>\n")];
        let findings = scan_diff(&[], &new);
        assert!(
            findings.iter().any(|f| f.kind.tag() == "raw-html"),
            "{findings:?}",
        );
    }

    #[test]
    fn flags_script_tag_stripped() {
        let new = [page("b.md", "<script>alert(1)</script>\n")];
        let findings = scan_diff(&[], &new);
        assert!(
            findings
                .iter()
                .any(|f| f.kind.tag() == "sanitiser-stripped"),
            "expected sanitiser-stripped, got {findings:?}",
        );
    }

    #[test]
    fn flags_dynamic_uri() {
        let new = [page("b.md", "See [x](https://{{env.host}}/path)")];
        let findings = scan_diff(&[], &new);
        assert!(
            findings.iter().any(|f| f.kind.tag() == "dynamic-uri"),
            "{findings:?}",
        );
    }

    #[test]
    fn deterministic_order() {
        let new = [
            page("z.md", "[a](https://a.test)"),
            page("a.md", "[b](https://b.test)"),
        ];
        let a = scan_diff(&[], &new);
        let b = scan_diff(&[], &new);
        assert_eq!(a, b);
    }

    #[test]
    fn empty_input_no_findings() {
        let findings: Vec<Finding> = scan_diff(&[], &[]);
        assert!(findings.is_empty());
    }
}
