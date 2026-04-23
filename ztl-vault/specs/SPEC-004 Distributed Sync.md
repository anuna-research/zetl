# SPEC-004: Distributed Sync

Proposes distributed vault synchronization using a Spritely Goblins sidecar. See [[Distributed Sync Future]] for the decision record.

**Status:** Proposed — not yet implemented.

## Core insight

ztl stays pure Rust for local operations. A Guile Scheme Goblins sidecar handles networking, capability-based access control, and sync. Files remain the source of truth.

## Architecture

```
┌─────────┐  JSON-RPC   ┌──────────────┐  OCapN/CapTP  ┌──────────┐
│  ztl   │ ──────────> │   Goblins    │ ────────────> │  Remote  │
│  (Rust) │  Unix sock  │  (Guile)     │   Tor/TCP     │  Peers   │
└─────────┘             └──────────────┘               └──────────┘
```

## Capability model

- Access controlled via sturdyrefs (cryptographic capability tokens)
- Folder-based scopes: read, write, admin
- Capabilities can be created, delegated, attenuated, and revoked

## Sync protocol

- Delta-based file transfer (not full vault sync)
- Conflict detection via vector clocks
- Explicit merge — never silent overwrite
- Initially: conflict staging (always ask user)

## User profiles

- Solo researcher with multiple machines
- Research team sharing a knowledge base
- Agent-to-agent handoff between LLM sessions

## Why a sidecar?

- Keeps ztl's core free of networking dependencies
- Goblins provides battle-tested capability security
- The sidecar is optional — ztl works fully offline
- Aligns with [[decisions/Local-first Design]]

## Research spikes needed

1. Syrup serialization in Rust
2. Goblins sidecar prototype
3. Conflict detection accuracy testing

See also: [[Spec Index]], [[Distributed Sync Future]], [[decisions/Local-first Design]]
