//! `[collab.auth]` configuration lens (SPEC-041 REQ-4102, REQ-4114, REQ-4120,
//! CON-4102).
//!
//! Parses `[collab.auth]` from `.zetl/config.toml`, following the lens
//! precedent in `src/cap/public_repo.rs` (`ZetlConfigLens`/`AccessConfig`):
//! deserialise only the `[collab]` section, tolerate unknown sibling
//! top-level keys, but `deny_unknown_fields` *within* `[collab.auth]` so a
//! typo at the auth schema is a startup error, not a silent default
//! (REQ-4120 grammar recognition).
//!
//! Phase-1 scope: the methods list and the structural shape of the per-method
//! sub-tables. Each sub-table is left opaque (a raw `toml::Value`) at this
//! task — it gets a strict, typed schema in its own phase
//! (`proxy_header` in Phase 2, `password` in Phase 3, `capability_url` in
//! Phase 4, `oidc` in Phase 5).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;

use crate::web::session::SessionStore;

use super::agent_token::AgentTokenAuthenticator;
use super::capability_url::CapabilityUrlAuthenticator;
use super::passkey::PasskeyAuthenticator;
use super::password::PasswordAuthenticator;
use super::proxy_header::{ProxyHeaderAuthenticator, ProxyHeaderConfig};
use super::{AuthChain, Authenticator};

/// Stable identifier for an authentication method, the same string used in
/// `[collab.auth] methods` and in the audit trail (REQ-4115).
///
/// Kebab-case at the TOML boundary keeps the schema consistent with
/// `[access]`-block enums (e.g. `SplitKeySecondFactor` in
/// `src/cap/public_repo.rs`).
#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum MethodId {
    Passkey,
    AgentToken,
    ProxyHeader,
    Password,
    CapabilityUrl,
    Oidc,
}

impl MethodId {
    /// The stable wire string — what the operator types in `methods` and what
    /// shows up in audit lines.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            MethodId::Passkey => "passkey",
            MethodId::AgentToken => "agent-token",
            MethodId::ProxyHeader => "proxy-header",
            MethodId::Password => "password",
            MethodId::CapabilityUrl => "capability-url",
            MethodId::Oidc => "oidc",
        }
    }
}

/// Top-level lens for `.zetl/config.toml`. Only the `[collab]` section is
/// consumed here; other top-level keys round-trip through their own lenses
/// elsewhere (e.g. `ZetlConfigLens` for `[vault]`/`[access]`).
#[derive(Deserialize, Default)]
struct ConfigLens {
    #[serde(default)]
    collab: Option<CollabSection>,
}

#[derive(Deserialize, Default)]
struct CollabSection {
    #[serde(default)]
    auth: Option<CollabAuthConfig>,
}

/// The `[collab.auth]` block.
///
/// `deny_unknown_fields` — every field declared here is the authoritative
/// schema; a typo (`methds = [...]`, `proxyheader = {...}`) fails startup
/// (REQ-4120, REQ-4114). The per-method sub-tables are typed as
/// `Option<toml::Value>` for Phase 1 and replaced with strict schemas as each
/// downstream phase ships its method.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub(crate) struct CollabAuthConfig {
    /// Ordered precedence chain (REQ-4102, NFR-4104 deterministic). Absent
    /// `[collab.auth]` block ⇒ this defaults to `[passkey, agent-token]`,
    /// reproducing pre-SPEC-041 behaviour exactly (REQ-4103).
    #[serde(default = "default_methods")]
    pub methods: Vec<MethodId>,

    /// `[collab.auth.proxy_header]` — typed schema landed with Phase 2
    /// (task-auth-proxy-header). Required when `proxy-header` is in
    /// `methods`; the chain builder enforces that.
    #[serde(default)]
    pub proxy_header: Option<ProxyHeaderConfig>,

    /// `[collab.auth.password]` — schema filled in by Phase 3.
    #[serde(default)]
    pub password: Option<toml::Value>,

    /// `[collab.auth.oidc]` — schema filled in by Phase 5.
    #[serde(default)]
    pub oidc: Option<toml::Value>,

    /// `[collab.auth.capability_url]` — schema filled in by Phase 4.
    #[serde(default)]
    pub capability_url: Option<toml::Value>,
}

fn default_methods() -> Vec<MethodId> {
    vec![MethodId::Passkey, MethodId::AgentToken]
}

impl Default for CollabAuthConfig {
    fn default() -> Self {
        Self {
            methods: default_methods(),
            proxy_header: None,
            password: None,
            oidc: None,
            capability_url: None,
        }
    }
}

