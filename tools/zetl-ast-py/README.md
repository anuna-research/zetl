# zetl-ast-py

First-party helper library for authoring [zetl](https://zetl.dev) transform-stage hooks in Python. Pinned to AST schema **major v1** (SPEC-032 REQ-3210).

This release ships the declarative dispatch surface (REQ-3218): node-name constants, `walk()`, `map_nodes()`, `dispatch()`, and the `@on_node` decorator. The protocol client (`run`, manifest helpers) is scheduled for `task-helper-py` under SPEC-032 Phase B and will layer on top of the dispatch primitives exposed here.

## Install

```sh
pip install zetl-ast-py
```

Python 3.9+.

## Hello hook

```python
from zetl_ast import dispatch, on_node, walk, Wikilink, BlockQuote


@on_node(Wikilink)
def autolink_alias(node, ctx):
    if not node.get("alias"):
        node["alias"] = node["target"]


@on_node(BlockQuote)
def rewrite_callout(node, ctx):
    # ... detect `> [!note]` heads, rewrite to a template-var blob
    ...


def transform(ast, ctx):
    table = {}
    autolink_alias(table)
    rewrite_callout(table)
    return dispatch(ast, ctx, table)
```

Low-level `walk` is still available when you want a visit-and-filter
pass:

```python
for link in walk(ast, type=Wikilink):
    if not link["alias"]:
        link["alias"] = link["target"]
```

## API surface

- **AST constants**: `Document`, `BlockQuote`, `Wikilink`, etc. — both dispatch keys and `walk(type=...)` filters.
- **`walk(root, *, type=None)`**: depth-first generator, optionally filtered.
- **`map_nodes(root, replacer)`**: one-pass in-place replacement.
- **`dispatch(root, ctx, table)`**: declarative visitor with per-type handlers plus reserved `Block` / `Inline` / `_fallback` / `Blocks` / `Inlines` keys (REQ-3218).
- **`@on_node(type)`**: decorator factory that attaches a handler to a dispatch table via `handler(table)`.

Reserved keys:

| Key         | Fires for                                                             |
| ----------- | --------------------------------------------------------------------- |
| `Block`     | Any block node not covered by a direct-type handler.                  |
| `Inline`    | Any inline node not covered by a direct-type handler.                 |
| `_fallback` | Any node not otherwise covered (including `Document` + `ListItem`).   |
| `Blocks`    | Each block-children list (pre-descent, parent passed as 2nd arg).     |
| `Inlines`   | Each inline-children list (pre-descent, parent passed as 2nd arg).    |

Direct-type handlers take precedence over `Block` / `Inline`, which take precedence over `_fallback`.

## Versioning

The package major tracks the AST schema major: `1.x.y` only speaks zetl-ext v1.x.

## License

MIT.
