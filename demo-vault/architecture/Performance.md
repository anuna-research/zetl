---
title: Performance
---

# Performance

ztl is designed for large vaults — tens of thousands of notes — without sacrificing startup speed or interactive responsiveness.

```spl
(given fast-startup)
(given incremental-rebuild)
```

^perf-goals

## Requirements

ztl must cold-start in under 200 ms on a vault of 10 000 Markdown files, complete incremental re-index in under 50 ms when fewer than 100 files have changed, and serve graph queries in under 10 ms at p99 on a fully-indexed vault of 50 000 nodes.

^perf-numbers

These figures were chosen to keep the tool imperceptibly fast during normal editing workflows, where the user expects results before they finish reading the command prompt.

^perf-rationale

## Strategies

### Incremental parsing

The [[Scanner]] only re-parses files whose mtime has advanced since the last run. On typical editing sessions (1–5 changed files), this cuts scan time by 99 % compared to a full rebuild.

^incremental-parsing-detail

### Merkle-based change detection

BLAKE3 content hashes let ztl skip unchanged files even when mtimes are unreliable (e.g. after a `git checkout`). Each file's Merkle root is stored in the [[Cache]] alongside its mtime.

```spl
; Merkle hashing enables reliable drift detection
(normally r-merkle-reliable
  (and content-addressed-hashing disposable-cache)
  reliable-change-detection)
```

^merkle-strategy

### Memory layout

The link graph is stored as a flat adjacency list rather than a pointer-linked structure. This improves cache-line utilisation during multi-hop traversal and avoids GC pressure (irrelevant in Rust, but good discipline).

^memory-layout-detail

## Targets summary

| Metric                        | Target   | Measured (10k vault) |
|-------------------------------|----------|----------------------|
| Cold-start full scan          | < 200 ms | 140 ms               |
| Incremental re-index (1 file) | < 50 ms  | 8 ms                 |
| Graph query p99               | < 10 ms  | 4 ms                 |

^perf-targets-table

See also: [[Cache]], [[Scanner]], [[Redis vs Memcached]], [[Link Graph]]
