//! Reverse-proxy header [`Authenticator`] (SPEC-041 REQ-4105, REQ-4106,
//! REQ-4111, REQ-4112, REQ-4120, CON-4108, ADR-4103).
//!
//! Derives the authenticated `user_id` from a configurable request header
//! (default `X-Forwarded-User`) set by a trusted upstream proxy
//! (oauth2-proxy, Authelia, Tailscale Serve, Cloudflare Access, an
//! SSO-aware ingress, etc.).
//!
//! Trust gate (ADR-4103, defence in depth — three layers):
//!
//! 1. The server must have been started with `--trust-proxy`; otherwise
//!    `proxy-header` appearing in `[collab.auth] methods` is a hard startup
//!    error (REQ-4106 / REQ-4114), enforced by the chain builder.
//! 2. At request time, the immediate peer IP must be in the operator's
//!    `peer_allow` CIDR list; the header is *ignored* on any non-allowlisted
//!    peer.
//! 3. The header value must satisfy the CON-4108 grammar — REJECTED, not
//!    normalised, on mismatch (REQ-4120, LangSec).
//!
//! Issues a stateless [`Principal`] (`cookie_session = false`) — generalises
//! the pre-SPEC-041 Bearer-only CSRF exemption to every method the chain
//! declares stateless (REQ-4112).

use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::ConnectInfo;
use axum::http::request::Parts;
use serde::Deserialize;

use super::provision::{ensure_user, ProvisionOutcome, ProvisionPolicy};
use super::{AuthOutcome, Authenticator, Principal, PrincipalId};

/// `[collab.auth.proxy_header]` configuration.
///
/// `deny_unknown_fields` so a typo at the schema is a startup error
/// (REQ-4120, REQ-4114). The default for `user_header` is `X-Forwarded-User`
/// — the convention used by oauth2-proxy, Authelia, and Tailscale Serve.
#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProxyHeaderConfig {
    /// Header name carrying the authenticated `user_id`. ASCII, case-insensitive
    /// at the HTTP layer.
    #[serde(default = "default_user_header")]
    pub user_header: String,

    /// CIDR allowlist for the immediate peer IP. Required and non-empty —
    /// an empty `peer_allow` is a configuration error (REQ-4106 fail-closed).
    pub peer_allow: Vec<String>,

    /// Opt-in auto-provisioning for principals with no `UserProfile`
    /// (REQ-4111, ADR-4107 Reader-ceiling).
    #[serde(default)]
    pub auto_provision: bool,
}

fn default_user_header() -> String {
    "X-Forwarded-User".to_string()
}

/// A parsed CIDR (IPv4 or IPv6). Bespoke implementation because the repo
/// has no CIDR-matching crate; the routine is small and exhaustively tested
/// here.
#[derive(Debug, Clone)]
pub(crate) struct Cidr {
    network: IpAddr,
    prefix: u8,
}

impl Cidr {
    pub(crate) fn parse(s: &str) -> Result<Self, String> {
        let (addr, prefix) = s
            .split_once('/')
            .ok_or_else(|| format!("invalid CIDR {s:?}: missing '/'"))?;
        let network: IpAddr = addr
            .parse()
            .map_err(|e| format!("invalid CIDR {s:?}: {e}"))?;
        let prefix: u8 = prefix
            .parse()
            .map_err(|e| format!("invalid CIDR {s:?}: bad prefix: {e}"))?;
        let max = if network.is_ipv4() { 32 } else { 128 };
        if prefix > max {
            return Err(format!(
                "invalid CIDR {s:?}: prefix {prefix} exceeds {max}"
            ));
        }
        Ok(Self { network, prefix })
    }

    pub(crate) fn contains(&self, ip: IpAddr) -> bool {
        match (self.network, ip) {
            (IpAddr::V4(net), IpAddr::V4(test)) => {
                let mask = if self.prefix == 0 {
                    0u32
                } else {
                    u32::MAX << (32 - self.prefix)
                };
                (u32::from(net) & mask) == (u32::from(test) & mask)
            }
            (IpAddr::V6(net), IpAddr::V6(test)) => {
                let mask = if self.prefix == 0 {
                    0u128
                } else {
                    u128::MAX << (128 - self.prefix)
                };
                (u128::from(net) & mask) == (u128::from(test) & mask)
            }
            // Different families: no match (a v4 client never satisfies a v6
            // allowlist entry and vice versa).
            _ => false,
        }
    }
}

