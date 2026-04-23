# History Command

`ztl history` provides commands for browsing vault history. Requires `--features history` at build time. See [[Feature Gates]].

History is powered by jj-lib, which silently initializes a VCS repository at `.ztl/jj/` on first use. Snapshots are created automatically during `ztl index` and `ztl watch`.

## Subcommands

### `history log`

Reverse-chronological timeline of graph-level deltas. Each entry shows what changed between consecutive snapshots: pages added/removed and net link-count deltas. Identical vault states (same root hash) are collapsed.

```bash
ztl -d ./my-vault history log
ztl history log --since "last week"
ztl history log --limit 50
```

| Flag | Default | Description |
|------|---------|-------------|
| `--since <time-expr>` | none | Show only snapshots since this time expression |
| `--limit N` | 20 | Maximum entries to show |

### `history page`

Shows the evolution of a specific page across snapshots — when it was created, modified, and how its link count changed over time.

```bash
ztl history page "Scanner"
ztl history page "Cache" --limit 10
```

| Flag | Default | Description |
|------|---------|-------------|
| `--limit N` | 20 | Maximum snapshots to show |

### `history timeline`

Lists recent snapshots with timestamps and brief graph statistics.

```bash
ztl history timeline
ztl history timeline --limit 50
```

| Flag | Default | Description |
|------|---------|-------------|
| `--limit N` | 20 | Maximum snapshots to show |

## Time-travel with `--at`

The global `--at` flag allows querying vault state at any historical point in time. It works on all read-only subcommands:

```bash
ztl --at "3 days ago" links "Scanner"
ztl --at "2024-01-15" stats
ztl --at "last monday" check
ztl --at "HEAD~3" backlinks "Cache"
```

Time expressions accept:

- **ISO 8601 dates** — `2024-01-15`, `2024-01-15T10:30:00`
- **Relative natural language** — `3 days ago`, `last monday`, `yesterday`
- **VCS refs** — `HEAD~1`, change-ID prefixes

The `--at` flag resolves the vault to the closest snapshot at or before the given time, then runs the command against that historical state. Results are served from an LRU cache for performance.

## Template context

When history is available, templates receive additional variables:

**`vault.history`** — vault-level history summary (snapshot count, trend points, oldest/newest timestamps)

**`page.history`** — per-page history (created_at, last_changed, age_days, stable_days, link_trend, recent_changes)

**`page.backlinks[].since`** — RFC 3339 timestamp of when each backlink first appeared (null when history is unavailable)

## History API (serve mode)

When running `ztl serve` with history enabled, the following API endpoints are available:

| Endpoint | Description |
|----------|-------------|
| `GET /api/history` | Graph-level delta log |
| `GET /api/history/page/:name` | Page evolution timeline |
| `GET /api/history/at?expr=<time>` | Resolve a time expression to snapshot metadata |
| `GET /api/history/diff?from=<expr>&to=<expr>` | Diff between two time expressions |

## Build export

`ztl build` writes `history-index.json` to the output directory containing vault-level trend data and per-page history summaries. This can be consumed by client-side JavaScript for history visualizations.

## Graceful degradation

When the `history` feature is not compiled in, or when jj-lib encounters an error:

- `vault.history` and `page.history` template variables are null
- `page.backlinks[].since` is null
- History API endpoints return empty results
- The `--at` flag is not available
- All other commands continue to work normally

See also: [[CLI Reference]], [[Feature Gates]], [[Watch Command]], [[Diff Command]]
