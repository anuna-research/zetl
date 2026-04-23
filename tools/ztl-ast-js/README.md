# ztl-ast-js

First-party helper library for authoring [ztl](https://ztl.dev) transform-stage hooks in JavaScript / TypeScript. Pinned to AST schema **major v1** (SPEC-032 REQ-3210).

## Install

```sh
npm install ztl-ast-js
```

Node.js 18+ (tested on 22). Works in Deno via `npm:ztl-ast-js`. ESM-first with a CJS fallback entry.

## Hello hook

```ts
import { run, walk, dispatch, onNode, Wikilink } from "ztl-ast-js";

run((ast, ctx) => {
    // Low-level: walk + filter.
    for (const link of walk(ast, { type: Wikilink })) {
        if (!link.alias) link.alias = link.target;
    }

    // High-level: declarative dispatch (REQ-3218).
    return dispatch(ast, ctx, {
        BlockQuote(node) {
            // ... rewrite callouts
        },
        Inline(node) {
            // fallback for any inline not covered above
        },
    });
});
```

The `run()` entry point drives ztl's persistent-mode wire protocol: handshake on stdout, line-delimited JSON for every subsequent page. Use `runOneShot()` for filter-style integrations that read the whole document from stdin and write once to stdout.

## Manifest helpers

```ts
import { renderManifest, parseManifest } from "ztl-ast-js";

const toml = renderManifest({
    extension_id: "callouts",
    ordering: { before: ["admonition"], after: [] },
    ast_type: "ztl-ext",
    contract: { preserves: ["Wikilink", "Embed", "SplBlock"] },
});
```

## API surface

- **AST**: typed node interfaces (`DocumentNode`, `BlockQuoteNode`, …) + runtime constants (`Document`, `BlockQuote`, `Wikilink`, …).
- **`walk(root, { type? })`**: depth-first generator, optionally filtered by node type.
- **`dispatch(root, ctx, table)`**: declarative visitor with per-type handlers plus reserved `Block` / `Inline` / `_fallback` / `Blocks` / `Inlines` keys (REQ-3218).
- **`onNode(type, fn)`**: tuple factory for composing dispatch tables.
- **`run(transform, opts?)`** / **`runOneShot(transform, opts?)`**: protocol drivers.
- **`parseManifest(text)`** / **`renderManifest(m)`**: sidecar manifest helpers.
- **`validateDocument(v)`**: fast-fail structural validator pinned to AST major 1.

See `src/*.ts` for full type annotations.

## Versioning

The package major tracks the AST schema major: `1.x.y` of this package only speaks ztl-ext v1.x. Attempting to run against a mismatched `ast_schema_version` produces an `ast_version_mismatch` protocol error (CON-3210).

## License

MIT.
