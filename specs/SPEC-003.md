---
title: "SPEC-003: zetl — Agent Ergonomics & Robustness"
version: 0.1.0
status: draft
audience: agent, human
date: 2026-02-18
---

# SPEC-003: zetl — Agent Ergonomics & Robustness

## Information Table

| Field          | Value                                              |
| -------------- | -------------------------------------------------- |
| Document ID    | SPEC-003                                           |
| Title          | zetl — Agent Ergonomics & Robustness               |
| Version        | 0.1.0                                              |
| Status         | Draft                                              |
| Author         | Agent (USDD Protocol v1.0.0)                       |
| Date           | 2026-02-18                                         |
| Audience       | Agent, Human                                       |
| Trace          | USDD Agent Protocol v1.0.0                         |
| Parent         | SPEC-001: zetl — Bi-directional Link Graph CLI     |
| Related        | SPEC-002: zetl search — Full-Text Content Search   |

---

## 1. Overview

This specification addresses issues discovered by running zetl as an LLM agent would — exercising every command, piping JSON output for analysis, handling errors, and attempting common multi-step workflows. The issues fall into three categories:

1. **Crash bugs** — inputs that cause panics instead of graceful errors
2. **Broken agent contract** — errors that violate the JSON output contract, making programmatic parsing impossible
3. **Missing capabilities** — commands and options that agents need for common workflows but don't exist

### 1.1 Motivation

An LLM agent using zetl as a tool does:

```
output = shell("zetl -f json <command>")
data = json.parse(output)
# use data to make decisions
```

This breaks when:
- **The process panics** (exit 101, garbled output on stderr) — agent sees an unrecoverable error
- **Errors go to stderr as plain text** (`Page not found: 'foo'`) while the agent expects JSON on stdout — `json.parse()` fails on empty input
- **Output contains duplicate entries** — agent wastes tokens processing redundant data and may make incorrect counts
- **There's no command for a basic operation** — agent has to fall back to raw file I/O, losing vault-awareness

### 1.2 Discovery Method

Simulated agent session against `demo-vault` (25 files, 199 links). Every command was exercised with JSON output, results were piped through `python3 -c "json.load(sys.stdin)"` to validate parseability. Edge cases (empty queries, nonexistent pages, special characters, multi-hop traversals) were tested systematically.

### 1.3 Scope

**In scope:**

- Fix crash on empty search query (P0 — data loss / panic)
- Structured JSON error responses for all error paths
- Deduplicated link/backlink results
- `list` command for page enumeration
- `--path` / `--glob` filter for search scoping
- Graph export (adjacency list JSON)

**Out of scope:**

- Tag/frontmatter extraction (future SPEC, per SPEC-001 §10)
- Combined query operations (search + graph intersection)
- Interactive/TUI features
- Watch mode

---

## 2. User Profiles

The existing user profiles from SPEC-001 §2 apply. This specification extends the **Agent Operator** profile with observed friction points.

### 2.1 Agent Operator — Observed Session

```
Role: LLM coding agent (Claude Code)
Workflow observed:
  1. `zetl -f json stats` → understand vault shape                    ✓ works
  2. `zetl -f json check` → find problems                            ✓ works (exit 1 expected)
  3. `zetl -f json links "Zettelkasten" --depth 1` → explore page    ✗ 14 entries, 9 unique targets, massive duplication
  4. `zetl -f json links "..." --depth 3` → deep traversal           ✗ 153 entries, agent overwhelmed
  5. `zetl -f json search "" --limit 1` → edge case                  ✗ PANIC (byte boundary)
  6. `zetl -f json links "nonexistent"` → error handling              ✗ plain text on stderr, no JSON
  7. `zetl -f json list` → enumerate all pages                        ✗ command does not exist
  8. `zetl -f json search "note" --path concepts/` → scoped search   ✗ no --path filter
  9. JSON piped to python3 → parse error data                         ✗ errors not JSON-formatted

Friction points:
  - Every error path requires try/catch on JSON parse failure
  - Duplicate entries in links/backlinks force post-processing dedup
  - No way to get a flat list of all pages without parsing stats
  - No way to scope search to a subdirectory
  - No way to export the full graph for custom analysis
```

