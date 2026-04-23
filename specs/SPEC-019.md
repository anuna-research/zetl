---
title: "SPEC-019: Git Commit Anchoring in History Snapshots"
status: draft
version: 0.1.0
date: 2026-03-06
parent: SPEC-017
---

# SPEC-019: Git Commit Anchoring in History Snapshots

## Information Table

| Field        | Value                                                          |
| ------------ | -------------------------------------------------------------- |
| Document ID  | SPEC-019                                                       |
| Title        | Git Commit Anchoring in History Snapshots                      |
| Version      | 0.1.0                                                          |
| Status       | Draft                                                          |
| Author       | Agent (USDD Protocol v1.3.0)                                   |
| Date         | 2026-03-06                                                     |
| Audience     | Agent, Human                                                   |
| Trace        | USDD Agent Protocol v1.3.0                                     |
| Parent       | SPEC-017: ztl history — Invisible Temporal Graph Navigation   |
| Related      | SPEC-006: Merkle Tree; SPEC-008: Watch Mode                    |
| Dependencies | SPEC-017 (jj-lib history); `vcs.rs` (git metadata reader)     |

---

## 1. Overview

SPEC-017 gives ztl fine-grained temporal navigation: every vault state is captured as a jj snapshot, and any past state can be queried with `--at`. Each snapshot carries a `vault_root_hash` (SPEC-006) that identifies the vault's content at that moment.

What snapshots do *not* carry is a reference to the **git commit** that was current when the snapshot was taken. This means the history system can tell you *when* a note last changed and *what* the vault looked like, but it cannot tell you *where in the project's git history* that change sits.

This matters because ztl vaults often document codebases. A note about `src/cli.rs` was last changed on March 1st, but the user's real question is: "What git commits landed in the codebase since I last updated this note?" Without a git commit anchor, answering that question requires fuzzy timestamp matching against `git log` — unreliable when clocks differ, commits are rebased, or the vault and repo live on different machines.

This specification adds a single, small piece of data: the git HEAD commit hash at snapshot time. This anchors each ztl snapshot to a precise point in git history, enabling exact `git log <anchor>..HEAD` queries without timestamp guesswork.

### 1.1 Design Principle: Supplementary, Not Required

Consistent with `vcs.rs`'s existing contract (NFR-017 §1.6): git metadata enriches output but does not alter correctness or cache decisions. Vaults without git continue to work identically. The git commit is `null` in all outputs when unavailable.

### 1.2 Scope

**In scope:**

- Embedding git HEAD commit hash in jj snapshot descriptions
- Extracting git commit from snapshot descriptions (pure function)
- Surfacing `git_commit` in `PageHistoryContext`, `PageHistoryEntry`, and `VaultHistoryContext`
- Including `git_commit` fields in `history-index.json` export
- Including `git_commit` in template context (`page.history`, `vault.history`)
- Including `git_commit` in hook context (`HookContext.history`)
- Observability for git metadata capture

**Out of scope:**

- Running `git log` or `git diff` from within ztl (that is a hook/agent concern)
- Tracking per-file git blame or authorship
- Any git write operations
- Frontmatter extraction or "sync source" declarations
- Staleness detection logic (belongs in hooks/agents, not ztl core)

---

## 2. User Profiles

### 2.1 User Profile: Priya — Documentation Maintainer

```
Name:        Priya
Role:        Staff engineer; maintains a 500-note vault documenting a large Rust codebase
Goals:       Know which notes are stale relative to the code they document;
             produce a staleness report for sprint planning
Constraints: Uses git daily; runs ztl build to publish internal docs;
             comfortable writing shell hooks
Workflow:    Edits vault notes when code changes significantly; runs ztl build
             weekly; wants a hook that flags notes not updated since relevant code changed
Pain point:  "I know my CLI Architecture note is outdated because the code changed,
             but I can't prove it without manually checking git log dates."
```

### 2.2 User Profile: Agent — CI/CD Documentation Bot

```
Name:        Agent
Role:        Automated process running in CI after merge to main
Goals:       Read history-index.json, identify notes whose git_commit anchor
             is behind HEAD, produce a staleness diff for human review
Constraints: No interactive access; reads JSON; runs git commands;
             outputs structured reports
Workflow:    1. Read history-index.json for per-page last_changed_git_commit
             2. For each page with a commit anchor, run git log <anchor>..HEAD -- <tracked paths>
             3. Emit structured staleness report
Pain point:  "Without a commit anchor, I have to parse timestamps and hope they align.
             Rebases, amended commits, and timezone drift make this unreliable."
```

