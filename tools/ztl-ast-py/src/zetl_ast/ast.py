"""
Node-name constants mirror ``tools/ztl-ast-schema-v1.json`` (v1.0).

Nodes themselves are passed around as plain ``dict`` (the same JSON shape
the Rust side emits over stdio). This lets hooks write idiomatic Python
without a generated type layer, matching the JS helper's use of plain
object literals.

Every constant is a frozen string so it works as both a dict key (in
dispatch tables) and as the discriminator of an on-the-wire node.
"""

from __future__ import annotations

from typing import Any, FrozenSet, Mapping

#: Major schema version this helper is pinned to (REQ-3210).
AST_MAJOR: int = 1

#: Exact two-component schema version emitted by ztl at
#: ``Document.ast_version``.
AST_VERSION: str = "1.0"

# ── Node-name constants ──────────────────────────────────────────────────────

Document: str = "Document"
Heading: str = "Heading"
Paragraph: str = "Paragraph"
BlockQuote: str = "BlockQuote"
List: str = "List"
ListItem: str = "ListItem"
CodeBlock: str = "CodeBlock"
ThematicBreak: str = "ThematicBreak"
HtmlBlock: str = "HtmlBlock"
SplBlock: str = "SplBlock"
Embed: str = "Embed"
Text: str = "Text"
Emphasis: str = "Emphasis"
Strong: str = "Strong"
Code: str = "Code"
Link: str = "Link"
Image: str = "Image"
LineBreak: str = "LineBreak"
SoftBreak: str = "SoftBreak"
HtmlInline: str = "HtmlInline"
Wikilink: str = "Wikilink"

BLOCK_TYPES: FrozenSet[str] = frozenset(
    {
        Heading,
        Paragraph,
        BlockQuote,
        List,
        CodeBlock,
        ThematicBreak,
        HtmlBlock,
        SplBlock,
        Embed,
    }
)

INLINE_TYPES: FrozenSet[str] = frozenset(
    {
        Text,
        Emphasis,
        Strong,
        Code,
        Link,
        Image,
        LineBreak,
        SoftBreak,
        HtmlInline,
        Wikilink,
    }
)

#: Every node-name constant known to this helper version. Grep-friendly,
#: order matches the schema's declaration order.
KNOWN_NODE_TYPES = (
    Document,
    Heading,
    Paragraph,
    BlockQuote,
    List,
    ListItem,
    CodeBlock,
    ThematicBreak,
    HtmlBlock,
    SplBlock,
    Embed,
    Text,
    Emphasis,
    Strong,
    Code,
    Link,
    Image,
    LineBreak,
    SoftBreak,
    HtmlInline,
    Wikilink,
)


Node = Mapping[str, Any]
"""Type alias for on-the-wire nodes. A plain mapping with a ``type`` key."""


def is_block(node: Node) -> bool:
    """True when ``node["type"]`` is a block-level node name."""
    return node.get("type") in BLOCK_TYPES


def is_inline(node: Node) -> bool:
    """True when ``node["type"]`` is an inline node name."""
    return node.get("type") in INLINE_TYPES


def is_document(node: Node) -> bool:
    """True when ``node["type"] == "Document"``."""
    return node.get("type") == Document
