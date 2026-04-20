# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Render-pipeline hooks (SPEC-032).** A new three-stage hook pipeline
  — `pre-parse` (raw markdown), `transform` (typed AST), `post-render`
  (HTML fragment) — runs alongside the existing SPEC-016 lifecycle
  hooks. Hooks live under `.zetl/hooks/<stage>.d/` and stay resident
  via a JSON-lines persistent-mode protocol over stdin/stdout (CON-3201;
  release p95 ≈ 896 µs for a 500-node AST echo). The pipeline carries
  a typed AST (`zetl-ast` schema v1.0; published as
  `tools/zetl-ast-schema-v1.json`), a `BuildContext` snapshot, and a
  shared `build_data` channel with cross-page visibility.

  Composition resolves theme + vault `<stage>.d/` directories, runs a
  topological sort on the manifest's `[ordering]` table, and applies
  same-name shadow / disable rules. Hooks declare a TOML manifest
  (`<name>.<ext>.toml`) covering selectors (glob + frontmatter
  predicate + content regex), behavioural contracts (`preserves`,
  `idempotent`, `may_restructure`, `expansion_bound`), per-stage
  AST-type opt-in, and timeouts.

- **Hook authoring CLI.** Seven new subcommands close the
  write → run → diff → fix loop:

  ```sh
  zetl hook new <stage> <name> [--lang py|js|sh] [--ecosystem ...]
  zetl hook test <name> [--update]
  zetl hook fixture --from <page> --hook <name>
  zetl hook watch <name>          # restarts persistent process on edit
  zetl hook coverage [--stage X]  # matched pages, invocations, latency
  zetl hook dry-run <stage>/<name>
  zetl hook capabilities [--stage X] [--json]
  ```

  Plus `zetl ast sample <file>` and `zetl ast diff <a> <b>` for AST
  introspection. Each scaffolded hook ships with a starter fixture +
  golden so `hook test` passes immediately on the fresh skeleton.

- **Behavioural contracts and property-test harness.** Hooks declare a
  `[contract]` block; the pipeline enforces `preserves` (named node
  types must survive), `idempotent` (canonical-form equality across
  two runs), `may_restructure` (block-shape gate, pre-parse only), and
  `expansion_bound` (advisory output-size ratio). Contract violations
  surface as `HookDiagnostic` records with the standard five-part
  format (summary / context / observed / cause / hint).

- **Failure scoping.** When a hook errors mid-pipeline, the page reverts
  to the previous stage's output and the pipeline continues — failures
  no longer abort the build. A `FailureRecord` per failure lands in
  `diagnostics.json`.

- **Helper libraries.** `tools/zetl-ast-js/` (TypeScript / npm) and
  `tools/zetl-ast-py/` (Python, hatchling) ship typed AST classes,
  `walk()`/`map_nodes()` traversal, an `@on_node` dispatch decorator,
  and a persistent-mode protocol client. A cross-impl conformance gate
  (`make helper-contracts`) drives all three implementations through
  10 shared JSON fixtures in CI.

- **Plugin ecosystems (SPEC-033).** First-class adapters for Pandoc
  filters (`ecosystem-pandoc`), mdBook preprocessors (`ecosystem-mdbook`),
  and remark plugins (`ecosystem-remark`) — gated behind per-ecosystem
  cargo features (each compiled in by default in release builds).
  Hooks declare `ecosystem = "pandoc"` (or `mdbook`/`remark`) plus the
  per-ecosystem fields the chosen adapter requires (`exec`/`lua_filter`,
  `exec`+`scope`, `package`+`version`+`options`); the pipeline routes
  them through the matching adapter and translates the foreign AST back
  to `zetl-ast` for downstream stages.

  The new `zetl ecosystem check` subcommand reports per-ecosystem
  runtime detection (binary path + version), the count of configured
  hooks, and the set of reachable plugins — exit 0 unless a configured
  ecosystem's runtime is missing.

  Mixed-parser misconfigurations (a hook expecting Pandoc AST attached
  to a CommonMark-parsed page) surface a five-part diagnostic with
  remediation suggestions; `zetl build --strict-parsers` upgrades the
  warning to a fatal error. A per-ecosystem compatibility matrix lives
  at `tools/zetl-ecosystem-matrix.toml` (gated by structural + tier-
  downgrade tests in CI).

