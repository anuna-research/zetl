---
title: "SPEC-030: Theme Data Contract — External-Facing Template Context"
version: 0.1.0
status: draft
date: 2026-04-18
audience: agent, human
related:
  - SPEC-027  # History UI (page.history consumer)
  - SPEC-028  # Graph UI (_graph partial consumer)
  - SPEC-020  # Multi-user collab (editor.html consumer)
---

# SPEC-030: Theme Data Contract — External-Facing Template Context

## Information Table

| Field          | Value                                                                       |
| -------------- | --------------------------------------------------------------------------- |
| Document ID    | SPEC-030                                                                    |
| Title          | Theme Data Contract — External-Facing Template Context                      |
| Version        | 0.1.0                                                                       |
| Status         | Draft                                                                       |
| Author         | Agent (USDD Protocol v1.3.0)                                                |
| Date           | 2026-04-18                                                                  |
| Audience       | Agent, Human (theme authors, Rust maintainers, integrators)                 |
| Trace          | USDD Agent Protocol v1.3.0                                                  |
| Parent         | —                                                                           |
| Related        | SPEC-027 History UI, SPEC-028 Graph UI, SPEC-020 Multi-User Editing         |
| Dependencies   | minijinja (template engine); `serde` (context serialisation)                |

---

## 1. Overview

zetl renders every user-facing page by hydrating a set of Rust-side context structs — `VaultContext`, `PageContext`, `PageHistoryContext`, `FolderContext`, `TagCloudContext` — and handing them to minijinja via `serde`. The theme layer (`themes/default/*.html` plus three bundled siblings and an unknown number of third-party overrides under `.zetl/themes/<theme>/`) reads fields from that hydrated context to produce HTML.

**The contract between the two sides has historically been implicit.** Fields are accessed freely from templates; names, optionality, and types are inferred by reading `src/web/context.rs`. Two recent defects traced directly to this gap:

1. **SPEC-028 Graph UI merge** — `themes/default/base.html` linked to `./_graph.html` in a string that a test matched via `contains()`; when the `_graph.html` static-build emission landed, the same literal string appeared in a JS comment inside `base.html` and silently passed the `!serve_html.contains("_graph.html")` assertion until the comment's wording was changed. (See CI failure recovered by commit `e742e69`.)
2. **plan-author-attribution co-author byline** — `PageHistoryEntry` gained a new field (`co_authors: Vec<(String, String)>`); the template was updated in the same series, but in a future world where only the Rust side changes and the template isn't updated, a reader on a stale theme would see the byline silently omit the co-author — a trust-eroding failure mode invisible in CI.

SPEC-030 specifies the contract explicitly and adds machine-checkable enforcement so every future field addition, removal, or rename fails the build when it breaks a theme, rather than degrading silently at runtime.

### 1.1 Motivation

- **Anti-Slop Bias (Protocol §Constitutional).** Template code is an "artifact accepted on surface quality" today — it appears to work because missing fields render as empty strings. The contract test enumerates failure modes explicitly.
- **Third-party theme viability.** `themes/default/` is one of four bundled themes (`docs`, `fountain`, `minimal`). zetl already supports disk-level theme overrides at `.zetl/themes/<name>/`. Without a contract, every zetl upgrade silently breaks downstream themes whose authors cannot read the Rust source.
- **Traceability (Protocol §Traceability).** REQ → CON → TEST → CODE must hold at the theme boundary. Today there is no CON for the context; templates reference fields that are not documented anywhere except as `pub` fields on a struct.
- **Integration-First Testing (Protocol §Constitutional).** A realistic fixture exercising every `render_*` entrypoint in strict mode is the cheapest realistic test we can write for this surface.

### 1.2 Design Principles

