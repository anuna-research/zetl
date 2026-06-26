//! SPEC-050 CON-5005 — topic value-type language + build-time recogniser.
//!
//! A deliberately small type language (LangSec principle 6 — minimise grammatical
//! power):
//! ```text
//! type-expr   = "string" | "bool" | "int" | "number"
//!             | "enum(" literal { "," literal } ")"
//!             | "{" field { "," field } "}"
//! field       = ident ":" scalar-type
//! scalar-type = "string" | "bool" | "int" | "number" | "enum(...)"
//! literal     = double-quoted json string (case-sensitive)
//! ```
//! Records are flat (no nesting in v1). This module parses the expression and recognises
//! a TOML `default` value against it (the runtime value recogniser is the JS bus side,
//! sharing these semantics). Errors: `island-topic-type-invalid`, `island-payload-type`.

use super::IslandError;

/// A scalar (record-field-eligible) type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScalarType {
    String,
    Bool,
    Int,
    Number,
    Enum(Vec<String>),
}

/// A topic value type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueType {
    String,
    Bool,
    Int,
    Number,
    Enum(Vec<String>),
    /// A flat record: ordered, unique field names each with a scalar type.
    Record(Vec<(String, ScalarType)>),
}

fn invalid(msg: impl Into<String>) -> IslandError {
    IslandError::new("island-topic-type-invalid", msg.into())
}

/// Parse a `type-expr` string (CON-5005).
pub fn parse_type_expr(src: &str) -> Result<ValueType, IslandError> {
    let s = src.trim();
    match s {
        "string" => return Ok(ValueType::String),
        "bool" => return Ok(ValueType::Bool),
        "int" => return Ok(ValueType::Int),
        "number" => return Ok(ValueType::Number),
        _ => {}
    }
    if let Some(inner) = s.strip_prefix("enum(").and_then(|r| r.strip_suffix(')')) {
        let lits = parse_enum_literals(inner)?;
        return Ok(ValueType::Enum(lits));
    }
    if let Some(inner) = s.strip_prefix('{').and_then(|r| r.strip_suffix('}')) {
        return parse_record(inner);
    }
    Err(invalid(format!("unparseable type-expr `{src}`")))
}

fn parse_scalar(src: &str) -> Result<ScalarType, IslandError> {
    let s = src.trim();
    match s {
        "string" => Ok(ScalarType::String),
        "bool" => Ok(ScalarType::Bool),
        "int" => Ok(ScalarType::Int),
        "number" => Ok(ScalarType::Number),
        _ => {
            if let Some(inner) = s.strip_prefix("enum(").and_then(|r| r.strip_suffix(')')) {
                Ok(ScalarType::Enum(parse_enum_literals(inner)?))
            } else {
                Err(invalid(format!("invalid scalar type `{src}`")))
            }
        }
    }
}

/// Parse comma-separated double-quoted literals (the enum members).
fn parse_enum_literals(inner: &str) -> Result<Vec<String>, IslandError> {
    let mut out = Vec::new();
    for part in split_top_level(inner, ',') {
        let p = part.trim();
        let lit = p
            .strip_prefix('"')
            .and_then(|r| r.strip_suffix('"'))
            .ok_or_else(|| invalid(format!("enum member `{p}` must be a double-quoted literal")))?;
        if lit.chars().any(|c| c.is_control()) {
            return Err(invalid("enum literal contains a control char"));
        }
        out.push(lit.to_string());
    }
    if out.is_empty() {
        return Err(invalid("empty enum"));
    }
    // unique
    let mut seen = std::collections::BTreeSet::new();
    for l in &out {
        if !seen.insert(l) {
            return Err(invalid(format!("duplicate enum literal `{l}`")));
        }
    }
    Ok(out)
}

fn parse_record(inner: &str) -> Result<ValueType, IslandError> {
    let mut fields = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for part in split_top_level(inner, ',') {
        let p = part.trim();
        if p.is_empty() {
            return Err(invalid("empty record field"));
        }
        let (name, ty) = p
            .split_once(':')
            .ok_or_else(|| invalid(format!("record field `{p}` must be `ident:type`")))?;
        let name = name.trim().to_string();
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            return Err(invalid(format!("invalid record field ident `{name}`")));
        }
        if !seen.insert(name.clone()) {
            return Err(invalid(format!("duplicate record field `{name}`")));
        }
        fields.push((name, parse_scalar(ty)?));
    }
    if fields.is_empty() {
        return Err(invalid("empty record"));
    }
    Ok(ValueType::Record(fields))
}

