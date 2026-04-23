//! SPEC-032 TEST-3209 — integration coverage for `ztl hook dry-run`.
//!
//! TEST-3209 requires:
//! - Running `ztl hook dry-run transform/callouts` against a fixture vault
//!   emits a byte-identical matched page list (sorted, one path per line).
//! - Exit code 1 when the selector matches zero pages; 0 otherwise.
//! - The hook itself is *not* invoked (no subprocess spawn, no writes).
//!
//! The tests use a self-contained temp vault so the golden stays stable
//! against changes to `demo-vault/`.

#![cfg(unix)]

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

/// Build a vault with a `transform/callouts` hook whose selector requires a
/// path under `posts/` AND a `:::note` content probe. Three pages land in
/// distinct selector cells:
///
/// | Page                    | Matches |
/// |-------------------------|---------|
/// | `posts/alpha.md`        | yes     |
/// | `posts/bravo.md`        | yes     |
/// | `posts/charlie.md`      | no (path ok, body has no `:::note`) |
/// | `notes/delta.md`        | no (path excluded) |
fn scaffold_vault(vault: &Path) {
    // Hook manifest + stub executable. The stub is NEVER invoked by
    // `dry-run`, but composition requires the file to exist and be
    // marked executable for the hook to count as "enabled".
    let hook_dir = vault.join(".ztl/hooks/transform.d");
    fs::create_dir_all(&hook_dir).unwrap();
    write(
        &hook_dir.join("callouts.py"),
        "#!/usr/bin/env python3\nraise SystemExit('dry-run must not invoke me')\n",
    );
    // Mark +x so composition classifies the hook as enabled.
    let exe_path = hook_dir.join("callouts.py");
    let mut perms = fs::metadata(&exe_path).unwrap().permissions();
    use std::os::unix::fs::PermissionsExt;
    perms.set_mode(0o755);
    fs::set_permissions(&exe_path, perms).unwrap();

    write(
        &hook_dir.join("callouts.py.toml"),
        r#"stage = "transform"

[select]
include = ["posts/**/*.md"]
content_probe = [":::note"]
"#,
    );

    // Pages.
    write(
        &vault.join("posts/alpha.md"),
        "---\ntitle: Alpha\n---\n\n:::note\nhello\n:::\n",
    );
    write(
        &vault.join("posts/bravo.md"),
        "---\ntitle: Bravo\n---\n\n:::note\nworld\n:::\n",
    );
    write(
        &vault.join("posts/charlie.md"),
        "---\ntitle: Charlie\n---\n\nno callout markers here\n",
    );
    write(
        &vault.join("notes/delta.md"),
        "---\ntitle: Delta\n---\n\n:::note\nwrong path\n:::\n",
    );
}

fn run_dry_run(vault: &Path, args: &[&str]) -> std::process::Output {
    cargo_bin_cmd!("ztl")
        .args(["--dir", vault.to_str().unwrap(), "hook", "dry-run"])
        .args(args)
        .output()
        .expect("run ztl hook dry-run")
}

#[test]
fn dry_run_emits_sorted_matched_pages_and_exits_zero() {
    let tmp = TempDir::new().unwrap();
    let vault = tmp.path();
    scaffold_vault(vault);

    let out = run_dry_run(vault, &["transform/callouts"]);
    let stdout = String::from_utf8(out.stdout).unwrap();
    let stderr = String::from_utf8(out.stderr).unwrap();

    assert!(
        out.status.success(),
        "expected exit 0 with matches; stderr={stderr}"
    );
    // Byte-stable sorted list — TEST-3209 golden.
    assert_eq!(stdout, "posts/alpha.md\nposts/bravo.md\n");
}

