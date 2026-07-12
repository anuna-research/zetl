---
id: SPEC-047
title: "Loro CRDT Store + P2P Realtime Sync Daemon with DHT-Bootstrapped SPAKE2 Pairing"
status: draft
version: 0.10.0-strawman
last-updated: 2026-07-12
---

# SPEC-047: Loro CRDT Store + P2P Realtime Sync Daemon with DHT-Bootstrapped SPAKE2 Pairing

> **Strawman notice.** A Phase-1 sketch produced during brainstorming, **before**
> the Phase 0 surveys and synthetic-user simulations in
> `plans/DESIGN-047-loro-p2p-realtime-sync.spl`. Sections tagged
> **`[Provisional — DESIGN-047 task X]`** are placeholders. **Do not implement
> against this version.** Per [[PROTO-001]] §AI Trust Boundaries this artefact
> touches three no-go areas — [[SPAKE2|cryptography]], the
> [[Authentication Core]], and a [[Threat Model|network trust boundary]] — making
> it **Tier-1**: cross-model adversarial review, the fresh-context comprehension
> gate, **and** human domain-expert approval are required before any code.

## Orientation

```
Intent:    Make a vault editable in realtime across a person's devices (and,
           later, a small team) with no central server — peers find each other
           and merge edits conflict-free using only the public BitTorrent DHT.
Metaphor:  A CB radio with a secret call-sign. You speak a short phrase once
           (the call-sign); after that each radio knows the other's voice and
           they talk directly on a scrambled channel — no operator, no
           switchboard.

           clients (CLI · zetl view · web · mobile)
                       │  CON-470  control: length-prefixed CBOR (LangSec)
                       ▼
   ┌──────────────────── zetld daemon ─────────────────────┐
   │  iroh / pkarr I/O          crdt::store (.zetl/loro/)   │
   │  CON-473 peer frames ─┐    persists Loro snapshot+oplog │
   │  CON-474 pairing      ├─AST▶┌──────────────────────────┐│
   │  control plane: CBCL  │     │ PURE CORE                ││
   │  DPDA (DCFL, §8.1);   │     │  proto: CBCL recognise   ││
   │  data plane: binary   │     │  pair: rendezvous·spake  ││
   │                       │     │  loro: materialise·import││
   │                       │     │  merkle: vault_root·diff ││ (SPEC-006)
   └───────────────────────┴─────┴──────────────────────────┘
                       │  reconcile: compare vault_root, then ship Loro ops
        pkarr ▶ Mainline DHT (discovery) ; QUIC ▶ peer zetld (sync)
   arrows point inward → core never imports shell (Purity Boundary Map §13)

Legend:    CBCL = the Lean-verified control-message language (ADR-479), whose
           deterministic pushdown recogniser (DPDA) accepts a decidable
           context-free language (DCFL) — the LangSec warrant, §8; roster =
           the per-vault list of admitted NodeIds (CON-477); CON-### =
           interface contracts (§7); ─AST▶ = only typed, validated parse
           output crosses into the pure core.

Decisions:    [[#ADR-470 Loro as Canonical Store Markdown+Git as Export]] ·
              [[#ADR-472 iroh + pkarr for Transport and Discovery]] ·
              [[#ADR-473 Phrase-Derived DHT Rendezvous for SPAKE2]] (crypto · human-gated, not rejected) ·
              [[#ADR-477 Single Per-Vault Group Key]] ·
              [[#ADR-478 Merkle DAG as Convergence Witness and Reconciliation Index]] ·
              [[#ADR-479 CBCL as the Control-Plane Message Language]]
Load-bearing: [[#REQ-491 SPAKE2 Channel Authentication]] (the pairing secret) ·
              [[#REQ-476 DHT-Bootstrapped SPAKE2 Pairing]] (rendezvous discovery) ·
              [[#REQ-477 Phrase OOB-Only Non-Leak]] ·
              [[#REQ-482 Roster-Gated Encrypted Transport]] ·
              [[#REQ-474 Conflict-Free Offline Merge]] ·
              [[#NFR-475 Pairing Secret Entropy Floor]]
Open:         threat-model §H rendezvous enumeration vs phrase entropy (owner: HOC, gating
              the Tier-1 crypto review) → [[#18. Open Questions]] Q1
Detail:       this document — the nodes below are the room; this block is the door.
```

## Conformance

The key words MUST, MUST NOT, REQUIRED, SHALL, SHALL NOT, SHOULD, SHOULD NOT,
RECOMMENDED, MAY, and OPTIONAL in this document are to be interpreted as
described in BCP 14 (RFC 2119, RFC 8174) when, and only when, they appear in all
capitals.

## Information Table

