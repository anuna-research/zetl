---
title: Migrating from Obsidian
tags: [migration, obsidian, getting-started]
---

# Migrating from Obsidian

There is no migration. Point ztl at your Obsidian vault and it works. The two tools read the same Markdown, the same wikilink syntax, and the same YAML frontmatter — your notes are the source of truth for both.

## The short version

```bash
ztl -d ~/Documents/ObsidianVault index
ztl -d ~/Documents/ObsidianVault serve
```

Open your vault in Obsidian at the same time. Both read your `.md` files. Neither locks anything. ztl never writes to your notes — only to a disposable cache under `.ztl/`.

## Syntax compatibility

Every wikilink shape Obsidian uses, ztl parses the same way:

| Syntax | Meaning |
|--------|---------|
| `[[Zettelkasten Method]]` | Standard link. |
| `[[Zettelkasten Method\|the method]]` | Aliased display text. |
| `[[Zettelkasten Method#History]]` | Link to a specific heading. |
| `[[Zettelkasten Method^abc123]]` | Link to a block by ID. |
| `![[Book Notes]]` | Embed (transclude) the whole page. |
| `![[Book Notes#Ch 3]]` | Embed a section. |

See [[Wikilinks]] for the ztl-specific details (resolution rules, ambiguity handling).

## Frontmatter

ztl parses YAML frontmatter the same way Obsidian does. Existing `tags`, `aliases`, `publish`, and anything else you rely on will surface in `page.frontmatter.*` for templates, and tags flow into [[Tags and Frontmatter]] queries. Custom keys — `status`, `project`, `deadline` — are preserved verbatim and available in templates and hooks.

A frontmatter block Obsidian wrote this morning:

```markdown
---
tags: [research, reading]
aliases: [Adler, How to Read]
status: in-progress
---
```

…just works in ztl with no changes.

## Hidden folders and `.obsidian/`

By default ztl skips dotdirs — `.obsidian/`, `.trash/`, `.git/`, and friends — during vault scans. Your Obsidian config, plugins, and workspace state are invisible to ztl, which is almost always what you want.

If you need ztl to walk those folders (for example, to publish `.obsidian/` docs as part of a static site), pass `--include-hidden`:

```bash
ztl --include-hidden index
ztl --include-hidden build
```

`.git/`, `.ztl/`, and `node_modules/` remain excluded regardless — they're hardcoded in the scanner and not overridable.

See [[Organising Your Vault]] for the full layered exclusion model and `.ztlignore`.

## Obsidian feature → ztl equivalent

| Obsidian | ztl |
|----------|------|
| Backlinks pane | `ztl backlinks "Page"` (CLI) or the right-rail backlinks in `ztl serve`. |
| Graph view | The Sigma-based graph widget in `ztl serve` / `ztl build`, plus `/_graph` full-screen. See [[The Link Graph]]. |
| Quick switcher | `ztl view` opens a page picker; `ztl search` for content. |
| Search | `ztl search "query"` (full-text), `ztl search --regex`, `ztl similar` for fuzzy page-name matching. |
| Tags (`#tag`) | Parsed from YAML `tags: [...]`. Query via `page.frontmatter.tags` in templates. See [[Tags and Frontmatter]]. |
| Embeds (`![[...]]`) | Same syntax; rendered inline in `ztl serve` / `ztl build`. See [[Embeds and Transclusion]]. |
| Canvas (`.canvas`) | Not supported. Canvas files are skipped. |
| Daily Notes plugin | Not supported as a plugin. Build daily files by hand or with a `pre-build` lifecycle hook. |
| Templater / Dataview | Not supported. The render-pipeline hooks (see [[Render Pipeline Hooks]]) are the ztl equivalent for transforming content mid-build. |
| Publish / Obsidian Sync | `ztl build` writes static HTML; [[Capability URLs]] share live views without a server. |

## What doesn't port over

The parts of your Obsidian setup that live in `.obsidian/` — plugins, workspace layout, hotkeys, themes, community plugin settings — don't map into ztl, because ztl isn't a GUI editor. Your *notes* port perfectly; your *Obsidian environment* does not.

- **Canvas files** (`.canvas`) are skipped. The JSON format isn't parsed.
- **Templater / QuickAdd / Dataview / Excalidraw** — these are Obsidian-specific extensions that run inside the Obsidian app. For ztl, the nearest equivalent is [[Lifecycle Hooks]] (shell scripts at pre/post-build) or [[Render Pipeline Hooks]] (AST-level transforms during `ztl build`).
- **Daily Notes** as a built-in feature doesn't exist. Either create daily files manually or script them in a hook.
- **Plugins** in general. ztl's extension model is render hooks + ecosystem adapters (Pandoc, mdBook, remark). See [[Plugin Ecosystems]].

## Running both at once

This is the comfortable path while you try ztl. Open your vault in Obsidian as usual; run `ztl serve` from the same directory in another terminal. Both processes read the same `.md` files on disk.

- Save a note in Obsidian → ztl's serve watcher picks up the change and re-indexes.
- Save a note in `ztl serve`'s browser editor → the file on disk updates; Obsidian's file-watcher reloads.

The only coordination point is `.ztl/`, which is ztl's private cache. Obsidian ignores it. Add `.ztl/` to your `.gitignore` if you version-control the vault.

## Read-only guarantee

ztl never modifies your `.md` files during `index`, `check`, `view`, or `build`. `serve` writes only when *you* edit and save through its browser UI. If that still feels too close, run your whole session in a read-only copy:

```bash
cp -r ~/Documents/ObsidianVault /tmp/vault-copy
ztl -d /tmp/vault-copy index
ztl -d /tmp/vault-copy serve
```

## Related

- [[What is ztl]]
- [[Quick Start]]
- [[Your First Vault]]
- [[Wikilinks]]
- [[Organising Your Vault]]
