# SPEC-007: Graph Diff

Adds graph-level diffing against Git history. See [[Diff Command]] for the CLI.

```spl
(given spec-007-documented)
```

## Overview

`zetl diff` reconstructs the vault graph at a Git ref by re-parsing only changed files, then computes a structural diff showing what changed in the link graph. This is the only zetl command that requires Git.

## Key requirements

| ID | Requirement |
|----|-------------|
| REQ-046 | Git ref baseline (`--from <ref>`) |
| REQ-047 | Since-date baseline (`--since <date>`) |
| REQ-048 | Default baseline (HEAD~1) |
| REQ-049 | Structured diff output |
| REQ-050 | Diff filter by category |
| REQ-051 | Git unavailable error |
| REQ-052 | Efficient reconstruction |

## Reconstruction algorithm

See [[ADR-012 Changed Files Reconstruction]]:

1. `git diff --name-only` identifies changed `.md` files
2. `git show` reads old content of each changed file
3. Old content parsed for wikilinks
4. Set differences yield the diff

## Diff output

- Pages added / removed
- Links added / removed
- Orphans gained / resolved
- Dead links added / resolved

## Architecture decisions

- [[ADR-011 No Snapshots]] — Git is the history backend
- [[ADR-012 Changed Files Reconstruction]] — reconstruct only changed files

## Performance

Diff completes in < 500ms for 2,000 pages with 50 changed files. Scales with change size, not vault size.

## User profiles

- **Knowledge worker** reviewing what changed in their vault since yesterday
- **Dev agent** running incremental reasoning cycles

See also: [[Spec Index]], [[Diff Command]], [[Watch Command]]
