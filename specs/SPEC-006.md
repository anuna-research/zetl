---
title: "SPEC-006: Content-Addressed Merkle Tree over Markdown and SPL AST"
version: 0.1.0
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
| Version        | 0.1.0                                                              |
| Status         | Draft                                                              |
| Author         | Agent (USDD Protocol v1.0.0)                                       |
| Date           | 2026-02-24                                                         |
| Audience       | Agent, Human                                                       |
| Trace          | USDD Agent Protocol v1.0.0                                         |
| Parent         | SPEC-001: zetl — Bi-directional Link Graph CLI                     |
| Related        | SPEC-005: zetl reason — Defeasible Logic over Markdown Vaults      |
| Dependencies   | pulldown-cmark (Markdown AST), spindle-parser (SPL AST), sha2/blake3 |

---

## 1. Overview

SPEC-001 established zetl's cache invalidation on file-level modification timestamps (mtime). SPEC-005 extended this to the theory cache: if any SPL-containing file's mtime changes, the entire theory is rebuilt. This works for performance — but it says nothing about **what changed**. Mtime tells you *when* a file was touched, not *whether the content that matters actually differs*.

This specification introduces a **content-addressed Merkle tree** rooted at the vault level, where every node in the Markdown and SPL abstract syntax trees is content-hashed into a hierarchical structure. The result is a durable, verifiable fingerprint for every piece of content in the vault — from individual paragraphs and SPL facts up to the entire vault as a whole.

### 1.1 The Drift Problem

SPL theories embedded in Markdown files make claims that are grounded in the surrounding prose. A note titled "Redis vs Memcached" might contain:

````markdown
We benchmarked Redis at 120k ops/sec under our workload profile.

```spl
(given redis-benchmarked)
(given redis-fast-enough)
(normally r-prefer-redis
  (and redis-benchmarked redis-fast-enough)
  decided-use-redis)
```
````

The SPL block formalises the note's claim. But what happens when the prose changes?

- **Scenario A:** The author updates the benchmark to 85k ops/sec and adds "below our threshold." The SPL still asserts `redis-fast-enough`. The theory now contradicts its own source document. The SPL has **drifted** from its grounding prose.
- **Scenario B:** The author fixes a typo in a paragraph above the SPL block. The file's mtime changes. The theory cache is invalidated and rebuilt — unnecessarily, since the SPL content is identical. This is a **false invalidation**.
- **Scenario C:** An agent creates a note with SPL, then a different agent modifies the prose months later. The second agent has no way to know whether the SPL is still consistent with the updated prose. There is no **content provenance** linking the theory to a specific version of the document.

Mtime-based caching handles none of these. It is a performance optimisation, not a correctness mechanism.

### 1.2 The Solution: Content-Addressed Merkle Tree

A Merkle tree hashes content from leaves upward: each leaf node is the hash of an atomic content unit; each interior node is the hash of its children's hashes. Any change to any leaf propagates to the root. Comparing roots tells you instantly whether *anything* changed; comparing interior nodes tells you *where*.

Applied to a Markdown vault:

```
                        ┌─────────────────────┐
                        │    Vault Root Hash   │
                        │  H(file₁ ‖ file₂ ‖ …)│
                        └──────────┬──────────┘
                                   │
                  ┌────────────────┼────────────────┐
                  │                │                 │
           ┌──────▼──────┐ ┌──────▼──────┐  ┌──────▼──────┐
           │  File Hash₁  │ │  File Hash₂  │  │  File Hash₃  │
           │  H(node₁‖…)  │ │  H(node₁‖…)  │  │  H(node₁‖…)  │
           └──────┬───────┘ └──────┬───────┘  └─────────────┘
                  │                │
        ┌─────┬──┴──┬─────┐      ┌┴─────┬──────┐
        │     │     │     │      │      │      │
       ┌▼┐  ┌▼┐  ┌─▼─┐ ┌▼┐   ┌─▼─┐  ┌▼┐  ┌─▼─┐
       │H│  │P│  │SPL│ │P│   │ H │  │P│  │SPL│
       │1│  │ │  │   │ │ │   │   │  │ │  │   │
       └─┘  └─┘  └───┘ └─┘   └───┘  └─┘  └───┘

       H = Heading leaf          SPL = SPL block leaf (tagged)
       P = Paragraph leaf
```

**Leaves** are the atomic AST nodes produced by pulldown-cmark (headings, paragraphs, code blocks, lists, etc.) and spindle-parser (facts, rules, defeaters, superiority relations). Each leaf is content-hashed independently.

**SPL block leaves are tagged.** When the scanner encounters an `spl`-tagged code block, it produces a special leaf node that contains both the raw SPL content hash and the parsed SPL AST hash. This dual hashing means: (a) changes to SPL block formatting without semantic change can be detected, and (b) changes to the logical content are tracked separately from the surrounding Markdown.

**File nodes** are interior nodes whose hash is derived from the ordered concatenation of their children's hashes. Reordering paragraphs changes the file hash. Adding a paragraph changes the file hash. Modifying a single character in any paragraph changes the file hash.

**The vault root** is the top-level interior node whose hash is derived from all file hashes, sorted by canonical path. Adding, removing, or modifying any file changes the vault root hash.

### 1.3 What This Enables

| Capability | Mechanism |
| --- | --- |
| **O(1) vault change detection** | Compare vault root hashes |
| **O(log n) change localisation** | Walk the tree to the first divergent node |
| **SPL drift detection** | Compare the SPL leaf hash against the hashes of its sibling prose nodes. If prose changed but SPL didn't, flag as potential drift |
| **Conclusion freshness** | Each conclusion's provenance includes the content hashes of the rules/facts that derived it. Re-verify without re-reasoning by checking hashes |
| **False invalidation elimination** | Mtime changed but content hash is identical → skip rebuild |
| **Durable provenance references** | Theory provenance stores content hashes alongside file paths and line numbers. Even if the file is later modified, the provenance references a specific, verifiable content state |
| **Cross-agent content verification** | Agent B can verify that the prose Agent A based its SPL on has not changed since the theory was built |

### 1.4 Design Philosophy

1. **Content over time.** Mtime answers "when was this touched?" Content hashing answers "what does it say?" Both are useful; content hashing is authoritative.
2. **Mtime as pre-filter.** Hashing is more expensive than stat(). Mtime remains the first check: if mtime hasn't changed, skip hashing. If mtime changed but the hash is the same, skip reprocessing. This is a two-tier invalidation strategy.
3. **Trees, not flat hashes.** A flat per-file hash (SHA-256 of file content) would detect changes but not localise them. The Merkle tree structure enables targeted questions: "did the SPL block change?" "did the heading change?" "which paragraph changed?"
4. **AST boundaries, not byte boundaries.** Hashing raw bytes is fragile — trailing whitespace, BOM markers, and line endings cause false positives. Hashing normalised AST nodes is semantically stable.
5. **Tagged leaves.** SPL blocks are not just code blocks — they are semantically significant content that feeds the reasoning engine. Tagging them in the Merkle tree enables SPL-specific queries (drift detection, conclusion freshness) without scanning the full tree.

### 1.5 Scope

**In scope:**

- Merkle tree construction from pulldown-cmark AST events
- SPL block leaves with dual hashing (raw content + parsed SPL AST)
- File-level and vault-level Merkle roots
- Integration with existing cache system (mtime + content hash two-tier invalidation)
- SPL drift detection: flagging SPL blocks whose sibling prose has changed since the theory was last built
- Durable provenance: attaching content hashes to theory provenance metadata
- CLI commands for inspecting and comparing Merkle trees

**Out of scope:**

