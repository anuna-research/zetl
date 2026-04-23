---
title: Web Server
tags: [reading, web, serve]
---

# Web Server

`ztl serve` starts a local web server that renders your vault as a browsable site. You get rendered Markdown, working wikilinks, a live backlink panel, a persistent graph mini-map, and an in-browser editor. It's the fastest way to actually read — and edit — a vault of more than a handful of notes.

## Why run a server

A static export ([[Static Site Export]]) shows your vault as it was when you last built it. A terminal viewer ([[Terminal Viewer]]) shows one note at a time. `ztl serve` is what you want between those: a live view of the vault *right now*, with:

- **Rendered pages** — Markdown, wikilinks, embeds, and code blocks, rendered through Minijinja templates with YAML frontmatter in scope.
- **Backlinks panel** — every page that points here, updated on every save.
- **Transclusion panel** — forward-link excerpts as cards in the right rail, connected to anchors in the main text by SVG bridges (the same Xanadu idea as the terminal viewer).
- **Live search** — full-text search across the vault from a modal.
- **CodeMirror editor** — click Edit, change the note, hit Save. The index rebuilds on save; no reload dance. A delete button is a click away.
- **Persistent graph mini-map** — a Sigma.js WebGL graph of your vault, docked bottom-right, that survives page navigation (no flash, no re-layout).
- **History strip** — when ztl was built with `--features history`, every page shows when it last changed and links to its timeline. See [[Page History]].

## Running it

```bash
ztl serve                            # http://localhost:3000
ztl -d ~/notes serve
ztl serve --port 8080
ztl serve --theme paper
```

Open `http://localhost:3000/` to land on the vault index. Wikilinks in rendered pages are clickable. `Ctrl-P` (or whatever the active theme binds) opens the search modal.

## Flags that matter

| Flag | Default | Purpose |
|------|---------|---------|
| `-p, --port <PORT>` | `3000` | TCP port. |
| `--theme <THEME>` | `default` | Theme from `.ztl/themes/<name>/`. See [[Customising the Look]]. |
| `--public <DIR>` | — | Files in this directory override generated pages (good for `robots.txt`, custom error pages). |
| `--collab` | off | Switches to multi-user mode with passkeys and CRDT sync. See [[Running a Team Server]]. |
| `--safe-mode` | off | Skip every hook except ones declared in the theme. |
| `--at <TIME-EXPR>` | — | Serve a past snapshot. See [[Time Travel]]. |
| `--exclude <PATTERN>` | — | Gitignore-syntax exclusion, repeatable. |

## Single-user vs collab

Out of the box, `ztl serve` is single-user and trusts whoever reaches the port. There's no login, no accounts; editing works immediately. This is the right mode when the server is on your laptop or behind a VPN.

With `--collab`, ztl turns into a multi-user server: WebAuthn passkey login, real-time CRDT editing over WebSockets, access control, invitations. That whole stack — bootstrapping an owner, inviting collaborators, deterministic server keys — lives in [[Running a Team Server]].

## API endpoints

`serve` exposes a small JSON API on the same port. A brief sample:

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/api/pages` | GET | List every page |
| `/api/pages/{slug}` | GET/PUT/DELETE | Read, update, remove |
| `/api/search?q=...` | GET | Full-text search |
| `/api/graph` | GET | Full link graph JSON |
| `/api/index` | POST | Trigger a reindex |

See [[CLI Overview]] for the full endpoint list including the collab-only WebSocket and access-request endpoints.

## Related

- [[Static Site Export]]
- [[Terminal Viewer]]
- [[Customising the Look]]
- [[Running a Team Server]]
- [[CLI Overview]]
