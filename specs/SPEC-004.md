---
title: "SPEC-004: ztl — Distributed Vault Sync via Goblins Sidecar"
version: 0.1.0
status: draft
audience: agent, human
date: 2026-02-18
---

# SPEC-004: ztl — Distributed Vault Sync via Goblins Sidecar

## Information Table

| Field          | Value                                                    |
| -------------- | -------------------------------------------------------- |
| Document ID    | SPEC-004                                                 |
| Title          | ztl — Distributed Vault Sync via Goblins Sidecar       |
| Version        | 0.1.0                                                    |
| Status         | Draft                                                    |
| Author         | Agent (USDD Protocol v1.0.0)                             |
| Date           | 2026-02-18                                               |
| Audience       | Agent, Human                                             |
| Trace          | USDD Agent Protocol v1.0.0                               |
| Parent         | SPEC-001: ztl — Bi-directional Link Graph CLI           |
| Related        | SPEC-029: xm Agent Memory System, XM Whitepaper         |

---

## 1. Overview

ztl is a single-machine, read-only CLI tool. It indexes a local vault of Markdown files, builds a link graph, and answers queries. It never modifies files and has no awareness of other vaults on other machines.

This specification proposes extending ztl with **distributed vault synchronization** by adding a **Spritely Goblins sidecar** — a separate long-running process that handles networking, capability-based access control, and vault merging. The Goblins sidecar speaks OCapN (Object Capability Network) for secure peer-to-peer communication and uses sturdyrefs for persistent, delegatable capability tokens.

### 1.1 Core Insight

xm (SPEC-029) tried to be the entire stack: storage engine (Oxigraph), security layer (Goblins), networking (OCapN), and CLI — all in one. This created a monolithic system that's hard to ship and harder to debug.

The better decomposition:

| Concern | Owner | Why |
| --- | --- | --- |
| File parsing, graph building, queries | **ztl** (Rust) | Already built, fast, single binary |
| Networking, capabilities, sync protocol | **Goblins sidecar** (Guile Scheme) | OCapN is the best existing solution for capability-secured P2P |
| Conflict resolution, merge semantics | **Shared** (ztl proposes, sidecar negotiates) | Merge policy is a local decision informed by network state |
| File storage | **Filesystem** | Markdown files remain the source of truth |

ztl stays pure Rust, fast, and simple for local operations. The sidecar is optional — ztl works perfectly without it. When the sidecar is running, ztl gains the ability to share vault content with remote peers, receive updates, and merge changes.

### 1.2 Design Philosophy

1. **Files remain the source of truth.** The sidecar never bypasses ztl. All changes materialize as Markdown files in the vault directory. ztl re-indexes to discover them.
2. **Capabilities, not passwords.** Access to a remote vault (or subset of a vault) is granted by sharing a sturdyref — an unforgeable, attenuable, revocable token. No usernames, no passwords, no ACLs.
3. **Local-first, network-optional.** The sidecar is a separate process. If it's not running, ztl works as before. If the network is down, the sidecar works from its local state.
4. **Merge is explicit.** The sidecar never silently overwrites local files. Incoming changes are staged, conflicts are reported, and the user (or agent) decides.
5. **Minimal trust boundary.** ztl trusts the sidecar via a Unix domain socket capability. The sidecar trusts remote peers via OCapN sturdyrefs with attenuated permissions. No ambient authority exists at any layer.

### 1.3 Scope

**In scope:**

- Architecture for ztl ↔ Goblins sidecar communication (IPC protocol)
- Capability model for vault sharing (what granularity, what permissions)
- Sync protocol for vault merging (what data flows, how conflicts are detected)
- Merge strategies for Markdown files and link graphs
- Sidecar lifecycle management (`ztl sync start`, `ztl sync stop`)
- Research spikes for critical unknowns (Syrup in Rust, merge semantics)

**Out of scope:**

- Full CRDT implementation (defer to future SPEC if needed)
- Browser-based vault access (WebSocket netlayer)
- Multi-vault federation (3+ vaults; this SPEC covers bilateral sync)
- Real-time collaborative editing (Google Docs-style)
- xm RDF/SPARQL integration (xm and ztl are separate tools with different data models)

---

## 2. User Profiles

### 2.1 Solo Researcher with Multiple Machines

```
Role: Individual knowledge worker with a vault on laptop and desktop
Goals:
  - Keep both vaults in sync without manual file copying
  - Work offline on either machine, merge when connected
  - No cloud services, no third-party infrastructure
Constraints:
  - Both machines on same LAN, or reachable via Tor
  - Comfortable with CLI tooling
  - Wants automatic conflict detection, manual resolution
Daily workflow:
  1. Write notes on laptop during the day
  2. Connect to home network in the evening
  3. Run `ztl sync push` to send changes
  4. On desktop: `ztl sync pull` to receive and merge
  5. Resolve any conflicts flagged by ztl
```

### 2.2 Research Team Sharing Knowledge

