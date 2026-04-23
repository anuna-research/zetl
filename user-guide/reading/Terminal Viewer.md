---
title: Terminal Viewer
tags: [reading, cli, tui]
---

# Terminal Viewer

`ztl view` is a Xanadu-inspired two-pane reader for reading linked notes without leaving the terminal. It exists because most of the time you don't want a browser — you want to sit with one note, glance at what it points to, and jump onwards when something catches your eye.

## Why two panes

When you read a note in a normal editor, forward-links are just names — `[[Zettelkasten Method]]` tells you nothing about what Zettelkasten Method actually *says*. Ted Nelson's original vision for Xanadu put the cited text right next to the citing text, bridged by a visible connector. `ztl view` does exactly that in a TTY:

- **Main pane** (left) — the note you asked for, rendered as Markdown. Every wikilink is annotated with a numbered anchor glyph `[1]`, `[2]`, `[3]` in reading order.
- **Context column** (right) — a stack of context cards, one per forward link. Each card shows the first few lines of the linked note, coloured to match its anchor.
- **Bridge column** (middle) — coloured connectors from each `[N]` anchor to its card, so your eye can follow the citation at a glance.

In terminals narrower than 60 columns the layout collapses to single-pane and context cards become inline footnotes. You lose the bridges but keep the anchors.

## Opening a page

```bash
ztl view "Zettelkasten Method"      # by page name
ztl view                            # interactive page picker
ztl view "Project Notes" --context-lines 10
ztl view "Project Notes" --main-width 65
```

With no argument, `ztl view` launches an interactive picker of every page in the vault — type to filter, Enter to open.

## Flags

| Flag | Default | Purpose |
|------|---------|---------|
| `--context-lines <N>` | `5` | Lines shown per context card. Range 1–20. |
| `--main-width <pct>` | `58` | Percent of terminal columns for the main pane. Range 30–80. |
| `--at <TIME-EXPR>` | — | Read a past snapshot (see [[Time Travel]]). |
| `-d, --dir <DIR>` | `.` | Vault root. |

## Keybindings

| Key | Action |
|-----|--------|
| `j` / `k` | Scroll down / up (or cycle anchors in focus mode) |
| `Ctrl-d` / `Ctrl-u` | Half-page down / up |
| `g` / `G` | Jump to top / bottom of the note |
| `Tab` | Toggle between scroll mode and focus mode |
| `Enter` | Navigate to the focused link |
| `[` / `]` | Session history: back / forward |
| `/` | Open the page picker |
| `?` | Toggle the keybindings overlay |
| `q` | Quit |

**Focus mode** is what makes the navigation fast. Hit `Tab`, then `j`/`k` steps from one anchor to the next (not one line at a time); the bridge column highlights the selected anchor's connector; `Enter` opens that page. `[` returns to where you came from, so you can explore breadth-first through a chain of notes and back-track without losing your place.

## When to reach for it

Use `ztl view` when you're:

- Thinking through a research question where the notes form a web and you want citations visible without tab-switching.
- Proofreading a chain of linked notes on a server over SSH where a browser isn't available.
- Navigating a vault whose backlinks and forward-links you don't know by heart yet.

For heavier reading — embedded images, wide tables, the full graph view — reach for [[Web Server]] instead.

## Related

- [[Web Server]]
- [[Following Links]]
- [[Wikilinks]]
- [[Time Travel]]
- [[CLI Overview]]
