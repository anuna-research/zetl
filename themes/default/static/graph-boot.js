/* Resolve relative URLs in __zetlGraphConfig at script-load time, before
   the SPA shell can move document.baseURI. The boot function runs later
   (vendor lazy-load → user opens mobile overlay), which in build mode
   may be after an SPA nav — at which point "../graph-index.json" would
   resolve against the wrong page and 404. Pinning here fixes the fetch
   and keeps click-nav from slipping across subsequent navigations. */
(function(){
  var CFG = (typeof window !== 'undefined' && window.__zetlGraphConfig) || null;
  if (!CFG || CFG.__zetlResolved) return;
  try {
    if ('root' in CFG) CFG.root = new URL(CFG.root, document.baseURI).href;
    if (CFG.graphUrl) CFG.graphUrl = new URL(CFG.graphUrl, document.baseURI).href;
  } catch (e) {}
  CFG.__zetlResolved = true;
})();

window.__zetlBootGraph = function(){
  if (window.__zetlGraphBooted) return;
  window.__zetlGraphBooted = true;

  /* Config is emitted inline from _graph.html into window.__zetlGraphConfig
     so this file itself is template-free and cacheable. Note: indexFile
     is legitimately the empty string in serve mode (URLs end in "/"), so
     we can't use `||` — that would coerce "" to the build-mode default
     and break node-click navigation in serve mode. */
  var CFG = (typeof window !== 'undefined' && window.__zetlGraphConfig) || {};
  var ROOT = 'root' in CFG ? CFG.root : "";
  var INDEX_FILE = 'indexFile' in CFG ? CFG.indexFile : "index.html";
  var GRAPH_URL = 'graphUrl' in CFG ? CFG.graphUrl : "";
  /* In build mode ROOT is relative to the initial page's location (`./`,
     `../`, etc.). Resolve it once against the boot-time baseURI; later
     click-navigations must not re-resolve against document.baseURI
     because SPA pushState moves it, producing nested `/foo/bar/` paths
     on the second click. */
  var BASE_HREF;
  try { BASE_HREF = new URL(ROOT, document.baseURI).href; }
  catch (e) { BASE_HREF = document.baseURI; }
  var MODE_KEY = "zetl:graph:mode";
  var MODE_FALLBACK = "local";

  var container = document.getElementById('zetl-graph');
  if (!container) return;
  var widget = container.closest('[data-mode]') || container.parentElement || container;

  /* ── CSS custom-property palette (REQ-114) ───────────────────────────── */
  function cssVar(name, fallback) {
    try {
      var v = getComputedStyle(widget).getPropertyValue(name).trim();
      return v || fallback;
    } catch (e) { return fallback; }
  }
  function readPalette() {
    return {
      node:       cssVar('--zetl-graph-node',       '#6366f1'),
      nodeDead:   cssVar('--zetl-graph-node-dead',  '#9ca3af'),
      nodeFaded:  cssVar('--zetl-graph-node-faded', 'rgba(99,102,241,0.25)'),
      nodeActive: cssVar('--zetl-graph-node-active','#f59e0b'),
      edge:       cssVar('--zetl-graph-edge',       '#9ca3af'),
      edgeDead:   cssVar('--zetl-graph-edge-dead',  'rgba(156,163,175,0.6)'),
      edgeFaded:  cssVar('--zetl-graph-edge-faded', 'rgba(156,163,175,0.15)'),
      label:      cssVar('--zetl-graph-label',      '#1f2937'),
      labelFont:  cssVar('--zetl-graph-label-font', 'ui-sans-serif, system-ui, sans-serif')
    };
  }
  var palette = readPalette();

  /* ── Mode state (REQ-108) ────────────────────────────────────────────── */
  function restoreMode(){
    try {
      var saved = sessionStorage.getItem(MODE_KEY);
      if (saved) widget.setAttribute('data-mode', saved);
    } catch (e) {}
    if (!widget.getAttribute('data-mode')) widget.setAttribute('data-mode', MODE_FALLBACK);
  }
  restoreMode();
  function currentMode(){ return widget.getAttribute('data-mode') || MODE_FALLBACK; }
  function setMode(mode){
    if (mode !== 'local' && mode !== 'vault' && mode !== 'off') return;
    widget.setAttribute('data-mode', mode);
    try { sessionStorage.setItem(MODE_KEY, mode); } catch (e) {}
  }

  /* ── Graph/renderer state ────────────────────────────────────────────── */
  var graph = null, renderer = null, layoutSupervisor = null;
  var hasDashedProgram = false;
  var activeSlug = document.body.getAttribute('data-slug') || '';
  var neighbourSet = null;
  var hoveredNode = null;
  var hoveredNeighbours = null;

  function computeNeighbours(slug){
    neighbourSet = null;
    if (!graph || !slug || !graph.hasNode(slug)) return;
    neighbourSet = new Set([slug]);
    graph.forEachNeighbor(slug, function(n){ neighbourSet.add(n); });
  }

  /* ── External filter (task-graph-search-highlight, task-graph-filter-degree)
     The fullscreen /_graph host calls window.__zetlGraph.setFilter({search,minDegree}).
     Nodes not matching become fully hidden from the visible graph; matching
     nodes render at full prominence. Edges whose endpoints are hidden vanish
     too. Both predicates AND together. */
  var filterState = { search: '', minDegree: 0 };
  var focusState  = { slug: '', depth: 1, reachable: null };
  /* Memoised BFS results keyed by "slug@depth". Invalidated on graph rebuild
     (see onGraphReload below). Keeps focus-slider input responsive on large
     graphs where the walk is non-trivial. */
  var focusCache = Object.create(null);

  function recomputeFocus(){
    focusState.reachable = null;
    if (!graph || !focusState.slug || !graph.hasNode(focusState.slug)) return;
    var depth = Math.max(0, focusState.depth | 0);
    var key = focusState.slug + '@' + depth;
    var cached = focusCache[key];
    if (cached) { focusState.reachable = cached; return; }
    var reach = new Set([focusState.slug]);
    var frontier = [focusState.slug];
    for (var h = 0; h < depth; h++) {
      var next = [];
      for (var i = 0; i < frontier.length; i++) {
        graph.forEachNeighbor(frontier[i], function(n){
          if (!reach.has(n)) { reach.add(n); next.push(n); }
        });
      }
      frontier = next;
      if (!frontier.length) break;
    }
    focusState.reachable = reach;
    focusCache[key] = reach;
  }
  /* Invalidate the cache whenever the underlying graph is replaced — called
     from the data-load path if the host ever swaps graphs at runtime. */
  function invalidateFocusCache(){ focusCache = Object.create(null); }

  function passesFilter(node, attrs){
    if (focusState.reachable && !focusState.reachable.has(node)) return false;
    if (filterState.minDegree > 0) {
      var d = (attrs.outlink_count || 0) + (attrs.backlink_count || 0);
      if (d < filterState.minDegree) return false;
    }
    if (filterState.search) {
      var hay = ((attrs.label || '') + ' ' + (attrs.slug || node)).toLowerCase();
      if (hay.indexOf(filterState.search) < 0) return false;
    }
    return true;
  }

  /* ── Folder colouring (task-graph-folder-color) ──────────────────────────
     Top-level folder of a slug → deterministic HSL hue. Root-level pages and
     dead phantoms do NOT get coloured — they keep the palette/dead defaults.
     Widgets can opt out by setting data-folder-colour="off" on the host. */
  var folderColoursEnabled = widget.getAttribute('data-folder-colour') !== 'off';
  var folderColourCache = Object.create(null);
  function topFolder(slug){
    if (!slug) return '';
    var i = slug.indexOf('/');
    return i < 0 ? '' : slug.slice(0, i);
  }
  function hashCode(s){
    var h = 0, i;
    for (i = 0; i < s.length; i++) { h = ((h << 5) - h + s.charCodeAt(i)) | 0; }
    return Math.abs(h);
  }
  function hslToHex(h, s, l){
    /* h in [0,360), s and l in [0,1] */
    var c = (1 - Math.abs(2*l - 1)) * s;
    var hp = h / 60;
    var x = c * (1 - Math.abs((hp % 2) - 1));
    var r=0,g=0,b=0;
    if (hp < 1) { r=c; g=x; }
    else if (hp < 2) { r=x; g=c; }
    else if (hp < 3) { g=c; b=x; }
    else if (hp < 4) { g=x; b=c; }
    else if (hp < 5) { r=x; b=c; }
    else { r=c; b=x; }
    var m = l - c/2;
    function hex(v){ v = Math.round((v + m) * 255); return ('0' + v.toString(16)).slice(-2); }
    return '#' + hex(r) + hex(g) + hex(b);
  }
  function folderColour(folder){
    if (!folder) return null;
    if (folderColourCache[folder]) return folderColourCache[folder];
    var hue = hashCode(folder) % 360;
    /* Sigma's WebGL node program expects hex; hsl() strings render as
       near-black on some builds. Convert at cache-write. */
    var c = hslToHex(hue, 0.62, 0.55);
    folderColourCache[folder] = c;
    return c;
  }
  function folderLegend(){
    if (!graph) return [];
    var seen = Object.create(null);
    graph.forEachNode(function(n, attrs){
      if (attrs && attrs.is_dead) return;
      var f = topFolder(attrs && attrs.slug || n);
      if (!f || seen[f]) return;
      seen[f] = true;
    });
    var out = Object.keys(seen).sort().map(function(f){
      return { folder: f, colour: folderColour(f) };
    });
    return out;
  }

  /* ── Reducers (read palette every frame) ─────────────────────────────── */
  /* REQ-112: dead-link (is_dead:true) nodes and their terminating edges
     render with the muted palette and — for edges — Sigma's "dashed" type.
     This matches the muted treatment used elsewhere in the UI for phantom
     link targets. Active-node highlighting still wins over dead styling. */
  function nodeReducer(node, attrs){
    var out = {};
    for (var k in attrs) out[k] = attrs[k];
    out.label = attrs.label || attrs.slug || node;
    var weight = (attrs.outlink_count || 0) + (attrs.backlink_count || 0);
    out.size = Math.max(3, 3 + Math.sqrt(weight));

    /* Apply external filter before other decorations: non-matching nodes
       are hidden entirely. The edge reducer hides edges with hidden endpoints. */
    if (!passesFilter(node, attrs)) {
      out.hidden = true;
      return out;
    }

    var isDead = !!attrs.is_dead;
    if (isDead) {
      out.color = palette.nodeDead;
      out.size = Math.max(2, out.size * 0.75);
      out.zIndex = 0;
    } else if (folderColoursEnabled) {
      var folderC = folderColour(topFolder(attrs.slug || node));
      out.color = folderC || palette.node;
    } else {
      out.color = palette.node;
    }

    /* Focus target gets the active treatment (colour + size + label) so the
       viewer can tell at a glance which node the subgraph is centred on.
       Suppressed while the user hovers a different node — hover owns the
       temporary styling; the focus ring resumes when the hover ends. */
    if (focusState.slug && node === focusState.slug && !hoveredNode) {
      out.color = palette.nodeActive;
      out.size = out.size * 1.6;
      out.highlighted = true;
      out.zIndex = 2;
    }

    /* Hover: highlight hovered node + its neighbours, fade everything else */
    if (hoveredNode && hoveredNeighbours) {
      if (node === hoveredNode) {
        out.color = palette.nodeActive;
        out.size = out.size * 1.4;
        out.highlighted = true;
        out.zIndex = 2;
      } else if (hoveredNeighbours.has(node)) {
        /* keep current color, show label */
        out.zIndex = 1;
      } else {
        out.color = palette.nodeFaded;
        out.label = '';
        out.zIndex = 0;
      }
      return out;
    }

    var mode = currentMode();
    if (mode === 'local' && activeSlug && neighbourSet) {
      if (node === activeSlug) {
        out.color = palette.nodeActive;
        out.size = out.size * 1.6;
        out.highlighted = true;
        out.zIndex = 2;
      } else if (!neighbourSet.has(node)) {
        out.color = palette.nodeFaded;
        out.label = '';
      }
    }
    return out;
  }
  function edgeReducer(edge, attrs){
    var out = {};
    for (var k in attrs) out[k] = attrs[k];
    var srcAttrs = graph.getNodeAttributes(graph.source(edge));
    var targetAttrs = graph.getNodeAttributes(graph.target(edge));

    /* Hide edge if either endpoint is filtered out. */
    if (!passesFilter(graph.source(edge), srcAttrs) || !passesFilter(graph.target(edge), targetAttrs)) {
      out.hidden = true;
      return out;
    }

    var isDead = !!attrs.is_dead || !!(targetAttrs && targetAttrs.is_dead);
    if (isDead) {
      out.color = palette.edgeDead;
      if (hasDashedProgram) out.type = 'dashed';
      out.zIndex = 0;
    } else {
      out.color = palette.edge;
    }

    /* Hover: only show edges connected to the hovered node */
    if (hoveredNode && hoveredNeighbours) {
      var s = graph.source(edge), t = graph.target(edge);
      if (s === hoveredNode || t === hoveredNode) {
        out.color = palette.edge;
        out.size = 2;
        out.zIndex = 1;
      } else {
        out.hidden = true;
      }
      return out;
    }

    var mode = currentMode();
    if (mode === 'local' && activeSlug && neighbourSet) {
      var s = graph.source(edge), t = graph.target(edge);
      if (!neighbourSet.has(s) || !neighbourSet.has(t)) {
        out.color = palette.edgeFaded;
      }
    }
    return out;
  }

  /* ── Mode application: reducer-only refresh, no relayout ─────────────── */
  function applyMode(){
    if (!renderer) return;
    var mode = currentMode();
    if (mode === 'local') computeNeighbours(activeSlug);
    else neighbourSet = null;
    renderer.refresh();
    if (mode === 'vault' && renderer.getCamera) {
      try { renderer.getCamera().animatedReset({ duration: 250 }); } catch (e) {}
    }
  }

  /* ── Data loading (inline → fetch) ───────────────────────────────────── */
  function loadGraph(){
    var inline = document.getElementById('zetl-graph-index');
    if (inline && inline.textContent.trim()) {
      try { return Promise.resolve(JSON.parse(inline.textContent)); }
      catch (e) { /* fall through to fetch */ }
    }
    if (!GRAPH_URL) return Promise.resolve(null);
    if (typeof fetch !== 'function') return Promise.resolve(null);
    return fetch(GRAPH_URL, { credentials: 'same-origin' })
      .then(function(r){ return r.ok ? r.json() : null; })
      .catch(function(){ return null; });
  }

  /* ── FA2: Web Worker when available, sync fallback ───────────────────── */
  function resolveFA2(){
    return window.graphologyLayoutForceatlas2
        || window.graphologyLibForceatlas2
        || window.forceAtlas2
        || null;
  }
  function resolveFA2Supervisor(){
    var fa2 = resolveFA2();
    if (fa2 && fa2.FA2LayoutSupervisor) return fa2.FA2LayoutSupervisor;
    return window.FA2LayoutSupervisor
        || window.graphologyLayoutForceatlas2Supervisor
        || null;
  }
  function runLayout(){
    var fa2 = resolveFA2();
    var Supervisor = resolveFA2Supervisor();
    var settings = (fa2 && typeof fa2.inferSettings === 'function')
      ? fa2.inferSettings(graph)
      : { barnesHutOptimize: graph.order > 500, strongGravityMode: true, gravity: 1 };

    if (Supervisor && typeof Worker !== 'undefined') {
      try {
        layoutSupervisor = new Supervisor(graph, { settings: settings });
        layoutSupervisor.start();
        setTimeout(function(){
          if (layoutSupervisor) { try { layoutSupervisor.stop(); } catch (e) {} }
          onLayoutStop();
        }, 2000);
        return;
      } catch (e) { /* fall through to sync */ }
    }
    if (fa2 && typeof fa2.assign === 'function') {
      fa2.assign(graph, { iterations: 150, settings: settings });
    }
    onLayoutStop();
  }

  function onLayoutStop(){
    try { performance.measure('zetl:graph:render', 'zetl:graph:render:start'); }
    catch (e) { /* marks may be disabled */ }
    if (renderer) renderer.refresh();
    window.dispatchEvent(new CustomEvent('zetl:graph:layoutstop'));
  }

  /* ── Boot: runs after vendor scripts load ────────────────────────────── */
  function boot(){
    if (!window.graphology || !window.Sigma) return false;
    var DirectedGraph = window.graphology.DirectedGraph || window.graphology.Graph;
    if (!DirectedGraph) return false;

    loadGraph().then(function(data){
      if (!data) return;
      try {
        graph = new DirectedGraph();
        graph.import(data);
      } catch (e) { return; }

      /* FA2 requires initial coordinates; seed randomly if absent. */
      graph.forEachNode(function(n, a){
        if (typeof a.x !== 'number' || typeof a.y !== 'number') {
          graph.setNodeAttribute(n, 'x', Math.random());
          graph.setNodeAttribute(n, 'y', Math.random());
        }
      });

      /* REQ-117 follow-up: mobile + collapsed-docked states apply display:none
         to the widget, so the canvas has a 0×0 box at DOM-ready. Calling
         `new Sigma(...)` against such a container throws "Container has no
         width" and leaves the widget permanently broken once the user opens
         the overlay. Defer instantiation until the container actually has
         layout, then retry via a ResizeObserver on the canvas and a
         MutationObserver on the widget — whichever fires first when the
         overlay/launcher reveals the canvas. */
      function hasSize(){
        return container.offsetWidth > 0 && container.offsetHeight > 0;
      }
      function instantiate(){
        if (renderer) return true;
        if (!hasSize()) return false;

        try { performance.mark('zetl:graph:render:start'); } catch (e) {}

        /* REQ-112: register a "dashed" edge program so edges emitted with
           type:'dashed' by the reducer render with a broken stroke. Falls
           back to the line program where Sigma's dashed program is missing
           or on an older build — in that case the muted colour alone still
           conveys the dead-link state. */
        var edgePrograms = {};
        try {
          var sigmaNS = window.Sigma || {};
          var DashedProgram =
            (sigmaNS.edgePrograms && sigmaNS.edgePrograms.dashed)
            || sigmaNS.EdgeDashedProgram
            || window.EdgeDashedProgram
            || null;
          if (DashedProgram) edgePrograms.dashed = DashedProgram;
        } catch (e) {}
        hasDashedProgram = !!edgePrograms.dashed;

        var sigmaSettings = {
          nodeReducer: nodeReducer,
          edgeReducer: edgeReducer,
          labelFont: palette.labelFont,
          labelColor: { color: palette.label },
          renderLabels: true,
          defaultEdgeType: 'line',
          zIndex: true,
          /* Sigma's internal resize loop (ResizeObserver) fires a resize()
             whenever the container's width transitions through 0 — which
             happens at every SPA nav and every media-query toggle that
             hides the mini-widget. Setting this to true tells Sigma to
             skip the render pass instead of throwing. Our `hasSize()`
             probe still guards the initial `new Sigma(...)` call. */
          allowInvalidContainer: true,
          /* BUG-505: stagePadding keeps nodes off the canvas edge so
             labels extending to the right of a node don't clip at the
             viewport boundary. Sigma uses this value (in pixels) as a
             camera margin — larger on mobile where labels eat more of
             the horizontal budget per node, smaller on desktop where
             the canvas is wide enough to absorb edge labels naturally. */
          stagePadding: (typeof window !== 'undefined' && window.innerWidth < 900) ? 120 : 60
        };
        if (hasDashedProgram) sigmaSettings.edgeProgramClasses = edgePrograms;

        try {
          renderer = new window.Sigma(graph, container, sigmaSettings);
        } catch (e) {
          /* Lost the race — container reported size but Sigma still refused.
             Leave renderer null so the observer retries on the next tick. */
          renderer = null;
          return false;
        }

        /* REQ-111: node-click → page navigation.
           - Modifier keys (meta / ctrl / shift): open the slug in a new tab,
             mirroring the native <a>-click muscle memory.
           - SPA shell mounted: synthesise an anchor click so the
             document-level interceptor swaps `[data-zetl-volatile]` without a
             full reload.
           - SPA disabled / failed to boot: fall through to `location.href`
             for a standard full-page navigation. */
        /* Hover: show only connected nodes + edges */
        renderer.on('enterNode', function(evt){
          hoveredNode = evt.node;
          hoveredNeighbours = new Set();
          graph.forEachNeighbor(evt.node, function(n){ hoveredNeighbours.add(n); });
          if (hasSize()) renderer.refresh();
        });
        renderer.on('leaveNode', function(){
          hoveredNode = null;
          hoveredNeighbours = null;
          if (hasSize()) renderer.refresh();
        });

        /* Click-to-focus UX on the fullscreen /_graph view:
             click node (not yet focused) → focus at depth 1 (pan + highlight)
             click node (already focused)  → navigate to its page
             double-click (desktop fast path) → navigate
             modifier+click → open in new tab
           No timer / debounce — avoids the 300 ms mobile tap delay and
           feels instant on touch. The "is it already focused?" check is
           the only disambiguation, which is also intuitive: tap to learn
           more, tap again to go.

           The docked mini-widget on ordinary pages keeps the legacy
           single-click-to-navigate because there's no focus UX available
           in that small canvas — discovery there comes from clicking
           through. */
        var isFullscreen = widget.getAttribute('data-placement') === 'fullscreen';

        function navigateToSlug(slug, original) {
          var href;
          try { href = new URL(slug + '/' + INDEX_FILE, BASE_HREF).href; }
          catch (e) { href = ROOT + slug + '/' + INDEX_FILE; }
          if (original && (original.metaKey || original.ctrlKey || original.shiftKey)) {
            window.open(href, '_blank', 'noopener');
            return;
          }
          if (window.__zetlSpaMounted) {
            var a = document.createElement('a');
            a.href = href;
            a.rel = 'zetl-graph-nav';
            document.body.appendChild(a);
            a.click();
            document.body.removeChild(a);
            return;
          }
          location.href = href;
        }

        function focusOnNodeFromClick(slug) {
          focusState.slug = slug;
          focusState.depth = 1;
          recomputeFocus();
          /* Pan camera to the focus node so it's obvious we moved. */
          try {
            var a = graph.getNodeAttributes(slug);
            var cam = renderer.getCamera && renderer.getCamera();
            if (cam && typeof a.x === 'number' && typeof a.y === 'number') {
              cam.animate({ x: a.x, y: a.y, ratio: 0.4 }, { duration: 350 });
            }
          } catch (e) {}
          renderer.refresh();
          /* Reflect the new focus in the URL so refresh preserves it and the
             focus chip in vault_graph.html picks it up via applyFocusFromUrl.
             We also dispatch a synthetic event the chip listens to. */
          try {
            var url = new URL(location.href);
            url.searchParams.set('focus', slug);
            url.searchParams.set('depth', '1');
            history.replaceState(null, '', url.toString());
            window.dispatchEvent(new CustomEvent('zetl:graph:focus-changed', {
              detail: { slug: slug, depth: 1 }
            }));
          } catch (e) {}
        }

        renderer.on('clickNode', function(evt){
          var slug = evt.node;
          if (!slug) return;
          var orig = evt.event && evt.event.original;
          /* Modifier keys always open the page, never focus — matches the
             native <a>-click muscle memory for new-tab. */
          if (orig && (orig.metaKey || orig.ctrlKey || orig.shiftKey)) {
            navigateToSlug(slug, orig);
            return;
          }
          if (!isFullscreen) {
            navigateToSlug(slug, orig);
            return;
          }
          /* Fullscreen: clicking the already-focused node navigates; clicking
             any other node changes focus. */
          if (focusState.slug === slug) {
            navigateToSlug(slug, orig);
          } else {
            focusOnNodeFromClick(slug);
          }
        });

        /* Desktop fast-path: sigma emits doubleClickNode on mouse double-click
           (touch doesn't reliably emit it). Navigate immediately without
           requiring the click-focus-click dance. */
        renderer.on('doubleClickNode', function(evt){
          var slug = evt.node;
          if (!slug) return;
          var orig = evt.event && evt.event.original;
          navigateToSlug(slug, orig);
          /* Suppress sigma's built-in doubleclick-to-zoom. */
          if (evt.preventSigmaDefault) evt.preventSigmaDefault();
        });

        computeNeighbours(activeSlug);
        runLayout();
        applyMode();

        window.__zetlGraph = {
          getRenderer: function(){ return renderer; },
          getGraph: function(){ return graph; },
          setMode: setMode,
          getMode: currentMode,
          refreshPalette: function(){ palette = readPalette(); if (renderer && hasSize()) renderer.refresh(); },
          folderLegend: folderLegend,
          folderColour: folderColour,
          topFolder: topFolder,
          /* External filter surface. Call with partial object to merge. */
          setFilter: function(next){
            if (!next) return;
            if ('search' in next) filterState.search = String(next.search || '').toLowerCase().trim();
            if ('minDegree' in next) filterState.minDegree = Math.max(0, +next.minDegree || 0);
            if (renderer) renderer.refresh();
          },
          getFilter: function(){ return { search: filterState.search, minDegree: filterState.minDegree }; },
          /* Focus mode: restrict the visible graph to the N-hop neighbourhood
             of a named slug. Pass {slug: ''} to clear. */
          setFocus: function(next){
            if (!next) return;
            if ('slug' in next) focusState.slug = String(next.slug || '');
            if ('depth' in next) focusState.depth = Math.max(0, +next.depth || 0);
            recomputeFocus();
            if (renderer) {
              renderer.refresh();
              /* Pan + zoom the camera to the focus node so the viewer doesn't
                 have to scan the canvas looking for the highlighted ring. */
              if (focusState.slug && graph && graph.hasNode(focusState.slug)) {
                try {
                  var a = graph.getNodeAttributes(focusState.slug);
                  var cam = renderer.getCamera && renderer.getCamera();
                  if (cam && typeof a.x === 'number' && typeof a.y === 'number') {
                    cam.animate({ x: a.x, y: a.y, ratio: 0.4 }, { duration: 400 });
                  }
                } catch (e) {}
              } else if (!focusState.slug && renderer.getCamera) {
                /* Clearing focus → fit to the whole (filtered) graph. */
                try { renderer.getCamera().animatedReset({ duration: 300 }); } catch (e) {}
              }
            }
          },
          getFocus: function(){ return { slug: focusState.slug, depth: focusState.depth }; },
          /* Max degree in the current graph — lets hosts size their slider. */
          maxDegree: function(){
            var m = 0;
            if (!graph) return 0;
            graph.forEachNode(function(n, a){
              var d = (a.outlink_count || 0) + (a.backlink_count || 0);
              if (d > m) m = d;
            });
            return m;
          }
        };
        try { window.dispatchEvent(new CustomEvent('zetl:graph:ready', { detail: { legend: folderLegend() } })); } catch (e) {}
        return true;
      }

      if (!instantiate()) {
        var ro = null, mo = null, resizeHandler = null;
        function stopWaiting(){
          if (ro) { try { ro.disconnect(); } catch (e) {} ro = null; }
          if (mo) { try { mo.disconnect(); } catch (e) {} mo = null; }
          if (resizeHandler) {
            window.removeEventListener('resize', resizeHandler);
            resizeHandler = null;
          }
        }
        function retry(){ if (instantiate()) stopWaiting(); }
        if (typeof ResizeObserver === 'function') {
          try { ro = new ResizeObserver(retry); ro.observe(container); }
          catch (e) { ro = null; }
        }
        /* Mobile `--open` toggle and desktop `data-graph="expanded"` flips
           cascade into the canvas's display; ResizeObserver picks most of
           those up, but a class/attribute observer on the widget is a cheap
           belt-and-braces for browsers that skip RO on display transitions. */
        try {
          mo = new MutationObserver(retry);
          mo.observe(widget, {
            attributes: true,
            attributeFilter: ['class', 'style', 'data-mode', 'data-placement']
          });
          mo.observe(document.documentElement, {
            attributes: true,
            attributeFilter: ['data-graph']
          });
        } catch (e) { mo = null; }
        resizeHandler = retry;
        window.addEventListener('resize', resizeHandler);
      }
    });
    return true;
  }

  function whenVendorReady(tries){
    if (boot()) return;
    if (tries <= 0) return;
    setTimeout(function(){ whenVendorReady(tries - 1); }, 60);
  }
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', function(){ whenVendorReady(50); });
  } else {
    whenVendorReady(50);
  }

  /* ── Live observers ──────────────────────────────────────────────────── */
  /* [data-mode] mutation → reducer-only refresh (REQ-108). */
  try {
    new MutationObserver(function(muts){
      for (var i = 0; i < muts.length; i++) {
        if (muts[i].attributeName === 'data-mode') { applyMode(); return; }
      }
    }).observe(widget, { attributes: true, attributeFilter: ['data-mode'] });
  } catch (e) {}

  /* Theme mutation → re-read CSS vars. */
  function refreshPalette(){
    palette = readPalette();
    if (renderer) renderer.refresh();
  }
  try {
    new MutationObserver(refreshPalette).observe(document.documentElement, {
      attributes: true, attributeFilter: ['data-theme', 'class']
    });
  } catch (e) {}
  if (window.matchMedia) {
    try {
      var mm = window.matchMedia('(prefers-color-scheme: dark)');
      if (mm.addEventListener) mm.addEventListener('change', refreshPalette);
      else if (mm.addListener) mm.addListener(refreshPalette);
    } catch (e) {}
  }

  /* SPA navigation lifecycle (REQ-115): update active slug + mode, refresh,
     no Sigma re-instantiation. */
  window.addEventListener('zetl:after-navigate', function(e){
    var detail = e && e.detail ? e.detail : {};
    var root = detail.contentRoot || document.querySelector('[data-zetl-volatile]') || document.body;
    activeSlug = detail.slug || root.getAttribute('data-slug') || document.body.getAttribute('data-slug') || '';
    var url = detail.url || (typeof location !== 'undefined' ? location.pathname : '');
    var isVault = /(^|\/)_graph(\.html)?($|[?#])/.test(url) || url === ROOT || url === '/' || url === (ROOT + INDEX_FILE);
    setMode(isVault ? 'vault' : 'local');
    applyMode();
  });
};
