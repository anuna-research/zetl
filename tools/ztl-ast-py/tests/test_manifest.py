"""
Manifest-helper tests (SPEC-032 REQ-3206 / REQ-3217 / REQ-3221 / REQ-3224).

Parity with ``tools/ztl-ast-js/test/manifest.test.ts`` — same round-trip
shapes, same ordering-alias resolution, same canonical key order in the
writer.
"""

from __future__ import annotations

import unittest

from ztl_ast import (
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


class TestRender(unittest.TestCase):
    def test_minimal_emits_canonical_order(self) -> None:
        out = render_manifest(
            Manifest(extension_id="callouts", optional=True, ast_version="1.0")
        )
        self.assertEqual(
            out,
            'extension_id = "callouts"\noptional = true\nast_version = "1.0"\n',
        )

    def test_ordering_table_preferred_over_top_level_aliases(self) -> None:
        out = render_manifest(
            Manifest(
                extension_id="x",
                ordering=OrderingTable(before=["wikilinks"], after=["tags"]),
                before=["ignored"],
            )
        )
        self.assertIn("[ordering]", out)
        self.assertIn('before = ["wikilinks"]', out)
        self.assertIn('after = ["tags"]', out)
        self.assertNotIn("ignored", out)

    def test_top_level_aliases_survive_when_ordering_absent(self) -> None:
        out = render_manifest(Manifest(extension_id="x", before=["a"], after=["b"]))
        self.assertIn('before = ["a"]', out)
        self.assertIn('after = ["b"]', out)
        self.assertNotIn("[ordering]", out)

    def test_contract_preserves_in_its_own_table(self) -> None:
        out = render_manifest(
            Manifest(
                extension_id="x",
                contract=ContractTable(preserves=["Wikilink", "Embed"]),
            )
        )
        self.assertIn("\n[contract]\n", out)
        self.assertIn('preserves = ["Wikilink", "Embed"]', out)

    def test_extras_are_emitted_verbatim(self) -> None:
        out = render_manifest(
            Manifest(
                extension_id="x",
                extras={"authored_by": '"me"'},
            )
        )
        self.assertIn('authored_by = "me"', out)

    def test_string_escape_is_toml_safe(self) -> None:
        out = render_manifest(Manifest(extension_id='quote"in'))
        self.assertIn('extension_id = "quote\\"in"', out)


class TestParse(unittest.TestCase):
    def test_parses_full_manifest(self) -> None:
        src = """
extension_id = "callouts"
optional = true
ast_type = "ztl-ext"
ast_version = "1.0"

[ordering]
before = ["wikilinks"]
after = ["tags", "toc"]

[contract]
preserves = ["Wikilink"]
"""
        m = parse_manifest(src)
        self.assertEqual(m.extension_id, "callouts")
        self.assertIs(m.optional, True)
        self.assertEqual(m.ast_type, "ztl-ext")
        self.assertEqual(m.ast_version, "1.0")
        self.assertEqual(m.ordering.before, ["wikilinks"])
        self.assertEqual(m.ordering.after, ["tags", "toc"])
        self.assertEqual(m.contract.preserves, ["Wikilink"])

    def test_unknown_ast_type_is_rejected(self) -> None:
        with self.assertRaises(ManifestError) as cm:
            parse_manifest('ast_type = "fictional-ext"\n')
        self.assertEqual(cm.exception.key, "ast_type")

    def test_unknown_top_level_key_lands_in_extras(self) -> None:
        m = parse_manifest('extension_id = "x"\nfuture_field = 42\n')
        self.assertEqual(m.extras["future_field"], "42")

    def test_unknown_table_is_skipped(self) -> None:
        src = (
            'extension_id = "x"\n\n'
            "[experimental]\n"
            'feature = "foo"\n\n'
            "[contract]\n"
            'preserves = ["Embed"]\n'
        )
        m = parse_manifest(src)
        self.assertEqual(m.extension_id, "x")
        self.assertEqual(m.contract.preserves, ["Embed"])

    def test_inline_comments_and_blank_lines(self) -> None:
        src = '# top\nextension_id = "x"  # inline\n\n'
        m = parse_manifest(src)
        self.assertEqual(m.extension_id, "x")

    def test_malformed_line_raises(self) -> None:
        with self.assertRaises(ManifestError):
            parse_manifest("no equals sign\n")

    def test_round_trip_preserves_key_values(self) -> None:
        original = Manifest(
            extension_id="callouts",
            ordering=OrderingTable(before=["wikilinks"]),
            contract=ContractTable(preserves=["Wikilink"]),
        )
        parsed = parse_manifest(render_manifest(original))
        self.assertEqual(parsed.extension_id, "callouts")
        self.assertEqual(parsed.ordering.before, ["wikilinks"])
        self.assertEqual(parsed.contract.preserves, ["Wikilink"])


class TestResolvedOrdering(unittest.TestCase):
    def test_table_and_alias_are_unioned(self) -> None:
        m = Manifest(
            ordering=OrderingTable(before=["a"], after=["b"]),
            before=["c"],
            after=["d"],
        )
        self.assertEqual(resolved_before(m), ["a", "c"])
        self.assertEqual(resolved_after(m), ["b", "d"])

    def test_empty_manifest_yields_empty_lists(self) -> None:
        self.assertEqual(resolved_before(Manifest()), [])
        self.assertEqual(resolved_after(Manifest()), [])


class TestDefaultExtensionId(unittest.TestCase):
    def test_strips_numeric_prefix_and_extension(self) -> None:
        self.assertEqual(default_extension_id("20-tasks.py"), "tasks")

    def test_no_prefix_is_a_noop(self) -> None:
        self.assertEqual(default_extension_id("callouts.py"), "callouts")

    def test_no_extension(self) -> None:
        self.assertEqual(default_extension_id("callouts"), "callouts")

    def test_prefix_without_dash_is_not_stripped(self) -> None:
        # "20foo.py" — digits only count when followed by `-`.
        self.assertEqual(default_extension_id("20foo.py"), "20foo")


if __name__ == "__main__":
    unittest.main()