- Cryptographic signing of Merkle proofs (future SPEC, builds on spindle-core's trust module)
- Distributed Merkle tree synchronisation across vaults (future SPEC, builds on SPEC-004 sync)
- Incremental Merkle tree updates (future optimisation; v1 rebuilds file trees from scratch)
- Embedding-based semantic drift detection (future SPEC; this spec covers structural drift only)
- Git-style object storage (the Merkle tree is computed in-memory and cached as metadata, not stored as a content-addressable object store)

---

## 2. User Profiles

### 2.1 Agent Operator — Theory Integrity Verifier

```
Role: LLM agent maintaining a knowledge base with SPL theories
Goals:
  - Verify that SPL theories are still consistent with their source prose
  - Detect when prose changes invalidate existing SPL claims
  - Avoid unnecessary theory rebuilds when non-SPL content changes
  - Reference specific content states in provenance metadata
Constraints:
  - Requires structured JSON output
  - Invokes CLI commands non-interactively
  - May operate on the same vault as other agents concurrently
Daily workflow:
  1. Run `zetl merkle status` to get the current vault root hash
  2. Run `zetl merkle drift` to check for SPL blocks whose surrounding
     prose has changed since the theory was last built
  3. If drift detected, run `zetl reason status` to rebuild with fresh hashes
  4. Store vault root hash as a checkpoint for future comparison
```

### 2.2 Human Knowledge Worker — Content Auditor

```
Role: Researcher auditing the evolution of their knowledge base
Goals:
  - Understand what changed between vault states
  - Verify that formal claims (SPL) are still grounded in current prose
  - See which conclusions may be stale due to prose changes
Constraints:
  - Prefers human-readable table output
  - May not understand Merkle trees — needs actionable summaries
  - Works from the terminal alongside a text editor
Daily workflow:
  1. Run `zetl merkle drift -f table` after editing notes
  2. Review flagged SPL blocks and update or confirm them
  3. Run `zetl reason status` to see updated conclusions
```

### 2.3 Agent Team — Coordinated Knowledge Verification

```
Role: Multiple LLM agents contributing to a shared knowledge base (via hence)
Goals:
  - Verify that another agent's edits haven't invalidated existing theories
  - Use content hashes as durable references in task coordination
  - Detect conflicts arising from concurrent edits to shared documents
Constraints:
  - Agents write concurrently (append-only, no lock contention)
  - Vault root hash serves as a coordination checkpoint
  - Hence can compare pre/post hashes to validate agent contributions
Daily workflow:
  1. Hence records vault root hash before assigning task to agent-A
  2. Agent-A edits files and runs `zetl merkle status`
  3. Hence compares pre/post vault root hashes
  4. If changed, hence runs `zetl merkle diff <hash-before> <hash-after>`
     to understand what changed
  5. Hence runs `zetl merkle drift` to verify no SPL drift
```

### 2.4 Happy Paths

```
Happy Path: Agent Detects SPL Drift After Prose Edit

Preconditions:
  - Vault has a file "Redis.md" with prose and an SPL block
  - Theory was built with content hashes recorded in provenance
  - Another agent modifies the benchmark numbers in the prose
Steps:
  1. `zetl merkle drift -d ./vault`
     → Returns: "Redis.md: SPL block at line 8 — surrounding prose changed
        (heading hash changed, paragraph hash changed), SPL content unchanged.
        Theory provenance may be stale."
  2. Agent reads the file and evaluates whether SPL still holds
  3. If SPL is still valid: `zetl merkle acknowledge "Redis.md" --block 8`
     → Updates the drift baseline without modifying the file
  4. If SPL needs updating: agent edits the SPL block
  5. `zetl reason status` — theory rebuilt with fresh hashes
Postconditions:
  - All SPL blocks are confirmed consistent with their prose
  - Provenance metadata references current content hashes
Failure modes:
  - SPL block references a line that no longer exists (file was
    restructured) → diagnostic with old and new line numbers
```

```
Happy Path: Cache Avoids Unnecessary Theory Rebuild

Preconditions:
  - Vault was indexed with Merkle hashes cached
  - User edits a file that has no SPL blocks
Steps:
  1. File mtime changes, triggering a reparse
  2. Scanner re-extracts wikilinks and computes new AST hash
  3. File's Merkle root changes (prose was edited)
  4. Vault root changes
  5. Theory cache check: no SPL-containing file's SPL leaf hash changed
  6. Theory cache remains valid — no reasoning rebuild
Postconditions:
  - Link index is updated (new prose might have new wikilinks)
  - Theory cache is NOT rebuilt (SPL unchanged)
  - User sees faster response than a full rebuild
Failure modes:
  - None — this is the optimal fast path
```

---

## 3. Merkle Tree Structure

### 3.1 Hash Algorithm

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
     + Merkle tree mode is built into the design (BLAKE3 IS a Merkle tree internally)
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
  - BLAKE3's internal Merkle tree structure means our application-level
    Merkle tree benefits from optimal cache behaviour
  - 32 bytes (256 bits) provides collision resistance well beyond our
    needs (birthday bound at 2^128)
  - The blake3 crate is pure Rust with optional SIMD acceleration

Consequences:
  + Sub-millisecond hashing for typical files (<10 KB)
  + Compact hash storage: 32 bytes per node
  - Adds blake3 as a dependency (~100 KB to binary size)
  - Hashes are not directly comparable with git SHA-1 objects
```

### 3.2 Node Types

The Merkle tree has four kinds of nodes, corresponding to increasing levels of aggregation:

```
┌──────────────────────────────────────────────────────────┐
│                       Node Types                          │
├───────────────┬──────────────────────────────────────────┤
│ Leaf          │ Atomic content unit from the AST.        │
│               │ Hash = BLAKE3(normalised_content)        │
├───────────────┼──────────────────────────────────────────┤
│ SPL Leaf      │ Special leaf for `spl`-tagged blocks.    │
│               │ Hash = BLAKE3(content_hash ‖ ast_hash)   │
│               │   content_hash = BLAKE3(raw_spl_text)    │
│               │   ast_hash = BLAKE3(serialised_spl_ast)  │
├───────────────┼──────────────────────────────────────────┤
│ File Interior │ Intermediate node for one file.          │
│               │ Hash = BLAKE3(child₁_hash ‖ child₂_hash │
│               │        ‖ … ‖ childₙ_hash)               │
├───────────────┼──────────────────────────────────────────┤
│ Vault Root    │ Top-level node for the entire vault.     │
│               │ Hash = BLAKE3(file₁_hash ‖ file₂_hash   │
│               │        ‖ … ‖ fileₘ_hash)                │
│               │ Files sorted by canonical relative path. │
└───────────────┴──────────────────────────────────────────┘
```

### 3.3 Leaf Node Construction from pulldown-cmark Events

pulldown-cmark produces a stream of `(Event, Range<usize>)` tuples. The Merkle tree groups these into leaf nodes at the **block level** — each top-level block element in the Markdown AST becomes one leaf:

| pulldown-cmark Event Sequence | Leaf Type | Hash Input |
| --- | --- | --- |
| `Start(Heading)` … `End(Heading)` | `Heading` | Normalised text content, heading level |
| `Start(Paragraph)` … `End(Paragraph)` | `Paragraph` | Normalised text content |
| `Start(CodeBlock(Fenced("spl")))` … `End(CodeBlock)` | `SplBlock` | Dual hash (see §3.2) |
| `Start(CodeBlock(…))` … `End(CodeBlock)` | `CodeBlock` | Raw content, language tag |
| `Start(List)` … `End(List)` | `List` | Normalised items text |
| `Start(BlockQuote)` … `End(BlockQuote)` | `BlockQuote` | Normalised content |
| `Start(Table)` … `End(Table)` | `Table` | Normalised cells |
| `Start(MetadataBlock)` … `End(MetadataBlock)` | `Frontmatter` | Raw YAML content |
| `Rule` (thematic break) | `ThematicBreak` | Constant sentinel |
| `Html(…)` (block-level) | `HtmlBlock` | Raw HTML content |

**Normalisation rules for text content:**

1. Collapse consecutive whitespace to a single space
2. Trim leading/trailing whitespace
3. Strip inline formatting markers (bold, italic, etc.) — hash the plain text content, not the Markdown syntax
4. Preserve case (case changes ARE content changes)
5. Normalise line endings to `\n`

**Rationale for block-level granularity:** Finer granularity (individual inline elements) would produce deeper trees with more nodes but minimal practical benefit — drift detection operates at the block level ("did the paragraph above the SPL block change?"), not at the word level. Coarser granularity (entire file as one leaf) would lose the ability to localise changes within a file.

### 3.4 SPL Leaf Dual Hashing

An SPL leaf contains two hashes that serve different purposes:

```rust
struct SplLeafHash {
    /// BLAKE3 hash of the raw SPL text between the fences, after
    /// normalising whitespace and comments. Detects textual changes
    /// (renamed labels, added facts, reformatted expressions).
    content_hash: [u8; 32],

    /// BLAKE3 hash of the serialised SPL AST produced by spindle-parser.
    /// Detects semantic changes (new rules, removed facts, changed
    /// superiority) even if the textual representation is reformatted.
    /// Computed by serialising the parsed Theory fragment to a
    /// canonical byte representation and hashing it.
    ast_hash: [u8; 32],

    /// The combined hash used as this leaf's contribution to the
    /// parent file node: BLAKE3(content_hash ‖ ast_hash).
    combined_hash: [u8; 32],
}
```

**Why dual hashing?**

- `content_hash` changes when the SPL text is reformatted (comment added, whitespace changed) even if the logical meaning is identical. This is useful for detecting **any** edit to the block.
- `ast_hash` changes only when the parsed AST changes — a new fact, removed rule, or changed superiority relation. Reformatting without semantic change leaves `ast_hash` unchanged. This is useful for detecting **meaningful** edits.
- The combined hash feeds into the file-level Merkle tree, ensuring that any change to the SPL block propagates upward.

**SPL AST canonical serialisation:**

The parsed SPL fragment is serialised to a canonical byte representation by:

1. Sorting rules by label (lexicographic)
2. Sorting facts by literal name
3. Sorting superiority relations by (superior, inferior) pair
4. For each element, encoding: `type_tag | label_bytes | body_bytes | head_bytes`
5. Concatenating all encoded elements
6. Hashing the concatenation with BLAKE3

This ensures that logically equivalent SPL fragments (same facts, rules, superiority) with different textual orderings produce the same `ast_hash`.

### 3.5 File Interior Node

Each file's Merkle root is computed from its ordered list of leaf hashes:

```
file_hash = BLAKE3(leaf₁_hash ‖ leaf₂_hash ‖ … ‖ leafₙ_hash)
```

The leaves are in **document order** — the order in which pulldown-cmark produces them, which corresponds to the top-to-bottom order of the Markdown file. This means:

- Reordering sections (moving a heading + paragraph above another) changes the file hash
- Inserting a new paragraph between existing ones changes the file hash
- Deleting a section changes the file hash
- Editing a single paragraph changes only that leaf's hash, which propagates to the file hash

### 3.6 Vault Root Node

The vault root is computed from file hashes, sorted by canonical relative path:

```
vault_hash = BLAKE3(
    file_hash("architecture/Cache.md") ‖
    file_hash("architecture/Scanner.md") ‖
    file_hash("concepts/Wikilinks.md") ‖
    …
)
```

**Canonical path sorting:** paths are sorted lexicographically by their UTF-8 bytes after normalisation to forward slashes. This ensures the vault root is deterministic regardless of filesystem iteration order.

### 3.7 Standalone SPL Files

Standalone `.spl` files (not embedded in Markdown) are treated as single-leaf files:

1. The entire file content is parsed by spindle-parser
2. One SPL leaf is produced with dual hashing (content + AST)
3. The file interior node has exactly one child — the SPL leaf
4. `file_hash = BLAKE3(spl_leaf_combined_hash)` (single-child degenerate case)

---

## 4. Requirements

### 4.1 Functional Requirements

```
REQ-037: Merkle Tree Construction from Markdown AST

The system SHALL construct a Merkle tree for each Markdown file in the
vault by:
  a) Parsing the file with pulldown-cmark to produce an AST event stream
  b) Grouping events into block-level leaf nodes (headings, paragraphs,
     code blocks, lists, blockquotes, tables, frontmatter, thematic breaks,
     HTML blocks)
  c) Computing a BLAKE3 hash for each leaf node from its normalised content
  d) Computing the file's Merkle root as BLAKE3 of the ordered concatenation
     of leaf hashes

