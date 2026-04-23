---
title: "SPEC-021: ztl mcp — Model Context Protocol Server for Graph-Native Knowledge Tools"
version: 0.2.0
status: draft
date: 2026-04-07
audience: agent, human
parent: SPEC-001
related:
  - SPEC-002  # Graph Model
  - SPEC-013  # Full-Text BM25 Search
  - SPEC-018  # Semantic/Hybrid Vector Search
  - SPEC-005  # Defeasible Reasoning (SPL)
  - SPEC-003  # Vault Diagnostics
  - SPEC-020  # Multi-User Collaborative Editing (agent tokens, auth)
dependencies:
  - rmcp (Rust MCP SDK)
  - tokio (async runtime, already a dependency)
  - jsonwebtoken (JWT encoding/decoding with EdDSA)
  - ed25519-dalek (already a SPEC-020 dependency)
  - SPEC-020 (user identity, did:key, stored identity key)
---

| Field        | Value                                                     |
|--------------|-----------------------------------------------------------|
| Document     | SPEC-021                                                  |
| Title        | ztl mcp — Model Context Protocol Server for Graph-Native Knowledge Tools |
| Version      | 0.2.0                                                     |
| Status       | Draft                                                     |
| Author       | Agent (USDD Protocol v1.3.0)                              |
| Date         | 2026-04-07                                                |
| Audience     | agent, human                                              |
| Trace        | USDD §2 (Vision → Specification)                          |
| Parent       | SPEC-001                                                  |
| Related      | SPEC-002, SPEC-013, SPEC-018, SPEC-005, SPEC-003, SPEC-020 |
| Feature Gate | `--features mcp`                                          |

---

## 1. Overview

### 1.1 Problem

LLM agents operating over personal knowledge bases need structured access to the vault's content, search indexes, and graph topology. Today, an agent can interact with ztl through the HTTP API (`ztl serve`) or by invoking CLI commands. Both approaches have friction:

- **HTTP API** requires a running server, URL configuration, and authentication setup. It is designed for web UIs and programmatic clients, not for the tool-calling pattern that MCP clients use.
- **CLI invocation** requires subprocess management, output parsing, and lacks the bidirectional capability negotiation that MCP provides.

Neither approach exposes ztl's graph structure as first-class tool affordances. An LLM that wants to find the shortest path between two concepts, or discover what a page links to, must either know the right CLI flags or the right API endpoints. MCP's tool discovery protocol eliminates this — the agent sees a typed tool catalog and calls what it needs.

### 1.2 Core Insight

ztl's wikilink graph makes its MCP server fundamentally more capable than a search-only MCP server. Most knowledge-base MCP integrations expose search and retrieval. ztl can additionally expose **graph traversal** — forward links, backlinks, shortest path, neighbourhood — giving agents structural reasoning over the knowledge base, not just keyword lookup. The graph is the differentiator.

### 1.3 Design Philosophy

- **MCP server imports only the public SDK API.** The MCP layer calls the same Rust functions that `ztl serve` and the CLI use (`LinkGraph::forward_links`, `LinkGraph::backlinks`, `LinkGraph::shortest_path`, the Tantivy search index, the vault scanner). It never reaches into internal state or bypasses the pipeline.
- **Tools are atomic and composable.** Each MCP tool does one thing. An agent composes `search` → `get` → `links` → `get` to navigate the vault. The server does not try to be clever.
- **Transport is a deployment choice, not a code change.** The same tool implementations serve both stdio and HTTP transports. The transport layer is configured at startup.
- **Read-only by default.** The MCP server does not expose write operations. ztl's MCP server is a query interface over the vault's content and graph. Write operations remain in the HTTP API (SPEC-020) where authentication and ACL apply.

### 1.4 Scope

**In scope:**

- MCP server binary/mode exposing ztl's read capabilities as typed tools
- stdio transport (default, for subprocess-based MCP clients like Claude Desktop, Cursor, VS Code)
- HTTP+SSE transport (daemon mode, for persistent connections and remote deployment)
- Remote deployment: bind address configuration, bearer token authentication, healthcheck endpoint
- Authentication for HTTP transport via SPEC-020 agent tokens (reuse existing BIP39-derived credentials)
- Tools: `search`, `get`, `links`, `backlinks`, `path`, `similar`, `check`, `status`
- Feature-gated tool: `reason` (requires `--features reason`, SPEC-005)
- MCP resource exposure: vault page list as resource directory
- Server capability negotiation (tools, resources)
- Three deployment patterns: local stdio, local HTTP daemon, remote HTTP worker

**Out of scope (future):**

- Write tools (create/edit/delete pages via MCP)
- MCP prompts (predefined prompt templates)
- MCP sampling (server-initiated LLM calls)
- TLS termination (delegated to reverse proxy; guidance provided in deployment docs)
- Streaming/subscription for vault change notifications (future: MCP resource subscriptions)

---

## 2. User Profiles

### UP-021-001: Claude Desktop User

**Goals:** Ask Claude questions about their personal knowledge base without leaving the conversation. "What links to my Architecture Decision Records page?" "Find notes related to distributed systems." "What's the shortest path from Kafka to Event Sourcing in my vault?"

**Constraints:** Uses Claude Desktop with MCP support. Expects to configure ztl as an MCP server in `claude_desktop_config.json` and have it just work. Does not want to run `ztl serve` separately.

**Happy path:**
1. Adds ztl to Claude Desktop config with `"command": "ztl", "args": ["mcp", "--vault", "/path/to/vault"]`
2. Starts a conversation: "What are the backlinks to my 'Project Roadmap' page?"
3. Claude calls the `backlinks` tool → receives structured JSON → synthesizes a natural-language answer
4. User follows up: "Show me the shortest path from Project Roadmap to Sprint Planning"
5. Claude calls `path` tool → receives path → explains the connection chain

### UP-021-002: Cursor/IDE Agent

**Goals:** Use vault knowledge during coding sessions. "Find my notes about the authentication flow." "What does the API Design page say about rate limiting?"

**Constraints:** Runs as an MCP server in Cursor's tool configuration. Must start quickly (stdio). Must not block the IDE.

**Happy path:**
1. Configures ztl MCP in Cursor settings
2. During coding, asks: "Search my vault for authentication middleware patterns"
3. Cursor agent calls `search` → gets ranked results → presents relevant snippets
4. Agent calls `get` on the top result → retrieves full page content

### UP-021-003: Custom Agent Pipeline

**Goals:** Build an automated reasoning pipeline that queries the vault's graph structure. Run nightly analysis of vault health. Feed graph context into a RAG pipeline.

**Constraints:** Uses HTTP transport for persistent connections. Needs structured JSON responses. May run multiple concurrent queries.

**Happy path:**
1. Starts ztl MCP in HTTP mode: `ztl mcp --transport http --port 3100 --vault /path/to/vault`
2. Agent connects via HTTP+SSE
3. Agent calls `status` → gets vault stats and index health
4. Agent calls `check` → gets dead links and orphans
5. Agent calls `search` with semantic mode → gets conceptually related pages
6. Agent calls `links` and `backlinks` to map the neighbourhood around key pages

### UP-021-004: Remote Knowledge Worker

**Goals:** Serve a team's shared vault from a central machine (VPS, homelab, CI runner). Multiple agents and team members connect remotely, each authenticated via their own agent token. The vault stays on one machine; clients query it over the network.

**Constraints:** The vault may be large (10,000+ pages). Multiple concurrent clients from different machines. Must authenticate to prevent unauthorized access. Network latency is non-negligible. Cannot assume clients are on the same LAN. TLS is handled by a reverse proxy (nginx, Caddy, Cloudflare Tunnel).

**Happy path:**
1. Admin syncs the vault to the server (git pull, rsync, or NFS mount)
2. Admin runs `ztl index` to build/refresh the index
3. Admin starts: `ztl mcp --transport http --port 3100 --host 0.0.0.0 --auth-token-file /etc/ztl/tokens`
4. Caddy reverse proxy terminates TLS at `mcp.team.example.com` → `localhost:3100`
5. Remote agent connects with `Authorization: Bearer <agent-token>` header
6. Agent calls `search`, `links`, `backlinks` — same tools as local, over the network
7. Admin periodically runs `ztl index` (via cron or webhook) to pick up vault changes; agents call `status` with `reindex: true` if needed

**Failure modes:**
- Missing or invalid bearer token → 401 Unauthorized (clear error, not MCP tool error)
- Stale index → `status` tool shows `stale: true`; agent can trigger reindex
- Server unreachable → client-side timeout; retry with backoff

---

## 3. Requirements

### 3.1 MCP Server Lifecycle

#### REQ-110: MCP Server Command

The system SHALL provide a `ztl mcp` subcommand that starts an MCP server:

```
ztl mcp [--vault <path>] [--transport stdio|http] [--port <port>] [--host <addr>]
         [--allowed-issuer <DID>]... [--insecure] [--cors-origin <origin>]
```

| Flag                 | Default          | Description                                                     |
|----------------------|------------------|-----------------------------------------------------------------|
| `--vault`            | `.` (cwd)        | Path to the vault root                                          |
| `--transport`        | `stdio`          | Transport mode: `stdio` or `http`                               |
| `--port`             | `3100`           | HTTP transport listen port (ignored for stdio)                  |
| `--host`             | `127.0.0.1`      | HTTP transport bind address (ignored for stdio)                 |
| `--allowed-issuer`   | all vault users  | Restrict which SPEC-020 user DIDs may issue tokens (repeatable) |
| `--insecure`         | `false`          | Allow network-accessible HTTP without authentication            |
| `--cors-origin`      | —                | Allowed CORS origin for HTTP transport                          |

