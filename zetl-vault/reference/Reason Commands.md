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

What does the vault's combined [[concepts/Spindle Lisp]] theory conclude? Filter by polarity (`--positive`, `--negative`), strength (`--definite`, `--defeasible`), or literal name (`--literal "release*"`).

### `reason explain <literal>`

Why does a conclusion hold? Produces a proof tree tracing the derivation through rules back to facts, with full [[concepts/Provenance]] (file, line, page). Output as JSON, table, natural language, or Graphviz dot.

### `reason why-not <literal>`

Why can't something be proved? Identifies missing facts, blocked rules, or active defeaters. This is the inverse of `explain` — useful for understanding gaps in the vault's knowledge.

### `reason require <literal>`

What facts would make a goal provable? Abductive reasoning — finds the minimal sets of missing facts. Useful for answering "what would it take to make `release-candidate` true?"

### `reason what-if <spl>`

Hypothetical reasoning. Temporarily adds facts or rules and shows what changes. Accepts inline [[concepts/Spindle Lisp]] or a file via `--file`. Focus on a specific literal with `--goal`.

### `reason conflicts`

Detects unresolved logical contradictions — pairs of competing rules with no `(prefer ...)` relation. Use `--suggest` for resolution hints and `--fail-on-conflicts` for CI.

### `reason export`

Exports the combined theory as JSON or reconstructed [[concepts/Spindle Lisp]] with [[concepts/Provenance]] comments. Use `--with-conclusions` to include reasoning results.

### `reason provenance <literal>`

Traces a conclusion back to source files, cross-referenced with the [[Link Graph]]. Shows which pages contributed the facts and rules behind a result, including grounding freshness from [[Drift Detection]].

## Try it on this vault

```bash
zetl -d . reason status
zetl -d . reason explain "vault-ready-to-publish" --format natural
zetl -d . reason conflicts
```

See also: [[CLI Reference]], [[Reasoning Engine]], [[concepts/Defeasible Reasoning]], [[concepts/Provenance]]
