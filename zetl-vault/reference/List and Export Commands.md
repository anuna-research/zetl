# List and Export Commands

Two utility commands for enumerating vault contents.

## `zetl list`

Lists all pages in the vault.

```bash
zetl -d ./my-vault list
```

Returns page names and file paths. Useful for agent workflows that need to enumerate available pages before querying them.

Added in response to agent ergonomics findings — see [[Agent Ergonomics]] and [[SPEC-003 Agent Ergonomics]].

## `zetl export`

Exports the complete [[Link Graph]] as JSON — every page and every link.

```bash
zetl -d ./my-vault export
```

The output is a full adjacency list of the vault's link structure. This enables external tools to consume zetl's graph data for visualization, analysis, or integration.

## Relationship to other commands

| Need | Command |
|------|---------|
| Just page names | `list` |
| Full graph structure | `export` |
| Summary statistics | [[Stats Command]] |
| Query specific links | [[Links Command]] |

See also: [[CLI Reference]], [[Stats Command]], [[Agent Ergonomics]]
