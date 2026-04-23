---
title: "SPEC-002: ztl search — Full-Text Content Search"
version: 0.1.0
status: draft
audience: agent, human
date: 2026-02-18
---

# SPEC-002: ztl search — Full-Text Content Search

## Information Table

| Field          | Value                                              |
| -------------- | -------------------------------------------------- |
| Document ID    | SPEC-002                                           |
| Title          | ztl search — Full-Text Content Search             |
| Version        | 0.1.0                                              |
| Status         | Draft                                              |
| Author         | Agent (USDD Protocol v1.0.0)                       |
| Date           | 2026-02-18                                         |
| Audience       | Agent, Human                                       |
| Trace          | USDD Agent Protocol v1.0.0                         |
| Parent         | SPEC-001: ztl — Bi-directional Link Graph CLI     |

---

## 1. Overview

SPEC-001 defined ztl as a link-graph tool. It explicitly placed full-text search **out of scope**, deferring to `rg`, `grep`, or dedicated tools. In practice, users reach for `similar` when they want content search, but `similar` is a SimHash page-name matcher — it cannot find arbitrary text inside files.

This specification adds a `search` subcommand that performs fast, local, case-insensitive content search across all Markdown files in the vault. It reuses ztl's existing file-walking and ignore-pattern infrastructure but does **not** require the link graph or SimHash index — it reads raw file contents directly.

### 1.1 Motivation

The gap between `similar` (page-name similarity) and what users actually want (content search) was exposed by a concrete failure: searching `similar "idea"` returned "Notion" (SimHash distance 9, no content match) and "GTD" (distance 11, content match on line 22). The user wanted "find where 'idea' appears in my vault" — a content search, not a name search.

A dedicated `search` command solves this cleanly:

- `similar` remains focused on fuzzy **page name** matching (typo detection, duplicate avoidance)
- `search` handles **content** lookup (find text, get line numbers, see surrounding context)

### 1.2 Design Principles

1. **Fast by default.** No index required. Read files, match text, stream results. For 10,000 average-sized files, complete in under 2 seconds.
2. **Body-text aware.** By default, skip YAML frontmatter, fenced code blocks, inline code, and HTML comments — the same exclusion zones the scanner already computes for wikilink parsing. A `--all` flag searches raw content without exclusions.
3. **Context-rich output.** Every match includes the page name, file path, line number, column, and configurable surrounding text — enough for both agents and humans to locate the result without opening the file.
4. **Consistent interface.** Follows the same patterns as `links`/`backlinks`: `--context N`, JSON/table output, global flags (`--dir`, `--format`, `--no-cache`, etc.).

### 1.3 Scope

**In scope:**

- Case-insensitive literal text search across vault Markdown file contents
- Optional regex mode (`--regex`)
- Body-text-only search (default) and raw-content search (`--all`)
- Line number, column, and configurable context for each match
- Respects `.ztlignore` and default ignore patterns (`.git`, `node_modules`, `.ztl`)
- Result limiting (`--limit`)
- JSON and table output

**Out of scope:**

- Full-text search indexing (inverted index, BM25 ranking, etc.)
- Semantic / embedding-based search (future SPEC, per SPEC-001 section 10)
- Search across non-Markdown files
- Interactive / incremental search (TUI)

---

## 2. User Profiles

The existing user profiles from SPEC-001 (section 2) apply. This specification extends their workflows:

### 2.1 Agent Operator — Extended Workflow

```
Daily workflow (updated):
  1. Create/edit markdown files with [[wikilinks]]
  2. Run `ztl check` to validate link integrity
  3. Run `ztl search "concept"` to find all mentions of a topic across the vault
  4. Run `ztl similar <query>` to find near-duplicate page names before creating new pages
  5. Run `ztl backlinks <page>` to discover link context
```

