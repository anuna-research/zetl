//! SPEC-048 REQ-4814 — Prop validation against the manifest.
//!
//! Runs in the component-resolution layer (not minijinja). Two surfaces:
//! [`check_known_props`] validates the *names* present at an invocation site
//! statically (the kwarg identifiers are literal in template source); [`validate_props`]
//! does full value-level type/enum/required checking when concrete values are available.

use super::manifest::{Manifest, PropType};
use super::{CResult, ComponentError};
use minijinja::value::{Value, ValueKind};
use std::collections::BTreeMap;

/// Static check usable at lowering time: every supplied prop name must be declared,
/// and every required prop with no default must be present. `supplied` is the set of
/// kwarg identifiers literally present at the invocation site.
pub fn check_known_props(manifest: &Manifest, supplied: &[String]) -> CResult<()> {
    for name in supplied {
        if !manifest.props.contains_key(name) {
            return Err(ComponentError::new(
                "component-unknown-prop",
                format!("unknown prop `{name}` for component `{}`", manifest.name),
            ));
        }
    }
    for (name, def) in &manifest.props {
        if def.required && def.default.is_none() && !supplied.iter().any(|s| s == name) {
            return Err(ComponentError::new(
                "component-prop-missing",
                format!(
                    "required prop `{name}` missing for component `{}`",
                    manifest.name
                ),
            ));
        }
    }
    Ok(())
}

/// Full value-level validation. Returns the bound prop map (supplied values plus
/// manifest defaults for omitted optionals).
pub fn validate_props(
    manifest: &Manifest,
    supplied: &BTreeMap<String, Value>,
) -> CResult<BTreeMap<String, Value>> {
    for key in supplied.keys() {
        if !manifest.props.contains_key(key) {
            return Err(ComponentError::new(
                "component-unknown-prop",
                format!("unknown prop `{key}` for component `{}`", manifest.name),
            ));
        }
    }

    let mut bound = BTreeMap::new();
    for (name, def) in &manifest.props {
        match supplied.get(name) {
            Some(v) => {
                check_type(manifest, name, def.ty, v)?;
                check_enum(manifest, name, def, v)?;
                bound.insert(name.clone(), v.clone());
            }
            None => {
                if let Some(default) = &def.default {
                    bound.insert(name.clone(), toml_to_value(default));
                } else if def.required {
                    return Err(ComponentError::new(
                        "component-prop-missing",
                        format!(
                            "required prop `{name}` missing for component `{}`",
                            manifest.name
                        ),
                    ));
                }
            }
        }
    }
    Ok(bound)
}

fn check_type(manifest: &Manifest, name: &str, ty: PropType, v: &Value) -> CResult<()> {
    let ok = match ty {
        PropType::String => v.kind() == ValueKind::String,
        PropType::Bool => v.kind() == ValueKind::Bool,
        // minijinja represents all numbers under ValueKind::Number; `int` additionally
        // requires an integral value.
        PropType::Number => v.kind() == ValueKind::Number,
        PropType::Int => v.kind() == ValueKind::Number && i64::try_from(v.clone()).is_ok(),
        PropType::List => v.kind() == ValueKind::Seq,
        PropType::Map => v.kind() == ValueKind::Map,
        // SPEC-049 CON-4903: `url` is a string subtype. The trusted (SPEC-048) path
        // only checks the value is a string; scheme validation happens at content
        // ingestion (REQ-4904), where the value crosses the trust boundary.
        PropType::Url => v.kind() == ValueKind::String,
    };
    if ok {
        Ok(())
    } else {
        Err(ComponentError::new(
            "component-prop-type",
            format!(
                "prop `{name}` for component `{}` expected {} but got {:?}",
                manifest.name,
                ty.as_str(),
                v.kind()
            ),
        ))
    }
}

fn check_enum(
    manifest: &Manifest,
    name: &str,
    def: &super::manifest::PropDef,
    v: &Value,
) -> CResult<()> {
    let Some(allowed) = &def.enum_values else {
        return Ok(());
    };
    let got = value_cmp_key(v);
    if allowed.iter().any(|a| toml_cmp_key(a) == got) {
        Ok(())
    } else {
        Err(ComponentError::new(
            "component-prop-enum",
            format!(
                "prop `{name}` value `{got}` not in declared enum for component `{}`",
                manifest.name
            ),
        ))
    }
}

