//! Integration tests for SPEC-017: zetl history — Invisible Temporal Graph
//! Navigation via jj-lib.
//!
//! Tests in this file require `--features history` to compile.
//!
//! Test ranges: TEST-080 through TEST-113.
//! TEST-080–089 are covered here (task-jj-backend).
//! Remaining tests will be added incrementally as IMPL-017 tasks complete.

use std::fs;
use std::path::Path;

use zetl::history::jj_backend::{JjBackend, VcsBackend as _};

fn write(root: &Path, name: &str, content: &str) {
    fs::write(root.join(name), content).unwrap();
}

// TEST-080: VCS initialisation stores jj metadata in .zetl/jj/ (REQ-075, ADR-045).
// Verifies: .zetl/jj/.jj/ created; no .jj/ at vault root; idempotent.
#[test]
fn test_080_vcs_init_creates_zetl_jj_dir() {
    let dir = tempfile::TempDir::new().unwrap();
    let vault_root = dir.path();

    // First call: must initialise.
    JjBackend::open_or_init_at_vault_root(vault_root)
        .expect("open_or_init_at_vault_root must succeed on first call");

    // .zetl/jj/.jj/ must exist (jj metadata is inside .zetl/).
    assert!(
        vault_root.join(".zetl/jj/.jj").is_dir(),
        ".zetl/jj/.jj/ must exist after init"
    );

    // .jj/ must NOT exist at the vault root (ADR-045: invisible to users).
    assert!(
        !vault_root.join(".jj").exists(),
        ".jj/ must not exist at vault root"
    );

    // Second call: must be idempotent.
    JjBackend::open_or_init_at_vault_root(vault_root)
        .expect("open_or_init_at_vault_root must succeed on subsequent calls");
}

// TEST-081: Opening an already-initialised workspace succeeds (original open_or_init).
#[test]
fn test_081_open_existing_workspace() {
    let dir = tempfile::TempDir::new().unwrap();
    JjBackend::open_or_init(dir.path()).expect("first init");
    JjBackend::open_or_init(dir.path()).expect("second open must not fail");
}

// TEST-082: First snapshot of a non-empty vault returns a change ID.
#[test]
fn test_082_first_snapshot_returns_change_id() {
    let dir = tempfile::TempDir::new().unwrap();
    write(dir.path(), "page.md", "# Hello");
    let mut b = JjBackend::open_or_init(dir.path()).unwrap();
    let cid = b.snapshot("test").unwrap();
    assert!(cid.is_some(), "must return Some(change_id) for new content");
}

// TEST-083: Snapshotting with no file changes returns None.
#[test]
fn test_083_unchanged_snapshot_is_none() {
    let dir = tempfile::TempDir::new().unwrap();
    write(dir.path(), "page.md", "static content");
    let mut b = JjBackend::open_or_init(dir.path()).unwrap();
    b.snapshot("first").unwrap().unwrap();
    let second = b.snapshot("second").unwrap();
    assert!(
        second.is_none(),
        "snapshot with no file changes must return None"
    );
}

// TEST-084: Adding a file triggers a new snapshot.
#[test]
fn test_084_new_file_triggers_snapshot() {
    let dir = tempfile::TempDir::new().unwrap();
    write(dir.path(), "a.md", "first");
    let mut b = JjBackend::open_or_init(dir.path()).unwrap();
    b.snapshot("snap1").unwrap().unwrap();
    write(dir.path(), "b.md", "second");
    let s2 = b.snapshot("snap2").unwrap();
    assert!(s2.is_some(), "adding a file must produce a new snapshot");
}

// TEST-085: list_changes returns no zetl commits before any snapshot.
#[test]
fn test_085_list_changes_empty_before_snapshot() {
    let dir = tempfile::TempDir::new().unwrap();
    write(dir.path(), "x.md", "content");
    let b = JjBackend::open_or_init(dir.path()).unwrap();
    let changes = b.list_changes(100).unwrap();
    assert!(
        changes.is_empty(),
        "must have no zetl commits before snapshot; got {changes:?}"
    );
}

// TEST-086: list_changes returns commits after snapshots.
#[test]
fn test_086_list_changes_after_two_snapshots() {
    let dir = tempfile::TempDir::new().unwrap();
    write(dir.path(), "a.md", "v1");
    let mut b = JjBackend::open_or_init(dir.path()).unwrap();
    b.snapshot("snap1").unwrap().unwrap();
    write(dir.path(), "a.md", "v2");
    b.snapshot("snap2").unwrap().unwrap();
    let changes = b.list_changes(100).unwrap();
    assert_eq!(changes.len(), 2, "must have exactly 2 commits; got {changes:?}");
    // Newest-first ordering.
    assert_eq!(changes[0].description, "snap2");
    assert_eq!(changes[1].description, "snap1");
}

// TEST-087: resolve_change_id returns correct metadata.
#[test]
fn test_087_resolve_change_id_roundtrip() {
    let dir = tempfile::TempDir::new().unwrap();
    write(dir.path(), "r.md", "roundtrip content");
    let mut b = JjBackend::open_or_init(dir.path()).unwrap();
    let cid = b.snapshot("roundtrip desc").unwrap().unwrap();
    let info = b.resolve_change_id(&cid).unwrap();
    assert_eq!(info.description, "roundtrip desc");
    assert!(!info.commit_id.is_empty(), "commit_id must be non-empty");
}

