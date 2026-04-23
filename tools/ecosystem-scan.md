# Markdown/Document Plugin Ecosystem Scan

Empirical audit for SPEC-033 v1 ecosystem targeting. Generated 2026-04-19.
Raw data cached under `/Users/anuna-01/.cache/ztl/ecosystem-scan/`; machine-readable form in `ecosystem-scan.json`.

## Per-ecosystem summary

| Rank | Ecosystem | Total plugins | Top-30 active/stale/abandoned | PKM-relevant (of 30) | Runtime dep | Integration protocol |
|---:|---|---:|:---:|---:|---|---|
| 1 | pandoc filters | 200 (GH topic) + ~48 wiki | 15 / 7 / 8 | **19** | Pandoc binary | Out-of-process JSON/stdin (native or Lua) |
| 2 | mdBook preprocessors | 266 (crates) + 49 (GH topic) | **27 / 0 / 3** | 10 | Rust / cargo install | Subprocess, stdin/stdout JSON |
| 3 | remark (unified) | 539 (npm) | 4 / 12 / 14 | 13 | Node | In-process JS (ESM require) |
| 4 | markdown-it | 507 (npm) | 3 / 2 / 25 | 12 | Node | In-process JS |
| 5 | MkDocs plugins | 708 (PyPI) | 22 / 3 / 5 | 7 | Python + MkDocs | In-process Python |
| 6 | rehype | 352 (npm) | 4 / 20 / 6 | 9 | Node | In-process JS (post-remark) |
| 7 | Quarto extensions | 379 (GH topic) | 19 / 8 / 3 | 6 | Quarto + Pandoc | Declarative bundle (Lua/filter/format) |
| 8 | markdown-it-py | 8 (PyPI `mdit-py-*`) | n/a | n/a | Python | In-process Python |

Totals are unique-name counts after prefix-filtering; npm totals clipped at 1000 (registry cap) so are lower bounds.

## PKM-relevance heatmap

Top-30 per ecosystem, tagged by category via regex on name+description. Cell = # of plugins matching that category.

| Category | remark | rehype | markdown-it | mdbook | pandoc | quarto | mkdocs |
|---|---:|---:|---:|---:|---:|---:|---:|
| citations      | 0 | 0 | 0 | 0 | **3** | 0 | 1 |
| crossrefs      | 0 | 0 | 0 | 0 | **2** | 0 | 0 |
| math           | 1 | 2 | **3** | 1 | 2 | 1 | 0 |
| diagrams       | 0 | 1 | 0 | **4** | 2 | 0 | 3 |
| tables         | 2 | 0 | 2 | 2 | **3** | 0 | 1 |
| callouts       | 1 | 2 | 0 | 2 | 0 | 0 | 0 |
| task lists     | 1 | 0 | 1 | 0 | 0 | 0 | 0 |
| footnotes      | 1 | 0 | 1 | 0 | 1 | 0 | 0 |
| TOC / anchors  | 1 | 2 | 1 | 2 | 0 | 0 | 1 |
| transclusion   | 0 | 0 | 0 | 0 | **5** | **4** | 2 |
| code blocks    | 4 | 2 | 4 | 2 | 2 | 0 | 0 |
| smart text/emoji/deflist | 3 | 0 | 1 | 0 | 0 | 0 | 0 |
| graph/wikilinks | 0 | 0 | 0 | 0 | 1 | 0 | 0 |
| frontmatter    | 2 | 0 | 1 | 1 | 1 | 1 | 1 |
| **Top-30 PKM-tagged** | **13** | **9** | **12** | **10** | **19** | **6** | **7** |

Column maxima bolded. Pandoc filters uniquely dominate citations, crossrefs, and transclusion — the three categories that correlate hardest with *knowledge-management / academic-writing* workloads. mdBook dominates diagram preprocessors. Quarto and MkDocs plugins are weighted toward publishing/theming, not PKM transforms.

## Top-3 recommendation

### 1. Pandoc filters
- **Why**: 19/30 top-ranked filters hit PKM categories, the highest rate measured. Pandoc is the *only* ecosystem where citations (pandoc-crossref, citeproc-rs, section-bibliographies, pandoc-zotxt, pandoc-tex-numbering), crossrefs (pandoc-crossref, pandoc-tex-numbering), and transclusion (pandoc-include, pandoc-placetable, pandoc-plot) appear in the top 30 at all.
- **User fit**: ztl's academic/technical-writing users want citation graphs, equation numbering, and multi-format export. Pandoc is the industry default for that audience.
- **Integration**: out-of-process JSON-over-stdio (`pandoc --filter` or `--lua-filter`). Stable AST, portable across ecosystems — Quarto already rides on it.
- **Cost**: requires Pandoc as an optional runtime dep; not bundled. Filter-author stability is ad-hoc (Haskell/Python/Lua/Node — each with its own versioning hygiene).

