---
id: BUG-GRAPH-STATIC-001
title: `zetl build`: graph widget shows no nodes on the rendered static site
status: open
reporter: Claude Opus 4.7 (1M context)
detection-method: real user, building a 383-page Lean4 wiki at /Users/anuna-02/Code/learn-lean
date: 2026-05-04
binary: zetl (release, install method unknown — `which zetl` → /Users/anuna-02/.local/bin/zetl)
vault: /Users/anuna-02/Code/learn-lean/wiki (383 pages, 2,668 wikilinks, 0 dead, 0 orphans aside from `_FORMAT.md`)
---

## Summary

After `zetl build`, opening `dist/_graph.html` (or any page's docked
mini-widget) shows a blank graph stage. The expected behaviour is
383 nodes + 2,149 edges (matching `dist/graph-index.json`). The
breakage has **two independent causes**, both visible under the
default-theme build but compounded under the `docs` theme.

## Repro

```bash
mkdir vault && cd vault
echo '# Hello\n[[bar]]' > index.md
echo '# Bar' > bar.md
zetl --no-cache build
python3 -m http.server 8000 --directory dist
# open http://localhost:8000/_graph.html → blank stage
```

## BUG-1 — `_graph.html` uses `../` paths despite living at the dist root

The full-page graph route is emitted at `dist/_graph.html` (root level).
Its inline config and script tags use relative paths assuming the file
is nested:

```html
window.__zetlGraphConfig = {
  root: "../",
  indexFile: "index.html",
  graphUrl: "../graph-index.json"
};
<script src="../_static/graph-boot.js" defer></script>
```

`graph-index.json` actually lives at `dist/graph-index.json` (next to
`_graph.html`), so `../graph-index.json` resolves to outside `dist/`,
returning 404 in any sane web server. Same for the static asset
references and the `<link rel=prefetch>` tags for `pages.json` and
`search-index.json`.

This affects the full-page graph route only — page-level mini-widgets
on nested pages (`dist/<slug>/index.html`) work because *those* pages
genuinely are one level deep, so `../graph-index.json` resolves
correctly.

### Patch

`_graph.html` is at the dist root, so it should reference siblings
with `./`. Either:

1. Emit the full-page graph at `dist/_graph/index.html` so the existing
   `../` paths resolve correctly (matches the existing convention for
   every other page); **or**
2. Detect that the graph partial is being rendered at the dist root and
   emit `./graph-index.json` / `./_static/graph-boot.js`.

Option (1) is the smaller fix and aligns with how every other page
already nests under its slug.

### Workaround the user applied

```bash
sed -i '' \
  -e 's|graphUrl: "../graph-index.json"|graphUrl: "./graph-index.json"|' \
  -e 's|root: "../"|root: "./"|' \
  -e 's|src="\.\.&#x2f;_static/|src="./_static/|g' \
  -e 's|href="\.\.&#x2f;_static/|href="./_static/|g' \
  -e 's|"\.\./|"./|g' \
  dist/_graph.html
```

After this, `curl http://localhost:8000/graph-index.json` returns 200
— but the graph still shows nothing. See BUG-2.

## BUG-2 — `docs` theme is missing `__zetlEnsureGraph` lazy vendor loader

`graph-boot.js` expects `window.graphology`, `window.Sigma`, and
`graphologyLibForceatlas2` as globals. The default theme's `base.html`
defines a lazy loader (`__zetlEnsureGraph`, lines 660-696 of
`themes/default/base.html`) that injects:

```
_static/vendor/sigma/graphology.min.js
_static/vendor/sigma/graphology-layout-forceatlas2.min.js
_static/vendor/sigma/sigma.min.js
```

…on first reveal. The `docs` theme's `base.html` (3 files exported via
`zetl theme export docs`) **does not contain this loader**. Result:
the graph widget partial mounts a `<div class="vg-stage">`, the
launcher button works, but the engine never instantiates because no
graph library is in scope.

### Repro

```bash
zetl theme export docs --dir wiki
# inspect: no __zetlEnsureGraph, no sigma/graphology references
grep -E 'graphology|sigma|EnsureGraph' wiki/.zetl/themes/docs/base.html
# (no output)

zetl build --theme docs
grep -E 'graphology|sigma' dist/_graph.html
# (no output — none of the libs are loaded)
```

### Patch

Either:

1. Move `__zetlEnsureGraph` out of `themes/default/base.html` into
   shared theme infrastructure (a `_partials/graph-vendor.html` that
   every theme `{% include %}`s), so non-default themes inherit it;
   **or**
2. Document that any theme overriding `base.html` must include the
   block, and add a `zetl theme lint` check that flags themes missing
   the loader; **or**
3. (Less invasive but ugly) inline the three `<script src>` tags
   unconditionally near `</body>` of `_graph.html` itself.

Option (1) is the right architectural fix: the graph widget is core
zetl functionality that should not silently break when a user picks
a non-default theme.

### Workaround applied

