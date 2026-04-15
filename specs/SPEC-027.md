---
title: "SPEC-027: History UI — Per-Page and Per-Vault History Surfaces"
version: 0.1.0
status: draft
date: 2026-04-15
audience: agent, human
parent: SPEC-017
related:
  - SPEC-017  # zetl history (temporal graph)
  - SPEC-019  # Git commit anchoring in snapshots
  - SPEC-020  # Multi-user collaborative editing
---

# SPEC-027: History UI — Per-Page and Per-Vault History Surfaces

## Information Table

| Field          | Value                                                                       |
| -------------- | --------------------------------------------------------------------------- |
| Document ID    | SPEC-027                                                                    |
| Title          | History UI — Per-Page and Per-Vault History Surfaces                        |
| Version        | 0.1.0                                                                       |
| Status         | Draft                                                                       |
| Author         | Agent (USDD Protocol v1.3.0)                                                |
| Date           | 2026-04-15                                                                  |
| Audience       | Agent, Human                                                                |
| Trace          | USDD Agent Protocol v1.3.0                                                  |
| Parent         | SPEC-017: zetl history — Invisible Temporal Graph                           |
| Related        | SPEC-019 Git Commit Anchoring, SPEC-020 Multi-User Editing                  |
| Dependencies   | `history` feature (jj-lib backed snapshots); minijinja templates            |

---

## 1. Overview

`zetl` has invested substantial infrastructure in temporal history (SPEC-017, SPEC-019): snapshotting, per-page timelines, backlink `since` attribution, vault trend sampling, a `history-index.json` export, and a serve-only `/pages/<slug>/_history` route backed by `render_page_history` and a 325-line default `page_history.html` template. The data pipeline runs on every serve and every build.

**The user-facing surface is not wired up.** The default `page.html` (`themes/default/page.html`) contains no reference to `page.history`, no link to the per-page history route, and no metadata strip. There is no per-vault "recent changes" template. Build mode emits `history-index.json` but no static HTML for either surface. A user running stock zetl cannot discover that history exists unless they type the URL by hand.

**Note on URL scheme.** The existing per-page route is `/{slug}/_history` — the `_` prefix avoids collision with a user page literally named "history" (src/web/routes.rs:374–377). This spec preserves that convention and extends it to the vault-wide page: `/_history` at vault root, and `_history.html` under `zetl build`.

This spec closes that gap by specifying two discoverable UI surfaces on the default theme, with parity between `serve` (dynamic) and `build` (static) modes.

### 1.1 Motivation

- **Discoverability.** History is a zetl differentiator vs. flat static-site wiki tools. Without surface affordances, the feature is invisible.
- **Karpathy's LLM-wiki pattern.** A wiki whose state is visible as a trend (growing / decaying / stable) is meaningfully more useful to both humans and agents than a point-in-time snapshot. zetl already computes the data — we just need to render it.
- **Stale-content signal.** `page.history.stable_days` is already populated. Surfacing it in the page UI turns it from latent data into a decision signal ("this note hasn't changed in 9 months — still accurate?").
- **Static parity.** Users deploying to a CDN via `zetl build` currently lose all history UI. The data is there (`history-index.json`); the rendering is not.

### 1.2 Design Principles

1. **Data already exists — render, don't recompute.** `page.history`, `vault.history`, and `history-index.json` are the sole sources of truth for this UI.
2. **Serve and build reach parity.** Every history surface that renders under `serve` MUST also be emitted as static HTML under `build`. Mutating surfaces (restore, diff-interactive) remain serve-only and are explicitly gated.
3. **Metadata on the page is minimal by default.** The inline metadata strip on `page.html` is a single visually-light line; a graph view lives on the dedicated history page, not the page itself.
4. **Themeable.** Every new template MUST live under `themes/default/` and be overridable via `.zetl/themes/<theme>/`.
5. **Graceful absence.** When `page.history` or `vault.history` is `null` (history feature disabled, no snapshots yet), the UI degrades silently — no metadata strip, no history link rendered, no error.
6. **No new data pipelines.** If a proposed UI element cannot be rendered from existing `page.history` / `vault.history` / `history-index.json` fields, it is out of scope for this spec.

### 1.3 Scope

**In scope:**

