"""
Dispatch contract tests for ``zetl_ast.dispatch`` (SPEC-032 REQ-3218).

Mirrors ``tools/zetl-ast-js/test/dispatch.test.ts`` so the two helper
libraries stay in lockstep on observable behaviour.
"""

from __future__ import annotations

import unittest
from typing import Any, Dict, List

from zetl_ast import (
    BlockQuote,
    Paragraph,
    Text,
    Wikilink,
    dispatch,
    on_node,
)
from zetl_ast.dispatch import DispatchTable

from .fixtures import pos, sample_ast


CTX: Dict[str, Any] = {
    "page_slug": "test",
    "frontmatter": {},
    "stage": "transform",
    "env": {"mode": "build", "theme": None, "vault_root": "/v"},
    "build_data": {},
}


class TestDispatch(unittest.TestCase):
    def test_routes_by_type(self) -> None:
        ast = sample_ast()
        seen: List[str] = []
        dispatch(
            ast,
            CTX,
            {
                Wikilink: lambda node, ctx: seen.append(f"wl:{node['target']}"),
                BlockQuote: lambda node, ctx: seen.append(
                    f"bq:{len(node['children'])}"
                ),
            },
        )
        self.assertEqual(seen, ["wl:Other page", "bq:1"])

    def test_falls_back_to_inline_block_and_catchall(self) -> None:
        ast = sample_ast()
        counters = {"inline": 0, "block": 0, "fallback": 0}

        def bump(key: str):
            def h(node: Any, ctx: Any) -> None:
                counters[key] += 1

            return h

        dispatch(
            ast,
            CTX,
            {
                "Inline": bump("inline"),
                "Block": bump("block"),
                "_fallback": bump("fallback"),
            },
        )
        # Every Text/Wikilink/Emphasis → inline;
        # Heading/Paragraph/BlockQuote/CodeBlock → block;
        # Document itself → fallback.
        self.assertGreaterEqual(counters["inline"], 5)
        self.assertGreaterEqual(counters["block"], 4)
        self.assertEqual(counters["fallback"], 1)

    def test_replaces_node_when_handler_returns_one(self) -> None:
        ast = sample_ast()

        def replace_link(node: Any, ctx: Any) -> Dict[str, Any]:
            return {
                "type": "Text",
                "position": node["position"],
                "text": f"link:{node['target']}",
            }

        dispatch(ast, CTX, {Wikilink: replace_link})

        paragraph = next(c for c in ast["children"] if c["type"] == "Paragraph")
        self.assertTrue(
            any(
                n["type"] == "Text" and n.get("text", "").startswith("link:")
                for n in paragraph["children"]
            )
        )

    def test_inlines_sequence_handler_can_mutate_children(self) -> None:
        ast = sample_ast()

        def inlines(children: List[Any], parent: Any, ctx: Any):
            if parent["type"] == "Heading":
                return [{"type": "Text", "position": pos(), "text": "REWRITTEN"}]
            return None

        dispatch(ast, CTX, {"Inlines": inlines})

        heading = next(c for c in ast["children"] if c["type"] == "Heading")
        self.assertEqual(len(heading["children"]), 1)
        self.assertEqual(heading["children"][0]["text"], "REWRITTEN")

    def test_type_handler_takes_precedence_over_inline_and_fallback(self) -> None:
        ast = sample_ast()
        counters = {"text": 0, "inline": 0}

        def text(node: Any, ctx: Any) -> None:
            counters["text"] += 1

        def inline(node: Any, ctx: Any) -> None:
            counters["inline"] += 1

        dispatch(ast, CTX, {Text: text, "Inline": inline})

        self.assertEqual(counters["text"], 5)
        # Wikilink + Emphasis → 2 unmatched inlines.
        self.assertEqual(counters["inline"], 2)

    def test_direct_type_handler_wins_over_catchall(self) -> None:
        ast = sample_ast()
        seen: List[str] = []
        dispatch(
            ast,
            CTX,
            {
                Text: lambda node, ctx: seen.append("text"),
                "_fallback": lambda node, ctx: seen.append("fallback"),
            },
        )
        # 5 texts in the fixture + fallback for everything else
        # (Document, Heading, Paragraph, BlockQuote, Paragraph, Emphasis,
        # Wikilink, CodeBlock).
        self.assertEqual(seen.count("text"), 5)
        self.assertGreater(seen.count("fallback"), 0)
        self.assertNotIn("inline", seen)


class TestOnNode(unittest.TestCase):
    def test_decorator_installs_into_table(self) -> None:
        @on_node(Wikilink)
        def handle_link(node: Any, ctx: Any) -> None:
            node["alias"] = node["alias"] or node["target"]

        table: DispatchTable = {}
        handle_link(table)
        self.assertIn("Wikilink", table)
        self.assertTrue(callable(table["Wikilink"]))

    def test_decorator_exposes_metadata(self) -> None:
        @on_node(BlockQuote)
        def handle_quote(node: Any, ctx: Any) -> None:
            return None

        node_type, original = handle_quote.__zetl_dispatch__  # type: ignore[attr-defined]
        self.assertEqual(node_type, BlockQuote)
        self.assertTrue(callable(original))
        # __wrapped__ preserves the original handler for introspection.
        self.assertTrue(hasattr(handle_quote, "__wrapped__"))

    def test_chains_multiple_decorators_into_one_table(self) -> None:
        @on_node(BlockQuote)
        def q(node: Any, ctx: Any) -> None:
            return None

        @on_node(Paragraph)
        def p(node: Any, ctx: Any) -> None:
            return None

        table: DispatchTable = {}
        q(table)
        p(table)
        self.assertIn("BlockQuote", table)
        self.assertIn("Paragraph", table)

    def test_decorator_runs_through_dispatch(self) -> None:
        calls: List[str] = []

        @on_node(Wikilink)
        def handle_link(node: Any, ctx: Any) -> None:
            calls.append(node["target"])

        table: DispatchTable = {}
        handle_link(table)
        dispatch(sample_ast(), CTX, table)
        self.assertEqual(calls, ["Other page"])


if __name__ == "__main__":
    unittest.main()
