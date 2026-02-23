---
title: JSON by Default
---

# JSON by Default

All zetl commands emit JSON by default. Human-readable output is available via `--format table` (or `--format natural` / `--format dot` for `reason explain`).

```spl
(given json-default-output)
(given structured-errors)
(given nonzero-exit-codes)
```

## Why agent-first

zetl is designed to work both as a human CLI tool and as a building block for AI agents and scripts. JSON output means:

- Agents can parse results without scraping tables
- Structured errors include error codes, affected files, and suggested fixes
- Non-zero exit codes signal failure to shell scripts and CI pipelines
- Output is composable with `jq`, `fx`, and other JSON tools

## Human output

For interactive use, `--format table` renders results as aligned tables via `comfy-table`. The [[TUI]] provides a richer interactive experience — see [[TUI]].

## Both audiences

This dual-output design means a single tool serves both audiences. An agent might run `zetl reason status` and parse the JSON, while a human runs `zetl reason status --format table` and reads the output directly.

See also: [[Rust for CLI]], [[TUI]], [[Reason Commands]]
