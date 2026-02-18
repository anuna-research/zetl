---
title: Knowledge Graph
---

# Knowledge Graph

A knowledge graph is a network representation where nodes are concepts and edges are relationships between them. In the context of [[Knowledge Management]], your interlinked notes form a personal knowledge graph.

## Properties

- **Nodes** — Individual [[Atomic Notes]] or pages
- **Edges** — [[Bidirectional Links]] between notes
- **Clusters** — Groups of densely connected notes (topics)
- **Hubs** — Highly connected nodes ([[Map of Content]] pages)

## Metrics

Useful graph metrics for a [[Zettelkasten]]:

- **Orphan ratio** — % of notes with zero incoming links
- **Average degree** — mean number of links per note
- **Connected components** — isolated subgraphs (ideally just one)
- **Shortest path** — how many hops between any two ideas

Tools like [[zetl]] can compute these metrics with `zetl stats`.

## Visualization

Graph visualization tools (like Obsidian's graph view) render the knowledge graph spatially, revealing structure that isn't obvious from reading individual notes.

See also: [[Emergent Structure]], [[Bidirectional Links]]
