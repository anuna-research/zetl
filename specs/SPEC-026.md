---
title: "SPEC-026: AST-Aware Graph Compression — Token-Efficient Vault Export for LLM Consumption"
version: 0.1.0
status: draft
date: 2026-04-07
audience: agent, human
parent: SPEC-001
related:
  - SPEC-002  # Full-text search (shared heading-detection infrastructure)
  - SPEC-018  # Semantic search (complementary retrieval path)
  - SPEC-021  # MCP server (primary consumer of compressed output)
dependencies:
  - pulldown-cmark (existing; markdown AST event stream)
---

# SPEC-026: AST-Aware Graph Compression

Adds a compact, structurally faithful export format that compresses vault pages into a token-efficient representation readable by any LLM. Where raw markdown wastes context window on formatting, whitespace, and prose filler, AST-aware compression preserves the page's semantic skeleton — headings, wikilinks, lists, code references — at roughly 10–30x fewer tokens.

## 1. Motivation

### 1.1 The Context Window Problem

When an LLM asks "what's connected to this page?" via the MCP server (SPEC-021), the system must choose what to return. The extremes are both bad:

- **Raw markdown**: Faithful but expensive. A 2,000-word page consumes ~2,500 tokens. Returning a page and its 8 neighbours costs ~22,000 tokens — over 10% of a typical context window for a single query.
- **Page titles only**: Cheap but useless. `["OAuth Flow", "Session Management", "Clerk Setup"]` tells the LLM nothing about *what* those pages contain or *why* they're linked.

The middle ground is a compressed representation that preserves structural semantics — what the page is about, how it's organised, what it links to, and what code it references — while discarding prose filler, redundant formatting, and inline noise.

### 1.2 Why AST-Aware, Not Keyword Heuristic

MemPalace's AAAK dialect (see `mempalace/dialect.py`) achieves 30x compression using keyword matching on flat text. This works for conversational memories but fails on structured documents because it cannot distinguish a heading from a paragraph, a wikilink from a plain word, or a code block from prose. The result is a bag of keywords with inferred flags — lossy and structurally blind.

zetl already parses markdown via `pulldown-cmark`. The AST event stream provides typed nodes (headings, paragraphs, links, code blocks, lists, tables, block quotes, images) with zero ambiguity. An AST-walking compressor exploits this structure directly:

1. **Headings are free labels** — they *are* the topic hierarchy, no inference needed.
2. **Wikilinks are first-class graph edges** — `[[page]]` is already explicit, not reverse-engineered from metadata.
3. **Code blocks get special treatment** — preserved as technical signal, not stripped as noise.
4. **Lists compress naturally** — already compact, just flatten.
5. **Frontmatter becomes a metadata header** — tags, aliases, dates for free.

### 1.3 User Stories

**MCP context retrieval**: A developer asks Claude "what do my notes say about authentication?" The MCP server runs a search, identifies 5 relevant pages, and needs to return their content. With raw markdown, that's ~12,000 tokens. With AST compression, it's ~800 tokens — the LLM gets the same structural understanding at 1/15th the cost.

**Graph neighbourhood export**: `zetl export --compact auth-design` outputs the page and its forward/back links in a format an LLM can reason over. Useful for feeding vault context into any AI tool, not just zetl's own MCP server.

**Batch vault summary**: `zetl export --compact --all` produces a whole-vault skeleton. A 500-page vault that would be ~1.2M tokens as raw markdown compresses to ~60K tokens — small enough to fit in a single Claude context window for architectural analysis.

## 2. Requirements

### 2.1 Functional Requirements