### 2.2 Happy Path: Agent Explores Unknown Vault

```
Preconditions: Agent has vault path, no prior knowledge of content
Steps:
  1. `zetl -f json index -d /path` → index vault
     Expected: JSON with files_scanned, links_found
  2. `zetl -f json list -d /path` → get all page names and paths          [NEW]
     Expected: JSON array of {page, path} objects
  3. `zetl -f json stats -d /path` → understand shape
     Expected: JSON with pages, links, orphans, most_linked
  4. `zetl -f json links "TopPage" --depth 1 --context 30` → explore
     Expected: deduplicated link entries with context                      [FIXED]
  5. `zetl -f json search "concept" --context 40 --path concepts/` → find content  [NEW]
     Expected: only results from concepts/ subdirectory
  6. `zetl -f json links "nonexistent"` → handle missing page
     Expected: JSON error object with error field                          [FIXED]
Postconditions: Agent has structured knowledge of vault topology and content
Failure modes:
  - Empty vault → stats returns zeros (currently works)
  - Bad page name → JSON error (currently broken: plain text stderr)
```

---

## 3. Requirements

### 3.1 Functional Requirements — Bug Fixes (P0)

```
REQ-019: Graceful Handling of Empty Search Query

The system SHALL reject an empty search query string with a structured
error message and exit code 2, instead of panicking,
FOR all user roles
WITH the error message "Empty search query" in JSON format when -f json
is specified.

Trace:
- TEST-019
- CON-008 (extends)
```

```
REQ-020: Structured JSON Error Responses

The system SHALL, when -f json is specified, output all error messages
as JSON objects on stdout with the structure:
  {"error": "<message>", "code": <exit_code>}
instead of plain text on stderr,
FOR all user roles
WITH exit codes preserved (1 for not-found, 2 for invalid input)
AND stderr reserved for warnings/verbose output only.

Rationale: An agent parsing `json.load(stdout)` currently gets a
JSONDecodeError when errors produce plain text on stderr and empty
stdout. This makes every zetl call require try/catch wrapping and
stderr inspection, which is fragile and non-standard.

Trace:
- TEST-020
- CON-009
```

### 3.2 Functional Requirements — Quality Improvements (P1)

```
REQ-021: Deduplicated Link Results

The system SHALL deduplicate forward link and backlink results such that
each (source, target, line) triple appears at most once in the output,
FOR all user roles
WITH the total count reflecting unique entries.

Current behaviour: `zetl links "Zettelkasten" --depth 1` returns 14
entries for 9 unique targets. "Atomic Notes" appears 4 times,
"Folder-based Organization" appears twice, "Logseq" appears twice.
This is because a single line can contain multiple wikilinks and the
graph stores an edge per link occurrence — but for the agent, each
(source, target, line) triple is the same fact repeated.

Trace:
- TEST-021
- CON-003 (extends)
```

```
REQ-022: Context Extraction UTF-8 Safety in extract_context

The system SHALL safely handle multi-byte UTF-8 characters (em-dash,
accented characters, CJK, emoji) when extracting context around
wikilinks in `links` and `backlinks` commands,
FOR all user roles
WITH byte-offset slicing snapped to character boundaries using the
same floor_char_boundary/ceil_char_boundary approach used in
search (SPEC-002).

Current behaviour: `extract_context()` in main.rs line 836 does
`target_line[ctx_start..ctx_end]` without checking char boundaries.
This will panic on files containing multi-byte characters near
wikilinks. The search module already has this fix
(floor_char_boundary/ceil_char_boundary in search.rs) — the same
pattern should be applied to the wikilink context extractor.

Trace:
- TEST-022
- CON-003 (extends)
```

### 3.3 Functional Requirements — New Capabilities (P2)

```
REQ-023: List All Pages

The system SHALL provide a `list` subcommand that returns all pages
in the vault as an array of objects, each containing:
  - page: String (page name)
  - path: String (relative path from vault root)
FOR all user roles
WITH results sorted alphabetically by page name
AND output in JSON (default) or table format.

Rationale: An agent exploring an unknown vault currently has no way
to enumerate pages. `stats` shows counts and top-linked pages, but
not a full list. The agent must fall back to `find . -name '*.md'`
and manually derive page names, losing vault-awareness (.zetlignore
patterns, page name resolution logic).

Trace:
- TEST-023
- CON-010
```

