---
title: Your First Vault
tags: [tutorial, getting-started]
---

# Your First Vault

A vault is just a folder of `.md` files. This page walks you through creating one from scratch — three linked notes, an index, a health check, and a web UI — so you have a feel for vault hygiene before you pour a decade of notes in.

## Make the folder

```bash
mkdir ~/notes
cd ~/notes
```

That's the whole setup. No config files, no database, no `init` command. zetl treats any folder containing Markdown as a vault.

## Create three linked notes

Open your editor and create three files. The content matters less than the wikilinks — we want to see the graph form.

**`Projects.md`**:

```markdown
---
tags: [hub, projects]
---
# Projects

Active work this quarter. My overarching goals for the year live
in [[2026 Goals]]. Day-to-day progress notes accumulate in
[[Daily Journal]].

## Current

- Rewriting the customer onboarding flow
- Writing a long-form piece about [[2026 Goals#writing]]
```

**`2026 Goals.md`**:

```markdown
---
tags: [planning, goals]
year: 2026
---
# 2026 Goals

Three things I want to get done this year. Progress lives
in [[Daily Journal]], and specific projects are tracked
in [[Projects]].

## Writing

Publish a monthly essay. See [[Projects]] for the current draft.

## Health

Run three times a week.

## Learning

Read one book a month, notes in the journal.
```

**`Daily Journal.md`**:

```markdown
---
tags: [journal]
---
# Daily Journal

## 2026-04-20

Started sketching the onboarding rewrite from [[Projects]].
Reminder: the writing goal in [[2026 Goals#writing]] is
behind — only one essay shipped so far.

## 2026-04-19

Read two chapters. Need to backfill notes on those tomorrow.
```

Three files. Six wikilinks between them. One of them (`[[2026 Goals#writing]]`) points at a specific heading.

## Build the index

```bash
zetl index
```

If your terminal shows a table like:

```
Metric              Value
Pages               3
Links               6
Unique targets      3
Dead links          0
Orphans             0
```

…then zetl parsed everything correctly. The `.zetl/` cache now holds the graph, the search index, and (if built with `--features history`) a snapshot.

## Check vault health

```bash
zetl check
```

With three internally-consistent files, you should see `No issues found` (or an empty diagnostics block in JSON mode). Typical things `check` catches as a vault grows:

- **Dead links** — `[[Some Page]]` where no file `Some Page.md` exists.
- **Orphan pages** — notes nothing links to and that link to nothing.
- **Syntax errors** — malformed wikilinks, broken frontmatter.

Run `zetl check --dead-links` or `zetl check --orphans` to isolate one category. See [[Finding Orphans and Dead Links]] for the full workflow.

## Query the graph

Who points at *Projects*?

```bash
zetl backlinks "Projects"
```

Answer: both `2026 Goals` and `Daily Journal` do. What does *2026 Goals* link out to?

```bash
zetl links "2026 Goals"
```

Answer: `Daily Journal` and `Projects`. Two hops:

```bash
zetl path "Daily Journal" "Projects"
```

zetl finds the shortest path through your link graph — in this three-note case, one hop, but the same command scales to hundred-hop traversals in a large vault.

## Read it in the browser

```bash
zetl serve
# Serving on http://localhost:3000
```

You get a three-column layout: sidebar with all three pages, rendered Markdown in the middle, backlinks and transclusion cards on the right. Click `[[2026 Goals]]` in `Projects.md` and the page navigates without reload — the graph widget in the corner updates its highlighted node.

Edits in the browser save straight back to `~/notes/*.md`. Your text editor sees the update immediately. zetl re-indexes in the background.

## Wikilink syntax cheat-sheet

Everything you just used, plus a few extras you'll want:

| Form | Meaning |
|------|---------|
| `[[Projects]]` | Link to the page `Projects.md`. |
| `[[Projects\|current work]]` | Same link, displayed as "current work". |
| `[[2026 Goals#Writing]]` | Link to the *Writing* heading in `2026 Goals.md`. |
| `[[Projects^abc123]]` | Link to the block with ID `abc123`. |
| `![[Daily Journal]]` | Embed (transclude) the target's content. |

See [[Wikilinks]] for every variant and [[Linking Pages]] for the day-to-day workflow.

## What just happened

You made a folder, wrote three Markdown files, and pointed zetl at it. That's a vault. Add 397 more files and you have a Zettelkasten. zetl never wrote to any of your notes; everything it needed to remember lives in `~/notes/.zetl/` and can be deleted without losing a word.

## Next

- [[Writing Pages]] — titles, Markdown flavours, frontmatter conventions.
- [[Linking Pages]] — aliases, heading links, block references, day-to-day practice.
- [[The Link Graph]] — how zetl thinks about your notes.
- [[Finding Orphans and Dead Links]] — keeping the vault healthy as it grows.

## Related

- [[Writing Pages]]
- [[Linking Pages]]
- [[The Link Graph]]
- [[Finding Orphans and Dead Links]]
- [[Migrating from Obsidian]]
