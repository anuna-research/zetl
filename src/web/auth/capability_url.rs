//! Capability-URL authenticator (SPEC-041 REQ-4116, REQ-4117, REQ-4118,
//! REQ-4119, REQ-4120, CON-4109, ADR-4109, ADR-4110, ADR-4111).
//!
//! A capability token is an [`EdDSA`] JWT (`sub = "zetl-capability"`)
//! produced by `zetl collab share`, recognised by the shared
//! [`crate::web::auth::token`] recogniser. The token rides in the URL as
//! `?cap=<token>` (ADR-4110: query, not fragment — a server-side
//! authenticator must see it).
//!
//! Threat model (Threat Model H in the spec): the URL is a [`Bearer Token`].
//! Mitigations live in this module:
//!
//! * **Scope-bound, role-bound, expiring, revocable.** REQ-4117/4118 — a
//!   leaked link exposes only the granted slice, not the whole vault.
//! * **Pseudonymous principal.** REQ-4119 — never auto-provisioned, never
//!   satisfies `admin_gate`, even for an `editor`-role token.
//! * **`Referrer-Policy: no-referrer`.** ADR-4110(c) — the token does not
//!   leak via the `Referer` header to outbound links. Set by a response
//!   middleware in `src/web/mod.rs` for any request whose URL carries a
//!   `cap=` parameter.
//! * **Log redaction.** ADR-4110(d) — the resolved [`Principal`] carries
//!   only the `jti`-derived handle, not the raw token; audit lines key off
//!   the handle.

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::http::request::Parts;
use ed25519_dalek::VerifyingKey;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::user::Role;
use crate::web::auth::token;

use super::{AuthOutcome, Authenticator, CapabilityGrant, Principal, PrincipalId};

const COLLAB_DIR: &str = ".zetl/collab";
const REVOKED_FILE: &str = "cap-revoked.json";
const SHARES_FILE: &str = "cap-shares.json";

/// `[collab.auth.capability_url]` schema. `deny_unknown_fields` so typos
/// fail at parse (REQ-4120/REQ-4114).
#[derive(Deserialize, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct CapabilityUrlConfig {
    /// Default `--expires` for newly-minted tokens. Parsed by [`parse_duration`].
    /// Absent ⇒ 7d.
    #[serde(default)]
    pub default_ttl: Option<String>,
    /// Upper bound on `--expires`. Absent ⇒ 90d.
    #[serde(default)]
    pub max_ttl: Option<String>,
}

/// The claims a capability token carries. Mirrored by `zetl collab share`
/// when minting.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CapabilityClaims {
    /// Token-type discriminator. MUST equal `"zetl-capability"` — the shared
    /// recogniser refuses to hand a caller these claims if `sub` differs
    /// (REQ-4120, ADR-4109).
    pub sub: String,
    /// Folder/page glob the token is bound to (REQ-4117). The unchanged SPL
    /// layer interprets this as a [`Principal`] attribute (ADR-4111).
    pub scope: String,
    /// `"reader"` or `"editor"`. A capability principal never satisfies
    /// `admin_gate` regardless of this value (REQ-4119).
    pub role: String,
    /// Expiry as Unix timestamp seconds (REQ-4117).
    pub exp: u64,
    /// 128-bit hex token id — used for revocation and as the seed for the
    /// pseudonymous principal handle (REQ-4118, REQ-4119).
    pub jti: String,
}

/// Why a capability operation failed.
#[derive(Debug, Error)]
pub enum CapabilityError {
    #[error("capability-URL store I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("capability-URL store JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid capability token: {0}")]
    Token(String),
    #[error("invalid capability role {0:?} — expected \"reader\" or \"editor\"")]
    Role(String),
    #[error("invalid duration {0:?}: {1}")]
    Duration(String, String),
}

// ─── Revocation set persistence (REQ-4118) ─────────────────────────────

/// Return the revocation-set file path for a vault.
pub fn revoked_path(vault_root: &Path) -> PathBuf {
    vault_root.join(COLLAB_DIR).join(REVOKED_FILE)
}

