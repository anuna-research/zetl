---
title: Customising the Look
tags: [reading, themes, templates]
---

# Customising the Look

Both [[Web Server]] and [[Static Site Export]] accept `--theme <name>`. A theme is a folder under `.zetl/themes/<name>/` containing Minijinja templates, static assets, and an optional `theme.toml`. You only override what you want — everything you don't ship falls back to the built-in defaults.

## Why themes matter

The default theme is deliberately neutral so it works as a starting point, not an endpoint. You will probably want to change at least the colours, the typography, and the name at the top of the sidebar. Most of that is CSS-only: zetl exposes a versioned set of CSS custom properties so you can restyle the graph widget, the shell, and page typography without touching any HTML.

The bigger reason the theming story is careful: zetl's graph widget is a single, persistent Sigma.js instance that survives page navigation (no flash, no re-layout). Themes that rewrite `base.html` have to preserve two DOM markers to keep that property — otherwise the graph re-initialises on every click. The contract below is the bargain that makes "persistent graph across a multi-page site" possible.

## Creating a theme

Bundled themes are built into the binary. Export one to the vault to start editing:

```bash
zetl theme list                        # what's available
zetl theme export default paper        # copies default to .zetl/themes/paper/
```

Then point `serve` or `build` at it:

```bash
zetl serve --theme paper
zetl build --theme paper
```

## What lives where

```
.zetl/themes/paper/
  theme.toml                  # config for SPA, graph, contract version
  base.html                   # master layout (sidebar, modals, scripts)
  index.html                  # vault landing page
  page.html                   # one page view
  folder.html                 # folder index
  help.html                   # /help page
  static/
    theme.css                 # your CSS
    enhance.js                # extra JS (runs per navigation)
    fonts/
```

All templates use [Minijinja](https://github.com/mitsuhiko/minijinja) (Jinja2-compatible). Child templates `{% extends "base.html" %}` and override blocks; missing templates fall back to built-ins.

## CSS-only restyling

For most visual changes you don't need to override any HTML. The default theme exposes CSS custom properties for every colour, size, and layout track the graph and shell use. Override them in your own `static/theme.css`:

```css
:root {
  --zetl-graph-node: oklch(0.7 0.15 250);
  --zetl-graph-edge: oklch(0.6 0.08 250 / 0.3);
  --zetl-graph-widget-width: 360px;
  --zetl-shell-sidebar-area: 18rem;
}
```

The full property list (`--zetl-graph-*`, `--zetl-graph-widget-*`, `--zetl-shell-*`) lives in [[Configuration]]. Sigma reducers read the properties via `getComputedStyle` and refresh on `prefers-color-scheme` or `data-theme` changes, so a dark-mode toggle is a CSS class flip, not a theme rebuild.

## `theme.toml`

The config file groups settings into tables:

```toml
[theme]
name = "paper"
version = "1.0.0"
contract = "1"                      # theme-contract major version

[spa]
enabled = true                      # persistent-shell SPA navigation
transition = "crossfade"

[graph]
placement = "docked"                # "docked", "tabs", or "stacked"
```

### Graph placements

The persistent graph mini-map has three placements, switched by a single `theme.toml` key:

| `placement` | Layout |
|-------------|--------|
| `docked` (default) | Fixed mini-map bottom-right of the viewport, 280 × 200 px, click to expand to `/_graph`. |
| `tabs` | Widget shares the transclusion right rail via a two-tab header. |
| `stacked` | Widget sits above the transclusion panel in the right rail. |

Switching placement sets a `data-placement` attribute on the shell and flips CSS — the Sigma instance itself is untouched.

See [[Configuration]] for the full `theme.toml` reference including `[vendor.*]` pins for the bundled Sigma / graphology packages.

## Preserving the persistent shell

If you rewrite `base.html` from scratch, preserve two markers so the SPA shell keeps working:

- Wrap the sidebar and graph widget in `{% block persistent_shell %}` — this region never gets swapped.
- Put `data-zetl-volatile` on the element (usually `<main>`) whose `innerHTML` should be replaced on navigation.

Omitting them is a valid opt-out: the theme still renders, but the graph re-initialises on every page click.

## Lifecycle events for custom JS

When `[spa].enabled = true`, the shell fires two `window` events around every same-origin navigation:

- `zetl:before-navigate` — cancelable; `detail = { fromSlug, toSlug, url }`.
- `zetl:after-navigate` — `detail = { slug, contentRoot }`.

Use them to re-run Mermaid, KaTeX, or any other enhancement on swapped content:

```js
window.addEventListener('zetl:after-navigate', (e) => {
  if (window.mermaid) {
    mermaid.run({ nodes: e.detail.contentRoot.querySelectorAll('.mermaid') });
  }
});
```

## Distributing a theme

Themes are a folder, so they ship as a git repo. `zetl theme install <git-url>` clones one into `.zetl/themes/`, `zetl theme remove <name>` deletes it.

## Related

- [[Configuration]]
- [[Web Server]]
- [[Static Site Export]]
- [[Frontmatter Fields]]
- [[Lifecycle Hooks]]
