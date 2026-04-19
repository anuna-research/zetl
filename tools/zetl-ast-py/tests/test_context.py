"""
Context-layer tests (REQ-3219 / REQ-3220 / REQ-3214).

Exercises the ``build_context`` factory, the emit helpers, and
``writes_to_result_payload`` — the exact shape the protocol layer
serialises.
"""

from __future__ import annotations

import unittest

from zetl_ast import (
    BuildDataView,
    BuildEnv,
    build_context,
    writes_to_result_payload,
)


class TestBuildDataView(unittest.TestCase):
    def test_view_exposes_lookup(self) -> None:
        v = BuildDataView({"graph": {"nodes": 4}})
        self.assertEqual(v.get("graph", "nodes"), 4)
        self.assertIsNone(v.get("absent", "x"))
        self.assertIn("graph", v)

    def test_view_tolerates_non_mapping(self) -> None:
        v = BuildDataView(None)
        self.assertEqual(dict(v.by_extension), {})


class TestBuildContext(unittest.TestCase):
    def test_factory_captures_env_and_frontmatter(self) -> None:
        env = BuildEnv(vault_root="/v", extension_id="unit")
        ctx, _writes = build_context(
            page_slug="pages/p",
            frontmatter={"title": "t"},
            stage="transform",
            env=env,
        )
        self.assertEqual(ctx.page_slug, "pages/p")
        self.assertEqual(ctx.stage, "transform")
        self.assertEqual(ctx.env.vault_root, "/v")
        self.assertEqual(dict(ctx.frontmatter), {"title": "t"})

    def test_non_mapping_frontmatter_becomes_empty_dict(self) -> None:
        ctx, _ = build_context(
            page_slug="p",
            frontmatter=[1, 2],
            stage="transform",
            env=BuildEnv(),
        )
        self.assertEqual(dict(ctx.frontmatter), {})


class TestEmit(unittest.TestCase):
    def _ctx(self):
        return build_context(
            page_slug="pages/p",
            frontmatter={},
            stage="transform",
            env=BuildEnv(),
        )

    def test_emit_template_vars_accumulates(self) -> None:
        ctx, writes = self._ctx()
        ctx.emit_template_vars({"a": 1})
        ctx.emit_template_vars({"b": 2})
        self.assertEqual(writes.template_vars, {"a": 1, "b": 2})

    def test_emit_vault_template_vars_lands_under_reserved_key(self) -> None:
        ctx, writes = self._ctx()
        ctx.emit_vault_template_vars({"v": 1})
        msg = writes_to_result_payload(None, writes)
        self.assertEqual(msg["template_vars"]["__vault__"], {"v": 1})

    def test_diag_helpers_record_page_slug(self) -> None:
        ctx, writes = self._ctx()
        ctx.warn("careful")
        ctx.info("fyi")
        ctx.error("kaboom")
        severities = [d.severity for d in writes.diagnostics]
        self.assertEqual(severities, ["warn", "info", "error"])
        self.assertEqual(writes.diagnostics[0].page_slug, "pages/p")

    def test_empty_writes_omitted_from_result_payload(self) -> None:
        _ctx, writes = self._ctx()
        msg = writes_to_result_payload({"x": 1}, writes)
        self.assertEqual(msg, {"type": "result", "payload": {"x": 1}})

    def test_empty_page_slug_yields_no_page_slug_on_diag(self) -> None:
        ctx, writes = build_context(
            page_slug="",
            frontmatter={},
            stage="transform",
            env=BuildEnv(),
        )
        ctx.warn("x")
        self.assertIsNone(writes.diagnostics[0].page_slug)


if __name__ == "__main__":
    unittest.main()
