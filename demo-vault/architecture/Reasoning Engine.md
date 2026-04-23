---
title: Reasoning Engine
---

# Reasoning Engine

The reasoning engine takes [[Spindle Lisp]] extracted by the [[Scanner]] from across the entire vault, merges it into a single theory, and computes conclusions using [[Defeasible Reasoning]]. Every conclusion carries full [[Provenance]] — you can trace any result back to the exact file and line that contributed to it.

## Pipeline

1. **Extract** — [[Scanner]] finds SPL blocks in Markdown fences and `.spl` files
2. **Parse** — `spindle-parser` converts SPL text into rule objects
3. **Provenance** — source file, line number, and page name are attached to each rule and fact
4. **Combine** — all rules and facts are merged into a single theory
5. **Validate** — check for undefined labels, duplicate definitions
6. **Reason** — `spindle-core` computes conclusions
7. **Annotate** — each conclusion is traced back to its proof sources

```spl
(given spindle-core-integrated)
(given four-conclusion-types)
```

## Conclusion types

The engine produces four types of conclusion — see [[Defeasible Reasoning]] for details:

| Tag | Meaning |
|-----|---------|
| `+D` | Definitely provable |
| `-D` | Definitely not provable |
| `+d` | Defeasibly provable |
| `-d` | Defeasibly not provable |

## Feature gate

The reasoning engine is compiled only when `--features reason` is enabled — see [[Feature Gates]]. Without it, `ztl reason` prints a clear error rather than failing silently.

See also: [[Reason Commands]], [[Provenance]], [[Spindle Lisp]]
