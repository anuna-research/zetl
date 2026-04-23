//! Integration tests for SPEC-035: Collaborative Static Asset Uploads.
//!
//! Covers upload, serve, list, delete, ACL gating, and quota enforcement.

use std::fs;
use std::path::Path;
use std::sync::{Arc, RwLock};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::routing::{get, post};
use axum::Router;
use tower::ServiceExt;

use zetl::search_index::SearchIndex;
use zetl::web::engine::TemplateEngine;
use zetl::web::routes::{
    admin_assets_handler, delete_asset_handler, index_handler, list_assets_handler,
    serve_asset_handler, upload_asset_handler,
};
use zetl::web::WebState;

// ── helpers ───────────────────────────────────────────────────────────────────

fn write_file(root: &Path, relative: &str, content: &[u8]) {
    let full = root.join(relative);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).expect("create parent dirs");
    }
    fs::write(&full, content).expect("write test file");
}

/// Build a WebState in collab mode with an owner profile.
fn build_collab_state(vault_root: &Path) -> (WebState, String, String) {
    fs::create_dir_all(vault_root.join(".zetl/collab")).unwrap();
    fs::create_dir_all(vault_root.join(".zetl/users")).unwrap();

    // Create owner profile
    let owner_id = zetl::user::generate_user_id("Owner");
    let profile = zetl::user::UserProfile {
        id: owner_id.clone(),
        name: "Owner".to_string(),
        created_at: "2026-04-23T10:00:00Z".to_string(),
        invited_by: None,
        owner: true,
        credentials: vec![],
        recovery_pubkey: "dGVzdA".to_string(),
        agent_token_generation: 0,
    };
    zetl::user::save_profile(vault_root, &profile).unwrap();

    let data = zetl::web::reindex(vault_root).expect("reindex");
    let search_index = SearchIndex::build(vault_root, &data.files).expect("build search index");

    let state = WebState {
        data: Arc::new(RwLock::new(data)),
        vault_root: Arc::new(vault_root.to_path_buf()),
        search_index: Arc::new(search_index),
        engine: Arc::new(TemplateEngine::new(vault_root, "default", true, false)),
        theme: "default".to_string(),
        verbose: false,
        sessions: zetl::web::session::SessionStore::new(),
        recovery_challenges: Arc::new(zetl::user::recovery::RecoveryChallengeStore::new()),
        mnemonic_shown: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        bootstrap_used: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        rate_limiters: zetl::web::rate_limit::AuthRateLimiters::new(),
        collab: true,
        tls: false,
        trust_proxy: false,
        #[cfg(feature = "reason")]
        acl_cache: Arc::new(std::sync::Mutex::new(zetl::web::AclCache::new())),
        git_commit_lock: None,
        ws_hub: zetl::web::ws::WsHub::new(),
        ticket_store: zetl::web::ws::TicketStore::new(),
        crdt_store: zetl::web::ws::CrdtDocStore::new(Arc::new(vault_root.to_path_buf())),
        wal_store: Arc::new(zetl::web::wal::WalStore::new(vault_root)),
        pending_writes: zetl::web::fs_watch::PendingWrites::new(),
        passkey_mgr: None,
        public_dir: None,
        scan_options: zetl::scanner::ScanOptions::default(),
        #[cfg(feature = "semantic")]
        vector_index: None,
        asset_storage: zetl::assets::store::StorageCounterGuard::new(0),
        asset_max_file_bytes: 1024 * 1024, // 1 MiB for tests
        asset_max_total_bytes: 5 * 1024 * 1024, // 5 MiB for tests
    };

    let session_token = state.sessions.create(&owner_id);
    let csrf = state.sessions.csrf_token(&session_token).unwrap();
    (state, session_token, csrf)
}

fn asset_router(state: WebState) -> Router {
    Router::new()
        .route("/", get(index_handler))
        .route("/assets/{*path}", get(serve_asset_handler))
        .route("/api/assets", get(list_assets_handler))
        .route(
            "/api/assets/{*slug}",
            post(upload_asset_handler).delete(delete_asset_handler),
        )
        .route("/_admin/assets", get(admin_assets_handler))
        .with_state(state)
}

async fn get_status(app: &Router, uri: &str) -> StatusCode {
    let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    resp.status()
}

