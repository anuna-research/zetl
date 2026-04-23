---
title: "SPEC-032: Three-Stage Render Hooks with First-Class File Selection"
version: 0.1.0
status: draft
date: 2026-04-19
audience: agent, human
supersedes: SPEC-031
parent: SPEC-004
related:
  - SPEC-004  # Web UI and static export (render pipeline integration)
  - SPEC-026  # Vault scanning and ignore files (selection layer interaction)
  - SPEC-027  # History UI (extension-data durability concerns)
  - SPEC-028  # Interactive graph view (theme-layer precedent, versioned contract)
  - SPEC-031  # Obsidian plugin shim (superseded; scan findings retained as justification)
---

# SPEC-032: Three-Stage Render Hooks with First-Class File Selection

## Information Table

| Field        | Value                                                                   |
| ------------ | ----------------------------------------------------------------------- |
| Document ID  | SPEC-032                                                                |
| Title        | Three-Stage Render Hooks with First-Class File Selection                |
| Version      | 0.1.0                                                                   |
| Status       | Draft                                                                   |
| Author       | Agent (USDD Protocol v1.3.0)                                            |
| Date         | 2026-04-19                                                              |
| Audience     | Agent, Human                                                            |
| Trace        | USDD Agent Protocol v1.3.0                                              |
| Parent       | SPEC-004: Web UI and static export                                      |
| Supersedes   | SPEC-031 (Obsidian plugin shim — retained for historical record)        |
| Related      | SPEC-026 (scanning), SPEC-027 (history), SPEC-028 (theme contract)      |
| Dependencies | Existing hook system; Markdown parser; Minijinja render pipeline         |

---

## 1. Overview

`ztl` already ships a lifecycle hook system — executable scripts at `pre-build`, `post-build`, `post-index`, `on-save`, and others, receiving JSON context on stdin. What it does *not* have is a per-page, per-render extension surface: a way for users (or first-party canonical extensions shipped with the default theme) to transform a page's content during rendering, not after it. This gap is what users ask for when they ask for "Obsidian plugin support" — the *published output* of post-processor plugins like Callouts, Tasks, and Admonition.

SPEC-031 explored running actual Obsidian plugins under an embedded JavaScript runtime. A top-50 community plugin scan (2026-04-19) showed that bet was worse than it looked: zero top-50 plugins are pure `MarkdownPostProcessor` (the only class a bounded v1 shim could render), 24% are hybrids whose post-processor is entangled with editor extensions, and 76% are unusable in any shim. The engineering cost (3–6 months, QuickJS supply chain, shim surface chasing a moving API) did not justify the coverage.

This spec takes the opposite direction: ztl defines its own render-pipeline extension contract, keeps Obsidian compatibility as a matter of *visual parity for canonical patterns* rather than *code execution*, and ships the patterns users actually want (Callouts, Tasks, Admonition) as first-party extensions against that contract.

The contract has three stages, each with a different data shape reflecting the stage's responsibility:

| Stage         | Data in            | Data out           | Typical use                                          |
| ------------- | ------------------ | ------------------ | ---------------------------------------------------- |
| `pre-parse`   | raw Markdown text  | raw Markdown text  | Templater-style preprocessing, includes, variables   |
| `transform`   | ztl-AST JSON      | ztl-AST JSON      | Callouts, Tasks, Admonition, Dataview-subset         |
| `post-render` | HTML fragment      | HTML fragment      | DOM injection, analytics, external-CSS-class fixups  |

Each stage supports composition via directory-of-drop-ins, theme-then-vault precedence, and local failure scoping. **File selection is first-class**: every hook declares a selector (path globs, frontmatter predicates, cheap content probes) that ztl evaluates before invoking the hook, so extensions skip the pages they don't care about without paying serialisation cost or process-spawn overhead.

### 1.1 Motivation

- **The Obsidian-migrant user (SPEC-031 Profile 2.1) still needs answering.** They want Callouts, Tasks, Admonition rendering when they publish their vault with `ztl build`. The scan showed those patterns are small, tractable, and author-able in under 200 lines each.
- **Operate on the right abstraction.** Text hooks (SPEC-031 ADR-3102 alternative) get fooled by code blocks and quoted prose. HTML hooks fight the renderer. AST hooks match the *structure* we mean — BlockQuote-whose-first-paragraph-starts-with-`[!note]` — making extensions reliable across renderer changes.
- **First-class selection = real performance.** In a 2,000-page vault with 10 enabled extensions, running every extension on every page is 20,000 invocations. Most of those are pages the extension doesn't touch. A declared selector evaluated cheaply (path match → frontmatter probe → content substring probe) reduces that to the actual working set, often 100× smaller.
- **Selection is also semantics.** "This folder runs the Tasks extension, that folder doesn't" is a real governance boundary in team vaults. Per-file frontmatter opt-out (`extensions.callouts: false`) is how users escape-hatch individual pages. Both are user-visible contracts, not implementation details.
- **Protocol stability, not shim stability.** Defining our own versioned AST JSON schema couples us to CommonMark (stable) rather than Obsidian's API (moving). Maintenance cost is in a schema we control.
- **Out-of-process is ergonomic for hook authors.** Python, Node, Ruby, Go, Rust — any language with a JSON library and an HTTP-stream-of-consciousness mental model can author a hook. No cargo feature flag, no QuickJS upgrade risk. First-party helper libraries (`ztl-ast-py`, `ztl-ast-js`) close most of the ergonomic gap with in-process APIs.

### 1.2 Design Principles

