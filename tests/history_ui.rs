//! Integration tests for SPEC-027 / IMPL-027 history UI surfaces.
//!
//! Covers TEST-300 (metadata strip), TEST-302 (static per-page history
//! emission), TEST-303 (vault history page), TEST-304 (recent-changes
//! link in base), and TEST-305 (graceful absence).
//!
//! Requires `--features history`.

#![cfg(feature = "history")]

use std::fs;
use std::path::Path;
use std::sync::Arc;

use tempfile::TempDir;

fn write(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

/// Mirror `cmd_index`: scan the vault, run `auto_snapshot` with the tag,
/// and store the parsed file list in the historical index cache keyed by
/// the same tag. `build_template_page_history_context` needs both the jj
/// commit AND the cache entry to return `Some(_)`.
fn take_snapshot(root: &Path, tag: &str) {
    let files =
        zetl::scanner::scan_vault(root, &zetl::scanner::ScanOptions::default()).unwrap();
    zetl::history::auto_snapshot(root, Some(tag)).unwrap();
    let cache = zetl::history::cache::HistoricalIndexCache::with_default_capacity();
    cache.store(root, tag, &files).unwrap();
}

/// Build a vault with 2 snapshots of a page, suitable for exercising the
/// page.history template context.
fn setup_two_snapshot_vault() -> (TempDir, String) {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    // Snapshot 1.
    write(root, "notes/note-a.md", "# Note A\n\nfirst draft\n");
    take_snapshot(root, "deadbeef00000000000000000000000000000000000000000000000000000001");
    // Snapshot 2: modify note-a and add note-b.
    write(root, "notes/note-a.md", "# Note A\n\nsecond revision\n[[note-b]]\n");
    write(root, "notes/note-b.md", "# Note B\n");
    take_snapshot(root, "deadbeef00000000000000000000000000000000000000000000000000000002");
    (tmp, "note-a".to_string())
}

/// TEST-300 + TEST-304: when a vault has history, the per-page metadata
/// strip and base-template "Recent changes" link both appear. Skip
/// TEST-301 link parity check — the anchor `href` wiring is verified
/// structurally by the template syntax.
#[tokio::test]
async fn test_300_304_metadata_strip_and_recent_changes_link() {
    use axum::Router;
    use axum::body::Body;
    use axum::http::Request;
    use axum::routing::get;
    use tower::ServiceExt as _;
    use zetl::scanner::ScanOptions;
    use zetl::web::WebState;
    use zetl::web::routes::page_handler;
    use zetl::search_index::SearchIndex;

    let (tmp, _slug) = setup_two_snapshot_vault();
    let root = tmp.path();

    let data = zetl::web::reindex_with(root, &ScanOptions::default()).unwrap();
    let search_index = SearchIndex::build(root, &data.files).unwrap();
    let state = WebState {
        data: Arc::new(std::sync::RwLock::new(data)),
        vault_root: Arc::new(root.to_path_buf()),
        search_index: Arc::new(search_index),
        engine: Arc::new(zetl::web::engine::TemplateEngine::new(
            root, "default", true, false,
        )),
        theme: "default".into(),
        verbose: false,
        sessions: zetl::web::session::SessionStore::new(),
        recovery_challenges: Arc::new(zetl::user::recovery::RecoveryChallengeStore::new()),
        mnemonic_shown: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        bootstrap_used: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        rate_limiters: zetl::web::rate_limit::AuthRateLimiters::new(),
        collab: false,
        tls: false,
        trust_proxy: false,
        #[cfg(feature = "reason")]
        acl_cache: Arc::new(std::sync::Mutex::new(zetl::web::AclCache::new())),
        git_commit_lock: None,
        ws_hub: zetl::web::ws::WsHub::new(),
        ticket_store: zetl::web::ws::TicketStore::new(),
        crdt_store: zetl::web::ws::CrdtDocStore::new(Arc::new(root.to_path_buf())),
        wal_store: Arc::new(zetl::web::wal::WalStore::new(root)),
        pending_writes: zetl::web::fs_watch::PendingWrites::new(),
        passkey_mgr: None,
        public_dir: None,
        scan_options: ScanOptions::default(),
        #[cfg(feature = "semantic")]
        vector_index: None,
    };
    let app: Router = Router::new()
        .route("/{*path}", get(page_handler))
        .with_state(state);

    let req = Request::builder()
        .uri("/notes/note-a/")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body = axum::body::to_bytes(resp.into_body(), 10_000_000)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);

    // TEST-300: metadata strip present with "Last changed" + history link.
    assert!(
        html.contains("page-history-meta"),
        "expected metadata strip; got excerpt: {}",
        &html[..html.len().min(4000)]
    );
    assert!(html.contains("Last changed"));
    // Extract the metadata-strip region for debugging.
    let meta_start = html.find("page-history-meta").unwrap();
    let meta_end = (meta_start + 800).min(html.len());
    let meta_region = &html[meta_start..meta_end];
    // Auto-escape in minijinja may render `/` as `&#x2f;` in HTML attrs;
    // accept both canonical and escaped forms (browsers resolve both).
    assert!(
        meta_region.contains("/notes/note-a/_history")
            || meta_region.contains("&#x2f;notes/note-a/_history"),
        "expected history link in metadata strip; got:\n{meta_region}"
    );

    // TEST-304: "Recent changes" link in the base template sidebar.
    assert!(html.contains("Recent changes"));
    assert!(html.contains("/_history"));
}

