# pantable (experimental)

Canary fixture seeded by **task-pandoc-matrix-seed** for SPEC-033
§REQ-3311.

`pantable` is a Python pandoc filter that converts CSV (and other
tabular source formats) into native pandoc Table nodes, which then
render through the normal HTML writer path. This fixture exercises
the minimum viable case: a `.table` fenced code block with inline
CSV content and a caption.

## Contents

- `input.md` — one `.table` fenced code block with a two-column,
  two-row CSV body and a caption attribute.
- `expected.html` — pandoc's default HTML5 rendering of the Table
  node that pantable injects.

## Running locally

```shell
pandoc --filter pantable --from markdown --to html5 \
  tests/ecosystem-fixtures/pandoc/pantable/input.md
```

Requires `python3 -m pantable` to be resolvable; see the matrix
entry's `notes` field for the REQ-3313 runtime-probe convention.

## Promotion notes (experimental → partial)

- [x] Matrix entry with `version_range`, maintainer, upstream URL.
- [x] Working fixture at the declared path.
- [ ] Golden-HTML runner that actually invokes pantable — lands with
      **task-eco-matrix**.
- [x] Known limitations documented in the matrix entry's `notes`
      field (Python module probe, `colspecs` best-effort alignment).
