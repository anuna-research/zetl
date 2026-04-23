# Link Graph

ztl builds a directed graph where nodes are pages and edges are [[concepts/Wikilinks]]. The graph powers all link-based queries.

```spl
(given directed-graph)
(given multi-hop-traversal)
```

## Data model

- **Node** — a Markdown page, identified by its title (filename without `.md`, case-insensitive)
- **Edge** — a wikilink from one page to another, with optional alias, heading, or block reference

The graph is built by the [[Scanner]] and cached by [[architecture/Cache]] for incremental re-scans.

## Resolution

Page names are resolved case-insensitively. When a wikilink target like `[[cache]]` appears, ztl matches it to `Cache.md` regardless of casing. For disambiguation between pages with the same name in different directories, use path-qualified links like `[[architecture/Cache]]`.

## Queries

| Command | Description |
|---------|-------------|
| `links` | Forward links from a page, with configurable depth |
| `backlinks` | Pages linking to a target, with depth traversal |
| `path` | Shortest link path between any two pages |
| `export` | Full graph as JSON |

These are exposed through [[Links Command]], [[Path Command]], and [[List and Export Commands]]. With the `--with-conclusions` flag, link results are cross-referenced with the [[Reasoning Engine]] to show what each linked page contributes to the vault's logic.

## Library

Built on `petgraph`, a Rust graph data structure library. The graph is stored as a flat adjacency list for cache-line efficiency during multi-hop traversal. See [[Performance]].

See also: [[Scanner]], [[Links Command]], [[Check Command]], [[concepts/Wikilinks]]
