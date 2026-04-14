# Cache

zetl uses two-tier incremental caching for both the [[Link Graph]] and the [[Reasoning Engine]]'s theory.

```spl
(given mtime-based-cache)
(given incremental-rebuild)
(given disposable-cache)
```

## How it works

The cache lives in `.zetl/` at the vault root:

- `index.json` — serialised link graph with per-file [[Merkle Tree]] roots
- `theory.json` — serialised reasoning theory, conclusions, and grounding data

## Two-tier invalidation

1. **Tier 1 (mtime):** If a file's modification time hasn't changed, skip it entirely — no I/O beyond the stat call
2. **Tier 2 (content hash):** If the mtime changed but the BLAKE3 content hash matches the cached value, skip reprocessing

This handles the common case where `git checkout` updates mtimes without changing content.

## SPL-specific invalidation

For the theory cache, zetl uses the SPL AST hash (not content hash) to determine if re-reasoning is needed. This means:

- Prose edits that don't touch SPL → no theory rebuild
- SPL reformatting (comments, whitespace) → no theory rebuild
- SPL logical changes (new facts, modified rules) → theory rebuild

See [[Merkle Tree]] for the dual-hashing design.

## Design tension

There is an unresolved design tension in how aggressively to cache reasoning results:

```spl
; Cache the theory for fast startup
(normally r-cache-theory
  mtime-based-cache
  cache-reasoning-results)

; But recomputing ensures results are always fresh
(normally r-recompute-theory
  incremental-rebuild
  (not cache-reasoning-results))
```

Both arguments have merit. Run `zetl reason conflicts` on this vault to see how zetl surfaces this kind of tension.

## Safety

The cache is disposable — deleting `.zetl/` and re-running `zetl index` regenerates it. See [[Local-first Design]].

## Cache format

The index cache includes per-file entries with mtime, page name, links, SPL blocks, diagnostics, and Merkle root hash. The theory cache includes SPL AST hashes, section grounding hashes, and explicit grounding references. See [[SPEC-006 Merkle Tree]] for the full schema.

See also: [[Scanner]], [[Link Graph]], [[Reasoning Engine]], [[Merkle Tree]]
