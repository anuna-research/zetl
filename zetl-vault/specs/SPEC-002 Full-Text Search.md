# SPEC-002: Full-Text Search

Adds `zetl search` for content search across vault Markdown files. See [[Search Command]] for usage.

```spl
(given spec-002-documented)
```

## Motivation

The [[Similar Command]] does page-name matching, not content search. Users need to find where specific text appears in their vault.

## Key requirements

| ID | Requirement |
|----|-------------|
| REQ-013 | Full-text content search |
| REQ-014 | Body-text-only search (skip frontmatter/code by default, `--all` override) |
| REQ-015 | Regex mode |
| REQ-016 | Case-sensitive mode |

## Performance targets

- Search <= 2 seconds for 10,000 files
- Memory <= 50 MB above baseline

## Design decision

[[ADR-002 Search Without Index]] — index-free scan chosen over inverted index for simplicity and correctness. The trade-off is acceptable because:

- Most vaults have fewer than 10,000 files
- Scanning is I/O-bound and fast on SSDs
- No index build step or invalidation logic needed

## Content filtering

By default, search skips frontmatter and fenced code blocks to focus on prose. This prevents noise from YAML metadata and code examples. The `--all` flag overrides this behavior.

## Relationship to similar

`similar` matches page **names** via [[architecture/SimHash]]. `search` matches page **content** via direct scanning. They serve different use cases.

See also: [[Spec Index]], [[Search Command]], [[ADR-002 Search Without Index]]
