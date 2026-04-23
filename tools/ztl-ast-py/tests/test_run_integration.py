"""
Subprocess-level integration test for ``run()`` (SPEC-032 CON-3201).

Spawns a real Python process that uses ``ztl_ast.run`` as its entry
point and drives it through a full handshake → init → run → finalise →
shutdown cycle from the outside — exactly the shape
``src/hooks/persistent.rs`` drives it with on the ztl side.

Running as a separate process catches packaging / import-path bugs the
in-memory tests miss.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import unittest
from pathlib import Path
from typing import Any, Dict


HOOK_SCRIPT = r"""
import sys
sys.path.insert(0, {src!r})
from ztl_ast import run

def transform(payload, ctx):
    if isinstance(payload, dict):
        payload = dict(payload)
        payload['touched_by'] = ctx.env.extension_id
    ctx.emit_template_vars({{'echoed_slug': ctx.page_slug}})
    return payload

if __name__ == '__main__':
    run(transform, hook_id='subprocess-echo', version='0.0.1')
"""


def _src_path() -> str:
    # ``tests/test_run_integration.py`` → ``tools/ztl-ast-py/src``
    here = Path(__file__).resolve().parent
    return str(here.parent / "src")


def _send(proc: subprocess.Popen, msg: Dict[str, Any]) -> None:
    proc.stdin.write(json.dumps(msg) + "\n")
    proc.stdin.flush()


def _recv(proc: subprocess.Popen) -> Dict[str, Any]:
    line = proc.stdout.readline()
    if not line:
        raise EOFError(f"hook closed stdout: stderr={proc.stderr.read() if proc.stderr else ''!r}")
    return json.loads(line)


class TestRunSubprocess(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = Path(os.environ.get("TMPDIR", "/tmp")) / "ztl-ast-py-run-it"
        self.tmp.mkdir(exist_ok=True)
        self.hook_path = self.tmp / "hook.py"
        self.hook_path.write_text(HOOK_SCRIPT.format(src=_src_path()))

    def tearDown(self) -> None:
        try:
            self.hook_path.unlink()
        except FileNotFoundError:
            pass

    def test_full_lifecycle(self) -> None:
        proc = subprocess.Popen(
            [sys.executable, str(self.hook_path)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
        )
        try:
            handshake = _recv(proc)
            self.assertEqual(handshake["ztl_ast"], 1)
            self.assertEqual(handshake["hook"], "subprocess-echo")
            self.assertIs(handshake["ready"], True)

            _send(
                proc,
                {
                    "type": "init",
                    "stage": "transform",
                    "ztl_version": "0.1.0",
                    "ast_schema_version": "1.0",
                    "ctx": {"vault_root": "/v"},
                },
            )
            init = _recv(proc)
            self.assertEqual(init, {"type": "result", "payload": None})

            _send(
                proc,
                {
                    "type": "run",
                    "page_slug": "pages/foo",
                    "frontmatter": {"t": "ok"},
                    "payload": {"body": "hi"},
                    "deadline_ms": 1000,
                },
            )
            run_msg = _recv(proc)
            self.assertEqual(run_msg["type"], "result")
            self.assertEqual(run_msg["payload"]["body"], "hi")
            self.assertEqual(run_msg["payload"]["touched_by"], "subprocess-echo")
            self.assertEqual(run_msg["template_vars"]["echoed_slug"], "pages/foo")

            _send(proc, {"type": "finalise"})
            finalise = _recv(proc)
            self.assertEqual(finalise, {"type": "result", "payload": None})

            _send(proc, {"type": "shutdown"})
            # No response expected for shutdown.
            rc = proc.wait(timeout=5)
            self.assertEqual(rc, 0)
        finally:
            if proc.poll() is None:
                proc.kill()
                proc.wait(timeout=5)
            for s in (proc.stdin, proc.stdout, proc.stderr):
                if s is not None:
                    s.close()


if __name__ == "__main__":
    unittest.main()
