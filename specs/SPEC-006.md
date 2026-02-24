---
title: "SPEC-006: Content-Addressed Merkle Tree over Markdown and SPL AST"
version: 0.4.0
status: draft
audience: agent, human
date: 2026-02-24
---

# SPEC-006: Content-Addressed Merkle Tree over Markdown and SPL AST

## Information Table

| Field          | Value                                                              |
| -------------- | ------------------------------------------------------------------ |
| Document ID    | SPEC-006                                                           |
| Title          | Content-Addressed Merkle Tree over Markdown and SPL AST            |
| Version        | 0.3.0                                                              |
| Status         | Draft                                                              |
| Author         | Agent (USDD Protocol v1.0.0)                                       |
| Date           | 2026-02-24                                                         |
| Audience       | Agent, Human                                                       |
| Trace          | USDD Agent Protocol v1.0.0                                         |
| Parent         | SPEC-001: zetl — Bi-directional Link Graph CLI                     |
| Related        | SPEC-005: zetl reason — Defeasible Logic over Markdown Vaults      |
| Dependencies   | pulldown-cmark (Markdown AST), spindle-parser (SPL AST), blake3    |

---

## 1. Overview

SPEC-001 established zetl's cache invalidation on file-level modification timestamps (mtime). SPEC-005 extended this to the theory cache: if any SPL-containing file's mtime changes, the entire theory is rebuilt. This works for performance — but it says nothing about **what changed**. Mtime tells you *when* a file was touched, not *whether the content that matters actually differs*.

This specification introduces a **content-addressed Merkle tree** built transparently during `zetl index`, where every block-level node in the Markdown AST is content-hashed into a hierarchical structure. The Merkle tree is invisible infrastructure — users never interact with it directly. Instead, it enables three user-visible capabilities:

1. **Smarter caching** — theory rebuilds only when SPL content actually changes, not when surrounding prose is edited
2. **Drift detection** — `zetl check` warns when prose surrounding an SPL block has changed but the SPL hasn't been updated
3. **Content grounding** — SPL facts and rules are automatically linked to the Markdown prose they formalise, creating a verifiable connection between informal claims and formal logic
4. **Content-addressed references** — `zetl blocks` exposes the Merkle leaf hashes of every content block in a file, giving agents read-only, position-independent references to specific paragraphs, headings, and tables without modifying source files

### 1.1 The Drift Problem

SPL theories embedded in Markdown files make claims that are grounded in the surrounding prose. A note titled "Redis vs Memcached" might contain:

````markdown
## Benchmark Results

We benchmarked Redis at 120k ops/sec under our workload profile.

```spl
(given redis-benchmarked)
(given redis-fast-enough)
(normally r-prefer-redis
  (and redis-benchmarked redis-fast-enough)
  decided-use-redis)
```
````

The SPL block formalises the section's claim. But what happens when the prose changes?

- **Scenario A — Semantic drift:** The author updates the benchmark to 85k ops/sec and adds "below our threshold." The SPL still asserts `redis-fast-enough`. The theory now contradicts its own source document. The SPL has **drifted** from its grounding prose.
- **Scenario B — False invalidation:** The author fixes a typo in a different section. The file's mtime changes. The theory cache is invalidated and rebuilt — unnecessarily, since the SPL content is identical.
- **Scenario C — No content provenance:** An agent creates a note with SPL, then a different agent modifies the prose months later. The second agent has no way to know whether the SPL is still consistent with the updated prose. There is no verifiable link between the theory and a specific version of the prose.

Mtime-based caching handles none of these. It is a performance optimisation, not a correctness mechanism.

### 1.2 The Solution: Content-Addressed Merkle Tree with Grounding

The solution has two parts:

**Part 1: Merkle tree as infrastructure.** A Merkle tree is built during `zetl index` by hashing block-level AST nodes from pulldown-cmark. It propagates upward: leaf hashes → file hashes → vault root hash. This replaces mtime as the authoritative cache invalidation signal and enables precise change detection at the block level.

**Part 2: Section grounding.** Each SPL block is automatically "grounded in" its containing Markdown section — the heading above it through to the next heading at the same or higher level. The section's content hash (computed from its non-SPL Merkle leaves) is stored as the SPL block's **grounding hash**. When the section prose changes but the SPL doesn't, `zetl check` reports a drift warning. For precision, authors can explicitly ground individual SPL facts in specific content blocks using Obsidian's `^block-id` syntax.

```
              ┌─────────────────────┐
              │    Vault Root Hash   │
              └──────────┬──────────┘
                         │
           ┌─────────────┼─────────────┐
           │             │              │
     ┌─────▼─────┐ ┌────▼────┐  ┌─────▼─────┐
     │ File Hash₁ │ │File Hash₂│  │ File Hash₃ │
     └─────┬─────┘ └────┬────┘  └───────────┘
           │             │
     ┌──┬──┴──┬──┐    ┌──┴──┬─────┐
     │  │     │  │    │     │     │
    ┌▼┐┌▼┐ ┌─▼┐┌▼┐  ┌▼┐  ┌▼┐  ┌─▼┐
    │H││P│ │SP││P│  │H│  │P│  │SP│
    └─┘└─┘ └──┘└─┘  └─┘  └─┘  └──┘
     ╰──section──╯    ╰──section──╯
       grounding        grounding
       context          context
```

### 1.3 What This Enables

| Capability | How Users See It |
| --- | --- |
| **Smarter theory caching** | `zetl reason status` is faster — skips rebuilds when only prose changed |
| **Drift warnings** | `zetl check --drift` warns: "SPL in Redis.md §Benchmarks may be stale — section was edited" |
| **Stale provenance detection** | `zetl reason provenance` warns when source content has changed since the theory was built |
| **False invalidation elimination** | `zetl index` doesn't re-process files that were touched but not changed |
| **Content-addressed references** | `zetl blocks` returns Merkle hashes for every content block; `zetl blocks --resolve` maps a hash back to its file and line; agents use hashes as read-only `:source` references without modifying files |
| **Explicit content references** | SPL can pin facts to specific paragraphs via `:source "^block-id"` or `:source "e5f6a7b8"` (Merkle hash) |
| **Cross-agent verification** | Agents can verify that prose grounding a theory hasn't changed since it was built |

### 1.4 Design Philosophy

1. **Invisible infrastructure.** The Merkle tree is an implementation detail of the scanner and cache. Users never see hashes, never run `merkle` commands, never think about tree structure. They see faster caching, drift warnings, and content references.
2. **Content over time.** Mtime answers "when was this touched?" Content hashing answers "what does it say?" Both are useful; content hashing is authoritative.
3. **Mtime as pre-filter.** Hashing is more expensive than stat(). Mtime remains the first check: if mtime hasn't changed, skip hashing. Two-tier invalidation.
4. **Section grounding by default, precision on demand.** SPL blocks are automatically grounded in their containing section — this handles the 80% case. Authors who need tighter coupling use `:source "^block-id"` — this handles the 20%.
5. **AST boundaries, not byte boundaries.** Hashing normalised AST nodes is semantically stable across whitespace and formatting changes.

### 1.5 Scope

**In scope:**

- Merkle tree construction from pulldown-cmark AST events, built during `zetl index`
- SPL block leaves with dual hashing (raw content + parsed SPL AST)
- File-level and vault-level Merkle roots
- Two-tier cache invalidation (mtime + content hash)
- Section grounding: implicit linking of SPL blocks to their containing Markdown section
- Explicit grounding via `:source` — three forms: Merkle hash (agent-friendly, read-only), `^block-id` (human-friendly), `[[Page^block-id]]` (cross-file)
- Content block discovery: `zetl blocks <page>` exposes Merkle leaf hashes for agent consumption; `zetl blocks --resolve <hash>` maps hashes back to source locations
- Drift detection integrated into `zetl check`
- Durable provenance: content hashes in theory provenance metadata

**Out of scope:**

- Low-level Merkle tree inspection commands (no `zetl merkle` subcommand)
- Cryptographic signing of Merkle proofs (future SPEC)
- Distributed Merkle tree synchronisation across vaults (future SPEC)
- Incremental Merkle tree updates (future optimisation; v1 rebuilds file trees from scratch)
- Embedding-based semantic drift detection (future SPEC; this spec covers structural drift only)
- Git-style content-addressable object storage

---

## 2. User Profiles

### 2.1 Agent Operator — Knowledge Builder

```
Role: LLM agent maintaining a knowledge base with SPL theories
Goals:
  - Write notes with SPL blocks that formalise the surrounding prose
  - Get warned when edits to prose invalidate existing SPL claims
  - Avoid unnecessary theory rebuilds when editing non-SPL content
  - Ground SPL facts in specific content blocks without modifying files
Constraints:
  - Requires structured JSON output
  - Invokes CLI commands non-interactively
  - May operate on the same vault as other agents concurrently
Daily workflow:
  1. Read an existing note and want to formalise a claim
  2. Run `zetl blocks "Redis vs Memcached"` to see content blocks with hashes
  3. Write SPL grounding a fact in a specific paragraph:
     (given redis-fast-enough :source "e5f6a7b8")
  4. Run `zetl index` (Merkle tree built transparently)
  5. Run `zetl reason status` (theory uses content hashes for caching)
  6. Later, another agent edits the source paragraph
  7. Run `zetl check --drift` — see that the grounding is stale
  8. Run `zetl blocks --resolve e5f6a7b8` — find that the hash no longer
     resolves (content changed), identify what file/line it used to reference
```

