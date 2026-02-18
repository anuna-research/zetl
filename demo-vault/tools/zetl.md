---
title: zetl
---

# zetl

zetl is a CLI tool for analyzing [[Bidirectional Links]] in a [[Zettelkasten]]. It parses `[[wikilink]]` syntax from Markdown files, builds a [[Knowledge Graph]], and exposes query and validation commands.

## Commands

- `zetl index` — scan and index all Markdown files
- `zetl links <page>` — show forward links from a page
- `zetl backlinks <page>` — show all pages linking to a page
- `zetl check` — find dead links, orphans, and syntax errors
- `zetl similar <query>` — fuzzy search for similar page names
- `zetl stats` — print graph statistics
- `zetl path <from> <to>` — find shortest link path between pages

## Use Cases

- **Agents** — validate link integrity before committing changes
- **Humans** — discover orphan notes, audit broken links, explore connections
- **CI/CD** — run `zetl check --fail-on error` in pipelines

## Built With

Written in Rust. Uses `[[wikilink]]` parsing, the `petgraph` graph library, and SimHash for fuzzy matching.

See also: [[Obsidian]], [[Logseq]], [[Knowledge Management]]
