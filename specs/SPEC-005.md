---
title: "SPEC-005: zetl reason — Defeasible Logic over Markdown Vaults"
version: 0.1.0
status: draft
audience: agent, human
date: 2026-02-23
---

# SPEC-005: zetl reason — Defeasible Logic over Markdown Vaults

## Information Table

| Field          | Value                                                        |
| -------------- | ------------------------------------------------------------ |
| Document ID    | SPEC-005                                                     |
| Title          | zetl reason — Defeasible Logic over Markdown Vaults          |
| Version        | 0.1.0                                                        |
| Status         | Draft                                                        |
| Author         | Agent (USDD Protocol v1.0.0)                                 |
| Date           | 2026-02-23                                                   |
| Audience       | Agent, Human                                                 |
| Trace          | USDD Agent Protocol v1.0.0                                   |
| Parent         | SPEC-001: zetl — Bi-directional Link Graph CLI               |
| Related        | SPEC-002: zetl search, SPEC-003: Agent Ergonomics            |
| Dependencies   | spindle-core (defeasible logic engine), spindle-parser (SPL)  |

---

## 1. Overview

SPEC-001 established zetl as a tool that builds a **structural graph** from `[[wikilinks]]` in Markdown files — it tells you what documents link to each other. This specification adds a second layer: a **logical graph** built from Spindle Lisp (SPL) code blocks embedded in those same documents.

Documents in a Zettelkasten don't just link to each other — they make **claims**. A note on "Redis vs Memcached" doesn't just link to [[Caching Strategy]]; it **concludes** that Redis is the better choice. A later note on [[License Audit]] might **defeat** that conclusion with new evidence. Today, these claims exist only as natural language prose. Zetl's link graph cannot distinguish "this document mentions caching" from "this document argues Redis is correct."

