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

// TEST-080: JjBackend initialises a new workspace.
#[test]
fn test_080_init_creates_jj_dir() {
    let dir = tempfile::TempDir::new().unwrap();
    JjBackend::open_or_init(dir.path()).expect("open_or_init must succeed");
    assert!(
        dir.path().join(".jj").is_dir(),
        "jj metadata directory must exist after init"
    );
}

// TEST-081: Opening an already-initialised workspace succeeds.
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
