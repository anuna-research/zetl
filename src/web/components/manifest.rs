//! SPEC-048 CON-4801 — Component Manifest (`<name>.toml`) recogniser.
//!
//! A strict TOML parser (LangSec: recognise before act). Unknown top-level keys are
//! rejected — the manifest is zetl-defined, not an external standard — so the keys
//! `publishes`/`subscribes` reserved for SPEC-050 (islands) are rejected as unknown in
//! v1. Out-of-grammar input yields `component-malformed`, never a partial accept.

use super::{CResult, ComponentError};
use serde::Deserialize;
use std::collections::BTreeMap;

/// NFR-4803 fail-closed bound: a manifest file may not exceed 64 KiB.
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;

/// A render-context tier a component may require (REQ-4801).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Site,
    Page,
    Folder,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Site => "site",
            Tier::Page => "page",
            Tier::Folder => "folder",
        }
    }

    fn parse(s: &str) -> Option<Tier> {
        match s {
            "site" => Some(Tier::Site),
            "page" => Some(Tier::Page),
            "folder" => Some(Tier::Folder),
            _ => None,
        }
    }
}

/// Declared type of a prop (CON-4801 `ptype`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropType {
    String,
    Bool,
    Int,
    Number,
    List,
    Map,
}

impl PropType {
    pub fn as_str(self) -> &'static str {
        match self {
            PropType::String => "string",
            PropType::Bool => "bool",
            PropType::Int => "int",
            PropType::Number => "number",
            PropType::List => "list",
            PropType::Map => "map",
        }
    }

    fn parse(s: &str) -> Option<PropType> {
        match s {
            "string" => Some(PropType::String),
            "bool" => Some(PropType::Bool),
            "int" => Some(PropType::Int),
            "number" => Some(PropType::Number),
            "list" => Some(PropType::List),
            "map" => Some(PropType::Map),
            _ => None,
        }
    }
}

/// A single prop definition from the manifest `[props]` table.
#[derive(Debug, Clone)]
pub struct PropDef {
    pub ty: PropType,
    pub required: bool,
    pub default: Option<toml::Value>,
    /// Allowed value set; `None` means unconstrained.
    pub enum_values: Option<Vec<toml::Value>>,
}

/// A recognised, typed component manifest.
#[derive(Debug, Clone)]
pub struct Manifest {
    pub name: String,
    pub requires: Vec<Tier>,
    /// Declared named slots (the default slot is always implicit).
    pub slots: Vec<String>,
    pub props: BTreeMap<String, PropDef>,
}

impl Manifest {
    pub fn requires_tier(&self, tier: Tier) -> bool {
        self.requires.contains(&tier)
    }
}

// ---- raw deserialisation (deny_unknown_fields gives us strict recognition) ----

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    name: String,
    #[serde(default)]
    requires: Vec<String>,
    #[serde(default)]
    slots: Vec<String>,
    #[serde(default)]
    props: BTreeMap<String, RawProp>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProp {
    #[serde(rename = "type")]
    ty: String,
    #[serde(default)]
    required: bool,
    default: Option<toml::Value>,
    #[serde(rename = "enum")]
    enum_values: Option<Vec<toml::Value>>,
}