/// Why a `[collab.auth]` configuration was rejected at parse or validate
/// time. The string is suitable for direct display to the operator — it names
/// the offending key and (where applicable) the corrective action (REQ-4114).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid [collab.auth] configuration: {0}")]
pub(crate) struct ConfigError(pub String);

/// Parse `[collab.auth]` from a TOML body via the lens.
///
/// An absent `[collab.auth]` block returns the default config (REQ-4103). The
/// `toml` crate is the declared recogniser (REQ-4120 grammar = RFC-8259-adjacent
/// TOML); the typed lens is the schema layered on top.
pub(crate) fn parse(toml_body: &str) -> Result<CollabAuthConfig, ConfigError> {
    let lens: ConfigLens = toml::from_str(toml_body).map_err(|e| ConfigError(e.to_string()))?;
    Ok(lens.collab.and_then(|c| c.auth).unwrap_or_default())
}

/// Validate a parsed [`CollabAuthConfig`] under the REQ-4114 rule set that
/// applies in Phase 1.
///
/// Per-method sub-table validation (`proxy_header` requires `peer_allow`;
/// `oidc` requires `issuer`/`client_id`; etc.) is added by each downstream
/// phase as it ships its method. The Phase-1 rules check the methods list
/// itself.
pub(crate) fn validate(cfg: &CollabAuthConfig) -> Result<(), ConfigError> {
    if cfg.methods.is_empty() {
        return Err(ConfigError(
            "[collab.auth] methods cannot be empty (omit the block for the \
             default `[passkey, agent-token]`)"
                .to_string(),
        ));
    }
    let mut seen: HashSet<MethodId> = HashSet::new();
    for m in &cfg.methods {
        if !seen.insert(*m) {
            return Err(ConfigError(format!(
                "[collab.auth] methods contains the duplicate {:?} — each \
                 method may appear at most once",
                m.as_str()
            )));
        }
    }
    Ok(())
}

/// Assemble the [`AuthChain`] from a validated [`CollabAuthConfig`]
/// (SPEC-041 REQ-4102 / REQ-4103).
///
/// `methods` order is the chain order — `auth_resolve` walks it first-match-
/// wins (NFR-4104). Methods whose implementation is feature-gated out or not
/// yet shipped fail startup with a clear message (REQ-4114): the operator
/// either removes them or upgrades zetl. The order of construction calls
/// here matches the declared `methods` list — no reordering, no implicit
/// fallbacks.
pub(crate) fn build_chain(
    cfg: &CollabAuthConfig,
    sessions: SessionStore,
    vault_root: Arc<PathBuf>,
    trust_proxy: bool,
) -> Result<AuthChain, ConfigError> {
    validate(cfg)?;

    let mut chain: Vec<Box<dyn Authenticator + Send + Sync>> =
        Vec::with_capacity(cfg.methods.len());
    for method in &cfg.methods {
        let authenticator: Box<dyn Authenticator + Send + Sync> = match method {
            MethodId::Passkey => Box::new(PasskeyAuthenticator::new(sessions.clone())),
            MethodId::AgentToken => Box::new(AgentTokenAuthenticator::new(vault_root.clone())),
            MethodId::ProxyHeader => {
                // REQ-4106: proxy-header requires --trust-proxy. Refusing to
                // start without it is the fail-closed mitigation against a
                // spoofed header.
                if !trust_proxy {
                    return Err(ConfigError(
                        "[collab.auth] methods contains \"proxy-header\" but \
                         the server was not started with --trust-proxy — \
                         remove the method or start with --trust-proxy \
                         (SPEC-041 REQ-4106)"
                            .to_string(),
                    ));
                }
                let phc = cfg.proxy_header.as_ref().ok_or_else(|| {
                    ConfigError(
                        "[collab.auth] methods contains \"proxy-header\" but \
                         [collab.auth.proxy_header] is missing — at minimum, \
                         set `peer_allow` to the CIDR(s) of your trusted \
                         proxy hop"
                            .to_string(),
                    )
                })?;
                Box::new(
                    ProxyHeaderAuthenticator::new(phc, vault_root.clone()).map_err(ConfigError)?,
                )
            }
            MethodId::Password => Box::new(PasswordAuthenticator::new(
                sessions.clone(),
                vault_root.clone(),
            )),
            MethodId::CapabilityUrl => {
                // Token verification needs the vault's Ed25519 signing key —
                // load or create it on the same code path the SPEC-020 invite
                // flow uses (REQ-4114 sub-table check).
                let verifying_key = crate::user::invite::server_verifying_key(&vault_root)
                    .map_err(|e| {
                        ConfigError(format!(
                            "[collab.auth] capability-url cannot load vault \
                             server key: {e}"
                        ))
                    })?;
                Box::new(CapabilityUrlAuthenticator::new(
                    verifying_key,
                    vault_root.clone(),
                ))
            }
            MethodId::Oidc => {
                #[cfg(feature = "collab-oidc")]
                {
                    let oidc_value = cfg.oidc.as_ref().ok_or_else(|| {
                        ConfigError(
                            "[collab.auth] methods contains \"oidc\" but \
                             [collab.auth.oidc] is missing — at minimum, set \
                             `issuer`, `client_id`, and `client_secret_file`"
                                .to_string(),
                        )
                    })?;
                    let oidc_cfg: super::oidc::OidcConfig =
                        oidc_value
                            .clone()
                            .try_into()
                            .map_err(|e: toml::de::Error| {
                                ConfigError(format!("[collab.auth.oidc] schema error: {e}"))
                            })?;
                    Box::new(
                        super::oidc::OidcAuthenticator::new(
                            oidc_cfg,
                            sessions.clone(),
                            vault_root.clone(),
                        )
                        .map_err(|e| ConfigError(format!("[collab.auth.oidc] {e}")))?,
                    )
                }
                #[cfg(not(feature = "collab-oidc"))]
                {
                    return Err(ConfigError(
                        "[collab.auth] methods contains \"oidc\" but the \
                         `collab-oidc` cargo feature is not compiled in — \
                         rebuild with `--features collab,collab-oidc` \
                         (SPEC-041 REQ-4114, ADR-4105)"
                            .to_string(),
                    ));
                }
            }
        };
        chain.push(authenticator);
    }
    Ok(Arc::new(chain))
}

