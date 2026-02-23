use crate::types::{Diagnostic, ParsedFile};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

const CACHE_DIR: &str = ".zetl";
const CACHE_FILE: &str = "index.json";
const CACHE_VERSION: u32 = 1;

const THEORY_CACHE_FILE: &str = "theory.json";
const THEORY_CACHE_VERSION: u32 = 1;

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

// ── Theory cache ──────────────────────────────────────────────────────────

/// Cached representation of a rule suitable for reconstruction.
///
/// Uses string-based literal representations rather than interned `Literal`
/// objects, since spindle-core's interner state is process-local.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedRule {
    pub label: String,
    pub rule_type: CachedRuleType,
    /// Body literals as display-form strings (e.g. "bird", "~flies").
    pub body: Vec<String>,
    /// Head literals as display-form strings.
    pub head: Vec<String>,
    /// Source file relative to vault root.
    pub source_file: PathBuf,
    /// Absolute line number in the source file.
    pub source_line: u32,
    /// Page name.
    pub source_page: String,
}

/// Rule type for the theory cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CachedRuleType {
    Fact,
    Strict,
    Defeasible,
    Defeater,
}

/// Serialized theory cache stored in `.zetl/theory.json`.
///
/// Contains everything needed to reconstruct a spindle-core `Theory` without
/// re-parsing SPL text. Conclusions are NOT cached — re-reasoning happens on
/// every query (per ADR-006 / Open Question 4).
#[derive(Debug, Serialize, Deserialize)]
pub struct TheoryCache {
    version: u32,
    /// Mtimes (as seconds since UNIX epoch) of files containing SPL blocks.
    /// Used for targeted invalidation.
    spl_file_mtimes: HashMap<PathBuf, f64>,
    /// All rules (facts + named rules) with provenance.
    pub rules: Vec<CachedRule>,
    /// Superiority relations as (superior_label, inferior_label) pairs.
    pub superiorities: Vec<(String, String)>,
    /// Diagnostics collected during phases 1–4 (parse, annotate, combine, validate).
    pub diagnostics: Vec<Diagnostic>,
}

/// Load theory cache from `.zetl/theory.json`.
pub fn load_theory_cache(vault_root: &Path) -> Result<Option<TheoryCache>> {
    let cache_path = vault_root.join(CACHE_DIR).join(THEORY_CACHE_FILE);
    if !cache_path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&cache_path)?;
    let cache: TheoryCache = serde_json::from_str(&content)?;
    if cache.version != THEORY_CACHE_VERSION {
        return Ok(None);
    }
    Ok(Some(cache))
}

/// Save theory cache to `.zetl/theory.json`.
pub fn save_theory_cache(vault_root: &Path, cache: &TheoryCache) -> Result<()> {
    let cache_dir = vault_root.join(CACHE_DIR);
    std::fs::create_dir_all(&cache_dir)?;
    let cache_path = cache_dir.join(THEORY_CACHE_FILE);
    let json = serde_json::to_string_pretty(&cache)?;
    std::fs::write(&cache_path, json)?;
    Ok(())
}

/// Check whether the theory cache is still valid by comparing SPL file mtimes.
///
/// The cache is valid if and only if:
/// 1. The set of files containing SPL blocks is unchanged.
/// 2. Every SPL-containing file has the same mtime as when the cache was built.
pub fn theory_cache_valid(cache: &TheoryCache, files: &[ParsedFile]) -> bool {
    // Collect current SPL-containing files and their mtimes.
    let current_spl_mtimes: HashMap<&PathBuf, f64> = files
        .iter()
        .filter(|f| !f.spl_blocks.is_empty())
        .map(|f| {
            let secs = f
                .mtime
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_secs_f64();
            (&f.path, secs)
        })
        .collect();

    // Same number of SPL files?
    if current_spl_mtimes.len() != cache.spl_file_mtimes.len() {
        return false;
    }

    // Every cached file still present with same mtime?
    for (path, cached_mtime) in &cache.spl_file_mtimes {
        match current_spl_mtimes.get(path) {
            Some(&current_mtime) if (current_mtime - cached_mtime).abs() < 0.001 => {}
            _ => return false,
        }
    }

    true
}

