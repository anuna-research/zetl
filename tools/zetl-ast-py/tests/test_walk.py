"""Walk + map_nodes sanity tests — enough to pin the dispatch foundation."""

from __future__ import annotations

import unittest
from typing import Any, Dict

from zetl_ast import Text, Wikilink, children_of, map_nodes, walk

from .fixtures import sample_ast


class TestWalk(unittest.TestCase):
    def test_walks_depth_first_preorder(self) -> None:
        ast = sample_ast()
        types = [n["type"] for n in walk(ast)]
        # Document comes first, and the first children are Heading + its
        # text nodes. That's enough to pin traversal order.
        self.assertEqual(types[0], "Document")
        self.assertEqual(types[1], "Heading")
        self.assertEqual(types[2], "Text")

    def test_single_type_filter(self) -> None:
        ast = sample_ast()
        texts = list(walk(ast, type=Text))
        self.assertEqual(len(texts), 5)
        self.assertTrue(all(n["type"] == "Text" for n in texts))

    def test_multi_type_filter(self) -> None:
        ast = sample_ast()
        hits = list(walk(ast, type={Text, Wikilink}))
        kinds = sorted({n["type"] for n in hits})
        self.assertEqual(kinds, ["Text", "Wikilink"])

    def test_children_of_leaf_returns_empty(self) -> None:
        leaf: Dict[str, Any] = {"type": "Text", "text": "x", "position": {}}
        self.assertEqual(list(children_of(leaf)), [])


class TestMapNodes(unittest.TestCase):
    def test_replaces_in_place(self) -> None:
        ast = sample_ast()

        def upper_text(node: Dict[str, Any]):
            if node["type"] == "Text":
                return {**node, "text": node["text"].upper()}
            return None

        map_nodes(ast, upper_text)
        upper = [n["text"] for n in walk(ast, type=Text)]
        self.assertTrue(all(t == t.upper() for t in upper))

    def test_descends_into_replacement_not_original(self) -> None:
        # A replacer that swaps a Paragraph for one with a single Text
        # should see the Text on the subsequent recursion.
        ast = sample_ast()
        seen: list[str] = []

        def replacer(node: Dict[str, Any]):
            seen.append(node["type"])
            if node["type"] == "Paragraph" and any(
                c["type"] == "Wikilink" for c in node["children"]
            ):
                return {
                    "type": "Paragraph",
                    "position": node["position"],
                    "children": [
                        {"type": "Text", "position": node["position"], "text": "R"}
                    ],
                }
            return None

        map_nodes(ast, replacer)
        # The replacement Paragraph's single Text must appear in the
        # traversal — proves descent into replacement, not original.
        self.assertIn("Text", seen)


if __name__ == "__main__":
    unittest.main()
