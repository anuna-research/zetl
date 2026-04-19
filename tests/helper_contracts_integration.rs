//! SPEC-032 REQ-3210 / CON-3210 — Cross-implementation helper contract test.
//!
//! Every first-party helper library (Rust core, `zetl-ast-py`,
//! `zetl-ast-js`) must agree **bit-for-bit** on what an identity
//! transform of a v1 AST document does: read the AST, return the AST,
//! unmodified. Any disagreement is a contract regression and this
//! gate fails the build with a side-by-side diff of the culprit pair.
//!
//! The corpus lives under `tests/fixtures/helper-contracts/*.json`.
//! Each file is a canonical v1 `Document`. For each fixture the
//! test:
//!
//! 1. Runs the fixture through a **Rust** identity round-trip
//!    (`serde_json::from_value::<Document>(_)` then back to `Value`).
//! 2. Spawns a **Python** identity hook via `PersistentHook` and
//!    drives it through init → run → finalise → shutdown.
//! 3. Spawns a **JavaScript** identity hook the same way.
//! 4. Asserts all three outputs equal the fixture (structural /
//!    value-level equality; field ordering is not a contract).
//!
//! Skips (with a visible `eprintln!`) if either runtime is missing.
//! CI installs both, so the skips only kick in on a bare developer
//! laptop. See the `helper-contracts` Make target for the expected
//! one-liner invocation.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde_json::{json, Value};
use tempfile::TempDir;

use zetl::hooks::ast::Document;
use zetl::hooks::persistent::{HookMessage, PersistentHook, ProtocolError};
use zetl::hooks::pipeline::Stage;

// ── Runtime discovery ──────────────────────────────────────────────────────