```
REQ-024: Search Path Filter

The system SHALL support an optional `--path <GLOB>` flag on the
`search` command that restricts results to files matching the glob
pattern (relative to vault root),
FOR all user roles
WITH the glob matching using gitignore-style syntax (e.g., "concepts/",
"**/practice*.md").

Rationale: In a vault with 10,000 files, an agent often knows which
subdirectory to search. Without --path, the agent gets results from
all directories and must post-filter, wasting output tokens. During
testing, search for "note" returned 134 matches: 84 from concepts/,
14 from tools/, 2 from people/ — agent only needed concepts/.

Trace:
- TEST-024
- CON-008 (extends)
```

```
REQ-025: Graph Export

The system SHALL provide an `export` subcommand that outputs the
complete link graph as a JSON adjacency list:
  {
    "nodes": [{"page": "...", "path": "..."}],
    "edges": [{"source": "...", "target": "...", "line": N}]
  }
FOR all user roles
WITH edges deduplicated to unique (source, target) pairs.

Rationale: An agent doing custom graph analysis (community detection,
centrality, cluster identification) currently has to reconstruct the
graph by calling `links` for every single page. For a 1,000-page
vault, that's 1,000 CLI invocations. A single `export` call gives
the agent the complete graph for in-memory analysis.

Trace:
- TEST-025
- CON-011
```

### 3.4 Non-Functional Requirements

```
NFR-008: Error Response Latency

Error responses (page not found, invalid input) SHALL return in
≤ 10ms UNDER all conditions WITH 99th percentile.

Rationale: Currently, error paths like `links "nonexistent"` still
run the full pipeline (scan files, build graph) before discovering
the page doesn't exist. For error cases, the pipeline is wasted work
— but the latency is still acceptable for small vaults. This NFR
ensures errors don't regress.
```

```
NFR-009: List Command Performance

The `list` command SHALL return all pages in ≤ 100ms for a vault
of 10,000 files UNDER single-threaded execution WITH 95th percentile.

Rationale: Since `list` only needs page names and paths (no graph
construction), it should be faster than a full pipeline run.
```

---

## 4. Architecture

### 4.1 Architecture Decisions

```
ADR-003: Structured JSON Errors via Output Envelope

Status: Proposed

Context:
  Currently, errors in zetl are emitted in two ways:
  1. `eprintln!("{e}"); std::process::exit(1)` — plain text on stderr
  2. `anyhow::Result` propagated to main — printed by anyhow's handler

  Neither produces JSON. An agent calling `zetl -f json links "bad"`
  gets empty stdout and "Page not found: 'bad'" on stderr. The agent's
  JSON parser fails.

  Three approaches were considered:

  Option A — Always output JSON errors on stdout:
    {"error": "Page not found: 'bad'", "code": 1}
    Pros: Agent always gets parseable JSON. Simple.
    Cons: Mixes success and error types on stdout.

  Option B — Output JSON errors only when -f json is specified:
    Same as A but only when format=json. Plain text errors for
    table mode (existing behaviour).
    Pros: Backwards compatible for human users.
    Cons: Two code paths.

  Option C — Use stderr with JSON when -f json:
    Pros: Clean separation of success/error streams.
    Cons: Agents typically only capture stdout for JSON parsing.
    Most shell wrappers (`subprocess.check_output`) capture stdout only.

Decision:
  Implement Option B — JSON errors on stdout when -f json, plain text
  on stderr for table mode.

Rationale:
  - Agents can reliably parse stdout as JSON in all cases
  - Human users see plain text errors as before
  - The `"error"` key distinguishes error responses from success responses
  - Exit codes are preserved for shell scripting (0=success, 1=not-found, 2=bad-input)

Consequences:
  + Agent workflow: json.parse(stdout) always succeeds
  + Backwards compatible for table format users
  - Agents must check for "error" key in parsed JSON
  - Success/error types are different shapes (not a union type)
```

