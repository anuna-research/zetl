//! Passkey + session [`Authenticator`] adapter (SPEC-041 task-auth-passkey-impl).
//!
//! Wraps the existing `SessionStore` validation in
//! `src/web/session.rs` behind the [`Authenticator`] trait so the chain in
//! `auth_resolve` can recognise the `zetl_session` cookie the way today's
//! `collab_gate` does — bit-for-bit unchanged. No crypto, no storage, no
//! protocol change; this file is purely an adapter.
//!
//! The passkey *routes* themselves (`/auth/login`, `/api/passkey/auth/*`,
//! `/api/passkey/register/*`, `/auth/recover`, etc.) continue to live in
//! `src/web/mod.rs`'s `auth_routes` group for Phase 0; this authenticator's
//! `routes()` is empty. Moving them under `Authenticator::routes()` is a
//! later-phase refinement that doesn't affect the resolution semantics this
//! task delivers.

use axum::http::request::Parts;

use crate::web::session::{token_from_cookies, SessionStore};

use super::{AuthOutcome, Authenticator, Principal, PrincipalId};

/// Cookie-session authenticator: a valid `zetl_session` cookie ⇒ a
/// [`Principal`] for the user it identifies.
///
/// Treats a present-but-invalid cookie (expired, unknown, malformed) as
/// [`AuthOutcome::Abstain`] rather than [`AuthOutcome::Reject`]. That matches
/// today's `collab_gate` behaviour: a stale cookie does not 401 — it falls
/// through to the redirect/login path. Reject is reserved for failures the
/// operator genuinely wants to fail closed on; an expired cookie is not one.
pub(crate) struct PasskeyAuthenticator {
    sessions: SessionStore,
}

impl PasskeyAuthenticator {
    pub(crate) fn new(sessions: SessionStore) -> Self {
        Self { sessions }
    }
}

impl Authenticator for PasskeyAuthenticator {
    fn id(&self) -> &'static str {
        "passkey"
    }

    fn authenticate(&self, parts: &Parts) -> AuthOutcome {
        let Some(token) = token_from_cookies(&parts.headers) else {
            return AuthOutcome::Abstain;
        };
        let Some(user_id) = self.sessions.validate(&token) else {
            // Cookie present but invalid (expired, unknown, etc.) — abstain so
            // the gate produces the standard login redirect, matching the
            // pre-SPEC-041 behaviour.
            return AuthOutcome::Abstain;
        };
        AuthOutcome::Authenticated(Principal {
            identity: PrincipalId::User(user_id),
            method: "passkey",
            cookie_session: true,
            capability: None,
        })
    }

    fn issues_cookie_session(&self) -> bool {
        true
    }

    fn login_redirect(&self) -> Option<&str> {
        Some("/auth/login")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::Request;
    use axum::http::header;

    fn parts_with_cookie(value: &str) -> Parts {
        let mut req: Request<()> = Request::new(());
        req.headers_mut()
            .insert(header::COOKIE, value.parse().unwrap());
        req.into_parts().0
    }

    /// TEST-4101 (passkey portion): a valid `zetl_session` cookie produces a
    /// User principal whose `method` and `cookie_session` match the trait.
    #[test]
    fn valid_cookie_authenticates() {
        let store = SessionStore::new();
        let token = store.create("alice");
        let auth = PasskeyAuthenticator::new(store);
        let parts = parts_with_cookie(&format!("zetl_session={token}"));

        match auth.authenticate(&parts) {
            AuthOutcome::Authenticated(p) => {
                assert_eq!(p.identity, PrincipalId::User("alice".to_string()));
                assert_eq!(p.method, "passkey");
                assert!(p.cookie_session);
                assert!(p.capability.is_none());
            }
            other => panic!("expected Authenticated, got {other:?}"),
        }
    }

    /// TEST-4101 (passkey portion): no cookie ⇒ Abstain (the chain proceeds).
    #[test]
    fn no_cookie_abstains() {
        let store = SessionStore::new();
        let auth = PasskeyAuthenticator::new(store);
        let parts = Request::new(()).into_parts().0;
        assert!(matches!(auth.authenticate(&parts), AuthOutcome::Abstain));
    }

    /// A cookie that doesn't validate (unknown / destroyed session) abstains —
    /// it does NOT Reject. This matches today's `collab_gate`: a stale cookie
    /// is treated as "no credential", not as "credential failed", so the
    /// login redirect path runs.
    #[test]
    fn invalid_cookie_abstains_not_rejects() {
        let store = SessionStore::new();
        let token = store.create("alice");
        store.destroy(&token);
        let auth = PasskeyAuthenticator::new(store);
        let parts = parts_with_cookie(&format!("zetl_session={token}"));
        assert!(matches!(auth.authenticate(&parts), AuthOutcome::Abstain));
    }

    /// REQ-4112: the passkey adapter declares cookie-session-backed.
    #[test]
    fn declares_cookie_session() {
        let auth = PasskeyAuthenticator::new(SessionStore::new());
        assert!(auth.issues_cookie_session());
        assert_eq!(auth.login_redirect(), Some("/auth/login"));
        assert_eq!(auth.id(), "passkey");
    }
}
