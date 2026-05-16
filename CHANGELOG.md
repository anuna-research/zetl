# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Pluggable `--collab` authentication** ([SPEC-041](specs/SPEC-041-pluggable-collab-auth.md)).
  `[collab.auth] methods = [...]` in `.zetl/config.toml` selects from
  six authenticators that share one `Authenticator` trait and one
  `auth_resolve` middleware:
  - **`passkey`** (default) — pre-SPEC-041 WebAuthn behaviour, unchanged.
  - **`agent-token`** (default) — pre-SPEC-041 Bearer behaviour, unchanged.
  - **`proxy-header`** — `X-Forwarded-User` from a trusted upstream
    proxy. Three-layer trust gate: `--trust-proxy` + `peer_allow` CIDR
    list + strict header-value grammar.
  - **`password`** — argon2id static passwords in
    `.zetl/collab/passwords.json` (0600). New `zetl collab passwd
    add/remove/list` CLI; TTY-only password entry.
  - **`capability-url`** — signed `?cap=<EdDSA-JWT>` bearer URLs bound
    to a scope glob and a role. New `zetl collab share
    mint/list/revoke` CLI. Pseudonymous scope-capped principals — never
    satisfy `admin_gate` regardless of encoded role. `Referrer-Policy:
    no-referrer` set on responses so the token does not leak via the
    `Referer` header.
  - **`oidc`** — OpenID Connect Authorization Code Flow with PKCE
    (`--features collab,collab-oidc`). Discovery + JWKS + ID-token
    validation rolled by hand on `jsonwebtoken` + `reqwest` to keep
    the dependency surface auditable. The default `--features collab`
    build gains zero new dependencies (NFR-4102).

  A vault with no `[collab.auth]` block behaves exactly as before
  (`methods = ["passkey", "agent-token"]` default). Operator guide:
  [`docs/collab-auth.md`](docs/collab-auth.md). Threat model:
  [`research/SPEC-041-threat-model.md`](research/SPEC-041-threat-model.md).

  SPEC-041 itself remains `draft` / `0.1.0-strawman` pending the
  cross-model Tier-1 review of the implementation (the in-session
  self-review checkpoint is at
  [`.hence/reviews/auth-tier1-2026-05-15.md`](.hence/reviews/auth-tier1-2026-05-15.md)).

## [0.7.2] - 2026-05-13

### Added

- **`zetl skill init` self-bootstraps a Claude Code skill** for the
  current vault under `.claude/skills/zetl/`, so agents working inside
  the vault pick up zetl-specific conventions automatically. Documented
  in the user-guide CLI Overview and MCP Server pages.
- **Folder-grouped index** is the new default for multi-folder vaults
  in the bundled theme. Single-folder vaults keep the flat alphabetical
  index.
- **`gen-vault` synthetic-vault generator** for build-perf
  measurements, plus a `perf-diff.sh` determinism check (`scripts/`)
  that catches non-deterministic byte drift between two builds.

### Performance

- **Parallel scanner + page render** via `rayon`. Per-file parse and
  per-page render now fan out across cores. Combined with the
  pre-pass items below, end-to-end build wall-clock improves
  substantially on multi-folder vaults; baseline + post-pass figures
  recorded in `bench/`.
- **`PageNameResolver` replaces the `O(n)` `Vec` scan in
  `resolve_page_name`** so wikilink resolution drops from quadratic to
  near-linear on large vaults.
- **Hoisted `git2::Repository::discover` out of the per-page loop**;
  one discovery per build instead of one per page.
- **Dropped a redundant `has_pages` check** from the folder-index
  pass.

### Fixed

- **`zetl serve` / `zetl build` mobile shell (SPEC-040).** A clutch of
  fixes for the embedded Tauri WebView:
  - Mixed-content (`HTTPS → http://127.0.0.1` loopback) allowed via
    `MainActivity` flag — the embedded serve port is loopback-only, so
    cleartext to it is safe and required.
  - `HOME` set before `libgit2` caches it in the Tauri `setup` hook;
    seed an empty `known_hosts` so `libssh2` init succeeds.
  - Trust-on-first-use SSH host-key check for first-time mobile clone.
  - Tauri `opener:open-url` capability authorised for the outbound
    URL handler.
  - Vault sub-path picker counts subpath candidates recursively (a
    vault may live below the repo root).
  - Theme picker auto-picks the active vault's theme on launch.
  - Cancel button on the capture form; back link on the onboarding
    page when vaults already exist; in-page confirm modal for
    Remove (was using JS `confirm()`).
- **`fix(theme/default)`**: keep `<main>` width stable on pages with
  no transclusions (eliminates a layout shift).

### Documentation

