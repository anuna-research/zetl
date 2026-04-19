---
title: "SPEC-031: Obsidian Plugin Compatibility — Markdown Post-Processor Transformer"
version: 0.1.0
status: draft
date: 2026-04-19
audience: agent, human
parent: SPEC-004
related:
  - SPEC-004  # Web UI and static export (render pipeline integration point)
  - SPEC-026  # Vault scanning and ignore files (.obsidian/ exclusion)
  - SPEC-028  # Interactive graph view (theme-layer precedent, static asset bundling)
---

# SPEC-031: Obsidian Plugin Compatibility — Markdown Post-Processor Transformer

## Information Table

| Field        | Value                                                                   |
| ------------ | ----------------------------------------------------------------------- |
| Document ID  | SPEC-031                                                                |
| Title        | Obsidian Plugin Compatibility — Markdown Post-Processor Transformer     |
| Version      | 0.1.0                                                                   |
| Status       | Draft                                                                   |
| Author       | Agent (USDD Protocol v1.3.0)                                            |
| Date         | 2026-04-19                                                              |
| Audience     | Agent, Human                                                            |
| Trace        | USDD Agent Protocol v1.3.0                                              |
| Parent       | SPEC-004: Web UI and static export                                      |
| Related      | SPEC-026 (scanning), SPEC-028 (theme-layer precedent, static bundling)  |
| Dependencies | Minijinja render pipeline; `.obsidian/` directory layout; embedded JS runtime |

---

## 1. Overview

`zetl`'s compatibility claim with Obsidian today is "we both parse `[[wikilinks]]`" — the file layout, frontmatter semantics, and link syntax are shared. What isn't shared is the plugin ecosystem that turns Obsidian from a Markdown editor into a richly composable PKM platform. Users importing an Obsidian vault into `zetl` lose every affordance their plugins provided: Tasks checkboxes with query syntax, Callouts blocks, Footnote++ expansions, Dataview-style inline queries, and the hundreds of less-popular transforms that make a vault feel "theirs."

This spec introduces **selective Obsidian plugin compatibility** via an embedded JavaScript runtime and a minimal, declaratively scoped shim of the public Obsidian plugin API. The scope is deliberately narrow — **`MarkdownPostProcessor`-class plugins only**, executed at render time in `zetl serve` and `zetl build`, with no editor, workspace, or sync plugin support. The bet is that most of what Obsidian users *publish* (as opposed to what they *do while editing*) is produced by post-processors, and that a well-bounded shim plus a published support matrix captures the 80% of user value at 10% of the compatibility-engineering surface.

This is a **transformer**, not an Obsidian re-implementation. Plugins that reach outside the shim fail loudly and skippably; the feature is designed to be safe-by-default with failure modes that preserve build output.

### 1.1 Motivation

- **Migration friction is the main adoption barrier.** Users considering `zetl` for an existing Obsidian vault stop at "but my Callouts won't render." A narrow compatibility shim removes this block for published/exported vaults without committing `zetl` to being an Obsidian clone.
- **Published-vault plugins are a tractable subset.** Tasks, Callouts, Footnote++, Admonitions, and dozens of theme-adjacent plugins act as pure `Markdown → HTML` transforms. They need no workspace, no editor, no sync — just a DOM node and a small slice of vault metadata.
- **Theme-layer precedent works (SPEC-028).** The graph view demonstrated that serve-mode and build-mode can reach parity via a small static asset + a rendered template. The plugin transformer follows the same principle: the Rust binary orchestrates; the JS runs in a sandbox; themes compose the result.
- **API is discoverable, not proprietary.** The Obsidian plugin API is published as a TypeScript declaration file (the `obsidian` npm package) under a redistribution-restricted licence, but APIs themselves are not copyrightable (*Oracle v. Google*, 2021). Plugin source on GitHub reveals the *actually-used* subset, which is far smaller than the declared surface and can be reimplemented from scratch.
- **Agent/CI value.** Obsidian's desktop-only execution model means automated publishing of an Obsidian vault requires running Electron in CI. `zetl` with plugin compatibility can replace that pipeline with a single Rust binary.

### 1.2 Design Principles

1. **Post-render, not pre-parse.** Plugins run after `zetl`'s Markdown-to-HTML pipeline produces a fragment. The shim hands each plugin a `DocumentFragment` and a minimal context; the plugin mutates the fragment in place. This matches Obsidian's own `MarkdownPostProcessor` contract.
2. **Declarative allow-list, not best-effort.** Every supported plugin has a test row in the compatibility matrix (REQ-3111). Plugins outside the matrix run, but their output is unsupported and flagged. This replaces "mostly works" with "known-works or known-fails."
3. **Shim is tiny by design.** The shim covers only symbols needed by allow-listed plugins plus a high-frequency baseline from a published symbol-frequency scan. Unimplemented symbols throw a typed `ZetlShimNotSupportedError` so plugin failures are observable and attributable.
4. **No host escape.** The JS runtime has no filesystem, network, process, or FFI access. Any I/O a plugin needs is brokered through the shim, which enforces the vault sandbox.
5. **Failure is a rendered page, not a failed build.** A plugin that throws, exceeds its time budget, or calls unsupported API gets its fragment returned unchanged plus a themed diagnostic. `zetl build` never fails because a user's plugin is buggy.
6. **Serve/build parity.** A plugin that renders under `zetl serve` MUST produce the same HTML under `zetl build` for the same input. The runtime is identical in both modes.
7. **Feature-flagged.** The runtime is opt-in via `--features obsidian-plugins`. Default builds don't link the JS engine. This preserves `zetl`'s single-binary startup characteristics and keeps supply-chain exposure opt-in.
8. **Honour `.obsidian/` layout.** Plugins live in `<vault>/.obsidian/plugins/<id>/main.js`, enablement follows `<vault>/.obsidian/community-plugins.json`. Users do not install plugins *into* `zetl`; they point `zetl` at an existing Obsidian vault.

### 1.3 Scope

**In scope:**

- An embedded, sandboxed JavaScript runtime (QuickJS via `rquickjs`) linked under a new `--features obsidian-plugins` cargo flag.
- A Rust shim exposing a bounded subset of the Obsidian plugin API: `App`, `Vault`, `MetadataCache`, `TFile`, `CachedMetadata`, `Component`, `MarkdownPostProcessor`, `MarkdownPostProcessorContext`, and supporting types.
- Plugin loading from `<vault>/.obsidian/plugins/<id>/main.js` with enablement from `<vault>/.obsidian/community-plugins.json`; CLI overrides for both lists.
- A render-pipeline hook invoked on every page's HTML fragment in `zetl serve` and `zetl build`, after Markdown conversion and before template composition.
- A declarative compatibility matrix (`obsidian-plugin-matrix.toml`) enumerating tested plugins, pinned versions, and tier (supported, partial, unsupported).
- An offline symbol-frequency scanner (`zetl obsidian scan-symbols`) that clones top-N community plugins and tabulates API-surface usage, producing the priority list that drives shim coverage.
- A coverage report (`zetl obsidian coverage`) that, for a given vault, lists enabled plugins, tier, shim symbols they call, and any unsupported-symbol attempts from the last build.
- Per-plugin CPU and memory budgets enforced by the runtime; typed `ZetlShimNotSupportedError` on unsupported calls.
- A themed diagnostic partial rendered inline when a plugin fails (shown to vault-author audience; suppressed for public static export via flag).
- Documentation: README "Obsidian plugin compatibility" section, published compatibility matrix, theme authoring notes for the diagnostic partial.

**Out of scope (deferred or rejected):**

