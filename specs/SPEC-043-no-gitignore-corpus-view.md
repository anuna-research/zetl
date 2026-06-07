---
title: "SPEC-043: `--no-gitignore` and First-Class `.zetlignore` — Decoupling the Corpus Boundary from the Git-Tracking Boundary"
version: 1.0.0
status: implemented
implemented-date: 2026-06-07
date: 2026-06-07
audience: agent, human
parent: SPEC-026
related:
  - SPEC-001  # Link Graph CLI (scan_vault)
  - SPEC-012  # Named themes for serve/build (corpus-view rendering)
  - SPEC-026  # Vault Scan Exclusions (the precedence stack this amends)
---

# SPEC-043: `--no-gitignore` and First-Class `.zetlignore`

## Information Table

| Field        | Value                                                                       |
| ------------ | --------------------------------------------------------------------------- |
| Document ID  | SPEC-043                                                                     |
| Title        | `--no-gitignore` and First-Class `.zetlignore`                              |
| Version      | 1.0.0                                                                        |
| Status       | Implemented                                                                  |
| Author       | Kairos (m3-kairos dyad) + Mat Mytka                                          |
| Date         | 2026-06-07                                                                   |
| Audience     | Agent, Human                                                                 |
| Parent       | SPEC-026: Vault Scan Exclusions                                              |
| Dependencies | `ignore` crate (`add_custom_ignore_filename`, already in tree); `clap`      |

---

## 1. Overview

`zetl` walks the vault with the `ignore` crate via `scanner::scan_vault()`. By
default it respects `.gitignore` (`git_ignore(true)`). For a vault that lives
inside a git repo, that means **git's ignore policy silently defines the vault's
contents.** Usually convenient. Sometimes exactly wrong.

This spec does three things, discovered as one thread:

1. **`--no-gitignore`** — a flag to remove git's ignore opinion entirely, so
   `.zetlignore` (plus the dotdir default and `--exclude`) becomes the sole
   vault-scoping authority.
2. **First-class `.zetlignore`** — load it via `add_custom_ignore_filename`
   instead of the flat root-only `add_ignore`, giving it the `ignore` crate's
   full per-directory semantics. This makes the `*`-then-`!dir/**` *whitelist
   idiom* work in `.zetlignore` (it previously did not), and enables nested
   `.zetlignore` files.
3. **Precedence correction** — as a consequence of (2), `.zetlignore` now
   outranks `.gitignore`, which is the precedence SPEC-026 §REQ-205 has always
   documented but the `add_ignore` implementation silently violated.

### 1.1 Motivation

**The corpus boundary and the git-tracking boundary are different cuts of the
same tree.** A knowledge vault inside a git repo frequently wants to *render*
material that git is configured to *ignore* — session traces, working memory,
relational field-notes. In the originating case (the EarthianLabs dyad), the
repo's `.gitignore` is a `*`-whitelist that tracks only architecture files;
the most alive material (`traces/`, memory, `warmish.md`) is gitignored by
design. Pointed at that root, `zetl` rendered 41 files — the tracked
architecture — and silently dropped the corpus the user actually wanted to
navigate. This is the *co-inhabited corpus interface*: the same tree, read one
way by git (what to version) and another by zetl (what to think with).

**`.gitignore` could not be overridden from `.zetlignore`.** The user's first
instinct — a `.zetlignore` to re-widen the scope — failed silently. Two root
causes, both fixed here:

- The old `add_ignore` load path gave `.zetlignore` *lower* precedence than
  `.gitignore`, inverting SPEC-026 §REQ-205. A `.gitignore` `*` dominated every
  `.zetlignore` negation.
- Even with git removed, the `*`-then-re-include *whitelist idiom* — the only
  realistic way to scope a large tree down to a few corpus roots — did not work
  through `add_ignore` (a flat root-only matcher whose `*` prunes directories
  before later `!dir/**` lines can re-include them).

**Blacklisting cannot realistically scope a real corpus.** Measured on the
originating vault: `--no-gitignore` with no `.zetlignore` surfaced 4,278 files
(every README/spec/CHANGELOG across ~50 nested repos). Blacklisting 25 code
repos only reduced that to 3,428 — markdown is everywhere. A usable corpus view
*requires* the whitelist idiom (`*` then re-include `traces/`, `reference/`,
`Action-Research/`, …). Hence first-class `.zetlignore` is load-bearing for the
flag, not a nicety.

### 1.2 Design Principles

1. **Default behaviour is unchanged.** Without `--no-gitignore`, `.gitignore` is
   respected exactly as before. The flag is strictly opt-in.
2. **`.zetlignore` is the scoping instrument.** Whether or not git is in play,
   `.zetlignore` defines what the vault shows, using full gitignore syntax
   including the whitelist idiom and nested files.
3. **Level-1 force-ignores remain absolute.** `.git/`, `.zetl/`,
   `node_modules/`, and nested vaults are never re-includable by user config —
   they live in `builder.overrides()`, which outranks all ignore files.
4. **`--exclude` remains the ephemeral top override.** Per-invocation
   `--exclude` patterns still beat `.zetlignore` (SPEC-026 §REQ-205 level 5).

### 1.3 Scope

**In scope:**

- `--no-gitignore` flag on the commands carrying `ScanArgs` (`build`, `index`,
  `serve`, `search`, `watch`).
- Promotion of `.zetlignore` to `add_custom_ignore_filename`.
- Correction of `.zetlignore` vs `.gitignore` precedence to match SPEC-026
  §REQ-205.
