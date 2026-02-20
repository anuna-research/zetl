# zetl

Bi-directional wikilink graph CLI for personal knowledge management.

zetl parses `[[wikilinks]]` from Markdown files, builds an in-memory link graph, and exposes query, validation, search, and visualization commands. Designed for both AI agents (JSON output) and humans (tables, interactive TUI).

## Features

- **Wikilink parsing** - `[[target]]`, `[[target|alias]]`, `[[target#heading]]`, `[[target^block-id]]`, `![[embeds]]`
- **Graph queries** - forward links, backlinks, multi-hop traversal, shortest path between pages
- **Vault diagnostics** - dead links, orphan pages, syntax errors
- **Full-text search** - content search with regex support, frontmatter/code-block awareness
- **Fuzzy matching** - SimHash-based page name similarity
- **Interactive TUI** - dashboard, page browser, link explorer, graph view, inline wikilink navigation
- **Incremental caching** - mtime-based index for fast re-scans
- **Agent-friendly** - JSON by default, structured errors, non-zero exit codes

## Install

```
cargo install --path .
```

Requires Rust 1.88.0+.

## Usage

Point zetl at a vault (a directory of Markdown files):

```bash
# Build the link index
zetl index --root ./my-vault

# Query links
zetl links "Some Page" --root ./my-vault
zetl backlinks "Some Page" --root ./my-vault
zetl backlinks "Some Page" --depth 2    # multi-hop

# Find shortest path between pages
zetl path "Page A" "Page B" --root ./my-vault

# Search content
zetl search "query" --root ./my-vault
zetl search "pattern" --regex

# Validate vault
zetl check --root ./my-vault
zetl check --dead-links --fail-on dead-links

# Fuzzy page name matching
zetl similar "zettelkasen" --root ./my-vault

# Stats and export
zetl stats --root ./my-vault
zetl list --root ./my-vault
zetl export --root ./my-vault    # full graph as JSON

# Interactive TUI
zetl tui --root ./my-vault
```

All commands default to JSON output. Add `--format table` for human-readable tables.

## TUI

`zetl tui` launches an interactive terminal interface with:

| View | Description |
|------|-------------|
| Dashboard | Vault stats and most-linked pages |
| Pages | Filterable page list |
| Links | Forward/back link explorer |
| Search | Full-text search with context |
| Diagnostics | Dead links, orphans, syntax issues |
| Page | Rendered markdown with wikilink navigation |
| Graph | Local link graph with depth toggle |

Navigate with `Tab`/`Shift+Tab` to cycle views, `Ctrl+K` for the quick switcher, `j`/`k` for scrolling, `Enter` to follow wikilinks, `Backspace` to go back.

## Compatibility

Works with any Markdown vault using `[[wikilink]]` syntax:

- Obsidian
- Logseq
- Foam
- Dendron

zetl never modifies your files. The index is a disposable cache stored in `.zetl/`.

## Development

```bash
make check    # fmt + clippy
make test     # run tests
make build    # debug build
make release  # release build
```

## License

MIT
