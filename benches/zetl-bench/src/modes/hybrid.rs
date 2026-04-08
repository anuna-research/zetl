// Mode 3: BM25 + semantic hybrid fusion

#[cfg(feature = "semantic")]
use std::collections::HashMap;

#[cfg(feature = "semantic")]
use anyhow::Result;

#[cfg(feature = "semantic")]
use zetl::search_index::SearchIndex;
#[cfg(feature = "semantic")]
use zetl::semantic::VectorIndex;

#[cfg(feature = "semantic")]
use super::RetrievalResult;

/// Run hybrid BM25 + semantic retrieval with reciprocal rank fusion.
///
/// `fusion_weight` controls interpolation: 0.0 = pure BM25, 1.0 = pure semantic.
/// Both result lists are min-max normalised to [0, 1] before fusion.
#[cfg(feature = "semantic")]
pub fn retrieve(
    search_index: &SearchIndex,
    vector_index: &VectorIndex,
    query: &str,
    top_k: usize,
    fusion_weight: f64,
) -> Result<Vec<RetrievalResult>> {
    let over_fetch = top_k * 3;

    // 1. BM25 results
    let bm25_hits = search_index.query(query, over_fetch)?;
    let bm25_scores: Vec<(String, f64)> = bm25_hits
        .into_iter()
        .map(|h| (h.page_name, h.score))
        .collect();

    // 2. Semantic results
    let sem_hits = vector_index.query_text(query, over_fetch)?;
    let sem_scores: Vec<(String, f64)> = sem_hits
        .into_iter()
        .map(|h| (h.page_name, h.score as f64))
        .collect();

    // 3. Min-max normalise each list to [0, 1]
    let bm25_norm = min_max_normalize(&bm25_scores);
    let sem_norm = min_max_normalize(&sem_scores);

    // 4. Fuse: for each unique session_id, compute weighted score
    let mut fused: HashMap<String, f64> = HashMap::new();

    for (id, score) in &bm25_norm {
        let entry = fused.entry(id.clone()).or_insert(0.0);
        *entry += (1.0 - fusion_weight) * score;
    }
    for (id, score) in &sem_norm {
        let entry = fused.entry(id.clone()).or_insert(0.0);
        *entry += fusion_weight * score;
    }

    // 5. Sort descending by fused score, take top_k
    let mut ranked: Vec<(String, f64)> = fused.into_iter().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    ranked.truncate(top_k);

    // 6. Return as RetrievalResult with ranks
    let results = ranked
        .into_iter()
        .enumerate()
        .map(|(rank, (session_id, score))| RetrievalResult {
            session_id,
            score,
            rank,
        })
        .collect();

    Ok(results)
}

/// Min-max normalise a list of (id, score) pairs to [0, 1].
///
/// If all scores are equal, every item gets 0.0 (or 1.0 if there is a single item).
#[cfg(feature = "semantic")]
fn min_max_normalize(scores: &[(String, f64)]) -> Vec<(String, f64)> {
    if scores.is_empty() {
        return Vec::new();
    }
    let min = scores.iter().map(|(_, s)| *s).fold(f64::INFINITY, f64::min);
    let max = scores
        .iter()
        .map(|(_, s)| *s)
        .fold(f64::NEG_INFINITY, f64::max);
    let range = max - min;

    scores
        .iter()
        .map(|(id, s)| {
            let norm = if range > 0.0 { (s - min) / range } else { 0.0 };
            (id.clone(), norm)
        })
        .collect()
}
