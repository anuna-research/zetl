---
title: MCP Server
tags: [automation, mcp, agents]
---

# MCP Server

Expose your vault to AI agents as typed [Model Context Protocol](https://modelcontextprotocol.io) tools. Claude Desktop, Cursor, or any MCP-aware agent can then search pages, traverse backlinks, resolve [[Wikilinks]], ask for similar pages, and run [[Running Queries|reasoning queries]] — without giving up control of the underlying files.

> **Requires `--features mcp` at install time.** See [[Installation]].

## Why

Agents need structured access to your knowledge. Dumping a vault into a context window loses the graph; scraping files on disk loses the reasoning layer. MCP gives the agent a *typed tool surface* — `search`, `get_page`, `links`, `backlinks`, `path`, `similar`, `check`, `status`, `reason` — backed by the same index ztl uses itself. You stay local-first; the agent never touches your files directly.

## Transports

```bash
# stdio — for Claude Desktop, Cursor, and any editor that spawns the server
ztl -d ./my-vault mcp

# HTTP — for remote agents, CI bots, shared deployments
ztl -d ./my-vault mcp --transport http --port 3100
```

`--transport stdio` is the default. HTTP binds to `127.0.0.1` by default; pass `--insecure` to allow a non-loopback bind (don't do this without a reverse proxy and a delegate token — see below). `--cors-origin https://agent.example.com` scopes the browser-side surface.

## The available tools

| Tool | What it does |
|------|--------------|
| `search` | Full-text search across the vault |
| `get_page` | Fetch a page by name — Markdown + frontmatter |
| `links` | List forward links from a page |
| `backlinks` | List pages pointing at a page |
| `path` | Shortest path between two pages in the link graph |
| `similar` | SimHash-nearest pages (see [[Similar Pages]]) |
| `check` | Run `ztl check` — dead links, orphans |
| `status` | Reasoning conclusions for a page (requires `--features reason`) |
| `reason` | Evaluate an SPL query (requires `--features reason`) |

The reasoning tools exist only when ztl was built with both `mcp` and `reason` features — see [[What is Defeasible Reasoning]] for what they return.

## Delegate tokens

An agent shouldn't get your unscoped identity. ztl issues user-signed JWTs — **delegate tokens** — that scope access per-tool and per-page glob:

```bash
# All tools, all pages, no expiry (convenient; pair with a short-lived key)
ztl delegate

# Read-only subset
ztl delegate --tools search,get_page,links,backlinks

# Scope to a folder
ztl delegate --scope "projects/**"

# Time-limited
ztl delegate --expiry 7d

# First-time key setup from a BIP39 mnemonic
ztl delegate --mnemonic "word1 word2 ... word12" --save-key
```

`--save-key` writes the derived signing key to `~/.config/ztl/identity.key` so later `delegate` runs don't need the mnemonic. The server verifies tokens against the issuing identity and rejects anything missing the claimed scope or expired. `--allowed-issuer <DID>` on the `mcp` command restricts which identities may issue tokens for that server — the right knob for a shared HTTP deployment.

## Claude Desktop config

Drop this into `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "ztl": {
      "command": "ztl",
      "args": ["-d", "/path/to/vault", "mcp"]
    }
  }
}
```

Claude Desktop spawns one stdio subprocess per MCP server it knows about, so the agent has live access the whole time the chat is open. Restart Claude Desktop after editing the config. The ztl server logs to stderr — visible in the Desktop log pane if a tool call misbehaves.

## Cursor / other IDEs

Any MCP-aware client that takes a `command` + `args` pair works the same way. Point the config at the `ztl` binary with `-d <vault>` and `mcp`; the client handles transport. For HTTP clients, configure the token header with the output of `ztl delegate`.

## What the agent sees

Tools are typed. `search` takes a query string and an optional result limit; `get_page` takes a page name and returns structured `{frontmatter, content, path}`. The agent's tool-use loop can chain calls — `search` for a term, `backlinks` on the top result, `path` between that and a reference note — without ever seeing a raw filesystem path it could mis-interpret.

Writes are not exposed. The MCP surface is read-only. Agents that need to *create* pages should use the [[Web Server|ztl serve]] API, which has proper auth and CRDT-aware saves.

## Related

- [[Lifecycle Hooks]] — the `on-agent` hook fires on every MCP tool call; use it for audit logging.
- [[Running a Team Server]] — sharing a vault across users; MCP plays well with the same collab identity keys.
- [[Capability URLs]] — the static-site analogue for sharing without a running server.
- [[Installation]] — `cargo install --features mcp`.
- [[Running Queries]] — what the `reason` tool evaluates.
