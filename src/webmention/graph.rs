//! Read-side merge of external mentions into the existing backlinks
//! view. Keeps the `LinkGraph` type unchanged in v1 — external edges
//! live in `.zetl/webmentions/received.jsonl` and are consulted at
//! render time.

use std::path::Path;

use crate::webmention::persist::load_external_edges;
use crate::webmention::types::ExternalEdge;

/// Read the live external-backlink set for a target URL. Live = not
/// tombstoned. Caller filters by `target` after load to avoid re-reading
/// per query in v1; with the volumes a self-hosted vault sees this is
/// O(receive-log-length) per render and is acceptable.
pub fn external_backlinks(vault_root: &Path, target: &str) -> std::io::Result<Vec<ExternalEdge>> {
    let all = load_external_edges(vault_root)?;
    Ok(all.into_iter().filter(|e| e.target == target).collect())
}

/// Read every live external backlink, regardless of target. Used by the
/// CLI list view + observability counters.
pub fn all_external_backlinks(vault_root: &Path) -> std::io::Result<Vec<ExternalEdge>> {
    load_external_edges(vault_root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::webmention::persist::append_external_edge;
    use crate::webmention::types::ExternalEdge;
    use tempfile::tempdir;

    fn edge(src: &str, tgt: &str) -> ExternalEdge {
        ExternalEdge {
            source: src.into(),
            target: tgt.into(),
            accepted_at: 1,
            last_seen: 1,
            source_title: None,
            tombstoned: false,
        }
    }

    #[test]
    fn external_backlinks_filters_by_target() {
        let dir = tempdir().unwrap();
        append_external_edge(
            dir.path(),
            &edge("https://a.example/", "https://me.example/p"),
        )
        .unwrap();
        append_external_edge(
            dir.path(),
            &edge("https://b.example/", "https://me.example/q"),
        )
        .unwrap();
        let r = external_backlinks(dir.path(), "https://me.example/p").unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].source, "https://a.example/");
    }
}
