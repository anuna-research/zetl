//! SPEC-048 REQ-4805 / CON-4802 / ADR-4803 — `{% component %}` source lowering.
//!
//! minijinja exposes no custom-statement-tag API, so the component surface is a thin
//! pre-parse rewrite into native minijinja: a component is a macro (registered in a
//! synthetic `__zetl_components.html`, imported as `__zc`); the default slot lowers to
//! `caller()`; named `{% slot "s" %}…{% endslot %}` blocks lower to `{% set _slot_s %}`
//! captures passed as macro args. No custom engine tag is introduced.
//!
//! Forms recognised:
//! - block: `{% component "name" k=v %}…body…{% endcomponent %}`
//! - self-closing: `{% component "name" k=v /%}` (IMPL-048 pins `/%}` as the
//!   unambiguous self-closing spelling of CON-4802)
//! - named slot: `{% slot "name" %}…{% endslot %}` inside a block body
//!
//! kwarg values in v1 are a quoted string, number, bool, or a bare dotted path
//! (no embedded spaces); richer expressions are deferred to IMPL-048.

use super::manifest::Manifest;
use super::validate::check_known_props;
use super::{CResult, ComponentError};
use regex::Regex;
use std::collections::BTreeMap;
use std::sync::LazyLock;

/// Name of the synthetic template that holds every component macro.
pub const COMPONENTS_TEMPLATE: &str = "__zetl_components.html";
const IMPORT_LINE: &str = "{% import \"__zetl_components.html\" as __zc %}\n";

/// Lookup of component manifests by name, for validation during lowering.
pub trait ComponentLookup {
    fn manifest(&self, name: &str) -> Option<&Manifest>;
}

/// A trivial map-backed lookup (used in tests and simple call sites).
pub struct MapLookup(pub BTreeMap<String, Manifest>);

impl ComponentLookup for MapLookup {
    fn manifest(&self, name: &str) -> Option<&Manifest> {
        self.0.get(name)
    }
}

static KWARG_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"([a-zA-Z_][a-zA-Z0-9_-]*)\s*=\s*("(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'|[^\s%/]+)"#)
        .expect("kwarg regex")
});

static SLOT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)\{%\s*slot\s+"([^"]+)"\s*%\}(.*?)\{%\s*endslot\s*%\}"#).expect("slot regex")
});

static NAME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^"([^"]+)"|^'([^']+)'"#).expect("name re"));

/// Lower a template source. If it invokes no component, it is returned byte-for-byte
/// unchanged (REQ-4813 backward compatibility, idempotence).
pub fn lower_template(src: &str, lookup: &dyn ComponentLookup) -> CResult<String> {
    if !src.contains("{% component") {
        return Ok(src.to_string());
    }
    let body = transform(src, lookup)?;
    Ok(format!("{IMPORT_LINE}{body}"))
}

/// The set of component names invoked at the top level of `src` (for the static
/// invocation graph, REQ-4807). Self-closing and block forms both count.
pub fn invoked_components(src: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = src;
    while let Some(idx) = rest.find("{% component") {
        let after = &rest[idx..];
        if let Some((name, _kwargs, _self_closing, tag_end)) = parse_open_tag(after) {
            names.push(name);
            rest = &after[tag_end..];
        } else {
            rest = &after[12..];
        }
    }
    names
}

fn transform(src: &str, lookup: &dyn ComponentLookup) -> CResult<String> {
    let mut out = String::new();
    let mut rest = src;
    while let Some(idx) = rest.find("{% component") {
        out.push_str(&rest[..idx]);
        let after = &rest[idx..];
        let (name, kwargs, self_closing, tag_end) = parse_open_tag(after).ok_or_else(|| {
            ComponentError::new("component-malformed", "malformed {% component %} tag")
        })?;

        // Resolve + validate prop names against the manifest (REQ-4804/4814).
        let manifest = lookup.manifest(&name).ok_or_else(|| {
            ComponentError::new(
                "component-not-found",
                format!("component `{name}` is not resolvable"),
            )
        })?;
        let supplied: Vec<String> = kwargs.iter().map(|(k, _)| k.clone()).collect();
        check_known_props(manifest, &supplied)?;

        if self_closing {
            out.push_str(&render_self_close(&name, &kwargs));
            rest = &after[tag_end..];
        } else {
            let (raw_body, after_block) = match_block(&after[tag_end..])?;
            let lowered_body = transform(raw_body, lookup)?;
            let (default_body, slots) = extract_named_slots(&lowered_body);
            out.push_str(&render_call(&name, &kwargs, &default_body, &slots));
            rest = after_block;
        }
    }
    out.push_str(rest);
    Ok(out)
}

