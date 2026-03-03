use std::collections::HashMap;

/// Extract a chain target page name from a frontmatter field value.
///
/// Handles both plain strings and wikilink syntax:
///   "Scene 2"           -> Some("Scene 2")
///   "[[Scene 2]]"       -> Some("Scene 2")
///   "[[Scene 2|Next]]"  -> Some("Scene 2")
///   "[[Scene 2#Act 1]]" -> Some("Scene 2")
///
/// Returns None if the field is missing or not a string.
pub fn extract_chain_target(fm: &serde_json::Value, field: &str) -> Option<String> {
    let raw = fm.get(field)?.as_str()?;
    let mut s = raw.trim();

    // Strip wikilink delimiters if present
    if s.starts_with("[[") && s.ends_with("]]") {
        s = &s[2..s.len() - 2];
    }

    // Strip alias (take text before pipe)
    if let Some(pipe_pos) = s.find('|') {
        s = &s[..pipe_pos];
    }

    // Strip heading/block reference (take text before #)
    if let Some(hash_pos) = s.find('#') {
        s = &s[..hash_pos];
    }

    let result = s.trim();
    if result.is_empty() {
        None
    } else {
        Some(result.to_string())
    }
}

/// Resolve a page name to its slug using case-insensitive matching.
///
/// Returns `(resolved_name, slug)` if found, where `resolved_name` is the
/// canonical key from the map.
pub fn resolve_page_name(
    name: &str,
    page_slug_map: &HashMap<String, String>,
) -> Option<(String, String)> {
    page_slug_map
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(k, v)| (k.clone(), v.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plain_string() {
        let fm = json!({"next": "Scene 2"});
        assert_eq!(
            extract_chain_target(&fm, "next"),
            Some("Scene 2".to_string())
        );
    }

    #[test]
    fn wikilink() {
        let fm = json!({"next": "[[Scene 2]]"});
        assert_eq!(
            extract_chain_target(&fm, "next"),
            Some("Scene 2".to_string())
        );
    }

    #[test]
    fn aliased_wikilink() {
        let fm = json!({"next": "[[Scene 2|alias]]"});
        assert_eq!(
            extract_chain_target(&fm, "next"),
            Some("Scene 2".to_string())
        );
    }

    #[test]
    fn heading_ref() {
        let fm = json!({"next": "[[Scene 2#heading]]"});
        assert_eq!(
            extract_chain_target(&fm, "next"),
            Some("Scene 2".to_string())
        );
    }

    #[test]
    fn combined_heading_and_alias() {
        let fm = json!({"next": "[[Scene 2#heading|alias]]"});
        assert_eq!(
            extract_chain_target(&fm, "next"),
            Some("Scene 2".to_string())
        );
    }

    #[test]
    fn missing_field() {
        let fm = json!({"other": "value"});
        assert_eq!(extract_chain_target(&fm, "next"), None);
    }

    #[test]
    fn non_string_field() {
        let fm = json!({"next": 42});
        assert_eq!(extract_chain_target(&fm, "next"), None);
    }

    #[test]
    fn resolve_case_insensitive() {
        let mut map = HashMap::new();
        map.insert("Scene 2".to_string(), "scene-2".to_string());
        map.insert("Epilogue".to_string(), "epilogue".to_string());

        let result = resolve_page_name("scene 2", &map);
        assert!(result.is_some());
        let (name, slug) = result.unwrap();
        assert_eq!(name, "Scene 2");
        assert_eq!(slug, "scene-2");
    }

    #[test]
    fn resolve_no_match() {
        let mut map = HashMap::new();
        map.insert("Scene 2".to_string(), "scene-2".to_string());

        assert_eq!(resolve_page_name("nonexistent", &map), None);
    }
}