---

## 3. Happy Paths

### 3.1 Happy Path: Priya Checks Note Staleness

```
Preconditions:
  - Vault is inside a git repository
  - History feature is enabled (--features history)
  - ztl index has been run at least twice (creating snapshots with git anchors)

Steps:
  1. Priya edits "CLI Architecture.md" and runs `ztl index`
     -> Snapshot created with vault_root_hash and git_commit=abc123
  2. Three days later, a teammate merges PRs touching src/cli.rs
     -> git HEAD advances to def456; vault is unchanged
  3. Priya runs `ztl build`
     -> history-index.json includes:
        "CLI Architecture": { "last_changed_git_commit": "abc123", ... }
  4. A hook reads history-index.json and runs:
     git log abc123..HEAD -- src/cli.rs
     -> Outputs: 3 commits, +47 -12 lines changed
  5. Priya sees the staleness report and updates her note

Postconditions:
  - Priya knows exactly which git commits landed since her last note edit
  - No timestamp fuzzy-matching required

Failure modes:
  - Vault not in a git repo: git_commit is null; hook skips this page
  - Commit abc123 was force-pushed away: git log errors; hook reports "anchor commit not found"
  - Vault and code are in different repos: git_commit anchors the vault's repo, not the code's; hook must handle this
```

### 3.2 Happy Path: CI Agent Produces Staleness Report

```
Preconditions:
  - ztl build has been run, producing history-index.json
  - CI has access to the git repository

Steps:
  1. Agent reads history-index.json
  2. For each page with a non-null last_changed_git_commit:
     a. Agent reads the page's frontmatter (or a config mapping) to find tracked source paths
     b. Agent runs: git log <last_changed_git_commit>..HEAD -- <tracked paths>
     c. If commits exist, the page is stale
  3. Agent emits a JSON report: { "stale": [...], "fresh": [...], "unanchored": [...] }

Postconditions:
  - Structured staleness report available for human review or further automation
  - Pages without git anchors (new pages, non-git vaults) listed separately as "unanchored"
```

---

## 4. Requirements

### REQ-102: Git Commit Embedding in Snapshot Descriptions

The `auto_snapshot` function (SPEC-017 REQ-076) SHALL read the git HEAD commit hash via `vcs::get_git_metadata()` at snapshot creation time and embed it in the jj commit description using the format:

```
ztl-snapshot vault_root_hash=<64-hex> git_commit=<40-hex>
```

When the vault is not inside a git repository, or the git commit cannot be resolved, the `git_commit=` field SHALL be omitted. The snapshot description format degrades to the existing format:

```
ztl-snapshot vault_root_hash=<64-hex>
```

The `git_dirty` flag SHALL NOT be embedded in the description. Rationale: dirtiness is transient state at snapshot time with no clear consumer value, and would complicate description parsing for marginal benefit.

Trace:
- TEST-124
- CON-031
- ADR-055

### REQ-103: Git Commit Extraction from Descriptions (Pure Function)

A pure function `extract_git_commit_from_description(description: &str) -> Option<String>` SHALL extract the git commit hash from a jj commit description. It SHALL:

- Return `Some(hash)` when the description contains a `git_commit=` field with exactly 40 lowercase hexadecimal characters
- Return `None` when the field is absent or malformed
- Not perform I/O, VCS calls, or allocations beyond the returned String

This function mirrors `extract_vault_root_hash_from_description` (SPEC-017) in structure and testing approach.

Trace:
- TEST-125, TEST-126
- CON-031

### REQ-104: Git Commit in PageHistoryEntry

The `PageHistoryEntry` struct SHALL include a `git_commit: Option<String>` field. When constructing entries from snapshots (in `extract_page_history`), the git commit SHALL be extracted from the snapshot's `ChangeInfo.description` using the function from REQ-103.

The field is `None` when:
- The snapshot was created in a non-git vault
- The snapshot predates this feature (description lacks `git_commit=`)

Trace:
- TEST-127
- CON-031

### REQ-105: Git Commit in PageHistoryContext

The `PageHistoryContext` struct SHALL include:

- `last_changed_git_commit: Option<String>` — the git commit from the most recent `PageHistoryEntry` (the snapshot where the page's neighbourhood last changed)
- `created_at_git_commit: Option<String>` — the git commit from the oldest `PageHistoryEntry` (the snapshot where the page first appeared)

These fields are derived from the first and last entries of the page's history timeline (newest-first), respectively. They are `None` when the corresponding entry's `git_commit` is `None`.

Trace:
- TEST-128
- CON-031

### REQ-106: Git Commit in VaultHistoryContext

The `VaultHistoryContext` struct SHALL include:

- `newest_git_commit: Option<String>` — the git commit from the most recent snapshot
- `oldest_git_commit: Option<String>` — the git commit from the oldest snapshot

These are extracted from the first and last entries of the vault's snapshot list. They are `None` when the corresponding snapshot lacks a git commit.

Trace:
- TEST-129
- CON-031

### REQ-107: Git Commit in History Index Export

The `serialize_history_index` function SHALL include git commit fields in its JSON output:

- Vault level: `newest_git_commit` and `oldest_git_commit`
- Page level: `last_changed_git_commit` and `created_at_git_commit`

All four fields are nullable strings. When `None`, they serialize as JSON `null`.

Trace:
- TEST-130
- CON-032

### REQ-108: Git Commit in Template Context

The `page.history.last_changed_git_commit` and `page.history.created_at_git_commit` template variables SHALL be available in page templates. The `vault.history.newest_git_commit` and `vault.history.oldest_git_commit` template variables SHALL be available in vault-level templates.

When git is unavailable, all four variables evaluate to `none` (minijinja null). Templates SHALL NOT require these fields — the graceful degradation contract from SPEC-017 (REQ-084) applies.

Trace:
- TEST-131
- CON-031

### REQ-109: Git Commit in Hook History Context

The `HookContext.history` object SHALL include `newest_git_commit: Option<String>` (from the most recent snapshot at hook execution time). Hooks that fire after `ztl index` or `ztl build` receive the git commit that was current when the snapshot was created.

Trace:
- TEST-132
- CON-031

---

## 5. Contracts

### CON-031: Snapshot Description Format (Extended)

**Format (with git):**

```
ztl-snapshot vault_root_hash=<64-hex-lowercase> git_commit=<40-hex-lowercase>
```

**Format (without git):**

```
ztl-snapshot vault_root_hash=<64-hex-lowercase>
```

**Format (minimal, legacy):**

```
ztl-snapshot
```

**Parsing rules:**

- Fields are whitespace-separated key=value pairs after the `ztl-snapshot` prefix
- `vault_root_hash` is exactly 64 lowercase hex characters
- `git_commit` is exactly 40 lowercase hex characters (SHA-1)
- Unknown fields SHALL be ignored (forward compatibility)
- Field order is not significant
- Both fields are optional

**Backward compatibility:**

Snapshots created before SPEC-019 lack `git_commit=`. The extraction function returns `None` for these descriptions. No migration is required — the feature fills in automatically as new snapshots are created.

Implements:
- REQ-102, REQ-103, REQ-104, REQ-105, REQ-106, REQ-107, REQ-108, REQ-109

Verified by:
- TEST-124, TEST-125, TEST-126

### CON-032: History Index JSON Schema (Extended)

Extends the `history-index.json` schema from SPEC-017 (REQ-088):

```json
{
  "vault": {
    "trend": [...],
    "snapshot_count": 42,
    "unique_states": 38,
    "oldest": "2026-01-15T...",
    "newest": "2026-03-06T...",
    "oldest_git_commit": "a1b2c3d4e5f6...(40 hex)",
    "newest_git_commit": "f6e5d4c3b2a1...(40 hex)"
  },
  "pages": {
    "CLI Architecture": {
      "link_trend": [...],
      "created_at": "2026-01-15T...",
      "last_changed": "2026-03-01T...",
      "created_at_git_commit": "a1b2c3d4e5f6...(40 hex)",
      "last_changed_git_commit": "d4c3b2a1e5f6...(40 hex)"
    }
  }
}
```

**New fields (all nullable):**

| Field | Type | Location | Description |
|-------|------|----------|-------------|
| `oldest_git_commit` | `string \| null` | `vault` | Git commit at oldest snapshot |
| `newest_git_commit` | `string \| null` | `vault` | Git commit at newest snapshot |
| `created_at_git_commit` | `string \| null` | `pages.<name>` | Git commit when page first appeared |
| `last_changed_git_commit` | `string \| null` | `pages.<name>` | Git commit when page last changed |

**Backward compatibility:**

Consumers that do not read the new fields are unaffected. The existing fields (`trend`, `snapshot_count`, `unique_states`, `oldest`, `newest`, `link_trend`, `created_at`, `last_changed`) are unchanged in semantics and position.

Implements:
- REQ-107

Verified by:
- TEST-130

---

## 6. Architecture Decisions

### ADR-055: Embed Git Commit in Snapshot Description, Not Separate Metadata

**Context:** We need to associate a git commit hash with each jj snapshot. Options considered:

1. **Embed in jj commit description** (chosen) — append `git_commit=<hash>` to the existing `ztl-snapshot vault_root_hash=<hash>` description string
2. **Separate metadata file** — write `.ztl/history/git-anchors.json` mapping jj change IDs to git commits
3. **jj commit metadata/tags** — use jj's native metadata features

**Decision:** Option 1.

**Rationale:**

- **Consistency**: `vault_root_hash` already uses this pattern. Adding `git_commit` extends it naturally.
- **Atomicity**: The git commit is captured at the same instant as the snapshot, in the same operation. No race between snapshot creation and metadata write.
- **Durability**: jj commit descriptions are part of the commit object. They survive compaction, garbage collection, and export. A separate file could become orphaned or out of sync.
- **Simplicity**: No new file format, no new I/O path, no new cache invalidation concern. One `format!()` call changes.
- **Extraction**: The pure extraction function mirrors the existing `extract_vault_root_hash_from_description`, keeping the parsing logic symmetric and testable.

**Rejected alternatives:**

- Option 2 adds a coordination problem (snapshot + file must be written atomically) and a new file to manage.
- Option 3 depends on jj-lib API surface for commit metadata, which is less stable than commit descriptions.

**Trace:** REQ-102, REQ-103

### ADR-056: No `git_dirty` in Snapshot Descriptions

**Context:** `vcs::get_git_metadata()` returns both `git_commit` and `git_dirty`. Should we embed both?

**Decision:** Embed only `git_commit`. Omit `git_dirty`.

**Rationale:**

- **Transient state**: `git_dirty` reflects the working tree at snapshot instant. By the time anyone reads the snapshot, the working tree has changed. The flag answers a question nobody asks later.
- **Parse complexity**: Adding boolean flags to the description format introduces a third value type (`key=hash`, `key=bool`) with no clear consumer.
- **No consumer**: The primary use case — `git log <anchor>..HEAD` — needs a commit hash. Whether the tree was dirty at snapshot time does not affect the query.
- **Available if needed**: If a future use case requires dirtiness, it can be added without breaking the format (unknown fields are ignored per CON-031).

**Trace:** REQ-102

---

## 7. Purity Boundary Map

### Pure Core (no I/O, no shared state, deterministic)

- `extract_git_commit_from_description(&str) -> Option<String>`: Parse git commit from snapshot description
- `build_page_history_context(...)` (extended): Derive `last_changed_git_commit` and `created_at_git_commit` from entries
- `serialize_history_index(...)` (extended): Include git commit fields in JSON output

### Effectful Shell (orchestrates I/O, calls pure core)

- `auto_snapshot(vault_root, vault_root_hash)` (extended): Calls `vcs::get_git_metadata()` to read git HEAD before building the description string
- `build_history_index_json(...)` (extended): Passes git commit data through to `serialize_history_index`

### Boundary Contracts

- `Option<String>` (git commit hash) flows from shell to core as a parameter, never the reverse
- Core functions never call `vcs::get_git_metadata()` or any I/O

### Dependency Rule

Shell -> Core. Core MUST NOT import from shell. The only new I/O call is `vcs::get_git_metadata()` in `auto_snapshot`, which already lives in the effectful shell.

### Enforcement

- `extract_git_commit_from_description` is a standalone pure function with no `use` of `vcs`, `std::fs`, or `std::process`
- Existing `#[cfg(test)]` tests for the extraction function use string literals, not live git repos

---

## 8. Test Specifications

### TEST-124: Git Commit Embedded in Snapshot Description

**Verifies:** REQ-102

**Scenario:** Create a snapshot in a vault that is inside a git repository.

**Steps:**
1. Initialise a temporary git repository with at least one commit
2. Create a vault inside it with one markdown file
3. Run `auto_snapshot(vault_root, Some(vault_root_hash))`
4. Read the most recent jj snapshot's description

**Expected:** Description matches `ztl-snapshot vault_root_hash=<64-hex> git_commit=<40-hex>` where the git_commit matches the git HEAD at step 3.

### TEST-125: Git Commit Extraction — Valid Description

**Verifies:** REQ-103

**Scenario:** Extract git commit from a well-formed description.

**Steps:**
1. Call `extract_git_commit_from_description("ztl-snapshot vault_root_hash=aaa...aaa git_commit=bbb...bbb")` (64 a's, 40 b's)

**Expected:** Returns `Some("bbb...bbb")`.

### TEST-126: Git Commit Extraction — Missing, Malformed, Legacy

**Verifies:** REQ-103

**Scenario:** Extract git commit from descriptions that lack or have malformed git_commit fields.

**Cases:**

| Input | Expected |
|-------|----------|
| `"ztl-snapshot vault_root_hash=aaa...aaa"` | `None` |
| `"ztl-snapshot"` | `None` |
| `"ztl-snapshot git_commit=short"` | `None` |
| `"ztl-snapshot git_commit=XXXX...XXXX"` (40 non-hex chars) | `None` |
| `"ztl-snapshot vault_root_hash=aaa...aaa git_commit=bbb...bbb extra_field=ccc"` | `Some("bbb...bbb")` (unknown fields ignored) |

### TEST-127: PageHistoryEntry Includes Git Commit

**Verifies:** REQ-104

**Scenario:** Build page history entries from snapshots that contain git commits.

**Steps:**
1. Construct `ChangeInfo` entries with descriptions containing `git_commit=<hash>`
2. Call `extract_page_history(page_name, snapshots, files_per_snapshot, limit)`
3. Inspect returned entries

**Expected:** Each `PageHistoryEntry.git_commit` matches the hash from its source snapshot's description. Entries from snapshots without `git_commit=` have `git_commit: None`.

### TEST-128: PageHistoryContext Includes Git Commit Anchors

**Verifies:** REQ-105

**Scenario:** Build page history context from a page with multiple history entries.

**Steps:**
1. Construct snapshots spanning a page's lifetime, some with git commits, some without
2. Call `build_page_history_context(page_name, snapshots, files_per_snapshot, now)`

**Expected:**
- `last_changed_git_commit` equals the git commit from the newest entry (or `None` if that entry lacks one)
- `created_at_git_commit` equals the git commit from the oldest entry (or `None` if that entry lacks one)

### TEST-129: VaultHistoryContext Includes Git Commit Anchors

**Verifies:** REQ-106

**Scenario:** Build vault history context from snapshots with git commits.

**Expected:**
- `newest_git_commit` matches the git commit from the most recent snapshot
- `oldest_git_commit` matches the git commit from the oldest snapshot
- Both are `None` when snapshots lack git commits

### TEST-130: History Index JSON Includes Git Commit Fields

**Verifies:** REQ-107

**Scenario:** Serialize history index and verify JSON schema.

**Steps:**
1. Build `VaultHistoryContext` and `PageHistoryContext` with known git commits
2. Call `serialize_history_index(vault, pages)`
3. Parse the resulting JSON

**Expected:**
- `vault.newest_git_commit` and `vault.oldest_git_commit` present and correct
- `pages.<name>.last_changed_git_commit` and `pages.<name>.created_at_git_commit` present and correct
- Fields are `null` (not absent) when the source value is `None`

### TEST-131: Template Context Exposes Git Commit

**Verifies:** REQ-108

**Scenario:** Render a page template that references `page.history.last_changed_git_commit`.

**Steps:**
1. Build a page context with history containing a git commit
2. Render a template containing `{{ page.history.last_changed_git_commit }}`

**Expected:** Output contains the 40-hex git commit hash. When history is unavailable, the variable evaluates to empty/none without error.

### TEST-132: Hook Context Includes Git Commit

**Verifies:** REQ-109

**Scenario:** Fire an on-index hook and inspect the history context.

**Steps:**
1. Run `ztl index` in a git-tracked vault with history enabled
2. Inspect the hook context JSON passed to the hook

**Expected:** `history.newest_git_commit` contains the git HEAD hash at index time.

---

## 9. Observability

### OBS-021: Git Metadata Capture Timing

The `auto_snapshot` function SHALL emit a verbose log line when git metadata is captured:

```
[ztl] snapshot: git_commit=<40-hex> git_dirty=<true|false> duration_ms=<N>
```

This line is emitted only when `--verbose` is active and git metadata was successfully read. The `duration_ms` covers the `vcs::get_git_metadata()` call only (not the full snapshot operation, which is already timed by OBS-011).

When git is unavailable, no line is emitted (absence signals non-git vault).

Trace:
- REQ-102

---

## 10. Non-Functional Requirements

### NFR-042: Git Metadata Read Latency

The `vcs::get_git_metadata()` call SHALL complete in ≤ 5ms for repositories with ≤ 100,000 commits UNDER typical filesystem conditions. This function already exists and reads `.git/HEAD` and ref files directly without spawning a subprocess. The `git_dirty` check (which does spawn `git status`) is already called by existing code and is not on the new critical path — only `git_commit` (file read) is needed for the snapshot description.

Trace:
- TEST-124
- OBS-021

### NFR-043: Zero Overhead for Non-Git Vaults

Vaults that are not inside a git repository SHALL experience zero additional latency from this feature. The `vcs::get_git_metadata()` function returns `(None, None)` after a single `find_git_dir()` walk that terminates at the filesystem root. No new I/O is introduced for non-git vaults.

Trace:
- TEST-126

---

## 11. Implementation Notes

### 11.1 Changes to `auto_snapshot`

The function signature does not change. The git commit is read internally:

```rust
pub fn auto_snapshot(
    vault_root: &Path,
    vault_root_hash: Option<&str>,
) -> anyhow::Result<Option<String>> {
    let mut backend = jj_backend::JjBackend::open_or_init_at_vault_root(vault_root)?;

    // Read git HEAD (file I/O only, no subprocess for commit hash)
    let (git_commit, _git_dirty) = crate::vcs::get_git_metadata(vault_root);

    let mut description = match vault_root_hash {
        Some(hash) => format!("ztl-snapshot vault_root_hash={hash}"),
        None => "ztl-snapshot".to_owned(),
    };
    if let Some(ref commit) = git_commit {
        description.push_str(&format!(" git_commit={commit}"));
    }

    // ... existing deduplication and snapshot logic unchanged ...
}
```

### 11.2 Changes to `core.rs`

New pure function alongside `extract_vault_root_hash_from_description`:

```rust
pub fn extract_git_commit_from_description(description: &str) -> Option<String> {
    for part in description.split_whitespace() {
        if let Some(hash) = part.strip_prefix("git_commit=") {
            if hash.len() == 40 && hash.chars().all(|c| c.is_ascii_hexdigit()) {
                return Some(hash.to_string());
            }
        }
    }
    None
}
```

### 11.3 Struct Extensions

```rust
// PageHistoryEntry — add field
pub struct PageHistoryEntry {
    // ... existing fields ...
    pub git_commit: Option<String>,
}

// PageHistoryContext — add fields
pub struct PageHistoryContext {
    // ... existing fields ...
    pub last_changed_git_commit: Option<String>,
    pub created_at_git_commit: Option<String>,
}

// VaultHistoryContext — add fields
pub struct VaultHistoryContext {
    // ... existing fields ...
    pub newest_git_commit: Option<String>,
    pub oldest_git_commit: Option<String>,
}
```

### 11.4 Affected Files

| File | Change |
|------|--------|
| `src/history/mod.rs` | `auto_snapshot`: read git metadata, append to description |
| `src/history/core.rs` | New `extract_git_commit_from_description`; extend `PageHistoryEntry`, `PageHistoryContext`, `VaultHistoryContext`, `serialize_history_index`, `build_page_history_context` |
| `src/web/context.rs` | Pass new fields through to template context |
| `src/web/build.rs` | Pass new fields through to build output |
| `src/web/routes.rs` | Pass new fields through to serve routes |
| `src/hooks/context.rs` | Include `newest_git_commit` in hook history context |
| `tests/history_integration.rs` | New tests for git commit embedding and extraction |

---

## 12. Traceability Matrix

```
REQ-102 (snapshot embedding)     -> TEST-124         -> CON-031 -> ADR-055 -> OBS-021
REQ-103 (extraction function)    -> TEST-125, 126    -> CON-031
REQ-104 (PageHistoryEntry)       -> TEST-127         -> CON-031
REQ-105 (PageHistoryContext)     -> TEST-128         -> CON-031
REQ-106 (VaultHistoryContext)    -> TEST-129         -> CON-031
REQ-107 (history index JSON)     -> TEST-130         -> CON-032
REQ-108 (template context)       -> TEST-131         -> CON-031
REQ-109 (hook context)           -> TEST-132         -> CON-031
```