- Inline metadata strip on `themes/default/page.html` rendering `page.history.last_changed`, `stable_days`, and a link to the per-page history page.
- Static emission of `/pages/<slug>/_history.html` under `zetl build` for every page (mirrors existing serve route).
- A new per-vault history page (`vault_history.html` template) showing recent changes, vault trend sparkline, snapshot count, and links to affected pages. Served at `/_history` and emitted as static `_history.html`.
- Sidebar / footer link from default theme to `/_history`.
- Graceful absence handling (null/empty history).
- Documentation: `README.md` "History UI" section; CHANGELOG entry.

**Out of scope:**

- Interactive diff viewer and restore affordances (serve-only, already exist via `/api/history/*`; no spec change here).
- A "log.md" append-only per-Karpathy convention (interesting, but a separate spec).
- Per-user / ACL-gated history views (SPEC-020 territory).
- Changes to snapshot storage, jj integration, or the cache format (SPEC-017, SPEC-019).
- New data fields on `page.history` or `vault.history` — anything missing is deferred to a successor spec.
- Custom themes beyond updating `default/`. Other bundled themes (`docs`, `fountain`, `minimal`) may opt in; not required.

---

## 2. User Profiles

### 2.1 Agent-Using Knowledge Worker (carries from SPEC-001)

Browses their vault via `zetl serve` on localhost and occasionally publishes via `zetl build` to a static host. Has accumulated 3–18 months of snapshots. Wants to answer "when did I last touch this?" and "what did I change this week?" without leaving the browser.

### 2.2 Reader of a Published Vault

Lands on a statically-hosted zetl vault via a public URL. Has no shell access. Expects the published site to expose the same temporal affordances they see when browsing locally — "last updated" dates, recent-changes feed, trend indicators. Currently gets none of these.

### 2.3 Agent (LLM) Reading via Static Export

Fetches HTML pages from a deployed zetl build. Uses visible metadata (last-changed dates, stability signals) to judge how much to trust a page's claims. Currently must parse `history-index.json` out-of-band; the HTML carries no signal.

---

## 3. Happy Paths

### 3.1 Happy Path: Discoverable Page History

**Preconditions:** User runs `zetl serve` on a vault with ≥2 snapshots of the page `Note A`.

**Steps:**

1. User navigates to `/pages/note-a/`.
2. Page renders with a metadata strip: `Last changed 2026-03-18 · stable 28d · [history]`.
3. User clicks `history`.
4. Browser loads `/pages/note-a/_history`, showing timeline of changes with timestamps, diff summaries, and a link-count trend sparkline.
5. User clicks browser back → returns to the page.

**Postconditions:** User has seen the change history for the page without prior knowledge that the feature exists.

**Failure modes:**

- Vault has zero snapshots → metadata strip is omitted; `[history]` link is not rendered. No broken link, no empty page.
- `history` feature disabled at build time → same graceful absence.

### 3.2 Happy Path: Vault Recent Changes

**Preconditions:** User runs `zetl serve`. Vault has ≥5 snapshots spanning recent days.

**Steps:**

1. User clicks "Recent changes" link in sidebar (or footer).
2. Browser loads `/_history`.
3. Page shows: total snapshot count, oldest and newest snapshot timestamps, a small sparkline of vault-wide link count over time, and a reverse-chronological list of the last N (default 50) changed pages with their timestamps and slugs.
4. User clicks a page title → lands on that page.

**Postconditions:** User has oriented to "what has changed recently in this vault" in under 5 seconds.

**Failure modes:**

- Snapshot count is zero → page renders a single line: "No history yet. Run `zetl index` or edit a page to take the first snapshot." Never 500s.

### 3.3 Happy Path: Static Deploy Parity

**Preconditions:** User runs `zetl build --out dist/`. Vault has history.

**Steps:**

1. `dist/_history.html` exists and mirrors the `/_history` serve output at build time.
2. `dist/pages/<slug>/_history.html` exists for every page with history data.
3. User opens `dist/pages/note-a/_history.html` directly in a browser → renders identically to serve mode.

**Postconditions:** No history surface is serve-only except the explicitly mutating `/api/history/restore` and `/api/history/file-diff` endpoints.

---

## 4. Requirements

### REQ-300: Page Metadata Strip

