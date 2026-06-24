//! SPEC-048 — integration tests for template components, templated static pages,
//! design tokens, deduped component CSS, and addressed transclusion.
//!
//! Each test builds a fixture vault via `zetl build` and asserts on the emitted output,
//! mirroring the happy paths (HP2–HP6) and the negative cases in §9.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// Write a file under `dir`, creating parent directories.
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

/// A nav-header component requiring only the site tier (cross-context reusable).
fn write_nav_header(vault: &Path) {
    write(
        vault,
        ".zetl/components/nav-header/nav-header.html",
        "<nav data-z=\"{{ _name }}\" class=\"nav-header\"><a href=\"{{ site.root_path }}\">{{ site.name }}</a>{{ caller() }}</nav>",
    );
    write(
        vault,
        ".zetl/components/nav-header/nav-header.toml",
        "name = \"nav-header\"\nrequires = [\"site\"]\n[props]\nactive = { type = \"string\", default = \"\" }\n",
    );
    write(
        vault,
        ".zetl/components/nav-header/nav-header.css",
        ".nav-header { display: flex; }",
    );
}

#[test]
fn hp3_component_on_static_page_with_site_context() {
    let dir = TempDir::new().unwrap();
    let v = dir.path();
    write_nav_header(v);
    write(v, "home.md", "# Home\n\nHello.\n");
    write(
        v,
        ".zetl/static/about.html.jinja",
        "<!doctype html><html><body>{% component \"nav-header\" active=\"about\" %}<span>x</span>{% endcomponent %}</body></html>",
    );

    let (ok, log) = build(v);
    assert!(ok, "build failed: {log}");

    let html = fs::read_to_string(v.join("dist/about/index.html")).expect("about/index.html");
    // component rendered with the data-z marker, site name, depth-correct root_path
    assert!(
        html.contains("data-z=\"nav-header\""),
        "marker missing: {html}"
    );
    assert!(html.contains("class=\"nav-header\""));
    assert!(
        html.contains("href=\"../\""),
        "depth-correct root_path: {html}"
    );
    assert!(
        html.contains("<span>x</span>"),
        "default slot (caller) missing"
    );
    // data-z appears exactly once (one component root)
    assert_eq!(html.matches("data-z=\"nav-header\"").count(), 1);
}

