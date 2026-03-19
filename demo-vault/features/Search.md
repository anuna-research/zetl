---
title: Search
---

# Searchs

`zetl search` provides full-text content search across all Markdown files in the vault. [[search-history]]

```spl
(given hello)
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

## Content filtering

By default, search skips frontmatter and fenced code blocks to focus on prose. Use `--all` to search raw file content including [[Spindle Lisp]] blocks and YAML frontmatter.

## Fuzzy page name matching

For finding pages by name rather than content, use `zetl similar`:

```bash
zetl -d . similar "reasning engine"
```

This uses SimHash-based similarity to find close matches even with typos.

See also: [[Graph Queries]], [[TUI]], [[Scanner]]