/// Recognise a header value against the CON-4108 grammar
/// `1*256( ALPHA / DIGIT / "." / "_" / "-" / "@" / "+" )`.
/// Reject — never normalise — on mismatch (REQ-4120 LangSec).
pub(crate) fn recognise_user_value(s: &str) -> Option<&str> {
    if s.is_empty() || s.len() > 256 {
        return None;
    }
    if s.bytes().all(|b| {
        b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'@' | b'+')
    }) {
        Some(s)
    } else {
        None
    }
}

/// The reverse-proxy header authenticator.
pub(crate) struct ProxyHeaderAuthenticator {
    user_header: String,
    peer_allow: Vec<Cidr>,
    auto_provision: bool,
    vault_root: Arc<PathBuf>,
}

impl ProxyHeaderAuthenticator {
    /// Construct from a validated [`ProxyHeaderConfig`] and the vault root.
    ///
    /// Returns an error if any `peer_allow` entry fails CIDR parsing, or if
    /// the list is empty (REQ-4106 fail-closed: an empty allowlist would
    /// trust every peer, defeating the gate).
    pub(crate) fn new(
        cfg: &ProxyHeaderConfig,
        vault_root: Arc<PathBuf>,
    ) -> Result<Self, String> {
        if cfg.peer_allow.is_empty() {
            return Err(
                "[collab.auth.proxy_header] peer_allow must not be empty — \
                 specify at least one CIDR for the trusted proxy hop"
                    .to_string(),
            );
        }
        let peer_allow: Vec<Cidr> = cfg
            .peer_allow
            .iter()
            .map(|s| Cidr::parse(s))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("[collab.auth.proxy_header] {e}"))?;
        Ok(Self {
            user_header: cfg.user_header.clone(),
            peer_allow,
            auto_provision: cfg.auto_provision,
            vault_root,
        })
    }

    fn peer_is_allowed(&self, peer: IpAddr) -> bool {
        self.peer_allow.iter().any(|c| c.contains(peer))
    }
}

