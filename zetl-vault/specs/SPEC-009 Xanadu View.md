# SPEC-009: Xanadu View

Adds an interactive TUI for reading and navigating the vault with visible connections. See [[View Command]] for the CLI and [[Xanadu Lineage]] for the conceptual background.

```spl
(given spec-009-documented)
```

## Xanadu lineage

Implements Ted Nelson's vision of parallel pages with visible connections. The left pane shows the current note; the right pane shows context cards for linked notes; a bridge column holds color-coded connectors.

## Key requirements

| ID | Requirement |
|----|-------------|
| REQ-062 | View command entry point |
| REQ-063 | Two-pane layout |
| REQ-064 | Wikilink anchor annotations (`[N]`) |
| REQ-065 | Context pane excerpt cards |
| REQ-066 | Bridge column connectors |
| REQ-067 | Dynamic scroll tracking |
| REQ-068 | Selected-link focus mode |
| REQ-069 | Link navigation (Enter) |
| REQ-070 | Backlink mode toggle (b) |
| REQ-071 | Navigation history ([/]) |
| REQ-072 | No-index graceful fallback |
| REQ-073 | Page-not-found fuzzy suggestion |
| REQ-074 | Context excerpt length configuration |

## Architecture decisions

- [[Xanadu View Design]] (ADR-016, ADR-017, ADR-018)
- Uses ratatui + crossterm (same as [[TUI]])
- Color-paired anchor numbers as bridge rendering
- Supports true-color, 8-color, and no-color terminals

## Performance targets

- Startup: <= 200ms
- Scroll frame time: <= 16ms (60fps)

## Layout

```
┌─ Main Pane ──────────┐ ┌─Bridge─┐ ┌─ Context Cards ─────┐
│ # Scanner             │ │        │ │ ┌─ [1] Link Graph ─┐ │
│                       │ │  1 ──  │ │ │ Builds a directed │ │
│ The scanner [1] is    │ │        │ │ │ graph where nodes  │ │
│ zetl's entry point    │ │  2 ──  │ │ └──────────────────-┘ │
│ for understanding a   │ │        │ │ ┌─ [2] Cache ───────┐ │
│ vault. It uses the    │ │        │ │ │ Two-tier mtime +   │ │
│ cache [2] for...      │ │        │ │ │ hash invalidation  │ │
│                       │ │        │ │ └────────────────────┘ │
└───────────────────────┘ └────────┘ └───────────────────────┘
```

## User profiles

- **Knowledge worker** reading notes with visible connections
- **Researcher** navigating citation clusters on an SSH terminal

See also: [[Spec Index]], [[View Command]], [[Xanadu Lineage]], [[Xanadu View Design]]
