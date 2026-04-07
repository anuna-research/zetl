//! Tool handler implementations — one function per MCP tool.

use crate::mcp::auth::resolve_page;
use crate::mcp::types::{McpState, ToolError};
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Validate that a search query is not empty or whitespace-only.
fn validate_search_params(query: &str) -> Result<(), ToolError> {
    if query.trim().is_empty() {
        return Err(ToolError::InvalidParam("query must not be empty".into()));
    }
    Ok(())
}

/// Clamp a limit to [1, max].
fn clamp_limit(limit: usize, max: usize) -> usize {
    limit.min(max).max(1)
}

// ---------------------------------------------------------------------------
// Tool handlers
// ---------------------------------------------------------------------------

/// **search** — full-text search over the vault via Tantivy.
///
/// Returns up to `limit` (clamped to 1–100) results ranked by score.
/// `mode` can be "fulltext" (default), "semantic", or "hybrid".
pub fn tool_search(
    state: &McpState,
    query: &str,
    limit: usize,
    mode: &str,
) -> Result<Value, ToolError> {
    validate_search_params(query)?;
    let limit = clamp_limit(limit, 100);

    match mode {
        "semantic" | "hybrid" => {
            return Err(ToolError::FeatureUnavailable(
                "semantic/hybrid search requires --features semantic".into(),
            ));
        }
        _ => {} // "fulltext" or anything else falls through to tantivy
    }

    let hits = state
        .tantivy
        .query(query, limit)
        .map_err(|e| ToolError::Internal(format!("search error: {e}")))?;

    let results: Vec<Value> = hits
        .iter()
        .map(|h| {
            // Build a basic snippet: first 200 chars of file content.
            let snippet = {
                let rel_path = state
                    .file_index
                    .iter()
                    .find(|(name, _)| name == &h.page_name)
                    .map(|(_, p)| p.clone());
                rel_path
                    .and_then(|rp| {
                        let abs = state.vault_root.join(&rp);
                        std::fs::read_to_string(abs).ok()
                    })
                    .map(|content| {
                        let trimmed = content.trim_start();
                        let end = trimmed
                            .char_indices()
                            .nth(200)
                            .map(|(i, _)| i)
                            .unwrap_or(trimmed.len());
                        trimmed[..end].replace('\n', " ")
                    })
                    .unwrap_or_default()
            };

            json!({
                "page": h.page_name,
                "path": h.path,
                "score": h.score,
                "snippet": snippet,
            })
        })
        .collect();

    Ok(json!({
        "query": query,
        "mode": mode,
        "count": results.len(),
        "results": results,
    }))
}

/// **get** — retrieve the raw Markdown content of a page.
pub fn tool_get(state: &McpState, page: &str) -> Result<Value, ToolError> {
    let resolved = resolve_page(page, &state.page_names)?;

    // Find the file path from the file index (Vec<(page_name, PathBuf)>).
    let rel_path = state
        .file_index
        .iter()
        .find(|(name, _)| name == &resolved)
        .map(|(_, p)| p.clone())
        .ok_or_else(|| ToolError::Internal(format!("page '{resolved}' not in file index")))?;

    let abs_path = state.vault_root.join(&rel_path);
    let content = std::fs::read_to_string(&abs_path)
        .map_err(|e| ToolError::Internal(format!("reading '{}': {e}", abs_path.display())))?;

    let size_bytes = content.len();

    Ok(json!({
        "page": resolved,
        "path": rel_path.to_string_lossy(),
        "content": content,
        "size_bytes": size_bytes,
    }))
}

/// **links** — forward links (outgoing wikilinks) from a page.
pub fn tool_links(state: &McpState, page: &str) -> Result<Value, ToolError> {
    let resolved = resolve_page(page, &state.page_names)?;
    let links = state.graph.forward_links(&resolved);

    let link_list: Vec<Value> = links
        .iter()
        .map(|fl| {
            let exists = state.graph.resolved.contains(&fl.target);
            json!({
                "target": fl.target,
                "line": fl.meta.line,
                "exists": exists,
                "alias": fl.meta.alias,
                "heading": fl.meta.heading,
                "block_ref": fl.meta.block_ref,
                "is_embed": fl.meta.is_embed,
            })
        })
        .collect();

    Ok(json!({
        "page": resolved,
        "count": link_list.len(),
        "links": link_list,
    }))
}