The system SHALL render on every page (via `themes/default/page.html`) a single-line metadata strip containing `last_changed` (formatted date), `stable_days` (humanised — "3d", "2w", "9mo"), and an anchor link labelled "history" pointing to the per-page history URL, WHEN `page.history` is non-null. WHEN `page.history` is null, the strip SHALL be omitted entirely (no empty element, no placeholder text).

Trace:
- TEST-300
- CON-300

### REQ-301: Per-Page History Link Parity

The per-page history link rendered by REQ-300 SHALL resolve to `/pages/<slug>/_history` under `zetl serve` AND to `/pages/<slug>/_history.html` under `zetl build`, using the active theme's URL scheme. The link SHALL be derived from the existing `page_slug` context, not recomputed.

Trace:
- TEST-301

### REQ-302: Static Emission of Per-Page History

Under `zetl build`, the system SHALL emit `pages/<slug>/_history.html` for every page for which `history::build_template_page_history_context` returns a non-null result. Each emitted file SHALL be rendered via the existing `render_page_history` engine method, with the same template (`page_history.html`) used in serve mode.

Trace:
- TEST-302
- CON-302

### REQ-303: Vault Recent-Changes Page

The system SHALL render a vault-wide history page at `/_history` (serve) and `_history.html` (build) using a new template `vault_history.html`. The page SHALL display:

1. Snapshot count (from `vault.history.snapshot_count`).
2. Oldest and newest snapshot timestamps (ISO 8601).
3. A vault link-count trend sparkline derived from `vault.history.trend` (sampled ≤30 points).
4. A reverse-chronological list of the last N changed pages (default N=50, configurable per REQ-306) with page title, slug, and change timestamp. Each entry links to the page.

WHEN `vault.history` is null, the page SHALL render the graceful-absence body described in §3.2 failure modes.

Trace:
- TEST-303
- CON-303

### REQ-304: Sidebar or Footer Link to /_history

The default theme's `base.html` (or equivalent common layout) SHALL include a link labelled "Recent changes" (or localised equivalent) pointing to `/_history` when served and `_history.html` when built. The link SHALL be omitted when `vault.history` is null.

Trace:
- TEST-304

### REQ-305: Graceful Absence

For every history surface (metadata strip, per-page history link, vault history page link, vault history page body), the system SHALL render NO visible output (not an error, not an empty element) WHEN the backing history context is `null` or `snapshot_count == 0`. This requirement explicitly supersedes any default content.

Trace:
- TEST-305

### REQ-306: Recent-Changes List Length Configurable

The default N=50 of REQ-303 SHALL be overridable via a template variable `vault.history.recent_limit`, defaulting to 50 when absent. This variable is read-only from the template; no new CLI flag is required.

Trace:
- TEST-306

### REQ-307: Theme Override Compatibility

Every new or modified template introduced by this spec SHALL remain overridable via `.zetl/themes/<theme>/<template>.html` per the three-tier resolution in `build_env` (src/web/engine.rs:234). A theme that overrides `page.html` but not `vault_history.html` SHALL still receive the bundled `default/vault_history.html`.

Trace:
- TEST-307
- ADR-307

### REQ-308: Documentation

`README.md` SHALL gain a "History UI" section describing the two surfaces, the graceful-absence behaviour, and how to disable via `themes/<theme>/`. `CHANGELOG.md` SHALL note the new surfaces under the next minor version bump.

Trace:
- TEST-308

### NFR-300: Render Overhead

Per-page render time (serve) SHALL NOT regress by more than 3ms at the 95th percentile on a 500-page vault versus pre-change behaviour. Build-mode wall-clock for a 500-page vault SHALL NOT regress by more than 10% (p95 over 5 runs). Historical context is already computed; the only added cost is template rendering of additional strings.

Trace:
- TEST-NFR-300
- OBS-300

### NFR-301: Accessibility

The metadata strip, recent-changes list, and sparkline SHALL meet WCAG 2.2 AA per Constitutional Principle #9. Specifically: the sparkline SHALL have a text alternative (first/last values and trend direction in an aria-label); the recent-changes list SHALL use semantic `<ol>` or `<ul>` markup; date strings SHALL use `<time datetime="...">` for machine-readability.

Trace:
- TEST-NFR-301

---

## 5. Architecture Decision Records

### ADR-307: Static Emission of Per-Page History via Build Loop, Not Pre-Rendered Cache

