---
title: Vault Diagnostics
---

# Vault Diagnostics

`zetl check` validates the vault and reports issues. It examines both the [[Link Graph]] and [[Spindle Lisp]] content.

```spl
(given dead-link-detection)
(given orphan-detection)
(given spl-diagnostics)
```

## Issue types

### Dead links

A [[Wikilinks|wikilink]] that points to a page that doesn't exist. For example, `[[Plugin System]]` in this vault is a dead link — there is no `Plugin System.md` file.

```bash
zetl -d . check --dead-links
```

### Orphan pages

Pages with no incoming links — nothing in the vault references them. These may be forgotten drafts or entry points that need a [[Wikilinks|wikilink]] from somewhere.

```bash
zetl -d . check --orphans
```

### Syntax errors

Malformed [[Wikilinks]] like unclosed brackets.

### SPL diagnostics

Parse errors, duplicate rule labels, undefined references, and unreachable literals in [[Spindle Lisp]] blocks. Enabled with `--spl`:

```bash
zetl -d . check --spl
```

## CI integration

Use `--fail-on error` to exit non-zero when issues are found:

```bash
zetl -d ./my-vault check --dead-links --fail-on error
```

See also: [[Graph Queries]], [[Reason Commands]], [[Scanner]]