### 2.2 Human Knowledge Worker — Decision Documenter

```
Role: Researcher documenting decisions with formal justification
Goals:
  - Write decision documents where conclusions are formally expressed
  - Get warned when revisiting old decisions that the SPL may be stale
  - Understand which conclusions are grounded in current prose
Constraints:
  - Prefers human-readable table output
  - Doesn't know or care about Merkle trees
  - Wants actionable warnings, not hash values
Daily workflow:
  1. Write notes in Obsidian with ```spl blocks
  2. Run `zetl check` — sees drift warnings alongside dead links and orphans
  3. Review flagged SPL blocks and update or confirm them
  4. Run `zetl reason status` — sees which conclusions are current
```

### 2.3 Agent Team — Multi-Agent Knowledge Coordination

```
Role: Multiple LLM agents contributing to a shared knowledge base (via hence)
Goals:
  - Verify that another agent's prose edits haven't invalidated theories
  - Ground facts in specific evidence using content-addressed references
  - Detect when concurrent edits create drift in shared documents
Constraints:
  - Agents write concurrently (append-only)
  - Hence lifecycle hooks can run `zetl check --drift --fail-on drift`
  - Content hashes serve as coordination checkpoints
Daily workflow:
  1. Hence assigns research task to agent-A
  2. Agent-A reads existing notes, runs `zetl blocks "Redis Benchmarks"`
     to get content hashes for the evidence paragraphs
  3. Agent-A writes SPL grounding facts in those hashes:
     (given redis-fast-enough :source "e5f6a7b8")
     No file modification needed — the hash references content as-is.
  4. Agent-B later edits the benchmark paragraph
  5. Hence post-edit hook: `zetl check --drift --fail-on drift`
     → Fails: "fact redis-fast-enough grounded in e5f6a7b8 — no matching
        content block found (original paragraph was modified)"
  6. Reconciliation agent runs `zetl blocks --resolve e5f6a7b8` to see
     what the hash referenced, discovers it no longer resolves
  7. Agent runs `zetl blocks "Redis Benchmarks"` to get updated hashes,
     rewrites the :source with the new hash
```

### 2.4 Happy Paths

```
Happy Path: Drift Detected During Routine Check

Preconditions:
  - Vault has "Redis.md" with section "## Benchmarks" containing prose
    and an SPL block
  - Theory was built with section grounding hashes recorded
  - Author modifies the benchmark numbers in the prose paragraph
Steps:
  1. `zetl check -d ./vault`
     → Reports alongside dead links and orphans:
       "drift: Redis.md:8 — SPL block in section '## Benchmarks' may be
        stale. Adjacent paragraph (line 5) was modified since the theory
        was built. Review whether SPL claims still hold."
  2. Author reads the file, confirms the SPL needs updating
  3. Author updates the SPL block
  4. `zetl reason status` — theory rebuilt with fresh grounding hashes
Postconditions:
  - All SPL blocks are grounded in current prose
  - No drift warnings on next check
Failure modes:
  - SPL block was moved to a different section → grounding is
    recomputed for the new section; old grounding baseline is discarded
```

```
Happy Path: Cache Avoids Unnecessary Theory Rebuild

Preconditions:
  - Vault was indexed with Merkle hashes cached
  - User edits a file that has no SPL blocks
Steps:
  1. File mtime changes, triggering a reparse by the scanner
  2. Scanner re-extracts wikilinks and computes new Merkle tree
  3. File's Merkle root changes (prose was edited)
  4. Theory cache check: no SPL-containing file's SPL leaf AST hash changed
  5. Theory cache remains valid — no reasoning rebuild
Postconditions:
  - Link index is updated (new prose might have new wikilinks)
  - Theory cache is NOT rebuilt (SPL unchanged)
  - User sees faster response than a full rebuild
Failure modes:
  - None — this is the optimal fast path
```

```
Happy Path: Agent Discovers Content Blocks and Grounds SPL

Preconditions:
  - Vault has "Redis.md" with several sections of prose
  - The vault has been indexed (Merkle tree exists in cache)
Steps:
  1. Agent reads "Redis.md" and decides to formalise the benchmark claim
  2. `zetl blocks "Redis vs Memcached"`
     → Returns content blocks with hashes and text previews:
       [
         {"type": "Heading", "lines": [5,5], "hash": "a1b2...", "text": "## Benchmark Results"},
         {"type": "Paragraph", "lines": [7,9], "hash": "e5f6a7b8", "text": "We benchmarked Redis at 120k ops/sec..."},
         {"type": "Table", "lines": [11,14], "hash": "c9d0...", "text": "| Metric | Value | ..."},
         ...
       ]
  3. Agent identifies the paragraph at hash "e5f6a7b8" as the evidence
  4. Agent writes an SPL block in another file (or the same file):
     (given redis-fast-enough :source "e5f6a7b8")
  5. `zetl index` — reindexes, resolves the hash reference
  6. `zetl check` — no errors, grounding is valid
Postconditions:
  - The fact is grounded in a specific paragraph via content hash
  - No files were modified to add block-id tags
  - If the paragraph is later edited, the hash won't match and
    drift is detected automatically
Failure modes:
  - Hash references a block that was deleted between discovery and
    indexing → zetl check reports broken grounding error
```

```
Happy Path: Explicit Grounding Catches Cross-Section Drift

Preconditions:
  - "Architecture.md" has a section "## Performance" with paragraph
    tagged ^perf-numbers
  - "Decisions.md" has SPL grounded in that paragraph:
    (given performance-acceptable :source "[[Architecture^perf-numbers]]")
  - The performance paragraph in Architecture.md is edited
Steps:
  1. `zetl check --drift`
     → Reports: "drift: Decisions.md:12 — fact 'performance-acceptable'
        grounded in [[Architecture]]^perf-numbers — target content changed"
  2. Agent reviews whether the fact still holds given the new numbers
Postconditions:
  - Cross-file grounding drift is detected
Failure modes:
  - ^perf-numbers block-id no longer exists → error diagnostic:
    "grounding target ^perf-numbers not found in Architecture.md"
```

---

## 3. Content Grounding Model

This section defines how SPL blocks are linked to the Markdown prose they formalise. The grounding model is the primary user-facing feature enabled by the Merkle tree.

### 3.1 Section Grounding (Implicit, Default)

Every SPL block is automatically grounded in its **containing section**. A section is defined as:

1. The nearest preceding heading (any level: `#`, `##`, `###`, etc.)
2. All content between that heading and the next heading at the **same or higher level**, or end of file
3. If no preceding heading exists (SPL block is before the first heading), the grounding context is all content from the start of file to the first heading

The **section grounding hash** is computed as:

```
section_grounding_hash = BLAKE3(
    non_spl_leaf₁_hash ‖ non_spl_leaf₂_hash ‖ … ‖ non_spl_leafₖ_hash
)
```

Only non-SPL leaves within the section contribute to the grounding hash. This means the grounding hash captures the prose, headings, lists, tables, and other content that the SPL block is "about" — but not the SPL block itself. When the prose changes, the grounding hash changes, triggering a drift warning. When only the SPL block changes, the grounding hash is unaffected.

**Example:**

````markdown
## Benchmark Results             ← section start (heading leaf)

We tested Redis at 120k ops/sec  ← paragraph leaf (in grounding context)
under production workload.

| Metric | Value |               ← table leaf (in grounding context)
| ops/sec | 120,000 |
| p99 latency | 2.1ms |

```spl                            ← SPL leaf (NOT in grounding context)
(given redis-benchmarked)
(given redis-fast-enough)
```

More discussion of results.      ← paragraph leaf (in grounding context)

## Next Steps                    ← next section starts (same heading level)
````

The grounding hash for the SPL block at line 10 is computed from the hashes of: the "## Benchmark Results" heading, the paragraph about testing Redis, the metrics table, and the "More discussion" paragraph. Not the SPL block itself.

### 3.2 Explicit Grounding via `:source`

For cases where implicit section grounding is too coarse, authors can explicitly ground individual SPL constructs to specific content blocks using the `:source` metadata key and Obsidian's `^block-id` syntax:

**Content-addressed grounding (agent-friendly, read-only):**

````markdown
```spl
(given redis-fast-enough :source "e5f6a7b8")
```
````

The hash `e5f6a7b8` is a truncated Merkle leaf hash obtained from `zetl blocks`. The system resolves it by searching all Merkle leaves in the vault for a matching prefix. No file modification is needed — the agent references content as-is. If the content changes, the hash no longer matches and drift is detected.

This is the primary mechanism for agents. An agent reads a file, runs `zetl blocks` to discover content hashes, and writes SPL referencing the hashes it cares about.

**Same-file block-id grounding (human-friendly):**

````markdown
We benchmarked Redis at 120k ops/sec under production workload. ^benchmark-results

```spl
(given redis-fast-enough :source "^benchmark-results")
```
````

The fact `redis-fast-enough` is pinned to the paragraph tagged `^benchmark-results`. This requires the `^block-id` tag to exist in the source file. Humans writing in Obsidian naturally use this syntax.

**Cross-file grounding:**

