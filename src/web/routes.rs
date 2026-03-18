use std::collections::HashSet;
use std::path::PathBuf;

use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use serde::Deserialize;

use crate::hooks;
use crate::hooks::context::{build_hook_context, HookSaved};
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

    let mut vault_ctx = build_vault_context(&data, &vault_name);
    #[cfg(feature = "history")]
    {
        // OBS-013: time vault history context build.
        let hist_start = std::time::Instant::now();
        if let Some(hist) = crate::history::build_template_history_context(&state.vault_root) {
            let hist_ms = hist_start.elapsed().as_millis();
            if state.verbose {
                eprintln!(
                    "[zetl] history-context: vault trend={} points recent={} changes duration_ms={}",
                    hist.trend.len(),
                    hist.recent_changes.len(),
                    hist_ms
                );
            }
            vault_ctx.history = serde_json::to_value(hist).unwrap_or(serde_json::Value::Null);
        }
    }
    #[cfg(feature = "semantic")]
    {
        vault_ctx.semantic_available = state.vector_index.is_some();
    }
    match state.engine.render_index(&vault_ctx, "serve", "", "") {
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
            let mut vault_ctx = build_vault_context(&data, &vault_name);
            #[cfg(feature = "history")]
            {
                // OBS-013: time vault history context build.
                let hist_start = std::time::Instant::now();
                if let Some(hist) =
                    crate::history::build_template_history_context(&state.vault_root)
                {
                    let hist_ms = hist_start.elapsed().as_millis();
                    if state.verbose {
                        eprintln!(
                            "[zetl] history-context: vault trend={} points recent={} changes duration_ms={}",
                            hist.trend.len(),
                            hist.recent_changes.len(),
                            hist_ms
                        );
                    }
                    vault_ctx.history =
                        serde_json::to_value(hist).unwrap_or(serde_json::Value::Null);
                }
            }
            #[cfg(feature = "semantic")]
            {
                vault_ctx.semantic_available = state.vector_index.is_some();
            }
            let folder_ctx = build_folder_context(&data, slug, folder_name);
            return match state
                .engine
                .render_folder(&vault_ctx, &folder_ctx, "serve", "", "")
            {
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
                let is_fountain = file.path.extension().map_or(false, |e| e == "fountain");
                let html = if is_fountain {
                    let body = crate::web::build::strip_fountain_frontmatter(&content);
                    format!(
                        "<script type=\"text/fountain\" id=\"fountain-source\">{}</script>\n<div id=\"fountain-render\"></div>",
                        html_escape(&body)
                    )
                } else {
                    markdown::render_to_html(&content, &data.page_slug_map, "/", "")
                };
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
            .map(|content| markdown::render_preview_html(&content, &data.page_slug_map, "/", ""))
            .unwrap_or_else(|| format!("<p><em>{}</em></p>", html_escape("(page does not exist)")));

        transclusion_cards.push_str(&format!(
            r#"<div class="transclusion-card" data-target-href="/{href}/" style="border-left-color: {color};">
  <a href="/{href}/" class="tc-title" style="color: {color};">{name}</a>
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
    #[cfg(feature = "history")]
    {
        // OBS-013: time per-page history context build.
        let hist_start = std::time::Instant::now();
        if let Some(hist) =
            crate::history::build_template_page_history_context(&page_name, &state.vault_root)
        {
            let hist_ms = hist_start.elapsed().as_millis();
            if state.verbose {
                eprintln!(
                    "[zetl] history-context: page {:?} trend={} points created={} duration_ms={}",
                    page_name,
                    hist.link_trend.len(),
                    hist.created_at,
                    hist_ms
                );
            }
            page_ctx.history = serde_json::to_value(hist).unwrap_or(serde_json::Value::Null);
        }
    }
    #[cfg(feature = "history")]
    {
        let sources: Vec<String> = page_ctx.backlinks.iter().map(|b| b.title.clone()).collect();
        let since_map =
            crate::history::build_backlink_since_map(&page_name, &sources, &state.vault_root);
        if !since_map.is_empty() {
            for bl in &mut page_ctx.backlinks {
                bl.since = since_map.get(&bl.title.to_lowercase()).cloned();
            }
        }
    }

    let mut vault_ctx = build_vault_context(&data, &vault_name);
    #[cfg(feature = "history")]
    {
        // OBS-013: time vault history context build.
        let hist_start = std::time::Instant::now();
        if let Some(hist) = crate::history::build_template_history_context(&state.vault_root) {
            let hist_ms = hist_start.elapsed().as_millis();
            if state.verbose {
                eprintln!(
                    "[zetl] history-context: vault trend={} points recent={} changes duration_ms={}",
                    hist.trend.len(),
                    hist.recent_changes.len(),
                    hist_ms
                );
            }
            vault_ctx.history = serde_json::to_value(hist).unwrap_or(serde_json::Value::Null);
        }
    }
    #[cfg(feature = "semantic")]
    {
        vault_ctx.semantic_available = state.vector_index.is_some();
    }
    match state
        .engine
        .render_page(&vault_ctx, &page_ctx, "serve", "", "")
    {
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
    let slug = slug.trim_end_matches('/');

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

    // Fire on-save hooks asynchronously so the response returns immediately.
    {
        let vault_root = state.vault_root.clone();
        let theme = state.theme.clone();
        let content_length = body.len();
        let rel_path = full_path
            .strip_prefix(vault_root.as_ref())
            .unwrap_or(&full_path)
            .to_string_lossy()
            .into_owned();
        let page_name = full_path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();

        tokio::task::spawn_blocking(move || {
            let theme_hooks = hooks::resolve_theme_hooks(&vault_root, &theme);
            let manifest = hooks::discover_hooks(&vault_root, theme_hooks.path());

            if hooks::hooks_for(&manifest, "on-save").is_empty() {
                return;
            }

            // Re-scan vault for fresh graph data used by hook context.
            let files = match crate::scanner::scan_vault(&vault_root, &[]) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("on-save hook: scan error: {e}");
                    return;
                }
            };
            let file_index: Vec<(String, std::path::PathBuf)> = files
                .iter()
                .map(|f| (f.page_name.clone(), f.path.clone()))
                .collect();
            let mut resolved = std::collections::HashMap::new();
            for file in &files {
                for link in &file.links {
                    let key = link.raw_target.clone();
                    if !resolved.contains_key(&key) {
                        if let Some(r) =
                            crate::scanner::resolve_page_name(&link.target_page, &file_index)
                        {
                            resolved.insert(key, r);
                        }
                    }
                }
            }
            let graph = crate::graph::LinkGraph::build(&files, &resolved);

            let mut ctx = build_hook_context(
                "on-save",
                &vault_root,
                &theme,
                env!("CARGO_PKG_VERSION"),
                &files,
                &graph,
            );
            ctx.saved = Some(HookSaved {
                file: rel_path.clone(),
                page: page_name.clone(),
                content_length,
            });

            let context_json = match serde_json::to_vec(&ctx) {
                Ok(j) => j,
                Err(e) => {
                    eprintln!("on-save hook: json error: {e}");
                    return;
                }
            };

            let hook_env = hooks::HookEnv {
                vault_root: vault_root.to_path_buf(),
                theme: theme.clone(),
                zetl_version: env!("CARGO_PKG_VERSION").to_string(),
                extra_vars: vec![
                    ("ZETL_SAVED_FILE".into(), rel_path),
                    ("ZETL_SAVED_PAGE".into(), page_name),
                ],
            };

            let results = hooks::run_hooks(&manifest, "on-save", &context_json, &hook_env);
            for result in results {
                match result {
                    Ok(output) if !output.success() => {
                        eprintln!(
                            "warning: on-save hook '{}' ({}) exited with code {}",
                            output.path.display(),
                            output.source,
                            output.exit_code.unwrap_or(-1),
                        );
                        if !output.stderr.is_empty() {
                            eprintln!("  stderr: {}", output.stderr.trim_end());
                        }
                    }
                    Err(e) => {
                        eprintln!("warning: on-save hook failed to execute: {e}");
                    }
                    _ => {}
                }
            }
        });
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
    /// Search mode: "bm25" (default), "semantic", or "hybrid". REQ-100.
    pub mode: Option<String>,
}

/// GET /api/search — Full-text, semantic, or hybrid search over the vault index.
///
/// Query parameters:
///   - q (required): search query string
///   - limit (optional, default 20): max results
///   - mode (optional, default "bm25"): search mode — "bm25", "semantic", or "hybrid"
///
/// Returns 400 if `q` is absent or whitespace-only.
/// Returns 503 if mode=semantic or mode=hybrid but the vector index is unavailable.
///
/// REQ-013-012, REQ-100, CON-013-003.
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
    let mode = params.mode.as_deref().unwrap_or("bm25");

    // When the semantic feature is not compiled, reject semantic/hybrid modes immediately.
    #[cfg(not(feature = "semantic"))]
    if mode == "semantic" || mode == "hybrid" {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Semantic search requires the `semantic` feature. \
             Rebuild with: cargo build --features semantic",
        )
            .into_response();
    }

    // ── semantic / hybrid modes (REQ-100) ────────────────────────────────────
    #[cfg(feature = "semantic")]
    if mode == "semantic" || mode == "hybrid" {
        let vec_index_arc = match &state.vector_index {
            Some(arc) => arc.clone(),
            None => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Vector index not available. Run `zetl index` first.",
                )
                    .into_response();
            }
        };

        if mode == "semantic" {
            // Pure vector search: embed query, scan index, return chunk-level hits.
            let q_owned = q.clone();
            let vec_limit = limit;
            let hits = tokio::task::spawn_blocking(move || {
                let idx = vec_index_arc.lock().map_err(|_| {
                    anyhow::anyhow!("vector index lock poisoned")
                })?;
                idx.query_text(&q_owned, vec_limit)
            })
            .await;

            let hits = match hits {
                Ok(Ok(h)) => h,
                Ok(Err(e)) => {
                    eprintln!("api/search semantic error: {e}");
                    return (StatusCode::INTERNAL_SERVER_ERROR, "Semantic search failed")
                        .into_response();
                }
                Err(e) => {
                    eprintln!("api/search semantic join error: {e}");
                    return (StatusCode::INTERNAL_SERVER_ERROR, "Semantic search failed")
                        .into_response();
                }
            };

            let total = hits.len();
            let results: Vec<SearchMatch> = hits
                .into_iter()
                .map(|hit| SearchMatch {
                    page: hit.page_name,
                    path: hit.path,
                    line: 0,
                    column: 0,
                    context: hit.heading.clone(),
                    heading: hit.heading,
                    heading_level: None,
                    score: hit.score as f64,
                })
                .collect();

            let output = SearchOutput {
                query: q,
                total_matches: total,
                near: None,
                depth: None,
                neighbourhood_size: None,
                results,
            };
            return Json(output).into_response();
        }

        // mode == "hybrid": RRF fusion of BM25 + vector search (ADR-053, REQ-095).
        let bm25_hits = match state.search_index.query(&q, limit.saturating_mul(2)) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("api/search bm25 error: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, "Search failed").into_response();
            }
        };

        let q_owned = q.clone();
        let vec_limit = limit.saturating_mul(2);
        let vec_hits = tokio::task::spawn_blocking(move || {
            let idx = vec_index_arc.lock().map_err(|_| {
                anyhow::anyhow!("vector index lock poisoned")
            })?;
            idx.query_text(&q_owned, vec_limit)
        })
        .await;

        let vec_hits = match vec_hits {
            Ok(Ok(h)) => h,
            Ok(Err(e)) => {
                eprintln!("api/search vector error: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, "Hybrid search failed")
                    .into_response();
            }
            Err(e) => {
                eprintln!("api/search hybrid join error: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, "Hybrid search failed")
                    .into_response();
            }
        };

        // Build ranked lists at page level for RRF.
        let bm25_ranks: Vec<(String, usize)> = bm25_hits
            .iter()
            .enumerate()
            .map(|(i, h)| (h.page_name.clone(), i + 1))
            .collect();
        let vec_ranks: Vec<(String, usize)> = {
            // Deduplicate chunks by page: keep highest-scoring chunk per page.
            let mut seen: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            for hit in &vec_hits {
                let rank = seen.len() + 1;
                let entry = seen.entry(hit.page_name.clone()).or_insert(0);
                // rank is 1-based; we record order of first appearance
                if *entry == 0 {
                    *entry = rank;
                }
            }
            let mut page_order: Vec<(String, usize)> = seen.into_iter().collect();
            page_order.sort_by_key(|(_, r)| *r);
            page_order
        };

        let fused = crate::semantic::core::reciprocal_rank_fusion(
            &bm25_ranks,
            &vec_ranks,
            crate::semantic::RRF_K,
        );

        // Build a score map from page_name → fused score.
        let score_map: std::collections::HashMap<String, f64> =
            fused.into_iter().collect();

        // Collect BM25 hits for pages in the fused set, scored by RRF.
        let terms: Vec<String> = q.split_whitespace().map(|t| t.to_lowercase()).collect();
        let mut all_matches: Vec<SearchMatch> = Vec::new();

        for hit in &bm25_hits {
            let rrf_score = match score_map.get(&hit.page_name) {
                Some(&s) => s,
                None => continue,
            };
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
                        score: rrf_score,
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
        return Json(output).into_response();
    }

    // ── BM25 mode (default) ──────────────────────────────────────────────────
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

/// GET /_print — Combined print view of all pages for PDF export.
pub async fn print_handler(State(state): State<WebState>) -> Response {
    let data = state.data.read().unwrap();

    // Collect only fountain scenes, sorted alphabetically by title (sidebar order)
    let mut fountain_files: Vec<_> = data
        .files
        .iter()
        .filter(|f| f.path.extension().map_or(false, |e| e == "fountain"))
        .collect();
    fountain_files.sort_by(|a, b| a.page_name.to_lowercase().cmp(&b.page_name.to_lowercase()));

    let mut sections = Vec::new();
    for file in &fountain_files {
        let full_path = state.vault_root.join(&file.path);
        let content = match std::fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let body = crate::web::build::strip_fountain_frontmatter(&content);
        sections.push(format!(
            "<section class=\"print-section\">\
             <script type=\"text/fountain\">{src}</script>\
             <div class=\"fountain-out\"></div></section>",
            src = html_escape(&body),
        ));
    }

    // Inline the Fountain.js parser from the bundled theme
    let fountain_js = crate::web::engine::bundled_template("fountain", "base.html")
        .and_then(|base| {
            let start_marker = "<!-- fountain-js";
            let end_marker = "</script>\n\n  <!-- Render fountain";
            let start = base.find(start_marker)?;
            let end = base.find(end_marker).map(|i| i + "</script>".len())?;
            Some(base[start..end].to_string())
        })
        .unwrap_or_default();

    let html = format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Print — Script</title>
<style>
  @font-face {{
    font-family: 'Courier Prime';
    src: url('/_static/CourierPrime-Regular.woff2') format('woff2');
    font-weight: 400; font-style: normal; font-display: swap;
  }}
  @font-face {{
    font-family: 'Courier Prime';
    src: url('/_static/CourierPrime-Bold.woff2') format('woff2');
    font-weight: 700; font-style: normal; font-display: swap;
  }}
  @font-face {{
    font-family: 'Courier Prime';
    src: url('/_static/CourierPrime-Italic.woff2') format('woff2');
    font-weight: 400; font-style: italic; font-display: swap;
  }}
  * {{ margin: 0; padding: 0; box-sizing: border-box; }}
  body {{
    font-family: 'Courier Prime', 'Courier New', Courier, monospace;
    font-size: 12pt;
    line-height: 1;
    color: #1a1a1a;
    background: #c8c8c8;
  }}
  .print-wrap {{
    max-width: 8.5in;
    margin: 1rem auto;
    background: white;
    padding: 1in 1in 1in 1.5in;
    box-shadow: 0 2px 14px rgba(0,0,0,0.22);
  }}
  .scene-heading {{ font-weight: 700; text-transform: uppercase; margin-top: 2em; margin-bottom: 1em; }}
  .action {{ margin: 1em 0; }}
  .character {{ text-transform: uppercase; margin-left: 2in; margin-top: 1em; margin-bottom: 0; }}
  .dialog {{ margin: 0; padding: 0; }}
  .dialog .character {{ margin-top: 1em; }}
  .lines, .dialogue {{ margin-left: 1in; width: 3.5in; margin-top: 0; margin-bottom: 0; }}
  .paren, .parenthetical {{ margin-left: 1.6in; width: 2in; margin-top: 0; margin-bottom: 0; }}
  .trans, .transition {{ text-transform: uppercase; text-align: right; width: 100%; margin: 1em 0; }}
  .center {{ text-align: center; width: 100%; margin: 1em 0; }}
  hr.page-break {{ border: none; page-break-after: always; margin: 0; }}
  .bold {{ font-weight: 700; }}
  .italic {{ font-style: italic; }}
  .underline {{ text-decoration: underline; }}
  a.wikilink, a.wikilink-dead {{ color: inherit; text-decoration: none; }}
  /* Title page */
  .title-page {{
    display: flex; flex-direction: column; align-items: center;
    justify-content: center; min-height: 9in; text-align: center;
    page-break-after: always;
  }}
  .title-page h1 {{ font-size: 24pt; font-weight: 400; text-decoration: underline; margin-bottom: 1em; }}
  .title-page .credit {{ margin-top: 2em; }}
  .title-page .authors {{ margin-top: 0.5em; }}
  .title-page .source {{ margin-top: 0.5em; }}
  .title-page .draft-date, .title-page .date {{ margin-top: 2em; }}
  .title-page .contact {{ margin-top: auto; align-self: flex-start; text-align: left; }}
  .title-page .copyright {{ margin-top: 0.5em; align-self: flex-start; text-align: left; }}
  .title-page .notes {{ margin-top: 1em; font-style: italic; }}
  /* Keep character name + dialogue/parenthetical together across page breaks */
  .dialog {{ page-break-inside: avoid; }}
  .character {{ page-break-after: avoid; }}
  .scene-heading {{ page-break-after: avoid; page-break-before: auto; }}
  .scene-heading + .action {{ page-break-before: avoid; }}

  .title-page {{ counter-reset: page; }}
  @page {{
    size: letter;
    margin: 1in 1in 1in 1.5in;
    @bottom-right {{
      content: counter(page) ".";
      font-family: 'Courier Prime', 'Courier New', Courier, monospace;
      font-size: 12pt;
    }}
  }}
  @page :first {{
    @bottom-right {{ content: none; }}
  }}
  @media print {{
    body {{ background: white; }}
    .print-wrap {{ box-shadow: none; padding: 0; max-width: none; margin: 0; }}
    hr.page-break {{ page-break-after: always; }}
  }}
</style>
</head>
<body>
<div class="print-wrap">
{sections}
</div>

{fountain_js}

<script>
(function(){{
  document.querySelectorAll('.print-section').forEach(function(sec){{
    var src = sec.querySelector('script[type="text/fountain"]');
    var dest = sec.querySelector('.fountain-out');
    if(!src||!dest) return;
    var result = window.fountain.parse(src.textContent);
    var out = '';
    if(result.html.title_page){{
      var d=new Date();
      var months=['January','February','March','April','May','June','July','August','September','October','November','December'];
      var dateStr=months[d.getMonth()]+' '+d.getDate()+', '+d.getFullYear();
      out += '<div class="title-page">' + result.html.title_page + '<p class="draft-date">' + dateStr + '</p></div>';
    }}
    out += result.html.script;
    dest.innerHTML = out;
  }});
  setTimeout(function(){{ window.print(); }}, 600);
}})();
</script>
</body>
</html>"##,
        sections = sections.join("\n"),
        fountain_js = fountain_js,
    );

    Html(html).into_response()
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

/// Convert days since Unix epoch to (year, month, day).
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Algorithm from Howard Hinnant's chrono-compatible date math
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

// ─── History API handlers (REQ-087, CON-027, ADR-050) ─────────────────────────
//
// All four handlers are compiled only with `--features history`.

/// Query parameters for GET /api/history.
#[cfg(feature = "history")]
#[derive(Deserialize)]
pub struct HistoryLogParams {
    pub since: Option<String>,
    pub limit: Option<usize>,
}

/// Query parameters for GET /api/history/page/{name}.
#[cfg(feature = "history")]
#[derive(Deserialize)]
pub struct HistoryPageParams {
    pub limit: Option<usize>,
}

/// Query parameters for GET /api/history/at.
#[cfg(feature = "history")]
#[derive(Deserialize)]
pub struct HistoryAtParams {
    pub t: Option<String>,
}

/// Query parameters for GET /api/history/diff.
#[cfg(feature = "history")]
#[derive(Deserialize)]
pub struct HistoryDiffParams {
    pub from: Option<String>,
    pub to: Option<String>,
}

/// Map a history error message to the appropriate HTTP status code.
///
/// `NO_HISTORY`, `SNAPSHOT_NOT_FOUND`, and `PAGE_NOT_FOUND` error codes
/// map to 404; everything else maps to 500.
#[cfg(feature = "history")]
fn history_status_code(msg: &str) -> StatusCode {
    if msg.contains("NO_HISTORY")
        || msg.contains("SNAPSHOT_NOT_FOUND")
        || msg.contains("PAGE_NOT_FOUND")
    {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

/// Build a JSON error response for a history error.
#[cfg(feature = "history")]
fn history_err_response(err: anyhow::Error) -> Response {
    let msg = err.to_string();
    let status = history_status_code(&msg);
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}

/// GET /api/history — vault timeline (REQ-087, CON-027).
///
/// Query params: `since` (optional time expression), `limit` (optional, default 50).
/// Returns `Vec<HistoryEntry>` as JSON.
/// Returns 404 when history has never been initialised (NO_HISTORY).
#[cfg(feature = "history")]
pub async fn api_history_log_handler(
    State(state): State<WebState>,
    Query(params): Query<HistoryLogParams>,
) -> Response {
    let vault_root = state.vault_root.clone();
    let since = params.since.clone();
    let limit = params.limit.unwrap_or(50).max(1);

    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        use crate::history::core::build_vault_history;
        use crate::history::jj_backend::VcsBackend as _;
        use chrono::Local;

        let backend = crate::history::open_history(&vault_root)?;
        let snapshots = backend.list_changes(10_000)?;
        let now = Local::now().fixed_offset();
        build_vault_history(&snapshots, &vault_root, since.as_deref(), limit, now)
    })
    .await;

    match result {
        Ok(Ok(entries)) => Json(entries).into_response(),
        Ok(Err(e)) => history_err_response(e),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// GET /api/history/page/{name} — per-page evolution timeline (REQ-087, CON-027).
///
/// Path param: `name` (URL-decoded page name).
/// Query params: `limit` (optional, default 50).
/// Returns `Vec<PageHistoryEntry>` as JSON.
/// Returns 404 when NO_HISTORY or when the page is not found in any snapshot.
#[cfg(feature = "history")]
pub async fn api_history_page_handler(
    State(state): State<WebState>,
    Path(name): Path<String>,
    Query(params): Query<HistoryPageParams>,
) -> Response {
    let vault_root = state.vault_root.clone();
    let page_name = urldecode(&name);
    let limit = params.limit.unwrap_or(50).max(1);

    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        use crate::history::cache::HistoricalIndexCache;
        use crate::history::core::{
            extract_page_history, extract_vault_root_hash_from_description,
        };
        use crate::history::jj_backend::VcsBackend as _;

        let backend = crate::history::open_history(&vault_root)?;
        let snapshots = backend.list_changes(10_000)?;

        let cache = HistoricalIndexCache::with_default_capacity();
        let files_per_snapshot: Vec<Option<Vec<_>>> = snapshots
            .iter()
            .map(|snap| {
                let hash = extract_vault_root_hash_from_description(&snap.description)?;
                let file_map = cache.load(&vault_root, &hash).ok().flatten()?;
                Some(file_map.into_values().collect())
            })
            .collect();

        let entries = extract_page_history(&page_name, &snapshots, &files_per_snapshot, limit);

        if entries.is_empty() {
            anyhow::bail!("PAGE_NOT_FOUND: page '{page_name}' not found in any snapshot");
        }

        Ok(entries)
    })
    .await;

    match result {
        Ok(Ok(entries)) => Json(entries).into_response(),
        Ok(Err(e)) => history_err_response(e),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// GET /api/history/at — vault state at a point in time (REQ-087, CON-027).
///
/// Query params: `t` (required time expression).
/// Returns 400 when `t` is absent or blank.
/// Returns 404 when NO_HISTORY or SNAPSHOT_NOT_FOUND.
/// Returns 202 Accepted (non-blocking) when the historical index is not yet
/// cached for the resolved snapshot (ADR-050).
#[cfg(feature = "history")]
pub async fn api_history_at_handler(
    State(state): State<WebState>,
    Query(params): Query<HistoryAtParams>,
) -> Response {
    let t = match params.t.as_deref() {
        Some(s) if !s.trim().is_empty() => s.to_owned(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "missing required parameter 't'" })),
            )
                .into_response();
        }
    };

    let vault_root = state.vault_root.clone();

    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        use crate::history::cache::HistoricalIndexCache;
        use crate::history::core::{extract_vault_root_hash_from_description, resolve_snapshot};
        use crate::history::jj_backend::VcsBackend as _;
        use chrono::Local;

        let backend = crate::history::open_history(&vault_root)?;
        let snapshots = backend.list_changes(10_000)?;
        let now = Local::now().fixed_offset();
        let snap = resolve_snapshot(&t, now, &snapshots)?;

        let vault_root_hash = extract_vault_root_hash_from_description(&snap.description);
        let cache = HistoricalIndexCache::with_default_capacity();
        let pages = vault_root_hash.as_deref().and_then(|hash| {
            let file_map = cache.load(&vault_root, hash).ok().flatten()?;
            let mut pages: Vec<_> = file_map
                .into_values()
                .map(|f| {
                    serde_json::json!({
                        "page_name": f.page_name,
                        "link_count": f.links.len(),
                    })
                })
                .collect();
            pages.sort_by(|a, b| {
                a["page_name"]
                    .as_str()
                    .unwrap_or("")
                    .cmp(b["page_name"].as_str().unwrap_or(""))
            });
            Some(pages)
        });

        Ok((
            snap.change_id.clone(),
            snap.timestamp.to_rfc3339(),
            vault_root_hash,
            pages,
        ))
    })
    .await;

    match result {
        Ok(Ok((change_id, timestamp, vault_root_hash, Some(pages)))) => Json(serde_json::json!({
            "snapshot": {
                "change_id": change_id,
                "timestamp": timestamp,
                "vault_root_hash": vault_root_hash,
            },
            "status": "ok",
            "pages": pages,
        }))
        .into_response(),
        Ok(Ok((change_id, timestamp, vault_root_hash, None))) => {
            // Non-blocking cache miss: index not yet cached for this snapshot.
            (
                StatusCode::ACCEPTED,
                Json(serde_json::json!({
                    "snapshot": {
                        "change_id": change_id,
                        "timestamp": timestamp,
                        "vault_root_hash": vault_root_hash,
                    },
                    "status": "pending",
                    "pages": [],
                })),
            )
                .into_response()
        }
        Ok(Err(e)) => history_err_response(e),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// GET /api/history/diff — graph delta between two points in time (REQ-087, CON-027).
///
/// Query params: `from` (required), `to` (required time expressions).
/// Returns 400 when either param is absent.
/// Returns 404 when NO_HISTORY or SNAPSHOT_NOT_FOUND.
/// Returns 202 Accepted when cached index is unavailable for either endpoint.
#[cfg(feature = "history")]
pub async fn api_history_diff_handler(
    State(state): State<WebState>,
    Query(params): Query<HistoryDiffParams>,
) -> Response {
    let from_expr = match params.from.as_deref() {
        Some(s) if !s.trim().is_empty() => s.to_owned(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "missing required parameter 'from'" })),
            )
                .into_response();
        }
    };
    let to_expr = match params.to.as_deref() {
        Some(s) if !s.trim().is_empty() => s.to_owned(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "missing required parameter 'to'" })),
            )
                .into_response();
        }
    };

    let vault_root = state.vault_root.clone();

    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        use crate::history::cache::HistoricalIndexCache;
        use crate::history::core::{
            compute_graph_delta, extract_vault_root_hash_from_description, resolve_snapshot,
        };
        use crate::history::jj_backend::VcsBackend as _;
        use chrono::Local;

        let backend = crate::history::open_history(&vault_root)?;
        let snapshots = backend.list_changes(10_000)?;
        let now = Local::now().fixed_offset();

        let from_snap = resolve_snapshot(&from_expr, now, &snapshots)?;
        let to_snap = resolve_snapshot(&to_expr, now, &snapshots)?;

        let cache = HistoricalIndexCache::with_default_capacity();
        let from_hash = extract_vault_root_hash_from_description(&from_snap.description);
        let to_hash = extract_vault_root_hash_from_description(&to_snap.description);

        let from_files = from_hash
            .as_deref()
            .and_then(|h| cache.load(&vault_root, h).ok().flatten())
            .map(|m| m.into_values().collect::<Vec<_>>());
        let to_files = to_hash
            .as_deref()
            .and_then(|h| cache.load(&vault_root, h).ok().flatten())
            .map(|m| m.into_values().collect::<Vec<_>>());

        let delta = match (&from_files, &to_files) {
            (Some(from), Some(to)) => Some(compute_graph_delta(from, to)),
            _ => None,
        };

        Ok((
            from_snap.change_id.clone(),
            from_snap.timestamp.to_rfc3339(),
            to_snap.change_id.clone(),
            to_snap.timestamp.to_rfc3339(),
            delta,
        ))
    })
    .await;

    match result {
        Ok(Ok((from_id, from_ts, to_id, to_ts, Some(delta)))) => Json(serde_json::json!({
            "from": { "change_id": from_id, "timestamp": from_ts },
            "to":   { "change_id": to_id,   "timestamp": to_ts },
            "delta": delta,
        }))
        .into_response(),
        Ok(Ok((from_id, from_ts, to_id, to_ts, None))) => {
            // Non-blocking cache miss.
            (
                StatusCode::ACCEPTED,
                Json(serde_json::json!({
                    "from": { "change_id": from_id, "timestamp": from_ts },
                    "to":   { "change_id": to_id,   "timestamp": to_ts },
                    "status": "pending",
                })),
            )
                .into_response()
        }
        Ok(Err(e)) => history_err_response(e),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ── Passkey registration guidance routes (REQ-020-046) ──────────────────

/// GET /passkey/register — Passkey registration guidance screen.
pub async fn passkey_register_handler(
    State(state): State<WebState>,
    Query(params): Query<PasskeyRegisterParams>,
) -> Response {
    let vault_name = state
        .vault_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "vault".to_string());

    let user_id = params.user_id.as_deref().unwrap_or("");

    match state.engine.render_passkey_register(&vault_name, user_id) {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            eprintln!("{}", e.stderr_line("passkey/register"));
            (StatusCode::INTERNAL_SERVER_ERROR, Html(e.to_error_html())).into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct PasskeyRegisterParams {
    pub user_id: Option<String>,
}

/// POST /api/passkey/register/start — Begin passkey registration ceremony.
pub async fn passkey_register_start_handler(
    State(state): State<WebState>,
    Json(body): Json<PasskeyApiRequest>,
) -> Response {
    let vault_root = &*state.vault_root;

    let passkey_mgr = match crate::user::passkey::PasskeyManager::new(
        "localhost",
        "http://localhost:3000",
        "zetl vault",
    ) {
        Ok(mgr) => mgr,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to initialize passkey manager: {e}"),
            )
                .into_response();
        }
    };

    let profile = match crate::user::load_profile(vault_root, &body.user_id) {
        Ok(Some(p)) => p,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                format!("user not found: {}", body.user_id),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to load profile: {e}"),
            )
                .into_response();
        }
    };

    let existing =
        crate::user::passkey::load_passkeys(vault_root, &body.user_id).unwrap_or_default();

    match passkey_mgr.start_registration(&body.user_id, &profile.name, &existing) {
        Ok(ccr) => Json(ccr).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("registration start failed: {e}"),
        )
            .into_response(),
    }
}

