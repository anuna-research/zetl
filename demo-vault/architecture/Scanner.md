---
title: Scanner
---

# Scanner

The scanner is zetl's entry point for understanding a vault. It walks every Markdown file and standalone `.spl` file, extracting [[Wikilinks]] and [[Spindle Lisp]] blocks in a single pass.

## Wikilink extraction

The scanner recognises all common wikilink forms:

- `[[Page]]` — basic link
- `[[Page|alias]]` — aliased link
- `[[Page#heading]]` — heading anchor
- `[[Page^block-id]]` — block reference
- `![[Page]]` — embed

Extracted links feed into the [[Link Graph]] for query and traversal.

## SPL extraction

Fenced code blocks tagged ` ```spl ` are extracted verbatim, along with their source file and line number. Standalone `.spl` files anywhere in the vault are also picked up. Both feed into the [[Reasoning Engine]] with full [[Provenance]].

```spl
(given wikilink-extraction)
(given spl-extraction)
```

## Implementation

Built on `pulldown-cmark` for Markdown parsing and `ignore` for `.gitignore`-aware file walking. The scanner is deliberately read-only — see [[Local-first Design]].

See also: [[Cache]], [[Link Graph]], [[Reasoning Engine]]
