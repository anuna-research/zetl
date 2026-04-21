---
title: Wikilinks
tags: [concepts, wikilinks, syntax]
---

# Wikilinks

A **wikilink** is how you point from one note to another by name. zetl parses five shapes of wikilink from your Markdown. This page is the syntax reference; for workflow advice see [[Linking Pages]].

## The five shapes

| Shape | Example | Purpose |
|-------|---------|---------|
| Plain | `[[Zettelkasten Method]]` | Link to a page; display text is the target name |
| Aliased | `[[Zettelkasten Method\|the slip-box method]]` | Link with custom display text |
| Heading | `[[Zettelkasten Method#Origins]]` | Link to a specific heading on a page |
| Block | `[[Zettelkasten Method^b3a9f1]]` | Link to a specific content-addressable block |
| Embed | `![[Zettelkasten Method]]` | Transclude the target's content inline (see [[Embeds and Transclusion]]) |

All five can be combined with a heading or block anchor on the target side: `![[Zettelkasten Method#Origins]]` embeds a single section.

## How resolution works

When you write `[[Some Page]]`, zetl does not look up a URL or an ID. It looks for a file named `Some Page.md` in your vault. Specifically:

- **By filename**, not by the `# H1` inside the file. The name in brackets must match the file stem.
- **Case-insensitive**. `[[some page]]`, `[[SOME PAGE]]`, and `[[Some Page]]` all resolve to `Some Page.md`.
- **Across subfolders**. zetl searches the whole vault tree; you don't write `[[projects/zetl]]`, just `[[zetl]]`.
- **Slugified** for the web output — `Some Page.md` becomes `/page/some-page/` in `zetl serve` and `zetl build`. You never need to think about slugs when writing; that's what the resolver is for.

If two files share a name across folders, the first match wins and the other is reported as ambiguous by `zetl check`. Rename one.

## Dead links are fine (until they're not)

A wikilink to a page that does not yet exist is a **dead link**. zetl does not crash, does not refuse to render, and does not auto-create the file. It records the edge and surfaces it in:

```bash
zetl check --dead-links
```

Many people use this as a to-do list: write what you wish existed, then fill in the targets later. See [[Finding Orphans and Dead Links]].

## Examples from a real note

```markdown
# Research on Note-Taking

I've been reading about [[Zettelkasten Method]] — especially the idea of
[[Zettelkasten Method#atomic notes|atomic notes]]. One concrete claim from
[[Luhmann's Workflow^b3a9f1]] is worth pulling in verbatim:

![[Luhmann's Workflow^b3a9f1]]

See also [[Evergreen Notes]] and [[Andy Matuschak]].
```

This note has five outbound edges, one of which is an embed. If `Evergreen Notes.md` doesn't exist yet, the link stays in place; `zetl check` will nag you politely.

## What wikilinks are not

- **Not standard Markdown.** `[[...]]` is an extension. zetl, Obsidian, Logseq, and a few others understand it; vanilla CommonMark does not. If you need to publish with a tool that doesn't, `zetl build` turns wikilinks into regular `<a href>` links for you.
- **Not case-sensitive.** Good: no one remembers capitalisation. If you prefer strict matching, name your files consistently.
- **Not paths.** `[[Some Page]]` is not a file path. Do not write `[[./notes/Some Page.md]]` — it won't work, and you do not need to.

## Related

- [[Linking Pages]]
- [[Embeds and Transclusion]]
- [[The Link Graph]]
- [[Headings and Blocks]]
- [[Finding Orphans and Dead Links]]
