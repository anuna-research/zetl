---
title: "SPEC-013: ztl search — Tantivy Full-Text Search, Graph Scoping, and Browser Search"
version: 0.3.0
status: draft
audience: agent, human
date: 2026-03-01
---

# SPEC-013: ztl search — Tantivy Full-Text Search, Graph Scoping, and Browser Search

## Information Table

| Field          | Value                                                          |
| -------------- | -------------------------------------------------------------- |
| Document ID    | SPEC-013                                                       |
| Title          | ztl search — Tantivy Full-Text Search, Graph Scoping, and Browser Search |
| Version        | 0.3.0                                                          |
| Status         | Draft                                                          |
| Author         | Agent (USDD Protocol v1.0.0)                                   |
| Date           | 2026-03-01                                                     |
| Audience       | Agent, Human                                                   |
| Trace          | USDD Agent Protocol v1.0.0                                     |
| Parent         | SPEC-002: ztl search — Full-Text Content Search               |
| Related        | SPEC-001: ztl — Bi-directional Link Graph CLI, SPEC-012: Web Templates |
| Dependencies   | SPEC-001 graph engine, SPEC-002 search, SPEC-012 web, tantivy crate |

---

## 1. Overview

SPEC-002 gave ztl full-text content search via brute-force grep. Results are unranked, searches have no awareness of the link graph, matches report line numbers without structural context, and search is CLI-only. This specification replaces the grep engine with Tantivy and extends search to the browser:

1. **Tantivy-backed search.** All `ztl search` queries go through Tantivy's inverted index. Results are BM25-scored — a search for "algorithm" ranks the dedicated concept page above a journal entry that mentions it in passing. No `--rank` flag; scoring is always on.

2. **Graph-scoped search** (`--near <PAGE> --depth N`) restricts results to pages within N link-hops of a given page — a query natural for graph-structured knowledge but impossible with text search alone.

3. **Heading-aware context** enriches every search result with the heading the match falls under — structural context that helps agents and humans decide whether to read the full page.

4. **Browser search.** In `ztl serve`, a backend API endpoint (`GET /api/search`) queries the same Tantivy index. In `ztl build`, a compact search index is emitted as a static asset alongside a client-side JS BM25 scorer, upgrading the Cmd+K modal from fuzzy page-name matching to full-text ranked search.

### 1.1 Motivation

**Why Tantivy replaces grep.** The SPEC-002 search is a linear scan: read every file, match text, emit results. Adding scoring, tokenization, or term frequency to a grep loop is reimplementing a search engine badly. Tantivy is the standard Rust answer: BM25, tokenization, inverted index, sub-millisecond queries. For a tool that already maintains `.ztl/` caches with invalidation logic, adding a search index is a natural extension. Users who want unranked substring matching can use `grep` or `rg` directly on their vault folder.

**Why graph scoping.** Agents frequently want to search within a topic cluster. An agent writing about "spaced repetition" wants mentions of "recall" in the connected neighbourhood, not in unrelated pages. Today this requires running `ztl links --depth 2`, extracting page names, and filtering manually. A `--near` flag expresses this intent directly.

**Why heading context.** A match on line 45 of a 200-line document is ambiguous. The heading hierarchy answers whether the match is in the introduction, a subsection on caveats, or a footnote.

**Why browser search.** The web UI (serve and build) currently has a Cmd+K fuzzy finder that matches page names only. Full-text search in the browser closes the gap between the CLI and the web interface.

### 1.2 Design Principles

1. **Single search engine.** Tantivy is the only search path. There is no grep fallback. `ztl search` always queries the Tantivy index and returns BM25-scored results.
2. **Index builds alongside the graph.** `ztl index` builds both the link graph cache and the Tantivy search index. Both live in `.ztl/` and share the same invalidation semantics.
3. **Lazy index construction.** If no index exists when `ztl search` is invoked, it is built on the fly. Users are not forced to run `ztl index` first.
4. **Composable flags.** `--near`, heading context, and all existing flags (`--case-sensitive`, `--path`, `--context`, `--limit`) compose orthogonally.
5. **Always-on headings.** Heading context appears on every search result as a nullable field.
6. **Parity across interfaces.** CLI, serve, and build produce the same search results for the same query. The ranking algorithm (BM25) is identical; only the transport differs.

### 1.3 Scope

**In scope:**

- Tantivy-backed search index built during `ztl index`
- BM25 relevance scoring on all `ztl search` results
- `score` field on every search result
- Index invalidation using existing file-mtime cache mechanism
- Lazy index build when no index exists
- Body-text-only indexing (same exclusions as SPEC-002: no frontmatter, code blocks, inline code, HTML comments)
- `--near <PAGE> --depth N` graph-scoped search
- Bidirectional BFS for neighbourhood computation
- `heading` and `heading_level` fields on every search result
- `GET /api/search` endpoint in `ztl serve`
- Cmd+K modal upgrade in serve mode to use full-text search
- Static search index generation in `ztl build`
- Client-side full-text search in built HTML

**Out of scope:**

- Stemming / language-specific tokenizers (defer)
- Phrase queries and boolean queries (Tantivy supports them; exposing the syntax deferred)
- Fuzzy text matching (defer)
- Heading-based filtering (e.g., "search only under `## Design`")
- Setext heading detection
- Graph-scoped search in browser (CLI-only for v1)
- `--regex` flag (removed; Tantivy handles word-level matching; use `rg` for character-level regex)

---

## 2. User Profiles

