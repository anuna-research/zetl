---
title: The Link Graph
tags: [concepts, graph, model]
---

# The Link Graph

ztl thinks about your vault as a **directed graph**: pages are nodes, wikilinks are edges. Every command you will run — `backlinks`, `path`, `orphans`, `export`, the graph widget in `ztl serve` — is a query over this graph. Understanding the model makes the rest of ztl obvious.

## What a node is

A **node** is one page: one `.md` file in your vault. The node's identity is its filename (minus `.md`, case-normalised). The node's content is what's between the frontmatter and the end of file. That's it — no IDs, no database keys.

## What an edge is

Every `[[wikilink]]` you write is an **edge** from the current page to the target. Edges are directed: `A.md` writing `[[B]]` creates an edge `A → B`, not `B → A`. Backlinks are the inverse — the set of pages that link *at* you.

The graph distinguishes:

- **Plain edges** — `[[Page]]`, `[[Page|alias]]`
- **Heading edges** — `[[Page#heading]]`
- **Block edges** — `[[Page^b3a9f1]]`, which target a specific [[Blocks|block]]
- **Embed edges** — `![[Page]]`, same edge, different rendering

All four contribute to forward-links and backlinks. An aliased link is still an edge to `Page`, not to `alias`.

## Dead links and orphans

Two graph concepts show up often enough to name:

- A **dead link** is an edge whose target has no matching file. The edge exists in the graph; the node on the far end is empty. `ztl check --dead-links` lists them.
- An **orphan** is a node with no inbound edges — nothing links *to* it. An orphan may still have outbound links. `ztl check --orphans` lists them.

Neither is an error. A dead link can be a stub you intend to fill in. An orphan can be a brand-new note waiting for its first backlink, or an index page whose incoming links are all from the sidebar. See [[Finding Orphans and Dead Links]] for when to care.

## Backlinks are the graph's secret

The real reason wikilink graphs are useful is the backlink view. Your outbound links capture what you were thinking about when you wrote the page. The inbound links capture what the rest of your vault thinks of it — often including connections you forgot you made. Every page in `ztl serve` has a backlinks pane for exactly this reason. See [[Backlinks]].

## Traversal

Because it's a graph, you can ask graph-shaped questions:

- **Multi-hop backlinks.** `ztl backlinks "Rust" --depth 2` walks two steps inward.
- **Shortest path.** `ztl path "Note A" "Note B"` finds the chain that connects two ideas, if one exists.
- **Following forward links.** `ztl links "Some Page"` is the outbound side. See [[Following Links]].
- **Full export.** `ztl export` dumps the whole graph as JSON for external tools.

## Incremental rebuild

The graph is not recomputed from scratch on every command. `ztl index` walks the vault once, and uses a two-tier cache keyed on **mtime + content hash**:

1. If the file's mtime matches the cache, assume unchanged — skip.
2. If the mtime changed, hash the content and compare. If the hash matches (mtime-only touch), skip parsing.
3. Otherwise, reparse the file, update its outbound edges, and invalidate downstream inverses.

The result: reindexing a vault after editing three files takes about as long as parsing three files, not the whole tree. The cache lives in `.ztl/` and is safe to delete.

## Staying out of your way

ztl never modifies a page to "fix" the graph. A renamed target stays a dead link until you rename its incoming references too. `ztl check` tells you what changed; the edit is yours. See [[Local-first]].

## Related

- [[Backlinks]]
- [[Finding Orphans and Dead Links]]
- [[Following Links]]
- [[Wikilinks]]
- [[Searching]]
