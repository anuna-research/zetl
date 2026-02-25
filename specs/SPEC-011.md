---
title: "SPEC-011: zetl — Capability-Based Knowledge Base Federation via OcapN"
version: 0.1.0
status: draft
audience: agent, human
date: 2026-02-25
---

# SPEC-011: zetl — Capability-Based Knowledge Base Federation via OcapN

## Information Table

| Field          | Value                                                              |
| -------------- | ------------------------------------------------------------------ |
| Document ID    | SPEC-011                                                           |
| Title          | zetl — Capability-Based Knowledge Base Federation via OcapN        |
| Version        | 0.1.0                                                              |
| Status         | Draft                                                              |
| Author         | Agent (USDD Protocol v1.0.0)                                       |
| Date           | 2026-02-25                                                         |
| Audience       | Agent, Human                                                       |
| Trace          | USDD Agent Protocol v1.0.0                                         |
| Parent         | SPEC-001: zetl — Bi-directional Link Graph CLI                     |
| Related        | SPEC-004: Distributed Vault Sync via Goblins Sidecar              |
| Dependencies   | SPEC-006: Content-Addressed Merkle Tree, ed25519-dalek, blake3, spake2, bip39 |

---

## 1. Overview

Two private vaults, each with their own zetl index, selectively share portions of their link graphs via OcapN capability-based security. Joining is a graph overlay — no vault files are modified, no central authority exists. By default only index metadata is shared; the read-content tier optionally materializes peer files as a local cache. Unjoining revokes the overlay cleanly. This specification is informed by the xm experiment (SPEC-029) and supersedes the sidecar approach in SPEC-004.

### 1.1 Core Insight

SPEC-004 proposed a Guile Scheme Goblins sidecar for distributed vault sync. The xm experiment (SPEC-029) validated the capability model — graph facets, sturdyrefs, gatekeeper query rewriting, pubsub sync — but revealed that a polyglot sidecar adds build complexity, operational burden (two processes, two runtimes), and an FFI boundary.

The better decomposition:

| Concern | SPEC-004 Approach | SPEC-011 Approach |
| --- | --- | --- |
| Capability security | Goblins sidecar (Guile) | Native Rust (ed25519 tokens) |
| Networking | OCapN via Goblins | OCapN CapTP in Rust (incremental) |
| Sync scope | Full file sync (read + write) | Read-only sharing (graph overlay; opt-in content tier) |
| Merge semantics | Three-way merge, conflict staging | No merge — cached peer indexes, not files |
| Process model | Two processes (zetl + sidecar) | Single binary (zetl) |
| Identity | Sturdyrefs (live references) | Self-contained signed tokens (offline-verifiable) |

zetl stays a single Rust binary. Federation is additive — if no peers are configured, zetl works identically to today. When peers are added, the link graph gains remote nodes mediated by capabilities.

### 1.2 Design Philosophy

1. **Graph overlay, not file sync.** Joining shares index metadata (page names, links, Merkle roots), not file contents by default. Files remain the source of truth in their home vault. Content sharing is an opt-in capability tier.
2. **Capabilities, not passwords.** Access is conferred by holding a signed token — an unforgeable, attenuable, revocable bearer credential. No usernames, no passwords, no ACLs, no user registry.
3. **Local-first, network-optional.** Peer caches work offline. Staleness is tolerated and reported, not blocked on. The network is consulted when available, never required.
4. **Show all matches.** When a wikilink target exists in multiple vaults, all matches are surfaced with provenance. The user sees the full picture and decides — no silent resolution.
5. **Merkle-verified integrity.** The existing BLAKE3 Merkle tree (SPEC-006) provides efficient change detection, content integrity verification, and cross-vault drift detection without additional infrastructure.

### 1.3 Scope

**In scope:**

- Vault identity model (Ed25519 keypair generation and management)
- Capability token format (self-contained, signed, offline-verifiable)
- Capability lifecycle (creation, attenuation, delegation, revocation)
- Peer management (`zetl peer add`, `remove`, `sync`, `list`)
- Joined link graph construction (local + peer caches)
- Multi-match link resolution (all vaults queried, all matches surfaced)
- Merkle-based delta sync (efficient peer cache refresh)
- Content integrity verification (signed vault roots)
- Cross-vault drift detection (remote page changed since local reference)
- Tiered capability operations (graph, read-index, read-content)
- Tiered local persistence (JSON metadata at graph/read-index; file materialization at read-content)
- Phrase-based capability exchange (SPAKE2 + BIP39 for human-friendly sharing)

**Out of scope:**

- Write-path federation (bidirectional file sync — deferred to SPEC-004 evolution)
- Conflict resolution and merge strategies (no files are written to vault)
- CRDT-based collaborative editing
- Browser-based vault access
- Real-time collaborative editing
- xm RDF/SPARQL integration (xm and zetl remain separate tools)
- Full OCapN CapTP implementation (incremental; basic federation works with signed tokens alone)

### 1.4 Relation to SPEC-004

SPEC-004 covers **read-write sync** (push, pull, merge, conflict resolution) via a Goblins sidecar. This spec covers **read-only federation** (index sharing, graph joining) via native Rust.

They are complementary:

```
SPEC-011 (this spec)         SPEC-004 (future)
─────────────────────        ─────────────────────
Read-only sharing        →   Read-write file sync
Graph overlay            →   Vault merge
Signed tokens            →   Live OCapN sturdyrefs
Single binary            →   Optional sidecar for write-path
```

SPEC-011 can be implemented and shipped independently. SPEC-004's write-path can build on SPEC-011's identity and capability primitives later.

### 1.5 Experiment-First Assessment

Per USDD decision framework:

| Factor | Rating | Rationale |
| --- | --- | --- |
| Problem clarity | Medium | Design explored but details remain |
| Interface stability | Low | First iteration of peer commands |
| Technical novelty | High | OcapN primitives in Rust |
| Data certainty | Medium | Index format is stable, peer cache is new |
| Performance risk | Medium | Network round-trips, Merkle tree walks |
| Safety/regulatory risk | Medium | Cryptographic tokens (Ed25519 signing) |
| Reversibility | High | Additive feature, no existing behavior changes |

Two factors are High → research spikes SHOULD precede implementation of core cryptographic and networking primitives. Graph composition and link resolution can be prototyped with local-only federation (two vaults on one machine, no network).

---

## 2. User Profiles

### 2.1 Solo Researcher with Multiple Machines

```
Role: Academic with a home vault and a lab vault
Goals:
  - Cross-reference notes across machines
  - Keep lab-specific notes private from personal vault
  - Work offline — both machines not always on simultaneously
  - zetl links and zetl backlinks span both vaults
Constraints:
  - Both machines on same LAN, or reachable via Tor
  - Comfortable with CLI tooling
  - Tolerates stale caches when offline
Daily workflow:
  1. On lab machine: write notes, run zetl index
  2. Grant read capability for research/ folder
  3. On home machine: zetl peer add lab <token>
  4. Run zetl index — link graph includes lab pages
  5. zetl backlinks "Attention Mechanism" shows results from both vaults
  6. Offline: cached data used, staleness warning shown
```

### 2.2 Research Team

```
Role: 3-5 researchers sharing a project knowledge base
Goals:
  - Each member exposes their shared/ folder
  - Each member keeps drafts/ private
  - Team lead can see everything
  - Members join and leave over time
Constraints:
  - No central server
  - Each person's vault has private sections that must never be visible
  - Members may be on different networks
Daily workflow:
  1. Team lead creates root capability (admin, scope: **)
  2. Lead attenuates per member:
     - Alice: graph+read-index, scope: shared/*
     - Bob: graph only, scope: shared/public/*
  3. Members run zetl peer add <teammate> <token>
  4. zetl links spans all shared folders
  5. zetl check detects dead links to revoked peers
  6. When Carol leaves: lead revokes her token
  7. Carol's next sync attempt fails; other members' caches of Carol's
     pages are marked stale, then purged after configurable period
```

### 2.3 Agent-to-Agent Knowledge Handoff

```
Role: LLM agent session handing context to a successor session
Goals:
  - New session sees previous session's findings without re-reading vault
  - Capability is self-contained (no live connection needed)
  - Scoped to the working set, not the entire vault
Constraints:
  - Sessions are ephemeral
  - Agents invoke zetl CLI non-interactively (JSON output)
  - Capability must be injectable as flag or env var
Daily workflow:
  1. Agent A finishes research, creates capability:
     zetl peer grant --scope "project/auth/*" --ops read-content
  2. Orchestrator passes token to Agent B
  3. Agent B imports: zetl peer add prev-session --cap <token>
  4. Agent B runs zetl index — cached peer index provides immediate context
  5. Agent B queries: zetl backlinks "OAuth2" --format json
     → Results include pages from prev-session peer
  6. Session ends; peer cache can be purged
```

### 2.4 Happy Paths

```
Happy Path: First-Time Vault Joining (Two Peers)

Preconditions:
  - Alice and Bob both have zetl installed with vault identity (zetl init)
  - Both vaults are indexed (zetl index)

Steps:
  1. Alice generates a capability:
     zetl peer grant --scope "research/*" --ops graph
     → Outputs: signed capability token (JSON)
  2. Alice sends the token to Bob out-of-band (email, file, QR code)
  3. Bob registers the peer:
     zetl peer add alice --cap ./alice-cap.json
     → Validates token signature
     → Attempts initial fetch from Alice's vault
     → Caches Alice's page manifest (research/ pages with links + Merkle roots)
  4. Bob runs zetl index:
     → Link graph includes Alice's research/ pages as remote nodes
     → Bob's [[Topic A]] (previously dead link) now resolves to alice:research/Topic A.md
  5. Bob queries:
     zetl backlinks "Topic A"
     → Shows backlinks from both Bob's vault and Alice's research/ pages
  6. Bob runs zetl check:
     → No dead links for targets that resolve via alice peer
     → Shadow warning if "Topic A" exists in both vaults

Postconditions:
  - Bob's link graph spans both vaults
  - Alice's files are untouched — only index metadata was shared
  - Subsequent zetl index refreshes only fetch deltas (Merkle root comparison)

Failure modes:
  - Alice unreachable → peer registered as 'pending', no cached index
  - Token signature invalid → zetl peer add fails with structured error
  - Token expired → refresh fails, peer marked 'stale', cached data retained
```

