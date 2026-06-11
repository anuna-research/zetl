---
title: "SPEC-044: Concept Graph — Subject-Predicate-Object Relations, Emergent Vocabulary, and In-Situ Ratification"
version: 0.4.0
status: strawman
date: 2026-06-11
audience: agent, human
parent: null
related:
  - SPEC-026  # Vault scan exclusions (the .zetlignore layer the corpus view rides on)
  - SPEC-043  # --no-gitignore + first-class .zetlignore (the corpus view this graph lives in)
  - SPEC-045  # Wikilink predicate language (the body-inline `::` substrate this layer rides on)
  - SPEC-012  # Named themes (the render layer the graph surfaces through)
  - SPEC-005  # Defeasible reasoning / SPL (consumes the triples)
---

# SPEC-044: Concept Graph — SPO Relations, Emergent Vocabulary, In-Situ Ratification

> **Status: strawman (v0.4.0).** This document sets out a direction and a
> validated manual pilot, not a finished design. Schemas, thresholds, and
> endpoints are provisional. It is offered for review and revision. v0.4.0
> adds the **Tier-1 design** (§6 reconciled to body-inline edges as the source
> of truth; §7.1 ratify-not-fill as the *accessibility mechanism*; §7.2 the
> five predicate families; §7.3 user-local vs shared-corpus settings; §7.4 the
> core / plugin / MCP boundary), grounded by the 2026-06-11 dogfood of SPEC-045
> on the EarthianLabs corpus. v0.3.0 added §1.2 (contexts of use, as job
> stories), which establishes the surface-coverage scope that distinguishes an
> organisational product from an author's toolkit.

## Information Table

| Field        | Value                                                                 |
| ------------ | --------------------------------------------------------------------- |
| Document ID  | SPEC-044                                                               |
| Title        | Concept Graph — SPO Relations, Emergent Vocabulary, In-Situ Ratification |
| Version      | 0.3.0 (strawman)                                                       |
| Status       | Strawman                                                               |
| Author       | Mat Mytka & Kairos (m3-kairos dyad)                                    |
| Date         | 2026-06-09                                                             |

---

## 1. The problem

A markdown corpus authored as *documents* (notes, reports, papers) is fractured:
the concepts that recur across it live implicitly, distributed in prose, rather
than as nodes with explicit edges. Full-text search finds strings; semantic
(embedding) search finds nearby meaning; neither yields a **navigable structure**
— how concepts interrelate, where each is first developed versus merely used, what
its lineage is. The corpus is not a graph because it was never authored as one.

Two reader modes need this graph, and the same substrate must serve both: a
**person** browsing associatively, and a **machine** querying it as structured
facts. The design must not collapse the first into the second.

This is intended as a **general capability**, not infrastructure specific to one
corpus. Turning a markdown corpus into a typed, navigable, machine-reasonable
knowledge graph generalises to organisational knowledge management and human-AI
collaboration, so the design must be transferable rather than tied to any one
domain vocabulary.

### 1.1 Framing: a learning system, not a knowledge store

A knowledge-management system used alongside AI should function as a **learning
system — for both humans and machines**, not merely a faster store. The rationale
is the present condition of knowledge work: human-AI collaboration produces
knowledge artefacts faster than people can absorb them. Berardi's distinction
between *connection* (fast, formatted, machine-compatible exchange) and
*conjunction* (slower, embodied, situated meaning-making) names the risk: a tool
that only stores and retrieves faster accelerates the overload it claims to
relieve.

The design response is to make the system's core act a **learning act** whose
by-product is the graph edge, rather than a data-entry act that happens to teach.
This draws on two well-established findings: the **generation effect** (material a
person generates is retained better than material merely read; Slamecka & Graf,
1978) and **desirable difficulty** (appropriate effort during encoding improves
durable learning; Bjork, 1994). It also addresses the **irony of automation**
(Bainbridge, 1983): when a system performs the cognitive work, the human loses the
practice that competence depends on. Locating a small, deliberate learning act at
the point of comprehension keeps the human coupled to the knowledge the system
records.

### 1.2 Contexts of use (job stories)

User needs are expressed as **job stories** — *when [situation], I want to
[motivation], so I can [outcome]* (Klement, 2013; Christensen et al., 2016) —
rather than persona profiles. The surface a person needs is determined by the
situation they are in, not by a fixed identity: a single person occupies several
of the jobs below over time. A job story is included here only if it forces a
**distinct authoring/consumption surface or answerability path**; that gate keeps
the set from degrading into unsituated personas.

