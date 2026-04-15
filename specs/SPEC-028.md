---
title: "SPEC-028: Interactive Graph View — Sigma.js + graphology Component for Serve and Build"
version: 0.1.0
status: draft
date: 2026-04-15
audience: agent, human
parent: SPEC-027
related:
  - SPEC-027  # History UI (per-page and per-vault surfaces)
  - SPEC-017  # zetl history — invisible temporal graph
  - SPEC-004  # Web UI and static export (if applicable)
---

# SPEC-028: Interactive Graph View — Sigma.js + graphology Component for Serve and Build

## Information Table

| Field        | Value                                                                        |
| ------------ | ---------------------------------------------------------------------------- |
| Document ID  | SPEC-028                                                                     |
| Title        | Interactive Graph View — Sigma.js + graphology Component for Serve and Build |
| Version      | 0.1.0                                                                        |
| Status       | Draft                                                                        |
| Author       | Agent (USDD Protocol v1.3.0)                                                 |
| Date         | 2026-04-15                                                                   |
| Audience     | Agent, Human                                                                 |
| Trace        | USDD Agent Protocol v1.3.0                                                   |
| Parent       | SPEC-027: History UI                                                         |
| Related      | SPEC-017 (history), web UI and static export specs                           |
| Dependencies | Minijinja theme engine; existing `/api/graph` route; `build_search_index` pattern |

---

## 1. Overview

`zetl` currently exposes a full link graph via the JSON API (`GET /api/graph`) and writes a static `search_index.json` + embeds a per-page `search_index` string into every rendered template as `{{ search_index | safe }}`. This pattern — a pre-serialized JSON asset, available both as a static file and as a template variable — has proven robust for search.

There is, however, no visual graph view. Obsidian's built-in force-directed graph is one of the features that made wiki-style link visualisation mainstream; zetl users ask for something equivalent. The building blocks are in place (the graph is already computed and JSON-serialisable), but no client-side renderer is wired up and no template surface exposes it.

This spec introduces an **interactive graph view** as a themeable, optional UI component, served dynamically under `zetl serve` and emitted statically under `zetl build`, rendered client-side by **Sigma.js v3** on WebGL with the **graphology** data model and the **ForceAtlas2** layout (all MIT). The rendering is entirely in the theme layer — the Rust binary only provides the data, mirroring the `search_index` / `history-index.json` contract.

### 1.1 Motivation

- **Discoverability of structure.** A vault's link topology is its most zetl-specific affordance. A graph view turns the backlinks-as-list into backlinks-as-shape.
- **Feature parity with Obsidian.** Obsidian's graph view is closed source, but users expect an equivalent when they import an Obsidian vault into zetl. Sigma.js with the ForceAtlas2 layout visually approximates the Obsidian graph panel, while scaling to much larger vaults via WebGL.
- **Reuses proven pipeline.** The existing `search_index` and `history_index` template variables and static-file emission pattern (see `src/web/engine.rs`, `src/web/build.rs`) extend naturally to a `graph_index`. No new data plumbing is required — only serialisation and a template.
- **Static-first.** `zetl build` users currently lose all interactive surfaces. A client-rendered graph preserves parity: a static HTML page + static JSON asset + a JS bundle, no server required.
- **Themeable.** Themes can opt in, restyle, or replace the graph view without patching the binary. The component is a self-contained partial template.

### 1.2 Design Principles

1. **Data already exists — serialise, don't recompute.** The in-memory link graph (outlinks, backlinks, page metadata) is the sole source of truth. The renderer consumes a single JSON artefact.
2. **Serve and build reach parity.** Every graph surface that renders under `serve` MUST also render under `build`. Both modes consume the same `graph_index` JSON.
3. **Payload is externalised by default.** The graph JSON is written to a static file (`graph-index.json`) and fetched client-side, not inlined into every page. Inlining is available as a template opt-in for small vaults or offline use.
4. **Themeable.** The graph component is a Minijinja partial (`_graph.html`) overridable under `.zetl/themes/<theme>/`. Default styling lives alongside the component and respects existing CSS tokens.
5. **Graceful absence.** When JavaScript is disabled, the graph container degrades to a `<noscript>` message with a link to the page list. When the vault has zero links, the component renders an empty-state message.
6. **Vendor isolation.** Sigma.js, graphology, and `graphology-layout-forceatlas2` are bundled as static assets under `_static/vendor/sigma/`; no CDN dependency at runtime.
7. **No new data fields.** If a proposed graph feature cannot be rendered from existing graph data, it is out of scope for this spec. New fields (e.g., community detection) are deferred to a successor.

### 1.3 Scope

**In scope:**

- Serialisation of the link graph to `graph-index.json` (build mode) and exposure via `/graph-index.json` (serve mode, in addition to the existing `/api/graph`).
- A `graph_index_url` template variable (string) available in all templates, pointing at the externalised JSON asset.
- An optional `graph_index` template variable (string, `""` by default), empty unless the theme opts in via `graph_inline = true` in `theme.toml`.
- A bundled partial template `themes/default/_graph.html` rendering a Sigma.js instance (on a graphology graph) into a configurable container.
- A dedicated full-page graph view at `/_graph` (serve) and `_graph.html` (build), mirroring the `/_history` pattern (SPEC-027).
- Per-page "local graph" mode: the same component restricted to the current page plus its N-hop neighbourhood, rendered by passing `{page_slug, depth}` to the partial.
- Sidebar link from the default theme to `/_graph`.
- Sigma.js v3 + graphology + `graphology-layout-forceatlas2` bundled under `themes/default/static/vendor/sigma/` (pinned versions, MIT-licensed).
- Graceful absence handling (no JS, no links, disabled feature flag).
- Documentation: `README.md` "Graph view" section; CHANGELOG entry; theme authoring reference updated with `graph_index_url`, `graph_index`, and the `_graph.html` partial.

**Out of scope:**