- BUG note: capability-mode build error reproduction case.

## [0.7.0] - 2026-05-09

### Added

- **RSS / Atom / JSON Feed support (SPEC-038 v1.0.0).** Pure-core
  serialisers in `src/feed/` produce standards-conformant RSS 2.0,
  Atom 1.0, and (opt-in) JSON Feed v1.1 outputs from a unified
  `FeedItem` projection of the vault snapshot. Supported surfaces:
  - **Outbound build mode** emits `dist/feed.xml`, `dist/atom.xml`, and
    optionally `dist/feed.json`; pages get `<link rel="alternate">`
    discovery tags. Determinism is byte-stable across rebuilds (NFR-3804
    + NFR-3805) — same vault produces byte-identical feed bytes.
  - **Outbound serve mode** exposes `GET /feed.xml`, `/atom.xml`, and
    `/feed.json` with collab-mode `no-store` vs public `max-age=300`
    cache controls.
  - **Hugo's scoped subscriptions** — public catalog at
    `/.well-known/zetl-subscriptions.json` advertising every
    `[[feed.scopes]]`; per-scope feeds at the configured paths;
    AST-backed changelog feed with monotonic sequence numbers and
    sealed archive ranges (REQ-3816 / REQ-3817).
  - **Capability cohort feeds** — opt-in per-cohort feeds at
    `/caps/<token>/feed.xml`, never advertised in the public catalog,
    with token-leak audit (REQ-3829..REQ-3831 + NFR-3809).
  - **Inbound feed pull** — SSRF-safe (RFC 1918 / link-local /
    loopback / multicast / RFC 6598 / file:// / data:// rejected at
    every redirect hop), XXE-safe (DOCTYPE + ENTITY rejection),
    decompression-bomb-safe (1 MiB cap), conditional-request
    (If-Modified-Since / If-None-Match) honouring; first-seen identity
    dedup over GUID + canonical-link + content-fingerprint (REQ-3812).
  - **Inbound authentication** (Basic / Bearer / query-param) reading
    from `.zetl/credentials.toml` (mode-0600 enforced; credentials
    never accepted in `.zetl/config.toml`); cross-origin redirect
    drops credentials (REQ-3826); persistent 401/403 enters suspended
    state until operator action (REQ-3828).
  - **Creative-Commons-aware republication** (REQ-3818..REQ-3823 +
    ADR-3809). Per-license eligibility table (CC0 → full, CC-BY →
    full/excerpt by operator choice, CC-BY-SA → compatible-vault gate,
    CC-BY-NC → non-commercial-vault gate, CC-BY-ND → excerpt-only,
    Unknown → default-deny unless `i_have_permission=true`).
    Attribution preservation enforced at build time; retraction
    propagates from source to local + republished feeds.
  - **Per-subscription retention** with archive-not-delete default
    (ADR-3812); explicit erasure via
    `zetl feed forget <sub-id> <pattern>` mints tombstone records that
    block re-import on subsequent fetches (REQ-3834 / T22).
  - **CLI surface** — `zetl feed pull|list|status|validate|forget`
    with consistent `--json` autodetection and exit codes matching
    the rest of zetl.
  - **Observability** — `zetl_feed_*` Prometheus-style counters and
    `zetl_feed_build_duration_seconds` histogram, with bounded
    cardinality (subscription_id / cohort_id from config; license /
    decision / action / reason from closed enums; never `page_slug`
    as a label). Cohort token labels enforce REQ-3831's never-leak
    invariant.

  Tracked in plan `IMPL-038` (33 tasks); follow-up wires landed under
  `IMPL-038-wires` (5 tasks: outbound emission into `zetl build`, CLI
  surface attach, fail-loud config validation, theme Subscribe
  affordance, end-to-end playtest). 2,169 lib + 24 integration tests
  cover per-format determinism, cross-format equivalence, threat-model
  corpus (T1..T22), the CC eligibility matrix, RFC 4287 §4.1.1
  feed-author conformance, and JSON Feed v1.1 minimum-required-fields.

  Operator how-to: [`user-guide/reading/Feeds.md`](user-guide/reading/Feeds.md).
  Reference spec: `specs/SPEC-038-rss-support.md`. CLI surface in v1.0:
  `zetl feed validate` (offline strict-parser smoke test) is fully
  wired; `pull|list|status|forget` exit non-zero with structured
  "not yet wired" stubs pointing at the deferred shell-side work.

### Fixed

- **BUG-001**: Windows-authored nested-page paths produced an empty
  `<article>` body. `page_slug_from_path` now normalises backslash
  separators; a slug-miss diagnostic surfaces the offending path, and
  a Windows cross-check job in CI guards against regression.

