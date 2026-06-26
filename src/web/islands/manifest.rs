//! SPEC-050 CON-5002 — island manifest fields recogniser.
//!
//! Interprets the SPEC-050 island fields captured (recognised-and-reserved) by the
//! component manifest parser ([`IslandRaw`](crate::web::components::manifest::IslandRaw))
//! into a typed [`IslandManifest`], applying the CON-5002 pre-conditions. A component's
//! **trust tier** is its `content_invocable` flag (REQ-4910/REQ-5010): a content island
//! may publish only `content:` topics and may not publish a free-string-typed topic.

use super::topic::{recognise_topic, TopicKind};
use super::value_type::{parse_type_expr, recognise_default, ValueType};
use super::{IResult, IslandError};
use crate::web::components::manifest::{IslandRaw, Manifest};
use std::collections::BTreeMap;

/// Island render mode (CON-5002). Worker is the default, secure mode (REQ-5025); iframe
/// is the opt-in full-DOM escape hatch (REQ-5015).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    Worker,
    Iframe,
}

/// Per-island hydration strategy (REQ-5024, after Astro `client:*`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HydrateStrategy {
    Load,
    Idle,
    Visible(Option<String>),
    Media(String),
}

impl HydrateStrategy {
    /// The wire form emitted into the island DOM marker (matches islands.js).
    pub fn as_attr(&self) -> String {
        match self {
            HydrateStrategy::Load => "load".into(),
            HydrateStrategy::Idle => "idle".into(),
            HydrateStrategy::Visible(None) => "visible".into(),
            HydrateStrategy::Visible(Some(m)) => format!("visible({m})"),
            HydrateStrategy::Media(q) => format!("media({q})"),
        }
    }
}

/// A declared topic (`[island.topics].<name>`).
#[derive(Debug, Clone)]
pub struct TopicDecl {
    pub type_expr: String,
    pub ty: ValueType,
    pub persisted: bool,
    pub default: Option<toml::Value>,
}

/// Author capability requests (`[island.requests]`) — inert until an operator approves
/// them in `[security.csp]` (REQ-5028). Never authority.
#[derive(Debug, Clone, Default)]
pub struct Requests {
    pub connect_src: Vec<String>,
    pub bundles: Vec<String>,
    pub reason: Option<String>,
}

/// A recognised island manifest.
#[derive(Debug, Clone)]
pub struct IslandManifest {
    pub component: String,
    /// True when this is a content-author island (REQ-4910 trigger = `content_invocable`).
    pub content_island: bool,
    pub publishes: Vec<String>,
    pub subscribes: Vec<String>,
    pub topics: BTreeMap<String, TopicDecl>,
    pub render: RenderMode,
    pub sandbox: bool,
    pub paints: bool,
    pub hydrate: HydrateStrategy,
    pub requests: Option<Requests>,
}

fn err(code: &'static str, msg: impl Into<String>) -> IslandError {
    IslandError::new(code, msg.into())
}

/// Read a `toml::Value` array-of-strings field.
fn string_array(v: &toml::Value, field: &str) -> IResult<Vec<String>> {
    let arr = v.as_array().ok_or_else(|| {
        err(
            "island-topic-malformed",
            format!("`{field}` must be an array of topic strings"),
        )
    })?;
    let mut out = Vec::new();
    for item in arr {
        let s = item.as_str().ok_or_else(|| {
            err(
                "island-topic-malformed",
                format!("`{field}` entries must be strings"),
            )
        })?;
        out.push(s.to_string());
    }
    Ok(out)
}

