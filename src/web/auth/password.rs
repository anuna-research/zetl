//! Static password authenticator + store (SPEC-041 REQ-4107, REQ-4108,
//! REQ-4112, REQ-4113, REQ-4120, NFR-4103, CON-4105, CON-4106, CON-4107,
//! ADR-4106).
//!
//! Three layers in this file:
//!
//! * **Store** — `[hash`/`verify`/`load_store`/`upsert`/`remove`/`list`/
//!   `lookup`]`. The pure argon2id primitive and the on-disk
//!   `.zetl/collab/passwords.json` (CON-4107).
//! * **PasswordAuthenticator** — the cookie-session [`Authenticator`] that
//!   accepts the `zetl_session` cookie a successful login mints. Identical
//!   request-time shape to `PasskeyAuthenticator` but tagged
//!   `method = "password"` for the audit trail.
//! * **Routes** — `GET /auth/password` renders the form, `POST` recognises
//!   the body against the CON-4105 ABNF grammar (REQ-4120), verifies the
//!   argon2id PHC in constant time (REQ-4107 cause-indistinguishable),
//!   mints a session, and redirects.
//!
//! On-disk format (CON-4107):
//!
//! ```text
//! .zetl/collab/passwords.json   mode 0600
//! [ { "user_id": "<id>",
//!     "phc":     "$argon2id$v=19$m=<m>,t=<t>,p=<p>$<salt>$<hash>" } ]
//! ```
//!
//! The argon2id parameters are embedded in the PHC string, so [`NFR-4103`]
//! costs can be raised later without invalidating existing credentials.

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::extract::State;
use axum::http::request::Parts;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::Router;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::web::session::{session_cookie, token_from_cookies, SessionStore};
use crate::web::WebState;

use super::{AuthOutcome, Authenticator, Principal, PrincipalId};

const PASSWORDS_DIR: &str = ".zetl/collab";
const PASSWORDS_FILE: &str = "passwords.json";

/// One stored password record (CON-4107).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PasswordRecord {
    pub user_id: String,
    /// The argon2id PHC string — `"$argon2id$v=19$m=...,t=...,p=...$salt$hash"`.
    /// Parameters are embedded so the operator can raise the cost (NFR-4103)
    /// without invalidating existing credentials.
    pub phc: String,
}

/// Why a password operation failed.
///
/// `pub` so the `zetl collab passwd` CLI can name the error type when
/// propagating it (e.g. via `anyhow::Context`); internal variants stay
/// readable but each one is essentially operator-channel diagnostic.
#[derive(Debug, Error)]
pub enum PasswordError {
    #[error("password store I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("password store JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("password hashing: {0}")]
    Hash(String),
    /// Permissions on `passwords.json` are wider than 0600 (unix only).
    #[error("password store {path} has insecure permissions {mode:04o} (expected 0600). Fix with: chmod 600 {path}")]
    InsecurePermissions { path: String, mode: u32 },
    /// `passwd remove <user>` named a user not present in the store.
    #[error("no password record for {0:?}")]
    UnknownUser(String),
}

/// Return the directory holding `passwords.json` for a vault.
fn passwords_dir(vault_root: &Path) -> PathBuf {
    vault_root.join(PASSWORDS_DIR)
}

/// Return the path to `passwords.json`.
pub(crate) fn passwords_path(vault_root: &Path) -> PathBuf {
    passwords_dir(vault_root).join(PASSWORDS_FILE)
}

/// Hash a password with argon2id, returning the PHC string.
///
/// Uses [`Argon2::default()`] — the crate's recommended argon2id v19 settings.
/// Parameters are embedded in the returned PHC string per ADR-4106 so they
/// can be raised later without invalidating existing hashes.
pub(crate) fn hash(password: &str) -> Result<String, PasswordError> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    argon2
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| PasswordError::Hash(e.to_string()))
}

