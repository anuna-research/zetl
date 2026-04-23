---
title: Glossary
tags: [reference, glossary, terminology]
---

# Glossary

Every term used across the ztl guide, in one alphabetised list. Entries link to the primary page where the concept is explained in depth.

## A

**AST** — Abstract syntax tree. ztl uses a canonical schema (*ztl-ext AST v1*) that every render-pipeline stage receives and returns. Inspect with `ztl ast sample`. See [[Render Pipeline Hooks]].

## B

**Backlink** — An inbound wikilink. If page *A* contains `[[B]]`, then *B* has a backlink from *A*. Surface with `ztl backlinks`. See [[Backlinks]].

**BLAKE3** — The cryptographic hash ztl uses for content-addressable blocks and the Merkle leaves that feed the incremental cache. Fast, parallelisable, keyed. See [[Blocks]].

**Block** — A content-addressable unit within a page: a heading, paragraph, code fence, table, blockquote, list, SPL block, or frontmatter. Each block has a BLAKE3 hash. See [[Blocks]].

## C

**Capability URL** — A sharing link with an embedded fragment secret (`#k=…`) that decrypts a static-built vault client-side. No server required. See [[Capability URLs]].

**CRDT** — Conflict-free replicated data type. ztl uses Peritext for real-time co-editing so two authors can type into the same paragraph without conflicts or last-writer-wins. See [[Co-editing]].

## D

**Dead link** — A wikilink that points at a nonexistent page. `ztl check --dead-links` lists them. See [[Finding Orphans and Dead Links]].

**Defeasible logic** — A logic where conclusions can be revoked when stronger evidence arrives: "birds fly; penguins don't; Tweety is a penguin" correctly concludes Tweety doesn't fly. ztl's optional reasoning layer is built on this. See [[What is Defeasible Reasoning]].

## E

**Ecosystem** — An external render toolchain ztl can plug into: Pandoc, mdBook, or remark. Each is a set of render-pipeline adapters gated behind a cargo feature flag (`ecosystem-pandoc`, `ecosystem-mdbook`, `ecosystem-remark`). See [[Plugin Ecosystems]].

## F

**Forward link** — An outbound wikilink. If page *A* contains `[[B]]`, *A* has a forward link to *B*. Surface with `ztl links`. See [[Following Links]].

**Frontmatter** — YAML metadata fenced between `---` lines at the top of a page. ztl reads a handful of reserved keys (`title`, `tags`, `parser`, `description`) and passes the rest through to templates. See [[Frontmatter Fields]].

## H

**Hook** — An executable ztl invokes at a lifecycle point (`pre-build`, `post-build`, `on-save`) or inside the render pipeline (`pre-parse`, `transform`, `post-render`). Hooks live under `.ztl/hooks/` in the vault or `hooks/` in a theme. See [[Lifecycle Hooks]], [[Render Pipeline Hooks]].

## J

**jj (Jujutsu)** — The VCS engine that backs ztl's silent snapshots when `--features history` is enabled. Commits happen in `.ztl/jj/` without touching your git repo. See [[Snapshots Under the Hood]].

## M

**MCP** — Model Context Protocol. A standard for exposing structured tool access to LLM agents. `ztl mcp` serves the vault as an MCP endpoint. See [[MCP Server]].

**Merkle tree** — A hash tree over vault blocks. Changing one paragraph only invalidates the hashes along its root-path, so ztl's cache reparses just that page and its consumers. See [[Blocks]].

## O

**Orphan** — A page with no inbound *and* no outbound wikilinks. Either write one, link to it, or delete it. See [[Finding Orphans and Dead Links]].

## P

**Passkey** — A WebAuthn credential — Touch ID, Windows Hello, a hardware key like a YubiKey. ztl's multi-user mode uses passkeys so there are no passwords in the vault. See [[Passkeys and Accounts]].

## S

**SimHash** — A locality-sensitive hash that produces similar bit-strings for similar inputs. `ztl similar` uses it to find title matches within a Hamming-distance threshold. See [[Similar Pages]].

**Slug** — The URL-safe version of a page title, used in paths on the built site. `Zettelkasten Method.md` becomes the slug `zettelkasten-method`.

**SPA shell** — Single-page-application navigation shell used by the web server and built site. Same-origin clicks swap only the `<main data-ztl-volatile>` region, keeping the sidebar and graph widget mounted. Opt in via `[spa] enabled = true` in `theme.toml`. See [[Customising the Look]].

**SPL (Spindle Lisp)** — The defeasible-logic language ztl uses for vault rules under `--features reason`. Rules live in fenced code blocks (```` ```spl ````) inside your pages; the reasoner consumes them globally. See [[Writing SPL]].

## T

**Transclusion** — Embedding part of one page inside another. The syntax is `![[Page]]` to embed a whole page, `![[Page#Heading]]` for a section, or `![[Page^block-id]]` for a specific block. See [[Embeds and Transclusion]].

## V

**Vault** — Any directory ztl operates on. No registration, no config file required. A vault becomes *ztl-aware* the first time `ztl index` runs against it and the `.ztl/` cache directory appears. See [[Vaults]].

## W

**WebAuthn** — The W3C standard for passkey authentication. ztl's multi-user mode is a WebAuthn relying-party. See [[Passkeys and Accounts]].

**Wikilink** — A `[[Page Name]]` reference. The foundational link syntax ztl is built around. Variants: `[[Page|Alias]]`, `[[Page#Heading]]`, `[[Page^block-id]]`, `![[Page]]` (transclusion). See [[Wikilinks]].

## Related

- [[Index|ztl User Guide]]
- [[CLI Overview]]
- [[Configuration]]
- [[Frontmatter Fields]]
