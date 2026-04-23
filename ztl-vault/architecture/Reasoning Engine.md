# Reasoning Engine

The reasoning engine takes [[concepts/Spindle Lisp]] extracted by the [[Scanner]] from across the entire vault, merges it into a single theory, and computes conclusions using [[concepts/Defeasible Reasoning]]. Every conclusion carries full [[concepts/Provenance]].

```spl
(given spindle-core-integrated)
(given four-conclusion-types)
```

## Pipeline

1. **Extract** — the [[Scanner]] finds SPL blocks in Markdown fences and standalone `.spl` files
2. **Parse** — `spindle-parser` converts SPL text into rule objects
3. **Provenance** — source file, line number, and page name are attached to each rule and fact
4. **Merge** — all rules and facts are merged into a single vault-wide theory
5. **Validate** — check for undefined labels, duplicate definitions, broken source references
6. **Reason** — `spindle-core` computes conclusions using the defeasible reasoning algorithm
7. **Annotate** — each conclusion is traced back to its proof sources with grounding freshness

## Conclusion types

| Tag | Meaning |
|-----|---------|
| `+D` | Definitely provable — strict derivation, no defeating possible |
| `-D` | Definitely not provable |
| `+d` | Defeasibly provable — inferred, no active defeaters |
| `-d` | Defeasibly not provable — blocked or no derivation path |

## Theory caching

The merged theory and its conclusions are cached in `.ztl/theory.json`. Invalidation uses the SPL AST hash — prose-only edits don't trigger a rebuild. See [[architecture/Cache]] and [[Merkle Tree]].

## Feature gate

The reasoning engine is compiled only when `--features reason` is enabled — see [[Feature Gates]]. Without it, `ztl reason` prints a clear error rather than failing silently.

## Cross-referencing

The `--with-conclusions` flag on [[Links Command]] bridges the structural graph with the logical theory, showing what each linked page contributes to the vault's conclusions.

See also: [[Reason Commands]], [[concepts/Provenance]], [[concepts/Spindle Lisp]], [[concepts/Defeasible Reasoning]]
