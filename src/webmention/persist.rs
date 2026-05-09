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
        rationale: None,
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

/// Remove every queue entry whose `(source, target)` matches the given
/// pair. Atomic write-to-tempfile + rename so a crash mid-rewrite leaves
/// the original file intact. Used by the moderator-decision CLI path
/// (`zetl webmention accept|reject`) to dequeue a decided mention so it
/// stops surfacing in `zetl webmention list`.
pub fn remove_from_queue(vault_root: &Path, source: &str, target: &str) -> std::io::Result<usize> {
    use std::io::Write;
    let queue_path = vault_dir(vault_root).join(QUEUE_FILE);
    let entries: Vec<IncomingMention> = read_jsonl(&queue_path)?;
    let before = entries.len();
    let kept: Vec<IncomingMention> = entries
        .into_iter()
        .filter(|m| !(m.source == source && m.target == target))
        .collect();
    let removed = before - kept.len();
    if removed == 0 {
        return Ok(0);
    }
    if !queue_path.exists() {
        return Ok(0);
    }
    let parent = queue_path
        .parent()
        .ok_or_else(|| std::io::Error::other("queue path has no parent"))?;
    std::fs::create_dir_all(parent)?;
    let tmp = parent.join(format!("{QUEUE_FILE}.tmp"));
    {
        let mut f = std::fs::File::create(&tmp)?;
        for m in &kept {
            let line = serde_json::to_string(m)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            f.write_all(line.as_bytes())?;
            f.write_all(b"\n")?;
        }
        f.sync_data()?;
    }
    std::fs::rename(&tmp, &queue_path)?;
    Ok(removed)
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
            rationale: None,
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
    fn remove_from_queue_atomically_filters_decided_entries() {
        let dir = tempdir().unwrap();
        let m1 = crate::webmention::types::IncomingMention {
            source: "https://a.example/x".into(),
            target: "https://me.example/p".into(),
            received_at: 1,
            ..Default::default()
        };
        let m2 = crate::webmention::types::IncomingMention {
            source: "https://b.example/y".into(),
            target: "https://me.example/p".into(),
            received_at: 2,
            ..Default::default()
        };
        append_queue(dir.path(), &m1).unwrap();
        append_queue(dir.path(), &m2).unwrap();
        let removed =
            remove_from_queue(dir.path(), "https://a.example/x", "https://me.example/p").unwrap();
        assert_eq!(removed, 1);
        let remaining = load_queue(dir.path()).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].source, "https://b.example/y");
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
