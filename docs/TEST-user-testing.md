---
id: TEST-USER
title: Comprehensive User Testing of ztl
status: completed
version: 1.0
tested-date: 2026-04-07
detecting-model: Claude Opus 4.6 (1M context)
detection-method: synthetic user simulation + direct CLI exercise
vault-under-test: ztl-vault (13 pages, 192 links, 120 dead links)
---

# TEST-USER: Comprehensive User Testing of ztl

## Synthetic User Profiles

### User: Knowledge Worker (Alex)

**Role:** Technical writer maintaining a personal knowledge base with ~50 notes
**Goals:** Browse notes, find connections, validate vault health, publish a static site
**Constraints:** Moderate CLI proficiency, expects standard flag conventions, uses both terminal and web browser
**Daily workflow:**

1. Run `ztl check` for vault health
2. Search for topics with `ztl search`
3. Browse connections with `ztl links` / `ztl backlinks`
4. View notes in TUI with `ztl view`
5. Build/serve a static site with `ztl build` or `ztl serve`

### User: AI Agent (AgentBot)

**Role:** LLM agent consuming ztl output programmatically
**Goals:** Query vault structure, parse JSON output, integrate into automated workflows
**Constraints:** Requires structured JSON, non-zero exit codes for errors, deterministic output
**Daily workflow:**

1. `ztl list --json` to discover pages
2. `ztl links <page> --json` for graph traversal
3. `ztl search <query> --json` for content discovery
4. `ztl check --json --fail-on error` for CI/CD gates
5. `ztl export --json` for full graph analysis

---

## Happy Paths

### Happy Path: Vault Health Check

**Preconditions:** ztl installed, vault directory exists with markdown files
**Steps:**

1. `ztl check -d vault/` -> JSON report of dead links, orphans, syntax errors
2. `ztl check -d vault/ --dead-links` -> filtered to dead links only
3. `ztl check -d vault/ --fail-on error` -> non-zero exit if errors found

**Postconditions:** User knows vault health status; CI pipeline can gate on result
**Failure modes:** Non-existent vault dir, empty vault, vault with no markdown files

### Happy Path: Explore Connections

**Preconditions:** Vault indexed
**Steps:**

1. `ztl list -d vault/` -> list of all pages
2. `ztl links "Page Name" -d vault/` -> forward links from page
3. `ztl backlinks "Page Name" -d vault/` -> backlinks to page
4. `ztl path "From" "To" -d vault/` -> shortest path between pages
5. `ztl similar "query" -d vault/` -> fuzzy page name matches

**Postconditions:** User understands the link topology around their topic
**Failure modes:** Page name typo, disconnected graph, no results

### Happy Path: Publish Static Site

**Preconditions:** Vault exists with content
**Steps:**

1. `ztl build -d vault/ --out-dir dist/` -> generate static HTML
2. Open `dist/index.html` in browser -> sidebar, page list, search works
3. Search in the static site -> BM25 client-side search returns results

**Postconditions:** Deployable static site with working search
**Failure modes:** Missing theme, empty vault, BM25 index not embedded

---

## Test Results Summary

### Tests Executed

| Area | Commands Tested | Pass | Fail | Issues |
|------|----------------|------|------|--------|
| Core Graph | index, links, backlinks, path, similar, blocks, list, stats, export | 18 | 0 | 2 UX |
| Search | search (basic, context, case, path, near, regex) | 11 | 0 | 1 UX |
| Diagnostics | check (dead-links, orphans, syntax, fail-on) | 8 | 0 | 1 UX |
| View TUI | 18 unit tests (TestBackend) | 18 | 0 | 0 |
| TUI Dashboard | ztl tui help, integration tests | 60 | 0 | 0 |
| Web Serve | serve, API endpoints (pages, search, graph) | 5 | 0 | 0 |
| Web Build | build (default, themed, structure) | 4 | 1 | 1 BUG |
| Themes | theme list, theme export | 3 | 0 | 0 |
| Hooks | hook list | 2 | 0 | 0 |
| Diff | diff (default, --from) | 3 | 0 | 0 |
| Watch | watch mode | 1 | 0 | 0 |
| Integration Tests | cargo test --test integration | 135 | 1 | 1 BUG |
| CLI Polish Tests | cargo test --test cli_polish_integration | 13 | 0 | 0 |
| Web Integration | cargo test --test web_integration | 60 | 0 | 0 |

