use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderName, StatusCode};

/// Cookie name for the session token.
pub const SESSION_COOKIE_NAME: &str = "zetl_session";

/// Session is destroyed after this duration of inactivity.
const IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60); // 30 minutes

/// Session is destroyed unconditionally after this duration from creation.
const MAX_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60); // 24 hours

/// A single active session.
pub struct Session {
    pub user_id: String,
    pub created_at: Instant,
    pub last_accessed: Instant,
    /// Per-session CSRF token (REQ-020-064).
    pub csrf_token: String,
}

/// In-memory session store backed by a `HashMap`.
#[derive(Clone)]
pub struct SessionStore {
    sessions: Arc<RwLock<HashMap<String, Session>>>,
}

impl Default for SessionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionStore {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a new session for `user_id`, returning the opaque token.
    pub fn create(&self, user_id: &str) -> String {
        let token = generate_token();
        let csrf_token = generate_token();
        let now = Instant::now();
        let session = Session {
            user_id: user_id.to_string(),
            created_at: now,
            last_accessed: now,
            csrf_token,
        };
        self.sessions
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(token.clone(), session);
        token
    }

    /// Validate the token: check idle/max timeouts, touch `last_accessed`.
    /// Returns the `user_id` if valid.
    pub fn validate(&self, token: &str) -> Option<String> {
        let mut sessions = self.sessions.write().unwrap_or_else(|e| e.into_inner());
        let session = sessions.get_mut(token)?;
        let now = Instant::now();

        if now.duration_since(session.created_at) > MAX_TIMEOUT {
            sessions.remove(token);
            return None;
        }
        if now.duration_since(session.last_accessed) > IDLE_TIMEOUT {
            sessions.remove(token);
            return None;
        }

        session.last_accessed = now;
        Some(session.user_id.clone())
    }

    /// Destroy a session by token (logout).
    pub fn destroy(&self, token: &str) {
        self.sessions
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(token);
    }

    /// Return the CSRF token for a given session token, if the session is valid.
    pub fn csrf_token(&self, token: &str) -> Option<String> {
        let sessions = self.sessions.read().unwrap_or_else(|e| e.into_inner());
        sessions.get(token).map(|s| s.csrf_token.clone())
    }

    /// Count active (non-expired) sessions for a given user.
    pub fn active_session_count(&self, user_id: &str) -> usize {
        let sessions = self.sessions.read().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        sessions
            .values()
            .filter(|s| {
                s.user_id == user_id
                    && now.duration_since(s.created_at) <= MAX_TIMEOUT
                    && now.duration_since(s.last_accessed) <= IDLE_TIMEOUT
            })
            .count()
    }

    /// Remove all expired sessions (housekeeping).
    pub fn purge_expired(&self) {
        let now = Instant::now();
        self.sessions
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|_, s| {
                now.duration_since(s.created_at) <= MAX_TIMEOUT
                    && now.duration_since(s.last_accessed) <= IDLE_TIMEOUT
            });
    }
}

/// Build the `Set-Cookie` header value for a session token.
///
/// When `tls` is true, the `Secure` flag is included so the cookie is only
/// sent over HTTPS connections.
pub fn session_cookie(token: &str, tls: bool) -> String {
    let secure = if tls { "; Secure" } else { "" };
    format!("{SESSION_COOKIE_NAME}={token}; HttpOnly; SameSite=Strict; Path=/{secure}",)
}

/// Build a `Set-Cookie` header value that clears the session cookie.
pub fn clear_session_cookie() -> String {
    format!("{SESSION_COOKIE_NAME}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0",)
}

/// Generate an opaque session token (64 hex chars / 256 bits).
fn generate_token() -> String {
    // Use blake3 keyed hash of a UUID v4 to get 256-bit token
    let id = uuid::Uuid::new_v4();
    let hash = blake3::hash(id.as_bytes());
    hash.to_hex().to_string()
}