```
Happy Path: Unjoining a Peer

Preconditions:
  - Bob has alice registered as a peer with cached index

Steps:
  1. Bob removes the peer:
     zetl peer remove alice
     → Deletes .zetl/peers/alice/ (capability + cached index + materialized files)
  2. Bob runs zetl index:
     → Link graph rebuilt without Alice's pages
     → Bob's [[Topic A]] becomes a dead link again
  3. Bob runs zetl check:
     → Reports: "3 dead links (2 previously resolved via peer 'alice')"
     → Context helps Bob understand these are not new broken links

Postconditions:
  - Bob's vault is unchanged — no files modified
  - Only the graph overlay was removed
  - Bob can re-join later with a new capability token

Failure modes:
  - None — unjoining is a local operation, always succeeds
```

```
Happy Path: Multi-Vault Link Resolution

Preconditions:
  - Alice has local page "Topic A" in her vault
  - Alice peers with Bob, who also has "Topic A" in his shared scope
  - Alice peers with Carol, who has "Topic A" in her shared scope

Steps:
  1. Alice writes a note containing [[Topic A]]
  2. Alice runs zetl index:
     → [[Topic A]] resolves to local page (primary match)
     → Bob's "Topic A" and Carol's "Topic A" also match
  3. Alice runs zetl links "My Note":
     → Output shows:
       { "target": "Topic A",
         "matches": [
           { "vault": "local", "path": "Topic A.md" },
           { "vault": "peer:bob", "path": "research/Topic A.md" },
           { "vault": "peer:carol", "path": "shared/Topic A.md" }
         ] }
  4. Alice runs zetl check:
     → Diagnostic: "shadow: [[Topic A]] matches in 3 vaults (local, bob, carol)"
  5. For graph traversal (backlinks, path, etc.), local "Topic A" is used
  6. Alice can inspect each version to understand the shadow

Postconditions:
  - All matches visible — nothing hidden
  - Graph traversal uses local (deterministic)
  - User has full agency to rename, scope, or accept the shadow

Failure modes:
  - All three pages could have different content — zetl surfaces this,
    does not attempt to merge or choose
```

---

## 3. Architecture

### 3.1 Component Architecture

```
Alice's Machine                              Bob's Machine
┌──────────────────────────┐                ┌──────────────────────────┐
│                          │                │                          │
│  ┌────────────────────┐  │                │  ┌────────────────────┐  │
│  │   zetl CLI (Rust)  │  │                │  │   zetl CLI (Rust)  │  │
│  │                    │  │                │  │                    │  │
│  │ index, links,      │  │                │  │ index, links,      │  │
│  │ backlinks, search, │  │                │  │ backlinks, search, │  │
│  │ check, peer        │  │                │  │ check, peer        │  │
│  └─────────┬──────────┘  │                │  └─────────┬──────────┘  │
│            │             │                │            │             │
│    ┌───────┴───────┐     │                │    ┌───────┴───────┐     │
│    │  Peer Engine  │     │   OcapN/CapTP  │    │  Peer Engine  │     │
│    │  (native Rust)│◄────┼───────────────►┤    │  (native Rust)│     │
│    │               │     │  signed tokens │    │               │     │
│    │ - identity    │     │  + Merkle sync │    │ - identity    │     │
│    │ - capabilities│     │                │    │ - capabilities│     │
│    │ - peer cache  │     │                │    │ - peer cache  │     │
│    └───────┬───────┘     │                │    └───────┬───────┘     │
│            │             │                │            │             │
│    .zetl/                │                │    .zetl/                │
│    ├── identity.json     │                │    ├── identity.json     │
│    ├── identity.key      │                │    ├── identity.key      │
│    ├── index.json        │                │    ├── index.json        │
│    ├── revocations.json  │                │    ├── revocations.json  │
│    └── peers/            │                │    └── peers/            │
│        └── bob/          │                │        └── alice/        │
│           ├── cap.json   │                │           ├── cap.json   │
│           ├── index.json │                │           ├── index.json │
│           ├── status.json│                │           ├── status.json│
│           └── files/     │                │           └── files/     │
│              └── (read-  │                │              └── (read-  │
│                 content) │                │                 content) │
│                          │                │                          │
│  vault-a/                │                │  vault-b/                │
│  ├── research/           │                │  ├── research/           │
│  ├── drafts/ (private)   │                │  ├── notes/              │
│  └── .zetl/              │                │  └── .zetl/              │
│                          │                │                          │
└──────────────────────────┘                └──────────────────────────┘
```

### 3.2 Component Responsibilities

**zetl CLI (Rust)** — extended with `peer` subcommand group:

| Responsibility | Details |
| --- | --- |
| Local vault operations | All existing commands unchanged |
| Vault identity | Ed25519 keypair generation and management at `zetl init` |
| Capability tokens | Creation, signing, verification, attenuation |
| Peer management | `zetl peer {add,remove,sync,list,grant,revoke}` |
| Joined graph construction | `LinkGraph::build` takes local files + peer caches |
| Multi-match resolution | Link queries return matches from all vaults with provenance |
| Merkle delta sync | Compare vault root hashes, fetch only changed file metadata |
| Integrity verification | Verify peer vault roots against Ed25519 signatures |
| Cross-vault drift | Detect when peer pages have changed since local reference |

**Filesystem** — the storage:

| Location | Purpose |
| --- | --- |
| `vault/*.md` | Markdown source files (source of truth, never modified by federation) |
| `.zetl/index.json` | Local link graph index (existing, unchanged) |
| `.zetl/identity.json` | Vault public key (stable identity) |
| `.zetl/identity.key` | Vault private key (file mode 0600, never transmitted) |
| `.zetl/revocations.json` | Nonces of revoked capabilities |
| `.zetl/peers/<label>/cap.json` | Capability token for this peer |
| `.zetl/peers/<label>/index.json` | Cached peer page manifest |
| `.zetl/peers/<label>/status.json` | Peer status (active, pending, stale, revoked) |
| `.zetl/peers/<label>/files/` | Materialized peer markdown files (`read-content` tier only) |

### 3.3 Identity Model

```
ADR-001: Vault Identity — Ed25519 Keypair

Status: Proposed

Context:
  Vaults need a stable cryptographic identity independent of content.
  The Merkle root (SPEC-006) changes with every file edit, so it cannot
  serve as identity. Options evaluated:

  A. Ed25519 keypair (signing + verification)
     + Compact keys (32 bytes public, 64 bytes private)
     + Fast signing (deterministic, no nonce needed)
     + Same primitive used by OcapN/CapTP
     + Strong Rust ecosystem (ed25519-dalek crate)
     - Not suitable for encryption (need X25519 for that)

  B. X25519 keypair (Diffie-Hellman key agreement)
     + Enables encrypted channels directly
     - Cannot sign tokens (not a signing algorithm)
     - Would need a separate signing key anyway

  C. RSA keypair
     + Well-understood, universal support
     - Large keys (2048+ bits), slow signing
     - Overkill for this use case

  D. No cryptographic identity (use random UUIDs)
     + Simplest implementation
     - Cannot sign capabilities — tokens are forgeable
     - Cannot verify content integrity

Decision:
  Ed25519 keypair (Option A). Generated once at `zetl init`, stored in
  .zetl/identity.json (public) and .zetl/identity.key (private, 0600).
  The public key IS the vault's stable identity.

Consequences:
  + Self-contained capability tokens — verifiable offline
  + Content integrity — vault roots are signed
  + OcapN compatibility — same primitive, easy migration to full CapTP
  + Single binary — ed25519-dalek compiles natively, no FFI
  - Users must protect identity.key (same as SSH keys, GPG keys)
  - Key rotation requires re-issuing all capabilities
```

**No user identity.** Capabilities are bearer tokens — possession is authorization. The system never records or transmits who holds a capability. This is the object-capability (ocap) insight: authorization is conferred by holding a reference, not by proving who you are.

### 3.4 Capability Model

```
ADR-002: Capability Format — Self-Contained Signed Tokens

Status: Proposed

Context:
  OcapN capabilities are live actor references (sturdyrefs) that require
  a network connection to enliven. For a local-first tool that must work
  offline, we need self-contained tokens. Options:

  A. Live sturdyrefs (OcapN native)
     + Full OcapN compatibility, promise pipelining
     - Requires live connection to verify
     - Cannot work offline

  B. Self-contained signed tokens (UCAN-aligned)
     + Verifiable offline (check Ed25519 signature)
     + Portable (JSON, can be emailed, stored in files)
     + Delegation via signature chains
     - Revocation requires online check (best-effort)
     - No promise pipelining

  C. Macaroons (Google/Chalmers-style)
     + Contextual caveats (IP range, time, etc.)
     + Third-party attenuation
     - More complex implementation
     - Less aligned with OcapN ecosystem

Decision:
  Self-contained signed tokens (Option B) for the initial implementation.
  Token format is UCAN-aligned and extensible to delegation chains.
  Migration path to live sturdyrefs (Option A) preserved for when full
  CapTP is implemented.

Consequences:
  + Works offline — critical for local-first
  + Simple to implement — JSON + Ed25519 signature
  + Extensible — UCAN delegation chains add multi-hop later
  - Revocation is best-effort (online check when available)
  - No real-time capability updates without polling
```

**Capability Token Format:**

```json
{
  "version": 1,
  "granter": "ed25519:a1b2c3d4e5f6...",
  "scope": "research/*",
  "ops": ["graph"],
  "expires": "2026-06-01T00:00:00Z",
  "nonce": "f8a9b2c1d3e4...",
  "issued_at": "2026-02-25T12:00:00Z",
  "signature": "7f8a9b2c..."
}
```

