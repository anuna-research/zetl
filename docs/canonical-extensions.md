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
