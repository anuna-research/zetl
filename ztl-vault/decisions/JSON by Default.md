# JSON by Default

All ztl commands emit JSON by default. Human-readable output is available via `--format table` (or `--format natural` / `--format dot` for `reason explain`).

```spl
(given json-default-output)
```

## Why agent-first

ztl is designed to work both as a human CLI tool and as a building block for AI agents and scripts. JSON output means:

- Agents can parse results without scraping tables
- Structured errors include error codes, affected files, and suggested fixes (see [[ADR-003 JSON Errors]])
- Non-zero exit codes signal failure to shell scripts and CI pipelines
- Output is composable with `jq`, `fx`, and other JSON tools

## Human output

For interactive use, `--format table` renders results as aligned tables via `comfy-table`. The [[TUI]] and [[View Command]] provide richer interactive experiences.

## Both audiences

This dual-output design means a single tool serves both audiences. An agent might run `ztl reason status` and parse the JSON, while a human runs `ztl reason status --format table` and reads the output directly.

```spl
; Serving both audiences is a design principle
(normally r-dual-audience
  (and json-default-output tui-complete seven-views)
  serves-both-audiences)
```

See also: [[CLI Reference]], [[ADR-003 JSON Errors]], [[Agent Ergonomics]]
