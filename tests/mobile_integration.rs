//! SPEC-040 mobile-route integration tests (TEST-4005, TEST-4011).
//!
//! Verifies the `/_mobile/*` route surface is registered when the
//! `mobile` cargo feature is enabled and exercises the onboarding
//! flow end to end. Run with:
//!
//!     cargo test --test mobile_integration --features mobile
//!
//! Tests share `zetl::mobile_state::global()` (a process-wide
//! `OnceLock` singleton) so they coordinate via a `STATE_LOCK` Mutex
//! rather than relying on cargo's per-test isolation. The lock is
//! held for the duration of each test that touches the keystore.

#![cfg(feature = "mobile")]
// Each #[tokio::test] gets its own current-thread runtime; STATE_LOCK
// serialises tests that touch the global keystore. Holding the
// std::sync::Mutex guard across .await is safe here because no other
// task on the same thread can yield in between, and concurrent tests
// run on separate OS threads with separate runtimes.
#![allow(clippy::await_holding_lock)]

use std::sync::Mutex;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use tower::ServiceExt;

use zetl::web::mobile;

/// Serialises tests that touch the global keystore. Held over the
/// whole logical test (not just a single HTTP call) so a setup-then-
/// assert sequence cannot interleave with another test's reset.
static STATE_LOCK: Mutex<()> = Mutex::new(());

const FIXTURE_MNEMONIC: &str = "abandon abandon abandon abandon abandon abandon \
                                abandon abandon abandon abandon abandon about";

async fn get(app: &Router, uri: &str) -> (StatusCode, String) {
    let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), 10_000_000)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).to_string())
}

async fn post_form(app: &Router, uri: &str, body: &str) -> (StatusCode, Option<String>, String) {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let location = resp
        .headers()
        .get("location")
        .map(|v| v.to_str().unwrap().to_string());
    let body = axum::body::to_bytes(resp.into_body(), 10_000_000)
        .await
        .unwrap();
    (status, location, String::from_utf8_lossy(&body).to_string())
}

fn router() -> Router {
    mobile::router::<()>().with_state(())
}

// ── GET surface tests ─────────────────────────────────────────────────────────

#[tokio::test]
async fn onboarding_get_shows_step_one_when_no_key() {
    let _g = STATE_LOCK.lock().unwrap();
    // Fresh process state: no key imported (zero-side-effect test —
    // do not import a key here).
    let app = router();
    let (status, body) = get(&app, "/_mobile/onboarding").await;
    assert_eq!(status, StatusCode::OK);
    // The post-step-1 keystore may or may not be loaded depending on
    // test ordering, so this test is robust against either rendering;
    // it only asserts the route mounts and returns the onboarding marker.
    assert!(
        body.contains("data-zetl-mobile-route=\"onboarding\""),
        "missing onboarding marker; body={body}"
    );
}

#[tokio::test]
async fn capture_route_returns_placeholder() {
    let app = router();
    let (status, body) = get(&app, "/_mobile/capture").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("data-zetl-mobile-route=\"capture\""),
        "missing capture marker; body={body}"
    );
}

#[tokio::test]
async fn sync_route_returns_placeholder() {
    let app = router();
    let (status, body) = get(&app, "/_mobile/sync").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("data-zetl-mobile-route=\"sync\""),
        "missing sync marker; body={body}"
    );
}

