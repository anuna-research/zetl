//! SPEC-050 REQ-5027 — Content-Security-Policy computation & emission.
//!
//! The per-page effective CSP is a **fail-closed union**: a default-deny baseline ∪ the
//! operator's `[security.csp]` widenings ([`super::theme::CspConfig`]). Absence of
//! `[security.csp]` yields the baseline (NOT "no CSP"). The policy is emitted two ways,
//! byte-identically derived: a `<meta http-equiv>` tag (the authoritative form on static
//! / `file://`) and a served-deploy headers artifact carrying the directives `<meta>`
//! cannot set (`frame-ancestors`, `report-uri`). Mandatory for content-island pages.

use super::theme::CspConfig;
use std::collections::BTreeMap;

/// Compute the effective per-page CSP policy string for a page that hosts ≥1 content
/// island. `script_hashes` are the base64 `sha256-…` digests of the inline island
/// bootstrap + pre-paint scripts (REQ-5018/5019) admitted by hash, never `unsafe-inline`.
pub fn content_island_policy(
    csp: &CspConfig,
    script_hashes: &[String],
    style_hashes: &[String],
) -> String {
    // default-deny baseline (REQ-5027)
    let mut directives: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    directives.insert("default-src", vec!["'none'".into()]);
    let mut script = vec!["'self'".into()];
    for h in script_hashes {
        script.push(format!("'sha256-{h}'"));
    }
    directives.insert("script-src", script);
    directives.insert("worker-src", vec!["'self'".into(), "blob:".into()]);
    directives.insert("connect-src", vec!["'none'".into()]);
    directives.insert("img-src", vec!["'self'".into()]);
    directives.insert("media-src", vec!["'self'".into()]);
    directives.insert("font-src", vec!["'self'".into()]);
    // Inline `<style>` blocks shipped by the theme are admitted by hash (REQ-5027 — never
    // `unsafe-inline`); external stylesheets are covered by `'self'`.
    let mut style = vec!["'self'".into()];
    for h in style_hashes {
        style.push(format!("'sha256-{h}'"));
    }
    directives.insert("style-src", style);
    directives.insert("base-uri", vec!["'none'".into()]);
    directives.insert("form-action", vec!["'none'".into()]);

    // union the operator widenings (a directive set to ['none'] is replaced by the
    // self+hosts form when widened; otherwise hosts are appended).
    for (dir, hosts) in &csp.directives {
        if hosts.is_empty() {
            continue;
        }
        let entry = directives.entry(dir.as_str()).or_insert_with(|| vec!["'self'".into()]);
        // a baseline of 'none' is dropped when the operator widens the directive
        if entry == &vec!["'none'".to_string()] {
            *entry = vec!["'self'".into()];
        }
        for h in hosts {
            if !entry.contains(h) {
                entry.push(h.clone());
            }
        }
    }

    serialise(&directives)
}

/// Deterministic serialisation: directives in a fixed canonical order.
fn serialise(directives: &BTreeMap<&str, Vec<String>>) -> String {
    const ORDER: &[&str] = &[
        "default-src",
        "script-src",
        "worker-src",
        "connect-src",
        "img-src",
        "media-src",
        "font-src",
        "style-src",
        "base-uri",
        "form-action",
    ];
    let mut parts = Vec::new();
    for d in ORDER {
        if let Some(vals) = directives.get(d) {
            parts.push(format!("{d} {}", vals.join(" ")));
        }
    }
    // any extra (non-ORDER) directives, sorted, for forward-compat
    for (d, vals) in directives {
        if !ORDER.contains(d) {
            parts.push(format!("{d} {}", vals.join(" ")));
        }
    }
    parts.join("; ")
}

/// The `<meta http-equiv>` tag, intended as the **first** `<head>` child (REQ-5027).
pub fn meta_tag(policy: &str) -> String {
    format!(
        "<meta http-equiv=\"Content-Security-Policy\" content=\"{}\">",
        html_attr_escape(policy)
    )
}

/// The served-deploy headers artifact body. It carries ONLY `frame-ancestors` — the
/// directive `<meta>` cannot express. Script/style/default enforcement (with per-page
/// inline-asset hashes) lives in each page's authoritative `<meta>` CSP; duplicating it in
/// a single build-wide header would block the theme's own inline assets, and a header
/// `connect-src 'none'` would override the operator's per-page declared egress. So the
/// header stays minimal and never conflicts with the meta (Codex P2). `_csp` is accepted
/// for signature stability / future report-uri support.
pub fn headers_artifact(_csp: &CspConfig) -> String {
    "Content-Security-Policy: frame-ancestors 'none'\n".to_string()
}

fn html_attr_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_is_default_deny() {
        let p = content_island_policy(&CspConfig::default(), &[], &[]);
        assert!(p.contains("default-src 'none'"));
        assert!(p.contains("connect-src 'none'"));
        assert!(p.contains("worker-src 'self' blob:"));
        assert!(p.contains("script-src 'self'"));
        assert!(p.contains("base-uri 'none'"));
        assert!(p.contains("form-action 'none'"));
    }

    #[test]
    fn script_hashes_included() {
        let p = content_island_policy(&CspConfig::default(), &["abc123".into()], &[]);
        assert!(p.contains("'sha256-abc123'"));
    }

    #[test]
    fn widening_unions_connect_src() {
        let mut csp = CspConfig::default();
        csp.directives.insert("connect-src".into(), vec!["https://api.example.com".into()]);
        let p = content_island_policy(&csp, &[], &[]);
        assert!(p.contains("connect-src 'self' https://api.example.com"), "{p}");
        assert!(!p.contains("connect-src 'none'"));
    }

    #[test]
    fn deterministic() {
        let mut csp = CspConfig::default();
        csp.directives.insert("img-src".into(), vec!["https://cdn.example.com".into()]);
        let a = content_island_policy(&csp, &["h".into()], &[]);
        let b = content_island_policy(&csp, &["h".into()], &[]);
        assert_eq!(a, b);
    }

    #[test]
    fn meta_carries_policy_and_header_carries_frame_ancestors() {
        let p = content_island_policy(&CspConfig::default(), &[], &[]);
        assert!(meta_tag(&p).contains(&html_attr_escape(&p)));
        // the header is minimal (frame-ancestors only) so it cannot conflict with the
        // per-page meta's inline-asset hashes
        let h = headers_artifact(&CspConfig::default());
        assert!(h.contains("frame-ancestors 'none'"));
        assert!(!h.contains("script-src"));
    }

    #[test]
    fn style_hashes_included() {
        let p = content_island_policy(&CspConfig::default(), &[], &["styh".into()]);
        assert!(p.contains("style-src 'self' 'sha256-styh'"), "{p}");
    }
}
