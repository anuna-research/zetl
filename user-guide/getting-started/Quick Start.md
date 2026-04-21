---
title: Quick Start
tags: [tutorial, getting-started]
---

# Quick Start

Sixty seconds from install to first result. This page uses the `demo-vault/` bundled with the zetl repo — a self-referential knowledge base about zetl itself. Run each command; what you see should match what's described.

## Get the demo vault

If you built zetl from source, `demo-vault/` is already next to your clone. From the zetl source directory:

```bash
cd /path/to/zetl
ls demo-vault/
# architecture/  concepts/  decisions/  features/  theories/  Demo Script.md ...
```

## Build the index

```bash
zetl -d ./demo-vault index
```

You'll see a one-line summary and a JSON block (zetl auto-detects JSON when output is redirected; in a terminal it prints a table). Something like:

```json
{
  "files_scanned": 27,
  "links_found": 152,
  "dead_links": 2,
  "diagnostics": 0,
  "elapsed_ms": 28
}
```

27 files, 152 wikilinks, 2 dead links. The index now lives in `demo-vault/.zetl/` and is reused on subsequent runs — zetl only reparses files whose mtime or hash changed.

## Follow a link

Every Markdown file that mentions `[[Scanner]]` gets threaded into the graph. Ask what *Scanner* points at:

```bash
zetl -d ./demo-vault links "Scanner"
```

In a terminal you'll get a table:

```
Forward links from 'Scanner':
+-----+---------+--------------------+------+
| Hop | Source  | Target             | Line |
+===========================================+
| 1   | Scanner | Reasoning Engine   | 30   |
| 1   | Scanner | Link Graph         | 30   |
| 1   | Scanner | Cache              | 30   |
| 1   | Scanner | Local-first Design | 28   |
| 1   | Scanner | Spindle Lisp       | 3    |
+-----+---------+--------------------+------+
```

Each row is one wikilink, with the line number in the source file so you can jump straight to it.

## Walk backlinks, two hops

Which notes point at `Cache` — and which notes point at *those* notes?

```bash
zetl -d ./demo-vault backlinks "Cache" --depth 2
```

`--depth 2` means "include second-hop citations." You'll see direct backlinks (hop 1) and transitive ones (hop 2) labelled so you know which is which.

## Open the web UI

```bash
zetl -d ./demo-vault serve
# Serving on http://localhost:3000
```

Browse to the URL. You get a sidebar with every page, rendered Markdown, a backlink list, a transclusion panel showing excerpts of forward-linked pages, and a mini link-graph widget bottom-right. Edits save straight back to the `.md` file on disk. Ctrl-C stops the server.

## Reason over the vault

> **Requires `--features reason` at install time.** See [[Installation]].

The demo vault has SPL facts and rules sprinkled through its pages — zetl's reasoning engine pulls them out and draws conclusions.

```bash
zetl -d ./demo-vault reason status
```

You'll see a theory summary and a list of conclusions, each labelled with one of `+D` (definitely provable), `-D` (definitely not), `+d` (defeasibly provable), or `-d` (defeasibly not), along with the source file and line number that grounds each one. For example:

```
Theory: 46 facts, 24 rules, 0 defeaters, 0 superiority relations from 18 files

+-----+---------------------------+-------------------------------+
| Tag | Literal                   | Sources                       |
+=================================================================+
| +D  | backlinks-done            | Graph Queries:11              |
| +D  | fast-startup              | Performance:10                |
| +D  | forward-links-done        | Graph Queries:10              |
| +d  | release-candidate         | (derived)                     |
+-----+---------------------------+-------------------------------+
```

Want to know *why* `release-candidate` holds? Ask for the proof tree:

```bash
zetl -d ./demo-vault reason explain "release-candidate" --format natural
```

## That's the loop

1. `zetl index` to scan.
2. `zetl links` / `zetl backlinks` / `zetl search` to query.
3. `zetl serve` or `zetl view` to read.
4. `zetl reason` to draw conclusions (if you've enabled that feature).

Next up: set up your own vault in [[Your First Vault]], or skim every command in the [[CLI Overview]].

## Related

- [[Your First Vault]]
- [[CLI Overview]]
- [[Installation]]
- [[Searching]]
- [[Backlinks]]
