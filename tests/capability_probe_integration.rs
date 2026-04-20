//! SPEC-032 REQ-3216 / TEST-3216 — capability probe integration tests.
//!
//! Exercises:
//! - end-to-end probe round-trip with a Python fixture hook;
//! - probe deadline enforcement — a hook that never responds returns
//!   [`ProbeOutcome::ProbeError`] within the configured deadline;
//! - probe_result with `ready: false` classifies as [`ProbeOutcome::Declined`],
//!   not an error;
//! - probe_result whose `stages` omit the composed stage is flagged as
//!   [`ProbeOutcome::StageMismatch`];
//! - `zetl hook capabilities` CLI subcommand prints a table / JSON
//!   against a fixture vault and exits non-zero on probe failures.
//!
//! Python3 is required; tests are skipped gracefully when it's absent.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use assert_cmd::cargo::cargo_bin_cmd;
use tempfile::TempDir;

use zetl::hooks::capability::{classify, probe_hook, ProbeOutcome, DEFAULT_PROBE_TIMEOUT};
use zetl::hooks::composition::{ComposedHook, CompositionSource};
use zetl::hooks::persistent::{AppliesWhen, PersistentHook, ProbeResult};
use zetl::hooks::pipeline::Stage;
use zetl::hooks::translators::AstType;

