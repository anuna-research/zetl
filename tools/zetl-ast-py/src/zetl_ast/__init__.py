"""
Public surface of ``zetl-ast-py`` (SPEC-032 REQ-3210 / CON-3210).

Import pattern::

    from zetl_ast import dispatch, on_node, walk, Wikilink, BlockQuote

This release ships the declarative dispatch surface (REQ-3218) and the
bare minimum of AST node-name constants + walk helpers needed to drive
it. The protocol client (``run``, manifest helpers, ``runOneShot``) is
scheduled for ``task-helper-py`` under SPEC-032 Phase B.
"""

from .ast import (
    AST_MAJOR,
    AST_VERSION,
    BLOCK_TYPES,
    INLINE_TYPES,
    KNOWN_NODE_TYPES,
    BlockQuote,
    Code,
    CodeBlock,
    Document,
    Embed,
    Emphasis,
    Heading,
    HtmlBlock,
    HtmlInline,
    Image,
    LineBreak,
    Link,
    List,
    ListItem,
    Paragraph,
    SoftBreak,
    SplBlock,
    Strong,
    Text,
    ThematicBreak,
    Wikilink,
    is_block,
    is_document,
    is_inline,
)
from .dispatch import dispatch, on_node
from .walk import children_of, map_nodes, walk

__all__ = [
    "AST_MAJOR",
    "AST_VERSION",
    "BLOCK_TYPES",
    "INLINE_TYPES",
    "KNOWN_NODE_TYPES",
    "BlockQuote",
    "Code",
    "CodeBlock",
    "Document",
    "Embed",
    "Emphasis",
    "Heading",
    "HtmlBlock",
    "HtmlInline",
    "Image",
    "LineBreak",
    "Link",
    "List",
    "ListItem",
    "Paragraph",
    "SoftBreak",
    "SplBlock",
    "Strong",
    "Text",
    "ThematicBreak",
    "Wikilink",
    "children_of",
    "dispatch",
    "is_block",
    "is_document",
    "is_inline",
    "map_nodes",
    "on_node",
    "walk",
]
