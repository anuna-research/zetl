---
title: "SPEC-008: zetl watch — Vault Watch Mode with Graph Event Streaming"
version: 0.1.0
status: draft
audience: agent, human
date: 2026-02-24
---

# SPEC-008: zetl watch — Vault Watch Mode with Graph Event Streaming

## Information Table

| Field        | Value                                                              |
| ------------ | ------------------------------------------------------------------ |
| Document ID  | SPEC-008                                                           |
| Title        | zetl watch — Vault Watch Mode with Graph Event Streaming           |
| Version      | 0.1.0                                                              |
| Status       | Draft                                                              |
| Author       | Agent (USDD Protocol v1.0.0)                                       |
| Date         | 2026-02-24                                                         |
| Audience     | Agent, Human                                                       |
| Trace        | USDD Agent Protocol v1.0.0                                         |
| Parent       | SPEC-001: zetl — Bi-directional Link Graph CLI                     |
| Related      | SPEC-007: zetl diff, SPEC-006: index cache format                  |
| Dependencies | notify crate (cross-platform FS events), SPEC-006 (Merkle tree, two-tier cache invalidation) |
| Inspiration  | rowboat workspace-watcher pattern                                  |

---

## 1. Overview

Every zetl command is invoked and exits. For a single query this is fine, but it means any process that wants to stay current with the vault — an editor extension, a live dashboard, an AI agent reasoning loop — must poll: run `zetl index` on a timer and hope it didn't miss a change or fire when nothing changed.

`zetl watch` changes this. It starts a persistent process that monitors the vault directory using OS file-system events, incrementally re-indexes changed files on each write, and emits **graph-level events** as newline-delimited JSON (NDJSON) on stdout. Consumers react to changes instead of polling for them.

The event vocabulary mirrors the `zetl diff` output schema (SPEC-007): pages added/removed, links added/removed, orphans gained/resolved, dead links added/resolved. This deliberate alignment means any consumer that can process `zetl diff` output can also process `zetl watch` events.

### 1.1 Design Principles

This feature follows zetl's existing philosophy:

- **Files are the source of truth.** zetl watch never modifies vault files.
- **Agent-first, human-friendly.** NDJSON by default; `--format table` for human observation.
- **Fast and disposable.** The in-memory index maintained by the watch process is rebuilt from the current cache on startup; no new persistent state is introduced.
- **Minimal surprise.** Events are emitted exactly when the graph changes — not on every file write, not on content-identical saves.

### 1.2 Scope

**In scope:**

- `zetl watch [<vault-path>]` command
- OS file-system event monitoring via the `notify` crate (cross-platform)
- Debounced, incremental re-index on `.md` file changes
- NDJSON graph event stream on stdout (one event per line)
- `--exec <cmd>` to pipe each event to an external command
- Graceful shutdown on SIGINT / SIGTERM
- Hybrid mtime + SHA-256 change detection to suppress content-identical saves

**Out of scope:**

- Remote/networked vault watching (local file system only)
- Watching non-`.md` files (attachments, images, PDFs)
- A persistent daemon with socket IPC (future; see §11.1)
- Event replay or event log persistence (future; see §11.2)
- Watch mode for the `zetl reason` subsystem (SPL file changes, theory invalidation; future SPEC)

---

## 2. User Profiles

### 2.1 User Profile: Akiko — Personal Knowledge Worker

```
Name:        Akiko
Role:        Product manager; 2,000-note Zettelkasten
Goals:       Know immediately when she creates a dead link or new orphan while writing;
             run zetl check automatically without manually invoking it after each save
Constraints: Comfortable with CLI; runs zetl in a terminal pane alongside her editor;
             does not want to think about polling intervals
Workflow:    Writes notes in Obsidian; keeps a terminal with zetl watch running in a
             split pane; watches events scroll as she saves; filters orphans and dead
             links with grep
Pain point:  "I find dead links hours later when I could catch them the moment I type them."
```

### 2.2 User Profile: Dev Agent — Incremental Reasoning Loop

```
Name:        Dev Agent (AI agent running locally or in CI)
Role:        Continuous reasoning consumer; wants to re-reason only over changed subgraph
Goals:       React to vault changes in near-real-time; process only affected pages;
             avoid re-processing 2,000 pages on every save
Constraints: Non-interactive; stdout must be machine-parseable; must be able to pipe
             each event to a handler command
Workflow:    1. Start: zetl watch --exec "python handle_event.py"
             2. handle_event.py reads one event from stdin, updates local index, re-reasons
                over the affected neighbourhood, exits
             3. Repeat for each vault write
Pain point:  "Polling zetl index every 5 seconds wastes CPU and misses sub-5s bursts."
```

