# Performance

zetl is designed for large vaults — tens of thousands of notes — without sacrificing startup speed or interactive responsiveness.

```spl
(given fast-startup)
```

## Targets

| Metric | Target | Measured (10k vault) |
|--------|--------|----------------------|
| Cold-start full scan | < 200 ms | 140 ms |
| Incremental re-index (1 file) | < 50 ms | 8 ms |
| Graph query p99 | < 10 ms | 4 ms |
| Search (10k files) | < 2 sec | — |
| Reasoning (10k rules) | < 500 ms | — |
| TUI startup | < 200 ms | — |
| View startup | < 200 ms | — |
| Scroll frame time | < 16 ms | — |
| Merkle construction overhead | < 20% | — |
| Diff (2k pages, 50 changed) | < 500 ms | — |
| Watch event latency (p95) | < 500 ms | — |

## Strategies

### Incremental parsing

The [[Scanner]] only re-parses files whose mtime has advanced since the last run. On typical editing sessions (1–5 changed files), this cuts scan time by 99% compared to a full rebuild. See [[architecture/Cache]].

### Merkle-based change detection

BLAKE3 content hashes let zetl skip unchanged files even when mtimes are unreliable (e.g., after `git checkout`). See [[Merkle Tree]].

### Memory layout

The [[Link Graph]] is stored as a flat adjacency list rather than a pointer-linked structure. This improves cache-line utilisation during multi-hop traversal.

### Index-free search

[[Search Command]] uses direct file scanning rather than an inverted index. This trades query speed on very large vaults for zero build-time indexing cost. See [[ADR-002 Search Without Index]].

See also: [[architecture/Cache]], [[Scanner]], [[Merkle Tree]], [[Link Graph]]
