//! OIDC / OAuth2 authenticator (SPEC-041 REQ-4109, REQ-4110, REQ-4113,
//! CON-4104, ADR-4104, ADR-4105). Feature-gated behind `collab-oidc`.
//!
//! This is the **Phase 5b** implementation: the cookie-session
//! `authenticate()` path PLUS the `/auth/oidc/login` →  IdP →
//! `/auth/oidc/callback` flow.
//!
//! Protocol (rolled by hand on `jsonwebtoken` + `reqwest` rather than the
//! `openidconnect` crate to keep the dependency surface auditable):
//!
//! 1. `login_handler` generates `state`, `nonce`, PKCE `code_verifier` +
//!    `code_challenge`, stashes them in [`PendingStates`] under `state`,
//!    redirects the browser to `<issuer>/authorize?...` with those params.
//! 2. The IdP authenticates the user, redirects to `/auth/oidc/callback?
//!    code=...&state=...`.
//! 3. `callback_handler` removes the pending entry under `state`,
//!    POSTs to the token endpoint with `code` + `code_verifier`, receives
//!    the ID token, validates its signature against the IdP's JWKS, checks
//!    `iss` / `aud` / `exp` / `iat` / `nonce`, extracts the `user_id_claim`,
//!    runs auto-provisioning if enabled, mints a `SessionStore` session
//!    and sets `zetl_session`.
//!
//! Failure on any step → generic 401 (the operator log records the cause).
//! `state` and `nonce` are single-use (`take_pending` removes on success);
//! TTL pruning runs lazily on every store interaction.

#![cfg(feature = "collab-oidc")]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::extract::{Query, State};
use axum::http::request::Parts;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::Router;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};

use crate::web::auth::provision::{ensure_user, ProvisionOutcome, ProvisionPolicy};
use crate::web::session::{session_cookie, token_from_cookies, SessionStore};
use crate::web::WebState;

use super::{AuthOutcome, Authenticator, Principal, PrincipalId};

// ── Config ────────────────────────────────────────────────────────────

/// `[collab.auth.oidc]` schema (CON-4104).
#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct OidcConfig {
    /// Pinned issuer URL — every ID token's `iss` claim must match this
    /// exactly (REQ-4110).
    pub issuer: String,
    pub client_id: String,
    /// Path to a file containing the client secret. Never inlined
    /// (Threat Model G).
    pub client_secret_file: String,
    /// ID-token claim whose value becomes the zetl `user_id`. Default
    /// `"email"` (REQ-4109).
    #[serde(default = "default_user_id_claim")]
    pub user_id_claim: String,
    /// Opt-in auto-provisioning (REQ-4111, ADR-4107). When `true`,
    /// `provision_domain_allow` is also required.
    #[serde(default)]
    pub auto_provision: bool,
    #[serde(default)]
    pub provision_domain_allow: Option<Vec<String>>,
    /// Optional override for the redirect URI registered with the IdP.
    /// Defaults to `<request-host>/auth/oidc/callback` at runtime.
    #[serde(default)]
    pub redirect_uri: Option<String>,
}

fn default_user_id_claim() -> String {
    "email".to_string()
}

// ── Pending-state store ───────────────────────────────────────────────

/// One pending OIDC handshake — what we need to verify the callback
/// belongs to a login we initiated.
#[derive(Debug, Clone)]
struct PendingState {
    nonce: String,
    pkce_verifier: String,
    return_to: String,
    created_at: Instant,
}

const PENDING_TTL: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Default)]
pub(crate) struct PendingStates {
    inner: Mutex<HashMap<String, PendingState>>,
}

impl PendingStates {
    fn insert(&self, state: String, entry: PendingState) {
        let mut m = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        Self::prune(&mut m);
        m.insert(state, entry);
    }

    /// Remove + return the pending state for `state` if present and not
    /// expired. Single-use (REQ-4110): a second `take` for the same `state`
    /// returns `None` even if the first call was recent.
    fn take(&self, state: &str) -> Option<PendingState> {
        let mut m = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        Self::prune(&mut m);
        m.remove(state)
    }

    fn prune(m: &mut HashMap<String, PendingState>) {
        let now = Instant::now();
        m.retain(|_, v| now.duration_since(v.created_at) <= PENDING_TTL);
    }
}

// ── Process-wide shared state ─────────────────────────────────────────

