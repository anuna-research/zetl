---
title: FAQ
tags: [help, faq, questions]
---

# FAQ

Answers to questions people actually ask before they install ztl. Each answer is short; follow the link for depth.

## Does ztl modify my files?

No. ztl treats vault files as read-only. The only directory ztl writes to is `.ztl/` at the vault root — the index, semantic model cache, optional jj snapshots, and (in collab mode) the auth and CRDT state all live there. The whole `.ztl/` directory is disposable: delete it, rerun `ztl index`, and you're back where you started.

See [[Local-first]] and [[Vaults]].

## Is ztl compatible with Obsidian / Logseq / Foam / Dendron?

Yes. ztl shares the `[[wikilink]]` syntax those tools use. Point ztl at an existing Obsidian or Logseq vault and `ztl index` just works — nothing in `.obsidian/` or `logseq/` is touched. You can keep editing in your current tool and use ztl for search, reasoning, history, or static-site export.

Import-specific notes live at [[Migrating from Obsidian]].

## Can I use ztl offline?

Yes. Everything — parsing, graph queries, reasoning, the web UI, collab, the terminal reader — runs locally. The only network activity by default is the optional one-time semantic-model download when you first use semantic search; everything else is file-system-local. You can use ztl on a plane.

See [[Local-first]].

## Do I need to learn Lisp to use ztl?

No. The graph features — wikilinks, backlinks, search, blocks, embeds, the web UI, collab — require no Lisp at all. Spindle Lisp (SPL) is an optional reasoning layer gated behind `--features reason`. Write plain Markdown, run plain ztl, and you never touch SPL.

If you're curious: [[What is SPL]].

## Can I use ztl without a git repo?

Yes. ztl never requires `.git/`. If you enable the history feature, temporal snapshots use jj stored in `.ztl/jj/` — a separate VCS from your repo's `.git/`. You can have both, either, or neither. In collab mode, saves auto-commit to a git repo if one exists; without one, edits still persist to disk and through the CRDT.

See [[Snapshots Under the Hood]].

## Does ztl support Windows?

ztl is written in portable Rust and targets Linux, macOS, and Windows. Build from source with a Rust toolchain. Some collab-mode features (passkey flows, WebSocket behaviour) are most exercised on Linux and macOS; file an issue if a Windows-specific rough edge bites.

See [[Installation]].

## How big a vault does ztl scale to?

ztl uses incremental caching — a two-tier (mtime + BLAKE3 hash) index — so re-runs only reparse changed files. Real-world scaling numbers aren't published; in practice the design aim is "your vault, however big it grows, stays snappy". If you hit a slow spot, [[Troubleshooting]] has a section on it and the issue tracker wants to hear.

## Can multiple people use the same vault?

Two models:

1. **Live collab server** — run `ztl serve --collab` and invite collaborators with passkeys. Edits sync in real time through a CRDT engine; every save auto-commits to git with author attribution. See [[Running a Team Server]].
2. **Shared git repo** — commit your vault to git and let collaborators clone, edit, and push as they would any other text repo. ztl doesn't care whether the file on disk came from a human, a collaborator, or a merge commit.

You can combine the two.

## What's the difference between a lifecycle hook and a render-pipeline hook?

**Lifecycle hooks** run at coarse events — `pre-build`, `post-build`, `on-save`, etc. They get the whole vault context as JSON on stdin, run once per event, and are ideal for tasks like generating an RSS feed or posting to a webhook.

**Render-pipeline hooks** run inside the build, per page, at one of three fine-grained stages — `pre-parse`, `transform`, or `post-render`. They're persistent subprocesses speaking a JSON-lines protocol, operate on a typed AST, and are the right tool for custom Markdown extensions, callouts, or pandoc integration.

Details: [[Lifecycle Hooks]] and [[Render Pipeline Hooks]].

## Is there a plugin marketplace?

Not yet. Two things take the pressure off until there is:

- ztl ships first-class adapters for **Pandoc**, **mdBook**, and **remark** — three huge existing plugin ecosystems. You can run a Pandoc Lua filter as a render-pipeline hook with a four-line manifest.
- The hook authoring CLI (`ztl hook new/test/watch`) makes writing your own a few-minute task, not a weekend project.

See [[Plugin Ecosystems]].

## What about AI agents?

ztl exposes graph, search, and reasoning as typed tools over the Model Context Protocol. Build with `--features mcp`, run `ztl mcp` over stdio or HTTP, and issue scoped delegate tokens to your agent.

See [[MCP Server]] and [[Plugin Ecosystems]].

## Is ztl free?

Yes. ztl is released under **AGPL-3.0-or-later**. The source is at <https://codeberg.org/anuna/ztl>.

## Where does ztl store its data?

Everything ztl writes lives in `.ztl/` next to your vault:

```
your-vault/
  Some Page.md
  Another.md
  .ztl/
    index/        # link graph, content-addressable blocks
    jj/           # history snapshots (with --features history)
    themes/       # custom themes
    hooks/        # lifecycle and render-pipeline hooks
    collab/       # auth, CRDT state (in --collab mode)
```

All of it is disposable. Delete `.ztl/` and your notes are untouched; a fresh `ztl index` rebuilds the cache from scratch. See [[Configuration]].

## Related

- [[Troubleshooting]]
- [[What is ztl]]
- [[Installation]]
- [[Quick Start]]
- [[Local-first]]
