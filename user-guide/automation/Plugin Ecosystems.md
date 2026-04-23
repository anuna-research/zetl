---
title: Plugin Ecosystems
tags: [automation, hooks, pandoc, mdbook, remark]
---

# Plugin Ecosystems

Rather than rewriting every transform in ztl, reuse the ones that already exist. The plugin-ecosystem adapters let a [[Render Pipeline Hooks|render-pipeline hook]] delegate to a **Pandoc filter**, an **mdBook preprocessor**, or a **remark plugin**. ztl handles AST translation in both directions; the hook manifest just names the plugin.

If you already have a Lua filter you wrote for an academic paper, or an `mdbook-mermaid` you use for a book, you can reach for it from a ztl vault without a rewrite.

## Declaring an ecosystem hook

Pick the stage, name the ecosystem, point at the plugin. A Pandoc Lua filter in the transform stage:

```toml
# .ztl/hooks/transform.d/smallcaps.py.toml
ecosystem = "pandoc"
lua_filter = "filters/smallcaps.lua"   # OR: exec = "pandoc-smallcaps"

stage = "transform"
mode  = "persistent"
extension_id = "smallcaps"
```

Each ecosystem has its own required/optional manifest fields:

| Ecosystem | Required | Optional |
|-----------|----------|----------|
| `pandoc`  | `exec` **or** `lua_filter` | `args` |
| `mdbook`  | `exec`   | `scope = "page" \| "vault"` |
| `remark`  | `package` | `version`, `options` |

Pandoc hooks default to the `transform` stage; mdBook preprocessors run at `pre-parse` (they operate on raw Markdown); remark targets the `transform` stage with the mdast AST type. See each ecosystem's deep-dive guide in the main repo's `docs/ecosystems/` directory for translation details and per-plugin quirks.

## Scaffolding

The authoring CLI seeds the right manifest fields (and, for Pandoc, drops a starter identity Lua filter on disk) when you pass `--ecosystem`:

```bash
ztl hook new transform smallcaps --ecosystem pandoc
ztl hook new pre-parse admonish  --ecosystem mdbook
ztl hook new transform math      --ecosystem remark
```

The generated hook works out of the box: `ztl hook test smallcaps` will pass against the seeded fixture. From there you edit the filter, add selectors, and tune the behavioural contract like any other [[Render Pipeline Hooks|pipeline hook]].

## Probing ecosystems

Before a build — especially in CI — check every configured ecosystem is reachable:

```bash
$ ztl ecosystem check
{
  "entries": [
    { "id": "pandoc", "status": "detected", "version": "pandoc 3.7.0.2",
      "executable": "/opt/homebrew/bin/pandoc",
      "configured": 1, "available_plugins": ["smallcaps"] },
    { "id": "mdbook", "status": "detected", "version": "mdbook v0.5.2", ... },
    { "id": "remark", "status": "detected", "version": "v22.12.0", ... }
  ],
  "hooks_configured_total": 1
}
```

Exit code 0 when every *configured* ecosystem is available; the zero-configured state also exits 0. Missing-runtime cases surface hints pointing at the install path for the tool (`brew install pandoc`, `cargo install mdbook`, etc.). `--json` is the same output forced to JSON for piping into CI gates.

## Feature flags

Each adapter lives behind a cargo feature:

- `ecosystem-pandoc`
- `ecosystem-mdbook`
- `ecosystem-remark`

All three are compiled into default release builds. If you want a minimal binary — embedded deployment, a containerised collab server with no Pandoc on disk — build with `cargo install --no-default-features --features "<your flags>"` and include only the ecosystems you need.

A hook declaring `ecosystem = "pandoc"` against a binary compiled without that feature fails fast with a `RuntimeAbsence` diagnostic pointing at the ecosystem matrix entry — you won't get cryptic "exec not found" mysteries.

## Mixed-parser diagnostic

ztl's native parser is CommonMark. Pandoc has its own, richer parser; remark's mdast is different again. Most of the time this doesn't matter — the adapter translates at the boundary — but if a page parsed by CommonMark is routed through a hook expecting Pandoc AST (or vice versa), you'll see a **five-part mixed-parser diagnostic** with three concrete remediations:

1. Set `parser:` in the page's frontmatter to force the parser.
2. Narrow the hook's selector so it doesn't match incompatible pages.
3. Disable the hook for that page (frontmatter opt-out).

By default the diagnostic is a warning and the build continues. Pass `ztl build --strict-parsers` to upgrade it to a fatal error — the right choice for CI.

## Per-ecosystem guides

The deep-dive guides in the ztl repo's `docs/ecosystems/` cover install, the specific manifest fields, and the compatibility matrix of known-working plugins:

- [pandoc.md](https://codeberg.org/anuna/ztl/src/branch/main/docs/ecosystems/pandoc.md) — filter vs. native mode, Quarto support.
- [mdbook.md](https://codeberg.org/anuna/ztl/src/branch/main/docs/ecosystems/mdbook.md) — envelope shape, scope settings.
- [remark.md](https://codeberg.org/anuna/ztl/src/branch/main/docs/ecosystems/remark.md) — harness architecture, package resolution.
- [matrix-contribution.md](https://codeberg.org/anuna/ztl/src/branch/main/docs/ecosystems/matrix-contribution.md) — how to file a known-working plugin.

## Related

- [[Render Pipeline Hooks]] — the underlying hook mechanism that ecosystem adapters plug into.
- [[Lifecycle Hooks]] — the around-the-build sibling for shell scripts that don't need AST access.
- [[Customising the Look]] — how a theme can bundle an ecosystem hook.
- [[CLI Overview]] — every `ztl hook` and `ztl ecosystem` subcommand in one reference.
- [[Installation]] — cargo feature flags at install time.