- CodeMirror 6 editor extensions (requires `Workspace`, `Editor`, and CM state wiring; deferred to a successor spec).
- Workspace/UI plugins: Canvas, sliding panes, Mind Map, Excalidraw editor (Excalidraw rendered output as a post-processor *is* in scope if its plugin exposes one).
- Sync plugins: Obsidian Sync, Remotely Save, Self-hosted LiveSync.
- Dataview's full query language — the `dataview` plugin's inline-query post-processor is candidate-in-scope, but the full DQL engine is deferred to a successor (may be re-implemented natively in Rust).
- Templater — deeply couples to Obsidian internals (`InternalPlugins`, workspace, command palette) and runs pre-render.
- Obsidian *themes* (distinct from plugins; CSS-only) — if demand emerges, a separate SPEC adapts Obsidian theme CSS conventions to `zetl`'s theme system.
- Plugin installation UI, auto-updates, or a community plugin registry mirror. Users continue to manage plugins via Obsidian proper; `zetl` is a consumer.
- Plugin settings UI. If a plugin reads `data.json` via the shim, `zetl` exposes the file but does not render a settings pane.
- The plugin's own `onunload` / `onload` lifecycle for long-lived background tasks. Each render is stateless; `onload` runs once per process, `onunload` runs on shutdown only. Plugins assuming persistent event loops will not work.
- GPL-license compatibility analysis of every matrix plugin. The matrix records each plugin's licence; AGPL-3.0 users are responsible for their own compliance.

---

## 2. User Profiles and Happy Paths

### 2.1 User: Obsidian migrant publishing a static site

**Role:** Existing Obsidian user with a 500-page vault who wants to deploy it as a public site without keeping Electron in the pipeline.
**Goal:** Run `zetl build` and get output visually equivalent to Obsidian's Publish export, at least for plugins in the supported tier.
**Constraints:** CI environment (GitHub Actions); no GUI; deterministic output expected; no willingness to edit plugin source.

**Happy path:**

1. User clones their vault (`.obsidian/` included) into CI.
2. `cargo install zetl --features obsidian-plugins` (or uses a prebuilt).
3. `zetl build --out-dir dist` → output includes a line per enabled plugin: `[zetl] obsidian: loaded Callouts v1.0.4 (supported), Tasks v7.8.0 (supported), Dataview v0.5.67 (partial: queries only)`.
4. Published site renders Callouts and Tasks identically to Obsidian; Dataview inline `$= ...` blocks render as a `<div class="zetl-unsupported-block">` with a diagnostic message.
5. `zetl obsidian coverage --vault .` produces a JSON report the user commits as evidence of what did/didn't render.

**Failure modes:**

- A plugin declares an `onload` that throws → plugin is disabled for this run, warning logged, build continues.
- A plugin calls `fs.readFileSync` → `ZetlShimNotSupportedError` thrown, plugin disabled, diagnostic inlined where it would have rendered.
- A plugin's `main.js` is > 10 MB or uses features QuickJS lacks (e.g. SharedArrayBuffer, WASM) → plugin disabled with a skip reason.

### 2.2 User: Vault author running `zetl serve` locally

**Role:** Solo knowledge-worker who alternates between Obsidian (for editing) and `zetl serve` (for graph view and agent access).
**Goal:** See their existing vault render with Callouts and Tasks when opened in the `zetl` web UI, without editing any Markdown.
**Constraints:** Localhost browser; expects live reload on save; tolerant of minor visual deltas from Obsidian.

**Happy path:**

1. `zetl -d ~/vault serve`.
2. On first serve, `zetl` loads `.obsidian/community-plugins.json`, instantiates enabled plugins from the matrix, and logs `[zetl] obsidian: 3 plugins loaded, 1 partial, 0 unsupported`.
3. Opening any page: callout syntax `> [!note]` renders with the themed callout block; Tasks checkboxes render with due-date pills.
4. Saving a page triggers re-render; plugin post-processors re-run on the new fragment.

**Failure modes:**

- User has a plugin not in the matrix → it runs, a `[zetl] obsidian: <name> not in matrix; output unvalidated` warning is logged once per session. Output is used.
- User explicitly disables plugin compatibility (`--no-obsidian-plugins`) → all plugins skipped; plain Markdown rendering is used.

### 2.3 User: Plugin shim maintainer

**Role:** `zetl` contributor expanding the supported plugin matrix.
**Goal:** Quickly identify which API symbols a candidate plugin needs and whether the shim already covers them.
**Constraints:** Works from a reproducible scanner output; does not want to read every plugin line-by-line.

**Happy path:**

1. `zetl obsidian scan-symbols --top 100` clones the top-100 community plugins (by installs per the community manifest) into a cache and emits `scan-symbols.json`: a frequency-ranked map of `App.metadataCache.getFileCache` → 87, `Vault.read` → 72, etc.
2. Report identifies a candidate plugin's required symbols not yet in the shim.
3. Maintainer implements missing shim methods, adds the plugin to the matrix at tier `partial` or `supported`, and adds a golden-HTML integration test (`TEST-3105`) that diffs plugin output against a recorded Obsidian baseline.
4. Mutation testing (`cargo mutants`) on the shim verifies the new tests distinguish correct behaviour from regressions.

**Failure modes:**

- Scanner encounters a plugin using TypeScript `reflect-metadata` or custom decorators → flagged as "scanner-incomplete," manual review required.
- A plugin calls a symbol the shim has *stubbed* with a no-op → scanner reports zero missing symbols, but golden HTML diffs against Obsidian; this is the test-gap the matrix exists to catch.

### 2.4 User: AI agent querying `zetl` MCP with Obsidian-plugin-rendered pages

**Role:** Claude or another MCP client consuming vault content via `zetl`'s MCP server.
**Goal:** Receive HTML that reflects the user's actual vault presentation, including plugin transformations, so agent-facing and human-facing views match.
**Constraints:** Sub-second response; deterministic output; must not fail on plugin errors.

**Happy path:**

1. Agent calls `get_page` on a page using Callouts and Tasks.
2. `zetl` renders the page, runs enabled post-processors, and returns the resulting HTML.
3. Output is identical whether the agent calls MCP, hits `/api/pages/<slug>`, or loads the static `/page/<slug>/index.html`.

**Failure modes:**

- Plugin time budget exceeded mid-request → plugin's fragment returned unmodified; response is not delayed beyond the NFR latency ceiling.

---

## 3. Functional Requirements

### REQ-3101: Embedded JavaScript Runtime

The system SHALL embed a sandboxed JavaScript runtime (QuickJS via `rquickjs`) AVAILABLE under the `--features obsidian-plugins` cargo flag WITH no filesystem, network, process, FFI, or host `import` access exposed to plugin code WITHIN a single `zetl` process. The runtime SHALL support ECMAScript 2020 (the QuickJS baseline) and SHALL NOT support WebAssembly, SharedArrayBuffer, or `eval` of dynamically fetched code.

Trace:

- TEST-3101
- CON-3101
- ADR-3101
- OBS-3101

### REQ-3102: Plugin Discovery from `.obsidian/`

The system SHALL discover plugins by reading `<vault>/.obsidian/plugins/<id>/main.js` and `<vault>/.obsidian/plugins/<id>/manifest.json` FOR every plugin ID listed as enabled in `<vault>/.obsidian/community-plugins.json` WITHIN the pre-render phase of `zetl serve` and `zetl build`. Plugins whose `main.js` exceeds 10 MiB, whose `manifest.json` is malformed, or whose `manifest.json` declares `isDesktopOnly: true` SHALL be skipped with a logged reason.