/// Parse the island manifest from a component manifest, or `None` if it declares no
/// island fields (a plain SPEC-048 component — REQ-5012 backward compat).
pub fn parse(manifest: &Manifest) -> IResult<Option<IslandManifest>> {
    let raw: &IslandRaw = &manifest.island_raw;
    if raw.is_empty() {
        return Ok(None);
    }
    let component = manifest.name.clone();
    let content_island = manifest.content_invocable;

    let publishes = raw
        .publishes
        .as_ref()
        .map(|v| string_array(v, "publishes"))
        .transpose()?
        .unwrap_or_default();
    let subscribes = raw
        .subscribes
        .as_ref()
        .map(|v| string_array(v, "subscribes"))
        .transpose()?
        .unwrap_or_default();

    // ---- topics (`[island.topics]`) + requests (`[island.requests]`) ----
    let island_tbl = raw.island.as_ref().and_then(|v| v.as_table());
    let mut topics = BTreeMap::new();
    if let Some(topics_tbl) = island_tbl
        .and_then(|t| t.get("topics"))
        .and_then(|v| v.as_table())
    {
        for (name, decl) in topics_tbl {
            recognise_topic(name)?; // CON-5001
            let decl_tbl = decl.as_table().ok_or_else(|| {
                err(
                    "island-topic-malformed",
                    format!("[island.topics.\"{name}\"] must be a table"),
                )
            })?;
            let type_expr = decl_tbl
                .get("type")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    err(
                        "island-topic-type-invalid",
                        format!("topic `{name}` missing `type`"),
                    )
                })?
                .to_string();
            let ty = parse_type_expr(&type_expr)?;
            let persisted = decl_tbl
                .get("persisted")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let default = decl_tbl.get("default").cloned();
            if persisted {
                match &default {
                    Some(d) => recognise_default(d, &ty)?,
                    None => {
                        return Err(err(
                            "island-persisted-no-default",
                            format!("persisted topic `{name}` requires a conforming `default`"),
                        ))
                    }
                }
            } else if let Some(d) = &default {
                // a non-persisted default still must conform if present
                recognise_default(d, &ty)?;
            }
            topics.insert(
                name.clone(),
                TopicDecl {
                    type_expr,
                    ty,
                    persisted,
                    default,
                },
            );
        }
    }

    let requests = island_tbl
        .and_then(|t| t.get("requests"))
        .and_then(|v| v.as_table())
        .map(parse_requests)
        .transpose()?;

    // ---- render / sandbox / paints / hydrate ----
    let render = match raw.render.as_ref().and_then(|v| v.as_str()) {
        None | Some("worker") => RenderMode::Worker,
        Some("iframe") => RenderMode::Iframe,
        Some(other) => {
            return Err(err(
                "island-render-invalid",
                format!("unknown render mode `{other}`"),
            ))
        }
    };
    let sandbox = raw
        .sandbox
        .as_ref()
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    match render {
        RenderMode::Worker if raw.sandbox.is_some() => {
            return Err(err(
                "island-render-invalid",
                "`sandbox` is meaningful only for render=\"iframe\"",
            ));
        }
        RenderMode::Iframe if !sandbox => {
            return Err(err(
                "island-content-unsandboxed",
                "render=\"iframe\" requires sandbox = true",
            ));
        }
        _ => {}
    }
    let paints = raw
        .paints
        .as_ref()
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if matches!(render, RenderMode::Iframe) && raw.paints.is_some() {
        return Err(err(
            "island-render-invalid",
            "`paints` is meaningful only for render=\"worker\"",
        ));
    }
    let hydrate = parse_hydrate(
        raw.hydrate
            .as_ref()
            .and_then(|v| v.as_str())
            .unwrap_or("load"),
    )?;

    // ---- trust-tier pre-conditions (REQ-5010/5011/5022) ----
    for t in &publishes {
        let kind = recognise_topic(t)?;
        // every published/subscribed topic must be declared
        if !topics.contains_key(t) {
            return Err(err(
                "island-topic-malformed",
                format!("published topic `{t}` has no [island.topics] declaration"),
            ));
        }
        if content_island {
            if kind == TopicKind::Trusted {
                return Err(err(
                    "island-capability-ungranted",
                    format!("content island `{component}` may not publish trusted topic `{t}`"),
                ));
            }
            // REQ-5022: content islands may not publish a free string / string-field record.
            if let Some(decl) = topics.get(t) {
                if type_has_free_string(&decl.ty) {
                    return Err(err(
                        "island-content-value-type",
                        format!("content island `{component}` may not publish free-string-typed topic `{t}`"),
                    ));
                }
            }
        }
    }
    for t in &subscribes {
        recognise_topic(t)?;
        if !topics.contains_key(t) {
            return Err(err(
                "island-topic-malformed",
                format!("subscribed topic `{t}` has no [island.topics] declaration"),
            ));
        }
    }

    Ok(Some(IslandManifest {
        component,
        content_island,
        publishes,
        subscribes,
        topics,
        render,
        sandbox,
        paints,
        hydrate,
        requests,
    }))
}

fn parse_requests(tbl: &toml::map::Map<String, toml::Value>) -> IResult<Requests> {
    let arr = |key: &str| -> IResult<Vec<String>> {
        match tbl.get(key) {
            None => Ok(Vec::new()),
            Some(v) => {
                let a = v.as_array().ok_or_else(|| {
                    err(
                        "island-render-invalid",
                        format!("[island.requests].{key} must be an array"),
                    )
                })?;
                a.iter()
                    .map(|x| {
                        x.as_str().map(|s| s.to_string()).ok_or_else(|| {
                            err(
                                "island-render-invalid",
                                format!("[island.requests].{key} entries must be strings"),
                            )
                        })
                    })
                    .collect()
            }
        }
    };
    Ok(Requests {
        connect_src: arr("connect-src")?,
        bundles: arr("bundles")?,
        reason: tbl
            .get("reason")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    })
}

