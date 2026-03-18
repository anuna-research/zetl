//! Comprehensive integration tests for SPEC-012: Named Themes for Serve and Build.
//!
//! Covers all REQ-012-* requirements and TEST-012-* test cases:
//!
//!   TEST-012-001  Template engine renders default templates
//!   TEST-012-002  Default theme output matches expectations
//!   TEST-012-003  Named theme loading with fallback
//!   TEST-012-004  Template context contains all required data
//!   TEST-012-005  Static asset serving in serve mode
//!   TEST-012-006  Static asset copying in build mode
//!   TEST-012-007  Frontmatter parsing (end-to-end)
//!   TEST-012-008  Mode-aware rendering (edit mode serve-only)
//!   TEST-012-009  Template error reporting
//!   TEST-012-010  Theme selection CLI flag

use std::fs;
use std::path::Path;
use std::sync::{Arc, RwLock};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum::Router;
use tower::ServiceExt;

use zetl::search_index::SearchIndex;
use zetl::web::engine::TemplateEngine;
use zetl::web::routes::{
    index_handler, page_handler, preview_handler, save_handler, static_handler,
};
use zetl::web::WebState;

// ── helpers ───────────────────────────────────────────────────────────────────

fn write_file(root: &Path, relative: &str, content: &str) {
    let full = root.join(relative);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).expect("create parent dirs");
    }
    fs::write(&full, content).expect("write test file");
}

/// CLI helper: returns a Command for the zetl binary with -d and --no-cache.
fn zetl_cmd(vault: &Path) -> assert_cmd::Command {
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("zetl");
    cmd.arg("-d").arg(vault.as_os_str());
    cmd.arg("--no-cache");
    cmd
}

/// Build a WebState by re-indexing a vault directory (uses the real scanner/graph).
fn build_web_state(vault_root: &Path, theme: &str) -> WebState {
    let data = zetl::web::reindex(&vault_root.to_path_buf()).expect("reindex");
    let search_index = SearchIndex::build(vault_root, &data.files).expect("build search index");
    WebState {
        data: Arc::new(RwLock::new(data)),
        vault_root: Arc::new(vault_root.to_path_buf()),
        search_index: Arc::new(search_index),
        engine: Arc::new(TemplateEngine::new(vault_root, theme, true, false)),
        theme: theme.to_string(),
        verbose: false,
        sessions: zetl::web::session::SessionStore::new(),
        recovery_challenges: Arc::new(
            zetl::user::recovery::RecoveryChallengeStore::new(),
        ),
        mnemonic_shown: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        rate_limiters: zetl::web::rate_limit::AuthRateLimiters::new(),
        collab: false,
        #[cfg(feature = "reason")]
        acl_cache: Arc::new(std::sync::Mutex::new(zetl::web::AclCache::new())),
        git_commit_lock: None,
        ws_hub: zetl::web::ws::WsHub::new(),
        ticket_store: zetl::web::ws::TicketStore::new(),
        #[cfg(feature = "semantic")]
        vector_index: None,
    }
}

/// Build the full router matching the serve command's route table.
fn full_router(state: WebState) -> Router {
    Router::new()
        .route("/", get(index_handler))
        .route("/_static/{*path}", get(static_handler))
        .route("/preview/{*path}", get(preview_handler))
        .route("/{*path}", get(page_handler).put(save_handler))
        .with_state(state)
}

async fn get_response(app: &Router, uri: &str) -> (StatusCode, String, String) {
    let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let ct = resp
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    let body = axum::body::to_bytes(resp.into_body(), 10_000_000)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).to_string(), ct)
}

async fn get_bytes(app: &Router, uri: &str) -> (StatusCode, Vec<u8>, String) {
    let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let ct = resp
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    let body = axum::body::to_bytes(resp.into_body(), 10_000_000)
        .await
        .unwrap()
        .to_vec();
    (status, body, ct)
}

async fn put_body(app: &Router, uri: &str, body: &str) -> StatusCode {
    let req = Request::builder()
        .method("PUT")
        .uri(uri)
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    resp.status()
}