| Field | Purpose |
| --- | --- |
| `version` | Token format version (currently 1) |
| `granter` | Granting vault's public key (identity anchor) |
| `scope` | Glob pattern over page paths relative to vault root |
| `ops` | Permitted operations: `["graph"]`, `["graph","read-index"]`, or `["graph","read-index","read-content"]` |
| `expires` | Optional expiry timestamp (ISO-8601). Null = no expiry |
| `nonce` | Random 32 bytes (hex). Enables revocation — checked against granter's revocation list |
| `issued_at` | Creation timestamp |
| `signature` | Ed25519 signature over all fields above (canonical JSON serialization) |

**Operation Tiers:**

| Tier | What is shared | Use case |
| --- | --- | --- |
| `graph` | Page names, outgoing link targets, file Merkle roots | Link graph integration — dead links resolve, backlinks span vaults |
| `read-index` | Above + section headings, SPL block metadata, block IDs | Structural visibility — see what a page covers without reading it |
| `read-content` | Above + full file contents (materialized locally) | Full access — search, embed, transclude peer content |

Each tier is a strict superset. A `read-content` capability implicitly permits `read-index` and `graph`.

**Capability Lifecycle:**

```
1. CREATE — Alice generates a capability for a scope:
   zetl peer grant --scope "research/*" --ops graph
   → Writes signed token to stdout (JSON)

2. DELEGATE — Alice sends the token to Bob (out-of-band):
   email, chat, file, QR code, environment variable

3. ATTENUATE — Bob can derive a narrower capability:
   zetl peer attenuate ./alice-cap.json --scope "research/public/*" --ops graph
   → New token: scope narrowed, ops same or narrower, expiry same or earlier
   → Signed by Bob's key, with chain link to Alice's token

4. USE — Bob registers and syncs:
   zetl peer add alice --cap ./alice-cap.json
   zetl peer sync alice
   zetl index (peers refreshed by default)

5. REVOKE — Alice revokes Bob's access:
   zetl peer revoke --nonce <nonce-from-token>
   → Nonce added to .zetl/revocations.json
   → Bob's next sync attempt checks revocation list → rejected
   → Bob's cached data marked 'revoked'
```

**Alternative: Phrase-Based Exchange (Human-Friendly)**

The token-based flow above requires out-of-band transfer of a JSON blob — hostile to humans. For interactive, synchronous scenarios (phone calls, in-person, pair sessions), a phrase-based flow uses SPAKE2 + BIP39 to reduce capability sharing to four spoken words:

```
1. INVITE — Alice starts an invite session:
   zetl peer invite --scope "research/*" --ops graph
   → Generates random BIP39 phrase: "tiger maple ocean drift"
   → Starts listening for SPAKE2 connection
   → Displays phrase to Alice

2. COMMUNICATE — Alice tells Bob the phrase (any channel):
   voice, text, in person, paper

3. JOIN — Bob uses the phrase to connect:
   zetl peer join --phrase "tiger maple ocean drift" --label alice
   → Both sides execute SPAKE2 using the phrase as shared password
   → SPAKE2 derives a shared symmetric key
   → Wrong phrase → key mismatch → handshake fails immediately
   → Over the SPAKE2-encrypted channel:
     Alice sends: signed capability token + vault public key + Merkle root
     Bob verifies: Ed25519 signature on token
     Bob fetches: initial peer index
   → Phrase discarded. Real security is now the Ed25519 token.
```

Both flows produce the same result: a stored capability token in `.zetl/peers/`. Token-based is better for agents and automation. Phrase-based is better for humans.

```
ADR-004: Phrase-Based Capability Exchange — SPAKE2 + BIP39

Status: Proposed

Context:
  SPEC-011's capability tokens are JSON blobs with Ed25519 signatures.
  They work well for automation (pipe JSON, env vars) but are hostile
  to humans — you cannot read a 500-character JSON token over the phone.

  The "Magic Wormhole" pattern (Brian Warner, 2016) solves this:
  two parties who share a short passphrase can establish an encrypted
  channel without any prior key exchange or PKI. The passphrase is
  ephemeral — used once to bootstrap the channel, then discarded.

  Options evaluated:

  A. No change — keep JSON token exchange only
     + Simplest implementation
     - Poor UX for humans sharing capabilities

  B. QR codes
     + Visual, can encode full token
     - Requires camera/screen, not usable over voice
     - Still need a fallback for non-visual contexts

  C. SPAKE2 + BIP39 phrases (Magic Wormhole pattern)
     + 4 words spoken over the phone
     + Secure against eavesdroppers (SPAKE2 resists offline dictionary attacks)
     + No PKI, no certificates, no key servers
     + Well-proven pattern (Magic Wormhole, Signal safety numbers)
     + Excellent Rust crate support (spake2, bip39, magic-wormhole)
     - Requires both parties to be online simultaneously
     - Requires a rendezvous mechanism (or direct address)

  D. Pre-shared key via NFC/Bluetooth
     + Tap-to-share UX
     - CLI tool, not a mobile app
     - Platform-specific, complex

Decision:
  SPAKE2 + BIP39 phrases (Option C) as an interactive complement to
  token-based exchange. Both flows coexist:

    Token-based (agents, async):  zetl peer grant → JSON → zetl peer add
    Phrase-based (humans, sync):  zetl peer invite → 4 words → zetl peer join

  BIP39 word list provides 11 bits of entropy per word. Four words =
  44 bits — sufficient for a one-time exchange with a short validity
  window (default 5 minutes). The phrase only needs to resist brute-force
  during the invite session; the long-term security comes from the
  Ed25519-signed capability token transmitted over the SPAKE2 channel.

Consequences:
  + Dramatically better UX for human-to-human capability sharing
  + Secure against passive and active network attackers
  + No additional infrastructure required (direct connect on LAN)
  + Optional rendezvous server enables cross-NAT use
  + Same result as token-based flow — no separate code paths downstream
  - Requires both parties online simultaneously (synchronous)
  - Adds spake2 and bip39 crate dependencies
  - Invite session has a timeout (default 5 min) — phrase expires
```

### 3.5 Local Persistence Model

Peer data is stored locally according to the capability tier. The key insight: **if you can't unread, then read should allow local persistence.** Once a vault has received file contents via `read-content`, there is no mechanism to force the recipient to forget — the data has already been transmitted. Persisting it locally simply makes this reality explicit and useful.

**Tiered materialization:**

| Tier | What is persisted locally | Storage location | Approx. size/page |
| --- | --- | --- | --- |
| `graph` | Page names, link targets, Merkle roots | `.zetl/peers/<label>/index.json` | ~1KB |
| `read-index` | Above + section headings, SPL metadata, block IDs | `.zetl/peers/<label>/index.json` | ~5KB |
| `read-content` | Above + full markdown files as real files on disk | `.zetl/peers/<label>/files/<path>` | File size + ~1KB |

At `graph` and `read-index` tiers, everything lives in the JSON index cache — compact, fast to load, ephemeral. At `read-content`, the peer's shared files are materialized as real markdown files in a shadow directory. This enables:

- **`zetl search`** works over peer content (the files are real, so existing search code works unchanged)
- **Transclusion** via `zetl view` can render peer content inline
- **Offline access** to full peer content, not just metadata
- **External tools** (grep, editors, etc.) can read the files
- **Existing Rust file scanner** can parse them with zero changes

The shadow directory (`files/`) mirrors the peer's folder structure within the capability scope:

```
.zetl/peers/alice/
├── cap.json
├── index.json
├── status.json
└── files/
    └── research/
        ├── Topic A.md
        ├── Topic B.md
        └── subfolder/
            └── Deep Note.md
```

**Lifecycle:**

- **On sync (`zetl peer sync`):** files within scope are fetched and written to `files/`. Changed files are overwritten (Merkle delta determines which files changed). Deleted files on the peer are deleted locally.
- **On unjoin (`zetl peer remove`):** the entire `.zetl/peers/<label>/` directory is deleted, including `files/`. The materialized files are gone.
- **On revocation:** when a capability is revoked and the peer detects it, materialized files are deleted. If the peer is offline and cannot detect revocation, files persist with the stale cache — this is the "can't unread" reality. The revocation purge happens on next successful sync attempt.
- **On scope narrowing:** if a capability is replaced with a narrower scope, files outside the new scope are deleted on next sync.

**Not part of the vault.** Materialized peer files live under `.zetl/`, not in the vault root. They are excluded from the local index, local Merkle tree, and local link graph. They exist solely as a local cache for read access and search. The canonical source remains the peer's vault.

```
ADR-005: Tiered Local Persistence — Materialize Files at read-content

Status: Proposed

Context:
  When a vault grants read-content access to a peer, the peer receives
  full file contents. Three storage strategies were considered:

  A. Metadata cache only (all tiers store JSON)
     + Minimal disk usage
     + Simple cleanup
     - zetl search cannot index peer content (no files to scan)
     - Transclusion requires live fetch every time
     - Offline access limited to cached metadata

  B. Shadow directory with real files (read-content materializes)
     + zetl search works over peer files unchanged
     + Offline access to full content
     + External tools can read the files
     + Existing scanner/parser code works without modification
     + Honest about reality: recipient has the data, might as well
       make it useful
     - More disk usage at read-content tier
     - Must handle cleanup on unjoin/revocation

  C. Mount into vault namespace (symlinks or virtual FS)
     + Files appear in vault alongside local pages
     - Breaks vault purity — external content in the vault root
     - Confuses git (untracked files from peers)
     - Editing a symlinked file modifies the peer's vault (if writable)

  D. On-demand fetch (fetch content only when accessed)
     + Minimal storage
     - Requires network for every content access
     - Defeats local-first principle
     - Latency on every search, read, or transclusion

Decision:
  Shadow directory (Option B). read-content capabilities materialize
  peer files into .zetl/peers/<label>/files/, mirroring the peer's
  directory structure within the capability scope.

  graph and read-index tiers store only JSON metadata (compact,
  ephemeral). read-content adds real files on disk.

  The principle: if you can't unread, then read should allow local
  persistence. The files are a cache — useful, explicit, and cleanly
  removable.

Consequences:
  + zetl search spans peer content without code changes
  + Transclusion and view work offline with peer content
  + Cleanup is simple: rm -rf .zetl/peers/<label>/
  + Scanner/parser need no peer-awareness — files are just files
  - Disk usage proportional to peer content at read-content tier
  - Revocation cannot force data deletion if peer is offline
  - Users must understand that read-content peers have their files
```