#[test]
fn hp4_tokens_single_source_merge() {
    let dir = TempDir::new().unwrap();
    let v = dir.path();
    write(v, "home.md", "# Home\n");
    // theme-tier tokens (two keys) then vault override of one key
    write(
        v,
        ".zetl/themes/custom/tokens.toml",
        "moss = \"#5B8C5A\"\nwarm = \"#d98c4a\"\n",
    );
    write(v, ".zetl/tokens.toml", "moss = \"#4da6a6\"\n");

    let output = cargo_bin_cmd!("zetl")
        .current_dir(v)
        .args(["build", "-o", "dist", "--theme", "custom"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let css = fs::read_to_string(v.join("dist/_static/tokens.css")).expect("tokens.css");
    // vault overrode moss key-by-key; warm inherited from theme
    assert!(css.contains("--moss: #4da6a6;"), "merged moss: {css}");
    assert!(css.contains("--warm: #d98c4a;"), "inherited warm: {css}");
    // exactly one :root and exactly one moss declaration
    assert_eq!(css.matches("--moss:").count(), 1);
}

#[test]
fn hp6_transclude_named_section() {
    let dir = TempDir::new().unwrap();
    let v = dir.path();
    write(
        v,
        "handbook.md",
        "# Handbook\n\n## Mission\n\nRespect the user.\n\n## Other\n\nNope.\n",
    );
    write(
        v,
        ".zetl/static/about.html.jinja",
        "<!doctype html><html><body><main>{{ transclude(\"handbook#Mission\") }}</main></body></html>",
    );

    let (ok, log) = build(v);
    assert!(ok, "build failed: {log}");
    let html = fs::read_to_string(v.join("dist/about/index.html")).unwrap();
    assert!(
        html.contains("Respect the user."),
        "transcluded section: {html}"
    );
    assert!(
        !html.contains("Nope."),
        "only the addressed section is pulled"
    );
}

#[test]
fn req4809_component_css_deduped_and_emitted() {
    let dir = TempDir::new().unwrap();
    let v = dir.path();
    write_nav_header(v);
    write(v, "home.md", "# Home\n");
    write(
        v,
        ".zetl/static/a.html.jinja",
        "<html><body>{% component \"nav-header\" /%}{% component \"nav-header\" /%}</body></html>",
    );
    let (ok, log) = build(v);
    assert!(ok, "build failed: {log}");
    let css = fs::read_to_string(v.join("dist/_static/components.css")).expect("components.css");
    // used twice, emitted once
    assert_eq!(css.matches(".nav-header { display: flex; }").count(), 1);
}

#[test]
fn req4813_backward_compatible_default() {
    let dir = TempDir::new().unwrap();
    let v = dir.path();
    write(v, "home.md", "# Home\n");
    // a plain static file (no render marker) must be copied verbatim
    write(v, ".zetl/static/raw.html", "<p>{{ not_rendered }}</p>");

    let (ok, log) = build(v);
    assert!(ok, "build failed: {log}");
    // no opt-in → no token or component stylesheet emitted
    assert!(!v.join("dist/_static/tokens.css").exists(), "no tokens.css");
    assert!(
        !v.join("dist/_static/components.css").exists(),
        "no components.css"
    );
    // plain static copied verbatim (braces untouched)
    let raw = fs::read_to_string(v.join("dist/_static/raw.html")).expect("raw.html copied");
    assert_eq!(raw, "<p>{{ not_rendered }}</p>");
}

#[test]
fn threat_g_page_requiring_component_rejected_on_static_page() {
    let dir = TempDir::new().unwrap();
    let v = dir.path();
    write(v, "home.md", "# Home\n");
    write(
        v,
        ".zetl/components/backlinks/backlinks.html",
        "<div data-z=\"{{ _name }}\">{{ page.title }}</div>",
    );
    write(
        v,
        ".zetl/components/backlinks/backlinks.toml",
        "name = \"backlinks\"\nrequires = [\"page\"]\n",
    );
    write(
        v,
        ".zetl/static/about.html.jinja",
        "<html><body>{% component \"backlinks\" /%}</body></html>",
    );
    let (ok, log) = build(v);
    assert!(
        !ok,
        "build should fail when a page-tier component is used on a static page"
    );
    assert!(
        log.contains("component-context-unavailable"),
        "expected context-unavailable error, got: {log}"
    );
}

#[test]
fn req4807_component_cycle_rejected_at_compile() {
    let dir = TempDir::new().unwrap();
    let v = dir.path();
    write(v, "home.md", "# Home\n");
    // a -> b -> a mutual cycle
    write(
        v,
        ".zetl/components/a/a.html",
        "<div>{% component \"b\" /%}</div>",
    );
    write(
        v,
        ".zetl/components/a/a.toml",
        "name = \"a\"\nrequires = [\"site\"]\n",
    );
    write(
        v,
        ".zetl/components/b/b.html",
        "<div>{% component \"a\" /%}</div>",
    );
    write(
        v,
        ".zetl/components/b/b.toml",
        "name = \"b\"\nrequires = [\"site\"]\n",
    );
    let (ok, log) = build(v);
    assert!(!ok, "build should fail on a component cycle");
    assert!(
        log.contains("component-cycle"),
        "expected component-cycle error, got: {log}"
    );
}

#[test]
fn serve_build_parity_static_page_and_css() {
    use zetl::web::build::{compute_components_css, compute_tokens_css};
    use zetl::web::engine::TemplateEngine;

    let dir = TempDir::new().unwrap();
    let v = dir.path();
    write_nav_header(v);
    write(v, ".zetl/tokens.toml", "moss = \"#5B8C5A\"\n");
    write(
        v,
        "handbook.md",
        "# Handbook\n\n## Mission\n\nRespect the user.\n",
    );
    let src = "<html><head><link rel=\"stylesheet\" href=\"{{ site.tokens_url }}\"></head><body>\
{% component \"nav-header\" /%}<main>{{ transclude(\"handbook#Mission\") }}</main></body></html>";

    let engine = TemplateEngine::new(v, "default", false, false);
    let build_html = engine
        .render_static_page("v", src, "about", "build")
        .unwrap();
    let serve_html = engine
        .render_static_page("v", src, "about", "serve")
        .unwrap();

    // Both modes render the same component + transclusion (parity of behaviour).
    for html in [&build_html, &serve_html] {
        assert!(html.contains("data-z=\"nav-header\""), "component: {html}");
        assert!(html.contains("Respect the user."), "transclusion: {html}");
    }
    // The ONLY intended difference is the link base: build relative, serve absolute.
    assert!(
        build_html.contains("../_static/tokens.css"),
        "build root_path: {build_html}"
    );
    assert!(
        serve_html.contains("/_static/tokens.css"),
        "serve root_path: {serve_html}"
    );

    // tokens.css / components.css are mode-independent (shared helpers).
    assert_eq!(
        compute_tokens_css(v, "default").unwrap().unwrap(),
        ":root {\n  --moss: #5B8C5A;\n}\n"
    );
    assert!(compute_components_css(v, "default")
        .unwrap()
        .unwrap()
        .contains(".nav-header"));
}

#[test]
fn req4818_unresolved_transclude_fails_closed() {
    let dir = TempDir::new().unwrap();
    let v = dir.path();
    write(v, "home.md", "# Home\n");
    write(
        v,
        ".zetl/static/about.html.jinja",
        "<html><body>{{ transclude(\"ghost-page\") }}</body></html>",
    );
    let (ok, log) = build(v);
    assert!(!ok, "build should fail on unresolved transclude target");
    assert!(
        log.contains("transclude-target-unresolved") || log.contains("unresolved"),
        "expected unresolved error, got: {log}"
    );
}
