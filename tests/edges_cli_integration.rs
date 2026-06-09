//! SPEC-045 REQ-4509 / CON-4503: `zetl edges` typed-graph query CLI.

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;
use std::fs;
use tempfile::TempDir;

/// A vault reproducing SPEC-045 HP2 (Decision Log) plus a bare link.
fn setup_vault() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("Decision Log.md"),
        "# Decision Log\n\n## Provenance\n\n\
         - derived_from::informed_by::[[2026 Q1 Retro]]\n\
         - supersedes::[[Decision Log 2025]]\n\
         - contradicts::[[Move Fast Doctrine]]\n\n\
         See also [[2026 Q1 Retro]].\n",
    )
    .unwrap();
    fs::write(dir.path().join("2026 Q1 Retro.md"), "# 2026 Q1 Retro\n").unwrap();
    dir
}

fn edges_json(dir: &TempDir, args: &[&str]) -> Vec<Value> {
    let mut full = vec!["-f", "json", "-d", dir.path().to_str().unwrap(), "edges"];
    full.extend_from_slice(args);
    let out = cargo_bin_cmd!("zetl").args(&full).assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    serde_json::from_str(&stdout).unwrap()
}

#[test]
fn no_filter_lists_typed_and_untyped() {
    let dir = setup_vault();
    let rows = edges_json(&dir, &[]);
    // 2 typed (derived_from + informed_by) + supersedes + contradicts + 1 untyped "see also".
    assert_eq!(rows.len(), 5);
    let untyped = rows.iter().filter(|r| r["predicate"].is_null()).count();
    assert_eq!(untyped, 1);
}

#[test]
fn predicate_filter_selects_one_kind() {
    let dir = setup_vault();
    let rows = edges_json(&dir, &["--predicate", "contradicts"]);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["predicate"], "contradicts");
    assert_eq!(rows[0]["target"], "Move Fast Doctrine");
    assert_eq!(rows[0]["is_ghost"], true);
}

#[test]
fn predicate_filter_is_repeatable_or() {
    let dir = setup_vault();
    let rows = edges_json(&dir, &["--predicate", "derived_from", "--predicate", "supersedes"]);
    assert_eq!(rows.len(), 2);
}

#[test]
fn untyped_filter_isolates_bare_links() {
    let dir = setup_vault();
    let rows = edges_json(&dir, &["--untyped"]);
    assert_eq!(rows.len(), 1);
    assert!(rows[0]["predicate"].is_null());
}

#[test]
fn by_predicate_histogram() {
    let dir = setup_vault();
    let rows = edges_json(&dir, &["--by-predicate"]);
    // contradicts:1, derived_from:1, informed_by:1, supersedes:1, (untyped):1
    let find = |p: &str| {
        rows.iter()
            .find(|r| r["predicate"] == p)
            .map(|r| r["count"].as_u64().unwrap())
    };
    assert_eq!(find("contradicts"), Some(1));
    assert_eq!(find("derived_from"), Some(1));
    assert_eq!(find("(untyped)"), Some(1));
}

#[test]
fn unknown_predicate_is_zero_rows_not_error() {
    let dir = setup_vault();
    let rows = edges_json(&dir, &["--predicate", "never_used"]);
    assert!(rows.is_empty());
}

#[test]
fn from_filter_scopes_to_source_page() {
    let dir = setup_vault();
    let rows = edges_json(&dir, &["--from", "decision log"]); // case-insensitive
    assert_eq!(rows.len(), 5);
    assert!(rows.iter().all(|r| r["source"] == "Decision Log"));
}
