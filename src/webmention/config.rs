//! `.zetl/config.toml` lens for the `[webmention]` section per CON-3905.
//!
//! Pure parser: hands the shell a fully-validated configuration tree or
//! a structured error naming the offending key.

use serde::{Deserialize, Serialize};

/// Default rate-limit thresholds per NFR-3906.
pub const DEFAULT_RATE_LIMIT_PER_HOST_PER_MIN: u32 = 60;
pub const DEFAULT_RATE_LIMIT_GLOBAL_PER_MIN: u32 = 1000;
/// Default endpoint path. The W3C REC says `rel=webmention` discovery
/// can point anywhere; we standardise on `/webmention` so the `zetl
/// serve` route and the static-build `<link>` tag agree.
pub const DEFAULT_ENDPOINT_PATH: &str = "/webmention";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebmentionConfig {
    pub enabled: bool,
    pub receive_enabled: bool,
    pub send_enabled: bool,
    pub endpoint_path: String,
    pub default_decision: ModerationDefault,
    pub allowlist_domains: Vec<String>,
    pub denylist_domains: Vec<String>,
    pub rate_limit_per_source_host_per_minute: u32,
    pub rate_limit_global_per_minute: u32,
    /// Override the per-fetch body cap. `None` -> reuse SPEC-038's
    /// `MAX_BODY_BYTES` (1 MiB).
    pub max_body_bytes_override: Option<u64>,
}

impl Default for WebmentionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            receive_enabled: true,
            send_enabled: true,
            endpoint_path: DEFAULT_ENDPOINT_PATH.to_string(),
            default_decision: ModerationDefault::Queue,
            allowlist_domains: Vec::new(),
            denylist_domains: Vec::new(),
            rate_limit_per_source_host_per_minute: DEFAULT_RATE_LIMIT_PER_HOST_PER_MIN,
            rate_limit_global_per_minute: DEFAULT_RATE_LIMIT_GLOBAL_PER_MIN,
            max_body_bytes_override: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModerationDefault {
    Accept,
    Queue,
    Deny,
}

#[derive(Debug, thiserror::Error)]
pub enum WebmentionConfigError {
    #[error("invalid value for [webmention].{key}: {reason}")]
    Invalid { key: &'static str, reason: String },
    #[error("toml parse error: {0}")]
    Toml(String),
}

impl WebmentionConfig {
    /// Parse a `.zetl/config.toml` body. Recognises only the
    /// `[webmention]` table; unknown sections elsewhere in the document
    /// are passed through (the wider config has many consumers).
    ///
    /// Named `from_str` to mirror the existing
    /// [`crate::feed::config::FeedConfigLens::from_str`] convention.
    /// Not implemented as `std::str::FromStr` because the error type
    /// carries borrowed `&'static str` keys that don't match the trait
    /// bound.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(toml_body: &str) -> Result<Self, WebmentionConfigError> {
        #[derive(Deserialize)]
        struct Outer {
            #[serde(default)]
            webmention: Option<RawSection>,
        }

        #[derive(Deserialize)]
        struct RawSection {
            #[serde(default)]
            enabled: Option<bool>,
            #[serde(default)]
            receive_enabled: Option<bool>,
            #[serde(default)]
            send_enabled: Option<bool>,
            #[serde(default)]
            endpoint_path: Option<String>,
            #[serde(default)]
            default_decision: Option<String>,
            #[serde(default)]
            allowlist_domains: Vec<String>,
            #[serde(default)]
            denylist_domains: Vec<String>,
            #[serde(default)]
            rate_limit_per_source_host_per_minute: Option<u32>,
            #[serde(default)]
            rate_limit_global_per_minute: Option<u32>,
            #[serde(default)]
            max_body_bytes_override: Option<u64>,
        }

        let outer: Outer =
            toml::from_str(toml_body).map_err(|e| WebmentionConfigError::Toml(e.to_string()))?;
        let raw = outer.webmention.unwrap_or(RawSection {
            enabled: None,
            receive_enabled: None,
            send_enabled: None,
            endpoint_path: None,
            default_decision: None,
            allowlist_domains: Vec::new(),
            denylist_domains: Vec::new(),
            rate_limit_per_source_host_per_minute: None,
            rate_limit_global_per_minute: None,
            max_body_bytes_override: None,
        });

        let enabled = raw.enabled.unwrap_or(false);

        let endpoint_path = raw
            .endpoint_path
            .unwrap_or_else(|| DEFAULT_ENDPOINT_PATH.to_string());
        if !endpoint_path.starts_with('/') {
            return Err(WebmentionConfigError::Invalid {
                key: "endpoint_path",
                reason: format!("must start with '/' (got `{endpoint_path}`)"),
            });
        }

