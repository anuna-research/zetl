use crate::types::ParsedFile;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const CACHE_DIR: &str = ".zetl";
const CACHE_FILE: &str = "index.json";
const CACHE_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct CacheIndex {
    version: u32,
    files: HashMap<PathBuf, ParsedFile>,
}

/// Load cached index from .zetl/index.json
pub fn load_cache(vault_root: &Path) -> Result<Option<HashMap<PathBuf, ParsedFile>>> {
    let cache_path = vault_root.join(CACHE_DIR).join(CACHE_FILE);
    if !cache_path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&cache_path)?;
    let index: CacheIndex = serde_json::from_str(&content)?;
    if index.version != CACHE_VERSION {
        return Ok(None);
    }
    Ok(Some(index.files))
}

/// Save parsed files to .zetl/index.json
pub fn save_cache(vault_root: &Path, files: &[ParsedFile]) -> Result<()> {
    let cache_dir = vault_root.join(CACHE_DIR);
    std::fs::create_dir_all(&cache_dir)?;
    let cache_path = cache_dir.join(CACHE_FILE);

    let mut file_map = HashMap::new();
    for f in files {
        file_map.insert(f.path.clone(), f.clone());
    }

    let index = CacheIndex {
        version: CACHE_VERSION,
        files: file_map,
    };

    let json = serde_json::to_string_pretty(&index)?;
    std::fs::write(&cache_path, json)?;
    Ok(())
}

/// Determine which files need re-parsing based on mtime changes
pub fn files_needing_reparse(
    cached: &HashMap<PathBuf, ParsedFile>,
    current_files: &[(PathBuf, std::time::SystemTime)],
) -> Vec<PathBuf> {
    current_files
        .iter()
        .filter(|(path, mtime)| {
            cached
                .get(path)
                .map(|cached_file| cached_file.mtime != *mtime)
                .unwrap_or(true)
        })
        .map(|(path, _)| path.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ParsedFile;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use tempfile::TempDir;

    /// Helper: build a minimal ParsedFile with the given relative path and mtime.
    fn make_parsed_file(rel_path: &str, mtime: SystemTime) -> ParsedFile {
        ParsedFile {
            path: PathBuf::from(rel_path),
            page_name: rel_path
                .strip_suffix(".md")
                .unwrap_or(rel_path)
                .to_string(),
            links: vec![],
            spl_blocks: vec![],
            diagnostics: vec![],
            mtime,
        }
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = TempDir::new().unwrap();
        let vault = dir.path();

        // Use a fixed mtime to avoid floating-point jitter from SystemTime::now().
        let mtime_a = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let mtime_b = UNIX_EPOCH + Duration::from_secs(1_700_001_000);

        let files = vec![
            make_parsed_file("notes/alpha.md", mtime_a),
            make_parsed_file("notes/beta.md", mtime_b),
        ];

        save_cache(vault, &files).unwrap();

        let loaded = load_cache(vault).unwrap().expect("cache should exist");
        assert_eq!(loaded.len(), 2);

        let alpha = loaded.get(Path::new("notes/alpha.md")).unwrap();
        assert_eq!(alpha.page_name, "notes/alpha");
        assert_eq!(alpha.mtime, mtime_a);

        let beta = loaded.get(Path::new("notes/beta.md")).unwrap();
        assert_eq!(beta.page_name, "notes/beta");
        assert_eq!(beta.mtime, mtime_b);
    }

    #[test]
    fn missing_cache_file_returns_none() {
        let dir = TempDir::new().unwrap();
        let result = load_cache(dir.path()).unwrap();
        assert!(result.is_none(), "missing cache file should return None");
    }

    #[test]
    fn version_mismatch_returns_none() {
        let dir = TempDir::new().unwrap();
        let cache_dir = dir.path().join(CACHE_DIR);
        std::fs::create_dir_all(&cache_dir).unwrap();

        // Write a cache file with a different version number.
        let bad_index = serde_json::json!({
            "version": CACHE_VERSION + 999,
            "files": {}
        });
        std::fs::write(
            cache_dir.join(CACHE_FILE),
            serde_json::to_string(&bad_index).unwrap(),
        )
        .unwrap();

        let result = load_cache(dir.path()).unwrap();
        assert!(result.is_none(), "version mismatch should return None");
    }

    #[test]
    fn files_needing_reparse_detects_changed_mtime() {
        let t1 = UNIX_EPOCH + Duration::from_secs(1_000_000);
        let t2 = UNIX_EPOCH + Duration::from_secs(2_000_000);

        let cached: HashMap<PathBuf, ParsedFile> = [
            (
                PathBuf::from("a.md"),
                make_parsed_file("a.md", t1),
            ),
            (
                PathBuf::from("b.md"),
                make_parsed_file("b.md", t1),
            ),
        ]
        .into_iter()
        .collect();

        let current_files = vec![
            (PathBuf::from("a.md"), t1), // unchanged
            (PathBuf::from("b.md"), t2), // mtime changed
            (PathBuf::from("c.md"), t1), // new file, not in cache
        ];

        let mut need_reparse = files_needing_reparse(&cached, &current_files);
        need_reparse.sort();

        assert_eq!(need_reparse, vec![PathBuf::from("b.md"), PathBuf::from("c.md")]);
    }

    #[test]
    fn files_needing_reparse_empty_cache_returns_all() {
        let cached: HashMap<PathBuf, ParsedFile> = HashMap::new();
        let t = UNIX_EPOCH + Duration::from_secs(1_000_000);

        let current_files = vec![
            (PathBuf::from("x.md"), t),
            (PathBuf::from("y.md"), t),
        ];

        let need_reparse = files_needing_reparse(&cached, &current_files);
        assert_eq!(need_reparse.len(), 2, "all files should need parsing when cache is empty");
    }

    #[test]
    fn files_needing_reparse_unchanged_returns_empty() {
        let t = UNIX_EPOCH + Duration::from_secs(1_500_000);

        let cached: HashMap<PathBuf, ParsedFile> = [(
            PathBuf::from("only.md"),
            make_parsed_file("only.md", t),
        )]
        .into_iter()
        .collect();

        let current_files = vec![(PathBuf::from("only.md"), t)];

        let need_reparse = files_needing_reparse(&cached, &current_files);
        assert!(need_reparse.is_empty(), "unchanged file should not need reparse");
    }

    #[test]
    fn save_overwrites_existing_cache() {
        let dir = TempDir::new().unwrap();
        let vault = dir.path();
        let t = UNIX_EPOCH + Duration::from_secs(1_700_000_000);

        // First save
        let files_v1 = vec![make_parsed_file("old.md", t)];
        save_cache(vault, &files_v1).unwrap();

        // Second save with different content
        let files_v2 = vec![make_parsed_file("new.md", t)];
        save_cache(vault, &files_v2).unwrap();

        let loaded = load_cache(vault).unwrap().expect("cache should exist");
        assert_eq!(loaded.len(), 1);
        assert!(loaded.contains_key(Path::new("new.md")));
        assert!(!loaded.contains_key(Path::new("old.md")));
    }
}