The existing user profiles from SPEC-001 (section 2) and SPEC-002 (section 2) apply.

### 2.1 Agent Operator — Extended Workflow

```
Daily workflow (updated):
  1. Create/edit markdown files with [[wikilinks]]
  2. Run `ztl index` to build the link graph and search index
  3. Run `ztl search "recall"` to find relevant pages ranked
     by BM25 — the dedicated concept page surfaces above
     journal entries that mention the term in passing
  4. Run `ztl search "recall" --near "Spaced Repetition"` to
     narrow further: only results in the topic neighbourhood
  5. Examine the `heading` field to decide which sections to read
```

### 2.2 Human Knowledge Worker — Extended Workflow

```
Daily workflow (updated):
  1. Write notes in Obsidian/Logseq/editor
  2. Run `ztl serve` or open the built static site
  3. Press Cmd+K, type "emergence" — full-text search returns
     ranked results with heading context, right in the browser
  4. Navigate directly to the most relevant sections
```

---

## 3. Requirements

### 3.1 Functional Requirements — Tantivy Search

```
REQ-013-001: Search Index Construction

The system SHALL build a Tantivy full-text search index during
`ztl index`, indexing the body text of every Markdown file in
the vault,
FOR all user roles
WITH each document in the index containing:
  - page_name: the page name (filename sans .md)
  - path: relative path from vault root
  - body: the body text content (same exclusion zones as
    SPEC-002: no frontmatter, code blocks, inline code, HTML
    comments)
AND the index stored in `.ztl/search/`
AND index construction respecting `.ztlignore` and default
ignore patterns.

Trace:
- TEST-013-001
- CON-013-002
- ADR-013-001
```

```
REQ-013-002: BM25-Scored Search Results

The system SHALL return all `ztl search` results ordered by
descending BM25 relevance score,
FOR all user roles
WITH each result including a `score` field containing the BM25
relevance score as a floating-point number
AND ties broken by path (ascending) then line number (ascending)
AND results limited by `--limit` (default 50).

Trace:
- TEST-013-002
- CON-013-001
```

```
REQ-013-003: Search Index Invalidation

The system SHALL invalidate and rebuild the search index when
file contents change, using the same mtime-based cache
invalidation mechanism as the link graph cache (REQ-011),
FOR all user roles
WITH `--no-cache` forcing a full index rebuild
AND incremental updates: only re-index files whose mtime has
changed since the last index build.

Trace:
- TEST-013-003
```

```
REQ-013-004: Lazy Index Construction

The system SHALL, when `ztl search` is invoked and no search
index exists in `.ztl/search/`, build the index on the fly
before executing the query,
FOR all user roles
WITH a diagnostic message to stderr: "Building search index
(run `ztl index` to avoid this delay on future queries)"
AND the built index persisted for future queries.

Trace:
- TEST-013-004
```

```
REQ-013-005: Line-Level Results with Scores

The system SHALL return per-occurrence results with line and
column positions, consistent with SPEC-002 output schema,
FOR all user roles
WITH the BM25 score applied at the page level (all matches
from the same page share the same score)
AND matches within a page ordered by line number (ascending)
AND per-occurrence positions found by re-scanning the matched
file with the query terms (Tantivy provides document-level
scores, not character offsets).

Trace:
- TEST-013-005
- CON-013-001
```

### 3.2 Functional Requirements — Graph-Scoped Search

```
REQ-013-006: Graph-Scoped Search via --near

The system SHALL, when the `--near <PAGE>` flag is provided to
`ztl search`, restrict search results to only those pages that
are within `--depth N` link-hops of the specified anchor page,
FOR all user roles
WITH the anchor page itself included in the search scope
AND the default depth of 1 (direct neighbours only)
AND the neighbourhood computed via bidirectional BFS (both
outgoing and incoming edges traversed).

Trace:
- TEST-013-006
- CON-013-001
- ADR-013-002
```

```
REQ-013-007: Neighbourhood Depth Control

The system SHALL accept a `--depth N` flag (positive integer,
≥ 1) to control the radius of the graph neighbourhood when
`--near` is specified,
FOR all user roles
WITH an error if `--depth` is used without `--near`
AND an error if `--depth` is zero or negative.

Trace:
- TEST-013-007
- CON-013-001
```

```
REQ-013-008: Anchor Page Resolution

The system SHALL resolve the `--near <PAGE>` argument using
the same page name resolution rules as other graph commands
(SPEC-001 section 3.2: case-insensitive, normalized
whitespace/hyphens/underscores, path-qualified if contains `/`),
FOR all user roles
WITH an error and suggested similar names if the page cannot
be resolved.

Trace:
- TEST-013-008
- CON-013-001
```

```
REQ-013-009: Neighbourhood Metadata in Output

The system SHALL, when `--near` is active, include metadata in
the search output envelope:
  - `near`: the resolved anchor page name
  - `depth`: the depth used
  - `neighbourhood_size`: number of pages in the neighbourhood
FOR all user roles
WITH these fields absent when `--near` is not used.

Trace:
- TEST-013-009
- CON-013-001
```

### 3.3 Functional Requirements — Heading-Aware Context

```
REQ-013-010: Heading-Aware Search Results

The system SHALL include in every search result the text and
level of the nearest preceding ATX heading (lines matching
`^#{1,6}\s`) in the same file,
FOR all user roles
WITH the `heading` field set to the heading text (without
leading `#` characters or trailing whitespace)
AND the `heading_level` field set to the heading depth (1–6)
AND both fields set to null when the match occurs before the
first heading in the file.

