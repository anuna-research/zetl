---
title: Link Graph
---

# Link Graph

zetl buildss a directed graph where nodes are pages and edges are [[Wikilinks]]. The graph powers all link-based queries: forward links, backlinks, multi-hop traversal, shortest path, and orphan detection.

## Data model

- **Node** — a Markdown page, identified by its title (filename without `.md`)
- **Edge** — a wikilink from one page to another, with optional alias, heading, or block reference

The graph is built by the [[Scanner]] and cached by the [[Cache]] for incremental re-scans.

```spl
(given directed-graph)
(given multi-hop-traversal)
```

## Queries

| Command | Description |
|---------|-------------|
| `links` | Forward links from a page, with configurable depth |
| `backlinks` | Pages linking to a target, with depth traversal |
| `path` | Shortest link path between any two pages |
| `export` | Full graph as JSON |

These are exposed through [[Graph Queries]]. With the `--with-conclusions` flag, link results are cross-referenced with the [[Reasoning Engine]] to show what each linked page contributes to the vault's logic.

## Library

Built on `petgraph`, a Rust graph data structure library.

See also: [[Scanner]], [[Graph Queries]], [[Vault Diagnostics]]