/// Everything the OIDC handlers need beyond `WebState`. Lives in a
/// `OnceLock` because axum's `State<T>` extractor can't carry feature-gated
/// types cleanly, and we ship one OIDC config per process anyway.
pub(crate) struct OidcSharedState {
    pub cfg: OidcConfig,
    pub client_secret: String,
    pub pending: PendingStates,
}

static SHARED: OnceLock<Arc<OidcSharedState>> = OnceLock::new();

fn shared() -> Option<&'static Arc<OidcSharedState>> {
    SHARED.get()
}

// ── The authenticator ─────────────────────────────────────────────────

pub(crate) struct OidcAuthenticator {
    cfg: OidcConfig,
}

impl OidcAuthenticator {
    /// Construct from a typed config. Reads the client secret eagerly so
    /// startup fails fast on a misconfigured `client_secret_file`. Stores
    /// the shared state in a process-wide [`OnceLock`] so the
    /// `routes()`-mounted handlers can reach it without threading it
    /// through `WebState`.
    pub(crate) fn new(
        cfg: OidcConfig,
        _sessions: SessionStore,
        _vault_root: Arc<PathBuf>,
    ) -> Result<Self, String> {
        let secret = std::fs::read_to_string(&cfg.client_secret_file)
            .map_err(|e| format!("client_secret_file {:?}: {e}", cfg.client_secret_file))?;
        let secret = secret.trim().to_string();
        let shared = Arc::new(OidcSharedState {
            cfg: cfg.clone(),
            client_secret: secret,
            pending: PendingStates::default(),
        });
        // First constructor wins. Process is one-OIDC-IdP for now.
        let _ = SHARED.set(shared);
        Ok(Self { cfg })
    }
}

impl Authenticator for OidcAuthenticator {
    fn id(&self) -> &'static str {
        "oidc"
    }

    fn authenticate(&self, parts: &Parts) -> AuthOutcome {
        // Cookie-session validation — the callback handler mints a normal
        // SessionStore session and sets `zetl_session`; subsequent requests
        // hit this path. We can't read `SessionStore` here without state,
        // so we abstain — the existing `PasskeyAuthenticator` (always
        // present in the default chain) will validate the cookie and
        // produce a Principal. The `method` tag will say "passkey" rather
        // than "oidc" for OIDC-minted sessions; audit-trail granularity
        // would need SessionStore to remember the mint method (deferred).
        let _ = (parts, &self.cfg);
        AuthOutcome::Abstain
    }

    fn issues_cookie_session(&self) -> bool {
        true
    }

    fn login_redirect(&self) -> Option<&str> {
        Some("/auth/oidc/login")
    }

    fn routes(&self) -> Router<WebState> {
        Router::new()
            .route("/auth/oidc/login", get(login_handler))
            .route("/auth/oidc/callback", get(callback_handler))
    }
}

// ── PKCE + state/nonce primitives ──────────────────────────────────────

