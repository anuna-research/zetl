---
title: "SPEC-037: 3D Space Graph — Semantic Projection View"
version: 0.1.0
status: draft
date: 2026-04-30
audience: agent, human
parent: SPEC-028
related:
  - SPEC-028  # Interactive 2D Graph View (Sigma.js + graphology)
  - SPEC-018  # Semantic Search (ONNX embeddings, 384-dim vectors)
---

# SPEC-037: 3D Space Graph — Semantic Projection View

## Information Table

| Field          | Value                                                                        |
| -------------- | ---------------------------------------------------------------------------- |
| Document ID    | SPEC-037                                                                     |
| Title          | 3D Space Graph — Semantic Projection View                                    |
| Version        | 0.1.0                                                                        |
| Status         | Draft                                                                        |
| Author         | Agent (USDD Protocol v1.5.0)                                                 |
| Date           | 2026-04-30                                                                   |
| Audience       | Agent, Human                                                                 |
| Trace          | USDD Agent Protocol v1.5.0                                                   |
| Parent         | [[SPEC-028]] (Interactive 2D Graph View)                                     |
| Related        | [[SPEC-018]] (Semantic Search), [[SPEC-028]] (2D Graph View)                 |
| Dependencies   | `--features semantic` (Phase 2+); Three.js or `3d-force-graph` (client); existing `graph-index.json` pipeline |
| Review tier    | Tier 3 (standard feature; visualisation, no security surface)               |

---

## 1. Overview

[[SPEC-028]] provides a 2D force-directed graph view (Sigma.js + graphology + ForceAtlas2) rendered from the link topology in `graph-index.json`. The view reveals **structural** relationships — which pages link to which — but it cannot surface **semantic** relationships between pages that share meaning without sharing links.

[[SPEC-018]] computes 384-dimensional normalised embeddings (`all-MiniLM-L6-v2`) for every page chunk and stores them at `.zetl/search/vectors/`. These embeddings encode semantic proximity: two pages about feedback systems in different domains will occupy nearby positions in the 384-dim vector space even if no `[[wikilink]]` connects them.

This spec introduces a **separate, lazy-loaded 3D "space" view** that projects page embeddings into three dimensions and renders them as an interactive scene. The 2D link graph ([[SPEC-028]]) is not replaced; both views coexist, linked by a "2D / 3D / Density" switch on the `/_graph` page and by a dedicated `/_space` route.

### 1.1 Motivation

- **Semantic topology is invisible in the link graph.** Orphan pages that share concepts with well-connected pages appear isolated in the 2D force layout. A 3D projection makes conceptual clusters visible even in the absence of links.
- **TDA Mapper layers expose density structure.** Topological data analysis (Mapper) can partition the projected space into overlapping regions, surfacing dense conceptual areas and boundary pages that bridge domains.
- **Precomputed coordinates preserve static builds.** All dimensionality reduction runs at build/serve time (Rust side). The browser receives a small, deterministic JSON payload and renders it with a lazy-loaded Three.js bundle — no client-side UMAP, no layout jitter, no large WASM dependency.
- **Incremental delivery.** Phase 1 reuses the existing link graph in 3D (no semantic feature required). Phase 2 adds semantic projection. Phase 3 adds TDA Mapper. Each phase ships independently.

### 1.2 Design Principles

1. **Separate view, not a replacement.** The 3D space graph is a distinct route and a distinct template. The existing 2D Sigma.js graph ([[SPEC-028]]) is untouched.
2. **Precompute server-side, render client-side.** All PCA / clustering / Mapper computation runs in Rust during `zetl build` or `zetl serve`. The browser receives a static `graph-space.json` with pre-baked 3D coordinates and cluster labels. This keeps static deployments working, avoids a large client bundle, and guarantees deterministic rendering.
3. **Lazy-loaded.** The 3D renderer (Three.js / `3d-force-graph`) is only fetched when the user navigates to the space view. The existing 2D graph payload is unaffected.
4. **Progressive enhancement via `--features semantic`.** Without the semantic feature, Phase 1 renders the link graph in 3D using random or graph-distance-based initial coordinates. With the semantic feature, Phase 2 replaces coordinates with PCA-projected page-mean embeddings.
5. **Themeable.** The space view is a Minijinja partial (`_space.html`) in the theme layer, overridable via `.zetl/themes/<theme>/`.
6. **Feature-gated JSON.** `graph-space.json` is only written when the space view is enabled (theme opt-in or explicit flag). When absent, the `/_space` route returns a graceful "Space view requires semantic indexing" message.

### 1.3 Scope

**In scope:**