The leaf node construction SHALL occur during the same scan pass as
wikilink and SPL extraction (SPEC-001 REQ-001, SPEC-005 REQ-026).

FOR all user roles
WITH the Merkle tree computed incrementally alongside existing parsing
AND no modification to the source files.

Trace:
- TEST-038
- CON-019
- ADR-008
```

```
REQ-038: SPL Block Dual Hashing

The system SHALL produce a dual-hash SPL leaf node for every `spl`-tagged
fenced code block by:
  a) Computing a content hash: BLAKE3 of the raw SPL text, normalised
     (whitespace-collapsed, comments stripped, line endings normalised)
  b) Computing an AST hash: BLAKE3 of the canonically-serialised SPL AST
     produced by spindle-parser (rules sorted by label, facts sorted by
     literal, superiority sorted by pair)
  c) Computing a combined hash: BLAKE3(content_hash ‖ ast_hash) as the
     leaf's contribution to the file Merkle tree

If spindle-parser fails to parse the SPL block, the AST hash SHALL be
a sentinel value (all zeros) and a diagnostic SHALL be emitted. The
content hash is still computed from the raw text.

FOR all user roles
WITH dual hashing enabling both textual and semantic change detection.

Trace:
- TEST-039
- CON-019
```

```
REQ-039: Vault-Level Merkle Root

The system SHALL compute a vault-level Merkle root hash by:
  a) Collecting file Merkle roots for all indexed files (Markdown and
     standalone SPL)
  b) Sorting files by canonical relative path (UTF-8 lexicographic,
     forward-slash normalised)
  c) Computing BLAKE3 of the ordered concatenation of file hashes

The vault root hash SHALL be stored in the cache alongside the existing
index and theory caches.

FOR all user roles
WITH the vault root serving as a single-value integrity fingerprint
for the entire vault state.

Trace:
- TEST-040
- CON-019
```

```
REQ-040: Two-Tier Cache Invalidation (Mtime + Content Hash)

The system SHALL implement a two-tier cache invalidation strategy:
  a) Tier 1 (fast): Check file mtime against cached mtime. If unchanged,
     skip hashing and reuse cached Merkle tree nodes.
  b) Tier 2 (authoritative): If mtime changed, recompute the file's
     Merkle tree. If the new file Merkle root equals the cached root,
     the file's content is semantically unchanged — skip downstream
     processing (link resolution, theory rebuild).

This two-tier strategy SHALL replace the existing mtime-only invalidation
for both the link index (SPEC-001 REQ-011) and the theory cache
(SPEC-005 NFR-011).

FOR all user roles
WITH the mtime check as a performance pre-filter and the content hash
as the authoritative invalidation signal.

Trace:
- TEST-041
- ADR-009
```

```
REQ-041: SPL-Specific Theory Cache Invalidation

The system SHALL use SPL leaf AST hashes (not file-level hashes or mtime)
to determine whether the theory cache is valid:
  a) Collect all SPL leaf AST hashes from the current Merkle tree
  b) Compare against the SPL leaf AST hashes stored in the theory cache
  c) If the set of AST hashes is identical, the theory cache is valid —
     skip theory reconstruction and re-reasoning
  d) If any AST hash differs (or the set of SPL blocks changed), invalidate
     the theory cache and rebuild

