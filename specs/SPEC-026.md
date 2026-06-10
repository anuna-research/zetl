---
title: "SPEC-026: Vault Scan Exclusions — Dotdir Defaults, .zetlignore, and --exclude"
version: 0.1.0
status: draft
date: 2026-04-15
audience: agent, human
parent: SPEC-001
related:
  - SPEC-002
  - SPEC-008
  - SPEC-013
---

# SPEC-026: Vault Scan Exclusions — Dotdir Defaults, `.zetlignore`, and `--exclude`

## Information Table

| Field          | Value                                                                     |
| -------------- | ------------------------------------------------------------------------- |
| Document ID    | SPEC-026                                                                  |
| Title          | Vault Scan Exclusions — Dotdir Defaults, `.zetlignore`, and `--exclude`   |
| Version        | 0.1.0                                                                     |
| Status         | Draft                                                                     |
| Author         | Agent (USDD Protocol v1.3.0)                                              |
| Date           | 2026-04-15                                                                |
| Audience       | Agent, Human                                                              |
| Trace          | USDD Agent Protocol v1.3.0                                                |
| Parent         | SPEC-001: Link Graph CLI                                                  |
| Related        | SPEC-002 Full-Text Search, SPEC-008 Watch Mode, SPEC-013 Tantivy Search   |
| Dependencies   | `ignore` crate (already in tree); `clap` CLI plumbing                     |

---

## 1. Overview

`zetl` walks the vault with the `ignore` crate via `scanner::scan_vault()` (src/scanner.rs:20). The walker currently:

- Does **not** consult `.gitignore` (`git_ignore(false)` — see SPEC-043)
- Loads `.zetlignore` at vault root and in subdirectories (first-class custom ignore filename)
- Force-ignores `.git/`, `node_modules/`, `.zetl/` via override rules
- **Does not** set `hidden(true)` — comment says "user may have .files as notes"

The consequence: any dotdir that is not one of the three hardcoded names is walked and its markdown is compiled into `dist/`. Real-world leak: `.claude/` (Claude Code session data), `.obsidian/`, `.vscode/`, `.cache/`, `.venv/`, `.terraform/`. Downstream builds ship them to production.

A parallel code path — `web::fs_watch::classify_external_event` (src/web/fs_watch.rs:599) — already filters *all* dotdirs by prefix (`starts_with('.')`). Scanner and watcher disagree. This spec aligns them and exposes the behaviour to users.

### 1.1 Motivation

**The dist leak is self-ironic.** `zetl build` publishes a knowledge graph; publishing your AI agent's scratchpad is both a privacy leak and a correctness problem — wikilinks inside `.claude/` session logs pollute backlink counts, the search index, and the link graph.

**Users already expect gitignore semantics.** Every comparable static-site or vault tool (Hugo, Jekyll, 11ty, Obsidian publish, Zola) skips dotdirs by default. The principle of least astonishment applies.

**The escape hatch already exists.** `scan_vault` has a `ignore_patterns: &[String]` parameter, plumbed through but never populated. All twelve call sites pass `&[]`. The plumbing is half-built.

### 1.2 Design Principles

1. **Skip dotdirs by default.** Dotdirs at any depth are excluded from scan unless explicitly allowed. Dotfiles at the vault root may still be walked (preserves `.zetlignore`, user-authored `.hidden-note.md`) — the exclusion is on **directories** whose name starts with `.`.
2. **`.zetlignore` is the sole file-based scoping authority.** Gitignore-syntax file at vault root and in subdirectories (SPEC-043). `.gitignore` is never consulted — the corpus boundary and the git-tracking boundary are independent.
3. **`--exclude` is the ephemeral override.** Per-invocation gitignore-style patterns for one-off builds.
4. **Scanner ≡ watcher.** Both code paths apply the same exclusion policy. No silent divergence.
6. **Backward compatible at the CLI surface.** Users relying on default behaviour today (walking `.claude/`) get a one-line migration: add `.claude/` to `.zetlignore`… wait, that's already the required behaviour. The breaking change IS the fix. Change gated behind minor version bump + CHANGELOG note.