| ID | Requirement | Trace |
|----|-------------|-------|
| REQ-120 | `zetl export --compact <page>` SHALL output the AST-compressed representation of a single page to stdout | TEST-144 |
| REQ-121 | `zetl export --compact --neighbours <page>` SHALL output the target page and all pages within 1 hop (forward links + backlinks) in compressed form | TEST-145 |
| REQ-122 | `zetl export --compact --all` SHALL output all vault pages in compressed form | TEST-146 |
| REQ-123 | The compressor SHALL walk the pulldown-cmark event stream and apply node-type-specific compression rules (see §3.1) | TEST-147 |
| REQ-124 | Wikilinks (`[[target]]`) SHALL be emitted as `→ target` in compressed output, preserving the link target exactly | TEST-148 |
| REQ-125 | Dead links (targets absent from the vault index) SHALL be marked with a `✗` prefix: `→ ✗ missing-page` | TEST-149 |
| REQ-126 | Frontmatter (YAML) SHALL be emitted as a compact metadata header when present | TEST-150 |
| REQ-127 | The compressed format SHALL be deterministic: identical input always produces identical output | TEST-151 |
| REQ-128 | Multi-page output SHALL include a graph summary section listing all inter-page links as `page-a → page-b` edges | TEST-152 |
| REQ-129 | The `--format json` flag SHALL wrap the compressed output in a JSON envelope with `page`, `compressed`, and `token_estimate` fields | TEST-153 |
| REQ-130 | The MCP server (SPEC-021) SHALL expose a `zetl_compress` tool that returns the compressed representation for a page or neighbourhood | TEST-154 |

### 2.2 Non-Functional Requirements

| ID | Attribute | Criterion |
|----|-----------|-----------|
| NFR-047 | Compression ratio | Compressed output SHALL be ≤ 15% of raw markdown token count for pages ≥ 200 words, measured by whitespace-split word count as token proxy |
| NFR-048 | Latency | Single-page compression SHALL complete in ≤ 5ms on a vault of 1,000 pages (excludes I/O) |
| NFR-049 | Readability | Compressed output SHALL be interpretable by a human reading it for the first time, without a format reference |
| NFR-050 | Fidelity | Every wikilink, heading, and code block identifier present in the source markdown SHALL be present in the compressed output (no structural information loss) |

## 3. Architecture

### 3.1 Compression Rules by AST Node Type

The compressor walks `pulldown-cmark::Event` variants and applies the following rules:

| AST Node | Compression Rule | Example Input | Example Output |
|----------|-----------------|---------------|----------------|
| `Start(Heading(n))` | Emit as `#`×n prefix, keep text verbatim | `## Authentication` | `## Authentication` |
| `Text` (in paragraph) | Strip stop words, keep nouns/verbs/adjectives; preserve sentences containing wikilinks verbatim | `We decided to use Clerk because of pricing and developer experience.` | `decided use Clerk — pricing, developer experience` |
| `Start(Link)` with `[[target]]` | Emit as `→ target` | `[[OAuth Flow]]` | `→ OAuth Flow` |
| `Start(List)` | Flatten items to comma-separated single line | `- token refresh\n- session management\n- CSRF protection` | `• token refresh, session management, CSRF protection` |
| `Start(CodeBlock)` | Emit language tag + first line + ellipsis if >1 line | ````rust\nfn compress(ast: &[Event]) -> String {\n    ...\n}\n```` | `⌜rust: fn compress(ast: &[Event]) -> String { … }⌝` |
| `Start(BlockQuote)` | Emit `>` + first sentence, truncate | `> This is a long quotation that spans multiple sentences. It contains important context.` | `> This is a long quotation that spans multiple sentences.` |
| `Start(Table)` | Emit header row + row count | A 3-column, 10-row table | `┃ Col1 │ Col2 │ Col3 ┃ (10 rows)` |
| `SoftBreak` / `HardBreak` | Collapse to single space | Multiple line breaks | Single space |
| `Start(Image)` | Emit alt text only | `![architecture diagram](img.png)` | `[img: architecture diagram]` |
| Frontmatter (YAML) | Emit as `@ key: value` lines, skip internal-only keys | `tags: [auth, security]\naliases: [authn]` | `@ tags: auth, security\n@ aliases: authn` |
| `Html` | Strip entirely | `<div class="note">` | *(omitted)* |
| `TaskListMarker` | Preserve as `☐`/`☑` | `- [ ] thing\n- [x] done` | `☐ thing, ☑ done` |

### 3.2 Paragraph Compression Strategy

Paragraph compression is the highest-impact rule and the most nuanced. The strategy has three tiers:

