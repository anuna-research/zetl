use std::collections::HashSet;
use std::path::PathBuf;

use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use serde::Deserialize;

use crate::scanner::{body_text_ranges, page_slug_from_path};
use crate::search::{
    byte_offset_to_line_col, detect_headings, extract_search_context, find_heading_for_offset,
    in_body_text, SearchMatch, SearchOutput,
};
use crate::search_index::SearchIndex;
use crate::web::context::{build_folder_context, build_page_context, build_vault_context};
use crate::web::engine::TemplateError;
use crate::web::html::{html_escape, urlencoding};
use crate::web::markdown;
use crate::web::{reindex, WebState};

/// GET / — Landing page with vault stats and page grid.
pub async fn index_handler(State(state): State<WebState>) -> Response {
    let data = state.data.read().unwrap();
    let vault_name = state
        .vault_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "vault".to_string());

    let vault_ctx = build_vault_context(&data, &vault_name);
    match state.engine.render_index(&vault_ctx) {
        Ok(html) => Html(html).into_response(),
        Err(e) => render_error_response(e),
    }
}

/// GET /{*path} — Rendered markdown page with backlinks, or folder index.
pub async fn page_handler(State(state): State<WebState>, Path(slug): Path<String>) -> Response {
    let slug = urldecode(&slug);
    let slug = slug.trim_end_matches('/');
    let data = state.data.read().unwrap();

    let vault_name = state
        .vault_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "vault".to_string());

    // Find the file by matching slug (relative path without extension)
    let file = data
        .files
        .iter()
        .find(|f| page_slug_from_path(&f.path).eq_ignore_ascii_case(slug));

    // If no page matches, check if slug is a folder prefix → render folder index
    if file.is_none() {
        let folder_prefix = format!("{}/", slug.to_lowercase());
        let has_pages = data.files.iter().any(|f| {
            let s = page_slug_from_path(&f.path);
            s.to_lowercase().starts_with(&folder_prefix)
        });

        if has_pages {
            let folder_name = slug.rsplit('/').next().unwrap_or(slug);
            let vault_ctx = build_vault_context(&data, &vault_name);
            let folder_ctx = build_folder_context(&data, slug, folder_name);
            return match state.engine.render_folder(&vault_ctx, &folder_ctx) {
                Ok(html) => Html(html).into_response(),
                Err(e) => render_error_response(e),
            };
        }
    }

    let (rendered, page_name, current_slug, raw_content) = if let Some(file) = file {
        let full_path = state.vault_root.join(&file.path);
        let file_slug = page_slug_from_path(&file.path);
        match std::fs::read_to_string(&full_path) {
            Ok(content) => {
                let html = markdown::render_to_html(&content, &data.page_slug_map);
                (html, file.page_name.clone(), file_slug, Some(content))
            }
            Err(_) => (
                "<p class=\"text-error\">Could not read file.</p>".to_string(),
                file.page_name.clone(),
                file_slug,
                None,
            ),
        }
    } else {
        // Page doesn't exist yet — open directly in edit mode
        let name = slug.rsplit('/').next().unwrap_or(slug).to_string();
        (
            String::new(),
            name.clone(),
            slug.to_string(),
            Some(format!("# {name}\n")),
        )
    };

    // Build transclusion cards HTML (still assembled as raw HTML for the template)
    let forward_links = data.graph.forward_links(&page_name);
    let mut seen_targets = HashSet::new();
    let mut unique_targets: Vec<String> = Vec::new();
    for link in &forward_links {
        let key = link.target.to_lowercase();
        if seen_targets.insert(key) {
            unique_targets.push(link.target.clone());
        }
    }

    let colors = [
        "#f472b6", "#60a5fa", "#34d399", "#fbbf24", "#a78bfa", "#fb923c", "#2dd4bf", "#f87171",
    ];

    let mut transclusion_cards = String::new();
    for (i, target) in unique_targets.iter().enumerate() {
        let color = colors[i % colors.len()];
        let target_slug = data.slug_for_page(target);
        let href = urlencoding(&target_slug);

        let preview_html = data
            .files
            .iter()
            .find(|f| f.page_name.eq_ignore_ascii_case(target))
            .and_then(|file| {
                let full_path = state.vault_root.join(&file.path);
                std::fs::read_to_string(&full_path).ok()
            })
            .map(|content| markdown::render_preview_html(&content, &data.page_slug_map))
            .unwrap_or_else(|| format!("<p><em>{}</em></p>", html_escape("(page does not exist)")));

        transclusion_cards.push_str(&format!(
            r#"<div class="transclusion-card" data-target-href="/{href}" style="border-left-color: {color};">
  <a href="/{href}" class="tc-title" style="color: {color};">{name}</a>
  <div class="tc-excerpt prose prose-sm max-w-none">{preview}</div>
</div>"#,
            href = href,
            color = color,
            name = html_escape(target),
            preview = preview_html,
        ));
    }

    // Build the page context using the builder, then fill in transclusion + raw_escaped
    let content_raw = raw_content.as_deref().unwrap_or("");
    let mut page_ctx = build_page_context(&data, &page_name, &current_slug, &rendered, content_raw);
    page_ctx.transclusion_cards = transclusion_cards;
    page_ctx.raw_escaped = raw_content.map(|c| html_escape(&c));

    let vault_ctx = build_vault_context(&data, &vault_name);
    match state.engine.render_page(&vault_ctx, &page_ctx, "serve") {
        Ok(html) => Html(html).into_response(),
        Err(e) => render_error_response(e),
    }
}

