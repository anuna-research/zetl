/* Backlinks grouping (task-backlinks-grouping).
   Progressive enhancement: pre-rendered flat list stays usable if this
   script fails. Regroups <li>s into folder sections and collapses any
   list with >5 items past the first 5 into a "Show N more" button.
   Externalised from page.html to stay under the 200 KB template budget. */
(function(){
  /* Inject the small CSS this needs; page.html carries no per-theme styles.
     Runs before the grouping so the folder headings render with the right
     type treatment on first paint. */
  if (!document.getElementById('zetl-backlinks-style')) {
    var s = document.createElement('style');
    s.id = 'zetl-backlinks-style';
    s.textContent = ''
      + '.zetl-backlink-hidden{display:none}'
      + '.zetl-backlink-folder{margin-top:.6rem;font-size:.65rem;font-weight:600;letter-spacing:.08em;text-transform:uppercase;opacity:.5}'
      + '.zetl-backlink-folder:first-child{margin-top:0}'
      + '.zetl-backlink-more button{font-size:.75rem;color:oklch(var(--p));background:transparent;border:0;padding:.15rem 0;cursor:pointer}'
      + '.zetl-backlink-more button:hover{text-decoration:underline}';
    document.head.appendChild(s);
  }
})();
(function(){
  var ul = document.querySelector('ul.zetl-backlinks');
  if (!ul) return;
  var items = Array.prototype.slice.call(ul.querySelectorAll('.zetl-backlink'));
  if (items.length < 2) return;

  var SHOW_LIMIT = 5;

  var buckets = {};
  items.forEach(function(li){
    var slug = li.getAttribute('data-slug') || '';
    var top = slug.indexOf('/') > 0 ? slug.slice(0, slug.indexOf('/')) : '';
    var key = top || '\u0000root';
    (buckets[key] = buckets[key] || []).push(li);
  });
  var keys = Object.keys(buckets).sort(function(a,b){
    if (a === '\u0000root') return 1;
    if (b === '\u0000root') return -1;
    return a.localeCompare(b);
  });
  if (keys.length < 2 && items.length <= SHOW_LIMIT) return;

  var frag = document.createDocumentFragment();
  keys.forEach(function(key){
    var bucket = buckets[key];
    var label = key === '\u0000root' ? 'root' : key;
    if (keys.length > 1) {
      var h = document.createElement('li');
      h.className = 'zetl-backlink-folder';
      h.textContent = label;
      frag.appendChild(h);
    }
    bucket.forEach(function(li, i){
      if (i >= SHOW_LIMIT) li.classList.add('zetl-backlink-hidden');
      frag.appendChild(li);
    });
    if (bucket.length > SHOW_LIMIT) {
      var more = document.createElement('li');
      more.className = 'zetl-backlink-more';
      var btn = document.createElement('button');
      btn.type = 'button';
      btn.textContent = 'Show ' + (bucket.length - SHOW_LIMIT) + ' more';
      btn.setAttribute('aria-expanded', 'false');
      btn.addEventListener('click', function(){
        bucket.forEach(function(li){ li.classList.remove('zetl-backlink-hidden'); });
        btn.setAttribute('aria-expanded', 'true');
        more.hidden = true;
      });
      more.appendChild(btn);
      frag.appendChild(more);
    }
  });
  ul.innerHTML = '';
  ul.appendChild(frag);
})();
