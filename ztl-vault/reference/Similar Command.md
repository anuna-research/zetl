# Similar Command

`ztl similar` finds pages with names similar to a query string, using [[concepts/SimHash]]-based fuzzy matching.

## Usage

```bash
ztl -d ./my-vault similar "zettelkasen"
```

## Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--threshold N` | 12 | Max Hamming distance (lower = stricter) |
| `--limit N` | 10 | Max results |

## How it works

ztl computes a SimHash fingerprint for each page name using character trigrams, then compares the query's fingerprint against all pages using Hamming distance. Pages within the threshold are returned, sorted by similarity.

This is useful for:

- Finding pages despite typos
- Discovering pages with similar names that might be candidates for merging
- Fuzzy lookup when you don't remember the exact page title

## Relationship to search

`similar` matches page **names**. [[Search Command]] matches page **content**. Use `similar` when you know roughly what a page is called; use `search` when you know something the page contains.

See also: [[CLI Reference]], [[concepts/SimHash]], [[architecture/SimHash]], [[Search Command]]