1. **Link-bearing sentences**: Any sentence containing a `[[wikilink]]` is preserved verbatim (the surrounding prose provides link context that is valuable to the LLM).
2. **Signal sentences**: Sentences containing decision markers ("decided", "chose", "because", "instead of", "trade-off") or technical terms (detected via code-block vocabulary in the same page) are preserved verbatim.
3. **Filler sentences**: All other sentences are reduced to their noun phrases and key verbs, with stop words removed.

This avoids the mempalace failure mode of keyword-matching on flat text — the AST tells us *where* we are (heading vs paragraph vs list), and sentence-level heuristics tell us *what matters*.

### 3.3 Multi-Page Graph Summary

When compressing multiple pages (`--neighbours` or `--all`), the output begins with a graph summary block:

```
=== GRAPH (5 pages, 12 edges) ===
auth-design → OAuth Flow, Session Management, Clerk Setup
OAuth Flow ← auth-design, API Gateway → Token Refresh
Session Management ← auth-design → Redis Config, TTL Policy
Clerk Setup ← auth-design
API Gateway → OAuth Flow, Rate Limiting
```

This gives the LLM the full topology before it encounters individual page content. The format uses `→` for forward links and `←` for backlinks, with pages listed in alphabetical order.

### 3.4 Output Format

Single-page output:

```
=== auth-design ===
@ tags: auth, security
@ aliases: authn

## Authentication Design

decided use Clerk — pricing, developer experience
→ OAuth Flow handles token exchange
→ Session Management for stateful clients

### Token Strategy
• JWT for API, session cookie for web, refresh token rotation
⌜rust: fn issue_token(claims: &Claims) -> Result<String> { … }⌝

### Open Questions
☐ CSRF protection strategy, ☑ provider selection
```

Multi-page output wraps each page in `=== page-name ===` delimiters, preceded by the graph summary.

### 3.5 Architecture Diagram

```
                     ┌──────────────────────────────┐
                     │         CLI / MCP             │
                     │  zetl export --compact <page> │
                     │  zetl_compress MCP tool       │
                     └──────────┬───────────────────┘
                                │
                                ▼
                     ┌──────────────────────────────┐
                     │       CompressedExport        │
                     │  orchestrates multi-page      │
                     │  builds graph summary         │
                     └──────────┬───────────────────┘
                                │
                    ┌───────────┴───────────┐
                    ▼                       ▼
          ┌─────────────────┐    ┌──────────────────┐
          │  PageCompressor │    │    LinkGraph      │
          │  walks AST,     │    │  forward_links()  │
          │  applies rules  │    │  backlinks()      │
          └────────┬────────┘    └──────────────────┘
                   │
                   ▼
          ┌─────────────────┐
          │  pulldown-cmark  │
          │  Event stream    │
          └─────────────────┘
```

## 4. Architecture Decisions

### ADR-060: AST-Walking vs Regex Compression

**Context**: MemPalace's AAAK dialect uses regex keyword matching on flat text. We need to choose between adapting that approach and building on the pulldown-cmark AST.

**Decision**: Walk the pulldown-cmark event stream directly.

**Rationale**:
- zetl already depends on pulldown-cmark; no new dependencies.
- The AST provides exact node boundaries — no false positives from regex ("decided" in a code comment vs in prose).
- Headings, code blocks, and wikilinks are first-class AST nodes — no heuristic detection needed.
- Deterministic: same AST always produces the same output.

**Trade-offs**:
- (+) Structurally faithful — preserves document hierarchy.
- (+) No false-positive flags — "architecture" in a heading is a heading, not a `TECHNICAL` flag.
- (+) Composable with existing zetl infrastructure (link_map, graph).
- (−) Requires handling all pulldown-cmark event variants; AAAK only handles flat text.
- (−) Paragraph compression still needs a stop-word / signal-word heuristic layer on top of the AST.

**Rejected alternative**: Port AAAK's keyword-matching approach. Rejected because it discards structural information (headings, code blocks, link context) and produces an opaque format requiring a reference to decode.

### ADR-061: Stop-Word Removal vs Extractive Summarisation

**Context**: Paragraphs need compression. Options: (a) remove stop words and keep content words, (b) extract key sentences, (c) use an LLM to summarise.

