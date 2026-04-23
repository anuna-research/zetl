"""
Public surface of ``ztl-ast-py`` (SPEC-032 REQ-3210 / CON-3210).

Import pattern::

    from ztl_ast import run, dispatch, on_node, walk, Wikilink, BlockQuote

This release ships:

* the AST node-name constants + traversal helpers (``walk`` / ``map_nodes``);
* the declarative dispatch surface (REQ-3218): ``dispatch`` / ``on_node``;
* the persistent-mode protocol client (REQ-3210 / CON-3201): ``run`` /
  ``run_one_shot`` plus the lower-level ``protocol`` primitives;
* manifest helpers (REQ-3206 / REQ-3217 / REQ-3221 / REQ-3224):
  ``render_manifest`` / ``parse_manifest`` and the ``Manifest`` dataclass.
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
from .context import (
    BuildDataView,
    BuildEnv,
    Context,
    ContextWrites,
    Diagnostic,
    build_context,
    writes_to_result_payload,
)
from .dispatch import dispatch, on_node
from .manifest import (
    AST_TYPES,
    ContractTable,
    Manifest,
    ManifestError,
    OrderingTable,
    default_extension_id,
    parse_manifest,
    render_manifest,
    resolved_after,
    resolved_before,
)
from .protocol import (
    ProtocolError,
    check_ast_major,
    handshake_line,
    parse_host_message,
    read_lines,
    serialize_hook_message,
)
from .run import TransformFn, run, run_one_shot
from .walk import children_of, map_nodes, walk

__all__ = [
    "AST_MAJOR",
    "AST_TYPES",
    "AST_VERSION",
    "BLOCK_TYPES",
    "BlockQuote",
    "BuildDataView",
    "BuildEnv",
    "Code",
    "CodeBlock",
    "Context",
    "ContextWrites",
    "ContractTable",
    "Diagnostic",
    "Document",
    "Embed",
    "Emphasis",
    "Heading",
    "HtmlBlock",
    "HtmlInline",
    "INLINE_TYPES",
    "Image",
    "KNOWN_NODE_TYPES",
    "LineBreak",
    "Link",
    "List",
    "ListItem",
    "Manifest",
    "ManifestError",
    "OrderingTable",
    "Paragraph",
    "ProtocolError",
    "SoftBreak",
    "SplBlock",
    "Strong",
    "Text",
    "ThematicBreak",
    "TransformFn",
    "Wikilink",
    "build_context",
    "check_ast_major",
    "children_of",
    "default_extension_id",
    "dispatch",
    "handshake_line",
    "is_block",
    "is_document",
    "is_inline",
    "map_nodes",
    "on_node",
    "parse_host_message",
    "parse_manifest",
    "read_lines",
    "render_manifest",
    "resolved_after",
    "resolved_before",
    "run",
    "run_one_shot",
    "serialize_hook_message",
    "walk",
    "writes_to_result_payload",
]
