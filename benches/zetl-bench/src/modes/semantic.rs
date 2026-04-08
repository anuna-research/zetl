// Mode 2: Semantic vector search

#[cfg(feature = "semantic")]
use anyhow::Result;

#[cfg(feature = "semantic")]
use super::RetrievalResult;

/// Run semantic retrieval for a single question against the vector index.
///
/// Calls `vector_index.query_text(query, top_k)` and maps each `VectorHit`
/// to a `RetrievalResult` where `session_id` is derived from the hit's
/// `page_name` (files are named `{session_id}.md`).
#[cfg(feature = "semantic")]
pub fn retrieve(
    vector_index: &zetl::semantic::VectorIndex,
    query: &str,
    top_k: usize,
) -> Result<Vec<RetrievalResult>> {
    let hits = vector_index.query_text(query, top_k)?;

    let results = hits
        .into_iter()
        .enumerate()
        .map(|(rank, hit)| RetrievalResult {
            session_id: hit.page_name,
            score: hit.score as f64,
            rank,
        })
        .collect();

    Ok(results)
}