```
ADR-004: Link Deduplication Strategy

Status: Proposed

Context:
  A single source page can reference the same target multiple times on
  the same line (e.g., a line with multiple wikilinks all referencing
  the same page through aliases or repeated mentions). The graph
  correctly stores each edge, but the query output for `links` and
  `backlinks` produces duplicate entries that are noise for agents.

  Testing showed:
  - `links "Zettelkasten" --depth 1`: 14 entries, 9 unique targets
  - `links "Zettelkasten" --depth 3`: 153 entries, ~35 unique targets

  Two approaches:

  Option A — Deduplicate at query output:
    Filter results to unique (source, target, line) triples before
    serialization. Keep all edges in the graph for accurate stats.
    Pros: Graph is still complete for stats/path queries.
    Cons: Multi-hop BFS still visits duplicates internally.

  Option B — Add --unique flag:
    Let the user opt in to dedup. Default preserves current behaviour.
    Pros: Backwards compatible.
    Cons: Agents must always remember to pass --unique.

Decision:
  Implement Option A — deduplicate by default at the output layer.

Rationale:
  - No agent or human benefits from seeing the same (source, target, line) twice
  - The underlying graph retains all edges for accurate path/stats computation
  - If a user genuinely wants raw edge data, the `export` command (REQ-025)
    can provide unfiltered edges
```

### 4.2 Component Impact

```
No new modules required. Changes are to existing components:

CLI (cli.rs):
  - Add `List` and `Export` subcommand variants
  - Add `--path` flag to `Search`

Main (main.rs):
  - Add error_json() helper for structured error output
  - Modify find_page() to return JSON error when -f json
  - Deduplicate entries in cmd_links() and cmd_backlinks()
  - Apply floor_char_boundary/ceil_char_boundary to extract_context()
  - Add cmd_list() and cmd_export() handlers

Search (search.rs):
  - Reject empty query with anyhow error
  - Support path filter in SearchConfig
```

---

## 5. Contract Specifications

```
CON-009: Structured Error Response (JSON mode)

When -f json is specified and an error occurs, zetl SHALL output
a JSON object on stdout:

{
  "error": "Page not found: 'nonexistent'",
  "code": 1
}

And exit with the corresponding code.

Error codes:
  1  Not found (page not found, no matches, no path)
  2  Invalid input (bad regex, empty query)

When -f table is specified, errors continue to go to stderr as plain
text (no change from current behaviour).

Implements:
- REQ-020

Verified by:
- TEST-020
```

```
CON-010: list

zetl list [OPTIONS]

List all pages in the vault.

Arguments: none

Options: (global flags only: --dir, --format, etc.)

Exit codes:
  0  Always (empty vault returns empty array)

Example output (JSON):
{
  "pages": [
    {"page": "Atomic Notes", "path": "concepts/Atomic Notes.md"},
    {"page": "Bidirectional Links", "path": "concepts/Bidirectional Links.md"},
    {"page": "Daily Note Practice", "path": "practices/Daily Note Practice.md"}
  ],
  "total": 3
}

Example output (table):
 Page                  | Path
-----------------------+------------------------------------------
 Atomic Notes          | concepts/Atomic Notes.md
 Bidirectional Links   | concepts/Bidirectional Links.md
 Daily Note Practice   | practices/Daily Note Practice.md

Implements:
- REQ-023

Verified by:
- TEST-023
```

```
CON-011: export

zetl export [OPTIONS]

Export the complete link graph as a JSON adjacency list.

Options:
  --edges-only    Omit node list, output only edges [default: false]

Exit codes:
  0  Always

Example output (JSON):
{
  "nodes": [
    {"page": "Atomic Notes", "path": "concepts/Atomic Notes.md"},
    {"page": "Zettelkasten", "path": "concepts/Zettelkasten.md"}
  ],
  "edges": [
    {"source": "Atomic Notes", "target": "Zettelkasten"},
    {"source": "Zettelkasten", "target": "Atomic Notes"}
  ],
  "node_count": 2,
  "edge_count": 2
}

Implements:
- REQ-025

Verified by:
- TEST-025
```

