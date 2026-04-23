---
title: Blocks
tags: [concepts, blocks, merkle]
---

# Blocks

ztl breaks every page into **blocks** — headings, paragraphs, code fences, lists, tables, blockquotes, and SPL fragments — and gives each block a stable hash. This turns a page into a small Merkle tree of content, which is what makes `[[Page^block-id]]` links work, what makes the incremental cache fast, and what will later make sync possible.

## What a block is

A block is the smallest unit of page content ztl tracks. Parsing a page yields a sequence of typed blocks:

| Type | Example |
|------|---------|
| `heading` | `## Origins` |
| `paragraph` | Regular prose, one logical paragraph per block |
| `code` | A fenced `` ``` `` code block (any language) |
| `spl` | A fenced ` ```spl ` block (only with `--features reason`) |
| `list` | A top-level list; nested items stay inside |
| `table` | A pipe or grid table |
| `blockquote` | A `> …` quoted section |
| `frontmatter` | The YAML header, if present |

List what's in a page:

```bash
ztl blocks "Zettelkasten Method"
ztl blocks "Zettelkasten Method" --type heading
```

## Why content-addressable

Every block's content is hashed with [BLAKE3](https://github.com/BLAKE3-team/BLAKE3), and the whole page is a Merkle tree with block hashes as leaves. "Content-addressable" means **the hash identifies the content, not a location**. This has three consequences that matter to you.

### 1. Block links survive file moves

`[[Some Page^b3a9f1]]` links to the block whose hash starts with `b3a9f1`. Rename `Some Page.md` to `Renamed Page.md` and the hash doesn't change — the block is the same content. ztl can still resolve the link. It keeps pointers to *what you said*, not to *where it lived*.

Resolve a hash back to its source:

```bash
ztl blocks --resolve b3a9f1
```

Paste that hash from anywhere (another page, a chat log, a commit message) and get the block back.

### 2. Incremental caching is cheap

When you reindex, ztl compares block hashes to the cached ones and skips anything unchanged. Edit one paragraph in a 300-block page, and only that one paragraph's block is re-parsed; the other 299 hashes match the cache, no work. That's how the two-tier index (see [[The Link Graph]]) stays fast on vaults with thousands of pages.

### 3. Deduplication and diffs are natural

Two blocks with identical content hash the same — whether they live on the same page, different pages, or different vaults. That's useful for:

- Finding where a quote or snippet has been reused.
- Diffing a page against a past snapshot (see [[Page History]]) — unchanged blocks line up trivially.
- Future sync between vaults: the same block on both sides doesn't need to cross the network.

## How block links are written

When you link to a block you rarely type the hash by hand. The common flow is:

1. Open a page in `ztl serve`; each block has an anchor with its short hash.
2. Copy the `[[Page^hash]]` form from the anchor.
3. Paste into your current note.

Obsidian's `^block-id` syntax is supported on read — if you imported a vault from Obsidian, those ids still resolve. See [[Migrating from Obsidian]].

## A worked example

A page `Luhmann's Workflow.md` with three paragraphs, each hashed independently:

```
heading     a11fce  "# Luhmann's Workflow"
paragraph   b3a9f1  "He wrote each idea on one slip..."
paragraph   c7e402  "The slip-box was organised by..."
paragraph   d2510a  "He cross-referenced slips by..."
```

Another note can now embed one paragraph, not the whole page:

```markdown
Luhmann's own summary:

![[Luhmann's Workflow^b3a9f1]]
```

Edit paragraph two; `b3a9f1` becomes something else, and the embed now points at the *new* content at that slot on the page. Block hashes track content; block *anchors* track position within a page. Both are useful, neither is magic.

## Related

- [[Headings and Blocks]]
- [[Embeds and Transclusion]]
- [[The Link Graph]]
- [[Page History]]
- [[Wikilinks]]
