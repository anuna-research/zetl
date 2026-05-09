//! Typed wrappers over [`crate::webmention::storage`] for the three
//! webmention JSONL files: `received.jsonl`, `sent.jsonl`, `queue.jsonl`.

use std::path::Path;

use crate::webmention::storage::{
    append_jsonl, read_jsonl, vault_dir, QUEUE_FILE, RECEIVED_FILE, SENT_FILE,
};
use crate::webmention::types::{ExternalEdge, IncomingMention, SentRecord};

/// Append an accepted external-backlink edge.
pub fn append_external_edge(vault_root: &Path, edge: &ExternalEdge) -> std::io::Result<()> {
    append_jsonl(&vault_dir(vault_root).join(RECEIVED_FILE), edge)
}

/// Read all live (non-tombstoned) external edges. Tombstones from the
/// same `(source, target)` pair shadow earlier accepts so a removal
/// chain produces the empty set.
pub fn load_external_edges(vault_root: &Path) -> std::io::Result<Vec<ExternalEdge>> {
    let raw: Vec<ExternalEdge> = read_jsonl(&vault_dir(vault_root).join(RECEIVED_FILE))?;
    Ok(fold_edges(raw))
}

fn fold_edges(raw: Vec<ExternalEdge>) -> Vec<ExternalEdge> {
    use std::collections::HashMap;
    let mut latest: HashMap<(String, String), ExternalEdge> = HashMap::new();
    for edge in raw {
        let key = (edge.source.clone(), edge.target.clone());
        latest.insert(key, edge);
    }
    latest.into_values().filter(|e| !e.tombstoned).collect()
}

/// Tombstone the edge — append a record with `tombstoned: true` so the
/// load path filters the live-set. Append-only, no in-place rewrite.
pub fn tombstone_external_edge(
    vault_root: &Path,
    source: &str,
    target: &str,
    last_seen: u64,
) -> std::io::Result<()> {
    let edge = ExternalEdge {
        source: source.to_string(),
        target: target.to_string(),
        accepted_at: last_seen,
        last_seen,
        source_title: None,
        tombstoned: true,
    };
    append_external_edge(vault_root, &edge)
}

/// Append a queued mention awaiting moderation.
pub fn append_queue(vault_root: &Path, mention: &IncomingMention) -> std::io::Result<()> {
    append_jsonl(&vault_dir(vault_root).join(QUEUE_FILE), mention)
}

pub fn load_queue(vault_root: &Path) -> std::io::Result<Vec<IncomingMention>> {
    read_jsonl(&vault_dir(vault_root).join(QUEUE_FILE))
}

/// Append a successful POST record to the sender's idempotency log.
pub fn append_sent_record(vault_root: &Path, record: &SentRecord) -> std::io::Result<()> {
    append_jsonl(&vault_dir(vault_root).join(SENT_FILE), record)
}

pub fn load_sent_log(vault_root: &Path) -> std::io::Result<Vec<SentRecord>> {
    read_jsonl(&vault_dir(vault_root).join(SENT_FILE))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::webmention::storage::ensure_dir;
    use tempfile::tempdir;

    fn fixture_edge(source: &str, target: &str, t: u64, tombstoned: bool) -> ExternalEdge {
        ExternalEdge {
            source: source.into(),
            target: target.into(),
            accepted_at: t,
            last_seen: t,
            source_title: None,
            tombstoned,
        }
    }

    #[test]
    fn external_edges_round_trip() {
        let dir = tempdir().unwrap();
        ensure_dir(dir.path()).unwrap();
        append_external_edge(
            dir.path(),
            &fixture_edge("https://a.example/", "https://me.example/p", 1, false),
        )
        .unwrap();
        append_external_edge(
            dir.path(),
            &fixture_edge("https://b.example/", "https://me.example/p", 2, false),
        )
        .unwrap();
        let live = load_external_edges(dir.path()).unwrap();
        assert_eq!(live.len(), 2);
    }

    #[test]
    fn tombstone_removes_edge_from_live_set() {
        let dir = tempdir().unwrap();
        append_external_edge(
            dir.path(),
            &fixture_edge("https://a.example/", "https://me.example/p", 1, false),
        )
        .unwrap();
        tombstone_external_edge(dir.path(), "https://a.example/", "https://me.example/p", 5)
            .unwrap();
        let live = load_external_edges(dir.path()).unwrap();
        assert!(live.is_empty());
    }

    #[test]
    fn re_accept_after_tombstone_brings_edge_back() {
        let dir = tempdir().unwrap();
        append_external_edge(
            dir.path(),
            &fixture_edge("https://a.example/", "https://me.example/p", 1, false),
        )
        .unwrap();
        tombstone_external_edge(dir.path(), "https://a.example/", "https://me.example/p", 5)
            .unwrap();
        append_external_edge(
            dir.path(),
            &fixture_edge("https://a.example/", "https://me.example/p", 10, false),
        )
        .unwrap();
        let live = load_external_edges(dir.path()).unwrap();
        assert_eq!(live.len(), 1);
    }
}
