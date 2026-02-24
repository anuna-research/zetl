---
title: "SPEC-007: zetl diff — Git-Backed Graph Diff"
version: 0.3.0
status: draft
audience: agent, human
date: 2026-02-24
---

# SPEC-007: zetl diff — Git-Backed Graph Diff

## Information Table

| Field        | Value                                                              |
| ------------ | ------------------------------------------------------------------ |
| Document ID  | SPEC-007                                                           |
| Title        | zetl diff — Git-Backed Graph Diff                                  |
| Version      | 0.3.0                                                              |
| Status       | Draft                                                              |
| Author       | Agent (USDD Protocol v1.0.0)                                       |
| Date         | 2026-02-24                                                         |
| Audience     | Agent, Human                                                       |
| Trace        | USDD Agent Protocol v1.0.0                                         |
| Parent       | SPEC-001: zetl — Bi-directional Link Graph CLI                     |
| Related      | SPEC-005: zetl reason                                              |
| Dependencies | git (runtime), pulldown-cmark (Markdown parsing), SPEC-006 (index cache format) |

---

## 1. Overview

Every zetl command operates on the current vault state. This is sufficient for point-in-time queries — "what links to this page right now?" — but tells you nothing about how the vault has evolved. When did this page become an orphan? Which connections were added this week? What changed in the knowledge base since the last agent cycle?

Git already tracks the complete history of vault files. What git cannot tell you is how those file changes translated into **graph-level changes**: a line added to `Architecture.md` may have introduced a new backlink that resolved another page's orphan status, or a dead link that needs a corresponding note. Git diffs files; it cannot diff the derived link graph.

`zetl diff` bridges this gap. Given any git reference as a baseline — a commit SHA, a branch name, a tag, or a date — zetl reconstructs the graph at that point from git history and computes a graph-level diff against the current state. No zetl-specific snapshot storage is required: git is the source of truth.

### 1.1 How Reconstruction Works

The key insight is that only files that *changed* between the baseline and the current state can affect the graph diff. If a file's content is identical at both points, its links are identical. Therefore:

1. `git diff --name-only <ref>` identifies which `.md` files changed
2. For each changed file: `git show <ref>:<path>` reads the old content
3. Old content is parsed for wikilinks using the same scanner as `zetl index`
4. The old graph is assembled from: current graph for *unchanged* files + re-parsed old content for *changed* files + old files that have since been deleted
5. Set differences between old and current graph yield the diff

This is efficient: for a 2,000-note vault where 12 files changed since yesterday, only 12 files are re-parsed from git history.

**Reconstruction scope.** Step 3 extracts only **wikilinks** from old file content — the same extraction `zetl index` performs for link-graph construction. The full SPEC-006 pipeline (Merkle leaf construction, SPL block extraction, section grounding) is not run on the baseline state; those structures are not needed to compute a link-graph diff. SPL-level changes between refs are out of scope for `zetl diff` (see §1.2 and §11.2).

**Current state.** The "current state" is the working tree as indexed by `zetl index`. `git diff --name-only <ref>` compares `<ref>` against the working tree, so files with uncommitted edits will appear in the diff. See CON-021 for details.

### 1.2 Scope

**In scope:**

- `zetl diff` command with `--from <git-ref>` and `--since <date>` baseline selection
- Graph diff output: pages added/removed, links added/removed, orphans gained/resolved, dead links added/resolved
- `--filter` to narrow output to one change category
- `--format json` and `--format table` output

**Out of scope:**

- Vaults not tracked in git (zetl diff requires git; the command errors gracefully when git is absent)
- zetl-managed snapshot storage (explicitly rejected; see ADR-011)
- SPL-level changes: added/removed facts, rules, or SPL blocks between refs (future SPEC; see §11.2; `zetl reason diff` is the intended surface for this)
- Diffing reasoning conclusions across git history (future SPEC, builds on SPEC-005 + SPEC-007)
- Page-level lifecycle timeline across all commits (future; requires scanning many commits)

**VCS dependency scope.** `zetl diff` is the only zetl command that requires git. All other commands — `zetl index`, `zetl check`, `zetl blocks`, `zetl reason` — are VCS-independent and operate identically whether or not the vault is inside a git repository (SPEC-006 §1.6, NFR-017).

---

## 2. User Profiles

### 2.1 User Profile: Akiko — Personal Knowledge Worker

```
Name:        Akiko
Role:        Product manager; maintains a 2,000-note Zettelkasten for research synthesis
Goals:       Understand how her vault changes over time; spot pages that became disconnected;
             review what she added in a work session
Constraints: Comfortable with CLI; understands git at a basic level (commits, branches);
             vault is a git repository
Workflow:    Writes notes during research sessions; commits periodically;
             runs zetl check after sessions to find issues
Pain point:  "I committed three times today. I want to see what the net graph change was,
             not read a git log."
```

