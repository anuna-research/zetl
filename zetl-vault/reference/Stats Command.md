# Stats Command

`zetl stats` prints summary statistics about the vault.

## Usage

```bash
zetl -d ./my-vault stats
zetl stats --top 20 --format table
```

## Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--top N` | 10 | Number of most-linked pages to show |

## Output

The stats include:

- **pages** — total page count
- **links** — total wikilink count
- **orphans** — pages with no incoming links
- **most_linked** — top N pages by incoming link count
- **vault_content_hash** — BLAKE3 root hash of the entire vault (from [[Merkle Tree]])
- **spl_blocks** — total SPL block count
- **grounded_spl_blocks** — SPL blocks with section or explicit grounding
- **explicitly_grounded_facts** — facts with `(meta ... (source ...))` references

When built with `--features history`, stats also includes a **history** section:

- **snapshot_count** — total number of jj snapshots
- **unique_states** — distinct vault states (by root hash)
- **oldest** / **newest** — timestamp range of snapshots
- **cache_entries** — number of cached historical indexes
- **cache_size_mb** — total size of the historical index cache

The vault content hash provides an integrity checkpoint — agents can use it to verify that the vault state hasn't changed between operations.

See also: [[CLI Reference]], [[List and Export Commands]], [[Merkle Tree]], [[History Command]]
