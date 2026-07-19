# SPEC-047 — Scope: did:crdt delta signing (multi-device + verified rotation)

**Headline:** the hard part is already built. `../did-crdt` has real Ed25519/k256
delta **signing**, real signature **verification** against verification-method
keys, and — the genuinely difficult piece — **order-independent causal
authorization** (`causal::verify_causal`, wired into `Document::merge`) that
enforces "only an already-authorized key may add another." All of that is
implemented *and tested with real keys*. What remains is **integration and a few
security-hygiene gaps**, not a crypto build. This scope is therefore much smaller
than "add signing to did-crdt" sounds.

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

## Workstream A — `../did-crdt` changes (upstream capability)

### A1 — Sign the genesis delta · **security-critical · S–M**
Today `Document::new` emits `SignedDelta::unsigned` (`document.rs:280`), takes no
signing key (`document.rs:231`), and `verify_signature` treats an empty proof on
a VM-less doc as valid (`validate.rs:53-59`) — so genesis is never
cryptographically verified.
- Add a signing key to genesis: `new_signed(pubkey_multibase, &SigningKey)` (or a
  key param on `new`) that calls `SignedDelta::new_genesis` (`delta.rs:232`).
- Tighten `verify_signature` so a genesis delta's signature must verify against
  the key it introduces (self-signed genesis); remove/limit the empty-proof
  bypass.
- **Watch:** the DID is `blake3(timestamp, proto_op, signer_key)` (`document.rs:245`).
  Keep the DID a hash of *content*, independent of the signature, so existing DIDs
  stay stable. Confirm under review.

### A2 — Document-level signed-mutation API · **M**
There are **no** authoring methods on `Document` (no add/remove verification
method); callers hand-assemble deltas via `SignedDelta::new_with_parents`
(`delta.rs:252`). Add convenience methods that build a *signed* delta at the
current DAG heads:
- `add_verification_method(&SigningKey, new_key, relationships)`,
  `remove_verification_method(...)`, `deactivate(...)`.
- Authorization is already enforced downstream by `verify_causal`; this is
  parent-selection + signing convenience. This is the method the **add-a-device**
  flow needs.

### A3 — Close the bare-`merge` footgun · **security-critical · S (broad)**
`Document::merge` (`document.rs:306`) does **not** verify signatures — only the
`_verified_` wrappers do; bare `merge` accepts unsigned/forged deltas
(`document.rs:300-305`). Either fold verification into `merge`, or make
`_verified_` the only public apply path and keep bare `merge` crate-private.
- Mechanical ripple: the many in-crate tests using `SignedDelta::unsigned` + bare
  `merge` (helper `merge_op`, `document.rs:1015`) must sign, or use a `#[cfg(test)]`
  unchecked path.

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

### B1 — Thread a signing key through identity · **M**
`MemberIdentity::genesis` / `DeviceIdentity` must hold a signing key and sign the
genesis (call A1). The device secret is *already* an Ed25519 key (the transport
secret) — so it can sign did:crdt deltas directly.
- **Decision for review (Q1/Q7):** that reuses the transport/DID key as the
  did:crdt signing key — cross-protocol reuse. Either accept it with domain
  separation (did:crdt's `signing_input` is a distinct structure) or derive a
  separate signing subkey from the device seed. Recommend a derived subkey.

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
- Needs member DID docs available to the resolver (the roster / group carries or
  fetches them; the did:crdt sync layer — `sync/gossip.rs`, `sync/live.rs` —
  already exists upstream).

### B4 — Verified rotation / revocation · **S–M**
Wire `revocation.rs` to did:crdt's signed **revoke-verification-method**; after
rotation, resolution rejects the revoked device key (`causal.rs` already refuses
deltas signed by a revoked key whose revocation is in their causal past). This
gives cryptographically-verified device removal, closing REQ-506's rotation loop.

---

## Ordering & what it unlocks

```
A1 (genesis signing) ─┐
A3 (merge footgun)    ─┼─► security foundation (do first)
A4 (cleanup)          ─┘
        │
A2 (mutation API) ──► B1 (sign genesis) ──► B2 (add device) ──► B3 (resolve device set) ──► B4 (rotation)
```

**Unlocks for zetl:** multi-device DIDs (one actor, many devices), a
cryptographically-verified device set (so `endpoint_owner` resolves via the
*signed* DID doc, not a single-key hash), and verified rotation (a removed
device's key stops resolving). It closes the crypto-guide §1a single-device limit
and the §1b "one DID, many devices" row.

**Rough total:** A ≈ 2 focused days (engine exists; A2/A3 are the substance),
B ≈ 2–3 days (B3 is the meatiest). The dominant *risk* is not effort but the two
review decisions: the genesis-signing tightening (A1) and the transport-key-reuse
question (B1) — both squarely in the Q1/Q7 crypto review, not something to settle
by implementing.

**Still out of scope (separate design):** per-*edit* provenance. This scope makes
the DID's *device set* verifiable; it does not sign individual Loro ops, so wiki
edit attribution stays cooperative (guide §1a) until op-level signing is designed.
