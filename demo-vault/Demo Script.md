<!DOCTYPE html>
<html lang="en" data-theme="default">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Demo Script — zetl</title>
  <script type="application/json" id="zetl-search-index">[{"n":"Cache","s":"architecture/cache"},{"n":"caching","s":"theories/caching"},{"n":"Defeasible Reasoning","s":"concepts/defeasible-reasoning"},{"n":"Demo Script","s":"demo-script"},{"n":"Deployment Decision","s":"decisions/deployment-decision"},{"n":"design-principles","s":"theories/design-principles"},{"n":"Feature Gates","s":"decisions/feature-gates"},{"n":"good-idea-2","s":"good-idea-2"},{"n":"Graph Queries","s":"features/graph-queries"},{"n":"JSON by Default","s":"decisions/json-by-default"},{"n":"Link Graph","s":"architecture/link-graph"},{"n":"Local-first Design","s":"decisions/local-first-design"},{"n":"Performance","s":"architecture/performance"},{"n":"Provenance","s":"concepts/provenance"},{"n":"Reason Commands","s":"features/reason-commands"},{"n":"Reasoning Engine","s":"architecture/reasoning-engine"},{"n":"Redis vs Memcached","s":"decisions/redis-vs-memcached"},{"n":"release-readiness","s":"theories/release-readiness"},{"n":"Rust for CLI","s":"decisions/rust-for-cli"},{"n":"Scanner","s":"architecture/scanner"},{"n":"Search","s":"features/search"},{"n":"search-history","s":"search-history"},{"n":"Spindle Lisp","s":"concepts/spindle-lisp"},{"n":"TUI","s":"architecture/tui"},{"n":"Vault Diagnostics","s":"features/vault-diagnostics"},{"n":"Wikilinks","s":"concepts/wikilinks"}]</script>
  
  <link href="https://cdn.jsdelivr.net/npm/daisyui@4/dist/full.min.css" rel="stylesheet">
  <script src="https://cdn.tailwindcss.com?plugins=typography"></script>
  
  <style>
    /* dead link */
    a.link-error { color: oklch(var(--er)); text-decoration: underline wavy; }
    /* line anchors for backlink scroll targets */
    .line-anchor { scroll-margin-top: 2rem; }
    :has(> .line-anchor:target) {
      animation: line-highlight 2s ease-out;
      border-radius: 4px;
    }
    @keyframes line-highlight {
      0%   { background: oklch(var(--wa) / 0.35); }
      100% { background: transparent; }
    }

    /* page + transclusion wrapper: stacked on mobile, side-by-side on desktop */
    .page-with-panel {
      display: flex;
      flex-direction: column;
    }
    @media (min-width: 1280px) {
      .page-with-panel { flex-direction: row; }
    }

    /* transclusion panel — mobile: inline below content */
    .transclusion-panel {
      border-top: 1px solid oklch(var(--b3));
      padding: 1rem;
      background: oklch(var(--b1));
    }
    .transclusion-panel .transclusion-card .tc-excerpt {
      display: block;
    }
    .transclusion-panel .transclusion-card {
      margin-bottom: 0.75rem;
    }
    /* transclusion panel — desktop: sticky sidebar */
    @media (min-width: 1280px) {
      .transclusion-panel {
        width: 36rem;
        flex-shrink: 0;
        border-top: none;
        border-left: 1px solid oklch(var(--b3));
        overflow-y: auto;
        position: sticky;
        top: 0;
        max-height: 100vh;
      }
      .transclusion-panel .transclusion-card .tc-excerpt {
        display: none;
      }
      .transclusion-panel .transclusion-card.tc-active .tc-excerpt {
        display: block;
      }
    }
    /* stats: horizontal scroll on small screens */
    .stats { overflow-x: auto; }
    .tp-header {
      font-size: 0.65rem;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.1em;
      opacity: 0.4;
      margin-bottom: 1rem;
    }
    .transclusion-card {
      border-left: 3px solid transparent;
      padding: 0.4rem 0.75rem;
      margin-bottom: 0.25rem;
      border-radius: 0.375rem;
      transition: background 0.15s ease, box-shadow 0.15s ease;
    }
    .transclusion-card:hover,
    .transclusion-card.tc-active {
      background: oklch(var(--b2) / 0.5);
      box-shadow: 0 0 0 1px oklch(var(--b3) / 0.4);
      padding: 0.6rem 0.75rem;
      margin-bottom: 0.5rem;
    }
    .transclusion-card .tc-title {
      font-size: 0.85rem;
      font-weight: 600;
      text-decoration: none;
    }
    .transclusion-card .tc-title:hover {
      text-decoration: underline;
    }
    .transclusion-card .tc-excerpt {
      display: none;
      margin-top: 0.5rem;
    }
    .transclusion-card .tc-excerpt.prose {
      font-size: 0.85rem;
      line-height: 1.6;
    }
    .transclusion-card.tc-active .tc-excerpt {
      display: block;
    }
    /* Mobile transclusion panel: stacked below content, all excerpts visible */
    @media (max-width: 1279px) {
      .transclusion-panel {
        border-top: 2px solid oklch(var(--b3));
        margin-top: 2rem;
      }
      .transclusion-panel .transclusion-card .tc-excerpt {
        display: block;
      }
    }

    /* wikilink color underline */
    a.wikilink {
      text-decoration-thickness: 2px;
      text-underline-offset: 3px;
      transition: background 0.12s ease;
    }
    a.wikilink.wl-active {
      background: rgba(0,0,0,0.08);
      border-radius: 2px;
      padding: 1px 3px;
      margin: 0 -3px;
    }

    /* Mobile link-preview tooltip */
    .wikilink-tooltip {
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
    }
    .wikilink-tooltip.tt-visible {
      opacity: 1;
      pointer-events: auto;
    }
    .tt-header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: 0.75rem 1rem;
      border-bottom: 1px solid oklch(var(--b3));
      position: sticky; top: 0;
      background: oklch(var(--b1));
      border-radius: 0.75rem 0.75rem 0 0;
    }
    .tt-header a {
      font-weight: 600;
      font-size: 0.95rem;
      text-decoration: none;
      color: oklch(var(--p));
    }
    .tt-header a:hover { text-decoration: underline; }
    .tt-close {
      background: none; border: none;
      font-size: 1.25rem; cursor: pointer;
      opacity: 0.5; padding: 0 0.25rem;
      color: oklch(var(--bc));
    }
    .tt-close:hover { opacity: 1; }
    .tt-body {
      padding: 0.75rem 1rem;
      overflow-y: auto;
      flex: 1;
    }
    .tt-body.prose { font-size: 0.85rem; line-height: 1.6; }
    .tt-footer {
      padding: 0.5rem 1rem 0.75rem;
      border-top: 1px solid oklch(var(--b3));
      text-align: right;
    }
    .tt-footer a {
      font-size: 0.8rem;
      font-weight: 500;
      color: oklch(var(--p));
      text-decoration: none;
    }
    .tt-footer a:hover { text-decoration: underline; }
    @media (min-width: 1280px) {
      .wikilink-tooltip { display: none !important; }
    }

    /* ── Search modal (Cmd+K) */
    .search-overlay {
      position: fixed; inset: 0;
      background: rgba(0,0,0,0.4);
      z-index: 100;
      display: none;
      align-items: flex-start;
      justify-content: center;
      padding-top: 15vh;
    }
    .search-overlay.open { display: flex; }
    .search-dialog {
      background: oklch(var(--b1));
      border: 1px solid oklch(var(--b3));
      border-radius: 0.75rem;
      width: 90vw; max-width: 480px;
      box-shadow: 0 16px 48px rgba(0,0,0,0.25);
      overflow: hidden;
    }
    .search-input {
      width: 100%; border: none; outline: none;
      padding: 0.75rem 1rem;
      font-size: 1rem;
      background: transparent;
      color: oklch(var(--bc));
      border-bottom: 1px solid oklch(var(--b3));
    }
    .search-results {
      max-height: 40vh; overflow-y: auto;
      padding: 0.25rem 0;
    }
    .search-result {
      display: flex;
      flex-direction: column;
      gap: 0.1rem;
      padding: 0.5rem 1rem;
      text-decoration: none;
      color: oklch(var(--bc));
      cursor: pointer;
    }
    .search-result:hover,
    .search-result.sr-active {
      background: oklch(var(--p) / 0.12);
    }
    .search-result .sr-name {
      font-weight: 500;
    }
    .search-result .sr-name b {
      color: oklch(var(--p));
      font-weight: 700;
    }
    .search-result .sr-slug {
      font-size: 0.75rem;
      opacity: 0.5;
    }
    .search-result .page-name {
      font-weight: 600;
      font-size: 0.9rem;
    }
    .search-result .heading {
      font-size: 0.78rem;
      opacity: 0.6;
      font-style: italic;
    }
    .search-result .context {
      font-size: 0.78rem;
      opacity: 0.5;
      overflow: hidden;
      white-space: nowrap;
      text-overflow: ellipsis;
    }
    .search-result .score {
      font-size: 0.7rem;
      opacity: 0.4;
      font-family: monospace;
    }
    .search-hint {
      padding: 1rem;
      text-align: center;
      font-size: 0.85rem;
      opacity: 0.5;
    }
    .search-btn {
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
    }
    .search-btn:hover { opacity: 1; }
    .search-btn kbd {
      margin-left: auto;
      font-size: 0.7rem;
      opacity: 0.5;
    }
    /* ── Sidebar tree */
    .sidebar-tree details > summary {
      cursor: pointer;
      list-style: none;
      padding: 0.25rem 0.5rem;
      font-weight: 600;
      font-size: 0.85rem;
      opacity: 0.7;
    }
    .sidebar-tree details > summary::-webkit-details-marker { display: none; }
    .sidebar-tree details > summary::before {
      content: '▶';
      display: inline-block;
      font-size: 0.6em;
      margin-right: 0.4em;
      transition: transform 0.15s ease;
    }
    .sidebar-tree details[open] > summary::before {
      transform: rotate(90deg);
    }
    .sidebar-tree details > ul {
      border-left: 1px solid oklch(var(--b3));
      margin-left: 0.75rem;
      padding-left: 0;
    }
    
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

      