**Key use case:** An agent creating a new page about "spaced repetition" runs `ztl search "spaced repetition"` to discover all existing mentions — finding related context in files that may not link to each other. This is fundamentally different from `backlinks` (which requires an existing page) and `similar` (which matches page names, not content).

### 2.2 Human Knowledge Worker — Extended Workflow

```
Daily workflow (updated):
  1. Write notes in Obsidian/Logseq/editor
  2. Run `ztl search "meeting notes" --context 60 -f table` to find scattered references
  3. Run `ztl stats` for an overview
  4. Run `ztl check --orphans` to find disconnected notes
```

**Key use case:** A researcher remembers writing about "elaborative encoding" but can't recall which notes contain it. `ztl search "elaborative encoding" --context 40` returns every occurrence with surrounding text, across all files, respecting their ignore patterns.

---

## 3. Requirements

### 3.1 Functional Requirements

```
REQ-013: Full-Text Content Search

The system SHALL search all Markdown files in the vault for occurrences
of a query string, returning every match with:
  - Page name (derived from filename)
  - File path (relative to vault root)
  - Line number (1-indexed)
  - Column number (1-indexed, byte offset of match start within the line)
  - Context snippet (configurable surrounding characters)
FOR all user roles
WITH case-insensitive matching by default
AND results ordered by file path, then line number.

Trace:
- TEST-013
- CON-008
```