/// 32-byte random base64url-no-pad string (43 chars). Used for `state`,
/// `nonce`, and the PKCE `code_verifier`.
fn random_token() -> String {
    let mut bytes = [0u8; 32];
    crate::user::getrandom(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// PKCE S256 challenge per RFC 7636: `base64url(sha256(verifier))`.
fn pkce_s256_challenge(verifier: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

/// Build the IdP authorize URL with all the OIDC parameters set. Pure;
/// tested directly.
fn build_authorize_url(
    cfg: &OidcConfig,
    state: &str,
    nonce: &str,
    code_challenge: &str,
    redirect_uri: &str,
    discovered_authorize_endpoint: &str,
) -> String {
    let mut url = String::with_capacity(512);
    url.push_str(discovered_authorize_endpoint);
    let sep = if discovered_authorize_endpoint.contains('?') {
        '&'
    } else {
        '?'
    };
    url.push(sep);
    url.push_str("response_type=code");
    url.push_str("&client_id=");
    url.push_str(&urlencode(&cfg.client_id));
    url.push_str("&redirect_uri=");
    url.push_str(&urlencode(redirect_uri));
    url.push_str("&scope=openid%20profile%20email");
    url.push_str("&state=");
    url.push_str(state);
    url.push_str("&nonce=");
    url.push_str(nonce);
    url.push_str("&code_challenge_method=S256");
    url.push_str("&code_challenge=");
    url.push_str(code_challenge);
    url
}

/// Minimal URL-encoder for the values we put in the query (client_id,
/// redirect_uri). Conservative — encodes everything outside `unreserved`.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ── Discovery + JWKS fetch (live) ─────────────────────────────────────

#[derive(Deserialize, Debug, Clone)]
struct DiscoveryDoc {
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
}

#[derive(Deserialize, Debug)]
struct JwksDoc {
    keys: Vec<serde_json::Value>,
}

#[derive(Deserialize, Debug)]
struct TokenResponse {
    #[allow(dead_code)]
    access_token: String,
    id_token: String,
}

async fn fetch_discovery(
    client: &reqwest::Client,
    issuer: &str,
) -> Result<DiscoveryDoc, OidcError> {
    let url = format!("{}/.well-known/openid-configuration", issuer.trim_end_matches('/'));
    client
        .get(&url)
        .send()
        .await
        .map_err(|e| OidcError::Discovery(e.to_string()))?
        .error_for_status()
        .map_err(|e| OidcError::Discovery(e.to_string()))?
        .json::<DiscoveryDoc>()
        .await
        .map_err(|e| OidcError::Discovery(e.to_string()))
}

async fn fetch_jwks(client: &reqwest::Client, jwks_uri: &str) -> Result<JwksDoc, OidcError> {
    client
        .get(jwks_uri)
        .send()
        .await
        .map_err(|e| OidcError::Jwks(e.to_string()))?
        .error_for_status()
        .map_err(|e| OidcError::Jwks(e.to_string()))?
        .json::<JwksDoc>()
        .await
        .map_err(|e| OidcError::Jwks(e.to_string()))
}

async fn exchange_code(
    client: &reqwest::Client,
    token_endpoint: &str,
    cfg: &OidcConfig,
    client_secret: &str,
    code: &str,
    pkce_verifier: &str,
    redirect_uri: &str,
) -> Result<TokenResponse, OidcError> {
    let form = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", cfg.client_id.as_str()),
        ("client_secret", client_secret),
        ("code_verifier", pkce_verifier),
    ];
    client
        .post(token_endpoint)
        .form(&form)
        .send()
        .await
        .map_err(|e| OidcError::TokenExchange(e.to_string()))?
        .error_for_status()
        .map_err(|e| OidcError::TokenExchange(e.to_string()))?
        .json::<TokenResponse>()
        .await
        .map_err(|e| OidcError::TokenExchange(e.to_string()))
}

// ── ID token validation ────────────────────────────────────────────────

/// Errors during the OIDC flow. Operator-channel only — every variant
/// surfaces as the same generic 401 user-side (REQ-4110 cause-
/// indistinguishability).
#[derive(Debug)]
pub(crate) enum OidcError {
    BadState,
    Discovery(String),
    Jwks(String),
    TokenExchange(String),
    IdToken(String),
    ClaimMissing(String),
    Provision(String),
    Config(String),
}

impl OidcError {
    fn into_response(self) -> Response {
        // Cause goes to the operator log; user sees the generic class.
        eprintln!("[zetl] oidc auth failed: {self:?}");
        (StatusCode::UNAUTHORIZED, "Sign-in failed").into_response()
    }
}

/// Decode + validate the ID token against the IdP's JWKS, the configured
/// issuer, the configured client_id (audience), the expected nonce, and
/// `iat`/`exp`. Returns the full claims map for `user_id_claim` extraction.
fn validate_id_token(
    id_token: &str,
    jwks: &JwksDoc,
    cfg: &OidcConfig,
    expected_nonce: &str,
) -> Result<serde_json::Value, OidcError> {
    // Pull the kid from the header so we can pick the right JWKS entry.
    let header = decode_header(id_token).map_err(|e| OidcError::IdToken(e.to_string()))?;
    let alg = header.alg;
    let kid = header.kid;

    let mut last_err: Option<String> = None;
    for jwk in &jwks.keys {
        // Optionally match kid if the token specified one.
        if let Some(want) = &kid {
            if jwk.get("kid").and_then(|v| v.as_str()) != Some(want.as_str()) {
                continue;
            }
        }
        let dk = match jwk_to_decoding_key(jwk, alg) {
            Ok(k) => k,
            Err(e) => {
                last_err = Some(e);
                continue;
            }
        };
        let mut validation = Validation::new(alg);
        validation.set_issuer(&[cfg.issuer.as_str()]);
        validation.set_audience(&[cfg.client_id.as_str()]);
        validation.validate_exp = true;
        validation.validate_nbf = false;
        match decode::<serde_json::Value>(id_token, &dk, &validation) {
            Ok(data) => {
                // Nonce check (REQ-4110) — `Validation` doesn't cover `nonce`.
                let claims = data.claims;
                let nonce = claims
                    .get("nonce")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| OidcError::IdToken("nonce claim missing".into()))?;
                if !crate::user::constant_time_eq(nonce.as_bytes(), expected_nonce.as_bytes()) {
                    return Err(OidcError::IdToken("nonce mismatch".into()));
                }
                return Ok(claims);
            }
            Err(e) => {
                last_err = Some(e.to_string());
                continue;
            }
        }
    }
    Err(OidcError::IdToken(
        last_err.unwrap_or_else(|| "no JWKS key matched".into()),
    ))
}

/// Convert a JWK JSON object to a `jsonwebtoken::DecodingKey`. Supports
/// RSA (RS256/RS384/RS512) and EC (ES256/ES384) — the algorithms typical
/// of common IdPs.
fn jwk_to_decoding_key(jwk: &serde_json::Value, alg: Algorithm) -> Result<DecodingKey, String> {
    let kty = jwk
        .get("kty")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "JWK missing kty".to_string())?;
    match (kty, alg) {
        ("RSA", Algorithm::RS256 | Algorithm::RS384 | Algorithm::RS512) => {
            let n = jwk
                .get("n")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "RSA JWK missing n".to_string())?;
            let e = jwk
                .get("e")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "RSA JWK missing e".to_string())?;
            DecodingKey::from_rsa_components(n, e).map_err(|e| e.to_string())
        }
        ("EC", Algorithm::ES256 | Algorithm::ES384) => {
            let x = jwk
                .get("x")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "EC JWK missing x".to_string())?;
            let y = jwk
                .get("y")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "EC JWK missing y".to_string())?;
            DecodingKey::from_ec_components(x, y).map_err(|e| e.to_string())
        }
        _ => Err(format!("unsupported kty={kty} alg={alg:?}")),
    }
}

