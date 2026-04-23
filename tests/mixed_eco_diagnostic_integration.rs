//! SPEC-033 TEST-3315 — mixed-parser diagnostic.
//!
//! A page configured with `parser: pandoc` in its frontmatter matched
//! by an `ecosystem = "mdbook"` hook's selector MUST produce a
//! diagnostic that cites both parsers and names the resolution.
//!
//! Build-time: `ztl build` warns by default; `ztl build
//! --strict-parsers` refuses. Config-time: `ztl ecosystem check`
//! surfaces the same diagnostic so authors catch it pre-flight.

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

/// Build a vault that is the canonical TEST-3315 acceptance scenario:
///
/// - one page `papers/whitepaper.md` with frontmatter `parser: pandoc`;
/// - one `pre-parse` hook that declares `ecosystem = "mdbook"` and a
///   selector matching every markdown page in the vault.
///
/// The hook executable is a no-op stub marked executable so the
/// composition layer classifies it as enabled. It is never invoked
/// during the detection path (pure selector evaluation).
fn scaffold_mixed_vault(vault: &Path) {
    let hook_dir = vault.join(".ztl/hooks/pre-parse.d");
    fs::create_dir_all(&hook_dir).unwrap();

    write(
        &hook_dir.join("book-index.py"),
        "#!/usr/bin/env python3\nraise SystemExit('should not be invoked')\n",
    );
    let exe = hook_dir.join("book-index.py");
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(&exe).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&exe, perms).unwrap();

    // SPEC-033 CON-3312 — ecosystem field on the manifest is the
    // signal that makes a hook eligible for mixed-parser detection.
    write(
        &hook_dir.join("book-index.py.toml"),
        r#"stage = "pre-parse"
ecosystem = "mdbook"

[select]
include = ["**/*.md"]
"#,
    );

    write(
        &vault.join("papers/whitepaper.md"),
        "---\nparser: pandoc\ntitle: Whitepaper\n---\n\n# body\n",
    );
}

fn run_build(vault: &Path, args: &[&str]) -> std::process::Output {
    let mut cmd = cargo_bin_cmd!("ztl");
    cmd.args(["--dir", vault.to_str().unwrap(), "build"])
        .args(args)
        .env("RUST_BACKTRACE", "0");
    cmd.output().expect("run ztl build")
}

fn run_ecosystem_check(vault: &Path) -> std::process::Output {
    let mut cmd = cargo_bin_cmd!("ztl");
    cmd.args(["--dir", vault.to_str().unwrap(), "ecosystem", "check"])
        .env("RUST_BACKTRACE", "0");
    cmd.output().expect("run ztl ecosystem check")
}

// ── TEST-3315 acceptance ────────────────────────────────────────────

#[test]
fn build_warns_on_mixed_parser_and_cites_both() {
    let tmp = TempDir::new().unwrap();
    let vault = tmp.path();
    scaffold_mixed_vault(vault);

    let out = run_build(
        vault,
        &[
            "-o",
            tmp.path().join("out").to_str().unwrap(),
            "--theme",
            "default",
        ],
    );
    let stderr = String::from_utf8(out.stderr).unwrap();

    // Cite the page's parser.
    assert!(
        stderr.contains("pandoc"),
        "expected stderr to name the page parser 'pandoc'; got:\n{stderr}"
    );
    // Cite the hook's ecosystem + expected parser.
    assert!(
        stderr.contains("mdbook"),
        "expected stderr to name hook ecosystem 'mdbook'; got:\n{stderr}"
    );
    assert!(
        stderr.contains("commonmark"),
        "expected stderr to name expected parser 'commonmark'; got:\n{stderr}"
    );
    // Five-part shape: summary, context, observed, Likely cause, Hint.
    assert!(
        stderr.contains("[ztl] mixed-parser configuration"),
        "expected `[ztl] mixed-parser configuration` summary; got:\n{stderr}"
    );
    assert!(
        stderr.contains("Likely cause:"),
        "expected `Likely cause:` line; got:\n{stderr}"
    );
    assert!(
        stderr.contains("Hint:"),
        "expected `Hint:` line; got:\n{stderr}"
    );
    // Name the resolution: either pick one parser OR disable the hook.
    assert!(
        stderr.contains("parser: commonmark") || stderr.contains("strict-parsers"),
        "expected hint to name the resolution; got:\n{stderr}"
    );
    // Hook extension_id appears so authors can find the offending file.
    assert!(
        stderr.contains("book-index"),
        "expected hook id 'book-index' to appear; got:\n{stderr}"
    );
    // Page path appears so authors can find the offending page.
    assert!(
        stderr.contains("papers/whitepaper.md"),
        "expected page path to appear; got:\n{stderr}"
    );
}

#[test]
fn strict_parsers_escalates_warning_to_failure() {
    let tmp = TempDir::new().unwrap();
    let vault = tmp.path();
    scaffold_mixed_vault(vault);

    let out = run_build(
        vault,
        &[
            "-o",
            tmp.path().join("out").to_str().unwrap(),
            "--theme",
            "default",
            "--strict-parsers",
        ],
    );
    let stderr = String::from_utf8(out.stderr).unwrap();

    assert!(
        !out.status.success(),
        "expected --strict-parsers to fail the build; stderr={stderr}"
    );
    assert!(
        stderr.contains("mixed-parser"),
        "stderr should cite mixed-parser failure; got:\n{stderr}"
    );
}

#[test]
fn clean_vault_produces_no_mixed_parser_warning() {
    // Same ecosystem / same parser — no violation.
    let tmp = TempDir::new().unwrap();
    let vault = tmp.path();
    let hook_dir = vault.join(".ztl/hooks/pre-parse.d");
    fs::create_dir_all(&hook_dir).unwrap();

    write(
        &hook_dir.join("book-index.py"),
        "#!/usr/bin/env python3\nraise SystemExit('should not be invoked')\n",
    );
    let exe = hook_dir.join("book-index.py");
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(&exe).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&exe, perms).unwrap();

    // mdbook hook expects commonmark; page has no `parser:` → defaults
    // to commonmark. No violation.
    write(
        &hook_dir.join("book-index.py.toml"),
        r#"stage = "pre-parse"
ecosystem = "mdbook"

[select]
include = ["**/*.md"]
"#,
    );
    write(
        &vault.join("notes/hello.md"),
        "---\ntitle: Hello\n---\n\n# body\n",
    );

    let out = run_build(
        vault,
        &[
            "-o",
            tmp.path().join("out").to_str().unwrap(),
            "--theme",
            "default",
        ],
    );
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        !stderr.contains("mixed-parser configuration"),
        "expected no mixed-parser warning for same-parser vault; got:\n{stderr}"
    );
}

#[test]
fn ecosystem_check_surfaces_mixed_parser_config_time() {
    // Config-time check: `ztl ecosystem check` should also surface
    // the mixed-parser diagnostic so CI catches it pre-flight.
    let tmp = TempDir::new().unwrap();
    let vault = tmp.path();
    scaffold_mixed_vault(vault);

    let out = run_ecosystem_check(vault);
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("mixed-parser configuration"),
        "expected ecosystem check to surface mixed-parser; got:\n{stderr}"
    );
    assert!(
        stderr.contains("papers/whitepaper.md"),
        "expected page path; got:\n{stderr}"
    );
}