- Force-directed physics written from scratch (`graphology-layout-forceatlas2` handles this).
- Server-side graph image rendering (SVG/PNG export) — may become a successor spec.
- 3D graph rendering.
- Graph editing (drag-to-link, node deletion) — read-only in this spec.
- Temporal overlays on the graph (link trends colouring edges by recency) — interesting, deferred to a successor that composes SPEC-027 history data with SPEC-028 visuals.
- Semantic/embedding-based node clustering — requires `--features semantic`, out of scope here.
- Mobile gesture polish beyond Sigma.js defaults.
- Theming of non-default bundled themes (`docs`, `fountain`, `minimal`). Those may opt in per-theme; this spec only modifies `default/`.

---

## 2. User Profiles and Happy Paths

### 2.1 User: Vault author browsing locally

**Role:** Individual knowledge-worker running `zetl serve`.
**Goal:** Visually explore the shape of their vault; discover clusters and orphans.
**Constraints:** Desktop browser; keyboard + mouse; expects Obsidian-like interaction.

**Happy path:**

1. `zetl serve` → navigate to `http://localhost:3000` → see "Graph" in sidebar → click → `/_graph` renders within 1s for vaults up to 2,000 pages.
2. Pan with drag, zoom with scroll, click a node → navigate to that page.
3. On any page, see a small "local graph" panel showing the current page and its 1-hop neighbourhood.

### 2.2 User: Static-site publisher

**Role:** Runs `zetl build`, deploys to GitHub Pages / Netlify / S3.
**Goal:** Give public readers the same graph view as the serve UI.
**Constraints:** No server; CDN-only hosting; assets must be relative-path-safe.

**Happy path:**

1. `zetl build --out-dir dist` → `dist/_graph.html`, `dist/graph-index.json`, and `dist/_static/vendor/sigma/*.js` are written.
2. Upload `dist/` to any static host → the deployed `_graph.html` renders identically to serve mode.
3. Per-page local-graph panels work via `fetch('/graph-index.json')` relative-URL resolution.

### 2.3 User: Theme author

**Role:** Customises `.zetl/themes/<theme>/`.
**Goal:** Restyle or disable the graph without patching `zetl`.
**Constraints:** Only overrides templates and static assets; no Rust.

**Happy path:**

1. Copy `_graph.html` from the bundled `default/` into their theme directory.
2. Override visual styling by setting CSS custom properties (see §10 / REQ-114) and, for structural changes, override the Sigma node/edge reducers inside a custom `_graph.html`.
3. Optionally set `graph_inline = true` in `theme.toml` to embed the JSON for offline distribution; or omit the partial entirely to disable.

---

## 3. Functional Requirements

### REQ-101: Graph Index Serialisation

The system SHALL serialise the vault's complete link graph to a single JSON document conforming to the graphology serialisation schema (an object with top-level `attributes`, `options`, `nodes: [{key, attributes}]`, and `edges: [{key, source, target, attributes}]`) WITHIN the build/serve pipeline FOR every theme render WITH stable ordering (alphabetical by slug) to produce deterministic diffs. The document SHALL be directly loadable via `graph.import(json)` on a fresh graphology `DirectedGraph` WITHOUT transformation.

Trace:

- TEST-101
- CON-101
- OBS-101

### REQ-102: Static `graph-index.json` Asset

Under `zetl build`, the system SHALL write `<out-dir>/graph-index.json` containing the serialised graph (REQ-101) exactly once per build AS part of the normal asset-write pass, co-located with `search_index.json` and `history-index.json`.

Trace:

- TEST-102
- CON-101

### REQ-103: Serve Route for Graph Index

Under `zetl serve`, the system SHALL expose the serialised graph at `GET /graph-index.json` RETURNING a JSON body matching REQ-101 WITH `Content-Type: application/json` AND a `Cache-Control: no-cache` header (graph updates on reindex).

Trace:

- TEST-103
- CON-102

### REQ-104: Template Variable `graph_index_url`

The system SHALL expose a string template variable `graph_index_url` in all Minijinja renders WITH value `"/graph-index.json"` in serve mode AND `"graph-index.json"` (relative) in build mode, such that themes may fetch the graph without hard-coding paths.

Trace:

- TEST-104

### REQ-105: Optional Inline Template Variable `graph_index`

The system SHALL expose a string template variable `graph_index` in all Minijinja renders, containing the serialised JSON document (REQ-101) AS a string WHEN the active theme's `theme.toml` sets `graph_inline = true`, OTHERWISE `""` (empty string). Empty-string semantics allow `{% if graph_index %}…{% else %}fetch…{% endif %}` patterns.

Trace:

- TEST-105

### REQ-106: Default Graph Partial

The default theme SHALL ship a partial template at `themes/default/_graph.html` that, given a container selector and optional `{page_slug, depth}` parameters, instantiates a graphology `DirectedGraph` from either the inline `graph_index` or the fetched `graph_index_url`, runs the `graphology-layout-forceatlas2` layout (in a Web Worker where available), AND mounts a Sigma.js v3 renderer bound to the graph as the default arrangement.

Trace:

- TEST-106

### REQ-107: Vault-Wide Graph Page

The system SHALL render a dedicated vault-wide graph surface at `/_graph` (serve) AND `_graph.html` (build root) USING the `themes/<theme>/_graph.html` partial embedded in a full-page layout (`vault_graph.html`) extending `base.html`, WITH a sidebar link labelled "Graph" in the default theme's sidebar.

Trace:

- TEST-107

### REQ-108: Persistent Graph Widget (Single Instance, Mode-Switched)

The default theme SHALL mount exactly ONE Sigma.js instance inside the persistent shell (outside the volatile content region, per REQ-113) AND SHALL NOT mount a second graph inside `page.html` or any other volatile element. The widget SHALL re-render in a new *mode* on `zetl:after-navigate` without re-instantiating Sigma or re-running layout.

Widget modes:

- **`local`** — default on page routes (`/<slug>`). Camera auto-zooms to the current page's N-hop neighbourhood (default N = 1); non-neighbour nodes fade via a reducer, not via layout recomputation.
- **`vault`** — default on the `/_graph` route and the vault index (`/`). Camera zoom-to-fit the full graph.
- **`off`** — widget hidden (DOM node remains mounted; `display: none` on the container). Layout and camera state preserved for re-activation.

Mode switching SHALL be instantaneous (zero layout work, a reducer refresh only) AND SHALL persist across navigation via `sessionStorage`.

Trace:

- TEST-108

### REQ-116: Default Widget Placement — Docked Mini-Map

The default theme SHALL render the persistent graph widget AS a fixed-position docked mini-map in the bottom-right of the viewport, 280 × 200 px by default, WITH a resize handle (CSS `resize: both`) AND a click-to-expand control that navigates to `/_graph`. The widget SHALL NOT overlap the transclusion panel (`.transclusion-panel`) NOR the existing sidebar in the default desktop layout, AND SHALL be positioned via CSS custom properties (`--zetl-graph-widget-width`, `--zetl-graph-widget-height`, `--zetl-graph-widget-bottom`, `--zetl-graph-widget-right`) SO THAT theme authors may restyle without rewriting the partial.

The default theme SHALL additionally document two opt-in alternative placements in `theme.toml` under `[graph.placement]`:

- `tabs` — widget and transclusion cards share a tabbed right-rail container.
- `stacked` — widget sits above the transclusion cards in the right rail.

Switching placement SHALL be a `theme.toml` flag + CSS-var override; no `_graph.html` rewrite required.

Trace:

- TEST-116

### REQ-117: Mobile Behaviour

On viewports narrower than a theme-configurable breakpoint (default 900 px), the widget SHALL be hidden by default AND reachable via a top-bar toggle button that expands the widget to a full-screen overlay. The toggle SHALL be keyboard-accessible (focus ring, Enter/Space to activate, Escape to dismiss) AND SHALL preserve the persistent Sigma instance (toggle manipulates visibility only).

When the viewport is resized across the breakpoint, the widget SHALL transition between docked mini-map and hidden-with-toggle without re-instantiating Sigma.

Trace:

- TEST-117

### REQ-109: Graceful Absence

WHEN JavaScript is disabled, the graph container SHALL render a `<noscript>` message with a link to `/` (the page list). WHEN the vault has zero pages or zero links, the graph surface SHALL render an empty-state message ("No links yet — create `[[wikilinks]]` between pages to build a graph"). WHEN the theme omits `_graph.html`, the feature SHALL degrade silently (no console errors, no broken links from `base.html`).

Trace:

- TEST-109

### REQ-110: Vendor Asset Bundling

The default theme SHALL include Sigma.js v3, graphology, AND `graphology-layout-forceatlas2` AS static files under `themes/default/static/vendor/sigma/` WITH pinned versions recorded in `theme.toml` (fields `vendor.sigma.version`, `vendor.graphology.version`, `vendor.graphology-layout-forceatlas2.version`) AND MUST NOT require a CDN or network fetch at runtime.

Trace:

- TEST-110

### REQ-111: Node Click Navigation

Clicking a node in any rendered graph SHALL navigate the browser to that page's slug URL (`/<slug>` in serve, `pages/<slug>/index.html` or equivalent in build) WITHOUT a full page reload where the theme supports client-side navigation, falling back to a standard anchor navigation otherwise.

Trace:

- TEST-111

### REQ-112: Dead-Link Visual Distinction

Edges targeting dead links (pages that do not exist in the vault) SHALL render with the same visual treatment as dead links in the existing page UI (e.g., muted colour, dashed stroke), USING the dead-link flag already present on outlinks.

Trace:

- TEST-112

### REQ-113: Persistent Shell and SPA Navigation

WHEN the active theme's `theme.toml` sets `spa.enabled = true`, the system SHALL inject a same-origin navigation shell that intercepts `<a>` clicks within the rendered document AND swaps only the designated volatile region (the element carrying `data-zetl-volatile` or the `<main>` fallback) SO THAT the graph component's DOM element and Sigma instance are never unmounted between navigations, WITH the result that camera position, computed layout coordinates, and selection state persist WITHOUT a visual flash.

When `spa.enabled` is unset or `false`, the system SHALL NOT inject the navigation shell, AND the graph SHALL re-initialise per page (documented degradation). Browser back/forward, meta-click / Ctrl-click / middle-click (open-in-new-tab), and cross-origin links SHALL continue to behave as native browser navigation in both cases.

Trace:

- TEST-113
- OBS-113

### REQ-114: Theme CSS Custom-Property Contract

The default theme SHALL document and consume a stable, versioned set of CSS custom properties (`--zetl-graph-*` for colours and typography, `--zetl-shell-*` for layout regions) AS its sole mechanism for graph visual styling, SO THAT custom themes may restyle the graph and shell by overriding those properties WITHOUT touching JavaScript or recompiling the binary. The Sigma reducers in `_graph.html` SHALL read these properties at render time via `getComputedStyle` and SHALL refresh on `data-theme` / `class` mutation or `prefers-color-scheme` change.

The property contract SHALL be versioned (breaking changes bump `theme.contract.version` in the theme authoring reference) and backwards-compatible within a major zetl release.

Trace:

- TEST-114

### REQ-115: Navigation Lifecycle Events

The SPA navigation shell SHALL dispatch two `window`-level events around each successful navigation:

- `zetl:before-navigate` — dispatched after the content fetch completes but before the DOM swap, with `detail = { fromSlug, toSlug, url }`. Cancelable via `preventDefault()`; when cancelled, the shell falls back to native navigation.
- `zetl:after-navigate` — dispatched immediately after the DOM swap, with `detail = { slug, contentRoot }`, where `contentRoot` is the newly-mounted volatile element.

Themes and graph reducers SHALL use `zetl:after-navigate` to re-run enhancements on swapped content (Mermaid, KaTeX, custom widgets) AND to update graph highlight state WITHOUT re-initialising the Sigma instance.

Trace:

- TEST-115

---

