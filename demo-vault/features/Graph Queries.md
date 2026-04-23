---
title: Graph Queries
---

# Graph Queries

ztl exposes the [[Link Graph]] through several query commands. All output JSON by default — see [[JSON by Default]].

```spl
(given forward-links-done)
(given backlinks-done)
(given shortest-path-done)
```

## Commands

### `ztl links <page>`

Show all pages that `<page>` links to. Supports `--depth N` for multi-hop traversal and `--fuzzy` for approximate page name matching.

### `ztl backlinks <page>`

Show all pages that link to `<page>`. Same depth and fuzzy options as `links`.

### `ztl path <from> <to>`

Find the shortest chain of [[Wikilinks]] connecting two pages. Useful for discovering how ideas relate through intermediate notes.

### `ztl export`

Dump the entire [[Link Graph]] as JSON — every page and every link.

### `ztl list`

List all pages in the vault.

### `ztl stats`

Summary statistics: page count, link count, orphan count, and the most-linked pages.

## Cross-referencing with reasoning

Add `--with-conclusions` to `links` or `backlinks` to see what [[Spindle Lisp]] conclusions each linked page contributes. This bridges the structural graph with the logical theory from the [[Reasoning Engine]].

```bash
ztl -d . links "Cache" --with-conclusions
```

See also: [[Link Graph]], [[Vault Diagnostics]], [[Search]], [[TUI]]
