---
title: "SPEC-033: Ecosystem Bridges — Typed Parser and Hook Architecture for Pandoc, mdBook, and remark"
version: 0.1.0
status: draft
date: 2026-04-19
audience: agent, human
supersedes: SPEC-031
parent: SPEC-032
related:
  - SPEC-031  # Obsidian plugin shim (superseded — scan findings retained)
  - SPEC-032  # Three-stage render hooks (parent; provides ast_type forward-compat machinery)
  - SPEC-004  # Web UI and static export
  - SPEC-028  # Versioned theme contract (precedent for multi-version compatibility matrices)
---

# SPEC-033: Ecosystem Bridges — Typed Parser and Hook Architecture for Pandoc, mdBook, and remark

## Information Table

| Field        | Value                                                                   |
| ------------ | ----------------------------------------------------------------------- |
| Document ID  | SPEC-033                                                                |
| Title        | Ecosystem Bridges — Typed Parser and Hook Architecture for Pandoc, mdBook, and remark |
| Version      | 0.1.0                                                                   |
| Status       | Draft                                                                   |
| Author       | Agent (USDD Protocol v1.3.0)                                            |
| Date         | 2026-04-19                                                              |
| Audience     | Agent, Human                                                            |
| Trace        | USDD Agent Protocol v1.3.0                                              |
| Parent       | SPEC-032: Three-stage render hooks + first-class file selection         |
| Supersedes   | SPEC-031 (Obsidian plugin shim)                                         |
| Related      | SPEC-004 (web UI), SPEC-028 (versioned theme contract)                  |
| Dependencies | SPEC-032's hook-execution machinery; external ecosystem runtimes (Pandoc binary, Node, mdBook preprocessor binaries) as optional runtime deps |

---

## 1. Overview

SPEC-032 established a three-stage render-hook system (`pre-parse` / `transform` / `post-render`) with first-class file selection, a versioned AST JSON schema (`zetl-ext`), and a reserved `ast_type` manifest field as forward-compatibility machinery. What SPEC-032 deliberately left for a successor is the strategic question of **what plugin ecosystems zetl actually integrates with**. This spec answers that question.

Zetl's goal is to **utilise existing plugin ecosystems, not to build its own**. Writing our own Callouts, Tasks, Admonition extensions as first-party Python hooks (the plan in SPEC-032 §§ REQ-3212, ADR-3204) duplicates work that mature ecosystems already solve better. The empirical case for this is strong: a 2026-04-19 scan of seven plugin ecosystems (cached at `tools/ecosystem-scan.md`) found that (a) Pandoc filters alone cover 63% of PKM-relevant categories (citations, crossrefs, transclusion, tables, math) in their top-30, (b) mdBook preprocessors have the best maintenance signal of any ecosystem (90% active in top-30) and architecturally match SPEC-032's JSON-stdio protocol exactly, and (c) remark is the largest JS ecosystem (539 plugins) with deep coverage of the web-docs use case. Writing our own extensions against those ecosystems' gaps is far cheaper than rebuilding their functional core.

This spec introduces an **ecosystem adapter** as a first-class concept: a declarative bridge between zetl's hook runtime and one of several supported plugin ecosystems, each with its own AST format, invocation conventions, and dependency model. v1 ships three adapters (Pandoc, mdBook, remark); the architecture accommodates additional ecosystems (djot, Quarto, MkDocs, and others) as incremental additions rather than redesigns.

The `ast_type` field reserved in SPEC-032's manifest format becomes live. A hook declares the ecosystem it targets:

```toml
# .zetl/hooks/transform.d/crossref.toml
stage = "transform"
ecosystem = "pandoc"
exec = "pandoc-crossref"
```

Zetl translates zetl-ext ↔ the ecosystem's AST at the adapter boundary, emulates the ecosystem's invocation conventions (env vars, argv, protocol shape), dispatches the plugin, and converts the response back. Mixed-ecosystem pipelines (a pandoc filter, followed by an mdBook preprocessor, followed by a remark plugin) compose because zetl owns every translation.

### 1.0 Relationship to SPEC-032

SPEC-032 owns the **protocol surface** — manifest schema, hook stages,
selector evaluation, `ast_type` contract, round-trip invariants, marker
conventions, translator trait obligations (REQ-3221, CON-3221, ADR-3206).

SPEC-033 owns the **concrete ecosystem adapters** — which ecosystems
(Pandoc, mdBook, remark), how each one's runtime is probed, the
marker-table instances for each ast_type (REQ-3307, REQ-3308), and the
per-ecosystem matrix of tested plugins.

Where this spec would restate SPEC-032 protocol obligations, it
cross-references them instead. Adding a future ecosystem (djot, Quarto,
etc.) is an addition to SPEC-033 and a registry entry — SPEC-032's
protocol surface does not change.

### 1.1 Motivation

- **Leverage beats originality.** A ten-year-old Pandoc filter like `pandoc-crossref` has had more eyeballs, edge-case fixes, and production deployments than anything zetl could ship as a v1 canonical extension. The right strategy is to make `pandoc-crossref` work under `zetl build`, not to reimplement crossref logic.
- **Ecosystem coverage compounds.** Adopting Pandoc gets the academic/technical-writing audience (citations, crossrefs, bibliography). Adopting mdBook gets the docs-site audience (diagrams, math, TOC). Adopting remark gets the JS-tooling audience (MDX, GFM variants, math). Three adapters cover most of what zetl's users would want.
- **Architectural fit is real.** Two of three chosen ecosystems (Pandoc and mdBook) already use out-of-process JSON-over-stdio, which is exactly SPEC-032's native protocol. We're not contorting; we're already architecturally aligned. The third (remark) is in-process JS and requires a Node subprocess harness — real engineering, but bounded and well-understood.
- **Typed protocol scales.** The `ecosystem` / `ast_type` field means new ecosystems are incremental additions. Adding `djot` support in v2 doesn't require redesigning the protocol — just implementing a translator and adapter.
- **Users already have installed these.** Pandoc is on most academic/technical writers' machines. Rust developers have `cargo install`'d mdBook preprocessors. Web developers have a `node_modules` full of remark plugins. Zetl consuming those existing installations respects the user's context instead of demanding they start over.

### 1.2 Design Principles

