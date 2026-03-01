use std::collections::HashSet;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};

use crate::scanner::page_slug_from_path;
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
pub async fn page_handler(
    State(state): State<WebState>,
    Path(slug): Path<String>,
) -> Response {
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
            Some(format!("# {}\n", name)),
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
            .unwrap_or_else(|| {
                format!(
                    "<p><em>{}</em></p>",
                    html_escape("(page does not exist)")
                )
            });

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
    let mut page_ctx =
        build_page_context(&data, &page_name, &current_slug, &rendered, content_raw);
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
            state.vault_root.join(format!("{}.md", slug))
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

    // Re-index the vault so the graph/links reflect the edit
    match reindex(&state.vault_root) {
        Ok(new_data) => {
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
        format!(
            "<em>{} (does not exist)</em>",
            html_escape(&slug)
        )
    };

    Html(preview)
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
