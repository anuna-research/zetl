// Multi-file result comparison: load JSONL files, aggregate by mode, print table

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::io::BufRead;
use std::path::Path;

use crate::modes::QuestionResult;

/// Per-mode aggregate stats for the comparison table.
struct ModeStats {
    count: usize,
    recall_5: f64,
    recall_10: f64,
    ndcg_5: f64,
    ndcg_10: f64,
    avg_ms: f64,
    cost_per_q: f64,
}

/// Load a JSONL file, returning all successfully parsed QuestionResult lines.
fn load_jsonl(path: &Path) -> Result<Vec<QuestionResult>> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    let reader = std::io::BufReader::new(file);
    let mut results = Vec::new();

    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Skip summary lines (they have "_type":"summary")
        if trimmed.contains("\"_type\"") {
            continue;
        }
        match serde_json::from_str::<QuestionResult>(trimmed) {
            Ok(qr) => results.push(qr),
            Err(_) => {
                // Skip lines that don't parse as QuestionResult
                continue;
            }
        }
    }

    Ok(results)
}

/// Aggregate a slice of QuestionResult into ModeStats.
fn aggregate(results: &[QuestionResult]) -> ModeStats {
    let n = results.len() as f64;
    if n == 0.0 {
        return ModeStats {
            count: 0,
            recall_5: 0.0,
            recall_10: 0.0,
            ndcg_5: 0.0,
            ndcg_10: 0.0,
            avg_ms: 0.0,
            cost_per_q: 0.0,
        };
    }

    let recall_5 = results.iter().map(|r| r.metrics.recall_any_5).sum::<f64>() / n;
    let recall_10 = results.iter().map(|r| r.metrics.recall_any_10).sum::<f64>() / n;
    let ndcg_5 = results.iter().map(|r| r.metrics.ndcg_5).sum::<f64>() / n;
    let ndcg_10 = results.iter().map(|r| r.metrics.ndcg_10).sum::<f64>() / n;
    let avg_ms = results.iter().map(|r| r.latency_ms as f64).sum::<f64>() / n;
    let total_cost: f64 = results
        .iter()
        .filter_map(|r| r.llm_stats.as_ref())
        .map(|s| s.cost_usd)
        .sum();

    ModeStats {
        count: results.len(),
        recall_5,
        recall_10,
        ndcg_5,
        ndcg_10,
        avg_ms,
        cost_per_q: total_cost / n,
    }
}

