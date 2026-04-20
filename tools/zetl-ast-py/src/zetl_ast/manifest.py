"""
Manifest helpers for authoring ``<executable>.toml`` sidecars
(SPEC-032 REQ-3206 / REQ-3217 / REQ-3221 / REQ-3224).

Exposes:

* The manifest schema as dataclasses.
* ``render_manifest()`` to emit a minimal canonical TOML document so
  tooling (e.g. ``zetl hook new``) can scaffold sidecars without pulling
  in a full TOML serialiser.
* ``parse_manifest()`` that reads the slice zetl's composition layer
  consumes (``extension_id`` / ``optional`` / ordering / ``ast_type`` /
  ``ast_version`` / ``contract.preserves``). Unknown top-level keys are
  preserved under ``extras`` in source form so callers can round-trip
  foreign metadata when editing existing files.

A full TOML parser is out of scope — the subset documented here is what
zetl reads. For arbitrary TOML, hook authors should use the stdlib
``tomllib`` (3.11+) or ``tomli`` in their own tooling; zetl only consumes
what is modelled here.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Dict, List, Optional, Union

#: Valid ``ast_type`` values at the composition boundary.
AST_TYPES = ("zetl-ext", "mdast-ext", "pandoc-ext")


class ManifestError(ValueError):
    """Raised on malformed manifest input."""

    __slots__ = ("key",)

    def __init__(self, message: str, key: Optional[str] = None) -> None:
        super().__init__(f"{key}: {message}" if key else message)
        self.key = key


@dataclass
class OrderingTable:
    before: Optional[List[str]] = None
    after: Optional[List[str]] = None


@dataclass
class ContractTable:
    """Behavioural-contract block (REQ-3224).

    Only ``preserves`` is read at composition time; other fields on this
    struct are reserved for the Tier-2 property-test runner
    (``task-behavioural-contracts``).
    """

    preserves: Optional[List[str]] = None


@dataclass
class Manifest:
    extension_id: Optional[str] = None
    optional: Optional[bool] = None
    ordering: Optional[OrderingTable] = None
    before: Optional[List[str]] = None
    after: Optional[List[str]] = None
    ast_type: Optional[str] = None
    ast_version: Optional[str] = None
    contract: Optional[ContractTable] = None
    #: Unknown top-level keys preserved verbatim (key → raw TOML value string).
    extras: Dict[str, str] = field(default_factory=dict)


# ── Writer ─────────────────────────────────────────────────────────────────


def render_manifest(manifest: Manifest) -> str:
    """Emit a canonical TOML document for ``manifest``.

    Keys are written in a stable order so generated files diff cleanly.
    Unknown keys in ``extras`` are emitted verbatim after the known ones.
    When ``manifest.ordering`` is populated the ``[ordering]`` table is
    preferred over the top-level ``before`` / ``after`` aliases, matching
    the JS helper.
    """
    out: List[str] = []

    if manifest.extension_id is not None:
        out.append(f"extension_id = {_toml_string(manifest.extension_id)}")
    if manifest.optional is not None:
        out.append(f"optional = {'true' if manifest.optional else 'false'}")
    if manifest.ast_type is not None:
        out.append(f"ast_type = {_toml_string(manifest.ast_type)}")
    if manifest.ast_version is not None:
        out.append(f"ast_version = {_toml_string(manifest.ast_version)}")

    has_ordering_table = manifest.ordering is not None and (
        manifest.ordering.before is not None
        or manifest.ordering.after is not None
    )
    if not has_ordering_table:
        if manifest.before is not None:
            out.append(f"before = {_toml_string_array(manifest.before)}")
        if manifest.after is not None:
            out.append(f"after = {_toml_string_array(manifest.after)}")

    for k, v in manifest.extras.items():
        out.append(f"{k} = {v}")

    if has_ordering_table and manifest.ordering is not None:
        out.append("")
        out.append("[ordering]")
        if manifest.ordering.before is not None:
            out.append(f"before = {_toml_string_array(manifest.ordering.before)}")
        if manifest.ordering.after is not None:
            out.append(f"after = {_toml_string_array(manifest.ordering.after)}")

    if manifest.contract is not None and manifest.contract.preserves is not None:
        out.append("")
        out.append("[contract]")
        out.append(f"preserves = {_toml_string_array(manifest.contract.preserves)}")

    return "\n".join(out) + "\n"


def _toml_string(s: str) -> str:
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def _toml_string_array(xs) -> str:
    if not xs:
        return "[]"
    return "[" + ", ".join(_toml_string(x) for x in xs) + "]"


# ── Parser (subset of TOML used by zetl composition) ───────────────────────

_TomlValue = Union[str, bool, float, int, list]


def parse_manifest(text: str) -> Manifest:
    """Parse a sidecar manifest.

    Supports: top-level scalar keys (string / boolean / string-array),
    ``[ordering]`` and ``[contract]`` tables, ``#`` line comments, and
    blank lines. Unknown tables are skipped (NFR-3206 additive-only).
    """
    manifest = Manifest()
    table: str = "root"  # "root" | "ordering" | "contract" | "unknown"

    for lineno, raw in enumerate(text.splitlines(), start=1):
        line = _strip_comment(raw).strip()
        if not line:
            continue

        if line.startswith("[") and line.endswith("]"):
            name = line[1:-1].strip()
            if name == "ordering":
                if manifest.ordering is None:
                    manifest.ordering = OrderingTable()
                table = "ordering"
            elif name == "contract":
                if manifest.contract is None:
                    manifest.contract = ContractTable()
                table = "contract"
            else:
                table = "unknown"
            continue

        eq = line.find("=")
        if eq < 0:
            raise ManifestError(f"unexpected line at {lineno}: {raw}")
        key = line[:eq].strip()
        value_str = line[eq + 1 :].strip()
        value = _parse_value(value_str, key, lineno)

        if table == "unknown":
            continue
        if table == "ordering":
            if key == "before":
                manifest.ordering.before = _as_string_array(value, "ordering.before")
            elif key == "after":
                manifest.ordering.after = _as_string_array(value, "ordering.after")
            continue
        if table == "contract":
            if key == "preserves":
                manifest.contract.preserves = _as_string_array(
                    value, "contract.preserves"
                )
            continue

        # Root table.
        if key == "extension_id":
            manifest.extension_id = _as_string(value, key)
        elif key == "optional":
            manifest.optional = _as_bool(value, key)
        elif key == "before":
            manifest.before = _as_string_array(value, key)
        elif key == "after":
            manifest.after = _as_string_array(value, key)
        elif key == "ast_type":
            s = _as_string(value, key)
            if s not in AST_TYPES:
                raise ManifestError(
                    f"unknown ast_type {s!r}; expected one of {', '.join(AST_TYPES)}",
                    key,
                )
            manifest.ast_type = s
        elif key == "ast_version":
            manifest.ast_version = _as_string(value, key)
        else:
            manifest.extras[key] = value_str

    return manifest


def resolved_before(manifest: Manifest) -> List[str]:
    """Union of ``[ordering].before`` and top-level ``before`` — canonical first.

    Mirrors ``CompositionManifest::resolved_before`` on the Rust side so
    authors can lint their sidecars identically in either language.
    """
    out: List[str] = []
    if manifest.ordering and manifest.ordering.before:
        out.extend(manifest.ordering.before)
    if manifest.before:
        out.extend(manifest.before)
    return out


def resolved_after(manifest: Manifest) -> List[str]:
    out: List[str] = []
    if manifest.ordering and manifest.ordering.after:
        out.extend(manifest.ordering.after)
    if manifest.after:
        out.extend(manifest.after)
    return out


def default_extension_id(filename: str) -> str:
    """Default ``extension_id`` from a hook filename.

    Matches the Rust composition layer's ``default_extension_id``:
    strip a leading ``\\d+-`` ordering prefix, then strip the final
    extension. ``20-tasks.py`` → ``tasks``; ``callouts`` → ``callouts``.
    """
    # Strip trailing extension (last '.' only).
    dot = filename.rfind(".")
    stem = filename if dot <= 0 else filename[:dot]
    # Strip leading `\d+-` ordering prefix.
    i = 0
    while i < len(stem) and stem[i].isdigit():
        i += 1
    if i > 0 and i < len(stem) and stem[i] == "-":
        return stem[i + 1 :]
    return stem


# ── TOML value helpers ─────────────────────────────────────────────────────


def _parse_value(text: str, key: str, lineno: int) -> _TomlValue:
    if text == "true":
        return True
    if text == "false":
        return False
    if text.startswith('"'):
        return _parse_string(text, key, lineno)
    if text.startswith("["):
        return _parse_array(text, key, lineno)
    # Integer / float scalar.
    try:
        if "." in text:
            return float(text)
        return int(text)
    except ValueError as e:
        raise ManifestError(
            f"line {lineno}: unsupported value {text!r}", key
        ) from e


def _parse_string(text: str, key: str, lineno: int) -> str:
    if not (text.startswith('"') and text.endswith('"') and len(text) >= 2):
        raise ManifestError(f"line {lineno}: malformed string {text!r}", key)
    body = text[1:-1]
    out: List[str] = []
    i = 0
    while i < len(body):
        c = body[i]
        if c == "\\" and i + 1 < len(body):
            nxt = body[i + 1]
            if nxt == "n":
                out.append("\n")
            elif nxt == "t":
                out.append("\t")
            elif nxt == "r":
                out.append("\r")
            elif nxt == "\\":
                out.append("\\")
            elif nxt == '"':
                out.append('"')
            else:
                raise ManifestError(f"line {lineno}: unknown escape \\{nxt}", key)
            i += 2
            continue
        out.append(c)
        i += 1
    return "".join(out)


def _parse_array(text: str, key: str, lineno: int) -> list:
    if not (text.startswith("[") and text.endswith("]")):
        raise ManifestError(f"line {lineno}: malformed array {text!r}", key)
    body = text[1:-1].strip()
    if not body:
        return []

    parts: List[str] = []
    depth = 0
    in_string = False
    start = 0
    i = 0
    while i < len(body):
        c = body[i]
        if in_string:
            if c == "\\":
                i += 2
                continue
            if c == '"':
                in_string = False
            i += 1
            continue
        if c == '"':
            in_string = True
        elif c == "[":
            depth += 1
        elif c == "]":
            depth -= 1
        elif c == "," and depth == 0:
            parts.append(body[start:i].strip())
            start = i + 1
        i += 1
    parts.append(body[start:].strip())
    return [_parse_value(p, key, lineno) for p in parts if p]


def _strip_comment(line: str) -> str:
    in_string = False
    prev = ""
    for i, c in enumerate(line):
        if c == '"' and prev != "\\":
            in_string = not in_string
        elif c == "#" and not in_string:
            return line[:i]
        prev = c
    return line


def _as_string(v: _TomlValue, key: str) -> str:
    if not isinstance(v, str):
        raise ManifestError("expected a string", key)
    return v


def _as_bool(v: _TomlValue, key: str) -> bool:
    if not isinstance(v, bool):
        raise ManifestError("expected a boolean", key)
    return v


def _as_string_array(v: _TomlValue, key: str) -> List[str]:
    if not isinstance(v, list):
        raise ManifestError("expected an array", key)
    out: List[str] = []
    for i, x in enumerate(v):
        if not isinstance(x, str):
            raise ManifestError(f"element {i} is not a string", key)
        out.append(x)
    return out
