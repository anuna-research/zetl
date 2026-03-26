//! Pure, effect-free functions for semantic search (SPEC-018).
//!
//! No I/O, no ONNX, no shared state. All functions are deterministic.
//! This module is compiled only when `--features semantic` is active.

/// A chunk of page content to be embedded.
///
/// Produced by `chunk_page`, consumed by the ONNX embedding shell (`mod.rs`).
/// REQ-100, ADR-054.
#[derive(Debug, Clone)]
pub struct Chunk {
    pub page_name: String,
    pub path: String,
    /// The h2 heading this chunk falls under, or `None` for intro/whole-page chunks.
    pub heading: Option<String>,
    /// BLAKE3 hash of `text`, used for incremental rebuild (REQ-097).
    pub content_hash: [u8; 32],
    pub text: String,
}

/// A hit from vector similarity search.
#[derive(Debug, Clone)]
pub struct VectorHit {
    pub page_name: String,
    pub path: String,
    pub heading: Option<String>,
    /// Cosine similarity score ∈ [0.0, 1.0]. REQ-099.
    pub score: f32,
}

/// Split page body text into chunks at h2 heading boundaries.
///
/// `headings` is a slice of `(byte_offset, level, text)` tuples sorted by offset,
/// as produced by `search::detect_headings`. Only level-2 headings are used as
/// split points (ADR-054).
///
/// Pages whose body text length is below `token_threshold` bytes are returned as a
/// single unchunked chunk regardless of headings.
///
/// REQ-100, ADR-054.
pub fn chunk_page(
    page_name: &str,
    path: &str,
    content: &str,
    headings: &[(usize, u8, String)],
    token_threshold: usize,
) -> Vec<Chunk> {
    let hash_bytes = |text: &str| *blake3::hash(text.as_bytes()).as_bytes();

    // Short pages → single chunk.
    if content.len() < token_threshold {
        return vec![Chunk {
            page_name: page_name.to_string(),
            path: path.to_string(),
            heading: None,
            content_hash: hash_bytes(content),
            text: content.to_string(),
        }];
    }

    // Collect h2 split points (byte_offset, heading_text).
    let splits: Vec<(usize, String)> = headings
        .iter()
        .filter(|(_, level, _)| *level == 2)
        .map(|(off, _, text)| (*off, text.clone()))
        .collect();

    if splits.is_empty() {
        return vec![Chunk {
            page_name: page_name.to_string(),
            path: path.to_string(),
            heading: None,
            content_hash: hash_bytes(content),
            text: content.to_string(),
        }];
    }

    let mut chunks = Vec::new();

    // Intro chunk: content before the first h2.
    let first_off = splits[0].0;
    if first_off > 0 {
        let intro = &content[..first_off];
        chunks.push(Chunk {
            page_name: page_name.to_string(),
            path: path.to_string(),
            heading: None,
            content_hash: hash_bytes(intro),
            text: intro.to_string(),
        });
    }

    // Per-section chunks.
    for (i, (start, heading_text)) in splits.iter().enumerate() {
        let end = splits.get(i + 1).map_or(content.len(), |(off, _)| *off);
        let text = &content[*start..end];
        chunks.push(Chunk {
            page_name: page_name.to_string(),
            path: path.to_string(),
            heading: Some(heading_text.clone()),
            content_hash: hash_bytes(text),
            text: text.to_string(),
        });
    }

    chunks
}

/// Cosine similarity between two vectors.
///
/// Both vectors must be normalised unit vectors for the result to lie in [0.0, 1.0].
/// REQ-099.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// Brute-force vector search: score all embeddings against a query, return top-N by cosine
/// similarity sorted descending.
///
/// Returns `(score, index)` pairs where `index` is the position in `embeddings`.
/// The result length is `min(limit, embeddings.len())`. TEST-119.
pub fn vector_search(
    embeddings: &[impl AsRef<[f32]>],
    query: &[f32],
    limit: usize,
) -> Vec<(f32, usize)> {
    let mut scored: Vec<(f32, usize)> = embeddings
        .iter()
        .enumerate()
        .map(|(i, e)| (cosine_similarity(e.as_ref(), query), i))
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(limit);
    scored
}