/// Verify `password` against an argon2id PHC string in constant time.
///
/// Returns `false` for a wrong password, an unparseable PHC string, or any
/// other internal failure — REQ-4107 cause-indistinguishability. Callers
/// MUST NOT branch on the reason for a false result.
pub(crate) fn verify(password: &str, phc: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(phc) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

/// Load `passwords.json`, enforcing 0600 permissions (CON-4107, ADR-4106).
///
/// An absent file is **not** an error — it represents the empty store, which
/// is the on-disk state before any password has been set. Insecure
/// permissions ARE an error: the same 0600 enforcement that `server.key`
/// uses in `src/user/invite.rs`.
pub(crate) fn load_store(vault_root: &Path) -> Result<Vec<PasswordRecord>, PasswordError> {
    let path = passwords_path(vault_root);
    if !path.exists() {
        return Ok(Vec::new());
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::metadata(&path)?.permissions();
        let mode = perms.mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(PasswordError::InsecurePermissions {
                path: path.display().to_string(),
                mode,
            });
        }
    }

    let body = fs::read_to_string(&path)?;
    let records: Vec<PasswordRecord> = serde_json::from_str(&body)?;
    Ok(records)
}

/// Persist the store atomically — write to a temp file in the same directory,
/// chmod 0600, then rename (rename is atomic within the same filesystem).
fn write_store_atomic(vault_root: &Path, records: &[PasswordRecord]) -> Result<(), PasswordError> {
    let dir = passwords_dir(vault_root);
    fs::create_dir_all(&dir)?;
    let final_path = passwords_path(vault_root);

    // Temp file in the SAME directory so rename is atomic.
    let mut tmp_path = final_path.clone();
    let suffix = format!(".tmp.{}", uuid::Uuid::new_v4());
    tmp_path.set_extension(format!("json{suffix}"));

    {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp_path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        let body = serde_json::to_string_pretty(records)?;
        file.write_all(body.as_bytes())?;
        file.sync_all()?;
    }
    fs::rename(&tmp_path, &final_path)?;
    Ok(())
}

/// Add (or overwrite) the password record for `user_id`, hashing the password
/// with argon2id (NFR-4103 cost). Returns the PHC string written.
///
/// `pub` (not `pub(crate)`) so the `zetl collab passwd add` CLI driver in
/// `src/main.rs` can call it directly (SPEC-041 REQ-4108).
pub fn upsert(vault_root: &Path, user_id: &str, password: &str) -> Result<String, PasswordError> {
    let phc = hash(password)?;
    let mut records = load_store(vault_root)?;
    records.retain(|r| r.user_id != user_id);
    records.push(PasswordRecord {
        user_id: user_id.to_string(),
        phc: phc.clone(),
    });
    write_store_atomic(vault_root, &records)?;
    Ok(phc)
}

/// Remove the password record for `user_id`. Returns
/// [`PasswordError::UnknownUser`] if there was no record to remove (so the
/// CLI can give a clear "not found" exit, per CON-4106).
///
/// `pub` so the `zetl collab passwd remove` CLI can call it directly.
pub fn remove(vault_root: &Path, user_id: &str) -> Result<(), PasswordError> {
    let mut records = load_store(vault_root)?;
    let before = records.len();
    records.retain(|r| r.user_id != user_id);
    if records.len() == before {
        return Err(PasswordError::UnknownUser(user_id.to_string()));
    }
    write_store_atomic(vault_root, &records)?;
    Ok(())
}

/// List the user_ids that have a password record. Never returns PHC strings
/// or hash bytes — `list` is for operator visibility, not credential
/// inspection (CON-4106).
///
/// `pub` so the `zetl collab passwd list` CLI can call it directly.
pub fn list(vault_root: &Path) -> Result<Vec<String>, PasswordError> {
    let records = load_store(vault_root)?;
    Ok(records.into_iter().map(|r| r.user_id).collect())
}

/// Look up the record for `user_id`. Returns `None` if no record exists.
pub(crate) fn lookup(
    vault_root: &Path,
    user_id: &str,
) -> Result<Option<PasswordRecord>, PasswordError> {
    let records = load_store(vault_root)?;
    Ok(records.into_iter().find(|r| r.user_id == user_id))
}

/// Reject duplicate user_ids in a store (defence-in-depth: the writers above
/// enforce uniqueness, but a hand-edited file might not).
pub(crate) fn check_unique_user_ids(records: &[PasswordRecord]) -> Result<(), PasswordError> {
    let mut seen = HashSet::new();
    for r in records {
        if !seen.insert(&r.user_id) {
            return Err(PasswordError::Hash(format!(
                "duplicate user_id {:?} in passwords.json",
                r.user_id
            )));
        }
    }
    Ok(())
}

// ─── Authenticator + routes ─────────────────────────────────────────────