/// Load the revoked-jti set. Absent file ⇒ empty set.
pub fn load_revoked(vault_root: &Path) -> Result<HashSet<String>, CapabilityError> {
    let path = revoked_path(vault_root);
    if !path.exists() {
        return Ok(HashSet::new());
    }
    let body = fs::read_to_string(&path)?;
    let list: Vec<String> = serde_json::from_str(&body)?;
    Ok(list.into_iter().collect())
}

/// Add `jti` to the revoked set, persisting via temp-file rename.
pub fn revoke(vault_root: &Path, jti: &str) -> Result<(), CapabilityError> {
    let dir = vault_root.join(COLLAB_DIR);
    fs::create_dir_all(&dir)?;
    let final_path = revoked_path(vault_root);
    let mut set = load_revoked(vault_root)?;
    set.insert(jti.to_string());
    let mut sorted: Vec<String> = set.into_iter().collect();
    sorted.sort();

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
        let body = serde_json::to_string_pretty(&sorted)?;
        file.write_all(body.as_bytes())?;
        file.sync_all()?;
    }
    fs::rename(&tmp_path, &final_path)?;
    Ok(())
}

// ─── Mint registry (for `share list`) — REQ-4118 supports ──────────────

/// One record of a minted capability — what the operator needs to list,
/// inspect, and revoke. Never stores the token bytes themselves (the
/// jti suffices for revocation; the operator already has the URL).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShareRecord {
    pub jti: String,
    pub scope: String,
    pub role: String,
    /// Unix timestamp seconds (REQ-4117).
    pub exp: u64,
    /// ISO 8601 — when the mint CLI ran.
    pub created_at: String,
}

fn shares_path(vault_root: &Path) -> PathBuf {
    vault_root.join(COLLAB_DIR).join(SHARES_FILE)
}

/// Load the mint registry; absent file ⇒ empty list.
pub fn load_shares(vault_root: &Path) -> Result<Vec<ShareRecord>, CapabilityError> {
    let path = shares_path(vault_root);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let body = fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&body)?)
}

/// Append `record` to the mint registry, persisted via atomic temp-file
/// rename + 0600 (consistent with the revocation set and the password store).
pub fn record_mint(vault_root: &Path, record: &ShareRecord) -> Result<(), CapabilityError> {
    let dir = vault_root.join(COLLAB_DIR);
    fs::create_dir_all(&dir)?;
    let final_path = shares_path(vault_root);
    let mut shares = load_shares(vault_root)?;
    shares.push(record.clone());

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
        let body = serde_json::to_string_pretty(&shares)?;
        file.write_all(body.as_bytes())?;
        file.sync_all()?;
    }
    fs::rename(&tmp_path, &final_path)?;
    Ok(())
}

// ─── Token minting + verification (REQ-4116/4117) ──────────────────────

