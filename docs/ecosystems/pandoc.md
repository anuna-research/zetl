# Pandoc ecosystem guide

The Pandoc adapter lets ztl run Pandoc filters (the
`pandoc-<name>` binary convention) and Pandoc-native features
(`--citeproc`, `--lua-filter=…`) over vault pages as part of the
normal hook pipeline. This guide covers install, configuration, the
set of plugins tracked in the compatibility matrix, and how Quarto
filters fit in.

The authoritative surface for the Pandoc adapter is
[SPEC-033 REQ-3303 / CON-3303](../../specs/SPEC-033.md); this doc
restates that material in a user-facing shape and is kept in sync
with the shipped `ecosystem-pandoc` feature flag.

<!-- toc -->
- [Install](#install)
- [Configuration](#configuration)
- [Invocation contract](#invocation-contract)
- [Known-working filters](#known-working-filters)
- [Using Quarto filters](#using-quarto-filters)
- [`ztl ecosystem check` walkthrough](#ztl-ecosystem-check-walkthrough)
- [Troubleshooting](#troubleshooting)

## Install

The Pandoc adapter requires a `pandoc` binary at version **2.11 or
later** on `$PATH` (the minimum is pinned in
`src/ecosystems/registry.rs`). 2.11 is the first release with
built-in `--citeproc`, which the adapter's native mode depends on.

Install Pandoc with your platform's package manager:

```sh
# macOS
brew install pandoc

# Debian / Ubuntu
apt install pandoc

# Arch
pacman -S pandoc
```

Filters are separate installs. For example:

```sh
# Haskell filters (use cabal, ghcup, or your distro's pandoc-* packages)
cabal install pandoc-crossref

# Python filters
pip install --user pantable

# cargo-installed filters land in $CARGO_HOME/bin
cargo install pandoc-include-code
```

ztl does not install filters for you — it defers to each
ecosystem's native install mechanism and only provides hints when a
configured filter is missing at build time (SPEC-033 §13 Q4).

The Pandoc adapter is compiled into default ztl builds via the
`ecosystem-pandoc` cargo feature. To build ztl without it:

```sh
cargo build --no-default-features --features "<your-other-flags>"
```

Hooks declaring `ecosystem = "pandoc"` in a binary compiled without
the feature fail fast with a `RuntimeAbsence` diagnostic pointing
at the matrix entry (SPEC-032 CON-3225).

## Configuration

Pandoc hooks live under `.ztl/hooks/transform.d/` (default) or
`.ztl/hooks/pre-parse.d/` (for native-mode pages whose entire
pipeline is Pandoc's). One TOML manifest per hook, named by the
default-extension-id convention (see SPEC-032 REQ-3217).

### Filter mode (default)

External filter plugins — the `pandoc-*` binary convention.

```toml
# .ztl/hooks/transform.d/crossref.toml
stage = "transform"
ecosystem = "pandoc"
exec = "pandoc-crossref"
mode = "persistent"         # preferred; see below
```

Filter mode runs at the `transform` stage only — Pandoc filters
operate on the pandoc-types AST, not on raw Markdown. The
`pre-parse` stage is rejected for filter-mode manifests.

When `mode = "persistent"` the adapter keeps the filter subprocess
alive across pages and exchanges pandoc-types JSON over line-
delimited stdio per SPEC-032 CON-3201. Filters that do not
implement the persistent handshake fall back to one-shot invocation
per page automatically.

### Native mode

Pandoc-native invocation modes (`--citeproc`, `--lua-filter`,
`--from markdown --to html` with flags). Use this for citations:
`pandoc-citeproc` was deprecated after Pandoc 2.11 and the correct
path is now the built-in `--citeproc` flag.

```toml
# .ztl/hooks/pre-parse.d/citeproc.toml
stage = "pre-parse"
ecosystem = "pandoc"
mode = "native"
args = ["--citeproc", "--bibliography=references.bib"]
```

Native mode at `pre-parse` replaces the page's raw Markdown with
Pandoc's HTML output before ztl's own parser runs — effectively
delegating the full pipeline to Pandoc for that page. Native mode
at `transform` is also supported: it runs after ztl's translator
boundary on the pandoc-types AST.

Lua filters live in native mode too:

```toml
stage = "transform"
ecosystem = "pandoc"
mode = "native"
lua_filter = "filters/emphasise-acronyms.lua"
```

Declare either `exec` or `lua_filter`, not both.

### Manifest fields

Base hook manifest fields (`stage`, `timeout_ms`, `ast_type`,
`select.*`, `before`, `after`, `optional`, `extension_id`) are
documented in SPEC-032 REQ-3217. Pandoc-specific fields per
SPEC-033 REQ-3312:

| Field        | Type            | Use                                                               |
|--------------|-----------------|-------------------------------------------------------------------|
| `exec`       | string          | Filter binary name. Required for `mode = "filter"`.               |
| `args`       | list of strings | argv[1..] passed to the filter or `pandoc` binary.                |
| `lua_filter` | string          | Path to a Lua filter file. Native mode only; mutually exclusive with `exec`. |
| `mode`       | `"filter"` \| `"native"` | Defaults to `"filter"`.                                    |
| `ast_type`   | `"pandoc-ext"`  | Defaults to `"pandoc-ext"` for Pandoc hooks; rarely overridden.   |

Non-Pandoc fields (e.g. `package` from the remark adapter) on a
Pandoc manifest are rejected at parse time (REQ-3312
cross-ecosystem field validation).

### Parser selection

For best fidelity, pages processed by Pandoc transform hooks
should also be parsed by Pandoc — otherwise ztl translates the
commonmark-ztl-ext AST to pandoc-types at the adapter boundary,
which is functional but lossy for Pandoc-specific syntax
pulldown-cmark does not recognise (attribute blocks, fenced divs,
extended table dialects).

Vault-wide:

```toml
# .ztl/config.toml
[parse]
default = "pandoc"
```

Directory-scoped:

```toml
[[parse.rule]]
pattern = "papers/**"
parser  = "pandoc"
```

Per-page frontmatter:

```markdown
---
title: Q2 review
parser: pandoc
---
```

Precedence is frontmatter > rule > vault default > ztl default
(`commonmark`). See CON-3306.

If you run Pandoc hooks over some pages but not others, ztl's
mixed-parser diagnostic (REQ-3315) will flag any
parser-ambiguous syntax it finds under `ztl build`. Pass
`--strict-parsers` to fail the build on any such warning.

## Invocation contract

Per CON-3303 the adapter invokes each filter with:

```
argv:
  [<filter-binary>, "html"]        # target format; or user-provided from `args`

env:
  PANDOC_VERSION           = "3.1.12.1"          # detected by the adapter probe
  PANDOC_API_VERSION       = "1,23,1"            # pandoc-types API version
  PANDOC_READER_OPTIONS    = "<json blob>"       # default or user-provided
  PANDOC_WRITER_OPTIONS    = "<json blob>"       # default for html
  PANDOC_SCRIPT_FILE       = "<abs path to exec>"

stdin:   pandoc-types AST, JSON, UTF-8
stdout:  pandoc-types AST, JSON, UTF-8
stderr:  free-form; ztl forwards under --verbose

persistent mode: line-delimited JSON per SPEC-032 CON-3201.
```

This is byte-identical to Pandoc's own filter contract (see
<https://pandoc.org/filters.html>). Filters that work under
`pandoc --filter <name>` work under ztl without modification, and
cannot distinguish ztl-run from pandoc-run invocations.

### Translation boundary

ztl-specific concepts are preserved across filter invocations via
marker conventions (REQ-3307):

| ztl-ext node | pandoc-types shape                                            |
|---------------|---------------------------------------------------------------|
| `Wikilink`    | `Span` with class `ztl-wikilink`; attrs `target`, `alias`, `heading`, `block_id`. |
| `Embed`       | `Span` with class `ztl-embed`; attrs `target`, `heading`, `block_id`. |
| `SplBlock`    | `CodeBlock` with language `spl` (both directions).            |
| `FrontMatter` | Pandoc `Meta` map at document root.                           |
| Source position | `sourcepos` attribute on the enclosing Pandoc node.         |

A filter that strips a wikilink span (by class match or attr
access) is caught by the round-trip preservation check defined in
SPEC-032 CON-3221: ztl counts node types before and after,
compares against the plugin's declared `preserves` list in the
matrix (or the manifest's own `[contract]` table), and emits a
`contract_violation` diagnostic naming the dropped node types.

The full node-type mapping is auto-generated at
[`docs/ecosystems/pandoc-translation.md`](./pandoc-translation.md)
from the translator source (CON-3307; generated alongside the
ast-reference-check gate).

## Known-working filters

The v1 compatibility matrix ships three seed entries in
`tools/ztl-ecosystem-matrix.toml`. All land at
`tier = "experimental"` — documenting the shape of a canonical
render without a live golden-HTML CI assertion yet. Promotion to
`partial` and `supported` is gated by the REQ-3311 tier checklist
(see `docs/ecosystems/matrix-contribution.md` when that lands).

### `pandoc-crossref`

- **Version range:** `>=0.3.14 <0.4` (tracks pandoc 3.x / pandoc-types 1.23.x).
- **Fixture:** `tests/ecosystem-fixtures/pandoc/pandoc-crossref/`.
- **Upstream:** <https://github.com/lierdakil/pandoc-crossref>.

Adds numbered cross-references for figures, equations, and tables.
Example manifest:

```toml
# .ztl/hooks/transform.d/crossref.toml
stage    = "transform"
ecosystem = "pandoc"
exec     = "pandoc-crossref"
mode     = "persistent"
```

Known limitations at experimental tier:

1. pandoc-types major version must match the installed pandoc binary; mismatches surface via REQ-3314 plugin-version-drift detection.
2. Round-trip preservation of ztl Wikilink/Embed/SPL markers is inferred from CON-3221 defaults but not yet verified by a live golden run.
3. Crossref may inject caption spans that inflate node counts beyond the default REQ-3224 expansion-bound advisory.

### `pandoc-citeproc` (legacy)

- **Version range:** `>=0.17 <0.18`.
- **Fixture:** `tests/ecosystem-fixtures/pandoc/pandoc-citeproc/`.
- **Upstream:** <https://github.com/jgm/pandoc-citeproc> (archived 2022).

Legacy external filter for citation processing. **New deployments
should use native-mode `--citeproc` instead** — the filter is
retained in the matrix only for users on pandoc <2.11 or a legacy
Makefile pipeline. Tier is capped at `experimental` for the
lifetime of v1; promotion is not planned.

```toml
# Preferred: native mode
stage     = "pre-parse"
ecosystem = "pandoc"
mode      = "native"
args      = ["--citeproc", "--bibliography=references.bib"]
```

### `pantable`

- **Version range:** `>=0.16 <0.17`.
- **Fixture:** `tests/ecosystem-fixtures/pandoc/pantable/`.
- **Upstream:** <https://github.com/ickc/pantable>.

CSV / dataframe → pandoc Table filter. Shipped as a Python module,
so the adapter invokes it via `python3 -m pantable` — REQ-3313
runtime probe checks for the Python module, not a standalone
binary.

```toml
# .ztl/hooks/transform.d/pantable.toml
stage    = "transform"
ecosystem = "pandoc"
exec     = "pantable"            # resolves to `python3 -m pantable`
```

Known limitations at experimental tier:

1. Tables produced may carry `colspecs` with non-default alignment that round-trip through ztl-ext as plain Table nodes; alignment fidelity is best-effort.
2. Promotion to `partial` blocks on a golden-HTML fixture confirming CSV-cell content survives the translate → filter → translate-back cycle without markup corruption.

### Other filters

Filters not in the matrix run too — ztl does not reject unknown
filter binaries — but emit a one-shot warning at first use:

```
[ztl] ecosystem pandoc: <plugin> not in matrix; behavioural
       contract unknown, no preservation checks active
```

To get preservation checks for an unlisted filter, declare a
`[contract]` table on the manifest (SPEC-032 REQ-3224):

```toml
stage    = "transform"
ecosystem = "pandoc"
exec     = "pandoc-plantuml"

[contract]
preserves   = ["Wikilink", "Embed", "SplBlock", "Link", "Image"]
idempotent  = true
```

Or contribute a matrix entry; see the tier-promotion checklist in
SPEC-033 REQ-3311.

## Using Quarto filters

Quarto layers on Pandoc, so if ztl runs Pandoc correctly, most
filters distributed as Quarto extensions work through the same
Pandoc adapter with no Quarto-specific code path.

The integration surface is the Pandoc-filter contract itself —
SPEC-033 §13 Q6 resolves this as the canonical documented path
rather than a distinct `ecosystem = "quarto"` adapter.

### Quarto Lua filters

Most Quarto extensions ship as Lua filters. Install the extension
with `quarto install extension …`, then point ztl's manifest at
the resolved Lua file:

```toml
# .ztl/hooks/transform.d/quarto-diagram.toml
stage       = "transform"
ecosystem   = "pandoc"
mode        = "native"
lua_filter  = "_extensions/quarto-ext/diagram/diagram.lua"
```

Resolve the path by running `quarto list extensions` in the vault
root — Quarto writes extension files under `_extensions/<org>/<name>/`.

### Quarto's own processing pipeline

Some Quarto extensions are not Lua filters but *shortcodes* or
pre-render scripts that depend on the full `quarto render` run
(R/Python execution, YAML header massaging, Bootstrap theming).
Those features do not run under the filter contract and are out of
scope for the Pandoc adapter. If you need the full Quarto
pipeline, run `quarto render` separately and serve its HTML output
outside ztl.

### What round-trips

Quarto-produced Lua filters behave like any other Pandoc filter:
wikilinks, embeds, and SPL blocks round-trip via the REQ-3307
marker conventions. Filters that unwrap spans indiscriminately
will strip those markers; the REQ-3224 preservation check catches
that in the build-summary diagnostics.

## `ztl ecosystem check` walkthrough

`ztl ecosystem check` probes every compiled-in adapter and reports
detection state, detected version, and plugin availability
(REQ-3310, CON-3310).

```
$ ztl ecosystem check
ECOSYSTEM  STATUS        VERSION              PLUGINS CONFIGURED   PLUGINS AVAILABLE
pandoc     detected      3.1.12.1             2                    2
mdbook     missing       (binary absent)      0                    0
remark     detected      node 20.10.0         3                    3 (./node_modules)
```

Per-ecosystem rows report:

- **STATUS** — `detected`, `missing` (runtime not found), or `wrong-version` (runtime present but below the adapter's minimum).
- **VERSION** — detected binary version string, or `"(binary absent)"` when missing.
- **PLUGINS CONFIGURED** — count of hooks for this ecosystem declared in `.ztl/hooks/` + theme-bundled manifests.
- **PLUGINS AVAILABLE** — count of those configured plugins actually resolvable on this machine.

### Zero-configured state

Fresh vault with no hooks declared:

```
$ ztl ecosystem check
ECOSYSTEM  STATUS        VERSION              PLUGINS CONFIGURED   PLUGINS AVAILABLE
pandoc     detected      3.1.12.1             0                    0
mdbook     detected      0.4.37               0                    0
remark     detected      node 20.10.0         0                    0

No ecosystem hooks configured in this vault.
To enable an ecosystem, add a manifest under .ztl/hooks/:
  https://ztl.codeberg.page/docs/ecosystems/
```

Exit code is 0 in the zero-configured state regardless of which
runtimes are detected.

### Exit codes

- `0` — all *configured* ecosystems are available (or none are configured).
- non-zero — at least one configured ecosystem is missing its runtime.

Under `ztl build`, a missing runtime disables the affected hook
and continues the build with a `RuntimeAbsence` diagnostic; it is
not a hard failure unless `--ecosystem-required=pandoc` is passed
(REQ-3313), which is the CI gate mode.

### `--json` output

Machine-readable form suitable for CI pre-flight:

```json
[
  { "id": "pandoc", "status": "detected", "version": "3.1.12.1",
    "configured": 2, "available_plugins": 2 },
  { "id": "mdbook", "status": "missing", "version": null,
    "configured": 0, "available_plugins": 0 },
  { "id": "remark", "status": "detected", "version": "node 20.10.0",
    "configured": 3, "available_plugins": 3 }
]
```

### Plugin-version drift

At probe time the adapter invokes each configured filter's
`--version` and compares against the `version_range` in
`tools/ztl-ecosystem-matrix.toml` (REQ-3314):

- **Exact match** — silent.
- **Minor drift** (same major, observed minor ≥ tested) — log `[ztl] ecosystem pandoc: pandoc-crossref v0.3.16 is newer than last-tested v0.3.14; proceeding` once per session. The hook still runs.
- **Incompatible** (different major, or below the tested range) — hook disabled with a `plugin_version_incompatible` diagnostic pointing at the matrix entry.

## Troubleshooting

### `pandoc` binary not found

```
[ztl] ecosystem pandoc: runtime missing
       Install pandoc 2.11 or later:
         brew install pandoc   # macOS
         apt install pandoc    # Debian / Ubuntu
```

The build continues with the hook disabled. Pass
`--ecosystem-required=pandoc` to fail fast instead.

### Filter binary not found

The adapter reports the filter path it tried and a hint:

```
[ztl] hook transform/crossref: filter "pandoc-crossref" not found on $PATH
       Install via: cabal install pandoc-crossref
       (matrix entry: tools/ztl-ecosystem-matrix.toml)
```

### Wikilinks / embeds missing from rendered output

Filter is stripping the `ztl-wikilink` / `ztl-embed` span markers.
ztl catches this via preservation checks and emits:

```
[ztl] contract violation: pandoc-crossref dropped 4 Wikilink nodes
       on projects/q2-review.md
       Hint: add `preserves = ["Wikilink", "Embed", "SplBlock"]`
             to the hook's [contract] table, or file an issue with
             the filter author.
```

If the loss is expected (e.g. a filter that intentionally
transforms wikilinks into other shapes), narrow the `preserves`
list in the manifest's `[contract]` table.

### Mixed-parser warnings

Running pandoc hooks on some pages but not others, in a vault
containing parser-ambiguous syntax (curly-brace attribute syntax,
extended table dialects, fenced-div shortcuts):

```
[ztl] warning: mixed parsers in vault with ambiguous syntax:
         papers/draft-a.md:42   (parser: pandoc)    :::note
         notes/idea-7.md:5      (parser: commonmark) :::note
       Consider unifying on one parser, or inspecting the flagged
       pages for intended behaviour.
```

Either set a vault-wide `[parse] default` or accept the warning as
advisory. Pass `--strict-parsers` to turn it into a build error.

### Transient filter crashes

Pandoc filter crashes mid-build revert the page fragment per
SPEC-032 REQ-3207 and record a `hook_failure` diagnostic. ztl
does not retry in v1 — if crashes look transient (GC pauses,
transient OOM), that is noted as SPEC-033 §13 Q10 and deferred
pending field data.

## See also

- [`docs/ecosystems/mdbook.md`](./mdbook.md) — companion guide for mdBook preprocessors.
- [`docs/ecosystems/remark.md`](./remark.md) — companion guide for remark plugins.
- [`docs/hook-security.md`](../hook-security.md) — env allowlist and message-size caps that apply to every persistent-mode subprocess, including Pandoc filters.
- [SPEC-033](../../specs/SPEC-033.md) — normative ecosystem-bridges specification.
- [SPEC-032](../../specs/SPEC-032.md) — normative hook-contract specification (contracts, selectors, preservation checks).