### 2.2 User Profile: Dev Agent — Automated Reasoning Cycle

```
Name:        Dev Agent (AI agent in CI or local automation)
Role:        Agentic memory consumer; queries zetl on a schedule or on git push
Goals:       Determine exactly which pages changed in the graph since the last run;
             avoid re-processing unchanged content
Constraints: Non-interactive; must receive structured JSON; vault is a git repository;
             agent stores the last-processed commit SHA
Workflow:    1. Run zetl diff --from <last-sha> --format json
             2. Inspect changed pages, new orphans, new dead links
             3. Re-reason only over affected subgraph
             4. Store current HEAD SHA for next cycle
Pain point:  "Without zetl diff, I re-process 2,000 pages every cycle."
```

---

## 3. Happy Paths

### 3.1 Happy Path: Session Review (Akiko)

```
Preconditions:
  - Vault is a git repository
  - Akiko made commits during a research session; HEAD is the latest

Steps:
  1. zetl diff --since yesterday --format table
     → Shows: 3 pages added, 11 links added, 1 orphan gained, 0 dead links added

  2. zetl diff --since yesterday --filter orphans --format table
     → Lists the one page that gained orphan status: "Redis Latency Spike Investigation"

  3. Akiko links it from her "Performance Issues" index page and commits.
     zetl diff --from HEAD~1 --format table
     → Shows: 0 pages added, 1 link added, 1 orphan resolved

Postconditions:
  - Akiko can see the net graph effect of her session in one command
  - No zetl-specific storage beyond the current index

Failure modes:
  - Vault not in git: error with code NOT_A_GIT_REPO
  - --since date before first commit: uses first commit as baseline, warns
```

### 3.2 Happy Path: Agentic Delta Processing (Dev Agent)

```
Preconditions:
  - Agent stored last-processed commit SHA: "540f0a9"
  - New commits have been pushed to the vault since

Steps:
  1. zetl diff --from 540f0a9 --format json
     → Returns structured diff: 2 pages added, 9 links added, 1 new orphan, 1 new dead link

  2. Agent re-reasons only over the 2 new pages and their immediate neighbours
  3. Agent stores new HEAD SHA

Postconditions:
  - Agent processed 2 pages instead of 2,000
  - No zetl state written; the agent manages its own cursor (the commit SHA)

Failure modes:
  - SHA not found in git history: error with code REF_NOT_FOUND
  - Git not available: error with code NOT_A_GIT_REPO
```

---

## 4. Functional Requirements

### REQ-046: Git Ref Baseline

The system SHALL support `zetl diff --from <git-ref>` where `<git-ref>` is any ref resolvable by `git rev-parse` (commit SHA, branch name, tag, `HEAD~N`, etc.), and SHALL reconstruct the vault graph at that ref using `git show <ref>:<path>` for each changed file, then compute and output a structured graph diff against the current index.

Trace:
- TEST-051
- CON-021

### REQ-047: Since-Date Baseline

The system SHALL support `zetl diff --since <datetime>` where `<datetime>` is an ISO 8601 date or datetime string, resolving the baseline to the most recent commit whose author date is ≤ `<datetime>` via `git rev-list --before=<datetime> -1 HEAD`. When no commit exists at or before the specified datetime, the system SHALL error with code `NO_COMMIT_BEFORE`.

Trace:
- TEST-052
- CON-021

### REQ-048: Default Baseline

When `zetl diff` is called with no baseline argument, the system SHALL use `HEAD~1` (the parent of the current commit) as the baseline, equivalent to `zetl diff --from HEAD~1`.

Trace:
- TEST-051
- CON-021

### REQ-049: Diff Output Schema

The system SHALL output a structured diff record containing: `from` (resolved git ref, commit SHA, commit timestamp), `to` (current HEAD SHA, timestamp), `pages_added`, `pages_removed`, `links_added`, `links_removed`, `orphans_gained`, `orphans_resolved`, `dead_links_added`, `dead_links_resolved`.

Trace:
- TEST-051, TEST-053, TEST-054, TEST-055, TEST-056
- CON-021

### REQ-050: Diff Filter

The system SHALL support `zetl diff --filter <category>` where category is one of: `pages`, `links`, `orphans`, `dead-links`. When specified, the diff output contains only the entries for the selected category; all other categories are omitted.

Trace:
- TEST-057
- CON-021

### REQ-051: Git Unavailable Error

