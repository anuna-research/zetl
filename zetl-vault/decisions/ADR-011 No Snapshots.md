# ADR-011: No zetl-Managed Snapshots

**Status:** Superseded (by `--features history`)

## Context

[[Diff Command]] needs a baseline to diff against. Should zetl maintain its own snapshot history, or delegate to Git?

## Original Decision

zetl uses Git as the sole history backend. It does not maintain snapshot storage.

## Rationale

- Most vaults are already in Git repositories
- Snapshot management adds significant storage and complexity
- Git provides rich baseline selection: refs, tags, dates, HEAD~N
- The [[Merkle Tree]] provides content-addressed change detection within a single point in time; Git provides the temporal axis

## Update: `--features history`

With the addition of the `history` feature flag, zetl now optionally manages its own snapshots via jj-lib, stored in `.zetl/jj/`. This provides:

- Automatic snapshotting on `zetl index` and `zetl watch`
- Time-travel queries via the `--at` flag
- A full `zetl history` command suite (log, page, timeline)
- History data in templates, hooks, and API endpoints

The original decision remains valid for the default (no-history) build — Git is still the fallback for `zetl diff`. With `--features history`, jj-lib replaces Git as the diff backend and adds temporal capabilities that Git alone couldn't provide.

## Trade-offs

- Without history: `zetl diff` requires Git — it's the only zetl command with this dependency
- With history: jj-lib adds significant dependencies but provides richer temporal querying

## Mitigation

When Git is unavailable (and history is not enabled), `zetl diff` produces a clear error message explaining the requirement. When history is enabled, no external VCS is required. See [[SPEC-007 Graph Diff]] for the full specification.

See also: [[Diff Command]], [[History Command]], [[ADR-012 Changed Files Reconstruction]], [[decisions/Local-first Design]]
