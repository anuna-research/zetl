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

use serde::Deserialize;

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

    /// `[collab.auth.proxy_header]` — schema filled in by Phase 2
    /// (task-auth-proxy-header).
    #[serde(default)]
    pub proxy_header: Option<toml::Value>,

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
    let lens: ConfigLens =
        toml::from_str(toml_body).map_err(|e| ConfigError(e.to_string()))?;
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
