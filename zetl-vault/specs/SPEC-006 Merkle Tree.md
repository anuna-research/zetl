# SPEC-006: Merkle Tree

Introduces content-addressed Merkle trees built during indexing. Enables two-tier cache invalidation, SPL grounding, and [[Drift Detection]]. See [[Merkle Tree]] for the architecture and [[Blocks Command]] for the CLI.

```spl
(given spec-006-documented)
(given content-addressed-hashing)
```

## The drift problem

SPL theories embedded in Markdown can drift from surrounding prose. Mtime-based caching cannot detect this — the file changed, but which part? The [[Merkle Tree]] solves this by hashing individual content blocks.

## Key requirements

| ID | Requirement |
|----|-------------|
| REQ-037 | Merkle tree construction during index |
| REQ-038 | SPL block dual hashing (content + AST) |
| REQ-039 | Two-tier cache invalidation (mtime + content hash) |
| REQ-040 | SPL-specific theory cache invalidation |
| REQ-041 | Implicit section grounding |
| REQ-042 | Explicit grounding via `(meta ... (source ...))` |
| REQ-043 | Drift detection in `zetl check --drift` |
| REQ-044 | Durable provenance with content hashes |
| REQ-045 | Content block discovery (`zetl blocks`) |

## Content grounding model

### Implicit (section-based)

Every SPL block is automatically grounded in its containing section. The grounding hash is computed from non-SPL leaves between the current heading and the next heading.

### Explicit

Facts can be explicitly grounded via `(meta label (source "ref"))` using:

- `^block-id` — same-file block reference
- Merkle hash prefix — content-addressed reference
- `[[Page^block-id]]` — cross-file reference

## Architecture decisions

- **ADR-008:** BLAKE3 hash algorithm (fast, cryptographic)
- **ADR-009:** Two-tier invalidation (mtime first, then content hash)
- **ADR-010:** Implicit section grounding as default

## Performance targets

- Construction overhead: <= 20% above baseline
- Memory: <= 30 MB additional
- Cache size: <= 5 MB additional

## VCS independence

All functionality works without Git. Git metadata is optional enrichment only. This distinguishes SPEC-006 from [[SPEC-007 Graph Diff]], which requires Git.

See also: [[Spec Index]], [[Merkle Tree]], [[Drift Detection]], [[Blocks Command]], [[architecture/Cache]]
