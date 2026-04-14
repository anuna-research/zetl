# ADR-002: Search Without Index

**Status:** Accepted

## Context

zetl needs full-text content search across vault files (see [[Search Command]]). The question is whether to build an inverted index (like Lucene/tantivy) or scan files directly on each query.

## Decision

Use index-free scanning — read and search files directly on each query.

## Rationale

- **Simplicity** — no index build step, no index format to maintain, no index invalidation logic beyond what [[architecture/Cache]] already handles
- **Correctness** — results always reflect the current file contents; no stale index entries
- **Acceptable performance** — scanning 10,000 files completes in under 2 seconds on commodity hardware (see [[Performance]])
- **Consistency** — zetl already has an mtime-based cache for the link graph; adding a second indexing system increases complexity disproportionately

## Trade-offs

- Slower than an inverted index for very large vaults (>50k files)
- No ranking or relevance scoring — matches are returned in file order
- Regex search performance depends on pattern complexity

## Mitigation

The `--path` filter (from [[Agent Ergonomics]]) lets users restrict search to a subdirectory, which reduces scan scope. The `--limit` flag bounds result size.

See also: [[Search Command]], [[Performance]], [[SPEC-002 Full-Text Search]]