<div class="page-with-panel flex-1">
  <main class="flex-1 p-4 sm:p-6 min-w-0">


    
    <nav class="text-sm breadcrumbs mb-4">
      <ul>
        <li><a href="&#x2f;">demo-vault</a></li>
        <li><a href="&#x2f;raw/">raw</a></li>
        <li>Demo Script</li>
      </ul>
    </nav>

    
    <div id="view-mode">
      
      <div class="flex justify-end mb-4">
        <a href="/edit/raw/Demo Script/" class="btn btn-sm btn-outline">Edit</a>
      </div>
      
    
      <article class="prose prose-lg max-w-none"></article>
      
    </div>

    
    
    <div id="edit-mode" style="display:none">
      <div class="flex gap-2 mb-4">
        <button onclick="saveEdit()" class="btn btn-sm btn-primary">Save</button>
        <button onclick="toggleEdit()" class="btn btn-sm btn-outline">Cancel</button>
      </div>
      <textarea id="editor" class="textarea textarea-bordered w-full font-mono"
                style="min-height:80vh"># Demo Script
</textarea>
    </div>
    

<script>
// Transclusion: SVG bridge lines + bidirectional hover
(function() {
  const COLORS = ["#f472b6","#60a5fa","#34d399","#fbbf24","#a78bfa","#fb923c","#2dd4bf","#f87171"];
  const cards = document.querySelectorAll('.transclusion-card');
  const colorMap = {};
  cards.forEach((card, i) => {
    colorMap[card.dataset.targetHref] = COLORS[i % COLORS.length];
  });

  document.querySelectorAll('a.wikilink:not(.wikilink-dead)').forEach(link => {
    const color = colorMap[link.getAttribute('href')];
    if (color) {
      link.style.textDecorationColor = color;
    }
  });

  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.style.cssText = 'position:fixed;top:0;left:0;width:100%;height:100%;pointer-events:none;z-index:40;';
  document.body.appendChild(svg);

  function drawBridge(fromEl, toEl, color) {
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
  }

  function clearBridge() {
    svg.innerHTML = '';
  }

  const isDesktop = () => window.matchMedia('(min-width: 1280px)').matches;
  const isMobile = () => !isDesktop();

  document.querySelectorAll('a.wikilink').forEach(link => {
    const href = link.getAttribute('href');
    const color = colorMap[href] || '#888';
    link.addEventListener('mouseenter', () => {
      const card = document.querySelector('.transclusion-card[data-target-href="' + href + '"]');
      if (card) {
        card.classList.add('tc-active');
        if (isDesktop()) drawBridge(link, card, color);
      }
    });
    link.addEventListener('mouseleave', () => {
      document.querySelectorAll('.transclusion-card.tc-active').forEach(c => c.classList.remove('tc-active'));
      clearBridge();
    });
  });

  cards.forEach(card => {
    const href = card.dataset.targetHref;
    card.addEventListener('mouseenter', () => {
      card.classList.add('tc-active');
      document.querySelectorAll('a.wikilink[href="' + href + '"]').forEach(l => l.classList.add('wl-active'));
    });
    card.addEventListener('mouseleave', () => {
      card.classList.remove('tc-active');
      document.querySelectorAll('a.wl-active').forEach(l => l.classList.remove('wl-active'));
    });
  });

  // ── Mobile tooltip preview ──────────────────────────────────────
  if (isMobile() && cards.length) {
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

    function showTooltip(linkEl, href) {
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
      if (below + window.innerHeight * 0.4 < window.innerHeight) {
        tooltip.style.top = below + 'px';
      } else {
        tooltip.style.bottom = (window.innerHeight - above) + 'px';
      }
      tooltip.classList.add('tt-visible');
      activeLink = linkEl;
    }

    function hideTooltip() {
      tooltip.classList.remove('tt-visible');
      activeLink = null;
    }

    ttClose.addEventListener('click', function(e) { e.preventDefault(); hideTooltip(); });

    document.addEventListener('click', function(e) {
      if (tooltip.contains(e.target)) return;
      var link = e.target.closest('a.wikilink:not(.wikilink-dead)');
      if (link) {
        var href = link.getAttribute('href');
        if (activeLink === link && tooltip.classList.contains('tt-visible')) {
          return; // second tap — navigate normally
        }
        var card = document.querySelector('.transclusion-card[data-target-href="' + href + '"]');
        if (card) {
          e.preventDefault();
          showTooltip(link, href);
          return;
        }
      }
      if (tooltip.classList.contains('tt-visible')) {
        hideTooltip();
      }
    });

    document.addEventListener('keydown', function(e) {
      if (e.key === 'Escape') hideTooltip();
    });
  }
})();
</script>
  </main>

  
  <aside class="transclusion-panel">
    <h3 class="tp-header">Linked Pages</h3>
    <div class="transclusion-card" data-target-href="/decisions/json-by-default/" style="border-left-color: #f472b6;">
  <a href="/decisions/json-by-default/" class="tc-title" style="color: #f472b6;">JSON by Default</a>
  <div class="tc-excerpt prose prose-sm max-w-none"><h1>JSON by Default</h1>