**Context:** Under `serve`, `/pages/<slug>/_history` is rendered on demand from live jj state + cache. Under `build`, we need the same output frozen to disk. Two options:

1. **Extend the existing per-page build loop** in `src/web/build.rs` to call `render_page_history` for each page after rendering `page.html`. One extra render per page, reusing the engine already constructed.
2. **Generate history HTML from `history-index.json` in a separate post-build step.** Decouples but duplicates rendering logic (`history-index.json` is a flat projection; `page_history.html` expects the full context including vault, breadcrumbs, draft flag).

**Decision:** Option 1. Reuses `render_page_history` unchanged — the template contract is identical to serve mode, eliminating drift. The per-page overhead is bounded by `build_template_page_history_context`, which is already called per-page today for the inline `page.history` context; in build mode we extend the loop to additionally render and write the standalone page.

**Consequences:**
- Build wall-clock adds one template render per page. Bounded; see NFR-300.
- No second rendering path to maintain.
- `page_history.html` is the single source of truth for per-page history rendering.

**Alternatives rejected:**
- **Only emit history HTML on demand via a post-deploy agent.** Unworkable on CDN deploys.
- **Skip build emission; document as serve-only.** Violates the design principle of serve/build parity and would silently break users publishing statically.

### ADR-308: Vault History Page Is a First-Class Template, Not an Extension of `index.html`

**Context:** The vault recent-changes view could live on the landing page (`index.html`) or as a dedicated `/_history` page.

**Decision:** Dedicated page. The landing page is user-customisable and often replaced or overridden; embedding recent-changes there risks disappearance. A dedicated template keeps the feature addressable and linkable.

**Consequences:**
- One new template file (`vault_history.html`) added to `KNOWN_TEMPLATES` in `src/web/engine.rs`.
- Themes opt into surfacing via `base.html` (link in nav/footer) but always get the page itself.

---

## 6. Contracts

### CON-300: `PageContext` Extension (None Required)

The spec consumes the existing `PageContext.history: serde_json::Value` (populated by `history::build_template_page_history_context`; see memory entries) with fields `created_at`, `last_changed`, `age_days`, `stable_days`, `link_trend`, `recent_changes`.

No new struct fields. No new serialisation. The template layer reads these fields directly.

Implements: REQ-300, REQ-301
Verified by: TEST-300, TEST-301

### CON-302: Build Loop Extension Contract

```rust
// Pseudo-signature; real change lives in src/web/build.rs inside the per-page loop.
fn build_page_history_static(
    engine: &TemplateEngine,
    vault_ctx: &VaultContext,
    page_name: &str,
    page_slug: &str,
    breadcrumbs: &[BreadcrumbEntry],
    vault_root: &Path,
    out_dir: &Path,
) -> Result<(), BuildError>;
```

Pre-conditions:
- `history::build_template_page_history_context(page_name, vault_root)` returns `Some(_)`.
- `out_dir` exists and is writable.

Post-conditions:
- `out_dir/pages/<slug>/_history.html` exists and is non-empty.
- Content matches `engine.render_page_history(...)` output.

Error model:
- If `render_page_history` returns `TemplateError::empty_output` or `TemplateError::Minijinja`, the build SHALL propagate with a diagnostic including the page slug. Graceful absence (null history) is NOT an error — the file is simply not written.

Implements: REQ-302
Verified by: TEST-302

### CON-303: `vault_history.html` Template Context

The `vault_history.html` template SHALL be rendered with the following context (new method `TemplateEngine::render_vault_history`):

```rust
context! {
    vault => vault_ctx,           // existing VaultContext (includes vault.history)
    recent_changes => Vec<RecentChangeEntry>, // new type; see below
    sparkline_points => &[f32],   // sampled from vault.history.trend
    mode => "serve" | "build",
    search_index => /* existing */,
    theme => &self.theme,
    root_path => /* existing */,
    index_file => /* existing */,
}
```

```rust
#[derive(Serialize)]
pub struct RecentChangeEntry {
    pub page_title: String,
    pub page_slug: String,
    pub changed_at: String,       // RFC 3339
    pub change_kind: ChangeKind,  // Added | Modified | Removed
}
```

`recent_changes` is derived via a new pure function in `src/history/core.rs`:

```rust
pub fn build_recent_changes(vault_root: &Path, limit: usize) -> Vec<RecentChangeEntry>;
```

