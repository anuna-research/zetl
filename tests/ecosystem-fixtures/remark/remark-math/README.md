# remark-math (experimental)

Canary fixture seeded by **task-remark-matrix-seed** for SPEC-033
§REQ-3311.

## Contents

- `input.md` — one inline math span (`$E = mc^2$`) and one display
  math block (`$$ … $$`) with a representative integral. Exercises
  both `inlineMath` and `math` mdast nodes that remark-math
  introduces.
- `expected.html` — the shape remark-math + remark-rehype produce on
  their own, without a KaTeX/MathJax layer. Math nodes serialise as
  `<code class="language-math math-inline">` / `<pre><code
  class="language-math math-display">…</code></pre>` so that
  downstream `rehype-katex` (or the zetl-native math renderer, if
  ever added) can pattern-match on the class list. Aspirational
  target, not a live CI assertion.

## Running locally

```shell
cat tests/ecosystem-fixtures/remark/remark-math/input.md \
  | node --input-type=module -e "
    import('unified').then(({unified}) => Promise.all([
      import('remark-parse'), import('remark-math'),
      import('remark-rehype'), import('rehype-stringify'),
    ]).then(([rp, rm, rr, rs]) => {
      let md = '';
      process.stdin.on('data', c => md += c);
      process.stdin.on('end', async () => {
        const html = String(await unified()
          .use(rp.default).use(rm.default)
          .use(rr.default).use(rs.default).process(md));
        process.stdout.write(html);
      });
    }))
  "
```

To render actual math glyphs, pipe the output through `rehype-katex`
(client-side CSS required) or `rehype-mathjax` (self-contained).

## Promotion notes (experimental → partial)

- [x] Matrix entry with `version_range`, maintainer, upstream URL.
- [x] Working fixture at the declared path.
- [ ] Golden-HTML runner that actually invokes remark-math — lands
      with **task-eco-matrix**.
- [x] Known limitations documented in the matrix entry's `notes`
      field (upstream archived / pass-through semantics, KaTeX
      layering is the consumer's concern).
