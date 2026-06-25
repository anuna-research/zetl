---
id: SPEC-049
title: "Content-Author Components & Directives"
status: draft
version: 0.1.0-strawman
last-updated: 2026-06-25
audience: agent, human
---

# SPEC-049: Content-Author Components & Directives

## Orientation

**Intent:** Let an **untrusted Markdown author** invoke a theme-allowlisted [[SPEC-048]]
component from inside their content — via directive syntax `:::name{…}` — and have the
untrusted attributes and body **recognised and sanitised** so a content author gains rich,
reusable components without being able to inject script, break out of the HTML/attribute
context, or read more of the vault than they should.

**Metaphor:** *a fill-in form, not a blank cheque.* The theme author publishes a small set of
**forms** (content-invocable components) with typed fields; a content author fills them in. The
form's structure (the template) is trusted and fixed; only the *filled-in values* and the
*body text* come from the untrusted author, and both are validated/sanitised at the counter
before anything is rendered.

**Structure** (`≤ 7` boxes; arrows = build-time data flow):

```
   untrusted Markdown                          trusted theme
  ┌────────────────────┐                  ┌───────────────────────┐
  │ :::callout{type=…}  │   parse          │ SPEC-048 component     │
  │   **body markdown** │ ───────►  Directive AST node            │
  │ :::                 │           (CON-4901)  + [props] schema   │
  └────────────────────┘                  │ + content_invocable    │
            │ transform-stage expansion (REQ-4906)  allowlist      │
            ▼                              └───────────────────────┘
  ┌─────────────────────────────────────────────────────────────┐
  │ content-invocable check (default-DENY, REQ-4903) → resolve →  │
  │ recognise props (untrusted, REQ-4904) → render template with  │
  │ SANITISED body + ESCAPED props → OUTPUT SANITISER (CON-4902)  │
  └─────────────────────────────────────────────────────────────┘
            ▼ sanitised static HTML (the no-JS fallback)
  ┌─────────────────────────────────────────────────────────────┐
  │ if the component also ships <name>.js → it is a content-      │
  │ author ISLAND; its runtime is governed by [[SPEC-050]]        │
  └─────────────────────────────────────────────────────────────┘
```