/// Parse the session token from the `Cookie` header.
pub fn token_from_cookies(headers: &axum::http::HeaderMap) -> Option<String> {
    let cookie_header = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    for part in cookie_header.split(';') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix(SESSION_COOKIE_NAME) {
            let value = value.strip_prefix('=')?;
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Extractor that yields the authenticated user's ID from the session cookie.
///
/// Returns `401 Unauthorized` if no valid session is present.
pub struct SessionUser {
    pub user_id: String,
    pub token: String,
}

impl<S> FromRequestParts<S> for SessionUser
where
    S: Send + Sync,
    super::WebState: FromRef<S>,
{
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let web_state = super::WebState::from_ref(state);
        let token = token_from_cookies(&parts.headers).ok_or(StatusCode::UNAUTHORIZED)?;
        let user_id = web_state
            .sessions
            .validate(&token)
            .ok_or(StatusCode::UNAUTHORIZED)?;
        Ok(SessionUser { user_id, token })
    }
}

/// Trait re-export so the extractor can pull `WebState` from composite state.
use axum::extract::FromRef;

use axum::body::Body;
use axum::extract::State;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

/// Extract a Bearer token from the `Authorization` header.
pub fn bearer_token_from_headers(headers: &axum::http::HeaderMap) -> Option<String> {
    let auth_header = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let token = auth_header.strip_prefix("Bearer ")?;
    if token.is_empty() {
        return None;
    }
    Some(token.to_string())
}

/// Verify a Bearer agent token against vault user profiles.
///
/// Returns the user_id if the token is valid, or `None` otherwise.
pub(crate) fn verify_bearer_token(vault_root: &std::path::Path, token_b64: &str) -> Option<String> {
    let user_id = crate::user::agent_token::extract_user_id(token_b64).ok()?;
    let profile = crate::user::load_profile(vault_root, &user_id).ok()??;
    crate::user::agent_token::verify_agent_token(
        token_b64,
        &profile.recovery_pubkey,
        profile.agent_token_generation,
    )
    .ok()
    .map(|claims| claims.user_id)
}

/// Middleware that gates routes behind session authentication when `--collab` is active.
///
/// In non-collab mode, all requests pass through unchanged.
/// In collab mode, accepts either a valid session cookie or a valid Bearer agent token.
/// Unauthenticated requests receive 401 Unauthorized.
pub async fn collab_gate(
    State(state): State<super::WebState>,
    request: axum::http::Request<Body>,
    next: Next,
) -> Response {
    if !state.collab {
        return next.run(request).await;
    }

    // Check for valid session cookie
    let has_session = token_from_cookies(request.headers())
        .and_then(|t| state.sessions.validate(&t))
        .is_some();

    if has_session {
        return next.run(request).await;
    }

    // Check for valid Bearer agent token
    let has_bearer = bearer_token_from_headers(request.headers())
        .and_then(|t| verify_bearer_token(&state.vault_root, &t))
        .is_some();

    if has_bearer {
        return next.run(request).await;
    }

    // Browsers get a redirect to the bootstrap/login page; API clients get 401.
    let accepts_html = request
        .headers()
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("text/html"));

    if accepts_html {
        axum::response::Redirect::temporary("/auth/login").into_response()
    } else {
        StatusCode::UNAUTHORIZED.into_response()
    }
}

/// Middleware that injects the `X-CSRF-Token` response header for authenticated sessions
/// (REQ-020-064). Applied to page-serving routes so the client can read the token.
pub async fn csrf_token_header(
    State(state): State<super::WebState>,
    request: axum::http::Request<Body>,
    next: Next,
) -> Response {
    let csrf = token_from_cookies(request.headers()).and_then(|t| state.sessions.csrf_token(&t));

    let mut response = next.run(request).await;

    if let Some(csrf) = csrf {
        response.headers_mut().insert(
            HeaderName::from_static("x-csrf-token"),
            axum::http::HeaderValue::from_str(&csrf).unwrap(),
        );
        // Also set a non-HttpOnly cookie so page.html inline JS can read it
        if let Ok(cookie_val) =
            axum::http::HeaderValue::from_str(&format!("zetl_csrf={csrf}; Path=/; SameSite=Strict"))
        {
            response
                .headers_mut()
                .append(axum::http::header::SET_COOKIE, cookie_val);
        }
    }

    response
}

