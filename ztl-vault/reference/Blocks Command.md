# Blocks Command

`ztl blocks` lists content-addressable [[Merkle Tree]] blocks for a page (forward mode) or resolves a block by its BLAKE3 hash prefix (reverse mode).

## Usage

```bash
# Forward mode: list all blocks for a page
ztl -d ./my-vault blocks "Scanner"

# Filter by type
ztl blocks "Scanner" --type heading

# Reverse mode: resolve a hash to its source
ztl blocks --resolve abc123de
```

## Forward mode

Given a page name, returns all Merkle leaf nodes in document order. Each block includes:

- **type** — Heading, Paragraph, SplBlock, Code, Table, List, Blockquote, Frontmatter
- **lines** — start and end line numbers
- **hash** — BLAKE3 content hash
- **text** — preview of the block content
- **spl_hashes** — for SPL blocks: `content_hash` and `ast_hash` (dual hashing)

### Block type filter

| `--type` value | Matches |
|----------------|---------|
| `heading` | Heading nodes |
| `paragraph` | Paragraph nodes |
| `spl` | SPL code blocks |
| `code` | Non-SPL code blocks |
| `table` | Table nodes |
| `list` | List nodes |
| `blockquote` | Blockquote nodes |
| `frontmatter` | YAML frontmatter |
| `all` | All block types (default) |

## Reverse mode

Given a hash prefix (minimum 8 hex characters), finds the source location:

```bash
ztl blocks --resolve e5f6a7b8
```

Returns the file, page name, line range, block type, and content. Useful for tracing [[concepts/Provenance]] from a hash back to its source.

Identical content at multiple locations produces identical hashes — this is by design, not an error. Ambiguous prefixes (different full hashes sharing a prefix) return an error suggesting a longer prefix.

## SPL grounding workflow

The hash values from `blocks` can be used as explicit source metadata in [[concepts/Spindle Lisp]]:

```spl
(given redis-fast-enough)
(meta redis-fast-enough (source "e5f6a7b8"))
```

This creates an explicit grounding link between the fact and the prose block. See [[Drift Detection]] for how grounding freshness is tracked.

See also: [[CLI Reference]], [[Merkle Tree]], [[concepts/Provenance]], [[Check Command]]
