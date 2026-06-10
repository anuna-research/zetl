---
title: "SPEC-043: First-Class `.zetlignore` — Decoupling the Corpus Boundary from the Git-Tracking Boundary"
version: 2.0.0
status: implemented
implemented-date: 2026-06-10
date: 2026-06-07
audience: agent, human
parent: SPEC-026
related:
  - SPEC-001  # Link Graph CLI (scan_vault)
  - SPEC-012  # Named themes for serve/build (corpus-view rendering)
  - SPEC-026  # Vault Scan Exclusions (the precedence stack this amends)
---

# SPEC-043: First-Class `.zetlignore`

## Information Table

| Field        | Value                                                                       |
| ------------ | --------------------------------------------------------------------------- |
| Document ID  | SPEC-043                                                                     |
| Title        | First-Class `.zetlignore` — Corpus Boundary ≠ Git-Tracking Boundary        |
| Version      | 2.0.0                                                                        |
| Status       | Implemented                                                                  |
| Author       | Kairos (m3-kairos dyad) + Mat Mytka                                          |
| Date         | 2026-06-07 (v1: flag; 2026-06-10 v2: always-off)                            |
| Audience     | Agent, Human                                                                 |
| Parent       | SPEC-026: Vault Scan Exclusions                                              |
| Dependencies | `ignore` crate (`add_custom_ignore_filename`, already in tree)               |

---

## 1. Overview

`zetl` walks the vault with the `ignore` crate via `scanner::scan_vault()`.
`.gitignore` is **never consulted** — `git_ignore(false)` is hardcoded.
`.zetlignore` is the sole file-based vault-scoping authority.

This spec does two things, discovered as one thread:

1. **First-class `.zetlignore`** — loaded via `add_custom_ignore_filename`
   instead of the flat root-only `add_ignore`, giving it the `ignore` crate's
   full per-directory semantics. The `*`-then-`!dir/**` *whitelist idiom* works
   in `.zetlignore`, and nested `.zetlignore` files are honoured.
2. **`.gitignore` disabled** — git's ignore opinion is removed entirely.
   The corpus boundary and the git-tracking boundary are independent.

### 1.1 Motivation

**The corpus boundary and the git-tracking boundary are different cuts of the
same tree.** A knowledge vault inside a git repo frequently wants to *render*
material that git is configured to *ignore* — session traces, working memory,
relational field-notes. In the originating case (the EarthianLabs dyad), the
repo's `.gitignore` is a `*`-whitelist that tracks only architecture files;
the most alive material (`traces/`, memory, `warmish.md`) is gitignored by
design. Pointed at that root, `zetl` rendered 41 files — the tracked
architecture — and silently dropped the corpus the user actually wanted to
navigate.

**`.gitignore` could not be overridden from `.zetlignore`.** Two root causes,
both now fixed:

- The old `add_ignore` load path gave `.zetlignore` *lower* precedence than
  `.gitignore`, inverting SPEC-026 §REQ-205. A `.gitignore` `*` dominated every
  `.zetlignore` negation.
- Even with git removed, the `*`-then-re-include *whitelist idiom* — the only
  realistic way to scope a large tree down to a few corpus roots — did not work
  through `add_ignore` (a flat root-only matcher whose `*` prunes directories
  before later `!dir/**` lines can re-include them).

**Blacklisting cannot realistically scope a real corpus.** Measured on the
originating vault: with no `.zetlignore`, 4,278 files surfaced (every
README/spec/CHANGELOG across ~50 nested repos). Blacklisting 25 code repos
only reduced that to 3,428 — markdown is everywhere. A usable corpus view
*requires* the whitelist idiom (`*` then re-include `traces/`, `reference/`,
`Action-Research/`, …). Hence first-class `.zetlignore` is load-bearing, not
a nicety.

### 1.2 Design Principles

1. **`.zetlignore` is the scoping instrument.** `.zetlignore` defines what
   the vault shows, using full gitignore syntax including the whitelist idiom
   and nested files. `.gitignore` is not involved at any level.
2. **Level-1 force-ignores remain absolute.** `.git/`, `.zetl/`,
   `node_modules/`, and nested vaults are never re-includable by user config —
   they live in `builder.overrides()`, which outranks all ignore files.
3. **`--exclude` remains the ephemeral top override.** Per-invocation
   `--exclude` patterns still beat `.zetlignore` (SPEC-026 §REQ-205 level 4).