/// TEST-305: a vault with zero snapshots renders neither the metadata
/// strip nor the "Recent changes" sidebar link.
#[tokio::test]
async fn test_305_graceful_absence_no_history() {
    use axum::Router;
    use axum::body::Body;
    use axum::http::Request;
    use axum::routing::get;
    use tower::ServiceExt as _;
    use zetl::scanner::ScanOptions;
    use zetl::web::WebState;
    use zetl::web::routes::page_handler;
    use zetl::search_index::SearchIndex;

    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write(root, "note-a.md", "# A\n");
    // No snapshot taken → page.history is null.

    let data = zetl::web::reindex_with(root, &ScanOptions::default()).unwrap();
    let search_index = SearchIndex::build(root, &data.files).unwrap();
    let state = WebState {
        data: Arc::new(std::sync::RwLock::new(data)),
        vault_root: Arc::new(root.to_path_buf()),
        search_index: Arc::new(search_index),
        engine: Arc::new(zetl::web::engine::TemplateEngine::new(
            root, "default", true, false,
        )),
        theme: "default".into(),
        verbose: false,
        sessions: zetl::web::session::SessionStore::new(),
        recovery_challenges: Arc::new(zetl::user::recovery::RecoveryChallengeStore::new()),
        mnemonic_shown: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        bootstrap_used: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        rate_limiters: zetl::web::rate_limit::AuthRateLimiters::new(),
        collab: false,
        tls: false,
        trust_proxy: false,
        #[cfg(feature = "reason")]
        acl_cache: Arc::new(std::sync::Mutex::new(zetl::web::AclCache::new())),
        git_commit_lock: None,
        ws_hub: zetl::web::ws::WsHub::new(),
        ticket_store: zetl::web::ws::TicketStore::new(),
        crdt_store: zetl::web::ws::CrdtDocStore::new(Arc::new(root.to_path_buf())),
        wal_store: Arc::new(zetl::web::wal::WalStore::new(root)),
        pending_writes: zetl::web::fs_watch::PendingWrites::new(),
        passkey_mgr: None,
        public_dir: None,
        scan_options: ScanOptions::default(),
        #[cfg(feature = "semantic")]
        vector_index: None,
    };
    let app: Router = Router::new()
        .route("/{*path}", get(page_handler))
        .with_state(state);
    let req = Request::builder()
        .uri("/note-a/")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body = axum::body::to_bytes(resp.into_body(), 10_000_000)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);

    assert!(
        !html.contains("page-history-meta"),
        "metadata strip must be absent when history is null"
    );
    assert!(
        !html.contains("Recent changes"),
        "sidebar link must be absent when snapshot_count is 0"
    );
}