### 1.3 Scope

**In scope:**

- Default exclusion of all directories whose name begins with `.` at any depth (except explicit allow list)
- `--exclude <PATTERN>` CLI flag on `zetl build`, `zetl index`, `zetl serve`, `zetl search`, `zetl watch` (repeatable)
- `--include-hidden` opt-out flag to restore pre-change behaviour
- Documentation of `.zetlignore` syntax and precedence
- Alignment of `scanner::scan_vault` and `web::fs_watch::classify_external_event`
- Traceability: all `scan_vault(root, &[])` call sites audited — exclusions must be propagated from CLI, not dropped silently

**Out of scope:**

- `.zetlignore` in subdirectories (only vault-root file is honoured; defer nested support to a future spec)
- `zetl ignore` subcommand to inspect effective ignore rules (future)
- Negated patterns in `--exclude` (gitignore supports `!foo`; we allow the syntax but document `.zetlignore` as the canonical place for complex rules)
- Changes to `copy_static_assets` in src/web/build.rs (it only walks `.zetl/static/` — not affected)
- `--public` overlay directory (by design copies everything; user's choice)

---

## 2. User Profiles

### 2.1 Agent-Using Knowledge Worker (carries over from SPEC-001)

Additional context for this spec: keeps vault inside a project directory alongside `.claude/`, `.vscode/`, or similar tool state. Expects "build" to mean "publish my notes", not "publish my tool state".

### 2.2 DevOps / Deploy Operator

Runs `zetl build` in CI, copies `dist/` to a CDN. Cannot manually scrub output. Needs the tool to produce a clean `dist/` on first run. Evidence this is real: the user has a `rebuild.sh` that manually removes `.claude` and `.zetl` from `dist/` after every build. That script is the spec-gap — this SPEC closes it.

---

## 3. Happy Paths

### 3.1 Happy Path: Default build after upgrade

**Preconditions:** Vault root contains `notes/`, `.claude/`, `.zetl/`, `.git/`. No `.zetlignore` file.

**Steps:**

1. User runs `zetl build` → `dist/` contains pages from `notes/` only.
2. `dist/.claude/` does not exist.
3. `dist/.zetl/` does not exist.
4. Build log reports `N pages` where N matches `find notes -name '*.md' | wc -l`.

**Postconditions:** No tool-state dotdirs in `dist/`. User's scrub script is no-op.

**Failure modes:** User has a legitimate dotdir (e.g., `.archive/`) they want published → they use `--include-hidden` OR add `!.archive/` to a `.zetlignore` (see REQ-003).

### 3.2 Happy Path: Vault with intentional dotdir

**Preconditions:** User stores archived notes in `.archive/` and wants them published.

**Steps:**

1. User creates `.zetlignore` with content `!.archive/`.
2. User runs `zetl build`.
3. `dist/.archive/` is populated; `dist/.claude/` still absent.

**Postconditions:** Explicit inclusion overrides dotdir default.

### 3.3 Happy Path: One-off exclusion from CLI

**Preconditions:** Vault contains a `drafts/` dir the user usually publishes but wants omitted today.

**Steps:**

1. User runs `zetl build --exclude 'drafts/'`.
2. `dist/drafts/` absent.
3. Subsequent `zetl build` (no flag) publishes `drafts/` again.

**Postconditions:** `--exclude` is ephemeral; no state persists.

---

## 4. Requirements

### REQ-200: Default Exclusion of Dotdirs

The system SHALL exclude from vault scanning any directory whose basename begins with `.` at any depth within the vault, UNLESS the directory is explicitly allowed via `.zetlignore`, `--exclude` negation, or `--include-hidden`.

Trace:
- TEST-200
- CON-200

### REQ-201: Dotfile Scanning Preserved

The system SHALL continue to scan dotfiles (files whose basename begins with `.`) at the vault root and inside allowed directories — only **directories** are excluded by the default dotdir rule.

Rationale: preserves the original comment's intent ("user may have .files as notes") for actual markdown notes, while closing the directory leak.

Trace:
- TEST-201

### REQ-202: `.zetlignore` First-Class Support

The system SHALL read a `.zetlignore` file at the vault root if present, interpreting its contents as gitignore-syntax patterns evaluated relative to the vault root. Negated patterns (`!pattern`) SHALL override the default dotdir exclusion (REQ-200).

Trace:
- TEST-202
- CON-202

### REQ-203: `--exclude` CLI Flag

The system SHALL accept a repeatable `--exclude <PATTERN>` flag on commands that scan the vault (`build`, `index`, `serve`, `search`, `watch`). Patterns use gitignore syntax, evaluated relative to the vault root, and combine with `.zetlignore` and the default dotdir rule via the precedence in REQ-205.

Trace:
- TEST-203
- CON-203

### REQ-204: `--include-hidden` Opt-Out

The system SHALL accept an `--include-hidden` flag on the same commands as REQ-203. When set, the default dotdir exclusion (REQ-200) is disabled. `.zetlignore` and `--exclude` still apply.

Trace:
- TEST-204

### REQ-205: Exclusion Precedence

The system SHALL apply exclusion rules in the following precedence (later rules override earlier):

1. Hardcoded force-ignores: `.git/`, `.zetl/`, `node_modules/`, nested vaults (dirs containing their own `.zetl/`)
2. Default dotdir exclusion (REQ-200) — unless `--include-hidden`
3. `.zetlignore` (vault root + nested, via `add_custom_ignore_filename` — SPEC-043)
4. `--exclude <PATTERN>` flags (repeatable, last wins on conflict)

`.gitignore` is never consulted (`git_ignore(false)` — SPEC-043 REQ-300).

The hardcoded force-ignores at level 1 SHALL NOT be overridable by user configuration — `.zetl/` must never be in `dist/`.

Trace:
- TEST-205
- ADR-205

### REQ-206: Scanner / Watcher Parity

The system SHALL apply identical exclusion rules in `scanner::scan_vault` (src/scanner.rs:20) and `web::fs_watch::classify_external_event` (src/web/fs_watch.rs:599). A file change ignored by the scanner SHALL also be ignored by the watcher, and vice versa.

Trace:
- TEST-206

### REQ-207: Call-Site Audit

Every call site of `scan_vault(..., &[])` SHALL be updated to propagate user-supplied exclusion patterns from the CLI layer, not silently pass an empty slice. Where a call site runs from a non-user context (e.g., test fixtures), that SHALL be documented at the call site.

Trace:
- TEST-207

### REQ-208: CHANGELOG and Documentation

The system SHALL document the behaviour change in `CHANGELOG.md` under a minor version bump, explicitly calling out that dotdirs are now skipped by default and the migration path (`--include-hidden` or `.zetlignore !pattern`). `README.md` SHALL gain a "Vault scanning and ignore files" section.

Trace:
- TEST-208

### NFR-200: No Scan Regression

Vault scan wall-clock time on a 1000-page vault SHALL NOT regress by more than 5% versus pre-change behaviour at the 95th percentile across 10 runs.

Trace:
- TEST-NFR-200
- OBS-200

---

## 5. Architecture Decision Records

### ADR-205: Dotdir Exclusion is Default On

**Context:** Current behaviour walks all dotdirs except three hardcoded names. This leaks tool state into `dist/`. Three options considered:

1. **Leave default off, require opt-in via flag.** (status quo + --exclude) — Does nothing about the leak. Rejected.
2. **Default on, opt-out via `--include-hidden`.** — Aligned with Hugo/Jekyll/Zola convention. Breaking change but small migration path. **Chosen.**
3. **Default on with a warning on first detected dotdir, require explicit opt-in or opt-out after warning.** — Too noisy for CI. Rejected.

**Decision:** Option 2. Ship behind a minor version bump. CHANGELOG entry describes migration.

**Consequences:**
- Breaking change for vaults that intentionally publish a dotdir (assessed: rare; zero known instances in the demo-vault or zetl-vault fixtures).
- Closes a privacy/correctness bug that required a shell scrub script to work around.
- Eliminates the scanner/watcher divergence.

**Alternatives considered and rejected:**
- Making the hardcoded list extensible via config — does not scale; users already expect gitignore syntax, which we already honour.
- Adding `.claude/` to the hardcoded list — only fixes the reported symptom, not the class of bug.

---

## 6. Contracts

### CON-200: `scanner::scan_vault` Signature Extension

```rust
pub struct ScanOptions {
    pub exclude_patterns: Vec<String>,
    pub include_hidden: bool,
}

pub fn scan_vault(root: &Path, opts: &ScanOptions) -> Result<Vec<ParsedFile>>;
```

Pre-conditions:
- `root` exists and is a directory.
- Patterns in `exclude_patterns` are valid gitignore syntax (validated on construction or surfaced as a scan-time error).

Post-conditions:
- Returned `ParsedFile`s include only files permitted by the rule stack in REQ-205.
- No file under a dotdir (unless `include_hidden` or whitelisted) appears in the result.

Error model:
- Invalid glob pattern → `Err` with a diagnostic naming the offending pattern.

Implements: REQ-200, REQ-202, REQ-203, REQ-204, REQ-205
Verified by: TEST-200, TEST-202, TEST-203, TEST-204, TEST-205

**Migration note for the `&[String]` → `&ScanOptions` change:** provide a `ScanOptions::default()` and an `impl From<&[String]> for ScanOptions` to minimise churn in tests. Or retain the `&[String]` overload as a thin wrapper. Decision deferred to IMPL.

### CON-203: CLI Flag Shape

Flags added to `build`, `index`, `serve`, `search`, `watch`:

```
--exclude <PATTERN>       Gitignore-syntax pattern. Repeatable. Combines with .zetlignore.
--include-hidden          Disable the default dotdir exclusion.
```

Implements: REQ-203, REQ-204
Verified by: TEST-203, TEST-204

---

## 7. Test Specifications

### TEST-200: Dotdir Excluded by Default

- **Given** a vault with `notes/a.md` and `.claude/session.md`
- **When** `zetl build` runs with no flags
- **Then** `dist/notes/a/index.html` exists AND `dist/.claude/` does not exist AND no page with slug derived from `.claude/session` appears.

### TEST-201: Dotfile at Root Still Walked

- **Given** a vault root with `.hidden-note.md` (not inside a dotdir)
- **When** `zetl build` runs
- **Then** `.hidden-note.md` is scanned (or documented behaviour: only files without leading dot are published — clarify during implementation and pin here).

### TEST-202: `.zetlignore` Negation Overrides Default

- **Given** a vault with `.archive/old.md` and a `.zetlignore` containing `!.archive/`
- **When** `zetl build` runs
- **Then** the `old` page appears in `dist/`.

### TEST-203: `--exclude` Pattern Honoured

- **Given** a vault with `drafts/d.md`
- **When** `zetl build --exclude 'drafts/'` runs
- **Then** `dist/drafts/` does not exist. A subsequent run without the flag publishes it again.

### TEST-204: `--include-hidden` Restores Walk

- **Given** a vault with `.claude/x.md`
- **When** `zetl build --include-hidden` runs
- **Then** `.claude/x` is published. And: `.zetl/` is STILL absent (REQ-205 level 1 unaffected).

### TEST-205: Precedence End-to-End

- **Given** a vault with `.foo/a.md`, `.zetlignore` containing `!.foo/`, and CLI flag `--exclude '.foo/'`
- **When** build runs
- **Then** `.foo/a` is NOT published (CLI overrides `.zetlignore`).

### TEST-206: Scanner / Watcher Parity (Property Test)

For a generated vault state + arbitrary ignore config: `scanner::scan_vault(root, opts)` returns a set of paths S; a synthetic batch of filesystem events for every file in the vault, filtered by the watcher's classifier under the same `opts`, produces the same set S. Violations are shrunk to a minimal reproducer.

### TEST-207: All Call Sites Audited

Grep-level check in CI: `scan_vault(.*&\[\])` appears only in test modules (`#[cfg(test)]` gated) or in files annotated with a `// SCAN-OPTS: intentional` marker. The check fails otherwise.

### TEST-208: README and CHANGELOG

Markdown lint / doc test: `CHANGELOG.md` for the target version contains the substring "dotdir"; `README.md` contains a heading "Vault scanning" or equivalent.

### TEST-NFR-200: Scan Performance

Benchmark on a 1000-page synthetic vault: mean wall-clock of `scan_vault` before and after. Assertion: `after_p95 ≤ 1.05 * before_p95`.

---

## 8. Observability

### OBS-200: Ignore-Decision Tracing

When `--verbose` is set, `scan_vault` emits one line per skipped top-level path: `[zetl] scan: skipped <path> reason=<dotdir|zetlignore|cli-exclude|hardcoded>`. Useful for debugging unexpected omissions. (`gitignore` is not a possible reason — `.gitignore` is never consulted.)

Trace: NFR-200 (does not fire on hot path without `--verbose`).

---

## 9. Purity Boundary Map

### Pure Core
- Pattern matching: `ignore::gitignore::Gitignore::matched` (already pure; used unchanged).
- Precedence resolution: a new pure function `resolve_exclusion(path, opts, fs_probe_for_nested_vault) -> Decision` that composes the five levels.

### Effectful Shell
- `scan_vault`: orchestrates walker, reads files, returns `ParsedFile`.
- CLI parsing in `src/cli.rs` and dispatch in `src/main.rs`.
- Watcher in `src/web/fs_watch.rs`.

### Boundary Contracts
- `ScanOptions` (CLI → scanner).
- `Decision { Include, Exclude(Reason) }` (pure core → shell, used for OBS-200 logging).

### Dependency Rule
Shell → core. The pure resolver must not read the filesystem except through an injected probe closure (for nested-vault detection).

### Enforcement
`#[deny(clippy::disallowed_methods)]` on the pure module to block `std::fs` imports. Integration test in TEST-206 verifies parity.

---

## 10. Migration & Rollout

1. **Feature branch.** Add `ScanOptions`, update `scan_vault` and all call sites.
2. **Dual-read window.** None needed — behaviour change is purely additive exclusions; no data format change.
3. **Docs.** CHANGELOG entry + README section land in the same commit as the behaviour change.
4. **Version.** Bump minor (e.g., 0.1.3 → 0.2.0). Current minor is 0.1.x (CHANGELOG verification required).
5. **Synthetic user run.** After implementation, run SPEC-026 §3 happy paths as a synthetic-user simulation; file findings as REQ amendments.

---

## 11. Open Questions

- **Q1.** Should `--include-hidden` disable level-1 force-ignores (`.git/`, `.zetl/`, `node_modules/`)? Proposed answer: **no** — level 1 is non-negotiable.
- **Q2.** Should `.zetlignore` be honoured when located in subdirectories (gitignore-style nested files)? Proposed answer: **defer** to future spec.
- **Q3.** Should we emit a one-time warning on first build when a dotdir is silently skipped (to smooth migration)? Proposed answer: **yes**, gated by a config flag, default off in CI.

---

## 12. Traceability Matrix

| REQ    | CON    | TEST    | OBS    |
|--------|--------|---------|--------|
| REQ-200 | CON-200 | TEST-200 | OBS-200 |
| REQ-201 | —      | TEST-201 | —      |
| REQ-202 | CON-200 | TEST-202 | OBS-200 |
| REQ-203 | CON-203 | TEST-203 | OBS-200 |
| REQ-204 | CON-203 | TEST-204 | OBS-200 |
| REQ-205 | CON-200 | TEST-205 | OBS-200 |
| REQ-206 | —      | TEST-206 | —      |
| REQ-207 | —      | TEST-207 | —      |
| REQ-208 | —      | TEST-208 | —      |
| NFR-200 | —      | TEST-NFR-200 | OBS-200 |

---

## 13. Status

**Draft.** Awaiting human review — in particular ADR-205 (breaking change acceptance), Q1–Q3 in §11, and the exact precedence semantics in REQ-205.
