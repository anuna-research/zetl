---
title: Vaults
tags: [concepts, vault, storage]
---

# Vaults

A **vault** is any directory that contains Markdown files. That is the whole definition. ztl does not own your notes, does not require a config file, and does not need to import anything — point it at a folder and it starts working.

## What's in a vault

A vault is a tree of `.md` files. Folders are just folders; there is no hidden database beside your notes. The only thing ztl adds is a `.ztl/` directory at the vault root, used for caching:

```
~/notes/
  Zettelkasten Method.md
  projects/
    ztl.md
    daily/
      2026-04-15.md
  .ztl/           <- ztl's cache; safe to delete
    index.json
    blocks/
    theory/        <- only with --features reason
    jj/            <- only with --features history
```

Everything in `.ztl/` is disposable. Delete it and the next `ztl index` rebuilds it from your files. This matters: your vault is your Markdown, not ztl's cache. See [[Local-first]].

## How ztl walks a vault

When you run `ztl index`, ztl walks the tree using a layered ignore stack. From lowest to highest precedence:

| Layer | Rule |
|-------|------|
| 1 | Hardcoded: `.git/`, `.ztl/`, `node_modules/` are never scanned |
| 2 | Dotdirs like `.obsidian/`, `.vscode/`, `.claude/` are skipped by default (`--include-hidden` disables) |
| 3 | `.gitignore` patterns, if present |
| 4 | `.ztlignore` at the vault root (gitignore syntax; `!pattern` re-includes) |
| 5 | `--exclude PATTERN` CLI flag (repeatable, highest priority) |

This means a ztl vault coexists cleanly with a git repo, a node project, an Obsidian workspace, or a Logseq graph. You do not need to rearrange files.

## Pointing ztl at a vault

Two ways, and they are equivalent:

```bash
# Explicit
ztl -d ~/notes index

# Or set the env var once per shell
export ztl_DIR=~/notes
ztl index
ztl links "Zettelkasten Method"
```

Run ztl with no `-d` and no `ztl_DIR`, and it uses the current directory.

## You can have many vaults

A vault is just a folder, so you can keep as many as you like — one for research, one for journaling, one for client work — each with its own `.ztl/` cache and its own graph. They do not share any state. Switching is just `cd` or `-d`.

Some people keep one enormous vault and lean on folders plus tags; others keep several small ones. Neither is wrong. See [[Organising Your Vault]].

## What a vault is *not*

- **Not a database.** Your notes are flat files. Open them in any editor; edit them with any tool.
- **Not a format.** ztl reads the Markdown you already have. No special syntax is required. Wikilinks and frontmatter are optional.
- **Not locked in.** If you stop using ztl tomorrow, you still have a directory of Markdown files. Everything ztl added is in `.ztl/`.

## Related

- [[Your First Vault]]
- [[Local-first]]
- [[Organising Your Vault]]
- [[Configuration]]