By embedding SPL — a formal language for defeasible (defeatable) rules — directly in Markdown code blocks, authors (human or agent) can express claims that zetl extracts, combines into a unified logical theory, and reasons over using the spindle-core engine. The result: a knowledge base that can detect contradictions across documents, explain why a conclusion holds (or doesn't), identify what knowledge is missing, and explore hypothetical scenarios — all with full provenance tracing back to source files and line numbers.

### 1.1 Core Insight

Every Zettelkasten tool in existence treats notes as **inert text connected by links**. The link graph answers "what connects to what?" but cannot answer "what follows from what?" or "which claims contradict each other?"

Luhmann's original slip-box was not just a filing system — it was a **reasoning partner** that surfaced contradictions and suggested connections. SPL-in-Markdown recaptures this by making documents active participants in a logical argument. Each note can assert facts, propose defeasible rules, and defeat claims made by other notes. The reasoning engine computes what the vault collectively concludes, explains why, and identifies gaps.

### 1.2 Design Philosophy

1. **Documents are the source of truth.** SPL blocks are embedded in Markdown files that the user already writes. No separate `.spl` files required (though they are supported). Zetl extracts but never modifies.
2. **Reasoning is whole-vault.** All SPL blocks across all documents are unioned into a single theory. This mirrors how zetl builds one link graph from all wikilinks — the theory is the logical analogue of the graph.
3. **Provenance is first-class.** Every rule, fact, and conclusion traces back to a specific file and line number. Explanation proof trees reference source documents by name. This bridges the structural graph (wikilinks) and the logical graph (SPL).
4. **Defeasible, not monotonic.** Claims can be defeated by stronger claims in other documents. This is the correct model for evolving knowledge bases where new information overrides old. Monotonic logic (Datalog, Prolog) cannot express "this was true until that note defeated it."
5. **Agent-first, human-friendly.** All reasoning output is structured JSON by default. Proof trees, conflict reports, and gap analyses are machine-parseable. Table and natural-language formats serve human users.

### 1.3 Scope

**In scope:**

- Extraction of SPL from fenced code blocks (` ```spl `) in Markdown files
- Support for standalone `.spl` files in the vault
- Combination of all extracted SPL into a unified spindle-core `Theory`
- Provenance tracking: mapping each rule/fact to source file, line number
- Reasoning: computing conclusions (+D, +d, -D, -d) over the combined theory
- Explanation: proof trees with document-level provenance
- Query operators: `what-if`, `why-not`, `require` (abductive reasoning)
- Conflict detection: identifying logical contradictions across documents
- Validation: detecting ill-formed SPL blocks during vault indexing
- CLI subcommands under `zetl reason`
- Integration with the existing link graph for cross-referencing

**Out of scope:**

- Modifying documents (zetl remains read-only)
- Real-time collaborative reasoning (future SPEC, builds on SPEC-004 sync)
- Embedding-based semantic similarity for SPL literals (future SPEC)
- Trust-weighted reasoning across authors (future SPEC, builds on spindle-core trust module)
- Temporal reasoning with Allen interval algebra (future SPEC, spindle-core supports it but the UX needs design)
- Variable grounding across documents (first-order logic; initial implementation is propositional only; grounding support added in a subsequent iteration)
- Process mining from reasoning history (future SPEC, builds on hence's mining infrastructure)

---

## 2. User Profiles

### 2.1 Agent Operator — Knowledge Builder

```
Role: LLM agent building a research knowledge base
Goals:
  - Write notes with both wikilinks (structure) and SPL (claims)
  - Validate that new claims don't contradict existing conclusions
  - Discover what knowledge gaps remain before a conclusion can be drawn
  - Get explanations for why a conclusion holds or doesn't
Constraints:
  - Requires structured JSON output for programmatic consumption
  - Invokes CLI commands non-interactively
  - May write SPL blocks as part of note creation workflows
  - Must handle structured error responses for invalid SPL
Daily workflow:
  1. Create a research note with [[wikilinks]] and an ```spl block
  2. Run `zetl reason status` to see what the vault collectively concludes
  3. Run `zetl reason explain "decided-use-redis"` to get the proof chain
  4. Run `zetl reason require "ready-for-production"` to find missing premises
  5. Run `zetl reason what-if "verified-load-test"` to explore consequences
  6. Run `zetl reason conflicts` to check for unresolved contradictions
  7. Run `zetl check` to validate both link integrity and SPL syntax
```

### 2.2 Human Knowledge Worker — Decision Documenter

```
Role: Researcher or engineer documenting decisions with formal justification
Goals:
  - Write decision documents where conclusions are formally expressed
  - See which decisions are currently "active" (not defeated)
  - Understand why a previous decision was overridden
  - Explore "what if we changed this assumption?" scenarios
Constraints:
  - Writes notes in Obsidian, Logseq, or a text editor
  - SPL blocks are optional — most notes have only wikilinks
  - Prefers table/natural-language output for reasoning results
  - Needs human-readable proof explanations, not raw logic
Daily workflow:
  1. Write an architecture decision record with an ```spl block
  2. Run `zetl reason status -f table` to see all active conclusions
  3. Run `zetl reason explain "decided-use-redis" -f table` to see proof
  4. Months later, write a new note that defeats the Redis decision
  5. Run `zetl reason explain "decided-use-redis" -f table` to see it's now defeated
  6. The system shows: "defeated by rule d1 in [[License Audit]] line 12"
```

### 2.3 Agent Team — Multi-Agent Research Coordination

```
Role: Multiple LLM agents contributing to a shared knowledge base (via hence)
Goals:
  - Each agent writes research findings as notes with SPL claims
  - The vault's theory is the union of all agents' claims
  - Hence coordinates which agent researches what; zetl validates the logic
  - Agents can check whether their findings conflict with prior research
Constraints:
  - Agents write concurrently to the same vault (append-only, no lock contention)
  - SPL `(claims ...)` blocks attribute facts to specific agents
  - Hence's context assembly pipeline can inject relevant conclusions into agent prompts
Daily workflow:
  1. Hence assigns "research caching options" to agent-A
  2. Agent-A creates notes with SPL claims about Redis
  3. Agent-A runs `zetl reason status` to validate no conflicts
  4. Hence assigns "research license constraints" to agent-B
  5. Agent-B creates a note with a defeater for the Redis conclusion
  6. Agent-B runs `zetl reason conflicts` to report the contradiction
  7. Hence assigns "resolve caching decision" to agent-C
  8. Agent-C reads both proofs, writes a resolution note with updated SPL
```

### 2.4 Happy Paths

```
Happy Path: Agent Validates a New Claim

Preconditions:
  - Vault has existing notes with SPL blocks asserting various facts and rules
  - Agent has created a new note with an ```spl block containing a new claim
Steps:
  1. `zetl reason status -d ./vault`
     → Returns all current conclusions with provenance
  2. Agent checks whether its new claim conflicts with existing conclusions
  3. `zetl reason explain "new-claim" -d ./vault`
     → Returns proof tree showing which rules/facts support the claim
     → Proof tree references source documents by name and line
  4. `zetl reason conflicts -d ./vault`
     → Returns empty list (no unresolved contradictions)
Postconditions:
  - Agent is confident its new claim is consistent with the vault's theory
Failure modes:
  - SPL syntax error in agent's note → `zetl reason status` returns structured
    diagnostic with file, line, column, and error message
  - New claim contradicts existing conclusion → `conflicts` reports the
    contradiction with both sides' provenance
```

```
Happy Path: Human Explores Why a Decision Changed

Preconditions:
  - Vault contains "Architecture Decision: Redis" from January with SPL
  - Vault contains "License Audit Results" from February with a defeater
Steps:
  1. `zetl reason explain "decided-use-redis" -f table`
     → Shows: "-d decided-use-redis (defeasibly not provable)"
     → Proof: "Rule r-prefer-redis in [[Redis vs Memcached]]:14 would prove it"
     → Defeat: "Defeated by d-license-risk in [[License Audit]]:8"
     → Because: "discovered-license-risk is +D (fact in [[License Audit]]:7)"
  2. User understands: the January decision was valid at the time but is now
     defeated by February's audit findings
  3. `zetl reason what-if "(not discovered-license-risk)" -f table`
     → Shows: if the license risk were removed, decided-use-redis would
       become +d again
Postconditions:
  - User understands the full reasoning chain across documents and time
Failure modes:
  - Literal not found → structured error with "did you mean?" suggestions
    from existing literals
```

---

## 3. SPL-in-Markdown Specification

### 3.1 Embedding Syntax

SPL is embedded in Markdown via fenced code blocks with the `spl` language tag:

````markdown
# My Research Note

This note argues that Redis is the better caching choice.
See [[Performance Benchmarks]] and [[Requirements Doc]].

```spl
(given evaluated-redis)
(given redis-supports-persistence)

(normally r-prefer-redis
  (and evaluated-redis redis-supports-persistence)
  decided-use-redis)
```

Further prose continues here...
````

**Rules:**

1. The opening fence MUST be ` ```spl ` (backtick fence with `spl` language tag). Tilde fences (`~~~spl`) are also accepted.
2. Multiple `spl` blocks per document are permitted. They are concatenated in document order and treated as a single theory fragment from that file.
3. SPL blocks inside HTML comments (`<!-- ```spl ... ``` -->`) SHALL be ignored (consistent with SPEC-001 §3.3).
4. SPL blocks inside other fenced code blocks (nested fences) SHALL be ignored.
5. Standalone `.spl` files in the vault are also indexed. They are treated identically to extracted SPL blocks but with the entire file content as the theory fragment.

### 3.2 Supported SPL Subset (v1)

The initial implementation supports the **propositional** subset of SPL:

| Construct | Syntax | Supported |
| --- | --- | --- |
| Facts | `(given literal)` | Yes |
| Strict rules | `(always label antecedent consequent)` | Yes |
| Defeasible rules | `(normally label antecedent consequent)` | Yes |
| Defeaters | `(except label antecedent consequent)` | Yes |
| Conjunction | `(and a b c)` | Yes |
| Negation | `(not literal)` | Yes |
| Superiority | `(prefer r1 r2)` | Yes |
| Superiority chains | `(prefer r3 r2 r1)` | Yes |
| Metadata | `(meta label :key "value")` | Yes |
| Claims blocks | `(claims source :at "..." ...)` | Yes |
| Comments | `; line comment` | Yes |
| Variables | `(normally r1 (parent ?x ?y) ...)` | No (future) |
| Temporal | `(during ...)` | No (future) |
| Modal | `(must ...)`, `(may ...)`, `(forbid ...)` | No (future) |
| Imports | `(import "path.spl")` | No (future) |

**Rationale for propositional-only in v1:** Grounding (variable instantiation) requires collecting all ground facts across all documents before reasoning. This is feasible but adds pipeline complexity. The propositional subset covers the primary use case — documenting decisions, claims, and their interactions — without grounding overhead. Variable support is planned for SPEC-005.1.

### 3.3 Literal Naming Conventions

To enable meaningful cross-referencing between the link graph and the logical theory, literals SHOULD follow a kebab-case convention with semantic prefixes:

| Prefix | Meaning | Example |
| --- | --- | --- |
| `decided-` | A decision or conclusion | `decided-use-redis` |
| `discovered-` | A finding or observation | `discovered-license-risk` |
| `given-` or bare | An assumed or known fact | `evaluated-redis`, `(given api-needs-pagination)` |
| `ready-` | A readiness condition | `ready-for-production` |
| `verified-` | A validated claim | `verified-load-test-passing` |
| `blocked-by-` | A blocking condition | `blocked-by-missing-credentials` |
| `needs-` | A requirement or dependency | `needs-security-review` |

These conventions are advisory, not enforced. Any valid SPL literal name is accepted.

### 3.4 Cross-Referencing: Wikilinks and SPL

A document may contain both wikilinks and SPL. The two systems are complementary:

- **Wikilinks** express structural relationships: "this document references that document."
- **SPL** expresses logical relationships: "this claim supports/defeats that conclusion."

Zetl builds both graphs from the same scan pass. Cross-referencing enables queries like:

- "Show me all documents that are both linked to [[Caching Strategy]] AND contribute facts about `decided-use-redis`" — intersection of the link graph and the theory provenance.
- "Which documents in the backlink chain of [[Architecture Decision]] contain defeated conclusions?" — graph traversal filtered by reasoning state.

These cross-referencing queries are exposed as flags on existing commands and on the new `reason` commands (see §5).

---

## 4. Requirements

### 4.1 Functional Requirements

```
REQ-026: SPL Extraction from Markdown

The system SHALL extract SPL content from all fenced code blocks tagged
`spl` (or `spindle`) in Markdown files during vault indexing, producing
for each block:
  - Source file path (relative to vault root)
  - Start line number (1-indexed, of the opening fence)
  - End line number (of the closing fence)
  - Raw SPL text content (between the fences)

SPL blocks inside HTML comments, nested code blocks, and YAML frontmatter
SHALL be ignored (consistent with SPEC-001 §3.3 exclusion rules).

Standalone `.spl` files in the vault SHALL also be indexed, with the
entire file content treated as a single SPL fragment.

FOR all user roles
WITH extraction occurring during the same scan pass as wikilink parsing
AND no modification to the source files.

Trace:
- TEST-026
- CON-012
```

```
REQ-027: Theory Construction from Extracted SPL

The system SHALL combine all extracted SPL fragments into a single
spindle-core Theory by:
  a) Parsing each fragment with spindle-parser
  b) Collecting all facts, rules, defeaters, and superiority relations
  c) Tracking provenance: each rule/fact is annotated with its source
     file path and line number (offset from the SPL block's start line)
  d) Detecting and reporting parse errors with file-level provenance

If any SPL block contains a parse error, the system SHALL:
  - Report the error with file, line, column, and diagnostic message
  - Exclude that block from the theory (partial theories are permitted)
  - Continue processing all other SPL blocks

FOR all user roles
WITH the combined theory available for reasoning queries
AND provenance preserved through all downstream operations.

Trace:
- TEST-027
- CON-012
```

```
REQ-028: Vault Reasoning

The system SHALL compute defeasible logic conclusions over the combined
theory using spindle-core's standard DL(d) reasoning algorithm,
producing for each literal in the theory:
  - Conclusion type: +D (definitely provable), -D (definitely not provable),
    +d (defeasibly provable), -d (defeasibly not provable)
  - Provenance: the rule(s) and fact(s) that contributed to the conclusion,
    each with source file and line number

The system SHALL expose these conclusions via the `zetl reason status`
subcommand.

FOR all user roles
WITH output in JSON (default) or table format.

Trace:
- TEST-028
- CON-012
```

```
REQ-029: Explanation with Document Provenance

