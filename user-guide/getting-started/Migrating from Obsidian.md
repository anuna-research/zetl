---
title: Migrating from Obsidian
tags: [migration, obsidian, getting-started]
---

# Migrating from Obsidian

There is no migration. Point zetl at your Obsidian vault and it works. The two tools read the same Markdown, the same wikilink syntax, and the same YAML frontmatter — your notes are the source of truth for both.

## The short version

```bash
zetl -d ~/Documents/ObsidianVault index
zetl -d ~/Documents/ObsidianVault serve
```

Open your vault in Obsidian at the same time. Both read your `.md` files. Neither locks anything. zetl never writes to your notes — only to a disposable cache under `.zetl/`.

## Syntax compatibility

Every wikilink shape Obsidian uses, zetl parses the same way:

| Syntax | Meaning |
|--------|---------|
| `[[Zettelkasten Method]]` | Standard link. |
| `[[Zettelkasten Method\|the method]]` | Aliased display text. |
| `[[Zettelkasten Method#History]]` | Link to a specific heading. |
| `[[Zettelkasten Method^abc123]]` | Link to a block by ID. |
| `![[Book Notes]]` | Embed (transclude) the whole page. |
| `![[Book Notes#Ch 3]]` | Embed a section. |

See [[Wikilinks]] for the zetl-specific details (resolution rules, ambiguity handling).

## Frontmatter

zetl parses YAML frontmatter the same way Obsidian does. Existing `tags`, `aliases`, `publish`, and anything else you rely on will surface in `page.frontmatter.*` for templates, and tags flow into [[Tags and Frontmatter]] queries. Custom keys — `status`, `project`, `deadline` — are preserved verbatim and available in templates and hooks.

A frontmatter block Obsidian wrote this morning:

```markdown
---
tags: [research, reading]
aliases: [Adler, How to Read]
status: in-progress
---
```

…just works in zetl with no changes.

## Hidden folders and `.obsidian/`

By default zetl skips dotdirs — `.obsidian/`, `.trash/`, `.git/`, and friends — during vault scans. Your Obsidian config, plugins, and workspace state are invisible to zetl, which is almost always what you want.

If you need zetl to walk those folders (for example, to publish `.obsidian/` docs as part of a static site), pass `--include-hidden`:

```bash
zetl --include-hidden index
zetl --include-hidden build
```

`.git/`, `.zetl/`, and `node_modules/` remain excluded regardless — they're hardcoded in the scanner and not overridable.

See [[Organising Your Vault]] for the full layered exclusion model and `.zetlignore`.

## Obsidian feature → zetl equivalent

| Obsidian | zetl |
|----------|------|
| Backlinks pane | `zetl backlinks "Page"` (CLI) or the right-rail backlinks in `zetl serve`. |
| Graph view | The Sigma-based graph widget in `zetl serve` / `zetl build`, plus `/_graph` full-screen. See [[The Link Graph]]. |
| Quick switcher | `zetl view` opens a page picker; `zetl search` for content. |
| Search | `zetl search "query"` (full-text), `zetl search --regex`, `zetl similar` for fuzzy page-name matching. |
| Tags (`#tag`) | Parsed from YAML `tags: [...]`. Query via `page.frontmatter.tags` in templates. See [[Tags and Frontmatter]]. |
| Embeds (`![[...]]`) | Same syntax; rendered inline in `zetl serve` / `zetl build`. See [[Embeds and Transclusion]]. |
| Canvas (`.canvas`) | Not supported. Canvas files are skipped. |
| Daily Notes plugin | Not supported as a plugin. Build daily files by hand or with a `pre-build` lifecycle hook. |
| Templater / Dataview | Not supported. The render-pipeline hooks (see [[Render Pipeline Hooks]]) are the zetl equivalent for transforming content mid-build. |
| Publish / Obsidian Sync | `zetl build` writes static HTML; [[Capability URLs]] share live views without a server. |

## What doesn't port over

The parts of your Obsidian setup that live in `.obsidian/` — plugins, workspace layout, hotkeys, themes, community plugin settings — don't map into zetl, because zetl isn't a GUI editor. Your *notes* port perfectly; your *Obsidian environment* does not.

- **Canvas files** (`.canvas`) are skipped. The JSON format isn't parsed.
- **Templater / QuickAdd / Dataview / Excalidraw** — these are Obsidian-specific extensions that run inside the Obsidian app. For zetl, the nearest equivalent is [[Lifecycle Hooks]] (shell scripts at pre/post-build) or [[Render Pipeline Hooks]] (AST-level transforms during `zetl build`).
- **Daily Notes** as a built-in feature doesn't exist. Either create daily files manually or script them in a hook.
- **Plugins** in general. zetl's extension model is render hooks + ecosystem adapters (Pandoc, mdBook, remark). See [[Plugin Ecosystems]].

## Running both at once

This is the comfortable path while you try zetl. Open your vault in Obsidian as usual; run `zetl serve` from the same directory in another terminal. Both processes read the same `.md` files on disk.

- Save a note in Obsidian → zetl's serve watcher picks up the change and re-indexes.
- Save a note in `zetl serve`'s browser editor → the file on disk updates; Obsidian's file-watcher reloads.

The only coordination point is `.zetl/`, which is zetl's private cache. Obsidian ignores it. Add `.zetl/` to your `.gitignore` if you version-control the vault.

## Read-only guarantee

zetl never modifies your `.md` files during `index`, `check`, `view`, or `build`. `serve` writes only when *you* edit and save through its browser UI. If that still feels too close, run your whole session in a read-only copy:

```bash
cp -r ~/Documents/ObsidianVault /tmp/vault-copy
zetl -d /tmp/vault-copy index
zetl -d /tmp/vault-copy serve
```

## Related

- [[What is zetl]]
- [[Quick Start]]
- [[Your First Vault]]
- [[Wikilinks]]
- [[Organising Your Vault]]
