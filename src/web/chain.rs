use std::collections::HashMap;

/// Extract a chain target (prev/next page name) from frontmatter.
///
/// Reads `fm[field]` as a string, then normalises it:
/// 1. Strips `[[` / `]]` wikilink delimiters if present.
/// 2. Strips alias text after `|`  (e.g. `Target|Display` → `Target`).
/// 3. Strips heading reference after `#` (e.g. `Target#Heading` → `Target`).
/// 4. Trims surrounding whitespace.
///
/// Returns `None` if the field is absent, not a string, or empty after trimming.
///
/// REQ-015-004, CON-015-001.
pub fn extract_chain_target(fm: &serde_json::Value, field: &str) -> Option<String> {
    let raw = fm.get(field)?.as_str()?;

    // Strip wikilink delimiters [[ ... ]]
    let inner = raw
        .strip_prefix("[[")
        .and_then(|s| s.strip_suffix("]]"))
        .unwrap_or(raw);

    // Strip alias after | (keep everything before the first |)
    let before_alias = inner.split('|').next().unwrap_or(inner);

    // Strip heading ref after # (keep everything before the first #)
    let before_heading = before_alias.split('#').next().unwrap_or(before_alias);

    let trimmed = before_heading.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Look up the URL slug for a page name using case-insensitive comparison
/// against `page_slug_map`.
///
/// Returns `Some(slug)` if found, `None` if the page name is not in the map.
///
/// REQ-015-004, CON-015-001.
pub fn resolve_page_name(page_name: &str, page_slug_map: &HashMap<String, String>) -> Option<String> {
    page_slug_map
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(page_name))
        .map(|(_, v)| v.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── extract_chain_target ─────────────────────────────────────────────────

    #[test]
    fn plain_string() {
        let fm = json!({ "prev": "Some Page" });
        assert_eq!(
            extract_chain_target(&fm, "prev"),
            Some("Some Page".to_string())
        );
    }

    #[test]
    fn wikilink_syntax() {
        let fm = json!({ "next": "[[Target Page]]" });
        assert_eq!(
            extract_chain_target(&fm, "next"),
            Some("Target Page".to_string())
        );
    }

    #[test]
    fn aliased_wikilink() {
        let fm = json!({ "prev": "[[Target Page|Display Text]]" });
        assert_eq!(
            extract_chain_target(&fm, "prev"),
            Some("Target Page".to_string())
        );
    }

    #[test]
    fn heading_ref_plain() {
        let fm = json!({ "next": "Target Page#Section" });
        assert_eq!(
            extract_chain_target(&fm, "next"),
            Some("Target Page".to_string())
        );
    }

    #[test]
    fn heading_ref_wikilink() {
        let fm = json!({ "prev": "[[Target Page#Section]]" });
        assert_eq!(
            extract_chain_target(&fm, "prev"),
            Some("Target Page".to_string())
        );
    }

    #[test]
    fn aliased_wikilink_with_heading() {
        // alias takes precedence: strip heading first, then alias
        let fm = json!({ "next": "[[Target Page#Section|Display]]" });
        assert_eq!(
            extract_chain_target(&fm, "next"),
            Some("Target Page".to_string())
        );
    }

    #[test]
    fn whitespace_trimmed() {
        let fm = json!({ "prev": "  Some Page  " });
        assert_eq!(
            extract_chain_target(&fm, "prev"),
            Some("Some Page".to_string())
        );
    }

    #[test]
    fn whitespace_inside_wikilink_trimmed() {
        let fm = json!({ "prev": "[[ Some Page ]]" });
        assert_eq!(
            extract_chain_target(&fm, "prev"),
            Some("Some Page".to_string())
        );
    }

    #[test]
    fn field_absent() {
        let fm = json!({ "title": "No Chain" });
        assert_eq!(extract_chain_target(&fm, "prev"), None);
    }

    #[test]
    fn field_not_a_string() {
        let fm = json!({ "prev": 42 });
        assert_eq!(extract_chain_target(&fm, "prev"), None);
    }

    #[test]
    fn empty_string_returns_none() {
        let fm = json!({ "prev": "" });
        assert_eq!(extract_chain_target(&fm, "prev"), None);
    }

    #[test]
    fn empty_wikilink_returns_none() {
        let fm = json!({ "prev": "[[]]" });
        assert_eq!(extract_chain_target(&fm, "prev"), None);
    }

    #[test]
    fn only_heading_ref_returns_none() {
        // "[[#Section]]" — no page name, just a heading anchor
        let fm = json!({ "prev": "[[#Section]]" });
        assert_eq!(extract_chain_target(&fm, "prev"), None);
    }

    // ── resolve_page_name ────────────────────────────────────────────────────

    fn make_slug_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn exact_match() {
        let map = make_slug_map(&[("My Page", "my-page")]);
        assert_eq!(
            resolve_page_name("My Page", &map),
            Some("my-page".to_string())
        );
    }

    #[test]
    fn case_insensitive_match() {
        let map = make_slug_map(&[("My Page", "my-page")]);
        assert_eq!(
            resolve_page_name("my page", &map),
            Some("my-page".to_string())
        );
        assert_eq!(
            resolve_page_name("MY PAGE", &map),
            Some("my-page".to_string())
        );
    }

    #[test]
    fn not_found_returns_none() {
        let map = make_slug_map(&[("My Page", "my-page")]);
        assert_eq!(resolve_page_name("Other Page", &map), None);
    }

    #[test]
    fn empty_map_returns_none() {
        let map = HashMap::new();
        assert_eq!(resolve_page_name("Anything", &map), None);
    }

    #[test]
    fn nested_slug_returned() {
        let map = make_slug_map(&[("Scanner", "architecture/Scanner")]);
        assert_eq!(
            resolve_page_name("scanner", &map),
            Some("architecture/Scanner".to_string())
        );
    }
}