/// Read `.zetl/config.toml` if present, parse `[collab.auth]`, validate, and
/// assemble the chain — the shell wrapper around [`parse`] / [`validate`] /
/// [`build_chain`]. An absent file yields the default config (REQ-4103).
pub(crate) fn build_chain_from_vault(
    vault_root: &Path,
    sessions: SessionStore,
    trust_proxy: bool,
) -> Result<(AuthChain, CollabAuthConfig), ConfigError> {
    let path = vault_root.join(".zetl/config.toml");
    let body = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Err(ConfigError(format!(
                "failed to read {}: {}",
                path.display(),
                e
            )))
        }
    };
    let cfg = parse(&body)?;
    let chain = build_chain(
        &cfg,
        sessions,
        Arc::new(vault_root.to_path_buf()),
        trust_proxy,
    )?;
    Ok((chain, cfg))
}

/// Format the OBS-4105 startup line enumerating the assembled chain and the
/// compiled auth feature set. Emitted to stderr by `src/web/mod.rs` at
/// startup so operators can see the active auth configuration at a glance.
pub(crate) fn format_chain_summary(cfg: &CollabAuthConfig) -> String {
    let methods = cfg
        .methods
        .iter()
        .map(|m| m.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let features = compiled_auth_features().join(", ");
    format!("[zetl] collab auth: methods=[{methods}] features=[{features}]")
}

/// The cargo features relevant to authentication that are compiled in.
/// Always includes `"collab"` when this code is reachable; conditional
/// `"collab-oidc"` joins it once Phase 5 lands.
fn compiled_auth_features() -> Vec<&'static str> {
    // `mut` is conditionally needed (only under `collab-oidc`).
    #[allow(unused_mut)]
    let mut features = vec!["collab"];
    #[cfg(feature = "collab-oidc")]
    features.push("collab-oidc");
    features
}

#[cfg(test)]
mod tests {
    use super::*;

    /// REQ-4103 / TEST-4103: an absent `[collab.auth]` block produces the
    /// default chain `[passkey, agent-token]` — pre-SPEC-041 behaviour.
    #[test]
    fn absent_block_defaults_to_passkey_agent_token() {
        let cfg = parse("").unwrap();
        assert_eq!(cfg.methods, vec![MethodId::Passkey, MethodId::AgentToken]);
        assert!(cfg.proxy_header.is_none());
        assert!(cfg.oidc.is_none());
        validate(&cfg).unwrap();
    }

    /// REQ-4102 / TEST-4102: `methods` is parsed in declared order.
    #[test]
    fn methods_parsed_in_order() {
        let body = r#"
            [collab.auth]
            methods = ["oidc", "capability-url", "agent-token"]
        "#;
        let cfg = parse(body).unwrap();
        assert_eq!(
            cfg.methods,
            vec![
                MethodId::Oidc,
                MethodId::CapabilityUrl,
                MethodId::AgentToken
            ]
        );
        validate(&cfg).unwrap();
    }