async fn post_bytes(
    app: &Router,
    uri: &str,
    body: Vec<u8>,
    content_type: &str,
    cookie: &str,
    csrf: &str,
) -> (StatusCode, String) {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::COOKIE, format!("zetl_session={cookie}"))
        .header("X-CSRF-Token", csrf)
        .header("X-Create", "true")
        .body(Body::from(body))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).to_string())
}

async fn delete_asset(
    app: &Router,
    uri: &str,
    cookie: &str,
    csrf: &str,
) -> StatusCode {
    let req = Request::builder()
        .method("DELETE")
        .uri(uri)
        .header(header::COOKIE, format!("zetl_session={cookie}"))
        .header("X-CSRF-Token", csrf)
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    resp.status()
}

async fn get_body(app: &Router, uri: &str) -> (StatusCode, Vec<u8>, String) {
    let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let ct = resp
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
        .await
        .unwrap()
        .to_vec();
    (status, body, ct)
}

async fn get_body_auth(app: &Router, uri: &str, cookie: &str) -> (StatusCode, Vec<u8>, String) {
    let req = Request::builder()
        .uri(uri)
        .header(header::COOKIE, format!("zetl_session={cookie}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let ct = resp
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
        .await
        .unwrap()
        .to_vec();
    (status, body, ct)
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn asset_upload_and_serve_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let (state, cookie, csrf) = build_collab_state(tmp.path());
    let app = asset_router(state);

    let slug = "images/test.png";
    let png_bytes = vec![0x89, 0x50, 0x4E, 0x47]; // PNG magic bytes

    // Upload
    let (status, body) = post_bytes(
        &app,
        &format!("/api/assets/{slug}"),
        png_bytes.clone(),
        "image/png",
        &cookie,
        &csrf,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "upload failed: {body}");
    assert!(body.contains("test.png"), "response should contain slug");

    // Serve (authenticated — collab mode defaults to mixed visibility)
    let (status, body, ct) = get_body_auth(&app, &format!("/assets/{slug}"), &cookie).await;
    assert_eq!(status, StatusCode::OK, "serve failed");
    assert_eq!(body, png_bytes, "served bytes mismatch");
    assert_eq!(ct, "image/png", "content-type mismatch");

    // List
    let (status, body, ct) = get_body_auth(&app, "/api/assets", &cookie).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ct, "application/json");
    let body_str = String::from_utf8_lossy(&body);
    assert!(body_str.contains("test.png"), "list should contain asset");

    // Delete
    let status = delete_asset(&app, &format!("/api/assets/{slug}"), &cookie, &csrf).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "delete failed");

    // Verify deleted
    let status = get_status(&app, &format!("/assets/{slug}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "asset should be gone");
}

#[tokio::test]
async fn asset_upload_requires_auth() {
    let tmp = tempfile::tempdir().unwrap();
    let (state, _cookie, _csrf) = build_collab_state(tmp.path());
    let app = asset_router(state);

    let req = Request::builder()
        .method("POST")
        .uri("/api/assets/foo.png")
        .header(header::CONTENT_TYPE, "image/png")
        .header("X-Create", "true")
        .body(Body::from(vec![0x89, 0x50, 0x4E, 0x47]))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn asset_upload_rejects_oversized_file() {
    let tmp = tempfile::tempdir().unwrap();
    let (state, cookie, csrf) = build_collab_state(tmp.path());
    let app = asset_router(state);

    let big = vec![0u8; 2 * 1024 * 1024]; // 2 MiB > 1 MiB limit
    let (status, body) = post_bytes(
        &app,
        "/api/assets/big.png",
        big,
        "image/png",
        &cookie,
        &csrf,
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "should reject oversized: {body}");
    assert!(body.contains("file_too_large"), "error should mention size");
}

#[tokio::test]
async fn asset_upload_rejects_disallowed_mime() {
    let tmp = tempfile::tempdir().unwrap();
    let (state, cookie, csrf) = build_collab_state(tmp.path());
    let app = asset_router(state);

    let (status, body) = post_bytes(
        &app,
        "/api/assets/malware.exe",
        vec![1, 2, 3, 4],
        "application/x-msdownload",
        &cookie,
        &csrf,
    )
    .await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE, "should reject bad mime: {body}");
    assert!(body.contains("mime_type_not_allowed"), "error should mention mime");
}

#[tokio::test]
async fn asset_upload_rejects_bad_slug() {
    let tmp = tempfile::tempdir().unwrap();
    let (state, cookie, csrf) = build_collab_state(tmp.path());
    let app = asset_router(state);

    let (status, _body) = post_bytes(
        &app,
        "/api/assets/../etc/passwd",
        vec![0x89, 0x50, 0x4E, 0x47],
        "image/png",
        &cookie,
        &csrf,
    )
    .await;
    // Bad slug is rejected with 400 Bad Request (REQ-3510)
    assert_eq!(status, StatusCode::BAD_REQUEST, "bad slug should return 400: got {status}");
}

#[tokio::test]
async fn asset_upload_requires_create_or_overwrite_header() {
    let tmp = tempfile::tempdir().unwrap();
    let (state, cookie, csrf) = build_collab_state(tmp.path());
    let app = asset_router(state);

    let req = Request::builder()
        .method("POST")
        .uri("/api/assets/noheader.png")
        .header(header::CONTENT_TYPE, "image/png")
        .header(header::COOKIE, format!("zetl_session={cookie}"))
        .header("X-CSRF-Token", &csrf)
        .body(Body::from(vec![0x89, 0x50, 0x4E, 0x47]))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "should require X-Create or X-Overwrite");
}

#[tokio::test]
async fn asset_admin_ui_renders() {
    let tmp = tempfile::tempdir().unwrap();
    let (state, cookie, csrf) = build_collab_state(tmp.path());
    let app = asset_router(state);

    let req = Request::builder()
        .uri("/_admin/assets")
        .header(header::COOKIE, format!("zetl_session={cookie}"))
        .header("X-CSRF-Token", &csrf)
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);
    assert_eq!(status, StatusCode::OK, "admin UI should render");
    assert!(html.contains("Assets"), "admin UI should contain title");
}