/// Middleware that enforces CSRF token validation on state-changing requests (REQ-020-064).
///
/// PUT, POST, and DELETE requests must include a valid `X-CSRF-Token` header matching
/// the token issued with the session. Requests authenticated via Bearer token are exempt
/// (agent tokens are not automatically attached by browsers). WebSocket upgrade requests
/// are also exempt.
pub async fn csrf_guard(
    State(state): State<super::WebState>,
    request: axum::http::Request<Body>,
    next: Next,
) -> Response {
    let method = request.method().clone();

    // Skip CSRF checks in non-collab mode — there are no sessions.
    if !state.collab {
        return next.run(request).await;
    }

    // Only check state-changing methods
    if method != axum::http::Method::PUT
        && method != axum::http::Method::POST
        && method != axum::http::Method::DELETE
    {
        return next.run(request).await;
    }

    // Exempt Bearer-authenticated requests
    if bearer_token_from_headers(request.headers()).is_some() {
        return next.run(request).await;
    }

    // Exempt WebSocket upgrade requests
    if request
        .headers()
        .get(axum::http::header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false)
    {
        return next.run(request).await;
    }

    // Require valid session + matching CSRF token
    let session_token = match token_from_cookies(request.headers()) {
        Some(t) => t,
        None => return StatusCode::FORBIDDEN.into_response(),
    };

    let expected_csrf = match state.sessions.csrf_token(&session_token) {
        Some(t) => t,
        None => return StatusCode::FORBIDDEN.into_response(),
    };

    let provided_csrf = request
        .headers()
        .get("X-CSRF-Token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !crate::user::constant_time_eq(provided_csrf.as_bytes(), expected_csrf.as_bytes()) {
        return StatusCode::FORBIDDEN.into_response();
    }

    next.run(request).await
}

/// Extractor that yields the authenticated user's role from the session cookie.
///
/// Loads the user profile and computes the role. Returns 401 if no session,
/// 403 if the user profile cannot be found.
pub struct SessionRole {
    pub user_id: String,
    pub role: crate::user::Role,
}

impl<S> FromRequestParts<S> for SessionRole
where
    S: Send + Sync,
    super::WebState: FromRef<S>,
{
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let web_state = super::WebState::from_ref(state);
        let token = token_from_cookies(&parts.headers).ok_or(StatusCode::UNAUTHORIZED)?;
        let user_id = web_state
            .sessions
            .validate(&token)
            .ok_or(StatusCode::UNAUTHORIZED)?;

        let profile = crate::user::load_profile(&web_state.vault_root, &user_id)
            .ok()
            .flatten()
            .ok_or(StatusCode::FORBIDDEN)?;

        let role = crate::user::Role::for_profile_with_vault(&profile, &web_state.vault_root);
        Ok(SessionRole { user_id, role })
    }
}

/// Extractor that yields the authenticated user's ID from a Bearer agent token.
///
/// Returns `401 Unauthorized` if no valid Bearer token is present.
pub struct BearerUser {
    pub user_id: String,
}

impl<S> FromRequestParts<S> for BearerUser
where
    S: Send + Sync,
    super::WebState: FromRef<S>,
{
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let web_state = super::WebState::from_ref(state);
        let token = bearer_token_from_headers(&parts.headers).ok_or(StatusCode::UNAUTHORIZED)?;
        let user_id =
            verify_bearer_token(&web_state.vault_root, &token).ok_or(StatusCode::UNAUTHORIZED)?;
        Ok(BearerUser { user_id })
    }
}

/// Extractor that accepts either a session cookie or a Bearer agent token.
///
/// Tries session first, then Bearer. Returns `401 Unauthorized` if neither is valid.
/// The `is_agent` field indicates which authentication method succeeded.
pub struct AuthUser {
    pub user_id: String,
    pub is_agent: bool,
}

impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
    super::WebState: FromRef<S>,
{
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let web_state = super::WebState::from_ref(state);

        // Try session cookie first
        if let Some(token) = token_from_cookies(&parts.headers) {
            if let Some(user_id) = web_state.sessions.validate(&token) {
                return Ok(AuthUser {
                    user_id,
                    is_agent: false,
                });
            }
        }

        // Try Bearer token
        if let Some(token) = bearer_token_from_headers(&parts.headers) {
            if let Some(user_id) = verify_bearer_token(&web_state.vault_root, &token) {
                return Ok(AuthUser {
                    user_id,
                    is_agent: true,
                });
            }
        }

        Err(StatusCode::UNAUTHORIZED)
    }
}

