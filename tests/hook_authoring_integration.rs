//! SPEC-032 TEST-3225 — integration matrix for `zetl hook new / test /
//! fixture / watch`.
//!
//! The matrix (per SPEC-032 §TEST-3225) exercises each subcommand on a
//! temp vault and checks the listed assertions. `watch` gets its own
//! timing-gated test in the Rust unit-test layer (via the `watch`
//! function's injected `WatchOptions::max_events`) so the CLI test
//! suite doesn't need a real file-system race.

#![cfg(unix)]

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// True iff `python3` is on PATH. The scaffolded hooks use Python for
/// persistent-mode JSON handling; skip the integration matrix when it
/// isn't available (CI must install it; local devs on bare macOS may
/// lack it).
fn python3_available() -> bool {
    std::process::Command::new("python3")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

macro_rules! require_python {
    () => {
        if !python3_available() {
            eprintln!("[hook_authoring] skipping: python3 not available");
            return;
        }
    };
}

fn touch_vault(dir: &TempDir) -> &Path {
    // An empty vault root is enough — all scaffolding writes relative
    // to `<vault>/.zetl/hooks/...` and `<vault>/tests/hook-fixtures/...`,
    // both of which the scaffolder creates on demand.
    dir.path()
}

fn run_zetl(vault: &Path, args: &[&str]) -> std::process::Output {
    cargo_bin_cmd!("zetl")
        .args(["--dir", vault.to_str().unwrap()])
        .args(args)
        .output()
        .expect("run zetl")
}

// ── Matrix row 1: `zetl hook new transform foo --lang py` ────────────────

#[test]
fn hook_new_scaffolds_at_expected_paths() {
    let tmp = TempDir::new().unwrap();
    let vault = touch_vault(&tmp);

    let out = run_zetl(vault, &["hook", "new", "transform", "foo", "--lang", "py"]);
    assert!(
        out.status.success(),
        "hook new failed: {}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let hook_path = vault.join(".zetl/hooks/transform.d/foo.py");
    // Composition reads `<filename-with-ext>.toml`; scaffold matches that.
    let manifest_path = vault.join(".zetl/hooks/transform.d/foo.py.toml");
    let input_path = vault.join("tests/hook-fixtures/foo/input.md");
    let expected_path = vault.join("tests/hook-fixtures/foo/expected.json");
    assert!(hook_path.is_file(), "{}", hook_path.display());
    assert!(manifest_path.is_file(), "{}", manifest_path.display());
    assert!(input_path.is_file(), "{}", input_path.display());
    assert!(expected_path.is_file(), "{}", expected_path.display());

    // Executable bit set (chmod 0755) so the persistent protocol can
    // invoke the file directly.
    use std::os::unix::fs::PermissionsExt;
    let mode = fs::metadata(&hook_path).unwrap().permissions().mode();
    assert_eq!(mode & 0o111, 0o111, "scaffold must set +x: mode={mode:o}");

    // Manifest parses as valid TOML + has the expected stage field.
    let manifest_text = fs::read_to_string(&manifest_path).unwrap();
    assert!(
        manifest_text.contains(r#"stage = "transform""#),
        "{manifest_text}"
    );
    assert!(manifest_text.contains(r#"extension_id = "foo""#));
}

#[test]
fn hook_new_then_hook_test_passes_on_fresh_scaffold() {
    require_python!();
    let tmp = TempDir::new().unwrap();
    let vault = touch_vault(&tmp);

    let out = run_zetl(vault, &["hook", "new", "transform", "bar", "--lang", "py"]);
    assert!(out.status.success());

    let out = run_zetl(vault, &["hook", "test", "bar"]);
    assert!(
        out.status.success(),
        "fresh-scaffold hook test must pass immediately:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("ok"),
        "expected match message, got: {stdout}"
    );
}

// ── Matrix row 2: `zetl hook test <existing>` no-op ──────────────────────

#[test]
fn hook_test_noop_exits_zero() {
    require_python!();
    let tmp = TempDir::new().unwrap();
    let vault = touch_vault(&tmp);

    let out = run_zetl(vault, &["hook", "new", "transform", "noop", "--lang", "py"]);
    assert!(out.status.success());

    // First test run.
    let out = run_zetl(vault, &["hook", "test", "noop"]);
    assert!(out.status.success());

    // Second run — still a no-op.
    let out = run_zetl(vault, &["hook", "test", "noop"]);
    assert!(out.status.success());
}

// ── Matrix row 3: `zetl hook test <existing>` after edit ─────────────────

#[test]
fn hook_test_after_edit_exits_non_zero_with_diff() {
    require_python!();
    let tmp = TempDir::new().unwrap();
    let vault = touch_vault(&tmp);

    // Use pre-parse stage: scaffold's golden is plain markdown, easiest to
    // mutate by editing the hook to prepend a banner line.
    let out = run_zetl(
        vault,
        &["hook", "new", "pre-parse", "edit_me", "--lang", "py"],
    );
    assert!(out.status.success());

    // Overwrite the hook with a non-identity version: prepend "MUTATED\n"
    // to every payload.
    let hook_path = vault.join(".zetl/hooks/pre-parse.d/edit_me.py");
    let mutated = r#"#!/usr/bin/env python3
import json, sys
sys.stdout.write(json.dumps({"zetl_ast":1,"hook":"edit_me","version":"0.1.0","ready":True}) + "\n")
sys.stdout.flush()
for line in sys.stdin:
    try:
        msg = json.loads(line)
    except Exception:
        continue
    if msg.get("type") == "shutdown":
        break
    payload = msg.get("payload", "") or ""
    payload = "MUTATED\n" + payload
    sys.stdout.write(json.dumps({"type":"result","payload":payload,"diagnostics":[],"template_vars":{}}) + "\n")
    sys.stdout.flush()
"#;
    fs::write(&hook_path, mutated).unwrap();
    // Preserve +x (fs::write resets mode on some platforms? not on unix,
    // but belt-and-braces).
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&hook_path, fs::Permissions::from_mode(0o755)).unwrap();

    let out = run_zetl(vault, &["hook", "test", "edit_me"]);
    assert!(!out.status.success(), "must exit non-zero after hook edit");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("FAIL"),
        "expected FAIL message, got: {stderr}"
    );
    // Diff output mentions the mutation.
    assert!(
        stderr.contains("MUTATED") || stderr.contains("+") && stderr.contains("-"),
        "expected diff markers / mutated line, got:\n{stderr}"
    );
}

// ── Matrix row 4: `zetl hook test --update` ──────────────────────────────

#[test]
fn hook_test_update_regenerates_golden_and_exits_zero() {
    require_python!();
    let tmp = TempDir::new().unwrap();
    let vault = touch_vault(&tmp);

    let out = run_zetl(vault, &["hook", "new", "pre-parse", "gold", "--lang", "py"]);
    assert!(out.status.success());

    // Mutate the hook as in row 3.
    let hook_path = vault.join(".zetl/hooks/pre-parse.d/gold.py");
    let mutated = r#"#!/usr/bin/env python3
import json, sys
sys.stdout.write(json.dumps({"zetl_ast":1,"hook":"gold","version":"0.1.0","ready":True}) + "\n")
sys.stdout.flush()
for line in sys.stdin:
    try:
        msg = json.loads(line)
    except Exception:
        continue
    if msg.get("type") == "shutdown":
        break
    payload = msg.get("payload", "") or ""
    payload = "REGEN\n" + payload
    sys.stdout.write(json.dumps({"type":"result","payload":payload,"diagnostics":[],"template_vars":{}}) + "\n")
    sys.stdout.flush()
"#;
    fs::write(&hook_path, mutated).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&hook_path, fs::Permissions::from_mode(0o755)).unwrap();

    // Without --update this should fail; with --update it should pass
    // AND the new golden should contain the mutation string.
    let out = run_zetl(vault, &["hook", "test", "gold"]);
    assert!(
        !out.status.success(),
        "diverged hook must fail without --update"
    );

    let out = run_zetl(vault, &["hook", "test", "gold", "--update"]);
    assert!(
        out.status.success(),
        "--update must exit zero: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let golden = fs::read_to_string(vault.join("tests/hook-fixtures/gold/expected.md")).unwrap();
    assert!(
        golden.starts_with("REGEN"),
        "new golden must reflect mutation: {golden:?}"
    );

    // Subsequent plain `hook test` now passes against the regenerated
    // golden.
    let out = run_zetl(vault, &["hook", "test", "gold"]);
    assert!(
        out.status.success(),
        "after --update, plain `hook test` must pass: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// ── Matrix row 5: `zetl hook fixture --from <page>` ──────────────────────

#[test]
fn hook_fixture_captures_vault_page_as_input() {
    require_python!();
    let tmp = TempDir::new().unwrap();
    let vault = touch_vault(&tmp);

    // Scaffold the hook first.
    let out = run_zetl(vault, &["hook", "new", "pre-parse", "cap", "--lang", "py"]);
    assert!(out.status.success());

    // Drop a real vault page at `projects/q2.md`.
    fs::create_dir_all(vault.join("projects")).unwrap();
    let page_body = "# Q2 Plan\n\nReview quarterly goals.\n";
    fs::write(vault.join("projects/q2.md"), page_body).unwrap();

    let out = run_zetl(
        vault,
        &[
            "hook",
            "fixture",
            "--from",
            "projects/q2.md",
            "--hook",
            "cap",
        ],
    );
    assert!(
        out.status.success(),
        "hook fixture failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let input = fs::read_to_string(vault.join("tests/hook-fixtures/cap/input.md")).unwrap();
    assert_eq!(
        input, page_body,
        "captured input.md must match the vault page content verbatim"
    );

    // Golden re-seeded from identity hook; `hook test` should pass.
    let out = run_zetl(vault, &["hook", "test", "cap"]);
    assert!(
        out.status.success(),
        "captured fixture must round-trip: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// ── Sanity: scaffold works for all three stages and all three langs ──────

#[test]
fn scaffold_matrix_all_stages_accepted() {
    let tmp = TempDir::new().unwrap();
    let vault = touch_vault(&tmp);
    for (stage, ext) in [
        ("pre-parse", "md"),
        ("transform", "json"),
        ("post-render", "html"),
    ] {
        let name = format!("h_{}", stage.replace('-', "_"));
        let out = run_zetl(vault, &["hook", "new", stage, &name, "--lang", "py"]);
        assert!(
            out.status.success(),
            "scaffold failed for stage={stage}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            vault
                .join(format!("tests/hook-fixtures/{name}/expected.{ext}"))
                .is_file(),
            "stage {stage} expects expected.{ext}"
        );
    }
}

#[test]
fn scaffold_refuses_overwrite_without_force() {
    let tmp = TempDir::new().unwrap();
    let vault = touch_vault(&tmp);
    let out = run_zetl(vault, &["hook", "new", "transform", "dup", "--lang", "py"]);
    assert!(out.status.success());
    let out = run_zetl(vault, &["hook", "new", "transform", "dup", "--lang", "py"]);
    assert!(
        !out.status.success(),
        "second scaffold without --force must fail"
    );
    let out = run_zetl(
        vault,
        &["hook", "new", "transform", "dup", "--lang", "py", "--force"],
    );
    assert!(out.status.success(), "scaffold --force must succeed");
}

// ── Matrix row 6: `zetl hook watch` restart ──────────────────────────────
//
// `zetl hook watch` blocks on a file watcher and Ctrl-C. Driving the
// full CLI here would require juggling child processes + signals in a
// way that's platform-brittle. Instead we test the watch loop *library*
// function directly — same code path the CLI wires, minus the blocking
// outer shell — with the `WatchOptions::max_events` knob. TEST-3225's
// "editing hook source triggers one restart within 500 ms" assertion
// maps cleanly to: after touching the file, exactly one `Restart`
// event lands in the channel within 500 ms.

#[test]
fn watch_loop_restarts_on_source_edit_within_500ms() {
    require_python!();
    let tmp = TempDir::new().unwrap();
    let vault = touch_vault(&tmp);

    // Scaffold an identity hook.
    let scaffold = zetl::hooks::authoring::scaffold(
        vault,
        &zetl::cli::AuthoringStage::Transform,
        "watched",
        &zetl::cli::HookLang::Py,
        None,
        false,
    )
    .unwrap();

    // Fork the watch loop into a background thread so we can edit the
    // file from the main thread and still observe events.
    use std::sync::mpsc;
    let (tx, rx) = mpsc::channel::<zetl::hooks::authoring::WatchEvent>();
    let vault_owned = vault.to_path_buf();
    let handle = std::thread::spawn(move || {
        let opts = zetl::hooks::authoring::WatchOptions {
            debounce: std::time::Duration::from_millis(50),
            max_events: Some(1), // stop after one restart
        };
        zetl::hooks::authoring::watch(&vault_owned, "watched", opts, |e| match e {
            zetl::hooks::authoring::WatchEvent::Spawned => {
                let _ = tx.send(zetl::hooks::authoring::WatchEvent::Spawned);
            }
            zetl::hooks::authoring::WatchEvent::Restart => {
                let _ = tx.send(zetl::hooks::authoring::WatchEvent::Restart);
            }
            _ => {}
        })
        .unwrap();
    });

    // Wait for initial spawn event.
    let initial = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("initial spawn event");
    assert!(
        matches!(initial, zetl::hooks::authoring::WatchEvent::Spawned),
        "first event must be Spawned"
    );

    // Edit the hook — touching mtime alone should be enough but to be
    // safe we rewrite the body with a trivial change.
    let body = fs::read_to_string(&scaffold.hook_file).unwrap();
    let t0 = std::time::Instant::now();
    // notify fires on Modify events; some platforms debounce very
    // aggressively, so we retry the write a few times if the first event
    // doesn't land within the budget.
    fs::write(&scaffold.hook_file, format!("{body}\n# touched\n")).unwrap();

    // Expect Restart within 500 ms — TEST-3225 assertion.
    let event = rx.recv_timeout(std::time::Duration::from_millis(500));
    let elapsed = t0.elapsed();
    assert!(
        matches!(event, Ok(zetl::hooks::authoring::WatchEvent::Restart)),
        "expected one Restart within 500 ms, got {event:?} after {elapsed:?}"
    );

    // Let the loop wind down.
    let _ = handle.join();
}

// ── Regression: --ecosystem pandoc scaffold composes without hand-edits ──

#[test]
fn ecosystem_pandoc_scaffold_writes_composition_canonical_manifest_and_lua_filter() {
    // Two pre-existing bugs surfaced by smoke-testing the IMPL-032-033 PR:
    //   1. scaffolder wrote `<name>.toml`, but composition reads
    //      `<filename-with-ext>.toml` — manifest was silently invisible.
    //   2. `--ecosystem pandoc` produced a manifest missing the required
    //      `exec`/`lua_filter` field, failing the per-ecosystem parser
    //      with the cryptic "must declare exec or lua_filter".
    // This test pins the fix end-to-end: scaffold a fresh pandoc hook,
    // assert the manifest sits at the composition-canonical path, and
    // load it through the ecosystem manifest parser.
    let tmp = TempDir::new().unwrap();
    let vault = touch_vault(&tmp);

    let out = run_zetl(
        vault,
        &[
            "hook",
            "new",
            "transform",
            "smallcaps",
            "--lang",
            "py",
            "--ecosystem",
            "pandoc",
        ],
    );
    assert!(
        out.status.success(),
        "scaffold failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    // Composition-canonical manifest path (was `smallcaps.toml` pre-fix).
    let manifest = vault.join(".zetl/hooks/transform.d/smallcaps.py.toml");
    assert!(
        manifest.is_file(),
        "manifest must use composition-canonical naming: {}",
        manifest.display()
    );
    // Identity Lua filter is on disk so `lua_filter = "..."` resolves.
    let lua = vault.join(".zetl/hooks/transform.d/filters/smallcaps.lua");
    assert!(
        lua.is_file(),
        "lua filter must be seeded: {}",
        lua.display()
    );

    // Manifest parses cleanly through the full base+ecosystem pipeline
    // without hand-edits — the pre-fix scaffold failed here with
    // "must declare exec or lua_filter".
    let toml_text = fs::read_to_string(&manifest).unwrap();
    let parsed = zetl::hooks::manifest::parse_manifest(&toml_text, Some(&manifest))
        .expect("scaffolded pandoc manifest must parse end-to-end");
    let eco = parsed
        .extra
        .as_ref()
        .expect("ecosystem block must be attached");
    assert_eq!(eco.id(), "pandoc", "ecosystem tag survives parsing");
}
