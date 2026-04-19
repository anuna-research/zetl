# mdBook ecosystem guide

The mdBook adapter lets zetl run
[mdBook](https://rust-lang.github.io/mdBook/) preprocessors
(the `mdbook-<name>` binary convention) over vault pages as part of
the pre-parse stage. Preprocessors are one-shot subprocesses that
read a JSON envelope on stdin and emit a transformed `Book` JSON on
stdout — the same contract they honour under `mdbook build`. This
guide covers install, configuration, the set of preprocessors
tracked in the compatibility matrix, and troubleshooting.

The authoritative surface for the mdBook adapter is
[SPEC-033 REQ-3304 / CON-3304 / REQ-3309 / CON-3309](../../specs/SPEC-033.md);
this doc restates that material in a user-facing shape and is kept
in sync with the shipped `ecosystem-mdbook` feature flag.

<!-- toc -->
- [Install](#install)
- [Configuration](#configuration)
- [Envelope shape (REQ-3309)](#envelope-shape-req-3309)
- [Invocation contract](#invocation-contract)
- [Scope (`page` vs `vault`)](#scope-page-vs-vault)
- [Translation boundary](#translation-boundary)
- [Known-working preprocessors](#known-working-preprocessors)
- [`zetl ecosystem check` walkthrough](#zetl-ecosystem-check-walkthrough)
- [Troubleshooting](#troubleshooting)

## Install

The mdBook adapter does **not** require the `mdbook` binary itself
on `$PATH` to function — preprocessors run independently and zetl
builds their stdin envelope synthetically (REQ-3309). Presence of
the `mdbook` binary is still *probed* so `zetl ecosystem check` can
report a version; its absence downgrades the status line to
advisory, not error.

Install any preprocessor with `cargo install`:

```sh
cargo install mdbook-mermaid
cargo install mdbook-toc
cargo install mdbook-katex
cargo install mdbook-admonish
```

zetl does not install preprocessors for you — it defers to `cargo`
and only reports at build time when a configured preprocessor
cannot be resolved (SPEC-033 §13 Q4).

If you want the ecosystem-level probe row to read `detected`
instead of `missing`, also install the mdbook CLI (≥ 0.4.0, pinned
in `src/ecosystems/registry.rs`):

```sh
# macOS / any platform with cargo
cargo install mdbook

# Debian / Ubuntu
apt install mdbook           # if packaged on your distro
```

The mdBook adapter is compiled into default zetl builds via the
`ecosystem-mdbook` cargo feature. To build zetl without it:

```sh
cargo build --no-default-features --features "<your-other-flags>"
```

Hooks declaring `ecosystem = "mdbook"` in a binary compiled without
the feature fail fast with a `RuntimeAbsence` diagnostic pointing
at the matrix entry (SPEC-032 CON-3225).

## Configuration

mdBook preprocessors live under `.zetl/hooks/pre-parse.d/` — the
mdBook adapter runs at the `pre-parse` stage only. Preprocessors
operate on raw Markdown text inside an mdBook `Chapter`, which
predates zetl's parse step. One TOML manifest per hook, named by
the default-extension-id convention (see SPEC-032 REQ-3217).

### Basic manifest

```toml
# .zetl/hooks/pre-parse.d/mermaid.toml
stage     = "pre-parse"
ecosystem = "mdbook"
exec      = "mdbook-mermaid"
```

With `exec = "mdbook-mermaid"` and the binary resolvable on
`$PATH`, zetl invokes it once per vault page at build time: first
`mdbook-mermaid supports html` as a probe, then a real run with the
synthetic envelope piped over stdin.

### Scope

```toml
stage     = "pre-parse"
ecosystem = "mdbook"
exec      = "mdbook-toc"
scope     = "vault"   # accepted; see "Scope" section below
```

`scope` defaults to `"page"` — one preprocessor call per page,
maximally parallelisable. `"vault"` is accepted for forward
compatibility but v1 still invokes once per page and surfaces an
advisory warning; whole-vault batching is deferred (see
[Scope](#scope-page-vs-vault)).

### Manifest fields

Base hook manifest fields (`stage`, `timeout_ms`, `ast_type`,
`select.*`, `before`, `after`, `optional`, `extension_id`) are
documented in SPEC-032 REQ-3217. mdBook-specific fields per
SPEC-033 REQ-3312:

| Field       | Type               | Use                                                                           |
|-------------|--------------------|-------------------------------------------------------------------------------|
| `exec`      | string             | Preprocessor binary name (conventionally `mdbook-<name>`). Required.          |
| `scope`     | `"page"` \| `"vault"` | Invocation cardinality. Defaults to `"page"`.                              |
| `ast_type`  | `"zetl-ext"`       | Fixed; pre-parse hooks operate on raw Markdown before AST construction.       |

Non-mdBook fields (e.g. `package` from the remark adapter, `mode`
from the Pandoc adapter) on an mdBook manifest are rejected at
parse time (REQ-3312 cross-ecosystem field validation).

### Envelope overrides

Under `[options]`, a manifest may override the two synthetic-context
fields that some preprocessors read for behaviour selection:

```toml
[options]
renderer       = "html"      # default; preprocessors that also support
                             # epub/linkcheck can be pointed at another
mdbook_version = "0.4.40"    # default; bump for preprocessors that
                             # version-gate their own behaviour
```

Defaults are the adapter's protocol-compatible minimums. The probe
(`<exec> supports html`) always runs with `renderer = "html"`
regardless of this override — an alternate renderer here only
affects the real run's envelope.

## Envelope shape (REQ-3309)

On every invocation, zetl writes a two-element JSON array to the
preprocessor's stdin:

```json
[
  {
    "root": "/path/to/vault",
    "config": {
      "book": {"title": "<vault.name>", "authors": [], "src": "."},
      "preprocessor": {}
    },
    "renderer": "html",
    "mdbook_version": "0.4.40"
  },
  {
    "sections": [
      {"Chapter": {
        "name": "<page.name>",
        "content": "<raw markdown>",
        "number": null,
        "sub_items": [],
        "path": "<page.slug>.md",
        "source_path": "<page.slug>.md",
        "parent_names": []
      }}
    ],
    "__non_exhaustive": null
  }
]
```

The canonical schema lives at
[`tools/zetl-mdbook-envelope-schema-v1.json`](../../tools/zetl-mdbook-envelope-schema-v1.json)
and is asserted against every constructed envelope in the test suite
(`test_3309_every_fixture_page_envelope_passes_schema_validation`).

The preprocessor writes the transformed `Book` (the second element,
without the surrounding context) to stdout. zetl reads the first
chapter's `content` field back out and feeds it into the pipeline's
next stage as transformed Markdown.

**Fidelity guarantees:**

- Envelope construction is a pure function
  ([`build_envelope_for_page`](../../src/ecosystems/mdbook.rs)); the
  input body is copied verbatim into `Chapter.content`.
- Extraction (`extract_chapter_content`) on the zetl-built envelope
  returns the input body byte-for-byte; this round-trip property is
  enforced by
  `test_3309_envelope_content_round_trip_is_byte_identical`.
- Inbound preprocessor responses are validated structurally before
  the adapter trusts them; a preprocessor that drops `Chapter` or
  emits a mis-typed field surfaces as a `malformed_output` failure
  rather than a silent content loss.

**Known flexibility in inbound validation:**

- The `__non_exhaustive: null` marker is required on *outgoing*
  envelopes (mdBook's own serde emits it) but optional on *inbound*
  responses, because preprocessors in non-Rust languages (Node,
  Python) typically don't reproduce the marker.
- `PartTitle` and `Separator` items in `book.sections` are accepted
  when present — zetl never emits them but preprocessors that
  restructure the book may.

## Invocation contract

mdBook preprocessors follow a two-call protocol per
[CON-3304](../../specs/SPEC-033.md#con-3304):

1. `<exec> supports html` — probe. Exit 0 = the preprocessor accepts
   the html renderer; exit non-zero = it refuses and the real run
   never spawns.
2. `<exec>` (no argv) — real run. Stdin = envelope above; stdout =
   transformed `Book` JSON.

```
argv (probe):    [<exec>, "supports", "html"]
argv (run):      [<exec>]
stdin (run):     envelope JSON (see above), UTF-8
stdout (run):    transformed Book JSON, UTF-8
stderr:          free-form; zetl forwards under --verbose
```

This is byte-identical to mdBook's own preprocessor contract (see
<https://rust-lang.github.io/mdBook/for_developers/preprocessors.html>).
Preprocessors that work under `mdbook build` work under zetl
without modification, and cannot distinguish zetl-run from
mdbook-run invocations.

The adapter bounds the probe to 5 seconds and the real run to the
manifest's declared `timeout_ms`. Binary-not-found, probe-failure,
non-zero exit, malformed JSON, and malformed envelope shape each map
to a typed `FailureReason` for the observability pipeline.

## Scope (`page` vs `vault`)

The manifest's `scope` field picks the invocation cardinality:

- `scope = "page"` (default): one preprocessor call per vault page;
  maximally parallelisable. The envelope always carries exactly one
  `Chapter`.
- `scope = "vault"`: accepted for forward compatibility, but v1
  still runs one call per page and surfaces a warning diagnostic.
  Whole-vault batching (for preprocessors like `mdbook-toc` that
  need to see sibling chapters) lands in a later phase per
  [CON-3309 "Vault-scope invocations — known semantic gap"](../../specs/SPEC-033.md#con-3309-mdbook-book-envelope-schema).

## Translation boundary

The mdBook adapter operates at the pre-parse stage on raw Markdown
text — there is no AST translation at the adapter boundary (REQ-3307
applies to transform-stage AST adapters, not pre-parse text
adapters). The envelope's `Chapter.content` is the page's raw
Markdown, and the preprocessor returns raw Markdown; zetl's own
parser runs afterward on whatever text the preprocessor emitted.

**What this means for zetl-ext features:**

- **Wikilinks** (`[[target]]`), **embeds** (`![[target]]`), and
  **SPL blocks** are raw text at the preprocessor boundary. A
  preprocessor that does pattern-specific rewrites on Markdown
  links or fenced blocks *may* modify or strip them; one that
  operates on a specific syntax (mermaid fences, admonish fences,
  math delimiters) leaves unrelated syntax alone.
- The preservation check for pre-parse hooks operates on the
  parsed AST *after* the pre-parse pipeline, comparing against the
  AST zetl would have built without the hook. Strip of a
  `[[wikilink]]` by an mdBook preprocessor surfaces as a
  `contract_violation` naming `Wikilink` in the diagnostic.
- Frontmatter is stripped by zetl before the envelope is built —
  preprocessors never see the YAML block and cannot read
  frontmatter for conditional behaviour. Mirror any frontmatter
  you want the preprocessor to see into page body or use the hook
  manifest's `select.frontmatter` guard instead.

## Known-working preprocessors

The v1 compatibility matrix ships four seed entries in
`tools/zetl-ecosystem-matrix.toml`. All land at
`tier = "experimental"` — documenting the shape of a canonical
render without a live golden-HTML CI assertion yet. Promotion to
`partial` requires wiring the fixtures into a green golden-HTML
runner that actually invokes `mdbook build` with the preprocessor
enabled (blocked on `task-eco-matrix`); promotion to `supported`
additionally requires declaring a `[plugin.contract]` sub-table
(preserves + idempotent) and CI double-run evidence per
TEST-3224-idempotent.

### `mdbook-mermaid`

- **Version range:** `>=0.14 <0.16`.
- **Fixture:** `tests/ecosystem-fixtures/mdbook/mdbook-mermaid/`.
- **Upstream:** <https://github.com/badboy/mdbook-mermaid>.

Rewrites ```` ```mermaid ```` fenced code blocks into
`<pre class="mermaid">` shells and injects the mermaid.js runtime
+ book-init CSS/JS assets via the HTML renderer.

```toml
# .zetl/hooks/pre-parse.d/mermaid.toml
stage     = "pre-parse"
ecosystem = "mdbook"
exec      = "mdbook-mermaid"
```

Known limitations at experimental tier:

1. Rendering is client-side — the preprocessor only swaps the fence; the actual SVG is produced by mermaid.js in the browser at page load. The golden-HTML gate can assert the shell shape, not the diagram.
2. The asset-injection path relies on mdBook's `renderer = ["html"]` output backend. Users running an alternative renderer see the shell markup but no runtime, which is cosmetically broken by design; the adapter surfaces this via `mdbook-mermaid install` guidance at probe time.
3. Round-trip preservation of zetl `Wikilink` / `Embed` / `SplBlock` markers inside a mermaid fence is undefined — the preprocessor treats fence bodies opaquely and zetl's SPL classifier fires first on ```` ```spl ````, so collisions are unlikely but untested.

### `mdbook-toc`

- **Version range:** `>=0.14 <0.15`.
- **Fixture:** `tests/ecosystem-fixtures/mdbook/mdbook-toc/`.
- **Upstream:** <https://github.com/badboy/mdbook-toc>.

Replaces the literal `<!-- toc -->` marker (or a configurable
alternate) with a generated `<ul>` list of in-chapter headings.

```toml
# .zetl/hooks/pre-parse.d/toc.toml
stage     = "pre-parse"
ecosystem = "mdbook"
exec      = "mdbook-toc"
```

Known limitations at experimental tier:

1. The marker is HTML-comment-based. zetl's frontmatter stripper runs first and the marker survives unmolested, but a user who wraps the marker inside a fenced code block sees it preserved literally — expected behaviour per upstream.
2. Heading-depth range is configurable via `[preprocessor.toc]` in `book.toml` (defaults `max-level = 4`); the canary fixture exercises only depths 1–2 to stay terse and version-stable.
3. Anchor slugs are computed by mdbook's own GitHub-style slugifier, not zetl's; a page carrying an Obsidian `^block-id` that mirrors an mdbook-generated anchor will collide silently. Promotion to `partial` requires a fixture exercising unicode heading slugs.

### `mdbook-katex`

- **Version range:** `>=0.9 <0.10`.
- **Fixture:** `tests/ecosystem-fixtures/mdbook/mdbook-katex/`.
- **Upstream:** <https://github.com/lzanini/mdbook-katex>.

Server-side renders `$...$` inline and `$$...$$` display math into
pre-rendered KaTeX HTML at preprocess time, avoiding a client-side
JS runtime.

```toml
# .zetl/hooks/pre-parse.d/katex.toml
stage     = "pre-parse"
ecosystem = "mdbook"
exec      = "mdbook-katex"
```

Known limitations at experimental tier:

1. The KaTeX output embeds both a `<math>` MathML subtree and an HTML-span fallback subtree inside a wrapping `<span class="katex">`; byte-stable golden-HTML comparison is brittle across KaTeX versions, so the canary fixture records the expected wrapper shape only and the full tree lands as an opt-in comparison mode under `task-eco-matrix`.
2. Users pairing this with the `smart_punctuation` `book.toml` option see stray curly-quote rewrites inside math delimiters; the canary fixture deliberately avoids strings that would trigger it.
3. Dollar-escape handling (`\$`) is inherited from the upstream tokenizer; escaped dollars adjacent to a literal `$...$` span have occasionally regressed upstream (see [lzanini/mdbook-katex#117](https://github.com/lzanini/mdbook-katex/issues/117)) and the fixture avoids that corner.

### `mdbook-admonish`

- **Version range:** `>=1.18 <2`.
- **Fixture:** `tests/ecosystem-fixtures/mdbook/mdbook-admonish/`.
- **Upstream:** <https://github.com/tommilligan/mdbook-admonish>.

Rewrites ```` ```admonish ```` fenced blocks (including the
`admonish note`, `admonish warning`, … flavoured opening lines)
into `<div class="admonition admonish admonish-<type>">` wrappers
with a title div and body div. Designated as the default upstream
backing for the SPEC-032 Callouts canonical extension per §13 Q1
resolution — the in-repo stub at
`tests/extension-fixtures/admonition/` defines the shared contract
and this matrix row points at the real plugin mdBook-backed builds
would wire up.

```toml
# .zetl/hooks/pre-parse.d/admonish.toml
stage     = "pre-parse"
ecosystem = "mdbook"
exec      = "mdbook-admonish"
```

Known limitations at experimental tier:

1. Class shape differs from the SPEC-032 Callouts canonical (`<div class="admonition <type>">`) — mdbook-admonish adds `admonish admonish-<type>` alongside. The canonical CSS targets `.admonition.<type>` which matches both shapes; theme overrides referencing the `.admonish-<type>` class break if the user swaps to the pandoc-admonition or Python-Markdown backing, so themes SHOULD target only the shared shape.
2. Title handling: Obsidian's `title:` header key vs mdbook-admonish's `[title]` bracket syntax. The CON-3212 selector accepts the Obsidian form and emits the shared output; this matrix row's fixture exercises the upstream `[title]` form so the golden-HTML runner can round-trip what the real binary produces.
3. Collapsible admonitions (`admonish collapsible.open note`) and custom anchor ids are a v1.x upstream feature not yet wired into the canonical extension contract — promotion to `partial` requires a decision on whether to surface those via Callouts frontmatter keys or leave them plugin-specific.

### Other preprocessors

Preprocessors not in the matrix run too — zetl does not reject
unknown `mdbook-*` binaries — but emit a one-shot warning at first
use:

```
[zetl] ecosystem mdbook: <exec> not in matrix; behavioural
       contract unknown, no preservation checks active
```

To get preservation checks for an unlisted preprocessor, declare a
`[contract]` table on the manifest (SPEC-032 REQ-3224):

```toml
stage     = "pre-parse"
ecosystem = "mdbook"
exec      = "mdbook-variables"

[contract]
preserves  = ["Wikilink", "Embed", "SplBlock"]
idempotent = true
```

Or contribute a matrix entry; see the tier-promotion checklist in
SPEC-033 REQ-3311.

## `zetl ecosystem check` walkthrough

`zetl ecosystem check` probes every compiled-in adapter and reports
detection state, detected version, and plugin availability
(REQ-3310, CON-3310).

```
$ zetl ecosystem check
ECOSYSTEM  STATUS        VERSION              PLUGINS CONFIGURED   PLUGINS AVAILABLE
pandoc     detected      3.1.12.1             0                    0
mdbook     detected      0.4.37               2                    2
remark     missing       (binary absent)      0                    0
```

For mdbook, per-row fields report:

- **STATUS** — `detected` (mdbook CLI ≥ 0.4.0 on `$PATH`), `missing` (mdbook CLI absent — advisory only; preprocessors still run), or `wrong-version` (mdbook CLI below 0.4.0).
- **VERSION** — detected `mdbook --version` output.
- **PLUGINS CONFIGURED** — count of `.zetl/hooks/pre-parse.d/` manifests declaring `ecosystem = "mdbook"`.
- **PLUGINS AVAILABLE** — count of those configured `exec` binaries actually resolvable on `$PATH`. The detection pass also scans `$PATH` for any `mdbook-*` binaries and lists them under the entry's plugin-discovery hint, independent of what the vault declares.

### Zero-configured state

Fresh vault with mdbook installed but no mdBook hooks:

```
$ zetl ecosystem check
ECOSYSTEM  STATUS        VERSION              PLUGINS CONFIGURED   PLUGINS AVAILABLE
mdbook     detected      0.4.37               0                    0

No ecosystem hooks configured in this vault.
To enable an ecosystem, add a manifest under .zetl/hooks/:
  https://zetl.codeberg.page/docs/ecosystems/
```

Exit code is 0 in the zero-configured state regardless of which
runtimes are detected.

### Exit codes

- `0` — all *configured* ecosystems are available (or none are configured).
- non-zero — at least one configured ecosystem is missing its runtime.

For mdbook, "missing runtime" means a configured `exec` binary is
absent from `$PATH`. The absence of the top-level `mdbook` binary
is not itself a failure — preprocessors run without it.

Under `zetl build`, a missing preprocessor disables the affected
hook and continues the build with a `RuntimeAbsence` diagnostic; it
is not a hard failure unless `--ecosystem-required=mdbook` is
passed (REQ-3313), which is the CI gate mode.

### Plugin-version drift

At probe time the adapter invokes each configured preprocessor's
`--version` and compares against the `version_range` in
`tools/zetl-ecosystem-matrix.toml` (REQ-3314):

- **Exact match** — silent.
- **Minor drift** (same major, observed minor ≥ tested) — log `[zetl] ecosystem mdbook: mdbook-mermaid v0.14.1 is newer than last-tested v0.14.0; proceeding` once per session. The hook still runs.
- **Incompatible** (different major, or below the tested range) — hook disabled with a `plugin_version_incompatible` diagnostic pointing at the matrix entry.

## Troubleshooting

### Preprocessor binary not found

The adapter reports the `exec` it tried and an install hint:

```
[zetl] hook pre-parse/mermaid: preprocessor "mdbook-mermaid" not found on $PATH
       Install via: cargo install mdbook-mermaid
       (matrix entry: tools/zetl-ecosystem-matrix.toml)
```

The build continues with the hook disabled. Pass
`--ecosystem-required=mdbook` to fail fast instead.

### Preprocessor refuses the `html` renderer

The `<exec> supports html` probe exited non-zero:

```
[zetl] hook pre-parse/<name>: preprocessor refused renderer "html"
       The preprocessor does not declare support for this renderer.
       Override with [options] renderer = "<other>" if the
       preprocessor supports an alternate backend.
```

This is typically a preprocessor misconfigured with a renderer-
gating `[preprocessor.<name>.renderer]` key. Remove the gate or
point the manifest's `[options] renderer` at a value the
preprocessor accepts.

### Probe timeout

The probe is bounded to 5 seconds, independent of the manifest's
`timeout_ms`:

```
[zetl] hook pre-parse/<name>: preprocessor probe "supports html" timed out
       after 5s; disabling hook for this run.
```

A probe that hangs usually indicates a preprocessor that reads
stdin before responding, which violates the mdBook contract. File
upstream if the binary is a published release; pin to a known-good
version via `version_range` in the matrix otherwise.

### Malformed preprocessor output

The preprocessor exited 0 but emitted something that doesn't
round-trip through the envelope validator:

```
[zetl] hook pre-parse/<name>: malformed output
       expected top-level book.sections[0].Chapter.content to be a string,
       got null on page notes/idea-7.md
```

Common causes: the preprocessor stripped a `Chapter` it considered
empty, the preprocessor emitted a non-UTF-8 byte sequence, or the
preprocessor wrote log text to stdout instead of stderr. Inspect
the raw output by running the preprocessor manually:

```sh
mdbook-<name> < /tmp/zetl-envelope.json
```

zetl logs the envelope bytes to a per-hook temp file under
`--verbose`.

### Wikilinks / embeds missing from rendered output

A preprocessor is rewriting Markdown link syntax in a way that
eats zetl's double-bracket markers. zetl catches this via
preservation checks and emits:

```
[zetl] contract violation: mdbook-<name> dropped 4 Wikilink nodes
       on projects/q2-review.md
       Hint: add `preserves = ["Wikilink", "Embed", "SplBlock"]`
             to the hook's [contract] table, or file an issue with
             the preprocessor author.
```

Most `mdbook-*` preprocessors operate on specific fenced-block
syntaxes (mermaid, admonish, math delimiters) and leave unrelated
Markdown alone. A preprocessor that rewrites general link syntax
is the typical offender; narrow its selector with `select.path` to
confine it to pages that don't carry wikilinks, or opt out
page-by-page via frontmatter (`zetl: { hooks: { disable: ["..."] } }`).

### Transient preprocessor crashes

mdBook preprocessor crashes mid-build revert the page fragment per
SPEC-032 REQ-3207 and record a `hook_failure` diagnostic. zetl
does not retry in v1 — if crashes look transient (GC pauses,
transient OOM), that is noted as SPEC-033 §13 Q10 and deferred
pending field data.

## See also

- [`docs/ecosystems/pandoc.md`](./pandoc.md) — companion guide for Pandoc filters.
- [`docs/ecosystems/remark.md`](./remark.md) — companion guide for remark plugins.
- [`docs/hook-security.md`](../hook-security.md) — env allowlist and message-size caps that apply to every subprocess, including mdBook preprocessors.
- [`tools/zetl-mdbook-envelope-schema-v1.json`](../../tools/zetl-mdbook-envelope-schema-v1.json) — canonical envelope schema.
- [SPEC-033](../../specs/SPEC-033.md) — normative ecosystem-bridges specification.
- [SPEC-032](../../specs/SPEC-032.md) — normative hook-contract specification (contracts, selectors, preservation checks).