- A new JSON artefact `graph-space.json` written by `zetl build` and served by `zetl serve`, containing 3D coordinates, cluster labels, density values, link edges, and semantic edges.
- A pure Rust function `serialize_graph_space` in `src/graph.rs` (Phase 1) and `serialize_graph_space_semantic` behind `--features semantic` (Phase 2+).
- A deterministic PCA-3 implementation in `src/semantic/` that reduces per-page mean embeddings from 384D to 3D.
- A TDA Mapper layer (Phase 3) that clusters projected pages into overlapping regions and exposes density/cluster metadata.
- A new template `themes/default/_space.html` rendering a Three.js or `3d-force-graph` scene.
- A new route `/_space` (serve) and `_space.html` (build).
- A "2D / 3D" view switch (or "2D / 3D / Density" after Phase 3) on the existing `/_graph` page that lazy-loads the space view.
- Lazy-loading: the 3D JS bundle is only fetched on the space view, never on other pages.
- Observability: `OBS-` counters for projection duration, node/edge counts, and payload size.
- Documentation: README section, theme authoring reference, CHANGELOG entry.

**Out of scope:**

- Replacing or modifying the existing 2D Sigma.js graph ([[SPEC-028]]).
- Real-time coordinate updates during serve mode (coordinates are computed at build/index time).
- Editing the graph from the 3D view (drag-to-link, node creation) — read-only, same as [[SPEC-028]].
- VR / AR rendering.
- Exporting 3D scenes to image files (SVG/PNG export of the 3D view).
- Client-side UMAP, t-SNE, or any iterative dimensionality reduction.
- Mobile gesture polish beyond Three.js / `3d-force-graph` defaults.
- Theming of non-default bundled themes.

---

## 2. User Profiles and Happy Paths

### 2.1 User: Vault author exploring semantic clusters

**Role:** Individual knowledge-worker running `zetl serve` with `--features semantic`.
**Goal:** Discover which of their pages are conceptually related but not linked.
**Constraints:** Desktop browser; keyboard + mouse; expects smooth 3D interaction (orbit, zoom, pan).

**Happy path:**

1. `zetl serve` → navigate to `http://localhost:3000` → see "Space" in sidebar → click → `/_space` renders within 2s for vaults up to 5,000 pages.
2. Orbit the scene with mouse drag, zoom with scroll. Pages that share meaning cluster together visually, regardless of link topology.
3. Click a node → navigate to that page. Hover a node → see label + semantic neighbours highlighted.
4. Switch to "Density" view → nodes are coloured by local embedding density, revealing conceptual hotspots.

### 2.2 User: Vault author without semantic feature

**Role:** Runs `zetl serve` without `--features semantic`.
**Goal:** See their link graph in 3D as a visual alternative to the 2D force layout.
**Constraints:** Desktop browser.

**Happy path:**

1. `zetl serve` → navigate to `/_space` → 3D graph renders using graph-distance-based coordinates (Phase 1).
2. Interact identically to the semantic user — orbit, zoom, click-to-navigate.
3. A banner explains that enabling `--features semantic` would add semantic projection.

### 2.3 User: Static-site publisher

**Role:** Runs `zetl build`, deploys to static host.
**Goal:** Offer public readers a 3D semantic view of the vault.
**Constraints:** No server; CDN-only hosting.

**Happy path:**

1. `zetl build --features semantic --out-dir dist` → `dist/_space.html`, `dist/graph-space.json`, `dist/_static/vendor/three/*.js` are written.
2. Deploy `dist/` → `/_space.html` renders identically to serve mode.
3. Lazy-loading ensures readers who never visit `/_space` never download the 3D bundle.

### 2.4 User: Theme author

**Role:** Customises `.zetl/themes/<theme>/`.
**Goal:** Restyle the 3D view, swap the renderer, or disable the space view entirely.
**Constraints:** Only overrides templates and static assets; no Rust.

**Happy path:**

1. Override `_space.html` → replace Three.js with a custom renderer using `graph-space.json`.
2. Set `space_view = false` in `theme.toml` → the `/_space` route returns 404 and the sidebar link is hidden.

---

## 3. Requirements

### 3.1 Functional Requirements

#### Phase 1: 3D Link Graph (no semantic feature required)

**REQ-201:** The system SHALL write a `graph-space.json` file to the build output directory during `zetl build` and serve it at `GET /graph-space.json` during `zetl serve`, using the `zetl-graph-space/v1` format, containing 3D coordinates derived from the existing link graph topology.

