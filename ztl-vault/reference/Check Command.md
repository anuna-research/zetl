# Check Command

`ztl check` validates the vault and reports issues across both the [[Link Graph]] and [[concepts/Spindle Lisp]] content.

```spl
(given dead-link-detection)
(given orphan-detection)
(given spl-diagnostics)
(given drift-detection)
```

## Usage

```bash
ztl -d ./my-vault check
ztl check --dead-links --fail-on error    # CI mode
ztl check --spl                           # SPL diagnostics only
ztl check --drift                         # detect SPL grounding drift
```

## Issue types

### Dead links

A [[concepts/Wikilinks|wikilink]] that points to a page that doesn't exist. For example, `[[Plugin System]]` would be a dead link if there's no `Plugin System.md` file.

### Orphan pages

Pages with no incoming links — nothing in the vault references them. These may be forgotten drafts or entry points that need a wikilink from somewhere.

### Syntax errors

Malformed wikilinks like unclosed brackets.

### SPL diagnostics

Parse errors, duplicate rule labels, undefined references, and unreachable literals in [[concepts/Spindle Lisp]] blocks. Enabled with `--spl`.

### Drift detection

SPL blocks whose surrounding prose has changed since the theory was last built, potentially invalidating the grounding of facts. Enabled with `--drift`. See [[Drift Detection]] and [[Merkle Tree]].

## Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--dead-links` | off | Show only dead links |
| `--orphans` | off | Show only orphan pages |
| `--syntax` | off | Show only syntax errors |
| `--spl` | off | Show only SPL diagnostics |
| `--drift` | off | Show only drift diagnostics |
| `--fail-on <level>` | `error` | Exit non-zero if issues at level (`error` or `warning`) |

Without any filter flag, all issue types are reported.

## CI integration

Use `--fail-on error` to exit non-zero when issues are found:

```bash
ztl -d ./my-vault check --dead-links --fail-on error
```

See also: [[CLI Reference]], [[Drift Detection]], [[Blocks Command]], [[Reason Commands]]
