---
id: IMPL-047
title: "Loro P2P Realtime Sync — Technical Implementation Plan"
status: implementing
version: 0.1.0
last-updated: 2026-07-18
audience: agent, human
---

# IMPL-047 — Loro P2P Realtime Sync Implementation Plan

> Phase-3 technical plan for [[SPEC-047-loro-p2p-realtime-sync]] (0.18.0),
> executed under the middle-path decision of that spec's §19. Governed by
> [[PROTO-001]]. Companion plan: `plans/IMPL-047.spl` (hence).

## Orientation

```
Intent:   Sequence SPEC-047 into buildable, test-first increments so the
          non-no-go substrate lands now and the auth-core/crypto layer
          slots in behind the human crypto review — never before it.

Metaphor: build the harbour before the ships. The daemon, the Loro store,
          materialisation, the manifest, reconciliation, and the CBCL
          control plane are the harbour (buildable today); pairing, MLS,
          the roster, and did:crdt are the ships that may only dock once
          the crypto review clears them in.

  M1 non-no-go (BUILD NOW)          M2 auth-core (GATED: crypto-review)
  ┌────────────────────────┐        ┌─────────────────────────────────┐
  │ T1 CLI surface         │        │ T8  SPAKE2 pairing (invite/join)│
  │ T2 daemon + control    │        │ T9  MLS group key + commits     │
  │ T3 Loro store + mtrlise│        │ T10 roster + role gate          │
  │ T4 guarded import      │        │ T11 did:crdt member identity    │
  │ T5 namespace manifest  │        │ T12 group-keyed sync frames     │
  │ T6 Merkle reconcile    │        │ T13 revocation + durable rotate │
  │ T7 CBCL control plane  │        └─────────────────────────────────┘
  └────────────────────────┘         scaffolded now as `not-yet-implemented`
                                      exits (ADR-480), coded after sign-off
```

The key words MUST, MUST NOT, REQUIRED, SHALL, SHALL NOT, SHOULD, SHOULD
NOT, RECOMMENDED, MAY, and OPTIONAL are to be interpreted as described in
BCP 14 (RFC 2119, RFC 8174) when, and only when, they appear in all
capitals.

## Codebase baseline (as-is, 2026-07-18)

- `src/crdt/{mod,diamond,marks_doc,backend,blocks,marks}.rs` — the current
  **diamond-types** engine behind the `CrdtBackend` trait. SPEC-047 §9
  removes the trait (one engine, [[Loro]]); this is the largest and
  riskiest migration and touches `src/web/ws.rs` and every storage path.
- `src/cap/pair.rs` — the existing [[SPEC-034]] [[SPAKE2]] primitive the
  pairing layer reuses (Simplicity Ladder rung 4). **Auth-core — gated.**
- `src/cli.rs` — `Command::Collab { CollabCommand::{Passwd, Share} }`; no
  `Daemon` command yet. `src/main.rs` dispatches at `Command::Collab`.
- `src/merkle.rs` — the [[SPEC-006]] BLAKE3 Merkle DAG reused by T6.
- Sibling crates for M2: `../cbcl-rs`, `../did-crdt`, plus `openmls`,
  `iroh`, `pkarr` (not yet in `Cargo.toml`).

## Gating rule (NON-NEGOTIABLE)

