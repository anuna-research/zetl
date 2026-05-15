//! Auto-provisioning for authenticators that can present a principal with no
//! pre-existing `UserProfile` (SPEC-041 REQ-4111, ADR-4107).
//!
//! Shared by Phase 2 (`proxy-header`) and Phase 5 (`oidc`). Capability-URL
//! principals are **never** routed through here — they are pseudonymous and
//! never backed by a profile (REQ-4119, ADR-4111).
//!
//! Policy invariants (ADR-4107 — least privilege):
//!
//! * **Opt-in.** `enabled = false` by default; an unknown principal is
//!   rejected.
//! * **Reader ceiling.** A newly-provisioned profile is created with
//!   `owner = false` and no override entry in `access.spl`, which makes
//!   [`crate::user::Role::for_profile_with_vault`] return [`Role::Reader`].
//!   Elevation to Editor/Admin is an explicit operator action — this module
//!   has no path to a higher role.
//! * **Domain gate (OIDC).** When an identity-claim domain allowlist is
//!   present, the user_id's `@`-tail must match an entry. Proxy-header does
//!   not use this gate.
//!
//! The function is intentionally idempotent: an existing profile passes
//! through unchanged. Only the "first-seen, allowed to provision" path
//! creates a profile.

use std::path::Path;

use crate::user::{access_request::now_iso8601, load_profile, save_profile, UserProfile};

/// Per-method auto-provisioning policy.
#[derive(Debug, Clone, Default)]
pub(crate) struct ProvisionPolicy {
    /// Operator opt-in. When `false`, an unknown principal is denied (the
    /// operator must pre-create the profile to grant access).
    pub enabled: bool,
    /// Identity-claim domain allowlist. `Some(list)` ⇒ the user_id's
    /// `@`-tail must be in `list` for provisioning to proceed. `None` ⇒ no
    /// domain check (proxy-header semantics). REQ-4111.
    pub domain_allow: Option<Vec<String>>,
}

/// The result of asking `ensure_user` whether this principal may be admitted.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ProvisionOutcome {
    /// The user_id resolves to a profile (pre-existing or just created).
    Allowed,
    /// Provisioning was denied. The variant carries the operator-channel
    /// cause for the audit log (REQ-4115) — the user-visible result is
    /// the same generic class regardless of the cause.
    Denied(DenyReason),
}

/// Why a principal was denied admission. Operator-channel only — never
/// surfaced to the user (SPEC-041 §1.3.8).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DenyReason {
    /// No existing profile and `auto_provision` is off.
    UnknownUserAndAutoProvisionDisabled,
    /// `auto_provision` is on, but the identity-claim domain is not in
    /// `provision_domain_allow`. Carries the rejected domain (operator log
    /// only).
    DomainNotInAllowlist(String),
    /// The user_id has no `@` separator but a domain allowlist was required.
    NoDomainForAllowlistCheck,
    /// Filesystem failure reading or writing the profile.
    IoError(String),
}

/// Resolve a principal's `user_id` to a profile, provisioning one at Reader
/// if (and only if) policy allows.
///
/// `method` is the authenticator id (e.g. `"proxy-header"`, `"oidc"`) — used
/// to set the new profile's `name` so the audit trail can tell auto-
/// provisioned users from invited ones.
pub(crate) fn ensure_user(
    vault_root: &Path,
    user_id: &str,
    method: &'static str,
    policy: &ProvisionPolicy,
) -> ProvisionOutcome {
    // 1. Existing profile? Pass through unchanged (idempotent).
    match load_profile(vault_root, user_id) {
        Ok(Some(_)) => return ProvisionOutcome::Allowed,
        Ok(None) => { /* fall through to provisioning */ }
        Err(e) => return ProvisionOutcome::Denied(DenyReason::IoError(e.to_string())),
    }

    // 2. Provisioning gated on operator opt-in.
    if !policy.enabled {
        return ProvisionOutcome::Denied(DenyReason::UnknownUserAndAutoProvisionDisabled);
    }

    // 3. Optional domain gate (OIDC). Proxy-header passes `None` here.
    if let Some(allow) = &policy.domain_allow {
        let Some(domain) = extract_domain(user_id) else {
            return ProvisionOutcome::Denied(DenyReason::NoDomainForAllowlistCheck);
        };
        if !allow.iter().any(|d| d.eq_ignore_ascii_case(domain)) {
            return ProvisionOutcome::Denied(DenyReason::DomainNotInAllowlist(
                domain.to_string(),
            ));
        }
    }

    // 4. Create the profile at the Reader ceiling. `owner = false` + no entry
    //    in access.spl ⇒ `Role::for_profile_with_vault` returns Reader
    //    (ADR-4107 least privilege). No code path here can set owner = true
    //    or write an access.spl override.
    let profile = UserProfile {
        id: user_id.to_string(),
        name: user_id.to_string(),
        created_at: now_iso8601(),
        invited_by: None,
        owner: false,
        credentials: Vec::new(),
        recovery_pubkey: String::new(),
        agent_token_generation: 0,
    };
    let _ = method; // reserved for future audit-line provenance (REQ-4115)
    if let Err(e) = save_profile(vault_root, &profile) {
        return ProvisionOutcome::Denied(DenyReason::IoError(e.to_string()));
    }
    ProvisionOutcome::Allowed
}

