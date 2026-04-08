// Mode 5: LLM rerank
//
// Retrieves a broad candidate set (top 50) via hybrid or BM25, then asks
// the LLM to re-rank the best `top_k` by relevance to the query.

use std::path::Path;

use anyhow::{Context, Result};

use zetl::search_index::SearchIndex;

use crate::llm::LlmClient;
use super::{LlmStats, RetrievalResult};

/// Run LLM-reranked retrieval for a single question.
///
/// 1. Retrieve top-50 candidates via hybrid (or BM25 fallback).
/// 2. Read each candidate's session file to build a numbered passage list.
/// 3. Ask the LLM to rank the top `top_k` by relevance.
/// 4. Parse the JSON array of session IDs from the response.
pub async fn retrieve(
    vault_dir: &Path,
    search_index: &SearchIndex,
    #[cfg(feature = "semantic")]
    vector_index: Option<&zetl::semantic::VectorIndex>,
    llm: &dyn LlmClient,
    query: &str,
    top_k: usize,
    _fusion_weight: f64,
) -> Result<(Vec<RetrievalResult>, LlmStats)> {
    let candidate_k = 20;

    // Step 1: Get broad candidate set
    #[cfg(feature = "semantic")]
    let candidates = if let Some(vi) = vector_index {
        super::hybrid::retrieve(search_index, vi, query, candidate_k, _fusion_weight)?
    } else {
        super::bm25::retrieve(search_index, query, candidate_k)?
    };

    #[cfg(not(feature = "semantic"))]
    let candidates = super::bm25::retrieve(search_index, query, candidate_k)?;

    if candidates.is_empty() {
        return Ok((Vec::new(), LlmStats {
            tool_calls: 0,
            tokens_in: 0,
            tokens_out: 0,
            cost_usd: 0.0,
        }));
    }

    // Step 2: Build numbered passage list by reading session files
    let sessions_dir = vault_dir.join("sessions");
    let mut passages = Vec::new();
    for (i, candidate) in candidates.iter().enumerate() {
        let file_path = sessions_dir.join(format!("{}.md", candidate.session_id));
        let text = match std::fs::read_to_string(&file_path) {
            Ok(content) => {
                // Take first 300 chars of content for the prompt
                let truncated: String = content.chars().take(150).collect();
                truncated
            }
            Err(_) => format!("[session {}]", candidate.session_id),
        };
        passages.push(format!("{}. [{}] {}", i + 1, candidate.session_id, text));
    }

    let passages_text = passages.join("\n");

    // Step 3: Single LLM call to rerank
    let system_prompt = format!(
        "Given a question and candidate passages, rank the top {top_k} by relevance. \
         Return ONLY a JSON array of session IDs in order of relevance (most relevant first), \
         e.g. [\"s1\", \"s2\", ...]. No explanation, just the JSON array."
    );

    let user_prompt = format!(
        "Question: {query}\n\nCandidate passages:\n{passages_text}"
    );

    let messages = vec![
        crate::llm::Message::system(system_prompt),
        crate::llm::Message::user(user_prompt),
    ];

    let response = llm.complete(messages, None).await
        .context("LLM rerank call failed")?;

    let tokens_in = response.tokens_in;
    let tokens_out = response.tokens_out;
    let cost_usd = llm.estimate_cost(tokens_in, tokens_out);

    // Step 4: Parse the response as a JSON array of session IDs
    let content = response.content.unwrap_or_default();
    let reranked_ids = parse_session_id_array(&content);

    // Build a lookup from candidate session_id -> original score (for fallback scoring)
    let candidate_scores: std::collections::HashMap<String, f64> = candidates
        .iter()
        .map(|c| (c.session_id.clone(), c.score))
        .collect();

    // Build results from reranked IDs
    let mut results: Vec<RetrievalResult> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for (rank, sid) in reranked_ids.iter().enumerate() {
        if seen.contains(sid) {
            continue;
        }
        seen.insert(sid.clone());
        // Use a decaying score based on rank position so ordering is preserved
        let score = candidate_scores.get(sid).copied().unwrap_or(0.0);
        results.push(RetrievalResult {
            session_id: sid.clone(),
            score,
            rank,
        });
        if results.len() >= top_k {
            break;
        }
    }

    // If LLM returned fewer than top_k, fill from original candidates
    if results.len() < top_k {
        for candidate in &candidates {
            if seen.contains(&candidate.session_id) {
                continue;
            }
            seen.insert(candidate.session_id.clone());
            results.push(RetrievalResult {
                session_id: candidate.session_id.clone(),
                score: candidate.score,
                rank: results.len(),
            });
            if results.len() >= top_k {
                break;
            }
        }
    }

    let stats = LlmStats {
        tool_calls: 0,
        tokens_in,
        tokens_out,
        cost_usd,
    };

    Ok((results, stats))
}

/// Parse a JSON array of session IDs from LLM output.
///
/// Tolerant of surrounding text — extracts the first `[...]` block found.
fn parse_session_id_array(text: &str) -> Vec<String> {
    // Find the first [ ... ] in the text
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