1. **The context is a versioned contract, not an implementation detail.** `VaultContext`, `PageContext`, `PageHistoryContext`, `FolderContext`, `TagCloudContext`, and their transitively-serialised children are stable fields. Breaking changes require a minor-version bump in the zetl crate (`Cargo.toml`) and a migration note in `CHANGELOG.md`.
2. **Additive evolution is always safe.** New fields SHALL be additive: templates that do not reference a newly-added field continue to render identically. Strict-undefined mode only errors on field *reads*, so an unread new field is invisible to existing themes.
3. **Field removal or rename is a breaking change.** A removed field is a silent break for any theme that still reads it. The contract test catches this for the default theme at CI time; third-party theme authors MUST be warned via CHANGELOG.
4. **Optional fields are always present, possibly as null.** A field that is semantically optional (e.g. `page.history` when the `history` feature is off) MUST still serialise (as JSON `null` / minijinja `none`) rather than be absent from the map. Templates may then test `{% if page.history %}` safely. Strict-undefined mode treats `none` as falsy but a missing key as an error.
5. **Feature-gated fields require a stub.** When a field is only computed under a Cargo feature, the Rust side MUST still emit a safe default (empty string, `null`, empty list) when the feature is off — never an absent key. Example: `humanise_days` filter is registered unconditionally so every theme can call it.
6. **Verification is executable, not documentary.** The contract test SHALL render every user-facing template with a fully-populated context under minijinja's strict-undefined mode. Prose documentation alone is insufficient.

### 1.3 Scope

**In scope:**

- A formal contract (CON-400..CON-404) specifying the fields, types, optionality, and semantic meaning of every field the default theme's templates read from Rust-side context structs.
- A CI-gated test (TEST-400) that renders every user-facing template path under minijinja's strict-undefined mode against a representative fixture. Failures block merge.
- An enforcement mechanism (`TemplateEngine::new_strict`, already landed on branch `fix/misc`) that enables strict-undefined rendering in tests without affecting production renders.
- Evolution policy: how fields are added, renamed, or removed, and what version-bumping discipline applies.
- Documentation updates in `README.md` ("Writing a theme") and `CHANGELOG.md` (contract-version trailer).

**Out of scope:**

- Auto-generated JSON Schema export (ADR-402 rationale) — deferred to a successor spec if third-party theme authorship takes off.
- Runtime validation of template-rendered HTML (e.g. "every `<a>` has an href"); this is a separate class of check.
- Contract for the WebSocket wire format (`ClientMsg`, `ServerMsg`) — covered by SPEC-020.
- Contract for HTTP routes and JSON API shapes (`/api/*` endpoints) — out of scope; covered separately where relevant.
- Non-default themes (`docs`, `fountain`, `minimal`, third-party) are NOT included in TEST-400 in v1.0; they inherit the contract but opt into verification individually (REQ-406).
- Cryptographic or tamper-evidence guarantees on the context JSON — context is computed server-side per render and not signed.

---

## 2. User Profiles

### 2.1 Default-Theme Developer

Role: zetl maintainer working on the Rust side of the template pipeline.
Goals: add, rename, or remove a context field and know immediately whether any bundled theme breaks.
Constraints: works in-tree; has access to `cargo test` locally; expects CI to catch what they miss.
Daily workflow: edit `src/web/context.rs` or a `render_*` method → run `cargo test --features history` → see green → merge.

### 2.2 Third-Party Theme Author

Role: zetl user with a custom theme under `.zetl/themes/<name>/` derived from the default.
Goals: track upstream zetl releases and update their theme when the contract changes, without having to read Rust source.
Constraints: knows HTML, CSS, and minijinja; does not read Rust; relies on documentation and CHANGELOG.
Daily workflow: upgrade zetl → skim CHANGELOG → if contract changed, patch their theme → run `zetl build --strict-theme` (future) or `cargo test` against a vendored harness.

### 2.3 Template Reviewer (Human or AI Agent)

Role: a second pair of eyes on a template change, per Protocol §Adversarial Review.
Goals: judge whether a template modification is safe against the documented contract.
Constraints: has the contract document (this spec) and the diff; does not have to read Rust source.
Daily workflow: open the PR → read the CON section → diff the template against the CON → flag any field the template reads that the CON does not list.

---

## 3. Happy Paths

### 3.1 Happy Path: Additive Field Addition

**Preconditions:** Developer wants to add `vault.stats.median_link_count` to enable a new template widget.

**Steps:**

1. Developer adds `pub median_link_count: usize` to `StatsContext` in `src/web/context.rs`.
2. Developer runs `cargo test --features history`.
3. TEST-400 continues to pass — no template reads the new field yet, so strict-undefined rendering is unaffected.
4. Developer writes or updates a template to render `{{ vault.stats.median_link_count }}`.
5. Developer adds a fixture value to the `rich_vault()` helper in `theme_contract_*` tests.
6. Developer re-runs `cargo test` — still green.
7. Developer adds a CHANGELOG entry: `theme-contract: +vault.stats.median_link_count (additive)`.
8. PR merges.

