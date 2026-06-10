---
id: SPEC-045
title: "Wikilink Predicate Language — typed named edges over `[[wikilinks]]`"
version: 0.1.9-strawman
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
  - v0.1.1 (design-conversation revision, 2026-06-09): the vocabulary is
    now EMERGENT BY DEFAULT — `.zetl/predicates.toml` is dropped as a
    seeded artefact and reframed as an OPTIONAL, opt-in **strict**
    vocabulary file (three-state trajectory: emergent → crystallising →
    controlled). The observed corpus predicate set IS the vocabulary;
    lints run against usage, not a registry. The file conveys
    GOVERNANCE/PRESENTATION metadata only — never semantic meaning, which
    lives in the predicate name + the target node + SPL rules (this is not
    an ontology system). Adds the template-variable contract (REQ-4515 /
    CON-4506) for theme exposure. Rewrites REQ-4507, REQ-4508, REQ-4511,
    ADR-4502, CON-4502; vocabulary governance otherwise defers to
    [[SPEC-044]].
  - v0.1.2 (design-conversation revision, 2026-06-09): resolves Open
    Question Q1 — **multiple predicates per link are supported**, chained
    on the `::` separator (`derived_from::informed_by::[[X]]`). The AST
    `Wikilink.predicate` (singular) becomes `predicates` (array); the
    graph expands one K-predicate link to K typed edges (dedup key
    `(source,target,predicate)`); set is order-insensitive + deduped.
    Updates REQ-4501/4503/4506, CON-4501/4505, HP2, the disposition
    matrix, and the `predicate-duplicate` lint.
  - v0.1.3 (design-conversation revision, 2026-06-09): resolves Open
    Question Q4 — edges are DIRECTED; a named inverse is opt-in per edge
    via the `forward/inverse::[[X]]` pred-spec, materialising a second
    directed edge `X→S` (authoring sugar for two directed edges, not a
    semantic inverse declaration). No symmetric shorthand (dropped after
    review — backlinks already serve symmetric relations). AST `predicates`
    becomes an array of `{predicate, inverse}` specs. Adds the
    Threat-Model-A escalation (S creating an outgoing edge on X).
    Updates REQ-4501/4503/4506, CON-4501/4505, the disposition matrix.
  - v0.1.4 (design-conversation revision, 2026-06-09): correction +
    de-scope. The plain `predicate::[[X]]` edge is already BIDIRECTIONAL
    (navigable + visible from both ends via backlinks); only the
    predicate's *reading* is directional. The materialised named-inverse
    (`forward/inverse::`, v0.1.3) is therefore DEFERRED out of committed
    v1 — its job is covered by inverse-labels + SPL, and it carried the
    sole "page writes an outgoing edge onto another page" escalation. The
    AST keeps a reserved `inverse` field (null in v1) for additive
    re-introduction. Reverts the `/` grammar/expansion to deferred; Q4
    now "partly resolved, rest deferred".
  - v0.1.5 (2026-06-09): open-questions refresh — adds Q10 (typed edges in
    the interactive graph view, SPEC-028/SPEC-037 — currently exposed
    everywhere except the graph widget's data feed) and a process note that
    the DESIGN-045 plan needs syncing to the current design.
  - v0.1.6 (2026-06-09): commits the graph-data feed — adds REQ-4517 +
    CON-4508 (`predicate`/`annotation` per edge in the SPEC-028/SPEC-037
    graph feed) to enable typed-edge FILTERING in the widget; the filter/
    colour UI stays a SPEC-028/SPEC-037 amendment. Q10's data-contract part
    is now committed; rendering/UI remains open.
  - v0.1.7 (2026-06-09): per direction "don't edit old specs — add to
    SPEC-045", the graph filter/styling BEHAVIOUR is now specified here as
    REQ-4518 (filter-by-predicate, legend, colour, arrowheads, annotation-
    on-hover, within SPEC-028 FPS/LCP budgets) and specified as consumer
    behaviour over the SPEC-028/SPEC-037 widgets, leaving those specs
    unmodified. Q10 resolved in SPEC-045 (no external amendment needed);
    only palette/placement left as implementation detail.
  - v0.1.8 (2026-06-09): consistency pass resolving six review findings —
    (P1) the `Wikilink.predicates` AST shape is now ONE normative form, an
    array of `{predicate, inverse}` objects, across REQ-4503 AND CON-4505
    (the stale array-of-strings in CON-4505 is replaced; matches the
    v0.1.3/v0.1.4 object design). (P1) CURIE predicates are now explicitly
    EXCLUDED from the SPL fact projection in v1 (no collision-free functor
    mapping; they live in graph/edges/RDF instead) — REQ-4510, CON-4504,
    CON-4501, and Q9 (now RESOLVED) aligned. (P1) CON-4508 now specifies
    the concrete graphology superset for multi-predicate links —
    `options.multi: true`, dedup key `(source,target,predicate)`,
    predicate-qualified edge `key` — as a backward-compatible extension of
    SPEC-028 CON-101's `multi: false` feed (extends, never supersedes; the
    baseline is unmodified and byte-identical when no typed edges exist);
    REQ-4517 references it. (P2) typed
    edge fields in the feed moved under `attributes` (graphology import
    shape). (P2) `edges_by_predicate`/`backlinks_by_predicate` now key
    untyped edges under the reserved collision-safe sentinel `"__untyped"`
    (CON-4506, REQ-4515). (P2) authoring source-of-truth disambiguated:
    SPEC-045 v1 is BODY-INLINE only (consistent with ADR-4505 + scope); the
    frontmatter `relations:` reconciliation with SPEC-044 is additive and
    does not gate v1.
  - v0.1.9 (2026-06-09): governance consistency pass on the cross-spec
    relationship. CON-4508 no longer claims to "supersede CON-101's
    multi/dedup clauses WITHOUT amending" SPEC-028 — a self-contradiction
    (a normative clause cannot be superseded without amendment). Per the
    direction "don't amend previously-implemented specs", the typed feed is
    reframed as a **backward-compatible superset** of CON-101: it extends,
    never supersedes; the producer emits it only when typed edges exist;
    with no typed edges the feed is byte-identical to CON-101, so
    SPEC-028/SPEC-037 stay literally unmodified and their behaviour
    preserved. `multi: true` with no parallel edges is `graph.import()`-
    compatible and renders identically to the `multi: false` baseline.
    Wording aligned across CON-4508, REQ-4517, and the v0.1.7/v0.1.8
    changelog entries. No design change; SPEC-028/SPEC-037 untouched.
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
| Version      | 0.1.9-strawman                                                              |
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

Two divergences are real; this spec resolves the second in SPEC-044's
favour and the first in its own (for v1 implementability):

