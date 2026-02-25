//! Integration tests for TEST-050: VCS Independence (NFR-017).
//!
//! These tests verify that all zetl commands work identically whether or not
//! a `.git` directory is present. The core requirement (NFR-017) is that
//! Merkle tree construction, cache invalidation, drift detection, and grounding
//! resolution produce identical results regardless of VCS presence.
//!
//! Three scenarios are covered:
//!
//! 1. **All commands work without Git** — vault in a plain directory (no `.git`);
//!    all commands succeed and git fields are null.
//!
//! 2. **Git metadata is optional enrichment** — vault inside a Git repo; `zetl
//!    reason provenance` shows git fields; removing `.git` nulls them while
//!    keeping provenance output otherwise identical. *(Skipped if `git` binary
//!    is not available.)*
//!
//! 3. **Cache validity is VCS-independent** — index inside a Git repo; removing
//!    `.git` and re-indexing succeeds with cache still valid. *(Skipped if `git`
//!    binary is not available.)*

use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a `Command` for `zetl` pointing at `vault`, bypassing the cache.
fn zetl_cmd(vault: &Path) -> Command {
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("zetl");
    cmd.arg("-d").arg(vault.as_os_str());
    cmd.arg("--no-cache");
    cmd
}

/// Build a `Command` for `zetl` pointing at `vault`, WITH caching enabled.
fn zetl_cmd_cached(vault: &Path) -> Command {
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("zetl");
    cmd.arg("-d").arg(vault.as_os_str());
    cmd
}

/// Create a file relative to `root`, creating parent directories as needed.
fn write_file(root: &Path, relative: &str, content: &str) {
    let full = root.join(relative);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).expect("create parent dirs");
    }
    fs::write(&full, content).expect("write test file");
}

/// Run the command, assert exit-zero, and parse stdout as JSON.
fn run_json(cmd: &mut Command) -> Value {
    let output = cmd.output().expect("failed to execute zetl");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "zetl exited non-zero.\nstdout: {stdout}\nstderr: {stderr}",
    );
    serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("failed to parse JSON: {e}\nstdout: {stdout}"))
}

