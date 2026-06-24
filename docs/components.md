# Template Components & Templated Static Pages

zetl themes can define reusable, parameterised **components**, render hand-authored
**static pages** through the same engine your themed pages use, share one set of brand
**design tokens** across both surfaces, and **transclude** live wiki content into any
page. This is the implementation of [SPEC-048](../specs/SPEC-048-components-and-static-overrides.md);
it works identically in `zetl build` and `zetl serve` (the only difference is the link
base — relative `../` in build, absolute `/` in serve).

A vault that uses none of these features builds byte-identically to before — everything
here is opt-in.

> JavaScript islands and an inter-island message bus are **not** part of this feature —
> they are deferred to [SPEC-050](../specs/SPEC-050-component-islands-and-messaging.md).
> A component's `<name>.js` is ignored today.

## Components

A component is a directory under `.zetl/components/<name>/` (or a theme's
`components/<name>/`). `<name>` must be kebab-case (`[a-z][a-z0-9-]*`).

```
.zetl/components/nav-header/
├── nav-header.html    # required — the markup (a minijinja fragment)
├── nav-header.toml    # required — the manifest
└── nav-header.css     # optional — styles (emitted once, deduped)
```

**Template** (`nav-header.html`) — reads `props.*`, renders the default slot with
`{{ caller() }}`, named slots by name, and carries the `data-z` marker on its root via
the provided `_name`:

```jinja
<nav data-z="{{ _name }}" class="nav-header">
  <a href="{{ site.root_path }}">{{ site.name }}</a>
  {{ caller() }}
</nav>
```

**Manifest** (`nav-header.toml`):

```toml
name = "nav-header"          # MUST equal the directory name
requires = ["site"]          # context tiers the component needs: site | page | folder

[props]
active = { type = "string", default = "" }
tone   = { type = "string", required = true, enum = ["info", "warning"] }
```

Prop types are `string | bool | int | number | list | map`. Unknown props, type
mismatches, missing required props, and out-of-enum values are build errors.

### Invoking a component

In any theme template **or** a templated static page:

```jinja
{# block form — body becomes the default slot (caller()) #}
{% component "nav-header" active="about" %}<span>About</span>{% endcomponent %}

{# self-closing form #}
{% component "nav-header" /%}

{# named slots #}
{% component "card" title="Hi" %}
  {% slot "header" %}<h2>Header</h2>{% endslot %}
  Body text (the default slot)
{% endcomponent %}
```

Under the hood `{% component %}` lowers to native minijinja macros + `{% call %}` before
parsing — there is no new template engine, and kebab names map to a legal macro
identifier internally (`nav-header` → `nav_header`).

### Resolution & overriding

A component resolves through the existing three-tier fallback: vault
`.zetl/components/<name>/` overrides the active theme's, which overrides the bundled
default. Resolution is **whole-directory** — overriding a component overrides its
template, manifest, and CSS as a unit. The build logs which layer each component came
from.

### Nesting

Components may nest. The build statically detects cycles (`component-cycle`, naming the
path) and bounds render depth (≤ 16). A bomb fails the build, never hangs it.

### CSS (v1: unscoped)

A component's `<name>.css` is collected, **deduplicated** (a component used N times
contributes its CSS once), and emitted deterministically to `_static/components.css`.
v1 does **not** scope selectors — namespace your own selectors by component name
(selector scoping is deferred to SPEC-051). The `data-z="<name>"` marker is already
emitted so a future scoping pass needs no markup change.

## The site context

Every render path — themed pages, folder indexes, and static pages — exposes a `site`
object:

| Field             | Meaning                                                    |
| ----------------- | ---------------------------------------------------------- |
| `site.name`       | the vault/site name                                        |
| `site.root_path`  | depth-correct link base (`../` in build, `/` in serve)     |
| `site.mode`       | `"build"` or `"serve"`                                     |
| `site.nav`        | navigation list (currently empty — config nav is a TODO)   |
| `site.tokens_url` | URL of the generated `tokens.css`                          |

