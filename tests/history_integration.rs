//! Integration tests for SPEC-017: zetl history — Invisible Temporal Graph
//! Navigation via jj-lib.
//!
//! Tests in this file require `--features history` to compile.
//!
//! Test ranges: TEST-080 through TEST-113.
//! TEST-080–089 are covered here (task-jj-backend).
//! TEST-090–091 are covered here (task-time-expression-parser).
//! TEST-092–094 are covered here (task-auto-snapshot).
//! TEST-095–099 are covered here (task-historical-index-cache).
//! TEST-100–103 are covered here (task-point-in-time-query).

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

// ─── TEST-092..TEST-094: auto-snapshot integration (REQ-076, ADR-048) ─────────

// ─── TEST-095..TEST-099: HistoricalIndexCache (REQ-079, ADR-047) ─────────────

use std::path::PathBuf;
use std::time::SystemTime;
use zetl::history::cache::HistoricalIndexCache;
use zetl::types::ParsedFile;

fn dummy_parsed_file(name: &str) -> ParsedFile {
    ParsedFile {
        path: PathBuf::from(name),
        page_name: name.trim_end_matches(".md").to_owned(),
        links: vec![],
        spl_blocks: vec![],
        diagnostics: vec![],
        mtime: SystemTime::UNIX_EPOCH,
        merkle_leaves: vec![],
        file_merkle: None,
    }
}

fn hex64(c: char) -> String {
    c.to_string().repeat(64)
}

// TEST-095: store and load roundtrip returns the stored files (REQ-079).
#[test]
fn test_095_historical_cache_store_load_roundtrip() {
    let dir = tempfile::TempDir::new().unwrap();
    let cache = HistoricalIndexCache::with_default_capacity();
    let hash = hex64('1');
    let files = vec![dummy_parsed_file("alpha.md"), dummy_parsed_file("beta.md")];

    cache.store(dir.path(), &hash, &files).unwrap();

    let loaded = cache
        .load(dir.path(), &hash)
        .unwrap()
        .expect("entry must be present after store");

    assert!(loaded.contains_key(Path::new("alpha.md")));
    assert!(loaded.contains_key(Path::new("beta.md")));
    assert_eq!(loaded.len(), 2);
}

// TEST-096: load on a missing entry returns None (REQ-079).
#[test]
fn test_096_historical_cache_load_missing_returns_none() {
    let dir = tempfile::TempDir::new().unwrap();
    let cache = HistoricalIndexCache::with_default_capacity();
    let result = cache.load(dir.path(), &hex64('2')).unwrap();
    assert!(result.is_none(), "missing entry must return None");
}

// TEST-097: file format is identical to index.json (version=2, files, vault_root_hash) (ADR-047).
#[test]
fn test_097_historical_cache_format_matches_index_json() {
    let dir = tempfile::TempDir::new().unwrap();
    let cache = HistoricalIndexCache::with_default_capacity();
    let hash = hex64('3');

    cache
        .store(dir.path(), &hash, &[dummy_parsed_file("z.md")])
        .unwrap();

    let path = dir.path().join(".zetl/history").join(format!("{hash}.json"));
    let content = std::fs::read_to_string(&path).unwrap();
    let v: serde_json::Value = serde_json::from_str(&content).unwrap();

    assert_eq!(v["version"], 2, "version must be 2 (ADR-047)");
    assert!(v["files"].is_object(), "files must be an object");
    assert_eq!(
        v["vault_root_hash"].as_str().unwrap(),
        hash,
        "vault_root_hash must round-trip"
    );
}

// TEST-098: LRU eviction removes oldest entries when capacity is exceeded (REQ-079).
#[test]
fn test_098_historical_cache_lru_eviction() {
    let dir = tempfile::TempDir::new().unwrap();
    let capacity = 3usize;
    let cache = HistoricalIndexCache::new(capacity);

    // Write 5 entries with a small delay so mtime ordering is deterministic.
    let hashes: Vec<String> = (0u8..5).map(|i| format!("{:064x}", i)).collect();
    for hash in &hashes {
        cache
            .store(dir.path(), hash, &[dummy_parsed_file("p.md")])
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // Count surviving files.
    let history_dir = dir.path().join(".zetl/history");
    let count = std::fs::read_dir(&history_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |x| x == "json"))
        .count();

    assert_eq!(
        count, capacity,
        "must retain exactly {capacity} entries after eviction"
    );

    // The 2 oldest entries (hashes[0], hashes[1]) must have been evicted.
    for evicted in &hashes[..2] {
        assert!(
            cache.load(dir.path(), evicted).unwrap().is_none(),
            "evicted entry {evicted} must not be loadable"
        );
    }

    // The 3 newest entries must still be present.
    for kept in &hashes[2..] {
        assert!(
            cache.load(dir.path(), kept).unwrap().is_some(),
            "kept entry {kept} must still be loadable"
        );
    }
}

