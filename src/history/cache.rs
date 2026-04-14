//! HistoricalIndexCache: per-vault-root-hash index snapshots (REQ-079, ADR-047).
//!
//! Stores snapshots of the zetl index in `.zetl/history/<vault_root_hash>.json`,
//! using the same JSON format as `.zetl/index.json` (version + files map +
//! vault_root_hash).  A bounded LRU eviction policy (default 100 entries) keeps
//! disk usage under control by removing the least-recently-written entries when
//! capacity is exceeded.

use crate::types::ParsedFile;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const HISTORY_SUBDIR: &str = ".zetl/history";
/// Cache version — must match the `index.json` format version (ADR-047).
const CACHE_VERSION: u32 = 2;
/// Default maximum number of history entries retained on disk.
pub const DEFAULT_CAPACITY: usize = 100;

/// On-disk format — identical to `CacheIndex` in `cache.rs` (ADR-047).
#[derive(Debug, Serialize, Deserialize)]
struct HistoricalIndex {
    version: u32,
    files: HashMap<PathBuf, ParsedFile>,
    #[serde(default)]
    vault_root_hash: Option<String>,
}

/// A disk-backed, bounded LRU cache of historical zetl index snapshots.
///
/// Each entry is stored as `.zetl/history/<vault_root_hash>.json` inside the
/// vault.  The file format is identical to `.zetl/index.json` so that the same
/// deserialization path can be used for both live and historical indexes.
///
/// # Eviction
///
/// After every [`store`](HistoricalIndexCache::store) call the directory is
/// scanned; if the number of `.json` files exceeds `capacity` the oldest files
/// (by last-modified time) are removed until the count equals `capacity`.
pub struct HistoricalIndexCache {
    capacity: usize,
}

impl HistoricalIndexCache {
    /// Create a new cache with a custom `capacity`.
    pub fn new(capacity: usize) -> Self {
        Self { capacity }
    }

    /// Create a new cache with [`DEFAULT_CAPACITY`] (100 entries).
    pub fn with_default_capacity() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }

    /// Absolute path for a single entry inside `vault_root`.
    fn entry_path(vault_root: &Path, vault_root_hash: &str) -> PathBuf {
        vault_root
            .join(HISTORY_SUBDIR)
            .join(format!("{vault_root_hash}.json"))
    }

    /// Load the snapshot for `vault_root_hash`, if it exists.
    ///
    /// Returns `Ok(None)` if the entry is absent or has an incompatible version.
    pub fn load(
        &self,
        vault_root: &Path,
        vault_root_hash: &str,
    ) -> Result<Option<HashMap<PathBuf, ParsedFile>>> {
        let path = Self::entry_path(vault_root, vault_root_hash);
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(&path)?;
        let index: HistoricalIndex = serde_json::from_str(&content)?;
        if index.version != CACHE_VERSION {
            return Ok(None);
        }
        Ok(Some(index.files))
    }

    /// Persist `files` as the snapshot for `vault_root_hash`.
    ///
    /// Creates `.zetl/history/` if it does not exist.  After writing, evicts
    /// the oldest entries until at most `capacity` entries remain.
    pub fn store(
        &self,
        vault_root: &Path,
        vault_root_hash: &str,
        files: &[ParsedFile],
    ) -> Result<()> {
        let history_dir = vault_root.join(HISTORY_SUBDIR);
        std::fs::create_dir_all(&history_dir)?;

        let file_map: HashMap<PathBuf, ParsedFile> =
            files.iter().map(|f| (f.path.clone(), f.clone())).collect();

        let index = HistoricalIndex {
            version: CACHE_VERSION,
            files: file_map,
            vault_root_hash: Some(vault_root_hash.to_owned()),
        };

        let json = serde_json::to_string_pretty(&index)?;
        std::fs::write(Self::entry_path(vault_root, vault_root_hash), json)?;

        self.evict_lru(&history_dir)?;
        Ok(())
    }

    /// Remove the oldest `.json` entries from `history_dir` until at most
    /// `self.capacity` entries remain.
    fn evict_lru(&self, history_dir: &Path) -> Result<()> {
        let mut entries: Vec<(PathBuf, std::time::SystemTime)> = std::fs::read_dir(history_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
            .filter_map(|e| {
                let mtime = e.metadata().ok()?.modified().ok()?;
                Some((e.path(), mtime))
            })
            .collect();

        if entries.len() <= self.capacity {
            return Ok(());
        }

        // Oldest first (smallest mtime).
        entries.sort_by_key(|(_, mtime)| *mtime);

        let to_remove = entries.len() - self.capacity;
        for (path, _) in entries.into_iter().take(to_remove) {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ParsedFile;
    use std::time::SystemTime;

    fn dummy_file(name: &str) -> ParsedFile {
        ParsedFile {
            path: PathBuf::from(name),
            page_name: name.trim_end_matches(".md").to_owned(),
            links: vec![],
            spl_blocks: vec![],
            diagnostics: vec![],
            mtime: SystemTime::UNIX_EPOCH,
            merkle_leaves: vec![],
            file_merkle: None,
        }
    }

    fn make_hash(c: char) -> String {
        c.to_string().repeat(64)
    }

    #[test]
    fn roundtrip_store_load() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache = HistoricalIndexCache::with_default_capacity();
        let hash = make_hash('a');
        let files = vec![dummy_file("note.md")];

        cache.store(dir.path(), &hash, &files).unwrap();
        let loaded = cache
            .load(dir.path(), &hash)
            .unwrap()
            .expect("must be Some");
        assert!(loaded.contains_key(Path::new("note.md")));
    }

    #[test]
    fn missing_entry_returns_none() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache = HistoricalIndexCache::with_default_capacity();
        assert!(cache.load(dir.path(), &make_hash('b')).unwrap().is_none());
    }

    #[test]
    fn lru_eviction_respects_capacity() {
        let dir = tempfile::TempDir::new().unwrap();
        let capacity = 3;
        let cache = HistoricalIndexCache::new(capacity);

        for i in 0u8..5 {
            // Use distinct hashes (each 64 hex chars).
            let hash = format!("{i:064x}");
            cache
                .store(dir.path(), &hash, &[dummy_file("x.md")])
                .unwrap();
            // Small delay so mtime ordering is deterministic.
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        let history_dir = dir.path().join(HISTORY_SUBDIR);
        let count = std::fs::read_dir(&history_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .count();
        assert_eq!(
            count, capacity,
            "eviction must keep exactly {capacity} entries"
        );
    }

    #[test]
    fn store_overwrites_existing_entry() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache = HistoricalIndexCache::with_default_capacity();
        let hash = make_hash('c');

        cache
            .store(dir.path(), &hash, &[dummy_file("v1.md")])
            .unwrap();
        cache
            .store(dir.path(), &hash, &[dummy_file("v2.md")])
            .unwrap();

        let loaded = cache
            .load(dir.path(), &hash)
            .unwrap()
            .expect("must be Some");
        assert!(
            loaded.contains_key(Path::new("v2.md")),
            "second write must win"
        );
        assert!(!loaded.contains_key(Path::new("v1.md")));
    }
}
