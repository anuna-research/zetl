// Retrieval mode orchestration

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Instant;

use zetl::scanner::scan_vault;
use zetl::search_index::SearchIndex;

use crate::dataset;
use crate::llm;
use crate::scoring;

pub mod bm25;
pub mod semantic;
pub mod hybrid;
pub mod graph;
pub mod rerank;
pub mod agentic;

// ---------------------------------------------------------------------------
// Common types shared across all modes
// ---------------------------------------------------------------------------

/// A single retrieved item in a ranked list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalResult {
    pub session_id: String,
    pub score: f64,
    pub rank: usize,
}

/// Evaluation metrics for one question.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metrics {
    pub recall_any_5: f64,
    pub recall_any_10: f64,
    pub ndcg_5: f64,
    pub ndcg_10: f64,
}

/// Optional LLM usage statistics (for rerank / agentic modes).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmStats {
    pub tool_calls: usize,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost_usd: f64,
}

/// Full result for a single question.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionResult {
    pub question_id: String,
    pub question_type: String,
    pub question: String,
    pub mode: String,
    pub ground_truth_sessions: Vec<String>,
    pub retrieved: Vec<RetrievalResult>,
    pub metrics: Metrics,
    pub latency_ms: u64,
    pub llm_stats: Option<LlmStats>,
}

// ---------------------------------------------------------------------------
// Scoring helper
// ---------------------------------------------------------------------------

/// Compute `Metrics` from retrieval results against ground truth.
///
/// Builds a corpus_ids list (from `retrieved`) and a rankings index list,
/// then delegates to `scoring::evaluate_retrieval`.
fn compute_metrics(
    retrieved: &[RetrievalResult],
    ground_truth: &[String],
) -> Metrics {
    let corpus_ids: Vec<String> = retrieved.iter().map(|r| r.session_id.clone()).collect();
    let rankings: Vec<usize> = (0..corpus_ids.len()).collect();

    let (r5, _, n5) = scoring::evaluate_retrieval(&rankings, ground_truth, &corpus_ids, 5);
    let (r10, _, n10) = scoring::evaluate_retrieval(&rankings, ground_truth, &corpus_ids, 10);

    Metrics {
        recall_any_5: r5,
        recall_any_10: r10,
        ndcg_5: n5,
        ndcg_10: n10,
    }
}

// ---------------------------------------------------------------------------
// JSONL output
// ---------------------------------------------------------------------------

