---
title: Vaults
tags: [concepts, vault, storage]
---

# Vaults

A **vault** is any directory that contains Markdown files. That is the whole definition. zetl does not own your notes, does not require a config file, and does not need to import anything — point it at a folder and it starts working.

## What's in a vault

A vault is a tree of `.md` files. Folders are just folders; there is no hidden database beside your notes. The only thing zetl adds is a `.zetl/` directory at the vault root, used for caching:

```
~/notes/
  Zettelkasten Method.md
  projects/
    zetl.md
    daily/
      2026-04-15.md
  .zetl/           <- zetl's cache; safe to delete
    index.json
    blocks/
    theory/        <- only with --features reason
    jj/            <- only with --features history
```

Everything in `.zetl/` is disposable. Delete it and the next `zetl index` rebuilds it from your files. This matters: your vault is your Markdown, not zetl's cache. See [[Local-first]].

## How zetl walks a vault

When you run `zetl index`, zetl walks the tree using a layered ignore stack. From lowest to highest precedence:

| Layer | Rule |
|-------|------|
| 1 | Hardcoded: `.git/`, `.zetl/`, `node_modules/` are never scanned |
| 2 | Dotdirs like `.obsidian/`, `.vscode/`, `.claude/` are skipped by default (`--include-hidden` disables) |
| 3 | `.gitignore` patterns, if present |
| 4 | `.zetlignore` at the vault root (gitignore syntax; `!pattern` re-includes) |
| 5 | `--exclude PATTERN` CLI flag (repeatable, highest priority) |

This means a zetl vault coexists cleanly with a git repo, a node project, an Obsidian workspace, or a Logseq graph. You do not need to rearrange files.

## Pointing zetl at a vault

Two ways, and they are equivalent:

```bash
# Explicit
zetl -d ~/notes index

# Or set the env var once per shell
export ZETL_DIR=~/notes
zetl index
zetl links "Zettelkasten Method"
```

Run zetl with no `-d` and no `ZETL_DIR`, and it uses the current directory.

## You can have many vaults

A vault is just a folder, so you can keep as many as you like — one for research, one for journaling, one for client work — each with its own `.zetl/` cache and its own graph. They do not share any state. Switching is just `cd` or `-d`.

Some people keep one enormous vault and lean on folders plus tags; others keep several small ones. Neither is wrong. See [[Organising Your Vault]].

## What a vault is *not*

- **Not a database.** Your notes are flat files. Open them in any editor; edit them with any tool.
- **Not a format.** zetl reads the Markdown you already have. No special syntax is required. Wikilinks and frontmatter are optional.
- **Not locked in.** If you stop using zetl tomorrow, you still have a directory of Markdown files. Everything zetl added is in `.zetl/`.

## Related

- [[Your First Vault]]
- [[Local-first]]
- [[Organising Your Vault]]
- [[Configuration]]
