//! SPEC-050 — integration tests for component islands & messaging (build-side).
//!
//! Builds fixture vaults via `zetl build` and asserts on emitted island assets, page
//! hydration markers, CSP, the wiring audit, the SPEC-049→050 content-island handoff
//! (REQ-4910), and the byte-identical backward-compatible default (REQ-5012).

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn write(dir: &Path, rel: &str, content: &str) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, content).unwrap();
}

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

fn page(vault: &Path, name: &str) -> String {
    fs::read_to_string(vault.join(format!("dist/{name}/index.html")))
        .unwrap_or_else(|_| panic!("missing dist/{name}/index.html"))
}

/// A content-invocable `poll` island (Worker, paints, publishes a content: enum topic).
fn write_poll(v: &Path) {
    write(
        v,
        ".zetl/components/poll/poll.html",
        "<div data-z=\"{{ _name }}\" class=\"poll\"><p>{{ props.question }}</p>{{ caller() }}</div>",
    );
    write(
        v,
        ".zetl/components/poll/poll.toml",
        r#"name = "poll"
requires = ["site"]
content_invocable = true
content_props = ["question"]
publishes = ["content:vote"]
render = "worker"
paints = true
hydrate = "visible"
[props]
question = { type = "string", default = "?" }
[island.topics."content:vote"]
type = "enum(\"yes\",\"no\")"
"#,
    );
    write(v, ".zetl/components/poll/poll.js", "self.onmessage=function(){};\n");
}

#[test]
fn req5001_emits_island_assets_once() {
    let dir = TempDir::new().unwrap();
    let v = dir.path();
    write_poll(v);
    write(v, "p.md", ":::poll{question=\"Ship it?\"}\nvote\n:::\n");
    let (ok, log) = build(v);
    assert!(ok, "build failed: {log}");
    assert!(v.join("dist/_static/zetl-islands.js").is_file(), "bus runtime emitted");
    assert!(v.join("dist/_static/islands/poll.js").is_file(), "worker script emitted");
    assert!(v.join("dist/_static/island-audit.json").is_file(), "wiring audit emitted");
}

#[test]
fn req4910_content_island_handoff_markers() {
    let dir = TempDir::new().unwrap();
    let v = dir.path();
    write_poll(v);
    write(v, "p.md", ":::poll{question=\"Ship it?\"}\nvote\n:::\n");
    let (ok, log) = build(v);
    assert!(ok, "build failed: {log}");
    let html = page(v, "p");
    // SPEC-049 expanded the directive, SPEC-050 stamped island markers on the data-z node
    assert!(html.contains("data-z=\"poll\""), "directive expanded");
    assert!(html.contains("data-island=\"poll\""), "island marker stamped");
    assert!(html.contains("data-island-worker="), "worker URL present");
    assert!(html.contains("data-island-paints=\"true\""), "paints grant present");
    assert!(html.contains("content:vote"), "grants/types present");
    assert!(html.contains("data-island-hydrate=\"visible\""), "hydrate strategy present");
    assert!(html.contains("zetl-islands.js"), "runtime bootstrap injected");
}

#[test]
fn req5027_csp_emitted_for_content_island_page() {
    let dir = TempDir::new().unwrap();
    let v = dir.path();
    write_poll(v);
    write(v, "p.md", ":::poll{}\nvote\n:::\n");
    let (ok, log) = build(v);
    assert!(ok, "build failed: {log}");
    let html = page(v, "p");
    assert!(html.contains("Content-Security-Policy"), "CSP meta injected");
    assert!(html.contains("default-src 'none'"), "default-deny baseline");
    assert!(html.contains("connect-src 'none'"), "egress denied by default");
    assert!(v.join("dist/_headers.csp").is_file(), "headers artifact emitted");
}

#[test]
fn req5027_operator_csp_widening_applied() {
    let dir = TempDir::new().unwrap();
    let v = dir.path();
    write_poll(v);
    write(
        v,
        ".zetl/config.toml",
        "[security.csp]\nconnect-src = [\"https://api.example.com\"]\n",
    );
    write(v, "p.md", ":::poll{}\nvote\n:::\n");
    let (ok, log) = build(v);
    assert!(ok, "build failed: {log}");
    let html = page(v, "p");
    assert!(
        html.contains("connect-src 'self' https://api.example.com"),
        "operator widening applied to CSP"
    );
}

#[test]
fn req5012_backward_compatible_no_island() {
    let dir = TempDir::new().unwrap();
    let v = dir.path();
    // a plain component (no island fields), used on a page → no island assets at all
    write(
        v,
        ".zetl/components/card/card.html",
        "<div data-z=\"{{ _name }}\">{{ caller() }}</div>",
    );
    write(
        v,
        ".zetl/components/card/card.toml",
        "name = \"card\"\nrequires = [\"site\"]\n[props]\n",
    );
    write(v, "p.md", "# plain\n\nNo islands.\n");
    let (ok, log) = build(v);
    assert!(ok, "build failed: {log}");
    assert!(!v.join("dist/_static/zetl-islands.js").exists(), "no bus runtime when no island");
    let html = page(v, "p");
    assert!(!html.contains("data-island="), "no island markers");
    assert!(!html.contains("zetl-islands.js"), "no bootstrap");
}

#[test]
fn malformed_island_manifest_fails_build() {
    let dir = TempDir::new().unwrap();
    let v = dir.path();
    write(
        v,
        ".zetl/components/bad/bad.html",
        "<div data-z=\"{{ _name }}\"></div>",
    );
    // publishes a topic with no [island.topics] declaration → island-topic-malformed
    write(
        v,
        ".zetl/components/bad/bad.toml",
        "name = \"bad\"\npublishes = [\"theme\"]\n[props]\n",
    );
    write(v, "p.md", "# x\n");
    let (ok, log) = build(v);
    assert!(!ok, "malformed island manifest must fail the build");
    assert!(log.contains("island-topic-malformed") || log.contains("island error"), "log: {log}");
}

#[test]
fn determinism_island_assets_byte_identical() {
    let dir = TempDir::new().unwrap();
    let v = dir.path();
    write_poll(v);
    write(v, "p.md", ":::poll{}\nvote\n:::\n");
    let (ok1, _) = build(v);
    assert!(ok1);
    // NFR-5003: the emitted island asset SET is byte-identical across builds.
    let read = |p: &str| fs::read(v.join(p)).unwrap();
    let a1 = read("dist/_static/island-audit.json");
    let r1 = read("dist/_static/zetl-islands.js");
    let w1 = read("dist/_static/islands/poll.js");
    let h1 = read("dist/_headers.csp");
    let markers1 = page(v, "p").contains("data-island=\"poll\"");
    // rebuild
    let (ok2, _) = build(v);
    assert!(ok2);
    assert_eq!(a1, read("dist/_static/island-audit.json"), "audit byte-identical (NFR-5003)");
    assert_eq!(r1, read("dist/_static/zetl-islands.js"), "runtime byte-identical");
    assert_eq!(w1, read("dist/_static/islands/poll.js"), "worker byte-identical");
    assert_eq!(h1, read("dist/_headers.csp"), "CSP headers byte-identical");
    assert!(markers1 && page(v, "p").contains("data-island=\"poll\""), "markers stable");
}
