# SPEC-008: Watch Mode

Adds persistent file-system monitoring with graph event streaming. See [[Watch Command]] for the CLI.

```spl
(given spec-008-documented)
```

## Overview

`zetl watch` starts a persistent process that monitors the vault via OS file system events, incrementally re-indexes changed files, and emits graph-level events as NDJSON on stdout. The event vocabulary mirrors the [[Diff Command]] output schema.

## Key requirements

| ID | Requirement |
|----|-------------|
| REQ-053 | Watch command entry point |
| REQ-054 | FS event monitoring |
| REQ-055 | Debounced incremental re-index |
| REQ-056 | Merkle tree change detection |
| REQ-057 | NDJSON graph event stream |
| REQ-058 | Event types (page/link/orphan/dead-link added/removed) |
| REQ-059 | `--exec` command invocation |
| REQ-060 | Graceful shutdown |
| REQ-061 | Startup error handling |

## Design principles

- Files are the source of truth
- Agent-first (NDJSON on stdout, status on stderr)
- Events emitted only on actual graph changes (not every file save)
- Fast and disposable

## Architecture decisions

- **ADR-013:** Use `notify` crate for FS events
- **ADR-014:** Inherit [[SPEC-006 Merkle Tree]] for change detection
- **ADR-015:** NDJSON on stdout, status on stderr

## Performance targets

- Event latency: <= 500ms at p95
- CPU idle: <= 0.5%
- Memory: <= 250 MB

## User profiles

- **Writer** monitoring for dead links while editing
- **Agent** running an incremental reasoning loop

See also: [[Spec Index]], [[Watch Command]], [[Diff Command]], [[Merkle Tree]]
