# ADR-012: Changed-Files-Only Reconstruction

**Status:** Accepted

## Context

When computing a graph diff, should ztl check out the full vault at the baseline ref and build a complete graph, or reconstruct only the changed portions?

## Decision

Reconstruct via changed files only:

1. `git diff --name-only` identifies changed `.md` files
2. `git show` retrieves old content of each changed file
3. Old content is parsed for wikilinks
4. Set differences between old and current graphs yield the diff

## Rationale

- Scales with the size of the change, not the size of the vault
- A 50-file change in a 10,000-file vault processes only those 50 files
- No temporary checkout needed — works in the current working directory
- Memory usage proportional to the change set, not the vault

## Trade-offs

- Cannot detect changes in files that were renamed (Git detects renames, but the wikilink graph doesn't track filename history)
- Requires the current index to be up to date

## Performance

For a 2,000-page vault with 50 changed files, diff completes in under 500ms. See [[Performance]] and [[SPEC-007 Graph Diff]].

See also: [[ADR-011 No Snapshots]], [[Diff Command]]