The system SHALL provide proof-tree explanations for any literal in
the theory, showing:
  - The conclusion type (+D, +d, -D, -d)
  - The rule chain that derives the conclusion (for +D/+d)
  - The defeat chain that blocks the conclusion (for -d)
  - For each rule/fact in the chain: the source document name, file
    path, and line number

Explanations SHALL be formatted as:
  - JSON proof trees (default, for agent consumption)
  - Natural language text (for human consumption, via -f table)

FOR all user roles
WITH explanations referencing source documents by page name (the same
name used in wikilinks) for cross-referencing.

Trace:
- TEST-029
- CON-013
```

```
REQ-030: Hypothetical Reasoning (what-if)

The system SHALL support hypothetical queries that temporarily add
facts or rules to the theory and compute what changes:
  - `zetl reason what-if "<spl-facts>" [--goal <literal>]`
  - Returns: new conclusions, changed conclusions, and newly provable/
    defeated literals compared to the base theory

Hypothetical additions SHALL NOT modify the vault or the cached theory.

FOR all user roles
WITH output showing the delta between base and hypothetical conclusions.

Trace:
- TEST-030
- CON-014
```

```
REQ-031: Failure Explanation (why-not)

The system SHALL explain why a literal is NOT provable:
  - `zetl reason why-not "<literal>"`
  - Returns: which rules could prove it, what body literals are missing
    or failed, and which defeaters are blocking it

Each missing/failed literal SHALL include the document(s) that would
need to assert it.

FOR all user roles
WITH output structured for both agent parsing and human reading.

Trace:
- TEST-031
- CON-015
```

```
REQ-032: Knowledge Gap Detection (require)

The system SHALL support abductive queries that identify what facts
would need to be added to make a goal literal provable:
  - `zetl reason require "<literal>"`
  - Returns: one or more sets of facts that, if added, would make
    the literal defeasibly provable

This enables agents to identify what research or documentation is
missing before a conclusion can be drawn.

FOR all user roles
WITH each required fact annotated with which rule needs it and in
which document that rule is defined.

Trace:
- TEST-032
- CON-016
```

```
REQ-033: Conflict Detection

The system SHALL identify logical conflicts in the vault's theory:
  a) Literals where both `p` and `~p` have applicable rules but no
     superiority relation resolves the conflict (ambiguity)
  b) Rules that fire but are defeated, with no clear winner

For each conflict, the system SHALL report:
  - The contested literal
  - The competing rules (with source document provenance for each)
  - Whether a superiority relation exists
  - Suggested resolution: which document would need a `(prefer ...)`
    declaration

FOR all user roles
WITH output via `zetl reason conflicts`.

Trace:
- TEST-033
- CON-017
```

```
REQ-034: SPL Validation in Check

The system SHALL extend the existing `zetl check` command to include
SPL syntax validation alongside dead links, orphans, and wikilink
syntax errors.

SPL diagnostics SHALL include:
  - Parse errors (malformed SPL syntax)
  - Undefined rule labels in superiority relations
  - Duplicate rule labels across documents
  - Rules with body literals that appear nowhere as a head or fact
    (potential typos, reported as warnings)

Diagnostics SHALL include file path, line number, and diagnostic message.

A `--spl` flag SHALL filter check output to SPL diagnostics only.
The existing `--fail-on` flag SHALL apply to SPL diagnostics.

FOR all user roles
WITH SPL diagnostics integrated into the existing check output format.

Trace:
- TEST-034
- CON-004 (extends)
```

```
REQ-035: Theory Export

The system SHALL provide an export of the combined theory as:
  a) Raw SPL (all extracted fragments concatenated with provenance comments)
  b) JSON representation of the theory (rules, facts, superiority, conclusions)

This enables external tools (including hence) to consume the vault's
theory programmatically.

FOR all user roles
WITH output via `zetl reason export`.

Trace:
- TEST-035
- CON-018
```

```
REQ-036: Cross-Reference — Graph and Theory

The system SHALL support querying the intersection of the link graph
and the logical theory:
  a) `zetl reason provenance "<literal>"` — show which documents
     contribute to a literal's proof, cross-referenced with the
     link graph (backlinks between those documents)
  b) `zetl links <page> --with-conclusions` — for each linked page,
     show which conclusions that page contributes to the theory
  c) `zetl backlinks <page> --with-conclusions` — for each backlinking
     page, show its logical contributions

These flags are additive — they enrich existing output with reasoning
data rather than replacing it.

FOR all user roles
WITH the cross-reference requiring both the link graph and the theory
to be computed (a full pipeline run).

Trace:
- TEST-036
- CON-013 (extends), CON-003 (extends)
```

### 4.2 Non-Functional Requirements

```
NFR-010: Reasoning Performance

Reasoning over the combined theory SHALL complete in ≤ 500ms for a
vault with ≤ 1,000 SPL blocks containing ≤ 10,000 total rules/facts
UNDER single-threaded execution on commodity hardware WITH 95th
percentile.

Rationale: spindle-core is optimized for theories of this scale.
The bottleneck is SPL extraction and parsing, not reasoning itself.
```

```
NFR-011: Incremental SPL Extraction

SPL extraction SHALL be incremental — only re-parsing SPL blocks from
files whose mtime has changed since the last index, consistent with
the existing cache strategy (SPEC-001 REQ-011).

The combined theory SHALL be cached in `.zetl/theory.json` alongside
the existing `.zetl/index.json`.

Trace:
- TEST-037
```

```
NFR-012: SPL Extraction Memory

Peak memory increase from SPL extraction and theory construction
SHALL be ≤ 50MB above baseline for a vault with 1,000 SPL blocks
containing 10,000 total rules/facts.
```

```
NFR-013: Graceful Degradation

If spindle-core is not available (e.g., the `reason` feature is
compiled out), all `zetl reason` commands SHALL return a structured
error: {"error": "Reasoning engine not available. Build with --features reason", "code": 2}.

All non-reasoning commands SHALL continue to work unchanged.
```

---

## 5. Architecture

### 5.1 Technology Decisions

```
ADR-005: Embed spindle-core as a Rust Library Dependency

Status: Proposed

Context:
  zetl needs a defeasible logic engine to reason over extracted SPL.
  Three integration approaches were evaluated:

  Option A — Embed spindle-core as a Rust crate dependency:
    + Same language, zero FFI overhead
    + Compiled into the same binary
    + Direct access to Theory, Reasoner, and Explanation APIs
    + spindle-core is already a library crate designed for embedding
    - Adds ~2MB to binary size (spindle-core + spindle-parser)
    - Couples zetl to spindle-core's API stability

  Option B — Shell out to spindle-cli:
    + Zero coupling, separate binary
    + Can upgrade spindle independently
    - Process spawn overhead per query (~50ms)
    - Serialization/deserialization at the boundary
    - User must install spindle-cli separately

  Option C — Use spindle-wasm:
    + Sandboxed execution
    - WASM overhead, limited to spindle-wasm's API surface
    - Unnecessary complexity for a Rust-to-Rust integration

Decision:
  Implement Option A — embed spindle-core and spindle-parser as Cargo
  dependencies behind a `reason` feature flag.

Rationale:
  - Same-language embedding eliminates serialization overhead
  - Feature flag keeps the dependency optional: `cargo build` without
    `--features reason` produces the current binary with no spindle code
  - spindle-core's Theory, Rule, Literal, and Explanation types map
    directly to zetl's data model
  - The ~2MB binary size increase (NFR-004 allows up to 10MB) is acceptable

Consequences:
  + Zero-overhead reasoning: Theory construction and reasoning are
    in-process function calls
  + Single binary distribution (when feature is enabled)
  + Direct access to spindle-core's query operators (what-if, why-not,
    abduce, explain) without building a CLI integration layer
  - zetl's compile time increases when the reason feature is enabled
  - spindle-core API changes require zetl updates
```

```
ADR-006: Theory Caching Strategy

Status: Proposed

Context:
  Reasoning over the combined theory requires:
  1. Extracting SPL from all Markdown files (I/O bound)
  2. Parsing SPL (CPU bound, ~1ms per block)
  3. Constructing the Theory (CPU bound, negligible for <10K rules)
  4. Running the reasoner (CPU bound, ~10-100ms for typical vaults)

  For interactive use (agent calling zetl repeatedly), steps 1-3
  should be cached. Step 4 is fast enough to re-run each time.

  Options:
  A. Cache the parsed Theory to .zetl/theory.json:
     + Skip steps 1-3 on cache hit
     + Consistent with existing index.json caching
     - Theory serialization format must be designed
     - Cache invalidation when any file with SPL changes

  B. Cache only the extracted SPL fragments:
     + Simpler cache format (raw SPL text + provenance)
     + Still need to re-parse and reason each time
     - Slower: parsing 1,000 blocks takes ~1 second

  C. No caching (always re-extract and reason):
     + Simplest implementation
     + Always correct
     - Full pipeline on every query (2-3 seconds for large vaults)