**Startup sequence:**

1. Locate vault root (same resolution as `ztl index`: walk upward from `--vault` to find `.ztl/`)
2. Load or build the vault index (reuse existing `.ztl/index.json` if fresh; re-index if stale or missing)
3. Load the link graph (`LinkGraph::build`)
4. Load the search index (Tantivy, if `search` feature enabled; vector index, if `vector` feature enabled)
5. Register MCP tools based on available features
6. Start the transport listener (stdio: read from stdin, write to stdout; HTTP: bind to host:port)
7. Respond to MCP `initialize` with server capabilities

The server SHALL hold the vault index, link graph, and search indexes in memory for the duration of the process. It SHALL NOT watch for file changes (unlike `ztl serve`). To pick up vault changes, the user restarts the MCP server or triggers reindex via the `status` tool's `reindex` parameter.

Trace: TEST-133, CON-033

#### REQ-111: Transport — stdio

The system SHALL support MCP communication over stdio (stdin/stdout) using JSON-RPC 2.0 as specified by the MCP protocol.

- All diagnostic/debug output SHALL go to stderr, never stdout (stdout is reserved for MCP protocol messages)
- The server SHALL handle `initialize`, `initialized`, `tools/list`, `tools/call`, `resources/list`, `resources/read`, and `ping` methods
- The server SHALL exit cleanly when stdin reaches EOF (client disconnected)
- The server SHALL set its process name to `ztl-mcp` for process manager visibility

Trace: TEST-134

#### REQ-112: Transport — HTTP+SSE

The system SHALL support MCP communication over HTTP with Server-Sent Events (SSE) for server-to-client messages, as specified by the MCP HTTP transport specification.

- `POST /mcp` — receives JSON-RPC requests from the client
- `GET /mcp/sse` — SSE stream for server-to-client notifications
- The server SHALL support multiple concurrent client connections
- The server SHALL respond with `Content-Type: application/json` for POST responses
- CORS headers SHALL be configurable via `--cors-origin <origin>` flag (default: none)

Trace: TEST-135

#### REQ-113: Capability Negotiation

On `initialize`, the server SHALL declare the following capabilities:

```json
{
  "capabilities": {
    "tools": {},
    "resources": {}
  },
  "serverInfo": {
    "name": "ztl-mcp",
    "version": "<ztl version>"
  }
}
```

The `tools` capability is always present. The `resources` capability is always present. Additional capabilities (prompts, sampling) are NOT declared — the server does not support them.

Trace: TEST-136

### 3.2 MCP Tools

#### REQ-114: Tool — `search`

The `search` tool SHALL perform full-text, semantic, or hybrid search over the vault.

**Tool definition:**

```json
{
  "name": "search",
  "description": "Search the vault for pages matching a query. Supports full-text (BM25), semantic (vector), and hybrid search modes.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "query": {
        "type": "string",
        "description": "The search query string"
      },
      "mode": {
        "type": "string",
        "enum": ["fulltext", "semantic", "hybrid"],
        "default": "fulltext",
        "description": "Search mode. 'fulltext' uses BM25 ranking, 'semantic' uses vector similarity, 'hybrid' combines both."
      },
      "limit": {
        "type": "integer",
        "default": 10,
        "minimum": 1,
        "maximum": 50,
        "description": "Maximum number of results to return"
      }
    },
    "required": ["query"]
  }
}
```

**Return value (MCP content array):**

```json
[
  {
    "type": "text",
    "text": "{ \"results\": [ { \"page\": \"Architecture Overview\", \"score\": 12.4, \"snippet\": \"The system uses an event-driven...\" }, ... ] }"
  }
]
```

The results JSON SHALL contain:
- `page`: the page name (title-cased, as stored in the index)
- `score`: numeric relevance score (BM25 score for fulltext, cosine similarity for semantic, weighted combination for hybrid)
- `snippet`: a text excerpt around the matching region (up to 200 characters), with the query terms highlighted using `**bold**` markers

**Feature availability:**
- `fulltext` mode requires the `search` feature (SPEC-013). If unavailable, return an error.
- `semantic` and `hybrid` modes require the `vector` feature (SPEC-018). If unavailable, return an error explaining the mode is not available.
- If no search features are compiled in, the `search` tool SHALL still be registered but SHALL return an error directing the user to rebuild with `--features search`.

Trace: TEST-137, CON-034

#### REQ-115: Tool — `get`

The `get` tool SHALL retrieve the full markdown content of a page.

**Tool definition:**

```json
{
  "name": "get",
  "description": "Retrieve the full markdown content of a page by name.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "page": {
        "type": "string",
        "description": "The page name (case-insensitive, matches against page titles)"
      }
    },
    "required": ["page"]
  }
}
```

**Return value:**

```json
[
  {
    "type": "text",
    "text": "# Architecture Overview\n\nThe system uses [[Event Sourcing]] for..."
  }
]
```

The content SHALL be the raw markdown as stored on disk. Page name resolution SHALL be case-insensitive and match against the resolved page title (same resolution as `ztl links`).

If the page does not exist, return `isError: true` with a message listing the closest matches (using the same fuzzy matching as the TUI page picker, up to 5 suggestions).

Trace: TEST-138, CON-034

#### REQ-116: Tool — `links`

The `links` tool SHALL return forward links from a page.

**Tool definition:**

```json
{
  "name": "links",
  "description": "List all pages that a given page links to (forward links / outgoing wikilinks).",
  "inputSchema": {
    "type": "object",
    "properties": {
      "page": {
        "type": "string",
        "description": "The source page name"
      }
    },
    "required": ["page"]
  }
}
```

**Return value:**

```json
[
  {
    "type": "text",
    "text": "{ \"page\": \"Architecture Overview\", \"links\": [ { \"target\": \"Event Sourcing\", \"line\": 12, \"exists\": true }, { \"target\": \"Missing Page\", \"line\": 25, \"exists\": false } ] }"
  }
]
```

Each link entry SHALL include:
- `target`: the linked page name
- `line`: the line number in the source file where the wikilink appears
- `exists`: whether the target page exists in the vault (true) or is a dead link (false)

Trace: TEST-139, CON-034

#### REQ-117: Tool — `backlinks`

The `backlinks` tool SHALL return pages that link to a given page.

**Tool definition:**

```json
{
  "name": "backlinks",
  "description": "List all pages that link to a given page (backlinks / incoming wikilinks).",
  "inputSchema": {
    "type": "object",
    "properties": {
      "page": {
        "type": "string",
        "description": "The target page name"
      }
    },
    "required": ["page"]
  }
}
```

**Return value:**

```json
[
  {
    "type": "text",
    "text": "{ \"page\": \"Event Sourcing\", \"backlinks\": [ { \"source\": \"Architecture Overview\", \"line\": 12 }, { \"source\": \"Design Patterns\", \"line\": 45 } ] }"
  }
]
```

Each backlink entry SHALL include:
- `source`: the page that contains the link
- `line`: the line number in the source file where the wikilink appears

Trace: TEST-139, CON-034

#### REQ-118: Tool — `path`

The `path` tool SHALL find the shortest wikilink path between two pages.

**Tool definition:**

```json
{
  "name": "path",
  "description": "Find the shortest path of wikilinks between two pages in the vault graph. Returns the chain of pages connecting the source to the target.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "from": {
        "type": "string",
        "description": "The starting page name"
      },
      "to": {
        "type": "string",
        "description": "The destination page name"
      },
      "max_depth": {
        "type": "integer",
        "default": 10,
        "minimum": 1,
        "maximum": 20,
        "description": "Maximum path length to search"
      }
    },
    "required": ["from", "to"]
  }
}
```

**Return value:**

```json
[
  {
    "type": "text",
    "text": "{ \"from\": \"Kafka\", \"to\": \"Event Sourcing\", \"path\": [\"Kafka\", \"Stream Processing\", \"CQRS\", \"Event Sourcing\"], \"length\": 3 }"
  }
]
```

- `path`: ordered array of page names from source to destination (inclusive)
- `length`: number of edges (hops), i.e., `path.len() - 1`

If no path exists within `max_depth`, return `isError: false` with a result indicating no path was found:

```json
{ "from": "Kafka", "to": "Unrelated Island", "path": null, "length": null, "message": "No path found within 10 hops" }
```

Trace: TEST-140, CON-034

#### REQ-119: Tool — `similar`

The `similar` tool SHALL find pages similar to a given page, using the page's content as the query vector for semantic search.

**Tool definition:**

```json
{
  "name": "similar",
  "description": "Find pages that are semantically similar to a given page, based on content vector similarity.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "page": {
        "type": "string",
        "description": "The reference page name"
      },
      "limit": {
        "type": "integer",
        "default": 10,
        "minimum": 1,
        "maximum": 50,
        "description": "Maximum number of similar pages to return"
      }
    },
    "required": ["page"]
  }
}
```

**Return value:**

```json
[
  {
    "type": "text",
    "text": "{ \"page\": \"Event Sourcing\", \"similar\": [ { \"page\": \"CQRS\", \"score\": 0.89 }, { \"page\": \"Domain Events\", \"score\": 0.82 } ] }"
  }
]
```

This tool requires the `vector` feature (SPEC-018). If unavailable, return `isError: true` with a message explaining the feature is not compiled in.

Trace: TEST-141, CON-034

