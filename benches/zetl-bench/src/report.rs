// Report utilities: auto-naming and summary JSONL output

use anyhow::Result;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::modes::QuestionResult;

/// Generate auto-named output path: results_{mode}_{timestamp}.jsonl
pub fn auto_output_path(mode: &str) -> PathBuf {
    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    PathBuf::from(format!("results_{mode}_{timestamp}.jsonl"))
}

/// Write a summary JSONL with aggregate metrics appended at the end.
///
/// Each QuestionResult is written as one JSON line, followed by a final
/// summary line containing aggregate metrics for the entire run.
pub fn write_summary(results: &[QuestionResult], path: &Path) -> Result<()> {
    let file = std::fs::File::create(path)?;
    let mut writer = std::io::BufWriter::new(file);

    for r in results {
        serde_json::to_writer(&mut writer, r)?;
        writeln!(writer)?;
    }

    // Append aggregate summary line
    if !results.is_empty() {
        let n = results.len() as f64;
        let r5: f64 = results.iter().map(|r| r.metrics.recall_any_5).sum::<f64>() / n;
        let r10: f64 = results.iter().map(|r| r.metrics.recall_any_10).sum::<f64>() / n;
        let n5: f64 = results.iter().map(|r| r.metrics.ndcg_5).sum::<f64>() / n;
        let n10: f64 = results.iter().map(|r| r.metrics.ndcg_10).sum::<f64>() / n;
        let avg_ms: f64 = results.iter().map(|r| r.latency_ms as f64).sum::<f64>() / n;
        let total_cost: f64 = results
            .iter()
            .filter_map(|r| r.llm_stats.as_ref())
            .map(|s| s.cost_usd)
            .sum();

        let mode = &results[0].mode;

        let summary = serde_json::json!({
            "_type": "summary",
            "mode": mode,
            "count": results.len(),
            "recall_any_5": r5,
            "recall_any_10": r10,
            "ndcg_5": n5,
            "ndcg_10": n10,
            "avg_latency_ms": avg_ms,
            "total_cost_usd": total_cost,
        });
        serde_json::to_writer(&mut writer, &summary)?;
        writeln!(writer)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auto_output_path_contains_mode() {
        let path = auto_output_path("bm25");
        let name = path.to_string_lossy();
        assert!(name.starts_with("results_bm25_"));
        assert!(name.ends_with(".jsonl"));
    }

    #[test]
    fn test_write_summary_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.jsonl");
        write_summary(&[], &path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.is_empty());
    }

    #[test]
    fn test_write_summary_with_results() {
        use crate::modes::{Metrics, QuestionResult};

        let results = vec![QuestionResult {
            question_id: "q1".into(),
            question_type: "single_session".into(),
            question: "test?".into(),
            mode: "bm25".into(),
            ground_truth_sessions: vec!["s1".into()],
            retrieved: vec![],
            metrics: Metrics {
                recall_any_5: 1.0,
                recall_any_10: 1.0,
                ndcg_5: 0.8,
                ndcg_10: 0.9,
            },
            latency_ms: 5,
            llm_stats: None,
        }];

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.jsonl");
        write_summary(&results, &path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        // 1 result line + 1 summary line
        assert_eq!(lines.len(), 2);
        // Summary line should contain "_type":"summary"
        assert!(lines[1].contains("\"_type\":\"summary\""));
    }
}
