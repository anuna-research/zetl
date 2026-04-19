"""
Low-level traversal helpers.

``walk()`` is a generator; ``map_nodes()`` does one-pass in-place
replacement. Prefer :func:`zetl_ast.dispatch` for 80% of hook work —
``walk`` is the building-block when you need to collect or filter.
"""

from __future__ import annotations

from typing import Any, Callable, Iterable, Iterator, List, Optional, Union

from .ast import Node

_CHILD_BEARING = frozenset(
    {
        "Document",
        "Paragraph",
        "Heading",
        "Emphasis",
        "Strong",
        "Link",
        "Image",
        "BlockQuote",
        "ListItem",
        "List",
    }
)


def children_of(node: Node) -> Iterable[Node]:
    """Child list for ``node`` in document order. ``()`` for leaves."""
    if node.get("type") in _CHILD_BEARING:
        return node.get("children") or ()
    return ()


def _matcher(
    filter_: Union[str, Iterable[str], None],
) -> Callable[[str], bool]:
    if filter_ is None:
        return lambda _t: True
    if isinstance(filter_, str):
        target = filter_
        return lambda t: t == target
    allowed = frozenset(filter_)
    return lambda t: t in allowed


def walk(
    root: Node,
    *,
    type: Union[str, Iterable[str], None] = None,
) -> Iterator[Node]:
    """
    Depth-first pre-order traversal.

    Yields ``root`` itself, then each child in order, recursing into
    child-bearing nodes. Skips leaves (``Text``, ``Code``, ``Wikilink``,
    etc.).

    If ``type`` is given, yields only matching nodes — either a single
    node-name constant or any iterable of them.
    """
    match = _matcher(type)
    yield from _walk_impl(root, match)


def _walk_impl(node: Node, match: Callable[[str], bool]) -> Iterator[Node]:
    if match(node.get("type", "")):
        yield node
    for child in children_of(node):
        yield from _walk_impl(child, match)


def map_nodes(
    root: Node,
    replacer: Callable[[Node], Optional[Node]],
) -> Node:
    """
    One-pass in-place replacement.

    Calls ``replacer(node)`` on every child. A returned node replaces
    the original; ``None`` (or the same object) leaves it unchanged.
    Descends into the replacement, not the original.
    """
    children = _mutable_children(root)
    if children is not None:
        for i, curr in enumerate(children):
            nxt = replacer(curr)
            if nxt is not None and nxt is not curr:
                children[i] = nxt
                map_nodes(nxt, replacer)
            else:
                map_nodes(curr, replacer)
    return root


def _mutable_children(node: Node) -> Optional[List[Any]]:
    if node.get("type") in _CHILD_BEARING:
        kids = node.get("children")
        if isinstance(kids, list):
            return kids
    return None
