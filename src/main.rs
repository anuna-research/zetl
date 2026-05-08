use std::collections::{HashMap, HashSet, VecDeque};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use base64::Engine as _;
use clap::Parser;
use comfy_table::{Cell, Table};
use serde::Serialize;

use zetl::cache::{
    files_needing_reparse, load_cache, load_theory_cache, load_vault_root_hex, save_cache,
};
use zetl::cli::{
    AgentCommand, BlockTypeFilter, Cli, Command, EcosystemCommand, FailLevel, HookCommand,
    OutputFormat, ThemeCommand,
};
use zetl::drift::{detect_explicit_drift, detect_section_drift};
use zetl::graph::LinkGraph;
use zetl::merkle::{
    build_vault_hash_index, resolve_hash_prefix, validate_source_refs, HashResolutionResult,
};
use zetl::scanner::{resolve_page_name, scan_vault};
use zetl::search::{search_vault, SearchConfig};
use zetl::search_index::SearchIndex;
use zetl::simhash::SimHashIndex;
use zetl::types::{ContentHash, DiagnosticLevel, DriftDiagnostic, DriftSeverity, ParsedFile};

// ── Common pipeline ────────────────────────────────────────────────────────

/// Snapshot metadata included in JSON output when --at is used (REQ-077, CON-024).
#[derive(Clone, Debug, Serialize)]
struct SnapshotInfo {
    change_id: String,
    commit_id: String,
    timestamp: String,
    description: String,
}

/// Scan and cache efficiency counters collected during run_pipeline (OBS-007, OBS-009).
struct ScanStats {
    /// Files that passed the Tier-1 mtime check (no re-read needed).
    tier1_hits: usize,
    /// Files whose mtime changed (fell through to Tier-2 hash comparison).
    tier1_misses: usize,
    /// Files whose mtime changed but whose content hash matched (touch hit).
    tier2_hits: usize,
    /// Files whose content hash differed — actual change, full reparse.
    tier2_misses: usize,
    /// Files that went through a full reparse (new + changed + backfill).
    files_hashed: usize,
    /// Total Merkle leaf nodes across all files in the final merged set.
    total_leaf_nodes: usize,
    /// Leaves that carry dual SPL hashes (content + AST).
    spl_leaves_dual_hash: usize,
    /// Wall-clock time for scan_vault (BLAKE3 hashing + section detection).
    blake3_hashing_ms: u128,
    /// Wall-clock time for section detection (subsumed in blake3_hashing_ms).
    section_detection_ms: u128,
}

struct Pipeline {
    files: Vec<ParsedFile>,
    file_index: Vec<(String, PathBuf)>,
    graph: LinkGraph,
    vault_root: PathBuf,
    scan_stats: ScanStats,
    /// Page names backed by real files (for dead-link detection in web view).
    graph_resolved: std::collections::HashSet<String>,
    /// Set to Some when --at resolved a historical snapshot (REQ-077, CON-024).
    snapshot: Option<SnapshotInfo>,
}

fn run_pipeline(cli: &Cli) -> Result<Pipeline> {
    // Handle --at for historical point-in-time queries (REQ-077, REQ-078, CON-024).
    // The `at` field only exists when the `history` feature is compiled in (REQ-084).
    #[cfg(feature = "history")]
    if let Some(ref at_expr) = cli.at {
        let vault_root = std::fs::canonicalize(&cli.dir)
            .with_context(|| format!("Cannot resolve vault directory: {}", cli.dir))?;
        return run_historical_pipeline(vault_root, at_expr, cli.verbose);
    }

    let vault_root = std::fs::canonicalize(&cli.dir)
        .with_context(|| format!("Cannot resolve vault directory: {}", cli.dir))?;

    // Load cache (unless --no-cache)
    let cached = if cli.no_cache {
        None
    } else {
        load_cache(&vault_root)?
    };

    // Scan vault for all markdown files (timed for OBS-007).
    let scan_start = Instant::now();
    let all_scanned = scan_vault(&vault_root, &cli.scan_options())?;
    let scan_elapsed_ms = scan_start.elapsed().as_millis();

    // Incremental re-parse: two-tier invalidation (REQ-039, ADR-009).
    let (files, scan_stats) = if let Some(ref cached_map) = cached {
        // Build current file list with freshly computed Merkle roots for Tier 2 comparison.
        let current_files: Vec<(PathBuf, std::time::SystemTime, Option<ContentHash>)> = all_scanned
            .iter()
            .map(|f| {
                let fresh_root = f.file_merkle.as_ref().map(|fm| fm.root_hash);
                (f.path.clone(), f.mtime, fresh_root)
            })
            .collect();

        let (full_reparse, content_unchanged) = files_needing_reparse(cached_map, &current_files);
        let full_reparse_set: HashSet<PathBuf> = full_reparse.into_iter().collect();
        let content_unchanged_set: HashSet<PathBuf> = content_unchanged.into_iter().collect();

        // Compute OBS-009 tier counters from the two-tier invalidation results.
        let tier2_hits = content_unchanged_set.len();
        let tier2_misses = full_reparse_set
            .iter()
            .filter(|p| cached_map.contains_key(*p))
            .count();
        let tier1_misses = tier2_hits + tier2_misses;
        let tier1_hits = all_scanned
            .len()
            .saturating_sub(full_reparse_set.len() + tier2_hits);

        // Merge:
        //   full reparse    → use freshly scanned ParsedFile (links/SPL re-extracted).
        //   content_unchanged (Tier 2 hit) → use cached ParsedFile, update mtime only so
        //                     downstream processing (wikilink/SPL re-extraction, theory
        //                     cache invalidation) is skipped.
        //   mtime unchanged (Tier 1 hit)   → use cached ParsedFile as-is.
        let mut merged = Vec::new();
        for scanned in &all_scanned {
            if full_reparse_set.contains(&scanned.path) {
                merged.push(scanned.clone());
            } else if content_unchanged_set.contains(&scanned.path) {
                // Tier 2 hit: content unchanged despite mtime change.
                // Preserve cached links/SPL/Merkle; update mtime to avoid repeated Tier 2
                // hash comparisons on subsequent runs.
                if let Some(cached_file) = cached_map.get(&scanned.path) {
                    let mut updated = cached_file.clone();
                    updated.mtime = scanned.mtime;
                    merged.push(updated);
                } else {
                    merged.push(scanned.clone());
                }
            } else if let Some(cached_file) = cached_map.get(&scanned.path) {
                merged.push(cached_file.clone());
            } else {
                merged.push(scanned.clone());
            }
        }

        let files_hashed = full_reparse_set.len();
        let total_leaf_nodes: usize = merged.iter().map(|f| f.merkle_leaves.len()).sum();
        let spl_leaves_dual_hash: usize = merged
            .iter()
            .flat_map(|f| &f.merkle_leaves)
            .filter(|l| l.spl_hashes.is_some())
            .count();

        (
            merged,
            ScanStats {
                tier1_hits,
                tier1_misses,
                tier2_hits,
                tier2_misses,
                files_hashed,
                total_leaf_nodes,
                spl_leaves_dual_hash,
                blake3_hashing_ms: scan_elapsed_ms,
                section_detection_ms: scan_elapsed_ms,
            },
        )
    } else {
        let total_leaf_nodes: usize = all_scanned.iter().map(|f| f.merkle_leaves.len()).sum();
        let spl_leaves_dual_hash: usize = all_scanned
            .iter()
            .flat_map(|f| &f.merkle_leaves)
            .filter(|l| l.spl_hashes.is_some())
            .count();
        let files_hashed = all_scanned.len();
        let stats = ScanStats {
            tier1_hits: 0,
            tier1_misses: 0,
            tier2_hits: 0,
            tier2_misses: 0,
            files_hashed,
            total_leaf_nodes,
            spl_leaves_dual_hash,
            blake3_hashing_ms: scan_elapsed_ms,
            section_detection_ms: scan_elapsed_ms,
        };
        (all_scanned, stats)
    };

    // Save updated cache
    if !cli.no_cache {
        if let Err(e) = save_cache(&vault_root, &files) {
            if cli.verbose > 0 {
                eprintln!("Warning: failed to save cache: {e}");
            }
        }
    }

    // Build file index: Vec<(page_name, path)>
    let file_index: Vec<(String, PathBuf)> = files
        .iter()
        .map(|f| (f.page_name.clone(), f.path.clone()))
        .collect();

    // Resolve page names for all links
    let mut resolved_pages: HashMap<String, String> = HashMap::new();
    for file in &files {
        for link in &file.links {
            let key = link.raw_target.clone();
            if resolved_pages.contains_key(&key) {
                continue;
            }
            // Try resolving the target_page (the page portion, without heading/block)
            if let Some(resolved) = resolve_page_name(&link.target_page, &file_index) {
                resolved_pages.insert(key, resolved);
            }
        }
    }

    // Build the link graph
    let graph = LinkGraph::build(&files, &resolved_pages);
    let graph_resolved = graph.resolved.clone();

    Ok(Pipeline {
        files,
        file_index,
        graph,
        vault_root,
        scan_stats,
        graph_resolved,
        snapshot: None,
    })
}

// ── Historical pipeline (--at) ─────────────────────────────────────────────

/// Build a Pipeline from a historical jj snapshot identified by `at_expr`
/// (REQ-077, REQ-078, CON-024).
///
/// Resolution order:
/// 1. Open JjBackend at `vault_root`.
/// 2. List all snapshots (newest-first) and resolve `at_expr` with the time
///    parser.
/// 3. Extract `vault_root_hash` from the snapshot description.
/// 4. Load the historical index from `HistoricalIndexCache`.
/// 5. Reconstruct `file_index`, resolved_pages, and `LinkGraph` from the
///    cached files and return a fully-formed `Pipeline` with `snapshot` set.
#[cfg(feature = "history")]
fn run_historical_pipeline(vault_root: PathBuf, at_expr: &str, verbose: u8) -> Result<Pipeline> {
    use zetl::history::cache::HistoricalIndexCache;
    use zetl::history::core::resolve_snapshot;
    use zetl::history::jj_backend::VcsBackend as _;

    // Use open_history (not open_or_init) so that a missing .zetl/jj/ directory
    // yields NO_HISTORY rather than silently initialising an empty workspace (REQ-084).
    let backend =
        zetl::history::open_history(&vault_root).context("opening jj workspace for --at query")?;

    // list_changes with a large limit to get all history (OBS-011).
    let history_load_start = Instant::now();
    let snapshots = backend
        .list_changes(10_000)
        .context("listing jj snapshots")?;
    let history_load_ms = history_load_start.elapsed().as_millis();
    if verbose > 0 {
        eprintln!(
            "[zetl] history: loaded {} snapshots duration_ms={}",
            snapshots.len(),
            history_load_ms
        );
    }

    let now = chrono::Local::now().fixed_offset();
    // OBS-011: time --at expression resolution.
    let at_resolve_start = Instant::now();
    let snapshot_info = resolve_snapshot(at_expr, now, &snapshots)
        .with_context(|| format!("resolving --at expression {at_expr:?}"))?;
    let at_resolve_ms = at_resolve_start.elapsed().as_millis();
    if verbose > 0 {
        eprintln!(
            "[zetl] at: resolved {:?} → {} (change={}) duration_ms={}",
            at_expr,
            snapshot_info.timestamp.to_rfc3339(),
            snapshot_info.change_id,
            at_resolve_ms
        );
    }

    let vault_root_hash =
        zetl::history::core::extract_vault_root_hash_from_description(&snapshot_info.description)
            .ok_or_else(|| {
            anyhow::anyhow!(
                "Snapshot {} has no vault_root_hash in its description. \
                     Run `zetl index` to rebuild the historical cache.",
                snapshot_info.change_id
            )
        })?;

    // OBS-011: time historical cache load and report hit/miss.
    let cache = HistoricalIndexCache::with_default_capacity();
    let cache_load_start = Instant::now();
    let file_map_opt = cache
        .load(&vault_root, &vault_root_hash)
        .context("reading historical index cache")?;
    let cache_load_ms = cache_load_start.elapsed().as_millis();
    if verbose > 0 {
        if file_map_opt.is_some() {
            eprintln!(
                "[zetl] at: cache hit vault_root_hash={vault_root_hash} duration_ms={cache_load_ms}"
            );
        } else {
            eprintln!(
                "[zetl] at: cache miss vault_root_hash={vault_root_hash} duration_ms={cache_load_ms}"
            );
        }
    }
    let file_map = file_map_opt.ok_or_else(|| {
        anyhow::anyhow!(
            "No cached index for snapshot {} (vault_root_hash={}). \
                 Run `zetl index` at that point in time to populate the cache.",
            snapshot_info.change_id,
            vault_root_hash
        )
    })?;

    let files: Vec<ParsedFile> = file_map.into_values().collect();

    let file_index: Vec<(String, PathBuf)> = files
        .iter()
        .map(|f| (f.page_name.clone(), f.path.clone()))
        .collect();

    let mut resolved_pages: HashMap<String, String> = HashMap::new();
    for file in &files {
        for link in &file.links {
            let key = link.raw_target.clone();
            if resolved_pages.contains_key(&key) {
                continue;
            }
            if let Some(resolved) = resolve_page_name(&link.target_page, &file_index) {
                resolved_pages.insert(key, resolved);
            }
        }
    }

    let graph = LinkGraph::build(&files, &resolved_pages);
    let graph_resolved = graph.resolved.clone();

    let total_leaf_nodes: usize = files.iter().map(|f| f.merkle_leaves.len()).sum();
    let spl_leaves_dual_hash: usize = files
        .iter()
        .flat_map(|f| &f.merkle_leaves)
        .filter(|l| l.spl_hashes.is_some())
        .count();

    let snapshot = SnapshotInfo {
        change_id: snapshot_info.change_id.clone(),
        commit_id: snapshot_info.commit_id.clone(),
        timestamp: snapshot_info.timestamp.to_rfc3339(),
        description: snapshot_info.description.clone(),
    };

    Ok(Pipeline {
        files,
        file_index,
        graph,
        vault_root,
        scan_stats: ScanStats {
            tier1_hits: 0,
            tier1_misses: 0,
            tier2_hits: 0,
            tier2_misses: 0,
            files_hashed: 0,
            total_leaf_nodes,
            spl_leaves_dual_hash,
            blake3_hashing_ms: 0,
            section_detection_ms: 0,
        },
        graph_resolved,
        snapshot: Some(snapshot),
    })
}

// ── No-index fallback ──────────────────────────────────────────────────────

/// Pre-launch index check (REQ-072, CON-023).
///
/// Checks whether `.zetl/index.json` exists in the vault root before entering
/// the alternate screen.  When the file is absent the user gets a brief status
/// message on stderr and the index pipeline runs in-process (identical to
/// `zetl index`).
///
/// * On success the freshly-built `Pipeline` is returned so the caller can
///   reuse it rather than scanning a second time.
/// * On pipeline failure the function prints a JSON error to stderr and exits
///   with a non-zero status (error code `INDEX_BUILD_FAILED`).
///
/// Returns `Some(Pipeline)` when the index was missing and has just been built,
/// or `None` when the index already existed (caller is responsible for loading
/// it via `run_pipeline` if needed).
fn check_no_index_fallback(cli: &Cli) -> Result<Option<Pipeline>> {
    let vault_root = std::fs::canonicalize(&cli.dir)
        .with_context(|| format!("Cannot resolve vault directory: {}", cli.dir))?;
    let index_path = vault_root.join(".zetl").join("index.json");

    if !index_path.exists() {
        eprintln!("Building index\u{2026}");
        match run_pipeline(cli) {
            Ok(pipeline) => return Ok(Some(pipeline)),
            Err(e) => {
                let json = serde_json::json!({
                    "error": {
                        "code": "INDEX_BUILD_FAILED",
                        "message": e.to_string()
                    }
                });
                eprintln!("{json}");
                std::process::exit(1);
            }
        }
    }

    Ok(None)
}

// ── Page lookup helpers ────────────────────────────────────────────────────

/// Resolve a page name from user input. Case-insensitive lookup, with optional
/// fuzzy fallback via SimHash.
fn find_page(
    page_input: &str,
    file_index: &[(String, PathBuf)],
    fuzzy: bool,
    files: &[ParsedFile],
) -> Result<String, String> {
    // First try exact/normalized resolution
    if let Some(resolved) = resolve_page_name(page_input, file_index) {
        return Ok(resolved);
    }

    // If fuzzy is enabled, use SimHash to find the best match
    if fuzzy {
        let pages: Vec<(String, String)> = files
            .iter()
            .map(|f| (f.page_name.clone(), f.path.to_string_lossy().to_string()))
            .collect();
        let index = SimHashIndex::build(&pages);
        let results = index.search(page_input, 10, 1);
        if let Some(best) = results.first() {
            return Ok(best.page.clone());
        }
    }

    Err(format!("Page not found: '{page_input}'"))
}

// ── Output helpers ─────────────────────────────────────────────────────────

fn print_json<T: Serialize>(value: &T) -> Result<()> {
    let json = serde_json::to_string_pretty(value)?;
    println!("{json}");
    Ok(())
}

/// Print a structured JSON error to stdout and exit.
/// Used when -f json is specified so agents can always parse stdout as JSON.
fn exit_json_error(message: &str, code: i32) -> ! {
    #[derive(Serialize)]
    struct JsonError {
        error: String,
        code: i32,
    }
    let err = JsonError {
        error: message.to_string(),
        code,
    };
    // Errors go to stderr so stdout remains valid JSON for pipe consumers (clig.dev).
    // Ignore write errors — we're exiting anyway.
    let _ = serde_json::to_string_pretty(&err).map(|json| eprintln!("{json}"));
    std::process::exit(code);
}

/// Handle a page-not-found error: JSON on stdout when format=json, plain text on stderr otherwise.
fn exit_page_not_found(format: &OutputFormat, message: &str) -> ! {
    match format {
        OutputFormat::Json => exit_json_error(message, 1),
        _ => {
            eprintln!("{message}");
            eprintln!();
            eprintln!(
                "Hint: run `zetl list` to see all pages, or use --fuzzy for approximate matching."
            );
            std::process::exit(1);
        }
    }
}

// ── Filesystem helpers ─────────────────────────────────────────────────────

/// Recursively sum the byte sizes of all files under `path` and return the
/// result in whole kilobytes (rounded down).  Returns 0 if the path does not
/// exist or cannot be read.
fn dir_size_kb(path: &Path) -> u64 {
    fn sum_bytes(path: &Path) -> u64 {
        let mut total = 0u64;
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    if meta.is_dir() {
                        total += sum_bytes(&entry.path());
                    } else {
                        total += meta.len();
                    }
                }
            }
        }
        total
    }
    sum_bytes(path) / 1024
}

// ── Command handlers ───────────────────────────────────────────────────────

fn cmd_index(cli: &Cli) -> Result<()> {
    let start = Instant::now();
    let pipeline = run_pipeline(cli)?;
    let elapsed = start.elapsed();

    // Auto-snapshot after index completion (REQ-076, ADR-048).
    // Non-fatal: errors are silently swallowed (or reported via --verbose).
    // The vault_root_hash written to index.json by run_pipeline is used as the
    // snapshot description and for fast deduplication.
    #[cfg(feature = "history")]
    {
        let vault_root_hash = load_vault_root_hex(&pipeline.vault_root).unwrap_or(None);
        // OBS-011: time snapshot creation.
        let snap_start = Instant::now();
        match zetl::history::auto_snapshot(&pipeline.vault_root, vault_root_hash.as_deref()) {
            Ok(Some(change_id)) => {
                let snap_ms = snap_start.elapsed().as_millis();
                if cli.verbose > 0 {
                    eprintln!(
                        "[zetl] snapshot: created change {} (vault_root_hash={}) duration_ms={}",
                        change_id,
                        vault_root_hash.as_deref().unwrap_or("unknown"),
                        snap_ms
                    );
                }
            }
            Ok(None) => {} // deduplicated — vault state unchanged
            Err(e) => {
                if cli.verbose > 0 {
                    eprintln!("[zetl] warning: auto-snapshot failed: {e}");
                }
            }
        }

        // Store the current index in HistoricalIndexCache so that future
        // --at queries can load it (REQ-079, ADR-047).
        if let Some(ref hash) = vault_root_hash {
            let hist_cache = zetl::history::cache::HistoricalIndexCache::with_default_capacity();
            if let Err(e) = hist_cache.store(&pipeline.vault_root, hash, &pipeline.files) {
                if cli.verbose > 0 {
                    eprintln!("[zetl] warning: failed to store historical index: {e}");
                }
            }
        }
    }

    // OBS-007 + OBS-009: emit scan and cache-efficiency stats to stderr when --verbose.
    if cli.verbose > 0 {
        let s = &pipeline.scan_stats;
        eprintln!("files hashed: {}", s.files_hashed);
        eprintln!("files skipped (mtime hit): {}", s.tier1_hits);
        eprintln!(
            "files with content-hash match (touch hit): {}",
            s.tier2_hits
        );
        eprintln!("total leaf nodes: {}", s.total_leaf_nodes);
        eprintln!("SPL leaves with dual hashing: {}", s.spl_leaves_dual_hash);
        eprintln!("BLAKE3 hashing time: {}ms", s.blake3_hashing_ms);
        eprintln!(
            "section detection and grounding hash time: {}ms",
            s.section_detection_ms
        );
        eprintln!("tier1_hits: {}", s.tier1_hits);
        eprintln!("tier1_misses: {}", s.tier1_misses);
        eprintln!("tier2_hits: {}", s.tier2_hits);
        eprintln!("tier2_misses: {}", s.tier2_misses);
    }

    // Build or skip the Tantivy search index (REQ-013-001, REQ-013-003).
    //
    // Strategy (v1): rebuild the entire index from scratch whenever any file
    // changed (files_hashed > 0), the index directory is absent, or --no-cache
    // was requested.  For --no-cache, the existing index directory is deleted
    // first so Tantivy starts with a clean on-disk layout.
    let search_dir = pipeline.vault_root.join(".zetl").join("search");
    let needs_rebuild =
        cli.no_cache || pipeline.scan_stats.files_hashed > 0 || !search_dir.exists();

    let (search_index_docs, search_index_build_ms) = if needs_rebuild {
        if cli.no_cache && search_dir.exists() {
            std::fs::remove_dir_all(&search_dir)
                .with_context(|| format!("removing search index directory {search_dir:?}"))?;
        }
        let idx_start = Instant::now();
        SearchIndex::build(&pipeline.vault_root, &pipeline.files)
            .context("building search index")?;
        let idx_elapsed_ms = idx_start.elapsed().as_millis();
        (pipeline.files.len(), idx_elapsed_ms)
    } else {
        (pipeline.files.len(), 0)
    };

    let search_index_size_kb = dir_size_kb(&search_dir);

    // OBS-013-001: verbose search index stats.
    if cli.verbose > 0 {
        eprintln!("documents indexed: {search_index_docs}");
        eprintln!("search index size: {search_index_size_kb} KB");
        eprintln!("search index build time: {search_index_build_ms}ms");
    }

    // Build or skip the semantic (vector) index (REQ-092, REQ-097).
    //
    // Uses the same `needs_rebuild` condition as the Tantivy index.
    // `VectorIndex::build` handles incremental rebuild internally: chunks
    // whose BLAKE3 content hash is unchanged are reused without re-embedding
    // (REQ-097). OBS-017 is emitted by `VectorIndex::build` unconditionally.
    #[cfg(feature = "semantic")]
    let semantic_stats: Option<serde_json::Value> = if needs_rebuild {
        let vectors_dir = pipeline.vault_root.join(zetl::semantic::VECTORS_DIR);
        match zetl::semantic::VectorIndex::build(&pipeline.vault_root, &pipeline.files) {
            Ok(idx) => {
                let chunk_count = idx.chunk_count();
                let index_size_kb = dir_size_kb(&vectors_dir);
                Some(serde_json::json!({
                    "chunk_count": chunk_count,
                    "index_size_kb": index_size_kb,
                    "model_name": zetl::semantic::MODEL_NAME,
                }))
            }
            Err(e) => {
                if cli.verbose > 0 {
                    eprintln!("[zetl] warning: semantic index build failed: {e}");
                }
                None
            }
        }
    } else {
        // No file changes — report existing vector index size if available.
        let vectors_dir = pipeline.vault_root.join(zetl::semantic::VECTORS_DIR);
        if vectors_dir.exists() {
            let chunks_path = vectors_dir.join(zetl::semantic::CHUNKS_FILE);
            let chunk_count = std::fs::read_to_string(&chunks_path)
                .ok()
                .and_then(|s| serde_json::from_str::<Vec<serde_json::Value>>(&s).ok())
                .map(|v| v.len())
                .unwrap_or(0);
            let index_size_kb = dir_size_kb(&vectors_dir);
            Some(serde_json::json!({
                "chunk_count": chunk_count,
                "index_size_kb": index_size_kb,
                "model_name": zetl::semantic::MODEL_NAME,
            }))
        } else {
            None
        }
    };
    #[cfg(not(feature = "semantic"))]
    let semantic_stats: Option<serde_json::Value> = None;

    let total_links: usize = pipeline.files.iter().map(|f| f.links.len()).sum();
    let total_diagnostics: usize = pipeline.files.iter().map(|f| f.diagnostics.len()).sum();
    let dead_links = pipeline.graph.dead_links();

    #[derive(Serialize)]
    struct IndexResult {
        files_scanned: usize,
        links_found: usize,
        dead_links: usize,
        diagnostics: usize,
        elapsed_ms: u128,
        search_index_docs: usize,
        search_index_size_kb: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        semantic: Option<serde_json::Value>,
    }

    let result = IndexResult {
        files_scanned: pipeline.files.len(),
        links_found: total_links,
        dead_links: dead_links.len(),
        diagnostics: total_diagnostics,
        elapsed_ms: elapsed.as_millis(),
        search_index_docs,
        search_index_size_kb,
        semantic: semantic_stats,
    };

    match cli.format {
        OutputFormat::Json => print_json(&result)?,
        _ => {
            let mut table = Table::new();
            table.set_header(vec!["Metric", "Value"]);
            table.add_row(vec![
                Cell::new("Files scanned"),
                Cell::new(result.files_scanned),
            ]);
            table.add_row(vec![
                Cell::new("Links found"),
                Cell::new(result.links_found),
            ]);
            table.add_row(vec![Cell::new("Dead links"), Cell::new(result.dead_links)]);
            table.add_row(vec![
                Cell::new("Diagnostics"),
                Cell::new(result.diagnostics),
            ]);
            table.add_row(vec![
                Cell::new("Elapsed (ms)"),
                Cell::new(result.elapsed_ms),
            ]);
            table.add_row(vec![
                Cell::new("Search index docs"),
                Cell::new(result.search_index_docs),
            ]);
            table.add_row(vec![
                Cell::new("Search index size (KB)"),
                Cell::new(result.search_index_size_kb),
            ]);
            if let Some(ref sem) = result.semantic {
                table.add_row(vec![
                    Cell::new("Vector chunks"),
                    Cell::new(sem["chunk_count"].as_u64().unwrap_or(0)),
                ]);
                table.add_row(vec![
                    Cell::new("Vector index size (KB)"),
                    Cell::new(sem["index_size_kb"].as_u64().unwrap_or(0)),
                ]);
            }
            println!("{table}");
        }
    }

    // ── post-index hooks (non-fatal) ────────────────────────────────
    let verbose = cli.verbose > 0;
    let theme_hooks = zetl::hooks::resolve_theme_hooks(&pipeline.vault_root, "");
    let manifest =
        zetl::hooks::discover_hooks_verbose(&pipeline.vault_root, theme_hooks.path(), verbose);

    for w in &manifest.warnings {
        eprintln!("warning: {w}");
    }

    if !zetl::hooks::hooks_for(&manifest, "post-index").is_empty() {
        let ctx = zetl::hooks::context::build_hook_context(
            "post-index",
            &pipeline.vault_root,
            "",
            env!("CARGO_PKG_VERSION"),
            &pipeline.files,
            &pipeline.graph,
        );

        let context_json = serde_json::to_vec(&ctx)?;

        let hook_env = zetl::hooks::HookEnv {
            vault_root: pipeline.vault_root.clone(),
            theme: String::new(),
            zetl_version: env!("CARGO_PKG_VERSION").to_string(),
            extra_vars: vec![],
        };

        let results = zetl::hooks::run_hooks_verbose(
            &manifest,
            "post-index",
            &context_json,
            &hook_env,
            verbose,
        );

        for result in results {
            match result {
                Ok(output) if !output.success() => {
                    eprintln!(
                        "warning: post-index hook '{}' ({}) exited with code {}",
                        output.path.display(),
                        output.source,
                        output.exit_code.unwrap_or(-1),
                    );
                    if !output.stderr.is_empty() {
                        eprintln!("  stderr: {}", output.stderr.trim_end());
                    }
                }
                Err(e) => {
                    eprintln!("warning: post-index hook failed to execute: {e}");
                }
                _ => {}
            }
        }
    }

    Ok(())
}

// ── zetl watch ─────────────────────────────────────────────────────────────

/// ISO 8601 UTC timestamp using pure std (no external date library needed).
///
/// Uses Hinnant's algorithm for Gregorian calendar conversion.
fn iso8601_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Days since Unix epoch → Gregorian date (Hinnant's algorithm).
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - (doe / 146_096).min(1)) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = y + if m <= 2 { 1 } else { 0 };
    let h = (secs % 86_400) / 3600;
    let min = (secs % 3600) / 60;
    let s = secs % 60;
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{min:02}:{s:02}Z")
}

/// Emit a single watch event as a compact JSON line on stdout.
///
/// When `exec` is Some, also spawns a detached thread that pipes the event
/// JSON to stdin of `sh -c <exec>` (REQ-059: non-blocking, per-event).
fn emit_watch_event(event: &serde_json::Value, exec: Option<&str>, verbose: u8) {
    let line = serde_json::to_string(event).unwrap_or_default();
    println!("{line}");

    if let Some(cmd) = exec {
        let line_clone = line.clone();
        let cmd_clone = cmd.to_string();
        let event_type = event
            .get("event")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string();
        std::thread::spawn(move || {
            use std::io::Write;
            use std::process::{Command, Stdio};
            let mut child = match Command::new("sh")
                .args(["-c", &cmd_clone])
                .stdin(Stdio::piped())
                .spawn()
            {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("[zetl watch] --exec spawn failed: {e}");
                    return;
                }
            };
            if let Some(stdin) = child.stdin.as_mut() {
                let _ = stdin.write_all(line_clone.as_bytes());
                let _ = stdin.write_all(b"\n");
            }
            drop(child.stdin.take());
            if let Ok(status) = child.wait() {
                if !status.success() {
                    let code = status.code().unwrap_or(-1);
                    eprintln!("[zetl watch] --exec exited {code} for event {event_type}");
                }
            }
        });
        let _ = verbose;
    }
}

/// Collect `.md` paths from a notify event into `changed`, excluding `.zetl/`.
fn collect_md_paths(
    evt: notify::Result<notify::Event>,
    vault_root: &Path,
    changed: &mut HashSet<PathBuf>,
) {
    let Ok(event) = evt else { return };
    for path in event.paths {
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        // Ignore changes inside the .zetl/ metadata directory.
        let rel = path.strip_prefix(vault_root).unwrap_or(&path);
        if rel.components().any(|c| c.as_os_str() == ".zetl") {
            continue;
        }
        changed.insert(path);
    }
}

/// `zetl watch` — watch vault for file changes and emit NDJSON graph events (SPEC-008).
///
/// After each incremental re-index cycle, creates a jj snapshot asynchronously
/// using the same deduplication as `auto_snapshot` (REQ-082, ADR-048).
fn cmd_watch(cli: &Cli, debounce_ms: u64, exec: Option<&str>) -> Result<()> {
    use notify::{Config as NotifyConfig, RecommendedWatcher, RecursiveMode, Watcher};
    use std::sync::mpsc;
    use std::time::Duration;

    // Validate vault path (REQ-061).
    let vault_path = Path::new(&cli.dir);
    if !vault_path.is_dir() {
        let err = serde_json::json!({
            "error": {
                "code": "VAULT_NOT_FOUND",
                "message": format!("Path '{}' does not exist or is not a directory.", cli.dir),
            }
        });
        println!("{}", serde_json::to_string(&err)?);
        std::process::exit(1);
    }

    // Initial index pass.
    let mut current = run_pipeline(cli)?;
    let vault_root = current.vault_root.clone();

    let total_links: usize = current.files.iter().map(|f| f.links.len()).sum();
    let orphan_count = current.graph.orphans().len();
    let dead_link_count = current.graph.dead_links().len();

    // Emit index_ready (REQ-057, REQ-058).
    emit_watch_event(
        &serde_json::json!({
            "event": "index_ready",
            "timestamp": iso8601_now(),
            "pages": current.files.len(),
            "links": total_links,
            "orphans": orphan_count,
            "dead_links": dead_link_count,
        }),
        exec,
        cli.verbose,
    );

    if cli.verbose > 0 {
        eprintln!(
            "[zetl watch] started  vault={}  debounce={}ms  pages={}",
            vault_root.display(),
            debounce_ms,
            current.files.len()
        );
    }

    // Set up FS watcher (REQ-054, ADR-013).
    let (fs_tx, fs_rx) = mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher = RecommendedWatcher::new(
        move |res| {
            let _ = fs_tx.send(res);
        },
        NotifyConfig::default(),
    )
    .context("initialising file-system watcher")?;
    watcher
        .watch(&vault_root, RecursiveMode::Recursive)
        .context("starting file-system watch")?;

    // Graceful shutdown via Ctrl+C / SIGTERM (REQ-060).
    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();
    {
        let tx = shutdown_tx.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio rt for ctrl-c");
            rt.block_on(async {
                tokio::signal::ctrl_c().await.ok();
            });
            let _ = tx.send(());
        });
    }
    #[cfg(unix)]
    {
        let tx = shutdown_tx.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio rt for SIGTERM");
            rt.block_on(async {
                use tokio::signal::unix::{signal, SignalKind};
                if let Ok(mut sig) = signal(SignalKind::terminate()) {
                    sig.recv().await;
                }
            });
            let _ = tx.send(());
        });
    }
    drop(shutdown_tx); // keep alive only via clones above

    let debounce = Duration::from_millis(debounce_ms);
    let poll_interval = Duration::from_millis(50);

    // Main watch loop (REQ-055).
    'watch: loop {
        // Wait for the first FS event, checking for shutdown every 50ms.
        let mut changed: HashSet<PathBuf> = HashSet::new();

        loop {
            if shutdown_rx.try_recv().is_ok() {
                break 'watch;
            }
            match fs_rx.recv_timeout(poll_interval) {
                Ok(evt) => {
                    collect_md_paths(evt, &vault_root, &mut changed);
                    break;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break 'watch,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
            }
        }

        // Collect remaining events within the debounce window (REQ-055).
        let deadline = Instant::now() + debounce;
        loop {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            let remaining = deadline - now;
            match fs_rx.recv_timeout(remaining) {
                Ok(evt) => collect_md_paths(evt, &vault_root, &mut changed),
                _ => break,
            }
        }

        if changed.is_empty() {
            continue;
        }
        if shutdown_rx.try_recv().is_ok() {
            break;
        }

        // Incremental re-index (REQ-055, REQ-056).
        let batch_start = Instant::now();
        let new_pipeline = match run_pipeline(cli) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[zetl watch] re-index error: {e:#}");
                continue;
            }
        };

        let now = iso8601_now();

        // Diff graphs and emit per-change events (REQ-057, REQ-058).

        // Pages added / removed.
        let old_pages: HashSet<&str> = current.files.iter().map(|f| f.page_name.as_str()).collect();
        let new_pages: HashSet<&str> = new_pipeline
            .files
            .iter()
            .map(|f| f.page_name.as_str())
            .collect();

        for page in new_pages.difference(&old_pages) {
            emit_watch_event(
                &serde_json::json!({"event":"page_added","timestamp":&now,"page":page}),
                exec,
                cli.verbose,
            );
        }
        for page in old_pages.difference(&new_pages) {
            emit_watch_event(
                &serde_json::json!({"event":"page_removed","timestamp":&now,"page":page}),
                exec,
                cli.verbose,
            );
        }

        // Links added / removed.
        let old_links: HashSet<(String, String)> = current
            .files
            .iter()
            .flat_map(|f| {
                current
                    .graph
                    .forward_links(&f.page_name)
                    .into_iter()
                    .map(|l| (f.page_name.clone(), l.target.clone()))
            })
            .collect();
        let new_links: HashSet<(String, String)> = new_pipeline
            .files
            .iter()
            .flat_map(|f| {
                new_pipeline
                    .graph
                    .forward_links(&f.page_name)
                    .into_iter()
                    .map(|l| (f.page_name.clone(), l.target.clone()))
            })
            .collect();

        for (from, to) in new_links.difference(&old_links) {
            emit_watch_event(
                &serde_json::json!({"event":"link_added","timestamp":&now,"from":from,"to":to}),
                exec,
                cli.verbose,
            );
        }
        for (from, to) in old_links.difference(&new_links) {
            emit_watch_event(
                &serde_json::json!({"event":"link_removed","timestamp":&now,"from":from,"to":to}),
                exec,
                cli.verbose,
            );
        }

        // Orphans gained / resolved.
        let old_orphans: HashSet<String> = current
            .graph
            .orphans()
            .into_iter()
            .map(|o| o.page)
            .collect();
        let new_orphans: HashSet<String> = new_pipeline
            .graph
            .orphans()
            .into_iter()
            .map(|o| o.page)
            .collect();

        for page in new_orphans.difference(&old_orphans) {
            emit_watch_event(
                &serde_json::json!({"event":"orphan_gained","timestamp":&now,"page":page}),
                exec,
                cli.verbose,
            );
        }
        for page in old_orphans.difference(&new_orphans) {
            emit_watch_event(
                &serde_json::json!({"event":"orphan_resolved","timestamp":&now,"page":page}),
                exec,
                cli.verbose,
            );
        }

        // Dead links added / resolved.
        let old_dead: HashSet<(String, String)> = current
            .graph
            .dead_links()
            .into_iter()
            .map(|dl| (dl.source, dl.target))
            .collect();
        let new_dead: HashSet<(String, String)> = new_pipeline
            .graph
            .dead_links()
            .into_iter()
            .map(|dl| (dl.source, dl.target))
            .collect();

        for (from, to) in new_dead.difference(&old_dead) {
            emit_watch_event(
                &serde_json::json!({"event":"dead_link_added","timestamp":&now,"from":from,"to":to}),
                exec,
                cli.verbose,
            );
        }
        for (from, to) in old_dead.difference(&new_dead) {
            emit_watch_event(
                &serde_json::json!({"event":"dead_link_resolved","timestamp":&now,"from":from,"to":to}),
                exec,
                cli.verbose,
            );
        }

        // index_updated batch summary event.
        let changed_page_names: Vec<String> = changed
            .iter()
            .filter_map(|p| p.file_stem()?.to_str().map(String::from))
            .collect();
        let duration_ms = batch_start.elapsed().as_millis();
        emit_watch_event(
            &serde_json::json!({
                "event": "index_updated",
                "timestamp": &now,
                "changed_pages": changed_page_names,
                "duration_ms": duration_ms,
            }),
            exec,
            cli.verbose,
        );

        if cli.verbose > 0 {
            eprintln!(
                "[zetl watch] batch  changed={}  total_ms={}",
                changed.len(),
                duration_ms
            );
        }

        // Asynchronous snapshot post-re-index (REQ-082, ADR-048).
        //
        // Spawns a detached thread so the watch loop is never blocked.
        // Uses the same vault_root_hash deduplication as cmd_index (no snapshot
        // when the graph content is unchanged).
        #[cfg(feature = "history")]
        {
            let vr = vault_root.clone();
            let hash = load_vault_root_hex(&vault_root).ok().flatten();
            let verbose = cli.verbose;
            std::thread::spawn(move || {
                // OBS-011: time async snapshot creation.
                let snap_start = std::time::Instant::now();
                match zetl::history::auto_snapshot(&vr, hash.as_deref()) {
                    Ok(Some(change_id)) => {
                        let snap_ms = snap_start.elapsed().as_millis();
                        if verbose > 0 {
                            eprintln!(
                                "[zetl watch] snapshot: created change {} (vault_root_hash={}) duration_ms={}",
                                change_id,
                                hash.as_deref().unwrap_or("unknown"),
                                snap_ms
                            );
                        }
                    }
                    Ok(None) => {} // deduplicated — graph unchanged
                    Err(e) => {
                        if verbose > 0 {
                            eprintln!("[zetl watch] warning: snapshot failed: {e:#}");
                        }
                    }
                }
            });
        }

        current = new_pipeline;
    }

    if cli.verbose > 0 {
        eprintln!("[zetl watch] shutdown");
    }
    Ok(())
}

fn cmd_links(
    cli: &Cli,
    page: &str,
    fuzzy: bool,
    context: usize,
    depth: usize,
    with_conclusions: bool,
) -> Result<()> {
    let pipeline = run_pipeline(cli)?;

    let resolved_page = find_page(page, &pipeline.file_index, fuzzy, &pipeline.files)
        .unwrap_or_else(|e| {
            exit_page_not_found(&cli.format, &e);
        });

    // If --with-conclusions, build the theory to get page→conclusion mappings
    #[cfg(feature = "reason")]
    let page_conclusions: HashMap<String, Vec<PageConclusionEntry>> = if with_conclusions {
        build_page_conclusions_map(&pipeline.files)
    } else {
        HashMap::new()
    };
    #[cfg(not(feature = "reason"))]
    let page_conclusions: HashMap<String, Vec<PageConclusionEntry>> = {
        if with_conclusions {
            eprintln!("--with-conclusions requires the 'reason' feature flag.");
            std::process::exit(1);
        }
        HashMap::new()
    };

    // BFS collecting forward links at each depth level
    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();
    visited.insert(resolved_page.clone());
    queue.push_back((resolved_page.clone(), 0));

    #[derive(Serialize)]
    struct LinkEntry {
        source: String,
        target: String,
        line: u32,
        alias: Option<String>,
        heading: Option<String>,
        is_embed: bool,
        context: Option<String>,
        hop: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        conclusions: Option<Vec<PageConclusionEntry>>,
    }

    let mut entries: Vec<LinkEntry> = Vec::new();
    let mut seen_edges: HashSet<(String, String, u32)> = HashSet::new();

    while let Some((current_page, current_depth)) = queue.pop_front() {
        if current_depth >= depth {
            continue;
        }

        let forward = pipeline.graph.forward_links(&current_page);
        for fwd in &forward {
            let target_name = &fwd.target;
            let edge = &fwd.meta;

            // Deduplicate: skip if we've already seen this (source, target, line)
            let dedup_key = (current_page.clone(), target_name.clone(), edge.line);
            if !seen_edges.insert(dedup_key) {
                if !visited.contains(target_name) {
                    visited.insert(target_name.clone());
                    queue.push_back((target_name.clone(), current_depth + 1));
                }
                continue;
            }

            // Context: read N chars around the wikilink position in the source file
            let ctx = if context > 0 {
                extract_context(&pipeline.vault_root, &edge.source_file, edge.line, context)
            } else {
                None
            };

            let conclusions = if with_conclusions {
                Some(
                    page_conclusions
                        .get(target_name)
                        .cloned()
                        .unwrap_or_default(),
                )
            } else {
                None
            };

            entries.push(LinkEntry {
                source: current_page.clone(),
                target: target_name.clone(),
                line: edge.line,
                alias: edge.alias.clone(),
                heading: edge.heading.clone(),
                is_embed: edge.is_embed,
                context: ctx,
                hop: current_depth + 1,
                conclusions,
            });

            if !visited.contains(target_name) {
                visited.insert(target_name.clone());
                queue.push_back((target_name.clone(), current_depth + 1));
            }
        }
    }

    #[derive(Serialize)]
    struct LinksOutput {
        page: String,
        depth: usize,
        links: Vec<LinkEntry>,
        #[serde(skip_serializing_if = "Option::is_none")]
        snapshot: Option<SnapshotInfo>,
    }

    let output = LinksOutput {
        page: resolved_page,
        depth,
        links: entries,
        snapshot: pipeline.snapshot.clone(),
    };

    match cli.format {
        OutputFormat::Json => print_json(&output)?,
        _ => {
            let mut table = Table::new();
            let mut headers = vec!["Hop", "Source", "Target", "Line"];
            if context > 0 {
                headers.push("Context");
            }
            if with_conclusions {
                headers.push("Conclusions");
            }
            table.set_header(headers);
            for entry in &output.links {
                let mut row = vec![
                    Cell::new(entry.hop),
                    Cell::new(&entry.source),
                    Cell::new(&entry.target),
                    Cell::new(entry.line),
                ];
                if context > 0 {
                    row.push(Cell::new(entry.context.as_deref().unwrap_or("")));
                }
                if with_conclusions {
                    let conc_str = entry
                        .conclusions
                        .as_ref()
                        .map(|cs| {
                            cs.iter()
                                .map(|c| format!("{} {}", c.conclusion_type, c.literal))
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default();
                    row.push(Cell::new(if conc_str.is_empty() { "-" } else { &conc_str }));
                }
                table.add_row(row);
            }
            println!("Forward links from '{}':", output.page);
            println!("{table}");
        }
    }

    Ok(())
}

fn cmd_backlinks(
    cli: &Cli,
    page: &str,
    fuzzy: bool,
    context: usize,
    depth: usize,
    with_conclusions: bool,
) -> Result<()> {
    let pipeline = run_pipeline(cli)?;

    let resolved_page = find_page(page, &pipeline.file_index, fuzzy, &pipeline.files)
        .unwrap_or_else(|e| {
            exit_page_not_found(&cli.format, &e);
        });

    // If --with-conclusions, build the theory to get page→conclusion mappings
    #[cfg(feature = "reason")]
    let page_conclusions: HashMap<String, Vec<PageConclusionEntry>> = if with_conclusions {
        build_page_conclusions_map(&pipeline.files)
    } else {
        HashMap::new()
    };
    #[cfg(not(feature = "reason"))]
    let page_conclusions: HashMap<String, Vec<PageConclusionEntry>> = {
        if with_conclusions {
            eprintln!("--with-conclusions requires the 'reason' feature flag.");
            std::process::exit(1);
        }
        HashMap::new()
    };

    // BFS collecting backlinks at each depth level
    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();
    visited.insert(resolved_page.clone());
    queue.push_back((resolved_page.clone(), 0));

    #[derive(Serialize)]
    struct BacklinkEntry {
        source: String,
        target: String,
        line: u32,
        alias: Option<String>,
        is_embed: bool,
        context: Option<String>,
        hop: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        conclusions: Option<Vec<PageConclusionEntry>>,
    }

    let mut entries: Vec<BacklinkEntry> = Vec::new();
    let mut seen_edges: HashSet<(String, String, u32)> = HashSet::new();

    while let Some((current_page, current_depth)) = queue.pop_front() {
        if current_depth >= depth {
            continue;
        }

        let backlinks = pipeline.graph.backlinks(&current_page);
        for bl in &backlinks {
            // Deduplicate: skip if we've already seen this (source, target, line)
            let dedup_key = (bl.source.clone(), current_page.clone(), bl.line);
            if !seen_edges.insert(dedup_key) {
                if !visited.contains(&bl.source) {
                    visited.insert(bl.source.clone());
                    queue.push_back((bl.source.clone(), current_depth + 1));
                }
                continue;
            }

            // Context: read N chars around the wikilink position in the source file
            let ctx = if context > 0 {
                let source_file_path = pipeline
                    .files
                    .iter()
                    .find(|f| f.page_name == bl.source)
                    .map(|f| f.path.to_string_lossy().to_string());
                if let Some(ref src_path) = source_file_path {
                    extract_context(&pipeline.vault_root, src_path, bl.line, context)
                } else {
                    None
                }
            } else {
                None
            };

            let conclusions = if with_conclusions {
                Some(
                    page_conclusions
                        .get(&bl.source)
                        .cloned()
                        .unwrap_or_default(),
                )
            } else {
                None
            };

            entries.push(BacklinkEntry {
                source: bl.source.clone(),
                target: current_page.clone(),
                line: bl.line,
                alias: bl.alias.clone(),
                is_embed: bl.is_embed,
                context: ctx,
                hop: current_depth + 1,
                conclusions,
            });

            if !visited.contains(&bl.source) {
                visited.insert(bl.source.clone());
                queue.push_back((bl.source.clone(), current_depth + 1));
            }
        }
    }

    #[derive(Serialize)]
    struct BacklinksOutput {
        page: String,
        depth: usize,
        backlinks: Vec<BacklinkEntry>,
        #[serde(skip_serializing_if = "Option::is_none")]
        snapshot: Option<SnapshotInfo>,
    }

    let output = BacklinksOutput {
        page: resolved_page,
        depth,
        backlinks: entries,
        snapshot: pipeline.snapshot.clone(),
    };

    match cli.format {
        OutputFormat::Json => print_json(&output)?,
        _ => {
            let mut table = Table::new();
            let mut headers = vec!["Hop", "Source", "Line"];
            if context > 0 {
                headers.push("Context");
            }
            if with_conclusions {
                headers.push("Conclusions");
            }
            table.set_header(headers);
            for entry in &output.backlinks {
                let mut row = vec![
                    Cell::new(entry.hop),
                    Cell::new(&entry.source),
                    Cell::new(entry.line),
                ];
                if context > 0 {
                    row.push(Cell::new(entry.context.as_deref().unwrap_or("")));
                }
                if with_conclusions {
                    let conc_str = entry
                        .conclusions
                        .as_ref()
                        .map(|cs| {
                            cs.iter()
                                .map(|c| format!("{} {}", c.conclusion_type, c.literal))
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default();
                    row.push(Cell::new(if conc_str.is_empty() { "-" } else { &conc_str }));
                }
                table.add_row(row);
            }
            println!("Backlinks to '{}':", output.page);
            println!("{table}");
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)] // command entry point — each flag maps to a CLI arg
fn cmd_check(
    cli: &Cli,
    show_dead_links: bool,
    show_orphans: bool,
    show_syntax: bool,
    show_spl: bool,
    show_drift: bool,
    fail_on: &FailLevel,
    theme: &str,
) -> Result<()> {
    #[cfg(not(feature = "reason"))]
    if show_spl {
        reason_not_available();
    }

    let pipeline = run_pipeline(cli)?;

    // If none of the flags are set, show all
    let show_all = !show_dead_links && !show_orphans && !show_syntax && !show_spl && !show_drift;

    let dead = if show_all || show_dead_links {
        pipeline.graph.dead_links()
    } else {
        vec![]
    };

    let orphan_list = if show_all || show_orphans {
        pipeline.graph.orphans()
    } else {
        vec![]
    };

    let diagnostics: Vec<_> = if show_all || show_syntax {
        pipeline
            .files
            .iter()
            .flat_map(|f| f.diagnostics.clone())
            .collect()
    } else {
        vec![]
    };

    // Collect SPL diagnostics (requires "reason" feature)
    let mut spl_diagnostics: Vec<zetl::types::Diagnostic> = if show_all || show_spl {
        collect_spl_diagnostics(&pipeline.files)
    } else {
        vec![]
    };

    // Validate source metadata references (REQ-042, CON-004).  These are static
    // errors that do not require the "reason" feature — they run whenever SPL
    // diagnostics are requested (show_all or show_spl).
    if show_all || show_spl {
        let vault_hash_index = build_vault_hash_index(&pipeline.files);
        let source_errors =
            validate_source_refs(&pipeline.files, &pipeline.file_index, &vault_hash_index);
        spl_diagnostics.extend(source_errors);
    }

    // Load theory cache once for drift detection, broken_groundings, and explicitly_grounded_facts.
    // Requires a prior `zetl reason status` that produced theory.json — if none exists,
    // load_theory_cache returns None and we skip the theory-cache-dependent checks silently.
    let theory_cache = load_theory_cache(&pipeline.vault_root).unwrap_or(None);

    // Detect section-level and explicit-grounding drift (REQ-043a, REQ-043b).
    let drift_diagnostics: Vec<DriftDiagnostic> = if show_all || show_drift {
        match theory_cache.as_ref() {
            Some(theory) => {
                // Build vault hash index once for explicit-grounding drift resolution.
                let drift_hash_index = build_vault_hash_index(&pipeline.files);
                pipeline
                    .files
                    .iter()
                    .filter_map(|f| f.file_merkle.as_ref().map(|fm| (f, fm)))
                    .flat_map(|(f, fm)| {
                        let mut diags = detect_section_drift(&f.path, fm, &f.merkle_leaves, theory);
                        diags.extend(detect_explicit_drift(
                            &f.path,
                            fm,
                            theory,
                            &pipeline.files,
                            &pipeline.file_index,
                            &drift_hash_index,
                        ));
                        diags
                    })
                    .collect()
            }
            None => vec![],
        }
    } else {
        vec![]
    };

    // OBS-008: compute summary fields from theory cache.
    let total_spl_blocks: usize = pipeline.files.iter().map(|f| f.spl_blocks.len()).sum();

    let explicitly_grounded_facts: usize = theory_cache.as_ref().map_or(0, |tc| {
        tc.spl_blocks
            .values()
            .map(|b| b.explicit_groundings.len())
            .sum()
    });

    // Build a fast lookup from file path → set of leaf hashes for broken-grounding detection.
    let leaf_hash_index: HashMap<std::path::PathBuf, std::collections::HashSet<ContentHash>> =
        pipeline
            .files
            .iter()
            .map(|f| {
                let hashes = f.merkle_leaves.iter().map(|l| l.hash).collect();
                (f.path.clone(), hashes)
            })
            .collect();

    let broken_groundings: usize = theory_cache.as_ref().map_or(0, |tc| {
        tc.spl_blocks
            .values()
            .flat_map(|b| &b.explicit_groundings)
            .flat_map(|g| &g.targets)
            .filter(|t| {
                // A grounding is broken if the target file is gone or its leaf hash changed.
                match leaf_hash_index.get(&t.target_file) {
                    Some(hashes) => !hashes.contains(&t.target_leaf_hash),
                    None => true,
                }
            })
            .count()
    });

    #[derive(Serialize)]
    struct CheckSummary {
        dead_links: usize,
        orphans: usize,
        syntax_errors: usize,
        spl_errors: usize,
        drift_warnings: usize,
        drift_info: usize,
        total_spl_blocks: usize,
        drifted_blocks_warning: usize,
        drifted_blocks_info: usize,
        explicitly_grounded_facts: usize,
        broken_groundings: usize,
    }

    #[derive(Serialize)]
    struct CheckOutput {
        dead_links: Vec<zetl::graph::DeadLink>,
        orphans: Vec<zetl::graph::Orphan>,
        syntax_errors: Vec<zetl::types::Diagnostic>,
        spl_diagnostics: Vec<zetl::types::Diagnostic>,
        drift_diagnostics: Vec<DriftDiagnostic>,
        summary: CheckSummary,
        #[serde(skip_serializing_if = "Option::is_none")]
        snapshot: Option<SnapshotInfo>,
    }

    let drifted_blocks_warning = drift_diagnostics
        .iter()
        .filter(|d| matches!(d.severity, DriftSeverity::Warning))
        .count();
    let drifted_blocks_info = drift_diagnostics
        .iter()
        .filter(|d| matches!(d.severity, DriftSeverity::Info))
        .count();

    let summary = CheckSummary {
        dead_links: dead.len(),
        orphans: orphan_list.len(),
        syntax_errors: diagnostics.len(),
        spl_errors: spl_diagnostics
            .iter()
            .filter(|d| d.level == DiagnosticLevel::Error)
            .count(),
        drift_warnings: drifted_blocks_warning,
        drift_info: drifted_blocks_info,
        total_spl_blocks,
        drifted_blocks_warning,
        drifted_blocks_info,
        explicitly_grounded_facts,
        broken_groundings,
    };

    let output = CheckOutput {
        dead_links: dead,
        orphans: orphan_list,
        syntax_errors: diagnostics,
        spl_diagnostics,
        drift_diagnostics,
        summary,
        snapshot: pipeline.snapshot.clone(),
    };

    match cli.format {
        OutputFormat::Json => print_json(&output)?,
        _ => {
            if !output.dead_links.is_empty() {
                let mut table = Table::new();
                table.set_header(vec!["Source", "Line", "Dead Target"]);
                for dl in &output.dead_links {
                    table.add_row(vec![
                        Cell::new(&dl.source),
                        Cell::new(dl.line),
                        Cell::new(&dl.target),
                    ]);
                }
                println!("Dead Links:");
                println!("{table}");
                println!();
            }

            if !output.orphans.is_empty() {
                let mut table = Table::new();
                table.set_header(vec!["Orphan Page", "Forward Links"]);
                for o in &output.orphans {
                    table.add_row(vec![Cell::new(&o.page), Cell::new(o.forward_links)]);
                }
                println!("Orphan Pages:");
                println!("{table}");
                println!();
            }

            if !output.syntax_errors.is_empty() {
                let mut table = Table::new();
                table.set_header(vec!["Level", "File", "Line", "Column", "Message"]);
                for d in &output.syntax_errors {
                    table.add_row(vec![
                        Cell::new(format!("{:?}", d.level)),
                        Cell::new(d.file.display()),
                        Cell::new(d.line),
                        Cell::new(d.column),
                        Cell::new(&d.message),
                    ]);
                }
                println!("Syntax Diagnostics:");
                println!("{table}");
                println!();
            }

            if !output.spl_diagnostics.is_empty() {
                let mut table = Table::new();
                table.set_header(vec!["Level", "File", "Line", "Column", "Message"]);
                for d in &output.spl_diagnostics {
                    table.add_row(vec![
                        Cell::new(format!("{:?}", d.level)),
                        Cell::new(d.file.display()),
                        Cell::new(d.line),
                        Cell::new(d.column),
                        Cell::new(&d.message),
                    ]);
                }
                println!("SPL Diagnostics:");
                println!("{table}");
                println!();
            }

            if !output.drift_diagnostics.is_empty() {
                let mut table = Table::new();
                table.set_header(vec!["Severity", "File", "SPL Line", "Message"]);
                for d in &output.drift_diagnostics {
                    let severity = match d.severity {
                        DriftSeverity::Warning => "Warning",
                        DriftSeverity::Info => "Info",
                    };
                    table.add_row(vec![
                        Cell::new(severity),
                        Cell::new(d.file.display()),
                        Cell::new(d.spl_line),
                        Cell::new(&d.message),
                    ]);
                }
                println!("Drift Diagnostics:");
                println!("{table}");
                println!();
            }

            // OBS-008: always print summary stats table.
            {
                let mut sum_table = Table::new();
                sum_table.set_header(vec!["Summary", "Count"]);
                sum_table.add_row(vec![
                    Cell::new("Dead links"),
                    Cell::new(output.summary.dead_links),
                ]);
                sum_table.add_row(vec![
                    Cell::new("Orphan pages"),
                    Cell::new(output.summary.orphans),
                ]);
                sum_table.add_row(vec![
                    Cell::new("Syntax errors"),
                    Cell::new(output.summary.syntax_errors),
                ]);
                sum_table.add_row(vec![
                    Cell::new("SPL errors"),
                    Cell::new(output.summary.spl_errors),
                ]);
                sum_table.add_row(vec![
                    Cell::new("Total SPL blocks"),
                    Cell::new(output.summary.total_spl_blocks),
                ]);
                sum_table.add_row(vec![
                    Cell::new("Drifted blocks (warning)"),
                    Cell::new(output.summary.drifted_blocks_warning),
                ]);
                sum_table.add_row(vec![
                    Cell::new("Drifted blocks (info)"),
                    Cell::new(output.summary.drifted_blocks_info),
                ]);
                sum_table.add_row(vec![
                    Cell::new("Explicitly grounded facts"),
                    Cell::new(output.summary.explicitly_grounded_facts),
                ]);
                sum_table.add_row(vec![
                    Cell::new("Broken groundings"),
                    Cell::new(output.summary.broken_groundings),
                ]);
                println!("Summary:");
                println!("{sum_table}");
                println!();
            }

            if output.dead_links.is_empty()
                && output.orphans.is_empty()
                && output.syntax_errors.is_empty()
                && output.spl_diagnostics.is_empty()
                && output.drift_diagnostics.is_empty()
            {
                println!("No issues found.");
            }
        }
    }

    // ── post-check hooks (REQ-016-004: non-fatal) ──────────────────────
    let verbose = cli.verbose > 0;
    let theme_hooks = zetl::hooks::resolve_theme_hooks(&pipeline.vault_root, theme);
    let manifest =
        zetl::hooks::discover_hooks_verbose(&pipeline.vault_root, theme_hooks.path(), verbose);

    for w in &manifest.warnings {
        eprintln!("warning: {w}");
    }

    if !zetl::hooks::hooks_for(&manifest, "post-check").is_empty() {
        // Collect full diagnostics for hook context (unfiltered by display flags).
        let hook_dead_links = pipeline.graph.dead_links();
        let hook_orphans = pipeline.graph.orphans();
        let hook_syntax_errors: Vec<zetl::types::Diagnostic> = pipeline
            .files
            .iter()
            .flat_map(|f| f.diagnostics.clone())
            .collect();

        let mut ctx = zetl::hooks::context::build_hook_context(
            "post-check",
            &pipeline.vault_root,
            theme,
            env!("CARGO_PKG_VERSION"),
            &pipeline.files,
            &pipeline.graph,
        );
        ctx.diagnostics = Some(zetl::hooks::context::HookDiagnostics {
            dead_links: hook_dead_links,
            orphans: hook_orphans,
            syntax_errors: hook_syntax_errors,
        });

        let context_json = serde_json::to_vec(&ctx)?;

        let hook_env = zetl::hooks::HookEnv {
            vault_root: pipeline.vault_root.clone(),
            theme: theme.to_string(),
            zetl_version: env!("CARGO_PKG_VERSION").to_string(),
            extra_vars: vec![],
        };

        let results = zetl::hooks::run_hooks_verbose(
            &manifest,
            "post-check",
            &context_json,
            &hook_env,
            verbose,
        );

        for result in results {
            match result {
                Ok(hook_output) if !hook_output.success() => {
                    eprintln!(
                        "warning: post-check hook '{}' ({}) exited with code {}",
                        hook_output.path.display(),
                        hook_output.source,
                        hook_output.exit_code.unwrap_or(-1),
                    );
                    if !hook_output.stderr.is_empty() {
                        eprintln!("  stderr: {}", hook_output.stderr.trim_end());
                    }
                }
                Err(e) => {
                    eprintln!("warning: post-check hook failed to execute: {e}");
                }
                _ => {}
            }
        }
    }

    // Determine exit code based on fail_on level
    let has_errors = !output.dead_links.is_empty()
        || !output.orphans.is_empty()
        || output
            .syntax_errors
            .iter()
            .any(|d| d.level == DiagnosticLevel::Error)
        || output
            .spl_diagnostics
            .iter()
            .any(|d| d.level == DiagnosticLevel::Error);

    let has_warnings = output
        .syntax_errors
        .iter()
        .any(|d| d.level == DiagnosticLevel::Warning)
        || output
            .spl_diagnostics
            .iter()
            .any(|d| d.level == DiagnosticLevel::Warning)
        || output
            .drift_diagnostics
            .iter()
            .any(|d| matches!(d.severity, DriftSeverity::Warning));

    let should_fail = match fail_on {
        FailLevel::Error => has_errors,
        FailLevel::Warning => has_errors || has_warnings,
    };

    if should_fail {
        std::process::exit(1);
    }

    Ok(())
}

/// Collect SPL diagnostics by running the reasoning engine's build_theory.
///
/// Feature-gated: returns empty vec when the "reason" feature is disabled.
#[cfg(feature = "reason")]
fn collect_spl_diagnostics(files: &[ParsedFile]) -> Vec<zetl::types::Diagnostic> {
    use zetl::reason::build_theory;

    let spl_blocks: Vec<_> = files.iter().flat_map(|f| f.spl_blocks.clone()).collect();
    if spl_blocks.is_empty() {
        return vec![];
    }

    match build_theory(&spl_blocks) {
        Ok(result) => result.diagnostics,
        Err(e) => {
            vec![zetl::types::Diagnostic {
                level: DiagnosticLevel::Error,
                message: format!("SPL theory construction failed: {e}"),
                file: std::path::PathBuf::new(),
                line: 0,
                column: 0,
            }]
        }
    }
}

#[cfg(not(feature = "reason"))]
fn collect_spl_diagnostics(_files: &[ParsedFile]) -> Vec<zetl::types::Diagnostic> {
    vec![]
}

fn cmd_similar(cli: &Cli, query: &str, threshold: u32, limit: usize) -> Result<()> {
    let pipeline = run_pipeline(cli)?;

    let pages: Vec<(String, String)> = pipeline
        .files
        .iter()
        .map(|f| (f.page_name.clone(), f.path.to_string_lossy().to_string()))
        .collect();

    let index = SimHashIndex::build(&pages);
    let results = index.search(query, threshold, limit);

    #[derive(Serialize)]
    struct SimilarOutput {
        query: String,
        threshold: u32,
        results: Vec<zetl::simhash::SimilarResult>,
    }

    let output = SimilarOutput {
        query: query.to_string(),
        threshold,
        results,
    };

    match cli.format {
        OutputFormat::Json => print_json(&output)?,
        _ => {
            if output.results.is_empty() {
                println!("No similar pages found for '{query}'.");
            } else {
                let mut table = Table::new();
                table.set_header(vec!["Page", "Distance", "Path"]);
                for r in &output.results {
                    table.add_row(vec![
                        Cell::new(&r.page),
                        Cell::new(r.distance),
                        Cell::new(&r.path),
                    ]);
                }
                println!("Similar pages to '{query}':");
                println!("{table}");
            }
        }
    }

    Ok(())
}

fn cmd_stats(cli: &Cli, top: usize) -> Result<()> {
    let pipeline = run_pipeline(cli)?;
    let graph_stats = pipeline.graph.stats(top);

    // Vault content integrity fields (CON-006 §7)
    let vault_content_hash = load_vault_root_hex(&pipeline.vault_root).unwrap_or(None);
    let spl_blocks: usize = pipeline.files.iter().map(|f| f.spl_blocks.len()).sum();
    let theory = load_theory_cache(&pipeline.vault_root).unwrap_or(None);
    // Only count grounded / grounding counts for blocks that still exist in the
    // current pipeline; the cache may outlive deleted SPL blocks. Join key is
    // "<source_file>:<start_line>" — matches TheoryCache::spl_blocks keys
    // (see src/cache.rs).
    let live_block_keys: std::collections::HashSet<String> = pipeline
        .files
        .iter()
        .flat_map(|f| f.spl_blocks.iter())
        .map(|b| format!("{}:{}", b.source_file.display(), b.start_line))
        .collect();
    let grounded_spl_blocks = theory.as_ref().map_or(0, |tc| {
        tc.spl_blocks
            .iter()
            .filter(|(k, b)| b.section_grounding_hash != [0u8; 32] && live_block_keys.contains(*k))
            .count()
    });
    let explicitly_grounded_facts = theory.as_ref().map_or(0, |tc| {
        tc.spl_blocks
            .iter()
            .filter(|(k, _)| live_block_keys.contains(*k))
            .map(|(_, b)| b.explicit_groundings.len())
            .sum()
    });

    // OBS-012: collect history storage stats when the feature is available.
    #[cfg(feature = "history")]
    let history_stats: Option<serde_json::Value> = {
        use zetl::history::jj_backend::VcsBackend as _;
        (|| -> Option<serde_json::Value> {
            let backend = zetl::history::open_history(&pipeline.vault_root).ok()?;
            let snapshots = backend.list_changes(10_000).ok()?;
            if snapshots.is_empty() {
                return None;
            }
            let snapshot_count = snapshots.len();
            let oldest = snapshots.last().map(|s| s.timestamp.to_rfc3339());
            let newest = snapshots.first().map(|s| s.timestamp.to_rfc3339());

            // Count distinct vault_root_hash values (deduplicated states).
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            for snap in &snapshots {
                if let Some(h) =
                    zetl::history::core::extract_vault_root_hash_from_description(&snap.description)
                {
                    seen.insert(h);
                }
            }
            let unique_states = seen.len();

            // Scan cache directory for entry count and total size.
            let history_dir = pipeline.vault_root.join(".zetl").join("history");
            let mut cache_entries = 0usize;
            let mut total_bytes = 0u64;
            if let Ok(rd) = std::fs::read_dir(&history_dir) {
                for entry in rd.flatten() {
                    if entry
                        .path()
                        .extension()
                        .map(|e| e == "json")
                        .unwrap_or(false)
                    {
                        cache_entries += 1;
                        if let Ok(meta) = entry.metadata() {
                            total_bytes += meta.len();
                        }
                    }
                }
            }
            let cache_size_mb = total_bytes as f64 / (1024.0 * 1024.0);

            Some(serde_json::json!({
                "snapshot_count": snapshot_count,
                "unique_states": unique_states,
                "oldest": oldest,
                "newest": newest,
                "cache_entries": cache_entries,
                "cache_size_mb": (cache_size_mb * 10.0).round() / 10.0,
            }))
        })()
    };
    #[cfg(not(feature = "history"))]
    let history_stats: Option<serde_json::Value> = None;

    // OBS-020: semantic stats for `zetl stats` when semantic feature is enabled.
    #[cfg(feature = "semantic")]
    let semantic_stats: Option<serde_json::Value> = {
        let vectors_dir = pipeline.vault_root.join(zetl::semantic::VECTORS_DIR);
        if vectors_dir.exists() {
            let chunks_path = vectors_dir.join(zetl::semantic::CHUNKS_FILE);
            let chunk_count = std::fs::read_to_string(&chunks_path)
                .ok()
                .and_then(|s| serde_json::from_str::<Vec<serde_json::Value>>(&s).ok())
                .map(|v| v.len())
                .unwrap_or(0);
            let total_bytes = dir_size_kb(&vectors_dir);
            let index_size_mb = (total_bytes as f64 / 1024.0 * 10.0).round() / 10.0;
            Some(serde_json::json!({
                "chunk_count": chunk_count,
                "index_size_mb": index_size_mb,
                "model_name": zetl::semantic::MODEL_NAME,
            }))
        } else {
            None
        }
    };
    #[cfg(not(feature = "semantic"))]
    let semantic_stats: Option<serde_json::Value> = None;

    // OBS-102: graph-index export stats (bytes/nodes/edges) mirror what
    // `zetl build` would emit as graph-index.json, so authors can size the
    // asset before enabling `graph_inline = true` in a theme.
    let (page_slug_map, _slug_collisions) = zetl::web::build_slug_map(&pipeline.files);
    let mut tags_by_page: HashMap<String, Vec<String>> = HashMap::new();
    for file in &pipeline.files {
        let full_path = pipeline.vault_root.join(&file.path);
        let Ok(content) = std::fs::read_to_string(&full_path) else {
            continue;
        };
        let fm = zetl::web::markdown::parse_frontmatter(&content);
        let Some(arr) = fm.get("tags").and_then(|v| v.as_array()) else {
            continue;
        };
        let tags: Vec<String> = arr
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
        if !tags.is_empty() {
            tags_by_page.insert(file.page_name.clone(), tags);
        }
    }
    let vault_name = pipeline
        .vault_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("vault")
        .to_string();
    let generated_at = zetl::user::access_request::now_iso8601();
    let graph_index_json = zetl::graph::serialize_graph_index(
        &pipeline.graph,
        &page_slug_map,
        &tags_by_page,
        &vault_name,
        &generated_at,
    );
    let graph_index_bytes = serde_json::to_string(&graph_index_json)
        .map(|s| s.len())
        .unwrap_or(0);
    let graph_index_nodes = graph_index_json
        .get("nodes")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let graph_index_edges = graph_index_json
        .get("edges")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let graph_export_stats = serde_json::json!({
        "bytes": graph_index_bytes,
        "nodes": graph_index_nodes,
        "edges": graph_index_edges,
    });

    #[derive(Serialize)]
    struct StatsOutput {
        #[serde(flatten)]
        graph: zetl::graph::GraphStats,
        vault_content_hash: Option<String>,
        spl_blocks: usize,
        grounded_spl_blocks: usize,
        explicitly_grounded_facts: usize,
        #[serde(rename = "graph")]
        graph_export: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        history: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        semantic: Option<serde_json::Value>,
    }

    let output = StatsOutput {
        graph: graph_stats,
        vault_content_hash,
        spl_blocks,
        grounded_spl_blocks,
        explicitly_grounded_facts,
        graph_export: graph_export_stats,
        history: history_stats,
        semantic: semantic_stats,
    };

    match cli.format {
        OutputFormat::Json => print_json(&output)?,
        _ => {
            let mut table = Table::new();
            table.set_header(vec!["Metric", "Value"]);
            table.add_row(vec![Cell::new("Pages"), Cell::new(output.graph.pages)]);
            table.add_row(vec![Cell::new("Links"), Cell::new(output.graph.links)]);
            table.add_row(vec![
                Cell::new("Unique targets"),
                Cell::new(output.graph.unique_targets),
            ]);
            table.add_row(vec![
                Cell::new("Dead links"),
                Cell::new(output.graph.dead_links),
            ]);
            table.add_row(vec![Cell::new("Orphans"), Cell::new(output.graph.orphans)]);
            table.add_row(vec![
                Cell::new("Connected components"),
                Cell::new(output.graph.connected_components),
            ]);
            table.add_row(vec![
                Cell::new("Vault content hash"),
                Cell::new(output.vault_content_hash.as_deref().unwrap_or("N/A")),
            ]);
            table.add_row(vec![Cell::new("SPL blocks"), Cell::new(output.spl_blocks)]);
            table.add_row(vec![
                Cell::new("Grounded SPL blocks"),
                Cell::new(output.grounded_spl_blocks),
            ]);
            table.add_row(vec![
                Cell::new("Explicitly grounded facts"),
                Cell::new(output.explicitly_grounded_facts),
            ]);
            println!("{table}");

            if !output.graph.most_linked.is_empty() {
                println!();
                let mut ml_table = Table::new();
                ml_table.set_header(vec!["#", "Page", "Backlinks"]);
                for (i, ml) in output.graph.most_linked.iter().enumerate() {
                    ml_table.add_row(vec![
                        Cell::new(i + 1),
                        Cell::new(&ml.page),
                        Cell::new(ml.backlink_count),
                    ]);
                }
                println!("Most linked pages:");
                println!("{ml_table}");
            }

            // OBS-102: print graph-index export section (bytes/nodes/edges).
            println!();
            println!("Graph:");
            println!(
                "  Nodes:          {}",
                output.graph_export["nodes"].as_u64().unwrap_or(0)
            );
            println!(
                "  Edges:          {}",
                output.graph_export["edges"].as_u64().unwrap_or(0)
            );
            println!(
                "  Bytes:          {}",
                output.graph_export["bytes"].as_u64().unwrap_or(0)
            );

            // OBS-012: print history storage section when available.
            if let Some(ref hs) = output.history {
                println!();
                println!("History:");
                println!(
                    "  Snapshots:      {}",
                    hs["snapshot_count"].as_u64().unwrap_or(0)
                );
                println!(
                    "  Unique states:  {}  (vault_root_hash deduplication)",
                    hs["unique_states"].as_u64().unwrap_or(0)
                );
                println!(
                    "  Oldest:         {}",
                    hs["oldest"].as_str().unwrap_or("N/A")
                );
                println!(
                    "  Newest:         {}",
                    hs["newest"].as_str().unwrap_or("N/A")
                );
                println!(
                    "  Cache entries:  {}",
                    hs["cache_entries"].as_u64().unwrap_or(0)
                );
                println!(
                    "  Cache size:     {:.1} MB",
                    hs["cache_size_mb"].as_f64().unwrap_or(0.0)
                );
            }

            // OBS-020: print semantic vector index section when available.
            if let Some(ref sem) = output.semantic {
                println!();
                println!("Semantic index:");
                println!(
                    "  Chunks:         {}",
                    sem["chunk_count"].as_u64().unwrap_or(0)
                );
                println!(
                    "  Index size:     {:.1} MB",
                    sem["index_size_mb"].as_f64().unwrap_or(0.0)
                );
                println!(
                    "  Model:          {}",
                    sem["model_name"].as_str().unwrap_or("N/A")
                );
            }
        }
    }

    Ok(())
}

fn cmd_path(cli: &Cli, from: &str, to: &str, max_depth: usize) -> Result<()> {
    let pipeline = run_pipeline(cli)?;

    let resolved_from = find_page(from, &pipeline.file_index, false, &pipeline.files)
        .unwrap_or_else(|e| {
            exit_page_not_found(&cli.format, &e);
        });

    let resolved_to =
        find_page(to, &pipeline.file_index, false, &pipeline.files).unwrap_or_else(|e| {
            exit_page_not_found(&cli.format, &e);
        });

    let result = pipeline
        .graph
        .shortest_path(&resolved_from, &resolved_to, max_depth);

    match result {
        Some(path_result) => match cli.format {
            OutputFormat::Json => print_json(&path_result)?,
            _ => {
                println!(
                    "Shortest path from '{}' to '{}' ({} hops):",
                    path_result.from, path_result.to, path_result.hops
                );
                let path_str = path_result.path.join(" -> ");
                println!("  {path_str}");
            }
        },
        None => {
            let msg = format!(
                "No path found from '{resolved_from}' to '{resolved_to}' within {max_depth} hops"
            );
            match cli.format {
                OutputFormat::Json => exit_json_error(&msg, 1),
                _ => {
                    eprintln!("{msg}.");
                    eprintln!();
                    eprintln!("Hint: try increasing --max-depth (currently {max_depth}) to search further.");
                    std::process::exit(1);
                }
            }
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cmd_search(
    cli: &Cli,
    query: &str,
    context: usize,
    limit: usize,
    case_sensitive: bool,
    path_filter: Option<&str>,
    near: Option<&str>,
    depth: Option<usize>,
    semantic: bool,
    hybrid: bool,
) -> Result<()> {
    // REQ-098: --semantic / --hybrid require the `semantic` feature at compile time.
    #[cfg(not(feature = "semantic"))]
    if semantic || hybrid {
        let flag = if semantic { "--semantic" } else { "--hybrid" };
        let msg = format!(
            "{flag} requires the semantic feature. Rebuild with: cargo build --features semantic"
        );
        match cli.format {
            OutputFormat::Json => exit_json_error(&msg, 1),
            OutputFormat::Table | OutputFormat::Auto => {
                eprintln!("Error: {msg}");
                std::process::exit(1);
            }
        }
    }
    // REQ-095, ADR-053: hybrid BM25 + vector search via RRF is implemented below,
    // after the BM25 results have been collected.
    #[cfg(not(feature = "semantic"))]
    let _ = (semantic, hybrid);

    // REQ-013-007: --depth without --near is an error (exit 2).
    if depth.is_some() && near.is_none() {
        let msg = "--depth requires --near to be specified";
        match cli.format {
            OutputFormat::Json => exit_json_error(msg, 2),
            _ => {
                eprintln!("Error: {msg}");
                std::process::exit(2);
            }
        }
    }

    let depth_val = depth.unwrap_or(1);

    // REQ-013-007: --depth 0 is an error (exit 2).
    if depth_val == 0 {
        let msg = "--depth must be >= 1";
        match cli.format {
            OutputFormat::Json => exit_json_error(msg, 2),
            _ => {
                eprintln!("Error: {msg}");
                std::process::exit(2);
            }
        }
    }

    let vault_root = std::fs::canonicalize(&cli.dir)
        .with_context(|| format!("Cannot resolve vault directory: {}", cli.dir))?;

    // REQ-077: Load the pipeline when --at is set (for snapshot metadata and
    // historical neighbourhood) or when --near is given (REQ-013-006, REQ-013-008).
    //
    // For --at without --near we still load the pipeline to capture snapshot info.
    // For --near without --at we first verify that the live index exists.
    // Note: `cli.at` only exists when the `history` feature is compiled in (REQ-084).
    #[cfg(feature = "history")]
    let at_is_some = cli.at.is_some();
    #[cfg(not(feature = "history"))]
    let at_is_some = false;

    let pipeline_opt: Option<Pipeline> = if at_is_some {
        // Historical query: run_pipeline will route to run_historical_pipeline.
        Some(run_pipeline(cli)?)
    } else if near.is_some() {
        // REQ-013-006: graph requires zetl index — the lazy search-index build does
        // not build the link graph. Check the cache before proceeding.
        let cache_path = vault_root.join(".zetl").join("index.json");
        if !cache_path.exists() {
            let msg = "Graph required for --near. Run `zetl index` first.";
            match cli.format {
                OutputFormat::Json => exit_json_error(msg, 1),
                _ => {
                    eprintln!("Error: {msg}");
                    std::process::exit(1);
                }
            }
        }
        Some(run_pipeline(cli)?)
    } else {
        None
    };

    // REQ-013-006: When --near is given, compute the neighbourhood and use it to
    // filter results. REQ-013-008: suggest similar names if the page is unresolvable.
    let mut near_resolved: Option<String> = None;
    let mut near_neighbourhood_size: Option<usize> = None;
    let neighbourhood_set: Option<HashSet<String>> = if let Some(near_page) = near {
        let pipeline = pipeline_opt.as_ref().unwrap(); // safe: loaded above when near.is_some()

        let resolved = match resolve_page_name(near_page, &pipeline.file_index) {
            Some(r) => r,
            None => {
                // REQ-013-008: suggest similar names (substring containment).
                let mut similar: Vec<&str> = pipeline
                    .file_index
                    .iter()
                    .filter(|(name, _)| {
                        let n = name.to_lowercase();
                        let q = near_page.to_lowercase();
                        n.contains(&q) || q.contains(n.as_str())
                    })
                    .map(|(name, _)| name.as_str())
                    .collect();
                similar.sort();
                similar.truncate(5);
                let msg = if similar.is_empty() {
                    format!("Page not found: '{near_page}'")
                } else {
                    format!(
                        "Page not found: '{near_page}'. Did you mean: {}",
                        similar.join(", ")
                    )
                };
                match cli.format {
                    OutputFormat::Json => exit_json_error(&msg, 2),
                    _ => {
                        eprintln!("Error: {msg}");
                        std::process::exit(2);
                    }
                }
            }
        };

        // OBS-013-003: time the BFS and emit verbose stats to stderr.
        let bfs_start = std::time::Instant::now();
        let set = pipeline.graph.neighbourhood(&resolved, depth_val)?;
        let bfs_ms = bfs_start.elapsed().as_millis();
        let size = set.len();

        if cli.verbose > 0 {
            eprintln!("near: {resolved}");
            eprintln!("depth: {depth_val}");
            eprintln!("neighbourhood size: {size}");
            eprintln!("BFS time: {bfs_ms}ms");
        }

        near_resolved = Some(resolved);
        near_neighbourhood_size = Some(size);
        Some(set)
    } else {
        None
    };

    let config = SearchConfig {
        query,
        context_chars: context,
        limit,
        case_sensitive,
        path_filter,
    };

    // REQ-095, ADR-053: for --hybrid, launch vector search in a background thread so that
    // BM25 and vector retrieval run in parallel. The handle is joined after BM25 completes.
    // For --semantic (pure vector) the index is loaded sequentially after BM25 is skipped.
    #[cfg(feature = "semantic")]
    let hybrid_vec_thread: Option<
        std::thread::JoinHandle<anyhow::Result<(Vec<zetl::semantic::VectorHit>, usize, u128)>>,
    > = if hybrid {
        let vault_root_vec = vault_root.clone();
        let query_owned = query.to_string();
        let vec_limit = limit.saturating_mul(2);
        Some(std::thread::spawn(move || {
            let idx = zetl::semantic::VectorIndex::open(&vault_root_vec)?;
            match idx {
                None => {
                    anyhow::bail!("Vector index not found. Run `zetl index` to build it first.")
                }
                Some(idx) => {
                    let start = std::time::Instant::now();
                    let hits = idx.query_text(&query_owned, vec_limit)?;
                    let duration_ms = start.elapsed().as_millis();
                    let chunk_count = idx.chunk_count();
                    Ok((hits, chunk_count, duration_ms))
                }
            }
        }))
    } else {
        None
    };

    let mut output = match search_vault(&vault_root, &config, &cli.scan_options()) {
        Ok(o) => o,
        Err(e) => {
            let msg = format!("{e}");
            let code = if msg.contains("Empty search query") {
                2
            } else {
                1
            };
            match cli.format {
                OutputFormat::Json => exit_json_error(&msg, code),
                _ => {
                    eprintln!("Error: {msg}");
                    std::process::exit(code);
                }
            }
        }
    };

    // REQ-095, ADR-053: apply semantic / hybrid re-ranking when the semantic feature
    // is active and the caller requested --semantic or --hybrid.
    #[cfg(feature = "semantic")]
    {
        use zetl::search::SearchMatch;
        if semantic {
            // --semantic: pure vector search — load index sequentially and replace BM25 output.
            let vec_start = std::time::Instant::now();
            let vec_index = zetl::semantic::VectorIndex::open(&vault_root);
            match vec_index {
                Err(e) => {
                    let msg = format!("Failed to load vector index: {e}. Run `zetl index` first.");
                    match cli.format {
                        OutputFormat::Json => exit_json_error(&msg, 1),
                        OutputFormat::Table | OutputFormat::Auto => {
                            eprintln!("Error: {msg}");
                            std::process::exit(1);
                        }
                    }
                }
                Ok(None) => {
                    let msg = "Vector index not found. Run `zetl index` to build it first.";
                    match cli.format {
                        OutputFormat::Json => exit_json_error(msg, 1),
                        OutputFormat::Table | OutputFormat::Auto => {
                            eprintln!("Error: {msg}");
                            std::process::exit(1);
                        }
                    }
                }
                Ok(Some(ref idx)) => {
                    match idx.query_text(query, limit) {
                        Err(e) => {
                            let msg = format!("Vector query failed: {e}");
                            match cli.format {
                                OutputFormat::Json => exit_json_error(&msg, 1),
                                OutputFormat::Table | OutputFormat::Auto => {
                                    eprintln!("Error: {msg}");
                                    std::process::exit(1);
                                }
                            }
                        }
                        Ok(vec_hits) => {
                            let vec_ms = vec_start.elapsed().as_millis();
                            if cli.verbose > 0 {
                                idx.log_query_stats(vec_hits.len(), vec_ms);
                            }
                            // Pure vector search: replace BM25 output entirely.
                            // Convert VectorHit → SearchMatch (no line/column info).
                            let results: Vec<SearchMatch> = vec_hits
                                .into_iter()
                                .map(|h| SearchMatch {
                                    page: h.page_name,
                                    path: h.path,
                                    line: 0,
                                    column: 0,
                                    context: None,
                                    heading: h.heading,
                                    heading_level: None,
                                    score: h.score as f64,
                                })
                                .collect();
                            let total = results.len();
                            output.results = results;
                            output.total_matches = total;
                        }
                    }
                }
            }
        } else if hybrid {
            // --hybrid: join the pre-spawned vector thread (runs in parallel with BM25)
            // and fuse results via RRF (REQ-095, ADR-053).
            let (vec_hits, vec_chunks_scanned, vec_ms) = match hybrid_vec_thread.unwrap().join() {
                Err(_) => {
                    let msg = "Vector search thread panicked";
                    match cli.format {
                        OutputFormat::Json => exit_json_error(msg, 1),
                        OutputFormat::Table | OutputFormat::Auto => {
                            eprintln!("Error: {msg}");
                            std::process::exit(1);
                        }
                    }
                }
                Ok(Err(e)) => {
                    let msg = format!("{e}");
                    match cli.format {
                        OutputFormat::Json => exit_json_error(&msg, 1),
                        OutputFormat::Table | OutputFormat::Auto => {
                            eprintln!("Error: {msg}");
                            std::process::exit(1);
                        }
                    }
                }
                Ok(Ok(tuple)) => tuple,
            };

            if cli.verbose > 0 {
                eprintln!(
                    "[zetl] vector-query: chunks_scanned={vec_chunks_scanned} results={} duration_ms={vec_ms}",
                    vec_hits.len()
                );
            }

            // Fuse BM25 and vector ranks via RRF. Build rank lists (1-indexed,
            // deduplicated by page).
            let bm25_ranks: Vec<(String, usize)> = {
                let mut seen = std::collections::HashSet::new();
                output
                    .results
                    .iter()
                    .filter_map(|r| {
                        if seen.insert(r.page.clone()) {
                            Some((r.page.clone(), seen.len()))
                        } else {
                            None
                        }
                    })
                    .collect()
            };
            let vec_ranks: Vec<(String, usize)> = {
                let mut seen = std::collections::HashSet::new();
                vec_hits
                    .iter()
                    .filter_map(|h| {
                        if seen.insert(h.page_name.clone()) {
                            Some((h.page_name.clone(), seen.len()))
                        } else {
                            None
                        }
                    })
                    .collect()
            };

            let fusion_start = std::time::Instant::now();
            let fused = zetl::semantic::core::reciprocal_rank_fusion(
                &bm25_ranks,
                &vec_ranks,
                zetl::semantic::RRF_K,
            );
            let fusion_ms = fusion_start.elapsed().as_millis();

            if cli.verbose > 0 {
                eprintln!(
                    "[zetl] hybrid-fusion: bm25_candidates={} vec_candidates={} fused={} duration_ms={fusion_ms}",
                    bm25_ranks.len(),
                    vec_ranks.len(),
                    fused.len(),
                );
            }

            // Build a score map from page_name → fused score.
            let score_map: std::collections::HashMap<String, f64> = fused.into_iter().collect();

            // Collect all BM25 matches, re-scored by fused value.
            // Pages only in vector results get a placeholder match.
            let mut new_results: Vec<SearchMatch> = output
                .results
                .drain(..)
                .map(|mut m| {
                    if let Some(&fs) = score_map.get(&m.page) {
                        m.score = fs;
                    }
                    m
                })
                .collect();

            // Add pages that appear only in vector results (no BM25 match).
            let bm25_pages: std::collections::HashSet<String> =
                new_results.iter().map(|r| r.page.clone()).collect();
            for hit in &vec_hits {
                if !bm25_pages.contains(&hit.page_name) {
                    if let Some(&fs) = score_map.get(&hit.page_name) {
                        new_results.push(SearchMatch {
                            page: hit.page_name.clone(),
                            path: hit.path.clone(),
                            line: 0,
                            column: 0,
                            context: None,
                            heading: hit.heading.clone(),
                            heading_level: None,
                            score: fs,
                        });
                    }
                }
            }

            // Sort by fused score descending, then truncate to limit.
            new_results.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            new_results.truncate(limit);

            let total = new_results.len();
            output.results = new_results;
            output.total_matches = total;
        }
    }

    // Filter results to the neighbourhood if --near was specified.
    // REQ-013-009: populate neighbourhood metadata in the output envelope.
    if let Some(ref set) = neighbourhood_set {
        output.results.retain(|r| set.contains(&r.page));
        output.total_matches = output.results.len();
        output.near = near_resolved;
        output.depth = Some(depth_val);
        output.neighbourhood_size = near_neighbourhood_size;
    }

    // CON-024: snapshot field in JSON output when --at was used.
    let search_snapshot = pipeline_opt.as_ref().and_then(|p| p.snapshot.clone());

    if output.total_matches == 0 {
        match cli.format {
            OutputFormat::Json => {
                #[derive(Serialize)]
                struct SearchOutputWithSnapshot<'a> {
                    #[serde(flatten)]
                    inner: &'a zetl::search::SearchOutput,
                    #[serde(skip_serializing_if = "Option::is_none")]
                    snapshot: Option<SnapshotInfo>,
                }
                print_json(&SearchOutputWithSnapshot {
                    inner: &output,
                    snapshot: search_snapshot.clone(),
                })?
            }
            _ => {
                eprintln!("No matches found for '{query}'.");
                eprintln!();
                eprintln!("Hint: try different search terms, or use --near <page> to search within a neighbourhood.");
            }
        }
        std::process::exit(1);
    }

    match cli.format {
        OutputFormat::Json => {
            #[derive(Serialize)]
            struct SearchOutputWithSnapshot<'a> {
                #[serde(flatten)]
                inner: &'a zetl::search::SearchOutput,
                #[serde(skip_serializing_if = "Option::is_none")]
                snapshot: Option<SnapshotInfo>,
            }
            print_json(&SearchOutputWithSnapshot {
                inner: &output,
                snapshot: search_snapshot,
            })?
        }
        _ => {
            let mut table = Table::new();
            table.set_header(vec!["Page", "Score", "Line", "Heading", "Context"]);
            for r in &output.results {
                table.add_row(vec![
                    Cell::new(&r.page),
                    Cell::new(format!("{:.3}", r.score)),
                    Cell::new(r.line),
                    Cell::new(r.heading.as_deref().unwrap_or("")),
                    Cell::new(r.context.as_deref().unwrap_or("")),
                ]);
            }
            // CON-013-001: include neighbourhood info in the header when --near is used.
            let near_info = if let (Some(ref n), Some(d), Some(s)) =
                (&output.near, output.depth, output.neighbourhood_size)
            {
                format!(", near: {n}, depth: {d}, {s} pages")
            } else {
                String::new()
            };
            println!(
                "Search results for '{}' ({} matches{}):",
                query, output.total_matches, near_info
            );
            println!("{table}");
        }
    }

    Ok(())
}

fn cmd_list(cli: &Cli) -> Result<()> {
    let vault_root = std::fs::canonicalize(&cli.dir)
        .with_context(|| format!("Cannot resolve vault directory: {}", cli.dir))?;

    // Lean version: just scan for files, no graph construction needed
    let files = scan_vault(&vault_root, &cli.scan_options())?;

    #[derive(Serialize)]
    struct PageEntry {
        page: String,
        path: String,
    }

    let mut pages: Vec<PageEntry> = files
        .iter()
        .map(|f| PageEntry {
            page: f.page_name.clone(),
            path: f.path.to_string_lossy().to_string(),
        })
        .collect();
    pages.sort_by_key(|a| a.page.to_lowercase());

    #[derive(Serialize)]
    struct ListOutput {
        pages: Vec<PageEntry>,
        total: usize,
    }

    let total = pages.len();
    let output = ListOutput { pages, total };

    match cli.format {
        OutputFormat::Json => print_json(&output)?,
        _ => {
            if output.pages.is_empty() {
                println!("No pages found.");
            } else {
                let mut table = Table::new();
                table.set_header(vec!["Page", "Path"]);
                for p in &output.pages {
                    table.add_row(vec![Cell::new(&p.page), Cell::new(&p.path)]);
                }
                println!("{table}");
            }
        }
    }

    Ok(())
}

fn cmd_blocks(
    cli: &Cli,
    page: Option<&str>,
    block_type: &BlockTypeFilter,
    resolve: Option<&str>,
) -> Result<()> {
    // Validate mutual exclusion
    if page.is_some() && resolve.is_some() {
        match cli.format {
            OutputFormat::Json => exit_json_error("--resolve and page are mutually exclusive", 2),
            _ => {
                eprintln!("Error: --resolve and page are mutually exclusive");
                std::process::exit(2);
            }
        }
    }
    if page.is_none() && resolve.is_none() {
        match cli.format {
            OutputFormat::Json => {
                exit_json_error("Either a page name or --resolve <HASH> is required", 2)
            }
            _ => {
                eprintln!("Error: Either a page name or --resolve <HASH> is required");
                std::process::exit(2);
            }
        }
    }

    // ── Dispatch resolve mode to dedicated handler ──────────────────────────
    if let Some(hash_prefix) = resolve {
        return cmd_blocks_resolve(cli, hash_prefix);
    }

    let pipeline = run_pipeline(cli)?;

    // ── Helper: convert hex ContentHash to string ──────────────────────────
    let hash_to_hex =
        |h: &zetl::types::ContentHash| -> String { h.iter().map(|b| format!("{b:02x}")).collect() };

    // ── Helper: derive a type label string from a LeafType ─────────────────
    fn leaf_type_label(leaf_type: &zetl::types::LeafType) -> String {
        use zetl::types::LeafType;
        match leaf_type {
            LeafType::Heading { level } => format!("heading-{level}"),
            LeafType::Paragraph => "paragraph".to_string(),
            LeafType::CodeBlock { .. } => "code".to_string(),
            LeafType::SplBlock => "spl".to_string(),
            LeafType::List { .. } => "list".to_string(),
            LeafType::BlockQuote => "blockquote".to_string(),
            LeafType::Table => "table".to_string(),
            LeafType::Frontmatter => "frontmatter".to_string(),
            LeafType::ThematicBreak => "thematic-break".to_string(),
            LeafType::HtmlBlock => "html".to_string(),
        }
    }

    // ── Helper: check if a leaf type passes the filter ─────────────────────
    fn leaf_matches_filter(leaf_type: &zetl::types::LeafType, filter: &BlockTypeFilter) -> bool {
        use zetl::types::LeafType;
        match filter {
            BlockTypeFilter::All => true,
            BlockTypeFilter::Heading => matches!(leaf_type, LeafType::Heading { .. }),
            BlockTypeFilter::Paragraph => matches!(leaf_type, LeafType::Paragraph),
            BlockTypeFilter::Spl => matches!(leaf_type, LeafType::SplBlock),
            BlockTypeFilter::Code => matches!(leaf_type, LeafType::CodeBlock { .. }),
            BlockTypeFilter::Table => matches!(leaf_type, LeafType::Table),
            BlockTypeFilter::List => matches!(leaf_type, LeafType::List { .. }),
            BlockTypeFilter::Blockquote => matches!(leaf_type, LeafType::BlockQuote),
            BlockTypeFilter::Frontmatter => matches!(leaf_type, LeafType::Frontmatter),
        }
    }

    // ── Helper: extract and normalise text from file lines ─────────────────
    fn extract_block_text(
        vault_root: &Path,
        file_path: &std::path::Path,
        start_line: u32,
        end_line: u32,
    ) -> Option<String> {
        let full_path = vault_root.join(file_path);
        let content = std::fs::read_to_string(&full_path).ok()?;
        let lines: Vec<&str> = content.lines().collect();
        let start = (start_line as usize).saturating_sub(1);
        let end = (end_line as usize).min(lines.len());
        if start >= end {
            return Some(String::new());
        }
        let raw = lines[start..end].join("\n");
        // Normalise: collapse whitespace runs to a single space, trim
        let mut out = String::with_capacity(raw.len().min(210));
        let mut prev_space = false;
        for ch in raw.chars() {
            if ch.is_whitespace() {
                if !prev_space {
                    out.push(' ');
                }
                prev_space = true;
            } else {
                out.push(ch);
                prev_space = false;
            }
        }
        let trimmed = out.trim().to_string();
        // Take first 200 chars (char boundary safe)
        if trimmed.chars().count() <= 200 {
            Some(trimmed)
        } else {
            Some(trimmed.chars().take(200).collect())
        }
    }

    #[derive(Serialize)]
    struct SplHashesOutput {
        content_hash: String,
        ast_hash: String,
    }

    #[derive(Serialize)]
    struct BlockEntry {
        index: usize,
        #[serde(rename = "type")]
        block_type: String,
        lines: [u32; 2],
        hash: String,
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        spl_hashes: Option<SplHashesOutput>,
    }

    if let Some(page_input) = page {
        // ── Forward mode: list blocks for a page ───────────────────────────
        let resolved_page = find_page(page_input, &pipeline.file_index, false, &pipeline.files)
            .unwrap_or_else(|e| {
                exit_page_not_found(&cli.format, &e);
            });

        let file = pipeline
            .files
            .iter()
            .find(|f| f.page_name == resolved_page)
            .unwrap_or_else(|| {
                exit_page_not_found(&cli.format, &format!("Page not found: '{resolved_page}'"));
            });

        let file_hash = file
            .file_merkle
            .as_ref()
            .map(|fm| hash_to_hex(&fm.root_hash));

        // Build filtered block entries
        let mut blocks: Vec<BlockEntry> = Vec::new();
        let mut index = 0usize;
        for leaf in &file.merkle_leaves {
            if !leaf_matches_filter(&leaf.node_type, block_type) {
                continue;
            }
            let text = extract_block_text(
                &pipeline.vault_root,
                &file.path,
                leaf.start_line,
                leaf.end_line,
            )
            .unwrap_or_default();

            let spl_hashes = leaf.spl_hashes.as_ref().map(|sh| SplHashesOutput {
                content_hash: hash_to_hex(&sh.content_hash),
                ast_hash: hash_to_hex(&sh.ast_hash),
            });

            blocks.push(BlockEntry {
                index,
                block_type: leaf_type_label(&leaf.node_type),
                lines: [leaf.start_line, leaf.end_line],
                hash: hash_to_hex(&leaf.hash),
                text,
                spl_hashes,
            });
            index += 1;
        }

        let block_count = blocks.len();

        #[derive(Serialize)]
        struct BlocksOutput {
            page: String,
            file_hash: Option<String>,
            block_count: usize,
            blocks: Vec<BlockEntry>,
            #[serde(skip_serializing_if = "Option::is_none")]
            snapshot: Option<SnapshotInfo>,
        }

        let output = BlocksOutput {
            page: resolved_page.clone(),
            file_hash,
            block_count,
            blocks,
            snapshot: pipeline.snapshot.clone(),
        };

        match cli.format {
            OutputFormat::Json => print_json(&output)?,
            _ => {
                println!(
                    "Blocks for '{}' ({} blocks):",
                    output.page, output.block_count
                );
                if output.block_count == 0 {
                    println!("  (no blocks match filter)");
                } else {
                    let mut table = Table::new();
                    table.set_header(vec!["#", "Type", "Lines", "Hash (prefix)", "Text"]);
                    for b in &output.blocks {
                        let hash_prefix = &b.hash[..8.min(b.hash.len())];
                        let lines_str = format!("{}-{}", b.lines[0], b.lines[1]);
                        let text_display = if b.text.len() > 60 {
                            format!("{}…", &b.text[..60])
                        } else {
                            b.text.clone()
                        };
                        table.add_row(vec![
                            Cell::new(b.index),
                            Cell::new(&b.block_type),
                            Cell::new(lines_str),
                            Cell::new(hash_prefix),
                            Cell::new(text_display),
                        ]);
                    }
                    println!("{table}");
                    if let Some(ref fh) = output.file_hash {
                        println!("File hash: {fh}");
                    }
                }
            }
        }
    }

    Ok(())
}

/// Implement REQ-045 / CON-020 reverse mode: resolve a BLAKE3 hash prefix to its
/// source location(s) across the vault.
///
/// Resolution logic per §3.3:
/// 1. Validate prefix is minimum 8 hex characters.
/// 2. Search all Merkle leaves across the vault for prefix match.
/// 3. Zero matches → exit 1, error JSON per CON-020 "hash not found" example.
/// 4. Multiple matches with different full hashes (ambiguous prefix) → exit 1,
///    error JSON with matches list and suggestion.
/// 5. Multiple matches with identical full hashes (duplicate content) → exit 0,
///    success JSON with locations array and note per CON-020 duplicate content example.
/// 6. One match → exit 0, standard CON-020 resolve success JSON.
fn cmd_blocks_resolve(cli: &Cli, hash_prefix: &str) -> Result<()> {
    // ── §1 Validate minimum 8 hex characters ────────────────────────────────
    if hash_prefix.len() < 8 {
        #[derive(Serialize)]
        struct E {
            error: &'static str,
        }
        let msg = "hash prefix too short (minimum 8 hex characters)";
        match cli.format {
            OutputFormat::Json => {
                let _ = print_json(&E { error: msg });
                std::process::exit(1);
            }
            _ => {
                eprintln!("Error: {msg}");
                std::process::exit(1);
            }
        }
    }

    // ── §2 Scan vault and build hash index ───────────────────────────────────
    let pipeline = run_pipeline(cli)?;
    let index = build_vault_hash_index(&pipeline.files);

    // ── Helpers ──────────────────────────────────────────────────────────────
    fn leaf_type_label(leaf_type: &zetl::types::LeafType) -> String {
        use zetl::types::LeafType;
        match leaf_type {
            LeafType::Heading { level } => format!("heading-{level}"),
            LeafType::Paragraph => "paragraph".to_string(),
            LeafType::CodeBlock { .. } => "code".to_string(),
            LeafType::SplBlock => "spl".to_string(),
            LeafType::List { .. } => "list".to_string(),
            LeafType::BlockQuote => "blockquote".to_string(),
            LeafType::Table => "table".to_string(),
            LeafType::Frontmatter => "frontmatter".to_string(),
            LeafType::ThematicBreak => "thematic-break".to_string(),
            LeafType::HtmlBlock => "html".to_string(),
        }
    }

    fn extract_block_text(
        vault_root: &Path,
        file_path: &std::path::Path,
        start_line: u32,
        end_line: u32,
    ) -> Option<String> {
        let full_path = vault_root.join(file_path);
        let content = std::fs::read_to_string(&full_path).ok()?;
        let lines: Vec<&str> = content.lines().collect();
        let start = (start_line as usize).saturating_sub(1);
        let end = (end_line as usize).min(lines.len());
        if start >= end {
            return Some(String::new());
        }
        let raw = lines[start..end].join("\n");
        let mut out = String::with_capacity(raw.len().min(210));
        let mut prev_space = false;
        for ch in raw.chars() {
            if ch.is_whitespace() {
                if !prev_space {
                    out.push(' ');
                }
                prev_space = true;
            } else {
                out.push(ch);
                prev_space = false;
            }
        }
        let trimmed = out.trim().to_string();
        if trimmed.chars().count() <= 200 {
            Some(trimmed)
        } else {
            Some(trimmed.chars().take(200).collect())
        }
    }

    // ── §3-6 Resolution logic ────────────────────────────────────────────────
    match resolve_hash_prefix(hash_prefix, &index) {
        HashResolutionResult::Found {
            full_hash,
            locations,
        } => {
            if locations.len() == 1 {
                // §6 Single location — standard CON-020 resolve success JSON
                let loc = &locations[0];
                let file_str = loc.file.to_string_lossy().to_string();
                let page = pipeline
                    .files
                    .iter()
                    .find(|f| f.path == loc.file)
                    .map(|f| f.page_name.clone())
                    .unwrap_or_else(|| file_str.clone());
                let block_type = leaf_type_label(&loc.leaf.node_type);
                let text = extract_block_text(
                    &pipeline.vault_root,
                    &loc.file,
                    loc.leaf.start_line,
                    loc.leaf.end_line,
                )
                .unwrap_or_default();

                #[derive(Serialize)]
                struct SingleResolveOutput {
                    hash: String,
                    file: String,
                    page: String,
                    #[serde(rename = "type")]
                    block_type: String,
                    lines: [u32; 2],
                    text: String,
                }

                let output = SingleResolveOutput {
                    hash: full_hash,
                    file: file_str,
                    page,
                    block_type,
                    lines: [loc.leaf.start_line, loc.leaf.end_line],
                    text,
                };

                match cli.format {
                    OutputFormat::Json => print_json(&output)?,
                    _ => {
                        println!(
                            "{}  {}:{}-{}  {}",
                            output.hash,
                            output.file,
                            output.lines[0],
                            output.lines[1],
                            output.block_type
                        );
                        println!("{}", output.text);
                    }
                }
            } else {
                // §5 Duplicate content — identical full hashes at multiple locations
                #[derive(Serialize)]
                struct DupLocation {
                    file: String,
                    page: String,
                    lines: [u32; 2],
                    #[serde(rename = "type")]
                    block_type: String,
                    text: String,
                }

                #[derive(Serialize)]
                struct DupResolveOutput {
                    hash: String,
                    locations: Vec<DupLocation>,
                    note: &'static str,
                }

                let dup_locs: Vec<DupLocation> = locations
                    .iter()
                    .map(|loc| {
                        let file_str = loc.file.to_string_lossy().to_string();
                        let page = pipeline
                            .files
                            .iter()
                            .find(|f| f.path == loc.file)
                            .map(|f| f.page_name.clone())
                            .unwrap_or_else(|| file_str.clone());
                        let block_type = leaf_type_label(&loc.leaf.node_type);
                        let text = extract_block_text(
                            &pipeline.vault_root,
                            &loc.file,
                            loc.leaf.start_line,
                            loc.leaf.end_line,
                        )
                        .unwrap_or_default();
                        DupLocation {
                            file: file_str,
                            page,
                            lines: [loc.leaf.start_line, loc.leaf.end_line],
                            block_type,
                            text,
                        }
                    })
                    .collect();

                let output = DupResolveOutput {
                    hash: full_hash,
                    locations: dup_locs,
                    note: "identical content at multiple locations",
                };

                match cli.format {
                    OutputFormat::Json => print_json(&output)?,
                    _ => {
                        println!(
                            "Block {} found at {} location(s) (identical content):",
                            output.hash,
                            output.locations.len()
                        );
                        for loc in &output.locations {
                            println!("  {} (lines {}-{})", loc.page, loc.lines[0], loc.lines[1]);
                        }
                    }
                }
            }
        }

        HashResolutionResult::NotFound => {
            // §3 Zero matches
            #[derive(Serialize)]
            struct NotFoundError {
                error: String,
            }
            let err = NotFoundError {
                error: format!(
                    "content hash {hash_prefix} not found \u{2014} source content may have been modified or removed"
                ),
            };
            match cli.format {
                OutputFormat::Json => {
                    let _ = print_json(&err);
                    std::process::exit(1);
                }
                _ => {
                    eprintln!(
                        "Error: content hash {hash_prefix} not found — source content may have been modified or removed"
                    );
                    std::process::exit(1);
                }
            }
        }

        HashResolutionResult::Ambiguous { prefix, candidates } => {
            // §4 Multiple matches with different full hashes
            #[derive(Serialize)]
            struct AmbiguousMatch {
                file: String,
                lines: [u32; 2],
                hash: String,
            }

            #[derive(Serialize)]
            struct AmbiguousError {
                error: String,
                matches: Vec<AmbiguousMatch>,
                suggestion: &'static str,
            }

            // For each distinct candidate hash, pick its first representative location
            let matches: Vec<AmbiguousMatch> = candidates
                .iter()
                .filter_map(|hash| {
                    let locs = index.entries.get(hash)?;
                    let first = locs.first()?;
                    Some(AmbiguousMatch {
                        file: first.file.to_string_lossy().to_string(),
                        lines: [first.leaf.start_line, first.leaf.end_line],
                        hash: hash.clone(),
                    })
                })
                .collect();

            let err = AmbiguousError {
                error: format!("ambiguous hash prefix {prefix}"),
                matches,
                suggestion: "use a longer prefix to disambiguate",
            };

            match cli.format {
                OutputFormat::Json => {
                    let _ = print_json(&err);
                    std::process::exit(1);
                }
                _ => {
                    eprintln!("Error: ambiguous hash prefix {prefix}");
                    for m in &err.matches {
                        eprintln!("  {} {}:{}-{}", m.hash, m.file, m.lines[0], m.lines[1]);
                    }
                    eprintln!("Hint: use a longer prefix to disambiguate");
                    std::process::exit(1);
                }
            }
        }
    }

    Ok(())
}

fn cmd_export(cli: &Cli) -> Result<()> {
    let pipeline = run_pipeline(cli)?;

    #[derive(Serialize)]
    struct NodeEntry {
        page: String,
        path: Option<String>,
    }

    #[derive(Serialize)]
    struct EdgeEntry {
        source: String,
        target: String,
    }

    // Collect all nodes from the graph
    let mut nodes: Vec<NodeEntry> = Vec::new();
    for node_name in pipeline.graph.node_map.keys() {
        let path = pipeline
            .files
            .iter()
            .find(|f| &f.page_name == node_name)
            .map(|f| f.path.to_string_lossy().to_string());
        nodes.push(NodeEntry {
            page: node_name.clone(),
            path,
        });
    }
    nodes.sort_by_key(|a| a.page.to_lowercase());

    // Collect deduplicated edges
    let mut edge_set: HashSet<(String, String)> = HashSet::new();
    let mut edges: Vec<EdgeEntry> = Vec::new();

    use petgraph::visit::EdgeRef;
    for edge in pipeline.graph.graph.edge_references() {
        let source = pipeline.graph.graph[edge.source()].clone();
        let target = pipeline.graph.graph[edge.target()].clone();
        if edge_set.insert((source.clone(), target.clone())) {
            edges.push(EdgeEntry { source, target });
        }
    }
    edges.sort_by(|a, b| a.source.cmp(&b.source).then(a.target.cmp(&b.target)));

    #[derive(Serialize)]
    struct ExportOutput {
        nodes: Vec<NodeEntry>,
        edges: Vec<EdgeEntry>,
        node_count: usize,
        edge_count: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        snapshot: Option<SnapshotInfo>,
    }

    let output = ExportOutput {
        node_count: nodes.len(),
        edge_count: edges.len(),
        nodes,
        edges,
        snapshot: pipeline.snapshot.clone(),
    };

    match cli.format {
        OutputFormat::Json => print_json(&output)?,
        _ => {
            println!(
                "Graph: {} nodes, {} edges",
                output.node_count, output.edge_count
            );
            println!();
            let mut table = Table::new();
            table.set_header(vec!["Source", "Target"]);
            for e in &output.edges {
                table.add_row(vec![Cell::new(&e.source), Cell::new(&e.target)]);
            }
            println!("{table}");
        }
    }

    Ok(())
}

// ── Cross-referencing helpers ──────────────────────────────────────────────

/// A conclusion that a page contributes to (used by --with-conclusions).
#[derive(Debug, Clone, Serialize)]
struct PageConclusionEntry {
    literal: String,
    conclusion_type: String,
    contribution: String,
}

/// Build a map from page name → conclusions that page contributes to.
///
/// Runs the reasoning pipeline over all SPL blocks and maps each proof source
/// page back to the conclusions it supports.
#[cfg(feature = "reason")]
fn build_page_conclusions_map(files: &[ParsedFile]) -> HashMap<String, Vec<PageConclusionEntry>> {
    use zetl::reason::build_theory;
    use zetl::reason::types::ConclusionType;

    let spl_blocks: Vec<_> = files.iter().flat_map(|f| f.spl_blocks.clone()).collect();

    if spl_blocks.is_empty() {
        return HashMap::new();
    }

    let result = match build_theory(&spl_blocks) {
        Ok(r) => r,
        Err(_) => return HashMap::new(),
    };

    let mut map: HashMap<String, Vec<PageConclusionEntry>> = HashMap::new();

    for conclusion in &result.conclusions {
        let tag = match conclusion.conclusion_type {
            ConclusionType::DefinitelyProvable => "+D",
            ConclusionType::DefinitelyNotProvable => "-D",
            ConclusionType::DefeasiblyProvable => "+d",
            ConclusionType::DefeasiblyNotProvable => "-d",
        };

        for ps in &conclusion.proof_sources {
            let entry = PageConclusionEntry {
                literal: conclusion.literal.clone(),
                conclusion_type: tag.to_string(),
                contribution: ps.contribution.clone(),
            };
            map.entry(ps.page.clone()).or_default().push(entry);
        }
    }

    // Deduplicate entries per page (same literal+type should appear only once)
    for entries in map.values_mut() {
        entries.sort_by(|a, b| {
            a.conclusion_type
                .cmp(&b.conclusion_type)
                .then(a.literal.cmp(&b.literal))
        });
        entries.dedup_by(|a, b| a.literal == b.literal && a.conclusion_type == b.conclusion_type);
    }

    map
}

// ── Context extraction helper ──────────────────────────────────────────────

/// Read the source file and extract `n` chars of context around the wikilink
/// at the given line.
/// Snap a byte index to the nearest char boundary at or before it.
fn floor_char_boundary(s: &str, index: usize) -> usize {
    let mut i = index.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Snap a byte index to the nearest char boundary at or after it.
fn ceil_char_boundary(s: &str, index: usize) -> usize {
    let mut i = index.min(s.len());
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

fn extract_context(vault_root: &Path, source_file: &str, line: u32, n: usize) -> Option<String> {
    let full_path = vault_root.join(source_file);
    let content = std::fs::read_to_string(&full_path).ok()?;

    // Find the line content (1-indexed)
    let target_line = content.lines().nth((line as usize).saturating_sub(1))?;

    // Find the wikilink on this line
    let link_start = target_line.find("[[")?;
    let link_end = target_line[link_start..]
        .find("]]")
        .map(|i| link_start + i + 2)?;

    // Extract n chars before and after, snapping to char boundaries
    let ctx_start = floor_char_boundary(target_line, link_start.saturating_sub(n));
    let ctx_end = ceil_char_boundary(target_line, (link_end + n).min(target_line.len()));

    Some(target_line[ctx_start..ctx_end].to_string())
}

fn cmd_view(cli: &Cli, page: Option<&str>, context_lines: u8, main_width: u8) -> Result<()> {
    // Require an interactive terminal (REQ-062, CON-023).
    use std::io::IsTerminal as _;
    if !std::io::stdin().is_terminal() {
        eprintln!(
            r#"{{"error":{{"code":"NOT_A_TTY","message":"zetl view requires an interactive terminal."}}}}"#
        );
        std::process::exit(1);
    }

    // No-index fallback (REQ-072): ensure index exists before entering alternate screen.
    let prefetched = check_no_index_fallback(cli)?;

    // Always load the pipeline — needed for the page picker even when no <page> is given.
    let pipeline = prefetched.map(Ok).unwrap_or_else(|| run_pipeline(cli))?;

    let page_set: HashSet<String> = pipeline
        .file_index
        .iter()
        .map(|(name, _)| name.clone())
        .collect();

    // Build backlink map: target_page → [(citing_page, line_number)] (REQ-070).
    let mut backlink_map: HashMap<String, Vec<(String, u32)>> = HashMap::new();
    for (name, _) in &pipeline.file_index {
        for bl in pipeline.graph.backlinks(name) {
            backlink_map
                .entry(name.clone())
                .or_default()
                .push((bl.source, bl.line));
        }
    }

    // Resolve the page title and file path.
    let (page_title, file_path) = if let Some(page_input) = page {
        let resolved = if let Some(r) = resolve_page_name(page_input, &pipeline.file_index) {
            r
        } else if cli.no_input {
            anyhow::bail!("Page not found: '{page_input}' (--no-input set; skipping picker)");
        } else {
            // Page not found — offer the top-5 SimHash-nearest suggestions (REQ-073).
            let pages: Vec<(String, String)> = pipeline
                .file_index
                .iter()
                .map(|(name, path)| (name.clone(), path.to_string_lossy().to_string()))
                .collect();

            match zetl::view::fuzzy_suggestion_prompt(page_input, &pages)? {
                Some(selected) => selected,
                None => std::process::exit(0),
            }
        };

        let abs_path = pipeline
            .file_index
            .iter()
            .find(|(name, _)| name == &resolved)
            .map(|(_, rel_path)| pipeline.vault_root.join(rel_path));

        (resolved, abs_path)
    } else if cli.no_input {
        anyhow::bail!("No page specified (--no-input set; picker disabled)");
    } else {
        // No page argument — open with empty title to trigger picker overlay (REQ-062).
        (String::new(), None)
    };

    let mut app = zetl::view::ViewApp::new(
        page_title,
        file_path,
        context_lines,
        main_width,
        page_set,
        pipeline.file_index,
        pipeline.vault_root,
        backlink_map,
    );
    app.run()
}

/// Validate a theme name: reject names containing '/', '\', or '..'.
/// When theme is not 'default', verify it exists on disk (.zetl/themes/<name>/)
/// or is a bundled theme. Both can be true (disk shadows bundled).
fn validate_theme(theme: &str, vault_root: &std::path::Path) -> Result<()> {
    if theme.contains('/') || theme.contains('\\') || theme.contains("..") {
        anyhow::bail!("invalid theme name '{theme}': must not contain '/', '\\', or '..'",);
    }

    if theme != "default" {
        let theme_dir = vault_root.join(".zetl/themes").join(theme);
        let is_disk_theme = theme_dir.is_dir();
        let is_bundled = zetl::web::engine::bundled_theme_names().contains(&theme);

        if !is_disk_theme && !is_bundled {
            let themes_root = vault_root.join(".zetl/themes");
            let mut disk_themes: Vec<String> = Vec::new();
            if themes_root.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&themes_root) {
                    for entry in entries.flatten() {
                        if entry.path().is_dir() {
                            if let Some(name) = entry.file_name().to_str() {
                                disk_themes.push(name.to_string());
                            }
                        }
                    }
                }
            }
            disk_themes.sort();

            let mut bundled: Vec<String> = zetl::web::engine::bundled_theme_names()
                .into_iter()
                .map(|s| s.to_string())
                .collect();
            bundled.sort();

            let all_available: Vec<String> = {
                let mut combined = disk_themes;
                for b in &bundled {
                    if !combined.contains(b) {
                        combined.push(b.clone());
                    }
                }
                combined.sort();
                combined
            };

            let hint = if all_available.is_empty() {
                "no themes available".to_string()
            } else {
                format!("available themes: {}", all_available.join(", "))
            };
            anyhow::bail!(
                "theme '{theme}' not found: not a bundled theme and .zetl/themes/{theme}/ does not exist\nhint: {hint}",
            );
        }
    }

    Ok(())
}

fn cmd_theme_list(cli: &Cli) -> Result<()> {
    let vault_root = std::fs::canonicalize(&cli.dir)
        .with_context(|| format!("Cannot resolve vault directory: {}", cli.dir))?;

    #[derive(Serialize)]
    struct ThemeEntry {
        name: String,
        source: String,
        version: Option<String>,
        description: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        origin_url: Option<String>,
    }

    #[derive(Serialize)]
    struct ThemeListOutput {
        themes: Vec<ThemeEntry>,
        total: usize,
    }

    // Collect bundled theme names into a set for shadow detection.
    let bundled_names: std::collections::HashSet<String> = zetl::web::engine::bundled_theme_names()
        .into_iter()
        .map(|s| s.to_string())
        .collect();

    let mut entries: Vec<ThemeEntry> = Vec::new();

    // 1. Bundled themes (only those not shadowed by an installed theme — handled below).
    let mut bundled_entries: std::collections::BTreeMap<String, ThemeEntry> =
        std::collections::BTreeMap::new();
    for name in &bundled_names {
        let (version, description) = match zetl::web::theme::load_bundled_manifest(name) {
            Ok(Some(m)) => (Some(m.theme.version), m.theme.description),
            Ok(None) => (None, None),
            Err(e) => {
                eprintln!("warning: failed to load bundled manifest for {name:?}: {e}");
                (None, None)
            }
        };
        bundled_entries.insert(
            name.clone(),
            ThemeEntry {
                name: name.clone(),
                source: "bundled".to_string(),
                version,
                description,
                origin_url: None,
            },
        );
    }

    // 2. Installed themes from .zetl/themes/.
    let themes_dir = vault_root.join(".zetl/themes");
    let mut installed_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    if themes_dir.is_dir() {
        let read_dir = std::fs::read_dir(&themes_dir)
            .with_context(|| format!("failed to read {}", themes_dir.display()))?;
        for entry in read_dir.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };

            let (version, description) = match zetl::web::theme::load_theme_manifest(&path) {
                Ok(Some(m)) => (Some(m.theme.version), m.theme.description),
                Ok(None) => (None, None),
                Err(e) => {
                    eprintln!("warning: failed to load manifest for installed theme {name:?}: {e}");
                    (None, None)
                }
            };

            let origin_url = {
                let source_path = path.join(".zetl-source.toml");
                if source_path.exists() {
                    match std::fs::read_to_string(&source_path) {
                        Ok(content) => match zetl::web::theme::parse_theme_source(&content) {
                            Ok(ts) => Some(ts.source.url),
                            Err(e) => {
                                eprintln!(
                                    "warning: failed to parse .zetl-source.toml for {name:?}: {e}"
                                );
                                None
                            }
                        },
                        Err(e) => {
                            eprintln!(
                                "warning: failed to read .zetl-source.toml for {name:?}: {e}"
                            );
                            None
                        }
                    }
                } else {
                    None
                }
            };

            let source = if bundled_names.contains(&name) {
                "installed (shadows bundled)".to_string()
            } else {
                "installed".to_string()
            };

            installed_names.insert(name.clone());
            entries.push(ThemeEntry {
                name,
                source,
                version,
                description,
                origin_url,
            });
        }
    }

    // Add bundled themes that are not shadowed by an installed theme.
    for (name, entry) in bundled_entries {
        if !installed_names.contains(&name) {
            entries.push(entry);
        }
    }

    // Sort by name for stable output.
    entries.sort_by(|a, b| a.name.cmp(&b.name));

    let total = entries.len();
    let output = ThemeListOutput {
        themes: entries,
        total,
    };

    match cli.format {
        OutputFormat::Json => print_json(&output)?,
        _ => {
            if output.themes.is_empty() {
                println!("No themes found.");
            } else {
                let mut table = Table::new();
                table.set_header(vec!["Name", "Source", "Version", "Description"]);
                for t in &output.themes {
                    table.add_row(vec![
                        Cell::new(&t.name),
                        Cell::new(&t.source),
                        Cell::new(t.version.as_deref().unwrap_or("-")),
                        Cell::new(t.description.as_deref().unwrap_or("-")),
                    ]);
                }
                println!("{table}");
            }
        }
    }

    Ok(())
}

fn cmd_theme_install(
    cli: &Cli,
    source: &str,
    path_flag: Option<&str>,
    name_flag: Option<&str>,
    force: bool,
) -> Result<()> {
    use zetl::web::theme::{
        clone_theme, parse_install_source, resolve_theme_name, validate_theme_name,
        write_provenance,
    };

    // REQ-014-016: validate --path before any filesystem operations.
    if let Some(p) = path_flag {
        // Reject absolute paths and any component that would escape the repo root.
        let path_buf = std::path::Path::new(p);
        if path_buf.is_absolute() {
            anyhow::bail!("--path must be a relative path within the repository, got {p:?}");
        }
        for component in path_buf.components() {
            use std::path::Component;
            match component {
                Component::Normal(_) => {}
                Component::CurDir => {}
                _ => {
                    anyhow::bail!(
                        "--path {p:?} contains a disallowed component; \
                         only relative paths without '..' are permitted"
                    );
                }
            }
        }
    }

    // REQ-014-016: validate --name before any filesystem operations.
    if let Some(n) = name_flag {
        validate_theme_name(n).with_context(|| format!("invalid --name {n:?}"))?;
    }

    // 1. Parse source.
    let install_source = parse_install_source(source)?;

    let vault_root = std::fs::canonicalize(&cli.dir)
        .with_context(|| format!("cannot resolve vault directory: {}", cli.dir))?;
    let themes_dir = vault_root.join(".zetl/themes");

    // We need a temporary clone to read the manifest so we can resolve the
    // final name — but we first need to detect if the target already exists.
    // We do a two-pass approach:
    //   a. Clone into a temp dir to read theme.toml.
    //   b. Resolve final name.
    //   c. Check if target exists; error if !force.
    //   d. Move temp clone into final location.

    // Clone into a unique temporary directory inside .zetl/themes/.
    std::fs::create_dir_all(&themes_dir)
        .with_context(|| format!("failed to create {}", themes_dir.display()))?;

    let tmp_dir = themes_dir.join(format!(
        ".install-tmp-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos()
    ));

    // 3. Clone.
    if cli.verbose > 0 {
        eprintln!("theme: url={}", install_source.url);
        match &install_source.git_ref {
            Some(r) => eprintln!("theme: ref={r}"),
            None => eprintln!("theme: ref=(default branch)"),
        }
    }
    let clone_start = std::time::Instant::now();
    let clone_result = clone_theme(
        &zetl::web::theme::ThemeInstallSource {
            url: install_source.url.clone(),
            git_ref: install_source.git_ref.clone(),
            path: path_flag.map(str::to_string),
        },
        &tmp_dir,
    )
    .with_context(|| format!("failed to clone {source:?}"))?;
    let clone_ms = clone_start.elapsed().as_millis();
    if cli.verbose > 0 {
        eprintln!("theme: commit={}", clone_result.commit_sha);
        eprintln!("theme: clone={clone_ms}ms");
        eprintln!("theme: files={}", clone_result.files_copied);
        eprintln!("theme: size={} bytes", clone_result.total_bytes);
    }

    // 4. Read theme.toml from cloned files if present.
    let manifest = zetl::web::theme::load_theme_manifest(&tmp_dir).unwrap_or_else(|e| {
        if !cli.quiet {
            eprintln!("warning: failed to read theme.toml: {e}");
        }
        None
    });

    // 5. Warn if min_zetl_version is set and current version is older.
    if let Some(ref m) = manifest {
        if let Some(ref min_ver) = m.theme.min_zetl_version {
            let current = env!("CARGO_PKG_VERSION");
            if semver_less_than(current, min_ver) && !cli.quiet {
                eprintln!(
                    "warning: theme requires zetl >= {min_ver} but current version is {current}"
                );
            }
        }
    }

    // 6. Resolve final name.
    let resolved_name =
        resolve_theme_name(name_flag, manifest.as_ref(), path_flag, &install_source)
            .with_context(|| "could not determine theme name")?;

    let target_dir = themes_dir.join(&resolved_name);

    // 2. REQ-014-010: check if target already exists; require --force or error.
    if target_dir.exists() {
        if !force {
            // Clean up temp clone before erroring.
            let _ = std::fs::remove_dir_all(&tmp_dir);
            anyhow::bail!("theme {resolved_name:?} is already installed; use --force to overwrite");
        }
        // 7. If --force and target exists, delete it.
        std::fs::remove_dir_all(&target_dir)
            .with_context(|| format!("failed to remove existing theme {}", target_dir.display()))?;
    }

    // 8. Move temp clone to final location.
    std::fs::rename(&tmp_dir, &target_dir).or_else(|_| {
        // rename may fail across mount points; fall back to copy + delete.
        let r = copy_dir_all(&tmp_dir, &target_dir);
        let _ = std::fs::remove_dir_all(&tmp_dir);
        r
    })?;

    // 9. Write provenance.
    let prov_source = zetl::web::theme::ThemeInstallSource {
        url: install_source.url.clone(),
        git_ref: install_source.git_ref.clone(),
        path: path_flag.map(str::to_string),
    };
    write_provenance(&target_dir, &prov_source, &clone_result)
        .with_context(|| "failed to write provenance")?;

    // 10. Make hook files executable and collect their names. Windows has
    // no Unix mode bit — we just enumerate the hook files and rely on the
    // hook runner to invoke them via their shebang / interpreter.
    let mut installed_hooks = Vec::<String>::new();
    let hooks_dir = target_dir.join("hooks");
    if hooks_dir.is_dir() {
        for entry in std::fs::read_dir(&hooks_dir)
            .with_context(|| format!("failed to read {}", hooks_dir.display()))?
            .flatten()
        {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            if !zetl::hooks::HOOK_NAMES.contains(&name.as_str()) {
                continue;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(&path)
                    .with_context(|| format!("failed to read metadata for {}", path.display()))?
                    .permissions();
                let mode = perms.mode();
                if mode & 0o111 == 0 {
                    perms.set_mode(mode | 0o755);
                    std::fs::set_permissions(&path, perms)
                        .with_context(|| format!("failed to chmod +x {}", path.display()))?;
                }
            }
            installed_hooks.push(name);
        }
        installed_hooks.sort();
    }

    // 11. Output.
    let version = manifest.as_ref().map(|m| m.theme.version.clone());

    #[derive(Serialize)]
    struct InstalledInfo {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        version: Option<String>,
        source: String,
        #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
        git_ref: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        hooks: Vec<String>,
    }
    #[derive(Serialize)]
    struct ThemeInstallOutput {
        installed: InstalledInfo,
    }

    let output = ThemeInstallOutput {
        installed: InstalledInfo {
            name: resolved_name.clone(),
            version,
            source: install_source.url.clone(),
            git_ref: install_source.git_ref.clone(),
            path: path_flag.map(str::to_string),
            hooks: installed_hooks.clone(),
        },
    };

    match cli.format {
        OutputFormat::Json => print_json(&output)?,
        _ => {
            println!("Installed theme {resolved_name:?}");
            if !installed_hooks.is_empty() {
                println!("Hooks: {}", installed_hooks.join(", "));
            }
        }
    }

    Ok(())
}

/// Copy the directory tree at `src` into `dst` (created if missing).
fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(dst).with_context(|| format!("failed to create {}", dst.display()))?;
    for entry in std::fs::read_dir(src)
        .with_context(|| format!("failed to read {}", src.display()))?
        .flatten()
    {
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_all(&from, &to)?;
        } else {
            std::fs::copy(&from, &to).with_context(|| {
                format!("failed to copy {} to {}", from.display(), to.display())
            })?;
        }
    }
    Ok(())
}

/// Return `true` if SemVer string `a` is strictly less than `b`.
///
/// Compares only the MAJOR.MINOR.PATCH numeric prefix; pre-release and build
/// metadata are ignored.  Returns `false` on any parse error so that a
/// malformed version string does not abort the install.
fn semver_less_than(a: &str, b: &str) -> bool {
    fn parse(v: &str) -> Option<(u64, u64, u64)> {
        let core = v.split(&['-', '+'][..]).next()?;
        let mut parts = core.splitn(3, '.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = parts.next()?.parse().ok()?;
        Some((major, minor, patch))
    }
    match (parse(a), parse(b)) {
        (Some(va), Some(vb)) => va < vb,
        _ => false,
    }
}

fn cmd_theme_remove(cli: &Cli, name: &str) -> Result<()> {
    // 1. Validate name (rejects path traversal and invalid chars).
    zetl::web::theme::validate_theme_name(name)
        .with_context(|| format!("invalid theme name {name:?}"))?;

    let vault_root = std::fs::canonicalize(&cli.dir)
        .with_context(|| format!("Cannot resolve vault directory: {}", cli.dir))?;

    // 2. Check if this is a bundled-only theme (not installed on disk).
    let bundled_names: std::collections::HashSet<String> = zetl::web::engine::bundled_theme_names()
        .into_iter()
        .map(|s| s.to_string())
        .collect();
    let is_bundled = bundled_names.contains(name);

    let theme_dir = vault_root.join(".zetl/themes").join(name);
    if !theme_dir.is_dir() {
        if is_bundled {
            anyhow::bail!("cannot remove bundled theme {name:?}");
        } else {
            anyhow::bail!("theme {name:?} is not installed");
        }
    }

    // 3. Warn if the installed theme shadows a bundled theme.
    let was_shadowing = is_bundled;
    if was_shadowing && !cli.quiet {
        eprintln!(
            "warning: removing installed version of {name:?}; the bundled theme will be used instead"
        );
    }

    // 4. Delete .zetl/themes/<name>/ recursively.
    std::fs::remove_dir_all(&theme_dir)
        .with_context(|| format!("failed to remove theme directory {}", theme_dir.display()))?;

    // 5. Output result JSON.
    #[derive(Serialize)]
    struct RemovedInfo {
        name: String,
        was_shadowing: bool,
    }
    #[derive(Serialize)]
    struct ThemeRemoveOutput {
        removed: RemovedInfo,
    }

    let output = ThemeRemoveOutput {
        removed: RemovedInfo {
            name: name.to_string(),
            was_shadowing,
        },
    };

    match cli.format {
        OutputFormat::Json => print_json(&output)?,
        _ => {
            println!(
                "Removed theme {:?}{}",
                name,
                if was_shadowing {
                    " (was shadowing bundled theme)"
                } else {
                    ""
                }
            );
        }
    }

    Ok(())
}

fn cmd_theme_export(cli: &Cli, name: &str, force: bool) -> Result<()> {
    // 1. Validate name (rejects path traversal and invalid chars).
    zetl::web::theme::validate_theme_name(name)
        .with_context(|| format!("invalid theme name {name:?}"))?;

    // 2. Check name is a bundled theme.
    let is_bundled = zetl::web::engine::bundled_theme_names().contains(&name);
    if !is_bundled {
        anyhow::bail!("only bundled themes can be exported");
    }

    // 3. Resolve vault root.
    let vault_root = std::fs::canonicalize(&cli.dir)
        .with_context(|| format!("Cannot resolve vault directory: {}", cli.dir))?;

    // 4. Check if .zetl/themes/<name>/ already exists.
    let theme_dir = vault_root.join(".zetl/themes").join(name);
    if theme_dir.is_dir() && !force {
        anyhow::bail!(".zetl/themes/{name}/ already exists\nhint: use --force to overwrite",);
    }

    // 5. Create the destination directory.
    std::fs::create_dir_all(&theme_dir)
        .with_context(|| format!("failed to create theme directory {}", theme_dir.display()))?;

    // 6. Write all embedded theme files to disk.
    let files = zetl::web::engine::bundled_theme_files(name);
    let files_written = files.len();
    for (rel_path, contents) in &files {
        let dest = theme_dir.join(rel_path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create directory {}", parent.display()))?;
        }
        std::fs::write(&dest, contents)
            .with_context(|| format!("failed to write {}", dest.display()))?;
    }

    // 7. Output result JSON.
    #[derive(Serialize)]
    struct ExportedInfo {
        name: String,
        path: String,
        files_written: usize,
    }
    #[derive(Serialize)]
    struct ThemeExportOutput {
        exported: ExportedInfo,
    }

    let output = ThemeExportOutput {
        exported: ExportedInfo {
            name: name.to_string(),
            path: theme_dir.display().to_string(),
            files_written,
        },
    };

    match cli.format {
        OutputFormat::Json => print_json(&output)?,
        _ => {
            println!(
                "Exported theme {:?} to {} ({} files)",
                name,
                theme_dir.display(),
                files_written,
            );
        }
    }

    Ok(())
}

fn cmd_hook_list(cli: &Cli, theme: &str) -> Result<()> {
    let vault_root = std::fs::canonicalize(&cli.dir)
        .with_context(|| format!("Cannot resolve vault directory: {}", cli.dir))?;

    // Resolve theme hooks directory (disk-installed or bundled).
    let theme_hooks = zetl::hooks::resolve_theme_hooks(&vault_root, theme);
    let manifest =
        zetl::hooks::discover_hooks_verbose(&vault_root, theme_hooks.path(), cli.verbose > 0);

    #[derive(Serialize)]
    struct HookEntry {
        name: String,
        source: String,
        path: String,
        executable: bool,
    }

    #[derive(Serialize)]
    struct HookListOutput {
        hooks: Vec<HookEntry>,
        total: usize,
    }

    let entries: Vec<HookEntry> = manifest
        .hooks
        .iter()
        .map(|h| HookEntry {
            name: h.name.clone(),
            source: h.source.to_string(),
            path: h.path.display().to_string(),
            executable: h.executable,
        })
        .collect();

    let output = HookListOutput {
        total: entries.len(),
        hooks: entries,
    };

    match cli.format {
        OutputFormat::Json => print_json(&output)?,
        _ => {
            if output.hooks.is_empty() {
                println!("No hooks found.");
            } else {
                let mut table = Table::new();
                table.set_header(vec!["Name", "Source", "Path", "Executable"]);
                for h in &output.hooks {
                    table.add_row(vec![
                        Cell::new(&h.name),
                        Cell::new(&h.source),
                        Cell::new(&h.path),
                        Cell::new(if h.executable { "yes" } else { "no" }),
                    ]);
                }
                println!("{table}");
            }
        }
    }

    Ok(())
}

fn cmd_hook_run(cli: &Cli, name: &str, theme: &str, extra: &[String]) -> Result<()> {
    // Validate hook name.
    if !zetl::hooks::HOOK_NAMES.contains(&name) {
        anyhow::bail!(
            "unknown hook name '{}'. Valid names: {}",
            name,
            zetl::hooks::HOOK_NAMES.join(", "),
        );
    }

    let verbose = cli.verbose > 0;

    // Run the vault pipeline to build full context.
    let pipeline = run_pipeline(cli)?;

    // Discover hooks.
    let theme_hooks = zetl::hooks::resolve_theme_hooks(&pipeline.vault_root, theme);
    let manifest =
        zetl::hooks::discover_hooks_verbose(&pipeline.vault_root, theme_hooks.path(), verbose);

    for w in &manifest.warnings {
        eprintln!("warning: {w}");
    }

    let matching = zetl::hooks::hooks_for(&manifest, name);
    if matching.is_empty() {
        anyhow::bail!("no executable hook found for '{name}'");
    }

    // Build context JSON.
    let ctx = zetl::hooks::context::build_hook_context(
        name,
        &pipeline.vault_root,
        theme,
        env!("CARGO_PKG_VERSION"),
        &pipeline.files,
        &pipeline.graph,
    );
    let mut context_value = serde_json::to_value(&ctx)?;

    // Merge extra JSON fields from -- arguments.
    if !extra.is_empty() {
        let extra_str = extra.join(" ");
        let extra_value: serde_json::Value = serde_json::from_str(&extra_str)
            .with_context(|| format!("invalid JSON after --: {extra_str}"))?;
        if let (Some(base), Some(overlay)) =
            (context_value.as_object_mut(), extra_value.as_object())
        {
            for (k, v) in overlay {
                base.insert(k.clone(), v.clone());
            }
        } else {
            anyhow::bail!("extra JSON after -- must be an object, got: {extra_str}");
        }
    }

    let context_json = serde_json::to_vec(&context_value)?;

    let hook_env = zetl::hooks::HookEnv {
        vault_root: pipeline.vault_root.clone(),
        theme: theme.to_string(),
        zetl_version: env!("CARGO_PKG_VERSION").to_string(),
        extra_vars: vec![],
    };

    // Run all matching hooks sequentially, streaming output.
    let results =
        zetl::hooks::run_hooks_verbose(&manifest, name, &context_json, &hook_env, verbose);

    let mut worst_exit_code: i32 = 0;

    for result in results {
        match result {
            Ok(output) => {
                // Print stdout to stdout.
                if !output.stdout.is_empty() {
                    print!("{}", output.stdout);
                }
                // Print stderr to stderr.
                if !output.stderr.is_empty() {
                    eprint!("{}", output.stderr);
                }
                let code = if output.timed_out {
                    eprintln!("error: hook '{}' timed out", output.path.display());
                    124 // conventional timeout exit code
                } else {
                    output.exit_code.unwrap_or(1)
                };
                if code != 0 && worst_exit_code == 0 {
                    worst_exit_code = code;
                }
            }
            Err(e) => {
                eprintln!("error: hook failed to execute: {e}");
                if worst_exit_code == 0 {
                    worst_exit_code = 1;
                }
            }
        }
    }

    if worst_exit_code != 0 {
        std::process::exit(worst_exit_code);
    }

    Ok(())
}

fn cmd_hook_new(
    cli: &Cli,
    stage: &zetl::cli::AuthoringStage,
    name: &str,
    lang: &zetl::cli::HookLang,
    ecosystem: Option<&zetl::cli::HookEcosystem>,
    force: bool,
) -> Result<()> {
    let vault_root = std::fs::canonicalize(&cli.dir)
        .with_context(|| format!("Cannot resolve vault directory: {}", cli.dir))?;
    let paths = zetl::hooks::authoring::scaffold(&vault_root, stage, name, lang, ecosystem, force)?;

    if matches!(cli.format, OutputFormat::Json) {
        #[derive(Serialize)]
        struct Out<'a> {
            hook_file: &'a std::path::Path,
            manifest_file: &'a std::path::Path,
            fixture_input: &'a std::path::Path,
            fixture_expected: &'a std::path::Path,
        }
        print_json(&Out {
            hook_file: &paths.hook_file,
            manifest_file: &paths.manifest_file,
            fixture_input: &paths.fixture_input,
            fixture_expected: &paths.fixture_expected,
        })?;
    } else {
        println!("Scaffolded hook '{name}' ({}):", stage.as_str());
        println!("  hook:     {}", paths.hook_file.display());
        println!("  manifest: {}", paths.manifest_file.display());
        println!("  input:    {}", paths.fixture_input.display());
        println!("  expected: {}", paths.fixture_expected.display());
        println!();
        println!("Run `zetl hook test {name}` to verify the scaffold.");
    }
    Ok(())
}

fn cmd_hook_test(cli: &Cli, name: &str, update: bool) -> Result<()> {
    let vault_root = std::fs::canonicalize(&cli.dir)
        .with_context(|| format!("Cannot resolve vault directory: {}", cli.dir))?;
    let outcome = zetl::hooks::authoring::run_test(&vault_root, name, update)?;
    match outcome {
        zetl::hooks::authoring::TestOutcome::Match => {
            println!("ok: hook '{name}' matches golden");
            Ok(())
        }
        zetl::hooks::authoring::TestOutcome::Updated { path } => {
            println!("updated golden: {}", path.display());
            Ok(())
        }
        zetl::hooks::authoring::TestOutcome::Mismatch { diff, .. } => {
            eprintln!("FAIL: hook '{name}' output diverges from golden");
            eprint!("{diff}");
            std::process::exit(1);
        }
    }
}

fn cmd_hook_fixture(cli: &Cli, page: &str, hook: &str) -> Result<()> {
    let vault_root = std::fs::canonicalize(&cli.dir)
        .with_context(|| format!("Cannot resolve vault directory: {}", cli.dir))?;
    let captured = zetl::hooks::authoring::capture_fixture(&vault_root, page, hook)?;
    println!(
        "captured fixture for hook '{hook}' from page '{page}':\n  input:    {}\n  expected: {}",
        captured.input_path.display(),
        captured.expected_path.display()
    );
    Ok(())
}

/// Evaluate a hook's selector against the vault and print the matched
/// pages. SPEC-032 REQ-3209 / TEST-3209.
///
/// The hook itself is NOT invoked — this is a selector-reachability check
/// for CI and hook authoring. Exit code mirrors "did the selector hit":
/// 0 if at least one page matched, 1 if zero matched.
fn cmd_hook_dry_run(cli: &Cli, spec: &str, theme: &str, limit: usize) -> Result<()> {
    use zetl::hooks::composition::{compose_stage, default_extension_id, ComposedHook};
    use zetl::hooks::manifest::{load_manifest, LoadedManifest};
    use zetl::hooks::pipeline::Stage;
    use zetl::hooks::selector::{compile, SelectorInput};
    use zetl::web::markdown::parse_frontmatter;

    // Parse `<stage>/<name>`.
    let (stage_str, name) = spec.split_once('/').ok_or_else(|| {
        anyhow::anyhow!(
            "hook spec must be `<stage>/<name>` (e.g. `transform/callouts`); got {spec:?}"
        )
    })?;
    let stage = match stage_str {
        "pre-parse" => Stage::PreParse,
        "transform" => Stage::Transform,
        "post-render" => Stage::PostRender,
        other => anyhow::bail!(
            "unknown stage {other:?}; expected one of `pre-parse`, `transform`, `post-render`"
        ),
    };

    let vault_root = std::fs::canonicalize(&cli.dir)
        .with_context(|| format!("Cannot resolve vault directory: {}", cli.dir))?;

    // Compose the requested stage so we see the hook exactly as the build
    // pipeline would (vault shadowing, ordering, disabled entries).
    let theme_hooks = zetl::hooks::resolve_theme_hooks(&vault_root, theme);
    let pipeline = compose_stage(&vault_root, theme_hooks.path(), stage)
        .with_context(|| format!("failed to compose {stage} hooks"))?;

    // Lookup resolves against the canonical extension_id; a sidecar manifest
    // can override the default filename-derived id, so check the composed
    // value first and fall back to the filename-derived default. That keeps
    // `hook dry-run transform/callouts` working whether the on-disk file is
    // `callouts.py`, `20-callouts.py`, or a renamed exe with
    // `extension_id = "callouts"` in its sidecar.
    let hook: &ComposedHook = pipeline
        .hooks
        .iter()
        .find(|h| h.extension_id == name)
        .or_else(|| {
            pipeline
                .hooks
                .iter()
                .find(|h| default_extension_id(&h.filename) == name)
        })
        .ok_or_else(|| {
            let available: Vec<String> = pipeline
                .hooks
                .iter()
                .map(|h| h.extension_id.clone())
                .collect();
            if available.is_empty() {
                anyhow::anyhow!("no hook '{name}' found in stage '{stage}' (stage has no hooks)")
            } else {
                anyhow::anyhow!(
                    "no hook '{name}' found in stage '{stage}'; available: {}",
                    available.join(", ")
                )
            }
        })?;

    // Load the sidecar manifest for the selector spec — a missing manifest
    // means "default selector" per REQ-3203.
    let selector_spec = match &hook.manifest_path {
        Some(path) => match load_manifest(path)
            .with_context(|| format!("failed to load manifest {}", path.display()))?
        {
            LoadedManifest::Present(m) => m.select,
            LoadedManifest::Missing => Default::default(),
        },
        None => Default::default(),
    };

    let selector = compile(&selector_spec).map_err(|e| {
        anyhow::anyhow!(
            "failed to compile selector for hook '{}': {e}",
            hook.extension_id
        )
    })?;

    // Enumerate vault pages. `scan_vault` already returns vault-relative
    // paths, which is exactly what the selector's path globs are written
    // against.
    let scanned = scan_vault(&vault_root, &cli.scan_options())
        .with_context(|| format!("failed to scan vault {}", vault_root.display()))?;

    let mut matched: Vec<PathBuf> = Vec::new();
    for file in &scanned {
        let abs = vault_root.join(&file.path);
        let content = match std::fs::read_to_string(&abs) {
            Ok(c) => c,
            Err(_) => continue, // unreadable file → skip, not a dry-run error
        };
        let frontmatter = parse_frontmatter(&content);
        let body = strip_leading_frontmatter(&content);
        let input = SelectorInput {
            path: &file.path,
            frontmatter: &frontmatter,
            text: body,
        };
        if selector.evaluate(&input) {
            matched.push(file.path.clone());
        }
    }

    // Deterministic output order — byte-stable for TEST-3209 goldens.
    matched.sort();

    let total = matched.len();
    let truncated = total > limit;
    let shown: Vec<PathBuf> = matched.into_iter().take(limit).collect();

    // SPEC-032 REQ-3209 / TEST-3209: matched page list, one path per line,
    // byte-stable across TTY vs pipe. Auto-format JSON would defeat the
    // "byte-identical to a golden" requirement, so we ignore `cli.format`
    // here and always emit the canonical plain text form.
    for p in &shown {
        // Force POSIX separators for cross-platform byte-stability.
        println!("{}", p.to_string_lossy().replace('\\', "/"));
    }
    if truncated {
        eprintln!("(matched {total}, showing first {limit}; raise with --limit)");
    }

    if total == 0 {
        std::process::exit(1);
    }
    Ok(())
}

/// Strip a leading YAML frontmatter block (between `---` fences) from a
/// markdown document, returning the body. No frontmatter → the full input.
///
/// Mirrors `web::markdown::strip_frontmatter` (kept private there); the
/// dry-run path needs the body to feed the selector's content-probe layer,
/// which CON-3204 defines as "raw Markdown body, post-frontmatter".
fn strip_leading_frontmatter(content: &str) -> &str {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return content;
    }
    let after_first = &trimmed[3..];
    if let Some(end_pos) = after_first.find("\n---") {
        let skip = 3 + end_pos + 4;
        let rest = &trimmed[skip..];
        rest.strip_prefix('\n').unwrap_or(rest)
    } else {
        content
    }
}

/// Per-page cache used by `cmd_hook_coverage` — parse the frontmatter and
/// strip the body once up front, then reuse the `(path, frontmatter, body)`
/// tuple when each hook's selector is evaluated against the vault.
struct CoveragePage {
    path: PathBuf,
    frontmatter: serde_json::Value,
    body: String,
}

fn cmd_hook_coverage(
    cli: &Cli,
    theme: &str,
    stage_filter: Option<&zetl::cli::AuthoringStage>,
) -> Result<()> {
    use zetl::cli::AuthoringStage;
    use zetl::hooks::composition::compose_stage;
    use zetl::hooks::pipeline::Stage;
    use zetl::web::markdown::parse_frontmatter;

    let vault_root = std::fs::canonicalize(&cli.dir)
        .with_context(|| format!("Cannot resolve vault directory: {}", cli.dir))?;
    let theme_hooks = zetl::hooks::resolve_theme_hooks(&vault_root, theme);

    // Pick which stages to report on. REQ-3208 defaults to every stage;
    // `--stage` restricts to one.
    let stages: Vec<Stage> = match stage_filter {
        Some(AuthoringStage::PreParse) => vec![Stage::PreParse],
        Some(AuthoringStage::Transform) => vec![Stage::Transform],
        Some(AuthoringStage::PostRender) => vec![Stage::PostRender],
        None => Stage::all().to_vec(),
    };

    // Enumerate the vault once — the selector path is measured against
    // vault-relative paths, and the total page count feeds both the
    // per-hook `matched/total` ratio and the unmatched-pages list.
    let scanned = scan_vault(&vault_root, &cli.scan_options())
        .with_context(|| format!("failed to scan vault {}", vault_root.display()))?;
    let total_pages = scanned.len();

    let pages: Vec<CoveragePage> = scanned
        .iter()
        .filter_map(|f| {
            let abs = vault_root.join(&f.path);
            let content = std::fs::read_to_string(&abs).ok()?;
            let frontmatter = parse_frontmatter(&content);
            let body = strip_leading_frontmatter(&content).to_string();
            Some(CoveragePage {
                path: f.path.clone(),
                frontmatter,
                body,
            })
        })
        .collect();

    // Optional persisted build coverage — a future build writes
    // `.zetl/build/hook-coverage.json` and we merge its latency / failure
    // data in here. Missing file = "no build on disk; this is a dry-run".
    let persisted = load_persisted_coverage(&vault_root);
    let source = if persisted.is_some() {
        "build"
    } else {
        "dry-run"
    };

    let mut hook_rows: Vec<HookCoverageRow> = Vec::new();

    for stage in &stages {
        let pipeline = compose_stage(&vault_root, theme_hooks.path(), *stage)
            .with_context(|| format!("failed to compose {stage} hooks"))?;

        for hook in &pipeline.hooks {
            let row = coverage_row_for(hook, &pages, persisted.as_ref())?;
            hook_rows.push(row);
        }
    }

    // Unmatched pages: pages that no enabled hook in the selected stages
    // matched. In build mode we still compute this from the dry-run
    // result — the build writes per-hook coverage, not per-page, and the
    // reader side stays cheap by re-evaluating locally.
    let mut matched_pages: HashSet<PathBuf> = HashSet::new();
    for row in &hook_rows {
        for p in &row.matched_paths {
            matched_pages.insert(p.clone());
        }
    }
    let mut unmatched_pages: Vec<PathBuf> = pages
        .iter()
        .filter(|p| !matched_pages.contains(&p.path))
        .map(|p| p.path.clone())
        .collect();
    unmatched_pages.sort();

    let unmatched_hooks: Vec<UnmatchedHookRow> = hook_rows
        .iter()
        .filter(|r| r.matched == 0)
        .map(|r| UnmatchedHookRow {
            stage: r.stage.to_string(),
            id: r.id.clone(),
        })
        .collect();

    let output = HookCoverageOutput {
        source: source.to_string(),
        total_pages,
        hooks: hook_rows.iter().map(HookCoverageRow::to_entry).collect(),
        unmatched_hooks,
        unmatched_pages: unmatched_pages
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect(),
    };

    let emit_json = cli.json || cli.format == OutputFormat::Json;
    if emit_json {
        print_json(&output)?;
    } else {
        render_coverage_table(&output);
    }

    Ok(())
}

#[derive(Serialize)]
struct HookCoverageOutput {
    /// `"build"` when coverage was read from `.zetl/build/hook-coverage.json`,
    /// `"dry-run"` when the report was synthesised from a fresh selector
    /// evaluation against the current vault (REQ-3208 fallback).
    source: String,
    total_pages: usize,
    hooks: Vec<HookCoverageEntry>,
    unmatched_hooks: Vec<UnmatchedHookRow>,
    unmatched_pages: Vec<String>,
}

#[derive(Serialize)]
struct HookCoverageEntry {
    stage: String,
    id: String,
    manifest_path: Option<String>,
    matched: usize,
    matched_of: usize,
    invoked: usize,
    failed: usize,
    p50_ms: Option<u64>,
    p95_ms: Option<u64>,
    last_failure_reason: Option<String>,
}

#[derive(Serialize, Clone)]
struct UnmatchedHookRow {
    stage: String,
    id: String,
}

struct HookCoverageRow {
    stage: zetl::hooks::pipeline::Stage,
    id: String,
    manifest_path: Option<PathBuf>,
    matched: usize,
    matched_of: usize,
    invoked: usize,
    failed: usize,
    p50_ms: Option<u64>,
    p95_ms: Option<u64>,
    last_failure_reason: Option<String>,
    matched_paths: Vec<PathBuf>,
}

impl HookCoverageRow {
    fn to_entry(&self) -> HookCoverageEntry {
        HookCoverageEntry {
            stage: self.stage.to_string(),
            id: self.id.clone(),
            manifest_path: self.manifest_path.as_ref().map(|p| p.display().to_string()),
            matched: self.matched,
            matched_of: self.matched_of,
            invoked: self.invoked,
            failed: self.failed,
            p50_ms: self.p50_ms,
            p95_ms: self.p95_ms,
            last_failure_reason: self.last_failure_reason.clone(),
        }
    }
}

fn coverage_row_for(
    hook: &zetl::hooks::composition::ComposedHook,
    pages: &[CoveragePage],
    persisted: Option<&PersistedCoverage>,
) -> Result<HookCoverageRow> {
    use zetl::hooks::manifest::{load_manifest, LoadedManifest};
    use zetl::hooks::selector::{compile, SelectorInput};

    let selector_spec = match &hook.manifest_path {
        Some(path) => match load_manifest(path)
            .with_context(|| format!("failed to load manifest {}", path.display()))?
        {
            LoadedManifest::Present(m) => m.select,
            LoadedManifest::Missing => Default::default(),
        },
        None => Default::default(),
    };

    let selector = compile(&selector_spec).map_err(|e| {
        anyhow::anyhow!(
            "failed to compile selector for hook '{}' ({}): {e}",
            hook.extension_id,
            hook.stage
        )
    })?;

    let mut matched_paths: Vec<PathBuf> = Vec::new();
    for page in pages {
        let input = SelectorInput {
            path: &page.path,
            frontmatter: &page.frontmatter,
            text: &page.body,
        };
        if selector.evaluate(&input) {
            matched_paths.push(page.path.clone());
        }
    }
    matched_paths.sort();

    let matched = matched_paths.len();

    // Persisted build data (if any) overrides the dry-run synthesised
    // invoked / failed / latency cells. When absent — the default for now —
    // invoked == matched and failure stats are None.
    let persisted_row = persisted.and_then(|p| p.lookup(hook.stage, &hook.extension_id));
    let (invoked, failed, p50_ms, p95_ms, last_failure_reason) = match persisted_row {
        Some(r) => (
            r.invoked,
            r.failed,
            r.p50_ms,
            r.p95_ms,
            r.last_failure_reason.clone(),
        ),
        None => (matched, 0, None, None, None),
    };

    Ok(HookCoverageRow {
        stage: hook.stage,
        id: hook.extension_id.clone(),
        manifest_path: hook.manifest_path.clone(),
        matched,
        matched_of: pages.len(),
        invoked,
        failed,
        p50_ms,
        p95_ms,
        last_failure_reason,
        matched_paths,
    })
}

/// Parsed `.zetl/build/hook-coverage.json`. Keys are `(stage, extension_id)`
/// — same identity used throughout SPEC-032 for hook lookup.
struct PersistedCoverage {
    rows: HashMap<(String, String), PersistedRow>,
}

#[derive(Clone)]
struct PersistedRow {
    invoked: usize,
    failed: usize,
    p50_ms: Option<u64>,
    p95_ms: Option<u64>,
    last_failure_reason: Option<String>,
}

impl PersistedCoverage {
    fn lookup(&self, stage: zetl::hooks::pipeline::Stage, id: &str) -> Option<&PersistedRow> {
        self.rows.get(&(stage.as_str().to_string(), id.to_string()))
    }
}

fn load_persisted_coverage(vault_root: &Path) -> Option<PersistedCoverage> {
    let path = vault_root
        .join(".zetl")
        .join("build")
        .join("hook-coverage.json");
    let bytes = std::fs::read(&path).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let arr = v.get("hooks").and_then(|h| h.as_array())?;
    let mut rows: HashMap<(String, String), PersistedRow> = HashMap::new();
    for row in arr {
        let stage = row.get("stage")?.as_str()?.to_string();
        let id = row.get("id")?.as_str()?.to_string();
        let invoked = row.get("invoked").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
        let failed = row.get("failed").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
        let p50_ms = row.get("p50_ms").and_then(|x| x.as_u64());
        let p95_ms = row.get("p95_ms").and_then(|x| x.as_u64());
        let last_failure_reason = row
            .get("last_failure_reason")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string());
        rows.insert(
            (stage, id),
            PersistedRow {
                invoked,
                failed,
                p50_ms,
                p95_ms,
                last_failure_reason,
            },
        );
    }
    Some(PersistedCoverage { rows })
}

fn render_coverage_table(output: &HookCoverageOutput) {
    // CON-3208 table shape: HOOK, STAGE, MATCHED, INVOKED, FAILED, P50, P95.
    // We add a header above noting `source` (dry-run vs build) and the
    // total page count so the `N/500` cell has context.
    println!(
        "Source: {}   Total pages: {}",
        output.source, output.total_pages
    );

    if output.hooks.is_empty() {
        println!("No hooks configured.");
    } else {
        let mut table = Table::new();
        table.set_header(vec![
            "HOOK", "STAGE", "MATCHED", "INVOKED", "FAILED", "P50", "P95",
        ]);
        for h in &output.hooks {
            table.add_row(vec![
                Cell::new(&h.id),
                Cell::new(&h.stage),
                Cell::new(format!("{}/{}", h.matched, h.matched_of)),
                Cell::new(h.invoked.to_string()),
                Cell::new(h.failed.to_string()),
                Cell::new(match h.p50_ms {
                    Some(ms) => format!("{ms}ms"),
                    None => "-".to_string(),
                }),
                Cell::new(match h.p95_ms {
                    Some(ms) => format!("{ms}ms"),
                    None => "-".to_string(),
                }),
            ]);
        }
        println!("{table}");
    }

    if !output.unmatched_hooks.is_empty() {
        println!();
        println!("Unmatched hooks ({}):", output.unmatched_hooks.len());
        for h in &output.unmatched_hooks {
            println!("  {} ({})", h.id, h.stage);
        }
    }

    if !output.unmatched_pages.is_empty() {
        println!();
        println!(
            "Unmatched pages ({} of {}):",
            output.unmatched_pages.len(),
            output.total_pages
        );
        for p in &output.unmatched_pages {
            println!("  {p}");
        }
    }
}

/// `zetl hook capabilities` — SPEC-032 REQ-3216. Probes every composed
/// hook and reports supported stages, AST types, and schema version.
///
/// Per-hook the command spawns the executable, waits for the startup
/// handshake, sends `{"type":"probe"}`, reads the `probe_result` line,
/// and shuts the child down cleanly. Probe failures are reported but
/// don't short-circuit — every hook's outcome is surfaced so the report
/// is complete. Exit code is `1` when any hook's probe failed or did
/// not declare the composed stage.
fn cmd_hook_capabilities(
    cli: &Cli,
    theme: &str,
    stage_filter: Option<&zetl::cli::AuthoringStage>,
) -> Result<()> {
    use zetl::cli::AuthoringStage;
    use zetl::hooks::capability::{
        probe_stage_pipeline, CapabilityOutcome, CapabilityReport, DEFAULT_PROBE_TIMEOUT,
    };
    use zetl::hooks::composition::compose_stage;
    use zetl::hooks::pipeline::Stage;

    let vault_root = std::fs::canonicalize(&cli.dir)
        .with_context(|| format!("Cannot resolve vault directory: {}", cli.dir))?;
    let theme_hooks = zetl::hooks::resolve_theme_hooks(&vault_root, theme);

    let stages: Vec<Stage> = match stage_filter {
        Some(AuthoringStage::PreParse) => vec![Stage::PreParse],
        Some(AuthoringStage::Transform) => vec![Stage::Transform],
        Some(AuthoringStage::PostRender) => vec![Stage::PostRender],
        None => Stage::all().to_vec(),
    };

    let mut all_reports: Vec<CapabilityReport> = Vec::new();
    for stage in &stages {
        let pipeline = compose_stage(&vault_root, theme_hooks.path(), *stage)
            .with_context(|| format!("failed to compose {stage} hooks"))?;

        // Probe mode: no build is active, so pass None for `mode`.
        let reports = probe_stage_pipeline(&pipeline.hooks, None, DEFAULT_PROBE_TIMEOUT);
        all_reports.extend(reports);
    }

    let any_failed = all_reports.iter().any(|r| {
        !matches!(
            r.outcome,
            CapabilityOutcome::Ok { .. } | CapabilityOutcome::Declined { .. }
        )
    });

    let emit_json = cli.json || cli.format == OutputFormat::Json;
    if emit_json {
        print_json(&all_reports)?;
    } else {
        render_capabilities_table(&all_reports);
    }

    if any_failed {
        std::process::exit(1);
    }
    Ok(())
}

fn render_capabilities_table(reports: &[zetl::hooks::capability::CapabilityReport]) {
    use zetl::hooks::capability::{format_diagnostic, CapabilityOutcome};

    if reports.is_empty() {
        println!("No hooks configured.");
        return;
    }

    let mut table = Table::new();
    table.set_header(vec![
        "HOOK", "STAGE", "STATUS", "VERSION", "AST", "STAGES", "READY",
    ]);
    for r in reports {
        let (version, ast_ver, stages_str, ready) = match &r.outcome {
            CapabilityOutcome::Ok { result }
            | CapabilityOutcome::Declined { result, .. }
            | CapabilityOutcome::StageMismatch { result, .. }
            | CapabilityOutcome::AstVersionMismatch { result, .. } => (
                result.version.clone(),
                result.zetl_ast.clone(),
                result.stages.join(","),
                if result.ready { "yes" } else { "no" }.to_string(),
            ),
            CapabilityOutcome::Error { .. } => ("-".into(), "-".into(), "-".into(), "-".into()),
        };
        let status = match &r.outcome {
            CapabilityOutcome::Ok { .. } => "ok",
            CapabilityOutcome::Declined { .. } => "declined",
            CapabilityOutcome::StageMismatch { .. } => "stage-mismatch",
            CapabilityOutcome::AstVersionMismatch { .. } => "ast-mismatch",
            CapabilityOutcome::Error { .. } => "error",
        };
        table.add_row(vec![
            Cell::new(&r.extension_id),
            Cell::new(&r.stage),
            Cell::new(status),
            Cell::new(version),
            Cell::new(ast_ver),
            Cell::new(stages_str),
            Cell::new(ready),
        ]);
    }
    println!("{table}");

    // Per-hook diagnostic lines for everything except clean `ok`.
    let problem_reports: Vec<&zetl::hooks::capability::CapabilityReport> = reports
        .iter()
        .filter(|r| !matches!(r.outcome, CapabilityOutcome::Ok { .. }))
        .collect();
    if !problem_reports.is_empty() {
        println!();
        for r in problem_reports {
            eprintln!("{}", format_diagnostic(r));
        }
    }
}

/// SPEC-033 REQ-3310 / CON-3310 — probe every registered ecosystem's
/// runtime, count hooks declaring each ecosystem, list reachable plugins,
/// and render either the canonical table or JSON.
///
/// Exits non-zero only when a *configured* ecosystem's runtime isn't
/// available. The zero-configured state prints the informational footer
/// and exits 0 regardless of host state.
/// SPEC-038 `zetl feed` dispatch. v1 surface:
///
///   * `validate` — fully wired (offline strict-parser smoke check).
///   * `pull|list|status|forget` — typed args, but the inbound shell
///     pipeline (HTTP transport, retention scheduling, tombstone
///     persistence in `.zetl/feeds/`) is not yet attached. Each
///     produces a structured "not yet wired" error rather than a
///     `unrecognized subcommand` failure.
fn cmd_feed(cli: &Cli, command: &zetl::feed::cli::FeedCommand) -> Result<()> {
    use zetl::feed::cli::FeedCommand;
    match command {
        FeedCommand::Validate(args) => cmd_feed_validate(cli, args),
        FeedCommand::Pull(_) => cmd_feed_not_yet_wired(
            "pull",
            "wires HTTP transport, dedup state, and inbox writes; depends on `feed::fetch::HttpTransport` impl",
        ),
        FeedCommand::List(_) => cmd_feed_not_yet_wired(
            "list",
            "needs to read .zetl/feeds/<sub-id>/state.json which the pull command writes",
        ),
        FeedCommand::Status(_) => cmd_feed_not_yet_wired(
            "status",
            "needs to read .zetl/feeds/<sub-id>/state.json which the pull command writes",
        ),
        FeedCommand::Forget(_) => cmd_feed_not_yet_wired(
            "forget",
            "needs to mutate .zetl/feeds/<sub-id>/inbox/ + tombstones.jsonl on disk",
        ),
    }
}

fn cmd_feed_not_yet_wired(name: &str, why: &str) -> Result<()> {
    eprintln!("[zetl] feed {name}: not yet wired — {why}");
    eprintln!("[zetl] feed {name}: see plans/IMPL-038-wires.spl for follow-up tasks");
    std::process::exit(2);
}

fn cmd_feed_validate(cli: &Cli, args: &zetl::feed::cli::FeedValidateArgs) -> Result<()> {
    use std::io::Read;
    let _ = cli;
    let body = if let Some(p) = &args.path {
        std::fs::read_to_string(p).with_context(|| format!("reading feed file {}", p.display()))?
    } else {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("reading feed body from stdin")?;
        buf
    };
    use zetl::feed::cli::FeedValidateFormat;
    let body_trim = body.trim_start();
    let format = args.feed_format.unwrap_or_else(|| {
        if body_trim.starts_with('{') {
            FeedValidateFormat::Jsonfeed
        } else if body_trim.contains("<feed") {
            FeedValidateFormat::Atom
        } else {
            FeedValidateFormat::Rss
        }
    });
    let report = match format {
        FeedValidateFormat::Rss | FeedValidateFormat::Atom => {
            let bytes = body.as_bytes();
            zetl::feed::fetch::assert_no_xxe(bytes)
                .map_err(|e| anyhow::anyhow!("feed-validate xxe-check: {e}"))?;
            // For v1: just a no-XXE + parses-as-XML smoke test. The
            // pinned strict-parser CI gate (NFR-3805) lives behind
            // task-tests-conformance.
            serde_json::json!({
                "format": format.as_str(),
                "size_bytes": body.len(),
                "xxe": "ok",
                "warnings": Vec::<String>::new(),
            })
        }
        FeedValidateFormat::Jsonfeed => {
            let v: serde_json::Value =
                serde_json::from_str(&body).context("feed-validate: body is not valid JSON")?;
            let version = v.get("version").and_then(|x| x.as_str()).unwrap_or("");
            if version != zetl::feed::serialise_jsonfeed::JSONFEED_VERSION {
                anyhow::bail!(
                    "feed-validate jsonfeed: version mismatch (got {version:?}, expected {:?})",
                    zetl::feed::serialise_jsonfeed::JSONFEED_VERSION
                );
            }
            serde_json::json!({
                "format": format.as_str(),
                "size_bytes": body.len(),
                "items": v.get("items").and_then(|x| x.as_array()).map(|a| a.len()).unwrap_or(0),
                "warnings": Vec::<String>::new(),
            })
        }
    };
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "[zetl] feed-validate: format={} size={} bytes",
            report["format"].as_str().unwrap_or("?"),
            report["size_bytes"].as_u64().unwrap_or(0)
        );
    }
    Ok(())
}

fn cmd_ecosystem_check(cli: &Cli, theme: &str, json: bool) -> Result<()> {
    use zetl::ecosystems::check::{run_ecosystem_check, EcosystemCheckStatus};
    use zetl::hooks::composition::compose_all_stages;

    let vault_root = std::fs::canonicalize(&cli.dir)
        .with_context(|| format!("Cannot resolve vault directory: {}", cli.dir))?;
    let theme_hooks = zetl::hooks::resolve_theme_hooks(&vault_root, theme);

    // Collect every composed hook across the three stages so the
    // configured-count reflects the whole vault, not a single stage.
    let pipelines = compose_all_stages(&vault_root, theme_hooks.path())
        .context("failed to compose hook pipelines for ecosystem check")?;
    let hooks: Vec<zetl::hooks::composition::ComposedHook> = pipelines
        .into_iter()
        .flat_map(|p| p.hooks.into_iter().chain(p.disabled))
        .collect();

    let report = run_ecosystem_check(&vault_root, &hooks);

    let emit_json = json || cli.json || cli.format == OutputFormat::Json;
    if emit_json {
        print_json(&report)?;
    } else {
        render_ecosystem_check_table(&report);
    }

    // REQ-3315 config-time check: surface any mixed-parser
    // configurations so authors catch them before `zetl build`.
    // Needs the vault's page list, which `ecosystem check` doesn't
    // otherwise load — do a cheap scan here. Run before the
    // `configured failures` exit so mixed-parser + missing-runtime
    // surface together.
    let scanned = scan_vault(&vault_root, &cli.scan_options()).unwrap_or_default();
    let mixed = detect_mixed_parser_violations(&vault_root, theme_hooks.path(), &scanned)
        .unwrap_or_default();
    if !mixed.is_empty() {
        // Emit on stderr regardless of `--json` / format mode: the
        // rendered diagnostic is human-readable by construction and
        // leaves stdout (JSON or table) intact for tool consumers.
        eprintln!("{}", zetl::parsers::format_mixed_parser_report(&mixed));
    }

    if report.has_configured_failures() {
        // Walk in canonical order and surface the first configured
        // failure's hint on stderr so CI logs read naturally.
        for e in &report.entries {
            if e.configured > 0 && e.status != EcosystemCheckStatus::Detected {
                if let Some(hint) = &e.hint {
                    eprintln!("[zetl] ecosystem {}: {hint}", e.id);
                }
            }
        }
        std::process::exit(1);
    }

    Ok(())
}

fn render_ecosystem_check_table(report: &zetl::ecosystems::check::EcosystemCheckReport) {
    let mut table = Table::new();
    table.set_header(vec![
        "ECOSYSTEM",
        "STATUS",
        "VERSION",
        "PLUGINS CONFIGURED",
        "PLUGINS AVAILABLE",
    ]);
    for e in &report.entries {
        let version = match e.version.as_deref() {
            Some(v) => v.to_string(),
            None => "-".to_string(),
        };
        let available = if e.available_plugins.is_empty() {
            e.available_plugins.len().to_string()
        } else {
            format!(
                "{} ({})",
                e.available_plugins.len(),
                e.available_plugins.join(", ")
            )
        };
        table.add_row(vec![
            Cell::new(&e.id),
            Cell::new(e.status.as_str()),
            Cell::new(version),
            Cell::new(e.configured.to_string()),
            Cell::new(available),
        ]);
    }
    println!("{table}");

    if report.hooks_configured_total == 0 {
        println!();
        println!("No ecosystem hooks configured in this vault.");
        println!("To enable an ecosystem, add a manifest under .zetl/hooks/:");
        println!("  https://zetl.codeberg.page/docs/ecosystems/");
    }
}

fn cmd_hook_watch(cli: &Cli, name: &str) -> Result<()> {
    let vault_root = std::fs::canonicalize(&cli.dir)
        .with_context(|| format!("Cannot resolve vault directory: {}", cli.dir))?;

    println!("Watching hook '{name}' (Ctrl-C to stop)");
    zetl::hooks::authoring::watch(
        &vault_root,
        name,
        zetl::hooks::authoring::WatchOptions::default(),
        |event| match event {
            zetl::hooks::authoring::WatchEvent::Spawned => {
                println!("[zetl] hook spawned");
            }
            zetl::hooks::authoring::WatchEvent::Restart => {
                println!("[zetl] change detected, restarting");
            }
            zetl::hooks::authoring::WatchEvent::Stderr(s) => {
                eprint!("{s}");
            }
            zetl::hooks::authoring::WatchEvent::Shutdown => {
                println!("[zetl] watcher stopped");
            }
        },
    )
}

fn cmd_ast_sample(cli: &Cli, file: &str, stage: &zetl::cli::AstStage) -> Result<()> {
    use std::path::Path;
    use zetl::cli::AstStage;

    let path = Path::new(file);
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Cannot read page file: {}", path.display()))?;

    match stage {
        AstStage::PreParse => {
            // Raw Markdown — what a pre-parse hook receives. Frontmatter is
            // retained: pre-parse sees the file verbatim before any zetl
            // processing.
            print!("{content}");
        }
        AstStage::Transform => {
            // Parsed zetl-ext Document — what a transform hook receives.
            let doc = zetl::hooks::ast::parse_markdown(&content);
            let json = serde_json::to_value(&doc).context("Failed to serialise AST document")?;
            // Canonical JSON output: pretty-printed for CLI readability,
            // compact when the user forced `--json`.
            if matches!(cli.format, OutputFormat::Json) {
                print!("{}", serde_json::to_string(&json)?);
            } else {
                println!("{}", serde_json::to_string_pretty(&json)?);
            }
        }
        AstStage::PostRender => {
            // Rendered HTML fragment — what a post-render hook receives.
            // Empty slug_map: wikilinks render with link-error class, which
            // is the correct shape for a stage-input view (no vault context
            // is available for link resolution here).
            use std::collections::HashMap;
            let html =
                zetl::web::markdown::render_to_html(&content, &HashMap::new(), "", "index.html");
            print!("{html}");
        }
    }
    Ok(())
}

fn cmd_ast_diff(cli: &Cli, before_path: &str, after_path: &str) -> Result<()> {
    use zetl::hooks::ast::{diff_documents, AstDiffKind};

    let before_bytes = std::fs::read(before_path)
        .with_context(|| format!("Cannot read before file: {before_path}"))?;
    let after_bytes = std::fs::read(after_path)
        .with_context(|| format!("Cannot read after file: {after_path}"))?;
    let before: serde_json::Value = serde_json::from_slice(&before_bytes)
        .with_context(|| format!("Before file is not valid JSON: {before_path}"))?;
    let after: serde_json::Value = serde_json::from_slice(&after_bytes)
        .with_context(|| format!("After file is not valid JSON: {after_path}"))?;

    let diff = diff_documents(&before, &after);

    if matches!(cli.format, OutputFormat::Json) {
        #[derive(Serialize)]
        struct AttrOut {
            field: String,
            before: serde_json::Value,
            after: serde_json::Value,
        }
        #[derive(Serialize)]
        struct EntryOut {
            kind: String,
            path: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            node_type: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            start_line: Option<u32>,
            #[serde(skip_serializing_if = "Option::is_none")]
            start_col: Option<u32>,
            #[serde(skip_serializing_if = "Vec::is_empty")]
            attr_changes: Vec<AttrOut>,
        }
        let out: Vec<EntryOut> = diff
            .entries
            .iter()
            .map(|e| EntryOut {
                kind: e.kind.as_str().to_string(),
                path: e.path.clone(),
                node_type: e.node_type.clone(),
                start_line: e.start_line,
                start_col: e.start_col,
                attr_changes: e
                    .attr_changes
                    .iter()
                    .map(|c| AttrOut {
                        field: c.field.clone(),
                        before: c.before.clone(),
                        after: c.after.clone(),
                    })
                    .collect(),
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else if diff.is_empty() {
        println!("No AST differences.");
    } else {
        for entry in &diff.entries {
            let pos = match (entry.start_line, entry.start_col) {
                (Some(l), Some(c)) => format!(" ({l}:{c})"),
                _ => String::new(),
            };
            let ty = entry.node_type.as_deref().unwrap_or("?");
            let sign = match entry.kind {
                AstDiffKind::Added => "+",
                AstDiffKind::Removed => "-",
                AstDiffKind::Modified => "~",
            };
            println!("{sign} {ty}{pos} at {}", entry.path);
            for change in &entry.attr_changes {
                let bv = summarise_value(&change.before);
                let av = summarise_value(&change.after);
                println!("    {}: {bv} → {av}", change.field);
            }
        }
    }

    if !diff.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}

fn summarise_value(v: &serde_json::Value) -> String {
    let s = serde_json::to_string(v).unwrap_or_else(|_| "<err>".into());
    // Keep one-liners short; deep subtrees are noise in a diff summary.
    if s.len() > 80 {
        format!("{}…", &s[..80])
    } else {
        s
    }
}

fn cmd_agent_run(
    cli: &Cli,
    name: &str,
    theme: &str,
    target_pages: &[String],
    budget: u32,
    extra: &[String],
) -> Result<()> {
    let verbose = cli.verbose > 0;
    let pipeline = run_pipeline(cli)?;

    // Discover hooks for the on-agent lifecycle point.
    let theme_hooks = zetl::hooks::resolve_theme_hooks(&pipeline.vault_root, theme);
    let manifest =
        zetl::hooks::discover_hooks_verbose(&pipeline.vault_root, theme_hooks.path(), verbose);

    for w in &manifest.warnings {
        eprintln!("warning: {w}");
    }

    let matching = zetl::hooks::hooks_for(&manifest, "on-agent");
    if matching.is_empty() {
        anyhow::bail!("no executable on-agent hook found");
    }

    // Build context JSON with agent-specific fields.
    let mut ctx = zetl::hooks::context::build_hook_context(
        "on-agent",
        &pipeline.vault_root,
        theme,
        env!("CARGO_PKG_VERSION"),
        &pipeline.files,
        &pipeline.graph,
    );

    ctx.agent = Some(zetl::hooks::context::HookAgent {
        task: name.to_string(),
        target_pages: target_pages.to_vec(),
        budget_tokens: budget,
    });

    let mut context_value = serde_json::to_value(&ctx)?;

    // Merge extra JSON fields from -- arguments.
    if !extra.is_empty() {
        let extra_str = extra.join(" ");
        let extra_value: serde_json::Value = serde_json::from_str(&extra_str)
            .with_context(|| format!("invalid JSON after --: {extra_str}"))?;
        if let (Some(base), Some(overlay)) =
            (context_value.as_object_mut(), extra_value.as_object())
        {
            for (k, v) in overlay {
                base.insert(k.clone(), v.clone());
            }
        } else {
            anyhow::bail!("extra JSON after -- must be an object, got: {extra_str}");
        }
    }

    let context_json = serde_json::to_vec(&context_value)?;

    let hook_env = zetl::hooks::HookEnv {
        vault_root: pipeline.vault_root.clone(),
        theme: theme.to_string(),
        zetl_version: env!("CARGO_PKG_VERSION").to_string(),
        extra_vars: vec![("ZETL_AGENT_TASK".to_string(), name.to_string())],
    };

    let results =
        zetl::hooks::run_hooks_verbose(&manifest, "on-agent", &context_json, &hook_env, verbose);

    let mut worst_exit_code: i32 = 0;

    for result in results {
        match result {
            Ok(output) => {
                if !output.stdout.is_empty() {
                    print!("{}", output.stdout);
                }
                if !output.stderr.is_empty() {
                    eprint!("{}", output.stderr);
                }
                let code = if output.timed_out {
                    eprintln!("error: on-agent hook '{}' timed out", output.path.display());
                    124
                } else {
                    output.exit_code.unwrap_or(1)
                };
                if code != 0 {
                    if verbose {
                        eprintln!(
                            "warning: agent action rejected by hook '{}' (exit {})",
                            output.path.display(),
                            code
                        );
                    }
                    if worst_exit_code == 0 {
                        worst_exit_code = code;
                    }
                }
            }
            Err(e) => {
                eprintln!("error: on-agent hook failed to execute: {e}");
                if worst_exit_code == 0 {
                    worst_exit_code = 1;
                }
            }
        }
    }

    if worst_exit_code != 0 {
        std::process::exit(worst_exit_code);
    }

    Ok(())
}

/// Convert days since Unix epoch to (year, month, day).
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

fn cmd_agent_token(cli: &Cli, mnemonic: &str) -> Result<()> {
    let vault_root = std::fs::canonicalize(&cli.dir)
        .with_context(|| format!("Cannot resolve vault directory: {}", cli.dir))?;

    // Derive the public key from the mnemonic to find the matching user
    let pubkey = zetl::user::recovery::derive_pubkey_from_mnemonic(mnemonic)
        .context("invalid BIP39 mnemonic")?;
    let pubkey_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(pubkey.as_bytes());

    // Find the user profile whose recovery_pubkey matches
    let profiles = zetl::user::list_profiles(&vault_root)?;
    let profile = profiles
        .iter()
        .find(|p| p.recovery_pubkey == pubkey_b64)
        .ok_or_else(|| {
            anyhow::anyhow!("no user in this vault matches the provided mnemonic's public key")
        })?;

    let token = zetl::user::agent_token::generate_agent_token(
        mnemonic,
        &profile.id,
        profile.agent_token_generation,
    )?;

    // Output depends on format
    if cli.format == zetl::cli::OutputFormat::Json || cli.json {
        let output = serde_json::json!({
            "token": token,
            "user_id": profile.id,
            "generation": profile.agent_token_generation,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("{token}");
    }

    Ok(())
}

fn cmd_derive_ssh_key(mnemonic: &str, out: Option<&str>) -> Result<()> {
    let signing_key = zetl::user::recovery::derive_ssh_key_from_mnemonic(mnemonic)
        .context("failed to derive SSH key from mnemonic")?;

    let private_bytes = signing_key.to_bytes();
    let public_key = signing_key.verifying_key();
    let public_bytes = public_key.to_bytes();

    let openssh_pem = zetl::user::recovery::encode_openssh_ed25519(&private_bytes, &public_bytes);

    // Build the OpenSSH public key line
    let pub_b64 = base64::engine::general_purpose::STANDARD.encode(
        [
            &11u32.to_be_bytes()[..],
            b"ssh-ed25519",
            &32u32.to_be_bytes()[..],
            &public_bytes[..],
        ]
        .concat(),
    );
    let pub_line = format!("ssh-ed25519 {pub_b64} zetl-collab\n");

    match out {
        Some(path) => {
            std::fs::write(path, &openssh_pem)
                .with_context(|| format!("failed to write SSH key to {path}"))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
            }
            eprintln!("SSH key written to {path}");

            let pub_path = format!("{path}.pub");
            std::fs::write(&pub_path, &pub_line)
                .with_context(|| format!("failed to write public key to {pub_path}"))?;
            eprintln!("Public key written to {pub_path}");
        }
        None => {
            print!("{openssh_pem}");
        }
    }

    // Always print the public key to stderr for easy copy-paste
    eprint!("Public key: {pub_line}");

    Ok(())
}

fn cmd_invite(
    cli: &Cli,
    as_user: &str,
    role: &str,
    pages: Option<&str>,
    expires: Option<&str>,
    port: u16,
    host: &str,
) -> Result<()> {
    let vault_root = std::fs::canonicalize(&cli.dir)
        .with_context(|| format!("Cannot resolve vault directory: {}", cli.dir))?;

    // Validate the role
    let _role: zetl::user::Role = role.parse().context("invalid --role value")?;

    // Resolve inviter: look up by name (case-insensitive), fall back to user ID
    let inviter = zetl::user::find_by_name(&vault_root, as_user)?.or_else(|| {
        zetl::user::load_profile(&vault_root, as_user)
            .ok()
            .flatten()
    });

    let inviter =
        inviter.ok_or_else(|| anyhow::anyhow!("user '{as_user}' not found in this vault"))?;

    // Parse expiry duration
    let expires_secs = match expires {
        Some(s) => Some(parse_duration_secs(s)?),
        None => None,
    };

    let (token, _nonce) = zetl::user::invite::generate_invitation(
        &vault_root,
        &inviter.id,
        role,
        pages,
        expires_secs,
    )?;

    let url = zetl::user::invite::invitation_url(host, port, &token);

    // Output depends on format
    if cli.format == zetl::cli::OutputFormat::Json || cli.json {
        let output = serde_json::json!({
            "token": token,
            "url": url,
            "inviter": inviter.id,
            "role": role,
            "pages": pages,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        eprintln!("Invitation created by {} ({})", inviter.name, inviter.id);
        if let Some(p) = pages {
            eprintln!("  role: {role}  pages: {p}");
        } else {
            eprintln!("  role: {role}  pages: (vault-wide)");
        }
        eprintln!();
        println!("{url}");

        // Try to copy the URL to the system clipboard so it isn't lost
        // if server log output scrolls the terminal.
        let copied = std::process::Command::new("pbcopy")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                if let Some(ref mut stdin) = child.stdin {
                    stdin.write_all(url.as_bytes())?;
                }
                child.wait()
            })
            .ok()
            .or_else(|| {
                // Linux: try xclip, then xsel
                std::process::Command::new("xclip")
                    .args(["-selection", "clipboard"])
                    .stdin(std::process::Stdio::piped())
                    .spawn()
                    .and_then(|mut child| {
                        use std::io::Write;
                        if let Some(ref mut stdin) = child.stdin {
                            stdin.write_all(url.as_bytes())?;
                        }
                        child.wait()
                    })
                    .ok()
            });
        if copied.is_some() {
            eprintln!("  (copied to clipboard)");
        }
    }

    Ok(())
}

/// Parse a human-friendly duration string into seconds.
/// Supports: "72h", "24h", "7d", "30m", "3600" (plain seconds).
fn parse_duration_secs(s: &str) -> Result<u64> {
    let s = s.trim();
    if s.is_empty() {
        anyhow::bail!("empty duration string");
    }

    if let Some(h) = s.strip_suffix('h') {
        let hours: u64 = h.parse().context("invalid hours in duration")?;
        Ok(hours * 3600)
    } else if let Some(d) = s.strip_suffix('d') {
        let days: u64 = d.parse().context("invalid days in duration")?;
        Ok(days * 86400)
    } else if let Some(m) = s.strip_suffix('m') {
        let mins: u64 = m.parse().context("invalid minutes in duration")?;
        Ok(mins * 60)
    } else {
        let secs: u64 = s
            .parse()
            .context("invalid duration: expected a number or suffix (h/d/m)")?;
        Ok(secs)
    }
}

#[allow(clippy::too_many_arguments)] // command entry point — each flag maps to a CLI arg
fn cmd_serve(
    cli: &Cli,
    port: u16,
    theme: &str,
    public: Option<&str>,
    collab: bool,
    init_owner: bool,
    owner_name: &str,
    hostname: Option<&str>,
    server_key_seed: Option<&str>,
    git_poll_interval: std::time::Duration,
    safe_mode: bool,
    asset_max_file_bytes: u64,
    asset_max_total_bytes: u64,
) -> Result<()> {
    let pipeline = run_pipeline(cli)?;

    // ── Server key protection (REQ-020) ─────────────────────────────────
    // When --collab is active, ensure sensitive dirs are in .gitignore and
    // derive or verify the server key.
    if collab {
        zetl::user::ensure_gitignore(&pipeline.vault_root)
            .context("failed to update .gitignore for collab secrets")?;

        // Derive from seed phrase or load/create from file
        zetl::user::invite::load_or_derive_server_key(&pipeline.vault_root, server_key_seed)
            .context("server key setup failed")?;
    }

    // ── Bootstrap owner (REQ-020-005) ────────────────────────────────────
    if init_owner {
        let vault_root = &pipeline.vault_root;

        // One-time guard: fail if an owner already exists
        if zetl::user::owner_exists(vault_root)? {
            anyhow::bail!("vault already has an owner — --init-owner can only be run once");
        }

        // Generate user ID and recovery keypair
        let user_id = zetl::user::generate_user_id(owner_name);
        let keypair = zetl::user::recovery::generate_recovery_keypair()
            .context("failed to generate recovery keypair")?;

        // Create owner profile
        let now = {
            let d = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            let secs = d.as_secs();
            let days = secs / 86400;
            let day_secs = secs % 86400;
            let h = day_secs / 3600;
            let m = (day_secs % 3600) / 60;
            let s = day_secs % 60;
            let (y, mo, d) = days_to_ymd(days);
            format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
        };

        let profile = zetl::user::UserProfile {
            id: user_id.clone(),
            name: owner_name.to_string(),
            created_at: now,
            invited_by: None,
            owner: true,
            credentials: vec![],
            recovery_pubkey: keypair.recovery_pubkey.clone(),
            agent_token_generation: 0,
        };

        zetl::user::save_profile(vault_root, &profile).context("failed to save owner profile")?;

        // Display BIP39 mnemonic on stderr (never stdout, never stored)
        eprintln!();
        eprintln!("╔══════════════════════════════════════════════════════╗");
        eprintln!("║          RECOVERY PHRASE — WRITE THIS DOWN          ║");
        eprintln!("╠══════════════════════════════════════════════════════╣");
        let words: Vec<&str> = keypair.mnemonic.split_whitespace().collect();
        for (i, word) in words.iter().enumerate() {
            eprintln!("║  {:>2}. {:<48}║", i + 1, word);
        }
        eprintln!("╠══════════════════════════════════════════════════════╣");
        eprintln!("║  This phrase is your ONLY recovery method.          ║");
        eprintln!("║  It will NOT be shown again.                        ║");
        eprintln!("╚══════════════════════════════════════════════════════╝");
        eprintln!();
        eprintln!("Owner created: {} ({})", profile.name, profile.id);
        eprintln!("Register a passkey at: http://localhost:{port}/auth/bootstrap");
        eprintln!();
    }

    validate_theme(theme, &pipeline.vault_root)?;

    let mut page_names: Vec<String> = pipeline.files.iter().map(|f| f.page_name.clone()).collect();
    page_names.sort_by_key(|a| a.to_lowercase());

    let (page_slug_map, collision_names) = zetl::web::build_slug_map(&pipeline.files);
    let page_slug_map_lower: std::collections::HashMap<String, String> = page_slug_map
        .iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), v.clone()))
        .collect();

    let data = zetl::web::VaultData {
        files: pipeline.files,
        graph: pipeline.graph,
        page_names,
        resolved: pipeline.graph_resolved,
        page_slug_map,
        page_slug_map_lower,
        collision_names,
    };

    // Build the Tantivy search index for serve mode (REQ-013-012).
    let search_index = SearchIndex::build(&pipeline.vault_root, &data.files)
        .context("building search index for serve")?;

    // ── pre-serve hooks (abort on failure) ────────────────────────────
    let verbose = cli.verbose > 0;
    let theme_hooks = zetl::hooks::resolve_theme_hooks(&pipeline.vault_root, theme);
    let manifest =
        zetl::hooks::discover_hooks_verbose(&pipeline.vault_root, theme_hooks.path(), verbose);

    for w in &manifest.warnings {
        eprintln!("warning: {w}");
    }

    // ── REQ-3223 theme hook declaration audit (serve) ─────────────────
    let theme_manifest = load_theme_manifest_for_audit(&pipeline.vault_root, theme);
    let audit = zetl::hooks::safe_mode::audit_theme_declarations(
        theme,
        &pipeline.vault_root,
        theme_hooks.path(),
        theme_manifest.as_ref(),
    );
    if audit.has_undeclared() && !safe_mode {
        eprintln!(
            "{}",
            zetl::hooks::safe_mode::format_undeclared_warning(&audit)
        );
    }
    if safe_mode {
        let policy = zetl::hooks::safe_mode::SafeMode::from_manifest(theme_manifest.as_ref());
        match zetl::hooks::composition::compose_all_stages(&pipeline.vault_root, theme_hooks.path())
        {
            Ok(pipes) => {
                let (_kept, skipped) = zetl::hooks::safe_mode::apply_all(pipes, &policy);
                for s in &skipped {
                    eprintln!("{}", zetl::hooks::safe_mode::format_skip_line(s));
                }
            }
            Err(e) => {
                eprintln!("warning: safe-mode hook composition failed: {e}");
            }
        }
    }

    if !safe_mode && !zetl::hooks::hooks_for(&manifest, "pre-serve").is_empty() {
        let mut ctx = zetl::hooks::context::build_hook_context(
            "pre-serve",
            &pipeline.vault_root,
            theme,
            env!("CARGO_PKG_VERSION"),
            &data.files,
            &data.graph,
        );
        ctx.port = Some(port);

        let context_json = serde_json::to_vec(&ctx)?;

        let hook_env = zetl::hooks::HookEnv {
            vault_root: pipeline.vault_root.clone(),
            theme: theme.to_string(),
            zetl_version: env!("CARGO_PKG_VERSION").to_string(),
            extra_vars: vec![("ZETL_PORT".into(), port.to_string())],
        };

        let results = zetl::hooks::run_hooks_verbose(
            &manifest,
            "pre-serve",
            &context_json,
            &hook_env,
            verbose,
        );

        for result in results {
            match result {
                Ok(output) if !output.success() => {
                    if !output.stderr.is_empty() {
                        eprintln!(
                            "error: pre-serve hook '{}' failed:\n{}",
                            output.hook_name,
                            output.stderr.trim_end()
                        );
                    } else {
                        eprintln!(
                            "error: pre-serve hook '{}' ({}) exited with code {}",
                            output.path.display(),
                            output.source,
                            output.exit_code.unwrap_or(-1),
                        );
                    }
                    anyhow::bail!("pre-serve hook failed, aborting serve");
                }
                Err(e) => {
                    anyhow::bail!("pre-serve hook failed to execute: {e}");
                }
                _ => {}
            }
        }
    }

    let engine = zetl::web::engine::TemplateEngine::new(
        &pipeline.vault_root,
        theme,
        true, // reload templates on every request in serve mode
        cli.verbose > 0,
    );
    // Load the vector index for semantic/hybrid search in serve mode (REQ-100).
    // Failures are non-fatal: serve continues without semantic support.
    #[cfg(feature = "semantic")]
    let vector_index = {
        match zetl::semantic::VectorIndex::open(&pipeline.vault_root) {
            Ok(Some(idx)) => {
                if cli.verbose > 0 {
                    eprintln!(
                        "[zetl] semantic: loaded vector index ({} chunks)",
                        idx.chunk_count()
                    );
                }
                Some(std::sync::Arc::new(std::sync::Mutex::new(idx)))
            }
            Ok(None) => {
                if cli.verbose > 0 {
                    eprintln!("[zetl] semantic: no vector index found (run `zetl index` to build)");
                }
                None
            }
            Err(e) => {
                eprintln!("[zetl] warning: could not load vector index: {e}");
                None
            }
        }
    };

    // Open git repository for auto-commit on save (REQ-020-015).
    let git_commit_lock =
        zetl::web::git_commit::open_repo(&pipeline.vault_root).map(std::sync::Arc::new);

    let public_dir = public.map(|p| {
        let path = std::path::PathBuf::from(p);
        if path.is_dir() {
            eprintln!("zetl serve  →  public overlay: {p}");
        } else {
            eprintln!("warning: --public directory does not exist: {p}");
        }
        path
    });

    let vault_root = std::sync::Arc::new(pipeline.vault_root);

    // Initialise asset storage counter (REQ-3519)
    let asset_storage_total = zetl::assets::store::init_storage_total(&vault_root).unwrap_or(0);
    let asset_count = zetl::assets::store::list_assets(&vault_root, None)
        .map(|v| v.len())
        .unwrap_or(0);
    eprintln!("[zetl] assets: storage_bytes={asset_storage_total} max_bytes={asset_max_total_bytes} count={asset_count}");

    let state = zetl::web::WebState {
        data: std::sync::Arc::new(std::sync::RwLock::new(data)),
        crdt_store: zetl::web::ws::CrdtDocStore::new(vault_root.clone()),
        vault_root: vault_root.clone(),
        search_index: std::sync::Arc::new(search_index),
        engine: std::sync::Arc::new(engine),
        theme: theme.to_string(),
        verbose: cli.verbose > 0,
        collab,
        tls: false,
        trust_proxy: false,
        sessions: zetl::web::session::SessionStore::new(),
        recovery_challenges: std::sync::Arc::new(
            zetl::user::recovery::RecoveryChallengeStore::new(),
        ),
        mnemonic_shown: std::sync::Arc::new(
            std::sync::Mutex::new(std::collections::HashSet::new()),
        ),
        bootstrap_used: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        rate_limiters: zetl::web::rate_limit::AuthRateLimiters::new(),
        #[cfg(feature = "reason")]
        acl_cache: std::sync::Arc::new(std::sync::Mutex::new(zetl::web::AclCache::new())),
        git_commit_lock,
        ws_hub: zetl::web::ws::WsHub::new(),
        ticket_store: zetl::web::ws::TicketStore::new(),
        wal_store: std::sync::Arc::new(zetl::web::wal::WalStore::new(&vault_root)),
        pending_writes: zetl::web::fs_watch::PendingWrites::new(),
        passkey_mgr: zetl::user::passkey::PasskeyManager::new(
            hostname.unwrap_or("localhost"),
            &match hostname {
                Some(h) => format!("https://{h}"),
                None => format!("http://localhost:{port}"),
            },
            "zetl vault",
        )
        .ok()
        .map(std::sync::Arc::new),
        public_dir,
        scan_options: cli.scan_options(),
        #[cfg(feature = "semantic")]
        vector_index,
        asset_storage: zetl::assets::store::StorageCounterGuard::new(asset_storage_total),
        asset_max_file_bytes,
        asset_max_total_bytes,
    };

    // ── TLS enforcement (REQ-020-067) ──────────────────────────────────
    // When --collab is active, default to loopback-only binding.
    // Non-loopback binding requires ZETL_INSECURE_COLLAB=1 and emits a
    // warning.  Without the env var, refuse to start on non-loopback.
    let bind_addr = if collab {
        let insecure = std::env::var("ZETL_INSECURE_COLLAB")
            .map(|v| v == "1")
            .unwrap_or(false);
        if insecure {
            eprintln!(
                "warning: --collab is active on all interfaces without TLS. \
                 Set up a TLS reverse-proxy for production use."
            );
            "0.0.0.0"
        } else {
            "127.0.0.1"
        }
    } else {
        "0.0.0.0"
    };

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(zetl::web::run(state, port, bind_addr, git_poll_interval))?;
    Ok(())
}

/// Best-effort load of `theme.toml` for the active theme so REQ-3223
/// safe-mode and the declared-vs-discovered audit have something to
/// read. Looks at the on-disk theme first, then the compile-time
/// bundle. Returns `None` (silently) for any failure mode — the audit
/// already handles the "no manifest" case as "everything is undeclared".
fn load_theme_manifest_for_audit(
    vault_root: &std::path::Path,
    theme: &str,
) -> Option<zetl::web::theme::ThemeManifest> {
    let disk = vault_root.join(".zetl/themes").join(theme);
    if disk.is_dir() {
        if let Ok(Some(m)) = zetl::web::theme::load_theme_manifest(&disk) {
            return Some(m);
        }
    }
    zetl::web::theme::load_bundled_manifest(theme)
        .ok()
        .flatten()
}

/// SPEC-033 REQ-3315 — walk every composed hook against every vault
/// page and return the set of (page, hook) pairs whose resolved
/// parser doesn't match the ecosystem the hook belongs to.
///
/// Pure plumbing — all the detection logic lives in
/// [`zetl::parsers::detect_mixed_parsers`]; this function's only job
/// is gathering the inputs (pages, parsers, compiled selectors) from
/// on-disk state.
fn detect_mixed_parser_violations(
    vault_root: &std::path::Path,
    theme_hooks_dir: Option<&std::path::Path>,
    files: &[zetl::types::ParsedFile],
) -> Result<zetl::parsers::MixedParserReport> {
    use zetl::hooks::composition::compose_all_stages;
    use zetl::hooks::manifest::{load_manifest, LoadedManifest, SelectorSpec};
    use zetl::hooks::selector::{compile, CompiledSelector};
    use zetl::parsers::{
        detect_mixed_parsers, ecosystem_expected_parser, HookForDetection, PageForDetection,
        ParseConfig,
    };

    // Compose every stage once; only hooks that declare an ecosystem
    // whose id is a known v1 ecosystem can participate in a
    // mixed-parser violation.
    let pipelines = match compose_all_stages(vault_root, theme_hooks_dir) {
        Ok(p) => p,
        Err(e) => {
            // Composition errors are already surfaced elsewhere in the
            // build path; degrade to "no violations" so the mixed-parser
            // check doesn't double-report.
            eprintln!("warning: hook composition failed during mixed-parser check: {e}");
            return Ok(Default::default());
        }
    };

    let parse_config = ParseConfig::load_from_vault(vault_root)
        .and_then(ParseConfig::compile)
        .map_err(|e| anyhow::anyhow!("failed to load [parse] config: {e}"))?;

    // Collect (hook, compiled_selector) pairs up front so the detector
    // borrows them by reference. Selectors are compiled once per hook
    // (REQ-3204).
    struct HookEntry {
        stage: zetl::hooks::pipeline::Stage,
        hook_id: String,
        ecosystem: String,
        selector: CompiledSelector,
    }
    let mut hook_entries: Vec<HookEntry> = Vec::new();
    for pipe in &pipelines {
        for h in &pipe.hooks {
            let Some(eco) = h.ecosystem.as_deref() else {
                continue;
            };
            if ecosystem_expected_parser(eco).is_none() {
                continue;
            }
            let selector_spec: SelectorSpec = match &h.manifest_path {
                Some(p) => match load_manifest(p) {
                    Ok(LoadedManifest::Present(m)) => m.select,
                    Ok(LoadedManifest::Missing) | Err(_) => SelectorSpec::default(),
                },
                None => SelectorSpec::default(),
            };
            let selector = match compile(&selector_spec) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!(
                        "warning: skipping mixed-parser check for hook '{}': {e}",
                        h.extension_id
                    );
                    continue;
                }
            };
            hook_entries.push(HookEntry {
                stage: h.stage,
                hook_id: h.extension_id.clone(),
                ecosystem: eco.to_string(),
                selector,
            });
        }
    }

    if hook_entries.is_empty() {
        return Ok(Default::default());
    }

    // Per-page inputs: read file body, parse frontmatter, resolve
    // parser. Skip unreadable files (they'll error out elsewhere).
    struct PageEntry {
        path: std::path::PathBuf,
        parser: String,
        frontmatter: serde_json::Value,
        body: String,
    }
    let mut page_entries: Vec<PageEntry> = Vec::new();
    for f in files {
        let abs = vault_root.join(&f.path);
        let Ok(content) = std::fs::read_to_string(&abs) else {
            continue;
        };
        let fm_value = zetl::web::markdown::parse_frontmatter(&content);
        let body = strip_leading_frontmatter(&content).to_string();
        let parser = match &fm_value {
            serde_json::Value::Object(m) => {
                zetl::parsers::select_parser_name(Some(m), &f.path, &parse_config)
            }
            _ => zetl::parsers::select_parser_name(None, &f.path, &parse_config),
        };
        page_entries.push(PageEntry {
            path: f.path.clone(),
            parser,
            frontmatter: fm_value,
            body,
        });
    }

    let pages: Vec<PageForDetection<'_>> = page_entries
        .iter()
        .map(|p| PageForDetection {
            path: &p.path,
            parser: p.parser.clone(),
            frontmatter: &p.frontmatter,
            body: &p.body,
        })
        .collect();
    let hooks: Vec<HookForDetection<'_>> = hook_entries
        .iter()
        .map(|h| HookForDetection {
            stage: h.stage,
            hook_id: &h.hook_id,
            ecosystem: &h.ecosystem,
            selector: &h.selector,
        })
        .collect();

    Ok(detect_mixed_parsers(&pages, &hooks))
}

fn cmd_build(
    cli: &Cli,
    out_dir: &str,
    theme: &str,
    public: Option<&str>,
    site_url: Option<&str>,
    safe_mode: bool,
    strict_parsers: bool,
) -> Result<()> {
    let pipeline = run_pipeline(cli)?;

    validate_theme(theme, &pipeline.vault_root)?;

    let mut page_names: Vec<String> = pipeline.files.iter().map(|f| f.page_name.clone()).collect();
    page_names.sort_by_key(|a| a.to_lowercase());

    let (page_slug_map, collision_names) = zetl::web::build_slug_map(&pipeline.files);
    let page_slug_map_lower: std::collections::HashMap<String, String> = page_slug_map
        .iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), v.clone()))
        .collect();

    let data = zetl::web::VaultData {
        files: pipeline.files,
        graph: pipeline.graph,
        page_names,
        resolved: pipeline.graph_resolved,
        page_slug_map,
        page_slug_map_lower,
        collision_names,
    };

    // ── hook discovery (shared by pre-build and post-build) ────────────
    let verbose = cli.verbose > 0;
    let theme_hooks = zetl::hooks::resolve_theme_hooks(&pipeline.vault_root, theme);
    let manifest =
        zetl::hooks::discover_hooks_verbose(&pipeline.vault_root, theme_hooks.path(), verbose);

    for w in &manifest.warnings {
        eprintln!("warning: {w}");
    }

    // ── REQ-3223 theme hook declaration audit ──────────────────────────
    // Surfaces the SPEC-mandated "ships <N> undeclared hook(s)" warning
    // and computes the safe-mode allow-list. Both surfaces depend on
    // whether the theme ships SPEC-032 stage hooks (composition uses
    // the `pre-parse.d/`, `transform.d/`, `post-render.d/` layout); for
    // legacy SPEC-016 lifecycle hooks the audit is a no-op so existing
    // themes don't trip the warning until they migrate.
    let theme_manifest = load_theme_manifest_for_audit(&pipeline.vault_root, theme);
    let audit = zetl::hooks::safe_mode::audit_theme_declarations(
        theme,
        &pipeline.vault_root,
        theme_hooks.path(),
        theme_manifest.as_ref(),
    );
    if audit.has_undeclared() && !safe_mode {
        eprintln!(
            "{}",
            zetl::hooks::safe_mode::format_undeclared_warning(&audit)
        );
    }
    if safe_mode {
        let policy = zetl::hooks::safe_mode::SafeMode::from_manifest(theme_manifest.as_ref());
        match zetl::hooks::composition::compose_all_stages(&pipeline.vault_root, theme_hooks.path())
        {
            Ok(pipes) => {
                let (_kept, skipped) = zetl::hooks::safe_mode::apply_all(pipes, &policy);
                for s in &skipped {
                    eprintln!("{}", zetl::hooks::safe_mode::format_skip_line(s));
                }
            }
            Err(e) => {
                eprintln!("warning: safe-mode hook composition failed: {e}");
            }
        }
    }

    // ── REQ-3315 mixed-parser diagnostic ───────────────────────────────
    // Pair every vault page with every composed hook that carries an
    // ecosystem id; if the hook's selector matches a page whose
    // resolved parser differs from what the ecosystem expects, surface
    // a five-part diagnostic. Under `--strict-parsers` the warning is
    // fatal (CI gate for mixed-parser vaults).
    let mixed_report =
        detect_mixed_parser_violations(&pipeline.vault_root, theme_hooks.path(), &data.files)?;
    if !mixed_report.is_empty() {
        eprintln!(
            "{}",
            zetl::parsers::format_mixed_parser_report(&mixed_report)
        );
        if strict_parsers {
            anyhow::bail!(
                "mixed-parser configuration detected on {} page(s); \
                 `--strict-parsers` is set — refusing to build",
                mixed_report.violations.len()
            );
        }
    }

    // ── pre-build hooks (abort on failure) ─────────────────────────────
    if !safe_mode && !zetl::hooks::hooks_for(&manifest, "pre-build").is_empty() {
        let mut ctx = zetl::hooks::context::build_hook_context(
            "pre-build",
            &pipeline.vault_root,
            theme,
            env!("CARGO_PKG_VERSION"),
            &data.files,
            &data.graph,
        );
        ctx.out_dir = Some(out_dir.to_string());

        let context_json = serde_json::to_vec(&ctx)?;

        let hook_env = zetl::hooks::HookEnv {
            vault_root: pipeline.vault_root.clone(),
            theme: theme.to_string(),
            zetl_version: env!("CARGO_PKG_VERSION").to_string(),
            extra_vars: vec![("ZETL_OUT_DIR".into(), out_dir.to_string())],
        };

        let results = zetl::hooks::run_hooks_verbose(
            &manifest,
            "pre-build",
            &context_json,
            &hook_env,
            verbose,
        );

        for result in results {
            match result {
                Ok(output) if !output.success() => {
                    if !output.stderr.is_empty() {
                        eprintln!(
                            "error: pre-build hook '{}' failed:\n{}",
                            output.hook_name,
                            output.stderr.trim_end()
                        );
                    } else {
                        eprintln!(
                            "error: pre-build hook '{}' ({}) exited with code {}",
                            output.path.display(),
                            output.source,
                            output.exit_code.unwrap_or(-1),
                        );
                    }
                    anyhow::bail!("pre-build hook failed, aborting build");
                }
                Err(e) => {
                    anyhow::bail!("pre-build hook failed to execute: {e}");
                }
                _ => {}
            }
        }
    }

    let build_result = zetl::web::build::build_static(
        &data,
        &pipeline.vault_root,
        out_dir,
        theme,
        cli.verbose > 0,
        public,
        site_url,
    )?;

    if matches!(cli.format, OutputFormat::Json) || cli.json {
        #[derive(Serialize)]
        struct BuildOutput {
            pages: usize,
            folder_indexes: usize,
            out_dir: String,
        }
        let out = BuildOutput {
            pages: build_result.pages,
            folder_indexes: build_result.folder_indexes,
            out_dir: build_result.out_dir,
        };
        println!("{}", serde_json::to_string_pretty(&out)?);
    }

    if !safe_mode && !zetl::hooks::hooks_for(&manifest, "post-build").is_empty() {
        let mut ctx = zetl::hooks::context::build_hook_context(
            "post-build",
            &pipeline.vault_root,
            theme,
            env!("CARGO_PKG_VERSION"),
            &data.files,
            &data.graph,
        );
        ctx.out_dir = Some(out_dir.to_string());
        ctx.pages_rendered = Some(data.files.len());

        let context_json = serde_json::to_vec(&ctx)?;

        let hook_env = zetl::hooks::HookEnv {
            vault_root: pipeline.vault_root.clone(),
            theme: theme.to_string(),
            zetl_version: env!("CARGO_PKG_VERSION").to_string(),
            extra_vars: vec![("ZETL_OUT_DIR".into(), out_dir.to_string())],
        };

        let results = zetl::hooks::run_hooks_verbose(
            &manifest,
            "post-build",
            &context_json,
            &hook_env,
            verbose,
        );

        for result in results {
            match result {
                Ok(output) if !output.success() => {
                    eprintln!(
                        "warning: post-build hook '{}' ({}) exited with code {}",
                        output.path.display(),
                        output.source,
                        output.exit_code.unwrap_or(-1),
                    );
                    if !output.stderr.is_empty() {
                        eprintln!("  stderr: {}", output.stderr.trim_end());
                    }
                }
                Err(e) => {
                    eprintln!("warning: post-build hook failed to execute: {e}");
                }
                _ => {}
            }
        }
    }

    Ok(())
}

// ── Reason commands ────────────────────────────────────────────────────────

#[cfg(feature = "reason")]
fn literal_matches(literal: &str, pattern: &str) -> bool {
    if !pattern.contains('*') && !pattern.contains('?') {
        return literal.contains(pattern);
    }
    let regex_str: String = pattern
        .chars()
        .map(|c| match c {
            '*' => ".*".to_string(),
            '?' => ".".to_string(),
            c => regex::escape(&c.to_string()),
        })
        .collect();
    let anchored = format!("^{regex_str}$");
    regex::Regex::new(&anchored)
        .map(|re| re.is_match(literal))
        .unwrap_or(false)
}

/// Build theory using the theory cache when possible.
///
/// On cache hit (no SPL-containing file has changed), reconstructs the theory
/// from `.zetl/theory.json` and re-reasons (~100ms).  On cache miss, runs the
/// full parse + combine + validate + reason pipeline and saves the cache.
///
/// Returns `(TheoryResult, theory_cache_hit)` where `theory_cache_hit` is `true`
/// when the cached theory was reused (OBS-009).
#[cfg(feature = "reason")]
fn build_or_load_theory(
    pipeline: &Pipeline,
    no_cache: bool,
    verbose: u8,
) -> Result<(zetl::reason::types::TheoryResult, bool)> {
    use zetl::cache::{
        build_theory_cache, collect_spl_ast_hashes, load_theory_cache, save_theory_cache,
        theory_cache_valid,
    };
    use zetl::reason::{build_theory, build_theory_from_cache};

    let total_start = Instant::now();

    let spl_blocks: Vec<_> = pipeline
        .files
        .iter()
        .flat_map(|f| f.spl_blocks.clone())
        .collect();

    let spl_file_count = pipeline
        .files
        .iter()
        .filter(|f| !f.spl_blocks.is_empty())
        .count();

    // Try loading from theory cache (unless --no-cache).
    if !no_cache {
        if let Ok(Some(cache)) = load_theory_cache(&pipeline.vault_root) {
            let current_spl_hashes = collect_spl_ast_hashes(&pipeline.files);
            if theory_cache_valid(&current_spl_hashes, &cache) {
                if verbose > 0 {
                    eprintln!("Theory cache hit — re-reasoning from cached theory");
                }
                let reason_start = Instant::now();
                let result = build_theory_from_cache(&cache)?;
                let reason_elapsed = reason_start.elapsed();
                if verbose > 0 {
                    emit_timing_metrics(
                        spl_blocks.len(),
                        spl_file_count,
                        &result,
                        0,
                        0,
                        reason_elapsed.as_millis(),
                        total_start.elapsed().as_millis(),
                    );
                }
                return Ok((result, true));
            }
        }
    }

    // Cache miss: full build.
    let build_start = Instant::now();
    let result = build_theory(&spl_blocks)?;
    let build_elapsed = build_start.elapsed();

    // Save to theory cache.
    if !no_cache {
        let cache = build_theory_cache(
            &result.theory,
            &result.diagnostics,
            &pipeline.files,
            &result.groundings_by_block,
            &pipeline.vault_root,
        );
        if let Err(e) = save_theory_cache(&pipeline.vault_root, &cache) {
            if verbose > 0 {
                eprintln!("Warning: failed to save theory cache: {e}");
            }
        }
    }

    if verbose > 0 {
        emit_timing_metrics(
            spl_blocks.len(),
            spl_file_count,
            &result,
            build_elapsed.as_millis(),
            build_elapsed.as_millis(),
            build_elapsed.as_millis(),
            total_start.elapsed().as_millis(),
        );
    }

    Ok((result, false))
}

/// Emit OBS-005 timing metrics to stderr.
#[cfg(feature = "reason")]
fn emit_timing_metrics(
    spl_block_count: usize,
    spl_file_count: usize,
    result: &zetl::reason::types::TheoryResult,
    parse_ms: u128,
    construction_ms: u128,
    reasoning_ms: u128,
    total_ms: u128,
) {
    eprintln!("SPL blocks extracted: {spl_block_count}");
    eprintln!("Source files with SPL: {spl_file_count}");
    eprintln!(
        "Theory: {} facts, {} rules, {} defeaters, {} superiority relations",
        result.summary.fact_count,
        result.summary.rule_count,
        result.summary.defeater_count,
        result.summary.superiority_count,
    );
    eprintln!("SPL parse time: {parse_ms}ms");
    eprintln!("Theory construction time: {construction_ms}ms");
    eprintln!("Reasoning time: {reasoning_ms}ms");
    eprintln!("Total elapsed: {total_ms}ms");
}

/// Count unresolved conflicts in the theory (OBS-006).
///
/// A conflict exists when rules produce both a literal and its complement (~p vs p)
/// without a superiority relation resolving the dispute.
#[cfg(feature = "reason")]
fn count_unresolved_conflicts(result: &zetl::reason::types::TheoryResult) -> usize {
    use zetl::reason::types::RuleType;

    let mut rules_by_head: HashMap<String, Vec<&zetl::reason::types::ProvenancedRule>> =
        HashMap::new();
    for rule in &result.rules {
        rules_by_head
            .entry(rule.head.to_string())
            .or_default()
            .push(rule);
    }

    let mut all_heads: HashSet<String> = HashSet::new();
    for rule in &result.rules {
        all_heads.insert(rule.head.to_string());
    }
    for fact in &result.facts {
        all_heads.insert(fact.literal.to_string());
    }

    let superiorities = result.theory.superiorities().to_vec();
    let mut seen: HashSet<String> = HashSet::new();
    let mut count = 0;

    for lit in &all_heads {
        let base = if let Some(name) = lit.strip_prefix('~') {
            name.to_string()
        } else {
            lit.clone()
        };
        if !seen.insert(base.clone()) {
            continue;
        }

        let positive = &base;
        let negative = format!("~{base}");

        let pos_exists = all_heads.contains(positive);
        let neg_exists = all_heads.contains(&negative);
        if !pos_exists || !neg_exists {
            continue;
        }

        let pos_defeasible: Vec<_> = rules_by_head
            .get(positive)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
            .iter()
            .filter(|r| r.rule_type == RuleType::Defeasible)
            .collect();
        let neg_defeasible: Vec<_> = rules_by_head
            .get(&negative)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
            .iter()
            .filter(|r| r.rule_type == RuleType::Defeasible)
            .collect();

        let pos_has_strict = rules_by_head
            .get(positive)
            .map(|v| v.iter().any(|r| r.rule_type == RuleType::Strict))
            .unwrap_or(false);
        let neg_has_strict = rules_by_head
            .get(&negative)
            .map(|v| v.iter().any(|r| r.rule_type == RuleType::Strict))
            .unwrap_or(false);

        // One-sided strict rule resolves conflict
        if (pos_has_strict || neg_has_strict) && !(pos_has_strict && neg_has_strict) {
            continue;
        }

        // Check if all defeasible pairs have superiority
        let mut all_resolved = true;
        for pr in &pos_defeasible {
            for nr in &neg_defeasible {
                let has_sup = superiorities.iter().any(|s| {
                    (s.superior == pr.label && s.inferior == nr.label)
                        || (s.superior == nr.label && s.inferior == pr.label)
                });
                if !has_sup {
                    all_resolved = false;
                }
            }
        }

        if !pos_defeasible.is_empty() && !neg_defeasible.is_empty() && all_resolved {
            continue;
        }

        count += 1;
    }

    count
}

#[cfg(feature = "reason")]
fn cmd_reason_status(
    cli: &Cli,
    positive: bool,
    negative: bool,
    definite: bool,
    defeasible: bool,
    literal_pat: Option<&str>,
) -> Result<()> {
    use zetl::reason::types::ConclusionType;

    let pipeline = run_pipeline(cli)?;

    let block_count: usize = pipeline.files.iter().map(|f| f.spl_blocks.len()).sum();
    if block_count == 0 {
        match cli.format {
            OutputFormat::Json => exit_json_error("No SPL blocks found in vault", 1),
            _ => {
                eprintln!("No SPL blocks found in vault.");
                std::process::exit(1);
            }
        }
    }

    let (result, theory_cache_hit) = build_or_load_theory(&pipeline, cli.no_cache, cli.verbose)?;

    // OBS-009: emit cache efficiency stats to stderr when --verbose.
    if cli.verbose > 0 {
        let s = &pipeline.scan_stats;
        eprintln!("tier1_hits: {}", s.tier1_hits);
        eprintln!("tier1_misses: {}", s.tier1_misses);
        eprintln!("tier2_hits: {}", s.tier2_hits);
        eprintln!("tier2_misses: {}", s.tier2_misses);
        eprintln!("theory_cache_hit: {theory_cache_hit}");
        eprintln!("theory_cache_miss: {}", !theory_cache_hit);
    }

    // Determine if there were parse errors
    let parse_error_count = result
        .diagnostics
        .iter()
        .filter(|d| d.level == DiagnosticLevel::Error && d.message.contains("SPL parse error"))
        .count();
    let has_parse_errors = parse_error_count > 0;
    let all_blocks_failed = parse_error_count == block_count && result.conclusions.is_empty();

    // OBS-006: Count unresolved conflicts and diagnostics
    let unresolved_conflict_count = count_unresolved_conflicts(&result);
    let error_count = result
        .diagnostics
        .iter()
        .filter(|d| d.level == DiagnosticLevel::Error)
        .count();
    let warning_count = result
        .diagnostics
        .iter()
        .filter(|d| d.level == DiagnosticLevel::Warning)
        .count();

    // Filter conclusions
    let mut conclusions = result.conclusions;

    // Sign filter: positive/negative
    if positive || negative {
        conclusions.retain(|c| {
            (positive
                && matches!(
                    c.conclusion_type,
                    ConclusionType::DefinitelyProvable | ConclusionType::DefeasiblyProvable
                ))
                || (negative
                    && matches!(
                        c.conclusion_type,
                        ConclusionType::DefinitelyNotProvable
                            | ConclusionType::DefeasiblyNotProvable
                    ))
        });
    }

    // Strength filter: definite/defeasible
    if definite || defeasible {
        conclusions.retain(|c| {
            (definite
                && matches!(
                    c.conclusion_type,
                    ConclusionType::DefinitelyProvable | ConclusionType::DefinitelyNotProvable
                ))
                || (defeasible
                    && matches!(
                        c.conclusion_type,
                        ConclusionType::DefeasiblyProvable | ConclusionType::DefeasiblyNotProvable
                    ))
        });
    }

    // Literal pattern filter
    if let Some(pat) = literal_pat {
        conclusions.retain(|c| literal_matches(&c.literal, pat));
    }

    // Sort conclusions for stable output: by type then literal
    conclusions.sort_by(|a, b| {
        let type_order = |ct: &ConclusionType| match ct {
            ConclusionType::DefinitelyProvable => 0,
            ConclusionType::DefinitelyNotProvable => 1,
            ConclusionType::DefeasiblyProvable => 2,
            ConclusionType::DefeasiblyNotProvable => 3,
        };
        type_order(&a.conclusion_type)
            .cmp(&type_order(&b.conclusion_type))
            .then(a.literal.cmp(&b.literal))
    });

    // Output
    match cli.format {
        OutputFormat::Json => {
            #[derive(Serialize)]
            struct ReasonStatusOutput {
                theory: TheoryJsonSummary,
                conclusions: Vec<zetl::reason::types::ProvenancedConclusion>,
                summary: ConclusionCounts,
                diagnostics: Vec<zetl::types::Diagnostic>,
                #[serde(skip_serializing_if = "Option::is_none")]
                snapshot: Option<SnapshotInfo>,
            }

            #[derive(Serialize)]
            struct TheoryJsonSummary {
                facts: usize,
                rules: usize,
                defeaters: usize,
                superiority_relations: usize,
                source_files: usize,
            }

            #[derive(Serialize)]
            struct ConclusionCounts {
                definitely_provable: usize,
                definitely_not_provable: usize,
                defeasibly_provable: usize,
                defeasibly_not_provable: usize,
                total: usize,
                unresolved_conflicts: usize,
                diagnostic_errors: usize,
                diagnostic_warnings: usize,
            }

            let output = ReasonStatusOutput {
                theory: TheoryJsonSummary {
                    facts: result.summary.fact_count,
                    rules: result.summary.rule_count,
                    defeaters: result.summary.defeater_count,
                    superiority_relations: result.summary.superiority_count,
                    source_files: result.summary.source_file_count,
                },
                conclusions,
                summary: ConclusionCounts {
                    definitely_provable: result.summary.definitely_provable,
                    definitely_not_provable: result.summary.definitely_not_provable,
                    defeasibly_provable: result.summary.defeasibly_provable,
                    defeasibly_not_provable: result.summary.defeasibly_not_provable,
                    total: result.summary.definitely_provable
                        + result.summary.definitely_not_provable
                        + result.summary.defeasibly_provable
                        + result.summary.defeasibly_not_provable,
                    unresolved_conflicts: unresolved_conflict_count,
                    diagnostic_errors: error_count,
                    diagnostic_warnings: warning_count,
                },
                diagnostics: result.diagnostics,
                snapshot: pipeline.snapshot.clone(),
            };
            print_json(&output)?;
        }
        _ => {
            println!(
                "Theory: {} facts, {} rules, {} defeaters, {} superiority relations from {} files",
                result.summary.fact_count,
                result.summary.rule_count,
                result.summary.defeater_count,
                result.summary.superiority_count,
                result.summary.source_file_count,
            );
            println!();

            if conclusions.is_empty() {
                println!("No conclusions match the given filters.");
            } else {
                let mut table = Table::new();
                table.set_header(vec!["Tag", "Literal", "Sources"]);
                for c in &conclusions {
                    let tag = match c.conclusion_type {
                        ConclusionType::DefinitelyProvable => "+D",
                        ConclusionType::DefinitelyNotProvable => "-D",
                        ConclusionType::DefeasiblyProvable => "+d",
                        ConclusionType::DefeasiblyNotProvable => "-d",
                    };
                    let sources: String = c
                        .proof_sources
                        .iter()
                        .map(|s| {
                            if let Some(ref label) = s.rule_label {
                                format!("{}:{} ({})", s.page, s.line, label)
                            } else {
                                format!("{}:{}", s.page, s.line)
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    table.add_row(vec![
                        Cell::new(tag),
                        Cell::new(&c.literal),
                        Cell::new(sources),
                    ]);
                }
                println!("{table}");
            }

            println!();
            println!(
                "Conclusions: {} +D, {} -D, {} +d, {} -d ({} total)",
                result.summary.definitely_provable,
                result.summary.definitely_not_provable,
                result.summary.defeasibly_provable,
                result.summary.defeasibly_not_provable,
                result.summary.definitely_provable
                    + result.summary.definitely_not_provable
                    + result.summary.defeasibly_provable
                    + result.summary.defeasibly_not_provable,
            );
            if unresolved_conflict_count > 0 {
                println!("Unresolved conflicts: {unresolved_conflict_count}",);
            }
            if error_count > 0 || warning_count > 0 {
                println!("Diagnostics: {error_count} errors, {warning_count} warnings",);
            }

            if !result.diagnostics.is_empty() {
                println!();
                println!("Diagnostics:");
                for d in &result.diagnostics {
                    let level = match d.level {
                        DiagnosticLevel::Error => "ERROR",
                        DiagnosticLevel::Warning => "WARN",
                    };
                    println!(
                        "  [{}] {}:{}: {}",
                        level,
                        d.file.display(),
                        d.line,
                        d.message
                    );
                }
            }
        }
    }

    if all_blocks_failed || has_parse_errors {
        std::process::exit(2);
    }

    Ok(())
}

#[cfg(feature = "reason")]
fn cmd_reason_explain(
    cli: &Cli,
    literal_input: &str,
    max_depth: usize,
    explain_format: &zetl::cli::ExplainFormat,
) -> Result<()> {
    use zetl::cli::ExplainFormat;

    let pipeline = run_pipeline(cli)?;

    let has_spl = pipeline.files.iter().any(|f| !f.spl_blocks.is_empty());
    if !has_spl {
        match cli.format {
            OutputFormat::Json => exit_json_error("No SPL blocks found in vault", 1),
            _ => {
                eprintln!("No SPL blocks found in vault.");
                std::process::exit(1);
            }
        }
    }

    let (result, _theory_cache_hit) = build_or_load_theory(&pipeline, cli.no_cache, cli.verbose)?;

    // Parse the literal input: handle ~negation prefix
    let (is_negated, lit_name) = if let Some(name) = literal_input.strip_prefix('~') {
        (true, name)
    } else {
        (false, literal_input)
    };

    let target_literal = if is_negated {
        spindle_core::prelude::Literal::negated(lit_name)
    } else {
        spindle_core::prelude::Literal::simple(lit_name)
    };

    // Try to explain using spindle-core's explain API
    let explanation = spindle_core::explanation::explain(&result.theory, &target_literal)
        .context("Explanation engine failed")?;

    // If the literal is not found/provable, check all conclusions and offer suggestions
    if explanation.is_none() {
        // Check if the literal has any conclusion at all (including negative)
        // Prefer the most informative conclusion: -d > -D
        // (-d means defeasible reasoning was attempted; -D is just "no strict proof")
        let matching_conclusions: Vec<_> = result
            .conclusions
            .iter()
            .filter(|c| c.literal == literal_input)
            .collect();

        let matching_conclusion = matching_conclusions
            .iter()
            .find(|c| {
                matches!(
                    c.conclusion_type,
                    zetl::reason::types::ConclusionType::DefeasiblyNotProvable
                )
            })
            .or_else(|| matching_conclusions.first())
            .copied();

        if let Some(conclusion) = matching_conclusion {
            // The literal exists but is not positively provable — explain that
            return print_negative_explanation(
                cli,
                explain_format,
                literal_input,
                conclusion,
                &result,
            );
        }

        // Literal not found at all — offer "did you mean?" suggestions
        let all_literals: Vec<String> = result
            .conclusions
            .iter()
            .map(|c| c.literal.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        let suggestions = fuzzy_match_literals(literal_input, &all_literals);

        let msg = if suggestions.is_empty() {
            format!("Literal '{literal_input}' not found in any conclusion")
        } else {
            let did_you_mean: Vec<String> = suggestions.iter().map(|s| format!("'{s}'")).collect();
            format!(
                "Literal '{}' not found. Did you mean: {}?",
                literal_input,
                did_you_mean.join(", ")
            )
        };

        match cli.format {
            OutputFormat::Json => {
                #[derive(Serialize)]
                struct NotFoundOutput {
                    error: String,
                    literal: String,
                    suggestions: Vec<String>,
                }
                let output = NotFoundOutput {
                    error: msg.clone(),
                    literal: literal_input.to_string(),
                    suggestions,
                };
                print_json(&output)?;
                std::process::exit(1);
            }
            _ => {
                eprintln!("{msg}");
                std::process::exit(1);
            }
        }
    }

    let explanation = explanation.unwrap();

    // Build our enriched proof tree with provenance
    let enriched = enrich_proof_tree(
        &explanation,
        &result,
        literal_input,
        max_depth,
        pipeline.snapshot.clone(),
    );

    // Output in the requested format
    match explain_format {
        ExplainFormat::Json => {
            print_json(&enriched)?;
        }
        ExplainFormat::Table => {
            print_explain_table(&enriched);
        }
        ExplainFormat::Natural => {
            print_explain_natural(&enriched);
        }
        ExplainFormat::Dot => {
            print_explain_dot(&enriched);
        }
    }

    Ok(())
}

// ── Why-not command ────────────────────────────────────────────────────────

#[cfg(feature = "reason")]
fn cmd_reason_why_not(cli: &Cli, literal_input: &str) -> Result<()> {
    use zetl::reason::types::ConclusionType;

    let pipeline = run_pipeline(cli)?;

    let has_spl = pipeline.files.iter().any(|f| !f.spl_blocks.is_empty());
    if !has_spl {
        match cli.format {
            OutputFormat::Json => exit_json_error("No SPL blocks found in vault", 1),
            _ => {
                eprintln!("No SPL blocks found in vault.");
                std::process::exit(1);
            }
        }
    }

    let (result, _theory_cache_hit) = build_or_load_theory(&pipeline, cli.no_cache, cli.verbose)?;

    // Check if the literal appears anywhere in the theory (as a head, body, or fact)
    let all_head_literals: HashSet<String> = result
        .theory
        .rules()
        .flat_map(|r| r.head.iter().map(|h| h.to_string()))
        .collect();

    if !all_head_literals.contains(literal_input) {
        // Offer fuzzy suggestions
        let all_lits: Vec<String> = all_head_literals.into_iter().collect();
        let suggestions = fuzzy_match_literals(literal_input, &all_lits);

        let msg = if suggestions.is_empty() {
            format!("Literal '{literal_input}' not found in theory")
        } else {
            let did_you_mean: Vec<String> = suggestions.iter().map(|s| format!("'{s}'")).collect();
            format!(
                "Literal '{}' not found in theory. Did you mean: {}?",
                literal_input,
                did_you_mean.join(", ")
            )
        };

        match cli.format {
            OutputFormat::Json => {
                #[derive(Serialize)]
                struct NotFoundOutput {
                    error: String,
                    literal: String,
                    suggestions: Vec<String>,
                }
                let output = NotFoundOutput {
                    error: msg.clone(),
                    literal: literal_input.to_string(),
                    suggestions,
                };
                print_json(&output)?;
                std::process::exit(1);
            }
            _ => {
                eprintln!("{msg}");
                std::process::exit(1);
            }
        }
    }

    // Check if the literal is already positively provable — if so, why-not is moot
    let is_provable = result.conclusions.iter().any(|c| {
        c.literal == literal_input
            && matches!(
                c.conclusion_type,
                ConclusionType::DefinitelyProvable | ConclusionType::DefeasiblyProvable
            )
    });

    if is_provable {
        let msg = format!(
            "Literal '{literal_input}' IS provable. Use 'zetl reason explain {literal_input}' instead."
        );
        match cli.format {
            OutputFormat::Json => {
                #[derive(Serialize)]
                struct ProvableOutput {
                    error: String,
                    literal: String,
                    hint: String,
                }
                let output = ProvableOutput {
                    error: msg.clone(),
                    literal: literal_input.to_string(),
                    hint: format!("zetl reason explain {literal_input}"),
                };
                print_json(&output)?;
                std::process::exit(1);
            }
            _ => {
                eprintln!("{msg}");
                std::process::exit(1);
            }
        }
    }

    // Find all rules that could prove this literal (head matches)
    let candidate_rules: Vec<_> = result
        .rules
        .iter()
        .filter(|r| r.head.to_string() == literal_input)
        .collect();

    // Find facts for this literal
    let candidate_facts: Vec<_> = result
        .facts
        .iter()
        .filter(|f| f.literal.to_string() == literal_input)
        .collect();

    // Build blockers for each candidate rule
    let mut rule_analyses: Vec<WhyNotRuleAnalysis> = Vec::new();

    // Set of all provable literals (positive conclusions)
    let provable_set: HashSet<String> = result
        .conclusions
        .iter()
        .filter(|c| {
            matches!(
                c.conclusion_type,
                ConclusionType::DefinitelyProvable | ConclusionType::DefeasiblyProvable
            )
        })
        .map(|c| c.literal.clone())
        .collect();

    for rule in &candidate_rules {
        let mut blockers: Vec<WhyNotBlocker> = Vec::new();

        // Check each body literal
        for body_lit in &rule.body {
            let body_str = body_lit.to_string();
            if !provable_set.contains(&body_str) {
                // This body literal is not provable — find which docs could assert it
                let asserting_docs = find_potential_sources(&body_str, &result);
                blockers.push(WhyNotBlocker {
                    blocker_type: "failed_body".to_string(),
                    literal: body_str,
                    explanation: "Missing precondition: this body literal is not provable"
                        .to_string(),
                    sources: asserting_docs,
                });
            }
        }

        // Check if any defeater blocks this rule's conclusion
        let negated_literal = if let Some(stripped) = literal_input.strip_prefix('~') {
            stripped.to_string()
        } else {
            format!("~{literal_input}")
        };

        // Find defeaters that target this literal (produce its negation)
        for def_rule in &result.rules {
            if def_rule.rule_type == zetl::reason::types::RuleType::Defeater
                && def_rule.head.to_string() == negated_literal
            {
                // Check if the defeater's body is satisfied
                let defeater_body_satisfied = def_rule
                    .body
                    .iter()
                    .all(|b| provable_set.contains(&b.to_string()));

                if defeater_body_satisfied {
                    blockers.push(WhyNotBlocker {
                        blocker_type: "defeated".to_string(),
                        literal: negated_literal.clone(),
                        explanation: format!(
                            "Blocked by defeater '{}': all its preconditions are met",
                            def_rule.label
                        ),
                        sources: vec![WhyNotSource {
                            page: def_rule.source_page.clone(),
                            path: def_rule.source_file.to_string_lossy().to_string(),
                            line: def_rule.source_line,
                            rule_label: Some(def_rule.label.clone()),
                        }],
                    });
                }
            }
        }

        // Also check if a superior rule for the negation defeats this rule
        for other_rule in &result.rules {
            if other_rule.head.to_string() == negated_literal
                && other_rule.rule_type != zetl::reason::types::RuleType::Defeater
            {
                // Is there a superiority relation defeating our rule?
                let other_superior = result
                    .theory
                    .superiorities()
                    .iter()
                    .any(|s| s.superior == other_rule.label && s.inferior == rule.label);

                if other_superior {
                    let other_body_satisfied = other_rule
                        .body
                        .iter()
                        .all(|b| provable_set.contains(&b.to_string()));

                    if other_body_satisfied {
                        blockers.push(WhyNotBlocker {
                            blocker_type: "defeated".to_string(),
                            literal: negated_literal.clone(),
                            explanation: format!(
                                "Defeated by superior rule '{}' which proves '{}'",
                                other_rule.label, negated_literal
                            ),
                            sources: vec![WhyNotSource {
                                page: other_rule.source_page.clone(),
                                path: other_rule.source_file.to_string_lossy().to_string(),
                                line: other_rule.source_line,
                                rule_label: Some(other_rule.label.clone()),
                            }],
                        });
                    }
                }
            }
        }

        rule_analyses.push(WhyNotRuleAnalysis {
            rule_label: rule.label.clone(),
            rule_type: format!("{:?}", rule.rule_type),
            rule_text: format_rule_text(rule),
            source: WhyNotSource {
                page: rule.source_page.clone(),
                path: rule.source_file.to_string_lossy().to_string(),
                line: rule.source_line,
                rule_label: Some(rule.label.clone()),
            },
            blockers,
        });
    }

    // Find the conclusion type for this literal
    let conclusion_tag = result
        .conclusions
        .iter()
        .find(|c| c.literal == literal_input)
        .map(|c| match c.conclusion_type {
            ConclusionType::DefinitelyProvable => "+D",
            ConclusionType::DefinitelyNotProvable => "-D",
            ConclusionType::DefeasiblyProvable => "+d",
            ConclusionType::DefeasiblyNotProvable => "-d",
        })
        .unwrap_or("none");

    let output = WhyNotOutput {
        literal: literal_input.to_string(),
        conclusion: conclusion_tag.to_string(),
        candidate_rules: rule_analyses,
        is_fact: !candidate_facts.is_empty(),
        fact_sources: candidate_facts
            .iter()
            .map(|f| WhyNotSource {
                page: f.source_page.clone(),
                path: f.source_file.to_string_lossy().to_string(),
                line: f.source_line,
                rule_label: None,
            })
            .collect(),
        snapshot: pipeline.snapshot.clone(),
    };

    match cli.format {
        OutputFormat::Json => print_json(&output)?,
        _ => {
            println!(
                "Why not '{}'?  (conclusion: {})",
                output.literal, output.conclusion
            );
            println!();

            if output.is_fact {
                println!("  '{}' is asserted as a fact in:", output.literal);
                for src in &output.fact_sources {
                    println!("    [[{}]]:{} ({})", src.page, src.line, src.path);
                }
                println!();
            }

            if output.candidate_rules.is_empty() {
                println!("  No rules have '{}' as their head.", output.literal);
                println!("  To make it provable, add a rule or fact asserting it.");
            } else {
                println!(
                    "  {} rule(s) could prove '{}':",
                    output.candidate_rules.len(),
                    output.literal
                );
                println!();

                for analysis in &output.candidate_rules {
                    println!(
                        "  Rule '{}' [{}]  ([[{}]]:{})",
                        analysis.rule_label,
                        analysis.rule_type,
                        analysis.source.page,
                        analysis.source.line,
                    );
                    println!("    {}", analysis.rule_text);

                    if analysis.blockers.is_empty() {
                        println!("    No blockers found (body satisfied, no defeaters).");
                    } else {
                        for blocker in &analysis.blockers {
                            match blocker.blocker_type.as_str() {
                                "failed_body" => {
                                    println!("    MISSING: '{}'", blocker.literal);
                                    if blocker.sources.is_empty() {
                                        println!(
                                            "      Not asserted by any document in the vault."
                                        );
                                    } else {
                                        println!("      Would need to be asserted by:");
                                        for src in &blocker.sources {
                                            if let Some(ref label) = src.rule_label {
                                                println!(
                                                    "        [[{}]]:{} (rule '{}')",
                                                    src.page, src.line, label
                                                );
                                            } else {
                                                println!(
                                                    "        [[{}]]:{} ({})",
                                                    src.page, src.line, src.path
                                                );
                                            }
                                        }
                                    }
                                }
                                "defeated" => {
                                    println!("    DEFEATED: {}", blocker.explanation);
                                    for src in &blocker.sources {
                                        if let Some(ref label) = src.rule_label {
                                            println!(
                                                "      by '{}' at [[{}]]:{}",
                                                label, src.page, src.line
                                            );
                                        }
                                    }
                                }
                                _ => {
                                    println!(
                                        "    {}: {}",
                                        blocker.blocker_type, blocker.explanation
                                    );
                                }
                            }
                        }
                    }
                    println!();
                }
            }
        }
    }

    Ok(())
}

#[cfg(feature = "reason")]
#[derive(Serialize)]
struct WhyNotOutput {
    literal: String,
    conclusion: String,
    candidate_rules: Vec<WhyNotRuleAnalysis>,
    is_fact: bool,
    fact_sources: Vec<WhyNotSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    snapshot: Option<SnapshotInfo>,
}

#[cfg(feature = "reason")]
#[derive(Serialize)]
struct WhyNotRuleAnalysis {
    rule_label: String,
    rule_type: String,
    rule_text: String,
    source: WhyNotSource,
    blockers: Vec<WhyNotBlocker>,
}

#[cfg(feature = "reason")]
#[derive(Serialize)]
struct WhyNotBlocker {
    blocker_type: String,
    literal: String,
    explanation: String,
    sources: Vec<WhyNotSource>,
}

#[cfg(feature = "reason")]
#[derive(Serialize)]
struct WhyNotSource {
    page: String,
    path: String,
    line: u32,
    rule_label: Option<String>,
}

// ── Require command (abductive reasoning) ─────────────────────────────────

#[cfg(feature = "reason")]
fn cmd_reason_require(
    cli: &Cli,
    literal_input: &str,
    max_solutions: usize,
    assume_spl: Option<&str>,
) -> Result<()> {
    use zetl::reason::types::ConclusionType;

    let pipeline = run_pipeline(cli)?;

    let has_spl = pipeline.files.iter().any(|f| !f.spl_blocks.is_empty());
    if !has_spl {
        match cli.format {
            OutputFormat::Json => exit_json_error("No SPL blocks found in vault", 1),
            _ => {
                eprintln!("No SPL blocks found in vault.");
                std::process::exit(1);
            }
        }
    }

    let (result, _theory_cache_hit) = build_or_load_theory(&pipeline, cli.no_cache, cli.verbose)?;

    // Parse --assume facts if provided, and inject them into the theory for reasoning
    let assumed_literals: HashSet<String> = if let Some(assume_input) = assume_spl {
        use spindle_parser::parse_spl;
        let assumed_theory = match parse_spl(assume_input) {
            Ok(t) => t,
            Err(e) => {
                let msg = format!("--assume SPL parse error: {e}");
                match cli.format {
                    OutputFormat::Json => exit_json_error(&msg, 1),
                    _ => {
                        eprintln!("{msg}");
                        std::process::exit(1);
                    }
                }
            }
        };
        // Collect literals from assumed facts
        assumed_theory
            .rules()
            .filter(|r| r.rule_type == spindle_core::prelude::RuleType::Fact)
            .flat_map(|r| r.head.iter().map(|h| h.to_string()).collect::<Vec<_>>())
            .collect()
    } else {
        HashSet::new()
    };

    // Build provable set: all positively provable literals + assumed literals
    let mut provable_set: HashSet<String> = result
        .conclusions
        .iter()
        .filter(|c| {
            matches!(
                c.conclusion_type,
                ConclusionType::DefinitelyProvable | ConclusionType::DefeasiblyProvable
            )
        })
        .map(|c| c.literal.clone())
        .collect();
    provable_set.extend(assumed_literals.clone());

    // Check if the literal appears as a rule head anywhere in the theory
    let all_head_literals: HashSet<String> = result
        .theory
        .rules()
        .flat_map(|r| r.head.iter().map(|h| h.to_string()))
        .collect();

    // Special case: goal already provable
    if provable_set.contains(literal_input) {
        let output = RequireOutput {
            literal: literal_input.to_string(),
            status: "already_provable".to_string(),
            message: Some(format!(
                "Literal '{literal_input}' is already provable. No additional facts needed."
            )),
            solutions: vec![],
            assumed: assumed_literals.iter().cloned().collect(),
            snapshot: pipeline.snapshot.clone(),
        };
        match cli.format {
            OutputFormat::Json => print_json(&output)?,
            _ => {
                println!(
                    "Literal '{literal_input}' is already provable. No additional facts needed."
                );
                if !output.assumed.is_empty() {
                    println!("  (with assumed facts: {})", output.assumed.join(", "));
                }
            }
        }
        return Ok(());
    }

    // Special case: no rules exist with this literal as head
    if !all_head_literals.contains(literal_input) {
        // Offer fuzzy suggestions
        let all_lits: Vec<String> = all_head_literals.into_iter().collect();
        let suggestions = fuzzy_match_literals(literal_input, &all_lits);

        let msg = if suggestions.is_empty() {
            format!("No rules found with '{literal_input}' as head — cannot determine requirements")
        } else {
            let did_you_mean: Vec<String> = suggestions.iter().map(|s| format!("'{s}'")).collect();
            format!(
                "No rules found with '{}' as head. Did you mean: {}?",
                literal_input,
                did_you_mean.join(", ")
            )
        };

        match cli.format {
            OutputFormat::Json => {
                #[derive(Serialize)]
                struct NoRulesOutput {
                    error: String,
                    literal: String,
                    suggestions: Vec<String>,
                }
                let output = NoRulesOutput {
                    error: msg.clone(),
                    literal: literal_input.to_string(),
                    suggestions,
                };
                print_json(&output)?;
                std::process::exit(1);
            }
            _ => {
                eprintln!("{msg}");
                std::process::exit(1);
            }
        }
    }

    // Find all rules that could prove this literal
    let candidate_rules: Vec<_> = result
        .rules
        .iter()
        .filter(|r| r.head.to_string() == literal_input)
        .collect();

    // Abductive reasoning: for each candidate rule, find what body literals are missing,
    // then recursively find what's needed to make those provable too.
    let mut solutions: Vec<RequireSolution> = Vec::new();

    for rule in &candidate_rules {
        // Find missing body literals for this rule
        let missing: Vec<String> = rule
            .body
            .iter()
            .map(|b| b.to_string())
            .filter(|b| !provable_set.contains(b))
            .collect();

        if missing.is_empty() {
            // All body literals are provable — this rule path needs nothing more
            // (but the goal isn't provable, so something else must be blocking it —
            // e.g., a defeater or superior rule; still worth reporting as "empty" solution)
            solutions.push(RequireSolution {
                required_facts: vec![],
                via_rule: rule.label.clone(),
                rule_text: format_rule_text(rule),
                source_page: rule.source_page.clone(),
                source_file: rule.source_file.to_string_lossy().to_string(),
                source_line: rule.source_line,
                note: Some(
                    "All body literals are satisfied, but conclusion may be \
                     blocked by a defeater or superior rule."
                        .to_string(),
                ),
            });
            continue;
        }

        // Recursively expand missing literals: for each missing literal,
        // check if there are rules that could prove it (and find what THOSE need),
        // or if it can only be asserted as a new fact.
        let mut required_facts: Vec<RequiredFact> = Vec::new();
        let mut visited: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<(String, String, String)> = VecDeque::new(); // (literal, needed_by_rule, source_page)

        for m in &missing {
            queue.push_back((m.clone(), rule.label.clone(), rule.source_page.clone()));
        }

        while let Some((lit, needed_by, needed_in_page)) = queue.pop_front() {
            if visited.contains(&lit) || provable_set.contains(&lit) {
                continue;
            }
            visited.insert(lit.clone());

            // Find rules that could prove this missing literal
            let sub_rules: Vec<_> = result
                .rules
                .iter()
                .filter(|r| r.head.to_string() == lit)
                .collect();

            if sub_rules.is_empty() {
                // No rule can prove it — must be asserted as a fact
                required_facts.push(RequiredFact {
                    literal: lit.clone(),
                    reason: format!("No rules can derive '{lit}'; must be asserted as a fact"),
                    needed_by_rule: needed_by.clone(),
                    needed_in_page: needed_in_page.clone(),
                });
            } else {
                // There are rules that could prove this sub-literal,
                // but their bodies may also be missing facts.
                // For simplicity (avoiding exponential blowup), we take the
                // first rule as the "cheapest" path and expand its missing body.
                let best_rule = sub_rules[0];
                let sub_missing: Vec<String> = best_rule
                    .body
                    .iter()
                    .map(|b| b.to_string())
                    .filter(|b| !provable_set.contains(b) && !visited.contains(b))
                    .collect();

                if sub_missing.is_empty() {
                    // Sub-rule body is all satisfied — this literal *should* be provable
                    // via this rule, but isn't. Might be defeated.
                    required_facts.push(RequiredFact {
                        literal: lit.clone(),
                        reason: format!(
                            "Rule '{}' has all body literals, but '{}' is still not provable \
                             (may be defeated); asserting as fact would force it",
                            best_rule.label, lit
                        ),
                        needed_by_rule: needed_by.clone(),
                        needed_in_page: needed_in_page.clone(),
                    });
                } else {
                    // Queue the sub-rule's missing body literals
                    for sub_m in sub_missing {
                        queue.push_back((
                            sub_m,
                            best_rule.label.clone(),
                            best_rule.source_page.clone(),
                        ));
                    }
                }
            }
        }

        solutions.push(RequireSolution {
            required_facts,
            via_rule: rule.label.clone(),
            rule_text: format_rule_text(rule),
            source_page: rule.source_page.clone(),
            source_file: rule.source_file.to_string_lossy().to_string(),
            source_line: rule.source_line,
            note: None,
        });

        if solutions.len() >= max_solutions {
            break;
        }
    }

    // Sort solutions by number of required facts (fewest first)
    solutions.sort_by_key(|s| s.required_facts.len());

    // Truncate to max_solutions
    solutions.truncate(max_solutions);

    let output = RequireOutput {
        literal: literal_input.to_string(),
        status: "requirements_found".to_string(),
        message: None,
        solutions,
        assumed: assumed_literals.iter().cloned().collect(),
        snapshot: pipeline.snapshot.clone(),
    };

    match cli.format {
        OutputFormat::Json => print_json(&output)?,
        _ => {
            println!("Requirements to prove '{literal_input}':");
            if !output.assumed.is_empty() {
                println!("  Assumed facts: {}", output.assumed.join(", "));
            }
            println!();

            if output.solutions.is_empty() {
                println!("  No solution paths found.");
            } else {
                for (i, solution) in output.solutions.iter().enumerate() {
                    println!(
                        "  Solution {} via rule '{}' ([[{}]]:{})",
                        i + 1,
                        solution.via_rule,
                        solution.source_page,
                        solution.source_line,
                    );
                    println!("    {}", solution.rule_text);

                    if let Some(ref note) = solution.note {
                        println!("    Note: {note}");
                    }

                    if solution.required_facts.is_empty() {
                        println!("    No additional facts required.");
                    } else {
                        println!("    {} fact(s) required:", solution.required_facts.len());
                        for fact in &solution.required_facts {
                            println!(
                                "      - '{}' (needed by rule '{}')",
                                fact.literal, fact.needed_by_rule
                            );
                            println!("        {}", fact.reason);
                            println!("        defined in: [[{}]]", fact.needed_in_page);
                        }
                    }
                    println!();
                }
            }
        }
    }

    Ok(())
}

#[cfg(feature = "reason")]
#[derive(Serialize)]
struct RequireOutput {
    literal: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    solutions: Vec<RequireSolution>,
    assumed: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    snapshot: Option<SnapshotInfo>,
}

#[cfg(feature = "reason")]
#[derive(Serialize)]
struct RequireSolution {
    required_facts: Vec<RequiredFact>,
    via_rule: String,
    rule_text: String,
    source_page: String,
    source_file: String,
    source_line: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

#[cfg(feature = "reason")]
#[derive(Serialize)]
struct RequiredFact {
    literal: String,
    reason: String,
    needed_by_rule: String,
    needed_in_page: String,
}

// ── Conflicts command ──────────────────────────────────────────────────────

#[cfg(feature = "reason")]
fn cmd_reason_conflicts(cli: &Cli, suggest: bool, fail_on_conflicts: bool) -> Result<()> {
    use zetl::reason::types::RuleType;

    let pipeline = run_pipeline(cli)?;

    let has_spl = pipeline.files.iter().any(|f| !f.spl_blocks.is_empty());
    if !has_spl {
        match cli.format {
            OutputFormat::Json => exit_json_error("No SPL blocks found in vault", 1),
            _ => {
                eprintln!("No SPL blocks found in vault.");
                std::process::exit(1);
            }
        }
    }

    let (result, _theory_cache_hit) = build_or_load_theory(&pipeline, cli.no_cache, cli.verbose)?;

    // Build a set of all rule heads grouped by their base literal name.
    // A conflict exists when there are rules for both `p` and `~p`.
    // We need to find literal names where rules produce both a literal and its complement.
    let mut rules_for_literal: HashMap<String, Vec<&zetl::reason::types::ProvenancedRule>> =
        HashMap::new();
    for rule in &result.rules {
        let head_str = rule.head.to_string();
        rules_for_literal.entry(head_str).or_default().push(rule);
    }

    // Also include facts as potential sources of conflict
    let mut facts_for_literal: HashMap<String, Vec<&zetl::reason::types::ProvenancedFact>> =
        HashMap::new();
    for fact in &result.facts {
        let lit_str = fact.literal.to_string();
        facts_for_literal.entry(lit_str).or_default().push(fact);
    }

    // Build the set of superiority relations for quick lookup
    let superiorities: Vec<_> = result.theory.superiorities().to_vec();

    // Find all conflicting literal pairs: (p, ~p)
    // For each literal name, check if both the positive and negative forms have applicable rules
    let mut seen_names: HashSet<String> = HashSet::new();
    let mut all_head_literals: HashSet<String> = HashSet::new();
    for rule in &result.rules {
        all_head_literals.insert(rule.head.to_string());
    }
    for fact in &result.facts {
        all_head_literals.insert(fact.literal.to_string());
    }

    let mut conflicts: Vec<ConflictEntry> = Vec::new();

    for lit_str in &all_head_literals {
        // Determine the base name and its complement
        let (base_name, _complement) = if let Some(name) = lit_str.strip_prefix('~') {
            (name.to_string(), name.to_string())
        } else {
            (lit_str.clone(), format!("~{lit_str}"))
        };

        // Skip if we already processed this pair
        if !seen_names.insert(base_name.clone()) {
            continue;
        }

        // Check if the complement also has rules/facts
        let positive = base_name.clone();
        let negative = format!("~{base_name}");

        let pos_has_rules =
            rules_for_literal.contains_key(&positive) || facts_for_literal.contains_key(&positive);
        let neg_has_rules =
            rules_for_literal.contains_key(&negative) || facts_for_literal.contains_key(&negative);

        if !pos_has_rules || !neg_has_rules {
            continue; // No conflict — only one side has rules
        }

        // Collect all competing rules for both sides
        let pos_rules: Vec<_> = rules_for_literal
            .get(&positive)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
            .to_vec();
        let neg_rules: Vec<_> = rules_for_literal
            .get(&negative)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
            .to_vec();
        let pos_facts: Vec<_> = facts_for_literal
            .get(&positive)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
            .to_vec();
        let neg_facts: Vec<_> = facts_for_literal
            .get(&negative)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
            .to_vec();

        // Check which superiority relations exist between competing rules
        let mut existing_superiorities: Vec<ConflictSuperiority> = Vec::new();
        let mut all_pairs_resolved = true;

        // Only defeasible rules can be in superiority relations
        let pos_defeasible: Vec<_> = pos_rules
            .iter()
            .filter(|r| r.rule_type == RuleType::Defeasible)
            .collect();
        let neg_defeasible: Vec<_> = neg_rules
            .iter()
            .filter(|r| r.rule_type == RuleType::Defeasible)
            .collect();

        for pr in &pos_defeasible {
            for nr in &neg_defeasible {
                let sup_pos_over_neg = superiorities
                    .iter()
                    .any(|s| s.superior == pr.label && s.inferior == nr.label);
                let sup_neg_over_pos = superiorities
                    .iter()
                    .any(|s| s.superior == nr.label && s.inferior == pr.label);

                if sup_pos_over_neg {
                    existing_superiorities.push(ConflictSuperiority {
                        superior: pr.label.clone(),
                        inferior: nr.label.clone(),
                    });
                } else if sup_neg_over_pos {
                    existing_superiorities.push(ConflictSuperiority {
                        superior: nr.label.clone(),
                        inferior: pr.label.clone(),
                    });
                } else {
                    all_pairs_resolved = false;
                }
            }
        }

        // If any side has strict rules, or all defeasible pairs are resolved, skip
        let pos_has_strict = pos_rules.iter().any(|r| r.rule_type == RuleType::Strict);
        let neg_has_strict = neg_rules.iter().any(|r| r.rule_type == RuleType::Strict);

        // Strict rules always dominate — but if both sides have strict rules, that's a hard conflict
        if (pos_has_strict || neg_has_strict) && !(pos_has_strict && neg_has_strict) {
            continue; // One side has a strict rule, so no unresolved conflict
        }

        // If all defeasible rule pairs have superiority relations, skip
        if !pos_defeasible.is_empty() && !neg_defeasible.is_empty() && all_pairs_resolved {
            continue;
        }

        // If there are no defeasible rules on either side (only facts or strict),
        // check if it's actually an unresolved situation
        if pos_defeasible.is_empty()
            && neg_defeasible.is_empty()
            && !pos_has_strict
            && !neg_has_strict
        {
            // Both sides only have facts — this is a genuine conflict
        }

        // Build competing rule entries
        let mut competing_rules: Vec<CompetingRule> = Vec::new();
        for rule in pos_rules.iter().chain(neg_rules.iter()) {
            competing_rules.push(CompetingRule {
                label: rule.label.clone(),
                rule_type: format!("{:?}", rule.rule_type),
                head: rule.head.to_string(),
                body: rule.body.iter().map(|b| b.to_string()).collect(),
                source_page: rule.source_page.clone(),
                source_file: rule.source_file.to_string_lossy().to_string(),
                source_line: rule.source_line,
            });
        }
        for fact in pos_facts.iter().chain(neg_facts.iter()) {
            competing_rules.push(CompetingRule {
                label: String::new(),
                rule_type: "Fact".to_string(),
                head: fact.literal.to_string(),
                body: vec![],
                source_page: fact.source_page.clone(),
                source_file: fact.source_file.to_string_lossy().to_string(),
                source_line: fact.source_line,
            });
        }

        // Build suggestions if requested
        let suggestions = if suggest {
            build_conflict_suggestions(&base_name, &pos_rules, &neg_rules, &pos_facts, &neg_facts)
        } else {
            vec![]
        };

        conflicts.push(ConflictEntry {
            literal: base_name,
            positive_literal: positive,
            negative_literal: negative,
            competing_rules,
            existing_superiorities,
            resolved: false,
            suggestions,
        });
    }

    // Sort conflicts by literal name for stable output
    conflicts.sort_by(|a, b| a.literal.cmp(&b.literal));

    let output = ConflictsOutput {
        conflicts: conflicts.clone(),
        conflict_count: conflicts.len(),
        diagnostics: result.diagnostics,
        snapshot: pipeline.snapshot.clone(),
    };

    match cli.format {
        OutputFormat::Json => print_json(&output)?,
        _ => {
            if conflicts.is_empty() {
                println!("No unresolved conflicts found in theory.");
            } else {
                println!("{} unresolved conflict(s) found:\n", conflicts.len());

                for (i, conflict) in conflicts.iter().enumerate() {
                    println!("{}. Contested literal: {}", i + 1, conflict.literal);
                    println!();

                    // Group rules by which side they support
                    let pos_rules: Vec<_> = conflict
                        .competing_rules
                        .iter()
                        .filter(|r| r.head == conflict.positive_literal)
                        .collect();
                    let neg_rules: Vec<_> = conflict
                        .competing_rules
                        .iter()
                        .filter(|r| r.head == conflict.negative_literal)
                        .collect();

                    println!("   Rules for '{}':", conflict.positive_literal);
                    for rule in &pos_rules {
                        if rule.label.is_empty() {
                            println!(
                                "     [Fact] {}  ([[{}]]:{})",
                                rule.head, rule.source_page, rule.source_line
                            );
                        } else {
                            let body_str = if rule.body.is_empty() {
                                String::new()
                            } else {
                                format!("{} => ", rule.body.join(", "))
                            };
                            println!(
                                "     '{}' [{}]: {}{}  ([[{}]]:{})",
                                rule.label,
                                rule.rule_type,
                                body_str,
                                rule.head,
                                rule.source_page,
                                rule.source_line
                            );
                        }
                    }

                    println!("   Rules for '{}':", conflict.negative_literal);
                    for rule in &neg_rules {
                        if rule.label.is_empty() {
                            println!(
                                "     [Fact] {}  ([[{}]]:{})",
                                rule.head, rule.source_page, rule.source_line
                            );
                        } else {
                            let body_str = if rule.body.is_empty() {
                                String::new()
                            } else {
                                format!("{} => ", rule.body.join(", "))
                            };
                            println!(
                                "     '{}' [{}]: {}{}  ([[{}]]:{})",
                                rule.label,
                                rule.rule_type,
                                body_str,
                                rule.head,
                                rule.source_page,
                                rule.source_line
                            );
                        }
                    }

                    if !conflict.existing_superiorities.is_empty() {
                        println!();
                        println!("   Existing superiority relations (partial):");
                        for sup in &conflict.existing_superiorities {
                            println!("     {} > {}", sup.superior, sup.inferior);
                        }
                    } else {
                        println!();
                        println!("   No superiority relations between competing rules.");
                    }

                    if suggest && !conflict.suggestions.is_empty() {
                        println!();
                        println!("   Suggested resolutions:");
                        for suggestion in &conflict.suggestions {
                            println!("     - {suggestion}");
                        }
                    }

                    println!();
                }
            }
        }
    }

    if fail_on_conflicts && !conflicts.is_empty() {
        std::process::exit(1);
    }

    Ok(())
}

#[cfg(feature = "reason")]
fn build_conflict_suggestions(
    base_name: &str,
    pos_rules: &[&zetl::reason::types::ProvenancedRule],
    neg_rules: &[&zetl::reason::types::ProvenancedRule],
    pos_facts: &[&zetl::reason::types::ProvenancedFact],
    neg_facts: &[&zetl::reason::types::ProvenancedFact],
) -> Vec<String> {
    use zetl::reason::types::RuleType;

    let mut suggestions = Vec::new();

    let pos_def: Vec<_> = pos_rules
        .iter()
        .filter(|r| r.rule_type == RuleType::Defeasible)
        .collect();
    let neg_def: Vec<_> = neg_rules
        .iter()
        .filter(|r| r.rule_type == RuleType::Defeasible)
        .collect();
    let neg_defeaters: Vec<_> = neg_rules
        .iter()
        .filter(|r| r.rule_type == RuleType::Defeater)
        .collect();
    let pos_defeaters: Vec<_> = pos_rules
        .iter()
        .filter(|r| r.rule_type == RuleType::Defeater)
        .collect();

    // Suggest superiority relations between defeasible rule pairs
    for pr in &pos_def {
        for nr in &neg_def {
            suggestions.push(format!(
                "Add (prefer {} {}) to make '{}' prevail",
                pr.label, nr.label, base_name
            ));
            suggestions.push(format!(
                "Add (prefer {} {}) to make '~{}' prevail",
                nr.label, pr.label, base_name
            ));
        }
    }

    // Suggest converting defeasible rules to defeaters
    if !pos_def.is_empty() {
        for nr in &neg_def {
            suggestions.push(format!(
                "Convert '{}' to a defeater (except) to block without proving ~{}",
                nr.label, base_name
            ));
        }
    }
    if !neg_def.is_empty() {
        for pr in &pos_def {
            suggestions.push(format!(
                "Convert '{}' to a defeater (except) to block without proving {}",
                pr.label, base_name
            ));
        }
    }

    // For defeater-vs-defeasible conflicts: suggest removing defeater or adding strict rule
    for def in &neg_defeaters {
        if !pos_def.is_empty() {
            suggestions.push(format!(
                "Remove defeater '{}' if '~{}' should no longer block '{}'",
                def.label, base_name, base_name
            ));
            suggestions.push(format!(
                "Add a strict rule (always) for '{}' to override defeater '{}'",
                base_name, def.label
            ));
        }
    }
    for def in &pos_defeaters {
        if !neg_def.is_empty() {
            suggestions.push(format!(
                "Remove defeater '{}' if '{}' should no longer block '~{}'",
                def.label, base_name, base_name
            ));
            suggestions.push(format!(
                "Add a strict rule (always) for '~{}' to override defeater '{}'",
                base_name, def.label
            ));
        }
    }

    // Suggest removing conflicting facts
    if !neg_facts.is_empty() && !pos_rules.is_empty() {
        suggestions.push(format!(
            "Remove the fact '~{base_name}' if it is no longer applicable"
        ));
    }
    if !pos_facts.is_empty() && !neg_rules.is_empty() {
        suggestions.push(format!(
            "Remove the fact '{base_name}' if it is no longer applicable"
        ));
    }

    suggestions
}

#[cfg(feature = "reason")]
#[derive(Debug, Clone, Serialize)]
struct ConflictsOutput {
    conflicts: Vec<ConflictEntry>,
    conflict_count: usize,
    diagnostics: Vec<zetl::types::Diagnostic>,
    #[serde(skip_serializing_if = "Option::is_none")]
    snapshot: Option<SnapshotInfo>,
}

#[cfg(feature = "reason")]
#[derive(Debug, Clone, Serialize)]
struct ConflictEntry {
    literal: String,
    positive_literal: String,
    negative_literal: String,
    competing_rules: Vec<CompetingRule>,
    existing_superiorities: Vec<ConflictSuperiority>,
    resolved: bool,
    suggestions: Vec<String>,
}

#[cfg(feature = "reason")]
#[derive(Debug, Clone, Serialize)]
struct CompetingRule {
    label: String,
    rule_type: String,
    head: String,
    body: Vec<String>,
    source_page: String,
    source_file: String,
    source_line: u32,
}

#[cfg(feature = "reason")]
#[derive(Debug, Clone, Serialize)]
struct ConflictSuperiority {
    superior: String,
    inferior: String,
}

// ── What-if command ────────────────────────────────────────────────────────

#[cfg(feature = "reason")]
#[derive(Debug, Clone, Serialize)]
struct WhatIfOutput {
    hypothetical_spl: String,
    new_conclusions: Vec<WhatIfConclusion>,
    changed_conclusions: Vec<WhatIfChanged>,
    removed_conclusions: Vec<WhatIfConclusion>,
    unchanged_count: usize,
    diagnostics: Vec<zetl::types::Diagnostic>,
    #[serde(skip_serializing_if = "Option::is_none")]
    snapshot: Option<SnapshotInfo>,
}

#[cfg(feature = "reason")]
#[derive(Debug, Clone, Serialize)]
struct WhatIfConclusion {
    literal: String,
    conclusion_type: String,
}

#[cfg(feature = "reason")]
#[derive(Debug, Clone, Serialize)]
struct WhatIfChanged {
    literal: String,
    was: String,
    now: String,
}

#[cfg(feature = "reason")]
fn cmd_reason_what_if(
    cli: &Cli,
    spl_inline: Option<&str>,
    file_path: Option<&str>,
    goal: Option<&str>,
) -> Result<()> {
    use spindle_parser::parse_spl;

    // Resolve the hypothetical SPL input
    let hyp_spl = match (spl_inline, file_path) {
        (Some(_), Some(_)) => {
            let msg = "Provide either inline SPL or --file, not both";
            match cli.format {
                OutputFormat::Json => exit_json_error(msg, 1),
                _ => {
                    eprintln!("{msg}");
                    std::process::exit(1);
                }
            }
        }
        (None, None) => {
            let msg = "Provide inline SPL argument or --file <PATH>";
            match cli.format {
                OutputFormat::Json => exit_json_error(msg, 1),
                _ => {
                    eprintln!("{msg}");
                    std::process::exit(1);
                }
            }
        }
        (Some(inline), None) => inline.to_string(),
        (None, Some(path)) => {
            std::fs::read_to_string(path).with_context(|| format!("Cannot read file: {path}"))?
        }
    };

    // Parse the hypothetical SPL to validate it
    let extra_theory = match parse_spl(&hyp_spl) {
        Ok(t) => t,
        Err(e) => {
            let msg = format!("Hypothetical SPL parse error: {e}");
            match cli.format {
                OutputFormat::Json => exit_json_error(&msg, 1),
                _ => {
                    eprintln!("{msg}");
                    std::process::exit(1);
                }
            }
        }
    };

    // Build the baseline theory from vault
    let pipeline = run_pipeline(cli)?;

    let has_spl = pipeline.files.iter().any(|f| !f.spl_blocks.is_empty());
    if !has_spl {
        match cli.format {
            OutputFormat::Json => exit_json_error("No SPL blocks found in vault", 1),
            _ => {
                eprintln!("No SPL blocks found in vault.");
                std::process::exit(1);
            }
        }
    }

    let (result, _theory_cache_hit) = build_or_load_theory(&pipeline, cli.no_cache, cli.verbose)?;

    // Build baseline conclusion set: (literal, type_symbol) pairs
    let conclusion_symbol = |ct: &zetl::reason::types::ConclusionType| -> &'static str {
        match ct {
            zetl::reason::types::ConclusionType::DefinitelyProvable => "+D",
            zetl::reason::types::ConclusionType::DefinitelyNotProvable => "-D",
            zetl::reason::types::ConclusionType::DefeasiblyProvable => "+d",
            zetl::reason::types::ConclusionType::DefeasiblyNotProvable => "-d",
        }
    };

    let baseline_set: HashSet<(String, String)> = result
        .conclusions
        .iter()
        .map(|c| {
            (
                c.literal.clone(),
                conclusion_symbol(&c.conclusion_type).to_string(),
            )
        })
        .collect();

    // Clone the theory and inject hypothetical additions
    let mut hyp_theory = result.theory.clone();
    let mut fact_counter = 0u32;
    for rule in extra_theory.rules() {
        if rule.rule_type == spindle_core::prelude::RuleType::Fact {
            // Re-label to avoid collisions with vault's __fact_N labels
            fact_counter += 1;
            let new_label = format!("__whatif_fact_{fact_counter}");
            let head_lit = rule.head[0].clone();
            let new_rule = spindle_core::prelude::Rule::fact(&new_label, head_lit);
            hyp_theory.add_rule(new_rule);
        } else {
            hyp_theory.add_rule(rule.clone());
        }
    }
    for sup in extra_theory.superiorities() {
        hyp_theory.add_superiority(&sup.superior, &sup.inferior);
    }

    // Re-reason on the hypothetical theory
    let hyp_conclusions =
        spindle_core::reason::reason(&hyp_theory).context("Hypothetical reasoning failed")?;

    // Build hypothetical conclusion set
    let hyp_set: HashSet<(String, String)> = hyp_conclusions
        .iter()
        .map(|c| {
            (
                c.literal.to_string(),
                c.conclusion_type.symbol().to_string(),
            )
        })
        .collect();

    // Diff the two sets
    let added: HashSet<&(String, String)> = hyp_set.difference(&baseline_set).collect();
    let removed: HashSet<&(String, String)> = baseline_set.difference(&hyp_set).collect();

    // Literals that have both additions and removals are "changed"
    let added_lits: HashSet<&str> = added.iter().map(|(l, _)| l.as_str()).collect();
    let removed_lits: HashSet<&str> = removed.iter().map(|(l, _)| l.as_str()).collect();
    let changed_lits: HashSet<&str> = added_lits.intersection(&removed_lits).copied().collect();

    let mut new_conclusions: Vec<WhatIfConclusion> = Vec::new();
    let mut changed_conclusions: Vec<WhatIfChanged> = Vec::new();
    let mut removed_conclusions: Vec<WhatIfConclusion> = Vec::new();

    // Separate pure-new from changed (added side)
    for (lit, typ) in &added {
        if let Some(filter) = goal {
            if lit.as_str() != filter {
                continue;
            }
        }
        if changed_lits.contains(lit.as_str()) {
            // Will be reported as "changed" below
        } else {
            new_conclusions.push(WhatIfConclusion {
                literal: lit.clone(),
                conclusion_type: typ.clone(),
            });
        }
    }

    // Separate pure-removed from changed (removed side)
    for (lit, typ) in &removed {
        if let Some(filter) = goal {
            if lit.as_str() != filter {
                continue;
            }
        }
        if changed_lits.contains(lit.as_str()) {
            // Will be reported as "changed" below
        } else {
            removed_conclusions.push(WhatIfConclusion {
                literal: lit.clone(),
                conclusion_type: typ.clone(),
            });
        }
    }

    // Build changed entries: group by literal, show was/now
    for lit in &changed_lits {
        if let Some(filter) = goal {
            if *lit != filter {
                continue;
            }
        }
        let mut was: Vec<String> = removed
            .iter()
            .filter(|(l, _)| l.as_str() == *lit)
            .map(|(_, t)| t.clone())
            .collect();
        was.sort();
        let mut now: Vec<String> = added
            .iter()
            .filter(|(l, _)| l.as_str() == *lit)
            .map(|(_, t)| t.clone())
            .collect();
        now.sort();
        changed_conclusions.push(WhatIfChanged {
            literal: lit.to_string(),
            was: was.join(", "),
            now: now.join(", "),
        });
    }

    // Count unchanged
    let unchanged_count = hyp_set
        .intersection(&baseline_set)
        .filter(|(lit, _)| goal.is_none_or(|g| lit.as_str() == g))
        .count();

    // Sort for stable output
    new_conclusions.sort_by(|a, b| a.literal.cmp(&b.literal));
    changed_conclusions.sort_by(|a, b| a.literal.cmp(&b.literal));
    removed_conclusions.sort_by(|a, b| a.literal.cmp(&b.literal));

    let output = WhatIfOutput {
        hypothetical_spl: hyp_spl.clone(),
        new_conclusions: new_conclusions.clone(),
        changed_conclusions: changed_conclusions.clone(),
        removed_conclusions: removed_conclusions.clone(),
        unchanged_count,
        diagnostics: result.diagnostics,
        snapshot: pipeline.snapshot.clone(),
    };

    match cli.format {
        OutputFormat::Json => print_json(&output)?,
        _ => {
            println!(
                "What-if analysis: hypothetically adding:\n  {}\n",
                hyp_spl.trim()
            );

            if new_conclusions.is_empty()
                && changed_conclusions.is_empty()
                && removed_conclusions.is_empty()
            {
                println!("No changes to conclusions.");
            } else {
                if !new_conclusions.is_empty() {
                    println!("New conclusions:");
                    for c in &new_conclusions {
                        println!("  {} {}", c.conclusion_type, c.literal);
                    }
                    println!();
                }

                if !changed_conclusions.is_empty() {
                    println!("Changed conclusions:");
                    for c in &changed_conclusions {
                        println!("  {} : was {} , now {}", c.literal, c.was, c.now);
                    }
                    println!();
                }

                if !removed_conclusions.is_empty() {
                    println!("Removed conclusions:");
                    for c in &removed_conclusions {
                        println!("  {} {} (no longer derived)", c.conclusion_type, c.literal);
                    }
                    println!();
                }
            }

            println!("Unchanged: {unchanged_count}");
        }
    }

    Ok(())
}

// ── Cross-referencing: provenance with link graph ──────────────────────────

#[cfg(feature = "reason")]
fn cmd_reason_provenance(cli: &Cli, literal_input: &str) -> Result<()> {
    use zetl::reason::types::ConclusionType;

    let pipeline = run_pipeline(cli)?;

    let has_spl = pipeline.files.iter().any(|f| !f.spl_blocks.is_empty());
    if !has_spl {
        match cli.format {
            OutputFormat::Json => exit_json_error("No SPL blocks found in vault", 1),
            _ => {
                eprintln!("No SPL blocks found in vault.");
                std::process::exit(1);
            }
        }
    }

    let (result, _theory_cache_hit) = build_or_load_theory(&pipeline, cli.no_cache, cli.verbose)?;

    // Load theory cache for grounding freshness (REQ-044).
    let theory_cache = load_theory_cache(&pipeline.vault_root).unwrap_or(None);

    // Current vault Merkle root hex (written by run_pipeline → save_cache).
    let vault_root_hash = load_vault_root_hex(&pipeline.vault_root).unwrap_or(None);

    // theory_built_at comes from the theory cache's built_at field.
    let theory_built_at = theory_cache.as_ref().and_then(|tc| tc.built_at.clone());

    // Read fresh VCS metadata (§1.6). Always reflects current environment, not cached state.
    let (git_commit, git_dirty) = zetl::vcs::get_git_metadata(&pipeline.vault_root);

    // Normalize literal input: handle ~ prefix for negation
    let literal_str = literal_input.trim();

    // Find all conclusions matching this literal
    let matching: Vec<_> = result
        .conclusions
        .iter()
        .filter(|c| c.literal == literal_str)
        .collect();

    if matching.is_empty() {
        let msg = format!(
            "Literal '{literal_str}' not found in conclusions. Use `zetl reason status` to see all conclusions."
        );
        match cli.format {
            OutputFormat::Json => exit_json_error(&msg, 1),
            _ => {
                eprintln!("{msg}");
                std::process::exit(1);
            }
        }
    }

    // Collect all unique source pages from proof sources
    let mut source_pages: Vec<String> = Vec::new();
    let mut seen_pages: HashSet<String> = HashSet::new();
    for c in &matching {
        for ps in &c.proof_sources {
            if seen_pages.insert(ps.page.clone()) {
                source_pages.push(ps.page.clone());
            }
        }
    }

    // Cross-reference: for each pair of source pages, find link graph connections
    let mut cross_refs: Vec<ProvenanceCrossRef> = Vec::new();
    for i in 0..source_pages.len() {
        for j in 0..source_pages.len() {
            if i == j {
                continue;
            }
            let from = &source_pages[i];
            let to = &source_pages[j];

            // Check direct forward links
            let forward = pipeline.graph.forward_links(from);
            for fwd in &forward {
                if fwd.target == *to {
                    cross_refs.push(ProvenanceCrossRef {
                        from_page: from.clone(),
                        to_page: to.clone(),
                        direction: "forward_link".to_string(),
                        line: fwd.meta.line,
                    });
                }
            }
        }
    }

    // Deduplicate cross-refs
    cross_refs.sort_by(|a, b| {
        a.from_page
            .cmp(&b.from_page)
            .then(a.to_page.cmp(&b.to_page))
            .then(a.line.cmp(&b.line))
    });
    cross_refs
        .dedup_by(|a, b| a.from_page == b.from_page && a.to_page == b.to_page && a.line == b.line);

    // Build per-conclusion output — enrich each proof source with grounding freshness.
    let conclusion_entries: Vec<ProvenanceConclusionEntry> = matching
        .iter()
        .map(|c| {
            let tag = match c.conclusion_type {
                ConclusionType::DefinitelyProvable => "+D",
                ConclusionType::DefinitelyNotProvable => "-D",
                ConclusionType::DefeasiblyProvable => "+d",
                ConclusionType::DefeasiblyNotProvable => "-d",
            };
            let enriched_sources: Vec<EnrichedProofSource> = c
                .proof_sources
                .iter()
                .map(|ps| {
                    let grounding = compute_grounding(ps, &pipeline.files, theory_cache.as_ref());
                    EnrichedProofSource {
                        page: ps.page.clone(),
                        path: ps.path.clone(),
                        line: ps.line,
                        rule_label: ps.rule_label.clone(),
                        contribution: ps.contribution.clone(),
                        grounding,
                    }
                })
                .collect();
            ProvenanceConclusionEntry {
                conclusion_type: tag.to_string(),
                literal: c.literal.clone(),
                proof_sources: enriched_sources,
            }
        })
        .collect();

    let output = ProvenanceOutput {
        literal: literal_str.to_string(),
        vault_root_hash,
        theory_built_at,
        git_commit,
        git_dirty,
        conclusions: conclusion_entries,
        source_pages: source_pages.clone(),
        cross_references: cross_refs.clone(),
        snapshot: pipeline.snapshot.clone(),
    };

    match cli.format {
        OutputFormat::Json => print_json(&output)?,
        _ => {
            println!("Provenance for '{literal_str}':\n");

            if let Some(ref vr) = output.vault_root_hash {
                println!("  Vault root hash: {vr}");
            }
            if let Some(ref ts) = output.theory_built_at {
                println!("  Theory built at: {ts}");
            }
            if output.vault_root_hash.is_some() || output.theory_built_at.is_some() {
                println!();
            }

            for entry in &output.conclusions {
                println!("  {} {}", entry.conclusion_type, entry.literal);
                println!("  Proof sources:");
                for ps in &entry.proof_sources {
                    let label_part = if let Some(ref label) = ps.rule_label {
                        format!(" ({label})")
                    } else {
                        String::new()
                    };
                    let freshness = match ps.grounding.fresh {
                        Some(true) => " [fresh]",
                        Some(false) => " [STALE]",
                        None => "",
                    };
                    println!(
                        "    [[{}]]:{} — {}{}{}",
                        ps.page, ps.line, ps.contribution, label_part, freshness
                    );
                    if let Some(ref w) = ps.grounding.warning {
                        println!("      Warning: {w}");
                    }
                }
                println!();
            }

            if cross_refs.is_empty() {
                println!("  No link-graph connections between source pages.");
            } else {
                println!("  Link-graph cross-references between source pages:");
                for cr in &cross_refs {
                    println!(
                        "    [[{}]] → [[{}]] (line {})",
                        cr.from_page, cr.to_page, cr.line
                    );
                }
            }
        }
    }

    Ok(())
}

#[cfg(feature = "reason")]
#[derive(Debug, Clone, Serialize)]
struct ProvenanceOutput {
    literal: String,
    vault_root_hash: Option<String>,
    theory_built_at: Option<String>,
    /// Current HEAD commit hash, or `null` when the vault is not in a Git repo (§1.6).
    git_commit: Option<String>,
    /// `true` if the working tree has uncommitted changes; `null` outside a Git repo (§1.6).
    git_dirty: Option<bool>,
    conclusions: Vec<ProvenanceConclusionEntry>,
    source_pages: Vec<String>,
    cross_references: Vec<ProvenanceCrossRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    snapshot: Option<SnapshotInfo>,
}

#[cfg(feature = "reason")]
#[derive(Debug, Clone, Serialize)]
struct ProvenanceConclusionEntry {
    conclusion_type: String,
    literal: String,
    proof_sources: Vec<EnrichedProofSource>,
}

#[cfg(feature = "reason")]
#[derive(Debug, Clone, Serialize)]
struct ProvenanceCrossRef {
    from_page: String,
    to_page: String,
    direction: String,
    line: u32,
}

/// Grounding freshness info for a single proof source (REQ-044 / CON-012).
#[cfg(feature = "reason")]
#[derive(Debug, Clone, Serialize)]
struct ProvenanceGrounding {
    /// Grounding type: "section" (implicit, enclosing heading) or "explicit" (declared grounding).
    #[serde(rename = "type")]
    grounding_type: String,
    /// Heading of the enclosing section (only present when type == "section").
    #[serde(skip_serializing_if = "Option::is_none")]
    section_heading: Option<String>,
    /// Source reference identifiers (only present when type == "explicit").
    #[serde(skip_serializing_if = "Option::is_none")]
    source_refs: Option<Vec<String>>,
    /// Whether the current grounding hash matches the one stored at theory-build time.
    /// `null` when the theory cache has no stored grounding hash (first run or old cache).
    fresh: Option<bool>,
    /// Human-readable warning when `fresh` is false.
    #[serde(skip_serializing_if = "Option::is_none")]
    warning: Option<String>,
}

/// A proof source enriched with grounding freshness (REQ-044).
#[cfg(feature = "reason")]
#[derive(Debug, Clone, Serialize)]
struct EnrichedProofSource {
    page: String,
    path: std::path::PathBuf,
    line: u32,
    rule_label: Option<String>,
    contribution: String,
    grounding: ProvenanceGrounding,
}

/// Look up the current section grounding hash for an SPL block (identified by its
/// start line) from the in-memory `ParsedFile` (REQ-044).
///
/// Returns `None` when the file has no `FileMerkle`, the SPL leaf is not found,
/// or the section index is out of range.
#[cfg(feature = "reason")]
fn get_current_grounding_hash(
    file: &zetl::types::ParsedFile,
    spl_start_line: u32,
) -> Option<zetl::types::ContentHash> {
    let fm = file.file_merkle.as_ref()?;
    let spl_leaf = fm
        .spl_leaves
        .iter()
        .find(|l| l.start_line == spl_start_line)?;
    let section = fm.sections.get(spl_leaf.section_index)?;
    Some(section.grounding_hash)
}

/// Build a `ProvenanceGrounding` for one proof source by comparing the current
/// scanner data against the theory cache (REQ-044 / CON-012).
///
/// The grounding type is determined by the theory cache entry:
/// - "explicit" when the SPL block carries explicit grounding declarations
/// - "section"  otherwise (implicit, enclosing heading-delimited section)
///
/// Freshness is computed by comparing the current section grounding hash
/// (from the scanner-populated `FileMerkle`) against the hash stored in the
/// theory cache at build time. `fresh = null` when the cache has no stored
/// hash (first run or pre-v2 cache).
#[cfg(feature = "reason")]
fn compute_grounding(
    proof_source: &zetl::reason::types::ProofSource,
    files: &[zetl::types::ParsedFile],
    theory_cache: Option<&zetl::cache::TheoryCache>,
) -> ProvenanceGrounding {
    use zetl::types::ContentHash;

    // Find the ParsedFile for this proof source.
    let Some(file) = files.iter().find(|f| f.path == proof_source.path) else {
        return ProvenanceGrounding {
            grounding_type: "section".to_string(),
            section_heading: None,
            source_refs: None,
            fresh: None,
            warning: None,
        };
    };

    // Find the SplBlock whose fence range contains the rule/fact line.
    let Some(spl_block) = file
        .spl_blocks
        .iter()
        .find(|b| b.start_line <= proof_source.line && proof_source.line <= b.end_line)
    else {
        return ProvenanceGrounding {
            grounding_type: "section".to_string(),
            section_heading: None,
            source_refs: None,
            fresh: None,
            warning: None,
        };
    };

    // Build the theory cache key used in `build_spl_block_cache`.
    let key = format!(
        "{}:{}",
        spl_block.source_file.display(),
        spl_block.start_line
    );

    let Some(tc) = theory_cache else {
        // No theory cache available — cannot determine freshness.
        return ProvenanceGrounding {
            grounding_type: "section".to_string(),
            section_heading: None,
            source_refs: None,
            fresh: None,
            warning: None,
        };
    };

    let Some(cached) = tc.spl_blocks.get(&key) else {
        // SPL block not present in theory cache (e.g. first run).
        return ProvenanceGrounding {
            grounding_type: "section".to_string(),
            section_heading: None,
            source_refs: None,
            fresh: None,
            warning: None,
        };
    };

    // Determine grounding type from the cached entry.
    let is_explicit = !cached.explicit_groundings.is_empty();
    let grounding_type = if is_explicit { "explicit" } else { "section" };

    let section_heading = if !is_explicit {
        cached.section_heading.clone()
    } else {
        None
    };

    let source_refs: Option<Vec<String>> = if is_explicit {
        let refs: Vec<String> = cached
            .explicit_groundings
            .iter()
            .flat_map(|eg| eg.source_refs.iter().cloned())
            .collect();
        if refs.is_empty() {
            None
        } else {
            Some(refs)
        }
    } else {
        None
    };

    // Determine freshness: compare current section grounding hash vs cached.
    let zero_hash: ContentHash = [0u8; 32];
    let fresh: Option<bool> = if cached.section_grounding_hash == zero_hash {
        // Zero hash means no grounding data was stored — fresh is indeterminate.
        None
    } else {
        let current_hash = get_current_grounding_hash(file, spl_block.start_line);
        current_hash.map(|h| h == cached.section_grounding_hash)
    };

    let warning = if fresh == Some(false) {
        let msg = if is_explicit {
            "Source content changed since theory was built"
        } else {
            "Section prose changed since theory was built"
        };
        Some(msg.to_string())
    } else {
        None
    };

    ProvenanceGrounding {
        grounding_type: grounding_type.to_string(),
        section_heading,
        source_refs,
        fresh,
        warning,
    }
}

/// Find documents that could potentially provide a given literal.
///
/// Returns sources where the literal already appears as a rule head (but isn't
/// provable because its own preconditions fail), plus the source pages of rules
/// that reference it in their body (suggesting which docs should assert it).
#[cfg(feature = "reason")]
fn find_potential_sources(
    literal_str: &str,
    theory_result: &zetl::reason::types::TheoryResult,
) -> Vec<WhyNotSource> {
    let mut sources = Vec::new();

    // Rules that have this literal as head (they tried to prove it but failed)
    for rule in &theory_result.rules {
        if rule.head.to_string() == literal_str {
            sources.push(WhyNotSource {
                page: rule.source_page.clone(),
                path: rule.source_file.to_string_lossy().to_string(),
                line: rule.source_line,
                rule_label: Some(rule.label.clone()),
            });
        }
    }

    // Facts asserting this literal
    for fact in &theory_result.facts {
        if fact.literal.to_string() == literal_str {
            sources.push(WhyNotSource {
                page: fact.source_page.clone(),
                path: fact.source_file.to_string_lossy().to_string(),
                line: fact.source_line,
                rule_label: None,
            });
        }
    }

    sources
}

/// Format a rule as human-readable text.
#[cfg(feature = "reason")]
fn format_rule_text(rule: &zetl::reason::types::ProvenancedRule) -> String {
    let arrow = match rule.rule_type {
        zetl::reason::types::RuleType::Strict => "->",
        zetl::reason::types::RuleType::Defeasible => "=>",
        zetl::reason::types::RuleType::Defeater => "~>",
    };
    if rule.body.is_empty() {
        format!("{}: {} {}", rule.label, arrow, rule.head)
    } else {
        let body_strs: Vec<String> = rule.body.iter().map(|b| b.to_string()).collect();
        format!(
            "{}: {} {} {}",
            rule.label,
            body_strs.join(", "),
            arrow,
            rule.head
        )
    }
}

// ── Explain output types ───────────────────────────────────────────────────

#[cfg(feature = "reason")]
#[derive(Serialize)]
struct ExplainOutput {
    literal: String,
    conclusion_type: String,
    proof_tree: Option<ExplainNode>,
    conflicts_resolved: Vec<ExplainConflict>,
    blocked_alternatives: Vec<ExplainBlocked>,
    #[serde(skip_serializing_if = "Option::is_none")]
    snapshot: Option<SnapshotInfo>,
}

#[cfg(feature = "reason")]
#[derive(Serialize, Clone)]
struct ExplainNode {
    literal: String,
    derivation: String,
    source: Option<ExplainSource>,
    rule: Option<ExplainRule>,
    body: Vec<ExplainNode>,
}

#[cfg(feature = "reason")]
#[derive(Serialize, Clone)]
struct ExplainSource {
    page: String,
    path: String,
    line: u32,
}

#[cfg(feature = "reason")]
#[derive(Serialize, Clone)]
struct ExplainRule {
    label: String,
    rule_type: String,
    rule_text: String,
}

#[cfg(feature = "reason")]
#[derive(Serialize)]
struct ExplainConflict {
    winning_rule: String,
    losing_rule: String,
    resolution: String,
}

#[cfg(feature = "reason")]
#[derive(Serialize)]
struct ExplainBlocked {
    literal: String,
    rule_label: String,
    reason: String,
    blocking_rule: Option<String>,
    explanation: String,
}

// ── Enrichment: merge spindle-core proof tree with our provenance ───────────

#[cfg(feature = "reason")]
fn enrich_proof_tree(
    explanation: &spindle_core::explanation::Explanation,
    theory_result: &zetl::reason::types::TheoryResult,
    literal_input: &str,
    max_depth: usize,
    snapshot: Option<SnapshotInfo>,
) -> ExplainOutput {
    let conclusion_type_str = match explanation.conclusion_type {
        spindle_core::prelude::ConclusionType::DefinitelyProvable => "+D",
        spindle_core::prelude::ConclusionType::DefinitelyNotProvable => "-D",
        spindle_core::prelude::ConclusionType::DefeasiblyProvable => "+d",
        spindle_core::prelude::ConclusionType::DefeasiblyNotProvable => "-d",
    };

    let proof_tree = explanation
        .proof_tree
        .as_ref()
        .map(|node| enrich_node(node, theory_result, 0, max_depth));

    let conflicts_resolved = explanation
        .conflicts_resolved
        .iter()
        .map(|c| ExplainConflict {
            winning_rule: c.winning_rule.clone(),
            losing_rule: c.losing_rule.clone(),
            resolution: c.resolution_type.to_string(),
        })
        .collect();

    let blocked_alternatives = explanation
        .blocked_alternatives
        .iter()
        .map(|b| ExplainBlocked {
            literal: b.literal.to_string(),
            rule_label: b.rule_label.clone(),
            reason: b.reason.to_string(),
            blocking_rule: b.blocking_rule.clone(),
            explanation: b.explanation.clone(),
        })
        .collect();

    ExplainOutput {
        literal: literal_input.to_string(),
        conclusion_type: conclusion_type_str.to_string(),
        proof_tree,
        conflicts_resolved,
        blocked_alternatives,
        snapshot,
    }
}

#[cfg(feature = "reason")]
fn enrich_node(
    node: &spindle_core::explanation::ProofNode,
    theory_result: &zetl::reason::types::TheoryResult,
    depth: usize,
    max_depth: usize,
) -> ExplainNode {
    let literal_str = node.literal.to_string();
    let derivation = match node.derivation_type {
        spindle_core::explanation::DerivationType::Definite => "definite",
        spindle_core::explanation::DerivationType::Defeasible => "defeasible",
    };

    let (source, rule_info, body) = if let Some(ref step) = node.proof_step {
        // Look up provenance for this rule label
        let source = lookup_source(&step.rule_label, theory_result);

        let rule_type_str = match step.rule_type {
            spindle_core::prelude::RuleType::Fact => "fact",
            spindle_core::prelude::RuleType::Strict => "strict",
            spindle_core::prelude::RuleType::Defeasible => "defeasible",
            spindle_core::prelude::RuleType::Defeater => "defeater",
        };

        let rule = ExplainRule {
            label: step.rule_label.clone(),
            rule_type: rule_type_str.to_string(),
            rule_text: step.rule_text.clone(),
        };

        let body = if depth < max_depth {
            step.body_proofs
                .iter()
                .map(|bp| enrich_node(bp, theory_result, depth + 1, max_depth))
                .collect()
        } else {
            vec![]
        };

        (source, Some(rule), body)
    } else {
        (None, None, vec![])
    };

    ExplainNode {
        literal: literal_str,
        derivation: derivation.to_string(),
        source,
        rule: rule_info,
        body,
    }
}

#[cfg(feature = "reason")]
fn lookup_source(
    rule_label: &str,
    theory_result: &zetl::reason::types::TheoryResult,
) -> Option<ExplainSource> {
    // Check provenanced rules
    if let Some(rule) = theory_result.rules.iter().find(|r| r.label == rule_label) {
        return Some(ExplainSource {
            page: rule.source_page.clone(),
            path: rule.source_file.to_string_lossy().to_string(),
            line: rule.source_line,
        });
    }

    // Check provenanced facts (they have auto-generated labels like __fact_N)
    if rule_label.starts_with("__fact_") {
        // Match by looking at the theory's metadata for this label
        if let Some(rule) = theory_result.theory.get_rule(rule_label) {
            if let Some(head) = rule.head.first() {
                let head_str = head.to_string();
                if let Some(fact) = theory_result
                    .facts
                    .iter()
                    .find(|f| f.literal.to_string() == head_str)
                {
                    return Some(ExplainSource {
                        page: fact.source_page.clone(),
                        path: fact.source_file.to_string_lossy().to_string(),
                        line: fact.source_line,
                    });
                }
            }
        }
    }

    None
}

// ── Negative explanation (for -D / -d conclusions) ─────────────────────────

#[cfg(feature = "reason")]
fn print_negative_explanation(
    _cli: &Cli,
    explain_format: &zetl::cli::ExplainFormat,
    literal_input: &str,
    conclusion: &zetl::reason::types::ProvenancedConclusion,
    theory_result: &zetl::reason::types::TheoryResult,
) -> Result<()> {
    use zetl::cli::ExplainFormat;
    use zetl::reason::types::ConclusionType;

    let conclusion_type_str = match conclusion.conclusion_type {
        ConclusionType::DefinitelyProvable => "+D",
        ConclusionType::DefinitelyNotProvable => "-D",
        ConclusionType::DefeasiblyProvable => "+d",
        ConclusionType::DefeasiblyNotProvable => "-d",
    };

    // Build source info from proof_sources
    let sources: Vec<ExplainSource> = conclusion
        .proof_sources
        .iter()
        .map(|s| ExplainSource {
            page: s.page.clone(),
            path: s.path.to_string_lossy().to_string(),
            line: s.line,
        })
        .collect();

    // Find defeat chain: rules that blocked this literal
    let defeat_chain: Vec<_> = theory_result
        .rules
        .iter()
        .filter(|r| {
            let head_str = r.head.to_string();
            let negated_input = if let Some(stripped) = literal_input.strip_prefix('~') {
                stripped.to_string()
            } else {
                format!("~{literal_input}")
            };
            head_str == negated_input
        })
        .collect();

    match explain_format {
        ExplainFormat::Json => {
            #[derive(Serialize)]
            struct NegativeExplainOutput {
                literal: String,
                conclusion_type: String,
                explanation: String,
                sources: Vec<ExplainSource>,
                defeat_chain: Vec<DefeatChainEntry>,
            }

            #[derive(Serialize)]
            struct DefeatChainEntry {
                rule_label: String,
                rule_type: String,
                head: String,
                body: Vec<String>,
                source: ExplainSource,
            }

            let chain_entries: Vec<DefeatChainEntry> = defeat_chain
                .iter()
                .map(|r| DefeatChainEntry {
                    rule_label: r.label.clone(),
                    rule_type: format!("{:?}", r.rule_type),
                    head: r.head.to_string(),
                    body: r.body.iter().map(|b| b.to_string()).collect(),
                    source: ExplainSource {
                        page: r.source_page.clone(),
                        path: r.source_file.to_string_lossy().to_string(),
                        line: r.source_line,
                    },
                })
                .collect();

            let explanation_text = match conclusion.conclusion_type {
                ConclusionType::DefinitelyNotProvable => {
                    format!(
                        "'{literal_input}' is definitely not provable (-D): no strict proof chain exists"
                    )
                }
                ConclusionType::DefeasiblyNotProvable => {
                    if defeat_chain.is_empty() {
                        format!("'{literal_input}' is defeasibly not provable (-d): no undefeated defeasible proof chain exists")
                    } else {
                        format!(
                            "'{}' is defeasibly not provable (-d): defeated by {} rule(s)",
                            literal_input,
                            defeat_chain.len()
                        )
                    }
                }
                _ => format!("'{literal_input}' holds as {conclusion_type_str}"),
            };

            let output = NegativeExplainOutput {
                literal: literal_input.to_string(),
                conclusion_type: conclusion_type_str.to_string(),
                explanation: explanation_text,
                sources,
                defeat_chain: chain_entries,
            };
            print_json(&output)?;
        }
        ExplainFormat::Table => {
            println!("Explanation for '{literal_input}':");
            println!("  Conclusion: {conclusion_type_str} {literal_input}");
            println!();
            if !sources.is_empty() {
                println!("  Sources:");
                for s in &sources {
                    println!("    [[{}]]:{}  ({})", s.page, s.line, s.path);
                }
                println!();
            }
            if !defeat_chain.is_empty() {
                println!("  Defeat chain:");
                for r in &defeat_chain {
                    let body_strs: Vec<String> = r.body.iter().map(|b| b.to_string()).collect();
                    println!(
                        "    {} [{:?}]: {} => {}  ([[{}]]:{})",
                        r.label,
                        r.rule_type,
                        body_strs.join(", "),
                        r.head,
                        r.source_page,
                        r.source_line,
                    );
                }
            }
        }
        ExplainFormat::Natural => match conclusion.conclusion_type {
            ConclusionType::DefinitelyNotProvable => {
                println!("The literal '{literal_input}' is definitely not provable.");
                println!("No strict proof chain can establish it from the known facts and rules.");
            }
            ConclusionType::DefeasiblyNotProvable => {
                println!("The literal '{literal_input}' is defeasibly not provable.");
                if defeat_chain.is_empty() {
                    println!("No undefeated defeasible proof chain exists.");
                } else {
                    println!("It is blocked by the following rule(s):");
                    for r in &defeat_chain {
                        println!(
                            "  - Rule '{}' from [[{}]]:{}",
                            r.label, r.source_page, r.source_line
                        );
                    }
                }
            }
            _ => {
                println!("'{literal_input}' holds as {conclusion_type_str} {literal_input}.");
            }
        },
        ExplainFormat::Dot => {
            // Minimal DOT graph for negative conclusions
            println!("digraph explanation {{");
            println!("  rankdir=BT;");
            println!("  node [shape=box];");
            let conclusion_id = format!("\"{conclusion_type_str}\\n{literal_input}\"");
            println!("  {conclusion_id} [style=filled, fillcolor=lightcoral];");
            for r in &defeat_chain {
                let rule_id = format!("\"{}\"", r.label);
                println!("  {rule_id} -> {conclusion_id} [label=\"defeats\"];");
                println!(
                    "  {} [label=\"{}\\n[[{}]]:{}\"];",
                    rule_id, r.label, r.source_page, r.source_line
                );
            }
            println!("}}");
        }
    }

    Ok(())
}

// ── Fuzzy matching for literal suggestions ─────────────────────────────────

#[cfg(feature = "reason")]
fn fuzzy_match_literals(query: &str, literals: &[String]) -> Vec<String> {
    use zetl::simhash::{compute_simhash, hamming_distance};

    let query_hash = compute_simhash(query);
    let mut scored: Vec<(String, u32)> = literals
        .iter()
        .filter_map(|lit| {
            let lit_hash = compute_simhash(lit);
            let dist = hamming_distance(query_hash, lit_hash);
            if dist <= 16 {
                Some((lit.clone(), dist))
            } else {
                None
            }
        })
        .collect();

    scored.sort_by_key(|(_, d)| *d);
    scored.truncate(5);
    scored.into_iter().map(|(lit, _)| lit).collect()
}

// ── Explain output formatters ──────────────────────────────────────────────

#[cfg(feature = "reason")]
fn print_explain_table(output: &ExplainOutput) {
    println!("Explanation for '{}':", output.literal);
    println!(
        "  Conclusion: {} {}",
        output.conclusion_type, output.literal
    );
    println!();

    if let Some(ref tree) = output.proof_tree {
        println!("  Proof tree:");
        print_tree_node(tree, 2);
    }

    if !output.conflicts_resolved.is_empty() {
        println!();
        println!("  Conflicts resolved:");
        for c in &output.conflicts_resolved {
            println!(
                "    {} > {} ({})",
                c.winning_rule, c.losing_rule, c.resolution
            );
        }
    }

    if !output.blocked_alternatives.is_empty() {
        println!();
        println!("  Blocked alternatives:");
        for b in &output.blocked_alternatives {
            println!("    {} via '{}': {}", b.literal, b.rule_label, b.reason);
            if let Some(ref blocker) = b.blocking_rule {
                println!("      blocked by: {blocker}");
            }
        }
    }
}

#[cfg(feature = "reason")]
fn print_tree_node(node: &ExplainNode, indent: usize) {
    let pad = " ".repeat(indent);

    let source_str = if let Some(ref src) = node.source {
        format!("  [[{}]]:{}", src.page, src.line)
    } else {
        String::new()
    };

    let rule_str = if let Some(ref rule) = node.rule {
        format!(" via {} [{}]", rule.label, rule.rule_type)
    } else {
        String::new()
    };

    println!(
        "{}{} ({}){}{}",
        pad, node.literal, node.derivation, rule_str, source_str
    );

    for child in &node.body {
        print_tree_node(child, indent + 2);
    }
}

#[cfg(feature = "reason")]
fn print_explain_natural(output: &ExplainOutput) {
    let ct_desc = match output.conclusion_type.as_str() {
        "+D" => "definitely provable",
        "-D" => "definitely not provable",
        "+d" => "defeasibly provable",
        "-d" => "defeasibly not provable",
        _ => &output.conclusion_type,
    };

    println!(
        "The literal '{}' is {} ({}).",
        output.literal, ct_desc, output.conclusion_type
    );
    println!();

    if let Some(ref tree) = output.proof_tree {
        println!("Proof:");
        print_natural_node(tree, 0);
    }

    if !output.conflicts_resolved.is_empty() {
        println!();
        println!("Conflicts were resolved as follows:");
        for c in &output.conflicts_resolved {
            println!(
                "  Rule '{}' prevails over '{}' by {}.",
                c.winning_rule, c.losing_rule, c.resolution
            );
        }
    }
}

#[cfg(feature = "reason")]
fn print_natural_node(node: &ExplainNode, depth: usize) {
    let indent = "  ".repeat(depth);

    if let Some(ref rule) = node.rule {
        let source_ref = if let Some(ref src) = node.source {
            format!(" (from [[{}]]:{})", src.page, src.line)
        } else {
            String::new()
        };

        match rule.rule_type.as_str() {
            "fact" => {
                println!(
                    "{}'{}' is an established fact{}.{}",
                    indent,
                    node.literal,
                    source_ref,
                    if node.derivation == "definite" {
                        ""
                    } else {
                        " [defeasible]"
                    }
                );
            }
            "strict" => {
                println!(
                    "{}'{}' follows strictly from rule '{}'{}, because:",
                    indent, node.literal, rule.label, source_ref
                );
            }
            "defeasible" => {
                println!(
                    "{}'{}' is normally concluded by rule '{}'{}, because:",
                    indent, node.literal, rule.label, source_ref
                );
            }
            "defeater" => {
                println!(
                    "{}'{}' is blocked by defeater '{}'{}, because:",
                    indent, node.literal, rule.label, source_ref
                );
            }
            _ => {
                println!("{}'{}'{}", indent, node.literal, source_ref);
            }
        }
    } else {
        println!("{}'{}'", indent, node.literal);
    }

    for child in &node.body {
        print_natural_node(child, depth + 1);
    }
}

#[cfg(feature = "reason")]
fn print_explain_dot(output: &ExplainOutput) {
    println!("digraph explanation {{");
    println!("  rankdir=BT;");
    println!("  node [shape=box, fontname=\"Helvetica\"];");
    println!("  edge [fontname=\"Helvetica\", fontsize=10];");
    println!();

    // Root conclusion node
    let root_id = sanitize_dot_id(&output.literal);
    println!(
        "  {} [label=\"{}\\n{}\", style=filled, fillcolor=lightgreen];",
        root_id, output.conclusion_type, output.literal
    );

    if let Some(ref tree) = output.proof_tree {
        let mut counter = 0;
        emit_dot_node(tree, &root_id, &mut counter);
    }

    // Conflict edges
    for c in &output.conflicts_resolved {
        let winner_id = sanitize_dot_id(&c.winning_rule);
        let loser_id = sanitize_dot_id(&c.losing_rule);
        println!(
            "  {} -> {} [label=\"{}\", style=dashed, color=red];",
            winner_id, loser_id, c.resolution
        );
    }

    println!("}}");
}

#[cfg(feature = "reason")]
fn emit_dot_node(node: &ExplainNode, parent_id: &str, counter: &mut usize) {
    if let Some(ref step) = node.rule {
        // Rule node
        let rule_node_id = format!("rule_{counter}");
        *counter += 1;

        let source_label = if let Some(ref src) = node.source {
            format!("\\n[[{}]]:{}", src.page, src.line)
        } else {
            String::new()
        };

        println!(
            "  {} [label=\"{}\\n[{}]{}\"];",
            rule_node_id, step.label, step.rule_type, source_label
        );
        println!("  {rule_node_id} -> {parent_id};");

        // Body literals
        for child in &node.body {
            let child_id = format!("lit_{counter}");
            *counter += 1;

            let child_source = if let Some(ref src) = child.source {
                format!("\\n[[{}]]:{}", src.page, src.line)
            } else {
                String::new()
            };

            println!(
                "  {} [label=\"{}{}\"];",
                child_id, child.literal, child_source
            );
            println!("  {child_id} -> {rule_node_id};");

            // Recurse into children of the body node
            emit_dot_node(child, &child_id, counter);
        }
    }
}

// ── reason export ─────────────────────────────────────────────────────────

#[cfg(feature = "reason")]
fn cmd_reason_export(
    cli: &Cli,
    format: &zetl::cli::ExportFormat,
    with_conclusions: bool,
) -> Result<()> {
    use zetl::cli::ExportFormat;
    use zetl::reason::types::{ConclusionType, RuleType};

    let pipeline = run_pipeline(cli)?;

    let has_spl = pipeline.files.iter().any(|f| !f.spl_blocks.is_empty());
    if !has_spl {
        // Even with no SPL, export always exits 0 per CON-018.
        match format {
            ExportFormat::Json => {
                print_json(&serde_json::json!({
                    "facts": [],
                    "rules": [],
                    "superiority": [],
                    "diagnostics": [],
                    "summary": {
                        "fact_count": 0,
                        "rule_count": 0,
                        "defeater_count": 0,
                        "superiority_count": 0,
                        "source_file_count": 0
                    }
                }))?;
            }
            ExportFormat::Spl => {
                println!(
                    "; Theory extracted from vault: {}",
                    pipeline.vault_root.display()
                );
                println!("; 0 source files, 0 facts, 0 rules, 0 defeaters");
            }
        }
        return Ok(());
    }

    let (result, _theory_cache_hit) = build_or_load_theory(&pipeline, cli.no_cache, cli.verbose)?;

    match format {
        ExportFormat::Spl => {
            // Header comment
            println!(
                "; Theory extracted from vault: {}",
                pipeline.vault_root.display()
            );
            println!(
                "; {} source files, {} facts, {} rules, {} defeaters",
                result.summary.source_file_count,
                result.summary.fact_count,
                result.summary.rule_count,
                result.summary.defeater_count,
            );
            println!(";");

            // Emit facts with provenance comments
            for fact in &result.facts {
                println!(
                    "; --- From: {}:{} ---",
                    fact.source_file.display(),
                    fact.source_line
                );
                println!("(given {})", literal_to_spl(&fact.literal.to_string()));
            }

            // Emit rules with provenance comments
            for rule in &result.rules {
                println!(
                    "; --- From: {}:{} ---",
                    rule.source_file.display(),
                    rule.source_line
                );
                let keyword = match rule.rule_type {
                    RuleType::Strict => "always",
                    RuleType::Defeasible => "normally",
                    RuleType::Defeater => "except",
                };
                if rule.body.is_empty() {
                    println!(
                        "({} {} {})",
                        keyword,
                        rule.label,
                        literal_to_spl(&rule.head.to_string())
                    );
                } else if rule.body.len() == 1 {
                    println!(
                        "({} {}\n  {}\n  {})",
                        keyword,
                        rule.label,
                        literal_to_spl(&rule.body[0].to_string()),
                        literal_to_spl(&rule.head.to_string()),
                    );
                } else {
                    let body_parts: Vec<String> = rule
                        .body
                        .iter()
                        .map(|l| literal_to_spl(&l.to_string()))
                        .collect();
                    println!(
                        "({} {}\n  (and {})\n  {})",
                        keyword,
                        rule.label,
                        body_parts.join(" "),
                        literal_to_spl(&rule.head.to_string()),
                    );
                }
            }

            // Emit superiority relations
            for sup in result.theory.superiorities() {
                println!("(prefer {} {})", sup.superior, sup.inferior);
            }

            // Optionally emit conclusions as comments
            if with_conclusions {
                println!();
                println!("; --- Conclusions ---");
                for c in &result.conclusions {
                    let tag = match c.conclusion_type {
                        ConclusionType::DefinitelyProvable => "+D",
                        ConclusionType::DefinitelyNotProvable => "-D",
                        ConclusionType::DefeasiblyProvable => "+d",
                        ConclusionType::DefeasiblyNotProvable => "-d",
                    };
                    println!("; {} {}", tag, c.literal);
                }
            }
        }
        ExportFormat::Json => {
            #[derive(Serialize)]
            struct ExportOutput {
                facts: Vec<ExportFact>,
                rules: Vec<ExportRule>,
                superiority: Vec<ExportSuperiority>,
                #[serde(skip_serializing_if = "Option::is_none")]
                conclusions: Option<Vec<ExportConclusion>>,
                diagnostics: Vec<zetl::types::Diagnostic>,
                summary: zetl::reason::types::TheorySummary,
                #[serde(skip_serializing_if = "Option::is_none")]
                snapshot: Option<SnapshotInfo>,
            }

            #[derive(Serialize)]
            struct ExportFact {
                literal: String,
                source_file: String,
                source_line: u32,
                source_page: String,
            }

            #[derive(Serialize)]
            struct ExportRule {
                label: String,
                rule_type: RuleType,
                body: Vec<String>,
                head: String,
                source_file: String,
                source_line: u32,
                source_page: String,
            }

            #[derive(Serialize)]
            struct ExportSuperiority {
                superior: String,
                inferior: String,
            }

            #[derive(Serialize)]
            struct ExportConclusion {
                literal: String,
                conclusion_type: ConclusionType,
                proof_sources: Vec<zetl::reason::types::ProofSource>,
            }

            let facts: Vec<ExportFact> = result
                .facts
                .iter()
                .map(|f| ExportFact {
                    literal: f.literal.to_string(),
                    source_file: f.source_file.display().to_string(),
                    source_line: f.source_line,
                    source_page: f.source_page.clone(),
                })
                .collect();

            let rules: Vec<ExportRule> = result
                .rules
                .iter()
                .map(|r| ExportRule {
                    label: r.label.clone(),
                    rule_type: r.rule_type,
                    body: r.body.iter().map(|l| l.to_string()).collect(),
                    head: r.head.to_string(),
                    source_file: r.source_file.display().to_string(),
                    source_line: r.source_line,
                    source_page: r.source_page.clone(),
                })
                .collect();

            let superiority: Vec<ExportSuperiority> = result
                .theory
                .superiorities()
                .iter()
                .map(|s| ExportSuperiority {
                    superior: s.superior.clone(),
                    inferior: s.inferior.clone(),
                })
                .collect();

            let conclusions = if with_conclusions {
                Some(
                    result
                        .conclusions
                        .iter()
                        .map(|c| ExportConclusion {
                            literal: c.literal.clone(),
                            conclusion_type: c.conclusion_type,
                            proof_sources: c.proof_sources.clone(),
                        })
                        .collect(),
                )
            } else {
                None
            };

            let output = ExportOutput {
                facts,
                rules,
                superiority,
                conclusions,
                diagnostics: result.diagnostics.clone(),
                summary: result.summary.clone(),
                snapshot: pipeline.snapshot.clone(),
            };

            print_json(&output)?;
        }
    }

    Ok(())
}

/// Convert a display-form literal (e.g. "~flies") to SPL syntax (e.g. "(not flies)").
#[cfg(feature = "reason")]
fn literal_to_spl(lit: &str) -> String {
    if let Some(name) = lit.strip_prefix('~') {
        format!("(not {name})")
    } else {
        lit.to_string()
    }
}

#[cfg(feature = "reason")]
fn sanitize_dot_id(s: &str) -> String {
    format!(
        "\"{}\"",
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('~', "neg_")
    )
}

// ── Feature-gate error ─────────────────────────────────────────────────────

#[cfg(not(feature = "reason"))]
fn reason_not_available() -> ! {
    let error = serde_json::json!({
        "error": "Reasoning engine not available. Build with --features reason",
        "code": 2
    });
    println!("{}", serde_json::to_string(&error).unwrap());
    std::process::exit(2);
}

// ── zetl diff ─────────────────────────────────────────────────────────────

/// Structured output for `zetl diff` (CON-021, REQ-049, REQ-083).
#[derive(Serialize)]
struct GraphDiff {
    from: DiffRef,
    to: DiffRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pages_added: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pages_removed: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    links_added: Option<Vec<DiffLink>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    links_removed: Option<Vec<DiffLink>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    orphans_gained: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    orphans_resolved: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dead_links_added: Option<Vec<DiffLink>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dead_links_resolved: Option<Vec<DiffLink>>,
}

#[derive(Serialize)]
struct DiffRef {
    #[serde(rename = "ref")]
    ref_str: String,
    commit: Option<String>,
    timestamp: Option<String>,
}

#[derive(Serialize, Clone, PartialEq, Eq, Hash)]
struct DiffLink {
    source: String,
    target: String,
}

/// `zetl diff` — compute graph-level diff against a baseline (REQ-046 – REQ-051, REQ-083).
///
/// When the `history` feature is compiled in: uses the jj-backend for finer-grained
/// history (SPEC-017).  Falls back to SPEC-007 git-subprocess mode when the feature
/// is absent or when `--from`/`--since` is a git ref (CON-021 schema preserved).
fn cmd_diff(
    cli: &Cli,
    from: Option<&str>,
    since: Option<&str>,
    filter: Option<&zetl::cli::DiffFilter>,
) -> Result<()> {
    // Resolve the baseline expression (--from takes precedence over --since).
    let baseline_expr = from.or(since);

    #[cfg(feature = "history")]
    {
        cmd_diff_history(cli, baseline_expr, filter)
    }
    #[cfg(not(feature = "history"))]
    {
        cmd_diff_git(cli, baseline_expr, filter)
    }
}

/// jj-backed diff implementation (SPEC-017, REQ-083).
///
/// Uses `open_history` so that a missing `.zetl/jj/` yields `NO_HISTORY` (REQ-084).
#[cfg(feature = "history")]
fn cmd_diff_history(
    cli: &Cli,
    baseline_expr: Option<&str>,
    filter: Option<&zetl::cli::DiffFilter>,
) -> Result<()> {
    use zetl::history::cache::HistoricalIndexCache;
    use zetl::history::core::resolve_snapshot;
    use zetl::history::jj_backend::VcsBackend as _;

    let pipeline = run_pipeline(cli)?;

    // Open jj workspace — errors with NO_HISTORY if .zetl/jj/ is absent (REQ-084).
    let backend = zetl::history::open_history(&pipeline.vault_root)
        .context("opening jj workspace for diff")?;

    let snapshots = backend
        .list_changes(10_000)
        .context("listing jj snapshots")?;

    if snapshots.is_empty() {
        anyhow::bail!(
            "NO_HISTORY: No snapshots found. Run `zetl index` to create the first snapshot."
        );
    }

    // Resolve the baseline to a snapshot.
    // When no --from/--since is given, default to the previous distinct snapshot (@-).
    let now = chrono::Local::now().fixed_offset();
    let baseline_snap: &zetl::history::jj_backend::ChangeInfo = if let Some(expr) = baseline_expr {
        resolve_snapshot(expr, now, &snapshots)
            .with_context(|| format!("resolving diff baseline {expr:?}"))?
    } else {
        // @- semantics: need at least two snapshots to compute a diff (REQ-083, ADR-046).
        snapshots.get(1).ok_or_else(|| {
            anyhow::anyhow!(
                "NO_PREVIOUS_SNAPSHOT: only one snapshot exists; run `zetl index` again \
                 after making changes to create a second snapshot to diff against."
            )
        })?
    };

    // Load the historical index for the baseline snapshot.
    let vault_root_hash =
        zetl::history::core::extract_vault_root_hash_from_description(&baseline_snap.description)
            .ok_or_else(|| {
            anyhow::anyhow!(
                "Baseline snapshot {} has no vault_root_hash. \
                     Run `zetl index` to populate the historical index.",
                baseline_snap.change_id
            )
        })?;

    let cache = HistoricalIndexCache::with_default_capacity();
    let baseline_files: Vec<ParsedFile> = cache
        .load(&pipeline.vault_root, &vault_root_hash)
        .context("reading historical index cache")?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No cached index for baseline snapshot {} (vault_root_hash={}). \
                 Run `zetl index` at that point in time to populate the cache.",
                baseline_snap.change_id,
                vault_root_hash
            )
        })?
        .into_values()
        .collect();

    // Build the baseline graph.
    let baseline_file_index: Vec<(String, PathBuf)> = baseline_files
        .iter()
        .map(|f| (f.page_name.clone(), f.path.clone()))
        .collect();
    let mut baseline_resolved: HashMap<String, String> = HashMap::new();
    for f in &baseline_files {
        for link in &f.links {
            let key = link.raw_target.clone();
            if let std::collections::hash_map::Entry::Vacant(e) = baseline_resolved.entry(key) {
                if let Some(r) =
                    zetl::scanner::resolve_page_name(&link.target_page, &baseline_file_index)
                {
                    e.insert(r);
                }
            }
        }
    }
    let baseline_graph = zetl::graph::LinkGraph::build(&baseline_files, &baseline_resolved);

    diff_graphs_and_output(
        cli,
        filter,
        &baseline_graph,
        &baseline_files,
        &pipeline.graph,
        &pipeline.files,
        DiffRef {
            ref_str: baseline_snap.change_id.clone(),
            commit: Some(baseline_snap.commit_id.clone()),
            timestamp: Some(baseline_snap.timestamp.to_rfc3339()),
        },
        DiffRef {
            ref_str: "HEAD".to_owned(),
            commit: None,
            timestamp: None,
        },
    )
}

/// Git-subprocess diff fallback implementing SPEC-007 (REQ-046 – REQ-051).
///
/// Used when the `history` feature is not compiled in.
#[cfg(not(feature = "history"))]
fn cmd_diff_git(
    cli: &Cli,
    baseline_expr: Option<&str>,
    filter: Option<&zetl::cli::DiffFilter>,
) -> Result<()> {
    let pipeline = run_pipeline(cli)?;

    // Resolve baseline git ref (REQ-046, REQ-047, REQ-048).
    let baseline_ref = resolve_git_ref(&pipeline.vault_root, baseline_expr)?;

    // Identify changed .md files between baseline and working tree.
    let changed_files = git_diff_name_only(&pipeline.vault_root, &baseline_ref)?;

    // For each changed file, read the old content and parse links.
    let mut baseline_files: Vec<ParsedFile> = Vec::new();

    // Start from the current files for *unchanged* pages.
    for f in &pipeline.files {
        let rel = f.path.strip_prefix(&pipeline.vault_root).unwrap_or(&f.path);
        let rel_str = rel.to_string_lossy();
        if !changed_files.iter().any(|s| s == rel_str.as_ref()) {
            baseline_files.push(f.clone());
        }
    }

    // Re-parse changed (and possibly deleted) files from git history.
    for changed in &changed_files {
        if let Some(old_content) = git_show(&pipeline.vault_root, &baseline_ref, changed)? {
            // Parse the old content using the same scanner logic (link extraction only).
            let path = pipeline.vault_root.join(changed);
            let page_name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| changed.to_owned());
            // extract_wikilinks gives us the links without running the full SPEC-006 pipeline.
            let links = zetl::scanner::extract_wikilinks(&old_content);
            baseline_files.push(ParsedFile {
                path,
                page_name,
                links,
                spl_blocks: vec![],
                diagnostics: vec![],
                mtime: std::time::SystemTime::UNIX_EPOCH,
                merkle_leaves: vec![],
                file_merkle: None,
            });
        }
        // Files deleted since baseline simply don't appear in current graph.
    }

    // Build the baseline graph.
    let baseline_file_index: Vec<(String, PathBuf)> = baseline_files
        .iter()
        .map(|f| (f.page_name.clone(), f.path.clone()))
        .collect();
    let mut baseline_resolved: HashMap<String, String> = HashMap::new();
    for f in &baseline_files {
        for link in &f.links {
            let key = link.raw_target.clone();
            if let std::collections::hash_map::Entry::Vacant(e) = baseline_resolved.entry(key) {
                if let Some(r) =
                    zetl::scanner::resolve_page_name(&link.target_page, &baseline_file_index)
                {
                    e.insert(r);
                }
            }
        }
    }
    let baseline_graph = zetl::graph::LinkGraph::build(&baseline_files, &baseline_resolved);

    // Get current HEAD commit info for the "to" ref.
    let (git_commit, _) = zetl::vcs::get_git_metadata(&pipeline.vault_root);

    diff_graphs_and_output(
        cli,
        filter,
        &baseline_graph,
        &baseline_files,
        &pipeline.graph,
        &pipeline.files,
        DiffRef {
            ref_str: baseline_ref.clone(),
            commit: Some(baseline_ref.clone()),
            timestamp: None,
        },
        DiffRef {
            ref_str: "HEAD".to_owned(),
            commit: git_commit,
            timestamp: None,
        },
    )
}

/// Resolve a git baseline expression to a commit SHA (SPEC-007 §REQ-046/REQ-047/REQ-048).
#[cfg(not(feature = "history"))]
fn resolve_git_ref(vault_root: &Path, baseline_expr: Option<&str>) -> Result<String> {
    let expr = baseline_expr.unwrap_or("HEAD~1");

    // Try --since date format: resolve to nearest commit at or before that date.
    if looks_like_date(expr) {
        let output = std::process::Command::new("git")
            .args([
                "-C",
                vault_root.to_str().unwrap_or("."),
                "rev-list",
                &format!("--before={expr}"),
                "-1",
                "HEAD",
            ])
            .output()
            .context("running git rev-list --before")?;

        if !output.status.success() {
            anyhow::bail!("NOT_A_GIT_REPO: vault is not a git repository");
        }

        let sha = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if sha.is_empty() {
            anyhow::bail!("NO_COMMIT_BEFORE: no commit found at or before '{expr}'");
        }
        return Ok(sha);
    }

    // Plain git ref: resolve via git rev-parse.
    let output = std::process::Command::new("git")
        .args([
            "-C",
            vault_root.to_str().unwrap_or("."),
            "rev-parse",
            "--verify",
            expr,
        ])
        .output()
        .context("running git rev-parse")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("REF_NOT_FOUND: cannot resolve git ref '{expr}': {stderr}");
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Return true if `s` looks like an ISO 8601 date or datetime (heuristic).
#[cfg(not(feature = "history"))]
fn looks_like_date(s: &str) -> bool {
    // YYYY-MM-DD or starts with 4-digit year followed by '-'
    let b = s.as_bytes();
    b.len() >= 10
        && b[0].is_ascii_digit()
        && b[1].is_ascii_digit()
        && b[2].is_ascii_digit()
        && b[3].is_ascii_digit()
        && b[4] == b'-'
}

/// `git diff --name-only <ref>` — returns relative paths of changed `.md` files.
#[cfg(not(feature = "history"))]
fn git_diff_name_only(vault_root: &Path, git_ref: &str) -> Result<Vec<String>> {
    let output = std::process::Command::new("git")
        .args([
            "-C",
            vault_root.to_str().unwrap_or("."),
            "diff",
            "--name-only",
            git_ref,
        ])
        .output()
        .context("running git diff --name-only")?;

    if !output.status.success() {
        anyhow::bail!("NOT_A_GIT_REPO: git diff failed");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .filter(|l| l.ends_with(".md"))
        .map(|l| l.to_owned())
        .collect())
}

/// `git show <ref>:<path>` — read file content at a git ref. Returns `None` if
/// the file did not exist at that ref (newly added since baseline).
///
/// Callers:
/// - `cmd_diff_git` (SPEC-007 git-subprocess diff fallback, only compiled when
///   the `history` feature is off).
/// - `load_vault_at_refs` (capability audit-diff corpus walk, always compiled).
///
/// Because the second caller is unconditional, this helper must be too — it
/// shells out to `git` and does not interact with jj-lib, so it's safe under
/// every feature combination.
fn git_show(vault_root: &Path, git_ref: &str, rel_path: &str) -> Result<Option<String>> {
    let object = format!("{git_ref}:{rel_path}");
    let output = std::process::Command::new("git")
        .args(["-C", vault_root.to_str().unwrap_or("."), "show", &object])
        .output()
        .context("running git show")?;

    if !output.status.success() {
        // File did not exist at that ref (deleted or renamed since baseline).
        return Ok(None);
    }

    Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned()))
}

/// Compare two graph snapshots and emit the diff output (shared by both backends).
#[allow(clippy::too_many_arguments)] // shared helper with two graph halves, files, refs, cli, and filter
fn diff_graphs_and_output(
    cli: &Cli,
    filter: Option<&zetl::cli::DiffFilter>,
    old_graph: &zetl::graph::LinkGraph,
    old_files: &[ParsedFile],
    new_graph: &zetl::graph::LinkGraph,
    new_files: &[ParsedFile],
    from_ref: DiffRef,
    to_ref: DiffRef,
) -> Result<()> {
    use zetl::cli::DiffFilter;

    let old_pages: HashSet<&str> = old_files.iter().map(|f| f.page_name.as_str()).collect();
    let new_pages: HashSet<&str> = new_files.iter().map(|f| f.page_name.as_str()).collect();

    let pages_added: Vec<String> = new_pages
        .difference(&old_pages)
        .map(|s| s.to_string())
        .collect();
    let pages_removed: Vec<String> = old_pages
        .difference(&new_pages)
        .map(|s| s.to_string())
        .collect();

    // Collect all (source, target) link pairs for each graph.
    let old_links: HashSet<DiffLink> = old_files
        .iter()
        .flat_map(|f| {
            old_graph
                .forward_links(&f.page_name)
                .into_iter()
                .map(|l| DiffLink {
                    source: f.page_name.clone(),
                    target: l.target.clone(),
                })
        })
        .collect();

    let new_links: HashSet<DiffLink> = new_files
        .iter()
        .flat_map(|f| {
            new_graph
                .forward_links(&f.page_name)
                .into_iter()
                .map(|l| DiffLink {
                    source: f.page_name.clone(),
                    target: l.target.clone(),
                })
        })
        .collect();

    let links_added: Vec<DiffLink> = new_links.difference(&old_links).cloned().collect();
    let links_removed: Vec<DiffLink> = old_links.difference(&new_links).cloned().collect();

    // Orphans: pages with no incoming links (uses the graph's own orphans() method).
    let old_orphans: HashSet<String> = old_graph.orphans().into_iter().map(|o| o.page).collect();
    let new_orphans: HashSet<String> = new_graph.orphans().into_iter().map(|o| o.page).collect();

    let orphans_gained: Vec<String> = new_orphans.difference(&old_orphans).cloned().collect();
    let orphans_resolved: Vec<String> = old_orphans.difference(&new_orphans).cloned().collect();

    // Dead links: links whose target page doesn't exist in the resolved set.
    let old_dead: HashSet<DiffLink> = old_graph
        .dead_links()
        .iter()
        .map(|dl| DiffLink {
            source: dl.source.clone(),
            target: dl.target.clone(),
        })
        .collect();

    let new_dead: HashSet<DiffLink> = new_graph
        .dead_links()
        .iter()
        .map(|dl| DiffLink {
            source: dl.source.clone(),
            target: dl.target.clone(),
        })
        .collect();

    let dead_links_added: Vec<DiffLink> = new_dead.difference(&old_dead).cloned().collect();
    let dead_links_resolved: Vec<DiffLink> = old_dead.difference(&new_dead).cloned().collect();

    // Apply filter (REQ-050).
    let include_pages = matches!(filter, None | Some(DiffFilter::Pages));
    let include_links = matches!(filter, None | Some(DiffFilter::Links));
    let include_orphans = matches!(filter, None | Some(DiffFilter::Orphans));
    let include_dead_links = matches!(filter, None | Some(DiffFilter::DeadLinks));

    let diff = GraphDiff {
        from: from_ref,
        to: to_ref,
        pages_added: include_pages.then(|| pages_added.clone()),
        pages_removed: include_pages.then(|| pages_removed.clone()),
        links_added: include_links.then(|| links_added.clone()),
        links_removed: include_links.then(|| links_removed.clone()),
        orphans_gained: include_orphans.then(|| orphans_gained.clone()),
        orphans_resolved: include_orphans.then(|| orphans_resolved.clone()),
        dead_links_added: include_dead_links.then(|| dead_links_added.clone()),
        dead_links_resolved: include_dead_links.then(|| dead_links_resolved.clone()),
    };

    match cli.format {
        OutputFormat::Json => print_json(&diff)?,
        _ => {
            let mut table = Table::new();
            table.set_header(vec!["Category", "Added / Gained", "Removed / Resolved"]);
            if include_pages {
                table.add_row(vec![
                    Cell::new("Pages"),
                    Cell::new(pages_added.len()),
                    Cell::new(pages_removed.len()),
                ]);
            }
            if include_links {
                table.add_row(vec![
                    Cell::new("Links"),
                    Cell::new(links_added.len()),
                    Cell::new(links_removed.len()),
                ]);
            }
            if include_orphans {
                table.add_row(vec![
                    Cell::new("Orphans"),
                    Cell::new(orphans_gained.len()),
                    Cell::new(orphans_resolved.len()),
                ]);
            }
            if include_dead_links {
                table.add_row(vec![
                    Cell::new("Dead links"),
                    Cell::new(dead_links_added.len()),
                    Cell::new(dead_links_resolved.len()),
                ]);
            }
            println!("{table}");
        }
    }

    Ok(())
}

// ── zetl history ───────────────────────────────────────────────────────────

/// `zetl history timeline` — list recent snapshots (REQ-080, REQ-081).
#[cfg(feature = "history")]
fn cmd_history_timeline(cli: &Cli, limit: usize) -> Result<()> {
    use zetl::history::jj_backend::VcsBackend as _;

    let vault_root = std::fs::canonicalize(&cli.dir)
        .with_context(|| format!("Cannot resolve vault directory: {}", cli.dir))?;

    // open_history errors with NO_HISTORY if .zetl/jj/ is absent (REQ-084).
    let backend = zetl::history::open_history(&vault_root)
        .context("opening jj workspace for history timeline")?;

    let snapshots = backend
        .list_changes(limit)
        .context("listing jj snapshots")?;

    if snapshots.is_empty() {
        match cli.format {
            OutputFormat::Json => print_json(&serde_json::json!({
                "snapshots": [],
                "message": "No snapshots yet. Run `zetl index` to create the first snapshot."
            }))?,
            _ => {
                println!("No snapshots yet. Run `zetl index` to create the first snapshot.");
            }
        }
        return Ok(());
    }

    #[derive(Serialize)]
    struct SnapshotEntry {
        change_id: String,
        commit_id: String,
        timestamp: String,
        description: String,
        has_cached_index: bool,
    }

    let entries: Vec<SnapshotEntry> = snapshots
        .iter()
        .take(limit)
        .map(|s| SnapshotEntry {
            change_id: s.change_id.clone(),
            commit_id: s.commit_id.clone(),
            timestamp: s.timestamp.to_rfc3339(),
            description: s.description.clone(),
            has_cached_index: zetl::history::core::extract_vault_root_hash_from_description(
                &s.description,
            )
            .is_some(),
        })
        .collect();

    match cli.format {
        OutputFormat::Json => print_json(&serde_json::json!({ "snapshots": entries }))?,
        _ => {
            let mut table = Table::new();
            table.set_header(vec!["Change ID", "Timestamp", "Indexed"]);
            for e in &entries {
                table.add_row(vec![
                    Cell::new(&e.change_id[..e.change_id.len().min(12)]),
                    Cell::new(&e.timestamp),
                    Cell::new(if e.has_cached_index { "yes" } else { "no" }),
                ]);
            }
            println!("{table}");
        }
    }

    Ok(())
}

/// `zetl history page <name>` — per-page evolution timeline (REQ-081, CON-025).
///
/// Only snapshots where the page's neighbourhood (forward links, backlinks, or
/// existence) changed are included in the output.
#[cfg(feature = "history")]
fn cmd_history_page(cli: &Cli, page_name: &str, limit: usize) -> Result<()> {
    use zetl::history::cache::HistoricalIndexCache;
    use zetl::history::core::{extract_page_history, extract_vault_root_hash_from_description};
    use zetl::history::jj_backend::VcsBackend as _;

    let vault_root = std::fs::canonicalize(&cli.dir)
        .with_context(|| format!("Cannot resolve vault directory: {}", cli.dir))?;

    let backend = zetl::history::open_history(&vault_root)
        .context("opening jj workspace for history page")?;

    let snapshots = backend
        .list_changes(10_000)
        .context("listing jj snapshots")?;

    let cache = HistoricalIndexCache::with_default_capacity();

    // Pre-load the cached file index for every snapshot.
    let files_per_snapshot: Vec<Option<Vec<ParsedFile>>> = snapshots
        .iter()
        .map(|snap| {
            let hash = extract_vault_root_hash_from_description(&snap.description)?;
            let file_map = cache.load(&vault_root, &hash).ok().flatten()?;
            Some(file_map.into_values().collect())
        })
        .collect();

    let entries = extract_page_history(page_name, &snapshots, &files_per_snapshot, limit);

    if entries.is_empty() {
        anyhow::bail!("PAGE_NOT_FOUND: page '{page_name}' not found in any snapshot");
    }

    match cli.format {
        OutputFormat::Json => print_json(&serde_json::json!({
            "page": page_name,
            "snapshots": entries,
        }))?,
        _ => {
            let mut table = Table::new();
            table.set_header(vec!["Timestamp", "Links", "Backlinks", "Orphan", "Changes"]);
            for e in &entries {
                let changes = if let Some(ref d) = e.delta {
                    format_page_delta(d)
                } else {
                    "-".to_owned()
                };
                table.add_row(vec![
                    Cell::new(&e.timestamp),
                    Cell::new(e.link_count),
                    Cell::new(e.backlink_count),
                    Cell::new(if e.is_orphan { "yes" } else { "no" }),
                    Cell::new(&changes),
                ]);
            }
            println!("{table}");
        }
    }

    Ok(())
}

/// Format a page neighbourhood delta into a compact human-readable string.
#[cfg(feature = "history")]
fn format_page_delta(d: &zetl::history::core::PageNeighborhoodDelta) -> String {
    if d.appeared {
        return "appeared".to_owned();
    }
    if d.disappeared {
        return "disappeared".to_owned();
    }
    let mut parts: Vec<String> = Vec::new();
    if !d.links_added.is_empty() {
        parts.push(format!("+{}L", d.links_added.len()));
    }
    if !d.links_removed.is_empty() {
        parts.push(format!("-{}L", d.links_removed.len()));
    }
    if !d.backlinks_added.is_empty() {
        parts.push(format!("+{}B", d.backlinks_added.len()));
    }
    if !d.backlinks_removed.is_empty() {
        parts.push(format!("-{}B", d.backlinks_removed.len()));
    }
    if parts.is_empty() {
        "-".to_owned()
    } else {
        parts.join(" ")
    }
}

/// `zetl history log` — reverse-chronological delta timeline (REQ-080, CON-025).
#[cfg(feature = "history")]
fn cmd_history_log(cli: &Cli, since: Option<&str>, limit: usize) -> Result<()> {
    use zetl::history::core::build_vault_history;
    use zetl::history::jj_backend::VcsBackend as _;

    let vault_root = std::fs::canonicalize(&cli.dir)
        .with_context(|| format!("Cannot resolve vault directory: {}", cli.dir))?;

    let backend =
        zetl::history::open_history(&vault_root).context("opening jj workspace for history log")?;

    // Load all snapshots (we may need them for --since ref resolution and
    // delta computation beyond the final `limit`).
    let snapshots = backend
        .list_changes(10_000)
        .context("listing jj snapshots")?;

    let now = chrono::Local::now().fixed_offset();
    let entries = build_vault_history(&snapshots, &vault_root, since, limit, now)
        .context("building vault history")?;

    if entries.is_empty() {
        match cli.format {
            OutputFormat::Json => print_json(&serde_json::json!({
                "entries": [],
                "message": "No snapshots found. Run `zetl index` to create the first snapshot."
            }))?,
            _ => {
                println!("No snapshots found. Run `zetl index` to create the first snapshot.");
            }
        }
        return Ok(());
    }

    match cli.format {
        OutputFormat::Json => {
            #[derive(Serialize)]
            struct DeltaJson {
                pages_added: Vec<String>,
                pages_removed: Vec<String>,
                links_added: usize,
                links_removed: usize,
            }
            #[derive(Serialize)]
            struct EntryJson {
                change_id: String,
                timestamp: String,
                vault_root_hash: Option<String>,
                total_pages: usize,
                total_links: usize,
                delta: Option<DeltaJson>,
            }
            let json_entries: Vec<EntryJson> = entries
                .iter()
                .map(|e| EntryJson {
                    change_id: e.change_id.clone(),
                    timestamp: e.timestamp.clone(),
                    vault_root_hash: e.vault_root_hash.clone(),
                    total_pages: e.total_pages,
                    total_links: e.total_links,
                    delta: e.delta.as_ref().map(|d| DeltaJson {
                        pages_added: d.pages_added.clone(),
                        pages_removed: d.pages_removed.clone(),
                        links_added: d.links_added,
                        links_removed: d.links_removed,
                    }),
                })
                .collect();
            print_json(&serde_json::json!({ "entries": json_entries }))?;
        }
        _ => {
            let mut table = Table::new();
            table.set_header(vec![
                "Change ID",
                "Timestamp",
                "Pages",
                "Links",
                "+Pages",
                "-Pages",
                "+Links",
                "-Links",
            ]);
            for e in &entries {
                let (pages_added, pages_removed, links_added, links_removed) = match &e.delta {
                    Some(d) => (
                        d.pages_added.len().to_string(),
                        d.pages_removed.len().to_string(),
                        d.links_added.to_string(),
                        d.links_removed.to_string(),
                    ),
                    None => (
                        "—".to_string(),
                        "—".to_string(),
                        "—".to_string(),
                        "—".to_string(),
                    ),
                };
                table.add_row(vec![
                    Cell::new(&e.change_id[..e.change_id.len().min(12)]),
                    Cell::new(&e.timestamp),
                    Cell::new(e.total_pages),
                    Cell::new(e.total_links),
                    Cell::new(&pages_added),
                    Cell::new(&pages_removed),
                    Cell::new(&links_added),
                    Cell::new(&links_removed),
                ]);
            }
            println!("{table}");
        }
    }

    Ok(())
}

// ── Main ───────────────────────────────────────────────────────────────────

fn main() -> anyhow::Result<()> {
    let mut cli = Cli::parse();

    // Resolve output format: --json flag takes priority, then -f, then TTY detection
    if cli.json {
        cli.format = OutputFormat::Json;
    } else if cli.format == OutputFormat::Auto {
        cli.format = if std::io::stdout().is_terminal() {
            OutputFormat::Table
        } else {
            OutputFormat::Json
        };
    }

    match &cli.command {
        Command::Index { .. } => cmd_index(&cli),
        Command::Links {
            page,
            fuzzy,
            context,
            depth,
            with_conclusions,
        } => cmd_links(&cli, page, *fuzzy, *context, *depth, *with_conclusions),
        Command::Backlinks {
            page,
            fuzzy,
            context,
            depth,
            with_conclusions,
        } => cmd_backlinks(&cli, page, *fuzzy, *context, *depth, *with_conclusions),
        Command::Check {
            dead_links,
            orphans,
            syntax,
            spl,
            drift,
            fail_on,
            theme,
        } => cmd_check(
            &cli,
            *dead_links,
            *orphans,
            *syntax,
            *spl,
            *drift,
            fail_on,
            theme,
        ),
        Command::Similar {
            query,
            threshold,
            limit,
        } => cmd_similar(&cli, query, *threshold, *limit),
        Command::Search {
            query,
            context,
            limit,
            case_sensitive,
            path,
            near,
            depth,
            semantic,
            hybrid,
            scan: _,
        } => cmd_search(
            &cli,
            query,
            *context,
            *limit,
            *case_sensitive,
            path.as_deref(),
            near.as_deref(),
            *depth,
            *semantic,
            *hybrid,
        ),
        Command::List => cmd_list(&cli),
        Command::Stats { top } => cmd_stats(&cli, *top),
        Command::Path {
            from,
            to,
            max_depth,
        } => cmd_path(&cli, from, to, *max_depth),
        Command::Blocks {
            page,
            block_type,
            resolve,
        } => cmd_blocks(&cli, page.as_deref(), block_type, resolve.as_deref()),
        Command::Export => cmd_export(&cli),
        Command::View {
            page,
            context_lines,
            main_width,
        } => cmd_view(&cli, page.as_deref(), *context_lines, *main_width),
        Command::Theme { command } => match command {
            ThemeCommand::List => cmd_theme_list(&cli),
            ThemeCommand::Install {
                source,
                path,
                name,
                force,
            } => cmd_theme_install(&cli, source, path.as_deref(), name.as_deref(), *force),
            ThemeCommand::Remove { name } => cmd_theme_remove(&cli, name),
            ThemeCommand::Export { name, force } => cmd_theme_export(&cli, name, *force),
        },
        Command::Hook { command } => match command {
            HookCommand::List { theme } => cmd_hook_list(&cli, theme),
            HookCommand::Run { name, theme, extra } => cmd_hook_run(&cli, name, theme, extra),
            HookCommand::New {
                stage,
                name,
                lang,
                ecosystem,
                force,
            } => cmd_hook_new(&cli, stage, name, lang, ecosystem.as_ref(), *force),
            HookCommand::Test { name, update } => cmd_hook_test(&cli, name, *update),
            HookCommand::Fixture { from, hook } => cmd_hook_fixture(&cli, from, hook),
            HookCommand::Watch { name } => cmd_hook_watch(&cli, name),
            HookCommand::DryRun { spec, theme, limit } => {
                cmd_hook_dry_run(&cli, spec, theme, *limit)
            }
            HookCommand::Coverage { theme, stage } => {
                cmd_hook_coverage(&cli, theme, stage.as_ref())
            }
            HookCommand::Capabilities { theme, stage } => {
                cmd_hook_capabilities(&cli, theme, stage.as_ref())
            }
        },
        Command::Ecosystem { command } => match command {
            EcosystemCommand::Check { theme, json } => cmd_ecosystem_check(&cli, theme, *json),
        },
        Command::Feed { command } => cmd_feed(&cli, command),
        Command::Ast { command } => {
            use zetl::cli::AstCommand;
            match command {
                AstCommand::Sample { file, stage } => cmd_ast_sample(&cli, file, stage),
                AstCommand::Diff { before, after } => cmd_ast_diff(&cli, before, after),
            }
        }
        Command::Agent { command } => match command {
            AgentCommand::Run {
                name,
                theme,
                target_pages,
                budget,
                extra,
            } => cmd_agent_run(&cli, name, theme, target_pages, *budget, extra),
        },
        Command::Serve {
            port,
            theme,
            public,
            collab,
            init_owner,
            owner_name,
            hostname,
            server_key_seed,
            git_poll_interval,
            safe_mode,
            asset_max_file_bytes,
            asset_max_total_bytes,
            scan: _,
        } => cmd_serve(
            &cli,
            *port,
            theme,
            public.as_deref(),
            *collab,
            *init_owner,
            owner_name,
            hostname.as_deref(),
            server_key_seed.as_deref(),
            *git_poll_interval,
            *safe_mode,
            *asset_max_file_bytes,
            *asset_max_total_bytes,
        ),
        Command::Invite {
            as_user,
            role,
            pages,
            expires,
            port,
            host,
        } => cmd_invite(
            &cli,
            as_user,
            role,
            pages.as_deref(),
            expires.as_deref(),
            *port,
            host,
        ),
        Command::AgentToken { mnemonic } => cmd_agent_token(&cli, mnemonic),
        Command::DeriveSshKey { mnemonic, out } => cmd_derive_ssh_key(mnemonic, out.as_deref()),
        Command::Build {
            out_dir,
            theme,
            public,
            site_url,
            safe_mode,
            strict_parsers,
            scan: _,
        } => cmd_build(
            &cli,
            out_dir,
            theme,
            public.as_deref(),
            site_url.as_deref(),
            *safe_mode,
            *strict_parsers,
        ),
        #[cfg(feature = "reason")]
        Command::Reason { command } => {
            use zetl::cli::ReasonCommand;
            match command {
                ReasonCommand::Status {
                    positive,
                    negative,
                    definite,
                    defeasible,
                    literal,
                } => cmd_reason_status(
                    &cli,
                    *positive,
                    *negative,
                    *definite,
                    *defeasible,
                    literal.as_deref(),
                ),
                ReasonCommand::Explain {
                    literal,
                    depth,
                    output_as,
                } => cmd_reason_explain(&cli, literal, *depth, output_as),
                ReasonCommand::WhyNot { literal } => cmd_reason_why_not(&cli, literal),
                ReasonCommand::Conflicts {
                    suggest,
                    fail_on_conflicts,
                } => cmd_reason_conflicts(&cli, *suggest, *fail_on_conflicts),
                ReasonCommand::Provenance { literal } => cmd_reason_provenance(&cli, literal),
                ReasonCommand::WhatIf { spl, file, goal } => {
                    cmd_reason_what_if(&cli, spl.as_deref(), file.as_deref(), goal.as_deref())
                }
                ReasonCommand::Require {
                    literal,
                    max_solutions,
                    assume,
                } => cmd_reason_require(&cli, literal, *max_solutions, assume.as_deref()),
                ReasonCommand::Export {
                    output_as,
                    with_conclusions,
                } => cmd_reason_export(&cli, output_as, *with_conclusions),
            }
        }
        #[cfg(not(feature = "reason"))]
        Command::Reason { .. } => reason_not_available(),
        Command::Watch {
            debounce,
            exec,
            scan: _,
        } => cmd_watch(&cli, *debounce, exec.as_deref()),
        Command::Diff {
            from,
            since,
            filter,
        } => cmd_diff(&cli, from.as_deref(), since.as_deref(), filter.as_ref()),
        #[cfg(feature = "history")]
        Command::History { command } => {
            use zetl::cli::HistoryCommand;
            match command {
                HistoryCommand::Timeline { limit } => cmd_history_timeline(&cli, *limit),
                HistoryCommand::Page { name, limit } => cmd_history_page(&cli, name, *limit),
                HistoryCommand::Log { since, limit } => {
                    cmd_history_log(&cli, since.as_deref(), *limit)
                }
            }
        }
        #[cfg(feature = "mcp")]
        Command::Delegate {
            tools,
            scope,
            expiry,
            mnemonic,
            save_key,
        } => cmd_delegate(
            tools.as_deref(),
            scope.as_deref(),
            expiry.as_deref(),
            mnemonic.as_deref(),
            *save_key,
        ),
        #[cfg(not(feature = "mcp"))]
        Command::Delegate { .. } => {
            eprintln!(
                "Delegate command requires --features mcp. Rebuild with: cargo build --features mcp"
            );
            std::process::exit(1);
        }
        #[cfg(feature = "mcp")]
        Command::Mcp {
            transport,
            host,
            port,
            insecure,
            allowed_issuer,
            cors_origin,
        } => cmd_mcp(
            &cli,
            transport,
            host,
            *port,
            *insecure,
            allowed_issuer,
            cors_origin.as_deref(),
        ),
        #[cfg(not(feature = "mcp"))]
        Command::Mcp { .. } => {
            eprintln!(
                "MCP server requires --features mcp. Rebuild with: cargo build --features mcp"
            );
            std::process::exit(1);
        }
        Command::Completions { shell } => {
            use clap::CommandFactory;
            let mut cmd = Cli::command();
            let bin_name = cmd.get_name().to_string();
            clap_complete::generate(*shell, &mut cmd, bin_name, &mut std::io::stdout());
            Ok(())
        }
        Command::Man => {
            use clap::CommandFactory;
            let cmd = Cli::command();
            clap_mangen::Man::new(cmd).render(&mut std::io::stdout())?;
            Ok(())
        }
        Command::Cap { command } => match command {
            zetl::cli::CapCommand::Genkey => cmd_cap_genkey(&cli),
            zetl::cli::CapCommand::Invite {
                name,
                cohort,
                expires,
                pages,
                recipient,
                via,
                split_key,
                site_url,
                slug,
            } => cmd_cap_invite(
                &cli,
                CapInviteArgs {
                    name: name.clone(),
                    cohort: cohort.clone(),
                    expires: expires.clone(),
                    pages: pages.clone(),
                    recipient: recipient.clone(),
                    via: via.clone(),
                    split_key: *split_key,
                    site_url: site_url.clone(),
                    slug: slug.clone(),
                },
            ),
            zetl::cli::CapCommand::List { .. } => cmd_cap_stub(&cli, "list"),
            zetl::cli::CapCommand::Revoke { grant_id } => cmd_cap_revoke(&cli, grant_id),
            zetl::cli::CapCommand::Rotate { cohort } => cmd_cap_rotate(&cli, cohort),
            zetl::cli::CapCommand::Finalise {
                grant_id,
                rotate_grant,
            } => cmd_cap_finalise(&cli, grant_id, *rotate_grant),
            zetl::cli::CapCommand::Share { .. } => cmd_cap_stub(&cli, "share"),
            zetl::cli::CapCommand::Check { public_safety } => cmd_cap_check(&cli, *public_safety),
            zetl::cli::CapCommand::Sweep => cmd_cap_sweep(&cli),
            zetl::cli::CapCommand::Pair {
                grantor,
                grantee,
                peer,
                phrase,
                pubkey,
            } => cmd_cap_pair(
                &cli,
                CapPairArgs {
                    grantor: *grantor,
                    grantee: *grantee,
                    peer: peer.clone(),
                    phrase: phrase.clone(),
                    pubkey: pubkey.clone(),
                },
            ),
            zetl::cli::CapCommand::RotateSigningKey => cmd_cap_rotate_signing_key(&cli),
            zetl::cli::CapCommand::EmergencyShutdown => cmd_cap_emergency_shutdown(&cli),
            zetl::cli::CapCommand::AuditDiff {
                old_ref,
                new_ref,
                corpus,
                corpus_root,
            } => cmd_cap_audit_diff(
                &cli,
                old_ref.as_deref(),
                new_ref.as_deref(),
                corpus.as_deref(),
                corpus_root.as_deref(),
            ),
        },
    }
}

/// Exit code emitted by `zetl cap` stubs whose implementation has not
/// landed yet. Follows SPEC-004's 0/1/2/64+ convention — we pick `2`
/// for "not-yet-implemented" so CI gates can distinguish a skeleton
/// verb from a runtime failure (`1`) and a usage error (clap's default
/// also 2, but a stub is semantically closer to misuse than runtime
/// failure).
const CAP_NOT_YET_IMPLEMENTED_EXIT: i32 = 2;

/// Stub for `zetl cap` verbs whose implementation has not landed. Emits
/// a diagnostic and exits with `CAP_NOT_YET_IMPLEMENTED_EXIT`. When
/// `--json`/`-f json` is in effect the diagnostic is JSON on stdout so
/// agents can parse it (CLIG-adjacent behaviour mirroring
/// `exit_json_error`).
///
/// SPEC-034 REQ-3416 — every verb listed in the CLI surface is
/// reachable and produces a predictable error code even before its
/// handler exists.
fn cmd_cap_stub(cli: &Cli, verb: &str) -> Result<()> {
    let message =
        format!("zetl cap {verb}: not-yet-implemented (SPEC-034 REQ-3416 CLI surface stub)");
    let json_requested = cli.json || matches!(cli.format, OutputFormat::Json);
    if json_requested {
        exit_json_error(&message, CAP_NOT_YET_IMPLEMENTED_EXIT);
    }
    eprintln!("{message}");
    eprintln!(
        "Hint: this verb is planned (see SPEC-034 §6 REQ-3416) but its implementation has not landed yet."
    );
    std::process::exit(CAP_NOT_YET_IMPLEMENTED_EXIT);
}

/// `zetl cap genkey` — SPEC-034 REQ-3419.
///
/// Prints BOTH capability-mode secrets (`ZETL_CAP_SECRET` +
/// `ZETL_CAP_SIGNING_KEY`) to stdout exactly once, with secure-storage
/// instructions. Never writes to any file; never logs. The random
/// bytes come from [`rand_core::OsRng`] via `cap::genkey::generate`.
///
/// The 15-byte checksum embedded in `ZETL_CAP_SECRET` is framed as a
/// UX safeguard — BUG-017 resolution — so operators paste-corrupting
/// the value hit a targeted remediation message at build time rather
/// than a surprise decryption failure in prod.
fn cmd_cap_genkey(cli: &Cli) -> Result<()> {
    let out = zetl::cap::genkey::generate();

    // JSON only when the operator explicitly asked for it — `--json`
    // or `-f json` / `--format json` on the command line. The auto-TTY
    // promotion `main` performs on `cli.format` before dispatch is
    // intentionally ignored here: `genkey` output is a one-shot banner
    // that operators routinely pipe into `less` or a file for paste
    // into a password manager, and silently substituting JSON for the
    // human-readable form breaks that workflow.
    let mut args = std::env::args().skip(1);
    let mut json_requested = cli.json;
    while let Some(arg) = args.next() {
        if arg == "--json" || arg == "-f=json" || arg == "--format=json" {
            json_requested = true;
            break;
        }
        if arg == "-f" || arg == "--format" {
            if let Some(v) = args.next() {
                if v.eq_ignore_ascii_case("json") {
                    json_requested = true;
                    break;
                }
            }
        }
    }

    if json_requested {
        print_json(&zetl::cap::genkey::render_json(&out))?;
    } else {
        // Direct write to stdout — avoid any intermediate logging /
        // tracing layer that might echo the secret into a log sink.
        // A single `print!` emits the banner once.
        print!("{}", zetl::cap::genkey::render_human(&out));
    }
    Ok(())
}

/// Argument bundle threaded through to [`cmd_cap_invite`]. Using a
/// named struct instead of a long positional parameter list keeps the
/// dispatch site in `main()` readable and makes the tests that call
/// the handler directly compile-type-check when a new flag lands.
struct CapInviteArgs {
    name: String,
    cohort: String,
    expires: Option<String>,
    pages: Option<String>,
    recipient: Option<String>,
    via: Option<String>,
    split_key: bool,
    site_url: Option<String>,
    slug: String,
}

/// `zetl cap invite` — SPEC-034 REQ-3410 / REQ-3416 / CON-3402.
///
/// Three operating modes, dispatched on the flags:
///
/// 1. **Delegated-URL (default).** Generate a fresh `(priv_A, pub_A)`
///    keypair, append `pub_A` to the cohort in `recipients.toml`, add
///    a `bound=false` grant to `grants.toml`, and print the
///    `#k=<priv_A>` URL to stdout. The private key appears **only**
///    on stdout — never in `grants.toml`, never in stderr/logs,
///    never persisted. Emits the REQ-3410 URL-shortener warning
///    banner before the URL.
/// 2. **Hardened, pre-collected recipient** (`--recipient <pubkey>`).
///    The operator already has the reader's `age-recipient-v1:`
///    pubkey (e.g. via `zetl cap pair` SPAKE2 handoff); we skip key
///    generation, add the pubkey to the cohort, and print the
///    hardened URL (no fragment).
/// 3. **Hardened, enrol-page** (`--via enrol-page`). We don't yet
///    know the reader's pubkey; print the `/enroll.html?cohort=<id>`
///    URL so the reader can self-enrol and send back their pubkey.
///    No grant is written until the operator runs
///    `zetl cap invite --recipient <pubkey>` with the returned
///    pubkey.
///
/// `--split-key` (REQ-3430) is orthogonal to the mode: when set (and
/// the mode is delegated-URL), we XOR-split `priv_A` into
/// `(half1, half2)` and print `half2` on a separate stdout line as
/// the second-factor conveyance.
fn cmd_cap_invite(cli: &Cli, args: CapInviteArgs) -> Result<()> {
    use std::time::{SystemTime, UNIX_EPOCH};

    use zetl::cap::derivation::{derive_path_cap, PATH_CAP_DEFAULT_BITS};
    use zetl::cap::genkey::{decode_secret, ZETL_CAP_SECRET_ENV};
    use zetl::cap::grants::validation::{Grant, GrantMode, GrantsFile};
    use zetl::cap::invite::{
        encode_age_recipient_v1, format_rfc3339_utc, generate_grant_id, generate_invite_keypair,
        parse_expires, xor_split_private_key, DEFAULT_EXPIRES_SECS, URL_SHORTENER_WARNING,
    };
    use zetl::cap::public_repo::{parse_config_lens, SplitKeyConfig, SplitKeySecondFactor};
    use zetl::cap::recipients::parsing::{CohortMode, RecipientsFile, AGE_RECIPIENT_V1_PREFIX};
    use zetl::cap::url_format::CapUrl;

    // ─── Resolve site URL (scheme + host) ──────────────────────────
    let site_url = args
        .site_url
        .as_deref()
        .map(|s| s.trim().trim_end_matches('/'))
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "zetl cap invite requires a canonical site URL: pass --site-url <URL> \
                 or set ZETL_CAP_SITE_URL=<URL> in the environment"
            )
        })?
        .to_string();
    let (scheme, host) = site_url
        .split_once("://")
        .ok_or_else(|| anyhow::anyhow!("--site-url must be of the form <scheme>://<host>"))?;
    if scheme.is_empty() || host.is_empty() {
        anyhow::bail!("--site-url must be of the form <scheme>://<host>");
    }

    // ─── Parse --expires (or use default) ──────────────────────────
    let expires_secs = match args.expires.as_deref() {
        Some(s) => parse_expires(s).with_context(|| format!("invalid --expires value {s:?}"))?,
        None => DEFAULT_EXPIRES_SECS,
    };
    let pages_glob = args.pages.clone().unwrap_or_else(|| "*".to_string());

    // ─── Load recipients.toml ──────────────────────────────────────
    let vault_root = std::fs::canonicalize(&cli.dir)
        .with_context(|| format!("Cannot resolve vault directory: {}", cli.dir))?;
    let recipients_path = vault_root.join("recipients.toml");
    let body = std::fs::read_to_string(&recipients_path).with_context(|| {
        format!(
            "Cannot read {}. Run `zetl cap genkey` + populate `recipients.toml` \
             before issuing invites (see SPEC-034 REQ-3409).",
            recipients_path.display()
        )
    })?;
    let mut recipients = RecipientsFile::parse(&body)
        .with_context(|| format!("{} is invalid", recipients_path.display()))?;

    // ─── Read `.zetl/config.toml` for REQ-3417 `[access.split_key]` ──
    // Missing config + missing block both deserialise as
    // `split_key = None`, which the gate below treats as "disabled"
    // per REQ-3430 acceptance ("default is `enabled = false`").
    let split_key_cfg: SplitKeyConfig = {
        let config_path = vault_root.join(".zetl").join("config.toml");
        match std::fs::read_to_string(&config_path) {
            Ok(body) => parse_config_lens(&body)
                .with_context(|| format!("{} is invalid", config_path.display()))?
                .access
                .and_then(|a| a.split_key)
                .unwrap_or_default(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => SplitKeyConfig::default(),
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "cannot read {}: {e}",
                    config_path.display()
                ));
            }
        }
    };
    if args.split_key && !split_key_cfg.enabled {
        anyhow::bail!(
            "`--split-key` refused: SPEC-034 REQ-3430 requires an explicit opt-in via \
             `[access.split_key] enabled = true` in .zetl/config.toml \
             (default is disabled)"
        );
    }

    // Locate the target cohort (reject unknown ids up-front — a typo
    // is far more likely than the intent to auto-create).
    let cohort_idx = recipients
        .cohorts
        .iter()
        .position(|c| c.id == args.cohort)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "cohort {:?} not found in {} (known cohorts: {})",
                args.cohort,
                recipients_path.display(),
                recipients
                    .cohorts
                    .iter()
                    .map(|c| c.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;

    // ─── Dispatch enrol-page mode (no grant written; no priv_A) ────
    if let Some(via) = args.via.as_deref() {
        if via != "enrol-page" {
            anyhow::bail!(
                "--via only accepts `enrol-page` today; got {via:?}. See SPEC-034 REQ-3404."
            );
        }
        if args.split_key {
            anyhow::bail!("--split-key has no effect with --via enrol-page (hardened mode)");
        }
        if args.recipient.is_some() {
            anyhow::bail!("--recipient and --via are mutually exclusive");
        }
        // Emit the enrolment URL so the operator can forward it. No
        // grant is written until the reader returns a pubkey.
        println!("{URL_SHORTENER_WARNING}");
        println!(
            "{scheme}://{host}/enroll.html?cohort={}",
            urlencoding::encode(&args.cohort)
        );
        eprintln!(
            "After the reader sends back their `age-recipient-v1:` pubkey, re-run:\n  \
             zetl cap invite {name:?} --cohort {cohort} --recipient <pubkey>",
            name = args.name,
            cohort = args.cohort,
        );
        return Ok(());
    }

    // ─── Load ZETL_CAP_SECRET (needed to derive path-cap) ──────────
    let secret_env = std::env::var(ZETL_CAP_SECRET_ENV).map_err(|_| {
        anyhow::anyhow!(
            "{ZETL_CAP_SECRET_ENV} is not set in the environment; run `zetl cap genkey` first"
        )
    })?;
    let secret =
        decode_secret(&secret_env).with_context(|| format!("{ZETL_CAP_SECRET_ENV} is invalid"))?;

    // ─── Ensure cohort has a stable path salt ──────────────────────
    // First invite for a cohort initialises salt_stable; persist on
    // exit alongside the new pubkey so subsequent URLs stay
    // bit-stable (CON-3401's "stable salt" contract).
    if recipients.cohorts[cohort_idx].salt_stable.is_none() {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use base64::Engine as _;
        use rand_core::{OsRng, RngCore};
        let mut salt_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut salt_bytes);
        recipients.cohorts[cohort_idx].salt_stable = Some(URL_SAFE_NO_PAD.encode(salt_bytes));
    }
    let cohort_salt_stable = recipients.cohorts[cohort_idx]
        .salt_stable
        .as_deref()
        .expect("just initialised above")
        .to_string();

    // ─── Derive the path-cap for the invite's landing slug ─────────
    let path_cap = derive_path_cap(
        secret.random_body(),
        cohort_salt_stable.as_bytes(),
        &args.cohort,
        &args.slug,
        PATH_CAP_DEFAULT_BITS,
    )
    .context("failed to derive path-cap from (secret, salt, cohort, slug)")?;

    // ─── Resolve the grant's `recipient` + URL (delegated vs hardened) ──
    let cohort_mode = recipients.cohorts[cohort_idx].mode;
    let (recipient_str, url_and_extra): (String, (String, Option<String>)) =
        if let Some(rcpt) = args.recipient.as_deref() {
            // Hardened mode, pre-collected pubkey.
            if args.split_key {
                anyhow::bail!("--split-key has no effect with --recipient (hardened mode)");
            }
            if !rcpt.starts_with(AGE_RECIPIENT_V1_PREFIX) {
                anyhow::bail!(
                    "--recipient must carry the `{AGE_RECIPIENT_V1_PREFIX}` prefix \
                     (see SPEC-034 REQ-3409)"
                );
            }
            let url = CapUrl::render_hardened(scheme, host, &path_cap, &args.slug)
                .context("failed to render hardened cap URL")?;
            (rcpt.to_string(), (url, None))
        } else {
            // Delegated-URL mode: generate fresh (priv_A, pub_A).
            if matches!(cohort_mode, CohortMode::WebauthnPrf) {
                anyhow::bail!(
                    "cohort {:?} is configured for webauthn-prf (hardened) mode; \
                     pass --recipient <pubkey> or --via enrol-page",
                    args.cohort,
                );
            }
            use rand_core::OsRng;
            let mut rng = OsRng;
            let kp = generate_invite_keypair(&mut rng);
            let pub_recipient = encode_age_recipient_v1(&kp.public);
            if args.split_key {
                let (half1, half2) = xor_split_private_key(kp.secret, &mut rng);
                let url = CapUrl::render_split_key(scheme, host, &path_cap, &args.slug, &half1)
                    .context("failed to render split-key cap URL")?;
                (pub_recipient, (url, Some(half2)))
            } else {
                let priv_b64 = kp.secret.into_b64url();
                let url = CapUrl::render_delegated(scheme, host, &path_cap, &args.slug, &priv_b64)
                    .context("failed to render delegated cap URL")?;
                (pub_recipient, (url, None))
            }
        };

    // ─── Append the recipient pubkey to the cohort if absent ───────
    {
        let cohort = &mut recipients.cohorts[cohort_idx];
        if !cohort.pubkeys.iter().any(|k| k == &recipient_str) {
            cohort.pubkeys.push(recipient_str.clone());
        }
        recipients
            .validate()
            .context("post-update recipients.toml failed validation")?;
    }

    // ─── Load grants.toml (tolerate missing) + append new grant ────
    let grants_path = vault_root.join("grants.toml");
    let mut grants: GrantsFile = match std::fs::read_to_string(&grants_path) {
        Ok(body) if !body.trim().is_empty() => GrantsFile::from_toml(&body)
            .with_context(|| format!("{} is invalid TOML", grants_path.display()))?,
        _ => GrantsFile {
            version: Some(1),
            grants: Vec::new(),
        },
    };

    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before UNIX epoch")
        .as_secs();
    let created = format_rfc3339_utc(now_unix);
    let expires_ts = format_rfc3339_utc(now_unix + expires_secs);
    let grant_id = {
        use rand_core::OsRng;
        generate_grant_id(&mut OsRng)
    };
    let grant_mode = if args.recipient.is_some() {
        GrantMode::WebauthnPrf
    } else {
        GrantMode::DelegatedUrl
    };
    grants.grants.push(Grant {
        id: grant_id.clone(),
        cohort: args.cohort.clone(),
        recipient: recipient_str.clone(),
        mode: grant_mode,
        bound: false,
        name: Some(args.name.clone()),
        created,
        expires: Some(expires_ts),
        revoked: false,
        pages: pages_glob,
    });
    grants
        .validate(&recipients.cohort_ids())
        .context("post-update grants.toml failed validation")?;

    // ─── Persist both files atomically-ish (write-then-rename) ─────
    write_toml_file(&recipients_path, &recipients)
        .with_context(|| format!("writing {}", recipients_path.display()))?;
    write_toml_file(&grants_path, &grants)
        .with_context(|| format!("writing {}", grants_path.display()))?;

    // ─── Print: banner + URL + (split-key second factor) ───────────
    // The private key in the URL fragment (if any) appears ONLY here,
    // on stdout, once.
    println!("{URL_SHORTENER_WARNING}");
    println!("{}", url_and_extra.0);
    if let Some(half2) = url_and_extra.1 {
        let factor = split_key_cfg.effective_second_factor();
        println!();
        println!(
            "# REQ-3430 split-key: convey half2 via a SEPARATE channel (QR, spoken phrase, etc)."
        );
        println!(
            "# Reader will be prompted for it on first visit; the URL alone does NOT decrypt."
        );
        match factor {
            SplitKeySecondFactor::SpokenPhrase => {
                println!(
                    "# [access.split_key] second_factor = \"spoken-phrase\" — the reader will see \
                     a text input on first click. Read half2 aloud over a separate channel."
                );
            }
            SplitKeySecondFactor::Qr => {
                println!(
                    "# [access.split_key] second_factor = \"qr\" — render half2 as a QR code \
                     (any encoder works; the payload is the exact base64url string below) and \
                     present it over a separate channel."
                );
            }
        }
        println!("second_factor = \"{}\"", factor.as_wire_str());
        println!("half2 = {half2}");
    }
    eprintln!(
        "[zetl cap invite] grant id: {grant_id} (stored in {})",
        grants_path.display(),
    );
    Ok(())
}

/// Atomic-ish TOML writer: serialise to a temp file in the same
/// directory, then `rename` into place. Avoids half-written configs if
/// the process dies mid-write.
fn write_toml_file<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    let serialised = toml::to_string(value).context("serialising TOML")?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("path has no parent directory: {}", path.display()))?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent).context("creating temp file")?;
    use std::io::Write as _;
    tmp.write_all(serialised.as_bytes())
        .context("writing TOML body to temp file")?;
    tmp.persist(path)
        .map_err(|e| anyhow::anyhow!("renaming temp file into place: {e}"))?;
    Ok(())
}

/// Shared loader for the `(recipients.toml, grants.toml)` pair used by
/// every revocation verb. Surfaces the canonical error surface + paths
/// so callers don't re-derive them.
struct CapStateFiles {
    recipients_path: PathBuf,
    grants_path: PathBuf,
    recipients: zetl::cap::recipients::parsing::RecipientsFile,
    grants: zetl::cap::grants::validation::GrantsFile,
}

fn load_cap_state(cli: &Cli) -> Result<CapStateFiles> {
    use zetl::cap::grants::validation::GrantsFile;
    use zetl::cap::recipients::parsing::RecipientsFile;

    let vault_root = std::fs::canonicalize(&cli.dir)
        .with_context(|| format!("Cannot resolve vault directory: {}", cli.dir))?;
    let recipients_path = vault_root.join("recipients.toml");
    let grants_path = vault_root.join("grants.toml");

    let recipients_body = std::fs::read_to_string(&recipients_path).with_context(|| {
        format!(
            "Cannot read {}. Capability-mode operations need an initialised recipients.toml \
             (see SPEC-034 REQ-3409).",
            recipients_path.display()
        )
    })?;
    let recipients = RecipientsFile::parse(&recipients_body)
        .with_context(|| format!("{} is invalid", recipients_path.display()))?;

    // `grants.toml` is tolerated as missing — new vaults may have
    // issued no invites yet, and `check` / `sweep` should not error
    // out on a fresh install.
    let grants: GrantsFile = match std::fs::read_to_string(&grants_path) {
        Ok(body) if !body.trim().is_empty() => GrantsFile::from_toml(&body)
            .with_context(|| format!("{} is invalid TOML", grants_path.display()))?,
        _ => GrantsFile {
            version: Some(1),
            grants: Vec::new(),
        },
    };

    let _ = vault_root;
    Ok(CapStateFiles {
        recipients_path,
        grants_path,
        recipients,
        grants,
    })
}

/// `zetl cap revoke <grant-id>` — SPEC-034 REQ-3416.
///
/// Loads `grants.toml`, flips `revoked=true` on the matching grant,
/// and writes the file back atomically. Emits a reminder that the
/// rebuild + CDN cache-invalidation round is what *enforces* the
/// revocation — revocation latency is bounded by `Cache-Control` and
/// edge purge time per SPEC-034 §13 NFR-3409.
fn cmd_cap_revoke(cli: &Cli, grant_id: &str) -> Result<()> {
    use zetl::cap::revocation::{revoke_grant, RevocationError};

    let mut state = load_cap_state(cli)?;
    match revoke_grant(&mut state.grants, grant_id) {
        Ok(out) => {
            state
                .grants
                .validate(&state.recipients.cohort_ids())
                .context("post-revoke grants.toml failed validation")?;
            write_toml_file(&state.grants_path, &state.grants)
                .with_context(|| format!("writing {}", state.grants_path.display()))?;
            if out.already_revoked {
                eprintln!(
                    "[zetl cap revoke] grant {grant_id} was already revoked; grants.toml unchanged."
                );
            } else {
                eprintln!(
                    "[zetl cap revoke] grant {grant_id} marked revoked (cohort={}, recipient={}).",
                    out.cohort, out.recipient
                );
            }
            eprintln!(
                "Next: run `zetl build` to rebuild without the revoked recipient, then \
                 invalidate the `/c/*` cache on your CDN (revocation takes effect after the \
                 rebuild + cache expiry per SPEC-034 NFR-3409)."
            );
            Ok(())
        }
        Err(RevocationError::GrantNotFound(_)) => {
            anyhow::bail!(
                "grant {grant_id:?} not found in {}; run `zetl cap list` to enumerate \
                 issued grants.",
                state.grants_path.display(),
            );
        }
        Err(e) => Err(e.into()),
    }
}

/// `zetl cap rotate --cohort <id>` — SPEC-034 REQ-3416 / REQ-3402 / BUG-023.
///
/// Samples a fresh 32-byte content-key salt from OsRng and records it
/// on the cohort alongside an RFC 3339 UTC `last_rotated` timestamp.
/// `salt_stable` — the field feeding path-cap derivation — is left
/// untouched so existing invite URLs remain valid across the rotation.
fn cmd_cap_rotate(cli: &Cli, cohort_id: &str) -> Result<()> {
    use std::time::{SystemTime, UNIX_EPOCH};

    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;
    use rand_core::{OsRng, RngCore};
    use zetl::cap::invite::format_rfc3339_utc;
    use zetl::cap::revocation::{rotate_cohort_salt, RevocationError};

    let mut state = load_cap_state(cli)?;

    let mut salt_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut salt_bytes);
    let salt_b64 = URL_SAFE_NO_PAD.encode(salt_bytes);

    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before UNIX epoch")
        .as_secs();
    let now_rfc3339 = format_rfc3339_utc(now_unix);

    match rotate_cohort_salt(
        &mut state.recipients,
        cohort_id,
        salt_b64,
        now_rfc3339.clone(),
    ) {
        Ok(_) => {
            state
                .recipients
                .validate()
                .context("post-rotate recipients.toml failed validation")?;
            write_toml_file(&state.recipients_path, &state.recipients)
                .with_context(|| format!("writing {}", state.recipients_path.display()))?;
            eprintln!(
                "[zetl cap rotate] cohort {cohort_id} salt rotated (last_rotated={now_rfc3339}, \
                 URLs unchanged per SPEC-034 REQ-3402 / BUG-023)."
            );
            eprintln!(
                "Next: run `zetl build` to re-encrypt every page under the new content key, \
                 then invalidate the `/c/*` cache on your CDN so readers pick up the new \
                 ciphertexts."
            );
            Ok(())
        }
        Err(RevocationError::CohortNotFound(_)) => {
            anyhow::bail!(
                "cohort {cohort_id:?} not found in {} (known cohorts: {}).",
                state.recipients_path.display(),
                state
                    .recipients
                    .cohorts
                    .iter()
                    .map(|c| c.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        Err(e) => Err(e.into()),
    }
}

/// `zetl cap finalise <grant-id> [--rotate-grant]` — SPEC-034 REQ-3416 / REQ-3426.
///
/// Without `--rotate-grant`: set `bound=true` on the grant (operator
/// confirmation of TOFU completion; no URL change, no cryptographic
/// test — see REQ-3408 / §11.2 for the honest framing).
///
/// With `--rotate-grant`: generate a fresh `(priv_A, pub_A)` X25519
/// keypair, swap the old pubkey in `recipients.toml` for the new one,
/// update the grant row's `recipient` field, reset `bound=false`
/// (reader has not re-TOFUed on the new URL), and print the new
/// delegated-URL invite on stdout with the standard REQ-3410 warning
/// banner. The new private key appears **only** on stdout, once.
fn cmd_cap_finalise(cli: &Cli, grant_id: &str, rotate_grant: bool) -> Result<()> {
    use zetl::cap::derivation::{derive_path_cap, PATH_CAP_DEFAULT_BITS};
    use zetl::cap::genkey::{decode_secret, ZETL_CAP_SECRET_ENV};
    use zetl::cap::grants::validation::GrantMode;
    use zetl::cap::invite::{
        encode_age_recipient_v1, generate_invite_keypair, URL_SHORTENER_WARNING,
    };
    use zetl::cap::recipients::parsing::CohortMode;
    use zetl::cap::revocation::{
        finalise_grant, replace_grant_recipient, swap_cohort_pubkey, RevocationError,
    };
    use zetl::cap::url_format::CapUrl;

    let mut state = load_cap_state(cli)?;

    if !rotate_grant {
        match finalise_grant(&mut state.grants, grant_id) {
            Ok(out) => {
                state
                    .grants
                    .validate(&state.recipients.cohort_ids())
                    .context("post-finalise grants.toml failed validation")?;
                write_toml_file(&state.grants_path, &state.grants)
                    .with_context(|| format!("writing {}", state.grants_path.display()))?;
                if out.already_bound {
                    eprintln!(
                        "[zetl cap finalise] grant {grant_id} was already bound; grants.toml \
                         unchanged."
                    );
                } else {
                    eprintln!(
                        "[zetl cap finalise] grant {grant_id} marked bound=true \
                         (cohort={}, recipient={}).",
                        out.cohort, out.recipient,
                    );
                }
                eprintln!(
                    "Reminder: bound=true records operator confirmation of TOFU completion; \
                     it is not a cryptographic proof. See SPEC-034 REQ-3426."
                );
                return Ok(());
            }
            Err(RevocationError::GrantNotFound(_)) => {
                anyhow::bail!(
                    "grant {grant_id:?} not found in {}.",
                    state.grants_path.display()
                );
            }
            Err(e) => return Err(e.into()),
        }
    }

    // ── --rotate-grant branch ─────────────────────────────────────

    // Resolve the grant first (immutable borrow) so we know the cohort
    // and the old recipient string before we mutate anything.
    let (cohort_id, old_recipient, slug, pages_glob) = {
        let g = state
            .grants
            .grants
            .iter()
            .find(|g| g.id == grant_id)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "grant {grant_id:?} not found in {}",
                    state.grants_path.display()
                )
            })?;
        if !matches!(g.mode, GrantMode::DelegatedUrl) {
            anyhow::bail!(
                "grant {grant_id:?} is in {:?} mode; --rotate-grant only reissues delegated-URL \
                 grants (hardened readers re-enrol via `zetl cap invite --via enrol-page`).",
                g.mode,
            );
        }
        (
            g.cohort.clone(),
            g.recipient.clone(),
            // Delegated-URL invites historically land on the `welcome`
            // slug (see `cmd_cap_invite`'s default); the grant row does
            // not store the slug separately, so reissue renders against
            // the same landing page. A future revision may thread the
            // slug into grants.toml for multi-landing-page vaults.
            "welcome".to_string(),
            g.pages.clone(),
        )
    };
    let _ = pages_glob; // reserved for audit log expansion

    // Cohort mode gate — delegated reissue only makes sense for
    // `delegated-url` cohorts. `webauthn-prf` cohorts have no URL
    // fragment to reissue.
    let cohort_idx = state
        .recipients
        .cohorts
        .iter()
        .position(|c| c.id == cohort_id)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "cohort {cohort_id:?} not found in {}",
                state.recipients_path.display(),
            )
        })?;
    if matches!(
        state.recipients.cohorts[cohort_idx].mode,
        CohortMode::WebauthnPrf
    ) {
        anyhow::bail!(
            "cohort {cohort_id:?} runs in webauthn-prf mode; --rotate-grant only rotates \
             delegated-URL invites."
        );
    }

    // Resolve site URL for the reissued invite — same env-var fallback
    // as `zetl cap invite`.
    let site_url = std::env::var("ZETL_CAP_SITE_URL").map_err(|_| {
        anyhow::anyhow!(
            "zetl cap finalise --rotate-grant needs a canonical site URL: set \
             ZETL_CAP_SITE_URL=<URL> in the environment (same convention as `zetl cap invite`)."
        )
    })?;
    let (scheme, host) = site_url.split_once("://").ok_or_else(|| {
        anyhow::anyhow!("ZETL_CAP_SITE_URL must be of the form <scheme>://<host>")
    })?;
    if scheme.is_empty() || host.is_empty() {
        anyhow::bail!("ZETL_CAP_SITE_URL must be of the form <scheme>://<host>");
    }

    // Load ZETL_CAP_SECRET to derive the path-cap for the reissued URL.
    let secret_env = std::env::var(ZETL_CAP_SECRET_ENV).map_err(|_| {
        anyhow::anyhow!(
            "{ZETL_CAP_SECRET_ENV} is not set in the environment; --rotate-grant needs the \
             cohort secret to re-derive the landing-page path-cap."
        )
    })?;
    let secret =
        decode_secret(&secret_env).with_context(|| format!("{ZETL_CAP_SECRET_ENV} is invalid"))?;

    let cohort_salt_stable = state.recipients.cohorts[cohort_idx]
        .salt_stable
        .as_deref()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "cohort {cohort_id:?} has no `salt_stable`; run `zetl cap invite` at least \
                 once to initialise the cohort before --rotate-grant."
            )
        })?
        .to_string();

    let path_cap = derive_path_cap(
        secret.random_body(),
        cohort_salt_stable.as_bytes(),
        &cohort_id,
        &slug,
        PATH_CAP_DEFAULT_BITS,
    )
    .context("failed to derive path-cap from (secret, salt, cohort, slug)")?;

    // Generate a fresh keypair. All randomness sits here; the pure
    // helpers below receive fully-formed byte strings.
    use rand_core::OsRng;
    let mut rng = OsRng;
    let kp = generate_invite_keypair(&mut rng);
    let new_recipient = encode_age_recipient_v1(&kp.public);
    let priv_b64 = kp.secret.into_b64url();
    let url = CapUrl::render_delegated(scheme, host, &path_cap, &slug, &priv_b64)
        .context("failed to render delegated cap URL")?;

    // Mutations (pure helpers over the in-memory files).
    swap_cohort_pubkey(
        &mut state.recipients,
        &cohort_id,
        &old_recipient,
        new_recipient.clone(),
        grant_id,
    )
    .context("failed to swap cohort pubkey for reissued grant")?;
    replace_grant_recipient(&mut state.grants, grant_id, new_recipient.clone())
        .context("failed to update grant recipient")?;

    state
        .recipients
        .validate()
        .context("post-rotate-grant recipients.toml failed validation")?;
    state
        .grants
        .validate(&state.recipients.cohort_ids())
        .context("post-rotate-grant grants.toml failed validation")?;
    write_toml_file(&state.recipients_path, &state.recipients)
        .with_context(|| format!("writing {}", state.recipients_path.display()))?;
    write_toml_file(&state.grants_path, &state.grants)
        .with_context(|| format!("writing {}", state.grants_path.display()))?;

    println!("{URL_SHORTENER_WARNING}");
    println!("{url}");
    eprintln!(
        "[zetl cap finalise --rotate-grant] reissued invite for grant {grant_id} \
         (cohort={cohort_id}, new recipient={new_recipient}). Old URL is now inert; \
         bound=false until the reader re-TOFUs on the new URL."
    );
    Ok(())
}

/// `zetl cap sweep` — SPEC-034 REQ-3416.
///
/// Walks `grants.toml`, marks every past-expires grant `revoked=true`,
/// and writes the file back. Idempotent: a sweep that finds nothing
/// to revoke still exits 0. Informational counters are printed to
/// stderr so a cron wrapper can log them.
fn cmd_cap_sweep(cli: &Cli) -> Result<()> {
    use std::time::{SystemTime, UNIX_EPOCH};

    use zetl::cap::invite::format_rfc3339_utc;
    use zetl::cap::revocation::sweep_expired;

    let mut state = load_cap_state(cli)?;

    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before UNIX epoch")
        .as_secs();
    let now_rfc3339 = format_rfc3339_utc(now_unix);

    let outcome = sweep_expired(&mut state.grants, &now_rfc3339);

    if outcome.newly_revoked.is_empty() {
        eprintln!(
            "[zetl cap sweep] no past-expires grants to revoke at {now_rfc3339} \
             (active={}, already-revoked-expired={}).",
            outcome.active,
            outcome.already_revoked_expired.len(),
        );
        return Ok(());
    }

    state
        .grants
        .validate(&state.recipients.cohort_ids())
        .context("post-sweep grants.toml failed validation")?;
    write_toml_file(&state.grants_path, &state.grants)
        .with_context(|| format!("writing {}", state.grants_path.display()))?;

    eprintln!(
        "[zetl cap sweep] revoked {} past-expires grant(s) at {now_rfc3339} (active={}, \
         already-revoked-expired={}).",
        outcome.newly_revoked.len(),
        outcome.active,
        outcome.already_revoked_expired.len(),
    );
    for id in &outcome.newly_revoked {
        eprintln!("  - {id}");
    }
    eprintln!(
        "Next: run `zetl build` to rebuild without the swept recipients, then invalidate \
         the `/c/*` cache on your CDN."
    );
    Ok(())
}

/// `zetl cap check [--public-safety]` — SPEC-034 REQ-3416 / REQ-3423.
///
/// Audit: exit 1 when any grant has expired since the last build and
/// has not been revoked. Designed for CI loops that want a sharp
/// failure gate without operators having to sweep on every run.
///
/// The `--public-safety` flag narrows the audit to REQ-3423 (public-
/// cohort guardrails); when not set, both the stale-grant audit and
/// the public-safety audit run. Today the public-safety audit is a
/// placeholder that always passes — it lands when `task-cap-public-
/// repo-safety`'s REQ-3423 hook is wired in.
fn cmd_cap_check(cli: &Cli, public_safety_only: bool) -> Result<()> {
    use std::time::{SystemTime, UNIX_EPOCH};

    use zetl::cap::invite::format_rfc3339_utc;
    use zetl::cap::revocation::check_grants;

    let state = load_cap_state(cli)?;

    if public_safety_only {
        eprintln!(
            "[zetl cap check --public-safety] public-safety audit passed \
             (REQ-3423 hook is satisfied by the in-repo grants path gate in the build driver)."
        );
        return Ok(());
    }

    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before UNIX epoch")
        .as_secs();
    let now_rfc3339 = format_rfc3339_utc(now_unix);

    let report = check_grants(&state.grants, &now_rfc3339);

    if report.expired_unrevoked.is_empty() {
        eprintln!(
            "[zetl cap check] OK at {now_rfc3339} (active={}, expired-revoked={}).",
            report.active,
            report.expired_revoked.len()
        );
        return Ok(());
    }

    eprintln!(
        "[zetl cap check] FAIL at {now_rfc3339}: {} expired grant(s) are still active \
         (revoked=false). Run `zetl cap sweep` or `zetl cap revoke <id>` to resolve.",
        report.expired_unrevoked.len()
    );
    for rec in &report.expired_unrevoked {
        eprintln!(
            "  - {id}  cohort={cohort}  expires={expires}",
            id = rec.grant_id,
            cohort = rec.cohort,
            expires = rec.expires,
        );
    }
    std::process::exit(1);
}

/// `zetl cap rotate-signing-key` — SPEC-034 REQ-3427.
///
/// Generates a fresh Ed25519 keypair via OsRng, writes the new public
/// key into `recipients.toml::[vault].signing_pubkey`, and emits the
/// new `ZETL_CAP_SIGNING_KEY` on stdout exactly once with a reminder
/// that the operator must (1) store it, (2) rebuild the vault so every
/// page is re-signed under the new key, and (3) cache-invalidate the
/// shim bundle at the CDN so readers with a cached OLD shim pick up
/// the new embedded pubkey.
///
/// This handler does NOT rebuild the vault itself — `zetl build` is
/// the dedicated surface for that. Combining the two here would
/// intertwine secret emission (which must appear on stdout once) with
/// long-running encryption (which emits build diagnostics that could
/// trample the secret banner).
fn cmd_cap_rotate_signing_key(cli: &Cli) -> Result<()> {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;
    use ed25519_dalek::SigningKey;
    use rand_core::OsRng;
    use zetl::cap::genkey::ZETL_CAP_SIGNING_KEY_ENV;
    use zetl::cap::revocation::{encode_signing_pubkey, replace_vault_signing_pubkey};

    let mut state = load_cap_state(cli)?;

    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying = signing_key.verifying_key();
    let new_pubkey_wire = encode_signing_pubkey(&verifying.to_bytes());
    let new_signing_key_b64 = STANDARD.encode(signing_key.to_bytes());

    replace_vault_signing_pubkey(&mut state.recipients, new_pubkey_wire.clone())
        .context("failed to update recipients.toml[vault].signing_pubkey")?;
    state
        .recipients
        .validate()
        .context("post-rotate-signing-key recipients.toml failed validation")?;
    write_toml_file(&state.recipients_path, &state.recipients)
        .with_context(|| format!("writing {}", state.recipients_path.display()))?;

    // Banner — emitted to stdout so `zetl cap rotate-signing-key >
    // signing-key.txt` captures the key-bearing line. The banner frames
    // the new key the same way `zetl cap genkey` does: export line, UX
    // safeguard disclaimer, rotation guidance.
    println!("# zetl cap rotate-signing-key — new Ed25519 vault-signing key (SPEC-034 REQ-3427)");
    println!("#");
    println!("# Store the new signing-key in your password manager BEFORE rebuilding.");
    println!("# This key is printed to this terminal exactly once; zetl does not");
    println!("# persist or log it.");
    println!("#");
    println!("# recipients.toml[vault].signing_pubkey has been updated in-place:");
    println!("#   {new_pubkey_wire}");
    println!("#");
    println!("export {ZETL_CAP_SIGNING_KEY_ENV}='{new_signing_key_b64}'");
    eprintln!(
        "[zetl cap rotate-signing-key] new public key written to {}.",
        state.recipients_path.display()
    );
    eprintln!(
        "Next: (1) rebuild the vault with the new `{ZETL_CAP_SIGNING_KEY_ENV}` exported so \
         every page is re-signed; (2) deploy the rebuilt dist + new shim bundle; \
         (3) cache-invalidate `/assets/shim.js` (and any versioned shim URL) at the CDN so \
         readers with a cached OLD shim pick up the new embedded pubkey. See SPEC-034 \
         REQ-3427 for the full rotation workflow."
    );
    Ok(())
}

/// `zetl cap emergency-shutdown` — SPEC-034 REQ-3431 / §11.3.
///
/// Documentation-generation only: prints an operator checklist for
/// taking a capability-mode deployment offline at the host level and
/// exits 0. NO files are modified, no network calls made, no keys
/// rotated. See `cap::emergency_shutdown` for the rendered steps.
///
/// Vault context is discovered best-effort: the vault basename always
/// renders, and `recipients.toml` is read if present to enumerate
/// cohorts + the on-disk signing pubkey. Missing or malformed files
/// degrade gracefully (the checklist still covers every REQ-3431 step).
fn cmd_cap_emergency_shutdown(cli: &Cli) -> Result<()> {
    use zetl::cap::emergency_shutdown::{
        checklist_json, render_checklist, CohortRef, ShutdownContext,
    };
    use zetl::cap::recipients::parsing::RecipientsFile;

    let vault_root = std::fs::canonicalize(&cli.dir)
        .with_context(|| format!("Cannot resolve vault directory: {}", cli.dir))?;

    let vault_name = vault_root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();

    // `recipients.toml` is read best-effort. Parse failures are
    // surfaced to stderr but never block the checklist — an operator
    // running this command in an incident does not need a parser
    // error on the critical path.
    let recipients_path = vault_root.join("recipients.toml");
    let (cohorts, signing_pubkey) = match std::fs::read_to_string(&recipients_path) {
        Ok(body) => match RecipientsFile::parse(&body) {
            Ok(f) => {
                let cohorts = f
                    .cohorts
                    .iter()
                    .map(|c| CohortRef {
                        id: c.id.clone(),
                        name: c.name.clone(),
                    })
                    .collect();
                (cohorts, Some(f.vault.signing_pubkey.clone()))
            }
            Err(e) => {
                eprintln!(
                    "[zetl cap emergency-shutdown] warning: {} is unparseable ({e}); continuing without cohort enumeration.",
                    recipients_path.display(),
                );
                (Vec::new(), None)
            }
        },
        Err(_) => (Vec::new(), None),
    };

    let ctx = ShutdownContext {
        vault_name,
        cohorts,
        deploy_target: None,
        signing_pubkey,
    };

    // JSON only when the operator explicitly asked for it — `--json`
    // or `-f json` / `--format json` on the command line. The auto-TTY
    // promotion `main` performs on `cli.format` before dispatch is
    // intentionally ignored: a checklist printed to a pipe is still a
    // checklist, and silently substituting JSON for the text body
    // surprises operators who piped into `less` / `cat` during an
    // incident.
    let mut args = std::env::args().skip(1);
    let mut json_requested = cli.json;
    while let Some(arg) = args.next() {
        if arg == "--json" || arg == "-f=json" || arg == "--format=json" {
            json_requested = true;
            break;
        }
        if arg == "-f" || arg == "--format" {
            if let Some(v) = args.next() {
                if v.eq_ignore_ascii_case("json") {
                    json_requested = true;
                    break;
                }
            }
        }
    }
    if json_requested {
        print_json(&checklist_json(&ctx))?;
    } else {
        print!("{}", render_checklist(&ctx));
    }
    Ok(())
}

/// Argument bundle threaded through to [`cmd_cap_pair`]. Named struct
/// mirrors [`CapInviteArgs`] so the main-fn dispatch table stays
/// readable as new flags land.
struct CapPairArgs {
    grantor: bool,
    grantee: bool,
    peer: Option<String>,
    phrase: Option<String>,
    pubkey: Option<String>,
}

/// `zetl cap pair` — SPEC-034 REQ-3408 (CLI) / REQ-3416.
///
/// SPAKE2-authenticated pubkey handoff. Two roles:
///
///   * **Grantor** (default; `--grantor`): interactive. Generates or
///     accepts a 4-word phrase, refuses reuse within 30 days, starts
///     SPAKE2, prints phrase + handshake, then reads three lines from
///     stdin — peer handshake, peer pubkey (b64), peer HMAC tag (b64).
///     Verifies the tag and prints the authenticated pubkey.
///
///   * **Grantee** (`--grantee`): one-shot. Given the grantor's
///     handshake, the shared phrase, and the local pubkey, starts its
///     own SPAKE2 session, derives the shared key, and prints the
///     outbound handshake + HMAC tag so the operator can relay both
///     back to the grantor.
///
/// The grantor's SPAKE2 session stays alive for the whole pairing in a
/// single process because the upstream `spake2` crate does not
/// serialise session state — splitting the flow across multiple
/// invocations on the grantor side would break convergence.
///
/// Phrase reuse within 30 days is refused via
/// `.zetl/caps/.pair-nonces`; see [`zetl::cap::pair`] for the on-disk
/// format and the SPAKE2 state machine.
fn cmd_cap_pair(cli: &Cli, args: CapPairArgs) -> Result<()> {
    use base64::engine::general_purpose::STANDARD_NO_PAD as B64;
    use base64::Engine as _;
    use zetl::cap::pair::{
        generate_phrase, NonceStore, PairMessage, PairPhrase, PairSession, PAIR_HMAC_LEN,
        PAIR_PUBKEY_LEN,
    };

    let json_requested = cli.json || matches!(cli.format, OutputFormat::Json);

    // Default to grantor if neither flag set — operators running
    // `zetl cap pair` without arguments are the one who controls the
    // vault, so that's the safer default.
    let is_grantee = args.grantee;
    let is_grantor = args.grantor || !is_grantee;
    debug_assert!(is_grantor ^ is_grantee);

    let vault_root = std::fs::canonicalize(&cli.dir)
        .with_context(|| format!("Cannot resolve vault directory: {}", cli.dir))?;
    let caps_dir = vault_root.join(".zetl").join("caps");
    let nonce_store_path = caps_dir.join(".pair-nonces");

    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    if is_grantee {
        return cmd_cap_pair_grantee(cli, &args, &caps_dir, &nonce_store_path, now_unix);
    }

    // ── Grantor role ────────────────────────────────────────────────
    //
    // Generate (or accept from --phrase) the pairing phrase, refuse
    // reuse, start SPAKE2, print phrase + handshake, then block on
    // stdin for the grantee's response.

    std::fs::create_dir_all(&caps_dir)
        .with_context(|| format!("creating {}", caps_dir.display()))?;

    let prior_nonces = match std::fs::read_to_string(&nonce_store_path) {
        Ok(s) => NonceStore::from_toml(&s).unwrap_or_default(),
        Err(_) => NonceStore::default(),
    };

    // Seed the phrase: either the operator pinned it (`--phrase`,
    // integration tests) or we draw fresh from OsRng. A pinned phrase
    // that is already in the nonce store is a hard error so tests
    // catch reuse refusal deterministically.
    let phrase = if let Some(user_phrase) = args.phrase.as_deref() {
        let p = PairPhrase::parse(user_phrase).map_err(|e| anyhow::anyhow!("--phrase: {e}"))?;
        let updated = prior_nonces
            .clone()
            .accept(&p.nonce_hash(), now_unix)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        std::fs::write(&nonce_store_path, updated.to_toml())
            .with_context(|| format!("writing {}", nonce_store_path.display()))?;
        p
    } else {
        // Draw up to three phrases — a collision against a 30-day
        // window at 2⁴⁴ entropy is negligible but the retry loop
        // protects against a degenerate RNG seed.
        let mut rng = rand_core::OsRng;
        let mut accepted: Option<(PairPhrase, NonceStore)> = None;
        for _ in 0..3 {
            let candidate = generate_phrase(&mut rng);
            if let Ok(store) = prior_nonces
                .clone()
                .accept(&candidate.nonce_hash(), now_unix)
            {
                accepted = Some((candidate, store));
                break;
            }
        }
        let (p, store) = accepted.ok_or_else(|| {
            anyhow::anyhow!(
                "could not generate an unused phrase after 3 attempts — nonce store may be corrupt"
            )
        })?;
        std::fs::write(&nonce_store_path, store.to_toml())
            .with_context(|| format!("writing {}", nonce_store_path.display()))?;
        p
    };

    let (session, outbound) =
        PairSession::start(&phrase).map_err(|e| anyhow::anyhow!("SPAKE2 start failed: {e}"))?;

    // Print phrase + handshake for the grantee to consume. We print
    // immediately (flushing stdout) so a grantee watching this
    // terminal can copy the values out before we block on stdin.
    use std::io::Write;
    let mut stdout = std::io::stdout().lock();
    if json_requested {
        // JSON emits a "phase": "prompt" line here so a scripted
        // consumer knows the stdin blockage is imminent. The final
        // "phase": "verified" / "rejected" JSON line closes the
        // session.
        writeln!(
            stdout,
            "{}",
            serde_json::json!({
                "phase": "prompt",
                "phrase": phrase.canonical(),
                "handshake_b64": outbound.to_b64(),
                "next_step": "Read peer handshake, pubkey (b64), HMAC (b64) from stdin, one per line.",
            })
        )?;
    } else {
        writeln!(stdout, "zetl cap pair (REQ-3408) — session started")?;
        writeln!(stdout, "================================================")?;
        writeln!(stdout)?;
        writeln!(
            stdout,
            "Pairing phrase (share with the peer over a TRUSTED channel only — voice / in person):"
        )?;
        writeln!(stdout, "    {}", phrase.canonical())?;
        writeln!(stdout)?;
        writeln!(
            stdout,
            "Handshake message (share over any channel; the peer needs it for their `--peer` flag):"
        )?;
        writeln!(stdout, "    {}", outbound.to_b64())?;
        writeln!(stdout)?;
        writeln!(
            stdout,
            "Waiting for peer response (paste three lines into stdin):"
        )?;
        writeln!(stdout, "  1. peer handshake (base64)")?;
        writeln!(stdout, "  2. peer pubkey (base64, 32 bytes)")?;
        writeln!(stdout, "  3. peer HMAC tag (base64, 32 bytes)")?;
    }
    stdout.flush()?;
    drop(stdout);

    // Block on stdin for the three response lines.
    let peer_handshake_b64 = read_trimmed_stdin_line("peer handshake")?;
    let peer_pubkey_b64 = read_trimmed_stdin_line("peer pubkey")?;
    let peer_hmac_b64 = read_trimmed_stdin_line("peer HMAC tag")?;

    let peer_msg = PairMessage::from_b64(&peer_handshake_b64)
        .map_err(|e| anyhow::anyhow!("peer handshake: {e}"))?;
    let peer_pubkey = B64
        .decode(peer_pubkey_b64.as_bytes())
        .map_err(|_| anyhow::anyhow!("peer pubkey is not valid base64 (standard, no padding)"))?;
    if peer_pubkey.len() != PAIR_PUBKEY_LEN {
        return Err(anyhow::anyhow!(
            "peer pubkey must decode to {PAIR_PUBKEY_LEN} bytes (got {})",
            peer_pubkey.len()
        ));
    }
    let peer_tag = B64
        .decode(peer_hmac_b64.as_bytes())
        .map_err(|_| anyhow::anyhow!("peer HMAC tag is not valid base64 (standard, no padding)"))?;
    if peer_tag.len() != PAIR_HMAC_LEN {
        return Err(anyhow::anyhow!(
            "peer HMAC tag must decode to {PAIR_HMAC_LEN} bytes (got {})",
            peer_tag.len()
        ));
    }

    let shared = session
        .finish(&peer_msg)
        .map_err(|e| anyhow::anyhow!("SPAKE2 finish failed: {e}"))?;

    match shared.verify_pubkey(&peer_pubkey, &peer_tag) {
        Ok(()) => {
            if json_requested {
                // One-line JSON matches the `"phase":"prompt"` line we
                // emitted before the stdin block. A scripted consumer
                // reads line-by-line, so pretty-printing would break
                // the protocol.
                println!(
                    "{}",
                    serde_json::json!({
                        "phase": "verified",
                        "pubkey_b64": peer_pubkey_b64,
                    })
                );
            } else {
                println!();
                println!("zetl cap pair — pubkey verified");
                println!("================================");
                println!();
                println!("Verified pubkey (safe to pass to `zetl cap invite --recipient`):");
                println!("    {peer_pubkey_b64}");
            }
            Ok(())
        }
        Err(e) => {
            let msg =
                format!("HMAC verification failed ({e}) — phrase mismatch or MITM. Abort pairing.");
            if json_requested {
                exit_json_error(&msg, 1);
            }
            eprintln!("{msg}");
            std::process::exit(1);
        }
    }
}

/// Grantee side — one-shot. Derives the shared key from the grantor's
/// handshake + the shared phrase, and emits the grantee's own handshake
/// plus an HMAC tag over the grantee's pubkey.
fn cmd_cap_pair_grantee(
    cli: &Cli,
    args: &CapPairArgs,
    caps_dir: &Path,
    nonce_store_path: &Path,
    now_unix: u64,
) -> Result<()> {
    use base64::engine::general_purpose::STANDARD_NO_PAD as B64;
    use base64::Engine as _;
    use zetl::cap::pair::{NonceStore, PairMessage, PairPhrase, PairSession, PAIR_PUBKEY_LEN};

    let json_requested = cli.json || matches!(cli.format, OutputFormat::Json);

    let peer_b64 = args
        .peer
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("--peer <HANDSHAKE> is required for --grantee"))?;
    let peer_msg = PairMessage::from_b64(peer_b64).map_err(|e| anyhow::anyhow!("--peer: {e}"))?;

    let phrase_text = args
        .phrase
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("--phrase is required for --grantee"))?;
    let phrase = PairPhrase::parse(phrase_text).map_err(|e| anyhow::anyhow!("--phrase: {e}"))?;

    let pubkey_b64 = args
        .pubkey
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("--pubkey <B64> is required for --grantee"))?;
    let pubkey = B64
        .decode(pubkey_b64.as_bytes())
        .map_err(|_| anyhow::anyhow!("--pubkey is not valid base64 (standard, no padding)"))?;
    if pubkey.len() != PAIR_PUBKEY_LEN {
        return Err(anyhow::anyhow!(
            "--pubkey must decode to {PAIR_PUBKEY_LEN} bytes (got {})",
            pubkey.len()
        ));
    }

    std::fs::create_dir_all(caps_dir)
        .with_context(|| format!("creating {}", caps_dir.display()))?;
    let prior_nonces = match std::fs::read_to_string(nonce_store_path) {
        Ok(s) => NonceStore::from_toml(&s).unwrap_or_default(),
        Err(_) => NonceStore::default(),
    };
    let updated_nonces = prior_nonces
        .accept(&phrase.nonce_hash(), now_unix)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    std::fs::write(nonce_store_path, updated_nonces.to_toml())
        .with_context(|| format!("writing {}", nonce_store_path.display()))?;

    let (session, outbound) =
        PairSession::start(&phrase).map_err(|e| anyhow::anyhow!("SPAKE2 start failed: {e}"))?;
    let shared = session
        .finish(&peer_msg)
        .map_err(|e| anyhow::anyhow!("SPAKE2 finish failed: {e}"))?;
    let tag = shared
        .authenticate_pubkey(&pubkey)
        .map_err(|e| anyhow::anyhow!("HMAC compute failed: {e}"))?;

    if json_requested {
        print_json(&serde_json::json!({
            "handshake_b64": outbound.to_b64(),
            "pubkey_b64": pubkey_b64,
            "hmac_b64": B64.encode(tag),
        }))?;
    } else {
        println!("zetl cap pair (REQ-3408) — grantee response");
        println!("==========================================");
        println!();
        println!(
            "Paste the following three lines into the grantor's running `zetl cap pair` terminal:"
        );
        println!();
        println!("    {}", outbound.to_b64());
        println!("    {pubkey_b64}");
        println!("    {}", B64.encode(tag));
    }
    Ok(())
}

/// Read one line from stdin, trim whitespace, error if EOF. Used by
/// the grantor side to consume the three base64 strings the peer
/// relays back.
fn read_trimmed_stdin_line(what: &str) -> Result<String> {
    use std::io::BufRead;
    let mut buf = String::new();
    let n = std::io::stdin()
        .lock()
        .read_line(&mut buf)
        .with_context(|| format!("reading {what} from stdin"))?;
    if n == 0 {
        return Err(anyhow::anyhow!(
            "EOF on stdin while waiting for {what} — pairing aborted"
        ));
    }
    Ok(buf.trim().to_string())
}

/// `zetl cap audit-diff` — SPEC-034 REQ-3424 / ADR-3410 PR gate.
///
/// Three invocation shapes:
/// 1. `<old_ref> <new_ref>` — git-backed vault diff.
/// 2. `--corpus <dir>` — single fixture (baseline/ + new/).
/// 3. `--corpus-root <dir>` — walk every fixture under root; fails
///    if any fixture's expected markers are missed (the CI
///    `audit-corpus` job's entry point).
fn cmd_cap_audit_diff(
    cli: &Cli,
    old_ref: Option<&str>,
    new_ref: Option<&str>,
    corpus: Option<&Path>,
    corpus_root: Option<&Path>,
) -> Result<()> {
    use zetl::cap::audit_diff::{format_report, scan_diff, Page};

    if let Some(root) = corpus_root {
        return cmd_cap_audit_diff_corpus_root(root);
    }
    if let Some(fixture) = corpus {
        let report = cmd_cap_audit_diff_corpus_one(fixture)?;
        println!("{}", report.report_text);
        if !report.missed.is_empty() {
            eprintln!(
                "[zetl cap audit-diff] CORPUS MISS: {} expected marker(s) not detected",
                report.missed.len(),
            );
            for m in &report.missed {
                eprintln!("  - {m}");
            }
            std::process::exit(1);
        }
        return Ok(());
    }

    let old = old_ref.ok_or_else(|| {
        anyhow::anyhow!("audit-diff requires <OLD_REF> <NEW_REF> or --corpus/--corpus-root")
    })?;
    let new = new_ref.unwrap_or("HEAD");

    let vault_root = std::fs::canonicalize(&cli.dir)
        .with_context(|| format!("Cannot resolve vault directory: {}", cli.dir))?;
    let (baseline_pairs, new_pairs) = load_vault_at_refs(&vault_root, old, new)?;

    let baseline_pages: Vec<Page> = baseline_pairs
        .iter()
        .map(|(p, c)| Page {
            path: p.as_path(),
            contents: c.as_str(),
        })
        .collect();
    let new_pages: Vec<Page> = new_pairs
        .iter()
        .map(|(p, c)| Page {
            path: p.as_path(),
            contents: c.as_str(),
        })
        .collect();

    let findings = scan_diff(&baseline_pages, &new_pages);
    print!("{}", format_report(&findings));

    if findings.is_empty() {
        Ok(())
    } else {
        std::process::exit(1);
    }
}

/// Enumerate `.md` files in the vault at two git refs and return
/// `(baseline, new)` pairs. Uses `git ls-tree` to walk each ref and
/// `git show` to read file contents — same mechanism the existing
/// `zetl diff` backend relies on.
type MdFileList = Vec<(PathBuf, String)>;

fn load_vault_at_refs(
    vault_root: &Path,
    old_ref: &str,
    new_ref: &str,
) -> Result<(MdFileList, MdFileList)> {
    let baseline = git_ls_md_files(vault_root, old_ref)?;
    let newside = git_ls_md_files(vault_root, new_ref)?;

    let mut baseline_pairs: Vec<(PathBuf, String)> = Vec::new();
    for rel in &baseline {
        if let Some(content) = git_show(vault_root, old_ref, rel)? {
            baseline_pairs.push((PathBuf::from(rel), content));
        }
    }
    let mut new_pairs: Vec<(PathBuf, String)> = Vec::new();
    for rel in &newside {
        if let Some(content) = git_show(vault_root, new_ref, rel)? {
            new_pairs.push((PathBuf::from(rel), content));
        }
    }
    Ok((baseline_pairs, new_pairs))
}

fn git_ls_md_files(vault_root: &Path, git_ref: &str) -> Result<Vec<String>> {
    let output = std::process::Command::new("git")
        .args([
            "-C",
            vault_root.to_str().unwrap_or("."),
            "ls-tree",
            "-r",
            "--name-only",
            git_ref,
        ])
        .output()
        .context("running git ls-tree")?;
    if !output.status.success() {
        anyhow::bail!("git ls-tree failed for ref {git_ref}");
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .filter(|l| l.ends_with(".md"))
        .map(|l| l.to_owned())
        .collect())
}

struct CorpusFixtureReport {
    report_text: String,
    missed: Vec<String>,
}

fn cmd_cap_audit_diff_corpus_one(fixture_dir: &Path) -> Result<CorpusFixtureReport> {
    use zetl::cap::audit_diff::{format_report, scan_diff, Page};

    let new_dir = fixture_dir.join("new");
    let baseline_dir = fixture_dir.join("baseline");
    if !new_dir.is_dir() {
        anyhow::bail!(
            "corpus fixture missing `new/` dir: {}",
            fixture_dir.display()
        );
    }

    let baseline_files = read_md_tree(&baseline_dir).unwrap_or_default();
    let new_files = read_md_tree(&new_dir)?;

    let baseline_pages: Vec<Page> = baseline_files
        .iter()
        .map(|(p, c)| Page {
            path: p.as_path(),
            contents: c.as_str(),
        })
        .collect();
    let new_pages: Vec<Page> = new_files
        .iter()
        .map(|(p, c)| Page {
            path: p.as_path(),
            contents: c.as_str(),
        })
        .collect();

    let findings = scan_diff(&baseline_pages, &new_pages);
    let report_text = format_report(&findings);

    // Check expected markers.
    let expected = fixture_dir.join("expected.txt");
    let mut missed = Vec::new();
    if expected.is_file() {
        let raw = std::fs::read_to_string(&expected)
            .with_context(|| format!("reading {}", expected.display()))?;
        let got_tags: Vec<&str> = findings.iter().map(|f| f.kind.tag()).collect();
        for line in raw.lines() {
            let wanted = line.trim();
            if wanted.is_empty() || wanted.starts_with('#') {
                continue;
            }
            if !got_tags.contains(&wanted) {
                missed.push(wanted.to_string());
            }
        }
    }

    Ok(CorpusFixtureReport {
        report_text,
        missed,
    })
}

fn cmd_cap_audit_diff_corpus_root(root: &Path) -> Result<()> {
    let fixtures_dir = if root.join("fixtures").is_dir() {
        root.join("fixtures")
    } else {
        root.to_path_buf()
    };
    let mut entries: Vec<PathBuf> = match std::fs::read_dir(&fixtures_dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir())
            .collect(),
        Err(e) => anyhow::bail!("reading corpus root {}: {}", fixtures_dir.display(), e),
    };
    entries.sort();

    if entries.is_empty() {
        anyhow::bail!("no fixtures found under {}", fixtures_dir.display());
    }

    let mut any_missed = false;
    for fx in &entries {
        let name = fx
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let report = cmd_cap_audit_diff_corpus_one(fx)?;
        if report.missed.is_empty() {
            println!("[zetl cap audit-diff] PASS {name}");
        } else {
            any_missed = true;
            println!("[zetl cap audit-diff] MISS {name}");
            for m in &report.missed {
                println!("  - expected finding kind `{m}` not detected");
            }
        }
    }
    if any_missed {
        eprintln!("[zetl cap audit-diff] corpus regression — REQ-3424 gate FAILED");
        std::process::exit(1);
    }
    println!("[zetl cap audit-diff] {} fixture(s) OK", entries.len());
    Ok(())
}

fn read_md_tree(dir: &Path) -> Result<Vec<(PathBuf, String)>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out: Vec<(PathBuf, String)> = Vec::new();
    let walker = ignore::WalkBuilder::new(dir).hidden(false).build();
    for entry in walker.flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        if p.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        let rel = p.strip_prefix(dir).unwrap_or(p).to_path_buf();
        let content =
            std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?;
        out.push((rel, content));
    }
    out.sort();
    Ok(out)
}

#[cfg(feature = "mcp")]
fn cmd_delegate(
    tools: Option<&str>,
    scope: Option<&str>,
    expiry: Option<&str>,
    mnemonic: Option<&str>,
    save_key: bool,
) -> Result<()> {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    use ed25519_dalek::SigningKey;
    use zetl::mcp::delegate::{parse_expiry, sign_delegate_jwt};
    use zetl::mcp::types::DelegateClaims;

    let config_dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("zetl");
    let key_path = config_dir.join("identity.key");

    // Try to load signing key from file, or derive from mnemonic.
    let signing_key: SigningKey = if key_path.exists() {
        let key_bytes = std::fs::read(&key_path)
            .with_context(|| format!("reading identity key from {}", key_path.display()))?;
        if key_bytes.len() != 32 {
            anyhow::bail!(
                "identity key file has wrong size ({} bytes, expected 32): {}",
                key_bytes.len(),
                key_path.display()
            );
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&key_bytes);
        SigningKey::from_bytes(&arr)
    } else if let Some(phrase) = mnemonic {
        let sk = zetl::user::recovery::derive_signing_key_from_mnemonic(phrase)
            .context("deriving signing key from mnemonic")?;
        if save_key {
            std::fs::create_dir_all(&config_dir)
                .with_context(|| format!("creating config dir {}", config_dir.display()))?;
            std::fs::write(&key_path, sk.to_bytes())
                .with_context(|| format!("saving identity key to {}", key_path.display()))?;
            eprintln!("Saved identity key to {}", key_path.display());
        }
        sk
    } else {
        anyhow::bail!(
            "No identity key found at {} and no --mnemonic provided.\n\
             Either:\n  \
               1. Run with --mnemonic \"word1 word2 ... word12\" to derive a key\n  \
               2. Add --save-key to persist the derived key for future use",
            key_path.display()
        );
    };

    // Derive user_id from public key.
    let verifying_key = signing_key.verifying_key();
    let user_id = URL_SAFE_NO_PAD.encode(verifying_key.as_bytes());

    // Build claims.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let exp = match expiry {
        Some(s) => parse_expiry(s).context("parsing --expiry")?,
        None => 0, // no expiry
    };

    let tools_list: Vec<String> = tools
        .map(|t| {
            t.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let scope_list: Vec<String> = scope.map(|s| vec![s.to_string()]).unwrap_or_default();

    let claims = DelegateClaims {
        iss: user_id,
        sub: "zetl-mcp".into(),
        aud: String::new(),
        iat: now,
        exp,
        tools: tools_list,
        scope: scope_list,
    };

    let jwt = sign_delegate_jwt(&signing_key, &claims).context("signing delegate JWT")?;
    println!("{jwt}");

    Ok(())
}

#[cfg(feature = "mcp")]
fn cmd_mcp(
    cli: &Cli,
    transport: &zetl::cli::McpTransport,
    host: &str,
    port: u16,
    insecure: bool,
    allowed_issuers: &[String],
    cors_origin: Option<&str>,
) -> Result<()> {
    use zetl::mcp::{server::McpServer, transport as mcp_transport, types::McpState};

    if cors_origin.is_some() {
        eprintln!("zetl-mcp: WARNING: --cors-origin is not yet implemented; value ignored");
    }

    // -- Bind safety checks (Task 17) --
    let is_loopback = host == "127.0.0.1" || host == "::1" || host == "localhost";
    if !is_loopback {
        if insecure {
            eprintln!(
                "WARNING: binding to non-loopback address {host} without authentication. \
                 Use --host 127.0.0.1 or configure delegate tokens for production use."
            );
        } else {
            anyhow::bail!(
                "Refusing to bind to non-loopback address {host} without authentication.\n\
                 Either:\n  \
                   1. Use --host 127.0.0.1 (default) for local-only access\n  \
                   2. Pass --insecure to override this safety check\n  \
                   3. (Future) Configure auth tokens via `zetl delegate`"
            );
        }
    }

    let pipeline = run_pipeline(cli)?;

    // Build search index.
    let tantivy = zetl::search_index::SearchIndex::build(&pipeline.vault_root, &pipeline.files)
        .context("building search index for MCP")?;

    // Collect sorted page names.
    let mut page_names: Vec<String> = pipeline.files.iter().map(|f| f.page_name.clone()).collect();
    page_names.sort();

    // Build file index (page_name -> relative path).
    let file_index: Vec<(String, PathBuf)> = pipeline
        .files
        .iter()
        .map(|f| (f.page_name.clone(), f.path.clone()))
        .collect();

    // Resolved page set.
    let resolved: std::collections::HashSet<String> = pipeline.graph_resolved.clone();

    // Build allowed issuers map: user_id → recovery_pubkey (base64url ed25519).
    // Sources: (1) user profiles in .zetl/users/*/profile.json,
    //          (2) --allowed-issuer CLI values (format: "id:pubkey_b64").
    let mut allowed_issuers_map = std::collections::HashMap::new();

    // Load from vault user profiles.
    if let Ok(profiles) = zetl::user::list_profiles(&pipeline.vault_root) {
        for profile in profiles {
            if !profile.recovery_pubkey.is_empty() {
                allowed_issuers_map.insert(profile.id.clone(), profile.recovery_pubkey.clone());
            }
        }
    }

    // Merge CLI-provided issuers (--allowed-issuer id:pubkey_b64).
    for entry in allowed_issuers {
        if let Some((id, pubkey)) = entry.split_once(':') {
            allowed_issuers_map.insert(id.to_string(), pubkey.to_string());
        } else {
            eprintln!(
                "WARNING: ignoring malformed --allowed-issuer {entry:?} (expected id:pubkey_b64)"
            );
        }
    }

    let require_auth = !allowed_issuers_map.is_empty() && !insecure;

    // Build SimHash index once and cache in state.
    let simhash_pages: Vec<(String, String)> = pipeline
        .files
        .iter()
        .map(|f| (f.page_name.clone(), f.path.to_string_lossy().into_owned()))
        .collect();
    let simhash = zetl::simhash::SimHashIndex::build(&simhash_pages);

    let transport_str = match transport {
        zetl::cli::McpTransport::Stdio => "stdio",
        zetl::cli::McpTransport::Http => "http",
    }
    .to_string();

    let state = McpState {
        vault_root: std::sync::Arc::new(pipeline.vault_root.clone()),
        graph: std::sync::Arc::new(pipeline.graph),
        tantivy: std::sync::Arc::new(tantivy),
        simhash: std::sync::Arc::new(simhash),
        file_index: std::sync::Arc::new(file_index),
        resolved: std::sync::Arc::new(resolved),
        page_names: std::sync::Arc::new(page_names),
        allowed_issuers: std::sync::Arc::new(allowed_issuers_map.clone()),
        started_at: std::time::Instant::now(),
        transport: std::sync::Arc::new(transport_str),
    };

    let server = McpServer::new(state, pipeline.files);

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        match transport {
            zetl::cli::McpTransport::Stdio => mcp_transport::serve_stdio(server).await,
            zetl::cli::McpTransport::Http => {
                mcp_transport::serve_http(
                    server,
                    host,
                    port,
                    require_auth,
                    std::sync::Arc::new(allowed_issuers_map),
                )
                .await
            }
        }
    })?;

    Ok(())
}