```
Role: 2-5 researchers maintaining related but independent vaults
Goals:
  - Share specific topic folders with each other (not entire vaults)
  - Grant read-only access to some peers, read-write to others
  - Revoke access when a collaborator leaves the team
  - Track provenance (who wrote what, when did it arrive)
Constraints:
  - Machines may be on different networks (need NAT traversal)
  - Cannot install cloud infrastructure
  - Each person's vault has private sections that must never sync
Daily workflow:
  1. Alice shares concepts/ folder with Bob (read-write)
  2. Alice shares references/ folder with Carol (read-only)
  3. Bob creates a new note in concepts/, runs `ztl sync push`
  4. Alice runs `ztl sync pull`, sees Bob's new note merged into her vault
  5. Carol runs `ztl sync pull`, sees the new note (read-only copy)
  6. Alice revokes Carol's access: `ztl sync revoke <sturdyref>`
```

### 2.3 Agent-to-Agent Knowledge Handoff

```
Role: LLM agents on different machines sharing research findings
Goals:
  - Agent A discovers facts during a debugging session, stores as vault pages
  - Agent B on a different machine picks up where A left off
  - Knowledge transfer happens via structured Markdown, not ad-hoc text
  - Capabilities scoped per-task so agents cannot access unrelated vaults
Constraints:
  - Agents invoke ztl CLI non-interactively
  - JSON output required for programmatic consumption
  - Capability must be injectable as an environment variable or flag
Daily workflow:
  1. Orchestrator grants Agent A a scoped capability: `ztl sync grant --scope project/auth --permissions rw`
  2. Agent A writes findings to project/auth/ folder
  3. Agent A pushes: `ztl sync push --scope project/auth`
  4. Orchestrator grants Agent B the same sturdyref
  5. Agent B pulls: `ztl sync pull --scope project/auth`
  6. Agent B has full context, continues the work
```

### 2.4 Happy Paths

```
Happy Path: First-Time Vault Sharing (Two Peers)

Preconditions:
  - Alice and Bob both have ztl installed with the Goblins sidecar
  - Both sidecars are running (`ztl sync start`)
Steps:
  1. Alice generates a share capability:
     `ztl sync share --scope concepts/ --permissions read`
     → Returns: ocapn://...onion.../s/abc123...
  2. Alice sends the sturdyref to Bob out-of-band (email, chat, QR code)
  3. Bob adds the remote:
     `ztl sync remote add alice <sturdyref>`
  4. Bob pulls:
     `ztl sync pull alice`
     → Sidecar connects to Alice's sidecar via OCapN
     → Negotiates what's changed since last sync (or everything, if first sync)
     → Downloads pages, writes to bob-vault/concepts/ (or a configurable path)
     → ztl re-indexes automatically
  5. Bob queries:
     `ztl backlinks "Zettelkasten" -f table`
     → Sees links from both his own pages and Alice's synced pages
Postconditions:
  - Bob has a local copy of Alice's concepts/ folder
  - Subsequent `ztl sync pull alice` only transfers deltas
Failure modes:
  - Network unreachable → sidecar retries with exponential backoff, reports status
  - Sturdyref expired → `ztl sync pull` returns structured error with explanation
  - File conflict → staged in .ztl/conflicts/, `ztl sync conflicts` lists them
```

```
Happy Path: Merge with Conflict Detection

Preconditions:
  - Alice and Bob both have read-write access to a shared scope
  - Both have modified the same page since last sync
Steps:
  1. Alice pushes her changes: `ztl sync push`
  2. Bob pushes his changes: `ztl sync push`
     → Sidecar detects divergent modifications to "Knowledge Graph.md"
  3. Bob runs: `ztl sync pull alice`
     → Sidecar downloads Alice's version
     → Detects conflict (both modified since common ancestor)
     → Writes Alice's version to .ztl/conflicts/Knowledge Graph.md.alice
     → Writes conflict marker: .ztl/conflicts/manifest.json
     → Does NOT overwrite Bob's working copy
  4. Bob runs: `ztl sync conflicts`
     → Shows: Knowledge Graph.md (local: modified 2026-02-18T10:30, remote: alice, modified 2026-02-18T09:15)
  5. Bob resolves manually (or agent resolves programmatically)
  6. Bob marks resolved: `ztl sync resolve "Knowledge Graph.md"`
  7. Next push includes the resolved version
Postconditions:
  - No data lost — both versions preserved until explicit resolution
  - Conflict history recorded in .ztl/sync/log.json
Failure modes:
  - Conflict left unresolved → subsequent pulls for this file are blocked until resolved
  - Three-way conflict (3+ peers) → each remote version staged separately
```

---

## 3. Architecture

### 3.1 Component Architecture