/// Run a bare `git` command in `dir`, returning whether it succeeded.
fn git_run(dir: &Path, args: &[&str]) -> bool {
    std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Return `true` if a `git` binary is available on PATH.
fn git_available() -> bool {
    std::process::Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Vault builder
// ---------------------------------------------------------------------------

/// Build a minimal vault with wikilinks and an SPL block.
///
/// Produces exactly one defeasible conclusion (`flies`) from two source pages,
/// so that `zetl reason provenance flies` can be tested end-to-end.
fn build_vcs_vault(root: &Path) {
    write_file(
        root,
        "Bird Facts.md",
        "\
# Bird Facts

See also [[Penguin Facts]].

```spl
(given bird)
(normally r-bird-flies
  bird
  flies)
```
",
    );

    write_file(
        root,
        "Penguin Facts.md",
        "\
# Penguin Facts

See [[Bird Facts]].

```spl
(given penguin)
(always r-penguin-is-bird
  penguin
  bird)
```
",
    );
}

// ===========================================================================
// Scenario 1: All commands work without Git (TEST-050 §1)
// ===========================================================================

/// TEST-050-1a: `zetl index` succeeds when the vault has no `.git` directory.
#[test]
fn test_050_1a_index_no_git() {
    let dir = TempDir::new().expect("create temp dir");
    build_vcs_vault(dir.path());

    let json = run_json(zetl_cmd(dir.path()).arg("index"));

    // Output conforms to the IndexResult schema; no git fields present.
    assert!(
        json["files_scanned"].is_number(),
        "files_scanned must be present"
    );
    assert!(
        json["links_found"].is_number(),
        "links_found must be present"
    );
    assert!(json["dead_links"].is_number(), "dead_links must be present");
    assert!(
        json.get("git_commit").is_none(),
        "index output must not contain git_commit (§1.6 schema rule)"
    );
    assert!(
        json.get("git_dirty").is_none(),
        "index output must not contain git_dirty (§1.6 schema rule)"
    );
}

/// TEST-050-1b: `zetl check` succeeds when the vault has no `.git` directory.
#[test]
fn test_050_1b_check_no_git() {
    let dir = TempDir::new().expect("create temp dir");
    build_vcs_vault(dir.path());

    let json = run_json(zetl_cmd(dir.path()).arg("check"));

    assert!(json["dead_links"].is_array(), "dead_links must be present");
    assert!(json["orphans"].is_array(), "orphans must be present");
    assert!(
        json.get("git_commit").is_none(),
        "check output must not contain git_commit"
    );
    assert!(
        json.get("git_dirty").is_none(),
        "check output must not contain git_dirty"
    );
}

/// TEST-050-1c: `zetl stats` succeeds when the vault has no `.git` directory.
#[test]
fn test_050_1c_stats_no_git() {
    let dir = TempDir::new().expect("create temp dir");
    build_vcs_vault(dir.path());

    let json = run_json(zetl_cmd(dir.path()).arg("stats"));

    assert!(json["pages"].is_number(), "pages must be present");
    assert!(json["spl_blocks"].is_number(), "spl_blocks must be present");
    assert!(
        json.get("git_commit").is_none(),
        "stats output must not contain git_commit (§1.6 schema rule)"
    );
    assert!(
        json.get("git_dirty").is_none(),
        "stats output must not contain git_dirty (§1.6 schema rule)"
    );
}

/// TEST-050-1d: `zetl reason status` succeeds when the vault has no `.git` directory.
#[test]
fn test_050_1d_reason_status_no_git() {
    let dir = TempDir::new().expect("create temp dir");
    build_vcs_vault(dir.path());

    let json = run_json(zetl_cmd(dir.path()).arg("reason").arg("status"));

    assert!(json["theory"].is_object(), "theory summary must be present");
    assert!(
        json["conclusions"].is_array(),
        "conclusions must be present"
    );
    // reason status output must not expose git fields
    assert!(
        json.get("git_commit").is_none(),
        "reason status must not contain git_commit"
    );
    assert!(
        json.get("git_dirty").is_none(),
        "reason status must not contain git_dirty"
    );
}

/// TEST-050-1e: `zetl reason provenance` succeeds in a plain dir and reports
///              git fields as JSON null (§1.6).
#[test]
fn test_050_1e_reason_provenance_git_null_no_git() {
    let dir = TempDir::new().expect("create temp dir");
    build_vcs_vault(dir.path());

    let json = run_json(
        zetl_cmd(dir.path())
            .arg("reason")
            .arg("provenance")
            .arg("flies"),
    );

    assert_eq!(json["literal"], "flies", "provenance literal must match");
    // git fields must exist as JSON keys and be null outside a git repo
    assert_eq!(
        json["git_commit"],
        Value::Null,
        "git_commit must be null outside a git repo"
    );
    assert_eq!(
        json["git_dirty"],
        Value::Null,
        "git_dirty must be null outside a git repo"
    );
}

// ===========================================================================
// Scenario 2: Git metadata is optional enrichment (TEST-050 §2)
// ===========================================================================

/// TEST-050-2: `zetl reason provenance` includes git fields inside a Git repo
///             and they become null after `.git` is removed, while all other
///             provenance output remains identical.
///
/// Skipped when the `git` binary is not available.
#[test]
fn test_050_2_git_metadata_optional_enrichment() {
    if !git_available() {
        eprintln!("SKIP: git binary not available");
        return;
    }

    let dir = TempDir::new().expect("create temp dir");
    build_vcs_vault(dir.path());

    // Initialise a git repo and make an initial commit.
    assert!(git_run(dir.path(), &["init"]), "git init failed");
    // Silence "dubious ownership" warnings in some CI environments
    let _ = git_run(dir.path(), &["config", "safe.directory", "*"]);
    assert!(git_run(dir.path(), &["add", "."]), "git add failed");
    assert!(
        git_run(dir.path(), &["commit", "--message", "init"]),
        "git commit failed"
    );

    // Run provenance inside the git repo.
    let json_with_git = run_json(
        zetl_cmd(dir.path())
            .arg("reason")
            .arg("provenance")
            .arg("flies"),
    );

    // git_commit should be a non-null 40-char hex string.
    let commit = json_with_git["git_commit"]
        .as_str()
        .expect("git_commit must be a string inside a git repo");
    assert_eq!(
        commit.len(),
        40,
        "git_commit must be a 40-char SHA-1 hex string, got: {commit}"
    );
    assert!(
        commit.chars().all(|c| c.is_ascii_hexdigit()),
        "git_commit must be hex, got: {commit}"
    );

    // git_dirty should be a boolean (false — clean working tree after commit).
    assert!(
        json_with_git["git_dirty"].is_boolean(),
        "git_dirty must be a boolean inside a git repo"
    );
    assert_eq!(
        json_with_git["git_dirty"],
        Value::Bool(false),
        "git_dirty must be false after a clean commit"
    );

    // Remove .git to simulate a non-VCS environment.
    fs::remove_dir_all(dir.path().join(".git")).expect("remove .git");

    // Re-run provenance (force fresh build with --no-cache).
    let json_without_git = run_json(
        zetl_cmd(dir.path())
            .arg("reason")
            .arg("provenance")
            .arg("flies"),
    );

    // Git fields must now be null.
    assert_eq!(
        json_without_git["git_commit"],
        Value::Null,
        "git_commit must be null after .git is removed"
    );
    assert_eq!(
        json_without_git["git_dirty"],
        Value::Null,
        "git_dirty must be null after .git is removed"
    );

    // All other provenance fields must be identical (VCS-independence).
    assert_eq!(
        json_with_git["literal"], json_without_git["literal"],
        "literal must be identical with and without git"
    );
    assert_eq!(
        json_with_git["conclusions"], json_without_git["conclusions"],
        "conclusions must be identical with and without git"
    );
    assert_eq!(
        json_with_git["source_pages"], json_without_git["source_pages"],
        "source_pages must be identical with and without git"
    );
}

/// TEST-050-2b: `zetl reason provenance` marks `git_dirty: true` when there
///              are uncommitted changes.
///
/// Skipped when the `git` binary is not available.
#[test]
fn test_050_2b_git_dirty_true_with_uncommitted_changes() {
    if !git_available() {
        eprintln!("SKIP: git binary not available");
        return;
    }

    let dir = TempDir::new().expect("create temp dir");
    build_vcs_vault(dir.path());

    assert!(git_run(dir.path(), &["init"]), "git init failed");
    let _ = git_run(dir.path(), &["config", "safe.directory", "*"]);
    assert!(git_run(dir.path(), &["add", "."]), "git add failed");
    assert!(
        git_run(dir.path(), &["commit", "--message", "init"]),
        "git commit failed"
    );

    // Add an uncommitted file to make the working tree dirty.
    write_file(dir.path(), "UncommittedNote.md", "# Uncommitted\n");

    let json = run_json(
        zetl_cmd(dir.path())
            .arg("reason")
            .arg("provenance")
            .arg("flies"),
    );

    assert!(
        json["git_dirty"].is_boolean(),
        "git_dirty must be a boolean"
    );
    assert_eq!(
        json["git_dirty"],
        Value::Bool(true),
        "git_dirty must be true when there are uncommitted changes"
    );
}

// ===========================================================================
// Scenario 3: Cache validity is VCS-independent (TEST-050 §3)
// ===========================================================================

/// TEST-050-3a: The index cache remains valid after `.git` is removed.
///
/// Indexes a vault inside a git repo, removes `.git`, and re-runs `zetl index`.
/// The second run must succeed (cache is content/mtime-based, not VCS-based).
///
/// Skipped when the `git` binary is not available.
#[test]
fn test_050_3a_index_cache_valid_after_git_removal() {
    if !git_available() {
        eprintln!("SKIP: git binary not available");
        return;
    }

    let dir = TempDir::new().expect("create temp dir");
    build_vcs_vault(dir.path());

    assert!(git_run(dir.path(), &["init"]), "git init failed");
    let _ = git_run(dir.path(), &["config", "safe.directory", "*"]);
    assert!(git_run(dir.path(), &["add", "."]), "git add failed");
    assert!(
        git_run(dir.path(), &["commit", "--message", "init"]),
        "git commit failed"
    );

    // First index: WITH cache (populates .zetl/index.json).
    let json1 = run_json(zetl_cmd_cached(dir.path()).arg("index"));
    let files1 = json1["files_scanned"]
        .as_u64()
        .expect("files_scanned numeric");

    // Remove .git — content files are unchanged.
    fs::remove_dir_all(dir.path().join(".git")).expect("remove .git");

    // Second index: WITH cache; files are unchanged so cache should still be
    // valid (mtime + content hash match).
    let json2 = run_json(zetl_cmd_cached(dir.path()).arg("index"));
    let files2 = json2["files_scanned"]
        .as_u64()
        .expect("files_scanned numeric");

    // Same file count — no phantom re-indexing due to VCS removal.
    assert_eq!(
        files1, files2,
        "file count must be identical with and without .git"
    );

    // The .zetl/index.json file must exist (cache was created or reused).
    assert!(
        dir.path().join(".zetl").join("index.json").exists(),
        ".zetl/index.json must exist after indexed run"
    );
}

/// TEST-050-3b: The theory cache remains valid after `.git` is removed.
///
/// Builds the theory cache inside a git repo, then removes `.git` and reruns
/// `zetl reason status`. The command must succeed and produce the same
/// conclusions (cache is VCS-independent). git_commit/git_dirty becoming null
/// in provenance output is tested in scenario 2.
///
/// Skipped when the `git` binary is not available.
#[test]
fn test_050_3b_theory_cache_valid_after_git_removal() {
    if !git_available() {
        eprintln!("SKIP: git binary not available");
        return;
    }

    let dir = TempDir::new().expect("create temp dir");
    build_vcs_vault(dir.path());

    assert!(git_run(dir.path(), &["init"]), "git init failed");
    let _ = git_run(dir.path(), &["config", "safe.directory", "*"]);
    assert!(git_run(dir.path(), &["add", "."]), "git add failed");
    assert!(
        git_run(dir.path(), &["commit", "--message", "init"]),
        "git commit failed"
    );

    // First run: build theory cache inside the git repo.
    let mut cmd1 = zetl_cmd_cached(dir.path());
    cmd1.arg("reason").arg("status");
    let json1 = run_json(&mut cmd1);
    let total1 = json1["summary"]["total"].as_u64().expect("total numeric");

    // Remove .git — SPL content is unchanged.
    fs::remove_dir_all(dir.path().join(".git")).expect("remove .git");

    // Second run: WITH cache, without .git; theory cache must still be valid.
    let mut cmd2 = zetl_cmd_cached(dir.path());
    cmd2.arg("reason").arg("status");
    let json2 = run_json(&mut cmd2);
    let total2 = json2["summary"]["total"].as_u64().expect("total numeric");

    assert_eq!(
        total1, total2,
        "conclusion count must be identical with and without .git (VCS-independent cache)"
    );

    // The .zetl/theory.json must exist (theory cache was created or reused).
    assert!(
        dir.path().join(".zetl").join("theory.json").exists(),
        ".zetl/theory.json must exist after theory build"
    );
}

/// TEST-050-3c: After removing `.git`, `zetl reason provenance` shows
///              `git_commit: null` and `git_dirty: null` even when the theory
///              cache was originally built inside a git repo.
///
/// Skipped when the `git` binary is not available.
#[test]
fn test_050_3c_provenance_git_fields_null_after_git_removal() {
    if !git_available() {
        eprintln!("SKIP: git binary not available");
        return;
    }

    let dir = TempDir::new().expect("create temp dir");
    build_vcs_vault(dir.path());

    assert!(git_run(dir.path(), &["init"]), "git init failed");
    let _ = git_run(dir.path(), &["config", "safe.directory", "*"]);
    assert!(git_run(dir.path(), &["add", "."]), "git add failed");
    assert!(
        git_run(dir.path(), &["commit", "--message", "init"]),
        "git commit failed"
    );

    // First run: build theory cache (WITH cache, so it's stored to disk).
    let _json1 = run_json(
        zetl_cmd_cached(dir.path())
            .arg("reason")
            .arg("provenance")
            .arg("flies"),
    );

    // Remove .git.
    fs::remove_dir_all(dir.path().join(".git")).expect("remove .git");

    // Second run: WITH cached theory but WITHOUT .git. git fields must be null
    // because they are read fresh from the environment, not from the cache.
    let json2 = run_json(
        zetl_cmd_cached(dir.path())
            .arg("reason")
            .arg("provenance")
            .arg("flies"),
    );

    assert_eq!(
        json2["git_commit"],
        Value::Null,
        "git_commit must be null after .git is removed, even when theory cache was built inside a repo"
    );
    assert_eq!(
        json2["git_dirty"],
        Value::Null,
        "git_dirty must be null after .git is removed"
    );
    // Core provenance fields must still be valid.
    assert_eq!(json2["literal"], "flies");
    assert!(
        json2["conclusions"].as_array().unwrap().len() > 0,
        "provenance must still show conclusions after .git removal"
    );
}