/// Reciprocal rank fusion of two ranked result lists.
///
/// `list_a` and `list_b` are `(page_name, rank)` pairs (1-indexed).
/// Returns all pages that appear in either list, sorted by descending fused score.
///
/// Score formula: `score(d) = Σ 1 / (k + rank_i(d))`, summed over all lists
/// in which `d` appears. ADR-053 sets `k = 60`.
///
/// REQ-095, ADR-053.
pub fn reciprocal_rank_fusion(
    list_a: &[(String, usize)],
    list_b: &[(String, usize)],
    k: usize,
) -> Vec<(String, f64)> {
    use std::collections::HashMap;
    let mut scores: HashMap<String, f64> = HashMap::new();
    for (page, rank) in list_a.iter().chain(list_b.iter()) {
        *scores.entry(page.clone()).or_default() += 1.0 / (k + rank) as f64;
    }
    let mut result: Vec<(String, f64)> = scores.into_iter().collect();
    result.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    result
}

/// Identify indices in `new_hashes` whose content has changed relative to `old_hashes`.
///
/// An index is considered stale when:
/// - it is absent in `old_hashes` (new chunk), or
/// - the hash at that position differs.
///
/// REQ-097.
pub fn detect_stale_chunks(old_hashes: &[[u8; 32]], new_hashes: &[[u8; 32]]) -> Vec<usize> {
    new_hashes
        .iter()
        .enumerate()
        .filter(|(i, hash)| old_hashes.get(*i).map_or(true, |old| old != *hash))
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // TEST-120: short pages are returned as a single chunk.
    #[test]
    fn test_chunk_page_short_is_single_chunk() {
        let content = "Short.";
        let chunks = chunk_page("p", "p.md", content, &[], 512 * 4);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, content);
        assert_eq!(chunks[0].heading, None);
    }

    // TEST-120: long pages with h2 headings are split at those boundaries.
    #[test]
    fn test_chunk_page_splits_at_h2() {
        // Use threshold = 1 so any content triggers splitting.
        let content = "Intro.\n## A\nContent A.\n## B\nContent B.\n";
        let headings = vec![
            (7usize, 2u8, "A".to_string()),
            (22usize, 2u8, "B".to_string()),
        ];
        let chunks = chunk_page("p", "p.md", content, &headings, 1);
        assert!(
            chunks.len() >= 2,
            "expected at least 2 chunks, got {}",
            chunks.len()
        );
        assert!(chunks.iter().any(|c| c.heading.as_deref() == Some("A")));
        assert!(chunks.iter().any(|c| c.heading.as_deref() == Some("B")));
    }

    // TEST-120: h1/h3 headings are ignored; only h2 is used as split point.
    #[test]
    fn test_chunk_page_ignores_non_h2() {
        let content = "Intro.\n# H1\n### H3\n";
        let headings = vec![
            (7usize, 1u8, "H1".to_string()),
            (12usize, 3u8, "H3".to_string()),
        ];
        let chunks = chunk_page("p", "p.md", content, &headings, 1);
        // No h2 → single chunk.
        assert_eq!(chunks.len(), 1);
    }

    // TEST-121: RRF produces correct ordering; all items from both lists appear.
    #[test]
    fn test_rrf_all_items_present() {
        let a = vec![("p1".to_string(), 1), ("p2".to_string(), 2)];
        let b = vec![("p2".to_string(), 1), ("p3".to_string(), 2)];
        let fused = reciprocal_rank_fusion(&a, &b, 60);
        let pages: Vec<&str> = fused.iter().map(|(p, _)| p.as_str()).collect();
        assert!(pages.contains(&"p1"));
        assert!(pages.contains(&"p2"));
        assert!(pages.contains(&"p3"));
        // p2 appears in both lists → highest score.
        assert_eq!(pages[0], "p2");
    }

    // TEST-121: RRF with empty list b equals single-list ranking.
    #[test]
    fn test_rrf_one_empty_list() {
        let a = vec![("p1".to_string(), 1), ("p2".to_string(), 2)];
        let fused = reciprocal_rank_fusion(&a, &[], 60);
        assert_eq!(fused.len(), 2);
        assert_eq!(fused[0].0, "p1");
    }

    // TEST-119: cosine similarity of identical unit vectors = 1.0.
    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![1.0f32, 0.0, 0.0];
        assert!((cosine_similarity(&a, &a) - 1.0).abs() < 1e-6);
    }

    // TEST-119: cosine similarity of orthogonal unit vectors = 0.0.
    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0f32, 0.0, 0.0];
        let b = vec![0.0f32, 1.0, 0.0];
        assert!(cosine_similarity(&a, &b).abs() < 1e-6);
    }

    // TEST-119: vector_search never returns more than `limit` results.
    #[test]
    fn test_vector_search_respects_limit() {
        // 10 unit embeddings in 3-D.
        let embeddings: Vec<Vec<f32>> = (0..10)
            .map(|i| {
                let v = i as f32;
                vec![v, 0.0, 0.0]
            })
            .collect();
        let query = vec![1.0f32, 0.0, 0.0];
        for limit in [0, 1, 5, 10, 20] {
            let results = vector_search(&embeddings, &query, limit);
            assert!(
                results.len() <= limit,
                "limit={limit}: got {} results",
                results.len()
            );
            assert!(
                results.len() <= embeddings.len(),
                "returned more results than embeddings"
            );
        }
    }

    // TEST-119: vector_search returns results sorted by descending score.
    #[test]
    fn test_vector_search_sorted_descending() {
        let embeddings: Vec<Vec<f32>> = vec![
            vec![0.0f32, 1.0, 0.0], // score against [1,0,0] = 0.0
            vec![1.0f32, 0.0, 0.0], // score = 1.0
            vec![0.6f32, 0.8, 0.0], // score = 0.6
        ];
        let query = vec![1.0f32, 0.0, 0.0];
        let results = vector_search(&embeddings, &query, 10);
        assert_eq!(results.len(), 3);
        // Scores must be non-increasing.
        for w in results.windows(2) {
            assert!(
                w[0].0 >= w[1].0,
                "scores not sorted: {} > {}",
                w[0].0,
                w[1].0
            );
        }
        // Best match is index 1 (score 1.0).
        assert_eq!(results[0].1, 1);
    }

    // TEST-119: vector_search on empty embeddings returns empty.
    #[test]
    fn test_vector_search_empty_embeddings() {
        let embeddings: Vec<Vec<f32>> = vec![];
        let query = vec![1.0f32, 0.0, 0.0];
        let results = vector_search(&embeddings, &query, 5);
        assert!(results.is_empty());
    }

    // TEST-119: vector_search with limit=0 returns empty.
    #[test]
    fn test_vector_search_limit_zero() {
        let embeddings: Vec<Vec<f32>> = vec![vec![1.0f32, 0.0], vec![0.0f32, 1.0]];
        let query = vec![1.0f32, 0.0];
        let results = vector_search(&embeddings, &query, 0);
        assert!(results.is_empty());
    }

    #[test]
    fn test_detect_stale_chunks_changed_and_new() {
        let old: Vec<[u8; 32]> = vec![[0u8; 32], [1u8; 32]];
        let new: Vec<[u8; 32]> = vec![[0u8; 32], [2u8; 32], [3u8; 32]];
        let stale = detect_stale_chunks(&old, &new);
        assert!(!stale.contains(&0)); // unchanged
        assert!(stale.contains(&1)); // changed
        assert!(stale.contains(&2)); // new
    }

    #[test]
    fn test_detect_stale_chunks_all_same() {
        let hashes: Vec<[u8; 32]> = vec![[42u8; 32]; 3];
        assert!(detect_stale_chunks(&hashes, &hashes).is_empty());
    }
}
