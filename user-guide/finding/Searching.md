---
title: Searching
tags: [finding, search, cli]
---

# Searching

`ztl search` scans your vault for a string and prints the matches, with context. It is deliberately small: a fast text finder for when you remember a phrase but not which note it lives in.

```bash
ztl search "attention is all you need"
```

No regex engine, no query DSL. If you want regex, pipe the raw index through `rg`. What ztl gives you instead is graph-aware narrowing.

## The two flags you will actually use

| Flag | What it does |
|------|--------------|
| `--near "Zettelkasten Method"` | Only match pages within `--depth` hops of that page |
| `--case-sensitive` | Exact case, no folding |

`--near` is the one most people miss. A keyword search over 4,000 notes gives you noise; a search restricted to "notes that cite *Zettelkasten Method* or one link away" gives you signal.

```bash
# Every note mentioning "CRDT" anywhere in the vault
ztl search "CRDT"

# Every note about CRDTs within two hops of my reading list
ztl search "CRDT" --near "2026 Research Plan" --depth 2
```

Other useful flags:

- `--path "journal/**"` — glob filter on file paths.
- `--context 120` — widen the snippet from the default 40 characters.
- `--limit 200` — default is 50; raise for grep-replace workflows.

See [[CLI Overview]] for the full list.

## JSON when piped

When stdout is a pipe or redirect, `ztl search` auto-switches to JSON. This is the contract that lets you chain searches into scripts:

```bash
ztl search "defeasible" --json \
  | jq -r '.results[] | select(.tags | contains(["active"])) | .page'
```

You can force JSON with `--json` on a TTY, and force the human table with `-f table`.

## Example: research workflow

You are writing a paper and want every note you tagged `active` that mentions CRDTs. Combine search with frontmatter filtering in jq:

```bash
ztl search "CRDT" --json \
  | jq -r '.results[]
           | select(.frontmatter.tags // [] | index("active"))
           | "\(.page)  \(.snippet)"'
```

Two lines, no database. The `--json` payload includes the page name, the match snippet, path, line number, and parsed frontmatter — enough to build any filter you want outside ztl.

## Semantic search (optional)

If you built ztl with `--features semantic`, two extra flags appear:

- `--semantic` — pure vector search, ranked by cosine similarity. Good for "find notes about this *idea*, even if they use different words."
- `--hybrid` — reciprocal-rank fusion of BM25 and vector results. This is usually what you want: the precision of keyword search with the recall of embeddings.

```bash
ztl search "spaced repetition" --hybrid --limit 20
```

Without the feature flag, both options error out with a build hint. See [[Installation]].

## Time travel

`--at "3 days ago"` runs the search against a historical snapshot. Requires `--features history` and a vault you have been snapshotting with `ztl watch`. See [[Time Travel]].

## Related

- [[Backlinks]]
- [[Similar Pages]]
- [[Following Links]]
- [[CLI Overview]]
