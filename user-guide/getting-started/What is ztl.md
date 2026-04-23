---
title: What is ztl
tags: [overview, getting-started]
---

# What is ztl

ztl is a command-line tool that reads a folder of Markdown notes, understands the `[[wikilinks]]` between them, and lets you query, search, and reason over the resulting graph. Your notes stay on your laptop, in the filesystem, in plain `.md` files. ztl just reads them.

## What it does

Point ztl at a folder of notes and it will:

- **Parse wikilinks** — every `[[Zettelkasten Method]]`, `[[Book Notes|my reading list]]`, `[[Daily Journal#2026-04-20]]`, and `![[Project Brief]]` embed.
- **Build a link graph** — who points at whom, how many hops between two pages, which pages have no inbound links.
- **Answer questions** — forward links, backlinks (including multi-hop), shortest path between two pages, orphaned notes, dead links, pages with similar names.
- **Search content** — full-text and regex across the vault, with frontmatter and code-block awareness.
- **Render the vault** — as a terminal viewer (`ztl view`), a local web server (`ztl serve`), or a static HTML site (`ztl build`).
- **Reason over SPL** — optional defeasible-logic layer that reads `spl` code blocks from your notes and draws conclusions with full provenance. See [[What is SPL]].

A concrete example. You have `~/notes/` with 400 Markdown files. You run:

```bash
cd ~/notes
ztl index
ztl backlinks "Zettelkasten Method" --depth 2
ztl check --dead-links
```

ztl tells you which notes cite *Zettelkasten Method* directly, which cite those, and flags every `[[broken link]]` in the vault. No database. No daemon. The cache lives in `~/notes/.ztl/` and is disposable.

## Who it's for

Solo knowledge workers and small teams who:

- Already keep notes in plain Markdown and don't want to migrate.
- Use `[[wikilinks]]` (Obsidian, Logseq, Foam, Dendron, or hand-written — all work).
- Want *command-line* answers about their graph, not just a GUI.
- Care about local-first tools that treat your files as source of truth.

With `--collab`, small teams can co-edit the same vault over WebSocket with passkey auth and real-time CRDT sync. See [[Running a Team Server]].

## What it's not

- **Not a server-only SaaS.** ztl runs on your machine. There's no account to sign up for. Team mode is opt-in and self-hosted.
- **Not a new Markdown flavour.** ztl reads the Markdown you already write. Wikilink syntax is the same one Obsidian popularised. YAML frontmatter, headings, code fences, task lists — all standard.
- **Not a lock-in.** ztl never writes to your notes. The index under `.ztl/` is a cache; delete it and re-run `ztl index`, everything comes back. Your `.md` files are the only durable state.
- **Not a replacement for your editor.** Use whatever you already use — Vim, VS Code, Obsidian, a plain text editor. ztl just queries what's there.

## Next step

If you haven't installed it yet, go to [[Installation]]. Otherwise, spend 60 seconds with [[Quick Start]].

## Related

- [[Installation]]
- [[Quick Start]]
- [[Your First Vault]]
- [[Vaults]]
- [[Local-first]]