### 3.6 Joined Link Graph

The joined graph is virtual and ephemeral — rebuilt on every `zetl index` from local files plus peer caches (and, at `read-content` tier, materialized files). It exists only in memory as a `petgraph::DiGraph`.

**Graph construction (extended `LinkGraph::build`):**

```
Input:
  - local_files: Vec<ParsedFile>         (from scanner, existing)
  - peer_caches: Vec<PeerCache>          (from .zetl/peers/*/index.json, new)

Algorithm:
  1. Build local nodes and edges (existing behavior, unchanged)
  2. For each active peer:
     a. Load peer's cached page manifest
     b. For each page in manifest:
        - Create a remote node with provenance: (peer_label, page_name, path, merkle_root)
        - Add to node_map with qualified key "peer:<label>:<page_name>"
        - Add to resolved set (so dead-link detection considers it)
     c. For each link in each peer page:
        - Create edge from peer node to target (local or remote)
  3. Resolve wikilinks:
     a. For each local wikilink [[Target]]:
        - Collect all matches: local node + any peer nodes with same page_name
        - Primary edge → local match (if exists)
        - Secondary edges → peer matches (for query output)
        - If no local match but peer match exists → edge to first peer match
     b. Record multi-match diagnostics
```

**Link resolution priority:**

```
1. Local match exists → primary, edge connects to local node
   Peer matches → recorded as shadows in diagnostics

2. No local match, single peer match → edge connects to peer node
   Dead link → resolved

3. No local match, multiple peer matches → edge connects to
   first peer (by registration order, deterministic)
   Other peer matches → recorded as multi-match diagnostic

4. No match anywhere → dead link (existing behavior)
```

### 3.7 Multi-Match Resolution

```
ADR-003: Link Resolution — Show All Matches

Status: Proposed

Context:
  When [[Topic A]] exists in the local vault AND in one or more peers,
  the system must decide how to handle the ambiguity. Options:

  A. Local-only (ignore peers for resolution)
     + Simplest, no behavior change
     - Peer pages never resolve dead links
     - Defeats the purpose of federation

  B. Priority-based (local > first peer > second peer)
     + Deterministic, predictable
     + Dead links resolve via peers
     - Silent — user doesn't know a peer page was chosen
     - Adding a new peer could silently change which page a link resolves to

  C. Show all matches with provenance
     + User sees the full picture
     + No silent resolution changes
     + Diagnostics warn about shadows
     - More verbose output
     - Graph traversal still needs a single target (pick local or first peer)

  D. Require explicit disambiguation (fail if ambiguous)
     + Forces users to be explicit
     - Too strict — most shadows are harmless

Decision:
  Show all matches (Option C) for query output. Use priority-based
  (Option B) for graph traversal (backlinks, shortest path, etc.)
  where a single target is needed. Emit shadow diagnostics so users
  can discover and address ambiguities when they choose to.

Consequences:
  + Full visibility — nothing hidden from the user
  + Deterministic graph traversal — local always wins
  + Agent-friendly — JSON output includes all matches, agent picks what it needs
  + Diagnosable — zetl check surfaces shadows
  - Slightly more complex link output format
  - Users may need to learn about shadow diagnostics
```

### 3.8 Merkle Tree Integration

The existing BLAKE3 Merkle tree (SPEC-006) serves three roles in federation:

**1. Efficient change detection:**

```
Alice cached Bob's vault root:  abc123...
Bob's current vault root:       def456...

→ Roots differ. Walk the file-level hashes:
  Topic A.md root: same    → skip
  Topic B.md root: changed → fetch new metadata
  Topic C.md root: new     → fetch

Data transfer: O(changed files), not O(total files)
```

**2. Content integrity verification:**

```
Bob signs his current vault root with his Ed25519 private key.
Alice verifies the signature against Bob's public key (from the
capability token's granter field).

If verification fails → refresh rejected, peer marked 'untrusted'.

The Merkle tree turns the trust model from "I trust Bob" into
"I trust math." Bob cannot serve tampered metadata without the
hashes diverging from the signed root.
```

**3. Cross-vault drift detection:**

Alice writes `[[Topic A]]` when Bob's Topic A has Merkle root `eee...`. Later, Bob edits Topic A (root becomes `fff...`). Alice's drift detector reports:

```
warning: linked page 'Topic A' (peer 'bob') has changed since this
         section was written
  --> notes/Design.md:15:5
   | [[Topic A]] references peer page with merkle root eee...
   | current peer page merkle root is fff...
```

This extends SPEC-006's existing drift model:

| Drift type | Single vault (existing) | Cross-vault (new) |
| --- | --- | --- |
| Section drift | SPL grounding hash != section prose hash | — |
| Explicit drift | Cross-file grounding target hash changed | Cross-vault grounding target hash changed |
| Link target drift | — | Peer page Merkle root differs from cached value at time of reference |

---

## 4. Requirements

### 4.1 Functional Requirements — Identity

```
REQ-001: Vault Identity Keypair

The system SHALL generate an Ed25519 keypair at `zetl init` and store
it in .zetl/identity.json (public key, JSON) and .zetl/identity.key
(private key, file mode 0600).

The public key SHALL serve as the vault's stable cryptographic identity,
independent of content. The keypair SHALL NOT change when files are
added, removed, or edited. If identity files already exist, `zetl init`
SHALL NOT overwrite them (idempotent).

The private key file SHALL be created with mode 0600 (owner read/write
only). The system SHALL warn if identity.key has permissions broader
than 0600.

Trace:
  - TEST-001
  - CON-001
  - ADR-001
```

```
REQ-002: No User Identity

The system SHALL NOT require user accounts, passwords, or
authentication. Capabilities are bearer tokens — possession of a
valid token is sufficient authorization.

The system SHALL NOT record or transmit the identity of capability
holders. No user registry, login flow, or identity provider SHALL
be required for federation to function.

Trace:
  - TEST-002
```

### 4.2 Functional Requirements — Capabilities

```
REQ-003: Capability Token Creation

The system SHALL provide `zetl peer grant` to create signed capability
tokens. The command SHALL:
  a) Read the vault's Ed25519 private key from .zetl/identity.key
  b) Generate a random 32-byte nonce
  c) Construct a token with: granter public key, scope glob, operation
     tier, optional expiry, nonce, issued_at timestamp
  d) Sign the token with Ed25519
  e) Output the token as JSON to stdout

The token SHALL be self-contained — verifiable offline by checking the
Ed25519 signature against the granter's public key.

Trace:
  - TEST-003
  - CON-002
  - ADR-002
```

```
REQ-004: Capability Operation Tiers

The system SHALL define three operation tiers, each a strict superset
of the previous:

  graph         — page names, outgoing link targets, file Merkle roots
  read-index    — above + section headings, SPL block metadata, block IDs
  read-content  — above + full file contents

A capability with read-content implicitly grants read-index and graph.
A capability with read-index implicitly grants graph. The system SHALL
reject operations that exceed the token's tier.

Trace:
  - TEST-004
  - CON-002
```

```
REQ-005: Capability Attenuation

The system SHALL provide `zetl peer attenuate` to derive a new
capability with strictly narrower permissions from an existing token.

The derived capability's scope MUST be a subset of the parent's scope
(glob intersection). The derived capability's operations MUST be a
subset of the parent's operations. The derived capability's expiry MUST
be equal to or earlier than the parent's.

The derived token SHALL be signed by the deriving party's vault key,
with a delegation chain linking back to the original granter. Any
party can verify the chain by walking signatures from leaf to root.

The system SHALL reject attenuation attempts that widen any dimension
of the parent capability.

Trace:
  - TEST-005
  - CON-003
```

```
REQ-006: Capability Revocation

The system SHALL provide `zetl peer revoke --nonce <hex>` to revoke
a previously granted capability by its nonce.

The granting vault SHALL maintain a revocation list in
.zetl/revocations.json (append-only JSON array of nonce hex strings).

When a peer refreshes, it SHALL check the granter's revocation list
if reachable. If the capability's nonce appears in the list, the peer
SHALL mark the capability as revoked, stop serving cached data from it,
and report the revocation to the user.

Revocation checking SHALL be best-effort — offline operation continues
with cached data and a staleness warning. The system SHALL NOT block
normal operations while waiting for revocation checks.

Trace:
  - TEST-006
  - CON-004
```

### 4.3 Functional Requirements — Peer Management

```
REQ-007: Peer Addition

The system SHALL provide `zetl peer add --label <name> --cap <path>`
to register a remote vault as a peer. The command SHALL:
  a) Parse and validate the capability token (check Ed25519 signature)
  b) Store the token in .zetl/peers/<label>/cap.json
  c) Write initial status to .zetl/peers/<label>/status.json
  d) Attempt an initial index fetch from the remote vault
  e) If fetch succeeds, cache the peer's page manifest in
     .zetl/peers/<label>/index.json
  f) If the remote is unreachable, register the peer with status
     'pending' and no cached index (warn, do not error)

The system SHALL reject duplicate labels (exit 1 with structured error).
The system SHALL reject tokens with invalid signatures (exit 1).

Trace:
  - TEST-007
  - CON-005
```

```
REQ-008: Peer Refresh (Sync)

The system SHALL provide `zetl peer sync [--label <name>]` to refresh
peer caches. Refresh SHALL:
  a) Compare the cached vault root hash against the remote vault's
     current root hash (single hash comparison)
  b) If identical, no data transfer (cache is current)
  c) If different, walk the Merkle tree to identify which files within
     the capability scope have changed
  d) Fetch only changed file metadata (delta sync)
  e) Update the cached index and vault root hash
  f) At read-content tier: fetch changed file contents and write to
     .zetl/peers/<label>/files/ (see REQ-019, REQ-020)

Refresh without --label SHALL refresh all active peers.
`zetl index` SHALL refresh peers by default (configurable via
--no-peers flag).

Trace:
  - TEST-008
  - CON-006
  - REQ-019
```

