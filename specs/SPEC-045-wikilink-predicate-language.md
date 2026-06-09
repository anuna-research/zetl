---
id: SPEC-045
title: "Wikilink Predicate Language — typed named edges over `[[wikilinks]]`"
version: 0.1.0-strawman
status: draft
date: 2026-06-09
audience: agent, human
related:
  - SPEC-044  # Concept Graph / SPO (sibling: the vision, UX, ratification, and frontmatter authoring surface this is the engineering companion to)
  - SPEC-001  # Link Graph CLI (the bare wikilink graph this types)
  - SPEC-005  # SPL / defeasible reasoning (named edges project to facts)
  - SPEC-032  # Hook pipeline + zetl-ext AST (the parse boundary this extends)
  - SPEC-002  # Full-Text Search (predicate becomes an index field)
plan: DESIGN-045-wikilink-predicate-language
source:
  - "Wikilinks and Named Edges — Agent Reference Guide (Christopher Allen): https://gist.github.com/ChristopherA/151aefa6a6bde1ce4fa6b1182656cebe"
revision_notes:
  - v0.1.0 (initial strawman): first-pass design drafted from the
    Christopher Allen "Wikilinks and Named Edges" reference guide and a
    single design-conversation exchange, before the Phase 1 surveys,
    synthetic-user runs, and cross-model adversarial review called for by
    DESIGN-045-wikilink-predicate-language. Establishes the
    `predicate::[[Target]]` named-edge grammar, the additive `Wikilink`
    AST extension, the curated-folksonomy vocabulary file, the `zetl
    edges` query surface, and the named-edge → SPL fact projection.
---

# SPEC-045: Wikilink Predicate Language

