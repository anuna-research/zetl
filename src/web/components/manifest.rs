//! SPEC-048 CON-4801 — Component Manifest (`<name>.toml`) recogniser, extended by
//! SPEC-049 CON-4903 (content-authoring gate + `url` ptype) and SPEC-050 CON-5002
//! (island fields).
//!
//! A strict TOML parser (LangSec: recognise before act). Genuinely-unknown top-level
//! keys are still rejected (`component-malformed`) — the manifest is zetl-defined, not
//! an external standard. The SPEC-049 keys `content_invocable`/`content_props` and the
//! SPEC-050 island keys (`publishes`/`subscribes`/`render`/`sandbox`/`paints`/`hydrate`
//! and the `[island]` table) are **recognised-and-reserved**: they are accepted (so a
//! manifest annotated for a successor still builds under a gate-off build, CON-4903
//! forward-compat) and the island detail is left to [`crate::web::islands::manifest`].
//! Out-of-grammar input yields `component-malformed`, never a partial accept.

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

/// Declared type of a prop (CON-4801 `ptype`, extended by SPEC-049 CON-4903 with
/// `url`). `Url` is a `string` whose value is scheme-validated per CON-4902 wherever it
/// lands in a URL context; for trusted-author (SPEC-048) validation it behaves exactly
/// like `String`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropType {
    String,
    Bool,
    Int,
    Number,
    List,
    Map,
    /// SPEC-049 CON-4903(a): a `string` ingestion-validated as a URL.
    Url,
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
            PropType::Url => "url",
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
            "url" => Some(PropType::Url),
            _ => None,
        }
    }

    /// Whether a prop of this type may be set from untrusted content (REQ-4904): only
    /// the scalar types and `url`. `list`/`map` are not content-settable in v1.
    pub fn is_content_settable(self) -> bool {
        matches!(
            self,
            PropType::String | PropType::Bool | PropType::Int | PropType::Number | PropType::Url
        )
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

/// Raw, un-interpreted SPEC-050 island fields, captured during manifest recognition so
/// the manifest stays one parse while the island detail (topic/type grammars, render
/// mode, grants) is recognised by [`crate::web::islands::manifest`]. Empty for a plain
/// SPEC-048 component.
#[derive(Debug, Clone, Default)]
pub struct IslandRaw {
    pub publishes: Option<toml::Value>,
    pub subscribes: Option<toml::Value>,
    pub render: Option<toml::Value>,
    pub sandbox: Option<toml::Value>,
    pub paints: Option<toml::Value>,
    pub hydrate: Option<toml::Value>,
    /// The `[island]` table (`[island.topics]`, `[island.requests]`).
    pub island: Option<toml::Value>,
}

impl IslandRaw {
    /// True when the manifest declares no island fields at all — the component ships no
    /// island and SPEC-050 emits nothing for it (REQ-5012 backward compat).
    pub fn is_empty(&self) -> bool {
        self.publishes.is_none()
            && self.subscribes.is_none()
            && self.render.is_none()
            && self.sandbox.is_none()
            && self.paints.is_none()
            && self.hydrate.is_none()
            && self.island.is_none()
    }
}

/// A recognised, typed component manifest.
#[derive(Debug, Clone)]
pub struct Manifest {
    pub name: String,
    pub requires: Vec<Tier>,
    /// Declared named slots (the default slot is always implicit).
    pub slots: Vec<String>,
    pub props: BTreeMap<String, PropDef>,
    /// SPEC-049 REQ-4903: theme-author opt-in making this component invocable from
    /// untrusted content via a `:::name{…}` directive. Default-deny (`false`).
    pub content_invocable: bool,
    /// SPEC-049 CON-4903(b): the exact props settable from content. Default `[]` (the
    /// narrowest surface — a prop is content-settable only if explicitly listed here).
    pub content_props: Vec<String>,
    /// SPEC-050 island fields, captured raw (see [`IslandRaw`]).
    pub island_raw: IslandRaw,
}

impl Manifest {
    pub fn requires_tier(&self, tier: Tier) -> bool {
        self.requires.contains(&tier)
    }

    /// Whether `prop` may be set from untrusted content (SPEC-049 REQ-4904): the
    /// component is `content_invocable` AND the prop is listed in `content_props`.
    pub fn is_content_settable_prop(&self, prop: &str) -> bool {
        self.content_invocable && self.content_props.iter().any(|p| p == prop)
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
    // ---- SPEC-049 CON-4903 (recognised-and-reserved) ----
    #[serde(default)]
    content_invocable: bool,
    #[serde(default)]
    content_props: Vec<String>,
    // ---- SPEC-050 CON-5002 island fields (recognised-and-reserved; detail parsed by
    // the islands module so this stays one manifest parse) ----
    publishes: Option<toml::Value>,
    subscribes: Option<toml::Value>,
    render: Option<toml::Value>,
    sandbox: Option<toml::Value>,
    paints: Option<toml::Value>,
    hydrate: Option<toml::Value>,
    island: Option<toml::Value>,
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

    let island_raw = IslandRaw {
        publishes: raw.publishes,
        subscribes: raw.subscribes,
        render: raw.render,
        sandbox: raw.sandbox,
        paints: raw.paints,
        hydrate: raw.hydrate,
        island: raw.island,
    };

    let manifest = Manifest {
        name: raw.name,
        requires,
        slots: raw.slots,
        props,
        content_invocable: raw.content_invocable,
        content_props: raw.content_props,
        island_raw,
    };

    // SPEC-049 CON-4903: the content-authoring gate is validated only when the
    // `content-components` feature is active. Under gate-off these keys are
    // accepted-and-ignored (REQ-4912 forward-compat), so no error here.
    #[cfg(feature = "content-components")]
    validate_content_gate(&manifest)?;

    Ok(manifest)
}

/// SPEC-049 CON-4903 pre-conditions on the content-authoring gate. A `content_props`
/// entry MUST name a declared prop that is scalar/`url`-typed; a `content_invocable`
/// component MUST be able to satisfy every required prop from content or a default.
#[cfg(feature = "content-components")]
fn validate_content_gate(m: &Manifest) -> CResult<()> {
    for cp in &m.content_props {
        match m.props.get(cp) {
            None => {
                return Err(ComponentError::new(
                    "content-manifest-unknown-ref",
                    format!("content_props names undeclared prop `{cp}`"),
                ));
            }
            Some(def) if !def.ty.is_content_settable() => {
                return Err(ComponentError::new(
                    "content-prop-unsupported",
                    format!(
                        "content_props `{cp}` has type `{}`; only string/bool/int/number/url are content-settable",
                        def.ty.as_str()
                    ),
                ));
            }
            Some(_) => {}
        }
    }

    if m.content_invocable {
        // Every required prop must be fulfillable from content (content-settable) or
        // carry a default — else a content directive could never satisfy it
        // (`content-invocable-unfulfillable`).
        for (name, def) in &m.props {
            if def.required && def.default.is_none() && !m.content_props.iter().any(|p| p == name) {
                return Err(ComponentError::new(
                    "content-invocable-unfulfillable",
                    format!(
                        "content_invocable component requires prop `{name}` but it is neither content-settable nor defaulted"
                    ),
                ));
            }
        }
    }

    Ok(())
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
    fn rejects_genuinely_unknown_top_level_key() {
        let src = r#"
            name = "toggle"
            frobnicate = true
        "#;
        let err = parse_manifest(src, "toggle").unwrap_err();
        assert_eq!(err.code, "component-malformed");
    }

    #[test]
    fn reserves_island_keys_instead_of_rejecting() {
        // SPEC-050 `publishes`/`subscribes` and friends are recognised-and-reserved
        // (CON-4903 forward-compat), captured into island_raw, not rejected.
        let src = r#"
            name = "toggle"
            publishes = ["content:filter"]
            subscribes = ["theme"]
            render = "worker"
            paints = true
            hydrate = "visible"
            [island.topics."content:filter"]
            type = "string"
        "#;
        let m = parse_manifest(src, "toggle").expect("reserved island keys accepted");
        assert!(!m.island_raw.is_empty());
        assert!(m.island_raw.publishes.is_some());
        assert!(m.island_raw.island.is_some());
    }

    #[test]
    fn url_ptype_parses() {
        let src = r#"
            name = "link-card"
            [props]
            href = { type = "url", required = true }
        "#;
        let m = parse_manifest(src, "link-card").expect("url ptype parses");
        assert_eq!(m.props["href"].ty, PropType::Url);
        assert!(m.props["href"].ty.is_content_settable());
    }

    #[test]
    fn content_invocable_and_props_parse() {
        let src = r#"
            name = "callout"
            content_invocable = true
            content_props = ["tone"]
            [props]
            tone = { type = "string", default = "info" }
        "#;
        let m = parse_manifest(src, "callout").expect("content gate parses");
        assert!(m.content_invocable);
        assert_eq!(m.content_props, vec!["tone".to_string()]);
        assert!(m.is_content_settable_prop("tone"));
        assert!(!m.is_content_settable_prop("other"));
    }

    #[cfg(feature = "content-components")]
    #[test]
    fn content_props_naming_undeclared_prop_rejected() {
        let src = r#"
            name = "callout"
            content_invocable = true
            content_props = ["nope"]
            [props]
            tone = { type = "string", default = "info" }
        "#;
        let err = parse_manifest(src, "callout").unwrap_err();
        assert_eq!(err.code, "content-manifest-unknown-ref");
    }

    #[cfg(feature = "content-components")]
    #[test]
    fn content_props_with_list_type_rejected() {
        let src = r#"
            name = "gallery"
            content_invocable = true
            content_props = ["items"]
            [props]
            items = { type = "list" }
        "#;
        let err = parse_manifest(src, "gallery").unwrap_err();
        assert_eq!(err.code, "content-prop-unsupported");
    }

    #[cfg(feature = "content-components")]
    #[test]
    fn content_invocable_unfulfillable_required_prop_rejected() {
        // `tone` is required, has no default, and is not content-settable.
        let src = r#"
            name = "callout"
            content_invocable = true
            [props]
            tone = { type = "string", required = true }
        "#;
        let err = parse_manifest(src, "callout").unwrap_err();
        assert_eq!(err.code, "content-invocable-unfulfillable");
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
