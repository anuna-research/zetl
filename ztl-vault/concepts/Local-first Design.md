# Local-first Design

ztl follows five local-first principles that make it safe to use with your knowledge base.

```spl
(given read-only-vault-access)
(given no-network-calls)
(given local-first-documented)
```

## Five principles

### 1. Read-only

ztl only reads Markdown and `.spl` files. It never writes to, renames, or deletes vault content. The only exception is the inline edit feature in [[Serve Command]], which writes only when the user explicitly saves.

### 2. Disposable cache

The `.ztl/` directory contains only derived data (the [[Link Graph]] index and [[Reasoning Engine]] theory cache). Deleting it loses nothing — `ztl index` regenerates it. Add `.ztl/` to your `.gitignore`.

### 3. No network

ztl makes no network calls. Everything runs locally. The [[Serve Command]] listens on localhost only. Future distributed sync (see [[Distributed Sync Future]]) would be opt-in via a separate sidecar.

### 4. No lock-in

Your vault is plain Markdown with optional [[concepts/Spindle Lisp]] blocks. Removing ztl leaves your files untouched. The `.ztl/` directory can be deleted without consequence.

### 5. Cross-tool compatibility

ztl works alongside Obsidian, Logseq, Foam, Dendron, or any editor. Multiple tools can read the same vault simultaneously without conflict. See [[Compatibility]].

## Why this matters

Users trust ztl with their knowledge base — years of accumulated notes. A tool that might corrupt, reformat, or accidentally delete files would be a non-starter. Read-only access removes that risk entirely.

See also: [[decisions/Local-first Design]], [[Compatibility]], [[architecture/Cache]]