This means:
  - Editing prose around an SPL block does NOT invalidate the theory cache
    (file hash changes, but SPL AST hash does not)
  - Reformatting an SPL block without changing its logical content does NOT
    invalidate the theory cache (content hash changes, but AST hash does not)
  - Adding, removing, or modifying an SPL block's logical content DOES
    invalidate the theory cache (AST hash changes)

FOR all user roles
WITH theory rebuilds occurring only when the logical content of SPL blocks
has actually changed.

Trace:
- TEST-042
- ADR-009
```

```
REQ-042: SPL Drift Detection

The system SHALL detect and report "drift" — cases where the Markdown
prose surrounding an SPL block has changed since the theory was last
built, while the SPL block itself has not.

For each SPL block in the vault, drift is detected by:
  a) Retrieving the file's Merkle tree from the last theory build
     (stored in the theory cache)
  b) Comparing each non-SPL leaf hash (headings, paragraphs, etc.)
     in the current tree against the cached tree for the same file
  c) If any non-SPL sibling leaf has changed AND the SPL leaf's
     AST hash is unchanged, the block is flagged as "potentially drifted"

Drift reports SHALL include:
  - File path and SPL block line number
  - Which sibling nodes changed (heading, paragraph, etc.)
  - The SPL block's content (for human review)
  - A severity level: "info" if only distant siblings changed, "warning"
    if immediately adjacent siblings changed

FOR all user roles
WITH output via `zetl merkle drift`.

Trace:
- TEST-043
- CON-020
```

```
REQ-043: Durable Provenance with Content Hashes

The system SHALL extend theory provenance metadata (SPEC-005 ADR-007)
to include content hashes:
  a) Each provenanced rule and fact SHALL include the SPL leaf's
     content_hash and ast_hash from the Merkle tree at the time
     the theory was built
  b) Each provenanced conclusion SHALL include the vault root hash
     at the time reasoning was performed
  c) The provenance command (`zetl reason provenance`) SHALL display
     content hashes alongside file paths and line numbers

This enables verification: given a conclusion's provenance, one can
check whether the source content has changed by comparing the stored
hash against the current Merkle tree's hash for the same node.

FOR all user roles
WITH content hashes stored in the theory cache and accessible via
existing provenance commands.

Trace:
- TEST-044
- CON-021
```

```
REQ-044: Merkle Tree Inspection

The system SHALL provide commands to inspect the Merkle tree:
  a) `zetl merkle status` — display the current vault root hash,
     file count, total leaf count, and SPL leaf count
  b) `zetl merkle tree <file>` — display the leaf-level Merkle tree
     for a specific file, showing each leaf's type, line range, and hash
  c) `zetl merkle diff <file>` — compare the current file Merkle tree
     against the cached version, showing which leaves changed

FOR all user roles
WITH output in JSON (default) or table format.

Trace:
- TEST-045
- CON-019, CON-020
```

### 4.2 Non-Functional Requirements

```
NFR-014: Merkle Tree Construction Performance

Merkle tree construction SHALL add ≤ 20% overhead to the existing
scan pass for a vault with ≤ 10,000 files UNDER single-threaded
execution on commodity hardware WITH 95th percentile.

Rationale: BLAKE3 hashing at 5 GB/s is negligible compared to file
I/O and Markdown parsing. The overhead is primarily from AST node
grouping and memory allocation for the tree structure.

Trace:
- TEST-046
```

```
NFR-015: Merkle Tree Memory Overhead

Peak memory increase from Merkle tree construction SHALL be ≤ 30 MB
above the baseline (SPEC-001 NFR-003: 200 MB) for a vault with
10,000 files at an average of 50 leaf nodes per file.

Rationale: Each leaf stores a 32-byte hash, node type tag (1 byte),
and line range (8 bytes) = ~41 bytes per leaf. 10,000 files × 50
leaves = 500,000 leaves × 41 bytes = ~20 MB. Interior nodes and
overhead account for the remaining 10 MB.

Trace:
- TEST-047
```

```
NFR-016: Merkle Cache Size

The serialised Merkle tree cache SHALL add ≤ 5 MB to the existing
cache files (.zetl/) for a vault with 10,000 files.

Rationale: Stored data per file is the file hash (32 bytes) plus
per-leaf hash and metadata (~50 bytes × 50 leaves = 2,500 bytes
per file). 10,000 × 2,532 bytes ≈ 25 MB. Compressing with the
existing JSON format and omitting full leaf data for non-SPL leaves
(storing only the file root + SPL leaf hashes) brings this under
5 MB.

Trace:
- TEST-048
```

---

## 5. Architecture

### 5.1 Technology Decisions

```
ADR-009: Two-Tier Cache Invalidation — Mtime + Content Hash

Status: Proposed

Context:
  The existing cache system (SPEC-001 REQ-011, SPEC-005 NFR-011) uses
  mtime-only invalidation. This has two weaknesses:

  1. False positives: mtime changes when a file is touched but not
     modified (e.g., `touch file.md`, backup restoration, git checkout).
     This triggers unnecessary reparsing and theory rebuilds.

  2. Inability to distinguish content types: when a file containing
     both prose and SPL is edited, mtime cannot tell whether the SPL
     changed. The theory cache is invalidated even if only prose changed.

  Options:
  A. Replace mtime with content hashing:
     + Authoritative — no false positives
     - Slower: must read and hash every file on every invocation
     - Defeats the purpose of caching for unchanged files

  B. Keep mtime as first tier, add content hash as second tier:
     + Fast path: mtime unchanged → skip entirely (same as today)
     + Accurate path: mtime changed → hash to determine if content
       actually differs
     + SPL-specific: compare SPL AST hashes to determine if theory
       needs rebuilding, independent of prose changes
     - Slightly more complex cache format
     - Must store hashes alongside mtimes

  C. Use filesystem watch events (inotify/kqueue):
     + Real-time notification of changes
     - Requires a persistent process (incompatible with CLI model)
     - Platform-specific complexity
     - Doesn't solve the prose-vs-SPL distinction

Decision:
  Implement Option B — two-tier invalidation with mtime pre-filter
  and content-hash authority.

Rationale:
  - The fast path (mtime unchanged) adds zero overhead to the common
    case where files haven't been touched
  - The accurate path (mtime changed, rehash) only applies to modified
    files — typically a small fraction of the vault
  - SPL AST hash comparison enables the key insight: theory rebuilds
    are triggered only by logical content changes to SPL blocks, not
    by prose edits or SPL reformatting
  - The cache format change is additive (new fields alongside existing
    mtime), preserving backward compatibility with the v1 cache

Consequences:
  + Eliminates false theory rebuilds from prose-only edits
  + Eliminates false rebuilds from file touches without content changes
  + Enables SPL drift detection as a natural byproduct
  - Cache size increases (32 bytes per file for file hash, 64 bytes
    per SPL leaf for dual hash)
  - First run after cache format upgrade triggers a full rehash
