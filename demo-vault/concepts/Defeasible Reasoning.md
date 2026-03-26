---
title: Defeasible Reasoning
---

# Defeasible Reasoning

Defeasible reasoning is a form of logic where conclusions can be drawn tentatively and later retracted when stronger evidence appears. This is how most real-world reasoning works — we act on the best information available, knowing new facts might change our minds.

## Why it fits knowledge managements

Decision documents, architecture records, and project plans all contain reasoning that is provisional. A conclusion like "use Redis" might be well-supported now but defeated by a future license audit. [[Spindle Lisp]] lets you express this directly in your notes, and zetl's [[Reasoning Engine]] computes what follows.

## Rule types

| Type | Syntax | Meaning |
|------|--------|---------|
| Fact | `(given X)` | X is unconditionally true |
| Strict | `(always name body head)` | If body, then head — no exceptions |
| Defeasible | `(normally name body head)` | If body, normally head — can be defeated |
| Defeater | `(except name body head)` | If body, block head — but don't assert the opposite |

## Superiority

When two rules conflict a superiority relation resolves the tie:

```
(prefer stronger-rule weaker-rule)
```

Without a declared preference, the conflict remains unresolved — `zetl reason conflicts` will flag it.

## Conclusion types

| Tag | Meaning |
|-----|---------|
| `+D` | Definitely provable — strict derivation, no defeating possible |
| `-D` | Definitely not provable |
| `+d` | Defeasibly provable — inferred, no active defeaters |
| `-d` | Defeasibly not provable — blocked or no derivation path |

## In zetl

The [[Reasoning Engine]] implements defeasible reasoning logic via `spindle-core`. See [[Reason Commands]] for the CLI interface and [[Provenance]] for how conclusions trace back to source files.
