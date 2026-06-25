//! SPEC-049 REQ-4904 — untrusted prop recognition for the content path.
//!
//! Recognises a directive's `{attrs}` against the component's `[props]` schema, **in the
//! component-resolution layer** (not minijinja), strictly and fail-closed. Only props the
//! theme explicitly marked content-settable (`content_invocable` + `content_props`,
//! CON-4903) are accepted; every other key is `content-prop-unknown`. Scalar lexemes are
//! coerced to the declared ptype by a single recogniser; a `url`-typed value is
//! **ingestion-validated** (scheme allowlist, [`super::sanitize::is_safe_url`]) so it is
//! safe wherever the template later places it.
//!
//! Recognised content values are **tainted** ([`Recognised::tainted`]). The
//! `{{ x | safe }}` escaper-bypass of a tainted value is rejected **statically** by the
//! CON-4904 context lint (the v0.3.0 runtime tripwire became a build-time rejection per
//! the spec); this module records taint so the lint and the expansion path know which
//! values crossed the trust boundary.

use super::directive::Attr;
use super::manifest::{Manifest, PropType};
use super::sanitize::is_safe_url;
use super::{CResult, ComponentError};
use minijinja::Value;
use std::collections::{BTreeMap, BTreeSet};

/// The recognised, validated content props for one directive invocation.
#[derive(Debug, Clone, Default)]
pub struct Recognised {
    /// All props bound for the component render: content-set values plus manifest
    /// defaults for the rest.
    pub props: BTreeMap<String, Value>,
    /// The subset of `props` whose value came from untrusted content (tainted).
    pub tainted: BTreeSet<String>,
}

/// Collapse a directive's ordered attrs into raw key→string-lexeme pairs, applying the
/// `#id`→`id` and `.class`→`class` (space-accumulated) shorthands (CON-4901).
fn collect_raw(attrs: &[Attr]) -> BTreeMap<String, String> {
    let mut raw: BTreeMap<String, String> = BTreeMap::new();
    for attr in attrs {
        match attr {
            Attr::Id(v) => {
                raw.insert("id".to_string(), v.clone());
            }
            Attr::Class(v) => {
                raw.entry("class".to_string())
                    .and_modify(|existing| {
                        existing.push(' ');
                        existing.push_str(v);
                    })
                    .or_insert_with(|| v.clone());
            }
            Attr::KeyValue { key, value } => {
                raw.insert(key.clone(), value.clone());
            }
            Attr::Flag(key) => {
                raw.insert(key.clone(), "true".to_string());
            }
        }
    }
    raw
}

/// Coerce a string lexeme to the declared ptype, fail-closed (CON-4901 one-ptype
/// recogniser). `url` is ingestion-validated here.
fn coerce(key: &str, ty: PropType, lexeme: &str, comp: &str) -> CResult<Value> {
    let type_err = |want: &str| {
        ComponentError::new(
            "content-prop-type",
            format!("content prop `{key}` for `{comp}`: `{lexeme}` is not a valid {want}"),
        )
    };
    match ty {
        PropType::String => Ok(Value::from(lexeme.to_string())),
        PropType::Url => {
            if is_safe_url(lexeme) {
                Ok(Value::from(lexeme.to_string()))
            } else {
                Err(ComponentError::new(
                    "content-prop-type",
                    format!(
                        "content prop `{key}` for `{comp}`: URL `{lexeme}` failed scheme validation"
                    ),
                ))
            }
        }
        PropType::Bool => match lexeme {
            "true" => Ok(Value::from(true)),
            "false" => Ok(Value::from(false)),
            _ => Err(type_err("bool")),
        },
        PropType::Int => lexeme
            .parse::<i64>()
            .map(Value::from)
            .map_err(|_| type_err("int")),
        PropType::Number => lexeme
            .parse::<f64>()
            .map(Value::from)
            .map_err(|_| type_err("number")),
        // Defensive: list/map are not content-settable (caught at manifest, CON-4903).
        PropType::List | PropType::Map => Err(ComponentError::new(
            "content-prop-unsupported",
            format!("content prop `{key}` for `{comp}`: {} is not content-settable", ty.as_str()),
        )),
    }
}

/// Check a coerced value against the prop's `enum` constraint, if any.
fn check_enum(key: &str, def: &super::manifest::PropDef, v: &Value, comp: &str) -> CResult<()> {
    let Some(allowed) = &def.enum_values else {
        return Ok(());
    };
    let got = v.as_str().map(|s| s.to_string()).unwrap_or_else(|| v.to_string());
    let ok = allowed.iter().any(|a| match a {
        toml::Value::String(s) => *s == got,
        other => other.to_string() == got,
    });
    if ok {
        Ok(())
    } else {
        Err(ComponentError::new(
            "content-prop-enum",
            format!("content prop `{key}` value `{got}` not in declared enum for `{comp}`"),
        ))
    }
}

