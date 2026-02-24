use crate::merkle::{compute_spl_hashes, compute_vault_root};
use crate::types::{ContentHash, Diagnostic, ExplicitGrounding, ParsedFile};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

const CACHE_DIR: &str = ".zetl";
const CACHE_FILE: &str = "index.json";
const CACHE_VERSION: u32 = 2;

const THEORY_CACHE_FILE: &str = "theory.json";
const THEORY_CACHE_VERSION: u32 = 2;

#[derive(Debug, Serialize, Deserialize)]
struct CacheIndex {
    version: u32,
    files: HashMap<PathBuf, ParsedFile>,
    /// Vault-level Merkle root (§4.6). Stored as a 64-char lowercase hex string.
    /// `None` if the vault has no hashed files yet.
    #[serde(default)]
    vault_root_hash: Option<String>,
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

/// Save parsed files to .zetl/index.json.
///
/// Also computes and persists the vault-level Merkle root (§4.6) from the
/// per-file roots stored in each file's [`FileMerkle::root_hash`].
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
        // Compute vault root from per-file Merkle roots (sorted by canonical path, §4.6).
        vault_root_hash: vault_root_hex(files),
    };

    let json = serde_json::to_string_pretty(&index)?;
    std::fs::write(&cache_path, json)?;
    Ok(())
}

/// Load the vault-level Merkle root hex string from `.zetl/index.json`.
///
/// Returns the raw 64-character lowercase hex string, or `None` if the cache
/// does not exist, has a version mismatch, or does not contain a vault root hash.
pub fn load_vault_root_hex(vault_root: &Path) -> Result<Option<String>> {
    let cache_path = vault_root.join(CACHE_DIR).join(CACHE_FILE);
    if !cache_path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&cache_path)?;
    let index: CacheIndex = serde_json::from_str(&content)?;
    if index.version != CACHE_VERSION {
        return Ok(None);
    }
    Ok(index.vault_root_hash.filter(|h| h.len() == 64))
}

/// Load the vault-level Merkle root from `.zetl/index.json`.
///
/// Returns `None` if the cache does not exist, has a version mismatch, or
/// does not yet contain a vault root hash.
pub fn load_vault_root(vault_root: &Path) -> Result<Option<ContentHash>> {
    let cache_path = vault_root.join(CACHE_DIR).join(CACHE_FILE);
    if !cache_path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&cache_path)?;
    let index: CacheIndex = serde_json::from_str(&content)?;
    if index.version != CACHE_VERSION {
        return Ok(None);
    }
    match index.vault_root_hash {
        None => Ok(None),
        Some(hex) if hex.len() != 64 => Ok(None),
        Some(hex) => {
            let mut bytes = [0u8; 32];
            for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
                let high = nibble(chunk[0]).map_err(|e| anyhow::anyhow!(e))?;
                let low = nibble(chunk[1]).map_err(|e| anyhow::anyhow!(e))?;
                bytes[i] = (high << 4) | low;
            }
            Ok(Some(bytes))
        }
    }
}

fn nibble(c: u8) -> Result<u8, &'static str> {
    match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        b'A'..=b'F' => Ok(c - b'A' + 10),
        _ => Err("invalid hex character in vault root hash"),
    }
}

/// Format a `SystemTime` as an RFC 3339 UTC timestamp (`YYYY-MM-DDThh:mm:ssZ`).
///
/// Uses the Howard Hinnant civil calendar algorithm to convert Unix seconds to
/// a proleptic Gregorian date without requiring an external crate.
#[cfg(any(feature = "reason", test))]
fn format_rfc3339_utc(t: std::time::SystemTime) -> String {
    let secs = t.duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO).as_secs();

    let sec = (secs % 60) as u32;
    let min = ((secs / 60) % 60) as u32;
    let hour = ((secs / 3600) % 24) as u32;

    // Days since Unix epoch (1970-01-01).
    // Civil calendar algorithm: http://howardhinnant.github.io/date_algorithms.html
    let z = (secs / 86400) as i64 + 719_468;
    let era: i64 = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hour, min, sec
    )
}