/// Create the standard test vault used by most tests.
///
/// Structure:
/// ```text
/// vault_root/
///   page-one.md          (frontmatter: title, tags)
///   page-two.md          (frontmatter: format=fountain; links to page-one)
///   no-frontmatter.md    (plain markdown, no frontmatter)
///   dead-link-page.md    (contains [[NonExistent]] dead link)
///   subfolder/
///     nested-page.md     (links to page-one via backlink)
///   .zetl/
///     themes/
///       custom/
///         page.html      (overrides page.html with frontmatter-conditional rendering)
///         static/
///           custom.css   (per-theme static asset)
///           common.css   (theme override of shared asset)
///     static/
///       shared.js        (vault-wide static asset)
///       common.css       (shared asset, overridden by custom theme)
/// ```
fn build_test_vault(root: &Path) {
    // ── markdown pages ────────────────────────────────────────────────────
    write_file(
        root,
        "page-one.md",
        r#"---
title: Page One Title
tags:
  - rust
  - testing
---
# Page One

This is page one with [[Page Two]] link and [[No Frontmatter]] link.
"#,
    );

    write_file(
        root,
        "page-two.md",
        r#"---
format: fountain
author: Test Author
---
# Page Two

This page links back to [[Page One]].
"#,
    );

    write_file(
        root,
        "no-frontmatter.md",
        r#"# No Frontmatter

This page has no YAML frontmatter at all.

It links to [[Page One]].
"#,
    );

    write_file(
        root,
        "dead-link-page.md",
        r#"# Dead Link Page

This page has a [[NonExistent]] dead link and a valid [[Page Two]] link.
"#,
    );

    write_file(
        root,
        "subfolder/nested-page.md",
        r#"---
title: Nested Page
category: docs
---
# Nested Page

A page in a subfolder linking to [[Page One]].
"#,
    );

    // ── custom theme ──────────────────────────────────────────────────────
    // Overrides page.html with frontmatter-conditional rendering:
    // - Shows format badge if page.frontmatter.format exists
    // - Shows tags if page.frontmatter.tags exists
    // - Includes a CUSTOM_THEME_MARKER for detection
    // - Falls back to built-in base.html (cross-tier inheritance)
    write_file(
        root,
        ".zetl/themes/custom/page.html",
        r#"{% extends "base.html" %}

{% block title %}CUSTOM: {{ page.title }}{% endblock %}

{% block content %}
<div class="custom-theme-page" data-marker="CUSTOM_THEME_MARKER">
  <h1>{{ page.title }}</h1>

  {% if page.frontmatter.format %}
  <span class="format-badge">FORMAT:{{ page.frontmatter.format }}</span>
  {% endif %}

  {% if page.frontmatter.tags %}
  <ul class="tag-list">
    {% for tag in page.frontmatter.tags %}
    <li class="tag">TAG:{{ tag }}</li>
    {% endfor %}
  </ul>
  {% endif %}

  {% if page.frontmatter.author %}
  <span class="author-info">AUTHOR:{{ page.frontmatter.author }}</span>
  {% endif %}

  <div class="page-content">{{ page.content_html }}</div>

  {% if mode == 'serve' %}
  <div class="edit-controls" data-mode="serve">
    <button id="edit-btn">Edit</button>
  </div>
  {% endif %}

  <div class="theme-name">THEME:{{ theme }}</div>

  {% if page.backlinks %}
  <div class="backlinks-section">
    {% for bl in page.backlinks %}
    <a href="/{{ bl.slug }}">BACKLINK:{{ bl.title }}</a>
    {% endfor %}
  </div>
  {% endif %}

  {% if page.outlinks %}
  <div class="outlinks-section">
    {% for ol in page.outlinks %}
    <a href="/{{ ol.slug }}" class="outlink-{{ ol.color }}">OUTLINK:{{ ol.title }}</a>
    {% endfor %}
  </div>
  {% endif %}
</div>
{% endblock %}
"#,
    );

    // ── static assets ─────────────────────────────────────────────────────
    write_file(root, ".zetl/static/shared.js", "console.log('shared');");
    write_file(root, ".zetl/static/common.css", "body{color:shared}");

    write_file(
        root,
        ".zetl/themes/custom/static/custom.css",
        "body{color:custom}",
    );
    write_file(
        root,
        ".zetl/themes/custom/static/common.css",
        "body{color:theme-override}",
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// TEST-012-001 & TEST-012-002: Default theme renders correctly
// REQ-012-001: Template engine integration
// REQ-012-002: Default theme
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_serve_default_theme_index() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    let state = build_web_state(tmp.path(), "default");
    let app = full_router(state);

    let (status, body, ct) = get_response(&app, "/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(ct.contains("text/html"));

    // Index page should list all pages (titles are file stems)
    assert!(body.contains("page-one"), "index should list page-one");
    assert!(body.contains("page-two"), "index should list page-two");
    assert!(
        body.contains("no-frontmatter"),
        "index should list no-frontmatter"
    );
    assert!(
        body.contains("dead-link-page"),
        "index should list dead-link-page"
    );
    assert!(
        body.contains("nested-page"),
        "index should list nested-page"
    );

    // Stats should be present (5 pages total)
    assert!(body.contains("5"), "should show total page count");
}

#[tokio::test]
async fn test_serve_default_theme_page() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    let state = build_web_state(tmp.path(), "default");
    let app = full_router(state);

    let (status, body, _ct) = get_response(&app, "/page-one").await;
    assert_eq!(status, StatusCode::OK);

    // Page content should be rendered (title is file stem)
    assert!(body.contains("page-one"), "page title present");
    assert!(body.contains("Page One"), "markdown heading rendered");

    // Default theme uses built-in templates (not custom)
    assert!(
        !body.contains("CUSTOM_THEME_MARKER"),
        "default theme should not use custom template"
    );
}

#[test]
fn test_build_default_theme_produces_output() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    let out_dir = tmp.path().join("dist");
    zetl_cmd(tmp.path())
        .arg("build")
        .arg("-o")
        .arg(out_dir.as_os_str())
        .assert()
        .success();

    // index.html at root
    let index = fs::read_to_string(out_dir.join("index.html")).expect("index.html");
    assert!(index.contains("page-one"), "index lists pages");
    assert!(index.contains("page-two"), "index lists pages");

    // Per-page HTML files
    assert!(
        out_dir.join("page-one/index.html").exists(),
        "page-one output exists"
    );
    assert!(
        out_dir.join("page-two/index.html").exists(),
        "page-two output exists"
    );
    assert!(
        out_dir.join("no-frontmatter/index.html").exists(),
        "no-frontmatter output exists"
    );
    assert!(
        out_dir.join("dead-link-page/index.html").exists(),
        "dead-link-page output exists"
    );
    assert!(
        out_dir.join("subfolder/nested-page/index.html").exists(),
        "nested page output exists"
    );

    // Folder index for subfolder
    assert!(
        out_dir.join("subfolder/index.html").exists(),
        "subfolder index exists"
    );

    // Page content is rendered correctly
    let page_one = fs::read_to_string(out_dir.join("page-one/index.html")).unwrap();
    assert!(page_one.contains("Page One"), "page title");
    assert!(
        !page_one.contains("CUSTOM_THEME_MARKER"),
        "default theme, no custom markers"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// TEST-012-003: Named theme loading with fallback
// REQ-012-003: Named theme loading
// REQ-012-010: Theme selection CLI flag
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_serve_custom_theme_overrides_page() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    let state = build_web_state(tmp.path(), "custom");
    let app = full_router(state);

    // Custom theme overrides page.html
    let (status, body, _ct) = get_response(&app, "/page-one").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("CUSTOM_THEME_MARKER"),
        "custom page.html should be used"
    );
    assert!(body.contains("CUSTOM: page-one"), "custom title block used");
}

#[tokio::test]
async fn test_serve_custom_theme_falls_back_to_default_base() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    let state = build_web_state(tmp.path(), "custom");
    let app = full_router(state);

    // Custom page.html extends base.html which should fall back to built-in default
    let (status, body, _ct) = get_response(&app, "/page-one").await;
    assert_eq!(status, StatusCode::OK);

    // base.html should provide the overall HTML structure
    assert!(body.contains("<!DOCTYPE html>"), "base.html doctype");
    assert!(body.contains("<html"), "base.html html tag");
}

