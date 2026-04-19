//! SPEC-032 TEST-3208 — integration coverage for `zetl hook coverage`.
//!
//! Build-mode persistence (`.zetl/build/hook-coverage.json`) is not yet
//! wired into `zetl build`, so TEST-3208 focuses on the dry-run fallback
//! path guaranteed by REQ-3208: "for the most-recent build (or a fresh
//! dry-run if none exists)". The tests cover:
//!
//! - Basic dry-run: per-hook `matched/total`, unmatched-hooks, and
//!   unmatched-pages sections are populated.
//! - `--json` emits the CON-3208 structurally-equivalent JSON object.
//! - `--stage` filters the report to one stage (ignores the other two).
//! - A persisted `hook-coverage.json` on disk flips the `source` field
//!   to `"build"` and overrides the invoked/failed/latency cells.

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

/// Build a vault with three hooks across two stages:
///
/// - `transform/callouts` — matches `posts/**/*.md` with a `:::note` probe.
/// - `transform/orphan`   — matches `never-matches/**` so it stays unmatched.
/// - `pre-parse/prelude`  — matches every `**/*.md` file.
///
/// Five pages total, with different selector cells:
///
/// | Page                     | callouts | orphan | prelude |
/// |--------------------------|----------|--------|---------|
/// | `posts/alpha.md`         | yes      | no     | yes     |
/// | `posts/bravo.md`         | yes      | no     | yes     |
/// | `posts/charlie.md`       | no       | no     | yes     |
/// | `notes/delta.md`         | no       | no     | yes     |
/// | `drafts/lonely.md`       | no       | no     | yes     |
fn scaffold_vault(vault: &Path) {
    scaffold_hook(
        vault,
        "transform",
        "callouts",
        r#"stage = "transform"

[select]
include = ["posts/**/*.md"]
content_probe = [":::note"]
"#,
    );
    scaffold_hook(
        vault,
        "transform",
        "orphan",
        r#"stage = "transform"

[select]
include = ["never-matches/**/*.md"]
"#,
    );
    scaffold_hook(
        vault,
        "pre-parse",
        "prelude",
        r#"stage = "pre-parse"

[select]
include = ["**/*.md"]
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
        "---\ntitle: Delta\n---\n\njust a note\n",
    );
    write(
        &vault.join("drafts/lonely.md"),
        "---\ntitle: Lonely\n---\n\nnothing matches me in transform\n",
    );
}

fn scaffold_hook(vault: &Path, stage: &str, name: &str, manifest: &str) {
    use std::os::unix::fs::PermissionsExt;
    let hook_dir = vault.join(".zetl/hooks").join(format!("{stage}.d"));
    fs::create_dir_all(&hook_dir).unwrap();
    let exe = hook_dir.join(format!("{name}.py"));
    write(
        &exe,
        "#!/usr/bin/env python3\nraise SystemExit('hook coverage must not invoke me')\n",
    );
    let mut perms = fs::metadata(&exe).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&exe, perms).unwrap();
    write(&hook_dir.join(format!("{name}.py.toml")), manifest);
}

fn run_coverage(vault: &Path, args: &[&str]) -> std::process::Output {
    cargo_bin_cmd!("zetl")
        .args(["--dir", vault.to_str().unwrap(), "hook", "coverage"])
        .args(args)
        .output()
        .expect("run zetl hook coverage")
}

/// Same as [`run_coverage`], but forces the table format — `assert_cmd`
/// pipes stdout, which auto-detects as `--json` otherwise.
fn run_coverage_table(vault: &Path, args: &[&str]) -> std::process::Output {
    cargo_bin_cmd!("zetl")
        .args([
            "--dir",
            vault.to_str().unwrap(),
            "-f",
            "table",
            "hook",
            "coverage",
        ])
        .args(args)
        .output()
        .expect("run zetl hook coverage")
}

