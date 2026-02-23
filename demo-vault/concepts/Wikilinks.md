---
title: Wikilinks
---

# Wikilinks

Wikilinks are inline references between pages using `[[double bracket]]` syntax. They are the primary connective tissue in a zetl vault, forming the edges of the [[Link Graph]].

## Syntax

| Form | Example | Description |
|------|---------|-------------|
| Basic | `[[Cache]]` | Link to a page |
| Aliased | `[[Cache\|caching layer]]` | Display text differs from target |
| Heading | `[[Cache#Design tension]]` | Link to a specific heading |
| Block | `[[Cache^summary]]` | Link to a block ID |
| Embed | `![[Cache]]` | Embed the target page inline |

## How zetl uses them

The [[Scanner]] extracts wikilinks from every Markdown file. The [[Link Graph]] stores them as directed edges, enabling:

- Forward link queries (`zetl links`)
- Backlink queries (`zetl backlinks`)
- Shortest path computation (`zetl path`)
- Dead link detection (`zetl check`)

## Cross-referencing with logic

When reasoning is enabled, `--with-conclusions` annotates link results with the [[Spindle Lisp]] conclusions that each linked page contributes. This bridges the [[Link Graph]] and the [[Reasoning Engine]].

## Compatibility

zetl supports the wikilink conventions used by Obsidian, Logseq, Foam, and Dendron. See [[Local-first Design]] for the principle that zetl never modifies your files.
