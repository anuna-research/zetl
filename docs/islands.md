# Component Islands & Inter-Island Messaging

Islands add **opt-in client-side interactivity** to otherwise-static component output: a
component ships a `<name>.js` that hydrates its `data-z` node in the browser, and islands
coordinate through a shell-provided retained message bus (`window.zetl`). This implements
[SPEC-050](../specs/SPEC-050-component-islands-and-messaging.md) on top of the
[component model](components.md).

Two trust tiers:

- **Trusted in-realm islands** (theme/component authors) run in the page realm with direct
  `window.zetl` access.
- **Content-author islands** (a `content_invocable` component that ships JS, see
  [content-directives.md](content-directives.md)) run in an **isolated Web Worker** and
  reach the bus only through a capability-scoped bridge — they can never touch the realm,
  the DOM, or `window.zetl` directly.

> ⚠️ **Not production-hardened.** SPEC-050 is a non-converged strawman; its own text
> requires a human security expert + executable fuzzing of the controlled-element renderer
> and bridge before shipping. The iframe escape-hatch transport is stubbed (the Worker
> transport is the v1 default). Gated behind the default-on `component-islands` cargo
> feature. Treat as a working preview.

## Quick start

```toml
# .zetl/components/poll/poll.toml
name = "poll"
requires = ["site"]
content_invocable = true        # makes it a content-author (sandboxed Worker) island
publishes = ["content:vote"]
render = "worker"               # default; "iframe" is the opt-in full-DOM escape hatch
paints = true                  # grants the Worker the render capability (Remote DOM)
hydrate = "visible"            # load (default) | idle | visible[(margin)] | media(query)
[island.topics."content:vote"]
type = "enum(\"yes\",\"no\")"   # CON-5005 topic value type
```

```js
// .zetl/components/poll/poll.js  — runs in a dedicated Worker (untrusted)
self.onmessage = function (e) {
  if (e.data.op === "boot") {
    self.postMessage({ op: "render", tree: {            // a controlled-element tree
      tag: "div", props: { class: "poll" }, children: [
        { tag: "button", props: { type: "button" }, children: ["Vote yes"] }
      ]
    }});
    self.postMessage({ op: "publish", topic: "content:vote", value: "yes" });
  }
};
```

`zetl build` emits `_static/zetl-islands.js` (the bus runtime, once), each island's
`_static/islands/<name>.js`, the per-page CSP, and stamps the page's `data-z="poll"` node
with the hydration markers. Pages with no island load **nothing** extra.

## The bus (`window.zetl`)

```js
window.zetl.store(topic)  // → { get(), set(value), subscribe(fn) → unsubscribe }
window.zetl.bus           // → { emit(topic, detail), on(topic, fn) → off }
```

- **Retained, replay-on-subscribe** (REQ-5005): a fresh subscriber fires immediately with
  the current value, then on every change; an unchanged `set()` coalesces.
- **Typed** (REQ-5013/CON-5005): every payload is recognised against the topic's declared
  type before storage/replay/delivery; non-conforming values are refused.
- **Single instance, survives SPA navigation** (REQ-5007); deep-frozen before any island
  runs (REQ-5019); bounded (≤256 topics, ≤1024 subscribers, ≤64 KiB/value, NFR-5002).
- **Persisted topics** (REQ-5006): a topic may declare `persisted=true` + a `default`; a
  render-blocking pre-paint script applies the stored value (recognised, else default)
  before first paint and is admitted by a `sha256` CSP hash.

## Topics (CON-5001)

`content:`-prefixed topics belong to content authors; any other first segment is a
**trusted** topic. A content island may publish only `content:` topics and may not publish
a free-string-typed topic (REQ-5022); it may read a trusted topic only if the theme grants
it via `[[theme.island-grants]]`.

## Content-island isolation (the security model)

- **Worker realm** (REQ-5025): no DOM, no `window`, no `window.zetl`. The Worker emits a
  declarative element tree; the **trusted host paints it** (Remote DOM / worker-dom model).
- **Capability bridge** (REQ-5016 / CON-5006): the parent is the sole reference monitor.
  Every inbound Worker message is checked: known handle ∧ `(topic, direction)` granted ∧
  payload type-recognised (with prototype-stripping). Failures answer `denied` and never
  reach the bus.
- **Controlled-element renderer** (CON-5007): a closed default-deny tag/attribute allowlist
  (excludes `script`/`style`/`iframe`/`form`/etc.), URL-scheme validation, `aria-*`/`role`
  allowed for a11y, `setAttribute`/`textContent` only (never `innerHTML`), depth/breadth/
  node/byte bounds, cycle rejection. This is what makes "untrusted code cannot inject
  markup" hold — verified in a real browser playtest.
- **Network egress** (REQ-5026/5027): confined by the page **Content-Security-Policy**, not
  per-worker policy. Operators widen it in `[security.csp]`; content authors may only
  *request* widenings via `[island.requests]` (inert until approved — surfaced in the
  wiring audit). The per-page baseline for content-island pages is:

  ```
  default-src 'none'; script-src 'self' 'sha256-…';   # inline scripts admitted by hash
  worker-src 'self' blob:; connect-src 'self';          # cross-origin egress blocked
  img-src 'self' data:; font-src 'self' data:;
  style-src 'self' 'unsafe-inline'; base-uri 'none'; form-action 'none'
  ```

  `script-src` is strict (per-inline-`<script>` `sha256`, never `unsafe-inline`) — that's
  the load-bearing control. `connect-src 'self'`, `img-src 'self' data:` and
  `style-src 'unsafe-inline'` are deliberate, theme-compatible relaxations that are safe
  for the islands threat model: the worker emits no script, the CON-5007 renderer forbids
  the `style` attribute and validates URL schemes, and cross-origin egress stays blocked.
  An operator with a CSP-clean theme can tighten them. The `<meta>` CSP is the first
  `<head>` child; `_headers.csp` carries only `frame-ancestors` (which `<meta>` can't set).

## Hydration strategies (REQ-5024)

`load` (immediate, default), `idle` (`requestIdleCallback`), `visible[(rootMargin)]`
(`IntersectionObserver` — defers Worker creation until scrolled into view), `media(query)`
(`matchMedia`). Hydration is idempotent and re-runs on SPA subtree swaps.

## Build artefacts

| Path | What |
|------|------|
| `_static/zetl-islands.js` | the bus runtime (once, if any island) |
| `_static/islands/<name>.js` | per-component island/worker script (deduped, deterministic) |
| `_headers.csp` | served-deploy CSP header artifact (content-island builds) |
| `_static/island-audit.json` | wiring graph: edges, findings, requests (REQ-5009) |
| page `<head>` | CSP `<meta>` (content-island pages) + pre-paint + runtime bootstrap |

## Backward compatibility (REQ-5012)

No island and no persisted topic → byte-identical to a build without this feature: no
`window.zetl`, no bus, no island scripts, no pre-paint, no CSP meta.
