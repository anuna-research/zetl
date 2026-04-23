---
title: Spindle Lisp
---

# Spindle Lisp

Spindle Lisp (SPL) is a domain-specific language for expressing [[Defeasible Reasoning]] theories. ztl extracts SPL from fenced code blocks in Markdown and from standalone `.spl` files.

## Syntax reference

### Facts

```
(given bird)
(given (not guilty))
```

### Strict rules

Cannot be defeated. If the body holds, the head must hold.

```
(always r-penguin-is-bird penguin bird)
```

### Defeasible rules

Can be defeated by stronger evidence. The label is optional.

```
(normally r-birds-fly bird flies)
(normally bird animal)
```

### Defeaters

Block a conclusion without asserting the opposite.

```
(except d-broken-wing broken-wing (not flies))
```

### Superiority

Resolves conflicts between competing rules.

```
(prefer r-stronger-rule r-weaker-rule)
```

### Conjunction

Combine multiple conditions in a rule body.

```
(normally r-ready
  (and tested documented reviewed)
  ready-to-ship)
```

### Negation

Use `(not ...)` inside rule heads and bodies:

```
(normally r-penguins-dont-fly penguin (not flies))
(except d-wing broken-wing (not flies))
```

### Comments

```
; This is a comment
```

## Embedding in Markdown

Wrap SPL in a fenced code block tagged `spl`:

````markdown
```spl
(given bird)
(normally r-birds-fly bird flies)
```
````

The [[Scanner]] extracts these blocks with line numbers for [[Provenance]]. Multiple blocks per page are supported — see [[Reason Commands]] for how to query the combined theory.

## Standalone files

Any `.spl` file in the vault is also picked up. This is useful for shared theories that don't belong to a single page — see `theories/` in this vault for examples.

See also: [[Defeasible Reasoning]], [[Reasoning Engine]], [[Provenance]]