When `zetl diff` is invoked outside a git repository or when the `git` binary is not found, the system SHALL exit non-zero with a structured error containing code `NOT_A_GIT_REPO` and a plain-text message explaining that `zetl diff` requires a git-tracked vault.

Trace:
- TEST-058
- CON-021

### REQ-052: Efficient Reconstruction

The system SHALL reconstruct the baseline graph by re-parsing only the files that differ between the baseline ref and the current working tree, as identified by `git diff --name-only <ref>`. Files not present in the diff SHALL be assumed unchanged and their current graph edges reused. Deleted files SHALL be identified via `git diff --diff-filter=D --name-only <ref>` and their edges excluded from the baseline graph.

Re-parsing SHALL extract only **wikilinks** from old file content. Merkle tree construction, SPL block extraction, and section grounding computation (SPEC-006) SHALL NOT be performed on the baseline state; those structures are not required for a link-graph diff.

Trace:
- TEST-059
- NFR-018

---

## 5. Non-Functional Requirements

### NFR-018: Diff Performance

`zetl diff` SHALL complete in ≤ 500ms for a vault of 2,000 pages where ≤ 50 files changed between the baseline and current state, WITH 95th percentile confidence. This includes git subprocess calls, file re-parsing, and set-difference computation.

Trace:
- TEST-060
- OBS-010

### NFR-019: Git Subprocess Isolation

The system SHALL invoke git via subprocess (not a git library binding) and SHALL NOT assume any specific git version beyond 2.0. All git calls SHALL be read-only (`git show`, `git rev-parse`, `git diff`, `git rev-list`).

Trace:
- TEST-058

---

## 6. Architecture Decisions

### ADR-011: No zetl-Managed Snapshot Storage

**Decision:** `zetl diff` uses git history exclusively as the source of past vault states. Zetl does not maintain its own snapshot files, manifest, or temporal database.

**Context:** An earlier design (SPEC-007 v0.1) proposed that `zetl index` automatically write compressed graph snapshots to `.zetl/snapshots/`. This was rejected after recognising that it largely reimplements git's object storage model for vaults that are already git-tracked.

**Rationale:** Git already provides content-addressed, deduplicated, append-only history of all file states. Maintaining a parallel snapshot store duplicates this, adds storage overhead, and introduces a new failure mode (snapshot writes failing silently). The only genuine value zetl adds is the *graph-level diff semantics* — interpreting file changes as graph changes. That value is independent of the storage mechanism; it can be computed on demand from git history.

**Trade-offs:**
- ✅ No additional storage in `.zetl/`
- ✅ No snapshot capture overhead on every `zetl index`
- ✅ Git history is already the user's backup and audit trail
- ⚠️ Requires git; vaults not in git cannot use `zetl diff`
- ⚠️ Diff computation requires git subprocess calls; performance depends on git

**Rejected alternative:** zetl-native snapshots as fallback for non-git vaults. Deferred — the use case of non-git vaults wanting temporal diffs is not well-understood yet; it should be specified separately if demand emerges.

**VCS boundary note:** This decision makes `zetl diff` the only zetl command that requires git. It does not change the VCS-independence guarantee of all other commands (SPEC-006 §1.6, NFR-017). The git dependency is intentionally contained here.

### ADR-012: Reconstruct via Changed-Files Only, Not Full Checkout

**Decision:** Reconstruct the baseline graph by re-parsing only changed files (via `git diff --name-only`), reusing current graph edges for unchanged files, rather than checking out the baseline ref in full or using `git archive`.

**Context:** Two alternatives:
1. **Full checkout / worktree**: create a temporary git worktree at the baseline ref, run `zetl index` on it, diff the two indexes
2. **Changed-files only** (selected): identify changed `.md` files, read their old content via `git show`, re-parse only those

**Rationale:** Option 1 is simple and correct but expensive: it requires a full filesystem checkout, a full `zetl index` run (scanning all files), and cleanup. For a vault where 10 files changed yesterday, this scans 1,990 files unnecessarily. Option 2 is correct by the invariant that unchanged files contribute identical edges to both graphs — only changed files can produce a graph delta. This makes `zetl diff` fast proportional to the size of the change, not the size of the vault (NFR-018).

**Trade-offs:**
- ✅ Performance scales with change size, not vault size
- ✅ No temporary directory or worktree required
- ⚠️ Slightly more complex implementation (must handle added, deleted, and modified files as distinct cases)
- ⚠️ Requires reading each changed file's old content individually via `git show` rather than a single bulk operation

---

## 7. Contract Specifications

### CON-021: `zetl diff`

