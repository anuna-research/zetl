# Wikilinks

Wikilinks are inline references between pages using `[[double bracket]]` syntax. They are the primary connective tissue in a ztl vault, forming the edges of the [[Link Graph]].

```spl
(given wikilink-syntax-documented)
```

## Syntax

| Form | Example | Description |
|------|---------|-------------|
| Basic | `[[Cache]]` | Link to a page |
| Aliased | `[[Cache\|caching layer]]` | Display text differs from target |
| Heading | `[[Cache#Design tension]]` | Link to a specific heading |
| Block | `[[Cache^summary]]` | Link to a block ID |
| Embed | `![[Cache]]` | Embed the target page inline |
| Self-heading | `[[#Syntax]]` | Link to heading in same page |

## Formal grammar

```
wikilink     = "[[" target ("#" fragment)? ("|" alias)? "]]"
embed        = "![[" target "]]"
target       = page-name
fragment     = heading | "^" block-id
heading      = text
block-id     = alphanumeric-with-hyphens
alias        = text
page-name    = text (may include "/" for path-qualified links)
```

## Parsing rules

The [[architecture/Scanner]] extracts wikilinks from every Markdown file. Wikilinks inside the following zones are **ignored**:

- Fenced code blocks (`` ``` ``)
- Inline code (`` ` ``)
- HTML comments (`<!-- -->`)
- YAML frontmatter (`---` fences)

## Resolution

Page names are matched case-insensitively. `[[cache]]` resolves to `Cache.md`. When ambiguity exists (e.g., `concepts/SimHash.md` and `architecture/SimHash.md`), use path-qualified links: `[[concepts/SimHash]]` or `[[architecture/SimHash]]`.

## Cross-referencing with logic

When reasoning is enabled, `--with-conclusions` annotates link results with the [[concepts/Spindle Lisp]] conclusions each linked page contributes. This bridges the [[Link Graph]] and the [[Reasoning Engine]].

## Compatibility

ztl supports the wikilink conventions used by Obsidian, Logseq, Foam, and Dendron. See [[Compatibility]] and [[Local-first Design]].

See also: [[Link Graph]], [[architecture/Scanner]], [[Links Command]]