Trace:
- TEST-013-010
- CON-013-001
```

```
REQ-013-011: Heading Exclusion Zone Awareness

The system SHALL NOT report headings that fall within exclusion
zones (YAML frontmatter, fenced code blocks, HTML comments) as
the enclosing heading for a match,
FOR all user roles
WITH the heading field reflecting only headings that appear in
body text.

Trace:
- TEST-013-011
```

### 3.4 Functional Requirements — Browser Search (Serve Mode)

```
REQ-013-012: Serve Mode Search API Endpoint

The system SHALL, when running `ztl serve`, expose a
`GET /api/search` endpoint that queries the Tantivy index and
returns JSON results,
FOR all user roles
WITH query parameters:
  - `q` (required): search query string
  - `limit` (optional, default 20): max results
WITH response body matching the CLI SearchOutput schema
  (query, total_matches, results with page, path, line, column,
  context, heading, heading_level, score)
AND the endpoint re-using the in-memory Tantivy index built
during server startup.

Trace:
- TEST-013-012
- CON-013-003
```

```
REQ-013-013: Serve Mode Cmd+K Full-Text Search

The system SHALL upgrade the existing Cmd+K modal in serve mode
to perform full-text search via the `/api/search` endpoint,
FOR all user roles
WITH results displaying page name, heading, context snippet,
  and BM25 score
AND debounced queries (≥ 150ms after last keystroke)
AND keyboard navigation (arrow keys + Enter).

Trace:
- TEST-013-013
```

### 3.5 Functional Requirements — Browser Search (Build Mode)

```
REQ-013-014: Static Search Index Generation

The system SHALL, during `ztl build`, emit a compact search
index file (`search-index.json`) containing the data needed for
client-side BM25 scoring,
FOR all user roles
WITH the index containing for each document:
  - page name, path/slug
  - term frequency map (term → count)
  - document length (total terms)
AND corpus-level statistics:
  - total documents
  - average document length
  - document frequency per term (term → doc count)
AND the file written to the build output directory root.

Trace:
- TEST-013-014
- CON-013-004
- ADR-013-004
```

```
REQ-013-015: Build Mode Client-Side Search

The system SHALL include in the built HTML a client-side search
implementation that loads `search-index.json` and performs BM25
scoring in the browser,
FOR all user roles
WITH the Cmd+K modal upgraded to display full-text ranked
results (page name, score)
AND the search index fetched lazily on first Cmd+K activation
AND results displayed within 100ms of keystroke for indexes
under 10,000 documents.

Trace:
- TEST-013-015
- ADR-013-004
```

### 3.6 Non-Functional Requirements

```
NFR-013-001: Index Build Performance

Search index construction SHALL complete in ≤ 5 seconds for a
vault of 10,000 files (average 5 KB each) UNDER single-threaded
execution on commodity hardware WITH 95th percentile.
```

```
NFR-013-002: Query Latency

A search query against a pre-built index SHALL return results
in ≤ 100ms UNDER a corpus of 10,000 indexed documents WITH
95th percentile.
```

```
NFR-013-003: Index Size

The Tantivy search index SHALL be ≤ 50% of the total vault
size UNDER a corpus of 10,000 files WITH average file size of
5 KB (index ≤ 25 MB for a 50 MB vault).
```

```
NFR-013-004: Neighbourhood Computation Performance

Neighbourhood BFS SHALL complete in ≤ 50ms for a vault of
10,000 pages with average degree 5 at depth 3 UNDER
single-threaded execution WITH 95th percentile.
```

```
NFR-013-005: Heading Detection Overhead

Adding heading detection SHALL increase per-file processing
time by ≤ 10% compared to SPEC-002 baseline UNDER 10,000
files WITH 95th percentile.
```

```
NFR-013-006: Static Search Index Size

The `search-index.json` emitted by `ztl build` SHALL be
≤ 30% of the total vault text content UNDER 10,000 files.
```

```
NFR-013-007: Serve API Latency

The `/api/search` endpoint SHALL return results in ≤ 50ms
for queries against 10,000 indexed documents WITH 95th
percentile.
```

---

## 4. Architecture

### 4.1 Architecture Decisions

```
ADR-013-001: Tantivy as the Single Search Engine

Status: Proposed

Context:
  SPEC-002 search is a brute-force grep. Three approaches for
  adding relevance scoring were considered:

  Option A — Index-free BM25: Score as a side effect of the
  grep scan. Not an established pattern, does not generalize.

  Option B — Tantivy only: Replace grep entirely with Tantivy.
  All queries go through the inverted index.

  Option C — Dual path: Keep grep for unranked, add Tantivy
  for ranked (behind a --rank flag).

Decision:
  Implement Option B (Tantivy only).

Rationale:
  - Maintaining two search engines (grep + Tantivy) doubles
    the surface area for bugs and behavioral divergence.
  - `ztl index` already exists for the link graph, so an
    index is always available. The grep path's "zero setup"
    benefit is moot.
  - Users who want unranked substring matching can use `grep`
    or `rg` directly on their vault.
  - Single engine means CLI, serve, and build all produce
    identical search results.

Consequences:
  + Single code path — simpler, fewer bugs
  + BM25 scoring on every query
  + Foundation for phrase, boolean, and fuzzy queries
  + Consistent behavior across CLI, serve, and build
  - `ztl search` now requires an index (built lazily if absent)
  - Adds tantivy to the dependency tree (~50 transitive deps)
  - Adds ~5–8 MB to binary size
  - `--regex` flag from SPEC-002 is removed (Tantivy handles
    word-level matching; use `rg` for character-level regex)