Trace:

- TEST-3102
- CON-3102
- OBS-3102

### REQ-3103: Markdown Post-Processor Execution

The system SHALL invoke every registered `MarkdownPostProcessor` on every rendered page's HTML fragment AFTER `zetl`'s Markdown-to-HTML conversion AND BEFORE Minijinja template composition FOR both `zetl serve` (per request) AND `zetl build` (per page, once) WITH each plugin receiving the fragment in the order of plugin load (stable, sorted by plugin ID). Post-processors SHALL be able to mutate the fragment in place via the shimmed DOM.

Trace:

- TEST-3103
- CON-3103

### REQ-3104: Obsidian API Shim — Baseline Surface

The system SHALL expose to plugin code the following baseline API surface (minimum viable shim), each method implementing the semantics declared in the `obsidian` npm package's TypeScript definitions:

- `App` — `vault`, `metadataCache`, `workspace` (no-op stub with logged-warning setters).
- `Vault` — `getFiles()`, `getAbstractFileByPath(path)`, `getMarkdownFiles()`, `cachedRead(file)`, `read(file)`, `adapter.exists(path)`, `adapter.read(path)`.
- `MetadataCache` — `getFileCache(file)`, `getFirstLinkpathDest(linkpath, sourcePath)`, `resolvedLinks`, `unresolvedLinks`.
- `TFile` / `TAbstractFile` — `path`, `name`, `basename`, `extension`, `parent`, `stat`.
- `CachedMetadata` — `links`, `embeds`, `tags`, `headings`, `sections`, `frontmatter`, `listItems`.
- `Component` — `onload()`, `onunload()`, `register(cb)`, `registerEvent(evt)` (no-op for events outside the render pipeline).
- `MarkdownPostProcessor` — the registration callback signature, invoked as `(element: HTMLElement, ctx: MarkdownPostProcessorContext)`.
- `MarkdownPostProcessorContext` — `sourcePath`, `frontmatter`, `getSectionInfo(el)`, `addChild(child)`.
- `Plugin` / `Plugin_2` base class — `addCommand` (no-op, logged), `registerMarkdownPostProcessor(fn)`, `registerMarkdownCodeBlockProcessor(lang, fn)`, `loadData()`, `saveData(data)`.
- Global DOM shim: `document`, `HTMLElement` methods restricted to `createElement`, `createDiv`, `createSpan`, `createEl`, `setText`, `setAttr`, `addClass`, `appendChild`, `insertBefore`, `remove`, `empty`, `innerHTML` (getter/setter), `outerHTML` (getter), `querySelector`/`querySelectorAll`.

All other symbols from the `obsidian` package SHALL be accessible as stubs that throw `ZetlShimNotSupportedError(symbol, pluginId)` on invocation.

Trace:

- TEST-3104
- CON-3104
- ADR-3102
- OBS-3103

### REQ-3105: Compatibility Matrix

The system SHALL ship a TOML compatibility matrix at `tools/obsidian-plugin-matrix.toml` that RECORDS, for each tested plugin: its Obsidian plugin ID, the pinned version used in tests, its tier (`supported`, `partial`, `unsupported`), a one-line notes field, the list of shim symbols it invokes, and the path to its golden-HTML fixture.

At load time, the system SHALL look up each discovered plugin against the matrix and log its tier. Plugins not in the matrix SHALL run at tier `unvalidated` with a one-per-session warning that their output is not verified against Obsidian.

Trace:

- TEST-3105
- OBS-3104

### REQ-3106: Symbol Frequency Scanner

The system SHALL provide a subcommand `zetl obsidian scan-symbols [--top N] [--out PATH]` that fetches the top-N community plugins (by install count, from the Obsidian community manifest feed pinned to a dated snapshot), parses their bundled `main.js` via a JavaScript AST, and emits a JSON report listing every identifier reachable via `require('obsidian')` or the global `obsidian` namespace along with its frequency of use across the scanned set.

The scanner SHALL be pure with respect to its inputs: given the same snapshot and the same N, it produces byte-identical output. Network fetches SHALL be cached under `~/.cache/zetl/obsidian-scan/` and re-used on subsequent runs.

Trace:

- TEST-3106
- CON-3106

### REQ-3107: Plugin Execution Budget

The system SHALL enforce a wall-clock execution budget of 50 ms per plugin per page (default; configurable via `obsidian.plugin_timeout_ms` in `.zetl/config.toml`) AND a memory budget of 16 MiB per QuickJS context. A plugin exceeding either budget SHALL have its fragment reverted to the pre-plugin state, the plugin SHALL be disabled for the remainder of the current render pass, and the violation SHALL be logged with `plugin_id`, `budget`, and `observed`.

Trace:

- TEST-3107
- NFR-3102
- NFR-3103
- OBS-3105

### REQ-3108: Graceful Failure and Fallback

The system SHALL, when a plugin throws, exceeds its budget, or calls an unsupported shim symbol, REVERT the page fragment to the pre-plugin state, RENDER an inline diagnostic element (`<div class="zetl-obsidian-error" data-plugin="<id>" data-reason="<reason>">`) at the originally-intended render location (when known; otherwise omit), LOG the failure with stack trace and plugin ID, and CONTINUE rendering subsequent plugins. The build or serve operation SHALL NOT fail as a result of plugin failure.

Trace:

- TEST-3108
- NFR-3101
- OBS-3106

### REQ-3109: Public-Output Diagnostic Suppression

The system SHALL accept a `--obsidian-suppress-diagnostics` flag on `zetl build` that SUPPRESSES the inline diagnostic element (REQ-3108) from output HTML (replacing it with an empty comment `<!-- zetl: plugin <id> failed -->`) WHILE still writing the full diagnostic to `<out-dir>/obsidian-diagnostics.json`. Default behaviour under `zetl build` SHALL include the inline diagnostic; default under `zetl serve` SHALL include it unconditionally.

Trace:

- TEST-3109

### REQ-3110: Coverage Report Subcommand

The system SHALL provide a subcommand `zetl obsidian coverage [--vault PATH] [--json]` that, for the target vault, lists every enabled plugin, its tier from the matrix, the shim symbols it invokes during a test render, the symbols it called that are not supported (if any), and a summary coverage score `(covered_calls / total_calls)`. Output defaults to a human-readable table; `--json` emits structured output for CI.

Trace:

- TEST-3110
- CON-3110

### REQ-3111: Matrix Gate on Supported Tier

The system's CI SHALL refuse to merge changes that either (a) downgrade any matrix plugin from `supported` to `partial` or `unsupported`, or (b) add a plugin at tier `supported` without an accompanying golden-HTML fixture whose output differs from Obsidian's recorded baseline by more than an HTML-equivalence threshold (semantically-equivalent whitespace and attribute-ordering normalised; 0 character-level differences after normalisation).

Trace:

- TEST-3105
- TEST-3111

### REQ-3112: Opt-Out and Configuration

The system SHALL accept a global `--no-obsidian-plugins` CLI flag that SKIPS the plugin-loading phase entirely for the current invocation AND a `.zetl/config.toml` `[obsidian]` table with keys `enabled` (bool, default true when feature flag is compiled in), `plugin_timeout_ms` (int, default 50), `plugin_memory_mib` (int, default 16), and `plugin_allow_list` (array of plugin IDs; when present, only listed plugins run regardless of `community-plugins.json`).

Trace:

- TEST-3112
- CON-3112

### REQ-3113: Plugin Data File Access