**Decision**: Tiered approach — preserve link-bearing and signal sentences verbatim; reduce filler sentences via stop-word removal.

**Rationale**:
- LLM summarisation adds latency, cost, and a runtime dependency — violates zetl's local-first principle.
- Pure extractive summarisation (pick top-N sentences) loses the surrounding context of wikilinks.
- Stop-word removal on all sentences loses too much context around links.
- The tiered approach preserves what matters (links + decisions) and compresses what doesn't.

**Trade-offs**:
- (+) Zero external dependencies; runs in <1ms per page.
- (+) Deterministic — no model variance.
- (−) Filler-sentence compression is lossy in a way that occasionally drops useful nuance.
- (−) Signal-word list needs curation; false positives possible.

### ADR-062: Inline Link Notation (`→ target`) vs Collected Link Section

**Context**: Wikilinks can be emitted inline (where they appear in the source) or collected into a links section at the end.

**Decision**: Inline `→ target` notation, preserving the link's position in the document structure.

**Rationale**:
- A link's *context* matters. `→ OAuth Flow handles token exchange` tells the LLM why the link exists. A collected `Links: OAuth Flow, Session Management` section does not.
- The graph summary block (§3.3) already provides the collected view for multi-page output.
- Inline notation mirrors how humans write — links appear where they're relevant.

**Trade-offs**:
- (+) Context-preserving — the LLM knows *why* each link exists.
- (+) Reads naturally.
- (−) Links scattered through the document are harder to enumerate by scanning; mitigated by the graph summary.

## 5. Contracts

### CON-037: CLI `zetl export --compact`

**Interface**:

```
zetl export --compact <page> [--neighbours] [--all] [--depth N] [--format json|text]
```

| Flag | Default | Description |
|------|---------|-------------|
| `<page>` | *(required unless `--all`)* | Page name or path |
| `--neighbours` | `false` | Include pages within `--depth` hops |
| `--depth` | `1` | Hop radius for `--neighbours` |
| `--all` | `false` | Compress entire vault |
| `--format` | `text` | Output format: `text` (raw compressed) or `json` (envelope) |

**Pre-conditions**:
- Vault root is discoverable (`.zetl/` exists in parent hierarchy).
- If `<page>` is given, it must resolve to an existing page in the vault index.

**Post-conditions**:
- Stdout contains the compressed representation.
- Exit code 0 on success; non-zero on error with diagnostic on stderr.

**Error model**:
- Page not found → exit 1, stderr: `error: page not found: <name>`
- No vault root → exit 1, stderr: `error: not inside a zetl vault`

**JSON envelope** (`--format json`):

```json
{
  "pages": [
    {
      "name": "auth-design",
      "compressed": "## Authentication Design\ndecided use Clerk…",
      "token_estimate": 142,
      "raw_token_estimate": 2480
    }
  ],
  "graph": "auth-design → OAuth Flow, Session Management\n…",
  "total_token_estimate": 892
}
```

`token_estimate` uses whitespace-split word count as a proxy (no tokeniser dependency).

Implements: REQ-120, REQ-121, REQ-122, REQ-128, REQ-129
Verified by: TEST-144, TEST-145, TEST-146, TEST-152, TEST-153

### CON-038: MCP Tool `zetl_compress`

**Interface** (JSON-RPC, per SPEC-021 MCP transport):

```json
{
  "name": "zetl_compress",
  "description": "Return AST-compressed representation of vault pages for token-efficient LLM consumption",
  "inputSchema": {
    "type": "object",
    "properties": {
      "page": { "type": "string", "description": "Page name" },
      "neighbours": { "type": "boolean", "default": false },
      "depth": { "type": "integer", "default": 1, "minimum": 1, "maximum": 3 }
    },
    "required": ["page"]
  }
}
```

**Response**: JSON envelope identical to CON-037's `--format json` output.

**Error model**: Standard MCP JSON-RPC error codes per CON-036 (SPEC-021).

Implements: REQ-130
Verified by: TEST-154

## 6. Purity Boundary Map

### Pure Core (no I/O, no shared state, deterministic)

