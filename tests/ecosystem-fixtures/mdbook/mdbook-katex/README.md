# mdbook-katex (experimental)

Canary fixture seeded by **task-mdbook-matrix-seed** for SPEC-033
§REQ-3311.

At `tier = "experimental"` the fixture is documentation: it declares
what a canonical `mdbook-katex` render looks like end-to-end, so the
follow-up task promoting the entry to `partial` can wire it into a
golden-HTML runner without re-designing the input surface.

## Contents

- `input.md` — one inline `$...$` span (Pythagorean identity) and
  one `$$...$$` display block (Euler's identity). Both are canonical
  short expressions, chosen so that an upstream KaTeX rendering
  regression would be obvious in a visual diff.
- `expected.html` — a **wrapper-only** shape: the outer
  `<span class="katex">` (inline) and `<span class="katex-display">`
  (display) plus the surrounding markdown-rendered paragraphs. The
  KaTeX-internal MathML + span tree is elided because byte-stable
  comparison of that tree is brittle across KaTeX minor versions.
  Under task-eco-matrix the runner will assert the full subtree
  against the pinned KaTeX version; this canary only asserts the
  envelope.

## Running locally

```toml
# book.toml
[book]
title = "canary"
[preprocessor.katex]
command = "mdbook-katex"
```

```shell
mkdir -p canary/src
cp tests/ecosystem-fixtures/mdbook/mdbook-katex/input.md \
   canary/src/chapter_1.md
printf -- '- [canary](chapter_1.md)\n' > canary/src/SUMMARY.md
(cd canary && mdbook build)
```

The chapter body inside `canary/book/chapter_1.html` will carry
the full KaTeX subtree where `expected.html` elides it — diff the
envelope (opening tags + paragraph structure) to sanity-check, and
fall through to the task-eco-matrix runner for the subtree gate.

## Promotion notes (experimental → partial)

The checklist in SPEC-033 REQ-3311 calls for:

- [x] Matrix entry with `version_range`, `tier = "experimental"`,
      maintainer contact, and upstream repo URL.
- [x] At least one working fixture at
      `tests/ecosystem-fixtures/mdbook/mdbook-katex/`.
- [ ] Golden-HTML fixture wired into a runner that actually invokes
      `mdbook build` with `mdbook-katex` enabled — lands with
      **task-eco-matrix** (and will need a version-pinned KaTeX to
      make the subtree comparison byte-stable).
- [x] Known limitations documented in the matrix entry's `notes`
      field (MathML + HTML fallback double-render; smart-punctuation
      interaction; escaped-dollar corner under lzanini#117).
