# mdbook-toc (experimental)

Canary fixture seeded by **task-mdbook-matrix-seed** for SPEC-033
§REQ-3311.

At `tier = "experimental"` the fixture is documentation: it declares
what a canonical `mdbook-toc` render looks like end-to-end, so the
follow-up task promoting the entry to `partial` can wire it into a
golden-HTML runner without re-designing the input surface.

## Contents

- `input.md` — a page that carries one `<!-- toc -->` marker between
  an H1 (the page title — excluded from the TOC by mdbook-toc's
  defaults) and two H2 sections. Depths 1–2 are enough to exercise
  marker substitution without locking the fixture to mdbook-toc's
  configurable `max-level`.
- `expected.html` — the shape `mdbook build` with the `toc`
  preprocessor enabled is expected to produce for the chapter body
  under mdbook-toc 0.14. Aspirational target, not a live CI
  assertion — checked against upstream output at promotion time.

## Running locally

```toml
# book.toml
[book]
title = "canary"
[preprocessor.toc]
command = "mdbook-toc"
marker = "<!-- toc -->"
```

```shell
mkdir -p canary/src
cp tests/ecosystem-fixtures/mdbook/mdbook-toc/input.md \
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
      `tests/ecosystem-fixtures/mdbook/mdbook-toc/`.
- [ ] Golden-HTML fixture wired into a runner that actually invokes
      `mdbook build` with `mdbook-toc` enabled — lands with
      **task-eco-matrix**.
- [x] Known limitations documented in the matrix entry's `notes`
      field (HTML-comment marker survives frontmatter strip; configurable
      heading-depth range; anchor slug collisions with `^block-id`).