/// **backlinks** — incoming wikilinks pointing to a page.
pub fn tool_backlinks(state: &McpState, page: &str) -> Result<Value, ToolError> {
    let resolved = resolve_page(page, &state.page_names)?;
    let backlinks = state.graph.backlinks(&resolved);

    let bl_list: Vec<Value> = backlinks
        .iter()
        .map(|bl| {
            json!({
                "source": bl.source,
                "line": bl.line,
                "alias": bl.alias,
                "is_embed": bl.is_embed,
                "context": bl.context,
            })
        })
        .collect();

    Ok(json!({
        "page": resolved,
        "count": bl_list.len(),
        "backlinks": bl_list,
    }))
}

/// **path** — shortest hop path between two pages via BFS.
pub fn tool_path(
    state: &McpState,
    from: &str,
    to: &str,
    max_depth: usize,
) -> Result<Value, ToolError> {
    let from_resolved = resolve_page(from, &state.page_names)?;
    let to_resolved = resolve_page(to, &state.page_names)?;

    match state
        .graph
        .shortest_path(&from_resolved, &to_resolved, max_depth)
    {
        Some(pr) => Ok(json!({
            "from": pr.from,
            "to": pr.to,
            "hops": pr.hops,
            "path": pr.path,
            "reachable": true,
        })),
        None => Ok(json!({
            "from": from_resolved,
            "to": to_resolved,
            "reachable": false,
        })),
    }
}

/// **similar** — fuzzy page-name similarity via SimHash + Hamming distance.
pub fn tool_similar(
    state: &McpState,
    query: &str,
    threshold: u32,
    limit: usize,
) -> Result<Value, ToolError> {
    validate_search_params(query)?;
    let limit = clamp_limit(limit, 100);

    // Use cached SimHashIndex from state (built once at startup).
    let results = state.simhash.search(query, threshold, limit);

    let result_list: Vec<Value> = results
        .iter()
        .map(|r| {
            json!({
                "page": r.page,
                "distance": r.distance,
                "path": r.path,
            })
        })
        .collect();

    Ok(json!({
        "query": query,
        "threshold": threshold,
        "count": result_list.len(),
        "results": result_list,
    }))
}

/// **check** — health check: dead links, orphans, and graph stats.
///
/// `category` controls what is returned:
/// - "all" (default): full report including dead links, orphans, and stats
/// - "dead_links": only dead link information
/// - "orphans": only orphan information
pub fn tool_check(state: &McpState, category: &str) -> Result<Value, ToolError> {
    let dead_links = state.graph.dead_links();
    let orphans = state.graph.orphans();

    let dead_list: Vec<Value> = dead_links
        .iter()
        .map(|dl| {
            json!({
                "source": dl.source,
                "line": dl.line,
                "target": dl.target,
            })
        })
        .collect();

    let orphan_list: Vec<Value> = orphans
        .iter()
        .map(|o| {
            json!({
                "page": o.page,
                "forward_links": o.forward_links,
            })
        })
        .collect();

    match category {
        "dead_links" => Ok(json!({
            "dead_links": dead_list,
            "dead_link_count": dead_list.len(),
        })),
        "orphans" => Ok(json!({
            "orphans": orphan_list,
            "orphan_count": orphan_list.len(),
        })),
        _ => {
            // "all" or anything else
            let stats = state.graph.stats(10);
            Ok(json!({
                "dead_links": dead_list,
                "dead_link_count": dead_list.len(),
                "orphans": orphan_list,
                "orphan_count": orphan_list.len(),
                "stats": {
                    "pages": stats.pages,
                    "links": stats.links,
                    "unique_targets": stats.unique_targets,
                    "dead_links": stats.dead_links,
                    "orphans": stats.orphans,
                    "ambiguous_links": stats.ambiguous_links,
                    "connected_components": stats.connected_components,
                },
            }))
        }
    }
}

/// **status** — server status and vault summary.
pub fn tool_status(state: &McpState) -> Result<Value, ToolError> {
    let stats = state.graph.stats(5);
    let uptime_secs = state.started_at.elapsed().as_secs();

    let most_linked: Vec<Value> = stats
        .most_linked
        .iter()
        .map(|ml| {
            json!({
                "page": ml.page,
                "backlink_count": ml.backlink_count,
            })
        })
        .collect();

    // List of available tool names (conditionally including "reason").
    #[allow(unused_mut)]
    let mut tool_names = vec![
        "get", "search", "links", "backlinks", "path", "similar", "check", "status",
    ];
    #[cfg(feature = "reason")]
    tool_names.push("reason");

    Ok(json!({
        "vault_root": state.vault_root.to_string_lossy(),
        "page_count": stats.pages,
        "link_count": stats.links,
        "uptime_secs": uptime_secs,
        "dead_links": stats.dead_links,
        "orphans": stats.orphans,
        "most_linked": most_linked,
        "tools": tool_names,
        "transport": state.transport.as_str(),
        "reindex": "not yet supported",
    }))
}