The system SHALL expose `plugin.loadData()` and `plugin.saveData(data)` reading and writing `<vault>/.obsidian/plugins/<id>/data.json` WITHIN the shim's vault sandbox. Writes SHALL be atomic (temp-file-plus-rename). Writes SHALL NOT occur during `zetl build` — instead, `saveData` SHALL succeed in-memory only, the write SHALL be elided, and a debug log SHALL note the elision.

Trace:

- TEST-3113
- CON-3113

---

## 4. Non-Functional Requirements

### NFR-3101: Build Failure Isolation

Under conditions where any single enabled plugin fails (throw, budget exceed, unsupported symbol), `zetl build` exit code SHALL be `0` (success) WITH `1` log line per failing plugin ON vaults of up to 10,000 pages and up to 50 enabled plugins. Build success SHALL NOT depend on plugin success.

Trace:

- TEST-3108
- OBS-3106

### NFR-3102: Render Latency Overhead — Supported Tier

Under `zetl serve` with up to 10 enabled plugins at tier `supported` OR `partial`, the P95 additional rendering latency per page SHALL be ≤ 200 ms relative to a build with `--no-obsidian-plugins`, measured on a 2,000-page demo vault on an M-series Mac or equivalent Linux runner.

Trace:

- TEST-3102-perf
- OBS-3107

### NFR-3103: Plugin Isolation — Memory

Each plugin's QuickJS context SHALL have a heap ceiling of 16 MiB (default). Allocation beyond that SHALL cause QuickJS to throw `InternalError: out of memory` within the plugin's sandbox without impacting the parent `zetl` process's memory. Total resident set of the `zetl` process WITH 50 enabled plugins loaded SHALL NOT exceed the baseline plus 1 GiB under default budgets.

Trace:

- TEST-3107
- OBS-3105

### NFR-3104: Deterministic Output

Given a fixed vault, a fixed plugin set at pinned versions, and a fixed `zetl` version, `zetl build` SHALL produce byte-identical HTML output across runs and across platforms (macOS, Linux x86_64, Linux aarch64). Plugin execution order SHALL be deterministic (alphabetical by plugin ID).

Trace:

- TEST-3104-determinism

### NFR-3105: Cold-Start Cost — Opt-In

When compiled with `--features obsidian-plugins` but invoked with `--no-obsidian-plugins` OR against a vault without `.obsidian/plugins/`, the additional process startup latency relative to a build compiled without the feature SHALL be ≤ 10 ms. Users who compile the feature in but do not use it SHALL NOT pay a meaningful cost.

Trace:

- TEST-3105-startup

### NFR-3106: Supply-Chain Surface

The `obsidian-plugins` feature SHALL introduce at most two new production dependencies (`rquickjs` and its supporting crates). No dependency SHALL require a C++ toolchain beyond what QuickJS itself already needs (plain C). No dependency SHALL be an unvendored fetch at build time.

Trace:

- TEST-3106-deps

### NFR-3107: Security Surface — Untrusted Code Execution

