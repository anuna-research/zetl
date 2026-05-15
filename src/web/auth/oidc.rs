//! OIDC / OAuth2 authenticator (SPEC-041 REQ-4109, REQ-4110, REQ-4113,
//! CON-4104, ADR-4104, ADR-4105).
//!
//! Phase 5a (task-auth-oidc-feature) — this file is **feature-gated**
//! behind `--features collab-oidc` (ADR-4105). It currently declares the
//! authenticator surface and the `[collab.auth.oidc]` config schema so the
//! chain builder under `--features collab-oidc` can construct it; the
//! Authorization-Code + PKCE + ID-token-validation flow lands in Phase 5b
//! (task-auth-oidc-impl) along with the `openidconnect` / `reqwest` deps.
//!
//! Until Phase 5b lands, [`OidcAuthenticator::authenticate`] always abstains
//! — an OIDC-minted [`SessionStore`] session (Phase 5b will mint one in the
//! callback handler) is observed by the existing `PasskeyAuthenticator` /
//! `PasswordAuthenticator` cookie path, so the *session* side already works
//! today; only the *initial login* requires Phase 5b.

#![cfg(feature = "collab-oidc")]

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::request::Parts;
use serde::Deserialize;

use crate::web::session::SessionStore;

use super::{AuthOutcome, Authenticator};

/// `[collab.auth.oidc]` schema (CON-4104).
///
/// `deny_unknown_fields` so a typo at the schema is a startup error
/// (REQ-4120). Phase 5b's authenticator consumes these fields to drive the
/// real flow; Phase 5a only parses them.
#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct OidcConfig {
    /// Pinned issuer URL (REQ-4110). The ID token's `iss` claim must match
    /// this string byte-for-byte.
    pub issuer: String,

    /// Registered client id at the IdP.
    pub client_id: String,

    /// Path to a file containing the client secret. Per ADR-4105 / Threat
    /// Model G, the secret is NEVER inlined in `[collab.auth.oidc]` —
    /// keeping it in a separate file makes it auditable and rotatable.
    pub client_secret_file: String,

    /// ID-token claim whose value becomes the zetl `user_id` (REQ-4109).
    /// Default `"email"`.
    #[serde(default = "default_user_id_claim")]
    pub user_id_claim: String,

    /// Opt-in auto-provisioning (REQ-4111, ADR-4107).
    #[serde(default)]
    pub auto_provision: bool,

    /// Domain allowlist for auto-provisioning. Required when
    /// `auto_provision = true` (validated in Phase 5b's chain builder).
    #[serde(default)]
    pub provision_domain_allow: Option<Vec<String>>,
}

fn default_user_id_claim() -> String {
    "email".to_string()
}

/// The OIDC authenticator. In Phase 5a it is a placeholder that
/// participates in the chain (so `oidc` no longer errors at startup under
/// `--features collab-oidc`) but always abstains — Phase 5b implements
/// `authenticate` against the cookie-session minted by the `/auth/oidc/callback`
/// handler.
pub(crate) struct OidcAuthenticator {
    // Carried for Phase 5b; not yet consumed.
    #[allow(dead_code)]
    cfg: OidcConfig,
    #[allow(dead_code)]
    sessions: SessionStore,
    #[allow(dead_code)]
    vault_root: Arc<PathBuf>,
}

impl OidcAuthenticator {
    pub(crate) fn new(
        cfg: OidcConfig,
        sessions: SessionStore,
        vault_root: Arc<PathBuf>,
    ) -> Self {
        Self {
            cfg,
            sessions,
            vault_root,
        }
    }
}

impl Authenticator for OidcAuthenticator {
    fn id(&self) -> &'static str {
        "oidc"
    }

    fn authenticate(&self, _parts: &Parts) -> AuthOutcome {
        // Phase 5b: read the `zetl_session` cookie, validate against
        // `SessionStore`, return Authenticated{method="oidc"}. For Phase 5a
        // we abstain — an OIDC-minted session (when Phase 5b lands) is
        // still observed by the PasskeyAuthenticator's cookie path, so
        // request-time auth doesn't actually fail; only the
        // /auth/oidc/login → IdP → /auth/oidc/callback path is pending.
        AuthOutcome::Abstain
    }

    fn issues_cookie_session(&self) -> bool {
        true
    }

    fn login_redirect(&self) -> Option<&str> {
        Some("/auth/oidc/login")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// REQ-4109 / CON-4104: the OIDC config parses with sensible defaults
    /// (user_id_claim = "email"; auto_provision = false).
    #[test]
    fn oidc_config_defaults() {
        let body = r#"
            issuer = "https://accounts.example.com"
            client_id = "abc123"
            client_secret_file = "/etc/zetl/oidc-secret"
        "#;
        let cfg: OidcConfig = toml::from_str(body).unwrap();
        assert_eq!(cfg.issuer, "https://accounts.example.com");
        assert_eq!(cfg.client_id, "abc123");
        assert_eq!(cfg.user_id_claim, "email");
        assert!(!cfg.auto_provision);
        assert!(cfg.provision_domain_allow.is_none());
    }

    /// REQ-4120: typo at the schema fails parse (deny_unknown_fields).
    #[test]
    fn oidc_config_rejects_typo() {
        let body = r#"
            issuer = "https://x"
            client_id = "x"
            client_secret_file = "/tmp/s"
            issuerr = "https://typo"
        "#;
        let err = toml::from_str::<OidcConfig>(body).unwrap_err();
        assert!(
            err.to_string().contains("issuerr") || err.to_string().contains("unknown"),
            "expected typo to be named in error: {err}"
        );
    }

    /// The Phase 5a stub abstains so it can sit in the chain without
    /// claiming requests it doesn't actually authenticate. Phase 5b
    /// replaces the body of `authenticate`.
    #[test]
    fn authenticator_abstains_in_phase_5a() {
        use axum::extract::Request;
        use tempfile::TempDir;
        let tmp = TempDir::new().unwrap();
        let cfg = OidcConfig {
            issuer: "https://x".to_string(),
            client_id: "x".to_string(),
            client_secret_file: "/tmp/s".to_string(),
            user_id_claim: "email".to_string(),
            auto_provision: false,
            provision_domain_allow: None,
        };
        let auth = OidcAuthenticator::new(
            cfg,
            SessionStore::new(),
            Arc::new(tmp.path().to_path_buf()),
        );
        let parts = Request::<()>::new(()).into_parts().0;
        assert!(matches!(auth.authenticate(&parts), AuthOutcome::Abstain));
        // REQ-4112 declarations.
        assert!(auth.issues_cookie_session());
        assert_eq!(auth.id(), "oidc");
        assert_eq!(auth.login_redirect(), Some("/auth/oidc/login"));
    }
}
