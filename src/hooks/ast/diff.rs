//! Tree-aware structural diff between two zetl-ext AST documents
//! (SPEC-032 REQ-3225 `zetl ast diff`).
//!
//! The diff operates on [`serde_json::Value`] rather than the typed
//! [`crate::hooks::ast::Document`] so it works even when one side of the
//! comparison contains nodes a downstream schema version introduced — the
//! CLI surface is useful for forward-compat exploration, not just
//! v1.0-exact values.
//!
//! Output shape:
//!
//! - **Added**   — node present in `after`, absent at the same path in `before`.
//! - **Removed** — inverse.
//! - **Modified** — a node kept its `type` but changed one or more
//!   non-children attributes. `attr_changes` lists the field-level
//!   before/after pairs. If only the `position` moved, we still report the
//!   modification so a source-position shift after a transform is visible.
//!
//! Matching heuristic: each parent pairs its children by position — same
//! index, same `type` → recurse into that pair; mismatched `type` at the
//! same index → flag the before side as Removed and the after side as
//! Added. This is cheap, stable, and matches the "did my transform do
//! what I expected?" mental model that motivates the command.

use serde_json::Value;

/// Kind of structural change applied to a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AstDiffKind {
    Added,
    Removed,
    Modified,
}

impl AstDiffKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AstDiffKind::Added => "added",
            AstDiffKind::Removed => "removed",
            AstDiffKind::Modified => "modified",
        }
    }
}

/// Single change to the tree.
#[derive(Debug, Clone, PartialEq)]
pub struct AstDiffEntry {
    pub kind: AstDiffKind,
    /// JSON-pointer-style path (e.g. `/children/2/children/0`) anchoring the
    /// change in the *before* document for Removed / Modified, or the
    /// *after* document for Added.
    pub path: String,
    /// The node type at the change site, for human-readable output.
    /// `None` when the node has no `type` field (e.g. primitive leaf).
    pub node_type: Option<String>,
    /// 1-indexed (line, col) of the change site, read from the node's
    /// `position`. `None` for nodes without a position.
    pub start_line: Option<u32>,
    pub start_col: Option<u32>,
    /// For [`AstDiffKind::Modified`], the per-attribute changes that
    /// motivated the diff. Empty for Added / Removed.
    pub attr_changes: Vec<AttrChange>,
}

/// Single field-level change on a modified node.
#[derive(Debug, Clone, PartialEq)]
pub struct AttrChange {
    pub field: String,
    pub before: Value,
    pub after: Value,
}

/// Collected diff over a full document pair.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AstDiff {
    pub entries: Vec<AstDiffEntry>,
}

impl AstDiff {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Compute the tree-aware diff between `before` and `after`.
pub fn diff_documents(before: &Value, after: &Value) -> AstDiff {
    let mut out = AstDiff::default();
    compare_node(before, after, String::new(), &mut out);
    out
}

fn compare_node(before: &Value, after: &Value, path: String, out: &mut AstDiff) {
    let before_type = node_type(before);
    let after_type = node_type(after);

    // Type mismatch at the same path → drop + add (keeps the report
    // intuitive rather than emitting a modification that crosses node
    // identities).
    if before_type != after_type {
        if let Some(_bt) = before_type.clone() {
            out.entries
                .push(entry(AstDiffKind::Removed, path.clone(), before));
        }
        if let Some(_at) = after_type {
            out.entries.push(entry(AstDiffKind::Added, path, after));
        }
        return;
    }

    // Non-object / non-array values: direct comparison only. (The tree
    // walker bottoms out on primitives; attribute-level diffs at the object
    // level handle field changes.)
    if !before.is_object() {
        return;
    }

    let before_obj = before.as_object().unwrap();
    let after_obj = after.as_object().unwrap();

    // Attribute-level changes (non-children, non-Document-only keys).
    let mut attr_changes = Vec::new();
    let mut all_keys: Vec<&String> = before_obj
        .keys()
        .chain(after_obj.keys())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    all_keys.sort();
    for key in &all_keys {
        let key = key.as_str();
        if key == "children" || key == "frontmatter" {
            continue;
        }
        let bv = before_obj.get(key).unwrap_or(&Value::Null);
        let av = after_obj.get(key).unwrap_or(&Value::Null);
        if bv != av {
            attr_changes.push(AttrChange {
                field: key.to_string(),
                before: bv.clone(),
                after: av.clone(),
            });
        }
    }

    // Frontmatter (Document-only) is compared as a single attr; leaf diffs
    // inside it aren't meaningful at the AST-tree level.
    if before_obj.contains_key("frontmatter") || after_obj.contains_key("frontmatter") {
        let bv = before_obj.get("frontmatter").unwrap_or(&Value::Null);
        let av = after_obj.get("frontmatter").unwrap_or(&Value::Null);
        if bv != av {
            attr_changes.push(AttrChange {
                field: "frontmatter".into(),
                before: bv.clone(),
                after: av.clone(),
            });
        }
    }

    if !attr_changes.is_empty() {
        let mut e = entry(AstDiffKind::Modified, path.clone(), before);
        e.attr_changes = attr_changes;
        out.entries.push(e);
    }

    // Recurse into children arrays.
    let before_children = children_of(before);
    let after_children = children_of(after);
    let n = before_children.len().max(after_children.len());
    for i in 0..n {
        let child_path = format!("{path}/children/{i}");
        match (before_children.get(i), after_children.get(i)) {
            (Some(b), Some(a)) => compare_node(b, a, child_path, out),
            (Some(b), None) => {
                out.entries.push(entry(AstDiffKind::Removed, child_path, b));
            }
            (None, Some(a)) => {
                out.entries.push(entry(AstDiffKind::Added, child_path, a));
            }
            (None, None) => {}
        }
    }
}

fn children_of(v: &Value) -> &[Value] {
    v.get("children")
        .and_then(|c| c.as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn node_type(v: &Value) -> Option<String> {
    v.get("type").and_then(|t| t.as_str()).map(str::to_owned)
}

fn entry(kind: AstDiffKind, path: String, node: &Value) -> AstDiffEntry {
    let (start_line, start_col) = node
        .get("position")
        .map(|p| {
            let sl = p
                .get("start_line")
                .and_then(Value::as_u64)
                .map(|n| n as u32);
            let sc = p.get("start_col").and_then(Value::as_u64).map(|n| n as u32);
            (sl, sc)
        })
        .unwrap_or((None, None));
    AstDiffEntry {
        kind,
        path,
        node_type: node_type(node),
        start_line,
        start_col,
        attr_changes: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc(children: Value) -> Value {
        json!({
            "ast_version": "1.0",
            "type": "Document",
            "position": {"start_line":1,"start_col":1,"end_line":1,"end_col":1},
            "children": children
        })
    }

    fn paragraph(pos: (u32, u32), text: &str) -> Value {
        json!({
            "type": "Paragraph",
            "position": {"start_line": pos.0, "start_col": pos.1, "end_line": pos.0, "end_col": pos.1},
            "children": [
                {
                    "type": "Text",
                    "position": {"start_line": pos.0, "start_col": pos.1, "end_line": pos.0, "end_col": pos.1},
                    "text": text
                }
            ]
        })
    }

    #[test]
    fn identical_documents_produce_empty_diff() {
        let a = doc(json!([paragraph((1, 1), "hello")]));
        let b = doc(json!([paragraph((1, 1), "hello")]));
        assert!(diff_documents(&a, &b).is_empty());
    }

    #[test]
    fn added_block_is_detected() {
        let a = doc(json!([paragraph((1, 1), "hello")]));
        let b = doc(json!([
            paragraph((1, 1), "hello"),
            paragraph((3, 1), "world")
        ]));
        let d = diff_documents(&a, &b);
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].kind, AstDiffKind::Added);
        assert_eq!(d.entries[0].node_type.as_deref(), Some("Paragraph"));
        assert_eq!(d.entries[0].start_line, Some(3));
        assert_eq!(d.entries[0].path, "/children/1");
    }

    #[test]
    fn removed_block_is_detected() {
        let a = doc(json!([
            paragraph((1, 1), "hello"),
            paragraph((3, 1), "world")
        ]));
        let b = doc(json!([paragraph((1, 1), "hello")]));
        let d = diff_documents(&a, &b);
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].kind, AstDiffKind::Removed);
        assert_eq!(d.entries[0].path, "/children/1");
    }

