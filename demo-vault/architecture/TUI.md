---
title: TUI
---

# TUI

`ztl tui` launches an interactive terminal interface for browsing your vault. It is the human-oriented complement to ztl's [[JSON by Default]] output.

## Views

```spl
(given tui-complete)
(given seven-views)
```

| View | Description |
|------|-------------|
| Dashboard | Vault stats and most-linked pages |
| Pages | Filterable page list |
| Links | Forward/back link explorer |
| Search | Full-text search with context |
| Diagnostics | Dead links, orphans, syntax issues |
| Page | Rendered Markdown with inline [[Wikilinks]] navigation |
| Graph | Local link graph with depth toggle |

## Navigation

- `Tab` / `Shift+Tab` — cycle views
- `Ctrl+K` — quick switcher
- `j` / `k` — scroll
- `Enter` — follow a wikilink
- `Backspace` — go back

## Implementation

Built on `ratatui` and `crossterm`. The TUI reads the same [[Link Graph]] and [[Cache]] as the CLI commands — no separate data path.

See also: [[Graph Queries]], [[Search]], [[Vault Diagnostics]]
