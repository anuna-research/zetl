"""
Declarative node-type dispatch (SPEC-032 REQ-3218).

Wraps a walk with a table keyed by node type. Reserved keys:

* ``Block``     — any block not otherwise covered
* ``Inline``    — any inline not otherwise covered
* ``_fallback`` — any node not otherwise covered
* ``Blocks``    — sequence-level; receives each block-children array
* ``Inlines``   — sequence-level; receives each inline-children array

Visit order is depth-first pre-order: the per-node handler fires on the
node, then the sequence handler fires on its children array (if any),
then each child is visited in turn. Handlers may mutate in place and
return ``None`` to keep the existing node, or return a replacement node
that is swapped into the parent's children.

Example::

    from ztl_ast import dispatch, on_node, Wikilink, BlockQuote

    @on_node(Wikilink)
    def link(node, ctx):
        node["alias"] = node["alias"] or node["target"]

    @on_node(BlockQuote)
    def quote(node, ctx):
        ...

    table = {}
    link(table)
    quote(table)
    dispatch(ast, ctx, table)
"""

from __future__ import annotations

from typing import Any, Callable, Dict, List, Optional

from .ast import BLOCK_TYPES, INLINE_TYPES, Node

NodeHandler = Callable[[Node, Any], Optional[Node]]
SequenceHandler = Callable[[List[Node], Node, Any], Optional[List[Node]]]
DispatchTable = Dict[str, Any]

_BLOCK_CONTAINERS = frozenset({"Document", "BlockQuote", "ListItem", "List"})
_INLINE_CONTAINERS = frozenset(
    {"Paragraph", "Heading", "Emphasis", "Strong", "Link", "Image"}
)
_CHILD_BEARING = _BLOCK_CONTAINERS | _INLINE_CONTAINERS


def dispatch(root: Node, ctx: Any, table: DispatchTable) -> Node:
    """
    Walk ``root`` and invoke handlers from ``table`` against each
    matching node. Returns ``root`` (possibly a replacement produced by
    a root-level handler).
    """
    return _visit(root, ctx, table)


def _visit(node: Node, ctx: Any, table: DispatchTable) -> Node:
    handler = _resolve_handler(node, table)
    if handler is not None:
        replacement = handler(node, ctx)
        if replacement is not None and replacement is not node:
            node = replacement

    _run_sequence(node, ctx, table)

    children = _mutable_children(node)
    if children is not None:
        for i, child in enumerate(children):
            children[i] = _visit(child, ctx, table)
    return node


def _resolve_handler(node: Node, table: DispatchTable) -> Optional[NodeHandler]:
    t = node.get("type")
    direct = table.get(t) if isinstance(t, str) else None
    if direct is not None:
        return direct
    if t in BLOCK_TYPES and "Block" in table:
        return table["Block"]
    if t in INLINE_TYPES and "Inline" in table:
        return table["Inline"]
    return table.get("_fallback")


def _run_sequence(node: Node, ctx: Any, table: DispatchTable) -> None:
    blocks = table.get("Blocks")
    inlines = table.get("Inlines")
    if blocks is None and inlines is None:
        return

    t = node.get("type")
    if t in _BLOCK_CONTAINERS and blocks is not None:
        kids = node.get("children")
        if isinstance(kids, list):
            nxt = blocks(kids, node, ctx)
            if nxt is not None:
                node["children"] = nxt  # type: ignore[index]
    elif t in _INLINE_CONTAINERS and inlines is not None:
        kids = node.get("children")
        if isinstance(kids, list):
            nxt = inlines(kids, node, ctx)
            if nxt is not None:
                node["children"] = nxt  # type: ignore[index]


def _mutable_children(node: Node) -> Optional[List[Node]]:
    if node.get("type") in _CHILD_BEARING:
        kids = node.get("children")
        if isinstance(kids, list):
            return kids
    return None


# ── on_node factory (REQ-3218) ──────────────────────────────────────────────


def on_node(node_type: str) -> Callable[[NodeHandler], NodeHandler]:
    """
    Declarative registration for a single node type.

    Used as a decorator, it attaches a ``__ztl_dispatch__`` tuple to
    the function and returns a callable that installs the handler into
    an existing dispatch table::

        @on_node("Wikilink")
        def link(node, ctx):
            ...

        table: DispatchTable = {}
        link(table)   # installs under "Wikilink"
        dispatch(ast, ctx, table)

    The decorator is intentionally thin — ``register.__ztl_dispatch__``
    exposes ``(type, original_handler)`` so tooling (coverage, audit)
    can enumerate hooks without re-parsing source.
    """

    def decorator(handler: NodeHandler) -> NodeHandler:
        def register(table: DispatchTable) -> DispatchTable:
            table[node_type] = handler
            return table

        register.__ztl_dispatch__ = (node_type, handler)  # type: ignore[attr-defined]
        register.__wrapped__ = handler  # type: ignore[attr-defined]
        register.__name__ = getattr(handler, "__name__", "register")
        register.__doc__ = handler.__doc__
        return register  # type: ignore[return-value]

    return decorator