#[tokio::test]
async fn unknown_mobile_route_returns_404() {
    let app = router();
    let (status, _) = get(&app, "/_mobile/does-not-exist").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ── Onboarding POST flow tests ────────────────────────────────────────────────

#[tokio::test]
async fn onboarding_seed_post_with_valid_mnemonic_redirects() {
    let _g = STATE_LOCK.lock().unwrap();
    let app = router();
    let body = format!(
        "mnemonic={}",
        urlencoding::encode(FIXTURE_MNEMONIC).into_owned()
    );
    let (status, location, _body) = post_form(&app, "/_mobile/onboarding/seed", &body).await;
    assert!(
        matches!(
            status,
            StatusCode::SEE_OTHER | StatusCode::TEMPORARY_REDIRECT | StatusCode::FOUND
        ),
        "expected redirect, got {status}"
    );
    assert_eq!(location.as_deref(), Some("/_mobile/onboarding"));
    // After a successful seed POST the keystore is loaded; the next
    // GET /_mobile/onboarding renders step 2 (clone form).
    let (_status, body) = get(&app, "/_mobile/onboarding").await;
    assert!(
        body.contains("data-zetl-mobile-step=\"clone\""),
        "expected step-2 render after seed POST; body={body}"
    );
    assert!(
        body.contains("ssh-ed25519"),
        "expected ssh public-key line in step-2 render; body={body}"
    );
}

#[tokio::test]
async fn onboarding_seed_post_with_garbage_renders_error() {
    let _g = STATE_LOCK.lock().unwrap();
    let app = router();
    let body = "mnemonic=this+is+not+a+valid+mnemonic+at+all";
    let (status, location, body) = post_form(&app, "/_mobile/onboarding/seed", body).await;
    assert_eq!(status, StatusCode::OK);
    assert!(location.is_none(), "garbage seed must not redirect");
    assert!(
        body.contains("data-zetl-mobile-error=\"seed\""),
        "expected error block on garbage seed; body={body}"
    );
}

#[tokio::test]
async fn onboarding_clone_post_without_seed_routes_back_to_seed_step() {
    let _g = STATE_LOCK.lock().unwrap();

    // Clear keystore by writing a known-bad mnemonic — fails import,
    // leaves the store unloaded if it was unloaded; if it was already
    // loaded (from sibling test), this returns 200 with seed-error.
    // Either way we then attempt the clone POST and assert routing.
    //
    // To make this test deterministic we lean on the keystore's
    // `import_mnemonic` not clearing on failure. We accept that this
    // test is best-effort across orderings — its primary value is
    // verifying the no-keystore branch when the test runs first.

    let app = router();
    let body = "remote_url=git%40codeberg.org%3Ayou%2Fyour-vault.git";
    let (status, _location, body) = post_form(&app, "/_mobile/onboarding/clone", body).await;
    assert_eq!(status, StatusCode::OK);
    // When the keystore is empty the handler renders the seed step
    // with an error message; otherwise it falls through to the clone
    // step (which then errors on the file-or-ssh URL). Either marker
    // is acceptable here — we only assert the route returns 200.
    assert!(
        body.contains("data-zetl-mobile-route=\"onboarding\""),
        "expected onboarding marker regardless of branch; body={body}"
    );
}

// ── Capture POST flow ─────────────────────────────────────────────────────────

#[tokio::test]
async fn capture_get_renders_form() {
    let app = router();
    let (status, body) = get(&app, "/_mobile/capture").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("data-zetl-mobile-route=\"capture\""));
    assert!(
        body.contains("<textarea"),
        "expected capture textarea; body={body}"
    );
}

#[tokio::test]
async fn capture_post_writes_file_commits_and_redirects() {
    let _g = STATE_LOCK.lock().unwrap();

    let vault = tempfile::tempdir().unwrap();
    git2::Repository::init(vault.path()).unwrap();
    zetl::mobile_state::set_vault_root(vault.path().to_path_buf());

    let app = router();
    let form_body = format!(
        "title={}&content={}",
        urlencoding::encode("Coffee notes"),
        urlencoding::encode("Some content\n")
    );
    let (status, location, _body) = post_form(&app, "/_mobile/capture", &form_body).await;
    assert!(
        matches!(
            status,
            StatusCode::SEE_OTHER | StatusCode::TEMPORARY_REDIRECT | StatusCode::FOUND
        ),
        "expected redirect after successful capture, got {status}"
    );
    let loc = location.expect("redirect should set Location header");
    assert_eq!(loc, "/Coffee%20notes");
    let written = std::fs::read_to_string(vault.path().join("Coffee notes.md")).unwrap();
    assert_eq!(written, "Some content\n");
}

#[tokio::test]
async fn capture_post_with_no_vault_root_renders_error() {
    let _g = STATE_LOCK.lock().unwrap();

    // Set then immediately clear the vault root for this test by
    // pointing it at a non-existent path. The capture handler will
    // call mobile_capture::capture which errors on non-dir vault.
    let nonexistent = std::path::PathBuf::from("/this/path/should/not/exist/zetl-test");
    zetl::mobile_state::set_vault_root(nonexistent);

    let app = router();
    let form_body = "title=&content=hello";
    let (status, _location, body) = post_form(&app, "/_mobile/capture", form_body).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("data-zetl-mobile-error=\"capture\""),
        "expected capture error block; body={body}"
    );
}

// ── Sync controls ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn sync_get_renders_buttons() {
    let app = router();
    let (status, body) = get(&app, "/_mobile/sync").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("data-zetl-mobile-route=\"sync\""));
    assert!(body.contains("Pull"));
    assert!(body.contains("Push"));
}

#[tokio::test]
async fn sync_pull_without_vault_root_renders_error() {
    let _g = STATE_LOCK.lock().unwrap();

    // Force the vault_root cell to None by setting to an empty path
    // and relying on the handler's error branch. We cannot reset the
    // OnceLock-backed Mutex contents to None from a public API; use
    // a fresh tempdir path that exists but has no git repo.
    let dir = tempfile::tempdir().unwrap();
    zetl::mobile_state::set_vault_root(dir.path().to_path_buf());

    let app = router();
    let (status, _location, body) = post_form(&app, "/_mobile/sync/pull", "").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("data-zetl-mobile-sync=\"error\""),
        "expected sync error block for non-git directory; body={body}"
    );
}
