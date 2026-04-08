// Mode 6: Agentic retrieval
//
// Simulates agentic retrieval by giving the LLM access to search, read_page,
// links, and backlinks tools. The LLM iteratively calls tools to gather
// information, then returns a ranked list of session IDs.

use std::path::Path;

use anyhow::{Context, Result};
use serde_json::json;

use zetl::graph::LinkGraph;
use zetl::search_index::SearchIndex;

use crate::llm::{LlmClient, Message, Tool};
use super::{LlmStats, RetrievalResult};

/// Run agentic retrieval for a single question.
///
/// The LLM is given tools (search, read_page, links, backlinks) and runs
/// an agent loop until it produces a `FINAL_ANSWER:` or hits `max_tool_calls`.
pub async fn retrieve(
    vault_dir: &Path,
    search_index: &SearchIndex,
    link_graph: &LinkGraph,
    llm: &dyn LlmClient,
    query: &str,
    top_k: usize,
    max_tool_calls: usize,
) -> Result<(Vec<RetrievalResult>, LlmStats)> {
    let tools = build_tools();

    let system_prompt = format!(
        "You are a retrieval agent. Your goal is to find the {top_k} most relevant sessions \
         that answer the user's question. You have these tools:\n\
         - search(query, limit): full-text search across all sessions\n\
         - read_page(page): read the full text of a session by its ID\n\
         - links(page): get forward links from a session\n\
         - backlinks(page): get backlinks to a session\n\n\
         Strategy: Start with a search, read promising results, follow links if needed.\n\
         When you have found the best sessions, respond with FINAL_ANSWER: followed by \
         a JSON array of session IDs ranked by relevance (most relevant first), e.g.:\n\
         FINAL_ANSWER: [\"session_1\", \"session_2\", ...]"
    );

    let mut messages = vec![
        Message::system(system_prompt),
        Message::user(format!("Find sessions that answer this question: {query}")),
    ];

    let mut total_tokens_in: u64 = 0;
    let mut total_tokens_out: u64 = 0;
    let mut total_tool_calls: usize = 0;
    let mut total_cost: f64 = 0.0;

    for _iteration in 0..max_tool_calls {
        let response = llm
            .complete(messages.clone(), Some(tools.clone()))
            .await
            .context("LLM agentic call failed")?;

        total_tokens_in += response.tokens_in;
        total_tokens_out += response.tokens_out;
        total_cost += llm.estimate_cost(response.tokens_in, response.tokens_out);

        // Check if LLM returned text with FINAL_ANSWER
        if let Some(ref text) = response.content {
            if let Some(answer) = extract_final_answer(text) {
                let stats = LlmStats {
                    tool_calls: total_tool_calls,
                    tokens_in: total_tokens_in,
                    tokens_out: total_tokens_out,
                    cost_usd: total_cost,
                };
                return Ok((build_results(&answer, top_k), stats));
            }

            // If there's text but no tool calls and no final answer, add it and continue
            if response.tool_calls.is_empty() {
                // LLM stopped without a final answer — treat the text as the answer attempt
                let stats = LlmStats {
                    tool_calls: total_tool_calls,
                    tokens_in: total_tokens_in,
                    tokens_out: total_tokens_out,
                    cost_usd: total_cost,
                };
                // Try to parse any JSON array from the response
                let ids = parse_session_id_array(text);
                return Ok((build_results(&ids, top_k), stats));
            }
        }

        // No tool calls and no content — bail
        if response.tool_calls.is_empty() && response.content.is_none() {
            break;
        }

        // Add assistant message with content if present
        if let Some(ref text) = response.content {
            messages.push(Message::assistant(text.clone()));
        }

        // Execute tool calls
        for tc in &response.tool_calls {
            total_tool_calls += 1;

            let result = execute_tool(
                &tc.name,
                &tc.arguments,
                vault_dir,
                search_index,
                link_graph,
            );

            messages.push(Message::tool_result(tc.id.clone(), result));
        }
    }

    // Hit max iterations — try to get whatever we can
    let stats = LlmStats {
        tool_calls: total_tool_calls,
        tokens_in: total_tokens_in,
        tokens_out: total_tokens_out,
        cost_usd: total_cost,
    };

    Ok((Vec::new(), stats))
}

