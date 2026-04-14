# ADR-004: Default Link Deduplication

**Status:** Accepted

## Context

A page may contain multiple wikilinks to the same target (e.g., `[[Cache]]` mentioned three times in a single page). Should `zetl links` and `zetl backlinks` report the target once or three times?

Discovered during the agent ergonomics audit — see [[Agent Ergonomics]].

## Decision

Deduplicate by default at the output layer. A page appears once in results even if multiple links point to it.

## Rationale

- Agent workflows typically want "what pages are connected?" not "how many times is this link repeated?"
- Deduplication at the output layer keeps the internal [[Link Graph]] faithful to the actual link structure
- Users who need link counts can use `zetl stats` or `zetl export` for the full data

## Implementation

Deduplication happens after graph traversal, before JSON/table serialization. The graph itself retains all edges.

See also: [[Links Command]], [[Agent Ergonomics]], [[SPEC-003 Agent Ergonomics]]