/// PUT /{*path} — Save edited markdown back to the vault file, then re-index.
pub async fn save_handler(
    State(state): State<WebState>,
    Path(slug): Path<String>,
    body: String,
) -> Response {
    let slug = urldecode(&slug);

    // Look up file path under read lock, then drop it before writing.
    // For new pages, create at vault_root/{slug}.md.
    let full_path = {
        let data = state.data.read().unwrap();
        let file = data
            .files
            .iter()
            .find(|f| page_slug_from_path(&f.path).eq_ignore_ascii_case(&slug));
        if let Some(file) = file {
            state.vault_root.join(&file.path)
        } else {
            state.vault_root.join(format!("{slug}.md"))
        }
    };

    // Ensure parent directory exists for nested paths
    if let Some(parent) = full_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("mkdir error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Cannot create directory").into_response();
        }
    }

    if let Err(e) = std::fs::write(&full_path, &body) {
        eprintln!("save error: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "Write failed").into_response();
    }

    // Re-index the vault so the graph/links and search index reflect the edit.
    match reindex(&state.vault_root) {
        Ok(new_data) => {
            // Rebuild Tantivy search index so the reader picks up the new content
            // (ReloadPolicy::OnCommitWithDelay causes the existing reader to reload).
            if let Err(e) = SearchIndex::build(&state.vault_root, &new_data.files) {
                eprintln!("search index rebuild error: {e}");
            }
            *state.data.write().unwrap() = new_data;
        }
        Err(e) => {
            eprintln!("reindex error: {e}");
            // File was saved; index is stale but not fatal
        }
    }

    StatusCode::OK.into_response()
}

/// GET /preview/{*path} — Returns a short HTML preview (for tooltip).
pub async fn preview_handler(
    State(state): State<WebState>,
    Path(slug): Path<String>,
) -> Html<String> {
    let slug = urldecode(&slug);
    let data = state.data.read().unwrap();

    let file = data
        .files
        .iter()
        .find(|f| page_slug_from_path(&f.path).eq_ignore_ascii_case(&slug));

    let preview = if let Some(file) = file {
        let full_path = state.vault_root.join(&file.path);
        match std::fs::read_to_string(&full_path) {
            Ok(content) => {
                let text = markdown::render_preview(&content);
                format!(
                    r#"<div class="font-semibold mb-1">{}</div><div class="opacity-80">{}</div>"#,
                    html_escape(&file.page_name),
                    text
                )
            }
            Err(_) => "<em>Could not read file.</em>".to_string(),
        }
    } else {
        format!("<em>{} (does not exist)</em>", html_escape(&slug))
    };

    Html(preview)
}

