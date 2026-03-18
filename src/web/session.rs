use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;

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
}

/// In-memory session store backed by a `HashMap`.
#[derive(Clone)]
pub struct SessionStore {
    sessions: Arc<RwLock<HashMap<String, Session>>>,
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
        let now = Instant::now();
        let session = Session {
            user_id: user_id.to_string(),
            created_at: now,
            last_accessed: now,
        };
        self.sessions
            .write()
            .expect("session lock poisoned")
            .insert(token.clone(), session);
        token
    }

    /// Validate the token: check idle/max timeouts, touch `last_accessed`.
    /// Returns the `user_id` if valid.
    pub fn validate(&self, token: &str) -> Option<String> {
        let mut sessions = self.sessions.write().expect("session lock poisoned");
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
            .expect("session lock poisoned")
            .remove(token);
    }

    /// Remove all expired sessions (housekeeping).
    pub fn purge_expired(&self) {
        let now = Instant::now();
        self.sessions
            .write()
            .expect("session lock poisoned")
            .retain(|_, s| {
                now.duration_since(s.created_at) <= MAX_TIMEOUT
                    && now.duration_since(s.last_accessed) <= IDLE_TIMEOUT
            });
    }
}

/// Build the `Set-Cookie` header value for a session token.
pub fn session_cookie(token: &str) -> String {
    format!(
        "{SESSION_COOKIE_NAME}={token}; HttpOnly; SameSite=Strict; Path=/",
    )
}

/// Build a `Set-Cookie` header value that clears the session cookie.
pub fn clear_session_cookie() -> String {
    format!(
        "{SESSION_COOKIE_NAME}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0",
    )
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
        let c = session_cookie("abc123");
        assert!(c.contains("HttpOnly"));
        assert!(c.contains("SameSite=Strict"));
        assert!(c.contains("Path=/"));
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
        assert_eq!(
            token_from_cookies(&headers),
            Some("deadbeef".to_string())
        );
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
}
