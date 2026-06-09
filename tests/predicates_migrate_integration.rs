//! SPEC-045 REQ-4513: read-only `zetl predicates migrate --dry-run`.

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;
use std::fs;
use tempfile::TempDir;

fn vault() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("Note.md"),
        "---\ntags:\n  - deep context architecture\n  - unrelated-tag\n---\n# Note\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("Deep Context Architecture.md"),
        "# Deep Context Architecture\n",
    )
    .unwrap();
    dir
}

#[test]
fn dry_run_reports_tags_that_name_pages() {
    let dir = vault();
    let out = cargo_bin_cmd!("zetl")
        .args([
            "-f",
            "json",
            "-d",
            dir.path().to_str().unwrap(),
            "predicates",
            "migrate",
            "--dry-run",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let rows: Vec<Value> = serde_json::from_str(&stdout).unwrap();
    // Only the tag that names a real page is reported.
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["tag"], "deep context architecture");
    assert_eq!(rows[0]["candidate_target"], "Deep Context Architecture");
}

#[test]
fn refuses_without_dry_run_and_modifies_nothing() {
    let dir = vault();
    let before = fs::read_to_string(dir.path().join("Note.md")).unwrap();
    cargo_bin_cmd!("zetl")
        .args(["-d", dir.path().to_str().unwrap(), "predicates", "migrate"])
        .assert()
        .failure();
    // The source file is untouched.
    let after = fs::read_to_string(dir.path().join("Note.md")).unwrap();
    assert_eq!(before, after);
}
