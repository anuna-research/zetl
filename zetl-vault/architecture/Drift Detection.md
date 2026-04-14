# Drift Detection

Drift occurs when the prose surrounding an SPL block changes but the SPL itself does not. The facts and rules may no longer accurately reflect the arguments in the surrounding text.

## The problem

SPL blocks in Markdown are grounded in their surrounding prose. When you write:

```markdown
## Benchmark Results
Redis achieves 185,000 reads/sec at p99.

```spl
(given redis-fast-enough)
```
```

The fact `redis-fast-enough` is grounded in the benchmark paragraph. If someone later edits the benchmark numbers but forgets to update the SPL, the fact drifts from its grounding.

## How zetl detects it

zetl uses the [[Merkle Tree]] to track grounding freshness:

1. During indexing, each SPL block's containing section is identified (heading through next heading)
2. A section grounding hash is computed from the non-SPL leaves in that section
3. When the theory is built, section grounding hashes are stored in the theory cache
4. On subsequent `zetl check --drift`, current grounding hashes are compared against cached values
5. Mismatches are reported as drift warnings

## Implicit vs explicit grounding

### Implicit (section-based)

The default. Every SPL block is grounded in its containing section's prose. No annotation needed.

### Explicit (source metadata)

Facts can be explicitly grounded to specific content:

```spl
(given redis-fast-enough)
(meta redis-fast-enough (source "^benchmark-results"))
```

Explicit grounding uses `^block-id`, Merkle hash prefixes, or cross-file `[[Page^block-id]]` references. See [[Blocks Command]] for hash discovery.

## Using drift detection

```bash
zetl check --drift                       # report drift warnings
zetl check --drift --fail-on warning     # CI: fail on drift
```

Drift requires a baseline from a previous theory build. New files have no baseline and are not flagged.

## Design decisions

- SPL changes are NOT drift (the SPL was updated intentionally)
- Section restructuring (inserting a new heading) correctly triggers drift because the section boundaries change
- Drift is reported as a warning, not an error, by default

See also: [[Merkle Tree]], [[Check Command]], [[architecture/Cache]], [[SPEC-006 Merkle Tree]]
