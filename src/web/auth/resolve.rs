//! The `auth_resolve` middleware — the single point at which a request is
//! resolved to a [`Principal`] (SPEC-041 REQ-4104, CON-4103, ADR-4102).
//!
//! The chain is walked exactly once per request, first-match-wins
//! (NFR-4104). The result — `Option<Principal>` — is stashed in the request
//! extensions; `collab_gate`, `admin_gate`, and every extractor read it via
//! [`request_principal`] instead of re-parsing headers. That single
//! resolution path is what the Phase-0 gate refactor (task-auth-gate-refactor)
//! collapses the current triple-duplicated logic onto.
//!
//! `auth_resolve` carries the [`AuthChain`] as its *own* middleware state
//! (`from_fn_with_state(chain, auth_resolve)`) rather than a `WebState` field:
//! only this middleware needs the chain, and each [`Authenticator`] is built
//! with its own dependencies at chain-assembly time, so `authenticate` needs
//! nothing but the request [`Parts`].

use axum::extract::{Request, State};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use super::{AuthChain, AuthOutcome, AuthRejection, Authenticator, Principal};

/// The result of walking the authenticator chain for one request.
#[derive(Debug)]
pub(crate) enum ChainOutcome {
    /// The first authenticator that did not abstain accepted the request.
    Authenticated(Principal),
    /// An authenticator hard-rejected — the chain terminated without trying
    /// the rest (a present-but-invalid credential the operator fails closed
    /// on).
    Rejected(AuthRejection),
    /// Every authenticator abstained — the request carries no credential this
    /// chain recognises. `collab_gate` decides redirect-vs-401 from here.
    Unauthenticated,
}

/// Walk `chain` in order and return the first non-abstaining outcome
/// (SPEC-041 NFR-4104: deterministic, first-match-wins, order = `methods`
/// order). Pure with respect to its own logic — any I/O is whatever an
/// individual [`Authenticator::authenticate`] performs.
pub(crate) fn resolve_chain(
    chain: &[Box<dyn Authenticator + Send + Sync>],
    parts: &Parts,
) -> ChainOutcome {
    for authenticator in chain {
        match authenticator.authenticate(parts) {
            AuthOutcome::Authenticated(principal) => {
                return ChainOutcome::Authenticated(principal)
            }
            AuthOutcome::Reject(rejection) => return ChainOutcome::Rejected(rejection),
            AuthOutcome::Abstain => continue,
        }
    }
    ChainOutcome::Unauthenticated
}

/// Axum middleware: resolve the request to a [`Principal`] exactly once and
/// stash `Option<Principal>` in the request extensions (CON-4103).
///
/// - First `Authenticated` → `Some(principal)` in extensions, request proceeds.
/// - All `Abstain` → `None` in extensions, request proceeds (`collab_gate`
///   produces the redirect/401).
/// - Any `Reject` → the chain terminates here with a generic `401`; the
///   operator-channel `cause` is dropped for now and wired into the audit log
///   by task-auth-observability.
///
/// MUST be layered ahead of `collab_gate` and `csrf_guard` — enforced by
/// construction in `src/web/mod.rs` and a middleware-ordering test added in
/// task-auth-gate-refactor.
pub(crate) async fn auth_resolve(
    State(chain): State<AuthChain>,
    request: Request,
    next: Next,
) -> Response {
    let (mut parts, body) = request.into_parts();

    match resolve_chain(&chain, &parts) {
        ChainOutcome::Authenticated(principal) => {
            parts.extensions.insert::<Option<Principal>>(Some(principal));
        }
        ChainOutcome::Unauthenticated => {
            parts.extensions.insert::<Option<Principal>>(None);
        }
        ChainOutcome::Rejected(AuthRejection { cause: _cause }) => {
            // Generic user-visible failure (SPEC-041 §1.3.8). The `cause`
            // category is operator-channel only and is logged by
            // task-auth-observability — until then it is intentionally dropped
            // rather than surfaced.
            return StatusCode::UNAUTHORIZED.into_response();
        }
    }

    next.run(Request::from_parts(parts, body)).await
}