1. **Zetl owns the translations, not the plugins.** Every `ecosystem ↔ zetl-ext` translator is a zetl-maintained Rust module with fuzz-quality round-trip tests. We do not delegate translation fidelity to third-party adapter binaries or plugins themselves.
2. **Marker-convention discipline for lossy boundaries.** Zetl concepts that don't map natively to a foreign AST (wikilinks, embeds, SPL blocks) are represented via that ecosystem's extension mechanism (Pandoc `Span` with classes, mdast custom nodes, mdBook literal marker syntax). A plugin that strips markers breaks round-trip; zetl detects and warns.
3. **Emulate the ecosystem's invocation contract faithfully.** A Pandoc filter run under zetl sees `PANDOC_VERSION`, `PANDOC_API_VERSION`, argv[1] = output format, and pandoc-types on stdin. An mdBook preprocessor sees the `supports <renderer>` probe and `{context, book}` on stdin. A remark plugin is loaded via `require()` in a Node harness. Ecosystem-native plugins behave as they would in their native host.
4. **Optional-runtime model.** Each ecosystem has an optional runtime dep (Pandoc binary; Node + npm; `cargo install`'d preprocessors). Zetl detects at startup, reports what's available, disables adapters with missing deps, and provides actionable installation hints. Zetl does not bundle any ecosystem runtime.
5. **One internal AST, many external views.** Zetl's canonical internal representation stays `zetl-ext` (as in SPEC-032). Ecosystem-specific formats (pandoc-types, mdast, mdBook Book) are views at the hook boundary. The core engine doesn't know about Pandoc or remark.
6. **Extensible registry.** Adding a new ecosystem is adding an adapter module + a translator + test coverage. Not redesigning protocols.
7. **No ecosystem bundling.** Zetl is not a Pandoc redistribution, nor an mdBook fork, nor a remark wrapper. Users install the ecosystem's own tooling; zetl orchestrates.

### 1.3 Scope

**In scope (v1):**

- Three first-class ecosystem adapters: **Pandoc filters**, **mdBook preprocessors**, **remark plugins**.
- Per-ecosystem translators: `zetl-ext ↔ pandoc-types`, `zetl-ext ↔ mdast`, `zetl-ext ↔ mdBook Book envelope` (the last is more "protocol wrapper" than "AST translator" since mdBook operates on raw Markdown strings).
- A **parser registry** with per-ecosystem parser adapters: Pandoc (via subprocess to installed `pandoc` binary) for pandoc-ext hooks, pulldown-cmark (default) for zetl-ext hooks. Parser selection is per-page (frontmatter or directory-pattern). Adds the foundation for further parser additions (djot, pandoc.wasm) without protocol changes.
- Ecosystem runtime detection at zetl startup; actionable diagnostics when an adapter's runtime is missing.
- Plugin discovery per ecosystem: `.zetl/hooks/transform.d/<name>.toml` manifests with `ecosystem = "pandoc" | "mdbook" | "remark"`; zetl resolves the executable or package per ecosystem conventions.
- Persistent-mode (Pandoc, mdBook) or harness-mode (remark) execution, reusing SPEC-032's budget and failure semantics.
- Translation contracts documented per ecosystem with "what round-trips / what's lossy" tables and golden-HTML fixtures against well-known plugins (pandoc-crossref, pandoc-citeproc, mdbook-mermaid, mdbook-toc, remark-gfm, remark-math).
- Compat matrix: `tools/zetl-ecosystem-matrix.toml` recording tested plugin versions per ecosystem at `supported` / `partial` / `experimental` tiers.
- Documentation: per-ecosystem authoring guides; migration notes for existing Pandoc-pipeline / mdBook / remark users; ecosystem runtime-install recipes.
- `zetl ecosystem check` subcommand — reports which ecosystem runtimes are detected, versions, and the set of adapters available this session.

**Out of scope (deferred or rejected):**

- Pandoc embedded as WASM / FFI — runtime `pandoc` binary subprocess is the v1 integration mechanism. WASM or FFI embedding is deferred to a successor if binary-dep friction becomes a real complaint.
- mdBook as a full build pipeline — zetl does not attempt to produce mdBook-compatible output. It runs mdBook preprocessors *as a convenience*, adapting zetl's vault into an mdBook `Book` envelope for the preprocessor and unpacking the result. No book-level output.
- remark as a parser (replacing pulldown-cmark) — v1 keeps remark as a *hook* runtime only; it does not parse zetl's Markdown. Users who want remark's parsing semantics can run `remark-parse` as a pre-parse-stage hook. Full parser-layer adoption is a v1.1 concern.
- Automatic npm install of remark plugins — zetl requires users to have already installed plugins in a local `node_modules` or globally. No package-management orchestration.
- `mdast-util-wiki-link` and similar plugins that require parser-level intervention — these work only when remark is the parser, not just a hook runtime. Deferred to v1.1 alongside parser-layer remark adoption.
- markdown-it, markdown-it-py, MkDocs, Quarto adapters in v1. On the basis of the ecosystem scan, these are either covered transitively (Quarto via Pandoc), runner-up by coverage (markdown-it overlaps heavily with remark), or orthogonal to PKM workflows (MkDocs mostly does site-generation, not AST transforms). Each is a candidate for a later release if demand emerges.
- Installation assistance beyond diagnostics — zetl prints actionable hints ("install pandoc: `brew install pandoc`") but does not run the installer.

---

## 1.4 Prior Art and Empirical Basis

Two artefacts sit under this spec:

- **`tools/parser-lit-survey.md`** (2026-04-19) — literature survey of unified/remark, Pandoc filters and Lua filters, mdBook preprocessors, markdown-it, plus an academic-systems narrative (OMeta, Racket #lang, SweetJS). Underpins SPEC-032's design; reused here for ecosystem-specific invocation conventions.
- **`tools/ecosystem-scan.md`** (2026-04-19) — empirical ranking of seven candidate ecosystems by plugin count, maintenance signal, and PKM-relevance hit rate. Drives this spec's v1 target selection. Key findings:
  - Pandoc filters dominate citations (3/30), crossrefs (2/30), and transclusion (5/30) — the three categories correlating most strongly with PKM/academic-writing workloads. 19/30 top plugins are PKM-relevant (63%), the highest measured.
  - mdBook preprocessors have the healthiest maintenance profile (27/30 active, 0 stale, 3 abandoned) and the strongest diagram coverage (4/30).
  - remark is the largest JS ecosystem (539 plugins) and covers the web-docs audience transitively through rehype and retext composition.
  - markdown-it dropped from the prior top-3 because its top-30 maintenance signal is the weakest measured (25/30 flagged abandoned by the 2-year threshold) and its category coverage largely duplicates remark's.
  - Obsidian was excluded (separate prior scan at `tools/obsidian-top50-scan.md` showed the top-50 is 0% pure-MPP and 76% unusable in a bounded shim).

Both artefacts should be cited when SPEC-033 undergoes stakeholder review — they are the empirical ground under this spec's v1 selections.

---

## 2. User Profiles and Happy Paths

### 2.1 Academic / technical writer using Pandoc filters

**Role:** Researcher maintaining a zetl vault of papers-in-progress with `[[wikilink]]` cross-references. Uses `pandoc-crossref` for numbered figures/sections, `pandoc-citeproc` for bibliography, `pantable` for complex tables.
**Goal:** Publish the vault as a static site (`zetl build`) with all Pandoc-filter transformations applied, without leaving zetl.
**Constraints:** Pandoc already installed (`brew install pandoc`). Has a `references.bib`. Does not want to learn a new extension API.

**Happy path:**

1. User runs `zetl ecosystem check` → sees `pandoc: detected (3.1.12.1); pandoc-types: 1.23`.
2. Writes `.zetl/hooks/transform.d/crossref.toml`:
   ```toml
   stage = "transform"
   ecosystem = "pandoc"
   exec = "pandoc-crossref"
   mode = "persistent"
   ```
3. Same pattern for `citeproc.toml` (ecosystem="pandoc", exec="pandoc-citeproc", with `args = ["--bibliography=references.bib"]`).
4. `zetl build` runs; on each page, zetl parses with Pandoc (because `transform` stage has `ecosystem="pandoc"` hooks and the page's frontmatter says `parser: pandoc`), emits pandoc-types AST, invokes `pandoc-crossref`, then `pandoc-citeproc`, renders to HTML.
5. Wikilinks in the source are preserved across Pandoc-filter invocations via marker conventions (`Span class="zetl-wikilink"` + attrs); pandoc-citeproc's `[@key]` citations render to Chicago-style footnotes.
6. Diagnostic report: `[zetl] ecosystem pandoc: 3 filters loaded, 247 page invocations, 0 round-trip warnings`.

**Failure modes:**

- User has `ecosystem="pandoc"` hook but no pandoc binary → `zetl ecosystem check` fails with hint: `"install pandoc 2.11+ — brew install pandoc / apt install pandoc"`. Hook disabled; build continues with plain output.
- `pandoc-crossref` version incompatible with pandoc binary version → Pandoc's own error surfaces through stderr; zetl forwards it; plugin disabled.
- A plugin strips wikilink markers silently → zetl detects via round-trip compare, logs warning, user sees `"pandoc-crossref dropped 4 wikilink markers on projects/q2-review.md"` in the coverage report.

### 2.2 Docs-site author using mdBook preprocessors

**Role:** Developer maintaining technical documentation. Wants mermaid diagrams, KaTeX math, auto-TOC, and the `mdbook-admonish` callout syntax — all of which exist as mdBook preprocessors.
**Goal:** Use these exact preprocessors under `zetl serve` and `zetl build`.
**Constraints:** Has `cargo install mdbook-mermaid mdbook-toc mdbook-katex mdbook-admonish` already. Doesn't want to run the full mdBook toolchain; just wants its preprocessors.

**Happy path:**

1. `zetl ecosystem check` → `mdbook: detected (preprocessors available: mdbook-mermaid, mdbook-toc, mdbook-katex, mdbook-admonish); mdBook binary not found (only needed for full-book pipeline; preprocessors run independently)`.
2. Writes manifests:
   ```toml
   # .zetl/hooks/transform.d/mermaid.toml
   stage = "pre-parse"
   ecosystem = "mdbook"
   exec = "mdbook-mermaid"
   ```
   (mdBook preprocessors operate on raw Markdown text in chapter content, so they fit zetl's `pre-parse` stage not `transform`.)
3. Builds. Zetl wraps each page's Markdown in a synthetic mdBook `Book` envelope (one chapter = one zetl page), invokes the preprocessor, unwraps the result, hands the transformed Markdown to the parser.
4. mermaid code blocks render as inline SVG; math renders as KaTeX HTML; admonitions become callout divs.

**Failure modes:**

- Preprocessor installed but incompatible with synthetic book envelope (expects real mdBook book.toml) → adapter provides a minimal book.toml stub; if preprocessor requires fields we don't supply, documented in the compat matrix as `partial`, with the specific broken feature noted.

### 2.3 Web / docs author using remark plugins

**Role:** MDX / Astro / Docusaurus user migrating content into zetl. Has 15 remark plugins they rely on (`remark-gfm`, `remark-math`, `remark-directive`, `remark-mdx`, `remark-smartypants`, etc.).
**Goal:** Those plugins should run under `zetl build` without requiring a full JS build pipeline.
**Constraints:** Has Node 18+ installed. Happy to `npm install` plugins into a project-local `node_modules`.

**Happy path:**

1. `zetl ecosystem check` → `remark: detected (node v20.10.0); harness at _static/zetl-remark-harness.mjs; resolvable plugins from ./node_modules`.
2. Installs plugins: `npm install --save-dev remark-gfm remark-math remark-directive`.
3. Writes manifests:
   ```toml
   # .zetl/hooks/transform.d/gfm.toml
   stage = "transform"
   ecosystem = "remark"
   package = "remark-gfm"
   options = {}
   ```
4. `zetl build` starts a persistent Node subprocess with zetl's embedded remark harness. The harness loads plugins via `require()`, receives mdast JSON per page, applies plugins, returns mdast.
5. Zetl translates zetl-ext → mdast at the boundary, invokes the harness, translates back.

**Failure modes:**

- Plugin requires a specific remark/unified major incompatible with the harness's pinned version → adapter reports a typed mismatch error, hook disabled, build continues.
- Plugin throws on a specific page → isolated failure per SPEC-032 REQ-3207, page fragment reverts, diagnostic logged.

### 2.4 Contributor adding a new ecosystem (`djot`)

**Role:** zetl contributor wants to add djot support because John MacFarlane's djot parser has features they want (explicit attr syntax, no HTML-passthrough ambiguity).
**Goal:** Ship a `ecosystem = "djot"` adapter that parses djot source and runs djot-ecosystem transforms.
**Constraints:** Must match the adapter trait defined here; pass translator round-trip tests; add matrix entries.

**Happy path:**

1. Implements `src/ecosystems/djot/adapter.rs` satisfying `EcosystemAdapter` trait (CON-3302).
2. Implements `src/ecosystems/djot/translator.rs` with `zetl_ext_to_djot` / `djot_to_zetl_ext` fns.
3. Adds property-test roundtrip suite; implements parser invocation.
4. Adds `zetl-ecosystem-matrix.toml` entry with test fixtures.
5. PR passes CI; ships in next release.

---

## 3. Functional Requirements

### REQ-3301: Ecosystem Registry

The system SHALL maintain a registry of supported plugin ecosystems, each identified by a stable string identifier (`"pandoc"`, `"mdbook"`, `"remark"` in v1). The registry SHALL record, per ecosystem: the adapter module, the translator module, the required external runtime (binary name and minimum version), the supported AST formats and versions, and the ecosystem-specific manifest schema.

Adding a new ecosystem SHALL require only:
- A new adapter module in `src/ecosystems/<name>/`.
- A new translator module with bidirectional conversion to/from zetl-ext.
- A registry entry.
- A compat-matrix section.
- Test coverage (round-trip property tests + golden-HTML against known plugins).

Changes to core protocols (SPEC-032's hook-execution machinery) SHALL NOT be required for new ecosystem additions.

Trace:
- TEST-3301
- CON-3301

### REQ-3302: Ecosystem Adapter Contract

Every ecosystem adapter SHALL implement a Rust trait `EcosystemAdapter` (CON-3302) exposing:

- `probe() -> RuntimeStatus` — detect the ecosystem's runtime (e.g., `pandoc --version`, `node --version`, presence of `cargo install`'d preprocessors on PATH).
- `translate_to_foreign(ast: ZetlExt) -> ForeignAST` — serialise zetl-ext into the ecosystem's AST shape.
- `translate_from_foreign(ast: ForeignAST) -> ZetlExt` — deserialise back to zetl-ext.
- `invoke_plugin(manifest, input, context) -> PluginResponse` — apply the invocation conventions of the ecosystem (env vars, argv, protocol shape) and return the result.
- `supported_stages() -> Vec<HookStage>` — which SPEC-032 stages this ecosystem's plugins can participate in (Pandoc: transform; mdBook: pre-parse; remark: transform).

The trait SHALL be implemented in the zetl binary, not loaded dynamically. Each adapter is compiled into zetl (feature-flagged per ecosystem to manage binary size).

Trace:
- TEST-3302
- CON-3302

### REQ-3303: Pandoc Adapter (v1)

The system SHALL ship a Pandoc adapter under the `--features
ecosystem-pandoc` cargo flag (on by default). The adapter SHALL
support two invocation modes, declared per manifest via `mode =
"filter" | "native"`:

**`mode = "filter"` (default)** — external filter plugins (the
"pandoc-*" binary convention). The adapter:

- **Probes** the `pandoc` binary via `pandoc --version`; parses out
  the binary version AND the pandoc-types API version (via
  `pandoc --from markdown --to json <<< "" | jq .pandoc-api-version`
  at first invocation, cached for the session).
- **Invokes plugins** by spawning the plugin executable with
  argv[1] = output format (`"html"` by default), setting env vars
  `PANDOC_VERSION`, `PANDOC_API_VERSION`, `PANDOC_READER_OPTIONS`,
  `PANDOC_WRITER_OPTIONS`, `PANDOC_SCRIPT_FILE`, feeding pandoc-types
  JSON on stdin, reading pandoc-types JSON on stdout.
- **Translates** zetl-ext to pandoc-types with marker conventions for
  zetl concepts (REQ-3307).
- **Persistent mode** SHALL be supported for plugins that implement it
  (detected via probe); fallback is one-shot invocation per page.
- **Runs in the `transform` stage** by default. `pre-parse` is not
  supported for Pandoc filters (Pandoc filters operate on AST, not
  raw Markdown).

**`mode = "native"`** — Pandoc-native invocation modes (`--citeproc`,
`--lua-filter`, `--from markdown --to html` with flags). The adapter:

- Invokes the installed `pandoc` binary directly with flags from the
  manifest `args` field (e.g. `args = ["--citeproc",
  "--bibliography=references.bib"]`) instead of piping JSON through
  a filter executable.
- Source is the raw page Markdown; output is Pandoc's HTML. Runs at
  `pre-parse` OR `transform`-after-translation, depending on manifest
  stage — `pre-parse` replaces page content with Pandoc's HTML before
  zetl's own rendering (the default when `mode = "native"`), which
  lets the user delegate the entire pipeline to Pandoc for that page.
- Existed because Pandoc's modern citation support is `--citeproc`
  (a Pandoc-native flag), not a filter. `pandoc-citeproc` was
  deprecated after Pandoc 2.11 and removed from later Pandoc
  releases; native mode is the correct path for citations. Lua
  filters (`--lua-filter=file.lua`) also live here.

Compat matrix SHALL include golden-HTML fixtures for at least, in
filter mode: `pandoc-crossref`, `pantable`, `pandoc-include-code`,
`pandoc-plantuml`; in native mode: `--citeproc` with CSL-JSON and a
BibTeX bibliography.

Trace:
- TEST-3303 (filter mode), TEST-3303n (native mode)
- CON-3303
- ADR-3302

### REQ-3304: mdBook Preprocessor Adapter (v1)

The system SHALL ship an mdBook adapter under `--features ecosystem-mdbook` (on by default). The adapter SHALL:

- **Probe** discovers `cargo install`'d preprocessors by scanning `$CARGO_HOME/bin/` and `$PATH` for `mdbook-*` executables and invoking each with `supports html` per mdBook's probe convention (SPEC-032 REQ-3216 style, matching mdBook's own).
- **Invoke plugins** by constructing a synthetic mdBook `Book` envelope containing the current page (or pages; see below) as a single-chapter book, serialising `[Context, Book]` to JSON on stdin, reading transformed `Book` on stdout, extracting the transformed chapter content.
- **Stage**: mdBook preprocessors operate on raw Markdown (chapter `.content`), so the adapter runs them at zetl's `pre-parse` stage, not `transform`.
- **Per-page vs per-vault invocation**: v1 invokes the preprocessor per page (one chapter per invocation) for determinism and parallelism; some preprocessors (e.g., `mdbook-toc` generating a whole-book TOC) expect the full book. Whole-vault invocation is a configurable manifest option (`scope = "vault"`) at the cost of losing page-level parallelism.
- **Synthetic book.toml**: adapter provides a minimal book.toml stub; preprocessors that require specific fields missing from the stub are flagged `partial` in the matrix.

Compat matrix SHALL include golden fixtures for at least: `mdbook-mermaid`, `mdbook-toc`, `mdbook-katex`, `mdbook-admonish`.

Trace:
- TEST-3304
- CON-3304
- ADR-3303

### REQ-3305: remark Adapter (v1)

The system SHALL ship a remark adapter under `--features ecosystem-remark` (on by default). The adapter SHALL:

- **Probe** detects `node` (Node.js 18+ required) and locates a project-local `node_modules` (first ancestor of the vault root containing one) or a user-configured plugin-resolution root.
- **Persistent harness**: zetl starts a single Node subprocess on first use, loading the `zetl-remark-harness.mjs` script (bundled with zetl under `_static/`), which imports `unified`, and exposes a JSON-RPC-like protocol on stdin/stdout for loading plugins by name, applying them to mdast documents, and unloading.
- **Invoke plugins** by name + options, e.g., `{"type": "invoke_plugin", "name": "remark-gfm", "options": {}, "ast": <mdast>}`. The harness applies the plugin and returns the transformed mdast.
- **Translate** zetl-ext ↔ mdast with marker conventions for zetl concepts (REQ-3308). mdast is closer to zetl-ext than pandoc-types is (both are CommonMark-derived), so translation loss is lower.
- **Stage**: `transform` only. Plugins that require parser-level intervention (`mdast-util-*`) are not supported in v1 — see §1.3 out-of-scope.
- **Isolation**: a manifest field `isolation = "shared" | "fresh-context"` (default `"shared"`) controls whether plugins share the long-lived harness's module cache. `"shared"` is the perf-default: plugins load once and reuse the cache across pages. `"fresh-context"` spawns a new Node subprocess per invocation, isolating plugins from each other at the cost of per-invocation startup (≈ 100–200 ms). Paranoid users who worry about plugin-A monkey-patching globals that plugin-B later relies on should pick `fresh-context`; the v1 default is `shared` on the basis that ecosystem plugins are expected to play nicely.

Compat matrix SHALL include golden fixtures for at least: `remark-gfm`, `remark-math`, `remark-directive`, `remark-frontmatter`, `remark-smartypants`.

Trace:
- TEST-3305
- CON-3305
- ADR-3304

### REQ-3306: Parser Registry and Per-Page Parser Selection

The system SHALL support multiple Markdown parsers via a parser registry, each identified by a stable name:

- `"commonmark"` (default) — pulldown-cmark with zetl's extensions (wikilinks, embeds, SPL blocks).
- `"pandoc"` — invokes `pandoc --from markdown --to json <page>` via the Pandoc adapter to produce pandoc-types; then converted to zetl-ext for internal pipeline.

Parser selection precedence (lowest-to-highest priority):

1. **Zetl default**: `commonmark`.
2. **Vault default**: `.zetl/config.toml` `[parse] default = "pandoc"`.
3. **Directory default**: globbed rules in `.zetl/config.toml` `[[parse.rule]]` — e.g., `pattern = "papers/**" parser = "pandoc"`.
4. **Per-page frontmatter**: `parser: pandoc`.

When a `transform`-stage hook has `ecosystem = "pandoc"`, the page's parser SHOULD be `"pandoc"` for best fidelity; otherwise, zetl translates from commonmark-zetl-ext to pandoc-types at the adapter boundary (lossy for pandoc-specific syntax not recognised by pulldown-cmark, but functional).

Trace:
- TEST-3306
- CON-3306
- ADR-3301

### REQ-3307: zetl-ext ↔ pandoc-types Translation — Marker Conventions

The general translation protocol (round-trip invariant, marker-strip
detection, version-range semantics, mixed-pipeline composition) is defined
by **SPEC-032 REQ-3221 + CON-3221** — this requirement is SPEC-033's
instance-specific marker table for the `pandoc-ext` ast_type.

Marker conventions for zetl concepts without native pandoc-types
equivalents:

- **Wikilinks** (zetl's `Wikilink` node) ↔ Pandoc `Span` with class `"zetl-wikilink"` and attrs `{target, alias, heading, block_id}`.
- **Embeds** (zetl's `Embed` node) ↔ Pandoc `Span` with class `"zetl-embed"` and attrs `{target, heading, block_id}`.
- **SPL blocks** — `CodeBlock` with language `"spl"` in both directions.
- **Frontmatter** — zetl's `FrontMatter` node ↔ Pandoc's `Meta` map at the document root.
- **Source positions** — preserved in a `sourcepos` attribute on enclosing Pandoc nodes (Pandoc has no native position concept).

The round-trip property (SPEC-032 CON-3221 invariant 1) and marker-strip
detection (SPEC-032 CON-3221 invariant 3) apply unchanged; this section
only lists the *content* of the marker table for pandoc-ext. The
auto-generated full node-type mapping lives at
`docs/ecosystems/pandoc-translation.md` (SPEC-032 CON-3207 contract
wording; CON-3307 here records the path).

Trace:
- TEST-3307
- CON-3307
- SPEC-032 REQ-3221, CON-3221

### REQ-3308: zetl-ext ↔ mdast Translation — Marker Conventions

General translation protocol as for REQ-3307 (SPEC-032 REQ-3221 + CON-3221).
Instance-specific marker table for the `mdast-ext` ast_type:

- **Wikilinks** ↔ mdast custom node `{type: "wikilink", target, alias, heading, block_id}` (the widely-adopted mdast convention from `remark-wiki-link`).
- **Embeds** ↔ mdast custom node `{type: "embed", ...}`.
- **SPL blocks** — `code` node with `lang: "spl"`.
- **Frontmatter** — mdast `yaml` node (the `remark-frontmatter` convention).
- **Position** — mdast has native `position` objects; direct mapping.

Because mdast is CommonMark-aligned, translation loss is lower than with
pandoc-types. mdast node types zetl doesn't natively represent (e.g.,
`definition`, `linkReference`) are preserved as opaque pass-through nodes
via zetl-ext's forward-compat mechanism (the AST schema accepts unknown
node types with a warning). Full mapping at
`docs/ecosystems/mdast-translation.md`.

Trace:
- TEST-3308
- CON-3308
- SPEC-032 REQ-3221, CON-3221

### REQ-3309: mdBook Book Envelope

When invoking an mdBook preprocessor, the system SHALL construct a synthetic mdBook `Book` envelope:

```json
{
  "root": "/path/to/vault",
  "config": {
    "book": {"title": "<vault.name>", "authors": [], "src": "."},
    "preprocessor": {}
  },
  "renderer": "html",
  "mdbook_version": "<adapter-declared version>",
  "book": {
    "sections": [
      {"Chapter": {
        "name": "<page.title>",
        "content": "<raw Markdown>",
        "number": null,
        "sub_items": [],
        "path": "<page.slug>.md",
        "source_path": "<page.slug>.md",
        "parent_names": []
      }}
    ],
    "__non_exhaustive": null
  }
}
```

Adapter SHALL declare this envelope shape in `docs/ecosystems/mdbook.md`. Known preprocessors incompatible with the per-page envelope are flagged in the matrix with the reason.

Trace:
- TEST-3309
- CON-3309

### REQ-3310: `zetl ecosystem check` Subcommand

The system SHALL provide `zetl ecosystem check [--json]` that:

- Invokes each registered adapter's `probe()` method.
- Reports per ecosystem: runtime status (`detected` / `missing` / `wrong-version`), detected version, actionable install hint when missing.
- Lists available plugins per ecosystem (for mdBook: scanning `$PATH` for `mdbook-*`; for Pandoc: listing filters configured in user manifests; for remark: resolving packages from `node_modules`).
- Exit code 0 if all *configured* ecosystems are available; non-zero if any configured-but-missing.

`--json` emits structured output suitable for CI pre-flight checks.

Trace:
- TEST-3310
- CON-3310

### REQ-3311: Ecosystem Matrix

The system SHALL ship `tools/zetl-ecosystem-matrix.toml` recording, per
tested (ecosystem, plugin, version) triple: tier (`supported`,
`partial`, `experimental`), `version_range` (REQ-3314), golden-fixture
paths, known limitations, test matrix status, and a `contract`
sub-table carrying the plugin's declared behavioural properties per
SPEC-032 REQ-3224.

Example entry:

```toml
[[plugin]]
ecosystem = "pandoc"
name = "pandoc-crossref"
version_range = ">=0.3 <0.4"
tier = "supported"
fixture = "tests/ecosystem-fixtures/pandoc-crossref/"
[plugin.contract]
preserves = ["Wikilink", "Embed", "SPL", "Link", "Image"]
idempotent = true       # verified by TEST-3224-idempotent
pure = true             # advisory in v1 (Tier-2 per SPEC-032 REQ-3224)
```

When a user's hook manifest does not declare a `[contract]` table,
zetl populates the runtime contract from the matching matrix entry
(CON-3224 "Ecosystem-adapter provenance"). A manifest-declared
contract always overrides the matrix — useful when a user patches a
plugin or invokes it with contract-altering flags.

Unknown plugins (not in the matrix) produce a warning at first use:
`[zetl] ecosystem <eco>: <plugin> not in matrix; behavioural contract
unknown, no preservation checks active`. Users wanting safety signals
SHOULD contribute a matrix entry or declare `[contract]` in their own
manifest.

CI SHALL gate merges that downgrade tier without accompanying
rationale (same pattern as SPEC-032 REQ-3213). Contract-field changes
(e.g. dropping `preserves = ["Wikilink"]` from a supported plugin's
declaration) are treated as tier downgrades and gated identically.

Trace:
- TEST-3311
- SPEC-032 REQ-3224, CON-3224

### REQ-3312: Ecosystem-Specific Manifest Fields

Beyond the SPEC-032 base manifest fields (`stage`, `mode`, `timeout_ms`, `ast_type`, `select.*`, `before`, `after`, `optional`, `extension_id`), ecosystem adapters MAY extend the manifest schema with ecosystem-specific fields:

- **Pandoc**: `exec = "<filter-binary>"`, `args = ["..."]`, `lua_filter = "<path>"` (either `exec` or `lua_filter`, not both).
- **mdBook**: `exec = "mdbook-<name>"`, `scope = "page" | "vault"` (default `"page"`).
- **remark**: `package = "remark-<name>"`, `version = "<semver>"`, `options = {...}`.

Manifest parsing SHALL validate that ecosystem-specific fields are only used with the declared ecosystem (a `package = ...` field on a `pandoc` manifest is a parse error).

Trace:
- TEST-3312
- CON-3312

### REQ-3313: Runtime Detection and Graceful Absence

At zetl startup, before any hook pipeline construction, the system SHALL:

- Invoke every compiled-in ecosystem adapter's `probe()`.
- Log detection results (`[zetl] ecosystem pandoc: detected v3.1.12.1 (pandoc-types 1.23)`).
- Identify which configured hooks (in `.zetl/hooks/` and theme-bundled hooks) target unavailable ecosystems; disable them with typed `ecosystem_runtime_missing` errors and actionable install hints.
- Continue with all other pipeline work.

Missing runtime SHALL NOT be a hard failure of `zetl build` or `zetl serve` unless `--ecosystem-required=<name>` is passed (CI gate mode).

Trace:
- TEST-3313

### REQ-3314: Plugin-Version Drift Detection

Matrix entries (REQ-3311) pin tested plugin versions, but the user's
installed binary may differ. The system SHALL:

- At ecosystem probe time, invoke each configured plugin's
  `--version` (Pandoc filters and mdBook preprocessors) or
  `package.json` version field (remark plugins) and record the
  observed version alongside the matrix-pinned tested version.
- Classify the observed version relative to a `version_range` field
  on the matrix entry (new matrix column):
  - **Exact match** — silent.
  - **Minor drift** (same major, observed minor ≥ tested minor) —
    log `[zetl] ecosystem <eco>: <plugin> v<observed> is newer than
    last-tested v<tested>; proceeding` once per session, hook runs.
  - **Incompatible** (different major, or observed lower than
    tested range) — hook disabled with typed
    `plugin_version_incompatible` error; actionable hint pointing at
    the matrix entry's version range.
- The matrix SHALL therefore carry a `version_range` column declaring
  the acceptable range per plugin (npm-style semver range syntax).

Trace:
- TEST-3314
- CON-3311 amended (version_range column added)

### REQ-3315: Mixed-Parser Diagnostic

REQ-3306 lets different pages declare different parsers (`commonmark`
vs `pandoc`). Syntax that is valid under both but means different
things (curly-brace attribute syntax `{.class}`, extended table
dialects, fenced-div shortcuts) can render inconsistently across the
vault without any error surfacing.

The system SHALL, on `zetl build`, scan every page's raw Markdown for
a defined set of **parser-ambiguous syntax patterns** and, for each
page, record the parser it was processed under. If the vault contains
any page matching an ambiguous pattern AND the vault has pages using
both parsers, zetl SHALL emit a warning listing:

- The ambiguous patterns detected and their file:line locations.
- The parser each page was processed under.
- A recommendation to unify on one parser, or to inspect the flagged
  pages for intended behaviour.

Under `zetl build --strict-parsers`, the warning becomes a fatal
error (CI gate for mixed-parser vaults).

The pattern set ships as `tools/zetl-parser-ambiguity.toml` and is
release-pinned; new patterns are additive per minor release.

Trace:
- TEST-3315

---

## 4. Non-Functional Requirements

### NFR-3301: Cold-Start Cost per Ecosystem

Activating a single ecosystem adapter (probing, loading persistent subprocess where applicable) SHALL complete in ≤ 200 ms at P95 for each ecosystem, measured from `zetl` invocation to readiness for first page invocation.

- Pandoc: `pandoc --version` subprocess; cache result session-scoped.
- mdBook: `mdbook-* supports html` probes for each detected preprocessor; parallelise.
- remark: starting Node subprocess + loading harness + `import unified` + pre-resolving `node_modules` paths.

Trace:
- TEST-3301-perf

### NFR-3302: Per-Page Invocation Latency — Pandoc and mdBook

In persistent mode, per-page invocation overhead (serialise → write → read → deserialise, excluding plugin's own processing) SHALL be ≤ 15 ms P95 for pages with ≤ 500 AST nodes. Inherits from SPEC-032 NFR-3207; tightened here to reflect that persistent Pandoc/mdBook plugins amortise the transport cost.

Trace:
- TEST-3302-perf

### NFR-3303: Per-Page Invocation Latency — remark (Node Harness)

The in-process remark harness SHALL round-trip a single-plugin invocation on a 500-node mdast in ≤ 30 ms P95. Higher than NFR-3302 because JSON marshalling across the Node boundary is more expensive than OS-pipe serialisation.

Trace:
- TEST-3303-perf

### NFR-3304: Binary Size per Ecosystem Feature

Each ecosystem feature flag SHALL add ≤ 2 MiB to zetl's release binary size. Features are independently toggleable; `cargo build --no-default-features --features="ecosystem-pandoc"` SHALL compile and produce a working zetl with only Pandoc ecosystem support.

Trace:
- TEST-3304-size

### NFR-3305: Translation Round-Trip Fidelity — Canonical-Form Equivalence

Strict byte-level AST equality across a round-trip through a foreign
AST is not achievable in general: foreign ASTs (pandoc-types
especially) admit multiple semantically-equivalent representations of
the same content (e.g., `Span [] [Str "x"]` vs `[Str "x"]` in inline
sequences). The NFR therefore uses **canonical-form equivalence**, not
byte-identity.

For every (ecosystem, ast_version) pair in the matrix, the round-trip
property is:

```
∀ A ∈ zetl-ext:  canonicalise(foreign_to_zetl(zetl_to_foreign(A))) == canonicalise(A)
```

where `canonicalise(·)` is a zetl-owned normaliser that collapses
representation-level degrees of freedom (empty-attribute spans,
singleton-wrapper elimination, whitespace normalisation in `Text`
nodes), defined per ast_type at
`src/ecosystems/<type>/canonicalise.rs`.

Property tests run a QuickCheck-style generator producing 10,000
diverse zetl-ext ASTs per release; each must satisfy the canonical-
form equivalence. The generator itself is published at
`tools/zetl-ext-generator/` and pinned to the AST schema major.

**Semantic fidelity** (stronger statement): for the same 10,000-AST
corpus, rendering `A` and `foreign_to_zetl(zetl_to_foreign(A))` to
HTML via zetl's default renderer SHALL produce byte-identical HTML.
This is the user-visible invariant — two representations that
canonicalise to the same form must render identically.

Trace:
- TEST-3305-fidelity

### NFR-3306: Determinism — Adapter-Layer Scope

Zetl's adapter and translation layers SHALL introduce no
non-determinism: the same (vault, zetl version, ecosystem matrix
snapshot) SHALL produce byte-identical HTML across runs and
platforms on the adapter side.

**Plugin-internal non-determinism is outside zetl's scope.** Known
sources that zetl does not and cannot control:

- Pandoc filters that sort citations by a locale-dependent collation
  (CSL style dependent).
- remark plugins that iterate over a native `Set` or `Map` whose
  iteration order depends on insertion order (usually deterministic,
  but not guaranteed across Node major versions).
- mdBook preprocessors that generate SVG element IDs via a
  random/timestamped seed (e.g., `mdbook-mermaid`'s SVG output may
  differ per-invocation under some versions).
- Filesystem-order-dependent behaviour in any of the above when
  scanning `node_modules` or theme directories.

Users needing strict end-to-end determinism SHOULD:

1. Pin plugin versions in the matrix (REQ-3311).
2. Audit each plugin in the matrix for determinism properties and
   record findings in the matrix entry.
3. Where a plugin is known non-deterministic, wrap its output in a
   post-process normaliser (e.g., rewrite SVG IDs to content-hash
   form) as a separate hook.

SPEC-032 NFR-3203 (determinism of the native pipeline) applies
unchanged to the `zetl-ext` path.

Trace:
- TEST-3306-determinism

### NFR-3307: Ecosystem Runtime Process-Lifecycle Bounds

- Pandoc persistent filters: ≤ 50 MiB resident per filter; respawn on memory ceiling.
- mdBook preprocessors: one-shot per invocation by default; opt-in persistent via manifest.
- remark Node harness: one harness per zetl process; respawn on OOM; ≤ 256 MiB default ceiling.

Trace:
- TEST-3307-memory
- OBS-3303

### NFR-3308: Combined-Ecosystem Budget

The per-ecosystem NFRs (3301–3303) specify costs in isolation.
Real users will enable multiple ecosystems on the same vault. With
all three v1 ecosystems enabled on the 2,000-page demo vault (at
least one active plugin per ecosystem, persistent mode where
supported), total `zetl build` wall time SHALL increase by **≤ 3×**
relative to a `--no-ecosystems` baseline.

Honest ceiling, not aspirational: three serialised subprocess
pipelines per page, one Node harness round-trip, plus the zetl-ext
translation layers at each boundary. The 3× bound is a
test-enforced upper limit, not a target — individual enabled-set
configurations should measure below it.

Trace:
- TEST-3308-perf

---

## 5. Contracts

### CON-3301: Ecosystem Registry Data

The registry lives at `src/ecosystems/registry.rs` as a static table:

```rust
pub struct EcosystemEntry {
    pub id: &'static str,                      // "pandoc" | "mdbook" | "remark"
    pub runtime_dep: RuntimeDep,               // binary name, min version
    pub adapter_ctor: fn() -> Box<dyn EcosystemAdapter>,
    pub supported_stages: &'static [HookStage],
    pub supported_ast_types: &'static [&'static str],  // "pandoc-ext", "mdast-ext", etc.
}
```

Adding an ecosystem is adding a const entry to this table plus the adapter module. No runtime registration.

Implements: REQ-3301.
Verified by: TEST-3301.

### CON-3302: `EcosystemAdapter` Trait

```rust
pub trait EcosystemAdapter: Send + Sync {
    fn probe(&mut self) -> RuntimeStatus;

    fn translate_to_foreign(
        &self,
        ast: &ZetlExtDocument,
    ) -> Result<ForeignAst, TranslationError>;

    fn translate_from_foreign(
        &self,
        ast: &ForeignAst,
    ) -> Result<ZetlExtDocument, TranslationError>;

    fn invoke_plugin(
        &mut self,
        manifest: &PluginManifest,
        input: StageInput,
        context: &HookContext,
    ) -> PluginResponse;

    fn supported_stages(&self) -> &[HookStage];
}
```

Where `ForeignAst` is an ecosystem-specific type (`PandocTypesDocument`, `MdastDocument`, `MdBookBook`); adapters define their own.

Implements: REQ-3302.
Verified by: TEST-3302.

### CON-3303: Pandoc Invocation Contract

Pandoc adapter's `invoke_plugin` behaviour per manifest:

```
env:
  PANDOC_VERSION = "3.1.12.1"               # detected
  PANDOC_API_VERSION = "1,23,1"             # detected
  PANDOC_READER_OPTIONS = "<json blob>"     # default (or user-provided)
  PANDOC_WRITER_OPTIONS = "<json blob>"     # default (for html)
  PANDOC_SCRIPT_FILE = "<abs path to exec>"

argv:
  [exec, "html"]                            # target format
                                            # (or user-provided from manifest args)

stdin:  pandoc-types AST as JSON, UTF-8
stdout: pandoc-types AST as JSON, UTF-8
stderr: free-form; zetl forwards under --verbose

persistent mode: line-delimited JSON per CON-3201 (SPEC-032)
```

Identical to Pandoc's own filter contract ([filters.html](https://pandoc.org/filters.html)); filters cannot distinguish zetl-run from pandoc-run invocations.

Implements: REQ-3303.
Verified by: TEST-3303.

### CON-3304: mdBook Invocation Contract

```
argv:
  [exec, "supports", "html"]                # probe; exit 0 = supports
  [exec]                                    # real run

stdin:  [Context, Book] as JSON (mdBook convention)
stdout: Book as JSON
stderr: forwarded
exit 0 = success; non-zero = failure per REQ-3207 (SPEC-032)

persistent mode: NOT supported in v1 (mdBook preprocessors are spec'd as one-shot).
                 Reconsider in v1.1 if profiling warrants.
```

Implements: REQ-3304.
Verified by: TEST-3304.

### CON-3305: remark Harness Protocol

Harness JSON-RPC-like messages (line-delimited JSON over subprocess stdin/stdout):

```
zetl → harness:  {"id": 1, "type": "load_plugin", "package": "remark-gfm", "options": {}}
harness → zetl:  {"id": 1, "type": "load_result", "ok": true, "plugin_id": "rp_001"}

zetl → harness:  {"id": 2, "type": "apply", "plugin_id": "rp_001", "ast": {...mdast...}}
harness → zetl:  {"id": 2, "type": "apply_result", "ok": true, "ast": {...mdast...}}

zetl → harness:  {"id": 3, "type": "shutdown"}
harness exits cleanly
```

Harness source (embedded at `_static/zetl-remark-harness.mjs`) is published with zetl; users may supply their own if they need custom plugin resolution or caching.

Implements: REQ-3305.
Verified by: TEST-3305.

### CON-3306: Parser Selection Resolution

```
for page:
    parser_name = page.frontmatter.parser                    # priority 1
              ?? match_first_pattern(config.parse.rule, page.path)  # priority 2
              ?? config.parse.default                               # priority 3
              ?? "commonmark"                                       # priority 4 (zetl default)

    parser = parser_registry.get(parser_name)                # error if unknown
    ast = parser.parse(page.content)                         # parser-specific
    zetl_ext = parser.translate_to_zetl_ext(ast)             # canonical form
```

The zetl-ext AST is always the internal form during hook execution, regardless of parser. Ecosystem-specific AST formats appear only at the adapter boundary.

Implements: REQ-3306.
Verified by: TEST-3306.

### CON-3307, CON-3308: Translation Maps

Full node-type mapping tables live at `docs/ecosystems/pandoc-translation.md` and `docs/ecosystems/mdast-translation.md`. Each records the zetl-ext node type, the foreign type, the transformation rule, and a round-trip-fidelity note. Generated from the translator source at CI time so they stay in sync.

Implements: REQ-3307, REQ-3308.
Verified by: TEST-3307, TEST-3308.

### CON-3309: mdBook Book Envelope Schema

See REQ-3309 for the envelope shape. Key constraints:

- `mdbook_version` field MUST be a semver version string acceptable to the preprocessor's own compatibility check.
- `book.sections` is a single `Chapter` object per invocation in page scope; an ordered list of `Chapter` objects in vault scope.
- Paths are vault-relative; preprocessors expecting absolute paths are flagged `partial` in the matrix.

**Vault-scope invocations — known semantic gap.** When a manifest
declares `scope = "vault"`, zetl constructs a synthetic `Book` with
one `Chapter` per vault page (ordered by slug). The preprocessor
sees the full set, which is what preprocessors like `mdbook-toc`
need. However:

- Vault-scope invocations have no per-page `HookContext` (SPEC-032
  passes frontmatter, page_slug, and deadline per page). The
  preprocessor sees raw chapter content + the synthetic book's
  top-level metadata only.
- Per-page `build_data` writes (SPEC-032 REQ-3219) are not available
  from a vault-scope hook — the hook doesn't know which page it's
  writing against.
- Output from a vault-scope hook is spliced back into zetl's per-page
  pipeline by matching chapter `path` → page slug; preprocessors
  that rename chapters, drop them, or add new ones have undefined
  behaviour under v1 and SHOULD be flagged `partial` in the matrix.

Explicitly a v1 limitation: vault-scope is appropriate for
whole-book TOC / cross-chapter numbering; per-page state must use
page-scope invocations instead.

Implements: REQ-3309.
Verified by: TEST-3309.

### CON-3310: Ecosystem Check Output

Table (mixed state example):
```
ECOSYSTEM  STATUS        VERSION              PLUGINS CONFIGURED   PLUGINS AVAILABLE
pandoc     detected      3.1.12.1             2                    2
mdbook     detected      n/a (binary absent)  1                    1 (preproc: mdbook-mermaid)
remark     detected      node 20.10.0         3                    3 (./node_modules)
```

**Zero-configured state.** When no ecosystem hooks are declared in the
vault, the command lists every detected runtime with `configured: 0`
and an informational hint, e.g.:

```
ECOSYSTEM  STATUS        VERSION              PLUGINS CONFIGURED   PLUGINS AVAILABLE
pandoc     detected      3.1.12.1             0                    0
mdbook     detected      (binary absent)      0                    0
remark     detected      node 20.10.0         0                    0

No ecosystem hooks configured in this vault.
To enable an ecosystem, add a manifest under .zetl/hooks/:
  https://zetl.codeberg.page/docs/ecosystems/
```

Exit 0 regardless of zero-configured state. Exit 0 when all
*configured* ecosystems are available; non-zero only when a
configured ecosystem is missing its runtime (and not disabled via
`--ecosystem-required=...` inverse semantics).

JSON: same data as an array of objects with `id`, `status`, `version`, `configured`, `available_plugins` fields.

Implements: REQ-3310.
Verified by: TEST-3310.

### CON-3312: Per-Ecosystem Manifest Schema

Defined in `src/ecosystems/<name>/manifest.rs` as a typed Serde struct. Base `PluginManifest` (from SPEC-032) composes with an ecosystem-specific `extra: EcosystemSpecific` field deserialised as a tagged union:

```rust
#[serde(tag = "ecosystem", rename_all = "kebab-case")]
pub enum EcosystemSpecific {
    Pandoc(PandocManifestFields),
    Mdbook(MdbookManifestFields),
    Remark(RemarkManifestFields),
    #[serde(rename = "zetl-native")] ZetlNative,
}
```

Implements: REQ-3312.
Verified by: TEST-3312.

---

## 6. Architecture Decisions

### ADR-3301: zetl-ext Stays the Canonical Internal AST

**Context:** Given Pandoc's #1 position in the ecosystem ranking (19/30 PKM-relevant plugins), an obvious move would be to adopt pandoc-types as zetl's canonical internal AST, making Pandoc filters native and skipping one translation layer. Was considered and debated extensively during SPEC-032 ADR-3201 and in the SPEC-033 drafting conversation.

**Decision:** zetl-ext remains the canonical internal AST. Pandoc-types, mdast, and mdBook's envelope are **views at adapter boundaries**, not internal representations.

**Rationale:**

- **Schema control.** Pandoc's release cadence bumps pandoc-types; adopting it couples zetl to Pandoc's evolution. mdast's versioning is remark's business, not ours. zetl-ext under our control lets us add wikilinks/embeds/SPL as first-class nodes instead of stringly-typed conventions.
- **Multi-ecosystem symmetry.** If we adopt pandoc-types, we're second-class for mdast and vice versa. A neutral canonical form gives every ecosystem equal translation cost.
- **CommonMark simplicity.** zetl-ext is a thin CommonMark superset. pandoc-types has a larger surface (multiple table representations, inline-format intermediaries) because Pandoc supports many output formats (EPUB, LaTeX, DOCX) zetl doesn't care about. Using pandoc-types internally imports complexity we don't benefit from.
- **Translation cost is small.** Measured at ~0.5 ms per typical page. Not a performance concern.

**Trade-offs accepted:** Pandoc filter fidelity requires pandoc-types-level precision; our translator must preserve Pandoc-specific syntax features even when the page was parsed with CommonMark. Mitigation: the parser registry (REQ-3306) lets a page with Pandoc-ecosystem hooks opt into Pandoc-parsing to avoid pre-filter information loss.

Status: Proposed.

### ADR-3302: Pandoc Integration via Binary Subprocess (v1)

**Context:** Four mechanisms considered for integrating Pandoc into zetl: (a) binary subprocess, (b) the `pandoc-types` Rust crate + pulldown-cmark (build pandoc-types AST from zetl's own parser), (c) `pandoc.wasm` embedded via wasmtime, (d) direct FFI to Pandoc via Haskell foreign-export-ccall.

**Decision:** (a) binary subprocess. User's installed `pandoc` binary is invoked as a subprocess. zetl does not bundle or embed Pandoc.

**Rationale:**

- **Zero binary bloat.** Alternatives (c) and (d) add 40–150 MiB to zetl's release artifact. Subprocess keeps zetl binary small.
- **Simplest cross-compilation story.** Pandoc on macOS aarch64, Linux aarch64, Linux x86_64, Windows — all work. Bundling GHC runtime or WASM-linking complicates every platform.
- **User trust.** Users install Pandoc themselves via `brew install pandoc` / `apt install pandoc`; zetl doesn't ship someone else's binary. Security and supply-chain clean.
- **Upgrade path.** User upgrades Pandoc independently of zetl; zetl's adapter only needs to keep compatible with pandoc-types API version ranges declared in the matrix.

**Trade-offs accepted:**

- **Per-page subprocess cost.** Mitigated by persistent-mode Pandoc filters (Pandoc supports persistent operation) and batched parsing (a future optimisation can batch page parses into single `pandoc` invocations).
- **User dependency management.** Users need to install Pandoc separately. Mitigated by `zetl ecosystem check` with actionable hints.

**Alternatives considered and rejected:**

- **(b) pandoc-types Rust crate**: Tempting because it keeps everything in-process. But Pandoc filters expect Pandoc's own markdown parser semantics (attribute syntax, bracketed spans, fenced divs); using pulldown-cmark and constructing pandoc-types by hand produces lossy input. Filters like pandoc-crossref would degrade visibly. Rejected because the point of ecosystem integration is working plugins, not "mostly working."
- **(c) pandoc.wasm**: Real-world ecosystem is thin; bundle bloat is substantial; engineering risk for benefits we don't clearly need.
- **(d) Haskell FFI**: Enormous engineering cost; GHC runtime binary bloat; no stable Rust↔Haskell FFI tooling for Pandoc; ongoing maintenance coupling.

**Revisit:** If v1 rollout reveals that subprocess cost is the bottleneck, (b) is the natural escalation target.

Status: Proposed.

### ADR-3303: mdBook Preprocessor Adapter Runs at `pre-parse` Stage

**Context:** mdBook preprocessors operate on raw Markdown chapter content (`.content: String` field of `Chapter`), not on an AST. Their natural zetl stage is therefore `pre-parse`, not `transform`.

**Decision:** mdBook adapter runs hooks at the `pre-parse` stage by default.

**Rationale:**

- **Data-shape match.** `pre-parse` is text-in/text-out; chapter.content is text. No impedance mismatch.
- **Preprocessor expectations preserved.** Preprocessors like `mdbook-mermaid` look for `` ```mermaid `` fences in raw Markdown; after parsing, those become `CodeBlock` nodes and the preprocessor wouldn't find them. Running pre-parse keeps the expectation intact.
- **Stage selection communicates semantics.** A user reading a manifest sees `stage = "pre-parse"` and understands this transforms source text; they see `transform` and understand AST operations. mdBook fits the former.

**Trade-offs:** some mdBook preprocessors that do post-parse work (via returning an `IntermediateBook` type that mdBook itself further processes) don't fit as cleanly. These are flagged `partial` in the matrix. The majority of popular preprocessors are text-substitution patterns.

**Cross-reference:** mdBook preprocessors authored under mdBook's own
contract commonly regex-rewrite raw Markdown. SPEC-032 REQ-3201's
"pre-parse caveat" documents the same risk surface for zetl-native
pre-parse hooks and links back here explicitly.

Status: Proposed.

### ADR-3304: remark Harness via Persistent Node Subprocess

**Context:** remark plugins are in-process ESM JavaScript. To run them without embedding a JS runtime in zetl (SPEC-031 tried this with QuickJS and it was rejected on scan data), we need an out-of-process bridge.

**Decision:** Persistent Node subprocess running a zetl-provided harness (`zetl-remark-harness.mjs`) that loads plugins via dynamic `import()` and exposes a JSON-RPC-like protocol on stdin/stdout.

**Rationale:**

- **Node is widely installed.** Any user running remark plugins already has Node 18+.
- **Plugin resolution is Node's native strength.** `node_modules` resolution, ESM imports, and semver ranges are all Node's problem, not zetl's. The harness uses them directly.
- **Persistent amortises marshalling.** mdast JSON is larger than pandoc-types JSON, so per-page marshalling matters more; persistent harness amortises the Node startup cost across the whole build.
- **Alternatives considered**: bundle a JS runtime (QuickJS per SPEC-031; rejected by scan data), bundle Deno (even larger), bundle pandoc.wasm style (not applicable to remark), in-process via embedded V8 (huge dep).

**Trade-offs:**

- **Second runtime dep.** Users installing remark ecosystem need Node. Mitigated: it's only needed if they enable remark ecosystem in the first place.
- **Harness maintenance.** Zetl owns the harness script; breaking changes to `unified` or `remark-core` may require harness updates. Harness version pinned in `tools/zetl-ecosystem-matrix.toml`.

Status: Proposed.

### ADR-3305: Three Ecosystems, Chosen Empirically

**Context:** Five or six ecosystems are plausible v1 targets. Choice matters because each adds maintenance surface.

**Decision:** Pandoc, mdBook, remark. Selected on `tools/ecosystem-scan.md` data.

**Rationale:**

- **Pandoc**: #1 by PKM-relevance (63% hit rate). The only ecosystem with citations, crossrefs, transclusion at top-30 popularity. Audience: zetl's academic/technical users.
- **mdBook**: Best maintenance (90% active). Architecturally simplest (JSON stdio). Audience: docs-site authors.
- **remark**: Largest JS ecosystem by active plugins. Critical for the web-docs audience. Transitively reaches rehype/retext via composition.

Runner-up rationale is in `tools/ecosystem-scan.md` §"Runners-up."

Status: Proposed. Revisit per release cadence based on user feedback and ecosystem-scan refresh.

---

## 7. Purity Boundary Map

### Pure Core

- `ecosystems::pandoc::translator::{to_pandoc_types, from_pandoc_types}` — pure AST conversion.
- `ecosystems::mdast::translator::{to_mdast, from_mdast}` — pure AST conversion.
- `ecosystems::mdbook::envelope::{build_book_envelope, extract_transformed_content}` — pure JSON shape construction.
- `ecosystems::remark::harness_protocol::{build_apply_msg, parse_apply_result}` — pure message construction/parsing.
- `ecosystems::registry::{lookup_by_id, supported_ecosystems}` — pure registry queries.
- `parser::resolve_parser_for_page({page, config})` — pure parser selection.

### Effectful Shell

- `ecosystems::pandoc::adapter` — subprocess spawning, env var setting, stdin/stdout piping.
- `ecosystems::mdbook::adapter` — subprocess spawning, `supports html` probing.
- `ecosystems::remark::harness_runtime` — Node subprocess lifecycle, long-lived harness management.
- `parser::pandoc_parser` — `pandoc -f markdown -t json` subprocess invocation.
- `ecosystems::discovery::scan_mdbook_preprocessors` — `$PATH` scanning.
- `ecosystems::discovery::resolve_remark_plugins` — `node_modules` resolution via Node itself.
- `ecosystems::detection::probe_runtimes` — startup-time runtime detection.

### Boundary Contracts

- `PandocTypesDocument` — shell → core at translator boundary.
- `MdastDocument` — shell → core at translator boundary.
- `MdBookBook` — shell → core at translator boundary.
- `RuntimeStatus` — adapter probe → registry; struct containing detected-version and plugin-availability enum.
- `PluginResponse { payload, diagnostics, build_data_writes, template_vars }` — shell produces; core consumes per SPEC-032 CON-3201.

### Dependency Rule

Core ecosystem modules (translators, registry, manifest validation) MUST NOT import `std::process`, `tokio::process`, or any subprocess crate. Adapters (effectful shell) depend on translators (pure core), never vice versa.

### Enforcement

- Crate-level lints in `src/ecosystems/core/`.
- CI-enforced module visibility; core re-exports nothing from adapter layers.

---

## 8. Test Strategy

### TEST-3301: Registry Integrity

Assert every adapter in the registry implements the full `EcosystemAdapter` trait; adapter's declared `supported_stages` matches stages declared in test fixtures; supported_ast_types list is non-empty.

Verifies: REQ-3301.

### TEST-3301-perf: Adapter Cold-Start

Benchmark: activate each ecosystem adapter in isolation, measure time to first-invocation-ready. Assert each ≤ 200 ms P95 over 100 runs.

Verifies: NFR-3301.

### TEST-3302: Adapter Trait Conformance

For each adapter, run a parameterised conformance suite that exercises probe → translate_to_foreign → invoke_plugin (identity) → translate_from_foreign on a corpus of zetl-ext fixtures. Assert output equals input (identity round-trip).

Mutation kill rate on adapter trait-impl modules ≥ 85%.

Verifies: REQ-3302.

### TEST-3303: Pandoc Adapter — Real Plugins

For each matrix plugin at tier `supported`: run `zetl build` on a golden fixture vault with the plugin active; assert output HTML equals the recorded baseline (run under real Pandoc, not zetl, for the baseline) to the normalisation threshold.

Matrix entries: `pandoc-crossref`, `pandoc-citeproc` (or contemporary equivalent), `pantable`, `pandoc-include-code`, `pandoc-plantuml` (if installed in CI) as minimum.

Verifies: REQ-3303.

### TEST-3303-perf: Pandoc Adapter Round-Trip

Benchmark: persistent-mode Pandoc filter (identity) on 500-node pandoc-types; assert round-trip transport ≤ 15 ms P95.

Verifies: NFR-3302.

### TEST-3304: mdBook Adapter — Real Preprocessors

For each matrix preprocessor at tier `supported`: golden HTML diff. Matrix entries: `mdbook-mermaid`, `mdbook-toc`, `mdbook-katex`, `mdbook-admonish`.

Verifies: REQ-3304.

### TEST-3305: remark Adapter — Real Plugins

For each matrix plugin: golden HTML diff. Matrix entries: `remark-gfm`, `remark-math`, `remark-directive`, `remark-frontmatter`, `remark-smartypants`.

Verifies: REQ-3305.

### TEST-3305-fidelity: Translator Round-Trip Property

For each translator (pandoc-types, mdast):
- Generate 10,000 diverse zetl-ext ASTs via property-based generator.
- Assert `translate_from(translate_to(A)) == A` for each.
- Shrink on failure.

Run in CI on every translator change. Mutation kill rate ≥ 90% on translator modules.

Verifies: NFR-3305.

### TEST-3306: Parser Selection

Matrix:

| Setup                                                                        | Expected parser |
| ---------------------------------------------------------------------------- | --------------- |
| No frontmatter, no config rule, no vault default                             | commonmark      |
| `[parse] default = "pandoc"`                                                 | pandoc          |
| `[[parse.rule]] pattern = "papers/**" parser = "pandoc"`, page in `papers/`  | pandoc          |
| Page frontmatter `parser: commonmark` with vault default `pandoc`            | commonmark      |
| Page frontmatter `parser: djot` (unknown parser)                             | parse error + skip |

Verifies: REQ-3306.

### TEST-3307, TEST-3308: Translation Correctness

Beyond round-trip property tests, hand-crafted fixtures for edge cases:
- Wikilinks inside a BlockQuote
- Embed with both heading and block_id
- SPL block with trailing whitespace
- Frontmatter with nested structures
- Position fidelity across translation

Verifies: REQ-3307, REQ-3308.

### TEST-3309: mdBook Envelope

Fixture: invoke `mdbook-mermaid` with the synthetic envelope; assert the returned envelope's chapter content has mermaid blocks replaced by SVG markup per mdbook-mermaid's documented behaviour.

Verifies: REQ-3309.

### TEST-3310: `zetl ecosystem check`

- All ecosystems present: assert table output.
- Missing pandoc binary: assert status `missing`, actionable hint in output, exit 0 (unless `--ecosystem-required=pandoc`).
- Mixed availability: assert JSON output shape.

Verifies: REQ-3310.

### TEST-3311: Matrix Gate

Simulated PR downgrading a matrix plugin's tier without rationale → CI fails.

Verifies: REQ-3311.

### TEST-3312: Manifest Validation

For each ecosystem, valid and invalid manifest examples. Parse errors for:
- `package = "..."` on `ecosystem = "pandoc"` manifest (remark-only field).
- Missing `exec` on pandoc manifest.
- Unknown `ecosystem` value.
- Both `exec` and `lua_filter` on pandoc manifest.

Verifies: REQ-3312.

### TEST-3313: Runtime Detection

Run zetl with no pandoc installed; assert `[zetl] ecosystem pandoc: missing (install hint: brew install pandoc)` on stderr. Run with `--ecosystem-required=pandoc`; assert exit non-zero.

Verifies: REQ-3313.

### Fuzzing — Translator Inputs

`cargo-fuzz` targets on `translate_from_pandoc_types` and `translate_from_mdast` with random JSON inputs. Assert no panics.

### Fuzzing — Adapter Protocol

Feed adversarial JSON from persistent-mode hooks; assert no host crashes, no memory safety violations, error paths covered.

### Synthetic-User Simulation

Profiles 2.1–2.4 walked against the draft spec. Findings converted to REQ/NFR amendments before status → `approved`.

---

## 9. Observability

### OBS-3301: Ecosystem Detection

At zetl startup: one log line per registered ecosystem.
`[zetl] ecosystem pandoc: detected v3.1.12.1 (pandoc-types 1.23)`
`[zetl] ecosystem mdbook: detected (preprocessors: mdbook-mermaid, mdbook-toc, mdbook-katex)`
`[zetl] ecosystem remark: detected (node v20.10.0, harness v1.0.3)`

Metric: `zetl_ecosystem_status{ecosystem, status}` gauge; `status` ∈ {detected, missing, version_incompatible}.

### OBS-3302: Per-Ecosystem Invocation Rate

Metric: `zetl_ecosystem_invocation_total{ecosystem, plugin, outcome}` counter.
Log per build summary: `[zetl] ecosystem pandoc: 3 filters loaded, 247 invocations, 0 failures, 2 marker-strip warnings`.

### OBS-3303: Ecosystem Runtime Resource Usage

Metrics:
- `zetl_ecosystem_subprocess_memory_mib{ecosystem, plugin}` gauge.
- `zetl_ecosystem_subprocess_cpu_ms{ecosystem, plugin}` histogram.
- `zetl_ecosystem_subprocess_restart_total{ecosystem, plugin, reason}` counter (OOM, crash, timeout).

### OBS-3304: Translation Fidelity Warnings

Metric: `zetl_ecosystem_marker_strip_total{ecosystem, plugin, marker_type}` counter.
Log: `[zetl] ecosystem pandoc: pandoc-crossref dropped 4 zetl-wikilink markers on projects/q2-review.md`.

### OBS-3305: Matrix Tier Distribution

`zetl_ecosystem_matrix_plugins_by_tier{ecosystem, tier}` gauge, emitted per build summary.

---

## 10. Security Considerations

- **Ecosystem runtimes are user-installed, user-trusted.** Zetl does not bundle Pandoc, Node, or mdBook; it spawns whatever the user's `$PATH` provides. Users who install untrusted binaries get whatever risk those binaries bring; zetl amplifies nothing.
- **Plugin code is untrusted** (same posture as SPEC-032 §10). Each ecosystem has its own isolation characteristics:
  - Pandoc filters: subprocess with the user's UID/GID. No zetl-level isolation beyond what the OS provides. Users wanting sandboxing should run zetl under bwrap/Firejail/Docker themselves.
  - mdBook preprocessors: same.
  - remark plugins: run inside Node's single-threaded event loop; a plugin can access the filesystem and network through Node's APIs. If a user `npm install`s a malicious plugin, zetl inherits the risk.
- **Translator boundary as trust boundary.** Translators deserialise foreign ASTs. Input validation: max AST depth (256, matches SPEC-032), max payload size (16 MiB), no recursion without depth-bounded fuel. Fuzz tested.
- **Supply chain posture per ecosystem:**
  - Pandoc: user installs via distro package manager; zetl verifies version via `--version` output. CVEs in Pandoc are Pandoc's team problem.
  - mdBook preprocessors: typically Rust binaries from `cargo install`; supply-chain risk is `crates.io`.
  - remark plugins: `npm install`; supply-chain risk is `npm`. Zetl does not run `npm audit`; users should.
- **AI Trust Boundary**: Tier 3 (standard feature code; untrusted JSON at the adapter boundary). Implementation review per USDD §Multi-Model Cognitive Diversity.

**Safe-mode and theme hook declaration:** SPEC-032 REQ-3223 defines a
shared safe-mode surface — `zetl build --no-hooks` skips every hook
(zetl-native and ecosystem) and produces plain pipeline output, and
themes that ship hooks MUST declare them in `theme.toml`. That
requirement applies uniformly to this spec's ecosystem hooks. Users
auditing an unfamiliar theme or vault SHOULD run
`zetl build --no-hooks` first, then re-enable incrementally by
inspecting `zetl theme show`'s declared-hooks list.

**remark harness poisoning note** — a long-lived Node harness with
dynamic `import()` can let a first-loaded plugin monkey-patch globals
that later plugins rely on. Users in a higher-risk posture SHOULD set
`isolation = "fresh-context"` in their remark manifests (REQ-3305) at
the cost of per-invocation startup.

---

## 11. Documentation Plan

- **README.md** — new "Ecosystem bridges" section. Lists v1 ecosystems, links to per-ecosystem guides.
- **`docs/ecosystems/pandoc.md`** — invocation contract, translation table, matrix excerpts, example manifests, `zetl ecosystem check` walkthrough.
- **`docs/ecosystems/mdbook.md`** — same shape.
- **`docs/ecosystems/remark.md`** — same shape; harness architecture notes.
- **`docs/ecosystems/adding-a-new-ecosystem.md`** — contributor guide. Walks through `djot` as worked example.
- **`docs/ecosystems/translation-tables.md`** — auto-generated from translator source; the `what-round-trips` reference.
- **CHANGELOG entry** for SPEC-033 rollout phases.
- **Migration notes** for SPEC-031 readers (no action needed; link to SPEC-033 as the superseding direction).

---

## 12. Rollout Plan

**Release target mapping (working estimate — revise per actual landings):**

| Phase | Description (below)                  | Target release | Depends on                 |
| ----- | ------------------------------------ | -------------- | -------------------------- |
| A     | Core machinery (registry, trait)     | 0.6.0          | SPEC-032 Phase A           |
| B     | Pandoc adapter                       | 0.7.0          | A                          |
| C     | mdBook adapter                       | 0.7.0 or 0.8.0 | A                          |
| D     | remark adapter                       | 0.8.0          | A (Node harness is larger) |
| E     | SPEC-031 supersession                | 0.6.0          | A                          |
| F     | Feature-flag retirement              | 0.9.0          | B, C, D stable 2 rel.      |

SPEC-033 Phase A co-ships with SPEC-032 Phase A — the shared protocol
surface (REQ-3221 + CON-3221) lives in SPEC-032 and lands once.
SPEC-033 Phase B (Pandoc adapter) is the precondition for SPEC-032's
theme-stub canonical extensions (SPEC-032 REQ-3212, Phase C).

**Phase A — Core machinery (no user-visible ecosystems):**

- Ecosystem registry trait, `EcosystemAdapter` interface.
- Per-ecosystem feature flags (`--features ecosystem-pandoc,ecosystem-mdbook,ecosystem-remark`).
- Translator modules with property-test harnesses.
- `zetl ecosystem check` subcommand (reports zero ecosystems available initially).
- Parser registry + commonmark + pandoc parser adapters.

Ships as preview behind `--features ecosystems-v1` umbrella flag.

**Phase B — Pandoc adapter (first user-visible ecosystem):**

- Pandoc invocation, pandoc-types translator, marker conventions.
- Matrix seeded with `pandoc-crossref`, `pandoc-citeproc`, `pantable`.
- Golden HTML tests.
- `docs/ecosystems/pandoc.md` published.

**Phase C — mdBook adapter:**

- mdBook envelope, `supports html` probe, page-scope invocation.
- Matrix seeded with `mdbook-mermaid`, `mdbook-toc`, `mdbook-katex`, `mdbook-admonish`.
- Documentation.

**Phase D — remark adapter:**

- Harness bundled, Node subprocess lifecycle, mdast translator.
- Matrix seeded with `remark-gfm`, `remark-math`, `remark-directive`.
- Documentation.

**Phase E — SPEC-031 formal supersession:**

- SPEC-031 status → `superseded`, link to SPEC-033.
- SPEC-032 REQ-3212 / ADR-3204 (canonical first-party extensions) marked as moved to SPEC-033 ecosystem selection; no canonical extensions shipped first-party.

**Phase F — Feature flag retirement:**

- `--features ecosystems-v1` umbrella retired when all three adapters are stable for two consecutive releases.

**Rollback:** If any adapter proves unstable post-release, it reverts to `experimental` tier in the matrix and is feature-flag-off by default. Users can opt back in if needed.

---

## 13. Open Questions

1. **SPEC-032 scope narrowing.** ~~SPEC-032 currently ships canonical
   Callouts/Tasks/Admonition extensions in the default theme…~~ **Resolved
   2026-04-19: option (b) — thin stubs.** SPEC-032 REQ-3212 and ADR-3204
   have been amended: the default theme ships CSS + template partials
   only; the transformation is delegated to an ecosystem plugin
   configured in `.zetl/hooks/`. This removes Python/Node as a
   default-theme runtime dependency, keeps theme-layer design ownership
   in zetl, and delegates transformation ownership to the ecosystem.
2. **Pandoc parser bundling vs subprocess.** ADR-3302 picks subprocess. If users complain about per-page Pandoc invocation cost, should zetl ship pandoc.wasm as an opt-in feature? *Proposed:* defer; measure real-world cost first.
3. **remark in-process via embedded Deno.** Deno has a Rust integration story (`deno_core`). Would avoid a Node subprocess. *Proposed:* defer; Deno integration is a big commit for marginal gain over persistent Node subprocess.
4. **Plugin-install orchestration.** Should `zetl ecosystem install pandoc-crossref` be a thing? *Proposed:* no; zetl defers to each ecosystem's native install mechanism and only provides hints.
5. **markdown-it as v1.1 addition.** Scan showed it has comparable category coverage to remark but weaker maintenance. Worth adding later? *Proposed:* only if user demand materialises; the scan data suggests remark covers most of the JS-side use cases.
6. **Quarto integration.** Quarto layers on Pandoc; if zetl supports Pandoc well, Quarto-produced filters mostly work. Should we document the Quarto path explicitly? *Proposed:* yes, in `docs/ecosystems/pandoc.md`, as a "Using Quarto filters" subsection.
7. **Matrix refresh cadence.** Each ecosystem's top plugins evolve. Who runs the scan at what cadence to keep the matrix current? *Proposed:* re-run the `tools/ecosystem-scan` before every minor release; matrix updates are a release-blocking check.
8. **Template-variable publishing per ecosystem.** SPEC-032 REQ-3214 defines `page.ext.<extension_id>` for zetl-native hooks. Pandoc filters don't emit zetl-style template vars. Do we heuristically scrape Pandoc `Meta` entries into `page.ext.<plugin>`? *Proposed:* yes for first-party adapter integrations; document the convention.
9. **Vault-level Pandoc mode.** Some users will want every page processed with Pandoc. Is `[parse] default = "pandoc"` sufficient, or do we need a "pandoc-mode" global switch that also enables Pandoc ecosystem hooks by default? *Proposed:* the config default is enough; don't add a meta-mode.
10. **Transient subprocess failures.** Pandoc filter crashes mid-build: does zetl retry? Per SPEC-032 REQ-3207 the page fragment reverts. But if the crash is transient (GC pause, transient OOM), a retry might succeed. *Proposed:* no retry in v1; revisit if field data shows transient failures are common.

---

## 14. Traceability Summary

| REQ      | Tests                        | Contracts  | ADRs         | OBS         |
| -------- | ---------------------------- | ---------- | ------------ | ----------- |
| REQ-3301 | TEST-3301, 3301-perf         | CON-3301   | ADR-3305     | OBS-3301    |
| REQ-3302 | TEST-3302                    | CON-3302   | —            | —           |
| REQ-3303 | TEST-3303, 3303-perf         | CON-3303   | ADR-3302     | OBS-3302    |
| REQ-3304 | TEST-3304                    | CON-3304   | ADR-3303     | OBS-3302    |
| REQ-3305 | TEST-3305, 3303-perf (remark)| CON-3305   | ADR-3304     | OBS-3302    |
| REQ-3306 | TEST-3306                    | CON-3306   | ADR-3301     | —           |
| REQ-3307 | TEST-3307, 3305-fidelity     | CON-3307   | ADR-3301     | OBS-3304    |
| REQ-3308 | TEST-3308, 3305-fidelity     | CON-3308   | ADR-3301     | OBS-3304    |
| REQ-3309 | TEST-3309                    | CON-3309   | ADR-3303     | —           |
| REQ-3310 | TEST-3310                    | CON-3310   | —            | OBS-3301    |
| REQ-3311 | TEST-3311                    | —          | —            | OBS-3305    |
| REQ-3312 | TEST-3312                    | CON-3312   | —            | —           |
| REQ-3313 | TEST-3313                    | —          | —            | OBS-3301    |
| NFR-3301 | TEST-3301-perf               | —          | —            | —           |
| NFR-3302 | TEST-3303-perf               | —          | —            | —           |
| NFR-3303 | TEST-3303-perf (remark arm)  | —          | ADR-3304     | —           |
| NFR-3304 | TEST-3304-size               | —          | —            | —           |
| NFR-3305 | TEST-3305-fidelity           | —          | ADR-3301     | OBS-3304    |
| NFR-3306 | TEST-3306-determinism        | —          | —            | —           |
| NFR-3307 | TEST-3307-memory             | —          | ADR-3304     | OBS-3303    |

---

## 15. Quality Gate Self-Check

- [x] Requirements unambiguous — measurable criteria (timeouts, version strings, translation round-trip byte identity).
- [x] Requirements verifiable — every REQ has a TEST reference.
- [x] Requirements atomic — single obligation per REQ.
- [x] No internal conflicts — REQs 3301–3313 are orthogonal.
- [x] Components have single responsibility — registry, adapters, translators, parsers, detection.
- [x] Functionality via well-defined interfaces — CON-3301 through CON-3312.
- [x] Tests derived from requirements — traceability table.
- [x] Security controls specified — §10 explicit.
- [x] Observability captured — OBS-3301 through OBS-3305.
- [x] Empirical basis recorded — §1.4 cites `tools/ecosystem-scan.md` and `tools/parser-lit-survey.md`.

**Not yet cleared:**

- [ ] Stakeholder validation on open questions §13 (ten items).
- [ ] Adversarial review from a fresh context (USDD principle 12).
- [ ] Synthetic-user simulations for Profiles 2.1–2.4.
- [ ] JSON schemas for pandoc-types translator and mdast translator written and validated.
- [ ] `EcosystemAdapter` trait API sketch reviewed (Rust ergonomics).
- [ ] Legal review of Pandoc/Node/mdBook ecosystem trademark and license compatibility (informational; no distribution dependency).
- [ ] SPEC-032 amended to remove REQ-3212 / ADR-3204 (canonical-extensions scope; moved here per §13 Q1).

Status remains `draft` until these clear.

---

**End of SPEC-033.**