/// POST /api/passkey/register/finish — Complete passkey registration ceremony.
pub async fn passkey_register_finish_handler(
    State(state): State<WebState>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let vault_root = &*state.vault_root;

    let user_id = match body.get("user_id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => {
            return (StatusCode::BAD_REQUEST, "missing user_id").into_response();
        }
    };

    let passkey_mgr = match crate::user::passkey::PasskeyManager::new(
        "localhost",
        "http://localhost:3000",
        "zetl vault",
    ) {
        Ok(mgr) => mgr,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to initialize passkey manager: {e}"),
            )
                .into_response();
        }
    };

    let reg_cred: webauthn_rs::prelude::RegisterPublicKeyCredential =
        match serde_json::from_value(body.clone()) {
            Ok(c) => c,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    format!("invalid credential response: {e}"),
                )
                    .into_response();
            }
        };

    match passkey_mgr.finish_registration(&user_id, &reg_cred) {
        Ok(passkey) => {
            let mut passkeys =
                crate::user::passkey::load_passkeys(vault_root, &user_id).unwrap_or_default();
            passkeys.push(passkey.clone());
            if let Err(e) = crate::user::passkey::save_passkeys(vault_root, &user_id, &passkeys) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("failed to save passkey: {e}"),
                )
                    .into_response();
            }

            // Update the profile credentials list
            let now = {
                let d = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default();
                let secs = d.as_secs();
                // Simple UTC ISO-8601 without chrono dependency
                let days = secs / 86400;
                let day_secs = secs % 86400;
                let h = day_secs / 3600;
                let m = (day_secs % 3600) / 60;
                let s = day_secs % 60;
                // Days since 1970-01-01 → Y/M/D (good enough for timestamps)
                let (y, mo, d) = days_to_ymd(days);
                format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
            };
            let cred = crate::user::passkey::passkey_to_credential(&passkey, None, &now);
            if let Ok(Some(mut profile)) = crate::user::load_profile(vault_root, &user_id) {
                profile.credentials.push(cred);
                let _ = crate::user::save_profile(vault_root, &profile);
            }

            (StatusCode::OK, "passkey registered").into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            format!("registration failed: {e}"),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct PasskeyApiRequest {
    pub user_id: String,
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
    use crate::search_index::SearchIndex;
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
        let search_index = SearchIndex::build(vault_root, &[]).unwrap();
        WebState {
            data: Arc::new(RwLock::new(data)),
            vault_root: Arc::new(vault_root.to_path_buf()),
            search_index: Arc::new(search_index),
            engine: Arc::new(TemplateEngine::new(vault_root, theme, false, false)),
            theme: theme.to_string(),
            verbose: false,
            sessions: crate::web::session::SessionStore::new(),
            #[cfg(feature = "semantic")]
            vector_index: None,
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

    // ── days_to_ymd tests ────────────────────────────────────────────────

    #[test]
    fn test_days_to_ymd_epoch() {
        // 1970-01-01
        assert_eq!(days_to_ymd(0), (1970, 1, 1));
    }

    #[test]
    fn test_days_to_ymd_known_date() {
        // 2026-03-18 is day 20530 since epoch
        assert_eq!(days_to_ymd(20530), (2026, 3, 18));
    }

    #[test]
    fn test_days_to_ymd_leap_year() {
        // 2024-02-29 is day 19782
        assert_eq!(days_to_ymd(19782), (2024, 2, 29));
    }

    // ── Passkey register handler tests ───────────────────────────────────

    #[tokio::test]
    async fn passkey_register_page_renders() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(tmp.path(), "default");

        let app = Router::new()
            .route("/passkey/register", axum::routing::get(passkey_register_handler))
            .with_state(state);

        let (status, body, _ct) = get_body(&app, "/passkey/register?user_id=alice-a1b2c3d4").await;
        assert_eq!(status, StatusCode::OK);
        let html = std::str::from_utf8(&body).unwrap();
        assert!(html.contains("Register a Passkey"));
        assert!(html.contains("alice-a1b2c3d4"));
        assert!(html.contains("pk-spinner"));
        assert!(html.contains("Try Again"));
        assert!(html.contains("recovery"));
    }

    #[tokio::test]
    async fn passkey_register_start_no_user() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(tmp.path(), "default");

        let app = Router::new()
            .route(
                "/api/passkey/register/start",
                axum::routing::post(passkey_register_start_handler),
            )
            .with_state(state);

        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/passkey/register/start")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                r#"{"user_id":"nonexistent-12345678"}"#,
            ))
            .unwrap();

        let resp = tower::ServiceExt::oneshot(app, req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
