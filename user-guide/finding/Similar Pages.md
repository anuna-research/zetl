---
title: Similar Pages
tags: [finding, simhash, names]
---

# Similar Pages

`ztl similar "Query"` finds pages whose **titles** are near-matches for the query string. It uses SimHash over page names, which makes it excellent at three things: recovering from typos, spotting duplicate-ish pages you meant to merge, and suggesting links as you type.

```bash
ztl similar "Zetelkasten Method"
```

```
Zettelkasten Method        (distance 3)
Zettelkasten Workflow      (distance 7)
Luhmann's Method           (distance 11)
```

The number is **Hamming distance** on the hash — smaller is more similar. Zero would be identical.

## What SimHash does, briefly

SimHash is a locality-sensitive hash: feed it a string, get back a fixed-length binary fingerprint such that similar strings produce similar fingerprints. Unlike a cryptographic hash (where flipping one letter scrambles the whole output), SimHash changes only a few bits when the input changes slightly. Comparing two hashes by Hamming distance — the count of differing bits — gives you a cheap similarity score without storing the originals.

ztl computes one hash per page name at index time. `ztl similar` then scans them and returns the closest N. No full-text comparison, no embedding model, no network — just arithmetic over bits. It finishes in under a millisecond for vaults of tens of thousands of pages.

The tradeoff: it only sees **names**. A page titled `Dogs` and a page titled `Canine Companions` are not similar to SimHash, even though they are the same topic. For content similarity, use `ztl search --semantic` (see [[Searching]]).

## The three flags

| Flag | Default | Use |
|------|---------|-----|
| `--threshold N` | `12` | Max Hamming distance to report. Lower = stricter |
| `--limit N` | `10` | Max results |
| `--json` | off | Force JSON output |

The default threshold of 12 is tuned to catch typos and near-duplicates without flooding the output. Lower it to 6 if you only want obvious matches; raise it to 20 to see the long tail.

## When to use it

**Recovering from typos.** You vaguely remember a page name, try a spelling, and ztl says "no such page."

```bash
ztl links "Zetelkasten Method"
# error: page not found

ztl similar "Zetelkasten Method"
# Zettelkasten Method  (distance 3)
```

**Catching duplicates.** Over time you accumulate `Deep Work`, `Deep Work (book)`, and `Deep Work Notes`. A periodic `ztl similar` sweep on your top-linked pages surfaces these before the graph splits in half.

```bash
ztl similar "Deep Work" --threshold 8
```

**Suggesting links while writing.** Before creating a new page, run `ztl similar "Proposed Title"`. If there is a page at distance 4, you probably meant to link to it, not spawn a sibling.

## Example: weekly duplicate audit

A one-liner to find every near-duplicate pair in the vault, suitable for a Sunday cleanup:

```bash
ztl list --json \
  | jq -r '.pages[].name' \
  | while read -r p; do
      ztl similar "$p" --threshold 6 --limit 3 --json \
        | jq -r --arg p "$p" '.matches[] | select(.name != $p) | "\($p)  ~  \(.name)  [\(.distance)]"'
    done \
  | sort -u
```

Run it once a month; act on the ones you recognise. Most will be false positives (e.g. `2026-04-01` and `2026-04-02` share nearly every bit) — the ones that matter will stand out.

## Related

- [[Searching]]
- [[Linking Pages]]
- [[Finding Orphans and Dead Links]]
- [[CLI Overview]]
