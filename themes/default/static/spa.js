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
    try { performance.mark('zetl:navigate:start'); } catch (e) {}
    if (doc.title) document.title = doc.title;
    oldRoot.replaceWith(newRoot);
    runScripts(newRoot);
    if (push) history.pushState({ zetl: true, url: url }, '', url);
    try { performance.measure('zetl:navigate', 'zetl:navigate:start'); } catch (e) {}
    window.dispatchEvent(new CustomEvent('zetl:after-navigate', {
      detail: { slug: toSlug, contentRoot: newRoot }
    }));
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
    e.preventDefault();
    navigate(a.href, true);
  });

  window.addEventListener('popstate', function () { navigate(location.href, false); });
})();
