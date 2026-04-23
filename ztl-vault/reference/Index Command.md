# Index Command

`ztl index` builds or refreshes the link index for a vault.

## Usage

```bash
ztl -d ./my-vault index
```

## What it does

1. Scans all `.md` files in the vault directory (recursively)
2. Parses each file with pulldown-cmark, extracting [[concepts/Wikilinks]] and [[concepts/Spindle Lisp]] blocks — see [[architecture/Scanner]]
3. Builds the [[Link Graph]] — a directed graph of page-to-page links
4. Computes [[Merkle Tree]] hashes for content-addressable blocks
5. Writes the index to `.ztl/index.json` — see [[architecture/Cache]]

## Incremental behavior

On subsequent runs, ztl uses two-tier cache invalidation:

- **Tier 1 (mtime):** If a file's modification time hasn't changed, skip it entirely
- **Tier 2 (hash):** If the mtime changed but the content hash matches, skip reprocessing

This makes re-indexing fast even for large vaults. Use `--no-cache` to force a full rescan.

## Auto-snapshotting

When built with `--features history`, `ztl index` automatically creates a jj snapshot after indexing. Snapshots are deduplicated — if the vault root hash hasn't changed since the last snapshot, no new snapshot is created. See [[History Command]].

## Verbose output

With `-v`, ztl reports scan statistics to stderr: files scanned, files skipped, links found, SPL blocks extracted, and timing. With history enabled, also reports snapshot creation timing.

See also: [[CLI Reference]], [[architecture/Scanner]], [[architecture/Cache]], [[Stats Command]], [[History Command]]
