use std::collections::HashSet;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use serde::Deserialize;

use crate::scanner::{body_text_ranges, page_slug_from_path};
use crate::search::{
    byte_offset_to_line_col, detect_headings, extract_search_context, find_heading_for_offset,
    in_body_text, SearchMatch, SearchOutput,
};
use crate::search_index::SearchIndex;
use crate::web::html::{
    breadcrumb_html, html_escape, layout, search_index_json, sidebar_html, urlencoding,
};
use crate::web::markdown;
use crate::web::{reindex, VaultData, WebState};

/// Build sidebar entries as `(display_name, slug)` tuples.
/// Display is just the page name if unique; `folder/Name` if there's a collision.
fn sidebar_entries(data: &VaultData) -> Vec<(String, String)> {
    let mut entries: Vec<(String, String)> = data
        .page_names
        .iter()
        .map(|name| {
            let slug = data
                .page_slug_map
                .get(name)
                .cloned()
                .unwrap_or_else(|| name.clone());
            let display = if data.collision_names.contains(name) {
                // Show parent folder for disambiguation
                if let Some(pos) = slug.rfind('/') {
                    if let Some(folder_start) = slug[..pos].rfind('/') {
                        slug[folder_start + 1..].to_string()
                    } else {
                        slug.clone()
                    }
                } else {
                    name.clone()
                }
            } else {
                name.clone()
            };
            (display, slug)
        })
        .collect();
    entries.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    entries
}

/// Look up the slug for a page name (case-insensitive).
fn slug_for_page(data: &VaultData, page_name: &str) -> String {
    data.page_slug_map
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(page_name))
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| page_name.to_string())
}

/// Render a folder index page showing all pages under a given folder prefix.
fn render_folder_index(
    data: &VaultData,
    folder_slug: &str,
    vault_name: &str,
    pages: &[&crate::types::ParsedFile],
) -> String {
    let folder_name = folder_slug.rsplit('/').next().unwrap_or(folder_slug);

    let mut grid = String::new();
    let mut sorted_pages: Vec<_> = pages.to_vec();
    sorted_pages.sort_by(|a, b| a.page_name.to_lowercase().cmp(&b.page_name.to_lowercase()));

    // Collect sub-folders under this folder
    let prefix = format!("{}/", folder_slug.to_lowercase());
    let mut subfolders: Vec<String> = Vec::new();
    let mut seen_subfolders = std::collections::HashSet::new();
    for page in &sorted_pages {
        let slug = page_slug_from_path(&page.path);
        let rest = &slug[prefix.len()..];
        if let Some(pos) = rest.find('/') {
            let subfolder = &rest[..pos];
            if seen_subfolders.insert(subfolder.to_string()) {
                subfolders.push(subfolder.to_string());
            }
        }
    }
    subfolders.sort();

    // Sub-folder cards
    for subfolder in &subfolders {
        let subfolder_slug = format!("{folder_slug}/{subfolder}");
        let count = sorted_pages
            .iter()
            .filter(|p| {
                let s = page_slug_from_path(&p.path).to_lowercase();
                s.starts_with(&format!("{}/", subfolder_slug.to_lowercase()))
            })
            .count();
        grid.push_str(&format!(
            r#"<a href="/{href}/" class="card bg-base-200 shadow-sm hover:shadow-md transition-shadow">
  <div class="card-body p-4">
    <h3 class="card-title text-sm">{name}/</h3>
    <p class="text-xs opacity-60">{count} pages</p>
  </div>
</a>"#,
            href = urlencoding(&subfolder_slug),
            name = html_escape(subfolder),
            count = count,
        ));
    }

    // Direct page cards (pages directly in this folder, not in sub-folders)
    for page in &sorted_pages {
        let slug = page_slug_from_path(&page.path);
        let rest = &slug[prefix.len()..];
        if rest.contains('/') {
            continue; // in a sub-folder
        }
        let fwd = data.graph.forward_links(&page.page_name).len();
        let back = data.graph.backlinks(&page.page_name).len();
        grid.push_str(&format!(
            r#"<a href="/{href}" class="card bg-base-200 shadow-sm hover:shadow-md transition-shadow">
  <div class="card-body p-4">
    <h3 class="card-title text-sm">{name}</h3>
    <p class="text-xs opacity-60">{fwd} out · {back} in</p>
  </div>
</a>"#,
            href = urlencoding(&slug),
            name = html_escape(&page.page_name),
            fwd = fwd,
            back = back,
        ));
    }

    let breadcrumb = breadcrumb_html(folder_slug, folder_name, vault_name);
    // For the folder index, the last breadcrumb segment is the folder itself (non-linked),
    // so we rebuild it slightly: all segments linked except the last
    let total = sorted_pages.len();

    let content = format!(
        r#"{breadcrumb}
<h1 class="text-3xl font-bold mb-6">{folder_name}</h1>
<p class="text-sm opacity-60 mb-4">{total} pages in this folder</p>
<div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-3">
  {grid}
</div>"#,
        breadcrumb = breadcrumb,
        folder_name = html_escape(folder_name),
        total = total,
        grid = grid,
    );

    let entries = sidebar_entries(data);
    let sidebar = sidebar_html(&entries, None);
    let si = search_index_json(&entries);
    layout(folder_name, &sidebar, &content, None, None, &si, false)
}