#[test]
fn coverage_table_lists_every_hook_with_matched_ratio() {
    let tmp = TempDir::new().unwrap();
    let vault = tmp.path();
    scaffold_vault(vault);

    let out = run_coverage_table(vault, &[]);
    let stdout = String::from_utf8(out.stdout).unwrap();
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(out.status.success(), "stderr={stderr}");

    // Source header + total pages reflects the five scaffolded pages.
    assert!(stdout.contains("Source: dry-run"), "stdout={stdout}");
    assert!(stdout.contains("Total pages: 5"), "stdout={stdout}");

    // Every hook id shows up (CON-3208 row per hook).
    assert!(stdout.contains("callouts"), "stdout={stdout}");
    assert!(stdout.contains("orphan"), "stdout={stdout}");
    assert!(stdout.contains("prelude"), "stdout={stdout}");

    // CON-3208 `N/total` MATCHED cell.
    assert!(
        stdout.contains("2/5") && stdout.contains("5/5") && stdout.contains("0/5"),
        "expected per-hook matched/total cells; stdout={stdout}"
    );
}

#[test]
fn coverage_json_structure_matches_con_3208() {
    let tmp = TempDir::new().unwrap();
    let vault = tmp.path();
    scaffold_vault(vault);

    let out = run_coverage(vault, &["--json"]);
    assert!(out.status.success(), "stderr={}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout is JSON");

    assert_eq!(v["source"], "dry-run");
    assert_eq!(v["total_pages"], 5);

    let hooks = v["hooks"].as_array().expect("hooks is array");
    assert_eq!(hooks.len(), 3);

    // Every entry has the CON-3208 column shape (array of structurally-
    // equivalent objects).
    for h in hooks {
        for field in [
            "stage",
            "id",
            "manifest_path",
            "matched",
            "matched_of",
            "invoked",
            "failed",
            "p50_ms",
            "p95_ms",
            "last_failure_reason",
        ] {
            assert!(
                h.get(field).is_some(),
                "hook entry missing `{field}`: {h}"
            );
        }
        assert_eq!(h["matched_of"], 5);
    }

    // Dry-run synthesises `invoked == matched` with no latency/failure data.
    let callouts = hooks
        .iter()
        .find(|h| h["id"] == "callouts")
        .expect("callouts row present");
    assert_eq!(callouts["matched"], 2);
    assert_eq!(callouts["invoked"], 2);
    assert_eq!(callouts["failed"], 0);
    assert_eq!(callouts["stage"], "transform");

    // Unmatched hooks = hooks with zero matches.
    let unmatched_hooks = v["unmatched_hooks"].as_array().unwrap();
    assert_eq!(unmatched_hooks.len(), 1);
    assert_eq!(unmatched_hooks[0]["id"], "orphan");
    assert_eq!(unmatched_hooks[0]["stage"], "transform");

    // Unmatched pages = pages no enabled hook matched. `prelude` matches
    // every page, so this list should be empty — even when transform-stage
    // hooks leave pages untouched.
    let unmatched_pages = v["unmatched_pages"].as_array().unwrap();
    assert!(
        unmatched_pages.is_empty(),
        "prelude matches everything → no unmatched pages; got {unmatched_pages:?}"
    );
}