```

```
ADR-013-002: Bidirectional BFS for Neighbourhood Computation

Status: Proposed

Context:
  Graph-scoped search needs to identify which pages are "near"
  an anchor page.

  Option A — Forward-only BFS
  Option B — Backward-only BFS
  Option C — Bidirectional BFS (both directions per hop)

Decision:
  Implement Option C (bidirectional BFS).

Rationale:
  - "Nearby" is symmetric: if A links to B, both are in each
    other's neighbourhood.
  - Bidirectional BFS at depth 3 with average degree 5 visits
    ~125 nodes — trivial cost.

Consequences:
  + Captures the intuitive notion of "neighbourhood"
  - Neighbourhood grows exponentially with depth (mitigated
    by default depth 1)
```

```
ADR-013-003: Inline Heading Detection via Line Scan

Status: Proposed

Context:
  Search needs to report which heading each match falls under.

Decision:
  Regex scan for ATX headings (`^#{1,6}\s`), filtered against
  body_text_ranges to exclude code blocks.

Rationale:
  - Search already reads file content and computes
    body_text_ranges. A line scan adds negligible cost.
  - Simpler than re-parsing with pulldown-cmark.

Consequences:
  + ~30 lines of code, < 10% per-file overhead
  - Does not detect setext headings (acceptable)
```

```
ADR-013-004: Client-Side JS BM25 for Static Build

Status: Proposed

Context:
  `ztl build` generates a static HTML site with no backend.
  Search must run entirely in the browser. Three approaches:

  Option A — Tantivy compiled to WASM: Identical engine to
  CLI/serve. The tantivy-wasm project demonstrates feasibility.
  However, the WASM binary is ~4 MB gzipped, requires patches
  to tantivy's memmap dependency, and the synchronous I/O
  model needs workarounds in the browser.

  Option B — Custom BM25 scorer in WASM: Compile a minimal
  Rust BM25 implementation to WASM (~50–100 KB gzipped). Build
  step exports a compact index (term frequencies, doc lengths,
  IDF values). Mathematically identical to Tantivy's BM25.

  Option C — Pure JavaScript BM25: ~50-line inline JS scorer.
  Build step exports the same compact index. No WASM build
  pipeline needed.

Decision:
  Implement Option C (pure JavaScript BM25) for v1.

Rationale:
  - BM25 is a simple formula. The JS implementation is ~50
    lines and produces identical scores to Tantivy when given
    the same TF/IDF/DL inputs.
  - No WASM build pipeline, no wasm-pack, no wasm-bindgen.
  - The search index JSON is implementation-agnostic;
    upgrading to WASM later is a drop-in replacement.
  - Total added payload: ~2 KB JS + search index JSON.

Consequences:
  + Zero build-time dependencies beyond what ztl already has
  + Works in all browsers without WASM support
  + Index format is implementation-agnostic — JS or WASM
  - Slightly different floating-point behavior vs Tantivy
    (negligible — same formula, same inputs)
  - No phrase or boolean query support in browser for v1
```

### 4.2 Component Integration

```
                     ┌────────────────┐
                     │      CLI       │
                     │   (commands)   │
                     └───────┬────────┘
                             │
       ┌─────────────────────┼──────────────────┐
       │                     │                   │
┌──────▼──────┐       ┌──────▼──────┐     ┌──────▼──────┐
│   Scanner   │       │    Graph    │     │   SimHash   │
│             │       │   Engine    │     │   Index     │
└──────┬──────┘       └──────┬──────┘     └─────────────┘
       │                     │
       │  ┌──────────────┐   │
       │  │   Tantivy    │   │
       ├─►│ Search Index │   │
       │  │              │   │
       │  │ - build      │   │
       │  │ - query      │   │
       │  │ - invalidate │   │
       │  └──────┬───────┘   │
       │         │           │
┌──────▼─────────▼──┐       │
│      Search       │◄──────┘
│                   │  page set (--near)
│ - tantivy query   │
│ - re-scan matches │
│ - detect headings │
│ - emit results    │
└─────────┬─────────┘
          │
          ├──── CLI output (JSON/table)
          │
          ├──── Serve: GET /api/search
          │     (queries same Tantivy index)
          │
          └──── Build: search-index.json
                (compact BM25 index for JS client)