fn python3_available() -> bool {
    Command::new("python3")
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn node_available() -> bool {
    Command::new("node")
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn manifest_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn helper_js_dist() -> PathBuf {
    manifest_dir().join("tools/zetl-ast-js/dist/esm/index.js")
}

fn helper_py_src() -> PathBuf {
    manifest_dir().join("tools/zetl-ast-py/src")
}

fn fixtures_dir() -> PathBuf {
    manifest_dir().join("tests/fixtures/helper-contracts")
}

// ── Hook script scaffolding ────────────────────────────────────────────────

fn write_py_identity_hook(dir: &Path) -> PathBuf {
    let src = helper_py_src();
    // JSON-encode the path literal so any oddities (quotes, backslashes,
    // non-ASCII) survive into the generated script — the helper-js
    // integration uses the same trick.
    let src_literal = serde_json::Value::String(src.display().to_string()).to_string();
    let script = [
        "#!/usr/bin/env python3",
        "import sys",
        &format!("sys.path.insert(0, {src_literal})"),
        "from zetl_ast import run",
        "",
        "def transform(ast, ctx):",
        "    return ast",
        "",
        "if __name__ == '__main__':",
        "    run(transform, hook_id='identity-py', version='contract-test')",
        "",
    ]
    .join("\n");
    let path = dir.join("identity.py");
    std::fs::write(&path, script).unwrap();
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();
    path
}

fn write_js_identity_hook(dir: &Path) -> PathBuf {
    let index = helper_js_dist();
    let script = format!(
        r#"#!/usr/bin/env node
import {{ run }} from {index};
run((ast) => ast, {{ hookId: "identity-js", version: "contract-test" }});
"#,
        index = serde_json::Value::String(index.display().to_string()),
    );
    let path = dir.join("identity.mjs");
    std::fs::write(&path, script).unwrap();
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();
    path
}

// ── Corpus enumeration ─────────────────────────────────────────────────────

/// Discover the fixture corpus sorted by filename so the diagnostic
/// output is deterministic.
fn load_corpus() -> Vec<(String, Value)> {
    let dir = fixtures_dir();
    let mut out: Vec<(String, Value)> = Vec::new();
    for entry in std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read fixtures dir {}: {e}", dir.display()))
    {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let raw = std::fs::read_to_string(&path).unwrap();
        let value: Value = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("fixture {} is not valid JSON: {e}", path.display()));
        out.push((
            path.file_name().unwrap().to_string_lossy().into_owned(),
            value,
        ));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    assert!(!out.is_empty(), "empty corpus under {}", dir.display());
    out
}

// ── Harness drivers ────────────────────────────────────────────────────────

/// Run one fixture through a persistent-mode hook and return its
/// `run` response payload. Panics on protocol failure — each helper is
/// in-process CI infrastructure, not something we expect to fail
/// mid-test.
fn drive_identity(hook_bin: &Path, hook_id: &'static str, payload: Value) -> Value {
    let mut cmd = Command::new(hook_bin);
    // NODE_OPTIONS can pull in loaders that break a helper bootstrap
    // when the host has custom Node tooling; the rest of the inherited
    // env is fine since REQ-3220 fields come from the init payload
    // rather than process env.
    cmd.env_remove("NODE_OPTIONS");

    let mut hook = PersistentHook::spawn(cmd, hook_id, Stage::Transform).unwrap_or_else(|e| {
        // One-shot diagnostic run so we can surface the real stderr
        // when the helper fails to handshake. PersistentHook::spawn
        // has already reaped its own child by now.
        let mut diag = Command::new(hook_bin);
        diag.env_remove("NODE_OPTIONS");
        let out = diag.output();
        let stderr = match out {
            Ok(o) => String::from_utf8_lossy(&o.stderr).into_owned(),
            Err(e) => format!("<diag run failed: {e}>"),
        };
        panic!("{hook_id}: spawn failed: {e}\ncommand: {hook_bin:?}\nstderr tail:\n{stderr}")
    });
    if let Err(e) = hook.init(json!({"theme": "contract-test"}), 5_000) {
        panic!(
            "{hook_id}: init failed: {e}\nstderr:\n{}",
            hook.peek_stderr()
        );
    }

    let resp = hook
        .run("fixture", json!({}), payload, 5_000)
        .unwrap_or_else(|e: ProtocolError| {
            panic!(
                "{hook_id}: run failed: {e}\nstderr:\n{}",
                hook.peek_stderr()
            )
        });
    let out = match resp {
        HookMessage::Result { payload, .. } => payload,
        HookMessage::Error { reason, detail } => {
            panic!("{hook_id}: hook returned error: {reason} ({detail})")
        }
    };

    // Finalise + shutdown so no orphans leak; errors here would be a
    // test-infra bug worth surfacing.
    let _ = hook.finalise(2_000);
    let _ = hook.shutdown();
    std::thread::sleep(Duration::from_millis(25));
    let stderr = hook.peek_stderr();
    if !stderr.is_empty() {
        eprintln!("[{hook_id}] stderr tail:\n{stderr}");
    }
    out
}

/// Rust-side identity round-trip: a schema-typed deserialise /
/// serialise pair. Anything the typed struct drops — `deny_unknown_fields`,
/// non-round-trippable enum discriminants — shows up here before the
/// process-level hooks get a chance to mask it.
fn rust_identity(fixture: &Value) -> Value {
    let doc: Document = serde_json::from_value(fixture.clone())
        .unwrap_or_else(|e| panic!("fixture is not a schema-valid Document: {e}"));
    serde_json::to_value(&doc).expect("Document re-serialises")
}

// ── Diff rendering ─────────────────────────────────────────────────────────

/// Render a side-by-side diff of two canonically pretty-printed JSON
/// values. Keys are stringified via `to_string_pretty` — `serde_json`
/// sorts object keys deterministically only when the `preserve_order`
/// feature is off, which matches this crate's configuration.
fn side_by_side_diff(a_label: &str, a: &Value, b_label: &str, b: &Value) -> String {
    let a_text = serde_json::to_string_pretty(a).unwrap();
    let b_text = serde_json::to_string_pretty(b).unwrap();
    let a_lines: Vec<&str> = a_text.lines().collect();
    let b_lines: Vec<&str> = b_text.lines().collect();
    let width = a_lines
        .iter()
        .map(|l| l.len())
        .max()
        .unwrap_or(0)
        .max(a_label.len())
        + 2;
    let trail = b_label.len() + 2;
    let mut out = String::new();
    out.push_str(&format!("{a_label:<width$}| {b_label}\n"));
    out.push_str(&format!("{:<width$}+{:-<trail$}\n", "", ""));
    let max_lines = a_lines.len().max(b_lines.len());
    for i in 0..max_lines {
        let la = a_lines.get(i).copied().unwrap_or("");
        let lb = b_lines.get(i).copied().unwrap_or("");
        let marker = if la == lb { " " } else { "!" };
        out.push_str(&format!("{la:<width$}{marker} {lb}\n"));
    }
    out
}

// ── The gate ───────────────────────────────────────────────────────────────

#[test]
fn helper_contracts_identity_round_trip() {
    let fixtures = load_corpus();
    eprintln!(
        "[helper-contracts] corpus: {} fixtures under {}",
        fixtures.len(),
        fixtures_dir().display()
    );

    // Availability gates — each helper is skipped individually so a
    // partial-runtime box still exercises whatever is installed. The
    // Rust harness always runs.
    let have_python = python3_available();
    let have_node = node_available() && helper_js_dist().exists();
    if !have_python {
        eprintln!("[helper-contracts] skip py harness: python3 not on PATH");
    }
    if !node_available() {
        eprintln!("[helper-contracts] skip js harness: node not on PATH");
    } else if !helper_js_dist().exists() {
        eprintln!(
            "[helper-contracts] skip js harness: {} is missing (run `make helper-js-build`)",
            helper_js_dist().display()
        );
    }

    let tmp = TempDir::new().unwrap();
    let py_hook = have_python.then(|| write_py_identity_hook(tmp.path()));
    let js_hook = have_node.then(|| write_js_identity_hook(tmp.path()));

    let mut failures: Vec<String> = Vec::new();
    for (name, fixture) in &fixtures {
        let rust_out = rust_identity(fixture);
        if &rust_out != fixture {
            failures.push(format!(
                "[rust] fixture '{name}' does not round-trip through typed Document:\n{}",
                side_by_side_diff("fixture", fixture, "rust_out", &rust_out)
            ));
            // No point running the helpers if the Rust round-trip
            // itself fails — the fixture is the bug, not the helpers.
            continue;
        }

        if let Some(py) = &py_hook {
            let py_out = drive_identity(py, "identity-py", fixture.clone());
            if &py_out != fixture {
                failures.push(format!(
                    "[py] fixture '{name}' differs after zetl-ast-py identity:\n{}",
                    side_by_side_diff("fixture", fixture, "py_out", &py_out)
                ));
            }
        }

        if let Some(js) = &js_hook {
            let js_out = drive_identity(js, "identity-js", fixture.clone());
            if &js_out != fixture {
                failures.push(format!(
                    "[js] fixture '{name}' differs after zetl-ast-js identity:\n{}",
                    side_by_side_diff("fixture", fixture, "js_out", &js_out)
                ));
            }
        }
    }

    if !failures.is_empty() {
        panic!(
            "helper-contracts disagreement ({} failure{}):\n\n{}",
            failures.len(),
            if failures.len() == 1 { "" } else { "s" },
            failures.join("\n────────────────────────────────────────────────\n")
        );
    }
}

// ── Diff rendering self-check ──────────────────────────────────────────────

/// Guards against the diff formatter silently producing empty output on
/// mismatch — the side-by-side view is the one thing the CI logs carry
/// when a contract breaks, so an assertion that it renders differing
/// lines with the `!` marker keeps the diagnostic honest.
#[test]
fn side_by_side_diff_marks_differing_lines() {
    let a = json!({"k": 1, "arr": [1, 2]});
    let b = json!({"k": 1, "arr": [1, 3]});
    let rendered = side_by_side_diff("left", &a, "right", &b);
    assert!(
        rendered.contains('!'),
        "expected diff marker in output: {rendered}"
    );
    assert!(
        rendered.contains("left"),
        "expected left label in output: {rendered}"
    );
    assert!(
        rendered.contains("right"),
        "expected right label in output: {rendered}"
    );
}