#### REQ-114a: Tool — `check`

The `check` tool SHALL run vault diagnostics and return dead links, orphan pages, and other structural issues.

**Tool definition:**

```json
{
  "name": "check",
  "description": "Run vault diagnostics: find dead links (wikilinks pointing to non-existent pages), orphan pages (pages with no incoming links), and other structural issues.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "category": {
        "type": "string",
        "enum": ["all", "dead_links", "orphans"],
        "default": "all",
        "description": "Which diagnostic category to return"
      }
    }
  }
}
```

**Return value:**

```json
[
  {
    "type": "text",
    "text": "{ \"dead_links\": [ { \"source\": \"Foo\", \"target\": \"Missing\", \"line\": 12 } ], \"orphans\": [ \"Isolated Note\" ], \"stats\": { \"dead_link_count\": 1, \"orphan_count\": 1 } }"
  }
]
```

When `category` is `dead_links` or `orphans`, only the requested section and its count are included.

Trace: TEST-142, CON-034

#### REQ-114b: Tool — `status`

The `status` tool SHALL return vault metadata, index health, and MCP server information.

**Tool definition:**

```json
{
  "name": "status",
  "description": "Get vault metadata, index statistics, and MCP server health information. Optionally trigger a re-index to pick up vault changes.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "reindex": {
        "type": "boolean",
        "default": false,
        "description": "If true, re-scan the vault and rebuild the index before returning status. Use this after making changes to the vault."
      }
    }
  }
}
```

**Return value:**

```json
[
  {
    "type": "text",
    "text": "{ \"vault\": { \"root\": \"/path/to/vault\", \"page_count\": 412, \"link_count\": 1847, \"dead_link_count\": 3, \"orphan_count\": 7 }, \"index\": { \"vault_root_hash\": \"a3f8c9d1...\", \"indexed_at\": \"2026-04-07T14:30:00Z\", \"stale\": false }, \"server\": { \"transport\": \"stdio\", \"uptime_seconds\": 3600, \"tools\": [\"search\", \"get\", \"links\", \"backlinks\", \"path\", \"similar\", \"check\", \"status\"] } }"
  }
]
```

When `reindex: true`, the server SHALL:
1. Re-scan the vault filesystem
2. Rebuild the link graph
3. Rebuild the search index (if enabled)
4. Update the in-memory state
5. Return the updated status

The `stale` field indicates whether the on-disk vault has changed since the last index (computed by comparing the current `vault_root_hash` with the stored one). This is a lightweight stat-based check, not a full re-scan.

The `tools` array lists all registered tool names, enabling the agent to discover available capabilities.

Trace: TEST-136, CON-034

#### REQ-114c: Tool — `reason` (Feature-Gated)

The `reason` tool SHALL query the defeasible reasoning engine (SPEC-005) for conclusions derived from SPL blocks in the vault.

**Tool definition:**

```json
{
  "name": "reason",
  "description": "Query the defeasible reasoning engine for conclusions derived from SPL (Spindle Policy Language) blocks in the vault's markdown pages. Returns what can be concluded, with proof chains.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "query": {
        "type": "string",
        "description": "The SPL query expression, e.g., '(is-recommended ?tool)'"
      },
      "page": {
        "type": "string",
        "description": "Optional: restrict reasoning to SPL blocks from a specific page"
      }
    },
    "required": ["query"]
  }
}
```

**Return value:**

```json
[
  {
    "type": "text",
    "text": "{ \"query\": \"(is-recommended ?tool)\", \"conclusions\": [ { \"binding\": { \"?tool\": \"Rust\" }, \"tag\": \"+d\", \"rule\": \"r-recommend-systems-lang\", \"sources\": [\"Tech Decisions.md:15\"] } ] }"
  }
]
```

This tool requires the `reason` feature (SPEC-005). If unavailable, the tool SHALL NOT be registered in `tools/list` (not present in the tool catalog at all, rather than returning an error).

Trace: TEST-142, CON-034

### 3.3 MCP Resources

#### REQ-114d: Resource — Page Directory

The MCP server SHALL expose vault pages as an MCP resource directory, enabling clients to browse and read pages through the MCP resource protocol.

**`resources/list` response:**

```json
{
  "resources": [
    {
      "uri": "ztl://vault/pages/Architecture Overview",
      "name": "Architecture Overview",
      "mimeType": "text/markdown"
    },
    {
      "uri": "ztl://vault/pages/Event Sourcing",
      "name": "Event Sourcing",
      "mimeType": "text/markdown"
    }
  ]
}
```

**`resources/read` response:**

```json
{
  "contents": [
    {
      "uri": "ztl://vault/pages/Architecture Overview",
      "mimeType": "text/markdown",
      "text": "# Architecture Overview\n\n..."
    }
  ]
}
```

The URI scheme SHALL be `ztl://vault/pages/<page-name>`. Page names in URIs are URL-encoded.

Trace: TEST-136, CON-035

### 3.4 Remote Deployment & Delegated Authentication

#### REQ-114e: User-Signed JWT Authentication for HTTP Transport

The system SHALL authenticate HTTP transport connections using signed JWTs issued by SPEC-020 users via `ztl delegate`. Each token is a capability: a signed permission slip specifying which MCP tools and pages the bearer can access, optionally with an expiry.

**Design principle:** The common case must be one command with no arguments. Scoping and expiry are available when needed, not required when not.

**Token structure:**

A delegation token is a JWT signed by the user's ed25519 private key (derived from their BIP39 mnemonic, stored locally after SPEC-020 registration).

```json
{
  "alg": "EdDSA",
  "typ": "ztl-delegate+jwt"
}
.
{
  "iss": "did:key:z6MkuserPublicKey...",
  "iat": 1744070400,
  "tools": ["*"],
  "scope": "**",
  "exp": null
}
.
<ed25519 signature>
```

**Token fields:**

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `iss` | `did:key` | User's DID | SPEC-020 user identity that signed this token |
| `iat` | integer | Now | Issued-at timestamp (Unix seconds) |
| `tools` | string[] | `["*"]` | MCP tools granted. `*` = all tools. |
| `scope` | string | `**` | Page glob pattern. `**` = all pages. |
| `exp` | integer \| null | `null` | Expiry timestamp. `null` = no expiry. |

**Capability scoping:**

The `tools` array restricts which MCP tools the bearer can call:

| Value | Grants |
|-------|--------|
| `*` | All MCP tools (search, get, links, backlinks, path, similar, check, status, reason) |
| `search` | `search` tool only |
| `get` | `get` tool only |
| `search,get,links,backlinks` | Listed tools only |

The `scope` field restricts which pages are accessible, using vault-relative glob patterns:

| Value | Meaning |
|-------|---------|
| `**` | All pages (default) |
| `projects/**` | Pages under `projects/` |
| `projects/alpha/**` | Pages under `projects/alpha/` |
| `daily/*.md` | Direct children of `daily/` |

Scope applies to all page-addressing tools: `get` checks the requested page; `search` filters results; `links`/`backlinks` check source/target; `check` filters diagnostics. `status` is not scopeable (vault-wide metadata).

**CLI — `ztl delegate`:**

```bash
# Default: all tools, all pages, no expiry
# Uses stored identity key (~/.config/ztl/identity.key from SPEC-020 registration)
ztl delegate
# → eyJhbGciOiJFZERTQSIs...
# Paste this into your MCP client config.

# Scoped: specific tools and pages
ztl delegate --tools search,get --scope "projects/**"
# → eyJhbGciOiJFZERTQSIs...

# Time-limited: for a short-lived task
ztl delegate --tools search --expiry 1h
# → eyJhbGciOiJFZERTQSIs...

# Full options
ztl delegate \
  --tools search,get,links,backlinks \
  --scope "projects/**" \
  --expiry 7d
# → eyJhbGciOiJFZERTQSIs...

# Recovery path: use mnemonic directly (when identity.key is unavailable)
ztl delegate --mnemonic "word1 word2 ... word24"
# → eyJhbGciOiJFZERTQSIs...
```

`ztl delegate` reads the private key from `~/.config/ztl/identity.key` (stored during SPEC-020 registration via `ztl auth init` or collab bootstrap). The `--mnemonic` flag is a fallback for recovery or first-time setup on a new machine — it derives the key, signs the token, and optionally stores the key for future use (`--save-key`).

**Server-side verification:**

On each HTTP request to `/mcp` or `/mcp/sse`:

1. Extract `Authorization: Bearer <jwt>` header
2. Decode the JWT; verify `typ` is `ztl-delegate+jwt`
3. Verify the ed25519 signature against the `iss` public key
4. Check `exp` — if present, must not have passed; if `null`, token is valid indefinitely
5. Check `iss` is a registered SPEC-020 user in the vault
6. Attach `DelegateContext { tools, scope, issuer }` to the request
7. On each tool call: verify the tool name is in `tools` (or `tools` contains `*`), and the target page (if any) matches the `scope` glob
8. If any check fails: HTTP 401 with `{"error": "unauthorized", "reason": "<specific failure>"}`

**Behaviour:**