- `compress_page(events: &[Event], links: &LinkMap, dead_links: &HashSet<String>) → String` — walks the AST event stream, applies compression rules, returns compressed text.
- `compress_paragraph(text: &str, has_wikilink: bool) → String` — tiered paragraph compression: verbatim if link-bearing/signal, stop-word-reduced otherwise.
- `build_graph_summary(pages: &[(String, Vec<String>, Vec<String>)]) → String` — formats the multi-page graph header from page names + forward/back link lists.
- `estimate_tokens(text: &str) → usize` — whitespace-split word count.

### Effectful Shell (orchestrates I/O, calls pure core)

- `cmd_export_compact(page, neighbours, all, depth, format)` — reads files from disk, builds LinkGraph, calls pure core, writes to stdout.
- `mcp_zetl_compress(request)` — MCP handler; reads vault, calls pure core, formats JSON-RPC response.

### Boundary Contracts

- `Vec<pulldown_cmark::Event>` flows from shell (file read + parse) → core.
- `LinkMap` and `HashSet<String>` (dead links) flow from shell (graph lookup) → core.
- `String` (compressed text) flows from core → shell (stdout/JSON-RPC).

### Dependency Rule

`compress` module imports from `pulldown_cmark` and `view::link_map` (for `LinkEntry`). It does NOT import from `main`, `graph`, `web`, or any I/O module.

### Enforcement

- `compress.rs` (or `src/compress/mod.rs`) contains only pure functions.
- `cmd_export_compact` lives in `src/main.rs` alongside other CLI commands.
- MCP handler lives in `src/mcp.rs` (SPEC-021).
- Integration tests verify the pure core with synthetic AST events, no file I/O.

## 7. Observability

| ID | Signal | Type | Condition |
|----|--------|------|-----------|
| OBS-021 | `zetl.compress.ratio` | metric | Ratio of compressed tokens to raw tokens; emitted per page |
| OBS-022 | `zetl.compress.duration_ms` | metric | Wall-clock time for single-page compression |
| OBS-023 | `zetl.compress.pages` | metric | Number of pages compressed in a single invocation |

Signals are emitted to stderr when `--verbose` is set, consistent with existing zetl CLI conventions.

## 8. Test Specifications

### TEST-144: Single-Page Compression

**Requirement**: REQ-120, REQ-123
**Type**: Unit
**Preconditions**: A markdown string with headings, paragraphs, wikilinks, a code block, and a list.
**Steps**:
1. Parse markdown with pulldown-cmark.
2. Call `compress_page()`.
3. Assert output contains all headings verbatim.
4. Assert output contains `→ target` for each wikilink.
5. Assert output contains `⌜lang:` for the code block.
6. Assert output contains `•` for the flattened list.
7. Assert no raw HTML in output.

### TEST-145: Neighbourhood Compression

**Requirement**: REQ-121, REQ-128
**Type**: Integration
**Preconditions**: A vault with page A linking to pages B and C; B links back to A.
**Steps**:
1. Run `zetl export --compact A --neighbours`.
2. Assert output contains `=== A ===`, `=== B ===`, `=== C ===` sections.
3. Assert graph summary shows `A → B, C` and `B → A`.

### TEST-146: Full Vault Compression

**Requirement**: REQ-122
**Type**: Integration
**Preconditions**: A vault with ≥ 3 pages.
**Steps**:
1. Run `zetl export --compact --all`.
2. Assert output contains one `=== page ===` section per vault page.
3. Assert graph summary lists all inter-page edges.

### TEST-147: Node-Type Coverage

**Requirement**: REQ-123
**Type**: Unit (property-based)
**Property**: For every pulldown-cmark `Event` variant that can appear in valid markdown, `compress_page` either emits a non-empty compressed representation or explicitly omits it (for `Html`, `SoftBreak`). No variant causes a panic or is silently ignored.
**Approach**: Generate random valid markdown via a markdown fuzzer; parse; compress; assert no panics and output is valid UTF-8.

### TEST-148: Wikilink Preservation

**Requirement**: REQ-124, NFR-050
**Type**: Unit
**Steps**:
1. Input: `"Check [[OAuth Flow]] and [[Session Management|sessions]] for details."`
2. Compress.
3. Assert output contains `→ OAuth Flow` and `→ Session Management`.
4. Assert both link targets are present exactly once.