/// Compute the vault-level Merkle root from a slice of parsed files and
/// return it as a 64-character lowercase hex string, or `None` if no file
/// has a Merkle root.
fn vault_root_hex(files: &[ParsedFile]) -> Option<String> {
    let pairs: Vec<(&Path, ContentHash)> = files
        .iter()
        .filter_map(|f| f.file_merkle.as_ref().map(|fm| (f.path.as_path(), fm.root_hash)))
        .collect();
    if pairs.is_empty() {
        None
    } else {
        let h = compute_vault_root(&pairs);
        Some(h.iter().map(|b| format!("{:02x}", b)).collect())
    }
}

/// Determine which files need re-parsing based on mtime changes or missing Merkle roots.
///
/// A `.md` file is flagged for reparse when either:
/// - its mtime has changed since the cache was written, or
/// - its cached entry has no [`FileMerkle`] root (i.e. was stored without
///   Merkle data, which must be backfilled).
pub fn files_needing_reparse(
    cached: &HashMap<PathBuf, ParsedFile>,
    current_files: &[(PathBuf, std::time::SystemTime)],
) -> Vec<PathBuf> {
    current_files
        .iter()
        .filter(|(path, mtime)| match cached.get(path) {
            None => true,
            Some(cached_file) => {
                let mtime_changed = cached_file.mtime != *mtime;
                let missing_merkle = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map_or(false, |ext| ext == "md")
                    && cached_file.file_merkle.is_none();
                mtime_changed || missing_merkle
            }
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

/// Cached Merkle data for a single SPL block (§12.2).
///
/// Stored in [`TheoryCache::spl_blocks`] keyed by `"path:line"` where
/// `path` is the relative path from vault root and `line` is the
/// 1-indexed start line of the SPL block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplBlockCache {
    /// AST-level BLAKE3 hash (§4.4).
    #[serde(with = "crate::types::content_hash_serde")]
    pub ast_hash: ContentHash,
    /// Normalised-content BLAKE3 hash (§4.4).
    #[serde(with = "crate::types::content_hash_serde")]
    pub content_hash: ContentHash,
    /// Heading text of the grounding section, if any.
    pub section_heading: Option<String>,
    /// Grounding hash of the enclosing section (§4.5).
    #[serde(with = "crate::types::content_hash_serde")]
    pub section_grounding_hash: ContentHash,
    /// Explicit grounding declarations attached to this block.
    pub explicit_groundings: Vec<ExplicitGrounding>,
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
    /// Vault-level Merkle root at build time (§12). Stored as a 64-char hex string.
    /// `None` if no files were hashed.
    #[serde(default)]
    pub vault_root_hash: Option<String>,
    /// RFC 3339 UTC timestamp when this cache was built.
    #[serde(default)]
    pub built_at: Option<String>,
    /// Per-SPL-block Merkle cache keyed by `"path:line"` (§12.2).
    #[serde(default)]
    pub spl_blocks: HashMap<String, SplBlockCache>,
    /// VCS commit hash at build time (§1.6). `None` outside a git repo.
    #[serde(default)]
    pub git_commit: Option<String>,
    /// VCS dirty flag at build time (§1.6). `None` outside a git repo.
    #[serde(default)]
    pub git_dirty: Option<bool>,
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

/// Collect the current set of SPL leaf AST hashes from all parsed files (REQ-040).
///
/// Returns a map keyed by `"<path>:<start_line>"` to the AST-level BLAKE3 hash.
///
/// For Markdown files with a populated [`FileMerkle`], hashes come from
/// the pre-computed [`SplLeafCached::ast_hash`] entries.
///
/// For standalone `.spl` files (`file_merkle` is `None`), hashes are computed
/// on-the-fly from [`SplBlock::content`] so that mtime-triggered re-parses
/// correctly invalidate the theory cache.
pub fn collect_spl_ast_hashes(files: &[ParsedFile]) -> HashMap<String, ContentHash> {
    let mut map = HashMap::new();
    for f in files {
        if let Some(fm) = &f.file_merkle {
            for leaf in &fm.spl_leaves {
                let key = format!("{}:{}", f.path.display(), leaf.start_line);
                map.insert(key, leaf.ast_hash);
            }
        } else {
            // Standalone .spl files: no FileMerkle; compute hash from content.
            for block in &f.spl_blocks {
                let key = format!("{}:{}", f.path.display(), block.start_line);
                let hashes = compute_spl_hashes(&block.content);
                map.insert(key, hashes.ast_hash);
            }
        }
    }
    map
}

/// Check whether the theory cache is still valid by comparing SPL AST hashes (REQ-040).
///
/// The cache is valid if and only if:
/// 1. The set of SPL block keys (`"<path>:<start_line>"`) is unchanged.
/// 2. Every SPL block has the same AST-level hash as when the cache was built.
///
/// A prose-only edit (file Merkle root changes, but SPL AST hash unchanged)
/// does NOT invalidate the cache.
pub fn theory_cache_valid(
    current_spl_hashes: &HashMap<String, ContentHash>,
    cached_theory: &TheoryCache,
) -> bool {
    // Same number of SPL blocks?
    if current_spl_hashes.len() != cached_theory.spl_blocks.len() {
        return false;
    }

    // Every cached block still present with same AST hash?
    for (key, cached_block) in &cached_theory.spl_blocks {
        match current_spl_hashes.get(key) {
            Some(&current_hash) if current_hash == cached_block.ast_hash => {}
            _ => return false,
        }
    }

    true
}

/// Build a `TheoryCache` from the current pipeline state and a built theory.
///
/// Called after `build_theory()` succeeds to persist the parsed theory for
/// subsequent queries.
///
/// `groundings_by_block` contains the explicit source groundings extracted from
/// `(meta LABEL (source ...))` forms during theory construction (REQ-042).
/// It is keyed by `"<path>:<start_line>"` matching each SPL block.
#[cfg(feature = "reason")]
pub fn build_theory_cache(
    theory: &spindle_core::prelude::Theory,
    diagnostics: &[Diagnostic],
    files: &[ParsedFile],
    groundings_by_block: &HashMap<String, Vec<ExplicitGrounding>>,
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
        vault_root_hash: vault_root_hex(files),
        built_at: Some(format_rfc3339_utc(std::time::SystemTime::now())),
        spl_blocks: build_spl_block_cache(files, groundings_by_block),
        git_commit: None,
        git_dirty: None,
    }
}

/// Build the per-SPL-block cache from the current set of parsed files (§12.2).
///
/// Each entry is keyed by `"<path>:<start_line>"` and stores the Merkle hashes
/// and grounding metadata for a single SPL block.
///
/// Markdown files use [`FileMerkle`] data to populate section grounding fields.
/// Standalone `.spl` files produce entries with `section_heading: None` and
/// `section_grounding_hash: [0u8; 32]` since section grounding does not apply
/// to them (§4.7). Their entries are still required for theory cache invalidation.
///
/// `groundings_by_block` provides explicit source groundings extracted during
/// theory construction (REQ-042), keyed by `"<path>:<start_line>"`.
#[cfg(feature = "reason")]
fn build_spl_block_cache(
    files: &[ParsedFile],
    groundings_by_block: &HashMap<String, Vec<ExplicitGrounding>>,
) -> HashMap<String, SplBlockCache> {
    let mut result = HashMap::new();
    for f in files {
        if let Some(fm) = &f.file_merkle {
            // Markdown files: populate section grounding from FileMerkle.
            for spl_leaf in &fm.spl_leaves {
                let key = format!("{}:{}", f.path.display(), spl_leaf.start_line);
                let section = fm.sections.get(spl_leaf.section_index);
                let section_heading = section.and_then(|s| {
                    if s.heading_text.is_empty() {
                        None
                    } else {
                        Some(s.heading_text.clone())
                    }
                });
                let section_grounding_hash =
                    section.map(|s| s.grounding_hash).unwrap_or([0u8; 32]);
                // Prefer freshly-extracted groundings; fall back to cached leaf data.
                let explicit_groundings = groundings_by_block
                    .get(&key)
                    .cloned()
                    .unwrap_or_else(|| spl_leaf.explicit_groundings.clone());
                result.insert(
                    key,
                    SplBlockCache {
                        ast_hash: spl_leaf.ast_hash,
                        content_hash: spl_leaf.content_hash,
                        section_heading,
                        section_grounding_hash,
                        explicit_groundings,
                    },
                );
            }
        } else {
            // Standalone .spl files: no section grounding (§4.7), but still
            // tracked for theory cache invalidation.
            for block in &f.spl_blocks {
                let key = format!("{}:{}", f.path.display(), block.start_line);
                let hashes = compute_spl_hashes(&block.content);
                let explicit_groundings =
                    groundings_by_block.get(&key).cloned().unwrap_or_default();
                result.insert(
                    key,
                    SplBlockCache {
                        ast_hash: hashes.ast_hash,
                        content_hash: hashes.content_hash,
                        section_heading: None,
                        section_grounding_hash: [0u8; 32],
                        explicit_groundings,
                    },
                );
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FileMerkle, ParsedFile, SplLeafCached};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use tempfile::TempDir;

    /// Helper: build a minimal ParsedFile with the given relative path and mtime.
    fn make_parsed_file(rel_path: &str, mtime: SystemTime) -> ParsedFile {
        ParsedFile {
            path: PathBuf::from(rel_path),
            page_name: rel_path.strip_suffix(".md").unwrap_or(rel_path).to_string(),
            links: vec![],
            spl_blocks: vec![],
            diagnostics: vec![],
            mtime,
            merkle_leaves: vec![],
            file_merkle: None,
        }
    }

    /// Helper: build a ParsedFile with a populated (non-empty) `file_merkle`.
    fn make_parsed_file_with_merkle(rel_path: &str, mtime: SystemTime) -> ParsedFile {
        let mut f = make_parsed_file(rel_path, mtime);
        f.file_merkle = Some(FileMerkle {
            root_hash: [1u8; 32],
            sections: vec![],
            spl_leaves: vec![],
        });
        f
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

        // a.md: same mtime AND has file_merkle → should NOT need reparse.
        // b.md: changed mtime → should need reparse.
        // c.md: new file → should need reparse.
        let cached: HashMap<PathBuf, ParsedFile> = [
            (
                PathBuf::from("a.md"),
                make_parsed_file_with_merkle("a.md", t1),
            ),
            (PathBuf::from("b.md"), make_parsed_file("b.md", t1)),
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

        assert_eq!(
            need_reparse,
            vec![PathBuf::from("b.md"), PathBuf::from("c.md")]
        );
    }

    #[test]
    fn files_needing_reparse_empty_cache_returns_all() {
        let cached: HashMap<PathBuf, ParsedFile> = HashMap::new();
        let t = UNIX_EPOCH + Duration::from_secs(1_000_000);

        let current_files = vec![(PathBuf::from("x.md"), t), (PathBuf::from("y.md"), t)];

        let need_reparse = files_needing_reparse(&cached, &current_files);
        assert_eq!(
            need_reparse.len(),
            2,
            "all files should need parsing when cache is empty"
        );
    }

    #[test]
    fn files_needing_reparse_unchanged_returns_empty() {
        let t = UNIX_EPOCH + Duration::from_secs(1_500_000);

        // Use a file with a populated file_merkle so mtime match + merkle present → no reparse.
        let cached: HashMap<PathBuf, ParsedFile> = [(
            PathBuf::from("only.md"),
            make_parsed_file_with_merkle("only.md", t),
        )]
        .into_iter()
        .collect();

        let current_files = vec![(PathBuf::from("only.md"), t)];

        let need_reparse = files_needing_reparse(&cached, &current_files);
        assert!(
            need_reparse.is_empty(),
            "unchanged .md file with merkle should not need reparse"
        );
    }

    #[test]
    fn files_needing_reparse_md_missing_merkle_returns_file() {
        let t = UNIX_EPOCH + Duration::from_secs(1_500_000);

        // Same mtime but file_merkle: None → needs reparse for .md files.
        let cached: HashMap<PathBuf, ParsedFile> =
            [(PathBuf::from("note.md"), make_parsed_file("note.md", t))]
                .into_iter()
                .collect();

        let current_files = vec![(PathBuf::from("note.md"), t)];

        let need_reparse = files_needing_reparse(&cached, &current_files);
        assert_eq!(
            need_reparse,
            vec![PathBuf::from("note.md")],
            ".md file missing file_merkle should need reparse even if mtime unchanged"
        );
    }

    #[test]
    fn files_needing_reparse_spl_missing_merkle_unchanged_ok() {
        let t = UNIX_EPOCH + Duration::from_secs(1_500_000);

        // .spl files with file_merkle: None and unchanged mtime should NOT trigger reparse.
        let cached: HashMap<PathBuf, ParsedFile> = [(
            PathBuf::from("theory.spl"),
            make_parsed_file("theory.spl", t),
        )]
        .into_iter()
        .collect();

        let current_files = vec![(PathBuf::from("theory.spl"), t)];

        let need_reparse = files_needing_reparse(&cached, &current_files);
        assert!(
            need_reparse.is_empty(),
            ".spl file without merkle and unchanged mtime should not need reparse"
        );
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
            vault_root_hash: None,
            built_at: None,
            spl_blocks: HashMap::new(),
            git_commit: None,
            git_dirty: None,
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

    // ── Helpers for theory_cache_valid / collect_spl_ast_hashes tests ──

    /// Build a current SPL hashes map from (key, hash) pairs.
    fn make_spl_hashes(entries: &[(&str, ContentHash)]) -> HashMap<String, ContentHash> {
        entries.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    /// Build a minimal SplBlockCache with the given AST hash.
    fn make_cached_spl_block(ast_hash: ContentHash) -> SplBlockCache {
        SplBlockCache {
            ast_hash,
            content_hash: [0u8; 32],
            section_heading: None,
            section_grounding_hash: [0u8; 32],
            explicit_groundings: vec![],
        }
    }

    /// Build a TheoryCache with the given spl_blocks and no other data.
    fn make_theory_cache_spl(spl_blocks: HashMap<String, SplBlockCache>) -> TheoryCache {
        TheoryCache {
            version: THEORY_CACHE_VERSION,
            spl_file_mtimes: HashMap::new(),
            rules: vec![],
            superiorities: vec![],
            diagnostics: vec![],
            vault_root_hash: None,
            built_at: None,
            spl_blocks,
            git_commit: None,
            git_dirty: None,
        }
    }

    #[test]
    fn theory_cache_valid_same_ast_hashes() {
        let hash: ContentHash = [0x01u8; 32];
        let current = make_spl_hashes(&[("a.md:5", hash)]);
        let mut spl_blocks = HashMap::new();
        spl_blocks.insert("a.md:5".to_string(), make_cached_spl_block(hash));
        assert!(theory_cache_valid(&current, &make_theory_cache_spl(spl_blocks)));
    }

    #[test]
    fn theory_cache_invalid_changed_ast_hash() {
        let hash_old: ContentHash = [0x01u8; 32];
        let hash_new: ContentHash = [0x02u8; 32];
        let current = make_spl_hashes(&[("a.md:5", hash_new)]); // AST changed
        let mut spl_blocks = HashMap::new();
        spl_blocks.insert("a.md:5".to_string(), make_cached_spl_block(hash_old));
        assert!(!theory_cache_valid(&current, &make_theory_cache_spl(spl_blocks)));
    }

    #[test]
    fn theory_cache_invalid_new_spl_block() {
        let hash: ContentHash = [0x01u8; 32];
        // current has two blocks; cache only knew about one
        let current = make_spl_hashes(&[("a.md:5", hash), ("b.md:10", hash)]);
        let mut spl_blocks = HashMap::new();
        spl_blocks.insert("a.md:5".to_string(), make_cached_spl_block(hash));
        assert!(!theory_cache_valid(&current, &make_theory_cache_spl(spl_blocks)));
    }

    #[test]
    fn theory_cache_invalid_removed_spl_block() {
        let hash: ContentHash = [0x01u8; 32];
        // current has one block; cache had two
        let current = make_spl_hashes(&[("a.md:5", hash)]);
        let mut spl_blocks = HashMap::new();
        spl_blocks.insert("a.md:5".to_string(), make_cached_spl_block(hash));
        spl_blocks.insert("b.md:10".to_string(), make_cached_spl_block(hash));
        assert!(!theory_cache_valid(&current, &make_theory_cache_spl(spl_blocks)));
    }

    #[test]
    fn theory_cache_v2_fields_round_trip() {
        let dir = TempDir::new().unwrap();
        let vault = dir.path();

        let mut mtimes = HashMap::new();
        mtimes.insert(PathBuf::from("a.md"), 1_700_000_000.0);

        let mut spl_blocks = HashMap::new();
        spl_blocks.insert(
            "a.md:5".to_string(),
            SplBlockCache {
                ast_hash: [0xabu8; 32],
                content_hash: [0xcdu8; 32],
                section_heading: Some("Background".to_string()),
                section_grounding_hash: [0xefu8; 32],
                explicit_groundings: vec![],
            },
        );

        let cache = TheoryCache {
            version: THEORY_CACHE_VERSION,
            spl_file_mtimes: mtimes,
            rules: vec![],
            superiorities: vec![],
            diagnostics: vec![],
            vault_root_hash: Some("ab".repeat(32)),
            built_at: Some("2024-01-15T12:34:56Z".to_string()),
            spl_blocks,
            git_commit: Some("abc123".to_string()),
            git_dirty: Some(false),
        };

        save_theory_cache(vault, &cache).unwrap();
        let loaded = load_theory_cache(vault)
            .unwrap()
            .expect("theory cache should exist");

        assert_eq!(loaded.vault_root_hash, Some("ab".repeat(32)));
        assert_eq!(loaded.built_at, Some("2024-01-15T12:34:56Z".to_string()));
        assert_eq!(loaded.git_commit, Some("abc123".to_string()));
        assert_eq!(loaded.git_dirty, Some(false));

        let block = loaded.spl_blocks.get("a.md:5").expect("spl block should exist");
        assert_eq!(block.ast_hash, [0xabu8; 32]);
        assert_eq!(block.content_hash, [0xcdu8; 32]);
        assert_eq!(block.section_heading, Some("Background".to_string()));
        assert_eq!(block.section_grounding_hash, [0xefu8; 32]);
    }

    #[test]
    fn format_rfc3339_utc_epoch() {
        let t = UNIX_EPOCH;
        assert_eq!(format_rfc3339_utc(t), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn format_rfc3339_utc_known_date() {
        // 2024-01-15T00:00:00Z = 1705276800 seconds since epoch
        let t = UNIX_EPOCH + Duration::from_secs(1_705_276_800);
        assert_eq!(format_rfc3339_utc(t), "2024-01-15T00:00:00Z");
    }

    #[test]
    fn theory_cache_v2_missing_optional_fields_deserialise_ok() {
        // A v2 cache written without the optional fields should still load.
        let dir = TempDir::new().unwrap();
        let cache_dir = dir.path().join(CACHE_DIR);
        std::fs::create_dir_all(&cache_dir).unwrap();

        let json = serde_json::json!({
            "version": THEORY_CACHE_VERSION,
            "spl_file_mtimes": {},
            "rules": [],
            "superiorities": [],
            "diagnostics": []
            // vault_root_hash, built_at, spl_blocks, git_commit, git_dirty absent
        });
        std::fs::write(
            cache_dir.join(THEORY_CACHE_FILE),
            serde_json::to_string(&json).unwrap(),
        )
        .unwrap();

        let loaded = load_theory_cache(dir.path())
            .unwrap()
            .expect("should load despite missing optional fields");
        assert!(loaded.vault_root_hash.is_none());
        assert!(loaded.built_at.is_none());
        assert!(loaded.spl_blocks.is_empty());
        assert!(loaded.git_commit.is_none());
        assert!(loaded.git_dirty.is_none());
    }

    #[test]
    fn theory_cache_valid_ignores_non_spl_file_changes() {
        // b.md has no SPL leaves → its changes don't appear in current_spl_hashes
        let hash: ContentHash = [0x01u8; 32];
        let current = make_spl_hashes(&[("a.md:5", hash)]); // only a.md's block
        let mut spl_blocks = HashMap::new();
        spl_blocks.insert("a.md:5".to_string(), make_cached_spl_block(hash));
        assert!(theory_cache_valid(&current, &make_theory_cache_spl(spl_blocks)));
    }

    #[test]
    fn theory_cache_valid_prose_only_edit_does_not_invalidate() {
        // Prose edit: file mtime and Merkle root change, but SPL AST hash stays same.
        // Only the AST hash matters — the cache should remain valid.
        let hash: ContentHash = [0xabu8; 32];
        let current = make_spl_hashes(&[("notes/theory.md:12", hash)]);
        let mut spl_blocks = HashMap::new();
        spl_blocks.insert(
            "notes/theory.md:12".to_string(),
            make_cached_spl_block(hash), // same ast_hash as current
        );
        assert!(theory_cache_valid(&current, &make_theory_cache_spl(spl_blocks)));
    }

    #[test]
    fn theory_cache_valid_empty_vault_no_spl() {
        let current: HashMap<String, ContentHash> = HashMap::new();
        assert!(theory_cache_valid(&current, &make_theory_cache_spl(HashMap::new())));
    }

    // ── collect_spl_ast_hashes tests ──────────────────────────────────────

    #[test]
    fn collect_spl_ast_hashes_from_file_with_merkle() {
        let hash: ContentHash = [0xabu8; 32];
        let t = UNIX_EPOCH;
        let mut f = make_parsed_file("notes/theory.md", t);
        f.file_merkle = Some(FileMerkle {
            root_hash: [0u8; 32],
            sections: vec![],
            spl_leaves: vec![SplLeafCached {
                start_line: 12,
                content_hash: [0u8; 32],
                ast_hash: hash,
                section_index: 0,
                explicit_groundings: vec![],
            }],
        });
        let result = collect_spl_ast_hashes(&[f]);
        assert_eq!(result.len(), 1);
        assert_eq!(result.get("notes/theory.md:12"), Some(&hash));
    }

    #[test]
    fn collect_spl_ast_hashes_skips_file_without_merkle() {
        let t = UNIX_EPOCH;
        let f = make_parsed_file("a.md", t); // file_merkle: None
        let result = collect_spl_ast_hashes(&[f]);
        assert!(result.is_empty());
    }

    #[test]
    fn collect_spl_ast_hashes_multiple_files_and_blocks() {
        let hash1: ContentHash = [0x01u8; 32];
        let hash2: ContentHash = [0x02u8; 32];
        let hash3: ContentHash = [0x03u8; 32];
        let t = UNIX_EPOCH;

        let mut f1 = make_parsed_file("a.md", t);
        f1.file_merkle = Some(FileMerkle {
            root_hash: [0u8; 32],
            sections: vec![],
            spl_leaves: vec![
                SplLeafCached {
                    start_line: 1,
                    content_hash: [0u8; 32],
                    ast_hash: hash1,
                    section_index: 0,
                    explicit_groundings: vec![],
                },
                SplLeafCached {
                    start_line: 20,
                    content_hash: [0u8; 32],
                    ast_hash: hash2,
                    section_index: 0,
                    explicit_groundings: vec![],
                },
            ],
        });

        let mut f2 = make_parsed_file("b.md", t);
        f2.file_merkle = Some(FileMerkle {
            root_hash: [0u8; 32],
            sections: vec![],
            spl_leaves: vec![SplLeafCached {
                start_line: 5,
                content_hash: [0u8; 32],
                ast_hash: hash3,
                section_index: 0,
                explicit_groundings: vec![],
            }],
        });

        let result = collect_spl_ast_hashes(&[f1, f2]);
        assert_eq!(result.len(), 3);
        assert_eq!(result.get("a.md:1"), Some(&hash1));
        assert_eq!(result.get("a.md:20"), Some(&hash2));
        assert_eq!(result.get("b.md:5"), Some(&hash3));
    }

    // ── TEST-042: Implicit Section Grounding ──────────────────────────────────

    /// TEST-042 scenario: section grounding hash and heading are copied from FileMerkle
    /// into the theory cache spl_blocks (REQ-041c).
    #[cfg(feature = "reason")]
    #[test]
    fn build_theory_cache_section_grounding_populated_from_file_merkle() {
        use crate::types::{FileMerkle, Section, SplLeafCached};
        use spindle_core::prelude::Theory;
        use std::time::UNIX_EPOCH;

        let grounding_hash: ContentHash = [0xabu8; 32];
        let ast_hash: ContentHash = [0x22u8; 32];
        let content_hash: ContentHash = [0x11u8; 32];

        // A file with one section ("Background") containing one SPL block.
        let section = Section {
            heading_line: 2,
            heading_text: "Background".to_string(),
            heading_level: 2,
            leaf_range: (0, 1),
            grounding_hash,
        };
        let spl_leaf = SplLeafCached {
            start_line: 5,
            content_hash,
            ast_hash,
            section_index: 0,
            explicit_groundings: vec![],
        };
        let mut f = make_parsed_file("notes/theory.md", UNIX_EPOCH);
        f.file_merkle = Some(FileMerkle {
            root_hash: [0u8; 32],
            sections: vec![section],
            spl_leaves: vec![spl_leaf],
        });

        let theory = Theory::new();
        let cache = build_theory_cache(&theory, &[], &[f], &HashMap::new());

        let block = cache
            .spl_blocks
            .get("notes/theory.md:5")
            .expect("spl_blocks entry should exist for notes/theory.md:5");
        assert_eq!(
            block.section_heading,
            Some("Background".to_string()),
            "section_heading should be populated from Section::heading_text"
        );
        assert_eq!(
            block.section_grounding_hash, grounding_hash,
            "section_grounding_hash should match Section::grounding_hash"
        );
        assert_eq!(block.ast_hash, ast_hash);
        assert_eq!(block.content_hash, content_hash);
    }

    /// TEST-042 scenario: SPL block in preamble (before first heading) has null
    /// section_heading and a non-zero grounding_hash from preamble prose.
    #[cfg(feature = "reason")]
    #[test]
    fn build_theory_cache_preamble_section_has_null_heading() {
        use crate::types::{FileMerkle, Section, SplLeafCached};
        use spindle_core::prelude::Theory;
        use std::time::UNIX_EPOCH;

        let preamble_grounding_hash: ContentHash = [0xffu8; 32];
        // Preamble section: heading_level == 0, heading_text == "" (empty = None in spl_blocks).
        let preamble_section = Section {
            heading_line: 0,
            heading_text: String::new(),
            heading_level: 0,
            leaf_range: (0, 1),
            grounding_hash: preamble_grounding_hash,
        };
        let spl_leaf = SplLeafCached {
            start_line: 3,
            content_hash: [0x01u8; 32],
            ast_hash: [0x02u8; 32],
            section_index: 0,
            explicit_groundings: vec![],
        };
        let mut f = make_parsed_file("notes/intro.md", UNIX_EPOCH);
        f.file_merkle = Some(FileMerkle {
            root_hash: [0u8; 32],
            sections: vec![preamble_section],
            spl_leaves: vec![spl_leaf],
        });

        let theory = Theory::new();
        let cache = build_theory_cache(&theory, &[], &[f], &HashMap::new());

        let block = cache
            .spl_blocks
            .get("notes/intro.md:3")
            .expect("spl_blocks entry should exist");
        assert_eq!(
            block.section_heading, None,
            "empty heading_text should yield None section_heading"
        );
        assert_eq!(
            block.section_grounding_hash, preamble_grounding_hash,
            "preamble grounding hash should still be stored"
        );
    }

    /// TEST-042 scenario: standalone .spl files produce spl_blocks entries with
    /// null section fields since section grounding does not apply (§4.7), but
    /// they are still tracked for theory cache invalidation.
    #[cfg(feature = "reason")]
    #[test]
    fn build_theory_cache_standalone_spl_null_section_fields() {
        use crate::types::SplBlock;
        use spindle_core::prelude::Theory;
        use std::time::UNIX_EPOCH;

        let mut f = make_parsed_file("theories/caching.spl", UNIX_EPOCH);
        f.spl_blocks = vec![SplBlock {
            source_file: "theories/caching.spl".into(),
            source_page: "theories/caching".to_string(),
            start_line: 1,
            end_line: 3,
            content: "(given x)\n".to_string(),
        }];
        f.file_merkle = None; // standalone .spl has no Merkle tree

        let theory = Theory::new();
        let cache = build_theory_cache(&theory, &[], &[f], &HashMap::new());

        // Standalone .spl files appear in spl_blocks (tracked for cache invalidation)
        // but with null section fields (§4.7).
        let block = cache
            .spl_blocks
            .get("theories/caching.spl:1")
            .expect("standalone .spl should have an spl_blocks entry for cache tracking");
        assert_eq!(
            block.section_heading, None,
            "standalone .spl has no section heading"
        );
        assert_eq!(
            block.section_grounding_hash,
            [0u8; 32],
            "standalone .spl has no section grounding hash"
        );
    }
}