````markdown
```spl
(given performance-acceptable :source "[[Architecture^perf-numbers]]")
```
````

The fact `performance-acceptable` is grounded in the `^perf-numbers` block in `Architecture.md`. Drift detection crosses file boundaries.

**Rule-level grounding:**

````markdown
```spl
(normally r-prefer-redis
  (and redis-benchmarked redis-fast-enough)
  decided-use-redis
  :source "e5f6a7b8")
```
````

An entire rule can be grounded in a specific content block identified by its Merkle hash.

### 3.3 `:source` Syntax

The `:source` key follows spindle-core's existing metadata syntax (SPEC-005 §3.2):

```
source_ref      ::= ':source' '"' target '"'
target          ::= hash_ref | local_ref | cross_file_ref
hash_ref        ::= hex_chars                          (8+ hex characters, Merkle leaf hash prefix)
local_ref       ::= '^' block_id
cross_file_ref  ::= '[[' page_name '^' block_id ']]'
block_id        ::= [a-zA-Z0-9-]+
hex_chars       ::= [0-9a-f]{8,64}
```

**Resolution rules:**

1. **`"e5f6a7b8"` (Merkle hash)** — resolve by prefix match against all Merkle leaf hashes in the vault. The hash is the hex-encoded prefix (minimum 8 characters) of a BLAKE3 leaf hash returned by `zetl blocks`. Resolution is position-independent: the same content at a different line number or even a different file still matches. If the prefix is ambiguous (matches multiple leaves), `zetl check` reports an error suggesting a longer prefix.
2. **`"^block-id"` (local block-id)** — resolve within the same file. Matched to the Merkle leaf containing the `^block-id` suffix.
3. **`"[[Page^block-id]]"` (cross-file block-id)** — resolve across files. Page name resolved via standard wikilink matching (SPEC-001 §3.2).

**Validation:**

- Hash reference matches zero leaves → error: "content hash e5f6a7b8 not found — source content may have been modified or removed"
- Hash reference matches multiple leaves → error: "ambiguous hash prefix e5f6 — use a longer prefix (found in File A line 5, File B line 12)"
- `^block-id` doesn't exist → error (analogous to dead wikilinks)
- `[[Page^block-id]]` page doesn't exist → error

**Why Merkle hashes as the primary agent mechanism:**

- **Read-only** — agents don't need to modify source files to add `^block-id` tags, which is consistent with zetl's read-only design philosophy (SPEC-001 §1.1)
- **Position-independent** — if a paragraph moves within or between files, the hash still resolves as long as the content is unchanged
- **Self-validating** — if the content changes, the hash stops matching, and drift is detected automatically
- **Discoverable** — `zetl blocks <page>` provides all hashes for a file, making it trivial for an agent to pick the right reference

**Multiple sources:**

A single fact or rule can have multiple `:source` references:

```spl
(given meets-requirements
  :source "^perf-numbers"
  :source "[[Security Audit^findings]]")
```

The grounding hash is the combination of all referenced blocks. Drift is detected if any one changes.

### 3.4 Grounding Precedence

When both implicit section grounding and explicit `:source` grounding apply to the same SPL block:

1. **If any construct in the block has an explicit `:source`**, that construct uses only explicit grounding for drift detection. Section grounding still applies to constructs without `:source`.
2. **If no construct has `:source`**, the entire block uses section grounding.
3. **Mixed blocks** (some constructs with `:source`, some without) are valid. Each construct is tracked independently.

This means adding `:source` to one fact doesn't disable section grounding for the other facts in the same block.

### 3.5 Grounding Hash Storage

Grounding hashes are stored in the theory cache (`.zetl/theory.json`), not in a separate file:

```json
{
  "version": 2,
  "vault_root_hash": "a1b2c3d4...",
  "spl_blocks": {
    "decisions/Redis.md:10": {
      "ast_hash": "d5e6f7a8...",
      "content_hash": "c4d5e6f7...",
      "section_grounding_hash": "e7f8a9b0...",
      "explicit_groundings": {
        "redis-fast-enough": {
          "source": "^benchmark-results",
          "target_hash": "f8a9b0c1..."
        }
      }
    }
  },
  "rules": [ ... ],
  "superiorities": [ ... ],
  "diagnostics": [ ... ]
}
```

---

## 4. Merkle Tree Structure

### 4.1 Hash Algorithm

```
ADR-008: Hash Algorithm Selection — BLAKE3

Status: Proposed

Context:
  The Merkle tree requires a hash function that is:
  - Fast: hashing thousands of AST nodes per second
  - Collision-resistant: content addresses must be unique
  - Fixed-width: for compact storage and comparison
  - Widely available: well-maintained Rust crate

  Options evaluated:
  A. SHA-256:
     + Industry standard, universally understood
     + 32 bytes, sufficient collision resistance
     - Slower than alternatives (~500 MB/s on modern hardware)
     - No tree hashing mode

  B. BLAKE3:
     + Extremely fast (~5 GB/s, 10x SHA-256 on modern hardware)
     + Built-in keyed hashing and key derivation
     + 32 bytes default, extendable output
     + Merkle tree mode is built into the design
     + Well-maintained Rust crate (blake3)
     - Less universally recognised than SHA-256

  C. xxHash / FNV:
     + Fastest
     - Not cryptographically secure (collision attacks feasible)
     - Insufficient for content addressing

Decision:
  Use BLAKE3 with 32-byte output. The blake3 crate provides both
  single-shot hashing and an incremental Hasher that maps naturally
  to streaming AST events.

Rationale:
  - 10x speed advantage over SHA-256 matters when hashing every AST
    node in a large vault (10,000+ files × 50+ nodes per file)
  - 32 bytes (256 bits) provides collision resistance well beyond our
    needs (birthday bound at 2^128)
  - The blake3 crate is pure Rust with optional SIMD acceleration

Consequences:
  + Sub-millisecond hashing for typical files (<10 KB)
  + Compact hash storage: 32 bytes per node
  - Adds blake3 as a dependency (~100 KB to binary size)
```

### 4.2 Node Types

```
┌──────────────────────────────────────────────────────────┐
│                       Node Types                          │
├───────────────┬──────────────────────────────────────────┤
│ Leaf          │ Atomic content unit from the AST.        │
│               │ Hash = BLAKE3(type_tag ‖ normalised_content) │
├───────────────┼──────────────────────────────────────────┤
│ SPL Leaf      │ Special leaf for `spl`-tagged blocks.    │
│               │ Hash = BLAKE3(content_hash ‖ ast_hash)   │
│               │   content_hash = BLAKE3(raw_spl_text)    │
│               │   ast_hash = BLAKE3(serialised_spl_ast)  │
├───────────────┼──────────────────────────────────────────┤
│ File Interior │ Intermediate node for one file.          │
│               │ Hash = BLAKE3(child₁_hash ‖ … ‖ childₙ) │
├───────────────┼──────────────────────────────────────────┤
│ Vault Root    │ Top-level node for the entire vault.     │
│               │ Hash = BLAKE3(file₁_hash ‖ … ‖ fileₘ)   │
│               │ Files sorted by canonical relative path. │
└───────────────┴──────────────────────────────────────────┘
```

### 4.3 Leaf Node Construction from pulldown-cmark Events

pulldown-cmark produces a stream of `(Event, Range<usize>)` tuples. The Merkle tree groups these into leaf nodes at the **block level** — each top-level block element becomes one leaf:

| pulldown-cmark Event Sequence | Leaf Type | Hash Input |
| --- | --- | --- |
| `Start(Heading)` … `End(Heading)` | `Heading` | Level byte + normalised text |
| `Start(Paragraph)` … `End(Paragraph)` | `Paragraph` | Normalised text |
| `Start(CodeBlock(Fenced("spl")))` … `End(CodeBlock)` | `SplBlock` | Dual hash (see §4.2) |
| `Start(CodeBlock(…))` … `End(CodeBlock)` | `CodeBlock` | Language tag + raw content |
| `Start(List)` … `End(List)` | `List` | Ordered flag + normalised items |
| `Start(BlockQuote)` … `End(BlockQuote)` | `BlockQuote` | Normalised content |
| `Start(Table)` … `End(Table)` | `Table` | Normalised cells |
| `Start(MetadataBlock)` … `End(MetadataBlock)` | `Frontmatter` | Raw YAML |
| `Rule` | `ThematicBreak` | Constant sentinel |
| `Html(…)` (block-level) | `HtmlBlock` | Raw HTML |

**Normalisation rules:**

1. Collapse consecutive whitespace to a single space
2. Trim leading/trailing whitespace
3. Strip inline formatting markers (bold, italic) — hash plain text content
4. Preserve case (case changes are content changes)
5. Normalise line endings to `\n`

### 4.4 SPL Leaf Dual Hashing

An SPL leaf contains two hashes:

- **`content_hash`** — BLAKE3 of the raw SPL text after normalising whitespace and stripping comments. Changes when the text is edited in any way.
- **`ast_hash`** — BLAKE3 of the canonically-serialised SPL AST (rules sorted by label, facts sorted by literal, superiority sorted by pair). Changes only when the logical content changes. Reformatting and comment edits are invisible.

The **combined hash** (`BLAKE3(content_hash ‖ ast_hash)`) feeds into the file-level Merkle tree. The `ast_hash` alone is used for theory cache invalidation (REQ-041).

