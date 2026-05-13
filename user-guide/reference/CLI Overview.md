---
title: CLI Overview
tags: [reference, cli, commands]
---

# CLI Overview

Every `zetl` subcommand at a glance, with the flags writers actually use. This page is the reference — the linked how-to pages give the narrative.

Run `zetl <cmd> --help` for the authoritative flag list for any command. This guide tracks zetl **v0.5.0**.

## Global flags

Every subcommand honours these, wherever they make sense.

| Flag | Env var | Purpose |
| --- | --- | --- |
| `-d, --dir <DIR>` | `ZETL_DIR` | Vault root. Default: `.` (current directory). |
| `-f, --format <FORMAT>` | `ZETL_FORMAT` | `json` / `table` / `auto`. Auto picks table for a TTY, JSON for a pipe. |
| `--json` | — | Shorthand for `-f json`. |
| `--no-cache` | `ZETL_NO_CACHE` | Ignore the cached index; force a full rescan. |
| `--no-color` | `NO_COLOR` | Disable ANSI colour. |
| `-q, --quiet` | — | Suppress non-essential output. |
| `-v, --verbose` | — | Increase verbosity. Repeatable (`-vv`). |
| `--no-input` | — | Disable interactive prompts; fail if input is needed. |
| `--at <TIME-EXPR>` | — | Query the vault at a historical point. Requires `--features history`. See [[Time Travel]]. |
| `-V, --version` | — | Print zetl version. |

## Subcommand summary

