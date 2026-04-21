---
title: Organising Your Vault
tags: [writing, organisation, folders]
---

# Organising Your Vault

zetl is indifferent to how your vault is laid out. Pages resolve by filename,
not path, so you can reorganise at any time without breaking links.

## No required structure

A handful of layouts people actually use:

**Flat.** Every page in the vault root. Works surprisingly well up to a few
hundred pages. The link graph provides the structure; the filesystem is just
a blob.

```
~/notes/
  Annual Review 2026.md
  Reading List 2026.md
  Scientific Management.md
  Seeing Like a State.md
  Zettelkasten Method.md
```

**Topic-based.** Top-level folders by subject.

```
~/notes/
  Books/
    Seeing Like a State.md
  Essays/
    Legibility and Craft.md
  Journal/
    2026-04-18.md
  Reference/
    SPL Cheatsheet.md
```

**Johnny Decimal.** Numbered categories. Good when the topics are stable and
you value predictable paths.

```
~/notes/
  10 Work/
    11 Projects/
    12 Meetings/
  20 Reading/
    21 Books/
    22 Papers/
  30 Personal/
```

**Zettelkasten-style.** Timestamped slip-boxes, no folders, atomic notes
linked into trails. Works, but not required — zetl is not a Zettelkasten
tool, just a link-graph tool.

```
~/notes/
  202604181423 Legibility as precondition.md
  202604181431 Taylor and Scott in parallel.md
```

None of these is "right." Pick whatever matches how your brain looks things
up when you're not thinking about tooling.

## Pragmatic advice: start flat

If you're starting a new vault, put everything in the root. When you pass
around 100 pages and finding files in your editor gets annoying, group the
obvious clusters into folders. You'll know which ones — they're the folders
you wish existed.

## Rename and move freely

Because links resolve by filename, moving `Seeing Like a State.md` from
`~/notes/` to `~/notes/Books/` doesn't break a single `[[Seeing Like a State]]`
reference. After the move:

```bash
zetl index     # refresh the cached graph
zetl check     # confirm no dead links
```

Renaming is a little more work — see [[Linking Pages]] for the rename workflow.

## `.zetlignore` — excluding drafts and private files

zetl scans the entire vault root by default. Put files and folders you want
zetl to ignore into a `.zetlignore` file using gitignore syntax:

```
# ~/notes/.zetlignore
drafts/
private/
*.tmp
scratch.md
```

Patterns match the vault root. Subdirectory `.zetlignore` files are not
honoured — keep it all in one file at the top.

zetl already excludes `.git/`, `.zetl/`, `node_modules/`, and dotdirs like
`.obsidian/` by default. Run `zetl build --include-hidden` to include them
anyway, or `zetl build --exclude 'pattern/'` for a one-off exclusion that
doesn't touch `.zetlignore`.

## Tags vs folders

Both are fine. They answer different questions.

| Folders answer | Tags answer |
|---|---|
| "Where does this file live?" | "What is this file about?" |
| One per page | Many per page |
| Exclusive | Overlapping |
| Filesystem-native | Metadata-native |

A meeting note might live in `Meetings/2026-04-18.md` (one place) and carry
`tags: [project-x, priya]` (two dimensions). Folders scale for filesystem
browsing; tags scale for cross-cutting queries. See
[[Tags and Frontmatter]].

## Naming conventions

Three patterns pull their weight:

- **Titles.** `Zettelkasten Method.md`, not `zettelkasten-method.md`. Human
  filenames. zetl slugifies at render.
- **Dates.** `2026-04-18` (ISO 8601) sorts right and never confuses month
  order. Use it in journal/meeting filenames.
- **Uniqueness.** Two pages with the same title on the graph is ambiguous.
  Disambiguate in the filename: `Meeting with Priya 2026-04-18.md`,
  `Scientific Management (book).md`.

## Related

- [[Vaults]]
- [[Tags and Frontmatter]]
- [[Writing Pages]]
- [[Linking Pages]]
- [[Configuration]]