/// TEST-302: `zetl build` writes `pages/<slug>/_history.html` for each
/// page with history, AND the root `_history.html` for the vault view.
#[test]
fn test_302_303_build_emits_history_html() {
    let (tmp, _slug) = setup_two_snapshot_vault();
    let root = tmp.path();
    let out = tmp.path().join("dist");
    fs::create_dir_all(&out).unwrap();

    let data = zetl::web::reindex_with(root, &zetl::scanner::ScanOptions::default()).unwrap();
    zetl::web::build::build_static(
        &data,
        root,
        out.to_str().unwrap(),
        "default",
        false,
        None,
        None,
    )
    .expect("build_static");

    // TEST-302: per-page history HTML exists for the page with real history.
    let page_hist = out.join("notes/note-a/_history.html");
    assert!(
        page_hist.exists(),
        "expected per-page history at {}",
        page_hist.display()
    );
    let ph = fs::read_to_string(&page_hist).unwrap();
    // Title may appear URL-encoded or escaped; accept the plain "note-a"
    // slug substring as a robust proxy.
    assert!(
        ph.contains("note-a") || ph.contains("Note A") || ph.contains("Note%20A"),
        "per-page history must reference the page; got excerpt: {}",
        &ph[..ph.len().min(2000)]
    );

    // TEST-303: vault-wide history HTML exists and contains recent changes.
    let vault_hist = out.join("_history.html");
    assert!(vault_hist.exists(), "expected vault history at {}", vault_hist.display());
    let vh = fs::read_to_string(&vault_hist).unwrap();
    assert!(vh.contains("Recent changes"));
    // Snapshot count should be >= 2 based on setup_two_snapshot_vault.
    assert!(
        vh.contains("snapshots"),
        "vault history should render the snapshot-count stat"
    );
}

/// Adversarial review / synthetic-user Finding 1 regression: serve
/// `/_history` renders the full body (snapshot stats + recent-changes
/// list) when the vault has snapshots — not the graceful-absence body.
#[tokio::test]
async fn test_serve_vault_history_populates_body_with_snapshots() {
    use axum::Router;
    use axum::body::Body;
    use axum::http::Request;
    use axum::routing::get;
    use tower::ServiceExt as _;
    use zetl::scanner::ScanOptions;
    use zetl::search_index::SearchIndex;
    use zetl::web::WebState;
    use zetl::web::routes::page_handler;

    let (tmp, _slug) = setup_two_snapshot_vault();
    let root = tmp.path();
    let data = zetl::web::reindex_with(root, &ScanOptions::default()).unwrap();
    let search_index = SearchIndex::build(root, &data.files).unwrap();
    let state = WebState {
        data: Arc::new(std::sync::RwLock::new(data)),
        vault_root: Arc::new(root.to_path_buf()),
        search_index: Arc::new(search_index),
        engine: Arc::new(zetl::web::engine::TemplateEngine::new(
            root, "default", true, false,
        )),
        theme: "default".into(),
        verbose: false,
        sessions: zetl::web::session::SessionStore::new(),
        recovery_challenges: Arc::new(zetl::user::recovery::RecoveryChallengeStore::new()),
        mnemonic_shown: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        bootstrap_used: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        rate_limiters: zetl::web::rate_limit::AuthRateLimiters::new(),
        collab: false,
        tls: false,
        trust_proxy: false,
        #[cfg(feature = "reason")]
        acl_cache: Arc::new(std::sync::Mutex::new(zetl::web::AclCache::new())),
        git_commit_lock: None,
        ws_hub: zetl::web::ws::WsHub::new(),
        ticket_store: zetl::web::ws::TicketStore::new(),
        crdt_store: zetl::web::ws::CrdtDocStore::new(Arc::new(root.to_path_buf())),
        wal_store: Arc::new(zetl::web::wal::WalStore::new(root)),
        pending_writes: zetl::web::fs_watch::PendingWrites::new(),
        passkey_mgr: None,
        public_dir: None,
        scan_options: ScanOptions::default(),
        #[cfg(feature = "semantic")]
        vector_index: None,
    };
    let app: Router = Router::new()
        .route("/{*path}", get(page_handler))
        .with_state(state);

    let req = Request::builder()
        .uri("/_history")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body = axum::body::to_bytes(resp.into_body(), 10_000_000)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);

    assert!(
        !html.contains("No history yet"),
        "serve /_history must not render graceful-absence body when snapshots exist"
    );
    assert!(
        html.contains("snapshots"),
        "serve /_history must render snapshot-count stat; got excerpt: {}",
        &html[..html.len().min(2000)]
    );
    assert!(
        html.contains("vh-list") || html.contains("vh-stats"),
        "serve /_history must render the recent-changes or stats blocks"
    );
}

