# remark-gfm (experimental)

Canary fixture seeded by **task-remark-matrix-seed** for SPEC-033
§REQ-3311.

At `tier = "experimental"` the fixture is documentation: it declares
what a canonical `remark-gfm` render looks like end-to-end, so the
follow-up task promoting the entry to `partial` can wire it into a
golden-HTML runner without re-designing the input surface.

## Contents

- `input.md` — exercises the four GFM features most likely to drift
  across remark majors: pipe tables, task lists, autolink literals,
  and strikethrough. Each feature is isolated enough that a
  regression in one will localise cleanly in the diff.
- `expected.html` — the shape
  `unified().use(remarkParse).use(remarkGfm).use(remarkRehype).use(rehypeStringify)`
  is expected to produce under remark-gfm v4 / remark-rehype v11.
  Aspirational target, not a live CI assertion — checked against
  upstream output at promotion time.

## Running locally

```shell
cat tests/ecosystem-fixtures/remark/remark-gfm/input.md \
  | node --input-type=module -e "
    import('unified').then(({unified}) => Promise.all([
      import('remark-parse'), import('remark-gfm'),
      import('remark-rehype'), import('rehype-stringify'),
    ]).then(([rp, rg, rr, rs]) => {
      let md = '';
      process.stdin.on('data', c => md += c);
      process.stdin.on('end', async () => {
        const html = String(await unified()
          .use(rp.default).use(rg.default)
          .use(rr.default).use(rs.default).process(md));
        process.stdout.write(html);
      });
    }))
  "
```

(The v1 harness invoked by `task-remark-harness` wraps this pipeline
behind the JSON-lines protocol — run-by-hand is for fixture
regeneration only.)

## Promotion notes (experimental → partial)

The checklist in SPEC-033 REQ-3311 calls for:

- [x] Matrix entry with `version_range`, `tier = "experimental"`,
      maintainer contact, and upstream repo URL.
- [x] At least one working fixture at
      `tests/ecosystem-fixtures/remark/remark-gfm/`.
- [ ] Golden-HTML fixture wired into a runner that actually invokes
      `remark-gfm` through the harness — lands with **task-eco-matrix**.
- [x] Known limitations documented in the matrix entry's `notes`
      field (frontmatter stripping is the caller's responsibility;
      task-list DOM shape diverged between GFM v3 and v4).
