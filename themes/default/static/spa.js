/* zetl SPA navigation shell — SPEC-028 REQ-113 / REQ-115 / OBS-113.
   Intercepts same-origin <a> clicks, fetches the next document, and swaps the
   element carrying [data-zetl-volatile] (or <main> fallback) so the persistent
   shell — sidebar, graph widget, scripts — is never unmounted. */
(function () {
  if (window.__zetlSpaMounted) return;
  window.__zetlSpaMounted = true;

  var VOLATILE = '[data-zetl-volatile]';
  function volatile(doc) { return doc.querySelector(VOLATILE) || doc.querySelector('main'); }
  function slugOf(url) {
    try { return new URL(url, location.href).pathname; }
    catch (e) { return url; }
  }
  function sameOrigin(href) {
    try { return new URL(href, location.href).origin === location.origin; }
    catch (e) { return false; }
  }
  /* The full-page graph route (/_graph or /_graph.html) suppresses the
     persistent mini-widget from the shell via `{% if active_slug != "_graph" %}`.
     SPA nav doesn't re-render the shell, so swapping into /_graph leaves two
     #zetl-graph in the DOM (mini + full, boot-guard blocks the full one), and
     swapping back out leaves the shell widgetless. Force a full reload on any
     transition that crosses /_graph in either direction. */
  function isGraphRoute(url) {
    try { return /^\/_graph(\.html)?(\/|$|[?#])/.test(new URL(url, location.href).pathname); }
    catch (e) { return false; }
  }
  /* The collab editor route (/edit/<slug>/) loads CodeMirror via
     <script type="module"> imports from esm.sh. SPA nav re-clones
     scripts but module import graphs don't always re-execute the way
     naive script cloning expects — the editor silently fails to
     mount on same-origin link clicks and only works after a full
     reload. Force a full page reload on any transition in or out of
     /edit/. */
  function isEditRoute(url) {
    try { return /^\/edit(\/|$)/.test(new URL(url, location.href).pathname); }
    catch (e) { return false; }
  }

  /* Persistent-shell links (sidebar, vault actions, wordmark) are rendered
     once with page-relative root_path hrefs (build mode: `../`, `../../`, …).
     SPA pushState changes the document location without re-rendering the shell,
     so on the next click those stale relative hrefs would resolve against an
     ever-deeper base and compound the path (#70). Pin them to absolute URLs
     now — while the document base still matches the page they were rendered
     for — so later pushState navigations can't shift their resolution.
     Links inside [data-zetl-volatile] are left untouched: they are swapped per
     navigation with markup built for the new page's correct root_path. */
  function freezeShellLinks() {
    var nodes = document.querySelectorAll('a[href]');
    for (var i = 0; i < nodes.length; i++) {
      var a = nodes[i];
      if (a.closest && a.closest(VOLATILE)) continue;
      var raw = a.getAttribute('href');
      if (!raw || raw[0] === '#') continue;
      /* a.href is the browser-resolved absolute against the still-correct
         base; writing it back as the literal href pins it. */
      a.setAttribute('href', a.href);
    }
  }
  freezeShellLinks();

  function runScripts(root) {
    /* Inline + same-origin <script> tags in swapped content don't execute on
       innerHTML replace — clone + re-insert so the browser runs them. */
    var scripts = root.querySelectorAll('script');
    for (var i = 0; i < scripts.length; i++) {
      var s = scripts[i], n = document.createElement('script');
      for (var j = 0; j < s.attributes.length; j++) n.setAttribute(s.attributes[j].name, s.attributes[j].value);
      n.text = s.textContent;
      s.parentNode.replaceChild(n, s);
    }
  }

  function swap(doc, url, push) {
    var oldRoot = volatile(document), newRoot = volatile(doc);
    if (!oldRoot || !newRoot) { location.href = url; return; }
    var fromSlug = slugOf(location.href), toSlug = slugOf(url);
    var before = new CustomEvent('zetl:before-navigate', {
      cancelable: true, detail: { fromSlug: fromSlug, toSlug: toSlug, url: url }
    });
    if (!window.dispatchEvent(before)) { location.href = url; return; }
    if (doc.title) document.title = doc.title;
    oldRoot.replaceWith(newRoot);
    runScripts(newRoot);
    if (push) history.pushState({ zetl: true, url: url }, '', url);
    /* Forward nav matches browser default: scroll to top, or to #hash if
       one is present. popstate (back/forward) skips this so the browser's
       built-in scroll restoration can place the reader where they were.
       Hash targets (e.g. backlink #line-42) also get a flash highlight
       applied via JS — the shell CSS uses :has(> .line-anchor:target)
       but :target doesn't reliably fire on pushState, so we stand in. */
    if (push) {
      var hash = '';
      try { hash = new URL(url, location.href).hash; } catch (e) {}
      if (hash && hash.length > 1) {
        var tgt = document.getElementById(hash.slice(1));
        if (tgt && tgt.scrollIntoView) {
          tgt.scrollIntoView();
          var parent = tgt.parentElement;
          if (parent) {
            parent.classList.remove('line-anchor-flash');
            /* force reflow so the re-added class restarts the animation */
            void parent.offsetWidth;
            parent.classList.add('line-anchor-flash');
            setTimeout(function(){ parent.classList.remove('line-anchor-flash'); }, 2100);
          }
        } else {
          window.scrollTo(0, 0);
        }
      } else {
        window.scrollTo(0, 0);
      }
    }
    window.dispatchEvent(new CustomEvent('zetl:after-navigate', {
      detail: { slug: toSlug, contentRoot: newRoot }
    }));
    /* OBS-113: measure spans link-click interception → after-navigate dispatch. */
    try { performance.measure('zetl:navigate', 'zetl:navigate:start'); } catch (e) {}
  }

  /* Hosts with SPA-style 404 fallback (e.g. Cloudflare Pages when the build
     has no 404.html) answer ANY unknown path with 200 + the homepage
     document, so `r.ok` alone can't detect a broken link — swap() would
     silently render the homepage under the phantom URL (#73). Every page
     rendered through base.html self-identifies via <body data-slug>; before
     swapping, verify the fetched document actually claims to be the page the
     URL points at, and hard-navigate on mismatch so the host's real
     404/redirect behaviour stays visible.

     URL → expected identity: the path stem (pathname minus a trailing
     /index.html, .html, or slashes) must end with "/" + the claimed slug,
     optionally followed by /_history (the page-history route renders with
     its page's slug). Documents that claim the empty slug (vault index) are
     only accepted at the site root, which is located at boot from the
     current — known-correct — document. Docs without the attribute (custom
     vault templates) are exempt: no claim, no verdict. */
  function pathStem(url) {
    var p = new URL(url, location.href).pathname;
    try { p = decodeURIComponent(p); } catch (e) {}
    return p.replace(/\/index\.html$/, '').replace(/\.html$/, '').replace(/\/+$/, '');
  }
  var ROOT_STEM = (function () {
    var claimed = document.body ? document.body.getAttribute('data-slug') : null;
    if (claimed === null) return null;
    var stem;
    try { stem = pathStem(location.href); } catch (e) { return null; }
    if (claimed === '') return stem;
    var tails = ['/' + claimed, '/' + claimed + '/_history'];
    for (var i = 0; i < tails.length; i++) {
      if (stem.slice(-tails[i].length) === tails[i]) {
        return stem.slice(0, stem.length - tails[i].length);
      }
    }
    return null; /* boot doc's own claim doesn't match its URL — no root fix */
  })();
  function docMatchesUrl(doc, url) {
    var claimed = doc.body && doc.body.getAttribute('data-slug');
    if (claimed === null || claimed === undefined) return true;
    var stem;
    try { stem = pathStem(url); } catch (e) { return true; }
    if (claimed === '') return ROOT_STEM !== null && stem === ROOT_STEM;
    var tails = ['/' + claimed, '/' + claimed + '/_history'];
    for (var i = 0; i < tails.length; i++) {
      if (stem.slice(-tails[i].length) === tails[i]) return true;
    }
    return false;
  }

  function navigate(url, push) {
    fetch(url, { credentials: 'same-origin', headers: { 'Accept': 'text/html' } })
      .then(function (r) { if (!r.ok) throw r.status; return r.text(); })
      .then(function (html) {
        var doc = new DOMParser().parseFromString(html, 'text/html');
        if (!docMatchesUrl(doc, url)) { location.href = url; return; }
        swap(doc, url, push);
      })
      .catch(function () { location.href = url; });
  }

  document.addEventListener('click', function (e) {
    if (e.defaultPrevented || e.button !== 0) return;
    if (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
    var a = e.target.closest && e.target.closest('a[href]');
    if (!a) return;
    var href = a.getAttribute('href');
    if (!href || href[0] === '#' || a.target === '_blank' || a.hasAttribute('download')) return;
    if (a.dataset.zetlSpa === 'off') return;
    if (!sameOrigin(a.href)) return;
    if (a.href.replace(/#.*$/, '') === location.href.replace(/#.*$/, '')) return;
    if (isGraphRoute(location.href) || isGraphRoute(a.href)) return;
    if (isEditRoute(location.href) || isEditRoute(a.href)) return;
    e.preventDefault();
    /* OBS-113: mark click interception — the measure on after-navigate dispatch
       reports total navigation-to-paint latency for NFR assertions. */
    try { performance.mark('zetl:navigate:start'); } catch (e2) {}
    navigate(a.href, true);
  });

  window.addEventListener('popstate', function () {
    try { performance.mark('zetl:navigate:start'); } catch (e) {}
    navigate(location.href, false);
  });
})();