// ---------------------------------------------------------------------------
// Reason tool — feature-gated
// ---------------------------------------------------------------------------

/// **reason** — defeasible reasoning over SPL blocks extracted from vault pages.
///
/// Only available when built with `--features reason`.
#[cfg(feature = "reason")]
pub fn tool_reason(
    state: &McpState,
    query: &str,
    files: &[String],
) -> Result<Value, ToolError> {
    use crate::types::SplBlock;
    use std::path::PathBuf;

    // Collect SPL blocks from specified files (or all files if none specified).
    let mut spl_blocks: Vec<SplBlock> = Vec::new();

    if files.is_empty() {
        // Collect from all pages in the file index by reading their content.
        for (page_name, rel_path) in state.file_index.iter() {
            let abs_path = state.vault_root.join(rel_path);
            let content = match std::fs::read_to_string(&abs_path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            extract_spl_blocks_from_content(page_name, rel_path, &content, &mut spl_blocks);
        }
    } else {
        // Collect from the specified files only.
        for file_name in files {
            let resolved = match resolve_page(file_name, &state.page_names) {
                Ok(r) => r,
                Err(_) => continue,
            };
            let rel_path = match state
                .file_index
                .iter()
                .find(|(name, _)| name == &resolved)
                .map(|(_, p)| p.clone())
            {
                Some(p) => p,
                None => continue,
            };
            let abs_path = state.vault_root.join(&rel_path);
            let content = match std::fs::read_to_string(&abs_path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            extract_spl_blocks_from_content(&resolved, &rel_path, &content, &mut spl_blocks);
        }
    }

    // If a query string is provided, inject it as an extra SPL block.
    if !query.trim().is_empty() {
        spl_blocks.push(SplBlock {
            source_file: PathBuf::from("<mcp-query>"),
            source_page: "<mcp-query>".to_string(),
            start_line: 0,
            end_line: 0,
            content: query.to_string(),
        });
    }

    let result = crate::reason::build_theory(&spl_blocks)
        .map_err(|e| ToolError::Internal(format!("reasoning error: {e}")))?;

    Ok(json!({
        "conclusions": result.conclusions,
        "diagnostics": result.diagnostics,
        "summary": result.summary,
    }))
}

/// Extract SPL blocks (``` ```spl ... ``` ```) from Markdown content.
#[cfg(feature = "reason")]
fn extract_spl_blocks_from_content(
    page_name: &str,
    rel_path: &std::path::Path,
    content: &str,
    out: &mut Vec<crate::types::SplBlock>,
) {
    use std::path::PathBuf;

    let mut in_spl = false;
    let mut block_start = 0u32;
    let mut block_lines: Vec<&str> = Vec::new();

    for (i, line) in content.lines().enumerate() {
        let lineno = (i + 1) as u32;
        let trimmed = line.trim();
        if !in_spl && (trimmed == "```spl" || trimmed == "```spindle") {
            in_spl = true;
            block_start = lineno;
            block_lines.clear();
        } else if in_spl && trimmed == "```" {
            out.push(crate::types::SplBlock {
                source_file: PathBuf::from(rel_path),
                source_page: page_name.to_string(),
                start_line: block_start,
                end_line: lineno,
                content: block_lines.join("\n"),
            });
            in_spl = false;
        } else if in_spl {
            block_lines.push(line);
        }
    }
}

/// Stub when the `reason` feature is not enabled.
#[cfg(not(feature = "reason"))]
pub fn tool_reason(
    _state: &McpState,
    _query: &str,
    _files: &[String],
) -> Result<Value, ToolError> {
    Err(ToolError::FeatureUnavailable(
        "reasoning engine requires building with --features reason".into(),
    ))
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_empty_query_rejected() {
        let err = validate_search_params("").unwrap_err();
        assert!(matches!(err, ToolError::InvalidParam(_)));
        let err = validate_search_params("   ").unwrap_err();
        assert!(matches!(err, ToolError::InvalidParam(_)));
    }

    #[test]
    fn search_non_empty_query_accepted() {
        assert!(validate_search_params("rust").is_ok());
        assert!(validate_search_params("  hello  ").is_ok());
    }

    #[test]
    fn clamp_limit_works() {
        // Normal case: within [1, max].
        assert_eq!(clamp_limit(10, 100), 10);
        // Over max → clamped to max.
        assert_eq!(clamp_limit(200, 100), 100);
        // Zero → clamped to 1.
        assert_eq!(clamp_limit(0, 100), 1);
        // Exactly at max.
        assert_eq!(clamp_limit(100, 100), 100);
        // Exactly 1.
        assert_eq!(clamp_limit(1, 100), 1);
    }
}
