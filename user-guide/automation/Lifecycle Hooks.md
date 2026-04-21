---
title: Lifecycle Hooks
tags: [automation, hooks, cli]
---

# Lifecycle Hooks

Git-style scripts that run at defined points during a zetl operation. Drop an executable into `.zetl/hooks/` named after a lifecycle point, and zetl invokes it with structured JSON on stdin and a handful of `ZETL_*` environment variables.

These are separate from the AST-level [[Render Pipeline Hooks]] — lifecycle hooks fire *around* the build, not inside it. Use them to publish an RSS feed after `zetl build`, notify a chat room on `on-save`, or run a schema validator on `post-check`.

## Lifecycle points

| Hook | Fires | Can abort? |
|------|-------|------------|
| `pre-build` | Before `zetl build` renders pages | Yes |
| `post-build` | After `zetl build` completes | No (warning on non-zero) |
| `post-index` | After `zetl index` completes | No |
| `post-check` | After `zetl check` collects diagnostics | No |
| `pre-serve` | Before `zetl serve` binds | Yes |
| `on-save` | After a page is saved via `zetl serve` | No |
| `on-agent` | When an agent API request arrives | No |
| `on-access-request` | When a user requests page access (collab mode) | No |

"Can abort" means a non-zero exit cancels the parent operation; every other hook just logs a warning and continues.

## What a hook receives

- **stdin** — a JSON object with vault metadata, the page list, the link graph, and hook-specific fields (e.g. `saved` for `on-save`). If [[Time Travel|history is enabled]], a `history` object is included.
- **environment** — `ZETL_HOOK` (the lifecycle name), `ZETL_VAULT_ROOT`, `ZETL_THEME`, `ZETL_VERSION`, plus hook-specific vars: `ZETL_OUT_DIR` on build hooks, `ZETL_PORT` on serve hooks.
- **working directory** — the vault root, regardless of where `zetl` was invoked from.
- **timeout** — 30 seconds wall-clock. A hook that hangs is killed.

## Writing a hook

Create an executable file named exactly after the lifecycle point (no extension):

```bash
# .zetl/hooks/post-build
#!/bin/bash
# Generate an RSS feed from pages with a `date:` frontmatter field.
set -euo pipefail

jq -r '
  .pages
  | map(select(.frontmatter.date))
  | sort_by(.frontmatter.date) | reverse | .[0:20]
  | map("<item><title>\(.title)</title><pubDate>\(.frontmatter.date)</pubDate></item>")
  | "<rss><channel>\(join(""))</channel></rss>"
' < /dev/stdin > "$ZETL_OUT_DIR/feed.xml"
```

Then mark it executable:

```bash
chmod +x .zetl/hooks/post-build
```

That's it. The next `zetl build` will invoke it and your feed will land alongside the other built assets.

## Theme-bundled hooks

Themes can ship hooks in their own `hooks/` subdirectory. When you build with `--theme fountain`, zetl runs `.zetl/themes/fountain/hooks/post-build` **before** `.zetl/hooks/post-build`. Both run if both exist — the theme gets first crack, the vault overrides or augments.

```
.zetl/themes/fountain/
  hooks/
    post-build
  base.html
  page.html
```

This lets a theme ship automation it genuinely needs (e.g. rewriting asset paths on publish) without stomping on vault-specific hooks you've added yourself.

## Listing and testing

Introspect the composed hook set for a vault/theme pair:

```bash
zetl hook list
zetl hook list --theme fountain
```

Run a hook manually against real vault context — handy during development, or for wiring into CI:

```bash
zetl hook run post-build
zetl hook run on-save -- '{"saved":{"file":"notes/today.md","page":"Today","content_length":1204}}'
```

Extra JSON after `--` is merged into the context, so you can simulate hook-specific payloads without actually triggering the underlying event.

## Safe mode

`zetl build --safe-mode` and `zetl serve --safe-mode` disable every vault hook. Only theme hooks declared in the theme's `[[theme.hooks]]` manifest table run. That's the switch you want when previewing someone else's vault or auditing a theme.

## Related

- [[Render Pipeline Hooks]] — AST-level transforms that run *inside* the build, not around it.
- [[Plugin Ecosystems]] — delegate transforms to Pandoc, mdBook, or remark.
- [[Static Site Export]] — what `post-build` has to work with.
- [[Watching for Changes]] — the built-in analogue of a `post-index` loop.
- [[Configuration]] — `theme.toml`, `ZETL_*` environment variables, and the `.zetl/` directory layout.