---

## 3. Happy Paths

### 3.1 Happy Path: Live Dead-Link Detection (Akiko)

```
Preconditions:
  - Vault at ~/notes; zetl watch running in a terminal pane

Steps:
  1. Akiko starts: zetl watch ~/notes --format table
     → emits: [watch] index_ready  pages=2000  links=8421

  2. She saves ADR-043.md containing [[Benchmark: REST vs gRPC]]
     → emits: page_added  ADR-043  (0.3s after save)
     → emits: link_added  ADR-043 → Benchmark: REST vs gRPC
     → emits: dead_link_added  ADR-043 → Benchmark: REST vs gRPC

  3. She creates Benchmark: REST vs gRPC.md and saves
     → emits: page_added  Benchmark: REST vs gRPC
     → emits: orphan_gained  Benchmark: REST vs gRPC
     → emits: dead_link_resolved  ADR-043 → Benchmark: REST vs gRPC

  4. She adds [[Benchmark: REST vs gRPC]] to her Index.md
     → emits: link_added  Index → Benchmark: REST vs gRPC
     → emits: orphan_resolved  Benchmark: REST vs gRPC

Postconditions:
  - Akiko saw the dead link at step 2; corrected it at step 3 in the same session
  - No polling; events arrived within 500ms of each save

Failure modes:
  - Editor writes temp file then renames: debounce window absorbs both events; one batch
  - Vault path does not exist: error before watch starts; exit non-zero
```

### 3.2 Happy Path: Incremental Agent Reasoning Loop (Dev Agent)

```
Preconditions:
  - zetl watch --exec "python reason.py" running as a background process
  - reason.py reads one JSON event from stdin, re-reasons over affected pages, exits

Steps:
  1. User saves Research/Topic-A.md; it gains a link to Research/Topic-B.md
  2. zetl watch detects the change (within 500ms)
  3. Emits two events piped to reason.py:
       {"event":"link_added","from":"Research/Topic-A","to":"Research/Topic-B",...}
       {"event":"index_updated","changed_pages":["Research/Topic-A"],...}
  4. reason.py re-reasons over Topic-A and its neighbours only

Postconditions:
  - Agent processed 1 page rather than 2,000
  - No zetl state written beyond the running in-memory index

Failure modes:
  - --exec command exits non-zero: zetl watch logs to stderr but continues watching
  - --exec command hangs: zetl watch does not block; next event queued
```

---

## 4. Functional Requirements

### REQ-053: Watch Command Entry Point

The system SHALL provide a `zetl watch [<vault-path>]` subcommand that begins monitoring the specified vault directory (defaulting to the current directory) for file-system changes and does not exit until terminated by the user (SIGINT/SIGTERM) or a fatal error.

Trace:
- TEST-061
- CON-022

### REQ-054: File-System Event Monitoring

The system SHALL use OS-native file-system event notifications (inotify on Linux, FSEvents on macOS, ReadDirectoryChangesW on Windows) to detect `.md` file creations, modifications, and deletions within the vault directory hierarchy, respecting `.zetlignore` and the default ignore patterns.

Trace:
- TEST-062
- CON-022
- ADR-013

### REQ-055: Debounced Incremental Re-Index

On detecting one or more `.md` file changes, the system SHALL wait for a debounce window (default: 150ms; configurable via `--debounce <ms>`) before re-indexing, so that rapid sequential writes (e.g. editor temp-file rename, multi-file batch save) are coalesced into a single re-index pass. Only files that changed SHALL be re-parsed; unchanged files are not touched.

Trace:
- TEST-063
- CON-022
- ADR-014

### REQ-056: Change Detection via SPEC-006 Merkle Tree

The system SHALL use the two-tier cache invalidation strategy defined in SPEC-006 REQ-039 and ADR-009 to determine whether a changed file genuinely affects the graph:

1. **Tier 1 (fast path):** If the file's mtime is unchanged since the last index pass, skip — no re-parse needed.
2. **Tier 2 (authoritative):** If mtime changed, re-run the SPEC-006 scanner pipeline on the file to produce a new Merkle root. If the new Merkle root equals the cached file Merkle root, suppress all events — the content is semantically identical. Only when the Merkle root differs SHALL graph recomputation and event emission proceed.

