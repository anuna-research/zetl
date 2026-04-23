"""
Wire-protocol types for persistent-mode hooks
(SPEC-032 CON-3201 / NFR-3207 — mirror of ``src/hooks/persistent.rs``).

ztl speaks line-delimited JSON: one message per line on stdin; the hook
replies with one line on stdout per host message (except ``shutdown``,
which has no response).

Kept dependency-free — no ``pydantic`` / ``msgspec`` — so the package
ships cleanly on every Python 3.9+. All parsed messages are plain dicts
with a typed ``"type"`` discriminator so authors can pattern-match with
``match`` (3.10+) or ``if/elif`` chains.
"""

from __future__ import annotations

import json
import sys
from typing import Any, Dict, IO, Iterator

from .ast import AST_MAJOR


class ProtocolError(Exception):
    """Raised when a host message cannot be parsed or violates the protocol."""

    __slots__ = ("kind",)

    def __init__(self, message: str, kind: str) -> None:
        super().__init__(message)
        self.kind = kind  # "parse" | "unexpected" | "ast_major_mismatch"


def handshake_line(hook: str, version: str, *, ready: bool = True) -> str:
    """Build the CON-3201 startup handshake line (trailing newline included)."""
    payload = {
        "ztl_ast": AST_MAJOR,
        "hook": hook,
        "version": version,
        "ready": ready,
    }
    return json.dumps(payload, separators=(",", ":")) + "\n"


def parse_host_message(line: str) -> Dict[str, Any]:
    """Parse one line of JSON into a host message dict.

    The helper does structural validation only — returns ``init`` /
    ``run`` / ``finalise`` / ``shutdown`` dicts with sane defaults
    filled in. Unknown types raise ``ProtocolError('unexpected')`` so
    callers can react (typically: send an ``error`` response and drop
    the frame).
    """
    line = line.strip()
    if not line:
        raise ProtocolError("empty host message line", "parse")
    try:
        value = json.loads(line)
    except json.JSONDecodeError as e:
        raise ProtocolError(f"malformed JSON line: {e}", "parse") from e
    if not isinstance(value, dict):
        raise ProtocolError(
            f"host message must be an object, got {type(value).__name__}",
            "parse",
        )
    t = value.get("type")
    if t == "init":
        ctx = value.get("ctx")
        return {
            "type": "init",
            "stage": str(value.get("stage", "")),
            "ztl_version": str(value.get("ztl_version", "")),
            "ast_schema_version": str(value.get("ast_schema_version", "")),
            "ctx": ctx if isinstance(ctx, dict) else {},
        }
    if t == "run":
        fm = value.get("frontmatter")
        return {
            "type": "run",
            "page_slug": str(value.get("page_slug", "")),
            "frontmatter": fm if isinstance(fm, dict) else {},
            "payload": value.get("payload"),
            "deadline_ms": (
                int(value["deadline_ms"])
                if isinstance(value.get("deadline_ms"), (int, float))
                else 0
            ),
            "build_data": (
                value["build_data"]
                if isinstance(value.get("build_data"), dict)
                else {}
            ),
        }
    if t == "finalise":
        return {"type": "finalise"}
    if t == "shutdown":
        return {"type": "shutdown"}
    raise ProtocolError(f"unknown host message type {t!r}", "unexpected")


def serialize_hook_message(msg: Dict[str, Any]) -> str:
    """Serialise a hook ``result`` / ``error`` message with trailing newline."""
    return json.dumps(msg, separators=(",", ":")) + "\n"


def read_lines(stream: IO[Any] = sys.stdin) -> Iterator[str]:
    """Yield line-delimited strings from ``stream`` until EOF.

    Blank lines and the lone ``\\r`` before ``\\n`` are stripped so
    host messages parse cleanly. A partial trailing line (no newline)
    is yielded if non-empty when the stream closes.
    """
    for raw in stream:
        line = raw.rstrip("\r\n")
        if line:
            yield line


def check_ast_major(schema_version: str) -> None:
    """Raise ``ProtocolError`` if ``schema_version`` advertises an incompatible major.

    The empty / absent case is tolerated — older ztl builds sent init
    payloads without the field, and REQ-3215 prohibits bumping the
    handshake requirements on minor releases.
    """
    if not schema_version:
        return
    head, _, _ = schema_version.partition(".")
    try:
        major = int(head)
    except ValueError as e:
        raise ProtocolError(
            f"malformed ast_schema_version {schema_version!r}",
            "parse",
        ) from e
    if major != AST_MAJOR:
        raise ProtocolError(
            f"ztl advertised ast_schema_version={schema_version}; "
            f"this hook speaks major {AST_MAJOR}.",
            "ast_major_mismatch",
        )
