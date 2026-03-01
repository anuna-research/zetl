use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};

use crate::scanner::{body_text_ranges, page_slug_from_path};
use crate::web::html::{
    breadcrumb_html, html_escape, layout, search_index_json, sidebar_html, urlencoding,
};
use crate::web::markdown;
use crate::web::VaultData;

/// Build sidebar entries as `(display_name, slug)` tuples for static build.
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

/// Tokenize body text and count term occurrences.
///
/// Mirrors Tantivy's default tokenizer: lowercase, split on non-alphanumeric characters.
fn tokenize_and_count(text: &str) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for token in text.split(|c: char| !c.is_alphanumeric()) {
        if token.is_empty() {
            continue;
        }
        let lower = token.to_lowercase();
        *counts.entry(lower).or_insert(0) += 1;
    }
    counts
}

/// Write `{out_dir}/search-index.json` with BM25-style corpus statistics.
///
/// Schema per CON-013-004: avgDl, docs[{n,s,dl,tf}], df.
/// REQ-013-014, ADR-013-004.
fn write_search_index_json(data: &VaultData, vault_root: &Path, out_dir: &Path) -> Result<()> {
    let mut doc_entries: Vec<serde_json::Value> = Vec::with_capacity(data.files.len());
    let mut df: HashMap<String, usize> = HashMap::new();
    let mut total_dl: usize = 0;

    for file in &data.files {
        let slug = data
            .page_slug_map
            .get(&file.page_name)
            .cloned()
            .unwrap_or_else(|| page_slug_from_path(&file.path));

        let full_path = vault_root.join(&file.path);
        let content = std::fs::read_to_string(&full_path).unwrap_or_default();

        // Extract body text using the same exclusions as the Tantivy indexer
        // (no frontmatter, fenced code blocks, inline code, HTML comments).
        let body: String = body_text_ranges(&content)
            .iter()
            .map(|&(start, end)| &content[start..end])
            .collect::<Vec<_>>()
            .join(" ");

        let tf = tokenize_and_count(&body);
        let dl: usize = tf.values().sum();
        total_dl += dl;

        for term in tf.keys() {
            *df.entry(term.clone()).or_insert(0) += 1;
        }

        let tf_json: serde_json::Map<String, serde_json::Value> = tf
            .into_iter()
            .map(|(k, v)| (k, serde_json::Value::Number(v.into())))
            .collect();

        doc_entries.push(serde_json::json!({
            "n": file.page_name,
            "s": slug,
            "dl": dl,
            "tf": tf_json,
        }));
    }

    let avg_dl: f64 = if data.files.is_empty() {
        0.0
    } else {
        total_dl as f64 / data.files.len() as f64
    };

    let df_json: serde_json::Map<String, serde_json::Value> = df
        .into_iter()
        .map(|(k, v)| (k, serde_json::Value::Number(v.into())))
        .collect();

    let index = serde_json::json!({
        "avgDl": avg_dl,
        "docs": doc_entries,
        "df": df_json,
    });

    let json_str = serde_json::to_string(&index).context("serializing search-index.json")?;
    std::fs::write(out_dir.join("search-index.json"), json_str)
        .context("writing search-index.json")?;

    Ok(())
}

/// Generate a complete static HTML site from the vault data.
pub fn build_static(data: &VaultData, vault_root: &Path, out_dir: &str) -> Result<()> {
    let out = Path::new(out_dir);
    std::fs::create_dir_all(out)
        .with_context(|| format!("Cannot create output directory: {out_dir}"))?;

    let entries = sidebar_entries(data);
    let sidebar = sidebar_html(&entries, None);
    let si = search_index_json(&entries);

    // ── search-index.json ────────────────────────────────────────────────
    write_search_index_json(data, vault_root, out)?;

    // ── index.html ──────────────────────────────────────────────────────
    let index_html = render_index(data, &sidebar, &si);
    std::fs::write(out.join("index.html"), index_html)?;

    // ── per-page HTML ───────────────────────────────────────────────────
    let mut count = 0usize;
    for file in &data.files {
        let slug = page_slug_from_path(&file.path);
        let page_dir = out.join(&slug);
        std::fs::create_dir_all(&page_dir)?;

        let full_path = vault_root.join(&file.path);
        let content = std::fs::read_to_string(&full_path)
            .with_context(|| format!("Cannot read {}", full_path.display()))?;

        let page_sidebar = sidebar_html(&entries, Some(&slug));
        let page_html = render_page(
            data,
            vault_root,
            &file.page_name,
            &slug,
            &content,
            &page_sidebar,
            &si,
        );
        std::fs::write(page_dir.join("index.html"), page_html)?;
        count += 1;
    }

    // ── folder index pages ─────────────────────────────────────────────
    let vault_name = vault_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "vault".to_string());
    let mut folders: HashSet<String> = HashSet::new();
    for file in &data.files {
        let slug = page_slug_from_path(&file.path);
        // Collect all ancestor folder prefixes
        let mut pos = 0;
        while let Some(sep) = slug[pos..].find('/') {
            let folder = &slug[..pos + sep];
            folders.insert(folder.to_string());
            pos += sep + 1;
        }
    }

    let mut folder_count = 0usize;
    for folder in &folders {
        let folder_prefix = format!("{}/", folder.to_lowercase());
        let folder_pages: Vec<_> = data
            .files
            .iter()
            .filter(|f| {
                let s = page_slug_from_path(&f.path);
                s.to_lowercase().starts_with(&folder_prefix)
            })
            .collect();

        if folder_pages.is_empty() {
            continue;
        }

        let folder_dir = out.join(folder);
        std::fs::create_dir_all(&folder_dir)?;

        let folder_sidebar = sidebar_html(&entries, None);
        let folder_html = render_folder_index(
            data,
            folder,
            &vault_name,
            &folder_pages,
            &folder_sidebar,
            &si,
        );
        std::fs::write(folder_dir.join("index.html"), folder_html)?;
        folder_count += 1;
    }

    eprintln!("zetl build  →  {count} pages + {folder_count} folder indexes written to {out_dir}/");
    Ok(())
}