Because SPEC-006 hashes normalised AST nodes (not raw bytes), purely cosmetic saves — whitespace, formatting, comment edits — produce an identical Merkle root and are suppressed without any file-level SHA comparison. This is strictly more accurate than a raw-content hash.

The watch loop holds the in-memory index loaded on startup (including cached file Merkle roots). The roots are updated in-memory as files are re-indexed; they are flushed to `.zetl/index.json` on graceful shutdown (REQ-060).

Trace:
- TEST-064
- SPEC-006/REQ-039
- SPEC-006/ADR-009

### REQ-057: NDJSON Graph Event Stream

The system SHALL emit graph-change events on stdout as newline-delimited JSON (one JSON object per line, each terminated by `\n`). Each event SHALL contain at minimum the fields: `event` (event type string), `timestamp` (ISO 8601 UTC), and type-specific payload fields (see CON-022).

Trace:
- TEST-061, TEST-065
- CON-022

### REQ-058: Event Types

The system SHALL emit the following event types in response to graph changes:

| Event type           | Trigger                                                     |
| -------------------- | ----------------------------------------------------------- |
| `index_ready`        | Initial index loaded; watch loop started                    |
| `page_added`         | A previously absent page now exists in the graph            |
| `page_removed`       | A page present in the graph no longer exists                |
| `link_added`         | A directed edge `from → to` was introduced                  |
| `link_removed`       | A directed edge `from → to` was removed                     |
| `orphan_gained`      | A page transitioned from having ≥1 backlink to 0 backlinks  |
| `orphan_resolved`    | A page transitioned from 0 backlinks to ≥1 backlinks        |
| `dead_link_added`    | A wikilink now targets a non-existent page                  |
| `dead_link_resolved` | A wikilink that previously targeted a non-existent page now resolves |
| `index_updated`      | Batch meta-event emitted once per debounce window after all per-change events; summarises files re-indexed |

Trace:
- TEST-065
- CON-022

### REQ-059: `--exec` Command Invocation

The system SHALL support `zetl watch --exec <cmd>`, where each event JSON is written to the stdin of a new invocation of `<cmd>` (one event per invocation). The system SHALL not wait for `<cmd>` to exit before processing the next file-system event. If `<cmd>` exits non-zero, the system SHALL log the exit code to stderr but SHALL continue watching.

Trace:
- TEST-066
- CON-022

### REQ-060: Graceful Shutdown

On receiving SIGINT or SIGTERM, the system SHALL complete any in-progress re-index pass, flush all buffered events, and exit with code 0.

Trace:
- TEST-067
- CON-022

### REQ-061: Startup Error — Vault Path Not Found

When the specified vault path does not exist or is not a directory, the system SHALL exit non-zero with a structured error containing code `VAULT_NOT_FOUND` before starting the watch loop.

Trace:
- TEST-068
- CON-022

---

## 5. Non-Functional Requirements

### NFR-020: Event Latency

From the moment a `.md` file write is completed by the OS to the moment the first graph event is emitted on stdout, the system SHALL take ≤ 500ms at the 95th percentile for a vault of ≤ 5,000 pages, when the changed file set per debounce window is ≤ 20 files. The debounce window itself (default 150ms) is included in this budget.

Trace:
- TEST-069
- OBS-011

### NFR-021: CPU Idle Overhead

When no file-system changes are occurring, `zetl watch` SHALL consume ≤ 0.5% CPU on the host machine (measured over a 60-second idle window). The watch loop MUST be event-driven, not polling.

Trace:
- TEST-070
- OBS-011

### NFR-022: Memory Ceiling

`zetl watch` SHALL maintain a resident memory footprint of ≤ 250MB for a vault of 10,000 pages. The in-memory index held by the watch process is the same structure as the single-shot `zetl index` result; no additional data structure is introduced beyond the event queue.

Trace:
- OBS-011

---

## 6. Architecture Decisions

### ADR-013: Use the `notify` Crate for File-System Events

