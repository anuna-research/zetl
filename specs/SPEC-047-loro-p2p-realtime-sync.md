---
id: SPEC-047
title: "Loro CRDT Store + P2P Realtime Sync Daemon with DHT-Bootstrapped SPAKE2 Pairing"
status: draft
version: 0.21.0-strawman
last-updated: 2026-07-18
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
                       │  CON-470  control: CBCL S-expressions (LangSec)
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
           the per-vault map of member DID → device NodeIds (CON-477),
           mutated only by Owner-signed MLS commits (REQ-505 · ADR-482,
           the mechanism realising ADR-477's single Group Key — so
           "who may add members" = the Owner, and only the Owner);
           CON-### = interface contracts (§7); ─AST▶ = only typed,
           validated parse output crosses into the pure core.

Decisions:    [[#ADR-470 Loro as Canonical Store Markdown+Git as Export]] ·
              [[#ADR-472 iroh + pkarr for Transport and Discovery]] ·
              [[#ADR-473 Phrase-Derived DHT Rendezvous for SPAKE2]] (crypto · human-gated, not rejected) ·
              [[#ADR-477 Single Per-Vault Group Key]] ·
              [[#ADR-478 Merkle DAG as Convergence Witness and Reconciliation Index]] ·
              [[#ADR-479 CBCL as the Control-Plane Message Language]] ·
              [[#ADR-481 did:crdt as the Member Identity Layer]] (auth-core · human-gated) ·
              [[#ADR-482 MLS for Group Key Agreement and Membership Commits]] (crypto · human-gated, stakeholder-directed)
Load-bearing: [[#REQ-491 SPAKE2 Channel Authentication]] (the pairing secret) ·
              [[#REQ-476 DHT-Bootstrapped SPAKE2 Pairing]] (rendezvous discovery) ·
              [[#REQ-477 Phrase OOB-Only Non-Leak]] ·
              [[#REQ-482 Roster-Gated Encrypted Transport]] ·
              [[#REQ-499 Group-Keyed Sync Frames]] (rotation bites the wire) ·
              [[#REQ-474 Conflict-Free Offline Merge]] ·
              [[#NFR-475 Pairing Secret Entropy Floor]]
Open:         threat-model §H/§J rendezvous enumeration + rendezvous-record
              write authority (a DHT-record property — membership authority
              is settled, Owner-only) vs
              phrase entropy (owner: HOC, gating the Tier-1 crypto review) →
              [[#18. Open Questions]] Q1; §B OOB-compromise recovery
              (fingerprint/SAS) → Q2
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
| Version      | 0.21.0-strawman                                                                          |
| Status       | Draft (strawman; pending DESIGN-047 execution)                                          |
| Author       | Agent (Claude Opus 4.8 [1M]) under [[PROTO-001]] v1.11.0                                |
| Audience     | Agent, Human                                                                            |
| Trace        | [[PROTO-001]] §Phase 1, §Phase 2, §AI Trust Boundaries                                  |
| Parent       | [[SPEC-020]] Multi-User Collaborative Editing                                           |
| Supersedes   | [[SPEC-004]] Distributed Sync (see [[#ADR-475 Supersede SPEC-004 Goblins OCapN Sidecar]]) |
| Related      | [[SPEC-034]], [[SPEC-036-spake2-onboarding]], [[SPEC-040-zetl-mobile]], [[SPEC-041-pluggable-collab-auth]]; downstream implementation evidence: [[elephant-3000]] (`../elephant-3000`, whose SPEC-002/SPEC-004 name this spec as their pattern source — F67–F71) |
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
located on the DHT. Identity is layered: a *device* is an ed25519 [[NodeId]]; a
*member* (person) is a [[did:crdt]] DID whose verification methods enumerate that
member's device NodeIds ([[#ADR-481 did:crdt as the Member Identity Layer]]).
There is no central server: no editing hub and no
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
discovery; a [[did:crdt]] member-identity layer over device [[NodeId]]s
([[#ADR-481 did:crdt as the Member Identity Layer]]); serverless [[SPAKE2]]
pairing into a per-vault [[Group Key]] managed by an [[MLS]] group
([[#ADR-482 MLS for Group Key Agreement and Membership Commits]]); CLI
under the existing groups — `zetl daemon {start,stop,status}` (mirroring
`zetl serve`) and `zetl collab {invite,join,peers,revoke}` (extending the existing
`zetl collab` group, reusing SPEC-036's `zetl collab join`) per
[[#ADR-480 CLI Surface Follows Existing zetl Conventions]]; input grammars,
threat model, observability.

**Out of scope (successor specs):** pairwise web-of-trust ACLs and
delegatable admission authority
([[#ADR-477 Single Per-Vault Group Key]], F66); multi-user as a *v1* milestone
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
  (enforceable among honest daemons — [[#REQ-505 Role-Gated Membership Authority]]
  makes honest verifiers reject non-owner membership commits; against a
  malicious key-holder membership authority remains local policy, not
  cryptography, until the web-of-trust successor; [[#12. Threat Model]] §N).
- **Constraints:** Owns the vault; drives `zetl collab invite`/`zetl collab revoke`.
- **Daily workflow:** invite on demand → audit `zetl collab peers` → revoke on loss.

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

1. `zetl collab invite --vault notes` on A → daemon mints a `num-word-word`
   [[Pairing Phrase]] (e.g. `4732-walnut-harbor` — the routing number is 4–5
   digits per [[#8.5 Pairing Phrase]]), prints it to stdout + a "waiting"
   status to stderr, derives the [[Rendezvous]] keypair, and publishes a short-TTL
   [[pkarr]] record at the rendezvous pubkey pointing at a fresh **ephemeral
   pairing endpoint** — a single-use ed25519 keypair minted for this ceremony;
   A's durable [[NodeId]] never appears in the enumerable record (F54,
   [[#12. Threat Model]] §H).
2. `zetl collab join` on B → CLI prompts for the phrase via TTY (never argv/env,
   mirroring `zetl collab passwd add`).
3. Operator types `4732-walnut-harbor` into B → B derives the same rendezvous
   keypair and resolves the record from the [[Mainline DHT]] → learns A's
   ephemeral pairing endpoint.
4. B opens a [[QUIC]] stream to A, itself using a fresh ephemeral pairing
   [[NodeId]] — the two ephemeral keys are the ceremony's
   [[Pre-Admission Pairing Identity|pre-admission identities]], against which
   pairing control-message signatures verify (F45,
   [[#REQ-487 Control-Plane Messages Recognised by the CBCL DPDA]]) → A and B
   run [[SPAKE2]] (phrase = password) → shared session key; mutual key
   confirmation ([[HMAC]] over a transcript that binds both ephemeral pairing
   NodeIds, both directions) completes before any key material is sealed (F41).
5. Inside the confirmed channel both disclose their durable [[NodeId]]s. For the
   solo profile, A authors a signed [[did:crdt]] delta adding B's durable
   [[NodeId]] as a verification method of A's member DID; for a new member, B
   presents a self-certifying DID **genesis delta**
   ([[#8.10 did:crdt Delta]], F47) accepted only inside this ceremony. A seals
   the vault [[Group Key]] to B and transfers the current roster + DID
   documents — B's bootstrap trust root *is* the ceremony → each writes the
   other into the roster.
6. B initialises `notes`, pulls [[Loro]] state via [[Version Vector]] delta sync,
   materialises Markdown → "synced".
7. Rendezvous torn down — the daemon closes the ephemeral pairing endpoint and
   stops republishing; the DHT record itself cannot be deleted and persists in
   caches until TTL expiry ([[#12. Threat Model]] §J, F58); phrase consumed →
   later discovery uses durable [[NodeId]] via [[pkarr]], no phrase.

**Postconditions:** Both devices hold identical content + [[Group Key]]; either
edits offline and merges on reconnect.

**Failure modes:** wrong phrase → generic auth failure
([[#REQ-479 Failure-Message Indistinguishability]]); rendezvous absent/expired →
generic auth failure; symmetric NAT, no relay → reachability error distinct from
auth ([[#ADR-474 Relay as Optional Fallback Not Requirement]]); phrase reused →
generic auth failure ([[#REQ-478 Single-Use Phrase]]); third-party wrong-phrase
attempt → phrase consumed (first-exchange-consumes,
[[#REQ-478 Single-Use Phrase]]) → owner's `invite` reports failure and the owner
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

1. Watcher detects the change → checks the write's **base ambiguity**: has the
   daemon materialised any new export generation for this document since the
   last external event on it? (Content hashes recognise only *unchanged*
   saves; an edited buffer's base is undecidable from its bytes — F59.)
2. No intervening export AND no concurrent daemon edit → the base is
   unambiguous → diff against it and fold the delta into [[Loro]] →
   "imported".
3. Concurrent daemon edit, or **ambiguous base** (an intervening export means
   the buffer may derive from a superseded generation — F51/F59) → stage to
   `.zetl/sync/conflicts/` → surface.

**Postconditions:** External edits never lost; [[Loro]] causality intact;
conflicts surfaced not hidden.

**Failure modes:** binary/non-UTF-8 write → staged, never folded; rapid churn →
coalesced per debounce window; NFD-encoded write → folded, then re-materialised
in canonical NFC (on-disk bytes differ from what the editor wrote; the import
diff compares under the canonical form so the fold→materialise→watch cycle
terminates — [[#ADR-471 Guarded Import for External Markdown Edits]], F42);
external delete/rename → guarded like a write
([[#REQ-484 Guarded Import of External Markdown Edits]], F38); editor saves a
buffer opened against a superseded export after the daemon has materialised
newer ops → staged, never folded — diffing the stale buffer against the current
export would silently turn the daemon's already-materialised edits into
deletions (F51).

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
decrypt post-revocation sync (enforced on the wire by
[[#REQ-499 Group-Keyed Sync Frames]] — the rotated key actually seals every
sync frame; F46). Mechanism: the rotation is the Owner's [[MLS]] Remove
commit for the revoked device's leaf, which advances the group epoch
([[#ADR-482 MLS for Group Key Agreement and Membership Commits]]).

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
`last_export` **and** the write's base export is *unambiguous* (a delete folds
as a namespace-manifest tombstone and a rename as a manifest rename op —
[[#REQ-504 Replicated Vault Namespace Manifest]], F38/F64), and staging it to
the conflict area otherwise — FOR every external write, delete, or rename,
WITH neither side silently discarded. The predicate is defined over [[Loro]]
*logical* time (presence of an unmaterialised op), NOT a wall-clock debounce
window, so it is not a timing-controllable data-authority oracle (F16). The
base-ambiguity condition closes the stale-buffer hole (F51) and is decidable
(F59): content hashes can identify the base only of an *unchanged* save — an
edited buffer matches no retained export hash, so "which export did these
bytes derive from" is unknowable from content alone. The daemon therefore
folds an edited save only when exactly one base is *possible*: no export
generation has been materialised for that document since the last external
event on it (previous external write folded/staged, or the document's initial
export). Any **intervening export** (the editor may have opened E0 while the
daemon later wrote E1 — e.g. from remote ops, leaving no unmaterialised op at
save time) makes the base ambiguous, and folding a stale-based diff would
silently convert the daemon's materialised edits into deletions — such a
write is staged. This conservative rule over-stages the mixed
daemon+external-editor workflow (a false positive, never data loss); a
cooperative editor-supplied provenance token (an export-generation stamp) is
the refinement path, owned by DESIGN-047 `adr-external-edits` (F59).

**Trace:** [[#TEST-484a]], [[#TEST-484b]], [[#TEST-484c]], [[#TEST-484d]], [[#TEST-484e]], [[#CON-471 Loro Store and Materialisation]], [[#OBS-479 External-edit import]].

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
roots alone do not witness equal [[Loro]] state (F33). When the roots are equal
but the [[Version Vector]]s differ (ops pending materialisation, or extra
operations that cancel in the materialised bytes), the [[Merkle DAG]] cannot
localise the difference; the session localises by exchanging **per-document
[[Version Vector]]s** instead and exchanges op deltas for every document whose
vectors differ — the equal-root path is a fast path, never a way to skip the
op exchange the completion condition requires (F50). Reconciliation LOOPS
(F65): after every op exchange the session recompares roots and
[[Version Vector]]s and repeats localisation until both are equal — a single
DAG descent is not completion, because a root mismatch localised to document
X coexists with, and does not reveal, an equal-root/unequal-vector divergence
in document Y (byte-cancelling or unmaterialised ops invisible to the DAG);
the recompare catches Y on the next iteration.

**Trace:** [[#TEST-486a]], [[#TEST-486b]], [[#TEST-486c]], [[#TEST-486d]], [[#CON-473 Peer Session]], [[#OBS-480 Convergence witness]].

### REQ-487: Control-Plane Messages Recognised by the CBCL DPDA

The system SHALL recognise every control-plane message (daemon control verbs,
pairing/reconcile choreography, presence, signed-root announcement) as a
[[CBCL]] message parsed by the shared deterministic pushdown automaton
([[DCFL]], parser-equivalence) before any semantic action, FOR all control-plane
inputs at a trust boundary, WITH every *network* control-plane message carrying
a [[CBCL]] **message attestation** — NOT "R4": R4 is CBCL's *dialect*-integrity
invariant, verified once at dialect load, and authenticates no ordinary
message (F61) — an Ed25519 signature under CBCL's attestation discipline
(cbcl-core `attest`, SPEC-015/SPEC-017: the signed preimage is the
domain-tagged canonical encoding `(cbcl-attest-v2 <suite> <content-hash>
<performative> <from-key> (<to-key>*) <thread> [<caused-ref>])`, or the
attest-v3 typed-Merkle-root commitment; verification dispatches on the
explicit `SignatureDiscipline` marker via `verify_with_discipline`,
fail-closed on mismatch), explicitly verified on every message with **key
lookup by session phase**: the attestation's `from` key MUST equal the roster
[[NodeId]] for post-admission sessions, or the connection's
[[Pre-Admission Pairing Identity]] (the ephemeral pairing [[NodeId]] that
authenticated the [[QUIC]] channel, F45/F54) for pairing-ceremony messages —
whose *authority to pair* is conferred not by the signature but by [[SPAKE2]]
key confirmation over a transcript that binds both pairing NodeIds
([[#REQ-491 SPAKE2 Channel Authentication]]) — and additionally the
message's vault binding verified per
[[#REQ-503 Vault-Bound Peer Sessions]] (local control-socket messages
authenticate by the socket's filesystem permissions instead —
[[#CON-470 Daemon Control Channel]]) and the shipped dialects validated
(R1–R5) at load with any R4-`Invalid` dialect refused (dialects are release
artefacts of the binary — their integrity rides the release, and the
R4-`Unsigned`-accepted-with-warning install path is never exercised for
peer-supplied material because dialects are not installed from peers; F37/F61;
[[#ADR-479 CBCL as the Control-Plane Message Language]]).

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

The system SHALL verify a connecting peer's durable [[NodeId]] against the
roster of the session's *selected* vault (named by the [[#8.11 Vault Selector]]
— [[#REQ-503 Vault-Bound Peer Sessions]], F63) before parsing any vault frame,
FOR every peer session, WITH off-roster [[NodeId]]s rejected pre-frame (the
recognition-before-action ordering of
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
payload rejected (no bare-id binding — F12). The hash alone detects
*substitution*, not an exact *replay* — a captured control message with its
byte-identical payload still carries the expected hash — so the signed
reference additionally carries a **per-session monotonic reference sequence**
covered by the message's attestation signature (F61); the receiver tracks the last sequence
per session and rejects any reference whose sequence is ≤ it (F53). This
matters most on the [[SPAKE2]] path, where a replayed handshake payload must
fail before the decoder runs.

**Trace:** [[#TEST-494a]], [[#TEST-494b]], [[#TEST-494c]], [[#CON-473 Peer Session]], [[#OBS-478 Off-roster rejections]].

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

### REQ-497: DID-Bound Member Identity

The system SHALL identify each roster member by a [[did:crdt]] DID whose
verification methods enumerate that member's device ed25519 [[NodeId]]s, FOR
every roster entry, WITH a connecting [[NodeId]] admitted by
[[#REQ-492 Roster Gate Before Vault Frame]] only while it is a currently-valid
verification method of an on-roster DID (`[Provisional — DESIGN-047 task
adr-identity]`; [[#ADR-481 did:crdt as the Member Identity Layer]]).

**Trace:** [[#TEST-497a]], [[#TEST-497b]], [[#CON-477 Group Key Roster and Revocation]], [[#OBS-482 Identity verification]].

### REQ-498: DID Key Removal Triggers Key Rotation

The system SHALL treat removal of a device verification method from an
on-roster [[did:crdt]] document as a revocation event that rotates the
[[Group Key]] epoch per [[#REQ-481 Revocation by Key Rotation]], FOR every
accepted key-removal delta, WITH the removed [[NodeId]] unable to decrypt
post-rotation sync — composing the DID layer's key revocation with the
transport layer's epoch rotation so neither can silently lag the other
([[#18. Open Questions]] Q11). Mechanism
([[#ADR-482 MLS for Group Key Agreement and Membership Commits]]): the
rotation is the Owner's [[MLS]] **Remove commit** for the removed
device's leaf, issued by the Owner's daemon on accepting the key-removal
delta. This dissolves the former bespoke rotation-event design (F49's
`rotation_id` dedup and the concurrent-removal epoch-precedence rule):
commits are totally epoch-ordered, so a re-delivered commit targets a
past epoch and is discarded, and partitioned peers can never mint
divergent keys — they only ever apply the Owner's commit. Residual
(Q11/§O): a key-removal delta that converges while every Owner device is
offline leaves the rotation pending until an Owner device commits — the
removed key stays inside the group for that window.

**Trace:** [[#TEST-498a]], [[#TEST-498b]], [[#TEST-498c]], [[#CON-477 Group Key Roster and Revocation]], [[#OBS-477 Roster audit]], [[#OBS-482 Identity verification]].

### REQ-499: Group-Keyed Sync Frames

The system SHALL seal every data-plane sync payload
([[#8.2 Peer Sync Frame]]) with an [[AEAD]] under the current vault
[[Group Key]] epoch, FOR every sync frame exchanged between paired peers, WITH
the frame carrying its `key_epoch` and a frame sealed under a non-current
epoch rejected before the [[Loro]] decoder runs. Pairwise [[QUIC]] encryption
alone cannot deliver the post-revocation undecryptability of
[[#REQ-481 Revocation by Key Rotation]] and
[[#REQ-498 DID Key Removal Triggers Key Rotation]] — rotating a key that never
protects sync frames protects nothing; the group-key AEAD is what makes
[[#TEST-481c]] and [[#TEST-498b]] satisfiable (F46). The epoch data key is
distributed to members as an [[MLS]] application message in the current
epoch, so a removed leaf never receives the post-rotation key
([[#ADR-482 MLS for Group Key Agreement and Membership Commits]]).
Presence frames remain
control-plane [[CBCL]] over the roster-gated channel (session metadata, not
vault content).

**Trace:** [[#TEST-499a]], [[#TEST-499b]], [[#CON-473 Peer Session]], [[#OBS-478 Off-roster rejections]].

### REQ-500: Order-Independent DID Authorization

The system SHALL authorise each [[did:crdt]] delta as a deterministic function
of the delta set alone — the signing key judged against the DID document state
given by the delta's own causal context (its [[Hybrid Logical Clock|HLC]]
predecessors), never the receiver's current state — FOR every received DID
delta, WITH any two peers holding the same set of signed deltas deriving
identical DID and roster state regardless of delivery order (F48). A delta
signed by key K concurrent with K's removal is therefore judged identically
everywhere (accepted — K was valid in the delta's causal past — while the
removal still rotates the epoch per
[[#REQ-498 DID Key Removal Triggers Key Rotation]]); the residual — a
compromised key back-dating its causal context — joins the Q11 human
auth-core review ([[#18. Open Questions]] Q11). Scope
([[#ADR-482 MLS for Group Key Agreement and Membership Commits]]): this
order-independence governs [[did:crdt]] *deltas*; roster **membership
commits** are totally epoch-ordered per
[[#REQ-502 Replicated Signed Roster]] and are not merged under these
semantics.

**Trace:** [[#TEST-500a]], [[#TEST-500b]], [[#CON-477 Group Key Roster and Revocation]], [[#OBS-482 Identity verification]].

### REQ-501: Bounded Frame Recognition

The system SHALL enforce the declared fixed-width length prefix and hard
per-frame maximum on **every untrusted network frame — data-plane AND
control-plane** — before allocating buffers or reading the payload, FOR every
[[#8.2 Peer Sync Frame]], [[#8.6 SPAKE2 Frame]], and network
[[#8.1 Control Envelope]] frame (including pre-authentication pairing frames
of both planes: the pairing *control* envelope arrives before [[SPAKE2]] and
was previously unbounded — F62), WITH an over-limit length advertisement
rejected at the prefix and a partially received frame's connection state
reclaimed after a read deadline of `[Provisional: 10 s]` — so an attacker who
advertises a huge payload, or stalls after the prefix before any [[SPAKE2]]
or roster authentication exists, cannot cause unbounded buffering or
connection-state exhaustion (F52/F62).

**Trace:** [[#TEST-501a]], [[#TEST-501b]], [[#TEST-501c]], [[#CON-473 Peer Session]], [[#CON-474 Pairing Protocol]], [[#OBS-478 Off-roster rejections]].

### REQ-502: Replicated Signed Roster

The system SHALL replicate vault membership as the Owner's totally-ordered
sequence of signed [[MLS]] **membership commits** — an Add commit on
admission (issued inside the completed [[SPAKE2]] ceremony), a Remove
commit on device or member removal
([[#ADR-482 MLS for Group Key Agreement and Membership Commits]]) —
carried on a replicated **membership lane** of the vault's [[Loro]] store
and processed strictly in epoch order, FOR every roster mutation, WITH a
commit for a future epoch buffered until its predecessors arrive, a
commit at or below the local epoch discarded as already applied
(re-delivery idempotent), any two peers having processed the same commit
prefix deriving an identical roster (members, devices, roles,
`key_epoch` = MLS epoch), and a third peer accepting a member it never
paired with only by processing the Owner's Add commit for that member's
leaf, authorised per
[[#REQ-505 Role-Gated Membership Authority]] (F60/F66). This is what
makes third-peer admission coherent: after A pairs C, B applies the
Owner's Add commit from the lane on reconnect — the ceremony-only
genesis rule of [[#8.10 did:crdt Delta]] binds the *pairing devices*;
everyone else verifies the commit. Welcome messages never ride the lane
(they travel only inside the pairing ceremony —
[[#CON-474 Pairing Protocol]]); epoch data-key application messages do.
The local [[#8.7 Roster Schema]] TOML is a **projection cache** of MLS
group state × [[did:crdt]] documents, never the authority. Total epoch
order supersedes the former order-independent membership-event merge:
F60's causal-context authorisation and the removal-wins /
epoch-precedence conflict rules survive only for [[did:crdt]] deltas
under [[#REQ-500 Order-Independent DID Authorization]].

**Trace:** [[#TEST-502a]], [[#TEST-502b]], [[#TEST-502c]], [[#CON-477 Group Key Roster and Revocation]], [[#OBS-477 Roster audit]], [[#OBS-482 Identity verification]].

### REQ-503: Vault-Bound Peer Sessions

The system SHALL negotiate a versioned ALPN (`[Provisional: zetl/p2p/1]`) and
recognise a bounded **vault selector** ([[#8.11 Vault Selector]]) as the
first frame of every peer connection — naming the vault the session is for —
and SHALL bind that vault identifier into every subsequent frame's
authentication (the attestation preimage's thread/context for control
messages; the [[AEAD]] associated data for [[#8.2 Peer Sync Frame]]s), FOR
every inbound peer connection on a daemon serving one or more vaults, WITH
the roster check of [[#REQ-492 Roster Gate Before Vault Frame]] performed
against the *selected* vault's roster and a frame whose bound vault differs
from the session's rejected before interpretation (F63 — without a selector,
a multi-vault daemon cannot know which roster to consult, and a frame for
vault X could be replayed into a session for vault Y).

**Trace:** [[#TEST-503a]], [[#TEST-503b]], [[#CON-473 Peer Session]], [[#OBS-478 Off-roster rejections]], [[#OBS-481 Message recognition]].

### REQ-504: Replicated Vault Namespace Manifest

The system SHALL replicate the vault *namespace* — the mapping from stable
[[DocId]]s to vault-relative paths — as a CRDT **manifest document**
([[Loro]] map/movable-tree) with create, rename, and delete (tombstone) as
first-class manifest operations that merge conflict-free, FOR the whole
vault, WITH any two converged peers deriving an identical DocId → path
mapping, a rename preserving the document's identity and history (no longer
provisional delete + create), and path collisions resolved by a
deterministic rule — paths compared under the canonical form of
[[#REQ-473 Deterministic Materialisation]] (NFC) plus a declared
case-folding policy for case-insensitive filesystems, with the losing
document materialised under a deterministic disambiguated path rather than
silently overwriting (F64 — document *contents* were CRDTs but the namespace
was not, so create/delete/rename, case collisions, and Unicode-normalised
paths had no convergence rule, leaving
[[#REQ-474 Conflict-Free Offline Merge]] /
[[#REQ-485 Merkle Convergence Witness]] /
[[#REQ-486 Merkle Anti-Entropy Reconciliation]] unsatisfiable at vault
scope). Materialisation reads the manifest; the [[Merkle Vault Root]]'s
path-sorted file roots are computed over manifest paths. **DocId scheme
and collision rule (adr-namespace, 0.20.0):** a [[DocId]] is a **minted,
opaque, path-independent 128-bit id** (32 lowercase hex) — so a rename or
move is a value change on a stable key, preserving identity and history
(not delete+create). The manifest is a [[Loro]] map `DocId → path`.
**Collision rule:** when several DocIds resolve to the same folded path,
the lexicographically-smallest DocId keeps the path and the rest take a
deterministic disambiguated path (`stem (docid-prefix).ext`) — never a
silent overwrite. Path folding is `[Provisional: ASCII lower-case; full
NFC + Unicode case-fold deferred]`.

**Trace:** [[#TEST-504a]], [[#TEST-504b]], [[#TEST-504c]], [[#CON-471 Loro Store and Materialisation]], [[#CON-473 Peer Session]], [[#OBS-471 Materialisation]], [[#OBS-472 Sync convergence]].

### REQ-505: Role-Gated Membership Authority

The system SHALL accept a membership commit ([[#8.12 Membership Commit]])
only when its sender leaf's signature key resolves, through the roster
projection at the commit's epoch, to a device of the `owner`-role member
(sole-committer — [[#ADR-482 MLS for Group Key Agreement and Membership Commits]];
a member manages their *own* device set by self-scoped [[did:crdt]] delta,
which the Owner's daemon realises as the corresponding Remove commit per
[[#REQ-498 DID Key Removal Triggers Key Rotation]]),
FOR every received membership commit, WITH the roster `role` field taking
exactly the values `owner | member`, fixed at admission (no role-mutation
events in v1; vault genesis creates the group with the Owner's DID as
`owner` and the creating device as the first leaf, *its [[NodeId]] on the
roster* — F70: a genesis
that omits the Owner's own device leaves the
[[#REQ-492 Roster Gate Before Vault Frame]] gate refusing the Owner's
sessions in both directions, the composed-loop defect `../elephant-3000`
hit as its BUG-002 and caught only by a two-daemon end-to-end test), and
an unauthorised commit rejected as `unauthorized-author` before it mutates
roster state. Authority is judged from the commit's *sender-leaf signature
key* resolved through the roster projection — never from any
author-identity field the commit declares about itself: an identity string
is attacker-chosen, and authorising on it admits an impersonation that
key-resolution refuses (F67; `../elephant-3000` enforces its equivalent
steward-only rule by MLS leaf index rather than credential-string
comparison for exactly this reason, with a regression test for the
forged-credential case).
Rationale (F66): the Owner-only policy of [[#CON-474 Pairing Protocol]] C1
bound only the *authoring* CLI, while the acceptance rule of
[[#REQ-502 Replicated Signed Roster]] validated **any** on-roster author —
so a compromised non-owner daemon (threat §D, a strictly weaker adversary
than §N's malicious key-holder) could author admission events every honest
peer accepted. This gate makes non-delegatable admission enforceable *among
honest verifiers at the replication boundary*, not just at the command line.
Excluded from the gate: a member's [[did:crdt]] delta adding or removing a
verification method of their **own** DID
([[#ADR-481 did:crdt as the Member Identity Layer]]) — self-scoped device
management is member-delegated by design and is not membership authority.
Role checking binds honest verifiers only; the §N key-holder residual
(sealing the [[Group Key]] onward, forking the roster) is unchanged, and
*delegatable* admission — owner-granted admission rights, transitive trust
chains — is explicitly deferred to the web-of-trust successor of
[[#ADR-477 Single Per-Vault Group Key]], where [[did:crdt]] controller
proofs give delegation an authenticated substrate.

**Trace:** [[#TEST-505a]], [[#TEST-505b]], [[#TEST-505c]], [[#CON-477 Group Key Roster and Revocation]], [[#OBS-477 Roster audit]], [[#OBS-482 Identity verification]].

### REQ-506: Durable Epoch Rotation

The system SHALL record every [[Group Key]] rotation durably — the
signed [[MLS]] commit
([[#ADR-482 MLS for Group Key Agreement and Membership Commits]]) and,
for a removal, the pre-rotation epoch — BEFORE the
rotating daemon advances its own epoch, completing the publish and
re-seal exactly once on crash recovery, FOR every rotation of
[[#REQ-481 Revocation by Key Rotation]] and
[[#REQ-498 DID Key Removal Triggers Key Rotation]], WITH a daemon that
crashes between advancing its own epoch and publishing the rotation
never leaving surviving peers stranded behind the new epoch (unable to
decrypt [[#REQ-499 Group-Keyed Sync Frames]] payloads or process later
rotations), and recovery guarded by the recorded pre-rotation epoch so
an interrupted rotation is completed, never repeated (F68 — REQ-481/498
specified *what* rotates but no durability ordering, so a crash mid-
rotation could strand the vault). Pattern source: the transactional
outbox of `../elephant-3000` (its SPEC-004 REQ-306 /
`e2ee::recover_outbox`) — the commit is written and fsynced before the
local epoch merge; recovery republishes the exact recorded bytes and
completes the key rotation at most once, keyed on the recorded
pre-rotation generation.

**Trace:** [[#TEST-506a]], [[#TEST-506b]], [[#CON-477 Group Key Roster and Revocation]], [[#OBS-477 Roster audit]].

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
window — [[#REQ-484 Guarded Import of External Markdown Edits]], F16) **and**
no **intervening export** makes the write's base ambiguous; else stage.
Tracking only unmaterialised ops is insufficient: a stale editor buffer saved
after the daemon has materialised newer ops passes the op predicate yet would
fold the daemon's edits away as deletions (F51). And base identification by
content hash is insufficient too: hashes recognise only *unchanged* saves —
an edited buffer matches no retained hash, so its base is undecidable from
content (F59). The decidable conservative predicate: stage whenever the
daemon has materialised any new export generation for the document since the
last external event on it; a cooperative editor provenance token is the
refinement (DESIGN-047 `adr-external-edits`).

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
[[QUIC]] channel. Short rendezvous TTL; the record carries only an endpoint
hint for an **ephemeral pairing endpoint** — a single-use keypair minted per
ceremony, never the owner's durable [[NodeId]]: an [[iroh]] endpoint address
embeds its endpoint identifier, and because every 4–5-digit rendezvous key is
enumerable, a record naming the durable identity would tell a scanner *which
durable identity* is pairing, not merely that a meeting room is busy (F54).
Durable [[NodeId]]s are disclosed only inside the [[SPAKE2]]-confirmed channel
(HP1 step 5).
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
[[#NFR-475 Pairing Secret Entropy Floor]]. (−) Because the rendezvous keypair
is derived from public `num`, **anyone holds the rendezvous *signing* key**:
an attacker can publish validly-signed competing records, pre-squat the whole
~110,000-key routing namespace, and win pkarr's timestamp-ordered,
cache-TTL'd write races — a fresh phrase does not escape a namespace-wide
squatter (F58). [[SPAKE2]] still denies auth to a squatter without
`word-word`, but availability and (combined with a leaked phrase, threat §B)
MitM positioning are residuals ([[#12. Threat Model]] §J). "Teardown" of a
rendezvous record means closing the endpoint and ceasing republication —
pkarr has no deletion primitive.

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

**Decision:** (A). Membership = roster; revocation = rotate + re-seal
(mechanism: [[#ADR-482 MLS for Group Key Agreement and Membership Commits]]). (B)
deferred (heavier key management; YAGNI until demand).

**Consequences:** (−) Any member reads the whole vault; revocation is coarse;
**admission authority is unenforceable against a malicious member** — any
key-holder can seal the [[Group Key]] onward or fork the roster, so
Owner-controlled membership binds honest daemons only
([[#12. Threat Model]] §N, F34) — though among honest verifiers it is now
enforced at the replication boundary, not just the CLI
([[#REQ-505 Role-Gated Membership Authority]], F66); the write/admission
dual moves to the web-of-trust successor along with (B). **Admission
authority is non-delegatable in v1** (F66): owner-granted admission rights
and transitive trust chains are deferred to that same successor — they are
precisely the transitive-authority problem it owns, and no v1 user goal
traces to them ([[users/vault-owner/user|Owner]] profile: "controls
membership"; the roster `role` field is the recorded extension point).

### ADR-478: Merkle DAG as Convergence Witness and Reconciliation Index

**Status:** Proposed (adr-merkle-sync resolved; Q3 CRDT granularity decided)

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
committed unmaterialised ops — F33); root mismatch →
descend the DAG to localise differing docs, then ship [[Loro]] op deltas for only
those; equal roots with **unequal** [[Version Vector]]s → the DAG cannot
localise (the difference is invisible in materialised bytes), so localisation
falls back to per-document [[Version Vector]] exchange (F50); after each
exchange, recompare and repeat until roots and vectors are both equal — the
mixed case (root mismatch in one document masking an invisible
equal-root divergence in another) terminates only through the loop (F65). Converged peers MUST match roots
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

**Q3 resolution — CRDT granularity (adr-merkle-sync, 0.19.0):** the vault is
**one [[Loro]] document per note**, keyed by the stable [[DocId]] of the
namespace manifest ([[#REQ-504 Replicated Vault Namespace Manifest]]), **plus
one vault-level manifest document** — *not* a single container tree for the
whole vault.

- *Why doc-per-note.* The reconciliation unit is already "documents"
  throughout [[#REQ-486 Merkle Anti-Entropy Reconciliation]] and §8; the
  manifest already makes a note a first-class `DocId → path` entity. Per-note
  documents give the [[Merkle DAG]] a natural localisation target (descend to
  the changed notes, ship only their [[Loro]] op deltas), independent per-note
  presence and [[Version Vector]]s, and per-note snapshot/oplog compaction
  (Q4) — matching zetl's file-per-note model. This resolves F43 (the
  localisation unit is doc-per-note; REQ-486/§8 need no rewording).
- *Why not one doc per vault.* `../elephant-3000` uses a single Loro document
  per theory because a theory's unit of state *is* one corpus (a list of
  signed entries); zetl's unit is many independently-edited notes, so its
  model does not transfer. A single vault-wide document would couple every
  note's oplog, defeat per-note localisation, and make snapshot cost
  whole-vault.
- *Trade-off.* Many small documents mean more per-document [[Version Vector]]
  bookkeeping across a large vault; the vault-level `vault_root` comparison
  (this ADR) is the coarse filter that keeps whole-vault "are we synced?" a
  single hash check, descending to per-note vectors only for the notes the DAG
  flags — so the bookkeeping is paid only on divergence, not every sync.

This unblocks IMPL-047 T3 (the [[Loro]] store): the store is a keyed
collection of per-note [[Loro]] documents plus the manifest document, each
persisted as snapshot + oplog under `.zetl/loro/<DocId>`.

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
core-preservation, **R4 Ed25519 *dialect* integrity over a canonical
encoding** (plus a separate per-message **attestation discipline**,
cbcl-core `attest`, SPEC-015/017 — F61), **R5
causal-protocol + shape contracts**.

**Options:** (A) bespoke CBOR + schema per message (status quo; many small
hand-rolled recognisers = parser-differential surface); (B) [[CBCL]] dialects for
the **control plane**, native length-prefixed binary for the **data plane**
([[Loro]]/[[SPAKE2]] blobs); (C) [[CBCL]] for *everything* including blobs
(rejected — DCFL S-expression text is wrong for bulk binary payloads).

**Decision:** (B). Define `zetl-pair` and `zetl-sync` [[CBCL]] dialects over the 8
core performatives (`tell ask reply hello bye ok error cancel`, immutable by R3).
One DPDA recognises all control messages (parser-equivalence → no
parser-differential attacks). Per-message authentication is CBCL's
**attestation discipline** (cbcl-core `attest`, SPEC-015 attest-v2 / SPEC-017
attest-v3; `verify_with_discipline`, fail-closed) on ed25519 — [[NodeId]] is
already ed25519, unifying with [[#8.8 Signed Vault Root]]; R4 authenticates
*dialects at load*, never ordinary messages (F61). R5 encodes the pairing and reconcile choreographies
as verified causal-protocol contracts ([[#REQ-488 Choreographies as Verified R5 Causal-Protocol Contracts]]),
verified at build time over the shipped dialect (not at peer-driven install).
[[Loro]] updates and [[SPAKE2]] bytes stay opaque on the data plane, referenced by
id from control messages. The `zetl-pair`/`zetl-sync` dialects are **shipped
with the CLI binary** (a fixed, release-versioned set) — NOT installed from peers
at runtime; protocol evolution happens by releasing a new `zetl`, not by DHT
gossip (deferred — [[#18. Open Questions]] Q10). zetl therefore uses [[CBCL]]'s
recognition + attestation + R4 + R5 properties but not its runtime
self-extension.

**Consequences:** (+) Principle 14 satisfied with a *formal* warrant
(Lean `dcfl_preserved`/`decidable_preserved`), not a hand-rolled validator;
parser-differential attacks excluded by construction (strengthens
[[#12. Threat Model]] §F); the attestation discipline supplies message auth
(F61 — R4 is dialect-load integrity only); R5 supplies a verified
handshake state machine; shipping a fixed dialect set with the binary keeps the
recognised language static per release (no runtime-extension attack surface — F17
dissolved); `no_std` suits the mobile daemon; reuses an existing dependency
(Simplicity-Ladder rung 4) and *deletes* the bespoke CBOR validators.
(−) [[DCFL]] exceeds the regular power minimal framing needs — recorded here as
the Principle 14 §6 justification (decidable + parser-equivalent, far below the
undecidable danger zone). (−) [[CBCL]]'s canonical encoding is S-expression text,
so control messages are textual (acceptable at control-plane volume; bulk binary
stays on the data plane — see [[#18. Open Questions]] Q9). (−) Per-message
attestation signing on the presence path costs one ed25519 sign + verify per
frame (~50 µs each on commodity hardware — well inside
[[#NFR-477 Remote Edit Propagation Latency]]'s budget; recorded so the cost is
a decision, not an accident — F37). (−) Adds a Tier-1
dependency whose attestation/R4 path is auth-core and must be in the
human-review package. (−) Divergent downstream evidence (0.15.0):
`../elephant-3000` evaluated [[CBCL]] for its *local* control plane and
chose loopback HTTP + bearer token instead (its ADR-106), arguing R1–R5
earn their weight only at the inter-agent trust boundary, not same-user
loopback.

**adr-control-proto resolution — two control planes (0.20.0):** [[CBCL]]
governs the **network** control plane (peer ↔ peer, [[#CON-473 Peer Session]]),
where untrusted peers make R1–R5 (parser-equivalence, attestation,
causal-protocol contracts) load-bearing. The **local** control plane
(daemon ↔ same-user CLI clients over the [[#CON-470 Daemon Control Channel]]
socket) uses **length-prefixed JSON**, not CBCL — the boundary is same-user
loopback, where R1–R5 buy nothing, and JSON keeps the client surface
trivial. This confirms `../elephant-3000`'s ADR-106 argument rather than
inheriting the single-recogniser claim silently, and matches the T2
implementation (`src/daemon/`). Q9 (a compact CBCL canonical encoding) is
therefore scoped to the **network** plane only. Consequence: CON-470's
recogniser is the local JSON decoder; CON-473's is the shared CBCL DPDA.

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

**Decision:** (B). `zetl collab {invite,join,peers,revoke}` (extending the existing
group, reusing SPEC-036's `join`); `zetl daemon {start,stop,status}` manages the
`zetld` process (as `dockerd`/`docker`). All inherit the global flags, the
TTY-only secret convention for the [[Pairing Phrase]], positional ids, and the
`not-yet-implemented` non-zero exit for unlanded verbs. Naming (0.17.0,
stakeholder direction): the owner-side verb is **`invite`**, not `pair` —
"pair" names the whole ceremony, leaving ambiguous which side runs it,
while `invite`/`join` are complementary speech acts naming each side's
role (`../elephant-3000` reached the same naming independently:
`theory invite` / `theory join`, `inviter_side`/`joiner_side`). "Pairing"
remains the *protocol* term ([[Pairing Phrase]], the [[SPAKE2]] pairing
ceremony); only the CLI verb changes.

**Consequences:** (+) Zero new CLI idioms; discoverable via the same `--help`
shape; the phrase reuses the audited TTY-only path; consistent machine output via
the global formatter. (−) `zetl collab` now spans server-auth *and* P2P verbs —
acceptable, both are "collaboration"; the encrustation guard
([[PROTO-001]] §Specification Status Lifecycle) is satisfied because pairing is
on `collab`'s existing responsibility, not a broadening of it.

### ADR-481: did:crdt as the Member Identity Layer

**`[Provisional — DESIGN-047 task adr-identity]` · No-go area: authentication
core, human review required.** · **Status:** Proposed

**Context:** The spec so far has only *device* identity — a flat roster of raw
ed25519 [[NodeId]]s. There is no *member* (person) identity: revoking a lost
phone is indistinguishable from expelling a person, the
[[users/solo-multi-device/user|solo user]]'s device fleet has no unifying
subject, and F34/threat §N showed membership authority needs a cryptographic
substrate that a flat key list cannot provide. `../did-crdt` ([[did:crdt]]) is a
sibling Rust crate: a [[W3C DID Core]] 1.0-compliant DID method whose documents
are signed CRDTs (G-Set/OR-Set/LWW-Map/Max-Register composition, Hybrid Logical
Clock ordering) — all DID operations are monotonic and coordination-free (CALM),
so identity updates merge offline exactly like vault content. Its core is pure
(zero I/O, WASM-compatible, property-tested for commutativity/associativity/
idempotence), it signs with ed25519 (the [[NodeId]] primitive), and its
feature-gated sync already rides [[iroh]].

**Options:** (A) status quo — flat NodeId roster, no member identity;
(B) **[[did:crdt]] DIDs as member identity** — one DID per member, device
[[NodeId]]s as verification methods, DID deltas exchanged over the existing
encrypted peer channel; (C) another DID method — `did:key` (single-key: no
rotation, no multi-device), `did:web` (reintroduces a server), ledger-based
methods (fees, confirmation delay — the coordination did-crdt exists to avoid).

**Decision:** (B). One [[did:crdt]] document per member; the roster maps
DID → {devices, role, added_at, key_epoch} and a device [[NodeId]] is admitted
only as a currently-valid verification method of an on-roster DID
([[#REQ-497 DID-Bound Member Identity]]). `role` takes exactly
`owner | member`, is fixed at admission (vault genesis mints the `owner`;
no role-mutation events in v1), and authorises exactly one thing:
membership commits per [[#REQ-505 Role-Gated Membership Authority]] —
delegatable admission is deferred with
[[#ADR-477 Single Per-Vault Group Key]]'s successor (F66). Pairing a new device adds a
verification method (a signed delta authored by an existing device key);
losing a device removes one, which MUST also rotate the [[Group Key]] epoch
([[#REQ-498 DID Key Removal Triggers Key Rotation]]). DID deltas are exchanged
**only over the roster-gated encrypted peer channel**
([[#REQ-482 Roster-Gated Encrypted Transport]]) — NOT public iroh-gossip and
NOT the public DHT (threat §P). Sits at Simplicity-Ladder rung 4: an existing
sibling dependency (like `../cbcl-rs`), pure core reused as-is; the secp256k1
feature stays off (unused surface).

**Consequences:** (+) Member ≠ device at last: device revocation and member
expulsion become distinct, correctly-scoped operations. (+) Identity updates
are offline-first CRDT merges — the same convergence model as the vault itself,
no coordination service. (+) DID controller proofs give the *authenticated
substrate* the web-of-trust successor needs (partial path out of threat §N —
though a current key-holder forking the [[Group Key]] remains that successor's
problem). (+) `resolve()` projects a standard W3C DID Document for external
interop. (−) A second Tier-1 auth-core dependency at version 0.1.0 — the
human-review package grows (its ADR-002 key-compromise recovery and ADR-003
Sybil analysis join the Q7/Q11 review). (−) Two revocation mechanisms must
compose, never race ([[#REQ-498 DID Key Removal Triggers Key Rotation]], Q11).
(−) Two logical-time systems coexist (did-crdt HLC for identity, [[Loro]]
logical time for content) — acceptable because the layers never share a clock,
recorded so the boundary is a decision. (−) A DID document enumerates a
member's whole device fleet — it MUST NOT be published to the public DHT
(threat §P). (−) `zetl collab revoke` grows a member-level form
(`revoke <did>` vs device-level `revoke <nodeid>`) — CLI detail deferred to
DESIGN-047 `adr-identity`.

### ADR-482: MLS for Group Key Agreement and Membership Commits

**`[Provisional — DESIGN-047 task adr-group-key]` · No-go area: group
cryptography, human review required. Adopted on stakeholder direction
(2026-07-18), following the prior-art review of `../elephant-3000`
(0.15.0).** · **Status:** Proposed

**Context:** The Q7 group-key package asked the human crypto review to
bless a bespoke sealed-sender scheme plus hand-rolled epoch, rotation, and
re-seal rules — the most dangerous novel construction in the spec.
`../elephant-3000` (its SPEC-004) implements the same requirement shape
with [[MLS]] (RFC 9420, openmls 0.8) under a sole-committer rule, and
0.14.0's [[#REQ-505 Role-Gated Membership Authority]] already fixed
membership authority as Owner-only *for policy reasons* — so MLS's main
P2P weakness (multi-committer fork resolution) costs zetl nothing.

**Options:** (A) bespoke sealed-sender group key (status quo — novel
Tier-1 crypto, all of Q7 open); (B) **[[MLS]] via openmls, Owner as sole
committer** — one group per vault, leaf per *device*, credential = the
member's [[did:crdt]] DID; (C) MLS with multi-committer + fork
resolution (dMLS) — more machinery than the v1 authority model permits.

**Decision:** (B). One MLS group per vault (`group_id` = vault id,
ciphersuite `[Provisional: MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519]`,
leaf keys HKDF-derived per vault from the device identity seed — never
the [[NodeId]] key itself, no cross-protocol reuse). One **leaf per
device**; the leaf credential carries the member's DID, and admission
verifies the leaf key is a current verification method of that DID
([[#REQ-497 DID-Bound Member Identity]]). Only devices of the
`owner`-role member commit (Add, Remove — [[#REQ-505 Role-Gated Membership Authority]]);
commit authorisation resolves the sender **leaf**'s signature key through
the roster projection to the owner DID — never credential-string equality
(F67). Commits ride a replicated **membership lane** of the vault's
[[Loro]] store, processed strictly in epoch order
([[#REQ-502 Replicated Signed Roster]]); Welcome messages travel only
inside the pairing ceremony ([[#CON-474 Pairing Protocol]]). Data-plane
composition: MLS distributes the current **epoch data key** as an
application message; sync frames stay AEAD-sealed under it with
`key_epoch` (= MLS epoch) and vault id in the associated data —
[[#REQ-499 Group-Keyed Sync Frames]] unchanged. No key-history keybook:
content at rest is plaintext on member disks
([[#ADR-476 Encryption at Rest is Opt-In]]) and a joiner receives history
over a session sealed under the *current* epoch, so only the current
generation is ever distributed (simpler than elephant's full-history
keybook — a deliberate divergence, recorded).

**Consequences:** (+) The human review audits a composition
(SPAKE2 ∘ MLS ∘ did:crdt) instead of a novel construction; Simplicity
Ladder rung 4 (openmls + the elephant/hark durable-provider pattern) on
the highest-risk component. (+) Total epoch order collapses the
F48/F49/F60 order-independent *membership* machinery: removal-wins,
rotation-event dedup, and concurrent-removal epoch precedence dissolve
into "process the Owner's commits in order, buffer ahead, discard
behind"; [[#REQ-500 Order-Independent DID Authorization]] rescopes to
[[did:crdt]] deltas only. (+) MLS gives post-compromise security via
commits — the bespoke design had none. (+) Removing a device's leaf *is*
the rotation, making [[#REQ-498 DID Key Removal Triggers Key Rotation]] a
mechanism rather than a cross-layer consistency rule. (−) openmls joins
the Tier-1 auth-core review package (in place of, not in addition to,
the bespoke scheme). (−) Membership changes and rotations require an
Owner device online: a member's key-removal delta converging while every
Owner device is offline leaves rotation pending until one commits — a
new stale-window residual joining threat §O and Q11. (−) Total Owner
device loss freezes membership (solo profile: equivalent to vault-backup
loss; team profile: documented ceiling).
// SIMPLIFY: sole-committer MLS; ceiling: delegatable admission /
// Owner-loss recovery in the web-of-trust successor; upgrade path:
// multi-committer MLS with fork resolution (dMLS / openmls helpers)
// (trace: this ADR, [[#ADR-477 Single Per-Vault Group Key]]).

---

## 7. Contracts

> [[PROTO-001]] §Contract Specification. External-input contracts carry a
> **Grammar / Recogniser** field per Principle 14; pre/post-conditions are
> per-clause for requirement-localised repair.

### CON-470: Daemon Control Channel

**Interface:** Local IPC (`[Provisional: $XDG_RUNTIME_DIR/zetld.sock`; named pipe
on Windows]`) — versioned request/response + subscription. Control verbs:
`attach`, `status`, `apply_ops`, `subscribe`, `invite`, `join`, `peers`, `revoke`.
**CLI mapping** ([[#ADR-480 CLI Surface Follows Existing zetl Conventions]]):
`zetl daemon {start,stop,status}` manages the `zetld` process (as `zetl serve`
runs the web server); `zetl collab {invite,join,peers,revoke}` front the
corresponding control verbs.

**Grammar / Recogniser:** **length-prefixed JSON** into closed `serde`
request/response types (`deny_unknown_fields`), fully recognised before
dispatch — NOT [[CBCL]]. Per the adr-control-proto resolution
([[#ADR-479 CBCL as the Control-Plane Message Language]]), the *local*
control plane is a same-user loopback socket where CBCL's R1–R5 buy
nothing; CBCL governs the *network* peer session ([[#CON-473 Peer Session]])
instead. Trust boundary: local process (socket `0600`). Over-length frames
are rejected at the length prefix before allocation.

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
`import_external(markdown, export_history, edit_state) -> ImportOutcome`
(guarded; `export_history` carries the current `last_export`, the retained
per-document export-generation hashes — which recognise *unchanged* saves —
and the per-document intervening-export marker since the last external event,
which decides base ambiguity for *edited* saves — F51/F59).

**Grammar / Recogniser:** `import_external` input is UTF-8 Markdown; grammar
[[#8.4 Materialised Markdown]]; recogniser = the existing zetl Markdown parser;
non-UTF-8/binary fails closed → staged.

**Pre-conditions:** C1 (REQ-472/473) `materialise` total over valid [[Loro]] docs,
no I/O; C2 (REQ-484) `import_external` receives live edit-state and the
export-generation history (base ambiguity is decided from recorded state,
never guessed from content — F59).

**Post-conditions:** C3 (REQ-473) `materialise` referentially transparent →
identical bytes; C4 (REQ-472) restart reloads canonical state with causal
history; C5 (REQ-484) `import_external` returns `Folded | Staged(path)`, discards
neither side.

**Error model:** `materialise` infallible on valid docs; `import_external`
surfaces `Staged(path)` instead of erroring.

**Implements:** [[#REQ-472 Loro Canonical Store]], [[#REQ-473 Deterministic Materialisation]], [[#REQ-484 Guarded Import of External Markdown Edits]], [[#REQ-504 Replicated Vault Namespace Manifest]] (materialisation reads the manifest for DocId → path — F64).
**Verified by:** [[#TEST-472a]], [[#TEST-472c]], [[#TEST-472d]], [[#TEST-473a]], [[#TEST-473c]], [[#TEST-484a]], [[#TEST-484b]], [[#TEST-484c]], [[#TEST-484d]], [[#TEST-484e]], [[#TEST-504a]], [[#TEST-504b]], [[#TEST-504c]].

### CON-473: Peer Session (iroh)

**Interface:** [[QUIC]] connection keyed by [[NodeId]] under versioned ALPN
`[Provisional: zetl/p2p/1]`; first frame = the [[#8.11 Vault Selector]]
naming the vault (F63, [[#REQ-503 Vault-Bound Peer Sessions]]); then streams
`reconcile` ([[Merkle Vault Root]] compare → DAG descent, looping — F65),
`sync` ([[Loro]] update exchange via [[Version Vector]] for the docs
`reconcile` localised), and `presence` ([[Ephemeral Store]]).

**Grammar / Recogniser:** **control plane** — `reconcile`/`presence`/signed-root
are [[CBCL]] `zetl-sync` messages recognised by the shared DPDA
([[#8.3 Presence Frame]], [[#8.8 Signed Vault Root]]); **data plane** — `sync`
carries length-prefixed, length-bounded (F52) [[Loro]] updates AEAD-sealed
under the current [[Group Key]] epoch (F46, [[#8.2 Peer Sync Frame]])
referenced by hash+sequence from a [[CBCL]] message and recognised by the
[[Loro]] import decoder (fuzzed). **The primary untrusted trust boundary.**

**Pre-conditions:** C1 (REQ-475) both endpoints discovered via [[pkarr]];
C2 (REQ-482/503) the [[#8.11 Vault Selector]] is recognised first and the
peer [[NodeId]] verified against the *selected* vault's roster before any
vault frame (F63); C3 (REQ-483) each frame fully recognised before apply.

**Post-conditions:** C4 (REQ-482) content frames exchanged only after the roster
check; C5 (REQ-486) `reconcile` runs before `sync`; op exchange is skipped only
on equal roots **and** equal [[Version Vector]]s — equal-root/unequal-vector
sessions localise via per-document [[Version Vector]] exchange (F50), and
reconciliation loops (recompare after every exchange) until both are equal
(F65); C6
(REQ-474) `sync` converges
to identical [[Loro]] state; C7 (REQ-485)
converged peers report equal [[Merkle Vault Root]] or raise an integrity alarm;
C8 (REQ-499) every sync payload is sealed under the current [[Group Key]]
epoch, stale-epoch frames rejected pre-decode; C9 (REQ-501) frame length
maxima and read deadlines enforced before allocation — on control envelopes
too (F62); C10 (REQ-503) the session's vault id is bound into every frame's
attestation context / AEAD associated data, cross-vault frames rejected
before interpretation (F63); C11 (REQ-504) the namespace manifest converges
with content — converged peers derive one DocId → path mapping (F64).

**Error model:** `not-on-roster` (reject pre-frame); `malformed-frame` (drop +
log); `root-mismatch-after-converge` (integrity alarm); `reachability-failed`
(distinct from auth).

**Implements:** [[#REQ-474 Conflict-Free Offline Merge]], [[#REQ-475 Serverless Peer Discovery]], [[#REQ-482 Roster-Gated Encrypted Transport]], [[#REQ-483 Full Recognition at Trust Boundaries]], [[#REQ-485 Merkle Convergence Witness]], [[#REQ-486 Merkle Anti-Entropy Reconciliation]], [[#REQ-487 Control-Plane Messages Recognised by the CBCL DPDA]], [[#REQ-488 Choreographies as Verified R5 Causal-Protocol Contracts]], [[#REQ-492 Roster Gate Before Vault Frame]], [[#REQ-493 Signed-Root Epoch Binding]], [[#REQ-494 Control-to-Data Binding]], [[#REQ-495 Signed-Root Freshness]], [[#REQ-499 Group-Keyed Sync Frames]], [[#REQ-501 Bounded Frame Recognition]], [[#REQ-503 Vault-Bound Peer Sessions]], [[#REQ-504 Replicated Vault Namespace Manifest]].
**Verified by:** [[#TEST-474a]], [[#TEST-474c]], [[#TEST-475a]], [[#TEST-475b]], [[#TEST-482a]], [[#TEST-483a]], [[#TEST-483c]], [[#TEST-485a]], [[#TEST-485c]], [[#TEST-486a]], [[#TEST-486b]], [[#TEST-486c]], [[#TEST-486d]], [[#TEST-492a]], [[#TEST-492b]], [[#TEST-493a]], [[#TEST-493b]], [[#TEST-494a]], [[#TEST-494b]], [[#TEST-494c]], [[#TEST-495a]], [[#TEST-495b]], [[#TEST-499a]], [[#TEST-499b]], [[#TEST-501a]], [[#TEST-501b]], [[#TEST-501c]], [[#TEST-503a]], [[#TEST-503b]], [[#TEST-504a]], [[#TEST-504b]], [[#TEST-504c]].

### CON-474: Pairing Protocol (`zetl collab invite` / `zetl collab join`)

**Interface:** `zetl collab invite`: mint phrase, publish rendezvous [[pkarr]]
record, await peer, run [[SPAKE2]], seal [[Group Key]]. `zetl collab join`:
TTY-prompt phrase, resolve rendezvous, connect, run [[SPAKE2]], receive sealed
key. Both honour the global `--format`/`--json` and `--vault` flags and the
standard non-zero-exit-on-error convention ([[#ADR-480 CLI Surface Follows Existing zetl Conventions]]).
`--vault` on `invite` selects which vault the invitation admits into (a
multi-vault daemon needs it — [[#REQ-503 Vault-Bound Peer Sessions]]);
on `join` it is an OPTIONAL **pin** — the joiner learns the vault id
inside the ceremony, and a supplied `--vault` makes the join fail closed
if the ceremony offers a different vault (the `expected_theory` guard of
`../elephant-3000`'s `joiner_side`).

**Grammar / Recogniser:** phrase grammar [[#8.5 Pairing Phrase]] (ABNF, regular,
TTY-only); handshake choreography is the [[CBCL]] `zetl-pair` dialect (DPDA;
the step order is an R5 causal-protocol contract — [[#REQ-488 Choreographies as Verified R5 Causal-Protocol Contracts]]);
[[SPAKE2]] bytes ride as an opaque payload [[#8.6 SPAKE2 Frame]] recognised by the
`cap::pair` decoder (fuzzed).

**Pre-conditions:** C1 (REQ-476) `invite` caller owns the vault; C2 (REQ-477) phrase
entered only via TTY; C3 (REQ-483) every [[SPAKE2]]/rendezvous frame recognised
before use.

**Post-conditions:** C4 (REQ-476) on success both rosters updated, key shared,
rendezvous torn down; C5 (REQ-478) phrase consumed; C6 (REQ-479) all failures
return one opaque `auth-failed`; C7 (REQ-480) membership granted only on
[[SPAKE2]] success; C8 (REQ-496) the inbound attempt budget is enforced —
exhaustion aborts the pairing and tears down the rendezvous record; C9
(REQ-487) pairing control messages attestation-verify (F61) against the ceremony's ephemeral
[[Pre-Admission Pairing Identity|pre-admission NodeIds]], with authority
conferred by [[SPAKE2]] key confirmation over the transcript (F45); C10
(REQ-501) length maxima and read deadlines are enforced before any
authentication exists on **both** pre-auth planes — [[SPAKE2]] frames AND the
pairing control envelope (F52/F62); C11 (REQ-476) a ceremony that fails at
*any* step leaves the joiner with no partial vault state — no roster entry,
no key material, no store directory (F71; the `../elephant-3000` join
ceremony materialises local state only after the sealed key handover for
exactly this reason, and clears a half-joined shell before re-joining).

**Error model:** single opaque `auth-failed`; distinct `reachability-failed`,
`malformed-input`.

**Implements:** [[#REQ-476 DHT-Bootstrapped SPAKE2 Pairing]], [[#REQ-477 Phrase OOB-Only Non-Leak]], [[#REQ-478 Single-Use Phrase]], [[#REQ-479 Failure-Message Indistinguishability]], [[#REQ-480 Group-Key Admission Gate]], [[#REQ-488 Choreographies as Verified R5 Causal-Protocol Contracts]], [[#REQ-491 SPAKE2 Channel Authentication]], [[#REQ-496 Pairing Attempt Rate Limit]], [[#REQ-501 Bounded Frame Recognition]].
**Verified by:** [[#TEST-476a]], [[#TEST-476b]], [[#TEST-476c]], [[#TEST-476d]], [[#TEST-477a]], [[#TEST-477b]], [[#TEST-477c]], [[#TEST-478a]], [[#TEST-478b]], [[#TEST-479a]], [[#TEST-479c]], [[#TEST-480a]], [[#TEST-480b]], [[#TEST-491a]], [[#TEST-496a]], [[#TEST-496b]], [[#TEST-501a]], [[#TEST-501b]].

### CON-477: Group Key Roster and Revocation

**Interface:** the per-vault [[MLS]] group and its replicated
**membership lane** of [[#REQ-502 Replicated Signed Roster]] (the Owner's
epoch-ordered Add/Remove commits + epoch data-key application messages,
[[#8.12 Membership Commit]], carried by the vault's [[Loro]] store over
the roster-gated channel — F60,
[[#ADR-482 MLS for Group Key Agreement and Membership Commits]]), locally
projected into a roster cache (`[Provisional: .zetl/peers.toml` mode
`0600`]`, never the authority) mapping [[did:crdt]] DID → {devices:
[[[NodeId]]], role: `owner | member` (immutable in v1 —
[[#REQ-505 Role-Gated Membership Authority]]), added_at, key_epoch}
([[#ADR-481 did:crdt as the Member Identity Layer]], [[#REQ-497 DID-Bound Member Identity]]);
`process_membership_commit(msg) -> Applied | Buffered | Rejected`
validates and applies one lane element in epoch order;
`zetl collab revoke <nodeid>` (device) rotates epoch + schedules re-seal
(positional id, mirroring `zetl collab share revoke <jti>`; the member-level
`revoke <did>` form is `[Provisional — DESIGN-047 task adr-identity]`);
`zetl collab peers` lists the roster and never prints key material (mirroring
`zetl collab share list`); `apply_did_delta(delta) -> Accepted | Rejected`
folds a signed [[did:crdt]] delta into a member's document.

**Grammar / Recogniser:** roster cache file is a declared [[TOML]] schema
([[#8.7 Roster Schema]]); recogniser = a `serde` TOML decoder + schema validation
(it is trusted local state, but recognised before use per Principle 14). DID
deltas arriving from peers are recognised per [[#8.10 did:crdt Delta]] before
`apply_did_delta` runs; membership commits per [[#8.12 Membership Commit]]
(CBCL envelope, then openmls validation) before
`process_membership_commit` applies anything (F60).

**Pre-conditions:** C1 (REQ-481) caller is the [[users/vault-owner/user|Owner]];
C5 (REQ-497/REQ-500) a delta's Linked-Data Proof verifies against a key valid
in the delta's *causal context* before it mutates the document
(order-independent — F48); a **genesis delta**, self-signed by the key it
introduces, is accepted only inside a completed [[SPAKE2]] pairing ceremony
that binds the new DID to the joiner's durable [[NodeId]] (F47); C9
(REQ-502/REQ-505) an Add commit for a DID already on the roster is
rejected `already-member` — a duplicate member identity would make roster
key-resolution ambiguous and lets a second entry carry the Owner's DID,
the credential-confusion `../elephant-3000` guards against at admission
(F69).

**Post-conditions:** C2 (REQ-481) revoked [[NodeId]] removed; new epoch sealed to
survivors within [[#NFR-474 Revocation Propagation]]; C3 (REQ-481) revoked peer
rejected by [[#CON-473 Peer Session]]; C4 (REQ-480) entries added only via
completed [[SPAKE2]]; C6 (REQ-498) an accepted key-removal delta is
realised as the Owner's Remove commit for the removed leaf before the
removal is considered applied — commits are epoch-ordered, so
re-delivery targets a past epoch and is discarded, and no receiver ever
mints its own key (F49 dissolved by ADR-482; Owner-offline deferral is
the recorded Q11/§O residual); C7 (REQ-502) any two peers having
processed the same commit prefix derive an identical roster — a third
peer admits a member it never paired with only by processing the Owner's
Add commit; ahead-of-epoch commits buffer, at-or-behind commits are
discarded (F60); C8 (REQ-505)
a membership commit is applied only when its sender leaf resolves to an
`owner`-role device (sole committer; a member's own device management
flows as did:crdt deltas), role fixed at admission — an unauthorised
commit is rejected
`unauthorized-author`, never merged (F66); C10 (REQ-506) a rotation is
durably recorded before the rotating daemon advances its own epoch, and
recovery completes an interrupted rotation exactly once (F68).

**Error model:** `not-on-roster`; `last-member` (cannot revoke the sole member);
`invalid-did-delta` (proof/recognition failure — dropped, logged);
`unauthorized-author` (membership commit whose sender leaf does not
resolve to an owner-role device — dropped, logged; F66); `already-member`
(Add commit for a
DID already on the roster — dropped, logged; F69); `epoch-gap` (commit
ahead of the local epoch — buffered, not an error; surfaced if the gap
persists past `[Provisional: 60 s]`).

**Implements:** [[#REQ-480 Group-Key Admission Gate]], [[#REQ-481 Revocation by Key Rotation]], [[#REQ-497 DID-Bound Member Identity]], [[#REQ-498 DID Key Removal Triggers Key Rotation]], [[#REQ-500 Order-Independent DID Authorization]], [[#REQ-502 Replicated Signed Roster]], [[#REQ-505 Role-Gated Membership Authority]], [[#REQ-506 Durable Epoch Rotation]].
**Verified by:** [[#TEST-480a]], [[#TEST-480b]], [[#TEST-481a]], [[#TEST-481b]], [[#TEST-481c]], [[#TEST-497a]], [[#TEST-497b]], [[#TEST-498a]], [[#TEST-498b]], [[#TEST-498c]], [[#TEST-500a]], [[#TEST-500b]], [[#TEST-502a]], [[#TEST-502b]], [[#TEST-502c]], [[#TEST-502d]], [[#TEST-505a]], [[#TEST-505b]], [[#TEST-505c]], [[#TEST-506a]], [[#TEST-506b]].

---

## 8. Input Grammars (LangSec)

> [[PROTO-001]] Principle 14. Each external/boundary input has a declared grammar
> at the lowest sufficient grammatical power, recognised in full before any
> semantic action ([[#REQ-483 Full Recognition at Trust Boundaries]]). All
> structured wire formats are length-prefixed binary or a typed schema — never
> string concatenation. Every length prefix is a **fixed-width u32
> (little-endian)** and every frame type declares a **hard maximum length**,
> enforced before any allocation or payload read, with a read deadline bounding
> how long a partially received frame may hold connection state
> ([[#REQ-501 Bounded Frame Recognition]], F52). `[Provisional — DESIGN-047
> task input-grammars]` for exact octet layouts.

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
`len₃₂ ‖ cbcl_text` — a [[CBCL]] `(verb …)` message in a `zetl-*` dialect.
`len₃₂` is a fixed-width u32 with hard maximum `[Provisional: 64 KiB]`,
enforced before allocation, with the [[#REQ-501 Bounded Frame Recognition]]
read deadline — network control envelopes (the pairing choreography included)
arrive **before any [[SPAKE2]] or roster authentication exists**, so they
need the same pre-auth bounding as the data plane (F62). A message that
references a data-plane payload MUST carry the reference as a **recognised**
`(ref <payload-id> <content-hash> <ref-seq>)` clause of the dialect grammar —
so the control→data binding ([[#REQ-494 Control-to-Data Binding]], F12) is
part of full DPDA recognition, not a post-parse check; `ref-seq` is the
per-session monotonic sequence that rejects byte-identical replays the hash
cannot (F53). Power: [[DCFL]] (payload) over regular framing. Recogniser:
shared [[CBCL]] DPDA + R1–R5 + per-message attestation verify (F61).
Boundary: **two boundaries** — the local control socket (authenticated by
filesystem permissions) AND **network, untrusted** ([[QUIC]] control streams
carrying `zetl-pair`/`zetl-sync` messages; the earlier "local process" label
was wrong — F62).

### 8.2 Peer Sync Frame
`len₃₂ ‖ key_epoch ‖ nonce ‖ aead_ciphertext` (data plane): the [[Loro]] update
is sealed with an [[AEAD]] (`[Provisional: XChaCha20-Poly1305]`) under the
vault [[Group Key]] of `key_epoch`, with the session's **vault identifier**
([[#8.11 Vault Selector]], F63) plus the referencing control message's
`(ref …)` clause as associated data — the [[Group Key]] protects the sync
content itself, not just the pairwise [[QUIC]] hop
([[#REQ-499 Group-Keyed Sync Frames]], F46). `len₃₂` is a fixed-width u32 with
hard maximum `[Provisional: 16 MiB]`, enforced before allocation (F52,
[[#REQ-501 Bounded Frame Recognition]]). Power: regular framing; plaintext
payload = [[Loro]] format. Recogniser: epoch check + AEAD open, then the
[[Loro]] crate import (fuzzed, [[#TEST-fuzz-loro]]). Boundary: **network,
untrusted**.

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
`len₃₂ ‖ side ‖ spake_msg` (data plane). `len₃₂` is a fixed-width u32 with hard
maximum `[Provisional: 1 KiB]`, enforced before allocation and **before any
[[SPAKE2]] or roster authentication exists** — this frame arrives
unauthenticated, so the bound plus the read deadline of
[[#REQ-501 Bounded Frame Recognition]] is the only thing standing between a
hostile advertiser and connection-state exhaustion (F52). Power: regular
framing. Recogniser: `cap::pair` decoder (fuzzed, [[#TEST-fuzz-spake]]).
Boundary: network, untrusted.

### 8.7 Roster Schema
[[TOML]] table-of-peers. Power: context-free (TOML). Recogniser: `serde` TOML +
schema check. Boundary: local file.

### 8.8 Signed Vault Root
[[CBCL]] `zetl-sync` message, attestation-signed (F61) `{nodeid, key_epoch, root_seq,
vault_root}` where `root_seq` is a per-signer monotonic counter (freshness — F29).
Power: [[DCFL]]. Recogniser: shared [[CBCL]] DPDA + attestation ed25519 verify vs roster
[[NodeId]], **rejecting any `key_epoch` ≠ the verifier's current epoch** (F9,
[[#REQ-493 Signed-Root Epoch Binding]]) **and any `root_seq` ≤ the last accepted
from that signer** (defeats same-epoch replay — F29, threat §K). Boundary:
network, untrusted.

### 8.9 pkarr Rendezvous Record
The DHT record resolved during discovery (HP1) and reconnect. Payload: an
endpoint hint (and, for durable peers, optionally an [[#8.8 Signed Vault Root]]).
A *rendezvous* record (published at an enumerable phrase-derived key) carries
only the **ephemeral pairing endpoint** of [[#ADR-473 Phrase-Derived DHT Rendezvous for SPAKE2]]
— never a durable [[NodeId]] (F54); durable-endpoint records are published at
the peer's own durable pubkey, which is not phrase-enumerable.
Power: regular framing + schema. Recogniser: generated decoder; **a resolved
record is an unauthenticated *hint* only — it confers no trust until [[SPAKE2]]
(pairing) or roster-NodeId verification (reconnect) succeeds** (F14). Boundary:
**network, untrusted** — at a phrase-derived rendezvous key this is stronger
than "anyone can publish": the keypair derives from public `num`, so anyone
holds the *signing* key and a "valid signature" on a rendezvous record proves
nothing (F58, threat §J). Durable-pubkey records are signed by a key only the
peer holds; their signature is meaningful but the endpoint they name is still
a hint (F14).

### 8.10 did:crdt Delta
A signed [[did:crdt]] document delta (verification-method add/remove, service
update) received from a roster peer over the encrypted channel — never from
the public DHT or open gossip (threat §P). Power: context-free (a declared
JSON schema over the did-crdt delta type). Recogniser: the `did-crdt`
`core::validate` path — schema recognition, then Linked-Data-Proof ed25519
verification against a key that is a valid verification method **in the DID
document state given by the delta's own causal context** (its HLC
predecessors), never the receiver's current state — authorization is a
deterministic function of the delta set, independent of delivery order
([[#REQ-500 Order-Independent DID Authorization]], F48) — then CRDT merge
(fuzzed, [[#TEST-fuzz-did]]). Two bootstrap exceptions, each valid **only
inside a completed [[SPAKE2]] pairing ceremony** (F47): a **genesis delta** is
self-certifying — signed by the very key it introduces, with the resulting DID
and the joiner's durable [[NodeId]] bound to the ceremony transcript
([[#REQ-497 DID-Bound Member Identity]]); and a fresh device with no local DID
or roster state accepts the roster + DID documents transferred inside the
ceremony as its bootstrap trust root (HP1 step 5). A **third-peer** genesis
(a member the receiver never paired with) is accepted only as carried by
the Owner's Add commit ([[#8.12 Membership Commit]], F60). Boundary: **network,
untrusted** (a roster peer may still be malicious — §N).

### 8.11 Vault Selector
The first frame of every peer connection after the versioned ALPN
(`[Provisional: zetl/p2p/1]`): a [[CBCL]] `zetl-sync` message naming the
target vault by identifier (`[Provisional: BLAKE3 of the vault genesis]`) —
bounded by the [[#8.1 Control Envelope]] framing, recognised **before** the
roster gate of [[#REQ-492 Roster Gate Before Vault Frame]] so the daemon
knows *which* roster to consult ([[#REQ-503 Vault-Bound Peer Sessions]],
F63). The vault id then binds every subsequent frame (attestation context /
AEAD associated data). Power: [[DCFL]] over regular framing. Recogniser:
shared [[CBCL]] DPDA. Boundary: network, untrusted (pre-roster).

### 8.12 Membership Commit
A signed [[MLS]] **membership commit** or **epoch data-key application
message** on the membership lane ([[#REQ-502 Replicated Signed Roster]],
[[#ADR-482 MLS for Group Key Agreement and Membership Commits]]): a
[[CBCL]] `zetl-sync` lane element carrying `{vault_id, epoch,
TLS-serialized MLSMessage}`. Recogniser: shared [[CBCL]] DPDA for the
envelope, THEN openmls full validation (TLS deserialisation, group /
epoch / leaf-signature checks; epoch ahead → buffer, at-or-behind →
discard), THEN commit authorisation: the sender **leaf**'s signature key
must resolve, through the roster projection, to a device of the
`owner`-role DID ([[#REQ-505 Role-Gated Membership Authority]], F66/F67 —
never credential-string equality); data-key application messages are
accepted only from an Owner leaf. Welcome messages never ride this lane
(pairing ceremony only — [[#CON-474 Pairing Protocol]]). Boundary:
network, untrusted (a roster peer may be malicious — §N; against a
key-holding adversary the sole-committer gate binds honest verifiers
only, F34).

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
| Member identity | 4 — existing sibling dependency `../did-crdt` ([[did:crdt]]) | shared `p2p::identity` over the pure `did-crdt` core (merge/validate/resolve); devices stay [[iroh]] [[NodeId]]s — no new key primitive ([[#ADR-481 did:crdt as the Member Identity Layer]]). |
| CLI verbs (pair/join/peers/revoke, daemon) | 6 — new verbs, but *on existing groups* | add to the existing `zetl collab` `clap` group + a `zetl daemon` group paralleling `zetl serve`; no new CLI idiom (Principle 15: options on the artefact with the right responsibility) ([[#ADR-480 CLI Surface Follows Existing zetl Conventions]]). |
| Wire framing (data plane) | 2/4 — std length-prefix + [[Loro]] codec | no bespoke serialisation (Principle 14 bans string-concat formats). |
| Namespace manifest | 4 — existing dependency ([[Loro]] map/movable tree) | `crdt::manifest` beside the content docs; the namespace converges by the same engine as content — no bespoke rename protocol ([[#REQ-504 Replicated Vault Namespace Manifest]], F64). |
| Roster replication | 4/5 — compose [[did:crdt]] deltas + [[CBCL]] messages | `p2p::identity::events`; membership commits ride the [[Loro]] lane and reuse the attestation machinery — no new consensus layer ([[#REQ-502 Replicated Signed Roster]], F60). |
| [[Group Key]] agreement + membership commits | 4 — existing dependency (openmls; provider pattern from `../elephant-3000`/hark) | shared `p2p::mls`; Owner-committed RFC 9420 group replaces the bespoke sealed-sender design ([[#ADR-482 MLS for Group Key Agreement and Membership Commits]]). |

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
| **TEST-476d** | neg-output | integration | ceremony failed at any post-SPAKE2 step → joiner retains no partial vault state (no roster entry, key material, or store); re-pairing succeeds (F71) | [[#REQ-476 DHT-Bootstrapped SPAKE2 Pairing]] |
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
| **TEST-484e** | neg-output | example | stale-base save (editor buffer from a superseded export, daemon ops since materialised) MUST be staged, never folded as deletions (F51) | [[#REQ-484 Guarded Import of External Markdown Edits]] |
| **TEST-485a** | positive | property | converged peers → equal [[Merkle Vault Root]] | [[#REQ-485 Merkle Convergence Witness]] |
| **TEST-485c** | neg-output | property | root mismatch under reported convergence → alarm, not silent | [[#REQ-485 Merkle Convergence Witness]] |
| **TEST-486a** | positive | example | equal roots **and equal [[Version Vector]]s** → session completes with zero op exchange (F50) | [[#REQ-486 Merkle Anti-Entropy Reconciliation]] |
| **TEST-486b** | neg-input | example | mismatch → DAG descent localises only differing docs | [[#REQ-486 Merkle Anti-Entropy Reconciliation]] |
| **TEST-486c** | neg-output | property | equal roots + unequal [[Version Vector]]s MUST NOT skip op exchange — localised via per-document vectors incl. ops that cancel in materialised bytes (F50) | [[#REQ-486 Merkle Anti-Entropy Reconciliation]] |
| **TEST-486d** | neg-output | example | mixed case: root mismatch in doc X **plus** byte-cancelling missing ops in doc Y → session MUST loop (recompare after exchanging X) and also ship Y's deltas before completing (F65) | [[#REQ-486 Merkle Anti-Entropy Reconciliation]] |
| **TEST-487a** | positive | example | valid [[CBCL]] control message accepted by the DPDA + attestation verify | [[#REQ-487 Control-Plane Messages Recognised by the CBCL DPDA]] |
| **TEST-487b** | neg-input | fuzz+example | non-conformant / attestation-invalid / wrong-phase-key message rejected, no action (F61) | [[#REQ-487 Control-Plane Messages Recognised by the CBCL DPDA]] |
| **TEST-488a** | positive | example | in-order handshake satisfies the R5 causal-protocol contract | [[#REQ-488 Choreographies as Verified R5 Causal-Protocol Contracts]] |
| **TEST-488b** | neg-input | property | out-of-order / undefined-step message rejected by the R5 contract | [[#REQ-488 Choreographies as Verified R5 Causal-Protocol Contracts]] |
| **TEST-489a** | positive | example | `zetl collab invite --json` emits valid JSON; verbs live under `collab`/`daemon` | [[#REQ-489 P2P CLI Follows Existing zetl Conventions]] |
| **TEST-489b** | neg-input | example | error path exits non-zero; unlanded verb → `not-yet-implemented` | [[#REQ-489 P2P CLI Follows Existing zetl Conventions]] |
| **TEST-490a** | positive | example | daemon survives client exit; committed state intact on reattach | [[#REQ-490 Daemon Survives Client Disconnection]] |
| **TEST-491a** | positive | example(e2e) | matching `word-word` → SPAKE2 success → shared key | [[#REQ-491 SPAKE2 Channel Authentication]] |
| **TEST-492a** | positive | integration | roster [[NodeId]] admitted, vault frames exchanged | [[#REQ-492 Roster Gate Before Vault Frame]] |
| **TEST-492b** | neg-input | integration | off-roster [[NodeId]] rejected before any frame parsed | [[#REQ-492 Roster Gate Before Vault Frame]] |
| **TEST-493a** | positive | example | signed root at current epoch from roster signer accepted | [[#REQ-493 Signed-Root Epoch Binding]] |
| **TEST-493b** | neg-input | example | stale-epoch root from since-revoked signer rejected (no witness) | [[#REQ-493 Signed-Root Epoch Binding]] |
| **TEST-494a** | positive | property | control message + matching-hash payload + fresh `ref-seq` → interpreted | [[#REQ-494 Control-to-Data Binding]] |
| **TEST-494b** | neg-input | property | substituted payload (hash mismatch) → rejected | [[#REQ-494 Control-to-Data Binding]] |
| **TEST-494c** | neg-input | example | **byte-identical replay** (hash matches, `ref-seq` ≤ last accepted) → rejected before the decoder runs (F53) | [[#REQ-494 Control-to-Data Binding]] |
| **TEST-495a** | positive | example | signed root with `root_seq` > last accepted → accepted | [[#REQ-495 Signed-Root Freshness]] |
| **TEST-495b** | neg-input | example | replayed root with `root_seq` ≤ last accepted → rejected | [[#REQ-495 Signed-Root Freshness]] |
| **TEST-496a** | positive | example | attempts within budget reach [[SPAKE2]] | [[#REQ-496 Pairing Attempt Rate Limit]] |
| **TEST-496b** | neg-input | example | excess attempts dropped pre-handshake; exhaustion aborts pairing + tears down rendezvous | [[#REQ-496 Pairing Attempt Rate Limit]] |
| **TEST-497a** | positive | integration | paired device's [[NodeId]] is a verification method of its member's DID; session admitted | [[#REQ-497 DID-Bound Member Identity]] |
| **TEST-497b** | neg-input | integration | [[NodeId]] absent from every on-roster DID → rejected pre-frame | [[#REQ-497 DID-Bound Member Identity]] |
| **TEST-498a** | positive | example | accepted key-removal delta → [[Group Key]] epoch rotates ≤ [[#NFR-474 Revocation Propagation]] | [[#REQ-498 DID Key Removal Triggers Key Rotation]] |
| **TEST-498b** | neg-output | example | removed [[NodeId]] MUST NOT decrypt post-rotation frames | [[#REQ-498 DID Key Removal Triggers Key Rotation]] |
| **TEST-498c** | neg-output | property | re-delivered Remove commit (at-or-below local epoch) discarded — the epoch advances exactly once; no receiver ever mints its own key (F49 via ADR-482 epoch order) | [[#REQ-498 DID Key Removal Triggers Key Rotation]] |
| **TEST-499a** | positive | integration | sync payload AEAD-sealed under current epoch → opened, decoded | [[#REQ-499 Group-Keyed Sync Frames]] |
| **TEST-499b** | neg-input | example | frame under a stale/foreign `key_epoch` rejected before the [[Loro]] decoder runs (F46) | [[#REQ-499 Group-Keyed Sync Frames]] |
| **TEST-500a** | positive | property | any permutation of the same signed delta set → identical DID + roster state (F48) | [[#REQ-500 Order-Independent DID Authorization]] |
| **TEST-500b** | neg-output | property | delta signed by K concurrent with K's removal MUST NOT be judged differently by delivery order | [[#REQ-500 Order-Independent DID Authorization]] |
| **TEST-501a** | neg-input | example | over-limit length advertisement rejected at the prefix, no allocation (F52) | [[#REQ-501 Bounded Frame Recognition]] |
| **TEST-501b** | neg-input | integration | stall-after-prefix (pre-auth) → connection state reclaimed at the read deadline | [[#REQ-501 Bounded Frame Recognition]] |
| **TEST-501c** | neg-input | example | over-limit **control envelope** (pre-SPAKE2 pairing choreography) rejected at the prefix, no allocation (F62) | [[#REQ-501 Bounded Frame Recognition]] |
| **TEST-502a** | positive | integration | three-peer offline admission: A (Owner) pairs C offline from B → B applies the Owner's Add commit from the lane on reconnect and admits C (F60) | [[#REQ-502 Replicated Signed Roster]] |
| **TEST-502b** | neg-input | example | a member/leaf appearing without the Owner's Add commit on the lane → rejected | [[#REQ-502 Replicated Signed Roster]] |
| **TEST-502c** | neg-output | property | commits delivered in any arrival order → identical roster (ahead buffered, at-or-behind discarded); an epoch gap is never skipped (F60 via ADR-482) | [[#REQ-502 Replicated Signed Roster]] |
| **TEST-505a** | positive | example | Add commit authored by an `owner`-role leaf → accepted (F66) | [[#REQ-505 Role-Gated Membership Authority]] |
| **TEST-505b** | neg-input | example | Add commit authored by a `member`-role leaf (on-roster, MLS-valid — incl. a forged owner-DID credential on a non-owner leaf, F67) → rejected `unauthorized-author`, roster unchanged (F66) | [[#REQ-505 Role-Gated Membership Authority]] |
| **TEST-505c** | neg-input | example | Remove commit authored by a `member`-role leaf (even for its own DID's device) → rejected; the same device removal expressed as a self-scoped [[did:crdt]] delta is accepted and realised by the Owner's Remove commit (REQ-498) | [[#REQ-505 Role-Gated Membership Authority]] |
| **TEST-502d** | neg-input | example | Add commit for a DID already on the roster → rejected `already-member`, roster unchanged (F69) | [[#REQ-502 Replicated Signed Roster]] |
| **TEST-506a** | positive | integration | rotating daemon killed between its own epoch advance and the publish → recovery republishes the recorded rotation; survivors reach the new epoch (F68) | [[#REQ-506 Durable Epoch Rotation]] |
| **TEST-506b** | neg-output | example | recovery of an interrupted removal MUST NOT rotate a second time (pre-rotation-epoch guard); re-delivery after recovery is idempotent | [[#REQ-506 Durable Epoch Rotation]] |
| **TEST-503a** | positive | integration | multi-vault daemon: vault selector names vault X → roster check runs against X's roster, session syncs X (F63) | [[#REQ-503 Vault-Bound Peer Sessions]] |
| **TEST-503b** | neg-input | example | frame bound to vault X replayed into a vault-Y session → rejected before interpretation (AD/attestation mismatch) | [[#REQ-503 Vault-Bound Peer Sessions]] |
| **TEST-504a** | positive | property(convergence) | concurrent create/rename/delete on disconnected peers → identical DocId → path mapping; rename preserves history (F64) | [[#REQ-504 Replicated Vault Namespace Manifest]] |
| **TEST-504b** | neg-input | example | colliding paths (case-fold / NFC-equal) created concurrently → deterministic disambiguation, no silent overwrite | [[#REQ-504 Replicated Vault Namespace Manifest]] |
| **TEST-504c** | neg-output | property | concurrent rename vs edit MUST lose neither — edits land in the renamed document, never a resurrected tombstone | [[#REQ-504 Replicated Vault Namespace Manifest]] |

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
| **TEST-fuzz-did** | fuzz | [[did:crdt]] delta recogniser + proof verifier vs hostile deltas (incl. forged causal contexts) | [[#REQ-483 Full Recognition at Trust Boundaries]], [[#REQ-497 DID-Bound Member Identity]], [[#REQ-500 Order-Independent DID Authorization]] |
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
| **OBS-477** Roster audit | log | roster add/revoke + key-epoch rotation, [[NodeId]] + ts; rejected membership commits with cause (`unauthorized-author` — F66); buffered epoch-gap surfaced past its deadline | [[#REQ-480 Group-Key Admission Gate]], [[#REQ-481 Revocation by Key Rotation]], [[#REQ-505 Role-Gated Membership Authority]], [[#REQ-506 Durable Epoch Rotation]] |
| **OBS-478** Off-roster rejections | metric | `zetl_offroster_rejections_total`, `zetl_malformed_frames_total`, `zetl_frame_reject_total{cause: stale-epoch\|over-limit\|read-deadline\|replayed-ref}` | [[#REQ-482 Roster-Gated Encrypted Transport]], [[#REQ-483 Full Recognition at Trust Boundaries]], [[#REQ-494 Control-to-Data Binding]], [[#REQ-499 Group-Keyed Sync Frames]], [[#REQ-501 Bounded Frame Recognition]] |
| **OBS-479** External-edit import | log | import outcome (`folded`/`staged`) | [[#REQ-484 Guarded Import of External Markdown Edits]] |
| **OBS-480** Convergence witness | metric | `zetl_root_mismatch_total` (integrity alarm), `zetl_reconcile_rounds`, `zetl_reconcile_skipped_total` (equal-root) | [[#REQ-485 Merkle Convergence Witness]], [[#REQ-486 Merkle Anti-Entropy Reconciliation]] |
| **OBS-481** Message recognition | metric+log | `zetl_cbcl_reject_total{cause}` (parse / R1–R5 at load / attest-sig / wrong-phase-key / vault-binding) on incoming control messages against the shipped dialect | [[#REQ-487 Control-Plane Messages Recognised by the CBCL DPDA]], [[#REQ-488 Choreographies as Verified R5 Causal-Protocol Contracts]], [[#REQ-503 Vault-Bound Peer Sessions]] |
| **OBS-482** Identity verification | metric+log | `zetl_did_delta_reject_total{cause}` (schema / proof / merge); audit line per verification-method add/remove with DID + [[NodeId]] + ts | [[#REQ-497 DID-Bound Member Identity]], [[#REQ-498 DID Key Removal Triggers Key Rotation]] |

> [[#OBS-475 Pairing failure cause]]'s `cause` label is operator-channel only and
> MUST NOT be exposed via any unauthenticated metrics endpoint, lest it become an
> oracle defeating [[#REQ-479 Failure-Message Indistinguishability]].

---

## 12. Threat Model (Summary)

> Full model: DESIGN-047 task `threat-model` → `research/SPEC-047-threat-model.md`.
> **§H is the decisive new risk** and gates the human crypto review.

- **A. Passive network observer** — recovers neither phrase (REQ-477) nor
  [[SPAKE2]] key (protocol); content transit-encrypted (REQ-482).
- **B. OOB-channel compromise (attacker learns the phrase)** — **complete
  pairing compromise, not a bounded guess** (F57). [[SPAKE2]]'s
  online-single-guess bound protects only against an attacker who does *not*
  know the password; matching passwords derive matching keys, so an attacker
  who learns the phrase can complete a fully *authenticated* exchange — racing
  the legitimate joiner to redeem the phrase first
  ([[#REQ-478 Single-Use Phrase]] merely creates that race), or completing
  separate authenticated exchanges with each victim (classic MitM) if it can
  interpose on the rendezvous (§J gives it the squat primitive to do so).
  *Mitigation:* the protocol therefore ASSUMES a **confidential, authentic OOB
  channel** for phrase delivery — this assumption is now explicit and joins
  the Tier-1 review package; the candidate in-protocol backstop is the
  [[NodeId]]-fingerprint / SAS confirmation of §E
  ([[#18. Open Questions]] Q2, upgraded from "phishing nicety" to the sole
  recovery against OOB compromise). Detection is best-effort: the owner *may*
  notice the legitimate join failing (race variant), but the MitM variant
  completes both sides and is invisible at pairing time. *Documented
  residual pending Q2.*
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
  Because the record itself is fetchable by anyone who enumerates the routing
  space, it carries only an **ephemeral pairing endpoint** — a scanner must
  not learn *which durable identity* is pairing from the record (F54;
  [[#8.9 pkarr Rendezvous Record]]).
  *Residual:* `num`-enumeration leaks the metadata "someone is pairing now,"
  pins a small routing space (§J), and the ephemeral record still exposes
  routing addresses (IPs / relay), which may correlate with a known peer —
  weighed by the DESIGN-047 `threat-model` task. No longer the decisive
  blocker, but the split itself remains in the Tier-1 crypto-review scope.
- **I. Forged / stale convergence witness** — a peer claims a
  [[Merkle Vault Root]] it has not reached, or replays an old signed root. *Mitigation:* the
  [[#8.8 Signed Vault Root]] frame is ed25519-signed and verified against the
  roster [[NodeId]] before trust (REQ-483); a root is only *trusted as converged*
  after the [[Loro]] state that produces it is actually applied — the witness
  cross-checks merge, it never substitutes for it (REQ-485).
- **J. Rendezvous squat / overwrite — attacker holds full write authority**
  (F58) — the rendezvous keypair is `HKDF(num)` and `num` is public, so
  **anyone can derive the rendezvous *private* key and sign valid competing
  [[pkarr]] records** — this is not mere unauthenticated-hint pollution but
  equal publishing authority with the owner. pkarr orders competing signed
  packets by *controller-authored timestamps* with best-effort consistency
  and resolver caches (default minimum TTL ~5 min), so the owner cannot
  reliably win a write race. The routing space is small
  (`4*5DIGIT` ≈ 110,000 keys), so an attacker can **pre-squat the entire
  namespace continuously** — choosing a fresh phrase is NOT a recovery from a
  namespace-wide squatter, only from a per-rendezvous one. *Mitigation:*
  [[SPAKE2]] still denies auth (the squatter lacks `word-word`; no secret
  leaks) — but see §B: against an attacker who *also* knows the phrase, the
  squat is the MitM interposition point. **Teardown semantics:** "rendezvous
  record torn down" (HP1 step 7) MUST be read as *the daemon closes the
  ephemeral endpoint and stops republishing* — pkarr offers no deletion;
  cached/replayed records persist until TTL expiry and are answered by a
  closed endpoint. *Residual:* namespace-wide availability DoS on pairing;
  MitM positioning when combined with §B — both in the Tier-1 crypto-review
  scope ([[#18. Open Questions]] Q1, Q2).
- **K. Cross-epoch signed-root replay by a revoked member** — revocation rotates
  the [[Group Key]] epoch, but a member's durable ed25519 [[NodeId]] does not
  change, so a since-revoked member can re-present an old attestation-signed
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
  drops excess pre-handshake; the burn is visible (the owner's `invite` fails)
  and the owner re-pairs. *Residual:* availability nuisance under active
  attack — accepted for v1 (pairing is rare and interactive).
- **N. Malicious-member admission / roster fork** — any current member holds
  the [[Group Key]] and can seal it onward to a third party, run `invite` on its
  own replica, or fork the roster; the Owner-only pre-conditions
  ([[#CON-474 Pairing Protocol]] C1, [[#CON-477 Group Key Roster and Revocation]] C1)
  bind *honest* daemons only — membership authority is local policy, not
  cryptography (F34). *Mitigation (partial — honest verifiers only):*
  [[#REQ-505 Role-Gated Membership Authority]] moves the Owner-only rule
  into the membership-commit acceptance check, so honest peers reject
  membership commits from non-`owner` leaves — closing the strictly weaker
  §D-grade hole where a *compromised* non-owner daemon could author
  admissions every honest peer accepted (F66). Against the key-holding §N
  adversary itself — sealing the key onward, forking the roster — none at
  this layer: the write/admission
  dual of §L and of [[#ADR-477 Single Per-Vault Group Key]]'s read-coarseness;
  the [[users/vault-owner/user|Owner]] profile's "controls membership" goal is
  scoped accordingly. *Residual:* documented; the web-of-trust successor's
  concern (delegatable admission deferred there too — F66).
- **O. Revocation propagation window** — a survivor offline at revoke time
  ([[#NFR-474 Revocation Propagation]] binds *online* survivors only) still
  holds the old epoch and the revoked [[NodeId]] on its local roster, and will
  sync **new** content with the revoked peer until it contacts a resealed
  member (F35) — distinct from §D's past-content retention. *Mitigation:*
  epoch precedence / anti-rollback — a peer that learns a higher `key_epoch`
  MUST refuse older epochs; specified in the Q7 group-key package
  ([[#18. Open Questions]] Q7). *Residual:* unbounded for indefinitely-offline
  survivors.
- **P. DID document as a device-fleet map** — a member's [[did:crdt]] document
  enumerates *all* of that member's device keys; published openly (public DHT,
  open iroh-gossip) it hands an observer the member's device graph and its
  churn over time — a richer metadata leak than §G's single-record liveness.
  *Mitigation:* DID deltas and documents travel **only** over the roster-gated
  encrypted channel ([[#ADR-481 did:crdt as the Member Identity Layer]],
  [[#8.10 did:crdt Delta]]); nothing DID-shaped reaches the public DHT.
  *Residual:* roster members see each other's fleets — inherent to
  membership; the DESIGN-047 `threat-model` task weighs per-member visibility
  scoping for the web-of-trust successor.

---

## 13. Purity Boundary Map

### Pure Core (no I/O, no shared state, deterministic)

- `p2p::pair::phrase::generate(rng) -> Phrase` — `num-word-word` generator.
- `p2p::pair::rendezvous::derive(phrase) -> RendezvousKeypair` — [[HKDF]];
  **no-go area, human-reviewed.**
- `p2p::pair::spake::{start, finish}` — wraps the [[SPEC-034]] [[SPAKE2]] driver.
- `p2p::pair::groupkey::{seal, open, rotate}` — [[Group Key]] envelope ops.
- `crdt::loro::materialise(doc) -> Markdown` — deterministic export (REQ-473).
- `crdt::loro::import::plan(markdown, export_history, edit_state) -> ImportOutcome`
  — guarded-import decision incl. base-export identification (REQ-484, F51).
- `crdt::merkle::vault_root(asts) -> ContentHash` + `merkle::diff(root_a, root_b)
  -> Vec<DocId>` — reuse `src/merkle.rs` ([[SPEC-006]]); convergence witness +
  DAG descent (REQ-485, REQ-486). Pure.
- `p2p::proto::recognise(text) -> Result<Message>` + R1–R5 validators — reuse
  [[CBCL]] `cbcl-core`/`cbcl-parser` (`no_std`, pure); control-plane recognition
  (REQ-487, REQ-488). Attestation/R4 signature *verification* is pure; key *custody* is shell.
- `p2p::pair::error::classify(cause) -> FailureCategory` — operator-only; user
  text constant (REQ-479).
- `p2p::identity::{merge, validate, resolve}` — reuse the [[did:crdt]] pure core
  (`merge(state, delta) → state`, commutative/associative/idempotent; zero I/O);
  member-identity recognition + document projection (REQ-497, REQ-498). Delta
  *verification* is pure; delta *exchange* and roster persistence are shell.

### Effectful Shell (orchestrates I/O, calls pure core)

- `daemon::zetld` — lifecycle, control socket, supervision.
- `p2p::iroh` — [[QUIC]] endpoint, accept loop, roster enforcement.
- `p2p::pkarr` — DHT publish/resolve (rendezvous + durable records).
- `crdt::store` — `.zetl/loro/` persistence; oplog append; snapshotting.
- `crdt::materialise_sink` — write Markdown + drive git/jj flush.
- `p2p::pair::store` — roster + nonce persistence; rendezvous TTL pruning.
- `p2p::identity::exchange` — DID-delta send/receive over the roster-gated
  channel (never public DHT/gossip — threat §P).
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
| 476 | 476a/d, mut-rendezvous, adv | 474 | 473 | 474 |
| 477 | 477a/b/c | 474 | 473 | 475 |
| 478 | 478a/b | 474 | 473 | 475 |
| 479 | 479a/c, NFR-473 | 474 | — | 475,476 |
| 480 | 480a/b | 474,477 | 477 | 477 |
| 481 | 481a/b/c, mut-roster, NFR-474 | 477 | 477 | 477 |
| 482 | 482a/b, mut-roster | 473 | 472,476 | 478 |
| 483 | 483a/b/c, fuzz-spake, fuzz-loro | 470,473,474 | 472 | 478 |
| 484 | 484a/b/c/d/e | 471 | 471 | 479 |
| 485 | 485a/c | 473 | 478 | 480 |
| 486 | 486a/b/c/d | 473 | 478 | 480 |
| 487 | 487a/b | 470,473,474 | 479 | 481 |
| 488 | 488a/b | 473,474 | 479 | 481 |
| 489 | 489a/b | 470,474,477 | 480 | — |
| 490 | 490a/c | 470 | — | 470 |
| 491 | 491a,476b,476c | 474 | 473 | 474 |
| 492 | 492a/b | 473 | 472,476 | 478 |
| 493 | 493a/b | 473 | 478 | 477 |
| 494 | 494a/b/c | 473 | 479 | 478 |
| 495 | 495a/b | 473 | 478 | 477 |
| 496 | 496a/b | 474 | 473 | 475 |
| 497 | 497a/b, fuzz-did | 477 | 481 | 482 |
| 498 | 498a/b/c | 477 | 481 | 477,482 |
| 499 | 499a/b | 473 | 477 | 478 |
| 500 | 500a/b, fuzz-did | 477 | 481 | 482 |
| 501 | 501a/b/c | 473,474 | — | 478 |
| 502 | 502a/b/c/d | 477 | 481 | 477,482 |
| 503 | 503a/b | 473 | 472,479 | 478,481 |
| 504 | 504a/b/c | 471,473 | 470,478 | 471,472 |
| 505 | 505a/b/c | 477 | 477,481 | 477,482 |
| 506 | 506a/b | 477 | 477,481 | 477 |

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
  - S₁₀ — adopted [[did:crdt]] (`../did-crdt`) as the member identity layer on
    user direction: ADR-481, REQ-497 (DID-bound member identity; roster keyed
    by DID, devices as verification methods), REQ-498 (DID key removal triggers
    [[Group Key]] rotation — the two revocation mechanisms compose, Q11), §8.10
    delta grammar + TEST-fuzz-did, CON-477 rework (DID-keyed roster,
    `apply_did_delta`), OBS-482, threat §P (device-fleet metadata — DID material
    never on the public DHT), §9/§13 placement (rung 4, pure core reused).
  - S₁₁ — fourth fresh-context adversarial pass (protocol blockers) → F45–F56
    applied. **F45** pre-admission pairing identity: pairing control messages
    R4-verify against the ceremony's ephemeral NodeIds, authority conferred by
    the [[SPAKE2]] transcript; roster verification only post-admission
    (REQ-487, HP1, CON-474 C9). **F46** [[#REQ-499 Group-Keyed Sync Frames]]:
    §8.2 sync payloads AEAD-sealed under the current key epoch — the rotation
    of REQ-481/498 now actually protects the wire (TEST-481c/498b
    satisfiable). **F47** DID genesis inside pairing: self-certifying genesis
    delta + roster/DID bootstrap valid only inside a completed ceremony
    (§8.10, HP1 step 5, CON-477 C5). **F48**
    [[#REQ-500 Order-Independent DID Authorization]] (causal-context validity;
    forged-context residual → Q11). **F49** rotation as one author-signed,
    `rotation_id`-deduplicated event — idempotent under CRDT re-delivery,
    convergent under partition (REQ-498, CON-477 C6, TEST-498c). **F50**
    equal-root/unequal-vector sessions localise via per-document
    [[Version Vector]]s (REQ-486, ADR-478, CON-473 C5; TEST-486a now requires
    equal vectors). **F51** guarded import verifies the external edit's *base
    export* against retained export generations — a stale editor buffer can no
    longer fold daemon edits away as deletions (REQ-484, ADR-471, HP5,
    CON-471, TEST-484e). **F52** [[#REQ-501 Bounded Frame Recognition]]:
    fixed-width u32 prefixes, hard per-frame maxima, pre-auth read deadlines
    (§8 preamble, §8.2, §8.6). **F53** control→data references gain a
    per-session monotonic `ref-seq` — byte-identical replays rejected
    (REQ-494, §8.1, TEST-494c). **F54** rendezvous records carry only an
    ephemeral pairing endpoint — durable NodeIds never in enumerable records
    (ADR-473, §8.9, threat §H, HP1). **F55** HP1's phrase example now matches
    the `4*5DIGIT` routing grammar. **F56** Orientation names [[CBCL]], not
    CBOR, as the control channel.
  - S₁₂ — fifth fresh-context adversarial pass (structural gaps beyond the
    declared open questions; findings verified against the `../cbcl-rs`
    source and pkarr/spake2 documentation) → F57–F65 applied. **F57** threat
    §B corrected: an attacker who *knows* the phrase completes authenticated
    [[SPAKE2]] exchanges — OOB compromise is complete pairing compromise, not
    one guess; confidential-OOB assumption made explicit, Q2 upgraded.
    **F58** `HKDF(num)` gives *anyone* the rendezvous signing key: §J/ADR-473
    now record full write authority, whole-namespace pre-squat, and
    teardown = stop-republishing (pkarr cannot delete). **F59** guarded
    import: hash-based base identification only recognises unchanged saves —
    replaced with the decidable conservative intervening-export rule +
    cooperative-token refinement path (REQ-484/ADR-471/CON-471/HP5).
    **F60** [[#REQ-502 Replicated Signed Roster]]: signed admission/removal
    membership events (§8.12) make third-peer admission coherent with
    §8.10's ceremony-only genesis; TOML roster demoted to a projection
    cache; TEST-502a three-peer offline admission. **F61** "CBCL R4
    per-message signatures" do not exist — R4 is dialect-load integrity;
    per-message auth now names CBCL's attestation discipline (attest-v2/v3
    preimages, `verify_with_discipline`, phase-key lookup) across
    REQ-487/494, ADR-479, §8.8, OBS-481. **F62** the pre-auth pairing
    *control* envelope was unbounded and mislabelled "local process" — §8.1
    gains framing, a hard max, and the network boundary; REQ-501 extended to
    every untrusted frame (TEST-501c). **F63**
    [[#REQ-503 Vault-Bound Peer Sessions]]: versioned ALPN + vault selector
    (§8.11), vault id bound into attestation context and AEAD associated
    data — a multi-vault daemon can now select the roster REQ-492 requires.
    **F64** [[#REQ-504 Replicated Vault Namespace Manifest]]: DocId → path
    as a manifest CRDT with tombstones, identity-preserving rename, and
    deterministic case/NFC collision handling — the namespace converges, not
    just document contents (Q12 raised). **F65** [[Merkle DAG]]
    reconciliation loops — recompare roots/vectors after every exchange; the
    mixed root-mismatch + byte-cancelling case gets TEST-486d.
  - S₁₃ — targeted review of a user design question ("should add-member
    permission be delegatable?") → decision **no for v1** (deferral recorded
    on ADR-477/481) and **F66** applied: the review found the Owner-only
    admission policy (CON-474 C1) enforced only at the authoring CLI while
    the REQ-502/§8.12 acceptance rule validated *any* on-roster author — a
    compromised non-owner daemon (§D) could author admissions every honest
    peer accepted. [[#REQ-505 Role-Gated Membership Authority]] defines the
    previously-undefined roster `role` field (`owner | member`, immutable in
    v1) and gates membership-event acceptance on it (TEST-505a/b/c,
    `unauthorized-author`, OBS-477); threat §N gains the partial
    honest-verifier mitigation; self-scoped DID device management stays
    member-delegated.
  - S₁₄ — prior-art review of `../elephant-3000` on user direction (a
    downstream sibling whose SPEC-002/SPEC-004 implement this spec's
    pairing/roster patterns over MLS) → F67–F71 applied: F67 REQ-505
    authority judged from the attestation signing key, never a
    self-declared author field (elephant's leaf-index-not-credential-string
    lesson + forged-credential regression test); F68
    [[#REQ-506 Durable Epoch Rotation]] — transactional-outbox
    crash-safety for rotation (record before advance, exactly-once
    recovery); F69 `already-member` duplicate-admission rejection
    (CON-477 C9, TEST-502d); F70 vault genesis seeds the roster with the
    Owner's DID *and creating-device NodeId* (elephant BUG-002: missing
    steward self-entry refused steward↔member sync both ways); F71
    CON-474 C11 — a failed ceremony leaves no partial joiner state
    (TEST-476d). Evidence recorded without deciding gated questions: Q7
    gains the MLS/keybook/sole-committer candidate analysis, Q1 the
    working downstream implementation of the routing/secret split,
    ADR-479 the divergent loopback-control-plane counter-argument.
  - S₁₅ — **adopted [[MLS]] on stakeholder direction** ("yes, adopt MLS"):
    [[#ADR-482 MLS for Group Key Agreement and Membership Commits]]
    (Proposed, crypto no-go, human-gated) — openmls, one group per vault,
    leaf per device, credential = member DID, Owner as sole committer
    (realising REQ-505's owner-only authority), commits on a replicated
    Loro membership lane processed in epoch order, epoch data key
    distributed as an MLS application message sealing REQ-499 frames, no
    key-history keybook (plaintext-at-rest + current-epoch join sync).
    Reworked REQ-498 (rotation = Owner's Remove commit; F49 rotation-event
    machinery dissolved; Owner-offline deferral residual → Q11/§O),
    REQ-502 (event-CRDT merge → epoch-ordered commit lane; buffer-ahead /
    discard-behind), REQ-505 (sole-committer wording; self-scoped device
    management via DID deltas realised by Owner commits), REQ-500 rescoped
    to DID deltas, §8.12 regrammared as the MLS commit envelope, CON-477
    reworked (`process_membership_commit`, `epoch-gap`), Q7 recast as the
    MLS composition review, §9 placement row added. Tier-1 human review
    NOT discharged — the decision is recorded as Proposed with the
    stakeholder directive noted, exactly as elephant recorded its
    ADR-105 supersession.
  - S₁₆ — CLI verb rename on stakeholder direction: owner-side
    `zetl collab pair` → **`zetl collab invite`** (ADR-480 rationale:
    `invite`/`join` name each side's role; "pair" named the whole
    ceremony ambiguously; elephant reached the same naming
    independently). "Pairing" stays as protocol terminology. CON-474
    also gains the `--vault` join **pin** (fail closed if the ceremony
    offers a different vault — elephant's `expected_theory` guard).
  - S₁₇ — middle-path kickoff (0.18.0): §19 restructured into the
    two-track plan; `plans/DESIGN-047-loro-p2p-realtime-sync.spl`
    authored (validated `--strict`) with the Tier-1 gates as dependency-
    encoded tasks. **Comprehension gate re-run** on the 0.18.0
    Orientation block by a fresh-context subagent (tool-less, block-only):
    PASSED intent restatement, both behaviour predictions (offline
    concurrent edit convergence; stranger-connection refusal at the
    roster gate), and locate-the-artefact c1–c3; c4 ("who may add
    members") was **unanswerable from the block** — with two related
    findings (ADR-477/482 relationship unexplained, roster keying
    NodeId-vs-DID ambiguous). All three folded back into the Legend and
    Open lines. Minor findings (rejected-connection failure mode,
    first-contact vs steady-state discovery drawn as one arrow, CON-473/
    474 legend asymmetry) recorded here as accepted at door altitude.
    Cross-model review and human crypto review remain PENDING.
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
| 497 | ⚠ ADR-481 | ✓ | ✓ | ✓ | ⚠ | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ | ✓ | ✓ | ✓ |
| 498 | ⚠ ADR-481 | ✓ | ✓ | ✓ | ⚠ | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ | ✓ | ✓ | ✓ |
| 499 | ⚠ Q7 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ⚠ | ✓ | ✓ | ✓ | ✓ | ✓ |
| 500 | ⚠ ADR-481 | ✓ | ✓ | ✓ | ⚠ | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ | ✓ | ✓ | ✓ |
| 501 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ⚠ | ✓ | ✓ | ✓ | ✓ | ✓ |
| 502 | ⚠ ADR-481 | ✓ | ✓ | ✓ | ⚠ | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ | ✓ | ✓ | ✓ |
| 503 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ⚠ | ✓ | ✓ | ✓ | ✓ | ✓ |
| 504 | ⚠ Q12 | ✓ | ✓ | ✓ | ⚠ | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ | ✓ | ✓ | ✓ |
| 505 | ⚠ ADR-481 | ✓ | ✓ | ✓ | ⚠ | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ | ✓ | ✓ | ✓ |
| 506 | ⚠ Q7 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ | ✓ | ✓ | ✓ |

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
> closes when DESIGN-047 `adr-rendezvous` fixes the provisional attempt budget;
> REQ-499's ⚠s close when the Q7 group-key package fixes the AEAD; REQ-501's
> ⚠ Precise closes when DESIGN-047 `input-grammars` fixes the provisional frame
> maxima and read deadline; REQ-503's ⚠ Precise closes when the ALPN string
> and vault-id derivation are fixed (same task); REQ-502's, REQ-505's, and
> REQ-506's ⚠s
> close with the
> Q7/Q11 membership-event review; REQ-504's ⚠s close when DESIGN-047
> `adr-namespace` fixes the DocId scheme and collision-disambiguation rule
> (Q12).

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
   [[#REQ-496 Pairing Attempt Rate Limit]]; and (d) the rendezvous
   **write-authority** consequence of the split — anyone derives the
   `HKDF(num)` signing key, so pkarr record authenticity at a rendezvous is
   void and the whole ~110k namespace is pre-squattable (F58, threat §J) —
   is an acceptable availability residual. *(owner: HOC)* Downstream
   evidence (0.15.0): `../elephant-3000` implements this split verbatim
   (its SPEC-002 ADR-102 / `p2p::invite`) — a BIP39 wordlist, one
   recogniser for the `4*5DIGIT-word-word` grammar, transcript-bound key
   confirmation before any payload — and inherits this review as an
   *open* item rather than assuming it sound.
2. **Phrase-compromise recovery / pairing phishing (§B, §E) — upgraded by
   F57.** OOB phrase compromise is *complete pairing compromise*, not a
   bounded guess; the protocol currently ASSUMES a confidential, authentic
   OOB channel. [[NodeId]]-fingerprint / SAS confirmation is the only
   in-protocol backstop — the human crypto review MUST decide whether it
   ships in v1 (mandatory vs opt-in) or the OOB-channel assumption is
   accepted and documented for the v1 profiles. (Carries
   [[SPEC-036-spake2-onboarding]] §G.)
3. **CRDT granularity — RESOLVED (0.19.0, adr-merkle-sync).** Decision:
   **one [[Loro]] doc per note + one vault-level manifest doc**, not a single
   container tree — see the Q3 resolution in
   [[#ADR-478 Merkle DAG as Convergence Witness and Reconciliation Index]].
   Consistent with REQ-486/§8's per-document localisation (F43 closed);
   diverges from `../elephant-3000`'s one-doc-per-theory because zetl's unit
   of state is many notes, not one corpus. Unblocks IMPL-047 T3.
4. **Oplog growth / shallow snapshots.** Compaction aggressiveness vs
   offline-merge correctness (HP3 fallback).
5. **Mobile daemon.** iOS/Android background limits ([[SPEC-040-zetl-mobile]])
   may force a foreground/push-triggered `zetld`.
6. **Migration from [[diamond-types]].** One-shot `from_markdown` → [[Loro]], or
   oplog-history migration?
7. **Group-key cryptography — now the MLS composition review.** The
   mechanism is decided on stakeholder direction (0.16.0):
   [[MLS]] with the Owner as sole committer
   ([[#ADR-482 MLS for Group Key Agreement and Membership Commits]],
   pattern proven in `../elephant-3000`); the bespoke sealed-sender
   design and its concurrent-rotation precedence rule (F49) are
   superseded. What remains for the human crypto review: (a) the
   composition SPAKE2 ∘ MLS ∘ [[did:crdt]] as a whole; (b) the
   provisional ciphersuite; (c) the epoch **data-key** pattern — a random
   key distributed as an MLS application message, sealing
   [[#REQ-499 Group-Keyed Sync Frames]] with epoch + vault id in the AAD
   — versus MLS exporter secrets; (d) the multi-device-Owner commit
   authorisation rule (any leaf resolving to the owner DID — F67: is the
   resolution unforgeable under a compromised member leaf?); (e)
   anti-rollback unchanged: a peer that learns a higher `key_epoch` MUST
   refuse older epochs, bounding the stale-survivor window of
   [[#12. Threat Model]] §O (F35).
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
11. **Revocation composition (DID layer ↔ MLS layer).**
    [[#REQ-498 DID Key Removal Triggers Key Rotation]] ties a verification-method
    removal to the Owner's Remove commit
    ([[#ADR-482 MLS for Group Key Agreement and Membership Commits]]),
    which simplifies but does not close this question. For the human
    auth-core review: a key-removal delta that converges *after* the
    removed device has already synced under the old epoch; the
    **Owner-offline deferral window** — a removal delta accepted while no
    Owner device is online leaves the removed leaf in the group until an
    Owner commits (new with ADR-482; compare §O's survivor window); the
    forged-causal-context residual of
    [[#REQ-500 Order-Independent DID Authorization]] (a compromised key
    back-dating its causal context to dodge its own removal — F48, now
    scoped to the DID layer);
    interaction with did-crdt's own ADR-002 key-compromise recovery (a
    compromised controller key can author removals). Joins the Q7 MLS
    composition review. ([[#ADR-481 did:crdt as the Member Identity Layer]])
12. **Namespace manifest design (F64) — RESOLVED (0.20.0, adr-namespace).**
    [[#REQ-504 Replicated Vault Namespace Manifest]]: [[DocId]] = minted
    opaque 128-bit id (32 hex), path-independent so renames preserve
    identity; manifest CRDT shape = a [[Loro]] **map** `DocId → path` (map,
    not movable tree — Q3 is doc-per-note, so the manifest is a flat
    id→path index, not a hierarchy); collision rule = smallest DocId keeps
    the folded path, others get `stem (docid-prefix).ext` — deterministic,
    no silent overwrite. Path folding is provisional ASCII lower-case (full
    NFC + Unicode case-fold deferred, `// SIMPLIFY:` in `crdt::manifest`).
    Implemented: IMPL-047 T5 (`src/crdt/manifest.rs`, TEST-504a/b green);
    TEST-504c (concurrent rename vs edit) is a follow-up. Details in
    [[#REQ-504 Replicated Vault Namespace Manifest]].

---

## 19. Status & Next Actions

**Two-track middle path (0.18.0, stakeholder decision):** the spec stays
`draft`, but work proceeds on two gated tracks rather than waiting whole.

- **Track 1 — spec refinement to approval.**
  `plans/DESIGN-047-loro-p2p-realtime-sync.spl` now exists (validated
  `--strict`) and is the working plan: one task per Provisional marker
  (`user-profiles`, `happy-paths`, the `adr-*` refinements,
  `input-grammars`, `perf-budget`, `threat-model`, `test-strategy`) plus
  the Tier-1 gates as explicit tasks — `comprehension-gate` (agent-run),
  `cross-model-review` (REQUIRES a non-Anthropic model family — every
  pass to date is Claude-family), and `crypto-review` (human, owner HOC:
  Q1 rendezvous split, Q2 OOB/SAS, Q7 SPAKE2 ∘ MLS ∘ did:crdt
  composition, Q11 revocation composition). `spec-approval` is
  unreachable in the plan's logic until all four gate tasks complete —
  the gates are encoded as dependencies, not prose.
- **Track 2 — non-no-go implementation MAY begin now** (task
  `impl-047-plan`): daemon lifecycle, [[Loro]] store + deterministic
  materialisation, guarded import, namespace manifest, [[Merkle DAG]]
  reconciliation, [[CBCL]] control plane, and CLI scaffolding with
  `not-yet-implemented` exits ([[#ADR-480 CLI Surface Follows Existing zetl Conventions]]).
  **Auth-core code — pairing, [[MLS]], roster, group key — MUST NOT be
  implemented before `crypto-review` completes**; IMPL-047 encodes that
  gate as a task dependency.
- On approval, re-issue at `1.0.0`, remove Provisional markers, author the dead
  concept pages surfaced by `zetl check --dead-links`, and mark [[SPEC-004]]
  `superseded`.

---

## Changelog

<details>
<summary>Revision history — 0.1.0 → 0.21.0</summary>

- 0.21.0-strawman — DESIGN-047 `adr-control-proto` resolved: **two control
  planes**. The *network* peer session ([[#CON-473 Peer Session]]) uses
  [[CBCL]] (untrusted peers → R1–R5 load-bearing); the *local* daemon
  control channel ([[#CON-470 Daemon Control Channel]]) uses length-prefixed
  JSON, not CBCL (same-user loopback → R1–R5 buy nothing), confirming
  elephant's ADR-106 argument and matching the T2 implementation. CON-470's
  grammar/recogniser corrected from CBCL to the local JSON decoder; Q9
  (compact CBCL encoding) scoped to the network plane. Implementation this
  cycle also landed the T6 Merkle convergence witness (REQ-485, reusing
  `merkle::compute_vault_root`) and the T3 Loro→Markdown export bridge
  (ADR-470, path-traversal-safe).
- 0.20.0-strawman — DESIGN-047 `adr-namespace` resolved **Q12 (namespace
  manifest)**: [[DocId]] = minted opaque 128-bit id (path-independent →
  renames preserve identity); manifest = a [[Loro]] map `DocId → path`;
  collision rule = smallest DocId keeps the folded path, others
  disambiguate deterministically. Provisional markers on REQ-504/Q12
  removed (ASCII case-fold stays provisional). Implemented as IMPL-047 T5
  (`src/crdt/manifest.rs`). Alongside the implementation track this cycle:
  T4 (guarded-import fold/stage decision) and the T6 version-vector
  reconciliation core also landed and verified; T3 Loro store foundation +
  property tests earlier.
- 0.19.0-strawman — DESIGN-047 `adr-merkle-sync` resolved **Q3 (CRDT
  granularity)**: one [[Loro]] document per note keyed by manifest
  [[DocId]], plus one vault-level manifest document — not a single
  container tree ([[#ADR-478 Merkle DAG as Convergence Witness and Reconciliation Index]]
  Q3-resolution block). Consistent with REQ-486/§8's per-document
  localisation (F43 closed); diverges from elephant's one-doc-per-theory
  because zetl's unit of state is many notes, not one corpus. ADR-478
  Provisional marker removed. Unblocks IMPL-047 T3 (the Loro store is a
  keyed collection of per-note documents + the manifest, each persisted
  as snapshot + oplog under `.zetl/loro/<DocId>`). Runs alongside the
  IMPL-047 implementation track: T1 (P2P CLI surface) and T2 (`zetld`
  daemon lifecycle) landed and verified on this branch.
- 0.18.0-strawman — middle-path kickoff (stakeholder decision): §19
  restructured into two gated tracks — spec refinement to approval
  (DESIGN-047 plan authored at
  `plans/DESIGN-047-loro-p2p-realtime-sync.spl`, `--strict`-valid, one
  task per Provisional marker plus `comprehension-gate` /
  `cross-model-review` / `crypto-review` / `spec-approval` with the gates
  encoded as task dependencies) and non-no-go implementation
  (`impl-047-plan`; auth-core code explicitly gated on `crypto-review`).
  Fresh-context comprehension gate re-run on the Orientation block:
  passed (intent, both behaviour predictions, locate c1–c3); its c4
  finding — membership authority unanswerable from the block, with the
  ADR-477/482 relationship and roster keying ambiguous — fixed in the
  Legend/Open lines; minor door-altitude findings recorded in §15 S₁₇.
  Cross-model and human crypto reviews remain pending.
- 0.17.0-strawman — CLI verb rename on stakeholder direction:
  `zetl collab pair` → **`zetl collab invite`** ([[#ADR-480 CLI Surface Follows Existing zetl Conventions]]
  records the rationale: `invite`/`join` are complementary speech acts
  naming each side's role, where "pair" named the whole ceremony and left
  ambiguous which side runs it; `../elephant-3000` reached `invite`/`join`
  independently). "Pairing" remains the protocol term ([[Pairing Phrase]],
  [[SPAKE2]] pairing ceremony, CON-474 "Pairing Protocol"); surface
  updated across §1.3, HP1, the Owner profile, CON-470's control verbs,
  CON-474, threat §M/§N, TEST-489a. CON-474 additionally specifies
  `--vault` semantics for multi-vault daemons: required selector on
  `invite`, OPTIONAL fail-closed pin on `join` (the joiner learns the
  vault id inside the ceremony — elephant's `expected_theory` guard).
- 0.16.0-strawman — **adopted [[MLS]] (RFC 9420) for group key agreement
  and membership on stakeholder direction**
  ([[#ADR-482 MLS for Group Key Agreement and Membership Commits]] —
  Proposed, crypto no-go area, human review still required): openmls, one
  group per vault (`group_id` = vault id), **leaf per device** with the
  member's [[did:crdt]] DID as credential, **Owner as sole committer**
  (the cryptographic realisation of
  [[#REQ-505 Role-Gated Membership Authority]]), commits on a replicated
  [[Loro]] **membership lane** processed strictly in epoch order
  (buffer-ahead / discard-behind), the epoch **data key** distributed as
  an MLS application message and sealing
  [[#REQ-499 Group-Keyed Sync Frames]] unchanged; no key-history keybook
  (deliberate divergence from elephant — plaintext at rest, joiners sync
  under the current epoch). Consequences propagated: REQ-498's bespoke
  rotation-event design (`rotation_id` dedup, F49 concurrent-removal
  precedence) dissolved into epoch order, with the **Owner-offline
  rotation-deferral window** recorded as a new Q11/§O residual; REQ-502
  reworked from order-independent membership-event merge to the
  epoch-ordered commit lane (F60 causal-context machinery retained only
  for [[did:crdt]] deltas under REQ-500, now rescoped); §8.12 regrammared
  as the MLS commit envelope (CBCL envelope → openmls validation →
  owner-leaf resolution, F67); CON-477 reworked
  (`process_membership_commit`, `epoch-gap` buffering); Q7 recast from
  "design a sealed-sender scheme" to "review the SPAKE2 ∘ MLS ∘ did:crdt
  composition" (ciphersuite, data-key-vs-exporter, multi-device-Owner
  commit authorisation); §9 gains the openmls placement row (rung 4 via
  the elephant/hark provider pattern); TEST-498c/502a-d/505a-c reworded
  to the commit mechanism.
- 0.15.0-strawman — prior-art review of `../elephant-3000` (downstream
  sibling; its SPEC-002/SPEC-004 implement this spec's pairing/roster
  patterns over [[MLS]] and name SPEC-047 as pattern source) → F67–F71.
  Implementation lessons folded back as requirements: authority judged
  from the attestation signing key, never a self-declared author field
  (F67, REQ-505 — elephant authorises its steward by MLS leaf index, not
  credential string, with a forged-credential regression test);
  [[#REQ-506 Durable Epoch Rotation]] — rotation recorded durably before
  the rotating daemon advances its own epoch, exactly-once completion on
  recovery (F68, elephant's transactional outbox; CON-477 C10,
  TEST-506a/b); admission of an already-on-roster DID rejected
  `already-member` (F69, CON-477 C9, TEST-502d); vault genesis seeds the
  roster with the Owner's DID **and creating-device [[NodeId]]** (F70 —
  elephant BUG-002: the missing steward self-entry made the roster gate
  refuse steward↔member sync in both directions, caught only by a
  composed two-daemon e2e test; REQ-505); failed ceremony leaves no
  partial joiner state (F71, CON-474 C11, TEST-476d). Evidence recorded
  into gated questions without deciding them: Q7 gains the
  MLS(openmls)/keybook/sole-committer candidate — which would collapse
  much of the F48/F49/F60 order-independent-merge apparatus into
  epoch-ordered Owner commits; Q1 notes the working downstream
  implementation of the routing/secret split (review inherited as open);
  ADR-479 records elephant's contrary loopback-HTTP control-plane
  choice for DESIGN-047 to answer. Info table Related row links
  [[elephant-3000]].
- 0.14.0-strawman — admission-authority delegation reviewed on user question
  → **non-delegatable in v1**, deferral recorded
  ([[#ADR-477 Single Per-Vault Group Key]] /
  [[#ADR-481 did:crdt as the Member Identity Layer]] consequences, §1.3
  scope-out). Applied **F66**: the Owner-only admission policy bound only
  the authoring CLI (CON-474 C1) while REQ-502/§8.12 accepted membership
  events from *any* on-roster author — a compromised non-owner daemon
  (threat §D) could author admissions every honest peer accepted. New
  [[#REQ-505 Role-Gated Membership Authority]]: roster `role` defined at
  last (`owner | member`, fixed at admission, genesis mints `owner`) and
  membership-event acceptance gated on author role (owner for admissions;
  owner-or-affected-member for removals; self-scoped DID device management
  stays member-delegated); §8.12 recogniser + REQ-502 + CON-477 (C8,
  `unauthorized-author`) tightened; TEST-505a/b/c; OBS-477 rejection
  logging; threat §N partial honest-verifier mitigation; Owner profile
  rescoped; §14/§16 rows.
- 0.13.0-strawman — fifth fresh-context adversarial pass (structural gaps
  beyond the declared open questions, verified against `../cbcl-rs` and
  pkarr/spake2 docs) → applied F57–F65. Threat model honesty: OOB phrase
  compromise = **complete pairing compromise** (SPAKE2 gives no guess bound
  against a password-holder) — confidential-OOB assumption explicit, §B
  rewritten, Q2 upgraded to the fingerprint/SAS decision (F57); phrase-derived
  rendezvous keys give **anyone signing authority** — §J/ADR-473 record
  namespace-wide pre-squat and teardown = stop-republishing (F58). Mechanism
  corrections: per-message auth renamed from "R4" to CBCL's **attestation
  discipline** with named preimage + phase-key lookup (REQ-487/494, ADR-479,
  §8.8 — F61); guarded import's base identification replaced with the
  decidable conservative intervening-export rule (REQ-484/ADR-471/HP5 —
  F59). New protocol surface: [[#REQ-502 Replicated Signed Roster]] +
  §8.12 membership events (third-peer admission, roster TOML = cache —
  F60); [[#REQ-503 Vault-Bound Peer Sessions]] + §8.11 vault selector +
  versioned ALPN, vault id in attestation context/AEAD AD (F63);
  [[#REQ-504 Replicated Vault Namespace Manifest]] — DocId → path manifest
  CRDT, identity-preserving rename, deterministic case/NFC collision rule
  (F64, Q12). Hardening: §8.1 control envelope gains framing/max/deadline
  and its network boundary (was "local process"); REQ-501 covers every
  untrusted frame (F62, TEST-501c); [[Merkle DAG]] reconciliation loops
  until roots **and** vectors match — mixed-case TEST-486d (F65).
- 0.12.0-strawman — fourth fresh-context adversarial pass (protocol blockers)
  → applied F45–F56. Bootstrap now satisfies its own gates: pairing control
  messages verify against an ephemeral [[Pre-Admission Pairing Identity]]
  bound to the [[SPAKE2]] transcript, switching to roster verification after
  admission (F45, REQ-487/HP1/CON-474); DID **genesis** deltas are
  self-certifying and valid only inside a completed ceremony, with roster/DID
  bootstrap for fresh devices (F47, §8.10/CON-477). Revocation is convergent
  and enforceable: [[#REQ-499 Group-Keyed Sync Frames]] seals every §8.2 sync
  payload under the current [[Group Key]] epoch (F46 — rotation now protects
  the wire); rotation is one author-signed, `rotation_id`-deduplicated event,
  idempotent under CRDT re-delivery (F49, REQ-498/TEST-498c);
  [[#REQ-500 Order-Independent DID Authorization]] makes DID/roster state a
  deterministic function of the delta set (F48). Reconciliation cannot stall:
  equal-root/unequal-vector sessions localise via per-document
  [[Version Vector]]s (F50, REQ-486/ADR-478/TEST-486a). Data-loss/replay/
  privacy/DoS paths closed: guarded import verifies the external edit's base
  export (F51, REQ-484/ADR-471/TEST-484e);
  [[#REQ-501 Bounded Frame Recognition]] bounds every untrusted frame length
  pre-allocation with pre-auth read deadlines (F52, §8 preamble/§8.2/§8.6);
  control→data refs carry a monotonic `ref-seq` so byte-identical replays
  fail (F53, REQ-494/§8.1/TEST-494c); rendezvous records carry only an
  ephemeral pairing endpoint, never a durable [[NodeId]] (F54,
  ADR-473/§8.9/§H). Consistency: HP1 phrase example matches the `4*5DIGIT`
  grammar (F55); Orientation names [[CBCL]], not CBOR (F56). Q7/Q11 extended
  (frame AEAD, epoch precedence for concurrent removals, forged-causal-context
  residual).
- 0.11.0-strawman — adopted [[did:crdt]] (`../did-crdt`) as the **member
  identity layer** on user direction
  ([[#ADR-481 did:crdt as the Member Identity Layer]]): member = W3C DID whose
  verification methods are the member's device [[NodeId]]s; roster re-keyed
  DID → devices ([[#REQ-497 DID-Bound Member Identity]], CON-477 rework);
  DID key removal composes with [[Group Key]] epoch rotation
  ([[#REQ-498 DID Key Removal Triggers Key Rotation]], Q11 raised); §8.10 delta
  grammar (recognise → proof-verify → merge, fuzzed via TEST-fuzz-did); DID
  material confined to the roster-gated channel — never the public DHT
  (threat §P, device-fleet metadata); OBS-482; §9 placement rung 4 (pure
  sibling core, like `../cbcl-rs`); a second Tier-1 auth-core dependency —
  did-crdt's key-compromise-recovery and Sybil ADRs join the human-review
  package.
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