Decision:
  Implement Option A — cache the parsed Theory. Use the same mtime-based
  invalidation strategy as the link index (SPEC-001 REQ-011). If any
  file containing an SPL block has changed, invalidate the theory cache
  and rebuild from all SPL blocks.

  Future optimization: track SPL-containing files separately so that
  changes to files without SPL blocks don't trigger theory rebuilds.

Consequences:
  + Repeated reason queries are fast (~100ms: load cache + reason)
  + Consistent caching model with existing index
  - Cache format is an additional maintenance surface
  - Full-vault re-extraction on any SPL file change (acceptable for v1)
```

```
ADR-007: Provenance Model — Source Mapping

Status: Proposed

Context:
  When spindle-core derives a conclusion, the explanation references
  rule labels and literal names. To be useful in a vault context,
  these must map back to the source document.

  Spindle-core's Meta system allows attaching arbitrary metadata to
  rules. We use this to attach provenance:

  For each rule/fact extracted from a Markdown file:
    meta.set("_source_file", "concepts/Redis vs Memcached.md")
    meta.set("_source_line", "14")
    meta.set("_source_page", "Redis vs Memcached")

  For each rule/fact from a standalone .spl file:
    meta.set("_source_file", "theories/caching.spl")
    meta.set("_source_line", "3")
    meta.set("_source_page", "caching")

Decision:
  Attach provenance metadata to every rule and fact during theory
  construction using spindle-core's meta API. Use underscore-prefixed
  keys (_source_file, _source_line, _source_page) to distinguish
  system metadata from user-authored metadata.

Consequences:
  + Explanation proof trees automatically include document references
  + Cross-referencing between link graph and theory is a metadata lookup
  + No changes to spindle-core required (uses existing meta API)
  - Slight memory overhead for provenance metadata (~100 bytes per rule)
```

### 5.2 Component Architecture

```
                         ┌──────────────┐
                         │     CLI      │
                         │  (commands)  │
                         └──────┬───────┘
                                │
          ┌─────────────────────┼─────────────────────┐
          │                     │                      │
   ┌──────▼──────┐       ┌─────▼──────┐        ┌─────▼──────────┐
   │   Scanner    │       │   Graph    │        │    Reason      │  NEW
   │              │       │   Engine   │        │    Engine      │
   │ - file walk  │       │            │        │                │
   │ - parse md   │       │ - build    │        │ - extract spl  │
   │ - extract    │       │ - query    │        │ - parse (spindle│
   │   wikilinks  │       │ - path     │        │   -parser)     │
   │ - extract    │◄──────│ - stats    │        │ - build theory │
   │   spl blocks │       │            │        │ - reason       │
   │ - validate   │       └────────────┘        │   (spindle-    │
   └──────┬───────┘                             │   core)        │
          │                                     │ - explain      │
          │         ┌───────────────┐            │ - what-if      │
          │         │   SimHash     │            │ - why-not      │
          │         │   Index       │            │ - require      │
          │         └───────────────┘            │ - conflicts    │
          │                                     └────────┬───────┘
          │                                              │
          └─────────────────┬────────────────────────────┘
                            │
                     ┌──────▼───────┐
                     │    Cache     │
                     │  .zetl/      │
                     │  index.json  │
                     │  theory.json │  NEW
                     └──────────────┘
```

**Scanner (extended)** — During the existing Markdown scan pass, the scanner now also identifies `spl`-tagged code blocks and extracts their content with provenance metadata. This is the dual of wikilink extraction: wikilinks are extracted from *outside* code blocks; SPL is extracted from *inside* specifically-tagged code blocks. The scanner also identifies standalone `.spl` files via the file walk.

**Reason Engine (new)** — Consumes extracted SPL fragments, parses them via spindle-parser, constructs a spindle-core `Theory` with provenance metadata, runs the reasoner, and exposes query methods. The engine is a thin integration layer between zetl's scanner output and spindle-core's API.

**Cache (extended)** — Adds `.zetl/theory.json` alongside the existing `.zetl/index.json`. The theory cache stores the serialized Theory and its conclusions. Invalidation follows the same mtime strategy.

### 5.3 Data Model

```rust
/// An extracted SPL block from a Markdown file or standalone .spl file
struct SplBlock {
    source_file: PathBuf,   // relative to vault root
    source_page: String,    // page name (filename sans extension)
    start_line: u32,        // 1-indexed, line of opening ``` fence (or 1 for .spl files)
    end_line: u32,          // line of closing ``` fence (or last line for .spl files)
    content: String,        // raw SPL text between fences
}

/// A parsed and indexed SPL fragment with provenance
struct SplFragment {
    block: SplBlock,
    rules: Vec<ProvenancedRule>,
    facts: Vec<ProvenancedFact>,
    superiority: Vec<SuperiorityRelation>,
    diagnostics: Vec<Diagnostic>,  // parse errors for this block
}

/// A rule with source provenance
struct ProvenancedRule {
    label: String,
    rule_type: RuleType,           // Strict, Defeasible, Defeater
    body: Vec<Literal>,
    head: Literal,
    source_file: PathBuf,
    source_line: u32,              // absolute line in the Markdown file
    source_page: String,
}

/// A fact with source provenance
struct ProvenancedFact {
    literal: Literal,
    source_file: PathBuf,
    source_line: u32,
    source_page: String,
}

/// A conclusion with its proof provenance
struct ProvenancedConclusion {
    literal: String,
    conclusion_type: ConclusionType,  // +D, -D, +d, -d
    proof_sources: Vec<ProofSource>,  // documents contributing to this conclusion
}

struct ProofSource {
    page: String,
    path: PathBuf,
    line: u32,
    rule_label: Option<String>,
    contribution: String,  // "fact", "strict_rule", "defeasible_rule", "defeater", "superiority"
}

enum ConclusionType {
    DefinitelyProvable,      // +D
    DefinitelyNotProvable,   // -D
    DefeasiblyProvable,      // +d
    DefeasiblyNotProvable,   // -d
}
```

### 5.4 SPL Extraction Algorithm

The scanner's existing Markdown pass already computes exclusion ranges (frontmatter, code blocks, inline code, HTML comments) for wikilink extraction. SPL extraction inverts the code-block logic:

1. During the file scan, when a fenced code block with tag `spl` or `spindle` is encountered:
   a. Record the start line (the ` ``` ` line).
   b. Capture all content until the closing fence.
   c. Record the end line.
   d. Create an `SplBlock` with the content and provenance.
2. For standalone `.spl` files (detected by extension during file walk):
   a. Read the entire file content.
   b. Create an `SplBlock` with start_line=1, end_line=last_line.
3. All `SplBlock`s are collected and passed to the Reason Engine.

**Line number mapping:** When spindle-parser reports a parse error at "line 3" within an SPL block, the absolute line in the Markdown file is `block.start_line + 3` (since start_line points to the fence line, and SPL content starts on the next line, the mapping is `block.start_line + parser_line`).

### 5.5 Theory Construction Pipeline

```
SplBlock[]  (from scanner)
    │
    ▼
┌───────────────┐
│ Parse          │  spindle-parser: SPL text → Rules, Facts, Superiority
│                │  Errors → Diagnostics with provenance
└───────┬───────┘
        │
        ▼
┌───────────────┐
│ Annotate       │  Attach _source_file, _source_line, _source_page
│ Provenance     │  metadata to each Rule and Fact via spindle-core Meta API
└───────┬───────┘
        │
        ▼
┌───────────────┐
│ Combine        │  Union all fragments into a single Theory
│                │  Detect duplicate rule labels across documents (warn)
└───────┬───────┘
        │
        ▼
┌───────────────┐
│ Validate       │  Check for:
│                │  - Undefined labels in (prefer ...) relations
│                │  - Body literals with no matching head/fact (warn)
│                │  - Duplicate rule labels (warn with provenance)
└───────┬───────┘
        │
        ▼
┌───────────────┐
│ Reason         │  spindle-core StandardReasoner: Theory → Conclusions
│                │  +D, -D, +d, -d for every literal
└───────┬───────┘
        │
        ▼
┌───────────────┐
│ Annotate       │  For each Conclusion, trace back through the proof
│ Conclusions    │  to collect ProofSource[] from rule metadata
└───────┬───────┘
        │
        ▼
  ProvenancedConclusion[]  (ready for CLI output)
```