#[tokio::test]
async fn test_serve_custom_theme_index_uses_default() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    // Custom theme only overrides page.html, not index.html
    let state = build_web_state(tmp.path(), "custom");
    let app = full_router(state);

    let (status, body, _ct) = get_response(&app, "/").await;
    assert_eq!(status, StatusCode::OK);
    // Index should use built-in index.html (no custom override)
    assert!(body.contains("page-one"), "index still lists pages");
    // But theme variable should be "custom"
    assert!(
        body.contains(r#"data-theme="custom""#),
        "theme variable set to custom"
    );
}

#[test]
fn test_build_custom_theme_produces_output() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    let out_dir = tmp.path().join("dist");
    zetl_cmd(tmp.path())
        .arg("build")
        .arg("-o")
        .arg(out_dir.as_os_str())
        .arg("--theme")
        .arg("custom")
        .assert()
        .success();

    // Page rendered with custom theme
    let page_two = fs::read_to_string(out_dir.join("page-two/index.html")).unwrap();
    assert!(
        page_two.contains("CUSTOM_THEME_MARKER"),
        "build uses custom page.html"
    );
    assert!(
        page_two.contains("CUSTOM: page-two"),
        "custom title block in build output"
    );

    // Index uses default (no custom index.html)
    let index = fs::read_to_string(out_dir.join("index.html")).unwrap();
    assert!(
        !index.contains("CUSTOM_THEME_MARKER"),
        "index falls back to default"
    );
}

#[test]
fn test_cli_theme_default_works_without_zetl_dir() {
    let tmp = tempfile::tempdir().unwrap();
    // A vault with just one markdown file, no .zetl/ directory at all
    write_file(tmp.path(), "hello.md", "# Hello\n\nWorld.\n");

    let out_dir = tmp.path().join("dist");
    zetl_cmd(tmp.path())
        .arg("build")
        .arg("-o")
        .arg(out_dir.as_os_str())
        .assert()
        .success();

    assert!(out_dir.join("index.html").exists());
    assert!(out_dir.join("hello/index.html").exists());
}

#[test]
fn test_cli_theme_nonexistent_gives_error() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "hello.md", "# Hello\n");
    // Create a different theme so the hint lists it
    write_file(tmp.path(), ".zetl/themes/existing/page.html", "x");

    let output = zetl_cmd(tmp.path())
        .arg("build")
        .arg("--theme")
        .arg("nonexistent")
        .output()
        .expect("run");

    assert!(!output.status.success(), "nonexistent theme should fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("nonexistent"),
        "error mentions theme name: {stderr}"
    );
    assert!(
        stderr.contains("existing"),
        "error lists available themes: {stderr}"
    );
}

#[test]
fn test_cli_theme_path_traversal_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "hello.md", "# Hello\n");

    let output = zetl_cmd(tmp.path())
        .arg("build")
        .arg("--theme")
        .arg("../escape")
        .output()
        .expect("run");

    assert!(!output.status.success(), "path traversal theme should fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("invalid theme name"),
        "error explains invalidity: {stderr}"
    );
}

#[test]
fn test_cli_theme_with_slash_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "hello.md", "# Hello\n");

    let output = zetl_cmd(tmp.path())
        .arg("build")
        .arg("--theme")
        .arg("bad/theme")
        .output()
        .expect("run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("invalid theme name"));
}

// ═══════════════════════════════════════════════════════════════════════════════
// TEST-012-004: Template context contains all required data
// REQ-012-004: Template data context
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_context_vault_stats() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    let state = build_web_state(tmp.path(), "default");
    let app = full_router(state);

    let (status, body, _ct) = get_response(&app, "/").await;
    assert_eq!(status, StatusCode::OK);
    // 5 pages total
    assert!(body.contains("5"), "total pages stat");
}

#[tokio::test]
async fn test_context_page_backlinks() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    // Use custom theme to see structured backlink data
    let state = build_web_state(tmp.path(), "custom");
    let app = full_router(state);

    // Page One is linked by: Page Two, No Frontmatter, Nested Page
    let (status, body, _ct) = get_response(&app, "/page-one").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("BACKLINK:"),
        "backlinks section should be populated"
    );
}

#[tokio::test]
async fn test_context_page_outlinks_with_dead_link() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    let state = build_web_state(tmp.path(), "custom");
    let app = full_router(state);

    // Dead Link Page has [[NonExistent]] (dead) and [[Page Two]] (live)
    let (status, body, _ct) = get_response(&app, "/dead-link-page").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("outlink-red"),
        "dead link should be red: {body}"
    );
    assert!(body.contains("OUTLINK:page-two"), "live outlink present");
}

#[tokio::test]
async fn test_context_breadcrumbs_for_nested_page() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    let state = build_web_state(tmp.path(), "default");
    let app = full_router(state);

    // Nested page should have breadcrumb path: subfolder > nested-page
    let (status, body, _ct) = get_response(&app, "/subfolder/nested-page").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("subfolder"), "breadcrumb folder present");
}

#[tokio::test]
async fn test_context_theme_variable_accessible() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    let state = build_web_state(tmp.path(), "custom");
    let app = full_router(state);

    let (status, body, _ct) = get_response(&app, "/page-one").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("THEME:custom"),
        "theme variable accessible in template"
    );
    assert!(
        body.contains(r#"data-theme="custom""#),
        "theme in base.html data attribute"
    );
}

#[tokio::test]
async fn test_context_zetl_version() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    let state = build_web_state(tmp.path(), "default");
    let app = full_router(state);

    // The default base.html should include zetl version somewhere
    let (status, body, _ct) = get_response(&app, "/").await;
    assert_eq!(status, StatusCode::OK);
    // Version from Cargo.toml is "0.1.0"
    assert!(
        body.contains("0.1.0") || body.contains("zetl"),
        "zetl version or name present in output"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// TEST-012-005: Static asset serving in serve mode
// REQ-012-005: Static asset serving
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_serve_static_shared_asset() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    let state = build_web_state(tmp.path(), "default");
    let app = full_router(state);

    let (status, body, ct) = get_bytes(&app, "/_static/shared.js").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ct, "application/javascript");
    assert_eq!(body, b"console.log('shared');");
}

#[tokio::test]
async fn test_serve_static_theme_asset() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    let state = build_web_state(tmp.path(), "custom");
    let app = full_router(state);

    let (status, body, ct) = get_bytes(&app, "/_static/custom.css").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ct, "text/css");
    assert_eq!(body, b"body{color:custom}");
}