- **Safe mode and security policy.** `zetl build --safe-mode` /
  `zetl serve --safe-mode` skips every vault hook and only runs theme
  hooks declared in the theme's `[[theme.hooks]]` manifest table.
  Persistent hooks spawn under a default `SecurityPolicy` that
  redacts the host environment to a small allowlist
  (PATH/HOME/USER/LANG/...), caps stderr at 1 MiB with a truncation
  marker, and rejects messages over 10 MiB in either direction.

- **Capability probes.** `zetl hook capabilities` issues a `probe`
  message to every composed hook and reports its supported stages,
  AST types, and AST schema version; mismatches against the running
  binary's schema version exit non-zero so CI catches drift before
  `build`.

- **Observability.** Hooks emit per-invocation log lines
  (`[zetl] hook: stage=X id=Y page=Z duration_ms=N`) and a build-end
  totals line (`[zetl] hooks: total_invocations=N total_duration_ms=M
  failures=K`). Failures additionally surface `status=failed
  reason=<r>` regardless of verbosity.

- **Documentation.** New guides under `docs/`:
  `docs/canonical-extensions.md`, `docs/hook-security.md`,
  `docs/zetl-ast-reference.md` (auto-generated, CI-gated),
  `docs/ecosystems/{pandoc,mdbook,remark}.md`, and
  `docs/ecosystems/matrix-contribution.md`.

### Fixed

- **`zetl hook new` writes the composition-canonical sidecar manifest.**
  The scaffolder previously wrote `<name>.toml`, but
  `compose_stage` looks for `<name>.<ext>.toml` (e.g. `callouts.py.toml`).
  Freshly scaffolded hooks were silently invisible to the pipeline until
  the manifest was renamed by hand. The scaffolder now emits the
  canonical form directly; `find_scaffolded_hook` (used by `hook test`
  and `hook watch`) accepts both the canonical and legacy filenames so
  hooks scaffolded by older builds keep working.
- **`zetl hook new --ecosystem <id>` seeds the SPEC-033 REQ-3312
  required fields.** Previously the scaffolded manifest carried only
  `ecosystem = "<id>"` and an explanatory comment, so the per-ecosystem
  manifest parser rejected it with the cryptic *"pandoc manifest must
  declare `exec = ...` or `lua_filter = ...`"*. The scaffolder now emits:
  - `pandoc` → `lua_filter = "filters/<name>.lua"` plus a starter
    identity Lua filter on disk at that path, so `dry-run` and `build`
    work without further hand-edits;
  - `mdbook` → `exec = "mdbook-<name>"` + `scope = "page"` (rename
    `exec` or place the binary on `PATH` before `build`);
  - `remark` → `package = "remark-<name>"` (install under the vault's
    `node_modules/`).

  The convention hint also strips ecosystem prefixes from the hook name
  so `zetl hook new transform pandoc-smallcaps --ecosystem pandoc` no
  longer suggests `exec = "pandoc-pandoc-smallcaps"`.

### Changed

- **`--features hooks-v2` umbrella retired.** SPEC-032's three-stage hook
  pipeline (pre-parse / transform / post-render), AST schema v1.0,
  selector evaluator, and persistent-mode protocol are default-on. The
  umbrella was originally planned as a Phase-A preview gate; in practice
  it shipped unconditionally because the schema converged faster than
  expected, so no `--no-hooks-v2` opt-out is provided (there is nothing
  to opt out of). To skip every hook, use the existing
  `zetl build --no-hooks` flag. (SPEC-032 §12 Phase D)
- **`--features ecosystems-v1` umbrella retired.** All three
  ecosystem adapters (Pandoc, mdBook, remark) have shipped stable
  across two consecutive releases, so the preview umbrella that
  bundled them is gone. The per-ecosystem cargo flags
  (`ecosystem-pandoc`, `ecosystem-mdbook`, `ecosystem-remark`)
  remain and are now the stable compile-time surface.

  **Migration.** Release binaries already compile every adapter in
  by default — if you use a packaged build, nothing changes. If you
  build from source and were passing `--features ecosystems-v1`,
  replace it with the three per-ecosystem flags explicitly:

  ```sh
  # before
  cargo build --features ecosystems-v1
  # after
  cargo build --features "ecosystem-pandoc ecosystem-mdbook ecosystem-remark"
  ```

  A minimal build that drops every ecosystem is
  `cargo build --no-default-features`. Each per-ecosystem flag can
  still be toggled independently (see `docs/ecosystems/*.md` for the
  corresponding opt-out instructions). The
  `ecosystems_v1_umbrella_is_retired` integration test guards
  against accidental reintroduction. (SPEC-033 §12 Phase F)

