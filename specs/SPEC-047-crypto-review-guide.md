# SPEC-047 — Crypto / Auth-Core Review Guide

**Status:** review handoff. Nothing in this document asserts that the code is
sound — it is the map a human cryptography reviewer needs to *decide that*. All
code described here is marked `UNREVIEWED` and sits on branch
`spec/047-loro-p2p`. It compiles and its behavioural tests pass; **that is not
evidence of cryptographic soundness.** The [[SPEC-047-loro-p2p-realtime-sync#Q1]]
/ Q2 / Q7 / Q11 open questions gate trust, merge, and ship. Only Hugo asserting
`verified-crypto-review-hoc-signed` on `plans/IMPL-047.spl` opens M2.

Author's note: this was written by the same agent that wrote the code, so treat
the "what to scrutinise" flags as *leads, not a clean bill*. The adversarial
review must come from fresh context (Constitutional Principles 11–12).

---

## 0. How to build and exercise it

```bash
cargo test --lib 'p2p::'          # 21 auth-core unit tests (~30s: real QUIC)
cargo test --lib 'daemon::p2p'    # 2 wired-daemon sync tests
cargo test --lib 'crdt::'         # 56 sync-substrate tests (crypto-independent)
```

All auth-core lives under `src/p2p/` plus `src/daemon/p2p.rs`. The loud
`UNREVIEWED` banner is `src/p2p/mod.rs:1`.

---

## 1. What is *composed* vs what is *new*

The single most important framing for the reviewer: **almost none of the
primitive cryptography is hand-rolled.** The security posture rests on three
audited/standard dependencies, and the new code is the *glue and the trust
decisions around them*. Review effort should concentrate on the glue, not on
re-auditing the primitives.

| Concern | Primitive (not ours) | Our glue |
|---|---|---|
| Pairing key agreement | `crate::cap::pair` SPAKE2+HMAC (SPEC-034, already audited) | `src/p2p/pair.rs` |
| Group key / membership | `openmls` 0.8 (MLS RFC 9420) | `src/p2p/group.rs` |
| Frame confidentiality | `openmls` application messages (MLS AEAD) | `group::seal`/`open` + `session::FrameSeal` |
| Member identity | `../did-crdt` (sibling crate) | `src/p2p/identity.rs` |
| Transport auth + encryption | `iroh` 1.0 QUIC/TLS (Ed25519 endpoint id) | `src/p2p/transport.rs` |
| Control-message parsing | `cbcl-parser` (Lean-verified DPDA) | `src/p2p/control.rs` |

**No bespoke cipher, KDF, AEAD, or signature scheme was written.** Where I was
tempted to (frame sealing), I deliberately routed through MLS application
messages instead — see §4.

---

## 1b. The identity hierarchy (one key per device; DID per user; roster per wiki)

```
wiki  = vault = one MLS group / roster            ── many actors per wiki
 └── actor = user = did:crdt DID                   ── one DID, many devices
      └── device = one Ed25519 key (DeviceIdentity)
           ├── transport endpoint id  (iroh NodeId, QUIC-authenticated)
           ├── did:crdt device key     (verification method in the DID doc)
           └── Loro actor id / PeerID  (per-replica, attributes this device's edits)
```

`DeviceIdentity` (`src/p2p/identity.rs`) is the single key a node creates before
joining; transport id, DID, and Loro PeerID all derive from it, so a node/user
is one identity across transport, membership, and content.

**Terminology caution — "actor" is overloaded.** At the *content* layer, Loro's
"actor" (PeerID) is **per-device** (per replica). At the *identity* layer, an
"actor" is a **user = DID**, which owns many devices. So the map is
`Loro PeerID → device key → DID`: many PeerIDs per DID, many DIDs per wiki.

**What is single-device today:** one DID is derived from one device key, so
device ≡ user. The "one DID, many devices" row — and the `PeerID → DID` reverse
map for a *secondary* device — lives in the DID document's verification-method
set, which needs did:crdt **signed deltas** (REQ-500, Q11). Until then, treat
each device as its own DID.

## 1a. Is a DID a *verified* identity? (matters if a DID is a content subject)

**Yes for single-device, by self-certification — not by assignment.** A did:crdt
DID is `did:crdt:blake3(genesis ‖ device_key)` (did-crdt `document.rs:245`), so
it is the content-address of a key. We exploit that:

- `GroupIdentity::new(endpoint_id)` (`group.rs`) **derives** the DID from the
  device's transport endpoint key — the DID is not a label anyone chooses.
- `verify_did_binding(did, endpoint_id)` (`group.rs`) recomputes the address and
  is the admission check; `key_package_from_bytes` rejects any leaf whose DID is
  not the self-certifying address of the endpoint it binds.
- The QUIC handshake proves control of the endpoint key ⟹ controlling the
  endpoint proves control of the DID. So a DID resolved by `endpoint_owner` is a
  **verified** identity, safe to use as a content subject / author.

**Limits the reviewer must weigh before relying on it for attribution:**
- **Single-device only.** One DID = one device key today. Multiple devices under
  one DID, and key rotation, are the did:crdt verification-method set, which
  needs **signed deltas** (did-crdt phase-2, REQ-500, Q11). Until then a DID that
  loses/rotates its device key cannot be re-proven.
- **The transport key *is* the DID device key.** That unification is what makes
  self-certification work without did-crdt signing; confirm it's acceptable that
  the network identity and the content-identity are the same key (it does mean a
  device-key compromise is a full identity compromise).
- **Attribution granularity.** Sync frames are authored by MLS *leaves*; a leaf's
  DID is now verified, so "authored by did:crdt:X" is sound — but only at the
  sync-session granularity. Per-edit signing (so a *stored* wiki edit carries a
  verifiable author, independent of who synced it) is **not** implemented; that
  is a separate design question if edit-level provenance is needed.

---

## 2. The trust-boundary decisions (review these first)

These are the load-bearing *design* choices encoded in code. A subtle error
here defeats the primitives underneath.

### 2.1 Owner-only membership is enforced by leaf index, not credential

`src/p2p/group.rs:221` `process_commit` — the Owner is **permanently MLS leaf
0**, and a commit is accepted only if `sender == Sender::Member(LeafNodeIndex(0))`
*and* leaf 0's credential matches the expected owner DID (`group.rs:239-244`).
This is modelled on the `../elephant-3000` forged-credential lesson: **do not
authorise by the attacker-chosen credential string.**

- **Is leaf 0 immutable? — investigated, answer: structurally no, effectively
  yes (sound).** Leaf reuse *is* real in MLS: `free_leaf_index`
  (openmls-0.8.1 `treesync/diff.rs:229`) returns the **leftmost blank** leaf, and
  within a commit removes are applied first (`proposal_store.rs:508`), so a
  `[Remove(0), Add(x)]` commit *would* refill leaf 0. What makes leaf 0
  effectively immutable here is the conjunction of three facts:
  1. the creator is leaf 0 (`builder.rs:186`);
  2. only leaf 0 may author an accepted commit — our gate at `group.rs:239`
     (`sender == Sender::Member(LeafNodeIndex(0))`), which also rejects external
     commits since their sender type is `NewMemberCommit`, not `Member`;
  3. a committer cannot remove itself — openmls enforces this at **creation**
     (`commit_builder.rs:575` `CreateCommitError::CannotRemoveSelf`) *and*
     **processing** (`proposal_store.rs:317` ValSem200 `SelfRemoval`).

  The only party allowed to commit (leaf 0) is exactly the one that cannot be
  removed by a commit ⟹ leaf 0 never blanks ⟹ `free_leaf_index` never returns 0
  ⟹ never reused. Corollary: this vindicates the leaf check over a credential
  check — a malicious owner adding a second member who *forges*
  `BasicCredential = owner_did` puts that member at leaf 1+, and the leaf-0 gate
  means they are **not** owner.
- **Residual for the reviewer:** (a) the guarantee is a *conjunction* of our gate
  + MLS's self-removal rule — it holds only while `process_commit` keeps the
  exact `Member(0)` sender check and never loosens it to a credential
  comparison; (b) **availability flip-side** — because leaf 0 can never be
  removed, losing the owner device key *permanently freezes* the group (no
  membership change ever again). Intended REQ-505 posture, but owner key-loss is
  unrecoverable; ties into the REQ-506 rotation story.
- **Note the asymmetry:** `roster.rs:52` determines Owner by *credential-string
  comparison* (`did == owner_did`). The roster is a **projection/view** for UI
  and the admit decision; the **cryptographic enforcement** is at commit time in
  `process_commit`. A reviewer should confirm nothing security-critical trusts
  the roster's string comparison in place of the leaf-0 check.

### 2.2 A resolved / authenticated address is never membership

Three layers each restate this, and the reviewer should confirm the chain has no
gap:

1. `transport.rs:145` `accept` returns the QUIC-**authenticated** peer endpoint
   id but **does not gate** — see the module doc `transport.rs:17-22`.
2. `session::roster_admits` (`session.rs:38`) is the membership predicate.
3. `daemon/p2p.rs:87` `admits` + `daemon/p2p.rs:102-106` refuse an unadmitted
   peer **before any vault frame** (fail-closed), then `finish()` the connection.

- **Binding — now cryptographic in the library (was provisioned).** The member
  leaf credential now binds `did ‖ endpoint_id` (`group.rs` `encode_credential`
  / `decode_credential`, a fixed 32-byte suffix grammar). At admission,
  `key_package_from_bytes` (`group.rs:184`) verifies **both** the joiner's DID
  *and* that the credential's endpoint id equals the pairing-authenticated one —
  so a joiner cannot bind an endpoint it does not control.
  `group::endpoint_owner` (`group.rs:117`) then resolves a QUIC-authenticated
  endpoint id to its member DID via the MLS-authenticated, Owner-authorised
  leaf. Chain: `pair authenticates E` → Owner admits KeyPackage binding
  `(DID, E)` → leaf is MLS-authenticated → `E` connects, QUIC authenticates it →
  `endpoint_owner` resolves `E` → member. No did:crdt signing needed (that
  becomes the multi-device generalisation). Tested:
  `endpoint_resolves_to_member_only` (member resolves, stranger → `None`,
  endpoint-mismatch admission rejected).
- **Scrutinise:** (a) the credential grammar is a fixed 32-byte suffix on
  `BasicCredential` bytes — a LeafNode extension would be cleaner (flagged as a
  ceiling in `group.rs:86`); confirm the suffix split can't be gamed (DID is the
  UTF-8 prefix, endpoint the raw 32-byte tail). (b) The binding is only as strong
  as the pairing ceremony that authenticates `E` (§2.3). (c) The leaf signing key
  and the transport key are **deliberately separate** (no cross-protocol Ed25519
  reuse); confirm that's the right call vs. binding them.

### 2.3 Mutual pubkey confirmation in pairing (MITM defence)

`src/p2p/pair.rs:87-96` — after the SPAKE2 exchange, each side sends its
transport pubkey plus an HMAC tag keyed by the SPAKE2-derived key, and verifies
the peer's (`key.authenticate_pubkey` / `key.verify_pubkey`, both from the
audited `cap::pair`). This binds the phrase-authenticated channel to the peer's
durable transport identity (REQ-491), defeating a relay MITM.