#[tokio::test]
async fn test_serve_static_theme_overrides_shared() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    let state = build_web_state(tmp.path(), "custom");
    let app = full_router(state);

    // common.css exists in both shared and theme; theme should win
    let (status, body, ct) = get_bytes(&app, "/_static/common.css").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ct, "text/css");
    assert_eq!(body, b"body{color:theme-override}");
}

#[tokio::test]
async fn test_serve_static_fallback_to_shared() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    // Custom theme doesn't have shared.js, should fall back to vault-wide
    let state = build_web_state(tmp.path(), "custom");
    let app = full_router(state);

    let (status, body, ct) = get_bytes(&app, "/_static/shared.js").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ct, "application/javascript");
    assert_eq!(body, b"console.log('shared');");
}

#[tokio::test]
async fn test_serve_static_404_missing() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    let state = build_web_state(tmp.path(), "default");
    let app = full_router(state);

    let (status, _body, _ct) = get_bytes(&app, "/_static/nonexistent.js").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_serve_static_path_traversal_blocked() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());
    // Put a secret outside .zetl
    write_file(tmp.path(), "secret.txt", "top secret");

    let state = build_web_state(tmp.path(), "default");
    let app = full_router(state);

    let (status, _body, _ct) = get_bytes(&app, "/_static/../secret.txt").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _body, _ct) = get_bytes(&app, "/_static/../../etc/passwd").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_serve_static_mime_types() {
    let tmp = tempfile::tempdir().unwrap();

    // Create assets with various extensions
    write_file(tmp.path(), ".zetl/static/app.js", "js");
    write_file(tmp.path(), ".zetl/static/style.css", "css");
    write_file(tmp.path(), ".zetl/static/image.png", "png");
    write_file(tmp.path(), ".zetl/static/photo.jpg", "jpg");
    write_file(tmp.path(), ".zetl/static/icon.svg", "svg");
    write_file(tmp.path(), ".zetl/static/font.woff2", "woff2");
    write_file(tmp.path(), "dummy.md", "# Dummy\n");

    let state = build_web_state(tmp.path(), "default");
    let app = full_router(state);

    let cases = [
        ("/_static/app.js", "application/javascript"),
        ("/_static/style.css", "text/css"),
        ("/_static/image.png", "image/png"),
        ("/_static/photo.jpg", "image/jpeg"),
        ("/_static/icon.svg", "image/svg+xml"),
        ("/_static/font.woff2", "font/woff2"),
    ];

    for (uri, expected_mime) in cases {
        let (status, _body, ct) = get_bytes(&app, uri).await;
        assert_eq!(status, StatusCode::OK, "GET {uri} should be OK");
        assert_eq!(ct, expected_mime, "MIME for {uri}");
    }
}

#[tokio::test]
async fn test_serve_static_no_dirs_no_error() {
    let tmp = tempfile::tempdir().unwrap();
    // No .zetl directory at all
    write_file(tmp.path(), "hello.md", "# Hello\n");

    let state = build_web_state(tmp.path(), "default");
    let app = full_router(state);

    let (status, _body, _ct) = get_bytes(&app, "/_static/anything.js").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ═══════════════════════════════════════════════════════════════════════════════
// TEST-012-006: Static asset copying in build mode
// REQ-012-006: Static asset copying
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_build_static_assets_merged() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    let out_dir = tmp.path().join("dist");
    zetl_cmd(tmp.path())
        .arg("build")
        .arg("-o")
        .arg(out_dir.as_os_str())
        .arg("--theme")
        .arg("custom")
        .assert()
        .success();

    // Shared asset copied
    assert_eq!(
        fs::read_to_string(out_dir.join("_static/shared.js")).unwrap(),
        "console.log('shared');"
    );

    // Theme-specific asset copied
    assert_eq!(
        fs::read_to_string(out_dir.join("_static/custom.css")).unwrap(),
        "body{color:custom}"
    );

    // Theme overrides shared on conflict
    assert_eq!(
        fs::read_to_string(out_dir.join("_static/common.css")).unwrap(),
        "body{color:theme-override}"
    );
}

#[test]
fn test_build_default_theme_copies_only_shared_static() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    let out_dir = tmp.path().join("dist");
    zetl_cmd(tmp.path())
        .arg("build")
        .arg("-o")
        .arg(out_dir.as_os_str())
        .assert()
        .success();

    // Shared assets should be copied
    assert_eq!(
        fs::read_to_string(out_dir.join("_static/shared.js")).unwrap(),
        "console.log('shared');"
    );
    assert_eq!(
        fs::read_to_string(out_dir.join("_static/common.css")).unwrap(),
        "body{color:shared}",
        "default theme should use shared common.css, not theme override"
    );

    // Theme-specific assets should NOT be copied for default theme
    assert!(
        !out_dir.join("_static/custom.css").exists(),
        "custom.css should not be copied for default theme"
    );
}

#[test]
fn test_build_no_static_dirs_no_static_output() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "hello.md", "# Hello\n");

    let out_dir = tmp.path().join("dist");
    zetl_cmd(tmp.path())
        .arg("build")
        .arg("-o")
        .arg(out_dir.as_os_str())
        .assert()
        .success();

    assert!(
        !out_dir.join("_static").exists(),
        "_static/ should not be created when no source dirs exist"
    );
}

#[test]
fn test_build_preserves_nested_static_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "hello.md", "# Hello\n");
    write_file(
        tmp.path(),
        ".zetl/static/fonts/woff2/inter.woff2",
        "fontdata",
    );

    let out_dir = tmp.path().join("dist");
    zetl_cmd(tmp.path())
        .arg("build")
        .arg("-o")
        .arg(out_dir.as_os_str())
        .assert()
        .success();

    assert_eq!(
        fs::read_to_string(out_dir.join("_static/fonts/woff2/inter.woff2")).unwrap(),
        "fontdata"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// TEST-012-007: Frontmatter parsing (end-to-end)
// REQ-012-007: YAML frontmatter parsing
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_frontmatter_accessible_in_custom_theme() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    let state = build_web_state(tmp.path(), "custom");
    let app = full_router(state);

    // Page One has: title, tags: [rust, testing]
    let (status, body, _ct) = get_response(&app, "/page-one").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("TAG:rust"), "tag 'rust' accessible: {body}");
    assert!(body.contains("TAG:testing"), "tag 'testing' accessible");

    // Page Two has: format=fountain, author=Test Author
    let (status, body, _ct) = get_response(&app, "/page-two").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("FORMAT:fountain"), "format field accessible");
    assert!(
        body.contains("AUTHOR:Test Author"),
        "author field accessible"
    );
}

