# Canonical extensions

This document tracks the first-party canonical extensions the default theme
ships (SPEC-032 REQ-3212). Per the REQ-3212 resolution of SPEC-033 §13 Q1,
each canonical extension is a **thin stub** — the default theme owns CSS
and template partials; the transformation itself is delegated to an
ecosystem plugin declared in the theme's `.zetl/hooks/` manifests.

Sections below are per-extension. Extensions not yet landed are listed
under their plan-task name for forward reference; this file grows as each
task merges.

<!-- toc -->
- [admonition](#admonition)
- [tasks](#tasks)

## admonition

Canonical extension recognising two admonition syntaxes:

- **Obsidian `ad-*` fenced blocks.**

  ````markdown
  ```ad-note
  title: Getting started

  Read the README.
  ```
  ````

  A leading `title: <text>` line inside the fenced block sets the block
  title; other recognised header lines (`collapse:`, `icon:`, `color:`) are
  consumed but ignored by the stub. Everything after the header runs as
  Markdown.

- **Python-Markdown MkDocs admonitions.**

  ```markdown
  !!! warning "Breaking change"
      Do not upgrade yet.

  !!! tip
      Use **keyboard shortcuts**.
  ```

  Titles may be quoted (`"Title"`) or bare. The body is lines indented by
  four spaces or one tab; the first non-indented non-blank line ends the
  block.

### Selector

A realistic default-theme selector matches any page whose body contains
either `` ```ad- `` or `!!! ` (space-terminated). The selector is gated
by the extension's manifest (not yet landed) and observed per page via
`zetl hook coverage --vault .`.

Per-page opt-out works the same as every canonical extension: frontmatter
key `extensions.admonition: false`.

### HTML output shape

Both syntaxes render to the shared admonition shape, compatible with
`mdbook-admonish` and Python-Markdown's `admonition` extension:

```html
<div class="admonition <type>">
<p class="admonition-title"><Title></p>
<!-- Markdown-rendered body -->
</div>
```

The default title is the type name with its first letter uppercased
(`note` → `Note`, `danger` → `Danger`).

### Backing ecosystem plugin

The theme stub ships CSS + templates only; the actual Obsidian `ad-*` →
`<div>` rewrite is expected to run through an ecosystem plugin (see
SPEC-033). Recommended backings:

- **`pandoc-admonition`** — drop-in for Pandoc-backed builds.
- **`mdbook-admonish`** — drop-in for mdBook-backed builds.

Without a backing plugin installed, the theme's degraded-render fallback
still produces a valid `<div class="admonition ...">` shell via the
in-repo stub — the gate that verifies this contract lives at
`tests/extension-fixtures/admonition/` and is driven by the shared
golden-HTML harness (SPEC-032 CON-3212, TEST-3212c).

### Golden-HTML fixture

- Input: `tests/extension-fixtures/admonition/input.md`
- Expected HTML: `tests/extension-fixtures/admonition/expected.html`
- Selector-match: `tests/extension-fixtures/admonition/selector-match.txt`

Run the gate:

```
cargo test --test ext_golden_html_integration
```

Regenerate the expected after a stub change:

```
cargo xtask update-golden admonition
```

Review the diff before committing — a surprising change often indicates a
regression in the stub, not a fixture refresh.

## tasks

Canonical extension recognising Obsidian-style task-list syntax. The
extension is a thin stub: it captures task syntax from the source,
emits structured template vars for the theme to render, and ships CSS
+ templates for the default render shape. The Obsidian Tasks
query-engine subset (SPEC-033 §13 Q4) is deferred — until that subset
lands the matrix tier stays `experimental`.

```markdown
- [ ] Pay rent
- [x] Buy milk @due(2026-04-25)
- [ ] Write weekly report @due(2026-04-30)
```

A contiguous run of lines matching `- [ ]`, `- [x]`, or `- [X]` is one
task list. The optional inline marker `@due(YYYY-MM-DD)` surfaces a due
date; the stub strips it from the visible label and emits a `<time>`
alongside.

### Template vars

The backing hook emits a `template_vars` payload that the default theme
reads as:

- `page.ext.tasks.open` — array of open (`[ ]`) tasks.
- `page.ext.tasks.done` — array of completed (`[x]` / `[X]`) tasks.
- `page.ext.tasks.due` — array of tasks (open or done) that carry a
  `@due(...)` marker, typically sorted ascending by date.

Each array element is an object with at least `{text, done, due}`. The
default theme partial iterates `page.ext.tasks.open` and `.done` to
render the lists in-place; theme authors can opt into a summary view by
reading `page.ext.tasks.due`.

### Selector

A realistic default-theme selector matches any page whose body contains
a line matching `^- \[[ xX]\] ` (the three accepted checkbox forms). The
selector is gated by the extension's manifest (not yet landed) and
observed per page via `zetl hook coverage --vault .`.

Per-page opt-out works the same as every canonical extension: frontmatter
key `extensions.tasks: false`. Per-list filtering (`filter: "not done"`)
is honoured by the backing ecosystem plugin, not the stub.

### HTML output shape

Both completed and open tasks render through the shared list shape:

```html
<ul class="tasks">
<li class="task task-open"><input disabled type="checkbox"> Pay rent</li>
<li class="task task-done"><input checked disabled type="checkbox"> Buy milk <time class="task-due" datetime="2026-04-25">2026-04-25</time></li>
</ul>
```

Inline markdown inside the label (bold, emphasis, inline code, links)
is rendered normally. The `<input>` element is always `disabled` — task
toggling is an authoring-time concern, not a runtime one.

### Backing ecosystem plugin

The theme stub ships CSS + templates only; real vaults run the
task-capture + template-var emission through an ecosystem plugin.
Recommended backings once SPEC-033 §13 Q4 lands:

- **Obsidian Tasks subset** — filter/sort/aggregate syntax matching a
  well-defined subset of the Obsidian Tasks plugin.

Without a backing plugin installed, the theme's degraded-render
fallback still produces the `<ul class="tasks">` shape via the in-repo
stub — the gate that verifies this contract lives at
`tests/extension-fixtures/tasks/` and is driven by the shared
golden-HTML harness (SPEC-032 CON-3212).

### Golden-HTML fixture

- Input: `tests/extension-fixtures/tasks/input.md`
- Expected HTML: `tests/extension-fixtures/tasks/expected.html`
- Selector-match: `tests/extension-fixtures/tasks/selector-match.txt`

Run the gate:

```
cargo test --test ext_golden_html_integration
```

Regenerate the expected after a stub change:

```
cargo xtask update-golden tasks
```

Review the diff before committing — a surprising change often indicates
a regression in the stub, not a fixture refresh.