/// Split `s` on `delim` at the top level (not inside nested `(...)`/`{...}`).
fn split_top_level(s: &str, delim: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for c in s.chars() {
        match c {
            '(' | '{' => {
                depth += 1;
                cur.push(c);
            }
            ')' | '}' => {
                depth -= 1;
                cur.push(c);
            }
            c if c == delim && depth == 0 => {
                parts.push(std::mem::take(&mut cur));
            }
            c => cur.push(c),
        }
    }
    if !cur.trim().is_empty() || !parts.is_empty() {
        parts.push(cur);
    }
    parts
}

/// Recognise a TOML `default` value against a value type (build-time check, CON-5005).
/// The runtime recogniser (JS bus) shares these semantics.
pub fn recognise_default(value: &toml::Value, ty: &ValueType) -> Result<(), IslandError> {
    let bad = || {
        IslandError::new(
            "island-payload-type",
            format!("default does not conform to {ty:?}"),
        )
    };
    match ty {
        ValueType::String => value.as_str().map(|_| ()).ok_or_else(bad),
        ValueType::Bool => value.as_bool().map(|_| ()).ok_or_else(bad),
        ValueType::Int => {
            // a TOML integer (reject floats)
            value.as_integer().map(|_| ()).ok_or_else(bad)
        }
        ValueType::Number => {
            if value.as_integer().is_some() || value.as_float().is_some() {
                Ok(())
            } else {
                Err(bad())
            }
        }
        ValueType::Enum(members) => match value.as_str() {
            Some(s) if members.iter().any(|m| m == s) => Ok(()),
            _ => Err(bad()),
        },
        ValueType::Record(fields) => {
            let table = value.as_table().ok_or_else(bad)?;
            if table.len() != fields.len() {
                return Err(bad());
            }
            for (name, scalar) in fields {
                let fv = table.get(name).ok_or_else(bad)?;
                recognise_default(fv, &scalar_to_value_type(scalar))?;
            }
            Ok(())
        }
    }
}

fn scalar_to_value_type(s: &ScalarType) -> ValueType {
    match s {
        ScalarType::String => ValueType::String,
        ScalarType::Bool => ValueType::Bool,
        ScalarType::Int => ValueType::Int,
        ScalarType::Number => ValueType::Number,
        ScalarType::Enum(m) => ValueType::Enum(m.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scalars() {
        assert_eq!(parse_type_expr("string").unwrap(), ValueType::String);
        assert_eq!(parse_type_expr("bool").unwrap(), ValueType::Bool);
        assert_eq!(parse_type_expr("int").unwrap(), ValueType::Int);
        assert_eq!(parse_type_expr("number").unwrap(), ValueType::Number);
    }

    #[test]
    fn parses_enum() {
        let t = parse_type_expr(r#"enum("light","dark")"#).unwrap();
        assert_eq!(t, ValueType::Enum(vec!["light".into(), "dark".into()]));
    }

    #[test]
    fn parses_record() {
        let t = parse_type_expr("{x:int,label:string}").unwrap();
        assert_eq!(
            t,
            ValueType::Record(vec![
                ("x".into(), ScalarType::Int),
                ("label".into(), ScalarType::String),
            ])
        );
    }

    #[test]
    fn rejects_invalid() {
        for s in [
            "",
            "widget",
            "enum()",
            "{}",
            "{x}",
            "{x:int,x:bool}",
            r#"enum("a","a")"#,
        ] {
            assert!(parse_type_expr(s).is_err(), "{s} should be invalid");
        }
    }

    #[test]
    fn recognises_default_conformance() {
        let t = parse_type_expr(r#"enum("light","dark")"#).unwrap();
        assert!(recognise_default(&toml::Value::String("dark".into()), &t).is_ok());
        assert!(recognise_default(&toml::Value::String("blue".into()), &t).is_err());

        let rec = parse_type_expr("{n:int}").unwrap();
        let mut tbl = toml::map::Map::new();
        tbl.insert("n".into(), toml::Value::Integer(3));
        assert!(recognise_default(&toml::Value::Table(tbl.clone()), &rec).is_ok());
        // extra key → reject (strict)
        tbl.insert("extra".into(), toml::Value::Integer(1));
        assert!(recognise_default(&toml::Value::Table(tbl), &rec).is_err());
    }
}
