# Wikilink Predicates — Typed Named Edges

zetl's link graph records **that** two pages are connected. The predicate
language lets you also record **how** — by typing a wikilink with a label:

```markdown
- derived_from::[[2026 Q1 Retro]]
- supersedes::[[Decision Log 2025]]
- contradicts::[[Move Fast Doctrine]]
```

A bare `[[wikilink]]` is just a named edge whose predicate is empty (an
*untyped* edge), so a vault that never writes a `::` behaves exactly as it did
before — typing is purely additive, with no migration and no flag day.

This is the engineering surface of the concept-graph idea (SPEC-045). It owns
the parse → graph → reasoner → CLI machine layer; the authoring UX and
vocabulary-governance philosophy are shared with SPEC-044.

## The grammar

A predicate is one or more lowercase ASCII letters, digits, and underscores,
terminated by `::`, immediately before a `[[`:

```
predicate::[[Target]]
```

* **List-item form** (the canonical shape): `- derived_from::[[Target]]`.
* **Line-leading inline form**: `derived_from::[[Target]]` at the start of a
  line's content.

> Mid-paragraph inline predicates (`see also derived_from::[[X]]` in the middle
> of a sentence) are **not** recognised in v1 — only the leading position is.

### Multiple predicates per link

Chain predicates on `::` to assert several relations to one target:

```markdown
- derived_from::informed_by::[[2026 Q1 Retro]]
```

The set is order-insensitive and de-duplicated (`a::a::[[X]]` collapses to one
edge). In the graph this expands to one directed edge **per predicate**.

### CURIE predicates (standard vocabularies)

A single internal colon introduces a namespaced standard term:

```markdown
- prov:wasDerivedFrom::[[Source]]
```

CURIE predicates are first-class in the graph, queries, template variables, and
RDF export. They are **not** projected to SPL facts (there is no collision-free
mapping onto the snake-functor space) — the snake form is the SPL surface, the
CURIE form the RDF surface.

### Edge annotations

A nested sub-bullet under a list-item named edge is captured as the edge's
**annotation** — the *why* of the edge, surfaced for progressive disclosure in
backlink panels, `zetl edges --annotated`, the graph view, and RDF export:

```markdown
- contradicts::[[Move Fast Doctrine]]
  - Specifically rejects the "skip the design doc" clause.
```

### What is *not* a predicate (conservative by design)

Malformed tokens are never normalised or guessed into a predicate. Each of
these resolves to plain text plus an ordinary untyped wikilink:

| You wrote | Result |
|---|---|
| `derived_from::[[X]]` | typed edge `derived_from` |
| `derived from::[[X]]` (space in name) | untyped edge |
| `derived_from:: [[X]]` (space after `::`) | untyped edge |
| `derived_from:::[[X]]` (triple colon) | untyped edge |
| `Derived_From::[[X]]` (uppercase) | untyped edge |
| `wasDerivedFrom::[[X]]` (bare camelCase) | untyped edge |
| `derived_from/has_derivative::[[X]]` (slash) | untyped edge |

So a typo never silently mints a phantom `derived_from` edge.

## Querying typed edges — `zetl edges`

`zetl edges` is the operator-friendly query surface — a strict superset of
`zetl links`:

```bash
zetl edges                                # every edge, typed and untyped
zetl edges --from "Decision Log"          # outgoing edges of a page
zetl edges --to "2026 Q1 Retro"           # incoming edges
zetl edges --predicate contradicts        # one predicate (repeatable for OR)
zetl edges --predicate derived_from --predicate supersedes
zetl edges --untyped                      # only bare links
zetl edges --annotated                    # only edges carrying an annotation
zetl edges --by-predicate                 # vocabulary-distribution histogram
```

`-f json` emits one row per edge as
`{source, target, predicate|null, annotation|null, line, is_ghost}`. The command
is read-only, opens no socket, and is deterministic.

## Vocabulary hygiene — `zetl check`

The vocabulary is **emergent by default**: the set of predicates is exactly the
set observed in the corpus, and `zetl check` runs advisory lints against that
usage (never an error unless you opt in — see below):

* **`predicate-drift`** — a low-frequency predicate within edit-distance 2 of a
  much-higher-frequency one (e.g. `infrmed_by` near `informed_by` — a likely
  typo).
* **`predicate-prefer-conforms-to`** — suggests `conforms_to::[[X Form
  Contract]]` wherever `is_a::` appears (a node *conforms to* a spec; it is not
  *identical* to one).
* **`predicate-relates-to-overuse`** — flags pages where `relates_to::` is the
  majority of typed edges (reach for a more specific predicate).
* **`predicate-undeclared-prefix`** — a CURIE whose prefix isn't declared in
  `[prefixes]` (still valid; RDF export falls back to the vault namespace).

## Earning a controlled vocabulary — `.zetl/predicates.toml`

Most vaults want to just type the edge, so there is **no file by default**. When
a team has stabilised a vocabulary, it can opt into a strict file that moves
along the *emergent → crystallising → controlled* trajectory. The file carries
**governance and presentation metadata only** — never semantic meaning (meaning
lives in the predicate name, the target page, and SPL rules).

