use std::collections::{HashMap, HashSet};

use serde::Serialize;

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

// ── Chain validation diagnostics ─────────────────────────────────────────────

/// A single chain integrity diagnostic produced by `validate_chain_links`.
///
/// REQ-015-006.
#[derive(Debug, Clone, Serialize)]
pub struct ChainDiagnostic {
    /// Severity: `"error"` or `"warning"`.
    pub level: String,
    /// Machine-readable kind:
    /// `"broken_forward_link"`, `"asymmetric"`, `"cycle"`, `"fan_in"`, `"orphaned"`.
    pub kind: String,
    /// Human-readable description of the problem.
    pub message: String,
}

/// Validate chain integrity across all markdown files in the vault.
///
/// Reads `prev`/`next` frontmatter from every `.md` file, builds forward and
/// backward adjacency maps, and returns diagnostics for:
///
/// - **broken_forward_link** (error): declared prev/next target does not exist.
/// - **asymmetric** (warning): A → next → B but B.prev ≠ A.
/// - **cycle** (error): cycle in chain, message includes full cycle path.
/// - **fan_in** (error): multiple pages declare the same page as their `next`.
/// - **orphaned** (warning): page is in a chain but unreachable from every head.
///
/// REQ-015-006.
pub fn validate_chain_links(
    vault_root: &std::path::Path,
    files: &[crate::types::ParsedFile],
    page_slug_map: &HashMap<String, String>,
) -> Vec<ChainDiagnostic> {
    use super::markdown::parse_frontmatter;

    let mut diags: Vec<ChainDiagnostic> = Vec::new();

    // ── Phase 1: read raw frontmatter ────────────────────────────────────────
    // raw_decls: page_name → (Option<raw_prev_target>, Option<raw_next_target>)
    let mut raw_decls: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
    for file in files {
        if !file.path.extension().map_or(false, |e| e == "md") {
            continue;
        }
        let full = vault_root.join(&file.path);
        let content = match std::fs::read_to_string(&full) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let fm = parse_frontmatter(&content);
        let prev_raw = extract_chain_target(&fm, "prev");
        let next_raw = extract_chain_target(&fm, "next");
        if prev_raw.is_some() || next_raw.is_some() {
            raw_decls.insert(file.page_name.clone(), (prev_raw, next_raw));
        }
    }

    // ── Phase 2: resolve targets; emit broken_forward_link errors ────────────
    // resolved_map: page_name → (Option<canonical_prev>, Option<canonical_next>)
    let mut resolved_map: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
    for (page, (prev_raw, next_raw)) in &raw_decls {
        let canon_prev = prev_raw.as_ref().and_then(|t| {
            let r = canonical_page_name_check(t, page_slug_map);
            if r.is_none() {
                diags.push(ChainDiagnostic {
                    level: "error".to_string(),
                    kind: "broken_forward_link".to_string(),
                    message: format!(
                        "chain: broken forward link: '{}' declares prev='{}' (page does not exist)",
                        page, t
                    ),
                });
            }
            r
        });
        let canon_next = next_raw.as_ref().and_then(|t| {
            let r = canonical_page_name_check(t, page_slug_map);
            if r.is_none() {
                diags.push(ChainDiagnostic {
                    level: "error".to_string(),
                    kind: "broken_forward_link".to_string(),
                    message: format!(
                        "chain: broken forward link: '{}' declares next='{}' (page does not exist)",
                        page, t
                    ),
                });
            }
            r
        });
        resolved_map.insert(page.clone(), (canon_prev, canon_next));
    }

    // ── Phase 3: build backward_map (who points to each page via `next`) ─────
    let mut backward_map: HashMap<String, Vec<String>> = HashMap::new();
    for (page, (_, next)) in &resolved_map {
        if let Some(n) = next {
            backward_map.entry(n.clone()).or_default().push(page.clone());
        }
    }

    // ── Phase 4: fan-in (multiple predecessors) ───────────────────────────────
    for (target, preds) in &backward_map {
        if preds.len() > 1 {
            let mut sorted = preds.clone();
            sorted.sort();
            diags.push(ChainDiagnostic {
                level: "error".to_string(),
                kind: "fan_in".to_string(),
                message: format!(
                    "chain: fan-in: '{}' is pointed to (as next) by multiple pages: {}",
                    target,
                    sorted
                        .iter()
                        .map(|p| format!("'{p}'"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            });
        }
    }

    // ── Phase 5: asymmetry ───────────────────────────────────────────────────
    for (a, (_, next)) in &resolved_map {
        if let Some(b) = next {
            let b_prev = resolved_map.get(b).and_then(|(p, _)| p.as_ref());
            if b_prev.map_or(true, |p| p != a) {
                let b_prev_str = b_prev
                    .map(|p| format!("'{p}'"))
                    .unwrap_or_else(|| "(none)".to_string());
                diags.push(ChainDiagnostic {
                    level: "warning".to_string(),
                    kind: "asymmetric".to_string(),
                    message: format!(
                        "chain: asymmetric: '{a}' → next → '{b}' but '{b}'.prev = {b_prev_str}"
                    ),
                });
            }
        }
    }

    // ── Phase 6: cycle detection ─────────────────────────────────────────────
    // Since each node has at most one outgoing next edge, the graph is a
    // collection of simple paths (possibly with a cycle at one end).
    let mut globally_visited: HashSet<String> = HashSet::new();
    let mut in_cycle: HashSet<String> = HashSet::new();

    let mut all_pages: Vec<String> = resolved_map.keys().cloned().collect();
    all_pages.sort();

    for start in &all_pages {
        if globally_visited.contains(start) {
            continue;
        }

        let mut walk_path: Vec<String> = Vec::new();
        let mut walk_set: HashSet<String> = HashSet::new();
        let mut current = start.clone();

        loop {
            if globally_visited.contains(&current) {
                // Already processed in a previous walk; stop.
                break;
            }
            if walk_set.contains(&current) {
                // Re-encountered a node in the current walk → cycle detected.
                if let Some(idx) = walk_path.iter().position(|p| *p == current) {
                    let cycle_pages = &walk_path[idx..];
                    let min_page = cycle_pages.iter().min().cloned().unwrap_or_default();
                    // Report cycle once (keyed on the lexicographic minimum member).
                    if !in_cycle.contains(&min_page) {
                        for p in cycle_pages {
                            in_cycle.insert(p.clone());
                        }
                        let path_str = cycle_pages
                            .iter()
                            .map(|p| format!("'{p}'"))
                            .collect::<Vec<_>>()
                            .join(" → ");
                        diags.push(ChainDiagnostic {
                            level: "error".to_string(),
                            kind: "cycle".to_string(),
                            message: format!(
                                "chain: cycle detected: {path_str} → '{current}'"
                            ),
                        });
                    }
                }
                break;
            }

            walk_path.push(current.clone());
            walk_set.insert(current.clone());

            let next = resolved_map
                .get(&current)
                .and_then(|(_, n)| n.clone());
            match next {
                Some(n) => current = n,
                None => break,
            }
        }

        for p in &walk_path {
            globally_visited.insert(p.clone());
        }
    }

    // ── Phase 7: orphaned nodes ──────────────────────────────────────────────
    // Orphaned: in resolved_map but unreachable from any chain head (page with
    // resolved_prev = None), and not already reported as part of a cycle.
    let mut reachable: HashSet<String> = HashSet::new();
    let heads: Vec<String> = resolved_map
        .iter()
        .filter(|(_, (prev, _))| prev.is_none())
        .map(|(name, _)| name.clone())
        .collect();

    for head in &heads {
        let mut cur = head.clone();
        let mut seen: HashSet<String> = HashSet::new();
        loop {
            if reachable.contains(&cur) || seen.contains(&cur) {
                break;
            }
            seen.insert(cur.clone());
            reachable.insert(cur.clone());
            let next = resolved_map.get(&cur).and_then(|(_, n)| n.clone());
            match next {
                Some(n) => cur = n,
                None => break,
            }
        }
    }

    let mut orphan_names: Vec<String> = resolved_map
        .keys()
        .filter(|p| !reachable.contains(*p) && !in_cycle.contains(*p))
        .cloned()
        .collect();
    orphan_names.sort();

    for name in orphan_names {
        diags.push(ChainDiagnostic {
            level: "warning".to_string(),
            kind: "orphaned".to_string(),
            message: format!(
                "chain: orphaned node: '{name}' is in a chain but unreachable from any chain head"
            ),
        });
    }

    diags
}

/// Case-insensitive lookup of the canonical page name in `page_slug_map`.
///
/// Returns `Some(canonical_name)` if found, `None` if not present.
fn canonical_page_name_check(
    target: &str,
    page_slug_map: &HashMap<String, String>,
) -> Option<String> {
    page_slug_map
        .keys()
        .find(|k| k.eq_ignore_ascii_case(target))
        .cloned()
}

/// Collect a verbose chain summary: number of chains and their lengths.
///
/// A chain is a maximal run of pages reachable from a chain head (prev=None,
/// next=Some) by following `next` pointers.  Stops at a dead end or cycle.
///
/// Returns `(chain_count, lengths)` where `lengths` is sorted ascending.
///
/// OBS-015-001.
pub fn chain_summary(
    chain_prev_next: &HashMap<String, (Option<String>, Option<String>)>,
) -> (usize, Vec<usize>) {
    let heads: Vec<String> = chain_prev_next
        .iter()
        .filter(|(_, (prev, next))| prev.is_none() && next.is_some())
        .map(|(name, _)| name.clone())
        .collect();

    let mut lengths: Vec<usize> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for head in heads {
        if seen.contains(&head) {
            continue;
        }
        let mut length = 0usize;
        let mut cur = head.clone();
        let mut walk_seen: HashSet<String> = HashSet::new();
        loop {
            if walk_seen.contains(&cur) {
                break; // cycle guard
            }
            walk_seen.insert(cur.clone());
            seen.insert(cur.clone());
            length += 1;
            let next = chain_prev_next
                .get(&cur)
                .and_then(|(_, n)| n.as_ref())
                .cloned();
            match next {
                Some(n) => cur = n,
                None => break,
            }
        }
        if length > 0 {
            lengths.push(length);
        }
    }

    lengths.sort_unstable();
    let chain_count = lengths.len();
    (chain_count, lengths)
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