```
REQ-014: Body-Text-Only Search (Default)

The system SHALL, by default, exclude matches found within:
  - YAML frontmatter (between `---` delimiters at file start)
  - Fenced code blocks (``` or ~~~)
  - Inline code (`)
  - HTML comments (<!-- -->)
FOR all user roles
WITH the same exclusion logic used by the wikilink parser (SPEC-001 REQ-001)
AND an `--all` flag to disable exclusions and search raw file content.

Trace:
- TEST-014
- CON-008
```

```
REQ-015: Regex Search Mode

The system SHALL support an optional `--regex` flag that interprets the
query as a regular expression instead of a literal string,
FOR all user roles
WITH invalid regex patterns reported as an error with exit code 2.

Trace:
- TEST-015
- CON-008
```

```
REQ-016: Case-Sensitive Search Mode

The system SHALL support a `--case-sensitive` flag that requires
exact case matching instead of the default case-insensitive behaviour,
FOR all user roles.

Trace:
- TEST-016
- CON-008
```

### 3.2 Non-Functional Requirements

```
NFR-006: Search Performance

Full-text search SHALL complete in ≤ 2 seconds for a vault of
10,000 files (average 5 KB each) UNDER single-threaded execution
on commodity hardware WITH 95th percentile.
```

```
NFR-007: Search Memory Usage

Peak memory consumption during search SHALL be ≤ 50 MB above
baseline UNDER a corpus of 10,000 files WITH average file size
of 5 KB, since search processes files sequentially and does not
require the full graph to be in memory.
```

---

## 4. Architecture

### 4.1 Architecture Decision

```
ADR-002: Search Without a Pre-Built Index

Status: Proposed

Context:
  The search command needs to find arbitrary text across all vault files.
  Two approaches were considered:

  Option A — Index-free scan: Walk the file tree, read each file, and
  search for matches. No persistent state. Straightforward and correct.

  Option B — Inverted index: Build and cache a full-text index for fast
  repeated queries. Significantly more complex (tokenization, storage,
  updates, cache invalidation).

Decision:
  Implement Option A (index-free scan).

Rationale:
  - For the target vault size (≤ 10,000 files, ≤ 50 MB total), sequential
    read + search completes in < 2 seconds. This meets NFR-006.
  - No new cache format or invalidation logic required.
  - Consistent with ztl's "fast and disposable" philosophy (SPEC-001 §1.1).
  - The existing file-walk and ignore-pattern infrastructure from the scanner
    module can be reused directly.
  - If vaults grow beyond 10K files, an inverted index can be added in a
    future SPEC without changing the CLI interface.

Consequences:
  + Zero additional storage or cache complexity
  + Correct by construction — always searches current file contents
  + Simple implementation: ~100 lines of new code
  - Linear scan time: O(total_bytes) per query
  - No ranking or relevance scoring (results are positional, not weighted)
  - Repeated queries re-read all files (no amortization)
```

### 4.2 Component Integration

Search integrates with the existing architecture as a lightweight path alongside the scanner:

```
                         ┌──────────────┐
                         │     CLI      │
                         │  (commands)  │
                         └──────┬───────┘
                                │
          ┌─────────────────────┼────────────────┐
          │                     │                 │
   ┌──────▼──────┐       ┌─────▼──────┐   ┌─────▼──────┐
   │   Scanner    │       │   Graph    │   │  SimHash   │
   │              │       │   Engine   │   │  Index     │
   └──────┬───────┘       └────────────┘   └────────────┘
          │
   ┌──────▼──────┐
   │   Search    │  NEW — reuses scanner's file walk
   │             │  and exclusion-range computation
   │ - walk files│
   │ - compute   │
   │   exclusions│
   │ - match text│
   │ - emit hits │
   └─────────────┘
```

**Search does NOT depend on the link graph or SimHash index.** It reuses only:

1. **File walking** — `scan_vault()` already walks the directory tree respecting ignore patterns. Search needs the same file list but not the parsed wikilinks.
2. **Exclusion ranges** — the scanner already computes which byte ranges are frontmatter, code blocks, inline code, and HTML comments. Search reuses this to implement body-text-only matching.

### 4.3 Data Model

```rust
/// A single search match
struct SearchMatch {
    page: String,          // page name (filename sans .md)
    path: String,          // relative path from vault root
    line: u32,             // 1-indexed line number
    column: u32,           // 1-indexed column (byte offset within line)
    context: Option<String>, // surrounding characters (when --context > 0)
}

/// Output envelope for the search command
struct SearchOutput {
    query: String,         // the search query
    regex: bool,           // whether regex mode was used
    total_matches: usize,  // total matches found (before limit)
    results: Vec<SearchMatch>,
}
```

### 4.4 Search Algorithm

For each file in the vault (respecting ignore patterns):

1. Read the file contents into a string.
2. If body-text-only mode (default): compute exclusion ranges using the scanner's existing logic. Build a set of excluded byte ranges.
3. For each line in the file:
   a. Find all occurrences of the query (case-insensitive by default, or regex).
   b. For each occurrence, compute its byte offset within the file.
   c. If body-text-only: skip the match if its byte range overlaps any exclusion range.
   d. Emit a `SearchMatch` with page name, path, line, column, and context snippet.
4. If `--limit` is reached, stop early.

**Context extraction:** For a match at position `pos` of length `len` on a line, extract characters from `max(0, pos - N)` to `min(line_len, pos + len + N)` where N is the `--context` value.

---

## 5. Contract Specification (CLI Interface)

```
CON-008: search

ztl search <QUERY> [OPTIONS]

Search vault file contents for text matching the query.

Arguments:
  <QUERY>  Search string (literal text, or regex with --regex)

Options:
  --context <N>       Include N characters of surrounding text [default: 0]
  --limit <N>         Max results to return [default: 50]
  --regex             Interpret query as a regular expression
  --case-sensitive    Require exact case match (default: case-insensitive)
  --all               Search raw file content (include frontmatter, code blocks,
                      comments). Default: search body text only.

Exit codes:
  0  Matches found
  1  No matches found
  2  Invalid query (e.g., bad regex syntax)

Example output (JSON):
{
  "query": "idea",
  "regex": false,
  "total_matches": 3,
  "results": [
    {
      "page": "GTD",
      "path": "concepts/GTD.md",
      "line": 22,
      "column": 26,
      "context": "knowledge** (ideas, concepts, insights)"
    },
    {
      "page": "Elaborative Encoding",
      "path": "concepts/Elaborative Encoding.md",
      "line": 8,
      "column": 15,
      "context": "connecting new ideas to existing knowledge"
    },
    {
      "page": "Evergreen Notes",
      "path": "concepts/Evergreen Notes.md",
      "line": 5,
      "column": 42,
      "context": "notes that develop one idea fully and are"
    }
  ]
}

Example output (table):
Search results for 'idea':
 Page                  | Line | Col | Context
-----------------------+------+-----+------------------------------------------
 GTD                   |   22 |  26 | knowledge** (ideas, concepts, insights)
 Elaborative Encoding  |    8 |  15 | connecting new ideas to existing knowledge
 Evergreen Notes       |    5 |  42 | notes that develop one idea fully and are

Implements:
- REQ-013, REQ-014, REQ-015, REQ-016

Verified by:
- TEST-013, TEST-014, TEST-015, TEST-016
```

---

## 6. Test Specifications

```
TEST-013: Basic Content Search

Scenario: Search for literal text across vault files
Given: A vault with 3 files:
  - "Alpha.md" containing "The quick brown fox" on line 3
  - "Beta.md" containing "A quick summary" on line 5
  - "Gamma.md" containing "No match here"
When: `ztl search "quick"` is run
Then:
  - Returns 2 results (Alpha line 3, Beta line 5)
  - Each result has page, path, line, column
  - Gamma.md does not appear
  - Results are ordered by path, then line number

Scenario: Search with context
Given: Same vault
When: `ztl search "quick" --context 10` is run
Then:
  - Each result has a context field with surrounding text
  - Alpha result context includes "The quick brown"
  - Beta result context includes "A quick summa"

Scenario: No matches
Given: Same vault
When: `ztl search "nonexistent"` is run
Then:
  - Returns empty results array with total_matches: 0
  - Exit code 1

Verifies: REQ-013
```

```
TEST-014: Body-Text Exclusion

Scenario: Frontmatter excluded by default
Given: A file with YAML frontmatter containing "title: Quick Start"
       and body text not containing "Quick Start"
When: `ztl search "Quick Start"` is run (default body-text mode)
Then:
  - No match found in that file

Scenario: Frontmatter included with --all
When: `ztl search "Quick Start" --all` is run
Then:
  - Match found in the frontmatter line

Scenario: Code block excluded by default
Given: A file with a fenced code block containing "quick_sort()" on line 10
       and body text containing "quick overview" on line 3
When: `ztl search "quick"` is run
Then:
  - Returns match at line 3 (body text)
  - Does NOT return match at line 10 (code block)

Scenario: Code block included with --all
When: `ztl search "quick" --all` is run
Then:
  - Returns matches at both line 3 and line 10

Verifies: REQ-014
```

```
TEST-015: Regex Search

Scenario: Regex pattern matching
Given: A vault with files containing "note", "notes", and "notation"
When: `ztl search "note[s]?" --regex` is run
Then:
  - Matches "note" and "notes" but not "notation"
    (since the pattern anchors to "note" or "notes" exactly within
    longer words — actually regex is unanchored, so "notation"
    would also match "note" within it. Test should use word
    boundaries if needed: `\bnotes?\b`)

Scenario: Invalid regex
When: `ztl search "[invalid" --regex` is run
Then:
  - Exit code 2
  - Error message describes the regex syntax problem

Verifies: REQ-015
```

```
TEST-016: Case Sensitivity

Scenario: Default case-insensitive search
Given: A file containing "Zettelkasten" on line 1 and "zettelkasten" on line 5
When: `ztl search "ZETTELKASTEN"` is run
Then:
  - Returns both matches (lines 1 and 5)

Scenario: Case-sensitive search
When: `ztl search "Zettelkasten" --case-sensitive` is run
Then:
  - Returns only line 1
  - Line 5 ("zettelkasten") is not matched

Verifies: REQ-016
```

```
TEST-017: Search Respects Ignore Patterns

Scenario: Ignored directories are not searched
Given: A vault with `.ztlignore` containing "drafts/"
       A file "drafts/Draft.md" containing "secret draft content"
       A file "Public.md" containing "public content"
When: `ztl search "content"` is run
Then:
  - Returns match from Public.md
  - Does NOT return match from drafts/Draft.md

Scenario: Default ignores apply
Given: Files under .git/ and node_modules/ containing searchable text
When: `ztl search "text"` is run
Then:
  - .git/ and node_modules/ files are not searched

Verifies: REQ-013 (inherits ignore behaviour from REQ-012)
```

```
TEST-018: Search Result Limiting

Scenario: Limit caps returned results
Given: A vault where "the" appears 100 times across files
When: `ztl search "the" --limit 5` is run
Then:
  - results array contains exactly 5 entries
  - total_matches reports the full count (100)

Verifies: REQ-013
```

---

## 7. Observability

```
OBS-003: Search Timing

When --verbose is specified, the search command SHALL emit to stderr:
  - Number of files scanned
  - Number of matches found
  - Elapsed time in milliseconds
to support performance monitoring.
```

---

## 8. Traceability Matrix

| REQ     | CON     | TEST              | OBS     |
| ------- | ------- | ----------------- | ------- |
| REQ-013 | CON-008 | TEST-013, TEST-017, TEST-018 | OBS-003 |
| REQ-014 | CON-008 | TEST-014          | ---     |
| REQ-015 | CON-008 | TEST-015          | ---     |
| REQ-016 | CON-008 | TEST-016          | ---     |

---

## 9. Impact on SPEC-001

### 9.1 Scope Update

SPEC-001 section 1.2 lists "Full-text search (defer to `rg`, `grep`, or dedicated tools)" as **out of scope**. This SPEC narrows that exclusion: ztl now provides basic content search over its own vault files, but does **not** replace general-purpose tools like `rg` for complex queries across arbitrary file types.

### 9.2 Relationship to `similar`

The `similar` command (REQ-008, CON-005) remains unchanged. It is a **page name** similarity tool using SimHash fingerprints. The `search` command is a **content** search tool. They serve different purposes:

| Dimension      | `similar`                           | `search`                          |
| -------------- | ----------------------------------- | --------------------------------- |
| Searches       | Page names                          | File contents                     |
| Algorithm      | SimHash + Hamming distance          | Literal text / regex match        |
| Use case       | Typo detection, duplicate avoidance | Find mentions of a topic          |
| Requires index | SimHash fingerprints (computed)     | None (reads files directly)       |
| Ranking        | By Hamming distance                 | By file path, then line number    |

### 9.3 `similar` Line/Context Fields

The `line` and `context` fields recently added to `SimilarResult` are a stopgap. With a proper `search` command available, these fields on `similar` become less important. They MAY be retained for convenience (showing where the query text happens to appear in a name-matched file) or removed in a future version to keep `similar` focused on its core purpose.

---

## 10. Open Questions

1. **Should `search` support `--page` to restrict search to a single file?** This would be convenient for agents that already know which page to look in. Recommendation: defer — agents can use `rg` for single-file search. Add if users request it.

2. **Should multiple matches on the same line produce one result or multiple?** Current design: one result per match occurrence (column distinguishes them). This matches `rg` behaviour and is more useful for agents parsing the output.

3. **Should results include a match-count-per-file summary?** For agents, per-occurrence results are more useful. A summary could be added as an optional `--count` mode in the future, similar to `rg -c`.

4. **Should `search` integrate with the graph?** For example, "search for X but only in pages that link to Y." This is a powerful cross-cutting query but significantly increases complexity. Recommendation: defer to a future SPEC. Users can compose `search` and `backlinks` output programmatically.

---

**END OF SPEC-002**
