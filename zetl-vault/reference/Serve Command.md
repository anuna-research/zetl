# Serve Command

`zetl serve` starts a local web server for browsing the vault in a browser.

## Usage

```bash
zetl -d ./my-vault serve              # http://localhost:3000
zetl -d ./my-vault serve --port 8080
```

## Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--port N` | 3000 | Port to listen on |

## Features

The web UI provides:

- **Rendered pages** — Markdown rendered to HTML with syntax highlighting
- **Sidebar** — page list with search/filter
- **Backlink list** — pages linking to the current page
- **Transclusion panel** — forward-link excerpt cards with SVG bridge connectors, mirroring the [[View Command]] design
- **Inline editing** — edit a page in the browser and save; zetl re-indexes on save

## Relationship to other viewers

| Viewer | Interface | Read-only? |
|--------|-----------|------------|
| `zetl tui` | Terminal (multi-view dashboard) | Yes |
| `zetl view` | Terminal (two-pane reader) | Yes |
| `zetl serve` | Browser (full web UI) | No (inline editing) |
| `zetl build` | Static HTML (deployable) | Yes |

## History API

When built with `--features history`, the serve mode exposes additional API endpoints:

| Endpoint | Description |
|----------|-------------|
| `GET /api/history` | Graph-level delta log |
| `GET /api/history/page/:name` | Page evolution timeline |
| `GET /api/history/at?expr=<time>` | Resolve a time expression to snapshot metadata |
| `GET /api/history/diff?from=<expr>&to=<expr>` | Diff between two time expressions |

See [[History Command]] for details.

## Design

The server uses the same [[Link Graph]] and [[architecture/Cache]] as all other commands. The transclusion panel implements the same Xanadu-inspired design as [[View Command]], but with SVG bridge connectors instead of terminal graphics. See [[Xanadu Lineage]] for the design philosophy.

See also: [[CLI Reference]], [[Build Command]], [[View Command]], [[TUI]], [[History Command]]
