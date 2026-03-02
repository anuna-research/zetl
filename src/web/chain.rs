use std::collections::{HashMap, HashSet};

/// Maximum number of steps when walking a chain in either direction.
const CHAIN_WALK_LIMIT: usize = 1000;

/// Compute the 1-based position, total length, and head slug for a page in a chain.
///
/// Walks backward via `prev` pointers to find the chain head (a page with no
/// prev), then forward via `next` pointers to count the total chain length and
/// locate `page_name`.
///
/// Returns `None` if:
/// - `page_name` has no entry in `chain_prev_next` (not declared in any chain), or
/// - both its prev and next pointers are `None`, or
/// - `page_name` is unreachable when walking forward from the head (broken chain).
///
/// Cycle detection: already-visited page names are not revisited.
/// Capped at `CHAIN_WALK_LIMIT` steps in each direction.
///
/// REQ-015-005, REQ-015-007.
pub fn compute_chain_position(
    page_name: &str,
    chain_prev_next: &HashMap<String, (Option<String>, Option<String>)>,
    page_slug_map: &HashMap<String, String>,
) -> Option<(usize, usize, String)> {
    let (prev, next) = chain_prev_next.get(page_name)?;
    if prev.is_none() && next.is_none() {
        return None;
    }

    // ── Step 1: walk backward to the chain head ─────────────────────────────
    let mut current = page_name.to_string();
    let mut visited: HashSet<String> = HashSet::new();
    visited.insert(current.clone());

    for _ in 0..CHAIN_WALK_LIMIT {
        let prev = chain_prev_next
            .get(&current)
            .and_then(|(p, _)| p.as_ref())
            .cloned();
        match prev {
            Some(p) if !visited.contains(&p) => {
                visited.insert(p.clone());
                current = p;
            }
            _ => break,
        }
    }

    let head = current;
    let head_slug = page_slug_map.get(&head)?.clone();

    // ── Step 2: walk forward to count length and find position ───────────────
    let mut length = 0;
    let mut position = None;
    let mut current = head.clone();
    let mut seen: HashSet<String> = HashSet::new();
    seen.insert(current.clone());

    loop {
        length += 1;
        if current == page_name {
            position = Some(length);
        }
        if length >= CHAIN_WALK_LIMIT {
            break;
        }
        let next = chain_prev_next
            .get(&current)
            .and_then(|(_, n)| n.as_ref())
            .cloned();
        match next {
            Some(n) if !seen.contains(&n) => {
                seen.insert(n.clone());
                current = n;
            }
            _ => break,
        }
    }

    Some((position?, length, head_slug))
}