### 4.5 Section Boundary Detection

Sections are detected by tracking heading levels during the Merkle leaf construction pass:

```
Input: ordered Vec<MerkleLeaf> for a file

Algorithm:
  sections = []
  current_section_start = 0
  current_level = 0  (0 = before first heading)

  for (index, leaf) in leaves:
    if leaf.type is Heading(level):
      if level <= current_level or current_level == 0:
        // New section: same or higher level heading
        sections.push(Section {
          start: current_section_start,
          end: index - 1,
          heading_level: current_level,
        })
        current_section_start = index
        current_level = level

  // Final section extends to end of file
  sections.push(Section {
    start: current_section_start,
    end: leaves.len() - 1,
    heading_level: current_level,
  })
```

Each section's **grounding hash** is computed by concatenating the hashes of all non-SPL leaves within the section's range and hashing the result.

### 4.6 Vault Root

File hashes are sorted by canonical relative path (UTF-8 lexicographic, forward-slash normalised) before computing the vault root. This ensures deterministic hashes regardless of filesystem scan order.

### 4.7 Standalone SPL Files

Standalone `.spl` files produce a single SPL leaf with dual hashing. Since there is no surrounding Markdown prose, section grounding does not apply. Standalone SPL files can still use explicit `:source` references to ground their content in other Markdown files.

---

## 5. Requirements

### 5.1 Functional Requirements

```
REQ-037: Merkle Tree Construction from Markdown AST

The system SHALL construct a Merkle tree for each file during
`zetl index` by:
  a) Parsing the file with pulldown-cmark to produce an AST event stream
  b) Grouping events into block-level leaf nodes (headings, paragraphs,
     code blocks, lists, blockquotes, tables, frontmatter, thematic breaks,
     HTML blocks)
  c) Computing a BLAKE3 hash for each leaf from its normalised content
  d) Computing the file's Merkle root from the ordered leaf hashes
  e) Computing the vault root from sorted file hashes

Merkle tree construction SHALL occur during the same scan pass as
wikilink and SPL extraction. It SHALL NOT require a separate command
or user action.

FOR all user roles
WITH no modification to the source files
AND no user-visible Merkle tree commands.

Trace:
- TEST-038
- ADR-008
```

```
REQ-038: SPL Block Dual Hashing

The system SHALL produce a dual-hash SPL leaf node for every `spl`-tagged
fenced code block by:
  a) Computing a content hash: BLAKE3 of normalised raw SPL text
  b) Computing an AST hash: BLAKE3 of the canonically-serialised SPL AST
  c) Computing a combined hash: BLAKE3(content_hash ‖ ast_hash)

If spindle-parser fails to parse the SPL block, the AST hash SHALL be
a sentinel value (all zeros) and a diagnostic SHALL be emitted.

FOR all user roles
WITH dual hashing enabling both textual and semantic change detection.

Trace:
- TEST-039
```

```
REQ-039: Two-Tier Cache Invalidation (Mtime + Content Hash)

The system SHALL implement a two-tier cache invalidation strategy:
  a) Tier 1 (fast): Check file mtime. If unchanged, skip hashing and
     reuse cached Merkle nodes.
  b) Tier 2 (authoritative): If mtime changed, recompute the file's
     Merkle tree. If the file Merkle root equals the cached root,
     skip downstream processing.

This SHALL replace the existing mtime-only invalidation for both the
link index and the theory cache.

FOR all user roles
WITH mtime as a pre-filter and content hash as the authority.

Trace:
- TEST-040
- ADR-009
```

```
REQ-040: SPL-Specific Theory Cache Invalidation

The system SHALL use SPL leaf AST hashes to determine theory cache
validity:
  a) Collect all SPL leaf AST hashes from the current Merkle tree
  b) Compare against the cached set
  c) If identical, skip theory rebuild
  d) If different, rebuild the theory

This means editing prose around an SPL block does NOT trigger a
theory rebuild. Only changes to the logical content of SPL blocks
(new facts, removed rules, changed superiority) invalidate the theory.

FOR all user roles
WITH theory rebuilds occurring only when SPL content actually changed.

Trace:
- TEST-041
- ADR-009
```

```
REQ-041: Implicit Section Grounding

The system SHALL automatically compute a section grounding hash for
each SPL block by:
  a) Identifying the containing section (nearest preceding heading
     through to the next heading at the same or higher level, or EOF)
  b) Computing the section grounding hash from the ordered hashes of
     all non-SPL leaves within the section
  c) Storing the grounding hash in the theory cache alongside the
     SPL block's dual hashes

The section grounding hash SHALL be used for drift detection (REQ-043).

FOR all user roles
WITH grounding computed automatically during indexing
AND no user configuration required.

Trace:
- TEST-042
```

```
REQ-042: Explicit Grounding via :source

The system SHALL support explicit content grounding for individual
SPL facts and rules using the :source metadata key in three forms:
  a) :source "e5f6a7b8" — ground in a specific content block identified
     by its Merkle leaf hash prefix (minimum 8 hex characters). Resolved
     by prefix match against all Merkle leaves in the vault.
  b) :source "^block-id" — ground in a specific ^block-id within the
     same file
  c) :source "[[Page^block-id]]" — ground in a specific ^block-id in
     another file

The system SHALL:
  - Resolve hash references by prefix match across all vault leaves
  - Resolve ^block-id references to specific Merkle leaves
  - Report an error if a hash prefix matches zero leaves or is ambiguous
  - Report an error if a ^block-id or page does not exist
  - Compute a grounding hash from the referenced leaf's content hash
  - Store explicit groundings in the theory cache

Hash-based references (:source "e5f6a7b8") are position-independent:
the same content at a different line or file still resolves. This is
the primary mechanism for agents, who discover hashes via `zetl blocks`.

When explicit :source is present, it takes precedence over implicit
section grounding for that specific fact or rule (see §3.4).

FOR all user roles
WITH validation of :source targets alongside dead link detection.

Trace:
- TEST-043
- CON-019
```

```
REQ-043: Drift Detection in Check

The system SHALL detect and report SPL drift as part of `zetl check`:
  a) For each SPL block with section grounding: compare the current
     section grounding hash against the cached version from the last
     theory build. If different (prose changed) and the SPL AST hash
     is unchanged, report drift.
  b) For each SPL fact/rule with explicit :source grounding: compare
     the current target leaf hash against the cached version. If
     different and the SPL construct is unchanged, report drift.

Drift diagnostics SHALL include:
  - File path and SPL block line number
  - Section heading (for section-grounded drift)
  - Target reference (for explicitly-grounded drift)
  - Severity: "warning" for adjacent changes, "info" for distant changes
  - Human-readable message describing what changed

Drift diagnostics SHALL appear in the existing `zetl check` output
alongside dead links, orphans, and syntax errors.

A `--drift` flag SHALL filter check output to drift diagnostics only.
The existing `--fail-on` flag SHALL apply to drift diagnostics.

FOR all user roles
WITH drift detection integrated into the existing check workflow.

Trace:
- TEST-044
- CON-019
```

```
REQ-044: Durable Provenance with Content Hashes

The system SHALL extend theory provenance metadata to include:
  a) Each provenanced rule and fact: the SPL leaf's content_hash,
     ast_hash, and section grounding hash
  b) Each provenanced conclusion: the vault root hash at reasoning time
  c) `zetl reason provenance` SHALL display a "stale" warning when
     the stored grounding hash no longer matches the current hash

FOR all user roles
WITH content hashes stored in the theory cache and surfaced via
existing provenance commands.

Trace:
- TEST-045
```

```
REQ-045: Content Block Discovery and Hash Resolution

The system SHALL provide a `zetl blocks` command with two modes:

**Forward mode (file → blocks):**

  `zetl blocks <page>` returns the Merkle leaf nodes for a given file,
  including:
    a) Leaf type (heading, paragraph, code block, SPL block, table, etc.)
    b) Line range (start and end line numbers)
    c) Merkle leaf hash (hex-encoded BLAKE3, usable as a :source reference)
    d) Text preview (first 200 characters of normalised content)

  The page argument SHALL use the same resolution as wikilinks (SPEC-001
  §3.2): case-insensitive, normalised matching.

**Reverse mode (hash → file:line):**

  `zetl blocks --resolve <hash>` resolves a Merkle leaf hash prefix to
  its source location, returning:
    a) File path (relative to vault root)
    b) Page name
    c) Line range (start and end line numbers)
    d) Leaf type
    e) Text preview (first 200 characters of normalised content)
    f) Full hash (hex-encoded BLAKE3)

  The hash argument is a hex prefix (minimum 8 characters). Resolution
  uses the same prefix-matching logic as :source hash references
  (REQ-042):
    - Zero matches → error: "content hash not found"
    - One match → success: return the leaf's location
    - Multiple matches → error: "ambiguous hash prefix" with list of
      matching locations; suggest a longer prefix

Both modes SHALL require that `zetl index` has been run (Merkle tree
exists in cache). If the cache is stale or missing, the command SHALL
index first (consistent with other query commands).

FOR agent and human user roles
WITH output in JSON (default) or table format
AND hashes usable directly as :source values in SPL.

Trace:
- TEST-049
- CON-020
```

