//! SPEC-040 mobile-specific serve routes (REQ-4005, CON-4004).
//!
//! Gated behind `--features mobile`. These routes provide the
//! capture / onboarding / sync surfaces consumed by the Tauri Mobile
//! shell. When the feature is off (the default for desktop serve),
//! the entire module is excluded from the build and no `/_mobile/*`
//! routes are registered.
//!
//! v0.1 ships placeholder bodies for the three GET surfaces so the
//! routing mechanism is end-to-end testable. Subsequent slices will:
//! 1. Replace placeholders with proper Minijinja templates loaded
//!    through the existing engine + theme contract.
//! 2. Add `POST /_mobile/capture`, `POST /_mobile/sync/pull`, and
//!    `POST /_mobile/sync/push` once the `git` and `keys` modules
//!    land (REQ-4006, REQ-4009, REQ-4010, REQ-4011).
//! 3. Wire share-extension inbox draining via `drain_share_inbox()`
//!    Tauri command (REQ-4007, CON-4003).
//!
//! The router is generic over the state type so it merges cleanly
//! into the main `Router<WebState>` and is also testable in isolation
//! against `Router<()>`. Handlers do not extract state because the
//! placeholders have no need for it; that changes when the real
//! template-rendering / git-driving handlers land.

use axum::{
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};

/// Build the `/_mobile/*` router. Generic over state so callers
/// (the main router or unit tests) decide the parameterisation.
pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/_mobile/onboarding", get(onboarding_handler))
        .route("/_mobile/capture", get(capture_handler))
        .route("/_mobile/sync", get(sync_handler))
}

/// `GET /_mobile/onboarding` — guided seed-import + remote-URL + clone wizard.
///
/// Placeholder. Real wizard arrives once `keys` (BIP39 → ed25519 →
/// keychain) and `git` (clone via SSH credential cb) modules ship.
async fn onboarding_handler() -> Response {
    Html(MOBILE_PLACEHOLDER_ONBOARDING).into_response()
}

/// `GET /_mobile/capture` — quick-capture form (FAB target + share-extension landing).
///
/// Placeholder. Real form binds to `POST /_mobile/capture` and supports
/// prefill from query string / share-extension inbox.
async fn capture_handler() -> Response {
    Html(MOBILE_PLACEHOLDER_CAPTURE).into_response()
}

/// `GET /_mobile/sync` — sync status + manual pull/push controls.
///
/// Placeholder. Real view shows ahead/behind counts, last-synced
/// timestamps, and pull/push buttons that POST to
/// `/_mobile/sync/{pull,push}`.
async fn sync_handler() -> Response {
    Html(MOBILE_PLACEHOLDER_SYNC).into_response()
}

const MOBILE_PLACEHOLDER_ONBOARDING: &str = "\
<!DOCTYPE html><html><head><meta charset=\"utf-8\">\
<title>zetl mobile · onboarding</title></head>\
<body data-zetl-mobile-route=\"onboarding\">\
<h1>zetl mobile · onboarding</h1>\
<p>Placeholder. Wizard lands once keys + git modules ship.</p>\
</body></html>";

const MOBILE_PLACEHOLDER_CAPTURE: &str = "\
<!DOCTYPE html><html><head><meta charset=\"utf-8\">\
<title>zetl mobile · capture</title></head>\
<body data-zetl-mobile-route=\"capture\">\
<h1>zetl mobile · capture</h1>\
<p>Placeholder. Capture form lands with POST /_mobile/capture.</p>\
</body></html>";

const MOBILE_PLACEHOLDER_SYNC: &str = "\
<!DOCTYPE html><html><head><meta charset=\"utf-8\">\
<title>zetl mobile · sync</title></head>\
<body data-zetl-mobile-route=\"sync\">\
<h1>zetl mobile · sync</h1>\
<p>Placeholder. Sync status + pull/push controls land with git module.</p>\
</body></html>";