---

## 6. Contract Specifications (CLI Interface)

### 6.1 Reason Subcommand Group

All `reason` commands operate on the vault's combined SPL theory. They require a full pipeline run (scan → parse → reason) unless the theory cache is valid.

```
CON-012: zetl reason status

zetl reason status [OPTIONS]

Show all conclusions derived from the vault's SPL theory.

Options:
  --positive       Show only provable conclusions (+D, +d)
  --negative       Show only non-provable conclusions (-D, -d)
  --definite       Show only definite conclusions (+D, -D)
  --defeasible     Show only defeasible conclusions (+d, -d)
  --literal <PAT>  Filter to literals matching pattern (glob-style)

Exit codes:
  0  Theory built and conclusions computed
  1  No SPL blocks found in vault
  2  SPL parse errors prevented theory construction

Example output (JSON):
{
  "theory": {
    "facts": 12,
    "rules": 8,
    "defeaters": 2,
    "superiority_relations": 3,
    "source_files": 5
  },
  "conclusions": [
    {
      "literal": "decided-use-redis",
      "type": "-d",
      "sources": [
        {
          "page": "Redis vs Memcached",
          "path": "decisions/Redis vs Memcached.md",
          "line": 14,
          "rule_label": "r-prefer-redis",
          "contribution": "defeasible_rule"
        }
      ],
      "defeated_by": [
        {
          "page": "License Audit",
          "path": "audits/License Audit.md",
          "line": 8,
          "rule_label": "d-license-risk",
          "contribution": "defeater"
        }
      ]
    },
    {
      "literal": "discovered-license-risk",
      "type": "+D",
      "sources": [
        {
          "page": "License Audit",
          "path": "audits/License Audit.md",
          "line": 7,
          "rule_label": null,
          "contribution": "fact"
        }
      ]
    }
  ],
  "summary": {
    "definitely_provable": 8,
    "defeasibly_provable": 3,
    "defeasibly_not_provable": 2,
    "conflicts": 0
  },
  "diagnostics": []
}

Implements:
- REQ-028

Verified by:
- TEST-028
```

```
CON-013: zetl reason explain

zetl reason explain <LITERAL> [OPTIONS]

Show the proof tree for a literal — why it is or isn't provable.

Arguments:
  <LITERAL>  The literal to explain (e.g., "decided-use-redis")

Options:
  --depth <N>      Max proof tree depth [default: 10]
  --format <FMT>   Output: json (default), table, natural, dot

Exit codes:
  0  Explanation generated
  1  Literal not found in theory (suggest similar literals)

Example output (JSON):
{
  "literal": "decided-use-redis",
  "conclusion": "-d",
  "explanation": {
    "type": "defeated",
    "would_prove": {
      "rule": "r-prefer-redis",
      "rule_type": "defeasible",
      "source": {
        "page": "Redis vs Memcached",
        "path": "decisions/Redis vs Memcached.md",
        "line": 14
      },
      "body": [
        {
          "literal": "evaluated-redis",
          "status": "+D",
          "source": {
            "page": "Redis vs Memcached",
            "path": "decisions/Redis vs Memcached.md",
            "line": 12
          }
        },
        {
          "literal": "redis-supports-persistence",
          "status": "+D",
          "source": {
            "page": "Redis vs Memcached",
            "path": "decisions/Redis vs Memcached.md",
            "line": 13
          }
        }
      ]
    },
    "defeated_by": {
      "rule": "d-license-risk",
      "rule_type": "defeater",
      "source": {
        "page": "License Audit",
        "path": "audits/License Audit.md",
        "line": 8
      },
      "body": [
        {
          "literal": "discovered-license-risk",
          "status": "+D",
          "source": {
            "page": "License Audit",
            "path": "audits/License Audit.md",
            "line": 7
          }
        }
      ]
    }
  }
}

Example output (natural language, via -f table):

  decided-use-redis is DEFEASIBLY NOT PROVABLE (-d)

  Rule r-prefer-redis in [[Redis vs Memcached]]:14 would prove it:
    IF evaluated-redis (FACT in [[Redis vs Memcached]]:12) ✓
    AND redis-supports-persistence (FACT in [[Redis vs Memcached]]:13) ✓
    THEN decided-use-redis

  BUT defeated by d-license-risk in [[License Audit]]:8:
    IF discovered-license-risk (FACT in [[License Audit]]:7) ✓
    THEN BLOCK decided-use-redis

Implements:
- REQ-029

Verified by:
- TEST-029
```

```
CON-014: zetl reason what-if

zetl reason what-if <SPL> [OPTIONS]

Hypothetically add facts/rules and show what changes.

Arguments:
  <SPL>  Inline SPL to add (e.g., "(given verified-load-test)")

Options:
  --goal <LITERAL>  Focus on a specific literal's change
  --file <PATH>     Read hypothetical SPL from a file instead of inline

Exit codes:
  0  Hypothetical reasoning completed
  2  Invalid SPL in the hypothetical

Example output (JSON):
{
  "hypothetical_additions": "(given verified-load-test)",
  "changes": [
    {
      "literal": "ready-for-production",
      "was": "-d",
      "now": "+d",
      "reason": "New fact verified-load-test satisfies body of r-ready-prod in [[Deployment Checklist]]:9"
    }
  ],
  "unchanged_count": 22,
  "new_conclusions_count": 1
}

Implements:
- REQ-030

Verified by:
- TEST-030
```

```
CON-015: zetl reason why-not

zetl reason why-not <LITERAL>

Explain why a literal is not provable.

Arguments:
  <LITERAL>  The literal to investigate

Exit codes:
  0  Explanation generated
  1  Literal not in theory

Example output (JSON):
{
  "literal": "ready-for-production",
  "provable": false,
  "blockers": [
    {
      "type": "failed_body",
      "rule": "r-ready-prod",
      "rule_source": {
        "page": "Deployment Checklist",
        "path": "processes/Deployment Checklist.md",
        "line": 9
      },
      "missing_literal": "verified-load-test",
      "explanation": "No fact or rule derives 'verified-load-test' in any document"
    },
    {
      "type": "failed_body",
      "rule": "r-ready-prod",
      "rule_source": {
        "page": "Deployment Checklist",
        "path": "processes/Deployment Checklist.md",
        "line": 9
      },
      "missing_literal": "verified-security-audit",
      "explanation": "No fact or rule derives 'verified-security-audit' in any document"
    }
  ]
}

Implements:
- REQ-031

Verified by:
- TEST-031
```

```
CON-016: zetl reason require

zetl reason require <LITERAL> [OPTIONS]

Find what facts are needed to make a literal provable.

Arguments:
  <LITERAL>  The goal literal

Options:
  --max-solutions <N>  Max abduction solutions [default: 5]
  --assume <SPL>       Assume these facts are already true

Exit codes:
  0  Solutions found
  1  No solutions exist (literal cannot be made provable)

Example output (JSON):
{
  "goal": "ready-for-production",
  "solutions": [
    {
      "required_facts": [
        {
          "literal": "verified-load-test",
          "needed_by_rule": "r-ready-prod",
          "rule_source": {
            "page": "Deployment Checklist",
            "path": "processes/Deployment Checklist.md",
            "line": 9
          }
        },
        {
          "literal": "verified-security-audit",
          "needed_by_rule": "r-ready-prod",
          "rule_source": {
            "page": "Deployment Checklist",
            "path": "processes/Deployment Checklist.md",
            "line": 9
          }
        }
      ]
    }
  ],
  "solutions_count": 1
}

Implements:
- REQ-032

Verified by:
- TEST-032
```