/// Render the landing page (mirrors `index_handler` in routes.rs).
fn render_index(data: &VaultData, sidebar: &str, search_index: &str) -> String {
    let total_pages = data.page_names.len();
    let total_links: usize = data.files.iter().map(|f| f.links.len()).sum();
    let dead_links = data.graph.dead_links().len();
    let orphans = data.graph.orphans().len();

    let mut grid = String::new();
    for name in data.page_names.iter() {
        let slug = slug_for_page(data, name);
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

    layout("Vault", sidebar, &content, None, None, search_index, true)
}

/// Render a folder index page for static build.
fn render_folder_index(
    data: &VaultData,
    folder_slug: &str,
    vault_name: &str,
    pages: &[&crate::types::ParsedFile],
    sidebar: &str,
    search_index: &str,
) -> String {
    let folder_name = folder_slug.rsplit('/').next().unwrap_or(folder_slug);

    let mut sorted_pages: Vec<_> = pages.to_vec();
    sorted_pages.sort_by(|a, b| a.page_name.to_lowercase().cmp(&b.page_name.to_lowercase()));

    let prefix = format!("{}/", folder_slug.to_lowercase());
    let mut subfolders: Vec<String> = Vec::new();
    let mut seen_subfolders = HashSet::new();
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

    let mut grid = String::new();

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

    for page in &sorted_pages {
        let slug = page_slug_from_path(&page.path);
        let rest = &slug[prefix.len()..];
        if rest.contains('/') {
            continue;
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

    layout(
        folder_name,
        sidebar,
        &content,
        None,
        None,
        search_index,
        true,
    )
}

/// Render a single page (mirrors `page_handler` in routes.rs, minus edit UI).
fn render_page(
    data: &VaultData,
    vault_root: &Path,
    page_name: &str,
    current_slug: &str,
    raw_content: &str,
    sidebar: &str,
    search_index: &str,
) -> String {
    let rendered = markdown::render_to_html(raw_content, &data.page_slug_map);

    // Backlinks
    let backlinks = data.graph.backlinks(page_name);
    let mut backlinks_html = String::new();
    if !backlinks.is_empty() {
        backlinks_html.push_str(r#"<div class="divider"></div><h2 class="text-lg font-semibold mb-2">Backlinks</h2><ul class="list-none space-y-1">"#);
        for bl in &backlinks {
            let bl_slug = slug_for_page(data, &bl.source);
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
    let forward_links = data.graph.forward_links(page_name);
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
        let target_slug = slug_for_page(data, target);
        let href = urlencoding(&target_slug);

        let preview_html = data
            .files
            .iter()
            .find(|f| f.page_name.eq_ignore_ascii_case(target))
            .and_then(|file| {
                let full_path = vault_root.join(&file.path);
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

    let colors_json = format!(
        "[{}]",
        colors
            .iter()
            .map(|c| format!("\"{c}\""))
            .collect::<Vec<_>>()
            .join(",")
    );

    let vault_name = vault_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "vault".to_string());
    let breadcrumb = breadcrumb_html(current_slug, page_name, &vault_name);

    // Static page: no edit button, no edit-mode div, no toggleEdit/saveEdit JS.
    // Keep: transclusion bridge SVG JS, wikilink colors, line anchors.
    let content = format!(
        r#"{breadcrumb}
<article class="prose prose-lg max-w-none">{rendered}</article>
{backlinks_html}
<script>
// Transclusion: SVG bridge lines + bidirectional hover
(function() {{
  const COLORS = {colors_json};
  const cards = document.querySelectorAll('.transclusion-card');
  const colorMap = {{}};
  cards.forEach((card, i) => {{
    colorMap[card.dataset.targetHref] = COLORS[i % COLORS.length];
  }});

  document.querySelectorAll('a.wikilink:not(.wikilink-dead)').forEach(link => {{
    const color = colorMap[link.getAttribute('href')];
    if (color) {{
      link.style.textDecorationColor = color;
    }}
  }});

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
    );

    layout(
        page_name,
        sidebar,
        &content,
        Some(current_slug),
        right_panel,
        search_index,
        true,
    )
}
