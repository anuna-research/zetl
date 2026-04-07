---
title: "SPEC-025: Graph-Aware Snippet Extraction for Search and Link Results"
version: 0.1.0
status: draft
date: 2026-04-07
audience: agent, human
parent: SPEC-013
related:
  - SPEC-018
  - SPEC-002
  - SPEC-003
  - SPEC-001
---

# SPEC-025: Graph-Aware Snippet Extraction for Search and Link Results

## Information Table

| Field          | Value                                                                     |
| -------------- | ------------------------------------------------------------------------- |
| Document ID    | SPEC-025                                                                  |
| Title          | Graph-Aware Snippet Extraction for Search and Link Results                |
| Version        | 0.1.0                                                                     |
| Status         | Draft                                                                     |
| Author         | Agent (USDD Protocol v1.0.0)                                              |
| Date           | 2026-04-07                                                                |
| Audience       | Agent, Human                                                              |
| Trace          | USDD Agent Protocol v1.0.0                                                |
| Parent         | SPEC-013: zetl search -- Tantivy Full-Text Search, Graph Scoping, Browser |
| Related        | SPEC-018: Semantic Search, SPEC-002: Full-Text Search, SPEC-003: Agent Ergonomics, SPEC-001: Link Graph CLI |
| Dependencies   | SPEC-013 search engine, SPEC-001 graph engine, SPEC-018 semantic search   |

---

## 1. Overview

SPEC-013 and SPEC-018 give zetl ranked search results with heading context, BM25 scores, and optional cosine similarity scores. Each result includes a page name, line number, heading, and a fixed-length `context` string (a character window around the match point). This is sufficient for programmatic filtering but insufficient for human or agent triage -- the context string may split mid-sentence, omit the paragraph's thesis, or strip the wikilinks that make the match meaningful in a graph-structured vault.

This specification adds **graph-aware snippet extraction**: bounded, structure-respecting excerpts around match points that resolve wikilinks, report graph position, and consolidate multiple matches within a page into a coherent reading unit.

### 1.1 Motivation

**Why snippets matter.** A search for "emergence" in a 500-page vault returns 30 results. The user (human or agent) must decide which results to read in full. Today's output provides:

```json
{
  "page": "Complex Systems",
  "line": 47,
  "heading": "Emergent Properties",
  "score": 6.82,
  "context": "...emergence arises when local interaction..."
}
```

The `context` field is a 60-character window centered on the match. It may cut off mid-clause. It does not indicate whether the surrounding paragraph is two sentences or twenty. It does not reveal that the snippet contains `[[Self-Organization]]`, a hub page with 14 backlinks. An agent making a retrieval decision needs more.

**Why graph-awareness.** zetl's unique asset is the link graph. A snippet containing `[[Self-Organization]]` is more informative when the agent knows that Self-Organization is a hub (14 backlinks, central to the vault). A snippet from a page with zero backlinks (an orphan) signals different things than a snippet from a page with 30. Snippet extraction that ignores the graph wastes the investment in building it.

**Why structure-respecting boundaries.** Markdown has natural semantic units: paragraphs, list items, code blocks, heading sections. A snippet that respects these boundaries is more readable than a fixed character window. A snippet that starts at a paragraph boundary and ends at the next paragraph boundary carries a complete thought.

### 1.2 Design Principles

1. **Structure over character count.** Snippets are bounded by markdown structural elements (paragraphs, headings, code fences), not by a fixed character window.
2. **Graph enrichment is additive.** Snippet extraction works without the graph (pure text extraction). Graph metadata is layered on top when available.
3. **Multi-match consolidation.** Multiple matches in the same page produce merged or separated snippets based on proximity, not N independent context windows.
4. **Agent-first, human-readable.** JSON output includes enough context for an LLM to assess relevance without fetching the full page. Table output is scannable by a human.
5. **Opt-in, backward-compatible.** Existing `--context` behavior is unchanged. Snippets are activated via `--snippet` flag. Default output format is not altered.

### 1.3 Scope

**In scope:**

