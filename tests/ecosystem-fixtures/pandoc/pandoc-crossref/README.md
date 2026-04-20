# pandoc-crossref (experimental)

Canary fixture seeded by **task-pandoc-matrix-seed** for SPEC-033
§REQ-3311.

At `tier = "experimental"` the fixture is documentation: it declares
what a canonical `pandoc-crossref` render looks like end-to-end, so
that the follow-up task promoting the entry to `partial` can wire it
into a golden-HTML runner without re-designing the input surface.

## Contents

- `input.md` — a minimal cross-reference: one labelled figure referenced
  by `@fig:…`, plus a trivial equation reference. Exercises the two
  crossref features most likely to break under translation
  (Wikilink/Embed/SPL markers must survive alongside crossref-injected
  `<figure>` wrapping and numbered link spans).
- `expected.html` — the shape `pandoc --filter pandoc-crossref` is
  expected to produce on the input under pandoc 3.x. This is an
  aspirational target, not a live CI assertion — it is checked against
  upstream output at promotion time, not on every build.

## Running locally

```shell
pandoc --filter pandoc-crossref --from markdown --to html5 \
  tests/ecosystem-fixtures/pandoc/pandoc-crossref/input.md
```

## Promotion notes (experimental → partial)

The checklist in SPEC-033 REQ-3311 calls for:

- [x] Matrix entry with `version_range`, maintainer contact, upstream
      repo URL.
- [x] At least one working fixture in
      `tests/ecosystem-fixtures/pandoc/pandoc-crossref/`.
- [ ] Golden-HTML fixture wired into a runner that actually invokes
      `pandoc --filter pandoc-crossref` — landing with the follow-up
      **task-eco-matrix** work.
- [x] Known limitations documented in the matrix entry's `notes` field.

The middle checkbox is the remaining blocker for `partial`.
