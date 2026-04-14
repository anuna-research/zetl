# Scanner

The scanner is zetl's entry point for understanding a vault. It walks every Markdown file and standalone `.spl` file, extracting [[concepts/Wikilinks]] and [[concepts/Spindle Lisp]] blocks in a single pass.

```spl
(given wikilink-extraction)
(given spl-extraction)
```

## Wikilink extraction

The scanner recognises all common wikilink forms:

- `[[Page]]` — basic link
- `[[Page|alias]]` — aliased link
- `[[Page#heading]]` — heading anchor
- `[[Page^block-id]]` — block reference
- `![[Page]]` — embed

Extracted links feed into the [[Link Graph]] for query and traversal.

## Exclusion zones

The scanner skips wikilink-like patterns that appear inside:

- Fenced code blocks (`` ``` ``)
- Inline code (`` ` ``)
- HTML comments (`<!-- -->`)
- YAML frontmatter (`---` fences)

This prevents false positives from code examples or metadata.

## SPL extraction

Fenced code blocks tagged `` ```spl `` are extracted verbatim, along with their source file and line number. Standalone `.spl` files anywhere in the vault are also picked up. Both feed into the [[Reasoning Engine]] with full [[concepts/Provenance]].

## Merkle tree construction

During scanning, the scanner groups pulldown-cmark AST nodes into [[Merkle Tree]] leaf nodes (headings, paragraphs, code blocks, tables, lists, etc.) and computes BLAKE3 hashes. SPL blocks receive dual hashing: a content hash and an AST hash — the latter enables theory cache invalidation even when formatting changes. See [[architecture/Cache]].

## Implementation

Built on `pulldown-cmark` for Markdown parsing and `ignore` for `.gitignore`-aware file walking. The scanner is deliberately read-only — see [[Local-first Design]].

## Performance

The scanner processes over 2,000 files per second on commodity hardware. Incremental scanning (via mtime-based cache) reduces re-scan time by 99% on typical editing sessions. See [[Performance]].

See also: [[Link Graph]], [[architecture/Cache]], [[Reasoning Engine]], [[Merkle Tree]]
