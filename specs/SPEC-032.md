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

`zetl` already ships a lifecycle hook system — executable scripts at `pre-build`, `post-build`, `post-index`, `on-save`, and others, receiving JSON context on stdin. What it does *not* have is a per-page, per-render extension surface: a way for users (or first-party canonical extensions shipped with the default theme) to transform a page's content during rendering, not after it. This gap is what users ask for when they ask for "Obsidian plugin support" — the *published output* of post-processor plugins like Callouts, Tasks, and Admonition.

SPEC-031 explored running actual Obsidian plugins under an embedded JavaScript runtime. A top-50 community plugin scan (2026-04-19) showed that bet was worse than it looked: zero top-50 plugins are pure `MarkdownPostProcessor` (the only class a bounded v1 shim could render), 24% are hybrids whose post-processor is entangled with editor extensions, and 76% are unusable in any shim. The engineering cost (3–6 months, QuickJS supply chain, shim surface chasing a moving API) did not justify the coverage.

This spec takes the opposite direction: zetl defines its own render-pipeline extension contract, keeps Obsidian compatibility as a matter of *visual parity for canonical patterns* rather than *code execution*, and ships the patterns users actually want (Callouts, Tasks, Admonition) as first-party extensions against that contract.

The contract has three stages, each with a different data shape reflecting the stage's responsibility:

| Stage         | Data in            | Data out           | Typical use                                          |
| ------------- | ------------------ | ------------------ | ---------------------------------------------------- |
| `pre-parse`   | raw Markdown text  | raw Markdown text  | Templater-style preprocessing, includes, variables   |
| `transform`   | zetl-AST JSON      | zetl-AST JSON      | Callouts, Tasks, Admonition, Dataview-subset         |
| `post-render` | HTML fragment      | HTML fragment      | DOM injection, analytics, external-CSS-class fixups  |

Each stage supports composition via directory-of-drop-ins, theme-then-vault precedence, and local failure scoping. **File selection is first-class**: every hook declares a selector (path globs, frontmatter predicates, cheap content probes) that zetl evaluates before invoking the hook, so extensions skip the pages they don't care about without paying serialisation cost or process-spawn overhead.

### 1.1 Motivation

- **The Obsidian-migrant user (SPEC-031 Profile 2.1) still needs answering.** They want Callouts, Tasks, Admonition rendering when they publish their vault with `zetl build`. The scan showed those patterns are small, tractable, and author-able in under 200 lines each.
- **Operate on the right abstraction.** Text hooks (SPEC-031 ADR-3102 alternative) get fooled by code blocks and quoted prose. HTML hooks fight the renderer. AST hooks match the *structure* we mean — BlockQuote-whose-first-paragraph-starts-with-`[!note]` — making extensions reliable across renderer changes.
- **First-class selection = real performance.** In a 2,000-page vault with 10 enabled extensions, running every extension on every page is 20,000 invocations. Most of those are pages the extension doesn't touch. A declared selector evaluated cheaply (path match → frontmatter probe → content substring probe) reduces that to the actual working set, often 100× smaller.
- **Selection is also semantics.** "This folder runs the Tasks extension, that folder doesn't" is a real governance boundary in team vaults. Per-file frontmatter opt-out (`extensions.callouts: false`) is how users escape-hatch individual pages. Both are user-visible contracts, not implementation details.
- **Protocol stability, not shim stability.** Defining our own versioned AST JSON schema couples us to CommonMark (stable) rather than Obsidian's API (moving). Maintenance cost is in a schema we control.
- **Out-of-process is ergonomic for hook authors.** Python, Node, Ruby, Go, Rust — any language with a JSON library and an HTTP-stream-of-consciousness mental model can author a hook. No cargo feature flag, no QuickJS upgrade risk. First-party helper libraries (`zetl-ast-py`, `zetl-ast-js`) close most of the ergonomic gap with in-process APIs.

### 1.2 Design Principles