| Field        | Value                                                                                  |
| ------------ | -------------------------------------------------------------------------------------- |
| Document ID  | SPEC-047                                                                                |
| Title        | Loro CRDT Store + P2P Realtime Sync Daemon with DHT-Bootstrapped SPAKE2 Pairing         |
| Version      | 0.10.0-strawman                                                                          |
| Status       | Draft (strawman; pending DESIGN-047 execution)                                          |
| Author       | Agent (Claude Opus 4.8 [1M]) under [[PROTO-001]] v1.11.0                                |
| Audience     | Agent, Human                                                                            |
| Trace        | [[PROTO-001]] §Phase 1, §Phase 2, §AI Trust Boundaries                                  |
| Parent       | [[SPEC-020]] Multi-User Collaborative Editing                                           |
| Supersedes   | [[SPEC-004]] Distributed Sync (see [[#ADR-475 Supersede SPEC-004 Goblins OCapN Sidecar]]) |
| Related      | [[SPEC-034]], [[SPEC-036-spake2-onboarding]], [[SPEC-040-zetl-mobile]], [[SPEC-041-pluggable-collab-auth]] |
| Plan         | `plans/DESIGN-047-loro-p2p-realtime-sync.spl`                                           |
| Review tier  | Tier 1 (cryptography + authentication core + network trust boundary)                    |

---

## 1. Overview

This specification sketches a **greenfield** realtime, multiplayer backend for
[[zetl]] vaults built on three pillars: a persistent local [[Daemon]] (`zetld`)
that owns vault state and network sessions; a [[Loro]] [[CRDT]] store as the
*canonical* content representation (replacing the [[diamond-types]] dual-oplog
engine), with Markdown + git as a deterministic export; and a [[P2P]] transport
over [[iroh]] ([[QUIC]] + [[NAT Traversal]]) where peer identity *is* an ed25519
[[NodeId]], discovery runs over [[pkarr]] records on the [[Mainline DHT]], and
trust is bootstrapped by a [[SPAKE2]] pairing ceremony whose rendezvous is itself
located on the DHT. There is no central server: no editing hub and no
zetl-operated [[Relay]] (relays are an optional fallback —
[[#ADR-474 Relay as Optional Fallback Not Requirement]]).

### 1.1 Motivation

The vault is today single-machine in its realtime story: the [[WebSocket]]
editing layer is hub-and-spoke, the [[CRDT]] is rebuilt from Markdown each
session (cross-session causal history is lost), and inter-machine sync is
git-centric. [[SPEC-004]] anticipated [[P2P]] sync via a Goblins/OCapN sidecar
but was never implemented; [[SPEC-036-spake2-onboarding]] generalised the
[[SPAKE2]] primitive but kept it server-mediated. A daemon + [[Loro]] + [[iroh]]
+ [[pkarr]] buys, in one move: offline-first conflict-free merge, serverless
reachability, identity = address = key, and a spoken-phrase onboarding that needs
no shared infrastructure.

### 1.2 Design Principles

1. The daemon is the single owner of state and sockets; clients attach over a
   local control channel.
2. [[Loro]] is canonical; Markdown + git is a materialised export (external
   edits handled by guarded import —
   [[#ADR-471 Guarded Import for External Markdown Edits]]).
3. The [[Pairing Phrase]] MUST NOT travel in any URL, argv, env, or network
   message — only a phrase-*derived* [[Rendezvous]] locator and [[SPAKE2]]
   messages reach the network.
4. Pairing entropy is the enumeration-resistance budget (the rendezvous locator
   is phrase-derived; phrase entropy bounds DHT discoverability of in-flight
   pairings).
5. All input at a trust boundary is recognised against a declared grammar before
   any action ([[PROTO-001]] Principle 14; see [[#8. Input Grammars (LangSec)]]).
6. Failure UX MUST NOT leak protocol-level distinctions.
7. Encryption in transit is mandatory; [[Encryption at Rest]] is opt-in
   ([[#ADR-476 Encryption at Rest is Opt-In]]).

### 1.3 Scope

**In scope:** the `zetld` daemon + local control plane; a [[Loro]]-backed store
with deterministic [[Materialization]] to Markdown; a [[P2P]] sync engine
([[Version Vector]] delta sync + [[Ephemeral Store]] presence); [[pkarr]]/[[Mainline DHT]]
discovery; serverless [[SPAKE2]] pairing into a per-vault [[Group Key]]; CLI
under the existing groups — `zetl daemon {start,stop,status}` (mirroring
`zetl serve`) and `zetl collab {pair,join,peers,revoke}` (extending the existing
`zetl collab` group, reusing SPEC-036's `zetl collab join`) per
[[#ADR-480 CLI Surface Follows Existing zetl Conventions]]; input grammars,
threat model, observability.

**Out of scope (successor specs):** pairwise web-of-trust ACLs
([[#ADR-477 Single Per-Vault Group Key]]); multi-user as a *v1* milestone
(machinery supports it; v1 targets multi-device); migration tooling from
[[diamond-types]]; mobile daemon packaging beyond noted constraints; relay
operation/federation; **runtime dialect gossip** — v1 ships the [[CBCL]] dialects
with the CLI binary, peer-to-peer dialect propagation is deferred
([[#18. Open Questions]] Q10).

---

## 2. User Profiles

> **`[Provisional — DESIGN-047 task user-profiles]`** [[PROTO-001]]
> §Synthetic User Protocol template; canonical copies at `/users/{group}/user.md`.

### [[users/solo-multi-device/user|Solo Multi-Device User]] (primary; Milestone 1)

- **Role:** Individual running [[zetl]] on ≥ 2 personal devices.
- **Goals:** Edits on one device appear on the others; no git mechanics; no
  self-hosted server; no conflicts after concurrent offline edits.
- **Constraints:** CLI-fluent on desktop; mobile shell ([[SPEC-040-zetl-mobile]])
  on phone; data MUST NOT traverse a third party in plaintext.
- **Daily workflow:** open vault → edit across devices → expect convergence.

### [[users/team-collaborator/user|Small-Team Collaborator]] (Milestone 2)

- **Role:** Peer co-editing a shared vault in realtime.
- **Goals:** Live co-editing, visible presence, conflict-free merge.
- **Constraints:** Already shares a private channel with the owner at join time;
  copies a short [[Pairing Phrase]] in ≤ 30 s.
- **Daily workflow:** join once → co-edit → leave.

### [[users/vault-owner/user|Vault Owner / Admin]]

- **Role:** Controls vault [[Group Key]] membership.
- **Goals:** Pair devices/peers; audit the roster; revoke so sync actually stops
  (enforceable among honest daemons — membership authority is local policy, not
  cryptography, until the web-of-trust successor; [[#12. Threat Model]] §N).
- **Constraints:** Owns the vault; drives `zetl collab pair`/`zetl collab revoke`.
- **Daily workflow:** pair on demand → audit `zetl collab peers` → revoke on loss.

### [[users/operator/user|Operator]] (self-host; optional)

- **Role:** Optionally runs an [[iroh]] [[Relay]] / [[pkarr]] republisher.
- **Goals:** Reachability behind hostile NATs.
- **Constraints:** Empty in the default deployment (public [[Mainline DHT]] +
  iroh default relays suffice).
- **Daily workflow:** n/a in default deployment.

---

## 3. Happy Paths

> **`[Provisional — DESIGN-047 task happy-paths]`** [[PROTO-001]] Happy-Path
> template (`action → expected response`). The plan task runs the synthetic-user
> simulation and expands failure modes into REQs.

### HP1: Pair a Second Device (Solo Multi-Device) — Milestone 1

**Preconditions:** `zetld` running on Device A with vault `notes`; Device B has
[[zetl]] + `zetld`, no vault; same operator can read a phrase from A into B.

**Steps:**

1. `zetl collab pair --vault notes` on A → daemon mints a `num-word-word`
   [[Pairing Phrase]] (e.g. `7-walnut-harbor`), prints it to stdout + a "waiting"
   status to stderr, derives the [[Rendezvous]] keypair, and publishes a short-TTL
   [[pkarr]] record at the rendezvous pubkey pointing at A's [[iroh]] endpoint.
2. `zetl collab join` on B → CLI prompts for the phrase via TTY (never argv/env,
   mirroring `zetl collab passwd add`).
3. Operator types `7-walnut-harbor` into B → B derives the same rendezvous
   keypair and resolves the record from the [[Mainline DHT]] → learns A's
   endpoint.
4. B opens a [[QUIC]] stream to A → A and B run [[SPAKE2]] (phrase = password) →
   shared session key; mutual key confirmation ([[HMAC]] over the transcript,
   both directions) completes before any key material is sealed (F41).
5. A seals the vault [[Group Key]] to B and both exchange durable [[NodeId]]s →
   each writes the other into the roster.
6. B initialises `notes`, pulls [[Loro]] state via [[Version Vector]] delta sync,
   materialises Markdown → "synced".
7. Rendezvous record torn down; phrase consumed → later discovery uses durable
   [[NodeId]] via [[pkarr]], no phrase.

**Postconditions:** Both devices hold identical content + [[Group Key]]; either
edits offline and merges on reconnect.

**Failure modes:** wrong phrase → generic auth failure
([[#REQ-479 Failure-Message Indistinguishability]]); rendezvous absent/expired →
generic auth failure; symmetric NAT, no relay → reachability error distinct from
auth ([[#ADR-474 Relay as Optional Fallback Not Requirement]]); phrase reused →
generic auth failure ([[#REQ-478 Single-Use Phrase]]); third-party wrong-phrase
attempt → phrase consumed (first-exchange-consumes,
[[#REQ-478 Single-Use Phrase]]) → owner's `pair` reports failure and the owner
re-pairs with a fresh phrase ([[#12. Threat Model]] §M).

### HP2: Realtime Co-Editing After Pairing

**Preconditions:** A and B paired, both online.

**Steps:**

1. A opens `notes/Daily.md` in `zetl view` → [[TUI]] attaches to `zetld`.
2. B opens the same note → each daemon publishes cursor via the
   [[Ephemeral Store]].
3. A types → A's daemon appends [[Loro]] ops, streams the delta → B applies it,
   renders A's text + cursor within the [[Doherty Threshold]]
   ([[#NFR-477 Remote Edit Propagation Latency]]).
4. Both edit concurrently → [[Loro]] [[Peritext]] merge, no conflict markers.
5. On quiescence → each daemon materialises Markdown + runs the git/jj flush →
   identical content hashes
   ([[#REQ-473 Deterministic Materialisation]]).

**Postconditions:** Identical [[Loro]] state + Markdown; git records the merge
with per-contributor attribution.

**Failure modes:** transient disconnect → ops buffered, replayed (HP3); clock
skew → irrelevant ([[Loro]] uses logical ordering).

### HP3: Offline Divergence + Reconvergence

**Preconditions:** A and B paired; both offline; each edits the same note.

**Steps:**

1. Each daemon persists its oplog locally → no edit lost.
2. On reconnect, peers rediscover via [[pkarr]], exchange [[Version Vector]]s →
   compute missing ops both ways.
3. Each applies received ops → converges, no marker.

**Postconditions:** Identical [[Loro]] state → one deterministic Markdown.

**Failure modes:** one oplog truncated by compaction → full-snapshot sync
fallback ([[#18. Open Questions]] Q4).

### HP4: Revoke a Peer

**Preconditions:** Owner paired a now-lost device into the [[Group Key]].

**Steps:**

1. `zetl collab revoke <nodeid>` → daemon rotates the [[Group Key]] epoch.
2. Daemon re-seals the new key to survivors on next contact → removes the
   revoked [[NodeId]] from the roster.
3. Daemon refuses [[QUIC]] from the revoked [[NodeId]] → writes a tombstone.

**Postconditions:** Revoked peer cannot decrypt post-revocation sync nor re-pair
without a fresh phrase.

**Failure modes:** revoke sole member → rejected (`last-member`); survivor
offline → re-seal deferred ([[#NFR-474 Revocation Propagation]]) — an unresealed
survivor can still sync *new* content with the revoked peer until it contacts a
resealed member ([[#12. Threat Model]] §O).

### HP5: External Markdown Edit (`git pull`, editor write)

**Preconditions:** `notes/Daily.md` changed on disk by a non-daemon actor.

**Steps:**

1. Watcher detects the change → diffs on-disk Markdown vs last export.
2. No concurrent daemon edit → fold the delta into [[Loro]] → "imported".
3. Concurrent daemon edit → stage to `.zetl/sync/conflicts/` → surface.

**Postconditions:** External edits never lost; [[Loro]] causality intact;
conflicts surfaced not hidden.

**Failure modes:** binary/non-UTF-8 write → staged, never folded; rapid churn →
coalesced per debounce window; NFD-encoded write → folded, then re-materialised
in canonical NFC (on-disk bytes differ from what the editor wrote; the import
diff compares under the canonical form so the fold→materialise→watch cycle
terminates — [[#ADR-471 Guarded Import for External Markdown Edits]], F42);
external delete/rename → guarded like a write
([[#REQ-484 Guarded Import of External Markdown Edits]], F38).

---

## 4. Functional Requirements

> [[PROTO-001]] §Requirement Templates, with BCP-14 atomicity (one obligation
> keyword per REQ). IDs use the SPEC-047 block (470+); finalised by DESIGN-047
> task `draft-requirements`.

### REQ-470: Persistent Daemon Ownership

The system SHALL run a persistent daemon `zetld` that owns all vault [[CRDT]]
state and [[P2P]] sockets independently of any client process, FOR the
[[users/solo-multi-device/user|Solo Multi-Device User]] and
[[users/team-collaborator/user|Collaborator]], WITH every vault mutation flowing
through the daemon and no client mutating `.zetl/loro/` directly.

> Atomicity (F5): the durability-across-disconnect obligation is split out to
> [[#REQ-490 Daemon Survives Client Disconnection]].

**Trace:** [[#TEST-470a]], [[#CON-470 Daemon Control Channel]], [[#OBS-470 Daemon health]].

### REQ-471: Idempotent Control Lifecycle

The system SHALL expose start/stop/status/attach control operations that are
idempotent and return a machine-readable daemon state, FOR clients of the control
channel, WITH a repeated operation producing the same state as a single
invocation.

**Trace:** [[#TEST-471a]], [[#TEST-471b]], [[#TEST-471c]], [[#CON-470 Daemon Control Channel]], [[#OBS-470 Daemon health]].

### REQ-472: Loro Canonical Store

The system SHALL persist each vault document as a [[Loro]] [[CRDT]] (snapshot +
oplog) as the canonical representation, FOR all vault content, WITH causal
history surviving daemon restart and crash — an op is **committed** once
fsync-appended to the oplog (`[Provisional]` policy; committed ops survive
`SIGKILL`/power loss, and "committed" in
[[#REQ-490 Daemon Survives Client Disconnection]] and HP3 means exactly this;
the [[WAL]]'s durability contract transfers here — F39).

**Trace:** [[#TEST-472a]], [[#TEST-472c]], [[#TEST-472d]], [[#CON-471 Loro Store and Materialisation]], [[#OBS-471 Materialisation]].

### REQ-473: Deterministic Materialisation

The system SHALL materialise each [[Loro]] document to Markdown under a **declared
canonical form** — fixed line ending (LF), Unicode NFC, fixed frontmatter key
order, codepoint-collated file ordering, and a fixed trailing-whitespace policy —
such that identical [[Loro]] state yields byte-identical Markdown (and identical
Merkle hash), FOR all vault content, WITH zero variance across runs, devices, and
host OS (the canonical form is the falsifiable predicate — not an unscoped "100%";
F11).

**Trace:** [[#TEST-473a]], [[#TEST-473c]], [[#CON-471 Loro Store and Materialisation]], [[#OBS-471 Materialisation]].

### REQ-474: Conflict-Free Offline Merge

The system SHALL merge concurrent edits made on disconnected peers to the same
document without conflict markers and without losing any committed edit, FOR any
two paired peers, WITH convergence to identical [[Loro]] state once both have
exchanged their full op sets via [[Version Vector]] reconciliation.

**Trace:** [[#TEST-474a]], [[#TEST-474c]], [[#CON-473 Peer Session]], [[#OBS-472 Sync convergence]].

### REQ-475: Serverless Peer Discovery

The system SHALL resolve a paired peer's current network endpoint from that
peer's durable ed25519 [[NodeId]] using [[pkarr]] records
([[#8.9 pkarr Rendezvous Record]]) on the [[Mainline DHT]] without any
zetl-operated server, FOR any two paired peers, WITHIN
[[#NFR-472 Reconnect Discovery Latency]] and tolerating peer IP changes between
sessions (a resolved record is an unauthenticated hint until roster-[[NodeId]]
verification — [[#REQ-492 Roster Gate Before Vault Frame]]).

**Trace:** [[#TEST-475a]], [[#TEST-475b]], [[#CON-473 Peer Session]], [[#OBS-473 Discovery]].

### REQ-476: DHT-Bootstrapped SPAKE2 Pairing

The system SHALL discover a joining peer during pairing by deriving a
[[Rendezvous]] locator from the `num` (routing) component of the
[[Pairing Phrase]] and resolving the corresponding [[pkarr]] record, FOR the
[[users/vault-owner/user|Owner]] and a joining peer, WHEN both are online.

> Atomicity (F6): this REQ is now *discovery only*. The channel-authentication
> obligation is [[#REQ-491 SPAKE2 Channel Authentication]]; key-seal-on-success is
> [[#REQ-480 Group-Key Admission Gate]]; the latency bound is
> [[#NFR-471 Pairing Completion Latency]].

**Trace:** [[#TEST-476a]], [[#CON-474 Pairing Protocol]], [[#OBS-474 Pairing]].

### REQ-477: Phrase OOB-Only Non-Leak

The system SHALL confine the [[Pairing Phrase]] to TTY entry such that it appears
in no URL, argv visible to other processes, environment variable, or network
message (only a phrase-derived [[Rendezvous]] locator and [[SPAKE2]] messages
reach the network), FOR every pairing, WITH a phrase supplied by any non-TTY
channel refused.

**Trace:** [[#TEST-477a]], [[#TEST-477b]], [[#TEST-477c]], [[#CON-474 Pairing Protocol]], [[#OBS-475 Pairing failure cause]].

### REQ-478: Single-Use Phrase

The system SHALL consume each [[Pairing Phrase]] on its first [[SPAKE2]]
exchange, completed **or failed** — a *redemption* is any SPAKE2 exchange
initiated against the phrase's rendezvous, regardless of outcome (F32) — FOR
every pairing, WITH any subsequent redemption attempt producing the generic
failure of [[#REQ-479 Failure-Message Indistinguishability]].
First-exchange-consumption preserves the one-online-guess bound on the
`word-word` secret ([[#NFR-475 Pairing Secret Entropy Floor]]); the pairing-burn
DoS it admits is a documented residual, visible to the owner, who re-pairs with
a fresh phrase ([[#12. Threat Model]] §M).

**Trace:** [[#TEST-478a]], [[#TEST-478b]], [[#CON-474 Pairing Protocol]], [[#OBS-475 Pairing failure cause]].

### REQ-479: Failure-Message Indistinguishability

The system SHALL emit identical user-visible error text for the distinct internal
pairing-failure causes (wrong phrase, expired/absent rendezvous, replayed phrase,
[[SPAKE2]] verification failure, rate-limited), FOR every failed pairing, WITH
only the operator-channel log distinguishing the cause and the response-time
delta bounded by [[#NFR-473 Failure-Cause Timing Indistinguishability]].

**Trace:** [[#TEST-479a]], [[#TEST-479c]], [[#CON-474 Pairing Protocol]], [[#OBS-476 Pairing outcome log]].

### REQ-480: Group-Key Admission Gate

The system SHALL grant vault [[Group Key]] membership only on completion of a
[[SPAKE2]] pairing ([[#REQ-476 DHT-Bootstrapped SPAKE2 Pairing]]), FOR every
admitted peer, WITH any admission attempt lacking a completed pairing rejected.

**Trace:** [[#TEST-480a]], [[#TEST-480b]], [[#CON-477 Group Key Roster and Revocation]], [[#OBS-477 Roster audit]].

### REQ-481: Revocation by Key Rotation

The system SHALL revoke a peer by rotating the [[Group Key]] epoch and re-sealing
it to surviving roster members, FOR the [[users/vault-owner/user|Owner]], WITHIN
[[#NFR-474 Revocation Propagation]] and such that the revoked [[NodeId]] cannot
decrypt post-revocation sync.

**Trace:** [[#TEST-481a]], [[#TEST-481b]], [[#TEST-481c]], [[#CON-477 Group Key Roster and Revocation]], [[#OBS-477 Roster audit]].

### REQ-482: Roster-Gated Encrypted Transport

The system SHALL exchange vault content only over a mutually authenticated,
encrypted [[QUIC]] channel ([[iroh]]), FOR every peer session, WITH presence and
sync frames never sent in cleartext.

> Atomicity (F7): the roster-admission-before-frame obligation is split out to
> [[#REQ-492 Roster Gate Before Vault Frame]].

**Trace:** [[#TEST-482a]], [[#TEST-482b]], [[#CON-473 Peer Session]], [[#OBS-478 Off-roster rejections]].

### REQ-483: Full Recognition at Trust Boundaries (LangSec)

The system SHALL fully recognise every network and control input against its
declared grammar ([[#8. Input Grammars (LangSec)]]) before taking any semantic
action on it, FOR all inputs crossing a trust boundary, WITH malformed input
rejected (fail-closed) and never repaired or partially acted upon
([[PROTO-001]] Principle 14).

**Trace:** [[#TEST-483a]], [[#TEST-483b]], [[#TEST-483c]], [[#CON-470 Daemon Control Channel]], [[#CON-473 Peer Session]], [[#CON-474 Pairing Protocol]], [[#OBS-478 Off-roster rejections]].

### REQ-484: Guarded Import of External Markdown Edits

The system SHALL route each external (non-daemon) Markdown write, delete, or
rename per the guarded-import decision — folding it into the canonical [[Loro]]
store when no unmaterialised daemon op exists for that document since
`last_export` (a delete folds as a document tombstone; a rename is treated as
delete + create pending identity-preserving refinement by DESIGN-047
`adr-external-edits` — F38), and staging it to the conflict area otherwise —
FOR every external write, delete, or rename, WITH neither side
silently discarded. The predicate is defined over [[Loro]] *logical* time
(presence of an unmaterialised op), NOT a wall-clock debounce window, so it is not
a timing-controllable data-authority oracle (F16).

**Trace:** [[#TEST-484a]], [[#TEST-484b]], [[#TEST-484c]], [[#CON-471 Loro Store and Materialisation]], [[#OBS-479 External-edit import]].

### REQ-485: Merkle Convergence Witness

The system SHALL produce identical [[Merkle Vault Root]]s (the [[SPEC-006]]
BLAKE3 `vault_root` over the materialised AST) on any two peers that have
converged to the same [[Loro]] state, FOR every pair of converged peers, WITH a
root mismatch between peers that [[Loro]] reports as converged raised as an
integrity alarm rather than silently tolerated.

> Depends on canonical materialisation
> ([[#REQ-473 Deterministic Materialisation]]) and deterministic AST chunking
> ([[SPEC-006]]). The [[Merkle Vault Root]] witnesses **agreement on bytes**
> between honest peers — it detects divergence/corruption, NOT content
> *authorisation*: a malicious member can author valid ops that materialise to
> poisoned-but-converged state, which the witness will equally "confirm" (F10,
> threat §L). The witness cross-checks merge; it never substitutes for trust in
> the authoring member.

**Trace:** [[#TEST-485a]], [[#TEST-485c]], [[#CON-473 Peer Session]], [[#OBS-480 Convergence witness]].

### REQ-486: Merkle Anti-Entropy Reconciliation

The system SHALL reconcile two peers by first comparing their
[[Merkle Vault Root]]s and, on mismatch, descending the [[Merkle DAG]]
(path-sorted file roots → block leaves) to localise the differing documents
before exchanging [[Loro]] op deltas for only those documents, FOR every sync
session, WITH a session completing without any op exchange only when both the
roots and the peers' [[Loro]] [[Version Vector]]s are equal — a `vault_root` is
computed at materialisation and may lag committed unmaterialised ops, so equal
roots alone do not witness equal [[Loro]] state (F33).

**Trace:** [[#TEST-486a]], [[#TEST-486b]], [[#TEST-486c]], [[#CON-473 Peer Session]], [[#OBS-480 Convergence witness]].

### REQ-487: Control-Plane Messages Recognised by the CBCL DPDA

The system SHALL recognise every control-plane message (daemon control verbs,
pairing/reconcile choreography, presence, signed-root announcement) as a
[[CBCL]] message parsed by the shared deterministic pushdown automaton
([[DCFL]], parser-equivalence) before any semantic action, FOR all control-plane
inputs at a trust boundary, WITH every *network* control-plane message carrying
a [[CBCL]] **R4** Ed25519 signature verified against the sending peer's roster
[[NodeId]] (local control-socket messages authenticate by the socket's
filesystem permissions instead — [[#CON-470 Daemon Control Channel]]) and the
shipped dialects validated (R1–R5) at load with any `Invalid` dialect refused
(dialects are release artefacts of the binary — not roster-signed, not installed
from peers; F37; [[#ADR-479 CBCL as the Control-Plane Message Language]]).

> Specialises [[#REQ-483 Full Recognition at Trust Boundaries]]: [[CBCL]] is the
> *mechanism* — one Lean-verified recogniser for all control messages, so
> parser-differential attacks are excluded by construction (DCFL parser
> equivalence). Opaque binary payloads ([[#8.2 Peer Sync Frame]],
> [[#8.6 SPAKE2 Frame]]) ride on the data plane, referenced by id from the
> control message, recognised by their own decoders.

**Trace:** [[#TEST-487a]], [[#TEST-487b]], [[#CON-470 Daemon Control Channel]], [[#CON-473 Peer Session]], [[#CON-474 Pairing Protocol]], [[#OBS-481 Dialect verification]].

### REQ-488: Choreographies as Verified R5 Causal-Protocol Contracts

The system SHALL express the pairing choreography (hello → [[SPAKE2]] exchange →
[[Group Key]] seal) and the reconcile choreography ([[Merkle Vault Root]] compare
→ [[Merkle DAG]] descent → op exchange) as [[CBCL]] **R5** `(protocol …)`
causal-protocol contracts, FOR every pairing and sync session, WITH each contract
verified at build time over the shipped dialect (acyclic, reachable from `begin`,
performatives defined) so an out-of-order or undefined-step message is rejected.

**Trace:** [[#TEST-488a]], [[#TEST-488b]], [[#CON-474 Pairing Protocol]], [[#CON-473 Peer Session]], [[#OBS-481 Dialect verification]].

### REQ-489: P2P CLI Follows Existing zetl Conventions

The system SHALL expose the P2P verbs under the existing `zetl collab` group and a
`zetl daemon` group (not bare top-level commands) honouring the global
`--format`/`--json` output selection, the `--vault` selector, positional ids, and
the standard non-zero-exit-on-error convention, FOR all P2P CLI surface, WITH any
not-yet-implemented verb exiting non-zero with a `not-yet-implemented` diagnostic
([[#ADR-480 CLI Surface Follows Existing zetl Conventions]]; the
[[Pairing Phrase]]'s TTY-only entry is carried by
[[#REQ-477 Phrase OOB-Only Non-Leak]]).

**Trace:** [[#TEST-489a]], [[#TEST-489b]], [[#CON-470 Daemon Control Channel]], [[#CON-474 Pairing Protocol]], [[#CON-477 Group Key Roster and Revocation]].

> **Split-off & added requirements (review F5/F6/F7/F9/F12).** Numbered 490+ to
> keep existing ids stable (sequential, no reuse — [[PROTO-001]] §Numbering).

### REQ-490: Daemon Survives Client Disconnection

The system SHALL preserve all committed vault state across client disconnection
and client/daemon detachment, FOR every attached client, WITH the daemon
continuing to run and no committed [[Loro]] op lost when a client exits.

**Trace:** [[#TEST-490a]], [[#TEST-490c]], [[#CON-470 Daemon Control Channel]], [[#OBS-470 Daemon health]].

### REQ-491: SPAKE2 Channel Authentication

The system SHALL authenticate the pairing [[QUIC]] channel by a [[SPAKE2]]
handshake whose password is the `word-word` (secret) component of the
[[Pairing Phrase]], FOR every pairing, WITH key agreement failing closed when
the words do not match (mutual key confirmation — both directions over the
transcript — completes before the [[Group Key]] is sealed, and sealing occurs
only on success per [[#REQ-480 Group-Key Admission Gate]]; F41).

**Trace:** [[#TEST-491a]], [[#TEST-476b]], [[#TEST-476c]], [[#CON-474 Pairing Protocol]], [[#OBS-474 Pairing]].

### REQ-492: Roster Gate Before Vault Frame

The system SHALL verify a connecting peer's durable [[NodeId]] against the vault
roster before parsing any vault frame, FOR every peer session, WITH off-roster
[[NodeId]]s rejected pre-frame (the recognition-before-action ordering of
[[#REQ-483 Full Recognition at Trust Boundaries]] applied to access control).

**Trace:** [[#TEST-492a]], [[#TEST-492b]], [[#CON-473 Peer Session]], [[#OBS-478 Off-roster rejections]].

### REQ-493: Signed-Root Epoch Binding

The system SHALL reject a [[#8.8 Signed Vault Root]] whose `key_epoch` differs
from the verifier's current [[Group Key]] epoch, FOR every received signed root,
WITH a stale-epoch root from an otherwise-valid (incl. since-revoked) signer
treated as a non-witness — closing cross-epoch replay by a revoked member whose
durable ed25519 [[NodeId]] still verifies (F9, threat §K).

**Trace:** [[#TEST-493a]], [[#TEST-493b]], [[#CON-473 Peer Session]], [[#OBS-477 Roster audit]].

### REQ-494: Control-to-Data Binding

The system SHALL bind each opaque data-plane payload ([[#8.2 Peer Sync Frame]],
[[#8.6 SPAKE2 Frame]]) to its referencing [[CBCL]] control message by a
**content hash** carried in the control message and verified before the payload
is interpreted, FOR every control→data reference, WITH a substituted or replayed
payload rejected (no bare-id binding — F12).

**Trace:** [[#TEST-494a]], [[#TEST-494b]], [[#CON-473 Peer Session]], [[#OBS-478 Off-roster rejections]].

### REQ-495: Signed-Root Freshness

The system SHALL reject a [[#8.8 Signed Vault Root]] whose `root_seq` is ≤ the
last `root_seq` accepted from that signer, FOR every received signed root, WITH a
captured current-epoch root that is re-presented (same-epoch replay) treated as a
non-witness (F29, threat §K) — complementing the epoch check of
[[#REQ-493 Signed-Root Epoch Binding]].

**Trace:** [[#TEST-495a]], [[#TEST-495b]], [[#CON-473 Peer Session]], [[#OBS-477 Roster audit]].

### REQ-496: Pairing Attempt Rate Limit

The system SHALL rate-limit inbound pairing at the daemon — at most one
[[SPAKE2]] exchange per minted [[Pairing Phrase]]
([[#REQ-478 Single-Use Phrase]]) and ≤ `[Provisional: 5]` connection attempts
per minute at an open [[Rendezvous]], excess dropped pre-handshake — FOR every
open pairing, WITH budget exhaustion aborting the pairing, tearing down the
rendezvous record, and surfacing only the generic failure of
[[#REQ-479 Failure-Message Indistinguishability]]. This is the "daemon-side
rate-limiting" that [[#ADR-473 Phrase-Derived DHT Rendezvous for SPAKE2]] and
[[#NFR-475 Pairing Secret Entropy Floor]] cite as load-bearing for the ~22-bit
secret, previously unspecified (F31).

**Trace:** [[#TEST-496a]], [[#TEST-496b]], [[#CON-474 Pairing Protocol]], [[#OBS-475 Pairing failure cause]].

---

## 5. Non-Functional Requirements

> [[PROTO-001]]: `[Attribute] SHALL be [metric] UNDER [conditions] WITH
> [percentile]`. Thresholds attach to the dominant (no-options) usage profile.

### NFR-470: Local Edit-to-Render Latency

Local feedback for a keystroke (edit applied + re-rendered in an attached client)
SHALL be ≤ 16 ms UNDER a single-vault interactive session WITH 95th percentile.

**Trace:** [[#TEST-NFR-470]]; [[#OBS-470 Daemon health]].

### NFR-471: Pairing Completion Latency

Pairing time (HP1 step 2 → step 6) SHALL be ≤ 10 s UNDER nominal broadband AND
≤ 30 s UNDER adverse-NAT-with-relay conditions, each WITH 95th percentile.

**Trace:** [[#TEST-NFR-471]]; [[#OBS-474 Pairing]].

### NFR-472: Reconnect Discovery Latency

Endpoint rediscovery via [[pkarr]] after a network change SHALL be ≤ 15 s UNDER
nominal connectivity WITH 95th percentile.

**Trace:** [[#TEST-NFR-472]]; [[#OBS-473 Discovery]].

### NFR-473: Failure-Cause Timing Indistinguishability

User-visible response time across the *post-connection*
[[#REQ-479 Failure-Message Indistinguishability]] causes (wrong phrase, replayed
phrase, [[SPAKE2]] verification failure, rate-limited) SHALL be
indistinguishable to within `[Provisional: 50 ms]` UNDER LAN conditions WITH
95th percentile — implemented by padding to the slowest in-set cause. The
rendezvous-absent/expired cause is excluded from the *timing* bound: it fails
pre-connection at the DHT, whose records any DHT client can query anyway, so
timing-hiding it buys nothing (F40); REQ-479's identical user-visible *text*
still covers it.

**Trace:** [[#TEST-NFR-473]]; [[#REQ-479 Failure-Message Indistinguishability]].

### NFR-474: Revocation Propagation

[[Group Key]] rotation SHALL propagate to all online surviving roster members
WITHIN 60 s of `zetl collab revoke` completing UNDER nominal connectivity WITH
99th percentile.

**Trace:** [[#TEST-NFR-474]]; [[#REQ-481 Revocation by Key Rotation]].

### NFR-475: Pairing Secret Entropy Floor

The [[SPAKE2]] **secret** component (`word-word`) of the [[Pairing Phrase]] SHALL
carry ≥ `[Provisional: 22 bits]` of entropy (two BIP39 words) UNDER every
generation path WITH 100% conformance — this is the *sole* security floor,
adequate by [[SPAKE2]] online-single-guess + single-use
([[#REQ-478 Single-Use Phrase]]) + [[#REQ-496 Pairing Attempt Rate Limit]], and
stronger than [[Magic Wormhole]]'s shipped default (~16 bits). The **routing** component (`num`) is NOT a security parameter
and SHALL instead be sized so the probability of two concurrent serverless
pairings colliding on one rendezvous stays ≤ `[Provisional: 1%]` (a birthday
bound on expected concurrent pairings — `[Provisional: ~4–5 digits]`, refined by
DESIGN-047 `adr-rendezvous`).

**Trace:** [[#TEST-NFR-475]]; [[#ADR-473 Phrase-Derived DHT Rendezvous for SPAKE2]].

### NFR-476: Daemon Resource Footprint

`[Provisional — DESIGN-047 task perf-budget]` Idle daemon footprint (no active
edits, roster ≤ 8) SHALL be ≤ `[Provisional: 80 MB]` RSS AND ≤ `[Provisional:
1%]` CPU UNDER a reference-laptop baseline WITH median over a 10-minute idle
window.

**Trace:** [[#TEST-NFR-476]]; [[#OBS-470 Daemon health]].

### NFR-477: Remote Edit Propagation Latency

Remote edit visibility (a committed op on one peer applied and rendered on a
connected online peer) SHALL be ≤ `[Provisional: 400 ms]` (the
[[Doherty Threshold]] default of [[PROTO-001]] §UX Heuristics) UNDER nominal
LAN conditions WITH 95th percentile — the quantification of "realtime" that
[[#NFR-470 Local Edit-to-Render Latency]] (local echo only) does not carry (F36).

**Trace:** [[#TEST-NFR-477]]; [[#OBS-472 Sync convergence]].

---

## 6. Architecture Decision Records

> Structure: Context / Options / Decision / Consequences. Crypto/auth/network
> ADRs are Tier-1 review-gated.

### ADR-470: Loro as Canonical Store, Markdown+Git as Export

**`[Provisional — DESIGN-047 task adr-source-of-truth]`** · **Status:** Proposed

**Context:** Markdown is canonical today; offline-first merge
([[#REQ-474 Conflict-Free Offline Merge]]) needs persisted causal history, else
offline edits collide at the text layer on git merge, defeating the [[CRDT]].

**Options:** (A) Markdown canonical, [[Loro]] ephemeral (status quo, weak merge);
(B) [[Loro]] canonical, Markdown a committed export; (C) hybrid hot/at-rest.

**Decision:** (B). [[Loro]] snapshot+oplog under `.zetl/loro/` canonical; Markdown
a deterministic [[Materialization]] committed via the git/jj flush.

**Consequences:** (+) True offline merge; causal history survives; git stays
export/audit; the [[WAL]] is superseded — its durability contract moves to the
oplog's fsync-on-commit append ([[#REQ-472 Loro Canonical Store]], F39). (−) Largest departure from current
zetl; external edits need guarded import
([[#ADR-471 Guarded Import for External Markdown Edits]]); oplog growth (shallow
snapshots — [[#18. Open Questions]] Q4).

### ADR-471: Guarded Import for External Markdown Edits

**`[Provisional — DESIGN-047 task adr-external-edits]`** · **Status:** Proposed

**Context:** With [[Loro]] canonical, external writes are non-authoritative;
overwriting loses data, blind import corrupts causality.

**Options:** (A) ignore; (B) always import; (C) guarded import on concurrent-edit
detection.

**Decision:** (C) — fold iff no **unmaterialised daemon op** exists for the file
since `last_export` (a [[Loro]] *logical-time* predicate, not a wall-clock
window — [[#REQ-484 Guarded Import of External Markdown Edits]], F16), else stage.

**Consequences:** (+) Markdown stays a git/editor surface. (−) The
concurrent-edit predicate is security-relevant (covered by [[#TEST-484c]]).
(−) An external write in non-canonical form (e.g. NFD) is folded, then
re-materialised in canonical NFC — the on-disk bytes then differ from what the
editor wrote (spurious editor/git diffs); the import diff MUST compare under
the canonical form of [[#REQ-473 Deterministic Materialisation]] so the
fold→materialise→watch cycle terminates (F42).

### ADR-472: iroh + pkarr for Transport and Discovery

**`[Provisional — DESIGN-047 task adr-transport]`** · **Status:** Proposed

**Context:** [[P2P]] needs [[NAT Traversal]], an encrypted authenticated channel,
serverless rendezvous-by-key.

**Options:** [[iroh]] ([[QUIC]], hole-punching, pkarr, NodeId=ed25519);
[[libp2p]] (heavier); [[SPEC-004]] Goblins/OCapN sidecar (Guile/Tor).

**Decision:** [[iroh]] + [[pkarr]] on [[Mainline DHT]]; [[NodeId]] = ed25519 key
unifies identity, discovery, and [[SPAKE2]] handoff.

**Consequences:** (+) One keypair across three concerns; mature Rust QUIC;
supersedes [[SPEC-004]]. (−) Relies on iroh default relays unless self-run
([[#ADR-474 Relay as Optional Fallback Not Requirement]]); DHT privacy
([[#12. Threat Model]] §G/§H).

### ADR-473: Phrase-Derived DHT Rendezvous for SPAKE2

**`[Provisional — DESIGN-047 task adr-rendezvous]` · No-go area: human crypto
review required.** · **Status:** Proposed (strongest gate)

**Context:** Two un-acquainted daemons must find each other before key exchange;
a central mailbox reintroduces a server. A naïve `HKDF(whole phrase)` rendezvous
makes locator entropy equal to *phrase* entropy, so a DHT scanner can enumerate
live pairings (review F8). The fix is to stop conflating *routing* with *secret*.

**Options:** (A) central mailbox (server); (B) `HKDF(whole phrase)` rendezvous +
[[SPAKE2]] (rejected — couples routing to secret, the F8 hole); (C) **split the
phrase into a routing part and a secret part** — the [[Magic Wormhole]] code
structure (`num-word-word`): `num` derives the [[Rendezvous]] locator, `word-word`
is the [[SPAKE2]] password.

**Decision:** (C). The [[Rendezvous]] locator is `HKDF(num, "zetl/p2p-rdv/v1")`
— `num` is **public routing** (a meeting-room number, deliberately enumerable,
carries no secret). The [[SPAKE2]] password is `word-word` and **never reaches
the DHT** — it is exercised only inside the handshake over the discovered
[[QUIC]] channel. Short rendezvous TTL; the record carries only an endpoint hint.
This is the native idiom: the `spake2` crate already in `src/cap/pair.rs` is
[[Magic Wormhole]]'s, whose wormhole codes are exactly `num-word-word`.

**Consequences:** (+) Fully serverless; reuses [[SPEC-034]]
[[SPAKE2]]+[[HKDF]]. (+) **Enumerating the DHT reveals only routing, never the
secret** — F8's offline-enumeration hole closes because there is no secret in the
routing layer. The secret's resistance is [[SPAKE2]] online-single-guess over
`word-word`, bounded by single-use ([[#REQ-478 Single-Use Phrase]]) and the
[[#REQ-496 Pairing Attempt Rate Limit]] (which, unlike DHT lookups, *is*
enforceable at the daemon — F31). (−)
`num` must be sized for *collision-avoidance* between concurrent serverless
pairings (birthday bound), NOT for secrecy — see
[[#NFR-475 Pairing Secret Entropy Floor]]. (−) An attacker can squat or
overwrite a rendezvous record (DoS / MitM-positioning) — [[SPAKE2]] still denies
auth, but it is a residual ([[#12. Threat Model]] §J).

### ADR-474: Relay as Optional Fallback, Not Requirement

**Status:** Proposed. **Default:** direct [[iroh]] hole-punching; fall back to an
[[iroh]] [[Relay]] only when direct fails (symmetric NAT). The default serves the
dominant profile ([[users/solo-multi-device/user|home/broadband devices]], mostly
direct-connectable); operators MAY self-run a relay. Reachability failure MUST be
reported distinctly from auth failure.

### ADR-475: Supersede SPEC-004 (Goblins / OCapN Sidecar)

**Status:** Proposed. [[SPEC-004]]'s goal is met by this in-process Rust stack; on
approval, [[SPEC-004]] is marked `superseded`. (−) Loses OCapN object-capability
semantics; if needed, those move to the [[Group Key]]/roster layer.

### ADR-476: Encryption at Rest is Opt-In

**Status:** Proposed. **Default:** off. Transit is always encrypted
([[#REQ-482 Roster-Gated Encrypted Transport]]); [[Encryption at Rest]] of
`.zetl/loro/` addresses local disk theft — orthogonal to the P2P boundary — and
defaults off because the dominant profile stores vaults on already-encrypted OS
volumes; opt-in uses the [[SPEC-040-zetl-mobile]] OS-keychain. Recorded as an ADR
so the omission is a decision.

### ADR-477: Single Per-Vault Group Key (defer web-of-trust)

**`[Provisional — DESIGN-047 task adr-trust-model]`** · **Status:** Proposed

**Context:** Trust must be scoped.

**Options:** (A) one per-vault [[Group Key]]; (B) pairwise per-edge keys with
per-folder/role scoping ([[SPEC-036-spake2-onboarding]] style).

**Decision:** (A). Membership = roster; revocation = rotate + re-seal. (B)
deferred (heavier key management; YAGNI until demand).

**Consequences:** (−) Any member reads the whole vault; revocation is coarse;
**admission authority is unenforceable against a malicious member** — any
key-holder can seal the [[Group Key]] onward or fork the roster, so
Owner-controlled membership binds honest daemons only
([[#12. Threat Model]] §N, F34); the write/admission dual moves to the
web-of-trust successor along with (B).

### ADR-478: Merkle DAG as Convergence Witness and Reconciliation Index

**`[Provisional — DESIGN-047 task adr-merkle-sync]`** · **Status:** Proposed

**Context:** [[Loro]] is op/causal and authoritative for *merge*, but gives no
cheap "are we in sync?" check and no independent integrity witness. zetl already
computes a BLAKE3 [[Merkle DAG]] over the materialised AST ([[SPEC-006]]:
block-typed leaves → file roots → `vault_root`), already content-addresses
blocks, and already stamps `vault_root` onto jj snapshots — at the
**materialisation boundary**, the same boundary at which
[[#REQ-473 Deterministic Materialisation]] guarantees a deterministic AST.

**Options:** (A) ignore the Merkle layer; rely solely on [[Version Vector]]
exchange. (B) reuse the Merkle DAG as a convergence witness + anti-entropy
reconciliation index *layered on top of* [[Loro]]. (C) replace [[Loro]]'s sync
with Merkle reconciliation (rejected — Merkle cannot merge; no conflict
resolution).

**Decision:** (B). Two peers compare `vault_root` first
([[#REQ-486 Merkle Anti-Entropy Reconciliation]]); equal roots **and equal
[[Version Vector]]s** → converged, no op exchange (equal roots alone may lag
committed unmaterialised ops — F33); mismatch →
descend the DAG to localise differing docs, then ship [[Loro]] op deltas for only
those. Converged peers MUST match roots
([[#REQ-485 Merkle Convergence Witness]]); a mismatch under a [[Loro]]-reported convergence is an integrity
alarm. A peer MAY sign its `vault_root` with its [[NodeId]] and carry it in the
[[pkarr]] record or a direct peer-to-peer announcement as an authenticated state hint
([[#8.8 Signed Vault Root]]).

**Consequences:** (+) Near-free integrity cross-check on the merge/crypto core;
cheap convergence heartbeat (one hash); coarse "what changed" filter that pairs
with [[Loro]]'s fine "which ops"; authenticated, tamper-evident state history that
dovetails with existing jj `vault_root` snapshots. (−) Merkle is a
*verification + reconciliation* layer only — it never merges (Loro does); it is
computed at quiescence, not per keystroke; its witness property is contingent on
deterministic materialisation + AST chunking. (−) Full content-addressed
block *transfer* (fetch only missing leaves) is the natural extension but is
**deferred** ([[#18. Open Questions]] Q8) to keep v1 scoped.

### ADR-479: CBCL as the Control-Plane Message Language

**`[Provisional — DESIGN-047 task adr-control-proto]` · No-go-adjacent: the R4
signature path is auth-core, human-reviewed.** · **Status:** Proposed

**Context:** The control plane (daemon verbs, pairing/reconcile choreography,
presence, signed-root announcement) needs a recognised-before-acted wire format
([[PROTO-001]] Principle 14). The 0.3.0 draft hand-rolled per-message CBOR +
schema validators. `../cbcl-rs` ([[CBCL]]) is a Rust, `no_std`, Lean-verified
(0 sorries) agent-communication language restricting messages to [[DCFL]] so
validity stays decidable even under runtime dialect extension (zetl ships a
fixed dialect set and does not use that runtime-extension capability), with five
machine-checked invariants: R1 no-recursion, R2 resource-bounds, R3
core-preservation, **R4 Ed25519 integrity over a canonical encoding**, **R5
causal-protocol + shape contracts**.

**Options:** (A) bespoke CBOR + schema per message (status quo; many small
hand-rolled recognisers = parser-differential surface); (B) [[CBCL]] dialects for
the **control plane**, native length-prefixed binary for the **data plane**
([[Loro]]/[[SPAKE2]] blobs); (C) [[CBCL]] for *everything* including blobs
(rejected — DCFL S-expression text is wrong for bulk binary payloads).

**Decision:** (B). Define `zetl-pair` and `zetl-sync` [[CBCL]] dialects over the 8
core performatives (`tell ask reply hello bye ok error cancel`, immutable by R3).
One DPDA recognises all control messages (parser-equivalence → no
parser-differential attacks). R4 carries message/dialect authentication on
ed25519 ([[NodeId]] is already ed25519 — unifies with
[[#8.8 Signed Vault Root]]). R5 encodes the pairing and reconcile choreographies
as verified causal-protocol contracts ([[#REQ-488 Choreographies as Verified R5 Causal-Protocol Contracts]]),
verified at build time over the shipped dialect (not at peer-driven install).
[[Loro]] updates and [[SPAKE2]] bytes stay opaque on the data plane, referenced by
id from control messages. The `zetl-pair`/`zetl-sync` dialects are **shipped
with the CLI binary** (a fixed, release-versioned set) — NOT installed from peers
at runtime; protocol evolution happens by releasing a new `zetl`, not by DHT
gossip (deferred — [[#18. Open Questions]] Q10). zetl therefore uses [[CBCL]]'s
recognition + R4 + R5 properties but not its runtime self-extension.

**Consequences:** (+) Principle 14 satisfied with a *formal* warrant
(Lean `dcfl_preserved`/`decidable_preserved`), not a hand-rolled validator;
parser-differential attacks excluded by construction (strengthens
[[#12. Threat Model]] §F); R4 supplies message auth; R5 supplies a verified
handshake state machine; shipping a fixed dialect set with the binary keeps the
recognised language static per release (no runtime-extension attack surface — F17
dissolved); `no_std` suits the mobile daemon; reuses an existing dependency
(Simplicity-Ladder rung 4) and *deletes* the bespoke CBOR validators.
(−) [[DCFL]] exceeds the regular power minimal framing needs — recorded here as
the Principle 14 §6 justification (decidable + parser-equivalent, far below the
undecidable danger zone). (−) [[CBCL]]'s canonical encoding is S-expression text,
so control messages are textual (acceptable at control-plane volume; bulk binary
stays on the data plane — see [[#18. Open Questions]] Q9). (−) Per-message R4
signing on the presence path costs one ed25519 sign + verify per frame (~50 µs
each on commodity hardware — well inside
[[#NFR-477 Remote Edit Propagation Latency]]'s budget; recorded so the cost is
a decision, not an accident — F37). (−) Adds a Tier-1
dependency whose R4 path is auth-core and must be in the human-review package.

### ADR-480: CLI Surface Follows Existing zetl Conventions

**Status:** Proposed

**Context:** The 0.4.0 draft invented bare top-level verbs (`zetl pair`,
`zetl join`, `zetl peers`, `zetl revoke`) and a separate `zetld` *command*. That
contradicts the established CLI: domain verbs are grouped under a noun with nested
`clap` `Subcommand` enums (`zetl cap <verb>`, `zetl collab <verb> <subverb>`,
`zetl skill <verb>`), each carrying a `///` doc comment and an `after_help`
example block; secrets are read TTY-only (`zetl collab passwd add` — "never from
argv or env", SPEC-041 REQ-4108); output honours the global `--format`/`--json`
and `--vault`; revoke-by-id is a positional (`zetl collab share revoke <jti>`);
and not-yet-implemented verbs exit non-zero with a `not-yet-implemented`
diagnostic (`zetl cap`). [[SPEC-036-spake2-onboarding]] already named the redeem
verb `zetl collab join`.

**Options:** (A) bare top-level `zetl pair`/`zetl join`/… (status quo of the
draft; inconsistent, and `zetl pair` collides conceptually with the existing
`zetl cap pair`); (B) place pairing/roster verbs under the existing `zetl collab`
group and daemon management under a new `zetl daemon` group paralleling
`zetl serve`; (C) a brand-new top-level group (e.g. `zetl p2p`) — rejected as
gratuitous when `collab` already owns collaboration (Principle 15: options belong
on the artefact that already has the right responsibility).

**Decision:** (B). `zetl collab {pair,join,peers,revoke}` (extending the existing
group, reusing SPEC-036's `join`); `zetl daemon {start,stop,status}` manages the
`zetld` process (as `dockerd`/`docker`). All inherit the global flags, the
TTY-only secret convention for the [[Pairing Phrase]], positional ids, and the
`not-yet-implemented` non-zero exit for unlanded verbs.

**Consequences:** (+) Zero new CLI idioms; discoverable via the same `--help`
shape; the phrase reuses the audited TTY-only path; consistent machine output via
the global formatter. (−) `zetl collab` now spans server-auth *and* P2P verbs —
acceptable, both are "collaboration"; the encrustation guard
([[PROTO-001]] §Specification Status Lifecycle) is satisfied because pairing is
on `collab`'s existing responsibility, not a broadening of it.

---

## 7. Contracts

> [[PROTO-001]] §Contract Specification. External-input contracts carry a
> **Grammar / Recogniser** field per Principle 14; pre/post-conditions are
> per-clause for requirement-localised repair.

### CON-470: Daemon Control Channel

**Interface:** Local IPC (`[Provisional: $XDG_RUNTIME_DIR/zetld.sock`; named pipe
on Windows]`) — versioned request/response + subscription. Control verbs:
`attach`, `status`, `apply_ops`, `subscribe`, `pair`, `join`, `peers`, `revoke`.
**CLI mapping** ([[#ADR-480 CLI Surface Follows Existing zetl Conventions]]):
`zetl daemon {start,stop,status}` manages the `zetld` process (as `zetl serve`
runs the web server); `zetl collab {pair,join,peers,revoke}` front the
corresponding control verbs.

**Grammar / Recogniser:** [[CBCL]] control messages (`zetl-pair`/`zetl-sync`
dialects); grammar [[#8.1 Control Envelope]]; recogniser = the shared [[CBCL]]
DPDA ([[DCFL]], parser-equivalence) + R1–R5 validation — no per-message ad-hoc
parser. Trust boundary: local process.

**Pre-conditions:** C1 (REQ-470) caller is local, socket `0600`/per-user pipe;
C2 (REQ-483) each request fully recognised before dispatch.

**Post-conditions:** C3 (REQ-470) every mutation flows through this channel; no
client writes `.zetl/loro/` directly. C4 (REQ-471) `status` returns a
machine-readable enum; repeated control verbs are idempotent.

**Error model:** typed `vault-not-found`, `not-paired`, `reachability-failed`,
`malformed-request` (parse failure); single opaque `auth-failed` for all pairing
failure causes (REQ-479).

**Implements:** [[#REQ-470 Persistent Daemon Ownership]], [[#REQ-471 Idempotent Control Lifecycle]], [[#REQ-483 Full Recognition at Trust Boundaries]], [[#REQ-487 Control-Plane Messages Recognised by the CBCL DPDA]], [[#REQ-490 Daemon Survives Client Disconnection]].
**Verified by:** [[#TEST-470a]], [[#TEST-471a]], [[#TEST-471b]], [[#TEST-471c]], [[#TEST-483b]], [[#TEST-487a]], [[#TEST-490a]], [[#TEST-490c]].

### CON-471: Loro Store and Materialisation

**Interface:** `materialise(loro_doc) -> Markdown` (pure, deterministic);
`import_external(markdown, last_export, edit_state) -> ImportOutcome` (guarded).

**Grammar / Recogniser:** `import_external` input is UTF-8 Markdown; grammar
[[#8.4 Materialised Markdown]]; recogniser = the existing zetl Markdown parser;
non-UTF-8/binary fails closed → staged.

**Pre-conditions:** C1 (REQ-472/473) `materialise` total over valid [[Loro]] docs,
no I/O; C2 (REQ-484) `import_external` receives live edit-state.

**Post-conditions:** C3 (REQ-473) `materialise` referentially transparent →
identical bytes; C4 (REQ-472) restart reloads canonical state with causal
history; C5 (REQ-484) `import_external` returns `Folded | Staged(path)`, discards
neither side.

**Error model:** `materialise` infallible on valid docs; `import_external`
surfaces `Staged(path)` instead of erroring.

**Implements:** [[#REQ-472 Loro Canonical Store]], [[#REQ-473 Deterministic Materialisation]], [[#REQ-484 Guarded Import of External Markdown Edits]].
**Verified by:** [[#TEST-472a]], [[#TEST-472c]], [[#TEST-472d]], [[#TEST-473a]], [[#TEST-473c]], [[#TEST-484a]], [[#TEST-484b]], [[#TEST-484c]], [[#TEST-484d]].

### CON-473: Peer Session (iroh)

**Interface:** [[QUIC]] connection keyed by [[NodeId]]; streams `reconcile`
([[Merkle Vault Root]] compare → DAG descent), `sync` ([[Loro]] update exchange
via [[Version Vector]] for the docs `reconcile` localised), and `presence`
([[Ephemeral Store]]).

**Grammar / Recogniser:** **control plane** — `reconcile`/`presence`/signed-root
are [[CBCL]] `zetl-sync` messages recognised by the shared DPDA
([[#8.3 Presence Frame]], [[#8.8 Signed Vault Root]]); **data plane** — `sync`
carries opaque length-prefixed [[Loro]] updates ([[#8.2 Peer Sync Frame]])
referenced by id from a [[CBCL]] message and recognised by the [[Loro]] import
decoder (fuzzed). **The primary untrusted trust boundary.**

**Pre-conditions:** C1 (REQ-475) both endpoints discovered via [[pkarr]];
C2 (REQ-482) peer [[NodeId]] ∈ roster, verified before any frame; C3 (REQ-483)
each frame fully recognised before apply.

**Post-conditions:** C4 (REQ-482) content frames exchanged only after the roster
check; C5 (REQ-486) `reconcile` runs before `sync`; op exchange is skipped only
on equal roots **and** equal [[Version Vector]]s; C6 (REQ-474) `sync` converges
to identical [[Loro]] state; C7 (REQ-485)
converged peers report equal [[Merkle Vault Root]] or raise an integrity alarm.

**Error model:** `not-on-roster` (reject pre-frame); `malformed-frame` (drop +
log); `root-mismatch-after-converge` (integrity alarm); `reachability-failed`
(distinct from auth).

**Implements:** [[#REQ-474 Conflict-Free Offline Merge]], [[#REQ-475 Serverless Peer Discovery]], [[#REQ-482 Roster-Gated Encrypted Transport]], [[#REQ-483 Full Recognition at Trust Boundaries]], [[#REQ-485 Merkle Convergence Witness]], [[#REQ-486 Merkle Anti-Entropy Reconciliation]], [[#REQ-487 Control-Plane Messages Recognised by the CBCL DPDA]], [[#REQ-488 Choreographies as Verified R5 Causal-Protocol Contracts]], [[#REQ-492 Roster Gate Before Vault Frame]], [[#REQ-493 Signed-Root Epoch Binding]], [[#REQ-494 Control-to-Data Binding]], [[#REQ-495 Signed-Root Freshness]].
**Verified by:** [[#TEST-474a]], [[#TEST-474c]], [[#TEST-475a]], [[#TEST-475b]], [[#TEST-482a]], [[#TEST-483a]], [[#TEST-483c]], [[#TEST-485a]], [[#TEST-485c]], [[#TEST-486a]], [[#TEST-486b]], [[#TEST-486c]], [[#TEST-492a]], [[#TEST-492b]], [[#TEST-493a]], [[#TEST-493b]], [[#TEST-494a]], [[#TEST-494b]], [[#TEST-495a]], [[#TEST-495b]].

### CON-474: Pairing Protocol (`zetl collab pair` / `zetl collab join`)

**Interface:** `zetl collab pair`: mint phrase, publish rendezvous [[pkarr]]
record, await peer, run [[SPAKE2]], seal [[Group Key]]. `zetl collab join`:
TTY-prompt phrase, resolve rendezvous, connect, run [[SPAKE2]], receive sealed
key. Both honour the global `--format`/`--json` and `--vault` flags and the
standard non-zero-exit-on-error convention ([[#ADR-480 CLI Surface Follows Existing zetl Conventions]]).

**Grammar / Recogniser:** phrase grammar [[#8.5 Pairing Phrase]] (ABNF, regular,
TTY-only); handshake choreography is the [[CBCL]] `zetl-pair` dialect (DPDA;
the step order is an R5 causal-protocol contract — [[#REQ-488 Choreographies as Verified R5 Causal-Protocol Contracts]]);
[[SPAKE2]] bytes ride as an opaque payload [[#8.6 SPAKE2 Frame]] recognised by the
`cap::pair` decoder (fuzzed).

**Pre-conditions:** C1 (REQ-476) `pair` caller owns the vault; C2 (REQ-477) phrase
entered only via TTY; C3 (REQ-483) every [[SPAKE2]]/rendezvous frame recognised
before use.

**Post-conditions:** C4 (REQ-476) on success both rosters updated, key shared,
rendezvous torn down; C5 (REQ-478) phrase consumed; C6 (REQ-479) all failures
return one opaque `auth-failed`; C7 (REQ-480) membership granted only on
[[SPAKE2]] success; C8 (REQ-496) the inbound attempt budget is enforced —
exhaustion aborts the pairing and tears down the rendezvous record.

**Error model:** single opaque `auth-failed`; distinct `reachability-failed`,
`malformed-input`.

**Implements:** [[#REQ-476 DHT-Bootstrapped SPAKE2 Pairing]], [[#REQ-477 Phrase OOB-Only Non-Leak]], [[#REQ-478 Single-Use Phrase]], [[#REQ-479 Failure-Message Indistinguishability]], [[#REQ-480 Group-Key Admission Gate]], [[#REQ-488 Choreographies as Verified R5 Causal-Protocol Contracts]], [[#REQ-491 SPAKE2 Channel Authentication]], [[#REQ-496 Pairing Attempt Rate Limit]].
**Verified by:** [[#TEST-476a]], [[#TEST-476b]], [[#TEST-476c]], [[#TEST-477a]], [[#TEST-477b]], [[#TEST-477c]], [[#TEST-478a]], [[#TEST-478b]], [[#TEST-479a]], [[#TEST-479c]], [[#TEST-480a]], [[#TEST-480b]], [[#TEST-491a]], [[#TEST-496a]], [[#TEST-496b]].

### CON-477: Group Key Roster and Revocation

**Interface:** roster store (`[Provisional: .zetl/peers.toml` mode `0600`]`)
mapping [[NodeId]] → {role, added_at, key_epoch}; `zetl collab revoke <nodeid>`
rotates epoch + schedules re-seal (positional id, mirroring
`zetl collab share revoke <jti>`); `zetl collab peers` lists the roster and
never prints key material (mirroring `zetl collab share list`).

**Grammar / Recogniser:** roster file is a declared [[TOML]] schema
([[#8.7 Roster Schema]]); recogniser = a `serde` TOML decoder + schema validation
(it is trusted local state, but recognised before use per Principle 14).

**Pre-conditions:** C1 (REQ-481) caller is the [[users/vault-owner/user|Owner]].

**Post-conditions:** C2 (REQ-481) revoked [[NodeId]] removed; new epoch sealed to
survivors within [[#NFR-474 Revocation Propagation]]; C3 (REQ-481) revoked peer
rejected by [[#CON-473 Peer Session]]; C4 (REQ-480) entries added only via
completed [[SPAKE2]].

**Error model:** `not-on-roster`; `last-member` (cannot revoke the sole member).

**Implements:** [[#REQ-480 Group-Key Admission Gate]], [[#REQ-481 Revocation by Key Rotation]].
**Verified by:** [[#TEST-480a]], [[#TEST-480b]], [[#TEST-481a]], [[#TEST-481b]], [[#TEST-481c]].

---

## 8. Input Grammars (LangSec)

> [[PROTO-001]] Principle 14. Each external/boundary input has a declared grammar
> at the lowest sufficient grammatical power, recognised in full before any
> semantic action ([[#REQ-483 Full Recognition at Trust Boundaries]]). All
> structured wire formats are length-prefixed binary or a typed schema — never
> string concatenation. `[Provisional — DESIGN-047 task input-grammars]` for
> exact octet layouts.

**Control plane vs data plane.** All *control* messages
([[#8.1 Control Envelope]], [[#8.3 Presence Frame]], [[#8.8 Signed Vault Root]]
and the pairing/reconcile choreography) are [[CBCL]] messages in the
`zetl-pair`/`zetl-sync` dialects, recognised by one shared DPDA ([[DCFL]],
parser-equivalence → no parser-differential surface;
[[#ADR-479 CBCL as the Control-Plane Message Language]]). *Data-plane* blobs
([[#8.2 Peer Sync Frame]] [[Loro]] update, [[#8.6 SPAKE2 Frame]] bytes) are
opaque, length-prefixed, and bound to their control message by a **content
hash** (not a bare id — F12, [[#REQ-494 Control-to-Data Binding]]) before their
own decoders interpret them (defense in depth). Each grammar below is a
subsection so its anchor resolves under `zetl check --dead-links`.

### 8.1 Control Envelope
[[CBCL]] `(verb …)` in a `zetl-*` dialect. A message that references a data-plane
payload MUST carry the reference as a **recognised** `(ref <payload-id>
<content-hash>)` clause of the dialect grammar — so the control→data binding
([[#REQ-494 Control-to-Data Binding]], F12) is part of full DPDA recognition, not
a post-parse check. Power: [[DCFL]]. Recogniser: shared [[CBCL]] DPDA + R1–R5.
Boundary: local process.

### 8.2 Peer Sync Frame
`len ‖ loro_update_bytes` (data plane). Power: regular framing; payload =
[[Loro]] format. Recogniser: [[Loro]] crate import (fuzzed,
[[#TEST-fuzz-loro]]). Boundary: **network, untrusted**.

### 8.3 Presence Frame
[[CBCL]] `(tell … cursor …)`. Power: [[DCFL]]. Recogniser: shared [[CBCL]] DPDA.
Boundary: network, untrusted.

### 8.4 Materialised Markdown
UTF-8, a **declared restricted CommonMark profile** + `[[wikilinks]]` (the
profile is the grammar — CommonMark at large is defined operationally, not by a
clean CF grammar, so the accepted subset is named explicitly; F13). Power:
context-free (the restricted profile). Recogniser: the shared zetl Markdown
parser, treated as a trust-boundary recogniser and **fuzzed**
([[#TEST-fuzz-markdown]]). Boundary: local file / `git pull`.

### 8.5 Pairing Phrase
`number "-" word "-" word`. Power: regular (ABNF below). Recogniser: generated
validator. Boundary: TTY only.

```abnf
phrase = number "-" word "-" word
number = 4*5DIGIT            ; ROUTING ONLY — derives the rendezvous locator;
                            ; public, enumerable, no secret. Sized for
                            ; collision-avoidance, not secrecy (NFR-475).
word   = 3*8ALPHA            ; SECRET — the two words are the SPAKE2 password
                            ; (BIP39 English wordlist, ~22 bits); never on the DHT.
```

### 8.6 SPAKE2 Frame
`len ‖ side ‖ spake_msg` (data plane). Power: regular framing. Recogniser:
`cap::pair` decoder (fuzzed, [[#TEST-fuzz-spake]]). Boundary: network, untrusted.

### 8.7 Roster Schema
[[TOML]] table-of-peers. Power: context-free (TOML). Recogniser: `serde` TOML +
schema check. Boundary: local file.

### 8.8 Signed Vault Root
[[CBCL]] `zetl-sync` message, R4-signed `{nodeid, key_epoch, root_seq,
vault_root}` where `root_seq` is a per-signer monotonic counter (freshness — F29).
Power: [[DCFL]]. Recogniser: shared [[CBCL]] DPDA + R4 ed25519 verify vs roster
[[NodeId]], **rejecting any `key_epoch` ≠ the verifier's current epoch** (F9,
[[#REQ-493 Signed-Root Epoch Binding]]) **and any `root_seq` ≤ the last accepted
from that signer** (defeats same-epoch replay — F29, threat §K). Boundary:
network, untrusted.

### 8.9 pkarr Rendezvous Record
The DHT record resolved during discovery (HP1) and reconnect. Payload: an
endpoint hint (and, for durable peers, optionally an [[#8.8 Signed Vault Root]]).
Power: regular framing + schema. Recogniser: generated decoder; **a resolved
record is an unauthenticated *hint* only — it confers no trust until [[SPAKE2]]
(pairing) or roster-NodeId verification (reconnect) succeeds** (F14). Boundary:
**network, untrusted** (anyone can publish to the [[Mainline DHT]], incl. at a
phrase-derived rendezvous pubkey — threat §J).

No grammar in this spec exceeds context-free power; the control plane sits at
[[DCFL]] (a decidable subset of CF), justified in
[[#ADR-479 CBCL as the Control-Plane Message Language]] per Principle 14 §6.
Producers and consumers of each format share one recogniser — for the control
plane, the single [[CBCL]] DPDA — so there is no parser-differential surface by
construction.

---

## 9. Capability Placement & Simplicity Ladder

> [[PROTO-001]] Phase-2 task + Principle 15 (Right Place) + the Simplicity
> Ladder. For each new capability: the rung it settled at, and why it lives where
> it does. Composition-first — reused before built.

| Capability | Ladder rung | Placement (Principle 15) |
| ---------- | ----------- | ------------------------ |
| [[CRDT]] engine | 4 — existing dependency ([[Loro]]) | shared `crdt::` layer; used by all editors. Not hand-rolled (was [[diamond-types]]). |
| [[P2P]] transport + [[NAT Traversal]] | 4 — existing dependency ([[iroh]]) | shared `p2p::iroh`; one QUIC stack for all peers. |
| Discovery | 4 — existing dependency ([[pkarr]]) | shared `p2p::pkarr`; reused for rendezvous *and* durable records. |
| [[SPAKE2]] + [[HKDF]] | 4 — reuse `src/cap/pair.rs` ([[SPEC-034]]) | pure `p2p::pair`; new domain tag only, no new crypto primitive (Principle: no new constants). |
| Rendezvous derivation | 5 — minimum new code over [[HKDF]] | pure `p2p::pair::rendezvous`; the *only* genuinely new crypto glue → Tier-1 review. |
| Daemon control plane | 5 — minimum new code | `daemon::zetld`; the one new long-lived component, justified by REQ-470. |
| Markdown materialise/import | 5 — minimum new code over existing parser/flush | pure `crdt::loro::{materialise,import}` + reuse of the existing git/jj flush pipeline. |
| Merkle reconciliation + witness | 4 — existing `src/merkle.rs` ([[SPEC-006]]) | reuse the vault Merkle DAG at the materialisation boundary; layered on [[Loro]], not a new engine ([[#ADR-478 Merkle DAG as Convergence Witness and Reconciliation Index]]). |
| Control-plane message language | 4 — existing dependency `../cbcl-rs` ([[CBCL]]) | shared `p2p::proto` over `cbcl-core`/`cbcl-parser`; one DPDA replaces the bespoke per-message CBOR validators (deletion over addition) ([[#ADR-479 CBCL as the Control-Plane Message Language]]). |
| CLI verbs (pair/join/peers/revoke, daemon) | 6 — new verbs, but *on existing groups* | add to the existing `zetl collab` `clap` group + a `zetl daemon` group paralleling `zetl serve`; no new CLI idiom (Principle 15: options on the artefact with the right responsibility) ([[#ADR-480 CLI Surface Follows Existing zetl Conventions]]). |
| Wire framing (data plane) | 2/4 — std length-prefix + [[Loro]] codec | no bespoke serialisation (Principle 14 bans string-concat formats). |

No new abstraction is introduced with a single implementation
([[PROTO-001]] Discipline Rules); the [[CrdtBackend]] trait of the current code
is *removed* (one engine, [[Loro]]) rather than retained speculatively.

---

## 10. Test Specifications

### 10.1 Verification Strategy Selection

Per [[PROTO-001]] §Selecting a Verification Strategy — Tier-1, AI-synthesised,
security-critical ⇒ the union below.

| Characteristic (this spec) | Techniques |
| -------------------------- | ---------- |
| Pure core (rendezvous/[[HKDF]], materialise, import) | property + mutation |
| State machine (pairing handshake, key epochs) | property (transitions) + example |
| Parsers/protocol ([[SPAKE2]], [[Loro]] frames, control/presence) | **fuzzing** + property roundtrip (LangSec) |
| Security-critical (Tier 1) | all the above + contract assertions + **adversarial** |
| AI-synthesised spec | requirement-targeted decomposition + mutation + adversarial (mandatory) |

### 10.2 Requirement-Targeted Decomposition

Each REQ derives ≥ 1 test per applicable type (positive / negative-input /
negative-output); each TEST records `Validates:`.

| ID | Type | Technique | Target → assertion | Validates |
| -- | ---- | --------- | ------------------ | --------- |
| **TEST-470a** | positive | example+contract | start→attach→detach; daemon survives client exit | [[#REQ-470 Persistent Daemon Ownership]] |
| **TEST-490c** | neg-output | contract | client exit MUST NOT stop daemon or lose committed state | [[#REQ-490 Daemon Survives Client Disconnection]] |
| **TEST-471a** | positive | example | repeated `start`/`status` → same state | [[#REQ-471 Idempotent Control Lifecycle]] |
| **TEST-471b** | neg-input | example | `attach(missing-vault)` → `vault-not-found` | [[#REQ-471 Idempotent Control Lifecycle]] |
| **TEST-471c** | neg-output | contract | `status` MUST NOT return stale/non-enum state | [[#REQ-471 Idempotent Control Lifecycle]] |
| **TEST-472a** | positive | example | edit → restart → canonical state + history intact | [[#REQ-472 Loro Canonical Store]] |
| **TEST-472c** | neg-output | property | restart MUST NOT drop committed ops | [[#REQ-472 Loro Canonical Store]] |
| **TEST-472d** | neg-output | example | `SIGKILL` mid-edit MUST NOT lose fsync-committed ops | [[#REQ-472 Loro Canonical Store]] |
| **TEST-473a** | positive | property | identical state → byte-identical Markdown (2 runs/2 devices) | [[#REQ-473 Deterministic Materialisation]] |
| **TEST-473c** | neg-output | property | reject differing Merkle hash for identical state | [[#REQ-473 Deterministic Materialisation]] |
| **TEST-474a** | positive | property(convergence) | two op-sets → identical state, no marker | [[#REQ-474 Conflict-Free Offline Merge]] |
| **TEST-474c** | neg-output | property | reject dropped op or emitted conflict marker | [[#REQ-474 Conflict-Free Offline Merge]] |
| **TEST-475a** | positive | integration | known [[NodeId]] resolves after IP change | [[#REQ-475 Serverless Peer Discovery]] |
| **TEST-475b** | neg-input | integration | never-published [[NodeId]] → clean failure, no hang | [[#REQ-475 Serverless Peer Discovery]] |
| **TEST-476a** | positive | example(e2e) | HP1 phrase→rendezvous→[[SPAKE2]]→key→sync | [[#REQ-476 DHT-Bootstrapped SPAKE2 Pairing]] |
| **TEST-476b** | neg-input | example | wrong `word-word` → no key agreement → `auth-failed` | [[#REQ-491 SPAKE2 Channel Authentication]] |
| **TEST-476c** | neg-output | example | failed [[SPAKE2]] yields no key / no roster entry | [[#REQ-491 SPAKE2 Channel Authentication]] |
| **TEST-477a** | positive | property+capture | phrase absent from URL/argv/env/wire | [[#REQ-477 Phrase OOB-Only Non-Leak]] |
| **TEST-477b** | neg-input | example | phrase via argv/env refused (TTY-only) | [[#REQ-477 Phrase OOB-Only Non-Leak]] |
| **TEST-477c** | neg-output | property | no frame/DHT record contains a phrase-deriving value | [[#REQ-477 Phrase OOB-Only Non-Leak]] |
| **TEST-478a** | positive | example | first redemption succeeds | [[#REQ-478 Single-Use Phrase]] |
| **TEST-478b** | neg-input | example | second redemption rejected (generic) | [[#REQ-478 Single-Use Phrase]] |
| **TEST-479a** | positive | example | all 5 causes → byte-identical user text | [[#REQ-479 Failure-Message Indistinguishability]] |
| **TEST-479c** | neg-output | example | reject build where text differs by cause | [[#REQ-479 Failure-Message Indistinguishability]] |
| **TEST-480a** | positive | example | membership granted only after [[SPAKE2]] success | [[#REQ-480 Group-Key Admission Gate]] |
| **TEST-480b** | neg-input | example | admission attempt without pairing rejected | [[#REQ-480 Group-Key Admission Gate]] |
| **TEST-481a** | positive | example | `revoke` rotates epoch + re-seals survivors | [[#REQ-481 Revocation by Key Rotation]] |
| **TEST-481b** | neg-input | example | `revoke(non-member)`→`not-on-roster`; `revoke(last)`→`last-member` | [[#REQ-481 Revocation by Key Rotation]] |
| **TEST-481c** | neg-output | example | revoked [[NodeId]] MUST NOT decrypt post-rotation frame | [[#REQ-481 Revocation by Key Rotation]] |
| **TEST-482a** | positive | integration | roster peer completes mutual-auth [[QUIC]] + sync | [[#REQ-482 Roster-Gated Encrypted Transport]] |
| **TEST-482b** | neg-output | integration | presence/sync frame MUST NOT be emitted on an unencrypted path (off-roster pre-frame rejection is [[#TEST-492b]]) | [[#REQ-482 Roster-Gated Encrypted Transport]] |
| **TEST-483a** | positive | property(roundtrip) | `parse(serialise(x))==x` for each §8 format | [[#REQ-483 Full Recognition at Trust Boundaries]] |
| **TEST-483b** | neg-input | fuzz+example | malformed control/frame rejected, no action | [[#REQ-483 Full Recognition at Trust Boundaries]] |
| **TEST-483c** | neg-output | property | no semantic action before full recognition | [[#REQ-483 Full Recognition at Trust Boundaries]] |
| **TEST-484a** | positive | property | external write, no concurrent edit → folded | [[#REQ-484 Guarded Import of External Markdown Edits]] |
| **TEST-484b** | neg-input | property | binary/non-UTF-8 write → staged, never folded | [[#REQ-484 Guarded Import of External Markdown Edits]] |
| **TEST-484c** | neg-output | property | concurrent external write MUST NOT overwrite [[Loro]] | [[#REQ-484 Guarded Import of External Markdown Edits]] |
| **TEST-484d** | positive | property | external delete: no concurrent op → tombstoned; concurrent → staged | [[#REQ-484 Guarded Import of External Markdown Edits]] |
| **TEST-485a** | positive | property | converged peers → equal [[Merkle Vault Root]] | [[#REQ-485 Merkle Convergence Witness]] |
| **TEST-485c** | neg-output | property | root mismatch under reported convergence → alarm, not silent | [[#REQ-485 Merkle Convergence Witness]] |
| **TEST-486a** | positive | example | equal roots → session completes with zero op exchange | [[#REQ-486 Merkle Anti-Entropy Reconciliation]] |
| **TEST-486b** | neg-input | example | mismatch → DAG descent localises only differing docs | [[#REQ-486 Merkle Anti-Entropy Reconciliation]] |
| **TEST-486c** | neg-output | property | equal roots + unequal [[Version Vector]]s MUST NOT skip op exchange | [[#REQ-486 Merkle Anti-Entropy Reconciliation]] |
| **TEST-487a** | positive | example | valid [[CBCL]] control message accepted by the DPDA + R4 | [[#REQ-487 Control-Plane Messages Recognised by the CBCL DPDA]] |
| **TEST-487b** | neg-input | fuzz+example | non-conformant / R4-`Invalid` message rejected, no action | [[#REQ-487 Control-Plane Messages Recognised by the CBCL DPDA]] |
| **TEST-488a** | positive | example | in-order handshake satisfies the R5 causal-protocol contract | [[#REQ-488 Choreographies as Verified R5 Causal-Protocol Contracts]] |
| **TEST-488b** | neg-input | property | out-of-order / undefined-step message rejected by the R5 contract | [[#REQ-488 Choreographies as Verified R5 Causal-Protocol Contracts]] |
| **TEST-489a** | positive | example | `zetl collab pair --json` emits valid JSON; verbs live under `collab`/`daemon` | [[#REQ-489 P2P CLI Follows Existing zetl Conventions]] |
| **TEST-489b** | neg-input | example | error path exits non-zero; unlanded verb → `not-yet-implemented` | [[#REQ-489 P2P CLI Follows Existing zetl Conventions]] |
| **TEST-490a** | positive | example | daemon survives client exit; committed state intact on reattach | [[#REQ-490 Daemon Survives Client Disconnection]] |
| **TEST-491a** | positive | example(e2e) | matching `word-word` → SPAKE2 success → shared key | [[#REQ-491 SPAKE2 Channel Authentication]] |
| **TEST-492a** | positive | integration | roster [[NodeId]] admitted, vault frames exchanged | [[#REQ-492 Roster Gate Before Vault Frame]] |
| **TEST-492b** | neg-input | integration | off-roster [[NodeId]] rejected before any frame parsed | [[#REQ-492 Roster Gate Before Vault Frame]] |
| **TEST-493a** | positive | example | signed root at current epoch from roster signer accepted | [[#REQ-493 Signed-Root Epoch Binding]] |
| **TEST-493b** | neg-input | example | stale-epoch root from since-revoked signer rejected (no witness) | [[#REQ-493 Signed-Root Epoch Binding]] |
| **TEST-494a** | positive | property | control message + matching-hash payload → interpreted | [[#REQ-494 Control-to-Data Binding]] |
| **TEST-494b** | neg-input | property | substituted/replayed payload (hash mismatch) → rejected | [[#REQ-494 Control-to-Data Binding]] |
| **TEST-495a** | positive | example | signed root with `root_seq` > last accepted → accepted | [[#REQ-495 Signed-Root Freshness]] |
| **TEST-495b** | neg-input | example | replayed root with `root_seq` ≤ last accepted → rejected | [[#REQ-495 Signed-Root Freshness]] |
| **TEST-496a** | positive | example | attempts within budget reach [[SPAKE2]] | [[#REQ-496 Pairing Attempt Rate Limit]] |
| **TEST-496b** | neg-input | example | excess attempts dropped pre-handshake; exhaustion aborts pairing + tears down rendezvous | [[#REQ-496 Pairing Attempt Rate Limit]] |

### 10.3 Non-Functional & Robustness Tests

| ID | Technique | Target | Validates |
| -- | --------- | ------ | --------- |
| **TEST-NFR-470** | benchmark | edit→render ≤ 16 ms 95p | [[#NFR-470 Local Edit-to-Render Latency]] |
| **TEST-NFR-471** | benchmark | pairing ≤ 10 s / ≤ 30 s 95p | [[#NFR-471 Pairing Completion Latency]] |
| **TEST-NFR-472** | benchmark | reconnect resolve ≤ 15 s 95p | [[#NFR-472 Reconnect Discovery Latency]] |
| **TEST-NFR-473** | timing-side-channel | 50 ms-delta indistinguishability | [[#NFR-473 Failure-Cause Timing Indistinguishability]] |
| **TEST-NFR-474** | benchmark | revoke ≤ 60 s 99p to online survivors | [[#NFR-474 Revocation Propagation]] |
| **TEST-NFR-475** | property | generated phrases meet the entropy floor | [[#NFR-475 Pairing Secret Entropy Floor]] |
| **TEST-NFR-476** | benchmark | ≤ 80 MB RSS, ≤ 1% CPU idle | [[#NFR-476 Daemon Resource Footprint]] |
| **TEST-NFR-477** | benchmark | remote apply+render ≤ 400 ms 95p | [[#NFR-477 Remote Edit Propagation Latency]] |
| **TEST-fuzz-spake** | fuzz | [[SPAKE2]]/pairing decoder vs random bytes | [[#REQ-483 Full Recognition at Trust Boundaries]] |
| **TEST-fuzz-loro** | fuzz | [[Loro]] update import vs hostile frames | [[#REQ-483 Full Recognition at Trust Boundaries]] |
| **TEST-fuzz-markdown** | fuzz | restricted-CommonMark import recogniser vs hostile input (F13) | [[#REQ-483 Full Recognition at Trust Boundaries]], [[#REQ-484 Guarded Import of External Markdown Edits]] |
| **TEST-mut-rendezvous** | mutation | ≥ 90% kill on rendezvous derivation | [[#REQ-476 DHT-Bootstrapped SPAKE2 Pairing]], [[#NFR-475 Pairing Secret Entropy Floor]] |
| **TEST-mut-roster** | mutation | ≥ 90% kill on roster/revocation | [[#REQ-481 Revocation by Key Rotation]], [[#REQ-482 Roster-Gated Encrypted Transport]] |
| **TEST-adv-pairing** | adversarial | attack pairing/rendezvous to Adversary Exhaustion | [[#REQ-476 DHT-Bootstrapped SPAKE2 Pairing]]–[[#REQ-483 Full Recognition at Trust Boundaries]] |

---

## 11. Observability Signals

| ID | Type | Signal | Trace |
| -- | ---- | ------ | ----- |
| **OBS-470** Daemon health | metric | `zetld_uptime_seconds`, `zetld_attached_clients`, RSS/CPU | [[#REQ-470 Persistent Daemon Ownership]], [[#NFR-476 Daemon Resource Footprint]] |
| **OBS-471** Materialisation | metric | `zetl_materialise_duration_seconds`; determinism-mismatch counter | [[#REQ-473 Deterministic Materialisation]] |
| **OBS-472** Sync convergence | metric | `zetl_sync_converged_total`, `zetl_sync_lag_ops`, `zetl_remote_apply_latency_seconds` | [[#REQ-474 Conflict-Free Offline Merge]], [[#NFR-477 Remote Edit Propagation Latency]] |
| **OBS-473** Discovery | metric | `zetl_pkarr_resolve_duration_seconds`, success/fail counters | [[#REQ-475 Serverless Peer Discovery]], [[#NFR-472 Reconnect Discovery Latency]] |
| **OBS-474** Pairing | metric | `zetl_pairing_duration_seconds`, `zetl_pairing_started_total` | [[#REQ-476 DHT-Bootstrapped SPAKE2 Pairing]], [[#NFR-471 Pairing Completion Latency]] |
| **OBS-475** Pairing failure cause | metric | `zetl_pairing_failed_total{cause}` — **operator-channel label only** | [[#REQ-477 Phrase OOB-Only Non-Leak]], [[#REQ-479 Failure-Message Indistinguishability]], [[#REQ-496 Pairing Attempt Rate Limit]] |
| **OBS-476** Pairing outcome log | log | operator-channel line per outcome with cause | [[#REQ-479 Failure-Message Indistinguishability]] |
| **OBS-477** Roster audit | log | roster add/revoke + key-epoch rotation, [[NodeId]] + ts | [[#REQ-480 Group-Key Admission Gate]], [[#REQ-481 Revocation by Key Rotation]] |
| **OBS-478** Off-roster rejections | metric | `zetl_offroster_rejections_total`, `zetl_malformed_frames_total` | [[#REQ-482 Roster-Gated Encrypted Transport]], [[#REQ-483 Full Recognition at Trust Boundaries]] |
| **OBS-479** External-edit import | log | import outcome (`folded`/`staged`) | [[#REQ-484 Guarded Import of External Markdown Edits]] |
| **OBS-480** Convergence witness | metric | `zetl_root_mismatch_total` (integrity alarm), `zetl_reconcile_rounds`, `zetl_reconcile_skipped_total` (equal-root) | [[#REQ-485 Merkle Convergence Witness]], [[#REQ-486 Merkle Anti-Entropy Reconciliation]] |
| **OBS-481** Message recognition | metric+log | `zetl_cbcl_reject_total{cause}` (parse / R1–R5 / R4-sig) on incoming control messages against the shipped dialect | [[#REQ-487 Control-Plane Messages Recognised by the CBCL DPDA]], [[#REQ-488 Choreographies as Verified R5 Causal-Protocol Contracts]] |

> [[#OBS-475 Pairing failure cause]]'s `cause` label is operator-channel only and
> MUST NOT be exposed via any unauthenticated metrics endpoint, lest it become an
> oracle defeating [[#REQ-479 Failure-Message Indistinguishability]].

---

## 12. Threat Model (Summary)

> Full model: DESIGN-047 task `threat-model` → `research/SPEC-047-threat-model.md`.
> **§H is the decisive new risk** and gates the human crypto review.

- **A. Passive network observer** — recovers neither phrase (REQ-477) nor
  [[SPAKE2]] key (protocol); content transit-encrypted (REQ-482).
- **B. Active OOB-channel MitM** — learns the phrase; [[SPAKE2]] bounds to one
  online guess; single-use (REQ-478) + owner notices the legit join failing.
  *Documented residual.*
- **C. Off-roster connection** — rejected before any frame (REQ-482, REQ-483).
- **D. Compromised member device** — holds [[Group Key]] + already-synced
  plaintext ([[Encryption at Rest]] opt-in, ADR-476). Recovery = revoke + rotate
  (REQ-481); past-content retention inherent to shared-key models. *Residual.*
- **E. Active protocol MitM at pairing (phishing)** — relays [[SPAKE2]] between
  victim and real peer. Carries [[SPEC-036-spake2-onboarding]] §G; candidate =
  [[NodeId]]-fingerprint OOB confirmation. *Open; threat-model task.*
- **F. Malformed/hostile frames** — full recognition before action (REQ-483),
  fuzzed ([[#TEST-fuzz-loro]], [[#TEST-fuzz-spake]]); roster check precedes parse.
  Control-plane **parser-differential** attacks are eliminated by construction:
  the single [[CBCL]] DPDA gives parser-equivalence (one parse tree per input,
  Lean-verified — REQ-487, [[#ADR-479 CBCL as the Control-Plane Message Language]]).
- **G. DHT privacy / metadata** — durable [[NodeId]]→endpoint records leak
  liveness. *Mitigations:* short TTLs, rotating records; threat-model task. A
  [[#8.8 Signed Vault Root]] carried in the record additionally leaks *state*
  progression (root changes over time) → the threat-model task weighs whether to
  announce signed roots peer-to-peer only, not via the public DHT.
- **H. Rendezvous enumeration** — a DHT scanner can enumerate live
  [[Rendezvous]] points. **Resolved by the routing/secret split (ADR-473):** the
  locator is derived from `num` (public routing) only, so enumeration reveals
  *which meeting rooms are busy*, never the secret. The [[SPAKE2]] secret is
  `word-word`, never on the DHT; its resistance is online-single-guess (~22 bits)
  bounded by single-use ([[#REQ-478 Single-Use Phrase]]) and daemon-side
  rate-limiting — which, unlike un-throttleable DHT lookups, the daemon controls.
  *Residual:* `num`-enumeration leaks the metadata "someone is pairing now," and
  pins a small routing space (§J). No longer the decisive blocker, but the split
  itself remains in the Tier-1 crypto-review scope.
- **I. Forged / stale convergence witness** — a peer claims a
  [[Merkle Vault Root]] it has not reached, or replays an old signed root. *Mitigation:* the
  [[#8.8 Signed Vault Root]] frame is ed25519-signed and verified against the
  roster [[NodeId]] before trust (REQ-483); a root is only *trusted as converged*
  after the [[Loro]] state that produces it is actually applied — the witness
  cross-checks merge, it never substitutes for it (REQ-485).
- **J. Rendezvous squat / overwrite** — because `num` is public, an attacker can
  publish a [[pkarr]] record at a live rendezvous (squat) or overwrite the
  owner's, positioning as a MitM or denying the meeting (DoS). *Mitigation:*
  [[SPAKE2]] denies auth (the attacker lacks `word-word`, so key agreement fails
  and no secret leaks — same one-guess bound); the joiner sees a failed handshake
  and the owner re-pairs. *Residual:* availability/phishing pressure; interacts
  with §E (pairing phishing) — both weigh the [[NodeId]]-fingerprint OOB
  confirmation in DESIGN-047 `threat-model`.
- **K. Cross-epoch signed-root replay by a revoked member** — revocation rotates
  the [[Group Key]] epoch, but a member's durable ed25519 [[NodeId]] does not
  change, so a since-revoked member can re-present an old R4-signed
  `{nodeid, epoch_n, root}` after rotation; the signature still verifies.
  *Mitigation:* [[#REQ-493 Signed-Root Epoch Binding]] rejects any root whose
  `key_epoch` ≠ the verifier's current epoch, so a stale-epoch witness is not
  trusted (F9). **Same-epoch** replay (a captured current-epoch root re-presented)
  is defeated by the monotonic `root_seq` in [[#8.8 Signed Vault Root]] (F29).
  *Residual:* none beyond §D's past-content retention.
- **L. Malicious-member state poisoning** — a *current* roster member can author
  valid [[Loro]] ops that materialise to poisoned-but-well-formed content; both
  peers converge and the [[Merkle Vault Root]] equally "confirms" the poison
  (the witness attests *agreement*, not *authorisation* — F10,
  [[#REQ-485 Merkle Convergence Witness]]). *Mitigation:* none at this layer — it
  is the write-side dual of ADR-477's "any member reads the whole vault"; a
  documented residual. Sub-vault write scoping is the
  [[#ADR-477 Single Per-Vault Group Key|web-of-trust]] successor's concern.
- **M. Pairing-burn DoS** — rendezvous locators are enumerable (§H) and a
  phrase is consumed by its *first* [[SPAKE2]] exchange regardless of outcome
  ([[#REQ-478 Single-Use Phrase]], F32), so an attacker watching the DHT can
  connect to each new rendezvous with a garbage password and burn the pairing.
  *Deliberate trade:* first-exchange-consumption preserves the one-online-guess
  bound on the ~22-bit secret; the alternative (success-only consumption) would
  grant repeated guesses. *Mitigation:* [[#REQ-496 Pairing Attempt Rate Limit]]
  drops excess pre-handshake; the burn is visible (the owner's `pair` fails)
  and the owner re-pairs. *Residual:* availability nuisance under active
  attack — accepted for v1 (pairing is rare and interactive).
- **N. Malicious-member admission / roster fork** — any current member holds
  the [[Group Key]] and can seal it onward to a third party, run `pair` on its
  own replica, or fork the roster; the Owner-only pre-conditions
  ([[#CON-474 Pairing Protocol]] C1, [[#CON-477 Group Key Roster and Revocation]] C1)
  bind *honest* daemons only — membership authority is local policy, not
  cryptography (F34). *Mitigation:* none at this layer — the write/admission
  dual of §L and of [[#ADR-477 Single Per-Vault Group Key]]'s read-coarseness;
  the [[users/vault-owner/user|Owner]] profile's "controls membership" goal is
  scoped accordingly. *Residual:* documented; the web-of-trust successor's
  concern.
- **O. Revocation propagation window** — a survivor offline at revoke time
  ([[#NFR-474 Revocation Propagation]] binds *online* survivors only) still
  holds the old epoch and the revoked [[NodeId]] on its local roster, and will
  sync **new** content with the revoked peer until it contacts a resealed
  member (F35) — distinct from §D's past-content retention. *Mitigation:*
  epoch precedence / anti-rollback — a peer that learns a higher `key_epoch`
  MUST refuse older epochs; specified in the Q7 group-key package
  ([[#18. Open Questions]] Q7). *Residual:* unbounded for indefinitely-offline
  survivors.

---

## 13. Purity Boundary Map

### Pure Core (no I/O, no shared state, deterministic)

- `p2p::pair::phrase::generate(rng) -> Phrase` — `num-word-word` generator.
- `p2p::pair::rendezvous::derive(phrase) -> RendezvousKeypair` — [[HKDF]];
  **no-go area, human-reviewed.**
- `p2p::pair::spake::{start, finish}` — wraps the [[SPEC-034]] [[SPAKE2]] driver.
- `p2p::pair::groupkey::{seal, open, rotate}` — [[Group Key]] envelope ops.
- `crdt::loro::materialise(doc) -> Markdown` — deterministic export (REQ-473).
- `crdt::loro::import::plan(markdown, last_export, edit_state) -> ImportOutcome`
  — guarded-import decision (REQ-484).
- `crdt::merkle::vault_root(asts) -> ContentHash` + `merkle::diff(root_a, root_b)
  -> Vec<DocId>` — reuse `src/merkle.rs` ([[SPEC-006]]); convergence witness +
  DAG descent (REQ-485, REQ-486). Pure.
- `p2p::proto::recognise(text) -> Result<Message>` + R1–R5 validators — reuse
  [[CBCL]] `cbcl-core`/`cbcl-parser` (`no_std`, pure); control-plane recognition
  (REQ-487, REQ-488). R4 signature *verification* is pure; key *custody* is shell.
- `p2p::pair::error::classify(cause) -> FailureCategory` — operator-only; user
  text constant (REQ-479).

### Effectful Shell (orchestrates I/O, calls pure core)

- `daemon::zetld` — lifecycle, control socket, supervision.
- `p2p::iroh` — [[QUIC]] endpoint, accept loop, roster enforcement.
- `p2p::pkarr` — DHT publish/resolve (rendezvous + durable records).
- `crdt::store` — `.zetl/loro/` persistence; oplog append; snapshotting.
- `crdt::materialise_sink` — write Markdown + drive git/jj flush.
- `p2p::pair::store` — roster + nonce persistence; rendezvous TTL pruning.
- `daemon::watch` — external-edit watcher feeding guarded import.

### Boundary Contracts (data types crossing the boundary)

`Phrase` (pure→shell; never persisted plaintext), `RendezvousKeypair`,
`SpakeMessage` (shell↔shell; opaque to core), `GroupKey` (sealed by shell),
`LoroUpdate` (shell↔shell), `ImportOutcome` (core→shell), `FailureCategory`
(core→shell; operator log only).

### Dependency Rule

Dependencies point inward: shell → core. Core MUST NOT import from shell. (This
is the inward arrow drawn in the [[#Orientation]] diagram.)

### Enforcement

`clippy::disallowed_methods` banning `std::fs::*`, `std::time::SystemTime::now`,
`tokio::*`, iroh, and pkarr crates inside the pure modules; module visibility — as
in [[SPEC-036-spake2-onboarding]] §11.

---

## 14. Traceability

Per [[PROTO-001]] §Traceability Links — bidirectional, load-bearing for
requirement-localised repair; encoding = the `[[wikilinks]]` above, validated by
`zetl check --dead-links`.

```
REQ-### ←π→ TEST-### → CODE → OBS-###
        ↓                       ↑
      CON-###               BUG-### (none yet)
        ↓
      ADR-###
```

| REQ | TEST (π) | CON | ADR | OBS |
| --- | -------- | --- | --- | --- |
| 470 | 470a | 470 | — | 470 |
| 471 | 471a/b/c | 470 | — | 470 |
| 472 | 472a/c/d | 471 | 470 | 471 |
| 473 | 473a/c | 471 | 470 | 471 |
| 474 | 474a/c | 473 | 470 | 472 |
| 475 | 475a/b | 473 | 472 | 473 |
| 476 | 476a, mut-rendezvous, adv | 474 | 473 | 474 |
| 477 | 477a/b/c | 474 | 473 | 475 |
| 478 | 478a/b | 474 | 473 | 475 |
| 479 | 479a/c, NFR-473 | 474 | — | 475,476 |
| 480 | 480a/b | 474,477 | 477 | 477 |
| 481 | 481a/b/c, mut-roster, NFR-474 | 477 | 477 | 477 |
| 482 | 482a/b, mut-roster | 473 | 472,476 | 478 |
| 483 | 483a/b/c, fuzz-spake, fuzz-loro | 470,473,474 | 472 | 478 |
| 484 | 484a/b/c/d | 471 | 471 | 479 |
| 485 | 485a/c | 473 | 478 | 480 |
| 486 | 486a/b/c | 473 | 478 | 480 |
| 487 | 487a/b | 470,473,474 | 479 | 481 |
| 488 | 488a/b | 473,474 | 479 | 481 |
| 489 | 489a/b | 470,474,477 | 480 | — |
| 490 | 490a/c | 470 | — | 470 |
| 491 | 491a,476b,476c | 474 | 473 | 474 |
| 492 | 492a/b | 473 | 472,476 | 478 |
| 493 | 493a/b | 473 | 478 | 477 |
| 494 | 494a/b | 473 | 479 | 478 |
| 495 | 495a/b | 473 | 478 | 477 |
| 496 | 496a/b | 474 | 473 | 475 |

Every REQ links ≥ 1 TEST per applicable type and ≥ 1 OBS (post-release;
REQ-489 is a build-time CLI-surface conformance REQ with no runtime signal —
excepted); every TEST records `Validates:` (§10). π is total in both directions
across §4 and §10.

---

## 15. AI Trust Boundaries Record

Per [[PROTO-001]] §AI Trust Boundaries. Tier-1 ⇒ synthesis trajectory recorded.

- **Model / version:** Claude Opus 4.8 [1M].
- **Prompts / templates:** `superpowers:brainstorming` + [[PROTO-001]] v1.11.0
  templates; user direction (greenfield; persistent daemon; pkarr/DHT [[SPAKE2]];
  `num-word-word` phrase).
- **Parameters:** default.
- **Inputs:** user request; codebase survey (`src/crdt/`, `src/web/`,
  `src/cap/pair.rs`); [[SPEC-036-spake2-onboarding]]; [[PROTO-001]] v1.11.0.
- **Outputs:** this strawman.
- **Synthesis trajectory (Tier-1):**
  - S₀ — prose sketch (partial templates).
  - S₁ — rewrite to v1.6.0 templates (REQ/NFR/CON, test decomposition,
    traceability, ambiguity scan).
  - S₂ — **this** rewrite to v1.11.0: added the Orientation block + ASCII
    structure; BCP-14 conformance; split compound REQs to one keyword each
    (Δ: REQ-471→{canonical store, deterministic materialise}; OOB+single-use →
    {REQ-477, REQ-478}; admission+revocation → {REQ-480, REQ-481}); added LangSec
    §8 grammars + REQ-483 + per-CON Grammar/Recogniser; added §9 capability
    placement / Simplicity Ladder; moved history to the foot `## Changelog`.
  - S₃ — folded in the [[Merkle DAG]] reuse ([[SPEC-006]]): ADR-478, REQ-485
    (convergence witness), REQ-486 (anti-entropy reconciliation), §8.8 signed-root
    grammar, CON-473 `reconcile` phase, OBS-480, threat-model §I; content-addressed
    block transfer deferred to Q8.
  - S₄ — adopted [[CBCL]] (`../cbcl-rs`) as the control-plane message language:
    ADR-479, REQ-487 (DPDA recognition + R4), REQ-488 (R5 causal-protocol
    choreographies), §8 control/data-plane split, CON-470/473/474 grammar fields,
    OBS-481, threat-model §F (parser-differential eliminated by construction);
    bespoke CBOR validators removed; Q9 (canonical-encoding) raised.
  - S₅ — aligned the CLI to existing zetl conventions (ADR-480, REQ-489):
    bare `zetl pair`/`zetl join`/… → `zetl collab {pair,join,peers,revoke}` +
    `zetl daemon {start,stop,status}`; reused SPEC-036's `zetl collab join` and
    the TTY-only secret + global-format + positional-id + `not-yet-implemented`
    conventions; updated CON-470/474/477, §1.3, HP1/HP4.
  - S₆ — fresh-context adversarial review + comprehension gate (PROTO-001
    Principle 12 / §Comprehension gate). Fixed F1/F3/F15 (self-introduced
    cross-ref defects); resolved F8 via the routing/secret phrase split
    (ADR-473/NFR-475/§8.5/§H/§J).
  - S₇ — applied the remaining accepted batch (0.8.0): F2 (§8 grammar headings)
    + F14 (§8.9 pkarr), F5/F6/F7 atomicity splits (REQ-490/491/492), F9 (REQ-493
    + §K), F10 (REQ-485 wording + §L), F11 (REQ-473 canonical form), F12
    (REQ-494), F13 (TEST-fuzz-markdown), F16 (REQ-484 logical-time predicate),
    F18 (§16 corrections). F19 had been resolved in S-prior (gossip deferred).
  - S₈ — second review pass on 0.8.0 (0×S1/5×S2/5×S3). Fixed batch-introduced
    drift + half-applied fixes: F21/F22/F25 (§14 trace drift), F23/F24
    (TEST-482b → cleartext obligation), F26 (ADR-471 logical-time), F27 + "(no-go)"
    label (Orientation), F28 (§8.1 recognised binding), F29 (REQ-495 + `root_seq`
    + §K), F30 (atomicity note). → 0.9.0.
  - S₉ — third fresh-context adversarial pass (Claude Fable 5 — a different
    model but the **same vendor family** as the generator, so it does NOT
    discharge the Tier-1 cross-model gate) + the Orientation comprehension
    gate run by a fresh-context subagent given only the Orientation block
    (passed: intent restated, both behaviour predictions correct, all four
    locate-the-artefact probes correct). Applied F31–F44: REQ-496 pairing
    attempt rate limit (F31); REQ-478 first-exchange-consumes + threat §M
    (F32); REQ-486/ADR-478/CON-473 version-vector guard on the equal-root
    fast path (F33); ADR-477 + threat §N admission-authority residual (F34);
    threat §O revocation window + Q7 anti-rollback (F35); NFR-477 remote
    edit-propagation latency (F36); REQ-487 message-vs-dialect signature
    rewording (F37); REQ-484 deletes/renames (F38); committed = fsync-appended
    (REQ-472/ADR-470, F39); NFR-473 rescoped to post-connection causes with
    padding (F40); mutual key confirmation (HP1/REQ-491, F41); ADR-471 NFC
    re-materialisation consequence (F42); Q3 granularity dependency (F43);
    five line-wrapped wikilinks unwrapped (F44); Orientation legend +
    scrambled-channel metaphor + §H reference fixes (comprehension-gate
    findings).
  - Adversarial tests: not yet generated (DESIGN-047 `test-strategy` +
    cross-model).
- **Reviewer:** **PENDING** — Tier-1 requires cross-model adversarial review,
  the fresh-context **comprehension gate** on the Orientation block
  ([[PROTO-001]] §Comprehension as a Verifiable Gate), **and** a human domain
  expert (cryptography + auth core).
- **Decision:** **NOT APPROVED** — strawman; no implementation until the review
  package (with the §H enumeration analysis) is signed off.

---

## 16. Quality Attribute Checklist

Per [[PROTO-001]] (15 attributes), applied to §4. ⚠ = closes when the named
DESIGN-047 task lands.

| REQ | Unamb. | Correct | Complete | Underst. | Achiev. | Concise | Design-indep. | Prior. | Verif. | Precise | Consist. | Atomic | Non-redun. | Traceable | Error-aware |
| --- | :----: | :-----: | :------: | :------: | :-----: | :-----: | :-----------: | :----: | :----: | :-----: | :------: | :----: | :--------: | :-------: | :---------: |
| 470 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ | ✓ | ✓ | ✓ |
| 471 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ | ✓ | ✓ | ✓ |
| 472 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ | ✓ | ✓ | ✓ |
| 473 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| 474 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ | ✓ | ✓ | ✓ |
| 475 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| 476 | ⚠ ADR-473 | ✓ | ✓ | ✓ | ⚠ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| 477 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ | ✓ | ✓ | ✓ |
| 478 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ | ✓ | ✓ | ✓ |
| 479 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | NFR-473 | ✓ | ✓ | ✓ | ✓ | ✓ |
| 480 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ | ✓ | ✓ | ✓ |
| 481 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| 482 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ | ✓ | ✓ | ✓ |
| 483 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ | ✓ | ✓ | ✓ |
| 484 | ⚠ ADR-471 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ | ✓ | ✓ | ✓ |
| 485 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ | ✓ | ✓ | ✓ |
| 486 | ⚠ ADR-478 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ | ✓ | ✓ | ✓ |
| 487 | ⚠ ADR-479 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ | ✓ | ✓ | ✓ |
| 488 | ⚠ ADR-479 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ | ✓ | ✓ | ✓ |
| 489 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ | ✓ | ✓ | ✓ |
| 490 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ | ✓ | ✓ | ✓ |
| 491 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ | ✓ | ✓ | ✓ |
| 492 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ | ✓ | ✓ | ✓ |
| 493 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ | ✓ | ✓ | ✓ |
| 494 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ | ✓ | ✓ | ✓ |
| 495 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ | ✓ | ✓ | ✓ |
| 496 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ⚠ | ✓ | ✓ | ✓ | ✓ | ✓ |

> **F18/F30 corrections.** The atomicity splits (F5/F6/F7 → REQ-490/491/492) make
> REQ-470/476/482 genuinely single-obligation; their ✓Atomic now holds rather
> than being asserted over a bundle. For REQ-485/487/488 (F30): the trailing
> `WITH …` clause is the *error-aware* sub-clause of a single obligation (the
> alarm/rejection is how that obligation fails closed), not a second independent
> SHALL — ✓Atomic is retained on that reading; a future split is recorded as an
> option if the DESIGN-047 test-strategy task wants per-clause failure attribution. **Per-REQ test-type omissions** (where a REQ
> shows only `a`/`c` or `a`/`b`): each is the protocol-sanctioned omission for a
> pure output-constraining or input-constraining obligation respectively; the
> DESIGN-047 `test-strategy` task records the explicit justification per REQ
> before any code (currently a known gap, not a silent one). REQ-496's ⚠ Precise
> closes when DESIGN-047 `adr-rendezvous` fixes the provisional attempt budget.

---

## 17. Ambiguity Resolution

Per [[PROTO-001]] §Ambiguity Resolution — prohibited vague terms replaced.

| Vague | Where it would arise | Replaced with |
| ----- | -------------------- | ------------- |
| "fast" local echo | keystroke render | ≤ 16 ms 95p ([[#NFR-470 Local Edit-to-Render Latency]]) |
| "realtime" co-editing | remote edit visibility | ≤ 400 ms 95p ([[#NFR-477 Remote Edit Propagation Latency]]) |
| "quick" pairing | onboarding | ≤ 10 s / ≤ 30 s 95p ([[#NFR-471 Pairing Completion Latency]]) |
| "secure" pairing | [[SPAKE2]] ceremony | online-single-guess + single-use + entropy floor + full recognition ([[#REQ-476 DHT-Bootstrapped SPAKE2 Pairing]], [[#REQ-478 Single-Use Phrase]], [[#NFR-475 Pairing Secret Entropy Floor]], [[#REQ-483 Full Recognition at Trust Boundaries]]) |
| "reliable" merge | offline merge | conflict-free + zero loss + convergence ([[#REQ-474 Conflict-Free Offline Merge]]) |
| "lightweight" daemon | always-on process | ≤ 80 MB RSS, ≤ 1% CPU idle 95p ([[#NFR-476 Daemon Resource Footprint]]) |
| "promptly" revoked | revocation | ≤ 60 s 99p to online survivors ([[#NFR-474 Revocation Propagation]]) |

---

## 18. Open Questions

1. **Validate the routing/secret split (was: phrase-length decision).** ADR-473
   now splits `num` (public routing locator) from `word-word` (the [[SPAKE2]]
   secret, ~22 bits, never on the DHT), so [[#12. Threat Model]] §H enumeration
   hits only routing. DESIGN-047 `adr-rendezvous` + the human crypto review MUST
   confirm: (a) the split holds under an active DHT adversary; (b) `num` is sized
   for collision-avoidance ([[#NFR-475 Pairing Secret Entropy Floor]]); (c) ~22-bit
   `word-word` is adequate given first-exchange-consumption
   ([[#REQ-478 Single-Use Phrase]]) + the
   [[#REQ-496 Pairing Attempt Rate Limit]]. *(owner: HOC)*
2. **Pairing phishing (§E).** Ship [[NodeId]]-fingerprint OOB confirmation in v1
   or defer? (Carries [[SPEC-036-spake2-onboarding]] §G.)
3. **CRDT granularity.** One [[Loro]] doc per note, or one container tree per
   vault? Affects sync chattiness, presence, snapshot cost. Note:
   [[#REQ-486 Merkle Anti-Entropy Reconciliation]] and §8 word their
   localisation unit as *documents* (doc-per-note); resolving Q3 to a single
   container tree requires rewording that unit (F43).
4. **Oplog growth / shallow snapshots.** Compaction aggressiveness vs
   offline-merge correctness (HP3 fallback).
5. **Mobile daemon.** iOS/Android background limits ([[SPEC-040-zetl-mobile]])
   may force a foreground/push-triggered `zetld`.
6. **Migration from [[diamond-types]].** One-shot `from_markdown` → [[Loro]], or
   oplog-history migration?
7. **Group-key cryptography.** Sealed-sender scheme, key epochs, forward-secrecy —
   no-go-area decisions for human review. Include epoch precedence /
   anti-rollback: a peer that learns a higher `key_epoch` MUST refuse older
   epochs, bounding the stale-survivor window of [[#12. Threat Model]] §O (F35).
8. **Content-addressed block transfer (deferred).** Extend
   [[#ADR-478 Merkle DAG as Convergence Witness and Reconciliation Index]] so a
   joining/large-vault peer fetches only the [[Merkle DAG]] leaves it lacks
   (git-packfile / bitswap style) instead of a full [[Loro]] snapshot. The
   convergence-witness + anti-entropy layer ships in v1; block *transfer* is a
   successor optimisation — does it earn its complexity, and does dedup interact
   safely with [[Encryption at Rest]] ([[#ADR-476 Encryption at Rest is Opt-In]])?
9. **CBCL canonical encoding for the control plane.** [[CBCL]]'s canonical form
   (what R4 signs) is S-expression text. At control-plane volume this is
   acceptable, but does a compact/binary canonical encoding exist or is one
   warranted (signature-stability + bandwidth)?
   ([[#ADR-479 CBCL as the Control-Plane Message Language]])
10. **Runtime dialect gossip (deferred).** v1 ships the `zetl-pair`/`zetl-sync`
    dialects with the CLI binary (fixed per release —
    [[#ADR-479 CBCL as the Control-Plane Message Language]]). Peer-to-peer dialect
    *gossip* (runtime protocol evolution across a fleet) is deferred to a
    successor spec; it would reintroduce a runtime-extension attack surface and
    needs its own rate-limit + R1–R5-at-install threat analysis before it earns a
    place. Not needed for the v1 multi-device profile.

---

## 19. Status & Next Actions

- This strawman is an **input** to `plans/DESIGN-047-loro-p2p-realtime-sync.spl`,
  not an output. The plan refines each Provisional section and runs Phase-0
  prior-art research ([[Loro]], [[iroh]], [[pkarr]] maturity; DHT-privacy
  literature; mobile background constraints).
- **No implementation begins** until (a) Phase 1 + Phase 2 quality gates pass;
  (b) cross-model adversarial review completes; (c) the fresh-context
  comprehension gate on the [[#Orientation]] block passes; (d) a human domain
  expert approves the cryptography + auth-core package, with the §H
  enumeration analysis as the gating artefact.
- On approval, re-issue at `1.0.0`, remove Provisional markers, author the dead
  concept pages surfaced by `zetl check --dead-links`, and mark [[SPEC-004]]
  `superseded`.

---

## Changelog

<details>
<summary>Revision history — 0.1.0 → 0.10.0</summary>

- 0.10.0-strawman — third fresh-context review pass (on 0.9.0; Claude Fable 5,
  same-vendor — the Tier-1 cross-model gate remains undischarged) → 4×S2, 6×S3,
  4×S4, the first pass probing protocol *semantics* rather than document
  structure (severity non-convergence vs 0.9.0's 0×S1/5×S2 noted; spec stays
  strawman). **F31** [[#REQ-496 Pairing Attempt Rate Limit]] — the control
  ADR-473/NFR-475 cited but never specified. **F32** REQ-478 defines redemption
  as first-SPAKE2-exchange-consumes + threat §M pairing-burn DoS (deliberate
  trade preserving the one-guess bound). **F33** REQ-486/ADR-478/CON-473 —
  equal-root fast path now also requires equal version vectors (roots lag
  unmaterialised ops). **F34** ADR-477 + threat §N — admission authority
  unenforceable against a malicious member; Owner profile scoped. **F35**
  threat §O revocation-propagation window + Q7 anti-rollback. **F36** NFR-477
  remote edit-propagation latency (quantifies "realtime"; §17 row). **F37**
  REQ-487 reworded — network messages roster-signed, dialects release
  artefacts; ADR-479 records the per-frame signing cost. **F38** REQ-484
  extended to deletes/renames + TEST-484d. **F39** committed = fsync-appended
  (REQ-472, TEST-472d); ADR-470's WAL-redundancy claim now names the transfer
  of the durability contract. **F40** NFR-473 rescoped to post-connection
  causes, padding recorded, rendezvous-absent excluded from the timing bound.
  **F41** mutual key confirmation before any seal (HP1 step 4, REQ-491).
  **F42** ADR-471/HP5 NFC re-materialisation consequence. **F43** Q3
  granularity dependency on REQ-486's document unit. **F44** five line-wrapped
  wikilinks unwrapped. Orientation: Legend line added (CBCL/DPDA/DCFL, roster,
  CON-###, ─AST▶), metaphor gains "scrambled channel", §H reference labelled
  threat-model — from the fresh-context comprehension gate (passed on intent,
  behaviour prediction, and artefact location).
- 0.9.0-strawman — second fresh-context review pass (on 0.8.0) → 0×S1, 5×S2, 5×S3
  (down from 3×S1/10×S2). Fixed the regressions the batch introduced and the
  half-applied fixes: F21/F22/F25 (§14 traceability drift from the atomicity
  splits — orphaned/duplicated/double-counted traces), F23/F24 (TEST-482b
  repurposed to REQ-482's actual cleartext obligation; pre-frame rejection is
  TEST-492b), F26 (ADR-471 still recorded the wall-clock debounce REQ-484 had
  rejected → logical-time), F27 (Orientation now leads with REQ-491 for the
  pairing secret; REQ-476 labelled discovery), the "(no-go)" Orientation label
  clarified to "crypto · human-gated, not rejected", F28 (REQ-494 hash binding now
  a recognised `(ref id hash)` clause in the §8.1 grammar, not bolted on), F29
  (added monotonic `root_seq` to §8.8 + REQ-495 + threat §K same-epoch replay),
  F30 (§16 atomicity note extended to REQ-485/487/488).
- 0.8.0-strawman — applied the remaining accepted review batch. **F2** — §8
  grammars promoted from table rows to real subsection headings (8.1–8.9, names
  reconciled with their references) so the LangSec anchors resolve; added §8.9
  pkarr-record grammar (**F14**). **Atomicity (F5/F6/F7)** — narrowed REQ-470
  (ownership), REQ-476 (discovery-only), REQ-482 (encrypted transport); split-off
  obligations added as REQ-490 (survive-disconnect), REQ-491 (SPAKE2 channel
  auth), REQ-492 (roster-gate-before-frame) — numbered 490+ to keep ids stable.
  **Security/correctness** — REQ-493 signed-root epoch binding + threat §K (F9);
  REQ-494 control→data content-hash binding (F12); REQ-473 reworked to a declared
  canonical materialisation form (F11); REQ-484 guarded-import predicate moved to
  Loro logical time, not a wall-clock window (F16); REQ-485 reworded to
  "agreement, not authorisation" + threat §L malicious-member poisoning (F10);
  TEST-fuzz-markdown added (F13). §16 atomicity cells corrected + omission
  justification note (F18); traceability/quality tables extended to 494.
- 0.7.0-strawman — resolved review F19/F17: **deferred runtime dialect gossip**;
  the `zetl-pair`/`zetl-sync` [[CBCL]] dialects now **ship with the CLI binary**
  (fixed, release-versioned), so the recognised control language is static per
  release and the runtime-extension attack surface (F17 gossip DoS) is dissolved,
  not just deferred. Reworked ADR-479, REQ-487/488 wording (load/build-time vs
  peer install), OBS-481, §1.3 scope-out, Q10. Merkle witness/reconciliation
  (REQ-485/486) and CBCL recognition/R5 (REQ-487/488) retained per the decision.
  *Still pending: batch F2 (grammar-anchor headings), F5–F7 (atomicity splits +
  renumber), F9–F14, F16, F18.*
- 0.6.0-strawman — fresh-context adversarial review (PROTO-001 Principle 12) +
  comprehension gate run. Fixed structural defects the review found (all
  self-introduced): F1 — Orientation `Load-bearing` mis-cited `REQ-474` for the
  SPAKE2-pairing REQ (it is `REQ-476`); F3 — Orientation `Open` pointed at `#13`
  (Purity Map) not `#18`; F15 — threat letters reordered G→H→I.
  **Resolved F8 (the decisive §H risk) via the routing/secret split** — adopted
  the [[Magic Wormhole]] code structure: `num` derives the public [[Rendezvous]]
  locator (enumerable by design, no secret), `word-word` is the [[SPAKE2]]
  password (never on the DHT). Reworked ADR-473, reframed NFR-475 (secret floor
  on the words ~22 bits; `num` sized for collision-avoidance), §8.5 ABNF, §H, and
  added threat §J (rendezvous squat/overwrite). *(Batch F2/F5–F18 — completed in
  0.8.0.)*
- 0.5.0-strawman — aligned the CLI surface to existing zetl conventions
  ([[#ADR-480 CLI Surface Follows Existing zetl Conventions]],
  [[#REQ-489 P2P CLI Follows Existing zetl Conventions]]): replaced the invented
  bare `zetl pair`/`zetl join`/`zetl peers`/`zetl revoke` with
  `zetl collab {pair,join,peers,revoke}` (extending the existing `zetl collab`
  group, reusing [[SPEC-036-spake2-onboarding]]'s `zetl collab join`) and
  `zetl daemon {start,stop,status}` (paralleling `zetl serve`, managing the
  `zetld` process); inherits the global `--format`/`--json`/`--vault`, TTY-only
  phrase entry, positional ids, and `not-yet-implemented` non-zero-exit
  conventions; updated §1.3, HP1/HP4, CON-470/474/477, §9.
- 0.4.0-strawman — adopted [[CBCL]] (`../cbcl-rs`) as the **control-plane message
  language** ([[#ADR-479 CBCL as the Control-Plane Message Language]],
  [[#REQ-487 Control-Plane Messages Recognised by the CBCL DPDA]],
  [[#REQ-488 Choreographies as Verified R5 Causal-Protocol Contracts]]): one
  Lean-verified [[DCFL]] DPDA recognises all control messages (parser-equivalence
  → no parser-differential surface), R4 carries message auth, R5 encodes the
  pairing/reconcile choreographies as verified contracts; §8 split into a CBCL
  control plane and a binary data plane ([[Loro]]/[[SPAKE2]] blobs); bespoke CBOR
  validators removed; OBS-481, threat-model §F strengthened, Q9 raised.
- 0.3.0-strawman — reused the [[SPEC-006]] [[Merkle DAG]] as a layer over
  [[Loro]]: [[#ADR-478 Merkle DAG as Convergence Witness and Reconciliation Index]],
  [[#REQ-485 Merkle Convergence Witness]], [[#REQ-486 Merkle Anti-Entropy Reconciliation]],
  §8.8 signed-vault-root grammar, a `reconcile` phase on CON-473, OBS-480, and
  threat-model §G/§I; content-addressed block *transfer* deferred to Open
  Question Q8.
- 0.2.0-strawman — conformed to [[PROTO-001]] v1.11.0: added the **Orientation
  block** (Intent/Metaphor/ASCII structure/Decisions/Load-bearing/Open) and the
  **BCP-14 Conformance** declaration; enforced keyword atomicity by splitting
  compound REQs (canonical-store ⁄ deterministic-materialise; phrase
  non-leak ⁄ single-use; admission ⁄ revocation), renumbering to REQ-470…484;
  added **LangSec** §8 input grammars + REQ-483 + per-contract
  Grammar/Recogniser fields; added §9 **Capability Placement & Simplicity
  Ladder** (Principle 15); moved the revision history to this collapsed
  `## Changelog`.
- 0.1.0-strawman — initial sketch (daemon + Loro-as-truth + iroh/pkarr +
  DHT-bootstrapped SPAKE2; `num-word-word` phrase; threat-model §H raised).

</details>
