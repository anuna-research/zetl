"""Fixture AST shared across tests. Mirrors ``test/fixtures/sample.ts``."""

from __future__ import annotations

from typing import Any, Dict

from ztl_ast import AST_VERSION

POSITION_ORIGIN: Dict[str, int] = {
    "start_line": 1,
    "start_col": 1,
    "end_line": 1,
    "end_col": 1,
}


def pos() -> Dict[str, int]:
    return dict(POSITION_ORIGIN)


def sample_ast() -> Dict[str, Any]:
    """Small AST exercising block + inline + container nodes."""
    return {
        "type": "Document",
        "ast_version": AST_VERSION,
        "position": pos(),
        "frontmatter": {"title": "Sample", "extensions": {"callouts": True}},
        "children": [
            {
                "type": "Heading",
                "position": pos(),
                "level": 1,
                "children": [
                    {"type": "Text", "position": pos(), "text": "Hello"},
                    {"type": "Text", "position": pos(), "text": " world"},
                ],
            },
            {
                "type": "Paragraph",
                "position": pos(),
                "children": [
                    {"type": "Text", "position": pos(), "text": "See "},
                    {
                        "type": "Wikilink",
                        "position": pos(),
                        "target": "Other page",
                        "alias": None,
                        "heading": None,
                        "block_id": None,
                    },
                    {"type": "Text", "position": pos(), "text": " for context."},
                ],
            },
            {
                "type": "BlockQuote",
                "position": pos(),
                "children": [
                    {
                        "type": "Paragraph",
                        "position": pos(),
                        "children": [
                            {
                                "type": "Emphasis",
                                "position": pos(),
                                "children": [
                                    {
                                        "type": "Text",
                                        "position": pos(),
                                        "text": "quoted",
                                    }
                                ],
                            }
                        ],
                    }
                ],
            },
            {
                "type": "CodeBlock",
                "position": pos(),
                "fenced": True,
                "lang": "py",
                "info": "py",
                "text": "x = 1\n",
            },
        ],
    }


def big_ast(paragraphs: int = 2000) -> Dict[str, Any]:
    """
    Synthetic AST with ~``paragraphs`` paragraphs of mixed inline
    content. Used by the dispatch-overhead benchmark. Node count is
    roughly ``1 + 5 * paragraphs``.
    """
    children = []
    for i in range(paragraphs):
        children.append(
            {
                "type": "Paragraph",
                "position": pos(),
                "children": [
                    {"type": "Text", "position": pos(), "text": f"lead {i} "},
                    {
                        "type": "Wikilink",
                        "position": pos(),
                        "target": f"Page {i}",
                        "alias": None,
                        "heading": None,
                        "block_id": None,
                    },
                    {
                        "type": "Emphasis",
                        "position": pos(),
                        "children": [
                            {"type": "Text", "position": pos(), "text": "em"},
                        ],
                    },
                    {"type": "Text", "position": pos(), "text": " tail"},
                ],
            }
        )
    return {
        "type": "Document",
        "ast_version": AST_VERSION,
        "position": pos(),
        "frontmatter": {},
        "children": children,
    }