## 4. Non-Functional Requirements

### NFR-101: Initial Render Latency (Vault-Wide Graph)

First meaningful paint of `/_graph` SHALL be ≤ 1500 ms UNDER a vault of ≤ 2,000 pages and ≤ 10,000 edges ON a mid-range laptop (2020-era Intel, Chrome) WITH the 95th percentile measured across 10 cold loads.

Trace:

- TEST-201
- OBS-201

### NFR-102: Interaction Frame Rate

Pan and zoom interactions SHALL sustain ≥ 30 fps UNDER a vault of ≤ 2,000 pages MEASURED via `performance.now()` over a 2-second drag gesture.

Trace:

- TEST-202

### NFR-103: Static Asset Size Budget

The total bundled size of Sigma.js v3 + graphology + `graphology-layout-forceatlas2` (minified, gzipped) SHALL be ≤ 250 kB UNDER default bundling; vault builds SHALL NOT emit the vendor bundle more than once per build output.

Trace:

- TEST-203

### NFR-104: Graph JSON Size

`graph-index.json` SHALL be ≤ 1 MB UNCOMPRESSED FOR vaults up to 2,000 pages / 10,000 edges. Above this size, the build SHALL emit a warning to stderr recommending a server-mode deployment or link-graph filtering.

Trace:

- TEST-204

### NFR-105: Accessibility Baseline

The graph view SHALL provide a keyboard-navigable fallback: a `<details>` element containing the full page list grouped by cluster, rendered alongside the canvas, SO THAT users relying on keyboard or screen readers can reach every node WITHOUT relying on pointer interaction. WCAG 2.2 AA contrast applies to node and edge default colours.

Trace:

- TEST-205

---

## 5. Contracts

### CON-101: Graph Index JSON Schema

```jsonc
// graph-index.json — graphology serialisation format, directly loadable via graph.import()
{
  "options": { "type": "directed", "multi": false, "allowSelfLoops": true },
  "attributes": {
    "format": "zetl-graph/v1",
    "generated_at": "2026-04-15T12:00:00Z",
    "vault": { "name": "my-vault", "pages": 123, "links": 456 }
  },
  "nodes": [
    {
      "key": "some-page",              // stable slug — graphology node id
      "attributes": {
        "label": "Some Page",          // display title
        "slug": "some-page",
        "outlink_count": 5,
        "backlink_count": 3,
        "is_orphan": false,
        "is_dead": false,              // true for referenced-but-missing pages
        "tags": ["rust", "cli"]        // from frontmatter; [] if none
      }
    }
  ],
  "edges": [
    {
      "key": "some-page->another",
      "source": "some-page",
      "target": "another",
      "attributes": {}
    }
  ]
}
```

Implements: REQ-101, REQ-102, REQ-103.
Verified by: TEST-101, TEST-102, TEST-103.

### CON-102: Serve Route `/graph-index.json`

- Method: `GET`
- Pre-conditions: server running; index built.
- Post-conditions: response body conforms to CON-101; `Content-Type: application/json; charset=utf-8`; `Cache-Control: no-cache`.
- Error model: returns `500` with `{error, message}` JSON on serialisation failure, matching existing API error envelope.

Implements: REQ-103.
Verified by: TEST-103.

---

## 6. Architecture Decisions

### ADR-101: Client-Side Rendering via Sigma.js + graphology

**Context:** A graph view is needed for both serve and build. Four options were considered:

1. Server-rendered static SVG (GraphViz / Rust-side layout).
2. Client-rendered via Cytoscape.js (MIT, canvas renderer with a new WebGL renderer; CSS-like stylesheet API).
3. Client-rendered via Sigma.js v3 + graphology + ForceAtlas2 (all MIT, WebGL2-first, programmatic reducers).
4. Client-rendered via Cosmograph / Cosmos (MIT core, GPU force simulation, scales to 100k+ nodes).

**Decision:** Option 3 — Sigma.js v3 + graphology + `graphology-layout-forceatlas2`.

**Rationale:**

- **Scale headroom.** WebGL2 rendering comfortably handles the 10k+ node vaults zetl expects once users import large Obsidian libraries and once temporal / semantic overlays compose on the same canvas. Cytoscape's default canvas renderer tops out around 2–5k.
- **Clean data model.** graphology is a standalone, typed graph library. The same `DirectedGraph` instance feeds the renderer, the layout, and any analytics plugins (centrality, clustering) without re-serialisation. This directly enables future specs that compose history (SPEC-027) and reasoning into the graph view.
- **Bundle size.** Sigma + graphology + FA2 is ≤ 250 kB gzipped — smaller than Cytoscape + fcose in the WebGL-enabled configuration.
- **Theming integration.** Sigma's reducer pattern (`nodeReducer`, `edgeReducer`) is called every frame; combined with the CSS custom-property contract (REQ-114) reducers read computed styles via `getComputedStyle`, so theme authors still author in CSS. The loss of Cytoscape's CSS-like stylesheet DSL is compensated by a documented, versioned CSS-var API.
- **Server-rendered SVG** (option 1) is cheap but static — no hover, no local-graph exploration, no zoom-to-fit. It fails the discovery use case.
- **Cosmograph** (option 4) scales further but has a smaller ecosystem and less mature keyboard/accessibility story. It remains a documented future migration target if zetl regularly encounters 100k+ node vaults.

**Trade-offs:**

- No CSS-like stylesheet. Structural styling (node shapes, edge thickness curves, dashed patterns) requires editing reducer code in `_graph.html`. The CSS-var contract (REQ-114) covers the common case of colours, sizes, and typography.
- Labels are rendered to canvas by Sigma; sub-pixel rendering, unusual scripts, and emoji fidelity are weaker than DOM text. Accepted — label clarity at typical graph zoom levels is sufficient.
- Canvas is opaque to screen readers; NFR-105 keyboard/SR fallback (a `<details>`-grouped page list) is therefore REQUIRED, not optional.
- Users with JS disabled see the `<noscript>` fallback only. Accepted — aligns with the existing editor and search UI, both of which require JS.
- Compound/parent nodes (visually grouping by folder) are not native in Sigma; emulated via FA2 sub-clustering + colour if needed. Deferred.