```html
<!-- appended to wiki/.zetl/themes/docs/base.html before </body> -->
<script>
  window.__zetlEnsureGraph = (function(){
    var promise = null;
    var ROOT = "{{ root_path | safe }}";
    var files = [
      '_static/vendor/sigma/graphology.min.js',
      '_static/vendor/sigma/graphology-layout-forceatlas2.min.js',
      '_static/vendor/sigma/sigma.min.js'
    ];
    function load(src){
      return new Promise(function(res, rej){
        if (document.querySelector('script[src="'+src+'"]')) { res(); return; }
        var s = document.createElement('script');
        s.src = src; s.async = false;
        s.onload = res; s.onerror = function(){ rej(new Error('failed: '+src)); };
        document.head.appendChild(s);
      });
    }
    return function(){
      if (promise) return promise;
      promise = files.reduce(function(p, f){ return p.then(function(){ return load(ROOT + f); }); }, Promise.resolve())
        .then(function(){ if (typeof window.__zetlBootGraph === 'function') window.__zetlBootGraph(); });
      return promise;
    };
  })();
  (function(){
    var fullStage = document.querySelector('.zetl-graph-widget[data-placement="fullscreen"]');
    if (fullStage && window.__zetlEnsureGraph) window.__zetlEnsureGraph();
  })();
</script>
```

## BUG-3 — graph-index.json breaks `graphology.import` for any slug colliding with JS prototype keys

After patching BUGs 1 and 2, the canvas mounted but stayed empty.
The actual cause: `graph-boot.js`'s call to `graph.import(data)`
throws on any node whose key matches a JavaScript prototype
property — `constructor`, `__proto__`, `toString`, `hasOwnProperty`,
`valueOf`, `isPrototypeOf`, `propertyIsEnumerable`,
`toLocaleString`. The user's wiki had a `constructor.md` page (a
legitimate slug for the "constructor of an inductive type" Lean
concept), and the import failed at the *first* edge whose source or
target was named `constructor`. The silent catch around `graph.import`
in `graph-boot.js` (lines 403-406) swallows the error and the widget
shows nothing.

### Repro

```bash
cd /tmp && mkdir vbug && cd vbug
echo '# A\n[[constructor]]'  > a.md
echo '# constructor'         > constructor.md
zetl --no-cache build
node -e '
const g = require("graphology");
const d = JSON.parse(require("fs").readFileSync("dist/graph-index.json","utf8"));
new g.DirectedGraph().import(d);
'
# → Graph.addDirectedEdgeWithKey: an edge linking "a" to "constructor" already exists.
```

The "already exists" message is misleading — graphology is simply
checking `nodeMap["constructor"]`, which evaluates to
`Object.prototype.constructor` (truthy) on a plain `{}` map.

### Patch

This is fundamentally a graphology bug (ships with `0.26.0` at the
time of writing — uses plain objects as node/edge maps). `zetl` can
fix it from its side in any of:

1. **Sanitize emitted node keys.** Before writing
   `graph-index.json`, prefix or escape any key matching the
   poison set. Reject the user's filename or auto-rename to
   `_constructor`. Trade-off: breaks wikilink stability.
2. **Use a graphology version / config that doesn't have this bug.**
   Graphology master added `Map`-based internals; pin the vendor
   bundle to that. Or set `multi: true` (which forces the `MultiMap`
   path that uses real `Map`s — confirmed-working workaround).
3. **Stop using `graph.import` and add nodes/edges manually with
   `addNode`/`addDirectedEdge`** in a wrapper that uses
   `Object.create(null)` for any intermediate maps. Most invasive.

Recommendation: option (2) — change the import options that ship in
`graph-index.json` from

```json
{"options":{"allowSelfLoops":true,"multi":false,"type":"directed"}}
```

to `multi: true`. Confirmed locally that this lets the import
succeed with the existing graphology 0.26.0 vendor bundle. Costs
zero functionality — the renderer doesn't display parallel edges
differently anyway, and zetl already dedupes per `(source, target)`
pair before emit, so no duplicate edges actually appear.

### Workaround applied

Renamed the offending page: `constructor.md` → `data-constructor.md`,
updated the nine wikilinks to it. After the rename + rebuild,
`graphology.import(data)` succeeded with 383 nodes and 2,149 edges,
and the widget rendered.

For a robust user experience, `zetl` should either prevent these
filenames at *write* time (with a clear error message, e.g. "filename
'constructor' collides with a JavaScript prototype property and
would break the graph widget — please rename") or handle them
transparently downstream via option (2) above. Current behaviour
silently breaks the marquee feature for any vault that happens to
have `constructor.md`, `__proto__.md`, `hasOwnProperty.md`, or
`toString.md` — none of which are unreasonable note titles.

The same poison set should also be checked in `zetl check`'s
diagnostics output as a warning, since the graph widget's silent
failure mode means the user has no way to diagnose this without
inspecting the console of a real browser.

## Severity

All three bugs together render the marquee feature (the interactive vault
graph) completely broken on a static build, with no error message to
guide the reader. New users will assume the feature itself is broken
or that their content has no links. Recommend fixing before the next
release; until then, add a banner-style fallback message inside
`.zetl-graph-fallback` that explains "graph engine failed to load —
check the docs/troubleshooting page".

## Bonus observation — mermaid not rendered

Unrelated but noticed during the same build: ```` ```mermaid ```` fenced
code blocks render as literal `<pre><code class="language-mermaid">`
on the static site under any theme. The default theme's
documentation could call this out, or zetl could ship a default
mermaid hook (or theme block). This is low priority — easy
workaround is a `<script type="module">` injecting `mermaid@10` at
the bottom of `base.html`. But the *graph* widget being a marquee
feature, the inverse expectation applied: I assumed it would just
work, and was confused by silent failure.