```toml
# .zetl/predicates.toml — OPTIONAL. Not shipped; populated by crystallisation.

enforce     = true     # true  → undeclared predicate is an ERROR (controlled)
                       # false → warn + nearest-match    (crystallising)
                       # (file absent entirely           → emergent default)
accept_is_a = false    # true  → suppress the conforms_to suggestion

# CURIE prefix → namespace IRI (enables CURIE authoring + full-IRI RDF export)
[prefixes]
prov    = "http://www.w3.org/ns/prov#"
dcterms = "http://purl.org/dc/terms/"

# The declared (controlled) set. The value is governance/presentation metadata.
[predicates]
derived_from = { display = "Derived from", category = "provenance", maps_to = "prov:wasDerivedFrom" }
supersedes   = { display = "Supersedes",   category = "lifecycle",  maps_to = "dcterms:replaces" }
conforms_to  = { display = "Conforms to",  category = "classification", maps_to = "dcterms:conformsTo" }
contradicts  = { display = "Contradicts",  category = "structural" }
relates_to   = { display = "Relates to",   category = "structural", catch_all = true }
```

Under `enforce = true`, an undeclared predicate becomes an **error** that fails
`zetl check` and the build. The file expresses no `definition`, `inverse_of`,
`transitive`, `domain`, or `range` — those, if wanted, are SPL rules.

## Reasoning over typed edges (`--features reason`)

Build with `--features reason` and each typed **snake** edge projects to an SPL
fact `(<predicate> "<source>" "<target>")`, so the whole typed graph is
queryable by the defeasible engine. A rule like:

```spl
(normally r-superseded-stale
  (supersedes ?new ?old)
  (stale ?old))
```

fires over the projected `supersedes` facts, and `zetl reason explain` /
`zetl reason provenance` trace the conclusion back to the **asserting page and
line** — important because a predicate on page A asserts a fact *about* page B
(a trust boundary: treat any rule that acts on a projected fact accordingly).

Untyped edges and CURIE predicates are not projected. The default build (no
`reason` feature) pays nothing.

## In themes and the graph view

`zetl build` / `zetl serve` expose typed edges to templates:

* `page.edges` / `page.edges_by_predicate` — outgoing edges (untyped edges
  bucket under the reserved `__untyped` key).
* `page.backlinks` — extended with `predicate` / `label` / `annotation`; plus
  `page.backlinks_by_predicate` for the grouped panel.
* `vault.predicates` — the observed predicate set with counts.

In a rendered page body, a typed edge's predicate becomes a small **chip** in
front of the link (so `derived_from::[[Retro]]` reads *"⟨Derived from⟩ Retro"*,
not raw `derived_from::` text). The default theme renders the **backlink panel
grouped by predicate**, with the annotation surfaced inline. The interactive
graph view (`/_graph`) colours and labels edges by predicate, draws arrowheads,
shows a per-predicate legend with show/hide toggles, and reveals an edge's
annotation on hover. With no typed edges, every surface renders exactly as it
did pre-predicates.

### Styling the predicate chip (themes)

The render pipeline emits a stable, semantic markup contract that themes target
with CSS — the markup is generated by core, the *look* is entirely the theme's:

```html
<span class="zetl-edge-predicate" data-predicate="derived_from">Derived from</span>
```

* **`.zetl-edge-predicate`** — style the chip however you like (the default
  theme ships a muted, uniform pill; the loud per-predicate colour is reserved
  for the graph).
* **`[data-predicate="<name>"]`** — opt into per-predicate styling, e.g. echo
  the graph palette in the body:
  ```css
  .zetl-edge-predicate[data-predicate="contradicts"] { color: #b00; border-color: #b00; }
  .zetl-edge-predicate { text-transform: none; }   /* drop the small-caps */
  ```
* **No CSS at all** → the span degrades to readable plain text (the label),
  never a broken box — so a theme that does nothing still renders cleanly.

Authoring is unaffected by any of this: you always type
`- predicate::[[Target]]`; only the rendered HTML changes. (Themes that want to
restructure the markup itself — rather than restyle it — can do so with a
SPEC-032 transform hook.)

## Search

The search index gains a faceted `predicate` field capturing a page's outgoing
typed-edge predicates, so you can filter "pages that `contradicts` something" or
"pages with a `conforms_to` edge". Re-index on upgrade.

## Migrating `tags:` to predicates — `zetl predicates migrate`

Relationship-shaped frontmatter `tags:` (a tag that names another page) are
predicates in disguise. The read-only helper *reports* candidates without
touching any file:

```bash
zetl predicates migrate --dry-run            # report; never rewrites
zetl predicates migrate --dry-run --key tags --key topics
```

It prints the file, the tag, a candidate predicate, and the target page. You
edit the body yourself and choose the specific predicate.

## Semantic-web export — `zetl export --rdf`

Project the typed graph to RDF for any external triplestore (the `--format`
name is taken by the global `-f/--format` output selector, so the RDF flag is
`--rdf`):

```bash
zetl export --rdf turtle
zetl export --rdf ntriples
zetl export --rdf jsonld
zetl export --rdf turtle --base-iri "https://my-vault.example/"
```

Each typed edge becomes a triple; CURIE predicates and snake predicates with
`maps_to` expand to full IRIs via `[prefixes]`; annotations become
`rdfs:comment`; provenance (source page / file / line) becomes PROV-O on a
reified statement; and the predicate vocabulary is emitted as a SKOS concept
scheme. A SPARQL endpoint is out of scope — SPL is the in-tree query surface.

## See also

* [`docs/zetl-ast-reference.md`](zetl-ast-reference.md) — the AST node shape
  (`Wikilink.predicates`, schema v1.1).
* SPEC-045 (the design spec) for the full contract surface, threat model, and
  open questions.
