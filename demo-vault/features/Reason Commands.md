---
title: Reason Commands
---

# Reason Commands

`zetl reason` exposes the [[Reasoning Engine]] through eight subcommands. All require `--features reason` at build time — see [[Feature Gates]].

```spl
(given reason-status-done)
(given reason-explain-done)
(given reason-why-not-done)
(given reason-require-done)
(given reason-what-if-done)
(given reason-conflicts-done)
(given reason-export-done)
(given reason-provenance-done)
```

## Commands

### `reason status`

What does the vault's combined [[Spindle Lisp]] theory conclude? Filter by polarity (`--positive`, `--negative`), strength (`--definite`, `--defeasible`), or literal name (`--literal "release*"`).

### `reason explain <literal>`

Why does a conclusion hold? Produces a proof tree tracing the derivation through rules back to facts, with full [[Provenance]] (file, line, page). Output as JSON, table, natural language, or Graphviz dot.

### `reason why-not <literal>`

Why can't something be proved? Identifies missing facts, blocked rules, or active defeaters.

### `reason require <literal>`

What facts would make a goal provable? Abductive reasoning — finds the minimal sets of missing facts. Useful for answering "what would it take to make `release-candidate` true?"

### `reason what-if <spl>`

Hypothetical reasoning. Temporarily adds facts or rules and shows what changes. Accepts inline [[Spindle Lisp]] or a file via `--file`.

### `reason conflicts`

Detects unresolved logical contradictions — pairs of competing rules with no `(prefer ...)` relation. Use `--suggest` for resolution hints and `--fail-on-conflicts` for CI.

### `reason export`

Exports the combined theory as JSON or reconstructed [[Spindle Lisp]] with [[Provenance]] comments. Use `--with-conclusions` to include reasoning results.

### `reason provenance <literal>`

Traces a conclusion back to source files, cross-referenced with the [[Link Graph]]. Shows which pages contributed the facts and rules behind a result.

## Try it on this vault

```bash
zetl -d . reason status
zetl -d . reason explain "release-candidate" --format natural
zetl -d . reason conflicts
zetl -d . reason require "release-candidate"
zetl -d . reason what-if "(given docs-updated)" --goal "release-candidate"
```

See also: [[Reasoning Engine]], [[Defeasible Reasoning]], [[Provenance]], [[Spindle Lisp]]