fn python3_available() -> bool {
    Command::new("python3")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn write_hook(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    let script = format!("#!/usr/bin/env python3\n{body}");
    std::fs::write(&path, script).unwrap();
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();
    path
}

/// Composed hook pointing at `exe_path`. Bypasses `compose_stage` so
/// tests stay self-contained — composition scanning is covered by
/// `composition`-level tests.
fn mk_composed(exe_path: &Path, extension_id: &str, stage: Stage) -> ComposedHook {
    ComposedHook {
        stage,
        filename: exe_path.file_name().unwrap().to_string_lossy().to_string(),
        extension_id: extension_id.into(),
        path: exe_path.to_path_buf(),
        manifest_path: None,
        source: CompositionSource::Vault,
        before: Vec::new(),
        after: Vec::new(),
        optional: false,
        ast_type: AstType::ZetlExt,
        ast_version: None,
        preserves: Vec::new(),
        ecosystem: None,
        disabled: None,
    }
}

/// Minimal hook that replies to the probe message. Declares stages
/// the caller passes in. Exits after probe + shutdown so the test
/// doesn't hang waiting for further requests.
fn probe_only_hook(stages: &[&str], ready: bool) -> String {
    let stages_json = serde_json::to_string(stages).unwrap();
    format!(
        r#"
import json, sys
sys.stdout.write('{{"zetl_ast":1,"hook":"p","version":"0.1.0","ready":true}}\n')
sys.stdout.flush()
for line in sys.stdin:
    try:
        msg = json.loads(line)
    except Exception:
        continue
    t = msg.get("type")
    if t == "shutdown":
        break
    if t == "probe":
        resp = {{
            "type": "probe_result",
            "zetl_ast": "1.0",
            "hook": "p",
            "version": "0.1.0",
            "stages": {stages_json},
            "ast_types": ["zetl-ext"],
            "ready": {ready_py},
        }}
        sys.stdout.write(json.dumps(resp) + "\n")
        sys.stdout.flush()
        continue
    sys.stdout.write('{{"type":"result","payload":null,"diagnostics":[],"template_vars":{{}}}}\n')
    sys.stdout.flush()
"#,
        stages_json = stages_json,
        ready_py = if ready { "True" } else { "False" },
    )
}

#[test]
fn probe_round_trip_returns_ok_outcome() {
    if !python3_available() {
        eprintln!("skip: python3 not available");
        return;
    }
    let tmp = TempDir::new().unwrap();
    let hook_path = write_hook(tmp.path(), "ok.py", &probe_only_hook(&["transform"], true));
    let hook = mk_composed(&hook_path, "ok", Stage::Transform);

    match probe_hook(&hook, Some("build"), DEFAULT_PROBE_TIMEOUT) {
        ProbeOutcome::Ok(r) => {
            assert_eq!(r.hook, "p");
            assert_eq!(r.version, "0.1.0");
            assert_eq!(r.zetl_ast, "1.0");
            assert_eq!(r.stages, vec!["transform"]);
            assert!(r.ready);
        }
        other => panic!("expected Ok, got {other:?}"),
    }
}

#[test]
fn probe_ready_false_is_declined_not_error() {
    if !python3_available() {
        eprintln!("skip: python3 not available");
        return;
    }
    let tmp = TempDir::new().unwrap();
    let hook_path = write_hook(
        tmp.path(),
        "declined.py",
        &probe_only_hook(&["transform"], false),
    );
    let hook = mk_composed(&hook_path, "declined", Stage::Transform);

    match probe_hook(&hook, Some("build"), DEFAULT_PROBE_TIMEOUT) {
        ProbeOutcome::Declined { .. } => {}
        other => panic!("expected Declined, got {other:?}"),
    }
}

#[test]
fn probe_stage_mismatch_flags_the_composed_stage() {
    if !python3_available() {
        eprintln!("skip: python3 not available");
        return;
    }
    let tmp = TempDir::new().unwrap();
    // Hook declares only `post-render` but is composed into `transform`.
    let hook_path = write_hook(
        tmp.path(),
        "mismatch.py",
        &probe_only_hook(&["post-render"], true),
    );
    let hook = mk_composed(&hook_path, "mismatch", Stage::Transform);

    match probe_hook(&hook, Some("build"), DEFAULT_PROBE_TIMEOUT) {
        ProbeOutcome::StageMismatch {
            expected, declared, ..
        } => {
            assert_eq!(expected, Stage::Transform);
            assert_eq!(declared, vec!["post-render"]);
        }
        other => panic!("expected StageMismatch, got {other:?}"),
    }
}

#[test]
fn probe_timeout_classifies_as_error() {
    if !python3_available() {
        eprintln!("skip: python3 not available");
        return;
    }
    let tmp = TempDir::new().unwrap();
    // Hook returns a handshake but ignores the probe message. The
    // probe call should time out; probe_hook returns ProbeError.
    let body = r#"
import json, sys, time
sys.stdout.write('{"zetl_ast":1,"hook":"slow","version":"0.1.0","ready":true}\n')
sys.stdout.flush()
# Never respond to anything — just sleep.
for line in sys.stdin:
    time.sleep(10)
"#;
    let hook_path = write_hook(tmp.path(), "slow.py", body);
    let hook = mk_composed(&hook_path, "slow", Stage::Transform);

    // 250 ms is long enough to not flake on CI startup but short enough
    // that the test run finishes promptly.
    match probe_hook(&hook, Some("build"), Duration::from_millis(250)) {
        ProbeOutcome::ProbeError(msg) => {
            assert!(
                msg.to_lowercase().contains("deadline") || msg.to_lowercase().contains("timeout"),
                "error message should mention deadline/timeout; got: {msg}"
            );
        }
        other => panic!("expected ProbeError, got {other:?}"),
    }
}

#[test]
fn probe_malformed_response_classifies_as_error() {
    if !python3_available() {
        eprintln!("skip: python3 not available");
        return;
    }
    let tmp = TempDir::new().unwrap();
    // Hook handshakes fine, then returns a plain-text line when probed.
    let body = r#"
import json, sys
sys.stdout.write('{"zetl_ast":1,"hook":"bad","version":"0.1.0","ready":true}\n')
sys.stdout.flush()
for line in sys.stdin:
    msg = json.loads(line)
    if msg.get("type") == "probe":
        sys.stdout.write("not json\n")
        sys.stdout.flush()
    elif msg.get("type") == "shutdown":
        break
"#;
    let hook_path = write_hook(tmp.path(), "bad.py", body);
    let hook = mk_composed(&hook_path, "bad", Stage::Transform);

    match probe_hook(&hook, Some("build"), DEFAULT_PROBE_TIMEOUT) {
        ProbeOutcome::ProbeError(_) => {}
        other => panic!("expected ProbeError, got {other:?}"),
    }
}

#[test]
fn probe_via_persistent_hook_api_direct() {
    if !python3_available() {
        eprintln!("skip: python3 not available");
        return;
    }
    let tmp = TempDir::new().unwrap();
    let hook_path = write_hook(
        tmp.path(),
        "direct.py",
        &probe_only_hook(&["transform", "post-render"], true),
    );

    let mut hook =
        PersistentHook::spawn(Command::new(&hook_path), "direct", Stage::Transform).unwrap();
    let result: ProbeResult = hook.probe(Some("build"), 2_000).unwrap();
    assert_eq!(result.stages, vec!["transform", "post-render"]);
    assert_eq!(result.zetl_ast, "1.0");
    assert!(result.ready);

    // After probe, the hook should still accept further messages.
    let _ = hook.shutdown();
}

#[test]
fn classify_unit_checks_applies_when_is_preserved() {
    // Unit-layer sanity — if a hook emits applies_when the classifier
    // must preserve it in the Ok(result) so CLI callers can render it.
    let r = ProbeResult {
        zetl_ast: "1.0".into(),
        hook: "x".into(),
        version: "0.1.0".into(),
        stages: vec!["transform".into()],
        ast_types: Some(vec!["zetl-ext".into()]),
        applies_when: Some(AppliesWhen {
            modes: Some(vec!["build".into()]),
            ..Default::default()
        }),
        ready: true,
        reason: None,
    };
    match classify(r, Stage::Transform) {
        ProbeOutcome::Ok(out) => {
            assert_eq!(
                out.applies_when.as_ref().unwrap().modes.as_ref().unwrap(),
                &vec!["build".to_string()]
            );
        }
        other => panic!("expected Ok, got {other:?}"),
    }
}

// ── CLI coverage ──────────────────────────────────────────────────────────

fn scaffold_vault_with_hook(vault: &Path, stage: &str, name: &str, hook_body: &str) {
    let hook_dir = vault.join(format!(".zetl/hooks/{stage}.d"));
    std::fs::create_dir_all(&hook_dir).unwrap();
    let exe_path = hook_dir.join(format!("{name}.py"));
    let script = format!("#!/usr/bin/env python3\n{hook_body}");
    std::fs::write(&exe_path, script).unwrap();
    let mut perms = std::fs::metadata(&exe_path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&exe_path, perms).unwrap();
}

#[test]
fn cli_hook_capabilities_reports_every_composed_hook() {
    if !python3_available() {
        eprintln!("skip: python3 not available");
        return;
    }
    let tmp = TempDir::new().unwrap();
    let vault = tmp.path();

    scaffold_vault_with_hook(
        vault,
        "transform",
        "alpha",
        &probe_only_hook(&["transform"], true),
    );

    let out = cargo_bin_cmd!("zetl")
        .args([
            "--dir",
            vault.to_str().unwrap(),
            "hook",
            "capabilities",
            "--stage",
            "transform",
        ])
        .output()
        .expect("run zetl hook capabilities");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "capabilities must exit 0 when every probe succeeds; stderr={stderr}"
    );
    assert!(
        stdout.contains("alpha"),
        "expected hook id in output; got: {stdout}"
    );
    assert!(
        stdout.contains("transform"),
        "expected stage name in output; got: {stdout}"
    );
    assert!(
        stdout.contains("ok"),
        "expected ok status in output; got: {stdout}"
    );
}

