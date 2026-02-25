/// Wrap body HTML in the DaisyUI shell with sidebar and optional transclusion panel.
pub fn layout(
    title: &str,
    sidebar: &str,
    content: &str,
    active_page: Option<&str>,
    right_panel: Option<&str>,
) -> String {
    let _ = active_page; // used by sidebar_html to highlight

    let main_section = if let Some(panel) = right_panel {
        format!(
            r#"<div class="flex flex-1">
        <main class="flex-1 p-6 min-w-0">
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
            r#"<main class="flex-1 p-6 max-w-4xl mx-auto w-full">
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
  <link href="https://cdn.jsdelivr.net/npm/daisyui@4/dist/full.min.css" rel="stylesheet">
  <script src="https://cdn.tailwindcss.com?plugins=typography"></script>
  <style>
    /* dead link */
    a.link-error {{ color: oklch(var(--er)); text-decoration: underline wavy; }}

    /* transclusion panel */
    .transclusion-panel {{
      display: none;
      width: 36rem;
      flex-shrink: 0;
      border-left: 1px solid oklch(var(--b3));
      padding: 1rem;
      overflow-y: auto;
      position: sticky;
      top: 0;
      max-height: 100vh;
      background: oklch(var(--b1));
    }}
    @media (min-width: 1280px) {{
      .transclusion-panel {{ display: block; }}
    }}
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
      background: oklch(var(--b2));
      box-shadow: 0 0 0 1px oklch(var(--b3));
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
        {sidebar}
      </aside>
    </div>
  </div>

</body>
</html>"#,
        title = title,
        sidebar = sidebar,
        main_section = main_section,
    )
}

/// Build the sidebar HTML: a scrollable list of all page names.
pub fn sidebar_html(page_names: &[String], active_page: Option<&str>) -> String {
    let mut s = String::from(r#"<ul class="menu menu-sm">"#);
    for name in page_names {
        let active = if active_page == Some(name.as_str()) {
            " active"
        } else {
            ""
        };
        s.push_str(&format!(
            r#"<li><a href="/page/{href}" class="{active}">{name}</a></li>"#,
            href = urlencoding(name),
            active = active.trim(),
            name = html_escape(name),
        ));
    }
    s.push_str("</ul>");
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