- When `--host` is non-loopback: JWT auth is enforced by default. The server checks that the issuer (`iss`) is a registered SPEC-020 user.
- When `--host` is loopback (`127.0.0.1` / `::1`): auth is not required (but JWTs are validated if present).
- stdio transport: auth is never applied.
- `GET /health` is always unauthenticated.
- `--allowed-issuer <DID>` restricts which user DIDs are accepted as token issuers (useful for revoking a user's delegation rights without removing their vault access).
- `--insecure` disables auth enforcement on non-loopback (explicit opt-in to unauthenticated remote access).
- `tools/list` returns only tools the token grants. A bearer with `tools: ["search", "get"]` sees only those two tools in the catalog.
- A tool call outside the token's scope returns MCP tool error `INSUFFICIENT_CAPABILITY` (not HTTP 401 — the token is valid, but the capability is insufficient).

**Token is a bearer token.** Anyone who possesses the JWT can use it. TLS (via reverse proxy) is the defense against interception. Tokens without expiry are valid until the issuer is removed from the vault or the `--allowed-issuer` list is narrowed.

Trace: TEST-143a, CON-034

#### REQ-114f: Healthcheck Endpoint

The HTTP transport SHALL expose a `GET /health` endpoint that returns HTTP 200 with a JSON body:

```json
{
  "status": "ok",
  "server": "ztl-mcp",
  "version": "<ztl version>",
  "vault_pages": 412,
  "index_stale": false,
  "uptime_seconds": 3600
}
```

- The endpoint does NOT require authentication (load balancers and monitoring probes need unauthenticated access).
- The endpoint SHALL respond within 10ms (no index queries, no graph traversal — reads only cached metadata).
- `index_stale` is the same lightweight stat-based check as `status` tool's `stale` field.
- HTTP status is always 200 unless the server is shutting down (503 Service Unavailable during graceful shutdown).

Trace: TEST-143b, CON-033

#### REQ-114g: Bind Address and Remote Deployment Configuration

The `--host` flag (REQ-110) SHALL accept:

| Value | Meaning |
|-------|---------|
| `127.0.0.1` (default) | Listen on loopback only — local clients only |
| `0.0.0.0` | Listen on all interfaces — required for remote access |
| Specific IP | Listen on that interface only |

When `--host` is set to anything other than `127.0.0.1` or `::1`, the server SHALL:

1. Print a security notice to stderr: `ztl-mcp: listening on <host>:<port> (network-accessible)`
2. Enforce JWT authentication by default. Requests without a valid signed delegation token are rejected with 401.
3. If `--insecure` is passed, disable auth enforcement and print a warning: `ztl-mcp: WARNING: network-accessible without authentication. This is unsafe for production.`
4. Without `--insecure` and without any registered SPEC-020 users in the vault: refuse to start with exit code 1 and message `"Error: no registered users in vault. Token authentication requires at least one SPEC-020 user. Use --insecure for unauthenticated access."`

This prevents accidental exposure of an unauthenticated MCP server to the network.

Trace: TEST-143c, CON-033

---

## 4. Architecture

### 4.1 Module Structure

```
src/mcp/
  mod.rs          — public exports, feature gate
  server.rs       — McpServer struct, tool registration, dispatch
  tools.rs        — tool handler implementations (one fn per tool)
  transport.rs    — stdio and HTTP+SSE transport adapters
  resources.rs    — MCP resource handlers (page directory)
  auth.rs         — JWT signature verification, capability checking, scope enforcement, auth middleware
```

### 4.2 Data Flow

```
MCP Client (Claude Desktop / Cursor / custom)
    │
    │  JSON-RPC 2.0 over stdio or HTTP+SSE
    ▼
┌─────────────────────────────┐
│  Transport Layer            │
│  (StdioTransport or        │
│   HttpTransport)            │
└─────────┬───────────────────┘
          │  parsed JsonRpc request
          ▼
┌─────────────────────────────┐
│  McpServer                  │
│  - initialize handler       │
│  - tools/list handler       │
│  - tools/call dispatcher    │
│  - resources/* handlers     │
└─────────┬───────────────────┘
          │  dispatches to tool fn
          ▼
┌─────────────────────────────┐
│  Tool Handlers              │
│  search_tool()              │  ──→ TantivyIndex / VectorIndex
│  get_tool()                 │  ──→ VaultFiles (filesystem read)
│  links_tool()               │  ──→ LinkGraph::forward_links()
│  backlinks_tool()           │  ──→ LinkGraph::backlinks()
│  path_tool()                │  ──→ LinkGraph::shortest_path()
│  similar_tool()             │  ──→ VectorIndex::similar()
│  check_tool()               │  ──→ LinkGraph::dead_links() + orphans()
│  status_tool()              │  ──→ VaultState + GraphStats
│  reason_tool()              │  ──→ spindle_core::reason()
└─────────────────────────────┘
```

### 4.3 Shared State

The `McpServer` holds shared state in an `Arc<McpState>`:

```rust
struct McpState {
    vault_root: PathBuf,
    graph: RwLock<LinkGraph>,
    index: RwLock<VaultIndex>,
    tantivy: Option<TantivySearcher>,      // if search feature
    vector: Option<VectorSearcher>,         // if vector feature
    file_index: RwLock<HashMap<String, PathBuf>>,  // page name → file path
    allowed_issuers: Option<Vec<DidKey>>,  // None = all registered users; Some = restricted list
    started_at: Instant,
}
```

### 4.5 Authentication Flow (HTTP Transport)

```
HTTP Request
    │
    ▼
┌──────────────────────────┐
│  /health ?               │──yes──→ bypass auth, respond 200
└─────────┬────────────────┘
          │ no
          ▼
┌──────────────────────────┐
│  Auth enforced?          │──no───→ pass through (loopback or --insecure)
│  (non-loopback, no       │
│   --insecure)            │
└─────────┬────────────────┘
          │ yes
          ▼
┌──────────────────────────┐
│  Authorization: Bearer   │──no───→ 401 { "reason": "missing bearer token" }
│  header present?         │
└─────────┬────────────────┘
          │ yes
          ▼
┌──────────────────────────┐
│  Decode JWT,             │──fail─→ 401 { "reason": "invalid token: <detail>" }
│  verify typ =            │
│  ztl-delegate+jwt,      │
│  verify ed25519 sig      │
└─────────┬────────────────┘
          │ ok
          ▼
┌──────────────────────────┐
│  exp: null or            │──fail─→ 401 { "reason": "token expired" }
│  not passed?             │
└─────────┬────────────────┘
          │ ok
          ▼
┌──────────────────────────┐
│  iss is registered       │──fail─→ 401 { "reason": "issuer not recognized" }
│  SPEC-020 user?          │
│  (+ --allowed-issuer     │
│   check if configured)   │
└─────────┬────────────────┘
          │ ok
          ▼
    Route to MCP server
    (DelegateContext attached to request — tool handlers
     check tools + scope on each tool call)
```

Authentication is implemented as an axum middleware layer applied to `/mcp` and `/mcp/sse` routes. The `/health` route is mounted outside the authenticated group.

The middleware attaches a `DelegateContext` to the request extensions containing the verified tool list and scope glob. Each tool handler checks against this context before executing:

```rust
struct DelegateContext {
    issuer: DidKey,
    tools: Vec<String>,  // ["*"] or ["search", "get", ...]
    scope: String,       // "**" or "projects/**" etc.
    expires_at: Option<u64>,
}

impl DelegateContext {
    /// Check if this token grants the given tool on the given page
    fn permits(&self, tool: &str, page: Option<&str>) -> bool;
}
```

A tool call outside the token's scope returns an MCP tool error (`isError: true`) with code `INSUFFICIENT_CAPABILITY`, not an HTTP 401. The 401 means "you have no valid token at all." A tool error with `INSUFFICIENT_CAPABILITY` means "your token is valid but doesn't grant this tool or page."

### 4.6 Deployment Patterns

Three deployment patterns are supported. The MCP server code is identical across all three — only startup flags differ.

#### Pattern 1: Local stdio (default)

```
┌──────────────────┐    stdin/stdout    ┌──────────────┐
│  Claude Desktop  │◄──────────────────►│  ztl mcp    │
│  / Cursor / IDE  │                    │  (subprocess) │
└──────────────────┘                    └──────────────┘
```

`ztl mcp --vault ~/notes`

No network. No auth. Process lifecycle managed by the MCP client.

#### Pattern 2: Local HTTP daemon

```
┌──────────────────┐    localhost:3100   ┌──────────────┐
│  Custom agent    │◄───────────────────►│  ztl mcp    │
│  / RAG pipeline  │                     │  --http      │
└──────────────────┘                     └──────────────┘
```

`ztl mcp --transport http --vault ~/notes`

Loopback only. No auth needed (same machine). Persistent process — models stay loaded, no startup cost per query.

#### Pattern 3: Remote HTTP worker

```
┌──────────────────┐         ┌───────────────┐         ┌──────────────┐
│  Remote agents   │──TLS──►│  Reverse proxy │──HTTP──►│  ztl mcp    │
│  (bearer JWT)    │         │  (Caddy/nginx) │         │  --http      │
└──────────────────┘         └───────────────┘         │  --host 0.0.0.0│
                                                        └──────────────┘
```

`ztl mcp --transport http --host 0.0.0.0 --port 3100`

Network-accessible. Signed JWT auth enforced automatically (non-loopback). Each agent presents a token issued by `ztl delegate` on the user's machine. TLS terminated at reverse proxy. Vault synced to the server (git pull, rsync, or shared filesystem). Index refreshed by cron or agent-triggered reindex.

**Recommended reverse proxy config (Caddy):**

```
mcp.team.example.com {
    reverse_proxy localhost:3100
}
```

Caddy auto-provisions TLS via Let's Encrypt. No ztl configuration needed beyond `--host 0.0.0.0`.

**Recommended vault sync (cron):**

```
*/5 * * * * cd /srv/vault && git pull --ff-only && ztl index --quiet
```

Agents call `status` to check if the index is stale; the `reindex: true` option provides on-demand refresh without cron.

The `RwLock` wrappers enable the `reindex` operation (REQ-114b) to swap in a new index and graph while concurrent tool calls continue reading the old state. Write locks are held only during the swap, not during the scan.

### 4.4 Integration with Existing Code

The MCP tool handlers SHALL call the same public functions used by the CLI and HTTP API:

| MCP Tool    | Underlying call                                    |
|-------------|-----------------------------------------------------|
| `search`    | `TantivySearcher::search()` / `VectorSearcher::search()` |
| `get`       | `std::fs::read_to_string(file_index[page])`        |
| `links`     | `LinkGraph::forward_links(page)`                   |
| `backlinks` | `LinkGraph::backlinks(page)`                       |
| `path`      | `LinkGraph::shortest_path(from, to, max_depth)`    |
| `similar`   | `VectorSearcher::similar(page, limit)`             |
| `check`     | `LinkGraph::dead_links()` + `LinkGraph::orphans()` |
| `status`    | `LinkGraph::stats()` + index metadata              |
| `reason`    | `spindle_core::reason()` + query                   |

No new data structures are introduced for graph queries. The MCP layer is a thin adapter that serializes existing return types to JSON for MCP content responses.

### 4.7 Token Provisioning — How Agents Obtain Access

The user issues a signed token by running `ztl delegate`. The agent does not register, negotiate, or request access — the user gives it a token. The token is a signed JWT: the server verifies the ed25519 signature and checks that the issuer is a known vault user. No server-side token storage.

#### Delegation Lifecycle

```
1. User identity stored     2. User runs delegate      3. Agent connects
   (one-time, SPEC-020)        (one command)              (every session)

┌────────────────────┐      ┌──────────────────┐      ┌─────────────────┐
│ ztl auth init     │      │ ztl delegate    │      │ MCP Client:     │
│   --mnemonic "..." │      │                  │      │ reads token     │
│                    │      │ Signs JWT with   │      │ from config,    │
│ → identity.key     │      │ stored key       │─────►│ sends as        │
│   saved to         │      │                  │      │ Authorization:  │
│   ~/.config/ztl/  │      │ → eyJhbGci...    │      │ Bearer <jwt>    │
└────────────────────┘      └──────────────────┘      └─────────────────┘
```

Step 1 happens once (during SPEC-020 registration or standalone `ztl auth init`). Step 2 is one command, no arguments needed. Step 3 is automatic — paste the JWT into the MCP client config once.

#### Issuing a Token

```bash
# Default: all tools, all pages, no expiry
ztl delegate
# → eyJhbGciOiJFZERTQSIs...
# Paste this into your MCP client config.

# Scoped: specific tools and pages
ztl delegate --tools search,get --scope "projects/**"
# → eyJhbGciOiJFZERTQSIs...

# Time-limited: for a short-lived task
ztl delegate --tools search --expiry 1h
# → eyJhbGciOiJFZERTQSIs...

# Recovery path: mnemonic directly (new machine, no stored key)
ztl delegate --mnemonic "word1 word2 ... word24" --save-key
# → eyJhbGciOiJFZERTQSIs...
# → Identity key saved to ~/.config/ztl/identity.key
```

`ztl delegate` reads `~/.config/ztl/identity.key` (stored during SPEC-020 registration). The `--mnemonic` flag is a fallback for recovery or first-time setup — it derives the key, signs the token, and optionally stores the key for future use (`--save-key`).

**Defaults:**

| Flag | Default | Rationale |
|------|---------|-----------|
| `--tools` | `*` (all) | MCP server is read-only; restricting tools is opt-in |
| `--scope` | `**` (all pages) | Your vault, your agent |
| `--expiry` | none | No forced renewal for personal use; use `--expiry` when delegating to others or for short tasks |

#### Client Configuration

**Claude Desktop** (`~/Library/Application Support/Claude/claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "ztl": {
      "url": "https://mcp.team.example.com/mcp",
      "headers": {
        "Authorization": "Bearer eyJhbGciOiJFZERTQSIs..."
      }
    }
  }
}
```

For local stdio (no auth needed):

```json
{
  "mcpServers": {
    "ztl": {
      "command": "ztl",
      "args": ["mcp", "--vault", "/path/to/vault"]
    }
  }
}
```

**Cursor** (`.cursor/mcp.json`):

```json
{
  "mcpServers": {
    "ztl": {
      "url": "https://mcp.team.example.com/mcp",
      "headers": {
        "Authorization": "Bearer eyJhbGciOiJFZERTQSIs..."
      }
    }
  }
}
```

**Environment variable** (CI, scripts):

```bash
export ztl_MCP_TOKEN="eyJhbGciOiJFZERTQSIs..."

curl -X POST "https://mcp.team.example.com/mcp" \
  -H "Authorization: Bearer $ztl_MCP_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
```

**Python MCP client:**

```python
from mcp import ClientSession, HttpTransport

transport = HttpTransport(
    url="https://mcp.team.example.com/mcp",
    headers={"Authorization": f"Bearer {os.environ['ztl_MCP_TOKEN']}"}
)
async with ClientSession(transport) as session:
    tools = await session.list_tools()  # only tools granted by token appear
```

Note: `tools/list` returns only the tools the token grants. A bearer with `--tools search,get` sees only those two tools — it never discovers tools it cannot use.

#### Revocation

Tokens are stateless — the server does not store issued tokens. Revocation strategies:

| Strategy | Mechanism | Latency |
|----------|-----------|---------|
| **Expiry** | Issue with `--expiry 1h` or `--expiry 7d`. Token self-revokes. | Up to expiry window |
| **Issuer revocation** | `--allowed-issuer` flag on the server. Remove a user's DID to invalidate all tokens they issued. | Requires server restart |
| **Re-key** | User runs `ztl auth rotate` to generate a new keypair. All tokens signed with the old key become invalid. | Immediate for new requests |

For tokens without expiry: the user is the root of trust. If the token leaks, `ztl auth rotate` invalidates it (and all other tokens from that identity). Re-run `ztl delegate` and update client configs.

#### Security Considerations

- **Tokens are bearer tokens.** Anyone who possesses the JWT can use it. Treat them like API keys: do not commit to version control, use environment variables or secrets managers.
- **TLS is required for remote.** Tokens sent over plaintext HTTP are visible to network observers. TLS is terminated at the reverse proxy (Caddy, nginx). ztl does not handle TLS itself.
- **Identity key is the master secret.** `~/.config/ztl/identity.key` can sign unlimited tokens. Protect it. If compromised, `ztl auth rotate` generates a new keypair and invalidates all existing tokens.
- **Clock skew.** Token expiry (when set) is checked against server time. The server SHOULD allow a 60-second grace period for clock skew.

---

## 5. Contract Specifications

### CON-033: `ztl mcp` CLI Interface

**Interface:** `ztl mcp [--vault <path>] [--transport stdio|http] [--port <port>] [--host <addr>] [--auth-token <TOKEN>] [--auth-token-file <PATH>] [--insecure] [--cors-origin <origin>]`

**Pre-conditions:**

- Vault directory exists and contains at least one `.md` file
- `.ztl/` directory exists (or will be created by initial indexing)
- If `--auth-token-file` is provided, the file exists and is readable

**Post-conditions:**

- For stdio: the server reads JSON-RPC from stdin and writes JSON-RPC to stdout. All non-protocol output goes to stderr. Auth flags are ignored.
- For HTTP: the server binds to `host:port` and accepts connections. A startup message is printed to stderr: `ztl-mcp listening on http://<host>:<port>`
- For HTTP with auth: `GET /health` responds without auth; all other endpoints require valid bearer token.
- The server responds to MCP `initialize` with the capability object specified in REQ-113
- The server continues running until stdin EOF (stdio) or SIGTERM/SIGINT (HTTP)

**Error model:**

- Vault not found: exit 1 with stderr message `"Error: no vault found at <path>. Run ztl index first."`
- Port already in use (HTTP): exit 1 with stderr message `"Error: port <port> already in use"`
- Invalid transport: exit 1 with stderr message `"Error: unknown transport '<value>'. Use 'stdio' or 'http'."`
- Network-accessible without auth (no `--insecure`, no registered users): exit 1 with stderr message `"Error: no registered users in vault. Token authentication requires at least one SPEC-020 user. Use --insecure for unauthenticated access."`

**Implements:** REQ-110, REQ-111, REQ-112, REQ-114e, REQ-114f, REQ-114g

**Verified by:** TEST-133, TEST-134, TEST-135, TEST-143a, TEST-143b, TEST-143c

### CON-034: MCP Tool Call Contract

**Interface:** MCP `tools/call` JSON-RPC method

**Pre-conditions:**

- Server is initialized (client has sent `initialize` and received response)
- The requested tool name is in the registered tool list

**Post-conditions (success):**

- Response contains `content` array with one `text` element
- The `text` value is a JSON string parseable as the tool's documented return schema
- `isError` is absent or `false`

**Post-conditions (tool error):**

- Response contains `content` array with one `text` element describing the error
- `isError` is `true`
- Error text is a JSON object: `{ "error": { "code": "<ERROR_CODE>", "message": "<human-readable>" } }`

**Error codes:**

| Code                  | Meaning                                           |
|-----------------------|---------------------------------------------------|
| `PAGE_NOT_FOUND`      | The requested page does not exist in the vault    |
| `FEATURE_NOT_AVAILABLE` | The requested feature is not compiled in        |
| `INVALID_QUERY`       | The SPL query expression is malformed (reason tool) |
| `NO_PATH`             | No path exists between the two pages (informational, not isError) |
| `INDEX_ERROR`         | Search index is corrupted or unavailable          |
| `INSUFFICIENT_CAPABILITY` | Token is valid but does not grant this tool or page scope |

**Implements:** REQ-114 through REQ-119, REQ-114a through REQ-114c

**Verified by:** TEST-137 through TEST-142

### CON-035: MCP Resource Contract

**Interface:** MCP `resources/list` and `resources/read` JSON-RPC methods

**Pre-conditions:**

- Server is initialized

**Post-conditions for `resources/list`:**

- Response contains `resources` array
- Each resource has `uri` (format: `ztl://vault/pages/<url-encoded-page-name>`), `name` (page title), and `mimeType` (`text/markdown`)
- The list includes all pages in the vault index

**Post-conditions for `resources/read`:**

- Response contains `contents` array with one element
- The element has `uri`, `mimeType`, and `text` (raw markdown content)
- If the URI does not match a known page: error response with `PAGE_NOT_FOUND`

**Implements:** REQ-114d

**Verified by:** TEST-136

### CON-036: MCP JSON-RPC Error Codes

The server SHALL use standard JSON-RPC 2.0 error codes for protocol-level errors:

| Code    | Meaning                          |
|---------|----------------------------------|
| -32700  | Parse error (malformed JSON)     |
| -32600  | Invalid request                  |
| -32601  | Method not found                 |
| -32602  | Invalid params                   |
| -32603  | Internal error                   |

Tool-level errors (page not found, feature unavailable) are NOT JSON-RPC errors. They are successful responses with `isError: true` in the tool result, as specified by the MCP protocol. This distinction is important: a JSON-RPC error means the protocol failed; a tool error means the tool executed but the operation could not be completed.

**Implements:** REQ-111, REQ-112

**Verified by:** TEST-134, TEST-135

---

## 6. Non-Functional Requirements

### NFR-044: MCP Server Startup Latency

The MCP server SHALL complete initialization and be ready to accept tool calls in ≤ 2 seconds FOR a vault of 2,000 pages WITH the index already built (`.ztl/index.json` exists and is fresh) WITH 95th percentile confidence.

This is critical for stdio transport where the MCP client (e.g., Claude Desktop) spawns the server as a subprocess and expects quick readiness. The 2-second budget covers: vault root resolution, index loading (deserialize JSON), graph building, search index opening, and MCP `initialize` response.

When the index is stale or missing, startup may take longer (up to the full re-index time). The server SHALL emit a progress notification to stderr: `ztl-mcp: re-indexing vault...` so the user understands the delay.

Trace: TEST-133

### NFR-045: Tool Call Latency

Individual tool calls SHALL complete within the following latency budgets FOR a vault of 2,000 pages WITH 95th percentile confidence:

| Tool        | Latency budget |
|-------------|---------------|
| `get`       | ≤ 10ms        |
| `links`     | ≤ 10ms        |
| `backlinks` | ≤ 10ms        |
| `path`      | ≤ 50ms        |
| `check`     | ≤ 50ms        |
| `status`    | ≤ 10ms        |
| `search`    | ≤ 100ms       |
| `similar`   | ≤ 200ms       |
| `reason`    | ≤ 500ms       |

These budgets cover the time from receiving the JSON-RPC request to sending the JSON-RPC response, excluding network latency. The graph-based tools (`get`, `links`, `backlinks`) are in-memory lookups and should be sub-millisecond in practice; the budget includes JSON serialization overhead.

Trace: TEST-137

### NFR-046: Memory Overhead

The MCP server process SHALL consume ≤ 1.5x the memory of the equivalent in-memory state (index + graph + search indexes). For a 2,000-page vault, this is approximately 50-100 MB total. The MCP protocol handling and transport layers SHALL add ≤ 10 MB overhead.

The server SHALL NOT load page file contents into memory at startup. Page content is read from disk on demand (the `get` tool reads the file each time). Only the index, graph, and search structures are held in memory.

Trace: TEST-133

---

## 7. Architecture Decisions

### ADR-057: rmcp as MCP SDK

**Decision:** Use the `rmcp` crate (Rust MCP SDK) for protocol handling, tool registration, and transport management.

**Context:** Implementing the MCP protocol from scratch requires handling JSON-RPC 2.0 framing, SSE streaming, capability negotiation, and the full MCP method set (`initialize`, `tools/list`, `tools/call`, `resources/*`, `ping`, etc.). This is substantial protocol plumbing that is orthogonal to ztl's value.

**Rationale:** `rmcp` provides:

1. **Type-safe tool registration** — tools are defined as Rust structs with derive macros; input schemas are generated from types
2. **Transport abstraction** — stdio and HTTP+SSE transports are built-in; the tool implementation is transport-agnostic
3. **Protocol compliance** — JSON-RPC 2.0 framing, capability negotiation, and error handling are handled by the SDK
4. **Ecosystem alignment** — rmcp is the de facto Rust MCP SDK, maintained by the MCP community

**Trade-offs:**

- (+) Eliminates ~2,000 lines of protocol boilerplate
- (+) Automatic schema generation from Rust types
- (+) Transport switching via configuration, not code changes
- (-) New dependency (~500KB compiled)
- (-) Pre-1.0 crate; API may change
- (-) Must adapt ztl's error model to MCP's `isError` convention

**Rejected alternatives:**

1. *Hand-rolled JSON-RPC over stdio* — feasible for stdio-only, but maintaining HTTP+SSE transport manually is error-prone and duplicates work that rmcp already handles
2. *Wrapping the HTTP API with an MCP proxy* — adds a network hop, requires `ztl serve` running separately, and loses the subprocess deployment model that makes MCP convenient
3. *TypeScript MCP SDK with Rust FFI* — introduces a Node.js dependency, which contradicts ztl's single-binary design

**Mitigation for API instability:** Pin to a specific `rmcp` version. The tool handler functions are pure Rust calling ztl's public API; if rmcp's registration API changes, only `server.rs` needs adaptation.

### ADR-058: Read-Only MCP Server

**Decision:** The MCP server exposes only read operations. No tool can create, edit, or delete vault pages.

**Context:** MCP tools are invoked by LLM agents that may hallucinate, misunderstand context, or act on stale information. Giving a tool-calling agent write access to a knowledge base without explicit user confirmation creates a risk of data loss or corruption.

**Rationale:**

1. **Safety by default.** A read-only server cannot cause data loss regardless of how the agent uses it. The worst case is wasted queries, not deleted pages.
2. **Separation of concerns.** Write operations require authentication, ACL checks, git attribution, and hook execution (SPEC-020). The MCP server does not handle any of these. Routing writes through `ztl serve` (which does handle them) is the correct architecture.
3. **MCP ecosystem convention.** Most MCP servers for knowledge bases are read-only (query, retrieve, search). Write operations are typically exposed through separate, explicitly-authorized tool sets.
4. **Future extensibility.** Write tools can be added later behind a `--allow-writes` flag with appropriate confirmation UX, without changing the read-only default.

**Trade-offs:**

- (+) No risk of agent-caused data loss
- (+) No need for authentication in the MCP layer
- (+) Simpler implementation (no write-after-read consistency concerns)
- (-) Agent cannot create or edit pages through MCP (must use HTTP API or CLI)
- (-) The `reindex` option in `status` is the one mutation; it only rebuilds the index, not vault content

### ADR-059: User-Signed JWT Delegation for Remote HTTP Transport

**Decision:** Authenticate remote HTTP connections using user-signed JWTs with capability claims (`tools`, `scope`, `exp`), issued via `ztl delegate` from the user's stored ed25519 identity key. No agent keypairs, no proof chains, no server-side token state.

**Context:** The MCP server's HTTP transport can be bound to `0.0.0.0`, making it network-accessible. We evaluated five authentication approaches:

1. Flat bearer tokens (API keys) — simple but all-or-nothing, no scoping
2. UCAN — full capability delegation with audience binding and proof chains
3. Macaroons — attenuation via caveats with server-side root key
4. User-signed JWT — signed capability claims, bearer token, stateless verification
5. OAuth2/OIDC — authorization server with token endpoints

**Why user-signed JWTs over UCAN:** UCAN's audience binding (`aud` field) requires the bearer to prove they hold the audience's private key. MCP clients send a static `Authorization: Bearer` header — there is no request signing or challenge-response. In the MCP context, UCAN is a bearer token with extra metadata that cannot be enforced. The proof chain machinery adds complexity without security benefit when audience binding is unenforceable.

A simple JWT signed by the user's ed25519 key gives the same practical security: signed claims (tools, scope, expiry), stateless verification, user-driven delegation. It's simpler to implement, simpler to reason about, and honest about its security model — it's a bearer token with signed, unforgeable capability claims.

**Rationale:**

1. **One command, no arguments.** `ztl delegate` with sensible defaults (all tools, all pages, no expiry). The user's identity key is already stored from SPEC-020 registration. No mnemonics to type, no DIDs to copy, no agent keypairs to generate.
2. **User-driven delegation.** The user — not an admin — decides what their agent can do. Scoping (`--tools`, `--scope`) and expiry (`--expiry`) are available when needed, invisible when not.
3. **Stateless verification.** The server checks the ed25519 signature and looks up the `iss` in the SPEC-020 user registry. No token table, no session store, no revocation list.
4. **Identity reuse.** SPEC-020 `did:key` identities are JWT issuers. The same key that authenticates to `ztl serve --collab` signs delegation tokens. One identity, one key.
5. **Per-tool, per-page scoping.** The `tools` and `scope` claims are enforced on every tool call. An agent with `tools: ["search"]` cannot call `get`, even if it discovers the tool name.
6. **Honest security model.** The token is a bearer token. We don't pretend otherwise. TLS (via reverse proxy) protects it in transit. Key rotation (`ztl auth rotate`) revokes all tokens instantly.
7. **No expiry by default.** For your own agent accessing your own read-only vault, forced expiry is just chores. Expiry is opt-in for short-lived tasks or delegation to others.

**Trade-offs:**

- (+) Simplest possible UX: `ztl delegate` → paste into config → done
- (+) No agent keypairs, no DIDs to manage, no proof chains
- (+) Stateless verification — no server-side token storage
- (+) Ed25519 keys already exist (SPEC-020)
- (+) Honest bearer token model — no unenforceable audience binding
- (+) Healthcheck endpoint remains unauthenticated
- (-) No delegation chains (agent cannot further attenuate to sub-agent). Acceptable for v1; if needed, UCAN can be adopted later as the JWT is a compatible subset.
- (-) No audience binding (anyone with the token can use it). Mitigated by TLS and key rotation.
- (-) Revocation requires key rotation (invalidates ALL tokens) or `--allowed-issuer` (invalidates all tokens from one user). No per-token revocation.
- (-) TLS not handled by ztl — relies on reverse proxy.

**Rejected alternatives:**

1. *UCAN* — full capability delegation with proof chains. Audience binding is unenforceable in MCP's bearer-token transport. Proof chains add complexity without security benefit in this context. Agent keypairs add UX friction. If delegation chains are needed in the future, UCAN can be adopted as a backward-compatible extension (the current JWT is a strict subset of UCAN without `aud` and `prf`).
2. *Flat bearer tokens (API keys)* — no scoping, no user-driven delegation, no cryptographic provenance. Would need a second system for capability restrictions.
3. *Macaroons* — attenuation via caveats, but require a shared server secret (root key). Couples token issuance to the server, preventing offline delegation.
4. *OAuth2 / OIDC* — authorization server, redirect flows, token endpoints. Massive deployment overhead for a CLI tool.
5. *SPL-scoped tokens* — requires SPL evaluation on every request. Better as a future extension than a foundation.
6. *No auth, rely on network isolation* — preserved as `--insecure` for explicit opt-in.

---

## 8. Test Specifications

### TEST-133: MCP Server Startup — stdio

**Requirement:** REQ-110, NFR-044

**Preconditions:** Vault with 50 `.md` files; `.ztl/index.json` exists and is fresh

**Steps:**

1. Start `ztl mcp --vault <path>` as a subprocess
2. Send MCP `initialize` request via stdin
3. Verify response contains `serverInfo.name` = `"ztl-mcp"` and `capabilities.tools` object
4. Verify no output appeared on stdout before the `initialize` response (no startup banners on stdout)
5. Measure time from process start to `initialize` response; verify ≤ 2 seconds
6. Send `ping` request; verify `pong` response
7. Close stdin; verify process exits cleanly within 1 second

### TEST-134: stdio Transport — Protocol Compliance

**Requirement:** REQ-111

**Preconditions:** MCP server running via stdio

**Steps:**

1. Send malformed JSON → verify JSON-RPC error -32700
2. Send valid JSON-RPC with unknown method `"foo/bar"` → verify error -32601
3. Send `tools/call` with missing required parameter → verify error -32602
4. Send `tools/list` → verify response contains all registered tool names with `inputSchema` for each
5. Send `tools/call` for each registered tool with valid parameters → verify each returns a valid MCP content response
6. Verify all responses are valid JSON-RPC 2.0 (have `jsonrpc: "2.0"`, `id` matching request)

### TEST-135: HTTP Transport — Startup and SSE

**Requirement:** REQ-112

**Preconditions:** Port 3100 is available

**Steps:**

1. Start `ztl mcp --transport http --port 3100 --vault <path>`
2. Verify stderr contains `ztl-mcp listening on http://127.0.0.1:3100`
3. POST `http://127.0.0.1:3100/mcp` with MCP `initialize` → verify 200 with capabilities
4. GET `http://127.0.0.1:3100/mcp/sse` → verify SSE stream opens (Content-Type: text/event-stream)
5. POST `tools/list` → verify tool catalog returned
6. POST `tools/call` with `search` tool → verify result
7. Send SIGTERM → verify clean shutdown within 2 seconds

### TEST-136: Capability Negotiation and Resource Listing

**Requirement:** REQ-113, REQ-114d

**Preconditions:** Vault with pages "Foo", "Bar", "Baz"

**Steps:**

1. Send `initialize` → verify `capabilities.tools` and `capabilities.resources` are present
2. Send `resources/list` → verify 3 resources returned with `ztl://vault/pages/` URIs
3. Send `resources/read` with URI `ztl://vault/pages/Foo` → verify markdown content of Foo returned
4. Send `resources/read` with URI `ztl://vault/pages/Nonexistent` → verify error response

### TEST-137: Tool — search

**Requirement:** REQ-114

**Preconditions:** Vault with 20 pages; Tantivy index built; page "Distributed Systems" contains "consensus algorithm"

**Steps:**

1. Call `search` with `{ "query": "consensus algorithm" }` → verify results include "Distributed Systems"
2. Verify each result has `page`, `score`, `snippet` fields
3. Verify `score` is a positive number
4. Verify `snippet` contains text from the matching page
5. Call `search` with `{ "query": "consensus", "limit": 3 }` → verify ≤ 3 results
6. Call `search` with `{ "query": "consensus", "mode": "semantic" }` → if vector feature enabled, verify results; if not, verify `isError: true` with `FEATURE_NOT_AVAILABLE`
7. Call `search` with `{ "query": "" }` → verify `isError: true` with `INVALID_QUERY`

### TEST-138: Tool — get

**Requirement:** REQ-115

**Preconditions:** Vault with page "Architecture Overview" containing wikilinks

**Steps:**

1. Call `get` with `{ "page": "Architecture Overview" }` → verify raw markdown content returned
2. Call `get` with `{ "page": "architecture overview" }` (lowercase) → verify same content (case-insensitive)
3. Call `get` with `{ "page": "Nonexistent Page" }` → verify `isError: true` with `PAGE_NOT_FOUND`
4. Verify the error message includes fuzzy-match suggestions

### TEST-139: Tools — links and backlinks

**Requirement:** REQ-116, REQ-117

**Preconditions:** Vault where page "A" links to "B" and "C"; page "D" links to "B"

**Steps:**

1. Call `links` with `{ "page": "A" }` → verify targets include "B" and "C" with line numbers and `exists` flags
2. Call `backlinks` with `{ "page": "B" }` → verify sources include "A" and "D" with line numbers
3. Call `links` with `{ "page": "Nonexistent" }` → verify `isError: true` with `PAGE_NOT_FOUND`
4. Call `backlinks` with `{ "page": "Orphan" }` (page with no backlinks) → verify empty `backlinks` array (not an error)
5. Verify that dead links in `links` output have `"exists": false`

### TEST-140: Tool — path

**Requirement:** REQ-118

**Preconditions:** Vault with chain A → B → C → D; page "Island" has no connections

**Steps:**

1. Call `path` with `{ "from": "A", "to": "D" }` → verify `path` = `["A", "B", "C", "D"]`, `length` = 3
2. Call `path` with `{ "from": "A", "to": "A" }` → verify `path` = `["A"]`, `length` = 0
3. Call `path` with `{ "from": "A", "to": "Island" }` → verify `path` = null, message indicates no path found
4. Call `path` with `{ "from": "A", "to": "D", "max_depth": 2 }` → verify `path` = null (path is 3 hops, exceeds max_depth of 2)
5. Call `path` with `{ "from": "Nonexistent", "to": "A" }` → verify `isError: true` with `PAGE_NOT_FOUND`

### TEST-141: Tool — similar

**Requirement:** REQ-119

**Preconditions:** Vault with vector index built; pages with related content

**Steps:**

1. Call `similar` with `{ "page": "Event Sourcing" }` → verify results are semantically related pages
2. Verify each result has `page` and `score` fields
3. Verify the reference page itself is NOT in the results
4. Call `similar` with `{ "page": "Event Sourcing", "limit": 3 }` → verify ≤ 3 results
5. Call `similar` with `{ "page": "Nonexistent" }` → verify `isError: true` with `PAGE_NOT_FOUND`
6. If vector feature not compiled in: call `similar` → verify `isError: true` with `FEATURE_NOT_AVAILABLE`

### TEST-142: Tools — check and reason

**Requirement:** REQ-114a, REQ-114c

**Preconditions:** Vault with 1 dead link (A → Missing), 1 orphan (Isolated); SPL blocks defining reasoning rules

**Steps:**

1. Call `check` with `{}` → verify `dead_links` contains the A → Missing entry, `orphans` contains "Isolated"
2. Call `check` with `{ "category": "dead_links" }` → verify only `dead_links` in response
3. Call `check` with `{ "category": "orphans" }` → verify only `orphans` in response
4. If reason feature enabled: call `reason` with a valid query → verify conclusions with bindings and proof sources
5. If reason feature enabled: call `reason` with malformed SPL → verify `isError: true` with `INVALID_QUERY`
6. If reason feature not enabled: send `tools/list` → verify `reason` is NOT in the tool list

### TEST-143a: JWT Authentication and Capability Enforcement

**Requirement:** REQ-114e

**Preconditions:** Vault with 10 pages (including pages "Alpha" in `projects/alpha/` and "Beta" in `notes/`); SPEC-020 user registered (Alice, `did:key:z6MkAlice...`)

**Setup:** Generate three tokens via `ztl delegate`:
- `jwt_full`: tools=`*`, scope=`**` (no expiry)
- `jwt_scoped`: tools=`search,get`, scope=`projects/**` (no expiry)
- `jwt_expired`: tools=`*`, scope=`**`, expiry=-1h (already expired)

**Steps:**

1. Start `ztl mcp --transport http --host 0.0.0.0 --port 3100 --vault <path>`
2. POST `/mcp` with `Authorization: Bearer <jwt_full>` and `initialize` → verify 200 with capabilities listing all tools
3. POST `/mcp` with `Authorization: Bearer <jwt_scoped>` and `initialize` → verify 200 with capabilities listing only `search` and `get`
4. Using `jwt_scoped`: call `get` with `{ "page": "Alpha" }` → verify 200 with page content (page is within `projects/**` scope)
5. Using `jwt_scoped`: call `get` with `{ "page": "Beta" }` → verify `isError: true` with `INSUFFICIENT_CAPABILITY` (page outside scope)
6. Using `jwt_scoped`: call `links` with `{ "page": "Alpha" }` → verify `isError: true` with `INSUFFICIENT_CAPABILITY` (tool not granted)
7. POST `/mcp` with `Authorization: Bearer <jwt_expired>` → verify 401 with `"reason": "token expired"`
8. POST `/mcp` with no `Authorization` header → verify 401
9. POST `/mcp` with `Authorization: Bearer not-a-jwt` → verify 401 with `"reason": "invalid token: ..."`
10. GET `/health` with no auth → verify 200 (bypasses auth)

### TEST-143a-revoke: Token Revocation via Key Rotation

**Requirement:** REQ-114e

**Preconditions:** SPEC-020 user Alice registered; MCP server running with JWT auth enforced

**Setup:**
- Generate `jwt_valid` via `ztl delegate` (signed with Alice's current key)

**Steps:**

1. Using `jwt_valid`: call `search` with `{ "query": "test" }` → verify 200 (token works)
2. Run `ztl auth rotate` to generate a new keypair for Alice
3. Using `jwt_valid`: call `search` → verify 401 with `"reason": "issuer not recognized"` (old key no longer matches)
4. Generate `jwt_new` via `ztl delegate` (signed with new key)
5. Using `jwt_new`: call `search` → verify 200 (new token works)

### TEST-143b: Healthcheck Endpoint

**Requirement:** REQ-114f

**Preconditions:** MCP server running in HTTP mode with a 50-page vault

**Steps:**

1. GET `/health` → verify 200 with JSON body
2. Verify body contains `status: "ok"`, `server: "ztl-mcp"`, `version` (non-empty string), `vault_pages` (integer ≥ 50), `index_stale` (boolean), `uptime_seconds` (integer ≥ 0)
3. Measure response latency; verify ≤ 10ms
4. Modify a vault file on disk (touch a .md file)
5. GET `/health` → verify `index_stale` is now `true`
6. Trigger reindex via MCP `status` tool with `reindex: true`
7. GET `/health` → verify `index_stale` is now `false`

### TEST-143c: Network Bind Safety

**Requirement:** REQ-114g

**Preconditions:** Vault with no registered SPEC-020 users (fresh vault, no collab bootstrap)

**Steps:**

1. Start `ztl mcp --transport http --host 0.0.0.0 --vault <path>` (no users, no --insecure) → verify exit code 1
2. Verify stderr contains "no registered users" error message
3. Start `ztl mcp --transport http --host 0.0.0.0 --insecure --vault <path>` → verify server starts
4. Verify stderr contains "WARNING: network-accessible without authentication"
5. Register a SPEC-020 user (bootstrap); start `ztl mcp --transport http --host 0.0.0.0 --vault <path>` → verify server starts (JWT auth enforced, users exist)
6. Verify stderr contains "listening on 0.0.0.0:3100 (network-accessible)" (informational, not warning)
7. Start `ztl mcp --transport http --host 127.0.0.1 --vault <path>` (loopback, no users) → verify server starts without error (loopback is safe)

---

## 9. Purity Boundary Map

### Pure Core (no I/O, no shared state, deterministic)

- **JWT decoding**: `decode_delegate_jwt(jwt: &str) → Result<DelegateClaims>` — parse JWT, extract header/payload/signature without verification (pure deserialization)
- **JWT signature verification**: `verify_delegate_signature(jwt: &str, issuer_pubkey: &[u8; 32]) → Result<DelegateClaims>` — verify ed25519 signature against `iss` public key (pure crypto, no I/O)
- **Capability checking**: `check_capability(claims: &DelegateClaims, tool: &str, page: Option<&str>) -> bool` — check if the token's `tools` list includes the requested tool and the `scope` glob matches the page (pure glob matching)
- **Expiry checking**: `is_expired(claims: &DelegateClaims, now: u64, grace_seconds: u64) -> bool` — check `exp` field with clock skew grace (pure)
- **Tool input validation**: `validate_search_params(params) → Result<SearchParams>` — validates and normalizes search tool parameters
- **Tool input validation**: `validate_page_param(page: &str, index: &VaultIndex) → Result<String>` — case-insensitive page name resolution with fuzzy-match suggestions on failure
- **Result serialization**: `serialize_links_result(page: &str, links: Vec<ForwardLinkResult>) → Value` — converts LinkGraph output to MCP-compatible JSON
- **Result serialization**: `serialize_backlinks_result(page: &str, backlinks: Vec<BacklinkResult>) → Value` — converts backlinks to JSON
- **Result serialization**: `serialize_path_result(from: &str, to: &str, path: Option<PathResult>) → Value` — converts shortest-path output to JSON
- **Result serialization**: `serialize_check_result(dead_links: Vec<DeadLink>, orphans: Vec<Orphan>, category: &str) → Value` — converts diagnostics to JSON
- **Result serialization**: `serialize_search_result(results: Vec<SearchHit>) → Value` — converts search hits to JSON with snippet extraction
- **Result serialization**: `serialize_status(state: &VaultSnapshot, tools: &[&str], transport: &str, uptime: Duration) → Value` — builds status JSON
- **Error formatting**: `format_tool_error(code: &str, message: &str) → Value` — constructs MCP tool error JSON
- **Page name fuzzy matching**: `suggest_pages(query: &str, pages: &[String], limit: usize) → Vec<String>` — returns closest page name matches for error messages

### Effectful Shell (orchestrates I/O, calls pure core)

- **`McpServer`**: Holds `Arc<McpState>`, registers tools with rmcp, dispatches `tools/call` to handler functions
- **`StdioTransport`**: Reads JSON-RPC from stdin, writes to stdout; delegates to `McpServer`
- **`HttpTransport`**: Binds HTTP server, manages SSE connections; delegates to `McpServer`
- **`AuthMiddleware`**: Axum middleware layer; extracts `Authorization: Bearer <jwt>` header, calls pure `decode_delegate_jwt()` + `verify_delegate_signature()`, checks `iss` against registered SPEC-020 users, attaches `DelegateContext` to request or rejects with 401
- **`HealthEndpoint`**: Reads cached vault metadata from `McpState`, serializes `/health` response (mounted outside auth middleware group)
- **Tool handlers**: `search_tool()`, `get_tool()`, `links_tool()`, etc. — each reads from `McpState`, calls pure core for validation/serialization, performs I/O (file reads, search queries), and returns MCP content
- **Reindex handler**: Triggered by `status` tool with `reindex: true`; re-scans vault, rebuilds graph and indexes, swaps into `McpState` under write lock
- **Resource handlers**: `list_resources()`, `read_resource()` — enumerate pages from index, read files from disk

### Boundary Contracts (data types crossing the boundary)

- `SearchParams` (shell → core): validated search parameters (query, mode, limit)
- `VaultSnapshot` (shell → core): snapshot of vault metadata for status serialization (page count, link count, hash, timestamp)
- `serde_json::Value` (core → shell): serialized JSON tool results ready for MCP content wrapping
- `ToolError { code: String, message: String }` (core → shell): structured error for MCP `isError` responses

### Dependency Rule

Dependencies point inward: shell → core. The pure core module (`src/mcp/tools.rs` serialization functions) SHALL NOT import `rmcp`, `tokio`, `std::fs`, or any I/O crate. It operates on ztl's existing data types (`ForwardLinkResult`, `BacklinkResult`, `PathResult`, `DeadLink`, `Orphan`, `GraphStats`) and produces `serde_json::Value`.

### Enforcement

- Module structure: `src/mcp/mod.rs` (feature gate + exports), `src/mcp/server.rs` (shell), `src/mcp/tools.rs` (mixed: pure serialization + effectful handlers, separated by `// --- pure core ---` and `// --- effectful shell ---` sections), `src/mcp/transport.rs` (shell), `src/mcp/resources.rs` (shell), `src/mcp/auth.rs` (mixed: pure JWT validation/capability checking + effectful middleware)
- The serialization functions in `tools.rs` SHALL NOT depend on `rmcp`, `tokio`, or `std::fs`
- CI check: the `mcp` feature compiles cleanly with `cargo check --features mcp`
