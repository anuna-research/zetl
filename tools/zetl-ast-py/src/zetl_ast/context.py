"""
Per-invocation context for persistent-mode hooks.

Shapes mirror REQ-3219 (build_data) and REQ-3220 (build context). Emit
helpers accumulate writes into a response payload that :mod:`zetl_ast.run`
flushes back to zetl as part of the ``result`` message.

All state here is plain-Python (dicts, lists, frozen dataclasses) — no
external deps. The same helpers drive both persistent-mode and one-shot
hooks; the latter discards the accumulated writes since the filter-style
protocol has nowhere to attach them.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Dict, List, Mapping, MutableMapping, Optional

Severity = str  # "info" | "warn" | "error"


@dataclass
class Diagnostic:
    """A single structured diagnostic attached to a hook response."""

    severity: Severity
    message: str
    page_slug: Optional[str] = None

    def to_wire(self) -> Dict[str, Any]:
        out: Dict[str, Any] = {"severity": self.severity, "message": self.message}
        if self.page_slug is not None:
            out["page_slug"] = self.page_slug
        return out


class BuildDataView:
    """Read-only snapshot view of the shared ``build_data`` store (REQ-3219).

    The underlying mapping is keyed by writing-hook's ``extension_id``;
    each value is a mapping of that hook's own writes.
    """

    __slots__ = ("_by_extension",)

    def __init__(self, raw: Any) -> None:
        if isinstance(raw, Mapping):
            self._by_extension: Mapping[str, Mapping[str, Any]] = {
                str(k): dict(v) if isinstance(v, Mapping) else {}
                for k, v in raw.items()
            }
        else:
            self._by_extension = {}

    @property
    def by_extension(self) -> Mapping[str, Mapping[str, Any]]:
        return self._by_extension

    def get(self, extension_id: str, key: str, default: Any = None) -> Any:
        bag = self._by_extension.get(extension_id)
        if bag is None:
            return default
        return bag.get(key, default)

    def __contains__(self, extension_id: object) -> bool:
        return isinstance(extension_id, str) and extension_id in self._by_extension


@dataclass(frozen=True)
class BuildEnv:
    """Shape of the REQ-3220 context block zetl sends in ``init``."""

    mode: str = "build"
    theme: Optional[str] = None
    vault_root: str = ""
    out_dir: Optional[str] = None
    verbose: bool = False
    at: Optional[str] = None
    hook_path: Optional[str] = None
    extension_id: str = "zetl-hook"
    zetl_version: Optional[str] = None
    ast_schema_version: Optional[str] = None


@dataclass
class ContextWrites:
    """Mutable accumulator drained into the protocol response."""

    build_data: Dict[str, Any] = field(default_factory=dict)
    template_vars: Dict[str, Any] = field(default_factory=dict)
    vault_template_vars: Dict[str, Any] = field(default_factory=dict)
    diagnostics: List[Diagnostic] = field(default_factory=list)


class Context:
    """Per-invocation context passed to the hook's transform function.

    Attributes are read-only from the hook's perspective (except via the
    ``emit_*`` / ``diag`` methods, which accumulate structured writes).
    """

    __slots__ = (
        "page_slug",
        "frontmatter",
        "stage",
        "env",
        "build_data",
        "_writes",
    )

    def __init__(
        self,
        *,
        page_slug: str,
        frontmatter: Mapping[str, Any],
        stage: str,
        env: BuildEnv,
        build_data: BuildDataView,
        writes: ContextWrites,
    ) -> None:
        self.page_slug = page_slug
        self.frontmatter: Mapping[str, Any] = frontmatter
        self.stage = stage
        self.env = env
        self.build_data = build_data
        self._writes = writes

    def emit_build_data(self, writes: Mapping[str, Any]) -> None:
        """Write into the ``build_data`` response under this hook's id (REQ-3219)."""
        self._writes.build_data.update(writes)

    def emit_template_vars(self, writes: Mapping[str, Any]) -> None:
        """Write ``page.ext.<extension_id>.*`` template variables (REQ-3214)."""
        self._writes.template_vars.update(writes)

    def emit_vault_template_vars(self, writes: Mapping[str, Any]) -> None:
        """Write ``vault.ext.<extension_id>.*`` template variables (REQ-3214)."""
        self._writes.vault_template_vars.update(writes)

    def diag(self, severity: Severity, message: str) -> None:
        """Attach a structured diagnostic to the response."""
        page_slug = self.page_slug or None
        self._writes.diagnostics.append(
            Diagnostic(severity=severity, message=message, page_slug=page_slug)
        )

    def warn(self, message: str) -> None:
        self.diag("warn", message)

    def info(self, message: str) -> None:
        self.diag("info", message)

    def error(self, message: str) -> None:
        self.diag("error", message)


def build_context(
    *,
    page_slug: str,
    frontmatter: Any,
    stage: str,
    env: BuildEnv,
    build_data: Any = None,
) -> "tuple[Context, ContextWrites]":
    """Construct a fresh ``(Context, ContextWrites)`` pair for one host message."""
    fm: Mapping[str, Any] = (
        frontmatter if isinstance(frontmatter, Mapping) else {}
    )
    writes = ContextWrites()
    ctx = Context(
        page_slug=page_slug,
        frontmatter=dict(fm),
        stage=stage,
        env=env,
        build_data=BuildDataView(build_data),
        writes=writes,
    )
    return ctx, writes


def writes_to_result_payload(
    payload: Any, writes: ContextWrites
) -> MutableMapping[str, Any]:
    """Build the body of a ``result`` message from a hook return + writes.

    Mirrors ``zetl-ast-js``'s ``resultFromWrites``: empty sections are
    omitted so the wire stays small and diffs stay stable. The special
    ``__vault__`` key on ``template_vars`` carries vault-scoped writes
    (consumed by the ``VaultTemplateVars`` accumulator on the host side).
    """
    msg: Dict[str, Any] = {"type": "result", "payload": payload}
    if writes.diagnostics:
        msg["diagnostics"] = [d.to_wire() for d in writes.diagnostics]
    if writes.template_vars or writes.vault_template_vars:
        tv: Dict[str, Any] = dict(writes.template_vars)
        if writes.vault_template_vars:
            tv["__vault__"] = dict(writes.vault_template_vars)
        msg["template_vars"] = tv
    if writes.build_data:
        msg["build_data"] = dict(writes.build_data)
    return msg