/// Generate a fresh 128-bit random hex `jti` (token id) — 32 hex chars.
pub fn generate_jti() -> String {
    let mut bytes = [0u8; 16];
    crate::user::getrandom(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Parse a duration string of the form `<N>(s|m|h|d)` into seconds. Used
/// for the `--expires` CLI flag and the `default_ttl`/`max_ttl` config
/// fields. Strict — anything else is rejected (REQ-4120).
pub fn parse_duration(s: &str) -> Result<u64, CapabilityError> {
    if s.is_empty() {
        return Err(CapabilityError::Duration(
            s.to_string(),
            "empty".to_string(),
        ));
    }
    let (digits, unit) = s.split_at(s.len() - 1);
    let n: u64 = digits.parse().map_err(|e: std::num::ParseIntError| {
        CapabilityError::Duration(s.to_string(), e.to_string())
    })?;
    let multiplier: u64 = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 60 * 60,
        "d" => 60 * 60 * 24,
        other => {
            return Err(CapabilityError::Duration(
                s.to_string(),
                format!("unit {other:?} not one of s/m/h/d"),
            ))
        }
    };
    n.checked_mul(multiplier).ok_or_else(|| {
        CapabilityError::Duration(s.to_string(), "duration overflow".to_string())
    })
}

/// REQ-4117 TTL bounds enforced by the mint CLI.
pub const TTL_MIN_SECONDS: u64 = 5 * 60; // 5 minutes
pub const TTL_MAX_SECONDS: u64 = 90 * 24 * 60 * 60; // 90 days
pub const TTL_DEFAULT_SECONDS: u64 = 7 * 24 * 60 * 60; // 7 days

/// Mint a capability token (REQ-4116). Returns `(token, claims)` so the
/// caller can present both the URL and a structured summary.
pub fn mint(
    signing_key: &ed25519_dalek::SigningKey,
    scope: &str,
    role: &str,
    ttl_seconds: u64,
) -> Result<(String, CapabilityClaims), CapabilityError> {
    if role != "reader" && role != "editor" {
        return Err(CapabilityError::Role(role.to_string()));
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let claims = CapabilityClaims {
        sub: "zetl-capability".to_string(),
        scope: scope.to_string(),
        role: role.to_string(),
        exp: now + ttl_seconds,
        jti: generate_jti(),
    };
    let token = token::encode(signing_key, &claims)
        .map_err(|e| CapabilityError::Token(e.to_string()))?;
    Ok((token, claims))
}

/// Verify a capability token against `verifying_key`, returning the claims
/// on a successful recognition. The caller checks `exp` and revocation
/// separately so a single helper can be reused in tests.
pub fn verify_claims(
    verifying_key: &VerifyingKey,
    raw_token: &str,
) -> Result<CapabilityClaims, CapabilityError> {
    let jwt = token::recognise(raw_token, verifying_key)
        .map_err(|e| CapabilityError::Token(e.to_string()))?;
    let claims: CapabilityClaims = jwt
        .claims("zetl-capability")
        .map_err(|e| CapabilityError::Token(e.to_string()))?;
    Ok(claims)
}

/// Derive the pseudonymous principal handle from a `jti` — REQ-4119.
///
/// The handle is stable across requests for the same token (audit
/// continuity) but not guessable from a different token's handle. We use
/// the first 12 hex chars of the jti, prefixed `cap-`. That gives 48 bits
/// of distinguishing entropy — enough for audit-line uniqueness, while
/// still keeping the handle short and recognisable in logs.
pub fn handle_from_jti(jti: &str) -> String {
    let prefix: String = jti.chars().take(12).collect();
    format!("cap-{prefix}")
}

// ─── The authenticator ─────────────────────────────────────────────────

/// The capability-URL authenticator.
pub(crate) struct CapabilityUrlAuthenticator {
    verifying_key: VerifyingKey,
    vault_root: Arc<PathBuf>,
}

impl CapabilityUrlAuthenticator {
    pub(crate) fn new(verifying_key: VerifyingKey, vault_root: Arc<PathBuf>) -> Self {
        Self {
            verifying_key,
            vault_root,
        }
    }
}

impl Authenticator for CapabilityUrlAuthenticator {
    fn id(&self) -> &'static str {
        "capability-url"
    }

    fn authenticate(&self, parts: &Parts) -> AuthOutcome {
        // 1. Pull `cap=<token>` from the query string. Absent ⇒ abstain.
        let token_value = match extract_cap_query_value(parts.uri.query()) {
            Some(v) => v,
            None => return AuthOutcome::Abstain,
        };

        // 2. Recognise + signature-verify + sub-check via the shared
        //    EdDSA-JWT recogniser. Any failure here ⇒ Abstain (CON-4109:
        //    "generic — not Reject, so a stale link does not poison the
        //    chain for a user who also holds another credential").
        let claims = match verify_claims(&self.verifying_key, &token_value) {
            Ok(c) => c,
            Err(_) => return AuthOutcome::Abstain,
        };

        // 3. Role must be a known role string.
        let role = match claims.role.as_str() {
            "reader" => Role::Reader,
            "editor" => Role::Editor,
            _ => return AuthOutcome::Abstain,
        };

        // 4. Expiry check (REQ-4117).
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if now >= claims.exp {
            return AuthOutcome::Abstain;
        }

        // 5. Revocation check (REQ-4118).
        match load_revoked(&self.vault_root) {
            Ok(set) if set.contains(&claims.jti) => return AuthOutcome::Abstain,
            Err(_) => return AuthOutcome::Abstain,
            Ok(_) => { /* not revoked, proceed */ }
        }

        // 6. Issue a pseudonymous, scope-capped principal (REQ-4119).
        let handle = handle_from_jti(&claims.jti);
        AuthOutcome::Authenticated(Principal {
            identity: PrincipalId::Capability(handle),
            method: "capability-url",
            cookie_session: false,
            capability: Some(CapabilityGrant {
                scope: claims.scope,
                role,
            }),
        })
    }

    fn issues_cookie_session(&self) -> bool {
        false
    }
}

/// Extract the value of the `cap` query parameter from a raw query string.
/// Returns `None` if absent or if the parameter has an empty value.
///
/// Strict-ish: accepts `cap=<value>` anywhere in the standard
/// `&`-separated query (matches the typical browser/server behaviour for
/// query parsing). The token is otherwise opaque here — the JWT recogniser
/// is what validates its grammar.
pub fn extract_cap_query_value(query: Option<&str>) -> Option<String> {
    let q = query?;
    for pair in q.split('&') {
        if let Some(value) = pair.strip_prefix("cap=") {
            if value.is_empty() {
                return None;
            }
            // URL-decode the value — browsers may percent-encode JWT chars
            // (though `-`, `_`, and `.` are unreserved).
            return Some(percent_decode(value));
        }
    }
    None
}

/// Minimal percent-decode used by `extract_cap_query_value`. Tolerant —
/// malformed escapes leave the raw bytes through so the downstream JWT
/// recogniser rejects them.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (
                (bytes[i + 1] as char).to_digit(16),
                (bytes[i + 2] as char).to_digit(16),
            ) {
                out.push(((hi << 4) | lo) as u8 as char);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Returns `true` iff the request URL carries any `cap=` parameter — used
/// by the response middleware that sets `Referrer-Policy: no-referrer`
/// (ADR-4110(c)). Cheap query-string scan; does not validate the token.
pub fn request_has_cap_param(uri: &axum::http::Uri) -> bool {
    extract_cap_query_value(uri.query()).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    fn test_key() -> SigningKey {
        SigningKey::from_bytes(&[42u8; 32])
    }

    /// REQ-4116/4117: mint produces a token that verifies and carries the
    /// requested scope/role/ttl.
    #[test]
    fn mint_and_verify_roundtrip() {
        let key = test_key();
        let (token, claims_minted) =
            mint(&key, "review/draft-7/**", "reader", 60).unwrap();
        let claims = verify_claims(&key.verifying_key(), &token).unwrap();
        assert_eq!(claims.sub, "zetl-capability");
        assert_eq!(claims.scope, "review/draft-7/**");
        assert_eq!(claims.role, "reader");
        assert_eq!(claims.jti, claims_minted.jti);
        assert!(claims.exp > 0);
    }

    /// REQ-4117: a role outside {reader, editor} is rejected at mint time.
    #[test]
    fn mint_rejects_unknown_role() {
        let key = test_key();
        let err = mint(&key, "x", "admin", 60).unwrap_err();
        assert!(matches!(err, CapabilityError::Role(_)));
    }

    /// REQ-4120 / ADR-4109: a token signed by a different key fails
    /// verification (the shared recogniser's signature check).
    #[test]
    fn verify_rejects_wrong_key() {
        let signer = test_key();
        let other = SigningKey::from_bytes(&[9u8; 32]);
        let (token, _) = mint(&signer, "x", "reader", 60).unwrap();
        assert!(verify_claims(&other.verifying_key(), &token).is_err());
    }

    /// REQ-4120 / ADR-4109: a token with the wrong `sub` is rejected by the
    /// sub-discriminator check in the shared recogniser.
    #[test]
    fn verify_rejects_wrong_sub() {
        let key = test_key();
        // Forge a JWT with sub="zetl-invite" but signed with the same key.
        let forged = token::encode(
            &key,
            &serde_json::json!({"sub": "zetl-invite", "exp": u64::MAX, "iss": "x"}),
        )
        .unwrap();
        let err = verify_claims(&key.verifying_key(), &forged).unwrap_err();
        assert!(matches!(err, CapabilityError::Token(_)));
    }

    /// REQ-4119: handle is stable, derived from jti, and short.
    #[test]
    fn handle_from_jti_is_stable_and_short() {
        let jti = "0123456789abcdef0123456789abcdef";
        let h1 = handle_from_jti(jti);
        let h2 = handle_from_jti(jti);
        assert_eq!(h1, h2);
        assert_eq!(h1, "cap-0123456789ab");
        // Different jti ⇒ different handle.
        let other = handle_from_jti("ffffffffffffffffffffffffffffffff");
        assert_ne!(h1, other);
    }

    /// `parse_duration` — happy + negative.
    #[test]
    fn parse_duration_units() {
        assert_eq!(parse_duration("60s").unwrap(), 60);
        assert_eq!(parse_duration("5m").unwrap(), 5 * 60);
        assert_eq!(parse_duration("2h").unwrap(), 2 * 60 * 60);
        assert_eq!(parse_duration("7d").unwrap(), 7 * 24 * 60 * 60);
    }
    #[test]
    fn parse_duration_rejects_garbage() {
        for bad in ["", "5", "5y", "abc", "-5d"] {
            assert!(
                parse_duration(bad).is_err(),
                "should reject {bad:?}"
            );
        }
    }

    /// `extract_cap_query_value` — happy + absent + empty + URL-decode.
    #[test]
    fn extract_cap_value_basic() {
        assert_eq!(
            extract_cap_query_value(Some("cap=abc.def.ghi")),
            Some("abc.def.ghi".to_string())
        );
        assert_eq!(
            extract_cap_query_value(Some("foo=bar&cap=xyz&baz=qux")),
            Some("xyz".to_string())
        );
        assert_eq!(extract_cap_query_value(Some("cap=")), None);
        assert_eq!(extract_cap_query_value(Some("foo=bar")), None);
        assert_eq!(extract_cap_query_value(None), None);
        // URL-decode (`%2E` → `.`).
        assert_eq!(
            extract_cap_query_value(Some("cap=abc%2Edef")),
            Some("abc.def".to_string())
        );
    }

    /// REQ-4116/4117/4119: a valid `?cap=<token>` from a URI produces a
    /// pseudonymous scope-capped principal.
    #[test]
    fn authenticate_valid_token_yields_capability_principal() {
        use axum::extract::Request;
        let tmp = tempfile::TempDir::new().unwrap();
        let key = test_key();
        let (token, claims) =
            mint(&key, "shared/draft/**", "reader", 60).unwrap();

        let auth = CapabilityUrlAuthenticator::new(
            key.verifying_key(),
            Arc::new(tmp.path().to_path_buf()),
        );

        let mut req: Request<()> = Request::new(());
        *req.uri_mut() = format!("/shared/draft/page?cap={token}").parse().unwrap();
        let parts = req.into_parts().0;

        match auth.authenticate(&parts) {
            AuthOutcome::Authenticated(p) => {
                assert_eq!(p.method, "capability-url");
                assert!(!p.cookie_session); // REQ-4112 stateless
                match &p.identity {
                    PrincipalId::Capability(h) => {
                        assert_eq!(h, &handle_from_jti(&claims.jti));
                    }
                    other => panic!("expected Capability handle, got {other:?}"),
                }
                let grant = p.capability.expect("grant present");
                assert_eq!(grant.scope, "shared/draft/**");
                assert_eq!(grant.role, Role::Reader);
            }
            other => panic!("expected Authenticated, got {other:?}"),
        }
    }

    /// No `?cap=` ⇒ Abstain.
    #[test]
    fn authenticate_no_cap_query_abstains() {
        use axum::extract::Request;
        let tmp = tempfile::TempDir::new().unwrap();
        let key = test_key();
        let auth = CapabilityUrlAuthenticator::new(
            key.verifying_key(),
            Arc::new(tmp.path().to_path_buf()),
        );
        let req: Request<()> = Request::new(());
        let parts = req.into_parts().0;
        assert!(matches!(auth.authenticate(&parts), AuthOutcome::Abstain));
    }

    /// REQ-4117: expired token ⇒ Abstain.
    #[test]
    fn authenticate_expired_token_abstains() {
        use axum::extract::Request;
        let tmp = tempfile::TempDir::new().unwrap();
        let key = test_key();
        // Mint with negative TTL (well, 0 then we'll re-sign claims with
        // exp in the past). Simulate via direct encode.
        let claims = CapabilityClaims {
            sub: "zetl-capability".to_string(),
            scope: "x".to_string(),
            role: "reader".to_string(),
            exp: 1, // 1970-ish, definitely past
            jti: generate_jti(),
        };
        let token = token::encode(&key, &claims).unwrap();
        let auth = CapabilityUrlAuthenticator::new(
            key.verifying_key(),
            Arc::new(tmp.path().to_path_buf()),
        );
        let mut req: Request<()> = Request::new(());
        *req.uri_mut() = format!("/x?cap={token}").parse().unwrap();
        let parts = req.into_parts().0;
        assert!(matches!(auth.authenticate(&parts), AuthOutcome::Abstain));
    }

    /// REQ-4118: revoked jti ⇒ Abstain even with a structurally-valid token.
    #[test]
    fn authenticate_revoked_token_abstains() {
        use axum::extract::Request;
        let tmp = tempfile::TempDir::new().unwrap();
        let key = test_key();
        let (token, claims) = mint(&key, "x", "reader", 60).unwrap();

        // Revoke it.
        revoke(tmp.path(), &claims.jti).unwrap();

        let auth = CapabilityUrlAuthenticator::new(
            key.verifying_key(),
            Arc::new(tmp.path().to_path_buf()),
        );
        let mut req: Request<()> = Request::new(());
        *req.uri_mut() = format!("/x?cap={token}").parse().unwrap();
        let parts = req.into_parts().0;
        assert!(matches!(auth.authenticate(&parts), AuthOutcome::Abstain));
    }

    /// REQ-4118: revocation persists across `load_revoked` calls.
    #[test]
    fn revocation_persists() {
        let tmp = tempfile::TempDir::new().unwrap();
        revoke(tmp.path(), "abc").unwrap();
        revoke(tmp.path(), "def").unwrap();
        let set = load_revoked(tmp.path()).unwrap();
        assert!(set.contains("abc"));
        assert!(set.contains("def"));
    }

    /// REQ-4119: an `editor`-role capability principal still doesn't gain
    /// elevated rights — the role is recorded but `admin_gate` will reject
    /// (verified in admin_gate's own test that capability principals are
    /// forbidden).
    #[test]
    fn editor_role_still_stateless_pseudonymous() {
        use axum::extract::Request;
        let tmp = tempfile::TempDir::new().unwrap();
        let key = test_key();
        let (token, _) = mint(&key, "x/**", "editor", 60).unwrap();
        let auth = CapabilityUrlAuthenticator::new(
            key.verifying_key(),
            Arc::new(tmp.path().to_path_buf()),
        );
        let mut req: Request<()> = Request::new(());
        *req.uri_mut() = format!("/x?cap={token}").parse().unwrap();
        let parts = req.into_parts().0;
        match auth.authenticate(&parts) {
            AuthOutcome::Authenticated(p) => {
                assert!(matches!(p.identity, PrincipalId::Capability(_)));
                assert_eq!(p.capability.unwrap().role, Role::Editor);
                assert!(!p.cookie_session);
            }
            other => panic!("expected Authenticated, got {other:?}"),
        }
    }

    /// REQ-4112 declarations.
    #[test]
    fn declares_stateless_no_login_no_routes() {
        let tmp = tempfile::TempDir::new().unwrap();
        let key = test_key();
        let auth = CapabilityUrlAuthenticator::new(
            key.verifying_key(),
            Arc::new(tmp.path().to_path_buf()),
        );
        assert!(!auth.issues_cookie_session());
        assert_eq!(auth.login_redirect(), None);
        assert_eq!(auth.id(), "capability-url");
    }

    /// ADR-4110: detection helper used by the response-middleware that sets
    /// `Referrer-Policy: no-referrer`.
    #[test]
    fn request_has_cap_param_detects() {
        let with: axum::http::Uri = "/x?cap=abc".parse().unwrap();
        let without: axum::http::Uri = "/x".parse().unwrap();
        assert!(request_has_cap_param(&with));
        assert!(!request_has_cap_param(&without));
    }
}
