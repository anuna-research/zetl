---
title: "SPEC-009: ztl view — Xanadu-Inspired Transclusion TUI"
version: 0.1.0
status: draft
audience: agent, human
date: 2026-02-24
---

# SPEC-009: ztl view — Xanadu-Inspired Transclusion TUI

## Information Table

| Field        | Value                                                                    |
| ------------ | ------------------------------------------------------------------------ |
| Document ID  | SPEC-009                                                                 |
| Title        | ztl view — Xanadu-Inspired Transclusion TUI                             |
| Version      | 0.1.0                                                                    |
| Status       | Draft                                                                    |
| Author       | Agent (USDD Protocol v1.0.0)                                             |
| Date         | 2026-02-24                                                               |
| Audience     | Agent, Human                                                             |
| Trace        | USDD Agent Protocol v1.0.0                                               |
| Parent       | SPEC-001: ztl — Bi-directional Link Graph CLI                           |
| Related      | SPEC-006: index cache format, SPEC-008: ztl watch                       |
| Dependencies | ratatui (TUI framework), crossterm (terminal backend), SPEC-006 (index)  |
| Inspiration  | Project Xanadu (Ted Nelson, 1960–), OpenXanadu (2014), org-transclusion  |

---

## 1. Overview

Every ztl command produces output and exits. Reading through a vault with `cat` or your
editor gives you one note at a time, with `[[wikilinks]]` as inert text. You must hold the
connection graph in your head: open the linked file in another buffer, switch back, lose
your place, repeat. The graph is in the index; the connection is invisible at read time.

`ztl view` changes this. It is an interactive TUI that reads the vault through an
architecture Ted Nelson called **parallel pages with visible connections**: the note you
are currently reading occupies the left pane; the notes it links to (or that link to it)
appear as excerpt cards in the right pane; a narrow bridge column between the panes holds
color-coded connectors that visibly pair each `[[wikilink]]` in the text to its
corresponding card. You can see the connected content without navigating away.

This is the Xanadu transclusion model applied to ztl's wikilink graph.

### 1.1 The Xanadu Lineage

Ted Nelson proposed transclusion in 1965 as the principle that a quotation should remain
visibly connected to its origin — "the same content knowably in more than one place."
His 1972 paper "As We Will Think" showed the earliest known diagram of this: two screen
windows side by side, graphical lines connecting corresponding text spans across them. All
subsequent Xanadu implementations (Pyxi 1999, XanaduSpace 2007, OpenXanadu 2014) share
this same two-pane connected architecture.

The critical distinction from inline transclusion (Obsidian's `![[page]]`, Wikipedia
includes): in those systems the included content *replaces* the reference — the connection
disappears into the text. Nelson considered this a failure mode. In the Xanadu model,
the connection is always visible: you see both the place of citation and the cited content
simultaneously, connected by an explicit visual bridge.

`ztl view` implements this in a terminal. Terminals cannot draw arbitrary lines across
panes, but they can use color pairing and anchor glyphs to make the connection equally
explicit. Each `[[wikilink]]` in the current note is annotated with a colored anchor
glyph `[N]`; the corresponding context card in the right pane bears the same color and
number. The bridge column between the panes contains a horizontal connector at the row
where the anchor aligns with its card. The result is Nelson's "visible connection" in the
constraint of an 80×24 terminal.

### 1.2 Mapping Xanadu Concepts to ztl

| Xanadu concept         | ztl / SPEC-009 equivalent                                                |
| ---------------------- | ------------------------------------------------------------------------- |
| EDL (Edit Decision List) | The current note's set of `[[wikilinks]]` — each is a "content citation" |
| Source document        | The note referenced by a wikilink                                         |
| Transcluded span       | The excerpt of the referenced note shown in the context card              |
| Visible bridge / beam  | Bridge column connector + color-paired anchor glyph                       |
| Parallel pages         | Left pane (current note) + right pane (context cards)                     |
| Navigation / follow    | `Enter` on a selected link — the linked note becomes the new current note |
| Bidirectional link     | `b` mode — context pane shows backlinks instead of (or in addition to) forward links |

The difference from Xanadu's full EDL model: ztl notes are not *composed* from spans of
other notes (there is no `ztl include` syntax). The `[[wikilink]]` is a citation, not a
content span import. The transclusion effect is achieved by the TUI making the cited
content visible alongside the citation — the note file itself is unchanged.

### 1.3 Design Principles

Following SPEC-001's philosophy:

- **Files are the source of truth.** `ztl view` never modifies vault files.
- **Agent-first, human-friendly.** This command is human-facing; it has no structured
  output mode. Agents use `ztl index`, `ztl check`, `ztl diff`, `ztl watch`.
- **Fast and disposable.** The TUI is stateless: it reads the on-disk index and renders
  from it. No TUI-specific state is persisted between sessions.
- **Terminal-native.** Bridges use Unicode box-drawing characters and ANSI color codes;
  they degrade gracefully to ASCII and numbered anchors on limited terminals.

### 1.4 Scope Boundary

SPEC-001 §1.2 explicitly listed "GUI or TUI" as out of scope. SPEC-009 overrides this
for `ztl view` only. The rationale is documented in ADR-018. All other ztl commands
remain CLI-only and produce structured output. `ztl view` is purely a reading and
navigation tool; it produces no output and never modifies vault state.

### 1.5 Scope

**In scope:**

- `ztl view [<page>]` command
- Two-pane TUI with left (current note) and right (context cards) panes
- Bridge column with color-paired anchor connectors
- Dynamic scroll tracking (context pane follows main pane viewport)
- Selected-link focus and link navigation with history
- Forward-links mode (default), backlinks mode, combined mode
- Context excerpt cards (configurable line count)
- Graceful degradation: no-color terminals, narrow terminals
- Page-not-found fuzzy suggestion using SPEC-001 SimHash matching
- Interactive page picker when no `<page>` argument is supplied

