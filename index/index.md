# zetl content components & islands — demo

Two layered capabilities, both authored from plain Markdown, both opt-in:

- **[[content-directives|SPEC-049]]** — content authors invoke theme components from prose
  with `:::name{…}` directives. Untrusted attributes and body are validated + sanitised.
- **[[islands|SPEC-050]]** — components ship `<name>.js` that hydrates in the browser;
  untrusted code runs in a sandboxed Worker and paints through a controlled renderer.

→ See the [[islands]] page for the live, interactive island demos.

## Content directives (SPEC-049)

These are written as Markdown directives. The **template is trusted**; only the body and
props are sanitised/validated.

:::callout{tone=info title="A callout component"}
This whole block is `:::callout{tone=info title="…"}` in Markdown. The body is rendered
Markdown — **bold**, `code`, [links](https://anuna.io) — sanitised in isolation.
:::

### Untrusted body is sanitised

The next callout's body tries to inject script and a `javascript:` link. Watch them vanish
(view source to confirm):

:::callout{tone=danger title="Hostile body — neutralised"}
Injecting a script: <script>document.title='PWNED'</script> ← gone.
A bad link: <a href="javascript:alert(1)">click me</a> ← scheme stripped.
A safe link survives: [anuna.io](https://anuna.io).
:::

### Props are typed + validated

The `card` component has a **`url`-typed** prop (`href`) that is scheme-validated at
ingestion, and a slot for its body:

:::card{title="A typed component" href=https://anuna.io}
Props (`title`, `href`) are validated against the manifest. A bad `href` (e.g.
`javascript:…`) makes the directive fail closed.
:::

### Nesting (per-provenance barrier)

A directive body can contain other directives. Each level's own text is sanitised once;
a nested component's trusted markup is preserved:

:::callout{tone=warning title="Outer callout"}
Outer body text (sanitised). Below is a nested card whose trusted `<article>`/`<a>` markup
is **not** stripped by the outer sanitiser:

:::card{title="Nested card" href=https://anuna.io}
I am a component rendered inside another component.
:::
:::

### Leaf directive

A no-body `::divider{}` leaf directive:

::divider{label=section break}

### Default-deny + validation failures render inert

A directive for a component the theme did **not** mark content-invocable is refused — its
body is emitted inert (sanitised), never expanded:

:::raw-html{}
This component does not exist / is not content-invocable, so this text is shown inert and
any <script>alert(1)</script> here is still stripped.
:::

A prop that violates the manifest (`tone=bogus` is not in the enum) also fails closed:

:::callout{tone=bogus title="Won't expand"}
This body renders inert because `tone=bogus` fails enum validation (see the build log for
the `content-prop-enum` diagnostic).
:::