#[test]
fn cli_hook_capabilities_exits_nonzero_on_stage_mismatch() {
    if !python3_available() {
        eprintln!("skip: python3 not available");
        return;
    }
    let tmp = TempDir::new().unwrap();
    let vault = tmp.path();

    // Hook is composed into `transform` but declares only `post-render`.
    scaffold_vault_with_hook(
        vault,
        "transform",
        "mismatch",
        &probe_only_hook(&["post-render"], true),
    );

    let out = cargo_bin_cmd!("zetl")
        .args([
            "--dir",
            vault.to_str().unwrap(),
            "hook",
            "capabilities",
            "--stage",
            "transform",
        ])
        .output()
        .expect("run zetl hook capabilities");
    let code = out.status.code().unwrap_or(-1);
    assert_eq!(code, 1, "expected exit 1 on probe failure; got {code}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("stage-mismatch") || stdout.contains("mismatch"),
        "expected stage-mismatch marker in output; got: {stdout}"
    );
}

#[test]
fn cli_hook_capabilities_json_output_round_trips() {
    if !python3_available() {
        eprintln!("skip: python3 not available");
        return;
    }
    let tmp = TempDir::new().unwrap();
    let vault = tmp.path();

    scaffold_vault_with_hook(
        vault,
        "transform",
        "alpha",
        &probe_only_hook(&["transform"], true),
    );

    let out = cargo_bin_cmd!("zetl")
        .args([
            "--dir",
            vault.to_str().unwrap(),
            "hook",
            "capabilities",
            "--json",
            "--stage",
            "transform",
        ])
        .output()
        .expect("run zetl hook capabilities --json");
    assert!(
        out.status.success(),
        "json output must exit 0 when probes succeed; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value =
        serde_json::from_str(&stdout).expect("--json output must be valid JSON");
    let arr = v.as_array().expect("top-level must be array");
    assert_eq!(arr.len(), 1, "expected exactly one report; got {arr:?}");
    let first = &arr[0];
    assert_eq!(first["extension_id"], "alpha");
    assert_eq!(first["stage"], "transform");
    assert_eq!(first["outcome"]["status"], "ok");
    assert_eq!(first["outcome"]["result"]["stages"][0], "transform");
}
