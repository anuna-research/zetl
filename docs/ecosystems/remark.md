# remark ecosystem guide

The remark adapter lets ztl run
[unified / remark](https://unifiedjs.com) plugins (the
`remark-<name>` package convention on npm) over vault pages as part
of the normal hook pipeline. Plugins run inside a long-lived Node
subprocess — the *harness* — which ztl ships embedded in its own
binary; users bring Node and their `node_modules`. This guide
covers install, configuration, the harness architecture, the set of
plugins tracked in the compatibility matrix, and troubleshooting.

The authoritative surface for the remark adapter is
[SPEC-033 REQ-3305 / CON-3305 / ADR-3304](../../specs/SPEC-033.md);
this doc restates that material in a user-facing shape and is kept
in sync with the shipped `ecosystem-remark` feature flag.

<!-- toc -->
- [Install](#install)
- [Configuration](#configuration)
- [Harness architecture](#harness-architecture)
- [Translation boundary](#translation-boundary)
- [Known-working plugins](#known-working-plugins)
- [`ztl ecosystem check` walkthrough](#ztl-ecosystem-check-walkthrough)
- [Troubleshooting](#troubleshooting)

## Install

The remark adapter needs two things at runtime:

1. A `node` binary at version **18 or later** on `$PATH`. 18 is the
   first LTS with stable ESM `import()` and `fetch`; the minimum is
   pinned in `src/ecosystems/registry.rs`.
2. remark/unified plugins resolvable from the vault. ztl runs
   Node's own resolver — it walks up from the vault root looking
   for `node_modules`.

Install Node with your platform's package manager or a version
manager:

```sh
# macOS
brew install node

# Debian / Ubuntu
apt install nodejs npm

# nvm (any platform)
nvm install --lts
```

Then install plugins into the vault:

```sh
cd /path/to/vault
npm init -y                    # if no package.json exists yet
npm install --save-dev remark-gfm remark-math remark-directive
```

ztl does not call `npm install` for you — it defers to npm and
only reports at build time when a configured plugin cannot be
resolved (SPEC-033 §13 Q4). Automatic `npm install` is
explicitly out of scope for v1 (SPEC-033 §1.3).

The remark adapter is compiled into default ztl builds via the
`ecosystem-remark` cargo feature. To build ztl without it:

```sh
cargo build --no-default-features --features "<your-other-flags>"
```

Hooks declaring `ecosystem = "remark"` in a binary compiled without
the feature fail fast with a `RuntimeAbsence` diagnostic pointing
at the matrix entry (SPEC-032 CON-3225).

### Harness: bundled vs bring-your-own

The Node subprocess loads a ztl-provided script, `ztl-remark-
harness.mjs`, that imports `unified`, accepts JSON-RPC-like
messages on stdin, and replies on stdout (CON-3305). That script
is **bundled with ztl**: it is embedded into the release binary at
compile time (`include_str!("../../_static/ztl-remark-harness.mjs")`)
and written to a short-lived temp file inside the vault on first
use, so Node's ESM loader can find it alongside the project's
`node_modules`. There is nothing to install: if you have `node` on
`$PATH` and plugins in `node_modules`, ztl handles the rest.

If you need a custom harness — to add plugin caching, emit
telemetry, or resolve plugins from a non-standard location — the
public source is
[`_static/ztl-remark-harness.mjs`](../../_static/ztl-remark-harness.mjs)
in the ztl repo. Per CON-3305, "users may supply their own if
they need custom plugin resolution or caching." Vendor a copy into
your vault (for example at `.ztl/remark-harness.mjs`), make your
edits, and keep it in sync with the stanza block at the top — the
harness protocol is stable within a ztl minor release. A
first-class swap-in mechanism (`harness_path = "..."` on a
manifest, or a vault-level config knob) is a v1.1 candidate; until
then, users wanting BYO run their forked script directly and wire
it into the pipeline via a SPEC-032 persistent-mode hook rather
than through the remark adapter.

The bundled-harness version string is tracked at
`src/ecosystems/remark.rs::HARNESS_VERSION` and is echoed in the
ready-banner line logged at startup — pin against that if your
CI asserts a specific harness revision.

## Configuration

remark hooks live under `.ztl/hooks/transform.d/` — the remark
adapter runs at the `transform` stage only. Plugins that hook into
remark at the parse layer (`mdast-util-*`, `remark-parse`
extensions like `remark-wiki-link`) are out of scope for v1; full
remark-as-parser adoption is deferred to v1.1 (SPEC-033 §1.3).
One TOML manifest per hook, named by the default-extension-id
convention (see SPEC-032 REQ-3217).

### Basic manifest

```toml
# .ztl/hooks/transform.d/gfm.toml
stage     = "transform"
ecosystem = "remark"
package   = "remark-gfm"
version   = ">=4 <5"           # optional; see REQ-3314 drift detection
```

With `package = "remark-gfm"` and a `./node_modules/remark-gfm`
install, ztl loads the plugin into the shared harness on first
use and applies it to every page that passes the hook's selector.

### Passing plugin options

remark plugins accept an options object as their second argument to
`.use()` (for example `remark-math` takes `{ singleDollarTextMath:
true }`). ztl's manifest mirrors that:

```toml
stage     = "transform"
ecosystem = "remark"
package   = "remark-math"

[options]
singleDollarTextMath = true
```

The `[options]` table is passed verbatim to the harness as the
plugin's options argument; remark's own documentation is the
source of truth for per-plugin option schemas.

### Isolation modes

A manifest field `isolation = "shared" | "fresh-context"` controls
how the harness is shared across invocations:

- `"shared"` (default) — one long-lived Node subprocess per ztl
  build. Plugin imports are cached via Node's own ESM module
  cache; subsequent pages reuse the loaded plugin. This is the
  perf path: cold-start cost amortises across the whole build
  (NFR-3301 targets ≤ 200 ms P95 cold-start; NFR-3303 targets
  ≤ 30 ms P95 per-page round-trip for a 500-node mdast).
- `"fresh-context"` — a new Node subprocess is spawned per
  invocation and shut down afterwards. Isolates plugins from each
  other (plugin A cannot monkey-patch globals that plugin B later
  relies on) at the cost of per-invocation startup (≈ 100–200 ms).

```toml
stage     = "transform"
ecosystem = "remark"
package   = "some-untrusted-plugin"
isolation = "fresh-context"
```

See [Security](#harness-security-posture) below for when
`fresh-context` is worth the cost.

### Manifest fields

Base hook manifest fields (`stage`, `timeout_ms`, `ast_type`,
`select.*`, `before`, `after`, `optional`, `extension_id`) are
documented in SPEC-032 REQ-3217. remark-specific fields per
SPEC-033 REQ-3312:

| Field       | Type                | Use                                                                 |
|-------------|---------------------|---------------------------------------------------------------------|
| `package`   | string              | npm package name to `import()`. Required.                           |
| `version`   | string              | npm-style semver range. Optional; compared against the installed package's `package.json` at probe time (REQ-3314). |
| `options`   | table               | Options object passed as the plugin's second argument.              |
| `isolation` | `"shared"` \| `"fresh-context"` | Harness lifetime. Defaults to `"shared"`.               |
| `ast_type`  | `"mdast-ext"`       | Defaults to `"mdast-ext"` for remark hooks; rarely overridden.      |

Non-remark fields (e.g. `exec` from the Pandoc adapter, `scope`
from the mdBook adapter) on a remark manifest are rejected at
parse time (REQ-3312 cross-ecosystem field validation).

## Harness architecture

The adapter spawns `node <harness>.mjs` in the vault root, which
means Node's `node_modules` walk resolves plugins from the vault's
install. On startup the harness emits an out-of-band ready banner
so ztl can confirm the pipe is live before sending real requests:

```json
{"type":"ready","harness_version":"1.0.0","node_version":"v20.10.0","unified_available":true}
```

`unified_available: false` plus an `unified_import_error` string
means the harness started but `import("unified")` failed — usually
"no `unified` in `node_modules`". ztl surfaces that as a typed
diagnostic with a `npm install unified` hint.

### Protocol (CON-3305)

Line-delimited JSON over the subprocess's stdin/stdout; each
message carries a caller-allocated `id` that the response echoes:

```
ztl → harness:  {"id":1,"type":"load_plugin","package":"remark-gfm","options":{}}
harness → ztl:  {"id":1,"type":"load_result","ok":true,"plugin_id":"rp_1"}

ztl → harness:  {"id":2,"type":"apply","plugin_id":"rp_1","ast":{…mdast…}}
harness → ztl:  {"id":2,"type":"apply_result","ok":true,"ast":{…mdast…}}

ztl → harness:  {"id":3,"type":"shutdown"}
harness → ztl:  {"id":3,"type":"shutdown_result","ok":true}
harness exits 0
```

Errors are reported in-band (`{"ok":false,"error":"…"}` echoing the
request `id`); the harness never exits on a per-message failure.
Uncaught exceptions inside a plugin surface as an `apply_result`
with `ok: false` and stack-trace text, converted upstream into a
`hook_failure` diagnostic (SPEC-032 REQ-3207).

The full schema plus startup banner fields are hand-documented in
the harness source at
[`_static/ztl-remark-harness.mjs`](../../_static/ztl-remark-harness.mjs).

### Security posture

- Plugin code runs inside Node's single-threaded event loop with
  the user's UID/GID; it can touch the filesystem and network
  through Node's APIs. If you `npm install` a malicious plugin,
  ztl inherits the risk — as would any other remark host.
- Supply-chain risk is `npm`'s. ztl does not run `npm audit`;
  users should.
- Harness-poisoning: a long-lived `"shared"` harness lets
  plugin A monkey-patch globals (Prototype, JSON, Date) that
  plugin B later depends on. If you are running untrusted
  plugins, set `isolation = "fresh-context"` (SPEC-033 §10).
- Env-var allowlist, stderr byte-cap, and per-message size caps
  inherit from SPEC-032 §10 / `docs/hook-security.md` — the
  harness subprocess runs under the same restrictions as any
  other persistent-mode hook.

## Translation boundary

ztl's internal AST is `ztl-ext`; the remark adapter translates
ztl-ext ↔ mdast at the hook boundary. mdast is CommonMark-aligned
so translation loss is lower than with pandoc-types.

Marker conventions for ztl concepts without native mdast
equivalents (SPEC-033 REQ-3308):

| ztl-ext node   | mdast shape                                                         |
|-----------------|---------------------------------------------------------------------|
| `Wikilink`      | Custom node `{type: "wikilink", target, alias, heading, block_id}` (the `remark-wiki-link` convention). |
| `Embed`         | Custom node `{type: "embed", target, heading, block_id}`.           |
| `SplBlock`      | `code` node with `lang: "spl"`.                                     |
| `FrontMatter`   | `yaml` node at document root (the `remark-frontmatter` convention). |
| Source position | Native mdast `position` object — direct mapping.                    |

A plugin that strips custom nodes (by type match) is caught by the
round-trip preservation check defined in SPEC-032 CON-3221: ztl
counts node types before and after, compares against the plugin's
declared `preserves` list in the matrix (or the manifest's own
`[contract]` table), and emits a `contract_violation` diagnostic
naming the dropped node types.

mdast node types ztl doesn't natively represent (`definition`,
`linkReference`, custom extension nodes) pass through as opaque
unknown-type nodes via ztl-ext's forward-compat mechanism (the
AST schema accepts unknown nodes with a warning).

The full node-type mapping is auto-generated at
[`docs/ecosystems/mdast-translation.md`](./mdast-translation.md)
from the translator source (CON-3308; generated alongside the
ast-reference-check gate).

## Known-working plugins

The v1 compatibility matrix ships three seed entries in
`tools/ztl-ecosystem-matrix.toml`. All land at
`tier = "experimental"` — documenting the shape of a canonical
render without a live golden-HTML CI assertion yet. Promotion to
`partial` and `supported` is gated by the REQ-3311 tier checklist
(see `docs/ecosystems/matrix-contribution.md` when that lands).
SPEC-033 REQ-3305 calls out two further seed plugins
(`remark-frontmatter`, `remark-smartypants`) that will land in the
matrix as part of the tier-promotion sweep.

### `remark-gfm`

- **Version range:** `>=4.0 <5`.
- **Fixture:** `tests/ecosystem-fixtures/remark/remark-gfm/`.
- **Upstream:** <https://github.com/remarkjs/remark-gfm>.

Enables GitHub Flavored Markdown at the remark-parse layer: pipe
tables, task lists, strikethrough, autolink literals, and
footnotes.

```toml
# .ztl/hooks/transform.d/gfm.toml
stage     = "transform"
ecosystem = "remark"
package   = "remark-gfm"
```

Known limitations at experimental tier:

1. Frontmatter stripping is the caller's responsibility — remark-gfm does not ship a frontmatter parser. ztl hands the body over after its own frontmatter split, so a manifest that skips that split would see the YAML block echoed as a paragraph.
2. Task-list DOM shape drifted between v3 (`<li>` only) and v4 (`.contains-task-list` on `<ul>`, `.task-list-item` on `<li>`); the fixture targets v4.
3. Footnote `id` attributes embed the page heading; round-tripping a ztl `Wikilink` that targets a footnoted page is best-effort and not yet gated.

### `remark-math`

- **Version range:** `>=6.0 <7`.
- **Fixture:** `tests/ecosystem-fixtures/remark/remark-math/`.
- **Upstream:** <https://github.com/remarkjs/remark-math>.

Parses `$...$` inline math and `$$...$$` display math into
dedicated mdast `inlineMath` / `math` nodes.

```toml
# .ztl/hooks/transform.d/math.toml
stage     = "transform"
ecosystem = "remark"
package   = "remark-math"
```

Known limitations at experimental tier:

1. remark-math is purely a *parser* — actual glyph rendering requires a companion `rehype-katex` or `rehype-mathjax` stage. Users pairing ztl with KaTeX also need to serve KaTeX CSS from the theme.
2. Upstream's last release is 2023-09-19; the scan classifies it as "abandoned". The plugin still works under unified v11 but promotion above `experimental` is blocked on a fresh upstream release or a ztl-side fork decision.
3. Dollar-sign escape handling (`\$`) is inherited from `micromark-extension-math` and has edge cases around adjacent punctuation; the canary fixture deliberately avoids these.

### `remark-directive`

- **Version range:** `>=3.0 <4`.
- **Fixture:** `tests/ecosystem-fixtures/remark/remark-directive/`.
- **Upstream:** <https://github.com/remarkjs/remark-directive>.

Adds generic directive syntax — `:::name{attrs}` container form,
`::name[label]{attrs}` leaf form, `:name[label]{attrs}` inline
form — at the remark-parse layer.

```toml
# .ztl/hooks/transform.d/directive.toml
stage     = "transform"
ecosystem = "remark"
package   = "remark-directive"
```

Known limitations at experimental tier:

1. remark-directive on its own emits `containerDirective` / `leafDirective` / `textDirective` mdast nodes that remark-rehype renders as empty `<div>` / `<span>` placeholders — a caller-defined `map-to-hast` transform is required for meaningful HTML. The canary fixture encodes the canonical `div.directive.directive-<name>` + inline-name-as-tag mapping.
2. Authors targeting MDX/Astro sites that already consume directives should pair this plugin with the same hast-mapping transform their existing pipeline uses, not the canary's, to avoid class-name drift between ztl-built and MDX-built pages.
3. No preservation gate on user-defined directive names; if a directive name collides with an HTML element (`:::table`), behaviour depends entirely on the caller's map-to-hast transform.

### Other plugins

Plugins not in the matrix run too — ztl does not reject unknown
packages — but emit a one-shot warning at first use:

```
[ztl] ecosystem remark: <package> not in matrix; behavioural
       contract unknown, no preservation checks active
```

To get preservation checks for an unlisted plugin, declare a
`[contract]` table on the manifest (SPEC-032 REQ-3224):

```toml
stage     = "transform"
ecosystem = "remark"
package   = "remark-smartypants"

[contract]
preserves  = ["Wikilink", "Embed", "SplBlock", "Link", "Image"]
idempotent = true
```

Or contribute a matrix entry; see the tier-promotion checklist in
SPEC-033 REQ-3311.

### rehype plugins

unified composes remark (mdast) with rehype (hast), so a pipeline
that runs rehype plugins downstream of remark is the idiomatic way
to reach HTML-level transforms. v1's remark adapter stays on the
mdast side of the pipeline — the harness loads plugins via
`.use()` on a `unified().use(remarkParse)...` chain, which means
rehype plugins chained via `remark-rehype` work as long as they
run within a single `package =` invocation. Putting rehype at a
separate hook manifest requires v1.1 adapter support for the hast
AST and is tracked in the roadmap.

## `ztl ecosystem check` walkthrough

`ztl ecosystem check` probes every compiled-in adapter and reports
detection state, detected version, and plugin availability
(REQ-3310, CON-3310).

```
$ ztl ecosystem check
ECOSYSTEM  STATUS        VERSION              PLUGINS CONFIGURED   PLUGINS AVAILABLE
pandoc     detected      3.1.12.1             0                    0
mdbook     missing       (binary absent)      0                    0
remark     detected      node 20.10.0         3                    3 (./node_modules)
```

For remark, per-row fields report:

- **STATUS** — `detected` (node 18+ on `$PATH`), `missing` (node absent), or `wrong-version` (node below 18).
- **VERSION** — detected `node --version` output.
- **PLUGINS CONFIGURED** — count of `.ztl/hooks/` manifests declaring `ecosystem = "remark"`.
- **PLUGINS AVAILABLE** — count of those configured packages resolvable from the vault's `node_modules`.

### Zero-configured state

Fresh vault with Node installed but no remark hooks:

```
$ ztl ecosystem check
ECOSYSTEM  STATUS        VERSION              PLUGINS CONFIGURED   PLUGINS AVAILABLE
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
not a hard failure unless `--ecosystem-required=remark` is passed
(REQ-3313), which is the CI gate mode.

### Plugin-version drift

At probe time the adapter reads each configured plugin's installed
`package.json` version and compares against the `version_range` in
`tools/ztl-ecosystem-matrix.toml` (REQ-3314):

- **Exact match** — silent.
- **Minor drift** (same major, observed minor ≥ tested) — log `[ztl] ecosystem remark: remark-gfm v4.0.2 is newer than last-tested v4.0.1; proceeding` once per session. The hook still runs.
- **Incompatible** (different major, or below the tested range) — hook disabled with a `plugin_version_incompatible` diagnostic pointing at the matrix entry.

## Troubleshooting

### `node` binary not found

```
[ztl] ecosystem remark: runtime missing
       Install node 18 or later:
         brew install node                 # macOS
         apt install nodejs npm            # Debian / Ubuntu
         nvm install --lts                 # nvm
```

The build continues with the hook disabled. Pass
`--ecosystem-required=remark` to fail fast instead.

### Package not installed

The adapter reports the package and plugin-resolution root:

```
[ztl] hook transform/gfm: remark package "remark-gfm" not found
       Looked in: /path/to/vault/node_modules
       Install with: npm install --save-dev remark-gfm
```

If plugins are installed at a non-default location, ztl picks the
first ancestor of the vault root containing a `node_modules/` —
`npm install` from the vault root is the simplest fix.

### Harness failed to start

```
[ztl] ecosystem remark: harness startup failed
       import("unified") failed: Cannot find module 'unified'
       Install with: npm install --save-dev unified
```

The harness imports `unified` on startup (CON-3305). Installing
any remark plugin normally pulls unified in transitively, but a
fresh `npm install remark-gfm` on a machine with a pre-existing
partial install may miss it. `npm install unified` resolves this.

If the banner times out entirely (10 s), ztl reports:

```
[ztl] ecosystem remark: harness banner timeout after 10s
       The harness script may be broken; check for errors on
       stderr. Run `node _static/ztl-remark-harness.mjs` directly
       to reproduce.
```

This usually means Node itself is misconfigured (missing `--harmony-*`
flags on very old versions) or the harness file was corrupted by a
local override. Removing any vendored harness and re-running falls
back to the bundled copy.

### Wikilinks / embeds missing from rendered output

A plugin is stripping the custom `wikilink` / `embed` mdast nodes.
ztl catches this via preservation checks and emits:

```
[ztl] contract violation: remark-smartypants dropped 4 wikilink nodes
       on projects/q2-review.md
       Hint: add `preserves = ["Wikilink", "Embed", "SplBlock"]`
             to the hook's [contract] table, or file an issue with
             the plugin author.
```

Plugins that traverse mdast via `unist-util-visit` and match only
standard node types usually pass through custom nodes unchanged;
plugins that build a new tree from scratch (like some
`remark-rehype` hast converters) are the typical offenders.

### Plugin crashes mid-build

remark plugin crashes revert the page fragment per SPEC-032
REQ-3207 and record a `hook_failure` diagnostic carrying the
JS stack trace. ztl does not retry in v1. If a plugin crashes
non-deterministically (shared mutable state across pages),
try switching the manifest to `isolation = "fresh-context"` to
rule out harness-poisoning (SPEC-033 §10).

### Harness out-of-memory

The Node subprocess is capped at 256 MiB by default (SPEC-033
§6.4). Plugins that load large corpora into memory (citation
databases, image pipelines) can exceed this on big vaults. The
harness is respawned on OOM with a `subprocess_restart` counter
increment; if the restart loop itself runs three times in a build,
ztl gives up on the affected hook and reports a `hook_failure`.
The workaround is to move the heavy lifting into a one-shot
pre-build script, or to split the plugin's work across a smaller
selector.

## See also

- [`docs/ecosystems/pandoc.md`](./pandoc.md) — companion guide for Pandoc filters.
- [`docs/ecosystems/mdbook.md`](./mdbook.md) — companion guide for mdBook preprocessors.
- [`docs/hook-security.md`](../hook-security.md) — env allowlist and message-size caps that apply to every persistent-mode subprocess, including the remark harness.
- [`_static/ztl-remark-harness.mjs`](../../_static/ztl-remark-harness.mjs) — the bundled harness source (also the canonical BYO starting point).
- [SPEC-033](../../specs/SPEC-033.md) — normative ecosystem-bridges specification.
- [SPEC-032](../../specs/SPEC-032.md) — normative hook-contract specification (contracts, selectors, preservation checks).
