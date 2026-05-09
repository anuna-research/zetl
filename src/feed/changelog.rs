//! AST-backed changelog feed per REQ-3816 + REQ-3817 + CON-3810.
//!
//! Emits change events for stable, nameable nodes when comparing
//! consecutive AST Merkle snapshots. Each event carries:
//!
//!   * `event_id` — stable per (object_id, content_hash) pair
//!   * `object_id` — page slug or anchor + block id
//!   * `event_type` — `added` / `updated` / `deleted` / `moved`
//!   * `source_path` — vault-relative path
//!   * `node_kind` — `page` / `heading` / `block`
//!   * `selector` — operator-readable selector
//!   * `prev_content_hash`, `current_content_hash`
//!   * `changelog_seq` — monotonic publisher-local sequence number
//!
//! The pure side (this module) takes two snapshots + a base sequence
//! number and emits a `Vec<ChangeEvent>`. The caller persists the new
//! snapshot + appends events to `events.jsonl` per CON-3810.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Snapshot of a vault's stable nodes. Each entry maps a stable
/// `object_id` -> the latest `content_hash` for that node.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AstSnapshot {
    pub nodes: BTreeMap<String, NodeFingerprint>,
}

/// Per-node fingerprint at snapshot time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeFingerprint {
    pub source_path: String,
    pub node_kind: NodeKind,
    pub selector: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NodeKind {
    Page,
    Heading,
    Block,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EventType {
    Added,
    Updated,
    Deleted,
    Moved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeEvent {
    pub event_id: String,
    pub object_id: String,
    pub event_type: EventType,
    pub source_path: String,
    pub node_kind: NodeKind,
    pub selector: String,
    pub prev_content_hash: Option<String>,
    pub current_content_hash: Option<String>,
    pub changelog_seq: u64,
    pub at: String,
}

/// Diff two snapshots and produce events. `start_seq` is the next
/// sequence number to assign; the function returns the post-diff seq
/// alongside the events for the caller to persist. `at` is the build
/// timestamp (RFC 3339) — passed in so the function stays pure.
pub fn diff_snapshots(
    prev: &AstSnapshot,
    curr: &AstSnapshot,
    start_seq: u64,
    at: &str,
) -> (Vec<ChangeEvent>, u64) {
    let mut events = Vec::new();
    let mut seq = start_seq;
    // Walk current snapshot for added + updated + moved.
    for (obj_id, node) in &curr.nodes {
        match prev.nodes.get(obj_id) {
            None => {
                events.push(ChangeEvent {
                    event_id: format!("{obj_id}:{}", node.content_hash),
                    object_id: obj_id.clone(),
                    event_type: EventType::Added,
                    source_path: node.source_path.clone(),
                    node_kind: node.node_kind,
                    selector: node.selector.clone(),
                    prev_content_hash: None,
                    current_content_hash: Some(node.content_hash.clone()),
                    changelog_seq: seq,
                    at: at.to_string(),
                });
                seq += 1;
            }
            Some(prev_node) => {
                if prev_node.source_path != node.source_path {
                    events.push(ChangeEvent {
                        event_id: format!(
                            "{obj_id}:moved:{}->{}",
                            prev_node.content_hash, node.content_hash
                        ),
                        object_id: obj_id.clone(),
                        event_type: EventType::Moved,
                        source_path: node.source_path.clone(),
                        node_kind: node.node_kind,
                        selector: node.selector.clone(),
                        prev_content_hash: Some(prev_node.content_hash.clone()),
                        current_content_hash: Some(node.content_hash.clone()),
                        changelog_seq: seq,
                        at: at.to_string(),
                    });
                    seq += 1;
                } else if prev_node.content_hash != node.content_hash {
                    events.push(ChangeEvent {
                        event_id: format!("{obj_id}:{}", node.content_hash),
                        object_id: obj_id.clone(),
                        event_type: EventType::Updated,
                        source_path: node.source_path.clone(),
                        node_kind: node.node_kind,
                        selector: node.selector.clone(),
                        prev_content_hash: Some(prev_node.content_hash.clone()),
                        current_content_hash: Some(node.content_hash.clone()),
                        changelog_seq: seq,
                        at: at.to_string(),
                    });
                    seq += 1;
                }
            }
        }
    }
    // Walk previous snapshot for deletions.
    for (obj_id, node) in &prev.nodes {
        if !curr.nodes.contains_key(obj_id) {
            events.push(ChangeEvent {
                event_id: format!("{obj_id}:deleted:{}", node.content_hash),
                object_id: obj_id.clone(),
                event_type: EventType::Deleted,
                source_path: node.source_path.clone(),
                node_kind: node.node_kind,
                selector: node.selector.clone(),
                prev_content_hash: Some(node.content_hash.clone()),
                current_content_hash: None,
                changelog_seq: seq,
                at: at.to_string(),
            });
            seq += 1;
        }
    }
    (events, seq)
}

/// Determine which archive range a sequence number falls into.
/// `archive_size` is the sealing window per CON-3810.
pub fn archive_range_for(seq: u64, archive_size: u32) -> (u64, u64) {
    let size = archive_size as u64;
    let start = (seq / size) * size;
    (start, start + size - 1)
}

/// Serialise an event log into JSONL (one event per line).
pub fn events_to_jsonl(events: &[ChangeEvent]) -> Result<String, serde_json::Error> {
    let mut buf = String::new();
    for e in events {
        buf.push_str(&serde_json::to_string(e)?);
        buf.push('\n');
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp(path: &str, hash: &str) -> NodeFingerprint {
        NodeFingerprint {
            source_path: path.to_string(),
            node_kind: NodeKind::Page,
            selector: path.to_string(),
            content_hash: hash.to_string(),
        }
    }

    #[test]
    fn diff_added_when_object_appears() {
        let prev = AstSnapshot::default();
        let mut curr = AstSnapshot::default();
        curr.nodes.insert("foo".to_string(), fp("foo.md", "h1"));
        let (events, next_seq) = diff_snapshots(&prev, &curr, 0, "2026-05-08T00:00:00Z");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, EventType::Added);
        assert_eq!(next_seq, 1);
    }

    #[test]
    fn diff_updated_when_hash_changes() {
        let mut prev = AstSnapshot::default();
        prev.nodes.insert("foo".to_string(), fp("foo.md", "h1"));
        let mut curr = AstSnapshot::default();
        curr.nodes.insert("foo".to_string(), fp("foo.md", "h2"));
        let (events, _) = diff_snapshots(&prev, &curr, 0, "2026-05-08T00:00:00Z");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, EventType::Updated);
    }

    #[test]
    fn diff_deleted_when_object_disappears() {
        let mut prev = AstSnapshot::default();
        prev.nodes.insert("foo".to_string(), fp("foo.md", "h1"));
        let curr = AstSnapshot::default();
        let (events, _) = diff_snapshots(&prev, &curr, 0, "2026-05-08T00:00:00Z");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, EventType::Deleted);
    }

    #[test]
    fn diff_moved_when_source_path_changes() {
        let mut prev = AstSnapshot::default();
        prev.nodes.insert("foo".to_string(), fp("a.md", "h1"));
        let mut curr = AstSnapshot::default();
        curr.nodes.insert("foo".to_string(), fp("b.md", "h2"));
        let (events, _) = diff_snapshots(&prev, &curr, 0, "2026-05-08T00:00:00Z");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, EventType::Moved);
    }

    #[test]
    fn sequence_numbers_monotonic() {
        let prev = AstSnapshot::default();
        let mut curr = AstSnapshot::default();
        for i in 0..3 {
            curr.nodes
                .insert(format!("p{i}"), fp(&format!("p{i}.md"), "h"));
        }
        let (events, next) = diff_snapshots(&prev, &curr, 100, "2026-05-08T00:00:00Z");
        let seqs: Vec<u64> = events.iter().map(|e| e.changelog_seq).collect();
        assert_eq!(seqs, vec![100, 101, 102]);
        assert_eq!(next, 103);
    }

    #[test]
    fn event_id_is_seq_independent_for_moved_and_deleted() {
        // REQ-3817: event_id is stable per (object_id, content_hash)
        // pair regardless of where the running seq counter sits.
        let mut prev = AstSnapshot::default();
        prev.nodes.insert("foo".to_string(), fp("a.md", "h1"));
        let mut curr = AstSnapshot::default();
        curr.nodes.insert("foo".to_string(), fp("b.md", "h2"));
        let (e_at_zero, _) = diff_snapshots(&prev, &curr, 0, "2026-05-08T00:00:00Z");
        let (e_at_thousand, _) = diff_snapshots(&prev, &curr, 1000, "2026-05-08T00:00:00Z");
        assert_eq!(e_at_zero[0].event_id, e_at_thousand[0].event_id);

        // Same shape for Deleted.
        let mut prev = AstSnapshot::default();
        prev.nodes.insert("foo".to_string(), fp("a.md", "h1"));
        let curr = AstSnapshot::default();
        let (e_at_zero, _) = diff_snapshots(&prev, &curr, 0, "2026-05-08T00:00:00Z");
        let (e_at_thousand, _) = diff_snapshots(&prev, &curr, 1000, "2026-05-08T00:00:00Z");
        assert_eq!(e_at_zero[0].event_id, e_at_thousand[0].event_id);
    }

    #[test]
    fn no_events_when_snapshots_identical() {
        let mut s = AstSnapshot::default();
        s.nodes.insert("foo".to_string(), fp("foo.md", "h1"));
        let (events, next) = diff_snapshots(&s, &s, 0, "2026-05-08T00:00:00Z");
        assert!(events.is_empty());
        assert_eq!(next, 0);
    }

    #[test]
    fn archive_range_aligned() {
        assert_eq!(archive_range_for(0, 1000), (0, 999));
        assert_eq!(archive_range_for(999, 1000), (0, 999));
        assert_eq!(archive_range_for(1000, 1000), (1000, 1999));
        assert_eq!(archive_range_for(1500, 1000), (1000, 1999));
    }

    #[test]
    fn jsonl_round_trip() {
        let events = vec![ChangeEvent {
            event_id: "e1".to_string(),
            object_id: "foo".to_string(),
            event_type: EventType::Added,
            source_path: "foo.md".to_string(),
            node_kind: NodeKind::Page,
            selector: "foo".to_string(),
            prev_content_hash: None,
            current_content_hash: Some("h1".to_string()),
            changelog_seq: 0,
            at: "2026-05-08T00:00:00Z".to_string(),
        }];
        let jsonl = events_to_jsonl(&events).unwrap();
        let lines: Vec<&str> = jsonl.lines().collect();
        assert_eq!(lines.len(), 1);
        let v: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(v["event_id"], "e1");
        assert_eq!(v["event_type"], "added");
    }

    #[test]
    fn deleted_events_use_post_addition_sequence() {
        // Added comes first then deletion picks up the next seq.
        let mut prev = AstSnapshot::default();
        prev.nodes.insert("old".to_string(), fp("old.md", "h"));
        let mut curr = AstSnapshot::default();
        curr.nodes.insert("new".to_string(), fp("new.md", "h"));
        let (events, _) = diff_snapshots(&prev, &curr, 10, "2026-05-08T00:00:00Z");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, EventType::Added);
        assert_eq!(events[0].changelog_seq, 10);
        assert_eq!(events[1].event_type, EventType::Deleted);
        assert_eq!(events[1].changelog_seq, 11);
    }
}