## [0.3.0] - 2026-04-17

### Added

- **Interactive graph view.** The default theme ships a WebGL-rendered
  graph of the vault, powered by Sigma.js v3, graphology, and
  ForceAtlas2 (run in a Web Worker when available). A persistent widget
  appears on every page with three modes — `local` (current page's
  neighbourhood), `vault` (whole vault), and `off` — switchable from the
  widget and remembered in `sessionStorage`. Clicking a node navigates
  to that page. Dead-link nodes and edges render in a muted, dashed
  treatment. A new `/_graph` route (and `_graph.html` under
  `zetl build`) exposes the full-screen view, and the sidebar gains a
  **Graph** link. Dependencies are vendored under
  `themes/default/static/vendor/sigma/` (no CDN at runtime). (SPEC-028)
- **SPA navigation shell.** When `theme.toml` sets `[spa] enabled=true`
  (default on for the bundled theme), a small (<100 loc) vanilla JS
  module intercepts same-origin link clicks, fetches the next document,
  and swaps the `<main data-zetl-volatile>` element in place —
  preserving the WebGL context, Sigma camera state, and any other
  persistent-shell state across navigations. Modifier clicks
  (meta/ctrl/shift/middle-click, `target=_blank`, cross-origin) fall
  back to native behaviour, `popstate` is handled, and inline `<script>`
  tags in swapped content are re-executed. The shell dispatches
  cancelable `zetl:before-navigate` and `zetl:after-navigate` window
  events around each transition so themes and widgets can react without
  re-instantiating. (SPEC-028 / REQ-113 / REQ-115)
- **Persistent widget placement.** Default placement is a docked
  bottom-right mini-map (280×200 px, CSS resize handle, click-to-expand
  to `/_graph`). `theme.toml [graph.placement]` opts into `tabs` or
  `stacked` layouts via a `data-placement` attribute on the shell
  container — no template editing required. Below
  `--zetl-graph-widget-breakpoint` (default 900 px) the widget hides
  and is reachable via a top-bar toggle that expands to a full-screen
  overlay (focus ring, Enter/Space to open, Escape to dismiss);
  visibility-only toggling keeps the same Sigma instance live.
  (REQ-116, REQ-117)
- **CSS custom-property theming contract.** The full `--zetl-graph-*`
  and `--zetl-shell-*` variable surface is declared with sensible
  defaults; Sigma node/edge reducers read them via `getComputedStyle`,
  so themes restyle the graph with CSS alone (no JS override). The
  theme authoring reference documents every variable, the
  `{% block persistent_shell %}` / `{% block graph_widget %}` contract,
  the `[spa]` and `[graph.placement]` `theme.toml` tables, and the
  navigation events. (REQ-114)
- **Graceful absence matrix.** `<noscript>` fallback renders a
  `<details>`-grouped page list alongside the canvas for keyboard /
  screen-reader users; empty-state copy covers zero-page and zero-link
  vaults; sidebar link and `/_graph` route tolerate themes that strip
  `_graph.html`. `axe-core` reports zero critical violations on
  `/_graph`. (REQ-109, NFR-105)
- `graph-index.json` — graphology-serialised directed graph
  (`format: "zetl-graph/v1"`, stable alphabetical ordering,
  per-node `{label, slug, outlink_count, backlink_count, is_orphan,
  is_dead, tags}`). Written to `<out>/graph-index.json` by
  `zetl build`, served at `GET /graph-index.json` by `zetl serve`, and
  injected into templates as `graph_index_url` (always) plus
  `graph_index` (when `theme.toml` sets `graph_inline=true`).
  (REQ-101 / REQ-102 / REQ-103 / REQ-104 / REQ-105 / CON-101 / CON-102)
