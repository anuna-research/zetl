# SPEC-047 — Scope: did:crdt delta signing (multi-device + verified rotation)

**Headline:** `../did-crdt` needs essentially **no changes**. It already has real
Ed25519/k256 delta **signing**, signature **verification** against
verification-method keys, and — the genuinely hard piece — **order-independent
causal authorization** (`causal::verify_causal`, wired into `Document::merge`)
enforcing "only an already-authorized key may add another," all tested with real
keys. Genesis is *deliberately* hash-authenticated, not signed (see below), so
there is no genesis-signing work. **The entire remaining effort is zetl-side
integration** (Workstream B) that *uses* did:crdt's existing signed-mutation +
causal-authorization primitives to add multi-device and verified rotation.

All `../did-crdt` citations are `src/core/*` unless noted. This is a scope, not
an approval — the changes below are themselves auth-core and land under the same
Q1/Q2/Q7/Q11 review gate.

---

## What already works (do NOT rebuild)

| Capability | Status | Evidence |
|---|---|---|
| Ed25519 / k256 delta signing over canonical bytes | **done** | `delta.rs:264-287`; signs `{did, op, parents, timestamp}` (`delta.rs:346-362`), parents covered (REQ-369) |
| Signature verification vs the VM key + node-id binding | **done** | `validate.rs:47-133` (real `vk.verify`, `123`/`130`) |
| Order-independent causal authorization (REQ-500) | **done + wired** | `causal::verify_causal` (`causal.rs:55-98`) called by `Document::merge` (`document.rs:350`) |
| Verified merge paths (verify-before-apply) | **done** | `merge_verified_delta` (`document.rs:436`), `merge_verified_bundle` (`document.rs:594`); network boundaries use them (`sync/gossip.rs:459`, `service/handlers.rs:253`) |
| Test coverage of verify + causal + integration | **strong** | `validate.rs` round-trip/forgery tests; `causal.rs` authorization tests; `document.rs` phase4 real-key bundle tests (`2057-2165`) incl. forged-sig/tampered-op/order-independence |

The engine is present. The gaps are at the *edges*.

---

## Design principle: genesis = hash-authenticated; mutations = signature-authenticated