### 5.2 Non-Functional Requirements

```
NFR-014: Merkle Tree Construction Performance

Merkle tree construction SHALL add ≤ 20% overhead to the existing
scan pass for a vault with ≤ 10,000 files UNDER single-threaded
execution on commodity hardware WITH 95th percentile.

Rationale: BLAKE3 at ~5 GB/s is negligible compared to file I/O
and Markdown parsing. Overhead is from AST node grouping and
memory allocation.

Trace:
- TEST-046
```

```
NFR-015: Merkle Tree Memory Overhead

Peak memory increase from Merkle tree construction SHALL be ≤ 30 MB
above baseline for 10,000 files at ~50 leaves per file.

Rationale: ~41 bytes per leaf × 500,000 leaves ≈ 20 MB. Interior
nodes and overhead account for the remaining 10 MB.

Trace:
- TEST-047
```

```
NFR-016: Merkle Cache Size

Merkle data stored in .zetl/ SHALL add ≤ 5 MB for 10,000 files.

Rationale: Only file roots and SPL leaf hashes are persisted. Full
leaf trees are recomputed on demand.

Trace:
- TEST-048
```

---

## 6. Architecture

### 6.1 Technology Decisions

```
ADR-009: Two-Tier Cache Invalidation — Mtime + Content Hash

Status: Proposed

Context:
  The existing cache uses mtime-only invalidation (SPEC-001 REQ-011,
  SPEC-005 NFR-011). This has two weaknesses:

  1. False positives: mtime changes when a file is touched but not
     modified (touch, backup restoration, git checkout). Triggers
     unnecessary reparsing and theory rebuilds.

  2. Inability to distinguish content types: when a file containing
     both prose and SPL is edited, mtime cannot tell whether the SPL
     changed. Theory cache is invalidated even for prose-only edits.

Decision:
  Keep mtime as tier 1 pre-filter, add content hash as tier 2
  authority. SPL AST hashes determine theory cache validity
  independent of prose changes.

Rationale:
  - Fast path (mtime unchanged) adds zero overhead
  - Accurate path (mtime changed, rehash) only applies to modified
    files — typically a small fraction of the vault
  - SPL AST hash comparison means theory rebuilds are triggered only
    by logical changes to SPL, not prose edits or SPL reformatting

Consequences:
  + Eliminates false theory rebuilds from prose-only edits
  + Eliminates false rebuilds from file touches without content changes
  + Enables drift detection as a natural byproduct
  - Cache size increases (~96 bytes per SPL block)
  - First run after format upgrade triggers a full rehash
```

```
ADR-010: Section Grounding with Explicit Override

Status: Proposed

Context:
  SPL blocks formalise claims made in surrounding prose. The system
  needs a mechanism to link SPL to the prose it's based on, so that
  changes to the prose can trigger drift warnings.

  Options:
  A. No grounding — detect drift based on any change in the same file:
     + Simplest implementation
     - Too noisy: editing a different section triggers false drift

  B. Section grounding only — SPL grounded in containing section:
     + Automatic, zero ceremony
     + Sections are the natural semantic unit in a Zettelkasten
     - Cannot ground in specific paragraphs
     - Cannot ground across files

  C. Explicit grounding only — require :source on every fact:
     + Precise control
     - Too much ceremony for the common case
     - Most users won't annotate every fact

  D. Section grounding by default + explicit :source override:
     + Automatic for 80% of cases
     + Precise when needed (20%)
     + Cross-file grounding via [[Page^block-id]]
     + Zero ceremony for basic use, progressive disclosure
     - Slightly more complex grounding resolution logic

Decision:
  Implement Option D — implicit section grounding with explicit
  :source override.

Rationale:
  - Section grounding handles the common case where an SPL block
    formalises the prose in its immediate context
  - :source handles precision grounding and cross-file references
  - The ^block-id syntax already exists in the Obsidian ecosystem
  - Progressive disclosure: beginners never need :source; experts
    use it when precision matters

Consequences:
  + Zero-ceremony drift detection for all SPL blocks
  + Precise grounding available when needed
  + Cross-file grounding for multi-document theories
  - Section boundary detection adds logic to the scanner
  - :source validation adds checks to zetl check
```

### 6.2 Component Architecture

```
                     ┌──────────────────────┐
                     │        CLI           │
                     │  (existing commands) │
                     └──────────┬───────────┘
                                │
      ┌─────────────────────────┼──────────────────────────┐
      │                         │                           │
┌─────▼──────┐          ┌──────▼──────┐            ┌──────▼───────────┐
│  Scanner    │          │   Graph    │            │    Reason        │
│  (extended) │          │   Engine   │            │    Engine        │
│             │          │            │            │    (extended)    │
│ - file walk │          │ - build    │            │ - build theory  │
│ - parse md  │          │ - query    │            │ - reason        │
│ - wikilinks │          │            │            │ - provenance    │
│ - spl blocks│          └────────────┘            │   + hashes      │
│ - merkle    │                                    │   + grounding   │
│   leaves    │  ←── BLAKE3 hashing ──┐            │   + staleness   │
│ - sections  │                       │            └────────┬────────┘
└─────┬──────┘                        │                     │
      │                               │                     │
      │    ┌──────────────────────────▼─────────────────────┘
      │    │
      │    │   Merkle tree is internal to the scanner and cache.
      │    │   No separate "Merkle Engine" module — hashing is a
      │    │   step within the scanner, and grounding comparison
      │    │   is a step within cache validation and zetl check.
      │    │
      └────┴──────────┐
                      │
               ┌──────▼───────┐
               │    Cache     │
               │  .zetl/      │
               │  index.json  │  + file Merkle roots
               │  theory.json │  + SPL hashes + grounding hashes
               └──────────────┘
```

The Merkle tree is **not** a separate component. It is:
- **Computed** as part of the scanner's existing parse pass
- **Stored** as additional fields in the existing cache files
- **Compared** during cache validation (already in the pipeline)
- **Reported** via existing `zetl check` diagnostics

### 6.3 Data Model

```rust
/// Hash type alias for BLAKE3 output
type ContentHash = [u8; 32];

/// A leaf node in the file-level Merkle tree
struct MerkleLeaf {
    node_type: LeafType,
    start_line: u32,
    end_line: u32,
    hash: ContentHash,
    spl_hashes: Option<SplLeafHash>,
}

enum LeafType {
    Heading { level: u8 },
    Paragraph,
    CodeBlock { language: Option<String> },
    SplBlock,
    List { ordered: bool },
    BlockQuote,
    Table,
    Frontmatter,
    ThematicBreak,
    HtmlBlock,
}

/// Dual hash for SPL block leaves
struct SplLeafHash {
    content_hash: ContentHash,
    ast_hash: ContentHash,
}

/// A section within a file (for grounding)
struct Section {
    heading_line: u32,       // 0 if before first heading
    heading_text: String,    // "" if before first heading
    heading_level: u8,       // 0 if before first heading
    leaf_range: (usize, usize),  // inclusive range into file's leaf vec
    grounding_hash: ContentHash, // hash of non-SPL leaves in section
}

/// File-level Merkle data (stored in cache)
struct FileMerkle {
    root_hash: ContentHash,
    sections: Vec<Section>,
    spl_leaves: Vec<SplLeafCached>,
}

/// Cached SPL leaf data
struct SplLeafCached {
    start_line: u32,
    content_hash: ContentHash,
    ast_hash: ContentHash,
    section_index: usize,  // which section this SPL block belongs to
    explicit_groundings: Vec<ExplicitGrounding>,
}

/// An explicit :source grounding reference
struct ExplicitGrounding {
    /// The literal or rule label this grounding applies to
    construct: String,
    /// The :source reference (e.g., "^block-id" or "[[Page^block-id]]")
    source_ref: String,
    /// Resolved target: file path + leaf index
    target_file: PathBuf,
    target_leaf_hash: ContentHash,
}

/// Drift diagnostic
struct DriftDiagnostic {
    file: PathBuf,
    spl_line: u32,
    drift_type: DriftType,
    severity: DriftSeverity,
    message: String,
}

enum DriftType {
    /// Section prose changed, SPL unchanged
    SectionDrift {
        section_heading: String,
    },
    /// Explicit :source target changed, SPL construct unchanged
    ExplicitDrift {
        construct: String,
        source_ref: String,
    },
    /// Explicit :source target not found
    BrokenGrounding {
        construct: String,
        source_ref: String,
    },
}

enum DriftSeverity {
    Info,     // distant changes in section
    Warning,  // adjacent changes or explicit grounding broken
    Error,    // :source target not found
}
```

### 6.4 Construction Pipeline

The Merkle tree is built as an additional step within the scanner's existing file processing:

```
Existing scanner pipeline:
  file content → pulldown-cmark → extract_wikilinks()
                                → extract_spl_blocks()
                                → diagnostics

Extended pipeline:
  file content → pulldown-cmark → extract_wikilinks()
                                → extract_spl_blocks()
                                → build_merkle_leaves()     NEW
                                → detect_sections()         NEW
                                → compute_grounding_hashes() NEW
                                → diagnostics
```

All three extractors share the same pulldown-cmark event stream — there is no second parse pass.

### 6.5 Drift Detection Algorithm

Drift detection runs as part of `zetl check`, after the scanner has produced current Merkle trees:

```
Input:
  - cached: theory cache with grounding hashes from last build
  - current: freshly-computed Merkle trees from scanner

For each SPL block in current trees:
  1. SECTION DRIFT:
     a. Find the block's section grounding hash in current tree
     b. Find the same block's cached section grounding hash
     c. If section_grounding_hash changed AND spl ast_hash unchanged:
        → Drift detected
     d. Severity: check if immediately-adjacent leaves changed (Warning)
        or only distant leaves (Info)

  2. EXPLICIT DRIFT (for blocks with :source):
     a. Resolve each :source target to a current Merkle leaf
     b. Compare target leaf hash against cached target leaf hash
     c. If target changed AND the SPL construct is unchanged:
        → Explicit drift detected (severity: Warning)
     d. If target not found:
        → Broken grounding (severity: Error)

Output: Vec<DriftDiagnostic> (merged into zetl check results)
```

---

## 7. Contract Specifications (CLI Interface)

The Merkle tree does not introduce new subcommands. It extends existing contracts.

```
CON-019: zetl check (extended with --drift)

zetl check [OPTIONS]

Additional options:
  --drift          Show only drift diagnostics (SPL blocks with
                   changed grounding)

Drift diagnostics are included in the existing check output format
alongside dead links, orphans, syntax errors, and SPL diagnostics.
The existing --fail-on flag applies to drift diagnostics.

Example output (JSON, drift diagnostics):
{
  "dead_links": [...],
  "orphans": [...],
  "syntax_errors": [...],
  "spl_diagnostics": [...],
  "drift_diagnostics": [
    {
      "level": "warning",
      "file": "decisions/Redis vs Memcached.md",
      "spl_line": 10,
      "type": "section_drift",
      "section": "## Benchmark Results",
      "message": "SPL block at line 10 may be stale. Section '## Benchmark Results' was modified since the theory was built, but the SPL content is unchanged."
    },
    {
      "level": "warning",
      "file": "decisions/Architecture.md",
      "spl_line": 22,
      "type": "explicit_drift",
      "construct": "performance-acceptable",
      "source_ref": "[[Benchmarks^perf-numbers]]",
      "message": "Fact 'performance-acceptable' grounded in [[Benchmarks]]^perf-numbers — target content changed since the theory was built."
    },
    {
      "level": "error",
      "file": "decisions/Old Decision.md",
      "spl_line": 15,
      "type": "broken_grounding",
      "construct": "legacy-compatible",
      "source_ref": "^removed-section",
      "message": "Fact 'legacy-compatible' references ^removed-section which no longer exists."
    }
  ],
  "summary": {
    "dead_links": 0,
    "orphans": 0,
    "syntax_errors": 0,
    "spl_errors": 0,
    "drift_warnings": 2,
    "drift_errors": 1
  }
}

Implements:
- REQ-043

Verified by:
- TEST-044
```

```
CON-020: zetl blocks

zetl blocks [PAGE] [OPTIONS]

List the content blocks of a file with their Merkle leaf hashes,
or resolve a hash to its source location.

Arguments:
  [PAGE]  Page name (case-insensitive, same resolution as wikilinks).
          Required unless --resolve is used.

Options:
  --type <TYPE>      Filter by leaf type: heading, paragraph, spl, code,
                     table, list, blockquote, frontmatter [default: all]
  --resolve <HASH>   Resolve a Merkle hash prefix to its source location.
                     Minimum 8 hex characters. Mutually exclusive with PAGE.

Exit codes:
  0  Blocks listed / hash resolved
  1  Page not found / hash not found / ambiguous hash prefix

--- Forward mode: zetl blocks <PAGE> ---

Example output (JSON):
{
  "page": "Redis vs Memcached",
  "file": "decisions/Redis vs Memcached.md",
  "blocks": [
    {
      "index": 0,
      "type": "Frontmatter",
      "lines": [1, 3],
      "hash": "1a2b3c4d5e6f7a8b",
      "text": "title: Redis vs Memcached\ndate: 2026-01-15"
    },
    {
      "index": 1,
      "type": "Heading",
      "level": 2,
      "lines": [5, 5],
      "hash": "a1b2c3d4e5f6a7b8",
      "text": "## Benchmark Results"
    },
    {
      "index": 2,
      "type": "Paragraph",
      "lines": [7, 9],
      "hash": "e5f6a7b8c9d0e1f2",
      "text": "We benchmarked Redis at 120k ops/sec under production workload. The test ran for 24 hours with..."
    },
    {
      "index": 3,
      "type": "Table",
      "lines": [11, 14],
      "hash": "c9d0e1f2a3b4c5d6",
      "text": "| Metric | Value |\n| ops/sec | 120,000 |\n| p99 latency | 2.1ms |"
    },
    {
      "index": 4,
      "type": "SplBlock",
      "lines": [16, 21],
      "hash": "3a4b5c6d7e8f9a0b",
      "spl_hashes": {
        "content_hash": "4b5c6d7e8f9a0b1c",
        "ast_hash": "5c6d7e8f9a0b1c2d"
      },
      "text": "(given redis-benchmarked)\n(given redis-fast-enough)\n(normally r-prefer-redis ...)"
    }
  ],
  "file_hash": "f2a3b4c5d6e7f8a9",
  "block_count": 5
}

Example output (table):

  decisions/Redis vs Memcached.md (5 blocks, hash: f2a3b4c5)

  #  Type        Lines   Hash      Preview
  0  Frontmatter  1-3    1a2b3c4d  title: Redis vs Memcached...
  1  Heading(2)   5      a1b2c3d4  ## Benchmark Results
  2  Paragraph    7-9    e5f6a7b8  We benchmarked Redis at 120k ops/sec...
  3  Table        11-14  c9d0e1f2  | Metric | Value | ...
  4  SplBlock     16-21  3a4b5c6d  (given redis-benchmarked) ...

--- Reverse mode: zetl blocks --resolve <HASH> ---

Example output (JSON):
{
  "hash": "e5f6a7b8c9d0e1f2",
  "file": "decisions/Redis vs Memcached.md",
  "page": "Redis vs Memcached",
  "type": "Paragraph",
  "lines": [7, 9],
  "text": "We benchmarked Redis at 120k ops/sec under production workload. The test ran for 24 hours with..."
}

Example output (table):

  e5f6a7b8c9d0e1f2  decisions/Redis vs Memcached.md:7-9  Paragraph
  We benchmarked Redis at 120k ops/sec under production workload...

Example error — hash not found (JSON):
{
  "error": "content hash e5f6a7b8 not found — source content may have been modified or removed"
}

Example error — ambiguous prefix (JSON):
{
  "error": "ambiguous hash prefix e5f6a7b8",
  "matches": [
    {"file": "decisions/Redis.md", "lines": [7, 9], "hash": "e5f6a7b8c9d0e1f2"},
    {"file": "notes/Cache.md", "lines": [12, 14], "hash": "e5f6a7b8aabbccdd"}
  ],
  "suggestion": "use a longer prefix to disambiguate"
}

Usage:
  The hash values can be used directly as :source references in SPL:
    (given redis-fast-enough :source "e5f6a7b8")

  Resolve a hash back to its source location:
    zetl blocks --resolve e5f6a7b8

Implements:
- REQ-045

Verified by:
- TEST-049
```

```
CON-004 (extended): zetl check :source validation

Broken :source references (^block-id that doesn't exist, [[Page]] that
doesn't exist) are reported as errors in the spl_diagnostics section,
consistent with dead wikilink detection:

{
  "spl_diagnostics": [
    {
      "level": "error",
      "file": "decisions/Old Decision.md",
      "line": 16,
      "message": "SPL :source references ^removed-section which does not exist in this file"
    },
    {
      "level": "error",
      "file": "decisions/Architecture.md",
      "line": 23,
      "message": "SPL :source references [[Nonexistent Page^data]] — page 'Nonexistent Page' not found"
    }
  ]
}

Implements:
- REQ-042

Verified by:
- TEST-043
```

```
CON-006 (extended): zetl stats — vault root hash

`zetl stats` output is extended with vault content integrity data:

{
  "pages": 47,
  "links": 312,
  ...existing fields...
  "vault_content_hash": "a1b2c3d4...",
  "spl_blocks": 23,
  "grounded_spl_blocks": 23,
  "explicitly_grounded_facts": 5
}

Implements:
- REQ-037 (vault root hash exposure)

Verified by:
- TEST-038
```

```
CON-012 (extended): zetl reason provenance — staleness warnings

`zetl reason provenance` output is extended with grounding freshness:

{
  "literal": "decided-use-redis",
  "sources": [
    {
      "page": "Redis vs Memcached",
      "path": "decisions/Redis vs Memcached.md",
      "line": 10,
      "rule_label": "r-prefer-redis",
      "contribution": "defeasible_rule",
      "grounding": {
        "type": "section",
        "section": "## Benchmark Results",
        "fresh": false,
        "warning": "Section prose changed since theory was built"
      }
    },
    {
      "page": "Redis vs Memcached",
      "path": "decisions/Redis vs Memcached.md",
      "line": 11,
      "rule_label": null,
      "contribution": "fact",
      "grounding": {
        "type": "explicit",
        "source": "^benchmark-results",
        "fresh": true
      }
    }
  ],
  "vault_root_hash": "a1b2c3d4...",
  "theory_built_at": "2026-02-24T10:30:00Z"
}

Implements:
- REQ-044

Verified by:
- TEST-045
```

