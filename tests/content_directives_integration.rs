//! SPEC-049 — integration tests for content-author components & directives.
//!
//! Each test builds a fixture vault via `zetl build` and asserts on the emitted output,
//! covering TEST-4901..4912: recognition, default-deny invocability, prop recognition +
//! the CON-4904 context lint, isolated body sanitisation, the per-provenance barrier,
//! diagnostics, and the byte-identical backward-compatible default.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn write(dir: &Path, rel: &str, content: &str) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, content).unwrap();
}

/// Run `zetl build -o dist` in `vault`; return (success, stdout+stderr).
fn build(vault: &Path) -> (bool, String) {
    let output = cargo_bin_cmd!("zetl")
        .current_dir(vault)
        .args(["build", "-o", "dist"])
        .output()
        .expect("run zetl build");
    let mut log = String::from_utf8_lossy(&output.stdout).to_string();
    log.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.success(), log)
}

/// The page output for `name.md` lands at `dist/<name>/index.html`.
fn page_html(vault: &Path, name: &str) -> String {
    fs::read_to_string(vault.join(format!("dist/{name}/index.html")))
        .unwrap_or_else(|_| panic!("missing dist/{name}/index.html"))
}

/// A content-invocable callout component (safe template: props only in TEXT/dq-attr).
fn write_callout(vault: &Path) {
    write(
        vault,
        ".zetl/components/callout/callout.html",
        "<aside data-z=\"{{ _name }}\" class=\"callout callout-{{ props.tone }}\">{{ caller() }}</aside>",
    );
    write(
        vault,
        ".zetl/components/callout/callout.toml",
        r#"name = "callout"
requires = ["site"]
content_invocable = true
content_props = ["tone"]
[props]
tone = { type = "string", default = "info", enum = ["info", "warning"] }
"#,
    );
}

#[test]
fn hp1_directive_expands_with_sanitised_body() {
    let dir = TempDir::new().unwrap();
    let v = dir.path();
    write_callout(v);
    write(
        v,
        "post.md",
        ":::callout{tone=warning}\nHeads up **bold** and a <script>alert(1)</script> tag.\n:::\n",
    );
    let (ok, log) = build(v);
    assert!(ok, "build failed: {log}");
    let html = page_html(v, "post");
    assert!(html.contains("data-z=\"callout\""), "component not expanded: {html}");
    assert!(html.contains("callout-warning"), "tone prop not bound: {html}");
    assert!(html.contains("<strong>bold</strong>"), "body markdown not rendered");
    // the script element is stripped from the rendered body (sanitised in isolation)
    assert!(!html.contains("<script>alert(1)"), "script survived sanitiser: {html}");
}

#[test]
fn hp2_non_invocable_directive_fails_closed_inert() {
    let dir = TempDir::new().unwrap();
    let v = dir.path();
    write_callout(v);
    // `raw-html` is not a component at all → unknown for content → inert + diagnostic.
    write(
        v,
        "post.md",
        ":::raw-html{}\nplain body text <script>alert(2)</script>\n:::\n",
    );
    let (ok, log) = build(v);
    assert!(ok, "build should succeed (inert, not fatal): {log}");
    let html = page_html(v, "post");
    assert!(!html.contains("data-z=\"raw-html\""), "must not expand unknown component");
    assert!(html.contains("plain body text"), "inert body preserved");
    assert!(!html.contains("<script>alert(2)"), "inert body still sanitised");
}

#[test]
fn test4903_default_deny_non_invocable_component() {
    let dir = TempDir::new().unwrap();
    let v = dir.path();
    // a component that exists but is NOT content_invocable
    write(
        v,
        ".zetl/components/secret/secret.html",
        "<div data-z=\"{{ _name }}\">secret {{ props.x }}</div>",
    );
    write(
        v,
        ".zetl/components/secret/secret.toml",
        "name = \"secret\"\n[props]\nx = { type = \"string\", default = \"\" }\n",
    );
    // also need a content-invocable component so the expander activates at all
    write_callout(v);
    write(v, "post.md", ":::secret{x=1}\nbody\n:::\n");
    let (ok, _log) = build(v);
    assert!(ok);
    let html = page_html(v, "post");
    assert!(!html.contains("data-z=\"secret\""), "non-invocable component must not expand");
    assert!(html.contains("body"), "body preserved inert");
}

#[test]
fn test4904_prop_enum_violation_inert_with_diagnostic() {
    let dir = TempDir::new().unwrap();
    let v = dir.path();
    write_callout(v);
    write(v, "post.md", ":::callout{tone=danger}\nbody\n:::\n");
    let (ok, log) = build(v);
    assert!(ok, "prop error is per-directive inert, not fatal: {log}");
    assert!(log.contains("content-prop-enum"), "diagnostic surfaced: {log}");
    let html = page_html(v, "post");
    assert!(!html.contains("data-z=\"callout\""), "no component on prop error");
}

