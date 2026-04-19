"""
End-to-end tests for the persistent-mode ``run()`` loop (REQ-3210 /
CON-3201). Feeds host messages through an in-memory stdin and reads
back the hook's responses from an in-memory stdout — no subprocess.
"""

from __future__ import annotations

import io
import json
import unittest
from typing import Any, List

from zetl_ast import AST_VERSION, Wikilink, dispatch, on_node, run, run_one_shot
from zetl_ast.dispatch import DispatchTable

from .fixtures import sample_ast


def _drive(lines: List[dict], transform) -> List[dict]:
    """Feed ``lines`` through the run loop and parse responses."""
    stdin = io.StringIO("\n".join(json.dumps(m) for m in lines) + "\n")
    stdout = io.StringIO()
    stderr = io.StringIO()
    run(
        transform,
        hook_id="unit",
        version="0.0.1",
        stdin=stdin,
        stdout=stdout,
        stderr=stderr,
    )
    raw = [ln for ln in stdout.getvalue().splitlines() if ln]
    return [json.loads(ln) for ln in raw]


class TestRunLoop(unittest.TestCase):
    def test_handshake_is_first_line(self) -> None:
        responses = _drive([{"type": "shutdown"}], lambda ast, ctx: ast)
        self.assertEqual(responses[0]["zetl_ast"], 1)
        self.assertEqual(responses[0]["hook"], "unit")
        self.assertIs(responses[0]["ready"], True)

    def test_init_run_finalise_echo(self) -> None:
        def identity(payload: Any, ctx) -> Any:
            return payload

        responses = _drive(
            [
                {
                    "type": "init",
                    "stage": "transform",
                    "zetl_version": "0.5.0",
                    "ast_schema_version": AST_VERSION,
                    "ctx": {"vault_root": "/v"},
                },
                {
                    "type": "run",
                    "page_slug": "p",
                    "frontmatter": {"title": "t"},
                    "payload": {"hello": "world"},
                    "deadline_ms": 100,
                },
                {"type": "finalise"},
                {"type": "shutdown"},
            ],
            identity,
        )
        # [handshake, init result, run result, finalise result]
        self.assertEqual(len(responses), 4)
        self.assertEqual(responses[1], {"type": "result", "payload": None})
        self.assertEqual(responses[2]["type"], "result")
        self.assertEqual(responses[2]["payload"], {"hello": "world"})
        self.assertEqual(responses[3], {"type": "result", "payload": None})

    def test_hook_exception_is_typed_error(self) -> None:
        def boom(payload: Any, ctx) -> Any:
            raise RuntimeError("nope")

        responses = _drive(
            [
                {
                    "type": "run",
                    "page_slug": "p",
                    "frontmatter": {},
                    "payload": {},
                    "deadline_ms": 0,
                },
                {"type": "shutdown"},
            ],
            boom,
        )
        self.assertEqual(responses[1]["type"], "error")
        self.assertEqual(responses[1]["reason"], "hook_exception")
        self.assertIn("nope", responses[1]["detail"])

    def test_ast_version_mismatch_breaks_loop(self) -> None:
        responses = _drive(
            [
                {
                    "type": "init",
                    "stage": "transform",
                    "zetl_version": "9.9.9",
                    "ast_schema_version": "999.0",
                    "ctx": {},
                },
                # This run should never be processed — the loop breaks after
                # the error response.
                {
                    "type": "run",
                    "page_slug": "p",
                    "frontmatter": {},
                    "payload": {},
                    "deadline_ms": 0,
                },
                {"type": "shutdown"},
            ],
            lambda x, ctx: x,
        )
        # [handshake, error]
        self.assertEqual(len(responses), 2)
        self.assertEqual(responses[1]["type"], "error")
        self.assertEqual(responses[1]["reason"], "ast_version_mismatch")

    def test_context_emits_propagate_to_response(self) -> None:
        def emit(payload: Any, ctx) -> Any:
            ctx.emit_template_vars({"greet": "hi"})
            ctx.emit_vault_template_vars({"v": 1})
            ctx.emit_build_data({"graph": [1, 2]})
            ctx.warn("careful")
            return payload

        responses = _drive(
            [
                {
                    "type": "run",
                    "page_slug": "pages/p",
                    "frontmatter": {},
                    "payload": {"x": 1},
                    "deadline_ms": 0,
                },
                {"type": "shutdown"},
            ],
            emit,
        )
        result = responses[1]
        self.assertEqual(result["template_vars"]["greet"], "hi")
        self.assertEqual(result["template_vars"]["__vault__"], {"v": 1})
        self.assertEqual(result["build_data"], {"graph": [1, 2]})
        self.assertEqual(result["diagnostics"][0]["severity"], "warn")
        self.assertEqual(result["diagnostics"][0]["page_slug"], "pages/p")

    def test_init_populates_ctx_env(self) -> None:
        captured: List[Any] = []

        def capture(payload: Any, ctx) -> Any:
            captured.append(ctx.env)
            return payload

        _drive(
            [
                {
                    "type": "init",
                    "stage": "transform",
                    "zetl_version": "1.2.3",
                    "ast_schema_version": AST_VERSION,
                    "ctx": {"vault_root": "/v", "theme": "mytheme", "verbose": True},
                },
                {
                    "type": "run",
                    "page_slug": "p",
                    "frontmatter": {},
                    "payload": {},
                    "deadline_ms": 0,
                },
                {"type": "shutdown"},
            ],
            capture,
        )
        env = captured[0]
        self.assertEqual(env.vault_root, "/v")
        self.assertEqual(env.theme, "mytheme")
        self.assertIs(env.verbose, True)
        self.assertEqual(env.zetl_version, "1.2.3")
        self.assertEqual(env.ast_schema_version, AST_VERSION)

    def test_dispatch_interop_through_run(self) -> None:
        """``run()`` pairs with ``dispatch``/``on_node`` without seams."""

        @on_node(Wikilink)
        def fill_alias(node, ctx):
            if not node.get("alias"):
                node["alias"] = node["target"]

        def transform(ast, ctx):
            table: DispatchTable = {}
            fill_alias(table)
            return dispatch(ast, ctx, table)

        ast = sample_ast()
        responses = _drive(
            [
                {
                    "type": "run",
                    "page_slug": "p",
                    "frontmatter": ast["frontmatter"],
                    "payload": ast,
                    "deadline_ms": 100,
                },
                {"type": "shutdown"},
            ],
            transform,
        )
        paragraph = next(
            c for c in responses[1]["payload"]["children"] if c["type"] == "Paragraph"
        )
        wikilink = next(c for c in paragraph["children"] if c["type"] == "Wikilink")
        self.assertEqual(wikilink["alias"], "Other page")

    def test_malformed_line_yields_protocol_error_response(self) -> None:
        stdin = io.StringIO('not valid json\n{"type":"shutdown"}\n')
        stdout = io.StringIO()
        run(
            lambda x, ctx: x,
            hook_id="u",
            stdin=stdin,
            stdout=stdout,
        )
        responses = [json.loads(ln) for ln in stdout.getvalue().splitlines() if ln]
        # [handshake, error]
        self.assertEqual(responses[1]["type"], "error")
        self.assertEqual(responses[1]["reason"], "protocol_error")


class TestRunOneShot(unittest.TestCase):
    def test_writes_json_to_stdout(self) -> None:
        stdin = io.StringIO(json.dumps({"type": "Document", "children": []}))
        stdout = io.StringIO()
        run_one_shot(
            lambda ast, ctx: {**ast, "frontmatter": {"x": 1}},
            hook_id="one",
            stdin=stdin,
            stdout=stdout,
        )
        out = json.loads(stdout.getvalue())
        self.assertEqual(out["frontmatter"], {"x": 1})


if __name__ == "__main__":
    unittest.main()