```

**Query modes:**

| Mode | Flags | Graph needed | Notes |
|------|-------|-------------|-------|
| Default | (none) | No | Tantivy BM25 query |
| Scoped | `--near X` | Yes | Filter to neighbourhood |
| Scoped + depth | `--near X --depth N` | Yes | Wider neighbourhood |

Heading detection runs in all modes.

**Integration points:**

1. **Scanner → Tantivy (index build).** During `ztl index`, body text from each file is tokenized and added to the Tantivy index. Same `body_text_ranges()` exclusion logic.

2. **Tantivy → Search (query).** `ztl search` queries Tantivy, which returns matching documents with BM25 scores. The search module re-reads each matched file to extract per-occurrence line/column/context/heading data.

3. **Graph → Search (neighbourhood filter).** When `--near` is used, the graph engine computes the neighbourhood page set, used to filter Tantivy results.

4. **Tantivy → Serve API.** The `/api/search` handler queries the same in-memory Tantivy index.

5. **Tantivy → Build Index.** During `ztl build`, corpus statistics (TF, DF, document lengths) are extracted from the Tantivy index and serialized to `search-index.json`.

### 4.3 Tantivy Index Schema

```rust
/// Each Markdown file → one Tantivy document.
/// Fields:
///   page_name: STRING (stored, indexed for filtering)
///   path:      STRING (stored, not indexed)
///   body:      TEXT   (indexed for full-text search, stored)
///
/// body = concatenated body-text ranges (frontmatter, code,
/// inline code, HTML comments excluded).
```

**Tokenization:** Tantivy's default tokenizer (lowercase, split on whitespace and punctuation).

**Index location:** `.ztl/search/`

### 4.4 Query Flow

1. Parse the query string using Tantivy's `QueryParser` against the `body` field.
2. Execute with a `TopDocs` collector, limited to `--limit`.
3. If `--near` is active, filter to neighbourhood page set.
4. For each matched document:
   a. Read the original file from disk.
   b. Find per-occurrence positions by scanning for query terms in body text.
   c. Detect headings.
   d. Attach heading context to each occurrence.
   e. Emit `SearchMatch` entries with the document's BM25 score.
5. Return results ordered by descending score, ties broken by path then line.

### 4.5 Neighbourhood Algorithm

**Input:** Link graph G, anchor page A, depth D.

**Output:** `HashSet<String>` of page names within D bidirectional hops.

1. Resolve A to a node in G. Error if not found.
2. Initialize visited = {A}, queue = [(A, 0)].
3. While queue not empty:
   a. Pop (page, d). If d ≥ D, skip.
   b. For each neighbour (outgoing + incoming):
      If not visited: add to visited, push (neighbour, d + 1).
4. Return visited.

### 4.6 Heading Detection Algorithm

**Input:** File content, body-text ranges.

**Output:** Sorted `Vec<FileHeading>`.

1. Scan line by line, tracking byte offsets.
2. For lines starting with 1–6 `#` followed by a space:
   a. If byte offset is in a body-text range, record heading.
3. For match at byte offset M, binary-search for largest heading offset ≤ M.

### 4.7 Data Model

```rust
/// Search match — with heading and score
struct SearchMatch {
    page: String,
    path: String,
    line: u32,
    column: u32,
    context: Option<String>,
    heading: Option<String>,     // enclosing heading text
    heading_level: Option<u8>,   // heading depth (1–6)
    score: f64,                  // BM25 score (always present)
}

/// Search output envelope
struct SearchOutput {
    query: String,
    total_matches: usize,
    near: Option<String>,              // anchor page (--near)
    depth: Option<usize>,              // hop depth (--near)
    neighbourhood_size: Option<usize>, // pages in scope (--near)
    results: Vec<SearchMatch>,
}

/// Heading within a file
struct FileHeading {
    byte_offset: usize,
    level: u8,
    text: String,
}
```

### 4.8 Serve Search Architecture

```
Browser                        Server (ztl serve)
  │                               │
  │ Cmd+K → type query            │
  │                               │
  │  GET /api/search?q=...&limit= │
  │──────────────────────────────►│
  │                               │── query Tantivy index
  │                               │── re-scan files for positions
  │                               │── attach headings
  │  200 OK (SearchOutput JSON)   │
  │◄──────────────────────────────│
  │                               │
  │ Render results in modal       │
```

The Tantivy index is opened once at server startup and held in `WebState`. The `/api/search` handler acquires a read lock, executes the query, and returns the same `SearchOutput` JSON that the CLI produces.

### 4.9 Build Search Architecture

```
ztl build                          Browser (static site)
  │                                     │
  │  1. Build Tantivy index             │
  │  2. Extract corpus stats            │
  │  3. Write search-index.json:        │
  │     {                               │
  │       "avgDl": 245.3,              │
  │       "docs": [                     │
  │         { "n": "Page",             │
  │           "s": "slug",             │
  │           "dl": 312,               │
  │           "tf": {"term":3,...} },  │
  │         ...                         │
  │       ],                            │
  │       "df": {"term":42,...}        │
  │     }                               │
  │  4. Embed BM25 scorer JS           │
  │     in layout HTML                  │
  │                                     │
  │                                Cmd+K → fetch search-index.json
  │                                     │  (lazy, cached)
  │                                     │── tokenize query
  │                                     │── BM25 score each doc
  │                                     │── sort by score
  │                                     │── render top results
```

The `search-index.json` format is designed for compact serialization:

- Term maps use short keys (lowercased tokens)
- Document entries include only non-zero term frequencies
- The format is implementation-agnostic — any BM25 scorer (JS, WASM, or otherwise) can consume it

The inline BM25 scorer (~50 lines of JS) implements the standard formula:

```
score(q, d) = Σ IDF(t) · (tf(t,d) · (k1 + 1)) / (tf(t,d) + k1 · (1 - b + b · dl/avgDl))

where:
  k1 = 1.2, b = 0.75
  IDF(t) = ln((N - df(t) + 0.5) / (df(t) + 0.5) + 1)
  tf(t,d) = term frequency of t in document d
  dl = document length (total terms)
  avgDl = average document length across corpus
  N = total documents
```

---

## 5. Contract Specifications