impl Authenticator for ProxyHeaderAuthenticator {
    fn id(&self) -> &'static str {
        "proxy-header"
    }

    fn authenticate(&self, parts: &Parts) -> AuthOutcome {
        // Layer 2 of the trust gate: the peer must be in the allowlist.
        // Without `ConnectInfo`, we cannot prove this — abstain rather than
        // trust by default (REQ-4106 fail-closed).
        let Some(ConnectInfo(peer)) = parts.extensions.get::<ConnectInfo<std::net::SocketAddr>>()
        else {
            return AuthOutcome::Abstain;
        };
        if !self.peer_is_allowed(peer.ip()) {
            return AuthOutcome::Abstain;
        }

        // Layer 3: read & recognise the header value against the CON-4108
        // grammar. Anything that doesn't match is rejected — not normalised.
        let header_value = parts
            .headers
            .get(self.user_header.as_str())
            .and_then(|v| v.to_str().ok());
        let user_id = match header_value.and_then(recognise_user_value) {
            Some(u) => u.to_string(),
            None => return AuthOutcome::Abstain,
        };

        // Provision-or-lookup: an unknown user is admitted only when
        // `auto_provision = true` and (for proxy-header) no domain gate.
        let policy = ProvisionPolicy {
            enabled: self.auto_provision,
            domain_allow: None,
        };
        match ensure_user(&self.vault_root, &user_id, "proxy-header", &policy) {
            ProvisionOutcome::Allowed => AuthOutcome::Authenticated(Principal {
                identity: PrincipalId::User(user_id),
                method: "proxy-header",
                cookie_session: false,
                capability: None,
            }),
            ProvisionOutcome::Denied(_) => AuthOutcome::Abstain,
        }
    }

    fn issues_cookie_session(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::Request;
    use std::net::SocketAddr;

    fn parts_with_peer_and_header(
        peer: &str,
        header: Option<(&'static str, &str)>,
    ) -> Parts {
        let mut req: Request<()> = Request::new(());
        let sock: SocketAddr = peer.parse().unwrap();
        req.extensions_mut().insert(ConnectInfo(sock));
        if let Some((name, value)) = header {
            req.headers_mut()
                .insert(name, value.parse().unwrap());
        }
        req.into_parts().0
    }

    fn authenticator(peer_allow: &[&str], auto_provision: bool) -> ProxyHeaderAuthenticator {
        let tmp = tempfile::TempDir::new().unwrap();
        let cfg = ProxyHeaderConfig {
            user_header: "X-Forwarded-User".to_string(),
            peer_allow: peer_allow.iter().map(|s| s.to_string()).collect(),
            auto_provision,
        };
        let auth =
            ProxyHeaderAuthenticator::new(&cfg, Arc::new(tmp.path().to_path_buf())).unwrap();
        // Leak the tempdir so the path stays valid for the test (each test
        // function has its own); good enough for unit scope.
        std::mem::forget(tmp);
        auth
    }

    /// CIDR matcher — IPv4 host inside its /24.
    #[test]
    fn cidr_v4_basic() {
        let c = Cidr::parse("10.0.0.0/8").unwrap();
        assert!(c.contains("10.1.2.3".parse().unwrap()));
        assert!(!c.contains("11.0.0.0".parse().unwrap()));
    }

    /// CIDR matcher — IPv4 /32 single-host.
    #[test]
    fn cidr_v4_host() {
        let c = Cidr::parse("127.0.0.1/32").unwrap();
        assert!(c.contains("127.0.0.1".parse().unwrap()));
        assert!(!c.contains("127.0.0.2".parse().unwrap()));
    }

    /// CIDR matcher — IPv6 prefix.
    #[test]
    fn cidr_v6_basic() {
        let c = Cidr::parse("fd00::/8").unwrap();
        assert!(c.contains("fd00::1".parse().unwrap()));
        assert!(!c.contains("fe00::1".parse().unwrap()));
    }

    /// CIDR matcher — different families never match.
    #[test]
    fn cidr_mixed_family_never_matches() {
        let v4 = Cidr::parse("10.0.0.0/8").unwrap();
        let v6 = Cidr::parse("fd00::/8").unwrap();
        assert!(!v4.contains("fd00::1".parse().unwrap()));
        assert!(!v6.contains("10.0.0.1".parse().unwrap()));
    }

    /// CIDR matcher — /0 matches everything in its family.
    #[test]
    fn cidr_zero_prefix() {
        let c = Cidr::parse("0.0.0.0/0").unwrap();
        assert!(c.contains("1.2.3.4".parse().unwrap()));
        assert!(c.contains("255.255.255.255".parse().unwrap()));
    }

    /// CIDR parse — malformed input is rejected.
    #[test]
    fn cidr_parse_errors() {
        for bad in ["bogus", "10.0.0.0", "10.0.0.0/abc", "10.0.0.0/33"] {
            assert!(Cidr::parse(bad).is_err(), "expected {bad:?} to fail");
        }
    }

    /// CON-4108 grammar: a valid header value is recognised.
    #[test]
    fn recognise_valid_user_values() {
        for ok in ["alice", "alice@example.com", "a.b_c-d+e", "User123"] {
            assert_eq!(recognise_user_value(ok), Some(ok));
        }
    }

    /// REQ-4120: a header value with characters outside the grammar is
    /// rejected — not normalised, not trimmed.
    #[test]
    fn recognise_rejects_out_of_grammar() {
        for bad in [
            "",
            "alice bob",     // space
            "a/b",           // slash
            "alice;drop",    // semicolon
            "alice\nadmin",  // newline
            "\u{00e9}lise",  // non-ASCII
        ] {
            assert_eq!(recognise_user_value(bad), None, "should reject {bad:?}");
        }
    }

    /// REQ-4120: a header value over 256 chars is rejected.
    #[test]
    fn recognise_rejects_overlong() {
        let s = "a".repeat(257);
        assert_eq!(recognise_user_value(&s), None);
    }

    /// REQ-4106 layer 2: header from non-allowlisted peer is ignored
    /// (Abstain, not Authenticated).
    #[test]
    fn header_from_non_allowlisted_peer_abstains() {
        let auth = authenticator(&["127.0.0.1/32"], true);
        let parts =
            parts_with_peer_and_header("8.8.8.8:54321", Some(("X-Forwarded-User", "alice")));
        assert!(matches!(auth.authenticate(&parts), AuthOutcome::Abstain));
    }

    /// REQ-4105 / REQ-4111: grammatical header from allowlisted peer with
    /// auto-provision admits — stateless principal.
    #[test]
    fn grammatical_header_from_allowlisted_peer_authenticates() {
        let auth = authenticator(&["10.0.0.0/8"], true);
        let parts =
            parts_with_peer_and_header("10.1.2.3:54321", Some(("X-Forwarded-User", "alice")));
        match auth.authenticate(&parts) {
            AuthOutcome::Authenticated(p) => {
                assert_eq!(p.identity, PrincipalId::User("alice".to_string()));
                assert_eq!(p.method, "proxy-header");
                assert!(!p.cookie_session); // REQ-4112 stateless
            }
            other => panic!("expected Authenticated, got {other:?}"),
        }
    }

    /// REQ-4111: with auto-provision off, an unknown user is Abstain.
    #[test]
    fn auto_provision_off_unknown_user_abstains() {
        let auth = authenticator(&["10.0.0.0/8"], false);
        let parts =
            parts_with_peer_and_header("10.1.2.3:54321", Some(("X-Forwarded-User", "alice")));
        assert!(matches!(auth.authenticate(&parts), AuthOutcome::Abstain));
    }

    /// REQ-4106: with no ConnectInfo on the request (cannot verify peer),
    /// abstain rather than fall back to trusting the header.
    #[test]
    fn no_connect_info_abstains() {
        let auth = authenticator(&["10.0.0.0/8"], true);
        let mut req: Request<()> = Request::new(());
        req.headers_mut()
            .insert("X-Forwarded-User", "alice".parse().unwrap());
        let parts = req.into_parts().0;
        assert!(matches!(auth.authenticate(&parts), AuthOutcome::Abstain));
    }

    /// REQ-4106 + REQ-4120: ungrammatical header value from allowlisted
    /// peer abstains (rejected, not normalised).
    #[test]
    fn ungrammatical_header_value_abstains() {
        let auth = authenticator(&["10.0.0.0/8"], true);
        let parts = parts_with_peer_and_header(
            "10.1.2.3:54321",
            Some(("X-Forwarded-User", "alice;drop")),
        );
        assert!(matches!(auth.authenticate(&parts), AuthOutcome::Abstain));
    }

    /// REQ-4106 fail-closed: empty `peer_allow` rejected at construction.
    #[test]
    fn empty_peer_allow_construction_fails() {
        let tmp = tempfile::TempDir::new().unwrap();
        let cfg = ProxyHeaderConfig {
            user_header: "X-Forwarded-User".to_string(),
            peer_allow: vec![],
            auto_provision: true,
        };
        let err = match ProxyHeaderAuthenticator::new(&cfg, Arc::new(tmp.path().to_path_buf())) {
            Ok(_) => panic!("expected construction error"),
            Err(e) => e,
        };
        assert!(err.contains("peer_allow must not be empty"));
    }

    /// Invalid CIDR entry rejected at construction with a clear message.
    #[test]
    fn bad_cidr_construction_fails() {
        let tmp = tempfile::TempDir::new().unwrap();
        let cfg = ProxyHeaderConfig {
            user_header: "X-Forwarded-User".to_string(),
            peer_allow: vec!["nonsense".to_string()],
            auto_provision: false,
        };
        let err = match ProxyHeaderAuthenticator::new(&cfg, Arc::new(tmp.path().to_path_buf())) {
            Ok(_) => panic!("expected construction error"),
            Err(e) => e,
        };
        assert!(err.contains("nonsense"));
    }

    /// REQ-4112: declares stateless, no login surface.
    #[test]
    fn declares_stateless_no_login_no_routes() {
        let auth = authenticator(&["127.0.0.1/32"], false);
        assert!(!auth.issues_cookie_session());
        assert_eq!(auth.login_redirect(), None);
        assert_eq!(auth.id(), "proxy-header");
    }
}