```
CON-017: zetl reason conflicts

zetl reason conflicts [OPTIONS]

Detect unresolved logical conflicts in the vault's theory.

Options:
  --suggest    Include suggested resolutions

Exit codes:
  0  No conflicts (or conflicts listed successfully)
  1  Conflicts found (with --fail-on-conflicts flag)

Example output (JSON):
{
  "conflicts": [
    {
      "literal": "decided-use-redis",
      "positive_rules": [
        {
          "label": "r-prefer-redis",
          "source": {
            "page": "Redis vs Memcached",
            "path": "decisions/Redis vs Memcached.md",
            "line": 14
          }
        }
      ],
      "negative_rules": [
        {
          "label": "r-prefer-memcached",
          "source": {
            "page": "Performance Review",
            "path": "reviews/Performance Review.md",
            "line": 22
          }
        }
      ],
      "has_superiority": false,
      "suggestion": "Add (prefer r-prefer-redis r-prefer-memcached) or (prefer r-prefer-memcached r-prefer-redis) to resolve"
    }
  ],
  "conflict_count": 1
}

Implements:
- REQ-033

Verified by:
- TEST-033
```

```
CON-018: zetl reason export

zetl reason export [OPTIONS]

Export the combined theory.

Options:
  --format <FMT>  Output format: spl (reconstructed SPL with provenance
                  comments), json (structured theory) [default: json]
  --with-conclusions  Include reasoning results in export

Exit codes:
  0  Always

Example output (SPL format):
; Theory extracted from vault: ./my-vault
; 5 source files, 12 facts, 8 rules, 2 defeaters
;
; --- From: decisions/Redis vs Memcached.md:12 ---
(given evaluated-redis)
; --- From: decisions/Redis vs Memcached.md:13 ---
(given redis-supports-persistence)
; --- From: decisions/Redis vs Memcached.md:14 ---
(normally r-prefer-redis
  (and evaluated-redis redis-supports-persistence)
  decided-use-redis)
; --- From: audits/License Audit.md:7 ---
(given discovered-license-risk)
; --- From: audits/License Audit.md:8 ---
(except d-license-risk discovered-license-risk (not decided-use-redis))

Implements:
- REQ-035

Verified by:
- TEST-035
```

```
CON-004 (extended): zetl check --spl

zetl check [OPTIONS]

Additional options:
  --spl            Show only SPL diagnostics (parse errors, undefined
                   labels, duplicate rules, unreachable literals)

SPL diagnostics are included in the existing output format alongside
dead links, orphans, and wikilink syntax errors.

Example output (JSON, SPL diagnostics):
{
  "dead_links": [...],
  "orphans": [...],
  "syntax_errors": [...],
  "spl_diagnostics": [
    {
      "level": "error",
      "file": "decisions/Bad Decision.md",
      "line": 18,
      "column": 5,
      "message": "SPL parse error: expected closing parenthesis, found EOF"
    },
    {
      "level": "warning",
      "file": "decisions/Redis vs Memcached.md",
      "line": 15,
      "message": "Superiority references undefined rule label 'r-nonexistent'"
    },
    {
      "level": "warning",
      "file": "reviews/Performance Review.md",
      "line": 22,
      "message": "Duplicate rule label 'r-prefer-redis' (also defined in decisions/Redis vs Memcached.md:14)"
    },
    {
      "level": "warning",
      "file": "processes/Deployment Checklist.md",
      "line": 10,
      "message": "Body literal 'verified-pentest' appears in no rule head or fact (possible typo)"
    }
  ],
  "summary": {
    "dead_links": 0,
    "orphans": 0,
    "syntax_errors": 0,
    "spl_errors": 1,
    "spl_warnings": 3
  }
}

Implements:
- REQ-034

Verified by:
- TEST-034
```

---

## 7. Test Specifications

```
TEST-026: SPL Extraction from Markdown

Scenario: Extract SPL blocks from a Markdown file
Given: A file "concepts/Caching.md" containing:
  Line 1:  # Caching
  Line 3:  Some prose about [[Redis]].
  Line 5:  ```spl
  Line 6:  (given evaluated-redis)
  Line 7:  (normally r1 evaluated-redis decided-use-redis)
  Line 8:  ```
  Line 10: More prose.
When: The scanner processes this file
Then:
  - One SplBlock is extracted
  - source_file = "concepts/Caching.md"
  - source_page = "Caching"
  - start_line = 5
  - end_line = 8
  - content = "(given evaluated-redis)\n(normally r1 evaluated-redis decided-use-redis)"

Scenario: Multiple SPL blocks in one file
Given: A file with two ```spl blocks (lines 5-8 and lines 15-18)
When: The scanner processes this file
Then: Two SplBlocks extracted, concatenated in document order

Scenario: SPL block inside HTML comment is ignored
Given: A file containing:
  <!-- ```spl
  (given secret-fact)
  ``` -->
When: The scanner processes this file
Then: No SplBlock extracted

Scenario: Standalone .spl file
Given: A file "theories/caching.spl" containing SPL
When: The scanner processes the vault
Then: One SplBlock with start_line=1, content=entire file

Verifies: REQ-026
```

```
TEST-027: Theory Construction with Provenance

Scenario: Build theory from multiple documents
Given: Two files with SPL blocks:
  - "A.md" line 5: (given bird)
  - "B.md" line 10: (normally r1 bird flies)
When: The theory is constructed
Then:
  - Theory has 1 fact (bird) and 1 rule (r1)
  - bird's provenance: file="A.md", line=6 (block start + 1)
  - r1's provenance: file="B.md", line=11

Scenario: SPL parse error in one block
Given: Three files with SPL blocks; file B has invalid syntax
When: The theory is constructed
Then:
  - Files A and C contribute to the theory
  - File B is excluded
  - A diagnostic is reported for B with file, line, message
  - Theory is partial but valid

Verifies: REQ-027
```

```
TEST-028: Vault Reasoning

Scenario: Basic defeasible reasoning across documents
Given: Three files:
  - "Birds.md" SPL: (given bird) (given penguin)
  - "Flight.md" SPL: (normally r1 bird flies)
  - "Penguins.md" SPL: (normally r2 penguin (not flies)) (prefer r2 r1)
When: `zetl reason status` is run
Then:
  - bird is +D (fact from Birds.md)
  - penguin is +D (fact from Birds.md)
  - (not flies) is +d (r2 defeats r1 via superiority)
  - flies is -d
  - Each conclusion's sources reference the correct documents

Verifies: REQ-028
```

```
TEST-029: Explanation with Document Provenance

Scenario: Explain a defeated conclusion
Given: The vault from TEST-028
When: `zetl reason explain "flies"` is run
Then:
  - conclusion: "-d"
  - explanation shows r1 in [[Flight]]:line would prove it
  - explanation shows r2 in [[Penguins]]:line defeats it
  - superiority (prefer r2 r1) referenced from [[Penguins]]

Scenario: Explain a provable conclusion
When: `zetl reason explain "bird"` is run
Then:
  - conclusion: "+D"
  - explanation shows it is a fact from [[Birds]]:line

Scenario: Literal not in theory
When: `zetl reason explain "swims"` is run
Then:
  - Exit code 1
  - Error suggests similar literals if any exist

Verifies: REQ-029
```

```
TEST-030: Hypothetical Reasoning

Scenario: Add a fact and see consequences
Given: A vault where "ready-for-production" requires "verified-load-test"
       (via a rule), but "verified-load-test" is not a fact
When: `zetl reason what-if "(given verified-load-test)"` is run
Then:
  - Shows "ready-for-production" changed from -d to +d
  - Shows the rule that now fires, with its source document

Scenario: Hypothetical doesn't modify vault
Given: Same vault
When: `zetl reason what-if "(given verified-load-test)"` is run
Then: Subsequent `zetl reason status` still shows ready-for-production as -d

Verifies: REQ-030
```

```
TEST-031: Why-Not Explanation

Scenario: Explain missing preconditions
Given: A vault where "ready-for-production" has a rule requiring
       "verified-load-test" AND "verified-security-audit", both missing
When: `zetl reason why-not "ready-for-production"` is run
Then:
  - Reports 2 blockers (both missing body literals)
  - Each blocker references the rule and its source document
  - "type" is "failed_body" for both

Scenario: Explain defeat
Given: A literal that is defeated by a superior rule
When: `zetl reason why-not "decided-use-redis"` is run
Then:
  - Reports the defeater with its source document

Verifies: REQ-031
```

```
TEST-032: Knowledge Gap Detection