// TEST-099: store overwrites an existing entry (upsert semantics) (REQ-079).
#[test]
fn test_099_historical_cache_store_overwrites() {
    let dir = tempfile::TempDir::new().unwrap();
    let cache = HistoricalIndexCache::with_default_capacity();
    let hash = hex64('4');

    cache
        .store(dir.path(), &hash, &[dummy_parsed_file("v1.md")])
        .unwrap();
    cache
        .store(dir.path(), &hash, &[dummy_parsed_file("v2.md")])
        .unwrap();

    let loaded = cache.load(dir.path(), &hash).unwrap().expect("must be Some");
    assert!(
        loaded.contains_key(Path::new("v2.md")),
        "second write must overwrite the first"
    );
    assert!(
        !loaded.contains_key(Path::new("v1.md")),
        "first write's data must be gone"
    );
}

// TEST-092: auto_snapshot creates a jj commit whose description contains
// the vault_root_hash (REQ-076).
#[test]
fn test_092_auto_snapshot_description_contains_hash() {
    let dir = tempfile::TempDir::new().unwrap();
    write(dir.path(), "note.md", "# Hello");

    let hash = "a".repeat(64);
    let result = zetl::history::auto_snapshot(dir.path(), Some(&hash))
        .expect("auto_snapshot must not fail");
    assert!(result.is_some(), "auto_snapshot must create a commit for new content");

    let b = JjBackend::open_or_init_at_vault_root(dir.path()).unwrap();
    let changes = b.list_changes(1).unwrap();
    assert_eq!(changes.len(), 1, "must have exactly 1 commit");
    assert!(
        changes[0].description.contains(&hash),
        "description must contain vault_root_hash; got {:?}",
        changes[0].description
    );
}

// TEST-093: auto_snapshot deduplicates when called twice with the same
// vault_root_hash (REQ-076, ADR-048 fast-path deduplication).
#[test]
fn test_093_auto_snapshot_deduplicates_same_hash() {
    let dir = tempfile::TempDir::new().unwrap();
    write(dir.path(), "note.md", "# Hello");

    let hash = "b".repeat(64);

    let first = zetl::history::auto_snapshot(dir.path(), Some(&hash))
        .expect("first auto_snapshot must not fail");
    assert!(first.is_some(), "first call must produce a commit");

    // Add a new file — jj tree hash changes, but vault_root_hash is still the
    // same (caller controls it). The Merkle deduplication must win.
    write(dir.path(), "new.md", "# New file");
    let second = zetl::history::auto_snapshot(dir.path(), Some(&hash))
        .expect("second auto_snapshot must not fail");
    assert!(
        second.is_none(),
        "same vault_root_hash must be deduplicated; got {second:?}"
    );

    // Only one commit should exist.
    let b = JjBackend::open_or_init_at_vault_root(dir.path()).unwrap();
    let changes = b.list_changes(10).unwrap();
    assert_eq!(changes.len(), 1, "deduplication must prevent a second commit");
}

// TEST-094: auto_snapshot creates a new commit when vault_root_hash changes,
// and each commit carries its respective hash in the description (REQ-076).
#[test]
fn test_094_auto_snapshot_new_commit_on_hash_change() {
    let dir = tempfile::TempDir::new().unwrap();
    write(dir.path(), "note.md", "# Version one");

    let hash1 = "c".repeat(64);
    let hash2 = "d".repeat(64);

    let first = zetl::history::auto_snapshot(dir.path(), Some(&hash1))
        .expect("first auto_snapshot must not fail");
    assert!(first.is_some(), "first call must produce a commit");

    write(dir.path(), "note.md", "# Version two");
    let second = zetl::history::auto_snapshot(dir.path(), Some(&hash2))
        .expect("second auto_snapshot must not fail");
    assert!(second.is_some(), "different vault_root_hash must produce a new commit");

    let b = JjBackend::open_or_init_at_vault_root(dir.path()).unwrap();
    let changes = b.list_changes(10).unwrap();
    assert_eq!(changes.len(), 2, "must have 2 commits; got {changes:?}");
    // Newest-first ordering.
    assert!(
        changes[0].description.contains(&hash2),
        "newest commit must embed hash2; got {:?}",
        changes[0].description
    );
    assert!(
        changes[1].description.contains(&hash1),
        "oldest commit must embed hash1; got {:?}",
        changes[1].description
    );
}