| Command | Description | Feature flag |
| --- | --- | --- |
| [`index`](#zetl-index) | Build or refresh the link index | — |
| [`list`](#zetl-list) | List every page in the vault | — |
| [`stats`](#zetl-stats) | Summary counts and most-linked pages | — |
| [`links`](#zetl-links) | Forward links from a page | — |
| [`backlinks`](#zetl-backlinks) | Backlinks to a page | — |
| [`check`](#zetl-check) | Dead links, orphans, syntax, SPL diagnostics | — |
| [`search`](#zetl-search) | Full-text search (BM25, optional semantic) | `semantic` for `--semantic`/`--hybrid` |
| [`similar`](#zetl-similar) | SimHash title matches | — |
| [`path`](#zetl-path) | Shortest link path between two pages | — |
| [`export`](#zetl-export) | Dump the full link graph | — |
| [`blocks`](#zetl-blocks) | List or resolve Merkle blocks | — |
| [`view`](#zetl-view) | Two-pane terminal reader | — |
| [`serve`](#zetl-serve) | Local web server | — |
| [`build`](#zetl-build) | Static-site HTML export | — |
| [`watch`](#zetl-watch) | Stream vault changes as NDJSON | — |
| [`diff`](#zetl-diff) | Graph diff against a git ref or snapshot | — |
| [`theme`](#zetl-theme) | List / install / remove / export themes | — |
| [`hook`](#zetl-hook) | Author, test, and inspect hooks | — |
| [`ecosystem`](#zetl-ecosystem) | Probe Pandoc / mdBook / remark runtimes | `ecosystems-v1` |
| [`ast`](#zetl-ast) | Inspect or diff zetl-ext AST | — |
| [`agent`](#zetl-agent) | Run an agent-lifecycle hook | — |
| [`invite`](#zetl-invite) | Multi-user invitation token | collab (built-in) |
| [`agent-token`](#zetl-agent-token) | Derive a headless API token | collab |
| [`derive-ssh-key`](#zetl-derive-ssh-key) | Derive an SSH ed25519 key from a mnemonic | collab |
| [`cap`](#zetl-cap) | Capability-URL static sharing operations | — |
| [`reason`](#zetl-reason) | SPL queries over the vault | `reason` |
| [`mcp`](#zetl-mcp) | Start an MCP server | `mcp` |
| [`delegate`](#zetl-delegate) | Issue a delegate JWT | `mcp` |
| [`completions`](#zetl-completions) | Emit a shell completion script | — |
| [`man`](#zetl-man) | Emit a roff(7) man page | — |
| [`skill`](#zetl-skill) | Install the bundled Claude Code `SKILL.md` | — |

---

## zetl index

Scan the vault and write the cached link index to `.zetl/index.json`. Incremental by default; `--no-cache` forces a full rescan.

Key flags: `--exclude <PATTERN>` (repeatable gitignore syntax), `--include-hidden` (walk `.claude/`, `.obsidian/`, etc.).

```bash
zetl index
zetl -d ~/notes index --exclude 'drafts/**'
```

## zetl list

Enumerate every page zetl can see. Useful in pipes.

```bash
zetl list --json | jq -r '.pages[].title'
```

## zetl stats

Counts: files, links, orphans, dead links, plus the most-linked pages. `--top <N>` sets how many leaders to print (default 10).

## zetl links

Forward links from a page. See [[Following Links]].

Key flags: `--depth <N>` (traverse N hops), `--context <N>` (include N characters of surrounding text), `--fuzzy` (case-insensitive partial page-name match), `--with-conclusions` (SPL conclusions each linked page contributes; requires `reason`).

```bash
zetl links "Zettelkasten Method"
zetl links "Zettelkasten Method" --depth 2
```

## zetl backlinks

Same flags as `zetl links`, but inbound. See [[Backlinks]].

```bash
zetl backlinks "Zettelkasten Method" --context 60
```

## zetl check

Validate vault health: dead links, orphans, syntax errors, SPL diagnostics, and SPL drift. See [[Finding Orphans and Dead Links]].

Key flags: `--dead-links`, `--orphans`, `--syntax`, `--spl`, `--drift` (show only that category), `--fail-on error|warning` (CI gate), `--theme <NAME>` (for hook-discovery).

```bash
zetl check
zetl check --dead-links --fail-on error
```

## zetl search

Full-text search across vault content. See [[Searching]].

Key flags: `--limit <N>` (default 50), `--context <N>` (default 40), `--case-sensitive`, `--path <GLOB>` (restrict to matching files), `--near <PAGE>` with `--depth <N>` (restrict to pages within N link-hops of PAGE), `--semantic` and `--hybrid` (require `--features semantic`).

```bash
zetl search "wikilink"
zetl search "API" --near "Backend Overview" --depth 2
zetl search "memory" --hybrid --limit 20
```

## zetl similar

SimHash locality-sensitive title match. See [[Similar Pages]].

Key flags: `--threshold <N>` (max Hamming distance, default 12), `--limit <N>` (default 10).

## zetl path

Shortest link path between two pages. See [[Following Links]].

Key flag: `--max-depth <N>` (default 10).

```bash
zetl path "Quick Start" "Capability URLs"
```

## zetl export

Dump the full link graph — pages + edges — to JSON. Pipe into Gephi, Obsidian Canvas, or your own tooling.

## zetl blocks

List the [[Blocks|Merkle blocks]] of a page, or resolve a block by its BLAKE3 hash.

Key flags: `--type heading|paragraph|spl|code|table|list|blockquote|frontmatter|all`, `--resolve <HASH>` (full or prefix; mutually exclusive with PAGE).

```bash
zetl blocks "Zettelkasten Method" --type heading
zetl blocks --resolve 0f3ac1
```

## zetl view

Xanadu-style two-pane terminal reader. See [[Terminal Viewer]].

Key flags: `--main-width <pct>` (30–80, default 58), `--context-lines <N>` (1–20, default 5). Run without a page name to open a picker.

## zetl serve

Local web server. See [[Web Server]].

Key flags: `--port <PORT>` (default 3000), `--theme <NAME>`, `--public <DIR>` (files that override generated pages), `--collab` (multi-user), `--init-owner` / `--owner-name <NAME>` (first-time collab bootstrap), `--hostname <HOST>` (WebAuthn relying-party; env: `ZETL_HOSTNAME`), `--server-key-seed <MNEMONIC>` (deterministic keys; env: `ZETL_SERVER_KEY_SEED`), `--git-poll-interval 30s`, `--safe-mode` (only theme-declared hooks run).

```bash
zetl serve
zetl serve --collab --init-owner --owner-name Jo
zetl serve --port 8080 --theme fountain
```

## zetl build

Generate a static HTML site. See [[Static Site Export]].

Key flags: `-o, --out-dir <DIR>` (default `dist`), `--theme <NAME>`, `--public <DIR>`, `--site-url <URL>` (absolute og:image URLs), `--safe-mode`, `--strict-parsers` (promote mixed-parser warnings to errors).

```bash
zetl build
zetl build --theme docs --site-url https://notes.example
```

## zetl watch

Watch the vault for changes and emit NDJSON graph events. See [[Watching for Changes]].

Key flags: `--debounce <MS>` (default 150, min 10, max 5000), `--exec <CMD>` (shell command run once per event with the event JSON on stdin), `--exclude`, `--include-hidden`.

```bash
zetl watch --exec 'jq .type'
```

## zetl diff

Graph-level diff against a git ref or jj change-ID.

Key flags: `--from <REF>` or `--since <DATE>`, `--filter pages|links|orphans|dead-links`.

```bash
zetl diff --from HEAD~5
zetl diff --since "last monday" --filter dead-links
```

## zetl theme

Manage themes. See [[Customising the Look]].

Subcommands:

| Subcommand | Purpose |
| --- | --- |
| `zetl theme list` | List bundled + installed themes |
| `zetl theme install <SOURCE>` | Install from git (`user/repo`, URL, `git@…#ref`); flags: `--path`, `--name`, `--force` |
| `zetl theme remove <NAME>` | Remove an installed theme |
| `zetl theme export <NAME>` | Copy a bundled theme into `.zetl/themes/` for customisation; `--force` |

## zetl hook

Author, test, and inspect hooks. See [[Lifecycle Hooks]] and [[Render Pipeline Hooks]].

| Subcommand | Purpose |
| --- | --- |
| `zetl hook list` | Active hooks for this vault + theme |
| `zetl hook run <NAME> [-- <EXTRA>...]` | Run a lifecycle hook with real vault context |
| `zetl hook new <STAGE> <NAME>` | Scaffold a render-pipeline hook; flags: `--lang py\|js\|sh`, `--ecosystem pandoc\|mdbook\|remark`, `--force` |
| `zetl hook test <NAME>` | Run a hook against its fixture and diff the golden; `--update` regenerates |
| `zetl hook fixture --from <PAGE> --hook <NAME>` | Capture a vault page into the hook's fixture dir |
| `zetl hook watch <NAME>` | Live-reload the hook's persistent-mode subprocess on source change |
| `zetl hook coverage` | Per-hook coverage from the most-recent build; `--stage`, `--json` |
| `zetl hook dry-run <STAGE>/<NAME>` | Print pages the hook's selector matches; hook is not invoked; `--limit`, `--theme` |
| `zetl hook capabilities` | Probe every hook's supported stages + AST types; `--stage`, `--json` |

## zetl ecosystem

Probe plugin ecosystems. Requires `ecosystems-v1` feature flag.

- `zetl ecosystem check` — per-ecosystem detection, version, configured-hook count, reachable plugins. Exits 0 when all *configured* ecosystems are available. See [[Plugin Ecosystems]].

## zetl ast

Inspect the zetl-ext AST for a page or diff two AST JSON documents.

- `zetl ast sample <FILE>` — print canonical AST JSON. `--stage pre-parse|transform|post-render` chooses which stage's input to emit (default `transform`).
- `zetl ast diff <BEFORE> <AFTER>` — tree-aware structural diff; non-zero exit on non-empty diff.

## zetl agent

Agent lifecycle integration for LLM / automation hooks.

- `zetl agent run <NAME> [-- <EXTRA>...]` — run an on-agent hook with task context. Flags: `--pages <TARGET>...`, `--budget <TOKENS>` (0 = unlimited), `--theme`.

## zetl invite

Generate an invitation token for a collaborator. See [[Invitations]].

Key flags: `--as <USER>` (inviter), `--role reader|editor|admin`, `--pages <GLOB>` (scoped grant), `--expires 72h|24h|7d` (default 72h), `--host`, `--port` (URL generation).

```bash
zetl invite --as alice --role editor
zetl invite --as alice --role reader --pages 'projects/*' --expires 7d
```

## zetl agent-token

Derive a headless API token from a BIP39 mnemonic.

```bash
zetl agent-token --mnemonic "word1 word2 ... word12"
```

## zetl derive-ssh-key

Derive an SSH ed25519 key from a BIP39 mnemonic. Useful for ephemeral containers where one seed covers the server key and the git SSH key.

```bash
zetl derive-ssh-key --mnemonic "word1 ... word12" --out /root/.ssh/id_ed25519
```

## zetl cap

Capability-URL static-site operations. See [[Capability URLs]].

| Subcommand | Purpose |
| --- | --- |
| `zetl cap genkey` | Emit the content-encryption secret + signing keypair (once) |
| `zetl cap invite <NAME> --cohort <ID>` | Issue an invitation grant; flags: `--expires`, `--pages <GLOB>`, `--recipient <PUBKEY>`, `--via enrol-page`, `--split-key`, `--site-url`, `--slug` |
| `zetl cap list` | List issued grants; `--cohort`, `--output` |
| `zetl cap revoke <GRANT_ID>` | Revoke a grant |
| `zetl cap rotate --cohort <ID>` | Rotate a cohort's content-key salt (URLs stay stable) |
| `zetl cap finalise <GRANT_ID>` | Mark a grant operator-confirmed; `--rotate-grant` |
| `zetl cap check` | Stale-grant + public-safety audit; `--public-safety` |
| `zetl cap sweep` | Mark past-expiry grants as revoked |
| `zetl cap pair` | SPAKE2 pubkey handoff; `--grantor` / `--grantee`, `--peer`, `--phrase`, `--pubkey` |
| `zetl cap audit-diff [OLD] [NEW]` | Scan a diff for malicious content; `--corpus`, `--corpus-root` |
| `zetl cap rotate-signing-key` | Rotate the Ed25519 vault-signing key (rebuilds every page) |
| `zetl cap emergency-shutdown` | Print the takedown checklist (no files changed) |

## zetl reason

SPL queries over the vault. Requires `--features reason`. See [[Running Queries]].

Subcommands (consult `zetl reason --help` inside a feature-reason build): `status`, `explain`, `conflicts`, `why-not`, `what-if`.

## zetl mcp

Start a Model Context Protocol server. Requires `--features mcp`. See [[MCP Server]].

## zetl delegate

Issue a delegate JWT for an MCP client. Requires `--features mcp`.

## zetl completions

Print a shell-completion script.

```bash
zetl completions zsh > ~/.zsh/completions/_zetl
```

Supported shells: `bash`, `zsh`, `fish`, `elvish`, `powershell`.

## zetl man

Print a roff(7) man page. Preview with `zetl man | man -l -`.

## zetl skill

Install the bundled [Claude Code](https://docs.claude.com/claude-code) skill so the agent knows when to reach for `zetl`. The `SKILL.md` is embedded in the binary; the on-disk copy carries the same version as the binary that wrote it, so an upgrade + re-run refreshes the skill.

```bash
zetl skill init             # write ./.claude/skills/zetl/SKILL.md  (project scope, commit it)
zetl skill init --user      # write ~/.claude/skills/zetl/SKILL.md  (global)
zetl skill init --global    # alias for --user
```

The file follows the [agentskills.io](https://agentskills.io) frontmatter schema (`name`, `description`, `license`, `compatibility`, `metadata`). Pairs naturally with [[MCP Server|zetl mcp]] — the skill teaches the agent which `zetl` command to reach for; the MCP server gives it live access to vault contents.

## Related

- [[Configuration]]
- [[Installation]]
- [[Quick Start]]
- [[Glossary]]
- [[Frontmatter Fields]]