fn write_jsonl(results: &[QuestionResult], path: &Path) -> Result<()> {
    use std::io::Write;
    let file = std::fs::File::create(path)?;
    let mut writer = std::io::BufWriter::new(file);
    for r in results {
        serde_json::to_writer(&mut writer, r)?;
        writeln!(writer)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Summary printer
// ---------------------------------------------------------------------------

fn print_summary(mode: &str, results: &[QuestionResult]) {
    if results.is_empty() {
        eprintln!("[{mode}] no results");
        return;
    }

    let n = results.len() as f64;
    let r5: f64 = results.iter().map(|r| r.metrics.recall_any_5).sum::<f64>() / n;
    let r10: f64 = results.iter().map(|r| r.metrics.recall_any_10).sum::<f64>() / n;
    let n5: f64 = results.iter().map(|r| r.metrics.ndcg_5).sum::<f64>() / n;
    let n10: f64 = results.iter().map(|r| r.metrics.ndcg_10).sum::<f64>() / n;
    let avg_ms: f64 = results.iter().map(|r| r.latency_ms as f64).sum::<f64>() / n;

    eprintln!("[{mode}] {} questions", results.len());
    eprintln!("[{mode}]   Recall@5:  {r5:.4}");
    eprintln!("[{mode}]   Recall@10: {r10:.4}");
    eprintln!("[{mode}]   NDCG@5:    {n5:.4}");
    eprintln!("[{mode}]   NDCG@10:   {n10:.4}");
    eprintln!("[{mode}]   Avg latency: {avg_ms:.1} ms");

    // Per-type breakdown
    let type_results: Vec<(String, f64, f64, f64, f64)> = results
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
    let agg = scoring::aggregate_by_type(&type_results);
    for tm in &agg {
        eprintln!(
            "[{mode}]   {}: n={} R@5={:.4} R@10={:.4} NDCG@5={:.4} NDCG@10={:.4}",
            tm.question_type, tm.count, tm.recall_any_5, tm.recall_any_10, tm.ndcg_5, tm.ndcg_10,
        );
    }
}

// ---------------------------------------------------------------------------
// Main orchestrator
// ---------------------------------------------------------------------------

pub fn run(
    vault_dir: &Path,
    data: &Path,
    modes: &[&str],
    top_k: usize,
    limit: Option<usize>,
    #[allow(unused)] fusion_weight: f64,
    _max_tool_calls: usize,
    _llm_provider: &str,
    _llm_model: &str,
    _llm_base_url: Option<&str>,
    output: Option<&Path>,
    verbose: bool,
) -> Result<()> {
    // Validate requested modes
    let known = ["bm25", "semantic", "hybrid", "graph", "rerank", "agentic"];
    for m in modes {
        if !known.contains(m) {
            bail!("unknown mode: {m}");
        }
    }

    // --- Load vault ---
    eprintln!("[run] scanning vault at {}", vault_dir.display());
    let files = scan_vault(vault_dir, &[])?;
    eprintln!("[run] found {} files", files.len());

    // --- Build search index (needed for bm25, hybrid, graph, rerank, agentic) ---
    let search_index = if modes.iter().any(|m| ["bm25", "hybrid", "graph", "rerank", "agentic"].contains(m)) {
        eprintln!("[run] building search index...");
        Some(SearchIndex::build(vault_dir, &files)?)
    } else {
        None
    };

    // --- Build vector index (needed for semantic, hybrid, graph, rerank) ---
    #[cfg(feature = "semantic")]
    let vector_index = if modes.iter().any(|m| ["semantic", "hybrid", "graph", "rerank"].contains(m)) {
        eprintln!("[run] building vector index...");
        Some(zetl::semantic::VectorIndex::build(vault_dir, &files)?)
    } else {
        None
    };

    // --- Build link graph (needed for graph, agentic modes) ---
    #[allow(unused)]
    let link_graph = if modes.iter().any(|m| *m == "graph" || *m == "agentic") {
        eprintln!("[run] building link graph...");
        let resolved_pages: std::collections::HashMap<String, String> = files
            .iter()
            .map(|f| (f.page_name.clone(), f.page_name.clone()))
            .collect();
        Some(zetl::graph::LinkGraph::build(&files, &resolved_pages))
    } else {
        None
    };

    // --- Build LLM client (needed for rerank, agentic modes) ---
    let llm_client = if modes.iter().any(|m| *m == "rerank" || *m == "agentic") {
        eprintln!("[run] creating LLM client ({_llm_provider}/{_llm_model})...");
        Some(llm::create_client(_llm_provider, _llm_model, _llm_base_url)?)
    } else {
        None
    };

    // --- Build tokio runtime (needed for async LLM calls) ---
    let rt = if modes.iter().any(|m| *m == "rerank" || *m == "agentic") {
        Some(tokio::runtime::Runtime::new()?)
    } else {
        None
    };

    // --- Load dataset ---
    let entries = dataset::load(data, limit)?;

    // --- Run each mode ---
    for mode in modes {
        eprintln!("[run] running mode: {mode}");

        let results = match *mode {
            "bm25" => {
                let idx = search_index.as_ref().expect("search index built for bm25");
                run_bm25(idx, &entries, top_k, verbose)?
            }
            #[cfg(feature = "semantic")]
            "semantic" => {
                let vi = vector_index.as_ref().expect("vector index built for semantic");
                run_semantic(vi, &entries, top_k, verbose)?
            }
            #[cfg(feature = "semantic")]
            "hybrid" => {
                let si = search_index.as_ref().expect("search index built for hybrid");
                let vi = vector_index.as_ref().expect("vector index built for hybrid");
                run_hybrid(si, vi, &entries, top_k, fusion_weight, verbose)?
            }
            #[cfg(feature = "semantic")]
            "graph" => {
                let si = search_index.as_ref().expect("search index built for graph");
                let vi = vector_index.as_ref().expect("vector index built for graph");
                let lg = link_graph.as_ref().expect("link graph built for graph");
                run_graph(si, vi, lg, &entries, top_k, fusion_weight, verbose)?
            }
            #[cfg(not(feature = "semantic"))]
            "semantic" | "hybrid" | "graph" => bail!("{mode} mode requires the 'semantic' feature"),
            "rerank" => {
                let si = search_index.as_ref().expect("search index built for rerank");
                let client = llm_client.as_ref().expect("LLM client built for rerank");
                let runtime = rt.as_ref().expect("tokio runtime for rerank");
                #[cfg(feature = "semantic")]
                let vi = vector_index.as_ref();
                run_rerank(
                    vault_dir, si,
                    #[cfg(feature = "semantic")]
                    vi,
                    client.as_ref(), &entries, top_k, fusion_weight, verbose, runtime,
                )?
            }
            "agentic" => {
                let si = search_index.as_ref().expect("search index built for agentic");
                let lg = link_graph.as_ref().expect("link graph built for agentic");
                let client = llm_client.as_ref().expect("LLM client built for agentic");
                let runtime = rt.as_ref().expect("tokio runtime for agentic");
                run_agentic(
                    vault_dir, si, lg, client.as_ref(), &entries, top_k,
                    _max_tool_calls, verbose, runtime,
                )?
            }
            _ => unreachable!(),
        };

        print_summary(mode, &results);

        // Write JSONL output
        let out_path = if let Some(p) = output {
            p.to_path_buf()
        } else {
            let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
            std::path::PathBuf::from(format!("bench_{mode}_{timestamp}.jsonl"))
        };
        write_jsonl(&results, &out_path)?;
        eprintln!("[run] wrote {} results to {}", results.len(), out_path.display());
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Per-question mode: fresh vault per question (mempalace-compatible)
// ---------------------------------------------------------------------------

pub fn run_per_question(
    data: &Path,
    modes: &[&str],
    top_k: usize,
    limit: Option<usize>,
    output: Option<&Path>,
    verbose: bool,
) -> Result<()> {
    // Only BM25 and semantic supported in per-question mode
    for m in modes {
        if !["bm25", "semantic"].contains(m) {
            bail!("per-question mode only supports bm25 and semantic, got: {m}");
        }
    }

    let entries = crate::dataset::load(data, limit)?;
    eprintln!("[per-question] {} questions, building fresh vault per question", entries.len());

    for mode in modes {
        let mut results = Vec::with_capacity(entries.len());

        for (qi, entry) in entries.iter().enumerate() {
            let start = Instant::now();

            // Build a temp vault with only this question's sessions
            let tmp_dir = tempfile::tempdir()?;
            let vault_dir = tmp_dir.path();
            let sessions_dir = vault_dir.join("sessions");
            std::fs::create_dir_all(&sessions_dir)?;

            // Write each session as a .md file (user turns only, matching mempalace)
            for (i, sid) in entry.haystack_session_ids.iter().enumerate() {
                let user_turns: Vec<&str> = entry.haystack_sessions[i]
                    .iter()
                    .filter(|t| t.role == "user")
                    .map(|t| t.content.as_str())
                    .collect();
                if user_turns.is_empty() {
                    continue;
                }
                let body = user_turns.join("\n");
                let content = format!("{body}\n");
                std::fs::write(sessions_dir.join(format!("{sid}.md")), content)?;
            }

            // Build search index for this mini-vault
            let files = scan_vault(vault_dir, &[])?;
            let search_index = SearchIndex::build(vault_dir, &files)?;

            let retrieved = match *mode {
                "bm25" => bm25::retrieve(&search_index, &entry.question, top_k)?,
                #[cfg(feature = "semantic")]
                "semantic" => {
                    let vi = zetl::semantic::VectorIndex::build(vault_dir, &files)?;
                    semantic::retrieve(&vi, &entry.question, top_k)?
                }
                _ => Vec::new(),
            };

            let latency_ms = start.elapsed().as_millis() as u64;
            let metrics = compute_metrics(&retrieved, &entry.answer_session_ids);

            if verbose {
                eprintln!(
                    "  [{mode}/pq] {} ({}): R@5={:.0} R@10={:.0} NDCG@5={:.4} ({} ms, {} sessions)",
                    entry.question_id, entry.question_type,
                    metrics.recall_any_5, metrics.recall_any_10, metrics.ndcg_5,
                    latency_ms, files.len(),
                );
            }

            results.push(QuestionResult {
                question_id: entry.question_id.clone(),
                question_type: entry.question_type.clone(),
                question: entry.question.clone(),
                mode: format!("{mode}/per-question"),
                ground_truth_sessions: entry.answer_session_ids.clone(),
                retrieved,
                metrics,
                latency_ms,
                llm_stats: None,
            });

            if (qi + 1) % 50 == 0 {
                eprintln!("[per-question] {}/{} done", qi + 1, entries.len());
            }
        }

        let mode_label = format!("{mode}/per-question");
        print_summary(&mode_label, &results);

        let out_path = if let Some(p) = output {
            p.to_path_buf()
        } else {
            let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
            std::path::PathBuf::from(format!("bench_{mode}_pq_{timestamp}.jsonl"))
        };
        write_jsonl(&results, &out_path)?;
        eprintln!("[per-question] wrote {} results to {}", results.len(), out_path.display());
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// BM25 runner
// ---------------------------------------------------------------------------

fn run_bm25(
    search_index: &SearchIndex,
    entries: &[dataset::LongMemEntry],
    top_k: usize,
    verbose: bool,
) -> Result<Vec<QuestionResult>> {
    let mut results = Vec::with_capacity(entries.len());

    for entry in entries {
        let start = Instant::now();
        let retrieved = bm25::retrieve(search_index, &entry.question, top_k)?;
        let latency_ms = start.elapsed().as_millis() as u64;

        let metrics = compute_metrics(&retrieved, &entry.answer_session_ids);

        if verbose {
            eprintln!(
                "  [bm25] {} ({}): R@5={:.0} R@10={:.0} NDCG@5={:.4} ({} ms)",
                entry.question_id,
                entry.question_type,
                metrics.recall_any_5,
                metrics.recall_any_10,
                metrics.ndcg_5,
                latency_ms,
            );
        }

        results.push(QuestionResult {
            question_id: entry.question_id.clone(),
            question_type: entry.question_type.clone(),
            question: entry.question.clone(),
            mode: "bm25".to_string(),
            ground_truth_sessions: entry.answer_session_ids.clone(),
            retrieved,
            metrics,
            latency_ms,
            llm_stats: None,
        });
    }

    Ok(results)
}

// ---------------------------------------------------------------------------
// Semantic runner
// ---------------------------------------------------------------------------

#[cfg(feature = "semantic")]
fn run_semantic(
    vector_index: &zetl::semantic::VectorIndex,
    entries: &[dataset::LongMemEntry],
    top_k: usize,
    verbose: bool,
) -> Result<Vec<QuestionResult>> {
    let mut results = Vec::with_capacity(entries.len());

    for entry in entries {
        let start = Instant::now();
        let retrieved = semantic::retrieve(vector_index, &entry.question, top_k)?;
        let latency_ms = start.elapsed().as_millis() as u64;

        let metrics = compute_metrics(&retrieved, &entry.answer_session_ids);

        if verbose {
            eprintln!(
                "  [semantic] {} ({}): R@5={:.0} R@10={:.0} NDCG@5={:.4} ({} ms)",
                entry.question_id,
                entry.question_type,
                metrics.recall_any_5,
                metrics.recall_any_10,
                metrics.ndcg_5,
                latency_ms,
            );
        }

        results.push(QuestionResult {
            question_id: entry.question_id.clone(),
            question_type: entry.question_type.clone(),
            question: entry.question.clone(),
            mode: "semantic".to_string(),
            ground_truth_sessions: entry.answer_session_ids.clone(),
            retrieved,
            metrics,
            latency_ms,
            llm_stats: None,
        });
    }

    Ok(results)
}

// ---------------------------------------------------------------------------
// Hybrid runner
// ---------------------------------------------------------------------------

#[cfg(feature = "semantic")]
fn run_hybrid(
    search_index: &SearchIndex,
    vector_index: &zetl::semantic::VectorIndex,
    entries: &[dataset::LongMemEntry],
    top_k: usize,
    fusion_weight: f64,
    verbose: bool,
) -> Result<Vec<QuestionResult>> {
    let mut results = Vec::with_capacity(entries.len());

    for entry in entries {
        let start = Instant::now();
        let retrieved = hybrid::retrieve(
            search_index,
            vector_index,
            &entry.question,
            top_k,
            fusion_weight,
        )?;
        let latency_ms = start.elapsed().as_millis() as u64;

        let metrics = compute_metrics(&retrieved, &entry.answer_session_ids);

        if verbose {
            eprintln!(
                "  [hybrid] {} ({}): R@5={:.0} R@10={:.0} NDCG@5={:.4} ({} ms)",
                entry.question_id,
                entry.question_type,
                metrics.recall_any_5,
                metrics.recall_any_10,
                metrics.ndcg_5,
                latency_ms,
            );
        }

        results.push(QuestionResult {
            question_id: entry.question_id.clone(),
            question_type: entry.question_type.clone(),
            question: entry.question.clone(),
            mode: "hybrid".to_string(),
            ground_truth_sessions: entry.answer_session_ids.clone(),
            retrieved,
            metrics,
            latency_ms,
            llm_stats: None,
        });
    }

    Ok(results)
}

// ---------------------------------------------------------------------------
// Graph-augmented runner
// ---------------------------------------------------------------------------

#[cfg(feature = "semantic")]
fn run_graph(
    search_index: &SearchIndex,
    vector_index: &zetl::semantic::VectorIndex,
    link_graph: &zetl::graph::LinkGraph,
    entries: &[dataset::LongMemEntry],
    top_k: usize,
    fusion_weight: f64,
    verbose: bool,
) -> Result<Vec<QuestionResult>> {
    let mut results = Vec::with_capacity(entries.len());

    for entry in entries {
        let start = Instant::now();
        let retrieved = graph::retrieve(
            search_index,
            vector_index,
            link_graph,
            &entry.question,
            top_k,
            fusion_weight,
        )?;
        let latency_ms = start.elapsed().as_millis() as u64;

        let metrics = compute_metrics(&retrieved, &entry.answer_session_ids);

        if verbose {
            eprintln!(
                "  [graph] {} ({}): R@5={:.0} R@10={:.0} NDCG@5={:.4} ({} ms)",
                entry.question_id,
                entry.question_type,
                metrics.recall_any_5,
                metrics.recall_any_10,
                metrics.ndcg_5,
                latency_ms,
            );
        }

        results.push(QuestionResult {
            question_id: entry.question_id.clone(),
            question_type: entry.question_type.clone(),
            question: entry.question.clone(),
            mode: "graph".to_string(),
            ground_truth_sessions: entry.answer_session_ids.clone(),
            retrieved,
            metrics,
            latency_ms,
            llm_stats: None,
        });
    }

    Ok(results)
}

// ---------------------------------------------------------------------------
// Rerank runner
// ---------------------------------------------------------------------------

fn run_rerank(
    vault_dir: &Path,
    search_index: &SearchIndex,
    #[cfg(feature = "semantic")]
    vector_index: Option<&zetl::semantic::VectorIndex>,
    llm: &dyn llm::LlmClient,
    entries: &[dataset::LongMemEntry],
    top_k: usize,
    fusion_weight: f64,
    verbose: bool,
    rt: &tokio::runtime::Runtime,
) -> Result<Vec<QuestionResult>> {
    let mut results = Vec::with_capacity(entries.len());

    for (qi, entry) in entries.iter().enumerate() {
        // Rate limit: wait between LLM calls (Groq free tier = 6K TPM)
        if qi > 0 {
            std::thread::sleep(std::time::Duration::from_secs(15));
        }

        let start = Instant::now();
        let result = rt.block_on(rerank::retrieve(
            vault_dir,
            search_index,
            #[cfg(feature = "semantic")]
            vector_index,
            llm,
            &entry.question,
            top_k,
            fusion_weight,
        ));
        let latency_ms = start.elapsed().as_millis() as u64;

        let (retrieved, llm_stats) = match result {
            Ok(r) => r,
            Err(e) => {
                eprintln!("  [rerank] {} ERROR: {e:#}", entry.question_id);
                (Vec::new(), LlmStats { tool_calls: 0, tokens_in: 0, tokens_out: 0, cost_usd: 0.0 })
            }
        };

        let metrics = compute_metrics(&retrieved, &entry.answer_session_ids);

        if verbose {
            eprintln!(
                "  [rerank] {} ({}): R@5={:.0} R@10={:.0} NDCG@5={:.4} ({} ms, {} tok_in, {} tok_out, ${:.6})",
                entry.question_id,
                entry.question_type,
                metrics.recall_any_5,
                metrics.recall_any_10,
                metrics.ndcg_5,
                latency_ms,
                llm_stats.tokens_in,
                llm_stats.tokens_out,
                llm_stats.cost_usd,
            );
        }

        results.push(QuestionResult {
            question_id: entry.question_id.clone(),
            question_type: entry.question_type.clone(),
            question: entry.question.clone(),
            mode: "rerank".to_string(),
            ground_truth_sessions: entry.answer_session_ids.clone(),
            retrieved,
            metrics,
            latency_ms,
            llm_stats: Some(llm_stats),
        });
    }

    Ok(results)
}

// ---------------------------------------------------------------------------
// Agentic runner
// ---------------------------------------------------------------------------

fn run_agentic(
    vault_dir: &Path,
    search_index: &SearchIndex,
    link_graph: &zetl::graph::LinkGraph,
    llm: &dyn llm::LlmClient,
    entries: &[dataset::LongMemEntry],
    top_k: usize,
    max_tool_calls: usize,
    verbose: bool,
    rt: &tokio::runtime::Runtime,
) -> Result<Vec<QuestionResult>> {
    let mut results = Vec::with_capacity(entries.len());

    for (qi, entry) in entries.iter().enumerate() {
        // Rate limit: wait between LLM calls (agentic uses ~6K tokens/question)
        if qi > 0 {
            std::thread::sleep(std::time::Duration::from_secs(65));
        }

        let start = Instant::now();
        let result = rt.block_on(agentic::retrieve(
            vault_dir,
            search_index,
            link_graph,
            llm,
            &entry.question,
            top_k,
            max_tool_calls,
        ));
        let latency_ms = start.elapsed().as_millis() as u64;

        let (retrieved, llm_stats) = match result {
            Ok(r) => r,
            Err(e) => {
                eprintln!("  [agentic] {} ERROR: {e:#}", entry.question_id);
                (Vec::new(), LlmStats { tool_calls: 0, tokens_in: 0, tokens_out: 0, cost_usd: 0.0 })
            }
        };

        let metrics = compute_metrics(&retrieved, &entry.answer_session_ids);

        if verbose {
            eprintln!(
                "  [agentic] {} ({}): R@5={:.0} R@10={:.0} NDCG@5={:.4} ({} ms, {} calls, {} tok_in, {} tok_out, ${:.6})",
                entry.question_id,
                entry.question_type,
                metrics.recall_any_5,
                metrics.recall_any_10,
                metrics.ndcg_5,
                latency_ms,
                llm_stats.tool_calls,
                llm_stats.tokens_in,
                llm_stats.tokens_out,
                llm_stats.cost_usd,
            );
        }

        results.push(QuestionResult {
            question_id: entry.question_id.clone(),
            question_type: entry.question_type.clone(),
            question: entry.question.clone(),
            mode: "agentic".to_string(),
            ground_truth_sessions: entry.answer_session_ids.clone(),
            retrieved,
            metrics,
            latency_ms,
            llm_stats: Some(llm_stats),
        });
    }

    Ok(results)
}
