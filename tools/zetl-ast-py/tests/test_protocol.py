"""
Protocol-layer tests for ``zetl_ast.protocol`` (SPEC-032 CON-3201 / REQ-3215).

Covers line-framed JSON parsing, handshake emission, AST-major gating,
and the result/error shapes that the persistent-hook run-loop writes
back to zetl.
"""

from __future__ import annotations

import io
import json
import unittest

from zetl_ast import (
    AST_MAJOR,
    AST_VERSION,
    ProtocolError,
    check_ast_major,
    handshake_line,
    parse_host_message,
    read_lines,
    serialize_hook_message,
)


class TestHandshake(unittest.TestCase):
    def test_emits_zetl_ast_and_ready(self) -> None:
        line = handshake_line("my-hook", "1.2.3")
        self.assertTrue(line.endswith("\n"))
        payload = json.loads(line)
        self.assertEqual(payload["zetl_ast"], AST_MAJOR)
        self.assertEqual(payload["hook"], "my-hook")
        self.assertEqual(payload["version"], "1.2.3")
        self.assertIs(payload["ready"], True)

    def test_ready_false_is_supported(self) -> None:
        line = handshake_line("x", "0", ready=False)
        payload = json.loads(line)
        self.assertIs(payload["ready"], False)


class TestParseHostMessage(unittest.TestCase):
    def test_init_defaults(self) -> None:
        msg = parse_host_message(
            json.dumps(
                {
                    "type": "init",
                    "stage": "transform",
                    "zetl_version": "0.5.0",
                    "ast_schema_version": AST_VERSION,
                    "ctx": {"vault_root": "/v"},
                }
            )
        )
        self.assertEqual(msg["type"], "init")
        self.assertEqual(msg["stage"], "transform")
        self.assertEqual(msg["ast_schema_version"], AST_VERSION)
        self.assertEqual(msg["ctx"], {"vault_root": "/v"})

    def test_run_coerces_frontmatter_and_deadline(self) -> None:
        msg = parse_host_message(
            json.dumps(
                {
                    "type": "run",
                    "page_slug": "pages/foo",
                    "frontmatter": {"title": "x"},
                    "payload": {"body": "hi"},
                    "deadline_ms": 250.0,
                }
            )
        )
        self.assertEqual(msg["page_slug"], "pages/foo")
        self.assertEqual(msg["frontmatter"], {"title": "x"})
        self.assertEqual(msg["payload"], {"body": "hi"})
        self.assertEqual(msg["deadline_ms"], 250)

    def test_run_tolerates_non_object_frontmatter(self) -> None:
        msg = parse_host_message(
            json.dumps(
                {
                    "type": "run",
                    "page_slug": "p",
                    "frontmatter": [],
                    "payload": 1,
                    "deadline_ms": 0,
                }
            )
        )
        self.assertEqual(msg["frontmatter"], {})

    def test_finalise_and_shutdown(self) -> None:
        self.assertEqual(parse_host_message('{"type":"finalise"}'), {"type": "finalise"})
        self.assertEqual(parse_host_message('{"type":"shutdown"}'), {"type": "shutdown"})

    def test_malformed_json_raises_parse(self) -> None:
        with self.assertRaises(ProtocolError) as cm:
            parse_host_message("not json")
        self.assertEqual(cm.exception.kind, "parse")

    def test_unknown_type_raises_unexpected(self) -> None:
        with self.assertRaises(ProtocolError) as cm:
            parse_host_message('{"type":"noise"}')
        self.assertEqual(cm.exception.kind, "unexpected")

    def test_non_object_line_raises_parse(self) -> None:
        with self.assertRaises(ProtocolError) as cm:
            parse_host_message("42")
        self.assertEqual(cm.exception.kind, "parse")


class TestSerializeHookMessage(unittest.TestCase):
    def test_result_is_single_line(self) -> None:
        line = serialize_hook_message({"type": "result", "payload": {"x": 1}})
        self.assertTrue(line.endswith("\n"))
        self.assertEqual(line.count("\n"), 1)
        self.assertEqual(json.loads(line), {"type": "result", "payload": {"x": 1}})

    def test_error_shape(self) -> None:
        line = serialize_hook_message(
            {"type": "error", "reason": "hook_exception", "detail": "boom"}
        )
        msg = json.loads(line)
        self.assertEqual(msg["type"], "error")
        self.assertEqual(msg["reason"], "hook_exception")


class TestReadLines(unittest.TestCase):
    def test_yields_lines_and_skips_blank(self) -> None:
        stream = io.StringIO("a\n\nb\r\n")
        self.assertEqual(list(read_lines(stream)), ["a", "b"])


class TestCheckAstMajor(unittest.TestCase):
    def test_empty_is_tolerated(self) -> None:
        check_ast_major("")

    def test_matching_major_passes(self) -> None:
        check_ast_major(f"{AST_MAJOR}.7")
        check_ast_major(str(AST_MAJOR))

    def test_mismatched_major_raises(self) -> None:
        with self.assertRaises(ProtocolError) as cm:
            check_ast_major(f"{AST_MAJOR + 1}.0")
        self.assertEqual(cm.exception.kind, "ast_major_mismatch")

    def test_malformed_raises_parse(self) -> None:
        with self.assertRaises(ProtocolError) as cm:
            check_ast_major("vNEXT.0")
        self.assertEqual(cm.exception.kind, "parse")


if __name__ == "__main__":
    unittest.main()