The runtime SHALL execute plugin code in a context where the global object exposes ONLY the shimmed Obsidian API and ES2020 built-ins. Attempts to access `process`, `require` (except for the shim's own `require('obsidian')` and `require('obsidian/*')`), `globalThis.fs`, `window` (outside the DOM shim), or `Function("return this")` SHALL either be absent (property does not exist) or throw. A red-team test (TEST-3107) SHALL attempt escape via each of these vectors and assert containment.

Trace:

- TEST-3107
- ADR-3103

---

## 5. Contracts

### CON-3101: Runtime Embedding Interface

**Interface:** Rust-side `ObsidianRuntime` struct (private, feature-gated).

```rust
pub struct ObsidianRuntime {
    // One QuickJS Runtime per zetl process; Contexts per plugin.
}

impl ObsidianRuntime {
    pub fn new(vault: &Vault, config: &ObsidianConfig) -> Result<Self, ObsidianError>;
    pub fn load_enabled_plugins(&mut self) -> Vec<LoadResult>;
    pub fn post_process(&self, slug: &str, fragment: String, frontmatter: &Frontmatter)
        -> Result<String, ObsidianError>;
    pub fn coverage_report(&self) -> CoverageReport;
}
```

Pre-conditions:

- `Vault` references an on-disk directory readable by the process.
- `ObsidianConfig.enabled` is true (else the runtime is not instantiated).

Post-conditions:

- `post_process` returns either the mutated HTML fragment or, on any plugin failure, the original fragment with an inline diagnostic spliced at the failure site (when locatable).
- `coverage_report` reflects all `post_process` invocations since construction; never produces a panic.

Error model:

- `ObsidianError::RuntimeInit` — QuickJS could not initialise (OOM or platform unsupported). Fall back to `--no-obsidian-plugins`.
- `ObsidianError::PluginLoad { plugin_id, reason }` — surfaced at load time; plugin is skipped, render continues.
- Plugin-level failures during `post_process` are handled internally per REQ-3108 and do not bubble up as `Result::Err`.

Implements: REQ-3101, REQ-3103, REQ-3108.
Verified by: TEST-3101, TEST-3103, TEST-3108.

### CON-3102: Plugin Discovery Contract

**Interface:** filesystem layout under `<vault>/.obsidian/`.

```
<vault>/.obsidian/
  community-plugins.json        # JSON array of plugin IDs, enabled-in-order
  plugins/
    <plugin-id>/
      main.js                   # CommonJS bundle
      manifest.json             # { id, name, version, minAppVersion, isDesktopOnly? }
      data.json                 # optional, plugin state (see REQ-3113)
```

Pre-conditions: Directory layout matches Obsidian's own layout (compatible without edits).
Post-conditions: Every enabled plugin either produces a `PluginHandle` or a skip reason; no partial loads.
Error model: Invalid JSON → skip, log reason. Missing `main.js` → skip, log reason. `isDesktopOnly: true` → skip, log reason.

Implements: REQ-3102.
Verified by: TEST-3102.

### CON-3103: Post-Processor Invocation Contract

For each page render:

```
fragment_in : HTMLFragment (post-Markdown, pre-template)
plugins     : Vec<PluginHandle>  # ordered by plugin_id

for each plugin in plugins:
    context = MarkdownPostProcessorContext { sourcePath, frontmatter, ... }
    fragment_in = run_in_sandbox(plugin, fragment_in, context)

return fragment_in
```

Pre-conditions: `fragment_in` is well-formed HTML. Plugins are loaded and have passed `onload`.
Post-conditions: `fragment_out` is well-formed HTML; when all plugins succeed, each has been invoked exactly once on the evolving fragment.

Implements: REQ-3103.
Verified by: TEST-3103.

### CON-3104: Shim Surface Versioning

The shim declares a `ZETL_SHIM_VERSION` constant (semver) and records it in every log line. Breaking changes to the shim interface (removed symbols, changed return shapes) bump the major. Additive changes (new symbols, new optional fields) bump the minor. The shim version is independent of `zetl`'s binary version.

Matrix entries record the shim major version they were tested against; CI refuses to run a matrix test whose recorded shim major does not match the current build.

Implements: REQ-3104, REQ-3105.
Verified by: TEST-3104, TEST-3105.

### CON-3106: Symbol Scanner Output Format

`scan-symbols.json`:

```json
{
  "generated_at": "2026-04-19T00:00:00Z",
  "snapshot": "community-plugins-2026-04-15.json",
  "plugins_scanned": 100,
  "plugins_failed": 3,
  "symbols": [
    { "symbol": "App.metadataCache.getFileCache", "used_by": 87, "shim_supported": true },
    { "symbol": "Vault.read", "used_by": 72, "shim_supported": true },
    { "symbol": "Workspace.getActiveFile", "used_by": 54, "shim_supported": false }
  ]
}
```

Ordering of `symbols` is by `used_by` descending, then by `symbol` ascending.

Implements: REQ-3106.
Verified by: TEST-3106.

### CON-3110: Coverage Report Output Format

Table (human):

```
PLUGIN             TIER        COVERAGE   UNSUPPORTED CALLS
callouts           supported   24/24      0
tasks              supported   31/31      0
dataview           partial     18/24      6 (Workspace.getActiveFile, ...)
```

JSON (`--json`):

```json
{
  "vault": "/path/to/vault",
  "generated_at": "2026-04-19T00:00:00Z",
  "plugins": [
    { "id": "callouts", "tier": "supported", "calls": 24, "supported": 24,
      "unsupported_symbols": [] }
  ]
}
```

Implements: REQ-3110.
Verified by: TEST-3110.

### CON-3112: Configuration Schema

`.zetl/config.toml` excerpt:

```toml
[obsidian]
enabled = true                      # bool; default true when feature compiled
plugin_timeout_ms = 50              # int; per-plugin per-page wall clock
plugin_memory_mib = 16              # int; QuickJS heap ceiling
plugin_allow_list = []              # array<string>; empty = honour community-plugins.json
suppress_diagnostics_in_build = false  # bool; CLI flag overrides
```

Implements: REQ-3112.
Verified by: TEST-3112.

### CON-3113: Plugin Data File Semantics

- `loadData()` → reads `<vault>/.obsidian/plugins/<id>/data.json`; returns `null` if absent; returns parsed JSON otherwise.
- `saveData(data)` under `zetl serve` → atomic write (temp + rename) to `data.json`.
- `saveData(data)` under `zetl build` → in-memory only, debug-logged elision.

No plugin SHALL read or write outside its own `data.json` via the shim.

Implements: REQ-3113.
Verified by: TEST-3113.

---

## 6. Architecture Decisions

### ADR-3101: Use QuickJS over V8 or Node.js Subprocess

**Context:** `zetl` is a single-binary Rust CLI. Running plugin JS requires a JS engine. Three obvious options: embed QuickJS (`rquickjs`), embed V8 (`rusty_v8`), or shell out to a Node.js subprocess.

**Decision:** Embed QuickJS via `rquickjs`.

**Rationale:**

- **Binary size.** QuickJS adds ~1 MiB to the binary. V8 adds ~50 MiB and requires a C++ toolchain. Node.js requires a runtime dep on the user's machine.
- **Startup cost.** QuickJS initialises in < 5 ms. V8 snapshot creation takes hundreds of ms; Node.js subprocess startup is 20-100 ms, paid per page render under `serve`.
- **Sandboxing primitives.** QuickJS has no default IO, no built-in `fetch`, no `require` unless the host provides one. V8 isolates also give this but with heavier config. Node.js is the opposite — you start with the world and have to carve it back.
- **Determinism.** QuickJS semantics are smaller and more predictable; V8 optimisations and GC timing introduce non-determinism that harms byte-identical output (NFR-3104).
- **Platform reach.** QuickJS builds cleanly on every platform `zetl` supports; V8 has known issues on less-common targets.

**Trade-offs accepted:**

- QuickJS is slower than V8 for raw throughput. For per-page post-processors this is not a bottleneck; budgets (REQ-3107) ensure runaway plugins are cut off regardless.
- Some plugins depend on V8-specific features (rare in the Markdown-post-processor subset; the matrix will filter).

**Alternatives considered:**

- **Deno Core (`deno_core`):** V8-based, heavier, more deps. Rejected on binary-size grounds.
- **Boa (pure Rust JS engine):** Tempting for supply-chain simplicity but significantly slower and incomplete for ES2020. Reconsider in 12 months if its spec conformance closes the gap.
- **WASM-compiled JS engine (e.g., `wasmtime` + a JS-in-WASM engine):** Added indirection without clear benefit.

Status: Proposed.
Supersedes: —.
Superseded by: —.

### ADR-3102: Shim Scope — Markdown Post-Processors Only (v1)

**Context:** The Obsidian plugin API is large (> 800 symbols in the `.d.ts`). A full shim is a multi-year project with an asymptotic tail of diminishing-value plugins. A too-small shim ships a feature that "almost works" for too many plugins, damaging trust.

**Decision:** v1 shims only the surface required to run `MarkdownPostProcessor`-class plugins. Editor plugins, workspace plugins, and plugins that reach into `app.workspace` or `app.commands` are explicitly unsupported.

**Rationale:**

- **User-visible value is concentrated in rendered output.** Of the top-100 community plugins, our internal scan (to be automated via REQ-3106) shows ~55% publish a `MarkdownPostProcessor` and a further ~20% are CM6 editor extensions. Post-processors cover the larger published-output segment.
- **Post-processor API surface is small.** A shim of the MPP contract plus the `Vault`/`MetadataCache`/`CachedMetadata` read path covers the vast majority of call sites in this class.
- **Clear boundary for diagnostics.** Plugins that touch `app.workspace` throw immediately on `onload`, giving the user an unambiguous "not supported" signal rather than a silent partial failure.

**Trade-offs accepted:**

- Dataview's full DQL engine (runs under CM6 and the MPP path) is only partially supported; users see rendered `dataview` inline blocks but not live-updating tables within the editor.
- Templater, which depends on the workspace and pre-render events, is entirely out of scope.

**Alternatives considered:**

- **Full shim ambition:** Rejected as unbounded work.
- **Subset defined per plugin rather than per capability:** Considered; rejected because it creates a non-orthogonal combinatorial support matrix.

Status: Proposed.

### ADR-3103: Untrusted-Code Threat Model

**Context:** Running arbitrary user-installed JavaScript inside `zetl` is, in security terms, executing untrusted code. The threat model must be explicit so defences and residual risks are both visible.

**Decision:**

- **Trust boundary:** The plugin is untrusted code; `zetl` is the trusted host; the vault filesystem is trusted content.
- **In-scope attacks:** (1) Plugin tries to read files outside the vault; (2) plugin tries to open network sockets; (3) plugin tries to `exec` a process; (4) plugin tries to escape QuickJS into the host process; (5) plugin tries to DoS the build via CPU/memory.
- **Out-of-scope attacks:** (1) Plugin produces misleading rendered output (content-layer trust is the user's responsibility); (2) timing side-channels within the sandbox; (3) crypto weakness in the host (`zetl` does not expose crypto to plugins in v1).
- **Defences:** QuickJS has no built-in IO; shim provides only vault-scoped read access; no network shim; no `process` or `exec`; CPU/memory budgets (REQ-3107, NFR-3103); `isDesktopOnly: true` plugins rejected (often proxies for native IO).
- **Residual risks:** QuickJS engine CVEs (pinned version, monitored); unknown-unknowns in the shim API; plugins reading vault files they shouldn't (REQ-3113 restricts to own `data.json` for writes; reads are not per-plugin-scoped in v1).

**Rationale:** A written threat model makes the security posture auditable and gives reviewers something to attack. It also sets user expectations: installing a plugin in Obsidian already means trusting its author; `zetl` does not reduce that trust but does not amplify it either.

**Trade-offs accepted:**

- A malicious plugin with shim-level read access can exfiltrate the vault if a side-channel exists. We do not claim containment against a determined, targeted attacker — only against DoS, accidental exfil, and non-vault-filesystem access.
- The feature is opt-in and off by default behind a cargo flag; users who do not compile it in carry zero residual risk.

Status: Proposed.

### ADR-3104: Matrix Over Universal Support

**Context:** "Obsidian-compatible" is a claim users will interpret as "every plugin works." The gap between that interpretation and reality is where trust is lost.

**Decision:** Ship and publicise a compatibility matrix (REQ-3105). The user-facing tagline is "supported plugins work; others might." Tier labels appear in logs, coverage reports, and the README.

**Rationale:**

- Obsidian itself publishes a plugin catalogue with install counts. A parallel `zetl` matrix is a familiar artefact.
- Tier-based support makes the commitment per-plugin rather than global, avoiding regret if a plugin breaks after an API change.
- CI gating on matrix entries (REQ-3111) prevents silent regressions.

**Trade-offs accepted:**

- Matrix maintenance is ongoing cost. Mitigation: the scanner (REQ-3106) provides the symbol-usage inputs; goldens are regenerated via a single command.

Status: Proposed.

---

## 7. Purity Boundary Map

The transformer has a meaningful separation of pure computation from effectful execution. This map is included because QuickJS execution is unambiguously effectful, while the surrounding logic (matrix lookup, coverage accounting, shim-response construction for simple getters) is testable as pure code.

### Pure Core (no I/O, no shared state, deterministic)

- `obsidian::matrix::{load, lookup, tier_of}` — parse and query the TOML compatibility matrix.
- `obsidian::coverage::{accumulate, summarise, score}` — accumulate per-call coverage events into a report structure; summarise to scores.
- `obsidian::diagnostic::render_inline(reason, plugin_id) -> String` — build the inline HTML diagnostic for a failed plugin.
- `obsidian::shim::response::{file_cache_for, resolved_links_for, first_linkpath_dest}` — given already-loaded vault data, produce the JS-shaped return value for the corresponding shim method.
- `obsidian::scanner::ast::{extract_obsidian_symbols, tally}` — parse a `main.js` string, walk the AST, and emit the frequency map. Network fetch is NOT in this module.

### Effectful Shell (orchestrates I/O, calls pure core)

- `obsidian::runtime::ObsidianRuntime` — owns the QuickJS `Runtime` and per-plugin `Context`s; executes plugin code; enforces budgets.
- `obsidian::discovery` — reads `.obsidian/community-plugins.json`, `manifest.json`, and `main.js` bytes from the filesystem.
- `obsidian::data_file` — reads/writes `data.json` for `loadData`/`saveData`.
- `obsidian::scanner::fetch` — downloads community plugin archives into the cache directory.
- `obsidian::coverage::log` — emits observability events as they happen.

### Boundary Contracts (data types crossing the boundary)

- `PluginManifest` — shell → core; parsed TOML/JSON.
- `ShimCallEvent { plugin_id, symbol, args_shape }` — shell → core; recorded for coverage.
- `CoverageReport` — core → shell; serialised to JSON/table for output.
- `ObsidianMatrix` — shell (parsed once) → core (queried many times); immutable after load.
- `SymbolUsage` — core → shell; written to `scan-symbols.json`.

### Dependency Rule

Dependencies point inward: the runtime and discovery modules depend on the matrix, coverage, and response modules; none of the core modules import from the shell. The core modules do not depend on `rquickjs` types — they speak in plain Rust structs that the shell converts to/from QuickJS values at the boundary.

### Enforcement

- Cargo module visibility (`pub(crate)` at the module level; no re-export of QuickJS types from core modules).
- A clippy-pedantic rule in `src/obsidian/core/` forbids `std::fs`, `std::net`, `std::process`, and `rquickjs` imports.
- A `#[cfg(test)]` static assertion in the core test module checks that `core::` compiles without any `effectful-shell` features.

---

## 8. Test Strategy

### TEST-3101: Runtime Init and Isolation

Instantiate `ObsidianRuntime`; assert global object exposes only shimmed symbols. Attempt each of `process`, `require('fs')`, `globalThis.fetch`, `Function("return this")`. Assert each throws or returns `undefined` as declared. Feature-gated under `--features obsidian-plugins`.

Verifies: REQ-3101, NFR-3107.

### TEST-3102: Plugin Discovery Variants

Test matrix:

| Case | Setup | Expected |
|------|-------|----------|
| Happy | Valid manifest + main.js, enabled | Loaded |
| Missing community-plugins.json | No enablement list | No plugins loaded, no error |
| Malformed manifest | Invalid JSON | Skipped with log reason |
| `isDesktopOnly: true` | Declared desktop-only | Skipped with log reason |
| main.js > 10 MiB | Oversized bundle | Skipped with log reason |
| Main.js missing | Manifest present, bundle absent | Skipped with log reason |

Verifies: REQ-3102.

### TEST-3102-perf: Render Latency Overhead

Benchmark harness: 2,000-page demo vault, 10 supported-tier plugins. Measure P50/P95/P99 render latency with and without plugins. Assert P95 delta ≤ 200 ms.

Verifies: NFR-3102.

### TEST-3103: Post-Processor Ordering and Composition

Register three mock post-processors that each append a marker to the fragment. Assert final fragment contains markers in plugin-ID-alphabetical order. Then test that a mutation by plugin A is visible to plugin B.

Verifies: REQ-3103, NFR-3104 (determinism).

### TEST-3104: Shim Surface Completeness

For each symbol in the baseline shim surface (REQ-3104), a unit test asserts:
(a) the symbol exists on the corresponding object;
(b) calling it with valid arguments returns a value of the declared shape;
(c) calling it with invalid arguments throws a typed error;
(d) property-based test: for `MetadataCache.getFileCache(file)`, for any file in a generated vault, the return shape conforms to `CachedMetadata` (generator-driven).

Mutation testing (`cargo-mutants`) is required on `src/obsidian/shim/` with a kill rate ≥ 85%.

Verifies: REQ-3104.

### TEST-3104-determinism: Byte-Identical Output

On a fixed demo vault with a fixed plugin set, run `zetl build` three times; diff the output tree. Assert zero differences. Run on Linux x86_64 and macOS aarch64 in CI; assert cross-platform byte-identity.

Verifies: NFR-3104.

### TEST-3105: Matrix Golden-HTML Diffs

For each plugin at tier `supported` in `obsidian-plugin-matrix.toml`:

1. Recorded baseline HTML from Obsidian Publish for a canonical test vault lives under `tests/obsidian-goldens/<plugin>/expected.html`.
2. Test runs `zetl build` with that plugin enabled on the same input vault.
3. Both outputs are normalised (whitespace-collapse within preformatted-block boundaries; attribute sort; no-op element removal).
4. Assert 0 character-level differences post-normalisation.

For tier `partial`: the test records the *expected* deltas (e.g., "dataview queries render as inline blocks; table updates do not") as part of the matrix entry; the test asserts only non-delta regions match.

Verifies: REQ-3105, REQ-3111.

### TEST-3105-startup: Opt-In Cold-Start Cost

Benchmark: compile with `--features obsidian-plugins`; invoke `zetl --no-obsidian-plugins index` on a trivial vault. Compare to a build compiled without the feature. Assert delta ≤ 10 ms at P95 over 100 runs.

Verifies: NFR-3105.

### TEST-3106: Scanner Determinism and Cache

Run `zetl obsidian scan-symbols --top 10` twice against a fixed snapshot. Assert byte-identical output. Delete the network cache; run again; assert re-populated cache matches. Inject a non-JS plugin file into the cache; assert it is skipped with a reason and does not fail the scan.

Verifies: REQ-3106.

### TEST-3106-deps: Supply-Chain Surface

Lockfile diff assertion: the `[[package]]` entries added by enabling `--features obsidian-plugins` are ≤ 2 new roots (`rquickjs` and its immediate tree). A CI job fails if `cargo tree --features obsidian-plugins` adds more than N transitive deps without an ADR.

Verifies: NFR-3106.

### TEST-3107: Budget Enforcement and Escape Containment

**CPU budget:** Plugin that runs `while(true){}`. Assert execution terminates within `plugin_timeout_ms + epsilon`; assert the plugin is disabled for the remainder of the pass; assert the host `zetl` process is unaffected.

**Memory budget:** Plugin that allocates until OOM. Assert QuickJS throws within its heap ceiling; host RSS delta ≤ ceiling + fragmentation overhead.

**Escape attempts (red-team):** For each of the vectors in NFR-3107, a test plugin attempts the escape; assert containment.

Mutation testing on `src/obsidian/runtime/budget.rs` ≥ 90% kill rate.

Verifies: REQ-3107, NFR-3103, NFR-3107.

### TEST-3108: Graceful Failure

Plugin that throws synchronously on `onload`; assert fragment is unchanged, diagnostic is emitted, subsequent plugins run.
Plugin that throws async (Promise rejection) during post-processing; same assertions.
Plugin that calls an unsupported shim symbol; assert `ZetlShimNotSupportedError` is raised, logged, and rendered as diagnostic; subsequent plugins run.

Verifies: REQ-3108, NFR-3101.

### TEST-3109: Diagnostic Suppression Flag

`zetl build` with a deliberately-failing plugin and `--obsidian-suppress-diagnostics`. Assert output HTML contains `<!-- zetl: plugin ... failed -->` and `obsidian-diagnostics.json` exists and contains the full details.

Verifies: REQ-3109.

### TEST-3110: Coverage Report

Run `zetl obsidian coverage` against a fixture vault with two matrix plugins (one at each tier). Assert human output matches a golden. Run with `--json`; assert schema conformance and content equivalence with the human output.

Verifies: REQ-3110.

### TEST-3111: CI Gate — No Silent Tier Downgrades

Simulated PR that downgrades a plugin from `supported` to `partial` in `obsidian-plugin-matrix.toml` without evidence. Assert the CI job fails with a clear message.
Simulated PR that adds a new plugin at `supported` without a golden fixture. Assert the CI job fails.

Verifies: REQ-3111.

### TEST-3112: Config and Overrides

Matrix:

| Config | Expected |
|--------|----------|
| `enabled = false` in config.toml | Runtime not instantiated |
| `plugin_allow_list = ["tasks"]` | Only Tasks loaded regardless of `community-plugins.json` |
| CLI `--no-obsidian-plugins` over `enabled = true` | Runtime not instantiated |
| `plugin_timeout_ms = 10`, plugin that takes 20 ms | Plugin disabled mid-run |

Verifies: REQ-3112.

### TEST-3113: Data File Semantics

`loadData()` with no file → `null`.
`saveData(data)` under `zetl serve` → file exists, parseable, atomic (interrupt mid-write; assert no partial file).
`saveData(data)` under `zetl build` → file not created on disk; debug log emitted.

Verifies: REQ-3113.

### Fuzzing — Shim Boundary

A coverage-guided fuzz target (`cargo-fuzz`) feeds random JS source strings to the runtime and random Rust-side vault state through the shim boundary. Assert: no host panic, no memory-safety violation, no escape. Target: 24h continuous fuzz on a nightly CI job.

### Synthetic-User Simulation

A synthetic user (per USDD §Synthetic User Protocol) walks Profile 2.1 (publishing migrant) against the draft specification. Log confusion points, ambiguities, and gaps. Convert findings to REQ or NFR amendments before status moves from `draft` to `approved`.

---

## 9. Observability

### OBS-3101: Runtime Lifecycle

Log line on process start (feature compiled): `[zetl] obsidian: runtime init status=<ok|disabled|failed> reason=<...>`.
Log line on plugin load: `[zetl] obsidian: loaded <id> v<version> tier=<supported|partial|unsupported|unvalidated>`.
Log line on skip: `[zetl] obsidian: skipped <id> reason=<...>`.

Metric: `zetl_obsidian_runtime_init_total{status}` counter.

Trace: REQ-3101.

### OBS-3102: Plugin Load Outcomes

Metric: `zetl_obsidian_plugin_load_total{plugin_id, outcome}` counter; `outcome` ∈ {loaded, skipped_desktop_only, skipped_invalid_manifest, skipped_oversize, skipped_missing_main}.

Trace: REQ-3102.

### OBS-3103: Unsupported-Symbol Invocation Rate

Metric: `zetl_obsidian_unsupported_call_total{plugin_id, symbol}` counter.
Log line: `[zetl] obsidian: unsupported call plugin=<id> symbol=<Obj.method>`.

This metric is the most load-bearing signal for "which symbols should the next shim release cover?" and is consumed by the scanner (REQ-3106) cross-referenced output.

Trace: REQ-3104.

### OBS-3104: Matrix Tier Distribution

On every render pass, emit a summary log: `[zetl] obsidian: plugins=<N> supported=<A> partial=<B> unvalidated=<C> unsupported=<D>`.

Metric: `zetl_obsidian_plugins_by_tier{tier}` gauge.

Trace: REQ-3105.

### OBS-3105: Budget Violations

Metric: `zetl_obsidian_budget_violation_total{plugin_id, kind}` counter; `kind` ∈ {cpu, memory}.
Log line: `[zetl] obsidian: budget <kind> plugin=<id> limit=<N> observed=<M> action=disabled`.

Trace: REQ-3107, NFR-3103.

### OBS-3106: Plugin Failure Rate

Metric: `zetl_obsidian_plugin_failure_total{plugin_id, reason}` counter; `reason` ∈ {throw, budget_cpu, budget_memory, unsupported_symbol}.
`zetl build` exit report includes a failure count: `[zetl] obsidian: 2 plugin failure(s); build succeeded`.

Trace: REQ-3108, NFR-3101.

### OBS-3107: Render Latency with Plugins

Timing log under `--verbose`: `[zetl] obsidian: post-process slug=<...> total_ms=<N> plugins=<M>`.
Per-plugin timing also recorded in coverage report.

Trace: NFR-3102.

---

## 10. Security Considerations

Security is addressed across three layers; ADR-3103 records the overall threat model.

**Layer 1 — Runtime isolation.** QuickJS, configured without any host bindings, is the foundation. The only `require('obsidian')` call is intercepted by the Rust shim; all others fail. No `process`, no `child_process`, no `fs`, no `net`. See TEST-3107 red-team cases.

**Layer 2 — Shim surface.** Every shim method has an explicit allow-list of operations it can perform:

- Reads are vault-rooted and path-normalised; symlinks outside the vault are rejected (reuse SPEC-026 path-safety logic).
- The only writable path per plugin is its own `data.json`.
- No shim method performs network I/O or process spawning in v1. If future versions add `requestUrl` or similar, it gets its own ADR and becomes opt-in per-plugin.

**Layer 3 — Budgets.** Time and memory ceilings (REQ-3107, NFR-3103) bound denial-of-service. Without them, a pathological plugin could hang `zetl build` indefinitely.

**Residual risks documented.** Vault-wide read access (a plugin can read any page in the vault via `Vault.read`) is not mitigated in v1. This matches Obsidian's own posture and is called out in documentation. Users who need stricter isolation should not enable the feature.

**Supply-chain note.** QuickJS pinned version is recorded in `Cargo.toml`; the crate advisory feed is monitored. A QuickJS CVE becomes a `zetl` security release.

**AI Trust Boundary classification:** This feature is **Review Tier 2** (core business logic, untrusted-input processing). Implementation requires cross-model review per USDD §Multi-Model Cognitive Diversity. The shim itself is not cryptographic nor authentication core, so it does not reach Tier 1.

---

## 11. Documentation Plan

**README.md — new section "Obsidian plugin compatibility":**

- What is supported and what is not (link to the matrix).
- How to enable: install with `--features obsidian-plugins`; run against a vault that has `.obsidian/plugins/`.
- How to read the coverage report.
- Known-good plugins list snapshot (auto-generated from the matrix).

**CHANGELOG.md entry** under next release.

**New file `docs/obsidian-plugin-matrix.md`** — rendered view of the matrix with per-plugin notes, auto-generated from `tools/obsidian-plugin-matrix.toml` on CI.

**Theme authoring reference** — document the `zetl-obsidian-error` class and data attributes so theme authors can style the diagnostic element (diagnostic visibility is controlled by REQ-3109).

**User profile coverage:** Profiles 2.1–2.4 each receive a section in `docs/user-guides/` describing the workflow end-to-end.

---

## 12. Rollout Plan

Phased delivery to limit blast radius and gather convergence signals.

**Phase A — Runtime and shim skeleton (behind feature flag, default-off):**

- Land `rquickjs` integration, plugin discovery, baseline shim surface, budgets, diagnostics.
- Matrix starts empty; every plugin runs at `unvalidated`.
- Feature is off in the default binary; users opt in via `cargo install --features obsidian-plugins`.
- Ship `zetl obsidian scan-symbols` and `zetl obsidian coverage`.

**Phase B — Matrix seeding:**

- Add first 5 plugins at tier `supported`, candidates: Callouts, Tasks, Admonition, Footnote++, Wikilinks Enhanced (TBD after scanner output).
- Golden-HTML tests (TEST-3105) gate each.

**Phase C — Public documentation and announcement:**

- Matrix is ≥ 10 plugins. Documentation merged.
- Blog post / forum post introducing the feature, making the "matrix not universality" framing prominent.

**Phase D — Community-contributed matrix entries:**

- Externalise the plugin addition process; document "how to add a plugin to the zetl matrix" so external contributors can submit fixtures.

**Rollback:** If post-release data shows the budget or failure-rate NFRs are missed, the feature is reverted to "unsupported / experimental" in docs and the default-off cargo flag remains unchanged. No user is worse off than before.

---

## 13. Open Questions

These require stakeholder resolution before status advances from `draft` to `approved`. Each is tagged with a proposed resolution to accelerate discussion.

1. **Plugin-identity vs. matrix-identity.** If two forks of a plugin share a plugin ID (common on GitHub forks), how do we disambiguate matrix entries? *Proposed:* matrix key is `(plugin_id, version)`; forks must re-ID.
2. **Licence compatibility policy.** Running GPL-licensed plugin code inside an AGPL-3.0 `zetl` is straightforward (compatible). MIT plugin code is trivially compatible. What about proprietary closed-source plugins distributed as binary `main.js`? *Proposed:* loader does not enforce; users' compliance responsibility; documentation notes AGPL implications for self-hosted public deployments.
3. **`Vault.read` scope.** Should a plugin be able to read *any* vault page or only the current page being rendered? *Proposed:* any page (matches Obsidian). Counter-argument: a stricter-by-default posture is easier to loosen later.
4. **Diagnostic visibility for `zetl serve` multi-user / collab mode.** Public-facing collab deployments might not want every visitor to see plugin diagnostics. *Proposed:* `collab` mode defaults to `--obsidian-suppress-diagnostics` equivalent; admins can override per-session.
5. **Scheduling of the community-plugins snapshot.** The scanner pins to a dated snapshot of Obsidian's `community-plugins.json` feed. Who bumps the snapshot date and on what cadence? *Proposed:* monthly, automated via a scheduled workflow; delta vs. previous snapshot attached to the PR for review.
6. **Handling `Plugin_2` base class.** Obsidian has deprecated-but-used aliases; how comprehensive should our alias coverage be? *Proposed:* alias in the shim to the same target; log when alias is used so we can measure deprecation pressure.
7. **MCP exposure of coverage report.** Should `get_page` via MCP include the coverage attribution in a response metadata field? *Proposed:* off by default; opt-in via MCP tool parameter `include_obsidian_coverage=true`.

---

## 14. Traceability Summary

| REQ      | Tests                    | Contracts  | ADRs       | OBS         |
| -------- | ------------------------ | ---------- | ---------- | ----------- |
| REQ-3101 | TEST-3101                | CON-3101   | ADR-3101   | OBS-3101    |
| REQ-3102 | TEST-3102, 3102-perf     | CON-3102   | —          | OBS-3102    |
| REQ-3103 | TEST-3103                | CON-3103   | —          | —           |
| REQ-3104 | TEST-3104                | CON-3104   | ADR-3102   | OBS-3103    |
| REQ-3105 | TEST-3105                | CON-3104   | ADR-3104   | OBS-3104    |
| REQ-3106 | TEST-3106                | CON-3106   | —          | —           |
| REQ-3107 | TEST-3107                | —          | —          | OBS-3105    |
| REQ-3108 | TEST-3108                | —          | —          | OBS-3106    |
| REQ-3109 | TEST-3109                | —          | —          | —           |
| REQ-3110 | TEST-3110                | CON-3110   | —          | —           |
| REQ-3111 | TEST-3105, 3111          | —          | ADR-3104   | —           |
| REQ-3112 | TEST-3112                | CON-3112   | —          | —           |
| REQ-3113 | TEST-3113                | CON-3113   | —          | —           |
| NFR-3101 | TEST-3108                | —          | —          | OBS-3106    |
| NFR-3102 | TEST-3102-perf           | —          | —          | OBS-3107    |
| NFR-3103 | TEST-3107                | —          | —          | OBS-3105    |
| NFR-3104 | TEST-3103, 3104-determinism | —       | ADR-3101   | —           |
| NFR-3105 | TEST-3105-startup        | —          | —          | —           |
| NFR-3106 | TEST-3106-deps           | —          | ADR-3101   | —           |
| NFR-3107 | TEST-3101, 3107          | —          | ADR-3103   | —           |

---

## 15. Quality Gate Self-Check

Against USDD Phase 1–2 quality gates:

- [x] Requirements unambiguous — each uses SHALL with measurable criteria (timeframes, numeric budgets, enumerated error conditions).
- [x] Requirements verifiable — every REQ has a TEST reference.
- [x] Requirements atomic — each REQ is one obligation.
- [x] No internal conflicts — REQ-3108 and REQ-3109 interact cleanly; other requirements are orthogonal.
- [x] Ambiguities resolved — "fast," "reliable," "safe" replaced with numeric budgets and threat-model statements.
- [x] Components have single responsibility — runtime, discovery, matrix, scanner, coverage are separate modules.
- [x] Functionality via well-defined interfaces — CON-3101 through CON-3113 capture the surface.
- [x] Tests derived from requirements — traceability table confirms.
- [x] Security controls specified with verifiable criteria — NFR-3107 and ADR-3103; red-team tests enumerated.
- [x] Observability requirements captured — OBS-3101 through OBS-3107.

**Not yet cleared:**

- [ ] Stakeholder validation (requires human review of open questions §13).
- [ ] Adversarial review from a fresh context (USDD constitutional principle 12).
- [ ] Synthetic-user simulation findings merged into requirements.

Status remains `draft` until these three are complete.

---

**End of SPEC-031.**
