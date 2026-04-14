# Distributed Sync Future

**Status:** Proposed (not yet implemented)

## Context

Users with multiple machines or collaborating teams may want to sync vaults. [[SPEC-004 Distributed Sync]] proposes a design using a Spritely Goblins sidecar communicating via OCapN (Object Capability Network).

## Design overview

- zetl stays pure Rust for all local operations
- A Guile Scheme Goblins sidecar handles networking, capability-based access control, and sync
- Communication between zetl and the sidecar uses JSON-RPC over Unix domain sockets
- Files remain the source of truth — sync is delta-based with explicit merge
- Capabilities (not passwords) control access via sturdyrefs

## Why a sidecar?

- Keeps zetl's core free of networking dependencies
- Goblins provides battle-tested capability security (OCapN/CapTP)
- The sidecar is optional — zetl works fully offline without it
- Aligns with [[decisions/Local-first Design]]: network is opt-in, never required

## Current status

This is a future direction. No implementation exists yet. Research spikes are planned for Syrup serialization in Rust, Goblins sidecar prototyping, and conflict detection accuracy.

## Deliberate gap

This page has no SPL facts — modeling the `zetl reason why-not` use case. Running `zetl reason why-not "distributed-sync-ready"` would show that no facts support this conclusion because the feature doesn't exist yet.

See also: [[decisions/Local-first Design]], [[SPEC-004 Distributed Sync]]
