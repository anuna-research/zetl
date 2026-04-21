---
title: Headings and Blocks
tags: [writing, links, blocks, headings]
---

# Headings and Blocks

A plain wikilink takes a reader to a page. On a long page, that's a coarse
target — you may have wanted to point at one section, or one sentence. zetl
supports both.

## Heading links

Use `#` to point at a heading within a page:

```markdown
See [[Annual Review 2026#Priorities]] for the three-item list.
The argument is developed in [[Seeing Like a State#Legibility]].
```

Matching rules:

- **Case-insensitive.** `#Priorities`, `#priorities`, and `#PRIORITIES` all
  match `## Priorities`.
- **Slugified.** Spaces and punctuation are normalised, so
  `#getting started` matches `## Getting Started`.
- **Any heading level.** `#` in the link matches `##`, `###`, and so on — the
  link just names the heading text.

If multiple headings on a page share the same text, the first one wins.
That's rare in practice; rename one of them if it bites you.

## Block ids

When you want to point at something finer than a heading — a definition, a
quoted passage, a data row — use a **block id**. Append `^some-id` to the
end of the block, then link with `[[Page^some-id]]`:

```markdown
The central definition in the book:

> State simplifications are static typifications that abstract
> away from local knowledge in the service of administrative
> control.
> ^legibility-def

From another page:

Scott's definition of legibility — [[Seeing Like a State^legibility-def]] —
maps cleanly onto what Taylor was doing in factories.
```

Block ids can live on any paragraph, blockquote, list item, code block, or
table. Convention:

- Lower-case, hyphenated: `^legibility-def`, `^thesis-claim`, `^q3-numbers`
- Short and meaningful
- Unique within the page

## Why block links matter for long notes

Three situations where block ids pay off:

**Reference material.** A long glossary entry, a canonical definition, a
table of constants — block ids let other pages quote them surgically without
copying.

**Evolving claims.** A key argument on a draft page gets revised over time.
If other pages embed it via `![[Page^id]]` (see [[Embeds and Transclusion]]),
every quoting page updates automatically.

**Citations to your own notes.** In a research vault, you'll often want to
point at "the thing Alice said on 2026-03-12" — give that paragraph a block
id like `^alice-crdt-comment` and every downstream note can cite it.

## Stable under edits

Block ids are stable even if the block's surrounding page changes. Under the
hood, zetl assigns each block a content-addressed identity by hashing its
text into a Merkle tree (see [[Blocks]]). When you add a block id, zetl
anchors future links to that block's identity — reorder the page, add new
paragraphs, rewrite the intro, and `[[Seeing Like a State^legibility-def]]`
still resolves to the same block.

You can list a page's blocks (with their hashes) using:

```bash
zetl blocks "Seeing Like a State"
zetl blocks "Seeing Like a State" --type blockquote
```

And resolve a block by hash prefix:

```bash
zetl blocks --resolve 7a3f9c
```

Useful when you're debugging a transclusion or writing a reasoning rule
(see [[Running Queries]]) against a specific claim.

## Combining: heading + block

A block id wins if the page has both. Link to the heading for "this section,"
to the block id for "this specific passage." Don't try to combine them
(`Page#Section^id`) — zetl parses one or the other.

## Related

- [[Wikilinks]]
- [[Blocks]]
- [[Embeds and Transclusion]]
- [[Linking Pages]]
- [[The Link Graph]]
