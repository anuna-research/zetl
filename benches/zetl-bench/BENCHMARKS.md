# zetl-bench: LongMemEval Memory Retrieval Benchmark

**March 2026 — Retrieval quality evaluation across 6 modes, 500 questions, 19K sessions.**

---

## Summary

`zetl-bench` is a Rust benchmark crate that evaluates retrieval quality against the [LongMemEval](https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned) dataset — the standard benchmark for AI memory systems. It implements 6 retrieval modes from single-shot keyword search through agentic LLM-driven tool use, and supports both per-question haystacks (for comparison with published systems) and full-vault search (the harder, unsolved problem).

## Headline Results

### Per-question haystack (~50 sessions, matching mempalace methodology)

| System | Model | R@5 | R@10 | NDCG@5 |
|--------|-------|-----|------|--------|
| **zetl BM25** | none (Tantivy) | **93.8%** | **96.0%** | 0.881 |
| **zetl semantic** | all-MiniLM-L6-v2 | **93.8%** | **98.4%** | 0.869 |
| mempalace (raw) | all-MiniLM-L6-v2 | 96.6% | 98.2% | — |
| mempalace (BM25) | sparse keyword | ~70% | — | — |

zetl's Tantivy BM25 — with no embedding model — outperforms mempalace's own BM25 baseline by 24 points and comes within 3 points of their embedding-based headline number.

### Per-type breakdown (zetl BM25, 500 questions)

| Question Type | N | R@5 | R@10 |
|---------------|---|-----|------|
| knowledge-update | 78 | 100.0% | 100.0% |
| single-session-user | 70 | 98.6% | 100.0% |
| multi-session | 133 | 94.7% | 98.5% |
| temporal-reasoning | 133 | 94.0% | 95.5% |
| single-session-assistant | 56 | 87.5% | 89.3% |
| single-session-preference | 30 | 73.3% | 80.0% |

### Full-vault (19,195 sessions — the hard benchmark)

| Mode | R@5 | R@10 | Cost/q | Notes |
|------|-----|------|--------|-------|
| BM25 | 30.0% | 35.0% | $0 | Best single-shot |
| Semantic | 0% | 0% | $0 | Drowns in noise at scale |
| Hybrid | 25.0% | 30.0% | $0 | Semantic drags BM25 down |
| LLM Rerank | 0% | 0% | $0.00008 | Can't fix bad candidates |
| Agentic | 0% | 33.3% | $0.0003 | Only mode that finds new results |

## Why the Numbers Differ

The per-question benchmark builds a **fresh index per question** with only that question's ~50 haystack sessions. This is how mempalace benchmarks: the correct answer is guaranteed to be in a small pool.

The full-vault benchmark puts **all 19,195 sessions** into one index and searches across them. This is how a real knowledge base works — you don't pre-filter to 50 candidates before searching.

```
Per-question:  "Find 1 needle in a drawer of 50 items"     → 93.8% with keywords
Full-vault:    "Find 1 needle in a warehouse of 19,195"    → 30% with keywords
```

The per-question methodology flatters every system. Even random retrieval scores ~10% R@5. The full-vault benchmark exposes real retrieval limitations.

## Retrieval Modes

### Mode 1 — BM25 (Tantivy)

Single-shot full-text search. No model, no API key, no GPU. Uses Tantivy's BM25 scoring with query sanitization for natural language questions.

### Mode 2 — Semantic (all-MiniLM-L6-v2)

Single-shot vector search. Embeds query and documents with all-MiniLM-L6-v2 (384-dim, ONNX runtime). Cosine similarity ranking.

### Mode 3 — Hybrid (BM25 + Semantic)

Min-max normalized score fusion. Configurable weight: `score = (1-w) * bm25 + w * semantic`. Default w=0.5.

### Mode 4 — Graph-Augmented

Hybrid search + 1-hop wikilink expansion via LinkGraph. For each top candidate, follows forward links and backlinks, scores neighbors with 0.5 decay, merges and re-ranks.

### Mode 5 — LLM Rerank

Hybrid retrieval for top 20 candidates, then single LLM call to rerank by relevance. Supports Groq (Llama 3.1 8B, default), Anthropic (Haiku/Sonnet), or any OpenAI-compatible endpoint.

### Mode 6 — Agentic

LLM drives retrieval iteratively using tools: `search`, `read_page`, `links`, `backlinks`. The agent searches, reads results, follows links, refines queries, and produces a final ranked list. Capped at N tool calls (default 10). The only mode that found results the single-shot modes missed on the full-vault benchmark.

## Bug Found: Semantic Embedding Pooling

During benchmarking, we discovered that zetl's `embed_text()` was doing **plain mean pooling** instead of **attention-weighted mean pooling**. Padding tokens were polluting the embedding vectors.

**Fix (src/semantic/mod.rs):**
- Pad all inputs to 256 tokens (matching ChromaDB's approach)
- Use attention mask to exclude padding from mean pool: `sum(hidden * mask) / sum(mask)`

**Impact:**
- Before: answer ranked 39th/53 (score: -0.014)
- After: answer ranked 8th/53 (score: 0.021)
- R@5 on 500 questions: 5% → 93.8%

## Usage

```bash
# Download LongMemEval dataset (~300 MB)
curl -fsSL -o data.json \
  https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned/resolve/main/longmemeval_s_cleaned.json

# Ingest into a zetl vault
zetl-bench ingest data.json --vault-dir /tmp/vault --no-entities

# Run per-question BM25 (no model needed, ~2 minutes)
zetl-bench run /tmp/vault --data data.json --mode bm25 --per-question

# Run per-question semantic (needs --features semantic, ~12 minutes first run)
zetl-bench run /tmp/vault --data data.json --mode semantic --per-question

# Run full-vault BM25 (the hard benchmark)
zetl-bench run /tmp/vault --data data.json --mode bm25

# Run with LLM rerank (needs GROQ_API_KEY)
zetl-bench run /tmp/vault --data data.json --mode rerank \
  --llm-provider groq --llm-model llama-3.1-8b-instant

# Compare results
zetl-bench compare results_bm25.jsonl results_semantic.jsonl

# Quick test on 20 questions
zetl-bench run /tmp/vault --data data.json --mode bm25 --per-question --limit 20 --verbose
```

## Requirements

- Rust (builds as workspace member of zetl)
- LongMemEval dataset (~300 MB JSON)
- For semantic mode: `--features semantic` (downloads all-MiniLM-L6-v2, ~90 MB)
- For rerank/agentic: API key (`GROQ_API_KEY` or `ANTHROPIC_API_KEY`)
- No GPU required

## Tests

36 unit tests covering dataset parsing, entity extraction, scoring (DCG, NDCG, recall@k), vault ingestion, JSONL output, and result comparison.

```bash
cargo test -p zetl-bench
```
