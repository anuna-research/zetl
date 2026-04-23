---
title: "SPEC-015: ztl fountain — Screenplay Theme and Scene Chaining"
version: 0.3.0
status: draft
audience: agent, human
date: 2026-03-02
---

# SPEC-015: ztl fountain — Screenplay Theme and Scene Chaining

## Information Table

| Field          | Value                                                          |
| -------------- | -------------------------------------------------------------- |
| Document ID    | SPEC-015                                                       |
| Title          | ztl fountain — Screenplay Theme and Scene Chaining            |
| Version        | 0.3.0                                                          |
| Status         | Draft                                                          |
| Author         | Agent (USDD Protocol v1.3.0)                                   |
| Date           | 2026-03-02                                                     |
| Audience       | Agent, Human                                                   |
| Trace          | USDD Agent Protocol v1.3.0                                     |
| Parent         | SPEC-012: ztl — Named Themes for Serve and Build              |
| Related        | SPEC-014: Theme Distribution, SPEC-016: Lifecycle Hooks, SPEC-001: Bi-directional Link Graph |

---

## 1. Overview

Screenwriters work in scenes. Each scene is a self-contained unit — a location, a time, characters, dialogue, action — but scenes only become a screenplay when placed in sequence. This specification adds first-class screenplay support to ztl by combining two capabilities:

1. **A Fountain theme** — templates and CSS that render vault pages as screenplay pages using Courier Prime and industry-standard formatting.
2. **Scene chaining** — frontmatter fields (`prev` and `next`) that establish a linear reading order across scene files, with prev/next navigation in the web UI.

Screenplay assembly (walking the chain, stripping frontmatter/wikilinks, concatenating into a single `.fountain` file) is a natural use case for the lifecycle hooks system (SPEC-016). The fountain theme MAY bundle a `post-build` hook that performs assembly, keeping the core binary free of screenplay-specific logic.

### 1.1 Core Insight

A ztl vault of screenplay scenes is a **linked list encoded as a wiki**. Each scene file is a node; `prev`/`next` frontmatter fields are the pointers. Wikilinks within scenes cross-reference characters, locations, and story notes — useful during writing but can be stripped during assembly by a hook.

### 1.2 Design Philosophy

1. **Fountain is the source format** — scene files contain raw Fountain markup as their primary content.
2. **The wiki layer is for writing, not output** — wikilinks, frontmatter, and internal notes help the writer organise. A post-build hook can strip them to produce clean `.fountain` output.
3. **Assembly is a hook, not a core feature** — the assembled screenplay is produced by a theme-bundled post-build hook (per SPEC-016), not baked into the binary. This keeps the core engine generic.
4. **Scenes are files, order is frontmatter** — no special directory structure required. A flat folder or a nested hierarchy both work; the `prev`/`next` chain determines reading order.
5. **The theme is optional for chaining** — scene chaining and prev/next navigation work with any theme. The Fountain theme provides the best visual experience, but other themes get prev/next links automatically.

### 1.3 Scope

**In scope:**

- Bundled `fountain` theme with Courier Prime font, screenplay page layout, and prev/next navigation
- `prev` and `next` frontmatter fields for establishing scene order
- Prev/next navigation links in the page template (all themes, not just fountain)
- Scene chain validation in `ztl check` (broken chains, cycles, orphaned scenes)

**Out of scope:**

- Screenplay assembly (walking chains, stripping wikilinks/frontmatter, concatenating into `.fountain`) — this is a post-build hook concern per SPEC-016
- Fountain parsing or semantic validation (ztl treats Fountain as opaque text — it passes through unchanged)
- PDF generation (users pipe the `.fountain` output to dedicated tools like Highland, Fade In, or afterwriting)
- Real-time collaborative editing
- Fountain-specific search or indexing (standard full-text search from SPEC-013 applies)
- Automatic scene numbering (Fountain handles this natively with `#scene-number#` syntax)
- Revision tracking or colored revision pages (production concern, not a writing tool concern)
- A standalone `ztl compile` command

---

## 2. User Profiles

### 2.1 Solo Screenwriter

