//! SPEC-040 mobile-route integration tests (TEST-4005).
//!
//! Verifies the `/_mobile/*` route surface is registered when the
//! `mobile` cargo feature is enabled and absent otherwise. Run with:
//!
//!     cargo test --test mobile_integration --features mobile
//!
//! When the feature is off, the file still compiles (to satisfy the
//! cargo test discovery) but the meaningful tests are gated behind
//! the same feature.

#![cfg(feature = "mobile")]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use tower::ServiceExt;

use zetl::web::mobile;

async fn get(app: &Router, uri: &str) -> (StatusCode, String) {
    let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), 10_000_000)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).to_string())
}

fn router() -> Router {
    // The mobile module's router is generic over WebState; its handlers
    // ignore state, so we can finalise it with `()` for unit-style
    // testing without needing the full WebState construction harness.
    mobile::router::<()>().with_state(())
}

#[tokio::test]
async fn onboarding_route_returns_placeholder() {
    let app = router();
    let (status, body) = get(&app, "/_mobile/onboarding").await;
    assert_eq!(status, StatusCode::OK);
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
