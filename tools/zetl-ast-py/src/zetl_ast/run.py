"""
Persistent-mode run loop for zetl hooks authored in Python (REQ-3210).

Handles both **persistent mode** (handshake + line-delimited JSON loop —
the path zetl takes) and **one-shot mode** (stdin → stdout → exit, for
filter-style integrations). The same transform works in both; the helper
chooses the mode from how the entry point is called.

Example::

    from zetl_ast import run, Wikilink, dispatch, on_node

    @on_node(Wikilink)
    def autolink(node, ctx):
        if not node.get("alias"):
            node["alias"] = node["target"]

    def transform(ast, ctx):
        table = {}
        autolink(table)
        return dispatch(ast, ctx, table)

    if __name__ == "__main__":
        run(transform, hook_id="autolinker", version="0.1.0")

The entry point blocks until zetl sends ``shutdown``. On any fatal
protocol error the loop exits and the helper closes stdout cleanly —
zetl then treats the hook as dead for REQ-3207 purposes.
"""

from __future__ import annotations

import dataclasses
import json
import os
import sys
import traceback
from typing import Any, Callable, IO, Optional

from .context import (
    BuildEnv,
    Context,
    build_context,
    writes_to_result_payload,
)
from .protocol import (
    ProtocolError,
    check_ast_major,
    handshake_line,
    parse_host_message,
    read_lines,
    serialize_hook_message,
)

#: Transform signature. ``input`` is the stage-appropriate payload
#: (``str`` for pre-parse / post-render, AST-dict for transform) and
#: ``ctx`` is a :class:`Context`. Return a transformed payload.
TransformFn = Callable[[Any, Context], Any]


def run(
    transform: TransformFn,
    *,
    hook_id: Optional[str] = None,
    version: str = "0.0.0",
    stdin: Optional[IO[str]] = None,
    stdout: Optional[IO[str]] = None,
    stderr: Optional[IO[str]] = None,
    exit_on_fatal: Optional[bool] = None,
) -> None:
    """Persistent-mode entry point.

    Writes the startup handshake, then loops over host messages:
    ``init`` / ``run`` / ``finalise`` / ``shutdown``. Exceptions from
    ``transform`` become typed ``error`` responses so zetl's REQ-3207
    policy decides whether to continue.

    Parameters mirror :mod:`zetl-ast-js`'s ``run()``. ``exit_on_fatal``
    defaults to ``True`` only when the caller did not override stdin —
    tests pass their own streams and do not want ``SystemExit`` raised.
    """
    stdin_s = stdin if stdin is not None else sys.stdin
    stdout_s = stdout if stdout is not None else sys.stdout
    stderr_s = stderr if stderr is not None else sys.stderr
    if exit_on_fatal is None:
        exit_on_fatal = stdin is None
    resolved_hook_id = hook_id or os.environ.get("ZETL_EXTENSION_ID", "zetl-hook")

    try:
        stdout_s.write(handshake_line(resolved_hook_id, version))
        stdout_s.flush()

        env = _default_env_from_process(resolved_hook_id)
        stage = os.environ.get("ZETL_STAGE", "transform")

        for line in read_lines(stdin_s):
            try:
                msg = parse_host_message(line)
            except ProtocolError as e:
                stdout_s.write(
                    serialize_hook_message(
                        {
                            "type": "error",
                            "reason": "protocol_error",
                            "detail": str(e),
                        }
                    )
                )
                stdout_s.flush()
                continue

            kind = msg["type"]
            if kind == "shutdown":
                break

            if kind == "init":
                try:
                    check_ast_major(msg["ast_schema_version"])
                except ProtocolError as e:
                    stdout_s.write(
                        serialize_hook_message(
                            {
                                "type": "error",
                                "reason": "ast_version_mismatch",
                                "detail": str(e),
                            }
                        )
                    )
                    stdout_s.flush()
                    break
                stage = msg["stage"] or stage
                env = _env_from_init(
                    msg["ctx"],
                    resolved_hook_id,
                    env,
                    zetl_version=msg.get("zetl_version") or None,
                    ast_schema_version=msg.get("ast_schema_version") or None,
                )
                stdout_s.write(
                    serialize_hook_message({"type": "result", "payload": None})
                )
                stdout_s.flush()
                continue

            if kind == "finalise":
                stdout_s.write(
                    serialize_hook_message({"type": "result", "payload": None})
                )
                stdout_s.flush()
                continue

            # kind == "run"
            ctx, writes = build_context(
                page_slug=msg["page_slug"],
                frontmatter=msg["frontmatter"],
                stage=stage,
                env=env,
                build_data=msg.get("build_data"),
            )
            try:
                out = transform(msg["payload"], ctx)
            except Exception as e:  # noqa: BLE001 — surfaced as typed error
                stdout_s.write(
                    serialize_hook_message(
                        {
                            "type": "error",
                            "reason": "hook_exception",
                            "detail": _error_detail(e),
                        }
                    )
                )
                stdout_s.flush()
                continue

            stdout_s.write(
                serialize_hook_message(writes_to_result_payload(out, writes))
            )
            stdout_s.flush()
    except Exception as e:  # noqa: BLE001 — last-resort log + exit
        stderr_s.write(f"zetl-ast-py fatal: {e}\n")
        if exit_on_fatal:
            raise SystemExit(1) from e