#[test]
fn test4904_lint_rejects_unsafe_template_at_build() {
    let dir = TempDir::new().unwrap();
    let v = dir.path();
    // A content-invocable component whose template places a content prop in a JS context
    // → CON-4904 fatal build error.
    write(
        v,
        ".zetl/components/bad/bad.html",
        "<button onclick=\"{{ props.x }}\">x</button>",
    );
    write(
        v,
        ".zetl/components/bad/bad.toml",
        r#"name = "bad"
content_invocable = true
content_props = ["x"]
[props]
x = { type = "string", default = "" }
"#,
    );
    write(v, "post.md", "# hi\n");
    let (ok, log) = build(v);
    assert!(!ok, "build must fail on an unsafe content-invocable template");
    assert!(
        log.contains("content-context-unsafe") || log.contains("content-component error"),
        "CON-4904 lint error surfaced: {log}"
    );
}

#[test]
fn test4905_isolated_body_sanitisation_keeps_trusted_template_elements() {
    let dir = TempDir::new().unwrap();
    let v = dir.path();
    // The trusted template legitimately uses <button> (forbidden in the body allowlist);
    // it must NOT be stripped, while a <button> in the author body IS stripped.
    write(
        v,
        ".zetl/components/widget/widget.html",
        "<div data-z=\"{{ _name }}\"><button>trusted</button>{{ caller() }}</div>",
    );
    write(
        v,
        ".zetl/components/widget/widget.toml",
        "name = \"widget\"\ncontent_invocable = true\ncontent_props = []\n[props]\n",
    );
    write(
        v,
        "post.md",
        ":::widget{}\nauthor <button>evil</button> text\n:::\n",
    );
    let (ok, log) = build(v);
    assert!(ok, "build failed: {log}");
    let html = page_html(v, "post");
    assert!(html.contains("<button>trusted</button>"), "trusted template button kept: {html}");
    assert!(!html.contains("<button>evil</button>"), "author-body button stripped: {html}");
}

#[test]
fn test4906_nested_directive_trusted_fragment_preserved() {
    let dir = TempDir::new().unwrap();
    let v = dir.path();
    write_callout(v);
    // outer callout contains an inner callout; the inner's expanded <aside> must survive
    // the outer body sanitiser (per-provenance barrier).
    write(
        v,
        "post.md",
        "::::callout{tone=info}\nouter intro\n:::callout{tone=warning}\ninner body\n:::\n::::\n",
    );
    let (ok, log) = build(v);
    assert!(ok, "build failed: {log}");
    let html = page_html(v, "post");
    assert!(html.contains("callout-info"), "outer expanded");
    assert!(html.contains("callout-warning"), "inner expanded + preserved: {html}");
    assert!(html.contains("inner body"));
}

#[test]
fn test4907_restricted_context_no_transclude() {
    let dir = TempDir::new().unwrap();
    let v = dir.path();
    // A content-invocable component template that tries to use transclude() — the
    // restricted content context omits it, so the render fails closed (inert), never
    // leaking vault content.
    write(
        v,
        ".zetl/components/leaky/leaky.html",
        "<div data-z=\"{{ _name }}\">{{ transclude(\"secret-page\") }}</div>",
    );
    write(
        v,
        ".zetl/components/leaky/leaky.toml",
        "name = \"leaky\"\ncontent_invocable = true\ncontent_props = []\n[props]\n",
    );
    write(v, "secret-page.md", "---\ndraft: true\n---\nTOP SECRET\n");
    write(v, "post.md", ":::leaky{}\nx\n:::\n");
    let (ok, _log) = build(v);
    // build succeeds (per-directive inert on render error) but must not leak the secret.
    assert!(ok);
    let html = page_html(v, "post");
    assert!(!html.contains("TOP SECRET"), "transclude must be unavailable in content: {html}");
}

#[test]
fn test4912_backward_compatible_default_no_invocable() {
    let dir = TempDir::new().unwrap();
    let v = dir.path();
    // No content-invocable component → `:::name` is literal text (byte-identical default).
    write(v, "post.md", ":::callout{tone=warning}\nbody text\n:::\n");
    let (ok, log) = build(v);
    assert!(ok, "build failed: {log}");
    let html = page_html(v, "post");
    // the directive source survives as literal markdown text (no expansion)
    assert!(html.contains(":::callout"), "directive left literal when no invocable component: {html}");
}

#[test]
fn test4912_reserved_manifest_keys_build_under_plain_build() {
    let dir = TempDir::new().unwrap();
    let v = dir.path();
    // A manifest carrying content_invocable/content_props must build even though we are
    // not exercising any directive — reserved-and-accepted (CON-4903 forward-compat).
    write_callout(v);
    write(v, "plain.md", "# Plain page\n\nNo directives here.\n");
    let (ok, log) = build(v);
    assert!(ok, "manifest with content keys must build: {log}");
    let html = page_html(v, "plain");
    assert!(html.contains("Plain page"));
}