#[test]
fn coverage_stage_filter_restricts_output_to_one_stage() {
    let tmp = TempDir::new().unwrap();
    let vault = tmp.path();
    scaffold_vault(vault);

    let out = run_coverage(vault, &["--stage", "transform", "--json"]);
    assert!(out.status.success(), "stderr={}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();

    let hooks = v["hooks"].as_array().unwrap();
    assert_eq!(hooks.len(), 2, "transform stage has two hooks");
    for h in hooks {
        assert_eq!(h["stage"], "transform");
    }

    // With only transform-stage hooks selected, pages not in `posts/`
    // become unmatched — prelude is filtered out.
    let unmatched_pages = v["unmatched_pages"].as_array().unwrap();
    let names: Vec<&str> = unmatched_pages.iter().filter_map(|p| p.as_str()).collect();
    assert!(names.contains(&"posts/charlie.md"), "{names:?}");
    assert!(names.contains(&"notes/delta.md"), "{names:?}");
    assert!(names.contains(&"drafts/lonely.md"), "{names:?}");
}

#[test]
fn coverage_flags_unmatched_hooks_in_table_section() {
    let tmp = TempDir::new().unwrap();
    let vault = tmp.path();
    scaffold_vault(vault);

    let out = run_coverage_table(vault, &[]);
    let stdout = String::from_utf8(out.stdout).unwrap();

    assert!(
        stdout.contains("Unmatched hooks"),
        "expected unmatched-hooks section header; stdout={stdout}"
    );
    assert!(
        stdout.contains("orphan"),
        "unmatched orphan hook should be listed; stdout={stdout}"
    );
}

#[test]
fn coverage_lists_unmatched_pages_when_no_hook_matches() {
    let tmp = TempDir::new().unwrap();
    let vault = tmp.path();
    // Only scaffold the transform/callouts hook — everything else falls
    // into "unmatched pages".
    scaffold_hook(
        vault,
        "transform",
        "callouts",
        r#"stage = "transform"

[select]
include = ["posts/**/*.md"]
content_probe = [":::note"]
"#,
    );
    write(
        &vault.join("posts/alpha.md"),
        "---\ntitle: Alpha\n---\n\n:::note\nhi\n:::\n",
    );
    write(
        &vault.join("notes/delta.md"),
        "---\ntitle: Delta\n---\n\nunmatched\n",
    );

    let out = run_coverage(vault, &["--json"]);
    assert!(out.status.success(), "stderr={}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let unmatched = v["unmatched_pages"].as_array().unwrap();
    let names: Vec<&str> = unmatched.iter().filter_map(|p| p.as_str()).collect();
    assert_eq!(names, vec!["notes/delta.md"]);
}

#[test]
fn coverage_handles_vault_with_no_hooks() {
    let tmp = TempDir::new().unwrap();
    let vault = tmp.path();
    write(&vault.join("solo.md"), "---\ntitle: Solo\n---\n\n");

    let out = run_coverage(vault, &["--json"]);
    assert!(out.status.success(), "stderr={}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["total_pages"], 1);
    assert!(v["hooks"].as_array().unwrap().is_empty());
    assert!(v["unmatched_hooks"].as_array().unwrap().is_empty());
    // No hooks ⇒ every page is unmatched.
    let unmatched = v["unmatched_pages"].as_array().unwrap();
    assert_eq!(unmatched.len(), 1);
}

#[test]
fn coverage_reads_persisted_build_coverage_when_present() {
    let tmp = TempDir::new().unwrap();
    let vault = tmp.path();
    scaffold_vault(vault);

    // Future `zetl build` wires up persistence (§REQ-3208 "Persistence
    // semantics"). Simulate it here with a pre-populated file so we can
    // exercise the read path without a live build pipeline.
    write(
        &vault.join(".zetl/build/hook-coverage.json"),
        r#"{
  "hooks": [
    {
      "stage": "transform",
      "id": "callouts",
      "invoked": 2,
      "failed": 1,
      "p50_ms": 3,
      "p95_ms": 12,
      "last_failure_reason": "timeout"
    }
  ]
}
"#,
    );

    let out = run_coverage(vault, &["--json"]);
    assert!(out.status.success(), "stderr={}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();

    // Source flips to "build" once a persisted file is on disk.
    assert_eq!(v["source"], "build");

    let hooks = v["hooks"].as_array().unwrap();
    let callouts = hooks.iter().find(|h| h["id"] == "callouts").unwrap();
    assert_eq!(callouts["invoked"], 2);
    assert_eq!(callouts["failed"], 1);
    assert_eq!(callouts["p50_ms"], 3);
    assert_eq!(callouts["p95_ms"], 12);
    assert_eq!(callouts["last_failure_reason"], "timeout");

    // Hooks not in the persisted file fall back to dry-run defaults
    // (invoked == matched, failed == 0, no latency).
    let orphan = hooks.iter().find(|h| h["id"] == "orphan").unwrap();
    assert_eq!(orphan["invoked"], 0);
    assert_eq!(orphan["failed"], 0);
    assert!(orphan["p50_ms"].is_null());
}