> **Strawman notice.** This document is a first-pass design drafted from
> the [[Wikilinks and Named Edges]] reference guide and a single
> design-conversation exchange, *before* the Phase 1 surveys,
> synthetic-user runs, and cross-model adversarial review called for by
> [[DESIGN-045-wikilink-predicate-language]]. Per [[PROTO-001]]
> Constitutional Principle 11 ([[Anti-Slop Bias]]), treat every clause as
> carrying hidden debt until adversarial review proves otherwise. Sections
> marked **`[Provisional]`** are placeholders for grounded findings. The
> document reaches `0.1.0` (non-strawman) only after the Phase 1 + Phase 2
> quality gates of [[PROTO-001]] pass.
>
> **Sibling notice.** [[SPEC-044]] (Concept Graph — SPO Relations,
> Emergent Vocabulary, In-Situ Ratification) is the **vision/UX spec** for
> this same capability and was authored independently against the same
> [[Wikilinks and Named Edges]] guide. SPEC-045 is its **engineering
> companion** — it owns the parse → graph → reasoner → CLI machine layer
> (grammar, AST, contracts, tests, threat model); SPEC-044 owns the
> authoring UX (in-situ ratification), the learning-system frame, and
> vocabulary governance philosophy. Where the two diverge — the authoring
> surface and the telos of annotations — this spec **defers to SPEC-044**
> (see [[#1.2 Relationship to SPEC-044]]). The two MUST be reconciled
> before either leaves strawman; the reconciliation is the first task of
> [[DESIGN-045-wikilink-predicate-language]].

## Information Table

| Field        | Value                                                                       |
| ------------ | --------------------------------------------------------------------------- |
| Document ID  | [[SPEC-045-wikilink-predicate-language\|SPEC-045]]                           |
| Title        | Wikilink Predicate Language — typed named edges over `[[wikilinks]]`         |
| Version      | 0.1.0-strawman                                                              |
| Status       | Draft (strawman; pending [[DESIGN-045-wikilink-predicate-language]] execution) |
| Author       | Agent (Claude Opus 4.8, [[PROTO-001\|USDD Agent Protocol]] v1.8.0)           |
| Date         | 2026-06-09                                                                  |
| Audience     | Agent, Human                                                                |
| Trace        | [[PROTO-001]] §Phase 1, §Phase 2, §LangSec, §AI Trust Boundaries            |
| Source       | [[Wikilinks and Named Edges]] (Christopher Allen reference guide)            |
| Sibling      | [[SPEC-044]] Concept Graph / SPO (vision + UX + frontmatter authoring surface) |
| Related      | [[SPEC-001]] Link Graph, [[SPEC-005]] SPL, [[SPEC-032]] AST/Hooks, [[SPEC-002]] Search |
| Plan         | [[DESIGN-045-wikilink-predicate-language]]                                   |
| Feature Gate | core (parse + graph); SPL projection under `--features reason`               |
| Review tier  | Tier 2 (core graph semantics + a new trust boundary: page-authored facts)   |

---

## 1. Overview

### 1.1 Problem

zetl's [[Link Graph]] ([[SPEC-001]]) records **that** two pages are
connected — a `[[wikilink]]` from page A to page B becomes a directed
edge — but it records nothing about **how** they are connected. Every
edge is the same colour. An edge built from a citation, a refutation, a
"see also", a supersession, and a provenance trail are indistinguishable
once they land in [[graph.rs|`LinkGraph`]].

This flattening is lossy in exactly the place where an agent traversing
the graph needs discrimination. As the [[Wikilinks and Named Edges]]
guide puts it: *"the label is where the power lives — they let an agent
distinguish a citation from a counterargument from a loose
association."* An untyped backlink panel that lists forty incoming links
gives a reader forty undifferentiated obligations to chase. A typed one
("**contradicted by** 2 · **derived from** 1 · **see also** 37") lets
the reader spend their reading budget on the two edges that change the
meaning of the page.

Authors already reach for this. Vault content in the wild encodes edge
semantics in prose ("This supersedes [[Old Design]]"), in frontmatter
tags (`tags: [derived]`), and in ad-hoc conventions — none of which the
graph can query. The semantics exist; the graph cannot see them.

### 1.2 Relationship to [[SPEC-044]]

[[SPEC-044]] (Concept Graph — SPO Relations, Emergent Vocabulary, In-Situ
Ratification) was authored independently against the same
[[Wikilinks and Named Edges]] guide and targets the same capability:
*turn a markdown corpus into a typed, machine-reasonable knowledge
graph.* The two specs are deliberately split by **layer**, not forked in
competition:

| Concern | Owner |
|---|---|
| Vision, learning-system frame, "graph is the byproduct of learning" | [[SPEC-044]] |
| **Authoring UX** — in-situ ratification, suggest-and-ratify, the ratify-queue | [[SPEC-044]] |
| **Vocabulary governance philosophy** — emergent→crystallising, who governs | [[SPEC-044]] |
| Provenance-of-the-cut / answerability | [[SPEC-044]] |
| **Parse grammar + recogniser** (LangSec) | **SPEC-045** ([[#CON-4501]]) |
| **AST representation** of a typed edge | **SPEC-045** ([[#CON-4505]]) |
| **Typed [[Link Graph]]** + query CLI | **SPEC-045** ([[#REQ-4506]], [[#REQ-4509]]) |
| **[[SPL]] fact projection** + provenance plumbing | **SPEC-045** ([[#CON-4504]]) |
| Tests, threat model, NFRs | **SPEC-045** |

Two divergences are real and resolved in SPEC-044's favour:

1. **Authoring surface.** SPEC-044 chose **frontmatter `relations:`** as
   canonical (its reasons: validated shape from the pilot, render
   through a generic Connections block); it flags **body-inline**
   `predicate::[[Target]]` as an *open Tier-1 fork* (SPEC-044 §9.2). This
   spec specifies the body-inline form in full — but as the **in-prose
   authoring affordance that complements, not replaces, the frontmatter
   canonical store.** The reconciliation (author inline *and*
   derive/mirror to frontmatter, vs frontmatter-canonical with prominent
   render) is the FIRST task of [[DESIGN-045-wikilink-predicate-language]]
   and gates the rest; until it lands, treat [[#REQ-4501]]'s body-inline
   recognition as one of two candidate surfaces, not a settled decision.
2. **Telos of annotations.** This spec's framing of edge annotations as
   *agent reading-budget / progressive disclosure* is exactly the framing
   SPEC-044 §9.3 **declines** as "connective, not conjunctive." SPEC-044's
   reading governs: the annotation (the `note`) is first and foremost the
   **answerability layer** (the I-Thou trace of *why* the edge exists),
   and only incidentally a traversal aid. Wherever this spec says "reading
   budget," read it as subordinate to SPEC-044's answerability framing;
   the machine-facing capability (annotation is captured, queryable,
   rendered) is identical either way, so SPEC-045's contracts are
   telos-neutral by construction.

What SPEC-045 contributes that SPEC-044 lacks: the *designed-spec* layer
SPEC-044 explicitly says it does not yet have — a recognised grammar,
an additive AST contract, the graph/projection plumbing, and a verifiable
test + threat model. The machine substrate is the same triple either way
(`(predicate, source, target, note)` ≡ SPEC-044's SPO); only the
authoring surface and the surrounding UX differ.

### 1.3 Core Insight

**A typed edge is a fact, and zetl already has a fact engine.** The
[[Wikilinks and Named Edges]] guide proposes a plain-text syntax for
typed edges — `predicate_name::[[Target Node]]` — that needs nothing
but a text editor and `rg`. zetl can do better than `rg`: it already
parses `[[wikilinks]]` into a typed AST ([[SPEC-032]]
[[zetl-ext AST]]), already builds a [[Link Graph]] with per-edge
metadata ([[graph.rs|`EdgeMeta`]] at `src/graph.rs:18`), and already
runs a defeasible reasoner over [[SPL]] facts ([[SPEC-005]]).

The named-edge predicate `derived_from::[[Source]]` carries the same
information as the [[SPL]] fact `(derived_from "ThisPage" "Source")`.
So the design move — mirroring how [[SPEC-042]] made `public_paths`
*sugar over* [[SPL]] rather than a parallel policy surface — is:

* The predicate is **recognised at the parse boundary** and attached to
  the existing [[Wikilink]] AST node as an optional field. A bare
  `[[wikilink]]` is just a named edge whose predicate is `null` (an
  *untyped* edge). No new node type for the common case; full backward
  compatibility by construction.
* The predicate **flows into the [[Link Graph]]** as
  [[graph.rs|`EdgeMeta.predicate`]], turning the monochrome graph into a
  labelled multigraph. Backlinks, dead-links, and orphan detection all
  gain a predicate dimension for free.
* The predicate **projects to an [[SPL]] fact** under `--features
  reason`, so the entire vault's typed-edge structure becomes queryable
  by the same defeasible engine that already answers readiness and
  authorization questions. `zetl reason` can now ask "which pages
  `contradicts` a page that is `conforms_to` the [[Security Form
  Contract]]?" — a graph query expressed as logic.

One graph. One AST. One reasoner. The predicate language is not a new
subsystem bolted onto wikilinks; it is the **missing label on an edge
that already exists.**

### 1.4 Design Principles

1. **A bare wikilink is a named edge with a null predicate.** Typing is
   additive. A vault that never writes a `::` behaves exactly as it does
   today (REQ-4505). There is no migration, no flag day, no opt-in
   needed to keep working.
2. **The predicate binds to the link, not the prose.** The semantic unit
   is `predicate::[[Target]]` — a recogniser-parseable token, not a
   natural-language sentence. Per [[PROTO-001]] Constitutional Principle
   14 ([[LangSec]]), the syntax is a declared formal grammar (REQ-4502,
   [[#CON-4501]]); malformed predicates are rejected at parse time, not
   normalised or guessed.
3. **Curated folksonomy, not enforced ontology.** Following the guide's
   governance stance, an *undeclared* predicate is a **warning**, never
   an error (REQ-4508, [[#ADR-4502]]). The vocabulary
   ([[#CON-4502|`.zetl/predicates.toml`]]) starts small, grows through
   use, and is pruned periodically. zetl surfaces drift; it does not
   forbid invention.
4. **Predicates live in the body, not the frontmatter.** Per the guide's
   unconflation principle: YAML holds scalars (dates, word counts);
   body predicates hold relationships to *concepts that have their own
   pages* ([[#ADR-4505]]). A `tags:` entry that names another page is a
   predicate in disguise; the migration helper (REQ-4513) makes that
   conversion visible, never automatic.
5. **`conforms_to::` over `is_a::`.** A node conforms to a specification;
   it is not identical to one. zetl ships a default lint suggesting
   `conforms_to::[[X Form Contract]]` wherever it sees `is_a::`
   ([[#ADR-4503]]) — but as advice, honouring naming sovereignty
   (principle 3).
6. **Typed edges are sugar over [[SPL]].** Under `--features reason`,
   each named edge compiles to a fact `(predicate "Source" "Target")`
   ([[#ADR-4504]], [[#CON-4504]]). The reasoner is the single query
   engine; `zetl edges` is the operator-friendly surface; SPL is the
   expressiveness escape hatch — exactly the [[SPEC-042]] sugar pattern.
7. **Page-authored facts are a trust boundary.** A predicate on page A
   asserts a fact *about* page B. When those facts feed the reasoner,
   page A can make claims about B that B never agreed to (e.g.
   `validated_by::[[A]]` written on a page that was never validated).
   The projection MUST preserve provenance so trust can be scoped
   ([[#Threat Model A]], [[#ADR-4504]]).
8. **The same predicate query serves CLI, web, and search.** A typed
   backlink panel ([[#REQ-4511]]), a `zetl edges` query ([[#REQ-4509]]),
   and a predicate-filtered search ([[#REQ-4512]]) all read the one
   labelled graph. No second index, no divergent answers.

### 1.5 Scope

**In scope:**

- A `predicate::[[Target]]` named-edge **grammar** recognised in page
  bodies, in both list-item form (`- derived_from::[[X]]`) and inline
  form, with a declared ABNF ([[#REQ-4502]], [[#CON-4501]]).
- An **additive [[Wikilink]] AST extension** — an optional `predicate`
  field on the existing node ([[#REQ-4503]], [[#CON-4505]]), compatible
  with the [[SPEC-032]] additive-only schema-evolution rule
  ([[SPEC-032#NFR-3206]]).
- **Edge annotations** — the indented sub-content under a list-item
  predicate, captured as edge metadata for progressive disclosure
  ([[#REQ-4504]]).
- A **typed [[Link Graph]]** — `EdgeMeta.predicate`, typed backlinks,
  typed dead-link / orphan reporting ([[#REQ-4506]]).
- A **vocabulary declaration file** `.zetl/predicates.toml` seeded with
  the five guide categories (Classification, Provenance, Structural,
  Lifecycle, Generative) ([[#REQ-4507]], [[#CON-4502]]).
- **`zetl check` predicate lints** — undeclared-predicate warning,
  `relates_to::` over-use signal, `is_a::`→`conforms_to::` suggestion
  ([[#REQ-4508]]).
- A **`zetl edges` query CLI** — filter the typed graph by predicate,
  direction, source, or target ([[#REQ-4509]], [[#CON-4503]]).
- **Named-edge → [[SPL]] fact projection** under `--features reason`
  ([[#REQ-4510]], [[#CON-4504]]).
- **Typed backlink rendering** in `zetl build` web output, grouped by
  predicate with annotation-driven progressive disclosure
  ([[#REQ-4511]]).
- **Predicate-aware search** — predicate as a filterable field on the
  search index ([[#REQ-4512]]).
- A **read-only `tags:`→predicate migration helper** that *reports*
  candidate conversions without rewriting files ([[#REQ-4513]]).

**Out of scope:**

- **Predicate inheritance / subsumption hierarchies** (e.g.
  "`extends::` implies `relates_to::`"). The guide explicitly favours a
  flat curated folksonomy over an ontology with subsumption; a reasoning
  layer that derives super-predicates belongs in user-authored [[SPL]]
  rules, not in the core projection.
- **Cross-vault predicate translation** (the guide's "translate rather
  than converge" across systems). Single-vault only here; cross-vault
  edges touch the [[SPEC-004|Distributed Sync]] surface and belong in a
  follow-up.
- **Automatic file rewriting** for vocabulary migration. The helper
  (REQ-4513) reports; an author edits. Automatic rewriting of body
  predicates is a follow-up requiring the [[SPEC-032]] transform-hook
  attribution story.
- **Predicates in frontmatter.** Deliberately excluded ([[#ADR-4505]]);
  scalars stay in YAML, relationships stay in the body.
- **A general graph-pattern query language** (Cypher/SPARQL-style
  multi-hop joins). `zetl edges` does single-predicate filtering; multi-
  hop reasoning is delegated to [[SPL]] via the fact projection.

---

## 2. User Profiles

> **`[Provisional — refined by [[DESIGN-045-wikilink-predicate-language]]
> task user-profiles]`** Sketched from the [[Wikilinks and Named Edges]]
> guide and the design conversation; the plan task produces the grounded
> version after surveying vault authors who already hand-roll edge
> semantics.

### 2.1 The Knowledge Gardener

Maintains a personal or team vault of interlinked notes over years.
Already encodes edge meaning informally ("see also", "supersedes",
frontmatter tags) and wants the graph to *remember* those distinctions
so a year-old note's relationships are legible without re-reading the
prose. Cares most about typed backlinks and vocabulary hygiene.

### 2.2 The Traversing Agent

An LLM agent reading the vault to build context for a task. Has a finite
reading budget and must decide which edges to follow. Reads
classification predicates first (node type, maturity), then semantic
predicates and their annotations, then body content only if still
relevant — the guide's *progressive disclosure* traversal. Wants typed
edges in machine-readable form (`zetl edges --json`) and provenance on
projected facts so it can weight a `validated_by::` claim by who made
it.

### 2.3 The Reasoning Author

Writes [[SPL]] rules over the vault ([[SPEC-005]]). Wants the typed-edge
structure as facts so a rule can say "a page that `contradicts` a page
`conforms_to` the [[Security Form Contract]] is `(flagged review)`."
Cares that the projection is faithful, provenance-tagged, and stable
across builds.

### 2.4 The Vocabulary Curator

A team lead or maintainer who periodically audits the predicate
vocabulary — merging synonyms, tightening definitions, pruning the
`relates_to::` catch-all. Wants `zetl check` to surface drift and `zetl
edges --by-predicate` to show the vocabulary's actual distribution, not
the declared one.

---

## 3. Happy Paths

> **`[Provisional — refined by [[DESIGN-045-wikilink-predicate-language]]
> task happy-paths]`**

### 3.1 HP1: Default — No Predicates, Nothing Changes

**Preconditions:** An existing vault upgrades zetl. No page contains a
`::` before a wikilink; no `.zetl/predicates.toml` exists.

**Steps:** none.

**Postconditions:** Every `[[wikilink]]` parses, graphs, renders, and
searches exactly as before. The [[Wikilink]] AST node carries
`predicate: null` on every edge. `zetl edges` with no predicate filter
lists the same edge set as `zetl links`. Verified by the unchanged
[[SPEC-001]] link-graph test suite ([[#TEST-4505]]).

### 3.2 HP2: Author Types an Edge

**Preconditions:** The Knowledge Gardener is writing `Decision Log.md`.

**Steps:**

1. They write, in the body:
   ```markdown
   ## Provenance

   - derived_from::[[2026 Q1 Retro]]
     - The retro's "ship smaller" finding is the seed of this decision.
   - supersedes::[[Decision Log 2025]]
   - contradicts::[[Move Fast Doctrine]]
     - Specifically rejects the "skip the design doc" clause.
   ```
2. They run `zetl check`. `derived_from`, `supersedes`, and
   `contradicts` are all in the seeded `.zetl/predicates.toml`, so no
   warning fires.
3. They run `zetl edges --from "Decision Log"`:
   ```
   Decision Log
     derived_from  → 2026 Q1 Retro
     supersedes    → Decision Log 2025
     contradicts   → Move Fast Doctrine
   ```

**Postconditions:** Three typed edges land in the graph, each carrying
the predicate, the target, and (for the two annotated ones) the
annotation text. The bare prose "supersedes" of the old world is now a
queryable edge.

### 3.3 HP3: Typed Backlinks on the Target

**Preconditions:** Same vault; the Reasoning Author opens `Move Fast
Doctrine.md` in the `zetl build` web output.

**Steps:**

1. The page's backlink panel renders grouped by predicate:
   ```
   Backlinks
     Contradicted by (1)
       Decision Log — "Specifically rejects the 'skip the design
                       doc' clause."
     See also (4)
       …
   ```
2. The annotation surfaces inline so the reader knows *why* the edge
   exists before clicking ([[#REQ-4511]], progressive disclosure).

**Postconditions:** The incoming `contradicts` edge is visually and
semantically distinct from the four untyped "see also" edges.

### 3.4 HP4: An Undeclared Predicate Warns, Does Not Break

**Preconditions:** The author writes `inspired_by::[[Some Talk]]`, but
`inspired_by` is not in `.zetl/predicates.toml` (only `informed_by` is).

**Steps:**

1. `zetl check` emits a **warning** (not an error):
   ```
   warning[predicate-undeclared]: predicate `inspired_by` is not in
     .zetl/predicates.toml (did you mean `informed_by`?)
     --> Notes/Idea.md:12
     = note: add it to [predicates.provenance] to silence this, or run
             `zetl edges --by-predicate` to audit vocabulary drift
   ```
2. The build still succeeds. The edge is graphed and projected with
   `predicate: "inspired_by"` regardless — folksonomy first
   ([[#ADR-4502]]).
3. The Vocabulary Curator later decides `inspired_by` is a real
   addition and adds it to `[predicates.provenance]`, *or* decides it's
   a synonym and the author rewrites it to `informed_by`.

**Postconditions:** Invention is never blocked; drift is always visible.
`--fail-on warning` (existing `zetl check` flag, `src/cli.rs:187`) lets
a strict team escalate the warning to a CI gate by choice.

### 3.5 HP5: Reasoning Over Typed Edges

**Preconditions:** `--features reason` build; vault has typed edges and
an author-written rule in `Review Policy.spl`.

**Steps:**

1. Named edges project to facts (REQ-4510). For HP2's page:
   ```spl
   (derived_from "Decision Log" "2026 Q1 Retro")
   (supersedes "Decision Log" "Decision Log 2025")
   (contradicts "Decision Log" "Move Fast Doctrine")
   ```
2. The author's rule fires:
   ```spl
   (normally r-superseded-stale
     (and (supersedes ?new ?old))
     (stale ?old))
   ```
3. `zetl reason` concludes `(stale "Decision Log 2025")` and `zetl
   check` can surface stale-but-still-linked pages.

**Postconditions:** The typed-edge graph is queryable as logic; the
conclusion traces back through the projected fact to the source page,
line, and predicate (provenance preserved per [[#ADR-4504]]).

### 3.6 HP6: A Malformed Predicate Is Rejected, Not Guessed

**Preconditions:** The author fat-fingers `derived from::[[X]]` (space
in the predicate) or `derived_from:::[[X]]` (triple colon).

**Steps:**

1. The parser recognises neither against the [[#CON-4501]] grammar.
2. Per [[LangSec]] (be conservative in what you accept), the token is
   **not** silently normalised to `derived_from`. It either parses as
   plain text with no edge (the space case — `derived from` is prose,
   `[[X]]` is a bare untyped wikilink) or emits a `syntax` diagnostic
   (the triple-colon case — a `::`-adjacent malformed token), surfaced
   by `zetl check` (`src/main.rs:1642`).

**Postconditions:** No phantom `derived_from` edge from a typo. The
author sees the diagnostic and fixes the source. Determinism: the same
input always produces the same edge set ([[#TEST-4502]]).

---

## 4. Functional Requirements

> Numbering: SPEC-045 → REQ-45xx. Each REQ is decomposed into positive /
> negative-input / negative-output tests per [[PROTO-001]]
> §Requirement-Targeted Test Decomposition (see [[#9. Verification and
> Testing Strategy]]).

### REQ-4501: Named-Edge Recognition

The system SHALL recognise a **named edge** wherever a predicate token
immediately precedes a `[[wikilink]]` in a page body, in two forms:

* **List-item form:** a list item whose first inline content is
  `predicate::[[Target]]` (the guide's canonical form,
  `- derived_from::[[Target]]`).
* **Inline form:** `predicate::[[Target]]` appearing inline within a
  paragraph or other inline context.

The predicate binds to the single wikilink it immediately precedes. A
`[[wikilink]]` with no preceding predicate token is an **untyped edge**
(`predicate: null`) and is recognised exactly as today.

Recognition occurs at the same parse boundary that already extracts
wikilinks (`src/scanner.rs:394`, `src/hooks/ast/parse.rs:689`); named
edges are NOT a second scanning pass.

**Trace:** [[#TEST-4501]], [[#CON-4501]], [[#CON-4505]]; [[#3.2 HP2]].

### REQ-4502: Predicate-Name Grammar (LangSec)

A predicate token SHALL satisfy the grammar in [[#CON-4501]]: one or
more lowercase ASCII letters, digits, and underscores, terminated by the
two-character separator `::`, with no intervening whitespace, ending
immediately before the opening `[[`. The grammar is `regular` (the
lowest grammatical class sufficient — [[PROTO-001]] §LangSec principle
6) and is part of the parser contract, not an implementation detail.

Tokens that do not satisfy the grammar SHALL NOT be normalised, repaired,
or guessed into a valid predicate ([[PROTO-001]] Constitutional
Principle 14, "be conservative in what you accept"). A
`predicate::`-shaped token with a trailing or leading malformation
(e.g. `foo:::`, `:: [[X]]` with a space) SHALL either resolve to an
untyped edge plus surrounding text, or raise a `syntax` diagnostic — per
the disposition table in [[#CON-4501]].

**Trace:** [[#TEST-4502]], [[#CON-4501]]; [[PROTO-001]] §LangSec;
[[#3.6 HP6]].

### REQ-4503: Additive AST Representation

The [[zetl-ext AST]] [[Wikilink]] node ([[SPEC-032]] schema,
`src/hooks/ast/mod.rs:419`) SHALL gain an OPTIONAL `predicate` field
(`string` | `null`, `null` when absent). The field SHALL be additive
per the [[SPEC-032#NFR-3206]] additive-only evolution rule: the AST
schema minor version increments (e.g. `1.0` → `1.1`); existing hooks
that do not read `predicate` continue to round-trip the node unchanged;
the `Embed` node (`![[…]]`) does NOT gain a predicate (transclusion is
not a typed relationship — [[#ADR-4501]]).

**Trace:** [[#TEST-4503]], [[#CON-4505]], [[#ADR-4501]]; [[SPEC-032]].

### REQ-4504: Edge Annotations

WHEN a named edge appears in list-item form and the list item carries
nested block content (the guide's indented sub-bullets), the system
SHALL capture that nested content as the edge's **annotation** and attach
it to the edge in the [[Link Graph]] (`EdgeMeta.annotation`,
`src/graph.rs:18`). The annotation SHALL be available to the typed
backlink renderer ([[#REQ-4511]]) and the `zetl edges` query
([[#REQ-4509]]) for progressive disclosure. Annotations are OPTIONAL;
their absence is `null`, not an error.

The annotation is captured at the graph/edge layer from the surrounding
`ListItem` subtree, NOT as a field on the [[Wikilink]] AST node — the
AST node stays minimal and the annotation stays where Markdown already
structures it ([[#ADR-4501]]).

**Trace:** [[#TEST-4504]], [[#CON-4503]]; [[#3.2 HP2]], [[#3.3 HP3]].

### REQ-4505: Backward-Compatible Default

WHEN a vault contains no named edges (no `predicate::[[…]]` token) and
no `.zetl/predicates.toml`, the system SHALL behave exactly as the
pre-SPEC-045 release: identical link graph, identical backlinks,
identical search results, identical web output. Every edge carries
`predicate: null`; `zetl edges` lists the same edges as `zetl links`.

**Trace:** [[#TEST-4505]]; [[#3.1 HP1]].

### REQ-4506: Typed Link Graph

The [[Link Graph]] ([[graph.rs|`LinkGraph`]]) SHALL carry the predicate
on each edge (`EdgeMeta.predicate: Option<String>`, `src/graph.rs:18`).
The following SHALL gain a predicate dimension:

* **Backlinks** (`backlinks(page)`, `src/graph.rs:190`) — each
  `BacklinkResult` SHALL carry the predicate and annotation of the
  incoming edge.
* **Dead links** (`dead_links()`, `src/graph.rs:212`) — a typed edge to
  a phantom node SHALL report its predicate, so a curator can see "3
  dead `supersedes::` targets".
* **Orphans** (`orphans()`, `src/graph.rs:236`) — unchanged in count,
  but the API SHALL expose incoming-edge predicates for the
  centrality-by-predicate view ([[#REQ-4509]]).

**Trace:** [[#TEST-4506]], [[#CON-4503]]; [[SPEC-001]].

### REQ-4507: Vocabulary Declaration File

The system SHALL read an OPTIONAL `.zetl/predicates.toml` declaring the
curated vocabulary, structured by the five [[Wikilinks and Named
Edges]] categories. Its schema is [[#CON-4502]]. zetl SHALL ship a
`zetl predicates init` seed containing the guide's starter vocabulary
(≈30 predicates across Classification / Provenance / Structural /
Lifecycle / Generative). Absent file ⇒ every predicate is treated as
undeclared (all warn under REQ-4508) — a vault opting out of governance
still works, it just gets no hygiene signal.

REQ default value: the seeded file is OPT-IN via `zetl predicates init`;
the default state of a fresh vault is *no file*, because the dominant
profile ([[users/gardener/happy-paths]]) is an existing vault that
should not acquire governance it did not ask for ([[PROTO-001]]
§Requirement Templates — default justification).

**Trace:** [[#TEST-4507]], [[#CON-4502]]; [[#3.4 HP4]].

### REQ-4508: Predicate Lints in `zetl check`

`zetl check` (`src/main.rs:1610`) SHALL emit the following diagnostics,
all at **warning** severity by default (escalatable via the existing
`--fail-on warning`, `src/cli.rs:187`):

* `predicate-undeclared` — a predicate not present in
  `.zetl/predicates.toml`, with a nearest-match suggestion (Levenshtein
  ≤ 2 against declared predicates).
* `predicate-relates-to-overuse` — `relates_to::` exceeding a configured
  share of a page's edges (the guide's "`relates_to::` trap"); default
  threshold `[Provisional: > 50% of a page's typed edges]`.
* `predicate-prefer-conforms-to` — any `is_a::` edge, suggesting
  `conforms_to::[[X Form Contract]]` ([[#ADR-4503]]).

No predicate diagnostic SHALL be an error by default. An *undeclared*
predicate is never a build failure unless the operator opts into
`--fail-on warning` ([[#ADR-4502]]).

**Trace:** [[#TEST-4508]], [[#ADR-4502]], [[#ADR-4503]]; [[#3.4 HP4]].

### REQ-4509: `zetl edges` Query CLI

The system SHALL provide `zetl edges` to query the typed graph, with
filters:

* `--predicate <name>` — edges with this predicate (repeatable for OR).
* `--from <page>` / `--to <page>` — edges by source / target.
* `--by-predicate` — group-and-count the whole vault's edges by
  predicate (the vocabulary-distribution audit).
* `--untyped` — only `predicate: null` edges.
* `--annotated` — only edges carrying an annotation.

Output respects the existing `-f {auto,json,table}` convention
(`src/cli.rs`). The command SHALL be read-only with respect to the
vault, open no network socket, and be idempotent. With no filter it
lists every edge (typed and untyped), making it a strict superset of
`zetl links`.

**Trace:** [[#TEST-4509]], [[#CON-4503]]; [[#3.2 HP2]].

### REQ-4510: Named-Edge → SPL Fact Projection

Under `--features reason`, the system SHALL project each named edge to
an [[SPL]] fact of the shape `(<predicate> "<source-page>"
"<target-page>")`, inserted into the theory the reasoner evaluates
([[SPEC-005]], `src/reason/mod.rs:72`). The projection SHALL:

* emit one fact per typed edge (untyped edges, `predicate: null`, are
  NOT projected — there is no predicate to name the relation);
* normalise the predicate to a valid SPL functor (it already satisfies
  the lowercase-snake grammar of REQ-4502, which is SPL-functor-safe);
* preserve provenance via the existing SPL source-annotation mechanism
  (`src/reason/mod.rs:116`) — every projected fact carries
  `_source_file`, `_source_line`, `_source_page` so a conclusion traces
  back to the asserting page ([[#Threat Model A]], [[#ADR-4504]]);
* be DETERMINISTIC and stable across builds (same vault ⇒ same fact set
  in the same order) so `zetl reason` output is reproducible.

Under the default build (no `--features reason`) the projection is not
compiled in; named edges remain available via the graph and `zetl
edges` ([[#REQ-4509]]) — the SPL surface is the only thing that
degrades.

**Trace:** [[#TEST-4510]], [[#CON-4504]], [[#ADR-4504]]; [[#3.5 HP5]];
[[#Threat Model A]].

### REQ-4511: Typed Backlink Rendering

`zetl build` web output SHALL render the backlink panel grouped by
predicate, with a stable group ordering (predicate categories in the
guide's order: Classification, Provenance, Structural, Lifecycle,
Generative; untyped "see also" last). Each group SHALL show its
predicate label (human-readable, from `.zetl/predicates.toml`'s
`display` field if present, else the raw predicate) and the count; each
edge SHALL surface its annotation inline when present (progressive
disclosure — the reader assesses the edge before clicking).

WHEN no page has a typed backlink, the panel renders exactly as the
pre-SPEC-045 flat list (REQ-4505 continuity).

**Trace:** [[#TEST-4511]], [[#CON-4503]]; [[#3.3 HP3]].

### REQ-4512: Predicate-Aware Search

The search index (`src/search_index.rs:152`) SHALL gain a `predicate`
field (faceted, `STRING | STORED`) capturing the predicates of a page's
*outgoing* typed edges, so a query can filter "pages that `contradicts`
something" or "pages with a `conforms_to` edge". The existing
`page_name` / `path` / `body` fields are unchanged; the addition is
additive and re-index-on-upgrade ([[#TEST-4512]]).

**Trace:** [[#TEST-4512]]; [[SPEC-002]].

### REQ-4513: Read-Only `tags:`→Predicate Migration Helper

The system SHALL provide `zetl predicates migrate --dry-run` that scans
frontmatter `tags:` (and other configured scalar-list keys) and REPORTS,
without modifying any file, which entries name a page that exists in the
vault and could become a body predicate (the guide's `tags: [deep-
context]` → `in_domain::[[Deep Context Architecture]]`). The report
names the file, the tag, the candidate predicate, and the candidate
target page. The command SHALL NOT rewrite files (automatic rewriting is
out of scope, §1.5). Without `--dry-run` the command SHALL refuse to run
and print that only dry-run is supported in v1.

**Trace:** [[#TEST-4513]]; [[Wikilinks and Named Edges]] §"Tags should
become predicates".

### REQ-4514: External and Ghost Typed Edges

A named edge whose target does not resolve to a vault page SHALL be
recorded as a typed **ghost edge** (a typed edge to a phantom node —
`src/graph.rs:122` phantom-node mechanism), reported by `dead_links()`
with its predicate ([[#REQ-4506]]). A target marked external (the
guide's `[[Concept]]↗` convention, if recognised by the resolver) SHALL
be typed but excluded from dead-link reporting. The predicate is
preserved on ghost edges so a curator can see "where the typed graph
wants to grow".

**Trace:** [[#TEST-4514]]; [[#3.4 HP4]]; [[Wikilinks and Named
Edges]] §"Ghost links".

---

## 5. Non-Functional Requirements

### NFR-4501: Parse Overhead Per Edge

Predicate recognition SHALL add no more than **`[Provisional: 2 µs]`**
per wikilink at the 95th percentile to the existing parse hot path
(`src/hooks/ast/parse.rs:689`), measured on a vault page of
`[Provisional: 10 KB]`. Recognition reuses the existing wikilink scan;
it MUST NOT introduce a second full-document pass.

**Trace:** [[#TEST-NFR-4501]], [[#OBS-4501]].

### NFR-4502: Graph Query Latency

`zetl edges --predicate <name>` SHALL return in ≤ **`[Provisional:
50 ms]`** at the 95th percentile on a vault of **`[Provisional: 10 000
pages / 100 000 edges]`**, using an index keyed by predicate built once
at graph-construction time (not a per-query linear scan of all edges).

**Trace:** [[#TEST-NFR-4502]], [[#OBS-4502]].

### NFR-4503: SPL Projection Scale

Named-edge → fact projection SHALL complete in ≤ **`[Provisional:
200 ms]`** for a vault of **`[Provisional: 100 000 typed edges]`** and
SHALL be linear in edge count. Projection time attaches to the
`--features reason` path only; the default build pays nothing
(NFR optimised for the dominant no-reason profile — [[PROTO-001]]
§Non-Functional Requirement template).

**Trace:** [[#TEST-NFR-4503]], [[#OBS-4503]].

---

## 6. Architecture Decision Records

> ADRs sketched as positions. [[DESIGN-045-wikilink-predicate-language]]
> plan tasks finalise each.

### ADR-4501: Extend `Wikilink` vs New `NamedEdge` AST Node

**Status:** Proposed (strawman default)

**Context:** Two ways to represent a typed edge in the [[zetl-ext AST]]:
(a) add an optional `predicate` field to the existing [[Wikilink]] node;
(b) introduce a new `NamedEdge` block (or inline) node distinct from
`Wikilink`.

**Decision:** (a). A named edge *is* a wikilink that happens to carry a
label; the target/alias/heading/block-id machinery is identical. Adding
an optional field is additive under [[SPEC-032#NFR-3206]] (a new node
type in the `oneOf` is also additive, but it forces every existing
wikilink consumer — backlinks, dead-links, search, renderer — to learn a
second node shape). Reusing `Wikilink` means the entire existing
wikilink toolchain types edges for free; `predicate: null` is the
no-cost default.

**Consequences:** (+) One extraction site, one graph edge type, zero
new consumers. (+) Backward compatibility is structural, not bolted on.
(−) The `Wikilink` node now carries a field meaningful only in body
context; an inline wikilink inside a heading could in principle carry a
predicate that makes little semantic sense — mitigated by binding rules
in [[#CON-4501]]. (−) Annotations can't live on the node (they're
list-structured), so they attach at the graph layer (REQ-4504) — a
slight asymmetry between "predicate on the node, annotation on the
edge".

### ADR-4502: Curated Folksonomy (Warn) Over Enforced Ontology (Error)

**Status:** Proposed (strawman default)

**Context:** When an author writes an undeclared predicate, zetl can (a)
warn, (b) error/refuse the build, or (c) silently accept. The
[[Wikilinks and Named Edges]] guide is emphatic: *"Vocabulary requires
curation, not just enforcement"* and recommends a small core that grows
through use, then is pruned.

**Decision:** (a) warn, never error by default. An undeclared predicate
is graphed and projected regardless; `zetl check` surfaces it as a
warning with a nearest-match suggestion. Teams who want enforcement opt
in via the *existing* `--fail-on warning` flag — no new gate primitive.

**Consequences:** (+) Honours naming sovereignty (principle 3) and the
guide's folksonomy stance — invention is never blocked. (+) Drift is
visible without being fatal. (+) Reuses the existing diagnostic
severity + fail-on machinery; no new concept. (−) A sloppy vault
accumulates vocabulary drift if no one runs `zetl edges --by-predicate`;
mitigated by making that audit a one-liner and surfacing the warning on
every `zetl check`.

### ADR-4503: Ship `is_a::` → `conforms_to::` as Advice, Not Rewrite

**Status:** Proposed (strawman default)

**Context:** The guide argues `is_a::` asserts terminal identity and
should be replaced by `conforms_to::[[X Form Contract]]`, which names
revisable compliance with a specification and permits multi-contract
conformance. zetl could (a) lint-suggest, (b) auto-rewrite, (c) stay
silent.

**Decision:** (a). Emit `predicate-prefer-conforms-to` as a warning
wherever `is_a::` appears. Do NOT auto-rewrite (auto-rewriting body
predicates is out of scope and would violate naming sovereignty). Do NOT
forbid `is_a::` — a vault may have a legitimate identity relation.

**Consequences:** (+) Transmits the guide's hard-won design wisdom to
authors at the point of use. (+) Non-coercive. (−) Some authors will
dismiss the suggestion repeatedly; the warning is suppressible by
declaring `is_a` in `.zetl/predicates.toml` with an explicit
`accept_is_a = true` marker ([[#CON-4502]]).

### ADR-4504: Named Edges Project to Provenance-Tagged SPL Facts

**Status:** Proposed (strawman default)

**Context:** Typed edges and [[SPL]] facts carry the same information. We
can (a) project edges to facts so the reasoner sees the typed graph, (b)
keep them separate (graph queried by `zetl edges`, facts authored only
in `spl` blocks), or (c) build a separate typed-graph query engine.

**Decision:** (a), mirroring the [[SPEC-042]] "sugar over SPL" pattern.
Each typed edge becomes `(<predicate> "<source>" "<target>")` under
`--features reason`. Crucially, every projected fact is **provenance-
tagged** with the asserting page/line via the existing source-annotation
path (`src/reason/mod.rs:116`), because a predicate on page A asserts a
fact *about* page B ([[#Threat Model A]]).

**Consequences:** (+) The whole typed graph is queryable as logic with
one engine; no parallel query language ([[#1.5 Scope]] excludes a
graph-pattern DSL precisely because SPL covers multi-hop). (+) Reuses
the SPL provenance machinery, so trust can be scoped by author. (+)
Same sugar pattern operators already know from [[SPEC-042]]. (−)
Projection volume could be large (100 k facts) — bounded by NFR-4503
and the fact that untyped edges are not projected. (−) Page-authored
facts are a new trust boundary — addressed by provenance tagging and
[[#Threat Model A]], but reviewers MUST treat any rule that *acts* on a
projected fact as Tier-2 (a page can lie about another page).

### ADR-4505: Predicates Live in the Body, Not the Frontmatter

**Status:** Proposed (strawman default)

**Context:** Could predicates be declared in YAML frontmatter (e.g.
`derived_from: [2026 Q1 Retro]`)? The guide's unconflation principle
says no: YAML holds *scalars* (dates, counts, summaries); relationships
to *concepts that have their own pages* belong in the body where they
participate in the graph.

**Decision:** Body only. Frontmatter is explicitly out of scope
([[#1.5 Scope]]). The REQ-4513 migration helper exists precisely to move
relationship-shaped `tags:` *out* of frontmatter into body predicates.

**Consequences:** (+) Clean separation: scalars in YAML, edges in body,
matching the guide and the existing [[zetl-ext AST]] split (frontmatter
is a Document-root scalar object, not a child node — `src/hooks/ast`).
(+) Body predicates get positions, annotations, and graph membership
for free. (−) Authors migrating from tag-heavy vaults have a one-time
conversion (eased, not automated, by REQ-4513). (−) A predicate can't
be a pure document property (e.g. `has_status::[[Draft]]` is a body
edge, not a YAML field) — but the guide argues that's correct, since
status is a relationship to a status concept.

### ADR-4506: New `zetl edges` Subcommand vs Extending `zetl links`

**Status:** Proposed (strawman default)

**Context:** The query surface could (a) be a new `zetl edges`
subcommand, (b) be flags added to the existing `zetl links` /
`zetl backlinks` (`src/cli.rs:120`).

**Decision:** (a) a new `Command::Edges` variant. `zetl links` answers
"what connects to what" (the topological question); `zetl edges` answers
"what *kind* of connection" (the semantic question), with a richer
filter set (`--by-predicate`, `--annotated`, `--untyped`) that would
bloat `links`. Per [[PROTO-001]] Constitutional Principle 15 (right
place for the function), the new query responsibility is distinct enough
to warrant its own command rather than encrusting `links`.

**Consequences:** (+) `links` stays simple; `edges` owns the typed-query
responsibility cleanly. (+) Discoverable (`zetl edges --help` documents
the predicate model). (−) Two commands that overlap on the untyped case
(`zetl edges --untyped` ≈ `zetl links`) — documented as intentional;
`edges` is the superset surface.

---

## 7. Contracts

### CON-4501: Named-Edge Grammar

**Interface:** the page-body recogniser that extracts named edges,
sitting at the [[SPEC-032]] parse boundary (`src/hooks/ast/parse.rs`,
`src/scanner.rs`). Per [[PROTO-001]] §LangSec, the input language is
declared formally and recognised before any edge is constructed.

**Grammar (ABNF):**

```abnf
named-edge      = predicate "::" wikilink
predicate       = lower *( lower / DIGIT / "_" )
lower           = %x61-7A                 ; a-z
wikilink        = "[[" target [ "#" heading ] [ "#^" block-id ]
                  [ "|" alias ] "]]"
; target / heading / block-id / alias as defined by the existing
; SPEC-032 wikilink grammar (src/hooks/ast/mod.rs:419). The named-edge
; grammar adds ONLY the `predicate "::"` prefix.
```

**Binding rule:** the predicate binds to the single `wikilink` that
*immediately* follows the `::` with no intervening character (including
whitespace). `derived_from:: [[X]]` (space after `::`) is NOT a named
edge — it is the text `derived_from::` followed by an untyped wikilink.

**Pre-conditions:**
- Input is a recognised page body (frontmatter, fenced code, inline
  code, and HTML comments are excluded by the existing extractor —
  `src/scanner.rs`, `src/search_index.rs:183`). A `::` inside a code
  fence is never a predicate.

**Post-conditions:**
- A recognised named edge yields exactly one [[Wikilink]] node with
  `predicate = <predicate>` (REQ-4501, REQ-4503).
- An unrecognised `::`-adjacent token yields the disposition below.

**Malformed-token disposition (REQ-4502):**

| Source | Disposition |
|---|---|
| `derived_from::[[X]]` | named edge, `predicate="derived_from"` |
| `[[X]]` | untyped edge, `predicate=null` |
| `derived_from:: [[X]]` (space) | text `derived_from::` + untyped edge |
| `Derived_From::[[X]]` (uppercase) | NOT a predicate (grammar is lowercase); text + untyped edge + `predicate-undeclared`-adjacent diagnostic deferred to plan |
| `derived_from:::[[X]]` (triple colon) | `syntax` diagnostic; no edge from the malformed token |
| `derived from::[[X]]` (space in name) | text `derived from::` + untyped edge |

**Error model:** malformed tokens never produce a typed edge by
guessing. The recogniser is conservative (fail to untyped or to
diagnostic), never permissive.

**Implements:** [[#REQ-4501]], [[#REQ-4502]].
**Verified by:** [[#TEST-4501]], [[#TEST-4502]].

### CON-4502: `.zetl/predicates.toml` Schema

**Interface:** the optional vocabulary declaration file, parsed
alongside the existing config (`src/parsers/mod.rs:200`,
`src/web/auth/config.rs` lens pattern).

```toml
# .zetl/predicates.toml — curated predicate vocabulary (REQ-4507)
# Absent file ⇒ all predicates undeclared (warn-only). Opt in via
# `zetl predicates init`.

accept_is_a = false   # when true, suppress predicate-prefer-conforms-to

[predicates.classification]
conforms_to = { display = "Conforms to" }
has_status  = { display = "Status" }
in_domain   = { display = "In domain" }

[predicates.provenance]
derived_from   = { display = "Derived from" }
informed_by    = { display = "Informed by" }
extracted_from = { display = "Extracted from", construction = true }

[predicates.structural]
implements   = { display = "Implements" }
extends      = { display = "Extends" }
contradicts  = { display = "Contradicts" }
relates_to   = { display = "Relates to", catch_all = true }

[predicates.lifecycle]
supersedes   = { display = "Supersedes" }
validated_by = { display = "Validated by" }

[predicates.generative]
proposes  = { display = "Proposes" }
generates = { display = "Generates" }
```

**Pre-conditions:**
- Section keys MUST be one of the five guide categories
  (`classification`, `provenance`, `structural`, `lifecycle`,
  `generative`); `deny_unknown_fields` on the category set (a typo'd
  category is a config error, distinct from an undeclared *predicate*
  which is a content warning).
- Each predicate key MUST satisfy the REQ-4502 grammar.
- `display` is optional; `construction = true` marks a temporary
  scaffolding predicate (the guide's `extracted_from::`) eligible for an
  "upgrade me" lint; `catch_all = true` marks `relates_to::` for the
  over-use lint (REQ-4508); `accept_is_a` is a top-level escape hatch
  (ADR-4503).

**Post-conditions:**
- A predicate present in any category is *declared* (no
  `predicate-undeclared` warning).
- The category determines backlink group ordering (REQ-4511).

**Error model:** a malformed file is a startup/`check`-time config error
(fail closed — [[LangSec]] full recognition), distinct from a content
warning. The vault still builds the graph; only the *governance signal*
is unavailable until the file parses.

**Implements:** [[#REQ-4507]], [[#REQ-4508]], [[#REQ-4511]].
**Verified by:** [[#TEST-4507]], [[#TEST-4508]].

### CON-4503: `zetl edges` CLI

**Interface:** `zetl edges [FILTERS] [-f auto|json|table]`, a new
`Command::Edges` variant (`src/cli.rs:118`).

**Pre-conditions:** a scannable vault (same as `zetl links`). Read-only:
opens no socket, writes no file, idempotent (REQ-4509).

**Post-conditions:**
- With no filter: every edge (typed + untyped), a strict superset of
  `zetl links`.
- `--predicate P` (repeatable): edges whose predicate ∈ {P…}.
- `--from PAGE` / `--to PAGE`: edges by source / target page.
- `--by-predicate`: a histogram `{predicate → count}` over the vault.
- `--untyped`: `predicate=null` edges only.
- `--annotated`: edges with a non-null annotation only.
- `--json`: each row is `{source, target, predicate|null,
  annotation|null, line, is_ghost}`.

**Error model:** an unknown predicate filter is NOT an error (it yields
zero rows — a vocabulary may legitimately not be used yet); a
nonexistent `--from`/`--to` page yields zero rows with a stderr note.

**Implements:** [[#REQ-4509]], [[#REQ-4506]], [[#REQ-4504]].
**Verified by:** [[#TEST-4509]].

### CON-4504: SPL Fact Projection

**Interface:** the projection step inserting named-edge facts into the
reasoner's theory (`src/reason/mod.rs:72`), active under `--features
reason`.

**Pre-conditions:**
- The edge is typed (`predicate != null`); untyped edges are not
  projected (REQ-4510).
- The predicate satisfies the REQ-4502 grammar (lowercase-snake), which
  is a valid SPL functor by construction.

**Post-conditions:**
- One fact `(<predicate> "<source-page>" "<target-page>")` per typed
  edge, where page identifiers are the canonical page slugs the link
  resolver already produces.
- Each fact carries `_source_file`, `_source_line`, `_source_page`
  provenance metadata via the existing annotation path
  (`src/reason/mod.rs:116`) — REQUIRED, because the fact is *asserted by
  the source page about the target* ([[#Threat Model A]]).
- The fact set is deterministic and order-stable across builds for a
  fixed vault.

**Error model:** a predicate that (somehow) is not a valid functor is a
projection-time diagnostic, not a silent drop; the grammar (REQ-4502)
makes this unreachable for recognised edges, so it is a defence-in-depth
assertion.

**Property (roundtrip-adjacent):** for any typed edge `e`, the projected
fact's `(predicate, source, target)` equals `e`'s `(predicate, source,
target)` — projection is information-preserving on the triple
([[#TEST-4510]] property test).

**Implements:** [[#REQ-4510]].
**Verified by:** [[#TEST-4510]].

### CON-4505: `Wikilink` AST Extension

**Interface:** the [[zetl-ext AST]] [[Wikilink]] node
(`tools/zetl-ast-schema-v1.json`, `src/hooks/ast/mod.rs:419`).

**Pre/post-conditions:**
- Adds OPTIONAL property `predicate: { type: ["string","null"] }`;
  `null` when absent. The node's `required` set is UNCHANGED (the field
  is optional), preserving additive evolution ([[SPEC-032#NFR-3206]]).
- Schema minor version increments (`ast_version` `1.0` → `1.1`).
- A hook that does not read `predicate` round-trips the node byte-
  stably (the additive-evolution property test in
  `tests/ast_schema_integration.rs` gains a predicate case).

**Error model:** a `predicate` value that violates the REQ-4502 grammar
is a schema-validation failure (the schema SHOULD carry the
`pattern: "^[a-z][a-z0-9_]*$"` so the AST contract enforces the grammar,
not just the parser — defence in depth at the hook boundary).

**Implements:** [[#REQ-4503]].
**Verified by:** [[#TEST-4503]].

---

## 8. Purity Boundary Map

### Pure Core (no I/O, no shared state, deterministic)
- **Named-edge recogniser** (`predicate::[[…]]` → `Wikilink{predicate}`):
  pure function over a page-body string; the [[LangSec]] recogniser of
  [[#CON-4501]]. Ideal property-test + fuzz target.
- **Predicate vocabulary classifier** (predicate → category | undeclared
  + nearest-match): pure over `(predicate, declared-set)`.
- **Fact projection** (`Wikilink{predicate}` + source/target slugs →
  SPL fact triple): pure ([[#CON-4504]]); roundtrip property holds.
- **Edge-graph builder** (parsed files → labelled `LinkGraph`): pure
  over the parsed input.

### Effectful Shell (orchestrates I/O, calls pure core)
- Vault scan / file reads (`src/scanner.rs`).
- `.zetl/predicates.toml` load (`src/parsers/mod.rs`).
- `zetl edges` / `zetl check` / `zetl predicates` command handlers
  (`src/main.rs`).
- Search index write (`src/search_index.rs`).
- SPL theory assembly + reasoner invocation (`src/reason/mod.rs`).

### Boundary Contracts (data crossing the boundary)
- `Wikilink { …, predicate: Option<String> }` — parse → graph.
- `EdgeMeta { …, predicate: Option<String>, annotation: Option<String> }`
  — graph → query/render.
- `(predicate "src" "tgt")` + provenance — graph → reasoner.

### Dependency Rule
Dependencies point inward: shell → core. The recogniser, classifier,
and projector MUST NOT perform I/O or read global config directly — the
declared vocabulary is passed in as data.

### Enforcement
Existing zetl module structure + the [[SPEC-032]] purity discipline for
the parse pipeline; reviewed in [[DESIGN-045-wikilink-predicate-language]]
Phase 2.

---

## 9. Verification and Testing Strategy

> Per [[PROTO-001]] §Verification and Testing Techniques. This spec is an
> **AI-synthesised specification**, so per the protocol's selection table
> it requires **requirement-targeted decomposition + mutation testing +
> adversarial testing (mandatory)**. The named-edge recogniser is a
> parser at a trust boundary → **fuzzing REQUIRED**; the fact projection
> is a read/write-shaped transform → **roundtrip property test
> REQUIRED**.

### Selected techniques

| Surface | Techniques |
|---|---|
| Named-edge recogniser ([[#CON-4501]]) | Fuzzing (malformed `::` tokens) + property-based (recognition is total; no input panics — note the `src/hooks/ast/parse.rs` multibyte byte-index gotcha) + example-based disposition matrix |
| Fact projection ([[#CON-4504]]) | Property-based roundtrip (`triple(project(e)) == e`) + provenance-preservation property + determinism property |
| Vocabulary classifier ([[#REQ-4508]]) | Example-based (nearest-match suggestions) + property (declared ⇒ no warning) |
| Graph / query ([[#REQ-4506]], [[#REQ-4509]]) | Example-based + mutation testing on the predicate-filter logic |
| Backward compat ([[#REQ-4505]]) | Golden-output equivalence against the pre-SPEC-045 link-graph + web-build + search suites |

### Representative TEST specifications

Each `TEST-45xx` validates the same-numbered `REQ-45xx` and uses the
positive / negative-input / negative-output decomposition.

- **[[#TEST-4501]]** *(Validates [[#REQ-4501]])* — positive:
  `- derived_from::[[X]]` → one typed edge; negative-input: `[[X]]` with
  no predicate → untyped edge (predicate rules out a phantom label);
  negative-output: a predicate must NOT bind to a wikilink two tokens
  away (`derived_from:: and then [[X]]` produces no typed edge).
- **[[#TEST-4502]]** *(Validates [[#REQ-4502]])* — the [[#CON-4501]]
  disposition matrix as example cases + a fuzz target asserting no panic
  and no guessed normalisation; negative-output: `derived_from:::[[X]]`
  must NOT yield a `derived_from` edge.
- **[[#TEST-4503]]** *(Validates [[#REQ-4503]])* — schema round-trip:
  a `Wikilink{predicate}` validates against the bumped schema; a
  predicate-unaware hook round-trips it byte-stably; an invalid
  predicate value fails schema validation (negative-output).
- **[[#TEST-4505]]** *(Validates [[#REQ-4505]])* — golden equivalence:
  a predicate-free vault produces byte-identical graph/web/search output
  to the pre-SPEC-045 baseline.
- **[[#TEST-4510]]** *(Validates [[#REQ-4510]])* — property: projection
  is information-preserving on the triple and provenance-preserving;
  determinism: two builds of the same vault yield the identical fact set
  and order; negative-output: an untyped edge produces NO fact.
- **[[#TEST-4508]]** *(Validates [[#REQ-4508]])* — undeclared predicate
  warns with correct nearest match; declared predicate does not warn;
  `is_a::` triggers the conforms-to suggestion unless `accept_is_a`.

### Adversarial testing (mandatory, post-acceptance)

A fresh-context adversary ([[PROTO-001]] §Multi-Model Cognitive
Diversity) attacks: predicate-injection via crafted page content
([[#Threat Model A]]), Unicode/homoglyph predicates that look declared
but are not, `::` inside nested constructs (tables, footnotes, link
text), and projection-fact spoofing. Iterate to Adversary Exhaustion.

---

## 10. Observability

### OBS-4501: Predicate Parse Metrics
Counter of named edges recognised per build, split typed / untyped /
malformed; histogram of per-page recognition time (NFR-4501).

### OBS-4502: Vocabulary Drift Signal
`zetl edges --by-predicate` output emitted as a build-time structured
log: declared-vs-used predicate counts, top-N undeclared predicates,
`relates_to::` share. Feeds the curator's audit (user profile 2.4).

### OBS-4503: Projection Metrics
Under `--features reason`: count of facts projected, projection
duration (NFR-4503), count of provenance-tagged facts (MUST equal fact
count — a mismatch is a trust-boundary defect).

---

## 11. Threat Model

> The new trust boundary this spec introduces is **page-authored facts**:
> a predicate on page A makes a claim about page B. Enumerated per
> [[PROTO-001]] §Security by Design + §LangSec.

### Threat Model A: Predicate Fact Spoofing

**Threat:** Page A writes `validated_by::[[Untrustworthy Source]]` or
`conforms_to::[[Security Form Contract]]` — asserting, via the projected
fact, a property that B/the contract never granted. A naive [[SPL]] rule
acting on `(validated_by ?x ?y)` would treat A's self-assertion as
ground truth.

**Mitigation:** projected facts are ALWAYS provenance-tagged
([[#CON-4504]], [[#ADR-4504]]) with the asserting page. Rules that act
on projected facts MUST be able to scope trust by `_source_page`.
Reviewers treat any rule consuming a projected fact as **Tier 2** (a
page can lie about another page) — flagged in [[#9. Verification and
Testing Strategy]] adversarial testing. The spec does NOT auto-trust
projected facts; it makes provenance available so authors can choose.

### Threat Model B: Predicate Injection via Content

**Threat:** Crafted content (`::` inside a table cell, a footnote, an
autolink, an HTML block) tricks the recogniser into minting a typed edge
the author did not intend, or into a parser panic (recall the
`src/hooks/ast/parse.rs` multibyte byte-index gotcha).

**Mitigation:** [[LangSec]] full recognition — the recogniser operates
only on already-extracted body inline runs (frontmatter/code excluded,
`src/scanner.rs`), against the [[#CON-4501]] grammar, conservatively
(fail to untyped, never guess). Fuzzing is REQUIRED ([[#9. Verification
and Testing Strategy]]).

### Threat Model C: Vocabulary-File Confusion

**Threat:** A malformed or attacker-influenced `.zetl/predicates.toml`
silently disables governance (every predicate appears declared, or none
warn).

**Mitigation:** the file is a [[LangSec]]-recognised input with
`deny_unknown_fields` on the category set ([[#CON-4502]]); a parse
failure is a fail-closed config error (governance unavailable, loudly),
never a silent "everything is fine".

### Threat Model D: Ghost-Edge Title Leakage

**Threat:** A typed ghost edge (`supersedes::[[Private Draft]]`) on a
public page leaks the private target's title — interacts with
[[SPEC-042#REQ-4213]] (wikilink rendering on public pages referencing
private pages).

**Mitigation:** typed edges inherit the [[SPEC-042#REQ-4213]] public-
page rendering policy unchanged; the predicate adds a label but does
not change the redaction obligation. Cross-referenced as a dependency on
[[SPEC-042]] in [[#13. Open Questions]].

---

## 12. Traceability

```
REQ-4501 ──→ TEST-4501        (recognition)
REQ-4502 ──→ TEST-4502 ──→ CON-4501   (grammar / LangSec)
REQ-4503 ──→ TEST-4503 ──→ CON-4505   (AST extension)
REQ-4504 ──→ TEST-4504 ──→ CON-4503   (annotations)
REQ-4505 ──→ TEST-4505                (backward compat)
REQ-4506 ──→ TEST-4506 ──→ CON-4503   (typed graph)
REQ-4507 ──→ TEST-4507 ──→ CON-4502   (vocabulary file)
REQ-4508 ──→ TEST-4508 ──→ ADR-4502/4303 (lints)
REQ-4509 ──→ TEST-4509 ──→ CON-4503   (query CLI)
REQ-4510 ──→ TEST-4510 ──→ CON-4504 ──→ Threat Model A (projection)
REQ-4511 ──→ TEST-4511                (typed backlinks)
REQ-4512 ──→ TEST-4512                (search)
REQ-4513 ──→ TEST-4513                (migration helper)
REQ-4514 ──→ TEST-4514                (ghost/external edges)
NFR-4501 ──→ TEST-NFR-4501 ──→ OBS-4501
NFR-4502 ──→ TEST-NFR-4502 ──→ OBS-4502
NFR-4503 ──→ TEST-NFR-4503 ──→ OBS-4503
```

All cross-references in this document are `[[wikilinks]]` per
[[PROTO-001]] §Wikilinks Required In Downstream Outputs; `zetl check
--dead-links` over `specs/` is the CI gate. The intentionally-dead
links in this strawman — [[Wikilinks and Named Edges]],
[[DESIGN-045-wikilink-predicate-language]], [[Security Form Contract]],
[[Deep Context Architecture]], the `users/gardener/*` profiles — are the
working backlog of pages [[DESIGN-045-wikilink-predicate-language]] must
author or explicitly defer (they MUST NOT be deleted to clean the
report — [[PROTO-001]] §Key Technical Concepts).

---

## 13. Open Questions Surfaced by This Strawman

1. **Q1 — Multiple predicates on one link.** Can `derived_from::
   informed_by::[[X]]` express two relations to one target? The guide
   doesn't, and the AST field is singular. Lean: no (write two list
   items); confirm in [[DESIGN-045-wikilink-predicate-language]].
2. **Q2 — Predicate on an `![[embed]]`.** [[#ADR-4501]] says no
   (transclusion isn't a typed relation). Is there a real use case for
   `summarises::![[X]]`? Lean: no; revisit if a profile demands it.
3. **Q3 — Inline vs list-item annotation.** Inline-form edges have no
   natural place for the indented annotation (REQ-4504 captures it only
   in list-item form). Is that asymmetry acceptable? Lean: yes — the
   guide's annotation idiom is list-structured.
4. **Q4 — Bidirectional / inverse predicates.** Should `supersedes::`
   auto-surface an inverse `superseded_by` on the target's backlinks?
   The typed-backlink renderer (REQ-4511) shows the *incoming* edge with
   its forward predicate; an inverse-label map (`supersedes` →
   "Superseded by") could live in `.zetl/predicates.toml`. Deferred.
5. **Q5 — Trust scoping default for projected facts.** Should `zetl
   reason` default to trusting only *self-asserted* facts (page A's facts
   about A), requiring explicit opt-in for A-about-B facts
   ([[#Threat Model A]])? This is a Tier-2 security decision for
   [[DESIGN-045-wikilink-predicate-language]] + human review.
6. **Q6 — Interaction with [[SPEC-042]] public pages.** Typed ghost
   edges and the [[SPEC-042#REQ-4213]] redaction policy
   ([[#Threat Model D]]) — confirm no new leak surface.
7. **Q7 — Category of an undeclared predicate in the backlink renderer.**
   REQ-4511 orders groups by the five categories; an undeclared
   predicate has no category. Where does it sort? Lean: a sixth
   "Other (undeclared)" group before untyped.

---

**END OF STRAWMAN SPEC-045**