**REQ-202:** When `--features semantic` is disabled, the system SHALL compute initial 3D node positions using a deterministic graph-distance-based projection (e.g., spectral embedding of the graph Laplacian, or spring-electrical layout with a fixed seed), producing stable `x`, `y`, `z` coordinates that are identical across rebuilds of the same vault state.

**REQ-203:** `graph-space.json` SHALL include all nodes and edges from `graph-index.json` (same slugs, same edge pairs), ensuring the 3D view and 2D view present the same graph topology.

**REQ-204:** The system SHALL emit a template variable `space_index_url` (string) available in all templates, pointing at the externalised `graph-space.json` asset.

**REQ-205:** The system SHALL emit a template variable `space_index` (string, `""` by default), non-empty only when the theme opts in via `space_inline = true` in `theme.toml`.

**REQ-206:** The system SHALL provide a new route `/_space` (serve mode) and `_space.html` (build mode), rendering a 3D interactive graph view.

**REQ-207:** The system SHALL lazy-load the 3D renderer JavaScript bundle (Three.js or `3d-force-graph`) only when the user navigates to the space view, ensuring zero payload impact on other pages.

**REQ-208:** Node click in the 3D view SHALL navigate to the corresponding page, mirroring the [[SPEC-028]] click-to-navigate behaviour (modifier keys open in new tab; SPA-aware).

**REQ-209:** The system SHALL provide a view switch ("2D / 3D") on the existing `/_graph` page that transitions between the Sigma.js 2D view and the 3D space view without a full page reload, lazy-loading the 3D bundle on first switch.

#### Phase 2: Semantic Projection (requires `--features semantic`)

**REQ-210:** When `--features semantic` is active, the system SHALL compute a page-mean embedding for each page by averaging the 384-dimensional chunk embeddings (`[[EMBEDDING_DIM]]` = 384, see [[SPEC-018]] `src/semantic/mod.rs:28`) across all chunks belonging to that page, producing one 384-dim vector per page.

**REQ-211:** The system SHALL reduce the page-mean embeddings from 384 dimensions to 3 dimensions using deterministic Principal Component Analysis (PCA-3), selecting the top 3 principal components by explained variance.

**REQ-212:** PCA-3 projection SHALL be deterministic: for identical vault content and identical embeddings, the resulting `(x, y, z)` coordinates SHALL be byte-identical across runs, ensuring stable diffs in `graph-space.json`.

**REQ-213:** `graph-space.json` SHALL include a `projection` metadata object indicating the reduction method (`pca3`), the source (`page-mean-embeddings`), and the cumulative explained variance ratio for the 3 selected components.

**REQ-214:** `graph-space.json` SHALL include a `semantic_edges` array listing pairs of pages whose cosine similarity exceeds a configurable threshold (default 0.75), with each entry carrying `source`, `target`, and `score` fields.

**REQ-215:** The system SHALL normalise projected coordinates to the unit cube `[-1, 1]³` before writing to `graph-space.json`, ensuring consistent camera framing across vaults of different sizes.

#### Phase 3: TDA Mapper Layer

**REQ-216:** When `--features semantic` is active, the system SHALL cluster the projected 3D points into overlapping regions using a Mapper-style algorithm (bin the PCA-3 space, cluster within each bin, connect adjacent clusters), and assign each page a `cluster` integer label and a `density` float in `[0, 1]` representing local point density.

**REQ-217:** `graph-space.json` SHALL include `cluster` and `density` fields on each node when the Mapper layer has been computed.

**REQ-218:** The 3D view SHALL support a "Density" rendering mode where node colour and size encode the `density` field, visually distinguishing dense conceptual regions from sparse boundaries.

**REQ-219:** The view switch on `/_graph` SHALL expand to "2D / 3D / Density" when the Mapper layer is available.

### 3.2 Non-Functional Requirements

**NFR-201:** `graph-space.json` SHALL be ≤ 2× the byte size of `graph-index.json` for the same vault, measured at the 95th percentile across vaults of 100–10,000 pages.

**NFR-202:** The 3D view SHALL render its first frame within 3 seconds on a mid-range desktop browser (2024 hardware) for vaults up to 5,000 pages.

**NFR-203:** PCA-3 projection SHALL complete within the `zetl build` index phase, adding ≤ 30% overhead to the semantic embedding pipeline duration.

**NFR-204:** The lazy-loaded 3D JS bundle SHALL be ≤ 200 KB gzipped (Three.js minimal or `3d-force-graph` with tree-shaking).

**NFR-205:** Deterministic rebuilds: for identical vault content, `graph-space.json` SHALL produce byte-identical output across runs (stable sort, fixed seed, no randomness after embedding).