**Postconditions:** The field is part of the contract; third-party themes not referencing it are unaffected; the one theme that reads it is tested.

**Failure modes:**

- Forgetting step 5 → TEST-400 either fails (if the template references the field but the fixture lacks it) or is uninformative (field is not exercised in the strict render). The fixture-coverage audit (REQ-404) surfaces the latter.
- Adding a non-optional field that existing themes will never provide data for → acceptable only when the field is cheaply computed from vault state; otherwise ADR-400 applies.

### 3.2 Happy Path: Field Rename (Breaking)

**Preconditions:** Developer wants to rename `page.stable_days` to `page.days_since_change` for clarity.

**Steps:**

1. Developer renames the field in `src/history/core.rs` (`PageHistoryContext`).
2. Developer runs `cargo test --features history`.
3. TEST-400 fails on `render_page` because `page.html` still reads `page.history.stable_days` — strict-undefined mode reports the exact template line and field name.
4. Developer updates every template reference (in bundled themes and the bundled-other themes).
5. Developer re-runs tests — green.
6. Developer opens `CHANGELOG.md` and records: `**Breaking (theme contract)**: page.history.stable_days → page.history.days_since_change. Migration: replace all occurrences in your theme.` plus a version bump.
7. PR merges with the version bump.

**Postconditions:** The rename is atomic across Rust + bundled themes; downstream theme authors have documented migration steps.

**Failure modes:**

- Skipping step 6 → third-party themes break silently at their reader's next zetl upgrade. No CI catches this; only the CHANGELOG discipline does. This is the strongest argument for ADR-401 (semver-aware theme-contract versioning).
- Partial rename (missed a template) → TEST-400 catches in the default theme; test coverage for other bundled themes (REQ-406) catches them.

### 3.3 Happy Path: Field Removal

**Preconditions:** Developer is removing the feature behind `vault.semantic_available` (SPEC for semantic search is deprecated).

**Steps:**

1. Developer removes the field from `VaultContext`.
2. TEST-400 fails if any template still conditions on it (`{% if vault.semantic_available %}`).
3. Developer removes the conditional block from the default theme.
4. Developer removes the fixture population of that field (test cleanup).
5. CHANGELOG: `**Breaking (theme contract)**: vault.semantic_available removed; any `{% if vault.semantic_available %}` branches should be deleted.`
6. PR merges.

**Postconditions:** Field is gone from the contract; no template references it; readers have a migration note.

**Failure modes:** Same as 3.2.

---

## 4. Requirements

### REQ-400: Strict-Undefined Template Rendering Test

The system SHALL render every user-facing template via its public `render_*` entry point under minijinja's `UndefinedBehavior::Strict` mode WITHIN the `cargo test` pipeline FOR every merge into main WITH zero rendering errors.

**Templates covered (v1.0):** `index.html`, `page.html`, `folder.html`, `vault_graph.html`, `help.html`, `tag_cloud.html`, `editor.html`, and — under `--features history` — `page_history.html` and `vault_history.html`.

**Fixture requirement:** The test SHALL use a fixture (`rich_vault()`, `rich_page()`) in which every optional field of every context struct is populated with a non-null, well-formed value, so optional branches that exist only when data is present are exercised.

Trace:
- TEST-400
- CON-400..CON-404
- CODE: `src/web/engine.rs` `theme_contract_all_user_facing_templates_render_cleanly`

### REQ-401: Optional Fields Serialise as Null, Not Absent

Every optional field in the template context SHALL be serialised as JSON `null` (minijinja `none`) WHEN no value is available FOR every render path WITH no exceptions. Fields MUST NOT be silently omitted from the serialised map.

**Rationale:** Strict-undefined mode distinguishes `none` (explicit null) from undefined (missing key). Templates writing `{% if page.history %}` work with the former; they fail with the latter.

Trace:
- TEST-400
- CON-400..CON-404

### REQ-402: Feature-Gated Fields Have Off-Feature Stubs

Every template filter or context field gated behind a Cargo feature SHALL be available (as a safe stub) WHEN the feature is disabled FOR every render path WITH semantics matching the on-feature "empty" case.