**Decision:** Use the [`notify`](https://crates.io/crates/notify) Rust crate for cross-platform file-system event delivery. Do not implement platform-specific watchers directly.

**Context:** File-system event APIs differ significantly between Linux (inotify), macOS (FSEvents / kqueue), and Windows (ReadDirectoryChangesW). A correct cross-platform implementation would duplicate significant OS-specific code.

**Rationale:** `notify` abstracts all three platforms behind a single `Watcher` trait and is the de-facto standard for FS watching in the Rust ecosystem. It is actively maintained, has no unsafe code in the consumer-facing API, and supports both immediate and debounced event delivery. Using it keeps the zetl binary cross-platform without platform-specific conditionals.

**Trade-offs:**
- ✅ Cross-platform with a single dependency
- ✅ Well-tested; used by cargo, watchexec, and others
- ⚠️ Adds a dependency (~2 crate features needed: `macos_fsevent`, `inotify`); binary size increase is negligible (~50KB)
- ⚠️ `notify` debounce is coarse; zetl implements its own debounce (REQ-055) over raw events for tighter control

### ADR-014: Inherit SPEC-006 Two-Tier Cache Invalidation; Do Not Introduce a Separate Hash

**Decision:** `zetl watch` reuses the SPEC-006 Merkle tree pipeline (REQ-039, ADR-009) for change detection. It does **not** introduce a separate SHA-256 or raw-content hash mechanism.

**Context:** An earlier draft of this spec (v0.1.0 before this revision) proposed adding a SHA-256 hash of raw file content as the second tier — inspired by rowboat's change detection pattern. SPEC-006 was then identified as already specifying a superior mechanism: BLAKE3 over normalised AST nodes, with the file Merkle root stored in `.zetl/index.json`.

**Rationale:** SPEC-006's Merkle root is strictly better than a raw-content SHA-256:

| Property | Raw SHA-256 | SPEC-006 Merkle root |
|----------|-------------|----------------------|
| Hash algorithm | SHA-256 (~500 MB/s) | BLAKE3 (~5 GB/s) |
| Hash input | Raw file bytes | Normalised AST nodes |
| Whitespace-only save | Hash changes → event | Hash unchanged → suppressed |
| SPL-aware suppression | No | Yes (ast_hash unchanged → no theory drift) |
| Already in cache | No (must add) | Yes (`.zetl/index.json`, SPEC-006) |

The watch loop loads the existing index cache on startup — file Merkle roots are already present. No additional cache fields are needed. The SPEC-006 scanner pipeline is already the code path for re-indexing; the watch loop simply invokes it for changed files rather than requiring a separate hashing step.

**Trade-offs:**
- ✅ No new cache fields or dependencies; reuses existing infrastructure
- ✅ Suppresses purely cosmetic saves at the AST level, not just byte level
- ✅ BLAKE3 is ~10× faster than SHA-256
- ✅ Consistent with the rest of zetl's pipeline
- ⚠️ Requires SPEC-006 to be merged before SPEC-008 can be implemented; watch mode is blocked on the Merkle tree branch

### ADR-015: NDJSON on stdout; Status on stderr

**Decision:** Graph events are emitted as NDJSON on stdout. Operational messages (index progress, shutdown notice, --exec errors) go to stderr.

**Context:** `zetl watch` must be composable with downstream consumers via pipes and `--exec`. If events and operational messages share stdout, consumers cannot reliably parse the event stream.

**Rationale:** The stdout/stderr separation is the Unix convention for separating data from diagnostics. It makes `zetl watch | grep dead_link` or `zetl watch | jq 'select(.event == "orphan_gained")'` work naturally. It also means redirecting stdout to a file captures a clean event log while terminal status remains visible. This is the same principle as `zetl`'s existing `-f json` / `-f table` split.

**Trade-offs:**
- ✅ Composable with standard Unix tooling
- ✅ Consistent with zetl's existing output model
- ⚠️ `--format table` emits human-readable event lines on stdout (not stderr), because table mode is inherently human-facing and the composability requirement relaxes; documented in CON-022

---

## 7. Contract Specifications

### CON-022: `zetl watch`

**Interface:** `zetl watch [<vault-path>] [--debounce <ms>] [--exec <cmd>] [--format json|table]`

**Argument rules:**
- `<vault-path>`: directory to watch; defaults to current directory
- `--debounce <ms>`: debounce window in milliseconds; default 150; minimum 10; maximum 5000
- `--exec <cmd>`: shell command invoked once per event with the event JSON on stdin
- `--format json` (default): NDJSON on stdout
- `--format table`: human-readable event lines on stdout; not intended for machine consumption

**Pre-conditions:**
- `<vault-path>` is an existing directory
- `zetl index` has been run at least once (loads cache on startup; if no cache, runs a full index pass before starting the watch loop)

**Post-conditions:**
- Exits 0 on graceful shutdown (SIGINT/SIGTERM)
- Exits non-zero on fatal error

**NDJSON event schema:**

```json
// index_ready — emitted once at startup
{ "event": "index_ready", "timestamp": "2026-02-24T10:00:00Z",
  "pages": 2000, "links": 8421, "orphans": 42, "dead_links": 7 }

// page_added / page_removed
{ "event": "page_added", "timestamp": "2026-02-24T10:01:03Z", "page": "ADR-043" }
{ "event": "page_removed", "timestamp": "2026-02-24T10:01:03Z", "page": "OldDraft" }

// link_added / link_removed
{ "event": "link_added", "timestamp": "2026-02-24T10:01:03Z",
  "from": "ADR-043", "to": "Benchmark: REST vs gRPC" }

// orphan_gained / orphan_resolved
{ "event": "orphan_gained", "timestamp": "2026-02-24T10:01:03Z",
  "page": "Benchmark: REST vs gRPC" }

// dead_link_added / dead_link_resolved
{ "event": "dead_link_added", "timestamp": "2026-02-24T10:01:03Z",
  "from": "ADR-043", "to": "Benchmark: REST vs gRPC" }

// index_updated — batch summary, emitted once per debounce window after per-change events
{ "event": "index_updated", "timestamp": "2026-02-24T10:01:03Z",
  "changed_pages": ["ADR-043", "Benchmark: REST vs gRPC"],
  "duration_ms": 38 }
```

**Error model (fatal, before watch loop starts):**

```json
{ "error": { "code": "VAULT_NOT_FOUND", "message": "Path '/bad/path' does not exist or is not a directory." } }
{ "error": { "code": "WATCHER_INIT_FAILED", "message": "Could not initialise file-system watcher: permission denied." } }
```

**--exec stderr logging (non-fatal, during watch loop):**

```
[zetl watch] --exec exited 1 for event dead_link_added (ADR-043 → Benchmark: REST vs gRPC)
```

**Implements:** REQ-053–061

**Verified by:** TEST-061–070

---

## 8. Test Specifications

### TEST-061: Watch Starts, Emits index_ready, Runs Until SIGINT

**Requirement:** REQ-053, REQ-057, REQ-060

**Preconditions:** Vault with 10 .md files; `zetl index` run previously

**Steps:**
1. Start `zetl watch --format json` in background; collect stdout
2. Verify first line is `{"event":"index_ready",...}` within 2 seconds
3. Send SIGINT; verify process exits 0
4. Verify no additional lines emitted after shutdown

---

### TEST-062: File Creation Detected

**Requirement:** REQ-054, REQ-058

**Preconditions:** Vault being watched; no file named `NewNote.md`

**Steps:**
1. Create `NewNote.md` with content `[[ExistingPage]]`
2. Verify within 500ms: `page_added` event for `NewNote`, `link_added` event `NewNote → ExistingPage`

---

### TEST-063: Debounce Coalesces Rapid Writes

**Requirement:** REQ-055

**Preconditions:** `zetl watch --debounce 200` running

**Steps:**
1. Write `Page.md` three times in 50ms intervals (simulating editor temp-file rename pattern)
2. Collect events; verify exactly one `index_updated` event is emitted (not three)
3. Verify `changed_pages` contains `["Page"]`

---

### TEST-064: Content-Identical Save Suppressed

**Requirement:** REQ-056

**Preconditions:** `zetl watch` running; `Note.md` already indexed

**Steps:**
1. Write `Note.md` with identical content (mtime changes, content does not)
2. Wait 500ms after debounce window
3. Verify zero graph events emitted (no `index_updated`, no link events)

---

### TEST-065: All Event Types Emitted Correctly

**Requirement:** REQ-057, REQ-058

**Preconditions:** Vault with `Source.md` containing `[[Target]]`; `Target.md` does not exist (dead link)

**Steps:**
1. Create `Target.md` → verify `page_added`, `dead_link_resolved`
2. Add `[[Source]]` to `Target.md` → verify `link_added`
3. Delete `Source.md` → verify `page_removed`, `link_removed`, `orphan_gained` (for Target), `dead_link_added` (Target → Source now broken)

---

### TEST-066: `--exec` Invoked Per Event

**Requirement:** REQ-059

**Preconditions:** `zetl watch --exec "cat >> /tmp/events.log"` running

**Steps:**
1. Create `NewPage.md`
2. Verify `/tmp/events.log` contains exactly the `page_added` and `index_updated` event JSON lines
3. Modify `--exec` to exit 1; verify zetl watch continues running and logs to stderr

---

### TEST-067: Graceful Shutdown Flushes Events

**Requirement:** REQ-060

**Preconditions:** `zetl watch` running; a file write occurs simultaneously with SIGTERM

**Steps:**
1. Write `Page.md`; immediately send SIGTERM
2. Verify the in-progress debounce batch completes (events emitted) before process exits
3. Verify exit code 0

---

### TEST-068: Vault Path Not Found

**Requirement:** REQ-061

**Steps:**
1. `zetl watch /nonexistent --format json`
2. Verify exit non-zero; stdout contains `{"error":{"code":"VAULT_NOT_FOUND",...}}`

---

### TEST-069: Event Latency

**Requirement:** NFR-020

**Preconditions:** Vault with 5,000 .md pages; watch running

**Steps:**
1. Record wall-clock time T0; write `Changed.md` with a new link
2. Record T1 when `link_added` event appears on stdout
3. Repeat 20 times; verify T1 - T0 ≤ 500ms at p95 (note: includes 150ms debounce)

---

### TEST-070: CPU Idle

**Requirement:** NFR-021

**Preconditions:** `zetl watch` running; no file changes for 60 seconds

**Steps:**
1. Sample CPU usage every 5 seconds for 60 seconds
2. Verify average CPU consumption ≤ 0.5%

---

## 9. Observability

### OBS-011: Watch Loop Diagnostics

**Signal:** Verbose lines on stderr when `--verbose` passed

```
[zetl watch] started  vault=/Users/akiko/notes  debounce=150ms  pages=2000
[zetl watch] event    file=ADR-043.md  mtime_changed=true  hash_changed=true  reindex_ms=12
[zetl watch] event    file=Draft.md    mtime_changed=true  hash_changed=false  suppressed=true
[zetl watch] batch    changed=2  events_emitted=4  total_ms=38
[zetl watch] shutdown signal=SIGINT  uptime_s=3847
```

**Purpose:** Diagnose latency (NFR-020), verify hash suppression (REQ-056), confirm debounce coalescing (REQ-055).

---

## 10. Traceability Matrix

| REQ     | NFR     | CON    | ADR    | TEST                   | OBS    |
| ------- | ------- | ------ | ------ | ---------------------- | ------ |
| REQ-053 | —       | CON-022 | —      | TEST-061               | OBS-011 |
| REQ-054 | —       | CON-022 | ADR-013 | TEST-062               | OBS-011 |
| REQ-055 | —       | CON-022 | ADR-013 | TEST-063               | OBS-011 |
| REQ-056 | —       | CON-022 | ADR-014, SPEC-006/ADR-009 | TEST-064          | OBS-011 |
| REQ-057 | —       | CON-022 | ADR-015 | TEST-061, TEST-065     | —      |
| REQ-058 | —       | CON-022 | —      | TEST-065               | —      |
| REQ-059 | —       | CON-022 | ADR-015 | TEST-066               | —      |
| REQ-060 | —       | CON-022 | —      | TEST-067               | OBS-011 |
| REQ-061 | —       | CON-022 | —      | TEST-068               | —      |
| —       | NFR-020 | —      | —      | TEST-069               | OBS-011 |
| —       | NFR-021 | —      | ADR-013 | TEST-070               | OBS-011 |
| —       | NFR-022 | —      | —      | —                      | OBS-011 |

---

## 11. Future Work

### 11.1 Persistent Daemon with Socket IPC

`zetl watch` as specified requires the caller to remain attached to its stdout. A future `zetl daemon` mode would run as a background process writing events to a Unix domain socket, allowing multiple consumers to subscribe simultaneously (e.g. both an editor extension and a reasoning agent). This is a significant architectural addition and is not needed for the initial use cases.

### 11.2 Event Log Persistence

Today events are ephemeral — if no consumer is attached when a file changes, the event is lost. A future `--log <path>` flag could append all events to an NDJSON file, providing a queryable audit trail. Combined with `zetl diff`, this would give complete temporal coverage: `zetl diff` for git-bounded history, the event log for intra-commit deltas.

### 11.3 SPL Watch Integration

When `zetl reason watch` is implemented, it would subscribe to `index_updated` events from the watch loop and re-run theory derivation over the affected subgraph. The Merkle infrastructure from SPEC-006 would serve as the efficient early-exit check (unchanged SPL hashes → skip re-derivation).

### 11.4 MCP Event Subscription

A natural extension of `zetl watch` is exposing the event stream as an MCP resource, allowing LLM clients (Claude Desktop, etc.) to subscribe to vault-change notifications. This builds on the rowboat-inspired MCP server work that is also a candidate for a future SPEC.