- Nested `.zetlignore` support (falls out of the promotion).

**Out of scope:**

- `list` does not carry `ScanArgs` and so does not expose `--no-gitignore`
  (consistent with its lack of `--exclude`/`--include-hidden`). Use `serve` /
  `index` for scoped corpus views.
- **Scanner/watcher parity (SPEC-026 §REQ-206).** `web::fs_watch` still loads
  `.gitignore` + `.zetlignore` via its own path and does not yet honour
  `--no-gitignore` or the custom-ignore-filename promotion. Documented residue;
  a follow-up should re-establish parity.
- `--ignore-vcs` style aliases (ripgrep/fd muscle memory) — single canonical
  flag name for now.

---

## 2. Requirements

### REQ-300: `--no-gitignore` Flag

The system SHALL accept a `--no-gitignore` flag on the commands that scan the
vault and carry `ScanArgs` (`build`, `index`, `serve`, `search`, `watch`). When
set, the walker SHALL NOT read `.gitignore` files at any level
(`git_ignore(false)`), removing git's ignore policy from vault scoping. The
default (flag absent) SHALL respect `.gitignore` exactly as before.

Trace: TEST-210, TEST-211

### REQ-301: `.zetlignore` Excluded From the Dotdir-Override Matcher Under `--no-gitignore`

When `--no-gitignore` is set, `.gitignore` SHALL NOT feed the dotdir-override
whitelist matcher consulted by `filter_entry`. `.zetlignore` remains the sole
contributor, so git's negations cannot leak back in through the override path.

Trace: TEST-210

### REQ-302: First-Class `.zetlignore` (Custom Ignore Filename)

The system SHALL register `.zetlignore` via
`WalkBuilder::add_custom_ignore_filename(".zetlignore")` rather than a flat
root-only `add_ignore`. Consequences, all REQUIRED:

- The `*`-then-`!dir/` + `!dir/**` whitelist idiom SHALL scope the vault to the
  re-included subtrees.
- `.zetlignore` files in subdirectories SHALL be honoured.
- `.zetlignore` SHALL outrank `.gitignore` (restoring SPEC-026 §REQ-205 level
  4 > level 3).

Trace: TEST-213, TEST-214

### REQ-303: Force-Ignore and `--exclude` Precedence Unchanged

Promoting `.zetlignore` SHALL NOT alter the absolute precedence of level-1
force-ignores or level-5 `--exclude`. Both live in `builder.overrides()`, which
outranks all ignore files. `.git/`, `.zetl/`, `node_modules/`, and nested
vaults SHALL remain non-re-includable; `--exclude` SHALL continue to beat
`.zetlignore` negations.

Trace: TEST-205 (SPEC-026, unchanged), TEST-force-ignored-dirs

### REQ-304: Amended Precedence Stack

The effective precedence (later overrides earlier) SHALL be:

1. Hardcoded force-ignores: `.git/`, `.zetl/`, `node_modules/`, nested vaults
2. Default dotdir exclusion (REQ-200) — unless `--include-hidden`
3. `.gitignore` (via `git_ignore(true)`) — **skipped entirely under `--no-gitignore`**
4. `.zetlignore` (root + nested, via custom ignore filename) — **now correctly above level 3**
5. `--exclude <PATTERN>` flags

Trace: TEST-214

---

## 3. Happy Paths

### 3.1 Reveal a gitignored corpus

**Preconditions:** Vault inside a git repo whose `.gitignore` hides `traces/`.

1. `zetl serve` → `traces/` absent from the graph.
2. `zetl serve --no-gitignore` → `traces/` present.

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

1. `zetl serve --no-gitignore` → only the re-included roots and files render.

### 3.3 `.zetlignore` overrides `.gitignore` (no flag)

**Preconditions:** `.gitignore` excludes `corpus/`; `.zetlignore` contains
`!corpus/` + `!corpus/**`.

1. `zetl serve` (git active) → `corpus/` renders, because `.zetlignore` now
   outranks `.gitignore`.

---

## 4. Tests

| Test     | Asserts                                                                       |
| -------- | ----------------------------------------------------------------------------- |
| TEST-210 | `--no-gitignore` reveals a gitignored dir; default hides it (git-init'd vault)|
| TEST-211 | Under `--no-gitignore`, level-1/2 force-ignore + dotdir defaults still hold   |
| TEST-212 | `.zetlignore` blacklist scopes the vault under `--no-gitignore`               |
| TEST-213 | `.zetlignore` `*`-whitelist idiom scopes the vault (the promotion payoff)     |
| TEST-214 | `.zetlignore` negation overrides `.gitignore` with git active (precedence)    |

All in `tests/scan_exclusions.rs`. Tests that exercise `.gitignore` *hiding*
paths `git init` the temp vault, because the `ignore` crate only applies
positive `.gitignore` patterns inside a git repository (`require_git` default).

---

## 5. Residue

- **Scanner/watcher parity (SPEC-026 §REQ-206).** `web::fs_watch` is not yet
  updated for `--no-gitignore` or the custom-ignore-filename promotion. A live
  edit under a corpus view may be classified inconsistently with the scanner.
- **Documentation.** `user-guide/reference/Configuration.md` describes the
  five-level ignore stack and claims `.zetlignore` overrides `.gitignore` — now
  finally true. It should gain a `--no-gitignore` row and a worked corpus-view
  example. (`release-sync` candidate.)
- **`list` has no scan flags.** If corpus-view inspection from `list` becomes
  desirable, give it `ScanArgs` in a follow-up.