```
REQ-009: Peer Removal (Unjoining)

The system SHALL provide `zetl peer remove <label>` to unjoin a peer.
The command SHALL:
  a) Delete .zetl/peers/<label>/ (capability + cached index + status +
     materialized files if any)
  b) The next zetl index SHALL rebuild the graph without the removed
     peer's pages
  c) Wikilinks previously resolved via the removed peer SHALL become
     dead links
  d) zetl check SHALL report these with context: "dead link —
     previously resolved via peer '<label>'"

Removal is a local operation — it does not notify the remote vault.
All materialized files (read-content tier) are deleted with the peer
directory.

Trace:
  - TEST-009
  - CON-005
```

```
REQ-010: Peer Listing

The system SHALL provide `zetl peer list` to display all registered
peers with: label, vault public key (truncated), status, last sync
timestamp, number of cached pages, capability scope, and operation tier.

Output SHALL follow zetl's existing format conventions (JSON default,
table with --format table).

Trace:
  - TEST-010
  - CON-005
```

### 4.4 Functional Requirements — Joined Link Graph

```
REQ-011: Joined Graph Construction

The system SHALL build the LinkGraph from both local files AND active
peer caches. Remote pages SHALL be represented as nodes in the
petgraph::DiGraph with provenance metadata (peer label, vault public
key, Merkle root). Edges from local pages to remote pages (and from
remote to remote within the same peer) SHALL be included. The resolved
set SHALL include remote pages. Dead-link detection (SPEC-001/REQ-005)
SHALL consider remote pages as resolved targets.

The joined graph is virtual — rebuilt on every zetl index from local
index plus peer caches. No persistent merge.

Trace:
  - TEST-011
```

```
REQ-012: Multi-Vault Link Resolution

The system SHALL resolve [[wikilinks]] against all vaults (local +
peers) and return the full set of matches, ordered by locality:

  1. Local match: always included, marked as primary
  2. Peer matches: included for every peer where the page exists
     within capability scope, marked with peer label

When a target matches in multiple vaults, zetl links SHALL display all
matches with provenance annotations in the JSON output.

For graph traversal (backlinks, shortest path, connected components):
  - Local match exists → use local node
  - No local match, single peer match → use peer node
  - No local match, multiple peer matches → use first peer by
    registration order (deterministic)

zetl check SHALL emit a shadow diagnostic when a link target resolves
in multiple vaults.

Trace:
  - TEST-012
  - ADR-003
```

### 4.5 Functional Requirements — Merkle Integration

```
REQ-013: Merkle-Based Delta Sync

The system SHALL use the peer vault's Merkle root hash (SPEC-006) to
detect changes during refresh. When roots differ, the system SHALL
walk file-level hashes to identify changed files within the capability
scope, and fetch only those files' metadata. Data transfer SHALL be
O(changed files), not O(total files in scope).

Trace:
  - TEST-013
```

```
REQ-014: Content Integrity Verification

On each refresh, the peer SHALL provide its current vault root hash
with an Ed25519 signature. The system SHALL verify this signature
against the granter's public key (embedded in the capability token).

If verification fails, the refresh SHALL be rejected and the peer
marked as 'untrusted'. The system SHALL NOT cache unverified data.

Trace:
  - TEST-014
```

```
REQ-015: Cross-Vault Drift Detection

The system SHALL extend drift detection (SPEC-006/REQ-043) across
vault boundaries. When a local page contains a wikilink to a peer
page, and the peer page's Merkle root has changed since the local page
was last indexed, the system SHALL emit a drift diagnostic:

  "linked page '<page>' (peer '<label>') has changed since this
   section was written"

Drift severity SHALL be Info for graph-only links and Warning for
SPL grounding references that cross the vault boundary.

Trace:
  - TEST-015
```

### 4.6 Functional Requirements — Offline Behavior

```
REQ-016: Graceful Offline Degradation

The system SHALL operate fully when peers are unreachable. Peer caches
SHALL be used as-is with a staleness annotation (time since last
successful refresh). All graph queries SHALL work against cached data.

zetl check SHALL report stale peers (unreachable for > 24 hours,
configurable via --stale-threshold).

The system SHALL NOT block, retry indefinitely, or produce errors on
unreachable peers during normal operations (zetl index, zetl links,
zetl backlinks, zetl check, etc.). Unreachable peers SHALL produce
a single warning on stderr, not an error exit code.

Trace:
  - TEST-016
```

### 4.7 Functional Requirements — Phrase-Based Exchange

```
REQ-017: Phrase-Based Capability Invite

The system SHALL provide `zetl peer invite` to start an interactive
capability exchange session. The command SHALL:
  a) Generate a random BIP39 mnemonic phrase (4 words, ~44 bits entropy)
  b) Create a signed capability token (per REQ-003)
  c) Start listening for a SPAKE2 connection (direct TCP or via
     rendezvous, configurable)
  d) Display the phrase to the user
  e) Wait for a peer to connect using the phrase (timeout: 5 minutes,
     configurable via --timeout)
  f) On successful SPAKE2 handshake: transmit the capability token,
     vault public key, and current Merkle root over the encrypted channel
  g) On timeout or cancellation: clean up, discard the phrase

The phrase SHALL be single-use — after one successful exchange or
timeout, the invite session ends. The phrase SHALL NOT be stored
on disk.

Trace:
  - TEST-017
  - CON-007
  - ADR-004
```

```
REQ-018: Phrase-Based Capability Join

The system SHALL provide `zetl peer join --phrase <words> --label <name>`
to connect to an active invite session. The command SHALL:
  a) Parse the BIP39 phrase
  b) Connect to the inviting peer (direct address or rendezvous)
  c) Execute SPAKE2 using the phrase as the shared password
  d) If the phrase is wrong: SPAKE2 key mismatch detected, handshake
     fails immediately with a clear error ("phrase mismatch")
  e) On successful handshake: receive the capability token over the
     SPAKE2-encrypted channel
  f) Verify the token's Ed25519 signature
  g) Store the token and register the peer (same as REQ-007)
  h) Perform initial peer index fetch

The result SHALL be identical to `zetl peer add --cap` — a stored
capability token in .zetl/peers/<label>/. The phrase is discarded
after use. Long-term security is the Ed25519-signed token.

Trace:
  - TEST-018
  - CON-008
  - ADR-004
```

### 4.8 Functional Requirements — Local Persistence

```
REQ-019: Tiered Local Persistence

The system SHALL persist peer data locally according to the capability
tier:

  graph         — JSON metadata only (.zetl/peers/<label>/index.json)
  read-index    — JSON metadata only (richer index, same file)
  read-content  — JSON metadata + materialized markdown files in
                  .zetl/peers/<label>/files/

At the read-content tier, `zetl peer sync` SHALL write the peer's
shared files to .zetl/peers/<label>/files/, mirroring the peer's
directory structure within the capability scope. Files SHALL be
plain markdown — readable by zetl search, external tools, and the
file scanner without modification.

Materialized files SHALL NOT be included in the local vault's index,
Merkle tree, or link graph construction. They exist solely as a local
cache for read access and search.

Trace:
  - TEST-019a
  - ADR-005
```

```
REQ-020: File Materialization Lifecycle

Materialized peer files SHALL be managed as follows:

  a) On sync: fetch changed files (per Merkle delta), write to files/.
     Overwrite changed files, create new files, delete files removed
     by the peer or no longer within capability scope.
  b) On unjoin (zetl peer remove): delete the entire
     .zetl/peers/<label>/ directory, including files/.
  c) On revocation detected: delete materialized files. If the peer
     is unreachable and revocation cannot be detected, files persist
     with the stale cache until next successful contact.
  d) On scope narrowing: if a capability is replaced with a narrower
     scope, files outside the new scope SHALL be deleted on next sync.

The system SHALL NOT require network connectivity to read materialized
files. Materialized content is a local cache — available offline.

Trace:
  - TEST-019b
  - ADR-005
  - REQ-009
```

### 4.9 Non-Functional Requirements

```
NFR-001: Peer Sync Latency

Peer refresh latency SHALL be <= 500ms for a peer with <= 1,000 pages
when <= 10 pages have changed, measured from network round-trip start
to cache write complete, excluding network transit time.

Trace:
  - TEST-013
```

```
NFR-002: Zero-Peer Overhead

All zetl commands that work today SHALL continue to work with zero
degradation when no peers are configured. Peer-related code paths
SHALL add <= 50ms to `zetl index` when .zetl/peers/ does not exist
or is empty.

Trace:
  - TEST-016
```

```
NFR-003: Offline Tolerance

When peers are configured but unreachable, `zetl index` SHALL complete
within 200ms of the no-peer baseline (connection timeout, not blocking).

Trace:
  - TEST-016
```

```
NFR-004: Peer Cache Size

Per-page cache overhead SHALL be:
  - <= 1KB per page at the graph tier (JSON metadata only)
  - <= 5KB per page at the read-index tier (JSON metadata only)
  - <= file size + 1KB overhead per page at the read-content tier
    (JSON metadata + materialized file in .zetl/peers/<label>/files/)

At read-content tier, total peer storage is bounded by the size of
the peer's shared files within the capability scope. Users granting
read-content should understand that the recipient will store a full
copy of the shared files.

Trace:
  - TEST-019a
  - ADR-005
```

```
NFR-005: Transport Security

All peer-to-peer communication SHALL be encrypted. The minimum
transport is TCP+TLS with certificate pinning tied to the vault's
Ed25519 public key. Tor onion services SHALL be supported as an
optional transport for NAT traversal and anonymity.

No plaintext page content or index metadata SHALL traverse the network.

Trace:
  - TEST-018
```

```
NFR-006: Backward Compatibility

The federation subsystem SHALL be fully additive. Existing zetl
commands SHALL work identically whether or not peers are configured.
The peer subsystem SHALL NOT modify the existing index format or any
existing .zetl/ files except to add new files (identity, peers).

Trace:
  - TEST-019a
  - TEST-019b
```

---

## 5. Contract Specifications (CLI Interface)

