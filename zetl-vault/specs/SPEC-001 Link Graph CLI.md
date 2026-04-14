# SPEC-001: Link Graph CLI

The foundational specification for zetl. Defines the scanner, link graph, graph queries, SimHash fuzzy matching, cache, and CLI contracts.

```spl
(given spec-001-documented)
```

## Scope

- [[concepts/Wikilinks]] parsing (all forms: basic, alias, heading, block, embed)
- Directed graph construction via [[Link Graph]]
- Graph queries: forward links, backlinks, shortest path, dead links, orphans
- [[architecture/SimHash]] fuzzy page name matching
- [[architecture/Cache]] with mtime-based invalidation
- Structured output (JSON default, table optional)

## Key requirements

| ID | Requirement |
|----|-------------|
| REQ-001 | Index Markdown files, extract wikilinks |
| REQ-002 | Build directed link graph |
| REQ-003 | Query forward links with depth |
| REQ-004 | Query backlinks with depth |
| REQ-005 | Detect dead links |
| REQ-006 | Detect orphan pages |
| REQ-007 | Syntax validation |
| REQ-008 | SimHash fuzzy page name search |
| REQ-009 | Graph statistics |
| REQ-010 | Shortest path between pages |
| REQ-011 | Persistent cache |
| REQ-012 | Ignore patterns (.gitignore) |

## Performance targets

- Indexing: >= 2,000 files/sec
- Query latency: <= 100ms
- Memory: <= 200 MB for 50k pages
- Binary size: <= 10 MB

## Architecture decisions

- [[ADR-001 Rust]] — language selection
- Wikilink syntax follows Obsidian conventions

## Design philosophy

- Files are the source of truth
- Cross-application compatibility (Obsidian convention)
- Agent-first structured output ([[JSON by Default]])
- Fast, disposable indexing
- Zero configuration

See also: [[Spec Index]], [[architecture/Scanner]], [[Link Graph]], [[CLI Reference]]