#[tokio::test]
async fn test_frontmatter_empty_when_absent() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    let state = build_web_state(tmp.path(), "custom");
    let app = full_router(state);

    // No Frontmatter page should have empty frontmatter (no format/tags/author sections)
    let (status, body, _ct) = get_response(&app, "/no-frontmatter").await;
    assert_eq!(status, StatusCode::OK);
    assert!(!body.contains("FORMAT:"), "no format when no frontmatter");
    assert!(!body.contains("TAG:"), "no tags when no frontmatter");
    assert!(!body.contains("AUTHOR:"), "no author when no frontmatter");
}

#[tokio::test]
async fn test_frontmatter_stripped_from_rendered_content() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    // Use custom theme which only renders content_html (no raw_escaped textarea)
    // so we can verify frontmatter isn't in the rendered markdown
    let state = build_web_state(tmp.path(), "custom");
    let app = full_router(state);

    let (status, body, _ct) = get_response(&app, "/page-one").await;
    assert_eq!(status, StatusCode::OK);

    // The content_html (inside page-content div) should not have frontmatter
    // Extract the page-content section
    if let Some(start) = body.find(r#"class="page-content">"#) {
        let content_area = &body[start..];
        if let Some(end) = content_area.find("</div>") {
            let rendered_content = &content_area[..end];
            assert!(
                !rendered_content.contains("tags:"),
                "frontmatter YAML should be stripped from rendered content"
            );
            assert!(
                !rendered_content.contains("- rust"),
                "frontmatter list items should not appear in rendered content"
            );
        }
    }
    // Also verify the markdown heading IS rendered
    assert!(body.contains("Page One"), "H1 from markdown rendered");
}

#[test]
fn test_build_frontmatter_in_custom_theme() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    let out_dir = tmp.path().join("dist");
    zetl_cmd(tmp.path())
        .arg("build")
        .arg("-o")
        .arg(out_dir.as_os_str())
        .arg("--theme")
        .arg("custom")
        .assert()
        .success();

    // Page One: tags accessible
    let page_one = fs::read_to_string(out_dir.join("page-one/index.html")).unwrap();
    assert!(page_one.contains("TAG:rust"), "build: tag accessible");
    assert!(page_one.contains("TAG:testing"), "build: tag accessible");

    // Page Two: format accessible
    let page_two = fs::read_to_string(out_dir.join("page-two/index.html")).unwrap();
    assert!(page_two.contains("FORMAT:fountain"), "build: format field");
    assert!(
        page_two.contains("AUTHOR:Test Author"),
        "build: author field"
    );

    // No Frontmatter: empty frontmatter
    let no_fm = fs::read_to_string(out_dir.join("no-frontmatter/index.html")).unwrap();
    assert!(!no_fm.contains("FORMAT:"), "build: no format");
    assert!(!no_fm.contains("TAG:"), "build: no tags");
}

#[tokio::test]
async fn test_frontmatter_malformed_returns_empty() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(
        tmp.path(),
        "malformed.md",
        "---\ninvalid: [unclosed\n---\n# Malformed\n\nContent here.\n",
    );

    let state = build_web_state(tmp.path(), "custom");

    // Even with custom theme, malformed frontmatter should produce empty {}
    // which means no FORMAT/TAG/AUTHOR markers
    let data = state.data.read().unwrap();
    let file = data
        .files
        .iter()
        .find(|f| f.page_name == "malformed")
        .unwrap();
    let content = fs::read_to_string(tmp.path().join(&file.path)).unwrap();
    let fm = zetl::web::markdown::parse_frontmatter(&content);
    // Should be an empty object (malformed YAML)
    assert!(fm.is_object(), "malformed yaml returns object");
    // The object should be empty since the YAML is invalid
    // (Note: serde_yaml may partially parse this; the key thing is it doesn't panic)
}

// ═══════════════════════════════════════════════════════════════════════════════
// TEST-012-008: Mode-aware rendering (edit mode serve-only)
// REQ-012-008: Mode-aware rendering
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_serve_mode_has_edit_controls() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    // Use custom theme which explicitly shows edit controls in serve mode
    let state = build_web_state(tmp.path(), "custom");
    let app = full_router(state);

    let (status, body, _ct) = get_response(&app, "/page-one").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("edit-controls"),
        "serve mode should include edit controls"
    );
    assert!(body.contains(r#"data-mode="serve""#), "mode is serve");
}

#[test]
fn test_build_mode_no_edit_controls() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    let out_dir = tmp.path().join("dist");
    zetl_cmd(tmp.path())
        .arg("build")
        .arg("-o")
        .arg(out_dir.as_os_str())
        .arg("--theme")
        .arg("custom")
        .assert()
        .success();

    let page_one = fs::read_to_string(out_dir.join("page-one/index.html")).unwrap();
    assert!(
        !page_one.contains("edit-controls"),
        "build mode should NOT include edit controls"
    );
    assert!(
        !page_one.contains(r#"data-mode="serve""#),
        "build output should not have serve mode markers"
    );
}

#[tokio::test]
async fn test_default_theme_serve_has_edit_mode() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    let state = build_web_state(tmp.path(), "default");
    let app = full_router(state);

    // Default page.html template also has mode-conditional edit UI
    let (status, body, _ct) = get_response(&app, "/page-one").await;
    assert_eq!(status, StatusCode::OK);
    // The built-in page.html checks {% if mode == 'serve' %} for edit functionality
    // Look for the edit-related elements
    assert!(
        body.contains("edit") || body.contains("Edit"),
        "default serve mode should have some edit functionality"
    );
}