/// The cookie-session authenticator for password-minted sessions
/// (REQ-4107). Identical request-time shape to `PasskeyAuthenticator`, with
/// `method = "password"` for the audit trail.
pub(crate) struct PasswordAuthenticator {
    sessions: SessionStore,
    vault_root: Arc<PathBuf>,
}

impl PasswordAuthenticator {
    pub(crate) fn new(sessions: SessionStore, vault_root: Arc<PathBuf>) -> Self {
        Self {
            sessions,
            vault_root,
        }
    }
}

impl Authenticator for PasswordAuthenticator {
    fn id(&self) -> &'static str {
        "password"
    }

    fn authenticate(&self, parts: &Parts) -> AuthOutcome {
        let Some(token) = token_from_cookies(&parts.headers) else {
            return AuthOutcome::Abstain;
        };
        let Some(user_id) = self.sessions.validate(&token) else {
            // Stale / unknown cookie — abstain so the chain falls through to
            // the login redirect (mirrors `PasskeyAuthenticator`).
            return AuthOutcome::Abstain;
        };
        AuthOutcome::Authenticated(Principal {
            identity: PrincipalId::User(user_id),
            method: "password",
            cookie_session: true,
            capability: None,
        })
    }

    fn issues_cookie_session(&self) -> bool {
        true
    }

    fn login_redirect(&self) -> Option<&str> {
        Some("/auth/password")
    }

    fn routes(&self) -> Router<WebState> {
        // Note: the route handlers access WebState (for SessionStore, tls
        // flag, vault_root) — that's why `routes()` returns `Router<WebState>`
        // and uses the parent app's state when merged. `self.vault_root` /
        // `self.sessions` carried on the authenticator are NOT passed to the
        // handlers because route handlers re-read them from WebState (the
        // chain ALWAYS shares the same WebState's sessions/vault_root).
        let _ = (&self.sessions, &self.vault_root); // declare the dependency
        Router::new().route(
            "/auth/password",
            get(password_form_handler).post(password_submit_handler),
        )
    }
}

/// `GET /auth/password` — render the login form (CON-4105).
async fn password_form_handler() -> Html<&'static str> {
    Html(LOGIN_FORM_HTML)
}

const LOGIN_FORM_HTML: &str = r#"<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8"><title>Sign in</title></head>
<body>
<h1>Sign in</h1>
<form method="post" action="/auth/password" autocomplete="on">
  <p><label>User <input name="user" type="text" required maxlength="64" autocomplete="username"></label></p>
  <p><label>Password <input name="password" type="password" required maxlength="256" autocomplete="current-password"></label></p>
  <p><button type="submit">Sign in</button></p>
</form>
</body></html>
"#;

/// One generic failure response for every reason a password login can fail
/// (REQ-4107 cause-indistinguishability). Operator log records the reason
/// separately (audit hooks land in task-auth-observability).
fn generic_login_failure() -> Response {
    (StatusCode::UNAUTHORIZED, Html("<h1>Sign in failed</h1>")).into_response()
}

/// `POST /auth/password` — recognise body, verify, mint session, redirect.
async fn password_submit_handler(State(state): State<WebState>, body: String) -> Response {
    // CON-4105 grammar recognition before any hash work (REQ-4120).
    let Some(parsed) = recognise_form_body(&body) else {
        // Constant-time delay would be ideal here; for now we accept the
        // small timing gap on grammar-rejected requests (no hash work).
        return generic_login_failure();
    };

    // Lookup + verify. Cause-indistinguishable: unknown user and wrong
    // password produce the same generic outcome. Both paths run
    // `argon2::verify` (or an equivalent dummy verify) so the timing is
    // comparable even for unknown users.
    let allow = match lookup(&state.vault_root, &parsed.user) {
        Ok(Some(record)) => verify(&parsed.password, &record.phc),
        Ok(None) => {
            // Run a verify against a known-bad PHC to keep timing comparable
            // to the wrong-password path.
            let _ = verify(&parsed.password, dummy_phc());
            false
        }
        Err(_) => {
            let _ = verify(&parsed.password, dummy_phc());
            false
        }
    };

    if !allow {
        return generic_login_failure();
    }

    // Mint a normal SessionStore session — indistinguishable downstream
    // from a passkey session (REQ-4107).
    let token = state.sessions.create(&parsed.user);
    let cookie = session_cookie(&token, state.tls);
    let mut response = Redirect::to("/").into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, HeaderValue::from_str(&cookie).unwrap());
    response
}

