---
title: CLI Overview
tags: [reference, cli, commands]
---

# CLI Overview

Every `ztl` subcommand at a glance, with the flags writers actually use. This page is the reference — the linked how-to pages give the narrative.

Run `ztl <cmd> --help` for the authoritative flag list for any command. This guide tracks ztl **v0.5.0**.

## Global flags

Every subcommand honours these, wherever they make sense.

| Flag | Env var | Purpose |
| --- | --- | --- |
| `-d, --dir <DIR>` | `ztl_DIR` | Vault root. Default: `.` (current directory). |
| `-f, --format <FORMAT>` | `ztl_FORMAT` | `json` / `table` / `auto`. Auto picks table for a TTY, JSON for a pipe. |
| `--json` | — | Shorthand for `-f json`. |
| `--no-cache` | `ztl_NO_CACHE` | Ignore the cached index; force a full rescan. |
| `--no-color` | `NO_COLOR` | Disable ANSI colour. |
| `-q, --quiet` | — | Suppress non-essential output. |
| `-v, --verbose` | — | Increase verbosity. Repeatable (`-vv`). |
| `--no-input` | — | Disable interactive prompts; fail if input is needed. |
| `--at <TIME-EXPR>` | — | Query the vault at a historical point. Requires `--features history`. See [[Time Travel]]. |
| `-V, --version` | — | Print ztl version. |

## Subcommand summary

