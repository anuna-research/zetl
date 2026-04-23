# Path Command

`ztl path` finds the shortest chain of [[concepts/Wikilinks]] connecting two pages in the [[Link Graph]].

## Usage

```bash
ztl -d ./my-vault path "Scanner" "Provenance"
```

## Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--max-depth N` | 10 | Maximum path length to search |

## How it works

ztl performs a breadth-first search on the directed link graph from the source page toward the target. The result is the shortest sequence of pages connecting them.

This is useful for discovering how ideas relate through intermediate notes — a form of knowledge exploration that leverages the graph structure built from wikilinks.

## Output

JSON output includes the ordered list of pages in the path and the total hop count. Table output shows the path as an arrow-separated chain.

If no path exists within `--max-depth` hops, ztl reports that the pages are not connected.

See also: [[CLI Reference]], [[Links Command]], [[Link Graph]]