- **Scrutinise:** the ceremony is **symmetric** (both peers run identical code,
  `pair.rs:73`). Confirm the symmetry does not enable a reflection/relay attack
  where an adversary loops one side's messages back — SPAKE2's symmetric mode
  (as used in SPEC-034) is designed for this, but the *composition* (pubkey tag
  after key derivation) should be checked for transcript binding.

---

## 3. Failure-mode discipline (in scope for Q2/Q7)

- **Opaque pairing failure (REQ-479):** `pair.rs:48-49` collapses every auth
  cause (wrong phrase, tampered tag, protocol error) to a single peer-visible
  `auth-failed` / dropped connection; only the local caller sees the specific
  `PairError`. Confirm no error path leaks a distinguishing signal (timing is
  *not* addressed — see §6).
- **Fail-closed control plane (REQ-487):** `control.rs:28` `recognise` accepts a
  control message only on `PipelineResult::Success` from the shared
  `cbcl-parser` pipeline — one recogniser, no parser-differential, total over
  arbitrary input (`control.rs` test `recognition_is_total`).
- **Bounded recognition before allocation:** every framed reader rejects an
  over-limit length prefix before allocating — `pair.rs:121`, `session.rs:136`,
  `daemon/server.rs` control frames. Pre-auth bound on pairing is 4 KiB
  (`pair.rs:28`); sync frames 64 MiB (`session.rs:32`).