/// Query parameters for GET /api/search.
#[derive(Deserialize)]
pub struct SearchParams {
    pub q: Option<String>,
    pub limit: Option<usize>,
}

/// GET /api/search — Full-text search over the vault index.
///
/// Query parameters:
///   - q (required): search query string
///   - limit (optional, default 20): max results
///
/// Returns 400 if `q` is absent or whitespace-only.
///
/// REQ-013-012, CON-013-003.
pub async fn api_search_handler(
    State(state): State<WebState>,
    Query(params): Query<SearchParams>,
) -> Response {
    let q = match params.q.as_deref() {
        Some(s) if !s.trim().is_empty() => s.to_string(),
        _ => {
            return (StatusCode::BAD_REQUEST, "Missing or empty 'q' parameter").into_response();
        }
    };

    let limit = params.limit.unwrap_or(20).max(1);

    let hits = match state.search_index.query(&q, limit) {
        Ok(hits) => hits,
        Err(e) => {
            eprintln!("api/search error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Search failed").into_response();
        }
    };

    let terms: Vec<String> = q.split_whitespace().map(|t| t.to_lowercase()).collect();

    let mut all_matches: Vec<SearchMatch> = Vec::new();

    for hit in &hits {
        let abs_path = state.vault_root.join(&hit.path);
        let content = match std::fs::read_to_string(&abs_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let body_ranges = body_text_ranges(&content);
        let line_starts: Vec<usize> = std::iter::once(0)
            .chain(content.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        let headings = detect_headings(&content, &body_ranges);
        let search_content = content.to_lowercase();

        for term in &terms {
            let mut start = 0usize;
            while let Some(pos) = search_content[start..].find(term.as_str()) {
                let byte_offset = start + pos;
                start = byte_offset + 1;

                if !in_body_text(byte_offset, &body_ranges) {
                    continue;
                }

                let (line, col) = byte_offset_to_line_col(&line_starts, byte_offset);
                let ctx = extract_search_context(&content, byte_offset, term.len(), 80);
                let (heading, heading_level) = find_heading_for_offset(&headings, byte_offset);

                all_matches.push(SearchMatch {
                    page: hit.page_name.clone(),
                    path: hit.path.clone(),
                    line,
                    column: col,
                    context: ctx,
                    heading,
                    heading_level,
                    score: hit.score,
                });
            }
        }
    }

    all_matches.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.path.cmp(&b.path))
            .then(a.line.cmp(&b.line))
    });

    let total = all_matches.len();
    all_matches.truncate(limit);

    let output = SearchOutput {
        query: q,
        total_matches: total,
        near: None,
        depth: None,
        neighbourhood_size: None,
        results: all_matches,
    };

    Json(output).into_response()
}

/// GET /_static/{*path} — Serve static assets with two-tier lookup.
///
/// Lookup order:
///   1. .zetl/themes/<active-theme>/static/<path>  (per-theme override)
///   2. .zetl/static/<path>                         (vault-wide fallback)
///
/// Returns 404 if neither location has the file (or if the directories don't exist).
pub async fn static_handler(
    State(state): State<WebState>,
    Path(req_path): Path<String>,
) -> Response {
    // Reject path traversal: no ".." components, no null bytes
    if req_path.contains('\0') || req_path.split('/').any(|seg| seg == "..") {
        return StatusCode::NOT_FOUND.into_response();
    }

    // Also reject empty path
    if req_path.is_empty() {
        return StatusCode::NOT_FOUND.into_response();
    }

    // Build candidate paths (two-tier lookup)
    let zetl_dir = state.vault_root.join(".zetl");
    let candidates: Vec<PathBuf> = {
        let mut c = Vec::with_capacity(2);
        // 1. Theme-specific static dir
        if !state.theme.is_empty() {
            c.push(
                zetl_dir
                    .join("themes")
                    .join(&state.theme)
                    .join("static")
                    .join(&req_path),
            );
        }
        // 2. Vault-wide static dir
        c.push(zetl_dir.join("static").join(&req_path));
        c
    };

    for candidate in &candidates {
        // Canonicalize and ensure it's still within the expected static dir
        let Ok(canonical) = candidate.canonicalize() else {
            continue;
        };
        if !canonical.is_file() {
            continue;
        }

        // Safety: ensure canonical path is under .zetl/
        let Ok(zetl_canonical) = zetl_dir.canonicalize() else {
            continue;
        };
        if !canonical.starts_with(&zetl_canonical) {
            return StatusCode::NOT_FOUND.into_response();
        }

        let Ok(body) = std::fs::read(&canonical) else {
            continue;
        };

        let mime = mime_from_ext(&req_path);
        return ([(header::CONTENT_TYPE, mime)], body).into_response();
    }

    StatusCode::NOT_FOUND.into_response()
}