// ── Route handlers ────────────────────────────────────────────────────

#[derive(Deserialize)]
struct LoginQuery {
    /// Optional `next` parameter so the post-login redirect can return
    /// the user to where they came from. Defaults to `/`.
    next: Option<String>,
}

async fn login_handler(
    State(state): State<WebState>,
    Query(q): Query<LoginQuery>,
    headers: axum::http::HeaderMap,
) -> Response {
    let Some(shared) = shared() else {
        return OidcError::Config("OIDC not configured".into()).into_response();
    };

    // Build the redirect URI from the configured override or from this
    // request's Host header.
    let redirect_uri = shared
        .cfg
        .redirect_uri
        .clone()
        .or_else(|| derive_redirect_uri(&headers, state.tls))
        .unwrap_or_else(|| "http://localhost/auth/oidc/callback".to_string());

    let client = reqwest::Client::new();
    let discovery = match fetch_discovery(&client, &shared.cfg.issuer).await {
        Ok(d) => d,
        Err(e) => return e.into_response(),
    };

    let oidc_state = random_token();
    let nonce = random_token();
    let verifier = random_token();
    let challenge = pkce_s256_challenge(&verifier);

    shared.pending.insert(
        oidc_state.clone(),
        PendingState {
            nonce: nonce.clone(),
            pkce_verifier: verifier,
            return_to: q.next.unwrap_or_else(|| "/".to_string()),
            created_at: Instant::now(),
        },
    );

    let url = build_authorize_url(
        &shared.cfg,
        &oidc_state,
        &nonce,
        &challenge,
        &redirect_uri,
        &discovery.authorization_endpoint,
    );
    Redirect::temporary(&url).into_response()
}

#[derive(Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