    #[test]
    fn attr_change_surfaces_as_modification() {
        let a = json!({
            "type": "Wikilink",
            "position": {"start_line":1,"start_col":1,"end_line":1,"end_col":10},
            "target": "Alpha",
            "alias": null,
            "heading": null,
            "block_id": null
        });
        let b = json!({
            "type": "Wikilink",
            "position": {"start_line":1,"start_col":1,"end_line":1,"end_col":10},
            "target": "Alpha",
            "alias": "renamed",
            "heading": null,
            "block_id": null
        });
        let d = diff_documents(&a, &b);
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].kind, AstDiffKind::Modified);
        assert_eq!(d.entries[0].attr_changes.len(), 1);
        assert_eq!(d.entries[0].attr_changes[0].field, "alias");
        assert_eq!(d.entries[0].attr_changes[0].after, "renamed");
    }

    #[test]
    fn type_change_at_same_index_is_removed_plus_added() {
        let a = doc(json!([paragraph((1, 1), "hello")]));
        let b = doc(json!([{
            "type": "ThematicBreak",
            "position": {"start_line":1,"start_col":1,"end_line":1,"end_col":3}
        }]));
        let d = diff_documents(&a, &b);
        let kinds: Vec<_> = d.entries.iter().map(|e| e.kind).collect();
        assert!(kinds.contains(&AstDiffKind::Removed));
        assert!(kinds.contains(&AstDiffKind::Added));
    }

    #[test]
    fn frontmatter_change_is_one_attr_modification() {
        let a = json!({
            "ast_version": "1.0",
            "type": "Document",
            "position": {"start_line":1,"start_col":1,"end_line":1,"end_col":1},
            "frontmatter": {"title": "before"},
            "children": []
        });
        let b = json!({
            "ast_version": "1.0",
            "type": "Document",
            "position": {"start_line":1,"start_col":1,"end_line":1,"end_col":1},
            "frontmatter": {"title": "after"},
            "children": []
        });
        let d = diff_documents(&a, &b);
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].kind, AstDiffKind::Modified);
        assert_eq!(d.entries[0].attr_changes.len(), 1);
        assert_eq!(d.entries[0].attr_changes[0].field, "frontmatter");
    }

    #[test]
    fn nested_attr_change_carries_full_path() {
        let a = doc(json!([paragraph((1, 1), "hello")]));
        let b = doc(json!([paragraph((1, 1), "world")]));
        let d = diff_documents(&a, &b);
        // The Paragraph itself is unchanged (type + position match); the
        // Text child's `text` attribute changed.
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].kind, AstDiffKind::Modified);
        assert_eq!(d.entries[0].path, "/children/0/children/0");
        assert_eq!(d.entries[0].attr_changes[0].field, "text");
    }
}