/// Build a `TheoryCache` from the current pipeline state and a built theory.
///
/// Called after `build_theory()` succeeds to persist the parsed theory for
/// subsequent queries.
#[cfg(feature = "reason")]
pub fn build_theory_cache(
    theory: &spindle_core::prelude::Theory,
    diagnostics: &[Diagnostic],
    files: &[ParsedFile],
) -> TheoryCache {
    use spindle_core::prelude::{MetaValue, RuleType as CoreRuleType};

    // Collect SPL file mtimes.
    let spl_file_mtimes: HashMap<PathBuf, f64> = files
        .iter()
        .filter(|f| !f.spl_blocks.is_empty())
        .map(|f| {
            let secs = f
                .mtime
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_secs_f64();
            (f.path.clone(), secs)
        })
        .collect();

    // Extract rules from the theory.
    let rules: Vec<CachedRule> = theory
        .rules()
        .map(|rule| {
            let rule_type = match rule.rule_type {
                CoreRuleType::Fact => CachedRuleType::Fact,
                CoreRuleType::Strict => CachedRuleType::Strict,
                CoreRuleType::Defeasible => CachedRuleType::Defeasible,
                CoreRuleType::Defeater => CachedRuleType::Defeater,
            };
            let body: Vec<String> = rule.body.iter().map(|l| l.to_string()).collect();
            let head: Vec<String> = rule.head.iter().map(|l| l.to_string()).collect();

            // Extract provenance from theory metadata.
            let meta = theory.get_meta(&rule.label);
            let source_file = meta
                .and_then(|m| match m.properties.get("_source_file") {
                    Some(MetaValue::String(s)) => Some(PathBuf::from(s)),
                    _ => None,
                })
                .unwrap_or_default();
            let source_line = meta
                .and_then(|m| match m.properties.get("_source_line") {
                    Some(MetaValue::String(s)) => s.parse().ok(),
                    _ => None,
                })
                .unwrap_or(0);
            let source_page = meta
                .and_then(|m| match m.properties.get("_source_page") {
                    Some(MetaValue::String(s)) => Some(s.clone()),
                    _ => None,
                })
                .unwrap_or_default();

            CachedRule {
                label: rule.label.clone(),
                rule_type,
                body,
                head,
                source_file,
                source_line,
                source_page,
            }
        })
        .collect();

    // Extract superiority relations.
    let superiorities: Vec<(String, String)> = theory
        .superiorities()
        .iter()
        .map(|s| (s.superior.clone(), s.inferior.clone()))
        .collect();

    TheoryCache {
        version: THEORY_CACHE_VERSION,
        spl_file_mtimes,
        rules,
        superiorities,
        diagnostics: diagnostics.to_vec(),
    }
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

    // ── Theory cache tests ──────────────────────────────────────────────

    fn make_theory_cache(
        spl_file_mtimes: HashMap<PathBuf, f64>,
        rules: Vec<CachedRule>,
    ) -> TheoryCache {
        TheoryCache {
            version: THEORY_CACHE_VERSION,
            spl_file_mtimes,
            rules,
            superiorities: vec![],
            diagnostics: vec![],
        }
    }

    fn make_spl_file(rel_path: &str, mtime: SystemTime) -> ParsedFile {
        use crate::types::SplBlock;
        ParsedFile {
            path: PathBuf::from(rel_path),
            page_name: rel_path
                .strip_suffix(".md")
                .unwrap_or(rel_path)
                .to_string(),
            links: vec![],
            spl_blocks: vec![SplBlock {
                source_file: PathBuf::from(rel_path),
                source_page: "test".to_string(),
                start_line: 1,
                end_line: 2,
                content: "(given test)".to_string(),
            }],
            diagnostics: vec![],
            mtime,
        }
    }

    #[test]
    fn theory_cache_save_and_load_round_trip() {
        let dir = TempDir::new().unwrap();
        let vault = dir.path();

        let mut spl_mtimes = HashMap::new();
        spl_mtimes.insert(PathBuf::from("a.md"), 1_700_000_000.0);

        let cache = make_theory_cache(
            spl_mtimes,
            vec![CachedRule {
                label: "__fact_1".to_string(),
                rule_type: CachedRuleType::Fact,
                body: vec![],
                head: vec!["bird".to_string()],
                source_file: PathBuf::from("a.md"),
                source_line: 6,
                source_page: "A".to_string(),
            }],
        );

        save_theory_cache(vault, &cache).unwrap();

        let loaded = load_theory_cache(vault)
            .unwrap()
            .expect("theory cache should exist");
        assert_eq!(loaded.rules.len(), 1);
        assert_eq!(loaded.rules[0].label, "__fact_1");
        assert_eq!(loaded.rules[0].head, vec!["bird".to_string()]);
    }

    #[test]
    fn theory_cache_missing_returns_none() {
        let dir = TempDir::new().unwrap();
        let result = load_theory_cache(dir.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn theory_cache_version_mismatch_returns_none() {
        let dir = TempDir::new().unwrap();
        let cache_dir = dir.path().join(CACHE_DIR);
        std::fs::create_dir_all(&cache_dir).unwrap();

        let bad = serde_json::json!({
            "version": THEORY_CACHE_VERSION + 999,
            "spl_file_mtimes": {},
            "rules": [],
            "superiorities": [],
            "diagnostics": []
        });
        std::fs::write(
            cache_dir.join(THEORY_CACHE_FILE),
            serde_json::to_string(&bad).unwrap(),
        )
        .unwrap();

        let result = load_theory_cache(dir.path()).unwrap();
        assert!(result.is_none(), "version mismatch should return None");
    }

    #[test]
    fn theory_cache_valid_unchanged_files() {
        let t = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let t_secs = 1_700_000_000.0;

        let mut spl_mtimes = HashMap::new();
        spl_mtimes.insert(PathBuf::from("a.md"), t_secs);

        let cache = make_theory_cache(spl_mtimes, vec![]);
        let files = vec![make_spl_file("a.md", t)];

        assert!(theory_cache_valid(&cache, &files));
    }

    #[test]
    fn theory_cache_invalid_changed_mtime() {
        let _t1 = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let t2 = UNIX_EPOCH + Duration::from_secs(1_700_001_000);

        let mut spl_mtimes = HashMap::new();
        spl_mtimes.insert(PathBuf::from("a.md"), 1_700_000_000.0);

        let cache = make_theory_cache(spl_mtimes, vec![]);
        let files = vec![make_spl_file("a.md", t2)]; // mtime changed

        assert!(!theory_cache_valid(&cache, &files));
    }

    #[test]
    fn theory_cache_invalid_new_spl_file() {
        let t = UNIX_EPOCH + Duration::from_secs(1_700_000_000);

        let mut spl_mtimes = HashMap::new();
        spl_mtimes.insert(PathBuf::from("a.md"), 1_700_000_000.0);

        let cache = make_theory_cache(spl_mtimes, vec![]);
        // Two SPL files now, cache only knew about one
        let files = vec![
            make_spl_file("a.md", t),
            make_spl_file("b.md", t),
        ];

        assert!(!theory_cache_valid(&cache, &files));
    }

    #[test]
    fn theory_cache_invalid_removed_spl_file() {
        let t = UNIX_EPOCH + Duration::from_secs(1_700_000_000);

        let mut spl_mtimes = HashMap::new();
        spl_mtimes.insert(PathBuf::from("a.md"), 1_700_000_000.0);
        spl_mtimes.insert(PathBuf::from("b.md"), 1_700_000_000.0);

        let cache = make_theory_cache(spl_mtimes, vec![]);
        // Only one SPL file now
        let files = vec![make_spl_file("a.md", t)];

        assert!(!theory_cache_valid(&cache, &files));
    }

    #[test]
    fn theory_cache_valid_ignores_non_spl_file_changes() {
        let t = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let t2 = UNIX_EPOCH + Duration::from_secs(1_700_001_000);

        let mut spl_mtimes = HashMap::new();
        spl_mtimes.insert(PathBuf::from("a.md"), 1_700_000_000.0);

        let cache = make_theory_cache(spl_mtimes, vec![]);

        // a.md unchanged (has SPL), b.md changed but has no SPL blocks
        let files = vec![
            make_spl_file("a.md", t),
            make_parsed_file("b.md", t2), // no SPL blocks
        ];

        assert!(theory_cache_valid(&cache, &files));
    }
}
