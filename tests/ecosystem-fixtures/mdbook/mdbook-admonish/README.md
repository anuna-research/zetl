# mdbook-admonish (experimental)

Canary fixture seeded by **task-mdbook-matrix-seed** for SPEC-033
§REQ-3311. Doubles as the upstream backing for the SPEC-032 Callouts
canonical extension per §13 Q1 resolution — the in-repo stub at
`tests/extension-fixtures/admonition/` defines the shared contract
and this fixture captures what the `mdbook-admonish` binary emits
for the same input surface.

At `tier = "experimental"` the fixture is documentation: it declares
what a canonical `mdbook-admonish` render looks like end-to-end, so
the follow-up task promoting the entry to `partial` can wire it into
a golden-HTML runner without re-designing the input surface.

## Contents

- `input.md` — four admonitions exercising the four canonical
  flavours (`note`, `warning`, `tip`, `danger`) and both the
  `title=` attribute form (first two) and the type-default title
  fallback (last two). Intentionally parallel to the Callouts
  in-repo fixture at `tests/extension-fixtures/admonition/input.md`,
  which uses the `ad-<type>` header-kv form and the
  `!!! <type> "Title"` Python-Markdown form — the three inputs
  render to the same shared `.admonition.<type>` class shape
  (modulo mdbook-admonish's additional `.admonish.admonish-<type>`
  classes, documented in the matrix entry notes).
- `expected.html` — the shape `mdbook build` with the `admonish`
  preprocessor enabled produces for the chapter body under
  mdbook-admonish 1.18+. Aspirational target, not a live CI
  assertion — checked against upstream output at promotion time.

## Running locally

```toml
# book.toml
[book]
title = "canary"
[preprocessor.admonish]
command = "mdbook-admonish"
assets_version = "3.0.0"
[output.html]
additional-css = ["./mdbook-admonish.css"]
```

```shell
mkdir -p canary/src
cp tests/ecosystem-fixtures/mdbook/mdbook-admonish/input.md \
   canary/src/chapter_1.md
printf -- '- [canary](chapter_1.md)\n' > canary/src/SUMMARY.md
(cd canary && mdbook-admonish install . && mdbook build)
```

The chapter body inside `canary/book/chapter_1.html` should match
`expected.html` modulo mdbook's surrounding page chrome.

## Relationship to the Callouts canonical extension

- `tests/extension-fixtures/admonition/` — in-repo stub, defines
  the `.admonition.<type>` contract + Obsidian/Python-Markdown input
  surfaces. Gated by the shared golden-HTML harness
  (CON-3212 / TEST-3212c).
- `tests/ecosystem-fixtures/mdbook/mdbook-admonish/` — this
  fixture. Exercises the `mdbook-admonish` binary's own input
  surface (```` ```admonish <type> title="…" ````) and its additional
  `admonish admonish-<type>` class prefix. Gated by task-eco-matrix.

The two pass through the same CSS: a theme targeting
`.admonition.<type>` (not `.admonish-<type>`) renders correctly under
either backing, which is the cross-ecosystem contract §13 Q1 locked
in.

## Promotion notes (experimental → partial)

The checklist in SPEC-033 REQ-3311 calls for:

- [x] Matrix entry with `version_range`, `tier = "experimental"`,
      maintainer contact, and upstream repo URL.
- [x] At least one working fixture at
      `tests/ecosystem-fixtures/mdbook/mdbook-admonish/`.
- [ ] Golden-HTML fixture wired into a runner that actually invokes
      `mdbook build` with `mdbook-admonish` enabled — lands with
      **task-eco-matrix**.
- [x] Known limitations documented in the matrix entry's `notes`
      field (class-shape divergence from the canonical; `title=`
      vs `title:` header-kv divergence from Obsidian; collapsible +
      anchor-id features not yet in the canonical contract).
