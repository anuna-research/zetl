//! Per-file hook opt-out via page frontmatter (SPEC-032 REQ-3211 / CON-3211).
//!
//! A page may disable a single extension by setting `ztl.ext.<id>: false`
//! in its frontmatter. The pipeline consults this flag before invoking each
//! hook; a disabled hook is skipped silently — no invocation event, no
//! failure record, no duration accounted.
//!
//! ```yaml
//! ---
//! ztl:
//!   ext:
//!     callouts: false   # bypass the callouts hook for this page only
//! ---
//! ```
//!
//! Only the literal JSON value `false` opts the page out. Any other value
//! (missing, `null`, `true`, a nested object, a non-bool) leaves the hook
//! enabled — matching the spec's "absent means enabled" stance and keeping
//! forward-compatible room for per-page configuration payloads on the same
//! key (extension-specific semantics, opaque to the platform).
//!
//! This surface is independent of the author-controlled `frontmatter_where`
//! selector (REQ-3205): selectors are extension-specific and may omit the
//! opt-out by mistake; this check is the platform-guaranteed escape hatch
//! that fires for every hook regardless of what its selector says.

use serde_json::Value;

use crate::hooks::ast::Frontmatter;

/// Return `true` when `frontmatter.ztl.ext.<extension_id>` is the literal
/// JSON `false`. Any other shape returns `false`.
///
/// The look-up is case-sensitive on `extension_id` — the id resolved by
/// composition is the authoritative spelling.
pub fn is_disabled_by_frontmatter(frontmatter: &Frontmatter, extension_id: &str) -> bool {
    let ztl = match frontmatter.get("ztl") {
        Some(Value::Object(m)) => m,
        _ => return false,
    };
    let ext = match ztl.get("ext") {
        Some(Value::Object(m)) => m,
        _ => return false,
    };
    matches!(ext.get(extension_id), Some(Value::Bool(false)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fm(v: Value) -> Frontmatter {
        match v {
            Value::Object(m) => m,
            _ => panic!("fixture frontmatter must be an object"),
        }
    }

    #[test]
    fn literal_false_opts_out() {
        let f = fm(json!({ "ztl": { "ext": { "callouts": false } } }));
        assert!(is_disabled_by_frontmatter(&f, "callouts"));
    }

    #[test]
    fn literal_true_leaves_hook_enabled() {
        let f = fm(json!({ "ztl": { "ext": { "callouts": true } } }));
        assert!(!is_disabled_by_frontmatter(&f, "callouts"));
    }

    #[test]
    fn missing_key_leaves_hook_enabled() {
        let f = fm(json!({ "ztl": { "ext": { "tasks": false } } }));
        assert!(!is_disabled_by_frontmatter(&f, "callouts"));
    }

    #[test]
    fn empty_frontmatter_leaves_hook_enabled() {
        let f = fm(json!({}));
        assert!(!is_disabled_by_frontmatter(&f, "callouts"));
    }

    #[test]
    fn missing_ztl_branch_leaves_hook_enabled() {
        let f = fm(json!({ "title": "hi" }));
        assert!(!is_disabled_by_frontmatter(&f, "callouts"));
    }

    #[test]
    fn missing_ext_branch_leaves_hook_enabled() {
        let f = fm(json!({ "ztl": { "other": 1 } }));
        assert!(!is_disabled_by_frontmatter(&f, "callouts"));
    }

    #[test]
    fn non_object_ztl_leaves_hook_enabled() {
        // A page author who sets `ztl: "whatever"` accidentally — the
        // platform cannot infer an opt-out from that; don't disable.
        let f = fm(json!({ "ztl": "oops" }));
        assert!(!is_disabled_by_frontmatter(&f, "callouts"));
    }

    #[test]
    fn non_object_ext_leaves_hook_enabled() {
        let f = fm(json!({ "ztl": { "ext": 42 } }));
        assert!(!is_disabled_by_frontmatter(&f, "callouts"));
    }

    #[test]
    fn null_value_leaves_hook_enabled() {
        // `null` is not the same signal as `false`.
        let f = fm(json!({ "ztl": { "ext": { "callouts": null } } }));
        assert!(!is_disabled_by_frontmatter(&f, "callouts"));
    }

    #[test]
    fn object_value_leaves_hook_enabled() {
        // Extension-specific configuration under the same key must not be
        // read as a disable signal. The platform is opaque to the object
        // body and defers semantics to the extension.
        let f = fm(json!({ "ztl": { "ext": { "tasks": { "filter": "done" } } } }));
        assert!(!is_disabled_by_frontmatter(&f, "tasks"));
    }

    #[test]
    fn string_false_leaves_hook_enabled() {
        // `"false"` as a string is not the JSON `false` literal — don't
        // disable on it; require the strict type.
        let f = fm(json!({ "ztl": { "ext": { "callouts": "false" } } }));
        assert!(!is_disabled_by_frontmatter(&f, "callouts"));
    }

    #[test]
    fn extension_ids_are_case_sensitive() {
        let f = fm(json!({ "ztl": { "ext": { "callouts": false } } }));
        assert!(is_disabled_by_frontmatter(&f, "callouts"));
        assert!(!is_disabled_by_frontmatter(&f, "Callouts"));
        assert!(!is_disabled_by_frontmatter(&f, "CALLOUTS"));
    }

    #[test]
    fn each_extension_is_independent() {
        let f = fm(json!({
            "ztl": { "ext": { "callouts": false, "tasks": true } }
        }));
        assert!(is_disabled_by_frontmatter(&f, "callouts"));
        assert!(!is_disabled_by_frontmatter(&f, "tasks"));
        assert!(!is_disabled_by_frontmatter(&f, "admonition"));
    }
}
