# zetl

[![License: AGPL-3.0](https://img.shields.io/badge/License-AGPL--3.0--or--later-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-2021-orange.svg)](https://www.rust-lang.org/)
[![Status: alpha](https://img.shields.io/badge/status-alpha-yellow.svg)](#)

Bi-directional wikilink graph CLI with defeasible reasoning for personal knowledge management.

zetl parses `[[wikilinks]]` from Markdown files, builds an in-memory link graph, and exposes query, validation, search, and visualization commands. Optionally, it extracts [Spindle Lisp (SPL)](https://codeberg.org/anuna/spindle-rust) code blocks from your vault and performs defeasible reasoning — drawing conclusions that can be defeated by stronger evidence. Designed for both AI agents (JSON output) and humans (tables, web UI).

## Features

- **Wikilink parsing** — `[[target]]`, `[[target|alias]]`, `[[target#heading]]`, `[[target^block-id]]`, `![[embeds]]`
- **Graph queries** — forward links, backlinks, multi-hop traversal, shortest path between pages
- **Vault diagnostics** — dead links, orphan pages, syntax errors, SPL parse errors
- **Full-text search** — content search with regex support, frontmatter/code-block awareness
- **Fuzzy matching** — SimHash-based page name similarity
- **Page viewer** — Xanadu-inspired two-pane reader with context cards, bridge connectors, and wikilink navigation
- **Web UI** — local web server with rendered pages, transclusion panels, backlink navigation, and inline editing
- **Static site export** — generate a deployable HTML site from your vault (same look, no server required)
- **Vault history** — jj-backed temporal snapshots with time-travel queries (`--at "3 days ago"`), graph evolution timeline, and automatic snapshotting on index
- **Lifecycle hooks** — git-style executable hooks at `pre-build`, `post-build`, `post-index`, `post-check`, `pre-serve`, and `on-save` lifecycle points; receive vault context as JSON on stdin
- **Multi-user collaboration** — passkey (WebAuthn) authentication, role-based access control (reader/editor/admin), invitation links, CRDT-based real-time co-editing via WebSocket, BIP39 mnemonic account recovery
- **Custom themes** — override Minijinja templates and static assets via `.zetl/themes/`, with full access to frontmatter and vault context; themes can bundle hooks
- **Content-addressable blocks** — BLAKE3 Merkle leaves for headings, paragraphs, code blocks, and SPL
- **Incremental caching** — two-tier (mtime + hash) index for both wikilinks and reasoning theories
- **MCP server** — expose graph traversal, search, and reasoning as typed MCP tools over stdio and HTTP transports; user-signed JWT delegation with per-tool and per-page scoping
- **Agent-friendly** — auto-detects JSON when piped, structured errors on stderr, non-zero exit codes, shell completions + man page
- **Defeasible reasoning** — extract SPL facts and rules from Markdown, build a vault-wide theory, derive conclusions with full provenance
- **Proof trees** — explain why a conclusion holds, traced back to source files and line numbers
- **What-if analysis** — hypothetical reasoning: add temporary facts and see what changes
- **Abductive reasoning** — find what facts are needed to prove a goal
- **Conflict detection** — find unresolved logical contradictions with resolution suggestions
- **Cross-referencing** — link graph and logical theory annotate each other

## Install

Requires a Rust toolchain ([rustup](https://rustup.rs/)).

```bash
# Wikilink features only
make install

# With defeasible reasoning
cargo install --path . --features reason

# With vault history (jj-backed temporal snapshots)
cargo install --path . --features history

# Both reasoning and history
cargo install --path . --features "reason,history"

# All features (reasoning + history + semantic search + MCP)
cargo install --path . --features "reason,history,semantic,mcp"

# With MCP server only
cargo install --path . --features mcp
```

Collaboration mode (`--collab`) is always available — no feature flag needed. SPL-based access control requires `--features reason`.

Without `--features reason`, `zetl reason` prints a helpful error instead of failing silently. Without `--features history`, history-related template variables and API endpoints gracefully degrade to null.

Prebuilt binaries are not yet published. Users need a Rust toolchain to build from source.

### Shell completions and man page

`make install` installs the binary, `man zetl`, and bash/zsh/fish completions into `$PREFIX` (default `~/.local`). After installing, run `man zetl` directly — no extra steps, provided `~/.local/share/man` is on your `MANPATH`.

For manual or packaging use:

```bash
zetl man > /usr/local/share/man/man1/zetl.1    # install the man page
zetl man | man -l -                            # preview without installing

zetl completions bash > /etc/bash_completion.d/zetl
zetl completions zsh  > ~/.zfunc/_zetl
zetl completions fish > ~/.config/fish/completions/zetl.fish
zetl completions powershell > $PROFILE/zetl.ps1
```

### Non-interactive / CI usage

Pass `--no-input` to disable interactive prompts (e.g. the `zetl view` page picker). Commands that would otherwise prompt will exit non-zero instead.

## Quick start

The included `demo-vault/` is a self-referential knowledge base about zetl itself, with wikilinks and SPL throughout.

```bash
# Build the link index
zetl -d ./demo-vault index

# Query links
zetl -d ./demo-vault links "Scanner"
zetl -d ./demo-vault backlinks "Cache" --depth 2

# Run reasoning over all SPL in the vault
zetl -d ./demo-vault reason status
zetl -d ./demo-vault reason explain "release-candidate" --format natural
zetl -d ./demo-vault reason conflicts
```

## Usage

### Wikilink commands

```bash
# Build or refresh the link index
zetl -d ./my-vault index

# Forward and back links
zetl -d ./my-vault links "Some Page"
zetl -d ./my-vault backlinks "Some Page"
zetl -d ./my-vault backlinks "Some Page" --depth 2    # multi-hop

# Find shortest path between pages
zetl -d ./my-vault path "Page A" "Page B"

# Search content
zetl -d ./my-vault search "query"
zetl search "pattern" --regex

# Validate vault
zetl -d ./my-vault check
zetl check --dead-links --fail-on error   # cwd is vault
zetl check --spl                        # SPL diagnostics only
zetl check --drift                      # detect SPL changes since last theory build

# Fuzzy page name matching
zetl -d ./my-vault similar "zettelkasen"

# Content-addressable blocks
zetl -d ./my-vault blocks "Some Page"                    # all blocks
zetl -d ./my-vault blocks "Some Page" --type heading     # headings only
zetl -d ./my-vault blocks --resolve abc123               # resolve by hash prefix

# Stats and export
zetl -d ./my-vault stats
zetl -d ./my-vault list
zetl -d ./my-vault export    # full graph as JSON

# Page viewer (two-pane reader)
zetl -d ./my-vault view "Some Page"
zetl -d ./my-vault view                                  # opens page picker
zetl -d ./my-vault view "Some Page" --context-lines 10   # taller context cards

# Web UI
zetl -d ./my-vault serve                                 # http://localhost:3000
zetl -d ./my-vault serve --port 8080
zetl -d ./my-vault serve --theme paper                   # custom theme

# Multi-user collaboration
zetl -d ./my-vault serve --collab --init-owner --owner-name Alice  # first-time setup
zetl -d ./my-vault serve --collab                                  # start collab server
zetl -d ./my-vault serve --collab --server-key-seed "word1 ..."    # deterministic server key
zetl -d ./my-vault invite --as Alice --role editor                 # invite a collaborator
zetl -d ./my-vault invite --as Alice --role reader --pages "projects/*"
zetl derive-ssh-key --mnemonic "word1 ..." --out ~/.ssh/id_ed25519 # derive SSH key from seed

# Static site export
zetl -d ./my-vault build                                 # generates dist/
zetl -d ./my-vault build --out-dir site                  # custom output directory
zetl -d ./my-vault build --theme paper                   # build with custom theme
```

### History commands

Requires `--features history` at build time. History uses jj-lib for automatic, silent VCS snapshots stored in `.zetl/jj/`.

```bash
# View graph evolution timeline
zetl -d ./my-vault history log
zetl history log --since "last week"

# Track a page's evolution across snapshots
zetl history page "Some Page"

# Query any command at a point in time
zetl -d ./my-vault --at "3 days ago" links "Some Page"
zetl --at "2024-01-15" stats
zetl --at "last monday" check

# Watch vault and auto-snapshot on changes
zetl -d ./my-vault watch
```

The `--at` flag works on all read-only subcommands (`links`, `backlinks`, `stats`, `check`, `search`, etc.), resolving the vault state to a historical snapshot. Time expressions support ISO 8601 dates, relative natural language ("3 days ago", "last monday"), and VCS refs ("HEAD~1").

When the history feature is enabled, `zetl index` automatically creates a snapshot, `vault.history` and `page.history` are available in templates, `page.backlinks[].since` provides backlink timestamps, hooks receive a `history` context object, and `zetl build` writes `history-index.json`.

### Reasoning commands

Requires `--features reason` at build time. All commands operate on SPL extracted from ` ```spl ` fenced code blocks in Markdown files and standalone `.spl` files.

```bash
# What does the vault's combined theory conclude?
zetl -d ./demo-vault reason status
zetl reason status --positive              # only +D, +d conclusions
zetl reason status --literal "release*"    # wildcard filter

# Why does a conclusion hold? (proof tree with provenance)
zetl -d ./demo-vault reason explain "release-candidate"
zetl -d ./demo-vault reason explain "good-cli-tool" --format natural
zetl -d ./demo-vault reason explain "scanner-complete" --format dot

# Why can't something be proved?
zetl -d ./demo-vault reason why-not "docs-updated"

# What facts would make a goal provable?
zetl -d ./demo-vault reason require "release-candidate"
zetl -d ./demo-vault reason require "release-candidate" --assume "(given docs-updated)"

# Hypothetical: what if we add facts?
zetl -d ./demo-vault reason what-if "(given docs-updated)" --goal "release-candidate"
zetl -d ./demo-vault reason what-if --file extra.spl

# Find unresolved conflicts (the demo vault has a deliberate tension in Cache.md)
zetl -d ./demo-vault reason conflicts
zetl -d ./demo-vault reason conflicts --suggest --fail-on-conflicts

# Export the combined theory
zetl -d ./demo-vault reason export                         # JSON
zetl -d ./demo-vault reason export --format spl            # reconstructed SPL with provenance
zetl -d ./demo-vault reason export --with-conclusions

# Trace a conclusion back to source files
zetl -d ./demo-vault reason provenance "release-candidate"

# Cross-reference links with reasoning
zetl -d ./demo-vault links "Cache" --with-conclusions
zetl -d ./demo-vault backlinks "Reasoning Engine" --with-conclusions
```

Output format auto-detects: tables in an interactive terminal, JSON when piped or redirected. Force one with `--json` or `-f table` (global flags, can appear before or after the subcommand). The `reason explain` subcommand also accepts `--format natural` and `--format dot`. Errors go to stderr so piped stdout stays valid JSON.

### MCP server

Requires `--features mcp` at build time. Exposes zetl's graph, search, and reasoning as typed [MCP](https://modelcontextprotocol.io) tools for AI agents.

```bash
# Start MCP server over stdio (for Claude Desktop, Cursor, etc.)
zetl -d ./my-vault mcp

# Start over HTTP (for remote agents)
zetl -d ./my-vault mcp --transport http --port 3100

# Issue a delegate token for your agent
zetl delegate                                           # all tools, all pages, no expiry
zetl delegate --tools search,get --scope "projects/**"  # scoped access
zetl delegate --expiry 7d                               # time-limited
zetl delegate --mnemonic "word1 word2 ..." --save-key   # first-time key setup
```

**Available tools:** `search`, `get_page`, `links`, `backlinks`, `path`, `similar`, `check`, `status`, `reason`

**Claude Desktop config** (`claude_desktop_config.json`):
```json
{
  "mcpServers": {
    "zetl": {
      "command": "zetl",
      "args": ["-d", "/path/to/vault", "mcp"]
    }
  }
}
```

## SPL in Markdown

Embed Spindle Lisp in any Markdown file using fenced code blocks:

````markdown
# Rust for CLI

zetl is written in Rust for type safety and fast startup.

```spl
(given type-safe)
(given single-binary)
(given fast-startup)
```

These facts feed into the vault-wide theory.
````

You can also place standalone `.spl` files anywhere in the vault:

```spl
; release-readiness.spl — rules that combine facts from across the vault
(normally r-good-cli
  (and fast-startup single-binary type-safe)
  good-cli-tool)
```

zetl merges all SPL from across the vault into a single theory, reasons over it, and traces every conclusion back to its source file and line number. The `demo-vault/` included in this repo is a working example — it documents zetl itself using both wikilinks and SPL.

### Conclusion types

| Tag  | Meaning |
|------|---------|
| `+D` | Definitely provable (strict rules, no defeaters possible) |
| `-D` | Definitely not provable |
| `+d` | Defeasibly provable (inferred, no active defeaters) |
| `-d` | Defeasibly not provable (blocked or no derivation path) |

## Page viewer (`zetl view`)

Xanadu-inspired two-pane reader for focused page navigation. The left pane renders the current note with numbered `[N]` anchor glyphs at each wikilink. The right pane shows context cards — excerpts from forward-linked pages. A bridge column connects anchors to their cards with colored connectors. Falls back to single-pane layout in narrow terminals (<60 cols).

```bash
zetl view "Page Name"                  # open a page
zetl view                              # open page picker
zetl view "Page Name" --context-lines 10 --main-width 60
```

| Key | Action |
|-----|--------|
| `j`/`k` | Scroll (or cycle links in focus mode) |
| `Ctrl-d`/`Ctrl-u` | Half-page scroll |
| `g`/`G` | Top / bottom of note |
| `Tab` | Toggle between scroll and focus mode |
| `Enter` | Navigate to focused link |
| `[`/`]` | Session history back / forward |
| `/` | Open page picker |
| `?` | Toggle keybindings help |
| `q` | Quit |

## Web

### Live server (`zetl serve`)

Local web UI for browsing the vault. Renders Markdown pages with a sidebar, backlink list, transclusion panel (forward-link excerpt cards with SVG bridge connectors), and a CodeMirror 6 editor with save-and-reindex and page deletion. Pages are rendered through a Minijinja template engine with YAML frontmatter available in templates.

```bash
zetl -d ./my-vault serve                                        # single-user
zetl -d ./my-vault serve --collab --init-owner --owner-name Jo  # first-time collab setup
zetl -d ./my-vault serve --collab                                # multi-user mode
zetl -d ./my-vault serve --collab --server-key-seed "word1 ..."  # deterministic server key
zetl -d ./my-vault serve --port 8080 --theme dark                # custom port and theme
```

### API endpoints

The serve mode exposes JSON API endpoints (authenticated via session cookie or Bearer token in collab mode):

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/api/pages` | GET | List all pages |
| `/api/pages/{slug}` | GET | Get page content and metadata |
| `/api/pages/{slug}` | PUT | Update page content |
| `/api/pages/{slug}` | DELETE | Delete a page |
| `/api/search?q=...` | GET | Full-text search |
| `/api/graph` | GET | Full link graph |
| `/api/index` | POST | Trigger reindex |
| `/api/comments/{slug}` | GET/POST | Page comments |
| `/api/access-request` | POST | Request access to a page (collab mode) |
| `/api/ws/ticket` | POST | Obtain a WebSocket ticket (collab mode) |
| `/ws/edit/{slug}` | WS | Real-time collaborative editing (collab mode) |
| `/help` | GET | Built-in help page (install + usage; override via theme `help.html`) |

### Static site (`zetl build`)

Generates a static HTML site with the same look and feel as `zetl serve`, minus the edit button and save functionality. The output can be uploaded to any static host (GitHub Pages, Netlify, S3, etc.).

```bash
zetl -d ./my-vault build                  # generates dist/
zetl -d ./my-vault build --out-dir site   # custom output directory
zetl -d ./my-vault build --theme paper    # build with a custom theme

# Preview locally
python3 -m http.server -d dist 8080
```

Output structure:
```
dist/
  index.html              # vault overview with stats and page grid
  _static/                # copied from .zetl/themes/<theme>/static/
  page/
    Some Page/index.html   # one page per note
    Another/index.html
```

### Themes

Both `serve` and `build` support custom themes via `--theme <name>`. Themes live in `.zetl/themes/<name>/` and can override any of the built-in Minijinja templates:

```
.zetl/themes/paper/
  base.html       # master layout (sidebar, search modal, scripts)
  index.html      # vault landing page
  page.html       # single page view
  folder.html     # folder index
  help.html       # /help page (install + usage)
```

You only need to provide the templates you want to override — the rest fall back to the built-in defaults. All templates use [Minijinja](https://github.com/mitsuhiko/minijinja) syntax and extend `base.html` via `{% extends "base.html" %}`.

### Frontmatter

YAML frontmatter is parsed and available in page templates as `page.frontmatter`. For example, a page with:

```markdown
---
tags: [rust, cli]
status: draft
---
# My Page
```

exposes `page.frontmatter.tags` and `page.frontmatter.status` in templates.

### Static assets

Place static files (CSS, JS, images) in `.zetl/themes/<theme>/static/`. During `serve`, they're available at `/_static/<path>`. During `build`, they're copied to `_static/` in the output directory.

### Theme authoring reference

Templates use [Minijinja](https://github.com/mitsuhiko/minijinja) (Jinja2-compatible). All child templates should `{% extends "base.html" %}` and override blocks. You only need to provide the templates you want to change — missing ones fall back to the built-in defaults.

#### Template blocks

`base.html` defines these blocks for child templates:

| Block | Used by | Purpose |
|-------|---------|---------|
| `title` | all | Page `<title>` |
| `head` | all | Extra `<head>` content |
| `styles` | all | Extra `<style>` rules |
| `content` | all | Main content area |
| `sidebar` | all | Sidebar page list |
| `scripts` | all | Extra `<script>` tags |

`index.html` also exposes finer-grained blocks so a custom theme can replace a single region without rewriting the whole landing page:

| Block | Purpose |
|-------|---------|
| `index_title` | `<title>` for the vault index |
| `index_header` / `index_heading` | Top heading (defaults to `vault.name`) |
| `index_intro` | Empty by default — use for a banner, description, or widget |
| `index_stats` | The pages/links/dead/orphans stat row |
| `index_before_pages` / `index_after_pages` | Slots around the page grid |
| `index_pages` / `index_pages_heading` | The "All Pages" grid and its heading |

#### Template variables

**All templates** receive:

| Variable | Type | Description |
|----------|------|-------------|
| `vault.name` | string | Vault directory name |
| `vault.pages` | array | All pages (`title`, `slug`, `outlink_count`, `backlink_count`) |
| `vault.stats` | object | `total_pages`, `total_links`, `dead_links`, `orphans` |
| `vault.history` | object\|null | Vault history summary: snapshot count, trend, oldest/newest (null without history) |
| `search_index` | string | JSON search index (use with `{{ search_index \| safe }}`) |
| `theme` | string | Active theme name |
| `active_slug` | string | Current page slug (for sidebar highlighting) |

**`page.html`** also receives:

| Variable | Type | Description |
|----------|------|-------------|
| `page.title` | string | Page name |
| `page.slug` | string | URL slug |
| `page.content_html` | string | Rendered HTML (use with `\| safe`) |
| `page.frontmatter` | object | Parsed YAML frontmatter (e.g. `page.frontmatter.tags`) |
| `page.backlinks` | array | Backlinks (`title`, `slug`, `line`, `since`) — `since` is an RFC 3339 timestamp (null without history) |
| `page.history` | object\|null | Page history: `created_at`, `last_changed`, `age_days`, `stable_days`, `link_trend`, `recent_changes` (null without history) |
| `page.outlinks` | array | Outgoing links (`title`, `slug`, `is_dead`, `color`) |
| `page.breadcrumbs` | array | Path breadcrumbs (`title`, `slug`) |
| `page.transclusion_cards` | string | Pre-rendered transclusion HTML (`\| safe`) |
| `page.is_new` | bool | True if page doesn't exist yet (new page mode) |
| `page.raw_escaped` | string? | Raw markdown source (serve mode only, for editor) |
| `mode` | string | `"serve"` or `"build"` |

**`folder.html`** also receives:

| Variable | Type | Description |
|----------|------|-------------|
| `folder.name` | string | Folder name |
| `folder.slug` | string | Folder slug |
| `folder.breadcrumbs` | array | Path breadcrumbs (`title`, `slug`) |
| `folder.subfolders` | array | Child folders (`name`, `slug`, `page_count`) |
| `folder.pages` | array | Pages in folder (`title`, `slug`, `outlink_count`, `backlink_count`) |
| `folder.total_pages` | int | Count of direct child pages |

#### Minimal example

A theme that only changes the color scheme (override just `base.html`):

```
.zetl/themes/dark/
  base.html
```

The child templates (`index.html`, `page.html`, `folder.html`) automatically fall back to the built-ins and extend your custom `base.html`.

### Hooks

zetl supports git-style lifecycle hooks — executable scripts in `.zetl/hooks/` that run at defined points during vault operations. Hooks receive structured JSON context on stdin and environment variables, enabling custom automation without modifying the binary.

```bash
# List all active hooks for the current vault and theme
zetl -d ./my-vault hook list
zetl -d ./my-vault hook list --theme paper

# Manually run a hook with real vault context (useful for testing)
zetl -d ./my-vault hook run post-build
zetl -d ./my-vault hook run on-save -- '{"saved":{"file":"test.md","page":"Test","content_length":100}}'
```

#### Lifecycle points

| Hook | Trigger | Can Abort? |
|------|---------|------------|
| `pre-build` | Before `zetl build` renders pages | Yes |
| `post-build` | After `zetl build` completes | No (warn only) |
| `post-index` | After `zetl index` completes | No |
| `post-check` | After `zetl check` collects diagnostics | No |
| `pre-serve` | Before `zetl serve` starts the server | Yes |
| `on-save` | After a page is saved in `zetl serve` | No |
| `on-agent` | When an agent API request is received | No |
| `on-access-request` | When a user requests access to a page (collab mode) | No |

#### Writing a hook

Create an executable file in `.zetl/hooks/` named after the lifecycle point:

```bash
# .zetl/hooks/post-build
#!/bin/bash
# Generate an RSS feed from pages with "date" frontmatter
jq -r '.pages[] | select(.frontmatter.date) | ...' < /dev/stdin > "$ZETL_OUT_DIR/feed.xml"
```

```bash
chmod +x .zetl/hooks/post-build
```

Every hook receives:
- **stdin**: JSON context with vault metadata, page list, link graph, history (when available), and hook-specific fields
- **Environment**: `ZETL_HOOK`, `ZETL_VAULT_ROOT`, `ZETL_THEME`, `ZETL_VERSION`, plus hook-specific vars like `ZETL_OUT_DIR` and `ZETL_PORT`
- **Working directory**: vault root

Hooks have a 30-second timeout. Pre-hooks (`pre-build`, `pre-serve`) abort the parent operation on non-zero exit; all other hooks warn and continue.

#### Theme-bundled hooks

Themes can ship hooks in their `hooks/` subdirectory. When a theme is active (`--theme <name>`), its hooks run before vault hooks at each lifecycle point. Both theme and vault hooks run if both exist.

```
.zetl/themes/fountain/
  hooks/
    post-build    # runs automatically with --theme fountain
  base.html
  page.html
```

## Collaboration

zetl supports multi-user collaborative editing with `--collab` mode. Authentication uses WebAuthn passkeys (Touch ID, security keys), with BIP39 mnemonic recovery phrases as a fallback.

### Setup

```bash
# First-time: bootstrap the vault owner
zetl -d ./my-vault serve --collab --init-owner --owner-name Alice
# Save the 12-word recovery phrase printed to the terminal!

# Subsequent starts (owner already exists)
zetl -d ./my-vault serve --collab
```

On first start, register a passkey at `http://localhost:3000` when prompted.

### Inviting collaborators

```bash
# Generate an invitation link (copies to clipboard)
zetl -d ./my-vault invite --as Alice --role editor

# Scoped to specific pages
zetl -d ./my-vault invite --as Alice --role reader --pages "projects/*"

# Custom expiry (default 72h)
zetl -d ./my-vault invite --as Alice --role editor --expires 24h
```

Or use the web UI at `/_admin/invite` to create and manage invitations.

Roles: `reader` (view only), `editor` (view + edit), `admin` (full control including invitations).

### Real-time editing

When multiple users open the same page, edits sync in real-time via WebSocket using a Peritext CRDT engine. Each save auto-commits to git with the author's name.

### Account recovery

If you lose access to your passkey, recover your account at `/auth/recovery` using your display name and 12-word recovery phrase. This issues a new session so you can re-register a passkey.

### Deterministic keys from a seed phrase

For containerised or ephemeral deployments, a single BIP39 mnemonic can deterministically derive all keys zetl needs. This avoids managing key files across redeploys.

The seed derives three keys at distinct SLIP-0010 paths:

| Path | Purpose | Flag / Command |
|------|---------|----------------|
| `m/44'/0'/0'` | User account recovery | (generated at `--init-owner`) |
| `m/44'/1'/0'` | Collab server signing key | `--server-key-seed` |
| `m/44'/2'/0'` | SSH ed25519 key for git | `zetl derive-ssh-key` |

```bash
# Start the collab server with a deterministic server key
zetl -d ./my-vault serve --collab --server-key-seed "word1 word2 ... word12"

# Or set via environment variable
export ZETL_SERVER_KEY_SEED="word1 word2 ... word12"
zetl -d ./my-vault serve --collab

# Derive an SSH key (for git push) from the same seed
zetl derive-ssh-key --mnemonic "word1 word2 ... word12" --out ~/.ssh/id_ed25519
# Prints the public key for adding to your git remote (GitLab/GitHub)
```

When `--server-key-seed` is provided, the derived key is written to `.zetl/collab/server.key` so that all code paths use a consistent key. Destroy the volume, redeploy with the same seed — same server identity, same SSH key.

### Agent tokens

For headless API access (CI, scripts, bots):

```bash
zetl -d ./my-vault agent-token --mnemonic "word1 word2 ... word12"
```

Use the token as a Bearer token: `Authorization: Bearer <token>`.

### Security features

- WebAuthn passkey authentication (no passwords)
- CSRF protection on all state-changing endpoints
- Per-IP and per-user rate limiting on auth endpoints
- Session idle and absolute timeouts
- Ed25519-signed invitation tokens with single-use nonces
- Deterministic server key derivation from BIP39 mnemonic via SLIP-0010
- SPL-based access control with deontic modalities (when built with `--features reason`)
- Git auto-commit on every save with author attribution
- Write-ahead log for CRDT crash recovery
- Server key file permission enforcement

## Compatibility

Works with any Markdown vault using `[[wikilink]]` syntax:

- Obsidian
- Logseq
- Foam
- Dendron

SPL embedding is optional — vaults work fine with just wikilinks. zetl never modifies your files. The index and theory cache are disposable and stored in `.zetl/`.

## Development

```bash
make                    # fmt + clippy + build + test
make build              # debug build
make release            # release build
make test               # run tests
make test-reason        # run tests with reason feature
cargo test --features history  # run history integration tests
make check              # fmt + clippy
make fmt-fix            # auto-format code
make doc-open           # generate and open docs in browser
make clean              # remove build artifacts
```

## License

[AGPL-3.0](LICENSE)
