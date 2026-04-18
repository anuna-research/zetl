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
    window.dispatchEvent(new CustomEvent('zetl:after-navigate', {
      detail: { slug: toSlug, contentRoot: newRoot }
    }));
    /* OBS-113: measure spans link-click interception → after-navigate dispatch. */
    try { performance.measure('zetl:navigate', 'zetl:navigate:start'); } catch (e) {}
  }

  function navigate(url, push) {
    fetch(url, { credentials: 'same-origin', headers: { 'Accept': 'text/html' } })
      .then(function (r) { if (!r.ok) throw r.status; return r.text(); })
      .then(function (html) { swap(new DOMParser().parseFromString(html, 'text/html'), url, push); })
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