**Interface:** `zetl diff [--from <git-ref>] [--since <datetime>] [--filter pages|links|orphans|dead-links] [--format json|table]`

**Argument rules:**
- No arguments: baseline is `HEAD~1` (REQ-048)
- `--from <ref>`: baseline is the specified git ref (REQ-046)
- `--since <datetime>`: baseline is the most recent commit at or before the datetime (REQ-047)
- `--from` and `--since` are mutually exclusive; combining them returns `INVALID_ARGUMENTS`

**Pre-conditions:**
- Current directory is inside a git repository
- `git` binary is available on PATH
- `zetl index` has been run at least once with the SPEC-006 cache format (Merkle tree data present in `.zetl/index.json`); an index built before SPEC-006 is in place will produce incomplete results
- Baseline ref is resolvable by `git rev-parse`

**Working-tree note:** The current state is the working tree as read by `zetl index`. `git diff --name-only <ref>` compares `<ref>` against the working tree; files with uncommitted edits will appear in the diff alongside committed changes. This is intentional — `zetl index` operates on the working tree, so the current state always reflects it.

**Post-conditions:**
- Exit 0 on success, including when diff is empty
- Exit non-zero on all errors

**Output schema (JSON):**

```json
{
  "from": {
    "ref": "HEAD~1",
    "commit": "540f0a9c1d2e3f4a",
    "timestamp": "2026-02-23T07:58:41Z"
  },
  "to": {
    "commit": "4bfd1d1a2b3c4d5e",
    "timestamp": "2026-02-24T09:12:03Z"
  },
  "pages_added": ["Redis Latency Spike Investigation", "ADR-042 Reject gRPC"],
  "pages_removed": [],
  "links_added": [
    { "from": "ADR-042 Reject gRPC", "to": "Redis Latency Spike Investigation" },
    { "from": "ADR-042 Reject gRPC", "to": "Network Architecture Overview" }
  ],
  "links_removed": [],
  "orphans_gained": ["Redis Latency Spike Investigation"],
  "orphans_resolved": [],
  "dead_links_added": [
    { "from": "ADR-042 Reject gRPC", "to": "Benchmark: gRPC vs REST" }
  ],
  "dead_links_resolved": []
}
```

**Error model:**

```json
{ "error": { "code": "NOT_A_GIT_REPO", "message": "zetl diff requires a git-tracked vault. This directory is not inside a git repository." } }
{ "error": { "code": "REF_NOT_FOUND", "message": "Git ref '540f0a9' could not be resolved. Run git log to list available commits." } }
{ "error": { "code": "NO_COMMIT_BEFORE", "message": "No commit found at or before 2025-01-01T00:00:00Z. Earliest commit: 2026-01-15T10:23:04Z." } }
{ "error": { "code": "INVALID_ARGUMENTS", "message": "--from and --since are mutually exclusive." } }
{ "error": { "code": "INDEX_REQUIRED", "message": "No current index found. Run zetl index first." } }
```

**Implements:** REQ-046, REQ-047, REQ-048, REQ-049, REQ-050, REQ-051

**Verified by:** TEST-051–060

---

## 8. Test Specifications

### TEST-051: Diff — Default Baseline (HEAD~1)

**Requirement:** REQ-046, REQ-048, REQ-049

**Preconditions:** Vault in git with two commits; second commit adds `NewPage.md` with a link to `ExistingPage`

**Steps:**
1. Run `zetl index`; run `zetl diff --format json`
2. Verify `from.ref` is `HEAD~1`; `from.commit` matches `git rev-parse HEAD~1`
3. Verify `pages_added` contains `"NewPage"`
4. Verify `links_added` contains `{from: "NewPage", to: "ExistingPage"}`

---

### TEST-052: Diff — Since Date Baseline

**Requirement:** REQ-047

**Preconditions:** Vault with commits spanning multiple days

**Steps:**
1. Run `zetl diff --since <date-between-commits> --format json`
2. Verify `from.commit` is the most recent commit at or before the specified date
3. Run `zetl diff --since 2020-01-01 --format json`
4. Verify error `NO_COMMIT_BEFORE` with earliest commit timestamp

---

### TEST-053: Diff — Pages Added and Removed

**Requirement:** REQ-049

**Preconditions:** Commit B adds `NewPage.md`, deletes `OldPage.md`

**Steps:**
1. `zetl diff --from <commit-A> --format json`
2. Verify `pages_added: ["NewPage"]`, `pages_removed: ["OldPage"]`

---

### TEST-054: Diff — Links Added

**Requirement:** REQ-049

**Preconditions:** Commit adds `[[TargetPage]]` to `SourcePage.md`

