---
id: SPEC-049
title: "Content-Author Components & Directives"
status: implemented
version: 0.7.0
last-updated: 2026-06-26
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
[[SPEC-049-content-author-components#ADR-4903]] make each untrusted input safe *in isolation, before composition* (sanitised-body + ingestion-validated/autoescaped props) ·
[[SPEC-049-content-author-components#CON-4904]] a **sound static HTML-context lint** (minijinja `unstable_machinery` AST) forbids any content prop/slot reaching a CSS/JS/URL/unquoted context ·
[[SPEC-049-content-author-components#ADR-4904]] generic-directive syntax (`:::`/`::`/`:`) after the remark-directive / CommonMark prior art, not a bespoke grammar ·
[[SPEC-049-content-author-components#ADR-4905]] directives expand at the **transform stage** over the [[SPEC-032]] AST (resolves the transform-vs-render seam) ·
[[SPEC-049-content-author-components#ADR-4906]] raw HTML in untrusted content is **sanitised, not passed through**.

**Load-bearing requirements:**
[[SPEC-049-content-author-components#REQ-4901]] directive recognition ·
[[SPEC-049-content-author-components#REQ-4903]] default-deny content-invocable allowlist ·
[[SPEC-049-content-author-components#REQ-4904]] untrusted prop recognition + static context lint (CON-4904) ·
[[SPEC-049-content-author-components#REQ-4905]] isolated body sanitisation + static slot-context lint ·
[[SPEC-049-content-author-components#REQ-4906]] transform-stage expansion ·
[[SPEC-049-content-author-components#REQ-4907]] restricted content context (no page-tier/draft over-reach) ·
[[SPEC-049-content-author-components#REQ-4910]] island handoff to SPEC-050 ·
[[SPEC-049-content-author-components#REQ-4912]] backward-compatible default.

**Resolved by the reference implementation** (see
[[SPEC-049-content-author-components#12. Open Questions]]):
Q1 sanitiser engine (`ammonia` 4 closed allowlist) · Q2 inline-directive form (deferred in v1) ·
Q6 prop/slot context enforcement (sound static lint, CON-4904).
**Remaining tunables** (non-blocking; the impl picked a defensible default each):
Q3 nesting-depth bound (impl: 16) · Q4 `transclude()` from content (impl: disallowed,
REQ-4907) · Q5 per-theme vs global allowlist scope (impl: per-component `content_invocable`).

**Detail:** the full requirement, contract, and test nodes follow below.

> **Conformance.** The key words MUST, MUST NOT, REQUIRED, SHALL, SHALL NOT, SHOULD,
> SHOULD NOT, RECOMMENDED, MAY, and OPTIONAL in this document are to be interpreted as
> described in BCP 14 (RFC 2119 + RFC 8174) when, and only when, they appear in all capitals.

> **Implementation status — implemented, boundary not yet declared converged.** The three
> pre-implementation design passes each relocated the security core rather than solving it
> (v0.2.0 isolated-body, v0.3.0 contract+tripwire, v0.5.0 static lint, v0.6.0 environment
> preconditions). A **reference implementation** (PR #65, behind `content-components`) has since
> discharged those concretely: the CON-4904 lint runs over the real minijinja `unstable_machinery`
> AST, the `ammonia` body sanitiser is fixed-point, and the suite includes LangSec property/fuzz
> tests. It then passed **five post-implementation review rounds** (2 fresh-context adversarial +
> 3 Codex) — **every round found a genuine boundary bug** (sanitiser/lint context gaps:
> `srcdoc`, namespaced URL attrs, URL-scheme obfuscation), each fixed with a regression test.
> That the *implementation* still surfaced five real bypasses is the same signal the design passes
> gave: this untrusted-HTML boundary is hardened but **not exhausted**. The features are default-on
> only because they are **byte-identical no-ops until a theme ships a `content_invocable` component**
> (REQ-4912); **production reliance still REQUIRES a dedicated human security expert + sustained
> executable fuzzing** before the boundary is treated as converged.

| Field        | Value                                                                                  |
| ------------ | -------------------------------------------------------------------------------------- |
| Document ID  | [[SPEC-049-content-author-components\|SPEC-049]]                                        |
| Title        | Content-Author Components & Directives                                                  |
| Version      | 0.7.0                                                                                   |
| Status       | Implemented (reference impl, PR #65, behind `content-components`; untrusted-content boundary still pending dedicated human security review + executable fuzzing) |
| Author       | Agent (Claude Opus 4.8 [1M], [[PROTO-001\|USDD Agent Protocol]])                        |
| Date         | 2026-06-26                                                                              |
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
**inputs** — the directive's attributes and its body. The security work is three precise cuts,
and — critically — **each untrusted input is made safe *in isolation, before it is composed
into the trusted template*, so provenance is preserved by construction** (the move that lets
[[SPEC-050]] CON-5007 work — the untrusted side is never serialised into a mixed string the
sanitiser must un-mix):

1. **Which templates are reachable at all** — *default-deny*. Exposing every theme component to
   untrusted authors would leak capabilities (a component that transcludes arbitrary pages, or
   reads page-tier context). A theme MUST explicitly mark a component **content-invocable**
   (REQ-4903). Everything else is unreachable from content.
2. **The attribute values (props)** — recognised against the component's `[props]` schema
   (REQ-4904) and made safe by **ingestion validation + HTML autoescape**, not by a downstream
   sanitiser. A `url`-typed prop is **scheme-validated when parsed** (context-independent, so safe
   wherever it lands); `string`/scalar props rely on minijinja HTML autoescape (sound in element
   text + double-quoted attribute); a **tainted-value tripwire** aborts a `|safe` raw-emit of an
   untrusted value. And the CSS/JS/unquoted/URL contexts autoescape *cannot* neutralise are the
   target of a **static HTML-context lint** (CON-4904) — the Go `html/template` technique over
   minijinja's `unstable_machinery` AST — which rejects at build any content prop **or sanitised
   slot** reaching an unsafe context, transitively, fail-closed. This is a **build-time guarantee
   *conditional on* enforced environment preconditions** (global Html-autoescape, no
   whitespace-trim, inheritance-complete taint — CON-4904(0)); a third review showed those
   preconditions are load-bearing and only an implementation + fuzzing can verify them, so context
   safety is **closeable, not yet closed** (Q6 still open; a restricted template language is the
   strong alternative).
3. **The body and any raw HTML** — the body is the author's Markdown, rendered and **sanitised
   *in isolation* (CON-4902) before being slotted into the trusted template**. The sanitiser
   sees only untrusted body HTML, never the composite, so it can be a closed default-deny
   allowlist without gutting trusted templates and without a provenance escape hatch. Raw HTML
   in untrusted content is **sanitised, not passed through** (ADR-4906).

The second insight is a **pipeline** one: directives are recognised at **parse** (a typed
`Directive` AST node — a [[SPEC-032]] schema amendment this spec depends on, CON-4901),
expanded at the **transform stage**, where the **body is rendered + sanitised in isolation**
and only then composed into the SPEC-048 component invocation (ADR-4905). This is the clean
answer to the "transform-stage-vs-render-stage seam" SPEC-048 flagged — and it deliberately
sanitises the **untrusted body alone**, never the mixed-provenance composite.

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
`<img src=x onerror=alert(1)>`. The prop is escaped *in its allowed (text/quoted-attr) context*
(attribute breakout fails; a URL/CSS/JS context would be a build error or require the `url`
ptype); the body is **sanitised in isolation** before it is slotted in (the `onerror` attribute
and any non-allowlisted element stripped). The output contains no executable content — *subject
to the sanitiser (Q1) and prop-linter actually converging* (Threat A; closure pending, not
asserted).

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

### REQ-4904: Untrusted Prop Recognition + Context-Correct Escaping
A directive's `{attrs}` SHALL be recognised against the component's `[props]` schema in the
**component-resolution layer** ([[SPEC-048]] REQ-4814), strictly: unknown prop →
`content-prop-unknown`; wrong type → `content-prop-type`; missing required (no default) →
`content-prop-missing`; an `enum`-**constrained** value outside its set → `content-prop-enum`.
(Note: `enum` is a **constraint** on a `string`/`int`/`number` base ptype, not a peer type —
[[SPEC-048]] CON-4801 — so a content prop's *type* is one of `string`/`bool`/`int`/`number`/
`url`; `list`/`map` are **not** content-settable in v1, `content-prop-unsupported`.)

**Prop safety, with the residual stated honestly.** A *static* "lint every interpolation site"
guarantee (an earlier draft's claim) **is buildable** — minijinja **does** expose its parser + AST
via the **`unstable_machinery`** feature (carrying *no semver guarantee*; the project does not
currently enable it). The earlier "minijinja has no template AST" framing was an over-reading of
[[SPEC-048]] REQ-4808's "no *public* AST" — the machinery exists. The real costs of the static
route are an **unstable-API dependency** (version-pin/maintenance risk) + a **hand-built
context-aware HTML classifier** (minijinja autoescape is context-blind, like Go `html/template`
solves). v1 **defers** that route to Q6 and ships a simpler baseline — **context-independent
ingestion validation + HTML autoescape + a tainted-value tripwire** — naming the one context the
baseline leaves to a trusted-author contract:

- **`url` ptype → validated at *ingestion* (CON-4902), context-independently.** When a `url`-typed
  prop is parsed from the directive, its scheme is canonicalised + allowlist-checked *then*
  (`https`/`http`/`mailto`/origin-relative; `//host`, `javascript:`, `data:`, `blob:`, `vbscript:`,
  `file:` rejected). Because the value is validated when ingested, it is safe **wherever** the
  template later places it — no interpolation-site analysis needed. A URL-context prop SHOULD be
  declared `url`-typed (CON-4903 adds the ptype to [[SPEC-048]] CON-4801).
- **`string`/`int`/`number`/`bool` props → HTML autoescape at interpolation.** minijinja's default
  HTML autoescape is **sound in both element-content and double-quoted-attribute contexts** (the
  only contexts v1 sanctions for these props), neutralising `<`/`>`/`"`/`&`.
- **Tainted-value tripwire.** A content-settable prop value is a distinct **tainted** `Value`;
  `{{ x | safe }}` / raw emission of a tainted value is a **render abort** (`content-unsafe-emit`),
  so a trusted template cannot accidentally bypass autoescape on untrusted data.
- **CSS/JS/unquoted-attribute contexts → statically forbidden (CON-4904).** Autoescape does not
  neutralise CSS-internal (`}`, `url(...)`) or JS-string injection, so v1 does not rely on it
  there: the **sound static HTML-context lint** ([[SPEC-049-content-author-components#CON-4904]])
  **rejects at build** any content prop reaching a `CSS`/`JS`/unquoted-attribute/un-validated-URL
  context — including transitively through a trusted sub-component, and fail-closed on any context
  it cannot prove. This **closes** the prior "documented residual": prop-context safety is now a
  build-time guarantee, not a trusted-author contract (the `|safe` tripwire is now also a static
  rejection). The cost — pinning minijinja's `unstable_machinery` AST + a context classifier — is
  accepted ([[SPEC-049-content-author-components#NFR-4903]]).

**Trace:** [[SPEC-049-content-author-components#TEST-4904]], [[SPEC-048]]; [[SPEC-049-content-author-components#Threat A]], [[SPEC-049-content-author-components#Threat C]].

### REQ-4905: Isolated Body Sanitisation
The container body is **untrusted Markdown**. It SHALL be rendered to HTML and then **sanitised
in isolation** ([[SPEC-049-content-author-components#CON-4902]]) — *before* it is slotted into
the trusted component template. The sanitiser SHALL see **only the untrusted body HTML**, never
the composite output that also contains trusted-template nodes; this preserves provenance by
construction (the sanitiser need not — and cannot — distinguish trusted from untrusted nodes in
a mixed string, so it never tries). CON-4902 is a **closed, default-deny HTML allowlist**
(elements, per-element attributes, URL schemes) that strips `<script>`/`<style>`/`on*`/dangerous
URL schemes/`<iframe>`/`<object>`/foreign-content and any non-allowlisted node (and comments/PIs),
leaving inert text where it drops, and SHALL be **serialise→reparse fixed-point stable**
(re-sanitising its own output is a no-op — the mutation-XSS guard). Raw HTML embedded in
untrusted content SHALL be sanitised, **never passed through**
([[SPEC-049-content-author-components#ADR-4906]]). The **trusted template is NOT sanitised** (it
may legitimately use `<form>`/`<svg>`/`<button>`/etc.); only the untrusted body slot is. Prop
safety is REQ-4904's concern, not this sanitiser's.

**Slot landing context → statically enforced (CON-4904).** The sanitised body is **safe HTML**,
correct **only** in element-content position (sanitised HTML in an attribute or `href` is the
wrong language — e.g. body text `javascript:alert(1)` slotted into `<a href="{{ slot }}">` is an
injection the body sanitiser never saw as a URL). v1 binds the sanitised body as a **tainted
SafeHtml** fragment whose **only** safe context is `TEXT`, and the **sound static lint**
([[SPEC-049-content-author-components#CON-4904]]) **rejects at build** any interpolation of the
slot into an attribute / URL / CSS / JS position (including transitively). This closes B-2's
slot-context hole as a build-time guarantee, the dual of REQ-4904's prop rule — not a
trusted-author contract.

**Trace:** [[SPEC-049-content-author-components#TEST-4905]], [[SPEC-049-content-author-components#CON-4902]], [[SPEC-049-content-author-components#CON-4904]]; [[SPEC-049-content-author-components#Threat A]], [[SPEC-049-content-author-components#Threat E]].

### REQ-4906: Transform-Stage Expansion (the seam)
Directive expansion SHALL occur at the **transform stage** over the [[SPEC-032]] AST. The order
is pinned as **recognise → sanitise-body-in-isolation → compose → render**:
1. **Parse** recognises a directive into a typed **`Directive` AST node** (CON-4901). Adding this
   node type is a **[[SPEC-032]] schema amendment this spec depends on** (the AST uses
   `deny_unknown_fields` typed node enums; a new node is a substrate change, not a footnote) —
   tracked as a SPEC-032 dependency.
2. **Transform — bottom-up, with a per-provenance sanitisation barrier (closes the nesting
   hole).** Expansion proceeds **innermost-directive-first**. For each directive: its **own
   untrusted body** — the Markdown/HTML text **excluding** any nested `Directive` node's *already-
   composed* output — is rendered and **sanitised in isolation** (REQ-4905/CON-4902); each nested
   directive has *already* been expanded to a **trusted, already-safe composed fragment** that is
   slotted in as an **opaque subtree the enclosing body sanitiser does NOT re-sanitise** (it is
   tagged trusted-composed). So a nested component's legitimate `<form>`/`<button>` is **never
   stripped** by an outer body pass, *and* every directive's own untrusted text **is** sanitised
   exactly once. (This is the one place provenance is genuinely needed — but it is a *clean*
   wholesale tag on composed fragments, not the impossible "un-mix a flat string" of the rejected
   composite-sanitiser model.)
3. **Compose**: the sanitised own-body + the trusted nested fragments are bound as the component's
   slot value, and the directive's props (REQ-4904) as keyword args; the SPEC-048 invocation
   ([[SPEC-048]] REQ-4805) renders the **trusted** template.
This sanitises each level's **untrusted text alone**, never a mixed-provenance string and never a
nested component's trusted output — closing the transform-vs-render ambiguity *and* the depth-≥2
relocation of it.

**Trace:** [[SPEC-049-content-author-components#TEST-4906]], [[SPEC-032]]; [[SPEC-049-content-author-components#ADR-4905]].

### REQ-4907: Restricted Content Context (Deny-by-Default Allowlist)
A component rendered from the **content** path SHALL receive a **deny-by-default, explicitly
enumerated** context — an **allowlist**, not "everything minus `transclude`" (a denylist at a
trust boundary is the wrong posture, [[SPEC-049-content-author-components#ADR-4902]]). The content
render context SHALL bind **only**: the validated `props` (REQ-4904), the sanitised slot
(REQ-4905), and a **named, draft-filtered** subset of safe site-tier helpers. It SHALL NOT bind
**anything else**, in particular: `transclude` ([[SPEC-048]] REQ-4818 — omitted entirely, so a
call is an undefined-global error for the directly-invoked component **and** any trusted
sub-component it reaches transitively); page-tier fields (raw frontmatter, backlinks, edges); and
any site-tier helper that exposes the **full vault graph** (e.g. an unfiltered `site.nav`, search,
or edge walk that would reveal draft/unlisted page titles or paths). Any site-tier helper the
content context *does* bind SHALL be **draft/visibility-filtered** at the binding. The bound set
is the spec's allowlist; IMPL-049 enumerates it exhaustively and a reviewer can diff it. (If Q4
later re-enables content transclusion, it MUST bind a *restricted* `transclude` applying the closed
page-tier set [[SPEC-048]] CON-4806 **and** rejecting draft targets — deferred with Q4.)

**Trace:** [[SPEC-049-content-author-components#TEST-4907]], [[SPEC-048]]; [[SPEC-049-content-author-components#Threat F]].

### REQ-4908: Bounded, Acyclic Directive Expansion
Directive expansion SHALL be **bounded**, with a single, well-defined re-entrancy model:
- **Author body Markdown is parsed exactly once.** A directive **nested inside another
  directive's body** is recognised as part of that **same single parse** of the author's content
  (legitimate nesting) — not via a second scan. The nesting **depth** so produced SHALL be
  bounded by a **maximum content-directive nesting depth** (`[Provisional]`, Q3);
  over-depth → `content-directive-too-deep` (fail closed).
- **Component template output is NEVER re-scanned for directives**, and a nested directive's
  **composed output is never re-sanitised** by an enclosing body pass (REQ-4906 per-provenance
  barrier). Once a directive expands, its rendered HTML is neither re-parsed for directives nor
  re-fed to the sanitiser — so expansion cannot recurse through templates, and trusted nested
  output is not stripped, only through the (single-parse, depth-bounded) author body.
- **Component invocation graph cycles** (a component invoking itself transitively) reuse
  [[SPEC-048]] REQ-4807 cycle detection (Kahn topo-sort, [[SPEC-032]] algorithm) →
  `content-directive-cycle`.
Together these guarantee termination: one bounded parse of author content + acyclic, non-
re-scanned template expansion.

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
**`content_invocable = true` is the machine-readable trigger for SPEC-050's *content-island*
tier** (vs the trusted in-realm tier): SPEC-050 distinguishes the tiers "by author trust"
([[SPEC-050]] REQ-5010) but the only build-time signal is this flag — so a JS-bearing
`content_invocable` component MUST be mounted under SPEC-050's content-island enforcement
(Worker sandbox, `content:`-only publish, CON-5007 renderer), and a JS-bearing non-invocable
component is a trusted in-realm island. **A11y-parity is achieved by *who renders the static
fallback*, not by intersecting allowlists** (CON-4902 is a fixed prose set that forbids
`<form>`/`<input>`/`<button>`; CON-5007 is a per-render theme set that *needs* them — their
intersection is uncomputable and would strip exactly the interactive controls a `poll` fallback
needs): the static fallback is produced by the **trusted component template**, which is **NOT**
sanitised (REQ-4905) and MAY legitimately contain `<form>`/`<button>`/etc.; only the **untrusted
author body slotted into it** is CON-4902-sanitised. So the no-JS fallback can carry the same
interactive elements the hydrated CON-5007 render shows — parity holds because both come from the
trusted template, and the author-body slot is the only sanitised part in either mode.

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
With the `content-components` feature gate **off**: `:::name{…}` sequences SHALL pass through as
today (literal text / existing Markdown behaviour); and the new manifest keys
(`content_invocable`/`content_props`, CON-4903) SHALL be **reserved —
accepted-and-ignored**, NOT rejected as unknown (this spec amends [[SPEC-048]] CON-4801's
unknown-key rule to reserve them, exactly as [[SPEC-050]] does for `publishes`/`subscribes`), so
a theme already annotated for content-invocability still builds under a gate-off (SPEC-048-only)
build. Output SHALL be **byte-identical** to a build without this spec **for any vault with no
content directive expanded** — i.e. gate off, OR gate on with no `content_invocable` component
(default-deny means no directive resolves). A vault using neither is unaffected.

**Trace:** [[SPEC-049-content-author-components#TEST-4912]], [[SPEC-049-content-author-components#CON-4903]].

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
**Recognition phase (load-bearing):** directives SHALL be recognised in the CommonMark **block
phase for `:::`/`::`, and the inline phase for `:name[…]`, AFTER code-fence and code-span
tokenisation has claimed its content** (as remark-directive does). A `:::` (or `:name[…]`)
**inside a fenced/indented code block or an inline code span is literal text, never a directive**
— so documentation that shows directive syntax in a code sample is not expanded. The flat
grammar above describes a recognised directive's *shape*; it is subordinate to this phase order.
**Post-conditions:** a typed `Directive { form, name, attrs: map<key,value>, body? , pos }`
AST node — **a [[SPEC-032]] schema amendment this spec depends on** (REQ-4906), not an assumed
node; `body` is the **unparsed** Markdown source span (parsed once, in the same parse — REQ-4908).
**Pre-conditions:** ASCII-or-UTF-8 text input; `name`/`key` ASCII kebab; a `quoted` value is
**single-line** (an unterminated quote / a newline inside a quote → `content-directive-malformed`,
never swallow-to-EOL); an unterminated container, a directive in a code context, or a `<`/`>` in a
`bare` value → not a directive / reject. **One ptype coercion recogniser:** a `bare`/`quoted`
lexeme is coerced to the prop's declared ptype by a single recogniser (an `int`/`number` lexeme
that is not a valid numeral → `content-prop-type`, fail-closed; no ambiguous `1.2.3`-as-number).
**`#id`/`.class` shorthand:** maps to a declared `id`/`class` prop; mapping to an **undeclared**
prop is a **diagnostic** (`content-prop-unknown`), **not** a silent drop — same failure mode as
an equivalent `key=` (consistency with REQ-4904; no silent loss per REQ-4911). **Note:** the
`bare` value grammar admits `:`/`/`, so a `bare` value MAY carry a `javascript:`/`data:` URI
string — the grammar is **NOT** relied on for scheme safety; URL safety is REQ-4904's `url`-ptype
**ingestion** validation (CON-4902), never the directive grammar.
**Error model:** malformed → `content-directive-malformed` (inert text + diagnostic).
**Implements:** [[SPEC-049-content-author-components#REQ-4901]], [[SPEC-049-content-author-components#REQ-4906]].
**Verified by:** [[SPEC-049-content-author-components#TEST-4901]].

### CON-4902: Content Output Sanitiser (Closed HTML Allowlist)
**Interface:** the server-side HTML sanitiser applied to the **untrusted body in isolation**
([[SPEC-049-content-author-components#REQ-4905]]) — *not* the composite output. This is the
sanitiser policy SPEC-048 deferred (its "old Q2"); it remains **load-bearing and unsettled**
(Q1). A **closed, default-deny** recogniser (allowlist, never denylist): parse the body HTML,
rebuild only permitted nodes. Because it sees only untrusted body (provenance by construction,
REQ-4905), it never faces the mixed-trusted/untrusted string that made an earlier draft unsound.
**Element allowlist** (safe prose/structure; everything else → inert text):
text + `p h1..h6 ul ol li blockquote pre code em strong a img figure figcaption table thead
tbody tr th td hr br span div`. **Hard-forbidden, non-overridable:** `script style iframe object
embed applet form input button template noscript svg math foreignObject base meta link` + **all**
`on*`, `style`, `is`, `srcdoc`, `name`, `xlink:*` + **HTML comments / processing instructions /
CDATA** (classic mXSS pivots).
**URL validation (scheme *allowlist* after canonicalisation, not a denylist).** A URL attribute
value (the **closed `URL_ATTR` set shared verbatim with CON-4904(1)**: `href`/`src`/`srcset`/
`poster`/`action`/`formaction`/`cite`/`data`/`background`/`ping`/`usemap`/`longdesc`) and a
`url`-typed prop (REQ-4904, validated at ingestion) SHALL be
**canonicalised** (lowercase scheme, strip leading/embedded control chars + whitespace —
`Java\tscript:` ≡ `javascript:`) then accepted **only** if it is `https`/`http`/`mailto` **or** a
relative reference that is **path-absolute or path-relative**. It SHALL be **rejected** if it is
**scheme-relative** (begins `//` or `\\` after canonicalisation → resolves off-origin) or any
non-allowlisted scheme (`javascript:`/`data:`/`blob:`/`vbscript:`/`file:`/…).
**Normative validation rule (parse, do not pattern-match — as implemented).** A URL value SHALL
be validated by **parsing** it with the WHATWG `url` crate and **assessing the parsed scheme**
against the allowlist (`http`/`https`/`mailto`) — **never** by pattern-matching the raw string:
*no URL is trusted unless it properly parses.* An **unparseable** value ⇒ **reject**. A
**protocol-relative** value — **any** slash mix (`//`, `\\`, `/\`, `\/`) ⇒ **reject** (it would
resolve off-origin). Relative URLs
resolve against the **page origin**; a content-invocable template (and the body) SHALL NOT emit a
`<base>` element (it would redirect every relative URL) — `<base>` is build-rejected in the body
(allowlist) and a content-invocable template emitting `<base>` is a build error. **`srcset`** (a
descriptor-delimited list) → parsed per its sub-grammar, each candidate URL validated as above;
unparseable → dropped.
**mXSS / parser-differential discipline:** the sanitiser SHALL emit a **canonical serialisation**
(stable/sorted attribute order, fixed quoting + entity policy, explicit void-element form) and be
**serialise→reparse fixed-point stable** — re-sanitising its own serialised output yields
byte-identical output (so a browser-vs-server reparse cannot reveal a hidden node, and so output
is deterministic per NFR-4901). A single HTML namespace; no second divergent parser. (This is the hardest part and the reason Q1 needs executable fuzzing + human
review before any Phase 2 gate.)
**Egress note:** an allowlisted body `<img src="https://…">`/`<a href>` causes the *reader's
browser* to fetch a remote URL (a tracking/exfil egress). Confining that is the **operator's
host-document CSP** (`img-src`/`connect-src`, [[SPEC-050]] REQ-5026/5027), the same lever as for
islands — CON-4902 validates *scheme*, not *destination*; the spec states this rather than
implying body HTML is egress-free.
**Pre-conditions:** input is the rendered HTML of the **untrusted body only**; a poisoned/exotic
node → dropped. **Post-conditions:** a body HTML subtree with no script, event handler, dangerous
URL, comment/PI, or non-allowlisted element; **AND the output is a well-balanced element-content
subtree that is context-neutral in `TEXT` position** — i.e. slotting it into a template's
element-content position cannot shift the surrounding HTML context for following literal text
(every open tag is closed, no dangling quote/`<`). This balance post-condition is the lemma
**CON-4904(2) relies on** to permit the sanitised slot in `TEXT`. **Threat A's body path holds iff
this contract holds AND the prop/slot context path (REQ-4904/CON-4904 + its (0) preconditions)
holds** (an incomplete/fail-open allowlist re-admits XSS — closure is *pending* Q1 + fuzzing).
**Error model:** a dropped node/attribute is counted (OBS-4901); a missing/empty allowlist is a
**build error** (`sanitiser-policy-missing`), never fail-open.
**Engine/policy baseline** (a named `ammonia`-class allowlist sanitiser + the fixed-point
reparse) is `[Blocked: Q1]`.
**Implements:** [[SPEC-049-content-author-components#REQ-4905]].
**Verified by:** [[SPEC-049-content-author-components#TEST-4905]].

### CON-4903: Content-Invocable Manifest Fields + the `url` ptype
**Interface:** two amendments to the [[SPEC-048]] CON-4801 manifest, owned here.
**(a) A new `url` ptype.** CON-4801's `ptype` is extended with **`url`** (a `string` whose value
is scheme-validated per CON-4902 wherever used in a URL context). A `url` value is validated by
**parsing** (the WHATWG `url` crate) and **assessing the parsed scheme** against the allowlist
(`http`/`https`/`mailto`), **not** by pattern-matching the raw string — *no URL is trusted unless
it properly parses*; an unparseable value ⇒ reject; a protocol-relative value (any slash mix
`//`, `\\`, `/\`, `\/`) ⇒ reject (CON-4902). This closes REQ-4904's
URL-context prop path; `enum` remains a **constraint** (`enum = [...]`) on a base ptype, not a
ptype.
**(b) The content-authoring gate:**
```
content-invocable = "content_invocable" "=" bool ;   (* default false — REQ-4903 *)
content-props     = "content_props" "=" "[" { quoted-ident } "]" ;  (* the EXACT props settable
                     from content; default = [] (NONE — narrowest surface, m-2) *)
```
**v1 fills the *default slot only*.** Named content slots (`content_slots`) are **deferred**
(`[Blocked: Q8]`): they would re-open the slot-landing-context problem (REQ-4905/B-2) per named
slot and need an author syntax CON-4901 does not yet define. v1 content authors fill only the
container body → the default slot.
**Forward-compat (amends CON-4801's unknown-key rule):** `content_invocable`/`content_props`
SHALL be **reserved** — when the `content-components` gate is **off** they are
**accepted-and-ignored**, never `component-malformed`-rejected (mirrors how [[SPEC-050]] activates
the reserved `publishes`/`subscribes`). So a manifest annotated for content-invocability builds
under a SPEC-048-only build (REQ-4912).
**Pre-conditions:** these are **theme-author** (trusted) keys; a `content_props` entry MUST name a
declared prop; a content-settable prop MUST be `string`/`bool`/`int`/`number`/`url` (REQ-4904;
`list`/`map` not content-settable). The **default `content_props` is `[]` (none)** — the
narrowest surface; a component exposes a prop to content only by explicitly listing it (m-2: no
silent widening to "all scalars"). The SPEC-048-reserved `publishes`/`subscribes` and the
[[SPEC-050]] island fields remain owned by [[SPEC-050]].
**Post-conditions:** a per-component content-invocable flag + the explicit content-settable prop
set + the `url` ptype, consumed by REQ-4903/4904 and OBS-4901.
**Error model:** `content_props` naming an undeclared prop → `content-manifest-unknown-ref`;
`content_invocable = true` with a required prop that is neither content-settable nor defaulted →
`content-invocable-unfulfillable`; a tainted content value raw-emitted (`|safe`) → render abort
`content-unsafe-emit` (REQ-4904) — all fail-closed.
**Implements:** [[SPEC-049-content-author-components#REQ-4903]], [[SPEC-049-content-author-components#REQ-4904]], [[SPEC-049-content-author-components#REQ-4912]].
**Verified by:** [[SPEC-049-content-author-components#TEST-4903]].

### CON-4904: HTML-Context Classifier (Sound Static Prop/Slot-Context Lint)
**Interface:** a **build-time, sound** static check that no content-derived value (a
content-settable prop, or the sanitised slot) is interpolated into an unsafe HTML context in a
content-invocable component's template — closing the Q6 residual by adopting the **Go
`html/template` context-autoescaper technique** over minijinja's parser AST (reachable via the
`unstable_machinery` feature; [[SPEC-049-content-author-components#NFR-4903]] pins it). "Sound"
means **no false negatives**: it never *misses* an unsafe placement; it achieves this by
**failing closed** on anything it cannot prove safe (conservative — it *will* reject some safe
templates, the correct trade-off at a trust boundary; the ergonomic cost on real theme trees is
acknowledged, Q3/Q6).

**(0) Discharged environment preconditions (load-bearing — the soundness of (1)–(5) depends on
these being ENFORCED, not assumed).** The lint inspects a *template* AST, but several
HTML-context-relevant facts live in the **render environment**, not the AST, and MUST be pinned:
- **Autoescape MUST be `Html` over every tainted interpolation.** minijinja selects autoescape
  from the *environment* (default `None`; `Html` only for `.html`/`.xml` outputs), invisible to a
  macro's own AST — so a content macro imported into a `None`-escaped page would emit a prop raw.
  The build SHALL render content-invocable components and **every transitively-tainted callee**
  under `AutoEscape::Html`, and the lint SHALL treat any `{% autoescape false %}`/non-`Html`
  region over a tainted interpolation as `content-context-unsafe`. Without this, (2)'s "safe in
  TEXT/attr via autoescape" is false.
- **Whitespace control MUST NOT trim the literal spans the tokeniser reads.** `trim_blocks`/
  `lstrip_blocks`/`-%}` mutate `EmitRaw` text at token boundaries, so the reconstructed literal
  stream could diverge from the rendered bytes. The build SHALL forbid whitespace-trimming config
  for content-invocable templates (or run the tokeniser over the *post-trim* emitted bytes and
  prove equivalence); v1 forbids it.
- **AST coverage MUST be exhaustive.** The lint SHALL `match` minijinja AST node variants with
  **no `_` catch-all** (a minijinja upgrade adding a `Stmt`/`Expr` variant then fails to
  *compile*, not silently passes), and SHALL pin the minijinja **feature set** (variants are
  cfg-gated) — NFR-4903.

**(1) HTML context state machine.** The check SHALL run an HTML/CSS/JS **tokeniser state machine**
over the template's literal text spans (the ordered `EmitRaw` slices interleaved with `EmitExpr`
nodes from `machinery::parse`), tracking the context at every interpolation. Contexts: `TEXT` ·
`RCDATA` (`<title>`/`<textarea>`) · `TAG`/`ATTR_NAME` · `ATTR_VALUE_DQ`/`_SQ`/`_UQ` · `URL_ATTR` ·
`CSS` (inside `<style>`/`style=`) · `JS` (inside `<script>`/`on*=`) · `COMMENT`. The **`URL_ATTR`
trigger set is a closed, exhaustive list pinned ONCE and shared verbatim with CON-4902** (no
open-ended "…"): `href`, `src`, `srcset`, `poster`, `action`, `formaction`, `cite`, `data`,
`background`, `ping`, `usemap`, `longdesc`; an attribute not classified is treated as `URL_ATTR`
**only if** on the list, else its value position is `ATTR_VALUE_*` — and a *tainted* value in an
attribute the lint cannot positively classify → `content-context-indeterminate` (fail closed, not
silently "plain text"). An **interpolated element or attribute *name*** (`<{{ t }}>` /
`{{ a }}=`) → `content-template-unanalyzable` (the following context is unknowable). A literal HTML
that does not tokenise to a single well-defined context at an interpolation → `content-context-indeterminate`.

**Additional UNSAFE contexts for a content value (normative — each was a real review-round
finding, since fixed with a regression test).** The classifier SHALL treat the following as
unsafe for a content prop/slot value, *not* as a benign double-quoted attribute:
- **`srcdoc`** — the browser **entity-decodes `srcdoc` as a full HTML document**, so HTML
  autoescape does **not** neutralise its contents; a tainted value reaching a `srcdoc` attribute
  → `content-context-unsafe`. (`srcdoc` is also hard-forbidden in the body sanitiser, CON-4902.)
- **Namespaced URL attributes** (e.g. **`xlink:href`**) — URL attributes SHALL be matched **by
  their local name** (the part *after* the `:`), so `xlink:href` is classified as `URL_ATTR` (and
  thus a non-`url` tainted value in it → `content-context-unsafe`), exactly as bare `href` is. A
  namespace prefix MUST NOT let a URL attribute escape `URL_ATTR` classification.

**(2) Per-value-kind safe-context sets.** A *tainted* value (content prop / slot) is permitted
ONLY in:
- **`string`/`int`/`number`/`bool` content prop** → `TEXT`, `ATTR_VALUE_DQ` (HTML-autoescape is
  sound there). NOT `ATTR_VALUE_UQ`/`URL_ATTR`/`CSS`/`JS`/`COMMENT`/`ATTR_NAME`/`TAG`.
- **`url` content prop** → additionally `URL_ATTR` (its scheme was ingestion-validated, CON-4902).
- **sanitised slot (`caller()` / SafeHtml)** → `TEXT` **only** (sanitised *HTML* is wrong in any
  attribute — its internal `"` would break out of `ATTR_VALUE_DQ`).
Any tainted value in any other context → `content-context-unsafe` (build error). A tainted value
reached by `| safe` / `| raw` (escaper bypass) → `content-unsafe-emit` (build error) — the v0.3.0
tripwire is now a *static* rejection, not only a render abort.

**(3) Control-flow soundness (fixpoint).** Context SHALL be computed as a **join over the
template control-flow graph**: at an `{% if %}`/`{% elif %}` merge, the contexts of all branches
MUST be identical (else `content-context-indeterminate`); a `{% for %}` body MUST be
**context-loop-invariant** (the context at the body's end equals its start); `{% set x = expr %}`
**propagates taint** from `expr` to `x` (def-use), so `{% set u = props.href %}{{ u }}` is linted
as `props.href` in `u`'s context. An expression that *combines* a tainted subvalue with others
(`{{ 'https://' ~ props.host }}`) is tainted in its interpolation context (conservative).

**(4) Interprocedural taint — across ALL minijinja composition forms, not just `{% call %}`.**
Taint SHALL flow, recursively (acyclic, depth-bounded, same graph as REQ-4902/REQ-4908), across
**every** statically-resolvable construct that can carry a tainted value into another template
context: `{% call %}`/macro calls (arg→param); **template inheritance** `{% extends %}` +
`{% block %}` (a block's landing context is its **parent** template's — the lint MUST resolve the
override into the parent and taint the block by the parent's context, *not* analyse the child in
isolation); `{% import %}` / `{% from … import %}` (imported macro params); `{% filter %}` (a
filter applied to a tainted body — `| safe`/`| raw` here is `content-unsafe-emit`); `{% with %}` /
`{% set %}…{% endset %}` capture blocks (rebind taint under the new name); and `caller()` / slot
forwarding (a content slot handed to a sub-component is tainted in *that* component's context).
A tainted value reaching any of these via a target the lint **cannot statically resolve** →
`content-template-unanalyzable`. (Template inheritance is the dominant theme-composition path, so
omitting it — as an earlier draft did — would have made the "transitive" claim false.)

**(5) Analyzable-subset = a closed, fail-closed node-shape allowlist (this is what makes 1–4
decidable + sound).** A **content-invocable** template, and every template transitively reached
with tainted data, MUST consist **only** of an **enumerated allowlist of AST node shapes** the
lint handles — NOT "anything that doesn't defeat the tokeniser" (that would be circular and
undecidable). Concretely: statically-resolvable component/macro/`include`/`extends`/`import`
targets (no computed/dynamic names); **literal element AND attribute names** (no interpolated
tag/attr names, (1)); the control constructs (3)/(4) enumerate; and nothing else. **Any AST node
variant not on the allowlist → `content-template-unanalyzable`** (the exhaustive-match (0) makes
this the compiler-enforced default). The theme author rewrites such a template (trusted,
allowlisted, few). Soundness holds **only** on this subset — stated, not hidden.

**Pre-conditions:** (0)'s environment pins hold; the lint runs on content-invocable templates +
their tainted-reachable callees (incl. inherited parents); minijinja version + features pinned
(NFR-4903). **Post-conditions:** a proof that every content-derived value reaches only a safe
context **given the (0) preconditions and the subset**, or a fail-closed build error. **This
closes the prop/slot *context* path of Threat A at build time *conditional on* (0) — it is not an
unconditional "closed by design."** **Error model:** `content-context-unsafe` /
`content-context-indeterminate` / `content-unsafe-emit` / `content-template-unanalyzable` (all
build errors, fail-closed).
**Implements:** [[SPEC-049-content-author-components#REQ-4904]], [[SPEC-049-content-author-components#REQ-4905]].
**Verified by:** [[SPEC-049-content-author-components#TEST-4904]].

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

### NFR-4903: Pinned Unstable-Machinery Dependency
The CON-4904 context lint parses templates via minijinja's **`unstable_machinery`** API, which
carries **no semver guarantee** and whose AST variants are **cfg-gated by feature**. The build
SHALL: (a) **pin an exact minijinja version** (not a `^`/`~` range) **and an exact feature set**
while `unstable_machinery` is enabled; (b) make the lint **`match` AST variants exhaustively with
no `_` catch-all**, so a minijinja upgrade that adds a `Stmt`/`Expr` variant **fails to compile**
(not silently passes a construct the lint doesn't understand) — a *semantic* guard, stronger than
a shape snapshot; (c) additionally ship **AST-shape + whitespace-trim regression tests** (a
behaviour change in `EmitRaw` chunking or trim config must fail loudly, B-3); (d) gate the whole
feature on `content-components`. **This is a standing liability, not a fully-mitigated cost:** a
security-critical check on a no-semver-guarantee internal API is fragile, and the spec states so
plainly — which is exactly why the **dedicated restricted template language** (Q6 option b) is a
serious alternative, not just a fallback: it has *none* of CON-4904's environment-precondition
(autoescape/whitespace/inheritance/feature-set) exposure because the language is controlled.
**Trace:** [[SPEC-049-content-author-components#TEST-4904]], [[SPEC-049-content-author-components#CON-4904]].

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

### ADR-4903: Make Each Untrusted Input Safe *in Isolation*, Before Composition
Security lives at the **inputs**, never the template — and each untrusted input is made safe
**before** it touches the trusted template, so provenance is preserved by construction. **Props**
= context-correct escaping at interpolation + a restricted set of allowed contexts + the `url`
ptype (REQ-4904); **body** = rendered then **sanitised in isolation** (REQ-4905/CON-4902), then
slotted in. (+) The sanitiser sees only untrusted body, so it can be a closed allowlist without
gutting trusted templates *and* without a provenance escape hatch; prop safety is guaranteed by
escaping, not retroactively by a sanitiser that can't attribute bytes to a prop. (−) Two distinct
mechanisms to get right (a prop-context linter + a body sanitiser). **Rejected — and why it was
the original error:** *sanitising the final composite (rendered) HTML* — after render, trusted
and untrusted nodes are one indistinguishable string, so the sanitiser either strips legitimate
trusted-template elements (correctness break) or needs a heuristic pass-through that lets
untrusted body masquerade as template output (XSS hole). Also rejected: trusting the template to
sanitise its own slots (unsafe-by-default); sanitising only source Markdown (misses rendered
output). This is the static-HTML analogue of [[SPEC-050]] CON-5007's "untrusted side never
produces a mixed string" — now actually structural, not just borrowed phrasing.

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
An author injects script on **three paths**: the **body** (`<script>`, `<img onerror>`, an mXSS
`<svg>`); the **props** (attribute-breakout `title="\" onload=\"…`, or a dangerous value in a
URL/CSS/JS context); and the **slot landing site** — sanitised body *text* (e.g.
`javascript:alert(1)`) slotted by a trusted template into an attribute/URL context like
`<a href="{{ slot }}">` (the body sanitiser only validated it as element content, B-2). **Mitigation — three paths, all statically enforced:** (1) **body** rendered then **sanitised in
isolation** (REQ-4905/CON-4902, a closed fixed-point-stable allowlist; raw HTML sanitised not
passed through, ADR-4906); (2) **props** by **`url` ingestion validation + HTML autoescape**, with
every CSS/JS/unquoted/un-validated-URL placement **statically rejected** (CON-4904); (3) **slot
landing** **statically restricted** to element-content position (CON-4904) — both transitively,
fail-closed. **Closure rests on two static contracts, *conditional on CON-4904's (0) preconditions*:** it holds
**iff CON-4904 holds** — which itself requires the **enforced** environment pins (global
`Html`-autoescape over taint, no whitespace-trim, inheritance-complete taint, pinned feature set;
CON-4904(0)) — **and CON-4902 converges** (Q1). So the prop/slot *context* path is **closeable at
build time, but not yet *closed***: it is "closed **conditional on** (0) being discharged + the
analyzable-subset being expressible for real themes," which a third review flagged is **not the
same as "closed by design."** The remaining unsettled pieces are the (0) preconditions + the
body-sanitiser policy (Q1) — same honest posture as [[SPEC-050]] CON-5007; do not over-read. (Note: this is *injection*; **outbound egress** from an
allowlisted body `<img>`/`<a>` is the operator's CSP, not this threat — CON-4902.)

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
name + source location), every refusal (`content-directive-unknown`/`-malformed`, prop errors
incl. `content-unsafe-emit`, cycle/depth), and **sanitiser drops** as **kinds + counts** (e.g.
"3 `<script>` dropped, 1 `on*` attr dropped") so an author can see *what kind* was removed and an
operator can audit content reachability — but it SHALL **NOT echo the rejected markup/payload
verbatim** (that would be a sanitiser-probing oracle for the untrusted author). This composes
with [[SPEC-048]] OBS-4801 and (for interactive content components) the [[SPEC-050]] island
wiring graph (REQ-5009).
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

### TEST-4904: Untrusted Prop Recognition + Context Escaping
**Validates:** [[SPEC-049-content-author-components#REQ-4904]], [[SPEC-049-content-author-components#CON-4904]], [[SPEC-049-content-author-components#NFR-4903]], [[SPEC-049-content-author-components#Threat A]], [[SPEC-049-content-author-components#Threat C]]. Positive: a `url`-typed prop is scheme-validated at ingestion (safe anywhere); a scalar prop in `TEXT`/`ATTR_VALUE_DQ` HTML-autoescapes; a slot in `TEXT` renders. Negative-input (recognition): unknown/wrong-type/missing-required/out-of-`enum` → `content-prop-*`; `list`/`map` → `content-prop-unsupported`; a `url` value `javascript:`/`data:`/`//evil.com`/`Java\tscript:` → rejected at ingestion. **Negative-input (CON-4904 static lint) — the context matrix, each a build error:** a content prop in `style="…{{p}}…"` / `<style>` (`CSS`) / `on*="{{p}}"` / `<script>` (`JS`) / unquoted attr / a non-`url` prop in `href` → `content-context-unsafe`; the **slot** in any attribute (`<a href="{{slot}}">`) → `content-context-unsafe`; a prop passed to a **trusted sub-component** that places it in a URL/CSS context → caught **transitively**; `{{ props.x | safe }}` → `content-unsafe-emit`; a template whose context is branch-dependent or uses dynamic dispatch → `content-context-indeterminate`/`content-template-unanalyzable` (fail-closed). **Soundness:** no unsafe placement passes (the lint rejects what it cannot prove). **Precondition (0) cases (v0.6.0):** a content macro imported into a **`None`-autoescaped** page emits a tainted prop raw → `content-context-unsafe` (the autoescape-mode pin, B-1); a `{% block %}` whose **parent** template lands it in `<a href=…>` → caught via inheritance taint (B-2); a `trim_blocks`/`-%}` template for a content-invocable component → rejected (B-3). **NFR-4903:** a minijinja bump adding an AST variant **fails to compile** (exhaustive match, no `_`), not silently passes.

### TEST-4905: Isolated Body Sanitisation
**Validates:** [[SPEC-049-content-author-components#REQ-4905]], [[SPEC-049-content-author-components#CON-4902]], [[SPEC-049-content-author-components#Threat A]], [[SPEC-049-content-author-components#Threat E]]. **Provenance:** the sanitiser is invoked on the **body fragment alone**, before composition — assert a trusted template's legitimate `<form>`/`<svg>`/`<button>` is **NOT** stripped (it never reaches the sanitiser), while the same elements in the **body** ARE stripped. Positive: a body with allowlisted Markdown renders intact. Negative-input — **vector matrix**: `<script>`, `<img onerror>`, `<a href="javascript:">`, `<iframe>`, `<style>`, `data:`/`blob:` URL, `on*`, an HTML comment/PI, a `srcset` with a hidden `javascript:` descriptor, an mXSS/`<svg>`-foreign-content probe → each dropped to inert text, kind+count surfaced (OBS-4901, never the payload verbatim). **Fixed-point:** re-sanitising the sanitiser's own output is byte-identical (mXSS guard). Negative-output: no script/handler/dangerous-URL/comment/non-allowlisted node survives — **closure is conditional on CON-4902 converging (Q1)**; a missing allowlist → `sanitiser-policy-missing` (fail-closed).

### TEST-4906: Transform-Stage Expansion (Isolated-Body Order)
**Validates:** [[SPEC-049-content-author-components#REQ-4906]], [[SPEC-032]]. Positive: the order is parse(`Directive` node) → render+**sanitise body in isolation** → compose into the trusted template; assert the sanitiser input is the body fragment, never the composite. Negative-output: a trusted-template node is never sanitiser-stripped (B1 provenance); the `Directive` node is a declared SPEC-032 amendment (assert the AST schema admits it), not an ad-hoc field.

### TEST-4907: Restricted Content Context (Transitive)
**Validates:** [[SPEC-049-content-author-components#REQ-4907]], [[SPEC-049-content-author-components#Threat F]]. Positive: a content-rendered component sees only the allowlisted context; the `transclude` global is **absent**. Negative-input: a content component reading page-tier frontmatter/backlinks → absent; a content-invoked component (or a **trusted sub-component it reaches transitively**) calling `transclude(...)` → undefined-global render error, **no draft content leaks**. Negative-output: no page-tier field or draft content reaches the content path on any (direct or transitive) call.

### TEST-4908: Bounded, Acyclic Expansion
**Validates:** [[SPEC-049-content-author-components#REQ-4908]], [[SPEC-049-content-author-components#Threat D]]. Positive: nested content components within the depth bound expand. Negative-input: a cycle → `content-directive-cycle`; over-depth nesting → `content-directive-too-deep`; a directive in a component's own template output is **not** re-scanned. Negative-output: expansion terminates; no unbounded build-time work.

### TEST-4910: Interactive Content Component → SPEC-050 Island
**Validates:** [[SPEC-049-content-author-components#REQ-4910]], [[SPEC-050]]. Positive: a `content_invocable` component with `<name>.js` emits the sanitised static HTML (no-JS fallback) and, with JS, mounts as a SPEC-050 content-author island (Worker, `content:` topics). Negative-input: a JS-bearing component **not** `content_invocable` is unreachable from content. Negative-output: the static fallback is complete and sanitised regardless of the island (REQ-5002).

### TEST-4911: Author-Visible Diagnostics
**Validates:** [[SPEC-049-content-author-components#REQ-4911]], [[SPEC-049-content-author-components#OBS-4901]]. Positive: each failure surfaces a `HookDiagnostic` at the author's source location and renders inert text there; sanitiser drops are summarised per page. Negative-output: no failure is silent and none emits raw HTML.

### TEST-4912: Backward-Compatible Default
**Validates:** [[SPEC-049-content-author-components#REQ-4912]], [[SPEC-049-content-author-components#CON-4903]], [[SPEC-049-content-author-components#NFR-4901]]. Positive: with the gate off, `:::name` is literal text and output is byte-identical to a no-SPEC-049 build. **Reserved keys:** a manifest carrying `content_invocable`/`content_props` **builds** under a gate-off (SPEC-048-only) build (accepted-and-ignored, NOT `component-malformed`). Negative-input: the gate on with no `content_invocable` component changes nothing. Negative-output: two builds are byte-identical (determinism) for any vault with no directive expanded.

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

**Post-implementation honesty (v0.7.0).** The untrusted-content boundary saw **five independent
post-implementation review rounds** (2 fresh-context adversarial + 3 Codex), each of which found a
real, exploitable bypass at the untrusted-content/island trust boundary — all since fixed with a
regression test. Per this spec's own caution, production use still requires a **dedicated human
security review + executable fuzzing** of this boundary before it can be relied upon.

---

## 12. Open Questions

- **Q1 — Sanitiser engine + policy baseline. *Resolved (v0.7.0).*** The reference
  implementation uses **`ammonia` 4** as the closed-allowlist body sanitiser (CON-4902), with a
  **fixed-point post-condition** (re-sanitising the sanitiser's own serialised output is a no-op,
  the mXSS guard). *Original question:* Which sanitiser (an `ammonia`-class Rust
  allowlist sanitiser?) and what exact element/attribute/URL allowlist is the v1 default
  (CON-4902)? This is the **load-bearing** unknown (SPEC-048's "old Q2") and needs fuzzing +
  human security review before any Phase 2 gate. *(The implementation passed LangSec
  property/fuzz tests; a dedicated human security review of this boundary is still pending.)*
- **Q2 — Inline-directive prop ergonomics. *Resolved (v0.7.0).*** The inline
  `:name[label]{attrs}` form is **deferred in v1**: it is **recognised by the grammar but left
  literal** (it needs inline-level Markdown rendering *inside* a paragraph, which the block-segment
  expansion model cannot do without splitting the paragraph). *Original question:* Is the
  `:name[label]{attrs}` inline form worth the recogniser complexity in v1, or should v1 ship
  container/leaf only and defer inline?
- **Q3 — Nesting-depth bound.** The exact max content-directive nesting depth (REQ-4908) — ground
  against real theme component trees in Phase 1.
- **Q4 — Transclusion from content.** Should content components be allowed to `transclude()` at
  all (REQ-4907)? v1 default is **no**; revisit if a real use case needs it, with the draft/
  page-tier restrictions enforced.
- **Q5 — Allowlist scope.** Is `content_invocable` strictly per-component, or should a theme be
  able to declare a content-component *namespace*/folder allowlist? Per-component (explicit) is
  the v1 default.
- **Q6 — Prop/slot context enforcement. *Resolved (v0.7.0) — option (a).*** The reference
  implementation ships the **sound static HTML-context lint** (CON-4904) over the minijinja
  `unstable_machinery` template AST: it fails the build if a content prop/slot value can reach a
  JS / CSS / `on*` / unquoted-attribute / non-`url`-in-URL-attribute context, **transitively**
  (set / with / for / filter / call / macro taint propagation, **fail-closed** on anything
  unanalyzable). The (0) environment preconditions are enforced in the implementation. *Historical
  note (the question as it stood before resolution):* v0.5.0
  proposed option (a) — a static context lint ([[SPEC-049-content-author-components#CON-4904]]) —
  and v0.6.0 hardened it (discharged the autoescape/whitespace/inheritance/feature-set
  preconditions as *stated* requirements). But the third pass showed the lint's soundness now rests
  on a **growing list of environment preconditions** (CON-4904(0)) that prose can require but only
  an implementation + fuzzing can verify, and the analyzable-subset's ergonomics on real theme
  trees (template inheritance especially) are unproven. So Q6 weighs, more evenly than before:
  **(a)** the `unstable_machinery` context lint — viable but precondition-heavy and on a no-semver
  API (NFR-4903); **(b)** a **dedicated restricted template language** for content-invocable
  components — *more attractive now*, because it has none of (a)'s environment exposure (you control
  autoescape, whitespace, inheritance, parsing). The choice + the lint's own fuzzing is the
  human-expert gate; v1 SHOULD NOT ship content-invocable components until it lands. (Body-sanitiser
  policy remains Q1.) (`[Blocked: Q6]`.)
- **Q8 — Named content slots.** `content_slots` is deferred (CON-4903): it re-opens the
  slot-landing-context problem per named slot and needs an author syntax. v1 fills only the default
  slot (container body). Revisit if multi-slot content components are needed. (`[Blocked: Q8]`.)

---

<details>
<summary>Changelog</summary>

<summary>Revision history — 0.1.0 → 0.7.0</summary>

- **0.7.0** (2026-06-26) — *reference implementation landed (PR #65).* A complete reference
  implementation now exists on `feat/spec-049-050-content-islands`, behind the default-on
  `content-components` cargo feature, with **byte-identical backward-compatible defaults**; it
  passed clippy, ~2,535 lib tests, integration suites, and LangSec property/fuzz tests. **Resolved
  Q1** (`ammonia` 4 closed-allowlist body sanitiser with a fixed-point post-condition, CON-4902),
  **Q2** (inline `:name[label]{attrs}` deferred — recognised but left literal), and **Q6**
  (sound static HTML-context lint over the minijinja `unstable_machinery` AST, CON-4904). After
  **five post-implementation review rounds** (2 fresh-context adversarial + 3 Codex), each of
  which found a genuine security-boundary bug since fixed with a regression test: **hardened
  CON-4904** (`srcdoc` and namespaced URL attrs — `xlink:href`, matched by local name — added as
  unsafe contexts) and **CON-4902/4903** (parse-and-assess URL validation via the WHATWG `url`
  crate — no URL trusted unless it properly parses; unparseable ⇒ reject; protocol-relative, any
  slash mix, ⇒ reject). **Status → implemented**; the untrusted-content boundary still requires a
  dedicated **human security review + executable fuzzing** before production (see §7 Threat Model /
  §11 Composition-First).

- **0.6.0** (2026-06-25) — *third adversarial pass (Opus, fresh, source-grounded): 3 Blocking /
  3 Major / 3 Minor; verdict "CON-4904 RELOCATED — another relocation."* The reviewer verified
  against minijinja 2.16 source that the AST premise is real (`EmitRaw` preserves literal spans),
  but the "sound lint" rested on **three unstated environment preconditions**: **B-1** autoescape
  mode is environment-selected (`None` by default), invisible to the per-template AST → a content
  macro in a `None`-escaped page passed the lint and emitted a prop raw (a lint-approved XSS);
  **B-2** taint omitted **template inheritance** (`extends`/`block`/`import`/`filter`/`with`/`set`-
  block) — the dominant theme path; **B-3** whitespace control (`trim_blocks`/`-%}`) mutates the
  literal spans the tokeniser reads. Plus **B-4** "closed by design / Q6 resolved" overclaimed
  (the subset is a rewrite-or-reject author contract); **B-5** the subset must be a closed
  node-shape allowlist, not the circular "no construct defeats the tokeniser"; **B-6** the
  `URL_ATTR` set was a denylist-with-`…` drifting from CON-4902. Applied: CON-4904 gains a **(0)
  discharged-preconditions** block (enforced Html-autoescape over taint, no whitespace-trim,
  exhaustive no-`_` match + pinned feature set); taint (4) extended to all inheritance/import/
  filter/with/set-block/caller forms; subset (5) is now a closed fail-closed allowlist (literal
  element+attr names); the `URL_ATTR` list is closed + shared verbatim with CON-4902; CON-4902
  gains a **balance/context-neutral post-condition** (the lemma CON-4904 relies on for the slot);
  NFR-4903 hardened (compile-fail on new variants; standing-liability stated). **Threat A
  downgraded** to "closeable *conditional on* (0)", **Q6 re-opened (NOT resolved)** with a
  **restricted template language** now an equal alternative to the precondition-heavy lint. **The
  recurring 3-pass relocation is the signal:** v1 SHOULD NOT ship content-invocable components
  until Q1+Q6 land via a running PoC + executable fuzzing + a human security expert.

- **0.5.0** (2026-06-25) — *closes the prop/slot context residual with a sound static check
  (resolves Q6, option a).* Now that v0.4.0 established minijinja **does** expose a parser+AST
  (`unstable_machinery`), this makes the prop/slot context safety a **build-time guarantee** rather
  than a trusted-author contract. New **CON-4904 + NFR-4903 + TEST-4904**: a **sound HTML-context
  lint** (the Go `html/template` context-autoescaper technique) over the minijinja AST that, on a
  **statically-analyzable subset** of content-invocable templates, **rejects at build** any content
  prop or sanitised slot reaching a `CSS`/`JS`/unquoted-attribute/un-validated-`URL` context —
  including **transitively** through trusted sub-components, and **fail-closed** on any context it
  cannot prove (control-flow fixpoint: branches must agree, loops must be context-invariant; taint
  flows through `{% set %}`/`{% call %}`). "Sound" = no false negatives (no missed unsafe
  placement), at the cost of conservative false positives. The unstable-machinery dependency is
  **pinned + AST-shape-regression-tested** (NFR-4903), with a dedicated restricted template language
  as the documented fallback. REQ-4904/4905/§1.2/Threat A re-grounded: prop/slot *context* is
  closed by design; the only remaining unsettled piece is the **body-sanitiser policy (Q1)**. The
  v1 baseline (ingestion-validation + autoescape) is retained beneath the lint as defense-in-depth.

- **0.4.0** (2026-06-25) — *factual correction: "minijinja has no template AST" was wrong.*
  Verified against the minijinja 2.16.0 source: the `unstable_machinery` feature **does** expose
  `machinery::parse` + `machinery::ast` ("an unstable internal API (no semver guarantees)"; the
  project doesn't enable it). The v0.2.0/v0.3.0 reviews (and this spec) over-read [[SPEC-048]]
  REQ-4808's "no *public* AST" as "no AST," and wrongly concluded a static prop/slot context lint
  is **unbuildable**. It is buildable — at the cost of an unstable-API dependency + a hand-built
  HTML-context classifier (the Go `html/template` technique). Re-framed REQ-4904/4905/Threat A and
  **Q6**: the CSS/JS/unquoted prop+slot context is a **deferred v1 scope choice**, not a substrate
  dead-end, with three real options — (a) `unstable_machinery` AST + context-aware static lint,
  (b) a restricted template language, (c) the documented residual + tripwire. The v1 *baseline*
  (ingestion-validation + autoescape + tripwire) is unchanged; only the framing of the residual is
  corrected. **Follow-up:** [[SPEC-048]] REQ-4808's wording ("no public AST") should be tightened
  to "no *stable/semver-guaranteed* public AST; `unstable_machinery` exposes one" — a one-line fix
  in a separate SPEC-048 revision.

- **0.3.0** (2026-06-25) — *second adversarial pass (Opus, fresh): 3 Blocking / 5 Major / 4 Minor;
  verdict "NOT converging — round-N fixes resurfacing as round-N+1 defects" (the SPEC-050
  pattern). All applied.* Root cause (as understood at 0.3.0; **the "no AST" premise was
  corrected in 0.4.0**): v0.2.0 specified static template guarantees on minijinja, believed at the
  time to expose no template AST ([[SPEC-048]] REQ-4808). **B-1 (RELOCATED):** the prop-context
  *linter* was treated as unbuildable → re-grounded REQ-4904 on
  **ingestion-validation (`url` scheme, context-independent) + HTML autoescape + a tainted-value
  tripwire**, with the CSS/JS/unquoted context stated as an **honest residual** (Q6 — a restricted
  template language is the real fix), not a closed guarantee. **B-2 (PARTIAL):** the **slot
  landing context** was unguarded (sanitised HTML in `<a href="{{ slot }}">` = XSS) → REQ-4905 now
  states the element-content-only contract + the same Q6 residual. **B-3 (most dangerous):**
  nested directives re-introduced the v1 composite-sanitiser flaw → REQ-4906 now specifies
  **bottom-up expansion with a per-provenance barrier** (each level's own untrusted text sanitised
  once; nested composed fragments are trusted-safe and never re-sanitised). **M-1:** REQ-4907 is
  now a **deny-by-default context allowlist** (not just `transclude` removed — other site-tier
  reach plugged). **M-2:** dropped the incoherent "allowlist intersection" — the **trusted
  template** produces the static fallback (unsanitised), only the slot is sanitised, so a11y
  parity holds. **M-3:** URL validator is a **scheme allowlist after canonicalisation**, rejecting
  scheme-relative `//evil` + obfuscation + `<base>`. **M-4:** ptype-coercion recogniser,
  single-line quoted values, `#id`/`.class`→unknown-prop diagnostic. **M-5:** `content_slots`
  deferred (Q8). Minors: canonical serialiser (determinism + fixed-point), narrowest
  `content_props` default `[]`, anti-oracle diagnostics, Threat-A slot sub-vector. **Still NOT
  converged:** the Q6 prop/slot context residual + Q1 sanitiser are the substrate-level limits;
  the honest path is a restricted template language + executable fuzzing + a human security expert,
  and v0.3.0's own fixes are unreviewed.

- **0.2.0** (2026-06-25) — *first adversarial pass (Opus, fresh context): 5 Blocking / 4 Major /
  4 Minor, all applied.* The pass found the **architecture** was wrong, not just the deferred
  policy: sanitising the *final composite HTML* (B1) cannot distinguish trusted-template from
  untrusted-body nodes, so it either guts trusted templates or admits an XSS escape hatch.
  **Fix (ADR-4903 rewrite):** make each untrusted input safe **in isolation, before composition**
  — the **body is sanitised alone before being slotted in** (REQ-4905/4906/CON-4902; provenance
  by construction, the real CON-5007 analogue), and **prop safety is context-correct escaping**
  with a restricted set of allowed interpolation contexts + a new **`url` ptype** (REQ-4904/
  CON-4903), not the output sanitiser (B2/B3/B4 — autoescape ≠ context-correct; `enum` is a
  constraint not a type; URL-typing now exists). CON-4902 now demands a **serialise→reparse
  fixed-point** (mXSS), `srcset` sub-grammar, comment/PI handling, and an egress note; "Threat A
  closed" downgraded to **pending Q1 + fuzzing** on both body and prop paths (B5). M1: reserved
  manifest keys (gate-off tolerance) fixes the forward-compat break + REQ-4912. M2: REQ-4907 now
  **omits the `transclude` global** (mechanism, transitive) instead of a dead draft-forbid clause.
  M3: phase-ordered directive recognition (code-fence exclusion) + the `Directive` node declared a
  SPEC-032 amendment. M4: single-parse body re-entrancy. Minors: 049/050 allowlist reconciliation
  (REQ-4910), `content_invocable` named as SPEC-050's content-island trigger, diagnostics report
  kinds/counts not payloads (oracle), bare-grammar not relied on for scheme safety. **Still NOT
  converged:** the review's verdict was that closing Q1 alone would not have saved the original
  architecture; v0.2.0 fixes the architecture, but Q1 (sanitiser engine + complete grammar +
  fixed-point + fuzzing) and the prop-linter soundness remain the human-expert + executable-fuzz
  gate — and v0.2.0's own fixes have not had a clean-context pass.
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