---

## 4. Frame sealing (REQ-499) — the one place I resisted rolling crypto

`src/p2p/group.rs:274` `seal` / `group.rs:309` `open`. Each sync frame is sealed
as an **MLS application message** via `group.create_message` /
`process_message`, reusing MLS's own AEAD + per-leaf sender ratchet. `open`
**fails closed**: only an encrypted `PrivateMessage` carrying an
`ApplicationMessage` is accepted (`group.rs:311-323`) — a plaintext
`PublicMessage`, a commit, or a proposal is rejected.

`session::FrameSeal` (`session.rs:48`) keeps the sync protocol ignorant of MLS;
`GroupSealer` (`group.rs:290`) is the adapter; `session::sync_one_sealed`
(`session.rs:82`) is the sealed path. Test `sealed_frame_round_trips_within_the_group_only`
(`group.rs:411`) confirms a member opens it, a non-member cannot, and the
plaintext is not on the wire.

- **Scrutinise:** using MLS application messages for a **bidirectional
  request/response sync within one epoch** is slightly unusual. Both peers seal
  under their own leaf ratchet and open under the peer's. Confirm: (a) no
  generation-reuse / nonce-reuse across the two frames each side sends
  (`session.rs:82-104` sends exactly two: VV then delta); (b) that a dropped/
  reordered frame cannot desynchronise the ratchet into an accept-anything
  state; (c) that forward secrecy expectations across epochs hold given we do
  **not** commit between the two frames.