```
Machine A                                    Machine B
┌─────────────────────┐                     ┌─────────────────────┐
│                     │                     │                     │
│  ┌───────────────┐  │                     │  ┌───────────────┐  │
│  │   ztl CLI    │  │                     │  │   ztl CLI    │  │
│  │   (Rust)      │  │                     │  │   (Rust)      │  │
│  │               │  │                     │  │               │  │
│  │ index, links, │  │                     │  │ index, links, │  │
│  │ backlinks,    │  │                     │  │ backlinks,    │  │
│  │ search, check │  │                     │  │ search, check │  │
│  │ + sync cmds   │  │                     │  │ + sync cmds   │  │
│  └───────┬───────┘  │                     │  └───────┬───────┘  │
│          │          │                     │          │          │
│     Unix Socket     │                     │     Unix Socket     │
│     (OCapN/CapTP)   │                     │     (OCapN/CapTP)   │
│          │          │                     │          │          │
│  ┌───────┴───────┐  │    Tor / TCP+TLS   │  ┌───────┴───────┐  │
│  │   Goblins     │  │    (OCapN/CapTP)   │  │   Goblins     │  │
│  │   Sidecar     │◄─┼───────────────────►│  │   Sidecar     │  │
│  │   (Guile)     │  │    sturdyref-      │  │   (Guile)     │  │
│  │               │  │    authenticated   │  │               │  │
│  │ - caps        │  │                     │  │ - caps        │  │
│  │ - sync proto  │  │                     │  │ - sync proto  │  │
│  │ - remotes     │  │                     │  │ - remotes     │  │
│  └───────┬───────┘  │                     │  └───────┬───────┘  │
│          │          │                     │          │          │
│   .ztl/sync/       │                     │   .ztl/sync/       │
│   ├── state.json    │                     │   ├── state.json    │
│   ├── bloblin/      │                     │   ├── bloblin/      │
│   ├── remotes.json  │                     │   ├── remotes.json  │
│   └── conflicts/    │                     │   └── conflicts/    │
│                     │                     │                     │
│  vault-a/           │                     │  vault-b/           │
│  ├── concepts/      │                     │  ├── concepts/      │
│  ├── projects/      │                     │  ├── references/    │
│  └── .ztl/         │                     │  └── .ztl/         │
│                     │                     │                     │
└─────────────────────┘                     └─────────────────────┘
```

### 3.2 Component Responsibilities

**ztl CLI (Rust)** — unchanged for local operations, extended with `sync` subcommand group:

| Responsibility | Details |
| --- | --- |
| Local vault operations | All existing commands (index, links, backlinks, search, check, etc.) |
| Sync command dispatch | `ztl sync {start,stop,status,share,pull,push,conflicts,resolve,...}` |
| IPC with sidecar | Connects to sidecar via Unix socket, sends commands, receives results |
| Conflict presentation | `ztl sync conflicts` shows staged conflicts in JSON/table format |
| Merge execution | Writes incoming files to vault, applies merge policy, stages conflicts |

**Goblins Sidecar (Guile Scheme)** — long-running daemon process:

| Responsibility | Details |
| --- | --- |
| OCapN networking | Listens on Unix socket (local) and Tor/TCP+TLS (remote) |
| Capability management | Creates, attenuates, revokes sturdyrefs for vault scopes |
| Remote registry | Tracks known peers and their sturdyrefs |
| Sync protocol | Negotiates with remote sidecars: what changed, transfer deltas |
| State persistence | Bloblin store for sync state, capability metadata, remote registry |
| Conflict detection | Compares local and remote file hashes/timestamps to detect divergence |

**Filesystem** — the actual storage:

| Location | Purpose |
| --- | --- |
| `vault/*.md` | Markdown source files (source of truth) |
| `.ztl/index.json` | ztl's cached link graph (existing) |
| `.ztl/sync/state.json` | Sync vector clock / last-known state per remote |
| `.ztl/sync/bloblin/` | Goblins persistence store (capabilities, session state) |
| `.ztl/sync/remotes.json` | Known remotes (name → sturdyref mapping) |
| `.ztl/sync/conflicts/` | Staged conflict files awaiting resolution |
| `.ztl/sync/log.json` | Append-only sync history for auditability |

### 3.3 IPC Protocol (ztl ↔ Sidecar)

```
ADR-001: IPC Protocol — Syrup over Unix Domain Socket

Status: Proposed

Context:
  ztl (Rust) needs to communicate with the Goblins sidecar (Guile).
  Three options were evaluated:

  A. Full OCapN/CapTP over Unix socket
     + Native sturdyrefs, promise pipelining, third-party handoffs
     - Requires implementing CapTP + Syrup in Rust (no existing crate)
     - Significant effort; CapTP spec is complex

  B. JSON-RPC over Unix socket with Guile adapter
     + Simple to implement in Rust (serde_json already a dependency)
     + Well-understood protocol
     - Loses OCapN benefits (pipelining, handoffs)
     - Two serialization formats in the system (JSON + Syrup)

  C. Syrup-only RPC over Unix socket (simplified CapTP subset)
     + Single serialization format (Syrup) across all layers
     + Simpler than full CapTP but extensible toward it
     + Syrup is trivially implementable (~200 lines of Rust)
     - Custom protocol (not a standard RPC)

Decision:
  Start with Option B (JSON-RPC over Unix socket) for the initial
  implementation. Migrate to Option C (Syrup RPC) once the protocol
  stabilizes, and eventually to Option A (full CapTP) if interoperability
  with other OCapN peers via the Rust process becomes necessary.

  Rationale: JSON-RPC minimizes initial implementation risk. The sidecar
  acts as a translator between the simple IPC protocol and OCapN
  internally. Migration path is clear and incremental.

Consequences:
  + Fast time-to-working-prototype
  + ztl's Rust implementation stays simple initially
  + Sidecar handles all OCapN complexity
  - Two serialization hops for remote operations (JSON → Syrup → OCapN)
  - Promise pipelining not available to ztl CLI initially
```

**IPC Message Format (JSON-RPC 2.0):**

```json
// ztl → sidecar: request
{"jsonrpc": "2.0", "method": "sync.pull", "params": {"remote": "alice", "scope": "concepts/"}, "id": 1}

// sidecar → ztl: response
{"jsonrpc": "2.0", "result": {"files_received": 3, "conflicts": 1, "conflict_files": ["Knowledge Graph.md"]}, "id": 1}

// sidecar → ztl: error
{"jsonrpc": "2.0", "error": {"code": -32001, "message": "Remote unreachable", "data": {"remote": "alice", "last_seen": "2026-02-17T15:30:00Z"}}, "id": 1}
```

