---
title: "SPEC-044: Concept Graph — SPO Relations, Emergent-Crystallising Vocabulary, and In-Situ Ratification"
version: 0.1.0
status: strawman
date: 2026-06-08
audience: agent, human
parent: null
related:
  - SPEC-026  # Vault scan exclusions (the .zetlignore layer the corpus view rides on)
  - SPEC-043  # --no-gitignore + first-class .zetlignore (the corpus view this graph lives in)
  - SPEC-012  # Named themes (the render layer the graph surfaces through)
  - SPEC-005  # Defeasible reasoning / SPL (Hence — consumes the triples)
---

# SPEC-044: Concept Graph — SPO Relations, Emergent Vocabulary, In-Situ Ratification

> **Strawman.** Captured live during the 2026-06-08 think-with that built the
> first concept node (`concepts/relational-posture.md`) by hand and ran one
> manual ratification pass. This documents the *direction* and the validated
> loop; it is **not yet a designed spec**. Numbers, schemas, and endpoints are
> provisional. Uncommitted at capture.
>
> **Extended** by a follow-on 2026-06-08 think-with: the learning-system frame
> (§1.1), the Goodhart floor + do-not-build constraints (§3.1), and answerability
> / provenance of the cut (§5.2–5.3).

## Information Table

| Field        | Value                                                                 |
| ------------ | --------------------------------------------------------------------- |
| Document ID  | SPEC-044                                                               |
| Title        | Concept Graph — SPO Relations, Emergent Vocabulary, In-Situ Ratification |
| Version      | 0.1.0 (strawman)                                                       |
| Status       | Strawman                                                               |
| Author       | Kairos (m3-kairos dyad) + Mat Mytka                                    |
| Date         | 2026-06-08                                                             |

---

## 1. The problem

A markdown corpus authored as *documents* (notes, traces, papers) is fractured:
the concepts that recur across it live *implicitly distributed in prose*, not as
nodes with edges. Full-text search finds strings; semantic search finds nearby
meaning; neither gives a **navigable structure** — "how do my concepts
interrelate, where is each born vs merely used, what is its lineage." The corpus
is not a graph because it was never authored as one.

Two readers want this graph, in different moods (per the *co-inhabited corpus
interface*): the **human** wandering it associatively (I-Thou), and the
**machine** querying it as structured facts (I-It). The same substrate must
serve both without flattening the first into the second.

**This is an Anuna product, not just dyad infrastructure.** The capability —
*turn a markdown corpus into a typed, navigable, machine-reasonable knowledge
graph* — generalises to organisational knowledge management and human-AI
collaboration. So the design must be transferable, not idiosyncratic to one
dyad's research vocabulary.

### 1.1 The deeper frame: a learning system, not a knowledge store