```
CON-008 (extended): search --path filter

zetl search <QUERY> [OPTIONS]

Additional option:
  --path <GLOB>   Restrict results to files matching glob
                  (relative to vault root, gitignore syntax)

Example:
  zetl search "note" --path "concepts/" --context 30

Only files under concepts/ are searched.

Implements:
- REQ-024

Verified by:
- TEST-024
```

---

## 6. Test Specifications

```
TEST-019: Empty Search Query Rejection

Scenario: Empty string search
Given: Any vault
When: `zetl -f json search ""` is run
Then:
  - Exit code 2
  - Output is JSON: {"error": "Empty search query", "code": 2}
  - No panic, no backtrace output

Scenario: Whitespace-only search
When: `zetl -f json search "   "` is run
Then:
  - Same as empty string — rejected with exit code 2

Verifies: REQ-019
```

```
TEST-020: Structured JSON Error Responses

Scenario: Page not found in JSON mode
Given: A vault with pages A, B, C
When: `zetl -f json links "nonexistent"` is run
Then:
  - Exit code 1
  - Stdout contains: {"error": "Page not found: 'nonexistent'", "code": 1}
  - Stderr is empty

Scenario: Page not found in table mode
Given: Same vault
When: `zetl -f table links "nonexistent"` is run
Then:
  - Exit code 1
  - Stderr contains: "Page not found: 'nonexistent'"
  - Stdout is empty
  - (Existing behaviour, unchanged)

Scenario: No path found in JSON mode
Given: Two pages in disconnected components
When: `zetl -f json path "A" "disconnected"` is run
Then:
  - Exit code 1
  - Stdout contains JSON with "error" key

Scenario: Invalid regex in JSON mode
When: `zetl -f json search "[bad" --regex` is run
Then:
  - Exit code 2
  - Stdout contains: {"error": "Invalid regex: ...", "code": 2}

Verifies: REQ-020
```

```
TEST-021: Deduplicated Link Results

Scenario: Forward links are deduplicated
Given: Page "Zettelkasten" links to "Atomic Notes" from lines 8 and 12
When: `zetl -f json links "Zettelkasten" --depth 1` is run
Then:
  - Each (source, target, line) triple appears exactly once
  - Total entries ≤ number of unique (source, target, line) triples

Scenario: Depth-3 deduplication
Given: A vault with interconnected pages
When: `zetl -f json links "X" --depth 3` is run
Then:
  - No two entries share the same (source, target, line) triple
  - Unique target count is trackable without agent-side dedup

Verifies: REQ-021
```

```
TEST-022: UTF-8 Safety in Context Extraction

Scenario: Em-dash near wikilink
Given: A file containing "The method — see [[Page]] for details"
When: `zetl -f json links "SourcePage" --context 30` is run
Then:
  - Context is extracted without panic
  - Context string is valid UTF-8
  - Multi-byte character is either fully included or fully excluded

Scenario: CJK characters in file
Given: A file containing "知识管理 [[Knowledge Management]] 方法论"
When: `zetl -f json links "SourcePage" --context 10` is run
Then:
  - No panic
  - Context contains valid UTF-8

Verifies: REQ-022
```

```
TEST-023: List All Pages

Scenario: List pages in a vault
Given: A vault with 5 Markdown files
When: `zetl -f json list` is run
Then:
  - Returns JSON with "pages" array containing 5 entries
  - Each entry has "page" and "path" fields
  - Entries are sorted alphabetically by page name
  - "total" field equals 5

Scenario: Empty vault
Given: A directory with no .md files
When: `zetl -f json list` is run
Then:
  - Returns {"pages": [], "total": 0}
  - Exit code 0

Scenario: Respects .zetlignore
Given: A vault with .zetlignore containing "drafts/"
When: `zetl -f json list` is run
Then:
  - Files under drafts/ do not appear in the list

Verifies: REQ-023
```

```
TEST-024: Search Path Filter

Scenario: Filter to subdirectory
Given: A vault with files in concepts/, tools/, and people/
       All containing the word "note"
When: `zetl -f json search "note" --path "concepts/"` is run
Then:
  - All results have paths starting with "concepts/"
  - No results from tools/ or people/

Scenario: Glob pattern
When: `zetl -f json search "method" --path "**/*Practice*"` is run
Then:
  - Only files with "Practice" in their name are searched

Scenario: No --path (default)
When: `zetl -f json search "note"` is run
Then:
  - Results from all directories (existing behaviour, unchanged)

Verifies: REQ-024
```