### 3.4 Capability Model

```
ADR-002: Capability Granularity — Scope + Permissions

Status: Proposed

Context:
  Vault sharing requires access control. The question is what
  granularity of capabilities to support.

  Options evaluated:
  A. Whole vault (all or nothing)
  B. Folder-based scopes (share concepts/ but not journals/)
  C. Tag-based scopes (share all pages tagged #public)
  D. Page-level (share individual pages)

Decision:
  Folder-based scopes (Option B) as the primary mechanism, with
  optional glob patterns for flexibility.

  Rationale:
  - Folders are the natural organizational unit in Markdown vaults
  - Folder boundaries are unambiguous (no parsing needed)
  - Maps cleanly to filesystem operations (rsync-like delta transfer)
  - Users already organize by folder (concepts/, projects/, people/)
  - Tag-based (Option C) requires frontmatter extraction (future work)
  - Page-level (Option D) is too granular for practical use

  A scope is a glob pattern relative to the vault root:
    "concepts/"         — entire folder
    "concepts/AI*"      — pages starting with "AI" in concepts/
    "**"                — entire vault
    "projects/acme/"    — single project folder

  Permissions per scope:
    read   — pull files from this scope
    write  — push files to this scope (implies read)
    admin  — create sub-capabilities, revoke access

Consequences:
  + Simple, understandable mental model for users
  + Efficient sync (folder-level diffing)
  + Sturdyref encodes scope + permissions in a single token
  - Cannot share "all pages tagged #public" without folder co-location
  - Cross-folder pages require multiple capabilities
```

**Capability Lifecycle:**

```
1. CREATE — Alice generates a capability for a scope
   ztl sync share --scope "concepts/" --permissions read
   → ocapn://abc123.onion/s/7f8a9b2c...

2. DELEGATE — Alice sends the sturdyref to Bob (out-of-band)
   (email, chat, file, QR code, environment variable)

3. ATTENUATE — Bob can create a weaker capability from the one he holds
   ztl sync attenuate <sturdyref> --permissions read --expires 7d
   → ocapn://abc123.onion/s/new-weaker-ref...

4. USE — Bob connects and syncs using the capability
   ztl sync remote add alice <sturdyref>
   ztl sync pull alice

5. REVOKE — Alice revokes Bob's access
   ztl sync revoke <sturdyref>
   → Next pull by Bob fails with "capability revoked" error
```

### 3.5 Sync Protocol

The sync protocol is a delta-based file transfer with conflict detection. It does NOT require CRDTs — it uses a simpler model inspired by `git fetch` + `git merge`.

**Sync State:**

Each vault maintains a **vector clock** per remote per scope — a mapping of `(remote, scope, file_path) → (content_hash, mtime, sync_generation)`.

```json
{
  "remotes": {
    "alice": {
      "scopes": {
        "concepts/": {
          "last_sync": "2026-02-18T10:00:00Z",
          "generation": 42,
          "files": {
            "concepts/Zettelkasten.md": {
              "hash": "sha256:abc123...",
              "mtime": "2026-02-17T14:30:00Z",
              "size": 2048
            }
          }
        }
      }
    }
  }
}
```

**Pull Protocol:**

```
Bob's sidecar                              Alice's sidecar
     │                                          │
     │  1. SYNC_REQUEST                         │
     │  { scope: "concepts/",                   │
     │    last_generation: 42,                  │
     │    capability: <sturdyref> }             │
     │─────────────────────────────────────────►│
     │                                          │
     │                    2. Validate capability │
     │                    3. Compute delta since │
     │                       generation 42      │
     │                                          │
     │  4. SYNC_MANIFEST                        │
     │  { generation: 47,                       │
     │    added: ["concepts/New Note.md"],       │
     │    modified: ["concepts/Zettelkasten.md"],│
     │    deleted: ["concepts/Old Draft.md"],    │
     │    file_hashes: {...} }                  │
     │◄─────────────────────────────────────────│
     │                                          │
     │  5. Compare with local state             │
     │     - New Note.md → clean add            │
     │     - Zettelkasten.md → conflict check   │
     │     - Old Draft.md → clean delete        │
     │                                          │
     │  6. FILE_REQUEST                         │
     │  { files: ["concepts/New Note.md",        │
     │            "concepts/Zettelkasten.md"] }  │
     │─────────────────────────────────────────►│
     │                                          │
     │  7. FILE_DATA                            │
     │  { files: [                               │
     │    { path: "concepts/New Note.md",        │
     │      content: "# New Note\n...",          │
     │      hash: "sha256:...",                  │
     │      mtime: "..." },                      │
     │    { path: "concepts/Zettelkasten.md",    │
     │      content: "# Zettelkasten\n...",      │
     │      hash: "sha256:...",                  │
     │      mtime: "..." }                       │
     │  ] }                                      │
     │◄─────────────────────────────────────────│
     │                                          │
     │  8. Apply changes locally:               │
     │     - Write New Note.md to vault         │
     │     - Zettelkasten.md: detect conflict   │
     │       → stage in .ztl/sync/conflicts/   │
     │     - Delete Old Draft.md (or archive)   │
     │  9. Update sync state                    │
     │  10. Signal ztl to re-index             │
     │                                          │
```