Scenario: Find what's needed for a goal
Given: A vault where "ready-for-production" requires facts that don't exist
When: `zetl reason require "ready-for-production"` is run
Then:
  - Returns at least one solution with required facts
  - Each required fact identifies the rule that needs it
  - The rule references its source document

Scenario: Goal already provable
Given: A vault where "bird" is already +D
When: `zetl reason require "bird"` is run
Then:
  - Returns empty solution (no additional facts needed)
  - Message: "bird is already provable (+D)"

Scenario: Goal impossible
Given: A literal with no rules that could prove it
When: `zetl reason require "impossible-goal"` is run
Then:
  - Exit code 1
  - Message: "No rules exist that could derive 'impossible-goal'"

Verifies: REQ-032
```

```
TEST-033: Conflict Detection

Scenario: Ambiguous conflict detected
Given: Two documents:
  - "Pro.md" SPL: (normally r-yes evidence-a decided-yes)
  - "Con.md" SPL: (normally r-no evidence-b decided-no)
  where decided-yes and decided-no are not (not ...) of each other,
  but consider:
  - "Pro.md" SPL: (normally r-yes evidence-a use-redis)
  - "Con.md" SPL: (normally r-no evidence-b (not use-redis))
  - Both evidence-a and evidence-b are given facts
  - No (prefer ...) relation exists
When: `zetl reason conflicts` is run
Then:
  - Reports 1 conflict on literal "use-redis"
  - Shows r-yes from [[Pro]] and r-no from [[Con]]
  - has_superiority: false
  - suggestion includes "(prefer r-yes r-no)" or "(prefer r-no r-yes)"

Scenario: No conflicts
Given: A vault where all competing rules have superiority relations
When: `zetl reason conflicts` is run
Then:
  - conflict_count: 0

Verifies: REQ-033
```

```
TEST-034: SPL Validation in Check

Scenario: SPL parse error
Given: A file with an invalid SPL block (unclosed parenthesis)
When: `zetl check --spl` is run
Then:
  - Reports spl_diagnostic with level=error, file, line, message
  - Exit code matches --fail-on setting

Scenario: Duplicate rule label
Given: Two files each define rule "r1"
When: `zetl check --spl` is run
Then:
  - Reports warning: duplicate rule label with both file locations

Scenario: Undefined superiority label
Given: A file with (prefer r-exists r-phantom) where r-phantom is never defined
When: `zetl check --spl` is run
Then:
  - Reports warning with file and line

Scenario: Unreachable body literal
Given: A rule body references "some-literal" that appears in no head or fact
When: `zetl check --spl` is run
Then:
  - Reports warning: "Body literal 'some-literal' appears in no rule head or fact"

Verifies: REQ-034
```

```
TEST-035: Theory Export

Scenario: Export as SPL with provenance
Given: A vault with SPL blocks in 3 files
When: `zetl reason export --format spl` is run
Then:
  - Output contains all facts, rules, defeaters, superiority
  - Each item is preceded by a comment line with source file and line
  - Output is valid SPL (can be parsed by spindle-parser)

Scenario: Export as JSON
Given: Same vault
When: `zetl reason export --format json --with-conclusions` is run
Then:
  - JSON contains rules[], facts[], superiority[], conclusions[]
  - Each rule/fact has provenance fields (file, line, page)

Verifies: REQ-035
```

```
TEST-036: Cross-Reference — Graph and Theory

Scenario: Links with conclusions
Given: Page A links to pages B and C; B has SPL contributing to
       conclusion "decided-X" (+d); C has no SPL
When: `zetl links "A" --with-conclusions` is run
Then:
  - Page B entry includes conclusions: [{literal: "decided-X", type: "+d"}]
  - Page C entry includes conclusions: [] (empty)

Scenario: Provenance cross-referenced with backlinks
Given: Conclusion "decided-X" derives from rules in pages B and D;
       B and D are linked via [[wikilinks]]
When: `zetl reason provenance "decided-X"` is run
Then:
  - Shows pages B and D as contributing documents
  - Shows whether B and D link to each other (cross-reference with graph)

Verifies: REQ-036
```

```
TEST-037: Incremental Theory Cache

Scenario: Theory cache speeds up repeated queries
Given: A vault indexed once (theory cached to .zetl/theory.json)
When: `zetl reason status` is run again with no file changes
Then:
  - Completes in ≤ 50% of the initial reasoning time
  - Produces identical conclusions

Scenario: Cache invalidation on SPL file change
Given: A cached vault; one file with an SPL block is modified
When: `zetl reason status` is run
Then:
  - Theory is rebuilt from all SPL blocks
  - New conclusions reflect the change

Verifies: NFR-011
```

---

## 8. Observability

```
OBS-005: Reasoning Timing

When --verbose is specified, `zetl reason` commands SHALL emit to stderr:
  - Number of SPL blocks extracted
  - Number of source files contributing SPL
  - Total rules, facts, defeaters, superiority relations
  - SPL parse time (ms)
  - Theory construction time (ms)
  - Reasoning time (ms)
  - Total elapsed time (ms)
```

```
OBS-006: Theory Health Metrics

`zetl reason status` SHALL include a summary section reporting:
  - Total conclusions by type (+D, -D, +d, -d)
  - Number of unresolved conflicts
  - Number of SPL diagnostics (errors, warnings)
  - Number of source files contributing to the theory
to support vault health monitoring over time.
```

---

## 9. Traceability Matrix

| REQ     | CON              | TEST     | ADR     | OBS     |
| ------- | ---------------- | -------- | ------- | ------- |
| REQ-026 | CON-012          | TEST-026 | —       | OBS-005 |
| REQ-027 | CON-012          | TEST-027 | ADR-007 | OBS-005 |
| REQ-028 | CON-012          | TEST-028 | ADR-005 | OBS-006 |
| REQ-029 | CON-013          | TEST-029 | ADR-007 | —       |
| REQ-030 | CON-014          | TEST-030 | —       | —       |
| REQ-031 | CON-015          | TEST-031 | —       | —       |
| REQ-032 | CON-016          | TEST-032 | —       | —       |
| REQ-033 | CON-017          | TEST-033 | —       | —       |
| REQ-034 | CON-004 (ext)    | TEST-034 | —       | —       |
| REQ-035 | CON-018          | TEST-035 | —       | —       |
| REQ-036 | CON-013, CON-003 | TEST-036 | —       | —       |
| NFR-010 | —                | —        | ADR-005 | OBS-005 |
| NFR-011 | —                | TEST-037 | ADR-006 | —       |
| NFR-012 | —                | —        | —       | —       |
| NFR-013 | —                | —        | ADR-005 | —       |

---

## 10. Implementation Priority

### P0 — Core Pipeline

| Item | Effort | Dependencies |
| --- | --- | --- |
| SPL extraction from scanner (REQ-026) | 2 hours | Existing scanner |
| Theory construction with provenance (REQ-027) | 4 hours | spindle-core, spindle-parser |
| `reason status` command (REQ-028) | 2 hours | Theory construction |
| `check --spl` integration (REQ-034) | 2 hours | SPL extraction |

### P1 — Query Operators

| Item | Effort | Dependencies |
| --- | --- | --- |
| `reason explain` with provenance (REQ-029) | 4 hours | P0 complete |
| `reason conflicts` (REQ-033) | 2 hours | P0 complete |
| `reason why-not` (REQ-031) | 2 hours | P0 complete |

### P2 — Advanced Queries

| Item | Effort | Dependencies |
| --- | --- | --- |
| `reason what-if` (REQ-030) | 3 hours | P1 complete |
| `reason require` (REQ-032) | 3 hours | P1 complete |
| `reason export` (REQ-035) | 2 hours | P0 complete |
| Cross-referencing flags (REQ-036) | 4 hours | P1 complete |

### P3 — Caching

| Item | Effort | Dependencies |
| --- | --- | --- |
| Theory cache (NFR-011) | 3 hours | P0 complete |
| Feature flag gating (NFR-013) | 1 hour | P0 complete |

**Estimated total: ~34 hours** across all priorities.

---

## 11. Hence Integration Points

This specification is designed to compose with hence for multi-agent knowledge management. The integration points are:

### 11.1 Context Assembly (Layer 5: Topological + Logical Context)

Hence's 4-layer context model (plan meta, agent memory, repo context, live events) can be extended with a 5th layer: **vault reasoning context**. When an agent is assigned a task, hence can invoke:

```bash
zetl reason status -f json -d ./vault
zetl reason explain "relevant-literal" -f json -d ./vault
```

...and inject the conclusions and relevant proofs into the agent's prompt. This gives the agent awareness of what the knowledge base currently concludes, what's been defeated, and what gaps remain.

### 11.2 Validation as a Post-Complete Hook

Hence lifecycle hooks can invoke zetl validation after an agent completes a task:

```bash
# .hence/hooks/post-complete
zetl check --spl --fail-on warning -d ./vault
zetl reason conflicts --fail-on-conflicts -d ./vault
```

If the agent introduced a logical contradiction, the hook fails and hence can reassign the task.

### 11.3 Gap-Driven Task Generation

`zetl reason require` output can drive task creation:

```bash
# Find what's needed for the project goal
zetl reason require "ready-for-production" -f json -d ./vault
# → requires: verified-load-test, verified-security-audit

