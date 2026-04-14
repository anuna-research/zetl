# Merkle Tree

zetl builds a content-addressed Merkle tree over each Markdown file during indexing. This enables two-tier cache invalidation, SPL grounding, and [[Drift Detection]].

```spl
(given content-addressed-hashing)
```

## Structure

Each file produces a flat list of Merkle leaf nodes, computed from pulldown-cmark AST block-level elements:

| Leaf type | Source |
|-----------|--------|
| Heading | `## Section Title` |
| Paragraph | Prose text blocks |
| SplBlock | `` ```spl `` fenced code blocks |
| Code | Non-SPL fenced code blocks |
| Table | Markdown tables |
| List | Ordered/unordered lists |
| Blockquote | `>` block quotes |
| Frontmatter | YAML between `---` fences |

## BLAKE3 hashing

All leaf hashes use BLAKE3, chosen for speed and cryptographic strength. Content is normalized before hashing (whitespace trimmed, consistent line endings) so that formatting-only changes don't produce different hashes.

## SPL dual hashing

SPL blocks receive two hashes:

- **content_hash** — hash of the raw SPL text (changes on any edit, including comments)
- **ast_hash** — hash of the parsed SPL AST (changes only on logical modifications)

The AST hash drives theory cache invalidation — see [[architecture/Cache]]. The content hash detects any change for drift purposes.

## File and vault roots

Leaf hashes are combined into a per-file Merkle root. File roots are combined into a vault root hash, exposed in [[Stats Command]] as `vault_content_hash`.

## Section grounding

SPL blocks are implicitly grounded in their containing section. The grounding hash is computed from the non-SPL leaves in the same section (heading through next heading). When the prose changes but the SPL doesn't, this constitutes drift — see [[Drift Detection]].

## Explicit grounding

Facts can be explicitly grounded to specific content via `(meta label (source "ref"))`, where `ref` can be a `^block-id`, a Merkle hash prefix, or a cross-file `[[Page^block-id]]` reference. See [[Blocks Command]] for discovering hashes.

See also: [[architecture/Cache]], [[Drift Detection]], [[Blocks Command]], [[SPEC-006 Merkle Tree]]