Implements: REQ-303
Verified by: TEST-303

---

## 7. Test Specifications

### TEST-300: Metadata Strip Rendered When History Present

- **Given** a vault with ≥2 snapshots of `Note A` and `history` feature enabled
- **When** a GET to `/pages/note-a/` is issued under `zetl serve`
- **Then** the response body contains the strip: a `last_changed` date, a `stable_days` humanised label, and an anchor whose `href` resolves to the per-page history URL.
- **And** when `page.history` is `null` (feature disabled or no snapshots), the strip is absent from the response body (no empty element, no placeholder text).

### TEST-301: Per-Page History Link Parity

- **Given** page `Note A` and active theme `default`
- **When** serving, the link `href` matches the route registered at `src/web/routes.rs` for per-page history (currently derived from `page_slug`).
- **When** building, the link `href` points to `pages/<slug>/_history.html` relative to `root_path`.
- **Then** both URLs resolve to non-404 content on their respective hosts.

### TEST-302: Static Emission of Per-Page History

- **Given** a vault with history and 3 pages (`A`, `B`, `C`)
- **When** `zetl build --out dist/` runs
- **Then** `dist/pages/a/_history.html`, `dist/pages/b/_history.html`, `dist/pages/c/_history.html` exist, each is non-empty, and each contains the page title in its output.
- **And** for a vault where `build_template_page_history_context` returns `None` for `page B`, `dist/pages/b/_history.html` is NOT created (graceful absence per REQ-305).

### TEST-303: Vault History Page Renders

- **Given** a vault with `vault.history.snapshot_count == 5` and trend data
- **When** GET `/_history` under serve OR opening `dist/_history.html` after build
- **Then** the response contains the snapshot count, oldest and newest timestamps, a sparkline element (inline SVG or equivalent with `aria-label`), and a list of ≤50 `<li>` entries each with a page title and a `<time>` element.

### TEST-304: Recent-Changes Link in Base Template