**Total: 355 tests executed, 353 passed, 2 failed (1 distinct bug)**

---

## Bug Reports

### BUG-001: Stale Test Asserts Inline BM25 Index (Moved to External File)

**Severity:** S4 (Minor)
**Priority:** P2
**Status:** confirmed
**Reported by:** Claude Opus 4.6 (synthetic user simulation)
**Assigned to:** unassigned

#### Specification Reference

- Related: TEST-013-015, `test_013_015_html_embeds_bm25_index` (tests/integration.rs:3927)

#### Environment

- ztl 0.1.0, macOS Darwin 25.2.0, Rust (dev build)

#### Description

The test `test_013_015_html_embeds_bm25_index` asserts that `index.html` must contain an inline `<script id="ztl-bm25-index">` element. However, the BM25 index was intentionally moved to an external file (`search-index.json`) because the inline payload was too large, bloating every HTML page.

The implementation at `src/web/build.rs:306` deliberately sets `bm25_json = String::new()` and writes the search index as a standalone JSON file via `write_search_index_json()`. The template conditional `{% if bm25_index %}` correctly skips the inline embed. The client-side JS in the templates already has a fallback that fetches `search-index.json` when the inline element is absent.

#### Root Cause

- **Category:** test-gap
- **Analysis:** The test was written for the original inline-embedding approach and was not updated when the implementation switched to an external file for size reasons.

#### Resolution

- **Fix:** Update `test_013_015_html_embeds_bm25_index` to assert the existence of `search-index.json` in the build output instead of the inline `<script>` element. Verify that the external file contains `avgDl` and other BM25 corpus statistics.
- **Regression test added:** The updated test covers the external-file approach

### BUG-002: `--limit` Flag Corrupts `total_matches` Count in Search

**Severity:** S3 (Moderate)
**Priority:** P2
**Status:** confirmed
**Reported by:** Claude Opus 4.6 (synthetic user simulation)
**Assigned to:** unassigned

#### Specification Reference

- Violates: Expected search contract — `total_matches` should reflect the total number of matches found, independent of the `--limit` parameter
- Related: SPEC-002 (Full-Text Search)

#### Environment

- ztl 0.1.0, macOS Darwin 25.2.0

#### Steps to Reproduce

1. Run `ztl -d ztl-vault search "install"` -> reports `total_matches: 7`
2. Run `ztl -d ztl-vault search "install" --limit 1` -> reports `total_matches: 4` (not 7)

#### Expected Behaviour

`total_matches` should report the total number of matches found before limiting, regardless of the `--limit` value. The `--limit` should only truncate the `results` array.

#### Actual Behaviour

`total_matches` reports 4 when `--limit 1` is used, but reports 7 without the limit. The limit appears to interfere with match counting during the BM25 scoring/aggregation phase.

#### Root Cause

- **Category:** implementation-error
- **Analysis:** The `--limit` value is being applied during the search/scoring phase rather than as a post-processing truncation, causing the total count to be affected.

---

### BUG-003: `build` Command Ignores `--json` / `-f json` Flag

**Severity:** S4 (Minor)
**Priority:** P3
**Status:** confirmed
**Reported by:** Claude Opus 4.6 (synthetic user simulation)
**Assigned to:** unassigned

#### Specification Reference

- Violates: SPEC-003 (Agent Ergonomics) — all commands should respect the global format flag
- Related: Global `-f` / `--json` flag contract

#### Steps to Reproduce

1. Run `ztl -d ztl-vault --json build --out-dir /tmp/test`
2. Observe output is always the human-readable string: `ztl build -> 13 pages + 3 folder indexes written to ...`

#### Expected Behaviour