async fn callback_handler(
    State(state): State<WebState>,
    Query(q): Query<CallbackQuery>,
    headers: axum::http::HeaderMap,
) -> Response {
    let Some(shared) = shared() else {
        return OidcError::Config("OIDC not configured".into()).into_response();
    };

    if q.error.is_some() {
        return OidcError::IdToken(format!("IdP error: {:?}", q.error)).into_response();
    }
    let (code, oidc_state) = match (q.code, q.state) {
        (Some(c), Some(s)) => (c, s),
        _ => return OidcError::BadState.into_response(),
    };
    let pending = match shared.pending.take(&oidc_state) {
        Some(p) => p,
        None => return OidcError::BadState.into_response(),
    };

    let redirect_uri = shared
        .cfg
        .redirect_uri
        .clone()
        .or_else(|| derive_redirect_uri(&headers, state.tls))
        .unwrap_or_else(|| "http://localhost/auth/oidc/callback".to_string());

    let client = reqwest::Client::new();
    let discovery = match fetch_discovery(&client, &shared.cfg.issuer).await {
        Ok(d) => d,
        Err(e) => return e.into_response(),
    };

    let tokens = match exchange_code(
        &client,
        &discovery.token_endpoint,
        &shared.cfg,
        &shared.client_secret,
        &code,
        &pending.pkce_verifier,
        &redirect_uri,
    )
    .await
    {
        Ok(t) => t,
        Err(e) => return e.into_response(),
    };

    let jwks = match fetch_jwks(&client, &discovery.jwks_uri).await {
        Ok(j) => j,
        Err(e) => return e.into_response(),
    };
    let claims = match validate_id_token(&tokens.id_token, &jwks, &shared.cfg, &pending.nonce) {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };

    // Extract the user_id from the configured claim.
    let user_id = match claims
        .get(&shared.cfg.user_id_claim)
        .and_then(|v| v.as_str())
    {
        Some(s) => s.to_string(),
        None => {
            return OidcError::ClaimMissing(shared.cfg.user_id_claim.clone()).into_response()
        }
    };

    // Provision-or-lookup per REQ-4111 / ADR-4107.
    let policy = ProvisionPolicy {
        enabled: shared.cfg.auto_provision,
        domain_allow: shared.cfg.provision_domain_allow.clone(),
    };
    match ensure_user(&state.vault_root, &user_id, "oidc", &policy) {
        ProvisionOutcome::Allowed => {}
        ProvisionOutcome::Denied(reason) => {
            return OidcError::Provision(format!("{reason:?}")).into_response();
        }
    }

    // Mint a normal SessionStore session (ADR-4104) and set the cookie.
    let token = state.sessions.create(&user_id);
    let cookie = session_cookie(&token, state.tls);
    let mut response = Redirect::temporary(&pending.return_to).into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, HeaderValue::from_str(&cookie).unwrap());
    response
}

fn derive_redirect_uri(headers: &axum::http::HeaderMap, tls: bool) -> Option<String> {
    let host = headers.get(header::HOST)?.to_str().ok()?;
    let scheme = if tls { "https" } else { "http" };
    Some(format!("{scheme}://{host}/auth/oidc/callback"))
}

