---
title: "SPEC-017: zetl history — Invisible Temporal Graph Navigation via jj-lib"
version: 0.2.0
status: draft
audience: agent, human
date: 2026-03-04
---

# SPEC-017: zetl history — Invisible Temporal Graph Navigation via jj-lib

## Information Table

| Field        | Value                                                              |
| ------------ | ------------------------------------------------------------------ |
| Document ID  | SPEC-017                                                           |
| Title        | zetl history — Invisible Temporal Graph Navigation via jj-lib      |
| Version      | 0.2.0                                                              |
| Status       | Draft                                                              |
| Author       | Agent (USDD Protocol v1.3.0)                                       |
| Date         | 2026-03-04                                                         |
| Audience     | Agent, Human                                                       |
| Trace        | USDD Agent Protocol v1.3.0                                         |
| Parent       | SPEC-001: zetl — Bi-directional Link Graph CLI                     |
| Related      | SPEC-006: Content-Addressed Merkle Tree; SPEC-007: zetl diff; SPEC-008: zetl watch |
| Supersedes   | SPEC-007 (VCS backend only; diff semantics preserved)              |
| Dependencies | jj-lib (Jujutsu VCS library), blake3, SPEC-006 (index cache format) |

---

## 1. Overview

Zetl commands operate on the present. `zetl links Foo` tells you what Foo links to *right now*. `zetl check` finds dead links *right now*. The vault is always a single, frozen moment. To understand how the graph evolved — when a page became an orphan, how a cluster of notes grew over a week, what reasoning conclusions changed after a restructuring — the user must reconstruct the past manually.

SPEC-007 partially addressed this with `zetl diff`, computing graph-level deltas against git history. But it has three structural limitations:

1. **Granularity is limited to explicit git commits.** Edits between commits are invisible. Most knowledge workers commit infrequently if at all — their vault history has gaps measured in hours or days.
2. **Git is an external dependency.** SPEC-007 requires the `git` binary on PATH and shells out for every operation (NFR-019). This is fragile, slow, and architecturally inconsistent with zetl's otherwise self-contained design.
3. **Only diffs are supported.** You can see what changed between two points, but you cannot *query the graph at a past point*. There is no `zetl links Foo --at yesterday`.