### ADR-102: External JSON Asset vs Inline

**Context:** Following the `search_index` pattern, the graph could be inlined into every page as a `{{ graph_index | safe }}` string, or externalised to a static file fetched by the client.

**Decision:** Externalise by default, inline opt-in.

**Rationale:**

- A 500 kB graph inlined into every page of a 1,000-page build multiplies the deployed artefact size by 1,000.
- `graph-index.json` mirrors the existing `history-index.json` export — the pattern is already in the codebase (SPEC-027 OBS-013).
- Inline mode (`graph_inline = true`) is retained for small vaults, single-file exports, and offline distribution.

**Trade-off:** A client-side `fetch` adds one HTTP round-trip. Acceptable — the existing `_static/` assets already require multiple round-trips.

### ADR-103: ForceAtlas2 as Default Layout

**Context:** The graphology ecosystem ships multiple layouts: `graphology-layout` (random, circular), `graphology-layout-forceatlas2` (FA2), `graphology-layout-force` (Fruchterman-Reingold), and `graphology-layout-noverlap` (post-processing).

**Decision:** `graphology-layout-forceatlas2` (FA2), optionally post-processed with `graphology-layout-noverlap` for label legibility.

**Rationale:**

- Produces the Obsidian-like force-directed arrangement users expect.
- Deterministic given a fixed seed and iteration count — matters for reproducible static builds.
- Supports incremental iteration (run N steps, render, continue) so the layout can animate into place on first paint.
- Web Worker implementation (`graphology-layout-forceatlas2/worker`) runs off the main thread; frees the renderer to stay at 60 fps during layout.
- Active maintenance, MIT licence, written by the same team as graphology and Sigma.

**Trade-off:** FA2 adds ~40 kB gzipped over the bare renderer. Within NFR-103 budget. The Fruchterman-Reingold alternative is smaller but less visually Obsidian-like; FA2 is the deliberate choice.

Layout coordinates are persisted across SPA navigation via the persistent Sigma instance (REQ-113); layout is therefore run once per mount, not once per page load.

---

## 7. Observability

### OBS-101: Graph Serialisation Timing

`build_graph_index(vault_root)` SHALL emit a `[zetl] graph-export: pages=N edges=M duration_ms=X bytes=Y` line under `--verbose`, mirroring the existing `history-export:` instrumentation (SPEC-027 OBS-013).

Trace:

- REQ-101

### OBS-102: Stats Integration

`zetl stats` SHALL include the graph-index size (bytes, nodes, edges) under a `Graph:` section in table output AND as a `graph` field in JSON output, mirroring the existing `History:` section.

Trace:

- REQ-101

### OBS-113: SPA Navigation Timing

The SPA navigation shell SHALL emit a `performance.measure('zetl:navigate', ...)` spanning from link-click interception to `zetl:after-navigate` dispatch, allowing users and CI to assert on navigation-to-paint time via the browser Performance panel.

Trace:

- REQ-113

### OBS-201: Client Render Timing

The default `_graph.html` partial SHALL log `performance.mark('zetl:graph:render:start')` before layout and `performance.measure('zetl:graph:render', 'zetl:graph:render:start')` on `layoutstop`, allowing users to measure NFR-101 locally via the browser devtools Performance panel.

Trace:

- NFR-101

---

## 8. Test Specifications

### TEST-101: Graph Serialisation Roundtrip (Property-Based)

For all valid `VaultContext` inputs, `serialize_graph_index(vault_ctx)` produces valid JSON that, when parsed and passed to `graph.import(json)` on a fresh graphology `DirectedGraph` (via headless Chrome), yields a graph with node count equal to `vault_ctx.pages.len()` and edge count equal to the sum of `outlink_count` across all pages minus dead links.

Technique: property-based (pure core) + one integration fixture.

### TEST-102: Build-Mode Asset Emission

`zetl build --out-dir /tmp/out` on the demo vault produces `/tmp/out/graph-index.json` matching CON-101, and `/tmp/out/_graph.html` that references it via relative URL.

### TEST-103: Serve-Mode Route Contract

`GET /graph-index.json` against a serve-mode fixture returns `200`, `Content-Type: application/json`, and a body matching CON-101.

### TEST-104/105: Template Variable Presence

Minijinja render of a trivial template `{{ graph_index_url }}|{{ graph_index | length }}` produces the expected strings in serve and build modes, with and without `graph_inline = true`.

### TEST-106/107/108: Default Theme Integration

Snapshot tests: rendered `_graph.html` contains a `<div id="zetl-graph">`, a `<script>` loading Sigma, graphology, and `graphology-layout-forceatlas2` from `_static/vendor/sigma/`, and either an inline JSON blob or a `fetch(graph_index_url)` call.

### TEST-109: Graceful Absence Matrix

