# Xanadu View Design

**Status:** Accepted

## Context

zetl needed a focused reading mode that shows connections between notes. The design question was how to render Ted Nelson's Xanadu concept of "parallel pages with visible connections" in a terminal.

See [[Xanadu Lineage]] for the conceptual background and [[SPEC-009 Xanadu View]] for the full specification.

## Key decisions

### ADR-016: Use ratatui + crossterm

The TUI framework was already in use for `zetl tui`. Reusing it for `zetl view` provides consistent behavior across terminal emulators and avoids a second rendering dependency.

### ADR-017: Color-paired anchor numbers

Each wikilink gets a numbered anchor glyph `[N]` in the main pane. The corresponding context card in the right pane shares the same color. A bridge column between the panes draws connectors linking each anchor to its card.

This is more practical in a terminal than literal connecting lines (which would require pixel-level rendering). The color pairing provides the visual association that Xanadu's bridge connectors achieve.

### ADR-018: Override SPEC-001 TUI scope exclusion

SPEC-001 originally scoped out TUI functionality. The value of a Xanadu-style reader justified overriding this exclusion. The view command adds significant user value for knowledge exploration.

## Terminal compatibility

The view supports three rendering modes:

- **True-color** — full RGB color pairing for modern terminals
- **8-color** — mapped to the 8 basic ANSI colors
- **No-color** — plain text with numbered anchors only

Narrow terminals (<60 cols) fall back to a single-pane layout.

See also: [[View Command]], [[Xanadu Lineage]], [[TUI]]
