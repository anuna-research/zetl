# Compatibility

zetl works with any Markdown vault that uses `[[wikilink]]` syntax. It never modifies your files.

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

## What zetl reads

zetl parses `[[target]]`, `[[target|alias]]`, `[[target#heading]]`, `[[target^block-id]]`, and `![[embeds]]`. It identifies pages by filename (case-insensitive, `.md` extension stripped). See [[concepts/Wikilinks]] for the full grammar.

## What zetl ignores

- Frontmatter (YAML between `---` fences) — parsed but not treated as links
- Code blocks — wikilinks inside fenced code are ignored
- Comments — HTML comments are skipped

## SPL is optional

[[concepts/Spindle Lisp]] embedding is entirely optional. Vaults work fine with just wikilinks — SPL adds reasoning capabilities on top. If you don't use SPL, commands like `zetl links`, `zetl check`, and `zetl tui` work identically.

## The `.zetl/` directory

The index and theory cache are stored in `.zetl/` at the vault root. This directory is disposable — delete it any time, and `zetl index` rebuilds it. Add `.zetl/` to your `.gitignore`. See [[architecture/Cache]].

See also: [[Install]], [[concepts/Wikilinks]], [[Local-first Design]]