```
CON-013-001: search (Tantivy-backed, with --near and heading context)

ztl search <QUERY> [OPTIONS]

Changed behavior (replaces SPEC-002 CON-008):
  All queries go through the Tantivy index. Results are ordered
  by BM25 relevance score. The `score` field is always present.

New options:
  --near <PAGE>      Restrict to pages within --depth hops.
  --depth <N>        Neighbourhood radius [default: 1].
                     Requires --near. Must be ≥ 1.

Retained options (from CON-008):
  --case-sensitive   Require exact case when re-scanning matches
  --path <GLOB>      Filter results by file path glob
  --context <N>      Characters of context around matches
  --limit <N>        Max results [default: 50]
  -f <FORMAT>        Output format: json (default) or table

Removed options:
  --regex            Removed (use `rg` for regex)
  --all              Removed (Tantivy indexes body text only)

Output fields (always present):
  score              BM25 relevance score
  heading            Nearest preceding ATX heading, or null
  heading_level      Heading depth (1–6), or null

Output fields (when --near is used):
  near               Resolved anchor page name
  depth              Depth used
  neighbourhood_size Pages in the neighbourhood

Exit codes:
  0  Matches found
  1  No matches found
  2  Invalid query / --near page not found / bad --depth

Example output (JSON, --near):
{
  "query": "active recall",
  "total_matches": 3,
  "near": "Spaced Repetition",
  "depth": 1,
  "neighbourhood_size": 8,
  "results": [
    {
      "page": "Spaced Repetition",
      "path": "concepts/Spaced Repetition.md",
      "line": 35,
      "column": 10,
      "context": "...combines active recall with increasing...",
      "heading": "Relationship to Active Recall",
      "heading_level": 3,
      "score": 8.721
    },
    {
      "page": "Learning Techniques",
      "path": "concepts/Learning Techniques.md",
      "line": 18,
      "column": 22,
      "context": "...including active recall and spaced...",
      "heading": "Evidence-Based Methods",
      "heading_level": 2,
      "score": 5.104
    }
  ]
}

Example output (table):
Search results for 'active recall' (near: Spaced Repetition, depth: 1, 8 pages):
 Page                 | Score |  Line | Heading                      | Context
----------------------+-------+-------+------------------------------+----------
 Spaced Repetition    | 8.721 |    35 | ### Relationship to Active.. | ...active
 Learning Techniques  | 5.104 |    18 | ## Evidence-Based Methods    | ...active

Implements:
- REQ-013-002, REQ-013-005, REQ-013-006, REQ-013-007,
  REQ-013-008, REQ-013-009, REQ-013-010

Verified by:
- TEST-013-002, TEST-013-005, TEST-013-006, TEST-013-007,
  TEST-013-008, TEST-013-009, TEST-013-010
```

```
CON-013-002: index (extended with search index)

ztl index [OPTIONS]

Extended behavior (adds to CON-002):
  - After building the link graph, also builds the Tantivy
    full-text search index in `.ztl/search/`.
  - Only body text is indexed (same exclusions as SPEC-002).
  - Incremental: only re-indexes changed files.
  - Reports index stats alongside graph stats.

New output fields in JSON:
  search_index_docs    Number of documents in the search index
  search_index_size_kb Size of the search index directory in KB

Implements: REQ-013-001, REQ-013-003
Verified by: TEST-013-001, TEST-013-003
```

```
CON-013-003: Serve Search API Endpoint

GET /api/search

Query parameters:
  q       (required) Search query string
  limit   (optional) Max results [default: 20]

Response (200 OK):
  Content-Type: application/json
  Body: SearchOutput JSON (same schema as CLI output, without
  near/depth/neighbourhood_size — graph scoping is CLI-only)

Response (400 Bad Request):
  Empty or whitespace-only query

Example:
  GET /api/search?q=spaced+repetition&limit=5
  → {
    "query": "spaced repetition",
    "total_matches": 12,
    "results": [
      {
        "page": "Spaced Repetition",
        "path": "concepts/Spaced Repetition.md",
        "line": 1,
        "column": 3,
        "context": "# Spaced Repetition",
        "heading": null,
        "heading_level": null,
        "score": 12.34
      }
    ]
  }

Implements: REQ-013-012
Verified by: TEST-013-012
```

```
CON-013-004: Build Search Assets

ztl build [--output <DIR>]

Extended behavior (adds to existing build contract):
  - Emits `search-index.json` in the output directory root.
  - Embeds a BM25 scorer (~50 lines JS) in the layout HTML.
  - Upgrades the Cmd+K modal to perform full-text search.

search-index.json schema:
{
  "avgDl": <number>,
  "docs": [
    {
      "n": "<page_name>",
      "s": "<slug>",
      "dl": <doc_length>,
      "tf": { "<term>": <count>, ... }
    }
  ],
  "df": { "<term>": <doc_count>, ... }
}

Implements: REQ-013-014, REQ-013-015
Verified by: TEST-013-014, TEST-013-015
```

---

## 6. Test Specifications

### 6.1 Tantivy Search Tests

```
TEST-013-001: Search Index Construction

Scenario: Index built during ztl index
Given: A vault with 5 Markdown files
When: `ztl index` is run
Then:
  - `.ztl/search/` directory is created
  - JSON output includes search_index_docs: 5

Scenario: Index excludes non-body content
Given: A file with frontmatter "secret", code block "hidden",
       and body text "visible"
When: `ztl search "secret"` is run
Then: No results
When: `ztl search "visible"` is run
Then: Returns a result with a BM25 score

Scenario: Index respects ignore patterns
Given: A vault with `.ztlignore` containing "drafts/"
When: Index is built
Then: Files under drafts/ are not indexed

Verifies: REQ-013-001
```