**NFR-206:** Accessibility: the `/_space` route SHALL include a `<noscript>` fallback and a `<details>` page list (matching the [[SPEC-028]] pattern from `_graph.html`), ensuring all pages are reachable without JavaScript.

### 3.3 Contract Specifications

**CON-201: `graph-space.json` format**

```
{
  "format": "zetl-graph-space/v1",
  "projection": {
    "method": "pca3" | "graph-distance",
    "source": "page-mean-embeddings" | "link-topology",
    "explained_variance": [0.35, 0.22, 0.14]   // null when method=graph-distance
  },
  "nodes": [
    {
      "slug": "page-slug",
      "label": "Page Name",
      "x": 0.1,        // f64, normalised to [-1, 1]
      "y": -0.2,        // f64, normalised to [-1, 1]
      "z": 0.4,         // f64, normalised to [-1, 1]
      "density": 0.72,  // f64 in [0, 1], null when Mapper not computed
      "cluster": 3      // u32, null when Mapper not computed
    }
  ],
  "edges": [
    { "source": "slug-a", "target": "slug-b", "type": "wikilink" }
  ],
  "semantic_edges": [
    { "source": "slug-a", "target": "slug-c", "score": 0.86 }
  ]
}
```

Pre-conditions:
- `nodes` is sorted alphabetically by `slug`.
- `edges` is sorted alphabetically by `"source->target"` composite key.
- `semantic_edges` is sorted by descending `score`, then alphabetically by source.
- All `slug` values in `edges` and `semantic_edges` reference a node present in `nodes`.

Post-conditions:
- The JSON is valid `zetl-graph-space/v1`.
- Byte-identical for the same vault state (deterministic serialisation).

Error model:
- When `--features semantic` is disabled: `projection.method` = `"graph-distance"`, `semantic_edges` = `[]`, `projection.explained_variance` = `null`.
- When no embeddings exist (semantic feature enabled but `zetl index` not yet run): same as above, with an `attributes.warning` field set to `"semantic-index-missing"`.

Implements: REQ-201, REQ-203, REQ-213, REQ-214, REQ-215.

Verified by: TEST-201, TEST-203, TEST-213.

**CON-202: `/_space` template contract**

The template receives:

| Variable            | Type   | Description                                                        |
| ------------------- | ------ | ------------------------------------------------------------------ |
| `space_index_url`   | string | URL to `graph-space.json` (e.g. `"../graph-space.json"`)           |
| `space_index`       | string | Inline JSON when `space_inline = true`; `""` otherwise             |
| `root_path`         | string | Relative path to vault root (matches [[SPEC-028]])                 |
| `index_file`        | string | `"index.html"` in build mode, `""` in serve mode                   |
| `vault.stats`       | object | `{ total_pages, total_links }` for graceful-absence checks         |

The template MUST:

- Include a `<div id="zetl-space">` container for the 3D renderer.
- Include `<noscript>` fallback and empty-state messaging per [[SPEC-028]]'s pattern.
- Load `graph-boot.js` resolution logic (pin URLs at script-load time).
- Define `window.__zetlSpaceConfig` with `root`, `spaceUrl`, and `indexFile` fields.
- Lazy-load the 3D vendor bundle from `_static/vendor/three/`.
- Include a `<details>` keyboard-accessible page list as a fallback.

### 3.4 Architecture Decisions

**ADR-055: Precompute coordinates server-side, do not run UMAP/Mapper in the browser**

- **Context:** UMAP produces better embeddings than PCA for visualisation, but requires iterative optimisation and a large WASM bundle.
- **Decision:** All dimensionality reduction runs in Rust during `zetl build` / `zetl index`. The browser receives pre-baked `(x, y, z)` coordinates and only renders them.
- **Rationale:** Static builds must work without a server. Client-side UMAP would add 500 KB+ of WASM, produce non-deterministic layouts, and cause layout jitter on re-render. PCA-3 is deterministic, fast, and produces stable coordinates suitable for diffs.
- **Consequences:** Semantic clusters may appear less visually separated than UMAP would produce. Mitigated by Phase 3 Mapper overlay. Acceptable for MVP.
- **Alternatives considered:** Client-side UMAP (rejected: bundle size, jitter, static-host incompatibility); server-side UMAP (deferred: may be added as an opt-in in a successor spec).

**ADR-056: PCA-3 over UMAP for initial projection**

