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
) -> Result<()> {
    let pipeline = run_pipeline(cli)?;

    let resolved_page =
        find_page(page, &pipeline.file_index, fuzzy, &pipeline.files).unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(1);
        });

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
    }

    let mut entries: Vec<LinkEntry> = Vec::new();

    while let Some((current_page, current_depth)) = queue.pop_front() {
        if current_depth >= depth {
            continue;
        }

        let forward = pipeline.graph.forward_links(&current_page);
        for edge in &forward {
            // Determine target page name from the graph
            let target_name = {
                // Look up by raw_target in resolved_pages, or fall back to scanning graph nodes
                // We need to find the target node. The edge has source_file and line info.
                // Since forward_links returns EdgeMeta, we need to find the target.
                // Let's look at graph edges directly.
                let mut target = None;
                if let Some(&src_idx) = pipeline.graph.node_map.get(&current_page) {
                    use petgraph::visit::EdgeRef;
                    for e in pipeline
                        .graph
                        .graph
                        .edges_directed(src_idx, petgraph::Direction::Outgoing)
                    {
                        let w = e.weight();
                        if w.line == edge.line
                            && w.source_file == edge.source_file
                            && w.alias == edge.alias
                        {
                            target = Some(pipeline.graph.graph[e.target()].clone());
                            break;
                        }
                    }
                }
                target.unwrap_or_default()
            };

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

            entries.push(LinkEntry {
                source: current_page.clone(),
                target: target_name.clone(),
                line: edge.line,
                alias: edge.alias.clone(),
                heading: edge.heading.clone(),
                is_embed: edge.is_embed,
                context: ctx,
                hop: current_depth + 1,
            });

            if !visited.contains(&target_name) {
                visited.insert(target_name.clone());
                queue.push_back((target_name, current_depth + 1));
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
) -> Result<()> {
    let pipeline = run_pipeline(cli)?;

    let resolved_page =
        find_page(page, &pipeline.file_index, fuzzy, &pipeline.files).unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(1);
        });

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
    }

    let mut entries: Vec<BacklinkEntry> = Vec::new();

    while let Some((current_page, current_depth)) = queue.pop_front() {
        if current_depth >= depth {
            continue;
        }

        let backlinks = pipeline.graph.backlinks(&current_page);
        for bl in &backlinks {
            // Context: read N chars around the wikilink position in the source file
            let ctx = if context > 0 {
                // Find the source file path from the parsed files
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

            entries.push(BacklinkEntry {
                source: bl.source.clone(),
                target: current_page.clone(),
                line: bl.line,
                alias: bl.alias.clone(),
                is_embed: bl.is_embed,
                context: ctx,
                hop: current_depth + 1,
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
    fail_on: &FailLevel,
) -> Result<()> {
    let pipeline = run_pipeline(cli)?;

    // If none of the flags are set, show all
    let show_all = !show_dead_links && !show_orphans && !show_syntax;

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

    #[derive(Serialize)]
    struct CheckOutput {
        dead_links: Vec<zetl::graph::DeadLink>,
        orphans: Vec<zetl::graph::Orphan>,
        syntax_errors: Vec<zetl::types::Diagnostic>,
    }

    let output = CheckOutput {
        dead_links: dead,
        orphans: orphan_list,
        syntax_errors: diagnostics,
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

            if output.dead_links.is_empty()
                && output.orphans.is_empty()
                && output.syntax_errors.is_empty()
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
            .any(|d| d.level == DiagnosticLevel::Error);

    let has_warnings = output
        .syntax_errors
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
            eprintln!("{e}");
            std::process::exit(1);
        });

    let resolved_to =
        find_page(to, &pipeline.file_index, false, &pipeline.files).unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(1);
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
            #[derive(Serialize)]
            struct NoPath {
                from: String,
                to: String,
                path: Option<Vec<String>>,
            }
            let output = NoPath {
                from: resolved_from.clone(),
                to: resolved_to.clone(),
                path: None,
            };
            match cli.format {
                OutputFormat::Json => print_json(&output)?,
                OutputFormat::Table => {
                    eprintln!(
                        "No path found from '{resolved_from}' to '{resolved_to}' within {max_depth} hops."
                    );
                }
            }
            std::process::exit(1);
        }
    }

    Ok(())
}

fn cmd_search(
    cli: &Cli,
    query: &str,
    context: usize,
    limit: usize,
    regex: bool,
    case_sensitive: bool,
    all: bool,
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
    };

    let output = search_vault(&vault_root, &config)?;

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

// ── Context extraction helper ──────────────────────────────────────────────

/// Read the source file and extract `n` chars of context around the wikilink
/// at the given line.
fn extract_context(vault_root: &Path, source_file: &str, line: u32, n: usize) -> Option<String> {
    let full_path = vault_root.join(source_file);
    let content = std::fs::read_to_string(&full_path).ok()?;

    // Find the line content (1-indexed)
    let target_line = content.lines().nth((line as usize).saturating_sub(1))?;

    // Find the wikilink on this line
    let link_start = target_line.find("[[")?;
    let link_end = target_line[link_start..].find("]]").map(|i| link_start + i + 2)?;

    // Extract n chars before and after
    let ctx_start = link_start.saturating_sub(n);
    let ctx_end = (link_end + n).min(target_line.len());

    Some(target_line[ctx_start..ctx_end].to_string())
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
        } => cmd_links(&cli, page, *fuzzy, *context, *depth),
        Command::Backlinks {
            page,
            fuzzy,
            context,
            depth,
        } => cmd_backlinks(&cli, page, *fuzzy, *context, *depth),
        Command::Check {
            dead_links,
            orphans,
            syntax,
            fail_on,
        } => cmd_check(&cli, *dead_links, *orphans, *syntax, fail_on),
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
        } => cmd_search(&cli, query, *context, *limit, *regex, *case_sensitive, *all),
        Command::Stats { top } => cmd_stats(&cli, *top),
        Command::Path {
            from,
            to,
            max_depth,
        } => cmd_path(&cli, from, to, *max_depth),
    }
}
