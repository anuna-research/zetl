# zetl

Bi-directional wikilink graph CLI with defeasible reasoning for personal knowledge management.

zetl parses `[[wikilinks]]` from Markdown files, builds an in-memory link graph, and exposes query, validation, search, and visualization commands. Optionally, it extracts [Spindle Lisp (SPL)](https://codeberg.org/anuna/spindle-rust) code blocks from your vault and performs defeasible reasoning — drawing conclusions that can be defeated by stronger evidence. Designed for both AI agents (JSON output) and humans (tables, interactive TUI).

## Features

- **Wikilink parsing** — `[[target]]`, `[[target|alias]]`, `[[target#heading]]`, `[[target^block-id]]`, `![[embeds]]`
- **Graph queries** — forward links, backlinks, multi-hop traversal, shortest path between pages
- **Vault diagnostics** — dead links, orphan pages, syntax errors, SPL parse errors
- **Full-text search** — content search with regex support, frontmatter/code-block awareness
- **Fuzzy matching** — SimHash-based page name similarity
- **Interactive TUI** — dashboard, page browser, link explorer, graph view, inline wikilink navigation
- **Page viewer** — Xanadu-inspired two-pane reader with context cards, bridge connectors, and wikilink navigation
- **Content-addressable blocks** — BLAKE3 Merkle leaves for headings, paragraphs, code blocks, and SPL
- **Incremental caching** — two-tier (mtime + hash) index for both wikilinks and reasoning theories
- **Agent-friendly** — JSON by default, structured errors, non-zero exit codes
- **Defeasible reasoning** — extract SPL facts and rules from Markdown, build a vault-wide theory, derive conclusions with full provenance
- **Proof trees** — explain why a conclusion holds, traced back to source files and line numbers
- **What-if analysis** — hypothetical reasoning: add temporary facts and see what changes
- **Abductive reasoning** — find what facts are needed to prove a goal
- **Conflict detection** — find unresolved logical contradictions with resolution suggestions
- **Cross-referencing** — link graph and logical theory annotate each other

## Install

Requires a Rust toolchain ([rustup](https://rustup.rs/)).

```bash
# Wikilink features only
make install

# With defeasible reasoning
cargo install --path . --features reason
```

Without `--features reason`, running `zetl reason` prints a helpful error instead of failing silently.

## Quick start

The included `demo-vault/` is a self-referential knowledge base about zetl itself, with wikilinks and SPL throughout.

```bash
# Build the link index
zetl -d ./demo-vault index

# Query links
zetl -d ./demo-vault links "Scanner"
zetl -d ./demo-vault backlinks "Cache" --depth 2

# Run reasoning over all SPL in the vault
zetl -d ./demo-vault reason status
zetl -d ./demo-vault reason explain "release-candidate" --format natural
zetl -d ./demo-vault reason conflicts
```

## Usage

### Wikilink commands

```bash
# Build or refresh the link index
zetl -d ./my-vault index

# Forward and back links
zetl -d ./my-vault links "Some Page"
zetl -d ./my-vault backlinks "Some Page"
zetl -d ./my-vault backlinks "Some Page" --depth 2    # multi-hop

# Find shortest path between pages
zetl -d ./my-vault path "Page A" "Page B"

# Search content
zetl -d ./my-vault search "query"
zetl search "pattern" --regex

# Validate vault
zetl -d ./my-vault check
zetl check --dead-links --fail-on error   # cwd is vault
zetl check --spl                        # SPL diagnostics only
zetl check --drift                      # detect SPL changes since last theory build

# Fuzzy page name matching
zetl -d ./my-vault similar "zettelkasen"

# Content-addressable blocks
zetl -d ./my-vault blocks "Some Page"                    # all blocks
zetl -d ./my-vault blocks "Some Page" --type heading     # headings only
zetl -d ./my-vault blocks --resolve abc123               # resolve by hash prefix

# Stats and export
zetl -d ./my-vault stats
zetl -d ./my-vault list
zetl -d ./my-vault export    # full graph as JSON

# Interactive TUI
zetl -d ./my-vault tui

# Page viewer (two-pane reader)
zetl -d ./my-vault view "Some Page"
zetl -d ./my-vault view                                  # opens page picker
zetl -d ./my-vault view "Some Page" --context-lines 10   # taller context cards
```

### Reasoning commands

Requires `--features reason` at build time. All commands operate on SPL extracted from ` ```spl ` fenced code blocks in Markdown files and standalone `.spl` files.

```bash
# What does the vault's combined theory conclude?
zetl -d ./demo-vault reason status
zetl reason status --positive              # only +D, +d conclusions
zetl reason status --literal "release*"    # wildcard filter

# Why does a conclusion hold? (proof tree with provenance)
zetl -d ./demo-vault reason explain "release-candidate"
zetl -d ./demo-vault reason explain "good-cli-tool" --format natural
zetl -d ./demo-vault reason explain "scanner-complete" --format dot

# Why can't something be proved?
zetl -d ./demo-vault reason why-not "docs-updated"

# What facts would make a goal provable?
zetl -d ./demo-vault reason require "release-candidate"
zetl -d ./demo-vault reason require "release-candidate" --assume "(given docs-updated)"

# Hypothetical: what if we add facts?
zetl -d ./demo-vault reason what-if "(given docs-updated)" --goal "release-candidate"
zetl -d ./demo-vault reason what-if --file extra.spl

# Find unresolved conflicts (the demo vault has a deliberate tension in Cache.md)
zetl -d ./demo-vault reason conflicts
zetl -d ./demo-vault reason conflicts --suggest --fail-on-conflicts

# Export the combined theory
zetl -d ./demo-vault reason export                         # JSON
zetl -d ./demo-vault reason export --format spl            # reconstructed SPL with provenance
zetl -d ./demo-vault reason export --with-conclusions

# Trace a conclusion back to source files
zetl -d ./demo-vault reason provenance "release-candidate"

# Cross-reference links with reasoning
zetl -d ./demo-vault links "Cache" --with-conclusions
zetl -d ./demo-vault backlinks "Reasoning Engine" --with-conclusions
```

All commands default to JSON output. Add `--format table` (or `--format natural`/`--format dot` for explain) for human-readable output.

## SPL in Markdown

Embed Spindle Lisp in any Markdown file using fenced code blocks:

````markdown
# Rust for CLI

zetl is written in Rust for type safety and fast startup.

```spl
(given type-safe)
(given single-binary)
(given fast-startup)
```

These facts feed into the vault-wide theory.
````

You can also place standalone `.spl` files anywhere in the vault:

```spl
; release-readiness.spl — rules that combine facts from across the vault
(normally r-good-cli
  (and fast-startup single-binary type-safe)
  good-cli-tool)
```

zetl merges all SPL from across the vault into a single theory, reasons over it, and traces every conclusion back to its source file and line number. The `demo-vault/` included in this repo is a working example — it documents zetl itself using both wikilinks and SPL.

### Conclusion types

| Tag  | Meaning |
|------|---------|
| `+D` | Definitely provable (strict rules, no defeaters possible) |
| `-D` | Definitely not provable |
| `+d` | Defeasibly provable (inferred, no active defeaters) |
| `-d` | Defeasibly not provable (blocked or no derivation path) |

## TUI

### Dashboard (`zetl tui`)

Multi-tab terminal interface for vault exploration:

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

### Page viewer (`zetl view`)

Xanadu-inspired two-pane reader for focused page navigation. The left pane renders the current note with numbered `[N]` anchor glyphs at each wikilink. The right pane shows context cards — excerpts from forward-linked pages. A bridge column connects anchors to their cards with colored connectors. Falls back to single-pane layout in narrow terminals (<60 cols).

```bash
zetl view "Page Name"                  # open a page
zetl view                              # open page picker
zetl view "Page Name" --context-lines 10 --main-width 60
```

| Key | Action |
|-----|--------|
| `j`/`k` | Scroll (or cycle links in focus mode) |
| `Ctrl-d`/`Ctrl-u` | Half-page scroll |
| `g`/`G` | Top / bottom of note |
| `Tab` | Toggle between scroll and focus mode |
| `Enter` | Navigate to focused link |
| `[`/`]` | Session history back / forward |
| `/` | Open page picker |
| `?` | Toggle keybindings help |
| `q` | Quit |

## Compatibility

Works with any Markdown vault using `[[wikilink]]` syntax:

- Obsidian
- Logseq
- Foam
- Dendron

SPL embedding is optional — vaults work fine with just wikilinks. zetl never modifies your files. The index and theory cache are disposable and stored in `.zetl/`.

## Development

```bash
make                    # fmt + clippy + build + test
make build              # debug build
make release            # release build
make test               # run tests
make test-reason        # run tests with reason feature
make check              # fmt + clippy
make fmt-fix            # auto-format code
make doc-open           # generate and open docs in browser
make clean              # remove build artifacts
```

## License

MIT
