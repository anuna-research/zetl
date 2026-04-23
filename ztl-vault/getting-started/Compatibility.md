# Compatibility

ztl works with any Markdown vault that uses `[[wikilink]]` syntax. It never modifies your files.

```spl
(given obsidian-compatible)
(given logseq-compatible)
(given foam-compatible)
(given dendron-compatible)
```

## Supported tools

| Tool | Status | Notes |
|------|--------|-------|
| **Obsidian** | Full support | Standard `[[wikilink]]` syntax, aliases, headings, block references |
| **Logseq** | Full support | Wikilinks in Markdown mode |
| **Foam** | Full support | VS Code extension using `[[wikilinks]]` |
| **Dendron** | Full support | VS Code extension with Markdown wikilinks |

## What ztl reads

ztl parses `[[target]]`, `[[target|alias]]`, `[[target#heading]]`, `[[target^block-id]]`, and `![[embeds]]`. It identifies pages by filename (case-insensitive, `.md` extension stripped). See [[concepts/Wikilinks]] for the full grammar.

## What ztl ignores

- Frontmatter (YAML between `---` fences) — parsed but not treated as links
- Code blocks — wikilinks inside fenced code are ignored
- Comments — HTML comments are skipped

## SPL is optional

[[concepts/Spindle Lisp]] embedding is entirely optional. Vaults work fine with just wikilinks — SPL adds reasoning capabilities on top. If you don't use SPL, commands like `ztl links`, `ztl check`, and `ztl tui` work identically.

## The `.ztl/` directory

The index and theory cache are stored in `.ztl/` at the vault root. This directory is disposable — delete it any time, and `ztl index` rebuilds it. Add `.ztl/` to your `.gitignore`. See [[architecture/Cache]].

See also: [[Install]], [[concepts/Wikilinks]], [[Local-first Design]]