/// Canonical comparison key for a minijinja value (string/number/bool).
fn value_cmp_key(v: &Value) -> String {
    if let Some(s) = v.as_str() {
        s.to_string()
    } else {
        v.to_string()
    }
}

fn toml_cmp_key(t: &toml::Value) -> String {
    match t {
        toml::Value::String(s) => s.clone(),
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Float(f) => f.to_string(),
        toml::Value::Boolean(b) => b.to_string(),
        other => other.to_string(),
    }
}

/// Convert a TOML literal (a manifest default) to a minijinja value.
pub fn toml_to_value(t: &toml::Value) -> Value {
    match t {
        toml::Value::String(s) => Value::from(s.clone()),
        toml::Value::Integer(i) => Value::from(*i),
        toml::Value::Float(f) => Value::from(*f),
        toml::Value::Boolean(b) => Value::from(*b),
        toml::Value::Datetime(d) => Value::from(d.to_string()),
        toml::Value::Array(a) => Value::from(a.iter().map(toml_to_value).collect::<Vec<_>>()),
        toml::Value::Table(tbl) => {
            let map: BTreeMap<String, Value> = tbl
                .iter()
                .map(|(k, v)| (k.clone(), toml_to_value(v)))
                .collect();
            Value::from(map)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::components::manifest::parse_manifest;

    fn manifest() -> Manifest {
        parse_manifest(
            r#"
            name = "callout"
            [props]
            tone = { type = "string", required = true, enum = ["info", "warning"] }
            count = { type = "int", default = 1 }
            dismissible = { type = "bool", default = false }
            "#,
            "callout",
        )
        .unwrap()
    }

    #[test]
    fn valid_props_bind_and_defaults_fill() {
        let m = manifest();
        let mut supplied = BTreeMap::new();
        supplied.insert("tone".to_string(), Value::from("warning"));
        let bound = validate_props(&m, &supplied).unwrap();
        assert_eq!(bound["tone"].as_str(), Some("warning"));
        assert_eq!(i64::try_from(bound["count"].clone()).unwrap(), 1);
        assert!(!bound["dismissible"].is_true());
    }

    #[test]
    fn unknown_prop_rejected() {
        let m = manifest();
        let mut supplied = BTreeMap::new();
        supplied.insert("tone".to_string(), Value::from("info"));
        supplied.insert("bogus".to_string(), Value::from(1));
        assert_eq!(
            validate_props(&m, &supplied).unwrap_err().code,
            "component-unknown-prop"
        );
    }

    #[test]
    fn type_mismatch_rejected() {
        let m = manifest();
        let mut supplied = BTreeMap::new();
        supplied.insert("tone".to_string(), Value::from(42));
        assert_eq!(
            validate_props(&m, &supplied).unwrap_err().code,
            "component-prop-type"
        );
    }

    #[test]
    fn enum_miss_rejected() {
        let m = manifest();
        let mut supplied = BTreeMap::new();
        supplied.insert("tone".to_string(), Value::from("danger"));
        assert_eq!(
            validate_props(&m, &supplied).unwrap_err().code,
            "component-prop-enum"
        );
    }

    #[test]
    fn missing_required_rejected() {
        let m = manifest();
        let supplied = BTreeMap::new();
        assert_eq!(
            validate_props(&m, &supplied).unwrap_err().code,
            "component-prop-missing"
        );
    }

    #[test]
    fn static_name_check() {
        let m = manifest();
        assert!(check_known_props(&m, &["tone".to_string()]).is_ok());
        assert_eq!(
            check_known_props(&m, &["nope".to_string()])
                .unwrap_err()
                .code,
            "component-unknown-prop"
        );
        assert_eq!(
            check_known_props(&m, &[]).unwrap_err().code,
            "component-prop-missing"
        );
    }
}
