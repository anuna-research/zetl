---
title: Local-first
tags: [concepts, philosophy, design]
---

# Local-first

zetl is local-first. That phrase is doing real work — not "runs locally" and not "offline-capable", but a specific design stance about whose computer owns your notes. This page is the philosophy; every other design choice in zetl follows from it.

## The principles

**Your files are the source of truth.** Not a database, not a cloud tenant, not a proprietary binary format. A zetl vault is a directory of plain `.md` files. Whatever any zetl command says about your vault, the Markdown on disk is the authority. If zetl and the files disagree, the files win.

**zetl never modifies your Markdown.** Indexing, reasoning, rendering, validation — none of these rewrite your notes. Wikilinks stay wikilinks; frontmatter stays whatever YAML you wrote. The only bytes zetl writes are under `.zetl/` (its cache) and, optionally, an output directory from `zetl build`.

**Everything in `.zetl/` is disposable.** Link index, block hashes, snapshot history, reasoning theory — all of it is cache, derived from your files. Delete `.zetl/` and `zetl index` rebuilds it. Commit your vault without it; share vaults across machines by syncing the `.md` files alone.

**Works offline.** zetl is a single binary that reads from and writes to your local filesystem. No network is required for any core operation — indexing, searching, viewing, building, reasoning. A machine in airplane mode has the full feature set.

**Data outlives the tool.** If zetl disappears tomorrow, or if you decide it's not for you, your vault is still a folder of Markdown files. Open them in any text editor, any other wikilink tool, any static site generator. You did not sign up for a migration problem.

## Why this matters for knowledge work

A second brain is only useful if you trust it to still be there in ten years. Most knowledge tools violate at least one of the above: they own the format, they own the storage, they own the sync layer, or they break when offline. The cost compounds — every note you write increases your exposure to someone else's product decisions.

Local-first flips the dependency. You own the files. Tools like zetl *read* them and give you views — a graph, a search index, a rendered site, a reasoning layer — without taking ownership. When a better tool comes along, your vault is already portable to it.

## What local-first does *not* mean

- **Not single-machine.** You can sync a vault across devices with git, Syncthing, iCloud, Dropbox, rsync, or whatever you already use. zetl does not care how the files got there.
- **Not single-user.** zetl has an optional multi-user mode (`zetl serve --collab`) with real-time CRDT editing. But even there, the authoritative state is the Markdown on disk — the server commits changes back to files on save. See [[Running a Team Server]].
- **Not anti-cloud.** Publish a vault to a static host with `zetl build`. Hand out [[Capability URLs]] for private reads. The cloud is a place you choose to put copies, not a place your notes live by default.

## How zetl stays out of your way

| What could go wrong | What zetl does instead |
|---------------------|------------------------|
| Fix your "broken" wikilinks | Reports them; you choose the fix |
| Auto-create missing pages | Leaves dead links alone |
| Normalise your frontmatter | Reads whatever YAML you wrote |
| Rewrite your files on build | Reads them; writes output separately |
| Require a config file | Works with zero configuration |

This restraint is the feature.

## Related

- [[Vaults]]
- [[What is zetl]]
- [[Local-first|Data ownership]]
- [[Static Site Export]]