<p>All zetl commands emit JSON by default. Human-readable output is available via <code>--format table</code> (or <code>--format natural</code> / <code>--format dot</code> for <code>reason explain</code>).</p>
<pre><code class="language-spl">(given json-default-output)
(given structured-errors)
(given nonzero-exit-codes)
</code></pre>
<h2>Why agent-first</h2>
<p>zetl is designed to work both as a human CLI tool and as a building block for AI agents and scripts. JSON output means:</p>
<ul>
<li>Agents can parse results without scraping tables</li>
<li>Structured errors include error codes, affected files, and suggested fixes</li>
<li>Non-zero exit codes signal failure to shell scripts and CI pipelines</li>
<li>Output is composable with <code>jq</code>, <code>fx</code>, and other JSON tools</li>
</ul>
<h2>Human output</h2>
<p>For interactive use, <code>--format table</code> renders results as aligned tables via <code>comfy-table</code>. The <a href="/architecture/tui/" class="link link-primary wikilink">TUI</a> provides a richer interactive experience — see <a href="/architecture/tui/" class="link link-primary wikilink">TUI</a>.</p>
<h2>Both audiences</h2>
<p>This dual-output design means a single tool serves both audiences. An agent might run <code>zetl reason status</code> and parse the JSON, while a human runs <code>zetl reason status --format table</code> and reads the output directly.</p>
<p>See also: <a href="/decisions/rust-for-cli/" class="link link-primary wikilink">Rust for CLI</a>, <a href="/architecture/tui/" class="link link-primary wikilink">TUI</a>, <a href="/features/reason-commands/" class="link link-primary wikilink">Reason Commands</a></p>
</div>
</div><div class="transclusion-card" data-target-href="/concepts/provenance/" style="border-left-color: #60a5fa;">
  <a href="/concepts/provenance/" class="tc-title" style="color: #60a5fa;">Provenance</a>
  <div class="tc-excerpt prose prose-sm max-w-none"><h1>Provenance</h1>
<p>Provenance is the ability to trace a conclusion back to its origins. In zetl, every fact, rule, and conclusion carries metadata recording the source file, line number, and page name where it was defined.</p>
<h2>Why it matters</h2>
<p>A reasoning system that says “X is true” is only useful if you can ask “why?” and get a concrete answer. Provenance connects the <a href="/architecture/reasoning-engine/" class="link link-primary wikilink">Reasoning Engine</a>’s abstract logic back to the human-readable documents in your vault.</p>
<h2>What is tracked</h2>
<table><thead><tr><th>Element</th><th>Provenance</th></tr></thead><tbody>
<tr><td>Fact</td><td>File path, line number, page name</td></tr>
<tr><td>Rule</td><td>File path, line number, page name, rule label</td></tr>
<tr><td>Conclusion</td><td>Full proof tree of contributing rules and facts</td></tr>
</tbody></table>
<h2>Commands</h2>
<ul>
<li><code>zetl reason explain &lt;literal&gt;</code> — proof tree with source locations</li>
<li><code>zetl reason provenance &lt;literal&gt;</code> — cross-referenced with the <a href="/architecture/link-graph/" class="link link-primary wikilink">Link Graph</a></li>
<li><code>zetl reason export --format spl</code> — reconstructed <a href="/concepts/spindle-lisp/" class="link link-primary wikilink">Spindle Lisp</a> with provenance comments</li>
</ul>
<h2>Example</h2>
<p>Running <code>zetl reason provenance "release-candidate"</code> on this vault traces the conclusion through rules in <code>theories/release-readiness.spl</code> back to facts scattered across the architecture and feature pages.</p>
<p>See also: <a href="/architecture/reasoning-engine/" class="link link-primary wikilink">Reasoning Engine</a>, <a href="/features/reason-commands/" class="link link-primary wikilink">Reason Commands</a>, <a href="/concepts/defeasible-reasoning/" class="link link-primary wikilink">Defeasible Reasoning</a></p>
</div>
</div><div class="transclusion-card" data-target-href="/concepts/spindle-lisp/" style="border-left-color: #34d399;">
  <a href="/concepts/spindle-lisp/" class="tc-title" style="color: #34d399;">Spindle Lisp</a>
  <div class="tc-excerpt prose prose-sm max-w-none"><h1>Spindle Lisp</h1>
<p>Spindle Lisp (SPL) is a domain-specific language for expressing <a href="/concepts/defeasible-reasoning/" class="link link-primary wikilink">Defeasible Reasoning</a> theories. zetl extracts SPL from fenced code blocks in Markdown and from standalone <code>.spl</code> files.</p>
<h2>Syntax reference</h2>
<h3>Facts</h3>
<pre><code>(given bird)
(given (not guilty))
</code></pre>
<h3>Strict rules</h3>
<p>Cannot be defeated. If the body holds, the head must hold.</p>
<pre><code>(always r-penguin-is-bird penguin bird)
</code></pre>
<h3>Defeasible rules</h3>
<p>Can be defeated by stronger evidence. The label is optional.</p>
<pre><code>(normally r-birds-fly bird flies)
(normally bird animal)
</code></pre>
<h3>Defeaters</h3>
</div>
</div><div class="transclusion-card" data-target-href="/concepts/defeasible-reasoning/" style="border-left-color: #fbbf24;">
  <a href="/concepts/defeasible-reasoning/" class="tc-title" style="color: #fbbf24;">Defeasible Reasoning</a>
  <div class="tc-excerpt prose prose-sm max-w-none"><h1>Defeasible Reasoning</h1>
<p>Defeasible reasoning is a form of logic where conclusions can be drawn tentatively and later retracted when stronger evidence appears. This is how most real-world reasoning works — we act on the best information available, knowing new facts might change our minds.</p>
<h2>Why it fits knowledge managements</h2>
<p>Decision documents, architecture records, and project plans all contain reasoning that is provisional. A conclusion like “use Redis” might be well-supported now but defeated by a future license audit. <a href="/concepts/spindle-lisp/" class="link link-primary wikilink">Spindle Lisp</a> lets you express this directly in your notes, and zetl’s <a href="/architecture/reasoning-engine/" class="link link-primary wikilink">Reasoning Engine</a> computes what follows.</p>
<h2>Rule types</h2>
<table><thead><tr><th>Type</th><th>Syntax</th><th>Meaning</th></tr></thead><tbody>
<tr><td>Fact</td><td><code>(given X)</code></td><td>X is unconditionally true</td></tr>
<tr><td>Strict</td><td><code>(always name body head)</code></td><td>If body, then head — no exceptions</td></tr>
<tr><td>Defeasible</td><td><code>(normally name body head)</code></td><td>If body, normally head — can be defeated</td></tr>
<tr><td>Defeater</td><td><code>(except name body head)</code></td><td>If body, block head — but don’t assert the opposite</td></tr>
</tbody></table>
<h2>Superiority</h2>
<p>When two rules conflict a superiority relation resolves the tie:</p>
<pre><code>(prefer stronger-rule weaker-rule)
</code></pre>
<p>Without a declared preference, the conflict remains unresolved — <code>zetl reason conflicts</code> will flag it.</p>
<h2>Conclusion types</h2>
<table><thead><tr><th>Tag</th><th>Meaning</th></tr></thead><tbody>
<tr><td><code>+D</code></td><td>Definitely provable — strict derivation, no defeating possible</td></tr>
<tr><td><code>-D</code></td><td>Definitely not provable</td></tr>
<tr><td><code>+d</code></td><td>Defeasibly provable — inferred, no active defeaters</td></tr>
<tr><td><code>-d</code></td><td>Defeasibly not provable — blocked or no derivation path</td></tr>
</tbody></table>
<h2>In zetl</h2>
<p>The <a href="/architecture/reasoning-engine/" class="link link-primary wikilink">Reasoning Engine</a> implements defeasible reasoning via <code>spindle-core</code>. See <a href="/features/reason-commands/" class="link link-primary wikilink">Reason Commands</a> for the CLI interface and <a href="/concepts/provenance/" class="link link-primary wikilink">Provenance</a> for how conclusions trace back to source files.</p>
</div>
</div><div class="transclusion-card" data-target-href="/architecture/reasoning-engine/" style="border-left-color: #a78bfa;">
  <a href="/architecture/reasoning-engine/" class="tc-title" style="color: #a78bfa;">Reasoning Engine</a>
  <div class="tc-excerpt prose prose-sm max-w-none"><h1>Reasoning Engine</h1>
