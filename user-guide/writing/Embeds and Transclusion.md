---
title: Embeds and Transclusion
tags: [writing, embeds, transclusion]
---

# Embeds and Transclusion

An embed pulls another page's content *into* the current page. A plain
wikilink just points. An embed quotes — at full resolution, live, without
duplicating the source.

## The three embed forms

```markdown
![[Zettelkasten Method]]              # the whole page
![[Zettelkasten Method#History]]       # one section (by heading)
![[Seeing Like a State^legibility-def]] # one block (by block id)
```

The leading `!` is what turns a link into an embed. Everything else mirrors
the plain wikilink forms — see [[Wikilinks]].

## How embeds render

When you run `ztl serve` or `ztl build`, embeds render as **transclusion
cards** in a right-rail panel alongside the main content. The surrounding
prose keeps flowing; the embedded material appears next to the paragraph that
references it, in its own framed card showing the source page, the
quoted region, and a link back to the original.

In the terminal viewer (`ztl view`), embeds render in the two-pane layout,
with the source content expanded where the `![[...]]` appears — see
[[Terminal Viewer]].

## When to embed, when to link

| Situation | Use |
|---|---|
| You're pointing at related material | plain link `[[...]]` |
| You want the reader to *see* the quoted passage without leaving | embed `![[...]]` |
| You need a definition, claim, or diagram from elsewhere | embed a block `![[Page^id]]` |
| You're writing a meta-page (index, reading list, MOC) | mostly links |
| You're writing an essay that quotes your own notes | embeds for quoted material |

A good heuristic: embed when you'd otherwise be tempted to copy-paste.
Embeds stay in sync if the source changes; copies rot.

## A worked example

`Book Journal/Seeing Like a State.md`:

```markdown
---
title: Seeing Like a State
tags: [book, political-theory]
---

# Seeing Like a State

James C. Scott's central move is to define legibility as a prerequisite for
state power. The core definition is worth pinning down:

> State simplifications are static typifications that abstract away
> from local knowledge in the service of administrative control.
> ^legibility-def

Taylorism extends this into the workplace. See [[Scientific Management]].
```

`Essays/Legibility and Craft.md`:

```markdown
---
title: Legibility and Craft
tags: [essay, draft]
---

# Legibility and Craft

Scott's definition is the one I keep returning to:

![[Seeing Like a State^legibility-def]]

What Taylor did in factories, the state had already been doing to
forests, peasant villages, and names. The parallel isn't incidental.
```

The essay quotes the book without duplicating the passage. Update the
definition in the book note and every essay that embeds it picks up the
change on the next `ztl build`.

## For theme authors: `page.transclusion_cards`

If you're writing a custom theme, the pre-rendered transclusion panel is
available as `page.transclusion_cards` in the template context — a string of
HTML ready to drop into your layout with `| safe`. See
[[Customising the Look]] for theming, and [[Frontmatter Fields]] for the full
template variable list.

## Related

- [[Wikilinks]]
- [[Headings and Blocks]]
- [[Linking Pages]]
- [[Customising the Look]]
- [[Blocks]]