- JS disabled → `<noscript>` renders.
- Empty vault → empty-state copy renders.
- `_graph.html` partial deleted from override theme → sidebar link and `/_graph` route still render without JS error (route returns 404 or empty-state at spec's choice; see ambiguity resolution below).

### TEST-110: Vendor Version Pin

`theme.toml` includes non-empty `vendor.sigma.version`, `vendor.graphology.version`, and `vendor.graphology-layout-forceatlas2.version` strings; the files present under `static/vendor/sigma/` match those versions (verified via filename or a committed checksum).

### TEST-111: Click Navigation

Headless-browser test: clicking a node in the rendered graph navigates the document to that slug's URL.

### TEST-112: Dead-Link Visual Distinction

Graph JSON for a vault with a known dead link includes `"is_dead": true` on the corresponding node; snapshot of the rendered graph shows the edge rendered with the dead-link class.

### TEST-108: Single-Instance Mode Switching

Fixture loads three routes sequentially (`/page-a`, `/_graph`, `/page-b`) via the SPA shell. Assert:

- The Sigma `WebGLRenderingContext` identity is unchanged across all navigations (same instance).
- The widget's `data-mode` attribute transitions `local → vault → local` and the camera state matches each mode's expected framing.
- FA2 layout runs once (on initial mount); `layoutstop` fires exactly once across all navigations.
- Mode is persisted in `sessionStorage` under `zetl:graph:mode` and restored on reload.

### TEST-116: Default Docked Mini-Map Placement

Snapshot / geometric test on the default theme: widget container computed `position` is `fixed`, `bottom` and `right` match the CSS-var values, and `width`/`height` default to 280/200 px. Assert no overlap with `.transclusion-panel` or the sidebar via `getBoundingClientRect` intersection on a seeded page with multiple forward-links.

Toggling `[graph.placement] = "tabs"` in a fixture theme's `theme.toml` causes the widget to render inside the tabbed right-rail container; `[graph.placement] = "stacked"` causes it to render above `.transclusion-panel`. Both variants pass without editing `_graph.html`.

### TEST-117: Mobile Toggle

Puppeteer resizes viewport below the 900 px breakpoint; widget is `display: none` by default, toggle button is focusable and expands the widget to a full-screen overlay. Escape key dismisses. WebGL context identity is unchanged before, during, and after the toggle.

### TEST-113: No-Flash SPA Navigation

Headless-browser test with the default theme (`spa.enabled = true`):

1. Load `/_graph`; record the Sigma canvas's `webglContext` identity and current camera state.
2. Click a node; await `zetl:after-navigate`.
3. Assert: canvas's `webglContext` identity is unchanged (same GL context, same instance), camera state matches pre-navigation, URL updated, new page content is mounted in `data-zetl-volatile`, document title updated.
4. Browser back/forward returns the previous content without flash.
5. Meta-click on a link opens a new tab (native behaviour retained).
6. With `spa.enabled = false` in a sibling fixture, assert navigation falls back to full page reload (Sigma instance changes identity).

### TEST-114: CSS Custom-Property Theming

Snapshot / computed-style test: override each `--zetl-graph-*` and `--zetl-shell-*` property in a fixture theme; assert the rendered graph reflects the override (node colour sampled from the canvas via `readPixels` at a known node position; shell grid tracks measured via `getBoundingClientRect`).

### TEST-115: Navigation Lifecycle Events

Fixture page registers listeners for `zetl:before-navigate` and `zetl:after-navigate`. Test asserts:

- Both events fire on every SPA navigation, in order.
- `zetl:before-navigate` is cancelable; calling `preventDefault()` falls back to native navigation.
- `zetl:after-navigate.detail.contentRoot` is the newly-mounted volatile element and contains the expected page content.

### TEST-201–205: NFRs

- TEST-201: Puppeteer run measuring `LargestContentfulPaint` on a seeded 2,000-page synthetic vault; assert ≤ 1500 ms (P95 across 10 runs).
- TEST-202: Puppeteer scripted drag gesture; count `requestAnimationFrame` ticks; assert ≥ 30 fps.
- TEST-203: `gzip -c themes/default/static/vendor/sigma/*.min.js | wc -c` in CI; assert ≤ 250 kB.
- TEST-204: Build a seeded 2,000-page vault; assert `graph-index.json` ≤ 1 MB; build a 5,000-page vault; assert stderr contains the expected warning.
- TEST-205: axe-core run against `/_graph`; 0 critical violations; tab-navigable `<details>` fallback contains every node.

---

## 9. Purity Boundary Map

### Pure Core (no I/O, deterministic)

- `graph::serialize_graph_index(vault_ctx) -> serde_json::Value`: transforms in-memory `VaultContext` into CON-101-shaped JSON. Stable ordering. No file or network access.
- `graph::filter_neighbourhood(graph, root_slug, depth) -> Subgraph`: pure subgraph extraction for the per-page panel.

### Effectful Shell

- `web::build::write_graph_index_json(vault_ctx, out_dir)`: calls pure core, writes file, emits OBS-101 log line.
- `web::routes::graph_index_handler(state)`: calls pure core, serves response.
- `web::engine::TemplateEngine::*`: injects `graph_index_url` and `graph_index` variables.

### Boundary Contracts

- `VaultContext` (input, in-process): pure core consumes the same struct that `search_index` and `history_index` already consume.
- CON-101 JSON (output, serialised): crosses process boundary to the browser.

### Dependency Rule

`web::*` depends on `graph::*`; `graph::*` has no dependency on `web::*`. Enforced by module visibility and clippy lint.

---

## 10. Theme Integration Notes

The graph view is designed to be **additive** to the theme system: nothing in `base.html`, `page.html`, `index.html`, or `folder.html` changes semantically except for the optional sidebar link and the optional `{% include "_graph.html" %}` call in `page.html` for the local-graph panel.

| Touchpoint                        | Type           | Override path                                      |
| --------------------------------- | -------------- | -------------------------------------------------- |
| `_graph.html`                     | Partial        | `.zetl/themes/<theme>/_graph.html`                 |
| `vault_graph.html`                | Full page      | `.zetl/themes/<theme>/vault_graph.html`            |
| Sidebar "Graph" link              | `base.html`    | `.zetl/themes/<theme>/base.html`                   |
| Sigma + graphology assets         | Static files   | `.zetl/themes/<theme>/static/vendor/sigma/`        |
| `theme.toml` inline flag          | `graph_inline` | `.zetl/themes/<theme>/theme.toml`                  |

**Data contract to themes** (added to the theme authoring reference):

| Variable           | Type   | Mode       | Description                                                                |
| ------------------ | ------ | ---------- | -------------------------------------------------------------------------- |
| `graph_index_url`  | string | both       | Resolvable URL to `graph-index.json` (absolute in serve, relative in build)|
| `graph_index`      | string | both       | Inline JSON (empty when `graph_inline = false`, which is the default)      |

This mirrors the existing `search_index` contract line-for-line, so theme authors familiar with the search integration need no new mental model.

### 10.1 Static Output Parity

`zetl build` emits:

```
dist/
  _graph.html                              # vault-wide page
  graph-index.json                         # CON-101
  _static/
    vendor/sigma/
      sigma.min.js
      graphology.min.js
      graphology-layout-forceatlas2.min.js
  pages/
    Some Page/
      index.html                           # contains collapsed local-graph panel
```

All assets use relative paths, preserving zetl's "deploy-to-any-CDN" property. `graph_index_url` resolves relative to the emitted HTML's location.

### 10.2 SPA Shell and Persistent Regions

Themes that want "no-flash" graph continuity declare the capability in `theme.toml` and preserve two DOM conventions in their `base.html`.

**`theme.toml` declaration:**

```toml
# .zetl/themes/<theme>/theme.toml
name = "paper"

[spa]
enabled = true                # opt in to the SPA navigation shell
transition = "crossfade"      # "none" | "crossfade" (uses View Transitions API where available)
persistent_regions = ["graph", "sidebar"]   # informational; matches the block names below

[vendor.sigma]
version = "3.0.0"
[vendor.graphology]
version = "0.25.4"
```

**`base.html` structural contract:**

The persistent shell contains the sidebar and the single graph widget. The volatile region contains page content *and* the existing transclusion panel (which is page-specific — it re-renders per page, so it stays in the volatile region).

```html
<body data-slug="{{ page.slug }}">
  {% block persistent_shell %}
    <nav class="zetl-shell zetl-shell--sidebar">
      {% block sidebar %}{% include "_sidebar.html" %}{% endblock %}
    </nav>

    <!-- Single Sigma instance. Default placement: fixed mini-map bottom-right.
         Never unmounted; mode switches on zetl:after-navigate. -->
    <div class="zetl-graph-widget" data-mode="local">
      {% block graph_widget %}{% include "_graph.html" %}{% endblock %}
    </div>
  {% endblock %}

  <main data-zetl-volatile>
    {% block content %}{% endblock %}
    {# page.html renders its transclusion panel inside content — it's page-specific
       and re-renders per page. The graph widget above stays mounted; the
       transclusion panel swaps with the content. #}
  </main>
</body>
```

**Default placement (docked mini-map):**

```
┌──────────────────────────────────────────────────────────┐
│  [Sidebar]   │  page content        │  [Transclusion     │
│              │                      │   panel — sticky]  │
│              │  # Some Page         │                    │
│              │  body…               │  ┌─ forward-link ──┐│
│              │                      │  │ excerpt card   │ │
│              │                      │  └────────────────┘ │
│              │                      │                    │
│              │                      │      ┌──────────┐  │
│              │                      │      │ Graph    │  │ ← fixed
│              │                      │      │ ● ── ●   │  │   bottom-right
│              │                      │      │  ●       │  │   280×200 px
│              │                      │      └──────────┘  │   [ local ▾ ]
└──────────────────────────────────────────────────────────┘
```

The widget is `position: fixed`, offset from the viewport's bottom-right corner. On `/_graph` the widget expands (via CSS transition) to fill the content area; the transclusion panel is absent on `/_graph` because that route doesn't render `page.html`.

**Alternative placements** (opt-in via `theme.toml`):

| `[graph.placement]` | Layout                                                               |
| ------------------- | -------------------------------------------------------------------- |
| `docked` (default)  | Fixed mini-map, bottom-right of viewport.                            |
| `tabs`              | Widget shares the transclusion right rail via a two-tab header.      |
| `stacked`           | Widget sits above the transclusion panel in the right rail.          |

Switching placement changes only CSS and a `data-placement` attribute on the shell container — the Sigma instance, persistent DOM contract, and `_graph.html` partial are untouched.

Rules:

1. Anything inside `{% block persistent_shell %}` is **never swapped** on navigation.
2. The element carrying `data-zetl-volatile` (or the implicit `<main>` fallback) **is swapped** — its `innerHTML` is replaced by the corresponding element from the fetched document.
3. Themes that rewrite `base.html` from scratch MUST preserve both markers to retain the no-flash property. Omitting the markers is a valid opt-out: the theme still works, but the graph re-initialises per page.

**Styling contract (extends §10.1):**

The default theme exposes these CSS custom properties. Custom themes override any subset.

| Property                          | Purpose                                                |
| --------------------------------- | ------------------------------------------------------ |
| `--zetl-graph-node`               | Default node fill                                      |
| `--zetl-graph-node-dead`          | Node fill for dead-link targets                        |
| `--zetl-graph-edge`               | Default edge colour                                    |
| `--zetl-graph-edge-dead`          | Edge colour / pattern for dead-link edges              |
| `--zetl-graph-label`              | Node label colour                                      |
| `--zetl-graph-label-font`         | Node label `font-family` (passed to Sigma at init)     |
| `--zetl-shell-sidebar-area`       | Grid track size for the sidebar shell region           |
| `--zetl-graph-widget-width`       | Docked mini-map width (default 280 px)                 |
| `--zetl-graph-widget-height`      | Docked mini-map height (default 200 px)                |
| `--zetl-graph-widget-right`       | Offset from viewport right (default 16 px)             |
| `--zetl-graph-widget-bottom`      | Offset from viewport bottom (default 16 px)            |
| `--zetl-graph-widget-breakpoint`  | Min viewport width to show widget (default 900 px)     |

Sigma reducers in the default `_graph.html` read these via `getComputedStyle` and refresh on theme mutation. Theme authors who want structural changes (node shape, edge thickness curve, dashed patterns) override `_graph.html` itself.

**JS lifecycle contract:**

The SPA shell dispatches `window`-level events that any theme script can subscribe to:

```js
// .zetl/themes/<theme>/static/enhance.js
window.addEventListener('zetl:after-navigate', (e) => {
  // e.detail = { slug, contentRoot }
  if (window.mermaid) mermaid.run({ nodes: e.detail.contentRoot.querySelectorAll('.mermaid') });
  if (window.renderMathInElement) renderMathInElement(e.detail.contentRoot);
});

window.addEventListener('zetl:before-navigate', (e) => {
  // e.detail = { fromSlug, toSlug, url }
  // Call e.preventDefault() to fall back to native navigation (e.g., editor unsaved changes)
});
```

The graph's own reducers in `_graph.html` also listen for `zetl:after-navigate` and call `renderer.refresh()` with the new `active_slug` to update highlighting — never re-instantiating Sigma.

### 10.3 Serve Mode

`zetl serve` exposes:

- `GET /_graph` → `vault_graph.html`
- `GET /graph-index.json` → CON-101
- `GET /_static/vendor/sigma/*` → bundled assets
- Every rendered page includes the local-graph panel via the partial

No change to `/api/graph` — it stays as the authenticated, richer JSON API for MCP / agent consumption (this spec deliberately keeps the unauth public JSON and the auth API separate).

---

## 11. Open Questions / Ambiguities Requiring Resolution

1. **Sidebar behaviour when theme omits `_graph.html`.** Should `base.html` conditionally hide the "Graph" link, or always render it and let the route 404? Proposed: conditionally hide based on a `graph_enabled` template flag set by the Rust side when the partial resolves. (TEST-109 currently accepts either; tighten before `approved`.)
2. **Local-graph panel default depth.** 1-hop is proposed; Obsidian defaults to 1 with a depth slider. Slider is out of scope for v1. Depth-2 produces much denser graphs. Keep 1.
3. **Frontmatter-driven node colouring.** Proposed as a theme override example, not a core requirement. Fine as-is.
4. **Interaction with `--features history`.** Should nodes fade based on `stable_days`? Deferred — composes SPEC-027 data with SPEC-028 visuals, warrants its own successor spec.
5. **Dead-link nodes in the graph.** CON-101 includes `is_dead: true` nodes. Should they be included by default or filtered? Proposed: included by default, stylistically muted, with a theme-level filter toggle in a successor. REQ-112 codifies inclusion + styling.
6. **SPA shell in `serve` collab mode.** Collab mode injects live-editing UI; interaction with the SPA shell (WebSocket reconnect across navigation, unsaved-edit guard via `zetl:before-navigate.preventDefault()`) needs a brief compatibility note before `approved`. Proposed: the editor registers a `zetl:before-navigate` listener that cancels navigation when there are unsaved CRDT deltas and prompts the user — no structural change to this spec.
7. **Assets in swapped content.** `<script>` tags in swapped content don't execute after DOM replace; `<link rel="stylesheet">` does but causes a repaint. Proposed: the SPA shell re-runs `<script>` tags after swap (standard technique) and hoists common stylesheets to `base.html` — documented in the theme authoring guide.
8. **Shared page registry (deferred to SPEC-029).** `graph-index.json`, the inline `search_index` template variable, and `history-index.json` each carry overlapping per-page metadata (title, slug; and in graph's case also tags, counts, dead flag). The overlap with `search_index` alone is narrow (title + slug), and dropping `label` from graph nodes saves only ~10% of the graph payload because edges dominate. A cleaner long-term factoring is to extract a single `pages-index.json` carrying all presentation metadata, with `search_index`, `graph-index`, and `history-index` pointing into it by slug.

   **Decision for v1:** keep `graph-index.json` self-contained; the saving does not justify coupling two subsystems under one spec, and reshaping `search_index` is a backwards-incompatible change to a documented template variable.

   **Forward reference:** SPEC-029 (*Shared page registry*) will introduce `pages-index.json`, bump the theme contract version (REQ-114), migrate the three existing indices to reference it, and ship a compatibility shim for themes consuming the old `search_index` shape. SPEC-028 will depend on SPEC-029 once it lands; until then, the duplication is accepted technical debt.

---

## 12. Success Criteria

- [ ] All REQ-### have at least one TEST-### (traceability matrix complete).
- [ ] Default theme renders `/_graph` and per-page panel on the demo vault without console errors.
- [ ] `zetl build` on the demo vault emits `graph-index.json` and `_graph.html`; deploying `dist/` to a static host reproduces the serve-mode experience.
- [ ] NFR-101..105 pass on CI-sized vault fixtures.
- [ ] Vendor bundle size within NFR-103 budget.
- [ ] Theme authoring reference in `README.md` updated with `graph_index_url`, `graph_index`, and the `_graph.html` partial.
- [ ] CHANGELOG entry under a new `[0.3.0]` minor (this is a feature addition).

---

## 13. Trace Matrix (Seed)

| REQ     | TEST          | CON     | OBS     |
| ------- | ------------- | ------- | ------- |
| REQ-101 | TEST-101      | CON-101 | OBS-101 |
| REQ-102 | TEST-102      | CON-101 | OBS-102 |
| REQ-103 | TEST-103      | CON-102 | OBS-101 |
| REQ-104 | TEST-104      | —       | —       |
| REQ-105 | TEST-105      | —       | —       |
| REQ-106 | TEST-106      | —       | —       |
| REQ-107 | TEST-107      | —       | —       |
| REQ-108 | TEST-108      | —       | —       |
| REQ-109 | TEST-109      | —       | —       |
| REQ-110 | TEST-110      | —       | —       |
| REQ-111 | TEST-111      | —       | —       |
| REQ-112 | TEST-112      | —       | —       |
| REQ-113 | TEST-113      | —       | OBS-113 |
| REQ-114 | TEST-114      | —       | —       |
| REQ-115 | TEST-115      | —       | —       |
| REQ-116 | TEST-116      | —       | —       |
| REQ-117 | TEST-117      | —       | —       |
| NFR-101 | TEST-201      | —       | OBS-201 |
| NFR-102 | TEST-202      | —       | —       |
| NFR-103 | TEST-203      | —       | —       |
| NFR-104 | TEST-204      | —       | —       |
| NFR-105 | TEST-205      | —       | —       |

---

**END OF SPEC-028 (draft)**