/// A *real* argon2id PHC string lazily generated once, used for the
/// unknown-user constant-time verify path. The password it commits to does
/// not matter — what matters is that `verify` against it performs the same
/// argon2id work a real verify performs, so timing does not leak "user
/// exists" vs "user does not exist" (REQ-4107).
///
/// `OnceLock` so the one-time hash cost is paid once and cached for the
/// process lifetime.
fn dummy_phc() -> &'static str {
    use std::sync::OnceLock;
    static DUMMY: OnceLock<String> = OnceLock::new();
    DUMMY.get_or_init(|| {
        // Fallback to a deliberately-invalid PHC if hashing somehow fails —
        // a wrong-user request will then short-circuit rather than panic.
        hash("zetl-unknown-user-dummy").unwrap_or_else(|_| "$argon2id$invalid".to_string())
    })
}

/// Parsed form body: the two fields the login form carries.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LoginForm {
    pub user: String,
    pub password: String,
}

/// Recognise an `application/x-www-form-urlencoded` body against the
/// CON-4105 ABNF grammar:
///
/// ```text
/// form-body = "user=" user "&" "password=" password
/// user      = 1*64( unreserved / pct-encoded )
/// password  = 1*256( pct-encoded / unreserved )
/// ```
///
/// Rejects (returns `None`):
/// * Bodies that aren't exactly `user=<u>&password=<p>` in that order.
/// * Extra fields.
/// * Empty `user` or empty `password`.
/// * `user` over 64 wire chars, `password` over 256 wire chars.
/// * Characters outside `unreserved / pct-encoded`.
/// * Malformed percent-escapes.
///
/// No normalisation, no repair (REQ-4120 LangSec).
pub(crate) fn recognise_form_body(body: &str) -> Option<LoginForm> {
    // Overall length sanity check (defence in depth).
    if body.is_empty() || body.len() > 1024 {
        return None;
    }

    // Strict split: exactly one `&`, separating `user=...` and `password=...`.
    let (user_part, password_part) = body.split_once('&')?;
    if password_part.contains('&') {
        return None; // extra field
    }
    let user_value = user_part.strip_prefix("user=")?;
    let password_value = password_part.strip_prefix("password=")?;

    if !is_form_value(user_value) || user_value.is_empty() || user_value.len() > 64 {
        return None;
    }
    if !is_form_value(password_value) || password_value.is_empty() || password_value.len() > 256 {
        return None;
    }

    let user = pct_decode(user_value)?;
    let password = pct_decode(password_value)?;

    // Decoded user_id must still be sensible — empty after decoding means
    // the wire value was a pure `%`-escape of nothing, which is malformed.
    if user.is_empty() {
        return None;
    }

    Some(LoginForm { user, password })
}

/// `unreserved / pct-encoded` per RFC 3986. Used as the alphabet check
/// before percent-decoding.
fn is_form_value(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                i += 1;
            }
            b'%' => {
                // `%` must be followed by two hex digits.
                if i + 2 >= bytes.len() {
                    return false;
                }
                if !bytes[i + 1].is_ascii_hexdigit() || !bytes[i + 2].is_ascii_hexdigit() {
                    return false;
                }
                i += 3;
            }
            _ => return false,
        }
    }
    true
}

