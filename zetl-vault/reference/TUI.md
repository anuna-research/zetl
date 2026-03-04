# TUI

`zetl tui` launches an interactive terminal interface for browsing your vault. It is the human-oriented complement to zetl's [[JSON by Default]] output philosophy.

```spl
(given tui-complete)
(given seven-views)
```

## Views

| View | Description |
|------|-------------|
| Dashboard | Vault stats and most-linked pages |
| Pages | Filterable page list |
| Links | Forward/back link explorer |
| Search | Full-text search with context |
| Diagnostics | Dead links, orphans, syntax issues |
| Page | Rendered Markdown with inline [[concepts/Wikilinks]] navigation |
| Graph | Local link graph with depth toggle |
| Timeline | Temporal snapshot navigation (requires `--features history`) |

The Timeline view shows vault history as a navigable list of snapshots with graph-level deltas. See [[History Command]].

## Navigation

| Key | Action |
|-----|--------|
| `Tab` / `Shift+Tab` | Cycle views |
| `Ctrl+K` | Quick switcher |
| `j` / `k` | Scroll |
| `Enter` | Follow a wikilink |
| `Backspace` | Go back |
| `/` | Search/filter |

## Implementation

Built on `ratatui` and `crossterm`. The TUI reads the same [[Link Graph]] and [[architecture/Cache]] as the CLI commands — no separate data path. See [[SPEC-009 Xanadu View]] for the related two-pane reader.

## Relationship to View

The TUI is a multi-view dashboard. For focused reading with visible connections between notes, use [[View Command]] instead — it provides a Xanadu-inspired two-pane layout with bridge connectors.

See also: [[CLI Reference]], [[View Command]], [[Serve Command]]