| Command | Description | Feature flag |
| --- | --- | --- |
| [`index`](#ztl-index) | Build or refresh the link index | — |
| [`list`](#ztl-list) | List every page in the vault | — |
| [`stats`](#ztl-stats) | Summary counts and most-linked pages | — |
| [`links`](#ztl-links) | Forward links from a page | — |
| [`backlinks`](#ztl-backlinks) | Backlinks to a page | — |
| [`check`](#ztl-check) | Dead links, orphans, syntax, SPL diagnostics | — |
| [`search`](#ztl-search) | Full-text search (BM25, optional semantic) | `semantic` for `--semantic`/`--hybrid` |
| [`similar`](#ztl-similar) | SimHash title matches | — |
| [`path`](#ztl-path) | Shortest link path between two pages | — |
| [`export`](#ztl-export) | Dump the full link graph | — |
| [`blocks`](#ztl-blocks) | List or resolve Merkle blocks | — |
| [`view`](#ztl-view) | Two-pane terminal reader | — |
| [`serve`](#ztl-serve) | Local web server | — |
| [`build`](#ztl-build) | Static-site HTML export | — |
| [`watch`](#ztl-watch) | Stream vault changes as NDJSON | — |
| [`diff`](#ztl-diff) | Graph diff against a git ref or snapshot | — |
| [`theme`](#ztl-theme) | List / install / remove / export themes | — |
| [`hook`](#ztl-hook) | Author, test, and inspect hooks | — |
| [`ecosystem`](#ztl-ecosystem) | Probe Pandoc / mdBook / remark runtimes | `ecosystems-v1` |
| [`ast`](#ztl-ast) | Inspect or diff ztl-ext AST | — |
| [`agent`](#ztl-agent) | Run an agent-lifecycle hook | — |
| [`invite`](#ztl-invite) | Multi-user invitation token | collab (built-in) |
| [`agent-token`](#ztl-agent-token) | Derive a headless API token | collab |
| [`derive-ssh-key`](#ztl-derive-ssh-key) | Derive an SSH ed25519 key from a mnemonic | collab |
| [`cap`](#ztl-cap) | Capability-URL static sharing operations | — |
| [`reason`](#ztl-reason) | SPL queries over the vault | `reason` |
| [`mcp`](#ztl-mcp) | Start an MCP server | `mcp` |
| [`delegate`](#ztl-delegate) | Issue a delegate JWT | `mcp` |
| [`completions`](#ztl-completions) | Emit a shell completion script | — |
| [`man`](#ztl-man) | Emit a roff(7) man page | — |

---

## ztl index

Scan the vault and write the cached link index to `.ztl/index.json`. Incremental by default; `--no-cache` forces a full rescan.

Key flags: `--exclude <PATTERN>` (repeatable gitignore syntax), `--include-hidden` (walk `.claude/`, `.obsidian/`, etc.).

```bash
ztl index
ztl -d ~/notes index --exclude 'drafts/**'
```

## ztl list

Enumerate every page ztl can see. Useful in pipes.

```bash
ztl list --json | jq -r '.pages[].title'
```

## ztl stats

Counts: files, links, orphans, dead links, plus the most-linked pages. `--top <N>` sets how many leaders to print (default 10).

## ztl links

Forward links from a page. See [[Following Links]].

Key flags: `--depth <N>` (traverse N hops), `--context <N>` (include N characters of surrounding text), `--fuzzy` (case-insensitive partial page-name match), `--with-conclusions` (SPL conclusions each linked page contributes; requires `reason`).

```bash
ztl links "Zettelkasten Method"
ztl links "Zettelkasten Method" --depth 2
```

## ztl backlinks

Same flags as `ztl links`, but inbound. See [[Backlinks]].

```bash
ztl backlinks "Zettelkasten Method" --context 60
```

## ztl check

Validate vault health: dead links, orphans, syntax errors, SPL diagnostics, and SPL drift. See [[Finding Orphans and Dead Links]].

Key flags: `--dead-links`, `--orphans`, `--syntax`, `--spl`, `--drift` (show only that category), `--fail-on error|warning` (CI gate), `--theme <NAME>` (for hook-discovery).

```bash
ztl check
ztl check --dead-links --fail-on error
```

## ztl search

Full-text search across vault content. See [[Searching]].

Key flags: `--limit <N>` (default 50), `--context <N>` (default 40), `--case-sensitive`, `--path <GLOB>` (restrict to matching files), `--near <PAGE>` with `--depth <N>` (restrict to pages within N link-hops of PAGE), `--semantic` and `--hybrid` (require `--features semantic`).

```bash
ztl search "wikilink"
ztl search "API" --near "Backend Overview" --depth 2
ztl search "memory" --hybrid --limit 20
```

## ztl similar

SimHash locality-sensitive title match. See [[Similar Pages]].

Key flags: `--threshold <N>` (max Hamming distance, default 12), `--limit <N>` (default 10).

## ztl path

Shortest link path between two pages. See [[Following Links]].

Key flag: `--max-depth <N>` (default 10).

```bash
ztl path "Quick Start" "Capability URLs"
```

## ztl export

Dump the full link graph — pages + edges — to JSON. Pipe into Gephi, Obsidian Canvas, or your own tooling.

## ztl blocks

List the [[Blocks|Merkle blocks]] of a page, or resolve a block by its BLAKE3 hash.

Key flags: `--type heading|paragraph|spl|code|table|list|blockquote|frontmatter|all`, `--resolve <HASH>` (full or prefix; mutually exclusive with PAGE).

```bash
ztl blocks "Zettelkasten Method" --type heading
ztl blocks --resolve 0f3ac1
```

## ztl view

Xanadu-style two-pane terminal reader. See [[Terminal Viewer]].

Key flags: `--main-width <pct>` (30–80, default 58), `--context-lines <N>` (1–20, default 5). Run without a page name to open a picker.

## ztl serve

Local web server. See [[Web Server]].

Key flags: `--port <PORT>` (default 3000), `--theme <NAME>`, `--public <DIR>` (files that override generated pages), `--collab` (multi-user), `--init-owner` / `--owner-name <NAME>` (first-time collab bootstrap), `--hostname <HOST>` (WebAuthn relying-party; env: `ztl_HOSTNAME`), `--server-key-seed <MNEMONIC>` (deterministic keys; env: `ztl_SERVER_KEY_SEED`), `--git-poll-interval 30s`, `--safe-mode` (only theme-declared hooks run).

```bash
ztl serve
ztl serve --collab --init-owner --owner-name Jo
ztl serve --port 8080 --theme fountain
```

## ztl build

Generate a static HTML site. See [[Static Site Export]].

Key flags: `-o, --out-dir <DIR>` (default `dist`), `--theme <NAME>`, `--public <DIR>`, `--site-url <URL>` (absolute og:image URLs), `--safe-mode`, `--strict-parsers` (promote mixed-parser warnings to errors).

```bash
ztl build
ztl build --theme docs --site-url https://notes.example
```

## ztl watch

Watch the vault for changes and emit NDJSON graph events. See [[Watching for Changes]].

Key flags: `--debounce <MS>` (default 150, min 10, max 5000), `--exec <CMD>` (shell command run once per event with the event JSON on stdin), `--exclude`, `--include-hidden`.

```bash
ztl watch --exec 'jq .type'
```

## ztl diff

Graph-level diff against a git ref or jj change-ID.

Key flags: `--from <REF>` or `--since <DATE>`, `--filter pages|links|orphans|dead-links`.

```bash
ztl diff --from HEAD~5
ztl diff --since "last monday" --filter dead-links
```

## ztl theme

Manage themes. See [[Customising the Look]].

Subcommands:

| Subcommand | Purpose |
| --- | --- |
| `ztl theme list` | List bundled + installed themes |
| `ztl theme install <SOURCE>` | Install from git (`user/repo`, URL, `git@…#ref`); flags: `--path`, `--name`, `--force` |
| `ztl theme remove <NAME>` | Remove an installed theme |
| `ztl theme export <NAME>` | Copy a bundled theme into `.ztl/themes/` for customisation; `--force` |

## ztl hook

Author, test, and inspect hooks. See [[Lifecycle Hooks]] and [[Render Pipeline Hooks]].

| Subcommand | Purpose |
| --- | --- |
| `ztl hook list` | Active hooks for this vault + theme |
| `ztl hook run <NAME> [-- <EXTRA>...]` | Run a lifecycle hook with real vault context |
| `ztl hook new <STAGE> <NAME>` | Scaffold a render-pipeline hook; flags: `--lang py\|js\|sh`, `--ecosystem pandoc\|mdbook\|remark`, `--force` |
| `ztl hook test <NAME>` | Run a hook against its fixture and diff the golden; `--update` regenerates |
| `ztl hook fixture --from <PAGE> --hook <NAME>` | Capture a vault page into the hook's fixture dir |
| `ztl hook watch <NAME>` | Live-reload the hook's persistent-mode subprocess on source change |
| `ztl hook coverage` | Per-hook coverage from the most-recent build; `--stage`, `--json` |
| `ztl hook dry-run <STAGE>/<NAME>` | Print pages the hook's selector matches; hook is not invoked; `--limit`, `--theme` |
| `ztl hook capabilities` | Probe every hook's supported stages + AST types; `--stage`, `--json` |

## ztl ecosystem

Probe plugin ecosystems. Requires `ecosystems-v1` feature flag.

- `ztl ecosystem check` — per-ecosystem detection, version, configured-hook count, reachable plugins. Exits 0 when all *configured* ecosystems are available. See [[Plugin Ecosystems]].

## ztl ast

Inspect the ztl-ext AST for a page or diff two AST JSON documents.

- `ztl ast sample <FILE>` — print canonical AST JSON. `--stage pre-parse|transform|post-render` chooses which stage's input to emit (default `transform`).
- `ztl ast diff <BEFORE> <AFTER>` — tree-aware structural diff; non-zero exit on non-empty diff.

## ztl agent

Agent lifecycle integration for LLM / automation hooks.

- `ztl agent run <NAME> [-- <EXTRA>...]` — run an on-agent hook with task context. Flags: `--pages <TARGET>...`, `--budget <TOKENS>` (0 = unlimited), `--theme`.

## ztl invite

Generate an invitation token for a collaborator. See [[Invitations]].

Key flags: `--as <USER>` (inviter), `--role reader|editor|admin`, `--pages <GLOB>` (scoped grant), `--expires 72h|24h|7d` (default 72h), `--host`, `--port` (URL generation).

```bash
ztl invite --as alice --role editor
ztl invite --as alice --role reader --pages 'projects/*' --expires 7d
```

## ztl agent-token

Derive a headless API token from a BIP39 mnemonic.

```bash
ztl agent-token --mnemonic "word1 word2 ... word12"
```

## ztl derive-ssh-key

Derive an SSH ed25519 key from a BIP39 mnemonic. Useful for ephemeral containers where one seed covers the server key and the git SSH key.

```bash
ztl derive-ssh-key --mnemonic "word1 ... word12" --out /root/.ssh/id_ed25519
```

## ztl cap

Capability-URL static-site operations. See [[Capability URLs]].

| Subcommand | Purpose |
| --- | --- |
| `ztl cap genkey` | Emit the content-encryption secret + signing keypair (once) |
| `ztl cap invite <NAME> --cohort <ID>` | Issue an invitation grant; flags: `--expires`, `--pages <GLOB>`, `--recipient <PUBKEY>`, `--via enrol-page`, `--split-key`, `--site-url`, `--slug` |
| `ztl cap list` | List issued grants; `--cohort`, `--output` |
| `ztl cap revoke <GRANT_ID>` | Revoke a grant |
| `ztl cap rotate --cohort <ID>` | Rotate a cohort's content-key salt (URLs stay stable) |
| `ztl cap finalise <GRANT_ID>` | Mark a grant operator-confirmed; `--rotate-grant` |
| `ztl cap check` | Stale-grant + public-safety audit; `--public-safety` |
| `ztl cap sweep` | Mark past-expiry grants as revoked |
| `ztl cap pair` | SPAKE2 pubkey handoff; `--grantor` / `--grantee`, `--peer`, `--phrase`, `--pubkey` |
| `ztl cap audit-diff [OLD] [NEW]` | Scan a diff for malicious content; `--corpus`, `--corpus-root` |
| `ztl cap rotate-signing-key` | Rotate the Ed25519 vault-signing key (rebuilds every page) |
| `ztl cap emergency-shutdown` | Print the takedown checklist (no files changed) |

## ztl reason

SPL queries over the vault. Requires `--features reason`. See [[Running Queries]].

Subcommands (consult `ztl reason --help` inside a feature-reason build): `status`, `explain`, `conflicts`, `why-not`, `what-if`.

## ztl mcp

Start a Model Context Protocol server. Requires `--features mcp`. See [[MCP Server]].

## ztl delegate

Issue a delegate JWT for an MCP client. Requires `--features mcp`.

## ztl completions

Print a shell-completion script.

```bash
ztl completions zsh > ~/.zsh/completions/_ztl
```

Supported shells: `bash`, `zsh`, `fish`, `elvish`, `powershell`.

## ztl man

Print a roff(7) man page. Preview with `ztl man | man -l -`.

## Related

- [[Configuration]]
- [[Installation]]
- [[Quick Start]]
- [[Glossary]]
- [[Frontmatter Fields]]