- `zetl stats` gains a **Graph:** section (bytes / nodes / edges) in
  table output and a `graph` field in `--json` output.
- Client-side performance marks: `zetl:graph:render:start` +
  `zetl:graph:render` around FA2 layout, and `zetl:navigate` around SPA
  transitions, for devtools / NFR harness consumption. (OBS-201 /
  OBS-113)
- Verbose logging: `[zetl] graph-export: pages=N edges=M
  duration_ms=X bytes=Y` under `zetl build --verbose`. (OBS-101)

### Performance

- NFR gates enforced in CI via a Playwright harness against 2k- and
  5k-page synthetic vault fixtures: `/_graph` LCP ≤ 1500 ms P95
  (NFR-101), scripted-drag ≥ 30 fps (NFR-102), gzipped vendor JS
  ≤ 250 kB (NFR-103), and `graph-index.json` ≤ 1 MB at 2k pages with
  a stderr warning at 5k (NFR-104).

## [0.2.7] - 2026-04-15

### Fixed

- Build: cfg-gate Unix mode bits in `hooks` so the Windows
  cross-compile target (`x86_64-pc-windows-gnu`) compiles cleanly.

## [0.2.6] - 2026-04-15

### Fixed

- CI: disable `git2`'s default features (`https`, `ssh`). zetl only
  uses git2 for local repo inspection and auto-commits (no network),
  so both are dead weight. Dropping `https` also eliminates
  libgit2-sys's unconditional `stransport.c` compile on Apple targets
  — which was blocking the macOS cross-compile even with
  `vendored-openssl`, because libgit2 links `Security.framework`
  directly without an escape hatch.

## [0.2.5] - 2026-04-15

### Fixed

- CI: macOS cross-compile was failing one level deeper than 0.2.4 —
  `libgit2-sys` defaults to Apple Secure Transport (`stransport.c`)
  for HTTPS on darwin targets, which requires a complete
  `Security.framework` that osxcross doesn't ship. The
  `vendored-openssl` feature now also enables `git2/vendored-openssl`,
  routing libgit2's HTTPS through the already-vendored OpenSSL.

## [0.2.4] - 2026-04-15

### Fixed

- CI: macOS (arm64 + x86_64) and Windows release builds were failing
  in `openssl-sys` because no host sysroot OpenSSL exists for those
  cross-compile targets. Adds an opt-in `vendored-openssl` cargo
  feature that statically links a self-contained OpenSSL into the
  binary; enabled in the macOS and Windows release jobs. Linux builds
  continue to dynamic-link against the system libssl.

## [0.2.3] - 2026-04-15

### Fixed

- CI: arm64 (`aarch64-unknown-linux-gnu`) release build was failing in
  the `openssl-sys` build script because the cross-compile step
  installed only the gcc toolchain, not the aarch64 OpenSSL headers.
  Now enables Debian arm64 multiarch + `libssl-dev:arm64` and
  configures `pkg-config` for cross queries. Re-enables shipping arm64
  Linux binaries to `files.anuna.io/zetl/v0.2.3/`.

## [0.2.2] - 2026-04-15

### Changed

- Internal: `cargo fmt` pass across the 0.2.0 / 0.2.1 changesets. No
  behaviour change.

## [0.2.1] - 2026-04-15

### Fixed

- `zetl hook run` / in-process hook execution: retry `spawn()` on Linux
  `ETXTBSY` ("Text file busy") up to 20× with 10ms backoff. Defeats a
  kernel-level race when a hook script is written and immediately
  executed from the same process (common in tests, theme installers,
  and agent tooling that generates hooks on the fly).

## [0.2.0] - 2026-04-15

### Changed (breaking)

- **Vault scan now skips dotdirs by default.** Directories whose name starts with
  `.` (e.g. `.claude/`, `.obsidian/`, `.vscode/`, `.cache/`, `.venv/`,
  `.terraform/`) are no longer walked by `zetl build`, `zetl serve`,
  `zetl index`, `zetl search`, or `zetl watch`. Previously these were scanned
  unless explicitly ignored, causing tool-state and AI-agent scratchpads to
  leak into `dist/` and pollute the link graph and search index. The
  hardcoded force-ignores (`.git/`, `.zetl/`, `node_modules/`) behave as
  before. Dotfiles at the vault root (e.g. `.hidden-note.md`,
  `.zetlignore`, `.gitignore`) are still walked. (SPEC-026)

  **Migration:** if you intentionally publish a dotdir, either pass
  `--include-hidden` or add a negated pattern to a `.zetlignore` file at
  the vault root (e.g. `!.archive/`).