| # | Job story | Surface it forces | Answerability path |
|---|-----------|-------------------|--------------------|
| 1 | **Ingest.** When I first open the tool on a document set that is not yet a graph, I want latent structure surfaced and proposed in bulk, so I can make an existing corpus navigable without hand-editing every file. | Machine-surfaced candidates + bulk ratify | Ratification act + surfaced evidence |
| 2 | **Capture (non-specialist).** When I notice two things relate while doing my domain work, I want to record the relationship without learning a markup syntax, so my knowledge enters the graph. | Suggestion + ratify GUI (no markup) | Prompted rationale at ratify-time |
| 3 | **Author in flow.** When I already know a relationship as I write, I want to type the typed edge inline, so I do not break composition to use a separate tool. | Body-inline `predicate::[[Target]]` | The surrounding prose |
| 4 | **Orient.** When I reach a document I did not write, I want its typed relationships and their rationale shown, so I can spend attention on the edges that change its meaning. | Read-only typed-backlink render + progressive disclosure | Consumed (rationale shown in place) |
| 5 | **Ask.** When I have a question spanning the corpus, I want to query the typed graph and get a traceable answer, so I can trust and verify it. | Query: CLI/logic, or a query-builder for non-specialists | Provenance on the answer |
| 6 | **Steward.** When the vocabulary has drifted or sprawled, I want to see its real distribution and merge, prune, or promote predicates, so the graph stays coherent. | Audit + promote/deprecate GUI | The governance decision, recorded |
| 7 | **Traverse (machine).** When an agent builds context for a task, it wants typed edges plus provenance in machine-readable form, so it can choose what to traverse, weight claims by asserter, and propose edges back during authoring. | Programmatic (structured/JSON) | Assertion-provenance (source page/line) |
| 8 | **Inherit.** When I join a team and inherit a mature graph, I want to re-derive its load-bearing relationships rather than be handed them, so the knowledge becomes mine. | Guided re-ratification GUI | Prior provenance as scaffold + re-ratification |

Three consequences shape the scope of this spec:

- **Coverage, not personas, separates a product from a toolkit.** The eight jobs
  force eight distinct surfaces. A typed-edge engine (the companion engineering
  spec, SPEC-045) specifies the inline-authoring surface (job 3) and the read-only
  render (job 4); the remaining surfaces — bulk ratify, non-specialist capture,
  query-builder, stewardship, re-ratification — are the scope of *this* spec, and
  are what make the capability usable by an organisation rather than only by
  authors fluent in the markup.
- **Ingest and inherit (jobs 1, 8) are onboarding-phase, not core.** They run when
  a corpus or a person first meets the graph, produce a ratified graph, and hand
  off to the steady-state core. They belong in a separable onboarding layer, not
  the core parser — which relocates (rather than removes) the question of where
  onboarding-produced edges are materialised for the core to read.
- **Reading is the high-frequency job.** Job 4 (*orient*) is the most common
  context: most participants consume the graph and never author an edge. The
  authoring jobs (1–3) exist so that consumption pays off; the product value is
  largely downstream of them.

> **Answerability has two production paths** (see §5.2). Job 3 yields it for free —
> the surrounding prose *is* the rationale. Jobs 1, 2, 6, 8 produce it from the
> ratification act itself (surfaced evidence plus a prompted rationale), because
> there is no authored prose to carry it. One answerability layer, filled two ways.

## 2. The model: Subject-Predicate-Object

Relations are expressed as **SPO triples** — a transferable, machine-reasonable
substrate, deliberately chosen as the universal structure.

- **Subject** — the page itself (implicit; the file *is* the subject).
- **Predicate** — the relation type.
- **Object** — the target (another concept node, or a document).
- **Qualifier** — an optional human-readable `note` carrying the rationale for the
  relation. (See §5.2: this field is load-bearing, not decorative.)

Authored as **lightweight frontmatter** (shape validated by the pilot):

```yaml
relations:                 # concept → concept
  - type: measured-by
    target: semantic-coupling
    note: "the disposition vs the measured dynamic"
developed-in:              # concept → document (primary)
  - research/relational-posture-framework-sketch.md
documented-in:             # concept → document (typed, operative-elsewhere)
  - type: operationalised-in
    path: papers/cross-substrate-coupling-preprint.md
    note: "..."
```