**Rationale:** `humanise_days` is registered in `crate::history::core` behind `--features history`. A template that calls `{{ page.history.stable_days | humanise_days }}` within a `{% if page.history %}` guard would fail even in strict mode without the feature unless the filter itself is always registered. REQ-402 codifies the stub-always-register pattern.

**Reference implementation:** `src/web/engine.rs:humanise_days` filter (registered in both `#[cfg(feature = "history")]` and `#[cfg(not(feature = "history"))]` branches).

Trace:
- TEST-400 (runs against both feature flavours in CI)
- CON-404

### REQ-403: Theme-Contract Version Discipline

The `zetl` crate Cargo version SHALL be bumped AT MINOR LEVEL OR HIGHER WHENEVER a field is renamed, removed, or changes type in any of the context structs specified by CON-400..CON-404. Additive changes (new fields, new template filters) MAY ship in a patch version.

**Verification:** Reviewer checks `Cargo.toml` version against the nature of the change during PR review. CI does not yet enforce this automatically; ADR-401 discusses a future `cargo-semver-checks` integration.

Trace:
- CHANGELOG.md + `Cargo.toml` version

### REQ-404: Fixture Coverage Audit

The strict-render fixture SHALL populate every field of every context struct specified in CON-400..CON-404 WITH a non-null representative value EXCEPT where the field's type is inherently empty (e.g. `Vec<X>` fixtures MAY be empty when iteration is exercised elsewhere).

**Rationale:** A template that reads `page.backlinks[0].count` fails strict mode if `page.backlinks` is empty. Fixture coverage ensures the strict render exercises every reachable attribute chain.

**Verification:** Manual audit during PR review of the fixture; future automation via a struct-walker that asserts every `pub` field is referenced in the fixture.

Trace:
- TEST-400

### REQ-405: CHANGELOG Entry Format

Every contract change SHALL carry a CHANGELOG entry formatted as:

```
### Theme contract
- **Additive:** <field path> (<reason>)
- **Breaking:** <old> → <new> (migration: <what the theme author must do>)
- **Removed:** <field path> (migration: <...>)
```

Additive changes MAY skip the CHANGELOG when the field is reserved / not yet consumed by any template.

Trace:
- Documentation (CHANGELOG.md)

### REQ-406: Bundled-Theme Contract Parity

Every bundled theme (`docs`, `fountain`, `minimal`) SHOULD pass TEST-400 against its own `render_*` surface WHEN the theme is explicitly opted in VIA a per-theme contract test.

**v1.0 scope:** Only the `default` theme is covered. Bundled-theme parity is a follow-on: each theme's maintainer adds a `theme_contract_<theme>_strict` test mirroring the default one.

**Non-default themes that fail this requirement** ship at the theme author's risk; a warning appears in `zetl --help` / theme listing ("untested against data contract").

Trace:
- TEST-401 (placeholder: bundled-theme strict renders; not yet implemented)

### REQ-407: Contract Documentation in README

The default-theme README (`themes/default/README.md`, to be created) SHALL link to this spec as the authoritative source for the context shape, AND SHALL enumerate every context struct WITH a pointer to its CON-### in this document.

**Rationale:** A theme author reading their theme directory should find the contract without clicking into the Rust source.

Trace:
- Documentation

---

## 5. Non-Functional Requirements

### NFR-400: Strict-Render Latency

TEST-400 SHALL complete in ≤ 2 seconds WHEN run on developer-class hardware (Apple M-series, Linux x86_64 @ 3GHz+) WITH 95th-percentile confidence over 20 runs.

**Rationale:** The strict-undefined test runs on every `cargo test` invocation. Slow tests get disabled. A fixture-vault of one page and two context structs should not exceed microseconds per render; the guardrail is generous.

Trace:
- TEST-400 (wall-clock assertion in CI)
- OBS-400 (test runtime logged in CI output)

### NFR-401: Contract Stability Window

Breaking changes to CON-400..CON-404 SHALL occur at most once per minor release cycle (nominally 1-3 months) UNLESS driven by a security or correctness concern that cannot wait.

**Rationale:** Theme authors need a predictable upgrade cadence. Monthly breaking changes kill third-party adoption.

Trace:
- Policy (enforced in review)

---

## 6. Architecture Decision Records