Build links as `{{ site.root_path }}<target>` — never a hardcoded leading `/` — so a
shared nav resolves at every depth and under `file://`.

A component declares the tiers it needs via `requires`. A `requires = ["site"]`
component is legal on both themed and static pages. A `requires = ["page"]` component
used on a static page (which has no page context) is a build error
(`component-context-unavailable`).

## Templated static pages

Rename a hand-authored `static/about.html` to **`about.html.jinja`** and zetl renders
it (with **site context only**) instead of copying it verbatim:

```jinja
<!doctype html><html>
<head><link rel="stylesheet" href="{{ site.tokens_url }}"></head>
<body>
  {% component "nav-header" active="about" %}<span>About</span>{% endcomponent %}
  <main>{{ transclude("handbook#Mission") }}</main>
</body></html>
```

Output mapping (pretty-URL):

| Source (`.zetl/static/…`)   | Output            | URL        |
| --------------------------- | ----------------- | ---------- |
| `about.html.jinja`          | `about/index.html`| `/about/`  |
| `index.html.jinja`          | `index.html`      | `/`        |
| `blog/post.html.jinja`      | `blog/post/index.html` | `/blog/post/` |

A plain `.html` (no `.jinja`) is still copied verbatim. A static page may not read page
context — that's a compile error.

## Design tokens

Define brand tokens once in `.zetl/tokens.toml`; a theme may ship its own
`.zetl/themes/<theme>/tokens.toml`. zetl emits a single `_static/tokens.css`:

```toml
# .zetl/tokens.toml
moss   = "#5B8C5A"
radius = "8px"

[theme.dark]
moss   = "#80b47f"
```

```css
/* _static/tokens.css */
:root { --moss: #5B8C5A; --radius: 8px; }
[data-theme="dark"] { --moss: #80b47f; }
```

Layers **merge key-by-key**: a vault `tokens.toml` overrides individual theme tokens, so
changing one variable is one line, not a forked stylesheet. Both themed pages and static
pages link the same `tokens.css`. Token values are validated as CSS-token-safe — a value
that could break out of the declaration (`;`, `}`, comments, markup, a remote `url()`) is
rejected (`tokens-value-unsafe`).

## Transclusion

`{{ transclude("page") }}` pulls *live, named* wiki content into any page (themed or
static), reusing the same resolver that backs `![[…]]` embeds:

```jinja
{{ transclude("handbook") }}            {# whole page #}
{{ transclude("handbook#Mission") }}    {# a heading section #}
```

It returns the **rendered HTML** of the addressed content. It exposes only a closed
allow-list (rendered HTML + title) — never the target's backlinks, edges, or raw
frontmatter — so addressing a named page does not re-admit ambient page context. A draft
or non-existent target fails closed (`transclude-target-unresolved`) and shows as a dead
link in `zetl check`.

v1 limitations: internal wikilinks inside transcluded content are not re-resolved; block
addressing (`#^id`) falls back to the whole page; the `.title` accessor is deferred.

## Error reference

| Code                            | Meaning                                                  |
| ------------------------------- | -------------------------------------------------------- |
| `component-malformed`           | missing template/manifest, bad name, or unknown manifest key |
| `component-not-found`           | invoked a component that doesn't resolve                 |
| `component-unknown-prop` / `-prop-type` / `-prop-missing` / `-prop-enum` | prop validation failures |
| `component-context-unavailable` | a component needs a tier the render path doesn't expose  |
| `component-cycle`               | components invoke each other in a cycle                  |
| `component-depth-bound`         | nesting/transclusion exceeded the depth cap              |
| `tokens-value-unsafe`           | a token value is not CSS-token-safe                      |
| `transclude-target-unresolved`  | transclude target missing, draft, or unparseable         |
