# Serve Command

`ztl serve` starts a local web server for browsing the vault in a browser.

## Usage

```bash
ztl -d ./my-vault serve              # http://localhost:3000
ztl -d ./my-vault serve --port 8080
```

## Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--port N` | 3000 | Port to listen on |
| `--theme NAME` | default | Theme name (looks in `.ztl/themes/<name>/`) |
| `--collab` | off | Enable multi-user collaborative editing mode |
| `--init-owner` | | Bootstrap the vault owner (first-time setup, requires `--collab`) |
| `--owner-name NAME` | Owner | Display name for the vault owner (used with `--init-owner`) |
| `--git-poll-interval DURATION` | 30s | Git HEAD poll interval for external commit detection. Set to "0" to disable |

## Features

The web UI provides:

- **Rendered pages** — Markdown rendered to HTML with syntax highlighting
- **Sidebar** — page list with search/filter
- **Backlink list** — pages linking to the current page
- **Transclusion panel** — forward-link excerpt cards with SVG bridge connectors, mirroring the [[View Command]] design
- **Inline editing** — CodeMirror 6 editor with save-and-reindex, page deletion with slug confirmation
- **Real-time collaboration** — CRDT-based co-editing via WebSocket with cursor presence (requires `--collab`)
- **Authentication** — WebAuthn passkey login, BIP39 mnemonic recovery, invitation links (requires `--collab`)
- **Access control** — role-based (reader/editor/admin) with optional SPL-based ACL policies (requires `--features reason`)
- **Page history** — git log, diff view, and restore for each page
- **Comments** — per-page comments with HMAC integrity
- **Admin panel** — invitation management at `/_admin/invite`, permission management at `/_admin/permissions`

## Collaboration mode

```bash
# First-time setup
ztl -d ./my-vault serve --collab --init-owner --owner-name Alice

# Normal start
ztl -d ./my-vault serve --collab
```

In collab mode, all content routes require authentication. Unauthenticated browsers are redirected to the bootstrap/login page. API clients receive 401.

### Onboarding flow

1. Owner runs `--init-owner` — saves 12-word recovery phrase from terminal
2. Owner opens `http://localhost:3000` — redirected to passkey registration
3. Owner creates invitations via `/_admin/invite` or `ztl invite` CLI
4. Invitees open the invitation link — choose a name, save their recovery phrase, register a passkey

### Account recovery

Visit `/auth/recovery`, enter your display name and 12-word phrase. On success, you get a new session and can re-register a passkey.

## Relationship to other viewers

| Viewer | Interface | Read-only? |
|--------|-----------|------------|
| `ztl tui` | Terminal (multi-view dashboard) | Yes |
| `ztl view` | Terminal (two-pane reader) | Yes |
| `ztl serve` | Browser (full web UI) | No (inline editing) |
| `ztl build` | Static HTML (deployable) | Yes |

## History API

When built with `--features history`, the serve mode exposes additional API endpoints:

| Endpoint | Description |
|----------|-------------|
| `GET /api/history` | Graph-level delta log |
| `GET /api/history/page/:name` | Page evolution timeline |
| `GET /api/history/at?expr=<time>` | Resolve a time expression to snapshot metadata |
| `GET /api/history/diff?from=<expr>&to=<expr>` | Diff between two time expressions |

See [[History Command]] for details.

## Design

The server uses the same [[Link Graph]] and [[architecture/Cache]] as all other commands. The transclusion panel implements the same Xanadu-inspired design as [[View Command]], but with SVG bridge connectors instead of terminal graphics. See [[Xanadu Lineage]] for the design philosophy.

See also: [[CLI Reference]], [[Build Command]], [[View Command]], [[TUI]], [[History Command]]