fn parse_hydrate(s: &str) -> IResult<HydrateStrategy> {
    let s = s.trim();
    match s {
        "load" => Ok(HydrateStrategy::Load),
        "idle" => Ok(HydrateStrategy::Idle),
        "visible" => Ok(HydrateStrategy::Visible(None)),
        _ => {
            if let Some(inner) = s.strip_prefix("visible(").and_then(|r| r.strip_suffix(')')) {
                Ok(HydrateStrategy::Visible(Some(inner.trim().to_string())))
            } else if let Some(inner) = s.strip_prefix("media(").and_then(|r| r.strip_suffix(')')) {
                if inner.trim().is_empty() {
                    Err(err("island-hydrate-invalid", "media() requires a query"))
                } else {
                    Ok(HydrateStrategy::Media(inner.trim().to_string()))
                }
            } else {
                Err(err(
                    "island-hydrate-invalid",
                    format!("unknown hydrate strategy `{s}`"),
                ))
            }
        }
    }
}

/// Whether a value type is (or contains) a free, unconstrained string (REQ-5022).
fn type_has_free_string(ty: &ValueType) -> bool {
    use super::value_type::ScalarType;
    match ty {
        ValueType::String => true,
        ValueType::Record(fields) => fields.iter().any(|(_, s)| matches!(s, ScalarType::String)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::components::manifest::parse_manifest;

    fn parse_from(src: &str, dir: &str) -> IResult<Option<IslandManifest>> {
        let m = parse_manifest(src, dir).expect("component manifest parses");
        parse(&m)
    }

    #[test]
    fn plain_component_has_no_island() {
        let m = parse_manifest("name = \"x\"\n[props]\n", "x").unwrap();
        assert!(parse(&m).unwrap().is_none());
    }

    #[test]
    fn trusted_island_parses() {
        let src = r#"
            name = "toggle"
            publishes = ["theme"]
            subscribes = []
            render = "worker"
            paints = true
            hydrate = "idle"
            [island.topics.theme]
            type = "enum(\"light\",\"dark\")"
            persisted = true
            default = "light"
        "#;
        let im = parse_from(src, "toggle").unwrap().unwrap();
        assert!(!im.content_island);
        assert_eq!(im.publishes, vec!["theme".to_string()]);
        assert!(matches!(im.render, RenderMode::Worker));
        assert!(im.paints);
        assert_eq!(im.hydrate, HydrateStrategy::Idle);
        assert!(im.topics["theme"].persisted);
    }

    #[test]
    fn persisted_without_default_rejected() {
        let src = r#"
            name = "t"
            publishes = ["theme"]
            [island.topics.theme]
            type = "string"
            persisted = true
        "#;
        let e = parse_from(src, "t").unwrap_err();
        assert_eq!(e.code, "island-persisted-no-default");
    }

    #[test]
    fn content_island_cannot_publish_trusted_topic() {
        let src = r#"
            name = "poll"
            content_invocable = true
            content_props = []
            publishes = ["theme"]
            [props]
            [island.topics.theme]
            type = "int"
        "#;
        let e = parse_from(src, "poll").unwrap_err();
        assert_eq!(e.code, "island-capability-ungranted");
    }

    #[test]
    fn content_island_cannot_publish_free_string() {
        let src = r#"
            name = "poll"
            content_invocable = true
            content_props = []
            publishes = ["content:msg"]
            [props]
            [island.topics."content:msg"]
            type = "string"
        "#;
        let e = parse_from(src, "poll").unwrap_err();
        assert_eq!(e.code, "island-content-value-type");
    }

    #[test]
    fn content_island_publishes_enum_ok() {
        let src = r#"
            name = "poll"
            content_invocable = true
            content_props = []
            publishes = ["content:vote"]
            render = "worker"
            paints = true
            [props]
            [island.topics."content:vote"]
            type = "enum(\"yes\",\"no\")"
        "#;
        let im = parse_from(src, "poll").unwrap().unwrap();
        assert!(im.content_island);
        assert_eq!(im.publishes, vec!["content:vote".to_string()]);
    }

    #[test]
    fn iframe_requires_sandbox() {
        let src = r#"
            name = "x"
            render = "iframe"
            subscribes = []
        "#;
        let e = parse_from(src, "x").unwrap_err();
        assert_eq!(e.code, "island-content-unsandboxed");
    }

    #[test]
    fn invalid_hydrate_rejected() {
        let src = r#"
            name = "x"
            render = "worker"
            hydrate = "whenever"
            subscribes = []
        "#;
        let e = parse_from(src, "x").unwrap_err();
        assert_eq!(e.code, "island-hydrate-invalid");
    }

    #[test]
    fn undeclared_published_topic_rejected() {
        let src = r#"
            name = "x"
            publishes = ["theme"]
        "#;
        let e = parse_from(src, "x").unwrap_err();
        assert_eq!(e.code, "island-topic-malformed");
    }
}