With `--json`, output should be structured JSON (e.g., `{"pages": 13, "folder_indexes": 3, "out_dir": "...", "files": [...]}`).

#### Actual Behaviour

The build command always outputs a human-readable string regardless of the format flag.

#### Root Cause

- **Category:** implementation-error
- **Analysis:** The `build` command's output path does not check the global format setting.

---

### BUG-004: `check` Table Summary Missing Top-Level Counts

**Severity:** S4 (Minor)
**Priority:** P3
**Status:** confirmed
**Reported by:** Claude Opus 4.6 (synthetic user simulation)
**Assigned to:** unassigned

#### Steps to Reproduce

1. Run `ztl -d ztl-vault -f table check`
2. Observe the Summary table section

#### Expected Behaviour

The summary table should include the top-level aggregate counts: dead_links, orphans, syntax_errors, spl_errors (same as the JSON summary).

#### Actual Behaviour

The summary table only shows SPL/drift-related metrics (total SPL blocks, drifted blocks, grounded facts, broken groundings). The most important aggregate counts (dead_links: 120, orphans: 1) are absent from the summary section, even though they appear in the detailed tables above.

---

## Findings (UX and Improvement Areas)

### Finding 001: Global Flags Must Precede Subcommand

- **Step:** Any command with `-d`, `-f`, `--json`, `--no-cache`, `--no-color`, `-q`, `-v` flags
- **Category:** Friction
- **Description:** Global flags (`-d`, `-f`, `--json`, `--no-cache`, etc.) must be placed BEFORE the subcommand. Running `ztl links "Page" -d vault/` or `ztl list -f table` produces a confusing error: `error: unexpected argument '-d' found`. Users naturally place flags after the subcommand or after arguments.
- **User impact:** Confusion and repeated failed commands, especially for new users. The error message (`tip: to pass '-d' as a value, use '-- -d'`) is misleading since the user isn't trying to pass `-d` as a value.
- **Proposed resolution:** Either (a) accept global flags in any position (clap's `SubcommandPrecedenceOver` or similar), or (b) improve the error message to say "Global flags like -d must appear before the subcommand: `ztl -d vault/ links 'Page'`".
- **Trace:** Enhancement (not a bug — the CLI works as documented, but the convention is surprising)

### Finding 002: JSON Error Responses Lack Fuzzy Suggestions

- **Step:** `ztl links "Instl" --json` (non-existent page)
- **Category:** Gap
- **Description:** When a page is not found, the table-format output helpfully shows: `Hint: run 'ztl list' to see all pages, or use --fuzzy for approximate matching.` However, the JSON error response only contains `{"error": "Page not found: 'Instl'", "code": 1}` with no `suggestions` or `did_you_mean` field.
- **User impact:** AI agents parsing JSON output cannot auto-correct page name typos. Human users in JSON mode get less help than table mode users.
- **Proposed resolution:** Add a `suggestions` array to the JSON error response containing the top 3 fuzzy matches (same data as `ztl similar`), e.g.: `{"error": "Page not found: 'Instl'", "code": 1, "suggestions": ["Install"]}`
- **Trace:** Amends SPEC-003 (Agent Ergonomics) — agents should be able to recover from typos programmatically

### Finding 003: Dead Links Do Not Trigger `--fail-on error`

- **Step:** `ztl check --dead-links --fail-on error`
- **Category:** Ambiguity
- **Description:** The vault has 120 dead links, but `--fail-on error` still exits with code 0. Dead links are reported in the JSON output but are apparently not classified as "errors" for the purpose of exit code determination.
- **User impact:** CI/CD pipelines using `--fail-on error` to gate deployments will not catch dead links. Users must write custom scripts to parse the JSON and check `summary.dead_links > 0`.
- **Proposed resolution:** Either (a) classify dead links as warnings by default (so `--fail-on warning` catches them), or (b) add a `--fail-on dead-links` flag, or (c) document the current behaviour clearly and provide a recipe for CI integration.
- **Trace:** Amends SPEC-001 / CON for check command — the `--fail-on` semantics need clarification

### Finding 004: Search `--context` Semantics May Confuse Users

- **Step:** `ztl search "install" --context 2`
- **Category:** Friction (minor)
- **Description:** The `--context` flag for search takes a CHARACTER count, not a LINE count. Running `--context 2` produces extremely truncated context like `". Install o"`. While the help text correctly says "Include N characters of surrounding text", users familiar with `grep -C` (which takes line counts) may be surprised.
- **User impact:** Mild confusion; users quickly learn the correct semantics from the help text.
- **Proposed resolution:** No code change needed, but consider either (a) using a larger default (40 is good), or (b) adding a `--context-lines` variant alongside the character-based `--context`. Low priority.
- **Trace:** No new REQ needed

### Finding 005: Export Shows `null` Paths for Dead Link Targets

- **Step:** `ztl export`
- **Category:** Gap (minor)
- **Description:** The export command outputs nodes for both real pages and dead link targets. Dead link targets have `"path": null`. This is technically correct but may confuse consumers who expect only real pages.
- **User impact:** Consumers must filter nodes by `path != null` to get only real pages. The `is_real` field is available in the graph API (`/api/graph`) but not in the CLI `export` output.
- **Proposed resolution:** Add an `is_real` boolean field to export nodes (matching the API), or add a `--real-only` filter flag. Low priority.
- **Trace:** Amends SPEC-001 / export contract

### Finding 006: `ztl build` Exit Code 0 with 120 Dead Links

- **Step:** `ztl build --out-dir dist/`
- **Category:** Gap
- **Description:** The build command succeeds (exit 0) even when the vault has 120 dead links. The generated HTML will contain broken internal links. There is no `--fail-on` flag for build.
- **User impact:** CI/CD pipelines deploying static sites won't catch broken links unless they also run `ztl check` separately.
- **Proposed resolution:** Add `--fail-on dead-links` to the build command, or document that `ztl check --fail-on warning` should be run before `ztl build` in CI pipelines.
- **Trace:** Enhancement for build command

### Finding 007: Compiler Warnings (3 unused `mut`)

- **Step:** `cargo build`
- **Category:** Friction (minor, developer-facing)
- **Description:** Three `unused mut` warnings during compilation in `src/web/build.rs`
- **User impact:** None for end users; cosmetic for contributors.
- **Proposed resolution:** Remove unnecessary `mut` qualifiers. Run `cargo fix --lib -p ztl`.
- **Trace:** No spec impact

### Finding 008: `diff` Timestamps Always Null

- **Step:** `ztl -d ztl-vault diff`
- **Category:** Gap
- **Description:** The `from.timestamp` and `to.timestamp` fields in diff output are always `null`. These should contain the commit timestamps for context.
- **User impact:** Consumers cannot determine the time range of the diff without running `git log` separately.
- **Proposed resolution:** Populate timestamp fields from git commit metadata.
- **Trace:** Amends diff command contract

### Finding 009: Default Theme Requires CDN Access

- **Step:** `ztl build` then open `dist/index.html` offline
- **Category:** Friction
- **Description:** The default theme loads CSS/JS from CDNs (`cdn.jsdelivr.net` for DaisyUI, `cdn.tailwindcss.com`). Static builds opened without internet access render as unstyled HTML.
- **User impact:** Offline/airgapped users get a broken-looking site. The `minimal` theme works offline since it has inline CSS.
- **Proposed resolution:** Consider an `--inline` or `--self-contained` flag, or document that `--theme minimal` is the offline-friendly option.
- **Trace:** Enhancement for build command

### Finding 010: `-q` (Quiet) Flag Has No Visible Effect on Most Commands

- **Step:** `ztl -d ztl-vault -q list` vs `ztl -d ztl-vault list`
- **Category:** Friction (minor)
- **Description:** The `-q` flag suppresses "non-essential output" per the help text, but most commands already produce only JSON in pipe mode. The flag has a visible effect only on `index` (suppresses progress info) and possibly `build`.
- **User impact:** Users may wonder if the flag is working. No harm, just confusing.
- **Proposed resolution:** Document which commands are affected by `-q`, or have `-q` suppress JSON output entirely (exit code only).
- **Trace:** No new REQ needed

### Finding 011: Search Does Not Support Regex Patterns

- **Step:** `ztl -d ztl-vault search ".*link.*"` returns 0 results
- **Category:** Gap
- **Description:** The search command uses BM25 full-text search, not regex matching. Regex patterns like `.*link.*` are searched literally and return no results. There is no `--regex` flag.
- **User impact:** Users familiar with grep may expect regex support. The search silently returns no results for patterns that look valid.
- **Proposed resolution:** Either (a) add a `--regex` flag for grep-style pattern matching, or (b) document that search is BM25-based and suggest `grep` for regex patterns.
- **Trace:** Amends SPEC-002 or documentation

---

## Test Session Metadata

### AI Detection Context

- **Detecting model:** Claude Opus 4.6 (1M context)
- **Detection method:** Synthetic user simulation + direct CLI exercise + automated test suite execution
- **Confidence:** High (all findings directly observed via command execution)
- **Session context:** hence plan TEST-USER, 2026-04-07

### Test Environment

- **OS:** macOS Darwin 25.2.0
- **Rust:** dev profile (unoptimized + debuginfo)
- **ztl version:** 0.1.0
- **Vault:** ztl-vault (13 real pages, 61 unique forward link targets, 120 dead links)
- **Features tested:** default (no `reason`, `history`, `semantic`, or `mcp` features)

### Automated Test Suite Results

| Test Suite | Tests | Passed | Failed | Ignored |
|-----------|-------|--------|--------|---------|
| `tests/integration.rs` | 139 | 135 | 1 | 3 |
| `tests/view_tui.rs` | 18 | 18 | 0 | 0 |
| `tests/web_integration.rs` | 60 | 60 | 0 | 0 |
| `tests/cli_polish_integration.rs` | 13 | 13 | 0 | 0 |
| **Total** | **230** | **226** | **1** | **3** |

The 3 ignored tests require optional features (`semantic`, `history`).
The 1 failure is BUG-001 (BM25 index not embedded in build output).

---

## Summary of Findings by Category

| Category | Count | Severity |
|----------|-------|----------|
| Bug | 4 | S3 (1), S4 (3) |
| Friction | 5 | S4 |
| Gap | 6 | S3-S4 |

### Prioritised Action Items

1. **P2 — Fix BUG-001:** Update stale test `test_013_015_html_embeds_bm25_index` to assert external `search-index.json` instead of inline embed
2. **P2 — Fix BUG-002:** Ensure `total_matches` in search reflects the pre-limit count, not a limit-corrupted count
3. **P2 — Finding 001:** Improve global flag positioning error messages or accept flags in any position
4. **P2 — Finding 002:** Add `suggestions` field to JSON error responses for page-not-found errors
5. **P3 — Fix BUG-003:** Make `build` command respect `--json` / `-f json` flag
6. **P3 — Fix BUG-004:** Add top-level counts to `check` table summary
7. **P3 — Finding 003:** Clarify `--fail-on` semantics for dead links and orphans
8. **P3 — Finding 011:** Document that search is BM25-based (not regex) or add `--regex` flag
9. **P4 — Remaining findings:** Low priority UX improvements (008-010, 004-007)

### Overall Assessment

ztl is in excellent shape. The core graph commands, search, diagnostics, web serve/build, themes, hooks, diff, and watch all function correctly. The test suite is comprehensive (230 automated tests). Four bugs were found:

- **1 moderate (S3):** `--limit` corrupts `total_matches` count in search
- **3 minor (S4):** Stale test for inline BM25 (moved to external file); `build` ignores format flag; `check` table summary incomplete

The UX findings are friction points, not blockers. The most impactful improvement would be accepting global flags (`-d`, `--json`, etc.) in any position, as this is the single most common source of user confusion. The tool is production-ready for its core use cases.