    /// REQ-4114 / TEST-4114: an unknown method name is a parse error.
    /// REQ-4120 negative-input: malformed input is rejected, not normalised.
    #[test]
    fn unknown_method_name_fails() {
        let body = r#"
            [collab.auth]
            methods = ["passkey", "bogus"]
        "#;
        let err = parse(body).unwrap_err();
        // The error should name the offending value somewhere.
        assert!(
            err.0.contains("bogus") || err.0.contains("unknown variant"),
            "expected an error naming the bad method, got: {}",
            err.0
        );
    }

    /// REQ-4114: a typo at the `[collab.auth]` schema is rejected
    /// (deny_unknown_fields, REQ-4120).
    #[test]
    fn unknown_auth_field_fails() {
        let body = r#"
            [collab.auth]
            methods = ["passkey"]
            methds  = ["typo"]
        "#;
        let err = parse(body).unwrap_err();
        assert!(
            err.0.contains("methds") || err.0.contains("unknown field"),
            "expected an error naming the typo, got: {}",
            err.0
        );
    }

    /// Validation: empty `methods` list is rejected with a clear message
    /// (the operator probably meant to omit the block).
    #[test]
    fn empty_methods_list_rejected() {
        let body = r#"
            [collab.auth]
            methods = []
        "#;
        let cfg = parse(body).unwrap();
        let err = validate(&cfg).unwrap_err();
        assert!(err.0.contains("cannot be empty"));
    }

    /// Validation: duplicate method names are rejected.
    #[test]
    fn duplicate_method_rejected() {
        let body = r#"
            [collab.auth]
            methods = ["passkey", "agent-token", "passkey"]
        "#;
        let cfg = parse(body).unwrap();
        let err = validate(&cfg).unwrap_err();
        assert!(err.0.contains("duplicate"));
    }

    /// Tolerate sibling top-level keys — the wider zetl config has other
    /// consumers (`[vault]`, `[access]`, `[parse]`).
    #[test]
    fn unknown_sibling_top_level_keys_ok() {
        let body = r#"
            [vault]
            visibility = "private"

            [collab.auth]
            methods = ["passkey"]

            [some.other.section]
            anything = "goes"
        "#;
        let cfg = parse(body).unwrap();
        assert_eq!(cfg.methods, vec![MethodId::Passkey]);
    }

    /// Sub-tables are opaque in Phase 1 — they parse and round-trip; strict
    /// schemas land per-phase.
    #[test]
    fn sub_tables_parsed_opaquely() {
        let body = r#"
            [collab.auth]
            methods = ["proxy-header", "agent-token"]

            [collab.auth.proxy_header]
            user_header = "X-Forwarded-User"
            peer_allow  = ["127.0.0.1/32"]
        "#;
        let cfg = parse(body).unwrap();
        assert_eq!(
            cfg.methods,
            vec![MethodId::ProxyHeader, MethodId::AgentToken]
        );
        assert!(cfg.proxy_header.is_some());
    }