**Push Protocol:** Symmetric to pull — Bob's sidecar sends his changes to Alice's sidecar. Alice's sidecar validates Bob's write capability before accepting.

### 3.6 Merge Strategy

```
ADR-003: Merge Strategy — Three-Way with Conflict Staging

Status: Proposed

Context:
  When both peers modify the same file, we need a merge strategy.
  Options:
  A. Last-writer-wins (timestamp-based)
  B. CRDT-based automatic merge
  C. Three-way merge (common ancestor + two versions)
  D. Conflict staging (never auto-merge, always ask user)

Decision:
  Hybrid of C and D:
  - If only one side modified the file → fast-forward (no conflict)
  - If both sides modified, attempt three-way merge using the
    common ancestor (stored in sync state)
  - If three-way merge succeeds cleanly → apply automatically
  - If three-way merge has conflicts → stage both versions,
    report to user, block further sync of that file until resolved

  The common ancestor is the file content at the last successful
  sync point, stored as a hash in the sync state. The actual
  content can be reconstructed from either peer's history or
  from a local snapshot.

  For the initial implementation, skip three-way merge entirely
  and always stage conflicts (pure Option D). Add three-way merge
  in a future iteration once the basic sync protocol is proven.

Consequences:
  + No data loss — both versions always preserved
  + Simple initial implementation (no merge algorithm needed)
  + Users/agents retain full control over conflict resolution
  - More manual work for highly collaborative vaults
  - Cannot achieve fully automatic sync without future CRDT work
```

---

## 4. Requirements

### 4.1 Functional Requirements

```
REQ-001: Sidecar Lifecycle

The CLI SHALL provide commands to start, stop, and query the status
of the Goblins sidecar process:
  - `ztl sync start` — launch sidecar daemon
  - `ztl sync stop` — gracefully terminate sidecar
  - `ztl sync status` — report sidecar state (running/stopped,
    connected remotes, pending syncs)

The sidecar SHALL persist its state across restarts via Bloblin.

Trace:
  - TEST-001
  - CON-001
```

```
REQ-002: Capability Creation and Sharing

The CLI SHALL allow creating scoped capabilities for vault sharing:
  - `ztl sync share --scope <glob> --permissions <read|write|admin>`
  - Returns a sturdyref URI that encodes scope, permissions, and
    the sidecar's network address

The capability SHALL be a valid OCapN sturdyref resolvable by any
OCapN-compatible peer.

Trace:
  - TEST-002
  - CON-002
  - ADR-002
```

```
REQ-003: Remote Management

The CLI SHALL allow registering, listing, and removing remote peers:
  - `ztl sync remote add <name> <sturdyref>`
  - `ztl sync remote list`
  - `ztl sync remote remove <name>`

Remote metadata (name, sturdyref, last sync time, status) SHALL be
persisted in .ztl/sync/remotes.json.

Trace:
  - TEST-003
  - CON-003
```

```
REQ-004: Pull (Receive Changes)

The CLI SHALL allow pulling changes from a registered remote:
  - `ztl sync pull <remote> [--scope <glob>]`

Pull SHALL:
  a) Connect to the remote sidecar via OCapN
  b) Request a delta since the last known sync generation
  c) Download new and modified files
  d) Write non-conflicting files directly to the vault
  e) Stage conflicting files in .ztl/sync/conflicts/
  f) Update the sync state (vector clock, generation)
  g) Trigger ztl re-indexing of affected files

Trace:
  - TEST-004
  - CON-004
```

```
REQ-005: Push (Send Changes)

The CLI SHALL allow pushing local changes to a registered remote:
  - `ztl sync push <remote> [--scope <glob>]`

Push SHALL:
  a) Compute which local files have changed since last sync
  b) Connect to the remote sidecar via OCapN
  c) Send the delta (manifest + file contents)
  d) The remote sidecar validates write capability before accepting
  e) Report success/failure and any conflicts detected by the remote

Trace:
  - TEST-005
  - CON-005
```

```
REQ-006: Conflict Detection and Resolution

The system SHALL detect conflicts when both peers modify the same
file between sync points.

Conflicts SHALL be staged in .ztl/sync/conflicts/ with a manifest:
  {
    "file": "concepts/Knowledge Graph.md",
    "local_hash": "sha256:...",
    "remote_hash": "sha256:...",
    "ancestor_hash": "sha256:...",
    "remote_name": "alice",
    "detected_at": "2026-02-18T10:30:00Z"
  }

The CLI SHALL provide:
  - `ztl sync conflicts` — list all unresolved conflicts
  - `ztl sync resolve <file> [--accept local|remote|merged]`

Conflicts SHALL block further sync of the affected file until resolved.

Trace:
  - TEST-006
  - CON-006
```

```
REQ-007: Capability Attenuation and Revocation

Capability holders SHALL be able to create weaker child capabilities:
  - `ztl sync attenuate <sturdyref> --permissions read [--expires <duration>]`

Capability creators SHALL be able to revoke issued capabilities:
  - `ztl sync revoke <sturdyref>`

Revocation SHALL take effect on the next connection attempt by the
revoked peer.

Trace:
  - TEST-007
  - CON-007
```

