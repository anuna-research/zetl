//! Pluggable authentication for `zetl --collab` (SPEC-041).
//!
//! This module is the single seam through which an inbound request becomes an
//! authenticated [`Principal`]. The authorization layer downstream — SPL policy
//! evaluation, [`crate::user::Role`], `admin_gate`, edit attribution — never
//! asks *how* a request authenticated; it only consumes the `Principal`. That
//! invariant (SPEC-041 §1.2, REQ-4101) is what makes authentication pluggable
//! without touching authorization.
//!
//! Each authentication method is an [`Authenticator`] implementation in its own
//! file (`passkey.rs`, `agent_token.rs`, `proxy_header.rs`, `password.rs`,
//! `oidc.rs`, `capability_url.rs`). At startup the chain builder assembles an
//! ordered `Vec<Box<dyn Authenticator>>` from `[collab.auth] methods`
//! (SPEC-041 ADR-4101), and the `auth_resolve` middleware walks it once per
//! request, first-match-wins (REQ-4104, NFR-4104).
//!
//! This file (task-auth-trait) defines only the trait and its plain-data
//! types — no implementations, no middleware. Those land in later Phase-0
//! tasks.

// The trait and its types are unused until `auth_resolve` and the
// authenticator impls land later in Phase 0; the `allow` is removed at
// task-auth-gate-refactor, when the chain is wired into the router.
#![allow(dead_code)]

pub(crate) mod agent_token;
pub(crate) mod config;
pub(crate) mod passkey;
pub(crate) mod provision;
pub(crate) mod proxy_header;
pub(crate) mod resolve;
pub(crate) mod token;

use std::sync::Arc;

use axum::http::request::Parts;
use axum::Router;

use crate::user::Role;

use super::WebState;

/// The ordered authenticator chain (SPEC-041 ADR-4101).
///
/// Assembled once at startup from `[collab.auth] methods` (the chain builder
/// lands in Phase 1) and walked first-match-wins by the `auth_resolve`
/// middleware. `Arc`-wrapped so it can be cheap middleware state without
/// `WebState` having to carry it — only `auth_resolve` needs the chain;
/// everything downstream reads the resolved [`Principal`] from request
/// extensions.
pub(crate) type AuthChain = Arc<Vec<Box<dyn Authenticator + Send + Sync>>>;

/// The identity an [`Authenticator`] resolves a request to.
///
/// Most methods produce a [`PrincipalId::User`] backed by a `UserProfile`. The
/// capability-URL method (SPEC-041 REQ-4119) produces a
/// [`PrincipalId::Capability`] — a pseudonymous handle that is *not* backed by
/// a profile and is never elevated by auto-provisioning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PrincipalId {
    /// A real account. The string is the `user_id`; it resolves to a
    /// `UserProfile` (possibly auto-provisioned per REQ-4111).
    User(String),
    /// A pseudonymous capability-URL holder (REQ-4119). The string is a
    /// stable, non-guessable handle derived from the capability token's `jti`,
    /// used for attribution and audit only. It is NOT a `user_id` and has no
    /// `UserProfile`.
    Capability(String),
}

impl PrincipalId {
    /// The stable string handle for this identity — a `user_id` for accounts,
    /// a `jti`-derived handle for capability holders. Suitable for audit lines
    /// and attribution, not for `UserProfile` lookup unless [`Self::User`].
    pub(crate) fn handle(&self) -> &str {
        match self {
            PrincipalId::User(id) => id,
            PrincipalId::Capability(handle) => handle,
        }
    }
}

/// The authority a capability-URL token grants (SPEC-041 REQ-4117, CON-4109).
///
/// Present only on a [`Principal`] issued by the capability-URL authenticator.
/// The bound `scope` glob reaches the SPL layer as a `Principal` attribute
/// (ADR-4111); `role` is the role encoded in the token, capped by SPL at the
/// granted scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CapabilityGrant {
    /// Folder/page scope glob the token is bound to.
    pub scope: String,
    /// Role encoded in the token (`reader` or `editor`). A capability
    /// principal never satisfies `admin_gate` regardless of this value
    /// (REQ-4119).
    pub role: Role,
}

/// An authenticated request, produced by exactly one [`Authenticator`] and
/// stashed in the request extensions by the `auth_resolve` middleware
/// (SPEC-041 REQ-4104, CON-4103).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Principal {
    /// Who the request authenticated as.
    pub identity: PrincipalId,
    /// The `id()` of the authenticator that produced this principal — recorded
    /// in the audit trail (REQ-4115).
    pub method: &'static str,
    /// `true` iff this principal is cookie-session-backed, so the CSRF guard
    /// applies (REQ-4112). `false` for stateless/bearer-style methods
    /// (`agent-token`, `proxy-header`, stateless `capability-url`).
    pub cookie_session: bool,
    /// Present only for `capability-url` principals: the bound scope+role
    /// (REQ-4117). `None` for every account-backed method.
    pub capability: Option<CapabilityGrant>,
}

