---
title: FAQ
tags: [help, faq, questions]
---

# FAQ

Answers to questions people actually ask before they install zetl. Each answer is short; follow the link for depth.

## Does zetl modify my files?

No. zetl treats vault files as read-only. The only directory zetl writes to is `.zetl/` at the vault root — the index, semantic model cache, optional jj snapshots, and (in collab mode) the auth and CRDT state all live there. The whole `.zetl/` directory is disposable: delete it, rerun `zetl index`, and you're back where you started.

See [[Local-first]] and [[Vaults]].

## Is zetl compatible with Obsidian / Logseq / Foam / Dendron?

Yes. zetl shares the `[[wikilink]]` syntax those tools use. Point zetl at an existing Obsidian or Logseq vault and `zetl index` just works — nothing in `.obsidian/` or `logseq/` is touched. You can keep editing in your current tool and use zetl for search, reasoning, history, or static-site export.

Import-specific notes live at [[Migrating from Obsidian]].

## Can I use zetl offline?

Yes. Everything — parsing, graph queries, reasoning, the web UI, collab, the terminal reader — runs locally. The only network activity by default is the optional one-time semantic-model download when you first use semantic search; everything else is file-system-local. You can use zetl on a plane.

See [[Local-first]].

## Do I need to learn Lisp to use zetl?

No. The graph features — wikilinks, backlinks, search, blocks, embeds, the web UI, collab — require no Lisp at all. Spindle Lisp (SPL) is an optional reasoning layer gated behind `--features reason`. Write plain Markdown, run plain zetl, and you never touch SPL.

If you're curious: [[What is SPL]].

## Can I use zetl without a git repo?

Yes. zetl never requires `.git/`. If you enable the history feature, temporal snapshots use jj stored in `.zetl/jj/` — a separate VCS from your repo's `.git/`. You can have both, either, or neither. In collab mode, saves auto-commit to a git repo if one exists; without one, edits still persist to disk and through the CRDT.

See [[Snapshots Under the Hood]].

## Does zetl support Windows?

zetl is written in portable Rust and targets Linux, macOS, and Windows. Build from source with a Rust toolchain. Some collab-mode features (passkey flows, WebSocket behaviour) are most exercised on Linux and macOS; file an issue if a Windows-specific rough edge bites.

See [[Installation]].

## How big a vault does zetl scale to?

zetl uses incremental caching — a two-tier (mtime + BLAKE3 hash) index — so re-runs only reparse changed files. Real-world scaling numbers aren't published; in practice the design aim is "your vault, however big it grows, stays snappy". If you hit a slow spot, [[Troubleshooting]] has a section on it and the issue tracker wants to hear.

## Can multiple people use the same vault?

Two models:

1. **Live collab server** — run `zetl serve --collab` and invite collaborators with passkeys. Edits sync in real time through a CRDT engine; every save auto-commits to git with author attribution. See [[Running a Team Server]].
2. **Shared git repo** — commit your vault to git and let collaborators clone, edit, and push as they would any other text repo. zetl doesn't care whether the file on disk came from a human, a collaborator, or a merge commit.

You can combine the two.

## What's the difference between a lifecycle hook and a render-pipeline hook?

**Lifecycle hooks** run at coarse events — `pre-build`, `post-build`, `on-save`, etc. They get the whole vault context as JSON on stdin, run once per event, and are ideal for tasks like generating an RSS feed or posting to a webhook.

**Render-pipeline hooks** run inside the build, per page, at one of three fine-grained stages — `pre-parse`, `transform`, or `post-render`. They're persistent subprocesses speaking a JSON-lines protocol, operate on a typed AST, and are the right tool for custom Markdown extensions, callouts, or pandoc integration.

Details: [[Lifecycle Hooks]] and [[Render Pipeline Hooks]].

## Is there a plugin marketplace?

Not yet. Two things take the pressure off until there is:

- zetl ships first-class adapters for **Pandoc**, **mdBook**, and **remark** — three huge existing plugin ecosystems. You can run a Pandoc Lua filter as a render-pipeline hook with a four-line manifest.
- The hook authoring CLI (`zetl hook new/test/watch`) makes writing your own a few-minute task, not a weekend project.

See [[Plugin Ecosystems]].

## What about AI agents?

zetl exposes graph, search, and reasoning as typed tools over the Model Context Protocol. Build with `--features mcp`, run `zetl mcp` over stdio or HTTP, and issue scoped delegate tokens to your agent.

See [[MCP Server]] and [[Plugin Ecosystems]].

## Is zetl free?

Yes. zetl is released under **AGPL-3.0-or-later**. The source is at <https://codeberg.org/anuna/zetl>.

## Where does zetl store its data?

Everything zetl writes lives in `.zetl/` next to your vault:

```
your-vault/
  Some Page.md
  Another.md
  .zetl/
    index/        # link graph, content-addressable blocks
    jj/           # history snapshots (with --features history)
    themes/       # custom themes
    hooks/        # lifecycle and render-pipeline hooks
    collab/       # auth, CRDT state (in --collab mode)
```

All of it is disposable. Delete `.zetl/` and your notes are untouched; a fresh `zetl index` rebuilds the cache from scratch. See [[Configuration]].

## Related

- [[Troubleshooting]]
- [[What is zetl]]
- [[Installation]]
- [[Quick Start]]
- [[Local-first]]