```
REQ-008: Sync Auditability

All sync operations SHALL be recorded in an append-only log at
.ztl/sync/log.json with:
  - Timestamp
  - Operation (pull/push/share/revoke)
  - Remote name
  - Scope
  - Files transferred
  - Conflicts detected
  - Outcome (success/failure/partial)

Trace:
  - TEST-008
  - OBS-001
```

### 4.2 Non-Functional Requirements

```
NFR-001: Sync Latency

A pull/push of ≤ 10 changed files (average 5KB each) SHALL complete
WITHIN 5 seconds on a LAN (TCP+TLS) and WITHIN 30 seconds over
Tor, excluding conflict resolution time.
```

```
NFR-002: Sidecar Resource Usage

The sidecar process SHALL consume ≤ 100MB RSS memory and ≤ 1% CPU
when idle (no active sync operations).
```

```
NFR-003: Offline Resilience

If the sidecar cannot reach a remote peer, it SHALL:
  a) Queue the sync request
  b) Retry with exponential backoff (1s, 2s, 4s, ..., max 5min)
  c) Report status via `ztl sync status`
  d) Complete the sync when connectivity is restored

No data SHALL be lost due to network interruption.
```

```
NFR-004: Backward Compatibility

The sync subsystem SHALL be fully additive. Existing ztl commands
SHALL work identically whether or not the sidecar is running. The
sidecar SHALL NOT modify the vault index format or any existing
.ztl/ files.
```

```
NFR-005: Transport Security

All remote communication SHALL be encrypted. Tor onion services
provide end-to-end encryption by default. TCP+TLS SHALL use
certificate pinning tied to the sturdyref's cryptographic identity.
No plaintext file content SHALL traverse the network.
```

---

## 5. Contract Specifications (CLI Interface)

```
CON-001: ztl sync start / stop / status

ztl sync start [OPTIONS]
ztl sync stop
ztl sync status

Options:
  --transport <TYPE>   Network transport: tor | tcp-tls | both [default: tor]
  --port <PORT>        TCP+TLS listen port [default: 4185]

Start launches the Goblins sidecar as a background daemon.
Stop sends SIGTERM and waits for graceful shutdown.
Status reports sidecar state.

Example output (JSON, status):
{
  "running": true,
  "pid": 12345,
  "transport": ["tor", "tcp-tls"],
  "tor_address": "abc123...onion",
  "tcp_address": "192.168.1.50:4185",
  "remotes_connected": 2,
  "pending_syncs": 0,
  "uptime_seconds": 3600
}

Exit codes:
  0  Success
  1  Sidecar not running (for status/stop)
  2  Failed to start (port in use, Guile not found, etc.)

Implements: REQ-001
Verified by: TEST-001
```

```
CON-002: ztl sync share

ztl sync share --scope <GLOB> --permissions <PERMS> [OPTIONS]

Arguments:
  --scope <GLOB>        Folder or glob pattern relative to vault root
  --permissions <PERMS> Comma-separated: read, write, admin

Options:
  --expires <DURATION>  Capability expiration (e.g., 7d, 24h, 2026-03-01)
  --label <TEXT>        Human-readable label for this capability

Example output (JSON):
{
  "sturdyref": "ocapn://abc123...onion/s/7f8a9b2c...",
  "scope": "concepts/",
  "permissions": ["read"],
  "expires": "2026-02-25T00:00:00Z",
  "label": "Bob read access to concepts"
}

Implements: REQ-002
Verified by: TEST-002
```

```
CON-003: ztl sync remote {add,list,remove}

ztl sync remote add <NAME> <STURDYREF>
ztl sync remote list [--json]
ztl sync remote remove <NAME>

Example output (JSON, list):
{
  "remotes": [
    {
      "name": "alice",
      "scope": "concepts/",
      "permissions": ["read"],
      "last_sync": "2026-02-18T10:00:00Z",
      "status": "connected"
    }
  ]
}

Implements: REQ-003
Verified by: TEST-003
```

```
CON-004: ztl sync pull

ztl sync pull <REMOTE> [--scope <GLOB>] [--dry-run]

Pulls changes from a registered remote.
--dry-run shows what would change without applying.

Example output (JSON):
{
  "remote": "alice",
  "scope": "concepts/",
  "files_received": 3,
  "files_added": ["concepts/New Note.md"],
  "files_modified": ["concepts/Zettelkasten.md"],
  "files_deleted": [],
  "conflicts": ["concepts/Knowledge Graph.md"],
  "generation": 47,
  "elapsed_ms": 1230
}

Exit codes:
  0  Success (may include conflicts)
  1  Remote unreachable
  2  Capability rejected (expired, revoked, insufficient permissions)

Implements: REQ-004
Verified by: TEST-004
```

```
CON-005: ztl sync push

ztl sync push <REMOTE> [--scope <GLOB>] [--dry-run]

Pushes local changes to a registered remote.

Example output (JSON):
{
  "remote": "alice",
  "scope": "concepts/",
  "files_sent": 2,
  "files_added": ["concepts/My New Page.md"],
  "files_modified": ["concepts/Zettelkasten.md"],
  "accepted": true,
  "generation": 48,
  "elapsed_ms": 890
}

Exit codes:
  0  Accepted
  1  Remote unreachable
  2  Capability rejected (no write permission)
  3  Remote reported conflicts

Implements: REQ-005
Verified by: TEST-005
```