/// A chain-terminating authentication failure.
///
/// `cause` is an operator-channel category only — it is logged (OBS-4103) but
/// MUST NOT reach the user-visible layer, which always sees one generic
/// failure class (SPEC-041 §1.3.8, REQ-4115).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AuthRejection {
    /// Operator-channel cause category (e.g. `"bad-signature"`,
    /// `"rate-limited"`). Never user-visible.
    pub cause: &'static str,
}

/// The result of asking one [`Authenticator`] to inspect a request.
///
/// The `auth_resolve` middleware (CON-4103) walks the chain: it stops on the
/// first [`AuthOutcome::Authenticated`], terminates the chain on
/// [`AuthOutcome::Reject`], and proceeds to the next authenticator on
/// [`AuthOutcome::Abstain`].
#[derive(Debug)]
pub(crate) enum AuthOutcome {
    /// This authenticator recognised and accepted the request.
    Authenticated(Principal),
    /// Not this authenticator's concern — try the next one in the chain.
    /// Implementations MUST NOT mutate server state on this path (NFR-4101).
    Abstain,
    /// A hard, chain-terminating failure (e.g. a present-but-invalid
    /// credential the operator wants to fail closed on).
    Reject(AuthRejection),
}

/// Resolves one inbound request to an authenticated [`Principal`].
///
/// Implementations live one-per-file under `src/web/auth/`. Each is
/// constructed by the chain builder with whatever dependencies it needs
/// (a `SessionStore` clone, the vault root, parsed config), so `authenticate`
/// only ever needs the request [`Parts`] (SPEC-041 CON-4101).
pub(crate) trait Authenticator: Send + Sync {
    /// Stable id used in config `methods` lists and audit logs
    /// (e.g. `"passkey"`, `"agent-token"`, `"oidc"`).
    fn id(&self) -> &'static str;

    /// Inspect the request and decide. MUST be cheap on the
    /// [`AuthOutcome::Abstain`] path (NFR-4101) and MUST NOT mutate server
    /// state when abstaining.
    fn authenticate(&self, parts: &Parts) -> AuthOutcome;

    /// `true` iff the principals this method issues are cookie-session-backed
    /// (the CSRF guard applies). `false` for stateless/bearer-style methods
    /// (REQ-4112). The value MUST match the `cookie_session` field of every
    /// [`Principal`] this authenticator produces.
    fn issues_cookie_session(&self) -> bool;

    /// Where to redirect an unauthenticated *browser* request, if this method
    /// has a login surface. `None` for header/bearer methods that have no UI.
    fn login_redirect(&self) -> Option<&str> {
        None
    }

    /// Public routes this method needs mounted (login pages, OAuth callbacks).
    /// Mounted in the always-public `auth_routes` group, behind the per-IP
    /// rate limiter, never gated by `collab_gate` (SPEC-041 REQ-4113).
    fn routes(&self) -> Router<WebState> {
        Router::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TEST-4101 (type-shape portion): the plain-data types exist, are
    /// constructible, and round-trip through `Clone`/`PartialEq`/`Debug`.
    #[test]
    fn principal_types_are_plain_data() {
        let user = Principal {
            identity: PrincipalId::User("alice-a1b2c3d4".to_string()),
            method: "passkey",
            cookie_session: true,
            capability: None,
        };
        assert_eq!(user.identity.handle(), "alice-a1b2c3d4");
        assert_eq!(user.clone(), user);
        assert!(format!("{user:?}").contains("passkey"));

        let cap = Principal {
            identity: PrincipalId::Capability("cap-deadbeef".to_string()),
            method: "capability-url",
            cookie_session: false,
            capability: Some(CapabilityGrant {
                scope: "review/draft-7/**".to_string(),
                role: Role::Reader,
            }),
        };
        assert_eq!(cap.identity.handle(), "cap-deadbeef");
        assert!(cap.capability.is_some());
        assert_ne!(user, cap);
    }

    /// TEST-4101: `AuthOutcome` carries the three documented variants.
    #[test]
    fn auth_outcome_variants() {
        let authed = AuthOutcome::Authenticated(Principal {
            identity: PrincipalId::User("bob".to_string()),
            method: "agent-token",
            cookie_session: false,
            capability: None,
        });
        let abstain = AuthOutcome::Abstain;
        let reject = AuthOutcome::Reject(AuthRejection {
            cause: "bad-signature",
        });
        // Debug is required on the outcome type (acceptance criterion).
        assert!(format!("{authed:?}").contains("Authenticated"));
        assert!(format!("{abstain:?}").contains("Abstain"));
        assert!(format!("{reject:?}").contains("bad-signature"));
    }
}