/// A recognised opening tag: `(name, kwargs, self_closing, bytes_consumed_through_%}})`.
type OpenTag = (String, Vec<(String, String)>, bool, usize);

/// Parse an opening `{% component "name" k=v ... %}` (or `... /%}`).
fn parse_open_tag(s: &str) -> Option<OpenTag> {
    debug_assert!(s.starts_with("{% component"));
    let close = s.find("%}")?;
    let inner = &s[..close]; // "{% component \"name\" ... [/]"
    let tag_end = close + 2;

    // strip leading "{% component"
    let body = inner["{% component".len()..].trim();
    let self_closing = body.trim_end().ends_with('/');
    let body = body.trim_end().trim_end_matches('/').trim_end();

    // name is the first quoted string
    let caps = NAME_RE.captures(body.trim_start())?;
    let name = caps
        .get(1)
        .or_else(|| caps.get(2))
        .map(|m| m.as_str().to_string())?;
    let rest = body.trim_start()[caps.get(0)?.end()..].trim();

    let kwargs = parse_kwargs(rest);
    Some((name, kwargs, self_closing, tag_end))
}

fn parse_kwargs(s: &str) -> Vec<(String, String)> {
    KWARG_RE
        .captures_iter(s)
        .map(|c| (c[1].to_string(), c[2].to_string()))
        .collect()
}