**Decisions** (deliberate before implementing):
[[SPEC-049-content-author-components#ADR-4901]] directives invoke SPEC-048 components (reuse the macro substrate, not a new component system) ·
[[SPEC-049-content-author-components#ADR-4902]] **default-deny** content-invocability — a component is invocable from content only when the theme opts it in ·
[[SPEC-049-content-author-components#ADR-4903]] sanitise at the boundary — trusted template + escaped props + sanitised body + a closed output-HTML recogniser ·
[[SPEC-049-content-author-components#ADR-4904]] generic-directive syntax (`:::`/`::`/`:`) after the remark-directive / CommonMark prior art, not a bespoke grammar ·
[[SPEC-049-content-author-components#ADR-4905]] directives expand at the **transform stage** over the [[SPEC-032]] AST (resolves the transform-vs-render seam) ·
[[SPEC-049-content-author-components#ADR-4906]] raw HTML in untrusted content is **sanitised, not passed through**.

**Load-bearing requirements:**
[[SPEC-049-content-author-components#REQ-4901]] directive recognition ·
[[SPEC-049-content-author-components#REQ-4903]] default-deny content-invocable allowlist ·
[[SPEC-049-content-author-components#REQ-4904]] untrusted prop recognition ·
[[SPEC-049-content-author-components#REQ-4905]] body-as-sanitised-Markdown + output sanitiser ·
[[SPEC-049-content-author-components#REQ-4906]] transform-stage expansion ·
[[SPEC-049-content-author-components#REQ-4907]] restricted content context (no page-tier/draft over-reach) ·
[[SPEC-049-content-author-components#REQ-4910]] island handoff to SPEC-050 ·
[[SPEC-049-content-author-components#REQ-4912]] backward-compatible default.

**Open** (each blocks the Phase 2 gate — see
[[SPEC-049-content-author-components#12. Open Questions]]):
Q1 sanitiser engine/policy baseline · Q2 inline-directive prop ergonomics · Q3 nesting-depth
bound · Q4 whether content components may `transclude()` at all · Q5 per-theme vs global
allowlist scope.

**Detail:** the full requirement, contract, and test nodes follow below.

> **Conformance.** The key words MUST, MUST NOT, REQUIRED, SHALL, SHALL NOT, SHOULD,
> SHOULD NOT, RECOMMENDED, MAY, and OPTIONAL in this document are to be interpreted as
> described in BCP 14 (RFC 2119 + RFC 8174) when, and only when, they appear in all capitals.

> **Strawman notice.** This is a first strawman with **no adversarial pass yet**. It carves the
> deferred content-directive material out of [[SPEC-048]] (former REQ-4806/4815, CON-4803,
> ADR-4802, Threat A) and must converge a **sanitiser policy** (SPEC-048's "old Q2") before any
> Phase 2 gate. Like [[SPEC-050]], the untrusted-surface security here needs fresh-context
> review + executable fuzzing + ideally a human security expert; do not treat it as settled.

| Field        | Value                                                                                  |
| ------------ | -------------------------------------------------------------------------------------- |
| Document ID  | [[SPEC-049-content-author-components\|SPEC-049]]                                        |
| Title        | Content-Author Components & Directives                                                  |
| Version      | 0.1.0-strawman                                                                          |
| Status       | Draft (strawman; NOT converged — no adversarial pass; sanitiser policy unsettled)      |
| Author       | Agent (Claude Opus 4.8 [1M], [[PROTO-001\|USDD Agent Protocol]])                        |
| Date         | 2026-06-25                                                                              |
| Audience     | Agent, Human                                                                            |
| Trace        | [[PROTO-001]] §Phase 1, §Phase 2, §LangSec, §AI Trust Boundaries                        |
| Source       | Deferred from [[SPEC-048]] (content-directive row, §Deferred Capabilities)              |
| Related      | [[SPEC-048]] component core, [[SPEC-050]] islands + messaging, [[SPEC-032]] AST/Hooks, [[SPEC-051]] scoped-CSS |
| Predecessor  | [[SPEC-048]] (the trusted component core this opens an untrusted authoring surface onto)|
| Feature Gate | `content-components`                                                                    |
| Review tier  | Tier 2 (a trust boundary: **untrusted** content-author Markdown crosses into an HTML context) |

---

## 1. Context

### 1.1 Why this spec exists

[[SPEC-048]] gave **theme authors** components: typed, slotted, reusable HTML fragments invoked
from **templates** (`{% component "callout" … %}`). That surface is **trusted** — template
authors ship the theme. SPEC-048 deliberately deferred the *untrusted* mirror of this: letting
a **content author** (anyone writing Markdown in the vault, including via `--collab`) invoke a
component from **within their prose**, because that crosses a real trust boundary and needs a
settled output-sanitiser policy (SPEC-048's "old Q2") and a decision on *where in the pipeline*
a directive expands. This spec is that mirror.

Concretely, an author writes:

```markdown
Here's a warning:

:::callout{type=warning title="Heads up"}
This is **content-authored** and may contain a [link](https://example.com).
:::
```

and gets the theme's `callout` component — *if* the theme allowed `callout` to be invoked from
content — with `type`/`title` validated as props and the body rendered as **sanitised** Markdown.

### 1.2 Core Insight

**A content-author component is a *trusted template* invoked with *untrusted inputs*.** The
template (theme-authored, SPEC-048) is safe by assumption; the danger is entirely at the
**inputs** — the directive's attributes and its body — and at the **output**. So the security
work is three precise cuts, none of which touches the template:

1. **Which templates are reachable at all** — *default-deny*. Exposing every theme component to
   untrusted authors would leak capabilities (a component that transcludes arbitrary pages, or
   reads page-tier context). A theme MUST explicitly mark a component **content-invocable**
   (REQ-4903). Everything else is unreachable from content.
2. **The attribute values** — recognised against the component's `[props]` schema (REQ-4904),
   strictly typed, and interpolated **escaped** (never concatenated raw), so a prop can never
   break out of an attribute or inject markup ([[SPEC-048]] Threat E, now *enforced* at an
   untrusted boundary rather than left to convention).
3. **The body and any raw HTML** — the body is the author's Markdown, rendered through the
   **content output sanitiser** (CON-4902): a *closed, default-deny HTML allowlist* that strips
   script, event handlers, dangerous URL schemes, and disallowed elements. Raw HTML in untrusted
   content is **sanitised, not passed through** (ADR-4906).

The second insight is a **pipeline** one: directives are recognised at **parse** (a typed
`Directive` AST node over the [[SPEC-032]] AST), expanded at the **transform stage** into a
SPEC-048 component invocation, and rendered with the sanitiser at **render** — which is the
clean answer to the "transform-stage-vs-render-stage seam" SPEC-048 flagged (ADR-4905).

### 1.3 Design Principles

1. **Reuse, don't reinvent.** A directive is sugar over a SPEC-048 component invocation; prop
   validation, slots, `data-z`, and cycle detection are SPEC-048's, reused
   ([[SPEC-049-content-author-components#ADR-4901]]).
2. **Default-deny at the trust boundary.** Untrusted authors reach only what the theme
   explicitly exposes ([[SPEC-049-content-author-components#ADR-4902]]).
3. **Full recognition of untrusted input.** Both the directive grammar and the output HTML are
   LangSec recognisers — closed grammars, parsed once, fail-closed ([[PROTO-001]] §LangSec).
4. **Static-first; interactivity is a separate, layered capability.** A content component's
   sanitised HTML is the complete, no-JS artifact; if it *also* ships an island, [[SPEC-050]]
   governs the runtime and the static HTML is the fallback (REQ-4910, [[SPEC-050]] REQ-5002).
5. **The author cannot widen their own authority.** Not invocability, not props beyond the
   schema, not network egress (that is [[SPEC-050]] REQ-5026, operator-owned), not vault reach
   beyond the published surface (REQ-4907).

### 1.4 Scope

**In scope:** the directive syntax (container/leaf/inline); content-invocable allowlisting;
untrusted prop + body recognition; the output HTML sanitiser policy; transform-stage expansion;
the restricted content context; the handoff to SPEC-050 for interactive content components;
backward compatibility.

**Out of scope:** the component *definition* model (SPEC-048); the island **runtime** — Worker
sandbox, bus, capability bridge, controlled-element renderer (all [[SPEC-050]]); selector-scoped
CSS ([[SPEC-051]]); a general Markdown extension framework beyond directives; server-side
collaborative authoring auth ([[SPEC-041]]/[[SPEC-042]]).

---

## 2. Happy Paths

### 2.1 HP1 — a content author uses an allowed component
A theme marks `callout` `content_invocable = true`. An author writes `:::callout{type=warning}
…body… :::`. The build recognises the directive, validates `type` against `callout`'s `[props]`
enum, renders the `callout` template with the body as a **sanitised** default slot, and emits
the component's static HTML (with `data-z="callout"`). JS-off, it is fully readable.

### 2.2 HP2 — a non-allowlisted directive is refused, not rendered
The author writes `:::raw-html{}<script>…</script>:::` for a component the theme did **not**
mark content-invocable (or that does not exist). The directive is recognised as unknown-for-content
and **fail-closed**: it is rendered as inert text / dropped with a diagnostic
(`content-directive-unknown`), never expanded, never executed.

### 2.3 HP3 — a hostile prop or body is neutralised
The author writes `:::callout{title="\" onload=\"alert(1)"}` with a body containing
`<img src=x onerror=alert(1)>`. The prop is escaped at interpolation (attribute breakout fails);
the body's raw HTML is sanitised (the `onerror` attribute and any non-allowlisted element are
stripped). The rendered output contains no executable content (Threat A closed).

### 2.4 HP4 — an interactive content component degrades gracefully
A content-invocable `poll` component ships `<name>.js`. Its directive renders the sanitised
static poll (results as plain HTML) as the no-JS fallback; with JS, [[SPEC-050]] mounts it as a
content-author **island** (Worker, controlled-element render, `content:`-namespaced topics). The
SPEC-049 expansion and the SPEC-050 island are two layers over the same component.

---

## 3. Requirements

> REQ numbering is REQ-49xx; gaps are permitted. Each requirement is atomic and testable.

### REQ-4901: Directive Recognition
The system SHALL recognise **generic directives** in content Markdown in three forms
([[SPEC-049-content-author-components#CON-4901]], after the remark-directive / CommonMark
prior art): **container** `:::name{attrs}` … `:::` (block, with a Markdown body), **leaf**
`::name{attrs}` (block, no body), and **inline** `:name[label]{attrs}` (in text). The `name`
SHALL be a kebab identifier; `{attrs}` is an attribute block (`#id`, `.class`, `key=value`,
`key="quoted"`, bare `key` = boolean true). A malformed directive SHALL fail closed
(`content-directive-malformed`) and render as inert text, never as raw HTML.

**Trace:** [[SPEC-049-content-author-components#TEST-4901]], [[SPEC-049-content-author-components#CON-4901]].

### REQ-4902: Directive → Component Resolution
A recognised directive SHALL resolve `name` to a [[SPEC-048]] component of the same name and
invoke it via the existing component-resolution + macro path ([[SPEC-048]] REQ-4805): `{attrs}`
keys map to the component's keyword props; the container body maps to the **default slot**; a
`#id`/`.class`/named-slot mapping is defined in CON-4901. Invocation reuses SPEC-048 slot
rendering and the `data-z="<name>"` root marker — directives add **no** new component machinery.

**Trace:** [[SPEC-049-content-author-components#TEST-4902]], [[SPEC-048]]; [[SPEC-049-content-author-components#ADR-4901]].

### REQ-4903: Default-Deny Content-Invocability
A [[SPEC-048]] component SHALL be invocable from content **only** when its manifest declares
**`content_invocable = true`** ([[SPEC-049-content-author-components#CON-4903]]). A directive
naming a component that does not exist, or exists but is **not** content-invocable, SHALL fail
closed (`content-directive-unknown`, build diagnostic) and render as inert text — never expand.
Trusted **template** invocation ([[SPEC-048]]) is unaffected; `content_invocable` gates only the
**content** path. The flag is **per component**, theme-author-set (trusted), never author-set.

**Trace:** [[SPEC-049-content-author-components#TEST-4903]], [[SPEC-049-content-author-components#CON-4903]]; [[SPEC-049-content-author-components#Threat B]].

### REQ-4904: Untrusted Prop Recognition
A directive's `{attrs}` SHALL be recognised against the component's `[props]` schema in the
**component-resolution layer** ([[SPEC-048]] REQ-4814), strictly: unknown prop →
`content-prop-unknown`; wrong type → `content-prop-type`; missing required (no default) →
`content-prop-missing`; out-of-`enum` → `content-prop-enum`. Because the source is **untrusted**,
each accepted value SHALL flow as a **typed, autoescaped** minijinja value interpolated `{{ … }}`
(never concatenated into raw markup or an unquoted attribute), and a **URL-typed** prop SHALL be
scheme-validated (CON-4902) — `javascript:`/`data:`/`vbscript:` rejected. v1 content props are
restricted to scalar/`enum` types (`string`/`bool`/`int`/`number`/`enum`); `list`/`map` props
are **not** content-settable in v1 (`content-prop-unsupported`).

**Trace:** [[SPEC-049-content-author-components#TEST-4904]], [[SPEC-048]]; [[SPEC-049-content-author-components#Threat A]], [[SPEC-049-content-author-components#Threat C]].

### REQ-4905: Body Sanitisation + Output Sanitiser
The container body is **untrusted Markdown**. It SHALL be rendered through the standard content
Markdown path and then the rendered HTML — body and the whole content-component expansion — SHALL
pass the **content output sanitiser** ([[SPEC-049-content-author-components#CON-4902]]): a
**closed, default-deny HTML allowlist** (elements, attributes per element, URL schemes) that
strips `<script>`/`<style>`/event handlers/`on*`/dangerous URL schemes/`<iframe>`/`<object>` and
any non-allowlisted node, leaving inert text where it drops. Raw HTML embedded in untrusted
content SHALL be sanitised, **never passed through** ([[SPEC-049-content-author-components#ADR-4906]]).
The sanitiser is a single full-recognition pass (no second, divergent parser), fail-closed.

**Trace:** [[SPEC-049-content-author-components#TEST-4905]], [[SPEC-049-content-author-components#CON-4902]]; [[SPEC-049-content-author-components#Threat A]], [[SPEC-049-content-author-components#Threat E]].

### REQ-4906: Transform-Stage Expansion (the seam)
Directive expansion SHALL occur at the **transform stage** over the [[SPEC-032]] AST: parse
recognises a directive into a typed **`Directive` AST node** (CON-4901); a transform pass
rewrites it into a SPEC-048 component-invocation node (REQ-4902); render emits the component and
runs the sanitiser (REQ-4905). This pins the order — **recognise (parse) → expand (transform) →
sanitise (render)** — so sanitisation runs on the *final rendered* HTML, not the source, closing
the transform-vs-render ambiguity SPEC-048 flagged.

**Trace:** [[SPEC-049-content-author-components#TEST-4906]], [[SPEC-032]]; [[SPEC-049-content-author-components#ADR-4905]].

### REQ-4907: Restricted Content Context (No Over-Reach)
A component rendered from the **content** path SHALL receive a **restricted context**: it SHALL
NOT receive page-tier fields (raw frontmatter, backlinks, edges) or any capability beyond what a
content author may already see. In particular, if the component uses `transclude()`
([[SPEC-048]] REQ-4818/CON-4806), the content path SHALL apply the **already-closed** page-tier
field set AND additionally forbid transcluding **draft/unpublished** targets from content
(fail-closed, `content-transclude-forbidden`). Whether content components may `transclude()` at
all is `[Blocked: Q4]`; the conservative v1 default is **no transclusion from the content path**.

**Trace:** [[SPEC-049-content-author-components#TEST-4907]], [[SPEC-048]]; [[SPEC-049-content-author-components#Threat F]].

### REQ-4908: Bounded, Acyclic Directive Expansion
Directive expansion SHALL be **bounded**: a content directive whose component invokes further
components SHALL reuse [[SPEC-048]] REQ-4807 cycle detection (Kahn topo-sort, [[SPEC-032]]
algorithm) and SHALL enforce a **maximum content-directive nesting depth** (`[Provisional]`,
Q3); a cycle or depth breach SHALL fail closed (`content-directive-cycle` /
`content-directive-too-deep`), never expand unboundedly. Directives SHALL NOT be recognised
inside a component's own template output (no recursive re-scan), so expansion terminates.

**Trace:** [[SPEC-049-content-author-components#TEST-4908]], [[SPEC-048]]; [[SPEC-049-content-author-components#Threat D]].

### REQ-4910: Interactive Content Component → SPEC-050 Island
A content-invocable component MAY additionally ship a client `<name>.js`. When it does, the
SPEC-049 directive expansion SHALL produce the **static, sanitised HTML** (the no-JS fallback,
[[SPEC-050]] REQ-5002), and the runtime SHALL be governed by [[SPEC-050]]: the component is a
**content-author island** running in an isolated realm (Worker default / sandboxed iframe), its
manifest island fields (`publishes`/`subscribes` `content:`-namespaced, `render`, `paints`,
`hydrate`, `[island.requests]`) and its capability bridge are [[SPEC-050]]'s, not this spec's.
SPEC-049 owns the static authoring + sanitisation; SPEC-050 owns the runtime. A component that
ships JS but is **not** `content_invocable` is unreachable from content (REQ-4903).

**Trace:** [[SPEC-049-content-author-components#TEST-4910]], [[SPEC-050]]; [[SPEC-049-content-author-components#ADR-4901]].

### REQ-4911: Author-Visible Diagnostics
Every content-directive failure (unknown/non-invocable component, malformed directive, prop
error, sanitiser drop, forbidden transclusion, cycle/depth) SHALL surface as a build-time
[[HookDiagnostic]] tied to the **source location in the author's Markdown**, and SHALL render as
**inert text** at that point (never silent disappearance, never raw HTML). Sanitiser drops
(stripped element/attribute counts) SHALL be summarised per page so an author can see what was
removed.

**Trace:** [[SPEC-049-content-author-components#TEST-4911]], [[SPEC-049-content-author-components#OBS-4901]].

### REQ-4912: Backward-Compatible Default
With the `content-components` feature gate **off**, `:::name{…}` sequences SHALL pass through as
today (literal text / existing Markdown behaviour) and output SHALL be **byte-identical** to a
build without this spec. Enabling the gate with **no** `content_invocable` component present
changes nothing (default-deny means no directive resolves). A vault that uses neither is
unaffected.

**Trace:** [[SPEC-049-content-author-components#TEST-4912]].

---

## 4. Contracts (LangSec)

### CON-4901: Directive Grammar + AST Node
**Interface:** the recogniser for content directives ([[SPEC-049-content-author-components#REQ-4901]]).
**Grammar** (after remark-directive / CommonMark generic directives):
```
container   = ":::" name attrs? NL  body  ":::" NL ;
leaf        = "::" name attrs? NL ;
inline      = ":" name "[" label "]" attrs? ;
name        = lower { lower | digit | "-" } (lower | digit) ;   (* kebab; ≥1 char; no trailing "-" *)
attrs       = "{" { attr } "}" ;
attr        = id-shorthand | class-shorthand | kv | flag ;
id-shorthand    = "#" ident ;          (* → prop `id` if declared, else dropped *)
class-shorthand = "." ident ;          (* → appended to prop `class` if declared, else dropped *)
kv          = key "=" ( bare | quoted ) ;
flag        = key ;                     (* boolean true *)
key         = lower { lower | digit | "-" } ;
quoted      = '"' { any-but-dquote | '\"' } '"' ;
bare        = { alnum | "-" | "_" | "." | ":" | "/" } ;   (* no spaces, no quotes, no < > *)
```
**Post-conditions:** a typed `Directive { form, name, attrs: map<key,value>, body? , pos }`
AST node ([[SPEC-032]] AST extension); `body` is the **unparsed** Markdown source span (parsed
later as content). **Pre-conditions:** ASCII-or-UTF-8 text input; `name`/`key` ASCII kebab; an
unterminated container or a `<`/`>` in a `bare` value → reject (`content-directive-malformed`).
**Error model:** malformed → `content-directive-malformed` (inert text + diagnostic).
**Implements:** [[SPEC-049-content-author-components#REQ-4901]], [[SPEC-049-content-author-components#REQ-4906]].
**Verified by:** [[SPEC-049-content-author-components#TEST-4901]].

### CON-4902: Content Output Sanitiser (Closed HTML Allowlist)
**Interface:** the server-side HTML sanitiser applied to content-component output and untrusted
body Markdown ([[SPEC-049-content-author-components#REQ-4905]]). This is the **settled sanitiser
policy** SPEC-048 deferred (its "old Q2"). It is a **closed, default-deny** recogniser — an
allowlist, never a denylist — parsing the *rendered HTML tree* once and rebuilding only
permitted nodes (the static-HTML sibling of [[SPEC-050]] CON-5007's worker renderer).
**Element allowlist** (the safe prose/structure set; everything else dropped to inert text):
text + `p h1..h6 ul ol li blockquote pre code em strong a img figure figcaption table thead
tbody tr th td hr br span div` (+ the component template's own elements, which are **trusted**
and pass through). **Hard-forbidden, non-overridable:** `script style iframe object embed form
input button template noscript svg math base meta link` + **all** `on*` event attributes, `style`,
`is`, `srcdoc`, `name`, `xlink:*`.
**URL schemes:** URL attributes (`href`/`src`/`srcset`/`poster`) → `https`/`http`/`mailto` and
relative only; `javascript:`/`data:`/`blob:`/`vbscript:`/`file:` rejected.
**Pre-conditions:** input is the rendered HTML of the untrusted body / content expansion; a
single parse (no second divergent parser); a poisoned/exotic node → dropped.
**Post-conditions:** an HTML subtree containing no script, event handler, dangerous URL, or
non-allowlisted element — **Threat A holds iff this contract holds** (stated plainly: an
incomplete or fail-open allowlist re-admits XSS).
**Error model:** a dropped node/attribute is counted and surfaced (OBS-4901); a missing/empty
allowlist is a **build error** (`sanitiser-policy-missing`), never fail-open.
**Engine/policy baseline** (e.g. an `ammonia`-class allowlist sanitiser) is `[Blocked: Q1]`.
**Implements:** [[SPEC-049-content-author-components#REQ-4905]].
**Verified by:** [[SPEC-049-content-author-components#TEST-4905]].

### CON-4903: Content-Invocable Manifest Field
**Interface:** the [[SPEC-048]] CON-4801 manifest, extended with the content-authoring gate.
**Grammar:**
```
content-invocable = "content_invocable" "=" bool ;   (* default false — REQ-4903 *)
content-slots     = "content_slots" "=" "[" { quoted-ident } "]" ;  (* OPTIONAL: named slots a
                     content author may fill; default = the default slot only *)
content-props     = "content_props" "=" "[" { quoted-ident } "]" ;  (* OPTIONAL: subset of [props]
                     settable from content; default = all scalar/enum props *)
```
**Pre-conditions:** `content_invocable`/`content_slots`/`content_props` are **theme-author**
(trusted) keys; a `content_slots`/`content_props` entry MUST name a declared slot/prop;
`content_invocable = true` requires the component's content-settable props to be scalar/`enum`
(REQ-4904). The SPEC-048-reserved `publishes`/`subscribes` (and the [[SPEC-050]] island fields)
remain owned by [[SPEC-050]] — this spec adds only the content-authoring gate.
**Post-conditions:** a per-component flag + optional narrowing of the content-settable
slot/prop surface, consumed by REQ-4903/4904 and the audit (OBS-4901).
**Error model:** `content_slots`/`content_props` naming an undeclared slot/prop →
`content-manifest-unknown-ref`; `content_invocable = true` on a component with a non-settable
required prop and no content-settable alternative → `content-invocable-unfulfillable` (build errors).
**Implements:** [[SPEC-049-content-author-components#REQ-4903]], [[SPEC-049-content-author-components#REQ-4904]].
**Verified by:** [[SPEC-049-content-author-components#TEST-4903]].

---

## 5. Non-Functional Requirements

### NFR-4901: Determinism
For a given (content, theme, options) tuple, directive recognition, expansion, and sanitiser
output SHALL be **byte-identical** across repeated builds (no map-iteration-order, no
wall-clock). Sanitiser node ordering is the input document order.
**Trace:** [[SPEC-049-content-author-components#TEST-4912]].

### NFR-4902: Bounded Work; No Framework
Content-directive processing SHALL add no runtime/browser cost (build-time only, like
[[SPEC-048]]); expansion is depth-bounded (REQ-4908); the sanitiser is a single linear pass over
the rendered tree. No client-side framework is introduced (interactivity, if any, is
[[SPEC-050]]'s islands).
**Trace:** [[SPEC-049-content-author-components#TEST-4908]].

---

## 6. Architecture Decision Records

### ADR-4901: Directives Invoke SPEC-048 Components (Reuse the Substrate)
A directive is **sugar over a SPEC-048 component invocation**, not a new component system. (+)
Prop validation (REQ-4814), slots, `data-z`, cycle detection (REQ-4807), and the island handoff
(SPEC-050) are all reused; one component model serves both trusted templates and untrusted
content. (−) The content surface is bounded to what components can express. Rejected: a separate
"content widget" system (duplicate machinery, two trust models to audit); inline HTML/MDX
(hands untrusted authors a far larger, harder-to-sanitise surface).

### ADR-4902: Default-Deny Content-Invocability
A component is reachable from content **only** when the theme opts it in (`content_invocable`).
(+) The untrusted surface is exactly what the theme chose to expose; a component that transcludes
or reads page context is not silently reachable; adding a component never widens the content
attack surface by accident. (−) Theme authors must annotate. Rejected: all-components-invocable
(capability leak — Threat B); a denylist (open-by-default is the wrong posture at a trust
boundary — [[PROTO-001]] §LangSec).

### ADR-4903: Sanitise at the Boundary (Trusted Template + Escaped Props + Sanitised Body + Output Recogniser)
Security lives at the **inputs and output**, never the template. Props are typed + autoescaped
(REQ-4904); the body is sanitised Markdown; the final rendered HTML passes the CON-4902 output
sanitiser as the authoritative recogniser. (+) The template author writes normally (trusted);
the untrusted parts are each recognised; defense-in-depth (escape *and* sanitise). (−) A second
pass over rendered HTML has a cost (build-time, bounded). Rejected: trusting the template to
sanitise its own slots (every component author would have to get it right — unsafe by default);
sanitising only the source Markdown (misses what the template interpolates and what rendering
produces — the seam SPEC-048 worried about).

### ADR-4904: Generic-Directive Syntax (Prior Art)
The `:::name{…}` / `::name{…}` / `:name[…]{…}` forms are the **remark-directive / CommonMark
generic-directive** vocabulary (also MyST, and close to pandoc fenced divs). (+) Familiar,
documented, tooling exists; not a bespoke grammar an author must learn. (−) Three forms to
recognise. Rejected: a zetl-specific shortcode syntax (`{{< … >}}`, `[shortcode]`) — reinvents a
standard and fragments the Markdown ecosystem; raw MDX/JSX (a huge untrusted surface, ADR-4901).

### ADR-4905: Transform-Stage Expansion over the SPEC-032 AST
Directives parse to a typed `Directive` node and expand in the **transform** stage to a component
invocation; sanitisation runs at **render** on the final HTML. (+) Pins the order
(recognise→expand→sanitise), composes with the [[SPEC-032]] hook pipeline, and sanitises the
*rendered* output (not the source) — the clean resolution of SPEC-048's transform-vs-render seam.
(−) Couples to the SPEC-032 AST shape. Rejected: a pre-parse string rewrite (cannot see Markdown
structure, fragile); sanitising the source before render (misses template-interpolated output).

### ADR-4906: Raw HTML in Untrusted Content Is Sanitised, Not Passed Through
Untrusted content Markdown's embedded raw HTML SHALL be run through CON-4902, not emitted
verbatim. (+) Closes the most direct injection path (Threat A) by construction at the content
boundary. (−) A content author cannot drop arbitrary HTML — by design. Rejected:
pass-through raw HTML for content (the classic stored-XSS hole); a per-author "trusted" opt-out
(there is no trusted content author in the `--collab` model — that is [[SPEC-041]]/[[SPEC-042]]'s
auth surface, not a sanitiser bypass).

---

## 7. Threat Model

Trust boundary: **theme/component authors are trusted** (they ship templates + the
`content_invocable` allowlist); **content-author Markdown — directive names, attributes, bodies,
and any embedded HTML — is untrusted input** crossing into an HTML context. (Threat letters
align with [[SPEC-048]] for cross-spec parity; this spec **owns Threat A**, which SPEC-048
deferred here.)

### Threat A: Script Injection via a Content Directive *(the deferred SPEC-048 threat)*
An author injects script via a directive body (`<script>`, `<img onerror>`), a prop
attribute-breakout (`title="\" onload=\"…`), or a dangerous URL (`href="javascript:…"`).
**Mitigation:** props typed + autoescaped + URL-scheme-validated (REQ-4904); body rendered then
run through the CON-4902 **closed output sanitiser**; raw HTML sanitised not passed through
(ADR-4906). **The guarantee is exactly as strong as CON-4902** — a fail-open or incomplete
allowlist re-admits XSS (stated, not assumed).

### Threat B: Capability Leak via an Over-Exposed Component
An author invokes a component that was not meant for content — one that transcludes arbitrary
pages, reads page-tier context, or performs a privileged action. **Mitigation:** default-deny
`content_invocable` (REQ-4903); the restricted content context (REQ-4907) strips page-tier
fields and forbids draft transclusion even for an allowlisted component.

### Threat C: Prop Type Confusion / Injection
An author supplies a prop of the wrong type/shape hoping the template mishandles it.
**Mitigation:** strict prop recognition against `[props]` (REQ-4904), scalar/`enum` only from
content, autoescaped interpolation ([[SPEC-048]] Threat E enforced at an untrusted boundary).

### Threat D: Directive / Expansion Bomb
Deeply nested or mutually-recursive content directives expand unboundedly (build-time DoS).
**Mitigation:** REQ-4908 — cycle detection (SPEC-048 REQ-4807) + a max nesting depth +
no recursive re-scan of template output; fail-closed.

### Threat E: Markdown / Sanitiser Bypass (mutation-XSS, namespace)
A crafted body survives the Markdown render and re-parses as markup in the browser (mXSS), or a
namespace/parser differential slips a node past the sanitiser. **Mitigation:** CON-4902 is a
single full-recognition pass over the rendered tree with a closed allowlist and a single HTML
namespace; the sanitiser parses the same way the browser will (no second divergent parser). This
threat is the reason the sanitiser engine/policy baseline (Q1) needs fuzzing + human review.

### Threat F: Transclusion / Context Over-Reach
A content component uses `transclude()` to read drafts, unpublished pages, or page-tier fields.
**Mitigation:** REQ-4907 — the content path applies the closed page-tier set ([[SPEC-048]]
CON-4806) **and** forbids draft/unpublished targets; v1 default is **no transclusion from
content** pending Q4.

---

## 8. Observability

### OBS-4901: Content-Directive Audit
Per build, the system SHALL record, for each page: the content directives expanded (component
name + source location), every refusal (`content-directive-unknown`/`-malformed`, prop errors,
forbidden transclusion, cycle/depth), and **sanitiser drops** (counts of stripped elements/
attributes by kind), so an author sees what was removed and an operator can audit which
components are reachable from content. This composes with [[SPEC-048]] OBS-4801 and (for
interactive content components) the [[SPEC-050]] island wiring graph (REQ-5009).
**Trace:** [[SPEC-049-content-author-components#REQ-4911]], [[SPEC-048]], [[SPEC-050]].

---

## 9. Tests

> Decomposed per [[PROTO-001]]: positive / negative-input / negative-output.

### TEST-4901: Directive Recognition
**Validates:** [[SPEC-049-content-author-components#REQ-4901]], [[SPEC-049-content-author-components#CON-4901]]. Positive: container/leaf/inline forms parse to a `Directive` node with the right `name`/`attrs`/`body`. Negative-input: an unterminated container, a `<` in a bare value, a trailing-`-` name → `content-directive-malformed` (inert text). Negative-output: a malformed directive never emits raw HTML; output is deterministic.

### TEST-4902: Directive → Component Resolution
**Validates:** [[SPEC-049-content-author-components#REQ-4902]], [[SPEC-048]]. Positive: `:::callout{type=warning}body:::` renders the `callout` component with `type=warning` and `body` as the default slot, `data-z="callout"` on the root. Negative-input: a `#id`/named-slot mapping to an undeclared slot → diagnostic. Negative-output: no new component machinery (reuses the SPEC-048 macro path — assert identical output to the equivalent template invocation, modulo sanitisation).

### TEST-4903: Default-Deny Content-Invocability
**Validates:** [[SPEC-049-content-author-components#REQ-4903]], [[SPEC-049-content-author-components#CON-4903]]. Positive: a `content_invocable = true` component expands from content. Negative-input: a directive naming a non-existent component, or an existing component **without** `content_invocable` → `content-directive-unknown`, rendered inert (never expanded). Negative-output: trusted template invocation of the same (non-invocable) component still works (the flag gates only the content path); `content_invocable` cannot be set from content.

### TEST-4904: Untrusted Prop Recognition
**Validates:** [[SPEC-049-content-author-components#REQ-4904]], [[SPEC-049-content-author-components#Threat A]], [[SPEC-049-content-author-components#Threat C]]. Positive: typed/enum props validate and interpolate escaped. Negative-input: unknown/wrong-type/missing-required/out-of-enum prop → the matching `content-prop-*` error; a `list`/`map` prop from content → `content-prop-unsupported`; a `javascript:` URL prop → rejected. Negative-output: a prop value `" onload="alert(1)` cannot break out of an attribute (assert escaped); no prop reaches raw markup.

### TEST-4905: Body Sanitisation + Output Sanitiser
**Validates:** [[SPEC-049-content-author-components#REQ-4905]], [[SPEC-049-content-author-components#CON-4902]], [[SPEC-049-content-author-components#Threat A]], [[SPEC-049-content-author-components#Threat E]]. Positive: a body with allowlisted Markdown/HTML renders intact. Negative-input — **vector matrix**: `<script>`, `<img onerror>`, `<a href="javascript:…">`, `<iframe>`, `<style>`, a `data:`/`blob:` URL, an `on*` handler, an mXSS/namespace probe, a poisoned node → each dropped to inert text, counted (OBS-4901). Negative-output: **no script/handler/dangerous-URL/non-allowlisted node survives** (Threat A holds *because* CON-4902 holds); a missing allowlist → `sanitiser-policy-missing` build error (fail-closed, never pass-through).

### TEST-4906: Transform-Stage Expansion
**Validates:** [[SPEC-049-content-author-components#REQ-4906]], [[SPEC-032]]. Positive: the order is parse(`Directive` node) → transform(component invocation) → render(sanitise); the sanitiser runs on the **rendered** HTML (assert a template-interpolated unsafe value is caught at render, not missed at source). Negative-output: changing only the transform stage does not bypass the render-stage sanitiser.

### TEST-4907: Restricted Content Context
**Validates:** [[SPEC-049-content-author-components#REQ-4907]], [[SPEC-049-content-author-components#Threat F]]. Positive: a content-rendered component sees only the allowlisted context. Negative-input: a content component attempting to read page-tier frontmatter/backlinks → absent; a `transclude()` of a draft/unpublished target from content → `content-transclude-forbidden`. Negative-output: no page-tier field or draft content leaks through the content path.

### TEST-4908: Bounded, Acyclic Expansion
**Validates:** [[SPEC-049-content-author-components#REQ-4908]], [[SPEC-049-content-author-components#Threat D]]. Positive: nested content components within the depth bound expand. Negative-input: a cycle → `content-directive-cycle`; over-depth nesting → `content-directive-too-deep`; a directive in a component's own template output is **not** re-scanned. Negative-output: expansion terminates; no unbounded build-time work.

### TEST-4910: Interactive Content Component → SPEC-050 Island
**Validates:** [[SPEC-049-content-author-components#REQ-4910]], [[SPEC-050]]. Positive: a `content_invocable` component with `<name>.js` emits the sanitised static HTML (no-JS fallback) and, with JS, mounts as a SPEC-050 content-author island (Worker, `content:` topics). Negative-input: a JS-bearing component **not** `content_invocable` is unreachable from content. Negative-output: the static fallback is complete and sanitised regardless of the island (REQ-5002).

### TEST-4911: Author-Visible Diagnostics
**Validates:** [[SPEC-049-content-author-components#REQ-4911]], [[SPEC-049-content-author-components#OBS-4901]]. Positive: each failure surfaces a `HookDiagnostic` at the author's source location and renders inert text there; sanitiser drops are summarised per page. Negative-output: no failure is silent and none emits raw HTML.

### TEST-4912: Backward-Compatible Default
**Validates:** [[SPEC-049-content-author-components#REQ-4912]], [[SPEC-049-content-author-components#NFR-4901]]. Positive: with the gate off, `:::name` is literal text and output is byte-identical to a no-SPEC-049 build. Negative-input: the gate on with no `content_invocable` component changes nothing. Negative-output: two builds are byte-identical (determinism).

---

## 10. Relationship to Adjacent Specs

- **[[SPEC-048]] (predecessor).** SPEC-049 opens an **untrusted authoring surface** onto the
  SPEC-048 component model: it reuses CON-4801 (manifest, extended by CON-4903), REQ-4805
  (invocation/macros), REQ-4814 (prop validation), REQ-4807 (cycle detection), REQ-4818/CON-4806
  (transclusion, further restricted), and the `data-z` marker. It carries the content-directive
  material SPEC-048 deferred (former REQ-4806/4815, CON-4803, ADR-4802, Threat A).
- **[[SPEC-050]] (runtime sibling).** A content-invocable component that ships JS is a
  **content-author island**; SPEC-049's sanitised expansion is the no-JS fallback, and SPEC-050
  governs the Worker sandbox, capability bridge, `content:`-namespaced topics, and
  controlled-element renderer (CON-5007). SPEC-049 = static authoring + sanitisation; SPEC-050 =
  runtime. Network egress is **not** a SPEC-049 surface (it is SPEC-050 REQ-5026/5027,
  operator-owned).
- **[[SPEC-051]] (later).** Scoped CSS will scope the `data-z`-marked component subtree,
  including content-invoked ones; orthogonal to this spec.
- **[[SPEC-032]] (substrate).** Directives are a transform-stage operation over the SPEC-032 AST.

---

## 11. Composition-First Feasibility (Principle 15)

| Capability | Existing primitive attempted | Outcome / placement |
| ---------- | ---------------------------- | ------------------- |
| Component invocation from content | [[SPEC-048]] REQ-4805 macro/call + REQ-4814 prop validation | **Compose** — a directive lowers to the existing invocation; no new component model (ADR-4901) |
| Cycle/nesting safety | [[SPEC-048]] REQ-4807 (Kahn topo-sort, SPEC-032 algorithm) | **Compose** — reuse for content-directive expansion (REQ-4908) |
| Transclusion (restricted) | [[SPEC-048]] REQ-4818/CON-4806 (closed page-tier set) | **Extend** — add draft-forbid + content-default-off (REQ-4907) |
| Interactivity | [[SPEC-050]] islands (Worker, bus, CON-5007 renderer) | **Compose** — interactive content components are SPEC-050 islands (REQ-4910) |
| Directive recognition | remark-directive / CommonMark generic-directive grammar | **Compose (prior art)** — adopt the standard forms, not a bespoke syntax (ADR-4904) |
| Output sanitisation | an allowlist HTML sanitiser (`ammonia`-class) | **New (bounded)** — the closed CON-4902 recogniser; the one genuinely new trust-boundary artifact (Q1) |

---

## 12. Open Questions

- **Q1 — Sanitiser engine + policy baseline.** Which sanitiser (an `ammonia`-class Rust
  allowlist sanitiser?) and what exact element/attribute/URL allowlist is the v1 default
  (CON-4902)? This is the **load-bearing** unknown (SPEC-048's "old Q2") and needs fuzzing +
  human security review before any Phase 2 gate. (`[Blocked: Q1]`.)
- **Q2 — Inline-directive prop ergonomics.** Is the `:name[label]{attrs}` inline form worth the
  recogniser complexity in v1, or should v1 ship container/leaf only and defer inline?
- **Q3 — Nesting-depth bound.** The exact max content-directive nesting depth (REQ-4908) — ground
  against real theme component trees in Phase 1.
- **Q4 — Transclusion from content.** Should content components be allowed to `transclude()` at
  all (REQ-4907)? v1 default is **no**; revisit if a real use case needs it, with the draft/
  page-tier restrictions enforced.
- **Q5 — Allowlist scope.** Is `content_invocable` strictly per-component, or should a theme be
  able to declare a content-component *namespace*/folder allowlist? Per-component (explicit) is
  the v1 default.

---

<details>
<summary>Changelog</summary>

<summary>Revision history — 0.1.0</summary>

- **0.1.0** (2026-06-25) — initial strawman. Carves the content-directive surface out of
  [[SPEC-048]] (former REQ-4806/4815, CON-4803, ADR-4802, Threat A) into its allocated successor
  number (SPEC-048 ADR-4807 plan). Directives (`:::`/`::`/`:`) invoke allowlisted SPEC-048
  components (ADR-4901, default-deny ADR-4902); untrusted props recognised + escaped (REQ-4904);
  body + output run through a closed allowlist sanitiser (CON-4902, ADR-4903/4906 — the deferred
  "old Q2" policy, still **unsettled**, Q1); transform-stage expansion over the SPEC-032 AST
  (ADR-4905); restricted content context (REQ-4907); interactive content components hand off to
  [[SPEC-050]] (REQ-4910). **NOT converged:** no adversarial pass; the sanitiser policy (Q1) is
  load-bearing and unfuzzed — the same human-expert + executable-fuzzing gate as SPEC-050 applies.

</details>
