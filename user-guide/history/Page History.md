---
title: Page History
tags: [history, pages, ui]
---

# Page History

> **Requires `--features history` at install.** See [[Installation]].

Every page in your vault has a life story: when you wrote it, how often you've edited it, when each backlink appeared, how stable it's become. `zetl history page` and the history UI surface that story.

## The command

```bash
zetl history page "Zettelkasten Method"
```

Prints a reverse-chronological list of snapshots that touched the page, with timestamp, change-ID, and a short description of what changed (links added, links removed, content modified). Defaults to 20 entries; `--limit` goes higher.

```bash
zetl history page "Zettelkasten Method" --limit 100
zetl history page "Draft Essay" --json | jq
```

Case-insensitive on the page name; aliases resolve too.

## What's available in the web UI

When you run `zetl serve` (or publish with `zetl build`) against a history-enabled binary, every rendered page gains a visually-light metadata strip under the title:

```
Last changed 2026-03-18 · stable 28d · history
```

- **Last changed** — the date the page last received a meaningful edit.
- **Stable Nd** — how long it's been untouched. Useful signal for "this thought has settled" vs. "still in flux".
- **history** — a link to the per-page timeline. Opens a list of every snapshot where this page changed, with diffs.

The strip disappears on pages with no history (newly created, or vaults without the feature). No empty boxes, no `—` placeholders.

## Backlinks carry timestamps

Each backlink in the sidebar has a `since` field:

```
← [[Spaced Repetition]]    (since 2024-11-08)
← [[Active Recall]]        (since 2025-02-14)
```

When did *this particular reference* enter the graph? That's the question `since` answers. If you wrote *Zettelkasten Method* in January and *Spaced Repetition* started linking to it in November, that's a ten-month gap — meaningful signal for how ideas accrete around a note.

In templates, the field is `page.backlinks[].since` (RFC 3339 timestamp string, or `null` if the backlink predates the history floor).

## Template variables for custom themes

If you're customising the look ([[Customising the Look]]), the page context exposes:

| Variable | Type | Contents |
|----------|------|----------|
| `page.history` | object \| null | `created_at`, `last_changed`, `age_days`, `stable_days`, `link_trend`, `recent_changes` |
| `page.history.recent_changes` | array | Most-recent snapshots with timestamp and short delta description |
| `page.backlinks[].since` | string \| null | RFC 3339 timestamp of when each backlink appeared |

`page.history` is `null` when zetl was built without history, so templates that read it should use `{% if page.history %}` guards. The default theme's `page-history-meta` block is the reference implementation — copy it or strip it as you like.

## Why page history matters for writers

A note's value often correlates with how it changes over time:

- **A page that hasn't moved in 90 days** is either settled or forgotten. Cross-reference with backlink count to know which.
- **A page that changes daily** is either active thinking or noisy drift. Look at `recent_changes` to see whether edits are additive or thrashing.
- **A page with many late-arriving backlinks** is a concept that earned its keep — other notes grew toward it.

Combined with `--at` ([[Time Travel]]), you can reconstruct the arc of an idea: write the first draft, accumulate backlinks, stabilise, cite in a final piece.

## Related

- [[Time Travel]]
- [[Watching for Changes]]
- [[Snapshots Under the Hood]]
- [[Customising the Look]]
- [[Backlinks]]