<p>The reasoning engine takes <a href="/concepts/spindle-lisp/" class="link link-primary wikilink">Spindle Lisp</a> extracted by the <a href="/architecture/scanner/" class="link link-primary wikilink">Scanner</a> from across the entire vault, merges it into a single theory, and computes conclusions using <a href="/concepts/defeasible-reasoning/" class="link link-primary wikilink">Defeasible Reasoning</a>. Every conclusion carries full <a href="/concepts/provenance/" class="link link-primary wikilink">Provenance</a> — you can trace any result back to the exact file and line that contributed to it.</p>
<h2>Pipeline</h2>
<ol>
<li><strong>Extract</strong> — <a href="/architecture/scanner/" class="link link-primary wikilink">Scanner</a> finds SPL blocks in Markdown fences and <code>.spl</code> files</li>
<li><strong>Parse</strong> — <code>spindle-parser</code> converts SPL text into rule objects</li>
<li><strong>Provenance</strong> — source file, line number, and page name are attached to each rule and fact</li>
<li><strong>Combine</strong> — all rules and facts are merged into a single theory</li>
<li><strong>Validate</strong> — check for undefined labels, duplicate definitions</li>
<li><strong>Reason</strong> — <code>spindle-core</code> computes conclusions</li>
<li><strong>Annotate</strong> — each conclusion is traced back to its proof sources</li>
</ol>
<pre><code class="language-spl">(given spindle-core-integrated)
(given four-conclusion-types)
</code></pre>
<h2>Conclusion types</h2>
<p>The engine produces four types of conclusion — see <a href="/concepts/defeasible-reasoning/" class="link link-primary wikilink">Defeasible Reasoning</a> for details:</p>
<table><thead><tr><th>Tag</th><th>Meaning</th></tr></thead><tbody>
<tr><td><code>+D</code></td><td>Definitely provable</td></tr>
<tr><td><code>-D</code></td><td>Definitely not provable</td></tr>
<tr><td><code>+d</code></td><td>Defeasibly provable</td></tr>
<tr><td><code>-d</code></td><td>Defeasibly not provable</td></tr>
</tbody></table>
<h2>Feature gate</h2>
<p>The reasoning engine is compiled only when <code>--features reason</code> is enabled — see <a href="/decisions/feature-gates/" class="link link-primary wikilink">Feature Gates</a>. Without it, <code>zetl reason</code> prints a clear error rather than failing silently.</p>
<p>See also: <a href="/features/reason-commands/" class="link link-primary wikilink">Reason Commands</a>, <a href="/concepts/provenance/" class="link link-primary wikilink">Provenance</a>, <a href="/concepts/spindle-lisp/" class="link link-primary wikilink">Spindle Lisp</a></p>
</div>
</div><div class="transclusion-card" data-target-href="/architecture/link-graph/" style="border-left-color: #fb923c;">
  <a href="/architecture/link-graph/" class="tc-title" style="color: #fb923c;">Link Graph</a>
  <div class="tc-excerpt prose prose-sm max-w-none"><h1>Link Graph</h1>
<p>zetl builds a directed graph where nodes are pages and edges are <a href="/concepts/wikilinks/" class="link link-primary wikilink">Wikilinks</a>. The graph powers all link-based queries: forward links, backlinks, multi-hop traversal, shortest path, and orphan detection.</p>
<h2>Data model</h2>
<ul>
<li><strong>Node</strong> — a Markdown page, identified by its title (filename without <code>.md</code>)</li>
<li><strong>Edge</strong> — a wikilink from one page to another, with optional alias, heading, or block reference</li>
</ul>
<p>The graph is built by the <a href="/architecture/scanner/" class="link link-primary wikilink">Scanner</a> and cached by the <a href="/architecture/cache/" class="link link-primary wikilink">Cache</a> for incremental re-scans.</p>
<pre><code class="language-spl">(given directed-graph)
(given multi-hop-traversal)
</code></pre>
<h2>Queries</h2>
<table><thead><tr><th>Command</th><th>Description</th></tr></thead><tbody>
<tr><td><code>links</code></td><td>Forward links from a page, with configurable depth</td></tr>
<tr><td><code>backlinks</code></td><td>Pages linking to a target, with depth traversal</td></tr>
<tr><td><code>path</code></td><td>Shortest link path between any two pages</td></tr>
<tr><td><code>export</code></td><td>Full graph as JSON</td></tr>
</tbody></table>
<p>These are exposed through <a href="/features/graph-queries/" class="link link-primary wikilink">Graph Queries</a>. With the <code>--with-conclusions</code> flag, link results are cross-referenced with the <a href="/architecture/reasoning-engine/" class="link link-primary wikilink">Reasoning Engine</a> to show what each linked page contributes to the vault’s logic.</p>
<h2>Library</h2>
<p>Built on <code>petgraph</code>, a Rust graph data structure library.</p>
<p>See also: <a href="/architecture/scanner/" class="link link-primary wikilink">Scanner</a>, <a href="/features/graph-queries/" class="link link-primary wikilink">Graph Queries</a>, <a href="/features/vault-diagnostics/" class="link link-primary wikilink">Vault Diagnostics</a></p>
</div>
</div><div class="transclusion-card" data-target-href="/architecture/scanner/" style="border-left-color: #2dd4bf;">
  <a href="/architecture/scanner/" class="tc-title" style="color: #2dd4bf;">Scanner</a>
  <div class="tc-excerpt prose prose-sm max-w-none"><h1>Scanner</h1>