```
TEST-013-002: BM25-Scored Results

Scenario: Results ordered by relevance
Given: "Algorithm.md" (200 words, "algorithm" 15×) and
       "Journal.md" (5000 words, "algorithm" 1×)
When: `ztl search "algorithm"` is run
Then:
  - Algorithm.md results appear before Journal.md
  - Each result has a `score` field
  - Algorithm.md score > Journal.md score

Scenario: Score always present
When: `ztl search "test"` is run
Then: Every result has a `score` field as a number

Verifies: REQ-013-002
```

```
TEST-013-003: Search Index Invalidation

Scenario: Modified file is re-indexed
Given: A vault indexed once, then one file modified to add "newterm"
When: `ztl index` is run again
Then: `ztl search "newterm"` finds the modified file

Scenario: Deleted file is removed
Given: A vault indexed once, then one file deleted
When: `ztl index` is run again
Then: Deleted file does not appear in results

Scenario: --no-cache forces full rebuild
Given: A cached vault with no changes
When: `ztl index --no-cache` is run
Then: search_index_docs matches file count

Verifies: REQ-013-003
```

```
TEST-013-004: Lazy Index Construction

Scenario: Search without prior ztl index
Given: No `.ztl/search/` directory
When: `ztl search "test"` is run
Then:
  - Stderr includes index-building message
  - Results are returned with BM25 scores
  - `.ztl/search/` now exists

Verifies: REQ-013-004
```

```
TEST-013-005: Line-Level Results

Scenario: Per-occurrence positions
Given: A file where "algorithm" appears on lines 5, 12, 30
When: `ztl search "algorithm"` is run
Then:
  - 3 results for that file, each with correct line/column
  - All share the same BM25 score

Verifies: REQ-013-005
```

### 6.2 Graph-Scoped Search Tests

```
TEST-013-006: Graph-Scoped Search

Scenario: --near restricts results
Given: A→B, A→C, B→D, E isolated. All contain "test"
When: `ztl search "test" --near A` (depth 1)
Then: Results include A, B, C. Not D, E.

Scenario: --near depth 2
When: `ztl search "test" --near A --depth 2`
Then: Results include A, B, C, D. Not E.

Scenario: Backlinks included (bidirectional)
Given: X→Y. Both contain "hello"
When: `ztl search "hello" --near Y`
Then: Results include X and Y.

Verifies: REQ-013-006
```

```
TEST-013-007: Neighbourhood Depth Control

Scenario: --depth without --near rejected
When: `ztl search "test" --depth 2`
Then: Exit code 2

Scenario: --depth 0 rejected
When: `ztl search "test" --near A --depth 0`
Then: Exit code 2

Verifies: REQ-013-007
```

```
TEST-013-008: Anchor Page Resolution

Scenario: Case-insensitive
Given: "Spaced Repetition.md" exists
When: `ztl search "test" --near "spaced repetition"`
Then: Resolves to "Spaced Repetition"

Scenario: Unresolvable page
When: `ztl search "test" --near "Nonexistent"`
Then: Exit code 2, suggests similar names

Verifies: REQ-013-008
```

```
TEST-013-009: Neighbourhood Metadata

Scenario: Metadata present with --near
Given: A has 4 neighbours
When: `ztl search "test" --near A`
Then: near: "A", depth: 1, neighbourhood_size: 5

Scenario: Metadata absent without --near
When: `ztl search "test"`
Then: No near/depth/neighbourhood_size fields

Verifies: REQ-013-009
```

### 6.3 Heading-Aware Context Tests

```
TEST-013-010: Heading-Aware Results

Scenario: Match under a heading
Given: "## Details\nThe algorithm works..."
When: `ztl search "algorithm"`
Then: heading: "Details", heading_level: 2

Scenario: Match before any heading
Given: "Preamble target.\n# First"
When: `ztl search "target"`
Then: heading: null

Scenario: Nearest heading wins
Given: "# Top\n## Sub\n### Deep\ntarget"
When: `ztl search "target"`
Then: heading: "Deep", heading_level: 3

Verifies: REQ-013-010
```

```
TEST-013-011: Heading Exclusion Zone Awareness

Scenario: Heading in code block ignored
Given: "## Real\ntext\n```\n# Comment\nmatch\n```\nmatch"
When: searching for "match"
Then: Both matches have heading "Real"

Verifies: REQ-013-011
```

### 6.4 Browser Search Tests

```
TEST-013-012: Serve Search API

Scenario: Basic query
Given: `ztl serve` running with indexed vault
When: GET /api/search?q=algorithm
Then:
  - 200 OK with JSON body
  - Results ordered by BM25 score
  - Each result has page, path, line, column, score, heading

Scenario: Empty query
When: GET /api/search?q=
Then: 400 Bad Request

Scenario: Limit parameter
When: GET /api/search?q=test&limit=3
Then: At most 3 results returned

Verifies: REQ-013-012
```

```
TEST-013-013: Serve Cmd+K Full-Text Search

Scenario: Full-text search in modal
Given: `ztl serve` running
When: User presses Cmd+K and types "algorithm"
Then:
  - Results show page name, heading, context, score
  - Results are ordered by relevance
  - Arrow keys navigate, Enter follows link

Verifies: REQ-013-013
```

```
TEST-013-014: Static Search Index Generation

Scenario: search-index.json emitted
Given: A vault with 10 pages
When: `ztl build --output dist/`
Then:
  - dist/search-index.json exists
  - Contains avgDl, docs (10 entries), df
  - Each doc has n, s, dl, tf

Scenario: Index content correctness
Given: "Algorithm.md" with body text "the algorithm is fast"
When: `ztl build`
Then:
  - search-index.json doc for "Algorithm" has
    tf with algorithm and fast entries, dl > 0

