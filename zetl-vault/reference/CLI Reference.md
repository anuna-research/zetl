# CLI Reference

zetl is a single binary with subcommands. All commands output JSON by default — see [[JSON by Default]].

## Global flags

| Flag | Short | Description |
|------|-------|-------------|
| `--dir <path>` | `-d` | Vault root directory (default: `.`) |
| `--format <fmt>` | `-f` | Output format: `json` (default) or `table` |
| `--no-cache` | | Force full rescan, ignore cached index |
| `--no-color` | | Disable colored output |
| `--quiet` | `-q` | Suppress non-essential output |
| `--verbose` | `-v` | Increase verbosity (repeat for more: `-vv`) |
| `--at <time-expr>` | | Query vault at a historical point in time (requires `--features history`). Accepts ISO 8601 dates, relative expressions ("3 days ago"), or VCS refs ("HEAD~1") |

## Commands

### Wikilink commands

| Command | Description | Reference |
|---------|-------------|-----------|
| `index` | Build or refresh the link index | [[Index Command]] |
| `links <page>` | Query forward links from a page | [[Links Command]] |
| `backlinks <page>` | Query backlinks to a page | [[Links Command]] |
| `path <from> <to>` | Shortest link path between pages | [[Path Command]] |
| `search <query>` | Full-text content search | [[Search Command]] |
| `similar <query>` | Fuzzy page name matching (SimHash) | [[Similar Command]] |
| `check` | Validate vault: dead links, orphans, SPL errors | [[Check Command]] |
| `blocks [page]` | List or resolve Merkle content blocks | [[Blocks Command]] |
| `diff` | Git-backed graph diff | [[Diff Command]] |
| `watch` | NDJSON event stream on file changes | [[Watch Command]] |
| `list` | List all pages | [[List and Export Commands]] |
| `export` | Export complete link graph as JSON | [[List and Export Commands]] |
| `stats` | Vault summary statistics | [[Stats Command]] |
| `tui` | Interactive terminal UI | [[TUI]] |
| `view [page]` | Xanadu-style two-pane reader | [[View Command]] |
| `serve` | Local web server | [[Serve Command]] |
| `build` | Static site export | [[Build Command]] |

### Collaboration commands

| Command | Description | Reference |
|---------|-------------|-----------|
| `serve --collab` | Start multi-user collab server | [[Serve Command]] |
| `serve --collab --init-owner` | Bootstrap vault owner (first-time) | [[Serve Command]] |
| `invite --as <user> --role <role>` | Generate an invitation link | [[Serve Command]] |
| `agent-token --mnemonic "..."` | Derive a Bearer token for headless API access | [[Serve Command]] |

### History commands

Require `--features history` at build time. See [[Feature Gates]].

| Command | Description | Reference |
|---------|-------------|-----------|
| `history log` | Graph-level delta timeline (reverse chronological) | [[History Command]] |
| `history page <name>` | Evolution of a specific page across snapshots | [[History Command]] |
| `history timeline` | List recent snapshots with timestamps and graph stats | [[History Command]] |

### Reasoning commands

Require `--features reason` at build time. See [[Feature Gates]].

| Command | Description | Reference |
|---------|-------------|-----------|
| `reason status` | Current conclusions from vault-wide SPL | [[Reason Commands]] |
| `reason explain <literal>` | Proof tree with provenance | [[Reason Commands]] |
| `reason why-not <literal>` | Why a literal isn't provable | [[Reason Commands]] |
| `reason require <literal>` | Abductive: what facts are needed | [[Reason Commands]] |
| `reason what-if <spl>` | Hypothetical reasoning | [[Reason Commands]] |
| `reason conflicts` | Unresolved logical contradictions | [[Reason Commands]] |
| `reason export` | Export combined theory | [[Reason Commands]] |
| `reason provenance <literal>` | Trace conclusion to source files | [[Reason Commands]] |

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Error (dead links found with `--fail-on error`, conflicts with `--fail-on-conflicts`, etc.) |

Structured JSON errors are written to stdout, not stderr. See [[ADR-003 JSON Errors]].

See also: [[Install]], [[Quick Start]], [[JSON by Default]]