### Added

- **History UI surfaces on the default theme.** Every page now shows an
  inline metadata strip with `Last changed`, a humanised `stable` label
  (e.g. `3d`, `2w`, `9mo`), and a `history` link. A new `/_history`
  route (and `_history.html` under `zetl build`) surfaces vault-wide
  recent changes with a link-count trend sparkline and a
  reverse-chronological list of added / modified / removed pages. The
  sidebar gains a "Recent changes" link. All surfaces degrade silently
  when history is absent (SPEC-027 / REQ-305). Static builds now emit
  `pages/<slug>/_history.html` alongside `index.html` so deployed vaults
  get the same temporal affordances as `zetl serve`. Requires the
  `history` feature (enabled in default builds).
- `--exclude PATTERN` (repeatable) and `--include-hidden` flags on
  `zetl build`, `zetl index`, `zetl serve`, `zetl search`, and `zetl watch`.
  `--exclude` accepts gitignore-syntax patterns; `--include-hidden` disables
  the new dotdir default while preserving the level-1 `.git/`/`.zetl/`/
  `node_modules/` force-ignore. (SPEC-026)
- `.zetlignore` is now a documented first-class feature. Patterns use
  gitignore syntax and are evaluated relative to the vault root. Negated
  patterns (`!foo`) override the default dotdir exclusion.
- With `--verbose`, the scanner prints one stderr line per skipped path
  with a `reason=` tag (`hardcoded`, `nested-vault`, or `dotdir`) for
  debugging unexpected omissions.

### Fixed

- `zetl stats`: `grounded_spl_blocks` could exceed `spl_blocks` when the theory
  cache outlived deleted SPL blocks. Grounded / grounding counts are now joined
  against the live pipeline so only currently-present blocks are counted.
  (BUG-001)
- `zetl serve`: unknown pages now respond `404 Not Found` instead of `200 OK`.
  The "create this page" body is preserved — only the status code changes —
  so uptime probes, crawlers and monitoring see the correct signal. (BUG-002)
- `zetl build`: accept `--out` and `-o` as aliases for `--out-dir`. (BUG-005)

### Added

- `zetl completions <shell>` — generate shell completion scripts for bash, zsh, fish, elvish, and powershell.
- `zetl man` — generate a roff(7) man page on stdout. `make install` places it at `$(PREFIX)/share/man/man1/zetl.1` so `man zetl` works out of the box.
- `--no-input` global flag for unattended / CI usage; disables interactive prompts such as the `zetl view` page picker.
- Release profile tuning: `lto = true`, `codegen-units = 1`, `strip = true` (cuts binary size ~40%).
- Release pipeline: `release.sh`, `install.sh`, and `.woodpecker/release.yaml` for cross-platform binary distribution via Cloudflare R2.

### Changed

- Global flags (`--json`, `--format`, `--dir`, `--quiet`, `--verbose`, `--no-color`, `--no-cache`, `--at`) now propagate to subcommands — `zetl list --json` works, not just `zetl --json list`.
- JSON error output now goes to stderr instead of stdout, so `zetl … | jq` consumers get clean stdout on both success and failure.
- README: regrouped feature bullets into six themed sections; tagline broadened from "personal knowledge management" to "knowledge management, solo or team" to reflect the multi-user collab feature set.
- `make install` now also installs the man page and bash/zsh/fish completions under `$(PREFIX)/share/`.

### Fixed

- `Cargo.toml` license field corrected from `MIT` to `AGPL-3.0-or-later` (LICENSE file has always been AGPL).
- `Makefile` SPDX header corrected from `MIT` to `AGPL-3.0-or-later`.
- Stale `github.com/anuna/zetl` link in `--help` footer now points at `codeberg.org/anuna/zetl`.

## [0.1.0] — unreleased

Initial public release.
