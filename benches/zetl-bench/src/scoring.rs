// Recall@k, NDCG@k scoring — ported from LongMemEval Python implementation

use std::collections::HashMap;

/// Discounted Cumulative Gain.
/// score = sum(relevances[i] / log2(i + 2)) for i in 0..k
pub fn dcg(relevances: &[f64], k: usize) -> f64 {
    relevances
        .iter()
        .take(k)
        .enumerate()
        .map(|(i, &rel)| rel / ((i as f64 + 2.0).log2()))
        .sum()
}

/// Normalized DCG.
/// Build relevance vector: 1.0 if corpus_ids[rankings[i]] is in ground_truth, else 0.0.
/// Ideal = sorted relevances descending.
/// NDCG = DCG(actual) / DCG(ideal), or 0.0 if ideal DCG is 0.
pub fn ndcg(
    rankings: &[usize],
    ground_truth: &[String],
    corpus_ids: &[String],
    k: usize,
) -> f64 {
    let relevances: Vec<f64> = rankings
        .iter()
        .take(k)
        .map(|&idx| {
            if idx < corpus_ids.len() && ground_truth.contains(&corpus_ids[idx]) {
                1.0
            } else {
                0.0
            }
        })
        .collect();

    let actual = dcg(&relevances, k);

    let mut ideal_rels = relevances.clone();
    ideal_rels.sort_by(|a, b| b.partial_cmp(a).unwrap());
    let ideal = dcg(&ideal_rels, k);

    if ideal == 0.0 {
        0.0
    } else {
        actual / ideal
    }
}

/// Recall@k (any): at least one ground truth ID is in the top-k retrieved IDs.
/// Returns 1.0 or 0.0.
pub fn recall_any_at_k(
    rankings: &[usize],
    ground_truth: &[String],
    corpus_ids: &[String],
    k: usize,
) -> f64 {
    let top_k_ids: Vec<&String> = rankings
        .iter()
        .take(k)
        .filter_map(|&idx| corpus_ids.get(idx))
        .collect();

    if ground_truth.iter().any(|gt| top_k_ids.contains(&gt)) {
        1.0
    } else {
        0.0
    }
}

/// Recall@k (all): ALL ground truth IDs are in the top-k retrieved IDs.
/// Returns 1.0 or 0.0.
pub fn recall_all_at_k(
    rankings: &[usize],
    ground_truth: &[String],
    corpus_ids: &[String],
    k: usize,
) -> f64 {
    let top_k_ids: Vec<&String> = rankings
        .iter()
        .take(k)
        .filter_map(|&idx| corpus_ids.get(idx))
        .collect();

    if ground_truth.iter().all(|gt| top_k_ids.contains(&gt)) {
        1.0
    } else {
        0.0
    }
}

/// Extract session ID from a corpus ID.
/// Turn IDs look like "sess_123_turn_4" -> session part is "sess_123".
/// Session IDs are returned as-is.
pub fn session_id_from_corpus_id(corpus_id: &str) -> &str {
    // Look for "_turn_" and return everything before it
    if let Some(pos) = corpus_id.find("_turn_") {
        &corpus_id[..pos]
    } else {
        corpus_id
    }
}

/// Full evaluation: returns (recall_any, recall_all, ndcg) at given k.
pub fn evaluate_retrieval(
    rankings: &[usize],
    ground_truth: &[String],
    corpus_ids: &[String],
    k: usize,
) -> (f64, f64, f64) {
    let r_any = recall_any_at_k(rankings, ground_truth, corpus_ids, k);
    let r_all = recall_all_at_k(rankings, ground_truth, corpus_ids, k);
    let n = ndcg(rankings, ground_truth, corpus_ids, k);
    (r_any, r_all, n)
}

/// Per-question-type aggregation.
#[derive(Debug, Clone)]
pub struct TypeMetrics {
    pub question_type: String,
    pub count: usize,
    pub recall_any_5: f64,
    pub recall_any_10: f64,
    pub ndcg_5: f64,
    pub ndcg_10: f64,
}