- **Not yet wired into the daemon:** the live daemon sync (§5) currently runs
  **unsealed** (transport-auth + roster-gated). `seal`/`open` are implemented and
  tested but compose on top only once the daemon holds a durable MLS group —
  called out as a ceiling in `daemon/p2p.rs:125-128`.

---

## 5. What is deliberately *simplified* (SIMPLIFY ceilings — not defects, but review them)

Each is annotated in-code with an upgrade path and a traced artefact:

1. **Admit set is provisioned from a file *in the daemon only*** (`daemon/p2p.rs`).
   The cryptographic resolver (`group::endpoint_owner`, §2.2) now exists and is
   tested; the flat file is an **interim shim** only because the daemon does not
   yet hold the live MLS group (that is the durable-MLS-provider slice, elephant
   REQ-306). Once the daemon loads the group, `admits(E)` becomes
   `endpoint_owner(group, E).is_some()` and the file is deleted. The binding
   itself is no longer provisioned-trust — only its *consumption in the daemon*
   is pending. This moved from "top gap" to "wiring pending".
2. **In-daemon sync is unsealed** (`daemon/p2p.rs:125-128`): transport-auth +
   roster-gated only. REQ-499 sealing (§4) composes on top once a durable MLS
   provider exists.
3. **did:crdt deltas are unsigned** (`identity.rs:11-17`): the upstream crate is
   phase-1 and `Document::new` returns a genesis delta with an empty signature.
   So the identity layer lands the *structure*; the signed-delta authentication
   REQ-500 needs is upstream's follow-up, reviewed under Q11. **Do not treat DID
   deltas as authenticated yet.**
4. **Revocation durability** (`revocation.rs`): the rotation outbox
   (`write_outbox`/`read_outbox`/`recovery_action`) gives transactional,
   exactly-once epoch-rotation recovery (REQ-506), but the MLS group itself uses
   the in-memory `OpenMlsRustCrypto` provider — the durable provider + crash-safe
   group storage (elephant REQ-306) is deferred.

---

## 6. Known *not-addressed* concerns (call them out so they aren't assumed done)

- **Timing side channels:** no constant-time discipline was added beyond what the
  primitives provide. HMAC tag comparison is inside `cap::pair` (audited);
  confirm nothing in the new glue branches on secret-dependent timing.
- **Replay / freshness across sessions:** sync frames carry no session nonce
  beyond MLS's own generation counter; a reviewer should confirm the MLS ratchet
  is sufficient and that a replayed *whole session* cannot roll a peer back.
- **DoS / resource bounds under adversarial peers:** frame caps exist (§3) but
  there is no per-peer rate limiting, connection cap, or sync-time budget.
- **Key storage at rest:** `daemon/p2p.rs:136` reads a raw 32-byte endpoint key
  from `p2p/endpoint.key` (file-permission protected, same 0600 discipline as the
  control socket) — no OS keychain / encryption at rest.

---

## 7. Suggested reading order for the reviewer

1. `src/p2p/mod.rs` — the banner and the composition-over-invention thesis.
2. `src/p2p/pair.rs` — smallest, most self-contained, highest-stakes (§2.3).
3. `src/p2p/group.rs` — `process_commit` (§2.1) then `seal`/`open` (§4).
4. `src/p2p/session.rs` — the `FrameSeal` layering and the sealed sync loop.
5. `src/p2p/transport.rs` + `src/daemon/p2p.rs` — the endpoint-id→membership
   chain (§2.2) and the ceilings (§5).
6. `src/p2p/identity.rs` + `roster.rs` + `revocation.rs` — identity structure,
   the roster-as-view nuance, and rotation recovery.

The highest-value adversarial questions, ranked: **(a)** ~~is MLS leaf 0
immutable enough~~ — **investigated and resolved** (§2.1): effectively immutable
via our gate + MLS's dual self-removal prohibition; reviewer should confirm the
conjunction and the availability caveat. **(b)** ~~is the endpoint-id ↔ DID
binding real or provisioned-trust~~ — **addressed** (§2.2): now MLS-bound in the
leaf credential and verified at admission against the pairing-authenticated
endpoint; resolver tested. Remaining: credential-grammar-vs-extension, the
pairing dependency, and wiring the resolver into the daemon (needs durable MLS).
**(c)** is the bidirectional application-message sealing free of ratchet/nonce
hazards (§4)? — **now the top remaining crypto-design question.**
