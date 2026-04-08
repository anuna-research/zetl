// Mode 4: Graph-augmented retrieval (hybrid + 1-hop link expansion)

#[cfg(feature = "semantic")]
use std::collections::HashMap;

#[cfg(feature = "semantic")]
use anyhow::Result;

#[cfg(feature = "semantic")]
use zetl::graph::LinkGraph;
#[cfg(feature = "semantic")]
use zetl::search_index::SearchIndex;
#[cfg(feature = "semantic")]
use zetl::semantic::VectorIndex;

#[cfg(feature = "semantic")]
use super::RetrievalResult;

/// Graph-augmented retrieval: hybrid fusion + 1-hop link expansion.
///
/// 1. Run hybrid retrieval for initial candidates (top_k * 2).
/// 2. For each candidate, collect forward_links + backlinks from the LinkGraph (1 hop).
/// 3. Score neighbours with decay: `neighbor_score = origin_score * 0.5`.
/// 4. Merge into candidate set, deduplicate (keep highest score), re-sort.
/// 5. Return top_k results.
#[cfg(feature = "semantic")]
pub fn retrieve(
    search_index: &SearchIndex,
    vector_index: &VectorIndex,
    link_graph: &LinkGraph,
    query: &str,
    top_k: usize,
    fusion_weight: f64,
) -> Result<Vec<RetrievalResult>> {
    const DECAY: f64 = 0.5;

    // 1. Get initial candidates via hybrid fusion
    let initial_k = top_k * 2;
    let candidates = super::hybrid::retrieve(
        search_index,
        vector_index,
        query,
        initial_k,
        fusion_weight,
    )?;

    // Build a score map from candidates
    let mut score_map: HashMap<String, f64> = HashMap::new();
    for c in &candidates {
        score_map.insert(c.session_id.clone(), c.score);
    }

    // 2-3. For each candidate, expand 1 hop via the link graph
    for candidate in &candidates {
        let origin_score = candidate.score;
        let neighbor_score = origin_score * DECAY;

        // Forward links
        for fwd in link_graph.forward_links(&candidate.session_id) {
            let entry = score_map.entry(fwd.target.clone()).or_insert(0.0);
            if neighbor_score > *entry {
                *entry = neighbor_score;
            }
        }

        // Backlinks
        for bk in link_graph.backlinks(&candidate.session_id) {
            let entry = score_map.entry(bk.source.clone()).or_insert(0.0);
            if neighbor_score > *entry {
                *entry = neighbor_score;
            }
        }
    }

    // 4. Sort by score descending, take top_k
    let mut ranked: Vec<(String, f64)> = score_map.into_iter().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    ranked.truncate(top_k);

    // 5. Return as RetrievalResult with ranks
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
