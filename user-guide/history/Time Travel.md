---
title: Time Travel
tags: [history, time-travel, queries]
---

# Time Travel

> **Requires `--features history` at install.** See [[Installation]].

`--at` is a global flag that re-runs any read-only zetl command against a past state of your vault. Last Tuesday's backlinks, the orphan list from before the big refactor, the link count on the day you published — all one flag away.

## Why time-travel your notes

You remember writing about *Glass-Steagall* in the essay you posted in June, but the page doesn't mention it any more. Did you delete the link, or did you never write it? Without history, you'd dig through git log (if you even use git on your notes) and eyeball old files. With `--at`:

```bash
zetl --at "2024-06-01" links "Banking Reform"
```

The graph as it stood that day. Same output format, same flags, same wikilink resolver — just a different snapshot under the hood.

Other uses:

- **Audit a stable conclusion.** `zetl --at "last monday" check` to confirm the vault was clean when you claimed it was.
- **Recover a deleted page.** `zetl --at HEAD~5 view "Lost Thought"` shows the full rendered page from five snapshots ago.
- **Compare statistics across time.** `zetl --at "3 months ago" stats` against `zetl stats` — a graph-size delta without leaving the shell.
- **Retrace a thought.** When did I first link *Zettelkasten Method* to *Spaced Repetition*? `zetl --at "90 days ago" backlinks "Spaced Repetition"`.

## What `--at` accepts

| Expression | Example | Notes |
|------------|---------|-------|
| ISO 8601 date | `--at "2024-01-15"` | Resolves to the latest snapshot on or before that date. |
| Natural-language relative | `--at "3 days ago"` | Also `"last monday"`, `"yesterday"`, `"2 weeks ago"`. |
| VCS ref | `--at HEAD~1` | One snapshot before the current tip. |
| Change-ID prefix | `--at abc12` | jj change-ID, any unique prefix. |

Quote expressions that contain spaces — your shell will thank you.

## Which commands accept it

Every read-only subcommand. A partial list:

```bash
zetl --at "last week" links "Project X"
zetl --at "2024-06-01" stats
zetl --at HEAD~5 check
zetl --at "yesterday" backlinks "Draft"
zetl --at "3 days ago" search "deadline"
zetl --at "last monday" orphans
```

Commands that *write* — `index`, `watch` with side effects, hooks that mutate files — ignore `--at` or refuse to run against a historical state. You cannot accidentally edit a past vault.

## Typical workflow

1. `zetl stats` — current numbers.
2. `zetl --at "last monday" stats` — a week ago.
3. Eye the diff. If something's interesting, narrow: `zetl --at "last monday" links "That Page"`.
4. Still interesting? `zetl diff --from "last monday"` summarises the full graph-level delta.

No checkout, no stash, no branch — the live vault on disk doesn't move.

## Use case: "what did the vault look like when I wrote that blog post?"

You posted an essay on March 3rd. The essay cites five zetl pages. You want to know what *those pages* said on March 3rd — not their current, evolved state.

```bash
zetl --at "2024-03-03" view "Mental Models"
zetl --at "2024-03-03" links "Mental Models"
```

The view reads from that snapshot's content; the link graph matches. Export the essay's sources as they appeared at publication time, so the post's argument stays reproducible.

## When `--at` returns nothing useful

If you installed history late in the vault's life, snapshots only exist from that point forward. `zetl history timeline` lists every snapshot on record — your effective floor for time-travel.

## Related

- [[Watching for Changes]]
- [[Page History]]
- [[Snapshots Under the Hood]]
- [[CLI Overview]]