```
CON-001: zetl init (extended)

zetl init [OPTIONS]

Extended behavior: if .zetl/identity.json does not exist, generate
an Ed25519 keypair and write:
  - .zetl/identity.json: { "public_key": "ed25519:<hex>", "created": "<ISO-8601>" }
  - .zetl/identity.key: raw private key bytes (file mode 0600)

If identity files already exist, no-op (idempotent).

Options:
  --force-new-identity    Regenerate keypair (destructive — invalidates
                          all previously granted capabilities)

Example output (JSON):
{
  "vault_identity": "ed25519:a1b2c3d4e5f6...",
  "created": "2026-02-25T12:00:00Z",
  "identity_file": ".zetl/identity.json",
  "key_file": ".zetl/identity.key"
}

Exit codes:
  0  Success (created or already exists)
  1  Permission error (cannot write .zetl/)

Implements: REQ-001
Verified by: TEST-001
```

```
CON-002: zetl peer grant

zetl peer grant --scope <GLOB> --ops <TIER> [OPTIONS]

Arguments:
  --scope <GLOB>     Folder or glob pattern relative to vault root
  --ops <TIER>       One of: graph, read-index, read-content

Options:
  --expires <ISO-8601 or DURATION>  Capability expiry (e.g., 2026-06-01, 7d, 24h)
  --label <TEXT>                    Human-readable label (stored in token metadata)

Output: signed capability token (JSON) to stdout.

Example output:
{
  "version": 1,
  "granter": "ed25519:a1b2c3d4e5f6...",
  "scope": "research/*",
  "ops": ["graph"],
  "expires": "2026-06-01T00:00:00Z",
  "nonce": "f8a9b2c1d3e4...",
  "issued_at": "2026-02-25T12:00:00Z",
  "signature": "7f8a9b2c..."
}

Exit codes:
  0  Success
  1  No vault identity (run zetl init first)
  2  Invalid scope glob or ops tier

Implements: REQ-003, REQ-004
Verified by: TEST-003, TEST-004
```

```
CON-003: zetl peer attenuate

zetl peer attenuate <TOKEN_PATH> [OPTIONS]

Arguments:
  <TOKEN_PATH>       Path to parent capability token (JSON file)

Options:
  --scope <GLOB>     Narrower scope (must be subset of parent)
  --ops <TIER>       Narrower ops tier (must be subset of parent)
  --expires <ISO-8601 or DURATION>  Earlier expiry (must be <= parent's)

Output: derived capability token (JSON) to stdout, signed by this
vault's key, with delegation chain linking to parent.

Exit codes:
  0  Success
  1  Attenuation widens parent capability (scope, ops, or expiry)
  2  Parent token invalid or expired

Implements: REQ-005
Verified by: TEST-005
```

```
CON-004: zetl peer revoke

zetl peer revoke --nonce <HEX>

Appends the nonce to .zetl/revocations.json. Peers checking the
revocation list on next refresh will see the nonce.

Example output:
{
  "revoked_nonce": "f8a9b2c1d3e4...",
  "revocations_count": 3
}

Exit codes:
  0  Success (nonce added to revocation list)
  1  No vault identity

Implements: REQ-006
Verified by: TEST-006
```

```
CON-005: zetl peer {add, remove, list}

zetl peer add --label <NAME> --cap <TOKEN_PATH>
zetl peer remove <LABEL>
zetl peer list [--format json|table]

Add validates the token signature, stores it, and attempts initial fetch.
Remove deletes the peer directory.
List shows all registered peers.

Example output (JSON, list):
{
  "peers": [
    {
      "label": "alice",
      "vault_id": "ed25519:a1b2...f6",
      "status": "active",
      "last_sync": "2026-02-25T12:30:00Z",
      "cached_pages": 42,
      "scope": "research/*",
      "ops": "graph"
    }
  ]
}

Exit codes:
  0  Success
  1  Duplicate label (add), label not found (remove)
  2  Invalid token signature (add)

Implements: REQ-007, REQ-009, REQ-010
Verified by: TEST-007, TEST-009, TEST-010
```

```
CON-006: zetl peer sync

zetl peer sync [--label <NAME>] [--force]

Refreshes peer caches via Merkle-based delta sync.
Without --label, refreshes all active peers.
--force skips the Merkle root comparison and re-fetches everything.

Example output (JSON):
{
  "peers_refreshed": [
    {
      "label": "alice",
      "status": "updated",
      "previous_root": "abc123...",
      "current_root": "def456...",
      "pages_changed": 3,
      "pages_added": 1,
      "pages_removed": 0,
      "files_materialized": 4,
      "elapsed_ms": 230
    }
  ],
  "peers_unreachable": ["bob"],
  "peers_revoked": []
}

Exit codes:
  0  Success (may include unreachable peers — warning, not error)
  1  Label not found

Implements: REQ-008
Verified by: TEST-008
```

```
CON-007: zetl peer invite

zetl peer invite --scope <GLOB> --ops <TIER> [OPTIONS]

Arguments:
  --scope <GLOB>     Folder or glob pattern relative to vault root
  --ops <TIER>       One of: graph, read-index, read-content

Options:
  --expires <ISO-8601 or DURATION>  Capability expiry
  --timeout <SECONDS>               Invite session timeout [default: 300]
  --address <HOST:PORT>             Direct listen address (skip rendezvous)
  --rendezvous <URL>                Rendezvous server URL

Starts an interactive invite session. Displays a BIP39 phrase and
waits for a peer to connect via SPAKE2.

Example output (interactive, to stderr):
  Invite ready. Share this phrase with your peer:

    tiger maple ocean drift

  Waiting for connection... (expires in 5:00)

On successful exchange (JSON, to stdout):
{
  "status": "connected",
  "peer_vault_id": "ed25519:b2c3d4...",
  "capability_scope": "research/*",
  "capability_ops": "graph",
  "pages_shared": 42
}

Exit codes:
  0  Success (peer connected and received capability)
  1  Timeout (no peer connected within window)
  2  No vault identity (run zetl init first)

Implements: REQ-017
Verified by: TEST-017
```

```
CON-008: zetl peer join

zetl peer join --phrase <WORDS> --label <NAME> [OPTIONS]

Arguments:
  --phrase <WORDS>   BIP39 phrase from the inviting peer (4 words)
  --label <NAME>     Local label for this peer

Options:
  --address <HOST:PORT>    Direct connect address (skip rendezvous)
  --rendezvous <URL>       Rendezvous server URL

Connects to an active invite session using SPAKE2 with the shared
phrase. On success, receives and stores the capability token, then
fetches the initial peer index.

Example output (JSON):
{
  "label": "alice",
  "vault_id": "ed25519:a1b2c3...",
  "scope": "research/*",
  "ops": "graph",
  "status": "active",
  "cached_pages": 42,
  "method": "phrase"
}

Exit codes:
  0  Success (capability received, peer registered)
  1  Phrase mismatch (SPAKE2 handshake failed)
  2  Invite session expired or unreachable
  3  Token signature invalid (received bad token)

Implements: REQ-018
Verified by: TEST-018
```

---

## 6. Test Specifications

```
TEST-001: Vault Identity Generation

Scenario: Generate identity at init
Given: A vault with no .zetl/identity.json
When: `zetl init` is run
Then:
  - .zetl/identity.json is created with a valid Ed25519 public key
  - .zetl/identity.key is created with file mode 0600
  - Public key is 32 bytes (64 hex characters)
  - Running `zetl init` again does not overwrite the keypair

Verifies: REQ-001
```

```
TEST-002: No User Identity Required

Scenario: Federation works without user accounts
Given: Two vaults with identities
When: A capability token is created and used to add a peer
Then:
  - No username, password, or login was required at any step
  - The token does not contain any user-identifying information
  - The peer cache does not record who added the peer

Verifies: REQ-002
```

```
TEST-003: Capability Token Creation and Verification

Scenario: Create and verify a capability token
Given: A vault with identity
When: `zetl peer grant --scope "research/*" --ops graph --expires 7d`
Then:
  - Output is valid JSON with all required fields
  - Signature is verifiable against the granter's public key
  - Scope is "research/*", ops is ["graph"]
  - Expiry is ~7 days from now
  - Nonce is 32 random bytes (unique per invocation)

Scenario: Token verification rejects tampered tokens
Given: A valid token
When: Any field is modified (scope, ops, expiry, nonce)
Then: Signature verification fails

Verifies: REQ-003
```

```
TEST-004: Capability Operation Tiers

Scenario: Tier enforcement
Given: A peer added with ops=graph capability
When: A read-index operation is attempted (e.g., requesting section headings)
Then: The operation is rejected — capability insufficient
When: A graph operation is attempted (page names, links)
Then: The operation succeeds

Given: A peer added with ops=read-content capability
When: Any operation is attempted (graph, read-index, read-content)
Then: All operations succeed (read-content is superset)

Verifies: REQ-004
```

```
TEST-005: Capability Attenuation

Scenario: Valid attenuation
Given: A token with scope="**", ops=read-content, expires=2026-06-01
When: `zetl peer attenuate` with scope="research/*", ops=graph, expires=2026-03-01
Then: A new token is created with narrower scope, ops, and expiry

Scenario: Invalid attenuation (widening)
Given: A token with scope="research/*", ops=graph
When: `zetl peer attenuate` with scope="**" (wider scope)
Then: Command fails with exit code 1

Verifies: REQ-005
```

```
TEST-006: Capability Revocation

Scenario: Revoke a capability
Given: Alice granted a token with nonce X to Bob
When: Alice runs `zetl peer revoke --nonce X`
Then: Nonce X appears in .zetl/revocations.json
When: Bob attempts `zetl peer sync alice`
Then: Sync fails — capability revoked (if Alice is reachable)

Scenario: Offline revocation tolerance
Given: Alice revoked Bob's capability, but Alice is unreachable
When: Bob runs `zetl peer sync alice`
Then: Sync fails (unreachable), cached data retained with stale warning

Verifies: REQ-006
```

