# Shared brief — ztl user guide content agents

You are writing one section of a comprehensive ztl user guide. The guide lives at `/Users/anuna-01/Code/ztl-docs/` and is itself a ztl vault.

## Audience

**Writers and knowledge workers.** People comfortable with Markdown and a terminal, but not kernel hackers. Explain *why* before *how*. Use real examples from a note-taking context (research, journaling, project notes, documentation), not abstract ones.

## Source material (read these)

- `/Users/anuna-01/Code/ztl/README.md` — authoritative feature list (≈1200 lines).
- `/Users/anuna-01/Code/ztl/ztl-features.md` — shorter feature summary.
- `/Users/anuna-01/Code/ztl/docs/` — topic-specific deep dives (capability-mode, hook-security, signing, ecosystems/, canonical-extensions, reader-troubleshooting, ztl-ast-reference).
- `/Users/anuna-01/Code/ztl/ztl-vault/` — the previous iteration of the ztl docs vault; mine for accurate descriptions of internal machinery (scanner, cache, merkle tree, reasoning engine, drift detection). **Do not copy verbatim** — this guide is a fresh write with a different audience.
- `/Users/anuna-01/Code/ztl/demo-vault/` — a working example vault demonstrating most features.
- The `ztl` binary is on PATH. Run `ztl <cmd> --help` to verify flags and defaults before writing about them. Treat the binary as authoritative — it is v0.5.0.

## Target vault layout (context for wikilinks)

Every `[[link]]` target below is a real page name in the vault. Link to siblings in your section freely; link across sections by the exact names shown in `/Users/anuna-01/Code/ztl-docs/Index.md` (the landing page, which you should read before writing).

## Writing rules

1. **YAML frontmatter on every page**: `title:` matches the filename; `tags:` is a short list (2–4 tags).
2. **First line after frontmatter** is an `# H1` matching the title.
3. **Lead paragraph**: one or two sentences stating what the page is about. No throat-clearing.
4. **Length**: 300–800 words per page typical. Long pages can go to 1200 words if the topic warrants it.
5. **Code examples**: always with a language fence (```bash, ```markdown, ```toml, ```spl). Use realistic vault paths like `~/notes/`, page names like `Zettelkasten Method.md`, not `foo.md`.
6. **Tables** when comparing options, commands, or flags.
7. **Every page ends with a `## Related` section** containing 2–5 wikilinks to pages in other sections of the guide that are genuinely relevant.
8. **Wikilinks, not full paths.** Write `[[Quick Start]]`, not `[[getting-started/Quick Start]]` — ztl resolves by page name.
9. **No emojis** unless reproducing a UI element (e.g. terminal output).
10. **No throwaway promises** like "we'll cover this later" — just link to the relevant page.
11. **Feature-gated features**: if a command requires `--features reason` or `--features history`, say so in a boxed note at the top of the page. Example:
    > **Requires `--features reason` at install time.** See [[Installation]].

## Accuracy

Before describing a command or flag, verify it exists:

```bash
ztl <subcommand> --help
```

If the README describes a feature that `--help` doesn't back up, prefer what `--help` says — the binary is v0.5.0 and ships what's actually there. Don't invent flags. Don't invent subcommands.

## Voice

- Direct. Declarative. "`ztl index` scans the vault and writes a cached graph." Not "`ztl index` is a command that, when run, will scan the vault …"
- Second person is fine for instructions ("you run", "your vault").
- "ztl" is always lower-case, even at sentence start.

## File names

Use title-case filenames with spaces, ending in `.md`. Example: `Writing Pages.md`, not `writing-pages.md`. ztl resolves `[[Writing Pages]]` to this file.

## Before you start

Read `/Users/anuna-01/Code/ztl-docs/Index.md` so you know exactly which page names are canonical for cross-section links.
