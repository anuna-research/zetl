# remark-directive (experimental)

Canary fixture seeded by **task-remark-matrix-seed** for SPEC-033
§REQ-3311.

## Contents

- `input.md` — one `:::note` container directive with a title attribute
  and one inline `:abbr[label]{attrs}` leaf directive. Exercises both
  the container form (paragraph-terminated, body parsed as markdown)
  and the inline form.
- `expected.html` — the shape produced when remark-directive is paired
  with the canonical "map directives to hast nodes" transform the
  `mdast-util-directive` docs recommend: container directives become
  `<div class="directive directive-<name>">` with the original
  attributes passed through, `:abbr[...]{...}` becomes a real
  `<abbr>`. This fixture encodes the transform's output, not
  remark-directive's raw mdast — remark-directive by itself leaves
  unknown node types that remark-rehype renders as empty `<div>`.
  Aspirational target, not a live CI assertion.

## Running locally

```shell
cat tests/ecosystem-fixtures/remark/remark-directive/input.md \
  | node --input-type=module -e "
    import('unified').then(({unified}) => Promise.all([
      import('remark-parse'), import('remark-directive'),
      import('remark-rehype'), import('rehype-stringify'),
      import('unist-util-visit'),
    ]).then(([rp, rd, rr, rs, uv]) => {
      const mapDirectives = () => (tree) => {
        uv.visit(tree, (node) => {
          if (node.type === 'containerDirective' ||
              node.type === 'leafDirective'      ||
              node.type === 'textDirective') {
            const data = node.data || (node.data = {});
            const hName = node.type === 'containerDirective'
              ? 'div' : node.name;
            data.hName = hName;
            data.hProperties = {
              ...(node.attributes || {}),
              className: node.type === 'containerDirective'
                ? ['directive', 'directive-' + node.name] : undefined,
            };
          }
        });
      };
      let md = '';
      process.stdin.on('data', c => md += c);
      process.stdin.on('end', async () => {
        const html = String(await unified()
          .use(rp.default).use(rd.default).use(mapDirectives)
          .use(rr.default).use(rs.default).process(md));
        process.stdout.write(html);
      });
    }))
  "
```

## Promotion notes (experimental → partial)

- [x] Matrix entry with `version_range`, maintainer, upstream URL.
- [x] Working fixture at the declared path.
- [ ] Golden-HTML runner that actually invokes remark-directive +
      the above map-to-hast transform — lands with **task-eco-matrix**.
- [x] Known limitations documented in the matrix entry's `notes`
      field (directive-to-HTML mapping is caller-defined; unmapped
      directives render as empty `<div>`).