**Not raw RDF.** Efforts requiring agreement on formal ontologies with URIs before
authoring have a poor adoption record. The recommendation is to author the
friendly frontmatter, treat it as SPO semantically, and offer **RDF / JSON-LD
export** for interoperability where an organisation needs it. Derivation is
mechanical: for each page, for each relation, emit `(page, predicate, object,
note)`.

**Reasoner-ready.** SPO triples are directly consumable by a defeasible reasoner
(SPL, SPEC-005). The machine-reasoning graph and the SPO model are the same
artefact viewed from two directions.

## 3. Vocabulary: emergent and controlled (the folksonomy–taxonomy trajectory)

The **structure** (SPO) is universal and ships with the tool. The **predicate
vocabulary** is domain-specific and is **data, not code** — each corpus declares
its own (a research corpus might use `measured-by` / `developed-in` /
`operationalised-in` / `applied-in`; an organisation might use `decided-in` /
`supersedes` / `owned-by` / `depends-on` / `contradicts`).

This is the long-standing **folksonomy versus taxonomy** tension (Vander Wal,
2007; Shirky, 2005). A *folksonomy* is bottom-up, emergent, low-cost, and
flexible; a *controlled vocabulary* (taxonomy) is top-down, governed, and supports
validation and reasoning guarantees, at the cost of rigidity. The design treats
these not as a binary toggle but as a **trajectory**: predicates are emergent by
default and crystallise toward controlled as they prove out.

1. New predicates accrete freely as edges are drawn (emergent vocabulary is the
   default).
2. Recurring predicates (e.g. one used across many nodes) can be **promoted** into
   a declared, controlled vocabulary — and that promotion is itself a ratified
   decision (e.g. "this predicate has been used N times — promote it?").
3. An organisation moves toward a controlled vocabulary only as far, and as fast,
   as the corpus can sustain.

Premature rigidity is a documented failure mode: technically (the brittleness of
mandatory upfront ontologies), epistemically (apparent agreement that has not
actually been reached), and in maintenance terms (a controlled vocabulary larger
than its maintainers can keep coherent becomes overhead rather than value). The
controlled vocabulary should therefore be corpus-local; cross-corpus
standardisation is explicitly out of scope (see §9.2).

> **Configurable formalisation is a product feature** — but framed as *flexibility
> that can be tightened*, not *rigidity that can be loosened*. Emergence
> crystallises into structure only as fast as it is absorbed.

### 3.1 Learning is the objective; the graph is the by-product

A constraint that follows from §1.1: **the graph is the by-product of learning,
never the target.** If the activity is optimised to maximise edge production, it is
subject to Goodhart's law — the measure becomes the target and ceases to track the
goal — and the learning act degrades into output. Two **do-not-build** constraints
follow:

- **No edge-count metric.** No completion bars, no "N relations created" counters,
  no acceptance-rate to optimise. Metering edge-production re-introduces the
  production pressure the system exists to counter.
- **The success signal is re-derivability, not volume.** A healthy graph is one
  whose edges still cohere when a new reader re-examines them — not one with the
  most edges.

## 4. Emergent does not mean blank-page: the predicate must be suggested

A load-bearing usability constraint: a person should never be forced to invent a
predicate from a blank field. Emergence without suggestion is merely friction.
When an edge is drawn, the system **proposes predicates**:

- existing-vocabulary matches, ranked by fit, **plus**
- a proposed-new predicate inferred from the two endpoints' content.

The person **ratifies or overrides**. The suggestion narrows a wide field of
candidates so the person can recognise the fitting one — an act of relevance
realisation (Vervaeke et al., 2017). This is the same **ratify-not-fill**
principle as §5 (confirm a proposal rather than complete a blank form), applied at
the vocabulary layer. Without it, an "emergent vocabulary" is just an empty text
box.

## 5. In-situ ratification (the core workflow)

The ratification loop, validated by the manual pilot:

1. **Surface (machine, wide recall):** background processes traverse the corpus
   and pre-compute **candidate edges and suggested predicates** from existing
   retrieval signals (full-text, semantic/embedding, backlinks).
2. **Present in context (human):** when a file is opened in the renderer, its
   pending candidate decisions are already waiting, shown alongside it. The
   decision is co-located with reading and understanding the material, so it
   cannot be silently offloaded to the machine. This addresses automation
   complacency (Bainbridge, 1983) at the interface level.