---

## 8. Test Specifications

```
TEST-038: Merkle Tree Construction During Index

Scenario: Index builds Merkle tree transparently
Given: A vault with 5 Markdown files
When: `zetl index` is run
Then:
  - Each file has a Merkle root hash in the cache
  - A vault root hash is stored
  - No separate "merkle" command was needed

Scenario: Vault root is deterministic
Given: A vault with files scanned in different orders
When: `zetl index` is run twice
Then: The vault root hash is identical both times

Scenario: Normalisation makes formatting changes invisible
Given: Two files with identical text but different whitespace
When: Both are indexed
Then: Their Merkle roots are identical

Verifies: REQ-037
```

```
TEST-039: SPL Block Dual Hashing

Scenario: Content hash and AST hash computed
Given: A file with an SPL block containing "(given bird) (normally r1 bird flies)"
When: The file is indexed
Then:
  - The SPL leaf has a content_hash and ast_hash
  - Both are non-zero BLAKE3 hashes

Scenario: Reformatting changes content_hash but not ast_hash
Given: Two files with logically identical SPL but different formatting
When: Both are indexed
Then:
  - ast_hash is identical
  - content_hash may differ (comment and whitespace differences)

Scenario: Parse error produces sentinel AST hash
Given: An SPL block with "(given unclosed"
When: The file is indexed
Then:
  - content_hash is computed
  - ast_hash is all zeros
  - A diagnostic is emitted

Verifies: REQ-038
```

```
TEST-040: Two-Tier Cache Invalidation

Scenario: Mtime unchanged → skip hashing
Given: A cached vault with no file modifications
When: `zetl index` is run
Then: No BLAKE3 hashing occurs; cache is reused

Scenario: Mtime changed, content unchanged → skip reprocessing
Given: A file is `touch`ed but content is identical
When: `zetl index` is run
Then:
  - File is re-read and hashed
  - Hash matches cached → no downstream reprocessing

Scenario: Mtime changed, content changed → reprocess
Given: A file's content is modified
When: `zetl index` is run
Then: Full reprocessing occurs for that file

Verifies: REQ-039
```

```
TEST-041: SPL-Specific Theory Cache Invalidation

Scenario: Prose edit does NOT trigger theory rebuild
Given: A file with prose and SPL; theory is cached
When: Only prose is edited (SPL unchanged)
Then:
  - File Merkle root changes
  - SPL AST hash unchanged
  - Theory cache valid — no rebuild

Scenario: SPL reformatting does NOT trigger theory rebuild
Given: SPL block reformatted but logically unchanged
When: `zetl reason status` is run
Then: Theory cache valid — no rebuild

Scenario: SPL logical change triggers theory rebuild
Given: A new fact added to SPL block
When: `zetl reason status` is run
Then:
  - SPL AST hash changed
  - Theory rebuilt with new conclusions

Verifies: REQ-040
```

```
TEST-042: Implicit Section Grounding

Scenario: Section grounding hash computed
Given: A file with:
  ## Section A
  Paragraph about X.
  ```spl
  (given x)
  ```
  ## Section B
  Paragraph about Y.
When: The file is indexed
Then:
  - The SPL block is grounded in Section A
  - The grounding hash is computed from the Heading "## Section A"
    and the Paragraph "Paragraph about X" — NOT the SPL block itself
  - Section B content does not affect the grounding hash

Scenario: Section with no heading
Given: SPL block before first heading
When: The file is indexed
Then:
  - Grounding context is all content from file start to first heading

Verifies: REQ-041
```

```
TEST-043: Explicit Grounding via :source

Scenario: Same-file :source grounding
Given: A file with:
  We tested Redis at 120k ops/sec. ^benchmark-results
  ```spl
  (given redis-fast-enough :source "^benchmark-results")
  ```
When: The file is indexed
Then:
  - Fact redis-fast-enough has explicit grounding to ^benchmark-results
  - The grounding hash is the Merkle leaf hash of that paragraph

Scenario: Cross-file :source grounding
Given:
  File A: "Architecture.md" with paragraph tagged ^perf-numbers
  File B: SPL with (given ok :source "[[Architecture^perf-numbers]]")
When: Both files are indexed
Then:
  - Fact ok has explicit grounding to Architecture.md ^perf-numbers
  - Grounding hash is the target paragraph's Merkle leaf hash

Scenario: Broken :source detected by check
Given: SPL with :source "^nonexistent"
When: `zetl check` is run
Then:
  - Reports error: ":source references ^nonexistent which does not exist"

Scenario: Broken cross-file :source
Given: SPL with :source "[[Ghost Page^data]]"
When: `zetl check` is run
Then:
  - Reports error: "page 'Ghost Page' not found"

Verifies: REQ-042
```

```
TEST-044: Drift Detection in Check

Scenario: Section drift detected
Given:
  - File has: ## Results, Paragraph-A, SplBlock, Paragraph-B
  - Theory was built with these hashes cached
  - Paragraph-A is edited (SPL unchanged)
When: `zetl check` is run
Then:
  - Reports drift warning for the SPL block
  - Message references section "## Results"

Scenario: Explicit grounding drift detected
Given:
  - Fact grounded in ^benchmark-results via :source
  - The ^benchmark-results paragraph is edited
  - The SPL fact is unchanged
When: `zetl check --drift` is run
Then:
  - Reports drift warning naming the fact and :source reference

Scenario: SPL block itself changed — no drift
Given: Both prose and SPL are edited
When: `zetl check --drift` is run
Then: No drift reported (SPL was updated)

Scenario: No changes — no drift
Given: No modifications since theory build
When: `zetl check --drift` is run
Then: Zero drift diagnostics

Scenario: --fail-on applies to drift
Given: Drift warning exists
When: `zetl check --drift --fail-on warning` is run
Then: Exit code 1

Verifies: REQ-043
```

```
TEST-045: Durable Provenance with Staleness

Scenario: Provenance includes grounding freshness
Given: A theory built from vault with grounding hashes
When: `zetl reason provenance "literal"` is run
Then:
  - Sources include grounding type (section or explicit)
  - Sources include fresh: true/false

Scenario: Stale provenance warning
Given: A conclusion's source section was edited after the theory was built
When: `zetl reason provenance "literal"` is run
Then:
  - The source shows fresh: false
  - A warning message explains what changed

Verifies: REQ-044
```

```
TEST-049: Content Block Discovery and Hash Resolution

--- Forward mode ---

Scenario: List blocks for a file
Given: An indexed vault with file "Redis.md" containing a heading,
       two paragraphs, a table, and an SPL block
When: `zetl blocks "Redis"` is run
Then:
  - Returns 5+ blocks in document order
  - Each block has type, lines, hash, and text preview
  - SPL blocks include spl_hashes
  - Output matches CON-020 forward mode schema

Scenario: Hash is usable as :source
Given: `zetl blocks "Redis"` returns hash "e5f6a7b8" for a paragraph
When: An SPL block is written with (given fact :source "e5f6a7b8")
       and `zetl check` is run
Then:
  - The :source resolves successfully (no error)
  - The grounding hash matches the target paragraph

Scenario: Hash becomes stale after edit
Given: An SPL fact grounded in hash "e5f6a7b8"
When: The target paragraph is edited and `zetl check --drift` is run
Then:
  - Hash no longer matches any leaf
  - Broken grounding error is reported

Scenario: Position-independent resolution
Given: A paragraph with hash "e5f6a7b8" is moved from line 7 to line 20
       (content unchanged, only position changed)
When: `zetl check` is run
Then:
  - Hash still resolves (same content, same hash)
  - No drift or error reported

Scenario: Page not found
When: `zetl blocks "Nonexistent"` is run
Then: Exit code 1, page not found error

Scenario: Filter by type
When: `zetl blocks "Redis" --type paragraph` is run
Then: Only paragraph blocks are returned

--- Reverse mode ---

Scenario: Resolve hash to source location
Given: An indexed vault where "Redis.md" line 7-9 has a paragraph
       with Merkle hash e5f6a7b8c9d0e1f2
When: `zetl blocks --resolve e5f6a7b8` is run
Then:
  - Returns file "decisions/Redis vs Memcached.md"
  - Returns page "Redis vs Memcached"
  - Returns lines [7, 9]
  - Returns type "Paragraph"
  - Returns full hash "e5f6a7b8c9d0e1f2"
  - Returns text preview
  - Exit code 0

Scenario: Resolve hash not found
Given: No leaf in the vault matches prefix "deadbeef"
When: `zetl blocks --resolve deadbeef` is run
Then:
  - Error: "content hash deadbeef not found"
  - Exit code 1

Scenario: Resolve ambiguous hash prefix
Given: Two leaves in different files share the prefix "e5f6a7b8"
When: `zetl blocks --resolve e5f6a7b8` is run
Then:
  - Error: "ambiguous hash prefix e5f6a7b8"
  - Lists all matching locations with full hashes
  - Suggests using a longer prefix
  - Exit code 1

Scenario: Resolve with full hash
Given: A leaf with hash "e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0..."
When: `zetl blocks --resolve e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0` is run
Then:
  - Resolves unambiguously
  - Exit code 0

Scenario: Resolve hash too short
When: `zetl blocks --resolve e5f6` is run
Then:
  - Error: "hash prefix too short (minimum 8 hex characters)"
  - Exit code 1

Scenario: Roundtrip — forward then reverse
Given: `zetl blocks "Redis"` returns a block with hash "e5f6a7b8"
When: `zetl blocks --resolve e5f6a7b8` is run
Then:
  - Returns the same file, lines, type, and text as the forward mode

Verifies: REQ-045
```