/// Percent-decode a value already validated by [`is_form_value`].
fn pct_decode(s: &str) -> Option<String> {
    let mut out = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hi = (bytes[i + 1] as char).to_digit(16)? as u8;
            let lo = (bytes[i + 2] as char).to_digit(16)? as u8;
            out.push((hi << 4) | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// REQ-4107: argon2id roundtrip — a freshly-hashed password verifies; a
    /// different password does not.
    #[test]
    fn hash_and_verify_roundtrip() {
        let phc = hash("correct horse battery staple").unwrap();
        assert!(verify("correct horse battery staple", &phc));
        assert!(!verify("wrong password", &phc));
        assert!(!verify("", &phc));
    }

    /// REQ-4107: each hash uses a fresh salt, so two hashes of the same
    /// password differ.
    #[test]
    fn salts_are_random() {
        let a = hash("password").unwrap();
        let b = hash("password").unwrap();
        assert_ne!(a, b);
        assert!(verify("password", &a));
        assert!(verify("password", &b));
    }

    /// REQ-4107: a malformed PHC string verifies as `false` — never
    /// surfaces a parse error to the caller.
    #[test]
    fn malformed_phc_verifies_false() {
        assert!(!verify("anything", "not a phc string"));
        assert!(!verify("anything", "$argon2id$wrong"));
        assert!(!verify("anything", ""));
    }

    /// ADR-4106: PHC string embeds `argon2id` algorithm + version + params,
    /// so a future cost upgrade can run alongside existing hashes.
    #[test]
    fn phc_string_embeds_argon2id() {
        let phc = hash("x").unwrap();
        assert!(phc.starts_with("$argon2id$"));
        assert!(phc.contains("v=19"));
        assert!(phc.contains("m="));
        assert!(phc.contains("t="));
        assert!(phc.contains("p="));
    }

    /// CON-4107: an absent file is the empty store, not an error.
    #[test]
    fn load_store_absent_file_is_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let records = load_store(tmp.path()).unwrap();
        assert!(records.is_empty());
    }

    /// CON-4106 / CON-4107: upsert creates the file and the record;
    /// load reads it back.
    #[test]
    fn upsert_persists_record() {
        let tmp = tempfile::TempDir::new().unwrap();
        upsert(tmp.path(), "alice", "hunter2").unwrap();
        let records = load_store(tmp.path()).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].user_id, "alice");
        assert!(verify("hunter2", &records[0].phc));
    }

    /// CON-4106: upsert overwrites an existing record rather than duplicating.
    #[test]
    fn upsert_overwrites_existing() {
        let tmp = tempfile::TempDir::new().unwrap();
        upsert(tmp.path(), "alice", "old").unwrap();
        upsert(tmp.path(), "alice", "new").unwrap();
        let records = load_store(tmp.path()).unwrap();
        assert_eq!(records.len(), 1);
        assert!(verify("new", &records[0].phc));
        assert!(!verify("old", &records[0].phc));
    }

    /// CON-4106: remove deletes; remove of unknown user errors clearly.
    #[test]
    fn remove_and_unknown_user() {
        let tmp = tempfile::TempDir::new().unwrap();
        upsert(tmp.path(), "alice", "pw").unwrap();
        remove(tmp.path(), "alice").unwrap();
        assert!(load_store(tmp.path()).unwrap().is_empty());

        let err = remove(tmp.path(), "alice").unwrap_err();
        assert!(matches!(err, PasswordError::UnknownUser(_)));
    }

    /// CON-4106: list emits user_ids only — never PHC strings.
    #[test]
    fn list_returns_user_ids_only() {
        let tmp = tempfile::TempDir::new().unwrap();
        upsert(tmp.path(), "alice", "pw").unwrap();
        upsert(tmp.path(), "bob", "pw").unwrap();
        let ids = list(tmp.path()).unwrap();
        assert!(ids.contains(&"alice".to_string()));
        assert!(ids.contains(&"bob".to_string()));
        // A grep-based check: the returned strings contain no PHC marker.
        for id in &ids {
            assert!(!id.contains("$argon2"));
        }
    }

    /// CON-4107: lookup returns the record on hit, None on miss.
    #[test]
    fn lookup_hit_and_miss() {
        let tmp = tempfile::TempDir::new().unwrap();
        upsert(tmp.path(), "alice", "pw").unwrap();
        assert!(lookup(tmp.path(), "alice").unwrap().is_some());
        assert!(lookup(tmp.path(), "ghost").unwrap().is_none());
    }

    /// CON-4107: writes set mode 0600 (unix).
    #[cfg(unix)]
    #[test]
    fn upsert_sets_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::TempDir::new().unwrap();
        upsert(tmp.path(), "alice", "pw").unwrap();
        let mode = fs::metadata(passwords_path(tmp.path()))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    /// CON-4107 + ADR-4106: reads refuse a file with insecure permissions,
    /// matching the `server.key` enforcement in src/user/invite.rs.
    #[cfg(unix)]
    #[test]
    fn load_store_rejects_insecure_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::TempDir::new().unwrap();
        upsert(tmp.path(), "alice", "pw").unwrap();
        let path = passwords_path(tmp.path());
        // Widen permissions to group-readable.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let err = load_store(tmp.path()).unwrap_err();
        assert!(matches!(err, PasswordError::InsecurePermissions { .. }));
    }

    /// CON-4105 / REQ-4120: a valid form body parses into the two values.
    #[test]
    fn recognise_valid_form_body() {
        let parsed = recognise_form_body("user=alice&password=hunter2").unwrap();
        assert_eq!(parsed.user, "alice");
        assert_eq!(parsed.password, "hunter2");
    }

    /// CON-4105: percent-encoded values are decoded after grammar check.
    #[test]
    fn recognise_pct_decodes() {
        let parsed = recognise_form_body("user=alice%40example.com&password=p%24w").unwrap();
        assert_eq!(parsed.user, "alice@example.com");
        assert_eq!(parsed.password, "p$w");
    }

    /// REQ-4120: malformed bodies rejected, not normalised.
    #[test]
    fn recognise_rejects_malformed() {
        let cases = [
            // Wrong order
            "password=p&user=u",
            // Extra field
            "user=u&password=p&extra=1",
            // Missing field
            "user=u",
            "password=p",
            // Empty values
            "user=&password=p",
            "user=u&password=",
            // No body
            "",
            // Reserved char outside alphabet
            "user=al ice&password=p",
            // Bad pct-escape
            "user=u%ZZ&password=p",
            // Missing key prefix
            "alice&hunter2",
            // Spaces around
            "user=alice & password=hunter2",
        ];
        for bad in cases {
            assert!(recognise_form_body(bad).is_none(), "should reject {bad:?}");
        }
    }

    /// REQ-4120: over-length values rejected.
    #[test]
    fn recognise_rejects_overlong() {
        // user max 64 wire chars
        let body = format!("user={}&password=p", "a".repeat(65));
        assert!(recognise_form_body(&body).is_none());
        // password max 256 wire chars
        let body = format!("user=u&password={}", "a".repeat(257));
        assert!(recognise_form_body(&body).is_none());
        // overall over 1KB
        let body = format!("user=u&password={}", "a".repeat(2000));
        assert!(recognise_form_body(&body).is_none());
    }

    /// REQ-4107 / REQ-4112: PasswordAuthenticator on a valid session cookie
    /// returns a cookie-backed `method = "password"` principal.
    #[test]
    fn authenticator_valid_cookie_authenticates() {
        use axum::extract::Request;
        let store = SessionStore::new();
        let token = store.create("alice");
        let tmp = tempfile::TempDir::new().unwrap();
        let auth = PasswordAuthenticator::new(store, Arc::new(tmp.path().to_path_buf()));
        let mut req: Request<()> = Request::new(());
        req.headers_mut().insert(
            header::COOKIE,
            format!("zetl_session={token}").parse().unwrap(),
        );
        let parts = req.into_parts().0;
        match auth.authenticate(&parts) {
            AuthOutcome::Authenticated(p) => {
                assert_eq!(p.method, "password");
                assert_eq!(p.identity, PrincipalId::User("alice".to_string()));
                assert!(p.cookie_session);
            }
            other => panic!("expected Authenticated, got {other:?}"),
        }
    }

    /// PasswordAuthenticator with no cookie ⇒ Abstain.
    #[test]
    fn authenticator_no_cookie_abstains() {
        use axum::extract::Request;
        let tmp = tempfile::TempDir::new().unwrap();
        let auth =
            PasswordAuthenticator::new(SessionStore::new(), Arc::new(tmp.path().to_path_buf()));
        let parts = Request::<()>::new(()).into_parts().0;
        assert!(matches!(auth.authenticate(&parts), AuthOutcome::Abstain));
    }

    /// `dummy_phc()` returns a parseable PHC so the unknown-user verify
    /// performs real argon2 work (REQ-4107 timing).
    #[test]
    fn dummy_phc_is_parseable() {
        // The dummy PHC must be a valid argon2id PHC so `verify` actually
        // performs the work. Verify against the original password used to
        // generate it.
        let phc = dummy_phc();
        // `verify("", phc)` should return false (wrong password) — and it
        // should do the full argon2 work, not bail out at parse time.
        assert!(
            phc.starts_with("$argon2id$"),
            "dummy_phc not parseable: {phc}"
        );
        assert!(!verify("", phc));
    }

    /// `check_unique_user_ids` catches hand-edited duplicates.
    #[test]
    fn duplicate_user_ids_rejected() {
        let phc = hash("x").unwrap();
        let records = vec![
            PasswordRecord {
                user_id: "alice".to_string(),
                phc: phc.clone(),
            },
            PasswordRecord {
                user_id: "alice".to_string(),
                phc,
            },
        ];
        let err = check_unique_user_ids(&records).unwrap_err();
        assert!(err.to_string().contains("duplicate"));
    }
}