```
Role: Screenwriter working on a feature film or pilot episode
Goals:
  - Write each scene as a separate file for easy reordering and revision
  - See scenes rendered with proper screenplay formatting in the browser
  - Navigate between scenes with prev/next links
  - Get a compiled .fountain file from the build output for submission or import into Highland/Final Draft
  - Use wikilinks to cross-reference characters, locations, and story bibles during writing
Constraints:
  - Expects industry-standard formatting: Courier 12pt, specific margins
  - May not be technical — needs simple CLI commands
  - Works across macOS and Linux
  - Needs the compiled output to be a valid .fountain file readable by any screenplay app
Daily workflow:
  1. Open a scene file, write in Fountain syntax
  2. Preview in browser with `ztl serve --theme fountain`
  3. Navigate between scenes using prev/next
  4. Run `ztl build --theme fountain` to produce static site
  5. If a post-build assembly hook is installed, find the assembled .fountain file in the build output
```

**Happy Path: Write and Preview a Feature Film**

```
Preconditions: Vault exists with scene files containing Fountain content.
Steps:
  1. Create scene files with frontmatter:
     ---
     title: "BRICK & STEEL"
     author: "Jane Doe"
     credit: "Written by"
     next: "[[Scene 2 - Coffee Shop]]"
     ---
     INT. APARTMENT - MORNING
     ...

  2. ztl serve --theme fountain → browser shows screenplay pages with Courier Prime
  3. Click "Next Scene" → navigates to the next scene in chain
  4. Position indicator shows "Scene 1 of 12"
  5. ztl check → validates chain integrity (no broken links, no cycles)

Postconditions: Scenes render with screenplay formatting and are navigable in chain order.
Failure modes:
  - Broken chain (scene's next target doesn't exist) → ztl check reports error
  - Circular chain → detected, error with cycle path
```

### 2.2 TV Writers' Room

```
Role: Team of writers collaborating on an episodic series
Goals:
  - Each writer works on different scenes
  - Scenes reference shared character bibles and location descriptions via wikilinks
  - Multiple chains (one per episode) coexist in the same vault
  - Navigate between scenes per-episode using prev/next
Constraints:
  - Multiple scene chains in one vault (one per episode)
  - Need to identify which chain a scene belongs to
  - Shared reference pages (characters, locations) have no prev/next fields
Daily workflow:
  1. Write scenes with episode-specific chains
  2. Reference [[Character Bible/SARAH]] for consistency
  3. ztl serve --theme fountain → preview scenes with screenplay formatting
  4. Navigate between episode scenes using prev/next links
  5. ztl check → validates all chains are intact
```

**Happy Path: Multi-Episode Vault**

```
Preconditions: Vault with scenes for multiple episodes, plus shared reference pages.
Steps:
  1. E01 scenes chain: E01 Cold Open → E01 Scene 2 → ... → E01 Tag
  2. E02 scenes chain: E02 Teaser → E02 Scene 2 → ... → E02 Tag
  3. Reference pages (Character Bible, Locations) have no prev/next — they're wiki pages, not scenes
  4. ztl serve --theme fountain → each scene renders with screenplay formatting
  5. Prev/next navigation works within each episode's chain independently

Postconditions: Each chain is independently navigable. ztl check validates integrity.
Failure modes:
  - A scene appears in two chains → error: scene has multiple prev pointers
  - A scene's next points to a reference page → warning: target has no prev/next, chain ends
```

---

## 3. Requirements

### 3.1 Fountain Theme

REQ-015-001: Bundled Fountain Theme

The system SHALL include a bundled `fountain` theme in `themes/fountain/` that renders vault pages with industry-standard screenplay formatting:

- **Font:** Courier Prime (loaded via `@font-face` from bundled WOFF2 files in the theme's `static/` directory, with fallback to `Courier New, Courier, monospace`)
- **Page dimensions:** Content area styled to approximate a US Letter screenplay page (roughly 6 inches of text width within the viewport)
- **Margins:** Left margin wider than right (approximately 1.5" left, 1" right, scaled to viewport)
- **Line spacing:** Single-spaced with standard Fountain element spacing (blank line between elements)
- **Base colours:** White/off-white page on a neutral grey background, black text (light mode). Dark mode: dark grey page, light text.
- **Sidebar:** Collapsed by default to maximise the page view. Lists scenes in chain order when a valid chain exists, falling back to alphabetical.

The theme SHALL render without any external CDN dependencies. All fonts and CSS are self-contained in the theme's static assets.

Trace:
- TEST-015-001

---

REQ-015-002: Screenplay Element Styling

The fountain theme's CSS SHALL style standard HTML elements to approximate Fountain element formatting:

| Fountain Element | HTML Source | Visual Treatment |
| --- | --- | --- |
| Scene Heading | `<h2>`, `<h3>`, or lines matching `INT.`/`EXT.` patterns | Uppercase, bold, left-aligned |
| Action | `<p>` (default paragraphs) | Left-aligned, full width |
| Character | Identified by ALL CAPS paragraphs | Centered, approximately 3.7" from left |
| Dialogue | Paragraphs following Character | Centered, approximately 2.5" wide |
| Parenthetical | Paragraphs in `()` following Character/Dialogue | Centered, narrower than dialogue |
| Transition | Lines ending in `TO:` or forced with `>` | Right-aligned, uppercase |
| Centered | Wrapped in `> <` | Centered |

Note: ztl does not parse Fountain semantically. The theme uses CSS selectors and patterns to approximate formatting. Writers using standard Fountain conventions will get correct visual output. The assembled `.fountain` file is the authoritative format for precise rendering in dedicated screenplay software.

Trace:
- TEST-015-002

---

REQ-015-003: Prev/Next Navigation in Theme

The fountain theme's `page.html` template SHALL render prev/next navigation links when the current page's frontmatter contains `prev` and/or `next` fields:

- **Previous:** A left-arrow link at the top and bottom of the page, navigating to the `prev` scene
- **Next:** A right-arrow link at the top and bottom of the page, navigating to the `next` scene
- **Scene position indicator:** "Scene 3 of 12" or similar, shown between the arrows

The navigation links SHALL resolve the frontmatter wikilink targets (e.g., `[[Scene 2]]`) to page slugs using the vault's slug map.

Trace:
- TEST-015-003

---

### 3.2 Scene Chaining via Frontmatter

REQ-015-004: Prev/Next Frontmatter Fields

The system SHALL recognise `prev` and `next` fields in a page's YAML frontmatter as scene chain pointers. Values are page names (optionally wrapped in wikilink syntax for consistency with ztl conventions):

```yaml
---
prev: "[[Scene 1 - Apartment]]"
next: "[[Scene 3 - Coffee Shop]]"
---
```

Or without wikilink syntax:

```yaml
---
prev: "Scene 1 - Apartment"
next: "Scene 3 - Coffee Shop"
---
```

Both forms SHALL be accepted. When wikilink syntax is used, the `[[` and `]]` delimiters and any alias (text after `|`) are stripped to extract the target page name.

Trace:
- TEST-015-004
- CON-015-001

---

REQ-015-005: Chain Head Detection

A scene file with `next` but no `prev` SHALL be identified as a **chain head** — the first scene in a screenplay. A scene with `prev` but no `next` is the **chain tail** (last scene). A scene with both is an interior node.

The system SHALL detect all chain heads in a vault. Chain heads are the entry points for assembled screenplay views and build outputs.

Trace:
- TEST-015-005

---

REQ-015-006: Chain Validation

`ztl check` SHALL include chain validation when scene chain frontmatter is detected:

- **Broken forward link:** Scene A's `next` points to a page that does not exist → error
- **Broken backward link:** Scene A's `next` is Scene B, but Scene B's `prev` is not Scene A → warning (asymmetric chain)
- **Cycle detection:** Walking `next` pointers from any chain head revisits a scene → error with cycle path
- **Multiple predecessors:** Two different scenes list the same `next` target → error (fan-in creates ambiguous order)
- **Orphaned scenes:** Scenes with `prev`/`next` fields that are not reachable from any chain head → warning

Chain validation SHALL be included in the standard `ztl check` output as a new diagnostic category alongside dead links, orphans, and syntax errors.

Trace:
- TEST-015-006

---

### 3.3 Prev/Next Navigation (All Themes)

REQ-015-007: Prev/Next in Page Context

The template engine SHALL expose prev/next chain data in the `PageContext` struct, available to all themes:

```rust
pub struct PageContext {
    // ... existing fields ...
    pub prev_page: Option<ChainLink>,    // NEW
    pub next_page: Option<ChainLink>,    // NEW
    pub chain_position: Option<usize>,   // NEW: 1-indexed position in chain
    pub chain_length: Option<usize>,     // NEW: total scenes in this chain
    pub chain_head_slug: Option<String>, // NEW: slug of chain head (for assembled view link)
}

pub struct ChainLink {
    pub title: String,
    pub slug: String,
}
```

These fields are populated by reading the page's frontmatter `prev`/`next` fields and resolving the targets against the vault's page index. If `prev` or `next` is absent or the target page does not exist, the corresponding field is `None`.

`chain_position` and `chain_length` are computed by walking backward from the current page to the chain head (counting), then forward to the chain tail (counting). If the chain is broken or the page has no chain fields, these are `None`.

Trace:
- TEST-015-007
- CON-015-002

---

### 3.4 Non-Functional Requirements

NFR-015-001: Font Licensing

The Courier Prime font files bundled with the fountain theme SHALL be distributed under their original license (SIL Open Font License 1.1). The license file SHALL be included in the theme's `static/` directory.

Trace:
- TEST-015-NFR-001

---

## 4. Architecture Decisions

### ADR-015-001: Fountain as Opaque Text, Not Parsed

**Context:** Fountain is a plain-text markup language with well-defined syntax for scene headings, dialogue, action, transitions, etc. ztl could parse Fountain semantically (identifying elements, validating structure) or treat it as opaque text that passes through unchanged.

**Decision:** Treat Fountain content as opaque text. The theme provides visual approximation via CSS patterns; any assembly hook preserves content verbatim.

**Rationale:**

- **Simplicity** — no Fountain parser to build or maintain. Fountain parsing is a solved problem in dedicated tools (Highland, Fade In, afterwriting).
- **Fidelity** — zero risk of mangling the writer's content. What they wrote is what the theme renders and what any assembly hook outputs.
- **Separation of concerns** — ztl is the wiki/graph/view layer. Screenplay rendering is delegated to purpose-built tools.
- **Forward compatibility** — if Fountain syntax evolves, ztl doesn't need updating.

**Trade-off:** The theme's CSS-based formatting is approximate. Writers who need pixel-perfect screenplay rendering export the `.fountain` file (via an assembly hook) to a dedicated app. The web preview is for writing workflow, not final output.

---

### ADR-015-002: Prev/Next Frontmatter over Directory Ordering

**Context:** Scene order could be established by: (a) alphabetical/numerical file naming, (b) a manifest file listing scenes in order, (c) frontmatter fields linking scenes explicitly.

**Decision:** Use `prev`/`next` frontmatter fields in each scene file.

**Rationale:**

- **Explicit over implicit** — the order is declared in each file, not inferred from naming conventions that may break.
- **Reorderable** — changing scene order means editing two files' frontmatter, not renaming files and updating all references.
- **Multiple chains** — a vault can contain multiple independent chains (episodes, acts) without special directory structure.
- **Wiki-native** — the fields can use wikilink syntax (`[[Scene 2]]`), consistent with how ztl users already reference pages.
- **Inspectable** — `ztl check` can validate chain integrity. `ztl links` already shows forward/backward relationships.

**Trade-off:** Reordering requires editing frontmatter in two+ files (the moved scene, its old neighbours, its new neighbours). A manifest-based approach would centralise order in one file. However, the frontmatter approach keeps each scene self-describing and avoids a single-point-of-failure manifest.

**Alternative considered:** A `screenplay.yaml` manifest listing scenes in order. Rejected because it duplicates information (the scene list exists in the frontmatter chain), creates a sync problem, and doesn't compose well with multiple chains.

---

### ADR-015-003: CSS-Based Fountain Formatting over JavaScript Parser

**Context:** The theme could use a client-side JavaScript Fountain parser (e.g., fountain.js) to dynamically render content, or use CSS rules to approximate formatting based on HTML patterns.

**Decision:** Use CSS-based formatting. Do not include a JavaScript Fountain parser.

**Rationale:**

- **No JS dependency** — the theme works in environments with JavaScript disabled (print stylesheets, RSS readers, minimal browsers).
- **Performance** — CSS rendering is instantaneous. No DOM manipulation or parsing delay.
- **Consistency with ztl philosophy** — templates are inert HTML/CSS. No executable code in themes (REQ-014-015 from SPEC-014).
- **Build mode compatibility** — static HTML output works the same as serve mode.

**Trade-off:** CSS cannot perfectly identify all Fountain elements (e.g., distinguishing Character names from shouted Action requires semantic parsing). The approximation is good enough for writing workflow; final rendering is done in dedicated software.

---

## 5. Contracts

### CON-015-001: Scene Chain Frontmatter Schema

```yaml
---
# Standard Fountain title page fields (optional, used for title page generation)
title: "BRICK & STEEL"
credit: "Written by"
author: "Jane Doe"
source: "Based on the novel by John Smith"
draft_date: "March 2026"
contact: |
  Jane Doe
  jane@example.com

# Scene chain fields (one or both)
prev: "[[Scene 1 - Apartment]]"    # or plain: "Scene 1 - Apartment"
next: "[[Scene 3 - Coffee Shop]]"  # or plain: "Scene 3 - Coffee Shop"
---
```

Rules:
- `prev` and `next` values are strings. If wrapped in `[[...]]`, the delimiters are stripped.
- If the value contains `|`, text after the pipe is ignored (alias syntax).
- Page name resolution is case-insensitive, matching ztl's standard behaviour.

Implements:
- REQ-015-004

Verified by:
- TEST-015-004

---

### CON-015-002: Extended PageContext for Chain Data

```rust
// Added to src/web/context.rs

#[derive(Debug, Clone, Serialize)]
pub struct ChainLink {
    pub title: String,
    pub slug: String,
}

// Added fields to PageContext:
pub struct PageContext {
    // ... existing fields ...
    pub prev_page: Option<ChainLink>,
    pub next_page: Option<ChainLink>,
    pub chain_position: Option<usize>,   // 1-indexed
    pub chain_length: Option<usize>,
    pub chain_head_slug: Option<String>,
}
```

Template usage (any theme):

```html
{% if prev_page or next_page %}
<nav class="scene-nav">
  {% if prev_page %}
    <a href="{{ root_path }}{{ prev_page.slug }}/{{ index_file }}">← {{ prev_page.title }}</a>
  {% endif %}
  {% if chain_position %}
    <span>{{ chain_position }} of {{ chain_length }}</span>
  {% endif %}
  {% if next_page %}
    <a href="{{ root_path }}{{ next_page.slug }}/{{ index_file }}">{{ next_page.title }} →</a>
  {% endif %}
</nav>
{% endif %}
```

Implements:
- REQ-015-007

Verified by:
- TEST-015-007

---

## 6. Test Specifications

### TEST-015-001: Fountain Theme Rendering

Scenario: `ztl serve --theme fountain` with a scene file containing standard Fountain content.
Expected: Page renders with Courier Prime font, screenplay page layout, white page on grey background. No external CDN requests.

Scenario: `ztl build --theme fountain` produces static HTML.
Expected: Static files include font files in `_static/`, CSS is self-contained, pages render correctly when opened via `file://`.

---

### TEST-015-002: Screenplay Element CSS

Scenario: Scene file contains `INT. COFFEE SHOP - DAY`, action paragraphs, CHARACTER names in caps, dialogue, and a `CUT TO:` transition.
Expected: Scene heading is uppercase and bold. Character names are centered. Dialogue is narrower and centered. Transition is right-aligned.

---

### TEST-015-003: Prev/Next Navigation

Scenario: Three scene files with chain: A → B → C. View scene B in browser.
Expected: "← A" link at top/bottom. "C →" link at top/bottom. Position indicator shows "2 of 3".

Scenario: View scene A (chain head) in browser.
Expected: No prev link. "B →" link. Position "1 of 3".

Scenario: View scene C (chain tail) in browser.
Expected: "← B" link. No next link. Position "3 of 3".

---

### TEST-015-004: Frontmatter Field Parsing

Scenario: Frontmatter with `next: "[[Scene 2]]"`.
Expected: Parsed as target page "Scene 2".

Scenario: Frontmatter with `prev: "Scene 1 - Apartment"` (no wikilink syntax).
Expected: Parsed as target page "Scene 1 - Apartment".

Scenario: Frontmatter with `next: "[[Scene 2|Next]]"`.
Expected: Parsed as target page "Scene 2" (alias ignored).

---

### TEST-015-005: Chain Head Detection

Scenario: Vault with 5 scene files. Scene A has `next` only. Scene E has `prev` only. Scenes B, C, D have both.
Expected: Scene A identified as chain head. Scene E identified as chain tail.

Scenario: Vault with two independent chains (episodes).
Expected: Two chain heads detected.

---

### TEST-015-006: Chain Validation

Scenario: Scene A's `next` is "Nonexistent Scene".
Expected: `ztl check` reports broken chain error.

Scenario: Scene A → B → C → A (cycle).
Expected: `ztl check` reports cycle with path A → B → C → A.

Scenario: Scene A → B and Scene C → B (fan-in).
Expected: `ztl check` reports ambiguous predecessor for B.

Scenario: Scene D has `prev: "[[Scene C]]"` but is not reachable from any chain head.
Expected: Warning about orphaned scene.

---

### TEST-015-007: Page Context Chain Data

Scenario: Scene B in a 5-scene chain (A → B → C → D → E).
Expected: `prev_page` = {title: "A", slug: "..."}, `next_page` = {title: "C", slug: "..."}, `chain_position` = 2, `chain_length` = 5.

Scenario: Page with no prev/next frontmatter.
Expected: All chain fields are None.

---

## 7. Observability

OBS-015-001: Chain Validation in Check

`ztl check` verbose output SHALL include:
- Number of scene chains found
- Chain lengths
- Any validation warnings or errors

---

## 8. Phased Implementation

### Phase 1: Scene Chaining and Page Context

**Goal:** Add `prev`/`next` frontmatter parsing, chain data in `PageContext`, chain validation in `ztl check`, and basic prev/next navigation in all themes. This phase has no theme dependency.

**Changes:**
- Parse `prev`/`next` from frontmatter in `context.rs`
- Add `ChainLink`, `chain_position`, `chain_length`, `chain_head_slug` to `PageContext`
- Implement chain walking and chain head detection
- Add chain validation diagnostics to `ztl check`
- Add conditional prev/next nav to the default theme's `page.html` (shown only when chain data exists)

**Verification:** Create test vault with chained scenes. Verify chain data in page context. Verify `ztl check` reports broken chains. Verify prev/next links render in default theme.

### Phase 2: Fountain Theme

**Goal:** Create the bundled `fountain` theme with Courier Prime and screenplay formatting.

**Changes:**
- Create `themes/fountain/` with `base.html`, `page.html`, `index.html`, `folder.html`
- Bundle Courier Prime WOFF2 files in `static/`
- Include OFL license for Courier Prime
- Add screenplay-specific CSS for element formatting
- Enhanced prev/next navigation with scene position indicator
- Sidebar shows scenes in chain order when a valid chain exists

**Verification:** `ztl serve --theme fountain` renders scenes as screenplay pages. `ztl build --theme fountain` produces static HTML. Fonts load without CDN.

### Phase 3: Assembly Hook (Post SPEC-016)

**Goal:** Once the hooks system (SPEC-016) is implemented, create a theme-bundled `post-build` hook in the fountain theme that assembles chained scenes into a `.fountain` file.

**Changes:**
- Add `hooks/post-build` executable to `themes/fountain/`
- Hook reads chain data from `ztl_CONTEXT` stdin JSON, walks chains, strips frontmatter/wikilinks, concatenates, and writes `.fountain` files to the output directory
- This phase depends on SPEC-016 being implemented first

**Verification:** `ztl build --theme fountain` with a chained vault produces `screenplay/<slug>.fountain` in the output directory via the hook.

---

## 9. Open Questions

1. **Should the fountain theme detect Fountain elements via CSS or via a preprocessing step?**
   Pure CSS can match patterns like "all-caps paragraph" for Character detection, but it's fragile (e.g., acronyms in action text might be misidentified). A lightweight preprocessing step in the template engine could add CSS classes to Fountain elements. Recommendation: start with pure CSS, iterate based on user feedback.

2. **Should the Courier Prime font files be embedded in the binary or kept as theme static assets?**
   WOFF2 font files are ~30-50 KB each. Embedding them in the binary via the bundled theme system (SPEC-014) keeps things simple but increases binary size. Recommendation: embed them — the size is negligible relative to the overall binary, and it ensures offline functionality.

3. **What language should the assembly hook be written in?**
   Per SPEC-016, hooks are executables. A shell script would be simplest but less portable. A small Rust binary compiled alongside ztl would be fast but adds build complexity. A Python/Deno script would be easy to write but adds a runtime dependency. Recommendation: ship as a shell script initially (most users have bash), document how to replace with a compiled alternative.

---

## 10. Future Considerations

| Feature | Description |
| --- | --- |
| Serve-time assembly route | A `/screenplay/<slug>/` route in serve mode, rendered by a `pre-serve` hook or middleware |
| PDF rendering | `@media print` CSS in the fountain theme for browser-based PDF export, or integration with a headless renderer |
| Fountain syntax highlighting | Syntax-aware highlighting in the TUI view (SPEC-009) |
| Scene reordering | `ztl reorder` command to swap scene positions by updating frontmatter |
| Character index | Auto-generated character list from ALL CAPS names in scene content |
| Dialogue extraction | Extract all dialogue for a specific character across scenes |
| Fountain linting | Validate Fountain syntax (missing scene headings, unclosed dual dialogue, etc.) |
| Import from `.fountain` | Split a monolithic .fountain file into per-scene vault pages |
| Revision mode | Track changes between assembly versions, output colored revision pages |