3. **Ratify (one act = confirm + type):** the system pre-proposes a verdict
   (predicate plus evidence); the person accepts, overrides, rejects, or
   **reroutes** (assigns to a different concept node). This is **ratify-not-fill**:
   confirming a proposal, not completing a blank form.
4. **Commit:** on confirmation, the triple is written to the subject's frontmatter.

The same pattern — *candidate decisions ratified in context* — covers more than
relation edges: frontmatter generation, orphan linking, concept extraction (new
nodes to create), dead-link resolution, and predicate promotion (§3).

### 5.1 Open fork: where pending decisions live

- **In-frontmatter** (as the manual pilot did): simple, but background processes
  then write unratified machine guesses into every source file and may contend
  with one another.
- **Sidecar** (e.g. a `.zetl/decisions/` queue the renderer overlays): source
  files stay clean; a decision is written into the file only on confirmation,
  keeping "confirmation is what commits it" literally true. **Recommended:
  sidecar.**

### 5.2 Answerability: provenance of the cut

A ratification is a **cut** — selecting one edge or predicate and rejecting or
rerouting the alternatives. Such cuts are not neutral; the tool participates in
what it sorts. The governance response is not *wiser cuts* alone — the most
damaging sort-errors are invisible from within the same frame that produced them
(Ashby's law of requisite variety, 1956: a controller cannot fully regulate
variety it cannot perceive) — but **answerable cuts**: the commit records the
cut's *provenance*, not only the triple. Provenance includes what the retrieval
surfaced, what was chosen, what was rejected or rerouted and why, who ratified,
and when.

This provenance is **available, not obligatory**: collapsed and dormant by
default, expandable by whoever needs it. Most readers never open it; the point is
that they *can*. This is the correct form of explainability — never *forcing*
every reader through the deliberation (which re-creates the overload), and never
*foreclosing* it. It also keeps a cut revisable by those it affects, including
those not present when it was made.

This is the same three-level structure as **progressive disclosure** (cf. Allen,
2026, §9): predicate gives direction, the `note`/rationale gives the reason, the
target gives the full depth. The `note` qualifier is therefore not an afterthought
field but the **answerability layer** — the recorded reason an edge exists.

