# zetl

Bi-directional wikilink graph CLI with defeasible reasoning for personal knowledge management.

zetl parses `[[wikilinks]]` from Markdown files, builds an in-memory link graph, and exposes query, validation, search, and visualization commands. Optionally, it extracts [[concepts/Spindle Lisp]] code blocks from your vault and performs [[concepts/Defeasible Reasoning]] — drawing conclusions that can be defeated by stronger evidence.

```spl
(given vault-self-documenting)
(given vault-uses-wikilinks)
(given vault-uses-spl)
```

## Vault Map

### Getting Started

- [[Install]] — Rust toolchain, `make install`, `--features reason`
- [[Quick Start]] — five-step tour of zetl's core commands
- [[Demo Vault]] — the included demo-vault and how to explore it
- [[Compatibility]] — Obsidian, Logseq, Foam, and Dendron support

### Reference

- [[CLI Reference]] — global flags, exit codes, JSON-by-default
- [[Index Command]], [[Links Command]], [[Path Command]], [[Search Command]]
- [[Similar Command]], [[Check Command]], [[Blocks Command]], [[Diff Command]]
- [[Watch Command]], [[List and Export Commands]], [[Stats Command]], [[History Command]]
- [[TUI]], [[View Command]], [[Serve Command]], [[Build Command]]
- [[Reason Commands]] — all eight reasoning subcommands

### Architecture

- [[architecture/Scanner]] — pulldown-cmark, wikilink and SPL extraction
- [[Link Graph]] — petgraph DiGraph, resolution, query API
- [[architecture/Cache]] — two-tier invalidation, `.zetl/` schemas
- [[architecture/SimHash]] — trigram hashing, Hamming distance
- [[Merkle Tree]] — BLAKE3 leaves, section grounding, drift detection
- [[Reasoning Engine]] — SPL pipeline: extract, parse, merge, reason, annotate
- [[Drift Detection]] — implicit/explicit grounding, `check --drift`
- [[architecture/Performance]] — NFR targets, measured baselines

### Concepts

- [[concepts/Wikilinks]] — syntax reference and formal grammar
- [[concepts/Defeasible Reasoning]] — rule types, conclusion tags, superiority
- [[concepts/Spindle Lisp]] — full SPL syntax reference and embedding guide
- [[concepts/Provenance]] — what is tracked, commands, cross-referencing
- [[concepts/SimHash]] — conceptual explanation of fuzzy matching
- [[Xanadu Lineage]] — Ted Nelson's transclusion vision and zetl view
- [[Local-first Design]] — five principles: read-only, disposable, no network

### Decisions

- [[ADR-001 Rust]], [[ADR-002 Search Without Index]], [[ADR-003 JSON Errors]]
- [[ADR-004 Link Dedup]], [[ADR-011 No Snapshots]], [[ADR-012 Changed Files Reconstruction]]
- [[JSON by Default]], [[Feature Gates]], [[decisions/Local-first Design]]
- [[Agent Ergonomics]], [[Distributed Sync Future]], [[Xanadu View Design]]

### Specs

- [[Spec Index]] — all nine specifications with status
- [[SPEC-001 Link Graph CLI]], [[SPEC-002 Full-Text Search]]
- [[SPEC-003 Agent Ergonomics]], [[SPEC-004 Distributed Sync]]
- [[SPEC-005 Defeasible Reasoning]], [[SPEC-006 Merkle Tree]]
- [[SPEC-007 Graph Diff]], [[SPEC-008 Watch Mode]], [[SPEC-009 Xanadu View]]

### Theories

SPL rules that reason across the entire vault:

- `theories/vault-readiness.spl` — derives `vault-ready-to-publish`
- `theories/design-principles.spl` — design philosophy as defeasible rules
- `theories/caching.spl` — caching theory with intentional conflict
- `theories/agent-ergonomics.spl` — agent-friendliness modelled as rules

```spl
; This vault reasons about its own completeness.
; Run: zetl reason explain "vault-ready-to-publish" --format natural
(normally r-self-documenting
  (and vault-self-documenting vault-uses-wikilinks vault-uses-spl)
  vault-is-self-referential)
```
