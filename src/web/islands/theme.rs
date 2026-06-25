//! SPEC-050 — theme/operator island governance (`[[theme.island-grants]]`, `[security.csp]`).
//!
//! These are **trusted** declarations (CON-5002): a theme author grants a content island
//! read access to a trusted topic (`[[theme.island-grants]]`, subscribe-only), and an
//! operator widens the default-deny CSP baseline (`[security.csp]`, REQ-5027). Content
//! islands can declare neither — they may only *request* (`[island.requests]`).

use super::topic::recognise_topic;
use super::{IResult, IslandError};
use std::collections::BTreeMap;

/// The only grantable direction for a content island on a trusted topic (CON-5002):
/// read (subscribe) only — never publish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Subscribe,
}

/// A theme grant of a trusted-topic read to a content island.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IslandGrant {
    pub component: String,
    pub topic: String,
    pub direction: Direction,
}

/// Operator CSP widenings over the default-deny baseline (REQ-5027). Each key is a CSP
/// directive (`connect-src`, `img-src`, …) and each value a list of host sources. `"*"`
/// is rejected (`csp-wildcard`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CspConfig {
    pub directives: BTreeMap<String, Vec<String>>,
}

/// The CSP directives an operator may widen (closed set; values are host sources).
const WIDENABLE_DIRECTIVES: &[&str] =
    &["connect-src", "img-src", "media-src", "font-src", "style-src", "worker-src", "script-src"];

fn err(code: &'static str, msg: impl Into<String>) -> IslandError {
    IslandError::new(code, msg.into())
}

/// Parse `[[theme.island-grants]]` entries from a parsed `theme.toml` value.
pub fn parse_island_grants(theme_toml: &toml::Value) -> IResult<Vec<IslandGrant>> {
    let Some(arr) = theme_toml
        .get("theme")
        .and_then(|t| t.get("island-grants"))
        .and_then(|v| v.as_array())
    else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for entry in arr {
        let tbl = entry
            .as_table()
            .ok_or_else(|| err("island-grant-malformed", "[[theme.island-grants]] must be a table"))?;
        let component = tbl
            .get("component")
            .and_then(|v| v.as_str())
            .ok_or_else(|| err("island-grant-malformed", "grant missing `component`"))?
            .to_string();
        let topic = tbl
            .get("topic")
            .and_then(|v| v.as_str())
            .ok_or_else(|| err("island-grant-malformed", "grant missing `topic`"))?
            .to_string();
        recognise_topic(&topic)?;
        let direction = match tbl.get("direction").and_then(|v| v.as_str()) {
            Some("subscribe") | None => Direction::Subscribe,
            Some(other) => {
                return Err(err(
                    "island-grant-malformed",
                    format!("grant direction `{other}` invalid (only `subscribe`)"),
                ))
            }
        };
        out.push(IslandGrant { component, topic, direction });
    }
    Ok(out)
}

/// Parse the `[security.csp]` table from a parsed site-config/theme value.
pub fn parse_csp(config: &toml::Value) -> IResult<CspConfig> {
    let Some(tbl) = config
        .get("security")
        .and_then(|s| s.get("csp"))
        .and_then(|v| v.as_table())
    else {
        return Ok(CspConfig::default());
    };
    let mut directives = BTreeMap::new();
    for (key, val) in tbl {
        if !WIDENABLE_DIRECTIVES.contains(&key.as_str()) {
            return Err(err(
                "csp-directive-unknown",
                format!("[security.csp].{key} is not a widenable directive"),
            ));
        }
        let arr = val.as_array().ok_or_else(|| {
            err("csp-directive-invalid", format!("[security.csp].{key} must be an array of host sources"))
        })?;
        let mut hosts = Vec::new();
        for h in arr {
            let s = h.as_str().ok_or_else(|| {
                err("csp-directive-invalid", format!("[security.csp].{key} entries must be strings"))
            })?;
            if s == "*" || s.contains('*') {
                return Err(err("csp-wildcard", format!("[security.csp].{key} may not contain `*`")));
            }
            hosts.push(s.to_string());
        }
        directives.insert(key.clone(), hosts);
    }
    Ok(CspConfig { directives })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn val(s: &str) -> toml::Value {
        s.parse().unwrap()
    }

    #[test]
    fn parses_grants() {
        let t = val(r#"
            [[theme.island-grants]]
            component = "poll"
            topic = "theme"
            direction = "subscribe"
        "#);
        let g = parse_island_grants(&t).unwrap();
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].component, "poll");
        assert_eq!(g[0].topic, "theme");
        assert_eq!(g[0].direction, Direction::Subscribe);
    }

    #[test]
    fn no_grants_ok() {
        let t = val("[theme]\nname = \"x\"\n");
        assert!(parse_island_grants(&t).unwrap().is_empty());
    }

    #[test]
    fn parses_csp() {
        let t = val(r#"
            [security.csp]
            connect-src = ["https://api.example.com"]
            img-src = ["https://cdn.example.com"]
        "#);
        let c = parse_csp(&t).unwrap();
        assert_eq!(c.directives["connect-src"], vec!["https://api.example.com".to_string()]);
        assert_eq!(c.directives["img-src"].len(), 1);
    }

    #[test]
    fn rejects_wildcard() {
        let t = val("[security.csp]\nconnect-src = [\"*\"]\n");
        assert_eq!(parse_csp(&t).unwrap_err().code, "csp-wildcard");
    }

    #[test]
    fn rejects_unknown_directive() {
        let t = val("[security.csp]\nfrob-src = [\"x\"]\n");
        assert_eq!(parse_csp(&t).unwrap_err().code, "csp-directive-unknown");
    }
}
