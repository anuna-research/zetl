# zetl-ast-py

First-party helper library for authoring [zetl](https://zetl.dev) hooks in
Python. Pinned to AST schema **major v1** (SPEC-032 REQ-3210).

## Install

```sh
pip install zetl-ast-py
```

Python 3.9+.

## Hello hook

```python
from zetl_ast import run, dispatch, on_node, Wikilink


@on_node(Wikilink)
def autolink_alias(node, ctx):
    if not node.get("alias"):
        node["alias"] = node["target"]


def transform(ast, ctx):
    table = {}
    autolink_alias(table)
    return dispatch(ast, ctx, table)


if __name__ == "__main__":
    run(transform, hook_id="autolinker", version="0.1.0")
```

That's the whole persistent-mode hook. `run()` writes the handshake,
reads line-delimited JSON from stdin, and calls `transform` for every
page zetl sends — replying with a single `result` / `error` line per
call. On any exception, the loop emits a typed `error` frame; zetl's
REQ-3207 recovery policy decides whether to continue or drop the hook.

For filter-style invocation, `run_one_shot(transform)` reads the whole
stdin, applies `transform`, and writes the result to stdout.

## API surface

- **AST constants**: `Document`, `BlockQuote`, `Wikilink`, etc. — both
  dispatch keys and `walk(type=...)` filters.
- **Traversal**: `walk(root, *, type=None)`, `map_nodes(root, replacer)`.
- **Dispatch (REQ-3218)**: `dispatch(root, ctx, table)` with reserved
  `Block` / `Inline` / `_fallback` / `Blocks` / `Inlines` keys plus the
  `@on_node(type)` decorator.
- **Run loop (CON-3201)**: `run(transform, ...)`, `run_one_shot(transform, ...)`.
- **Context (REQ-3219 / REQ-3220 / REQ-3214)**: `Context` exposes
  `page_slug`, `frontmatter`, `stage`, `env: BuildEnv`, `build_data:
  BuildDataView`, plus emit helpers (`emit_template_vars`,
  `emit_vault_template_vars`, `emit_build_data`, `warn` / `info` /
  `error` / `diag`).
- **Manifest helpers (REQ-3206 / REQ-3217 / REQ-3221 / REQ-3224)**:
  `Manifest` / `OrderingTable` / `ContractTable` dataclasses,
  `render_manifest()`, `parse_manifest()`, `resolved_before()` /
  `resolved_after()`, `default_extension_id()`.
- **Protocol primitives (CON-3201)**: `handshake_line`,
  `parse_host_message`, `serialize_hook_message`, `check_ast_major`,
  `ProtocolError`.

### Reserved dispatch keys

| Key         | Fires for                                                             |
| ----------- | --------------------------------------------------------------------- |
| `Block`     | Any block node not covered by a direct-type handler.                  |
| `Inline`    | Any inline node not covered by a direct-type handler.                 |
| `_fallback` | Any node not otherwise covered (including `Document` + `ListItem`).   |
| `Blocks`    | Each block-children list (pre-descent, parent passed as 2nd arg).     |
| `Inlines`   | Each inline-children list (pre-descent, parent passed as 2nd arg).    |

Direct-type handlers take precedence over `Block` / `Inline`, which take
precedence over `_fallback`.

## Versioning

The package major tracks the AST schema major: `1.x.y` only speaks
zetl-ext v1.x. A zetl binary advertising a different AST major in the
init handshake is rejected with an `ast_version_mismatch` error
response.

## License

MIT.
