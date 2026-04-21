---
title: Linking Pages
tags: [writing, links, wikilinks]
---

# Linking Pages

Links are how your vault stops being a folder of files and starts being a graph.
This page covers the everyday workflow of linking — the full syntax reference
lives in [[Wikilinks]].

## Link as you write

The honest workflow: while drafting a note, when you reach a concept that
deserves its own page, just wrap it in double brackets.

```markdown
The book makes the case that [[Scientific Management]] hollowed out
craft knowledge in a way that [[Seeing Like a State]] would later
describe as legibility.
```

Two things happen:

1. If `Scientific Management.md` already exists, the link resolves — clicking
   it in `zetl view` or `zetl serve` navigates to the page.
2. If it doesn't exist, zetl flags it as a **dead link** but otherwise leaves
   you alone. The link is a promise, not an error.

Dead links are surfaced by `zetl check --dead-links`:

```bash
$ zetl check --dead-links
Scientific Management — referenced from: Taylorism and its Discontents.md:7
```

You can create the target page any time. Run `zetl index` (or let `zetl serve`
re-scan) and the link goes live.

## Creating the target page later

This is the rhythm most writers settle into:

1. Write your note. Link freely to pages that don't exist yet.
2. When the dead-link list gets long enough to bother you, pick one.
3. Open a file of that name and start writing the page.

In `zetl serve`, clicking a dead link opens a "new page" stub — the template
sees `page.is_new = true` and can render a "start writing" affordance. In
`zetl view`, a dead link shows as such but won't navigate.

You're never *required* to fill in a dead link. Some dead links are
aspirational and can stay that way for years.

## Renaming pages safely

zetl resolves links by *filename*, so if you rename a page, every existing
link to its old name turns into a dead link. Two approaches:

**Rename and fix up.** Rename the file, then `zetl check --dead-links` to find
every reference to the old name. Replace them with the new name in your
editor's find-and-replace.

**Leave a redirect.** Keep a short stub at the old filename:

```markdown
---
title: Old Name
---

# Old Name

Moved to [[New Name]].
```

Either works. The second is friendlier if external readers may have the old
URL.

## Link aliases

Sometimes the canonical page name reads awkwardly in a sentence. Use the pipe
syntax:

```markdown
Before: The [[Annual Review 2026]] sets out three priorities.
After:  This year's [[Annual Review 2026|annual review]] sets out three priorities.
```

The link still points to `Annual Review 2026.md`. Only the displayed text
changes. Aliases are good for:

- Grammatical smoothness (`[[Scientific Management|Taylorism]]`)
- Disambiguation (`[[Priya — Meeting 2026-04-18|our 1:1]]`)
- Keeping prose readable when the canonical name is long

## Heading and block links

Two flavours of deep link:

```markdown
See [[Annual Review 2026#Priorities]] for the full list.
The claim in [[Seeing Like a State^legibility-def]] is worth quoting.
```

Both get their own page — see [[Headings and Blocks]].

## A note on link density

Err on the side of linking too much. The graph is only as useful as the links
in it, and backlinks (see [[Backlinks]]) surface those links back to you later.
A page with no outgoing links is a dead end; a page with twenty is a junction.

## Related

- [[Wikilinks]]
- [[Embeds and Transclusion]]
- [[Headings and Blocks]]
- [[Finding Orphans and Dead Links]]
- [[Backlinks]]