```

### 5.2 Component Architecture

```
                         ┌──────────────────────┐
                         │        CLI           │
                         │  (existing + merkle  │
                         │   subcommands)       │
                         └──────────┬───────────┘
                                    │
          ┌─────────────────────────┼──────────────────────────┐
          │                         │                           │
   ┌──────▼──────┐          ┌──────▼──────┐            ┌──────▼───────────┐
   │   Scanner    │          │   Graph    │            │    Reason        │
   │   (extended) │          │   Engine   │            │    Engine        │
   │              │          │            │            │    (extended)    │
   │ - file walk  │          │ - build    │            │ - extract spl   │
   │ - parse md   │          │ - query    │            │ - build theory  │
   │ - extract    │          │ - path     │            │ - reason        │
   │   wikilinks  │          │            │            │ - provenance    │
   │ - extract    │          └────────────┘            │   + hashes      │  NEW
   │   spl blocks │                                    └────────┬────────┘
   │ - build      │  NEW                                        │
   │   leaf nodes │                                             │
   └──────┬───────┘                                             │
          │                                                     │
          │         ┌───────────────────────┐                   │
          │         │   Merkle Engine       │  NEW              │
          │         │                       │                   │
          ├────────►│ - compute leaf hashes │                   │
          │         │ - build file tree     │◄──────────────────┘
          │         │ - build vault root    │
          │         │ - drift detection     │
          │         │ - tree comparison     │
          │         └───────────┬───────────┘
          │                     │
          └──────────┬──────────┘
                     │
              ┌──────▼───────┐
              │    Cache     │
              │  .zetl/      │
              │  index.json  │
              │  theory.json │
              │  merkle.json │  NEW
              └──────────────┘
```

**Scanner (extended)** — During the existing Markdown scan pass, the scanner now also groups pulldown-cmark events into block-level leaf nodes. Each leaf is emitted alongside the existing wikilinks and SPL blocks. The scanner produces `ParsedFile` records that now include a `Vec<MerkleLeaf>`.

**Merkle Engine (new)** — Consumes leaf nodes from the scanner, computes BLAKE3 hashes, builds file-level Merkle trees, and assembles the vault root. Provides query methods: tree inspection, comparison against cached trees, drift detection. The drift detector compares current Merkle trees against the theory cache's snapshot to identify SPL blocks surrounded by changed prose.

**Reason Engine (extended)** — When building the theory, the reason engine now records SPL leaf hashes (both content and AST) in the provenance metadata. Theory cache validation uses SPL AST hashes instead of mtime.

**Cache (extended)** — Adds `.zetl/merkle.json` to store per-file Merkle roots and per-SPL-block dual hashes. The theory cache is extended with SPL AST hashes for validation.

### 5.3 Data Model

```rust
/// Hash type alias for BLAKE3 output
type ContentHash = [u8; 32];

/// A leaf node in the file-level Merkle tree
struct MerkleLeaf {
    /// What kind of Markdown block this leaf represents
    node_type: LeafType,
    /// 1-indexed start line in the source file
    start_line: u32,
    /// 1-indexed end line in the source file
    end_line: u32,
    /// BLAKE3 hash of the normalised content
    hash: ContentHash,
    /// Additional hashes for SPL blocks (None for non-SPL leaves)
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
    /// BLAKE3 of normalised raw SPL text
    content_hash: ContentHash,
    /// BLAKE3 of canonically-serialised SPL AST (all-zero if parse failed)
    ast_hash: ContentHash,
}

/// File-level Merkle tree
struct FileMerkleTree {
    /// Relative path from vault root
    path: PathBuf,
    /// Ordered list of leaf nodes (document order)
    leaves: Vec<MerkleLeaf>,
    /// BLAKE3(leaf₁_hash ‖ leaf₂_hash ‖ … ‖ leafₙ_hash)
    root_hash: ContentHash,
}

/// Vault-level Merkle state
struct VaultMerkle {
    /// BLAKE3(file₁_hash ‖ file₂_hash ‖ … ‖ fileₘ_hash)
    /// Files sorted by canonical relative path.
    root_hash: ContentHash,
    /// Per-file trees (for inspection and comparison)
    files: HashMap<PathBuf, FileMerkleTree>,
    /// Quick lookup: all SPL leaf AST hashes in the vault
    spl_ast_hashes: Vec<(PathBuf, u32, ContentHash)>,  // (file, line, ast_hash)
}

/// Extended theory provenance with content hashes
struct HashedProvenance {
    /// Existing provenance fields (SPEC-005)
    source_file: PathBuf,
    source_line: u32,
    source_page: String,
    /// NEW: SPL content hash at time of theory construction
    spl_content_hash: ContentHash,
    /// NEW: SPL AST hash at time of theory construction
    spl_ast_hash: ContentHash,
}

/// Drift report for an SPL block
struct DriftReport {
    /// File containing the SPL block
    file: PathBuf,
    /// Line number of the SPL block
    spl_line: u32,
    /// SPL content (for human review)
    spl_content: String,
    /// Whether the SPL block itself changed
    spl_changed: bool,
    /// Sibling nodes that changed
    changed_siblings: Vec<ChangedSibling>,
    /// Severity: "info" (distant changes), "warning" (adjacent changes)
    severity: DriftSeverity,
}

struct ChangedSibling {
    node_type: LeafType,
    start_line: u32,
    /// Distance in leaf positions from the SPL block
    /// (1 = immediately adjacent, 2 = one node away, etc.)
    distance: u32,
}

enum DriftSeverity {
    /// Non-SPL siblings changed, but not immediately adjacent to SPL block
    Info,
    /// Immediately adjacent sibling (preceding or following) changed
    Warning,
}
```

### 5.4 Merkle Tree Construction Pipeline

```
Markdown file content
        │
        ▼
┌───────────────────┐
│ pulldown-cmark     │  AST event stream: (Event, Range<usize>)
│ parse with offsets │
└───────┬───────────┘
        │
        ▼
┌───────────────────┐
│ Block Grouper      │  Group events into block-level units
│                    │  Each block = one Merkle leaf
│                    │  SPL blocks detected by language tag
└───────┬───────────┘
        │
        ▼
┌───────────────────┐
│ Content Normaliser │  Per leaf type:
│                    │  - Text: collapse whitespace, strip formatting
│                    │  - SPL: normalise whitespace, strip comments
│                    │  - Code: preserve raw content + language tag
│                    │  - Frontmatter: preserve raw YAML
└───────┬───────────┘
        │
        ▼
┌───────────────────┐
│ Leaf Hasher        │  BLAKE3 hash each normalised leaf
│                    │  SPL blocks: dual hash (content + AST)
│                    │  Produces Vec<MerkleLeaf>
└───────┬───────────┘
        │
        ▼
┌───────────────────┐
│ Tree Builder       │  Compute file Merkle root from ordered leaves
│                    │  BLAKE3(leaf₁ ‖ leaf₂ ‖ … ‖ leafₙ)
│                    │  Produces FileMerkleTree
└───────┬───────────┘
        │
        ▼  (repeated for all files)
┌───────────────────┐
│ Vault Root Builder │  Collect file roots, sort by path
│                    │  BLAKE3(file₁ ‖ file₂ ‖ … ‖ fileₘ)
│                    │  Produces VaultMerkle
└───────────────────┘
```

### 5.5 Drift Detection Algorithm

```
Input:
  - cached_trees: HashMap<PathBuf, FileMerkleTree>  (from last theory build)
  - current_trees: HashMap<PathBuf, FileMerkleTree>  (from current scan)

For each file in current_trees:
  1. If file not in cached_trees → skip (new file, no drift baseline)
  2. If file root_hash unchanged → skip (nothing changed)
  3. If file root_hash changed:
     a. Find all SPL leaves in current file (by LeafType::SplBlock)
     b. For each SPL leaf:
        i.  Find corresponding SPL leaf in cached file (by line proximity)
        ii. If SPL leaf ast_hash unchanged AND any non-SPL sibling hash
            changed → DRIFT DETECTED
        iii. Compute severity:
             - Check leaves at distance 1 (immediately before/after SPL block)
             - If any distance-1 sibling changed → Warning
             - Else → Info

Output: Vec<DriftReport>
```

**Line proximity matching:** When comparing SPL blocks between cached and current trees, the system matches by position in the leaf sequence (index within the file's leaves), not by absolute line number. This handles the case where line numbers shift due to insertions/deletions above the SPL block.

---

## 6. Contract Specifications (CLI Interface)

### 6.1 Merkle Subcommand Group

```
CON-019: zetl merkle status