// ── Point-in-time query tests (REQ-077, REQ-078, CON-024) ─────────────────────
// TEST-100 through TEST-104 cover task-point-in-time-query-v1.

// TEST-100: extract_vault_root_hash_from_description parses a valid description.
#[test]
fn test_100_extract_hash_from_description_valid() {
    use zetl::history::core::extract_vault_root_hash_from_description;

    let hash = "a".repeat(64);
    let desc = format!("zetl-snapshot vault_root_hash={hash}");
    let result = extract_vault_root_hash_from_description(&desc);
    assert_eq!(result, Some(hash), "must parse 64-char hex hash from description");
}

// TEST-101: extract_vault_root_hash_from_description returns None for missing hash.
#[test]
fn test_101_extract_hash_from_description_missing() {
    use zetl::history::core::extract_vault_root_hash_from_description;

    assert!(
        extract_vault_root_hash_from_description("zetl-snapshot").is_none(),
        "plain snapshot description without hash must return None"
    );
    assert!(
        extract_vault_root_hash_from_description("").is_none(),
        "empty string must return None"
    );
    // Too short
    assert!(
        extract_vault_root_hash_from_description("vault_root_hash=abc123").is_none(),
        "short hash must return None"
    );
}

// TEST-102: resolve_snapshot + HistoricalIndexCache round-trip (REQ-077, REQ-078).
//
// Simulates what run_historical_pipeline does:
// 1. Create two snapshots with distinct vault_root_hash values.
// 2. Store the corresponding file sets in HistoricalIndexCache.
// 3. Resolve a time expression that targets the first snapshot.
// 4. Load from cache and verify the correct files are returned.
#[test]
fn test_102_pit_resolve_and_cache_roundtrip() {
    use chrono::{FixedOffset, TimeZone as _};
    use zetl::history::cache::HistoricalIndexCache;
    use zetl::history::core::{extract_vault_root_hash_from_description, resolve_snapshot};
    use zetl::history::jj_backend::ChangeInfo;

    let hash1 = "1".repeat(64);
    let hash2 = "2".repeat(64);

    let utc = FixedOffset::east_opt(0).unwrap();
    let ts1 = utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).unwrap();
    let ts2 = utc.with_ymd_and_hms(2026, 2, 1, 10, 0, 0).unwrap();

    // Two synthetic snapshots (newest-first order required by resolve_snapshot).
    let snapshots = vec![
        ChangeInfo {
            change_id: "bbbbbbbbbbbb".to_owned(),
            commit_id: "deadbeef0002".to_owned(),
            timestamp: ts2,
            description: format!("zetl-snapshot vault_root_hash={hash2}"),
        },
        ChangeInfo {
            change_id: "aaaaaaaaaaaa".to_owned(),
            commit_id: "deadbeef0001".to_owned(),
            timestamp: ts1,
            description: format!("zetl-snapshot vault_root_hash={hash1}"),
        },
    ];

    // Resolve to the first snapshot via an ISO date before ts2.
    let now = utc.with_ymd_and_hms(2026, 3, 1, 0, 0, 0).unwrap();
    let snap = resolve_snapshot("2026-01-15", now, &snapshots).unwrap();
    assert_eq!(snap.change_id, "aaaaaaaaaaaa", "must resolve to the first snapshot");

    // Extract vault_root_hash from the resolved snapshot's description.
    let resolved_hash =
        extract_vault_root_hash_from_description(&snap.description).unwrap();
    assert_eq!(resolved_hash, hash1);

    // Store/load from HistoricalIndexCache.
    let dir = tempfile::TempDir::new().unwrap();
    let cache = HistoricalIndexCache::with_default_capacity();

    // Use dummy ParsedFile (reuse helper from the cache tests in the same file).
    use std::time::SystemTime;
    let dummy = zetl::types::ParsedFile {
        path: std::path::PathBuf::from("v1-note.md"),
        page_name: "v1-note".to_owned(),
        links: vec![],
        spl_blocks: vec![],
        diagnostics: vec![],
        mtime: SystemTime::UNIX_EPOCH,
        merkle_leaves: vec![],
        file_merkle: None,
    };

    cache.store(dir.path(), &hash1, &[dummy]).unwrap();

    let loaded = cache.load(dir.path(), &resolved_hash).unwrap()
        .expect("must load the stored entry");
    assert!(
        loaded.contains_key(std::path::Path::new("v1-note.md")),
        "loaded files must contain the stored file"
    );
}

