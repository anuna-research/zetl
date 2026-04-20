# pandoc-citeproc (experimental, legacy)

Canary fixture seeded by **task-pandoc-matrix-seed** for SPEC-033
§REQ-3311.

`pandoc-citeproc` was archived upstream in 2022 and its functionality
folded into pandoc itself as the `--citeproc` flag. This fixture
exists because the filter is still in use by legacy Makefile
pipelines and by users pinned to pandoc <2.11. New deployments
SHOULD declare `citeproc = true` on their pandoc manifest instead
— the matrix entry's `notes` field calls this out and the tier is
capped at `experimental` for the lifetime of zetl v1.

## Contents

- `input.md` — a page with one inline citation and a `# References`
  heading; the filter populates the heading's section with the CSL
  bibliography entry.
- `references.bib` — a one-entry BibTeX bibliography referenced from
  `input.md`'s frontmatter.
- `expected.html` — the shape pandoc + citeproc (or pandoc
  `--citeproc`) produces in the default Chicago author-date style.

## Running locally

With the legacy filter binary:

```shell
pandoc --filter pandoc-citeproc --from markdown --to html5 \
  tests/ecosystem-fixtures/pandoc/pandoc-citeproc/input.md
```

With the built-in equivalent (recommended, produces identical HTML):

```shell
pandoc --citeproc --from markdown --to html5 \
  tests/ecosystem-fixtures/pandoc/pandoc-citeproc/input.md
```

## Promotion notes

Not scheduled for promotion above `experimental` — the canonical
replacement is pandoc's built-in citation processing, surfaced in
zetl via the adapter's native-mode resolution path (SPEC-033
ADR-3302).