    /// REQ-4102 / REQ-4103: the default config builds a `[passkey,
    /// agent-token]` chain.
    #[test]
    fn build_chain_default() {
        let cfg = CollabAuthConfig::default();
        let chain = build_chain(
            &cfg,
            SessionStore::new(),
            Arc::new(std::path::PathBuf::from("/tmp")),
            false,
        )
        .unwrap();
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0].id(), "passkey");
        assert_eq!(chain[1].id(), "agent-token");
    }

    /// REQ-4102 / NFR-4104: chain order matches `methods` order exactly.
    #[test]
    fn build_chain_preserves_order() {
        let body = r#"
            [collab.auth]
            methods = ["agent-token", "passkey"]
        "#;
        let cfg = parse(body).unwrap();
        let chain = build_chain(
            &cfg,
            SessionStore::new(),
            Arc::new(std::path::PathBuf::from("/tmp")),
            false,
        )
        .unwrap();
        assert_eq!(chain[0].id(), "agent-token");
        assert_eq!(chain[1].id(), "passkey");
    }

    /// REQ-4114: methods whose Phase has not yet shipped fail with a clear
    /// message naming the offending method.
    #[test]
    fn build_chain_rejects_unimplemented_methods() {
        for unimplemented in ["oidc"] {
            let body = format!(
                r#"
                [collab.auth]
                methods = ["passkey", "{unimplemented}"]
                "#
            );
            let cfg = parse(&body).unwrap();
            // AuthChain contains `dyn Authenticator` (not Debug), so we can't
            // use `unwrap_err()` directly — match instead.
            let result = build_chain(
                &cfg,
                SessionStore::new(),
                Arc::new(std::path::PathBuf::from("/tmp")),
                false,
            );
            let err = match result {
                Ok(_) => panic!("expected error for unimplemented method {unimplemented}"),
                Err(e) => e,
            };
            assert!(
                err.0.contains(unimplemented),
                "expected error to name {unimplemented}, got: {}",
                err.0
            );
        }
    }

    /// REQ-4106: `proxy-header` named without `--trust-proxy` is a startup
    /// error naming the corrective action.
    #[test]
    fn proxy_header_without_trust_proxy_fails() {
        let body = r#"
            [collab.auth]
            methods = ["proxy-header"]

            [collab.auth.proxy_header]
            peer_allow = ["127.0.0.1/32"]
        "#;
        let cfg = parse(body).unwrap();
        let result = build_chain(
            &cfg,
            SessionStore::new(),
            Arc::new(std::path::PathBuf::from("/tmp")),
            /* trust_proxy */ false,
        );
        let err = match result {
            Ok(_) => panic!("expected REQ-4106 startup error"),
            Err(e) => e,
        };
        assert!(err.0.contains("--trust-proxy"), "got: {}", err.0);
    }

    /// REQ-4106: `proxy-header` named without a `[collab.auth.proxy_header]`
    /// sub-table is a startup error naming the missing section.
    #[test]
    fn proxy_header_without_subtable_fails() {
        let body = r#"
            [collab.auth]
            methods = ["proxy-header"]
        "#;
        let cfg = parse(body).unwrap();
        let result = build_chain(
            &cfg,
            SessionStore::new(),
            Arc::new(std::path::PathBuf::from("/tmp")),
            /* trust_proxy */ true,
        );
        let err = match result {
            Ok(_) => panic!("expected missing-subtable error"),
            Err(e) => e,
        };
        assert!(
            err.0.contains("[collab.auth.proxy_header]"),
            "got: {}",
            err.0
        );
    }

    /// REQ-4105 / REQ-4106: with trust_proxy on and a valid sub-table,
    /// `proxy-header` builds.
    #[test]
    fn proxy_header_builds_with_trust_proxy_and_subtable() {
        let body = r#"
            [collab.auth]
            methods = ["proxy-header"]

            [collab.auth.proxy_header]
            peer_allow = ["127.0.0.1/32", "10.0.0.0/8"]
        "#;
        let cfg = parse(body).unwrap();
        let chain = build_chain(
            &cfg,
            SessionStore::new(),
            Arc::new(std::path::PathBuf::from("/tmp")),
            /* trust_proxy */ true,
        )
        .unwrap();
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].id(), "proxy-header");
    }

    /// OBS-4105: startup summary line lists every method in declared order
    /// and the compiled-in feature set.
    #[test]
    fn format_chain_summary_lists_methods_and_features() {
        let body = r#"
            [collab.auth]
            methods = ["passkey", "agent-token"]
        "#;
        let cfg = parse(body).unwrap();
        let line = format_chain_summary(&cfg);
        assert!(line.starts_with("[zetl] collab auth: "));
        assert!(line.contains("methods=[passkey, agent-token]"));
        assert!(line.contains("features=[collab"));
    }

    /// `build_chain_from_vault` with no config file present yields the
    /// default chain (REQ-4103).
    #[test]
    fn build_chain_from_vault_absent_file_uses_default() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (chain, cfg) = build_chain_from_vault(tmp.path(), SessionStore::new(), false).unwrap();
        assert_eq!(chain.len(), 2);
        assert_eq!(cfg.methods, vec![MethodId::Passkey, MethodId::AgentToken]);
    }

    /// MethodId wire strings round-trip via serde (the kebab-case mapping
    /// matches the audit-log values).
    #[test]
    fn method_id_wire_strings() {
        assert_eq!(MethodId::Passkey.as_str(), "passkey");
        assert_eq!(MethodId::AgentToken.as_str(), "agent-token");
        assert_eq!(MethodId::ProxyHeader.as_str(), "proxy-header");
        assert_eq!(MethodId::Password.as_str(), "password");
        assert_eq!(MethodId::CapabilityUrl.as_str(), "capability-url");
        assert_eq!(MethodId::Oidc.as_str(), "oidc");
    }
}