```
TEST-046: Construction Performance

Scenario: Overhead within bounds
Given: A vault with ≥ 1,000 files
When: Scanning with Merkle construction vs without
Then: Total time ≤ 1.2× baseline

Verifies: NFR-014
```

```
TEST-047: Memory Overhead

Scenario: Memory within bounds
Given: 10,000 files, ~50 leaves per file
When: Merkle tree constructed
Then: Peak memory increase ≤ 30 MB

Verifies: NFR-015
```

```
TEST-048: Cache Size

Scenario: Cache within bounds
Given: 10,000 files with Merkle data
When: Cache written
Then: Additional cache data ≤ 5 MB

Verifies: NFR-016
```

---

## 9. Observability

```
OBS-007: Merkle Construction Timing

When --verbose is specified, `zetl index` SHALL emit to stderr:
  - Files hashed / files skipped (mtime unchanged) / files with
    content-hash match (touched but unchanged)
  - Total leaf nodes computed, SPL leaves with dual hashing
  - BLAKE3 hashing time (ms)
  - Section detection and grounding hash time (ms)
```

```
OBS-008: Drift Detection Metrics

`zetl check` SHALL include in its summary:
  - Total SPL blocks, drifted blocks (warning + info)
  - Explicitly grounded facts, broken groundings
```

```
OBS-009: Cache Efficiency

When --verbose is specified, `zetl index` and `zetl reason status`
SHALL emit:
  - Tier 1 hits (mtime) / Tier 1 misses
  - Tier 2 hits (hash match) / Tier 2 misses (actual change)
  - Theory cache hit/miss (SPL AST hash comparison)
```

---

## 10. Traceability Matrix

| REQ     | CON              | TEST     | ADR      | OBS     |
| ------- | ---------------- | -------- | -------- | ------- |
| REQ-037 | CON-006 (ext)    | TEST-038 | ADR-008  | OBS-007 |
| REQ-038 | —                | TEST-039 | ADR-008  | OBS-007 |
| REQ-039 | —                | TEST-040 | ADR-009  | OBS-009 |
| REQ-040 | —                | TEST-041 | ADR-009  | OBS-009 |
| REQ-041 | —                | TEST-042 | ADR-010  | —       |
| REQ-042 | CON-004 (ext)    | TEST-043 | ADR-010  | —       |
| REQ-043 | CON-019          | TEST-044 | —        | OBS-008 |
| REQ-044 | CON-012 (ext)    | TEST-045 | —        | —       |
| REQ-045 | CON-020          | TEST-049 | —        | —       |
| NFR-014 | —                | TEST-046 | ADR-008  | OBS-007 |
| NFR-015 | —                | TEST-047 | —        | —       |
| NFR-016 | —                | TEST-048 | —        | —       |

---

## 11. Implementation Priority

### P0 — Core Merkle Infrastructure

| Item | Effort | Dependencies |
| --- | --- | --- |
| Leaf node grouper in scanner (REQ-037) | 4 hours | Existing scanner |
| BLAKE3 leaf hashing + file roots (REQ-037) | 2 hours | blake3 crate |
| Vault root hash (REQ-037) | 1 hour | File roots |
| Two-tier cache invalidation (REQ-039) | 4 hours | File roots |
| SPL dual hashing (REQ-038) | 3 hours | spindle-parser |
| SPL-specific theory invalidation (REQ-040) | 2 hours | SPL dual hashing |

### P1 — Section Grounding and Drift

| Item | Effort | Dependencies |
| --- | --- | --- |
| Section boundary detection (REQ-041) | 2 hours | Leaf grouper |
| Section grounding hash computation (REQ-041) | 2 hours | Section detection |
| Drift detection in check (REQ-043) | 4 hours | Section grounding |
| Durable provenance hashes (REQ-044) | 2 hours | SPL dual hashing |

### P2 — Explicit Grounding and Content Discovery

| Item | Effort | Dependencies |
| --- | --- | --- |
| `zetl blocks` command (REQ-045) | 2 hours | P0 complete |
| :source parsing from SPL metadata (REQ-042) | 3 hours | spindle-parser meta |
| Merkle hash prefix resolution (REQ-042) | 3 hours | P0 complete |
| ^block-id resolution to Merkle leaves (REQ-042) | 2 hours | Existing scanner |
| Cross-file :source resolution (REQ-042) | 2 hours | Block-id resolution |
| :source validation in check (REQ-042) | 2 hours | Resolution |
| Explicit grounding drift detection (REQ-043) | 2 hours | P1 + resolution |

**Estimated total: ~42 hours** across all priorities.

---

## 12. Cache Format

### 12.1 Index Cache Extension

The existing `.zetl/index.json` is extended with per-file Merkle roots:

```json
{
  "version": 2,
  "files": {
    "decisions/Redis.md": {
      "mtime": 1708770000.0,
      "page_name": "Redis",
      "links": [...],
      "spl_blocks": [...],
      "diagnostics": [...],
      "merkle_root": "b3c4d5e6..."
    }
  },
  "vault_root_hash": "a1b2c3d4..."
}
```

### 12.2 Theory Cache Extension

The existing `.zetl/theory.json` is extended with grounding data:

```json
{
  "version": 2,
  "vault_root_hash": "a1b2c3d4...",
  "built_at": "2026-02-24T10:30:00Z",
  "spl_blocks": {
    "decisions/Redis.md:10": {
      "ast_hash": "d5e6f7a8...",
      "content_hash": "c4d5e6f7...",
      "section_heading": "## Benchmark Results",
      "section_grounding_hash": "e7f8a9b0...",
      "explicit_groundings": [
        {
          "construct": "redis-fast-enough",
          "source_ref": "^benchmark-results",
          "target_file": "decisions/Redis.md",
          "target_hash": "f8a9b0c1..."
        }
      ]
    }
  },
  "rules": [...],
  "superiorities": [...],
  "diagnostics": [...]
}
```

No separate `merkle.json` file — Merkle data is folded into the existing caches.

---

## 13. Future Considerations

| Item | Rationale |
| --- | --- |
| Incremental Merkle tree updates | Update only affected leaves on file change, rather than rebuilding the file tree. Requires ordered tree structure. |
| Merkle proofs for provenance | Generate compact proofs that an SPL block was part of a specific vault state. Useful for auditing. |
| Cryptographic signing of vault root | Sign with author key for tamper detection in multi-agent environments. |
| Semantic drift detection | Use embedding similarity (not just hash equality) to detect meaning drift. |
| Cross-vault Merkle forests | Vault roots as leaves in a cross-vault tree. Builds on SPEC-004 sync. |
| Named SPL blocks with grounding | Combine SPEC-005 §12.2 named blocks with explicit grounding: `@{caching-base :source "^section"}`. |
| Grounding visualisation in TUI | Show which prose each SPL block is grounded in, highlighted in the page view. |
| Automatic :source suggestion | When drift is detected, suggest adding explicit :source to prevent false positives. |
| Grounding-aware what-if | `zetl reason what-if` could show which groundings would become stale if a hypothetical were applied. |

---

## 14. Open Questions

1. **Should section grounding use the heading text or heading hash as the section identifier?** Heading text is human-readable in drift messages. Heading hash is robust to position changes. Recommendation: use heading text for display, heading hash + position for matching.

2. **How should the system handle files with no headings?** The entire file is one implicit section. All SPL blocks are grounded in the full file content. Recommendation: this is fine for small files. For large files with no headings, consider a warning.

3. **Should `:source` be a spindle-core `(meta ...)` construct or inline syntax?** Inline (`:source "e5f6a7b8"` on the fact/rule line) is more readable but requires parser support. Meta (`(meta label :source "e5f6a7b8")`) works with the existing parser. Recommendation: support both; inline is sugar for meta.

7. **What is the minimum hash prefix length for unambiguous resolution?** 8 hex characters (32 bits) provides 4 billion distinct values, which is sufficient for typical vaults. For very large vaults, longer prefixes may be needed. Recommendation: minimum 8 characters, `zetl check` reports ambiguity if a prefix matches multiple leaves and suggests a longer prefix.

4. **Should drift detection have a "grace period" for new files?** A newly-created file has no baseline — everything is "new." Should the first check after file creation report drift? Recommendation: no. Drift requires a baseline from a previous theory build. New files have no baseline and are not flagged.

5. **What happens when a section is split (a new heading inserted in the middle)?** The SPL block's section shrinks. The grounding hash changes because the set of leaves in the section changed. This correctly triggers drift detection. Recommendation: this is the right behaviour — restructuring a section is a meaningful change.

6. **Should the vault root hash be exposed in `zetl stats` or only internally?** Exposing it in stats gives agents a coordination checkpoint. Recommendation: include it in `zetl stats` output as `vault_content_hash`.

---

**END OF SPEC-006**