/// Aggregate results by question type, computing mean for each metric.
/// results: Vec of (question_type, recall_any@5, recall_any@10, ndcg@5, ndcg@10)
pub fn aggregate_by_type(
    results: &[(String, f64, f64, f64, f64)],
) -> Vec<TypeMetrics> {
    let mut groups: HashMap<String, Vec<(f64, f64, f64, f64)>> = HashMap::new();

    for (qt, r5, r10, n5, n10) in results {
        groups
            .entry(qt.clone())
            .or_default()
            .push((*r5, *r10, *n5, *n10));
    }

    let mut metrics: Vec<TypeMetrics> = groups
        .into_iter()
        .map(|(qt, vals)| {
            let count = vals.len();
            let n = count as f64;
            TypeMetrics {
                question_type: qt,
                count,
                recall_any_5: vals.iter().map(|v| v.0).sum::<f64>() / n,
                recall_any_10: vals.iter().map(|v| v.1).sum::<f64>() / n,
                ndcg_5: vals.iter().map(|v| v.2).sum::<f64>() / n,
                ndcg_10: vals.iter().map(|v| v.3).sum::<f64>() / n,
            }
        })
        .collect();

    metrics.sort_by(|a, b| a.question_type.cmp(&b.question_type));
    metrics
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-4;

    #[test]
    fn test_dcg_basic() {
        // dcg([1.0, 0.0, 1.0], 3) = 1.0/log2(2) + 0.0/log2(3) + 1.0/log2(4) = 1.0 + 0.0 + 0.5 = 1.5
        let result = dcg(&[1.0, 0.0, 1.0], 3);
        assert!((result - 1.5).abs() < EPS, "expected 1.5, got {result}");
    }

    #[test]
    fn test_dcg_two_relevant() {
        // dcg([1.0, 1.0], 2) = 1.0/log2(2) + 1.0/log2(3) ≈ 1.0 + 0.6309 ≈ 1.6309
        let result = dcg(&[1.0, 1.0], 2);
        let expected = 1.0 + 1.0 / 3.0_f64.log2();
        assert!((result - expected).abs() < EPS, "expected {expected}, got {result}");
    }

    #[test]
    fn test_ndcg_perfect_ranking() {
        // Ground truth at rank 0 (first position) -> NDCG = 1.0
        let corpus_ids: Vec<String> = vec!["a", "b", "c"].into_iter().map(String::from).collect();
        let ground_truth = vec!["a".to_string()];
        let rankings = vec![0, 1, 2];
        let result = ndcg(&rankings, &ground_truth, &corpus_ids, 3);
        assert!((result - 1.0).abs() < EPS, "expected 1.0, got {result}");
    }

    #[test]
    fn test_ndcg_imperfect_ranking() {
        // Ground truth at rank 1 (second position) out of 2 -> NDCG < 1.0
        let corpus_ids: Vec<String> = vec!["a", "b"].into_iter().map(String::from).collect();
        let ground_truth = vec!["b".to_string()];
        let rankings = vec![0, 1];
        let result = ndcg(&rankings, &ground_truth, &corpus_ids, 2);
        assert!(result < 1.0, "expected < 1.0, got {result}");
        assert!(result > 0.0, "expected > 0.0, got {result}");
    }

    #[test]
    fn test_recall_any_hit() {
        // Ground truth "c" at index 2, rankings place it at rank 2 (within top 5)
        let corpus_ids: Vec<String> = (0..10).map(|i| format!("doc_{i}")).collect();
        let ground_truth = vec!["doc_2".to_string()];
        let rankings: Vec<usize> = (0..10).collect();
        let result = recall_any_at_k(&rankings, &ground_truth, &corpus_ids, 5);
        assert!((result - 1.0).abs() < EPS, "expected 1.0, got {result}");
    }

    #[test]
    fn test_recall_any_miss() {
        // Ground truth "doc_7" at rank 7, k=5 -> miss
        let corpus_ids: Vec<String> = (0..10).map(|i| format!("doc_{i}")).collect();
        let ground_truth = vec!["doc_7".to_string()];
        let rankings: Vec<usize> = (0..10).collect();
        let result = recall_any_at_k(&rankings, &ground_truth, &corpus_ids, 5);
        assert!((result - 0.0).abs() < EPS, "expected 0.0, got {result}");
    }

    #[test]
    fn test_recall_all_hit() {
        // Both ground truths in top 5
        let corpus_ids: Vec<String> = (0..10).map(|i| format!("doc_{i}")).collect();
        let ground_truth = vec!["doc_1".to_string(), "doc_3".to_string()];
        let rankings: Vec<usize> = (0..10).collect();
        let result = recall_all_at_k(&rankings, &ground_truth, &corpus_ids, 5);
        assert!((result - 1.0).abs() < EPS, "expected 1.0, got {result}");
    }

    #[test]
    fn test_recall_all_miss() {
        // Only 1 of 2 ground truths in top 5
        let corpus_ids: Vec<String> = (0..10).map(|i| format!("doc_{i}")).collect();
        let ground_truth = vec!["doc_1".to_string(), "doc_7".to_string()];
        let rankings: Vec<usize> = (0..10).collect();
        let result = recall_all_at_k(&rankings, &ground_truth, &corpus_ids, 5);
        assert!((result - 0.0).abs() < EPS, "expected 0.0, got {result}");
    }

    #[test]
    fn test_session_id_from_turn() {
        assert_eq!(session_id_from_corpus_id("sess_123_turn_4"), "sess_123");
    }

    #[test]
    fn test_session_id_bare() {
        assert_eq!(session_id_from_corpus_id("sess_123"), "sess_123");
    }

    #[test]
    fn test_aggregate_by_type() {
        let results = vec![
            ("typeA".to_string(), 1.0, 1.0, 0.8, 0.9),
            ("typeA".to_string(), 0.0, 1.0, 0.6, 0.7),
            ("typeB".to_string(), 1.0, 0.0, 1.0, 0.5),
        ];
        let agg = aggregate_by_type(&results);
        assert_eq!(agg.len(), 2);

        let a = agg.iter().find(|m| m.question_type == "typeA").unwrap();
        assert_eq!(a.count, 2);
        assert!((a.recall_any_5 - 0.5).abs() < EPS);
        assert!((a.recall_any_10 - 1.0).abs() < EPS);
        assert!((a.ndcg_5 - 0.7).abs() < EPS);
        assert!((a.ndcg_10 - 0.8).abs() < EPS);

        let b = agg.iter().find(|m| m.question_type == "typeB").unwrap();
        assert_eq!(b.count, 1);
        assert!((b.recall_any_5 - 1.0).abs() < EPS);
        assert!((b.recall_any_10 - 0.0).abs() < EPS);
        assert!((b.ndcg_5 - 1.0).abs() < EPS);
        assert!((b.ndcg_10 - 0.5).abs() < EPS);
    }
}