# Generate hence tasks from gaps
hence task assert plan.spl '
  (given needs-load-test)
  (normally r-test (and needs-load-test agent-tester-available) ready-load-test)
'
```

### 11.4 Theory Export to SPL Plan

`zetl reason export --format spl` produces valid SPL that can be imported into a hence plan:

```lisp
;; hence plan.spl
(import "./vault-theory.spl")  ;; exported from zetl

;; Add coordination rules on top of knowledge base conclusions
(normally r-proceed
  (and decided-use-redis verified-load-test)
  ready-for-deployment)
```

---

## 12. Literate Reasoning: The Knuth Connection

### 12.1 SPL-in-Markdown as Literate Programming

The design in this specification is a form of **literate programming** as conceived by Donald Knuth — programs (theories) are written primarily for human readers, with machine execution as a secondary concern. The Markdown prose explains *why* a claim is being made; the SPL block formalises *what* the claim is. Zetl's extraction of SPL from Markdown is analogous to **tangling** (producing runnable code from a literate source), and the proof-tree explanations with document provenance are analogous to **weaving** (producing documentation from the same source).

This parallel is not accidental. Zyedidia's [Literate](https://zyedidia.github.io/literate/) tool demonstrates the core pattern: named code blocks in Markdown-like documents that reference each other via `@{block name}`, assembled into programs by tangling. The key features that map to zetl's design:

| Literate Concept | Zetl Analogue |
| --- | --- |
| Named code blocks (`--- Block name`) | Named SPL blocks (` ```spl ` with optional label) |
| Block references (`@{block name}`) | Cross-document references via shared literal names |
| Tangling (assemble blocks into a program) | Theory construction (union all SPL into one Theory) |
| Weaving (generate docs from the same source) | Proof-tree explanations with document provenance |
| `+=` append modifier | Every SPL block is an append to the global theory |
| `:=` redefine modifier | Defeaters and superiority override prior conclusions |

### 12.2 Named SPL Blocks (Future Extension)

The literate programming model suggests an enhancement beyond the v1 design: **named, composable SPL blocks** that can reference each other across documents.

Current v1 design: all SPL blocks are anonymous fragments unioned into one theory. There's no way for one block to explicitly reference another.

Proposed extension (future SPEC):

````markdown
# Caching Strategy

```spl:caching-base
(given evaluated-redis)
(given evaluated-memcached)
(normally r-prefer-redis evaluated-redis decided-use-redis)
```

This establishes our baseline. See [[License Audit]] for constraints.
````

````markdown
# License Audit

Builds on @{caching-base} with license constraints:

```spl:caching-constrained
@{caching-base}
(given discovered-license-risk)
(except d-license discovered-license-risk (not decided-use-redis))
```
````

Here, `@{caching-base}` in the second document **includes** the first document's named SPL block by reference — exactly as Literate's `@{block name}` syntax works. Tangling resolves the reference and inlines the content before parsing.

This enables **modular theory composition**: a base theory defined in one document, extended or constrained by other documents that explicitly declare their dependencies. The dependency graph of `@{...}` references is itself a graph that zetl can visualize and validate (are there cycles? missing references?).

**Why this matters for agents:** An agent creating a new analysis can declare `@{caching-base}` to inherit the baseline assumptions, then add its own findings. The explicit reference makes the logical dependency visible — not just that the documents are wikilinked, but that the *theories* depend on each other.

### 12.3 Weaving: Proof-Enriched Documentation

The weaving direction — generating documentation from the theory — is equally valuable. A future `zetl weave` command could produce a rendered Markdown document (or static site) where:

- SPL blocks are replaced with their human-readable proof status (provable/defeated/conflicted)
- Proof trees are rendered inline below the SPL block
- Wikilinks to documents that contribute to proofs are highlighted
- Defeated conclusions are visually marked (strikethrough, warning banner)
- A "theory dashboard" page summarises all conclusions, conflicts, and gaps

This would make the vault a **self-documenting reasoning system** — the output of `zetl weave` is a readable document that shows not just what the knowledge base contains, but what it *concludes* and why.

---

## 13. Future Considerations

| Item | Rationale |
| --- | --- |
| Named SPL blocks with `@{block}` references | Literate-programming-style composition across documents; see §12.2 |
| `zetl weave` — proof-enriched documentation | Generate docs where SPL blocks show proof status; see §12.3 |
| Variable support (first-order grounding) | Enable `(normally r1 (parent ?x ?y) (ancestor ?x ?y))` across documents; requires cross-document fact collection |
| Temporal reasoning | Allen interval algebra for "this fact was true during this time period"; spindle-core already supports it |
| Trust-weighted reasoning | Attribute claims to authors/agents with credibility scores; spindle-core's trust module supports this |
| Claims blocks with signatures | Cryptographic attribution of knowledge claims; spindle-core supports `(claims ...)` with `:sig` |
| Watch mode integration | Re-reason when files change; builds on the watch mode planned in SPEC-001 §10 |
| MCP server for reasoning | Expose `reason` commands as MCP tools for direct agent invocation without shell |
| Embedding-based literal similarity | Use vector embeddings to suggest related literals (beyond exact name matching) |
| Process mining on reasoning history | Learn patterns from how theories evolve over time; builds on hence's mining infrastructure |
| Multi-vault federated reasoning | Reason across multiple vaults with scoped theories; builds on SPEC-004 sync |
| Graph visualization of theory | Render proof trees and conflict graphs as DOT/Mermaid/SVG; spindle-core has a DotFormatter |
| TUI integration | Add a "Reasoning" tab to the existing TUI showing live conclusions and proofs |

---

## 14. Open Questions

1. **Should rule labels be required or auto-generated?** SPL allows unlabeled rules (the parser assigns synthetic labels). In a multi-document vault, auto-generated labels may collide. Recommendation: encourage explicit labels (convention: `r-<page-slug>-<purpose>`) but accept unlabeled rules with auto-generated labels prefixed by source filename to avoid collisions.

2. **Should `zetl reason` build the link graph too, or only the theory?** Currently, `reason status` requires only SPL extraction and reasoning, not the full link graph. Cross-referencing (REQ-036) requires both. Recommendation: `reason status/explain/what-if/why-not/require/conflicts` build only the theory (fast); `reason provenance` and `--with-conclusions` flags trigger the full pipeline (link graph + theory).

3. **How should conflicting `(prefer ...)` declarations across documents be handled?** If document A says `(prefer r1 r2)` and document B says `(prefer r2 r1)`, this is a direct contradiction in the superiority ordering. Recommendation: report as an error in `zetl check --spl` and as a conflict in `zetl reason conflicts`. Do not attempt to auto-resolve.

4. **Should the theory cache store conclusions or just the parsed theory?** Storing conclusions avoids re-running the reasoner but risks cache staleness if the reasoner algorithm is updated. Recommendation: cache the parsed theory only; re-reason on each query (spindle-core reasoning is fast enough — <100ms for typical vaults per NFR-010).

5. **What is the maximum theory size before reasoning performance degrades?** Spindle-core is optimized for theories up to ~100,000 rules. Typical vaults will have far fewer. Recommendation: document the 10,000 rule/fact limit (NFR-010) and add a warning when the theory exceeds 50% of that.

6. **Should `zetl reason` support reading SPL from stdin?** This would allow agents to pipe SPL directly: `echo "(given new-fact)" | zetl reason what-if --stdin`. Recommendation: yes, but defer to a follow-up. The inline `<SPL>` argument to `what-if` covers the common case.

---

**END OF SPEC-005**
