/* Home-page sort/filter toolbar + pagination + /_search focus hook.
   Sourced from themes/default/index.html to keep the bundled-template
   byte budget (NFR-014-002) under 200 KB. */
(function(){
  var q = new URLSearchParams(window.location.search);
  if (q.get('focus') === 'search' && typeof window.openSearch === 'function') {
    setTimeout(window.openSearch, 50);
    q.delete('focus');
    var s = q.toString();
    var url = window.location.pathname + (s ? '?' + s : '') + window.location.hash;
    window.history.replaceState(null, '', url);
  }
})();
(function(){
  var grid = document.querySelector('.zetl-index-grid');
  if (!grid) return;
  var cards = Array.prototype.slice.call(grid.querySelectorAll('.zetl-index-card'));
  if (!cards.length) return;
  var sortBtns = document.querySelectorAll('.zetl-index-sort');
  var folderSel = document.querySelector('.zetl-index-folder');
  var counter = document.querySelector('.zetl-index-count');
  var pageState = { showAll: false };

  /* Show-N-more button uses style.display — DaisyUI's .btn sets
     display:inline-flex which wins over the HTML [hidden] attribute. */
  var showMoreBtn = document.createElement('button');
  showMoreBtn.type = 'button';
  showMoreBtn.style.display = 'none';
  showMoreBtn.className = 'btn btn-sm btn-outline mt-4 mx-auto zetl-index-more';
  showMoreBtn.setAttribute('aria-expanded', 'false');
  showMoreBtn.addEventListener('click', function(){
    pageState.showAll = true;
    showMoreBtn.setAttribute('aria-expanded', 'true');
    apply();
  });
  grid.parentNode.insertBefore(showMoreBtn, grid.nextSibling);

  /* Template ships compact attrs: data-s = slug, data-m = "out,in".
     Title is the h3.card-title text. Everything else is derived here. */
  var topFolders = {};
  cards.forEach(function(c){
    var slug = c.getAttribute('data-s') || '';
    var meta = (c.getAttribute('data-m') || '0,0').split(',');
    var out = +meta[0] || 0, back = +meta[1] || 0;
    var lastSlash = slug.lastIndexOf('/');
    var folder = lastSlash > 0 ? slug.slice(0, lastSlash) : '';
    c.dataset.slug = slug;
    c.dataset.folder = folder;
    c.dataset.title = (c.querySelector('.card-title') || {}).textContent || '';
    c.dataset.linksTotal = String(out + back);
    c.dataset.orphan = back === 0 ? '1' : '0';
    if (folder) {
      topFolders[folder.split('/')[0]] = true;
      var label = c.querySelector('.zetl-index-card-folder');
      if (label) { label.textContent = folder; label.hidden = false; }
    }
  });
  Object.keys(topFolders).sort().forEach(function(f){
    var o = document.createElement('option');
    o.value = f; o.textContent = f;
    folderSel.appendChild(o);
  });

  /* Build a heading row per top-level folder (plus one for root-level
     pages). They sit inside the grid as `grid-column: 1 / -1` full-width
     rows and are positioned by `order` so they slot in between the
     cards when sort=folder. Hidden otherwise. */
  var folderHeadings = {};
  function ensureHeading(folder){
    if (folderHeadings[folder]) return folderHeadings[folder];
    var h = document.createElement('div');
    h.className = 'zetl-index-folder-heading';
    h.style.gridColumn = '1 / -1';
    h.style.display = 'none';
    var label = folder ? folder + '/' : '(root)';
    h.innerHTML = '<h3 class="text-sm font-semibold uppercase tracking-wide opacity-60 mt-4">' +
      label.replace(/[&<>]/g, function(c){ return ({'&':'&amp;','<':'&lt;','>':'&gt;'})[c]; }) +
      '</h3>';
    grid.appendChild(h);
    folderHeadings[folder] = h;
    return h;
  }
  /* Default sort: "folder" when the vault has >=2 top-level folders
     (e.g. concepts/ + papers/), else "alpha" — flat vaults don't need
     headings. */
  var topFolderKeys = Object.keys(topFolders);
  /* Account for the implicit root group: if there are any cards with no
     folder, root counts as a group too. */
  var hasRootGroup = cards.some(function(c){ return !c.dataset.folder; });
  var defaultSort = (topFolderKeys.length + (hasRootGroup ? 1 : 0) >= 2) ? 'folder' : 'alpha';

  function getParams(){
    var q = new URLSearchParams(window.location.search);
    return {
      sort: q.get('sort') || defaultSort,
      folder: q.get('folder') || '',
      filter: q.get('filter') || ''
    };
  }
  function setParams(p){
    var q = new URLSearchParams();
    if (p.sort && p.sort !== defaultSort) q.set('sort', p.sort);
    if (p.folder) q.set('folder', p.folder);
    if (p.filter) q.set('filter', p.filter);
    var s = q.toString();
    var url = window.location.pathname + (s ? '?' + s : '') + window.location.hash;
    window.history.replaceState(null, '', url);
  }

  /* Dead-link detection: pages with an outgoing edge whose target is a phantom
     (is_dead) node. Fetched lazily from the graph index on first ?filter=dead. */
  var deadSources = null;
  var deadPromise = null;
  function loadDeadSources(){
    if (deadSources) return Promise.resolve(deadSources);
    if (deadPromise) return deadPromise;
    deadPromise = fetch('/graph-index.json', { credentials: 'same-origin' })
      .then(function(r){ return r.ok ? r.json() : null; })
      .then(function(g){
        if (!g || !g.nodes || !g.edges) { deadSources = {}; return deadSources; }
        var dead = {};
        g.nodes.forEach(function(n){ if (n.attributes && n.attributes.is_dead) dead[n.key] = true; });
        var srcs = {};
        g.edges.forEach(function(e){ if (dead[e.target]) srcs[e.source] = true; });
        deadSources = srcs;
        return srcs;
      })
      .catch(function(){ deadSources = {}; return deadSources; });
    return deadPromise;
  }

  function apply(){
    var p = getParams();
    sortBtns.forEach(function(b){
      b.setAttribute('aria-pressed', b.getAttribute('data-sort') === p.sort ? 'true' : 'false');
    });
    folderSel.value = p.folder;

    var ordered = cards.slice();
    if (p.sort === 'linked') {
      ordered.sort(function(a, b){
        return (+b.dataset.linksTotal) - (+a.dataset.linksTotal)
            || a.dataset.title.localeCompare(b.dataset.title);
      });
    } else if (p.sort === 'folder') {
      ordered.sort(function(a, b){
        var fa = (a.dataset.folder || '').split('/')[0];
        var fb = (b.dataset.folder || '').split('/')[0];
        return fa.localeCompare(fb)
            || a.dataset.title.localeCompare(b.dataset.title);
      });
    } else {
      ordered.sort(function(a, b){
        return a.dataset.title.localeCompare(b.dataset.title);
      });
    }

    /* Hide all folder headings up front; when sort=folder we'll show
       the ones that have at least one visible card under them. */
    Object.keys(folderHeadings).forEach(function(k){
      folderHeadings[k].style.display = 'none';
    });

    var needsDead = p.filter === 'dead';
    /* PAGE_SIZE threshold: vaults under it render everything; past it the
       first N visible cards render and the rest hide behind "Show more". */
    var PAGE_SIZE = 60;
    var proceed = function(){
      var visible = 0, shown = 0, pastLimit = 0;
      /* Allocate ordinals as we go. Headings get their own ordinal so
         they slot before the first card of each group; cards get an
         ordinal too. We use ordinal multiples of 2 for cards and
         insert headings at odd ordinals immediately before their
         first-visible card, so the grid renders headings ahead of
         their pages even though they live at the END of the DOM. */
      var lastFolder = null;
      var groupingByFolder = (p.sort === 'folder');
      var ord = 0;
      ordered.forEach(function(c){
        var show = true;
        var top = (c.dataset.folder || '').split('/')[0];
        if (p.folder && top !== p.folder) show = false;
        if ((p.sort === 'orphans' || p.filter === 'orphans') && c.dataset.orphan !== '1') show = false;
        if (needsDead && !(deadSources && deadSources[c.dataset.slug])) show = false;
        if (show) {
          /* In folder mode, surface a heading whenever the top-level
             folder transitions. The heading row goes BEFORE the next
             card via flex/grid `order`. */
          if (groupingByFolder && top !== lastFolder) {
            var h = ensureHeading(top);
            h.style.display = '';
            h.style.order = String(ord++);
            lastFolder = top;
          }
          visible++;
          if (!pageState.showAll && cards.length > PAGE_SIZE && shown >= PAGE_SIZE) {
            c.style.display = 'none';
            pastLimit++;
          } else {
            c.style.display = '';
            shown++;
          }
          c.style.order = String(ord++);
        } else {
          c.style.display = 'none';
          c.style.order = '';
        }
      });
      var filtered = !!p.folder || p.filter === 'dead' || p.filter === 'orphans' || p.sort === 'orphans';
      counter.textContent = filtered ? (visible + ' / ' + cards.length) : '';
      if (pastLimit > 0) {
        showMoreBtn.style.display = '';
        showMoreBtn.textContent = 'Show ' + pastLimit + ' more';
      } else {
        showMoreBtn.style.display = 'none';
      }
    };
    if (needsDead) loadDeadSources().then(proceed);
    else proceed();
  }

  sortBtns.forEach(function(b){
    b.addEventListener('click', function(){
      var p = getParams();
      p.sort = b.getAttribute('data-sort');
      /* Any explicit sort pill clears leftover ?filter= from a stat-card click. */
      p.filter = '';
      setParams(p);
      apply();
    });
  });
  folderSel.addEventListener('change', function(){
    var p = getParams();
    p.folder = folderSel.value;
    setParams(p);
    apply();
  });
  apply();
})();