/// Scan a body for its matching `{% endcomponent %}`, respecting nested non-self-closing
/// `{% component %}` blocks. Returns `(body, rest_after_endcomponent)`.
fn match_block(s: &str) -> CResult<(&str, &str)> {
    let mut depth = 1usize;
    let mut i = 0usize;
    while i < s.len() {
        if s[i..].starts_with("{% endcomponent %}") || s[i..].starts_with("{% endcomponent%}") {
            depth -= 1;
            if depth == 0 {
                let end_tag_len = if s[i..].starts_with("{% endcomponent %}") {
                    "{% endcomponent %}".len()
                } else {
                    "{% endcomponent%}".len()
                };
                return Ok((&s[..i], &s[i + end_tag_len..]));
            }
            i += "{% endcomponent".len();
        } else if s[i..].starts_with("{% component") {
            // nested open — only deepens if it is a block (not self-closing)
            if let Some((_, _, self_closing, tag_end)) = parse_open_tag(&s[i..]) {
                if !self_closing {
                    depth += 1;
                }
                i += tag_end;
            } else {
                i += "{% component".len();
            }
        } else {
            // advance one char boundary
            i += s[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        }
    }
    Err(ComponentError::new(
        "component-malformed",
        "unterminated {% component %} block (missing {% endcomponent %})",
    ))
}

/// Pull `{% slot "name" %}…{% endslot %}` segments out of a body; returns the remaining
/// default-slot body and the named-slot map.
fn extract_named_slots(body: &str) -> (String, BTreeMap<String, String>) {
    let mut slots = BTreeMap::new();
    for caps in SLOT_RE.captures_iter(body) {
        slots.insert(caps[1].to_string(), caps[2].to_string());
    }
    let default_body = SLOT_RE.replace_all(body, "").to_string();
    (default_body, slots)
}

fn props_literal(kwargs: &[(String, String)]) -> String {
    if kwargs.is_empty() {
        return "props={}".to_string();
    }
    let entries: Vec<String> = kwargs
        .iter()
        .map(|(k, v)| format!("\"{k}\": {v}"))
        .collect();
    format!("props={{{}}}", entries.join(", "))
}

/// Map a kebab component name to a minijinja-legal macro identifier. Component names
/// are `[a-z][a-z0-9-]*` (no underscores, REQ-4803), so `-` → `_` is collision-free.
pub fn macro_ident(name: &str) -> String {
    name.replace('-', "_")
}

fn render_self_close(name: &str, kwargs: &[(String, String)]) -> String {
    // Emit an empty {% call %} block (rather than a bare expression) so the component
    // macro's `caller()` is always defined and renders empty.
    format!(
        "{{% call __zc.{}({}) %}}{{% endcall %}}",
        macro_ident(name),
        props_literal(kwargs)
    )
}

fn render_call(
    name: &str,
    kwargs: &[(String, String)],
    default_body: &str,
    slots: &BTreeMap<String, String>,
) -> String {
    let mut out = String::new();
    // Capture each named slot into a variable.
    for slot_name in slots.keys() {
        out.push_str(&format!("{{% set __slot_{slot_name} %}}"));
        out.push_str(&slots[slot_name]);
        out.push_str("{% endset %}");
    }
    let mut args = vec![props_literal(kwargs)];
    for slot_name in slots.keys() {
        args.push(format!("{slot_name}=__slot_{slot_name}"));
    }
    out.push_str(&format!(
        "{{% call __zc.{}({}) %}}",
        macro_ident(name),
        args.join(", ")
    ));
    out.push_str(default_body.trim());
    out.push_str("{% endcall %}");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::components::manifest::parse_manifest;

    fn lookup() -> MapLookup {
        let mut m = BTreeMap::new();
        m.insert(
            "callout".to_string(),
            parse_manifest(
                "name = \"callout\"\n[props]\ntone = { type = \"string\" }",
                "callout",
            )
            .unwrap(),
        );
        m.insert(
            "card".to_string(),
            parse_manifest(
                "name = \"card\"\nslots = [\"header\"]\n[props]\ntitle = { type = \"string\" }",
                "card",
            )
            .unwrap(),
        );
        MapLookup(m)
    }

    #[test]
    fn no_component_is_unchanged() {
        let src = "<p>{{ page.title }}</p>";
        assert_eq!(lower_template(src, &lookup()).unwrap(), src);
    }

    #[test]
    fn block_form_lowers_to_call() {
        let src = r#"{% component "callout" tone="warning" %}Heads up.{% endcomponent %}"#;
        let out = lower_template(src, &lookup()).unwrap();
        assert!(out.contains("{% import \"__zetl_components.html\" as __zc %}"));
        assert!(out.contains(r#"{% call __zc.callout(props={"tone": "warning"}) %}"#));
        assert!(out.contains("Heads up."));
        assert!(out.contains("{% endcall %}"));
    }

    #[test]
    fn self_closing_lowers_to_empty_call() {
        let src = r#"{% component "callout" tone="info" /%}"#;
        let out = lower_template(src, &lookup()).unwrap();
        assert!(out.contains(r#"{% call __zc.callout(props={"tone": "info"}) %}{% endcall %}"#));
    }

    #[test]
    fn named_slot_lowers_to_set_capture() {
        let src = r#"{% component "card" title="Hi" %}{% slot "header" %}<h2>H</h2>{% endslot %}Body{% endcomponent %}"#;
        let out = lower_template(src, &lookup()).unwrap();
        assert!(out.contains("{% set __slot_header %}<h2>H</h2>{% endset %}"));
        assert!(out.contains("header=__slot_header"));
        assert!(out.contains("Body"));
    }

    #[test]
    fn nested_components_lower() {
        let src = r#"{% component "card" title="Hi" %}{% component "callout" tone="info" %}inner{% endcomponent %}{% endcomponent %}"#;
        let out = lower_template(src, &lookup()).unwrap();
        assert!(out.contains("__zc.card"));
        assert!(out.contains("__zc.callout"));
        // import prepended exactly once
        assert_eq!(out.matches("{% import").count(), 1);
    }

    #[test]
    fn unknown_component_errors() {
        let src = r#"{% component "ghost" %}x{% endcomponent %}"#;
        assert_eq!(
            lower_template(src, &lookup()).unwrap_err().code,
            "component-not-found"
        );
    }

    #[test]
    fn unknown_prop_errors() {
        let src = r#"{% component "callout" bogus="x" %}y{% endcomponent %}"#;
        assert_eq!(
            lower_template(src, &lookup()).unwrap_err().code,
            "component-unknown-prop"
        );
    }

    #[test]
    fn invoked_components_lists_names() {
        let src = r#"{% component "card" %}{% component "callout" /%}{% endcomponent %}"#;
        let names = invoked_components(src);
        assert!(names.contains(&"card".to_string()));
        assert!(names.contains(&"callout".to_string()));
    }
}