/// Recognise the content props for a directive invocation against `manifest`.
///
/// Errors: `content-prop-unknown` (key not content-settable), `content-prop-type`
/// (coercion/url failure), `content-prop-enum`, `content-prop-missing`,
/// `content-prop-unsupported`.
pub fn recognise(attrs: &[Attr], manifest: &Manifest) -> CResult<Recognised> {
    let comp = manifest.name.as_str();
    let raw = collect_raw(attrs);

    let mut out = Recognised::default();

    // 1. Every supplied key must be a content-settable prop (default-deny).
    for (key, lexeme) in &raw {
        if !manifest.is_content_settable_prop(key) {
            return Err(ComponentError::new(
                "content-prop-unknown",
                format!("prop `{key}` is not content-settable on `{comp}`"),
            ));
        }
        // safe: is_content_settable_prop implies the prop exists.
        let def = &manifest.props[key];
        let value = coerce(key, def.ty, lexeme, comp)?;
        check_enum(key, def, &value, comp)?;
        out.props.insert(key.clone(), value);
        out.tainted.insert(key.clone());
    }

    // 2. Fill manifest defaults for props the author did not set, and check that every
    //    required prop ends up bound.
    for (name, def) in &manifest.props {
        if out.props.contains_key(name) {
            continue;
        }
        if let Some(default) = &def.default {
            out.props
                .insert(name.clone(), super::validate::toml_to_value(default));
        } else if def.required {
            return Err(ComponentError::new(
                "content-prop-missing",
                format!("required prop `{name}` missing for content invocation of `{comp}`"),
            ));
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::components::manifest::parse_manifest;

    fn manifest(src: &str, dir: &str) -> Manifest {
        parse_manifest(src, dir).expect("manifest parses")
    }

    const CALLOUT: &str = r#"
        name = "callout"
        content_invocable = true
        content_props = ["tone", "count", "href", "open"]
        [props]
        tone = { type = "string", default = "info", enum = ["info", "warning"] }
        count = { type = "int", default = 0 }
        href = { type = "url", required = false }
        open = { type = "bool", default = false }
        secret = { type = "string", default = "x" }
    "#;

    fn attrs(pairs: &[(&str, &str)]) -> Vec<Attr> {
        pairs
            .iter()
            .map(|(k, v)| Attr::KeyValue { key: k.to_string(), value: v.to_string() })
            .collect()
    }

    #[test]
    fn recognises_scalars_and_defaults() {
        let m = manifest(CALLOUT, "callout");
        let r = recognise(&attrs(&[("tone", "warning"), ("count", "3")]), &m).unwrap();
        assert_eq!(r.props["tone"].as_str(), Some("warning"));
        assert_eq!(i64::try_from(r.props["count"].clone()).unwrap(), 3);
        // default filled for the unset bool
        assert_eq!(r.props["open"].is_true(), false);
        // tainted = only the content-set ones
        assert!(r.tainted.contains("tone"));
        assert!(r.tainted.contains("count"));
        assert!(!r.tainted.contains("open"));
        assert!(!r.tainted.contains("secret"));
    }

    #[test]
    fn url_ingestion_validation() {
        let m = manifest(CALLOUT, "callout");
        let ok = recognise(&attrs(&[("href", "https://example.com")]), &m).unwrap();
        assert_eq!(ok.props["href"].as_str(), Some("https://example.com"));
        let err = recognise(&attrs(&[("href", "javascript:alert")]), &m).unwrap_err();
        assert_eq!(err.code, "content-prop-type");
    }

    #[test]
    fn non_content_settable_prop_rejected() {
        let m = manifest(CALLOUT, "callout");
        // `secret` is a declared prop but NOT in content_props.
        let err = recognise(&attrs(&[("secret", "x")]), &m).unwrap_err();
        assert_eq!(err.code, "content-prop-unknown");
    }

    #[test]
    fn unknown_prop_rejected() {
        let m = manifest(CALLOUT, "callout");
        let err = recognise(&attrs(&[("nope", "x")]), &m).unwrap_err();
        assert_eq!(err.code, "content-prop-unknown");
    }

    #[test]
    fn bad_int_rejected() {
        let m = manifest(CALLOUT, "callout");
        let err = recognise(&attrs(&[("count", "1.2.3")]), &m).unwrap_err();
        assert_eq!(err.code, "content-prop-type");
    }

    #[test]
    fn enum_violation_rejected() {
        let m = manifest(CALLOUT, "callout");
        let err = recognise(&attrs(&[("tone", "danger")]), &m).unwrap_err();
        assert_eq!(err.code, "content-prop-enum");
    }

    #[test]
    fn missing_required_content_prop_rejected() {
        let src = r#"
            name = "x"
            content_invocable = true
            content_props = ["label"]
            [props]
            label = { type = "string", required = true }
        "#;
        let m = manifest(src, "x");
        let err = recognise(&[], &m).unwrap_err();
        assert_eq!(err.code, "content-prop-missing");
    }

    #[test]
    fn flag_and_class_shorthand() {
        let src = r#"
            name = "box"
            content_invocable = true
            content_props = ["open", "class"]
            [props]
            open = { type = "bool", default = false }
            class = { type = "string", default = "" }
        "#;
        let m = manifest(src, "box");
        let r = recognise(
            &[Attr::Flag("open".into()), Attr::Class("a".into()), Attr::Class("b".into())],
            &m,
        )
        .unwrap();
        assert!(r.props["open"].is_true());
        assert_eq!(r.props["class"].as_str(), Some("a b"));
    }
}