/// Infer a MIME type from a file path's extension.
fn mime_from_ext(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("");
    match ext.to_ascii_lowercase().as_str() {
        "js" | "mjs" => "application/javascript",
        "css" => "text/css",
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "xml" => "application/xml",
        "txt" => "text/plain",
        "map" => "application/json",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

/// Convert a TemplateError into a 500 response with a styled HTML error page.
fn render_error_response(err: TemplateError) -> Response {
    eprintln!("template error: {err}");
    (StatusCode::INTERNAL_SERVER_ERROR, Html(err.to_error_html())).into_response()
}

/// Decode %20-style URL encoding.
fn urldecode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.bytes();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let hi = chars.next().unwrap_or(b'0');
            let lo = chars.next().unwrap_or(b'0');
            let byte = hex_byte(hi, lo);
            result.push(byte as char);
        } else if b == b'+' {
            result.push(' ');
        } else {
            result.push(b as char);
        }
    }
    result
}

fn hex_byte(hi: u8, lo: u8) -> u8 {
    (hex_nibble(hi) << 4) | hex_nibble(lo)
}

fn hex_nibble(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use axum::routing::get;
    use axum::Router;
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, RwLock};
    use tower::ServiceExt;

    use crate::graph::LinkGraph;
    use crate::web::engine::TemplateEngine;
    use crate::web::{VaultData, WebState};

    /// Build a minimal WebState pointing at a temp dir.
    fn test_state(vault_root: &std::path::Path, theme: &str) -> WebState {
        let data = VaultData {
            files: vec![],
            graph: LinkGraph::build(&[], &HashMap::new()),
            page_names: vec![],
            resolved: HashSet::new(),
            page_slug_map: HashMap::new(),
            collision_names: HashSet::new(),
        };
        WebState {
            data: Arc::new(RwLock::new(data)),
            vault_root: Arc::new(vault_root.to_path_buf()),
            engine: Arc::new(TemplateEngine::new(vault_root, theme, false, false)),
            theme: theme.to_string(),
        }
    }

    fn static_router(state: WebState) -> Router {
        Router::new()
            .route("/_static/{*path}", get(static_handler))
            .with_state(state)
    }

    async fn get_status(app: &Router, uri: &str) -> StatusCode {
        let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        resp.status()
    }

    async fn get_body(app: &Router, uri: &str) -> (StatusCode, Vec<u8>, String) {
        let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let ct = resp
            .headers()
            .get("content-type")
            .map(|v| v.to_str().unwrap().to_string())
            .unwrap_or_default();
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap()
            .to_vec();
        (status, body, ct)
    }

    #[test]
    fn test_mime_from_ext() {
        assert_eq!(mime_from_ext("app.js"), "application/javascript");
        assert_eq!(mime_from_ext("style.css"), "text/css");
        assert_eq!(mime_from_ext("image.png"), "image/png");
        assert_eq!(mime_from_ext("photo.jpg"), "image/jpeg");
        assert_eq!(mime_from_ext("icon.svg"), "image/svg+xml");
        assert_eq!(mime_from_ext("font.woff2"), "font/woff2");
        assert_eq!(mime_from_ext("font.woff"), "font/woff");
        assert_eq!(mime_from_ext("data.json"), "application/json");
        assert_eq!(mime_from_ext("page.html"), "text/html");
        assert_eq!(mime_from_ext("unknown.xyz"), "application/octet-stream");
        // Case-insensitive extension
        assert_eq!(mime_from_ext("IMG.PNG"), "image/png");
        assert_eq!(mime_from_ext("STYLE.CSS"), "text/css");
    }

    #[tokio::test]
    async fn static_serves_from_vault_static() {
        let tmp = tempfile::tempdir().unwrap();
        let static_dir = tmp.path().join(".zetl/static");
        std::fs::create_dir_all(&static_dir).unwrap();
        std::fs::write(static_dir.join("app.js"), b"console.log('hi');").unwrap();

        let state = test_state(tmp.path(), "default");
        let app = static_router(state);

        let (status, body, ct) = get_body(&app, "/_static/app.js").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, b"console.log('hi');");
        assert_eq!(ct, "application/javascript");
    }

    #[tokio::test]
    async fn static_theme_overrides_vault() {
        let tmp = tempfile::tempdir().unwrap();
        // Vault-wide static
        let vault_static = tmp.path().join(".zetl/static");
        std::fs::create_dir_all(&vault_static).unwrap();
        std::fs::write(vault_static.join("style.css"), b"body{color:red}").unwrap();
        // Theme-specific static (should win)
        let theme_static = tmp.path().join(".zetl/themes/mytheme/static");
        std::fs::create_dir_all(&theme_static).unwrap();
        std::fs::write(theme_static.join("style.css"), b"body{color:blue}").unwrap();

        let state = test_state(tmp.path(), "mytheme");
        let app = static_router(state);

        let (status, body, ct) = get_body(&app, "/_static/style.css").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, b"body{color:blue}");
        assert_eq!(ct, "text/css");
    }

    #[tokio::test]
    async fn static_falls_back_to_vault_when_theme_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let vault_static = tmp.path().join(".zetl/static");
        std::fs::create_dir_all(&vault_static).unwrap();
        std::fs::write(vault_static.join("logo.png"), b"\x89PNG").unwrap();

        // Theme dir doesn't have logo.png
        let theme_static = tmp.path().join(".zetl/themes/mytheme/static");
        std::fs::create_dir_all(&theme_static).unwrap();

        let state = test_state(tmp.path(), "mytheme");
        let app = static_router(state);

        let (status, body, ct) = get_body(&app, "/_static/logo.png").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, b"\x89PNG");
        assert_eq!(ct, "image/png");
    }

    #[tokio::test]
    async fn static_404_for_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(tmp.path(), "default");
        let app = static_router(state);

        let status = get_status(&app, "/_static/nope.js").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn static_rejects_path_traversal() {
        let tmp = tempfile::tempdir().unwrap();
        // Put a secret file outside .zetl
        std::fs::write(tmp.path().join("secret.txt"), b"secret").unwrap();
        let vault_static = tmp.path().join(".zetl/static");
        std::fs::create_dir_all(&vault_static).unwrap();

        let state = test_state(tmp.path(), "default");
        let app = static_router(state);

        let status = get_status(&app, "/_static/../secret.txt").await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let status = get_status(&app, "/_static/../../etc/passwd").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn static_serves_nested_path() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join(".zetl/static/fonts/sub");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("inter.woff2"), b"woff2data").unwrap();

        let state = test_state(tmp.path(), "default");
        let app = static_router(state);

        let (status, body, ct) = get_body(&app, "/_static/fonts/sub/inter.woff2").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, b"woff2data");
        assert_eq!(ct, "font/woff2");
    }

    #[tokio::test]
    async fn static_no_dirs_returns_404_without_error() {
        let tmp = tempfile::tempdir().unwrap();
        // No .zetl directory at all
        let state = test_state(tmp.path(), "default");
        let app = static_router(state);

        let status = get_status(&app, "/_static/anything.js").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