Verifies: REQ-013-014
```

```
TEST-013-015: Build Mode Client-Side Search

Scenario: Cmd+K search in static site
Given: A built static site opened in a browser
When: User presses Cmd+K and types "algorithm"
Then:
  - search-index.json is fetched (once, cached)
  - Results ranked by BM25 score
  - Page name and slug displayed, navigable

Verifies: REQ-013-015
```

### 6.5 Composability Tests

```
TEST-013-016: Flags Compose

Scenario: --near with --path
Given: Pages in A's neighbourhood, some in "concepts/"
When: `ztl search "idea" --near A --path "concepts/**"`
Then: Only results from concepts/ in A's neighbourhood

Scenario: --near with --context
When: `ztl search "keyword" --near A --context 30`
Then: Results scoped, with context snippets and headings

Verifies: REQ-013-002, REQ-013-006, REQ-013-010
```

---

## 7. Observability

```
OBS-013-001: Index Build Metrics

When --verbose, `ztl index` SHALL emit to stderr:
  - Documents indexed, index size, index build time (ms)
```

```
OBS-013-002: Search Metrics

When --verbose, `ztl search` SHALL emit to stderr:
  - Whether index loaded from cache or built on the fly
  - Tantivy query time (ms)
  - Documents matched by Tantivy (before line-level expansion)
```

```
OBS-013-003: Graph-Scoped Search Timing

When --verbose and --near, emit to stderr:
  - Resolved anchor, depth, neighbourhood size, BFS time (ms)
```

```
OBS-013-004: Heading Detection Metrics

When --verbose, emit to stderr:
  - Total headings detected across searched files
```

---

## 8. Traceability Matrix

| REQ           | CON           | TEST                     | OBS           |
| ------------- | ------------- | ------------------------ | ------------- |
| REQ-013-001   | CON-013-002   | TEST-013-001             | OBS-013-001   |
| REQ-013-002   | CON-013-001   | TEST-013-002, 016        | OBS-013-002   |
| REQ-013-003   | CON-013-002   | TEST-013-003             | OBS-013-001   |
| REQ-013-004   | —             | TEST-013-004             | OBS-013-002   |
| REQ-013-005   | CON-013-001   | TEST-013-005, 016        | —             |
| REQ-013-006   | CON-013-001   | TEST-013-006, 016        | OBS-013-003   |
| REQ-013-007   | CON-013-001   | TEST-013-007             | —             |
| REQ-013-008   | CON-013-001   | TEST-013-008             | —             |
| REQ-013-009   | CON-013-001   | TEST-013-009             | OBS-013-003   |
| REQ-013-010   | CON-013-001   | TEST-013-010, 016        | OBS-013-004   |
| REQ-013-011   | —             | TEST-013-011             | —             |
| REQ-013-012   | CON-013-003   | TEST-013-012             | —             |
| REQ-013-013   | —             | TEST-013-013             | —             |
| REQ-013-014   | CON-013-004   | TEST-013-014             | —             |
| REQ-013-015   | —             | TEST-013-015             | —             |

---

## 9. Impact on Existing Specifications

### 9.1 Impact on SPEC-002

SPEC-002's grep-based search is replaced entirely by Tantivy. ADR-002 ("Search Without a Pre-Built Index") is superseded — `ztl search` now always uses an index (built lazily if absent). The `--regex` and `--all` flags from CON-008 are removed. The `SearchMatch` struct gains `heading`, `heading_level`, and `score` fields. The `SearchOutput` envelope gains optional neighbourhood metadata. The `regex` field is removed from the output envelope.

### 9.2 Impact on SPEC-001

`ztl index` (CON-002) is extended to build the search index. The graph engine gains a `neighbourhood()` method.

### 9.3 Impact on SPEC-012

The `ztl build` output gains a `search-index.json` file. The HTML layout gains an embedded BM25 scorer. The Cmd+K modal is upgraded from fuzzy page-name matching to full-text ranked search.

### 9.4 Impact on NFR-004 (Binary Size)

Tantivy adds ~5–8 MB. The effective binary size budget is relaxed per project decision.

### 9.5 Impact on Serve Mode

`ztl serve` gains a `/api/search` route. The Cmd+K modal is upgraded to query the API. `WebState` gains a Tantivy index handle.

---

## 10. Open Questions

1. **Should Tantivy's query syntax be exposed?** Tantivy's `QueryParser` supports `+required -excluded "exact phrase"`. For v1, the query string is passed directly to Tantivy's parser. Document that query semantics differ from SPEC-002's literal substring matching.

2. **Should stemming be enabled?** Tantivy supports stemming via `tantivy::tokenizer::Stemmer`. Defer — default tokenizer is sufficient for v1.

3. **Should `--near` support multiple anchors?** Defer — single anchor covers the primary use case.

4. **Should the static search index include heading information?** Adding headings to `search-index.json` would let the browser display heading context in results. Recommend yes for v2.

5. **Should WASM replace the JS BM25 scorer?** A WASM scorer (compiled from the same Rust BM25 code) would provide exact numerical parity with the CLI. The `search-index.json` format is the same regardless of client implementation, so this is a drop-in upgrade. Defer to a follow-up — the JS scorer produces equivalent results and requires no build pipeline changes.

6. **Should graph-scoped search be available in the browser?** In serve mode, the graph is available server-side — the API could accept `near` and `depth` parameters. In build mode, the graph would need to be serialized. Defer.

---

**END OF SPEC-013**