No task in **M2** may be implemented before the DESIGN-047 `crypto-review`
task completes (Q1/Q2/Q7/Q11 signed off, owner HOC). Until then M2's CLI
and control-channel entry points exist but return the `not-yet-implemented`
non-zero exit of [[SPEC-047-loro-p2p-realtime-sync#REQ-489 P2P CLI Follows Existing zetl Conventions]].
The `plans/IMPL-047.spl` plan encodes this as a task dependency; do not
route around it.

## M1 — Non-no-go substrate (buildable now)

### T1 — CLI surface (REQ-489, ADR-480)

Add the `zetl daemon {start,stop,status}` group and extend `CollabCommand`
with `Invite`, `Join`, `Peers`, `Revoke`. All inherit the global
`--format`/`--json`/`--vault` flags, positional ids, TTY-only secret entry,
and the `not-yet-implemented` non-zero exit. This is build-time CLI
conformance — no crypto — and is the first increment.

- Implements: [[SPEC-047-loro-p2p-realtime-sync#REQ-489 P2P CLI Follows Existing zetl Conventions]]
- Verified by: [[SPEC-047-loro-p2p-realtime-sync#TEST-489a]], [[SPEC-047-loro-p2p-realtime-sync#TEST-489b]]
- Files: `src/cli.rs` (enums), `src/main.rs` (dispatch → `not-yet-implemented`).

### T2 — Daemon `zetld` + local control channel (REQ-470/471/490, CON-470)

Persistent process owning vault state and sockets; loopback control plane;
idempotent lifecycle; survives client disconnection. Recognise control
input against the declared grammar before acting (LangSec). No network
crypto — the control channel authenticates by filesystem permissions.

- Implements: REQ-470, REQ-471, REQ-490; CON-470.
- Verified by: TEST-470*, TEST-471*, TEST-490*.

### T3 — Loro store + deterministic materialisation (REQ-472/473, CON-471)

Replace the diamond-types engine with a [[Loro]]-backed `crdt::store`;
remove the `CrdtBackend` trait (SPEC-047 §9). Deterministic materialisation
to Markdown (canonical form, NFC). **Largest risk** — stage behind a
temporary feature flag, migrate `src/web/ws.rs` last.

- Implements: REQ-472, REQ-473, REQ-474 (merge); CON-471.
- Verified by: TEST-472*, TEST-473*, TEST-474*; property: `parse(materialise(x)) == x`.

### T4 — Guarded import of external Markdown edits (REQ-484, ADR-471)

The F59 conservative intervening-export rule over Loro logical time; fold
vs stage decision; delete/rename as manifest ops.

- Implements: REQ-484. Verified by: TEST-484*.

### T5 — Namespace manifest (REQ-504)

DocId → path as a Loro manifest CRDT; create/rename/delete; deterministic
case/NFC collision rule. **Depends on the `adr-namespace` DESIGN-047 task
fixing the DocId scheme first.**

- Implements: REQ-504. Verified by: TEST-504*.

### T6 — Merkle reconciliation + convergence witness (REQ-485/486, ADR-478)

Reuse `src/merkle.rs`; vault_root compare → DAG descent → per-doc version
vectors → Loro op deltas; reconciliation loops until roots and vectors
match.

- Implements: REQ-485, REQ-486. Verified by: TEST-485*, TEST-486*.

### T7 — CBCL control-plane message language (REQ-487/488, ADR-479)

`zetl-pair`/`zetl-sync` dialects over `../cbcl-rs`; one DPDA recognises all
control messages. The data plane stays binary length-prefixed. Note the
0.15.0 ADR-479 open item: confirm the local-plane CBCL-vs-loopback-HTTP
choice against elephant's counter-argument (DESIGN-047 `adr-control-proto`).

- Implements: REQ-487, REQ-488; CON-470 grammar. Verified by: TEST-487*, TEST-488*.

## M2 — Auth-core (GATED on crypto-review)

Scaffolded now as `not-yet-implemented`; implemented only after sign-off.

- **T8** SPAKE2 pairing (`zetl collab invite`/`join`, CON-474) — REQ-476/478/479/491/496.
- **T9** MLS group key + Owner-committed membership (ADR-482) — REQ-499/502/506.
- **T10** roster + role gate (CON-477) — REQ-480/505.
- **T11** did:crdt member identity (ADR-481) — REQ-497/498/500.
- **T12** group-keyed sync frames + roster-gated transport — REQ-482/492/499/503.
- **T13** revocation + durable epoch rotation — REQ-481/498/506.

## Purity Boundary Map (implementation)

- **Pure core** (`crdt::loro`, `p2p::proto` recognise, `p2p::pair` derive,
  `merkle`): materialise/import, CBCL recognition, rendezvous derivation,
  vault_root/diff — deterministic, no I/O.
- **Effectful shell** (`daemon::zetld`, `p2p::iroh`, `p2p::pkarr`,
  `crdt::store` persistence): sockets, DHT, disk.
- **Rule:** shell → core; core MUST NOT import shell. Enforced by module
  visibility + review.

## Verification

Per [[SPEC-047-loro-p2p-realtime-sync#10. Test Specifications]]: example
tests per REQ (positive / neg-input / neg-output), property tests for
materialise/import roundtrip and reconciliation convergence, fuzzing at
every recogniser (CBCL, Loro import, SPAKE2, did:crdt — M2), mutation on
rendezvous + roster. Red Gate: each test observed to fail before its
implementation.

## Changelog

<details>
<summary>Revision history</summary>

- 0.1.0 — initial technical plan authored from SPEC-047 0.18.0 under the
  middle-path decision; M1/M2 split with the crypto-review gate on M2;
  grounded in the as-is codebase (diamond-types engine, `cap::pair`,
  `merkle`, CLI structure). Companion `plans/IMPL-047.spl`.
</details>