**Out of scope:**

- Inline content editing (files are read-only from the TUI's perspective)
- Full-text search within the TUI (use `ztl index` + external grep)
- Image, PDF, or attachment preview
- Mouse input (keyboard-only; mouse support is a future enhancement)
- Multi-vault or multi-window sessions
- MCP or agent API surface (this command is human-facing only)
- Block-level transclusion syntax (`![[page]]` inclusion) — this would require new
  markup and is a separate future SPEC

---

## 2. User Profiles

### 2.1 User Profile: Akiko — Personal Knowledge Worker

```
Name:        Akiko
Role:        Product manager; 2,000-note Zettelkasten
Goals:       Read a note and simultaneously see the content of the notes it
             references, without losing her place; navigate the graph without
             opening files in her editor; understand orphan and dead-link
             status at a glance while reading
Constraints: Comfortable with terminal; uses Vim keybindings; reads notes in
             long sessions; terminal is 120×40, true-color capable
Workflow:    Opens ztl view on a page she is reviewing; reads through it;
             when a [[wikilink]] catches her attention, the context card is
             already visible in the right pane; she presses Tab to focus the
             link, reads the full card, presses Enter to navigate if she wants
             the full note, or continues scrolling
Pain point:  "I open five tabs to read one note. I want to see the connections
             without leaving the current note."
```

### 2.2 User Profile: Marco — Literature Review Researcher

```
Name:        Marco
Role:        PhD researcher; vault of 800 paper-summary notes with dense
             cross-references and backlink clusters
Goals:       Navigate citation clusters; see which notes cite a given note
             (backlinks); understand the transclusion context for a citation
             before deciding whether to follow it
Constraints: 80×24 terminal; 8-color only (SSH into university server);
             no-color mode acceptable; prefers anchor numbers to color coding
Workflow:    Opens ztl view on a key paper summary; switches context pane to
             backlinks mode to see which of his other notes cite this one;
             navigates backwards through citation clusters; uses [ to retrace
Pane point:  "I need to see who cites this paper in my own notes, not just
             what it cites."
```

---

## 3. Happy Paths

### 3.1 Happy Path: Forward-Link Reading Session (Akiko)

```
Preconditions:
  - Vault indexed; ztl index up to date
  - Page "ADR-043: Reject gRPC" exists with 3 wikilinks

Steps:
  1. ztl view "ADR-043: Reject gRPC"
     → TUI opens; left pane shows note content; right pane shows 3 context
       cards: [1] Benchmark: REST vs gRPC, [2] Network Architecture Overview,
       [3] API Design Principles; bridge column shows colored connectors

  2. Akiko scrolls down with j
     → As the viewport passes [[Benchmark: REST vs gRPC]], context pane
       scrolls to bring card [1] into primary view position; bridge connector
       [1] highlights

  3. She presses Tab to enter link-selection mode
     → Card [1] becomes focused; anchor [1] in main pane highlights;
       context card expands to show more lines of the linked note

  4. She presses Enter
     → Main pane reloads with "Benchmark: REST vs gRPC" as the new current
       page; navigation history records the transition; context pane shows
       the links from "Benchmark: REST vs gRPC"

  5. She presses [ (left bracket)
     → Navigates back to "ADR-043: Reject gRPC"; main pane restores scroll
       position

Postconditions:
  - Akiko read note + context without opening any files in her editor
  - No vault state modified

Failure modes:
  - Dead link [[Nonexistent Page]]: anchor [N] shown in red; context card
    shows "dead link — page does not exist" with fuzzy suggestions
  - Page deleted between index and view: card shows "page removed since last
    index — run ztl index to refresh"
```

### 3.2 Happy Path: Backlink Cluster Navigation (Marco)

```
Preconditions:
  - Vault indexed; page "Paper: Attention Is All You Need" has 12 backlinks

Steps:
  1. ztl view "Paper: Attention Is All You Need" (80×24 terminal, 8-color)
     → TUI opens; anchor numbers [1]–[N] appear in text (no color; terminal
       reported no-color via TERM=xterm); context pane shows forward links

  2. Marco presses b
     → Context pane switches to backlinks mode; right pane shows excerpt
       cards for the 12 notes that link TO this paper; bridge column shows
       connectors from the paper title / abstract area to the backlink cards

  3. He scrolls the context pane with Tab + j/k
     → Cycles through backlink cards; each card shows the citing note's title
       and the sentence containing the backlink

  4. He finds card [7] "Transformer Variants Survey" and presses Enter
     → Main pane loads "Transformer Variants Survey"; context pane resets to
       its forward-links (default mode follows new page)

Postconditions:
  - Marco navigated the backlink cluster without leaving the TUI
  - No-color mode worked correctly throughout

Failure modes:
  - Terminal narrower than 60 columns: TUI falls back to single-pane mode
    (main pane only; context pane hidden; Ctrl-R toggles a full-screen context
    pane overlay)
  - Page has 0 backlinks in backlinks mode: context pane shows "no backlinks —
    this page is an orphan" with a visual indicator
```

---

## 4. Functional Requirements

### REQ-062: `ztl view` Command Entry Point

The system SHALL provide a `ztl view [<page>]` subcommand that launches an interactive
TUI for reading and navigating the vault. When `<page>` is supplied, the note matching
that title (exact or fuzzy via SPEC-001 SimHash matching) SHALL be loaded as the initial
current page. When `<page>` is omitted, the system SHALL display an interactive page
picker (searchable list of all indexed pages, filtered by typing).

Trace:
- TEST-071
- CON-023
- ADR-018

### REQ-063: Two-Pane Layout

The system SHALL render the TUI as two primary panes separated by a bridge column:

- **Left pane (main pane):** Displays the raw Markdown content of the current note,
  rendered with `[[wikilinks]]` annotated with anchor glyphs (REQ-064). Width: 55–60%
  of terminal columns, configurable via `--main-width <pct>`.
- **Bridge column:** 3 characters wide. Contains horizontal connectors linking anchors
  to context cards (REQ-066).
- **Right pane (context pane):** Displays ordered context cards for the wikilinks in the
  current viewport (REQ-065). Width: remaining terminal columns after main pane and
  bridge column.

When the terminal width is less than 60 columns, the system SHALL fall back to
single-pane mode with a full-screen context overlay toggled by `Ctrl-R`.

Trace:
- TEST-071
- TEST-072
- CON-023
- ADR-016

### REQ-064: Wikilink Anchor Annotations

For each `[[wikilink]]` that appears in the current note, the system SHALL annotate the
wikilink in the main pane with a colored anchor glyph of the form `[N]` (where N is the
ordinal position of the link in document order, starting at 1), rendered immediately
after the closing `]]` of the wikilink. The glyph SHALL be rendered in the link's
assigned bridge color (REQ-066). When color is unavailable (REQ-074), the glyph `[N]`
SHALL be rendered in the default terminal foreground color.

Dead links (wikilinks targeting non-existent pages) SHALL render their anchor glyph in
red (or with a `!` prefix in no-color mode: `![N]`).

Trace:
- TEST-072
- TEST-078
- CON-023

### REQ-065: Context Pane — Excerpt Cards

The system SHALL populate the context pane with one **context card** per wikilink whose
anchor is currently visible in the main pane viewport (scroll-tracking mode). Cards are
ordered from top to bottom matching the top-to-bottom order of their corresponding
anchors in the main pane. Each card SHALL contain:

1. A card header: anchor glyph `[N]` + page title + dead-link indicator if applicable
2. An excerpt: the first `--context-lines <N>` lines of the linked page's content
   (default: 5 lines; minimum: 1; maximum: 20)
3. A horizontal separator below the excerpt

When the currently focused link (REQ-067) is in the context pane, its card SHALL expand
to show `--context-lines × 3` lines (triple expansion) and SHALL be scrollable
independently via `j`/`k` while focus is in the context pane.

In backlinks mode (`b` toggle, REQ-070), cards instead represent notes that link TO the
current page, and excerpts show the sentence containing the backlink rather than the
note's opening lines.

Trace:
- TEST-072
- TEST-075
- CON-023

### REQ-066: Bridge Column Connectors

The system SHALL render the bridge column with horizontal connector characters that
visually pair each main-pane anchor to the top edge of its corresponding context card.
Each connector SHALL:

1. Span the full 3-column width of the bridge column using box-drawing characters:
   `─── ` (default) or `>>>` (ASCII fallback when the terminal does not support Unicode)
2. Be rendered at the terminal row closest to the vertical midpoint between the anchor's
   row in the main pane and the context card's header row in the context pane
3. Use the same color as the corresponding anchor glyph (REQ-064)

When a link's anchor and its card are at exactly the same vertical row (aligned), the
connector SHALL be rendered as `═══` (double horizontal line, or `===` ASCII fallback)
to indicate exact alignment.

When the terminal is in no-color mode, connector characters SHALL be rendered using the
default foreground color; the anchor number `[N]` on both the anchor and the card header
provides the pairing signal without color.

Trace:
- TEST-073
- TEST-078
- CON-023
- ADR-017

### REQ-067: Dynamic Scroll Tracking

As the user scrolls the main pane, the context pane SHALL update continuously to display
context cards for the wikilinks currently within the main pane's visible viewport. The
update SHALL occur synchronously with each scroll step (one line at a time), with a
target render latency of ≤ 16ms per scroll event (NFR-024). Links that scroll out of the
main pane viewport SHALL have their cards removed from the context pane; newly visible
links SHALL have their cards inserted.

When the viewport contains more wikilinks than the context pane has vertical space to
display, the system SHALL show cards for the topmost visible wikilinks and indicate the
count of hidden cards below with a `↓ N more` indicator at the bottom of the context pane.

Trace:
- TEST-074
- NFR-024
- CON-023

### REQ-068: Selected-Link Focus

The system SHALL support a focused-link state in which one wikilink is selected. The
user enters focused-link mode from main-pane scroll mode by pressing `Tab`. While in
focused-link mode:

- The focused link's anchor glyph in the main pane renders with a reverse-video
  background (or `>[N]<` in no-color mode)
- The focused link's context card in the right pane expands (triple lines as per REQ-065)
  and is highlighted with a distinct background color (or bordered with `┌─┐` / `└─┘`
  in no-color mode)
- `j`/`k` cycles focus to the next/previous wikilink in document order (wrapping)
- `Tab` exits focused-link mode and returns to scroll mode; context pane reverts to
  scroll-tracking

Trace:
- TEST-075
- CON-023

### REQ-069: Link Navigation

While in focused-link mode (REQ-068), pressing `Enter` SHALL navigate to the focused
link's target page: the main pane SHALL reload with the target page as the new current
note; the context pane SHALL update with the new page's forward links; the navigation
history SHALL record the transition (REQ-071); the system SHALL exit focused-link mode
and enter scroll mode on the new page.

When the focused link is a dead link (targets a non-existent page), `Enter` SHALL have
no effect; the system SHALL display a status-bar message: `dead link — run ztl index or
create the page`.

Trace:
- TEST-076
- CON-023

### REQ-070: Backlink Mode Toggle

The system SHALL support `b` as a toggle cycling the context pane through three modes:

1. **forward** (default): context cards show forward links from the current page
2. **back**: context cards show backlinks TO the current page (notes that link here)
3. **both**: context cards show forward links above a separator and backlinks below

The active mode SHALL be indicated in the context pane header. In **back** and **both**
modes, scroll tracking applies to backlink cards ordered by linking-note title
alphabetically, since backlinks do not have a natural position in the current note's
document order.

Trace:
- TEST-077
- CON-023

### REQ-071: Navigation History

The system SHALL maintain an in-session navigation history of depth ≥ 50 page
transitions. `[` (left bracket) SHALL navigate backward through history; `]` (right
bracket) SHALL navigate forward. History SHALL record the page title and the main pane
scroll position at the time of navigation, so that returning to a page restores the
scroll position.

Trace:
- TEST-076
- CON-023

### REQ-072: No-Index Graceful Fallback

When `ztl view` is invoked and no ztl index exists in the vault (no `.ztl/index.json`
or equivalent per SPEC-006), the system SHALL display a status message ("Building index…")
and run the SPEC-006 index pipeline in-process before opening the TUI. If indexing fails,
the system SHALL exit non-zero with a plain-text error.

Trace:
- TEST-071
- CON-023

### REQ-073: Page-Not-Found Fuzzy Suggestion

When the `<page>` argument does not match any indexed page exactly, the system SHALL use
the SPEC-001 SimHash + Hamming distance matching to find the top 5 closest page titles
and SHALL display them as a selection prompt in the TUI before opening. If the user
selects a suggestion, the TUI opens on that page. If the user presses `q` at the
suggestion prompt, the system exits 0 with no output.

Trace:
- TEST-071
- CON-023

### REQ-074: Context Excerpt Length Configuration

The system SHALL accept `--context-lines <N>` (integer 1–20, default 5) to configure the
number of excerpt lines shown per context card in non-focused mode. The focused card
always shows `N × 3` lines regardless of configuration. This flag is the only persistent
configuration surface; no `~/.ztlrc` entry is introduced by this spec.

Trace:
- TEST-072
- CON-023

---

## 5. Non-Functional Requirements

### NFR-023: Startup Time

`ztl view` SHALL open the TUI and render the first frame within ≤ 200ms of invocation,
measured from process start to first paint, UNDER the condition that a valid SPEC-006
index already exists for a vault of ≤ 5,000 pages. This time includes loading and
parsing the index JSON, laying out the initial panes, and rendering the first frame.

Trace:
- TEST-079
- OBS-012

### NFR-024: Scroll Frame Time

Each user scroll event (single-line scroll via `j` or `k`) SHALL result in a re-render
completing within ≤ 16ms, measured from keypress receipt to terminal write flush, for a
vault of ≤ 5,000 pages and a note containing ≤ 50 wikilinks. This ensures a subjectively
smooth scrolling experience (≥ 60-event-per-second responsiveness).

Trace:
- TEST-074
- OBS-012

### NFR-025: Terminal Compatibility

The system SHALL render a correct, usable layout at minimum terminal dimensions of
80 columns × 24 rows. The system SHALL detect terminal color support via the `TERM`,
`COLORTERM`, and `NO_COLOR` environment variables and adapt rendering accordingly:

- **True-color / 256-color:** Full colored anchor glyphs and bridge connectors
- **8-color (ANSI):** Bridge colors cycle through the 8 standard ANSI foreground colors
- **No-color (`NO_COLOR` set or `TERM=dumb`):** Anchor numbers and ASCII bridge
  characters provide the pairing signal without any color dependency

Trace:
- TEST-078
- CON-023
- ADR-017

---

## 6. Architecture Decisions

### ADR-016: Ratatui as TUI Framework

**Decision:** Use the [`ratatui`](https://crates.io/crates/ratatui) Rust crate as the
TUI layout and rendering framework, with `crossterm` as the terminal backend.

**Context:** Building a correct, cross-platform TUI from raw terminal escape sequences
is error-prone and expensive. The Rust TUI ecosystem has consolidated around two options:
`tui-rs` (the original, archived) and its active fork `ratatui`. Both provide widget
layouts, text styling, and event loops. `crossterm` is the dominant cross-platform
terminal backend for both Windows and Unix.

**Rationale:** `ratatui` is actively maintained (last release within 60 days of this
spec's date), has a large ecosystem of contributed widgets, and is used in production by
tools like `gitui`, `bottom`, and `lazygit`'s TUI test harness. It provides the layout
primitives needed for the two-pane + bridge-column design: `Layout::horizontal` for pane
sizing, `Paragraph` for scrollable text, and `Canvas` for bridge column drawing.
`crossterm` handles the color-support detection (`crossterm::style::Colors`) needed for
NFR-025.

**Trade-offs:**
- ✅ Proven cross-platform (macOS, Linux, Windows)
- ✅ Supports all three color modes needed for NFR-025
- ✅ Layout API maps naturally to the two-pane + bridge column design
- ✅ Active maintenance; no risk of framework abandonment
- ⚠️ Adds ~150KB to binary size (ratatui + crossterm; acceptable)
- ⚠️ `Canvas` widget for bridge column may require custom rendering; `Paragraph` with
  styled spans is simpler and sufficient for the anchor + bridge approach

**Rejected alternative:** `cursive` (higher-level TUI framework). Its widget model is
more opinionated and less suited to the custom bridge-column rendering needed here.
`termion` is Unix-only and excluded by NFR-025's Windows compatibility requirement.

### ADR-017: Color-Paired Anchor Numbers as the Bridge Rendering Approach

**Decision:** Use color-coded anchor glyphs `[N]` in the text + matching colors in
context card headers + bridge column connector characters as the visible-connection
mechanism. Do not attempt to draw diagonal or curved lines across panes.

**Context:** Xanadu's reference implementations (XanaduSpace, OpenXanadu) draw graphical
lines between parallel document panes. In a terminal, crossing pane boundaries with
arbitrary graphical lines is not possible — the terminal is a grid of cells, not a pixel
canvas. Several approaches are available:

1. **Full graphical lines across panes**: Not possible in a standard terminal.
2. **Colored anchor numbers (selected)**: Each link gets a unique color `[N]` appearing
   at the link position in the main pane and the card header in the context pane. A
   bridge column renders a horizontal connector at the alignment row.
3. **Indicator columns** (like git diff `+`/`-`): A narrow left-edge column in each
   pane showing `[N]` markers, with no center bridge column.
4. **Virtual text overlays** (Neovim extmarks style): Anchor numbers rendered as
   overlaid text after the wikilink, not consuming a separate column.

**Rationale:** Option 2 is the most faithful terminal-native translation of Xanadu's
bridge metaphor. The bridge column provides a dedicated visual lane for connection
indicators, analogous to the graphical beam area in XanaduSpace. The color pairing
provides the immediate visual association Nelson describes. Option 3 was rejected because
it removes the explicit spatial bridge. Option 4 is what option 2 does — the `[N]` glyph
IS virtual text added after the wikilink.

The critical design constraint is graceful degradation (NFR-025): the anchor number
`[N]` itself provides the pairing signal independently of color. Even on Marco's 8-color
terminal, `[1]` next to `[[Benchmark: REST vs gRPC]]` and `[1] Benchmark: REST vs gRPC`
as the context card header uniquely identify the pair without any color at all. Color
makes it faster; the number makes it correct.

**Trade-offs:**
- ✅ Works on all terminals including no-color
- ✅ No color-only information dependency (WCAG 1.4.1 compliance)
- ✅ Spatial bridge column is a recognisable visual metaphor for "connection"
- ⚠️ Notes with > 8 wikilinks in one viewport require color cycling (colors repeat);
  anchor numbers still distinguish pairs within the viewport
- ⚠️ The `[N]` glyph adds characters after wikilinks; this changes line lengths and may
  cause unexpected wrapping in notes with long wikilink-heavy lines. Mitigated by the
  fact that the main pane is the TUI's own layout — no external renderer is affected.

### ADR-018: SPEC-001 TUI Scope Override

**Decision:** Add `ztl view` as a TUI command, overriding the "GUI or TUI: out of
scope" decision in SPEC-001 §1.2.

**Context:** SPEC-001 was written as a foundational CLI scope document. At the time, the
priority was establishing ztl as a solid CLI tool with agent-friendly JSON output before
adding interactive features. Since then, SPEC-005 through SPEC-008 have established a
mature CLI surface. The research that preceded this spec (GitHub topic crawls, February
2026) found that the TUI knowledge graph space is almost entirely vacant, with the closest
competitor (Synapxis) having 0 stars and no visible-connection bridge UI. The Xanadu
two-pane model has never been implemented in a terminal. `ztl view` occupies a unique
position.

**Rationale:** The TUI scope exception is narrow and bounded:
- `ztl view` is the only interactive command
- It produces no structured output (no `--format json` mode)
- It does not modify vault files
- It is additive — existing commands and their agent-facing contracts are unchanged
- It depends entirely on the existing SPEC-006 index, introducing no new persistent state

Adding it does not compromise the agent-first design principle; agents still use the
existing CLI commands. `ztl view` serves human reading sessions, not agent pipelines.
The earlier exclusion was a deferral, not a permanent architectural constraint.

**Trade-offs:**
- ✅ Fills the one gap in ztl's human-facing surface
- ✅ Implements a novel, research-validated interaction model (Xanadu transclusion)
- ✅ Does not break or complicate any existing command
- ⚠️ Adds a ratatui + crossterm dependency to the binary; binary size increases ~150KB
- ⚠️ TUI testing is harder than CLI testing; requires virtual terminal infrastructure
  (see TEST-071–079 for the approach)

---

## 7. Contract Specifications

### CON-023: `ztl view`

**Interface:** `ztl view [<page>] [--context-lines <N>] [--main-width <pct>]`

**Argument rules:**
- `<page>`: optional; page title (exact or fuzzy via SPEC-001 SimHash). If omitted,
  shows interactive page picker.
- `--context-lines <N>`: integer 1–20; default 5. Lines shown per context card in
  non-focused mode.
- `--main-width <pct>`: integer 30–80; default 58. Percentage of terminal columns
  allocated to the main pane. The bridge column takes 3 columns; the remaining columns
  go to the context pane.

**Pre-conditions:**
- A ztl vault exists (`.ztl/index.json` or equivalent per SPEC-006). If not, `ztl
  view` will build the index before opening (REQ-072).
- Terminal is attached (stdin is a TTY). Running `ztl view` from a non-TTY context
  (pipe, script) SHALL exit immediately with error code `NOT_A_TTY`.

**Post-conditions:**
- Exits 0 on `q` (quit) or `Ctrl-C`
- Exits non-zero on fatal errors (index build failure, I/O error)
- Vault files are unchanged

**Keyboard bindings:**

| Key          | Context              | Action                                                  |
| ------------ | -------------------- | ------------------------------------------------------- |
| `j` / `↓`   | Main pane (scroll)   | Scroll main pane down one line                          |
| `k` / `↑`   | Main pane (scroll)   | Scroll main pane up one line                            |
| `Ctrl-d`     | Main pane (scroll)   | Scroll main pane down half a page                       |
| `Ctrl-u`     | Main pane (scroll)   | Scroll main pane up half a page                         |
| `g`          | Main pane (scroll)   | Go to top of current note                               |
| `G`          | Main pane (scroll)   | Go to bottom of current note                            |
| `Tab`        | Main pane (scroll)   | Enter focused-link mode on first visible link           |
| `j` / `k`   | Focused-link mode    | Cycle focus to next/previous wikilink                   |
| `Enter`      | Focused-link mode    | Navigate to focused link's target page                  |
| `Tab`        | Focused-link mode    | Exit focused-link mode; return to scroll mode           |
| `b`          | Any                  | Toggle context pane mode: forward → back → both         |
| `[`          | Any                  | Navigate backward in session history                    |
| `]`          | Any                  | Navigate forward in session history                     |
| `Ctrl-R`     | Any                  | Toggle context pane overlay (single-pane fallback mode) |
| `/`          | Any                  | Open page search (interactive filter over indexed pages)|
| `?`          | Any                  | Show key bindings help overlay                          |
| `q` / `Ctrl-C` | Any               | Quit `ztl view`                                        |

**Status bar (bottom of terminal):**

```
ztl view │ ADR-043: Reject gRPC  │  forward  │  [1] Benchmark: REST vs gRPC  │  3 links
```

Fields: tool name, current page title, context mode, focused link (if any), link count.

**Bridge column rendering contract:**

The bridge column is 3 characters wide, positioned between the main pane's right edge
and the context pane's left edge. For each link `N` whose anchor is visible in the main
pane and whose card is visible in the context pane, the bridge column SHALL contain a
connector sequence rendered at the row that is the vertical midpoint between the anchor
row (main pane) and the card header row (context pane):

- If anchor row and card header row are on the same terminal row: `═══` (or `===`)
- If they differ: `───` (or `---`) colored with the link's bridge color

When the terminal is in no-color mode, all connectors render in the default foreground
color; the `[N]` anchor numbers on both sides carry the pairing information.

**Error model:**

```json
{ "error": { "code": "NOT_A_TTY", "message": "ztl view requires an interactive terminal." } }
{ "error": { "code": "INDEX_BUILD_FAILED", "message": "Could not build index: <reason>." } }
{ "error": { "code": "PAGE_NOT_FOUND", "message": "No page matching '<title>' found.", "suggestions": ["...", "..."] } }
```

**Implements:** REQ-062–074

**Verified by:** TEST-071–079

---

## 8. Test Specifications

### TEST-071: View Opens on Specified Page; Context Pane Populated

**Requirement:** REQ-062, REQ-063, REQ-072

**Preconditions:** Vault with 10 pages; "PageA.md" contains `[[PageB]]` and `[[PageC]]`;
both targets exist; `ztl index` previously run

**Steps:**
1. Invoke `ztl view "PageA"` in a virtual terminal (80×24, 256-color)
2. Verify first frame renders within 200ms (NFR-023)
3. Verify main pane left shows content of PageA.md
4. Verify context pane right shows two context cards: one for PageB, one for PageC
5. Verify status bar shows `PageA` and `2 links`

---

### TEST-072: Wikilink Anchor Annotations and Context Card Structure

**Requirement:** REQ-064, REQ-065, REQ-074

**Preconditions:** "Source.md" with line: `See also [[Target One]] and [[Target Two]].`

**Steps:**
1. Open `ztl view "Source"` in 256-color virtual terminal
2. Verify main pane renders the line with `[1]` appended after `[[Target One]]` and
   `[2]` after `[[Target Two]]`, both in distinct colors
3. Verify context pane shows card header `[1] Target One` followed by 5 excerpt lines
   (default `--context-lines 5`)
4. Invoke with `--context-lines 3`; verify cards show exactly 3 excerpt lines

---

### TEST-073: Bridge Column Connectors

**Requirement:** REQ-066

**Preconditions:** "Source.md" with two wikilinks at different vertical positions

**Steps:**
1. Open `ztl view "Source"`
2. Capture raw terminal output (including escape sequences)
3. Verify a 3-character sequence (`───` or `===`) is present in the bridge column at
   the expected midpoint rows between anchor positions and card header positions
4. Verify the connector for link [1] uses the same ANSI color code as the anchor `[1]`
   and the card header `[1]` in the context pane

---

### TEST-074: Dynamic Scroll Tracking — Context Pane Updates on Scroll

**Requirement:** REQ-067, NFR-024

**Preconditions:** "Source.md" with 20 wikilinks across 100 lines; only 10 fit in the
80×24 viewport at a time

**Steps:**
1. Open `ztl view "Source"` (main pane shows lines 1–20 initially)
2. Press `j` 15 times to scroll down; measure time per render event
3. Verify context pane shows cards only for the wikilinks currently in the main pane
   viewport (not for links scrolled out of view)
4. Verify each scroll re-render completes within 16ms (NFR-024)

---

### TEST-075: Focused-Link Mode — Dual Highlight

**Requirement:** REQ-068

**Preconditions:** "Source.md" with two wikilinks visible simultaneously in viewport

**Steps:**
1. Open `ztl view "Source"` in scroll mode
2. Press `Tab` to enter focused-link mode on link [1]
3. Verify: anchor `[1]` in main pane renders with reverse-video / `>[1]<` marker
4. Verify: context card for [1] is expanded (triple lines) and highlighted/bordered
5. Press `j`: verify focus moves to link [2]; `[1]` returns to normal; `[2]` highlights
6. Press `Tab`: verify return to scroll mode; all cards return to normal size

---

### TEST-076: Link Navigation and History

**Requirement:** REQ-069, REQ-071

**Preconditions:** "PageA.md" links to "PageB.md"; "PageB.md" links to "PageC.md"

**Steps:**
1. Open `ztl view "PageA"`; press `Tab` to focus `[[PageB]]`; press `Enter`
2. Verify main pane now shows "PageB" content; status bar shows "PageB"
3. Press `Tab` to focus `[[PageC]]`; press `Enter`; verify main pane shows "PageC"
4. Press `[`: verify navigation back to "PageB" with scroll position restored
5. Press `[` again: verify navigation back to "PageA"
6. Press `]`: verify forward to "PageB"

---

### TEST-077: Backlink Mode

**Requirement:** REQ-070

**Preconditions:** "CentralPage.md" is referenced by "NoteX.md" and "NoteY.md"

**Steps:**
1. Open `ztl view "CentralPage"`
2. Press `b`: verify context pane header shows "back" mode
3. Verify context pane shows two cards for NoteX and NoteY
4. Each card excerpt SHALL show the sentence in NoteX / NoteY that contains the
   wikilink to CentralPage (not the opening lines of NoteX/NoteY)
5. Press `b` again: verify context pane shows "both" mode with forward links above
   separator and backlinks below
6. Press `b` again: verify return to "forward" mode

---

### TEST-078: No-Color Terminal Degradation

**Requirement:** REQ-064, REQ-066, NFR-025

**Preconditions:** `NO_COLOR=1` set in environment; "Source.md" with two wikilinks

**Steps:**
1. Invoke `ztl view "Source"` with `NO_COLOR=1`
2. Verify: no ANSI color escape sequences in output
3. Verify: anchor glyphs rendered as `[1]`, `[2]` (no color)
4. Verify: bridge column connectors rendered as `---` or `===` (ASCII fallback)
5. Verify: context card headers rendered as `[1] Target One`, `[2] Target Two` (no color)
6. Verify: the pairings are unambiguous via anchor numbers alone

---

### TEST-079: Startup Time (NFR-023)

**Requirement:** NFR-023

**Preconditions:** Vault with 5,000 pages; SPEC-006 index built and current

**Steps:**
1. Record wall-clock time T0 immediately before `ztl view "SomePage"` is invoked
2. Record T1 when the first rendered frame is written to the terminal (detectable via
   a sentinel escape sequence in the test harness)
3. Repeat 10 times; verify T1 − T0 ≤ 200ms at p95

---

## 9. Observability

### OBS-012: View Render Timing Diagnostics

**Signal:** Debug output when `ztl_DEBUG_RENDER=1` environment variable is set (not
`--verbose`; render timing is a debug concern separate from operational verbosity)

```
[ztl view] startup  index_load_ms=18  first_frame_ms=47  page="ADR-043: Reject gRPC"
[ztl view] render   event=scroll_j  layout_ms=2  paint_ms=4  total_ms=6
[ztl view] render   event=tab_focus  layout_ms=3  paint_ms=5  total_ms=8
[ztl view] navigate from="ADR-043: Reject gRPC"  to="Benchmark: REST vs gRPC"  history_depth=2
[ztl view] quit     uptime_s=847  pages_visited=12  nav_history_depth=24
```

**Purpose:** Verify NFR-023 (startup ≤ 200ms) and NFR-024 (scroll ≤ 16ms) in production
environments; diagnose slow renders on constrained hardware.

---

## 10. Traceability Matrix

| REQ     | NFR     | CON    | ADR              | TEST                   | OBS    |
| ------- | ------- | ------ | ---------------- | ---------------------- | ------ |
| REQ-062 | —       | CON-023 | ADR-016, ADR-018 | TEST-071               | OBS-012 |
| REQ-063 | NFR-025 | CON-023 | ADR-016          | TEST-071, TEST-072     | —      |
| REQ-064 | —       | CON-023 | ADR-017          | TEST-072, TEST-078     | —      |
| REQ-065 | —       | CON-023 | ADR-017          | TEST-072, TEST-075     | —      |
| REQ-066 | —       | CON-023 | ADR-017          | TEST-073, TEST-078     | —      |
| REQ-067 | NFR-024 | CON-023 | —                | TEST-074               | OBS-012 |
| REQ-068 | —       | CON-023 | —                | TEST-075               | —      |
| REQ-069 | —       | CON-023 | —                | TEST-076               | —      |
| REQ-070 | —       | CON-023 | —                | TEST-077               | —      |
| REQ-071 | —       | CON-023 | —                | TEST-076               | —      |
| REQ-072 | —       | CON-023 | ADR-018          | TEST-071               | —      |
| REQ-073 | —       | CON-023 | —                | TEST-071               | —      |
| REQ-074 | —       | CON-023 | —                | TEST-072               | —      |
| —       | NFR-023 | —      | —                | TEST-079               | OBS-012 |
| —       | NFR-024 | —      | —                | TEST-074               | OBS-012 |
| —       | NFR-025 | —      | ADR-017          | TEST-078               | —      |

---

## 11. Layout Reference

### 11.1 Annotated Screen Layout (80×24 terminal, forward-links mode, scroll mode)

```
┌──────────────────────────────────────────────┬───┬──────────────────────────────┐
│ ADR-043: Reject gRPC                         │   │  Context — forward (2 links) │
│ ─────────────────────────────────────────    │   │ ─────────────────────────── │
│ After extensive testing, we chose REST       │   │ [1] Benchmark: REST vs gRPC  │
│ over gRPC for our public API layer.          │───│ A comparison of REST and     │
│ See [[Benchmark: REST vs gRPC]][1] for       │   │ gRPC for our API layer.      │
│ detailed numbers.                            │   │ We benchmarked latency at    │
│                                              │   │ p50, p95, and p99 under...   │
│ The decision aligns with our layered         │   │ ─────────────────────────── │
│ design in [[Network Architecture             │───│ [2] Network Architecture     │
│ Overview]][2].                               │   │     Overview                 │
│                                              │   │ The top-level architecture   │
│ Consequences:                                │   │ separates the API gateway    │
│ - REST clients need no proto compilation     │   │ from internal services.      │
│ - gRPC reserved for internal service mesh   │   │ Each layer communicates...   │
│                                              │   │ ─────────────────────────── │
│                                              │   │                              │
│                                              │   │                              │
│                                              │   │                              │
│                                              │   │                              │
│                                              │   │                              │
│                                              │   │                              │
├──────────────────────────────────────────────┴───┴──────────────────────────────┤
│ ztl view │ ADR-043: Reject gRPC │ forward │ 2 links │ j/k scroll  Tab focus  ? │
└───────────────────────────────────────────────────────────────────────────────────┘
```

Key observations:
- `[1]` appears inline after the wikilink text in the main pane
- Bridge column (`───`) appears at the row midpoint between anchor and card header
- Context cards show title + 5 excerpt lines + separator
- Status bar anchors to the bottom row

### 11.2 Focused-Link Mode (link [1] selected)

```
┌──────────────────────────────────────────────┬───┬──────────────────────────────┐
│ ADR-043: Reject gRPC                         │   │  Context — forward (2 links) │
│ ─────────────────────────────────────────    │   │ ─────────────────────────── │
│ After extensive testing, we chose REST       │   │ ┌─[1] Benchmark: REST vs ──┐ │
│ over gRPC for our public API layer.          │═══│ │ A comparison of REST and  │ │
│ See [[Benchmark: REST vs gRPC]]>[1]< for     │   │ │ gRPC for our API layer.  │ │
│ detailed numbers.                            │   │ │ We benchmarked latency   │ │
│                                              │   │ │ at p50, p95, and p99     │ │
│ The decision aligns with our layered         │   │ │ under simulated load.    │ │
│ design in [[Network Architecture             │   │ │ REST showed 12% lower    │ │
│ Overview]][2].                               │   │ │ median latency...        │ │
│                                              │   │ │ ↓ j/k to scroll          │ │
│ Consequences:                                │   │ └──────────────────────────┘ │
│ - REST clients need no proto compilation     │───│ [2] Network Architecture     │
│ - gRPC reserved for internal service mesh   │   │ The top-level architecture   │
│                                              │   │ separates the API gateway    │
│                                              │   │ ─────────────────────────── │
│                                              │   │                              │
│                                              │   │                              │
│                                              │   │                              │
│                                              │   │                              │
│                                              │   │                              │
├──────────────────────────────────────────────┴───┴──────────────────────────────┤
│ ztl view │ ADR-043: Reject gRPC │ forward │ [1] Benchmark: REST vs gRPC │ Enter │
└───────────────────────────────────────────────────────────────────────────────────┘
```

Key observations:
- `>[1]<` marker on the selected anchor in the main pane
- Card [1] is boxed (`┌─┐/└─┘`), expanded to triple lines (15 lines), independently
  scrollable
- Bridge connector for [1] uses `═══` (double horizontal, "exact focus" style)
- Bridge connector for [2] uses `───` (normal)
- Status bar shows the focused link title and `Enter` hint

### 11.3 No-Color Mode (Marco's 8-color SSH terminal, backlinks mode)

```
ADR-043: Reject gRPC                         |||  Context — back (3 backlinks)
 ─────────────────────────────────────────   |||  ----------------------------
 After extensive testing, we chose REST      ---  [1] Weekly Review 2026-02-17
 over gRPC for our public API layer.         |||  "...decided to use REST fol-
 See [[Benchmark: REST vs gRPC]] [1].        |||  lowing ADR-043: Reject gRPC,
                                             |||  which concluded last week..."
 The decision aligns with our layered        ---  [2] Architecture Decision Log
 design in [[Network Architecture            |||  "...ADR-043: Reject gRPC was
 Overview]] [2].                             |||  approved unanimously in the
                                             |||  Feb 17 architecture review..."
 Consequences:                               ---  [3] API Design Principles
 - REST clients need no proto compilation    |||  "...for external-facing APIs
 - gRPC reserved for internal service mesh  |||  (see ADR-043: Reject gRPC),
                                             |||  REST is the default choice..."
                                             |||
---------------------------------------------------------------------------------------------
 ztl view | ADR-043: Reject gRPC | back | 3 backlinks | j/k scroll  Tab focus  ?
```

Key observations:
- No ANSI color escape sequences
- Bridge connectors use `---` ASCII fallback
- Bridge column uses `|||` as vertical filler (ASCII pipe characters)
- Anchor numbers `[1]`, `[2]`, `[3]` carry all pairing information
- Context cards in backlinks mode show the sentence containing the backlink, not the
  opening lines of the citing note

---

## 12. Future Work

### 12.1 Mouse Support

Clicking a `[[wikilink]]` in the main pane would directly focus that link. Hovering a
context card would highlight the corresponding anchor. Mouse support requires terminal
mouse protocol detection (`\x1b[?1000h`) and is a straightforward ratatui extension once
the keyboard model is stable.

### 12.2 Inline Edit Mode

An `e` key binding could open the current note in `$EDITOR` at the cursor line, returning
to `ztl view` after the editor exits. This would make `ztl view` a read-navigate-edit
loop. It requires careful state management (re-indexing the edited file on editor exit to
keep the context pane current) and is intentionally deferred until SPEC-008 (`ztl watch`)
is implemented — the watch event stream provides the natural trigger for a re-render after
an edit.

### 12.3 EDL-Based Transclusion Syntax

If ztl ever introduces explicit transclusion syntax (e.g. `{{include:page#section}}`),
`ztl view` would be the natural place to render it as a first-class Xanadu transclusion:
the included content appears in the main pane at the include site, and the source
document's context card shows the full context of the included section. This would
complete the Xanadu EDL model within ztl. Specified separately; requires new parsing
in SPEC-006 and new syntax design.

### 12.4 Graph Navigation Mode

A `G` (capital, different from current go-to-bottom binding) could switch the main pane
from document view to a text-mode graph view: the current page as a node, with ASCII
edges to its neighbours. Navigation would then hop through the graph directly. This
builds on the research finding that `netext` (70 stars) is the best terminal graph
rendering library and could be a drop-in for this mode.

### 12.5 Session Persistence

`ztl view` currently holds no state between sessions (intentional per §1.3). A future
`.ztl/view-session.json` could persist the navigation history and last-open page across
invocations. The design would need to address staleness (page renamed or deleted between
sessions). Deferred until there is demonstrated demand.