1. **Operate on the right stage.** Text for preprocessing (where the AST doesn't exist yet), AST for structural transforms (the abstraction we mean), HTML for DOM-level concerns (analytics, external-tool classnames). Refuse to collapse them into one.
2. **Selection is first-class.** Every hook has a selector. Zero-selector (runs on everything) is explicit and warned, not the default. Selectors are declarative, file-based, and evaluable without invoking the hook.
3. **Declarative over imperative.** Manifests describe stage, selector, mode, budget. Zetl orchestrates; hooks transform.
4. **Versioned contracts, not best-effort APIs.** The AST JSON schema has a semver. Helper libraries pin a major. Breaking changes require a major bump and a migration path.
5. **Out-of-process by default.** Hook failures, OOM, and crashes cannot damage `zetl`'s process. The ergonomic gap is closed by helper libraries, not by in-process execution.
6. **Composition by piping, not by coordination.** Each hook is a filter. Failures are scoped to one hook; the chain continues. No hook-to-hook communication primitives in v1.
7. **Theme-shipped extensions are first-class.** The default theme ships canonical extensions (Callouts, Tasks, Admonition) in `themes/default/hooks/`. Users override by filename collision or disable by empty file. Same mechanism as existing theme-bundled hooks.
8. **Build/serve parity.** A page rendered under `zetl build` at time T with a fixed extension set produces the same HTML as `zetl serve` rendering that same page at time T with the same extension set. Extensions must be deterministic; the render pipeline is.
9. **Obsidian compatibility is a by-product, not a goal.** The three stages are designed for zetl's own extension needs. That the same architecture happens to re-implement the Obsidian patterns users want is confirmation, not constraint.

### 1.3 Scope

**In scope:**

- Three new hook stages: `pre-parse`, `transform`, `post-render`, each with one-shot and persistent execution modes.
- A zetl-AST JSON schema: CommonMark subset + wikilink/embed/SPL-block extensions + frontmatter + source positions. Versioned.
- Helper libraries for Python (`zetl-ast-py`) and JavaScript/TypeScript (`zetl-ast-js`), published alongside the zetl release, pinned to the AST schema major.
- A hook manifest format (`<hook-path>.toml`) declaring stage, selector, mode, timeout, memory limit.
- Selector DSL: path globs (include/exclude), frontmatter predicates (dotted-path plus operator), content probes (substring or regex on raw text), with a defined precedence for cheap-to-expensive evaluation.
- Composition: `.d/` directory discovery, filename-sorted piping, theme-then-vault precedence, user-overrides-theme by filename collision.
- Persistent-mode line-delimited JSON protocol (spec'd in CON-3201).
- Subcommand `zetl hook dry-run <stage>/<name> [--vault PATH]` — evaluates the selector, prints matched pages, does not invoke the hook. For authoring iteration.
- Subcommand `zetl hook coverage [--vault PATH] [--json]` — reports, for the last build, which hooks matched which pages, average latency, failure counts.
- Failure semantics: local to one hook, chain continues with previous hook's output.
- Three first-party canonical extensions shipped with the default theme: `callouts`, `tasks`, `admonition`. Each has a golden-HTML test and a published selector.
- Documentation: new README section, theme-authoring updates, AST schema reference, helper-library quickstart, migration guide from existing hooks to the new stages.
- Observability: per-hook latency, match-rate, failure-rate metrics; `--verbose` trace output per invocation.

**Out of scope:**

- In-process execution (embedded scripting language). Reserve for a successor spec if live-hackability becomes a product goal.
- AST-level hooks with parser-native formats (pulldown-cmark events, rustdoc-markdown, etc.). The zetl-AST JSON is a stable contract; the underlying parser may change.
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

**Role:** Existing Obsidian user with a 500-page vault using Callouts, Tasks, and Admonition syntax. Wants to publish via `zetl build`.
**Goal:** Published HTML visually matches what they see in Obsidian Publish for those three features.
**Constraints:** No willingness to edit Markdown source; CI pipeline; prefers zero configuration.

**Happy path:**

1. User runs `zetl build --out-dir dist --theme default`.
2. Default theme ships `themes/default/hooks/transform.d/10-callouts.py`, `20-tasks.py`, `30-admonition.py` with selectors pre-configured (Callouts runs on any page containing `> [!`; Tasks runs on any page with a `tasks` fenced block; Admonition on any with `ad-*` fenced blocks).
3. Output renders visually equivalent to Obsidian Publish for these three patterns. `dist/obsidian-diagnostics.json` records per-extension match counts.
4. `zetl hook coverage --vault .` confirms coverage: `callouts matched 87/500, tasks matched 12/500, admonition matched 4/500`.

**Failure modes:**

- A page has malformed callout syntax → callouts extension leaves the block unchanged, diagnostic logged, build continues.
- User has vault-level opt-out of a canonical extension → they create an empty `.zetl/hooks/transform.d/10-callouts.py` which shadows (by filename collision) the theme-shipped one; that extension is effectively disabled.

### 2.2 User: Vault author writing a custom transform

**Role:** Power user who wants an extension that converts `{{cite:bibkey}}` inline tokens into formatted citations pulled from a BibTeX file.
**Goal:** Ship a 100-line Python script that handles this cleanly, without fighting the render pipeline.
**Constraints:** Comfortable in Python; wants AST-level access (to avoid mangling code blocks or URLs); wants the extension to run only on pages with `bibliography` in frontmatter.

**Happy path:**

1. User writes `.zetl/hooks/transform.d/citations.py` using `zetl-ast-py`:
   ```python
   from zetl_ast import run, walk, Text
   def transform(ast, context):
       for node in walk(ast, type=Text):
           node.text = expand_cites(node.text, context.frontmatter["bibliography"])
       return ast
   run(transform)
   ```
2. User writes `.zetl/hooks/transform.d/citations.toml`:
   ```toml
   stage = "transform"
   frontmatter_where = "bibliography != null"
   mode = "persistent"
   timeout_ms = 200
   ```
3. `zetl hook dry-run transform/citations` prints 23 pages matched, zero invocations (selector only).
4. `zetl build` invokes the hook only on those 23 pages.
5. User iterates by editing Python; persistent-mode restart is automatic on file change in `zetl serve`.

**Failure modes:**

- Hook crashes on a specific page → that page's AST is passed unchanged to the next hook, diagnostic logged.
- Selector matches zero pages → warning logged once: `hook citations matched 0 pages; did you mean ...?`.

### 2.3 User: Theme author

**Role:** Customising `.zetl/themes/scholar/` for an academic vault.
**Goal:** Ship a theme that includes citation rendering, disables Tasks (academic vaults don't need task dashboards), and modifies Callouts styling.
**Constraints:** Only edits files inside `.zetl/themes/scholar/`; no Rust.

**Happy path:**

1. Theme ships `themes/scholar/hooks/transform.d/10-callouts.py` (copied from default theme, restyled).
2. Theme ships empty `themes/scholar/hooks/transform.d/20-tasks.py` (empty file = disabled).
3. Theme ships `themes/scholar/hooks/transform.d/15-citations.py` (new extension between callouts and tasks-would-be).
4. User of this theme runs `zetl build --theme scholar`; citations run, tasks doesn't, callouts runs with scholar's styling.

**Failure modes:**

- Theme's Python script errors on an edge case → per-page diagnostic; theme author iterates using `zetl hook dry-run --theme scholar`.

### 2.4 User: CI operator enforcing extension coverage

**Role:** Team running `zetl build` in CI for a collaboratively-maintained vault.
**Goal:** Fail the build if any enabled canonical extension (Callouts, Tasks, Admonition) crashes, even if zetl's default is to soft-fail with diagnostic.
**Constraints:** Strict build semantics; wants to gate merges on extension health.

**Happy path:**

1. CI invokes `zetl build --hook-fail-on error` (new flag).
2. Build fails with exit code 2 if any hook exits non-zero; the diagnostic page list is printed to stderr.
3. CI operator receives actionable output: `hook callouts failed on pages: projects/q2-review.md, notes/daily-2026-04-18.md`.

**Failure modes:**

- Legitimate transient failure (hook OOM on one huge page) → CI flake; mitigation is per-page retry at the hook-runtime level (out of scope for v1; document).

---

## 3. Functional Requirements

### REQ-3201: Three Hook Stages

The system SHALL expose three new hook stages in the render pipeline:

- **`pre-parse`** — invoked after the raw Markdown source is read from disk, before frontmatter parsing or Markdown AST construction. Input/output: UTF-8 Markdown text.
- **`transform`** — invoked after Markdown parsing produces the zetl-AST, before AST-to-HTML rendering. Input/output: zetl-AST JSON.
- **`post-render`** — invoked after AST-to-HTML rendering produces the per-page content fragment, before Minijinja template composition into the full page. Input/output: HTML fragment string.

Stage ordering within a single page render is fixed: `pre-parse` → parse → `transform` → render → `post-render` → template compose.

**Pre-parse caveat (author guidance):** The `pre-parse` stage operates on raw Markdown source text. The hook author is responsible for preserving Markdown's block/inline structure — naive regex edits can easily break downstream parsing. mdBook's own documentation warns: "chapter.content is just a string which happens to be markdown. While it's entirely possible to use regular expressions or do a manual find & replace, you'll probably want to process the input into something more computer-friendly" ([mdBook dev guide](https://rust-lang.github.io/mdBook/for_developers/preprocessors.html)).

- **Valid use cases:** Template-variable expansion (`{{ page.title }}`), include-directive resolution, syntax-sugar replacement where the input/output are both Markdown-valid.
- **Anti-patterns:** Adding emphasis or block structure by find-replace (use a `transform` hook instead); naive HTML injection (breaks on being subsequently parsed as Markdown); regex matching of `[[wikilinks]]` without respecting code-block fences.

When in doubt, authors should prefer `transform` stage — it operates on the parsed AST where context (inside-code-block, inside-link, etc.) is explicit.

Trace:
- TEST-3201
- CON-3201

### REQ-3202: zetl-AST JSON Schema

The system SHALL define and publish a versioned JSON schema (`zetl-ast-schema`) for the intermediate AST representation used by the `transform` stage. The schema SHALL cover:

- All CommonMark block types: `Document`, `Heading`, `Paragraph`, `BlockQuote`, `List`, `ListItem`, `CodeBlock` (fenced and indented), `ThematicBreak`, `HtmlBlock`.
- All CommonMark inline types: `Text`, `Emphasis`, `Strong`, `Code`, `Link`, `Image`, `LineBreak`, `SoftBreak`, `HtmlInline`.
- zetl extensions: `Wikilink` (with target, alias, heading, block-id fields), `Embed` (for `![[...]]` transclusions), `SplBlock` (for `spl` fenced code blocks, if any are to be treated specially), `FrontMatter` (parsed YAML as JSON object at the document root).
- Source positions: every node has `start_line`, `start_col`, `end_line`, `end_col`.
- Schema version: every document declares `ast_version: "<major>.<minor>"` at the root.

The schema SHALL be published at `tools/zetl-ast-schema-v1.json` in JSON Schema Draft 2020-12 format, and SHALL be used by the helper libraries (REQ-3210) as the source of truth.

Trace:
- TEST-3202
- CON-3202
- ADR-3201

### REQ-3203: Hook Manifest Format

Every hook executable in a `<stage>.d/` directory SHALL have an optional sibling manifest file named `<executable>.toml`. The manifest declares stage metadata, selector, execution mode, and budgets:

```toml
# .zetl/hooks/transform.d/tasks.toml
stage = "transform"                 # enum: "pre-parse" | "transform" | "post-render"
mode = "persistent"                 # enum: "one-shot" | "persistent"; default "one-shot"
timeout_ms = 100                    # int; default 100
memory_mib = 64                     # int; default 64
ast_type = "zetl-ext"               # enum: "zetl-ext" | "pandoc-ext" | "mdast-ext"; default "zetl-ext"
ast_version = ">=1.0 <2"            # semver range; schema version for the declared ast_type

[select]
include = ["**/*.md"]               # array of globs; default ["**/*.md"]
exclude = []                        # array of globs; default []
frontmatter_where = "..."           # predicate expression (REQ-3204); optional
content_probe = []                  # array of regex or substring probes; optional
require_probe_match = "any"         # enum: "any" | "all"; default "any"
```

A hook with no manifest MUST still work — zetl treats missing manifest as all defaults and `select.include = ["**/*.md"]`. Hooks without a manifest SHALL emit a warning recommending one.

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

Selection results SHALL be recorded for the `zetl hook coverage` report (REQ-3208) regardless of whether the hook was subsequently invoked.

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

1. `<vault>/.zetl/hooks/<stage>.d/*` (vault hooks)
2. `<theme-dir>/hooks/<stage>.d/*` (theme-bundled hooks)

Executables within each directory SHALL be sorted by filename (lexicographic), and the combined list SHALL be the pipeline order. A vault hook with the same filename as a theme hook SHALL replace the theme hook (not merge).

An empty executable file (`0 bytes` OR shebang-only with no body) SHALL disable the corresponding theme hook (by filename collision) without invoking anything.

Single-file hooks at `.zetl/hooks/<stage>` (not in a `.d/` directory) SHALL continue to work — treated as a one-entry `.d/` directory.

Trace:
- TEST-3206
- CON-3206

### REQ-3207: Failure Scoping and Pipeline Continuation

For a given page and stage, when hook `H_k` in the ordered pipeline fails (non-zero exit, timeout, memory overrun, malformed output), the system SHALL:

- Discard `H_k`'s output.
- Record a failure diagnostic with `plugin_id=filename`, `stage`, `page_slug`, `reason`, `duration_ms`.
- Pass `H_k`'s **input** to `H_{k+1}` (i.e., the output of `H_{k-1}`, or the stage input if `k == 0`).
- Continue the pipeline. One hook's failure does not cascade.

Under `zetl build --hook-fail-on error`, the build SHALL exit non-zero after rendering is complete if any hook failed, with an actionable summary to stderr. Default behaviour SHALL be `--hook-fail-on never`.

Trace:
- TEST-3207
- CON-3207
- OBS-3205

### REQ-3208: `zetl hook coverage` Subcommand

The system SHALL provide `zetl hook coverage [--vault PATH] [--json] [--stage STAGE]` that, for the most-recent build (or a fresh dry-run if none exists), reports per hook:

- Stage, manifest path, matched-page count, invoked-page count (may differ if selector passed but hook failed early), latency P50/P95, failure count, last failure reason.

Output defaults to table; `--json` emits structured output. Data persists between runs at `<out-dir>/.zetl/hook-coverage.json` for build mode and in-memory for serve mode.

Trace:
- TEST-3208
- CON-3208

### REQ-3209: `zetl hook dry-run` Subcommand

The system SHALL provide `zetl hook dry-run <stage>/<name> [--vault PATH] [--limit N]` that evaluates the hook's selector against the vault and prints the matched page list (up to `--limit`, default 50). The hook itself SHALL NOT be invoked. Exit code 0 if any pages matched; 1 if zero matched (to aid CI "is this selector reachable" checks).

Trace:
- TEST-3209

### REQ-3210: Helper Libraries

The system SHALL publish two first-party helper libraries alongside each zetl release:

- **`zetl-ast-py`** — Python 3.9+ package on PyPI. Provides `run(transform_fn)` entry point, typed node classes (`Document`, `Paragraph`, `BlockQuote`, `Wikilink`, …), `walk(ast, type=Foo)` iterator, and a `context` object exposing page metadata.
- **`zetl-ast-js`** — npm package (Node 18+ and Deno). Equivalent API.

Both libraries SHALL pin to a specific AST schema major and SHALL refuse to run against a mismatched zetl version (fail fast with a clear error). The `run()` entry point SHALL handle both one-shot (stdin→stdout→exit) and persistent (line-delimited JSON loop) modes, transparently, so hook authors write the same code for both.

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
- Object values are passed to the extension as page-level configuration (opaque to zetl; extension-specific semantics).

The baseline check (`frontmatter.extensions.<name> != false`) SHALL be part of the extension's default selector; theme authors replacing an extension MUST preserve this opt-out to remain behaviour-compatible.

Trace:
- TEST-3211
- CON-3211

### REQ-3212: Canonical Extensions — First Party

The default theme SHALL ship three canonical `transform`-stage extensions:

- **`callouts`** — recognises `> [!TYPE] Title` blockquotes and rewrites them to `<div class="zetl-callout zetl-callout--<type>">` structures. Selector: path `**/*.md`, content probe `^>\s*\[!`, frontmatter opt-out honoured.
- **`tasks`** — recognises `tasks` fenced code blocks, interprets the query syntax (subset: `not done`, `due before <date>`, `path includes "..."`), walks the vault index to collect matching task lines, emits an interactive (checkbox) HTML list. Selector: content probe `^```tasks\s*$`, frontmatter opt-out honoured.
- **`admonition`** — recognises `ad-*` fenced code blocks (Obsidian's legacy syntax) and rewrites them to callout structures equivalent to the `callouts` extension. Selector: content probe `^```ad-`, frontmatter opt-out honoured. Runs *after* `callouts` so both syntaxes coexist.

Each extension SHALL have a published selector, a golden-HTML fixture, and a matrix entry at tier `supported`.

Trace:
- TEST-3212a, TEST-3212b, TEST-3212c
- CON-3212

### REQ-3213: Canonical Extension Matrix

The system SHALL ship `tools/zetl-extension-matrix.toml` recording, for each first-party extension: name, tier (`supported`, `partial`, `experimental`), pinned AST schema version, selector, golden-fixture path, and notes. CI SHALL gate changes that downgrade tier or remove a fixture without an accompanying rationale.

Trace:
- TEST-3213

### REQ-3214: Template Variable Publishing

Every hook response (all three stages) MAY include an optional `template_vars` field containing a JSON object of arbitrary shape. The system SHALL accumulate these across all hooks run on a page into the Minijinja template context at the path `page.ext.<extension_id>`.

**Namespace rules:**

- **Reserved root.** `page.ext` is reserved for extensions. Zetl itself SHALL NOT write into this root in any present or future release without a contract major bump (matches SPEC-028's theme contract convention). Themes and extensions can rely on it as their exclusive surface.
- **`<extension_id>` default.** Defaults to the hook's filename without extension (e.g., `tasks.py` → `tasks`) AND without any leading numeric-plus-dash ordering prefix (so `20-tasks.py` → `tasks`, not `20-tasks`). This keeps the template-readable name stable across re-ordering.
- **Manifest override.** A hook's manifest MAY declare `extension_id = "..."` to override the default. Useful when multiple hooks cooperate under one conceptual name, or when the filename would produce an undesirable id.
- **Collision-on-filename resolution.** When a vault hook replaces a theme hook via the REQ-3206 filename-collision rule, the replacement takes over the same `page.ext.<extension_id>`. The theme's templates reading `page.ext.tasks.completed` continue to work — the vault author is obligated to emit the same-shaped data if they want existing theme templates to render correctly. This is the design contract, not a constraint.
- **No cross-namespace writes.** Extensions cannot write under another extension's `page.ext.<id>`. Zetl enforces this at the protocol layer by keying the merge on the invoked hook's id, not on any hook-supplied key.
- **Cross-extension coordination via shared `extension_id`.** Two cooperating hooks can share an `extension_id` (declared in both manifests) to coalesce into one namespace; in that case, pipeline-order wins (later hook's emission replaces earlier, with a warning logged).

**Semantic rules:**

- **Multi-stage emissions by the same hook:** If the same hook runs at multiple stages and emits vars at each, the later stage's vars replace the earlier stage's (with a warning logged). Within a single stage's response, the emitted object is final.
- **Shape:** Any valid JSON value (object, array, string, number, bool, null). Zetl validates size ≤ 1 MiB per hook per page; oversize payloads are dropped with a warning, but the AST/HTML payload is still used.
- **Autoescape:** String values emitted into templates go through the standard Minijinja autoescape path. No new XSS surface.
- **Opt-in at author time, opt-in at theme time:** Extensions choose whether to emit; themes choose whether to read. Absent vars resolve to `undefined` in Minijinja (no error).
- **Build/serve parity:** Both modes expose the same `page.ext.<id>` namespace with identical semantics.

Example: a hook at `.zetl/hooks/transform.d/20-tasks.py` returning `{"template_vars": {"total": 12, "completed": 7}}` makes `{{ page.ext.tasks.total }}` and `{{ page.ext.tasks.completed }}` available in all templates that render that page. A sibling manifest declaring `extension_id = "my_tasks"` would surface the same data at `page.ext.my_tasks.*` instead.

Vault-level aggregation (`vault.ext.<extension_id>`) is explicitly out of scope for v1 — see §13 Open Questions.

Trace:
- TEST-3214
- CON-3214
- OBS-3209

### REQ-3215: Dual-Version Exposure (Binary and AST Schema)

The system SHALL expose two distinct version strings to every hook invocation: `ZETL_VERSION` (the binary semver, changes every release) and `ZETL_AST_VERSION` (the JSON-schema semver, changes only when the AST shape changes). Both SHALL be available as environment variables at hook start AND as top-level fields in the persistent-mode handshake message (`{"zetl_version": "...", "ast_version": "1.2"}`).

Hooks declare their required AST-schema range in the manifest:

```toml
ast_version = ">=1.0 <2"   # npm-style range; default ">=1.0 <2" for v1
```

At zetl startup, the system SHALL compute the effective `ZETL_AST_VERSION` and compare against each hook's declared range. Version-drift policy (REQ-3215.1):

- **Incompatible range** (hook demands >=2.0 on a 1.x binary, or vice versa): hook is disabled with a typed error; log line `[zetl] hook incompatible: <id> requires ast_version=<range>, have <version>`. Build continues.
- **Compatible but minor mismatch** (hook wrote against 1.0, binary offers 1.2): hook runs; warning logged once per hook per session.
- **Exact match**: silent success.

Rationale: follows Pandoc's `PANDOC_API_VERSION` / `PANDOC_VERSION` split ([filters.html](https://pandoc.org/filters.html)). Decouples AST-breaking changes from zetl's normal release cadence. Hooks authored against AST v1.0 survive zetl binary releases 1.x.y → 1.y.z so long as the AST schema remains backwards-compatible (NFR-3206).

Trace:
- TEST-3215
- CON-3201
- ADR-3201

### REQ-3216: Capability Probe

The system SHALL invoke every hook in **probe mode** once at pipeline initialisation, before any page is processed. Probe mode is signalled by argv (`<hook-executable> --probe`) or by the first-line protocol message `{"type": "probe"}` in persistent mode. The hook SHALL respond with a single JSON document declaring:

```json
{
  "type": "probe_result",
  "zetl_ast": "1.0",
  "hook": "callouts",
  "version": "1.0.3",
  "stages": ["transform"],
  "applies_when": {"modes": ["build", "serve"], "themes": null, "formats": ["html"]},
  "ready": true
}
```

Semantics:

- `stages` — the stages this hook handles. A one-shot hook appearing in `transform.d/` but reporting `stages: ["post-render"]` is a manifest/probe mismatch and the hook SHALL be disabled with a diagnostic.
- `applies_when` — optional; allows a hook to exclude itself from e.g. `zetl serve` (build-only hooks) or specific themes. When absent, defaults to applies-always.
- `ready: false` — hook explicitly declines to run; no error, diagnostic logged with optional `reason` field.

Probe failures (non-zero exit, malformed response, timeout > 5s) SHALL disable the hook for the current session with an actionable diagnostic. `zetl hooks check` (new subcommand) SHALL run every hook's probe and report status without running the build.

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

**Resolution algorithm:** zetl performs a topological sort over the hook set for each stage, respecting `before`/`after` constraints, with filename order as the tiebreaker for unordered hooks. Cycles (A before B, B before A) SHALL be reported as a build error with the cycle path in the diagnostic.

**Optional:** A hook marked `optional = true` whose executable is missing, non-executable, or whose probe fails SHALL emit a warning and be skipped, but SHALL NOT fail the build or abort pipeline construction. Default is `optional = false`.

Rationale: mdBook's ordering and optional flags ([mdBook config](https://rust-lang.github.io/mdBook/format/configuration/preprocessors.html)). Filename ordering works for small pipelines; named constraints scale to ten-hook vaults without prefix-renumbering every time a hook is added.

Trace:
- TEST-3217
- CON-3217

### REQ-3218: Declarative Node-Type Dispatch in Helper Libraries

The system's helper libraries (`zetl-ast-py`, `zetl-ast-js`) SHALL provide a `dispatch` entry point that takes a table (dict/object) keyed by AST node type and invokes the corresponding function for each matching node as zetl walks the tree. A reserved `Inline` key matches any inline node not otherwise covered; `Block` matches any block node; `*` matches any node.

Python API:
```python
from zetl_ast import dispatch

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

- **Read-only from the hook's perspective at invocation time** — zetl injects the current store snapshot into the context passed to the hook; hook-requested writes are returned in the response.
- **Hook-requested writes merged by zetl** — a hook returning `{"build_data": {"unresolved_links": [...]}}` writes into the next hook's snapshot. Writes are namespaced by writing-hook's `extension_id` to avoid collisions (`build_data[extension_id][key] = value`).
- **Build-scoped**: cleared between builds (`zetl build`) or between page renders (`zetl serve`).
- **Size-capped**: 16 MiB total per build; oversize writes dropped with a warning.

Access in hook:
```python
def transform(ast, ctx):
    # ctx.build_data is a read-only view of the current store
    prior_citations = ctx.build_data.get("citations", {}).get("keys", [])
    ctx.emit_build_data(keys=prior_citations + new_keys)
    return ast
```

Rationale: unified's `processor.data()` ([unified](https://github.com/unifiedjs/unified)) and markdown-it's `env` parameter ([markdown-it architecture](https://github.com/markdown-it/markdown-it/blob/master/docs/architecture.md)) both exist because cross-hook coordination through frontmatter is fragile. Zetl's `page.ext` namespace is one-way (hook → template); `build_data` is two-way (hook → hook).

Trace:
- TEST-3219
- CON-3219

### REQ-3220: Expose Build Context to Hooks

The system SHALL expose the following context to every hook invocation, available both as environment variables and as fields in the invocation JSON payload:

- `ZETL_MODE` — `"build"` or `"serve"`.
- `ZETL_THEME` — active theme name.
- `ZETL_VAULT_ROOT` — absolute path to the vault.
- `ZETL_OUT_DIR` — build output directory (null under serve).
- `ZETL_VERBOSE` — `"true"` / `"false"`.
- `ZETL_AT` — historical-build timestamp if `--at` is used, else absent.
- `ZETL_HOOK_PATH` — absolute path of the hook's own executable, for resolving sibling resources (equivalent to Pandoc's `PANDOC_SCRIPT_FILE`).
- `ZETL_EXTENSION_ID` — the extension_id zetl resolved for this hook (manifest override or filename default).

Rationale: Pandoc's `PANDOC_READER_OPTIONS`, `PANDOC_WRITER_OPTIONS`, `PANDOC_SCRIPT_FILE` ([filters.html](https://pandoc.org/filters.html), [lua-filters.html](https://pandoc.org/lua-filters.html)). Without a canonical exposure mechanism, hooks reinvent detection via filesystem introspection and side-channels.

Trace:
- TEST-3220
- CON-3220

### REQ-3221: Typed AST-Protocol (ast_type) for Transform-Stage Hooks

The system SHALL support multiple AST formats at the `transform` stage via a declared `ast_type` on the hook manifest. The `ast_type` field identifies which ecosystem's AST shape and invocation conventions the hook expects; zetl translates between its internal representation and the declared type at the protocol boundary.

**Supported types (v1):**

- **`zetl-ext`** (default) — zetl's native AST JSON (REQ-3202 / CON-3202). CommonMark subset + wikilink, embed, SPL-block, and frontmatter extensions. Required for v1 conformance.
- **`pandoc-ext`** — reserved for v1.1; accepted in manifests in v1 with a "not yet implemented" parse error. When implemented, zetl SHALL serialise in `pandoc-types` JSON shape, set the `PANDOC_VERSION`, `PANDOC_API_VERSION`, `PANDOC_READER_OPTIONS`, `PANDOC_WRITER_OPTIONS`, and `PANDOC_SCRIPT_FILE` env vars, and pass the output format as argv[1] per Pandoc filter conventions ([filters.html](https://pandoc.org/filters.html)).
- **`mdast-ext`** — reserved for v2.x; accepted-with-error in v1 and v1.1. Would require an in-process JS harness and a Node subprocess runner.

**Translation contract (zetl-ext ↔ pandoc-ext, to ship in v1.1):**

Zetl owns the bidirectional mapping. Every zetl-ext concept that does not have a native pandoc-types equivalent is represented via Pandoc's escape hatches with a stable marker convention:

- **Wikilinks** (`Wikilink`) ↔ `Span` with class `zetl-wikilink` and attrs for `target`, `alias`, `heading`, `block_id`.
- **Embeds** (`Embed`) ↔ `Span` with class `zetl-embed` and attrs for `target`, `heading`, `block_id`.
- **SPL blocks** — `CodeBlock` with language `spl` in both directions.
- **Frontmatter** — zetl's `FrontMatter` node ↔ Pandoc's `Meta` map (on the root `Pandoc` document).
- **Position info** — `start_line`/`start_col`/`end_line`/`end_col` ↔ Pandoc has no native position concept; preserved in an `sourcepos` attribute on `Span` / `Div` wrappers and restored on the reverse translation.

A pandoc-ext hook that unknowingly strips `class="zetl-wikilink"` or its attrs from a `Span` SHALL cause wikilinks in its output to be corrupted; zetl logs this as a warning (detected by comparing the round-trip input and output for loss of markers). This is the fundamental trade-off: pandoc-ext filters that respect foreign-marker convention work losslessly; those that don't will erode zetl-specific structure.

**Round-trip round-trip property:** For any zetl-ext AST *A* and a zetl-authored identity pandoc-ext filter *I*, `zetl_to_pandoc(I(pandoc_to_zetl(A)))` SHALL equal *A* byte-for-byte. This is the strictest test of translator fidelity and is gated by TEST-3221.

**Protocol-convention emulation:** When zetl invokes a `pandoc-ext` hook, the hook sees Pandoc's invocation contract (env vars, argv), not zetl's. When it invokes a `zetl-ext` hook, it sees zetl's contract (REQ-3220's `ZETL_*` env vars). The `ast_type` field drives both payload serialisation and invocation-convention selection.

**Version compatibility per type:** `ast_version` in the manifest is a semver range interpreted against the declared `ast_type`'s version scheme — `>=1.22 <2` for pandoc-ext means pandoc-types v1.22+ below v2, not zetl-AST v1.22+. Zetl maintains a compat matrix (`tools/zetl-ast-compat.toml`) mapping each ast_type and version to the ones the current binary supports.

**Mixed pipelines:** Two hooks in the same stage may declare different `ast_type`s. Zetl translates the prior hook's output back to its internal representation (if different from zetl-ext), then serialises anew for the next hook's declared type. No hook sees another hook's AST in a format it didn't request.

Trace:
- TEST-3221
- CON-3221
- ADR-3206

---

## 4. Non-Functional Requirements

### NFR-3201: Selection Evaluation Latency

Evaluating the full selector chain (path glob + frontmatter predicate + content probe) for one hook on one page SHALL complete in ≤ 2 ms at P95 for pages up to 100 KB on a 2020-or-newer laptop, AVERAGED across path/frontmatter/content probe passes. Selectors are the hot path; they must be cheap.

Trace:
- TEST-3201-perf
- OBS-3202

### NFR-3202: Render Overhead — Canonical Extensions

Under `zetl build` with the three canonical extensions (`callouts`, `tasks`, `admonition`) all enabled on the 2,000-page demo vault, total render time SHALL increase by ≤ 15% relative to a build with `--no-hooks` (new flag). Persistent-mode extensions amortise spawn cost; this is the target after that amortisation.

Trace:
- TEST-3202-perf
- OBS-3206

### NFR-3203: Deterministic Output

Given a fixed vault, fixed extension set, fixed zetl version, and fixed AST schema version, `zetl build` SHALL produce byte-identical HTML output across runs and platforms (macOS aarch64, Linux x86_64, Linux aarch64). Extensions are themselves required to be deterministic; documentation states the contract.

Trace:
- TEST-3203-determinism

### NFR-3204: Build Failure Isolation

Under default behaviour (`--hook-fail-on never`), `zetl build` exit code SHALL be 0 even when every enabled hook on every page fails. The pipeline degrades gracefully; diagnostics are emitted; the output HTML contains the unmodified content.

Trace:
- TEST-3207
- OBS-3205

### NFR-3205: Startup Cost When No Hooks Present

On a vault with no `.zetl/hooks/` directory and a theme with no `hooks/` directory, the additional process startup cost relative to pre-SPEC-032 zetl SHALL be ≤ 5 ms at P95. The feature has zero cost for users who don't use it.

Trace:
- TEST-3205-startup

### NFR-3206: AST Schema Stability

The zetl-AST schema SHALL follow semver. Within a major version: additive changes only (new node types, new optional fields). Breaking changes (removed fields, changed node shapes) require a major bump AND a one-release deprecation window where both majors are emitted behind a flag.

Trace:
- Reviewed at every schema change; tracked in CHANGELOG.

### NFR-3207: Protocol Overhead — Persistent Mode

For persistent-mode hooks, the per-page overhead (serialise AST → write to stdin → read response from stdout → deserialise) SHALL be ≤ 10 ms P95 for pages with ≤ 500 AST nodes (covers the typical case of a 10 KB Markdown page).

Trace:
- TEST-3207-perf
- OBS-3207

### NFR-3208: Memory Containment

Each hook process SHALL have a configurable memory ceiling (default 64 MiB, set via manifest). Exceeding the ceiling SHALL terminate the process; for persistent-mode hooks, zetl SHALL respawn on the next page requiring that hook.

Trace:
- TEST-3208
- OBS-3208

---

## 5. Contracts

### CON-3201: Persistent-Mode Wire Protocol

For `mode = "persistent"` hooks, zetl and the hook communicate over line-delimited JSON on stdin/stdout. Stderr is free-form (used for diagnostic logging only; never consumed by zetl as data).

**Handshake (one message):** On process start, the hook writes a single line:
```json
{"zetl_ast": 1, "hook": "callouts", "version": "1.0.3", "ready": true}
```
Zetl reads this line; if `zetl_ast` major differs from the current schema, zetl disables the hook and logs `ast_version_mismatch`.

**Per-page request (from zetl, one line):**
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

**Shutdown:** Zetl closes stdin. Hook SHOULD exit cleanly within 1 s. Hard-kill on timeout.

**Return-type contract (`transform` stage):** The hook's `payload` response field MUST be an AST document whose root is a `Document` node (same shape as the input). For in-tree transformations, every rewritten node MUST have a `type` field; a node of type X may be replaced by a node of type X, a node of a compatible type per the AST schema (e.g., `Paragraph` → `BlockQuote` is valid because both are block-level), or an array of compatible-type nodes (replacing one `Paragraph` with three). Replacing a block-level node with an inline-level node (or vice versa) SHALL be rejected with a typed validation error and the hook's output discarded per REQ-3207. (Follows Pandoc's strict return-type discipline: [lua-filters.html](https://pandoc.org/lua-filters.html) "Pandoc will throw an error if this condition is violated.")

**Return-type contract (`pre-parse` stage):** Payload must be valid UTF-8 text. Size limit: 16 MiB.

**Return-type contract (`post-render` stage):** Payload must be a UTF-8 string that parses as well-formed HTML (validated via a streaming parser). Malformed HTML is rejected per REQ-3207.

**Error surfacing convention:**
- **stdout**: structured JSON per this protocol. Non-JSON output on stdout is a hook error.
- **stderr**: free-form human-readable logs; zetl forwards under `--verbose` and includes in failure diagnostics unconditionally.
- **Non-zero exit**: aborts the hook's pipeline participation per REQ-3207; build overall continues unless `--hook-fail-on error` OR the hook manifest has `fail_hard = true`.

**Version-drift policy:** Enforced per REQ-3215. The handshake includes `ast_version`; zetl compares against the binary's supported range. Incompatible → hook disabled with typed error. Compatible-but-minor-mismatch → warning, hook runs. Exact match → silent.

Implements: REQ-3201, REQ-3210, REQ-3215.
Verified by: TEST-3201, TEST-3210, TEST-3215.

### CON-3202: zetl-AST JSON Schema (Summary)

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

Full JSON Schema (Draft 2020-12) lives at `tools/zetl-ast-schema-v1.json`. The schema is normative; anywhere this document disagrees, the schema file wins.

**Default traversal order for `transform`-stage helpers:** zetl helper libraries default to **typewise** traversal (visits all `Inline` nodes, then all `Block` nodes, then `Document`). This matches Pandoc Lua filters' default and is empirically the order most extension authors expect ([lua-filters.html](https://pandoc.org/lua-filters.html) "Traversal order"). Helpers MAY offer a `topdown` mode for extensions that need root-to-leaves iteration; the selection is a flag on the `dispatch`/`walk` call, never on the zetl-side pipeline. Hook authors can override per call:

```python
dispatch(ast, ctx, handlers, traverse="topdown")   # root-first; Pandoc added this in 2.17
```

**AST size and depth caps:** Documents exceeding 10 MiB serialised JSON OR 256 nesting levels SHALL be rejected at the protocol boundary with a typed error. Matches CommonMark's nesting limit and prevents protocol-level DoS.

Implements: REQ-3202.
Verified by: TEST-3202.

### CON-3203: Manifest File Format

See REQ-3203 for the full TOML grammar. Every field is optional except `stage` (required when the manifest is used for a stage-specific behaviour; omitted when zetl infers stage from directory name). Unknown fields SHALL be rejected with a parse error to catch typos; zetl may later add fields, but additions follow the AST schema additive-only rule (NFR-3206).

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
.zetl/hooks/transform.d/
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

Python (`zetl-ast-py`):
```python
from zetl_ast import run, walk, Node, Document, Paragraph, BlockQuote, Text, Wikilink
from zetl_ast.context import Context

def transform(ast: Document, ctx: Context) -> Document:
    for node in walk(ast, type=BlockQuote):
        ...
    return ast

run(transform)   # handles one-shot and persistent modes transparently
```

JavaScript (`zetl-ast-js`):
```javascript
import { run, walk, Document, BlockQuote, Text } from 'zetl-ast';
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

**Invariant:** For every supported `(ast_type, ast_version)` pair, zetl ships a bidirectional translator `zetl_ext ↔ foreign_ext` with the following guarantees:

1. **Round-trip identity for zetl-native concepts.** For any zetl-ext AST *A*, `foreign_to_zetl(zetl_to_foreign(A)) == A` byte-for-byte. Tested via property tests on a CommonMark-derived generator.
2. **Forward preservation of foreign concepts.** For any foreign AST *F*, converting to zetl-ext and back produces a representation that renders identically to the original HTML (semantic round-trip, not byte-identical — foreign AST may have attributes zetl doesn't track).
3. **Marker conventions for non-native concepts.** Each foreign AST declares a "marker namespace" (e.g., pandoc-ext uses `class="zetl-*"` and attrs on `Span`/`Div`) for zetl concepts it can't represent natively. A foreign-ext hook that strips markers corrupts round-trip but is detectable via REQ-3221's loss-detection logging.
4. **Protocol-convention table.** Each ast_type's invocation conventions (env vars, argv, handshake shape, error response format) are spelled out in `docs/ast-types/<type>.md` and form part of the zetl contract.

**Failure modes and diagnostics:**

| Failure                                                   | Detection                              | Zetl response                              |
| --------------------------------------------------------- | -------------------------------------- | ------------------------------------------ |
| Hook strips marker attrs on zetl-wikilink spans          | Post-hook scan: count wikilink nodes in pre vs post | Warning with list of lost nodes; hook continues |
| Hook returns foreign AST that fails to parse              | Foreign-ast deserialise                | REQ-3207 failure; unmodified input passed  |
| Hook returns foreign AST that fails to translate back     | Translator validation                  | REQ-3207 failure                           |
| ast_type declared in manifest but not supported by binary | Pipeline init                          | Hook disabled with actionable error        |
| ast_version range incompatible with binary's translator   | Pipeline init                          | Hook disabled with actionable error        |

**Translator implementation module:** `src/hooks/translators/<ast_type>.rs`. Each translator is in the pure core (pure function from AST to AST), not the effectful shell. Test as a pure Rust module.

Implements: REQ-3221.
Verified by: TEST-3221.

### CON-3214: Template Variable Semantics

**Lifecycle:** For each page, zetl initialises an empty `page.ext = {}` context object before the pipeline runs. Each hook's response `template_vars` (if present) is deep-merged under `page.ext[hook_id]`. At template render time, Minijinja receives the final `page.ext` dict alongside the existing page variables.

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

---

## 6. Architecture Decisions

### ADR-3201: Define a zetl-Specific AST JSON Format (Not pulldown-cmark Events)

**Context:** The `transform` stage needs a stable data-interchange format. Options: (a) re-use pulldown-cmark's event stream, (b) adopt the unified/remark `mdast` format, (c) define a zetl-specific schema.

**Decision:** Define a zetl-specific schema derived from CommonMark AST conventions, with explicit extensions for wikilinks, embeds, and frontmatter.

**Rationale:**
- **Parser independence.** pulldown-cmark's event stream is an iteration protocol, not a stable document format; it serialises awkwardly. If we ever swap parsers, tying hooks to pulldown-cmark's shape is a migration nightmare.
- **Vault-domain extensions.** `mdast` doesn't cover wikilinks or zetl's `![[...]]` embed syntax natively. We'd need extensions anyway; once you're extending, defining the whole schema is cleaner than patching someone else's.
- **Versioning control.** A zetl-owned schema means we decide when to bump, what additive changes mean, and how to deprecate. `mdast` evolves at remark's cadence, which doesn't match ours.
- **Serialisation shape.** Nested tree JSON is ergonomic for both Python and JS AST-walking code. Event streams require users to track open/close state — more error-prone.

**Trade-offs accepted:**
- Schema maintenance burden is ours. Offset by the fact that the schema is small (~20 node types) and CommonMark-stable.
- Conversion cost: pulldown-cmark events → zetl-AST JSON is a serialise step. Measured overhead is ~0.5 ms for a typical page; negligible.

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
- **Failure isolation.** A buggy hook crashes its process; `zetl` keeps running. In-process embedding makes OOM and panics the host's problem.
- **Supply chain.** No embedded interpreter = no QuickJS/Steel CVE exposure in `zetl`'s dependency tree.
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
- **Evaluable without invoking.** Zetl parses the manifest once at startup; selector evaluation per page doesn't require the hook to run. Critical for NFR-3201.
- **Co-located with the executable.** The hook and its selector travel together — themes, version control, distribution archives all move one `(executable, manifest)` pair as a unit.
- **Discoverable.** Users browsing `.zetl/hooks/transform.d/` see `callouts.py` and `callouts.toml`; the relationship is obvious.
- **Tool-friendly.** `zetl hook dry-run` parses only the manifest; no need to spawn the hook process just to learn its selector.

**Trade-offs accepted:**
- Two files per hook (not one). Mitigated: manifests are optional; hooks without one get sensible defaults with a warning.
- Selector is static — can't depend on runtime state. This is a feature (determinism) more than a limitation.

**Alternatives considered:**
- Config.toml namespace — rejected: centralising every hook's config in one file breaks the per-hook encapsulation that makes themes composable.
- Hook-declared selector (describe-yourself call) — rejected: forces a process spawn to learn the selector, defeating the perf argument.

Status: Proposed.

### ADR-3204: Canonical Extensions Ship as Hooks, Not Native Rust

**Context:** Callouts, Tasks, Admonition could ship as (a) native Rust modules behind a `--features markdown-extensions` flag, (b) canonical hooks in the default theme's `hooks/transform.d/`, (c) both.

**Decision:** (b) — ship as hooks in the default theme.

**Rationale:**
- **Consistency of the extension surface.** If zetl's own extensions ship as Rust modules, users learning the hook contract face a mismatch: "why don't callouts look like the hooks I'm writing?" One mechanism, one mental model.
- **Authoring template.** The canonical extensions become working reference implementations. User adds their own extension by copying `callouts.py` and modifying it.
- **Theme-override capability preserved.** Theme authors can restyle Callouts by editing a file; native modules require template-level overrides, which is a different mental model.
- **Cheaper to ship and iterate.** Adding a canonical extension is adding a file, not a feature flag and a Rust module.

**Trade-offs accepted:**
- Per-page cost is higher than a native Rust implementation (persistent-mode Python vs. Rust function call). Measured at ~5–20 ms per extension per matched page; acceptable within NFR-3202.
- Python dependency for default theme. Mitigated: documentation requires Python 3.9+ for default theme extensions; pure-JS alternatives (Node 18+) are available for any theme that prefers JS. Users running `zetl build` in minimal environments (e.g., Alpine CI) need to install one runtime.

**Alternatives considered:**
- Native Rust (a) — rejected on consistency grounds; becomes a second extension surface competing with the hook surface.
- Both (c) — rejected as over-engineered. Reconsider only if profiling shows canonical extensions dominate build time.

Status: Proposed.

### ADR-3206: Typed AST Protocol (Multiple `ast_type`s) Over Single-Format

**Context:** SPEC-032's transform-stage AST is zetl-ext — our own CommonMark-derived format with wikilink/embed/SPL extensions (ADR-3201). This gives schema control but closes the door on the vast existing ecosystem of Pandoc filters (hundreds of extensions: pandoc-crossref, pandoc-citeproc, pantable, etc.) and unified/remark plugins (thousands). The question is how to open that door without surrendering zetl-ext.

Three shapes were considered:

- **(a)** Adopt pandoc-types (or mdast) wholesale as zetl's AST. Full ecosystem compatibility; no translation layer needed. Cedes schema control, couples zetl to Pandoc's release cadence, and represents wikilinks/embeds/SPL as stringly-typed conventions over `Span`/`Raw` (ADR-3201 is essentially a rejection of this).
- **(b)** Keep zetl-ext as the only AST. Ship a separate `zetl-pandoc-adapter` binary as an optional companion tool. Users invoke it explicitly in their manifest. Works, but requires users to know about adapter tools, install them separately, and reason about the two-layer abstraction.
- **(c)** Typed protocol: each hook declares its `ast_type` in the manifest; zetl's own dispatch layer knows how to serialise to each supported type. Translation is owned by zetl, not a third-party tool. Mixed pipelines (zetl-ext + pandoc-ext + mdast-ext) compose because zetl translates at each boundary.

**Decision:** (c). The protocol is typed; `zetl-ext` is one of several supported `ast_type` values, not the only one. v1 accepts and defaults to `zetl-ext`; `pandoc-ext` and `mdast-ext` are reserved values that produce actionable errors in v1 and are implemented in v1.1 and v2.x respectively.

**Rationale:**

- **Borrow strength without cession.** zetl retains schema control (ADR-3201 stands) while offering first-class compatibility with external ecosystems through translators zetl owns.
- **Clean composition.** Mixed-`ast_type` pipelines work out of the box. A user can chain pandoc-crossref (pandoc-ext), a native zetl Callouts extension (zetl-ext), and a remark-gfm-style plugin (mdast-ext) in one pipeline; zetl translates at each boundary.
- **First-party translator quality.** Adapter binaries live in third-party repos with their own release cadences, test thoroughness, and abandonment risk. A zetl-owned translator is testable in-tree with the rest of the hook runtime.
- **Per-type versioning.** `ast_version` ranges are interpreted relative to the declared `ast_type`'s version scheme. pandoc-types v1.22 and v2.0 can be supported simultaneously in the compat matrix without coupling zetl-ext's semver.
- **Future-proofing.** New ecosystems (Djot, Pollen) are additions to the type registry, not redesigns of the protocol.
- **Precedent.** LSP capability negotiation, Tree-sitter's per-query language fields, and DAP's similarly-typed protocols all demonstrate this shape works for protocol-level extensibility.

**Trade-offs accepted:**

- **Translator implementation cost.** Each supported type is a non-trivial Rust module with fuzz-quality round-trip tests against real ecosystem hooks. pandoc-ext alone is substantial. Managed by gating implementation to v1.1 (pandoc-ext) and v2.x (mdast-ext).
- **Translation loss is intrinsic.** Foreign ASTs (Pandoc, mdast) don't have native wikilinks or SPL. Marker conventions are unavoidable. Documentation must make round-trip guarantees explicit.
- **Protocol-convention emulation.** Supporting pandoc-ext means emulating Pandoc's invocation contract (env vars, argv). Real engineering; managed by scoping the contract in CON-3221 and documenting in `docs/ast-types/pandoc-ext.md`.
- **Complexity signal to users.** Users now have one more manifest field. Default (`zetl-ext`) covers the 80% case; only ecosystem-migration users need to think about `ast_type`.

**Alternatives rejected:**

- **Single-type with adapter binary** (shape b) — rejected on user-experience grounds. "Install this other tool to use Pandoc filters" is materially worse than "set `ast_type = \"pandoc-ext\"`."
- **Adopt pandoc-types wholesale** (shape a) — rejected under ADR-3201. Still correct.
- **Defer typed protocol entirely to v2.0** — considered. Rejected because the manifest field is free to define now (it costs nothing to reserve) and retrofit later would require a manifest-schema breaking change.

Status: Proposed.
Supersedes: —. Extends ADR-3201 (which establishes zetl-ext as a native type) by placing it in a registry rather than as the sole type.

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

For a canonical demo vault, render through the pipeline; validate emitted AST against `zetl-ast-schema-v1.json` using a standard JSON Schema validator. Assert 0 validation errors.

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

Run `zetl build` three times on a fixed vault with fixed canonical extensions. Assert zero byte-differences across runs and across platforms (macOS aarch64 + Linux x86_64 in CI).

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

Benchmark on a fixture vault with no `.zetl/hooks/` directory. Compare startup time to a control build (pre-SPEC-032 code or `--no-hooks` flag). Assert delta ≤ 5 ms P95.

Verifies: NFR-3205.

### TEST-3206: Composition and Precedence

Fixture vault with both theme and vault hooks for the same stage. Assert the resulting pipeline order and override semantics match CON-3206's worked example. Add a test for the "empty file disables theme version" edge case.

Verifies: REQ-3206.

### TEST-3207: Failure Scoping

Chain `[10-a.py, 20-fail.py, 30-b.py]`. `20-fail.py` throws. Assert `30-b.py` receives `10-a.py`'s output, not the stage input. Assert the failure is recorded in the diagnostics but the pipeline completes.

`zetl build --hook-fail-on error`: assert build exits non-zero; assert output HTML is still written (failure is reported, not build-aborting for partial output).

Verifies: REQ-3207, NFR-3204.

### TEST-3207-perf: Persistent-Mode Overhead

Benchmark: persistent-mode hook that returns input unmodified. Measure per-page round-trip (serialise AST → write → read → deserialise) on pages with 500 AST nodes. Assert P95 ≤ 10 ms.

Verifies: NFR-3207.

### TEST-3208: Coverage Report

Run build with mixed hook outcomes; assert `hook-coverage.json` content matches a golden schema. Verify `zetl hook coverage --json` emits structurally-equivalent data.

Verifies: REQ-3208.

### TEST-3209: Dry-Run

`zetl hook dry-run transform/callouts --vault demo-vault` against a fixture; assert the matched page list is byte-identical to a golden. Assert exit code 1 when zero pages match.

Verifies: REQ-3209.

### TEST-3210: Helper Library Contract Tests

Python (`zetl-ast-py`): a minimal `identity` transform hook built with the library is run as a real persistent-mode subprocess against the zetl pipeline. Assert round-trip equivalence for a diverse page set.

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

### Fuzzing — Predicate and Manifest Parsers

`cargo-fuzz` targets on `hooks::predicate::parse` and `hooks::manifest::parse`. Run 24h nightly. Assert no panics, no UB.

### Fuzzing — AST Deserialisation

`cargo-fuzz` target on `hooks::ast::deserialise_from_json`. Feeds random JSON; asserts either a successful AST or a typed error — never a panic.

### Synthetic-User Simulation

Profiles 2.1 (migrant), 2.2 (custom transform author), 2.3 (theme author) walked against this draft. Findings converted to REQ/NFR amendments before status → `approved`.

---

## 9. Observability

### OBS-3201: Hook Discovery

Log line at zetl startup (once): `[zetl] hooks: discovered <N> in <stage>.d/ (theme=<M>, vault=<K>)` for each stage.
Metric: `zetl_hooks_discovered_total{stage, source}`.

### OBS-3202: Selector Evaluation

Metric: `zetl_hook_selector_latency_seconds{hook_id, layer}` histogram, where `layer` ∈ {path, frontmatter, probe, total}.
Under `--verbose`: `[zetl] hook selector: <id> matched=<bool> evaluated_layers=<N> duration_ms=<M>`.

### OBS-3203: Selection Hit Rate

Metric: `zetl_hook_selector_match_total{hook_id, outcome}` counter, `outcome` ∈ {matched, excluded_path, failed_frontmatter, failed_probe}.

This feeds the coverage report and is the principal "is this hook earning its place?" signal.

### OBS-3204: Hook Invocation Latency

Metric: `zetl_hook_invoke_duration_seconds{hook_id, mode}` histogram, `mode` ∈ {one-shot, persistent}.
Under `--verbose`: `[zetl] hook invoke: <id> page=<slug> duration_ms=<M>`.

### OBS-3205: Failure Rate

Metric: `zetl_hook_failure_total{hook_id, reason}` counter, `reason` ∈ {non_zero_exit, timeout, memory_exceeded, malformed_output, crash}.
Log line per failure (severity `warn` under default, `error` under `--hook-fail-on error`).

### OBS-3206: End-to-End Render Overhead

`zetl build` summary line: `[zetl] hooks: total_invocations=<N> total_duration_ms=<M> failures=<K>`.
Delta against a `--no-hooks` baseline available in the coverage report.

### OBS-3207: Persistent-Mode Protocol Overhead

Metric: `zetl_hook_protocol_duration_seconds{hook_id, phase}` histogram, `phase` ∈ {serialise, transport, deserialise}.

### OBS-3208: Memory

Metric: `zetl_hook_memory_high_water_mib{hook_id}` gauge, sampled at each persistent-mode invocation boundary.

### OBS-3209: Template Variable Emission

Metric: `zetl_hook_template_vars_total{hook_id, outcome}` counter, `outcome` ∈ {emitted, dropped_oversize, overwritten_by_later_stage}.
Log line (debug): `[zetl] hook template_vars: <hook_id> page=<slug> bytes=<N> keys=<K>`.
Coverage report (REQ-3208) includes a per-hook `template_var_bytes_avg` column for visibility into extension payload sizes.

Trace: REQ-3214.

---

## 10. Security Considerations

- **Untrusted code execution.** Hooks are arbitrary user/theme-author code. Zetl treats them as untrusted.
  - **Isolation:** out-of-process; no shared-memory primitive; subprocess stdin/stdout only.
  - **No privilege escalation:** hook inherits the user's UID/GID; zetl does not gate per-hook permissions in v1 (future work).
  - **Filesystem access:** a hook has whatever filesystem access the user running `zetl` has. Zetl does not jail hooks. Documentation notes this explicitly.
  - **Network:** same — hooks can open sockets. Users who want stricter isolation should run zetl under their own sandboxing (bwrap, Firejail, Docker).
- **Selector as DoS vector.** A pathological regex in `content_probe` could be catastrophic (ReDoS). Mitigation: use a regex engine with linear-time guarantees (`regex` crate); reject patterns that compile to non-linear. Fuzz test the probe path.
- **Manifest parse.** Malformed TOML is user error. Parser hardening via TOML crate + `cargo-fuzz`.
- **AST JSON deserialisation.** Hooks send JSON back; zetl deserialises. Untrusted-JSON attacks (deeply nested objects, huge arrays) mitigated by (a) 10 MiB max message size, (b) recursion depth limit of 256 (matches CommonMark's nesting cap).
- **Threat model (v1) — out of scope:** targeted exfiltration by a theme author's malicious hook (same risk posture as Obsidian plugins or VS Code extensions; user consent to the theme is consent to its hooks).
- **AI Trust Boundary classification:** Tier 3 (standard feature code; untrusted-input processing at the JSON protocol boundary). Implementation requires same-model fresh-context review OR cross-model review. Not Tier 1 — no cryptography or authentication in scope.

---

## 11. Documentation Plan

- **README.md** — new "Extension hooks" section: three stages, selector basics, canonical extensions list, how to disable.
- **CHANGELOG.md** — entry for the new stages, the AST schema, and the supersession of SPEC-031.
- **`docs/hook-authoring.md`** — primary tutorial. Walks through writing a Python hook with `zetl-ast-py`, writing the manifest, running `zetl hook dry-run`, iterating.
- **`docs/zetl-ast-schema.md`** — reference for the AST JSON schema with per-node-type examples.
- **`docs/hook-migration.md`** — for any (hypothetical) SPEC-031 users: there's nothing to migrate; SPEC-031 never shipped. This doc exists only to anchor the link from SPEC-031's `superseded-by` field.
- **`docs/canonical-extensions.md`** — per-extension reference for Callouts, Tasks, Admonition, covering syntax, selector, configuration, per-file opt-out, HTML output shape.
- **Theme authoring reference update (SPEC-028's pattern)** — add the `hooks/<stage>.d/` convention and the empty-file-disables rule.

---

## 12. Rollout Plan

**Phase A — Plumbing:** AST schema, JSON Schema published, selector evaluator, manifest parser, one-shot + persistent protocol, coverage + dry-run subcommands. No canonical extensions yet. Ships behind `--features hooks-v2` flag for the first release to give early adopters a preview while we converge the schema.

**Phase B — Helper libraries:** `zetl-ast-py` and `zetl-ast-js` published to PyPI and npm. Version pinned to AST schema v1.0.

**Phase C — Canonical extensions:** `callouts` first (smallest and highest ROI). Golden-HTML tests. `tasks` and `admonition` follow.

**Phase D — Feature flag retirement:** once canonical extensions are green in CI for two consecutive releases and no major schema changes are in flight, `--features hooks-v2` becomes default; opt-out via `--no-hooks-v2`.

**Phase E — SPEC-031 supersession:** SPEC-031 marked `superseded` with `superseded-by: SPEC-032`. Top-50 scan findings retained. No code changes required (SPEC-031 never shipped).

**Rollback:** if the AST schema proves unworkable post-release, bump to v2, keep v1 emission behind a flag for one release, deprecate. No user code path is lost.

---

## 13. Open Questions

1. **Python vs. JS as the default canonical-extension language.** Default theme ships canonical extensions in one language — which? Python is more familiar to the data/notes-writing crowd; JS is already in the theme (graph widget, SPA shell). *Proposed:* Python. Revisit if CI-minimal-environment feedback says otherwise.
2. **Persistent-mode restart triggers in serve.** Should edits to a hook file trigger a persistent-mode restart automatically, or require a user action? *Proposed:* automatic on file change (filewatch), with a `--no-hook-reload` flag to opt out.
3. **AST-level opt-out vs. frontmatter opt-out.** Some users may want to mark a single block as "don't transform" via a Markdown-level syntax (e.g., `{: .zetl-skip }`) rather than frontmatter. *Proposed:* defer; frontmatter covers the 80% case. Reconsider if demand emerges.
4. **Tasks extension query syntax.** How much of Obsidian Tasks' DQL to support? *Proposed:* document a named subset (`not done`, `due before`, `path includes`, `tag includes`) as "supported"; anything else is "unsupported, will render as an unsupported-query placeholder." Avoid chasing the full grammar.
5. **Dataview as a fourth canonical extension.** Worth shipping a subset as `dataview`? *Proposed:* defer to a separate SPEC once canonical three are stable. Dataview's query language deserves its own design conversation.
6. **Multi-output hook stages.** Should a `transform` hook be able to emit *multiple* output ASTs (e.g., splitting a page into two)? *Proposed:* no in v1; single-in / single-out per stage. Multi-output is a page-generation concern, not a transform concern.
7. **MCP exposure of hook coverage.** `get_page` via MCP could include which hooks touched the page in a metadata field. *Proposed:* off by default; opt-in via MCP tool parameter.
8. **Selection layer cache.** Should selector evaluation results be cached across pages (e.g., a content probe compiled regex cached, a frontmatter parse reused)? *Proposed:* yes, cache per build; invalidate on file change in serve mode.
9. **Vault-level template vars.** Should there be a `vault.ext.<hook_id>` namespace aggregated across pages (dead-link totals, tag clouds, cross-vault task summaries)? This requires a finalisation pass after all pages render and a distinct protocol message (`{"type": "finalise", ...}` for persistent-mode hooks, or a separate invocation for one-shot). *Proposed:* defer to v1.1; per-page `page.ext` covers the majority case without the finalisation complexity.
10. **Schema declarations for template vars.** Should extensions declare the shape of their emitted vars in the manifest so themes can introspect and zetl can validate? *Proposed:* no in v1; loose typing + documentation suffices. Reconsider if theme-authoring feedback shows "I don't know what fields this extension emits" is a real pain point.
11. **Template-var publishing from `pre-parse` hooks.** `pre-parse` runs before parsing, so extensions there have only the raw Markdown text — less rich than AST-stage extensions. Is it worth supporting emission at that stage, or restrict template-vars to `transform` and `post-render`? *Proposed:* allow at all three stages; the preprocessing stage has legitimate use (e.g., a word-counter that emits `page.ext.word_count.value` without parsing cost).
12. **Executable-bit fallback vs. interpreter map.** Pandoc's filter loader guesses an interpreter from extension when the executable bit is absent (`.py→python`, `.js→node`, `.hs→runhaskell`, etc.; see [filters.html](https://pandoc.org/filters.html)). Should zetl replicate this for authoring ergonomics, or require the executable bit (unambiguous, prevents "why isn't my hook running")? *Proposed:* require the executable bit in v1 — clear error, good diagnostic. Add an extension-to-interpreter map in v1.1 if feedback requests it.
13. **Symbol-scanner analogue for canonical extensions.** SPEC-031 had a symbol-scanner subcommand to prioritise shim work against the Obsidian plugin ecosystem. For SPEC-032's canonical extensions, the analogous question is "which patterns in the wild would benefit from a canonical extension we ship?" — answerable by scanning a sample of published vaults for `> [!`, `ad-*`, `tasks`, inline queries, etc. *Proposed:* defer to a tools-level follow-up, not a spec requirement.

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
- [ ] JSON Schema for the AST written and validated (currently referenced but not yet drafted; lives at `tools/zetl-ast-schema-v1.json`, to be produced in Phase A).
- [ ] Helper-library API sketch (Python, JS) reviewed by respective ecosystem experts.
- [ ] Prior-art literature survey (`tools/parser-lit-survey.md`) — complete (2026-04-19), findings folded into §1.4 Prior Art, REQ-3215..3220, and CON-3201/3202 updates.

Status remains `draft` until these clear.

---

**End of SPEC-032.**
