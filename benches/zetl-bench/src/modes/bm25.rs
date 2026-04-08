// Mode 1: BM25 full-text search via Tantivy

use anyhow::Result;
use zetl::search_index::SearchIndex;

use super::RetrievalResult;

/// Run BM25 retrieval for a single question against the vault.
///
/// Calls `search_index.query(query, top_k)` and maps each hit to a
/// `RetrievalResult` where `session_id` is derived from the hit's
/// `page_name` (files are named `{session_id}.md`).
/// Sanitize a query for Tantivy: remove special characters that break the parser.
fn sanitize_query(query: &str) -> String {
    query
        .chars()
        .map(|c| match c {
            ':' | '(' | ')' | '[' | ']' | '{' | '}' | '!' | '^' | '~' | '\\' | '"' => ' ',
            _ => c,
        })
        .collect()
}

pub fn retrieve(
    search_index: &SearchIndex,
    query: &str,
    top_k: usize,
) -> Result<Vec<RetrievalResult>> {
    let sanitized = sanitize_query(query);
    let hits = match search_index.query(&sanitized, top_k) {
        Ok(h) => h,
        Err(_) => {
            // Fallback: try just the first few words if full query fails
            let fallback: String = sanitized.split_whitespace().take(5).collect::<Vec<_>>().join(" ");
            search_index.query(&fallback, top_k).unwrap_or_default()
        }
    };

    let results = hits
        .into_iter()
        .enumerate()
        .map(|(rank, hit)| RetrievalResult {
            session_id: hit.page_name,
            score: hit.score,
            rank,
        })
        .collect();

    Ok(results)
}