### TEST-149: Dead Link Marking

**Requirement**: REQ-125
**Type**: Unit
**Steps**:
1. Input contains `[[Nonexistent Page]]`.
2. Dead-link set contains `"Nonexistent Page"`.
3. Assert output contains `→ ✗ Nonexistent Page`.

### TEST-150: Frontmatter Compression

**Requirement**: REQ-126
**Type**: Unit
**Steps**:
1. Input: markdown with YAML frontmatter containing `tags`, `aliases`, `date`.
2. Assert output starts with `@ tags:`, `@ aliases:`, `@ date:` lines.
3. Assert internal keys (if any defined, e.g., `zetl-internal`) are omitted.

### TEST-151: Determinism

**Requirement**: REQ-127
**Type**: Unit (property-based)
**Property**: `compress_page(events) == compress_page(events)` for all valid event sequences.
**Approach**: Generate 100 random markdown documents; compress each twice; assert byte-identical output.

### TEST-152: Graph Summary Correctness

**Requirement**: REQ-128
**Type**: Unit
**Steps**:
1. Construct a 4-page graph: A→B, A→C, B→A, D→A.
2. Call `build_graph_summary()`.
3. Assert output contains `A → B, C` (forward links sorted).
4. Assert output contains `B → A`.
5. Assert output contains `D → A`.
6. Assert header shows `4 pages, 4 edges`.

### TEST-153: JSON Envelope

**Requirement**: REQ-129
**Type**: Integration
**Steps**:
1. Run `zetl export --compact A --format json`.
2. Parse stdout as JSON.
3. Assert `pages[0].name == "A"`.
4. Assert `pages[0].compressed` is a non-empty string.
5. Assert `pages[0].token_estimate` is a positive integer.
6. Assert `pages[0].token_estimate < pages[0].raw_token_estimate`.

### TEST-154: MCP Tool Integration

**Requirement**: REQ-130
**Type**: Integration
**Steps**:
1. Send `tools/call` JSON-RPC request with `name: "zetl_compress"`, `arguments: { "page": "A" }`.
2. Assert response contains `content[0].text` with valid JSON envelope.
3. Assert envelope matches CON-038 schema.

### Verification Strategy

| Technique | Scope |
|-----------|-------|
| Example-based testing | All TEST-144 through TEST-154 |
| Property-based testing | TEST-147 (node-type coverage), TEST-151 (determinism) |
| Mutation testing | `compress_page` and `compress_paragraph` — kill rate ≥ 90% on compression rules |

## 9. Future Considerations

- **Configurable compression depth**: Allow users to set aggressiveness (e.g., `--compact=deep` strips more prose, `--compact=light` preserves more context). Current spec defines a single fixed level.
- **SPL block awareness**: zetl pages can contain embedded SPL (defeasible logic) blocks. A future revision could compress these using SPL-specific rules rather than treating them as code blocks.
- **Streaming compression**: For very large vaults, compress pages as they're scanned rather than loading all into memory. Current spec assumes the vault fits in memory (consistent with existing zetl assumptions).
- **Embedding-aware compression**: If SPEC-018 (semantic search) is implemented, compression could prioritise sentences that are semantically distant from the page's heading (high-information sentences) over those that are semantically redundant with it.
- **Custom stop-word lists**: Allow per-vault stop-word configuration for domain-specific terminology that should never be stripped.

## 10. Relationship to Existing Specs

- **SPEC-001** (Link Graph CLI): Provides the `LinkGraph` with `forward_links()` and `backlinks()` used for `--neighbours` and graph summary generation.
- **SPEC-002** (Full-Text Search): Shares the heading-detection infrastructure (`detect_headings`). Compressed output could be indexed for search in a future revision.
- **SPEC-018** (Semantic Search): Complementary — semantic search finds relevant pages, compression makes returning their content token-efficient.
- **SPEC-021** (MCP Server): Primary consumer. The `zetl_compress` MCP tool (CON-038) builds on SPEC-021's transport, auth, and error-handling infrastructure.