```
TEST-025: Graph Export

Scenario: Export full graph
Given: A vault with pages A→B, B→C, C→A (triangle)
When: `zetl -f json export` is run
Then:
  - "nodes" contains 3 entries with page and path
  - "edges" contains 3 entries (A→B, B→C, C→A)
  - Edges are deduplicated to unique (source, target) pairs
  - "node_count" is 3, "edge_count" is 3

Scenario: Export includes dead link targets
Given: Page A links to nonexistent page "Ghost"
When: `zetl -f json export` is run
Then:
  - "Ghost" appears in nodes (with path: null or empty)
  - Edge A→Ghost appears in edges
  - This matches the graph's actual structure (dead links are edges
    to unresolved nodes)

Verifies: REQ-025
```

---

## 7. Observability

```
OBS-004: Error Telemetry

When --verbose is specified, error responses SHALL emit to stderr:
  - The command that failed
  - The specific error condition
  - Any suggestions (e.g., "did you mean 'X'?" for page-not-found)
to support debugging without breaking JSON output on stdout.
```

---

## 8. Traceability Matrix

| REQ     | CON            | TEST     | ADR     | OBS     | Priority |
| ------- | -------------- | -------- | ------- | ------- | -------- |
| REQ-019 | CON-008 (ext)  | TEST-019 | —       | —       | P0       |
| REQ-020 | CON-009        | TEST-020 | ADR-003 | OBS-004 | P0       |
| REQ-021 | CON-003 (ext)  | TEST-021 | ADR-004 | —       | P1       |
| REQ-022 | CON-003 (ext)  | TEST-022 | —       | —       | P1       |
| REQ-023 | CON-010        | TEST-023 | —       | —       | P2       |
| REQ-024 | CON-008 (ext)  | TEST-024 | —       | —       | P2       |
| REQ-025 | CON-011        | TEST-025 | —       | —       | P2       |

---

## 9. Implementation Priority

### P0 — Critical (agent workflow is broken)

| Issue | Impact | Effort |
| ----- | ------ | ------ |
| REQ-019: Empty query panic | Process crash, exit 101, garbled output | 15 min |
| REQ-020: JSON error responses | Every error path breaks agent JSON parsing | 1 hour |

### P1 — Important (agent gets wrong/noisy data)

| Issue | Impact | Effort |
| ----- | ------ | ------ |
| REQ-021: Link dedup | Agent counts wrong, wastes tokens on dupes | 30 min |
| REQ-022: UTF-8 context safety | Latent panic on non-ASCII files | 15 min |

### P2 — Enhancement (agent needs workarounds)

| Issue | Impact | Effort |
| ----- | ------ | ------ |
| REQ-023: `list` command | Agent can't enumerate pages | 30 min |
| REQ-024: `search --path` | Agent can't scope searches | 30 min |
| REQ-025: `export` command | Agent rebuilds graph from N queries | 1 hour |

---

## 10. Findings Log

Raw observations from the agent testing session, preserved for traceability.

### 10.1 Crash (P0)

**FINDING-001: Empty search query panics on UTF-8 boundary**

```
$ zetl -f json search "" --limit 1
thread 'main' panicked at src/search.rs:182:49:
byte index 313 is not a char boundary; it is inside '—' (bytes 312..315)
```

Root cause: Empty pattern matches at every byte position. `start = byte_offset + 1` (search.rs:184) advances by 1 byte, which lands inside a multi-byte character. `search_content[start..]` panics because `start` isn't a char boundary.

Fix: Reject empty queries before entering the match loop. Add `if query.is_empty() { return Err(anyhow!("Empty search query")); }` at the top of `search_vault()`.

### 10.2 Broken Agent Contract (P0)

**FINDING-002: Error messages are plain text, not JSON**

```
$ zetl -f json links "nonexistent" 2>&1
Page not found: 'nonexistent'
```

