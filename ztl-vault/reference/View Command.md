# View Command

`ztl view` is a Xanadu-inspired two-pane reader for focused page navigation. See [[Xanadu Lineage]] for the design philosophy.

## Usage

```bash
ztl view "Scanner"               # open a specific page
ztl view                         # open the page picker
ztl view "Scanner" --context-lines 10 --main-width 60
```

## Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--context-lines N` | 5 | Lines shown per context card (1–20) |
| `--main-width pct` | 58 | Percentage of terminal width for the main pane (30–80) |

## Layout

The screen is divided into three columns:

- **Left pane** — the current note, rendered with numbered `[N]` anchor glyphs at each wikilink
- **Bridge column** — color-coded connectors pairing anchors to cards
- **Right pane** — context cards: excerpts from forward-linked pages

In narrow terminals (<60 cols), the view falls back to a single-pane layout.

## Keybindings

| Key | Action |
|-----|--------|
| `j` / `k` | Scroll (or cycle links in focus mode) |
| `Ctrl-d` / `Ctrl-u` | Half-page scroll |
| `g` / `G` | Top / bottom of note |
| `Tab` | Toggle between scroll and focus mode |
| `Enter` | Navigate to focused link |
| `[` / `]` | Session history back / forward |
| `b` | Toggle backlink mode |
| `/` | Open page picker |
| `?` | Toggle keybindings help |
| `q` | Quit |

## Implementation

Built on `ratatui` and `crossterm`, like the [[TUI]]. Supports true-color, 8-color, and no-color terminals. See [[Xanadu View Design]] for design decisions and [[SPEC-009 Xanadu View]] for the full specification.

See also: [[CLI Reference]], [[TUI]], [[Xanadu Lineage]], [[Serve Command]]