zetl merkle status [OPTIONS]

Display the current vault Merkle tree summary.

Exit codes:
  0  Merkle tree computed successfully

Example output (JSON):
{
  "vault_root_hash": "a1b2c3d4e5f6...",
  "file_count": 47,
  "total_leaves": 2341,
  "spl_leaves": 23,
  "spl_files": 12,
  "cache_state": "valid",
  "last_computed": "2026-02-24T10:30:00Z"
}

Implements:
- REQ-044

Verified by:
- TEST-045
```

```
CON-020: zetl merkle drift

zetl merkle drift [OPTIONS]

Detect SPL blocks whose surrounding prose has changed since the
theory was last built.

Options:
  --file <PATH>    Check only a specific file
  --severity <LVL> Minimum severity to report: info | warning [default: info]

Exit codes:
  0  No drift detected (or drift reported successfully)
  1  Drift detected at warning severity (with --fail-on-drift flag)

Example output (JSON):
{
  "drift_reports": [
    {
      "file": "decisions/Redis vs Memcached.md",
      "spl_line": 8,
      "spl_content": "(given redis-benchmarked)\n(given redis-fast-enough)...",
      "spl_changed": false,
      "severity": "warning",
      "changed_siblings": [
        {
          "node_type": "Paragraph",
          "start_line": 5,
          "distance": 1
        }
      ],
      "message": "SPL block at line 8 unchanged, but adjacent paragraph at line 5 was modified. Review whether SPL claims still reflect the updated prose."
    }
  ],
  "summary": {
    "total_spl_blocks": 23,
    "drifted_blocks": 1,
    "warning_count": 1,
    "info_count": 0
  }
}

Implements:
- REQ-042

Verified by:
- TEST-043
```

```
CON-021: zetl merkle tree

zetl merkle tree <FILE> [OPTIONS]

Display the leaf-level Merkle tree for a specific file.

Arguments:
  <FILE>  File path (relative to vault root)

Exit codes:
  0  Tree displayed
  1  File not found

Example output (JSON):
{
  "file": "decisions/Redis vs Memcached.md",
  "root_hash": "b3c4d5e6f7a8...",
  "leaves": [
    {
      "index": 0,
      "type": "Frontmatter",
      "lines": [1, 4],
      "hash": "1a2b3c4d..."
    },
    {
      "index": 1,
      "type": "Heading",
      "level": 1,
      "lines": [5, 5],
      "hash": "2b3c4d5e..."
    },
    {
      "index": 2,
      "type": "Paragraph",
      "lines": [7, 9],
      "hash": "3c4d5e6f..."
    },
    {
      "index": 3,
      "type": "SplBlock",
      "lines": [11, 16],
      "hash": "4d5e6f7a...",
      "spl_hashes": {
        "content_hash": "5e6f7a8b...",
        "ast_hash": "6f7a8b9c..."
      }
    }
  ]
}

Implements:
- REQ-044

Verified by:
- TEST-045
```

```
CON-022: zetl merkle diff

zetl merkle diff <FILE> [OPTIONS]

Compare the current Merkle tree for a file against its cached version.

Arguments:
  <FILE>  File path (relative to vault root)

Exit codes:
  0  Comparison completed (may show changes or no changes)
  1  File not found or not in cache

Example output (JSON):
{
  "file": "decisions/Redis vs Memcached.md",
  "cached_root_hash": "b3c4d5e6f7a8...",
  "current_root_hash": "x9y8z7w6v5u4...",
  "changed": true,
  "leaf_changes": [
    {
      "index": 2,
      "type": "Paragraph",
      "lines": [7, 9],
      "change": "modified",
      "cached_hash": "3c4d5e6f...",
      "current_hash": "a1b2c3d4..."
    }
  ],
  "added_leaves": [],
  "removed_leaves": [],
  "spl_leaves_changed": false
}

Implements:
- REQ-044

Verified by:
- TEST-045
```

---

## 7. Test Specifications

```
TEST-038: Merkle Tree Construction from Markdown AST

Scenario: Build Merkle tree for a simple Markdown file
Given: A file containing:
  Line 1:  ---
  Line 2:  title: Test
  Line 3:  ---
  Line 4:  # Heading
  Line 5:
  Line 6:  A paragraph with [[wikilink]].
  Line 7:
  Line 8:  ```spl
  Line 9:  (given test-fact)
  Line 10: ```
  Line 11:
  Line 12: Another paragraph.
When: The scanner processes this file with Merkle tree construction
Then:
  - 4 leaf nodes are produced: Frontmatter, Heading, Paragraph, SplBlock,
    Paragraph
  - Wait, 5 leaves: Frontmatter, Heading(1), Paragraph, SplBlock, Paragraph
  - Each leaf has a non-zero BLAKE3 hash
  - The SplBlock leaf has spl_hashes with content_hash and ast_hash
  - The file root hash = BLAKE3(leaf₁ ‖ leaf₂ ‖ leaf₃ ‖ leaf₄ ‖ leaf₅)

Scenario: Leaf order matches document order
Given: A file with Heading, Paragraph, Heading, Paragraph
When: The Merkle tree is built
Then:
  - leaves[0] is Heading, leaves[1] is Paragraph, etc.
  - Swapping the two sections would produce a different root hash

Scenario: Normalisation makes formatting-only changes invisible
Given: Two files with identical text content but different whitespace
  File A: "Some  text   with   extra  spaces"
  File B: "Some text with extra spaces"
When: Both files produce Merkle trees
Then:
  - The paragraph leaf hashes are identical
  - The file root hashes are identical

Verifies: REQ-037
```

```
TEST-039: SPL Block Dual Hashing

Scenario: Content hash and AST hash computed for SPL block
Given: A file with an SPL block:
  ```spl
  (given bird)
  (normally r1 bird flies)
  ```
When: The SPL leaf is hashed
Then:
  - content_hash = BLAKE3(normalised "(given bird)\n(normally r1 bird flies)")
  - ast_hash = BLAKE3(canonical serialisation of {fact: bird, rule: r1})
  - combined_hash = BLAKE3(content_hash ‖ ast_hash)

Scenario: Reformatted SPL changes content_hash but not ast_hash
Given: Two files with logically identical SPL:
  File A: "(given bird)\n(normally r1 bird flies)"
  File B: "(given   bird)\n\n; a comment\n(normally  r1  bird  flies)"
When: Both SPL blocks are dual-hashed
Then:
  - content_hash differs (different raw text after normalisation may differ
    depending on comment stripping; with comments stripped and whitespace
    collapsed, they should be equal)
  - ast_hash is identical (same parsed AST)

Scenario: SPL parse error produces sentinel AST hash
Given: An SPL block with invalid syntax: "(given unclosed"
When: The SPL leaf is hashed
Then:
  - content_hash is computed from the raw text
  - ast_hash is [0u8; 32] (sentinel)
  - A diagnostic is emitted

Verifies: REQ-038
```

```
TEST-040: Vault-Level Merkle Root

Scenario: Vault root is deterministic
Given: A vault with 3 files: a.md, b.md, c.md
When: The vault Merkle root is computed twice (without changes)
Then:
  - Both computations produce the same root hash

Scenario: File ordering is canonical
Given: Files are scanned in random filesystem order
When: The vault root is computed
Then:
  - The root hash is the same regardless of scan order
  - Files are sorted by relative path before hashing

Scenario: Adding a file changes the vault root
Given: A vault with root hash H1
When: A new file d.md is added
Then:
  - The new vault root hash H2 ≠ H1

Scenario: Removing a file changes the vault root
Given: A vault with root hash H1 and file c.md
When: c.md is deleted
Then:
  - The new vault root hash H2 ≠ H1

Verifies: REQ-039
```

```
TEST-041: Two-Tier Cache Invalidation