#[tokio::test]
async fn asset_serve_includes_cache_control() {
    let tmp = tempfile::tempdir().unwrap();
    let (state, cookie, csrf) = build_collab_state(tmp.path());
    let app = asset_router(state);

    let slug = "files/doc.pdf";
    let pdf = b"%PDF-1.4 fake";
    let (status, _body) = post_bytes(
        &app,
        &format!("/api/assets/{slug}"),
        pdf.to_vec(),
        "application/pdf",
        &cookie,
        &csrf,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let req = Request::builder()
        .uri(format!("/assets/{slug}"))
        .header(header::COOKIE, format!("zetl_session={cookie}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let cc = resp
        .headers()
        .get("cache-control")
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    assert!(
        cc.contains("immutable") || cc.contains("max-age"),
        "cache-control should be set: {cc}"
    );
}

#[tokio::test]
async fn asset_storage_counter_under_concurrency() {
    let tmp = tempfile::tempdir().unwrap();
    let (state, cookie, csrf) = build_collab_state(tmp.path());
    let app = asset_router(state);

    let mut handles = vec![];
    for i in 0..20 {
        let app = app.clone();
        let cookie = cookie.clone();
        let csrf = csrf.clone();
        let handle = tokio::spawn(async move {
            let slug = format!("concurrent/file{i}.txt");
            let body = vec![b'x'; 100];
            let req = Request::builder()
                .method("POST")
                .uri(format!("/api/assets/{slug}"))
                .header(header::CONTENT_TYPE, "text/plain")
                .header(header::COOKIE, format!("zetl_session={cookie}"))
                .header("X-CSRF-Token", &csrf)
                .header("X-Create", "true")
                .body(Body::from(body))
                .unwrap();
            let resp = app.oneshot(req).await.unwrap();
            resp.status()
        });
        handles.push(handle);
    }

    let mut created = 0;
    for h in handles {
        if h.await.unwrap() == StatusCode::CREATED {
            created += 1;
        }
    }

    // All concurrent uploads should succeed
    assert_eq!(created, 20, "all concurrent uploads should succeed");

    // Verify counter matches actual on-disk size
    let expected_total: u64 = (0..20).map(|_| 100u64).sum();
    let actual_total = zetl::assets::store::init_storage_total(tmp.path()).unwrap_or(0);
    assert_eq!(actual_total, expected_total, "storage counter should match actual size");
}