### ADR-400: Strict-Undefined Rendering vs. JSON Schema Export

**Context:** Two enforcement mechanisms exist for the theme contract: (a) render every template in strict mode against a fixture, or (b) derive a JSON Schema from the context structs and validate templates against the schema.

**Decision:** Ship (a) in v1.0. Defer (b) to a successor spec.

**Rationale:**

- (a) catches the actual failure mode — a template reading a non-existent field — with zero new dependencies. It runs in the existing test pipeline. It requires no new build step and produces human-readable errors tied to specific template lines.
- (b) requires a `schemars` or equivalent dependency, a schema-export build step, a validator (e.g. a template linter that understands minijinja), and CI to diff the generated schema against a checked-in artefact. The payoff materialises only when there are third-party theme authors reading the schema; we have none today.
- (a) also verifies more than field presence — it catches type mismatches (iterating a string), filter misuse (calling `humanise_days` on a string), and conditional-evaluation errors that a pure-schema check cannot.

**Consequences:**

- Templates must be renderable in isolation with a fixture. Templates that depend on vault-wide state (e.g. sibling pages' backlink counts) are already handled this way via `VaultContext`.
- Third-party theme authors don't get a schema file to read; they read this spec and the CON sections.
- Should third-party adoption grow, a follow-on spec can add (b) on top without removing (a).

**Trace:** TEST-400 (implements the decision)

### ADR-401: Semver Policy for Theme Contract

**Context:** When is a field rename a major, minor, or patch change? The `zetl` crate's `Cargo.toml` version has historically bumped by developer intuition.

**Decision:** Theme-contract changes track the crate version with the following rules, codified in REQ-403:

- **Additive** (new field, new filter, widened type): patch bump is sufficient.
- **Breaking** (renamed, removed, or type-narrowed field): minimum minor bump (e.g. `0.3.x → 0.4.0`).
- **Security / correctness emergency**: patch bump is acceptable with a prominent CHANGELOG note.

**Rationale:** Pre-1.0, semver is looser by convention. A minor bump for breaking changes gives theme authors a clear signal ("expect to review your theme") without blocking on 1.0.

**Consequences:**

- CHANGELOG becomes the canonical migration log.
- Future `cargo-semver-checks` integration (ADR-402) can automate detection of REQ-403 violations.

**Trace:** REQ-403, REQ-405

### ADR-402: No Auto-Generated Schema in v1.0

**Context:** We could export a JSON Schema at build time and commit it to the tree, then CI-diff against regenerated schema on every build.

**Decision:** Not in v1.0.

**Rationale:**

- `schemars` would add ~200 KB of derives across context types for an output no current consumer reads.
- Strict-undefined rendering already catches the class of bugs a schema would catch (missing field references).
- A schema export would help third-party theme authors — but we have zero data on how many exist or whether they'd consume a schema.

**Consequences:** Revisit when (1) a third-party theme author requests it, (2) zetl gains a second programmatic consumer of the context (e.g. a non-minijinja template engine).

**Trace:** Deferred to successor spec.

### ADR-403: Context Keys Are Always Present, Optional Fields Carry Null

**Context:** `PageContext.history: serde_json::Value` serialises as `null` when the `history` feature is off. Alternative: use `Option<PageHistoryContext>` and rely on serde's `skip_serializing_if` to omit the key.

**Decision:** Always serialise the key. Use `Null` for unavailable.

**Rationale:**

- Strict-undefined mode treats a missing key as an error, but `none` as falsy. `{% if page.history %}` is the idiomatic template pattern; omitting the key breaks it under strict mode.
- The serialised JSON is slightly larger but the overhead is negligible.
- A template upgrade that learns to read `page.history.new_field` does not also have to learn that `page.history` might be absent.

**Consequences:** REQ-401 codifies the pattern. All new context fields MUST follow it.

**Trace:** REQ-401

---

## 7. Contracts

Each CON below specifies one Rust-side context struct, its serialised field set, optionality, and the user-facing template paths that consume it. The field types are given in Rust notation for precision; minijinja receives them as serde-serialised equivalents.

### CON-400: `VaultContext` — Site-Wide Data

**Defined in:** `src/web/context.rs::VaultContext`

**Consumed by:** every `render_*` entry point; accessed as `{{ vault.* }}` in any template that `{% extends "base.html" %}`.

**Fields:**

| Name                | Type                       | Optionality | Semantics                                                                            |
| ------------------- | -------------------------- | ----------- | ------------------------------------------------------------------------------------ |
| `name`              | `String`                   | required    | Vault display name, derived from the vault directory basename                        |
| `pages`             | `Vec<PageEntry>`           | required    | Every indexable page. MAY be empty                                                   |
| `sidebar_tree`      | `Vec<FolderEntry>`         | required    | Hierarchical sidebar. MAY be empty                                                   |
| `stats`             | `StatsContext`             | required    | `total_pages`, `total_links`, `dead_links`, `orphans` — all `usize`                  |
| `history`           | `serde_json::Value`        | required    | `null` if `history` feature off or vault has no snapshots; otherwise object per CON-403 |
| `semantic_available`| `bool`                     | required    | True when `semantic` feature + vector index are both active                          |
| `site_url`          | `String`                   | required    | Canonical site URL when set via `zetl build --site-url`; empty string otherwise      |

**Implements:** REQ-400, REQ-401

**Verified by:** TEST-400 (via `rich_vault()` fixture)

### CON-401: `PageContext` — Single-Page Data

**Defined in:** `src/web/context.rs::PageContext`

**Consumed by:** `render_page`, `render_editor`, `render_page_history`; accessed as `{{ page.* }}`.

**Fields:**

| Name                 | Type                       | Optionality | Semantics                                                                 |
| -------------------- | -------------------------- | ----------- | ------------------------------------------------------------------------- |
| `title`              | `String`                   | required    | Page title, from first H1 or frontmatter                                  |
| `slug`               | `String`                   | required    | URL-safe slug derived from file path                                      |
| `content_html`       | `String`                   | required    | Rendered markdown body (safe HTML)                                        |
| `content_raw`        | `String`                   | required    | Original markdown source                                                  |
| `frontmatter`        | `serde_json::Value`        | required    | Parsed frontmatter as object; `{}` when absent                            |
| `description`        | `String`                   | required    | Meta-description; empty when none derivable                               |
| `backlinks`          | `Vec<BacklinkEntry>`       | required    | Pages linking to this one; MAY be empty. See CON-402                      |
| `outlinks`           | `Vec<OutlinkEntry>`        | required    | Wikilinks from this page; MAY be empty                                    |
| `breadcrumbs`        | `Vec<BreadcrumbEntry>`     | required    | Folder breadcrumbs; MAY be empty                                          |
| `transclusion_cards` | `String`                   | required    | Pre-rendered HTML for the linked-pages rail; empty when no transclusions  |
| `is_new`             | `bool`                     | required    | True when page has no on-disk backing file yet (new-page UI)              |
| `raw_escaped`        | `Option<String>`           | optional    | HTML-escaped source for the editor textarea; `null` when view-only        |
| `history`            | `serde_json::Value`        | required    | `null` when feature off or no snapshots; otherwise object per CON-403     |

**Implements:** REQ-400, REQ-401

**Verified by:** TEST-400 (via `rich_page()` fixture)

### CON-402: `BacklinkEntry` — Reverse-Link Data

**Defined in:** `src/web/context.rs::BacklinkEntry`

**Consumed by:** `page.html` backlinks section; accessed as `{{ bl.* }}` within a `{% for bl in page.backlinks %}`.

**Fields:**

| Name    | Type             | Optionality | Semantics                                                                      |
| ------- | ---------------- | ----------- | ------------------------------------------------------------------------------ |
| `title` | `String`         | required    | Display title of the source page                                               |
| `slug`  | `String`         | required    | URL-safe slug of the source page                                               |
| `line`  | `usize`          | required    | 1-based source line number where the wikilink appears                          |
| `count` | `usize`          | required    | Number of times the source page links here (dedup count per SPEC-030-related)  |
| `since` | `Option<String>` | optional    | RFC 3339 timestamp of earliest snapshot with this backlink; `null` when history unavailable |

**Implements:** REQ-400, REQ-401

**Verified by:** TEST-400

### CON-403: `PageHistoryContext` — Per-Page Temporal Data

**Defined in:** `src/history/core.rs::PageHistoryContext` (feature `history`)

**Consumed by:** `page.html` byline, `page_history.html`; accessed as `{{ page.history.* }}`.

**Serialised shape:**

```json
{
  "created_at":    "RFC 3339 timestamp (String)",
  "last_changed":  "RFC 3339 timestamp (String)",
  "age_days":      "i64 (days since created_at)",
  "stable_days":   "i64 (days since last_changed)",
  "link_trend":    [{"ts": "...", "forward": <i64>, "backlink": <i64>}],
  "recent_changes": [
    {
      "change_id":     "jj change id prefix (String)",
      "timestamp":     "RFC 3339 timestamp (String)",
      "author_name":   "String",
      "author_email":  "String",
      "co_authors":    [["Name (String)", "email (String)"], ...],
      "link_count":    "usize",
      "backlink_count":"usize",
      "is_orphan":     "bool",
      "delta":         null | { ...PageNeighborhoodDelta... }
    }
  ]
}
```

**Optionality:** The entire object is `null` when the `history` feature is off OR the page has no snapshots. When present, every listed field is guaranteed.

**Implements:** REQ-400, REQ-401, REQ-402

**Verified by:** TEST-400 (via `rich_page_history()` fixture), TEST-300 (SPEC-027)

**Note:** `recent_changes` is newest-first, capped at 5 entries. `co_authors` is empty for solo commits, populated by the `Co-authored-by:` trailer parser (see plan-author-attribution).

### CON-404: Template Globals and Filters

**Defined in:** `src/web/engine.rs::build_env_with_strictness`

**Consumed by:** every template.

**Globals injected into every render:**

| Name              | Type     | Semantics                                                                               |
| ----------------- | -------- | --------------------------------------------------------------------------------------- |
| `graph_placement` | `String` | `"docked"`, `"tabs"`, `"stacked"`, or `"fullscreen"`; from theme `theme.toml`           |
| `spa_enabled`     | `bool`   | `true` when theme opts into SPA navigation via `theme.toml [spa] enabled`               |

**Per-render context additions (every `render_*` method MUST provide all of these):**

| Name               | Type     | Semantics                                                                              |
| ------------------ | -------- | -------------------------------------------------------------------------------------- |
| `mode`             | `String` | `"serve"` or `"build"`                                                                 |
| `search_index`     | `String` | JSON string for the ⌘K palette; empty in build mode                                    |
| `bm25_index`       | `String` | BM25 index pre-rendered JSON; empty when not available                                 |
| `history_index`    | `String` | JSON for history-index; empty when feature off                                         |
| `theme`            | `String` | Active theme name                                                                      |
| `active_slug`      | `String` | Slug of the currently-rendering page (empty on folder/landing/graph)                   |
| `root_path`        | `String` | Relative path prefix to vault root (handles subpath deployments)                       |
| `index_file`       | `String` | `"index.html"` in build, empty in serve                                                |
| `graph_index_url`  | `String` | Relative URL for `graph-index.json`                                                    |
| `graph_index`      | `String` | Inline JSON when `graph_inline=true` in theme; empty otherwise                         |

**Filters:**

| Name            | Signature                      | Semantics                                                                    |
| --------------- | ------------------------------ | ---------------------------------------------------------------------------- |
| `tojson`        | `fn(Value) -> String`          | Serialises any serde value to a JSON literal (escape + quote)                |
| `humanise_days` | `fn(i64) -> String`            | `"3d"`, `"2w"`, `"9mo"`. Stub when `--features history` off: returns `"{n}d"` |
| `safe`          | (minijinja built-in)           | Mark HTML as trusted (do not escape)                                         |

**Implements:** REQ-400, REQ-401, REQ-402

**Verified by:** TEST-400

---

## 8. Test Specifications

### TEST-400: Strict-Undefined Rendering of Every User-Facing Template

**Scope:** Unit test `theme_contract_all_user_facing_templates_render_cleanly` in `src/web/engine.rs`.

**Setup:**
- Build a `TemplateEngine` via `new_strict`, which enables `UndefinedBehavior::Strict` on the minijinja environment.
- Populate `rich_vault()`, `rich_page()`, `rich_page_history()`, `rich_vault_history()` with non-null values for every field defined in CON-400..CON-404.

**Actions:**
- Call each public `render_*` method in both `"serve"` and `"build"` modes (where applicable).
- Under `#[cfg(feature = "history")]`, additionally call `render_page_history` and `render_vault_history`.

**Assertions:**
- Every `render_*` returns `Ok(_)`. Any `Err` produced by minijinja (undefined variable, unknown filter, type mismatch) fails the test with the template path and line number surfaced in the panic message.

**Traceability:**
- Verifies: REQ-400, REQ-401, REQ-402
- Implemented in: `src/web/engine.rs` (commit `963c557`)
- Also proves: CON-400..CON-404 field shapes are consumed correctly by the default theme

### TEST-401: Bundled-Theme Contract Parity (Placeholder)

**Scope:** Per-theme strict-render tests for `docs`, `fountain`, `minimal`.

**Status:** Not yet implemented. Blocked on (a) extending `TemplateEngine::new_strict` to accept a theme name override and (b) each theme having complete templates for every render entry point (some bundled themes currently rely on the default's fallback).

**When implemented:** Each bundled theme gets its own test that mirrors TEST-400 against its own `render_*` surface.

**Traceability:**
- Verifies: REQ-406
- Implementation path: follow-on spec or direct PR

### TEST-402: Fixture Coverage Audit (Manual)

**Scope:** Quarterly manual review that every field in CON-400..CON-404 is populated in the strict-render fixture.

**Actions:**
- Developer walks every `pub` field in `VaultContext`, `PageContext`, `PageHistoryContext`, `FolderContext`, `TagCloudContext`.
- For each, verifies the corresponding `rich_*()` helper sets a non-null, representative value.
- Missing coverage is filed as a BUG against this spec and patched in the fixture.

**Traceability:**
- Verifies: REQ-404
- Automation path: a struct-walker using `serde`'s reflection or a proc-macro could detect uncovered fields; tracked as a future enhancement in a successor spec.

### TEST-403: Feature-Flag Parity (Regression)

**Scope:** TEST-400 SHALL run in CI under both `cargo test` (default features) and `cargo test --features history`.

**Assertions:** Both runs pass.

**Rationale:** Some fields (e.g. `page.history`, `vault.history`) only carry data under `--features history`. The strict-undefined test exercises the "null" branch under default features and the "populated" branch under the feature; both MUST render cleanly.

**Traceability:**
- Verifies: REQ-402
- Implementation: CI matrix (GitHub Actions / Woodpecker pipeline)

---

## 9. Observability

### OBS-400: Test Runtime Telemetry

TEST-400's wall-clock time SHALL be logged in CI output AND SHALL be tracked per-commit so NFR-400's 2-second ceiling is visible to reviewers.

**Mechanism:** `cargo test` already reports per-test timings. CI aggregation (if any) captures the `theme_contract_*` row.

**Trace:** NFR-400

### OBS-401: Contract-Change Commit Audit

When CHANGELOG records a `**Breaking (theme contract):**` entry, the commit message SHALL explicitly reference `SPEC-030` AND a migration note per REQ-405.

**Rationale:** Downstream theme authors watching the commit log get a machine-greppable signal (`grep 'SPEC-030'` in `git log`) for contract changes.

**Trace:** REQ-405

---

## 10. Open Questions

- **Q1 (ADR-402):** When should auto-generated JSON Schema be revisited? Trigger: the first external bug report referencing "the theme contract" or the second third-party theme surfacing.
- **Q2 (REQ-406):** Should the bundled `docs`, `fountain`, `minimal` themes gain strict-render tests now, or wait until one of them diverges from the default enough to warrant the investment?
- **Q3 (ADR-403):** `serde_json::Value` is a loose type for `page.history` / `vault.history` — a stronger-typed `Option<PageHistoryContext>` would give better Rust-side ergonomics. Worth revisiting after a few releases of field stability.

---

## 11. References

- USDD Agent Protocol v1.3.0 (`../handbook/engineering/usdd-agent-protocol.md`)
- SPEC-027: History UI — Per-Page and Per-Vault History Surfaces (`page.history` consumer)
- SPEC-028: Interactive Graph View (`_graph.html` partial consumer)
- SPEC-020: Multi-User Collaborative Editing (`editor.html` consumer)
- plan-author-attribution (`.hence/plan-author-attribution.spl`) — added `co_authors` to `PageHistoryEntry`; the extension that motivated this spec.
- Commit `963c557`: first implementation of TEST-400 against CON-400..CON-404 in branch `fix/misc`.