/// Extract the domain (rightmost `@`-tail) from a user_id, or `None` if the
/// user_id contains no `@`.
fn extract_domain(user_id: &str) -> Option<&str> {
    user_id.rsplit_once('@').map(|(_, domain)| domain).filter(|d| !d.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::user::Role;

    /// REQ-4111: an unknown principal with `auto_provision = false` is
    /// denied.
    #[test]
    fn unknown_user_with_provisioning_off_is_denied() {
        let tmp = tempfile::TempDir::new().unwrap();
        let policy = ProvisionPolicy {
            enabled: false,
            domain_allow: None,
        };
        let outcome = ensure_user(tmp.path(), "alice@example.com", "oidc", &policy);
        assert_eq!(
            outcome,
            ProvisionOutcome::Denied(DenyReason::UnknownUserAndAutoProvisionDisabled)
        );
    }

    /// REQ-4111 / ADR-4107: an unknown principal with `auto_provision = true`
    /// is admitted, and the created profile resolves to Reader (never higher).
    #[test]
    fn auto_provisioned_profile_is_reader() {
        let tmp = tempfile::TempDir::new().unwrap();
        let policy = ProvisionPolicy {
            enabled: true,
            domain_allow: None,
        };
        let outcome = ensure_user(tmp.path(), "alice", "proxy-header", &policy);
        assert_eq!(outcome, ProvisionOutcome::Allowed);

        // Profile exists; its resolved role is Reader.
        let profile = load_profile(tmp.path(), "alice").unwrap().unwrap();
        assert!(!profile.owner);
        assert!(profile.invited_by.is_none());
        assert_eq!(Role::for_profile_with_vault(&profile, tmp.path()), Role::Reader);
    }

    /// REQ-4111: existing profile is passed through unchanged regardless of
    /// policy — idempotent.
    #[test]
    fn existing_profile_passes_through() {
        let tmp = tempfile::TempDir::new().unwrap();
        // Pre-create an Editor (owner=true → Admin actually; let's use a
        // distinct id to keep this clean).
        let owner_profile = UserProfile {
            id: "boss".to_string(),
            name: "Boss".to_string(),
            created_at: "2026-05-15T00:00:00Z".to_string(),
            invited_by: None,
            owner: true,
            credentials: Vec::new(),
            recovery_pubkey: String::new(),
            agent_token_generation: 0,
        };
        save_profile(tmp.path(), &owner_profile).unwrap();

        // Even with provisioning off, the existing user passes through.
        let policy = ProvisionPolicy {
            enabled: false,
            domain_allow: None,
        };
        assert_eq!(
            ensure_user(tmp.path(), "boss", "proxy-header", &policy),
            ProvisionOutcome::Allowed
        );

        // The profile was not overwritten — owner=true preserved.
        let reloaded = load_profile(tmp.path(), "boss").unwrap().unwrap();
        assert!(reloaded.owner);
    }

    /// REQ-4111 OIDC domain gate: in-allowlist domain provisions.
    #[test]
    fn domain_allowlist_admits_listed() {
        let tmp = tempfile::TempDir::new().unwrap();
        let policy = ProvisionPolicy {
            enabled: true,
            domain_allow: Some(vec!["example.com".to_string()]),
        };
        assert_eq!(
            ensure_user(tmp.path(), "alice@example.com", "oidc", &policy),
            ProvisionOutcome::Allowed
        );
    }

    /// REQ-4111 OIDC domain gate: out-of-allowlist domain denies.
    #[test]
    fn domain_allowlist_rejects_unlisted() {
        let tmp = tempfile::TempDir::new().unwrap();
        let policy = ProvisionPolicy {
            enabled: true,
            domain_allow: Some(vec!["example.com".to_string()]),
        };
        let outcome = ensure_user(tmp.path(), "alice@evil.org", "oidc", &policy);
        assert_eq!(
            outcome,
            ProvisionOutcome::Denied(DenyReason::DomainNotInAllowlist(
                "evil.org".to_string()
            ))
        );
    }

    /// REQ-4111: domain gate active but user_id has no `@` ⇒ denied (an OIDC
    /// claim shaped like `"alice"` cannot be domain-checked).
    #[test]
    fn domain_allowlist_rejects_no_at_sign() {
        let tmp = tempfile::TempDir::new().unwrap();
        let policy = ProvisionPolicy {
            enabled: true,
            domain_allow: Some(vec!["example.com".to_string()]),
        };
        assert_eq!(
            ensure_user(tmp.path(), "alice", "oidc", &policy),
            ProvisionOutcome::Denied(DenyReason::NoDomainForAllowlistCheck)
        );
    }

    /// Domain check is case-insensitive (matches DNS semantics).
    #[test]
    fn domain_allowlist_case_insensitive() {
        let tmp = tempfile::TempDir::new().unwrap();
        let policy = ProvisionPolicy {
            enabled: true,
            domain_allow: Some(vec!["Example.COM".to_string()]),
        };
        assert_eq!(
            ensure_user(tmp.path(), "alice@example.com", "oidc", &policy),
            ProvisionOutcome::Allowed
        );
    }

    /// ADR-4107 property: this module has no path to a non-Reader role.
    /// Provisioning ten different users through every legal policy never
    /// produces a profile that resolves above Reader.
    #[test]
    fn provisioning_never_elevates_above_reader() {
        let policies = [
            ProvisionPolicy {
                enabled: true,
                domain_allow: None,
            },
            ProvisionPolicy {
                enabled: true,
                domain_allow: Some(vec!["example.com".to_string()]),
            },
        ];
        for (i, policy) in policies.iter().enumerate() {
            for n in 0..10 {
                let tmp = tempfile::TempDir::new().unwrap();
                let uid = format!("user{i}-{n}@example.com");
                if let ProvisionOutcome::Allowed = ensure_user(tmp.path(), &uid, "proxy-header", policy)
                {
                    let p = load_profile(tmp.path(), &uid).unwrap().unwrap();
                    let role = Role::for_profile_with_vault(&p, tmp.path());
                    assert_eq!(
                        role,
                        Role::Reader,
                        "policy={i} n={n} produced role={role:?}"
                    );
                }
            }
        }
    }
}