- **Given** a vault with non-null `vault.history`
- **When** any page is rendered
- **Then** the rendered HTML contains an anchor whose text matches `Recent changes` (or the default theme's equivalent) and whose `href` resolves to the vault-history URL.
- **And** when `vault.history` is null, the anchor is absent.

### TEST-305: Graceful Absence

For each of {metadata strip, per-page history link, vault history page link, vault history body recent-changes list}:

- **Given** the backing context is null/zero
- **When** the page renders
- **Then** no element with the associated CSS class / selector appears in the output AND the response status is 200 AND no server-side error is logged.

### TEST-306: Recent-Changes Limit

- **Given** a vault with 200 recent changes and `vault.history.recent_limit` unset
- **Then** the rendered list contains exactly 50 entries.
- **Given** `recent_limit=10` overridden via template context (test harness sets it)
- **Then** the list contains exactly 10 entries.

### TEST-307: Theme Override Respected

- **Given** a custom theme that provides `page.html` but not `vault_history.html`
- **When** rendering under that theme
- **Then** `page.html` is loaded from the theme dir AND `vault_history.html` falls back to the bundled default per the three-tier loader.

### TEST-308: README and CHANGELOG

Doc test: `README.md` contains a heading matching `History UI` or `History views`; `CHANGELOG.md` for the target version contains the substring `history UI` or `per-page history`.

### TEST-NFR-300: Render Overhead

Benchmark on a 500-page synthetic vault: per-page serve render p95 before vs after; build wall-clock p95 over 5 runs before vs after. Assertions:

- `serve_p95_after - serve_p95_before ≤ 3ms`
- `build_wallclock_p95_after ≤ 1.10 * build_wallclock_p95_before`

### TEST-NFR-301: Accessibility

Automated A11y check (axe-core or equivalent) on rendered `page.html` and `vault_history.html` fixtures: zero critical violations; semantic date elements present; sparkline has non-empty `aria-label`.

---

## 8. Observability

### OBS-300: Per-Page History Build Timing

When `--verbose` is set, build mode emits one line per page whose per-page history HTML is written:

```
[zetl] history-static: page "Note A" slug="note-a" bytes=12345 duration_ms=4
```

Reuses the existing `[zetl] history-context:` convention (see memory — OBS-013). No hot-path cost without `--verbose`.

Trace: NFR-300.

### OBS-301: Vault History Route Access

Under `serve`, the `/_history` handler SHALL emit an existing-convention log line on each request when `--verbose`:

```
[zetl] history-route: path=/_history entries=50 duration_ms=8
```

Trace: NFR-300.

---

## 9. Purity Boundary Map

### Pure Core
- `history::core::build_recent_changes(vault_root, limit) -> Vec<RecentChangeEntry>` — new; reads jj + cache via the existing pure loaders, returns ordered list. Uses injected probe per existing `core::*` conventions.
- `history::core::sample_vault_trend(points, max)` — already exists as part of SPEC-017 infra; reused.
- Template data shaping (humanising `stable_days` → "3d" / "2w" / "9mo") lives in a pure function `history::core::humanise_days(n: i64) -> String`.

### Effectful Shell
- `src/web/build.rs` per-page loop — orchestrates `render_page_history` and writes files.
- `src/web/routes.rs` `/_history` handler — orchestrates `build_recent_changes` + `render_vault_history` response.
- `src/web/engine.rs` `render_vault_history` — new render method; calls the pure context builder, passes to minijinja.

### Boundary Contracts
- `RecentChangeEntry` (pure core → shell).
- `vault_history.html` context (shell → template).

### Dependency Rule
Shell → core. New pure functions MUST NOT import `std::fs` except via the existing history loader API.

### Enforcement
`#[deny(clippy::disallowed_methods)]` on `history::core` new additions. Integration tests in `tests/history_integration.rs` verify behaviour.

---

## 10. Migration & Rollout

1. **Feature branch.** Implement template + build-loop changes behind `#[cfg(feature = "history")]` where applicable.
2. **Default theme ships updated templates.** `themes/default/page.html`, `themes/default/base.html`, and new `themes/default/vault_history.html`.
3. **Docs.** CHANGELOG entry + README section land in the same commit as the behaviour change.
4. **Version.** Minor bump (current 0.1.x → next minor). No schema change, no breaking API.
5. **Synthetic user run.** After implementation, run §3 happy paths as a synthetic-user simulation; file findings as REQ amendments.
6. **Adversarial review.** Per Constitution §12 — fresh context, cross-model preferred (Tier 3).

---

## 11. Open Questions

- **Q1.** Should the metadata strip appear above or below the page title? Proposed: **below the title, above the body content**, matching blog-post-style "byline" convention. Resolve in IMPL.
- **Q2.** What date format for `last_changed`? Proposed: **localised "March 18, 2026" with `<time datetime="2026-03-18T...">` for machine-readability**. Localisation deferred to a future spec.
- **Q3.** Should `vault_history.html` paginate past the first 50 entries? Proposed: **no** — beyond 50 entries, redirect users to `history-index.json` or the SPEC-017 CLI. Paginate in a future spec if usage demands.
- **Q4.** Should the sparkline be inline SVG or Canvas? Proposed: **inline SVG** — accessible, no JS required, prints correctly, static-deploy-friendly.
- **Q5.** Should the build loop write `pages/<slug>/history.html` even when the page's history has zero entries beyond creation? Proposed: **yes** when `build_template_page_history_context` returns `Some(_)` with ≥1 entry; **no** when `None` or zero entries. Clarified in TEST-302.

---

## 12. Traceability Matrix

| REQ     | CON     | TEST          | OBS     |
|---------|---------|---------------|---------|
| REQ-300 | CON-300 | TEST-300      | —       |
| REQ-301 | —       | TEST-301      | —       |
| REQ-302 | CON-302 | TEST-302      | OBS-300 |
| REQ-303 | CON-303 | TEST-303      | OBS-301 |
| REQ-304 | —       | TEST-304      | —       |
| REQ-305 | —       | TEST-305      | —       |
| REQ-306 | CON-303 | TEST-306      | —       |
| REQ-307 | —       | TEST-307      | —       |
| REQ-308 | —       | TEST-308      | —       |
| NFR-300 | —       | TEST-NFR-300  | OBS-300, OBS-301 |
| NFR-301 | —       | TEST-NFR-301  | —       |

---

## 13. Status

**Draft.** Awaiting human review — in particular ADR-307 (build-loop extension vs post-build step), Q1–Q5 in §11, and NFR-300 overhead budgets.
