# ADR-003: JSON Errors on Stdout

**Status:** Accepted

## Context

When ztl encounters an error (e.g., page not found, empty search query), how should it report it? The initial implementation wrote plain-text errors to stderr, which broke agent parsing of JSON output.

This was discovered during the agent ergonomics audit — see [[Agent Ergonomics]] and [[SPEC-003 Agent Ergonomics]].

## Decision

When `--format json` is active (the default), errors are written as structured JSON to **stdout**. Diagnostic messages (verbose logging, timing) remain on stderr.

```spl
(given structured-errors)
(given nonzero-exit-codes)
```

## Error format

```json
{
  "error": "page not found: Nonexistent",
  "suggestion": "did you mean: Nonfiction?"
}
```

## Rationale

- Agents parse stdout as JSON; a plain-text error on stdout breaks the parser
- Structured errors include actionable fields (suggestions, affected files)
- Non-zero exit codes signal failure to shell scripts and CI
- stderr remains available for human-readable diagnostics

## When `--format table` is active

Errors are rendered as human-readable messages on stderr, as is conventional for CLI tools.

See also: [[JSON by Default]], [[Agent Ergonomics]], [[CLI Reference]]
