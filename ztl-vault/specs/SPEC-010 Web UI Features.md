# SPEC-010: Web UI Features

Exploration of features that could be built into the `ztl serve` and `ztl build` web interfaces. The CLI is remarkably deep (22 commands, Merkle trees, defeasible reasoning, graph traversal), but the web UI is essentially a pretty markdown viewer with transclusion. There's a lot of headroom to expose that power through the browser.

```spl
(given spec-010-documented)
```

## Features worth building in

### 1. Full-text search (high impact, low effort)

The CLI has `ztl search` with regex, case sensitivity, glob filters, and context snippets. The web UI has no search bar at all. This is probably the single biggest usability gap — a search input in the sidebar or a `Cmd+K` palette would transform navigation.

For `ztl serve`, search can hit an API endpoint backed by `search_vault()`. For `ztl build` static sites, search needs a client-side approach — either a pre-built JSON index shipped with the static output, or a WASM-compiled searcher.

### 2. Graph visualization (high impact, medium effort)

The [[TUI]] has a graph tab. The CLI can `ztl export` the full link graph as JSON. A force-directed graph view (d3-force or similar) in the browser would let users visually explore connections — clickable nodes, hover previews, cluster detection. This is a flagship feature for tools like [[Obsidian]].

### 3. Multi-hop link exploration (medium impact, low effort)

`ztl links` and `ztl backlinks` support `--depth N` for BFS traversal. The web UI only shows depth-1 links. An expandable tree or "explore" mode that lets you click deeper (2-hop, 3-hop neighbors) would surface the graph structure without a full visualization.

### 4. Shortest path finder (medium impact, low effort)

`ztl path <from> <to>` finds the shortest wikilink path between two pages. A simple "How are these connected?" UI — pick two pages, see the chain — is compelling and easy to build on top of the existing endpoint.

### 5. Vault diagnostics dashboard (medium impact, medium effort)

`ztl check` reports dead links, orphan pages, syntax errors, and SPL drift. A diagnostics panel (or page) showing these as a checklist with clickable links to the offending pages/lines would help vault hygiene.

### 6. Fuzzy / similar page suggestions (medium impact, low effort)

`ztl similar` uses SimHash to find pages with similar content. Could surface this as a "Related pages" section below content, or as suggestions when creating new pages ("Did you mean...?").

### 7. Reasoning UI (high impact, high effort, requires `--features reason`)

This is ztl's most unique differentiator — SPL [[Defeasible Reasoning]] embedded in markdown. Currently zero web exposure. Possible surfaces:

- **Conclusions panel:** show what the vault's SPL blocks currently prove/disprove
- **Proof tree visualization:** `reason explain` outputs dot graphs — render them inline
- **What-if sandbox:** type hypothetical SPL and see how conclusions change
- **Conflict alerts:** `reason conflicts` could show as warnings on affected pages

### 8. Content-addressable block references (low-medium impact, medium effort)

The [[Merkle Tree]] system (`ztl blocks`) gives every heading, paragraph, code block, etc. a BLAKE3 hash. Could enable:

- Permalink to any block via hash
- "This block changed since you last visited" indicators
- Block-level transclusion (embed a specific paragraph from another page)

### 9. Watch mode / live reload (medium impact, low effort)

[[SPEC-008 Watch Mode]] specifies NDJSON events on file changes. Wiring this to a WebSocket or SSE stream would give live reload — edit in Obsidian/VS Code, see changes instantly in the browser. The reindex-on-save already exists for the editor; this extends it to external edits.

### 10. Command palette (medium impact, medium effort)

A `Cmd+K` / `Ctrl+K` overlay combining search, page navigation, and commands (check vault, find path, show stats) — making the CLI power accessible without the CLI.

## Suggested priority

| Tier | Feature | Why |
|------|---------|-----|
| P0 | Search | Table stakes — every knowledge tool needs it |
| P0 | Live reload (watch) | Small effort, big quality-of-life win |
| P1 | Graph visualization | Visual differentiator, users expect it |
| P1 | Diagnostics dashboard | Makes vault health actionable |
| P1 | Command palette | Unifies access to everything |
| P2 | Multi-hop exploration | Deepens the browsing experience |
| P2 | Shortest path | Fun, useful, unique |
| P2 | Similar pages | Low-effort discoverability boost |
| P3 | Reasoning UI | Most unique feature but niche audience, high effort |
| P3 | Block permalinks | Powerful but complex UX |

## Static site considerations

Features that need special handling for `ztl build` (no server):

| Feature | Serve approach | Build approach |
|---------|---------------|----------------|
| Search | API endpoint (`/api/search`) | Pre-built JSON index + client-side search (e.g. Fuse.js, lunr, or custom) |
| Live reload | SSE / WebSocket | N/A (static) |
| Graph | API or inline JSON | Inline JSON blob in page, client-side d3 rendering |
| Diagnostics | API endpoint | Pre-rendered HTML page at build time |
| Shortest path | API endpoint | Pre-computed paths or client-side BFS on inline graph JSON |