/// Middleware that gates `/_admin/*` routes behind a hardcoded owner-or-admin check.
///
/// This runs *before* any SPL policy evaluation and cannot be overridden by access rules.
/// Requires the user to be authenticated (session cookie or Bearer token) and their
/// profile to have `owner == true` (which maps to `Role::Admin` via `Role::for_profile`).
///
/// Returns 401 if unauthenticated, 403 if the user is not an owner/admin.
pub async fn admin_gate(
    State(state): State<super::WebState>,
    request: axum::http::Request<Body>,
    next: Next,
) -> Response {
    // Resolve user_id from session cookie or Bearer token
    let user_id = token_from_cookies(request.headers())
        .and_then(|t| state.sessions.validate(&t))
        .or_else(|| {
            bearer_token_from_headers(request.headers())
                .and_then(|t| verify_bearer_token(&state.vault_root, &t))
        });

    let user_id = match user_id {
        Some(id) => id,
        None => return StatusCode::UNAUTHORIZED.into_response(),
    };

    // Load profile and check owner/admin — hardcoded, not SPL-driven
    let profile = crate::user::load_profile(&state.vault_root, &user_id)
        .ok()
        .flatten();

    match profile {
        Some(p)
            if p.owner
                || crate::user::Role::for_profile_with_vault(&p, &state.vault_root)
                    >= crate::user::Role::Admin =>
        {
            next.run(request).await
        }
        _ => StatusCode::FORBIDDEN.into_response(),
    }
}

