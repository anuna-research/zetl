# mdbook-mermaid (experimental)

Canary fixture seeded by **task-mdbook-matrix-seed** for SPEC-033
§REQ-3311.

At `tier = "experimental"` the fixture is documentation: it declares
what a canonical `mdbook-mermaid` render looks like end-to-end, so the
follow-up task promoting the entry to `partial` can wire it into a
golden-HTML runner without re-designing the input surface.

## Contents

- `input.md` — a minimal three-node flowchart inside a ```` ```mermaid ````
  fenced code block, flanked by plain paragraphs so a regression in
  the preprocessor's fence handling localises cleanly in the diff.
- `expected.html` — the shape `mdbook build` with the `mermaid`
  preprocessor enabled is expected to produce for the chapter body
  under mdbook-mermaid 0.14+/0.15. Aspirational target, not a live
  CI assertion — checked against upstream output at promotion time.

## Running locally

Wire the preprocessor into a throwaway book:

```toml
# book.toml
[book]
title = "canary"
[preprocessor.mermaid]
command = "mdbook-mermaid"
[output.html]
additional-js = ["mermaid.min.js", "mermaid-init.js"]
```

Then:

```shell
mkdir -p canary/src
cp tests/ecosystem-fixtures/mdbook/mdbook-mermaid/input.md \
   canary/src/chapter_1.md
printf -- '- [canary](chapter_1.md)\n' > canary/src/SUMMARY.md
(cd canary && mdbook build)
```

The chapter body inside `canary/book/chapter_1.html` should match
`expected.html` modulo mdbook's surrounding page chrome.

## Promotion notes (experimental → partial)

The checklist in SPEC-033 REQ-3311 calls for:

- [x] Matrix entry with `version_range`, `tier = "experimental"`,
      maintainer contact, and upstream repo URL.
- [x] At least one working fixture at
      `tests/ecosystem-fixtures/mdbook/mdbook-mermaid/`.
- [ ] Golden-HTML fixture wired into a runner that actually invokes
      `mdbook build` with `mdbook-mermaid` enabled — lands with
      **task-eco-matrix**.
- [x] Known limitations documented in the matrix entry's `notes`
      field (client-side SVG render; renderer-gated asset injection;
      untested interaction with SplBlock classifier on ```spl fences).
