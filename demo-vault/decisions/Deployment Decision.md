---
title: Deployment Decision
---

# Deployment Decision

This document captures the deployment strategy for zetl: how it is distributed, where it runs, and what infrastructure assumptions it makes.

```spl
(given single-binary)
(given no-network-calls)
(given local-first-design)
```

^deployment-context

## Distribution model

zetl ships as a single static binary via `cargo install`. There is no package server, no auto-update mechanism, and no telemetry. Users are responsible for keeping their installation current.

^distribution-model

## Performance acceptability

The performance requirements documented in [[Performance]] are acceptable for the target deployment environment (developer workstations and CI runners).

```spl
; Perf targets from Performance.md ^perf-numbers are acceptable for our deployment
(meta perf-acceptable (source "[[Performance^perf-numbers]]"))
```

^perf-acceptability-block

CI runners (GitHub Actions, GitLab CI) typically have 2–4 vCPUs and SSDs, which means zetl's incremental re-index target of < 50 ms is achievable even for large repositories.

^ci-context

## Caching backend decision

For deployments that opt into a caching backend, [[Redis vs Memcached]] is the reference decision. Locally, no external cache is required — zetl uses its own file-based [[Cache]].

```spl
; Adopt Redis where a caching backend is needed
(normally r-use-redis-in-deployment
  (and prefer-redis-over-memcached single-node-deployment)
  use-redis-backend)
```

^caching-backend-rule

## Platform targets

| Platform        | Status      | Notes                              |
|-----------------|-------------|------------------------------------|
| macOS (aarch64) | Primary     | Developer machines                 |
| Linux (x86_64)  | Primary     | CI and server environments         |
| Windows (x86_64)| Supported   | Tested via cross-compilation       |

^platform-targets

### CI integration

zetl can be invoked from CI to fail a build when dead links or SPL drift are detected:

```bash
zetl index && zetl check --exit-code
```

^ci-integration-snippet

See also: [[Performance]], [[Redis vs Memcached]], [[Cache]], [[Local-first Design]]
