use std::collections::HashSet;

use axum::extract::{Path, State};
use axum::response::Html;

use crate::web::html::{html_escape, layout, sidebar_html, urlencoding};
use crate::web::markdown;
use crate::web::WebState;

/// GET / — Landing page with vault stats and page grid.
pub async fn index_handler(State(state): State<WebState>) -> Html<String> {
    let total_pages = state.page_names.len();
    let total_links: usize = state.files.iter().map(|f| f.links.len()).sum();
    let dead_links = state.graph.dead_links().len();
    let orphans = state.graph.orphans().len();

    // Page grid
    let mut grid = String::new();
    for name in state.page_names.iter() {
        let fwd = state.graph.forward_links(name).len();
        let back = state.graph.backlinks(name).len();
        grid.push_str(&format!(
            r#"<a href="/page/{href}" class="card bg-base-200 shadow-sm hover:shadow-md transition-shadow">
  <div class="card-body p-4">
    <h3 class="card-title text-sm">{name}</h3>
    <p class="text-xs opacity-60">{fwd} out · {back} in</p>
  </div>
</a>"#,
            href = urlencoding(name),
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
        total_pages = total_pages,
        total_links = total_links,
        dead_links = dead_links,
        orphans = orphans,
        grid = grid,
    );

    let sidebar = sidebar_html(&state.page_names, None);
    Html(layout("Vault", &sidebar, &content, None, None))
}

/// GET /page/:page_name — Rendered markdown page with backlinks.
pub async fn page_handler(
    State(state): State<WebState>,
    Path(page_name): Path<String>,
) -> Html<String> {
    let page_name = urldecode(&page_name);

    // Find the file
    let file = state
        .files
        .iter()
        .find(|f| f.page_name.eq_ignore_ascii_case(&page_name));

    let (rendered, title) = if let Some(file) = file {
        let full_path = state.vault_root.join(&file.path);
        match std::fs::read_to_string(&full_path) {
            Ok(content) => {
                let html = markdown::render_to_html(&content, &state.resolved);
                (html, file.page_name.clone())
            }
            Err(_) => (
                "<p class=\"text-error\">Could not read file.</p>".to_string(),
                page_name.clone(),
            ),
        }
    } else {
        (
            format!(
                "<div class=\"alert alert-warning\"><span>Page <strong>{}</strong> does not exist (phantom link target).</span></div>",
                html_escape(&page_name)
            ),
            page_name.clone(),
        )
    };

    // Backlinks
    let backlinks = state.graph.backlinks(&title);
    let mut backlinks_html = String::new();
    if !backlinks.is_empty() {
        backlinks_html.push_str(r#"<div class="divider"></div><h2 class="text-lg font-semibold mb-2">Backlinks</h2><ul class="list-none space-y-1">"#);
        for bl in &backlinks {
            backlinks_html.push_str(&format!(
                r#"<li><a href="/page/{href}" class="link link-secondary">{source}</a><span class="text-xs opacity-50 ml-2">line {line}</span></li>"#,
                href = urlencoding(&bl.source),
                source = html_escape(&bl.source),
                line = bl.line,
            ));
        }
        backlinks_html.push_str("</ul>");
    }

    // Transclusion panel: forward-link excerpt cards
    let forward_links = state.graph.forward_links(&title);
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
        let href = urlencoding(target);

        let preview_html = state
            .files
            .iter()
            .find(|f| f.page_name.eq_ignore_ascii_case(target))
            .and_then(|file| {
                let full_path = state.vault_root.join(&file.path);
                std::fs::read_to_string(&full_path).ok()
            })
            .map(|content| markdown::render_preview_html(&content, &state.resolved))
            .unwrap_or_else(|| format!("<p><em>{}</em></p>", html_escape("(page does not exist)")));

        transclusion_cards.push_str(&format!(
            r#"<div class="transclusion-card" data-target-href="/page/{href}" style="border-left-color: {color};">
  <a href="/page/{href}" class="tc-title" style="color: {color};">{name}</a>
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

    let content = format!(
        r#"<article class="prose prose-lg max-w-none">{rendered}</article>
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

  // Hover wikilink → draw bridge line + expand card
  document.querySelectorAll('a.wikilink').forEach(link => {{
    const href = link.getAttribute('href');
    const color = colorMap[href] || '#888';
    link.addEventListener('mouseenter', () => {{
      const card = document.querySelector('.transclusion-card[data-target-href="' + href + '"]');
      if (card) {{
        card.classList.add('tc-active');
        drawBridge(link, card, color);
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
}})();
</script>"#,
        rendered = rendered,
        backlinks_html = backlinks_html,
        colors_json = format!(
            "[{}]",
            colors
                .iter()
                .map(|c| format!("\"{}\"", c))
                .collect::<Vec<_>>()
                .join(",")
        ),
    );

    let sidebar = sidebar_html(&state.page_names, Some(&title));
    Html(layout(&title, &sidebar, &content, Some(&title), right_panel))
}

/// GET /preview/:page_name — Returns a short HTML preview (for tooltip).
pub async fn preview_handler(
    State(state): State<WebState>,
    Path(page_name): Path<String>,
) -> Html<String> {
    let page_name = urldecode(&page_name);

    let file = state
        .files
        .iter()
        .find(|f| f.page_name.eq_ignore_ascii_case(&page_name));

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
            html_escape(&page_name)
        )
    };

    Html(preview)
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
