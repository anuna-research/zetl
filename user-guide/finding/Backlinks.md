---
title: Backlinks
tags: [finding, backlinks, graph]
---

# Backlinks

`ztl backlinks "Page"` lists every page that links **to** `Page`. It is the quiet workhorse of a wikilink system — more useful, over time, than the links you write yourself.

```bash
ztl backlinks "Zettelkasten Method"
```

Output: the file path, the line of the match, and (with `--context N`) a snippet of the surrounding prose.

## Why backlinks matter

Forward links are a claim you made at the time of writing. A backlink is every claim anyone ever made that **referred back to** this idea — including you, six months later, with more context. That asymmetry is the whole research payoff:

- You read a paper and start a note on it. A year later the note has eight backlinks: three from project docs, two from meeting minutes, one from a book review. You did not plan any of that. The graph assembled it.
- You are revisiting an old concept page. Instead of re-reading it in isolation, you walk its backlinks — every context in which you once found it relevant. That is a research tool disguised as a list.

Forward links (see [[Following Links]]) answer *what does this page talk about?* Backlinks answer *what talks about this page?* The second question is almost always more interesting for notes older than a few weeks.

## Multi-hop with `--depth`

By default `ztl backlinks` returns direct, one-hop references. `--depth N` walks the reverse graph:

```bash
# Direct backlinks only
ztl backlinks "Zettelkasten Method"

# Every page that reaches "Zettelkasten Method" in up to 3 reverse hops
ztl backlinks "Zettelkasten Method" --depth 3
```

Depth 2–3 is where the transitive structure gets interesting: you find the notes that cite notes that cite this concept. Depth 5+ tends to return most of your vault — the graph is small-world.

## Example: walking backwards from a concept

You have a page called `CRDT`. You want to see every context in which you once thought CRDTs were relevant, so you can decide whether your 2026 research plan should build on them.

```bash
# Everyone who pointed at CRDT directly
ztl backlinks "CRDT" --context 80

# The second-order neighbourhood: cites that cite
ztl backlinks "CRDT" --depth 2
```

Read the backlinks in the order printed (depth-first, then alphabetical). For each one, ask: *did I mean the same thing by CRDT here as I do now?* You will almost always find one or two notes where you used the term loosely, and one or two where you made a stronger claim than you remembered.

## Useful flags

| Flag | Use |
|------|-----|
| `--context 80` | Show 80 characters of snippet around each link |
| `--depth 2` | Multi-hop reverse traversal |
| `--fuzzy` | Tolerate typos in the page name |
| `--with-conclusions` | Show SPL conclusions each linker contributes (requires `--features reason`) |

See [[CLI Overview]] for the rest.

## JSON output

Piped output auto-switches to JSON. Useful for feeding backlink lists to an AI agent, or building "related reading" sections programmatically:

```bash
ztl backlinks "2026 Research Plan" --json \
  | jq -r '.backlinks[].page'
```

## Time travel

`--at "last monday"` runs the query against a past snapshot. Requires `--features history`. Answer: *whose backlinks did this page have before I started the refactor?* See [[Time Travel]].

## Related

- [[Following Links]]
- [[The Link Graph]]
- [[Searching]]
- [[Finding Orphans and Dead Links]]