def run_one_shot(
    transform: TransformFn,
    *,
    hook_id: Optional[str] = None,
    stdin: Optional[IO[str]] = None,
    stdout: Optional[IO[str]] = None,
) -> None:
    """One-shot mode — read all of stdin as one JSON document, apply
    ``transform``, write the result to stdout, exit.

    For filter-style integrations where a wrapper invokes the hook per
    page. No handshake is emitted. ``template_vars`` / ``diagnostics``
    / ``build_data`` accumulated on the context are dropped — there is
    no back-channel in one-shot mode. If a hook relies on those
    channels, use :func:`run` instead.
    """
    stdin_s = stdin if stdin is not None else sys.stdin
    stdout_s = stdout if stdout is not None else sys.stdout
    resolved_hook_id = hook_id or os.environ.get("ZETL_EXTENSION_ID", "zetl-hook")

    raw = stdin_s.read()
    parsed = json.loads(raw)
    env = _default_env_from_process(resolved_hook_id)
    ctx, _writes = build_context(
        page_slug=os.environ.get("ZETL_PAGE_SLUG", ""),
        frontmatter={},
        stage=os.environ.get("ZETL_STAGE", "transform"),
        env=env,
    )
    out = transform(parsed, ctx)
    stdout_s.write(json.dumps(out, separators=(",", ":")))
    stdout_s.flush()


# ── Internals ──────────────────────────────────────────────────────────────


def _default_env_from_process(hook_id: str) -> BuildEnv:
    return BuildEnv(
        mode=os.environ.get("ZETL_MODE", "build"),
        theme=os.environ.get("ZETL_THEME") or None,
        vault_root=os.environ.get("ZETL_VAULT_ROOT", ""),
        out_dir=os.environ.get("ZETL_OUT_DIR") or None,
        verbose=os.environ.get("ZETL_VERBOSE") == "true",
        at=os.environ.get("ZETL_AT") or None,
        hook_path=os.environ.get("ZETL_HOOK_PATH") or None,
        extension_id=os.environ.get("ZETL_EXTENSION_ID", hook_id),
        zetl_version=os.environ.get("ZETL_VERSION") or None,
        ast_schema_version=os.environ.get("ZETL_AST_SCHEMA_VERSION") or None,
    )


def _env_from_init(
    ctx: Any,
    hook_id: str,
    fallback: BuildEnv,
    *,
    zetl_version: Optional[str],
    ast_schema_version: Optional[str],
) -> BuildEnv:
    if not isinstance(ctx, dict):
        ctx = {}
    fields = {f.name for f in dataclasses.fields(BuildEnv)}
    merged = {f: getattr(fallback, f) for f in fields}
    for key in ("mode", "theme", "vault_root", "out_dir", "verbose", "at", "hook_path", "extension_id"):
        if key in ctx:
            merged[key] = ctx[key]
    if zetl_version:
        merged["zetl_version"] = zetl_version
    if ast_schema_version:
        merged["ast_schema_version"] = ast_schema_version
    if not merged.get("extension_id"):
        merged["extension_id"] = hook_id
    return BuildEnv(**merged)


def _error_detail(e: BaseException) -> str:
    tb = "".join(traceback.format_exception(type(e), e, e.__traceback__))
    return tb if tb.strip() else f"{type(e).__name__}: {e}"
