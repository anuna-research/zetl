//! SPEC-032 TEST-3210 — `ztl-ast-js` helper library contract test.
//!
//! Spawns a minimal identity-transform hook written with
//! `tools/ztl-ast-js` against the Rust-side `PersistentHook`
//! driver and asserts round-trip equivalence for a small AST.
//!
//! Requires a pre-built `dist/esm/` (CI runs `npm run build` in
//! `tools/ztl-ast-js/` before this test). Skips if Node is not on
//! PATH or the build output is missing.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde_json::json;
use tempfile::TempDir;

use ztl::hooks::ast::{Document, DocumentKind, Inline, Paragraph, Position, Text, AST_VERSION};
use ztl::hooks::persistent::{HookMessage, PersistentHook};
use ztl::hooks::pipeline::Stage;

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

fn helper_dist_root() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.join("tools/ztl-ast-js/dist/esm")
}

fn write_hook_script(dir: &Path) -> PathBuf {
    let dist = helper_dist_root();
    let index = dist.join("index.js");
    let script = format!(
        r#"#!/usr/bin/env node
import {{ run }} from {index};
run((ast) => ast, {{ hookId: "identity-js", version: "test" }});
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

fn small_ast() -> Document {
    Document {
        ast_version: AST_VERSION.to_string(),
        kind: DocumentKind::Document,
        position: Position::origin(),
        frontmatter: None,
        children: vec![ztl::hooks::ast::Block::Paragraph(Paragraph {
            position: Position::origin(),
            children: vec![Inline::Text(Text {
                position: Position::origin(),
                text: "hello from helper-js".into(),
            })],
        })],
    }
}

#[test]
fn helper_js_identity_round_trip() {
    if !node_available() {
        eprintln!("skip: node not on PATH");
        return;
    }
    let dist = helper_dist_root();
    if !dist.join("index.js").exists() {
        eprintln!(
            "skip: helper-js dist is not built at {} (run `npm run build` in tools/ztl-ast-js)",
            dist.display()
        );
        return;
    }

    let tmp = TempDir::new().unwrap();
    let hook_path = write_hook_script(tmp.path());

    let mut cmd = Command::new(&hook_path);
    cmd.env_remove("NODE_OPTIONS");

    let mut hook = PersistentHook::spawn(cmd, "identity-js", Stage::Transform).unwrap();
    assert_eq!(hook.handshake().hook, "identity-js");
    assert_eq!(hook.handshake().ztl_ast, 1);

    let init = hook.init(json!({"theme": "default"}), 2_000).unwrap();
    matches!(init, HookMessage::Result { .. });

    let ast = small_ast();
    let payload = serde_json::to_value(&ast).unwrap();
    let resp = hook
        .run("demo", json!({"title": "Demo"}), payload.clone(), 2_000)
        .unwrap();
    match resp {
        HookMessage::Result {
            payload: returned, ..
        } => {
            let doc: Document = serde_json::from_value(returned).unwrap();
            assert_eq!(
                doc, ast,
                "helper-js identity must round-trip the AST bit-for-bit"
            );
        }
        other => panic!("unexpected run response: {other:?}"),
    }

    let _ = hook.finalise(2_000).unwrap();
    hook.shutdown().unwrap();

    // Give the shutdown grace a moment — Node's event loop needs to
    // observe stdin close and exit. Any residual stderr is surfaced
    // below for debuggability when the test flakes.
    std::thread::sleep(Duration::from_millis(50));
    let stderr = hook.peek_stderr();
    if !stderr.is_empty() {
        eprintln!("helper-js stderr: {stderr}");
    }
    assert!(hook.is_dead());
}
