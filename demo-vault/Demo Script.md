# Demo Script

## What is zetl?

zetl is a CLI that turns your Markdown notes into a queryable knowledge graph. Point it at a folder of [[Wikilinks|wikilink]]-connected files — Obsidian, Logseq, Foam, or plain Markdown — and it gives you [[Graph Queries|graph queries]], [[Vault Diagnostics|validation]], [[Search|full-text search]], and a web UI, all from a single binary.

It's designed LLM-agent-first. Every command outputs structured [[JSON by Default|JSON]] with deterministic schemas, non-zero exit codes on failure, and structured error objects — so an AI agent can call `zetl search`, `zetl check`, or `zetl links` and parse the results without scraping human-formatted text. Humans get the same data with `--format table`.

## Vault scan

`zetl index` scans your Markdown files and builds a [[Link Graph]] in under a second. It finds every [[Wikilinks|wikilink]], resolves targets, and caches results so subsequent runs are instant.

```sh
zetl index
```

25 pages, 157 links — ready to query.

## Graph queries

[[Graph Queries]] are the core of zetl. Ask what a page links to, who links back, or find the shortest path between any two pages.

```sh
zetl links Scanner --depth 2
zetl backlinks "Reasoning Engine"
zetl path Cache "Spindle Lisp"
```

`zetl links` with `--depth 2` gives you the full two-hop neighbourhood around [[Scanner]]. `zetl backlinks` on [[Reasoning Engine]] returns 19 pages. `zetl path` finds the shortest route from [[Cache]] to [[Spindle Lisp]].

## Validation

[[Vault Diagnostics]] catch problems before they compound. `zetl check` audits the whole vault — dead links, orphan pages, ambiguous targets.

```sh
zetl check
```

It found one dead link: Cache references a page called "Plugin System" that doesn't exist. It also flags orphan pages with no backlinks.

## Search

[[Search]] is powered by Tantivy full-text indexing. It returns the matching page, the heading it falls under, a context snippet, a relevance score, and a line number you can jump to.

```sh
zetl search "defeasible reasoning"
```

Results link directly to the matching line in the web UI.

## Web UI

`zetl serve` launches a local web app. You get a Cmd+K search modal, rendered Markdown with backlinks, and transclusion cards that preview every page you link to. Hover a wikilink and a bridge line connects it to its card in the sidebar.

```sh
zetl serve
```

The search modal works in two modes — Tantivy full-text in serve mode, client-side BM25 in static builds. Both return headings, context, and line anchors.

## Themes

Drop a folder into `.zetl/themes/` and override any template — `base.html`, `page.html`, `folder.html`. Pass `--theme` to serve or build and zetl loads your custom templates, falling back to built-in defaults for anything you haven't overridden. Static assets go in the theme's `static/` folder.

```sh
zetl serve --theme ocean
zetl build --out-dir dist --theme ocean
```

This is a [[Local-first Design]] decision — themes live in your vault, version-controlled alongside your notes.

## Static build

`zetl build` generates a fully static HTML site. Every link is relative, the search index is embedded inline — open it from disk or deploy it anywhere.

```sh
zetl build --out-dir dist
```

The output works on `file://` protocol and deploys to any static host. This demo vault is live on Cloudflare Pages.

## What's under the hood

zetl is written in [[Rust for CLI|Rust]] for speed and ships as a single binary. Key architectural choices:

- [[Scanner]] parses wikilinks with an incremental two-tier cache (mtime + BLAKE3 hash)
- [[Link Graph]] stores the bidirectional graph in memory for fast traversal
- [[Reasoning Engine]] runs [[Defeasible Reasoning]] over [[Spindle Lisp]] code blocks with full [[Provenance]] tracking
- [[JSON by Default]] keeps CLI output agent-friendly — pipe it to `jq`, feed it to an LLM, or read it as a human with `--format table`