1. **Authoring surface — SPEC-045 v1 source of truth is BODY-INLINE.**
   SPEC-044 chose **frontmatter `relations:`** as *its* canonical store
   (validated pilot shape, generic Connections-block render) and flags
   **body-inline** `predicate::[[Target]]` as an *open Tier-1 fork*
   (SPEC-044 §9.2). SPEC-045 cannot leave that fork open and still be
   implementable: its entire contract surface — the LangSec recogniser
   ([[#CON-4501]]), the AST node ([[#CON-4505]]), the SPL projection
   ([[#CON-4504]]), the graph feed ([[#CON-4508]]) — is defined over the
   body-inline form, and [[#ADR-4505]] + [[#1.5 Scope]] deliberately
   EXCLUDE predicates from frontmatter. So the **single, settled v1 source
   of truth for SPEC-045 is body-inline `predicate::[[Target]]`**;
   frontmatter `relations:` is SPEC-044's authoring concern and is **not a
   SPEC-045 v1 surface**. This removes the earlier "one of two candidate
   surfaces, not settled" hedge, which contradicted ADR-4505.

   What [[DESIGN-045-wikilink-predicate-language]] still owns is the
   *reconciliation with SPEC-044's store* — whether (and which direction)
   to mirror between body edges and a frontmatter `relations:` view. That
   reconciliation is **additive and does NOT gate v1**: body-inline stands
   alone as the canonical authored form; any frontmatter bridge is a later
   convenience layered on top, never a precondition for parsing, the
   graph, SPL, or the graph feed.
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
3. **Emergent by default; controlled only by opt-in.** The vocabulary is
   the set of predicates **observed in the corpus** — there is no
   required declaration file. A predicate is whatever an author types;
   lints (nearest-match, `relates_to::` over-use) run against *usage*,
   not a registry (REQ-4508). A team that has earned a stable vocabulary
   MAY opt in to a strict `.zetl/predicates.toml` ([[#REQ-4507]],
   [[#CON-4502]]) — a three-state trajectory *emergent → crystallising →
   controlled* (the [[SPEC-044]] §3 arc), where strict mode can finally
   make an undeclared predicate an **error**. Declaring is the act of
   choosing to *stop* being a folksonomy; absence is the default because
   the dominant profile just wants to type the edge.
   **Meaning is distributed, never declared.** A predicate's semantics
   live in three places, none of them a config file: (a) the
   self-documenting **name** (the multi-word discipline exists for this);
   (b) the **target node** it points at (`conforms_to::[[X Form
   Contract]]` means what X says — meaning-by-reference, kept revisable);
   (c) **[[SPL]] rules** for anything machine-operational (inverse,
   transitive, symmetric, domain/range — authored defeasibly in the
   engine the edges already project into). The optional file carries
   *governance/presentation* metadata only (is-this-sanctioned, a display
   label, a grouping category) — it never says what a predicate *means*
   or *entails*. **This is not an ontology system** ([[#ADR-4502]]).
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
8. **The same predicate query serves CLI, web, templates, and search.** A
   typed backlink panel ([[#REQ-4511]]), a `zetl edges` query
   ([[#REQ-4509]]), the `page.edges` template variables ([[#REQ-4515]]),
   and a predicate-filtered search ([[#REQ-4512]]) all read the one
   labelled graph. No second index, no divergent answers, no separate
   label store — display labels are auto-derived from the predicate name
   (overridable only by a strict file).

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
- An **OPTIONAL strict-vocabulary file** `.zetl/predicates.toml` — absent
  by default (emergent vocabulary); when present, it declares the
  controlled set + governance/presentation metadata and can escalate an
  undeclared predicate to an error ([[#REQ-4507]], [[#CON-4502]]). No
  seed; carries no semantic meaning.
- **`zetl check` predicate lints** computed against *observed usage* —
  nearest-match drift warning, `relates_to::` over-use signal,
  `is_a::`→`conforms_to::` suggestion ([[#REQ-4508]]).
- A **`zetl edges` query CLI** — filter the typed graph by predicate,
  direction, source, or target; `--by-predicate` is the vocabulary-
  distribution view ([[#REQ-4509]], [[#CON-4503]]).
- **Named-edge → [[SPL]] fact projection** under `--features reason` —
  the single semantic surface ([[#REQ-4510]], [[#CON-4504]]).
- **Typed-edge template variables** — `page.edges`,
  `page.edges_by_predicate`, predicate-extended `page.backlinks`, and
  `vault.predicates`, with auto-derived labels ([[#REQ-4515]],
  [[#CON-4506]]).
- **Typed backlink rendering** in `zetl build` web output, grouped by
  predicate with annotation-driven progressive disclosure, reading the
  [[#REQ-4515]] template vars ([[#REQ-4511]]).
- **Predicate-aware search** — predicate as a filterable field on the
  search index ([[#REQ-4512]]).
- **Typed edges in the interactive graph** — `predicate`/`annotation` in
  the graph-data feed ([[#REQ-4517]], [[#CON-4508]]) AND filter-by-
  predicate + colour/legend/arrowhead styling ([[#REQ-4518]]), both
  specified here — the feed as a backward-compatible superset of CON-101,
  the styling as consumer behaviour — leaving [[SPEC-028]]/[[SPEC-037]]
  unmodified.
- **Semantic-web interop export** — `zetl export
  --format {jsonld,turtle,ntriples}` projecting typed edges to RDF
  (PROV-O provenance, SKOS vocabulary), with optional `[prefixes]` /
  `maps_to` / CURIE-authoring for standard vocabularies ([[#REQ-4516]],
  [[#CON-4507]]). SPARQL endpoint excluded.
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

   - derived_from::informed_by::[[2026 Q1 Retro]]
     - The retro both seeded and shaped this decision.
   - supersedes::[[Decision Log 2025]]
   - contradicts::[[Move Fast Doctrine]]
     - Specifically rejects the "skip the design doc" clause.
   ```
   (The first link carries **two** predicates — `derived_from` and
   `informed_by` — to the one target, chained on `::` ([[#REQ-4503]]).)
2. They run `zetl check`. No file, no fuss — the predicates join the
   emergent vocabulary; each is used consistently, so no
   `predicate-drift` note fires.
3. They run `zetl edges --from "Decision Log"` (the multi-predicate link
   expands to one row per predicate, [[#REQ-4506]]):
   ```
   Decision Log
     derived_from  → 2026 Q1 Retro
     informed_by   → 2026 Q1 Retro
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

### 3.4 HP4: A New Predicate Just Works; Drift Is Surfaced

**Preconditions:** Emergent default — the vault has **no**
`.zetl/predicates.toml`. The author writes `inspired_by::[[Some Talk]]`;
the vault already uses `informed_by` 47 times.

**Steps:**

1. The edge is graphed and projected with `predicate: "inspired_by"`
   immediately — there is nothing to declare, the corpus *is* the
   vocabulary ([[#ADR-4502]]).
2. `zetl check` emits an advisory **`predicate-drift`** note (the
   low-frequency `inspired_by` sits within nearest-match of the
   high-frequency `informed_by`):
   ```
   note[predicate-drift]: `inspired_by` (1 use) is close to
     `informed_by` (47 uses) — possible synonym/typo
     --> Notes/Idea.md:12
     = run `zetl edges --by-predicate` to audit the vocabulary
   ```
   The frequency asymmetry — not a registry — is the signal. Build
   succeeds.
3. The author either keeps `inspired_by` (a deliberate new coinage) or
   rewrites it to `informed_by` (`rg`/sed across files). Nothing forced.

**Variant — controlled vault (opt-in strict):** the team has earned a
stable vocabulary and added `.zetl/predicates.toml` with `enforce =
true` listing `informed_by` but not `inspired_by`. Now step 2 is an
**error** that fails `zetl check` / the build (`predicate-undeclared`) —
*because the team chose a controlled vocabulary*. To add `inspired_by`
they promote it into the file ([[SPEC-044]] ratification act).

**Postconditions:** In the default, invention is never blocked and drift
is still visible; error-by-default is reachable only by the explicit
opt-in. One mechanism, three states ([[#REQ-4507]]).

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

The predicate(s) bind to the single wikilink they immediately precede.

**One or more predicates** MAY precede a single wikilink, chained on the
`::` separator: `derived_from::informed_by::[[Target]]` asserts BOTH
relations to the one target (REQ-4503, [[#CON-4501]]). The predicate set
is **order-insensitive** (a set, not a sequence) and de-duplicated;
canonicalised (sorted) wherever determinism matters (projection, export).
A `[[wikilink]]` with no preceding predicate is an **untyped edge**
(`predicates: []`) and is recognised exactly as today. (Resolves Open
Question Q1 — see [[#13. Open Questions]].)

**The edge is bidirectional; the predicate's *meaning* is directional.**
`derived_from::[[X]]` on page S:

* is **bidirectionally navigable** — S→X by following the link, X→S via
  backlinks (zetl's core property); and
* is **visible from both ends** — X's typed-backlink panel shows the edge
  (`← derived_from — S`, [[#REQ-4511]]). So X *sees* the relation.
* What is directional is only the predicate's *reading* (the edge means
  "S derived_from X", never the converse) and which end owns it as an
  *outgoing* relation (S does; X sees it incoming).

A page therefore does **not** need extra syntax to make a relation
bidirectional — it already is. Where the reverse should read in X's *own*
words (`superseded_by` rather than an incoming `supersedes`), the
intended mechanisms are an **inverse-*label*** (presentation —
[[#REQ-4511]]) or, for a genuine reverse *fact*, an **SPL inverse rule**
(`(has_derivative ?t ?s) <- (derived_from ?s ?t)`) — meaning stays in
SPL, not the predicate layer ([[#ADR-4502]]).

A *materialised* named-inverse authoring form (`from/to::[[X]]` minting
an outgoing edge `X→S`) is **deferred** — see Open Question Q4
([[#13. Open Questions]]) and its trust-boundary note. It is not part of
the committed v1 surface; the data model reserves an `inverse` field
([[#CON-4505]]) so it can be added additively if a concrete need appears.

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
`src/hooks/ast/mod.rs:419`) SHALL gain an OPTIONAL `predicates` field —
an **array of predicate specs** (the authored set, possibly empty;
empty/absent ⇒ untyped). Each spec is an object `{ "predicate": string,
"inverse": string | null }`:

```jsonc
// derived_from::informed_by::[[X]]
"predicates": [ { "predicate": "derived_from", "inverse": null },
                { "predicate": "informed_by",  "inverse": null } ]
```

The `inverse` field is **reserved and always `null` in committed v1** —
it exists so the deferred materialised named-inverse form (Q4) can be
added *additively* (populating a field, not changing the array's item
type) without breaking hooks. v1 authoring never sets it.

The [[Link Graph]] expands each spec to directed edges ([[#REQ-4506]]):
the `predicate` → `S→X`, and (when present) the `inverse` → `X→S`. The
field SHALL be additive per the [[SPEC-032#NFR-3206]] additive-only
evolution rule: the AST schema minor version increments (`1.0` → `1.1`);
existing hooks that do not read `predicates` round-trip the node
unchanged; the `Embed` node (`![[…]]`) does NOT gain predicates
(transclusion is not a typed relationship — [[#ADR-4501]]).

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
`predicate: null` (empty `predicates`); `zetl edges` lists the same edges
as `zetl links`.

**Trace:** [[#TEST-4505]]; [[#3.1 HP1]].

### REQ-4506: Typed Link Graph

The [[Link Graph]] ([[graph.rs|`LinkGraph`]]) SHALL carry the predicate
on each edge (`EdgeMeta.predicate: Option<String>`, `src/graph.rs:18`).
**One graph edge carries exactly one predicate, in one direction.** A
wikilink authored on page S expands (REQ-4503) to one directed edge `S→X`
per predicate: a K-predicate link yields K edges, an untyped link yields
one untyped edge. All edges share line/annotation and are
provenance-tagged to S. The dedup key is `(source, target, predicate)`,
not `(source, target)`. (A *reverse* edge `X→S` from a materialised
named-inverse is DEFERRED — Q4; v1 produces forward edges only, and the
reverse stays a backlink.) The following SHALL gain a predicate
dimension:

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

### REQ-4507: Optional Strict-Vocabulary File

The vocabulary is **emergent by default**: the set of predicates the
system recognises is exactly the set **observed in the corpus**, and no
declaration file is required. The system SHALL, however, read an OPTIONAL
`.zetl/predicates.toml` ([[#CON-4502]]) that lets a team move along the
*emergent → crystallising → controlled* trajectory ([[SPEC-044]] §3):

| State | File | Undeclared predicate | Label source |
|---|---|---|---|
| **Emergent** (default) | absent | valid; advisory drift lint only ([[#REQ-4508]]) | auto-derived ([[#REQ-4515]]) |
| **Crystallising** | present, `enforce = false` | warns + nearest-match | file `display`, else auto |
| **Controlled** | present, `enforce = true` | **error** (gates `zetl check` / build) | file `display`, else auto |

The file SHALL NOT be seeded. zetl SHALL NOT ship a starter
`.zetl/predicates.toml`; the guide's ≈30 predicates serve only as a
suggestion seed-bank for tooling, never as a canonical import
([[SPEC-044]] §9.2). The file is *populated by crystallisation* — a
recurring predicate is promoted into it (the [[SPEC-044]] in-situ
ratification act, which owns the promotion UX), not declared up front.

The file conveys **governance and presentation metadata only** — which
predicates are sanctioned, an optional `display` label, an optional
grouping `category`, the `enforce` flag, and an optional `maps_to`
interop binding ([[#REQ-4516]]). It SHALL NOT convey semantic meaning:
no definitions, no inverse/transitive/symmetric properties, no
domain/range constraints. Predicate meaning lives in the name, the target
node, and [[SPL]] rules (Design Principle 3, [[#ADR-4502]]).

REQ default value: a fresh or upgraded vault has **no file** (emergent),
because the dominant profile just types the edge; a controlled vocabulary
is earned by the minority of teams that have stabilised one
([[PROTO-001]] §Requirement Templates — default justification).

**Trace:** [[#TEST-4507]], [[#CON-4502]], [[#ADR-4502]]; [[#3.4 HP4]].

### REQ-4508: Predicate Lints in `zetl check`

`zetl check` (`src/main.rs:1610`) SHALL emit the following diagnostics.
In the **emergent** default (no strict file) they are advisory
(warning/info), computed against **observed corpus usage** — there is no
registry to compare against, so the corpus is the reference:

* `predicate-drift` — a low-frequency predicate within nearest-match
  distance (Levenshtein ≤ 2) of a high-frequency one, e.g. `inspired_by`
  (1 use) near `informed_by` (47 uses). The frequency asymmetry, not a
  declared list, is the typo signal.
* `predicate-relates-to-overuse` — `relates_to::` exceeding a configured
  share of a page's typed edges (the guide's "`relates_to::` trap");
  default threshold `[Provisional: > 50%]`.
* `predicate-prefer-conforms-to` — any `is_a::` edge, suggesting
  `conforms_to::[[X Form Contract]]` ([[#ADR-4503]]).
* `predicate-undeclared-prefix` — a CURIE predicate (`prefix:term`,
  [[#CON-4501]]) whose `prefix` is not declared in `[prefixes]`
  ([[#CON-4502]]); the edge is still valid (opaque), but RDF export
  ([[#REQ-4516]]) will fall back to the vault namespace.
* `predicate-duplicate` — the same predicate chained twice on one link
  (`a::a::[[X]]`, [[#CON-4501]]); de-duplicated to one edge + a hint.

WHEN a **strict** file is present (`enforce = true`, [[#REQ-4507]]):

* `predicate-undeclared` — a predicate not in the declared set —
  escalates to **error** (the team chose a controlled vocabulary; an
  unsanctioned predicate now fails `zetl check` / the build). Under
  `enforce = false` it is a warning + nearest-match against the declared
  set.

In the emergent default no predicate diagnostic is an error unless the
operator opts into the existing `--fail-on warning` (`src/cli.rs:187`);
error-by-default for undeclared predicates is reachable ONLY via an
explicit strict file ([[#ADR-4502]]).

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

* emit one fact per typed **snake** edge (untyped edges, `predicate:
  null`, are NOT projected — there is no predicate to name the relation);
* use the snake predicate directly as the SPL functor — it already
  satisfies the lowercase-snake grammar of REQ-4502, which is
  SPL-functor-safe with no transformation;
* **exclude CURIE predicates** (`prefix:localName`) from SPL projection
  in committed v1. A CURIE's `:` and camelCase have no lossless,
  collision-free mapping onto the lowercase-snake functor space (any
  underscore-based escape collides with a legitimately-authored snake
  predicate — see [[#13. Open Questions]] Q9), so v1 does NOT invent one
  ([[PROTO-001]] "be conservative in what you accept"). CURIE edges remain
  fully present in the [[Link Graph]], `zetl edges` ([[#REQ-4509]]), the
  template vars ([[#REQ-4515]]), and the RDF export ([[#REQ-4516]]) — the
  latter being the CURIE's native semantic home. Snake → SPL and CURIE →
  RDF are two projections of the one labelled graph; SPL is simply not the
  surface for the namespaced-vocabulary subset in v1;
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
predicate, reading the [[#REQ-4515]] template variables. Each group
SHALL show its predicate **label** — **auto-derived** from the predicate
name (replace `_` with space, capitalise the first letter:
`derived_from` → "Derived from"; a CURIE predicate uses its local part),
**overridden** only by a strict file's `display` ([[#CON-4502]]) — and
the count; each edge SHALL surface its annotation inline when present
(progressive disclosure — the reader assesses the edge before clicking).

Group **ordering** is a render-layer cosmetic, NOT a SPEC-045 semantic
requirement: the default is by descending count then predicate name; a
strict file's optional `category` may impose the guide's
Classification/Provenance/Structural/Lifecycle/Generative order; untyped
"see also" last. The choice of visual grammar (collapsible, right-rail,
colour-by-category) belongs to the theme layer and to [[SPEC-044]].

WHEN no page has a typed backlink, the panel renders exactly as the
pre-SPEC-045 flat list (REQ-4505 continuity).

**Trace:** [[#TEST-4511]], [[#REQ-4515]], [[#CON-4506]]; [[#3.3 HP3]].

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

### REQ-4515: Typed-Edge Template Variables

The `zetl build` page context (`src/web/context.rs:123`, `PageContext`)
SHALL expose a page's typed edges and typed backlinks to theme templates.
All additions are **additive** — existing themes that read `page.title`,
`page.backlinks[].title/slug/line/count`, etc. are unaffected
([[#REQ-4505]]):

* `page.edges` — outgoing edges, each
  `{ predicate, label, target, target_slug, is_dead, annotation, line }`.
  `predicate` is `null` for untyped edges; `label` is the **auto-derived**
  display ([[#CON-4506]]) unless a strict-file `display` overrides;
  `annotation` is `null` when absent.
* `page.edges_by_predicate` — a map `{ predicate → [edge…] }` for the
  Connections-block idiom (`{% for pred, edges in page.edges_by_predicate %}`).
  Untyped edges bucket under the reserved sentinel key `"__untyped"` (map
  keys cannot be `null`; the sentinel is collision-safe — [[#CON-4506]]).
* `page.backlinks` — EXTENDED: each entry (`src/web/context.rs:101`,
  `BacklinkEntry`) gains `predicate`, `label`, `annotation` (all `null`
  for untyped). Existing fields unchanged.
* `page.backlinks_by_predicate` — the grouped map the typed-backlink
  panel ([[#REQ-4511]]) renders from; untyped backlinks bucket under the
  same `"__untyped"` sentinel key ([[#CON-4506]]).
* `vault.predicates` — the observed predicate set with counts (the
  `zetl edges --by-predicate` data, `src/web/context.rs:24` `VaultContext`)
  for nav / index / tag-cloud widgets.

Missing fields render empty via minijinja `Chainable` undefined behaviour
(matching the [[SPEC-032]] `page.ext.*` convention,
`src/hooks/template_vars.rs:750`), so `{{ edge.annotation }}` on an
un-annotated edge is empty, not an error. No template variable carries
semantic meaning beyond `predicate` + `label` + `annotation` (ADR-4502).

**Trace:** [[#TEST-4515]], [[#CON-4506]]; [[#3.3 HP3]]; [[SPEC-044]] §6
(the earthian Connections block consumes these vars).

### REQ-4516: Semantic-Web Interop Export

`zetl export` (`src/main.rs:3578`) SHALL gain RDF serialisations
`--format {jsonld,turtle,ntriples}` projecting each typed edge to a
triple: subject = source-page IRI, predicate = the predicate's IRI,
object = target-page IRI (or a literal for an external/ghost target).
The projection SHALL:

* mint page IRIs and un-mapped predicate IRIs in the **vault's own
  namespace** (configurable base IRI); it SHALL NOT force predicates into
  any external ontology (ADR-4502 ruling 4 — translate, don't converge);
* expand a CURIE predicate (`prov:wasDerivedFrom`) or a snake predicate
  carrying `maps_to` ([[#CON-4502]]) to its full IRI via `[prefixes]`;
* emit the edge **annotation/`note` as `rdfs:comment`** on the statement;
* emit each edge's **provenance** (`_source_page`/`_source_file`/
  `_source_line`, the same data as the SPL projection [[#CON-4504]]) as
  **PROV-O** (`prov:wasAttributedTo` / reification), so the answerability
  trail is standards-expressible;
* emit the predicate vocabulary itself as a **SKOS** concept scheme.

A SPARQL endpoint is OUT OF SCOPE (§1.5): SPL is the in-tree query
surface ([[#REQ-4510]]); the RDF export feeds any external triplestore
that wants SPARQL. Authoring in a standard vocabulary uses the CURIE
grammar ([[#CON-4501]]); zetl implements none of the imported
vocabulary's entailments (those remain SPL rules).

**Trace:** [[#TEST-4516]], [[#CON-4507]], [[#ADR-4502]]; [[SPEC-044]] §2,
§8 (RDF/JSON-LD export + triples-endpoint open questions).

### REQ-4517: Typed-Edge Graph-Data Feed (enables graph filtering)

The graph-data feed serialised for the interactive graph view
([[SPEC-028]] Sigma.js/graphology) and the 3D space graph ([[SPEC-037]])
SHALL include, **per edge**, its `predicate` (`null` for untyped) and
`annotation` (`null` when absent), sourced from the forward directed
edges of [[#REQ-4506]] ([[#CON-4508]]). This is the **data contract that
enables typed-edge filtering** in the widget — the graph SHALL be able to
show/hide and style edges by predicate (e.g. "only `contradicts`", "hide
`relates_to`"), mirroring `zetl edges --predicate` ([[#REQ-4509]]).

Because a K-predicate link emits K parallel edges between the same pair,
the feed SHALL use graphology `options.multi: true` with
predicate-qualified edge keys and place the typed fields under each edge's
`attributes` ([[#CON-4508]] gives the concrete superset of [[SPEC-028]]
CON-101's `multi: false` / `(source,target)` dedup). The addition is
additive: consumers reading only `source`/`target` are unaffected
([[#REQ-4505]]). The feed is the *data contract*; the filtering/styling
*behaviour* is [[#REQ-4518]] — both specified **here in SPEC-045**,
applied to the [[SPEC-028]] / [[SPEC-037]] graph components without
modifying those specs. Filtering is also a performance win —
hiding a predicate reduces rendered edges, helping the [[SPEC-028]]
FPS/LCP gates rather than straining them.

**Trace:** [[#TEST-4517]], [[#CON-4508]]; [[SPEC-028]], [[SPEC-037]];
[[#13. Open Questions]] Q10.

### REQ-4518: Typed-Edge Filtering & Styling in the Graph View

Reading the [[#REQ-4517]] feed, the interactive graph view ([[SPEC-028]])
and the 3D space graph ([[SPEC-037]]) SHALL support typed-edge filtering
and styling. This requirement is specified **in SPEC-045** and applies to
those components as the consumer of the feed; it does not amend their
specifications. The behaviour:

* **Filter by predicate** — the user can show/hide edges per predicate
  (multi-select), with an explicit **"untyped"** bucket for `predicate:
  null` edges. Default: all predicates visible (REQ-4505 continuity).
* **Predicate legend** — a key listing the predicates present (from
  `vault.predicates`, [[#REQ-4515]]), each entry a colour swatch + a
  show/hide toggle; counts shown per predicate.
* **Colour-by-predicate** by default; **colour-by-`category`** when a
  strict file ([[#CON-4502]]) supplies categories; untyped edges a neutral
  colour.
* **Direction** — edges render with arrowheads (edges are directed,
  [[#REQ-4506]]).
* **Annotation on demand** — an edge's annotation surfaces on
  hover/selection (progressive disclosure), NOT as an always-on label
  (labels are expensive in force layouts).

**Performance:** filtering and per-predicate colour SHALL NOT regress the
[[SPEC-028]] LCP/FPS budgets ([[SPEC-028]] TEST-201/TEST-202); colour is
a cheap per-edge attribute and hiding a predicate *reduces* the rendered
edge count. Exact palette, arrowhead style, and legend placement are
implementation details for [[DESIGN-045-wikilink-predicate-language]].

WHEN a vault has no typed edges, the graph renders exactly as the
pre-SPEC-045 view (REQ-4505).

**Trace:** [[#TEST-4518]], [[#REQ-4517]], [[#CON-4508]]; [[SPEC-028]],
[[SPEC-037]]; [[#13. Open Questions]] Q10.

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

### ADR-4502: Emergent-by-Default Vocabulary; No Ontology; Strict by Opt-In

**Status:** Proposed (strawman default — supersedes the v0.1.0 "curated
folksonomy via a seeded file" position after the 2026-06-09 design
conversation)

**Context:** A v0.1.0 draft shipped a seeded `.zetl/predicates.toml` and
warned on undeclared predicates. Review (design conversation) exposed two
problems: (1) *why declare a vocabulary at all if it's a folksonomy?* —
a seeded file is ontology-first, the exact premature-rigidity failure the
guide and [[SPEC-044]] §3 warn against; (2) everything the file did
(drift reference, display labels, the predicate set) is **derivable from
observed usage** — the corpus *is* the vocabulary. A third question
(*should the file convey semantic meaning?*) and a fourth (*should we
allow existing vocabularies like PROV-O?*) further pressured the model.

**Decision:** Four coupled rulings.

1. **Emergent by default.** No file. The recognised vocabulary is the set
   observed in the corpus; lints run against usage ([[#REQ-4508]]). This
   is the dominant-profile default.
2. **Strict by opt-in, on a trajectory.** A team MAY add a file to move
   *emergent → crystallising (`enforce=false`, warn) → controlled
   (`enforce=true`, undeclared = error)* — the [[SPEC-044]] §3 arc.
   Declaring is the act of choosing to *stop* being a folksonomy, so
   error-by-default is finally legitimate *because it was chosen*. The
   file is populated by crystallisation (the [[SPEC-044]] ratification
   act), never seeded.
3. **The file carries no semantic meaning.** Meaning lives in the
   predicate **name**, the **target node** (`conforms_to::[[X Form
   Contract]]` means what X says — revisable by reference), and **[[SPL]]
   rules** (inverse/transitive/domain-range, authored defeasibly). The
   file holds governance/presentation metadata only (`enforce`,
   `display`, `category`) + interop bindings ([[#REQ-4516]]). Putting
   semantics in the file would create two sources of truth and a second,
   weaker reasoning engine competing with SPL — rejected.
4. **Allow existing vocabularies as naming, not as semantics.** A team
   may author standard terms via the CURIE form `prefix:localname`
   ([[#CON-4501]]) with the prefix declared in `[prefixes]`
   ([[#CON-4502]]); these export to full IRIs ([[#REQ-4516]]). zetl still
   treats `prov:wasDerivedFrom` as an opaque token — it implements none
   of PROV-O's entailments (those remain SPL rules). "Translate, don't
   converge" (guide / [[SPEC-044]] §9): plain-words + `maps_to` is the
   default; native CURIE authoring is the deliberate opt-in.

**Consequences:** (+) Resolves the folksonomy paradox: you only declare
when you've chosen control. (+) Honours naming sovereignty; invention is
never blocked. (+) Not an ontology system — the "semantic web died on
agree-first" failure is avoided by construction. (+) Standard-vocab
interop is available without importing standard-vocab rigidity. (−) An
emergent vault accumulates drift if no one runs `zetl edges
--by-predicate`; mitigated by surfacing `predicate-drift` on every `zetl
check`. (−) Three states + a CURIE grammar branch are more surface than
"one seeded file"; justified by keeping the *default* (type an edge,
nothing else) maximally simple.

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
authors at the point of use. (+) Non-coercive — runs against observed
usage, needs no file. (−) Some authors will dismiss the suggestion
repeatedly; the warning is suppressible only by opting into a strict file
and declaring `is_a` in it with an explicit `accept_is_a = true` marker
([[#CON-4502]]) — in the emergent default the suggestion is advisory and
simply ignorable.

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
named-edge      = 1*( predicate "::" ) wikilink   ; one OR MORE predicates
predicate       = snake-pred / curie-pred
; DEFERRED extension (Q4): a `predicate "/" predicate` pred-spec would add
; a named inverse; NOT in committed v1 — a `/` in the predicate region
; falls to text in v1.
snake-pred      = lower *( lower / DIGIT / "_" )   ; emergent default
curie-pred      = prefix ":" localname            ; standard-vocab opt-in
prefix          = lower *( lower / DIGIT )
localname       = ALPHA *( ALPHA / DIGIT / "_" / "-" )  ; camelCase allowed
lower           = %x61-7A                 ; a-z
wikilink        = "[[" target [ "#" heading ] [ "#^" block-id ]
                  [ "|" alias ] "]]"
; target / heading / block-id / alias as defined by the existing
; SPEC-032 wikilink grammar (src/hooks/ast/mod.rs:419). The named-edge
; grammar adds ONLY the `predicate "::"` prefix (one or more, chained).
; The grammar stays REGULAR and is parseable WITHOUT config: a `prefix:`
; whose namespace is undeclared still parses (it is an opaque predicate);
; declaration only affects RDF export (REQ-4516) and the
; `predicate-undeclared-prefix` lint (REQ-4508).
```

**CURIE form (REQ-4516, ADR-4502 ruling 4):** a single internal `:`
introduces a namespaced standard term, e.g. `prov:wasDerivedFrom::[[X]]`
→ `predicate = "prov:wasDerivedFrom"`. Recognition is unambiguous because
the recogniser keys on the `::[[` sequence; the lone `:` inside the
predicate is distinct from the `::` separator. A **bare** camelCase token
(`wasDerivedFrom::[[X]]`, no prefix) is deliberately NOT a predicate —
standard terms MUST be namespaced — so plain lowercase-snake remains the
only un-prefixed form and no accidental capitalised predicates arise. A
CURIE predicate is a first-class graph/query/RDF citizen but, because its
`:`/camelCase have no collision-free SPL-functor mapping, is **excluded
from the SPL fact projection** in committed v1 ([[#REQ-4510]],
[[#CON-4504]]; the snake form is the SPL surface, the CURIE form the RDF
surface — see Q9).

**Multiple predicates (REQ-4503):** one or more predicates may be chained
on `::` before a single wikilink — `derived_from::informed_by::[[X]]`.
The recogniser keys on the final `::[[`, then walks back over the
`1*( predicate "::" )` run; every segment must satisfy the grammar or the
whole run fails to text (conservative). The set is order-insensitive and
de-duplicated (`a::a::[[X]]` → one edge + a `predicate-duplicate` lint);
canonicalised (sorted) for the projection ([[#CON-4504]]) and export
([[#CON-4507]]).

**Named inverse — DEFERRED (Q4).** A `forward/inverse::[[X]]` form that
materialises a reverse edge `X→S` is NOT in committed v1; a `/` in the
predicate region falls to text. The plain form is already bidirectionally
navigable + visible (REQ-4501); the reverse, when wanted, is an
inverse-*label* or an SPL rule. See [[#13. Open Questions]] Q4 for the
deferred form and its trust-boundary note.

**Binding rule:** the predicate run binds to the single `wikilink` that
*immediately* follows the final `::` with no intervening character
(including whitespace). `derived_from:: [[X]]` (space after `::`) is NOT a
named edge — it is the text `derived_from::` followed by an untyped
wikilink.

**Pre-conditions:**
- Input is a recognised page body (frontmatter, fenced code, inline
  code, and HTML comments are excluded by the existing extractor —
  `src/scanner.rs`, `src/search_index.rs:183`). A `::` inside a code
  fence is never a predicate.

**Post-conditions:**
- A recognised named edge yields exactly one [[Wikilink]] node whose
  `predicates` array holds one `{predicate, inverse}` spec per chained
  segment, with `inverse` always `null` in committed v1 (REQ-4501,
  REQ-4503).
- An unrecognised `::`-adjacent token yields the disposition below.

**Malformed-token disposition (REQ-4502):**

| Source | Disposition |
|---|---|
| `derived_from::[[X]]` | named edge, `predicates=[{derived_from, ∅}]` → 1 directed edge `S→X` |
| `derived_from::informed_by::[[X]]` | named edge, two specs → 2 edges `S→X` |
| `prov:wasDerivedFrom::[[X]]` | named edge (CURIE); `predicate-undeclared-prefix` lint if `prov` not in `[prefixes]` |
| `[[X]]` | untyped edge, `predicates=[]` |
| `derived_from/has_derivative::[[X]]` (any `/`) | v1: NOT a named edge — `/` falls to text + untyped edge (the materialised named-inverse form is DEFERRED, Q4) |
| `derived_from:: [[X]]` (space) | text `derived_from::` + untyped edge |
| `wasDerivedFrom::[[X]]` (bare camelCase, no prefix) | NOT a predicate (un-prefixed terms are lowercase-snake); text + untyped edge |
| `Derived_From::[[X]]` (uppercase, no prefix) | NOT a predicate; text + untyped edge |
| `derived_from:::[[X]]` (triple colon) | `syntax` diagnostic; no edge from the malformed token |
| `derived from::[[X]]` (space in name) | text `derived from::` + untyped edge |

**Error model:** malformed tokens never produce a typed edge by
guessing. The recogniser is conservative (fail to untyped or to
diagnostic), never permissive.

**Implements:** [[#REQ-4501]], [[#REQ-4502]].
**Verified by:** [[#TEST-4501]], [[#TEST-4502]].

### CON-4502: `.zetl/predicates.toml` Schema (Optional)

**Interface:** the OPTIONAL strict-vocabulary file (REQ-4507), parsed
alongside the existing config (`src/parsers/mod.rs:200`,
`src/web/auth/config.rs` lens pattern). **Absent by default** — its
absence is the emergent vocabulary, not an error. It is NEVER seeded.
It carries **governance/presentation/interop metadata only — no semantic
meaning** (ADR-4502 ruling 3).

```toml
# .zetl/predicates.toml — OPTIONAL. Present only when a team opts into a
# crystallising/controlled vocabulary. Not shipped; populated by
# crystallisation (SPEC-044 ratification), not seeded.

enforce      = true    # true  → undeclared predicate is an ERROR (controlled)
                       # false → warn + nearest-match    (crystallising)
                       # (file absent entirely           → emergent default)
accept_is_a  = false   # true  → suppress predicate-prefer-conforms-to (ADR-4503)

# Interop prefix map (REQ-4516): CURIE prefix → namespace IRI. Enables
# `prov:wasDerivedFrom::[[X]]` authoring and full-IRI RDF export.
[prefixes]
prov    = "http://www.w3.org/ns/prov#"
dcterms = "http://purl.org/dc/terms/"

# The declared (controlled) set. Each entry is a predicate; the VALUE is
# governance/presentation metadata, never a definition.
[predicates]
derived_from   = { display = "Derived from", category = "provenance", maps_to = "prov:wasDerivedFrom" }
informed_by    = { display = "Informed by", category = "provenance" }
extracted_from = { display = "Extracted from", category = "provenance", construction = true }
conforms_to    = { display = "Conforms to", category = "classification", maps_to = "dcterms:conformsTo" }
supersedes     = { display = "Supersedes", category = "lifecycle", maps_to = "dcterms:replaces" }
contradicts    = { display = "Contradicts", category = "structural" }
relates_to     = { display = "Relates to", category = "structural", catch_all = true }
```

**Pre-conditions:**
- Top level: `enforce` (bool, default `true` when the file exists),
  `accept_is_a` (bool), `[prefixes]` (map of CURIE-prefix → IRI string),
  `[predicates]` (the declared set). `deny_unknown_fields` at top level.
- Each `[predicates]` key MUST satisfy the [[#CON-4501]] grammar (snake
  or CURIE).
- Per-predicate value fields, ALL optional and ALL administrative:
  `display` (label override; default is auto-derived — [[#REQ-4515]]),
  `category` (one of classification/provenance/structural/lifecycle/
  generative — a *grouping cosmetic* for rendering, [[#REQ-4511]], NOT a
  semantic class), `maps_to` (a CURIE for RDF export, [[#REQ-4516]]),
  `construction = true` (marks scaffolding predicates for an "upgrade me"
  lint), `catch_all = true` (marks `relates_to::` for the over-use lint).
- **No field expresses meaning:** no `definition`, `inverse_of`,
  `transitive`, `domain`, `range`. Those, if wanted, are SPL rules
  ([[#REQ-4510]], ADR-4502).

**Post-conditions:**
- A predicate present in `[predicates]` is *declared*; under `enforce =
  true` an undeclared predicate is an error, under `enforce = false` a
  warning ([[#REQ-4508]]).
- A `prefix` present in `[prefixes]` resolves CURIE predicates to full
  IRIs on export ([[#REQ-4516]]); an undeclared prefix lints but the edge
  is still valid.
- `display`/`category` feed the template vars + render ([[#REQ-4515]],
  [[#REQ-4511]]); absent ⇒ auto-derived label, count-ordered groups.

**Error model:** a malformed file is a startup/`check`-time config error
(fail closed — [[LangSec]] full recognition), distinct from a content
warning. The vault still builds the graph and the emergent vocabulary
still works; only the *strict governance signal* is unavailable until the
file parses ([[#Threat Model C]]).

**Implements:** [[#REQ-4507]], [[#REQ-4508]], [[#REQ-4511]], [[#REQ-4516]].
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
- The predicate is a **snake** predicate (`^[a-z][a-z0-9_]*$`), which is a
  valid SPL functor by construction (used verbatim, no transformation).
- **CURIE predicates are NOT projected** in v1 (REQ-4510): there is no
  collision-free functor mapping for `prefix:localName` (see Q9), so they
  are skipped here and reach the reasoner only if an author writes an
  explicit SPL rule. They still export to RDF ([[#REQ-4516]]).

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

**Error model:** a snake predicate is always a valid functor by the
REQ-4502 grammar, so projection of a snake edge cannot fail on functor
shape (a defence-in-depth assertion still guards it). A CURIE predicate is
**deliberately skipped** (not projected, not an error) per the v1
exclusion above; the skip is observable via the same diagnostic channel at
verbose level so the asymmetry with RDF export is never silent.

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
- Adds OPTIONAL property `predicates` — an **array of predicate-spec
  objects** (default `[]`); empty/absent ⇒ untyped. This is the SAME shape
  REQ-4503 specifies (the single normative AST shape — the field is an
  array of objects, NOT an array of strings). Each item is
  `{ "predicate": string, "inverse": string | null }`, where `inverse` is
  RESERVED and always `null` in committed v1 (it exists so the deferred
  materialised named-inverse (Q4) can populate a field additively rather
  than change the item type from string→object — [[#REQ-4503]]). Schema
  sketch:

  ```jsonc
  "predicates": {
    "type": "array",
    "items": {
      "type": "object",
      "additionalProperties": false,
      "required": ["predicate", "inverse"],
      "properties": {
        "predicate": { "type": "string",
          "pattern": "^[a-z][a-z0-9_]*$|^[a-z][a-z0-9]*:[A-Za-z][A-Za-z0-9_-]*$" },
        "inverse":   { "type": ["string", "null"] }
      }
    }
  }
  ```

  A multi-predicate link is one node with multiple array entries
  (REQ-4503). The node's `required` set is UNCHANGED (the field is
  optional), preserving additive evolution ([[SPEC-032#NFR-3206]]).
- Schema minor version increments (`ast_version` `1.0` → `1.1`).
- A hook that does not read `predicates` round-trips the node byte-
  stably (the additive-evolution property test in
  `tests/ast_schema_integration.rs` gains a predicates case).

**Error model:** a `predicates` item whose `predicate` value violates the
[[#CON-4501]] grammar is a schema-validation failure (the `pattern` on
`properties.predicate` — snake OR CURIE — makes the AST contract enforce
the grammar, not just the parser; defence in depth at the hook boundary).

**Implements:** [[#REQ-4503]].
**Verified by:** [[#TEST-4503]].

### CON-4506: Page Template-Variable Contract for Typed Edges

**Interface:** the serde shape added to `PageContext` /
`VaultContext` (`src/web/context.rs:123` / `:24`) and exposed to
minijinja (`src/web/engine.rs`).

**Post-conditions (serialized shape):**

```jsonc
// page.edges[i]
{ "predicate": "derived_from" | null,   // null = untyped edge
  "label":     "Derived from",          // auto-derived; strict display overrides
  "target":    "2026 Q1 Retro",
  "target_slug": "2026-q1-retro",
  "is_dead":   false,                    // ghost/unresolved target
  "annotation": "…" | null,
  "line":      12,
  "authored_by": null }                  // RESERVED: always null in v1
                                         // (every edge is self-authored).
                                         // Reserved for the deferred
                                         // named-inverse (Q4): would name
                                         // the page that authored an
                                         // other-authored reverse edge so
                                         // the renderer can flag it.
// page.edges_by_predicate: { "<predicate>": [ <edge>, … ],
//                            "__untyped": [ <edge>, … ] }   // untyped sentinel key
// page.backlinks[i]: existing fields + "predicate"|null, "label"|null, "annotation"|null
// page.backlinks_by_predicate: { "<predicate>": [ <backlink>, … ],
//                                "__untyped": [ <backlink>, … ] }
// vault.predicates: [ { "predicate": "derived_from", "count": 14 }, … ]
```

**Auto-label function** (the default when no strict `display`): take the
predicate's local part (after any CURIE `prefix:`), replace `_`/`-` with
spaces, capitalise the first letter. `derived_from` → "Derived from";
`prov:wasDerivedFrom` → "wasDerivedFrom" (or a strict `display`). Pure,
deterministic, no I/O.

**Untyped sentinel key.** JSON object / template-map keys cannot be
`null`, so untyped edges (`predicate: null`) bucket under the reserved
string key **`"__untyped"`** in BOTH `page.edges_by_predicate` and
`page.backlinks_by_predicate`. The sentinel is collision-safe: no valid
predicate can start with `_` (snake is `^[a-z]…`, CURIE prefix is
`^[a-z]…` — [[#CON-4501]]), so `"__untyped"` can never shadow a real
predicate group. The per-edge `predicate` field stays `null` (the maps key
on the sentinel; the edge object itself does not). A template iterating
the map may special-case the `__untyped` key for a "see also" heading;
its render label is "See also" (the lone label not auto-derived from a
predicate name). The bucket is present only when ≥1 untyped edge exists.

**Pre-conditions:** additive — the addition MUST NOT remove or rename any
existing `PageContext` field ([[#REQ-4505]]). Missing per-edge fields are
JSON `null`; minijinja `Chainable` undefined behaviour renders absent
chains empty (`src/hooks/template_vars.rs:750`).

**Error model:** a template referencing a predicate that no edge uses
yields an empty list/group, never an error (mirrors [[#CON-4503]]).

**Implements:** [[#REQ-4515]], [[#REQ-4511]].
**Verified by:** [[#TEST-4515]].

### CON-4507: RDF / JSON-LD Interop Export

**Interface:** `zetl export --format {jsonld,turtle,ntriples}`
(`src/main.rs:3578`), extending the existing graph-dump exporter.

**Pre-conditions:** read-only over the vault; a configurable base IRI for
the vault namespace; `[prefixes]` ([[#CON-4502]]) for CURIE/`maps_to`
expansion (absent ⇒ all predicates export in the vault namespace +
`predicate-undeclared-prefix` lint).

**Post-conditions (per typed edge):**
- one statement `<source-iri> <predicate-iri> <target-iri-or-literal>`;
- predicate IRI = mapped IRI if the predicate is a CURIE or carries
  `maps_to`, else `<vault-ns>predicate/<name>`;
- `annotation` → `rdfs:comment` on the (reified) statement;
- provenance (`_source_page`/`-file`/`-line`) → **PROV-O**
  (`prov:wasAttributedTo` + reification), identical data to [[#CON-4504]];
- the predicate vocabulary → a **SKOS** `skos:ConceptScheme`
  (each predicate a `skos:Concept`; `maps_to` ⇒ `skos:exactMatch`).
- DETERMINISTIC, order-stable output for a fixed vault.

**Property (roundtrip-adjacent):** the triple set is information-equivalent
to the [[#CON-4504]] SPL fact set on `(predicate, source, target)` — RDF
and SPL are two projections of the one labelled graph, never divergent.

**Error model:** an un-expandable predicate never blocks export — it falls
back to the vault namespace and lints. Export never invents entailments
(no OWL reasoning); it serialises only asserted edges.

**Implements:** [[#REQ-4516]].
**Verified by:** [[#TEST-4516]].

### CON-4508: Graph-Data Edge Contract

**Interface:** the per-edge shape in the graph-data feed consumed by the
[[SPEC-028]] interactive graph and [[SPEC-037]] 3D graph. The
serialisation site is the existing graph-data producer (`src/graph.rs`,
the `graph-index.json` edge loop at `src/graph.rs:641`); the feed is
graphology's `graph.import()` format ([[SPEC-028]] CON-101).

**Graphology shape (the migration this requires).** [[SPEC-028]] CON-101
declares `options.multi: false` and the producer dedupes edges by
`(source, target)` (`src/graph.rs:641` `seen_edges`). A typed graph needs
*multiple distinct edges between the same pair* (one per predicate). So
CON-4508 defines a **backward-compatible superset** of the [[SPEC-028]]
CON-101 edge contract — it *extends*, and never supersedes or amends,
that contract (per the "add to SPEC-045, don't edit old specs"
direction). The superset is what the producer emits when typed edges are
present; when no edge carries a predicate the feed is byte-identical to
CON-101 ([[#REQ-4505]]). SPEC-028/SPEC-037 therefore remain unmodified
and their existing behaviour is preserved; folding multi-mode into the
CON-101 baseline itself is out of scope for SPEC-045 and unnecessary,
because the superset is backward-compatible. The superset differs from
the baseline only as follows:

- `options.multi` becomes `true` (parallel predicate-edges between a pair
  are first-class; graphology requires multi mode to hold them). `multi:
  true` with no parallel edges is `graph.import()`-compatible and renders
  identically to the CON-101 `multi: false` baseline, so a consumer that
  never opts into typed edges is unaffected.
- The dedup key changes from `(source, target)` to `(source, target,
  predicate)` — `a::a::[[X]]` still collapses to one edge per REQ-4503,
  but `contradicts::[[X]]` and `relates_to::[[X]]` to the same target are
  two edges.
- Each edge carries a **predicate-qualified `key`** so the keys stay
  unique under `multi: true`: `"<source>-><target>#<predicate>"`, using
  the sentinel `#__untyped` for `predicate: null` (no valid predicate
  starts with `_`, so the sentinel cannot collide — matches the
  [[#CON-4506]] map-key sentinel).
- Typed-edge fields live under **`attributes`**, where graphology's
  `graph.import()` puts them and the [[SPEC-028]] edge reducer reads them
  ([[SPEC-028]] CON-101 edge shape `{ key, source, target, attributes }`).
  They are NOT top-level keys.

**Post-conditions (per edge):**

```jsonc
{ "key":    "design-doc-discipline->move-fast-doctrine#contradicts",
  "source": "design-doc-discipline",
  "target": "move-fast-doctrine",
  "attributes": {
    "predicate":  "contradicts",         // null ⇒ key uses #__untyped
    "annotation": "…",                    // null when absent
    "is_dead":    false                   // ghost/unresolved target
  } }
```

- One feed edge per forward directed graph edge ([[#REQ-4506]]); a
  K-predicate link yields K feed edges (distinct keys, each filterable and
  colourable independently — [[#REQ-4518]]).
- `attributes.predicate`/`attributes.annotation` are additive — a widget
  reading only `source`/`target` (and pre-existing attributes) is
  unaffected ([[#REQ-4505]]); only consumers that opted into `multi: true`
  + predicate keys see the parallel edges.
- Deferred named-inverse ([[#13. Open Questions]] Q4) would add reverse
  feed edges; v1 emits forward only.

**Error model:** an absent predicate ⇒ `attributes.predicate: null` with a
`#__untyped` key, never omitted — so a filter UI can offer an explicit
"untyped" bucket ([[#REQ-4518]]).

**Implements:** [[#REQ-4517]].
**Verified by:** [[#TEST-4517]].

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
| Vocabulary lints ([[#REQ-4508]]) | Example-based (drift nearest-match vs observed frequency) + property (emergent ⇒ never errors; strict + undeclared ⇒ errors) |
| Graph / query ([[#REQ-4506]], [[#REQ-4509]]) | Example-based + mutation testing on the predicate-filter logic |
| Template vars ([[#REQ-4515]], [[#CON-4506]]) | Property (auto-label is deterministic; additive — existing theme fields unchanged) + example (Chainable empty-on-missing) |
| Interop export ([[#REQ-4516]], [[#CON-4507]]) | Property (RDF triple set ≡ SPL fact set on the triple) + example (CURIE/`maps_to` IRI expansion, PROV-O provenance round-trip) |
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

**Escalation the committed v1 AVOIDS — named-inverse edges (deferred,
Q4).** A materialised `from/to::` form would be the first construct where
page S creates an **outgoing edge on a different page** (`X→S`) that X
never authored — a stronger claim than a backlink, visible when an agent
walks X's *outgoing* edges. This is the main reason the form is **not in
committed v1**: keeping it out keeps every outgoing edge self-authored.
*If* it is later adopted, the mitigation is per-edge provenance
(`_source_page = S`, surfaced as the reserved `authored_by` template var,
[[#CON-4506]]) so consumers can distinguish self- from other-authored
outgoing edges, plus trust scoping by `_source_page` as for any forward
fact. Until then, the reverse direction is only ever a backlink (incoming),
which carries no such escalation.

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

**Mitigation:** the file is OPTIONAL and a [[LangSec]]-recognised input
with top-level `deny_unknown_fields` ([[#CON-4502]]); a parse failure is
a fail-closed config error (strict governance unavailable, loudly) — the
emergent vocabulary still works, so a broken file degrades to the safe
default rather than silently disabling a gate the team thought was on.

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
REQ-4507 ──→ TEST-4507 ──→ CON-4502 ──→ ADR-4502 (optional strict file)
REQ-4508 ──→ TEST-4508 ──→ ADR-4502/4503 (lints vs observed usage)
REQ-4509 ──→ TEST-4509 ──→ CON-4503   (query CLI)
REQ-4510 ──→ TEST-4510 ──→ CON-4504 ──→ Threat Model A (projection)
REQ-4511 ──→ TEST-4511 ──→ CON-4506   (typed backlinks)
REQ-4512 ──→ TEST-4512                (search)
REQ-4513 ──→ TEST-4513                (migration helper)
REQ-4514 ──→ TEST-4514                (ghost/external edges)
REQ-4515 ──→ TEST-4515 ──→ CON-4506   (template variables)
REQ-4516 ──→ TEST-4516 ──→ CON-4507   (RDF/JSON-LD interop export)
REQ-4517 ──→ TEST-4517 ──→ CON-4508   (graph-data feed; enables filtering)
REQ-4518 ──→ TEST-4518 ──→ CON-4508   (graph filter + styling behaviour)
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

1. **Q1 — Multiple predicates on one link. RESOLVED (2026-06-09): yes,
   supported.** `derived_from::informed_by::[[X]]` chains predicates on
   the `::` separator and asserts both relations to the one target
   ([[#REQ-4501]], [[#REQ-4503]], [[#CON-4501]]). The AST node carries a
   `predicates` array; the graph expands to one edge per predicate
   ([[#REQ-4506]]); the set is order-insensitive + de-duplicated. This
   matches [[SPEC-044]]'s SPO model (multiple triples sharing
   source+target). Remaining sub-question for
   [[DESIGN-045-wikilink-predicate-language]]: confirm chained-`::` over
   a comma-list (`a,b::[[X]]`) reads better for authors — the AST/graph
   shape is identical either way.
2. **Q2 — Predicate on an `![[embed]]`.** [[#ADR-4501]] says no
   (transclusion isn't a typed relation). Is there a real use case for
   `summarises::![[X]]`? Lean: no; revisit if a profile demands it.
3. **Q3 — Inline vs list-item annotation.** Inline-form edges have no
   natural place for the indented annotation (REQ-4504 captures it only
   in list-item form). Is that asymmetry acceptable? Lean: yes — the
   guide's annotation idiom is list-structured.
4. **Q4 — Bidirectional / inverse predicates. PARTLY RESOLVED; rest
   DEFERRED (2026-06-09).** Settled: the plain `predicate::[[X]]` edge is
   **already bidirectional** in the senses that matter — bidirectionally
   *navigable* (S→X by link, X→S by backlink) and *visible from both ends*
   (X's typed-backlink panel shows it, [[#REQ-4511]]). What is directional
   is only the predicate's *reading* and which end owns the *outgoing*
   relation. So no extra syntax is needed to make a relation bidirectional.
   **Deferred:** a materialised named-inverse authoring form
   (`forward/inverse::[[X]]` minting an outgoing `X→S` edge with a distinct
   name). It is NOT in committed v1 because (a) it's the only construct
   that makes a page write an *outgoing* edge onto another page (the
   [[#Threat Model A]] escalation), and (b) its two unique contributions —
   the reverse read in X's own words, and a reverse outgoing fact — are
   already served more cheaply by an **inverse-*label*** (presentation,
   [[#REQ-4511]]) and an **SPL inverse rule** respectively. The data model
   reserves an `inverse` field ([[#CON-4505]], always null in v1) so the
   form can be added additively if [[DESIGN-045-wikilink-predicate-language]]
   surfaces a concrete case a label + SPL can't cover. No symmetric
   shorthand under any option (backlinks already serve symmetric
   relations).
5. **Q5 — Trust scoping default for projected facts.** Should `zetl
   reason` default to trusting only *self-asserted* facts (page A's facts
   about A), requiring explicit opt-in for A-about-B facts
   ([[#Threat Model A]])? This is a Tier-2 security decision for
   [[DESIGN-045-wikilink-predicate-language]] + human review.
6. **Q6 — Interaction with [[SPEC-042]] public pages.** Typed ghost
   edges and the [[SPEC-042#REQ-4213]] redaction policy
   ([[#Threat Model D]]) — confirm no new leak surface.
7. **Q7 — Group ordering with no categories.** In the emergent default
   no predicate has a `category`, so [[#REQ-4511]] groups order by
   descending count then name. Is count-ordering the right default, or
   should first-appearance or alphabetical win? Lean: count-ordering
   (most-used relations first); the five-category order applies only when
   a strict file supplies `category`.
8. **Q8 — Which standard vocabularies to bless.** [[#REQ-4516]] supports
   CURIE authoring + `maps_to` for any namespace, but which prefixes ship
   as *recognised* (PROV-O, SKOS, Dublin Core, schema.org, CITO, …) and
   whether to validate a `maps_to` against a known term list is a
   [[SPEC-044]] vocabulary-governance call. Lean: recognise PROV-O +
   Dublin Core + SKOS by name (used internally by the export); accept any
   other prefix as opaque. Resolve in the SPEC-044 reconciliation.
9. **Q9 — SPL functor form of a CURIE predicate. RESOLVED (in
   SPEC-045): excluded in v1.** A CURIE's `:` and camelCase are not
   SPL-functor-safe, and no underscore-based sanitisation is collision-free
   — `prov:wasDerivedFrom` → `prov__wasderivedfrom` would collide with a
   legitimately-authored snake predicate `prov__wasderivedfrom`, so any
   such mapping silently merges distinct edges into one fact. Rather than
   ship a lossy transform, committed v1 **does not project CURIE predicates
   to SPL** ([[#REQ-4510]], [[#CON-4504]]); they reach the reasoner only
   via an author-written SPL rule, and they export losslessly to RDF
   ([[#REQ-4516]]), which is their native semantic surface. A future,
   collision-free functor mapping (e.g. a reserved separator that the snake
   grammar forbids) is deferred to
   [[DESIGN-045-wikilink-predicate-language]] — it would be additive (more
   facts), never a behaviour change to existing snake projection.
10. **Q10 — Typed edges in the interactive graph view. RESOLVED (in
    SPEC-045).** Both the data feed ([[#REQ-4517]], [[#CON-4508]]) and the
    filtering/styling behaviour ([[#REQ-4518]]) are specified **here in
    SPEC-045**: the feed is a *backward-compatible superset* of the
    [[SPEC-028]] CON-101 contract ([[#CON-4508]]) and the styling is
    *consumer behaviour* over the [[SPEC-028]] / [[SPEC-037]] graph
    components — neither amends those specs. Committed: filter-by-
    predicate (with an "untyped" bucket), a predicate legend from
    `vault.predicates`, colour-by-predicate (or `category`), directional
    arrowheads, annotation on hover/selection, all inside the [[SPEC-028]]
    FPS/LCP budgets. Remaining are implementation details for
    [[DESIGN-045-wikilink-predicate-language]] (exact palette, arrowhead
    style, legend placement), not open design questions.

> **Process note (not a design question):** the
> [[DESIGN-045-wikilink-predicate-language]] plan still describes the
> v0.1.0 seeded-vocabulary, singular-predicate model and must be synced to
> the emergent-vocabulary + multi-predicate + deferred-inverse design
> before it is executed.

---

**END OF STRAWMAN SPEC-045**
