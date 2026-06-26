# Content-Author Components & Directives

Content authors (anyone writing Markdown in the vault, including `--collab` editors) can
invoke a theme component **from inside their prose** with directive syntax — *if* the
theme explicitly allows it. This implements
[SPEC-049](../specs/SPEC-049-content-author-components.md) on top of the
[component model](components.md) (SPEC-048).

The trust model is the whole point: the **template is trusted** (theme authors ship it),
but the **directive's attributes and body are untrusted**, so both are recognised and
sanitised before anything is rendered.

> ⚠️ **Not production-hardened.** SPEC-049 is a non-converged strawman; its own text says
> the untrusted-HTML boundary needs a human security expert + executable fuzzing before
> shipping content-invocable components. The implementation is gated behind the default-on
> `content-components` cargo feature. Treat it as a working preview, not a security
> guarantee.

## Quick start

1. Mark a component content-invocable in its manifest and list the props content may set:

   ```toml
   # .zetl/components/callout/callout.toml
   name = "callout"
   requires = ["site"]
   content_invocable = true          # default false — opt-in (REQ-4903)
   content_props = ["tone"]          # the EXACT props settable from content; default []
   [props]
   tone = { type = "string", default = "info", enum = ["info", "warning"] }
   ```

2. Author writes a directive in any `.md` page:

   ```markdown
   :::callout{tone=warning}
   Heads up — this is **content-authored** with a [link](https://example.com).
   :::
   ```

3. `zetl build` recognises the directive, validates `tone` against the manifest, renders
   the `callout` template with the body as a **sanitised** default slot, and emits the
   component's static HTML (carrying `data-z="callout"`). It works with JS disabled.

## Directive syntax (CON-4901)

Three forms, after the remark-directive / CommonMark generic-directive prior art:

| Form | Syntax | Body |
|------|--------|------|
| Container | `:::name{attrs}` … `:::` | a Markdown body → the default slot |
| Leaf | `::name{attrs}` | none |
| Inline | `:name[label]{attrs}` | **deferred in v1** — stays literal text (Q2) |

> The inline form is recognised by the grammar but **not expanded in v1** (open question
> Q2): expanding it correctly needs inline-level Markdown rendering within a paragraph,
> which the current block-segment expansion cannot do without splitting the paragraph. So
> `:name[…]` renders as literal text for now. Use container/leaf forms.

- `name` is kebab-case. `{attrs}` accepts `#id`, `.class`, `key=value`, `key="quoted"`,
  and bare `key` flags (boolean true). Quoted values are single-line.
- Container nesting uses the colon-count rule: a `::::` container may hold a `:::` one.
- Directive syntax inside a fenced code block or inline code span is **literal text**
  (so documentation showing the syntax is not expanded).
- A malformed directive **fails closed** — it stays inert text, never raw HTML.

## Default-deny (REQ-4903)

A component is reachable from content **only** when its manifest sets
`content_invocable = true`. A directive naming a component that does not exist, or exists
but is not content-invocable, renders **inert** (its body sanitised, no component) with a
`content-directive-unknown` diagnostic. The flag is theme-author-set; an author can never
set it from content.

## What is checked (the security model)

Each untrusted input is made safe **in isolation, before composition** (ADR-4903):

- **Props (REQ-4904).** Recognised against the `[props]` schema in the resolution layer:
  unknown prop → `content-prop-unknown`; wrong type → `content-prop-type`; missing
  required → `content-prop-missing`; out-of-`enum` → `content-prop-enum`; `list`/`map` →
  `content-prop-unsupported`. A `url`-typed prop is **scheme-validated at ingestion**
  (`https`/`http`/`mailto`/relative; `javascript:`/`data:`/`//host` rejected), so it is
  safe wherever the template places it.
- **Body (REQ-4905 / CON-4902).** The body is rendered to HTML then **sanitised in
  isolation** with a closed default-deny allowlist (built on `ammonia`): `<script>`,
  `<style>`, `<iframe>`, `on*` handlers, comments, dangerous URL schemes, and any
  non-allowlisted element are stripped. The trusted template is **not** sanitised (it may
  use `<form>`/`<button>`/etc.) — only the untrusted body slotted into it.
- **Context lint (CON-4904).** A build-time, sound static check (over minijinja's AST)
  rejects any content prop or the sanitised slot reaching a CSS / JS / URL / unquoted-
  attribute context — the contexts HTML autoescape cannot neutralise — including
  transitively, **failing the build** (`content-context-unsafe` /
  `content-context-indeterminate` / `content-template-unanalyzable`). It fails closed on
  anything it cannot prove safe, so a content-invocable template that places a content
  value unsafely will not build. `{{ prop | safe }}` on a content value is rejected
  (`content-unsafe-emit`).
- **Restricted context (REQ-4907).** Content components render through a deny-by-default
  env that binds only `props` + the sanitised slot — **no** `transclude`, no page-tier
  fields, no vault graph. An attempt to reach them fails closed (strict-undefined), never
  leaking draft/unlisted content.
- **Bounds (REQ-4908).** Nesting is depth-bounded (`content-directive-too-deep`); cycles
  reuse the SPEC-048 cycle check; component output is never re-scanned for directives.

Every failure surfaces as a build diagnostic tied to the author's source line
(REQ-4911); sanitiser drops are reported as kinds + counts, never the payload verbatim
(OBS-4901).

## Interactive content components → islands

A content-invocable component that also ships `<name>.js` becomes a **content-author
island** (REQ-4910): SPEC-049 produces the sanitised static HTML as the no-JS fallback,
and [SPEC-050](islands.md) governs the runtime (a sandboxed Worker, capability-scoped
bus access, `content:`-namespaced topics).

## Backward compatibility (REQ-4912)

With no `content_invocable` component, `:::name{…}` is literal Markdown text and the
output is byte-identical to a build without this feature. The manifest keys
`content_invocable`/`content_props` are reserved-and-accepted even under a build that
does not use them.