> **Worked example (manual pilot).** One ratification pass recorded its cut inline
> in the file: a frontmatter comment naming what was dropped and why (e.g. "X
> belongs to a different concept node"; "Y is too generic — a tag, not a
> relation"), a `note` on each kept edge, and the pass dated in the body. The
> capability is one formalisation step from the manual pilot.

### 5.3 Grade the friction by stakes

Not every edge warrants the same depth. Most knowledge can safely become a
"black box" used without re-derivation (one need not understand fibre-optic
physics to use the internet); some is load-bearing and must remain understandable.
Friction-depth should therefore be **graded by stakes**: a low-stakes edge is
accepted lightly; a high-stakes edge requires the full generative ratification and
provenance trail (§5.2). The retrieval layer can *suggest* the grade (from
centrality, recurrence, backlink density); the person ratifies it. Friction-depth
is itself a ratified, answerable property. Mis-grading is the real failure mode:
treating load-bearing knowledge as a black box produces brittleness (Bainbridge,
1983); demanding effortful engagement with what could be black-boxed produces
fatigue.

## 6. Render — the read surface (body edges are the source of truth)

**Reconciliation (dogfood 2026-06-11).** SPEC-045 settled the surface fork
empirically: typed edges live in the **body** as `predicate::[[Target]]`, and the
edge graph + SPL projection read *only* the body. Frontmatter `relations:` is
**invisible** to `zetl edges` — two frontmatter-only relations vanished from the
graph entirely in testing. The render therefore reads the **body edge graph**, not
a frontmatter schema; frontmatter carries scalars only (`title`, `tags`, status).

What the engine already exposes ([[SPEC-045]]):

- each typed edge renders as `.zetl-edge-predicate[data-predicate="…"]` — the
  predicate **auto-humanises** (`developed_in` → "Developed in") and carries a
  **per-predicate CSS hook**, so distinguishing relationship-types *visually*
  (colour, icon, weight) is a **theme task, not a typing task**;
- `page.edges_by_predicate` / `page.backlinks_by_predicate` / `vault.predicates`
  template vars expose the grouped view; the backlink panel already groups by
  predicate; `/_graph` colours, labels, and filters typed edges with a per-predicate
  legend;
- concept→concept objects link to `/concepts/<slug>/`; non-existent targets render
  as new-page stubs — the **accretion affordance**.

To be designed: a collapsible / right-rail **Connections panel** that collects a
page's *body* edges grouped by family (heavy inline today); **per-family colour**
(the `data-predicate` hook plus the `.zetl/predicates.toml` `category` field already
make this a theme-CSS change, not an engine change).

## 7. Implementation tiers

- **Tier 0 — manual (complete).** Authoring by hand; worked example nodes
  (`relational-posture`, `autopoiesis`) plus ratification passes validated the node
  shape, the surfacing (signal versus noise), and the loop.
- **Tier 1 — ratify-queue plus write-back.** Surface pending candidates (sidecar —
  §5.1) as a ratify-queue with predicate suggestions; a backend endpoint writes the
  confirmed body edge. Removes raw-markdown edit friction. Buildable in the tool
  alone. **Design detailed in §7.1–§7.4.**
- **Tier 2 — background generation.** Asynchronous processes (an **MCP / agent
  surface**, §7.4) pre-compute candidate edges and suggested predicates across the
  corpus — including the **generative-stub loop** (draft a node for a high-frequency
  ghost, with content + typed edges + a `machine-drafted` flag requiring human
  review) and **reference-checking** (web-search / source-verify for the academic and
  scientific literature concepts draw on). The renderer displays the pending set per
  file; the UI only renders. The model proposes; the human ratifies.

### 7.1 Tier-1 design — ratify-not-fill is the accessibility mechanism

The `predicate::[[Target]]` syntax is the **storage substrate**, comfortably
authored only by the small minority fluent in Markdown + a formal grammar — and
brittle even for them: a single space (`developed_in:: [[X]]`), inserted by hand or
by a formatter on save, **silently demotes every typed edge to untyped, with no
warning** (dogfood 2026-06-11). Manual typing is therefore not the authoring model;
it is the storage format.

The authoring model is **ratify-not-fill**, and this is the **accessibility
mechanism**, not merely a convenience. A person **recognises and confirms** a
proposed relation rather than **composing** its syntax. Recognition is available to
nearly anyone; syntactic composition requires a fluency a small fraction of the
population has — so the same act that removes friction for engineers is what opens
the tool to non-engineers *at all*. The design serves the §1.2 job stories whose
actors are not Markdown-native. Two surfaces over one substrate:

- **Syntax surface** (Markdown `::`) — for prose-native authors. Unchanged; the
  source of truth.
- **GUI surface** (the rendered web view) — for everyone else. Draw a link, or
  accept a surfaced candidate; the tool proposes a **ranked predicate list**
  (existing-vocabulary matches by fit + one inferred from the two endpoints — §4,
  *the predicate must be suggested*); pick one; the `::predicate` is **written for
  you**. The non-engineer never types `::` — and per SPEC-045's conservative
  recogniser, cannot produce a malformed predicate this way.

**Robustness lint (Tier-1, cheap).** Extend the `predicate-drift` lint family with a
**spaced-near-miss** check: a bare `[[X]]` immediately preceded by `word:: ` is
almost certainly a typed edge a formatter broke — flag it (advisory) rather than let
it demote silently. The same class of fix as the schema-rebuild bug: detect the
near-miss, don't fail silently.

### 7.2 Predicate families — the suggestion + grouping substrate

The emergent vocabulary (13 predicates from the first two nodes) already clusters
into **five functional families**:

| Family | Predicates (observed) | The relation it carries |
| --- | --- | --- |
| **Lineage / derivation** | `rooted_in` · `developed_in` · `draws_on` · `originates` | where it comes from |
| **Mapping / correspondence** | `maps_to` · `instance_of` · `has_dimension` · `measured_by` | how it relates conceptually |
| **Application** | `applied_in` · `operationalised_in` | where it does work |
| **Tension / revision** | `contrasted_with` · `reframed_by` | the dialectic |
| **Support** | `underpins` | what it grounds |

The families are **not** a controlled vocabulary (declaring one now, at 13
predicates from 2 nodes, would freeze the folksonomy and kill the viscosity gap —
§3). They are the *latent structure*, and they do three jobs at once: the render
**grouping/colour** (via the `category` field), the suggestion engine's **ranking
buckets**, and the drift detector's **categories**. Crystallisation stays
**collision-driven**: when two predicates compete for the *same* relation — e.g.
`underpins` vs `maps_to`, both observed pointing at `coherence-entrainment` — that
collision is the corpus asking for a canonical choice. Crystallise *that relation*,
not the whole vocabulary. Seeds the suggestion engine may rank against without
imposing: SKOS (`broader`/`narrower`/`related`), Dublin Core relations, the
Christopher Allen predicate set (already [[SPEC-045]]'s source).

### 7.3 Local, user-specific settings vs the shared corpus

Presentation and authoring preference are **personal**; the corpus is **shared**.
Conflating them entrains every reader to one person's view. Two layers:

- **Corpus layer — `.zetl/predicates.toml` (committed, shared).** Team governance
  and presentation metadata: `category` (the family), `display` label, `maps_to`
  IRI, sanctioned-or-not. One per corpus.
- **User layer — per-person, NOT committed (gap to name).** Personal colour scheme,
  preferred / pinned predicates, the **authoring-surface choice** (syntax vs GUI),
  suggestion-ranking personalisation. This must **not** live in the committed
  `.zetl/` (which is shared); it needs a per-person location — a gitignored
  `.zetl/local/` overlay, or an external `~/.config/zetl/`. Open question (§8):
  which, and how the layers compose (user overrides corpus *presentation*, never
  corpus *semantics*).

### 7.4 Core zetl vs plugin vs MCP — the architecture boundary

The substrate and the ratification loop are **core and model-free**; generation is
an **MCP that proposes into them**; presentation is a **swappable plugin**.

- **Core zetl** (universal, LangSec-strict, portable): the `::` grammar + AST; the
  typed edge graph, `zetl edges`, SPL projection; the **sidecar candidate /
  ratification data model** (`.zetl/decisions/`, §5.1); `zetl check` lints (incl.
  the §7.1 near-miss); the render hooks (`data-predicate`, `edges_by_predicate`). The
  engine and the data model must be identical everywhere or the graph is not
  portable.
- **Theme / plugin** (presentation, swappable): per-family colour, the Connections
  panel, display labels, `/_graph` styling. Reads the core hooks; never touches the
  substrate.
- **MCP / agent surface** (generative, opt-in): candidate *generation* —
  stub-drafting, predicate *inference*, and reference-checking / source-sourcing via
  web search. The model does work *here*, proposing into the sidecar candidate store;
  the human ratifies through the GUI.

**The boundary principle:** a vault with **no MCP and the default theme is fully
usable** — hand-authored or GUI-ratified. The agent layer only accelerates candidate
*supply*; it is never required to author, ratify, or read the graph. Generation
augments; it is not a dependency. (Same demotion-validation posture as the
criticality substrate: the model may draft freely because the human ratification
gate is the validation, and the `machine-drafted` flag keeps the demotion visible.)

## 8. Open questions

- **User-layer location and composition (§7.3).** Where do per-person, non-committed
  settings live — a gitignored `.zetl/local/` overlay, or an external
  `~/.config/zetl/`? And how do the layers compose: the user layer must override
  corpus *presentation* (colour, display label, preferred predicates, authoring
  surface) but never corpus *semantics* (the predicate name, the target, the SPL
  projection). The split is the safeguard against one person's view entraining the
  shared graph.
- **One pattern, three triggers.** Vocabulary promotion (§3), concept maturation,
  and an edge acquiring defeasible/provenance structure are the same pattern —
  flat by default, structure earned per item, ratified in context — but with
  different triggers: vocabulary by *frequency*, edges by *contestation*, concepts
  by a *coherence judgement*. This implies one ratification workflow but **not** one
  detection mechanism. Most triples can remain flat, simple facts; an edge earns
  reification (ratified-by/when, evidence, defeated-by) only when it becomes
  contested. Defeasibility therefore need not be encoded now, only **not
  foreclosed**.
- **Settled is a resting state, not a terminus.** Knowledge claims move; an
  `applied-in` edge can change when a project changes or new evidence appears.
  Implication: the data model must keep ratified edges **re-openable by default**;
  nothing should make a ratified edge final.
- **Use is the detector.** The triggers above are not artefacts to pre-build —
  relations are set as material is read in context; contestation arises when a
  concept meets wider scrutiny; a resting state shifts as the corpus is used.
  Pre-building such detectors risks modelling the dynamics instead of letting them
  emerge. The priority is getting the tool into use so the corpus evolves through
  use.
- **Suggestion mechanism without an LLM.** The tool has no language model;
  rich suggestion is agent work (Tier 2). What can a pure-tool Tier 1 suggest?
  (Ranking existing vocabulary by co-occurrence and backlink overlap may suffice
  for ratification.)
- **Object identity.** When does a target acquire a stable identifier versus a slug
  that can be renamed? (Rename-safety for the triple store. See §9.1 for a naming
  convention that helps.)
- **Vocabulary governance.** Promotion thresholds, and who governs a corpus's
  controlled vocabulary in an organisation.
- **Export and query.** RDF/JSON-LD export shape; whether to expose a triples /
  SPARQL-style query endpoint.
- **Relationship to SPEC-037** (3D space graph) — the triple store *is* that graph.
- **Generalisation.** The first test corpus is research-oriented; the capability
  must be tested on a non-research corpus before the general-product claim holds.
- **Onboarding as re-ratification.** Institutional knowledge must survive staff
  turnover: the people who absorbed it leave. Stored-but-not-re-learnable knowledge
  decays into inert artefact (a self-referential record; cf. Baudrillard, 1981, on
  signs that no longer refer). The design implication: an arriving person
  *re-derives* load-bearing edges rather than receiving them, using the prior
  provenance (§5.2) as scaffold; an edge re-opened by a new reader is the
  answerable cut functioning. Design unspecified.
- **Who governs the grading.** Deciding what must be understood versus what may be
  black-boxed (§5.3) is value-laden, and is the decision most easily delegated to
  the AI. If the automated layer sets what people must learn, the acceleration sets
  its own limit; accountability for the grading cannot be abstracted into the agent
  layer. Open: where the grading function lives, and who is answerable for
  *aggregate* errors (no single cut looks wrong, but the distribution erodes
  collective capacity to respond).
- **Screen as medium.** The tool runs on a screen, a medium suited to connection
  more than conjunction. Generating, discriminating, and explaining are
  sensorimotor acts, but thin ones. A possible consequence: the tool should be
  *deliberately partial* — declining to be the whole learning loop and, at points,
  directing the person back to richer, situated engagement rather than substituting
  for it.

## 9. Convergence with Allen's *Wikilinks and Named Edges*

Christopher Allen's agent-reference guide *Wikilinks and Named Edges* (Allen,
2026) is an independent, mature design for typed knowledge graphs in plain
markdown, from the self-sovereign-identity and decentralised-systems tradition.
Its **named edges** (`predicate::[[Target]]`) are equivalent to this spec's SPO
triples. The convergence is strong corroboration that the SPO direction is not
idiosyncratic to one corpus — it is a problem multiple practitioners are solving
similarly, and it sits within the folksonomy–taxonomy lineage (§3). The guide's
own principle for divergent vocabularies — *translate, do not standardise* — is
adopted here as the stance for engaging it.

### 9.1 Adopt (concrete conventions this spec lacked)

- **Multi-word predicate discipline** — underscored, reading as a sentence
  fragment, no single-word predicates (which collide, drift, and cannot be
  disambiguated). This spec had no naming rule; adopt directly.
- **Multi-word wikilink naming** — 2–7 words, specific enough that independent
  authors would generate the same title. This addresses the **object-identity** open
  question (§8): human-replicable canonical titles as the rename-stable identifier.
- **The catch-all-predicate trap** — before using a generic `relates_to`, name the
  *kind* of relationship. Operationalises ratify-not-fill at the vocabulary layer
  (§4).
- **Annotated predicates / progressive disclosure** — an indented rationale beneath
  an edge, with the three-level structure (predicate = direction, annotation =
  rationale, target = depth). This matches the answerability layer (§5.2);
  "progressive disclosure" is adopted as the name for available-not-obligatory
  provenance. (See §9.3 for a divergence in purpose.)
- **Construction predicates with an upgrade path** — predicates describing *how a
  node was built* (e.g. `extracted_from`), to be matured into semantic predicates
  and never downgraded to the generic catch-all; stale construction edges are noise
  to be pruned. This is the concrete mechanism for the flat-by-default →
  earned-structure trajectory (§3.1) and supplies the pruning the retrieval layer
  lacks.
- **Unconflation** — split a predicate that answers several distinct questions into
  separate predicates (Allen's example splits a single status predicate into
  maturity / confidence / curation-state / visibility / lifecycle-state). A useful
  precision discipline; adopt.
- **Vocabulary curation as gardening** — five activities (awareness, review,
  consolidation, clarification, enforcement) framed as weeding, seeding, and
  fertilising. Operationalises the maintenance discipline §3 calls for.
- **Ghost links and an external-reference marker** — links to non-existent targets
  signal where the graph should grow (the accretion affordance), and a marker
  distinguishes a cross-boundary reference from a broken link (useful for
  multi-repository corpora).

### 9.2 Translate (same intent, different form — keep this spec's form, map across)

- **Body-inline predicates versus frontmatter `relations:`.** Allen's reasoning —
  relationships in the body are visible to readers, whereas frontmatter is hidden
  from the narrative — corresponds to the §5 principle of co-locating decisions with
  comprehension. The Connections render (§6) restores visibility from frontmatter,
  but his stronger point (authoring the edge at the point in the prose where the
  relationship is made) is not satisfied by a detached block. **Open Tier-1 fork:**
  author inline and derive/mirror to frontmatter for machine use and rendering,
  versus frontmatter-canonical with prominent rendering.
- **`conforms_to` over `is_a`.** Conformance is revisable and pluralisable (multiple
  conformances coexist; two `is_a` declarations read as contradiction), which
  matches the "settled is a resting state, not a terminus" position (§8). Adopt
  `conforms_to` as the concrete form of defeasible classification.
- **Allen's predicate inventory versus "vocabulary is data, not code."** Do **not**
  canonicalise his ~40-predicate list — his own principle forbids importing another
  system's vocabulary wholesale. Use it as a **seed-bank for the suggestion engine**
  (§4): excellent material to *suggest*, ratified per corpus.
- **Reading order versus surfacing.** His agent reading-order (classification
  predicates first, then semantic, then body) is *traversal of an existing graph*;
  this spec's surfacing (§5.1) is *candidate discovery*. They compose: surfacing for
  recall, reading-order for the ratified graph.

### 9.3 Decline (keep this spec's position)

- **Annotations as agent reading-budget.** Allen frames annotations as serving an
  agent's traversal efficiency; this spec frames the same field as serving human
  understanding and answerability (§1.1, §5.2). The syntax is identical but the
  purpose differs — adopting the efficiency framing would shift the tool from a
  learning system toward a faster store. Keep the note as the answerability layer.
- **Rejecting undeclared predicates at creation.** Allen's curation includes
  rejecting undeclared predicates at creation time. That is the blank-page rigidity
  §4 refuses; this spec's mechanism is suggest-and-ratify, not reject.
- **What the guide does not cover** (so: no conflict, but this spec's additional
  contribution) — the learning-system framing (§1.1), provenance-as-governance and
  answerability to those affected (§5.2), friction graded by stakes (§5.3), and the
  **two-timescale selection model**: a fast retrieval/relevance layer over a slow,
  ratified-structure layer, kept at different update rates so the fast layer does
  not form a tight feedback loop with the slow one.

### 9.4 Out of scope (harvest later)

Compound nodes, **renditions** (markdown copies of external sources, marked with a
provenance predicate — equivalent to this spec's `documented-in`), archives, and
sidecar files for binaries are sound structural patterns but orthogonal to the SPO
and ratification core. Defer.

---

## References

- Allen, C. (2026). *Wikilinks and Named Edges — Agent Reference Guide.*
  https://gist.github.com/ChristopherA/151aefa6a6bde1ce4fa6b1182656cebe
- Ashby, W. R. (1956). *An Introduction to Cybernetics.* (Law of requisite variety.)
- Bainbridge, L. (1983). Ironies of automation. *Automatica*, 19(6), 775–779.
- Baudrillard, J. (1981). *Simulacra and Simulation.*
- Berardi, F. (2015). *And: Phenomenology of the End.* (Connection versus conjunction; semiocapitalism.)
- Bjork, R. A. (1994). Memory and metamemory considerations in the training of human beings. (Desirable difficulties.)
- Christensen, C. M., Hall, T., Dillon, K., & Duncan, D. S. (2016). *Competing Against Luck.* (Jobs to be Done.)
- Goodhart, C. A. E. (1975). Problems of monetary management. (Goodhart's law.)
- Klement, A. (2013). *Replacing the User Story with the Job Story.* (Job-story format.)
- Shirky, C. (2005). *Ontology is Overrated: Categories, Links, and Tags.*
- Slamecka, N. J., & Graf, P. (1978). The generation effect. *Journal of Experimental Psychology: Human Learning and Memory*, 4(6), 592–604.
- Vander Wal, T. (2007). *Folksonomy.* (Coinage and definition.)
- Vervaeke, J., Lillicrap, T. P., & Richards, B. A. (2012). Relevance realization and the emerging framework in cognitive science. *Journal of Logic and Computation.*
