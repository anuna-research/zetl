# Spec Index

zetl's design is documented in ten specifications, each covering a major feature area.

```spl
(given all-specs-documented)
```

## Specifications

| Spec | Title | Status | Primary Feature |
|------|-------|--------|-----------------|
| [[SPEC-001 Link Graph CLI]] | Bi-directional Link Graph CLI | Implemented | Wikilinks, graph queries, SimHash, cache |
| [[SPEC-002 Full-Text Search]] | Full-Text Content Search | Implemented | `zetl search` |
| [[SPEC-003 Agent Ergonomics]] | Agent Ergonomics & Robustness | Implemented | JSON errors, dedup, list/export |
| [[SPEC-004 Distributed Sync]] | Distributed Vault Sync | Proposed | Goblins sidecar, OCapN |
| [[SPEC-005 Defeasible Reasoning]] | Defeasible Logic over Markdown | Implemented | `zetl reason` (8 subcommands) |
| [[SPEC-006 Merkle Tree]] | Content-Addressed Merkle Tree | Implemented | BLAKE3 hashing, grounding, drift |
| [[SPEC-007 Graph Diff]] | Git-Backed Graph Diff | Implemented | `zetl diff` |
| [[SPEC-008 Watch Mode]] | Vault Watch with Event Stream | Implemented | `zetl watch` |
| [[SPEC-009 Xanadu View]] | Xanadu-Inspired Transclusion TUI | Implemented | `zetl view` |
| [[SPEC-010 Web UI Features]] | Web UI Feature Exploration | Proposed | `zetl serve` / `zetl build` |

## How specs relate

```
SPEC-001 (foundation)
  ├── SPEC-002 (search)
  ├── SPEC-003 (agent ergonomics)
  ├── SPEC-004 (future sync)
  ├── SPEC-005 (reasoning) ──── SPEC-006 (merkle/grounding)
  ├── SPEC-007 (diff)
  ├── SPEC-008 (watch)
  ├── SPEC-009 (view)
  └── SPEC-010 (web UI)
```

SPEC-001 defines the foundation (scanner, graph, cache, CLI). Each subsequent spec builds on it. SPEC-006 extends SPEC-005 with content-addressed grounding.

## Traceability

Each spec includes requirements (REQ-), contracts (CON-), tests (TEST-), and architecture decisions (ADR-). Cross-references between specs maintain traceability.

See also: [[CLI Reference]], [[Index]]
