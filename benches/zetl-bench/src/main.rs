mod dataset;
mod entity;
mod ingest;
mod scoring;
mod report;
mod compare;
mod modes;
mod llm;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "zetl-bench", about = "LongMemEval memory retrieval benchmark for zetl")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Ingest LongMemEval JSON into a zetl-compatible Markdown vault
    Ingest {
        /// Path to longmemeval_s_cleaned.json
        data: PathBuf,
        /// Output vault directory
        #[arg(long)]
        vault_dir: PathBuf,
        /// Skip entity extraction and wikilink generation (faster ingestion)
        #[arg(long)]
        no_entities: bool,
    },
    /// Run benchmark against an ingested vault
    Run {
        /// Path to ingested vault
        vault_dir: PathBuf,
        /// Path to original LongMemEval JSON (for ground truth)
        #[arg(long)]
        data: PathBuf,
        /// Comma-separated retrieval modes: bm25,semantic,hybrid,graph,rerank,agentic
        #[arg(long, default_value = "bm25")]
        mode: String,
        /// Number of results to retrieve
        #[arg(long, default_value = "10")]
        top_k: usize,
        /// Run on first N questions only
        #[arg(long)]
        limit: Option<usize>,
        /// Hybrid fusion weight (0.0 = pure BM25, 1.0 = pure semantic)
        #[arg(long, default_value = "0.5")]
        fusion_weight: f64,
        /// Max tool calls for agentic mode
        #[arg(long, default_value = "10")]
        max_tool_calls: usize,
        /// LLM provider: groq, anthropic, openai-compat
        #[arg(long, default_value = "groq")]
        llm_provider: String,
        /// LLM model name
        #[arg(long, default_value = "llama-3.1-8b-instant")]
        llm_model: String,
        /// Custom LLM API base URL
        #[arg(long)]
        llm_base_url: Option<String>,
        /// JSONL output path (auto-named if omitted)
        #[arg(long)]
        output: Option<PathBuf>,
        /// Print per-question results
        #[arg(long)]
        verbose: bool,
        /// Per-question haystack (fresh vault per question, like mempalace). Apples-to-apples comparison.
        #[arg(long)]
        per_question: bool,
    },
    /// Compare results across multiple JSONL files
    Compare {
        /// JSONL result files to compare
        files: Vec<PathBuf>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Ingest { data, vault_dir, no_entities } => {
            ingest::run(&data, &vault_dir, no_entities)
        }
        Command::Run {
            vault_dir,
            data,
            mode,
            top_k,
            limit,
            fusion_weight,
            max_tool_calls,
            llm_provider,
            llm_model,
            llm_base_url,
            output,
            verbose,
            per_question,
        } => {
            let modes: Vec<&str> = mode.split(',').collect();
            if per_question {
                modes::run_per_question(
                    &data, &modes, top_k, limit, output.as_deref(), verbose,
                )
            } else {
                modes::run(
                    &vault_dir, &data, &modes, top_k, limit, fusion_weight,
                    max_tool_calls, &llm_provider, &llm_model,
                    llm_base_url.as_deref(), output.as_deref(), verbose,
                )
            }
        }
        Command::Compare { files } => {
            compare::run(&files)
        }
    }
}