Agent runs `json.parse(stdout)` → JSONDecodeError. The `find_page()` function uses `eprintln!()` and `std::process::exit(1)` regardless of output format.

**FINDING-003: `check` exit code 1 with valid JSON is confusing but correct**

`check` returns exit 1 with issues found, but the JSON is valid. This is actually fine — the exit code signals "issues found," not "error." Agents should handle this. No change needed, but worth documenting.

### 10.3 Duplicate Data (P1)

**FINDING-004: Links output has duplicate entries**

```
$ zetl -f json links "Zettelkasten" --depth 1
Total: 14 entries, Unique targets: 9
Duplicates: {'Logseq': 2, 'Folder-based Organization': 2, 'Atomic Notes': 4}
```

The graph stores one edge per wikilink occurrence on a line. A line like `[[Obsidian]] and [[Logseq]]` produces correct edges, but if the same target appears in multiple edges from the same source line (due to the BFS visiting edges, not unique targets), duplicates appear.

**FINDING-005: Depth-3 traversal explodes**

```
$ zetl -f json links "Daily Note Practice" --depth 3
Total: 153 entries
  hop 1: 7 links (6 unique targets)
  hop 2: 58 links (18 unique targets)
  hop 3: 88 links (17 unique targets)
```

Even with dedup, depth-3 on a well-connected vault returns a lot of data. Dedup would reduce this significantly. A `--unique-targets` flag could further collapse to one entry per target.

### 10.4 UTF-8 Safety (P1)

**FINDING-006: extract_context() doesn't check char boundaries**

`main.rs:836` does `target_line[ctx_start..ctx_end].to_string()` without verifying char boundaries. Files with em-dashes, accented characters, or CJK near wikilinks will trigger the same panic class as FINDING-001. The search module already has `floor_char_boundary()` and `ceil_char_boundary()` — same pattern should be applied.

### 10.5 Missing Capabilities (P2)

**FINDING-007: No `list` command**

Agent ran `zetl list` → `error: unrecognized subcommand 'list'`. Agent had to parse `stats` output and use `most_linked` as a partial page list, or fall back to `find . -name '*.md'`.

**FINDING-008: No search path filter**

Agent searching for "note" got 134 matches across 3 directories. Needed only concepts/. Had to post-filter: `results.filter(r => r.path.startsWith("concepts/"))` — wasting output tokens and processing time.

**FINDING-009: No graph export**

Agent wanting to do graph analysis (find clusters, bridges) had to call `links` for each page individually. For a 25-page vault that's manageable; for 1,000 pages it's 1,000 CLI calls.

**FINDING-010: `similar` returns very few results**

`zetl similar "zettelkasten"` returns only 1 result (exact match, distance 0). The SimHash fingerprint approach with current threshold=12 is too strict for short queries. This is a known limitation (SimHash works on page names, not content) and is addressed by `search` in SPEC-002.

**FINDING-011: `cargo run` build output contaminates stdout piping**

When using `cargo run --release -- ...`, cargo's build messages go to stderr but duplicate invocations sometimes mix streams. Not a zetl bug — just a workflow note. Agent should use the binary directly: `./target/release/zetl`.

---

## 11. Open Questions

1. **Should `list` include link counts?** Adding `forward_link_count` and `backlink_count` to each entry would make `list` more useful but requires the full pipeline. A lean version (just page + path) could skip graph construction entirely, using only the scanner. Recommendation: lean version first (page + path only), with a `--with-counts` flag that triggers the full pipeline.

2. **Should `export` output DOT format in addition to JSON?** DOT format is useful for visualization tools (Graphviz, d3). Recommendation: JSON only in this SPEC; DOT via a `--format dot` extension in a future SPEC.

3. **Should dedup be opt-out?** If a user genuinely wants to see every edge occurrence (e.g., for link-density analysis), the current raw output is useful. Recommendation: dedup by default (REQ-021), add `--raw` flag to preserve current behaviour if needed.

4. **Should JSON errors use HTTP-style status codes?** Using codes like 404/400 would be more familiar to web developers. Recommendation: keep Unix-style exit codes (1, 2) — they map to process exit codes and are more natural for CLI tools.

---

**END OF SPEC-003**