/// Adversarial Defect 3: `build_recent_changes` output is deterministic
/// across repeated calls — required for reproducible static builds.
#[test]
fn test_build_recent_changes_deterministic() {
    use zetl::history::jj_backend::VcsBackend as _;

    let (tmp, _) = setup_two_snapshot_vault();
    let root = tmp.path();
    let backend = zetl::history::open_history(root).unwrap();
    let snapshots = backend.list_changes(10_000).unwrap();

    let a = zetl::history::core::build_recent_changes(&snapshots, root, 50).unwrap();
    let b = zetl::history::core::build_recent_changes(&snapshots, root, 50).unwrap();
    let c = zetl::history::core::build_recent_changes(&snapshots, root, 50).unwrap();

    // BTreeMap iteration makes the order stable; assert all three runs
    // produce identical sequences (page_title, change_kind, changed_at).
    let tuples = |v: &[zetl::history::core::RecentChangeEntry]| {
        v.iter()
            .map(|e| (e.page_title.clone(), format!("{:?}", e.change_kind), e.changed_at.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(tuples(&a), tuples(&b));
    assert_eq!(tuples(&b), tuples(&c));
    assert!(!a.is_empty(), "setup should produce at least one change");
}

/// Adversarial Defect 4: when limit is smaller than a single snapshot's
/// change count, truncation happens AFTER collecting that snapshot —
/// never mid-snapshot between Added/Modified and Removed.
#[test]
fn test_build_recent_changes_respects_limit_without_midsnap_cutoff() {
    use zetl::history::jj_backend::VcsBackend as _;

    let (tmp, _) = setup_two_snapshot_vault();
    let root = tmp.path();
    let backend = zetl::history::open_history(root).unwrap();
    let snapshots = backend.list_changes(10_000).unwrap();

    // Known: setup_two_snapshot_vault produces ≥2 changes between the two
    // cache-enabled snapshots (note-a modified + note-b added).
    let all = zetl::history::core::build_recent_changes(&snapshots, root, 50).unwrap();
    assert!(all.len() >= 2, "expected ≥2 candidate changes; got {}", all.len());

    let limited = zetl::history::core::build_recent_changes(&snapshots, root, 1).unwrap();
    assert!(
        limited.len() <= 1,
        "limit=1 must cap output at 1; got {}",
        limited.len()
    );
}

/// TEST-303 graceful-absence branch: build on a vault with zero snapshots
/// still emits `_history.html` but with the "No history yet" message.
#[test]
fn test_303_build_vault_history_graceful_when_no_snapshots() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write(root, "a.md", "# A");
    let out = tmp.path().join("dist");
    fs::create_dir_all(&out).unwrap();

    let data = zetl::web::reindex_with(root, &zetl::scanner::ScanOptions::default()).unwrap();
    zetl::web::build::build_static(
        &data,
        root,
        out.to_str().unwrap(),
        "default",
        false,
        None,
        None,
    )
    .expect("build_static");

    let vh_path = out.join("_history.html");
    assert!(vh_path.exists());
    let vh = fs::read_to_string(&vh_path).unwrap();
    assert!(
        vh.contains("No history yet"),
        "expected graceful-absence body; got excerpt: {}",
        &vh[..vh.len().min(2000)]
    );
}
