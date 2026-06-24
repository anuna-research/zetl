---
id: SPEC-048
title: "Template Components & Templated Static Pages"
status: draft
version: 0.2.1-strawman
last-updated: 2026-06-24
audience: agent, human
---

# SPEC-048: Template Components & Templated Static Pages

## Orientation

**Intent:** Let zetl render its *own* hand-authored static pages (landing, about,
404) with the **same** navigation, link base, and brand tokens its themed vault
pages already use — so a site needs no second generator and brand details cannot
drift between the two surfaces.

**Metaphor:** *One print shop, shared plates.* Components are reusable plates and
`tokens.css` is the single tin of ink; the press stamps both the bound book (themed
vault pages) and the loose posters (static override pages) from the same plates and
the same ink, so the two can never disagree on what the brand looks like.

**Structure** (v1 core — `≤ 7` boxes; arrows = data/flow direction):

```
          ┌──────────── Site Context (REQ-4801 / REQ-4802) ─────────────┐
          │  site.nav · tokens · root_path · build mode — ALWAYS present │
          └──────┬───────────────────────────────────────────┬─────────┘
                 │ (site+page tier)              (site tier)   │
        ┌────────▼─────────┐                      ┌────────────▼─────────┐
        │ Themed content   │                      │ Templated static page│
        │ page             │                      │ *.html.jinja (4811)  │
        └────────┬─────────┘                      └────────────┬─────────┘
                 │       {% component %} / native {% call %}    │
                 └───────────────────┬──────────────────────────┘
                                     ▼
                   ┌────────────────────────────────────┐
                   │ Component resolver → minijinja MACRO │
                   │ (REQ-4803 / REQ-4804 / REQ-4805)     │
                   └───────┬──────────────────────┬───────┘
                           ▼                      ▼
                 ┌──────────────────┐   ┌────────────────────────┐
                 │ Token compiler   │   │ Component CSS:          │
                 │ tokens.css(4812) │   │ dedup + deterministic   │
                 │                  │   │ emit, UNSCOPED (4809)   │
                 └──────────────────┘   └────────────────────────┘
```

