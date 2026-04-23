//! CRDT Write-Ahead Log for crash recovery (REQ-020-044).
//!
//! Each CRDT operation is appended to `.ztl/crdt/<slug>.wal` as a JSON line.
//! On successful flush the WAL is truncated. On startup, any non-empty WAL
//! files are replayed to recover edits that were not yet flushed.
//!
//! A per-document WAL size limit (default 10 MB) triggers a force-flush when
//! exceeded, preventing unbounded disk growth.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use crate::web::ws::OpEntry;

/// Maximum WAL size per document before a force-flush is triggered (10 MB).
pub const MAX_WAL_SIZE: u64 = 10 * 1024 * 1024;

/// Manages WAL files under `.ztl/crdt/`.
#[derive(Clone, Debug)]
pub struct WalStore {
    /// Path to the `.ztl/crdt/` directory.
    crdt_dir: PathBuf,
}

impl WalStore {
    /// Create a new `WalStore` rooted at `vault_root/.ztl/crdt/`.
    pub fn new(vault_root: &Path) -> Self {
        Self {
            crdt_dir: vault_root.join(".ztl").join("crdt"),
        }
    }

    /// Resolve the WAL file path for a slug.
    ///
    /// Slugs may contain `/` for nested pages (e.g. `notes/daily`), so we
    /// percent-encode them to keep all WAL files in a single directory.
    /// `%` is encoded as `%25` first, then `/` as `%2F`, to avoid ambiguity.
    pub fn wal_path(&self, slug: &str) -> PathBuf {
        let safe_name = slug.replace('%', "%25").replace('/', "%2F");
        self.crdt_dir.join(format!("{safe_name}.wal"))
    }

    /// Ensure the `.ztl/crdt/` directory exists.
    fn ensure_dir(&self) -> std::io::Result<()> {
        fs::create_dir_all(&self.crdt_dir)
    }

    /// Append a batch of operations to the WAL for `slug`.
    ///
    /// Each call writes one JSON line containing the full `ops` array,
    /// preceded by a newline if the file is non-empty. The file is opened
    /// in append mode and flushed immediately.
    pub fn append_ops(&self, slug: &str, ops: &[OpEntry]) -> std::io::Result<()> {
        if ops.is_empty() {
            return Ok(());
        }
        self.ensure_dir()?;
        let path = self.wal_path(slug);
        let mut f = OpenOptions::new().create(true).append(true).open(&path)?;
        let json = serde_json::to_string(ops).map_err(std::io::Error::other)?;
        writeln!(f, "{json}")?;
        f.flush()?;
        // fsync to ensure durability
        f.sync_all()?;
        Ok(())
    }

    /// Truncate (delete) the WAL file for `slug` after a successful flush.
    pub fn truncate(&self, slug: &str) -> std::io::Result<()> {
        let path = self.wal_path(slug);
        if path.exists() {
            fs::remove_file(&path)?;
        }
        Ok(())
    }

    /// Return the current size of the WAL file for `slug` (0 if absent).
    pub fn wal_size(&self, slug: &str) -> u64 {
        let path = self.wal_path(slug);
        fs::metadata(&path).map(|m| m.len()).unwrap_or(0)
    }

    /// List all slugs that have non-empty WAL files (for startup replay).
    pub fn pending_slugs(&self) -> Vec<String> {
        let Ok(entries) = fs::read_dir(&self.crdt_dir) else {
            return Vec::new();
        };
        let mut slugs = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("wal") {
                // Check non-empty
                if fs::metadata(&path).map(|m| m.len() > 0).unwrap_or(false) {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        // Reverse the percent-encoding: `%2F` → `/`, then `%25` → `%`
                        let slug = stem.replace("%2F", "/").replace("%25", "%");
                        slugs.push(slug);
                    }
                }
            }
        }
        slugs
    }

    /// Replay all operations from a WAL file, returning them in order.
    ///
    /// Each line in the WAL file is a JSON array of `OpEntry`. Lines that
    /// fail to parse are skipped (logged to stderr).
    pub fn replay(&self, slug: &str) -> Vec<Vec<OpEntry>> {
        let path = self.wal_path(slug);
        let Ok(file) = File::open(&path) else {
            return Vec::new();
        };
        let reader = BufReader::new(file);
        let mut batches = Vec::new();
        for (line_num, line) in reader.lines().enumerate() {
            let Ok(line) = line else {
                continue;
            };
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Vec<OpEntry>>(&line) {
                Ok(ops) => batches.push(ops),
                Err(e) => {
                    eprintln!(
                        "warning: WAL replay for {slug}: skipping line {}: {e}",
                        line_num + 1
                    );
                }
            }
        }
        batches
    }
}

