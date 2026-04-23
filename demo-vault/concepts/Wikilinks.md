---
title: Wikilinks
---

# Wikilinks

Wikilinks are inline references between pages using `[[double bracket]]` syntax. They are the primary connective tissue in a ztl vault, forming the edges of the [[Link Graph]].

## Syntax

| Form | Example | Description |
|------|---------|-------------|
| Basic | `[[Cache]]` | Link to a page |
| Aliased | `[[Cache\|caching layer]]` | Display text differs from target |
| Heading | `[[Cache#Design tension]]` | Link to a specific heading |
| Block | `[[Cache^summary]]` | Link to a block ID |
| Embed | `![[Cache]]` | Embed the target page inline |

## How ztl uses them

The [[Scanner]] extracts wikilinks from every Markdown file. The [[Link Graph]] stores them as directed edges, enabling:

- Forward link queries (`ztl links`)
- Backlink queries (`ztl backlinks`)
- Shortest path computation (`ztl path`)
- Dead link detection (`ztl check`)

## Cross-referencing with logic

When reasoning is enabled, `--with-conclusions` annotates link results with the [[Spindle Lisp]] conclusions that each linked page contributes. This bridges the [[Link Graph]] and the [[Reasoning Engine]].

## Compatibility

ztl supports the wikilink conventions used by Obsidian, Logseq, Foam, and Dendron. See [[Local-first Design]] for the principle that ztl never modifies your files.
