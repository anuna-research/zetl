---
title: Running Queries
tags: [reasoning, cli, queries]
---

# Running Queries

Once your vault contains [[Writing SPL|some SPL]], `ztl reason` is how you ask questions of it. This page walks the everyday command surface: status, conflicts, export, and provenance.

> **Requires `--features reason` at install.** See [[Installation]].

## `reason status` — the default view

`ztl reason status` prints every conclusion the combined theory produces, tagged with `+D`, `-D`, `+d`, or `-d` (see [[What is Defeasible Reasoning]] for what those mean).

```bash
cd ~/notes
ztl reason status
```

In a TTY you get a table. Piped or redirected, you get JSON — same data, machine-readable.

### Filters

Status output gets long fast. A few flags keep it manageable:

| Flag                      | Shows |
|---------------------------|-------|
| `--positive`              | Only `+D` and `+d` conclusions |
| `--negative`              | Only `-D` and `-d` conclusions |
| `--definite`              | Only `+D` and `-D` (the strict layer) |
| `--defeasible`            | Only `+d` and `-d` (the soft layer) |
| `--literal "release*"`    | Wildcard filter on the literal name (`*` and `?` supported) |

Combine them. "What are all the things my Acme project is currently defeasibly committed to?" becomes:

```bash
ztl reason status --positive --defeasible --literal "acme*"
```

## `reason conflicts` — the contradictions report

When two rules conclude opposite things about the same literal — and no `(prefer ...)` picks a winner — ztl calls that an **unresolved conflict**. `reason conflicts` lists them:

```bash
ztl reason conflicts
```

Useful flags:

- `--suggest` — for each conflict, propose a preference that would resolve it.
- `--fail-on-conflicts` — exit with code 1 if any conflicts exist. Drop this into CI to gate merges on a contradiction-free theory.

```bash
# In CI
ztl reason conflicts --fail-on-conflicts
```

A fresh vault almost always has one or two conflicts — someone wrote a defeater but forgot the preference. The report points at both competing rules by file and line.

## `reason export` — the whole theory

`reason export` dumps the combined theory (every fact, every rule, every preference). Two shapes:

```bash
ztl reason export                      # JSON (structured, default)
ztl reason export --as spl             # reconstructed SPL with provenance comments
ztl reason export --with-conclusions   # include current reasoning results
```

The SPL form is handy for debugging "where is this rule actually coming from?" — each rule is prefixed with a comment naming its source file and line. The JSON form is what you pipe to an agent or a downstream tool.

## `reason provenance <literal>` — the sources

`reason provenance` answers *which files and line numbers contributed to this conclusion?* It cross-references the proof sources with the link graph so you can see the pages involved alongside the raw file paths.

```bash
ztl reason provenance "acme-ready-to-start"
```

Output includes:

- Every fact whose literal appears in the proof.
- Every rule that fired, with its source page and line.
- The cross-referenced list of pages that contributed — the same pages you'd see as backlinks in a graph view.

This is the command to run when you want to say in a team meeting, "here's exactly where in the vault this conclusion came from" without hunting through files.

## Cross-referencing with the link graph

Reasoning is not separate from your wikilink graph — the same pages power both. Two graph commands take a `--with-conclusions` flag that overlays reasoning state onto their output:

```bash
ztl backlinks "Acme Website" --with-conclusions
```

This prints every page that links into `Acme Website` alongside the SPL conclusions each of those pages contributes. Answer: *which notes are actually asserting things that feed this project's readiness?*

See [[Backlinks]] for the base command.

## Output formatting rules

All `reason` subcommands follow the same two rules as the rest of ztl:

1. **In a terminal**, you get a table.
2. **Piped or redirected**, you get JSON.

Force one with the global `--json` flag or `-f table` — they work before or after the subcommand. Errors go to stderr so piped stdout stays valid JSON. See [[CLI Overview]] for the house rules.

## Time travel

Every `reason` subcommand accepts `--at`. With `--features history` installed, you can ask what the theory concluded at any past point:

```bash
ztl reason status --at "last monday"
ztl reason conflicts --at "2026-03-01"
```

Useful for post-mortems and decision audits. See [[Time Travel]].

## Related

- [[Writing SPL]]
- [[Proof Trees]]
- [[What-if and Abduction]]
- [[CLI Overview]]
- [[Time Travel]]