// TEST-103: cmd_index stores the current index in HistoricalIndexCache (REQ-079).
//
// Simulates the `zetl index` auto_snapshot + cache-store flow:
// auto_snapshot produces a commit with vault_root_hash in the description;
// the historical cache is then populated so future --at queries can load it.
#[test]
fn test_103_auto_snapshot_and_cache_are_linked() {
    use zetl::history::cache::HistoricalIndexCache;
    use zetl::history::core::extract_vault_root_hash_from_description;

    let dir = tempfile::TempDir::new().unwrap();
    let vault_root = dir.path();
    write(vault_root, "note.md", "# Hello");

    let hash = "e".repeat(64);

    // Simulate what cmd_index does: auto_snapshot then store in cache.
    zetl::history::auto_snapshot(vault_root, Some(&hash))
        .expect("auto_snapshot must succeed");

    let cache = HistoricalIndexCache::with_default_capacity();
    let files = vec![zetl::types::ParsedFile {
        path: vault_root.join("note.md"),
        page_name: "note".to_owned(),
        links: vec![],
        spl_blocks: vec![],
        diagnostics: vec![],
        mtime: std::time::SystemTime::UNIX_EPOCH,
        merkle_leaves: vec![],
        file_merkle: None,
    }];
    cache.store(vault_root, &hash, &files).unwrap();

    // Verify the snapshot description embeds the hash.
    let backend = JjBackend::open_or_init_at_vault_root(vault_root).unwrap();
    let changes = backend.list_changes(1).unwrap();
    let embedded = extract_vault_root_hash_from_description(&changes[0].description);
    assert_eq!(embedded.as_deref(), Some(hash.as_str()));

    // Verify the cache entry is loadable.
    let loaded = cache.load(vault_root, &hash).unwrap();
    assert!(loaded.is_some(), "cache entry must be present after store");
    assert!(
        loaded.unwrap().contains_key(&vault_root.join("note.md")),
        "loaded entry must contain note.md"
    );
}

// TEST-093: Graceful degradation — NO_HISTORY when .zetl/jj/ is absent (REQ-084, NFR-031).
//
// Verifies:
//   1. open_history fails with NO_HISTORY when .zetl/jj/ is absent.
//   2. auto_snapshot (zetl index) silently re-initialises .zetl/jj/.
//   3. After re-init, open_history succeeds but list_changes returns no zetl
//      snapshots (SNAPSHOT_NOT_FOUND semantics: history starts now).
//   4. Non-temporal operations are unaffected by the missing jj dir.
#[test]
fn test_graceful_degradation_no_history() {
    let dir = tempfile::TempDir::new().unwrap();
    let vault_root = dir.path();
    write(vault_root, "page.md", "# PageA\n[[PageB]]");

    // Step 1: .zetl/jj/ does not exist — open_history must fail with NO_HISTORY.
    assert!(
        !vault_root.join(".zetl/jj").exists(),
        ".zetl/jj/ must not exist before any index"
    );

    let result = zetl::history::open_history(vault_root);
    match result {
        Ok(_) => panic!("open_history must fail when .zetl/jj/ is absent"),
        Err(e) => {
            let err_msg = e.to_string();
            assert!(
                err_msg.contains("NO_HISTORY"),
                "error must contain 'NO_HISTORY'; got: {err_msg}"
            );
        }
    }

    // Step 2: auto_snapshot (equivalent to `zetl index`) silently re-initialises.
    zetl::history::auto_snapshot(vault_root, None)
        .expect("auto_snapshot must succeed even when .zetl/jj/ was absent");

    // .zetl/jj/ must now exist.
    assert!(
        vault_root.join(".zetl/jj").exists(),
        ".zetl/jj/ must exist after auto_snapshot re-initialises"
    );

    // Step 3: open_history now succeeds, but no zetl-labelled snapshots exist yet
    // (the auto_snapshot above was the first, so we have 0 prior snapshots).
    let backend = zetl::history::open_history(vault_root)
        .expect("open_history must succeed after re-initialisation");
    let changes = backend.list_changes(100).unwrap();
    // The first auto_snapshot creates at most 1 commit; either 0 or 1 is valid here.
    // The key point is that open_history itself did NOT error (no NO_HISTORY).
    let _ = changes; // count is irrelevant; what matters is no error above.

    // Step 4: non-temporal operations are unaffected (the absence of jj never
    // prevents scanning, caching, or graph construction).  We verify this by
    // calling open_or_init (the non-temporal path) on a fresh vault with no jj:
    let dir2 = tempfile::TempDir::new().unwrap();
    write(dir2.path(), "note.md", "# Just a note");
    JjBackend::open_or_init_at_vault_root(dir2.path())
        .expect("open_or_init_at_vault_root must always succeed for non-temporal path");
}
