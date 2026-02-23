---
title: Local-first Design
---

# Local-first Design

zetl never modifies your files. It is strictly read-only against the vault.

```spl
(given read-only-vault-access)
(given disposable-cache)
```

## Principles

- **Read-only** — zetl only reads Markdown and `.spl` files. It never writes to, renames, or deletes vault content.
- **Disposable cache** — the `.zetl/` directory contains only derived data (the [[Link Graph]] index and [[Reasoning Engine]] theory cache). Deleting it loses nothing; `zetl index` regenerates it.
- **No network** — zetl makes no network calls. Everything runs locally.
- **No lock-in** — your vault is plain Markdown with optional [[Spindle Lisp]] blocks. Removing zetl leaves your files untouched.

## Why this matters

Users trust zetl with their knowledge base — years of accumulated notes. A tool that might corrupt, reformat, or accidentally delete files would be a non-starter. Read-only access removes that risk entirely.

The [[Cache]] is the only thing zetl writes, and it lives in a clearly-marked directory that can be gitignored.

## Compatibility

This design means zetl works alongside Obsidian, Logseq, Foam, Dendron, or any editor. Multiple tools can read the same vault simultaneously without conflict.

See also: [[Cache]], [[Scanner]], [[Rust for CLI]]