### 1.3 Scope

**In scope:**

- Hardcoding `git_ignore(false)` in `scanner::scan_vault`.
- Promotion of `.zetlignore` to `add_custom_ignore_filename`.
- Nested `.zetlignore` support (falls out of the promotion).
- Removal of the (v1) `--no-gitignore` flag and `ScanOptions::no_gitignore`
  field.

**Out of scope:**

- `list` does not carry `ScanArgs` (consistent with its lack of
  `--exclude`/`--include-hidden`). Use `serve` / `index` for scoped corpus
  views.

---

## 2. Requirements

### REQ-300: `.gitignore` Is Never Consulted

The system SHALL set `git_ignore(false)` on `WalkBuilder` unconditionally.
`.gitignore` files at any depth SHALL NOT influence vault scoping.

Trace: TEST-210, TEST-211, TEST-214

### REQ-302: First-Class `.zetlignore` (Custom Ignore Filename)

The system SHALL register `.zetlignore` via
`WalkBuilder::add_custom_ignore_filename(".zetlignore")` rather than a flat
root-only `add_ignore`. Consequences, all REQUIRED:

- The `*`-then-`!dir/` + `!dir/**` whitelist idiom SHALL scope the vault to the
  re-included subtrees.
- `.zetlignore` files in subdirectories SHALL be honoured.

Trace: TEST-213

### REQ-303: Force-Ignore and `--exclude` Precedence Unchanged

Promoting `.zetlignore` SHALL NOT alter the absolute precedence of level-1
force-ignores or level-4 `--exclude`. Both live in `builder.overrides()`, which
outranks all ignore files. `.git/`, `.zetl/`, `node_modules/`, and nested
vaults SHALL remain non-re-includable; `--exclude` SHALL continue to beat
`.zetlignore` negations.

Trace: TEST-205 (SPEC-026, unchanged), TEST-force-ignored-dirs

### REQ-304: Amended Precedence Stack

The effective precedence (later overrides earlier) SHALL be:

1. Hardcoded force-ignores: `.git/`, `.zetl/`, `node_modules/`, nested vaults
2. Default dotdir exclusion (REQ-200) — unless `--include-hidden`
3. `.zetlignore` (root + nested, via custom ignore filename)
4. `--exclude <PATTERN>` flags

`.gitignore` does not appear in this stack.

Trace: TEST-213

---

## 3. Happy Paths

### 3.1 Render a gitignored corpus

**Preconditions:** Vault inside a git repo whose `.gitignore` hides `traces/`.

1. `zetl serve` → `traces/` present (`.gitignore` is disregarded).

### 3.2 Scope a large tree to a few corpus roots (whitelist idiom)

**Preconditions:** Vault root with dozens of subdirectories, most of them code.
A `.zetlignore`:

```gitignore
*
!traces/
!traces/**
!reference/
!reference/**
!Action-Research/
!Action-Research/**
!kairos.md
```

1. `zetl serve` → only the re-included roots and files render.

---

## 4. Tests

| Test     | Asserts                                                                       |
| -------- | ----------------------------------------------------------------------------- |
| TEST-210 | `.gitignore` exclusions are always ignored; gitignored dirs are visible       |
| TEST-211 | `.gitignore` wildcard has no effect; levels 1–2 still hold                   |
| TEST-212 | `.zetlignore` blacklist scopes the vault                                      |
| TEST-213 | `.zetlignore` `*`-whitelist idiom scopes the vault (the promotion payoff)     |
| TEST-214 | `.gitignore` exclusions are disregarded even in a git-init'd vault            |

All in `tests/scan_exclusions.rs`. Tests that verify `.gitignore` is really
ignored `git init` the temp vault, because the `ignore` crate only reads
`.gitignore` inside a git repository by default — otherwise the test would be
vacuous (no gitignore would apply regardless).

---

## 5. Migration from v1 (`--no-gitignore` flag)

v1 (2026-06-07) shipped a `--no-gitignore` flag and left `.gitignore` respected
by default. v2 (2026-06-10) makes the v1 flag's behaviour the unconditional
default and removes the flag.

**Migration:** remove `--no-gitignore` from any scripts or aliases. The
behaviour you opted into is now the default.

If you depended on `.gitignore` scoping your zetl vault (e.g. a `*`-whitelist
`.gitignore` was silently keeping the vault small), move those patterns to
`.zetlignore`.
