---
title: Render Pipeline Hooks
tags: [automation, hooks, ast, advanced]
---

# Render Pipeline Hooks

Transform a page *mid-build* by mutating a typed AST rather than its serialised form. Unlike the around-the-build [[Lifecycle Hooks]], render-pipeline hooks run on every matching page as it moves through the renderer — callouts, custom blocks, link rewrites, banner injection, citation expansion, anything that needs to touch structure.

Hooks live under `.ztl/hooks/<stage>.d/` (or `.ztl/themes/<name>/hooks/<stage>.d/` for theme-bundled ones) and stay resident for the duration of a build via a JSON-lines protocol over stdin/stdout. No per-page subprocess spawn, no shell fork on every paragraph.

## The three stages

| Stage | Payload at the boundary | Typical use |
|-------|-------------------------|-------------|
| `pre-parse` | raw Markdown string | Frontmatter rewrites, include/import expansion, prelude injection |
| `transform` | typed AST (`ztl-ast` schema v1) | Callouts, custom blocks, link-graph mutations, ecosystem plugins |
| `post-render` | HTML fragment string | Banner injection, post-processing, accessibility fixes |

Stages run in that order, once per page. A `pre-parse` hook sees what the author wrote; a `transform` hook sees the parsed tree; a `post-render` hook sees the nearly-final HTML.

## A hook is two files

Scaffold one with the authoring CLI — it writes the skeleton, a sidecar manifest, and a seeded fixture so `ztl hook test` passes immediately:

```bash
ztl hook new transform callouts                # Python (default)
ztl hook new pre-parse prelude --lang sh       # shell
ztl hook new post-render banner --lang js      # JavaScript
```

Resulting layout:

```
.ztl/hooks/transform.d/
  callouts.py            # persistent-mode skeleton (executable)
  callouts.py.toml       # sidecar manifest — composition reads <name>.<ext>.toml
tests/hook-fixtures/callouts/
  input.md
  expected.json          # golden output, seeded so `hook test` passes
```

The sidecar TOML manifest is where all the interesting configuration lives:

```toml
# .ztl/hooks/transform.d/callouts.py.toml
stage = "transform"
mode  = "persistent"
extension_id = "callouts"

[selector]
glob = "**/*.md"
frontmatter = "layout == 'note' && !draft"
content = "re:^> \\[!"          # pages containing callout syntax

[contract]
preserves = ["Wikilink", "Embed"]   # these node types must survive untouched
idempotent = true                    # running twice = running once
may_restructure = false              # pre-parse only; denied at transform
expansion_bound = 2.0                # output ≤ 2× input size

[timeouts]
run_ms = 200
```

Selectors narrow *which* pages this hook applies to: a glob, a frontmatter predicate, and an optional content regex (prefix with `re:`). Only pages matching all three are dispatched. Contracts are declared behaviour the pipeline then **enforces** — more on that below.

## Authoring workflow

```bash
# Run the hook against its golden fixture, diff the output
ztl hook test callouts

# Watch the source file and restart the persistent process on edit
ztl hook watch callouts

# Capture a real vault page as a fresh fixture
ztl hook fixture --from projects/q2.md --hook callouts

# Probe every composed hook for supported stages + AST schema version
ztl hook capabilities --stage transform

# Check selector reachability without invoking the hook
ztl hook dry-run transform/callouts

# Per-hook coverage from the most-recent build (pages matched, failures, latency)
ztl hook coverage --stage transform
```

`hook dry-run` is the fast feedback loop for selectors. `hook capabilities` is how you catch a hook that's declaring an AST schema version the running ztl binary can't speak. `hook coverage` tells you whether the hook actually *did* anything the last time you built.

## The AST

The transform stage gets a parsed `Document` matching `ztl-ast` schema v1. The full node list, JSON shape, and rendering contract are in the [ztl-ast reference](https://codeberg.org/anuna/ztl/src/branch/main/docs/ztl-ast-reference.md) — every node type (Paragraph, Heading, Wikilink, Embed, SplBlock, etc.) with canonical examples. Don't duplicate that content in your hook; just import the helper library and walk the tree.

Helper libraries ship alongside the runtime so hooks don't hand-roll JSON framing:

- **Python** — `tools/ztl-ast-py/`, py3.9+, `walk()`, `map_nodes()`, `@on_node` decorator.
- **TypeScript** — `tools/ztl-ast-js/`, typed AST classes, `onNode()` dispatch table.

Inspect or diff AST directly with:

```bash
ztl ast sample notes/foo.md --stage transform
ztl ast diff before.json after.json
```

## Behavioural contracts

The `[contract]` block is not just documentation — ztl enforces it:

- `preserves = ["Wikilink", "Embed"]` — counts these node types pre- and post-hook; a strict decrease fails the hook.
- `idempotent = true` — the pipeline runs the hook a second time on its own output; if the result differs, it fails.
- `may_restructure = true` — allowed only at `pre-parse`; denied at `transform`/`post-render`.
- `expansion_bound = 2.0` — output byte size ratio ceiling.

A violation surfaces as a five-part diagnostic — *summary, context, observed, cause, hint* — with concrete remediations. Noisy hooks with loose contracts are easy to audit because the diagnostic names the exact node type that went missing.

## When a hook fails

The pipeline no longer aborts. When a hook errors mid-stage, the page reverts to the *previous stage's output* and the pipeline keeps going. The failure is appended to `diagnostics.json` in the build output as a `FailureRecord` with the hook, stage, page, reason, and duration. The rest of your vault builds normally.

Use `ztl hook coverage` after the build to see which hooks failed on which pages.

## Safe mode

`ztl build --safe-mode` and `ztl serve --safe-mode` skip every vault hook. Only hooks declared in the active theme's `[[theme.hooks]]` manifest run. That's the switch for previewing an untrusted vault or auditing a theme release. Full security model — env-var redaction, stdout/stderr caps, per-hook timeouts — is documented in the [hook-security guide](https://codeberg.org/anuna/ztl/src/branch/main/docs/hook-security.md).

## Related

- [[Lifecycle Hooks]] — hooks that fire *around* the build, not inside it.
- [[Plugin Ecosystems]] — delegate transforms to Pandoc filters, mdBook preprocessors, or remark plugins.
- [[MCP Server]] — expose the vault to AI agents once your hooks have shaped it.
- [[Customising the Look]] — theme hooks and template overrides side by side.
- [[Configuration]] — the `.ztl/hooks/` directory, theme manifest keys, and `ztl_*` env vars.
