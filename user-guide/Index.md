---
title: ztl — Quick Start
tags: [tutorial, getting-started, home]
---

# ztl — Quick Start

**ztl** is a command-line tool that turns a folder of Markdown files with `[[wikilinks]]` into a queryable knowledge graph — with optional defeasible reasoning, vault history, collaborative editing, and an MCP server for AI agents. Your files stay as they are; everything ztl generates lives in a disposable `.ztl/` cache next to them.

Source: <https://codeberg.org/anuna/ztl>. This guide is itself a ztl vault — every page you read is a `.md` file in this folder.

---

## 1. Install

**macOS / Linux:**

```bash
curl -fsSL https://files.anuna.io/ztl/latest/install.sh | bash
```

**Windows:** download [ztl-windows-x86_64.zip](https://files.anuna.io/ztl/latest/ztl-windows-x86_64.zip), extract `ztl.exe`, add it to your `PATH`.

**Build from source** (requires a [Rust toolchain](https://rustup.rs/)):

```bash
git clone https://codeberg.org/anuna/ztl && cd ztl
make install                                            # core features
cargo install --path . --features "reason,history,mcp"  # everything
```

You get `ztl` on your `$PATH`, a man page, and shell completions. Feature flags, custom install paths: [[Installation]].

## 2. Get the demo vault

The ztl repo ships with `demo-vault/`, a self-referential knowledge base about ztl itself — wikilinks and SPL throughout. From your clone:

```bash
ls demo-vault/
# architecture/  concepts/  decisions/  features/  theories/  Demo Script.md ...
```

## 3. Build the index

```bash
ztl -d ./demo-vault index
```

ztl auto-detects output format: a table in a terminal, JSON when redirected. You'll see something like:

```json
{
  "files_scanned": 27,
  "links_found": 152,
  "dead_links": 2,
  "diagnostics": 0,
  "elapsed_ms": 28
}
```

27 files, 152 wikilinks, 2 dead links. The cache lives in `demo-vault/.ztl/` and is reused on subsequent runs — ztl only reparses files whose mtime or hash changed.

## 4. Follow a link

Every Markdown file that mentions `[[Scanner]]` threads into the graph. Ask what *Scanner* points at:

```bash
ztl -d ./demo-vault links "Scanner"
```

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

Each row is one wikilink, with the line number in the source file.

## 5. Walk backlinks, two hops

Which notes point at `Cache` — and which notes point at *those* notes?

```bash
ztl -d ./demo-vault backlinks "Cache" --depth 2
```

`--depth 2` means "include second-hop citations." Direct backlinks (hop 1) and transitive ones (hop 2) are labelled so you can tell them apart.

## 6. Open the web UI

```bash
ztl -d ./demo-vault serve
# Serving on http://localhost:3000
```

You get a sidebar with every page, rendered Markdown, a backlink list, a transclusion panel showing excerpts of forward-linked pages, and a mini link-graph widget bottom-right. Edits save straight back to the `.md` file on disk. Ctrl-C stops the server.

## 7. Reason over the vault

> **Requires `--features reason` at install time.** See [[Installation]].

The demo vault has SPL facts and rules sprinkled through its pages — ztl's reasoning engine pulls them out and draws conclusions.

```bash
ztl -d ./demo-vault reason status
```

```
Theory: 46 facts, 24 rules, 0 defeaters, 0 superiority relations from 18 files

+-----+---------------------------+-------------------------------+
| Tag | Literal                   | Sources                       |
+=================================================================+
| +D  | backlinks-done            | Graph Queries:11              |
| +D  | fast-startup              | Performance:10                |
| +d  | release-candidate         | (derived)                     |
+-----+---------------------------+-------------------------------+
```

Tags: `+D` definitely provable, `-D` definitely not, `+d` defeasibly provable, `-d` defeasibly not. Every conclusion carries a line-numbered provenance trail.

Ask *why* `release-candidate` holds:

```bash
ztl -d ./demo-vault reason explain "release-candidate" --format natural
```

## That's the loop

1. `ztl index` to scan.
2. `ztl links` / `ztl backlinks` / `ztl search` to query.
3. `ztl serve` or `ztl view` to read.
4. `ztl reason` to draw conclusions (if `--features reason`).

---

## Read this guide the same way

Everything above works on this vault too. The vault ships with a bundled `quickstart` theme that redirects `/` straight here, so pass `--theme quickstart` (or use the `Makefile`) when you serve or build:

```bash
ztl -d . serve --theme quickstart          # read the guide in a browser
make serve                                  # same, via the bundled Makefile
ztl -d . view Index                        # two-pane terminal reader
ztl -d . build --theme quickstart --out-dir site  # static HTML for any host
ztl -d . check                             # 54 pages, 550 wikilinks, 0 dead
```

Or open the folder in Obsidian, Logseq, Foam, or Dendron — wikilinks work everywhere.

---

## Where to go next

### Start here if you're new
- [[What is ztl]] · [[Your First Vault]] · [[Migrating from Obsidian]]

### Core concepts
- [[Vaults]] · [[Wikilinks]] · [[The Link Graph]] · [[Blocks]] · [[Frontmatter]] · [[Local-first]] · [[What is SPL]] · [[Intellectual Heritage]] · [[LLM Wikis]]

### Writing
- [[Writing Pages]] · [[Linking Pages]] · [[Embeds and Transclusion]] · [[Headings and Blocks]] · [[Organising Your Vault]] · [[Tags and Frontmatter]]

### Finding
- [[Searching]] · [[Backlinks]] · [[Similar Pages]] · [[Following Links]] · [[Finding Orphans and Dead Links]]

### Reading & rendering
- [[Terminal Viewer]] · [[Web Server]] · [[Static Site Export]] · [[Customising the Look]]

### Vault history — requires `--features history`
- [[Time Travel]] · [[Watching for Changes]] · [[Page History]] · [[Snapshots Under the Hood]]

### Reasoning — requires `--features reason`
- [[What is Defeasible Reasoning]] · [[Writing SPL]] · [[Running Queries]] · [[Proof Trees]] · [[What-if and Abduction]]

### Collaboration
- [[Running a Team Server]] · [[Passkeys and Accounts]] · [[Invitations]] · [[Co-editing]] · [[Access Control]] · [[Capability URLs]]

### Automation & extensibility
- [[Lifecycle Hooks]] · [[Render Pipeline Hooks]] · [[Plugin Ecosystems]] · [[MCP Server]]

### Reference
- [[CLI Overview]] · [[Configuration]] · [[Frontmatter Fields]] · [[Glossary]]

### Help
- [[Troubleshooting]] · [[FAQ]]