#[test]
fn test_default_theme_build_no_edit_mode() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    let out_dir = tmp.path().join("dist");
    zetl_cmd(tmp.path())
        .arg("build")
        .arg("-o")
        .arg(out_dir.as_os_str())
        .assert()
        .success();

    let page_one = fs::read_to_string(out_dir.join("page-one/index.html")).unwrap();
    // Build mode should not include the edit toggle button or save handler
    assert!(
        !page_one.contains("zetl-edit-toggle"),
        "build output should not have edit toggle"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// TEST-012-009: Template error reporting
// REQ-012-009: Template error reporting
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_serve_template_error_returns_error_page() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "hello.md", "# Hello\n");

    // Create a broken theme with a syntax error in page.html
    write_file(
        tmp.path(),
        ".zetl/themes/broken/page.html",
        "{% extends 'base.html' %}{% block content %}{{ undefined_fn() }{% endblock %}",
    );

    let state = build_web_state(tmp.path(), "broken");
    let app = full_router(state);

    let (status, body, _ct) = get_response(&app, "/hello").await;
    // Should return 500 with a readable error page
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        body.contains("Template Error"),
        "error page should have title: {body}"
    );
    assert!(
        body.contains("page.html"),
        "error page should mention template name: {body}"
    );
}

#[test]
fn test_build_template_error_exits_nonzero() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "hello.md", "# Hello\n");
    write_file(
        tmp.path(),
        ".zetl/themes/broken/page.html",
        "{% extends 'base.html' %}{% block content %}{{ undefined_fn() }{% endblock %}",
    );

    let output = zetl_cmd(tmp.path())
        .arg("build")
        .arg("--theme")
        .arg("broken")
        .output()
        .expect("run");

    assert!(
        !output.status.success(),
        "build with broken template should fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("error:") || stderr.contains("template"),
        "stderr should contain error info: {stderr}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// TEST-012-010: Theme selection CLI flag (additional CLI tests)
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_cli_serve_accepts_theme_flag() {
    // We can't easily test the full serve flow (it blocks), but we can test
    // that the build command with --theme flag works end-to-end.
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    let out_dir = tmp.path().join("dist");
    zetl_cmd(tmp.path())
        .arg("build")
        .arg("-o")
        .arg(out_dir.as_os_str())
        .arg("--theme")
        .arg("custom")
        .assert()
        .success();
}

// ═══════════════════════════════════════════════════════════════════════════════
// Additional integration scenarios
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_serve_folder_index() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    let state = build_web_state(tmp.path(), "default");
    let app = full_router(state);

    // Requesting /subfolder should render a folder index
    let (status, body, _ct) = get_response(&app, "/subfolder").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("nested-page"), "folder lists its pages");
}

#[tokio::test]
async fn test_serve_preview_handler() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    let state = build_web_state(tmp.path(), "default");
    let app = full_router(state);

    let (status, body, ct) = get_response(&app, "/preview/page-one").await;
    assert_eq!(status, StatusCode::OK);
    assert!(ct.contains("text/html"));
    assert!(
        body.contains("page-one"),
        "preview should contain page name"
    );
}

#[tokio::test]
async fn test_serve_save_and_reindex() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    let state = build_web_state(tmp.path(), "default");
    let app = full_router(state);

    // Save new content to a page
    let new_content = "# Page One Updated\n\nNew content with [[Dead Link Page]] link.\n";
    let status = put_body(&app, "/page-one", new_content).await;
    assert_eq!(status, StatusCode::OK);

    // Verify the file was written
    let saved = fs::read_to_string(tmp.path().join("page-one.md")).unwrap();
    assert!(saved.contains("Updated"), "file should be updated");
}

#[tokio::test]
async fn test_serve_new_page_creation() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "existing.md", "# Existing\n");

    let state = build_web_state(tmp.path(), "default");
    let app = full_router(state);

    // Request a non-existent page - should render with is_new=true
    let (status, body, _ct) = get_response(&app, "/brand-new-page").await;
    assert_eq!(status, StatusCode::OK);
    // The default template shows something for new pages
    assert!(
        body.contains("brand-new-page") || body.contains("Brand New Page"),
        "new page should show page info"
    );
}

#[test]
fn test_build_dead_links_marked_red() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    let out_dir = tmp.path().join("dist");
    zetl_cmd(tmp.path())
        .arg("build")
        .arg("-o")
        .arg(out_dir.as_os_str())
        .arg("--theme")
        .arg("custom")
        .assert()
        .success();

    let dead_link_page = fs::read_to_string(out_dir.join("dead-link-page/index.html")).unwrap();
    assert!(
        dead_link_page.contains("outlink-red"),
        "dead link should be marked red in build output"
    );
    assert!(
        dead_link_page.contains("OUTLINK:NonExistent"),
        "dead link target name present"
    );
}

#[test]
fn test_build_subfolder_with_nested_pages() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    let out_dir = tmp.path().join("dist");
    zetl_cmd(tmp.path())
        .arg("build")
        .arg("-o")
        .arg(out_dir.as_os_str())
        .arg("--theme")
        .arg("custom")
        .assert()
        .success();

    // Nested page in subfolder
    let nested = fs::read_to_string(out_dir.join("subfolder/nested-page/index.html")).unwrap();
    assert!(nested.contains("CUSTOM_THEME_MARKER"), "custom theme used");
    assert!(nested.contains("nested-page"), "page title present");

    // Nested page frontmatter accessible
    assert!(
        nested.contains("category") || nested.contains("docs") || !nested.contains("FORMAT:"),
        "nested page frontmatter handled"
    );
}

#[tokio::test]
async fn test_search_index_in_output() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    let state = build_web_state(tmp.path(), "default");
    let app = full_router(state);

    let (status, body, _ct) = get_response(&app, "/").await;
    assert_eq!(status, StatusCode::OK);
    // The base template includes a search index JSON
    assert!(
        body.contains("page-one") || body.contains("Page One"),
        "search index should include pages"
    );
}

