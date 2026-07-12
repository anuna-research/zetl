//! SPEC-048 REQ-4812 / CON-4804 — Design tokens (`tokens.toml` → `tokens.css`).
//!
//! One `_static/tokens.css` with a single `:root` (+ optional `[data-theme]` blocks),
//! each token a `--ident: value;` declaration. Layered resolution **merges** key by key
//! (a vault token overrides a theme token individually — not wholesale file
//! replacement). Every value is recognised as CSS-token-safe before emission (Threat F):
//! it cannot terminate the declaration or open a new rule/comment/markup context. An
//! out-of-grammar value is `tokens-value-unsafe` (no emission of the offending decl).

use super::{CResult, ComponentError};
use std::collections::BTreeMap;

/// A parsed token layer: root tokens plus per-theme override blocks.
#[derive(Debug, Clone, Default)]
pub struct TokenLayer {
    pub root: BTreeMap<String, String>,
    /// `theme name` (e.g. `light`, `dark`) → tokens.
    pub themes: BTreeMap<String, BTreeMap<String, String>>,
}

/// `ident` grammar: `[a-z][a-z0-9-]*` → `--ident` custom property.
fn is_valid_token_ident(ident: &str) -> bool {
    let mut chars = ident.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// CON-4804 value recogniser: reject anything that could break out of the CSS
/// declaration context — `;`, `}`, `{`, comment delimiters, markup `<`/`>`, backslash
/// escapes, or a `url(` with a non-`data:`/non-relative scheme.
pub fn is_css_token_safe(value: &str) -> bool {
    if value.contains(';')
        || value.contains('}')
        || value.contains('{')
        || value.contains('<')
        || value.contains('>')
        || value.contains('\\')
        || value.contains("/*")
        || value.contains("*/")
    {
        return false;
    }
    // url(...) is allowed only for data: or relative references, never an arbitrary
    // scheme (which could exfiltrate or load remote content).
    let lower = value.to_ascii_lowercase();
    if let Some(idx) = lower.find("url(") {
        let inner = &lower[idx + 4..];
        let inner = inner.trim_start().trim_start_matches(['"', '\'']);
        let scheme_ok = inner.starts_with("data:")
            || inner.starts_with('/')
            || inner.starts_with('.')
            || inner.starts_with('#')
            || !inner.contains(':'); // bare relative path
        if !scheme_ok {
            return false;
        }
    }
    true
}

fn parse_token_table(
    table: &toml::value::Table,
    out: &mut BTreeMap<String, String>,
) -> CResult<()> {
    for (key, val) in table {
        let value = match val {
            toml::Value::String(s) => s.clone(),
            toml::Value::Integer(i) => i.to_string(),
            toml::Value::Float(f) => f.to_string(),
            toml::Value::Boolean(b) => b.to_string(),
            _ => {
                return Err(ComponentError::new(
                    "tokens-value-unsafe",
                    format!("token `{key}` must be a scalar value"),
                ));
            }
        };
        if !is_valid_token_ident(key) {
            return Err(ComponentError::new(
                "tokens-value-unsafe",
                format!("token name `{key}` is not a valid CSS custom-property ident"),
            ));
        }
        if !is_css_token_safe(&value) {
            return Err(ComponentError::new(
                "tokens-value-unsafe",
                format!("token `{key}` value `{value}` is not CSS-token-safe"),
            ));
        }
        out.insert(key.clone(), value);
    }
    Ok(())
}

/// Parse a single `tokens.toml` layer. Top-level scalar keys are root tokens;
/// `[theme.<name>]` tables become per-theme override blocks.
pub fn parse_tokens(src: &str) -> CResult<TokenLayer> {
    let parsed: toml::Value = toml::from_str(src).map_err(|e| {
        ComponentError::new(
            "tokens-value-unsafe",
            format!("tokens.toml parse error: {}", e.message()),
        )
    })?;
    let toml::Value::Table(table) = parsed else {
        return Err(ComponentError::new(
            "tokens-value-unsafe",
            "tokens.toml must be a table",
        ));
    };

    let mut layer = TokenLayer::default();
    for (key, val) in &table {
        if key == "theme" {
            // [theme.<name>] sub-tables
            if let toml::Value::Table(theme_tables) = val {
                for (theme_name, theme_val) in theme_tables {
                    if let toml::Value::Table(tt) = theme_val {
                        let mut block = BTreeMap::new();
                        parse_token_table(tt, &mut block)?;
                        layer.themes.insert(theme_name.clone(), block);
                    }
                }
            }
        } else if let toml::Value::Table(_) = val {
            // ignore other tables for forward-compat (only [theme.*] is defined)
        } else {
            // a root scalar token
            let mut single = toml::value::Table::new();
            single.insert(key.clone(), val.clone());
            parse_token_table(&single, &mut layer.root)?;
        }
    }
    Ok(layer)
}

/// Merge `over` onto `base` key-by-key (REQ-4812 merge-not-replace). `over` (e.g. the
/// vault layer) wins per individual token.
pub fn merge_layers(base: &TokenLayer, over: &TokenLayer) -> TokenLayer {
    let mut merged = base.clone();
    for (k, v) in &over.root {
        merged.root.insert(k.clone(), v.clone());
    }
    for (theme, block) in &over.themes {
        let entry = merged.themes.entry(theme.clone()).or_default();
        for (k, v) in block {
            entry.insert(k.clone(), v.clone());
        }
    }
    merged
}

/// Render the merged layer to a single deterministic `tokens.css` string.
pub fn emit_css(layer: &TokenLayer) -> String {
    let mut out = String::new();
    out.push_str(":root {\n");
    for (k, v) in &layer.root {
        out.push_str(&format!("  --{k}: {v};\n"));
    }
    out.push_str("}\n");
    for (theme, block) in &layer.themes {
        out.push_str(&format!("[data-theme=\"{theme}\"] {{\n"));
        for (k, v) in block {
            out.push_str(&format!("  --{k}: {v};\n"));
        }
        out.push_str("}\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_root_and_theme_blocks() {
        let src = r##"
            moss = "#5B8C5A"
            radius = "8px"
            [theme.dark]
            moss = "#80b47f"
        "##;
        let layer = parse_tokens(src).unwrap();
        assert_eq!(layer.root["moss"], "#5B8C5A");
        assert_eq!(layer.root["radius"], "8px");
        assert_eq!(layer.themes["dark"]["moss"], "#80b47f");
    }

    #[test]
    fn merge_overrides_single_key() {
        let theme = parse_tokens("moss = \"#5B8C5A\"\nwarm = \"#d98c4a\"").unwrap();
        let vault = parse_tokens("moss = \"#4da6a6\"").unwrap();
        let merged = merge_layers(&theme, &vault);
        // one key changed, the other inherited
        assert_eq!(merged.root["moss"], "#4da6a6");
        assert_eq!(merged.root["warm"], "#d98c4a");
    }

    #[test]
    fn rejects_declaration_breakout() {
        let err = parse_tokens(r#"x = "red; } body { display:none""#).unwrap_err();
        assert_eq!(err.code, "tokens-value-unsafe");
    }

    #[test]
    fn rejects_comment_and_markup() {
        assert_eq!(
            parse_tokens(r#"x = "a /* b""#).unwrap_err().code,
            "tokens-value-unsafe"
        );
        assert_eq!(
            parse_tokens(r#"x = "<script>""#).unwrap_err().code,
            "tokens-value-unsafe"
        );
    }

    #[test]
    fn rejects_remote_url_scheme() {
        assert!(!is_css_token_safe("url(https://evil.example/x.png)"));
        assert!(is_css_token_safe("url(data:image/png;base64,AAAA)") == false); // contains ;
        assert!(is_css_token_safe("url(./local.png)"));
        assert!(is_css_token_safe("url(#frag)"));
    }

    #[test]
    fn deterministic_emit() {
        let a = parse_tokens("b = \"2\"\na = \"1\"").unwrap();
        assert_eq!(emit_css(&a), emit_css(&a));
        let css = emit_css(&a);
        // sorted by ident
        assert!(css.find("--a:").unwrap() < css.find("--b:").unwrap());
    }
}