**Decisions** (deliberate before implementing):
[[SPEC-048-components-and-static-overrides#ADR-4803]] components ARE minijinja macros,
not a new engine tag ·
[[SPEC-048-components-and-static-overrides#ADR-4804]] site/page tiering is the
load-bearing primitive ·
[[SPEC-048-components-and-static-overrides#ADR-4805]] static pages render via site
context, opt-in by `.html.jinja` suffix ·
[[SPEC-048-components-and-static-overrides#ADR-4809]] one generator, not "Jekyll + zetl" ·
[[SPEC-048-components-and-static-overrides#ADR-4810]] transclusion is an addressed site
capability, not a page tier.

**Load-bearing requirements:**
[[SPEC-048-components-and-static-overrides#REQ-4801]] site/page tier split ·
[[SPEC-048-components-and-static-overrides#REQ-4802]] depth-correct `root_path` ·
[[SPEC-048-components-and-static-overrides#REQ-4805]] macro substrate + optional sugar ·
[[SPEC-048-components-and-static-overrides#REQ-4811]] templated static pages ·
[[SPEC-048-components-and-static-overrides#REQ-4812]] single-source merged tokens ·
[[SPEC-048-components-and-static-overrides#REQ-4813]] byte-identical default ·
[[SPEC-048-components-and-static-overrides#REQ-4818]] addressed vault transclusion.

**Open** (each blocks the Phase 2 gate — see
[[SPEC-048-components-and-static-overrides#12. Open Questions]]):
Q1 static-render marker · Q5 cross-vault packages · Q6 sidebar vs top-strip ·
Q7 unscoped-CSS collision convention · Q8 transclusion syntax form (owner: spec author,
to ground in Phase 1 / IMPL-048). *(Q3 embed-relationship resolved by REQ-4818.)*

**Detail:** the full requirement, contract, and test nodes follow below — this
one-pager is the door, not the room.

> **Conformance.** The key words MUST, MUST NOT, REQUIRED, SHALL, SHALL NOT, SHOULD,
> SHOULD NOT, RECOMMENDED, MAY, and OPTIONAL in this document are to be interpreted as
> described in BCP 14 (RFC 2119, RFC 8174) when, and only when, they appear in all
> capitals ([[PROTO-001#Requirement-Level Keywords (BCP 14)]]).

> **Strawman notice.** A *first* draft from a design exploration — **NOT** converged.
> No Phase 1 surveys, no synthetic-user runs, no fresh-context adversarial review. Per
> [[PROTO-001]] Constitutional Principle 11 ([[Anti-Slop Bias]]), treat every clause
> as carrying hidden debt until adversarial review proves otherwise.
> **`[Blocked: Qn]`** marks a clause depending on an open question that MUST NOT pass
> the Phase 2 gate until it closes; **`[Provisional]`** marks a value still to be
> grounded in Phase 1.

## Information Table

| Field        | Value                                                                                  |
| ------------ | -------------------------------------------------------------------------------------- |
| Document ID  | [[SPEC-048-components-and-static-overrides\|SPEC-048]]                                  |
| Title        | Template Components & Templated Static Pages                                            |
| Version      | 0.2.1-strawman                                                                          |
| Status       | Draft (strawman; NOT converged — pending Phase 1 + Phase 2 gates)                       |
| Author       | Agent (Claude Opus 4.8 [1M], [[PROTO-001\|USDD Agent Protocol]] v1.8.0)                 |
| Date         | 2026-06-24                                                                              |
| Audience     | Agent, Human                                                                            |
| Trace        | [[PROTO-001]] §Phase 1, §Phase 2, §LangSec, §UX Heuristics, §AI Trust Boundaries        |
| Source       | minijinja macro/call substrate; observed drift in the anuna-web site (§1.1)             |
| Related      | [[SPEC-032]] AST/Hooks, [[SPEC-028]] SPA shell, [[SPEC-045-wikilink-predicate-language]] edges, [[SPEC-040-zetl-mobile]], [[SPEC-002]] search |
| Successors   | SPEC-049 content directives, SPEC-050 islands + messaging, SPEC-051 scoped-CSS compiler |
| Feature Gate | `components` (component definition + invocation); `static-render` (templated static pages) |
| Review tier  | Tier 2 (a trust boundary: author-supplied token values + props cross into a CSS/HTML context) |

---

## 1. Overview

### 1.1 Problem

zetl renders every vault page through [[minijinja]] with a rich context, and its
themes already compose internally via `{% extends %}`, `{% include %}`, and a
self-recursive `{% macro render_tree %}`. But two capabilities are missing, and
their absence produces concrete, shipped duplication:

1. **No reusable, parameterised component unit.** A theme cannot define a
   `nav-header` once and invoke it with props in several places; it must inline or
   hand-copy markup. minijinja supports `{% macro %}` + `{% call %}` (parameterised
   fragments with a default `caller()` slot), but zetl exposes no component
   convention over them — no manifest, no prop validation, no asset collection.
   This is a [[Right Place for the Function|Principle-15]] gap.

2. **Static override pages are copied verbatim, not rendered.** Anything an author
   drops into the vault that is not a `.md` page (a hand-authored landing page, an
   `about` page, a `404`) is handled by `copy_static_assets` — **no templating,
   no shared context**. Such a page therefore *cannot* share a component, a token,
   or a nav with the themed pages. The two render paths share no context, and only
   one path renders at all.

**Evidence (the [[anuna-web]] site).** A real zetl-based site exhibits exactly
the drift this spec exists to prevent — measured, not hypothetical:

| Thing that should have one source of truth | Observed reality |
| ------------------------------------------ | ---------------- |
| `--moss` brand colour                      | **three** values in play: `#5B8C5A` (theme + `static/css/style.css`), `#4da6a6` (membership page inline), `#80b47f` (tools page + global-nav fallback) |
| Full design-token set (bg/text/moss/warm/fonts/wraps) | defined **twice identically** (`theme/base.html` `:root` ≡ `static/css/style.css` `:root`), then re-overridden per page |
| Nav link list (Tools / Blog / Membership / CTA) | hardcoded in **~6** static pages **and** again as the theme sidebar |
| Wordmark markup                            | drifts *within* static: `class="nav-wordmark">anuna` vs `class="wordmark">anuna<span class="dot">·` |
| Link base                                  | static pages use absolute `/tools/`; themed pages use `{{ root_path }}…` — the static form **breaks on `file://` and non-root deploys** |

This is, concretely, what a "Jekyll + zetl" split would *institutionalise*: two
generators cannot share a render context, so the nav and tokens would be hand-synced
across a tool boundary — relocating the drift, not curing it. The fix is for zetl to
render its own static pages with the **same** site context the themed pages use.

There are two *distinct* duplications, and they want different fixes — conflating
them is the central design trap ([[SPEC-048-components-and-static-overrides#ADR-4804]]):

- **Intra-static duplication** (pure waste): the ~6 marketing pages each re-roll the
  same nav and re-declare the same tokens inline. Fix: extract one component + one
  token file, and *render* the static pages instead of copying them.
- **Cross-boundary duplication** (static ↔ themed vault): the vault legitimately uses
  a *sidebar* while the marketing pages use a *horizontal strip*. The markup
  difference is real; what must be shared is the **tokens**, the **nav link data**,
  and the **link base** — not necessarily the markup.

### 1.2 Core Insight

**Components and static overrides are the same problem viewed twice.** Both want
*layered, granular, composable resolution* in place of *opaque whole-file
replacement*. The unifying primitive is a **context tier split**:

- A **[[Site Context]]** — site name, navigation data, link base (`root_path`),
  design tokens, build mode — guaranteed present in **every** render path, including
  static override pages.
- A **[[Page Context]]** — title, content, backlinks, frontmatter, edges — present
  only on rendered content pages.

A component declares which tiers it consumes. A `nav-header` that declares
`requires = ["site"]` is, *by construction*, legal to render in both a themed page
and a static override page, because both paths expose site context. A backlinks
panel that declares `requires = ["page"]` is statically rejected from a static
override page where no page context exists
([[SPEC-048-components-and-static-overrides#REQ-4808]]).

### 1.3 Design Principles

1. **Specify a convention, not a new engine.** A component *is* a minijinja macro;
   the `{% component %}` sugar (if provided) lowers to `{% call %}` + `{% set %}`
   capture before parse. No custom statement tag is added to minijinja (it has no
   API for one), and no new expression language is introduced
   ([[SPEC-048-components-and-static-overrides#ADR-4803]], [[PROTO-001]] Principle 15).
2. **Static-first output.** v1 output is **fully server-rendered HTML+CSS** — it
   works on `file://` with JS disabled, and is fully indexable by [[SPEC-002]] search
   by construction (no client-rendered content exists until islands land in
   SPEC-050). ([[SPEC-048-components-and-static-overrides#ADR-4801]]).
3. **One resolver for templates, components, and statics.** Component lookup and
   static-page layering reuse the existing three-tier theme fallback
   ([[SPEC-048-components-and-static-overrides#ADR-4806]]).
4. **Recognise before acting ([[LangSec]]).** Component manifests, invocation
   attributes, and token files each have a declared grammar with concrete bounds;
   recognition completes before any render
   ([[SPEC-048-components-and-static-overrides#CON-4801]]–[[SPEC-048-components-and-static-overrides#CON-4804]]).
5. **No behaviour without invocation.** A vault with no components, no `tokens.toml`,
   and no templated static page behaves byte-identically to the pre-SPEC-048 release
   ([[SPEC-048-components-and-static-overrides#REQ-4813]]).
6. **The link base is shared, not retyped.** `root_path` is part of site context,
   computed from each output file's location — including static pages
   ([[SPEC-048-components-and-static-overrides#REQ-4802]]).
7. **Nesting is bounded and halting.** Macro-in-macro expansion is cycle-detected
   statically and bounded at render
   ([[SPEC-048-components-and-static-overrides#REQ-4807]]).

### 1.4 Scope

**In scope (v1 core):** the [[Site Context]]/[[Page Context]] tier split
([[SPEC-048-components-and-static-overrides#REQ-4801]],
[[SPEC-048-components-and-static-overrides#REQ-4802]]); a macro-based
[[zetl Component]] definition + manifest
([[SPEC-048-components-and-static-overrides#REQ-4803]]); component resolution via the
existing fallback ([[SPEC-048-components-and-static-overrides#REQ-4804]]);
template-author invocation ([[SPEC-048-components-and-static-overrides#REQ-4805]]);
bounded acyclic nesting ([[SPEC-048-components-and-static-overrides#REQ-4807]]);
context declaration + static verification
([[SPEC-048-components-and-static-overrides#REQ-4808]]); deduped, deterministic
emission of (unscoped) component CSS
([[SPEC-048-components-and-static-overrides#REQ-4809]]); templated static override
pages ([[SPEC-048-components-and-static-overrides#REQ-4811]]); design tokens with
merge-not-replace ([[SPEC-048-components-and-static-overrides#REQ-4812]]); a
backward-compatible default ([[SPEC-048-components-and-static-overrides#REQ-4813]]);
prop validation ([[SPEC-048-components-and-static-overrides#REQ-4814]]); addressed
vault transclusion into any render path
([[SPEC-048-components-and-static-overrides#REQ-4818]]).

**Deferred to successors (NOT in this spec):** content-author Markdown directives +
output sanitisation (→ **SPEC-049**); JS islands + inter-island messaging bus +
manifest-declared topics (→ **SPEC-050**); build-time **scoped** CSS / selector
rewriting (→ **SPEC-051**). See
[[SPEC-048-components-and-static-overrides#Deferred Capabilities (Successor Specs)]].

**Permanently out of scope:** runtime shadow-DOM / custom-element registration; a CSS
preprocessor (Sass/Less); arbitrary code execution inside components (no
Turing-complete component logic beyond minijinja's expression language —
[[SPEC-048-components-and-static-overrides#ADR-4803]]); cross-vault component
registries (`[Blocked: Q5]`); theming the graph widget internals.

---

## Deferred Capabilities (Successor Specs)

To keep the v1 core convergeable, four capabilities from the v0.1.x drafts are moved
to named successors. The REQ/ADR/CON numbers they vacated are **retired, not reused**
(gaps are permitted — [[PROTO-001]] §Numbering Rules).

| Capability (former clause) | Why deferred | Successor |
| -------------------------- | ------------ | --------- |
| Content-author Markdown directives `:::name{…}` + output sanitisation (REQ-4806, REQ-4815, CON-4803, ADR-4802, Threat A) | Untrusted-input trust boundary; needs a settled sanitiser policy (old Q2) and resolves a transform-stage-vs-render-stage seam — too much for the core | **SPEC-049** Content-Author Components |
| JS islands, inter-island messaging bus (`store`/`bus`), manifest-declared topics (REQ-4810, REQ-4816, REQ-4817, ADR-4808) | Substantial runtime on the [[SPEC-028]] shell; v1 is intentionally JS-free. Note: a content-authored island's bus "isolation" is only a naming convention against same-realm JS — must be designed as real enforcement there | **SPEC-050** Component Islands & Messaging |
| Build-time **scoped** CSS (selector rewriting to `[data-z]`, scope-escape rejection — the scoping half of REQ-4809, Threat C) | Needs a full CSS parser; attribute-prefix scoping also leaks across nesting unless every element is stamped — a real design problem deferred from the core. v1 emits component CSS **unscoped + deduped** | **SPEC-051** Scoped-CSS Compiler |

Successor numbers are candidate allocations (gaps allowed), to be confirmed when each
successor is opened. SPEC-047 remains reserved for contribution-authenticity/signing
([[SPEC-046-okf-interchange#Q7]]).

---

## 2. User Profiles

> **`[Provisional — refined by Phase 1 synthetic-user runs.]`**

### 2.1 The Theme Author
Builds and maintains a zetl theme. Wants to define a `nav-header`, `callout`, or
`card` once and reuse it across `base.html`, `index.html`, and folder pages with
different props — without copy-paste drift.

### 2.2 The Site Operator (anuna-web archetype)
Runs a real site that mixes generated vault pages with a few hand-authored
marketing/landing pages. Wants one nav and one set of brand tokens across **both**
surfaces, a single edit to propagate everywhere, and **does not** want a second
generator (Jekyll) to maintain. Today suffers measured drift
([[SPEC-048-components-and-static-overrides#1.1 Problem]]).

### 2.3 The Reviewer / Auditor
Wants to verify that a component used on a static page only needs site context, that
prop and token values cannot break out of their CSS/HTML context, and that nesting
cannot blow up a build. Depends on the declarations and bounds being machine-checkable.

---

## 3. Happy Paths

> **`[Provisional — refined by Phase 1.]`**

### 3.1 HP1: Default — No Components, Nothing Changes
**Pre:** vault defines no components, no `tokens.toml`, marks no static page for
rendering. **Post:** byte-identical output to the pre-SPEC-048 release; static files
copied verbatim as today ([[SPEC-048-components-and-static-overrides#REQ-4813]],
[[SPEC-048-components-and-static-overrides#TEST-4813]]).

### 3.2 HP2: Define and Invoke a Template Component
The theme author creates `components/callout/` (macro template + manifest + optional
CSS), then writes `{% component "callout" tone="warning" %}Heads up.{% endcomponent %}`
in `page.html` (or the equivalent native `{% call %}`). The build resolves the
component through the three-tier fallback, validates `tone` against the manifest,
routes the inner content to the default slot, and emits `callout.css` once into
`_static/` ([[SPEC-048-components-and-static-overrides#REQ-4803]],
[[SPEC-048-components-and-static-overrides#REQ-4805]],
[[SPEC-048-components-and-static-overrides#REQ-4809]]).

### 3.3 HP3: One nav-header Across Vault Pages AND a Static Override Page
The operator defines `components/nav-header/` with `requires = ["site"]`. In
`base.html` they write `{% component "nav-header" active=active_slug %}`. They rename
`static/about.html` to `static/about.html.jinja`; the build now **renders** it with
site context, and the same `nav-header` resolves there (the optional `active` prop
omitted). Both surfaces show the identical nav; links resolve correctly at every
depth via `root_path`. This is the path that replaces a separate generator for the
marketing pages ([[SPEC-048-components-and-static-overrides#REQ-4802]],
[[SPEC-048-components-and-static-overrides#REQ-4808]],
[[SPEC-048-components-and-static-overrides#REQ-4811]]).

### 3.4 HP4: One Token File Feeds Both Surfaces
The operator extracts brand tokens to `tokens.toml`. The build renders one
`_static/tokens.css`; both `base.html` and `about.html.jinja` link it; component
stylesheets consume the same custom properties. `--moss` is now defined in exactly
one place; the three-greens drift is structurally impossible
([[SPEC-048-components-and-static-overrides#REQ-4812]],
[[SPEC-048-components-and-static-overrides#TEST-4812]]).

### 3.5 HP5: A Component Bomb Fails Closed
A component that recursively invokes itself, or two components that invoke each
other, are detected at compile time (cycle) or capped at render time (depth),
producing a [[HookDiagnostic]] and a non-zero build under `--strict`, never a hang or
OOM ([[SPEC-048-components-and-static-overrides#REQ-4807]],
[[SPEC-048-components-and-static-overrides#Threat B]]).

### 3.6 HP6: A Static Page Transcludes Live Wiki Content
The operator wants their `about.html.jinja` landing page to show the project's mission
straight from the vault, so it can never go stale. They write
`{{ transclude("handbook#mission") }}`. The build resolves it through the same
[[Embed]] resolver that backs `![[handbook#mission]]` on themed pages, renders that
section's HTML into the static page, and exposes `transclude("handbook").title` for a
heading. Editing the mission once in `handbook.md` updates both the wiki page and the
landing page. Backlinks and edges of `handbook` remain unreachable from the static
page; renaming `handbook` surfaces a dead link in `zetl check`
([[SPEC-048-components-and-static-overrides#REQ-4818]],
[[SPEC-048-components-and-static-overrides#CON-4806]],
[[SPEC-048-components-and-static-overrides#ADR-4810]]).

---

## 4. Functional Requirements

> Numbering: SPEC-048 → REQ-48xx. Retained numbers from v0.1.x are kept; gaps
> (4806, 4810, 4815–4817) point to successors. Each REQ is atomic and decomposed into
> positive / negative-input / negative-output tests ([[PROTO-001]] §9).

### REQ-4801: Render-Context Tier Split (Site vs Page)
The system SHALL partition the minijinja render context into two named tiers: a
**[[Site Context]]** available to every render path, and a **[[Page Context]]**
available only when rendering a content page. Site context SHALL be a strict subset
shared by all paths; page context SHALL NOT be reachable from any path that lacks a
page (folder indexes expose site + a folder tier; static override pages expose site
only). The existing `vault` / `page` template variables SHALL be preserved as today
for backward compatibility; the tier split is an *additional* declared boundary used
for component context-requirement verification
([[SPEC-048-components-and-static-overrides#REQ-4808]]), not a rename. The set of
tiers a given render path exposes SHALL be a statically known property of that path
(content page → {site, page}; folder index → {site, folder}; static override →
{site}), so context-requirement checks can run at compile time.

**Trace:** [[SPEC-048-components-and-static-overrides#TEST-4801]], [[SPEC-048-components-and-static-overrides#ADR-4804]], [[SPEC-048-components-and-static-overrides#CON-4805]].

### REQ-4802: Site Context Contents and Depth-Correct Link Base
[[Site Context]] SHALL contain at minimum: the site/vault name; the navigation data
(`site.nav`, an ordered list of `{label, href}` plus the existing `sidebar_tree`);
the design tokens ([[SPEC-048-components-and-static-overrides#REQ-4812]]); the build
mode; and a **`root_path`** computed from the *output location of the file currently
being rendered*, by the identical rule used for content pages (`compute_root_path` in
`src/web/engine.rs`: `/` in serve mode, `./` / `../`×depth in build mode). A static
override page at output depth N SHALL receive the same `root_path` a content page at
depth N receives. A component or static page that emits a navigation link SHALL build
it as `{{ root_path }}<target>` and SHALL NOT hardcode a leading `/`. Rationale: a
shared nav is only correct if its links resolve at every depth and under `file://`;
an absolute `/tools/` breaks both.

**Trace:** [[SPEC-048-components-and-static-overrides#TEST-4802]], [[SPEC-048-components-and-static-overrides#CON-4805]]; [[SPEC-048-components-and-static-overrides#3.3 HP3]].

### REQ-4803: zetl Component Definition
A **[[zetl Component]]** SHALL be a directory `components/<name>/` resolvable in the
vault (`.zetl/components/<name>/`) or a theme (`<theme>/components/<name>/`),
containing: a required template `<name>.html` (a [[minijinja]] fragment defining the
component body, reading `props.*` and rendering [[Slot]] content); a required
manifest `<name>.toml` ([[Component Manifest]],
[[SPEC-048-components-and-static-overrides#CON-4801]]); and an optional stylesheet
`<name>.css` ([[SPEC-048-components-and-static-overrides#REQ-4809]]). `<name>` SHALL
match `[a-z][a-z0-9-]*` (kebab-case, [[PROTO-001]] feature-naming). A component
directory missing its template or manifest, or with a `<name>` out of grammar, SHALL
be rejected at compile time with a [[HookDiagnostic]] (`component-malformed`, error).
(An optional island script `<name>.js` is reserved for **SPEC-050** and SHALL be
ignored — not emitted — in v1.)

**Trace:** [[SPEC-048-components-and-static-overrides#TEST-4803]], [[SPEC-048-components-and-static-overrides#CON-4801]].

### REQ-4804: Component Resolution via Three-Tier Fallback
Component lookup by `<name>` SHALL reuse the existing template-resolution precedence
([[Three-Tier Resolution]] in `src/web/engine.rs`): vault
`.zetl/components/<name>/` overrides the active theme's `components/<name>/`, which
overrides the bundled default theme's. Resolution SHALL be **whole-directory** (a
vault override replaces the theme component's template, manifest, and CSS as a unit —
never a partial merge), so that overriding a component overrides its emitted statics
by construction. An unresolvable `<name>` at an invocation site SHALL be a
compile-time error (`component-not-found`), never a silent empty render.

**Trace:** [[SPEC-048-components-and-static-overrides#TEST-4804]], [[SPEC-048-components-and-static-overrides#ADR-4806]].

### REQ-4805: Template-Author Invocation (Macro Substrate + Optional Sugar)
A component SHALL be **compiled to / loaded as a minijinja macro** whose parameters
are the component's props and whose default [[Slot]] is minijinja `caller()`
([[SPEC-048-components-and-static-overrides#ADR-4803]]). The engine SHALL make a
resolved component invocable from any template and from templated static pages
([[SPEC-048-components-and-static-overrides#REQ-4811]]) by:

- **(native form)** importing the component macro and invoking it via
  `{% call <name>(k=v …) %}…slot…{% endcall %}` (and a no-body call for the
  self-closing case); and
- **(sugar form, OPTIONAL)** a thin **source-lowering pass** that rewrites
  `{% component "<name>" k=v … %}…slot…{% endcomponent %}` and named
  `{% slot "<slotname>" %}…{% endslot %}` blocks into native minijinja **before
  parse**: the default body lowers to `caller()`; each named slot lowers to a
  `{% set _slot_<slotname> %}…{% endset %}` capture passed as a macro argument the
  component renders as `{{ <slotname> }}`. The pass introduces **no** custom
  statement tag into minijinja and **no** new expression language.

Invocation SHALL resolve `<name>`
([[SPEC-048-components-and-static-overrides#REQ-4804]]), validate keyword arguments
against the manifest ([[SPEC-048-components-and-static-overrides#REQ-4814]]) in the
component-resolution layer (not the engine), and render the component with `props` +
the component's declared context tiers
([[SPEC-048-components-and-static-overrides#REQ-4808]]). Attribute syntax is governed
by [[SPEC-048-components-and-static-overrides#CON-4802]].

**Trace:** [[SPEC-048-components-and-static-overrides#TEST-4805]], [[SPEC-048-components-and-static-overrides#CON-4802]], [[SPEC-048-components-and-static-overrides#CON-4805]]; [[SPEC-048-components-and-static-overrides#3.2 HP2]].

### REQ-4807: Bounded, Acyclic Nesting
Component invocation MAY nest (a component macro MAY invoke other components; a slot
body MAY contain component invocations). The system SHALL bound nesting two ways:
(a) **static cycle detection** — template-author component invocations are
statically discoverable (a macro's `{% call %}`/`{% component %}` sites are visible
in source); the system SHALL build the component-invocation graph and reject a cycle
at compile time (`component-cycle`, error) naming the cycle path, reusing the
[[SPEC-032]] composition topo-sort *algorithm* (Kahn's) over component nodes — an
analogous use, not the same hook graph; (b) **render-time depth bound** — total
component nest depth SHALL be capped ([[SPEC-048-components-and-static-overrides#NFR-4803]]),
combining minijinja's own recursion limit with an explicit zetl depth counter; a
breach SHALL produce a [[HookDiagnostic]] (`component-depth-bound`) and fail the build
under `--strict` rather than recurse unboundedly.

**Trace:** [[SPEC-048-components-and-static-overrides#TEST-4807]], [[SPEC-032]]; [[SPEC-048-components-and-static-overrides#Threat B]].

### REQ-4808: Context-Requirement Declaration and Verification
Each [[Component Manifest]] SHALL declare `requires` — a subset of
`{"site", "page", "folder"}` ([[SPEC-048-components-and-static-overrides#REQ-4801]]).
Verification has **two distinct layers**, and the spec does not conflate them:

- **(compile-time, total)** For every static invocation site, the system SHALL
  compare the component's declared `requires` against the statically known tier set
  the render path exposes ([[SPEC-048-components-and-static-overrides#REQ-4801]]). A
  component requiring `page` invoked from a static override page (site-only) SHALL be
  a compile-time error (`component-context-unavailable`) naming the missing tier.
  This check needs only the manifest and the path kind — no template parsing.
- **(render-time, per-exercised-path)** Whether a component template actually reads
  only variables within its declared tiers (plus `props` and `slot`) SHALL be
  enforced by the existing strict-undefined render mode
  (`env.set_undefined_behavior(Strict)`, `src/web/engine.rs:284`): reading an
  undeclared/absent variable is a strict-undefined **render** failure, not an empty
  string. The spec does NOT claim a compile-time guarantee here — minijinja exposes
  no public template AST to statically enumerate variable reads.

`requires = ["site"]` is the precondition that makes a component cross-context
(themed-page + static-page) reusable, and that precondition is checked at compile
time per the first layer.

**Trace:** [[SPEC-048-components-and-static-overrides#TEST-4808]], [[SPEC-048-components-and-static-overrides#CON-4801]], [[SPEC-048-components-and-static-overrides#CON-4805]]; [[SPEC-048-components-and-static-overrides#3.3 HP3]].

### REQ-4809: Deduplicated, Deterministic Component CSS Emission (Unscoped in v1)
WHEN a component ships `<name>.css`, the build SHALL (a) **collect and deduplicate** —
a component used N times on a page contributes its CSS **once**; and (b) **emit** the
deduped, deterministically ordered result into the existing `_static/` layer, linked
by every page that uses the component ([[SPEC-048-components-and-static-overrides#NFR-4802]]).
In v1 the CSS is emitted **as authored (unscoped)**; the component is a *trusted
theme/vault author* surface, so authors namespace their own selectors by convention.
**Selector scoping** (rewriting rules to a `[data-z="<name>"]` subtree, rejecting
scope-escaping selectors, and the full CSS parser that requires) is **deferred to
SPEC-051** ([[SPEC-048-components-and-static-overrides#Deferred Capabilities (Successor Specs)]]).
v1 SHALL still stamp the component root with a `data-z="<name>"` marker so SPEC-051
can scope later without changing emitted HTML.

**Trace:** [[SPEC-048-components-and-static-overrides#TEST-4809]], [[SPEC-048-components-and-static-overrides#NFR-4802]].

### REQ-4811: Templated Static Override Pages
A static file whose name ends in `.html.jinja` (and `[Blocked: Q1]` whether a
front-matter flag on a plain `.html` is also honoured) under the vault's static
sources SHALL be **rendered through minijinja with [[Site Context]] only** (plus
component invocation), its `.html.jinja` suffix reduced to `.html` in the output
(the existing pretty-URL `/index.html` convention applies exactly as it does to
content pages; `foo.html.jinja → foo/index.html`, `index.html.jinja → index.html`).
A plain static file with no render marker SHALL be copied verbatim exactly as today
([[SPEC-048-components-and-static-overrides#REQ-4813]]). The four-tier static layering
precedence (`bundled default → bundled theme → .zetl/static → theme static`) SHALL be
preserved; the render pass applies **after** resolution, to the winning file. A
templated static page MUST NOT access page context; an attempt is a compile-time
error per the first layer of
[[SPEC-048-components-and-static-overrides#REQ-4808]]. Because the output is fully
server-rendered HTML, it remains indexable by [[SPEC-002]] search with no extra work.

**Trace:** [[SPEC-048-components-and-static-overrides#TEST-4811]], [[SPEC-048-components-and-static-overrides#REQ-4802]], [[SPEC-048-components-and-static-overrides#ADR-4805]]; [[SPEC-048-components-and-static-overrides#3.3 HP3]].

### REQ-4812: Design Tokens — Single Source, Merge Not Replace
The system SHALL read an optional `tokens.toml` ([[Design Token]] table,
[[SPEC-048-components-and-static-overrides#CON-4804]]) and render exactly one
`_static/tokens.css` exposing each token as a CSS custom property under a single
`:root` (and an optional `[data-theme]` block for light/dark). Token resolution SHALL
**merge** across layers — a vault `tokens.toml` overrides individual theme tokens
key-by-key (not wholesale file replacement), so changing one variable needs one line.
Both themed pages and templated static pages SHALL link the single emitted
`tokens.css`; component stylesheets
([[SPEC-048-components-and-static-overrides#REQ-4809]]) SHALL consume these custom
properties rather than re-declare values. Token *values* are recognised against
[[SPEC-048-components-and-static-overrides#CON-4804]] and SHALL NOT be able to break
out of the CSS declaration context
([[SPEC-048-components-and-static-overrides#Threat F]]).

**Trace:** [[SPEC-048-components-and-static-overrides#TEST-4812]], [[SPEC-048-components-and-static-overrides#CON-4804]]; [[SPEC-048-components-and-static-overrides#3.4 HP4]].

### REQ-4813: Backward-Compatible Default
WHEN a vault defines no components, no `tokens.toml`, and no `.html.jinja` static
page, the build output SHALL be byte-identical to the pre-SPEC-048 release: static
files copied verbatim, no `tokens.css` emitted, no component statics, no lowering
pass invoked. All SPEC-048 behaviour SHALL be reachable only by opting in (defining a
component, a token file, or a `.html.jinja` page).

**Trace:** [[SPEC-048-components-and-static-overrides#TEST-4813]]; [[SPEC-048-components-and-static-overrides#3.1 HP1]].

### REQ-4814: Prop Validation Against the Manifest
On invocation, each supplied keyword argument SHALL be validated **in the
component-resolution layer** (not minijinja, which only knows positional/keyword
binding) against the component manifest's `[props]` schema
([[SPEC-048-components-and-static-overrides#CON-4801]]): an unknown prop name → error
(`component-unknown-prop`); a value of the wrong declared type → error
(`component-prop-type`); a missing required prop with no default → error
(`component-prop-missing`); a declared `enum` value outside its set → error
(`component-prop-enum`). Defaults from the manifest SHALL fill omitted optional props.
Validation SHALL complete before the component macro renders (recognise before act).
Props flow into an HTML attribute / text context inside the component, so values
SHALL be escaped at use per the component template's autoescape, never concatenated
raw ([[SPEC-048-components-and-static-overrides#Threat E]]).

**Trace:** [[SPEC-048-components-and-static-overrides#TEST-4814]], [[SPEC-048-components-and-static-overrides#CON-4801]], [[SPEC-048-components-and-static-overrides#CON-4802]]; [[SPEC-048-components-and-static-overrides#Threat E]].

### REQ-4818: Addressed Vault Transclusion in Site Context
[[Site Context]] SHALL expose a read-only **[[Vault Transclusion]]** capability — a
minijinja function `transclude(<target>)` — available to every render path (themed
pages, folder indexes, and templated static override pages
([[SPEC-048-components-and-static-overrides#REQ-4811]])). The capability SHALL resolve
`<target>` as a **named, content-addressed** reference (a whole page, a `#heading`
section, or a `#^block-id` block) **reusing the existing [[Embed]] resolver** that
backs `![[…]]` transclusion — one recogniser, one resolver
([[SPEC-048-components-and-static-overrides#ADR-4810]], [[PROTO-001]] §LangSec
one-parser-per-language). It SHALL return the *rendered HTML* of the addressed
content plus an **allow-listed metadata subset** (`title`, and frontmatter fields the
target marks publishable) — and SHALL NOT expose the target page's backlinks, edges,
raw frontmatter, or any other page-tier field, so addressing a *named* page does not
re-admit the ambient [[Page Context]] forbidden on static pages
([[SPEC-048-components-and-static-overrides#Threat G]],
[[SPEC-048-components-and-static-overrides#Threat H]]). The capability SHALL respect
page visibility — a `transclude` of a draft/unpublished or non-existent target SHALL
fail closed with a [[HookDiagnostic]] (`transclude-target-unresolved`, error) and
SHALL surface as a dead link in `zetl check --dead-links`, never as silent empty
output. Transclusion MAY nest (a transcluded page may itself transclude) and SHALL be
bounded by the [[SPEC-048-components-and-static-overrides#REQ-4807]] depth/cycle bound,
sharing its counter so a transclude chain cannot exceed the nest cap nor form a cycle.
Because resolution and render happen at build time, transcluded content SHALL remain
byte-deterministic ([[SPEC-048-components-and-static-overrides#NFR-4802]]) and fully
indexable by [[SPEC-002]] search.

**Trace:** [[SPEC-048-components-and-static-overrides#TEST-4818]], [[SPEC-048-components-and-static-overrides#CON-4806]], [[SPEC-048-components-and-static-overrides#ADR-4810]], [[SPEC-048-components-and-static-overrides#REQ-4807]]; [[SPEC-048-components-and-static-overrides#Threat H]]; [[SPEC-048-components-and-static-overrides#3.6 HP6]].

---

## 5. Non-Functional Requirements

### NFR-4801: Per-Page Component Expansion Latency
Component expansion SHALL add ≤ 15% to the per-page render time at the 95th
percentile for a page invoking ≤ 20 component instances of ≤ 3 distinct types,
measured over 50 runs on the project reference CI runner (`[Provisional: pin runner
class/CPU in IMPL-048]`). The 20-instance / 3-type figure is `[Provisional]` pending
the Phase 1 theme-complexity survey. The dominant no-component page
([[SPEC-048-components-and-static-overrides#REQ-4813]]) carries **zero** added cost.

**Trace:** [[SPEC-048-components-and-static-overrides#TEST-4801]], [[SPEC-048-components-and-static-overrides#OBS-4801]].

### NFR-4802: Deterministic, Deduplicated Asset Emission
For a given (vault, theme, options) tuple, the emitted `tokens.css` and the set and
order of component CSS blocks SHALL be byte-identical across repeated builds (no
map-iteration-order, no wall-clock). Deduplication SHALL be exact: a component used N
times emits its CSS once. Ordering SHALL be a declared total order (component name,
then source layer) so output is reproducible.

**Trace:** [[SPEC-048-components-and-static-overrides#TEST-4809]], [[SPEC-048-components-and-static-overrides#OBS-4802]].

### NFR-4803: Component Input Bounds (Fail-Closed)
The recognisers SHALL enforce concrete fail-closed bounds, checked before render:
manifest ≤ 64 KiB; per-component template ≤ 1 MiB; per-component CSS ≤ 1 MiB; props
per invocation ≤ 64; per-prop value ≤ 64 KiB; component nest depth ≤ 16
([[SPEC-048-components-and-static-overrides#REQ-4807]]). All numeric values are
`[Provisional]` defaults to confirm in IMPL-048; the *presence* of each bound and the
fail-closed rule are normative.

**Trace:** [[SPEC-048-components-and-static-overrides#TEST-4807]], [[SPEC-048-components-and-static-overrides#OBS-4803]]; [[SPEC-048-components-and-static-overrides#Threat B]].

---

## 6. Architecture Decision Records

### ADR-4801: Build-Time Server Render, Not Runtime Shadow DOM
Encapsulation and composition are resolved at build time (macro expansion + deduped
emission), not via browser shadow DOM or custom-element registration. (+) Works on
`file://` and with JS disabled; no runtime cost; matches zetl's static-first output;
fully indexable by [[SPEC-002]]. (−) No true *style* isolation in v1 (component CSS
is unscoped — selector scoping is deferred to SPEC-051, and even attribute scoping is
not shadow-DOM isolation). Rejected: runtime web components (requires always-on JS,
breaks static export).

### ADR-4803: Components ARE minijinja Macros — No New Engine, No New Tag
A component is implemented as a minijinja **macro**; slots are `caller()` / captured
`{% set %}` blocks; props are macro arguments. The optional `{% component %}` sugar is
a **source-lowering pass** into native `{% call %}` (+ `{% set %}` captures for named
slots) that runs before parse. (+) Reuses a proven engine (the theme `render_tree`
macro already nests recursively); inherits autoescape, chainable/strict-undefined; no
custom statement tag is added to minijinja — which is load-bearing, because **minijinja
exposes no API to register one** (confirmed: it is extended only via
`add_filter`/`add_function`/`add_global` in `src/web/engine.rs`). (−) Component logic
is limited to minijinja's expression language (no arbitrary code) — accepted, and a
security feature ([[PROTO-001]] LangSec principle 6). Rejected: (1) a custom
`{% component %}` engine tag — **infeasible** on stock minijinja; (2) a bespoke
component VM — unjustified new surface ([[PROTO-001]] Principle 15); (3) adopting a
separate static-site generator (Jekyll/Hugo) for static pages — would split the render
context across a tool boundary and *re-create* the §1.1 drift, the very thing this
spec removes ([[SPEC-048-components-and-static-overrides#ADR-4809]]).

### ADR-4804: Site/Page Context Tiering Is the Load-Bearing Primitive
Rather than make every render path carry every variable, the context is split into
declared tiers, and components declare which they need. (+) Makes cross-context reuse
(static ↔ themed) *checkable* at compile time; a `requires=["site"]` nav-header is
provably legal on a static page; the failure mode (page-only component on a static
page) is a clear compile error, not a blank render. (−) One more concept for theme
authors. This is the decision that distinguishes the two duplications in §1.1 and
routes each to the right fix.

### ADR-4805: Static Override Pages Render Through Site Context (Opt-In by Suffix)
A `.html.jinja` static file is rendered with site context; a plain static file is
copied verbatim. (+) Backward compatible by construction
([[SPEC-048-components-and-static-overrides#REQ-4813]]); the operator opts a single
page in by renaming; the depth-correct `root_path` fixes the absolute-link fragility
observed in anuna-web. (−) Two file conventions to learn; the plain-`.html`
front-matter-flag alternative is `[Blocked: Q1]`. Rejected: render *all* static
`.html` (breaks verbatim assets; surprises operators who ship hand-tuned HTML).

### ADR-4806: Reuse the Three-Tier Resolver and Composition Cycle Detection
Component resolution reuses the existing theme fallback (`src/web/engine.rs`) and
cycle detection reuses the SPEC-032 composition topo-sort *algorithm* (applied to the
component-invocation graph, not the hook graph). (+) One resolution model for
templates, components, and statics; overriding a component overrides its statics for
free; the topo-sort is already implemented and tested. (−) Couples components to the
theme-resolution code path (acceptable — that is precisely the shared layer per
[[PROTO-001]] Principle 15).

### ADR-4807: SPEC-048 Number Allocation and Successor Gaps
This document is **SPEC-048**, skipping SPEC-047 (reserved for the
contribution-authenticity/signing capability from
[[SPEC-046-okf-interchange#Q7]]). The capabilities deferred in v0.2.0 are allocated
candidate successors **SPEC-049/050/051**
([[SPEC-048-components-and-static-overrides#Deferred Capabilities (Successor Specs)]]).
REQ/ADR/CON numbers vacated by the deferral (REQ-4806/4810/4815–4817, ADR-4802/4808,
CON-4803) are retired, not reused. Numbering gaps are permitted ([[PROTO-001]]
§Numbering Rules).

### ADR-4809: One Generator, Not Two (zetl Static-Render vs "Jekyll + zetl")
The marketing/landing pages are rendered **by zetl** (templated static overrides),
not by a second static-site generator running alongside zetl. (+) The shared
site context — `root_path`, `site.nav`, `tokens.css` — is one source of truth *by
construction*; a single edit propagates to both surfaces. (−) zetl does not (yet)
provide blog-collection/pagination machinery a generator like Jekyll/Hugo has — if the
marketing surface grows into a paginated CMS, revisit. Rejected: a parallel generator
for static pages — two engines cannot share a render context, so the nav and tokens
would be hand-synced across a tool boundary, *relocating* the §1.1 drift rather than
removing it. This ADR records the decision that motivated the v0.2.0 tightening: the
v1 core is exactly the slice needed to retire the second-generator proposal for the
anuna-web archetype.

### ADR-4810: Transclusion Is an Addressed Site Capability, Not a Page Tier
Pulling vault content into a static (or any) page is modelled as a **content-addressed
read function** (`transclude("page#section")`) living in [[Site Context]], NOT as
ambient access to a [[Page Context]] tier
([[SPEC-048-components-and-static-overrides#REQ-4818]]). The pivotal distinction:
*ambient* page context = "the content/backlinks of the page being rendered" (undefined
for a static page — correctly forbidden by
[[SPEC-048-components-and-static-overrides#Threat G]]); *addressed* transclusion =
"the published content of *this named page*" (a deliberate, explicit read). (+) Keeps
the tier model clean — tiers describe ambient context a path exposes; transclusion is
a function over named data, so it composes onto every path (incl. static) without
widening the page tier ([[PROTO-001]] Principle 15). (+) Reuses the existing
[[Embed]] resolver, so `![[embed]]` in `.md` and `transclude()` in templates share one
recogniser — this **resolves Q3** rather than growing a second transclusion path.
(+) The allow-listed exposed-field set
([[SPEC-048-components-and-static-overrides#CON-4806]]) keeps backlinks/edges/raw
frontmatter out, so an addressed read cannot smuggle the page tier back in. (−) Authors
learn one capability (mitigated: it mirrors `![[…]]` they already know). Rejected:
exposing a `page`/`vault` object with arbitrary fields to static pages — that *is* the
page tier under another name and re-opens [[SPEC-048-components-and-static-overrides#Threat G]];
rejected: a client-side fetch — breaks `file://`, determinism, and indexability
([[SPEC-048-components-and-static-overrides#ADR-4801]]).

---

## 7. Contracts (LangSec)

> Every contract below accepts author-supplied input and therefore declares a
> grammar; full recognition precedes any render ([[PROTO-001]] §LangSec). Grammars
> are EBNF-style sketches to be pinned exactly in IMPL-048.

### CON-4801: Component Manifest (`<name>.toml`)
**Interface:** the per-component manifest read at compile time.
**Grammar (subset of TOML, recognised by a strict TOML parser — no ad-hoc matching):**
```
manifest    = name-line , requires-line , [ slots-line ] , [ props-table ] ;
name        = "name" "=" quoted-kebab ;            (* MUST equal directory name *)
requires    = "requires" "=" "[" { tier } "]" ;    (* tier ∈ "site" | "page" | "folder" *)
slots       = "slots" "=" "[" { quoted-ident } "]" ; (* default slot implicit; named slots listed *)
props-table = "[props]" , { prop-def } ;
prop-def    = ident "=" "{" "type" "=" ptype
               [ "," "required" "=" bool ]
               [ "," "default" "=" literal ]
               [ "," "enum" "=" "[" { literal } "]" ] "}" ;
ptype       = "string" | "bool" | "int" | "number" | "list" | "map" ;
```
**Pre-conditions:** file ≤ 64 KiB ([[SPEC-048-components-and-static-overrides#NFR-4803]]);
valid UTF-8; `name` equals the directory; every `requires` tier in the allowed set.
**Post-conditions:** a typed `Manifest{ name, requires, slots, props }`; unknown
top-level keys → error (no inert tolerance — the manifest is zetl-defined, not an
external standard). The keys `publishes`/`subscribes` (island topics) are **reserved
for SPEC-050** and SHALL be rejected as unknown in v1. **Error model:** out-of-grammar
→ `component-malformed` (error), no partial accept.
**Implements:** [[SPEC-048-components-and-static-overrides#REQ-4803]],
[[SPEC-048-components-and-static-overrides#REQ-4808]],
[[SPEC-048-components-and-static-overrides#REQ-4814]].
**Verified by:** [[SPEC-048-components-and-static-overrides#TEST-4803]],
[[SPEC-048-components-and-static-overrides#TEST-4814]].

### CON-4802: Template Invocation Attributes
**Interface:** the keyword-argument list on a component invocation (native `{% call %}`
or the `{% component %}` sugar).
**Grammar:**
```
invocation = "{%" "component" string { kwarg } "%}" ;   (* sugar; lowers to {% call %} *)
kwarg      = ident "=" expr ;        (* expr = minijinja expression, NOT raw text *)
```
**Pre-conditions:** `string` resolves to a known component
([[SPEC-048-components-and-static-overrides#REQ-4804]]); each `ident` is a declared
prop ([[SPEC-048-components-and-static-overrides#REQ-4814]]); ≤ 64 kwargs.
**Post-conditions:** a validated `props` map bound for the component render; values
flow as typed minijinja values (escaped at use, never concatenated —
[[SPEC-048-components-and-static-overrides#Threat E]]). **Error model:** unknown prop
/ type mismatch / enum miss → compile-time error per
[[SPEC-048-components-and-static-overrides#REQ-4814]].
**Implements:** [[SPEC-048-components-and-static-overrides#REQ-4805]],
[[SPEC-048-components-and-static-overrides#REQ-4814]].
**Verified by:** [[SPEC-048-components-and-static-overrides#TEST-4805]],
[[SPEC-048-components-and-static-overrides#TEST-4814]].

### CON-4804: Design Tokens (`tokens.toml` → `tokens.css`)
**Interface:** the token table compiled to CSS custom properties.
**Grammar:**
```
tokens      = { token } , [ theme-table ] ;
token       = ident "=" value ;
theme-table = "[theme." ident "]" , { token } ;   (* e.g. [theme.light] *)
ident       = lower alnum-hyphen* ;                (* → --ident custom property *)
value       = css-safe-string ;                    (* see post-conditions *)
```
**Pre-conditions:** valid UTF-8; each `ident` matches the grammar; each `value`
recognised as a **CSS-token-safe** string: no `;`, no `}`, no `/*`/`*/`, no `<`/`>`,
no `url(` with a non-`data:`/non-relative scheme, no `\` escapes that re-open a
context — i.e. the value cannot terminate the declaration or open a new
rule/comment/markup context ([[SPEC-048-components-and-static-overrides#Threat F]]).
**Post-conditions:** exactly one `_static/tokens.css` with one `:root` (+ optional
`[data-theme=…]` blocks), each token a `--ident: value;` declaration; layered
**merge** semantics (vault key overrides theme key individually).
**Error model:** an out-of-grammar value → `tokens-value-unsafe` (error), no emission
of the offending declaration.
**Implements:** [[SPEC-048-components-and-static-overrides#REQ-4812]].
**Verified by:** [[SPEC-048-components-and-static-overrides#TEST-4812]].

### CON-4805: Component Render Contract
**Interface:** the act of rendering a resolved component with bound props + tiers.
**Pre-conditions:** props validated (CON-4801/CON-4802); the supplying render path
exposes every tier in `requires`
([[SPEC-048-components-and-static-overrides#REQ-4808]] compile-time layer); nest depth
within bound ([[SPEC-048-components-and-static-overrides#REQ-4807]]).
**Post-conditions (per-clause, one per implemented REQ):**
- (REQ-4805) the body is rendered into the default/named slots; the component root
  carries the `data-z="<name>"` marker (for SPEC-051 scoping later).
- (REQ-4808) the template reads only `props`, `slot`, and declared tiers; reading an
  undeclared variable is a strict-undefined **render** failure, not an empty string.
- (REQ-4809) the component's CSS is collected once for emission.
- (REQ-4802) any link the component emits is `root_path`-relative.
**Error model:** any precondition failure aborts the render with a [[HookDiagnostic]];
partial HTML is never emitted.
**Implements:** [[SPEC-048-components-and-static-overrides#REQ-4801]],
[[SPEC-048-components-and-static-overrides#REQ-4802]],
[[SPEC-048-components-and-static-overrides#REQ-4805]],
[[SPEC-048-components-and-static-overrides#REQ-4808]],
[[SPEC-048-components-and-static-overrides#REQ-4809]].
**Verified by:** [[SPEC-048-components-and-static-overrides#TEST-4805]],
[[SPEC-048-components-and-static-overrides#TEST-4808]].

### CON-4806: Vault Transclusion (`transclude(<target>)`)
**Interface:** the read-only site-context capability that resolves a named vault
reference to rendered HTML + safe metadata
([[SPEC-048-components-and-static-overrides#REQ-4818]]). The `<target>` is
author-supplied data and is therefore recognised against a grammar before any
resolution ([[PROTO-001]] §LangSec).
**Grammar (the existing [[Embed]] wikilink-target grammar — reused, not re-rolled):**
```
target   = page [ fragment ] ;
page     = path-segment { "/" path-segment } ;   (* a resolvable vault page name *)
fragment = "#" heading | "#^" block-id ;          (* section or block address *)
heading  = text ;                                 (* matched against the page's headings *)
block-id = ident ;                                (* matched against ^block markers *)
```
**Pre-conditions:** `<target>` matches the grammar; `page` resolves to an existing,
**publishable** vault page (draft/unpublished → fail closed); the fragment (if any)
resolves to a real heading/block; the transclude does not exceed the
[[SPEC-048-components-and-static-overrides#REQ-4807]] depth bound nor close a cycle.
**Post-conditions:** returns the addressed content as **rendered HTML** plus an
allow-listed metadata map limited to `{ title, <publishable frontmatter fields> }`;
the result re-enters the page as recognised, already-escaped output (the vault content
is trusted, but autoescape governs interpolation of the returned metadata values). The
exposed set is **closed** — `backlinks`, `edges`, `frontmatter` (raw/unpublishable),
and any other page-tier field are NOT reachable through this capability
([[SPEC-048-components-and-static-overrides#Threat H]]).
**Error model:** out-of-grammar `<target>`, unresolved page/fragment, or
draft/unpublished target → `transclude-target-unresolved` (error), surfaced as a dead
link in `zetl check --dead-links`; depth/cycle breach →
`component-depth-bound` ([[SPEC-048-components-and-static-overrides#REQ-4807]]); no
silent empty output in any case.
**Implements:** [[SPEC-048-components-and-static-overrides#REQ-4818]].
**Verified by:** [[SPEC-048-components-and-static-overrides#TEST-4818]].

---

## 8. Threat Model

The trust boundary: **theme/component authors are trusted** (they ship code);
**token values and props are author-supplied data** crossing into a CSS/HTML context.
(The *untrusted* content-author surface — Markdown directives — is deferred with its
threats to **SPEC-049**; v1 has no content-author component path.)

### Threat B: Expansion Bomb (Nesting DoS)
A self-referential component or a mutual cycle aiming to hang or OOM the build.
**Mitigation:** static cycle detection (compile error) + a render-time depth cap
combining minijinja's recursion limit with an explicit counter
([[SPEC-048-components-and-static-overrides#REQ-4807]],
[[SPEC-048-components-and-static-overrides#NFR-4803]]); breach → diagnostic +
fail-closed, never unbounded recursion.

### Threat D: Component Override Confusion
A vault `.zetl/components/<name>/` shadows a theme component. The vault author is
trusted, so this is *intended* override — but it must be **visible**: the build SHALL
log which layer each resolved component came from (`[zetl] component <name>: from
vault`), so an override is never silent
([[SPEC-048-components-and-static-overrides#OBS-4801]]).

### Threat E: Prop Injection into Attribute Context
A prop value like `" onload="alert(1)` aims to break out of an HTML attribute in the
component template. **Mitigation:** props flow as typed minijinja values under
autoescape; the component template interpolates `{{ props.x }}` (escaped), never
concatenates raw ([[SPEC-048-components-and-static-overrides#REQ-4814]],
[[SPEC-048-components-and-static-overrides#CON-4802]]).

### Threat F: CSS Injection via a Token Value
A `tokens.toml` value like `red; } body { display:none` aims to terminate the
declaration and inject a rule. **Mitigation:** CON-4804 recognises values as
CSS-token-safe (no `;`/`}`/comment/markup/`\`-reopen); an out-of-grammar value is
rejected, not emitted ([[SPEC-048-components-and-static-overrides#REQ-4812]]).

### Threat G: Static-Render *Ambient* Context Leak
A `.html.jinja` static page tries to reach the **ambient** page context (the content
or backlinks of "the current page", which does not exist for a static page).
**Mitigation:** static pages are rendered with site-only context; page-tier access is
a compile error (REQ-4808 first layer) and a strict-undefined render failure if
attempted dynamically ([[SPEC-048-components-and-static-overrides#REQ-4811]]). This is
distinct from **addressed** transclusion ([[SPEC-048-components-and-static-overrides#REQ-4818]]),
which reads a *named* page through a closed allow-list and is permitted — see
[[SPEC-048-components-and-static-overrides#Threat H]].

### Threat H: Transclusion Over-Exposure (Page-Tier Smuggling)
An author (or a crafted `<target>`) uses `transclude()` hoping to reach more than
rendered content — the target page's `backlinks`, `edges`, raw/unpublishable
frontmatter, or a draft page — thereby re-admitting the page tier that Threat G
closes, but through the addressed path. **Mitigation:** the
[[SPEC-048-components-and-static-overrides#CON-4806]] post-condition fixes a **closed
allow-list** (`title` + publishable frontmatter fields + rendered HTML only); no other
field is reachable, by construction. `<target>` is grammar-recognised before
resolution; draft/unpublished/non-existent targets fail closed
(`transclude-target-unresolved`). Transclusion depth/cycles reuse the
[[SPEC-048-components-and-static-overrides#REQ-4807]] bound, so a transclude chain is
also an expansion-bomb mitigation ([[SPEC-048-components-and-static-overrides#Threat B]]).

> Deferred threats: **Threat A** (script injection via a content directive) and
> **Threat C** (CSS scope escape) move with their capabilities to **SPEC-049** and
> **SPEC-051** respectively. v1 has neither an untrusted content-author surface nor a
> selector-scoping pass, so neither threat is reachable here.

---

## 9. Test Specifications

> Each TEST decomposes its REQ into positive / negative-input / negative-output per
> [[PROTO-001]] §Requirement-Targeted Test Decomposition. This is an AI-synthesised
> spec → adversarial testing is **mandatory** before convergence.

### TEST-4801: Context Tier Split + Expansion Latency
**Validates:** [[SPEC-048-components-and-static-overrides#REQ-4801]],
[[SPEC-048-components-and-static-overrides#NFR-4801]]. Positive: a page render exposes
`vault`/`page` as today and the path's tier set is the statically expected one.
Negative-input: a render path with no page rejects page-tier reads (strict-undefined).
Negative-output: latency budget breach on the 20-instance fixture fails the NFR gate.

### TEST-4802: Depth-Correct Link Base
**Validates:** [[SPEC-048-components-and-static-overrides#REQ-4802]]. Positive: a
nav-header on a page at depth 2 emits `../../foo/`; in serve mode emits `/foo/`.
Negative-input: a component hardcoding `/foo` is flagged by a lint. Negative-output: a
static page at depth N must not emit a link that 404s on `file://` (golden link-base
matrix).

### TEST-4803: Component Definition + Manifest
**Validates:** [[SPEC-048-components-and-static-overrides#REQ-4803]],
[[SPEC-048-components-and-static-overrides#CON-4801]]. Positive: a well-formed
`components/callout/` loads. Negative-input: missing template / missing manifest /
name mismatch / non-kebab name → `component-malformed`. Negative-output: a manifest
with an unknown top-level key (incl. the reserved `publishes`/`subscribes`) is
rejected (no inert tolerance).

### TEST-4804: Three-Tier Resolution + Override Visibility
**Validates:** [[SPEC-048-components-and-static-overrides#REQ-4804]],
[[SPEC-048-components-and-static-overrides#Threat D]]. Positive: vault component
shadows theme; resolved whole-directory. Negative-input: unknown `<name>` at an
invocation → `component-not-found`. Negative-output: an override is logged, never
silent.

### TEST-4805: Invocation + Slots (Macro Substrate + Sugar Lowering)
**Validates:** [[SPEC-048-components-and-static-overrides#REQ-4805]],
[[SPEC-048-components-and-static-overrides#CON-4805]]. Positive: both the native
`{% call %}` form and the lowered `{% component %}`/`{% slot %}` sugar render the same
golden HTML (default slot via `caller()`, a named slot via `{% set %}` capture).
Negative-input: a body for a component declaring no slots warns. Negative-output: the
scope marker `data-z` is present exactly once on the component root; the lowering pass
introduces no minijinja custom tag (asserted by rendering through stock minijinja).

### TEST-4807: Bounded Acyclic Nesting
**Validates:** [[SPEC-048-components-and-static-overrides#REQ-4807]],
[[SPEC-048-components-and-static-overrides#NFR-4803]],
[[SPEC-048-components-and-static-overrides#Threat B]]. Positive: a 3-level nest
renders. Negative-input: an a→b→a cycle → `component-cycle` at compile, naming the
path. Negative-output: a depth-17 fixture → `component-depth-bound`, build fails under
`--strict`, no hang/OOM (bounded wall-clock asserted).

### TEST-4808: Context-Requirement Verification
**Validates:** [[SPEC-048-components-and-static-overrides#REQ-4808]],
[[SPEC-048-components-and-static-overrides#Threat G]]. Positive: `requires=["site"]`
nav-header renders on both a content page and a static page. Negative-input (compile):
a `requires=["page"]` component invoked from a static page →
`component-context-unavailable` naming `page`. Negative-output (render): a component
reading an undeclared/absent tier variable → strict-undefined failure on the exercised
path.

### TEST-4809: Deduped, Deterministic Component CSS
**Validates:** [[SPEC-048-components-and-static-overrides#REQ-4809]],
[[SPEC-048-components-and-static-overrides#NFR-4802]]. Positive: a component used 5×
emits its CSS once, linked by the page; the component root carries `data-z`.
Negative-input: a missing `<name>.css` emits nothing. Negative-output: two builds are
byte-identical (stable order: name, then source layer).

### TEST-4811: Templated Static Override Pages
**Validates:** [[SPEC-048-components-and-static-overrides#REQ-4811]],
[[SPEC-048-components-and-static-overrides#ADR-4805]]. Positive: `about.html.jinja`
renders with site context + components → `about/index.html`, sharing the nav and
tokens with themed pages. Negative-input: a plain `.html` static file is copied
verbatim (no render). Negative-output: a static page accessing page context →
compile error (REQ-4808 first layer).

### TEST-4812: Token Single-Source + Merge
**Validates:** [[SPEC-048-components-and-static-overrides#REQ-4812]],
[[SPEC-048-components-and-static-overrides#CON-4804]],
[[SPEC-048-components-and-static-overrides#Threat F]]. Positive: `tokens.toml` → one
`tokens.css`; a vault override changes one key only. Negative-input: an unsafe value
(`;`/`}`/comment) → `tokens-value-unsafe`. Negative-output: the anuna-web regression —
`--moss` resolves to exactly one value across both surfaces.

### TEST-4813: Backward-Compatible Default
**Validates:** [[SPEC-048-components-and-static-overrides#REQ-4813]]. Positive: a
no-component vault builds byte-identically to the pre-SPEC-048 baseline (golden
snapshot). Negative-input: no `tokens.css` emitted when no `tokens.toml` exists.
Negative-output: no lowering pass or component machinery alters an unused build.

### TEST-4814: Prop Validation
**Validates:** [[SPEC-048-components-and-static-overrides#REQ-4814]],
[[SPEC-048-components-and-static-overrides#Threat E]]. Positive: valid props bind;
defaults fill. Negative-input: unknown prop / type mismatch / enum miss / missing
required → distinct errors. Negative-output: a `" onload=` prop value is escaped in
the rendered attribute, not active.

### TEST-4818: Addressed Vault Transclusion
**Validates:** [[SPEC-048-components-and-static-overrides#REQ-4818]],
[[SPEC-048-components-and-static-overrides#CON-4806]],
[[SPEC-048-components-and-static-overrides#Threat H]]. Positive: `about.html.jinja`
calls `transclude("handbook#mission")` and renders that section's HTML, identical to
the `![[handbook#mission]]` embed on a themed page (shared-resolver golden);
`transclude("handbook").title` exposes the title. Negative-input: a non-existent page,
an unresolved `#heading`, or a draft/unpublished target → `transclude-target-unresolved`
+ a `zetl check --dead-links` hit (no empty output); a transclude cycle / depth-17 chain
→ `component-depth-bound`. Negative-output: `transclude("handbook").backlinks` (and
`edges`, raw `frontmatter`) are NOT reachable — the closed allow-list rejects page-tier
fields (fuzz the `<target>` grammar and the field accessor — page-tier smuggling is
inert).

---

## 10. Observability

### OBS-4801: Component Render Stats
Per build, emit counts of component instances rendered (by name and source layer),
max nest depth reached, and a per-component "resolved from {vault|theme|default}" line
so overrides are visible ([[SPEC-048-components-and-static-overrides#Threat D]]).
**Trace:** [[SPEC-048-components-and-static-overrides#REQ-4804]], [[SPEC-048-components-and-static-overrides#REQ-4807]].

### OBS-4802: Asset Dedup Ratio
Emit the count of distinct component stylesheets emitted vs total invocations (the
dedup ratio), and a stable hash of `tokens.css`, to detect nondeterminism
regressions.
**Trace:** [[SPEC-048-components-and-static-overrides#NFR-4802]].

### OBS-4803: Bound Rejections
Emit counts of `component-depth-bound`, `component-cycle`, `tokens-value-unsafe`, and
`transclude-target-unresolved` rejections, so fail-closed events are auditable.
**Trace:** [[SPEC-048-components-and-static-overrides#NFR-4803]], [[SPEC-048-components-and-static-overrides#REQ-4807]], [[SPEC-048-components-and-static-overrides#REQ-4818]].

---

## 11. Composition-First Feasibility (Principle 15)

Per [[PROTO-001]] Phase 2, each new capability is checked against existing components
before specifying new surface:

| Capability | Existing primitive attempted | Outcome / placement |
| ---------- | ---------------------------- | ------------------- |
| Parameterised reusable fragment | minijinja `{% macro %}` + `{% call %}`/`caller()` + `{% import %}` (already used by `render_tree`) | **Compose** — a component *is* a macro; no new engine, no new tag ([[SPEC-048-components-and-static-overrides#ADR-4803]]) |
| `{% component %}`/`{% slot %}` ergonomics | source-lowering into `{% call %}` + `{% set %}` capture before parse | **Compose (thin pass)** — optional sugar; minijinja has no custom-tag API, so a pre-parse rewrite is the only honest route ([[SPEC-048-components-and-static-overrides#REQ-4805]]) |
| Resolution / override | three-tier theme fallback (`src/web/engine.rs`) | **Compose** — reuse verbatim ([[SPEC-048-components-and-static-overrides#REQ-4804]]) |
| Cycle detection | SPEC-032 composition topo-sort *algorithm* | **Compose** — same Kahn's sort applied to the component-invocation graph ([[SPEC-048-components-and-static-overrides#REQ-4807]]) |
| Nesting halt | minijinja recursion limit + explicit depth counter | **Compose/Extend** — bound at render ([[SPEC-048-components-and-static-overrides#REQ-4807]]) |
| Static asset layering | four-tier `copy_static_assets` | **Extend** — add a render pass for `.html.jinja`; verbatim copy unchanged ([[SPEC-048-components-and-static-overrides#REQ-4811]]) |
| Per-output link base | `compute_root_path` | **Extend** — lift into site context for static pages too ([[SPEC-048-components-and-static-overrides#REQ-4802]]) |
| Token compile + dedup component CSS emission | *(none)* | **New** — the only genuinely new *build* surface in v1; selector **scoping** is deferred to SPEC-051 ([[SPEC-048-components-and-static-overrides#REQ-4809]], [[SPEC-048-components-and-static-overrides#REQ-4812]]) |
| Pull named vault content into any page | existing [[Embed]] resolver (`![[page#section]]`) | **Compose/Extend** — expose the same resolver as a site-context `transclude()` function; add only the closed exposed-field allow-list ([[SPEC-048-components-and-static-overrides#REQ-4818]], [[SPEC-048-components-and-static-overrides#ADR-4810]]) |

Net new surface in v1 is confined to (a) the token compiler, (b) deduped component CSS
emission (collection only, no scoping), and (c) the `transclude()` allow-list wrapper
over the existing embed resolver. Everything else composes existing primitives — the
spec's central design claim, now grounded against minijinja's actual extension surface.

---

## 12. Open Questions

- **Q1 — Static render marker.** Is `.html.jinja` the only opt-in, or also a
  front-matter flag on a plain `.html`? (`[Blocked: Q1]`,
  [[SPEC-048-components-and-static-overrides#ADR-4805]]). Decision needs the Phase 1
  operator survey: how do operators currently name hand-authored pages?
- **Q3 — Relationship to `![[embed]]`.** *Resolved by
  [[SPEC-048-components-and-static-overrides#REQ-4818]] /
  [[SPEC-048-components-and-static-overrides#ADR-4810]]:* the embed resolver is reused —
  exposed to templates and static pages as a site-context `transclude()` function —
  rather than forking a second transclusion path. Whether the auto-generated
  transclusion *card* chrome should itself become a built-in `embed` component is a
  successor concern, not a v1 blocker.
- **Q8 — Transclusion syntax form (new in v0.2.1).** Should a `.html.jinja` static page
  also accept literal `![[page#section]]` markdown-style syntax, or *only* the explicit
  `{{ transclude("page#section") }}` function form? The function form is cleaner inside
  an HTML/jinja file and is the spec's default; confirm against how operators expect to
  author these pages in the Phase 1 survey.
- **Q5 — Cross-vault component packages.** Out of scope for v1, but the whole-directory
  resolution model is forward-compatible with a future package registry. Worth a
  successor spec? (`[Blocked: Q5]`).
- **Q6 — Sidebar vs top-strip unification.** Should the vault sidebar and the marketing
  top-strip be one variant-parameterised `nav-header`, or two components sharing
  `site.nav` data + tokens? §1.1 argues the markup legitimately differs; confirm with
  Phase 1.
- **Q7 — Unscoped-CSS collision convention (new in v0.2.0).** With scoping deferred to
  SPEC-051, v1 component CSS is emitted as authored. What naming convention (e.g.
  selectors prefixed with the component name) keeps two components' stylesheets from
  colliding in the interim, and should a lint warn on a component selector that does
  not mention its own name? Pin in IMPL-048.

> Resolved/relocated since v0.1.x: the content-HTML sanitiser policy (old Q2) and the
> theme-toggle/island-bus coordination (old Q4) move with their capabilities to
> **SPEC-049** and **SPEC-050**. The CSS side of theming (`[data-theme]` token blocks)
> stays in v1 (CON-4804); the *toggle behaviour* needs JS and is therefore SPEC-050.

---

## 13. Convergence Status

**NOT converged.** This is a tightened strawman with no adversarial pass. Before the
Phase 2 gate ([[PROTO-001]] §Success Criteria) this spec requires, at minimum:
(1) a fresh-context adversarial review ([[PROTO-001]] Principle 12) — expected to
press the macro-lowering feasibility (REQ-4805), the compile-vs-render split
(REQ-4808), and the token grammar (CON-4804/Threat F); (2) Phase 1 operator/author
profiles + happy-paths to ground every `[Provisional]` default and close Q1/Q6/Q7;
(3) a feasibility spike proving the `{% component %}`→`{% call %}` lowering (incl.
named slots via `{% set %}` capture) against the current `src/web/engine.rs`;
(4) IMPL-048 to pin the EBNF grammars, the numeric bounds, and the unscoped-CSS
convention. The successor specs (SPEC-049 directives, SPEC-050 islands+messaging,
SPEC-051 scoped CSS) are deliberately out of this document and gate independently.

---

## Changelog

<details>
<summary>Revision history — 0.1.0 → 0.2.1</summary>

- **0.2.1** (2026-06-24) — *normative; additive.* Added **addressed vault transclusion**
  so static (and any) pages can pull live, named wiki content: REQ-4818 (`transclude()`
  in site context, reusing the existing [[Embed]] resolver, closed exposed-field
  allow-list, visibility-gated, bounded by REQ-4807), CON-4806 (target grammar + exposed
  set), ADR-4810 (addressed read is a site capability, NOT a page tier — the distinction
  that keeps Threat G intact), Threat H (page-tier smuggling, closed by the allow-list),
  TEST-4818, HP6, OBS-4803 extension. **Resolves Q3** (embed relationship); adds Q8
  (transclusion syntax form: `transclude()` function vs literal `![[…]]`). Clarified
  Threat G as *ambient*-context leak, distinct from addressed transclusion.
- **0.2.0** (2026-06-24) — *normative; scope reduction + reframing.* **Tightened to the
  v1 core.** Reframed the central feasibility claim: components are minijinja **macros**
  (`{% macro %}` + `{% call %}`/`caller()` + `{% import %}`), optionally fronted by a
  thin source-lowering pass for the `{% component %}`/`{% slot %}` sugar — NOT a new
  custom statement tag (minijinja has no block-tag extension API), grounding the "no new
  engine" claim (ADR-4803). Split REQ-4808 into a compile-time tier check and a
  render-time strict-undefined check (v0.1.x conflated them). Reduced REQ-4809 to
  dedup + deterministic emission of **unscoped** component CSS (selector scoping needs a
  full parser → deferred). **Moved out** to named successors: content-author Markdown
  directives + sanitisation (REQ-4806/4815, CON-4803, ADR-4802, Threat A → SPEC-049);
  JS islands + inter-island bus + manifest topics (REQ-4810/4816/4817, ADR-4808 →
  SPEC-050); **scoped** CSS / selector rewriting (scoping half of REQ-4809, Threat C →
  SPEC-051). Added ADR-4809 (one generator, not "Jekyll + zetl"). Conformance pass to
  [[PROTO-001]] v1.11.0: added the Orientation block (Intent/Metaphor/ASCII Structure/
  Decisions/Load-bearing/Open/Detail), the BCP 14 conformance declaration, scalar-only
  frontmatter, and this `## Changelog` (history moved out of frontmatter). Fixed the
  0.1.0/0.1.1 version-field inconsistency. Retained REQ numbers; gaps (4806, 4810,
  4815–4817) point to successors.
- **0.1.1** (2026-06-24) — *normative.* Added inter-island messaging (REQ-4816/4817,
  ADR-4808). *(Relocated to SPEC-050 in 0.2.0.)*
- **0.1.0** (2026-06-24) — *initial strawman.* First draft from an exploration of
  web-component-like nesting + extending templating to static overrides.

</details>