        let default_decision = match raw.default_decision.as_deref() {
            None | Some("queue") => ModerationDefault::Queue,
            Some("accept") => ModerationDefault::Accept,
            Some("deny") => ModerationDefault::Deny,
            Some(other) => {
                return Err(WebmentionConfigError::Invalid {
                    key: "default_decision",
                    reason: format!("expected one of accept|queue|deny (got `{other}`)"),
                });
            }
        };

        // Domain-list normalisation: lowercase + strip whitespace, reject
        // empty entries (configuration typo guard).
        let allowlist_domains = normalise_domains(&raw.allowlist_domains, "allowlist_domains")?;
        let denylist_domains = normalise_domains(&raw.denylist_domains, "denylist_domains")?;

        Ok(Self {
            enabled,
            receive_enabled: raw.receive_enabled.unwrap_or(true),
            send_enabled: raw.send_enabled.unwrap_or(true),
            endpoint_path,
            default_decision,
            allowlist_domains,
            denylist_domains,
            rate_limit_per_source_host_per_minute: raw
                .rate_limit_per_source_host_per_minute
                .unwrap_or(DEFAULT_RATE_LIMIT_PER_HOST_PER_MIN),
            rate_limit_global_per_minute: raw
                .rate_limit_global_per_minute
                .unwrap_or(DEFAULT_RATE_LIMIT_GLOBAL_PER_MIN),
            max_body_bytes_override: raw.max_body_bytes_override,
        })
    }

    /// True iff the config wants any webmention work at all (receive or
    /// send) — the discovery emission, sender hook, and serve-route
    /// registration all key off this.
    pub fn any_enabled(&self) -> bool {
        self.enabled && (self.receive_enabled || self.send_enabled)
    }
}

fn normalise_domains(
    raw: &[String],
    key: &'static str,
) -> Result<Vec<String>, WebmentionConfigError> {
    let mut out = Vec::with_capacity(raw.len());
    for d in raw {
        let trimmed = d.trim().to_ascii_lowercase();
        if trimmed.is_empty() {
            return Err(WebmentionConfigError::Invalid {
                key,
                reason: "empty domain entry".to_string(),
            });
        }
        out.push(trimmed);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_body_yields_disabled_default() {
        let cfg = WebmentionConfig::from_str("").unwrap();
        assert!(!cfg.enabled);
        assert!(!cfg.any_enabled());
        assert_eq!(cfg.endpoint_path, "/webmention");
        assert_eq!(cfg.default_decision, ModerationDefault::Queue);
    }

    #[test]
    fn minimal_enabled_table() {
        let cfg = WebmentionConfig::from_str("[webmention]\nenabled = true\n").unwrap();
        assert!(cfg.enabled);
        assert!(cfg.any_enabled());
        assert!(cfg.receive_enabled);
        assert!(cfg.send_enabled);
    }

    #[test]
    fn fully_specified_table() {
        let body = r#"
[webmention]
enabled = true
receive_enabled = true
send_enabled = false
endpoint_path = "/wm"
default_decision = "accept"
allowlist_domains = ["Friend.example", " Other.example "]
denylist_domains = ["spammer.example"]
rate_limit_per_source_host_per_minute = 30
rate_limit_global_per_minute = 500
max_body_bytes_override = 524288
"#;
        let cfg = WebmentionConfig::from_str(body).unwrap();
        assert!(cfg.enabled);
        assert!(!cfg.send_enabled);
        assert_eq!(cfg.endpoint_path, "/wm");
        assert_eq!(cfg.default_decision, ModerationDefault::Accept);
        assert_eq!(
            cfg.allowlist_domains,
            vec!["friend.example".to_string(), "other.example".to_string()]
        );
        assert_eq!(cfg.denylist_domains, vec!["spammer.example".to_string()]);
        assert_eq!(cfg.rate_limit_per_source_host_per_minute, 30);
        assert_eq!(cfg.max_body_bytes_override, Some(524288));
    }

    #[test]
    fn rejects_relative_endpoint_path() {
        let body = "[webmention]\nendpoint_path = \"webmention\"\n";
        let err = WebmentionConfig::from_str(body).unwrap_err();
        assert!(matches!(
            err,
            WebmentionConfigError::Invalid {
                key: "endpoint_path",
                ..
            }
        ));
    }

    #[test]
    fn rejects_unknown_default_decision() {
        let body = "[webmention]\ndefault_decision = \"maybe\"\n";
        let err = WebmentionConfig::from_str(body).unwrap_err();
        assert!(matches!(
            err,
            WebmentionConfigError::Invalid {
                key: "default_decision",
                ..
            }
        ));
    }

    #[test]
    fn rejects_empty_domain_entry() {
        let body = "[webmention]\nallowlist_domains = [\"good.example\", \"\"]\n";
        let err = WebmentionConfig::from_str(body).unwrap_err();
        assert!(matches!(
            err,
            WebmentionConfigError::Invalid {
                key: "allowlist_domains",
                ..
            }
        ));
    }
}