<p>The scanner is zetl’s entry point for understanding a vault. It walks every Markdown file and standalone <code>.spl</code> file, extracting <a href="/concepts/wikilinks/" class="link link-primary wikilink">Wikilinks</a> and <a href="/concepts/spindle-lisp/" class="link link-primary wikilink">Spindle Lisp</a> blocks in a single pass.</p>
<h2>Wikilink extraction</h2>
<p>The scanner recognises all common wikilink forms:</p>
<ul>
<li><code>[[Page]]</code> — basic link</li>
<li><code>[[Page|alias]]</code> — aliased link</li>
<li><code>[[Page#heading]]</code> — heading anchor</li>
<li><code>[[Page^block-id]]</code> — block reference</li>
<li><code>![[Page]]</code> — embed</li>
</ul>
<p>Extracted links feed into the <a href="/architecture/link-graph/" class="link link-primary wikilink">Link Graph</a> for query and traversal.</p>
<h2>SPL extraction</h2>
<p>Fenced code blocks tagged <code>```spl</code> are extracted verbatim, along with their source file and line number. Standalone <code>.spl</code> files anywhere in the vault are also picked up. Both feed into the <a href="/architecture/reasoning-engine/" class="link link-primary wikilink">Reasoning Engine</a> with full <a href="/concepts/provenance/" class="link link-primary wikilink">Provenance</a>.</p>
<pre><code class="language-spl">(given wikilink-extraction)
(given spl-extraction)
</code></pre>
<h2>Implementation</h2>
<p>Built on <code>pulldown-cmark</code> for Markdown parsing and <code>ignore</code> for <code>.gitignore</code>-aware file walking. The scanner is deliberately read-only — see <a href="/decisions/local-first-design/" class="link link-primary wikilink">Local-first Design</a>.</p>
<p>See also: <a href="/architecture/cache/" class="link link-primary wikilink">Cache</a>, <a href="/architecture/link-graph/" class="link link-primary wikilink">Link Graph</a>, <a href="/architecture/reasoning-engine/" class="link link-primary wikilink">Reasoning Engine</a></p>
</div>
</div><div class="transclusion-card" data-target-href="/decisions/rust-for-cli/" style="border-left-color: #f87171;">
  <a href="/decisions/rust-for-cli/" class="tc-title" style="color: #f87171;">Rust for CLI</a>
  <div class="tc-excerpt prose prose-sm max-w-none"><h1>Rust for CLI</h1>
<p>zetl is written in Rust. This was a deliberate choice driven by the requirements of a CLI tool that handles large vaults and complex reasoning.</p>
<pre><code class="language-spl">(given type-safe)
(given single-binary)
(given fast-startup)
</code></pre>
<h2>Why Rust</h2>
<ul>
<li><strong>Type safety</strong> — the <a href="/architecture/scanner/" class="link link-primary wikilink">Scanner</a>, <a href="/architecture/link-graph/" class="link link-primary wikilink">Link Graph</a>, and <a href="/architecture/reasoning-engine/" class="link link-primary wikilink">Reasoning Engine</a> involve complex data flows. Rust’s type system catches entire classes of bugs at compile time.</li>
<li><strong>Single binary</strong> — <code>cargo install</code> produces one executable with no runtime dependencies. Users don’t need Python, Node, or a JVM.</li>
<li><strong>Fast startup</strong> — CLI tools that take hundreds of milliseconds to start feel sluggish. Rust’s zero-cost abstractions keep startup instant even on large vaults.</li>
<li><strong>Memory safety</strong> — no garbage collector pauses, no null pointer surprises. Important when processing thousands of files.</li>
</ul>
<h2>Trade-offs</h2>
<ul>
<li>Slower to iterate during development compared to a scripting language</li>
<li>The <a href="/decisions/feature-gates/" class="link link-primary wikilink">Feature Gates</a> mechanism adds build complexity</li>
<li>Compile times are non-trivial</li>
</ul>
<p>These trade-offs are acceptable for a tool meant to be installed once and used daily.</p>
<h2>Dependencies</h2>
<p>Key crates: <code>clap</code> (CLI parsing), <code>petgraph</code> (<a href="/architecture/link-graph/" class="link link-primary wikilink">Link Graph</a>), <code>pulldown-cmark</code> (<a href="/architecture/scanner/" class="link link-primary wikilink">Scanner</a>), <code>ratatui</code> (<a href="/architecture/tui/" class="link link-primary wikilink">TUI</a>), <code>spindle-core</code> / <code>spindle-parser</code> (<a href="/architecture/reasoning-engine/" class="link link-primary wikilink">Reasoning Engine</a>).</p>
<p>See also: <a href="/decisions/feature-gates/" class="link link-primary wikilink">Feature Gates</a>, <a href="/decisions/json-by-default/" class="link link-primary wikilink">JSON by Default</a>, <a href="/decisions/local-first-design/" class="link link-primary wikilink">Local-first Design</a></p>
</div>
</div><div class="transclusion-card" data-target-href="/decisions/local-first-design/" style="border-left-color: #f472b6;">
  <a href="/decisions/local-first-design/" class="tc-title" style="color: #f472b6;">Local-first Design</a>
  <div class="tc-excerpt prose prose-sm max-w-none"><h1>Local-first Design</h1>
<p>zetl never modifies your files. It is strictly read-only against the vault.</p>
<pre><code class="language-spl">(given read-only-vault-access)
(given disposable-cache)
</code></pre>
<h2>Principles</h2>
<ul>
<li><strong>Read-only</strong> — zetl only reads Markdown and <code>.spl</code> files. It never writes to, renames, or deletes vault content.</li>
<li><strong>Disposable cache</strong> — the <code>.zetl/</code> directory contains only derived data (the <a href="/architecture/link-graph/" class="link link-primary wikilink">Link Graph</a> index and <a href="/architecture/reasoning-engine/" class="link link-primary wikilink">Reasoning Engine</a> theory cache). Deleting it loses nothing; <code>zetl index</code> regenerates it.</li>
<li><strong>No network</strong> — zetl makes no network calls. Everything runs locally.</li>
<li><strong>No lock-in</strong> — your vault is plain Markdown with optional <a href="/concepts/spindle-lisp/" class="link link-primary wikilink">Spindle Lisp</a> blocks. Removing zetl leaves your files untouched.</li>
</ul>
<h2>Why this matters</h2>
<p>Users trust zetl with their knowledge base — years of accumulated notes. A tool that might corrupt, reformat, or accidentally delete files would be a non-starter. Read-only access removes that risk entirely.</p>
<p>The <a href="/architecture/cache/" class="link link-primary wikilink">Cache</a> is the only thing zetl writes, and it lives in a clearly-marked directory that can be gitignored.</p>
<h2>Compatibility</h2>
<p>This design means zetl works alongside Obsidian, Logseq, Foam, Dendron, or any editor. Multiple tools can read the same vault simultaneously without conflict.</p>
<p>See also: <a href="/architecture/cache/" class="link link-primary wikilink">Cache</a>, <a href="/architecture/scanner/" class="link link-primary wikilink">Scanner</a>, <a href="/decisions/rust-for-cli/" class="link link-primary wikilink">Rust for CLI</a></p>
</div>
</div><div class="transclusion-card" data-target-href="/features/search/" style="border-left-color: #60a5fa;">
  <a href="/features/search/" class="tc-title" style="color: #60a5fa;">Search</a>
  <div class="tc-excerpt prose prose-sm max-w-none"><h1>Test</h1>
</div>
</div><div class="transclusion-card" data-target-href="/features/vault-diagnostics/" style="border-left-color: #34d399;">
  <a href="/features/vault-diagnostics/" class="tc-title" style="color: #34d399;">Vault Diagnostics</a>
  <div class="tc-excerpt prose prose-sm max-w-none"><h1>Vault Diagnostics</h1>
<p><code>zetl check</code> validates the vault and reports issues. It examines both the <a href="/architecture/link-graph/" class="link link-primary wikilink">Link Graph</a> and <a href="/concepts/spindle-lisp/" class="link link-primary wikilink">Spindle Lisp</a> content.</p>
<pre><code class="language-spl">(given dead-link-detection)
(given orphan-detection)
(given spl-diagnostics)
</code></pre>
<h2>Issue types</h2>
<h3>Dead links</h3>
<p>A <a href="/concepts/wikilinks/" class="link link-primary wikilink">wikilink</a> that points to a page that doesn’t exist. For example, <code>[[Plugin System]]</code> in this vault is a dead link — there is no <code>Plugin System.md</code> file.</p>
<pre><code class="language-bash">zetl -d . check --dead-links
</code></pre>
<h3>Orphan pages</h3>
<p>Pages with no incoming links — nothing in the vault references them. These may be forgotten drafts or entry points that need a <a href="/concepts/wikilinks/" class="link link-primary wikilink">wikilink</a> from somewhere.</p>
<pre><code class="language-bash">zetl -d . check --orphans
</code></pre>
<h3>Syntax errors</h3>
<p>Malformed <a href="/concepts/wikilinks/" class="link link-primary wikilink">Wikilinks</a> like unclosed brackets.</p>
</div>
</div><div class="transclusion-card" data-target-href="/architecture/cache/" style="border-left-color: #fbbf24;">
  <a href="/architecture/cache/" class="tc-title" style="color: #fbbf24;">Cache</a>
  <div class="tc-excerpt prose prose-sm max-w-none"><h1>Cache</h1>
<p>zetl uses mtiame-based incremental caching for both the <a href="/architecture/link-graph/" class="link link-primary wikilink">Link Graph</a> and the <a href="/architecture/reasoning-engine/" class="link link-primary wikilink">Reasoning Engine</a>’s theory. On subsequent runs, only files modified since the last scan are re-parsed.</p>
<h2>How it works</h2>
<p>The cache lives in <code>.zetl/</code> at the vault root:</p>
<ul>
<li><code>index.json</code> — serialised link graph</li>
<li><code>theory.json</code> — serialised reasoning theory and conclusions</li>
</ul>
<p>Each entry records the file’s last-modified timestamp. When zetl starts, it compares mtimes and only re-scans changed files. Use <code>--no-cache</code> to force a full rebuild.</p>
<pre><code class="language-spl">(given mtime-based-cache)
(given incremental-rebuild)
</code></pre>
<h2>Design tension</h2>
<p>There is an unresolved design tension in how aggressively to cache reasoning results:</p>
<pre><code class="language-spl">; Cache the theory for fast startup
(normally r-cache-theory
  mtime-based-cache
  cache-reasoning-results)

; But recomputing ensures results are always fresh
(normally r-recompute-theory
  incremental-rebuild
  (not cache-reasoning-results))
</code></pre>
<p>Both arguments have merit. Try <code>zetl reason conflicts</code> on this vault to see how zetl surfaces this kind of tension. A future <a href="/Plugin%20System/" class="link-error wikilink wikilink-dead">Plugin System</a> could let users configure the trade-off.</p>
<h2>Safety</h2>
</div>
</div><div class="transclusion-card" data-target-href="/features/graph-queries/" style="border-left-color: #a78bfa;">
  <a href="/features/graph-queries/" class="tc-title" style="color: #a78bfa;">Graph Queries</a>
  <div class="tc-excerpt prose prose-sm max-w-none"><h1>Graph Queries</h1>
<p>zetl exposes the <a href="/architecture/link-graph/" class="link link-primary wikilink">Link Graph</a> through several query commands. All output JSON by default — see <a href="/decisions/json-by-default/" class="link link-primary wikilink">JSON by Default</a>.</p>
<pre><code class="language-spl">(given forward-links-done)
(given backlinks-done)
(given shortest-path-done)
</code></pre>
<h2>Commands</h2>
<h3><code>zetl links &lt;page&gt;</code></h3>
<p>Show all pages that <code>&lt;page&gt;</code> links to. Supports <code>--depth N</code> for multi-hop traversal and <code>--fuzzy</code> for approximate page name matching.</p>
<h3><code>zetl backlinks &lt;page&gt;</code></h3>
<p>Show all pages that link to <code>&lt;page&gt;</code>. Same depth and fuzzy options as <code>links</code>.</p>
<h3><code>zetl path &lt;from&gt; &lt;to&gt;</code></h3>
<p>Find the shortest chain of <a href="/concepts/wikilinks/" class="link link-primary wikilink">Wikilinks</a> connecting two pages. Useful for discovering how ideas relate through intermediate notes.</p>
<h3><code>zetl export</code></h3>
<p>Dump the entire <a href="/architecture/link-graph/" class="link link-primary wikilink">Link Graph</a> as JSON — every page and every link.</p>
</div>
</div><div class="transclusion-card" data-target-href="/concepts/wikilinks/" style="border-left-color: #fb923c;">
  <a href="/concepts/wikilinks/" class="tc-title" style="color: #fb923c;">Wikilinks</a>
  <div class="tc-excerpt prose prose-sm max-w-none"><h1>Wikilinks</h1>
<p>Wikilinks are inline references between pages using <code>[[double bracket]]</code> syntax. They are the primary connective tissue in a zetl vault, forming the edges of the <a href="/architecture/link-graph/" class="link link-primary wikilink">Link Graph</a>.</p>
<h2>Syntax</h2>
<table><thead><tr><th>Form</th><th>Example</th><th>Description</th></tr></thead><tbody>
<tr><td>Basic</td><td><code>[[Cache]]</code></td><td>Link to a page</td></tr>
<tr><td>Aliased</td><td><code>[[Cache|caching layer]]</code></td><td>Display text differs from target</td></tr>
<tr><td>Heading</td><td><code>[[Cache#Design tension]]</code></td><td>Link to a specific heading</td></tr>
<tr><td>Block</td><td><code>[[Cache^summary]]</code></td><td>Link to a block ID</td></tr>
<tr><td>Embed</td><td><code>![[Cache]]</code></td><td>Embed the target page inline</td></tr>
</tbody></table>
<h2>How zetl uses them</h2>
<p>The <a href="/architecture/scanner/" class="link link-primary wikilink">Scanner</a> extracts wikilinks from every Markdown file. The <a href="/architecture/link-graph/" class="link link-primary wikilink">Link Graph</a> stores them as directed edges, enabling:</p>
<ul>
<li>Forward link queries (<code>zetl links</code>)</li>
<li>Backlink queries (<code>zetl backlinks</code>)</li>
<li>Shortest path computation (<code>zetl path</code>)</li>
<li>Dead link detection (<code>zetl check</code>)</li>
</ul>
<h2>Cross-referencing with logic</h2>
<p>When reasoning is enabled, <code>--with-conclusions</code> annotates link results with the <a href="/concepts/spindle-lisp/" class="link link-primary wikilink">Spindle Lisp</a> conclusions that each linked page contributes. This bridges the <a href="/architecture/link-graph/" class="link link-primary wikilink">Link Graph</a> and the <a href="/architecture/reasoning-engine/" class="link link-primary wikilink">Reasoning Engine</a>.</p>
<h2>Compatibility</h2>
<p>zetl supports the wikilink conventions used by Obsidian, Logseq, Foam, and Dendron. See <a href="/decisions/local-first-design/" class="link link-primary wikilink">Local-first Design</a> for the principle that zetl never modifies your files.</p>
</div>
</div>
  </aside>
</div>
  

    </div>

    <!-- Sidebar -->
    <div class="drawer-side">
      <label for="sidebar-toggle" aria-label="close sidebar" class="drawer-overlay"></label>
      <aside class="bg-base-200 w-64 min-h-screen p-4">
        <a href="&#x2f;" class="text-xl font-bold mb-4 block">zetl</a>
        <div class="divider my-1"></div>
        <button class="search-btn" onclick="openSearch()">
          <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><circle cx="11" cy="11" r="8"/><path d="m21 21-4.35-4.35"/></svg>
          Search…
          <kbd>⌘K</kbd>
        </button>
        
        
        
        <ul class="menu menu-sm sidebar-tree">
          <li>
            <details>
              <summary>architecture</summary>
              
        <ul class="menu menu-sm sidebar-tree">
          <li><a href="&#x2f;architecture/cache/"
            class="">Cache</a></li>
          <li><a href="&#x2f;architecture/link-graph/"
            class="">Link Graph</a></li>
          <li><a href="&#x2f;architecture/performance/"
            class="">Performance</a></li>
          <li><a href="&#x2f;architecture/reasoning-engine/"
            class="">Reasoning Engine</a></li>
          <li><a href="&#x2f;architecture/scanner/"
            class="">Scanner</a></li>
          <li><a href="&#x2f;architecture/tui/"
            class="">TUI</a></li>
        </ul>
        
            </details>
          </li>
          <li>
            <details>
              <summary>concepts</summary>
              
        <ul class="menu menu-sm sidebar-tree">
          <li><a href="&#x2f;concepts/defeasible-reasoning/"
            class="">Defeasible Reasoning</a></li>
          <li><a href="&#x2f;concepts/provenance/"
            class="">Provenance</a></li>
          <li><a href="&#x2f;concepts/spindle-lisp/"
            class="">Spindle Lisp</a></li>
          <li><a href="&#x2f;concepts/wikilinks/"
            class="">Wikilinks</a></li>
        </ul>
        
            </details>
          </li>
          <li>
            <details>
              <summary>decisions</summary>
              
        <ul class="menu menu-sm sidebar-tree">
          <li><a href="&#x2f;decisions/deployment-decision/"
            class="">Deployment Decision</a></li>
          <li><a href="&#x2f;decisions/feature-gates/"
            class="">Feature Gates</a></li>
          <li><a href="&#x2f;decisions/json-by-default/"
            class="">JSON by Default</a></li>
          <li><a href="&#x2f;decisions/local-first-design/"
            class="">Local-first Design</a></li>
          <li><a href="&#x2f;decisions/redis-vs-memcached/"
            class="">Redis vs Memcached</a></li>
          <li><a href="&#x2f;decisions/rust-for-cli/"
            class="">Rust for CLI</a></li>
        </ul>
        
            </details>
          </li>
          <li>
            <details>
              <summary>features</summary>
              
        <ul class="menu menu-sm sidebar-tree">
          <li><a href="&#x2f;features/graph-queries/"
            class="">Graph Queries</a></li>
          <li><a href="&#x2f;features/reason-commands/"
            class="">Reason Commands</a></li>
          <li><a href="&#x2f;features/search/"
            class="">Search</a></li>
          <li><a href="&#x2f;features/vault-diagnostics/"
            class="">Vault Diagnostics</a></li>
        </ul>
        
            </details>
          </li>
          <li>
            <details>
              <summary>theories</summary>
              
        <ul class="menu menu-sm sidebar-tree">
          <li><a href="&#x2f;theories/caching/"
            class="">caching</a></li>
          <li><a href="&#x2f;theories/design-principles/"
            class="">design-principles</a></li>
          <li><a href="&#x2f;theories/release-readiness/"
            class="">release-readiness</a></li>
        </ul>
        
            </details>
          </li>
          <li><a href="&#x2f;demo-script/"
            class="">Demo Script</a></li>
          <li><a href="&#x2f;good-idea-2/"
            class="">good-idea-2</a></li>
          <li><a href="&#x2f;search-history/"
            class="">search-history</a></li>
        </ul>
        
        
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
  (function(){
    var MODE='serve';
    var ROOT='/';
    var IDXFILE='';
    var overlay=document.getElementById('search-overlay');
    var input=document.getElementById('search-input');
    var results=document.getElementById('search-results');
    var active=-1;
    var filtered=[];

    var pageList=(function(){
      try{return JSON.parse(document.getElementById('zetl-search-index').textContent||'[]');}
      catch(e){return [];}
    })();

    function esc(s){return s.replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;');}
    function slugFromPath(path){return path.replace(/\.[^/.]+$/,'').toLowerCase().replace(/ /g,'-');}

    /* ── Fuzzy matching (works in all modes, no fetch needed) */
    function fuzzyMatch(query,text){
      var ql=query.length,tl=text.length;
      if(ql===0)return{score:0,indices:[]};
      if(ql>tl)return null;
      var qLow=query.toLowerCase(),tLow=text.toLowerCase();
      var qi=0;
      for(var ti=0;ti<tl&&qi<ql;ti++){if(tLow[ti]===qLow[qi])qi++;}
      if(qi<ql)return null;
      var bestScore=-Infinity,bestIndices=null;
      function solve(qi2,ti2,indices,score,prevMatch){
        if(qi2===ql){if(score>bestScore){bestScore=score;bestIndices=indices.slice();}return;}
        var remaining=ql-qi2;
        for(var t=ti2;t<=tl-remaining;t++){
          if(tLow[t]===qLow[qi2]){
            var s=0;
            if(prevMatch===t-1)s+=5;
            if(t===0)s+=8;
            else{var prev=text[t-1];if(prev===' '||prev==='-'||prev==='_'||prev==='/'||prev==='.')s+=7;else if(text[t]===text[t].toUpperCase()&&prev===prev.toLowerCase()&&/[a-zA-Z]/.test(prev))s+=6;}
            if(text[t]===query[qi2])s+=1;
            s+=Math.max(0,3-Math.floor(t/4));
            indices.push(t);solve(qi2+1,t+1,indices,score+s,t);indices.pop();
          }
        }
      }
      solve(0,0,[],0,-2);
      if(!bestIndices)return null;
      return{score:bestScore,indices:bestIndices};
    }

    function highlight(text,indices){
      var set={};indices.forEach(function(i){set[i]=true;});
      var out='';
      for(var i=0;i<text.length;i++){var c=esc(text[i]);out+=set[i]?'<b>'+c+'</b>':c;}
      return out;
    }

    function fuzzyFilter(q){
      if(!q)return pageList.map(function(p){return{n:p.n,s:p.s,_indices:null};});
      var scored=[];
      pageList.forEach(function(p){
        var m=fuzzyMatch(q,p.n);
        if(m)scored.push({n:p.n,s:p.s,score:m.score,_indices:m.indices});
      });
      scored.sort(function(a,b){return b.score-a.score;});
      return scored;
    }

    /* ── Render (two styles: fuzzy name results vs full-text) */
    function renderFuzzy(items){
      results.innerHTML='';
      if(items.length===0){
        results.innerHTML='<div class="search-hint">No results</div>';
        active=-1;return;
      }
      items.forEach(function(item,i){
        var a=document.createElement('a');
        a.className='search-result'+(i===active?' sr-active':'');
        a.href=ROOT+item.s+'/'+IDXFILE;
        var nameHtml=item._indices?highlight(item.n,item._indices):esc(item.n);
        a.innerHTML='<div class="sr-name">'+nameHtml+'</div><div class="sr-slug">'+esc(item.s)+'</div>';
        a.addEventListener('mouseenter',function(){active=i;updateActive();});
        results.appendChild(a);
      });
    }

    function renderFullText(items){
      results.innerHTML='';
      if(!input.value){
        results.innerHTML='<div class="search-hint">Type to search pages\u2026</div>';
        active=-1;return;
      }
      if(items.length===0){
        results.innerHTML='<div class="search-hint">No results</div>';
        active=-1;return;
      }
      active=0;
      items.forEach(function(item,i){
        var a=document.createElement('a');
        a.className='search-result'+(i===0?' sr-active':'');
        a.href=ROOT+item.slug+'/'+IDXFILE+(item.line?'#line-'+item.line:'');
        var html='<span class="page-name">'+esc(item.page)+'</span>';
        if(item.heading)html+='<span class="heading">'+esc(item.heading)+'</span>';
        if(item.context)html+='<span class="context">'+esc(item.context)+'</span>';
        if(item.score)html+='<span class="score">'+item.score.toFixed(2)+'</span>';
        a.innerHTML=html;
        a.addEventListener('mouseenter',function(){active=i;updateActive();});
        results.appendChild(a);
      });
    }

    function updateActive(){
      var els=results.querySelectorAll('.search-result');
      els.forEach(function(el,i){el.classList.toggle('sr-active',i===active);});
      if(active>=0&&els[active])els[active].scrollIntoView({block:'nearest'});
    }

    /* ── BM25 scorer for build mode (client-side) */
    var searchIndex=(function(){
      try{var el=document.getElementById('zetl-bm25-index');
        return el?JSON.parse(el.textContent):null;}
      catch(e){return null;}
    })();

    function findBestSection(doc,terms){
      if(!doc.secs||!doc.secs.length)return null;
      var best=null,bestCount=-1;
      doc.secs.forEach(function(sec){
        var text=(sec.h+' '+sec.t).toLowerCase();
        var count=0;
        terms.forEach(function(t){
          var idx=0;
          while((idx=text.indexOf(t,idx))!==-1){count++;idx+=t.length;}
        });
        if(count>bestCount){bestCount=count;best=sec;}
      });
      return bestCount>0?best:null;
    }

    function extractContext(text,terms){
      if(!text)return null;
      var lower=text.toLowerCase();
      var bestPos=-1;
      terms.forEach(function(t){
        var idx=lower.indexOf(t);
        if(idx!==-1&&(bestPos===-1||idx<bestPos))bestPos=idx;
      });
      if(bestPos===-1)bestPos=0;
      var start=Math.max(0,bestPos-40);
      var end=Math.min(text.length,start+120);
      var snippet=text.substring(start,end);
      if(start>0)snippet='\u2026'+snippet;
      if(end<text.length)snippet+='\u2026';
      return snippet;
    }

    function bm25Search(q,index){
      var k1=1.2,b=0.75;
      var terms=q.toLowerCase().split(/\W+/).filter(Boolean);
      var n=index.docs.length;var avgDl=index.avgDl||1;var scores={};
      terms.forEach(function(term){
        var df=index.df[term]||0;if(df===0)return;
        var idf=Math.log((n-df+0.5)/(df+0.5)+1);
        index.docs.forEach(function(doc,i){
          var tf=doc.tf[term]||0;if(tf===0)return;
          var norm=1-b+b*(doc.dl/avgDl);
          var sc=idf*(tf*(k1+1))/(tf+k1*norm);
          scores[i]=(scores[i]||0)+sc;
        });
      });
      return Object.keys(scores).map(function(i){
        var doc=index.docs[parseInt(i)];
        var sec=findBestSection(doc,terms);
        return{
          page:doc.n,slug:doc.s,score:scores[i],
          heading:sec?sec.h||null:null,
          context:sec?extractContext(sec.t,terms):null,
          line:sec?sec.l||0:0
        };
      }).sort(function(a,b){return b.score-a.score;});
    }

    /* ── Serve mode: /api/search (Tantivy full-text) */
    var currentCtrl=null;
    var debounceTimer=null;
    function serveSearch(q){
      if(q.length<=2){
        filtered=fuzzyFilter(q);
        active=0;renderFuzzy(filtered);return;
      }
      results.innerHTML='<div class="search-hint">Searching\u2026</div>';
      if(currentCtrl){currentCtrl.abort();}
      currentCtrl=new AbortController();
      fetch(ROOT+'api/search?q='+encodeURIComponent(q)+'&limit=20',{signal:currentCtrl.signal})
        .then(function(r){return r.ok?r.json():Promise.reject(r.status);})
        .then(function(data){
          currentCtrl=null;
          filtered=(data.results||[]).map(function(m){
            return{page:m.page,slug:slugFromPath(m.path),heading:m.heading||null,context:m.context||null,score:m.score,line:m.line||0};
          });
          renderFullText(filtered);
        })
        .catch(function(err){
          currentCtrl=null;
          if(err&&err.name==='AbortError')return;
          filtered=fuzzyFilter(q);
          active=0;renderFuzzy(filtered);
        });
    }

    /* ── Build mode: fuzzy + BM25 content search */
    function buildSearchRun(){
      var q=input.value;
      if(!q){
        filtered=fuzzyFilter('');
        active=0;renderFuzzy(filtered);return;
      }
      /* Always show fuzzy name matches first */
      if(!searchIndex){
        filtered=fuzzyFilter(q);
        active=0;renderFuzzy(filtered);return;
      }
      /* BM25 content search for longer queries when index is available */
      if(q.length>2){
        var bm25=bm25Search(q,searchIndex).slice(0,20);
        if(bm25.length>0){filtered=bm25;renderFullText(filtered);return;}
      }
      filtered=fuzzyFilter(q);
      active=0;renderFuzzy(filtered);
    }

    function runSearch(){
      var q=input.value;
      if(MODE==='serve'){
        if(!q){filtered=fuzzyFilter('');active=0;renderFuzzy(filtered);return;}
        serveSearch(q);
      }else{
        buildSearchRun();
      }
    }

    /* ── Open / close */
    window.openSearch=function(){
      overlay.classList.add('open');
      input.value='';
      filtered=fuzzyFilter('');
      active=0;renderFuzzy(filtered);
      input.focus();
    };
    window.closeSearch=function(){
      overlay.classList.remove('open');
      if(currentCtrl){currentCtrl.abort();currentCtrl=null;}
    };

    /* ── Input / keyboard */
    input.addEventListener('input',function(){
      active=0;runSearch();
    });

    document.addEventListener('keydown',function(e){
      if((e.metaKey||e.ctrlKey)&&e.key==='k'){
        e.preventDefault();
        if(overlay.classList.contains('open'))closeSearch();
        else openSearch();
        return;
      }
      if(!overlay.classList.contains('open'))return;
      if(e.key==='Escape'){closeSearch();return;}
      if(e.key==='ArrowDown'){
        e.preventDefault();
        if(active<filtered.length-1)active++;
        updateActive();
      }else if(e.key==='ArrowUp'){
        e.preventDefault();
        if(active>0)active--;
        updateActive();
      }else if(e.key==='Enter'){
        e.preventDefault();
        if(active>=0&&active<filtered.length){
          var s=filtered[active];
          var href=s.slug||s.s;
          var line=s.line;
          window.location.href=ROOT+href+'/'+IDXFILE+(line?'#line-'+line:'');
        }
      }
    });
  })();
  </script>
  

<script>
function toggleEdit() {
  var vm = document.getElementById('view-mode');
  var em = document.getElementById('edit-mode');
  if (em.style.display === 'none') {
    em.style.display = 'block';
    vm.style.display = 'none';
  } else {
    em.style.display = 'none';
    vm.style.display = 'block';
  }
}
async function saveEdit() {
  var content = document.getElementById('editor').value;
  var res = await fetch(window.location.pathname, {
    method: 'PUT',
    headers: { 'Content-Type': 'text/plain; charset=utf-8' },
    body: content
  });
  if (res.ok) window.location.reload();
  else alert('Save failed: ' + res.status);
}
</script>



</body>
</html>