Knowledge management in the epoch of AI has to be a **learning system — for
humans and machines both.** The predicament forces it: entangled human-AI
epistemics now produce knowledge artefacts faster than the human social body can
metabolise (Berardi's *connection* outrunning *conjunction*). A tool that only
stores and retrieves *faster* is an accelerant — it deepens the fatigue it claims
to cure. The wager here is to build a **conjunctive** tool inside that connective
acceleration: one whose core act keeps knowledge coupled to a body that
understands it (knowledge ↔ being-ness) and to its referent (map ↔ territory).

Concretely: the in-situ ratification act (§5) is not data entry that happens to
teach — it **is** a learning act, and the edge is its *byproduct* (the generation
effect / desirable difficulty turned into graph construction). This is how the
tool re-couples the human to the re-encoding that AI otherwise does *for* them —
the comprehension-debt / Bainbridge fix built into the interface. Sense and zetl
are not a KM tool that also teaches; they are learning systems that happen to
retain.

## 2. The model: Subject-Predicate-Object

Relations are **SPO triples** — the universal, transferable, machine-reasonable
substrate (a six-month recurring attractor in the dyad's thinking; chosen
deliberately).

- **Subject** — the page itself (implicit; the file *is* the subject).
- **Predicate** — the relation type.
- **Object** — the target (another concept node, or a document).
- **Qualifier** — an optional human-readable `note`. This is the I-Thou escape
  valve that pure triples lack; it rides on the I-It substrate.

Authored as **lightweight frontmatter** (validated shape, from the pilot):

```yaml
relations:                 # concept → concept
  - type: measured-by
    target: semantic-coupling
    note: "posture is the disposition; coupling is the measured dynamic"
developed-in:              # concept → document (primary)
  - Action-Research/research/relational-posture-framework-sketch.md
documented-in:             # concept → document (typed, operative-elsewhere)
  - type: operationalised-in
    path: publications/papers/cross-substrate-coupling-preprint.md
    note: "..."
```

**Not raw RDF.** The semantic web died on "agree on formal ontologies with URIs
first." Author the friendly frontmatter; treat it as SPO *semantically*; offer
**RDF / JSON-LD export** for interop when an org needs it. Derivation is trivial:
for each page, for each relation, emit `(page, predicate, object, note)`.

**Feeds Hence.** SPO triples are exactly what a defeasible reasoner (SPL/Hence,
SPEC-005) consumes. The "machine-reasoning graph" and the SPO reframe are the
same thing from two directions.

## 3. Vocabulary: emergent, crystallising — flexibility is home

The **structure** (SPO) is universal and ships in zetl. The **predicate
vocabulary** is domain-specific and is **data, not code** — each corpus declares
its own (research: `measured-by`/`developed-in`/`operationalised-in`/`applied-in`;
an org: `decided-in`/`supersedes`/`owned-by`/`depends-on`/`contradicts`).

Predicates are **emergent by default, crystallising into controlled as they
prove out** — *not* a static rigid/flexible toggle but a **trajectory**, the
same `marinating → settled` arc the concepts themselves follow:

1. New predicates accrete freely as edges are drawn (flexibility = home base).
2. Recurring ones (`measured-by` used across N nodes) can be **promoted** into a
   declared/controlled vocabulary — and that promotion is *itself an in-situ
   ratified decision* ("you've used `measured-by` 14 times — ratify it into the
   vocabulary?").
3. An org dials toward controlled (governance, validation, reasoning guarantees)
   only as far, and as fast, as the corpus can metabolise.

Premature rigidity is the known failure mode on every axis: technically (the
semantic web), epistemically (*performed-settled* vs settled), and ecologically
(over-codification past metabolic carrying-capacity = dead matter). Hence:
**flexibility is the default; rigidity is earned, incrementally.**

> **Configurable formalisation is the product feature** — but framed as
> *flexibility you can tighten*, not *rigidity you can loosen*. The tool mirrors
> its own subject: emergence crystallising into structure only as fast as it is
> metabolised.

### 3.1 Learning is the end; the graph is the byproduct (the Goodhart floor)

The conjunctive wager (§1.1) lives or dies on one polarity: **the graph is the
byproduct of learning, never the target.** The instant the activity is optimised
to *produce good edges*, it Goodharts — teaching to the edge-test — and
conjunction collapses back into connection (a faster knowledge store with a
kinder UI). Two **do-not-build** constraints follow:

- **No edge-count metric.** No completion bars, no "N relations created", no
  acceptance-rate to optimise. Metering edge-production turns the learning act
  into output and re-imports the acceleration the tool exists to counter
  (metering = entrainment).
- **Success signal is *re-derivability*, not volume.** The question a healthy
  graph answers is "does this edge still cohere when a new body re-opens it?" —
  not "how many edges do we have?"

## 4. Emergent ≠ blank-page: the predicate MUST be suggested

**The load-bearing UX constraint** (Mat, 2026-06-08): a human must never be
forced to invent a predicate from scratch. Emergence without suggestion is just
friction. When an edge is drawn, the machine **proposes predicates**:

- existing-vocabulary matches, ranked by fit, **plus**
- a **proposed-new** predicate inferred from the two endpoints' content.

The human **ratifies or overrides** — the predicate suggestion is itself a
relevance-realisation act (machine widens the candidate field; human realises
which fits). This is the same *ratify-not-fill* principle as §5, applied to the
vocabulary layer. Without it, "emergent vocabulary" is a blank text box and dies.

## 5. In-situ ratification (the UX spine)

The validated loop, from the manual pilot:

1. **Surface (machine, wide recall):** background-runner agents traverse the
   corpus and pre-compute **candidate edges + suggested predicates** (semantic
   net + term net + backlinks — all already in zetl/Sense).
2. **Present in situ (human, in context):** when a file is opened in the
   renderer, *its* pending candidate-decisions are already waiting, shown
   alongside it. **The decision is co-located with metabolising the meaning** —
   you cannot offload it to the machine because it is presented exactly when you
   are understanding the thing. This is the comprehension-debt fix as UX, and
   the anti-Bainbridge organ built into the interface.
3. **Ratify (one act = confirm + type):** the machine pre-proposes a verdict
   (predicate + evidence); the human accepts / overrides / rejects / **reroutes**
   ("belongs to a different concept node"). **Ratify-not-fill** — confirming a
   proposal, not filling a blank form. The confirming *is* the consolidation.
4. **Commit:** on confirm, the triple is written into the subject's frontmatter.

Background runners surface more than relation-edges: frontmatter generation,
orphan linking, concept extraction (new nodes to create), dead-link resolution,
predicate crystallisation (§3). All are the same shape — *candidate decisions
ratified in situ*.

### 5.1 Open fork: where do pending decisions live?

- **In-frontmatter** (what the pilot did by hand) — but background agents then
  write un-ratified machine guesses into every source file, racing each other.
- **Sidecar** (`.zetl/decisions/` queue the renderer *overlays*) — source files
  stay clean; a decision writes into the file **only on confirm**. Keeps "the
  confirming is what commits it" literally true. **Lean: sidecar.**

### 5.2 Answerability: provenance of the cut

A ratification is a **cut** — choosing this edge/predicate, rejecting or
rerouting the rest. Cuts are never neutral; the tool participates in what it
sorts. The governance answer is *not wiser cuts* (the worst sort-errors are
invisible from the place of cutting — you can't self-audit a sort from inside the
basin that made it) but **answerable cuts**: the commit writes the cut's
*provenance*, not just the triple — what the semantic/term net + backlinks
surfaced, what was chosen, what was rejected or rerouted and why, who ratified,
when.

This provenance is **available, not obligatory** — collapsed and dormant by
default, expandable by the one person at the one moment who needs it. Most readers
never open it; the point is that they *can*. That is explainable-AI reframed
correctly: never *force* everyone through the deliberation (that is the fatigue),
never *foreclose* it either. It is also what keeps a cut defeasible by its
*subjects* — including those not in the room when it was made.

**The `note` is the answerability layer.** Across the design the human-readable
qualifier kept turning out to carry more than decoration — defeasibility (the
revisability a flat triple can't hold), then provenance. It is not an afterthought
field; it is the I-Thou trace of *why* the I-It edge exists. First-class.

> **Already done by hand (Tier 0).** The `relational-posture` pass recorded its
> cut *in the file* — a frontmatter comment naming what was dropped and why
> (`ANALYSIS_H2H → belongs to [[semantic-coupling]]`; `RESEARCH_MAPPING → too
> generic, an RQ tag not a relation`), a `note:` on each kept edge, and the pass
> dated in the body. The capability is one formalisation step from the manual
> pilot.

### 5.3 Grade the friction by the sort

Not every edge earns the same depth. Most knowledge can safely cool into a black
box you stand on without re-deriving (you needn't learn fibre-optics to use the
internet); some is load-bearing and must stay *metabolisable*. So friction-depth
is **graded by stakes**: a low-stakes edge is accepted lightly; a load-bearing one
demands the full generative ratification + provenance trail (§5.2). Sense can
*suggest* the grade (centrality, recurrence, backlink density); the human ratifies
it. **Friction-depth is itself an answerable, ratified property** — and
mis-grading is the real failure mode: black-boxing what needed metabolising →
Bainbridge brittleness; demanding metabolisation of what didn't → fatigue.

## 6. Render (partially built, 2026-06-08)

The `earthian` theme now renders a concept node's frontmatter as a visual
grammar (the render target everything downstream builds on):

- frontmatter `title` preferred over the filename slug;
- `tags` as quiet pills;
- a **Connections** block — typed edges in groups (*Relates to* / *Developed in*
  / *Operative in*), each predicate a coloured type-tag + linked object + note.
- concept→concept objects link to `/concepts/<slug>/`; non-existent targets
  render as zetl new-page stubs — the **accretion affordance** (click = "define
  this next").

TBD: collapsible / right-rail placement (the block is heavy at the top of a
file); render must stay generic (it reads arbitrary frontmatter, not a fixed
schema) so it generalises across corpuses.

## 7. Tiers

- **Tier 0 — manual (done).** The dyad acts as the editor; the
  `relational-posture` node + one ratification pass proved the node shape, the
  surfacing (signal vs noise), and the loop-as-consolidation.
- **Tier 1 — zetl ratify-queue + write-back.** Render `candidate-connections`
  (or the sidecar) as a ratify-queue with predicate suggestions; a backend
  endpoint patches frontmatter surgically (promote candidate → typed triple).
  Removes the raw-markdown edit friction. Buildable in zetl alone.
- **Tier 2 — background-runner generation.** Async agents pre-compute candidates
  + suggested predicates across the corpus; the renderer displays the pending
  set per file. *This is what dissolves the "UI invokes an agent live" hard
  problem — the agent pre-populates; the UI only renders.*

## 8. Open questions / residue

- **Hardening is the third instance of the one move** (think-with, 2026-06-08).
  Vocabulary crystallising (§3), concepts settling, and an edge gaining
  defeasible/provenance structure are *the same move* — flat-by-default, structure
  earned per-edge, ratified in situ — with **different triggers**: vocabulary on
  *frequency*, edges on *contestation*, concepts on a *coherence/human call*. One
  ratification UX (build the loop once); **not** one sensor. Most triples stay flat
  monotonic facts forever; an edge earns reification (ratified-by/when, evidence,
  defeated-by) only when it becomes contested — so defeasibility need not be
  encoded now, only *not foreclosed*.
- **Settling is a resting point, not a terminus.** Movement is the ground state;
  "settled" is just low velocity. An `applied-in X` edge can move when the project
  moves or new evidence redirects. Implication: the data model must keep ratified
  edges **re-openable by default** — never build anything that makes a ratified
  edge feel final. Finality is the one thing this says is false.
- **Use is the detector, not a computed sensor.** The three triggers above are not
  design artifacts to pre-build — relations get set when knowledge is metabolised
  in situ; contestation arrives when a concept touches larger social epistemics; a
  resting point moves when the corpus is lived in. Pre-building the sensors would
  be *modelling the thing instead of letting it move*. The priority is getting zetl
  into use so the corpus evolves by being used; the dynamics emerge, not encoded.
- Predicate **suggestion** mechanism: zetl has no LLM — suggestion is agent work
  (Tier 2). What can a *pure-zetl* Tier 1 suggest? (existing-vocab ranking by
  co-occurrence / backlink overlap, without an LLM, may be enough for ratify.)
- Object identity: when does a `target`/`path` get a stable id vs a slug that can
  rename? (rename-safety for the triple store.)
- Crystallisation thresholds + who governs the controlled vocabulary in an org.
- RDF/JSON-LD export shape + whether to expose a SPARQL-lite / triples endpoint.
- Relationship to SPEC-037 (3D space graph) — the triple store *is* the graph.
- Provenance: the dyad's own corpus is the first dogfood; the relational-posture
  node is the worked example. Generalisation must be tested on a non-research
  corpus before the product claim holds.
- **Onboarding = re-ratification (the org product).** Institutional memory must
  survive body-churn: the metabolising knowers leave. Storage without
  re-learnability rots into dead artefact (the self-referential / simulacral
  corpus). The fold: the arriving body *re-derives* load-bearing edges rather than
  receiving them — the departed body's provenance (§5.2) as scaffold — and a cut
  re-opened by a new subject *is* the answerable cut working. Memory that stays
  re-learnable, not merely stored. The learning-KM system is the membrane between
  machine-speed production and the human social body. (Design unspecified.)
- **Who holds the sort?** Deciding what-must-be-metabolised vs
  what-can-be-black-boxed (§5.3) is power- and value-laden, and is the call most
  temptingly handed to the AI. If the connective layer sets what the human body
  must learn, the accelerant sets its own speed limit — accountability for the
  sort can't be abstracted into the agent layer (cf. cbcl-bus criticality floor).
  Open: where the sort-function lives, and who is answerable for *silent aggregate*
  errors (no single cut looks wrong; the distribution hollows collective capacity
  to respond — the irreversibility clause).
- **Can conjunction happen through glass?** The tool lives on a screen — the
  connective surface par excellence. Generating / discriminating / explaining are
  sensorimotor acts, but thin ones. Possible design consequence: the tool should
  be *deliberately partial* — refuse to be the whole learning loop, and at some
  point push the human back out to the embodied milieu where thick learning lives.
  The map that keeps pointing at the territory rather than replacing it.

## 9. Convergence: Christopher Allen's *Wikilinks and Named Edges* (external corroboration)

2026-06-09, Hugo surfaced Christopher Allen's agent-reference guide
([gist](https://gist.github.com/ChristopherA/151aefa6a6bde1ce4fa6b1182656cebe)) —
an independent, mature design for typed knowledge graphs in plain markdown, from
the self-sovereign-identity / decentralised-systems lineage. His **named edges**
`predicate::[[Target]]` *are* this spec's SPO triples. This is strong external
corroboration: the SPO direction is not the dyad's idiosyncrasy (six-month
internal recurrence-without-closure) — multiple people are grappling with the
same problem. The discipline in engaging it is **coherence, not entrainment**:
metabolise it, don't swallow it whole. (Fittingly, his own guidance for divergent
vocabularies is *translate, don't standardise* — so the right way to engage his
system is the way it tells you to engage difference.)

### 9.1 Adopt (concrete conventions we lacked; no conflict with the frame)

- **Multi-word predicate discipline** — underscores, reads as a sentence fragment,
  no single-word predicates (collision / drift / no-disambiguation-path). We had
  no naming rule; take this directly.
- **Multi-word wikilink naming** — 2–7 words, "specific enough that independent
  authors would generate the same title." This answers our §8 **object-identity**
  open question: human-replicable canonical titles as the rename-stable id.
- **The `relates_to::` trap** — "before using the catch-all, name the *kind* of
  relationship." Operationalises ratify-not-fill at the vocabulary layer (§4), with
  an audit command.
- **Annotated predicates / progressive disclosure** — indented rationale beneath an
  edge; "predicate gives direction, annotation gives rationale, target gives
  depth." This *is* our `note` / **answerability layer** (§5.2) and "progressive
  disclosure" is the better name for **available-not-obligatory** provenance. Adopt
  the term and the three-level structure. (Telos caveat: see §9.3.)
- **Construction predicates + upgrade path** — `extracted_from::` etc. mark *how a
  node was built*, to be matured into semantic predicates; "never downgrade to
  `relates_to::`"; "stale construction edges accumulate as noise." This is the
  concrete mechanism for our flat-by-default → hardened-when-earned (§3.1) **and**
  the pruning organ Sense lacks.
- **Unconflation** — a predicate answering multiple questions should split (his
  `has_status::` → `has_maturity` / `has_confidence` / `has_curation_state` /
  `has_visibility` / `has_lifecycle_state`). New discipline; keeps predicates
  precise. Adopt.
- **Gardening = five curation activities** (awareness → review → consolidation →
  clarification → enforcement; weed / seed / fertilise). Operationalises our
  use-is-the-detector / ecological-not-editorial hand-wave.
- **Ghost links + `↗` external marker** — non-existent targets "signal where the
  graph wants to grow" (= our accretion affordance), and `↗` marks an edge that
  crosses a repo boundary (useful for the co-inhabited / multi-repo corpus).

### 9.2 Translate (same intent, our form differs — keep ours, map to his)

- **Body-inline predicates ↔ our frontmatter `relations:`.** His reasoning —
  "graph participation requires visibility; YAML is hidden from narrative flow" —
  is our own *decision co-located with metabolising meaning* (§5). The earthian
  Connections block already gives visibility from frontmatter, but his deeper point
  (the edge authored *at the point in the prose where the relationship is made*) is
  not satisfied by a detached block. **Open Tier-1 fork:** author inline in body
  *and* derive/mirror to frontmatter for machine/render, vs frontmatter-canonical +
  prominent render.
- **`conforms_to::` over `is_a::` ↔ our defeasibility.** Conformance is *revisable*
  and *pluralisable* (multiple conformances coexist; two `is_a::` read as
  contradiction) — exactly "settling is a resting point, not a terminus." Adopt
  `conforms_to::` as the concrete form of defeasible classification.
- **His ~40-predicate inventory ↔ our "vocabulary = data, not code."** Do **not**
  canonicalise his list (his own principle forbids importing another system's
  vocabulary). Use it as the **seed-bank for the predicate-suggestion engine** (§4)
  — excellent raw material to *suggest*, ratified per-corpus.
- **Agent progressive-disclosure reading order ↔ Sense surfacing.** His protocol
  (classification predicates first → semantic → body last) is *traversal of the
  ratified graph*; Sense is *candidate surfacing*. They compose: Sense for recall,
  his reading-order for the cut graph.

### 9.3 Decline (keep ours — this is the conjunctive/connective fork)

- **Annotations-as-agent-reading-budget framing.** His annotations serve agent
  traversal *efficiency* (connective); ours serve *human metabolising* /
  answerability (conjunctive, §1.1). Same syntax, different soul — adopting his
  telos would silently slide the tool from conjunctive to connective (the exact
  entrainment risk). Keep the note as the answerability layer, not a
  budget-optimiser.
- **"Reject undeclared predicates at creation time"** (his curation activity #5).
  Hard rejection = the blank-page rigidity we explicitly refuse (§4: emergent ≠
  blank-page). Ours is **suggest-and-ratify**, never reject.
- **What he lacks entirely** (so: nothing to adopt; this is our contribution back
  in any translation) — the learning-system / conjunctive frame (§1.1),
  provenance-of-the-cut as *governance / answerability-to-subjects* (§5.2),
  friction-graded-by-the-sort (§5.3), and the Sense weather/geology
  **two-viscosity selection** (corpus-boundary vs relevance/sort). His system is
  author + agent centric; ours adds the human-coupling and the epistemics.

### 9.4 Out of scope (harvest later)

Compound nodes, **renditions** (`derived_from::` markdown copies of external
sources — our `documented-in`; the HCI overview is literally a rendition),
archives, and sidecar files are sound structural patterns but orthogonal to the
SPO / ratification core. Defer.
