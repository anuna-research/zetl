# Quick Start

A five-step tour of zetl's core capabilities. Assumes you've already run [[Install]].

## Step 1: Build the index

```bash
zetl -d ./demo-vault index
```

This scans all Markdown files, extracts [[concepts/Wikilinks]] and [[concepts/Spindle Lisp]] blocks, and builds the link graph. The index is cached in `.zetl/` for fast subsequent runs — see [[architecture/Cache]].

## Step 2: Explore stats

```bash
zetl -d ./demo-vault stats --format table
```

Shows page count, link count, orphan count, and the most-linked pages. All commands output JSON by default — add `--format table` for human-readable output. See [[Stats Command]].

## Step 3: Validate the vault

```bash
zetl -d ./demo-vault check --format table
```

Reports dead links, orphan pages, syntax errors, and SPL diagnostics. See [[Check Command]].

## Step 4: Launch the TUI

```bash
zetl -d ./demo-vault tui
```

An interactive terminal interface with seven views: dashboard, pages, links, search, diagnostics, page reader, and graph. Navigate with `Tab`, filter with `/`, follow wikilinks with `Enter`. See [[TUI]].

## Step 5: Run reasoning

```bash
zetl -d ./demo-vault reason status --format table
zetl -d ./demo-vault reason explain "release-candidate" --format natural
```

Extracts SPL from every page, merges it into a vault-wide theory, and derives conclusions. The `explain` command shows a proof tree traced back to source files. See [[Reason Commands]].

## Next steps

- [[Demo Vault]] — explore the included example vault
- [[CLI Reference]] — full flag and command reference
- [[View Command]] — Xanadu-inspired two-pane reader
- [[Serve Command]] — browse the vault as a local website

See also: [[Install]], [[Compatibility]]