Scenario: Mtime unchanged → skip hashing
Given: A cached vault where no files have been modified
When: `zetl index` is run
Then:
  - No BLAKE3 hashing occurs (mtime pre-filter catches all files)
  - The vault root hash is read from cache, not recomputed

Scenario: Mtime changed, content unchanged → skip reprocessing
Given: A file is `touch`ed (mtime updated) but content is identical
When: `zetl index` is run
Then:
  - The file is re-read and hashed (mtime check fails)
  - The new file hash equals the cached hash
  - No downstream reprocessing occurs (link resolution, theory rebuild)

Scenario: Mtime changed, content changed → full reprocess
Given: A file's content is actually modified
When: `zetl index` is run
Then:
  - The file is re-read and hashed
  - The new file hash differs from cached
  - Downstream reprocessing occurs

Verifies: REQ-040
```

```
TEST-042: SPL-Specific Theory Cache Invalidation

Scenario: Prose edit in SPL file does NOT trigger theory rebuild
Given: A file containing prose and an SPL block; theory is cached
When: Only the prose paragraph is edited (SPL block unchanged)
Then:
  - File mtime changes → file is rehashed
  - File Merkle root changes (prose leaf changed)
  - SPL leaf AST hash is unchanged
  - Theory cache remains valid — no theory rebuild

Scenario: SPL reformatting does NOT trigger theory rebuild
Given: A file whose SPL block is reformatted (extra whitespace, comments)
       but logically unchanged
When: `zetl reason status` is run
Then:
  - SPL content_hash may change
  - SPL ast_hash is unchanged
  - Theory cache remains valid

Scenario: SPL logical change DOES trigger theory rebuild
Given: A file where a new fact "(given new-fact)" is added to the SPL block
When: `zetl reason status` is run
Then:
  - SPL ast_hash changes
  - Theory cache is invalidated
  - Theory is rebuilt from all SPL blocks
  - New conclusions reflect the added fact

Verifies: REQ-041
```

```
TEST-043: SPL Drift Detection

Scenario: Adjacent prose change flags drift
Given: A file with structure: Heading, Paragraph-A, SplBlock, Paragraph-B
       Theory was built with these hashes cached.
When: Paragraph-A is edited (content changes) but SplBlock is unchanged
Then:
  - `zetl merkle drift` reports 1 drift at Warning severity
  - Report shows: SplBlock at line N, Paragraph at distance 1 changed

Scenario: Distant prose change flags drift at info level
Given: Same file structure; the Heading is edited, Paragraph-A is unchanged
When: `zetl merkle drift` is run
Then:
  - Reports 1 drift at Info severity (Heading is distance > 1 from SPL)

Scenario: SPL block itself changed — not drift
Given: Both the prose and the SPL block are edited
When: `zetl merkle drift` is run
Then:
  - No drift reported (SPL was updated alongside prose)

Scenario: No changes — no drift
Given: No files modified since theory was built
When: `zetl merkle drift` is run
Then:
  - 0 drift reports

Verifies: REQ-042
```

```
TEST-044: Durable Provenance with Content Hashes

Scenario: Provenance includes content hashes
Given: A theory built from vault with Merkle hashes
When: `zetl reason provenance "some-literal"` is run
Then:
  - Each proof source includes spl_content_hash and spl_ast_hash
  - The output includes the vault_root_hash at time of reasoning

Scenario: Provenance hash verification
Given: A conclusion with stored spl_ast_hash from a previous run
When: The source SPL block is modified and `zetl reason provenance` is run
Then:
  - The stored hash no longer matches the current Merkle tree's hash
  - A "stale provenance" warning is emitted

Verifies: REQ-043
```

```
TEST-045: Merkle Tree Inspection Commands

Scenario: merkle status
Given: An indexed vault with Merkle hashes cached
When: `zetl merkle status` is run
Then:
  - Returns vault root hash, file count, leaf count, SPL leaf count
  - Output matches CON-019 schema

Scenario: merkle tree for a specific file
Given: A file "test.md" in the vault
When: `zetl merkle tree "test.md"` is run
Then:
  - Returns ordered list of leaves with type, lines, hash
  - SPL leaves include spl_hashes
  - Output matches CON-021 schema

Scenario: merkle diff shows changes
Given: A cached vault where one file has been modified
When: `zetl merkle diff "modified.md"` is run
Then:
  - Shows which leaves changed, were added, or removed
  - Output matches CON-022 schema

Scenario: merkle diff shows no changes
Given: A file that hasn't changed since caching
When: `zetl merkle diff "unchanged.md"` is run
Then:
  - changed: false, leaf_changes: [], added_leaves: [], removed_leaves: []

Verifies: REQ-044
```

```
TEST-046: Merkle Tree Construction Performance

Scenario: Overhead within bounds
Given: A vault with ≥ 1,000 Markdown files
When: Scanning with Merkle tree construction vs. without
Then:
  - Total scan time with Merkle ≤ 1.2× scan time without

Verifies: NFR-014
```

```
TEST-047: Merkle Tree Memory Overhead

Scenario: Memory within bounds
Given: A vault with 10,000 files, ~50 leaves per file
When: Merkle tree is constructed
Then:
  - Peak memory increase ≤ 30 MB above baseline

Verifies: NFR-015
```

```
TEST-048: Merkle Cache Size

Scenario: Cache size within bounds
Given: A vault with 10,000 files cached with Merkle data
When: .zetl/merkle.json is written
Then:
  - File size ≤ 5 MB

Verifies: NFR-016
```

---

## 8. Observability

```
OBS-007: Merkle Tree Timing

When --verbose is specified, Merkle-related commands SHALL emit to stderr:
  - Number of files hashed
  - Number of files skipped (mtime unchanged)
  - Number of files with content-hash match (mtime changed but hash same)
  - Total leaf nodes computed
  - SPL leaf nodes computed (with dual hashing)
  - BLAKE3 hashing time (ms)
  - Total Merkle tree construction time (ms)
```

```
OBS-008: Drift Detection Metrics

`zetl merkle drift` SHALL include a summary section reporting:
  - Total SPL blocks in vault
  - Number of drifted blocks (total, warning, info)
  - Number of files with at least one drifted block
  - Time since theory was last built
to support vault health monitoring.
```

```
OBS-009: Cache Efficiency Metrics

When --verbose is specified, `zetl index` and `zetl reason status`
SHALL emit to stderr:
  - Cache tier 1 hits: files skipped by mtime check
  - Cache tier 1 misses: files re-hashed
  - Cache tier 2 hits: files re-hashed but content unchanged
  - Cache tier 2 misses: files with actual content changes
  - Theory cache hit/miss (SPL AST hash comparison result)
