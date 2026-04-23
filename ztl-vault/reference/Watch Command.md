# Watch Command

`ztl watch` starts a persistent process that monitors the vault for file changes and emits graph-level events as NDJSON on stdout.

## Usage

```bash
# Start watching (NDJSON events on stdout)
ztl -d ./my-vault watch

# Custom debounce interval
ztl watch --debounce 500

# Execute a command on each change
ztl watch --exec "ztl check --fail-on error"
```

## Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--debounce <ms>` | 200 | Debounce interval for batching rapid changes |
| `--exec <cmd>` | none | Command to invoke after each re-index |

## Event types

Events are emitted as NDJSON (one JSON object per line). The event vocabulary mirrors the [[Diff Command]] output schema:

| Event | Description |
|-------|-------------|
| `index_ready` | Initial index complete, watching started |
| `page_added` | New page appeared |
| `page_removed` | Page deleted |
| `link_added` | New wikilink appeared |
| `link_removed` | Wikilink removed |
| `orphan_gained` | Page became orphaned |
| `orphan_resolved` | Orphan page got an incoming link |
| `dead_link_added` | New dead link appeared |
| `dead_link_resolved` | Dead link target now exists |
| `index_updated` | Re-index cycle complete (summary) |

## How it works

ztl uses OS-level file system events (via the `notify` crate) to detect changes. When a `.md` file changes, ztl debounces the event, incrementally re-indexes only the affected files using the [[Merkle Tree]] change detection from [[architecture/Cache]], and emits events only when the graph actually changes.

## Auto-snapshotting

When built with `--features history`, each re-index cycle automatically creates a jj snapshot (deduplicated by vault root hash). This builds a continuous history timeline that can be queried with `ztl history log` or the `--at` flag. See [[History Command]].

## Agent integration

The NDJSON stream is designed for consumption by AI agents and automation scripts. Pipe it to tools like `jq` or feed it into an incremental reasoning loop.

See also: [[CLI Reference]], [[Diff Command]], [[History Command]], [[SPEC-008 Watch Mode]]