/// Return all chain heads: pages that have a `next` pointer but no `prev`.
///
/// These are the natural starting points of chains. The returned list is
/// sorted by page name.
pub fn find_chain_heads(
    chain_prev_next: &HashMap<String, (Option<String>, Option<String>)>,
) -> Vec<String> {
    let mut heads: Vec<String> = chain_prev_next
        .iter()
        .filter(|(_, (prev, next))| prev.is_none() && next.is_some())
        .map(|(name, _)| name.clone())
        .collect();
    heads.sort();
    heads
}

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

    // ── compute_chain_position ───────────────────────────────────────────────

    fn make_chain_map(
        entries: &[(&str, Option<&str>, Option<&str>)],
    ) -> HashMap<String, (Option<String>, Option<String>)> {
        entries
            .iter()
            .map(|(name, prev, next)| {
                (
                    name.to_string(),
                    (prev.map(|s| s.to_string()), next.map(|s| s.to_string())),
                )
            })
            .collect()
    }

    #[test]
    fn chain_position_three_page_linear() {
        // A → B → C
        let cm = make_chain_map(&[
            ("A", None, Some("B")),
            ("B", Some("A"), Some("C")),
            ("C", Some("B"), None),
        ]);
        let sm = make_slug_map(&[("A", "a"), ("B", "b"), ("C", "c")]);
        assert_eq!(
            compute_chain_position("A", &cm, &sm),
            Some((1, 3, "a".to_string()))
        );
        assert_eq!(
            compute_chain_position("B", &cm, &sm),
            Some((2, 3, "a".to_string()))
        );
        assert_eq!(
            compute_chain_position("C", &cm, &sm),
            Some((3, 3, "a".to_string()))
        );
    }

    #[test]
    fn chain_position_single_link_head_only() {
        // A → B (only A has next; B has prev but no next)
        let cm = make_chain_map(&[("A", None, Some("B")), ("B", Some("A"), None)]);
        let sm = make_slug_map(&[("A", "a"), ("B", "b")]);
        assert_eq!(
            compute_chain_position("A", &cm, &sm),
            Some((1, 2, "a".to_string()))
        );
        assert_eq!(
            compute_chain_position("B", &cm, &sm),
            Some((2, 2, "a".to_string()))
        );
    }

    #[test]
    fn chain_position_not_in_chain_returns_none() {
        let cm = make_chain_map(&[("A", None, Some("B")), ("B", Some("A"), None)]);
        let sm = make_slug_map(&[("A", "a"), ("B", "b"), ("C", "c")]);
        assert_eq!(compute_chain_position("C", &cm, &sm), None);
    }

    #[test]
    fn chain_position_absent_from_map_returns_none() {
        let cm = HashMap::new();
        let sm = make_slug_map(&[("A", "a")]);
        assert_eq!(compute_chain_position("A", &cm, &sm), None);
    }

    #[test]
    fn chain_position_cycle_detected() {
        // A → B → A (cycle)
        let cm = make_chain_map(&[("A", Some("B"), Some("B")), ("B", Some("A"), Some("A"))]);
        let sm = make_slug_map(&[("A", "a"), ("B", "b")]);
        // Should return Some (not hang), with a valid position
        let result = compute_chain_position("A", &cm, &sm);
        assert!(result.is_some());
    }

    #[test]
    fn chain_position_head_slug_is_nested() {
        let cm = make_chain_map(&[("X", None, Some("Y")), ("Y", Some("X"), None)]);
        let sm = make_slug_map(&[("X", "folder/x"), ("Y", "folder/y")]);
        assert_eq!(
            compute_chain_position("Y", &cm, &sm),
            Some((2, 2, "folder/x".to_string()))
        );
    }

    // ── find_chain_heads ─────────────────────────────────────────────────────

    #[test]
    fn chain_heads_detects_pages_with_next_no_prev() {
        let cm = make_chain_map(&[
            ("A", None, Some("B")),
            ("B", Some("A"), Some("C")),
            ("C", Some("B"), None),
        ]);
        assert_eq!(find_chain_heads(&cm), vec!["A".to_string()]);
    }

    #[test]
    fn chain_heads_excludes_middle_and_tail() {
        let cm = make_chain_map(&[
            ("X", None, Some("Y")),
            ("Y", Some("X"), Some("Z")),
            ("Z", Some("Y"), None),
        ]);
        let heads = find_chain_heads(&cm);
        assert_eq!(heads, vec!["X".to_string()]);
    }

    #[test]
    fn chain_heads_multiple_chains() {
        let cm = make_chain_map(&[
            ("A", None, Some("B")),
            ("B", Some("A"), None),
            ("P", None, Some("Q")),
            ("Q", Some("P"), None),
        ]);
        let mut heads = find_chain_heads(&cm);
        heads.sort();
        assert_eq!(heads, vec!["A".to_string(), "P".to_string()]);
    }

    #[test]
    fn chain_heads_empty_map() {
        let cm: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
        assert_eq!(find_chain_heads(&cm), Vec::<String>::new());
    }

    #[test]
    fn chain_heads_tail_only_not_a_head() {
        // Page with only prev (tail of chain) should not appear as head
        let cm = make_chain_map(&[("T", Some("Prev"), None)]);
        assert_eq!(find_chain_heads(&cm), Vec::<String>::new());
    }
}
