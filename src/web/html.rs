/// Build compact JSON search index: `[{"n":"Page Name","s":"path/slug"},...]`
pub fn search_index_json(entries: &[(String, String)]) -> String {
    let items: Vec<String> = entries
        .iter()
        .map(|(name, slug)| {
            let n = name.replace('\\', "\\\\").replace('"', "\\\"");
            let s = slug.replace('\\', "\\\\").replace('"', "\\\"");
            format!(r#"{{"n":"{}","s":"{}"}}"#, n, s)
        })
        .collect();
    format!("[{}]", items.join(","))
}

/// Wrap body HTML in the DaisyUI shell with sidebar and optional transclusion panel.
pub fn layout(
    title: &str,
    sidebar: &str,
    content: &str,
    active_page: Option<&str>,
    right_panel: Option<&str>,
    search_index: &str,
) -> String {
    let _ = active_page; // used by sidebar_html to highlight

    let main_section = if let Some(panel) = right_panel {
        format!(
            r#"<div class="page-with-panel flex-1">
        <main class="flex-1 p-4 sm:p-6 min-w-0">
          {content}
        </main>
        <aside class="transclusion-panel">
          <h3 class="tp-header">Linked Pages</h3>
          {panel}
        </aside>
      </div>"#,
            content = content,
            panel = panel,
        )
    } else {
        format!(
            r#"<main class="flex-1 p-4 sm:p-6 max-w-4xl mx-auto w-full">
        {content}
      </main>"#,
            content = content,
        )
    };

    format!(
        r#"<!DOCTYPE html>
<html lang="en" data-theme="emerald">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{title} — zetl</title>
  <script type="application/json" id="zetl-search-index">{search_index}</script>
  <link href="https://cdn.jsdelivr.net/npm/daisyui@4/dist/full.min.css" rel="stylesheet">
  <script src="https://cdn.tailwindcss.com?plugins=typography"></script>
  <style>
    /* dead link */
    a.link-error {{ color: oklch(var(--er)); text-decoration: underline wavy; }}
    /* line anchors for backlink scroll targets */
    .line-anchor {{ scroll-margin-top: 2rem; }}
    :has(> .line-anchor:target) {{
      animation: line-highlight 2s ease-out;
      border-radius: 4px;
    }}
    @keyframes line-highlight {{
      0%   {{ background: oklch(var(--wa) / 0.35); }}
      100% {{ background: transparent; }}
    }}

    /* page + transclusion wrapper: stacked on mobile, side-by-side on desktop */
    .page-with-panel {{
      display: flex;
      flex-direction: column;
    }}
    @media (min-width: 1280px) {{
      .page-with-panel {{ flex-direction: row; }}
    }}

    /* transclusion panel — mobile: inline below content */
    .transclusion-panel {{
      border-top: 1px solid oklch(var(--b3));
      padding: 1rem;
      background: oklch(var(--b1));
    }}
    .transclusion-panel .transclusion-card .tc-excerpt {{
      display: block;
    }}
    .transclusion-panel .transclusion-card {{
      margin-bottom: 0.75rem;
    }}
    /* transclusion panel — desktop: sticky sidebar */
    @media (min-width: 1280px) {{
      .transclusion-panel {{
        width: 36rem;
        flex-shrink: 0;
        border-top: none;
        border-left: 1px solid oklch(var(--b3));
        overflow-y: auto;
        position: sticky;
        top: 0;
        max-height: 100vh;
      }}
      .transclusion-panel .transclusion-card .tc-excerpt {{
        display: none;
      }}
      .transclusion-panel .transclusion-card.tc-active .tc-excerpt {{
        display: block;
      }}
    }}
    /* stats: horizontal scroll on small screens */
    .stats {{ overflow-x: auto; }}
    .tp-header {{
      font-size: 0.65rem;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.1em;
      opacity: 0.4;
      margin-bottom: 1rem;
    }}
    .transclusion-card {{
      border-left: 3px solid transparent;
      padding: 0.4rem 0.75rem;
      margin-bottom: 0.25rem;
      border-radius: 0.375rem;
      transition: background 0.15s ease, box-shadow 0.15s ease;
    }}
    .transclusion-card:hover,
    .transclusion-card.tc-active {{
      background: oklch(var(--b2) / 0.5);
      box-shadow: 0 0 0 1px oklch(var(--b3) / 0.4);
      padding: 0.6rem 0.75rem;
      margin-bottom: 0.5rem;
    }}
    .transclusion-card .tc-title {{
      font-size: 0.85rem;
      font-weight: 600;
      text-decoration: none;
    }}
    .transclusion-card .tc-title:hover {{
      text-decoration: underline;
    }}
    .transclusion-card .tc-excerpt {{
      display: none;
      margin-top: 0.5rem;
    }}
    .transclusion-card .tc-excerpt.prose {{
      font-size: 0.85rem;
      line-height: 1.6;
    }}
    .transclusion-card.tc-active .tc-excerpt {{
      display: block;
    }}
    /* Mobile transclusion panel: stacked below content, all excerpts visible */
    @media (max-width: 1279px) {{
      .transclusion-panel {{
        border-top: 2px solid oklch(var(--b3));
        margin-top: 2rem;
      }}
      .transclusion-panel .transclusion-card .tc-excerpt {{
        display: block;
      }}
    }}

    /* wikilink color underline */
    a.wikilink {{
      text-decoration-thickness: 2px;
      text-underline-offset: 3px;
      transition: background 0.12s ease;
    }}
    a.wikilink.wl-active {{
      background: rgba(0,0,0,0.08);
      border-radius: 2px;
      padding: 1px 3px;
      margin: 0 -3px;
    }}

    /* Mobile link-preview tooltip */
    .wikilink-tooltip {{
      position: fixed;
      left: 1rem; right: 1rem;
      max-height: 60vh;
      background: oklch(var(--b1));
      border: 1px solid oklch(var(--b3));
      border-radius: 0.75rem;
      box-shadow: 0 8px 30px rgba(0,0,0,0.18);
      z-index: 50;
      display: flex;
      flex-direction: column;
      opacity: 0;
      pointer-events: none;
      transition: opacity 0.15s ease;
    }}
    .wikilink-tooltip.tt-visible {{
      opacity: 1;
      pointer-events: auto;
    }}
    .tt-header {{
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: 0.75rem 1rem;
      border-bottom: 1px solid oklch(var(--b3));
      position: sticky; top: 0;
      background: oklch(var(--b1));
      border-radius: 0.75rem 0.75rem 0 0;
    }}
    .tt-header a {{
      font-weight: 600;
      font-size: 0.95rem;
      text-decoration: none;
      color: oklch(var(--p));
    }}
    .tt-header a:hover {{ text-decoration: underline; }}
    .tt-close {{
      background: none; border: none;
      font-size: 1.25rem; cursor: pointer;
      opacity: 0.5; padding: 0 0.25rem;
      color: oklch(var(--bc));
    }}
    .tt-close:hover {{ opacity: 1; }}
    .tt-body {{
      padding: 0.75rem 1rem;
      overflow-y: auto;
      flex: 1;
    }}
    .tt-body.prose {{ font-size: 0.85rem; line-height: 1.6; }}
    .tt-footer {{
      padding: 0.5rem 1rem 0.75rem;
      border-top: 1px solid oklch(var(--b3));
      text-align: right;
    }}
    .tt-footer a {{
      font-size: 0.8rem;
      font-weight: 500;
      color: oklch(var(--p));
      text-decoration: none;
    }}
    .tt-footer a:hover {{ text-decoration: underline; }}
    @media (min-width: 1280px) {{
      .wikilink-tooltip {{ display: none !important; }}
    }}

    /* ── Search modal (Cmd+K) ─────────────────────────── */
    .search-overlay {{
      position: fixed; inset: 0;
      background: rgba(0,0,0,0.4);
      z-index: 100;
      display: none;
      align-items: flex-start;
      justify-content: center;
      padding-top: 15vh;
    }}
    .search-overlay.open {{ display: flex; }}
    .search-dialog {{
      background: oklch(var(--b1));
      border: 1px solid oklch(var(--b3));
      border-radius: 0.75rem;
      width: 90vw; max-width: 480px;
      box-shadow: 0 16px 48px rgba(0,0,0,0.25);
      overflow: hidden;
    }}
    .search-input {{
      width: 100%; border: none; outline: none;
      padding: 0.75rem 1rem;
      font-size: 1rem;
      background: transparent;
      color: oklch(var(--bc));
      border-bottom: 1px solid oklch(var(--b3));
    }}
    .search-results {{
      max-height: 40vh; overflow-y: auto;
      padding: 0.25rem 0;
    }}
    .search-result {{
      display: flex;
      flex-direction: column;
      gap: 0.1rem;
      padding: 0.5rem 1rem;
      text-decoration: none;
      color: oklch(var(--bc));
      cursor: pointer;
    }}
    .search-result:hover,
    .search-result.sr-active {{
      background: oklch(var(--p) / 0.12);
    }}
    .search-result .page-name {{
      font-weight: 600;
      font-size: 0.9rem;
    }}
    .search-result .heading {{
      font-size: 0.78rem;
      opacity: 0.6;
      font-style: italic;
    }}
    .search-result .context {{
      font-size: 0.78rem;
      opacity: 0.5;
      overflow: hidden;
      white-space: nowrap;
      text-overflow: ellipsis;
    }}
    .search-result .score {{
      font-size: 0.7rem;
      opacity: 0.4;
      font-family: monospace;
    }}
    .search-hint {{
      padding: 1rem;
      text-align: center;
      font-size: 0.85rem;
      opacity: 0.5;
    }}
    .search-btn {{
      display: flex; align-items: center; gap: 0.5rem;
      width: 100%;
      padding: 0.4rem 0.75rem;
      border: 1px solid oklch(var(--b3));
      border-radius: 0.5rem;
      background: oklch(var(--b1));
      color: oklch(var(--bc));
      font-size: 0.8rem;
      cursor: pointer;
      opacity: 0.7;
      transition: opacity 0.15s;
      margin-bottom: 0.75rem;
    }}
    .search-btn:hover {{ opacity: 1; }}
    .search-btn kbd {{
      margin-left: auto;
      font-size: 0.7rem;
      opacity: 0.5;
    }}
  </style>
</head>
<body class="min-h-screen bg-base-100">

  <!-- Drawer layout for responsive sidebar -->
  <div class="drawer lg:drawer-open">
    <input id="sidebar-toggle" type="checkbox" class="drawer-toggle">

    <!-- Main content -->
    <div class="drawer-content flex flex-col">
      <!-- Top bar (mobile) -->
      <div class="navbar bg-base-200 lg:hidden">
        <label for="sidebar-toggle" class="btn btn-ghost drawer-button">
          <svg xmlns="http://www.w3.org/2000/svg" class="h-5 w-5" fill="none"
               viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                  d="M4 6h16M4 12h16M4 18h16"/>
          </svg>
        </label>
        <span class="text-lg font-bold ml-2">zetl</span>
      </div>

      {main_section}
    </div>

    <!-- Sidebar -->
    <div class="drawer-side">
      <label for="sidebar-toggle" aria-label="close sidebar" class="drawer-overlay"></label>
      <aside class="bg-base-200 w-64 min-h-screen p-4">
        <a href="/" class="text-xl font-bold mb-4 block">zetl</a>
        <div class="divider my-1"></div>
        <button class="search-btn" onclick="openSearch()">
          <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><circle cx="11" cy="11" r="8"/><path d="m21 21-4.35-4.35"/></svg>
          Search…
          <kbd>⌘K</kbd>
        </button>
        {sidebar}
      </aside>
    </div>
  </div>

  <!-- Search modal (Cmd+K) -->
  <div class="search-overlay" id="search-overlay" onclick="if(event.target===this)closeSearch()">
    <div class="search-dialog">
      <input class="search-input" id="search-input" type="text" placeholder="Search pages…" autocomplete="off">
      <div class="search-results" id="search-results"></div>
    </div>
  </div>

  <script>
  (function(){{
    var overlay=document.getElementById('search-overlay');
    var input=document.getElementById('search-input');
    var results=document.getElementById('search-results');
    var active=-1;
    var filtered=[];   // array of {{slug,page,heading,context,score}}
    var debounceTimer=null;
    var currentCtrl=null; // AbortController for in-flight fetch

    // Embedded page list for fast client-side fallback on short queries (<=2 chars)
    var pageList=(function(){{
      try{{return JSON.parse(document.getElementById('zetl-search-index').textContent||'[]');}}
      catch(e){{return [];}}
    }})();

    function esc(s){{return s.replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;');}}

    function slugFromPath(path){{return path.replace(/\.md$/i,'');}}

    function render(items){{
      results.innerHTML='';
      if(!input.value){{
        results.innerHTML='<div class="search-hint">Type to search pages\u2026</div>';
        active=-1;
        return;
      }}
      if(items.length===0){{
        results.innerHTML='<div class="search-hint">No results</div>';
        active=-1;
        return;
      }}
      active=0;
      items.forEach(function(item,i){{
        var a=document.createElement('a');
        a.className='search-result'+(i===0?' sr-active':'');
        a.href='/'+item.slug;
        var html='<span class="page-name">'+esc(item.page)+'</span>';
        if(item.heading)html+='<span class="heading">'+esc(item.heading)+'</span>';
        if(item.context)html+='<span class="context">'+esc(item.context)+'</span>';
        if(item.score)html+='<span class="score">'+item.score.toFixed(2)+'</span>';
        a.innerHTML=html;
        a.addEventListener('mouseenter',function(){{active=i;updateActive();}});
        results.appendChild(a);
      }});
    }}

    function showLoading(){{
      results.innerHTML='<div class="search-hint">Searching\u2026</div>';
    }}

    function fastFilter(q){{
      var ql=q.toLowerCase();
      return pageList
        .filter(function(p){{return p.n.toLowerCase().indexOf(ql)>=0;}})
        .slice(0,10)
        .map(function(p){{return {{page:p.n,slug:p.s,score:0}};}});
    }}

    function runSearch(){{
      var q=input.value;
      if(!q){{filtered=[];render([]);return;}}
      if(q.length<=2){{
        filtered=fastFilter(q);
        render(filtered);
        return;
      }}
      showLoading();
      if(currentCtrl){{currentCtrl.abort();}}
      currentCtrl=new AbortController();
      fetch('/api/search?q='+encodeURIComponent(q)+'&limit=20',{{signal:currentCtrl.signal}})
        .then(function(r){{return r.ok?r.json():Promise.reject(r.status);}})
        .then(function(data){{
          currentCtrl=null;
          filtered=(data.results||[]).map(function(m){{
            return {{
              page:m.page,
              slug:slugFromPath(m.path),
              heading:m.heading||null,
              context:m.context||null,
              score:m.score
            }};
          }});
          render(filtered);
        }})
        .catch(function(err){{
          currentCtrl=null;
          if(err&&err.name==='AbortError')return;
          filtered=fastFilter(q);
          render(filtered);
        }});
    }}

    window.openSearch=function(){{
      overlay.classList.add('open');
      input.value='';
      active=-1;
      filtered=[];
      render([]);
      input.focus();
    }};
    window.closeSearch=function(){{
      overlay.classList.remove('open');
      if(currentCtrl){{currentCtrl.abort();currentCtrl=null;}}
    }};

    function updateActive(){{
      var els=results.querySelectorAll('.search-result');
      els.forEach(function(el,i){{
        el.classList.toggle('sr-active',i===active);
      }});
      if(active>=0&&els[active])els[active].scrollIntoView({{block:'nearest'}});
    }}

    input.addEventListener('keyup',function(){{
      clearTimeout(debounceTimer);
      debounceTimer=setTimeout(runSearch,150);
    }});

    document.addEventListener('keydown',function(e){{
      if((e.metaKey||e.ctrlKey)&&e.key==='k'){{
        e.preventDefault();
        if(overlay.classList.contains('open'))closeSearch();
        else openSearch();
        return;
      }}
      if(!overlay.classList.contains('open'))return;
      if(e.key==='Escape'){{closeSearch();return;}}
      if(e.key==='ArrowDown'){{
        e.preventDefault();
        if(active<filtered.length-1)active++;
        updateActive();
      }}else if(e.key==='ArrowUp'){{
        e.preventDefault();
        if(active>0)active--;
        updateActive();
      }}else if(e.key==='Enter'){{
        e.preventDefault();
        if(active>=0&&active<filtered.length){{
          window.location.href='/'+filtered[active].slug;
        }}
      }}
    }});
  }})();
  </script>

</body>
</html>"#,
        title = title,
        sidebar = sidebar,
        main_section = main_section,
        search_index = search_index,
    )
}

/// Build the sidebar HTML: a scrollable list of all pages.
/// Each entry is `(display_name, slug)` where slug is the URL path (e.g. "architecture/Scanner").
pub fn sidebar_html(pages: &[(String, String)], active_slug: Option<&str>) -> String {
    let mut s = String::from(r#"<ul class="menu menu-sm">"#);
    for (display, slug) in pages {
        let active = if active_slug == Some(slug.as_str()) {
            " active"
        } else {
            ""
        };
        s.push_str(&format!(
            r#"<li><a href="/{href}" class="{active}">{name}</a></li>"#,
            href = urlencoding(slug),
            active = active.trim(),
            name = html_escape(display),
        ));
    }
    s.push_str("</ul>");
    s
}

/// Build breadcrumb HTML from a slug like "architecture/scanner".
/// `vault_name` is the root folder name shown as the first crumb.
/// Folder segments link to folder index pages (e.g., `/architecture/`).
/// Produces: vault_name / architecture / Scanner (current page not linked).
pub fn breadcrumb_html(slug: &str, page_name: &str, vault_name: &str) -> String {
    let root = html_escape(vault_name);
    let parts: Vec<&str> = slug.split('/').collect();
    if parts.len() <= 1 {
        return format!(
            r#"<nav class="text-sm breadcrumbs mb-4"><ul><li><a href="/">{root}</a></li><li>{page}</li></ul></nav>"#,
            root = root,
            page = html_escape(page_name),
        );
    }
    let mut s = format!(
        r#"<nav class="text-sm breadcrumbs mb-4"><ul><li><a href="/">{root}</a></li>"#,
        root = root,
    );
    // Build cumulative folder path for each segment's href
    let mut folder_path = String::new();
    for folder in &parts[..parts.len() - 1] {
        if !folder_path.is_empty() {
            folder_path.push('/');
        }
        folder_path.push_str(folder);
        s.push_str(&format!(
            r#"<li><a href="/{path}/">{name}</a></li>"#,
            path = urlencoding(&folder_path),
            name = html_escape(folder),
        ));
    }
    s.push_str(&format!("<li>{}</li>", html_escape(page_name)));
    s.push_str("</ul></nav>");
    s
}

/// Minimal URL-encoding for page names (spaces → %20, etc.).
pub fn urlencoding(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            ' ' => "%20".to_string(),
            '#' => "%23".to_string(),
            '?' => "%3F".to_string(),
            '&' => "%26".to_string(),
            '%' => "%25".to_string(),
            _ => c.to_string(),
        })
        .collect()
}

/// Escape HTML special characters.
pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