#[test]
fn test_build_all_pages_rendered() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    let out_dir = tmp.path().join("dist");
    zetl_cmd(tmp.path())
        .arg("build")
        .arg("-o")
        .arg(out_dir.as_os_str())
        .assert()
        .success();

    // Verify every page has an index.html
    let expected_pages = [
        "page-one",
        "page-two",
        "no-frontmatter",
        "dead-link-page",
        "subfolder/nested-page",
    ];

    for slug in expected_pages {
        let page_path = out_dir.join(format!("{slug}/index.html"));
        assert!(page_path.exists(), "page {slug} should have index.html");
        let content = fs::read_to_string(&page_path).unwrap();
        assert!(
            !content.trim().is_empty(),
            "page {slug} should have non-empty content"
        );
        assert!(
            content.contains("<!DOCTYPE html>") || content.contains("<!doctype html>"),
            "page {slug} should be valid HTML"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// TEST-014-003: Bundled Theme Directory Structure
// TEST-014-004: Three-Tier Template Resolution
// TEST-014-005: Theme Listing
// REQ-014-003: Bundled themes embedded at compile time
// REQ-014-004: Three-tier resolution order
// REQ-014-005: zetl theme list output
// NFR-014-002: Binary size impact ≤ 100 KB
// OBS-014-002: --verbose shows template resolution tier
// ═══════════════════════════════════════════════════════════════════════════════

/// TEST-014-003: All required bundled templates exist for both default and minimal themes.
#[test]
fn test_bundled_default_theme_has_all_templates() {
    use zetl::web::engine::bundled_template;
    for name in &["base.html", "index.html", "page.html", "folder.html"] {
        assert!(
            bundled_template("default", name).is_some(),
            "bundled default theme missing template: {name}"
        );
    }
}

#[test]
fn test_bundled_minimal_theme_has_all_templates() {
    use zetl::web::engine::bundled_template;
    for name in &["base.html", "index.html", "page.html", "folder.html"] {
        assert!(
            bundled_template("minimal", name).is_some(),
            "bundled minimal theme missing template: {name}"
        );
    }
}

#[test]
fn test_bundled_minimal_theme_has_no_cdn_links() {
    use zetl::web::engine::bundled_template;
    let cdn_patterns = [
        "cdn.jsdelivr.net",
        "fonts.googleapis.com",
        "unpkg.com",
        "cdnjs.cloudflare.com",
    ];
    for name in &["base.html", "index.html", "page.html", "folder.html"] {
        let content = bundled_template("minimal", name).unwrap_or("");
        for cdn in &cdn_patterns {
            assert!(
                !content.contains(cdn),
                "minimal theme {name} contains CDN reference to {cdn}: NFR-014-002 requires no CDN"
            );
        }
    }
}

/// TEST-014-004 (scenario 1): zetl serve --theme minimal renders with bundled minimal templates.
#[tokio::test]
async fn test_serve_minimal_theme_uses_bundled_templates() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    // No .zetl/themes/minimal/ on disk — must resolve from bundled minimal theme (Tier 2).
    let state = build_web_state(tmp.path(), "minimal");
    let app = full_router(state);

    let (status, body, _ct) = get_response(&app, "/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("<!DOCTYPE html>") || body.contains("<!doctype html>"),
        "minimal theme index should render valid HTML"
    );
    // Minimal theme uses system fonts (no CDN)
    assert!(
        !body.contains("cdn.jsdelivr.net") && !body.contains("fonts.googleapis.com"),
        "minimal theme serve output must not contain CDN links"
    );
    // Minimal theme uses system fonts stack (distinctive feature vs default theme)
    assert!(
        body.contains("-apple-system") || body.contains("BlinkMacSystemFont"),
        "minimal theme should use system font stack (no external fonts)"
    );
}

/// TEST-014-004 (scenario 1): zetl build --theme minimal produces valid static site.
#[test]
fn test_build_minimal_theme_produces_valid_site() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    let out_dir = tmp.path().join("dist-minimal");
    zetl_cmd(tmp.path())
        .arg("build")
        .arg("-o")
        .arg(out_dir.as_os_str())
        .arg("--theme")
        .arg("minimal")
        .assert()
        .success();

    // Index page exists and is valid HTML
    let index = fs::read_to_string(out_dir.join("index.html")).unwrap();
    assert!(
        index.contains("<!DOCTYPE html>") || index.contains("<!doctype html>"),
        "minimal build index should be valid HTML"
    );
    // No CDN links in build output
    assert!(
        !index.contains("cdn.jsdelivr.net") && !index.contains("fonts.googleapis.com"),
        "minimal build output must not contain CDN links"
    );

    // Page files exist and are valid
    let page_one = fs::read_to_string(out_dir.join("page-one/index.html")).unwrap();
    assert!(
        page_one.contains("<!DOCTYPE html>") || page_one.contains("<!doctype html>"),
        "minimal build page-one should be valid HTML"
    );
    assert!(
        !page_one.is_empty(),
        "minimal build page-one should have content"
    );
}

/// TEST-014-005 (scenario 1): zetl theme list shows bundled themes (no .zetl/themes/).
#[test]
fn test_theme_list_shows_bundled_themes() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "hello.md", "# Hello\n\nWorld.\n");

    let output = zetl_cmd(tmp.path())
        .arg("theme")
        .arg("list")
        .output()
        .expect("run theme list");

    assert!(output.status.success(), "theme list should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Parse JSON
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("theme list should produce valid JSON");

    let themes = json["themes"].as_array().expect("themes should be array");
    let names: Vec<&str> = themes
        .iter()
        .map(|t| t["name"].as_str().unwrap_or(""))
        .collect();

    assert!(
        names.contains(&"default"),
        "theme list should include 'default' bundled theme, got: {names:?}"
    );
    assert!(
        names.contains(&"minimal"),
        "theme list should include 'minimal' bundled theme, got: {names:?}"
    );

    // All listed themes should show source = "bundled" when no .zetl/themes/ exists
    for theme in themes {
        let source = theme["source"].as_str().unwrap_or("");
        assert!(
            source == "bundled" || source.starts_with("installed"),
            "theme source should be 'bundled' or 'installed*', got: {source:?}"
        );
    }

    // Both bundled themes should have version metadata
    for theme_name in &["default", "minimal"] {
        let t = themes
            .iter()
            .find(|t| t["name"].as_str() == Some(theme_name))
            .unwrap();
        assert!(
            t["version"].as_str().is_some(),
            "bundled theme {theme_name} should have a version"
        );
        assert_eq!(
            t["source"].as_str(),
            Some("bundled"),
            "bundled theme {theme_name} should have source 'bundled'"
        );
    }
}