/// GET / — Landing page with vault stats and page grid.
pub async fn index_handler(State(state): State<WebState>) -> Html<String> {
    let data = state.data.read().unwrap();

    let total_pages = data.page_names.len();
    let total_links: usize = data.files.iter().map(|f| f.links.len()).sum();
    let dead_links = data.graph.dead_links().len();
    let orphans = data.graph.orphans().len();

    // Page grid
    let mut grid = String::new();
    for name in data.page_names.iter() {
        let slug = slug_for_page(&data, name);
        let fwd = data.graph.forward_links(name).len();
        let back = data.graph.backlinks(name).len();
        grid.push_str(&format!(
            r#"<a href="/{href}" class="card bg-base-200 shadow-sm hover:shadow-md transition-shadow">
  <div class="card-body p-4">
    <h3 class="card-title text-sm">{name}</h3>
    <p class="text-xs opacity-60">{fwd} out · {back} in</p>
  </div>
</a>"#,
            href = urlencoding(&slug),
            name = html_escape(name),
            fwd = fwd,
            back = back,
        ));
    }

    let content = format!(
        r#"<h1 class="text-3xl font-bold mb-6">Vault</h1>

<div class="stats shadow mb-8">
  <div class="stat">
    <div class="stat-title">Pages</div>
    <div class="stat-value text-primary">{total_pages}</div>
  </div>
  <div class="stat">
    <div class="stat-title">Links</div>
    <div class="stat-value">{total_links}</div>
  </div>
  <div class="stat">
    <div class="stat-title">Dead links</div>
    <div class="stat-value text-error">{dead_links}</div>
  </div>
  <div class="stat">
    <div class="stat-title">Orphans</div>
    <div class="stat-value text-warning">{orphans}</div>
  </div>
</div>

<h2 class="text-xl font-semibold mb-4">All Pages</h2>
<div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-3">
  {grid}
</div>"#,
    );

    let entries = sidebar_entries(&data);
    let sidebar = sidebar_html(&entries, None);
    let si = search_index_json(&entries);
    Html(layout("Vault", &sidebar, &content, None, None, &si, false))
}