This specification solves all three by embedding [Jujutsu](https://github.com/jj-vcs/jj) (`jj-lib`) as zetl's temporal engine. Jujutsu is a Rust-native VCS that snapshots the working copy automatically — every save, every edit, every moment the vault changes produces a navigable point in history, without the user running any command.

### 1.1 Design Principle: Invisibility

The user learns nothing new. There are no `jj` commands to run, no repository to initialise, no snapshots to create, no revset syntax to learn. Temporal capabilities appear as natural extensions of commands they already use:

- `zetl diff` works exactly as SPEC-007 describes, but with finer-grained history and without requiring git
- `zetl links Foo --at yesterday` queries the graph as it existed yesterday
- `zetl history` shows how the graph evolved
- The TUI dashboard gains a timeline panel

Under the hood, jj-lib manages everything: initialisation, snapshotting, storage, history traversal. The user sees results. The plumbing is invisible.

### 1.2 Why jj-lib, Not Git

| Concern | git (SPEC-007) | jj-lib (this spec) |
|---|---|---|
| Snapshot granularity | Explicit commits only | Every working-copy state |
| Dependency | External binary on PATH | Rust library, compiled in |
| API | Subprocess + stdout parsing | Type-safe Rust API |
| Conflicts | Block merging; abort on failure | First-class conflict objects |
| Undo | `git reflog` (expert-only) | Operation log with full undo |
| Git compatibility | Native | Full (git storage backend) |
| User workflow change | Must `git commit` | None |

Critically, jj uses git as its storage backend. A vault that is already a git repository gains jj's capabilities without migration. Existing git commits, branches, and remotes are preserved and visible to jj.

### 1.3 Scope

**In scope:**

- Embedding jj-lib as an optional Cargo feature (`history`)
- Automatic, silent VCS initialisation within `.zetl/jj/`
- Automatic snapshotting on every `zetl index`
- `--at <time-expr>` flag on existing read-only commands (`links`, `backlinks`, `check`, `graph`, `search`, `blocks`, `reason`)
- `zetl history` command with graph-level timeline
- `zetl history page <name>` for per-page evolution
- Superseding SPEC-007's git-subprocess VCS backend with jj-lib
- Cached historical indexes keyed by `vault_root_hash` (SPEC-006)
- Watch-mode integration (SPEC-008) for continuous snapshotting
- Template context extensions: `vault.history` and `page.history` for themes
- Serve-mode API endpoints for temporal queries (`/api/history/*`)
- Build-mode static history export (`history-index.json`) for client-side use
- Graceful absence: themes render identically when history is unavailable

**Out of scope:**

- Exposing jj commands, revset syntax, or jj concepts to the user
- Write operations through jj (zetl never modifies vault files)
- Multi-vault sync or federation (SPEC-011)
- Merge conflict resolution UI (conflicts in jj are informational, not interactive)
- Distributed/remote jj operations (push, pull, fetch)

### 1.4 Relationship to SPEC-007

This specification **supersedes** SPEC-007's VCS backend implementation (git subprocess calls, ADR-011 no-snapshot storage, NFR-019 git subprocess isolation). The diff *semantics* defined in SPEC-007 — the output schema (CON-021), diff categories, filter flag — are preserved unchanged. What changes is the engine: jj-lib replaces git subprocesses, and automatic snapshotting replaces the user's explicit git commits as the primary history source.

SPEC-007's requirements (REQ-046 through REQ-052) remain valid and are extended, not replaced. Where this specification conflicts with SPEC-007's implementation decisions (ADR-011, NFR-019), this specification takes precedence.

---

## 2. User Profiles

### 2.1 User Profile: Kai — Casual Note-Taker

```
Name:        Kai
Role:        Graduate student; maintains a 400-note research vault in Obsidian
Goals:       Understand how their vault has grown; find when they stopped linking
             to a topic; review what changed during a writing session
Constraints: Does not use git; does not know what version control is;
             uses zetl for link-checking and search only
Workflow:    Opens Obsidian; writes notes throughout the day; runs zetl check
             occasionally to find broken links; never commits anything
Pain point:  "I reorganised my notes last week and now half my links are broken.
             I wish I could see what the vault looked like before I moved things."
```

### 2.2 User Profile: Akiko — Knowledge Worker (from SPEC-007)

```
Name:        Akiko
Role:        Product manager; maintains a 2,000-note Zettelkasten for research synthesis
Goals:       Review how her vault evolved during a work session; spot structural
             regressions; compare graph topology across time
Constraints: Comfortable with CLI; uses git; expects existing zetl diff to keep working
Workflow:    Writes notes during research; commits periodically; runs zetl diff
             to review session changes; runs zetl check for quality
Pain point:  "I want to see the vault as it was at any point — not just diffs,
             but actually query the past graph."
```

### 2.3 User Profile: Mika — Theme Developer

```
Name:        Mika
Role:        Front-end developer; builds custom zetl themes for clients
Goals:       Use temporal data in themes to show vault evolution; build
             dashboards with sparklines, activity feeds, and page age badges;
             create timeline-driven visualisations for the web UI
Constraints: Works with Jinja2 templates and JavaScript; does not touch Rust;
             themes must work in both serve and build modes; themes must
             degrade gracefully when history is unavailable
Workflow:    Edits templates in .zetl/themes/<name>/; tests with zetl serve;
             builds static sites with zetl build; ships themes as directories
Pain point:  "I can show what the vault looks like now, but I can't show how
             it got here. I want sparklines, activity feeds, page age — all the
             temporal metadata that makes a dashboard feel alive."
```

### 2.4 User Profile: Dev Agent — Agentic Memory Consumer (from SPEC-007)

```
Name:        Dev Agent (AI agent in CI or local automation)
Role:        Automated reasoning cycle; queries zetl on schedule or on file change
Goals:       Determine exactly which pages and links changed since last run;
             reconstruct graph at any prior point for comparison
Constraints: Non-interactive; structured JSON output; must not require manual
             intervention or external tool installation
Workflow:    1. zetl diff --since <last-run> --format json
             2. For interesting changes: zetl links <page> --at <timestamp> --format json
             3. Re-reason over affected subgraph
Pain point:  "I need point-in-time graph queries, not just diffs."
```

---

## 3. Happy Paths

### 3.1 Happy Path: First-Time History (Kai)

```
Preconditions:
  - Kai has a 400-note vault with no git repository
  - Kai has used zetl before (index.json exists)
  - Kai has never heard of jj or version control

Steps:
  1. Kai runs zetl index (as usual)
     → Index rebuilds as normal
     → Internally: zetl detects no .zetl/jj/ directory, silently initialises
       a jj repository with the vault as working copy, creates first snapshot
     → No output about jj, no new files visible to Kai

  2. Kai edits notes throughout the day, running zetl index periodically
     → Each zetl index silently creates a new jj snapshot
     → Kai sees normal index output only

  3. Next day, Kai realises they broke links during yesterday's reorganisation
     zetl check --at yesterday
     → Shows: 0 dead links yesterday (before the reorganisation)
     → Kai now knows the reorganisation caused the breakage

  4. Kai runs zetl diff --since yesterday
     → Shows: 12 links removed, 3 orphans gained, 2 dead links added
     → Kai fixes the issues

Postconditions:
  - Kai used temporal features without learning version control
  - No git repository exists; jj history lives inside .zetl/jj/
  - Kai's workflow is unchanged: edit notes, run zetl commands

Failure modes:
  - history feature not compiled in: --at flag and zetl history are absent;
    zetl diff falls back to git subprocess mode (SPEC-007) or errors gracefully
  - .zetl/jj/ corrupted or deleted: zetl silently re-initialises on next index;
    history before the corruption is lost; current operations unaffected
```

### 3.2 Happy Path: Point-in-Time Query (Akiko)

```
Preconditions:
  - Vault is a git repository with commits
  - zetl has been run multiple times (jj history exists in .zetl/jj/)
  - Akiko refactored "Architecture" cluster last week

Steps:
  1. zetl links "Architecture Overview" --at "last monday"
     → Shows: 14 forward links (the state before refactoring)

  2. zetl links "Architecture Overview"
     → Shows: 8 forward links (current state)

  3. zetl history page "Architecture Overview"
     → Shows timeline:
       2026-02-24 14:32  links: 14  backlinks: 7
       2026-02-25 09:15  links: 12  backlinks: 7  (-2 links)
       2026-02-25 09:41  links: 8   backlinks: 5  (-4 links, -2 backlinks)
       2026-02-28 16:00  links: 8   backlinks: 5  (no change)
       2026-03-03 11:22  links: 8   backlinks: 6  (+1 backlink)

  4. zetl diff --from "last monday" --format table
     → Shows the full graph diff since Monday, including all link changes

Postconditions:
  - Akiko queried the graph at a past point without checking out old files
  - Git history and jj snapshots are both available as history sources
  - Akiko's git workflow is unaffected

Failure modes:
  - Time expression before any snapshot: error with SNAPSHOT_NOT_FOUND
  - Ambiguous time expression: resolves to most recent snapshot at or before the time
```

### 3.3 Happy Path: Agentic Delta Processing (Dev Agent)

```
Preconditions:
  - Agent last processed the vault at timestamp "2026-03-03T08:00:00Z"
  - New edits have been made since

Steps:
  1. zetl diff --since "2026-03-03T08:00:00Z" --format json
     → Returns structured diff with pages/links/orphans/dead-links changes
     → Internally uses jj-lib (no git subprocess)

  2. Agent identifies 3 pages with link changes
     For each: zetl links <page> --at "2026-03-03T08:00:00Z" --format json
     → Returns the old link set for comparison

  3. Agent reasons over the delta and stores current timestamp

Postconditions:
  - Agent processed only changed subgraph
  - No external tool dependencies (no git binary required)
  - Structured JSON throughout

Failure modes:
  - Timestamp between snapshots: resolves to nearest snapshot at or before
  - No snapshots exist: error with NO_HISTORY
```

### 3.4 Happy Path: Graph Timeline in TUI

```
Preconditions:
  - Vault with jj history spanning multiple days
  - User launches zetl TUI dashboard

Steps:
  1. User opens TUI dashboard (zetl view or zetl serve TUI mode)
     → Dashboard shows current graph stats as usual
     → A timeline indicator appears in the status bar showing history range:
       "[2026-02-20 ← → 2026-03-04]  Now"

  2. User presses [ (left bracket) to step backward in time
     → Graph view updates to show the previous snapshot's state
     → Status bar shows: "[2026-02-20 ← → 2026-03-04]  2026-03-03 16:45"
     → Link counts, orphan count, dead link count update

  3. User presses ] (right bracket) to step forward
     → Returns toward present

  4. User presses Shift+[ to jump backward by day
     → Jumps to the last snapshot of the previous day

  5. User presses Escape or 'n' to return to "Now" (live state)
     → Dashboard returns to current vault state

Postconditions:
  - User scrubbed through graph history without leaving the TUI
  - No files were modified; all reads were from jj history

Failure modes:
  - No history: timeline indicator hidden; [ and ] keys are no-ops
  - At oldest snapshot: [ is a no-op; visual indicator shows "oldest"
  - At newest snapshot: ] returns to live state
```

### 3.5 Happy Path: Theme with Temporal Data (Mika)

```
Preconditions:
  - Vault with history spanning 2 weeks (multiple zetl index runs)
  - Mika is building a custom theme with a dashboard

Steps:
  1. Mika edits index.html to add a vault growth sparkline:

     {% if vault.history %}
     <div class="sparkline">
       {% for point in vault.history.trend %}
         <span data-pages="{{ point.pages }}" data-t="{{ point.timestamp }}"></span>
       {% endfor %}
     </div>
     {% endif %}

     → In serve mode: zetl populates vault.history.trend with the last 30 data points
     → In build mode: same data is pre-computed and baked into the HTML
     → When history feature is absent: the {% if %} block is skipped cleanly

  2. Mika edits page.html to show page age and link evolution:

     {% if page.history %}
     <span class="badge">Created {{ page.history.age_days }} days ago</span>
     <span class="badge">{{ page.history.link_trend|length }} snapshots</span>
     {% endif %}

     → Each page render includes its temporal metadata
     → Works identically in serve and build modes

  3. Mika adds a JavaScript timeline slider to the theme (serve mode only):

     {% if mode == "serve" and vault.history %}
     <input type="range" id="timeline"
            min="{{ vault.history.oldest_epoch }}"
            max="{{ vault.history.newest_epoch }}">
     <script>
       slider.addEventListener('input', async (e) => {
         const t = new Date(Number(e.target.value) * 1000).toISOString();
         const state = await fetch(`/api/history/at?t=${t}`).then(r => r.json());
         updateDashboard(state);
       });
     </script>
     {% endif %}

     → Theme JS calls the history API endpoint
     → Dashboard re-renders with historical vault context

  4. For build mode, Mika uses the pre-baked history index:

     {% if mode == "build" and vault.history %}
     <script>
       const historyData = {{ history_index | safe }};
       // Client-side timeline from pre-computed data
     </script>
     {% endif %}

     → zetl build writes history-index.json alongside search-index.json
     → Theme JS loads and navigates pre-computed snapshots client-side

  5. Mika runs zetl build --theme custom -o dist/
     → Static site includes all temporal data pre-rendered in HTML
     → history-index.json written to dist/ for JS-driven features
     → Site works from file:// with no server

Postconditions:
  - Theme uses temporal data in both serve and build modes
  - Theme degrades gracefully when history is unavailable
  - No Rust changes required; all data exposed via template context

Failure modes:
  - history feature not compiled in: vault.history is null; {% if %} guards skip blocks
  - No snapshots yet: vault.history is null (same behavior as feature absent)
  - /api/history/* endpoint called without history: returns JSON error
```

### 3.6 Happy Path: Static Site with History (Akiko)

```
Preconditions:
  - Akiko's 2,000-note vault has 2 weeks of history
  - Akiko uses the default theme
  - Akiko wants to publish her vault as a static site

Steps:
  1. zetl build -o site/
     → Static site generated as usual
     → Index page includes vault stats trend (sparkline data in template context)
     → Each page includes creation date and link evolution metadata
     → history-index.json written to site/ with timeline data

  2. Akiko opens site/index.html in a browser
     → Sees vault stats with trend indicators (↑ 12 pages this week)
     → Each page card shows "Created 11 days ago"

  3. Akiko clicks a page → sees backlink section with "Linked since Mar 1"
     timestamps on each backlink

Postconditions:
  - Static site contains temporal metadata with no server required
  - Default theme uses history data when available
  - Site renders correctly even if history-index.json is removed

Failure modes:
  - No history: pages render without temporal badges; stats show current only
  - Large vault with many snapshots: history-index.json limited to last 30
    data points per trend to keep file size bounded
```

---

## 4. Functional Requirements

### REQ-075: Invisible VCS Initialisation

The system SHALL automatically initialise a jj repository on the first invocation of `zetl index` when the `history` feature is enabled and no `.zetl/jj/` directory exists. Initialisation SHALL:

- Create the jj repository store inside `.zetl/jj/` (not at the vault root)
- Configure the vault root as the jj working copy
- If a `.git/` directory exists at or above the vault root, configure jj to use the git backend in colocated mode, preserving all existing git history
- If no `.git/` directory exists, configure jj with a standalone git backend (jj uses git object storage internally even without a user-facing git repo)
- Produce no user-visible output about the initialisation
- Add no files to the vault directory (all jj state lives inside `.zetl/jj/`)

The initialisation SHALL be idempotent: if `.zetl/jj/` already exists, it is reused without modification.

Trace:
- TEST-080
- ADR-044, ADR-045

### REQ-076: Automatic Snapshotting on Index

Every successful completion of `zetl index` SHALL create a jj snapshot (working-copy commit) capturing the current state of all vault files. The snapshot SHALL:

- Be created silently with no user-visible output (unless `--verbose` is set, in which case a single line `[zetl] snapshot: <change-id-prefix>` is emitted)
- Include the `vault_root_hash` (SPEC-006) in the jj commit description for traceability
- Include the current timestamp
- Be deduplicated: if the vault content is identical to the previous snapshot (same `vault_root_hash`), no new snapshot is created

Trace:
- TEST-081
- ADR-048

### REQ-077: Time Expression Syntax for `--at`

The system SHALL support `--at <time-expr>` on all read-only subcommands (`links`, `backlinks`, `check`, `graph`, `search`, `blocks`, `reason`). The `<time-expr>` SHALL accept:

- ISO 8601 datetime: `2026-03-01T14:30:00Z`, `2026-03-01`
- Relative natural-language expressions: `yesterday`, `"last monday"`, `"3 days ago"`, `"2 hours ago"`, `"last week"`
- Relative commit notation (if git history exists): `HEAD~3`, `main`

The time expression is resolved to the **most recent snapshot whose timestamp is at or before the resolved time**. If no such snapshot exists, the system SHALL error with code `SNAPSHOT_NOT_FOUND` and include the timestamp of the earliest available snapshot.

Trace:
- TEST-082, TEST-083
- CON-024

### REQ-078: Point-in-Time Graph Reconstruction

When `--at <time-expr>` is provided, the system SHALL:

1. Resolve the time expression to a jj change (snapshot)
2. Read the vault file tree at that change via jj-lib's `MergedTree` API
3. Compute the `vault_root_hash` of that tree
4. If a cached index exists at `.zetl/history/<vault_root_hash>.json`, load it
5. If no cached index exists, scan the historical file tree through zetl's normal parse pipeline (scanner, wikilink extraction, Merkle tree construction) and cache the result
6. Execute the requested subcommand against the historical index

The subcommand output SHALL be identical in format to the present-time output. The only indication that a historical state is being queried SHALL be a `snapshot` field in JSON output containing the snapshot timestamp and change ID prefix.

Trace:
- TEST-084, TEST-085
- CON-024
- ADR-047

### REQ-079: Historical Index Cache

The system SHALL cache historical index snapshots in `.zetl/history/<vault_root_hash>.json`, keyed by the BLAKE3 vault root hash (SPEC-006 §4.6). Cache semantics:

- Same `vault_root_hash` implies identical graph topology — a cached index is always valid for any snapshot that produced that hash
- Cache entries are never *invalidated* (a cached entry is correct for all time), but they may be *evicted* to reclaim space. Eviction does not affect correctness — the entry can be recomputed on next access.
- Cache entries SHALL use the same serialisation format as `.zetl/index.json` (SPEC-006)
- The system SHALL implement a bounded-LRU eviction policy with a configurable maximum cache size (default: 100 entries). When exceeded, the entry with the oldest last-access time is evicted. Evicted entries are silently re-scanned and re-cached on next query.

Trace:
- TEST-086
- NFR-027
- ADR-047

### REQ-080: `zetl history` — Graph Timeline

The system SHALL support `zetl history [--since <time-expr>] [--limit N] [--format json|table]` displaying a reverse-chronological timeline of graph-level changes (newest first). Each entry SHALL contain:

- Snapshot timestamp
- jj change ID prefix (8 characters, for internal traceability)
- Graph summary: total pages, total links, orphan count, dead link count
- Delta from previous entry: pages added/removed, links added/removed, orphans gained/resolved, dead links added/resolved

Default: last 20 snapshots. `--since` filters to snapshots after the given time. `--limit N` controls the number of entries.

When no changes occurred between adjacent snapshots (identical `vault_root_hash`), the snapshots SHALL be collapsed into a single entry showing the time range.

Trace:
- TEST-087, TEST-088
- CON-025

### REQ-081: `zetl history page <name>` — Page Evolution

The system SHALL support `zetl history page <name> [--since <time-expr>] [--limit N] [--format json|table]` displaying the evolution of a single page's graph neighborhood over time. Each entry SHALL contain:

- Snapshot timestamp
- Forward link count and backlink count at that point
- Delta from previous entry: links added/removed, backlinks added/removed
- Orphan status (true/false) at that point

Only snapshots where the page's neighborhood actually changed are shown (snapshots with identical link sets are collapsed).

Trace:
- TEST-089
- CON-025

### REQ-082: Watch-Mode Snapshot Integration

When `zetl watch` (SPEC-008) detects a file change and performs an incremental re-index, it SHALL create a jj snapshot after each re-index cycle, subject to the same deduplication rule as REQ-076 (no snapshot if `vault_root_hash` is unchanged). This enables continuous, fine-grained history capture during active editing sessions.

The snapshot creation SHALL NOT block the event stream: it runs asynchronously after the re-index completes.

Trace:
- TEST-090
- ADR-048

### REQ-083: Backward-Compatible `zetl diff`

`zetl diff` SHALL continue to accept all arguments defined in SPEC-007 (CON-021): `--from <ref>`, `--since <datetime>`, `--filter`, `--format`. The output schema SHALL be identical to SPEC-007's CON-021.

The implementation SHALL use jj-lib instead of git subprocesses. When `--from` is given a git ref (branch name, tag, SHA), jj-lib SHALL resolve it via the git backend. When `--since` is given a datetime, jj-lib SHALL find the nearest snapshot at or before that time.

**Default baseline (no arguments).** SPEC-007's CON-021 defines the no-arg default as `HEAD~1`, which requires git. When the `history` feature is enabled, the default baseline SHALL be the **previous snapshot** — the most recent jj snapshot whose `vault_root_hash` differs from the current one. This generalises `HEAD~1` to non-git vaults: in a git-tracked vault the previous snapshot typically corresponds to `HEAD~1` or a state between commits; in a non-git vault it is the last distinct graph state captured by `zetl index`. If no previous snapshot exists (only one snapshot in history), the system SHALL error with code `NO_PREVIOUS_SNAPSHOT` and a message: `"Only one snapshot exists. Run zetl index after making changes to create a diff baseline."`.

The `from` field in diff output SHALL preserve CON-021's existing fields (`ref`, `commit`, `timestamp`) and MAY add optional extension fields. Existing consumers that parse only CON-021 fields SHALL NOT break.

```json
{
  "from": {
    "ref": "@-",
    "commit": null,
    "timestamp": "2026-03-03T16:45:00Z",
    "vault_root_hash": "b7e2f4a0..."
  }
}
```

Field semantics:
- `ref`: the original ref string when `--from` was used; `"@-"` when the default (previous snapshot) baseline was used
- `commit`: the git commit SHA if the baseline resolves to a git commit; `null` when the baseline is a jj-only snapshot with no corresponding git commit (non-git vaults, or edits between git commits)
- `timestamp`: ISO 8601 timestamp of the baseline (always present; preserves CON-021 contract)
- `vault_root_hash`: BLAKE3 vault root hash at the baseline (new optional field; absent in SPEC-007 fallback mode)

When the `history` feature is not compiled in, `zetl diff` SHALL fall back to the SPEC-007 git-subprocess implementation with its `HEAD~1` default.

Trace:
- TEST-091, TEST-092, TEST-112, TEST-113
- CON-021 (output schema preserved; default baseline extended)

### REQ-084: Graceful Degradation

When the `history` feature is compiled in but the jj repository is unavailable (`.zetl/jj/` missing or corrupt), all non-temporal commands SHALL operate normally. Temporal operations (`--at`, `zetl history`) SHALL error with code `NO_HISTORY` and a message explaining that history will become available after the next `zetl index`.

When the `history` feature is not compiled in, the `--at` flag and `zetl history` subcommand SHALL be absent from the CLI (not present in `--help`). `zetl diff` SHALL fall back to SPEC-007 behavior.

Trace:
- TEST-093

### REQ-085: Vault History Template Context

When the `history` feature is enabled and history is available, the template rendering pipeline (SPEC-012) SHALL inject a `vault.history` object into the template context for all templates (`index.html`, `page.html`, `folder.html`, `base.html`). The `vault.history` object SHALL contain:

```
vault.history.oldest            # ISO 8601 timestamp of earliest snapshot
vault.history.newest            # ISO 8601 timestamp of latest snapshot
vault.history.oldest_epoch      # Unix epoch seconds (for JS range inputs)
vault.history.newest_epoch      # Unix epoch seconds
vault.history.snapshot_count    # Total number of snapshots
vault.history.unique_states     # Distinct vault_root_hash count

vault.history.trend[]           # Array of up to 30 data points, evenly sampled:
  .timestamp                    # ISO 8601
  .pages                        # Total page count at this point
  .links                        # Total link count
  .orphans                      # Orphan count
  .dead_links                   # Dead link count

vault.history.recent_changes[]  # Last 10 graph deltas:
  .timestamp                    # ISO 8601
  .pages_added[]                # Page names added
  .pages_removed[]              # Page names removed
  .links_added[]                # {from, to} objects
  .links_removed[]              # {from, to} objects
  .orphans_gained[]             # Page names
  .orphans_resolved[]           # Page names
```

When history is unavailable (feature absent, no snapshots, jj repo missing), `vault.history` SHALL be `null`. Templates MUST use `{% if vault.history %}` guards; the system SHALL NOT error when history is absent.

The data SHALL be identical in serve and build modes. In build mode, the `vault.history` context is computed once during the build pipeline and shared across all page renders.

Trace:
- TEST-099, TEST-100
- CON-026
- ADR-049

### REQ-086: Page History Template Context

When the `history` feature is enabled and history is available, the template rendering pipeline SHALL inject a `page.history` object into the `PageContext` for each page render. The `page.history` object SHALL contain:

```
page.history.created_at         # ISO 8601 — first snapshot containing this page
page.history.last_changed       # ISO 8601 — last snapshot where link neighborhood changed
page.history.age_days           # Integer — days since created_at
page.history.stable_days        # Integer — days since last_changed

page.history.link_trend[]       # Array of data points (only snapshots where this page changed):
  .timestamp                    # ISO 8601
  .links                        # Forward link count
  .backlinks                    # Backlink count
  .is_orphan                    # Boolean

page.history.recent_changes[]   # Last 5 link changes for this page:
  .timestamp                    # ISO 8601
  .links_added[]                # Page names
  .links_removed[]              # Page names
  .backlinks_added[]            # Page names
  .backlinks_removed[]          # Page names
```

When history is unavailable or the page has no history (new page created after the latest snapshot), `page.history` SHALL be `null`.

In build mode, page history is computed per page during the static site generation pipeline, using the same cached historical indexes as the CLI.

Trace:
- TEST-101, TEST-102
- CON-026
- ADR-049

### REQ-087: Serve-Mode History API Endpoints

When `zetl serve` is running with the `history` feature enabled, the server SHALL expose the following JSON API endpoints:

1. `GET /api/history` — Returns the vault timeline (same schema as CON-025 JSON output for `zetl history`)
2. `GET /api/history/page/{name}` — Returns the page timeline (same schema as CON-025 JSON output for `zetl history page <name>`)
3. `GET /api/history/at?t=<iso8601>` — Returns the full vault context (`VaultContext` + `StatsContext`) at the specified historical point, for theme JS to re-render the dashboard
4. `GET /api/history/diff?from=<time>&to=<time>` — Returns a `GraphDelta` between two time points

All endpoints SHALL return `Content-Type: application/json`. When history is unavailable, all endpoints SHALL return HTTP 404 with `{"error": {"code": "NO_HISTORY", ...}}`.

The `/api/history/at` endpoint SHALL use the historical index cache (REQ-079) and SHALL NOT block other requests while scanning.

Trace:
- TEST-103, TEST-104, TEST-110, TEST-111
- CON-027

### REQ-088: Build-Mode History Export

When `zetl build` is run with the `history` feature enabled and history is available, the build pipeline SHALL write a `history-index.json` file to the output directory. This file SHALL contain:

```json
{
  "vault": {
    "oldest": "2026-02-20T09:00:00Z",
    "newest": "2026-03-04T10:00:00Z",
    "snapshot_count": 147,
    "unique_states": 23,
    "trend": [ ... ]
  },
  "pages": {
    "Architecture Overview": {
      "created_at": "2026-02-21T14:30:00Z",
      "last_changed": "2026-03-03T11:22:00Z",
      "age_days": 11,
      "stable_days": 1,
      "link_trend": [ ... ]
    }
  },
  "timeline": [ ... ]
}
```

The `trend` and `timeline` arrays SHALL be limited to the last 30 entries to bound file size. Per-page `link_trend` arrays SHALL be limited to the last 10 entries per page.

The history index JSON SHALL be passed to templates as `history_index` (analogous to the existing `bm25_index` for search), enabling theme JavaScript to load and navigate history client-side without a server.

When history is unavailable, `history-index.json` SHALL NOT be written, and `history_index` SHALL be an empty string in the template context.

Trace:
- TEST-105, TEST-106
- CON-026
- ADR-049

### REQ-089: Backlink Timestamps in Page Context

When history is available, each entry in `page.backlinks` SHALL be extended with an optional `since` field containing the ISO 8601 timestamp of the earliest snapshot in which that backlink existed. This enables themes to display "Linked since Mar 1" alongside each backlink.

```
page.backlinks[]:
  .title                        # (existing) Source page name
  .slug                         # (existing) Source page slug
  .line                         # (existing) Line number in source
  .since                        # (new, optional) ISO 8601 — when this backlink first appeared
```

When history is unavailable, `since` SHALL be `null` for all backlinks. Templates using `{{ bl.since }}` without a guard SHALL render an empty string, not an error.

Trace:
- TEST-107
- CON-026

### REQ-090: Hook Context — History Fields

When history is available, the hook context (SPEC-016) SHALL be extended with a `history` object in the base context passed to all hooks:

```json
{
  "history": {
    "snapshot_count": 147,
    "oldest": "2026-02-20T09:00:00Z",
    "newest": "2026-03-04T10:00:00Z",
    "vault_root_hash": "a3f8c9d1...",
    "previous_vault_root_hash": "b7e2f4a0...",
    "delta": {
      "pages_added": ["New Idea"],
      "pages_removed": [],
      "links_added": [{"from": "New Idea", "to": "Log"}],
      "links_removed": []
    }
  }
}
```

The `delta` field contains the graph diff between the current snapshot and the previous one, enabling hooks to react to graph changes (e.g., a post-index hook that sends a notification when orphans increase).

When history is unavailable, the `history` field SHALL be `null`.

Trace:
- TEST-108

### REQ-091: TUI Timeline Navigation

When the `history` feature is enabled and history is available, the TUI dashboard (SPEC-009) SHALL provide temporal navigation:

- A status-bar indicator showing the available history range and current position
- `[` key to step to the previous snapshot
- `]` key to step to the next snapshot (or return to live state)
- `Shift+[` / `Shift+]` to jump backward/forward by one day
- `n` or `Escape` to return to the live (current) state
- When viewing a historical state, all dashboard panels (link list, graph stats, search) reflect that state
- Visual differentiation (e.g., dimmed status bar color) when viewing historical state vs live

Trace:
- TEST-094

---

## 5. Non-Functional Requirements

### NFR-026: Snapshot Creation Latency

Creating a jj snapshot after `zetl index` SHALL add ≤ 50ms of wall-clock time to the index operation FOR a vault of 2,000 files WITH 95th percentile confidence. This is amortised over the existing index time; the snapshot is the incremental cost.

Trace:
- TEST-095
- OBS-011

### NFR-027: Historical Index Cache Size

Each cached historical index entry SHALL consume ≤ 1.5× the size of the current `.zetl/index.json` for an equivalent vault. For a 2,000-page vault, this is approximately 1–2 MB per entry. The default cache limit of 100 entries therefore consumes ≤ 200 MB of disk.

Trace:
- TEST-086

### NFR-028: Point-in-Time Query Latency — Cache Hit

When a cached historical index exists for the requested snapshot, `zetl links <page> --at <time>` SHALL complete in ≤ 100ms FOR a 2,000-page vault WITH 95th percentile confidence. This is comparable to the present-time query latency.

Trace:
- TEST-096
- OBS-011

### NFR-029: Point-in-Time Query Latency — Cache Miss

When no cached index exists and the historical file tree must be scanned, `zetl links <page> --at <time>` SHALL complete in ≤ 3 seconds FOR a 2,000-page vault WITH 95th percentile confidence. This includes jj tree materialisation, scanning, and cache write.

Trace:
- TEST-097
- OBS-011

### NFR-030: jj-lib Binary Size Impact

The `history` feature SHALL add ≤ 15 MB to the compiled binary size. This is acceptable given zetl is distributed as a single static binary. The feature is opt-in; builds without `history` are unaffected.

Trace:
- TEST-098

### NFR-032: Build-Mode History Export Size

The `history-index.json` file written by `zetl build` SHALL be ≤ 500 KB FOR a vault of 2,000 pages WITH 30 trend points and 10 link trend points per page. This is bounded by the trend and timeline limits specified in REQ-088.

Trace:
- TEST-106

### NFR-033: Template Context Build Latency

Computing `vault.history` and `page.history` for all pages during `zetl build` SHALL add ≤ 2 seconds to the build pipeline FOR a vault of 2,000 pages with 100 cached historical index snapshots WITH 95th percentile confidence.

Trace:
- TEST-109
- OBS-011

### NFR-031: VCS-Independence Preservation

All zetl commands except `zetl diff` SHALL continue to function identically without the `--at` flag whether or not the `history` feature is compiled in, and whether or not `.zetl/jj/` exists. This extends NFR-017 (SPEC-006 §1.6): the `history` feature adds capabilities but never removes them.

`zetl diff` is explicitly exempt: when the `history` feature is enabled, its resolution backend changes from git subprocesses to jj-lib (REQ-083, ADR-046), which alters baseline resolution granularity (snapshots between git commits become visible) and enables operation without a git repository. This is an intentional capability upgrade, not a regression. When the `history` feature is disabled, `zetl diff` falls back to SPEC-007 behavior unchanged.

Trace:
- TEST-093

---

## 6. Architecture Decisions

### ADR-044: jj-lib as Embedded VCS Engine

**Decision:** Embed `jj-lib` as a Rust library dependency (behind a `history` Cargo feature flag) for all temporal operations, replacing git subprocess calls.

**Context:** SPEC-007 uses git subprocesses for graph diff reconstruction. This works but has structural limitations: it requires the `git` binary, parses stdout text, cannot access uncommitted history, and cannot support point-in-time queries without full checkout. jj-lib provides a type-safe Rust API for all VCS operations, uses git as its storage backend (maintaining compatibility), and automatically snapshots the working copy.

**Rationale:** Using jj-lib as a library rather than a CLI tool gives zetl:

1. **No external dependency** — the `jj` binary need not be installed; everything compiles into zetl's single binary
2. **Type-safe API** — no stdout parsing, no string-to-SHA conversion, no subprocess error handling
3. **Working-copy snapshotting** — jj's core innovation; every file state is captured without user action
4. **Git compatibility** — jj reads/writes git objects natively; existing git history is fully accessible
5. **Revset engine** — powerful query language for history traversal, used internally (not exposed to users)

**Trade-offs:**

- (+) Eliminates external `git` dependency for temporal features
- (+) Captures history between explicit git commits
- (+) Single binary; no installation friction
- (-) jj-lib is pre-1.0; API may change between versions
- (-) Adds ~10-15 MB to binary size
- (-) New dependency to maintain; jj-lib updates may require adaptation

**Mitigation for API instability:** The `VcsBackend` trait (REQ-080) isolates zetl's core from jj-lib's API surface. Pin to a specific jj-lib version; update deliberately.

**Rejected alternatives:**

1. *jj CLI via subprocess* — Same fragility as git subprocesses; does not solve SPEC-007's structural issues
2. *gitoxide (gix) library* — Rust-native git, but lacks automatic snapshotting; zetl would need to implement its own commit-on-index logic
3. *Custom snapshot format* — Rejected in SPEC-007 (ADR-011) as reimplementing VCS; using an actual VCS library is the principled approach

### ADR-045: VCS Metadata in `.zetl/jj/`

**Decision:** Store jj repository state inside `.zetl/jj/` rather than at the vault root (`.jj/`).

**Context:** jj normally creates a `.jj/` directory at the workspace root. For zetl, this would place a `.jj/` directory in the user's vault — visible in file explorers, potentially confusing, and requiring `.gitignore` entries in git-tracked vaults.

**Rationale:** `.zetl/` is already zetl's private state directory. Users know to ignore it. Placing jj state inside it maintains the invisibility principle: no new dotfiles appear in the vault. jj-lib's `Workspace` API supports configuring the repo path independently of the working copy root, enabling this separation.

**Trade-offs:**

- (+) No new files/directories visible to the user
- (+) Consistent with existing `.zetl/` convention
- (+) No `.gitignore` changes needed
- (-) Non-standard jj layout; cannot use `jj` CLI directly against the repo (this is intentional — the user should not need to)

**Fallback:** If jj-lib's API does not support custom repo paths in a future version, a symlink `.zetl/jj/ → .jj/` with `.jj/` added to `.gitignore` is acceptable.

### ADR-046: Supersede SPEC-007 VCS Backend

**Decision:** Replace SPEC-007's git-subprocess implementation with jj-lib for all VCS operations while preserving the diff output contract (CON-021).

**Context:** SPEC-007 defined `zetl diff` using git subprocesses (`git show`, `git diff`, `git rev-parse`). NFR-019 mandated subprocess isolation. This was appropriate when git was the only VCS option. jj-lib provides a strictly superior implementation path: same git history access, plus automatic snapshots, plus a library API.

**Rationale:** Maintaining two VCS backends (git subprocess for SPEC-007, jj-lib for new features) creates unnecessary complexity. jj-lib can do everything git subprocesses can — it reads the same git objects — plus more. Consolidating on one backend simplifies the codebase and testing surface.

**Migration path:**

1. When `history` feature is enabled: all VCS operations use jj-lib
2. When `history` feature is disabled: `zetl diff` falls back to SPEC-007's git subprocess implementation (the existing code is retained, not deleted)
3. NFR-019 (git subprocess isolation) applies only to the fallback path

**Trade-offs:**

- (+) Single VCS backend for all temporal features
- (+) Eliminates subprocess overhead for diff operations
- (+) Unlocks point-in-time queries and automatic snapshotting
- (-) `zetl diff` behavior changes subtly: it now sees snapshots between git commits, so `--since yesterday` may resolve to a snapshot rather than a git commit

### ADR-047: Content-Addressed Historical Index Cache

**Decision:** Cache historical index snapshots keyed by `vault_root_hash` (BLAKE3), not by jj change ID or timestamp.

**Context:** When the user queries `zetl links Foo --at yesterday`, zetl must reconstruct the graph at that point. This requires scanning all vault files at the historical revision — an expensive operation. Caching avoids repeated scans.

**Rationale:** The `vault_root_hash` is already computed during `zetl index` (SPEC-006 §4.6). It is a deterministic function of vault content. Two snapshots with identical `vault_root_hash` have identical graph topology — the same pages, same links, same orphans. Keying the cache by content hash rather than by time or change ID provides natural deduplication: many snapshots may map to the same graph state (e.g., when non-Markdown files changed, or when changes were reverted).

**Properties:**

- **Correctness by construction:** same hash = same graph, always
- **Natural deduplication:** 50 snapshots over a quiet week may produce only 3 distinct cache entries
- **VCS-agnostic:** the cache is valid regardless of whether the snapshot came from jj, git, or a manual file tree

**Trade-offs:**

- (+) Deduplication reduces disk usage and scan count
- (+) Cache validity does not depend on VCS state
- (-) First access to a new `vault_root_hash` requires a full scan (~1-3 seconds for 2,000 pages)
- (-) Cache eviction by last-access time is a heuristic; recently accessed but rarely recurring hashes may be evicted

### ADR-048: Automatic Snapshotting on Every Index

**Decision:** Create a jj snapshot on every successful `zetl index`, silently and unconditionally (modulo deduplication).

**Context:** The value of temporal navigation depends on history granularity. Git provides history only at explicit commits — often hours or days apart. jj's working-copy snapshot model captures every state, but requires a trigger. `zetl index` is the natural trigger: it is the operation that reads the vault, computes hashes, and updates the cache. It runs before every query.

**Rationale:** Making snapshotting automatic and tied to `zetl index` means:

1. **Every query is preceded by a snapshot** — the history is always current
2. **No user action required** — history capture is a side effect of normal usage
3. **Deduplication prevents bloat** — identical vault states produce no new snapshot (REQ-076)
4. **Watch mode captures continuously** — `zetl watch` calls `zetl index` on every file change (REQ-082)

**Trade-offs:**

- (+) History granularity matches usage frequency
- (+) Zero workflow change for the user
- (+) Deduplication keeps storage bounded
- (-) Storage grows with usage (mitigated by jj's content-addressed deduplication and git pack files)
- (-) Snapshot creation adds latency to `zetl index` (bounded by NFR-026: ≤ 50ms)

### ADR-049: Temporal Data as Template Context, Not Custom Tags

**Decision:** Expose history data through the existing template context system (`vault.history`, `page.history`) rather than introducing new Jinja2 tags, filters, or template functions.

**Context:** There are several ways to make temporal data available to themes:

1. **Template context objects** (selected): Add `vault.history` and `page.history` to the existing `VaultContext` and `PageContext` structs. Themes access data via `{{ vault.history.trend }}` etc.
2. **Custom Jinja2 filters/functions**: Register `{% history_trend %}` or `{{ page | age_days }}` as custom template functions. More expressive but couples themes to zetl-specific template extensions.
3. **Separate data files only**: Write `history-index.json` and let themes load it via JavaScript. No template-side access.

**Rationale:** Option 1 aligns with how all existing data flows to themes. `vault.stats`, `page.backlinks`, `page.outlinks` are all template context objects. Adding `vault.history` follows the same pattern — no new concepts for theme developers to learn. The context is identical in serve and build modes, ensuring themes work in both.

Option 2 introduces zetl-specific template syntax that breaks portability and adds learning overhead. Option 3 forces all temporal rendering to JavaScript, making server-rendered (build mode) sparklines impossible.

For interactive features (timeline slider, animated graph), option 1 is supplemented by serve-mode API endpoints (REQ-087) and build-mode JSON export (REQ-088) — the same dual-mode pattern already used for search.

**Trade-offs:**

- (+) Zero new concepts for theme developers
- (+) Works in both serve and build modes
- (+) Graceful degradation via `{% if vault.history %}` — existing pattern
- (+) Pre-computed data; no template-time I/O
- (-) Context size increases (bounded by trend limits: ≤ 30 vault points, ≤ 10 per page)
- (-) Cannot express complex temporal queries in templates (intentional — themes show data, don't query)

### ADR-050: Dual-Mode History — Serve API + Build JSON Export

**Decision:** Provide temporal data to themes through two complementary channels: live API endpoints in serve mode, and a pre-baked `history-index.json` in build mode.

**Context:** Themes that want JavaScript-driven interactivity (timeline slider, animated graph, dynamic filtering) need access to temporal data beyond what's in the template context. In serve mode, a live API is natural. In build mode (static site), there is no server — data must be pre-computed.

**Rationale:** This follows the exact pattern established by the BM25 search feature:

| Feature | Serve mode | Build mode |
|---|---|---|
| **Search** | `GET /api/search?q=...` (live Tantivy query) | `search-index.json` (client-side BM25 in JS) |
| **History** | `GET /api/history/*` (live jj-lib query) | `history-index.json` (client-side timeline in JS) |

Theme developers already understand this pattern. The template context (`vault.history`, `page.history`) handles server-rendered content (sparklines, badges, age labels). The JSON export / API handles client-rendered interactivity (timeline slider, graph animation).

**Trade-offs:**

- (+) Proven pattern; theme developers already work this way for search
- (+) Static sites work from `file://` with no server
- (+) API endpoints are simple JSON — easy to consume from any JS framework
- (-) Build-mode JSON is a snapshot, not queryable (e.g., cannot `--at` arbitrary times)
- (-) Two code paths to maintain (mitigated: both use the same underlying `HistoricalIndexCache`)

---

## 7. Purity Boundary Map

### Pure Core (no I/O, no shared state, deterministic)

- **Time expression parsing**: `parse_time_expr(&str) → Result<DateTime>` — converts human-readable time strings to timestamps
- **Graph delta computation**: `compute_graph_delta(IndexSnapshot, IndexSnapshot) → GraphDelta` — set differences between two index states
- **Page history extraction**: `extract_page_history(page: &str, snapshots: &[(DateTime, IndexSnapshot)]) → Vec<PageHistoryEntry>` — filters and diffs snapshots for a single page
- **Timeline collapsing**: `collapse_timeline(entries: &[TimelineEntry]) → Vec<TimelineEntry>` — merges adjacent entries with identical `vault_root_hash`
- **Snapshot deduplication check**: `should_snapshot(current_hash: &ContentHash, previous_hash: &ContentHash) → bool` — pure comparison
- **Cache key derivation**: `cache_path(vault_root_hash: &ContentHash) → PathBuf` — deterministic path from hash
- **Vault history context building**: `build_vault_history(snapshots: &[(DateTime, IndexSnapshot)], limit: usize) → VaultHistoryContext` — samples trend points, computes recent changes
- **Page history context building**: `build_page_history(page: &str, snapshots: &[(DateTime, IndexSnapshot)], limit: usize) → Option<PageHistoryContext>` — per-page temporal metadata
- **Backlink timestamp resolution**: `resolve_backlink_since(page: &str, source: &str, snapshots: &[(DateTime, IndexSnapshot)]) → Option<DateTime>` — finds earliest snapshot containing a backlink
- **History index serialisation**: `serialize_history_index(vault_history: &VaultHistoryContext, page_histories: &HashMap<String, PageHistoryContext>, timeline: &[TimelineEntry]) → Value` — produces the JSON for `history-index.json`
- **Trend sampling**: `sample_trend(entries: &[TimelineEntry], max_points: usize) → Vec<TrendPoint>` — evenly samples N points from a timeline for sparkline rendering

### Effectful Shell (orchestrates I/O, calls pure core)

- **`JjBackend`**: Wraps jj-lib; opens repo, creates snapshots, resolves time expressions to changes, reads file trees
- **`HistoricalIndexCache`**: Reads/writes `.zetl/history/*.json` files; manages eviction
- **`HistoricalScanner`**: Given a jj `MergedTree`, streams file contents into the existing scan pipeline and produces an `IndexSnapshot`
- **VCS initialisation**: Detects `.git/`, creates `.zetl/jj/`, configures jj workspace
- **History API handlers**: Axum route handlers for `/api/history/*` endpoints; read from cache, delegate to pure core for computation
- **History export writer**: Writes `history-index.json` during `zetl build`; calls pure core serialisation

### Boundary Contracts (data types crossing the boundary)

- `IndexSnapshot` (core → shell, shell → core): the parsed vault state at a point in time, identical to the in-memory representation of `index.json`
- `GraphDelta` (core → shell → user): structured diff record matching CON-021 schema
- `TimelineEntry` (core → shell → user): snapshot metadata + graph summary + delta
- `PageHistoryEntry` (core → shell → user): per-page snapshot with link counts and delta
- `ContentHash` ([u8; 32]) (shell → core, core → shell): vault root hash for cache keying
- `VaultHistoryContext` (core → shell → template): vault-level temporal data for `vault.history`
- `PageHistoryContext` (core → shell → template): page-level temporal data for `page.history`
- `TrendPoint` (core → shell → template/JSON): single data point in a sparkline-ready trend array

### Dependency Rule

Dependencies point inward: shell → core. Core MUST NOT import from shell. The pure core functions operate on `IndexSnapshot` and `GraphDelta` values; they never touch jj-lib types, file systems, or network.

### Enforcement

- Module structure: `src/history/mod.rs` (shell), `src/history/core.rs` (pure), `src/history/jj_backend.rs` (jj-lib wrapper)
- The `core` module SHALL NOT depend on `jj-lib`, `std::fs`, or `tokio`
- CI check: `cargo check` on `history/core.rs` with a deny-list for effectful imports

---

## 8. Contract Specifications

### CON-024: `--at <time-expr>` Flag

**Interface:** `zetl <subcommand> [args] --at <time-expr> [--format json|table]`

**Applicable subcommands:** `links`, `backlinks`, `check`, `graph`, `search`, `blocks`, `reason`

**Pre-conditions:**

- `history` feature is compiled in
- `.zetl/jj/` exists (at least one `zetl index` has been run)
- At least one snapshot exists at or before the resolved time

**Post-conditions:**

- Exit 0 on success
- Output format identical to present-time invocation, with an additional `snapshot` field in JSON output:

```json
{
  "snapshot": {
    "timestamp": "2026-03-01T14:30:00Z",
    "change_id": "kxryzmql",
    "vault_root_hash": "a3f8c9d1..."
  },
  "links": [ ... ]
}
```

- In table format, a header line indicates the historical state:

```
[snapshot: 2026-03-01 14:30 UTC]

Forward links from "Architecture Overview":
  → Design Patterns
  → System Boundaries
  ...
```

**Error model:**

```json
{ "error": { "code": "NO_HISTORY", "message": "No history available. Run zetl index to create the first snapshot." } }
{ "error": { "code": "SNAPSHOT_NOT_FOUND", "message": "No snapshot found at or before 2025-01-01. Earliest snapshot: 2026-02-20T09:00:00Z." } }
{ "error": { "code": "HISTORY_NOT_AVAILABLE", "message": "The --at flag requires the history feature. Rebuild zetl with --features history." } }
```

**Implements:** REQ-077, REQ-078

**Verified by:** TEST-082, TEST-083, TEST-084, TEST-085

### CON-025: `zetl history`

**Interface:** `zetl history [--since <time-expr>] [--limit N] [--format json|table]`

**Subcommand:** `zetl history page <name> [--since <time-expr>] [--limit N] [--format json|table]`

**Pre-conditions:**

- `history` feature is compiled in
- `.zetl/jj/` exists with at least one snapshot

**Post-conditions — `zetl history`:**

- Exit 0 on success (including empty timeline)
- JSON output:

```json
{
  "timeline": [
    {
      "timestamp": "2026-03-04T10:00:00Z",
      "change_id": "kxryzmql",
      "vault_root_hash": "a3f8c9d1...",
      "summary": {
        "pages": 412,
        "links": 1847,
        "orphans": 3,
        "dead_links": 1
      },
      "delta": {
        "pages_added": ["New Idea"],
        "pages_removed": [],
        "links_added": [{"from": "New Idea", "to": "Research Log"}],
        "links_removed": [],
        "orphans_gained": [],
        "orphans_resolved": ["Old Draft"],
        "dead_links_added": [],
        "dead_links_resolved": []
      }
    }
  ]
}
```

- Table output:

```
Timestamp             Pages  Links  Orphans  Dead   Delta
─────────────────────────────────────────────────────────────
2026-03-04 10:00 UTC   412   1847     3       1    +1 page, +1 link, −1 orphan
2026-03-03 16:45 UTC   411   1846     4       1    (no change)
2026-03-03 09:12 UTC   411   1846     4       1    −2 links, +1 orphan
...
```

**Post-conditions — `zetl history page <name>`:**

- JSON output:

```json
{
  "page": "Architecture Overview",
  "timeline": [
    {
      "timestamp": "2026-03-04T10:00:00Z",
      "links": 8,
      "backlinks": 6,
      "is_orphan": false,
      "delta": {
        "links_added": [],
        "links_removed": [],
        "backlinks_added": ["New Idea"],
        "backlinks_removed": []
      }
    }
  ]
}
```

- Table output:

```
Page: Architecture Overview

Timestamp             Links  Backlinks  Orphan  Delta
──────────────────────────────────────────────────────────
2026-03-04 10:00 UTC    8       6        no     +1 backlink
2026-02-25 09:41 UTC    8       5        no     −4 links, −2 backlinks
2026-02-25 09:15 UTC   12       7        no     −2 links
2026-02-24 14:32 UTC   14       7        no     (first snapshot)
```

**Error model:**

```json
{ "error": { "code": "NO_HISTORY", "message": "No history available. Run zetl index to create the first snapshot." } }
{ "error": { "code": "PAGE_NOT_FOUND", "message": "Page 'Nonexistent' not found in any snapshot." } }
```

**Implements:** REQ-080, REQ-081

**Verified by:** TEST-087, TEST-088, TEST-089

### CON-026: Template History Context

**Interface:** Template variables injected into the Minijinja rendering context

**Available in:** All templates (`base.html`, `index.html`, `page.html`, `folder.html`)

**Pre-conditions:**

- `history` feature is compiled in
- At least one `zetl index` has been run (snapshots exist)

**Vault-level context (`vault.history`):**

```json
{
  "vault": {
    "history": {
      "oldest": "2026-02-20T09:00:00Z",
      "newest": "2026-03-04T10:00:00Z",
      "oldest_epoch": 1740042000,
      "newest_epoch": 1741089600,
      "snapshot_count": 147,
      "unique_states": 23,
      "trend": [
        {
          "timestamp": "2026-02-20T09:00:00Z",
          "pages": 380,
          "links": 1650,
          "orphans": 8,
          "dead_links": 4
        }
      ],
      "recent_changes": [
        {
          "timestamp": "2026-03-04T10:00:00Z",
          "pages_added": ["New Idea"],
          "pages_removed": [],
          "links_added": [{"from": "New Idea", "to": "Research Log"}],
          "links_removed": [],
          "orphans_gained": [],
          "orphans_resolved": ["Old Draft"]
        }
      ]
    }
  }
}
```

**Page-level context (`page.history`):**

```json
{
  "page": {
    "history": {
      "created_at": "2026-02-21T14:30:00Z",
      "last_changed": "2026-03-03T11:22:00Z",
      "age_days": 11,
      "stable_days": 1,
      "link_trend": [
        {
          "timestamp": "2026-02-21T14:30:00Z",
          "links": 5,
          "backlinks": 2,
          "is_orphan": false
        }
      ],
      "recent_changes": [
        {
          "timestamp": "2026-03-03T11:22:00Z",
          "links_added": ["New Target"],
          "links_removed": [],
          "backlinks_added": ["New Source"],
          "backlinks_removed": []
        }
      ]
    }
  }
}
```

**Backlink `since` extension:**

```json
{
  "page": {
    "backlinks": [
      {
        "title": "Source Page",
        "slug": "source-page",
        "line": 15,
        "since": "2026-02-25T09:15:00Z"
      }
    ]
  }
}
```

**Null behavior:** When history is unavailable, `vault.history` and `page.history` are `null`. `backlinks[].since` is `null`. Templates MUST guard with `{% if vault.history %}`.

**Build-mode template variable:** `history_index` — contains the serialised `history-index.json` content as a string, for embedding in `<script>` tags. Empty string when unavailable.

**Implements:** REQ-085, REQ-086, REQ-088, REQ-089

**Verified by:** TEST-099, TEST-100, TEST-101, TEST-102, TEST-105, TEST-107

### CON-027: Serve-Mode History API

**Interface:** HTTP JSON endpoints under `/api/history/`

**Pre-conditions:**

- `zetl serve` is running with `history` feature enabled
- Snapshots exist

**Endpoints:**

**`GET /api/history`**

Returns the vault timeline. Accepts optional query parameters:
- `since` — ISO 8601 datetime filter
- `limit` — maximum entries (default 20)

Response: Same JSON schema as CON-025 `zetl history` output.

**`GET /api/history/page/{name}`**

Returns the page timeline. Accepts optional query parameters:
- `since` — ISO 8601 datetime filter
- `limit` — maximum entries (default 20)

Response: Same JSON schema as CON-025 `zetl history page` output.

**`GET /api/history/at?t=<iso8601>`**

Returns the full vault context at a historical point:

```json
{
  "snapshot": {
    "timestamp": "2026-03-01T14:30:00Z",
    "change_id": "kxryzmql",
    "vault_root_hash": "a3f8c9d1..."
  },
  "vault": {
    "name": "my-vault",
    "pages": [ ... ],
    "stats": {
      "total_pages": 405,
      "total_links": 1830,
      "dead_links": 1,
      "orphans": 4
    }
  }
}
```

Uses the historical index cache (REQ-079). Cache miss triggers background scan.

**`GET /api/history/diff?from=<time>&to=<time>`**

Returns a `GraphDelta` between two time points:

```json
{
  "from": { "timestamp": "...", "change_id": "..." },
  "to": { "timestamp": "...", "change_id": "..." },
  "pages_added": [...],
  "pages_removed": [...],
  "links_added": [...],
  "links_removed": [...],
  "orphans_gained": [...],
  "orphans_resolved": [...],
  "dead_links_added": [...],
  "dead_links_resolved": [...]
}
```

**Error model (all endpoints):**

```json
{ "error": { "code": "NO_HISTORY", "message": "No history available." } }
{ "error": { "code": "SNAPSHOT_NOT_FOUND", "message": "No snapshot found at or before 2025-01-01. Earliest snapshot: 2026-02-20T09:00:00Z." } }
{ "error": { "code": "PAGE_NOT_FOUND", "message": "Page 'Nonexistent' not found in any snapshot." } }
```

- HTTP 404 with `NO_HISTORY` when no snapshots exist
- HTTP 404 with `SNAPSHOT_NOT_FOUND` when the requested time is before the earliest snapshot (mirrors CLI `--at` semantics from CON-024)
- HTTP 404 with `PAGE_NOT_FOUND` when the requested page does not exist (`/api/history/page/{name}` only)
- HTTP 400 for malformed parameters (invalid ISO 8601, missing required params). For `/api/history/diff`, both `from` and `to` are required; omitting either is a 400 error.

Note: `NO_PREVIOUS_SNAPSHOT` applies only to the CLI `zetl diff` command (REQ-083) when invoked without arguments. The `/api/history/diff` endpoint always requires explicit `from` and `to` parameters and does not support a default baseline.

**Implements:** REQ-087

**Verified by:** TEST-103, TEST-104, TEST-110, TEST-111

---

## 9. Test Specifications

### TEST-080: Invisible VCS Initialisation

**Requirement:** REQ-075

**Preconditions:** Empty vault directory with one `.md` file; no `.git/` or `.zetl/` present

**Steps:**

1. Run `zetl index`
2. Verify `.zetl/jj/` directory exists
3. Verify no `.jj/` directory exists at vault root
4. Verify stdout contains no mention of "jj", "jujutsu", "snapshot", or "repository"
5. Verify `zetl index` output is identical to a build without the `history` feature

### TEST-081: Automatic Snapshotting and Deduplication

**Requirement:** REQ-076

**Preconditions:** Vault with 3 `.md` files; `zetl index` run once (initial snapshot exists)

**Steps:**

1. Modify one `.md` file (add a wikilink)
2. Run `zetl index`
3. Verify a new snapshot was created (jj-lib shows 2 changes)
4. Run `zetl index` again without modifying any files
5. Verify no new snapshot was created (still 2 changes; deduplication)
6. Modify the file back to its original content
7. Run `zetl index`
8. Verify a new snapshot was created but its `vault_root_hash` matches the first snapshot's hash (content-addressed deduplication at the cache level)

### TEST-082: Time Expression — ISO 8601

**Requirement:** REQ-077

**Preconditions:** Vault with snapshots spanning multiple days

**Steps:**

1. `zetl links PageA --at "2026-03-01" --format json`
2. Verify `snapshot.timestamp` is at or before `2026-03-01T23:59:59Z`
3. `zetl links PageA --at "2026-03-01T14:30:00Z" --format json`
4. Verify `snapshot.timestamp` is at or before `2026-03-01T14:30:00Z`

### TEST-083: Time Expression — Natural Language

**Requirement:** REQ-077

**Preconditions:** Vault with snapshots from today and yesterday

**Steps:**

1. `zetl links PageA --at yesterday --format json`
2. Verify `snapshot.timestamp` is within yesterday's date range
3. `zetl links PageA --at "2 hours ago" --format json`
4. Verify `snapshot.timestamp` is within 2 hours before command invocation

### TEST-084: Point-in-Time Query — Cache Miss

**Requirement:** REQ-078

**Preconditions:** Vault with history; `.zetl/history/` is empty

**Steps:**

1. `zetl links PageA --at yesterday --format json`
2. Verify correct link results for the historical state
3. Verify `.zetl/history/<hash>.json` file was created
4. Verify the cached file's content matches a fresh scan of the historical tree

### TEST-085: Point-in-Time Query — Cache Hit

**Requirement:** REQ-078

**Preconditions:** Cached historical index exists from TEST-084

**Steps:**

1. `zetl links PageA --at yesterday --format json` (same time as TEST-084)
2. Verify identical results to TEST-084
3. Measure wall-clock time; verify ≤ 100ms (NFR-028)
4. Verify no jj tree materialisation occurred (cache was used)

### TEST-086: Historical Index Cache — Size Limit and Eviction

**Requirement:** REQ-079

**Preconditions:** History cache configured with limit of 3 entries

**Steps:**

1. Create 4 distinct vault states (different `vault_root_hash`), query each with `--at`
2. Verify `.zetl/history/` contains exactly 3 entries
3. Verify the evicted entry is the one with the oldest last-access time
4. Verify querying the evicted state still works (triggers re-scan and re-cache)

### TEST-087: `zetl history` — Timeline Output

**Requirement:** REQ-080

**Preconditions:** Vault with 5 distinct snapshots showing page and link changes

**Steps:**

1. `zetl history --format json`
2. Verify `timeline` array contains entries in reverse chronological order
3. Verify each entry has `timestamp`, `summary`, and `delta` fields
4. Verify `delta` accurately reflects changes between adjacent snapshots
5. Verify collapsed entries (identical `vault_root_hash`) show time ranges

### TEST-088: `zetl history` — Since and Limit

**Requirement:** REQ-080

**Preconditions:** Vault with 20+ snapshots

**Steps:**

1. `zetl history --limit 5 --format json` → verify exactly 5 entries
2. `zetl history --since yesterday --format json` → verify all entries are from today
3. `zetl history --since yesterday --limit 3 --format json` → verify ≤ 3 entries, all from today

### TEST-089: `zetl history page` — Page Evolution

**Requirement:** REQ-081

**Preconditions:** Vault with page "Foo" that had links added and removed across snapshots

**Steps:**

1. `zetl history page Foo --format json`
2. Verify each entry shows correct link/backlink counts for that snapshot
3. Verify only snapshots where Foo's neighborhood changed are included
4. Verify `delta` fields accurately reflect link changes
5. `zetl history page "Nonexistent" --format json` → verify error `PAGE_NOT_FOUND`

### TEST-090: Watch-Mode Snapshot Integration

**Requirement:** REQ-082

**Preconditions:** `zetl watch` running in background; vault has initial snapshot

**Steps:**

1. Modify a `.md` file while watch is running
2. Wait for watch event to fire and re-index
3. Verify a new jj snapshot was created
4. Verify `zetl history --format json` shows the new snapshot
5. Modify the file back to its original content
6. Wait for watch event
7. Verify no duplicate snapshot (deduplication)

### TEST-091: `zetl diff` — Backward Compatibility

**Requirement:** REQ-083

**Preconditions:** Vault in git with commits; `history` feature enabled

**Steps:**

1. `zetl diff --from HEAD~1 --format json`
2. Verify output matches CON-021 schema exactly
3. Verify `from.ref`, `from.commit`, `to.commit` fields present
4. Compare output with a SPEC-007 (git-subprocess) implementation on the same vault
5. Verify semantic equivalence (same pages/links/orphans/dead-links in diff)

### TEST-092: `zetl diff` — Snapshot Resolution

**Requirement:** REQ-083

**Preconditions:** Vault with both git commits and jj-only snapshots between commits

**Steps:**

1. Make an edit, run `zetl index` (creates jj snapshot but no git commit)
2. `zetl diff --since "1 hour ago" --format json`
3. Verify the diff captures the uncommitted change (resolved via jj snapshot, not git commit)
4. `zetl diff --from HEAD --format json`
5. Verify the diff shows the uncommitted change relative to the last git commit

### TEST-093: Graceful Degradation

**Requirement:** REQ-084

**Preconditions:** Vault with index but no `.zetl/jj/` (e.g., jj repo deleted)

**Steps:**

1. `zetl links PageA` → verify normal output (no error)
2. `zetl links PageA --at yesterday` → verify error `NO_HISTORY`
3. `zetl index` → verify `.zetl/jj/` is re-created silently
4. `zetl links PageA --at yesterday` → verify error `SNAPSHOT_NOT_FOUND` (history starts now)

### TEST-094: TUI Timeline Navigation

**Requirement:** REQ-091

**Preconditions:** Vault with 5+ snapshots; TUI dashboard launched

**Steps:**

1. Verify status bar shows history range indicator
2. Press `[` → verify dashboard updates to previous snapshot state
3. Verify graph stats reflect the historical state
4. Press `]` → verify return toward present
5. Press `Shift+[` → verify jump by one day
6. Press `n` → verify return to live state
7. Verify visual differentiation between historical and live views

### TEST-095: Snapshot Creation Latency

**Requirement:** NFR-026

**Preconditions:** 2,000-file vault; jj repo initialised

**Steps:**

1. Run `zetl index` 20 times; measure total wall-clock time
2. Run `zetl index` 20 times with snapshotting disabled (feature-flag off); measure
3. Compute per-invocation overhead: (enabled_total - disabled_total) / 20
4. Verify overhead ≤ 50ms at p95

### TEST-096: Point-in-Time Query — Cache Hit Latency

**Requirement:** NFR-028

**Preconditions:** Cached historical index for target snapshot; 2,000-page vault

**Steps:**

1. Run `zetl links PageA --at <cached-time>` 20 times; measure
2. Verify p95 latency ≤ 100ms

### TEST-097: Point-in-Time Query — Cache Miss Latency

**Requirement:** NFR-029

**Preconditions:** No cached index for target snapshot; 2,000-page vault

**Steps:**

1. Run `zetl links PageA --at <uncached-time>` (first invocation triggers scan)
2. Measure wall-clock time
3. Verify ≤ 3 seconds

### TEST-098: Binary Size Impact

**Requirement:** NFR-030

**Steps:**

1. Build zetl without `history` feature; record binary size
2. Build zetl with `history` feature; record binary size
3. Verify delta ≤ 15 MB

### TEST-099: Vault History Template Context — Serve Mode

**Requirement:** REQ-085

**Preconditions:** Vault with 10+ snapshots spanning 3 days; `zetl serve` running with default theme

**Steps:**

1. Request `GET /` (index page)
2. Inspect the rendered HTML for vault history data (sparkline elements, trend data)
3. Verify `vault.history.trend` contains ≤ 30 data points
4. Verify `vault.history.recent_changes` contains ≤ 10 entries
5. Verify `vault.history.oldest` and `vault.history.newest` are valid ISO 8601 timestamps
6. Verify `vault.history.snapshot_count` matches the actual snapshot count

### TEST-100: Vault History Template Context — Graceful Absence

**Requirement:** REQ-085

**Preconditions:** Vault with no `.zetl/jj/` directory (history unavailable)

**Steps:**

1. Run `zetl serve` or `zetl build`
2. Verify no errors during rendering
3. Verify `vault.history` is `null` in the template context
4. Verify templates that guard with `{% if vault.history %}` skip the history blocks cleanly
5. Verify the rendered output is identical to a build without the `history` feature

### TEST-101: Page History Template Context

**Requirement:** REQ-086

**Preconditions:** Vault with page "Foo" that had links added and removed across 5 snapshots

**Steps:**

1. Request `GET /foo/` (page view)
2. Verify `page.history.created_at` matches the first snapshot containing "Foo"
3. Verify `page.history.age_days` is accurate
4. Verify `page.history.link_trend` contains only snapshots where Foo's links changed
5. Verify `page.history.recent_changes` contains ≤ 5 entries with correct link deltas

### TEST-102: Page History Template Context — New Page

**Requirement:** REQ-086

**Preconditions:** Page created after the latest snapshot (no history for this page)

**Steps:**

1. Create a new page, run `zetl serve` (but do NOT run `zetl index` first)
2. Request the new page
3. Verify `page.history` is `null`
4. Verify the page renders normally without history metadata

### TEST-103: Serve-Mode API — `/api/history`

**Requirement:** REQ-087

**Preconditions:** Vault with 15+ snapshots; `zetl serve` running

**Steps:**

1. `GET /api/history` → verify JSON matches CON-025 timeline schema
2. `GET /api/history?limit=5` → verify ≤ 5 entries
3. `GET /api/history?since=<yesterday>` → verify all entries are from today
4. Verify `Content-Type: application/json`

### TEST-104: Serve-Mode API — `/api/history/at`

**Requirement:** REQ-087

**Preconditions:** Vault with snapshots spanning multiple days

**Steps:**

1. `GET /api/history/at?t=<yesterday-iso>` → verify response contains historical `vault` context
2. Verify `snapshot.timestamp` is at or before the requested time
3. Verify `vault.stats` reflects the historical state (different from current)
4. `GET /api/history/at?t=2020-01-01` → verify HTTP 404 with `SNAPSHOT_NOT_FOUND`
5. With history unavailable: `GET /api/history/at?t=now` → verify HTTP 404 with `NO_HISTORY`

### TEST-110: Serve-Mode API — `/api/history/page/{name}`

**Requirement:** REQ-087

**Preconditions:** Vault with page "Foo" that had links added and removed across snapshots; `zetl serve` running

**Steps:**

1. `GET /api/history/page/Foo` → verify JSON matches CON-025 page timeline schema
2. Verify each entry shows correct link/backlink counts and deltas
3. `GET /api/history/page/Foo?limit=3` → verify ≤ 3 entries
4. `GET /api/history/page/Nonexistent` → verify HTTP 404 with `PAGE_NOT_FOUND`
5. With history unavailable: `GET /api/history/page/Foo` → verify HTTP 404 with `NO_HISTORY`

### TEST-111: Serve-Mode API — `/api/history/diff`

**Requirement:** REQ-087

**Preconditions:** Vault with snapshots spanning multiple days; `zetl serve` running

**Steps:**

1. `GET /api/history/diff?from=<yesterday-iso>&to=<today-iso>` → verify JSON matches GraphDelta schema
2. Verify `from.timestamp` and `to.timestamp` are resolved correctly
3. Verify `pages_added`, `links_added`, etc. reflect actual changes between the two points
4. `GET /api/history/diff?from=2020-01-01&to=<today>` → verify HTTP 404 with `SNAPSHOT_NOT_FOUND`
5. `GET /api/history/diff?from=invalid` → verify HTTP 400

### TEST-105: Build-Mode History Export — File Written

**Requirement:** REQ-088

**Preconditions:** Vault with 20+ snapshots; history available

**Steps:**

1. Run `zetl build -o dist/`
2. Verify `dist/history-index.json` exists
3. Parse the JSON; verify `vault.trend` contains ≤ 30 entries
4. Verify `pages` object contains entries for each page with `created_at`, `age_days`, `link_trend`
5. Verify per-page `link_trend` arrays contain ≤ 10 entries

### TEST-106: Build-Mode History Export — Size Bound

**Requirement:** REQ-088, NFR-032

**Preconditions:** 2,000-page vault with 100+ snapshots

**Steps:**

1. Run `zetl build -o dist/`
2. Measure `dist/history-index.json` file size
3. Verify ≤ 500 KB

### TEST-107: Backlink Timestamps

**Requirement:** REQ-089

**Preconditions:** Page "Foo" with backlink from "Bar" that first appeared in a snapshot 3 days ago

**Steps:**

1. Request `GET /foo/` or render via `zetl build`
2. Verify `page.backlinks` contains entry for "Bar" with `since` field
3. Verify `since` matches the timestamp of the earliest snapshot where Bar→Foo link existed
4. Verify `since` is `null` when history is unavailable

### TEST-108: Hook Context — History Fields

**Requirement:** REQ-090

**Preconditions:** Vault with history; post-index hook configured

**Steps:**

1. Modify a file, run `zetl index`
2. Capture the JSON context passed to the post-index hook
3. Verify `history` object is present with `snapshot_count`, `oldest`, `newest`, `vault_root_hash`
4. Verify `history.delta` contains the graph diff from the previous snapshot
5. With history unavailable: verify `history` is `null` in hook context

### TEST-109: Template Context Build Latency

**Requirement:** NFR-033

**Preconditions:** 2,000-page vault with 100 cached historical index snapshots

**Steps:**

1. Run `zetl build` 10 times; measure total wall-clock time
2. Run `zetl build` 10 times without history feature; measure
3. Compute per-build overhead: (enabled_total - disabled_total) / 10
4. Verify overhead ≤ 2 seconds at p95

### TEST-112: `zetl diff` — Non-Git Default Baseline

**Requirement:** REQ-083

**Preconditions:** Vault with NO `.git/` directory; `history` feature enabled; 3+ distinct snapshots exist (different `vault_root_hash` values)

**Steps:**

1. `zetl diff --format json` (no `--from`, no `--since`)
2. Verify `from.ref` is `"@-"`
3. Verify `from.snapshot` is a valid ISO 8601 timestamp
4. Verify `from.vault_root_hash` differs from the current vault root hash
5. Verify the diff reflects changes between the previous distinct snapshot and the current state
6. Verify the output schema otherwise matches CON-021 (same fields, same structure)

### TEST-113: `zetl diff` — No Previous Snapshot Error

**Requirement:** REQ-083

**Preconditions:** Vault with only one snapshot (first `zetl index` just ran); `history` feature enabled

**Steps:**

1. `zetl diff --format json` (no arguments)
2. Verify error code `NO_PREVIOUS_SNAPSHOT`
3. Verify error message mentions running `zetl index` after making changes

---

## 10. Observability

### OBS-011: History Operation Timing

**Signal:** Verbose output when `--verbose` is passed to any temporal operation

```
[zetl] snapshot: created change kxryzmql (vault_root_hash=a3f8c9d1) duration_ms=12
[zetl] at: resolved "yesterday" → 2026-03-03T16:45:00Z (change=kxryzmql)
[zetl] at: cache hit vault_root_hash=a3f8c9d1 duration_ms=3
[zetl] at: cache miss vault_root_hash=b7e2f4a0 scan_files=412 duration_ms=1847
[zetl] history: loaded 20 snapshots duration_ms=45
```

**Purpose:** Verify NFR-026, NFR-028, NFR-029; diagnose slow temporal queries; observe cache hit rates.

### OBS-012: History Storage Metrics

**Signal:** Included in `zetl graph stats` output when history is available

```
History:
  Snapshots:      147
  Unique states:   23  (vault_root_hash deduplication)
  Oldest:         2026-02-20 09:00 UTC
  Newest:         2026-03-04 10:00 UTC
  Cache entries:   23
  Cache size:      28.4 MB
```

**Purpose:** Monitor storage growth; validate deduplication effectiveness; inform cache limit tuning.

### OBS-013: Template History Context Timing

**Signal:** Verbose output during `zetl serve` and `zetl build`

```
[zetl] history-context: vault trend=30 points recent=10 changes duration_ms=85
[zetl] history-context: page "Foo" trend=8 points created=2026-02-21 duration_ms=12
[zetl] history-export: wrote history-index.json (142 KB, 412 pages) duration_ms=340
```

**Purpose:** Verify NFR-033; diagnose slow builds caused by history context computation; identify pages with expensive history.

---

## 11. Traceability Matrix

| REQ     | NFR     | CON     | ADR              | TEST              | OBS     |
| ------- | ------- | ------- | ---------------- | ----------------- | ------- |
| REQ-075 | NFR-031 | —       | ADR-044, ADR-045 | TEST-080          | —       |
| REQ-076 | NFR-026 | —       | ADR-048          | TEST-081          | OBS-011 |
| REQ-077 | —       | CON-024 | —                | TEST-082, TEST-083| —       |
| REQ-078 | NFR-028, NFR-029 | CON-024 | ADR-047 | TEST-084, TEST-085 | OBS-011 |
| REQ-079 | NFR-027 | —       | ADR-047          | TEST-086          | OBS-012 |
| REQ-080 | —       | CON-025 | —                | TEST-087, TEST-088| OBS-012 |
| REQ-081 | —       | CON-025 | —                | TEST-089          | —       |
| REQ-082 | NFR-026 | —       | ADR-048          | TEST-090          | OBS-011 |
| REQ-083 | —       | CON-021 | ADR-046          | TEST-091, TEST-092, TEST-112, TEST-113 | OBS-011 |
| REQ-084 | NFR-031 | CON-024 | —                | TEST-093          | —       |
| REQ-085 | —       | CON-026 | ADR-049          | TEST-099, TEST-100| OBS-013 |
| REQ-086 | —       | CON-026 | ADR-049          | TEST-101, TEST-102| OBS-013 |
| REQ-087 | —       | CON-027 | ADR-050          | TEST-103, TEST-104, TEST-110, TEST-111 | — |
| REQ-088 | NFR-032 | CON-026 | ADR-049, ADR-050 | TEST-105, TEST-106| OBS-013 |
| REQ-089 | —       | CON-026 | —                | TEST-107          | —       |
| REQ-090 | —       | —       | —                | TEST-108          | —       |
| REQ-091 | —       | —       | —                | TEST-094          | —       |
| —       | NFR-030 | —       | ADR-044          | TEST-098          | —       |
| —       | NFR-033 | —       | —                | TEST-109          | OBS-013 |

---

## 12. Future Work

### 12.1 Reasoning Timeline

`zetl history reason [--since <time>]` would show how SPL conclusions evolved over time. Combined with SPEC-006's drift detection, this would reveal when reasoning conclusions changed due to prose edits vs SPL edits. Requires building a historical theory at each snapshot — expensive but feasible with the cache infrastructure defined here.

### 12.2 Graph Animation Export

`zetl history export --format gif|mp4` could render an animation of the graph evolving over time. Each frame is a snapshot's graph state rendered via the existing graph visualisation. Useful for presentations and documentation.

### 12.3 Diff Between Arbitrary Points

`zetl diff --from "last monday" --to "last friday"` would compute the graph delta between two historical points (not just historical-to-present). This requires reconstructing two historical indexes and diffing them — straightforward with the cache infrastructure but adds a `--to` flag to CON-021.

### 12.4 Branch-Aware History

If the vault uses git branches (e.g., a "drafts" branch), jj-lib's revset engine could support queries like `zetl links Foo --at drafts` to see the graph state on a different branch. This is a natural extension of REQ-077's time expression syntax.