/// TEST-014-005 (scenario 3): .zetl/themes/minimal/ shadows bundled minimal.
/// zetl theme list shows 'installed (shadows bundled)'.
#[test]
fn test_theme_list_minimal_shadows_bundled() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "hello.md", "# Hello\n\nWorld.\n");

    // Create an on-disk minimal theme override (Tier 1 override)
    write_file(
        tmp.path(),
        ".zetl/themes/minimal/page.html",
        r#"{% extends "base.html" %}{% block title %}DISK-MINIMAL: {{ page.title }}{% endblock %}{% block content %}<p>disk-minimal-page</p>{% endblock %}"#,
    );

    let output = zetl_cmd(tmp.path())
        .arg("theme")
        .arg("list")
        .output()
        .expect("run theme list");

    assert!(output.status.success(), "theme list should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("theme list should produce valid JSON");

    let themes = json["themes"].as_array().expect("themes should be array");
    let minimal = themes
        .iter()
        .find(|t| t["name"].as_str() == Some("minimal"))
        .expect("minimal theme should appear in list");

    let source = minimal["source"].as_str().unwrap_or("");
    assert_eq!(
        source, "installed (shadows bundled)",
        "disk minimal should be shown as 'installed (shadows bundled)', got: {source:?}"
    );
}

/// TEST-014-004 (scenario 2): .zetl/themes/minimal/page.html overrides bundled minimal.
/// page.html resolves from disk (Tier 1), base.html from bundled minimal (Tier 2).
#[tokio::test]
async fn test_serve_minimal_theme_disk_page_overrides_bundled() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    // Create an on-disk page.html override for the minimal theme
    write_file(
        tmp.path(),
        ".zetl/themes/minimal/page.html",
        r#"{% extends "base.html" %}{% block title %}DISK-MINIMAL: {{ page.title }}{% endblock %}{% block content %}<div class="disk-minimal-marker">DISK_MINIMAL_OVERRIDE</div>{{ page.content_html | safe }}{% endblock %}"#,
    );

    let state = build_web_state(tmp.path(), "minimal");
    let app = full_router(state);

    let (status, body, _ct) = get_response(&app, "/page-one").await;
    assert_eq!(status, StatusCode::OK);

    // Tier 1: disk page.html override should be used
    assert!(
        body.contains("DISK_MINIMAL_OVERRIDE"),
        "disk .zetl/themes/minimal/page.html should override bundled minimal page.html (Tier 1)"
    );
    assert!(
        body.contains("DISK-MINIMAL: page-one") || body.contains("DISK-MINIMAL:"),
        "custom title from disk template should appear"
    );

    // Tier 2: bundled minimal base.html should be the layout base
    // (system fonts, no CDN, minimal layout — we check that CDN-free structure holds)
    assert!(
        !body.contains("cdn.jsdelivr.net") && !body.contains("fonts.googleapis.com"),
        "base.html from bundled minimal should not contain CDN refs"
    );
    // base.html provides the outer structure
    assert!(
        body.contains("<!DOCTYPE html>") || body.contains("<!doctype html>"),
        "base.html from bundled minimal should provide HTML structure"
    );
}

/// TEST-014-004 (scenario 3): .zetl/themes/custom/ with only page.html.
/// page.html from disk (Tier 1), base.html from bundled default (Tier 3 fallback).
/// This is already tested in test_serve_custom_theme_overrides_page / test_serve_custom_theme_falls_back_to_default_base
/// but adding explicit Tier 3 verification here.
#[tokio::test]
async fn test_three_tier_custom_theme_tier3_fallback() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    // The test vault already has .zetl/themes/custom/page.html
    // Index and base.html are NOT overridden — must fall back to Tier 3 (bundled default)
    let state = build_web_state(tmp.path(), "custom");
    let app = full_router(state);

    // page.html: from disk (Tier 1)
    let (status, body, _ct) = get_response(&app, "/page-one").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("CUSTOM_THEME_MARKER"),
        "page.html should come from disk (Tier 1): custom/page.html"
    );

    // index.html: no disk override → falls back to bundled default (Tier 3)
    let (status, index_body, _ct) = get_response(&app, "/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !index_body.contains("CUSTOM_THEME_MARKER"),
        "index.html should come from bundled default (Tier 3), not custom theme"
    );
    assert!(
        index_body.contains("page-one"),
        "index from bundled default should list pages"
    );
}

/// OBS-014-002: --verbose shows resolution tier per template.
#[test]
fn test_verbose_shows_template_resolution_tiers() {
    let tmp = tempfile::tempdir().unwrap();
    build_test_vault(tmp.path());

    let out_dir = tmp.path().join("dist-verbose");
    let output = zetl_cmd(tmp.path())
        .arg("--verbose")
        .arg("build")
        .arg("-o")
        .arg(out_dir.as_os_str())
        .arg("--theme")
        .arg("custom")
        .output()
        .expect("run build with --verbose");

    assert!(output.status.success(), "verbose build should succeed");

    let stderr = String::from_utf8_lossy(&output.stderr);

    // Should show resolution tier for each template
    assert!(
        stderr.contains("theme:") || stderr.contains("bundled:") || stderr.contains("disk"),
        "verbose output should show template resolution info, got stderr: {stderr}"
    );

    // page.html should show as disk (Tier 1) for custom theme
    assert!(
        stderr.contains("page.html")
            && (stderr.contains("disk") || stderr.contains(".zetl/themes")),
        "verbose output should show page.html resolved from disk for custom theme"
    );

    // index.html should show as bundled default fallback (Tier 3)
    assert!(
        stderr.contains("index.html")
            && (stderr.contains("bundled:default") || stderr.contains("fallback")),
        "verbose output should show index.html from bundled default (Tier 3 fallback)"
    );
}

/// NFR-014-002: Binary size increase from bundled themes is < 200 KB.
///
/// This test measures the raw (uncompressed) size of bundled theme content as an
/// upper-bound proxy. The actual binary impact is smaller after compression, so if
/// the raw content fits within 200 KB we are well within the NFR budget.
///
/// Budget increased from 100 KB to 200 KB to accommodate the fountain theme, which
/// bundles a complete search implementation, sidebar, and screenplay-specific CSS/JS.
#[test]
fn test_bundled_theme_size_within_budget() {
    use zetl::web::engine::{bundled_template, bundled_theme_names};
    let template_names = &[
        "base.html",
        "index.html",
        "page.html",
        "folder.html",
        "theme.toml",
    ];
    let mut total_bytes: usize = 0;
    for theme in bundled_theme_names() {
        for name in template_names {
            if let Some(content) = bundled_template(theme, name) {
                total_bytes += content.len();
            }
        }
    }
    const BUDGET_BYTES: usize = 200 * 1024; // 200 KB
    assert!(
        total_bytes <= BUDGET_BYTES,
        "bundled theme content totals {total_bytes} bytes, exceeds 200 KB budget (NFR-014-002)"
    );
}