```
CON-006: ztl sync conflicts / resolve

ztl sync conflicts [--json]
ztl sync resolve <FILE> [--accept local|remote]

Example output (JSON, conflicts):
{
  "conflicts": [
    {
      "file": "concepts/Knowledge Graph.md",
      "local_modified": "2026-02-18T10:30:00Z",
      "remote_modified": "2026-02-18T09:15:00Z",
      "remote_name": "alice",
      "local_version": ".ztl/sync/conflicts/Knowledge Graph.md.local",
      "remote_version": ".ztl/sync/conflicts/Knowledge Graph.md.alice"
    }
  ],
  "count": 1
}

Implements: REQ-006
Verified by: TEST-006
```

```
CON-007: ztl sync attenuate / revoke

ztl sync attenuate <STURDYREF> --permissions <PERMS> [--expires <DURATION>]
ztl sync revoke <STURDYREF>

Example output (JSON, attenuate):
{
  "original": "ocapn://...onion/s/original...",
  "attenuated": "ocapn://...onion/s/weaker...",
  "permissions": ["read"],
  "expires": "2026-02-25T00:00:00Z"
}

Implements: REQ-007
Verified by: TEST-007
```

---

## 6. Test Specifications

```
TEST-001: Sidecar Lifecycle

Scenario: Start, query, and stop the sidecar
Given: No sidecar is running
When: `ztl sync start` is run
Then:
  - A background process is launched
  - `ztl sync status` reports running=true with PID
  - The process listens on a Unix socket
When: `ztl sync stop` is run
Then:
  - The process terminates gracefully
  - `ztl sync status` reports running=false
  - State is persisted to .ztl/sync/bloblin/

Verifies: REQ-001
```

```
TEST-002: Capability Creation

Scenario: Create a scoped read capability
Given: Sidecar is running
When: `ztl sync share --scope "concepts/" --permissions read --expires 7d`
Then:
  - Returns a valid sturdyref URI
  - The sturdyref encodes scope=concepts/, permissions=read, expiry=+7d
  - The capability is registered in the sidecar's capability store

Verifies: REQ-002
```

```
TEST-003: Remote Registration

Scenario: Add, list, and remove a remote
Given: Sidecar is running, Alice's sturdyref is known
When: `ztl sync remote add alice <sturdyref>`
Then: `ztl sync remote list` shows alice with scope and permissions
When: `ztl sync remote remove alice`
Then: `ztl sync remote list` shows empty list

Verifies: REQ-003
```

```
TEST-004: Pull — Clean Sync

Scenario: Pull new files from a remote with no conflicts
Given: Alice's vault has concepts/NewNote.md that Bob doesn't have
When: Bob runs `ztl sync pull alice`
Then:
  - concepts/NewNote.md appears in Bob's vault
  - Bob's link graph includes the new page after re-indexing
  - Sync state updated with new generation number
  - No conflicts reported

Verifies: REQ-004
```

```
TEST-005: Push — Send Changes

Scenario: Push local changes to a remote
Given: Bob has write capability for Alice's concepts/ scope
       Bob creates concepts/BobNote.md locally
When: Bob runs `ztl sync push alice`
Then:
  - Alice's sidecar receives and writes concepts/BobNote.md
  - Push reports files_sent=1, accepted=true

Scenario: Push rejected (read-only capability)
Given: Bob has only read capability
When: Bob runs `ztl sync push alice`
Then:
  - Exit code 2, error: "capability does not permit write"

Verifies: REQ-005
```

```
TEST-006: Conflict Detection and Resolution

Scenario: Both peers modify the same file
Given: Alice and Bob both modify concepts/Zettelkasten.md after last sync
When: Bob runs `ztl sync pull alice`
Then:
  - Bob's working copy is NOT overwritten
  - Alice's version is staged in .ztl/sync/conflicts/Zettelkasten.md.alice
  - `ztl sync conflicts` reports 1 conflict with both versions' paths
When: Bob runs `ztl sync resolve "concepts/Zettelkasten.md" --accept local`
Then:
  - Conflict is removed from .ztl/sync/conflicts/
  - Bob's version is kept, marked as the resolution
  - Next push sends Bob's resolved version

Verifies: REQ-006
```

```
TEST-007: Capability Attenuation and Revocation

Scenario: Attenuate a capability
Given: Alice holds an admin capability for concepts/
When: Alice runs `ztl sync attenuate <sturdyref> --permissions read --expires 7d`
Then: Returns a new sturdyref with reduced permissions and expiry

Scenario: Revoke a capability
Given: Alice previously shared a capability with Bob
When: Alice runs `ztl sync revoke <sturdyref>`
Then: Bob's next `ztl sync pull` fails with "capability revoked"

Verifies: REQ-007
```

```
TEST-008: Sync Auditability

Scenario: All operations are logged
Given: A series of sync operations (share, pull, push, resolve)
When: .ztl/sync/log.json is inspected
Then:
  - Each operation has a timestamped entry
  - Files transferred are listed
  - Conflicts are recorded
  - Outcomes (success/failure) are captured

Verifies: REQ-008
```

---

## 7. Observability

```
OBS-001: Sync Log

All sync operations SHALL be logged to .ztl/sync/log.json in
append-only JSON-lines format for post-hoc auditability.
```

```
OBS-002: Sidecar Health

`ztl sync status` SHALL report:
  - Process uptime
  - Memory usage
  - Connected remotes and their last-seen timestamps
  - Pending/queued operations
  - Error counts since startup
```

---

## 8. Research Spikes Required