1. **Operate on the right stage.** Text for preprocessing (where the AST doesn't exist yet), AST for structural transforms (the abstraction we mean), HTML for DOM-level concerns (analytics, external-tool classnames). Refuse to collapse them into one.
2. **Selection is first-class.** Every hook has a selector. Zero-selector (runs on everything) is explicit and warned, not the default. Selectors are declarative, file-based, and evaluable without invoking the hook.
3. **Declarative over imperative.** Manifests describe stage, selector, mode, budget. ztl orchestrates; hooks transform.
4. **Versioned contracts, not best-effort APIs.** The AST JSON schema has a semver. Helper libraries pin a major. Breaking changes require a major bump and a migration path.
5. **Out-of-process by default.** Hook failures, OOM, and crashes cannot damage `ztl`'s process. The ergonomic gap is closed by helper libraries, not by in-process execution.
6. **Composition by piping, not by coordination.** Each hook is a filter. Failures are scoped to one hook; the chain continues. No hook-to-hook communication primitives in v1.
7. **Theme-shipped extensions are first-class.** The default theme ships canonical extensions (Callouts, Tasks, Admonition) in `themes/default/hooks/`. Users override by filename collision or disable by empty file. Same mechanism as existing theme-bundled hooks.
8. **Build/serve parity.** A page rendered under `ztl build` at time T with a fixed extension set produces the same HTML as `ztl serve` rendering that same page at time T with the same extension set. Extensions must be deterministic; the render pipeline is.
9. **Obsidian compatibility is a by-product, not a goal.** The three stages are designed for ztl's own extension needs. That the same architecture happens to re-implement the Obsidian patterns users want is confirmation, not constraint.

### 1.3 Scope

**In scope:**

- Three new hook stages: `pre-parse`, `transform`, `post-render`, each with one-shot and persistent execution modes.
- A ztl-AST JSON schema: CommonMark subset + wikilink/embed/SPL-block extensions + frontmatter + source positions. Versioned.
- Helper libraries for Python (`ztl-ast-py`) and JavaScript/TypeScript (`ztl-ast-js`), published alongside the ztl release, pinned to the AST schema major.
- A hook manifest format (`<hook-path>.toml`) declaring stage, selector, mode, timeout, memory limit.
- Selector DSL: path globs (include/exclude), frontmatter predicates (dotted-path plus operator), content probes (substring or regex on raw text), with a defined precedence for cheap-to-expensive evaluation.
- Composition: `.d/` directory discovery, filename-sorted piping, theme-then-vault precedence, user-overrides-theme by filename collision.
- Persistent-mode line-delimited JSON protocol (spec'd in CON-3201).
- Subcommand `ztl hook dry-run <stage>/<name> [--vault PATH]` — evaluates the selector, prints matched pages, does not invoke the hook. For authoring iteration.
- Subcommand `ztl hook coverage [--vault PATH] [--json]` — reports, for the last build, which hooks matched which pages, average latency, failure counts.
- Failure semantics: local to one hook, chain continues with previous hook's output.
- Three first-party canonical extensions shipped with the default theme: `callouts`, `tasks`, `admonition`. Each has a golden-HTML test and a published selector.
- Documentation: new README section, theme-authoring updates, AST schema reference, helper-library quickstart, migration guide from existing hooks to the new stages.
- Observability: per-hook latency, match-rate, failure-rate metrics; `--verbose` trace output per invocation.

**Out of scope:**

- In-process execution (embedded scripting language). Reserve for a successor spec if live-hackability becomes a product goal.
- AST-level hooks with parser-native formats (pulldown-cmark events, rustdoc-markdown, etc.). The ztl-AST JSON is a stable contract; the underlying parser may change.
- Hook-to-hook communication / shared state. Each hook is a pure filter.
- Automatic helper-library generation for languages beyond Python and JS in v1. Ruby, Go, Rust-as-host-language bindings can be community contributions.
- A plugin marketplace or community-extension registry. Hooks live in users' vaults or themes.
- Automatic migration from SPEC-031-era Obsidian-plugin directories (`.obsidian/plugins/`). Users who had SPEC-031's feature flag enabled (there is no such user yet) get a one-line release note; vault layout is unchanged.
- Extending SPL (the reasoning language) with imperative forms. SPL stays declarative; extensions are a separate concern.

---

## 1.4 Prior Art

SPEC-032's three-stage render-hook system sits squarely inside a well-established design space, and its main shape choices are validated by prior practice. The full survey lives at `tools/parser-lit-survey.md`; the load-bearing findings:

**The core protocol — out-of-process hooks communicating over line-delimited JSON — is exactly Pandoc's filter model** ([Pandoc filters](https://pandoc.org/filters.html)) and mdBook's preprocessor model ([mdBook preprocessors](https://rust-lang.github.io/mdBook/for_developers/preprocessors.html)). Both ecosystems cite multi-language extensibility as the key payoff, and both grew healthy plugin ecosystems on the strength of it. Pandoc explicitly supports five extension languages through an interpreter fallback table; mdBook's docs devote a section to Python plugins. SPEC-032's decision to ship Python and JS helper libraries mirrors Pandoc's `pandocfilters` and `Text.Pandoc.JSON`, which are widely credited as the lubricant that made Pandoc's filter ecosystem scale.

**The three-stage pipeline — `pre-parse` text→text, `transform` AST→AST, `post-render` HTML→HTML — is the same shape as unified's parse / run / stringify model** ([unified](https://github.com/unifiedjs/unified)), which powers remark, rehype, and retext. Unified is the largest plugin ecosystem in this space (thousands of published plugins) and converged on exactly this three-phase factorisation. markdown-it by contrast uses a four-chain token-stream model; its own docs defend this as a KISS alternative ([markdown-it architecture](https://github.com/markdown-it/markdown-it/blob/master/docs/architecture.md)) but acknowledge it costs you the AST surface area third-party tooling expects.

**SPEC-032's novel contributions** — first-class file selection (globs + frontmatter + content probe) and a reserved `page.ext.<extension_id>` template namespace — are genuine improvements over prior art. No priority-1 system offers declarative pre-dispatch file scoping (Pandoc, mdBook, markdown-it all apply every registered extension to every node); the closest precedent is mdBook's coarse `renderers = ["html"]` binding. The namespaced template surface is without precedent: Pandoc and mdBook expose no template variables from filters/preprocessors, and unified's `file.data` / markdown-it's `env` have no namespace discipline, leading to collision-by-convention.

**Six concrete practices imported from prior art** (each has its own REQ):

1. **Split binary version from AST-schema version** (REQ-3215) — Pandoc exposes `PANDOC_VERSION` and `PANDOC_API_VERSION` separately so filters survive non-AST-breaking upgrades.
2. **Capability probe before real invocation** (REQ-3216) — mdBook's `supports <renderer>` / exit-code pattern.
3. **Explicit `before` / `after` ordering with cycle detection** (REQ-3217) — mdBook's ordering system.
4. **`optional: true` for missing hooks** (REQ-3217) — demotes missing executable to warning.
5. **Declarative node-type dispatch** in the helper libraries (REQ-3218) — Pandoc Lua filters' `{ Str = fn, Heading = fn, Inline = fallback }` model is dramatically less error-prone than visit-all-and-filter.
6. **A shared build-scoped key/value channel between hooks** (REQ-3219) — unified's `processor.data()` and markdown-it's `env` both exist because cross-hook coordination through frontmatter/filesystem is fragile.

**Gotchas documented by prior systems and addressed in this spec:** version-drift policy (CON-3201), `transform` return-type contract (CON-3201), default AST traversal order (CON-3202), sandboxing/trust posture (§10), and the `pre-parse` roundtripping caveat (REQ-3201 §note). See `tools/parser-lit-survey.md` §5 for the full inventory and citations.

---

## 2. User Profiles and Happy Paths

### 2.1 User: Obsidian migrant publishing a static site

**Role:** Existing Obsidian user with a 500-page vault using Callouts, Tasks, and Admonition syntax. Wants to publish via `ztl build`.
**Goal:** Published HTML visually matches what they see in Obsidian Publish for those three features.
**Constraints:** No willingness to edit Markdown source; CI pipeline; prefers zero configuration.

**Happy path:**

1. User runs `ztl build --out-dir dist --theme default`.
2. Default theme ships `themes/default/hooks/transform.d/10-callouts.py`, `20-tasks.py`, `30-admonition.py` with selectors pre-configured (Callouts runs on any page containing `> [!`; Tasks runs on any page with a `tasks` fenced block; Admonition on any with `ad-*` fenced blocks).
3. Output renders visually equivalent to Obsidian Publish for these three patterns. `dist/obsidian-diagnostics.json` records per-extension match counts.
4. `ztl hook coverage --vault .` confirms coverage: `callouts matched 87/500, tasks matched 12/500, admonition matched 4/500`.

**Failure modes:**

- A page has malformed callout syntax → callouts extension leaves the block unchanged, diagnostic logged, build continues.
- User has vault-level opt-out of a canonical extension → they create an empty `.ztl/hooks/transform.d/10-callouts.py` which shadows (by filename collision) the theme-shipped one; that extension is effectively disabled.

### 2.2 User: Vault author writing a custom transform

**Role:** Power user who wants an extension that converts `{{cite:bibkey}}` inline tokens into formatted citations pulled from a BibTeX file.
**Goal:** Ship a 100-line Python script that handles this cleanly, without fighting the render pipeline.
**Constraints:** Comfortable in Python; wants AST-level access (to avoid mangling code blocks or URLs); wants the extension to run only on pages with `bibliography` in frontmatter.

**Happy path:**

1. User writes `.ztl/hooks/transform.d/citations.py` using `ztl-ast-py`:
   ```python
   from ztl_ast import run, walk, Text
   def transform(ast, context):
       for node in walk(ast, type=Text):
           node.text = expand_cites(node.text, context.frontmatter["bibliography"])
       return ast
   run(transform)
   ```
2. User writes `.ztl/hooks/transform.d/citations.toml`:
   ```toml
   stage = "transform"
   frontmatter_where = "bibliography != null"
   mode = "persistent"
   timeout_ms = 200
   ```
3. `ztl hook dry-run transform/citations` prints 23 pages matched, zero invocations (selector only).
4. `ztl build` invokes the hook only on those 23 pages.
5. User iterates by editing Python; persistent-mode restart is automatic on file change in `ztl serve`.

**Failure modes:**

- Hook crashes on a specific page → that page's AST is passed unchanged to the next hook, diagnostic logged.
- Selector matches zero pages → warning logged once: `hook citations matched 0 pages; did you mean ...?`.

### 2.3 User: Theme author

**Role:** Customising `.ztl/themes/scholar/` for an academic vault.
**Goal:** Ship a theme that includes citation rendering, disables Tasks (academic vaults don't need task dashboards), and modifies Callouts styling.
**Constraints:** Only edits files inside `.ztl/themes/scholar/`; no Rust.

**Happy path:**

1. Theme ships `themes/scholar/hooks/transform.d/10-callouts.py` (copied from default theme, restyled).
2. Theme ships empty `themes/scholar/hooks/transform.d/20-tasks.py` (empty file = disabled).
3. Theme ships `themes/scholar/hooks/transform.d/15-citations.py` (new extension between callouts and tasks-would-be).
4. User of this theme runs `ztl build --theme scholar`; citations run, tasks doesn't, callouts runs with scholar's styling.

**Failure modes:**

- Theme's Python script errors on an edge case → per-page diagnostic; theme author iterates using `ztl hook dry-run --theme scholar`.

### 2.4 User: CI operator enforcing extension coverage

**Role:** Team running `ztl build` in CI for a collaboratively-maintained vault.
**Goal:** Fail the build if any enabled canonical extension (Callouts, Tasks, Admonition) crashes, even if ztl's default is to soft-fail with diagnostic.
**Constraints:** Strict build semantics; wants to gate merges on extension health.

**Happy path:**

1. CI invokes `ztl build --hook-fail-on error` (new flag).
2. Build fails with exit code 2 if any hook exits non-zero; the diagnostic page list is printed to stderr.
3. CI operator receives actionable output: `hook callouts failed on pages: projects/q2-review.md, notes/daily-2026-04-18.md`.

**Failure modes:**

- Legitimate transient failure (hook OOM on one huge page) → CI flake; mitigation is per-page retry at the hook-runtime level (out of scope for v1; document).

---

## 3. Functional Requirements

### REQ-3201: Three Hook Stages

The system SHALL expose three new hook stages in the render pipeline:

- **`pre-parse`** — invoked after the raw Markdown source is read from disk, before frontmatter parsing or Markdown AST construction. Input/output: UTF-8 Markdown text.
- **`transform`** — invoked after Markdown parsing produces the ztl-AST, before AST-to-HTML rendering. Input/output: ztl-AST JSON.
- **`post-render`** — invoked after AST-to-HTML rendering produces the per-page content fragment, before Minijinja template composition into the full page. Input/output: HTML fragment string.

Stage ordering within a single page render is fixed: `pre-parse` → parse → `transform` → render → `post-render` → template compose.

**Pre-parse caveat (author guidance):** The `pre-parse` stage operates on raw Markdown source text. The hook author is responsible for preserving Markdown's block/inline structure — naive regex edits can easily break downstream parsing. mdBook's own documentation warns: "chapter.content is just a string which happens to be markdown. While it's entirely possible to use regular expressions or do a manual find & replace, you'll probably want to process the input into something more computer-friendly" ([mdBook dev guide](https://rust-lang.github.io/mdBook/for_developers/preprocessors.html)).

- **Valid use cases:** Template-variable expansion (`{{ page.title }}`), include-directive resolution, syntax-sugar replacement where the input/output are both Markdown-valid.
- **Anti-patterns:** Adding emphasis or block structure by find-replace (use a `transform` hook instead); naive HTML injection (breaks on being subsequently parsed as Markdown); regex matching of `[[wikilinks]]` without respecting code-block fences.

When in doubt, authors should prefer `transform` stage — it operates on the parsed AST where context (inside-code-block, inside-link, etc.) is explicit.

**Note on mdBook preprocessors:** SPEC-033 places mdBook preprocessors
(ecosystem = `mdbook`) at the `pre-parse` stage (SPEC-033 REQ-3304,
ADR-3303) because mdBook's contract operates on raw chapter text. Those
preprocessors are authored under mdBook's own semantics and often use
regex-level substitution (e.g. `mdbook-mermaid` replaces `` ```mermaid ``
fences with inline SVG). ztl inherits mdBook's safety posture at that
stage — the caveat above is a property of Markdown-as-string processing,
not a defect in either ecosystem. Authors targeting mdBook ecosystem see
it through mdBook's docs; authors writing native ztl pre-parse hooks
see it above. Same risk surface, two documentation paths.

Trace:
- TEST-3201
- CON-3201
- SPEC-033 ADR-3303 (mdBook stage placement)

### REQ-3202: ztl-AST JSON Schema

The system SHALL define and publish a versioned JSON schema (`ztl-ast-schema`) for the intermediate AST representation used by the `transform` stage. The schema SHALL cover:

- All CommonMark block types: `Document`, `Heading`, `Paragraph`, `BlockQuote`, `List`, `ListItem`, `CodeBlock` (fenced and indented), `ThematicBreak`, `HtmlBlock`.
- All CommonMark inline types: `Text`, `Emphasis`, `Strong`, `Code`, `Link`, `Image`, `LineBreak`, `SoftBreak`, `HtmlInline`.
- ztl extensions: `Wikilink` (with target, alias, heading, block-id fields), `Embed` (for `![[...]]` transclusions), `SplBlock` (for `spl` fenced code blocks, if any are to be treated specially), `FrontMatter` (parsed YAML as JSON object at the document root).
- Source positions: every node has `start_line`, `start_col`, `end_line`, `end_col`.
- Schema version: every document declares `ast_version: "<major>.<minor>"` at the root. This is an **exact** two-component version string emitted by ztl (e.g., `"1.0"`, `"1.2"`). In a hook *manifest* (REQ-3203), the same key accepts an **npm-style semver range** (e.g., `">=1.0 <2"`) interpreted against the schema version above — see REQ-3215 for the version-drift policy. The dual meaning is intentional: the schema emits a point version; manifests declare compatible ranges.

The schema SHALL be published at `tools/ztl-ast-schema-v1.json` in JSON Schema Draft 2020-12 format, and SHALL be used by the helper libraries (REQ-3210) as the source of truth.

**Human-readable reference:** `docs/ztl-ast-reference.md` SHALL be
auto-generated from the schema at CI time, with one section per
node type covering shape, attrs, canonical example, and
HTML-rendering expectations. Plugin authors should not have to
read a JSON Schema file to learn the AST. The generator lives at
`tools/ztl-ast-reference-gen/` (Rust) and runs in CI; a
discrepancy between schema and generated reference is a CI
failure.

Trace:
- TEST-3202
- CON-3202
- ADR-3201

### REQ-3203: Hook Manifest Format

Every hook executable in a `<stage>.d/` directory SHALL have an optional sibling manifest file named `<executable>.toml`. The manifest declares stage metadata, selector, execution mode, and budgets:

```toml
# .ztl/hooks/transform.d/tasks.toml
stage = "transform"                 # enum: "pre-parse" | "transform" | "post-render"
mode = "persistent"                 # enum: "one-shot" | "persistent"; default "one-shot"
timeout_ms = 100                    # int; default 100
memory_mib = 64                     # int; default 64
ast_type = "ztl-ext"               # enum: "ztl-ext" | "pandoc-ext" | "mdast-ext"; default "ztl-ext"
ast_version = ">=1.0 <2"            # semver range; schema version for the declared ast_type

[select]
include = ["**/*.md"]               # array of globs; default ["**/*.md"]
exclude = []                        # array of globs; default []
frontmatter_where = "..."           # predicate expression (REQ-3204); optional
content_probe = []                  # array of regex or substring probes; optional
require_probe_match = "any"         # enum: "any" | "all"; default "any"
```

A hook with no manifest MUST still work — ztl treats missing manifest as all defaults and `select.include = ["**/*.md"]`. Hooks without a manifest SHALL emit a warning recommending one.

Trace:
- TEST-3203
- CON-3203

### REQ-3204: Selector Evaluation Order

For each page, the system SHALL evaluate each enabled hook's selector in the following order, short-circuiting on the first failure:

1. **Path match** — `include` globs must match AND no `exclude` glob matches. Evaluated in microseconds against the page's vault-relative path.
2. **Frontmatter predicate** — `frontmatter_where` expression (REQ-3205) evaluates to true. Requires frontmatter parsing only; no full Markdown parse.
3. **Content probes** — `content_probe` entries (substring or regex) are tested against the raw Markdown text. Evaluated with the `require_probe_match` policy (any or all).
4. **Stage-specific data materialisation** — only if all above pass: parse to AST (for `transform`) or run preceding pipeline stages (for `post-render`).
5. **Hook invocation** — dispatch the (possibly persistent) process.

Selection results SHALL be recorded for the `ztl hook coverage` report (REQ-3208) regardless of whether the hook was subsequently invoked.

Trace:
- TEST-3204
- CON-3204
- OBS-3203

### REQ-3205: Frontmatter Predicate Syntax

The `frontmatter_where` field SHALL accept a predicate expression with the following grammar:

```
expr     ::= term (("&&" | "||") term)*
term     ::= path op value | path ("is null" | "is not null") | "!" term | "(" expr ")"
path     ::= IDENT ("." IDENT | "[" INT "]")*
op       ::= "==" | "!=" | "<" | "<=" | ">" | ">=" | "contains" | "matches"
value    ::= STRING | INT | FLOAT | BOOL | "null"
```

Examples:
- `tags contains "project"`
- `status == "published" && !draft`
- `frontmatter.extensions.tasks != false`
- `word_count > 500`
- `title matches "^Daily.*"`

The predicate is evaluated in a pure sandbox with no filesystem, network, or shell access. Unknown paths resolve to `null`. Type coercion is strict (`"5" != 5`).

Trace:
- TEST-3205
- CON-3205

### REQ-3206: Composition — Directory Drop-Ins

For each stage, the system SHALL discover hook executables from (in precedence order, highest first for tie-breaking by filename collision):

1. `<vault>/.ztl/hooks/<stage>.d/*` (vault hooks)
2. `<theme-dir>/hooks/<stage>.d/*` (theme-bundled hooks)

Executables within each directory SHALL be sorted by filename (lexicographic), and the combined list SHALL be the pipeline order. A vault hook with the same filename as a theme hook SHALL replace the theme hook (not merge).

A hook SHALL be treated as **disabled** (theme-hook shadowed without
invocation) when any of the following holds:

1. The file is `0` bytes.
2. The file is not executable AND has no shebang line (cannot be run).
3. The file's probe (REQ-3216) returns `{"ready": false}` or exits
   non-zero, OR the probe times out (> 5 s).

The earlier "shebang-only with no body" heuristic is dropped — it was
fooled by `#!/usr/bin/env true` (non-empty, technically runs, but
does nothing useful). The probe-based check in (3) is the robust
signal; (1) and (2) are cheap pre-probe filters that avoid even
starting the hook process.

Single-file hooks at `.ztl/hooks/<stage>` (not in a `.d/` directory) SHALL continue to work — treated as a one-entry `.d/` directory.

Trace:
- TEST-3206
- CON-3206

### REQ-3207: Failure Scoping and Pipeline Continuation

For a given page and stage, when hook `H_k` in the ordered pipeline fails (non-zero exit, timeout, memory overrun, malformed output), the system SHALL:

- Discard `H_k`'s output.
- Record a failure diagnostic with `plugin_id=filename`, `stage`, `page_slug`, `reason`, `duration_ms`.
- Pass `H_k`'s **input** to `H_{k+1}` (i.e., the output of `H_{k-1}`, or the stage input if `k == 0`).
- Continue the pipeline. One hook's failure does not cascade.

Under `ztl build --hook-fail-on error`, the build SHALL exit non-zero after rendering is complete if any hook failed, with an actionable summary to stderr. Default behaviour SHALL be `--hook-fail-on never`.

Trace:
- TEST-3207
- CON-3207
- OBS-3205

### REQ-3208: `ztl hook coverage` Subcommand

The system SHALL provide `ztl hook coverage [--vault PATH] [--json] [--stage STAGE]` that, for the most-recent build (or a fresh dry-run if none exists), reports per hook:

- Stage, manifest path, matched-page count, invoked-page count (may differ if selector passed but hook failed early), latency P50/P95, failure count, last failure reason.

Output defaults to table; `--json` emits structured output.

**Persistence semantics:** `hook-coverage.json` is **replaced** (not
merged) on each `ztl build` invocation — a build records its own
pass only. CI sees exactly the current run's coverage. Serve-mode
coverage is in-memory and cleared on restart.

A future `--coverage-append` flag for cumulative stats across builds
is deferred to §13 Open Questions.

Trace:
- TEST-3208
- CON-3208

### REQ-3209: `ztl hook dry-run` Subcommand

The system SHALL provide `ztl hook dry-run <stage>/<name> [--vault PATH] [--limit N]` that evaluates the hook's selector against the vault and prints the matched page list (up to `--limit`, default 50). The hook itself SHALL NOT be invoked. Exit code 0 if any pages matched; 1 if zero matched (to aid CI "is this selector reachable" checks).

Trace:
- TEST-3209

### REQ-3210: Helper Libraries

The system SHALL publish two first-party helper libraries alongside each ztl release:

- **`ztl-ast-py`** — Python 3.9+ package on PyPI. Provides `run(transform_fn)` entry point, typed node classes (`Document`, `Paragraph`, `BlockQuote`, `Wikilink`, …), `walk(ast, type=Foo)` iterator, and a `context` object exposing page metadata.
- **`ztl-ast-js`** — npm package (Node 18+ and Deno). Equivalent API.

Both libraries SHALL pin to a specific AST schema major and SHALL refuse to run against a mismatched ztl version (fail fast with a clear error). The `run()` entry point SHALL handle both one-shot (stdin→stdout→exit) and persistent (line-delimited JSON loop) modes, transparently, so hook authors write the same code for both.

Trace:
- TEST-3210
- CON-3210

### REQ-3211: Per-File Extension Opt-Out via Frontmatter

Every canonical extension SHALL honour a per-file frontmatter override under the reserved `extensions.<name>` path:

```yaml
---
extensions:
  callouts: false
  tasks:
    filter: "not done"
---
```

- Boolean `false` disables the extension for that page.
- Boolean `true` (the default) or absent leaves the extension enabled.
- Object values are passed to the extension as page-level configuration (opaque to ztl; extension-specific semantics).

The baseline check (`frontmatter.extensions.<name> != false`) SHALL be part of the extension's default selector; theme authors replacing an extension MUST preserve this opt-out to remain behaviour-compatible.

Trace:
- TEST-3211
- CON-3211

### REQ-3212: Canonical Extensions — Theme Stubs, Ecosystem-Backed

**Resolution of open question SPEC-033 §13 Q1: option (b) — thin stubs.**

The default theme SHALL ship **theme-layer CSS + template partials** for
three canonical patterns (Callouts, Tasks, Admonition) without shipping
their transformation code. The transform is delegated to an ecosystem
plugin (SPEC-033) declared in the default theme's `.ztl/hooks/` manifests:

- **`callouts`** — recognises `> [!TYPE] Title` blockquotes. Default theme
  ships `themes/default/static/callouts.css` (colour palette, icon
  mapping, light/dark tokens) and a `callouts.toml` manifest referencing
  a Pandoc or mdBook ecosystem plugin (e.g., `pandoc-admonition`,
  `mdbook-admonish`) that actually performs the block-quote → callout
  div rewrite. Users without the ecosystem plugin installed still get
  correctly-styled output if the plugin is present; otherwise the
  blockquote renders as a standard `<blockquote>` with no fatal error.
- **`tasks`** — default theme ships styling only; the query-evaluation
  extension is deferred to a named subset documented in SPEC-033 §13 Q4
  (Obsidian Tasks subset). Until that subset is picked, `tasks` blocks
  render as plain fenced code with a class hint; the matrix tier is
  `experimental`.
- **`admonition`** — same model as callouts; the `ad-*` legacy-syntax
  rewrite is backed by an ecosystem plugin; CSS ships in the theme.

**Why stubs, not implementations:** SPEC-033 empirically established
that ecosystem plugins cover these patterns with stronger maintenance
signals and wider reach than any code ztl could ship. The theme owns
*design*; the ecosystem owns *transformation*. This resolves
SPEC-033 §13 Q1 and removes the Python/Node runtime dependency the
prior text imposed on every `ztl build` (see ADR-3204 for the
decision trail).

Each stub SHALL have a matrix entry recording (a) its CSS/template
path, (b) the recommended ecosystem plugin backing it, (c) a golden-HTML
fixture asserting correct styling when the ecosystem plugin is active,
and (d) a fallback-render fixture asserting the no-ecosystem-plugin
degraded state is acceptable.

Trace:
- TEST-3212a, TEST-3212b, TEST-3212c
- CON-3212
- SPEC-033 §13 Q1 (resolved → option b)

### REQ-3213: Canonical Extension Matrix

The system SHALL ship `tools/ztl-extension-matrix.toml` recording, for each first-party extension: name, tier (`supported`, `partial`, `experimental`), pinned AST schema version, selector, golden-fixture path, and notes. CI SHALL gate changes that downgrade tier or remove a fixture without an accompanying rationale.

Trace:
- TEST-3213

### REQ-3214: Template Variable Publishing

Every hook response (all three stages) MAY include an optional `template_vars` field containing a JSON object of arbitrary shape. The system SHALL accumulate these across all hooks run on a page into the Minijinja template context at the path `page.ext.<extension_id>`.

**Namespace rules:**

- **Reserved root.** `page.ext` is reserved for extensions. ztl itself SHALL NOT write into this root in any present or future release without a contract major bump (matches SPEC-028's theme contract convention). Themes and extensions can rely on it as their exclusive surface.
- **`<extension_id>` default.** Defaults to the hook's filename without extension (e.g., `tasks.py` → `tasks`) AND without any leading numeric-plus-dash ordering prefix (so `20-tasks.py` → `tasks`, not `20-tasks`). This keeps the template-readable name stable across re-ordering.
- **Manifest override.** A hook's manifest MAY declare `extension_id = "..."` to override the default. Useful when multiple hooks cooperate under one conceptual name, or when the filename would produce an undesirable id.
- **Collision-on-filename resolution.** When a vault hook replaces a theme hook via the REQ-3206 filename-collision rule, the replacement takes over the same `page.ext.<extension_id>`. The theme's templates reading `page.ext.tasks.completed` continue to work — the vault author is obligated to emit the same-shaped data if they want existing theme templates to render correctly. This is the design contract, not a constraint.
- **No cross-namespace writes.** Extensions cannot write under another extension's `page.ext.<id>`. ztl enforces this at the protocol layer by keying the merge on the invoked hook's id, not on any hook-supplied key.
- **Cross-extension coordination via shared `extension_id`.** Two cooperating hooks can share an `extension_id` (declared in both manifests) to coalesce into one namespace; in that case, pipeline-order wins (later hook's emission replaces earlier, with a warning logged).

**Semantic rules:**

- **Multi-stage emissions by the same hook:** If the same hook runs at multiple stages and emits vars at each, the later stage's vars replace the earlier stage's (with a warning logged). Within a single stage's response, the emitted object is final.
- **Shape:** Any valid JSON value (object, array, string, number, bool, null). ztl validates size ≤ 1 MiB per hook per page; oversize payloads are dropped with a warning, but the AST/HTML payload is still used.
- **Autoescape:** String values emitted into templates go through the standard Minijinja autoescape path. No new XSS surface.
- **Opt-in at author time, opt-in at theme time:** Extensions choose whether to emit; themes choose whether to read. Absent vars resolve to `undefined` in Minijinja (no error).
- **Build/serve parity:** Both modes expose the same `page.ext.<id>` namespace with identical semantics.

Example: a hook at `.ztl/hooks/transform.d/20-tasks.py` returning `{"template_vars": {"total": 12, "completed": 7}}` makes `{{ page.ext.tasks.total }}` and `{{ page.ext.tasks.completed }}` available in all templates that render that page. A sibling manifest declaring `extension_id = "my_tasks"` would surface the same data at `page.ext.my_tasks.*` instead.

Vault-level aggregation (`vault.ext.<extension_id>`) is explicitly out of scope for v1 — see §13 Open Questions.

Trace:
- TEST-3214
- CON-3214
- OBS-3209

### REQ-3215: Dual-Version Exposure (Binary and AST Schema)

The system SHALL expose two distinct version strings to every hook invocation: `ztl_VERSION` (the binary semver, changes every release) and `ztl_AST_VERSION` (the JSON-schema semver, changes only when the AST shape changes). Both SHALL be available as environment variables at hook start AND as top-level fields in the persistent-mode handshake message (`{"ztl_version": "...", "ast_version": "1.2"}`).

Hooks declare their required AST-schema range in the manifest:

```toml
ast_version = ">=1.0 <2"   # npm-style range; default ">=1.0 <2" for v1
```

At ztl startup, the system SHALL compute the effective `ztl_AST_VERSION` and compare against each hook's declared range. Version-drift policy (REQ-3215.1):

- **Incompatible range** (hook demands >=2.0 on a 1.x binary, or vice versa): hook is disabled with a typed error; log line `[ztl] hook incompatible: <id> requires ast_version=<range>, have <version>`. Build continues.
- **Compatible but minor mismatch** (hook wrote against 1.0, binary offers 1.2): hook runs; warning logged once per hook per session.
- **Exact match**: silent success.

Rationale: follows Pandoc's `PANDOC_API_VERSION` / `PANDOC_VERSION` split ([filters.html](https://pandoc.org/filters.html)). Decouples AST-breaking changes from ztl's normal release cadence. Hooks authored against AST v1.0 survive ztl binary releases 1.x.y → 1.y.z so long as the AST schema remains backwards-compatible (NFR-3206).

Trace:
- TEST-3215
- CON-3201
- ADR-3201

### REQ-3216: Capability Probe

The system SHALL invoke every hook in **probe mode** once at pipeline initialisation, before any page is processed. Probe mode is signalled by argv (`<hook-executable> --probe`) or by the first-line protocol message `{"type": "probe"}` in persistent mode. The hook SHALL respond with a single JSON document declaring:

```json
{
  "type": "probe_result",
  "ztl_ast": "1.0",
  "hook": "callouts",
  "version": "1.0.3",
  "stages": ["transform"],
  "applies_when": {"modes": ["build", "serve"], "themes": null, "formats": ["html"]},
  "ready": true
}
```

Semantics:

- `stages` — the stages this hook handles. A one-shot hook appearing in `transform.d/` but reporting `stages: ["post-render"]` is a manifest/probe mismatch and the hook SHALL be disabled with a diagnostic.
- `applies_when` — optional; allows a hook to exclude itself from e.g. `ztl serve` (build-only hooks) or specific themes. When absent, defaults to applies-always.
- `ready: false` — hook explicitly declines to run; no error, diagnostic logged with optional `reason` field.

Probe failures (non-zero exit, malformed response, timeout > 5s) SHALL disable the hook for the current session with an actionable diagnostic. `ztl hooks check` (new subcommand) SHALL run every hook's probe and report status without running the build.

Rationale: follows mdBook's `supports <renderer>` pattern ([mdBook dev guide](https://rust-lang.github.io/mdBook/for_developers/preprocessors.html)). Cheap self-disablement and diagnostic surface without the cost of full-pipeline invocation.

Trace:
- TEST-3216
- CON-3216

### REQ-3217: Ordering Constraints and Optional Hooks

The system SHALL support two additional manifest fields to manage hook composition beyond filename order:

```toml
before = ["admonition"]       # run before these (by extension_id)
after = ["callouts"]          # run after these
optional = true               # missing hook executable downgrades to warning, not error
```

**Resolution algorithm:** ztl performs a topological sort over the hook set for each stage, respecting `before`/`after` constraints, with filename order as the tiebreaker for unordered hooks. Cycles (A before B, B before A) SHALL be reported as a build error with the cycle path in the diagnostic.

**Optional:** A hook marked `optional = true` whose executable is missing, non-executable, or whose probe fails SHALL emit a warning and be skipped, but SHALL NOT fail the build or abort pipeline construction. Default is `optional = false`.

Rationale: mdBook's ordering and optional flags ([mdBook config](https://rust-lang.github.io/mdBook/format/configuration/preprocessors.html)). Filename ordering works for small pipelines; named constraints scale to ten-hook vaults without prefix-renumbering every time a hook is added.

Trace:
- TEST-3217
- CON-3217

### REQ-3218: Declarative Node-Type Dispatch in Helper Libraries

The system's helper libraries (`ztl-ast-py`, `ztl-ast-js`) SHALL provide a `dispatch` entry point that takes a table (dict/object) keyed by AST node type and invokes the corresponding function for each matching node as ztl walks the tree. Reserved keys: `Inline` matches any inline node not otherwise covered; `Block` matches any block node; `_fallback` matches any node (was `*` in an earlier draft; `*` is easy to typo as empty-string and grep-hostile).

Python API:
```python
from ztl_ast import dispatch

def transform(ast, ctx):
    return dispatch(ast, ctx, {
        "BlockQuote": handle_callout,
        "CodeBlock": handle_tasks_block,
        "Inline": passthrough,   # fallback for any inline
    })
```

The low-level `walk(ast, type=Foo)` API remains available for visit-all-and-filter use cases; `dispatch` is the recommended 80% path.

Rationale: Pandoc Lua filters' function-table-keyed-by-element-name pattern ([lua-filters.html](https://pandoc.org/lua-filters.html)) eliminates an entire class of bugs ("I forgot to type-check the node before touching it") present in unified's visit-and-match pattern.

Additionally, sequence-level dispatch SHALL be supported via `Inlines` and `Blocks` keys that receive whole lists for operations that depend on element ordering (callouts matching `Heading` + following `BlockQuote`, for example).

Trace:
- TEST-3218
- CON-3218

### REQ-3219: Shared Build-Scoped Data Channel

The system SHALL expose a build-scoped, hook-writable key/value store accessible to every hook at every stage during a single build or page render. The store is:

- **Read-only from the hook's perspective at invocation time** — ztl injects the current store snapshot into the context passed to the hook; hook-requested writes are returned in the response.
- **Hook-requested writes merged by ztl** — a hook returning `{"build_data": {"unresolved_links": [...]}}` writes into the next hook's snapshot. Writes are namespaced by writing-hook's `extension_id` to avoid collisions (`build_data[extension_id][key] = value`).
- **Build-scoped**: cleared between builds (`ztl build`) or between page renders (`ztl serve`).
- **Size-capped**: 16 MiB total per build; oversize writes dropped with a warning.

Access in hook:
```python
def transform(ast, ctx):
    # ctx.build_data is a read-only view of the current store
    prior_citations = ctx.build_data.get("citations", {}).get("keys", [])
    ctx.emit_build_data(keys=prior_citations + new_keys)
    return ast
```

Rationale: unified's `processor.data()` ([unified](https://github.com/unifiedjs/unified)) and markdown-it's `env` parameter ([markdown-it architecture](https://github.com/markdown-it/markdown-it/blob/master/docs/architecture.md)) both exist because cross-hook coordination through frontmatter is fragile. ztl's `page.ext` namespace is one-way (hook → template); `build_data` is two-way (hook → hook).

Trace:
- TEST-3219
- CON-3219

### REQ-3220: Expose Build Context to Hooks

The system SHALL expose the following context to every hook invocation, available both as environment variables and as fields in the invocation JSON payload:

- `ztl_MODE` — `"build"` or `"serve"`.
- `ztl_THEME` — active theme name.
- `ztl_VAULT_ROOT` — absolute path to the vault.
- `ztl_OUT_DIR` — build output directory (null under serve).
- `ztl_VERBOSE` — `"true"` / `"false"`.
- `ztl_AT` — historical-build timestamp if `--at` is used, else absent.
- `ztl_HOOK_PATH` — absolute path of the hook's own executable, for resolving sibling resources (equivalent to Pandoc's `PANDOC_SCRIPT_FILE`).
- `ztl_EXTENSION_ID` — the extension_id ztl resolved for this hook (manifest override or filename default).

Rationale: Pandoc's `PANDOC_READER_OPTIONS`, `PANDOC_WRITER_OPTIONS`, `PANDOC_SCRIPT_FILE` ([filters.html](https://pandoc.org/filters.html), [lua-filters.html](https://pandoc.org/lua-filters.html)). Without a canonical exposure mechanism, hooks reinvent detection via filesystem introspection and side-channels.

Trace:
- TEST-3220
- CON-3220

### REQ-3221: Typed AST-Protocol (ast_type) for Transform-Stage Hooks

The system SHALL support multiple AST formats at the `transform` stage via
a declared `ast_type` on the hook manifest. The `ast_type` field identifies
which ecosystem's AST shape and invocation conventions the hook expects;
ztl translates between its internal representation and the declared type
at the protocol boundary.

This REQ defines the **protocol surface**: the obligations any ast_type
value must satisfy. The concrete set of supported ast_type values for v1
(`ztl-ext`, `pandoc-ext`, `mdast-ext`) and their instance-specific marker
tables are defined in **SPEC-033** — adding a new ecosystem is an
SPEC-033 amendment, not a SPEC-032 one.

**Built-in default:**

- **`ztl-ext`** (default) — ztl's native AST JSON (REQ-3202 / CON-3202).
  CommonMark subset + wikilink, embed, SPL-block, and frontmatter
  extensions. Required for v1 conformance and provided by this spec.

**Extension types** (concrete values defined by SPEC-033):

A hook manifest SHALL accept any ast_type registered in the binary's
ecosystem registry (SPEC-033 REQ-3301). Unknown values are a manifest
parse error. v1 binaries registering only `ztl-ext` SHALL reject
`pandoc-ext` / `mdast-ext` manifests with a typed "ecosystem-not-compiled"
error and an actionable hint (SPEC-033 REQ-3313).

**Translation contract (generic):**

For any registered non-default ast_type, the binary SHALL ship a
bidirectional translator with:

- A **marker-convention table** mapping ztl-ext concepts (Wikilink,
  Embed, SPL block, FrontMatter, position info) to the foreign AST's
  extension-point shape. Full tables live in SPEC-033 per ast_type and
  in `docs/ecosystems/<type>-translation.md` auto-generated from
  translator source.
- A **round-trip invariant**: for any ztl-ext AST *A* and any identity
  foreign-ext filter *I*, `foreign_to_ztl(I(ztl_to_foreign(A)))`
  equals *A* under the canonical-form equivalence relation defined in
  SPEC-033 NFR-3305.
- **Marker-strip detection**: after each foreign-ext hook invocation,
  ztl SHALL count instances of each node type listed in the hook's
  `contract.preserves` declaration (REQ-3224) in input vs output; a
  net decrease SHALL be logged as a warning
  (`"<plugin> dropped <N> <NodeType> on <page>"`), pipeline
  continues. The baseline v1 preserves-list for `pandoc-ext` and
  `mdast-ext` adapters is `["Wikilink", "Embed", "SPL"]` (injected
  by the adapter when the manifest doesn't declare its own), which
  matches the earlier hard-coded scope. Users wanting stricter
  guarantees extend their manifest's `contract.preserves`.

**Protocol-convention emulation:** When ztl invokes a non-default-ast_type
hook, the hook sees the ecosystem's native invocation contract
(env vars, argv, handshake shape) as specified in SPEC-033's per-ecosystem
CONs (e.g. CON-3303 for pandoc). When it invokes a `ztl-ext` hook, it
sees ztl's contract (REQ-3220's `ztl_*` env vars).

**Version compatibility per type:** `ast_version` in the manifest is a
semver range interpreted against the declared `ast_type`'s version scheme
— `>=1.22 <2` for `pandoc-ext` means pandoc-types v1.22+ below v2, not
ztl-ext v1.22+. ztl maintains a compat matrix
(`tools/ztl-ecosystem-matrix.toml`, SPEC-033 REQ-3311) mapping each
ast_type and version to the ones the current binary supports.

**Mixed pipelines:** Two hooks in the same stage may declare different
`ast_type`s. ztl translates the prior hook's output back to its internal
representation (ztl-ext), then serialises anew for the next hook's
declared type. No hook sees another hook's AST in a format it didn't
request.

Trace:
- TEST-3221
- CON-3221
- ADR-3206
- SPEC-033 REQ-3301, REQ-3307, REQ-3308

### REQ-3222: Pre-Parse Structural Safety Check

REQ-3201's pre-parse caveat is advisory — authors are told regex-over-
Markdown is risky, but nothing detects the breakage. The system SHALL
provide a structural-safety check driven by the hook's behavioural
contract (REQ-3224): a pre-parse hook declaring `contract.may_restructure
= false` (the default) triggers a ztl-side comparison of the block-tree
shape of the hook's input vs output (same parser, same extensions,
different text). A warning SHALL be emitted if the block tree changed
(different number of top-level blocks, different block types at the
same path, block-nesting depth delta > 1).

Authors whose hook legitimately rewrites block structure (e.g.
include-directive resolution) set `contract.may_restructure = true` to
suppress the check.

Promoted from deferred-v1.1 to v1 alongside REQ-3224's introduction of
the `[contract]` manifest table. The implementation is ~30 lines of
tree-shape diffing in the pure core.

Trace:
- TEST-3222
- CON-3222
- REQ-3224

### REQ-3223: Safe-Mode Build + Theme Hook Declaration

Hooks execute arbitrary code (SPEC-032 §10; same posture for SPEC-033
ecosystem plugins). Themes today don't execute code; after SPEC-032/033
themes may. The system SHALL provide two affordances to keep users in
control of what they consent to running:

**Safe-mode flag** — `ztl build --no-hooks` SHALL:

- Skip every hook in every stage, regardless of source (vault,
  theme-bundled, ecosystem adapter).
- Emit one log line per suppressed hook on stderr:
  `[ztl] --no-hooks: skipped <stage>/<extension_id> from <source>`.
- Complete the build with plain pipeline output (parse → render →
  template compose, nothing else). Exit code 0 on success.

`ztl serve --no-hooks` has identical semantics and is intended as
an audit / hostile-theme-inspection surface.

**Theme hook declaration** — a theme that ships hooks (files under
`themes/<name>/hooks/`) SHALL list them in `theme.toml` under a
`[[theme.hooks]]` array of objects with fields `stage`,
`extension_id`, `ecosystem` (optional), `summary` (one-line
description), and `contract` (REQ-3224, optional). ztl SHALL:

- At theme-selection time, compute the declared-vs-discovered diff
  (hooks on disk not listed in `theme.toml`, or vice versa).
- Emit a warning on first use of a theme with undeclared hooks:
  `[ztl] theme <name> ships <N> undeclared hook(s); run`
  `'ztl theme show <name>' for details, or --no-hooks to suppress`.
- Record the theme's hook declarations — including their declared
  contracts — in `ztl theme show`'s output so users can audit both
  execution surface (which hooks run) and behaviour surface (what
  each hook promises to do).

Theme authors gain one required bookkeeping step (declare what you
ship); users gain a single place to see what a theme will execute.
This is the minimum viable trust boundary; stricter mechanisms
(theme signatures, per-hook consent) are deferred to a successor
spec.

Trace:
- TEST-3223
- CON-3223
- §10 (threat model)

### REQ-3224: Behavioural Contract Declarations

Structural contracts (protocol, schema, version compat) tell ztl
*how* to talk to a hook. Behavioural contracts tell ztl (and users
auditing a vault or theme) *what properties the hook promises to
hold* about its transformation. This REQ introduces an opt-in
`[contract]` table in the hook manifest; REQ-3221 and REQ-3222
enforcement mechanisms read from it.

**Tier 1 fields (v1, verified by ztl):**

```toml
# .ztl/hooks/transform.d/callouts.toml

[contract]
preserves = ["Wikilink", "Embed", "CodeBlock", "FrontMatter"]
idempotent = true
may_restructure = false       # pre-parse hooks only; see REQ-3222
```

- **`preserves`** (array of AST node type names) — hook declares it
  does not strip these node types. ztl counts instances of each
  declared type in the hook's input vs output; a net decrease is a
  typed `contract_violation:preserves` diagnostic (REQ-3207 failure
  semantics apply). Generalises REQ-3221's existing wikilink /
  embed / SPL marker-strip detection — the mechanism is identical,
  the set is now per-manifest.
- **`idempotent`** (bool, default `false`) — hook asserts
  `f(f(x))` is canonical-form-equivalent to `f(x)` under NFR-3305's
  equivalence relation. ztl verifies in CI by running the hook
  twice on each matrix fixture page and asserting stability; a
  mismatch is a `contract_violation:idempotent` typed error and
  gates matrix tier (`supported` requires `idempotent = true`
  verified).
- **`may_restructure`** (bool, default `false`, `pre-parse` stage
  only) — enforced per REQ-3222.

**Tier 2 fields (reserved for v1.1, advisory in v1):**

```toml
[contract]
pure = true                   # reads only stage_input + context; no net/fs/clock
expansion_bound = 3.0         # output size ≤ N × input size
```

- **`pure`** — in v1 a documentation/trust label; surfaced in `ztl
  theme show` and `ztl hook coverage`. In v1.1, optional
  enforcement under `--sandbox-hooks` via bwrap / Firejail / Docker
  on platforms where those are available.
- **`expansion_bound`** — v1 reserves the field name; v1.1 enforces
  at the protocol boundary (one `len()` comparison; near-free).

**Default behaviour:** a manifest with no `[contract]` table has an
empty declaration. ztl behaves as today — no extra enforcement, no
trust signal. Ecosystem adapters (SPEC-033) MAY populate the table
on behalf of the plugin using values from the matrix (REQ-3311's
`contract` column).

**Output surface:**

- `ztl hook coverage` adds a per-hook `contract` column listing
  declared fields.
- `ztl theme show <theme>` lists each theme-bundled hook's
  contract alongside the REQ-3223 declaration.
- Ecosystem matrix entries (SPEC-033 REQ-3311) carry a `contract`
  field that maps into the runtime via adapter-owned translation.

**Contract violation → REQ-3207 failure path.** A declared contract
that ztl detects as violated produces the same failure-scoping
behaviour as any other hook error: the hook's output is discarded,
the input is passed to the next hook, a diagnostic with `reason =
"contract_violation"` and specific sub-reason (`preserves`,
`idempotent`, `may_restructure`) is logged. Under `--hook-fail-on
error`, a contract violation fails the build.

Trace:
- TEST-3224 (preservation diagnostic)
- TEST-3224-idempotent (CI double-run check)
- TEST-3222 (may_restructure enforcement)
- CON-3224
- REQ-3221, REQ-3222 (enforcement mechanisms)
- SPEC-033 REQ-3311 (matrix contract column)

### REQ-3225: Hook Authoring CLI

Plugin authoring is a tight iteration loop: write → run → diff →
fix. ztl has REQ-3209 `ztl hook dry-run` for selector iteration;
nothing else closes the loop. This REQ adds the subcommand surface
that makes "my first hook" a minutes-not-hours experience and
makes ongoing iteration feel like `cargo test` for Rust.

The system SHALL provide:

- **`ztl hook new <stage> <name> [--lang py|js|sh] [--ecosystem pandoc|mdbook|remark]`**
  — scaffold a working hook in the current vault. Emits:
  - `.ztl/hooks/<stage>.d/<name>.<ext>` — executable skeleton with
    an identity transform and a comment pointer to the helper
    library docs.
  - `.ztl/hooks/<stage>.d/<name>.toml` — manifest with sensible
    defaults (mode, timeout, empty `[contract]`, permissive selector).
  - `tests/hook-fixtures/<name>/input.md` + `expected.html` — minimal
    fixture the author can extend.

  With `--ecosystem`, scaffolds the corresponding ecosystem manifest
  (SPEC-033 REQ-3303/3304/3305) + a plugin-specific skeleton if
  applicable (e.g., a Lua filter stub for Pandoc).

- **`ztl hook test <hook>`** — run the hook against
  `tests/hook-fixtures/<hook>/input.md`, diff against `expected.html`.
  Non-zero exit on mismatch; prints a coloured line-diff. Honours
  `--update` to regenerate the golden (equivalent to `cargo insta
  accept` / `jest --updateSnapshot`).

- **`ztl hook fixture --from <page> --hook <name>`** — capture the
  current vault page's pre-hook input and post-hook output into
  `tests/hook-fixtures/<name>/`. Creates the fixture pair from an
  existing vault rather than requiring the author to synthesise one.

- **`ztl hook watch <hook>`** — file-watch the hook's source;
  restart the persistent-mode subprocess on change; stream the
  hook's stderr to the terminal. Identical to what `ztl serve`
  does internally for hot-reload, lifted as a user-invocable
  command so authors who are not actively serving can still get
  the iteration loop.

- **`ztl ast sample <file.md> [--stage pre-parse|transform|post-render]`**
  — emit the AST JSON (or Markdown text, or HTML fragment) that a
  hook at the given stage would receive as input for the given file.
  The single most useful debugging primitive: authors can see the
  exact bytes their hook will see without instrumenting the hook
  itself.

- **`ztl ast diff <before.json> <after.json>`** — structural diff of
  two AST documents as tree-aware delta (added / removed / modified
  nodes with source positions), not a line-diff of the JSON
  serialisation. Complements `ztl ast sample` for the
  "what did my transform change?" workflow.

Exit codes: 0 on success / match, 1 on expected failure (test
mismatch, hook error), 2 on invalid arguments / missing hook.

**Why this is v1, not v1.1:** the missing authoring loop is the
single biggest friction to writing a ztl hook today. Shipping
the machinery without the authoring surface means users see the
power but can't extract it. Each subcommand is under 200 LOC of
implementation and reuses existing machinery (discovery, manifest
parser, persistent-mode runtime).

Trace:
- TEST-3225 (CLI integration)
- CON-3225 (diagnostic message design, referenced here because
  `hook test` diff output uses the same template)

---

## 4. Non-Functional Requirements

### NFR-3201: Selection Evaluation Latency

Evaluating the full selector chain (path glob + frontmatter predicate + content probe) for one hook on one page SHALL complete in ≤ 2 ms at P95 for pages up to 100 KB on a 2020-or-newer laptop, AVERAGED across path/frontmatter/content probe passes. Selectors are the hot path; they must be cheap.

Trace:
- TEST-3201-perf
- OBS-3202

### NFR-3202: Render Overhead — Canonical Extensions

Under `ztl build` with the three canonical extensions (`callouts`, `tasks`, `admonition`) all enabled on the 2,000-page demo vault, total render time SHALL increase by ≤ 15% relative to a build with `--no-hooks` (new flag). Persistent-mode extensions amortise spawn cost; this is the target after that amortisation.

Trace:
- TEST-3202-perf
- OBS-3206

### NFR-3203: Deterministic Output

Given a fixed vault, fixed extension set, fixed ztl version, and fixed AST schema version, `ztl build` SHALL produce byte-identical HTML output across runs and platforms (macOS aarch64, Linux x86_64, Linux aarch64). Extensions are themselves required to be deterministic; documentation states the contract.

Trace:
- TEST-3203-determinism

### NFR-3204: Build Failure Isolation

Under default behaviour (`--hook-fail-on never`), `ztl build` exit code SHALL be 0 even when every enabled hook on every page fails. The pipeline degrades gracefully; diagnostics are emitted; the output HTML contains the unmodified content.

Trace:
- TEST-3207
- OBS-3205

### NFR-3205: Startup Cost When No Hooks Present

On a vault with no `.ztl/hooks/` directory and a theme with no `hooks/` directory, the additional process startup cost relative to pre-SPEC-032 ztl SHALL be ≤ 5 ms at P95. The feature has zero cost for users who don't use it.

Trace:
- TEST-3205-startup

### NFR-3206: AST Schema Stability

The ztl-AST schema SHALL follow semver. Within a major version: additive changes only (new node types, new optional fields). Breaking changes (removed fields, changed node shapes) require a major bump AND a one-release deprecation window where both majors are emitted behind a flag.

Trace:
- Reviewed at every schema change; tracked in CHANGELOG.

### NFR-3207: Protocol Overhead — Persistent Mode

For persistent-mode hooks, the per-page overhead (serialise AST → write to stdin → read response from stdout → deserialise) SHALL be ≤ 10 ms P95 for pages with ≤ 500 AST nodes (covers the typical case of a 10 KB Markdown page).

Trace:
- TEST-3207-perf
- OBS-3207

### NFR-3208: Memory Containment

Each hook process SHALL have a configurable memory ceiling (default 64 MiB, set via manifest). Exceeding the ceiling SHALL terminate the process; for persistent-mode hooks, ztl SHALL respawn on the next page requiring that hook.

Trace:
- TEST-3208
- OBS-3208

---

## 5. Contracts

### CON-3201: Persistent-Mode Wire Protocol

For `mode = "persistent"` hooks, ztl and the hook communicate over line-delimited JSON on stdin/stdout. Stderr is free-form (used for diagnostic logging only; never consumed by ztl as data).

**Handshake (one message):** On process start, the hook writes a single line:
```json
{"ztl_ast": 1, "hook": "callouts", "version": "1.0.3", "ready": true}
```
ztl reads this line; if `ztl_ast` major differs from the current schema, ztl disables the hook and logs `ast_version_mismatch`.

**Per-page request (from ztl, one line):**
```json
{"type": "invoke", "page_slug": "projects/q2", "frontmatter": {...},
 "payload": <stage-specific>, "deadline_ms": 100}
```
Where `payload` is a Markdown string for `pre-parse`, an AST JSON document for `transform`, or an HTML string for `post-render`.

**Per-page response (from hook, one line):**
```json
{"type": "result", "payload": <same-shape>, "diagnostics": [...], "template_vars": {...}}
```
Where `template_vars` is optional; when present, it is a JSON object merged into the page template context at `page.ext.<hook_id>` per REQ-3214.

Or on error:
```json
{"type": "error", "reason": "...", "detail": "..."}
```

**Shutdown:** ztl closes stdin. Hook SHOULD exit cleanly within 1 s. Hard-kill on timeout.

**Return-type contract (`transform` stage):** The hook's `payload` response field MUST be an AST document whose root is a `Document` node (same shape as the input). For in-tree transformations, every rewritten node MUST have a `type` field; a node of type X may be replaced by a node of type X, a node of a compatible type per the AST schema (e.g., `Paragraph` → `BlockQuote` is valid because both are block-level), or an array of compatible-type nodes (replacing one `Paragraph` with three). Replacing a block-level node with an inline-level node (or vice versa) SHALL be rejected with a typed validation error and the hook's output discarded per REQ-3207. (Follows Pandoc's strict return-type discipline: [lua-filters.html](https://pandoc.org/lua-filters.html) "Pandoc will throw an error if this condition is violated.")

**Return-type contract (`pre-parse` stage):** Payload must be valid UTF-8 text. Size limit: 16 MiB.

**Return-type contract (`post-render` stage):** Payload must be a UTF-8 string that parses as well-formed HTML (validated via a streaming parser). Malformed HTML is rejected per REQ-3207.

**Error surfacing convention:**
- **stdout**: structured JSON per this protocol. Non-JSON output on stdout is a hook error.
- **stderr**: free-form human-readable logs; ztl forwards under `--verbose` and includes in failure diagnostics unconditionally.
- **Non-zero exit**: aborts the hook's pipeline participation per REQ-3207; build overall continues unless `--hook-fail-on error` OR the hook manifest has `fail_hard = true`.

**Version-drift policy:** Enforced per REQ-3215. The handshake includes `ast_version`; ztl compares against the binary's supported range. Incompatible → hook disabled with typed error. Compatible-but-minor-mismatch → warning, hook runs. Exact match → silent.

Implements: REQ-3201, REQ-3210, REQ-3215.
Verified by: TEST-3201, TEST-3210, TEST-3215.

### CON-3202: ztl-AST JSON Schema (Summary)

A canonical example:

```json
{
  "ast_version": "1.0",
  "type": "Document",
  "position": {"start_line": 1, "start_col": 1, "end_line": 42, "end_col": 1},
  "frontmatter": {"title": "Q2 Review", "tags": ["project"]},
  "children": [
    {
      "type": "Heading",
      "level": 1,
      "position": {...},
      "children": [{"type": "Text", "text": "Q2 Review", "position": {...}}]
    },
    {
      "type": "BlockQuote",
      "position": {...},
      "children": [
        {"type": "Paragraph", "children": [{"type": "Text", "text": "[!note] Important"}]}
      ]
    },
    {
      "type": "Wikilink",
      "target": "Projects Index",
      "alias": null,
      "heading": null,
      "block_id": null,
      "position": {...}
    }
  ]
}
```

Full JSON Schema (Draft 2020-12) lives at `tools/ztl-ast-schema-v1.json`. The schema is normative; anywhere this document disagrees, the schema file wins.

**Default traversal order for `transform`-stage helpers:** ztl helper libraries default to **typewise** traversal (visits all `Inline` nodes, then all `Block` nodes, then `Document`). This matches Pandoc Lua filters' default and is empirically the order most extension authors expect ([lua-filters.html](https://pandoc.org/lua-filters.html) "Traversal order"). Helpers MAY offer a `topdown` mode for extensions that need root-to-leaves iteration; the selection is a flag on the `dispatch`/`walk` call, never on the ztl-side pipeline. Hook authors can override per call:

```python
dispatch(ast, ctx, handlers, traverse="topdown")   # root-first; Pandoc added this in 2.17
```

**AST size and depth caps:** Documents exceeding 10 MiB serialised JSON OR 256 nesting levels SHALL be rejected at the protocol boundary with a typed error. Matches CommonMark's nesting limit and prevents protocol-level DoS.

Implements: REQ-3202.
Verified by: TEST-3202.

### CON-3203: Manifest File Format

See REQ-3203 for the full TOML grammar. Every field is optional except `stage` (required when the manifest is used for a stage-specific behaviour; omitted when ztl infers stage from directory name). Unknown fields SHALL be rejected with a parse error to catch typos; ztl may later add fields, but additions follow the AST schema additive-only rule (NFR-3206).

Implements: REQ-3203.
Verified by: TEST-3203.

### CON-3204: Selection Evaluation — Pseudocode

```
for each page in vault:
    for each hook in pipeline_for(page.stage):
        if not paths_match(hook.include, page.path): continue
        if any_match(hook.exclude, page.path): continue
        if hook.frontmatter_where and not eval(hook.frontmatter_where, page.frontmatter): continue
        if hook.content_probes and not probes_pass(hook.content_probes, hook.require_probe_match, page.text): continue
        record_match(hook, page)
        # Hook is now eligible; data materialisation and invocation follow.
```

Implements: REQ-3204.
Verified by: TEST-3204.

### CON-3205: Frontmatter Predicate Semantics

See REQ-3205 grammar. Evaluation is left-to-right, short-circuiting for `&&` and `||`. Precedence: `!` > `==`/`!=`/`<`/`>` > `&&` > `||`. Comparisons are type-strict (string vs number returns `false`, never coerces). `contains` for strings is substring; for arrays is membership. `matches` is a regex match (ECMAScript-flavour, cached).

Implements: REQ-3205.
Verified by: TEST-3205.

### CON-3206: Hook Discovery and Precedence — Worked Example

Vault has:
```
.ztl/hooks/transform.d/
    20-tasks.py             (vault-authored, custom)
    10-callouts.py          (empty file, disables theme version)
```

Theme `default/` ships:
```
hooks/transform.d/
    10-callouts.py          (canonical)
    20-tasks.py             (canonical)
    30-admonition.py        (canonical)
```

Effective pipeline for stage `transform`, in order:
1. `10-callouts.py` — vault's empty file → disabled.
2. `20-tasks.py` — vault's version → runs (vault replaces theme).
3. `30-admonition.py` — theme's version → runs.

Implements: REQ-3206.
Verified by: TEST-3206.

### CON-3207: Failure Diagnostic Record

Every failure writes a record to `<out-dir>/hook-diagnostics.json` (build) or in-memory (serve):

```json
{
  "hook": "callouts",
  "stage": "transform",
  "page_slug": "projects/q2",
  "reason": "timeout",
  "detail": "Exceeded deadline_ms=100, killed at 180ms",
  "duration_ms": 180,
  "at": "2026-04-19T10:32:17Z"
}
```

Implements: REQ-3207.
Verified by: TEST-3207.

### CON-3208: Coverage Report Output

Table (default):
```
HOOK         STAGE      MATCHED  INVOKED  FAILED   P50   P95
callouts     transform  87/500   87       0        3ms   12ms
tasks        transform  12/500   12       1        45ms  180ms
admonition   transform  4/500    4        0        2ms   8ms
citations    transform  23/500   23       0        8ms   22ms
```

JSON: structurally equivalent array of objects.

Implements: REQ-3208.
Verified by: TEST-3208.

### CON-3210: Helper Library Signatures

Python (`ztl-ast-py`):
```python
from ztl_ast import run, walk, Node, Document, Paragraph, BlockQuote, Text, Wikilink
from ztl_ast.context import Context

def transform(ast: Document, ctx: Context) -> Document:
    for node in walk(ast, type=BlockQuote):
        ...
    return ast

run(transform)   # handles one-shot and persistent modes transparently
```

JavaScript (`ztl-ast-js`):
```javascript
import { run, walk, Document, BlockQuote, Text } from 'ztl-ast';
run((ast, ctx) => {
    for (const node of walk(ast, { type: BlockQuote })) { ... }
    return ast;
});
```

Both libraries embed the AST schema major and validate inputs on deserialisation.

Implements: REQ-3210.
Verified by: TEST-3210.

### CON-3211: Per-File Opt-Out Contract

Canonical extensions MUST include `frontmatter.extensions.<name> != false` in their selector's `frontmatter_where`. Third-party extensions MAY ignore this convention but SHOULD follow it for user ergonomics. Documentation notes this as a convention, not a runtime-enforced invariant.

Implements: REQ-3211.
Verified by: TEST-3211.

### CON-3221: AST Translation Contract

**Invariant:** For every supported `(ast_type, ast_version)` pair, ztl ships a bidirectional translator `ztl_ext ↔ foreign_ext` with the following guarantees:

1. **Round-trip identity for ztl-native concepts.** For any ztl-ext AST *A*, `foreign_to_ztl(ztl_to_foreign(A)) == A` byte-for-byte. Tested via property tests on a CommonMark-derived generator.
2. **Forward preservation of foreign concepts.** For any foreign AST *F*, converting to ztl-ext and back produces a representation that renders identically to the original HTML (semantic round-trip, not byte-identical — foreign AST may have attributes ztl doesn't track).
3. **Marker conventions for non-native concepts.** Each foreign AST declares a "marker namespace" (e.g., pandoc-ext uses `class="ztl-*"` and attrs on `Span`/`Div`) for ztl concepts it can't represent natively. A foreign-ext hook that strips markers corrupts round-trip but is detectable via REQ-3221's loss-detection logging.
4. **Protocol-convention table.** Each ast_type's invocation conventions (env vars, argv, handshake shape, error response format) are spelled out in `docs/ast-types/<type>.md` and form part of the ztl contract.

**Failure modes and diagnostics:**

| Failure                                                   | Detection                              | ztl response                              |
| --------------------------------------------------------- | -------------------------------------- | ------------------------------------------ |
| Hook strips marker attrs on ztl-wikilink spans          | Post-hook scan: count wikilink nodes in pre vs post | Warning with list of lost nodes; hook continues |
| Hook returns foreign AST that fails to parse              | Foreign-ast deserialise                | REQ-3207 failure; unmodified input passed  |
| Hook returns foreign AST that fails to translate back     | Translator validation                  | REQ-3207 failure                           |
| ast_type declared in manifest but not supported by binary | Pipeline init                          | Hook disabled with actionable error        |
| ast_version range incompatible with binary's translator   | Pipeline init                          | Hook disabled with actionable error        |

**Translator implementation module:** `src/hooks/translators/<ast_type>.rs`. Each translator is in the pure core (pure function from AST to AST), not the effectful shell. Test as a pure Rust module.

Implements: REQ-3221.
Verified by: TEST-3221.

### CON-3214: Template Variable Semantics

**Lifecycle:** For each page, ztl initialises an empty `page.ext = {}` context object before the pipeline runs. Each hook's response `template_vars` (if present) is deep-merged under `page.ext[hook_id]`. At template render time, Minijinja receives the final `page.ext` dict alongside the existing page variables.

**Merge semantics within `page.ext[hook_id]`:** Entire replacement (not recursive merge). If a hook emits vars at two stages, the later stage's `template_vars` object wholly replaces the earlier stage's. Rationale: extension authors think in whole-object updates; recursive merging produces surprising outcomes.

**Helper library API (Python):**
```python
def transform(ast, ctx):
    tasks = collect_tasks(ast, ctx.vault)
    ctx.emit_vars(
        total=len(tasks),
        completed=sum(1 for t in tasks if t.done),
        by_project=group_by_project(tasks),
    )
    return ast
```

`ctx.emit_vars(**kwargs)` accumulates into the hook's `template_vars` object; the final value is serialised as part of the response.

**Helper library API (JavaScript):**
```javascript
run((ast, ctx) => {
    const tasks = collectTasks(ast, ctx.vault);
    ctx.emitVars({
        total: tasks.length,
        completed: tasks.filter(t => t.done).length,
    });
    return ast;
});
```

**Size limit:** 1 MiB per hook per page, enforced at the JSON boundary. Oversize emissions cause the hook's `template_vars` to be dropped (with a warning) but the AST/HTML payload to be used normally.

Implements: REQ-3214.
Verified by: TEST-3214.

### CON-3212: Canonical Extension Fixture Format

For each canonical extension, `tests/extension-fixtures/<name>/` contains:
- `input.md` — sample vault page exercising the extension's full syntax
- `expected.html` — the expected post-render HTML fragment
- `selector-match.txt` — list of filenames from a fixture vault that the selector should match

CI runs the extension against `input.md`, asserts post-stage HTML equals `expected.html`, and asserts the selector matches exactly `selector-match.txt`.

Implements: REQ-3212.
Verified by: TEST-3212a, TEST-3212b, TEST-3212c.

### CON-3219: Shared Build-Scoped Data Channel — Concurrency

REQ-3219 specifies `build_data` as a build-scoped key/value store that
hooks read from context and extend via response writes. Concurrency
semantics:

- **Intra-page (within one page's hook pipeline): serial.** Hooks in
  a single page's pipeline run in order (REQ-3206 / CON-3217). Each
  hook receives the snapshot *including* writes emitted by earlier
  hooks on the same page. Order is deterministic.
- **Inter-page (across pages during `ztl build`): each page sees a
  snapshot fixed at its render-start.** When ztl renders pages
  concurrently, every page's pipeline reads from a snapshot frozen
  at the moment that page began rendering. Writes emitted by one
  page's pipeline are *not* visible to another page's pipeline
  rendering concurrently. This avoids races at the cost of
  cross-page write visibility being release-order-dependent.
- **End-of-build aggregation: out of scope here.** Hooks needing to
  see *all* pages' writes (e.g., a tag-cloud aggregator) must use the
  finalise mechanism proposed in §13 Q9 (`vault.ext.<id>`) — not
  `build_data`.
- **Serve mode: store cleared between page renders.** Under `ztl
  serve`, each page render starts with an empty `build_data`; the
  one-way-per-build semantics of build mode don't carry over.
- **Size cap: 16 MiB total per build, enforced on each write.**
  Oversize writes are dropped with a warning.

**Implication for hook authors:** `build_data` is for intra-page
coordination and for patterns where the store accumulates
*deterministically* (e.g., every citation filter stamps its key into
`build_data[citations].keys`, and a later per-page renderer reads its
own-page-only keys). Cross-page ordering or finality is not provided.

Implements: REQ-3219.
Verified by: TEST-3219.

### CON-3217: Composition Ordering — Worked Example

Hook pipelines combine two ordering inputs:

1. Filename lex-sort within a `<stage>.d/` directory (REQ-3206).
2. `before` / `after` manifest constraints naming other hooks by
   `extension_id` (REQ-3217).

The effective order is a topological sort over the set of enabled
hooks, with filename lex-sort as the tiebreaker whenever two or more
hooks are not constrained relative to each other.

**Worked example.** Given five hooks in `transform.d/`:

| File              | extension_id | before      | after        |
| ----------------- | ------------ | ----------- | ------------ |
| `05-prelude.py`   | prelude      | —           | —            |
| `10-callouts.py`  | callouts     | —           | —            |
| `10-tasks.py`     | tasks        | —           | `[callouts]` |
| `20-admon.py`     | admon        | `[tasks]`   | —            |
| `30-fini.py`      | fini         | —           | —            |

Resolution:

1. Build the constraint graph. Edges: `callouts → tasks` (from
   tasks.after); `admon → tasks` (from admon.before; `admon` runs
   before `tasks`).
2. Check for cycles. None.
3. Topologically sort. `callouts` must precede `tasks`; `admon`
   must precede `tasks`. `prelude`, `fini`, and the trio are
   otherwise unordered.
4. Apply filename lex-sort as tiebreaker. Lex order of filenames is
   `05-prelude.py`, `10-callouts.py`, `10-tasks.py`, `20-admon.py`,
   `30-fini.py`.
5. Emit the final order: `prelude`, `callouts`, `admon`, `tasks`,
   `fini`. (`admon` runs before `tasks` per the constraint, even
   though `20-admon.py` sorts after `10-tasks.py`; the constraint
   wins.)

**Cycles.** A cycle (e.g. adding `callouts.after = ["tasks"]` to the
above) is a build error; the diagnostic names the cycle path
(`callouts → tasks → callouts`) and the affected manifest files.

**`optional = true` interaction.** A `before` / `after` reference to an
`optional` hook that is absent/disabled is ignored (the constraint is
dropped, not an error). A reference to a non-optional missing hook is
a build error.

Implements: REQ-3217.
Verified by: TEST-3217.

### CON-3224: Behavioural Contract — Schema and Enforcement

**Manifest schema:** The `[contract]` table is optional. When present,
keys are:

```rust
#[derive(Serialize, Deserialize, Default)]
pub struct ContractDecl {
    /// AST node type names the hook promises not to strip.
    /// Tier-1. Enforced by post-hook type-counting (REQ-3221 generalisation).
    #[serde(default)]
    pub preserves: Vec<String>,

    /// f(f(x)) canonical-form-equivalent to f(x). Tier-1.
    /// Enforced by CI double-run on matrix fixtures.
    #[serde(default)]
    pub idempotent: bool,

    /// Pre-parse stage only. Tier-1. Enforced per REQ-3222.
    #[serde(default)]
    pub may_restructure: bool,

    /// Tier-2 (v1.1). In v1: advisory label only.
    #[serde(default)]
    pub pure: bool,

    /// Tier-2 (v1.1). In v1: field reserved.
    #[serde(default)]
    pub expansion_bound: Option<f32>,
}
```

**Enforcement loci:**

| Field              | When checked               | Cost per page   | Diagnostic kind                         |
| ------------------ | -------------------------- | --------------- | --------------------------------------- |
| `preserves`        | After each hook invocation | O(nodes_counted) single pass | `contract_violation:preserves` |
| `idempotent`       | CI double-run on fixtures  | CI only; not in hot path | `contract_violation:idempotent`      |
| `may_restructure`  | After each pre-parse hook  | O(block_tree) structural diff | `contract_violation:may_restructure` |
| `pure` (v1.1)      | Under `--sandbox-hooks`    | OS-sandbox cost | `contract_violation:pure`               |
| `expansion_bound`  | At hook output boundary (v1.1) | O(1)        | `contract_violation:expansion_bound`    |

**Diagnostic format:** contract violations reuse REQ-3207's failure
record (plugin_id, stage, page_slug, duration_ms) with an additional
`reason = "contract_violation"` and a `sub_reason` naming the
violated field. Under default `--hook-fail-on never`, the violation
is logged and the hook's output is discarded (fail-soft to plain
input). Under `--hook-fail-on error`, a contract violation fails the
build.

**Ecosystem-adapter provenance:** when a hook's `ecosystem` field
names an adapter (SPEC-033), ztl MAY populate `contract` from the
matrix entry (REQ-3311) if the hook's manifest leaves it empty —
equivalent to "inherit the matrix's declared contract". A manifest
that declares its own `[contract]` overrides the matrix for that
plugin, useful when a user patches a plugin or invokes it with
contract-altering flags.

Implements: REQ-3224.
Verified by: TEST-3224, TEST-3224-idempotent, TEST-3222.

### CON-3225: Diagnostic Message Design

Every diagnostic emitted by the hook subsystem SHALL follow a
five-part structure, progressively disclosing detail:

1. **Summary line** — one line, identifies the hook, the page (if
   applicable), and the failure class. Machine-grep-able first.
2. **Context** — which manifest field / which contract invariant /
   which matrix entry is implicated.
3. **Observed data** — concrete numbers or sample text showing what
   went wrong; not just "failed".
4. **Likely cause** — one sentence naming the most common cause of
   this failure class.
5. **Remediation hint** — a link or a command the author can run
   next to make progress.

**Worked example — contract preservation violation:**

```
[ztl] hook 'callouts' contract violation on projects/q2-review.md:
  contract.preserves = ["Wikilink", "Embed"]
  observed in input:  12 Wikilink, 3 Embed
  observed in output:  8 Wikilink, 3 Embed
  net change: -4 Wikilink

  Likely cause: the transform strips unrecognised Span classes or
  drops inline nodes whose type it doesn't match.
  Hint: run `ztl ast diff <before.json> <after.json>` on the fixture
  input to locate the removed nodes; see:
    https://ztl.codeberg.page/docs/hook-authoring/preservation
```

**Worked example — idempotence violation (CI double-run):**

```
[ztl] hook 'tasks' contract violation in CI double-run on
       tests/hook-fixtures/tasks/input.md:
  contract.idempotent = true (declared)
  observed: canonicalise(f(f(x))) != canonicalise(f(x))
  first run:  3 tasks blocks rendered, 0 nested
  second run: 3 tasks blocks rendered, 3 nested ← likely bug

  Likely cause: the transform runs over its own output, wrapping
  already-rendered blocks a second time.
  Hint: detect already-rendered output (e.g. presence of
  `class="ztl-tasks-rendered"`) and short-circuit, OR declare
  contract.idempotent = false if nested output is intended.
```

**Worked example — manifest parse error:**

```
[ztl] hook manifest error at .ztl/hooks/transform.d/foo.toml:
  unknown field 'ecosystm' (did you mean 'ecosystem'?)
  line 3, column 1

  Hint: valid top-level fields: stage, mode, timeout_ms, memory_mib,
  ast_type, ast_version, ecosystem, select, before, after, optional,
  extension_id, contract. See:
    https://ztl.codeberg.page/docs/hook-authoring/manifest-fields
```

**Colour and quieting:** diagnostic colouring is honoured per
`NO_COLOR` env var (standard) and via `--no-color`. Under `--quiet`,
only the summary line is emitted; under default, all five parts;
under `--verbose`, additionally the full stderr of the hook process
if the failure was a crash or timeout.

**Applicability:** this structure applies to every failure path
ztl logs from the hook subsystem — manifest parse errors, selector
evaluation failures, contract violations, protocol errors, ecosystem
runtime absence. Conforming to the structure is a quality-gate item
(§15) checked against fixture diagnostics.

Implements: CON for hook-diagnostic design across REQ-3207, REQ-3213,
REQ-3215, REQ-3222, REQ-3224, REQ-3225.
Verified by: TEST-3225 (fixture of each failure class, assert
diagnostic shape).

---

## 6. Architecture Decisions

### ADR-3201: Define a ztl-Specific AST JSON Format (Not pulldown-cmark Events)

**Context:** The `transform` stage needs a stable data-interchange format. Options: (a) re-use pulldown-cmark's event stream, (b) adopt the unified/remark `mdast` format, (c) define a ztl-specific schema.

**Decision:** Define a ztl-specific schema derived from CommonMark AST conventions, with explicit extensions for wikilinks, embeds, and frontmatter.

**Rationale:**
- **Parser independence.** pulldown-cmark's event stream is an iteration protocol, not a stable document format; it serialises awkwardly. If we ever swap parsers, tying hooks to pulldown-cmark's shape is a migration nightmare.
- **Vault-domain extensions.** `mdast` doesn't cover wikilinks or ztl's `![[...]]` embed syntax natively. We'd need extensions anyway; once you're extending, defining the whole schema is cleaner than patching someone else's.
- **Versioning control.** A ztl-owned schema means we decide when to bump, what additive changes mean, and how to deprecate. `mdast` evolves at remark's cadence, which doesn't match ours.
- **Serialisation shape.** Nested tree JSON is ergonomic for both Python and JS AST-walking code. Event streams require users to track open/close state — more error-prone.

**Trade-offs accepted:**
- Schema maintenance burden is ours. Offset by the fact that the schema is small (~20 node types) and CommonMark-stable.
- Conversion cost: pulldown-cmark events → ztl-AST JSON is a serialise step. Measured overhead is ~0.5 ms for a typical page; negligible.

**Alternatives considered:**
- `mdast` — rejected on extension-ergonomics and cadence grounds.
- pulldown-cmark events — rejected on stability and shape grounds.
- A binary format (CBOR, MessagePack) — rejected on debuggability. JSON wins until protocol overhead becomes a measured bottleneck.

Status: Proposed.

### ADR-3202: Out-of-Process Hook Execution (v1)

**Context:** Hooks could run in an embedded scripting language (Steel, QuickJS, Lua, WASM) or as subprocess pipes. The decision was debated in SPEC-031 and in discussion during this spec's drafting.

**Decision:** v1 uses out-of-process subprocess execution with line-delimited JSON protocol. In-process execution is deferred to a successor spec gated on explicit product demand for live-hackability.

**Rationale:**
- **Language choice.** Hook authors write in whatever language they know — Python, Node, Ruby, Rust, shell. In-process locks them to one runtime.
- **Failure isolation.** A buggy hook crashes its process; `ztl` keeps running. In-process embedding makes OOM and panics the host's problem.
- **Supply chain.** No embedded interpreter = no QuickJS/Steel CVE exposure in `ztl`'s dependency tree.
- **Ergonomics closable via helper libraries.** The gap vs. in-process (fast calls, no serialisation) is real but bridgeable with well-designed Python/JS libraries and persistent-mode pipes.

**Trade-offs accepted:**
- Per-page serialisation cost (~10 ms P95 in persistent mode per NFR-3207). Not free; tolerable for the canonical-extensions workload.
- No hot-reload during a single serve session without explicit hook-restart triggers. Serve mode can watch hook files and restart persistent processes on change — a v1.1 enhancement.

**Alternatives considered:**
- Steel Scheme (anuna-code pattern) — defer until live-hackability becomes an explicit product goal. Strong architecture but heavier lift than v1 needs.
- QuickJS (SPEC-031) — rejected by prior scan data.
- WASM — technically interesting; ecosystem for Markdown-processing WASM modules is thin.

Status: Proposed.
Supersedes: SPEC-031 ADR-3101 (QuickJS choice).

### ADR-3203: Selection in Sidecar Manifest (Not in Hook Body)

**Context:** A hook's selector could live in (a) a sidecar TOML manifest, (b) a config.toml section, (c) the hook's own stdout on a "describe yourself" call.

**Decision:** Sidecar manifest (`<executable>.toml`) next to the hook executable.

**Rationale:**
- **Evaluable without invoking.** ztl parses the manifest once at startup; selector evaluation per page doesn't require the hook to run. Critical for NFR-3201.
- **Co-located with the executable.** The hook and its selector travel together — themes, version control, distribution archives all move one `(executable, manifest)` pair as a unit.
- **Discoverable.** Users browsing `.ztl/hooks/transform.d/` see `callouts.py` and `callouts.toml`; the relationship is obvious.
- **Tool-friendly.** `ztl hook dry-run` parses only the manifest; no need to spawn the hook process just to learn its selector.

**Trade-offs accepted:**
- Two files per hook (not one). Mitigated: manifests are optional; hooks without one get sensible defaults with a warning.
- Selector is static — can't depend on runtime state. This is a feature (determinism) more than a limitation.

**Alternatives considered:**
- Config.toml namespace — rejected: centralising every hook's config in one file breaks the per-hook encapsulation that makes themes composable.
- Hook-declared selector (describe-yourself call) — rejected: forces a process spawn to learn the selector, defeating the perf argument.

Status: Proposed.

### ADR-3204: Canonical Extensions Ship as Theme-Layer CSS, Not First-Party Code

**Context:** Callouts, Tasks, Admonition could ship as (a) native Rust
modules, (b) first-party Python/JS canonical hooks in the default theme,
(c) both, or (d) CSS-only theme stubs with transformation delegated to an
ecosystem plugin (SPEC-033). An earlier draft of this ADR picked (b);
it was reconsidered alongside SPEC-033 §13 Q1.

**Decision:** (d) — thin CSS + template stubs in the default theme,
transformation delegated to an ecosystem plugin configured in the
default theme's hook manifests.

**Rationale:**
- **Leverage over originality.** SPEC-033's ecosystem scan found that
  mature ecosystem plugins (`mdbook-admonish`, Pandoc's div syntax,
  remark-directive) cover these three patterns with years of edge-case
  fixes. ztl shipping its own Python/JS implementations duplicates
  that work and takes on the maintenance burden.
- **No runtime dependency for the default theme.** Earlier draft
  imposed Python 3.9+ as a default-theme dependency. Thin stubs
  require only CSS; the ecosystem plugin is only active when the
  user has installed its runtime (Pandoc / Node / etc.) via SPEC-033.
  `ztl build` in Alpine CI with plain-Markdown content produces a
  usable output without installing any extra runtime.
- **Theme owns design, ecosystem owns transformation.** This is the
  cleanest separation — theme authors restyle by editing CSS;
  transformation-layer behaviour is the ecosystem plugin's concern.
- **Consistent extension surface.** User-written hooks still follow
  the SPEC-032 contract; the default theme simply doesn't _ship_ any
  first-party hook implementations — it configures which ecosystem
  plugin to run.

**Trade-offs accepted:**
- Migrant user without any ecosystem plugin installed sees a
  degraded render (plain blockquotes for Callouts). Mitigation: the
  default theme manifest lists the recommended plugin with a clear
  install hint surfaced by `ztl ecosystem check`.
- Some patterns (e.g., Tasks' query evaluation) don't have a
  ready ecosystem match and are deferred to a separate specification
  rather than shipped here.

**Alternatives considered:**
- **(a) Native Rust** — rejected on consistency grounds; becomes a
  second extension surface competing with the hook surface.
- **(b) First-party Python/JS hooks** — rejected because it imposed a
  runtime dependency on every `ztl build` and replicated what the
  ecosystem already does better (SPEC-033 §1.1).
- **(c) Both** — rejected as over-engineered.

Status: Proposed. Supersedes (within this spec's draft history) an
earlier variant that picked (b).

### ADR-3206: Typed AST Protocol (Multiple `ast_type`s) Over Single-Format

**Context:** SPEC-032's transform-stage AST is ztl-ext — our own CommonMark-derived format with wikilink/embed/SPL extensions (ADR-3201). This gives schema control but closes the door on the vast existing ecosystem of Pandoc filters (hundreds of extensions: pandoc-crossref, pandoc-citeproc, pantable, etc.) and unified/remark plugins (thousands). The question is how to open that door without surrendering ztl-ext.

Three shapes were considered:

- **(a)** Adopt pandoc-types (or mdast) wholesale as ztl's AST. Full ecosystem compatibility; no translation layer needed. Cedes schema control, couples ztl to Pandoc's release cadence, and represents wikilinks/embeds/SPL as stringly-typed conventions over `Span`/`Raw` (ADR-3201 is essentially a rejection of this).
- **(b)** Keep ztl-ext as the only AST. Ship a separate `ztl-pandoc-adapter` binary as an optional companion tool. Users invoke it explicitly in their manifest. Works, but requires users to know about adapter tools, install them separately, and reason about the two-layer abstraction.
- **(c)** Typed protocol: each hook declares its `ast_type` in the manifest; ztl's own dispatch layer knows how to serialise to each supported type. Translation is owned by ztl, not a third-party tool. Mixed pipelines (ztl-ext + pandoc-ext + mdast-ext) compose because ztl translates at each boundary.

**Decision:** (c). The protocol is typed; `ztl-ext` is one of several supported `ast_type` values, not the only one. v1 accepts and defaults to `ztl-ext`; `pandoc-ext` and `mdast-ext` are reserved values that produce actionable errors in v1 and are implemented in v1.1 and v2.x respectively.

**Rationale:**

- **Borrow strength without cession.** ztl retains schema control (ADR-3201 stands) while offering first-class compatibility with external ecosystems through translators ztl owns.
- **Clean composition.** Mixed-`ast_type` pipelines work out of the box. A user can chain pandoc-crossref (pandoc-ext), a native ztl Callouts extension (ztl-ext), and a remark-gfm-style plugin (mdast-ext) in one pipeline; ztl translates at each boundary.
- **First-party translator quality.** Adapter binaries live in third-party repos with their own release cadences, test thoroughness, and abandonment risk. A ztl-owned translator is testable in-tree with the rest of the hook runtime.
- **Per-type versioning.** `ast_version` ranges are interpreted relative to the declared `ast_type`'s version scheme. pandoc-types v1.22 and v2.0 can be supported simultaneously in the compat matrix without coupling ztl-ext's semver.
- **Future-proofing.** New ecosystems (Djot, Pollen) are additions to the type registry, not redesigns of the protocol.
- **Precedent.** LSP capability negotiation, Tree-sitter's per-query language fields, and DAP's similarly-typed protocols all demonstrate this shape works for protocol-level extensibility.

**Trade-offs accepted:**

- **Translator implementation cost.** Each supported type is a non-trivial Rust module with fuzz-quality round-trip tests against real ecosystem hooks. pandoc-ext alone is substantial. Managed by gating implementation to v1.1 (pandoc-ext) and v2.x (mdast-ext).
- **Translation loss is intrinsic.** Foreign ASTs (Pandoc, mdast) don't have native wikilinks or SPL. Marker conventions are unavoidable. Documentation must make round-trip guarantees explicit.
- **Protocol-convention emulation.** Supporting pandoc-ext means emulating Pandoc's invocation contract (env vars, argv). Real engineering; managed by scoping the contract in CON-3221 and documenting in `docs/ast-types/pandoc-ext.md`.
- **Complexity signal to users.** Users now have one more manifest field. Default (`ztl-ext`) covers the 80% case; only ecosystem-migration users need to think about `ast_type`.

**Alternatives rejected:**

- **Single-type with adapter binary** (shape b) — rejected on user-experience grounds. "Install this other tool to use Pandoc filters" is materially worse than "set `ast_type = \"pandoc-ext\"`."
- **Adopt pandoc-types wholesale** (shape a) — rejected under ADR-3201. Still correct.
- **Defer typed protocol entirely to v2.0** — considered. Rejected because the manifest field is free to define now (it costs nothing to reserve) and retrofit later would require a manifest-schema breaking change.

Status: Proposed.
Supersedes: —. Extends ADR-3201 (which establishes ztl-ext as a native type) by placing it in a registry rather than as the sole type.

### ADR-3205: Three Stages, Not Two or Four

**Context:** Could be one (text-in/text-out hooks, do everything at one level), two (pre-parse text + post-render HTML), three (add AST in the middle), or four (pre-parse, post-parse, pre-render, post-render).

**Decision:** Three stages: `pre-parse` (text), `transform` (AST), `post-render` (HTML).

**Rationale:**
- **Each stage has a unique operating abstraction.** Text before parsing = preprocessing (Templater-class); AST after parsing = structural transforms (Callouts-class); HTML after rendering = DOM-level (analytics-class).
- **Collapsing stages loses expressiveness.** A text-only world can't distinguish a `[!note]` inside a code block from one that's a legitimate callout. An HTML-only world fights the renderer. An AST-only world can't do preprocessing. Keeping three separates concerns cleanly.
- **Four stages is over-engineered.** A hypothetical post-parse-pre-transform stage adds no new abstraction level beyond `transform`. A pre-render-post-transform stage collapses into `transform` from the hook author's perspective.

**Trade-offs accepted:**
- Each stage is extra surface to document, test, and maintain. Offset by the fact that each stage is simple (one input type, one output type, clear selector semantics).

Status: Proposed.

---

## 7. Purity Boundary Map

### Pure Core

- `hooks::selector::{match_path, eval_frontmatter, run_content_probe, selector_passes}` — pure predicate evaluation.
- `hooks::manifest::{parse, validate, defaults_for_stage}` — pure TOML parsing and validation.
- `hooks::ast::{serialise_to_json, deserialise_from_json, validate_against_schema}` — pure AST conversions.
- `hooks::coverage::{accumulate, summarise_by_hook, format_table, format_json}` — pure event accumulation and reporting.
- `hooks::diagnostic::{format_failure, format_placeholder_html}` — pure rendering of diagnostic HTML fragments.

### Effectful Shell

- `hooks::discovery` — filesystem walk of `<stage>.d/` directories.
- `hooks::runtime` — subprocess spawning (one-shot and persistent), stdin/stdout piping, memory/CPU ceiling enforcement.
- `hooks::persistent_pool` — long-lived process management across pages.
- `hooks::filewatch` (serve mode) — `notify`-based hook-file reloads.
- `hooks::coverage::write_output` — writing `hook-coverage.json` to disk.

### Boundary Contracts

- `SelectorInput { path: PathBuf, frontmatter: Map<String, Value>, text_head: String }` — shell → core per page.
- `HookInvocation { stage, payload, context }` — core-defined shape; shell serialises to JSON.
- `HookResult { payload, diagnostics }` — shell deserialises from JSON; core validates shape.
- `CoverageEvent { hook_id, page_slug, matched: bool, invoked: bool, duration_ms, outcome }` — shell → core.

### Dependency Rule

Core modules MUST NOT import `std::process`, `std::fs::File`, `std::net`, `notify`, or any subprocess crate. The runtime and discovery modules depend on the core modules, never vice versa.

### Enforcement

- Crate-level `[lints]` section forbids `std::process` in `src/hooks/core/`.
- A `#[test]` in `src/hooks/core/mod.rs` uses `compile_fail` assertions to verify no effectful imports leak in.

---

## 8. Test Strategy

### TEST-3201: Three-Stage Execution Order

For a page with a hook registered at each stage, assert invocation order is: `pre-parse` input = file contents; `transform` input = AST of the `pre-parse` output; `post-render` input = HTML rendered from `transform`'s output AST. Use sentinel mutations at each stage to verify data flow.

Mutation kill rate on `src/hooks/pipeline.rs` ≥ 90%.

Verifies: REQ-3201.

### TEST-3201-perf: Selector Hot-Path Latency

Benchmark: 1,000 pages × 10 registered hooks (with varying selectors). Assert P95 selector evaluation ≤ 2 ms / (page × hook).

Verifies: NFR-3201.

### TEST-3202: AST Schema Validation

For a canonical demo vault, render through the pipeline; validate emitted AST against `ztl-ast-schema-v1.json` using a standard JSON Schema validator. Assert 0 validation errors.

Property test: for arbitrary Markdown generated via a QuickCheck-style generator, assert `parse → serialise → deserialise → render_to_html` is byte-equivalent to `parse → render_to_html` (roundtrip property).

Mutation kill rate on `src/hooks/ast/serde.rs` ≥ 85%.

Verifies: REQ-3202.

### TEST-3202-perf: Canonical Extension Overhead

Benchmark: 2,000-page demo vault. Run with all three canonical extensions enabled (persistent mode). Compare total build time to `--no-hooks`. Assert delta ≤ 15%.

Verifies: NFR-3202.

### TEST-3203: Manifest Parsing

Matrix:

| Case                              | Expected                                         |
| --------------------------------- | ------------------------------------------------ |
| Valid manifest, all fields        | Loaded with all fields                           |
| Valid manifest, minimal fields    | Loaded with defaults for unspecified             |
| Missing manifest                  | Defaults + warning                               |
| Unknown field                     | Parse error with clear message                   |
| Invalid `stage` value             | Parse error                                      |
| Invalid TOML syntax               | Parse error with line number                     |

Verifies: REQ-3203.

### TEST-3203-determinism: Build Determinism

Run `ztl build` three times on a fixed vault with fixed canonical extensions. Assert zero byte-differences across runs and across platforms (macOS aarch64 + Linux x86_64 in CI).

Verifies: NFR-3203.

### TEST-3204: Selector Short-Circuiting

For a selector with all four layers configured, assert that:
- Path mismatch → frontmatter not parsed, content not read, AST not built.
- Path match + frontmatter mismatch → content not read, AST not built.
- Path match + frontmatter match + probe mismatch → AST not built.
- All pass → hook invoked.

Instrumented by counting calls to the respective parsers.

Verifies: REQ-3204.

### TEST-3205: Frontmatter Predicate Grammar

Property test: for a grammar fuzzer, assert parseable predicates evaluate without panicking. Example-based tests cover:
- `tags contains "x"` on `tags: [x, y]` → true.
- `status == "draft" && !published` on matching frontmatter → true.
- `word_count > 500` on `word_count: "500"` → false (type-strict).
- `a.b.c == 1` on missing paths → false (null resolves).
- `title matches "^D.*"` with regex dialect → matches.

Mutation kill rate on `src/hooks/predicate/` ≥ 90%.

Verifies: REQ-3205.

### TEST-3205-startup: Zero-Hook Startup Cost

Benchmark on a fixture vault with no `.ztl/hooks/` directory. Compare startup time to a control build (pre-SPEC-032 code or `--no-hooks` flag). Assert delta ≤ 5 ms P95.

Verifies: NFR-3205.

### TEST-3206: Composition and Precedence

Fixture vault with both theme and vault hooks for the same stage. Assert the resulting pipeline order and override semantics match CON-3206's worked example. Add a test for the "empty file disables theme version" edge case.

Verifies: REQ-3206.

### TEST-3207: Failure Scoping

Chain `[10-a.py, 20-fail.py, 30-b.py]`. `20-fail.py` throws. Assert `30-b.py` receives `10-a.py`'s output, not the stage input. Assert the failure is recorded in the diagnostics but the pipeline completes.

`ztl build --hook-fail-on error`: assert build exits non-zero; assert output HTML is still written (failure is reported, not build-aborting for partial output).

Verifies: REQ-3207, NFR-3204.

### TEST-3207-perf: Persistent-Mode Overhead

Benchmark: persistent-mode hook that returns input unmodified. Measure per-page round-trip (serialise AST → write → read → deserialise) on pages with 500 AST nodes. Assert P95 ≤ 10 ms.

Verifies: NFR-3207.

### TEST-3208: Coverage Report

Run build with mixed hook outcomes; assert `hook-coverage.json` content matches a golden schema. Verify `ztl hook coverage --json` emits structurally-equivalent data.

Verifies: REQ-3208.

### TEST-3209: Dry-Run

`ztl hook dry-run transform/callouts --vault demo-vault` against a fixture; assert the matched page list is byte-identical to a golden. Assert exit code 1 when zero pages match.

Verifies: REQ-3209.

### TEST-3210: Helper Library Contract Tests

Python (`ztl-ast-py`): a minimal `identity` transform hook built with the library is run as a real persistent-mode subprocess against the ztl pipeline. Assert round-trip equivalence for a diverse page set.

Same test in JavaScript. Run in CI on every helper-library or schema change.

Verifies: REQ-3210.

### TEST-3211: Per-File Opt-Out

For each canonical extension, create two pages: one with `extensions.<name>: false` in frontmatter, one without. Assert the extension is bypassed on the first and runs on the second.

Verifies: REQ-3211.

### TEST-3212a/b/c: Canonical Extension Golden HTML

Per CON-3212: for each of `callouts`, `tasks`, `admonition`, run the extension against `input.md`, assert the rendered HTML fragment matches `expected.html` byte-for-byte (post-normalisation: collapse whitespace, sort attribute order).

Tasks specifically: fixture includes a multi-page vault with tasks in several files; assert the `tasks` query block on one page aggregates correctly.

Verifies: REQ-3212.

### TEST-3214: Template Variable Publishing

Matrix of behaviours to verify:

| Case                                                           | Expected                                                 |
| -------------------------------------------------------------- | -------------------------------------------------------- |
| Hook emits `template_vars: {total: 12}`                        | `page.ext.<hook_id>.total == 12` in rendered template    |
| Two hooks emit disjoint vars                                   | Both visible at their respective `page.ext.<id>` paths   |
| Same hook runs `transform` then `post-render`, both emit vars  | `post-render` values win; warning logged                 |
| Hook emits vars > 1 MiB                                        | Vars dropped with warning; AST/HTML payload still used   |
| Hook emits `template_vars: null` or omits the field            | `page.ext.<hook_id>` absent; template `if` checks false  |
| Template renders `{{ page.ext.unknown.field }}` with undefined | Renders empty (Minijinja default); no error              |
| String value contains `<script>` tags                          | Autoescape applies; output is `&lt;script&gt;` in HTML   |

Integration test: a canonical `tasks` extension emits stats; a test theme template reads them and renders a pill; assert golden HTML.

Verifies: REQ-3214.

### TEST-3213: Matrix Gate

CI simulation: PR that downgrades a canonical extension's tier or removes a fixture. Assert the check fails with an actionable message.

Verifies: REQ-3213.

### TEST-3222: Pre-Parse Structural Safety Diff

Fixture: a pre-parse hook that wraps every paragraph in an HTML
`<div>` (restructures the block tree). Manifest declares
`contract.may_restructure = false`. Assert ztl emits a
`contract_violation:may_restructure` diagnostic with the specific
paragraph → div path in the reason field. Flip manifest to
`may_restructure = true`; assert no diagnostic.

Verifies: REQ-3222, REQ-3224.

### TEST-3224: Preservation Contract Diagnostic

Fixture: a transform hook that strips all `Wikilink` nodes from
input. Manifest declares `contract.preserves = ["Wikilink"]`.
Assert ztl emits a `contract_violation:preserves` diagnostic
enumerating the count delta and the affected page. Hook output is
discarded per REQ-3207; next hook in the pipeline receives the
unmodified input.

Matrix of `preserves` declarations tested: empty (no enforcement),
single type (Wikilink), multiple types (Wikilink + Embed +
CodeBlock), unknown type (`"NotAType"` — manifest parse error).

Verifies: REQ-3224.

### TEST-3224-idempotent: Idempotence CI Double-Run

For every hook with `contract.idempotent = true`, CI runs the hook
twice on the matrix's fixture page and asserts
`canonicalise(f(f(input))) == canonicalise(f(input))`. A hook
flagged `idempotent = true` that fails this check is a tier
downgrade (`supported` → `partial`) and a gated CI failure.

Verifies: REQ-3224.

### TEST-3225: Authoring CLI Integration

Matrix:

| Subcommand                              | Fixture / assertion                                           |
| --------------------------------------- | ------------------------------------------------------------- |
| `ztl hook new transform foo --lang py` | scaffold appears at expected paths; `ztl hook test foo` passes before any edits |
| `ztl hook test <existing>` no-op       | diff is empty; exit 0                                         |
| `ztl hook test <existing>` after edit  | diff shown; exit 1                                            |
| `ztl hook test --update`               | golden regenerated; exit 0                                    |
| `ztl hook fixture --from projects/q2`  | `tests/hook-fixtures/<hook>/input.md` matches page content    |
| `ztl hook watch` restart               | editing hook source triggers one restart within 500 ms        |
| `ztl ast sample <file.md>`             | output validates against ztl-ast-schema-v1.json              |
| `ztl ast diff a.json b.json`           | tree-diff identifies known mutations; exit 1 on non-empty diff |

**Diagnostic-shape fixtures:** TEST-3225 also covers CON-3225 by
asserting every failure-class diagnostic (manifest parse error,
selector evaluation failure, contract violation, runtime absence,
protocol error) renders with all five parts (summary, context,
observed data, likely cause, remediation hint). Regression gate:
any diagnostic emitted by the hook subsystem without the full
shape is a CI failure.

Verifies: REQ-3225, CON-3225.

### Fuzzing — Predicate and Manifest Parsers

`cargo-fuzz` targets on `hooks::predicate::parse` and `hooks::manifest::parse`. Run 24h nightly. Assert no panics, no UB.

### Fuzzing — AST Deserialisation

`cargo-fuzz` target on `hooks::ast::deserialise_from_json`. Feeds random JSON; asserts either a successful AST or a typed error — never a panic.

### Synthetic-User Simulation

Profiles 2.1 (migrant), 2.2 (custom transform author), 2.3 (theme author) walked against this draft. Findings converted to REQ/NFR amendments before status → `approved`.

---

## 9. Observability

### Metric emission surface

The OBS items below name Prometheus-style metric families. The
**emission surface** — where those numbers become visible — is:

1. **Primary user-facing surface: `ztl hook coverage [--json]`
   (REQ-3208).** Rolls up the metric families into a per-hook table:
   match/invoke counts, P50/P95 latency, failure counts, reasons. This
   is what humans consume.
2. **Structured log lines on stderr under `--verbose`.** Each OBS
   item below names the log-line shape it emits at that verbosity.
3. **A full `/metrics` Prometheus endpoint is out of scope for this
   spec.** Adding one requires the `ztl serve` process to expose
   HTTP metrics infrastructure (counters, histograms, labels — none
   of which ztl currently wires up); that's a separate SPEC.

Metric-family names below are reserved and their shapes frozen as
contract — a future Prometheus-endpoint spec can wire them up without
redesigning the semantics. SPEC-033 OBS items (OBS-3301–3305) follow
the same surface convention.

### OBS-3201: Hook Discovery

Log line at ztl startup (once): `[ztl] hooks: discovered <N> in <stage>.d/ (theme=<M>, vault=<K>)` for each stage.
Metric: `ztl_hooks_discovered_total{stage, source}`.

### OBS-3202: Selector Evaluation

Metric: `ztl_hook_selector_latency_seconds{hook_id, layer}` histogram, where `layer` ∈ {path, frontmatter, probe, total}.
Under `--verbose`: `[ztl] hook selector: <id> matched=<bool> evaluated_layers=<N> duration_ms=<M>`.

### OBS-3203: Selection Hit Rate

Metric: `ztl_hook_selector_match_total{hook_id, outcome}` counter, `outcome` ∈ {matched, excluded_path, failed_frontmatter, failed_probe}.

This feeds the coverage report and is the principal "is this hook earning its place?" signal.

### OBS-3204: Hook Invocation Latency

Metric: `ztl_hook_invoke_duration_seconds{hook_id, mode}` histogram, `mode` ∈ {one-shot, persistent}.
Under `--verbose`: `[ztl] hook invoke: <id> page=<slug> duration_ms=<M>`.

### OBS-3205: Failure Rate

Metric: `ztl_hook_failure_total{hook_id, reason}` counter, `reason` ∈ {non_zero_exit, timeout, memory_exceeded, malformed_output, crash}.
Log line per failure (severity `warn` under default, `error` under `--hook-fail-on error`).

### OBS-3206: End-to-End Render Overhead

`ztl build` summary line: `[ztl] hooks: total_invocations=<N> total_duration_ms=<M> failures=<K>`.
Delta against a `--no-hooks` baseline available in the coverage report.

### OBS-3207: Persistent-Mode Protocol Overhead

Metric: `ztl_hook_protocol_duration_seconds{hook_id, phase}` histogram, `phase` ∈ {serialise, transport, deserialise}.

### OBS-3208: Memory

Metric: `ztl_hook_memory_high_water_mib{hook_id}` gauge, sampled at each persistent-mode invocation boundary.

### OBS-3209: Template Variable Emission

Metric: `ztl_hook_template_vars_total{hook_id, outcome}` counter, `outcome` ∈ {emitted, dropped_oversize, overwritten_by_later_stage}.
Log line (debug): `[ztl] hook template_vars: <hook_id> page=<slug> bytes=<N> keys=<K>`.
Coverage report (REQ-3208) includes a per-hook `template_var_bytes_avg` column for visibility into extension payload sizes.

Trace: REQ-3214.

---

## 10. Security Considerations

- **Untrusted code execution.** Hooks are arbitrary user/theme-author code. ztl treats them as untrusted.
  - **Isolation:** out-of-process; no shared-memory primitive; subprocess stdin/stdout only.
  - **No privilege escalation:** hook inherits the user's UID/GID; ztl does not gate per-hook permissions in v1 (future work).
  - **Filesystem access:** a hook has whatever filesystem access the user running `ztl` has. ztl does not jail hooks. Documentation notes this explicitly.
  - **Network:** same — hooks can open sockets. Users who want stricter isolation should run ztl under their own sandboxing (bwrap, Firejail, Docker).
- **Selector as DoS vector.** A pathological regex in `content_probe` could be catastrophic (ReDoS). Mitigation: use a regex engine with linear-time guarantees (`regex` crate); reject patterns that compile to non-linear. Fuzz test the probe path.
- **Manifest parse.** Malformed TOML is user error. Parser hardening via TOML crate + `cargo-fuzz`.
- **AST JSON deserialisation.** Hooks send JSON back; ztl deserialises. Untrusted-JSON attacks (deeply nested objects, huge arrays) mitigated by (a) 10 MiB max message size, (b) recursion depth limit of 256 (matches CommonMark's nesting cap).
- **Threat model (v1) — out of scope:** targeted exfiltration by a theme author's malicious hook (same risk posture as Obsidian plugins or VS Code extensions; user consent to the theme is consent to its hooks).
- **AI Trust Boundary classification:** Tier 3 (standard feature code; untrusted-input processing at the JSON protocol boundary). Implementation requires same-model fresh-context review OR cross-model review. Not Tier 1 — no cryptography or authentication in scope.

---

## 11. Documentation Plan

- **README.md** — new "Extension hooks" section: three stages, selector basics, canonical extensions list, how to disable.
- **CHANGELOG.md** — entry for the new stages, the AST schema, and the supersession of SPEC-031.
- **`docs/hook-authoring.md`** — primary tutorial. Walks through writing a Python hook with `ztl-ast-py`, writing the manifest, running `ztl hook new` / `test` / `watch`, iterating. Structured as a five-minute first-hook path followed by deeper topics (selectors, contracts, ecosystem hooks).
- **`docs/ztl-ast-reference.md`** — auto-generated from `tools/ztl-ast-schema-v1.json` at CI time (REQ-3202). One section per node type with shape, attrs, canonical example, and HTML-rendering expectations. Never hand-edited; a schema/reference discrepancy is a CI failure.
- **`docs/hook-diagnostics.md`** — a catalogue of every diagnostic class the hook subsystem can emit (manifest parse errors, selector evaluation failures, contract violations, protocol errors, ecosystem runtime absence). Each entry gives the summary-line format, a worked example following CON-3225's five-part structure, and a remediation section.
- **`docs/hook-migration.md`** — for any (hypothetical) SPEC-031 users: there's nothing to migrate; SPEC-031 never shipped. This doc exists only to anchor the link from SPEC-031's `superseded-by` field.
- **`docs/canonical-extensions.md`** — per-extension reference for Callouts, Tasks, Admonition (now CSS-stubs per REQ-3212), covering syntax, selector, configuration, per-file opt-out, HTML output shape, and which ecosystem plugin provides the transformation.
- **Theme authoring reference update (SPEC-028's pattern)** — add the `hooks/<stage>.d/` convention, the empty-file-disables rule (probe-based post-amendment), and the REQ-3223 `[[theme.hooks]]` declaration requirement.

---

## 12. Rollout Plan

**Release target mapping (working estimate — revise per actual landings):**

| Phase | Description (below)                  | Target release | Depends on       |
| ----- | ------------------------------------ | -------------- | ---------------- |
| A     | Plumbing                             | 0.6.0          | —                |
| B     | Helper libraries                     | 0.6.0 or 0.7.0 | A                |
| C     | Theme-stub extensions (CSS only)     | 0.7.0          | A; SPEC-033 B    |
| D     | Feature-flag retirement (paper-only) | 0.8.0          | C stable 2 rel.  |
| E     | SPEC-031 supersession                | 0.6.0          | A landed         |

SPEC-033 Phase A can co-ship with SPEC-032 Phase A (shared protocol
surface); SPEC-033 Phase B (Pandoc adapter) is a precondition for
SPEC-032 Phase C (theme-stub extensions need an ecosystem plugin to
provide the transform — Callouts/Admonition via `mdbook-admonish` or
Pandoc div syntax, Tasks deferred per §13 Q4).

**Phase A — Plumbing:** AST schema, JSON Schema published, selector evaluator, manifest parser, one-shot + persistent protocol, coverage + dry-run subcommands. No canonical extensions yet. Originally planned to ship behind a `--features hooks-v2` cargo flag for one preview release; in practice the plumbing landed unconditionally (no compile-time gate ever wired) because the schema converged faster than expected and the gate would have shipped as a no-op. Phase D below is therefore a documentation-only step rather than a code change.

**Phase B — Helper libraries:** `ztl-ast-py` and `ztl-ast-js` published to PyPI and npm. Version pinned to AST schema v1.0.

**Phase C — Canonical extensions:** `callouts` first (smallest and highest ROI). Golden-HTML tests. `tasks` and `admonition` follow.

**Phase D — Feature-flag retirement (completed 0.8.0):** the `--features hooks-v2` umbrella was never wired as a real cargo feature (see Phase A note), so retirement is a paper exercise: the flag is acknowledged as default-on from first release, no `--no-hooks-v2` opt-out is added (there is nothing to opt out of), and the changelog records the umbrella as retired. The original two-clean-releases gate on canonical extensions still applies as a stability check before the spec is allowed to drop the umbrella mention entirely.

**Phase E — SPEC-031 supersession:** SPEC-031 marked `superseded` with `superseded-by: SPEC-032`. Top-50 scan findings retained. No code changes required (SPEC-031 never shipped).

**Rollback:** if the AST schema proves unworkable post-release, bump to v2, keep v1 emission behind a flag for one release, deprecate. No user code path is lost.

---

## 13. Open Questions

1. **Python vs. JS as the default canonical-extension language.** Default theme ships canonical extensions in one language — which? Python is more familiar to the data/notes-writing crowd; JS is already in the theme (graph widget, SPA shell). *Proposed:* Python. Revisit if CI-minimal-environment feedback says otherwise.
2. **Persistent-mode restart triggers in serve.** Should edits to a hook file trigger a persistent-mode restart automatically, or require a user action? *Proposed:* automatic on file change (filewatch), with a `--no-hook-reload` flag to opt out.
3. **AST-level opt-out vs. frontmatter opt-out.** Some users may want to mark a single block as "don't transform" via a Markdown-level syntax (e.g., `{: .ztl-skip }`) rather than frontmatter. *Proposed:* defer; frontmatter covers the 80% case. Reconsider if demand emerges.
4. **Tasks extension query syntax.** How much of Obsidian Tasks' DQL to support? *Proposed:* document a named subset (`not done`, `due before`, `path includes`, `tag includes`) as "supported"; anything else is "unsupported, will render as an unsupported-query placeholder." Avoid chasing the full grammar.
5. **Dataview as a fourth canonical extension.** Worth shipping a subset as `dataview`? *Proposed:* defer to a separate SPEC once canonical three are stable. Dataview's query language deserves its own design conversation.
6. **Multi-output hook stages.** Should a `transform` hook be able to emit *multiple* output ASTs (e.g., splitting a page into two)? *Proposed:* no in v1; single-in / single-out per stage. Multi-output is a page-generation concern, not a transform concern.
7. **MCP exposure of hook coverage.** `get_page` via MCP could include which hooks touched the page in a metadata field. *Proposed:* off by default; opt-in via MCP tool parameter.
8. **Selection layer cache.** Should selector evaluation results be cached across pages (e.g., a content probe compiled regex cached, a frontmatter parse reused)? *Proposed:* yes, cache per build; invalidate on file change in serve mode.
9. **Vault-level template vars.** Should there be a `vault.ext.<hook_id>` namespace aggregated across pages (dead-link totals, tag clouds, cross-vault task summaries)? This requires a finalisation pass after all pages render and a distinct protocol message (`{"type": "finalise", ...}` for persistent-mode hooks, or a separate invocation for one-shot). *Proposed:* defer to v1.1; per-page `page.ext` covers the majority case without the finalisation complexity.
10. **Schema declarations for template vars.** Should extensions declare the shape of their emitted vars in the manifest so themes can introspect and ztl can validate? *Proposed:* no in v1; loose typing + documentation suffices. Reconsider if theme-authoring feedback shows "I don't know what fields this extension emits" is a real pain point.
11. **Template-var publishing from `pre-parse` hooks.** `pre-parse` runs before parsing, so extensions there have only the raw Markdown text — less rich than AST-stage extensions. Is it worth supporting emission at that stage, or restrict template-vars to `transform` and `post-render`? *Proposed:* allow at all three stages; the preprocessing stage has legitimate use (e.g., a word-counter that emits `page.ext.word_count.value` without parsing cost).
12. **Executable-bit fallback vs. interpreter map.** Pandoc's filter loader guesses an interpreter from extension when the executable bit is absent (`.py→python`, `.js→node`, `.hs→runhaskell`, etc.; see [filters.html](https://pandoc.org/filters.html)). Should ztl replicate this for authoring ergonomics, or require the executable bit (unambiguous, prevents "why isn't my hook running")? *Proposed:* require the executable bit in v1 — clear error, good diagnostic. Add an extension-to-interpreter map in v1.1 if feedback requests it.
13. **Symbol-scanner analogue for canonical extensions.** SPEC-031 had a symbol-scanner subcommand to prioritise shim work against the Obsidian plugin ecosystem. For SPEC-032's canonical extensions, the analogous question is "which patterns in the wild would benefit from a canonical extension we ship?" — answerable by scanning a sample of published vaults for `> [!`, `ad-*`, `tasks`, inline queries, etc. *Proposed:* defer to a tools-level follow-up, not a spec requirement.
14. **Global (user-level) hook discovery.** Should REQ-3206 extend the
    discovery path set to include a user-level location
    (`$XDG_CONFIG_HOME/ztl/hooks/<stage>.d/*`), matching the
    install-once-use-everywhere ergonomic of `cargo install` /
    `npm -g`? The trade-off is determinism and audit: NFR-3203 /
    SPEC-033 NFR-3306 promise byte-identical HTML across runs and
    machines; a global-hook layer breaks that unless
    carefully scoped. REQ-3223's `[[theme.hooks]]` declaration also
    doesn't cover global-installed hooks — they'd be invisible to
    a vault auditor. *Resolved 2026-04-19: defer entirely for v1 —
    keep discovery vault+theme only.* Ecosystem plugins (SPEC-033)
    already cover the shared-toolkit ergonomic at the PATH layer;
    ztl-native hooks distribute via git-clone-into-`.ztl/hooks/`
    without ztl-side orchestration. Reconsider in v1.1 with an
    explicit-allow-list shape (vault config names each global
    hook) if user feedback shows the friction is real.

---

## 14. Traceability Summary

| REQ      | Tests                          | Contracts  | ADRs         | OBS         |
| -------- | ------------------------------ | ---------- | ------------ | ----------- |
| REQ-3201 | TEST-3201, 3201-perf           | CON-3201   | ADR-3205     | OBS-3201    |
| REQ-3202 | TEST-3202                      | CON-3202   | ADR-3201     | —           |
| REQ-3203 | TEST-3203                      | CON-3203   | —            | —           |
| REQ-3204 | TEST-3204                      | CON-3204   | —            | OBS-3203    |
| REQ-3205 | TEST-3205                      | CON-3205   | —            | —           |
| REQ-3206 | TEST-3206                      | CON-3206   | —            | —           |
| REQ-3207 | TEST-3207                      | CON-3207   | —            | OBS-3205    |
| REQ-3208 | TEST-3208                      | CON-3208   | —            | —           |
| REQ-3209 | TEST-3209                      | —          | —            | —           |
| REQ-3210 | TEST-3210                      | CON-3210   | —            | —           |
| REQ-3211 | TEST-3211                      | CON-3211   | —            | —           |
| REQ-3212 | TEST-3212a, 3212b, 3212c       | CON-3212   | ADR-3204     | —           |
| REQ-3213 | TEST-3213                      | —          | —            | —           |
| REQ-3214 | TEST-3214                      | CON-3201, CON-3214 | —    | OBS-3209    |
| REQ-3215 | TEST-3215                      | CON-3201   | ADR-3201     | OBS-3201    |
| REQ-3216 | TEST-3216                      | CON-3216   | —            | OBS-3201    |
| REQ-3217 | TEST-3217                      | CON-3217   | —            | —           |
| REQ-3218 | TEST-3218                      | CON-3218   | —            | —           |
| REQ-3219 | TEST-3219                      | CON-3219   | —            | —           |
| REQ-3220 | TEST-3220                      | CON-3220   | —            | —           |
| REQ-3221 | TEST-3221                      | CON-3221   | ADR-3206     | —           |
| REQ-3222 | TEST-3222                      | CON-3224   | —            | —           |
| REQ-3223 | TEST-3223                      | CON-3223   | —            | —           |
| REQ-3224 | TEST-3224, 3224-idempotent, 3222 | CON-3224 | —            | —           |
| REQ-3225 | TEST-3225                      | CON-3225   | —            | —           |
| NFR-3201 | TEST-3201-perf                 | —          | ADR-3203     | OBS-3202    |
| NFR-3202 | TEST-3202-perf                 | —          | —            | OBS-3206    |
| NFR-3203 | TEST-3203-determinism          | —          | ADR-3201     | —           |
| NFR-3204 | TEST-3207                      | —          | —            | OBS-3205    |
| NFR-3205 | TEST-3205-startup              | —          | —            | —           |
| NFR-3206 | (process — CHANGELOG)          | —          | ADR-3201     | —           |
| NFR-3207 | TEST-3207-perf                 | —          | ADR-3202     | OBS-3207    |
| NFR-3208 | TEST-3208 (memory branch)      | —          | ADR-3202     | OBS-3208    |

---

## 15. Quality Gate Self-Check

- [x] Requirements unambiguous — each uses SHALL with measurable criteria.
- [x] Requirements verifiable — every REQ has at least one TEST reference.
- [x] Requirements atomic — each is one obligation.
- [x] No internal conflicts.
- [x] Components have single responsibility — selector/manifest/ast/runtime/coverage.
- [x] Functionality via well-defined interfaces — CON-3201 through CON-3212.
- [x] Tests derived from requirements — traceability table.
- [x] Security controls specified — §10 explicit threat model.
- [x] Observability captured — OBS-3201 through OBS-3208.

**Not yet cleared:**

- [ ] Stakeholder validation (open questions §13).
- [ ] Adversarial review from a fresh context.
- [ ] Synthetic-user simulation findings.
- [ ] JSON Schema for the AST written and validated (currently referenced but not yet drafted; lives at `tools/ztl-ast-schema-v1.json`, to be produced in Phase A).
- [ ] Helper-library API sketch (Python, JS) reviewed by respective ecosystem experts.
- [ ] Prior-art literature survey (`tools/parser-lit-survey.md`) — complete (2026-04-19), findings folded into §1.4 Prior Art, REQ-3215..3220, and CON-3201/3202 updates.

Status remains `draft` until these clear.

---

**End of SPEC-032.**