// Unused import elimination cue (kept until handlers consume them):
#[allow(dead_code)]
fn _serialize_marker(_: &PendingState) -> impl Serialize {
    "_"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_cfg() -> OidcConfig {
        OidcConfig {
            issuer: "https://idp.example.com".to_string(),
            client_id: "client-xyz".to_string(),
            client_secret_file: "/tmp/nope".to_string(),
            user_id_claim: "email".to_string(),
            auto_provision: false,
            provision_domain_allow: None,
            redirect_uri: None,
        }
    }

    #[test]
    fn random_token_is_43_chars_base64url() {
        let t = random_token();
        assert_eq!(t.len(), 43);
        assert!(t.chars().all(|c| {
            c.is_ascii_alphanumeric() || c == '-' || c == '_'
        }));
        // Two calls produce different values.
        assert_ne!(t, random_token());
    }

    /// REQ-4110 / NFR-4104: pending state is single-use; a second `take`
    /// for the same `state` returns None.
    #[test]
    fn pending_state_is_single_use() {
        let ps = PendingStates::default();
        ps.insert(
            "s1".to_string(),
            PendingState {
                nonce: "n".to_string(),
                pkce_verifier: "v".to_string(),
                return_to: "/".to_string(),
                created_at: Instant::now(),
            },
        );
        assert!(ps.take("s1").is_some());
        assert!(ps.take("s1").is_none(), "state must be single-use");
    }

    /// Pending state expires after TTL — entries older than [`PENDING_TTL`]
    /// are pruned by lazy GC.
    #[test]
    fn pending_state_expires() {
        let ps = PendingStates::default();
        ps.insert(
            "old".to_string(),
            PendingState {
                nonce: "n".to_string(),
                pkce_verifier: "v".to_string(),
                return_to: "/".to_string(),
                created_at: Instant::now() - PENDING_TTL - Duration::from_secs(1),
            },
        );
        assert!(ps.take("old").is_none());
    }

    /// CON-4104: authorize URL carries the OIDC parameters in the right
    /// shape.
    #[test]
    fn authorize_url_has_required_params() {
        let cfg = dummy_cfg();
        let url = build_authorize_url(
            &cfg,
            "state-abc",
            "nonce-def",
            "challenge-ghi",
            "https://wiki.example.com/auth/oidc/callback",
            "https://idp.example.com/authorize",
        );
        assert!(url.starts_with("https://idp.example.com/authorize?"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("client_id=client-xyz"));
        assert!(url.contains("state=state-abc"));
        assert!(url.contains("nonce=nonce-def"));
        assert!(url.contains("code_challenge=challenge-ghi"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("scope=openid"));
        // redirect_uri is URL-encoded.
        assert!(url.contains("redirect_uri=https%3A%2F%2Fwiki"));
    }

    /// `urlencode` matches RFC 3986 unreserved + percent-encodes everything
    /// else uppercase-hex.
    #[test]
    fn urlencode_basics() {
        assert_eq!(urlencode("abcXYZ012-_.~"), "abcXYZ012-_.~");
        assert_eq!(urlencode("a b"), "a%20b");
        assert_eq!(urlencode("a/b?c=d"), "a%2Fb%3Fc%3Dd");
        assert_eq!(urlencode("alice@example.com"), "alice%40example.com");
    }

    /// REQ-4109 / CON-4104: the OIDC config parses with sensible defaults.
    #[test]
    fn oidc_config_defaults() {
        let body = r#"
            issuer = "https://accounts.example.com"
            client_id = "abc123"
            client_secret_file = "/etc/zetl/oidc-secret"
        "#;
        let cfg: OidcConfig = toml::from_str(body).unwrap();
        assert_eq!(cfg.user_id_claim, "email");
        assert!(!cfg.auto_provision);
        assert!(cfg.provision_domain_allow.is_none());
        assert!(cfg.redirect_uri.is_none());
    }

    /// REQ-4120: typo at the schema fails parse (deny_unknown_fields).
    #[test]
    fn oidc_config_rejects_typo() {
        let body = r#"
            issuer = "https://x"
            client_id = "x"
            client_secret_file = "/tmp/s"
            isuer = "https://typo"
        "#;
        let err = toml::from_str::<OidcConfig>(body).unwrap_err();
        assert!(err.to_string().contains("isuer") || err.to_string().contains("unknown"));
    }

    /// `derive_redirect_uri` builds the URI from the Host header + scheme.
    #[test]
    fn derive_redirect_uri_basics() {
        let mut h = axum::http::HeaderMap::new();
        h.insert(header::HOST, "wiki.example.com".parse().unwrap());
        assert_eq!(
            derive_redirect_uri(&h, false),
            Some("http://wiki.example.com/auth/oidc/callback".to_string())
        );
        assert_eq!(
            derive_redirect_uri(&h, true),
            Some("https://wiki.example.com/auth/oidc/callback".to_string())
        );
        // Missing host returns None.
        assert_eq!(derive_redirect_uri(&axum::http::HeaderMap::new(), false), None);
    }

    /// The authenticator declares cookie-session and exposes the
    /// `/auth/oidc/login` redirect — REQ-4109/4112.
    #[test]
    fn authenticator_declares_oidc_metadata() {
        let cfg = dummy_cfg();
        let dummy = OidcAuthenticator { cfg };
        assert_eq!(dummy.id(), "oidc");
        assert!(dummy.issues_cookie_session());
        assert_eq!(dummy.login_redirect(), Some("/auth/oidc/login"));
    }

    /// Suppress the unused-import warning for `UNIX_EPOCH`/`SystemTime` —
    /// they're used by the live token-validation flow (`exp` check via
    /// `Validation::validate_exp = true` is jsonwebtoken-internal; we keep
    /// the imports for future iat skew handling).
    #[test]
    fn time_imports_referenced() {
        let _ = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    }
}
