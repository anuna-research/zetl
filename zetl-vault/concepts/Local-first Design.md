# Local-first Design

zetl follows five local-first principles that make it safe to use with your knowledge base.

```spl
(given read-only-vault-access)
(given no-network-calls)
(given local-first-documented)
```

## Five principles

### 1. Read-only

zetl only reads Markdown and `.spl` files. It never writes to, renames, or deletes vault content. The only exception is the inline edit feature in [[Serve Command]], which writes only when the user explicitly saves.

### 2. Disposable cache

The `.zetl/` directory contains only derived data (the [[Link Graph]] index and [[Reasoning Engine]] theory cache). Deleting it loses nothing — `zetl index` regenerates it. Add `.zetl/` to your `.gitignore`.

### 3. No network

zetl makes no network calls. Everything runs locally. The [[Serve Command]] listens on localhost only. Future distributed sync (see [[Distributed Sync Future]]) would be opt-in via a separate sidecar.

### 4. No lock-in

Your vault is plain Markdown with optional [[concepts/Spindle Lisp]] blocks. Removing zetl leaves your files untouched. The `.zetl/` directory can be deleted without consequence.

### 5. Cross-tool compatibility

zetl works alongside Obsidian, Logseq, Foam, Dendron, or any editor. Multiple tools can read the same vault simultaneously without conflict. See [[Compatibility]].

## Why this matters

Users trust zetl with their knowledge base — years of accumulated notes. A tool that might corrupt, reformat, or accidentally delete files would be a non-starter. Read-only access removes that risk entirely.

See also: [[decisions/Local-first Design]], [[Compatibility]], [[architecture/Cache]]