to support cache tuning and performance analysis.
```

---

## 9. Traceability Matrix

| REQ     | CON              | TEST     | ADR     | OBS     |
| ------- | ---------------- | -------- | ------- | ------- |
| REQ-037 | CON-019          | TEST-038 | ADR-008 | OBS-007 |
| REQ-038 | CON-019          | TEST-039 | ADR-008 | OBS-007 |
| REQ-039 | CON-019          | TEST-040 | —       | OBS-007 |
| REQ-040 | —                | TEST-041 | ADR-009 | OBS-009 |
| REQ-041 | —                | TEST-042 | ADR-009 | OBS-009 |
| REQ-042 | CON-020          | TEST-043 | —       | OBS-008 |
| REQ-043 | CON-021          | TEST-044 | —       | —       |
| REQ-044 | CON-019–022      | TEST-045 | —       | —       |
| NFR-014 | —                | TEST-046 | ADR-008 | OBS-007 |
| NFR-015 | —                | TEST-047 | —       | —       |
| NFR-016 | —                | TEST-048 | —       | —       |

---

## 10. Implementation Priority

### P0 — Core Merkle Infrastructure

| Item | Effort | Dependencies |
| --- | --- | --- |
| Leaf node grouper in scanner (REQ-037) | 4 hours | Existing scanner, pulldown-cmark |
| BLAKE3 leaf hashing (REQ-037) | 2 hours | blake3 crate |
| File-level Merkle root (REQ-037) | 1 hour | Leaf hashing |
| Vault-level Merkle root (REQ-039) | 1 hour | File-level roots |
| `merkle status` command (REQ-044 partial) | 1 hour | Vault root |

### P1 — SPL Integration

| Item | Effort | Dependencies |
| --- | --- | --- |
| SPL dual hashing (REQ-038) | 3 hours | P0 complete, spindle-parser |
| Two-tier cache invalidation (REQ-040) | 4 hours | P0 complete |
| SPL-specific theory invalidation (REQ-041) | 2 hours | SPL dual hashing |
| Durable provenance hashes (REQ-043) | 2 hours | SPL dual hashing |

### P2 — Drift Detection and Inspection

| Item | Effort | Dependencies |
| --- | --- | --- |
| Drift detection algorithm (REQ-042) | 4 hours | P1 complete |
| `merkle drift` command (CON-020) | 2 hours | Drift detection |
| `merkle tree` command (CON-021) | 1 hour | P0 complete |
| `merkle diff` command (CON-022) | 2 hours | P0 complete |
| Merkle cache serialisation (NFR-016) | 2 hours | P0 complete |

**Estimated total: ~31 hours** across all priorities.

---

## 11. Cache Format

### 11.1 Merkle Cache Structure

The Merkle cache is stored in `.zetl/merkle.json`:

```json
{
  "version": 1,
  "vault_root_hash": "a1b2c3d4...",
  "computed_at": "2026-02-24T10:30:00Z",
  "files": {
    "architecture/Cache.md": {
      "root_hash": "b3c4d5e6...",
      "mtime": 1708770000.0,
      "spl_leaves": [
        {
          "start_line": 15,
          "end_line": 20,
          "content_hash": "c4d5e6f7...",
          "ast_hash": "d5e6f7a8..."
        }
      ]
    }
  }
}
```

**Compact format rationale:** Only the file root hash and SPL leaf hashes are persisted. Full leaf-level trees are NOT cached to disk (they can be recomputed from the file in <1ms). This keeps the cache small (NFR-016) while retaining the data needed for:

- Vault root comparison (vault_root_hash)
- Two-tier invalidation (file root_hash + mtime)
- Theory invalidation (SPL ast_hash values)
- Drift detection (comparing current tree against cached per-file state)

Full leaf trees are computed on-demand when inspection commands (`merkle tree`, `merkle diff`) are invoked.

### 11.2 Theory Cache Extension

The existing `.zetl/theory.json` (SPEC-005 ADR-006) is extended:

```json
{
  "version": 2,
  "vault_root_hash": "a1b2c3d4...",
  "spl_ast_hashes": {
    "architecture/Cache.md:15": "d5e6f7a8...",
    "decisions/Redis.md:8": "e6f7a8b9..."
  },
  "rules": [ ... ],
  "superiorities": [ ... ],
  "diagnostics": [ ... ]
}
```

**Change from v1:** The `spl_file_mtimes` field is replaced by `spl_ast_hashes`. Theory cache validity is now determined by comparing the set of SPL AST hashes, not file mtimes. The `vault_root_hash` is stored for provenance references.

---

## 12. Integration with Existing Systems

### 12.1 Scanner Integration

The Merkle tree construction is integrated into the scanner's existing parse pass:

```
Existing flow:
  file → pulldown-cmark → extract_wikilinks() + extract_spl_blocks()

Extended flow:
  file → pulldown-cmark → extract_wikilinks() + extract_spl_blocks()
                                               + build_merkle_leaves()
```

`build_merkle_leaves()` operates on the same `(Event, Range)` stream that `extract_wikilinks()` and `extract_spl_blocks()` use. It groups events into block-level leaf nodes and returns `Vec<MerkleLeaf>`. The three extractors share the pulldown-cmark parse — there is no second pass.

### 12.2 Reason Engine Integration

The reason engine's `build_theory()` function is extended to:

1. Accept SPL leaf hashes alongside SPL blocks
2. Store `spl_content_hash` and `spl_ast_hash` in each rule's provenance metadata
3. Store the vault root hash in the theory cache
4. Use SPL AST hashes (not mtime) for theory cache validation

### 12.3 Hence Integration

The vault root hash serves as a **coordination checkpoint** in multi-agent workflows:

```bash
# Record vault state before agent task
VAULT_HASH=$(zetl merkle status | jq -r .vault_root_hash)

# Agent performs work...

# Verify what changed
NEW_HASH=$(zetl merkle status | jq -r .vault_root_hash)
if [ "$VAULT_HASH" != "$NEW_HASH" ]; then
  zetl merkle drift --fail-on-drift
fi
```

---

## 13. Future Considerations

| Item | Rationale |
| --- | --- |
| Incremental Merkle tree updates | Instead of rebuilding the entire file tree on change, update only the affected leaves. Requires an ordered tree structure (not just concatenation). |
| Merkle proofs for provenance | Generate compact proofs that a specific SPL block was part of a specific vault state. Useful for auditing and trust verification. |
| Cryptographic signing of vault root | Sign the vault root hash with an author key. Enables tamper detection and attribution in multi-agent environments. |
| Content-addressable object storage | Store the Merkle tree as a git-like object store where objects are addressed by their hash. Enables deduplication and efficient diffing across vault snapshots. |
| Semantic drift detection | Use embedding similarity (not just hash equality) to detect when prose meaning has drifted even if the specific hashed blocks haven't changed. |
| Cross-vault Merkle forests | Extend the tree to span multiple vaults with a forest root. Builds on SPEC-004 sync. |
| Merkle-based cache garbage collection | Use hash references to determine which cached data is still reachable from the current vault state. Unreferenced cache entries can be pruned. |
| Block-level provenance | Extend Merkle leaves to include `^block-id` references from SPEC-001, enabling sub-file provenance resolution. |

---

## 14. Open Questions

1. **Should the Merkle tree include non-Markdown files (images, PDFs)?** The current design covers `.md` and `.spl` files only. Binary files could be included as opaque hash leaves (hash the raw bytes). This would make the vault root hash a true content address for the entire vault, but adds complexity for minimal reasoning benefit. Recommendation: defer to a future iteration.

2. **Should leaf hashes include structural metadata (heading level, list type)?** The current design hashes normalised text content plus a type tag. Including structural metadata means changing a heading from `##` to `###` would change the hash. Recommendation: include it — heading level is semantically significant.

3. **How should the Merkle tree handle files that fail to parse (binary files mislabeled as .md)?** Recommendation: produce a single opaque leaf with the raw file hash. The file still contributes to the vault root but has no internal tree structure.

4. **Should drift detection use a configurable "proximity window" instead of adjacent-only?** The current design flags Warning for distance-1 siblings and Info for all others. A configurable window (e.g., "flag Warning for all siblings within 3 positions of the SPL block") might be more useful. Recommendation: start with the fixed policy, make it configurable in a follow-up based on user feedback.

5. **Should the `merkle.json` cache store full leaf trees or just roots + SPL hashes?** Full trees enable faster `merkle diff` but increase cache size. Roots + SPL hashes are compact and sufficient for the primary use cases (invalidation, drift detection). Recommendation: compact format (roots + SPL hashes) for v1, with full trees as an opt-in flag for inspection-heavy workflows.

6. **How should the system handle the transition from v1 (mtime-only) cache to v2 (mtime + hash)?** Recommendation: on first run with the new system, detect the old cache format, trigger a full rehash to populate the Merkle data, and write the new format. Subsequent runs use the two-tier strategy.

---

**END OF SPEC-006**