/// Build the tool definitions for the LLM.
fn build_tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "search".into(),
            description: "Full-text search across all sessions. Returns session IDs and scores.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The search query. Returns top 10 results."
                    }
                },
                "required": ["query"]
            }),
        },
        Tool {
            name: "read_page".into(),
            description: "Read the full text of a session by its ID.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "page": {
                        "type": "string",
                        "description": "The session ID (page name)"
                    }
                },
                "required": ["page"]
            }),
        },
        Tool {
            name: "links".into(),
            description: "Get forward links from a session to other sessions.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "page": {
                        "type": "string",
                        "description": "The session ID (page name)"
                    }
                },
                "required": ["page"]
            }),
        },
        Tool {
            name: "backlinks".into(),
            description: "Get backlinks — sessions that link to this session.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "page": {
                        "type": "string",
                        "description": "The session ID (page name)"
                    }
                },
                "required": ["page"]
            }),
        },
    ]
}

/// Execute a tool call and return the result as a string.
fn execute_tool(
    name: &str,
    args: &serde_json::Value,
    vault_dir: &Path,
    search_index: &SearchIndex,
    link_graph: &LinkGraph,
) -> String {
    match name {
        "search" => {
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
                .unwrap_or(10) as usize;

            match search_index.query(query, limit) {
                Ok(hits) => {
                    let results: Vec<serde_json::Value> = hits
                        .into_iter()
                        .map(|h| json!({"session_id": h.page_name, "score": h.score}))
                        .collect();
                    serde_json::to_string_pretty(&results).unwrap_or_else(|_| "[]".into())
                }
                Err(e) => format!("Error: {e}"),
            }
        }
        "read_page" => {
            let page = args
                .get("page")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let file_path = vault_dir.join("sessions").join(format!("{page}.md"));
            match std::fs::read_to_string(&file_path) {
                Ok(content) => {
                    // Truncate to avoid blowing up the context
                    let truncated: String = content.chars().take(2000).collect();
                    truncated
                }
                Err(_) => format!("Page not found: {page}"),
            }
        }
        "links" => {
            let page = args
                .get("page")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let fwd = link_graph.forward_links(page);
            let targets: Vec<&str> = fwd.iter().map(|l| l.target.as_str()).collect();
            serde_json::to_string(&targets).unwrap_or_else(|_| "[]".into())
        }
        "backlinks" => {
            let page = args
                .get("page")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let bk = link_graph.backlinks(page);
            let sources: Vec<&str> = bk.iter().map(|l| l.source.as_str()).collect();
            serde_json::to_string(&sources).unwrap_or_else(|_| "[]".into())
        }
        _ => format!("Unknown tool: {name}"),
    }
}

/// Extract session IDs from a FINAL_ANSWER: line.
fn extract_final_answer(text: &str) -> Option<Vec<String>> {
    let marker = "FINAL_ANSWER:";
    let idx = text.find(marker)?;
    let after = &text[idx + marker.len()..];
    let ids = parse_session_id_array(after);
    if ids.is_empty() {
        None
    } else {
        Some(ids)
    }
}

/// Parse a JSON array of session IDs from text. Tolerant of surrounding text.
fn parse_session_id_array(text: &str) -> Vec<String> {
    if let Some(start) = text.find('[') {
        if let Some(end) = text[start..].find(']') {
            let json_str = &text[start..start + end + 1];
            if let Ok(ids) = serde_json::from_str::<Vec<String>>(json_str) {
                return ids;
            }
        }
    }
    Vec::new()
}

/// Build RetrievalResult list from session IDs.
fn build_results(ids: &[String], top_k: usize) -> Vec<RetrievalResult> {
    ids.iter()
        .take(top_k)
        .enumerate()
        .map(|(rank, sid)| RetrievalResult {
            session_id: sid.clone(),
            // Score decays by rank so ordering is preserved in metrics
            score: 1.0 / (rank as f64 + 1.0),
            rank,
        })
        .collect()
}