// TEST-088: read_file_at returns correct file content.
#[test]
fn test_088_read_file_at_returns_content() {
    let dir = tempfile::TempDir::new().unwrap();
    write(dir.path(), "note.md", "# Zetl note\n[[backlink]]");
    let mut b = JjBackend::open_or_init(dir.path()).unwrap();
    let cid = b.snapshot("note snap").unwrap().unwrap();
    let bytes = b.read_file_at(&cid, "note.md").unwrap();
    assert_eq!(
        String::from_utf8(bytes).unwrap(),
        "# Zetl note\n[[backlink]]"
    );
}

// TEST-089: read_file_at on a non-existent path returns Err.
#[test]
fn test_089_read_file_at_missing_path_errors() {
    let dir = tempfile::TempDir::new().unwrap();
    write(dir.path(), "exists.md", "present");
    let mut b = JjBackend::open_or_init(dir.path()).unwrap();
    let cid = b.snapshot("snap").unwrap().unwrap();
    assert!(
        b.read_file_at(&cid, "ghost.md").is_err(),
        "missing path must return Err"
    );
}

// ─── TEST-090 / TEST-091: time-expression-parser integration ──────────────────
//
// These tests exercise parse_time_expr and resolve_snapshot against real jj
// snapshots (REQ-077, CON-024).

use chrono::{FixedOffset, TimeZone as _};
use zetl::history::core::{resolve_snapshot, TimeExpr, parse_time_expr};

// TEST-090: parse_time_expr resolves ISO 8601 and relative forms correctly.
#[test]
fn test_090_time_expression_parser_forms() {
    let now = FixedOffset::east_opt(0)
        .unwrap()
        .with_ymd_and_hms(2026, 3, 4, 12, 0, 0)
        .unwrap();

    // ISO 8601 date
    assert!(matches!(
        parse_time_expr("2026-01-01", now).unwrap(),
        TimeExpr::Absolute(_)
    ));

    // ISO 8601 datetime
    assert!(matches!(
        parse_time_expr("2026-01-01T00:00:00Z", now).unwrap(),
        TimeExpr::Absolute(_)
    ));

    // Relative
    assert!(matches!(
        parse_time_expr("yesterday", now).unwrap(),
        TimeExpr::Absolute(_)
    ));
    assert!(matches!(
        parse_time_expr("7 days ago", now).unwrap(),
        TimeExpr::Absolute(_)
    ));

    // HEAD refs
    assert_eq!(parse_time_expr("HEAD", now).unwrap(), TimeExpr::HeadOffset(0));
    assert_eq!(parse_time_expr("HEAD~2", now).unwrap(), TimeExpr::HeadOffset(2));

    // VCS ref passthrough
    assert_eq!(
        parse_time_expr("my-branch", now).unwrap(),
        TimeExpr::Ref("my-branch".to_owned())
    );
}

// TEST-091: resolve_snapshot returns most recent snapshot at or before the
// resolved time; SNAPSHOT_NOT_FOUND when no match.
#[test]
fn test_091_resolve_snapshot_against_real_jj() {
    let dir = tempfile::TempDir::new().unwrap();
    write(dir.path(), "v1.md", "version one");

    let mut b = JjBackend::open_or_init(dir.path()).unwrap();
    b.snapshot("snap-v1").unwrap().unwrap();

    write(dir.path(), "v2.md", "version two");
    b.snapshot("snap-v2").unwrap().unwrap();

    let snapshots = b.list_changes(100).unwrap();
    assert_eq!(snapshots.len(), 2);

    let now = chrono::Utc::now().fixed_offset();

    // HEAD~0 → most recent snapshot (snap-v2).
    let result = resolve_snapshot("HEAD~0", now, &snapshots).unwrap();
    assert_eq!(result.description, "snap-v2");

    // HEAD~1 → second snapshot (snap-v1).
    let result = resolve_snapshot("HEAD~1", now, &snapshots).unwrap();
    assert_eq!(result.description, "snap-v1");

    // HEAD~2 → out of range.
    let err = resolve_snapshot("HEAD~2", now, &snapshots).unwrap_err();
    assert!(
        err.to_string().contains("SNAPSHOT_NOT_FOUND"),
        "expected SNAPSHOT_NOT_FOUND, got {err}"
    );

    // ISO date far in the future → most recent snapshot (both qualify).
    let result = resolve_snapshot("2099-12-31", now, &snapshots).unwrap();
    assert_eq!(result.description, "snap-v2");

    // ISO date far in the past → SNAPSHOT_NOT_FOUND.
    let err = resolve_snapshot("2000-01-01", now, &snapshots).unwrap_err();
    assert!(
        err.to_string().contains("SNAPSHOT_NOT_FOUND"),
        "expected SNAPSHOT_NOT_FOUND, got {err}"
    );
}