Before implementation, the following unknowns require timeboxed prototypes:

### 8.1 Spike: Syrup Serialization in Rust

```
Hypothesis: Syrup can be implemented in Rust in ≤ 1 day with full
            round-trip compatibility with Guile's Syrup implementation.

Approach:
  - Implement Syrup encode/decode for all types (Boolean, Integer,
    String, ByteString, Symbol, Record, Sequence, Set, Dictionary)
  - Test round-trip with Guile: Rust encodes → Guile decodes → verify
  - Publish as standalone crate (syrup-rs)

Timebox: 1 day
Success metric: 100% round-trip compatibility on test vectors
Exit criteria: If Syrup has undocumented edge cases that prevent
               clean round-trip, fall back to JSON-RPC permanently
```

### 8.2 Spike: Goblins Sidecar Prototype

```
Hypothesis: A minimal Goblins sidecar can expose a vault-sync actor
            over Unix domain socket AND Tor onion service simultaneously,
            with Bloblin persistence, in ≤ 100 lines of Guile.

Approach:
  - Write a minimal sidecar using Goblins 0.17.0
  - Actor that accepts "list-files" and "get-file" messages
  - Register on both Unix socket and Tor netlayers
  - Test: Rust process connects via Unix socket, retrieves file list
  - Test: Remote Guile process connects via Tor, retrieves file list

Timebox: 2 days
Success metric: Both local and remote peers can list and retrieve
                files from the sidecar
Exit criteria: If Goblins cannot run both netlayers simultaneously,
               or if macOS compatibility issues block Tor, document
               the limitation and propose alternatives
```

### 8.3 Spike: Conflict Detection Accuracy

```
Hypothesis: SHA-256 content hashing + mtime tracking is sufficient
            to detect all conflicts without false positives/negatives
            for vaults with ≤ 10,000 files.

Approach:
  - Create two test vaults with known overlapping content
  - Simulate 100 concurrent edit scenarios
  - Verify conflict detection catches all true conflicts
  - Verify no false positives (identical edits flagged as conflicts)

Timebox: 0.5 day
Success metric: 100% true positive rate, 0% false positive rate
Exit criteria: If content hashing alone is insufficient, evaluate
               adding line-level diffing or git integration
```

---

## 9. Implementation Roadmap

| Phase | Deliverable | Effort | Dependencies |
| --- | --- | --- | --- |
| **0. Spikes** | Syrup-rs crate, sidecar prototype, conflict detection test | 3-4 days | Guile 3.0 + Goblins 0.17.0 installed |
| **1. Local IPC** | ztl ↔ sidecar communication over Unix socket (JSON-RPC) | 2-3 days | Spike results |
| **2. Sync core** | Pull/push protocol with conflict staging (LAN only, TCP+TLS) | 3-5 days | Phase 1 |
| **3. Capabilities** | Share, attenuate, revoke commands with folder scopes | 2-3 days | Phase 2 |
| **4. Tor transport** | Add Tor onion service for internet-wide sync | 1-2 days | Phase 3 |
| **5. Polish** | Conflict resolution UX, dry-run, audit log, error handling | 2-3 days | Phase 4 |

Total estimated effort: **13-20 days** (including spikes).

---

## 10. Open Questions

1. **Should the sidecar be bundled with ztl or installed separately?**
   Bundling (via `ztl sync install`) simplifies setup but adds a Guile dependency to the ztl distribution. Separate installation keeps ztl pure Rust but requires users to install Guile and Goblins independently. Recommendation: provide an install script (`ztl sync install`) that bootstraps the Guile environment, but keep it optional.

2. **Should deleted files propagate via sync?**
   If Alice deletes a page, should Bob's copy be deleted on next pull? Dangerous — could cause data loss. Options: (a) never propagate deletes, (b) propagate as "tombstone" markers that Bob must confirm, (c) configurable per-remote. Recommendation: option (b), tombstones with explicit confirmation.

3. **How should the sidecar handle vault restructuring (renames, moves)?**
   If Alice renames `concepts/AI.md` to `concepts/Artificial Intelligence.md`, the sync protocol sees a delete + add. This loses rename tracking and creates unnecessary conflict risk. Options: (a) accept the limitation, (b) add git-style rename detection by content similarity. Recommendation: accept the limitation initially, revisit if users report friction.

4. **Should sync support partial file updates (diffs) or always transfer whole files?**
   Whole-file transfer is simpler and sufficient for Markdown files (typically < 50KB). Diff-based transfer saves bandwidth for large files but adds complexity. Recommendation: whole-file transfer initially, add diff support if vaults with large files become a common use case.

5. **What happens when both Goblins and ztl's cache try to write to .ztl/ simultaneously?**
   The sidecar writes to `.ztl/sync/` while ztl writes to `.ztl/index.json`. These are separate subtrees, so no conflict. The sidecar should NOT modify `index.json`; instead it signals ztl to re-index after writing synced files. Mechanism: write a trigger file (`.ztl/sync/.reindex`) that ztl checks on next invocation.

6. **Is Goblins mature enough for this use case?**
   Goblins 0.17.0 explicitly warns against production use. However, ztl itself is a 0.1.0 tool — both are in early development. The sidecar architecture isolates Goblins instability: if it crashes, ztl continues working locally. The spikes in §8 will surface any showstopper issues before committing to the full implementation.

---

**END OF SPEC-004**