- Heading-bounded snippet extraction (match's enclosing section)
- Paragraph-bounded snippet extraction (match's enclosing paragraph)
- Configurable snippet mode: `heading`, `paragraph`, or `auto` (default)
- Maximum snippet length cap (default 500 characters, configurable)
- Wikilink resolution within snippets (page existence, backlink count)
- Graph context metadata on each result (backlink count, forward link count, hub/leaf classification)
- Multi-match consolidation within a page
- Integration with `zetl search` (all modes: BM25, semantic, hybrid)
- Integration with `zetl backlinks` (show context where the backlink appears)
- Integration with `zetl check` (show context around dead links)
- JSON and table output formats for snippets

**Out of scope:**

- Query-term highlighting (bold/color markup in snippets) -- defer to a presentation-layer spec
- Snippet caching or pre-computation at index time
- Snippet extraction for `zetl serve` or `zetl build` web interfaces -- defer to a web-layer spec
- Intent-aware snippet steering (selecting which of several matches to feature based on inferred query intent)
- Snippet ranking independent of the underlying search score

---

## 2. User Profiles

The existing user profiles from SPEC-001 (section 2) and SPEC-013 (section 2) apply.

### 2.1 Agent Operator -- Extended Workflow

```
Daily workflow (updated):
  1. Run `zetl search "emergence" --snippet` to get ranked results
     with structure-respecting excerpts
  2. Each result includes a snippet bounded by the enclosing
     heading section or paragraph, plus graph metadata:
     backlink count, forward link count, hub/leaf classification
  3. Wikilinks in the snippet are resolved: the agent sees that
     [[Self-Organization]] exists (14 backlinks, hub) and
     [[Downward Causation]] does not exist (dead link)
  4. The agent decides which pages to read in full based on
     snippet content and graph position -- no need to fetch
     every matched page
  5. For multi-match pages, nearby matches are consolidated
     into a single snippet; distant matches appear as separate
     snippets within the same result
```

### 2.2 Human Knowledge Worker -- Extended Workflow

```
Daily workflow (updated):
  1. Run `zetl search "emergence" --snippet -f table` to get
     a scannable table of results with excerpts
  2. Each row shows the page, score, heading, and a paragraph-
     length excerpt -- enough to judge relevance at a glance
  3. Run `zetl backlinks "Self-Organization" --snippet` to see
     where each backlink appears in context, not just page+line
  4. Run `zetl check --snippet` to see dead links in their
     surrounding paragraph, making it easier to decide whether
     to create the target page or fix the link
```

### 2.3 Vault Curator -- New Workflow

```
Refactoring workflow:
  1. Run `zetl check --snippet` to see all dead links with
     surrounding context
  2. For each dead link, the snippet shows the sentence and
     paragraph where the link appears, plus how many other
     pages link to the same dead target
  3. Run `zetl backlinks "Old Page Name" --snippet` before
     renaming a page, to see every usage in context and
     decide whether the link text needs updating
```

---

## 3. Requirements

### 3.1 Functional Requirements -- Snippet Extraction

```
REQ-146: Heading-Bounded Snippet Extraction

The system SHALL, when --snippet is specified with mode "heading",
extract a snippet consisting of the full content from the
enclosing heading to the next heading of equal or higher level
(or end of file),
FOR all user roles
WITH the snippet truncated to --snippet-max-len characters
  (default 500) if the section exceeds the limit
AND truncation occurring at the nearest paragraph boundary
  before the limit
AND the snippet including the heading text itself as the first
  line
AND matches before the first heading in a file using content
  from file start to the first heading as the snippet boundary.

Trace:
- TEST-169
- CON-041
- ADR-065
```

```
REQ-147: Paragraph-Bounded Snippet Extraction

The system SHALL, when --snippet is specified with mode
"paragraph", extract a snippet consisting of the enclosing
paragraph (delimited by blank lines or block boundaries) around
the match point,
FOR all user roles
WITH the paragraph extended by one additional paragraph before
  and after the match paragraph when the match paragraph alone
  is shorter than 80 characters
AND code blocks (fenced or indented) treated as atomic units:
  if the match falls inside a code block, the entire code block
  is included
AND list items treated as atomic units: if the match falls
  inside a list item, the entire list item is included, plus
  the list item's parent list context (preceding list items up
  to 3).

Trace:
- TEST-170
- CON-041
```

```
REQ-148: Auto-Mode Snippet Selection

The system SHALL, when --snippet is specified without an explicit
mode (or with mode "auto"), select the snippet boundary
automatically,
FOR all user roles
WITH the following heuristic:
  1. If the enclosing heading section is ≤ 500 characters
     (or --snippet-max-len), use the full section (heading mode)
  2. Otherwise, use paragraph mode
AND the chosen mode reported in the snippet metadata as
  "snippet_mode": "heading" or "snippet_mode": "paragraph".

Trace:
- TEST-171
- CON-041
```

```
REQ-149: Snippet Length Control

The system SHALL accept a --snippet-max-len <N> flag (positive
integer, in characters) to control the maximum length of
extracted snippets,
FOR all user roles
WITH a default of 500 characters
AND truncation always occurring at a structural boundary
  (paragraph end, list item end, code block end) rather than
  mid-token
AND truncated snippets ending with a "..." indicator
AND an error if --snippet-max-len is used without --snippet.

Trace:
- TEST-172
- CON-041
```

### 3.2 Functional Requirements -- Wikilink Resolution in Snippets

```
REQ-150: Wikilink Resolution Within Snippets

The system SHALL, when --snippet is specified, resolve every
[[wikilink]] that appears within the extracted snippet text,
FOR all user roles
WITH each resolved link annotated in the JSON output as an
  entry in a "snippet_links" array containing:
  - target: the link target text
  - exists: boolean (whether the target page exists in the vault)
  - backlink_count: integer (number of pages that link to the
    target, 0 if the page does not exist)
AND the snippet_links array ordered by appearance position
  in the snippet text
AND alias links ([[Target|Display]]) resolved by the target
  portion.

Trace:
- TEST-173
- CON-041
```

### 3.3 Functional Requirements -- Graph Context Enrichment

```
REQ-151: Graph Context Metadata on Search Results

The system SHALL, when --snippet is specified, include graph
context metadata on each search result,
FOR all user roles
WITH the following fields added to each result:
  - backlink_count: number of pages that link to this result page
  - forward_link_count: number of pages this result page links to
  - graph_role: one of "hub" (≥ 10 backlinks), "bridge"
    (backlink_count ≥ 3 AND forward_link_count ≥ 3),
    "leaf" (backlink_count ≤ 1 AND forward_link_count ≤ 3),
    or "node" (default, none of the above)
AND graph_role computed from the current link graph (built
  during zetl index or on-the-fly)
AND the graph context fields present even when the snippet
  itself contains no wikilinks.

Trace:
- TEST-174
- CON-041
```

### 3.4 Functional Requirements -- Multi-Match Consolidation

```
REQ-152: Multi-Match Consolidation Within a Page

The system SHALL, when --snippet is specified and a page
contains multiple matches for the query, consolidate nearby
matches into a single snippet and separate distant matches
into distinct snippets,
FOR all user roles
WITH "nearby" defined as matches whose enclosing structural
  units (paragraph or heading section) overlap or are adjacent
AND consolidated snippets spanning from the start of the first
  match's structural unit to the end of the last match's
  structural unit (subject to --snippet-max-len)
AND distant matches producing separate snippet entries within
  the same result, each with its own line number, heading, and
  snippet text
AND the result-level match_count field reflecting the total
  number of individual matches (not the number of snippets).

Trace:
- TEST-175
- CON-041
```

### 3.5 Functional Requirements -- Cross-Command Integration

```
REQ-153: Snippet Extraction in backlinks and check Commands

The system SHALL support the --snippet flag on `zetl backlinks`
and `zetl check` commands,
FOR all user roles
WITH `zetl backlinks <PAGE> --snippet` extracting a snippet
  around each occurrence of [[PAGE]] in the linking page
AND `zetl check --snippet` extracting a snippet around each
  dead link occurrence
AND the same snippet extraction logic (structure boundaries,
  wikilink resolution, graph context) applied as in search
AND --snippet-max-len and --snippet-mode flags accepted by
  both commands.

Trace:
- TEST-176
- CON-041
```

### 3.6 Non-Functional Requirements

```
NFR-055: Snippet Extraction Latency

Snippet extraction SHALL add ≤ 20ms per result to the total
query time UNDER a vault of 10,000 pages WITH 95th percentile,
WHEN --snippet is specified,
WITH the dominant cost being file I/O (re-reading matched files)
AND snippet extraction itself (parsing, boundary detection,
wikilink resolution) completing in ≤ 2ms per result on CPU.
```

```
NFR-056: Snippet Output Size

Snippet JSON output for a single result SHALL be ≤ 2 KB
(excluding base64 or binary content) UNDER --snippet-max-len
default of 500 characters,
WITH the total response size for 20 results ≤ 50 KB
AND this size budget sufficient for an LLM context window
to ingest multiple search result pages without truncation.
```

---

## 4. Architecture

### 4.1 Architecture Decisions

```
ADR-065: Structure-Bounded Snippets Over Fixed Character Windows

Status: Proposed

Context:
  Existing search results use a fixed character window
  (--context N) centered on the match point. Three approaches
  for improved snippet extraction were considered:

  Option A -- Wider fixed window: Increase the default --context
  from 60 to 300 characters. Simple, but still cuts mid-sentence
  and mid-paragraph. Does not respect markdown structure.

  Option B -- Sentence-bounded window: Use sentence detection
  (period + whitespace heuristic) to expand the window to
  complete sentences. Better than character windows, but
  sentence detection in markdown is unreliable (abbreviations,
  code, URLs all contain periods).

  Option C -- Structure-bounded extraction: Use markdown
  structural elements (headings, blank-line-delimited paragraphs,
  code fences, list markers) as snippet boundaries. Extract the
  enclosing structural unit of the match, with fallback to
  paragraph boundaries.

Decision:
  Implement Option C (structure-bounded extraction).

Rationale:
  - Markdown structure is unambiguous and cheap to detect.
    Blank lines delimit paragraphs, ATX headings delimit
    sections, triple backticks delimit code blocks. No NLP
    required.
  - Structure-bounded snippets carry complete thoughts.
    A paragraph is a semantic unit; a character window is not.
  - Heading-bounded mode reuses the heading detection
    infrastructure from SPEC-013 (REQ-013-010), reducing new
    code.
  - Auto-mode provides intelligent selection without user
    configuration in the common case.
  - Fixed character windows remain available via the existing
    --context flag for users who prefer them.

Consequences:
  + Snippets are always readable and structurally complete
  + Reuses existing heading detection from SPEC-013
  + Auto-mode handles most cases without configuration
  - Snippet length is variable (bounded by --snippet-max-len
    but not fixed), which may complicate display alignment
    in table format
  - File re-reading is required to detect structural boundaries
    (same cost as current heading detection)
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
│   Search    │       │  Backlinks  │     │    Check    │
│   Engine    │       │   Command   │     │   Command   │
└──────┬──────┘       └──────┬──────┘     └──────┬──────┘
       │                     │                   │
       └─────────────────────┼───────────────────┘
                             │
                      ┌──────▼──────┐
                      │   Snippet   │
                      │  Extractor  │
                      │             │
                      │ - extract   │
                      │ - resolve   │
                      │ - enrich    │
                      │ - merge     │
                      └──────┬──────┘
                             │
              ┌──────────────┼──────────────┐
              │              │              │
       ┌──────▼──────┐ ┌────▼─────┐ ┌──────▼──────┐
       │  Structure  │ │  Link    │ │   Graph     │
       │  Parser     │ │  Resolver│ │   Context   │
       │             │ │          │ │             │
       │ - headings  │ │ - exists │ │ - backlinks │
       │ - paragraphs│ │ - count  │ │ - fwd links │
       │ - code blks │ │          │ │ - role      │
       │ - lists     │ │          │ │             │
       └─────────────┘ └──────────┘ └─────────────┘
```

**Integration points:**

1. **Search Engine -> Snippet Extractor.** After Tantivy returns matched documents with BM25 scores and the search module identifies per-occurrence line/column positions, the snippet extractor receives the file content and match positions. It extracts structure-bounded snippets.

2. **Backlinks Command -> Snippet Extractor.** When `zetl backlinks <PAGE> --snippet` is invoked, the backlinks command identifies each file containing `[[PAGE]]` and passes the file content and link positions to the snippet extractor.

3. **Check Command -> Snippet Extractor.** When `zetl check --snippet` is invoked, the check command identifies dead link positions and passes them to the snippet extractor.

4. **Snippet Extractor -> Structure Parser.** The structure parser detects heading boundaries, paragraph boundaries (blank lines), code block boundaries (fenced and indented), and list item boundaries. It produces a `StructureMap` for the file.

5. **Snippet Extractor -> Link Resolver.** Wikilinks within the extracted snippet text are parsed and resolved against the vault's page set. Each link is annotated with existence and backlink count.

6. **Snippet Extractor -> Graph Context.** The result page's graph position (backlink count, forward link count, hub/leaf/bridge/node role) is computed from the link graph.

### 4.3 Snippet Extraction Algorithm

**Input:** File content, list of match positions (byte offsets), snippet mode (heading/paragraph/auto), max length.

**Output:** `Vec<Snippet>` -- one or more snippets for the file, each covering one or more consolidated matches.

```
1. Build StructureMap for file:
   a. Detect headings (reuse SPEC-013 heading detection)
   b. Detect paragraph boundaries (blank lines outside code blocks)
   c. Detect code block boundaries (``` fences)
   d. Detect list item boundaries (lines starting with - * + or 1.)

2. For each match position:
   a. Find the enclosing structural unit:
      - heading mode: content from enclosing heading to next
        heading of equal/higher level (or EOF)
      - paragraph mode: content from preceding blank line (or
        heading/SOF) to next blank line (or heading/EOF)
      - auto mode: if enclosing section ≤ max_len, use heading
        mode; otherwise use paragraph mode

3. Consolidation pass:
   a. Sort snippets by start position
   b. Merge overlapping or adjacent snippets (structural units
      that share a boundary)
   c. For merged snippets, record all constituent match positions

4. Truncation pass:
   a. For each snippet exceeding max_len:
      - Find the last structural boundary (paragraph end, list
        item end) before the limit
      - Truncate there and append "..."

5. Wikilink resolution pass:
   a. Scan snippet text for [[...]] patterns
   b. Resolve each against the page set
   c. Build snippet_links array

6. Return Vec<Snippet>
```

### 4.4 Structure Map

```rust
/// Structural boundaries detected in a file.
struct StructureMap {
    /// Heading positions, sorted by byte offset.
    headings: Vec<HeadingSpan>,
    /// Paragraph spans (start..end byte offsets), excluding code blocks.
    paragraphs: Vec<Range<usize>>,
    /// Code block spans (fenced ``` or ~~~).
    code_blocks: Vec<Range<usize>>,
    /// List item spans.
    list_items: Vec<ListItemSpan>,
}

struct HeadingSpan {
    byte_offset: usize,
    level: u8,
    text: String,
    /// Byte range of the entire section (heading to next heading or EOF).
    section_range: Range<usize>,
}

struct ListItemSpan {
    byte_range: Range<usize>,
    /// Index of the parent list's first item (for context inclusion).
    list_start_index: usize,
}
```

### 4.5 Graph Role Classification

```
graph_role(page, graph):
  bl = backlink_count(page, graph)
  fl = forward_link_count(page, graph)

  if bl >= 10:
    return "hub"
  if bl >= 3 AND fl >= 3:
    return "bridge"
  if bl <= 1 AND fl <= 3:
    return "leaf"
  return "node"
```

The thresholds (10, 3, 1, 3) are chosen to be meaningful for personal vaults of 100-5000 pages. A page with 10+ backlinks is in the top percentile of connectivity for most Zettelkasten vaults.

### 4.6 Purity Boundary Map

#### Pure Core (no I/O, no shared state, deterministic)

- `build_structure_map(content: &str) -> StructureMap` -- detects all structural boundaries
- `extract_snippet(content: &str, matches: &[usize], map: &StructureMap, mode: SnippetMode, max_len: usize) -> Vec<RawSnippet>` -- extracts and consolidates snippets
- `classify_graph_role(backlink_count: usize, forward_link_count: usize) -> GraphRole` -- pure classification
- `resolve_snippet_links(snippet_text: &str, page_set: &HashSet<String>, backlink_map: &HashMap<String, Vec<String>>) -> Vec<SnippetLink>` -- parses and resolves wikilinks in snippet text
- `consolidate_matches(snippets: Vec<RawSnippet>) -> Vec<RawSnippet>` -- merges overlapping/adjacent snippets
- `truncate_snippet(text: &str, max_len: usize, map: &StructureMap) -> String` -- truncates at structural boundary

#### Effectful Shell (orchestrates I/O, calls pure core)

- `SnippetExtractor::extract(path: &Path, matches: &[MatchPosition], config: &SnippetConfig, graph: &LinkGraph) -> Result<Vec<Snippet>>` -- reads file, calls pure core, assembles final snippets
- Integration in `cmd_search`, `cmd_backlinks`, `cmd_check` -- passes results through snippet extractor when `--snippet` is specified

#### Boundary Contracts

- `StructureMap` flows core -> core (produced by parsing, consumed by extraction)
- `RawSnippet` flows core -> shell (produced by extraction, enriched with I/O-dependent data)
- `Snippet` flows shell -> caller (final output type with all metadata)

#### Dependency Rule

Dependencies point inward: shell -> core. Core MUST NOT import from shell. File I/O lives exclusively in the shell.

#### Enforcement

- Module structure: `src/snippet/core.rs` (pure), `src/snippet/mod.rs` (effectful shell)
- Unit tests cover `core.rs` functions with in-memory content strings
- Integration tests cover the full pipeline via `assert_cmd`

### 4.7 Data Model

```rust
/// Configuration for snippet extraction.
struct SnippetConfig {
    mode: SnippetMode,       // heading, paragraph, auto
    max_len: usize,          // default 500
    resolve_links: bool,     // default true when --snippet
    include_graph_context: bool, // default true when --snippet
}

enum SnippetMode {
    Heading,
    Paragraph,
    Auto,
}

enum GraphRole {
    Hub,
    Bridge,
    Leaf,
    Node,
}

/// A resolved wikilink within a snippet.
struct SnippetLink {
    target: String,
    exists: bool,
    backlink_count: usize,
}

/// A single extracted snippet (may consolidate multiple matches).
struct Snippet {
    text: String,
    start_line: u32,
    end_line: u32,
    heading: Option<String>,
    heading_level: Option<u8>,
    snippet_mode: SnippetMode,
    match_count: usize,
    match_lines: Vec<u32>,
    truncated: bool,
    snippet_links: Vec<SnippetLink>,
}

/// Extended search result with snippet and graph context.
struct SnippetSearchResult {
    // Base fields (from SPEC-013 SearchMatch):
    page: String,
    path: String,
    score: f64,
    heading: Option<String>,
    heading_level: Option<u8>,

    // Snippet fields (present when --snippet):
    snippets: Vec<Snippet>,

    // Graph context fields (present when --snippet):
    backlink_count: usize,
    forward_link_count: usize,
    graph_role: GraphRole,
}
```

---

## 5. Contract Specifications

```
CON-041: search --snippet (graph-aware snippet extraction)

zetl search <QUERY> --snippet [OPTIONS]

New options:
  --snippet            Enable snippet extraction on results
  --snippet-mode <M>   Snippet boundary mode: heading, paragraph,
                       auto [default: auto]
  --snippet-max-len <N>  Maximum snippet length in characters
                         [default: 500]

Retained options (from CON-013-001):
  --near <PAGE>      Restrict to pages within --depth hops
  --depth <N>        Neighbourhood radius [default: 1]
  --case-sensitive   Require exact case when re-scanning matches
  --path <GLOB>      Filter results by file path glob
  --limit <N>        Max results [default: 50]
  -f <FORMAT>        Output format: json (default) or table

Also applies to:
  zetl backlinks <PAGE> --snippet [--snippet-mode M] [--snippet-max-len N]
  zetl check --snippet [--snippet-mode M] [--snippet-max-len N]

Output fields (when --snippet, JSON format):
  Each result gains:
    snippets[]           Array of snippet objects:
      .text              Extracted snippet text (string)
      .start_line        First line number of the snippet (u32)
      .end_line          Last line number of the snippet (u32)
      .heading           Enclosing heading text, or null
      .heading_level     Heading depth (1-6), or null
      .snippet_mode      "heading" or "paragraph"
      .match_count       Number of matches in this snippet (u32)
      .match_lines       Array of line numbers of matches (u32[])
      .truncated         Whether the snippet was truncated (bool)
      .snippet_links[]   Resolved wikilinks in snippet:
        .target          Link target text
        .exists          Whether target page exists (bool)
        .backlink_count  Pages linking to target (u32)
    backlink_count       Pages linking to this result page (u32)
    forward_link_count   Pages this result page links to (u32)
    graph_role           "hub", "bridge", "leaf", or "node"

Behavior:
  - --snippet can be combined with --near, --path, --limit,
    --case-sensitive, -f, and all SPEC-018 flags (--semantic,
    --hybrid)
  - Without --snippet, output is unchanged (backward compatible)
  - --snippet-mode and --snippet-max-len require --snippet;
    using them without --snippet produces an error
  - In table format, the snippet text is shown in a "Snippet"
    column, truncated to terminal width, with match lines and
    graph role shown in adjacent columns

Exit codes:
  0  Matches found
  1  No matches found
  2  Invalid flags / page not found / bad --snippet-max-len

Example output (JSON, search with --snippet):
{
  "query": "emergence",
  "total_matches": 5,
  "results": [
    {
      "page": "Complex Systems",
      "path": "concepts/Complex Systems.md",
      "score": 6.82,
      "heading": "Emergent Properties",
      "heading_level": 2,
      "backlink_count": 7,
      "forward_link_count": 12,
      "graph_role": "bridge",
      "snippets": [
        {
          "text": "## Emergent Properties\n\nEmergence arises when local interactions between components produce global patterns that no single component encodes. The canonical example is [[Self-Organization]] in biological systems, where simple rules at the cell level produce complex tissue architectures.\n\nThis is distinct from [[Downward Causation]], which posits top-down constraint.",
          "start_line": 45,
          "end_line": 51,
          "heading": "Emergent Properties",
          "heading_level": 2,
          "snippet_mode": "heading",
          "match_count": 2,
          "match_lines": [47, 51],
          "truncated": false,
          "snippet_links": [
            {
              "target": "Self-Organization",
              "exists": true,
              "backlink_count": 14
            },
            {
              "target": "Downward Causation",
              "exists": true,
              "backlink_count": 3
            }
          ]
        }
      ]
    }
  ]
}

Example output (JSON, backlinks with --snippet):
{
  "page": "Self-Organization",
  "backlinks": [
    {
      "page": "Complex Systems",
      "path": "concepts/Complex Systems.md",
      "backlink_count": 7,
      "forward_link_count": 12,
      "graph_role": "bridge",
      "snippets": [
        {
          "text": "The canonical example is [[Self-Organization]] in biological systems, where simple rules at the cell level produce complex tissue architectures.",
          "start_line": 47,
          "end_line": 48,
          "heading": "Emergent Properties",
          "heading_level": 2,
          "snippet_mode": "paragraph",
          "match_count": 1,
          "match_lines": [47],
          "truncated": false,
          "snippet_links": [
            {
              "target": "Self-Organization",
              "exists": true,
              "backlink_count": 14
            }
          ]
        }
      ]
    }
  ]
}

Example output (table, search with --snippet):
Search results for 'emergence' (5 matches):
 Page              | Score | Role   | BL | Snippet
-------------------+-------+--------+----+------------------------------------------
 Complex Systems   | 6.82  | bridge |  7 | ## Emergent Properties
                   |       |        |    | Emergence arises when local interactions
                   |       |        |    | between components produce global...
                   |       |        |    | [2 matches, lines 47,51]
-------------------+-------+--------+----+------------------------------------------
 Agent Behavior    | 4.31  | node   |  4 | Swarm emergence is a well-studied case
                   |       |        |    | where individual agents follow...
                   |       |        |    | [1 match, line 23]

Implements:
- REQ-146, REQ-147, REQ-148, REQ-149, REQ-150, REQ-151,
  REQ-152, REQ-153

Verified by:
- TEST-169, TEST-170, TEST-171, TEST-172, TEST-173, TEST-174,
  TEST-175, TEST-176
```

---

## 6. Test Specifications

| ID | Description | Type | Traces |
|----|-------------|------|--------|
| TEST-169 | Heading-bounded snippet: a match under `## Section A` extracts content from `## Section A` to the next `## ` heading; the snippet starts with the heading text and ends before the next heading | Unit | REQ-146, ADR-065 |
| TEST-170 | Paragraph-bounded snippet: a match in the middle of a multi-sentence paragraph extracts the full paragraph (blank-line to blank-line); a match in a paragraph shorter than 80 chars also includes the preceding and following paragraphs | Unit | REQ-147 |
| TEST-171 | Auto-mode selection: for a short section (200 chars), auto mode selects heading mode and `snippet_mode` is `"heading"`; for a long section (2000 chars), auto mode selects paragraph mode and `snippet_mode` is `"paragraph"` | Unit | REQ-148 |
| TEST-172 | Snippet length control: a snippet from a 1500-character section with `--snippet-max-len 300` is truncated at a paragraph boundary before 300 characters; the `truncated` field is `true`; the snippet ends with `"..."` | Unit | REQ-149 |
| TEST-173 | Wikilink resolution: a snippet containing `[[Existing Page]]` and `[[Dead Link]]` produces `snippet_links` with `exists: true, backlink_count: 5` and `exists: false, backlink_count: 0` respectively; alias links `[[Target\|Display]]` resolve by target | Unit | REQ-150 |
| TEST-174 | Graph context enrichment: a result page with 12 backlinks and 4 forward links has `graph_role: "hub"`; a page with 0 backlinks and 1 forward link has `graph_role: "leaf"`; a page with 4 backlinks and 5 forward links has `graph_role: "bridge"` | Unit | REQ-151 |
| TEST-175 | Multi-match consolidation: two matches 3 lines apart (same paragraph) produce 1 snippet with `match_count: 2`; two matches 50 lines apart (different sections) produce 2 separate snippets in the `snippets` array | Unit | REQ-152 |
| TEST-176 | Cross-command integration: `zetl backlinks "Page" --snippet` produces snippet output with resolved links and graph context; `zetl check --snippet` produces snippet output around dead links with the dead link's target in `snippet_links` with `exists: false` | Integration | REQ-153 |

### Verification Strategy

| System characteristic | Technique |
|-----------------------|-----------|
| Structure map building (headings, paragraphs, code blocks, lists) | Unit tests with synthetic markdown strings (TEST-169, TEST-170) |
| Snippet mode selection (auto heuristic) | Unit tests with short and long sections (TEST-171) |
| Truncation at structural boundaries | Unit tests with known content and length limits (TEST-172) |
| Wikilink resolution and graph role classification | Unit tests with mock page sets and link graphs (TEST-173, TEST-174) |
| Multi-match consolidation | Unit tests with multiple match positions in synthetic files (TEST-175) |
| Cross-command integration | Integration tests via `assert_cmd` for backlinks and check (TEST-176) |
| Backward compatibility | Existing SPEC-013 tests pass without modification (no --snippet = no change) |

---

## 7. Observability

| ID | Signal | Type | Condition |
|----|--------|------|-----------|
| OBS-025-001 | `[zetl] snippet: results=N snippets=M links_resolved=K duration_ms=D` | Log (stderr) | When `--verbose` and `--snippet` are both set |
| OBS-025-002 | `[zetl] snippet: mode=auto chose=heading section_len=L` | Log (stderr) | When `--verbose` and auto mode selects heading mode |
| OBS-025-003 | `[zetl] snippet: mode=auto chose=paragraph section_len=L` | Log (stderr) | When `--verbose` and auto mode selects paragraph mode |
| OBS-025-004 | `[zetl] snippet: consolidated M matches into N snippets for page P` | Log (stderr) | When `--verbose` and consolidation merges matches |

---

## 8. Dependencies

### New Module

| Module | Purpose |
|--------|---------|
| `src/snippet/core.rs` | Pure functions: structure map, extraction, consolidation, truncation, link resolution, graph role classification |
| `src/snippet/mod.rs` | Effectful shell: file I/O, integration with search/backlinks/check commands |

### Existing Dependencies (no new crates)

This specification adds no new crate dependencies. All functionality is implemented using existing infrastructure:

- **Heading detection**: Reuses SPEC-013 heading detection logic (ATX heading regex, body text range filtering)
- **Wikilink parsing**: Reuses SPEC-001 / `src/view/link_map.rs` wikilink regex
- **Link graph**: Reuses `src/graph.rs` `LinkGraph::backlinks()` and `LinkGraph::forward_links()`
- **Page set**: Reuses the existing `HashSet<String>` page set from the pipeline

---

## 9. Future Considerations

- **Query-term highlighting.** Snippets could include marker annotations (`<mark>...</mark>` or ANSI escape codes) around matched query terms. Deferred to a presentation-layer spec because the highlighting strategy differs between JSON output (markers), table output (ANSI), and web output (HTML).
- **Intent-aware snippet steering.** When a page has many matches, an LLM could select the most relevant snippet based on inferred query intent. This requires an LLM call in the search path, which violates the local-first, zero-latency principle. Deferred pending user demand.
- **Snippet caching.** For large vaults where the same pages are frequently queried, pre-computed structure maps could be cached in `.zetl/`. Deferred because the per-file cost of structure map building is < 1ms.
- **Web integration.** `zetl serve` could return snippets in the `/api/search` response, and the Cmd+K modal could display them. `zetl build` could include pre-extracted snippets in the static search index. Deferred to a web-layer spec.
- **Configurable graph role thresholds.** The hub/bridge/leaf thresholds (10, 3, 1) could be made configurable via `.zetl/config.toml` for vaults with unusual connectivity patterns.

---

## 10. Relationship to Existing Specs

- **SPEC-013** (Tantivy Search): Parent spec. This spec extends search results with snippet extraction. The `--snippet` flag composes with all existing SPEC-013 flags. The heading detection infrastructure from SPEC-013 is reused.
- **SPEC-018** (Semantic Search): Snippet extraction applies to `--semantic` and `--hybrid` search modes. The snippet extractor operates on match positions, which are produced by both BM25 and vector retrieval.
- **SPEC-002** (Full-Text Search): Original search spec, superseded by SPEC-013 for the search engine but still defines the body text exclusion zones (frontmatter, code blocks, inline code, HTML comments) that snippet extraction must respect.
- **SPEC-003** (Agent Ergonomics): Snippet extraction directly serves agent ergonomics -- the JSON output is designed for LLM consumption, with enough context to assess relevance without fetching full pages.
- **SPEC-001** (Link Graph): The graph context enrichment (backlink count, forward link count, graph role) depends on the link graph built by SPEC-001's pipeline. The wikilink resolution in snippets reuses SPEC-001's page name resolution rules.

See also: [[Spec Index]], [[Search Command]], [[Backlinks Command]], [[Check Command]]