/// GET /{*path} — Rendered markdown page with backlinks, or folder index.
pub async fn page_handler(State(state): State<WebState>, Path(slug): Path<String>) -> Html<String> {
    let slug = urldecode(&slug);
    // Strip trailing slash for folder index requests
    let slug = slug.trim_end_matches('/');
    let data = state.data.read().unwrap();

    // Find the file by matching slug (relative path without extension)
    let file = data
        .files
        .iter()
        .find(|f| page_slug_from_path(&f.path).eq_ignore_ascii_case(slug));

    // If no page matches, check if slug is a folder prefix → render folder index
    if file.is_none() {
        let folder_prefix = format!("{}/", slug.to_lowercase());
        let folder_pages: Vec<_> = data
            .files
            .iter()
            .filter(|f| {
                let s = page_slug_from_path(&f.path);
                s.to_lowercase().starts_with(&folder_prefix)
            })
            .collect();

        if !folder_pages.is_empty() {
            let vault_name = state
                .vault_root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "vault".to_string());
            let html = render_folder_index(&data, slug, &vault_name, &folder_pages);
            return Html(html);
        }
    }

    let (rendered, page_name, current_slug, raw_escaped) = if let Some(file) = file {
        let full_path = state.vault_root.join(&file.path);
        let file_slug = page_slug_from_path(&file.path);
        match std::fs::read_to_string(&full_path) {
            Ok(content) => {
                let html = markdown::render_to_html(&content, &data.page_slug_map);
                let escaped = html_escape(&content);
                (html, file.page_name.clone(), file_slug, Some(escaped))
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

    // Auto-open edit mode for new (empty) pages
    let is_new_page = file.is_none();

    // Backlinks
    let backlinks = data.graph.backlinks(&page_name);
    let mut backlinks_html = String::new();
    if !backlinks.is_empty() {
        backlinks_html.push_str(r#"<div class="divider"></div><h2 class="text-lg font-semibold mb-2">Backlinks</h2><ul class="list-none space-y-1">"#);
        for bl in &backlinks {
            let bl_slug = slug_for_page(&data, &bl.source);
            backlinks_html.push_str(&format!(
                r#"<li><a href="/{href}#line-{line}" class="link link-secondary">{source}</a><span class="text-xs opacity-50 ml-2">line {line}</span></li>"#,
                href = urlencoding(&bl_slug),
                source = html_escape(&bl.source),
                line = bl.line,
            ));
        }
        backlinks_html.push_str("</ul>");
    }

    // Transclusion panel: forward-link excerpt cards
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
        let target_slug = slug_for_page(&data, target);
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

    let right_panel = if transclusion_cards.is_empty() {
        None
    } else {
        Some(transclusion_cards.as_str())
    };

    let (view_display, edit_display) = if is_new_page {
        ("style=\"display:none\"", "")
    } else {
        ("", "style=\"display:none\"")
    };

    // Edit mode UI (only shown for real files)
    let edit_ui = if let Some(ref raw) = raw_escaped {
        format!(
            r#"<div id="edit-mode" {edit_display}>
  <div class="flex gap-2 mb-4">
    <button onclick="saveEdit()" class="btn btn-sm btn-primary">Save</button>
    <button onclick="toggleEdit()" class="btn btn-sm btn-outline">Cancel</button>
  </div>
  <textarea id="editor" class="textarea textarea-bordered w-full font-mono"
            style="min-height:80vh">{raw}</textarea>
</div>"#,
        )
    } else {
        String::new()
    };

    let edit_button = if raw_escaped.is_some() {
        r#"<div class="flex justify-end mb-4"><button onclick="toggleEdit()" class="btn btn-sm btn-outline">Edit</button></div>"#
    } else {
        ""
    };

    let vault_name = state
        .vault_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "vault".to_string());
    let breadcrumb = breadcrumb_html(&current_slug, &page_name, &vault_name);

    #[allow(clippy::format_in_format_args)]
    let content = format!(
        r#"{breadcrumb}
<div id="view-mode" {view_display}>
{edit_button}
<article class="prose prose-lg max-w-none">{rendered}</article>
{backlinks_html}
</div>
{edit_ui}
<script>
function toggleEdit() {{
  var vm = document.getElementById('view-mode');
  var em = document.getElementById('edit-mode');
  if (em.style.display === 'none') {{
    em.style.display = 'block';
    vm.style.display = 'none';
  }} else {{
    em.style.display = 'none';
    vm.style.display = 'block';
  }}
}}
async function saveEdit() {{
  var content = document.getElementById('editor').value;
  var res = await fetch(window.location.pathname, {{
    method: 'PUT',
    headers: {{ 'Content-Type': 'text/plain; charset=utf-8' }},
    body: content
  }});
  if (res.ok) window.location.reload();
  else alert('Save failed: ' + res.status);
}}
</script>
<script>
// Transclusion: SVG bridge lines + bidirectional hover
(function() {{
  const COLORS = {colors_json};
  const cards = document.querySelectorAll('.transclusion-card');
  const colorMap = {{}};
  cards.forEach((card, i) => {{
    colorMap[card.dataset.targetHref] = COLORS[i % COLORS.length];
  }});

  // Apply matching underline colors to content wikilinks
  document.querySelectorAll('a.wikilink:not(.wikilink-dead)').forEach(link => {{
    const color = colorMap[link.getAttribute('href')];
    if (color) {{
      link.style.textDecorationColor = color;
    }}
  }});

  // SVG overlay for bridge lines
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.style.cssText = 'position:fixed;top:0;left:0;width:100%;height:100%;pointer-events:none;z-index:40;';
  document.body.appendChild(svg);

  function drawBridge(fromEl, toEl, color) {{
    svg.innerHTML = '';
    const fr = fromEl.getBoundingClientRect();
    const tr = toEl.getBoundingClientRect();
    const x1 = fr.right + 4;
    const y1 = fr.top + fr.height / 2;
    const x2 = tr.left - 2;
    const y2 = tr.top + 16;
    const mx = (x1 + x2) / 2;

    const path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
    path.setAttribute('d',
      'M ' + x1 + ' ' + y1 + ' C ' + mx + ' ' + y1 + ', ' + mx + ' ' + y2 + ', ' + x2 + ' ' + y2);
    path.setAttribute('stroke', color);
    path.setAttribute('stroke-width', '2');
    path.setAttribute('fill', 'none');
    path.setAttribute('opacity', '0.6');
    svg.appendChild(path);

    var d1 = document.createElementNS('http://www.w3.org/2000/svg', 'circle');
    d1.setAttribute('cx', x1); d1.setAttribute('cy', y1);
    d1.setAttribute('r', '3'); d1.setAttribute('fill', color);
    d1.setAttribute('opacity', '0.6');
    svg.appendChild(d1);

    var d2 = document.createElementNS('http://www.w3.org/2000/svg', 'circle');
    d2.setAttribute('cx', x2); d2.setAttribute('cy', y2);
    d2.setAttribute('r', '3'); d2.setAttribute('fill', color);
    d2.setAttribute('opacity', '0.6');
    svg.appendChild(d2);
  }}

  function clearBridge() {{
    svg.innerHTML = '';
  }}

  const isDesktop = () => window.matchMedia('(min-width: 1280px)').matches;
  const isMobile = () => !isDesktop();

  // Hover wikilink → draw bridge line + expand card
  document.querySelectorAll('a.wikilink').forEach(link => {{
    const href = link.getAttribute('href');
    const color = colorMap[href] || '#888';
    link.addEventListener('mouseenter', () => {{
      const card = document.querySelector('.transclusion-card[data-target-href="' + href + '"]');
      if (card) {{
        card.classList.add('tc-active');
        if (isDesktop()) drawBridge(link, card, color);
      }}
    }});
    link.addEventListener('mouseleave', () => {{
      document.querySelectorAll('.transclusion-card.tc-active').forEach(c => c.classList.remove('tc-active'));
      clearBridge();
    }});
  }});

  // Hover card → highlight matching wikilinks + expand card
  cards.forEach(card => {{
    const href = card.dataset.targetHref;
    card.addEventListener('mouseenter', () => {{
      card.classList.add('tc-active');
      document.querySelectorAll('a.wikilink[href="' + href + '"]').forEach(l => l.classList.add('wl-active'));
    }});
    card.addEventListener('mouseleave', () => {{
      card.classList.remove('tc-active');
      document.querySelectorAll('a.wl-active').forEach(l => l.classList.remove('wl-active'));
    }});
  }});

  // ── Mobile tooltip preview ──────────────────────────────────────
  if (isMobile() && cards.length) {{
    var tooltip = document.createElement('div');
    tooltip.className = 'wikilink-tooltip';
    tooltip.innerHTML =
      '<div class="tt-header"><a class="tt-title" href="">Title</a><button class="tt-close">&times;</button></div>' +
      '<div class="tt-body prose prose-sm max-w-none"></div>' +
      '<div class="tt-footer"><a class="tt-go" href="">Go to page &rarr;</a></div>';
    document.body.appendChild(tooltip);

    var ttTitle = tooltip.querySelector('.tt-title');
    var ttBody  = tooltip.querySelector('.tt-body');
    var ttGo    = tooltip.querySelector('.tt-go');
    var ttClose = tooltip.querySelector('.tt-close');
    var activeLink = null;

    function showTooltip(linkEl, href) {{
      var card = document.querySelector('.transclusion-card[data-target-href="' + href + '"]');
      if (!card) return;
      var excerpt = card.querySelector('.tc-excerpt');
      if (!excerpt) return;
      var title = card.querySelector('.tc-title');
      ttTitle.textContent = title ? title.textContent : href;
      ttTitle.href = href;
      ttBody.innerHTML = excerpt.innerHTML;
      ttGo.href = href;
      var rect = linkEl.getBoundingClientRect();
      var below = rect.bottom + 8;
      var above = rect.top - 8;
      tooltip.style.bottom = 'auto';
      tooltip.style.top = 'auto';
      if (below + window.innerHeight * 0.4 < window.innerHeight) {{
        tooltip.style.top = below + 'px';
      }} else {{
        tooltip.style.bottom = (window.innerHeight - above) + 'px';
      }}
      tooltip.classList.add('tt-visible');
      activeLink = linkEl;
    }}

    function hideTooltip() {{
      tooltip.classList.remove('tt-visible');
      activeLink = null;
    }}

    ttClose.addEventListener('click', function(e) {{ e.preventDefault(); hideTooltip(); }});

    document.addEventListener('click', function(e) {{
      if (tooltip.contains(e.target)) return;
      var link = e.target.closest('a.wikilink:not(.wikilink-dead)');
      if (link) {{
        var href = link.getAttribute('href');
        if (activeLink === link && tooltip.classList.contains('tt-visible')) {{
          return; // second tap — navigate normally
        }}
        var card = document.querySelector('.transclusion-card[data-target-href="' + href + '"]');
        if (card) {{
          e.preventDefault();
          showTooltip(link, href);
          return;
        }}
      }}
      if (tooltip.classList.contains('tt-visible')) {{
        hideTooltip();
      }}
    }});

    document.addEventListener('keydown', function(e) {{
      if (e.key === 'Escape') hideTooltip();
    }});
  }}
}})();
</script>"#,
        view_display = view_display,
        edit_button = edit_button,
        rendered = rendered,
        backlinks_html = backlinks_html,
        edit_ui = edit_ui,
        colors_json = format!(
            "[{}]",
            colors
                .iter()
                .map(|c| format!("\"{c}\""))
                .collect::<Vec<_>>()
                .join(",")
        ),
    );

    let entries = sidebar_entries(&data);
    let sidebar = sidebar_html(&entries, Some(&current_slug));
    let si = search_index_json(&entries);
    Html(layout(
        &page_name,
        &sidebar,
        &content,
        Some(&current_slug),
        right_panel,
        &si,
        false,
    ))
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
