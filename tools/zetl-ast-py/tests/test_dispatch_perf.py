"""
Dispatch overhead benchmark (IMPL-032-033 task-node-dispatch).

Plan acceptance: "dispatch overhead per node <50µs".

Interpretation: walking a full AST through :func:`dispatch` with a
realistic table of handlers (mix of per-type, ``Block``, ``Inline``,
``Blocks``, ``Inlines`` entries) must average less than 50µs per
visited node. We run on a ~2k-paragraph fixture and divide total wall
time by the number of nodes yielded by a plain ``walk``.
"""

from __future__ import annotations

import os
import time
import unittest
from typing import Any

from zetl_ast import BlockQuote, Text, Wikilink, dispatch, walk

from .fixtures import big_ast

# 50 µs/node is the plan acceptance; keep a small slack because Python's
# cold-first-call is noisier than CPython's steady-state.
BUDGET_US_PER_NODE = 50.0


class TestDispatchOverhead(unittest.TestCase):
    def test_overhead_under_fifty_microseconds_per_node(self) -> None:
        if os.environ.get("ZETL_SKIP_BENCH"):
            self.skipTest("ZETL_SKIP_BENCH set")
        ast = big_ast(paragraphs=2000)
        node_count = sum(1 for _ in walk(ast))
        self.assertGreater(node_count, 5000)

        table: dict[str, Any] = {
            Wikilink: lambda node, ctx: None,
            Text: lambda node, ctx: None,
            BlockQuote: lambda node, ctx: None,
            "Block": lambda node, ctx: None,
            "Inline": lambda node, ctx: None,
            "Blocks": lambda children, parent, ctx: None,
            "Inlines": lambda children, parent, ctx: None,
        }
        ctx: dict[str, Any] = {}

        # Warm-up: JIT-like effects (method cache, branch prediction).
        dispatch(ast, ctx, table)

        samples: list[float] = []
        for _ in range(3):
            fresh = big_ast(paragraphs=2000)
            t0 = time.perf_counter()
            dispatch(fresh, ctx, table)
            samples.append((time.perf_counter() - t0) / node_count * 1_000_000.0)
        median_us = sorted(samples)[len(samples) // 2]

        self.assertLess(
            median_us,
            BUDGET_US_PER_NODE,
            f"dispatch overhead {median_us:.2f}µs/node exceeds "
            f"{BUDGET_US_PER_NODE}µs/node budget (samples={samples})",
        )


if __name__ == "__main__":
    unittest.main()
