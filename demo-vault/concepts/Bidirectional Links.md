---
title: Bidirectional Links
---

# Bidirectional Links

A bidirectional link is a connection between two notes that is navigable in both directions. When Note A links to Note B, Note B automatically shows a backlink to Note A.

This is the connective tissue of a [[Zettelkasten]]. Unlike traditional hyperlinks (which are one-way), bidirectional links make it easy to discover how ideas relate to each other from either direction.

## Why They Matter

- **Serendipitous discovery** — Backlinks reveal unexpected connections
- **Context preservation** — You can see every note that references a concept
- **Network effects** — Each new link increases the value of all connected notes

## Implementation

In tools like [[Obsidian]] and [[Logseq]], bidirectional links are created with `[[wikilink]]` syntax. The tool automatically computes backlinks.

For CLI-based workflows, [[zetl]] provides `zetl backlinks <page>` to query backlinks programmatically.

## Relation to Graph Theory

The set of all bidirectional links forms a [[Knowledge Graph]]. Each note is a node, each link is a directed edge. Analyzing this graph reveals:

- **Orphan notes** — notes with no incoming links (potentially disconnected ideas)
- **Hub notes** — notes with many links (key concepts, MOCs)
- **Dead links** — links pointing to non-existent notes

See also: [[Emergent Structure]], [[Map of Content]]