**Steps:**
1. `zetl diff --from HEAD~1 --format json`
2. Verify `links_added` contains `{from: "SourcePage", to: "TargetPage"}`

---

### TEST-055: Diff — Links Removed

**Requirement:** REQ-049

**Preconditions:** Commit removes `[[TargetPage]]` from `SourcePage.md`

**Steps:**
1. `zetl diff --from HEAD~1 --format json`
2. Verify `links_removed` contains `{from: "SourcePage", to: "TargetPage"}`

---

### TEST-056: Diff — Orphans Gained and Resolved

**Requirement:** REQ-049

**Preconditions:** Two separate commits: one removes the only backlink to `LonePage`, one restores it

**Steps:**
1. After backlink removed: `zetl diff --from HEAD~1` → `orphans_gained: ["LonePage"]`
2. After backlink restored: `zetl diff --from HEAD~1` → `orphans_resolved: ["LonePage"]`

---

### TEST-057: Diff — Filter

**Requirement:** REQ-050

**Preconditions:** Commit with page additions, link changes, and orphan changes

**Steps:**
1. `zetl diff --filter orphans --format json`
2. Verify response contains `orphans_gained` and `orphans_resolved` only
3. Verify `pages_added`, `links_added`, etc. are absent from response

---

### TEST-058: Git Not Available — Error

**Requirement:** REQ-051

**Preconditions:** Vault directory is not inside a git repository

**Steps:**
1. `zetl diff --format json`
2. Verify exit code is non-zero
3. Verify error `NOT_A_GIT_REPO`

---

### TEST-059: Efficient Reconstruction — Changed Files Only

**Requirement:** REQ-052

**Preconditions:** Vault with 500 pages; only 5 files changed since baseline commit

**Steps:**
1. Instrument git subprocess calls
2. Run `zetl diff --from <baseline>`
3. Verify `git show` is called exactly for the 5 changed files (plus the diff listing), not for all 500

---

### TEST-060: Diff Performance

**Requirement:** NFR-018

**Preconditions:** Vault with 2,000 pages; 50 files changed since baseline

**Steps:**
1. Run `zetl diff --from <baseline>` 10 times; measure wall-clock time

**Expected:** ≤ 500ms at p95.

---

## 9. Observability

### OBS-010: Diff Timing

**Signal:** Verbose output when `--verbose` passed to `zetl diff`

```
[zetl] diff: ref=HEAD~1 commit=540f0a9 files_changed=12 duration_ms=87
```

**Purpose:** Verify NFR-018; diagnose slow diffs on large vaults or slow git operations

---

## 10. Traceability Matrix

| REQ     | NFR     | CON    | ADR              | TEST                          | OBS    |
| ------- | ------- | ------ | ---------------- | ----------------------------- | ------ |
| REQ-046 | NFR-019 | CON-021 | ADR-011, ADR-012 | TEST-051, TEST-054            | OBS-010 |
| REQ-047 | —       | CON-021 | —                | TEST-052                      | —      |
| REQ-048 | —       | CON-021 | —                | TEST-051                      | —      |
| REQ-049 | —       | CON-021 | —                | TEST-051, TEST-053–056        | —      |
| REQ-050 | —       | CON-021 | —                | TEST-057                      | —      |
| REQ-051 | —       | CON-021 | ADR-011          | TEST-058                      | —      |
| REQ-052 | NFR-018 | —       | ADR-012          | TEST-059, TEST-060            | OBS-010 |

---

## 11. Future Work

### 11.1 Non-Git Vaults

If demand emerges for temporal diffs in non-git vaults, a minimal zetl-native snapshot mechanism could be specified separately. It would be a thin addition: `zetl index --snapshot` to explicitly capture a named checkpoint. This is intentionally not included here; git is the right default.

### 11.2 Reasoning Diff

`zetl reason diff --from <ref>` would compute changes in SPL reasoning conclusions across git history. Requires combining SPEC-005 (reasoning) with SPEC-007's reconstruction approach: re-run the theory over the baseline graph and diff conclusions. Reserved for a future SPEC.

SPEC-006's Merkle infrastructure provides a natural efficiency layer for this feature. The vault root hash (stored in `.zetl/theory.json`) offers a fast early exit: if the vault root hash at `<ref>` matches the current vault root hash, the theory is provably identical and no re-run is needed. Comparing only the SPL leaf AST hashes across the two states narrows reconstruction further to files where SPL content actually changed — avoiding a full theory rebuild when only prose was edited.

### 11.3 Page Timeline

`zetl history page <name>` scanning all commits touching a page is expensive and requires a different design (iterating git log, not just one diff). Deferred.
