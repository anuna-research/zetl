# Diff Command

`zetl diff` computes a graph-level diff against Git history, showing what changed in the link structure between two points in time.

## Usage

```bash
# Diff against previous commit (default: HEAD~1)
zetl -d ./my-vault diff

# Diff against a specific ref
zetl diff --from main~5

# Diff by date
zetl diff --since "2026-02-01"

# Filter by change category
zetl diff --filter links
```

## Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--from <ref>` | `HEAD~1` | Git ref to diff against |
| `--since <datetime>` | none | Find closest commit at or before this date |
| `--filter <category>` | none | Filter output: `pages`, `links`, `orphans`, `dead_links` |

## How it works

When built with `--features history`, zetl uses jj-lib as the primary diff backend, defaulting to the `@-` (previous change) baseline. The `--from` flag accepts jj change-IDs and time expressions in addition to Git refs.

Without the history feature, zetl falls back to an efficient Git-based reconstruction algorithm (see [[ADR-012 Changed Files Reconstruction]]):

1. `git diff --name-only` identifies changed `.md` files between the baseline and HEAD
2. `git show` retrieves the old content of each changed file
3. Old content is parsed for wikilinks
4. Set differences between old and new graphs yield the diff

This scales with the size of the change, not the size of the vault.

## Output

The diff reports:

- **Pages** added and removed
- **Links** added and removed
- **Orphans** gained and resolved
- **Dead links** added and resolved

## Design decision

Without the history feature, zetl uses Git as its diff backend. See [[ADR-011 No Snapshots]] for the original rationale. With `--features history`, diff uses jj-lib instead, providing richer temporal querying and consistency with the [[History Command]] snapshot system.

See also: [[CLI Reference]], [[Watch Command]], [[Check Command]], [[History Command]], [[SPEC-007 Graph Diff]]