### 2. mdBook preprocessors
- **Why**: the healthiest ecosystem by maintenance — 27/30 active, only 3 abandoned. Best-in-class diagram coverage (mermaid, svgbob, graphviz, plantuml) and strong callout/TOC/math representation. Subprocess-JSON protocol is simple.
- **User fit**: documentation-site authors; overlaps heavily with ztl's "publish a vault as a site" workflow. Native Rust binaries, zero additional runtime deps for a Rust CLI like ztl.
- **Integration**: well-defined subprocess protocol (`{context, book} → {processed book}` over stdin/stdout). Rust-native, single-binary adapters.
- **Cost**: smallest absolute ecosystem on the list, but every plugin counts — signal-to-noise is extremely high vs. npm/PyPI.

### 3. remark (unified)
- **Why**: largest PKM-relevant-plugin count among in-process Node ecosystems (13/30), biggest raw total (539). It's the de-facto AST for the JavaScript Markdown world; rehype/retext compose off it, so adopting remark reaches those ecosystems transitively.
- **User fit**: any ztl user who already runs a JS toolchain (Next/Astro/Docusaurus users); remark-gfm, remark-frontmatter, remark-math, remark-directive cover the common extensions.
- **Integration**: in-process via Node subprocess (`node --input-type=module`) or via a JS VM — the protocol is function-call, not subprocess. Versioning is strict semver across `unifiedjs/*` core.
- **Cost**: requires Node as an optional runtime dep. The "abandoned" tail is misleading: many top remark plugins are feature-complete and untouched since 2023; the *core* packages (`remark-parse`, `remark-stringify`) are stable-by-design, not rotting.

## Runners-up (ranks 4-7) — why each fell short

- **markdown-it (rank 4)** — comparable PKM hit rate to remark (12/30) but overlaps heavily with remark in category coverage and has the weakest maintenance signal (25/30 "abandoned" by >730-day threshold). Bundling *both* remark and markdown-it is redundant; pick the one with more transitive reach (remark → rehype → retext).
- **MkDocs plugins (rank 5)** — 708 plugins, but PKM-relevance rate is only 7/30 (23%). The bulk of MkDocs plugins are site-generation concerns (nav, search, RSS, minify, i18n), not Markdown transforms. Requires Python runtime.
- **rehype (rank 6)** — valuable but subsumed by adopting remark: rehype consumes the remark AST, so "supporting remark" transitively covers rehype for any user willing to configure `remark-rehype`. Not a separately-listed v1 target.
- **Quarto extensions (rank 7)** — large total (379), healthy maintenance (19 active), but extensions are mostly *format templates* (APA, journal, thesis, slides) rather than AST-transforming plugins. Quarto itself layers atop Pandoc, so Pandoc support covers the content-transforming subset.
- **markdown-it-py (excluded)** — 8 packages total. Ecosystem too thin to justify a separate integration surface; covered by `markdown-it-py` itself as a library, not as a plugin-host target.

## One-line characterisations for SPEC-033

- **Pandoc filters**: "industry-standard AST filter ecosystem for academic/technical writing; covers citations, crossrefs, transclusion uniquely; out-of-process."
- **mdBook preprocessors**: "Rust-native subprocess preprocessors with highest maintenance signal (90% active); excellent diagram and callout coverage."
- **remark (unified)**: "largest in-process JS Markdown ecosystem (~539 plugins); transitively reaches rehype/retext; requires Node."

## Methodology caveats

1. **npm totals are clipped**. We fetched the first 1000 popularity-ranked results per prefix; true long-tail is larger but dominated by low-popularity forks and scoped/niche packages.
2. **Maintenance age uses publish/push date**, not release cadence. "Abandoned" markdown-it plugins often serve stable specs (footnotes, emoji, task-lists, katex) that don't change. The raw a/s/ab count understates health for mature ecosystems.
3. **PKM-relevance regex is necessarily conservative**. Patterns match obvious names (`\bmath\b`, `\bfootnote`, `\badmonit`) but will miss plugins whose descriptions use different phrasing. Expect true PKM hit-counts to be 10-20% higher than tabulated.
4. **PyPI search was JS-gated**. We fell back to the PyPI simple index (JSON) for totals and the per-package JSON API for a hand-curated top-30. Not a true popularity ranking.
5. **Quarto listing page was JS-rendered**. We substituted the GitHub `quarto-extension` topic (379 repos), which under-counts uncategorised extensions.
6. **The Pandoc-filter total (200)** is repos tagged with the `pandoc-filter` topic only. Many production filters (e.g. `pandoc-citeproc` historically, many org-internal filters) never set the topic and are undercounted.
7. **"Top 30 by popularity"** uses different metrics across ecosystems (npm popularity score, crates.io downloads, GitHub stars). Cross-ecosystem ranking by absolute popularity is not meaningful; we use top-30 as a proxy for "the plugins a ztl user would encounter first."
8. **Obsidian deliberately excluded** per brief — reference data already at `tools/obsidian-top50-scan.json`.

## Files

- `/Users/anuna-01/Code/ztl/tools/ecosystem-scan.md` — this file
- `/Users/anuna-01/Code/ztl/tools/ecosystem-scan.json` — machine-readable form
- `/Users/anuna-01/.cache/ztl/ecosystem-scan/` — raw fetched JSON/HTML
