//! Integration tests for issues #72 and #73.
//!
//! #73 — `zetl build` must emit a `404.html` so static hosts (Cloudflare
//! Pages et al.) don't switch to SPA-fallback mode and answer every unknown
//! path with 200 + the homepage document, and every document rendered
//! through base.html must self-identify via `<body data-slug>` so
//! `spa.js` can verify a fetched document against the URL it asked for.
//!
//! #72 — the default theme's shell.css must carry dark-scheme values for
//! the predicate-chip colours and the `--zetl-graph-*` / `--zetl-shell-*`
//! tokens so those components stay legible under OS-enforced
//! `prefers-color-scheme: dark`.

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use tempfile::TempDir;

fn zetl_cmd(vault: &Path) -> Command {
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("zetl");
    cmd.arg("-d").arg(vault.as_os_str());
    cmd.arg("--no-cache");
    cmd
}

fn write_file(root: &Path, relative: &str, content: &str) {
    let full = root.join(relative);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).expect("create parent dirs");
    }
    fs::write(&full, content).expect("write test file");
}

/// Build a small vault (a root page + a page inside a folder, so a folder
/// index is emitted too) and return the dist directory.
fn build_vault(dir: &TempDir) -> std::path::PathBuf {
    write_file(
        dir.path(),
        "Hello.md",
        "# Hello\n\nRoot page linking [[notes/World]].\n",
    );
    write_file(dir.path(), "notes/World.md", "# World\n\nNested page.\n");

    let out_dir = dir.path().join("dist");
    let output = zetl_cmd(dir.path())
        .arg("build")
        .arg("-o")
        .arg(out_dir.as_os_str())
        .output()
        .expect("failed to execute zetl build");
    assert!(
        output.status.success(),
        "zetl build should succeed.\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    out_dir
}

/// Extract the `data-slug` attribute value from the document's `<body>` tag
/// (the marker may also appear on backlink `<li>`s, so scope to `<body`),
/// with minijinja's `&#x2f;` attribute-escaping of `/` decoded — browsers
/// decode entities at parse time, so this is the value spa.js sees.
fn body_data_slug(html: &str) -> String {
    let body_tag = html
        .split("<body")
        .nth(1)
        .and_then(|rest| rest.split('>').next())
        .expect("document should have a <body> tag");
    body_tag
        .split(r#"data-slug=""#)
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("<body> should carry a data-slug attribute")
        .replace("&#x2f;", "/")
}

/// #73: the build emits a root-level 404.html rendered with absolute asset
/// URLs (it can be served at any path depth) and an empty data-slug claim.
#[test]
fn build_emits_404_html() {
    let dir = TempDir::new().expect("create temp dir");
    let out = build_vault(&dir);

    let html = fs::read_to_string(out.join("404.html")).expect("dist/404.html should exist");
    assert!(
        html.contains("Page not found"),
        "404.html should carry the not-found message"
    );
    // minijinja escapes "/" as &#x2f; inside attributes; browsers decode it.
    assert!(
        html.contains(r#"href="/_static/shell.css""#)
            || html.contains(r#"href="&#x2f;_static/shell.css""#),
        "404.html must reference assets by absolute URL — it is served at arbitrary depths"
    );
    assert_eq!(
        body_data_slug(&html),
        "",
        "404.html claims the empty slug on <body>"
    );
}

/// #73: every page rendered through base.html self-identifies via
/// `<body data-slug="…">` — the marker spa.js verifies before swapping.
#[test]
fn built_documents_carry_body_data_slug() {
    let dir = TempDir::new().expect("create temp dir");
    let out = build_vault(&dir);

    let page = fs::read_to_string(out.join("hello/index.html")).expect("hello page");
    assert_eq!(
        body_data_slug(&page),
        "hello",
        "page document should claim its own slug on <body>"
    );

    let nested =
        fs::read_to_string(out.join("notes/world/index.html")).expect("nested page");
    assert_eq!(
        body_data_slug(&nested),
        "notes/world",
        "nested page document should claim its full slug"
    );

    // Folder indexes claim the folder slug (not ""): an empty claim would be
    // indistinguishable from a host's homepage 404-fallback in spa.js.
    let folder = fs::read_to_string(out.join("notes/index.html")).expect("folder index");
    assert_eq!(
        body_data_slug(&folder),
        "notes",
        "folder index should claim the folder slug on <body>"
    );

    let index = fs::read_to_string(out.join("index.html")).expect("vault index");
    assert_eq!(
        body_data_slug(&index),
        "",
        "vault index claims the empty slug on <body>"
    );
}

/// #72: shell.css defines dark-scheme values for the predicate chips and the
/// graph/shell colour tokens, and the chip colours derive from the theme's
/// base-content variable rather than literal black.
#[test]
fn shell_css_has_dark_scheme_tokens() {
    let dir = TempDir::new().expect("create temp dir");
    let out = build_vault(&dir);

    let css =
        fs::read_to_string(out.join("_static/shell.css")).expect("dist/_static/shell.css");
    assert!(
        css.contains("@media (prefers-color-scheme: dark)"),
        "shell.css must carry a dark-scheme override block"
    );
    for token in [
        "--zetl-graph-node:",
        "--zetl-graph-label:",
        "--zetl-shell-surface:",
        "--zetl-shell-border:",
    ] {
        assert!(
            css.matches(token).count() >= 2,
            "{token} should be defined for both light and dark schemes"
        );
    }

    // The chip styles must track the active theme (oklch(var(--bc) / …)),
    // not hardcode near-black rgba values that vanish on a dark background.
    let chip = css
        .split(".zetl-edge-predicate")
        .nth(1)
        .and_then(|rest| rest.split('}').next())
        .expect("shell.css should style .zetl-edge-predicate");
    assert!(
        chip.contains("var(--bc)"),
        "predicate chip colours should derive from the theme's base-content variable"
    );
    assert!(
        !chip.contains("rgba(0,0,0"),
        "predicate chip colours must not hardcode black (illegible on dark backgrounds)"
    );
}
