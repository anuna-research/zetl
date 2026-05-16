//! Agent-token [`Authenticator`] adapter (SPEC-041 task-auth-agent-token-impl).
//!
//! Wraps the existing `verify_bearer_token` logic in `src/web/session.rs`
//! behind the [`Authenticator`] trait. The agent-token *format* is a bespoke
//! `base64url(user_id || generation || ed25519_sig)` binary — see
//! `src/user/agent_token.rs` — **not** an EdDSA JWT, so it does not route
//! through the shared `auth/token.rs` recogniser despite the IMPL-041 task
//! note. One parser per format (REQ-4120) is honoured: `agent_token.rs`
//! remains the sole recogniser for this format, and this file is a thin
//! adapter on top.
//!
//! Stateless principal (`issues_cookie_session() == false`) — generalises the
//! pre-SPEC-041 Bearer-only CSRF exemption to every stateless method
//! (REQ-4112). No login redirect, no routes.

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::request::Parts;

use crate::web::session::{bearer_token_from_headers, verify_bearer_token};

use super::{AuthOutcome, Authenticator, Principal, PrincipalId};

/// Bearer-token authenticator: a valid `Authorization: Bearer <agent-token>`
/// header ⇒ a stateless [`Principal`] for the user it identifies.
pub(crate) struct AgentTokenAuthenticator {
    /// Vault root, needed to load the `UserProfile` whose `recovery_pubkey`
    /// verifies the token.
    vault_root: Arc<PathBuf>,
}

impl AgentTokenAuthenticator {
    pub(crate) fn new(vault_root: Arc<PathBuf>) -> Self {
        Self { vault_root }
    }
}

impl Authenticator for AgentTokenAuthenticator {
    fn id(&self) -> &'static str {
        "agent-token"
    }

    fn authenticate(&self, parts: &Parts) -> AuthOutcome {
        let Some(token) = bearer_token_from_headers(&parts.headers) else {
            return AuthOutcome::Abstain;
        };
        let Some(user_id) = verify_bearer_token(&self.vault_root, &token) else {
            // Present but invalid — abstain so the chain proceeds; the
            // pre-SPEC-041 `collab_gate` already treated bad bearer tokens as
            // "no credential", not as a hard reject.
            return AuthOutcome::Abstain;
        };
        AuthOutcome::Authenticated(Principal {
            identity: PrincipalId::User(user_id),
            method: "agent-token",
            cookie_session: false,
            capability: None,
        })
    }

    fn issues_cookie_session(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::Request;
    use axum::http::header;

    use crate::user::{recovery, save_profile, UserProfile};

    fn parts_with_auth(value: &str) -> Parts {
        let mut req: Request<()> = Request::new(());
        req.headers_mut()
            .insert(header::AUTHORIZATION, value.parse().unwrap());
        req.into_parts().0
    }

    fn make_profile(tmp: &std::path::Path, owner: bool) -> (UserProfile, String) {
        let kp = recovery::generate_recovery_keypair().unwrap();
        let profile = UserProfile {
            id: "agent-test-a1b2c3d4".to_string(),
            name: "Agent Test".to_string(),
            created_at: "2026-05-15T10:00:00Z".to_string(),
            invited_by: None,
            owner,
            credentials: vec![],
            recovery_pubkey: kp.recovery_pubkey.clone(),
            agent_token_generation: 0,
        };
        save_profile(tmp, &profile).unwrap();
        (profile, kp.mnemonic.clone())
    }

    /// TEST-4101 (agent-token portion): a valid Bearer token produces a
    /// stateless User principal — matching `verify_bearer_token`'s acceptance
    /// today.
    #[test]
    fn valid_bearer_authenticates_stateless() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (profile, mnemonic) = make_profile(tmp.path(), true);
        let token =
            crate::user::agent_token::generate_agent_token(&mnemonic, &profile.id, 0).unwrap();
        let auth = AgentTokenAuthenticator::new(Arc::new(tmp.path().to_path_buf()));
        let parts = parts_with_auth(&format!("Bearer {token}"));

        match auth.authenticate(&parts) {
            AuthOutcome::Authenticated(p) => {
                assert_eq!(p.identity, PrincipalId::User(profile.id));
                assert_eq!(p.method, "agent-token");
                assert!(!p.cookie_session); // TEST-4112 stateless portion
                assert!(p.capability.is_none());
            }
            other => panic!("expected Authenticated, got {other:?}"),
        }
    }

    /// TEST-4101 (agent-token portion): no Authorization header ⇒ Abstain.
    #[test]
    fn no_authorization_abstains() {
        let tmp = tempfile::TempDir::new().unwrap();
        let auth = AgentTokenAuthenticator::new(Arc::new(tmp.path().to_path_buf()));
        let parts = Request::new(()).into_parts().0;
        assert!(matches!(auth.authenticate(&parts), AuthOutcome::Abstain));
    }

    /// A Bearer token that doesn't verify (wrong generation, bad signature,
    /// unknown user) abstains — does not Reject. Matches the pre-SPEC-041
    /// behaviour where `collab_gate` falls through to the next credential.
    #[test]
    fn invalid_bearer_abstains_not_rejects() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (profile, mnemonic) = make_profile(tmp.path(), true);
        // Token at generation 0 but bump the profile to generation 1.
        let mut bumped = profile.clone();
        bumped.agent_token_generation = 1;
        save_profile(tmp.path(), &bumped).unwrap();
        let stale =
            crate::user::agent_token::generate_agent_token(&mnemonic, &profile.id, 0).unwrap();

        let auth = AgentTokenAuthenticator::new(Arc::new(tmp.path().to_path_buf()));
        let parts = parts_with_auth(&format!("Bearer {stale}"));
        assert!(matches!(auth.authenticate(&parts), AuthOutcome::Abstain));
    }

    /// REQ-4112: the agent-token adapter declares stateless and has no login
    /// surface.
    #[test]
    fn declares_stateless_no_login_no_routes() {
        let tmp = tempfile::TempDir::new().unwrap();
        let auth = AgentTokenAuthenticator::new(Arc::new(tmp.path().to_path_buf()));
        assert!(!auth.issues_cookie_session());
        assert_eq!(auth.login_redirect(), None);
        assert_eq!(auth.id(), "agent-token");
    }
}