**Genesis deltas do NOT need signing.** The DID is `blake3(genesis ‖ device_key)`
(`document.rs:245-248`), so the DID string *is* the commitment to the genesis
key. Verifying genesis = recomputing the hash and checking it equals the DID
(what zetl's `derive_did`/`verify_did_binding` already do). A signature over the
genesis would be circular — it would sign a commitment the hash already makes.
Control of the DID is proven by *using* the genesis key afterwards (post-genesis
signatures; in zetl, the QUIC handshake against the endpoint==genesis-key
binding). An unsigned genesis for a key you don't control merely yields a DID you
cannot act as. So the empty-proof genesis path (`validate.rs:53-59`,
`document.rs:280`) is **correct by design, not a gap** — leave it.

**Only mutations need signatures**, and did:crdt already provides that end to end
(sign `delta.rs:252`, verify `validate.rs:47`, authorize `causal.rs:55`). The
signature on an add/remove-verification-method delta is non-redundant: it proves
an *already-authorized* key authorized the change.

## Workstream A — `../did-crdt` changes: minimal

### A2 — Document-level signed-mutation API · **M · optional convenience**
There are **no** authoring methods on `Document` (no add/remove verification
method); callers hand-assemble deltas via `SignedDelta::new_with_parents`
(`delta.rs:252`) — which **already works**. A `Document::add_verification_method
(&SigningKey, new_key, relationships)` / `remove_verification_method` /
`deactivate` wrapper (parent-selection + signing convenience) would be nicer, but
zetl can call the primitive directly, so this is optional, not blocking.
Authorization is enforced downstream by `verify_causal`.

*(Dropped: A1 "sign the genesis" — genesis is hash-authenticated by design, see
above. Dropped: A3 "make merge verify" — bare `merge` is the apply primitive; the
trust boundary is `merge_verified_*`, which the real untrusted-input callers
already use. At most an optional guardrail, not a fix.)*

### A4 — De-risk / cleanup · **S**
- `validate::check_authorisation` (`validate.rs:170-219`) is **dead code** relative
  to the live causal path — keep-or-remove decision.
- Stale "not yet wired / review-gated follow-up" comments (`admission.rs:47-48`,
  `causal.rs:11-16`) contradict the code (causal IS the live authorizer) —
  reconcile so the security-critical comments don't mislead a reviewer.
- Re-confirm FINDING-003: `signing_input` does not bind `suite` /
  `verification_method` (`validate.rs:61-76`) — accepted risk, re-check under review.

---

## Workstream B — zetl integration (consume signing)

### B1 — Thread a signing key for *post-genesis* deltas · **M**
Genesis stays unsigned (hash-authenticated). What `DeviceIdentity` must gain is
the ability to **sign mutation deltas** — the device-1 key signing the delta that
introduces device 2. The device secret is *already* an Ed25519 key (the transport
secret), so it can sign did:crdt deltas directly via `SignedDelta::new_with_parents`.
- **Decision for review (Q1/Q7):** signing did:crdt deltas with the transport/DID
  key is cross-protocol reuse. Either accept it with domain separation (did:crdt's
  `signing_input` is a distinct structure) or derive a separate did:crdt signing
  subkey from the device seed. Recommend a derived subkey.

### B2 — Add-a-device ceremony · **M**
For "one actor, many devices": generate `DeviceIdentity` for device 2, then have
device 1 (already authorized) sign an `add_verification_method` delta (A2)
introducing device 2's endpoint key. Distribute the delta so members' DID docs
resolve device 2.

### B3 — Membership resolution: single-key → resolved signed device set · **M**
This is where "1 actor, many devices" actually lands for the wiki.
`group.rs::verify_did_binding` / `endpoint_owner` today recompute
`genesis(endpoint).did() == did` (single-device). Change to: **resolve the DID
document and check the connecting endpoint key is an authorized (signed)
verification method.** Any of a user's devices then resolves to the one DID.

**Transport decision — DID docs piggyback on the MLS membership lane (REQ-502),
NOT did:crdt's own gossip/DHT (`sync/gossip.rs`, `sync/live.rs`).** Rationale: one
authenticated channel, ordering consistent with membership changes, and no second
sync substrate or pkarr/DHT dependency to secure. Signed did:crdt deltas ride the
membership lane alongside MLS commits; each member keeps a `did_crdt::Document`
per member-DID, updated via `merge_verified_delta` (verifies signature + causal
authorization).

**Same lane, two distinct authorities** (the important part): a payload's
authorization is per-type, not per-lane —
- **MLS commit** → *Owner*-authorized (leaf 0): which DIDs are members, and which
  device leaves get the group key (REQ-505).
- **did:crdt delta** → *member-self*-authorized (causal authorization over their
  own DID): which endpoint keys are that member's devices (REQ-498).

So **adding a syncing device is jointly authorized**: the member's did:crdt delta
(signed by an existing device) vouches "endpoint E is my device," and — after the
Owner verifies that delta on the lane — the Owner commits E's MLS leaf with
credential `(member_DID, E)`, granting group-key access. The `endpoint_owner`
resolver can then key off the MLS leaf credential (the group *is* the
endpoint→DID registry), with the did:crdt device set as the authorization proof
for why the Owner bound it. Neither authority can act as the other: the Owner
cannot forge a member's device set, and a member cannot grant themselves group
membership.

### B4 — Verified rotation / revocation · **S–M**
Wire `revocation.rs` to did:crdt's signed **revoke-verification-method**; after
rotation, resolution rejects the revoked device key (`causal.rs` already refuses
deltas signed by a revoked key whose revocation is in their causal past). This
gives cryptographically-verified device removal, closing REQ-506's rotation loop.

---

## Ordering & what it unlocks

```
(did:crdt already provides sign + verify + causal authorization — no A-work required)

B1 (post-genesis signing key) ──► B2 (add device, signed by device 1)
     ──► B3 (resolve signed device set) ──► B4 (verified rotation)
```

**Unlocks for zetl:** multi-device DIDs (one actor, many devices), a
cryptographically-verified device set (so `endpoint_owner` resolves via the
*signed* DID doc, not a single-key hash), and verified rotation (a removed
device's key stops resolving). It closes the crypto-guide §1a single-device limit
and the §1b "one DID, many devices" row.

**Rough total:** did:crdt ≈ 0 required (A2 is optional convenience). zetl B ≈ 2–3
days (B3 is the meatiest). The dominant *risk* is one review decision — the
transport-key-reuse question (B1, recommend a derived did:crdt signing subkey) —
squarely in the Q1/Q7 crypto review, not settled by implementing.

**Still out of scope (separate design):** per-*edit* provenance. This scope makes
the DID's *device set* verifiable; it does not sign individual Loro ops, so wiki
edit attribution stays cooperative (guide §1a) until op-level signing is designed.