/// Load JSONL files, group by mode, and print a side-by-side comparison table.
pub fn run(files: &[impl AsRef<Path>]) -> Result<()> {
    // Collect all results from all files
    let mut all_results: Vec<QuestionResult> = Vec::new();
    for f in files {
        let path = f.as_ref();
        let results = load_jsonl(path)?;
        eprintln!("[compare] loaded {} results from {}", results.len(), path.display());
        all_results.extend(results);
    }

    if all_results.is_empty() {
        eprintln!("[compare] no results found");
        return Ok(());
    }

    // Group by mode (BTreeMap for sorted output)
    let mut by_mode: BTreeMap<String, Vec<QuestionResult>> = BTreeMap::new();
    for r in all_results {
        by_mode.entry(r.mode.clone()).or_default().push(r);
    }

    // Print header
    println!(
        "{:<12} {:>6} {:>6} {:>7} {:>7} {:>8} {:>8} {:>5}",
        "Mode", "R@5", "R@10", "NDCG@5", "NDCG@10", "Avg ms", "Cost/q", "N"
    );
    println!("{}", "\u{2500}".repeat(67));

    // Print each mode
    for (mode, results) in &by_mode {
        let stats = aggregate(results);
        println!(
            "{:<12} {:>6.3} {:>6.3} {:>7.3} {:>7.3} {:>7.0}ms ${:<7.4} {:>5}",
            mode,
            stats.recall_5,
            stats.recall_10,
            stats.ndcg_5,
            stats.ndcg_10,
            stats.avg_ms,
            stats.cost_per_q,
            stats.count,
        );
    }

    // Per-type breakdown across all modes
    if by_mode.len() > 1 {
        println!();
        println!("Per-type breakdown:");
        for (mode, results) in &by_mode {
            let type_data: Vec<(String, f64, f64, f64, f64)> = results
                .iter()
                .map(|r| {
                    (
                        r.question_type.clone(),
                        r.metrics.recall_any_5,
                        r.metrics.recall_any_10,
                        r.metrics.ndcg_5,
                        r.metrics.ndcg_10,
                    )
                })
                .collect();
            let agg = crate::scoring::aggregate_by_type(&type_data);
            for tm in &agg {
                println!(
                    "  {:<12} {:<20} n={:<4} R@5={:.3} R@10={:.3} NDCG@5={:.3} NDCG@10={:.3}",
                    mode, tm.question_type, tm.count,
                    tm.recall_any_5, tm.recall_any_10, tm.ndcg_5, tm.ndcg_10,
                );
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modes::{Metrics, QuestionResult, RetrievalResult};
    use std::io::Write;

    fn make_result(mode: &str, qtype: &str, r5: f64, r10: f64, n5: f64, n10: f64, ms: u64) -> QuestionResult {
        QuestionResult {
            question_id: format!("q_{mode}_{qtype}"),
            question_type: qtype.into(),
            question: "test question".into(),
            mode: mode.into(),
            ground_truth_sessions: vec!["s1".into()],
            retrieved: vec![RetrievalResult {
                session_id: "s1".into(),
                score: 1.0,
                rank: 0,
            }],
            metrics: Metrics {
                recall_any_5: r5,
                recall_any_10: r10,
                ndcg_5: n5,
                ndcg_10: n10,
            },
            latency_ms: ms,
            llm_stats: None,
        }
    }

    fn write_jsonl_file(path: &std::path::Path, results: &[QuestionResult]) {
        let mut f = std::fs::File::create(path).unwrap();
        for r in results {
            serde_json::to_writer(&mut f, r).unwrap();
            writeln!(f).unwrap();
        }
    }

    #[test]
    fn test_compare_two_files() {
        let dir = tempfile::tempdir().unwrap();

        let file1 = dir.path().join("bm25.jsonl");
        write_jsonl_file(&file1, &[
            make_result("bm25", "single", 0.8, 0.9, 0.7, 0.8, 3),
            make_result("bm25", "single", 0.6, 0.7, 0.6, 0.7, 4),
        ]);

        let file2 = dir.path().join("semantic.jsonl");
        write_jsonl_file(&file2, &[
            make_result("semantic", "single", 0.9, 1.0, 0.9, 0.95, 8),
            make_result("semantic", "multi", 1.0, 1.0, 1.0, 1.0, 10),
        ]);

        // Should not panic
        run(&[file1, file2]).unwrap();
    }

    #[test]
    fn test_compare_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("empty.jsonl");
        std::fs::write(&file, "").unwrap();
        run(&[file]).unwrap();
    }

    #[test]
    fn test_load_jsonl_skips_summary_lines() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("with_summary.jsonl");

        let mut f = std::fs::File::create(&file).unwrap();
        let qr = make_result("bm25", "single", 1.0, 1.0, 1.0, 1.0, 5);
        serde_json::to_writer(&mut f, &qr).unwrap();
        writeln!(f).unwrap();
        // Write a summary line
        writeln!(f, r#"{{"_type":"summary","mode":"bm25","count":1}}"#).unwrap();

        let results = load_jsonl(&file).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_aggregate_stats() {
        let results = vec![
            make_result("bm25", "single", 0.8, 0.9, 0.7, 0.8, 10),
            make_result("bm25", "single", 0.6, 0.7, 0.5, 0.6, 20),
        ];
        let stats = aggregate(&results);
        assert_eq!(stats.count, 2);
        assert!((stats.recall_5 - 0.7).abs() < 1e-6);
        assert!((stats.recall_10 - 0.8).abs() < 1e-6);
        assert!((stats.ndcg_5 - 0.6).abs() < 1e-6);
        assert!((stats.ndcg_10 - 0.7).abs() < 1e-6);
        assert!((stats.avg_ms - 15.0).abs() < 1e-6);
        assert!((stats.cost_per_q - 0.0).abs() < 1e-6);
    }
}