/// Read the resolved [`Principal`] from request extensions.
///
/// Returns `Some` only when `auth_resolve` ran *and* an authenticator
/// accepted. `None` covers both "ran, all abstained" and "middleware not in
/// this route's stack" — callers that require authentication treat both the
/// same way. This is the accessor `collab_gate` / `admin_gate` / the
/// extractors use instead of re-parsing headers (REQ-4104).
pub(crate) fn request_principal(parts: &Parts) -> Option<&Principal> {
    parts
        .extensions
        .get::<Option<Principal>>()
        .and_then(|resolved| resolved.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::auth::PrincipalId;

    /// Test authenticator with a fixed, configured behaviour.
    struct MockAuth {
        id: &'static str,
        behaviour: Behaviour,
    }

    #[derive(Clone, Copy, Debug)]
    enum Behaviour {
        Abstain,
        Accept,
        Reject,
    }

    impl Authenticator for MockAuth {
        fn id(&self) -> &'static str {
            self.id
        }
        fn authenticate(&self, _parts: &Parts) -> AuthOutcome {
            match self.behaviour {
                Behaviour::Abstain => AuthOutcome::Abstain,
                Behaviour::Accept => AuthOutcome::Authenticated(Principal {
                    identity: PrincipalId::User(self.id.to_string()),
                    method: self.id,
                    cookie_session: false,
                    capability: None,
                }),
                Behaviour::Reject => AuthOutcome::Reject(AuthRejection {
                    cause: self.id,
                }),
            }
        }
        fn issues_cookie_session(&self) -> bool {
            false
        }
    }

    fn chain(behaviours: &[(&'static str, Behaviour)]) -> Vec<Box<dyn Authenticator + Send + Sync>> {
        behaviours
            .iter()
            .map(|(id, b)| {
                Box::new(MockAuth {
                    id,
                    behaviour: *b,
                }) as Box<dyn Authenticator + Send + Sync>
            })
            .collect()
    }

    fn empty_parts() -> Parts {
        Request::new(()).into_parts().0
    }

    /// TEST-4104: the first non-abstaining authenticator wins, and earlier
    /// abstains do not shadow it.
    #[test]
    fn first_match_wins() {
        let c = chain(&[
            ("a", Behaviour::Abstain),
            ("b", Behaviour::Accept),
            ("c", Behaviour::Accept),
        ]);
        match resolve_chain(&c, &empty_parts()) {
            ChainOutcome::Authenticated(p) => assert_eq!(p.method, "b"),
            other => panic!("expected Authenticated(b), got {other:?}"),
        }
    }

    /// TEST-4104: a `Reject` terminates the chain — later authenticators that
    /// would have accepted are never consulted.
    #[test]
    fn reject_terminates_chain() {
        let c = chain(&[
            ("a", Behaviour::Abstain),
            ("b", Behaviour::Reject),
            ("c", Behaviour::Accept),
        ]);
        match resolve_chain(&c, &empty_parts()) {
            ChainOutcome::Rejected(r) => assert_eq!(r.cause, "b"),
            other => panic!("expected Rejected(b), got {other:?}"),
        }
    }

    /// TEST-4104: a chain where everyone abstains yields `Unauthenticated`.
    #[test]
    fn all_abstain_is_unauthenticated() {
        let c = chain(&[("a", Behaviour::Abstain), ("b", Behaviour::Abstain)]);
        assert!(matches!(
            resolve_chain(&c, &empty_parts()),
            ChainOutcome::Unauthenticated
        ));
        // The empty chain is also unauthenticated.
        assert!(matches!(
            resolve_chain(&[], &empty_parts()),
            ChainOutcome::Unauthenticated
        ));
    }

    /// TEST-NFR-4104: determinism — for every chain over the behaviour
    /// alphabet up to length 4, `resolve_chain` returns exactly the outcome
    /// of the first non-abstaining slot (or `Unauthenticated`), independent
    /// of anything but list order. Exhaustive over the generated space.
    #[test]
    fn deterministic_first_non_abstaining() {
        let alphabet = [Behaviour::Abstain, Behaviour::Accept, Behaviour::Reject];
        // All chains of length 0..=4 over the 3-symbol alphabet.
        for len in 0..=4usize {
            let combos = (0..3usize.pow(len as u32)).map(|mut n| {
                (0..len)
                    .map(|_| {
                        let sym = alphabet[n % 3];
                        n /= 3;
                        sym
                    })
                    .collect::<Vec<_>>()
            });
            for behaviours in combos {
                let labelled: Vec<(&'static str, Behaviour)> = behaviours
                    .iter()
                    .enumerate()
                    .map(|(i, b)| (["s0", "s1", "s2", "s3"][i], *b))
                    .collect();
                let c = chain(&labelled);
                let got = resolve_chain(&c, &empty_parts());

                // Expected: first slot that is not Abstain decides.
                let expected_idx = behaviours
                    .iter()
                    .position(|b| !matches!(b, Behaviour::Abstain));
                match (expected_idx.map(|i| (i, behaviours[i])), got) {
                    (None, ChainOutcome::Unauthenticated) => {}
                    (Some((i, Behaviour::Accept)), ChainOutcome::Authenticated(p)) => {
                        assert_eq!(p.method, ["s0", "s1", "s2", "s3"][i]);
                    }
                    (Some((i, Behaviour::Reject)), ChainOutcome::Rejected(r)) => {
                        assert_eq!(r.cause, ["s0", "s1", "s2", "s3"][i]);
                    }
                    (exp, actual) => {
                        panic!("chain {behaviours:?}: expected {exp:?}, got {actual:?}")
                    }
                }
            }
        }
    }

    /// TEST-4104: `request_principal` round-trips what `auth_resolve` would
    /// have inserted.
    #[test]
    fn request_principal_reads_extension() {
        let mut parts = empty_parts();
        assert!(request_principal(&parts).is_none());

        parts.extensions.insert::<Option<Principal>>(None);
        assert!(request_principal(&parts).is_none());

        let principal = Principal {
            identity: PrincipalId::User("alice".to_string()),
            method: "passkey",
            cookie_session: true,
            capability: None,
        };
        parts
            .extensions
            .insert::<Option<Principal>>(Some(principal.clone()));
        assert_eq!(request_principal(&parts), Some(&principal));
    }
}