```
TEST-007: Peer Addition

Scenario: Add a peer with valid token
Given: Alice's vault has identity, Bob has a valid token from Alice
When: Bob runs `zetl peer add --label alice --cap ./token.json`
Then:
  - .zetl/peers/alice/cap.json contains the token
  - .zetl/peers/alice/status.json shows "active" (if Alice reachable)
  - .zetl/peers/alice/index.json contains cached page manifest

Scenario: Add peer with invalid token
Given: A token with a tampered signature
When: `zetl peer add --label alice --cap ./bad-token.json`
Then: Exit code 2, error: "capability token signature invalid"

Scenario: Add peer when remote is unreachable
Given: A valid token, but Alice's machine is offline
When: `zetl peer add --label alice --cap ./token.json`
Then: Peer registered with status "pending", no cached index, warning printed

Verifies: REQ-007
```

```
TEST-008: Peer Refresh (Delta Sync)

Scenario: No changes
Given: Bob has alice as a peer, last synced vault root = abc123
When: Alice's current vault root is abc123 (unchanged)
Then: zetl peer sync completes instantly, no data transferred

Scenario: Partial changes
Given: Alice's vault root changed, 2 of 100 pages modified
When: Bob runs `zetl peer sync alice`
Then: Only 2 files' metadata are fetched and cached

Verifies: REQ-008, REQ-013
```

```
TEST-009: Peer Removal

Scenario: Remove a peer
Given: Bob has alice as an active peer
When: Bob runs `zetl peer remove alice`
Then:
  - .zetl/peers/alice/ directory is deleted
  - Next `zetl index` rebuilds graph without Alice's pages
  - [[Topic A]] (previously resolved via alice) becomes a dead link
  - `zetl check` reports dead link with "previously resolved via peer 'alice'"

Verifies: REQ-009
```

```
TEST-010: Peer Listing

Scenario: List peers
Given: Bob has two peers (alice: active, carol: stale)
When: `zetl peer list --format json`
Then: JSON output lists both peers with label, status, last_sync,
     cached_pages, scope, ops

Verifies: REQ-010
```

```
TEST-011: Joined Graph Construction

Scenario: Graph includes peer nodes
Given: Bob's vault has 5 local pages, alice peer cache has 10 pages
When: `zetl index` is run
Then:
  - LinkGraph has 15+ nodes (local + peer, minus any shared names)
  - Peer nodes are marked with provenance (peer:alice)
  - Edges from local pages to peer pages exist where wikilinks resolve
  - Dead-link count is reduced (targets resolved via peer)

Verifies: REQ-011
```

```
TEST-012: Multi-Vault Link Resolution

Scenario: Local and peer both have same page name
Given: Bob has "Topic A" locally, alice peer also has "Topic A"
When: `zetl links "My Note"` (which contains [[Topic A]])
Then:
  - Output shows two matches: { vault: "local" } and { vault: "peer:alice" }
  - Graph traversal uses local "Topic A" as primary target
  - `zetl check` emits shadow diagnostic

Scenario: Only peer has the page
Given: Bob has no "Topic A" locally, alice peer has it
When: `zetl links "My Note"` (which contains [[Topic A]])
Then:
  - Output shows one match: { vault: "peer:alice" }
  - [[Topic A]] is NOT a dead link

Verifies: REQ-012
```

```
TEST-013: Merkle-Based Delta Sync

Scenario: Only changed files are fetched
Given: Bob has alice peer synced, vault root = abc123, 100 pages cached
When: Alice edits 2 pages (vault root becomes def456)
When: Bob runs `zetl peer sync alice`
Then:
  - System compares vault roots (abc123 ≠ def456)
  - System walks file-level Merkle hashes within capability scope
  - Only 2 files' metadata are fetched (not 100)
  - Cached index is updated with new data for the 2 changed pages
  - Unchanged pages retain their cached data

Scenario: Identical roots skip fetch entirely
Given: Bob has alice peer synced, vault root = abc123
When: Alice's vault root is still abc123
When: Bob runs `zetl peer sync alice`
Then:
  - Vault root comparison detects no change
  - No file-level hashes are walked
  - No data is transferred
  - Sync completes in O(1)

Verifies: REQ-013
```

```
TEST-014: Content Integrity Verification

Scenario: Valid signature accepted
Given: Bob has alice peer with cached vault root
When: Alice's peer engine serves a vault root signed with her Ed25519 key
When: Bob runs `zetl peer sync alice`
Then:
  - Signature is verified against Alice's public key (from capability token)
  - Sync proceeds normally
  - Cached data is updated

Scenario: Invalid signature rejected
Given: Bob has alice peer with cached vault root
When: Alice's peer engine serves a vault root with a tampered signature
When: Bob runs `zetl peer sync alice`
Then:
  - Signature verification fails
  - Sync is rejected — no cached data is updated
  - Peer is marked as 'untrusted'
  - Warning emitted: "peer 'alice' vault root signature invalid"

Verifies: REQ-014
```

```
TEST-015: Cross-Vault Drift Detection

Scenario: Peer page changed since local reference
Given: Bob's "Design.md" links to [[Topic A]] (alice peer, root=eee)
When: Alice edits Topic A (root becomes fff)
When: Bob runs `zetl peer sync alice` then `zetl check --drift`
Then: Drift diagnostic emitted:
  "linked page 'Topic A' (peer 'alice') has changed since this
   section was written"

Verifies: REQ-015
```

```
TEST-016: Offline Degradation

Scenario: All peers unreachable
Given: Bob has two peers configured, both machines are offline
When: `zetl index` is run
Then:
  - Command succeeds (exit 0)
  - Cached peer data is used
  - Warning on stderr: "peer 'alice' unreachable, using cached data"
  - No blocking, no retry loops
  - Latency within 200ms of baseline

Verifies: REQ-016, NFR-002, NFR-003
```

```
TEST-017: Phrase-Based Invite

Scenario: Successful invite/join exchange
Given: Alice has a vault with identity
When: Alice runs `zetl peer invite --scope "research/*" --ops graph`
Then:
  - A 4-word BIP39 phrase is displayed
  - The process waits for a connection
When: Bob runs `zetl peer join --phrase "tiger maple ocean drift" --label alice`
Then:
  - SPAKE2 handshake succeeds
  - Bob receives a valid capability token
  - Bob's .zetl/peers/alice/ is populated
  - Alice's invite session completes (exit 0)

Scenario: Wrong phrase
Given: Alice is running an invite session
When: Bob runs `zetl peer join --phrase "wrong words here now" --label alice`
Then:
  - SPAKE2 key mismatch detected
  - Bob gets exit code 1: "phrase mismatch"
  - Alice's invite session continues waiting (wrong attempt does not consume invite)

Scenario: Invite timeout
Given: Alice runs `zetl peer invite --timeout 10`
When: No peer connects within 10 seconds
Then:
  - Invite session exits with code 1: "timeout — no peer connected"
  - No capability was issued
  - Phrase is discarded

Verifies: REQ-017
```

```
TEST-018: Phrase-Based Join Produces Same Result as Token-Based Add

Scenario: Equivalence of join and add
Given: Alice creates a capability via `zetl peer grant` (token to file)
       Alice also starts an invite with the same scope/ops
When: Bob uses `zetl peer add --cap ./token.json --label alice-token`
      Carol uses `zetl peer join --phrase "..." --label alice-phrase`
Then:
  - Both Bob and Carol have .zetl/peers/<label>/cap.json
  - Both tokens have the same scope, ops, and granter
  - Both peers can sync, query links, and see Alice's pages identically
  - The only difference is the "method" field in status.json (token vs phrase)

Verifies: REQ-018
```

```
TEST-019a: File Materialization at read-content Tier

Scenario: Sync materializes files
Given: Alice grants Bob a read-content capability for scope "research/*"
       Alice's research/ folder contains 3 markdown files
When: Bob runs `zetl peer add --label alice --cap ./token.json`
      Bob runs `zetl peer sync alice`
Then:
  - .zetl/peers/alice/files/research/ exists
  - All 3 markdown files are present as real files
  - File contents match Alice's originals
  - Files are not included in Bob's local zetl index or Merkle tree
  - `zetl search "keyword"` finds matches in Alice's materialized files

Scenario: graph tier does NOT materialize files
Given: Alice grants Bob a graph-only capability
When: Bob adds and syncs
Then:
  - .zetl/peers/alice/files/ does NOT exist
  - .zetl/peers/alice/index.json contains metadata only

Verifies: REQ-019
```

```
TEST-019b: File Materialization Lifecycle

Scenario: Unjoin deletes materialized files
Given: Bob has alice peer with read-content, files materialized
When: Bob runs `zetl peer remove alice`
Then:
  - .zetl/peers/alice/ is completely deleted, including files/
  - No materialized files remain

Scenario: Sync updates materialized files
Given: Bob has alice peer synced, research/Topic A.md materialized
When: Alice edits Topic A.md (Merkle root changes)
When: Bob runs `zetl peer sync alice`
Then:
  - .zetl/peers/alice/files/research/Topic A.md is overwritten with new content
  - Unchanged files are not re-written

Scenario: Peer deletes a file
Given: Bob has alice peer synced, research/Old Note.md materialized
When: Alice deletes Old Note.md
When: Bob runs `zetl peer sync alice`
Then:
  - .zetl/peers/alice/files/research/Old Note.md is deleted locally

Scenario: Scope narrowing removes out-of-scope files
Given: Bob has alice peer with scope "research/**", files materialized
When: Alice replaces the capability with scope "research/public/*"
When: Bob syncs with the new capability
Then:
  - Files outside research/public/ are deleted
  - Files inside research/public/ are retained

Verifies: REQ-020
```

---

## 7. Observability

```
OBS-001: Peer Sync Log

All peer sync operations SHALL be logged to
.zetl/peers/sync-log.jsonl in append-only JSON-lines format:

  {"timestamp": "...", "op": "sync", "peer": "alice", "status": "updated",
   "pages_changed": 3, "files_materialized": 3, "elapsed_ms": 230}
  {"timestamp": "...", "op": "add", "peer": "bob", "status": "pending"}
  {"timestamp": "...", "op": "remove", "peer": "carol", "files_purged": 42}
```