- **Context:** Need to reduce 384-dim embeddings to 3D for visualisation.
- **Decision:** Use deterministic PCA (power iteration or SVD on the covariance matrix) to extract the top 3 principal components.
- **Rationale:** PCA is deterministic, fast (O(n·d²) for n pages, d=384), requires no random seed, and produces stable output across runs. UMAP is non-deterministic and requires hyperparameter tuning.
- **Consequences:** Linear projection may compress non-linear manifold structure. Acceptable because Phase 3 Mapper adds non-linear cluster structure on top.
- **Alternatives considered:** Truncated SVD (equivalent to PCA for centred data; use whichever is simpler in `ndarray`); t-SNE (rejected: non-deterministic, O(n²) memory, per-run variation).

**ADR-057: Three.js via `3d-force-graph` for the client renderer**

- **Context:** Need a 3D graph renderer for the browser.
- **Decision:** Bundle [`3d-force-graph`](https://github.com/vasturiano/3d-force-graph) (MIT) as the default renderer, which wraps Three.js and provides graph-specific affordances (node/edge rendering, hover labels, click events).
- **Rationale:** `3d-force-graph` is widely used (8k+ GitHub stars), MIT-licensed, and handles WebGL setup, camera controls, and graph rendering out of the box. Theme authors can swap it for raw Three.js by overriding `_space.html`.
- **Consequences:** Adds ~150 KB gzipped to the lazy-loaded bundle. Force simulation is disabled (coordinates are precomputed), so the renderer is used purely as a display layer.
- **Alternatives considered:** Raw Three.js (rejected: too much boilerplate for graph rendering); `sigma.js` 3D mode (rejected: Sigma is 2D-only); `cytoscape.js` 3D (rejected: limited 3D support).

**ADR-058: Separate `graph-space.json` rather than extending `graph-index.json`**

- **Context:** Could add 3D coordinates and semantic edges to the existing `graph-index.json`.
- **Decision:** Create a separate `graph-space.json` file with its own format version (`zetl-graph-space/v1`).
- **Rationale:** Separation of concerns — the 2D graph ([[SPEC-028]]) does not need 3D coordinates, and the 3D view does not need graphology-specific attributes. Separate files allow independent caching, lazy loading, and format versioning. `graph-index.json` remains unchanged for all existing consumers.
- **Consequences:** Two JSON files to maintain. Mitigated by sharing the same source data (link graph + embeddings).

---

## 4. Data Flow

### 4.1 Build Pipeline (Phase 1 — link topology only)

```
                         ┌─────────────────────────────┐
  LinkGraph (petgraph) ──┤ serialize_graph_space()      │
  page_slug_map ─────────┤   graph-distance projection  ├──► graph-space.json
  tags_by_page ──────────┤   deterministic seed         │
                         └─────────────────────────────┘
```

`serialize_graph_space()` is a pure function in `src/graph.rs` (adjacent to the existing `serialize_graph_index()` at `src/graph.rs:595`). It reuses the same `LinkGraph` and `page_slug_map` inputs. For Phase 1, it computes a simple spectral embedding of the graph Laplacian to produce `(x, y, z)` coordinates.

### 4.2 Build Pipeline (Phase 2 — semantic projection)

```
                         ┌─────────────────────────────┐
  LinkGraph ─────────────┤ serialize_graph_space()      │
  page_slug_map ─────────┤   merge embeddings + links   │
  tags_by_page ──────────┤                              │
                         │   ┌────────────────────────┐ │
  VectorIndex.embeddings─┤───┤ page_mean_embeddings()  │ │
  VectorIndex.chunks ────┤   └──────────┬─────────────┘ │
                         │              │                │
                         │   ┌──────────▼─────────────┐ │
                         │   │ pca3_project()          │ │
                         │   │   ndaray SVD / power-it │ │
                         │   └──────────┬─────────────┘ │
                         │              │                │
                         │   ┌──────────▼─────────────┐ │
                         │   │ normalise_to_unit_cube  │ │
                         │   └──────────┬─────────────┘ │
                         │              │                │
                         │   ┌──────────▼─────────────┐ │
                         │   │ semantic_edges()        │ │
                         │   │   cosine_sim > threshold│ │
                         │   └──────────┬─────────────┘ │
                         │              ▼                │
                         │      graph-space.json        │
                         └─────────────────────────────┘
```

### 4.3 Client Rendering

```
  _space.html
    │
    ├─ window.__zetlSpaceConfig (root, spaceUrl, indexFile)
    │
    ├─ space-boot.js (inline, < 2 KB)
    │     └─ resolves URLs, lazy-loads vendor bundle
    │
    ├─ _static/vendor/three/3d-force-graph.min.js (lazy, ~150 KB gzip)
    │
    └─ fetch(graph-space.json) → render nodes + edges + semantic_edges
          │
          ├─ Nodes: spheres at (x,y,z), sized by degree, coloured by folder (matching [[SPEC-028]])
          ├─ Edges: lines (wikilink edges)
          ├─ Semantic edges: translucent lines (optional toggle)
          └─ Interaction: orbit, zoom, hover-label, click-to-navigate
```

---

## 5. File Layout

### 5.1 Rust Source Changes

| File | Change | Phase |
|------|--------|-------|
| `src/graph.rs` | Add `serialize_graph_space()` and `GraphSpaceNode` / `GraphSpaceEdge` types | 1 |
| `src/graph.rs` | Add `serialize_graph_space_semantic()` gated behind `#[cfg(feature = "semantic")]` | 2 |
| `src/semantic/projection.rs` | **New file.** Pure functions: `page_mean_embeddings()`, `pca3_project()`, `normalise_to_unit_cube()`, `semantic_edges()` | 2 |
| `src/semantic/mapper.rs` | **New file.** TDA Mapper clustering: `mapper_cluster()`, `compute_density()` | 3 |
| `src/semantic/mod.rs` | Add `pub mod projection;` and `pub mod mapper;` (behind `#[cfg(feature = "semantic")]`) | 2, 3 |
| `src/web/build.rs` | Add `write_graph_space_json()` (mirrors `write_graph_index_json()` at `src/web/build.rs:243`) | 1 |
| `src/web/serve.rs` | Add `GET /graph-space.json` route and `/_space` template route | 1 |
| `src/web/engine.rs` | Add `space_index_url` and `space_index` template variables | 1 |

### 5.2 Theme Assets

| File | Change | Phase |
|------|--------|-------|
| `themes/default/_space.html` | **New.** Minijinja partial for 3D view (mirrors `_graph.html`) | 1 |
| `themes/default/vault_space.html` | **New.** Full-page template for `/_space` route (mirrors `vault_graph.html`) | 1 |
| `themes/default/static/space-boot.js` | **New.** Lazy-load + URL resolution (mirrors `graph-boot.js`) | 1 |
| `themes/default/static/vendor/three/3d-force-graph.min.js` | **New.** Bundled renderer | 1 |
| `themes/default/static/vendor/three/LICENSE.txt` | **New.** MIT license | 1 |
| `themes/default/vault_graph.html` | Add "2D / 3D" view switch | 1 |
| `themes/default/base.html` | Add "Space" sidebar link | 1 |

### 5.3 JSON Output

| File | Format | Phase |
|------|--------|-------|
| `graph-space.json` | `zetl-graph-space/v1` (see CON-201) | 1 |

---

## 6. Implementation Phases

### Phase 1: 3D Link Graph (MVP)

**Goal:** Render the existing `graph-index.json` topology in 3D with bundled `3d-force-graph`. Fast, visually proves the concept, no semantic feature required.

**Tasks:**

1. Add `serialize_graph_space()` to `src/graph.rs` — pure function that takes the same inputs as `serialize_graph_index()` and produces `graph-space.json` with deterministic graph-distance-based coordinates.
2. Add `write_graph_space_json()` to `src/web/build.rs` — mirrors `write_graph_index_json()` at `src/web/build.rs:243`.
3. Add `GET /graph-space.json` and `GET /_space` routes to `src/web/serve.rs`.
4. Add `space_index_url` / `space_index` template variables to `src/web/engine.rs`.
5. Create `themes/default/_space.html`, `themes/default/vault_space.html`, `themes/default/static/space-boot.js`.
6. Bundle `3d-force-graph` under `themes/default/static/vendor/three/`.
7. Add "Space" sidebar link to `themes/default/base.html`.
8. Add "2D / 3D" view switch to `themes/default/vault_graph.html`.
9. Graceful absence: `<noscript>`, empty-state messaging, feature-off state.
10. Observability: `OBS-201` `[zetl] space-export: pages=N edges=M duration_ms=X bytes=Y`.

**Quality gates:**

- `graph-space.json` is byte-identical across rebuilds of the same vault.
- `/_space` renders in ≤ 3s for 2,000 pages.
- Lazy-loaded 3D bundle is not fetched on any page except `/_space`.
- All nodes from `graph-index.json` appear in `graph-space.json`.

### Phase 2: Semantic Projection

**Goal:** Average chunk embeddings per page, reduce 384D to 3D with deterministic PCA, write `graph-space.json` with semantic coordinates and semantic edges.

**Tasks:**

1. Create `src/semantic/projection.rs` with `page_mean_embeddings()`, `pca3_project()`, `normalise_to_unit_cube()`, `semantic_edges()`.
2. Add `serialize_graph_space_semantic()` to `src/graph.rs` behind `#[cfg(feature = "semantic")]`.
3. Wire into `write_graph_space_json()`: when `--features semantic` and a vector index exists, use semantic projection instead of graph-distance.
4. Add `projection` metadata to `graph-space.json` with method, source, and explained variance.
5. Update `space-boot.js` to render semantic edges as translucent lines.
6. Update `_space.html` to show a banner indicating "Semantic projection active" vs "Link topology only".

**Quality gates:**

- PCA-3 output is deterministic for the same embeddings.
- `projection.explained_variance` sums to a value in [0, 1].
- Semantic edges have `score` in [0, 1].
- `graph-space.json` byte size ≤ 2× `graph-index.json`.
- Projection adds ≤ 30% overhead to the embedding pipeline.

### Phase 3: TDA Mapper Layer

**Goal:** Cluster projected/embedded pages into overlapping regions, expose density and cluster views.

**Tasks:**

1. Create `src/semantic/mapper.rs` with `mapper_cluster()` and `compute_density()`.
2. Add `density` and `cluster` fields to `GraphSpaceNode`.
3. Update `serialize_graph_space_semantic()` to include Mapper output.
4. Add "Density" rendering mode to `space-boot.js` (colour/size by density).
5. Expand view switch to "2D / 3D / Density".
6. Add cluster legend to `_space.html`.

**Quality gates:**

- Mapper clusters are deterministic for the same input.
- `density` values are in [0, 1].
- Every node has a `cluster` label (no unassigned nodes).
- Cluster labels are stable across minor content changes (single-page edit does not renumber all clusters).

---

## 7. Purity Boundary Map

### Pure Core (no I/O, no shared state, deterministic)

- `page_mean_embeddings(chunks, embeddings) -> HashMap<String, [f32; 384]>`
- `pca3_project(embeddings: &[[f32; 384]]) -> Vec<[f32; 3]>`
- `normalise_to_unit_cube(coords: &[[f32; 3]]) -> Vec<[f32; 3]>`
- `semantic_edges(means: &HashMap<String, [f32; 384]>, threshold: f32) -> Vec<SemanticEdge>`
- `mapper_cluster(coords: &[[f32; 3]], config: MapperConfig) -> Vec<MapperResult>`
- `compute_density(coords: &[[f32; 3]], radius: f32) -> Vec<f32>`
- `serialize_graph_space(...)` (Phase 1 — pure data transformation)
- `serialize_graph_space_semantic(...)` (Phase 2+ — pure data transformation)

### Effectful Shell (orchestrates I/O, calls pure core)

- `write_graph_space_json()` in `src/web/build.rs` — reads vault files, calls pure core, writes `graph-space.json`
- `GET /graph-space.json` route handler in `src/web/serve.rs` — calls pure core, serialises to response
- `VectorIndex::build()` in `src/semantic/mod.rs` — loads ONNX model, calls pure core, persists to disk

### Boundary Contracts (data types crossing the boundary)

- `GraphSpaceNode` (Rust struct → JSON): crosses from pure core to effectful shell (serialisation)
- `HashMap<String, [f32; EMBEDDING_DIM]>` (page-mean embeddings): crosses from `VectorIndex` shell to `projection` pure core
- `Vec<[f32; 3]>` (projected coordinates): crosses from `projection` core to `serialize_graph_space_semantic`

### Dependency Rule

Dependencies point inward: shell → core. Core MUST NOT import from shell.

### Enforcement

Rust module visibility: `projection` and `mapper` are `pub` within the `semantic` module but their functions take only plain data types, never I/O handles. The `#[cfg(feature = "semantic")]` gate prevents any compile-time dependency leak.

---

## 8. Testing Strategy

### Verification Techniques

| Technique | Scope | Rationale |
|-----------|-------|-----------|
| Example-based testing | All REQs | Baseline coverage for every requirement |
| Property-based testing | PCA-3, Mapper, density | Algebraic invariants (determinism, bounds, symmetry) |
| Mutation testing | `projection.rs`, `mapper.rs` | Pure core — ideal target for mutation testing |

### Test Specifications

**TEST-201: `graph-space.json` format validity**

Validates: REQ-201

- Positive: given a vault with 3 pages and 2 edges, `serialize_graph_space()` produces valid JSON matching CON-201 schema.
- Negative-input: empty vault (0 pages) → `graph-space.json` has empty `nodes` and `edges` arrays.
- Negative-output: every node has exactly 3 numeric coordinates; no NaN, no Infinity.

**TEST-202: Deterministic graph-distance coordinates**

Validates: REQ-202

- Positive: two calls with identical `LinkGraph` and `page_slug_map` produce byte-identical JSON.
- Negative-output: swapping two pages in the graph produces different coordinates (non-constant output).

**TEST-203: Node/edge parity with `graph-index.json`**

Validates: REQ-203

- Positive: every node slug in `graph-index.json` appears in `graph-space.json` and vice versa.
- Positive: every edge `(source, target)` in `graph-index.json` appears in `graph-space.json.edges` and vice versa.

**TEST-210: Page-mean embedding correctness**

Validates: REQ-210

- Positive: a page with 3 chunks produces a mean embedding that is the arithmetic mean of the 3 chunk embeddings.
- Negative-input: a page with 0 chunks (edge case) → omitted from the output, not panicked.

**TEST-211: PCA-3 determinism and bounds**

Validates: REQ-211, REQ-212

- Positive: two calls with identical embeddings produce identical `(x, y, z)` triples (determinism).
- Property: for any input, all output coordinates are finite (no NaN, no Infinity).
- Property: the first principal component explains ≥ as much variance as the second, which explains ≥ as much as the third (monotonicity).

**TEST-214: Semantic edge threshold**

Validates: REQ-214

- Positive: two pages with cosine similarity 0.90 appear in `semantic_edges` when threshold = 0.75.
- Negative-input: two pages with cosine similarity 0.50 do NOT appear when threshold = 0.75.
- Negative-output: every `score` is in [threshold, 1.0].

**TEST-215: Unit-cube normalisation**

Validates: REQ-215

- Positive: all `(x, y, z)` values lie within `[-1, 1]`.
- Positive: at least one coordinate has absolute value > 0.9 (not collapsed to origin).
- Negative-output: NaN or Infinity coordinates are rejected.

**TEST-216: Mapper cluster completeness**

Validates: REQ-216

- Positive: every projected node receives a `cluster` label.
- Positive: overlapping clusters exist (some nodes belong to multiple bins).
- Negative-output: no `cluster` label is negative.

**TEST-217: Density bounds**

Validates: REQ-217

- Positive: all `density` values are in [0, 1].
- Positive: the densest node has `density` = 1.0 (normalised to max).

---

## 9. Observability

**OBS-201:** `[zetl] space-export: pages=N edges=M semantic_edges=S duration_ms=X bytes=Y`

Emitted under `--verbose` during `zetl build`, mirroring the existing `graph-export:` instrumentation at `src/web/build.rs:296`.

**OBS-202:** `[zetl] pca3: pages=N explained_variance=[V1,V2,V3] duration_ms=X`

Emitted when PCA-3 projection runs.

**OBS-203:** `[zetl] mapper: nodes=N clusters=C duration_ms=X`

Emitted when TDA Mapper clustering runs.

**OBS-204:** Browser-side `performance.measure('zetl:space:render', 'zetl:space:render:start')`

Mirrors the existing `zetl:graph:render` measure in `graph-boot.js`.

---

## 10. Risks and Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| PCA-3 produces poor visual separation for certain vaults | Medium | Low | Phase 3 Mapper overlay adds non-linear structure; theme authors can swap the renderer. Future successor spec may add UMAP as opt-in. |
| `3d-force-graph` bundle exceeds 200 KB gzip | Low | Medium | Tree-shake unused features; fall back to raw Three.js if needed. ADR-057 reserves the right to swap. |
| Mapper clustering is non-deterministic | Low | High | Use deterministic k-means within each bin with fixed seed; require TEST-202-style determinism test. |
| Semantic edges array is too large for densely-linked vaults | Medium | Low | Cap semantic edges to top-N per node (configurable, default 10). Downsample below threshold for vaults > 10,000 pages. |
| `--features semantic` binary size regression | Low | Low | PCA/Mapper code is small; `ndarray` is already a dependency of the semantic feature. No new heavy deps. |

---

## 11. Success Criteria

### Minimum requirements (Phase 1)

- [ ] All Phase 1 REQs implemented and tested
- [ ] `/_space` renders a 3D graph for vaults up to 2,000 pages within 3s
- [ ] `graph-space.json` is byte-identical across rebuilds
- [ ] Lazy-loading verified: 3D bundle not fetched on non-space pages
- [ ] Graceful absence: `<noscript>`, empty-state, feature-off all work

### Convergence signals (all phases)

- **Specification surface:** Review passes produce zero structural changes to REQs or ADRs.
- **Test surface:** Every REQ has at least one positive and one negative test. Mutation testing kill rate ≥ 90% on `projection.rs` and `mapper.rs`.
- **Implementation surface:** Code review findings are cosmetic, not structural.
- **Performance surface:** NFR-201 (payload size) and NFR-202 (render time) met with margin.