/// Replay all pending WAL files on server startup (REQ-020-044).
///
/// For each non-empty WAL:
/// 1. Load the page's markdown from disk into the CRDT store
/// 2. Replay all WAL operations
/// 3. Immediately flush via the flush pipeline
/// 4. Delete the WAL file (handled by flush_pipeline's truncate)
pub fn replay_pending_wals(state: &super::WebState) {
    let slugs = state.wal_store.pending_slugs();
    if slugs.is_empty() {
        return;
    }
    eprintln!("[ztl] WAL recovery: found {} pending WAL(s)", slugs.len());

    for slug in &slugs {
        // Register the real file path for this slug before loading.
        {
            let data = state.data.read().unwrap();
            if let Some(file) = data
                .files
                .iter()
                .find(|f| crate::scanner::page_slug_from_path(&f.path).eq_ignore_ascii_case(slug))
            {
                state
                    .crdt_store
                    .register_path(slug, state.vault_root.join(&file.path));
            }
        }

        // Step 1: Load page markdown into CRDT store
        if let Err(e) = state.crdt_store.load_or_get(slug) {
            eprintln!("[ztl] WAL recovery: failed to load {slug}: {e}");
            continue;
        }

        // Step 2: Replay WAL operations.
        // The WAL doesn't persist the originating user, and the operators
        // who sent these ops may no longer be connected; attributing the
        // recovery-triggered flush to any of them would be incorrect.
        // Pass an empty user_id so the contributor list stays unpopulated
        // and the flush pipeline falls back to the generic identity.
        let batches = state.wal_store.replay(slug);
        let mut op_count = 0;
        for ops in &batches {
            if let Err(e) = state.crdt_store.apply_ops(slug, "", ops) {
                eprintln!("[ztl] WAL recovery: op apply failed for {slug}: {e}");
            }
            op_count += ops.len();
        }
        eprintln!(
            "[ztl] WAL recovery: replayed {op_count} ops ({} batches) for {slug}",
            batches.len()
        );

        // Step 3–4: Flush to disk (this also truncates the WAL)
        match super::flush::flush_pipeline(state, slug) {
            Some(r) => {
                for w in &r.warnings {
                    eprintln!("[ztl] WAL recovery flush warning ({slug}): {w}");
                }
                eprintln!("[ztl] WAL recovery: flushed {slug}");
            }
            None => {
                // Flush returned None — doc may not be dirty if WAL ops were no-ops.
                // Clean up the WAL anyway.
                if let Err(e) = state.wal_store.truncate(slug) {
                    eprintln!("[ztl] WAL recovery: truncate failed for {slug}: {e}");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> (tempfile::TempDir, WalStore) {
        let dir = tempfile::TempDir::new().unwrap();
        let store = WalStore::new(dir.path());
        (dir, store)
    }

    #[test]
    fn append_and_replay_ops() {
        let (_dir, store) = test_store();
        let ops = vec![
            OpEntry::Splice {
                pos: 0,
                del: 0,
                text: "hello".into(),
            },
            OpEntry::Splice {
                pos: 5,
                del: 0,
                text: " world".into(),
            },
        ];
        store.append_ops("readme", &ops).unwrap();

        let batches = store.replay("readme");
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 2);
    }

    #[test]
    fn multiple_appends_produce_multiple_batches() {
        let (_dir, store) = test_store();
        store
            .append_ops(
                "page",
                &[OpEntry::Splice {
                    pos: 0,
                    del: 0,
                    text: "a".into(),
                }],
            )
            .unwrap();
        store
            .append_ops(
                "page",
                &[OpEntry::Splice {
                    pos: 1,
                    del: 0,
                    text: "b".into(),
                }],
            )
            .unwrap();

        let batches = store.replay("page");
        assert_eq!(batches.len(), 2);
    }

    #[test]
    fn truncate_removes_wal() {
        let (_dir, store) = test_store();
        store
            .append_ops(
                "page",
                &[OpEntry::Splice {
                    pos: 0,
                    del: 0,
                    text: "x".into(),
                }],
            )
            .unwrap();
        assert!(store.wal_size("page") > 0);

        store.truncate("page").unwrap();
        assert_eq!(store.wal_size("page"), 0);
        assert!(store.replay("page").is_empty());
    }

    #[test]
    fn truncate_nonexistent_is_ok() {
        let (_dir, store) = test_store();
        store.truncate("missing").unwrap();
    }

    #[test]
    fn pending_slugs_finds_non_empty_wals() {
        let (_dir, store) = test_store();
        store
            .append_ops(
                "alpha",
                &[OpEntry::Splice {
                    pos: 0,
                    del: 0,
                    text: "a".into(),
                }],
            )
            .unwrap();
        store
            .append_ops(
                "beta",
                &[OpEntry::Splice {
                    pos: 0,
                    del: 0,
                    text: "b".into(),
                }],
            )
            .unwrap();

        let mut slugs = store.pending_slugs();
        slugs.sort();
        assert_eq!(slugs, vec!["alpha", "beta"]);
    }

    #[test]
    fn empty_ops_not_appended() {
        let (_dir, store) = test_store();
        store.append_ops("page", &[]).unwrap();
        assert_eq!(store.wal_size("page"), 0);
    }

    #[test]
    fn wal_size_tracks_growth() {
        let (_dir, store) = test_store();
        assert_eq!(store.wal_size("page"), 0);
        store
            .append_ops(
                "page",
                &[OpEntry::Splice {
                    pos: 0,
                    del: 0,
                    text: "hello".into(),
                }],
            )
            .unwrap();
        let size = store.wal_size("page");
        assert!(size > 0, "WAL should have grown");
    }

    #[test]
    fn nested_slug_flattens_path() {
        let (_dir, store) = test_store();
        store
            .append_ops(
                "notes/daily",
                &[OpEntry::Splice {
                    pos: 0,
                    del: 0,
                    text: "x".into(),
                }],
            )
            .unwrap();

        // WAL file should use %2F encoding
        let wal_path = store.wal_path("notes/daily");
        assert!(wal_path
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .contains("%2F"));
        assert!(wal_path.exists());

        // Should appear in pending_slugs with original slug
        let slugs = store.pending_slugs();
        assert_eq!(slugs, vec!["notes/daily"]);
    }

    #[test]
    fn slug_with_percent_roundtrips() {
        let (_dir, store) = test_store();
        store
            .append_ops(
                "100%/done",
                &[OpEntry::Splice {
                    pos: 0,
                    del: 0,
                    text: "x".into(),
                }],
            )
            .unwrap();

        // Verify percent is encoded before slash
        let wal_path = store.wal_path("100%/done");
        let name = wal_path.file_name().unwrap().to_str().unwrap();
        assert!(name.contains("%25"), "percent should be encoded as %25");
        assert!(name.contains("%2F"), "slash should be encoded as %2F");

        let slugs = store.pending_slugs();
        assert_eq!(slugs, vec!["100%/done"]);
    }

    #[test]
    fn replay_skips_corrupt_lines() {
        let (_dir, store) = test_store();
        store.ensure_dir().unwrap();
        let path = store.wal_path("bad");
        let mut f = File::create(&path).unwrap();
        // Valid line
        writeln!(f, r#"[{{"action":"splice","pos":0,"del":0,"text":"ok"}}]"#).unwrap();
        // Corrupt line
        writeln!(f, "not json at all").unwrap();
        // Another valid line
        writeln!(f, r#"[{{"action":"splice","pos":2,"del":0,"text":"!"}}]"#).unwrap();
        f.flush().unwrap();

        let batches = store.replay("bad");
        assert_eq!(
            batches.len(),
            2,
            "should skip corrupt line and keep valid ones"
        );
    }

    #[test]
    fn replay_marks_and_unmarks() {
        let (_dir, store) = test_store();
        let ops = vec![
            OpEntry::Mark {
                name: "bold".into(),
                value: serde_json::Value::Bool(true),
                start: 0,
                end: 5,
            },
            OpEntry::Unmark {
                name: "bold".into(),
                start: 0,
                end: 5,
            },
        ];
        store.append_ops("styled", &ops).unwrap();

        let batches = store.replay("styled");
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 2);
    }
}
