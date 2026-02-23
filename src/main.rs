use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use clap::Parser;
use comfy_table::{Cell, Table};
use serde::Serialize;

use zetl::cache::{files_needing_reparse, load_cache, save_cache};
use zetl::cli::{Cli, Command, FailLevel, OutputFormat};
use zetl::graph::LinkGraph;
use zetl::scanner::{resolve_page_name, scan_vault};
use zetl::search::{search_vault, SearchConfig};
use zetl::simhash::SimHashIndex;
use zetl::types::{DiagnosticLevel, ParsedFile};

// ── Common pipeline ────────────────────────────────────────────────────────

struct Pipeline {
    files: Vec<ParsedFile>,
    file_index: Vec<(String, PathBuf)>,
    graph: LinkGraph,
    vault_root: PathBuf,
}

fn run_pipeline(cli: &Cli) -> Result<Pipeline> {
    let vault_root = std::fs::canonicalize(&cli.dir)
        .with_context(|| format!("Cannot resolve vault directory: {}", cli.dir))?;

    // Load cache (unless --no-cache)
    let cached = if cli.no_cache {
        None
    } else {
        load_cache(&vault_root)?
    };

    // Scan vault for all markdown files
    let all_scanned = scan_vault(&vault_root, &[])?;

    // Incremental re-parse: use cache to skip unchanged files
    let files = if let Some(ref cached_map) = cached {
        // Build current file mtime list
        let current_files: Vec<(PathBuf, std::time::SystemTime)> = all_scanned
            .iter()
            .map(|f| (f.path.clone(), f.mtime))
            .collect();

        let needs_reparse: HashSet<PathBuf> =
            files_needing_reparse(cached_map, &current_files)
                .into_iter()
                .collect();

        // Merge: use cached data for unchanged files, freshly scanned data for changed ones
        let mut merged = Vec::new();
        for scanned in &all_scanned {
            if needs_reparse.contains(&scanned.path) {
                merged.push(scanned.clone());
            } else if let Some(cached_file) = cached_map.get(&scanned.path) {
                merged.push(cached_file.clone());
            } else {
                merged.push(scanned.clone());
            }
        }
        merged
    } else {
        all_scanned
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

    Ok(Pipeline {
        files,
        file_index,
        graph,
        vault_root,
    })
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
    // Ignore write errors — we're exiting anyway
    let _ = serde_json::to_string_pretty(&err).map(|json| println!("{json}"));
    std::process::exit(code);
}

/// Handle a page-not-found error: JSON on stdout when format=json, plain text on stderr otherwise.
fn exit_page_not_found(format: &OutputFormat, message: &str) -> ! {
    match format {
        OutputFormat::Json => exit_json_error(message, 1),
        OutputFormat::Table => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    }
}

// ── Command handlers ───────────────────────────────────────────────────────

fn cmd_index(cli: &Cli) -> Result<()> {
    let start = Instant::now();
    let pipeline = run_pipeline(cli)?;
    let elapsed = start.elapsed();

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
    }

    let result = IndexResult {
        files_scanned: pipeline.files.len(),
        links_found: total_links,
        dead_links: dead_links.len(),
        diagnostics: total_diagnostics,
        elapsed_ms: elapsed.as_millis(),
    };

    match cli.format {
        OutputFormat::Json => print_json(&result)?,
        OutputFormat::Table => {
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
            table.add_row(vec![
                Cell::new("Dead links"),
                Cell::new(result.dead_links),
            ]);
            table.add_row(vec![
                Cell::new("Diagnostics"),
                Cell::new(result.diagnostics),
            ]);
            table.add_row(vec![
                Cell::new("Elapsed (ms)"),
                Cell::new(result.elapsed_ms),
            ]);
            println!("{table}");
        }
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

    let resolved_page =
        find_page(page, &pipeline.file_index, fuzzy, &pipeline.files).unwrap_or_else(|e| {
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
                extract_context(
                    &pipeline.vault_root,
                    &edge.source_file,
                    edge.line,
                    context,
                )
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
    }

    let output = LinksOutput {
        page: resolved_page,
        depth,
        links: entries,
    };

    match cli.format {
        OutputFormat::Json => print_json(&output)?,
        OutputFormat::Table => {
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
                    row.push(Cell::new(
                        entry.context.as_deref().unwrap_or(""),
                    ));
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
                    row.push(Cell::new(if conc_str.is_empty() {
                        "-"
                    } else {
                        &conc_str
                    }));
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

    let resolved_page =
        find_page(page, &pipeline.file_index, fuzzy, &pipeline.files).unwrap_or_else(|e| {
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
    }

    let output = BacklinksOutput {
        page: resolved_page,
        depth,
        backlinks: entries,
    };

    match cli.format {
        OutputFormat::Json => print_json(&output)?,
        OutputFormat::Table => {
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
                    row.push(Cell::new(
                        entry.context.as_deref().unwrap_or(""),
                    ));
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
                    row.push(Cell::new(if conc_str.is_empty() {
                        "-"
                    } else {
                        &conc_str
                    }));
                }
                table.add_row(row);
            }
            println!("Backlinks to '{}':", output.page);
            println!("{table}");
        }
    }

    Ok(())
}

fn cmd_check(
    cli: &Cli,
    show_dead_links: bool,
    show_orphans: bool,
    show_syntax: bool,
    show_spl: bool,
    fail_on: &FailLevel,
) -> Result<()> {
    let pipeline = run_pipeline(cli)?;

    // If none of the flags are set, show all
    let show_all = !show_dead_links && !show_orphans && !show_syntax && !show_spl;

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
    let spl_diagnostics: Vec<zetl::types::Diagnostic> = if show_all || show_spl {
        collect_spl_diagnostics(&pipeline.files)
    } else {
        vec![]
    };

    #[derive(Serialize)]
    struct CheckOutput {
        dead_links: Vec<zetl::graph::DeadLink>,
        orphans: Vec<zetl::graph::Orphan>,
        syntax_errors: Vec<zetl::types::Diagnostic>,
        spl_diagnostics: Vec<zetl::types::Diagnostic>,
    }

    let output = CheckOutput {
        dead_links: dead,
        orphans: orphan_list,
        syntax_errors: diagnostics,
        spl_diagnostics,
    };

    match cli.format {
        OutputFormat::Json => print_json(&output)?,
        OutputFormat::Table => {
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
                    table.add_row(vec![
                        Cell::new(&o.page),
                        Cell::new(o.forward_links),
                    ]);
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

            if output.dead_links.is_empty()
                && output.orphans.is_empty()
                && output.syntax_errors.is_empty()
                && output.spl_diagnostics.is_empty()
            {
                println!("No issues found.");
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
            .any(|d| d.level == DiagnosticLevel::Warning);

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
        OutputFormat::Table => {
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
    let stats = pipeline.graph.stats(top);

    match cli.format {
        OutputFormat::Json => print_json(&stats)?,
        OutputFormat::Table => {
            let mut table = Table::new();
            table.set_header(vec!["Metric", "Value"]);
            table.add_row(vec![Cell::new("Pages"), Cell::new(stats.pages)]);
            table.add_row(vec![Cell::new("Links"), Cell::new(stats.links)]);
            table.add_row(vec![
                Cell::new("Unique targets"),
                Cell::new(stats.unique_targets),
            ]);
            table.add_row(vec![Cell::new("Dead links"), Cell::new(stats.dead_links)]);
            table.add_row(vec![Cell::new("Orphans"), Cell::new(stats.orphans)]);
            table.add_row(vec![
                Cell::new("Connected components"),
                Cell::new(stats.connected_components),
            ]);
            println!("{table}");

            if !stats.most_linked.is_empty() {
                println!();
                let mut ml_table = Table::new();
                ml_table.set_header(vec!["#", "Page", "Backlinks"]);
                for (i, ml) in stats.most_linked.iter().enumerate() {
                    ml_table.add_row(vec![
                        Cell::new(i + 1),
                        Cell::new(&ml.page),
                        Cell::new(ml.backlink_count),
                    ]);
                }
                println!("Most linked pages:");
                println!("{ml_table}");
            }
        }
    }

    Ok(())
}

fn cmd_path(cli: &Cli, from: &str, to: &str, max_depth: usize) -> Result<()> {
    let pipeline = run_pipeline(cli)?;

    let resolved_from =
        find_page(from, &pipeline.file_index, false, &pipeline.files).unwrap_or_else(|e| {
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
            OutputFormat::Table => {
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
                OutputFormat::Table => {
                    eprintln!("{msg}.");
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
    regex: bool,
    case_sensitive: bool,
    all: bool,
    path_filter: Option<&str>,
) -> Result<()> {
    let vault_root = std::fs::canonicalize(&cli.dir)
        .with_context(|| format!("Cannot resolve vault directory: {}", cli.dir))?;

    let config = SearchConfig {
        query,
        context_chars: context,
        limit,
        regex,
        case_sensitive,
        body_only: !all,
        path_filter,
    };

    let output = match search_vault(&vault_root, &config) {
        Ok(o) => o,
        Err(e) => {
            let msg = format!("{e}");
            let code = if msg.contains("Empty search query") || msg.contains("Invalid regex") {
                2
            } else {
                1
            };
            match cli.format {
                OutputFormat::Json => exit_json_error(&msg, code),
                OutputFormat::Table => {
                    eprintln!("Error: {msg}");
                    std::process::exit(code);
                }
            }
        }
    };

    if output.total_matches == 0 {
        match cli.format {
            OutputFormat::Json => print_json(&output)?,
            OutputFormat::Table => {
                println!("No matches found for '{query}'.");
            }
        }
        std::process::exit(1);
    }

    match cli.format {
        OutputFormat::Json => print_json(&output)?,
        OutputFormat::Table => {
            let mut table = Table::new();
            let mut headers = vec!["Page", "Line", "Col"];
            if context > 0 {
                headers.push("Context");
            }
            table.set_header(headers);
            for r in &output.results {
                let mut row = vec![
                    Cell::new(&r.page),
                    Cell::new(r.line),
                    Cell::new(r.column),
                ];
                if context > 0 {
                    row.push(Cell::new(
                        r.context.as_deref().unwrap_or(""),
                    ));
                }
                table.add_row(row);
            }
            println!(
                "Search results for '{}' ({} matches):",
                query, output.total_matches
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
    let files = scan_vault(&vault_root, &[])?;

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
    pages.sort_by(|a, b| a.page.to_lowercase().cmp(&b.page.to_lowercase()));

    #[derive(Serialize)]
    struct ListOutput {
        pages: Vec<PageEntry>,
        total: usize,
    }

    let total = pages.len();
    let output = ListOutput { pages, total };

    match cli.format {
        OutputFormat::Json => print_json(&output)?,
        OutputFormat::Table => {
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
    nodes.sort_by(|a, b| a.page.to_lowercase().cmp(&b.page.to_lowercase()));

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
    }

    let output = ExportOutput {
        node_count: nodes.len(),
        edge_count: edges.len(),
        nodes,
        edges,
    };

    match cli.format {
        OutputFormat::Json => print_json(&output)?,
        OutputFormat::Table => {
            println!("Graph: {} nodes, {} edges", output.node_count, output.edge_count);
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
fn build_page_conclusions_map(
    files: &[ParsedFile],
) -> HashMap<String, Vec<PageConclusionEntry>> {
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
    let link_end = target_line[link_start..].find("]]").map(|i| link_start + i + 2)?;

    // Extract n chars before and after, snapping to char boundaries
    let ctx_start = floor_char_boundary(target_line, link_start.saturating_sub(n));
    let ctx_end = ceil_char_boundary(target_line, (link_end + n).min(target_line.len()));

    Some(target_line[ctx_start..ctx_end].to_string())
}

fn cmd_tui(cli: &Cli) -> Result<()> {
    let pipeline = run_pipeline(cli)?;

    // Build resolved_pages map for App::new
    let mut resolved_pages: HashMap<String, String> = HashMap::new();
    for file in &pipeline.files {
        for link in &file.links {
            let key = link.raw_target.clone();
            if resolved_pages.contains_key(&key) {
                continue;
            }
            if let Some(resolved) = resolve_page_name(&link.target_page, &pipeline.file_index) {
                resolved_pages.insert(key, resolved);
            }
        }
    }

    let mut app = zetl::tui::App::new(
        pipeline.files,
        pipeline.file_index,
        pipeline.graph,
        pipeline.vault_root,
        &resolved_pages,
    );

    zetl::tui::run(&mut app)?;
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

#[cfg(feature = "reason")]
fn cmd_reason_status(
    cli: &Cli,
    positive: bool,
    negative: bool,
    definite: bool,
    defeasible: bool,
    literal_pat: Option<&str>,
) -> Result<()> {
    use zetl::reason::build_theory;
    use zetl::reason::types::ConclusionType;

    let pipeline = run_pipeline(cli)?;

    // Collect all SPL blocks from parsed files
    let spl_blocks: Vec<_> = pipeline
        .files
        .iter()
        .flat_map(|f| f.spl_blocks.clone())
        .collect();

    if spl_blocks.is_empty() {
        match cli.format {
            OutputFormat::Json => exit_json_error("No SPL blocks found in vault", 1),
            OutputFormat::Table => {
                eprintln!("No SPL blocks found in vault.");
                std::process::exit(1);
            }
        }
    }

    let block_count = spl_blocks.len();
    let result = build_theory(&spl_blocks)?;

    // Determine if there were parse errors
    let parse_error_count = result
        .diagnostics
        .iter()
        .filter(|d| {
            d.level == DiagnosticLevel::Error && d.message.contains("SPL parse error")
        })
        .count();
    let has_parse_errors = parse_error_count > 0;
    let all_blocks_failed = parse_error_count == block_count && result.conclusions.is_empty();

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
                        ConclusionType::DefeasiblyProvable
                            | ConclusionType::DefeasiblyNotProvable
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
                },
                diagnostics: result.diagnostics,
            };
            print_json(&output)?;
        }
        OutputFormat::Table => {
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

    if all_blocks_failed {
        std::process::exit(2);
    } else if has_parse_errors {
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
    use zetl::reason::build_theory;

    let pipeline = run_pipeline(cli)?;

    // Collect all SPL blocks from parsed files
    let spl_blocks: Vec<_> = pipeline
        .files
        .iter()
        .flat_map(|f| f.spl_blocks.clone())
        .collect();

    if spl_blocks.is_empty() {
        match cli.format {
            OutputFormat::Json => exit_json_error("No SPL blocks found in vault", 1),
            OutputFormat::Table => {
                eprintln!("No SPL blocks found in vault.");
                std::process::exit(1);
            }
        }
    }

    let result = build_theory(&spl_blocks)?;

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

        let matching_conclusion = matching_conclusions.iter().find(|c| {
            matches!(
                c.conclusion_type,
                zetl::reason::types::ConclusionType::DefeasiblyNotProvable
            )
        }).or_else(|| matching_conclusions.first())
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
            format!("Literal '{}' not found in any conclusion", literal_input)
        } else {
            let did_you_mean: Vec<String> = suggestions.iter().map(|s| format!("'{}'", s)).collect();
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
            OutputFormat::Table => {
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
    use zetl::reason::build_theory;
    use zetl::reason::types::ConclusionType;

    let pipeline = run_pipeline(cli)?;

    let spl_blocks: Vec<_> = pipeline
        .files
        .iter()
        .flat_map(|f| f.spl_blocks.clone())
        .collect();

    if spl_blocks.is_empty() {
        match cli.format {
            OutputFormat::Json => exit_json_error("No SPL blocks found in vault", 1),
            OutputFormat::Table => {
                eprintln!("No SPL blocks found in vault.");
                std::process::exit(1);
            }
        }
    }

    let result = build_theory(&spl_blocks)?;

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
            format!("Literal '{}' not found in theory", literal_input)
        } else {
            let did_you_mean: Vec<String> =
                suggestions.iter().map(|s| format!("'{}'", s)).collect();
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
            OutputFormat::Table => {
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
            "Literal '{}' IS provable. Use 'zetl reason explain {}' instead.",
            literal_input, literal_input
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
                    hint: format!("zetl reason explain {}", literal_input),
                };
                print_json(&output)?;
                std::process::exit(1);
            }
            OutputFormat::Table => {
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
        let negated_literal = if literal_input.starts_with('~') {
            literal_input[1..].to_string()
        } else {
            format!("~{}", literal_input)
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
    };

    match cli.format {
        OutputFormat::Json => print_json(&output)?,
        OutputFormat::Table => {
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
                println!(
                    "  To make it provable, add a rule or fact asserting it."
                );
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
                                        println!(
                                            "      Would need to be asserted by:"
                                        );
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
                                    println!("    {}: {}", blocker.blocker_type, blocker.explanation);
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

// ── Conflicts command ──────────────────────────────────────────────────────

#[cfg(feature = "reason")]
fn cmd_reason_conflicts(cli: &Cli, suggest: bool, fail_on_conflicts: bool) -> Result<()> {
    use zetl::reason::build_theory;
    use zetl::reason::types::RuleType;

    let pipeline = run_pipeline(cli)?;

    let spl_blocks: Vec<_> = pipeline
        .files
        .iter()
        .flat_map(|f| f.spl_blocks.clone())
        .collect();

    if spl_blocks.is_empty() {
        match cli.format {
            OutputFormat::Json => exit_json_error("No SPL blocks found in vault", 1),
            OutputFormat::Table => {
                eprintln!("No SPL blocks found in vault.");
                std::process::exit(1);
            }
        }
    }

    let result = build_theory(&spl_blocks)?;

    // Build a set of all rule heads grouped by their base literal name.
    // A conflict exists when there are rules for both `p` and `~p`.
    // We need to find literal names where rules produce both a literal and its complement.
    let mut rules_for_literal: HashMap<String, Vec<&zetl::reason::types::ProvenancedRule>> =
        HashMap::new();
    for rule in &result.rules {
        let head_str = rule.head.to_string();
        rules_for_literal
            .entry(head_str)
            .or_default()
            .push(rule);
    }

    // Also include facts as potential sources of conflict
    let mut facts_for_literal: HashMap<String, Vec<&zetl::reason::types::ProvenancedFact>> =
        HashMap::new();
    for fact in &result.facts {
        let lit_str = fact.literal.to_string();
        facts_for_literal
            .entry(lit_str)
            .or_default()
            .push(fact);
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
            (lit_str.clone(), format!("~{}", lit_str))
        };

        // Skip if we already processed this pair
        if !seen_names.insert(base_name.clone()) {
            continue;
        }

        // Check if the complement also has rules/facts
        let positive = base_name.clone();
        let negative = format!("~{}", base_name);

        let pos_has_rules = rules_for_literal.contains_key(&positive)
            || facts_for_literal.contains_key(&positive);
        let neg_has_rules = rules_for_literal.contains_key(&negative)
            || facts_for_literal.contains_key(&negative);

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
        if pos_defeasible.is_empty() && neg_defeasible.is_empty() && !pos_has_strict && !neg_has_strict {
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
    };

    match cli.format {
        OutputFormat::Json => print_json(&output)?,
        OutputFormat::Table => {
            if conflicts.is_empty() {
                println!("No unresolved conflicts found in theory.");
            } else {
                println!(
                    "{} unresolved conflict(s) found:\n",
                    conflicts.len()
                );

                for (i, conflict) in conflicts.iter().enumerate() {
                    println!(
                        "{}. Contested literal: {}",
                        i + 1,
                        conflict.literal
                    );
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
                            println!("     - {}", suggestion);
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
            "Remove the fact '~{}' if it is no longer applicable",
            base_name
        ));
    }
    if !pos_facts.is_empty() && !neg_rules.is_empty() {
        suggestions.push(format!(
            "Remove the fact '{}' if it is no longer applicable",
            base_name
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

// ── Cross-referencing: provenance with link graph ──────────────────────────

#[cfg(feature = "reason")]
fn cmd_reason_provenance(cli: &Cli, literal_input: &str) -> Result<()> {
    use zetl::reason::build_theory;
    use zetl::reason::types::ConclusionType;

    let pipeline = run_pipeline(cli)?;

    let spl_blocks: Vec<_> = pipeline
        .files
        .iter()
        .flat_map(|f| f.spl_blocks.clone())
        .collect();

    if spl_blocks.is_empty() {
        match cli.format {
            OutputFormat::Json => exit_json_error("No SPL blocks found in vault", 1),
            OutputFormat::Table => {
                eprintln!("No SPL blocks found in vault.");
                std::process::exit(1);
            }
        }
    }

    let result = build_theory(&spl_blocks)?;

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
            "Literal '{}' not found in conclusions. Use `zetl reason status` to see all conclusions.",
            literal_str
        );
        match cli.format {
            OutputFormat::Json => exit_json_error(&msg, 1),
            OutputFormat::Table => {
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
    cross_refs.dedup_by(|a, b| {
        a.from_page == b.from_page && a.to_page == b.to_page && a.line == b.line
    });

    // Build per-conclusion output
    let conclusion_entries: Vec<ProvenanceConclusionEntry> = matching
        .iter()
        .map(|c| {
            let tag = match c.conclusion_type {
                ConclusionType::DefinitelyProvable => "+D",
                ConclusionType::DefinitelyNotProvable => "-D",
                ConclusionType::DefeasiblyProvable => "+d",
                ConclusionType::DefeasiblyNotProvable => "-d",
            };
            ProvenanceConclusionEntry {
                conclusion_type: tag.to_string(),
                literal: c.literal.clone(),
                proof_sources: c.proof_sources.clone(),
            }
        })
        .collect();

    let output = ProvenanceOutput {
        literal: literal_str.to_string(),
        conclusions: conclusion_entries,
        source_pages: source_pages.clone(),
        cross_references: cross_refs.clone(),
    };

    match cli.format {
        OutputFormat::Json => print_json(&output)?,
        OutputFormat::Table => {
            println!("Provenance for '{}':\n", literal_str);

            for entry in &output.conclusions {
                println!("  {} {}", entry.conclusion_type, entry.literal);
                println!("  Proof sources:");
                for ps in &entry.proof_sources {
                    if let Some(ref label) = ps.rule_label {
                        println!(
                            "    [[{}]]:{} — {} ({})",
                            ps.page, ps.line, ps.contribution, label
                        );
                    } else {
                        println!(
                            "    [[{}]]:{} — {}",
                            ps.page, ps.line, ps.contribution
                        );
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
    conclusions: Vec<ProvenanceConclusionEntry>,
    source_pages: Vec<String>,
    cross_references: Vec<ProvenanceCrossRef>,
}

#[cfg(feature = "reason")]
#[derive(Debug, Clone, Serialize)]
struct ProvenanceConclusionEntry {
    conclusion_type: String,
    literal: String,
    proof_sources: Vec<zetl::reason::types::ProofSource>,
}

#[cfg(feature = "reason")]
#[derive(Debug, Clone, Serialize)]
struct ProvenanceCrossRef {
    from_page: String,
    to_page: String,
    direction: String,
    line: u32,
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
            let negated_input = if literal_input.starts_with('~') {
                literal_input[1..].to_string()
            } else {
                format!("~{}", literal_input)
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
                    format!("'{}' is definitely not provable (-D): no strict proof chain exists", literal_input)
                }
                ConclusionType::DefeasiblyNotProvable => {
                    if defeat_chain.is_empty() {
                        format!("'{}' is defeasibly not provable (-d): no undefeated defeasible proof chain exists", literal_input)
                    } else {
                        format!(
                            "'{}' is defeasibly not provable (-d): defeated by {} rule(s)",
                            literal_input,
                            defeat_chain.len()
                        )
                    }
                }
                _ => format!("'{}' holds as {}", literal_input, conclusion_type_str),
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
            println!("Explanation for '{}':", literal_input);
            println!("  Conclusion: {} {}", conclusion_type_str, literal_input);
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
                        "    {} [{}]: {} => {}  ([[{}]]:{})",
                        r.label,
                        format!("{:?}", r.rule_type),
                        body_strs.join(", "),
                        r.head,
                        r.source_page,
                        r.source_line,
                    );
                }
            }
        }
        ExplainFormat::Natural => {
            match conclusion.conclusion_type {
                ConclusionType::DefinitelyNotProvable => {
                    println!("The literal '{}' is definitely not provable.", literal_input);
                    println!("No strict proof chain can establish it from the known facts and rules.");
                }
                ConclusionType::DefeasiblyNotProvable => {
                    println!("The literal '{}' is defeasibly not provable.", literal_input);
                    if defeat_chain.is_empty() {
                        println!("No undefeated defeasible proof chain exists.");
                    } else {
                        println!("It is blocked by the following rule(s):");
                        for r in &defeat_chain {
                            println!("  - Rule '{}' from [[{}]]:{}", r.label, r.source_page, r.source_line);
                        }
                    }
                }
                _ => {
                    println!("'{}' holds as {} {}.", literal_input, conclusion_type_str, literal_input);
                }
            }
        }
        ExplainFormat::Dot => {
            // Minimal DOT graph for negative conclusions
            println!("digraph explanation {{");
            println!("  rankdir=BT;");
            println!("  node [shape=box];");
            let conclusion_id = format!("\"{}\\n{}\"", conclusion_type_str, literal_input);
            println!("  {} [style=filled, fillcolor=lightcoral];", conclusion_id);
            for r in &defeat_chain {
                let rule_id = format!("\"{}\"", r.label);
                println!("  {} -> {} [label=\"defeats\"];", rule_id, conclusion_id);
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
    println!("  Conclusion: {} {}", output.conclusion_type, output.literal);
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
                println!("      blocked by: {}", blocker);
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
        "{}{} ({}){}{}", pad, node.literal, node.derivation, rule_str, source_str
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
        let rule_node_id = format!("rule_{}", counter);
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
        println!("  {} -> {};", rule_node_id, parent_id);

        // Body literals
        for child in &node.body {
            let child_id = format!("lit_{}", counter);
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
            println!("  {} -> {};", child_id, rule_node_id);

            // Recurse into children of the body node
            emit_dot_node(child, &child_id, counter);
        }
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

// ── Main ───────────────────────────────────────────────────────────────────

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match &cli.command {
        Command::Index => cmd_index(&cli),
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
            fail_on,
        } => cmd_check(&cli, *dead_links, *orphans, *syntax, *spl, fail_on),
        Command::Similar {
            query,
            threshold,
            limit,
        } => cmd_similar(&cli, query, *threshold, *limit),
        Command::Search {
            query,
            context,
            limit,
            regex,
            case_sensitive,
            all,
            path,
        } => cmd_search(
            &cli,
            query,
            *context,
            *limit,
            *regex,
            *case_sensitive,
            *all,
            path.as_deref(),
        ),
        Command::List => cmd_list(&cli),
        Command::Stats { top } => cmd_stats(&cli, *top),
        Command::Path {
            from,
            to,
            max_depth,
        } => cmd_path(&cli, from, to, *max_depth),
        Command::Export => cmd_export(&cli),
        Command::Tui => cmd_tui(&cli),
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
                    format,
                } => cmd_reason_explain(&cli, literal, *depth, format),
                ReasonCommand::WhyNot { literal } => cmd_reason_why_not(&cli, literal),
                ReasonCommand::Conflicts {
                    suggest,
                    fail_on_conflicts,
                } => cmd_reason_conflicts(&cli, *suggest, *fail_on_conflicts),
                ReasonCommand::Provenance { literal } => cmd_reason_provenance(&cli, literal),
                _ => {
                    eprintln!("This reason subcommand is not yet implemented.");
                    std::process::exit(1);
                }
            }
        }
    }
}