/// `<name>` must be kebab-case: `[a-z][a-z0-9-]*` (REQ-4803).
pub fn is_valid_component_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Parse a manifest from TOML `src`, verifying it matches the directory name
/// `dir_name`. Returns `component-malformed` on any grammar violation.
pub fn parse_manifest(src: &str, dir_name: &str) -> CResult<Manifest> {
    if src.len() > MAX_MANIFEST_BYTES {
        return Err(ComponentError::new(
            "component-malformed",
            format!(
                "manifest exceeds {} KiB cap ({} bytes)",
                MAX_MANIFEST_BYTES / 1024,
                src.len()
            ),
        ));
    }

    let raw: RawManifest = toml::from_str(src).map_err(|e| {
        ComponentError::new(
            "component-malformed",
            format!("manifest parse error: {}", e.message()),
        )
    })?;

    if !is_valid_component_name(&raw.name) {
        return Err(ComponentError::new(
            "component-malformed",
            format!("name `{}` is not kebab-case ([a-z][a-z0-9-]*)", raw.name),
        ));
    }

    if raw.name != dir_name {
        return Err(ComponentError::new(
            "component-malformed",
            format!(
                "manifest name `{}` must equal directory name `{}`",
                raw.name, dir_name
            ),
        ));
    }

    let mut requires = Vec::with_capacity(raw.requires.len());
    for tier in &raw.requires {
        match Tier::parse(tier) {
            Some(t) => requires.push(t),
            None => {
                return Err(ComponentError::new(
                    "component-malformed",
                    format!("unknown tier `{tier}` in requires (allowed: site|page|folder)"),
                ));
            }
        }
    }

    let mut props = BTreeMap::new();
    for (key, raw_prop) in raw.props {
        let ty = PropType::parse(&raw_prop.ty).ok_or_else(|| {
            ComponentError::new(
                "component-malformed",
                format!("prop `{key}`: unknown type `{}`", raw_prop.ty),
            )
        })?;
        props.insert(
            key,
            PropDef {
                ty,
                required: raw_prop.required,
                default: raw_prop.default,
                enum_values: raw_prop.enum_values,
            },
        );
    }

    Ok(Manifest {
        name: raw.name,
        requires,
        slots: raw.slots,
        props,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wellformed_manifest() {
        let src = r#"
            name = "callout"
            requires = ["site"]
            slots = ["header"]
            [props]
            tone = { type = "string", required = true, enum = ["info", "warning"] }
            dismissible = { type = "bool", default = false }
        "#;
        let m = parse_manifest(src, "callout").expect("should parse");
        assert_eq!(m.name, "callout");
        assert_eq!(m.requires, vec![Tier::Site]);
        assert_eq!(m.slots, vec!["header".to_string()]);
        assert_eq!(m.props.len(), 2);
        let tone = &m.props["tone"];
        assert_eq!(tone.ty, PropType::String);
        assert!(tone.required);
        assert!(tone.enum_values.is_some());
        assert!(!m.props["dismissible"].required);
    }

    #[test]
    fn rejects_name_dir_mismatch() {
        let src = r#"name = "callout""#;
        let err = parse_manifest(src, "card").unwrap_err();
        assert_eq!(err.code, "component-malformed");
        assert!(err.message.contains("directory"));
    }

    #[test]
    fn rejects_non_kebab_name() {
        let src = r#"name = "Callout""#;
        let err = parse_manifest(src, "Callout").unwrap_err();
        assert_eq!(err.code, "component-malformed");
    }

    #[test]
    fn rejects_unknown_top_level_key() {
        // `publishes` is reserved for SPEC-050 islands; rejected as unknown in v1.
        let src = r#"
            name = "toggle"
            publishes = ["theme"]
        "#;
        let err = parse_manifest(src, "toggle").unwrap_err();
        assert_eq!(err.code, "component-malformed");
    }

    #[test]
    fn rejects_unknown_tier() {
        let src = r#"
            name = "x"
            requires = ["galaxy"]
        "#;
        let err = parse_manifest(src, "x").unwrap_err();
        assert_eq!(err.code, "component-malformed");
        assert!(err.message.contains("galaxy"));
    }

    #[test]
    fn rejects_unknown_prop_type() {
        let src = r#"
            name = "x"
            [props]
            foo = { type = "widget" }
        "#;
        let err = parse_manifest(src, "x").unwrap_err();
        assert_eq!(err.code, "component-malformed");
    }

    #[test]
    fn name_validation() {
        assert!(is_valid_component_name("nav-header"));
        assert!(is_valid_component_name("a1"));
        assert!(!is_valid_component_name("Nav"));
        assert!(!is_valid_component_name("1nav"));
        assert!(!is_valid_component_name("nav_header"));
        assert!(!is_valid_component_name(""));
    }
}
