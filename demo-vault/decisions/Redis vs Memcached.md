---
title: Redis vs Memcached
---

# Redis vs Memcached

We evaluated Redis and Memcached as caching backends for the ztl index layer. The decision was driven by benchmark data and operational simplicity requirements.

```spl
(given redis-evaluated)
(given memcached-evaluated)
(given single-node-deployment)
```

^cache-decision-context

## Benchmark Results

We ran read/write throughput tests at various concurrency levels against both backends using a representative workload of 10 000 graph index lookups.

| Backend    | Reads/sec (p50) | Reads/sec (p99) | Writes/sec | Memory overhead |
|------------|-----------------|-----------------|------------|-----------------|
| Redis 7.2  | 185 000         | 210 000         | 92 000     | ~12 MB          |
| Memcached  | 170 000         | 198 000         | 88 000     | ~8 MB           |

^benchmark-results

The p99 numbers show Redis edges out Memcached on read latency under load, primarily because Redis pipelines responses more aggressively over a single connection.

^benchmark-interpretation

## Analysis

Given the benchmark data, Redis meets our throughput target of 150 000 reads/sec with comfortable headroom.

```spl
; Redis read throughput exceeds our threshold — grounded to the benchmark table
(meta redis-fast-enough (source "^benchmark-results"))
```

^analysis-block

Memcached also clears the threshold, but Redis offers additional features (pub/sub, persistence options, richer data structures) that future graph-query caching may exploit.

```spl
; Both backends acceptable for now; Redis preferred for future flexibility
(normally r-prefer-redis
  (and redis-fast-enough single-node-deployment)
  prefer-redis-over-memcached)
```

^redis-preference-rule

### Operational considerations

- Redis ships with `redis-cli` which simplifies debugging cache state during development.
- Both are available in standard distro package managers.
- Neither requires a separate service when running locally — ztl can be configured to skip caching entirely in read-only mode.

^operational-notes

## Decision

We adopt Redis as the recommended caching backend. Memcached remains a supported alternative for environments where Redis is unavailable.

See also: [[Cache]], [[Performance]], [[Local-first Design]]
