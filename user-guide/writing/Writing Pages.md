---
title: Writing Pages
tags: [writing, pages, markdown]
---

# Writing Pages

In a ztl vault, one page is one Markdown file. No databases, no sidecar XML, no
magic — if you can write a `.md` file, you can write a ztl page.

## The filename is the title

ztl uses the filename (minus `.md`) as the page title. Title-case filenames
with spaces are the convention:

```
~/notes/
  Zettelkasten Method.md
  Reading List 2026.md
  Meeting with Priya 2026-04-18.md
```

When another page writes `[[Zettelkasten Method]]`, ztl resolves the link by
matching the filename. Slugification happens at *render* time — the URL becomes
`/zettelkasten-method/`, but your filesystem stays human-readable. Case doesn't
matter for resolution; `[[zettelkasten method]]` works too.

There is no restriction on where a file lives. Folders are for your eyes, not
ztl's. See [[Organising Your Vault]].

## A minimal page

Every page is CommonMark-compatible Markdown with optional YAML frontmatter at
the top:

```markdown
---
title: Reading List 2026
tags: [reading, journal]
status: open
---

# Reading List 2026

Books I want to read this year, with a running note once I finish each.

- [[Gödel, Escher, Bach]] — started January
- [[The Glass Bead Game]] — queued
- [[Seeing Like a State]] — finished March

## Related

- [[Book Journal]]
- [[Annual Review 2026]]
```

That's it. No build step, no registration. Save the file; it's a page. Run
`ztl list` and you'll see it.

## Markdown flavour

ztl parses CommonMark plus a small set of widely-supported extensions:
fenced code blocks, tables, footnotes, strikethrough, task lists. Wikilinks
(`[[...]]` and `![[...]]`) and block anchors (`^id`) are ztl-specific — see
[[Wikilinks]] and [[Headings and Blocks]].

If you have been using Obsidian or another wikilink editor, your existing
Markdown will almost certainly parse without changes. See
[[Migrating from Obsidian]].

## ztl doesn't impose a method

You don't have to run a Zettelkasten. You don't have to run Johnny Decimal.
You don't have to write atomic notes, daily notes, MOCs, or any other thing. A
vault of long-form essays works. A vault of one-line bookmarks works. A vault
that mixes research notes, a reading journal, and project plans works.

The only rule ztl cares about: *pages are files, links are `[[links]]`*.
Everything else is your call.

## Writing workflows

A few idioms that tend to work well:

| Situation | What to do |
|---|---|
| New thought, not sure where it belongs | Create `Inbox Note 2026-04-18.md` at the vault root, link later |
| Long-form draft | Keep it in one file until a section starts getting linked to — then extract |
| Reference material | One page per concept; link liberally from narrative pages |
| Journal | `Journal/2026-04-18.md` per day; links to whatever you were thinking about |

## Previewing what you wrote

Three ways to see the rendered page:

```bash
ztl view "Reading List 2026"   # two-pane terminal view
ztl serve                      # local web server
ztl build                      # write dist/ as static HTML
```

See [[Terminal Viewer]], [[Web Server]], and [[Static Site Export]].

## Related

- [[Linking Pages]]
- [[Organising Your Vault]]
- [[Frontmatter]]
- [[Your First Vault]]
- [[Wikilinks]]