```
OBS-002: Peer Health in Stats

`zetl stats` SHALL include a peers section when peers are configured:

  "peers": {
    "count": 2,
    "active": 1,
    "stale": 1,
    "total_cached_pages": 52,
    "last_sync": "2026-02-25T12:30:00Z"
  }
```

---

## 8. Research Spikes Required

Before implementation, the following unknowns require timeboxed prototypes:

### 8.1 Spike: Ed25519 Token Format

```
Hypothesis: A self-contained capability token format using Ed25519
            signatures can be implemented in Rust, supporting creation,
            verification, and attenuation chain validation, in <= 3 days.

Approach:
  - Implement token struct with serde serialization (canonical JSON)
  - Implement signing and verification using ed25519-dalek
  - Implement attenuation with delegation chain validation
  - Test: create token, tamper with fields, verify rejection
  - Evaluate UCAN specification compatibility

Timebox: 3 days
Success metric: Token creation, verification, and attenuation all work
                correctly with >= 10 test vectors including edge cases
Exit criteria: If Ed25519 canonical JSON signing proves fragile (JSON
               field ordering issues), evaluate CBOR or Syrup as the
               canonical serialization format instead

AI trust boundary: Cryptography — requires explicit review of signing
                   and verification implementations.
```

### 8.2 Spike: OcapN CapTP Minimal Handshake in Rust

```
Hypothesis: A minimal CapTP handshake (enough for sturdyref exchange
            and basic request/response) can be prototyped in Rust in
            <= 1 week.

Approach:
  - Study the OcapN CapTP specification
  - Evaluate existing Rust crates (ocapn-rs, if any)
  - Implement: Syrup serialization, CapTP handshake, sturdyref enliven
  - Test: Rust peer connects to Guile Goblins peer, exchanges messages
  - Determine minimum viable CapTP subset for peer sync

Timebox: 1 week
Success metric: Rust process can enliven a sturdyref hosted by a
                Goblins process and exchange messages
Exit criteria: If CapTP complexity exceeds timebox, fall back to
               simpler TCP+TLS with token-based auth (tokens provide
               security, TLS provides transport encryption)
```

### 8.3 Spike: Local Federation Prototype

```
Hypothesis: Two vaults on one machine can be joined via filesystem
            path (no network) to validate graph composition, multi-match
            resolution, and drift detection in <= 1 week.

Approach:
  - Implement PeerCache struct and loading from .zetl/peers/
  - Extend LinkGraph::build to accept peer caches
  - Implement multi-match resolution in link queries
  - Implement cross-vault drift detection
  - Test: create two vaults with overlapping wikilinks, join them,
    verify graph queries span both vaults
  - No networking — peers discovered via local filesystem paths

Timebox: 1 week
Success metric:
  - zetl links shows matches from both vaults
  - zetl backlinks spans both vaults
  - zetl check reports shadows and cross-vault drift
  - zetl peer remove cleanly unjoins (including materialized files)
  - read-content tier materializes files; zetl search finds peer content
Exit criteria: If graph composition introduces unacceptable complexity
               to LinkGraph::build, evaluate a separate FederatedGraph
               wrapper instead
```

### 8.4 Spike: SPAKE2 + BIP39 Phrase Exchange

```
Hypothesis: A SPAKE2-based capability exchange using BIP39 phrases
            can be implemented in Rust and successfully transfer an
            Ed25519-signed capability token between two peers using
            a 4-word phrase, in <= 3 days.

Approach:
  - Use the spake2 crate (by Brian Warner) for key exchange
  - Use the bip39 crate for mnemonic phrase generation
  - Implement: invite (listen + generate phrase), join (connect + use phrase)
  - Test: successful exchange, wrong phrase rejection, timeout
  - Evaluate magic-wormhole crate vs building on raw spake2 + bip39
  - Evaluate rendezvous server options (magic-wormhole mailbox server
    compatibility vs custom lightweight relay)

Timebox: 3 days
Success metric:
  - 4-word phrase exchange works reliably over TCP on LAN
  - Wrong phrase is rejected within 1 second
  - Resulting capability token is identical to token-based flow
  - No phrase or derived key material persists to disk
Exit criteria: If spake2 crate has compatibility issues or insufficient
               documentation, evaluate noise-protocol handshake with
               BIP39 pre-shared key as alternative

AI trust boundary: Cryptography — requires explicit review of SPAKE2
                   usage and key derivation.
```

### 8.5 Spike: Transport Layer Evaluation

```
Hypothesis: TCP+TLS with Ed25519 certificate pinning provides a
            sufficient transport for peer sync without requiring
            full OcapN networking.

Approach:
  - Prototype a minimal peer sync server (Rust, using rustls)
  - Pin TLS certificates to vault Ed25519 public keys
  - Implement: vault root request, page manifest request, delta fetch
  - Evaluate Tor onion service integration (arti crate)
  - Compare with QUIC (quinn crate) for multiplexed streams

Timebox: 3 days
Success metric: Two zetl instances can sync peer caches over
                encrypted TCP with mutual authentication
Exit criteria: If TLS certificate pinning to Ed25519 keys proves
               impractical, evaluate noise protocol framework instead
```

---

## 9. Implementation Roadmap

| Phase | Deliverable | Effort | Dependencies |
| --- | --- | --- | --- |
| **0. Spikes** | Token format, local federation prototype, SPAKE2+BIP39, transport eval | 2-3 weeks | ed25519-dalek, spake2, bip39 crates |
| **1. Identity** | `zetl init` generates Ed25519 keypair | 1-2 days | Spike 8.1 results |
| **2. Tokens** | `zetl peer grant`, `attenuate`, `revoke` | 3-5 days | Phase 1 |
| **3. Local federation** | `peer add/remove/list`, joined graph, multi-match, file materialization | 5-7 days | Phase 2, Spike 8.3 |
| **4. Merkle sync** | Delta sync via Merkle root comparison, integrity verification | 3-5 days | Phase 3 |
| **5. Networking** | TCP+TLS transport, peer discovery, remote sync | 5-7 days | Phase 4, Spike 8.2/8.5 |
| **5a. Phrase exchange** | `peer invite` / `peer join` via SPAKE2+BIP39 | 3-4 days | Phase 5, Spike 8.4 |
| **6. Cross-vault drift** | Drift detection across peer boundaries | 2-3 days | Phase 4 |
| **7. Polish** | Diagnostics, shadow warnings, staleness, error handling | 3-5 days | Phase 6 |

Total estimated effort: **25-38 days** (including spikes).

Phases 1-3 can be shipped as a useful local-only federation feature before networking is implemented. Users can join vaults on the same machine via filesystem paths.

---

## 10. Open Questions

1. **Should zetl support multi-hop capability delegation (Alice -> Bob -> Carol)?** Adds complexity but enables team topologies without requiring the original granter to issue every token. UCAN provides a proven model for delegation chains. Recommendation: support in token format from the start (chain field), but validate only single-hop initially.

2. **Transitive peer visibility?** If Alice peers with Bob, and Bob peers with Carol, should Alice see Carol's pages through Bob? Pure ocap says no — Alice has no capability for Carol's vault. But Bob could attenuate and re-delegate. Recommendation: no transitive visibility by default. If Bob wants to share Carol's pages with Alice, he explicitly attenuates Carol's token and grants it to Alice.

3. **SPL reasoning across vaults?** If Alice's theory references Bob's facts, and Bob's facts change, Alice's conclusions may be invalidated. The Merkle grounding mechanism (SPEC-006) handles drift detection within a vault; extending it across vaults requires the reasoning engine to understand peer provenance. Recommendation: defer to a future SPEC that builds on SPEC-005 + SPEC-011.

4. **How stale is too stale?** Should `zetl check` warn about peer caches older than N hours/days? Users in the solo-researcher profile may go weeks between syncs and consider it normal. Recommendation: configurable threshold (default 24h), suppressible per-peer.

5. **Should peer removal purge history?** When `zetl peer remove` is run, should the sync log entries be removed? Recommendation: keep the log (append-only audit trail), delete only the cached index and capability token.

6. **Key rotation?** If a vault's Ed25519 key is compromised, all previously granted capabilities must be re-issued. Should zetl support key rotation with a signed "succession" record? Recommendation: out of scope for v1. Document that key compromise requires manual re-keying and re-granting.

7. **What URI scheme for cross-vault wikilinks?** This spec keeps wikilinks as-is (`[[Page Name]]`) and resolves them via the joined graph. Should a future spec introduce explicit cross-vault syntax (e.g., `[[alice::Page]]` or `[[peer:alice/Page]]`)? This would break Obsidian compatibility but provide unambiguous cross-vault references. Recommendation: defer. Implicit resolution via the graph is sufficient and preserves file portability.

8. **Rendezvous server for phrase-based exchange?** Direct TCP connection works on LAN but not across NATs. The Magic Wormhole project provides public mailbox servers that relay SPAKE2 handshakes. Options: (a) use Magic Wormhole's public infrastructure, (b) host a lightweight relay, (c) support both direct and relay modes. Recommendation: start with direct connect (LAN only), add relay support in a follow-up. The `magic-wormhole` Rust crate includes relay client support if we choose to adopt it.

9. **How many words in the phrase?** 4 words (~44 bits) is sufficient for a 5-minute window against brute-force. For higher-security contexts (longer windows, untrusted networks), 6 words (~66 bits) may be preferred. Recommendation: default to 4, allow `--words 6` for paranoid users.

10. **Should `zetl search` automatically include peer files?** Materialized peer files at `read-content` tier are real files on disk, so search _could_ include them. Options: (a) always include, (b) opt-in via `--peers` flag, (c) separate `zetl search --scope peer:alice`. Recommendation: include by default with provenance annotation in results, allow `--local-only` to exclude. This matches the "show all matches" philosophy (ADR-003).

11. **Should materialized files be `.gitignore`d?** The `.zetl/` directory is typically gitignored already, so materialized files in `.zetl/peers/<label>/files/` are excluded by default. But if a user has a non-standard `.gitignore`, peer files could accidentally be committed. Recommendation: document that `.zetl/` should be in `.gitignore` (existing guidance), no additional action needed.

---

**END OF SPEC-011**