/// Check that a session role meets the minimum required level.
///
/// Returns `Err(403 Forbidden)` if the role is insufficient.
pub fn require_role(
    actual: crate::user::Role,
    minimum: crate::user::Role,
) -> Result<(), StatusCode> {
    if actual >= minimum {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_validate() {
        let store = SessionStore::new();
        let token = store.create("alice");
        assert_eq!(store.validate(&token), Some("alice".to_string()));
    }

    #[test]
    fn invalid_token_returns_none() {
        let store = SessionStore::new();
        assert_eq!(store.validate("bogus"), None);
    }

    #[test]
    fn destroy_invalidates() {
        let store = SessionStore::new();
        let token = store.create("bob");
        store.destroy(&token);
        assert_eq!(store.validate(&token), None);
    }

    #[test]
    fn token_is_64_hex_chars() {
        let t = generate_token();
        assert_eq!(t.len(), 64);
        assert!(t.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn cookie_format() {
        let c = session_cookie("abc123", false);
        assert!(c.contains("HttpOnly"));
        assert!(c.contains("SameSite=Strict"));
        assert!(c.contains("Path=/"));
        assert!(c.contains("zetl_session=abc123"));
        assert!(!c.contains("Secure"));
    }

    #[test]
    fn cookie_format_tls() {
        let c = session_cookie("abc123", true);
        assert!(c.contains("HttpOnly"));
        assert!(c.contains("SameSite=Strict"));
        assert!(c.contains("Secure"));
        assert!(c.contains("zetl_session=abc123"));
    }

    #[test]
    fn clear_cookie_format() {
        let c = clear_session_cookie();
        assert!(c.contains("Max-Age=0"));
    }

    #[test]
    fn parse_token_from_cookie_header() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            "foo=bar; zetl_session=deadbeef; baz=qux".parse().unwrap(),
        );
        assert_eq!(token_from_cookies(&headers), Some("deadbeef".to_string()));
    }

    #[test]
    fn parse_token_missing() {
        let headers = axum::http::HeaderMap::new();
        assert_eq!(token_from_cookies(&headers), None);
    }

    #[test]
    fn purge_expired_keeps_valid() {
        let store = SessionStore::new();
        let token = store.create("eve");
        store.purge_expired();
        assert_eq!(store.validate(&token), Some("eve".to_string()));
    }

    #[test]
    fn require_role_admin_passes_all() {
        use crate::user::Role;
        assert!(require_role(Role::Admin, Role::Reader).is_ok());
        assert!(require_role(Role::Admin, Role::Editor).is_ok());
        assert!(require_role(Role::Admin, Role::Admin).is_ok());
    }

    #[test]
    fn require_role_editor_blocked_from_admin() {
        use crate::user::Role;
        assert!(require_role(Role::Editor, Role::Reader).is_ok());
        assert!(require_role(Role::Editor, Role::Editor).is_ok());
        assert!(require_role(Role::Editor, Role::Admin).is_err());
    }

    #[test]
    fn require_role_reader_blocked_from_editor_and_admin() {
        use crate::user::Role;
        assert!(require_role(Role::Reader, Role::Reader).is_ok());
        assert!(require_role(Role::Reader, Role::Editor).is_err());
        assert!(require_role(Role::Reader, Role::Admin).is_err());
    }

    #[test]
    fn parse_bearer_token_from_auth_header() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer my-token-123".parse().unwrap(),
        );
        assert_eq!(
            bearer_token_from_headers(&headers),
            Some("my-token-123".to_string())
        );
    }

    #[test]
    fn bearer_token_missing_returns_none() {
        let headers = axum::http::HeaderMap::new();
        assert_eq!(bearer_token_from_headers(&headers), None);
    }

    #[test]
    fn bearer_token_wrong_scheme_returns_none() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Basic abc123".parse().unwrap(),
        );
        assert_eq!(bearer_token_from_headers(&headers), None);
    }

    #[test]
    fn bearer_token_empty_value_returns_none() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer ".parse().unwrap(),
        );
        assert_eq!(bearer_token_from_headers(&headers), None);
    }

    #[test]
    fn verify_bearer_with_real_profile() {
        let tmp = tempfile::TempDir::new().unwrap();
        let kp = crate::user::recovery::generate_recovery_keypair().unwrap();
        let profile = crate::user::UserProfile {
            id: "agent-test-a1b2c3d4".to_string(),
            name: "Agent Test".to_string(),
            created_at: "2026-03-18T10:00:00Z".to_string(),
            invited_by: None,
            owner: true,
            credentials: vec![],
            recovery_pubkey: kp.recovery_pubkey.clone(),
            agent_token_generation: 0,
        };
        crate::user::save_profile(tmp.path(), &profile).unwrap();

        let token =
            crate::user::agent_token::generate_agent_token(&kp.mnemonic, &profile.id, 0).unwrap();

        let result = verify_bearer_token(tmp.path(), &token);
        assert_eq!(result, Some("agent-test-a1b2c3d4".to_string()));
    }

    #[test]
    fn verify_bearer_wrong_generation_fails() {
        let tmp = tempfile::TempDir::new().unwrap();
        let kp = crate::user::recovery::generate_recovery_keypair().unwrap();
        let profile = crate::user::UserProfile {
            id: "agent-test-a1b2c3d4".to_string(),
            name: "Agent Test".to_string(),
            created_at: "2026-03-18T10:00:00Z".to_string(),
            invited_by: None,
            owner: true,
            credentials: vec![],
            recovery_pubkey: kp.recovery_pubkey.clone(),
            agent_token_generation: 1, // profile is at generation 1
        };
        crate::user::save_profile(tmp.path(), &profile).unwrap();

        // Generate token at generation 0
        let token =
            crate::user::agent_token::generate_agent_token(&kp.mnemonic, &profile.id, 0).unwrap();

        let result = verify_bearer_token(tmp.path(), &token);
        assert_eq!(result, None); // Should fail: generation mismatch
    }

    #[test]
    fn csrf_token_issued_with_session() {
        let store = SessionStore::new();
        let token = store.create("alice");
        let csrf = store.csrf_token(&token);
        assert!(csrf.is_some());
        let csrf = csrf.unwrap();
        assert_eq!(csrf.len(), 64);
        assert!(csrf.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn csrf_token_stable_for_session() {
        let store = SessionStore::new();
        let token = store.create("alice");
        let csrf1 = store.csrf_token(&token).unwrap();
        let csrf2 = store.csrf_token(&token).unwrap();
        assert_eq!(csrf1, csrf2);
    }

    #[test]
    fn csrf_token_differs_between_sessions() {
        let store = SessionStore::new();
        let t1 = store.create("alice");
        let t2 = store.create("alice");
        let csrf1 = store.csrf_token(&t1).unwrap();
        let csrf2 = store.csrf_token(&t2).unwrap();
        assert_ne!(csrf1, csrf2);
    }

    #[test]
    fn csrf_token_invalid_session_returns_none() {
        let store = SessionStore::new();
        assert!(store.csrf_token("bogus").is_none());
    }

    #[test]
    fn csrf_token_destroyed_session_returns_none() {
        let store = SessionStore::new();
        let token = store.create("alice");
        store.destroy(&token);
        assert!(store.csrf_token(&token).is_none());
    }

    /// Helper to build a minimal WebState backed by a temp dir for admin_gate tests.
    fn test_web_state(tmp: &std::path::Path) -> crate::web::WebState {
        use std::sync::{Arc, Mutex, RwLock};
        let data = crate::web::VaultData {
            files: vec![],
            graph: crate::graph::LinkGraph::build(&[], &std::collections::HashMap::new()),
            page_names: vec![],
            resolved: std::collections::HashSet::new(),
            page_slug_map: std::collections::HashMap::new(),
            page_slug_map_lower: std::collections::HashMap::new(),
            collision_names: std::collections::HashSet::new(),
        };
        let search_index = crate::search_index::SearchIndex::build(tmp, &[]).unwrap();
        crate::web::WebState {
            data: Arc::new(RwLock::new(data)),
            vault_root: Arc::new(tmp.to_path_buf()),
            search_index: Arc::new(search_index),
            engine: Arc::new(crate::web::engine::TemplateEngine::new(
                tmp, "default", false, false,
            )),
            theme: "default".to_string(),
            verbose: false,
            collab: true,
            tls: false,
            trust_proxy: false,
            sessions: SessionStore::new(),
            recovery_challenges: Arc::new(crate::user::recovery::RecoveryChallengeStore::new()),
            mnemonic_shown: Arc::new(Mutex::new(std::collections::HashSet::new())),
            bootstrap_used: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            rate_limiters: crate::web::rate_limit::AuthRateLimiters::new(),
            #[cfg(feature = "reason")]
            acl_cache: Arc::new(Mutex::new(crate::web::AclCache::new())),
            git_commit_lock: None,
            ws_hub: crate::web::ws::WsHub::new(),
            ticket_store: crate::web::ws::TicketStore::new(),
            crdt_store: crate::web::ws::CrdtDocStore::new(Arc::new(tmp.to_path_buf())),
            wal_store: std::sync::Arc::new(crate::web::wal::WalStore::new(tmp)),
            pending_writes: crate::web::fs_watch::PendingWrites::new(),
            passkey_mgr: None,
            public_dir: None,
            scan_options: crate::scanner::ScanOptions::default(),
            asset_storage: crate::assets::store::StorageCounterGuard::new(0),
            asset_max_file_bytes: 10 * 1024 * 1024,
            asset_max_total_bytes: 100 * 1024 * 1024,
        }
    }

    /// Create a user profile in the temp vault and return its ID.
    fn create_test_user(tmp: &std::path::Path, name: &str, owner: bool) -> String {
        let id = crate::user::generate_user_id(name);
        let profile = crate::user::UserProfile {
            id: id.clone(),
            name: name.to_string(),
            created_at: "2026-03-18T10:00:00Z".to_string(),
            invited_by: None,
            owner,
            credentials: vec![],
            recovery_pubkey: "dGVzdC1wdWJrZXk".to_string(),
            agent_token_generation: 0,
        };
        crate::user::save_profile(tmp, &profile).unwrap();
        id
    }

    #[tokio::test]
    async fn admin_gate_allows_owner() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use axum::{middleware, routing::get, Router};
        use tower::ServiceExt;

        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_web_state(tmp.path());
        let user_id = create_test_user(tmp.path(), "Owner", true);
        let session_token = state.sessions.create(&user_id);

        let app = Router::new()
            .route("/test", get(|| async { "ok" }))
            .route_layer(middleware::from_fn_with_state(state.clone(), admin_gate))
            .with_state(state);

        let resp = app
            .oneshot(
                Request::get("/test")
                    .header("Cookie", format!("zetl_session={session_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn admin_gate_blocks_non_owner() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use axum::{middleware, routing::get, Router};
        use tower::ServiceExt;

        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_web_state(tmp.path());
        let user_id = create_test_user(tmp.path(), "Editor", false);
        let session_token = state.sessions.create(&user_id);

        let app = Router::new()
            .route("/test", get(|| async { "ok" }))
            .route_layer(middleware::from_fn_with_state(state.clone(), admin_gate))
            .with_state(state);

        let resp = app
            .oneshot(
                Request::get("/test")
                    .header("Cookie", format!("zetl_session={session_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn admin_gate_returns_401_without_session() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use axum::{middleware, routing::get, Router};
        use tower::ServiceExt;

        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_web_state(tmp.path());

        let app = Router::new()
            .route("/test", get(|| async { "ok" }))
            .route_layer(middleware::from_fn_with_state(state.clone(), admin_gate))
            .with_state(state);

        let resp = app
            .oneshot(Request::get("/test").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
