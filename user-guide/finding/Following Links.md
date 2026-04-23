---
title: Following Links
tags: [finding, graph, traversal]
---

# Following Links

Three commands walk the forward-link graph: `ztl links` for a page's neighbours, `ztl path` for the shortest route between two pages, and `ztl export` for dumping the whole graph as JSON. Together they turn your vault into a navigable graph you can query from the shell.

## `ztl links` — forward neighbours

```bash
ztl links "Zettelkasten Method"
```

Prints every page that `Zettelkasten Method` links **to**. This is the complement of [[Backlinks]]: forward links are what this page cites; backlinks are what cites it.

Walk further with `--depth`:

```bash
# Direct forward links
ztl links "2026 Research Plan"

# Two hops: pages linked from pages linked from the plan
ztl links "2026 Research Plan" --depth 2
```

Useful flags:

| Flag | Use |
|------|-----|
| `--depth N` | Multi-hop forward traversal |
| `--context 80` | Show surrounding prose of each link |
| `--fuzzy` | Tolerate typos in the source page name |

See [[CLI Overview]] for the rest.

## `ztl path` — shortest route between two pages

```bash
ztl path "CRDT" "Zettelkasten Method"
```

```
CRDT
-> Collaborative Editing
-> Knowledge Management
-> Zettelkasten Method
```

This answers *how is X connected to Y?* — a question that comes up constantly when you are trying to explain a decision ("why did I end up thinking about Luhmann while researching CRDTs?"). ztl performs an undirected BFS over wikilinks and returns the first shortest path.

Tune the search with `--max-depth N` (default 10). Raise it for large vaults where the connection is genuinely distant; lower it to fail fast when you suspect there is no path at all.

```bash
ztl path "2026 Research Plan" "Dream Journal 2019-03-04" --max-depth 6
# no path found within 6 hops
```

No path found is a meaningful answer: these two notes live in disconnected components of your graph, and that is a real fact about your thinking.

## `ztl export` — the full graph as JSON

```bash
ztl export > graph.json
```

Writes the entire vault graph: every page, every edge, every frontmatter block. This is the integration point for everything outside ztl — graph-viz tools, AI agents, custom dashboards, academic network analysis scripts.

The schema is stable and documented at [[CLI Overview]]. A trimmed view:

```json
{
  "pages": [
    { "name": "Zettelkasten Method", "path": "concepts/Zettelkasten Method.md",
      "frontmatter": { "tags": ["method"] },
      "links_out": ["Luhmann", "Index Card"],
      "links_in":  ["2026 Research Plan", "Note-Taking Comparison"] }
  ],
  "edges": [
    { "from": "Zettelkasten Method", "to": "Luhmann", "kind": "wikilink" }
  ]
}
```

### Practical uses

**Graph visualisation.** Pipe the export into Gephi, Cytoscape, or any force-directed tool:

```bash
ztl export --json \
  | jq '{nodes: [.pages[] | {id: .name}],
         edges: [.edges[] | {source: .from, target: .to}]}' \
  > cytoscape.json
```

**AI agents.** Feed the full graph to an agent so it can ground answers in your own note topology. The MCP server ([[MCP Server]]) exposes a thinner version of the same data.

**Diffing graphs over time.** Combine with `--at` (requires `--features history`) to compare your graph now vs. a month ago:

```bash
ztl export --at "30 days ago" > graph-last-month.json
ztl export > graph-now.json
# whatever diff tool you like
```

## Example: explain a decision

You made a call in `2026 Research Plan` to build on CRDTs. A colleague asks why. You retrace the chain:

```bash
ztl path "2026 Research Plan" "CRDT"
# 2026 Research Plan
# -> Collaborative Tools Review
# -> Real-time Sync
# -> CRDT
```

Each hop is a note you wrote with its own reasoning. Open them in order with `ztl view` and you have the audit trail — not reconstructed from memory, but from the links you dropped as you thought.

## Related

- [[Backlinks]]
- [[The Link Graph]]
- [[Searching]]
- [[MCP Server]]
