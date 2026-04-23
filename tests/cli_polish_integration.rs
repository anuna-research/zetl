use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

/// Helper: create a minimal vault with two pages
fn setup_vault() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("Hello.md"),
        "# Hello\n\nA link to [[World]].\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("World.md"),
        "# World\n\nBack to [[Hello]].\n",
    )
    .unwrap();
    dir
}

#[test]
fn test_json_flag_forces_json_output() {
    let dir = setup_vault();
    cargo_bin_cmd!("ztl")
        .args(["--json", "-d", dir.path().to_str().unwrap(), "list"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("{"));
}

#[test]
fn test_format_flag_json_produces_json() {
    let dir = setup_vault();
    cargo_bin_cmd!("ztl")
        .args(["-f", "json", "-d", dir.path().to_str().unwrap(), "list"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("{"));
}

#[test]
fn test_format_flag_table_produces_table() {
    let dir = setup_vault();
    cargo_bin_cmd!("ztl")
        .args(["-f", "table", "-d", dir.path().to_str().unwrap(), "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Hello"))
        .stdout(predicate::str::contains("World"));
}

#[test]
fn test_piped_output_defaults_to_json() {
    // When stdout is not a TTY (as in test processes), auto should resolve to JSON
    let dir = setup_vault();
    cargo_bin_cmd!("ztl")
        .args(["-d", dir.path().to_str().unwrap(), "list"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("{"));
}

#[test]
fn test_env_ztl_dir() {
    let dir = setup_vault();
    cargo_bin_cmd!("ztl")
        .env("ztl_DIR", dir.path().to_str().unwrap())
        .args(["--json", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Hello"));
}

#[test]
fn test_env_ztl_format_json() {
    let dir = setup_vault();
    cargo_bin_cmd!("ztl")
        .env("ztl_FORMAT", "json")
        .args(["-d", dir.path().to_str().unwrap(), "list"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("{"));
}

#[test]
fn test_env_ztl_format_table() {
    let dir = setup_vault();
    cargo_bin_cmd!("ztl")
        .env("ztl_FORMAT", "table")
        .args(["-d", dir.path().to_str().unwrap(), "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Hello"));
}

#[test]
fn test_flag_overrides_env_var() {
    let dir = setup_vault();
    // Flag -f json should override ztl_FORMAT=table
    cargo_bin_cmd!("ztl")
        .env("ztl_FORMAT", "table")
        .args(["-f", "json", "-d", dir.path().to_str().unwrap(), "list"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("{"));
}

#[test]
fn test_help_shows_examples() {
    cargo_bin_cmd!("ztl")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Examples:"))
        .stdout(predicate::str::contains("ztl list"));
}

#[test]
fn test_help_no_spec_references() {
    cargo_bin_cmd!("ztl")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("SPEC-").not())
        .stdout(predicate::str::contains("REQ-").not());
}

#[test]
fn test_subcommand_help_no_spec_references() {
    // Check a few subcommands that previously had spec refs
    for subcmd in &["watch", "diff", "view"] {
        cargo_bin_cmd!("ztl")
            .args([subcmd, "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("SPEC-").not())
            .stdout(predicate::str::contains("REQ-").not());
    }
}

#[test]
fn test_page_not_found_shows_hint() {
    let dir = setup_vault();
    cargo_bin_cmd!("ztl")
        .args([
            "-f",
            "table",
            "-d",
            dir.path().to_str().unwrap(),
            "links",
            "NonexistentPage",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("ztl list"));
}

#[test]
fn test_version_flag() {
    cargo_bin_cmd!("ztl")
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("ztl"));
}