> **Note.** `0.7.1` was skipped; no release was cut under that version.

## [0.6.1] - 2026-04-21

Hot-fix version-bump released the day after `0.6.0`. No source
changes between the two tags.

## [0.6.0] - 2026-04-21

### Added

- **Comprehensive user guide.** A ~40-page guide authored *as a zetl
  vault* lives under `user-guide/`; covers install, CLI overview,
  reading flow, editing flow, theming, hook authoring, capability
  mode, and the MCP server. The guide is itself an example of an
  externally-authored zetl vault.
- **Heading-slug ids** are emitted alongside line anchors on rendered
  pages, so deep-link URLs (`#section-name`) and Wikilink heading
  targets (`[[Page#Section]]`) now resolve to the heading element
  directly.
- **Capability-URL distribution (SPEC-034 v0.4.0).** A new
  `zetl build --capability` build mode encrypts every page with
  [`age`](https://age-encryption.org) and signs the resulting envelope
  with an Ed25519 vault-signing key, so a purely static host can serve
  a reader-scoped wiki without any server-side auth. Two authentication
  modes are selectable per cohort:

  - **Delegated-URL mode (default).** The operator mints a reader-specific
    X25519 keypair; the private half travels in a URL fragment that the
    reader's browser binds to a WebAuthn passkey on first visit
    (Trust-on-First-Use). After the TOFU handshake, the passkey-wrapped
    key is the reader's durable credential. Optional split-key mode
    (`--split-key`, REQ-3430) delivers the second half out of band via a
    spoken phrase or QR code to mitigate pre-first-click URL harvesting.
  - **WebAuthn-PRF-only mode (hardened, opt-in).** Readers self-enrol at
    a static `/enroll.html`, deriving a long-term X25519 identity from a
    per-cohort PRF output (REQ-3414 salts defeat cross-cohort pubkey
    linkage) and sending the public key to the operator out of band. URLs
    carry no cryptographic material.

  Both modes share the same pipeline: Ed25519 signature verification
  (REQ-3427) blocks CDN substitution, X25519 padding (REQ-3422) masks
  cohort size to a declared tier, `navigator.locks` serialises
  concurrent-tab TOFU (REQ-3429), and the browser shim unregisters any
  pre-existing ServiceWorkers on load (REQ-3428). Revocation is
  rebuild-and-redeploy; forward secrecy is an explicit non-goal
  (NFR-3414).

  A new `zetl cap` subcommand suite covers the full operator lifecycle:

  ```sh
  zetl cap genkey                      # mint ZETL_CAP_SECRET + Ed25519 vault-signing key
  zetl cap invite <name> --cohort <id> # issue an invite URL (delegated-URL) or entry URL (hardened)
      [--expires <d>] [--pages <filter>] [--split-key]
  zetl cap list    [--cohort <id>] [--output json|text]
  zetl cap revoke  <grant-id>
  zetl cap rotate  --cohort <id>       # new content salt; path-caps stable (REQ-3402)
  zetl cap finalise <grant-id>         # set bound=true post-confirmation
  zetl cap rotate-signing-key          # rotate Ed25519 key + rebuild all pages
  zetl cap check                       # stale-grant + public-repo-safety audit
  zetl cap sweep                       # mark past-expires revoked
  zetl cap pair                        # SPAKE2 pubkey handoff
  zetl cap audit-diff <old> <new>      # PR-gate malicious-author check (REQ-3424)
  zetl cap emergency-shutdown          # live incident-response checklist
  ```

  Configuration lives under `[access]` in `zetl.toml`
  (`[access.signing]`, `[access.split_key]`, `[access.sw_hygiene]`,
  cohort tables); `grants.toml` and `recipients.toml` track issued
  grants and out-of-band public keys. Deploy artifacts include
  `dist/assets/shim.js`, `dist/assets/vault-signing-key.pub`, per-host
  header snippets for `Cache-Control` + `Clear-Site-Data`, and the
  `/c/<path-cap>/<slug>.html` layout defined in CON-3401.

  **Operator documentation.** The task-oriented walkthrough is
  [`docs/capability-mode.md`](docs/capability-mode.md) (threat model,
  per-cohort mode selection, quickstart, grants lifecycle, deploy
  recipes, troubleshooting). The long-form security reference —
  quantitative bounds, acknowledged residuals, and the full attack /
  mitigation matrix — is [`docs/capability-security.md`](docs/capability-security.md).
  The signing-key lifecycle has its own reference in
  [`docs/signing.md`](docs/signing.md); reader-side error remediation
  is in [`docs/reader-troubleshooting.md`](docs/reader-troubleshooting.md).

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

### Fixed

- **`fix(semantic)`**: attention-weighted mean pooling + pad-to-256.
- **`fix(theme/default)`**: hide the transclusion panel on mobile.
- **User-created themes inherit the bundled default's static assets**,
  so a theme that overrides only one template no longer 404s on the
  default's CSS/JS.
- **`fix(hooks/persistent)`**: retry `ETXTBSY` inside
  `spawn_with_policy` so hook spawns don't race the executable's own
  `close()`.

## [0.5.0] - 2026-04-19

### Added

- **SPEC-031..033 drafts.** New design specs for the three-stage hook
  pipeline (SPEC-032) and ecosystem bridges (SPEC-033) landed in
  `specs/`. Implementation followed in `0.6.0`.
- **Graph tap-to-focus, single-click-focus, double-click-navigate.**
  On `/_graph`, a single click on a node focuses it (highlights its
  1-hop neighbourhood); a double-click navigates. Mobile gets the
  same behaviour via tap-once-to-focus, tap-again-to-open.

### Performance

- **`gzip` + `brotli` response compression** on `zetl serve` (text/*,
  application/javascript, application/json, etc.).
- **Brotli precompression** of build output — `zetl build` now emits
  `*.br` alongside HTML/JS/CSS, so a static host can serve the
  pre-compressed bytes directly with `Content-Encoding: br`.
- **ETag + `304 Not Modified`** on `text/html` responses.
- **Extracted inline `<style>` (~25 KB) to `/_static/shell.css`** and
  inline page + graph `<script>` blocks to `/_static/*.js`, so the
  shell hits the browser cache after first navigation and per-page
  HTML stays small.
- **Externalised `pages.json`** out of the inline template so the
  pages-index is cacheable and SPA-friendly.
- **`Cache-Control` on `/_static/*`** in serve mode (long-lived,
  immutable for fingerprinted assets).

### Fixed

- Graph node clicks were broken in serve mode after the script
  extraction; a missing event-binding inside the externalised module
  was the cause.
- SPA scroll and backlink highlight after navigate.
- `theme-install` chmod is now `cfg(unix)`-gated so the Windows
  cross-compile compiles cleanly (parallel guard to `0.2.7`).
- Tailwind content scan now covers `themes/default/**/*.js` so
  utility classes used by extracted scripts survive purge.
- Mobile stats overflow on `/stats`.

## [0.4.0] - 2026-04-19

### Added

- **CRDT-flush author attribution.** When a multi-user editing session
  flushes to git, the resulting commit credits every distinct
  contributor: the primary author plus `Co-authored-by:` trailers for
  each additional editor. `PageHistoryEntry` now parses those trailers
  and the default theme renders co-authors in the page-history byline.
- **Per-slug CRDT contributor tracking** in the websocket layer feeds
  the flush attribution above.
- **Print styles** for the default theme: edit button hidden,
  breadcrumbs flattened, sidebar collapsed; tested end-to-end against
  the print-CSS coverage plan.
- **Search-snippet highlighting (BUG-504).** Query matches are
  highlighted in search-result excerpts.
- **Inline SVG favicon** in the base template (no extra HTTP request,
  no theme override required).
- **`_graph.html` rendered during static build** so the graph view is
  reachable on every static deployment, not just `zetl serve`.
- **Tag cloud** widget and **dead-link node-and-edge styling**
  (muted + dashed) on `/_graph`.
- **Design polish on the graph:** focus mode with depth=0,
  search-highlight, degree filter, folder-coloured nodes, mobile
  controls, drawer z-index fixes.

### Documentation

- **SPEC-030 — theme data contract**: the external-facing template
  context is now a documented surface; a strict-undefined contract
  test exercises every render path so unknown variables fail loudly.

### Fixed

- **BUG-501**: "doesn't exist" banner now shows on the new-page save
  flow.
- **BUG-502**: "Recent changes" sidebar link restored on `/edit/`.
- **BUG-503**: print polish — hide Edit button, flatten breadcrumbs.
- **BUG-505**: prevent mobile label clipping via responsive
  `stagePadding`.
- **BUG-506**: replace default `:visited` magenta on wikilinks +
  backlinks.
- Graph render repair: defer vendor JS, stop mobile clipping.
- Backlinks dedupe by source and drop the raw line label.
- Guard Sigma instantiation against zero-width container (widget on
  hidden tab no longer crashes).
- SPA: force a full reload on `/edit/` transitions so CodeMirror
  remounts cleanly.
- Allow `data:` images under the editor CSP so the favicon loads.
- Shrink default theme under the bundle budget; clippy clean.
- "Recent changes" link visible on `/help` and `/{slug}/_history`
  too.

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
