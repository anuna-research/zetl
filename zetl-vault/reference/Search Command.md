# Search Command

`zetl search` provides full-text content search across all Markdown files in the vault.

```spl
(given fulltext-search-done)
(given regex-support)
```

## Usage

```bash
# Literal text search
zetl -d . search "defeasible"

# Regex search
zetl -d . search "reason(ing|ed)" --regex

# With surrounding context
zetl -d . search "Cache" --context 100

# Case-sensitive
zetl -d . search "SPL" --case-sensitive

# Restrict to a subdirectory
zetl -d . search "given" --path "theories/*"
```

## Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--context N` | 0 | Include N characters of surrounding text |
| `--limit N` | 50 | Max results to return |
| `--regex` | off | Interpret query as a regular expression |
| `--case-sensitive` | off | Require exact case match |
| `--all` | off | Search raw content (include frontmatter, code blocks) |
| `--path <glob>` | none | Restrict results to files matching glob |

## Content filtering

By default, search skips frontmatter and fenced code blocks to focus on prose. Use `--all` to search raw file content including [[concepts/Spindle Lisp]] blocks and YAML frontmatter.

## Design decision

zetl uses index-free scanning rather than an inverted index. This trades query speed on very large vaults for zero build-time indexing cost and simpler architecture. See [[ADR-002 Search Without Index]] for the rationale.

## Fuzzy page name matching

For finding pages by name rather than content, use [[Similar Command]] instead:

```bash
zetl -d . similar "reasning engine"
```

See also: [[CLI Reference]], [[Similar Command]], [[Links Command]], [[TUI]]
