# Spindle Lisp

Spindle Lisp (SPL) is a domain-specific language for expressing [[concepts/Defeasible Reasoning]] theories. zetl extracts SPL from fenced code blocks in Markdown and from standalone `.spl` files.

```spl
(given spl-documented)
```

## Syntax reference

### Facts

Unconditionally true propositions:

```
(given bird)
(given (not guilty))
```

### Strict rules

Cannot be defeated. If the body holds, the head must hold:

```
(always r-penguin-is-bird penguin bird)
```

### Defeasible rules

Can be defeated by stronger evidence. The label is optional:

```
(normally r-birds-fly bird flies)
(normally bird animal)
```

### Defeaters

Block a conclusion without asserting the opposite:

```
(except d-broken-wing broken-wing (not flies))
```

### Superiority

Resolves conflicts between competing rules:

```
(prefer r-stronger-rule r-weaker-rule)
```

### Conjunction

Combine multiple conditions in a rule body:

```
(normally r-ready
  (and tested documented reviewed)
  ready-to-ship)
```

### Negation

Use `(not ...)` in rule heads and bodies:

```
(normally r-penguins-dont-fly penguin (not flies))
```

### Metadata

Attach metadata to facts for explicit grounding (see [[Drift Detection]]):

```
(given redis-fast-enough)
(meta redis-fast-enough (source "^benchmark-results"))
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

The [[architecture/Scanner]] extracts these blocks with line numbers for [[concepts/Provenance]]. Multiple blocks per page are supported.

## Standalone files

Any `.spl` file in the vault is also picked up. This is useful for cross-cutting rules that don't belong to a single page — see the `theories/` directory in this vault for examples.

## Conclusion types

| Tag | Meaning |
|-----|---------|
| `+D` | Definitely provable (strict rules, no defeaters possible) |
| `-D` | Definitely not provable |
| `+d` | Defeasibly provable (inferred, no active defeaters) |
| `-d` | Defeasibly not provable (blocked or no derivation path) |

See also: [[concepts/Defeasible Reasoning]], [[Reasoning Engine]], [[concepts/Provenance]], [[Reason Commands]]