#[test]
fn dry_run_exits_one_when_selector_matches_nothing() {
    let tmp = TempDir::new().unwrap();
    let vault = tmp.path();
    scaffold_vault(vault);

    // Narrow the selector to a subdir that has no pages so the match set
    // is empty. Overwrite the sidecar manifest in place.
    let manifest = vault.join(".ztl/hooks/transform.d/callouts.py.toml");
    write(
        &manifest,
        r#"stage = "transform"

[select]
include = ["does-not-exist/**/*.md"]
"#,
    );

    let out = run_dry_run(vault, &["transform/callouts"]);
    let code = out.status.code().unwrap_or(-1);
    assert_eq!(
        code,
        1,
        "expected exit 1 on zero matches; stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stdout.is_empty(),
        "empty match set should produce empty stdout; got {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn dry_run_rejects_malformed_spec() {
    let tmp = TempDir::new().unwrap();
    let vault = tmp.path();
    scaffold_vault(vault);

    let out = run_dry_run(vault, &["bogus-no-slash"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("stage>/<name") || stderr.contains("transform/callouts"),
        "expected helpful error, got: {stderr}"
    );
}

#[test]
fn dry_run_rejects_unknown_stage() {
    let tmp = TempDir::new().unwrap();
    let vault = tmp.path();
    scaffold_vault(vault);

    let out = run_dry_run(vault, &["middle/callouts"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("unknown stage") || stderr.contains("pre-parse"),
        "expected unknown-stage error, got: {stderr}"
    );
}

#[test]
fn dry_run_rejects_missing_hook() {
    let tmp = TempDir::new().unwrap();
    let vault = tmp.path();
    scaffold_vault(vault);

    let out = run_dry_run(vault, &["transform/does-not-exist"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no hook 'does-not-exist'"),
        "expected missing-hook error, got: {stderr}"
    );
}

#[test]
fn dry_run_limit_truncates_output() {
    let tmp = TempDir::new().unwrap();
    let vault = tmp.path();
    scaffold_vault(vault);

    let out = run_dry_run(vault, &["transform/callouts", "--limit", "1"]);
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    // Only the first page (lex-sorted) survives the limit.
    assert_eq!(stdout, "posts/alpha.md\n");
}

#[test]
fn dry_run_matches_hook_with_numeric_prefix_via_default_extension_id() {
    // A hook file named `20-callouts.py` should still match the spec
    // `transform/callouts` because composition strips the ordering prefix
    // to derive the default extension id.
    let tmp = TempDir::new().unwrap();
    let vault = tmp.path();
    let hook_dir = vault.join(".ztl/hooks/transform.d");
    fs::create_dir_all(&hook_dir).unwrap();
    let exe = hook_dir.join("20-callouts.py");
    write(&exe, "#!/usr/bin/env python3\nraise SystemExit(0)\n");
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(&exe).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&exe, perms).unwrap();
    write(
        &hook_dir.join("20-callouts.py.toml"),
        "stage = \"transform\"\n[select]\ninclude = [\"**/*.md\"]\n",
    );
    write(&vault.join("alpha.md"), "body\n");

    let out = run_dry_run(vault, &["transform/callouts"]);
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "alpha.md\n");
}

#[test]
fn dry_run_does_not_invoke_hook() {
    // The stub exits with a non-zero status and writes a sentinel string;
    // if `dry-run` invoked it we'd see the sentinel on stderr or a
    // non-zero exit code. Neither happens.
    let tmp = TempDir::new().unwrap();
    let vault = tmp.path();
    scaffold_vault(vault);

    let out = run_dry_run(vault, &["transform/callouts"]);
    assert!(out.status.success(), "hook must not have been invoked");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !combined.contains("dry-run must not invoke me"),
        "hook was spawned; output: {combined}"
    );
}

#[test]
fn dry_run_frontmatter_predicate_gates_matches() {
    let tmp = TempDir::new().unwrap();
    let vault = tmp.path();

    let hook_dir = vault.join(".ztl/hooks/transform.d");
    fs::create_dir_all(&hook_dir).unwrap();
    let exe = hook_dir.join("callouts.py");
    write(&exe, "#!/usr/bin/env python3\n");
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(&exe).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&exe, perms).unwrap();
    write(
        &hook_dir.join("callouts.py.toml"),
        r#"stage = "transform"

[select]
frontmatter_where = 'status == "published"'
"#,
    );

    write(
        &vault.join("published.md"),
        "---\nstatus: published\n---\nbody\n",
    );
    write(&vault.join("draft.md"), "---\nstatus: draft\n---\nbody\n");

    let out = run_dry_run(vault, &["transform/callouts"]);
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "published.md\n");
}
