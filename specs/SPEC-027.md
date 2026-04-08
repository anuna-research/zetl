---
title: "SPEC-027: LongMemEval Memory Benchmark — Retrieval Quality Evaluation with Agentic Mode"
version: 0.1.0
status: draft
date: 2026-04-08
audience: agent, human
parent: SPEC-001
related:
  - SPEC-002  # Full-text search (BM25 mode baseline)
  - SPEC-018  # Semantic search (semantic + hybrid modes)
  - SPEC-021  # MCP server (agentic mode tool interface)
  - SPEC-026  # AST compression (potential graph-augmented optimization)
dependencies:
  - tantivy (existing; BM25 full-text search)
  - reqwest (new; LLM API calls for rerank + agentic modes)
  - serde_json (existing; dataset parsing + JSONL output)
  - tokio (existing; async LLM calls + MCP stdio)
---

# SPEC-027: LongMemEval Memory Benchmark

Ports the LongMemEval academic benchmark to Rust, evaluating zetl's retrieval quality across six modes — from single-shot BM25 through agentic LLM-driven graph traversal. Built as a separate `zetl-bench` crate in the workspace, using zetl as a library for modes 1–5 and the MCP server for mode 6 (agentic).

## 1. Motivation

### 1.1 Why Benchmark Memory Retrieval?

zetl has three search paths (BM25, semantic, hybrid) plus a wikilink graph with backlinks, forward links, and path traversal. There is no quantitative evaluation of how well these modes retrieve relevant information from a knowledge base. Without numbers, we cannot:

- Compare zetl's retrieval quality against published systems (Mem0, Supermemory, MemPal)
- Measure whether graph-augmented retrieval actually improves on flat search
- Evaluate whether an LLM agent driving zetl's tools outperforms single-shot retrieval

### 1.2 Why LongMemEval?

LongMemEval is the standard benchmark for AI memory systems. 500 questions across 6 question types, each with ~53 conversation sessions as the haystack and ground truth answer sessions. Published baselines exist for BM25 (~70%), dense retrievers (~85%), and state-of-the-art systems (96–100%). This gives us direct comparability.

### 1.3 Why Agentic Retrieval?

No published memory benchmark includes a mode where the LLM actively drives retrieval tools — searching, following links, refining queries, and navigating a graph iteratively. zetl's MCP server provides exactly this interface. Benchmarking it against single-shot modes quantifies the value of tool-use for memory retrieval.

## 2. Architecture

### 2.1 Crate Structure

```
benches/
  zetl-bench/
    Cargo.toml          # workspace member, depends on zetl
    src/
      main.rs           # CLI entry point (clap)
      ingest.rs         # JSON → Markdown vault with wikilinks
      entity.rs         # Entity extraction + wikilink generation
      modes/
        mod.rs
        bm25.rs         # Mode 1: Tantivy full-text
        semantic.rs     # Mode 2: Vector search
        hybrid.rs       # Mode 3: BM25 + semantic fusion
        graph.rs        # Mode 4: Hybrid + 1-hop link expansion
        rerank.rs       # Mode 5: Hybrid + LLM rerank
        agentic.rs      # Mode 6: LLM drives MCP tools
      llm/
        mod.rs          # LlmClient trait
        groq.rs         # Groq API (default)
        anthropic.rs    # Claude API
        openai_compat.rs # Generic OpenAI-compatible
      scoring.rs        # Recall@k, NDCG@k computation
      report.rs         # Console + JSONL output
      compare.rs        # Multi-file comparison
      dataset.rs        # LongMemEval JSON parsing
```

### 2.2 Integration Model

- **Modes 1–5** use zetl as a Rust library: direct calls to `SearchIndex`, `VectorIndex`, `LinkGraph`. No process spawning. Fast.
- **Mode 6 (agentic)** spawns `zetl mcp --transport stdio` as a child process. The benchmark acts as MCP client, translating tool calls between the LLM and MCP server. Tests the real agent experience.

### 2.3 LLM Client

```rust
#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn complete(
        &self,
        messages: Vec<Message>,
        tools: Option<Vec<Tool>>,
    ) -> Result<Response>;

    fn provider_name(&self) -> &str;
    fn model_name(&self) -> &str;
    fn estimate_cost(&self, tokens_in: u64, tokens_out: u64) -> f64;
}
```

Three implementations:
- **`GroqClient`** — Default. OpenAI-compatible API at `api.groq.com`. `GROQ_API_KEY` env var. Default model: `llama-3.1-8b-instant`.
- **`AnthropicClient`** — Claude API. `ANTHROPIC_API_KEY`. For Haiku/Sonnet comparison.
- **`OpenAICompatibleClient`** — Any OpenAI-compatible endpoint via `--llm-base-url`. Covers Ollama, vLLM, local models.

Selected via `--llm-provider groq|anthropic|openai-compat` and `--llm-model <name>`.

## 3. Dataset Ingestion

### 3.1 Input Format

LongMemEval JSON (`longmemeval_s_cleaned.json`): array of 500 question objects, each containing:
- `question_id`, `question`, `question_type` (6 types)
- `haystack_sessions`: array of ~53 conversation session strings
- `haystack_session_ids`: parallel array of session identifiers
- `haystack_dates`: parallel array of session timestamps
- `answer_session_ids`: ground truth session IDs

### 3.2 Vault Generation

One Markdown file per session:

```markdown
---
session_id: session_001
timestamp: "2024-03-15T10:30:00Z"
question_ids: ["q_001", "q_015"]
---

# Session 001

**User:** I've been thinking about switching from MySQL to [[PostgreSQL]] for the [[project-alpha]] backend.

**Assistant:** That's a solid choice. [[PostgreSQL]] has better support for JSON queries...

## Related

- [[session_042]] — also discusses [[PostgreSQL]] migration
- [[session_017]] — [[project-alpha]] architecture decisions
```

Entity hub pages in `entities/`:

```markdown
---
entity: postgresql
type: technology
mentions: 12
---

# PostgreSQL

Referenced in:
- [[session_001]] — considering migration from MySQL
- [[session_042]] — migration completed
- [[session_089]] — performance tuning
```

### 3.3 Entity Extraction

Deterministic heuristics (no LLM):

1. **Proper nouns** — capitalized multi-word sequences not at sentence start
2. **Quoted terms** — anything in quotes
3. **Repeated noun phrases** — phrases appearing in 3+ sessions
4. **Temporal markers** — dates, days, relative time expressions
5. **Technical terms** — backtick-wrapped identifiers

**Wikilink insertion rules:**
- First mention per session gets a `[[wikilink]]`; subsequent mentions stay plain text
- Entity names normalized: lowercase, collapse whitespace, stem plurals
- Cross-session links: sessions sharing 2+ entities get `## Related` sections linking them directly

### 3.4 Ingestion Output

```
Sessions generated:  26,500
Entities extracted:   3,420
Wikilinks inserted:  18,700
Hub pages created:    3,420
Cross-session links:  8,200
Vault size:          ~45 MB
```

Deterministic: same input JSON always produces the same vault.

## 4. Retrieval Modes

### 4.1 Mode 1 — BM25

Single call to `SearchIndex::search(query, top_k)`. Tantivy full-text with default BM25 scoring. No features required beyond base zetl. Baseline mode.

### 4.2 Mode 2 — Semantic

Single call to `VectorIndex::search(query, top_k)`. Cosine similarity on embeddings. Requires `--features semantic`.

### 4.3 Mode 3 — Hybrid

BM25 + semantic fusion. Retrieve `top_k` from both indexes, normalize scores to [0, 1], weighted combination:

```
score = (1 - weight) * bm25_norm + weight * semantic_norm
```

Configurable fusion weight via `--fusion-weight` (default 0.5). Requires `--features semantic`.

### 4.4 Mode 4 — Graph-Augmented

1. Hybrid search for initial `top_k` candidates
2. For each candidate, query `LinkGraph` for forward links and backlinks (1 hop)
3. Score neighbors: `neighbor_score = origin_score * decay` (default decay 0.5)
4. Merge into candidate set, deduplicate, re-sort by score
5. Return final `top_k`

This is the zetl-unique mode — leverages the wikilink graph to surface sessions connected to strong matches.

### 4.5 Mode 5 — LLM Rerank

1. Hybrid retrieval for top 50 candidates
2. Format candidates as numbered passages with session IDs
3. Single LLM call: "Given this question and these passages, rank the top 10 by relevance. Return session IDs in order."
4. Parse structured output → final ranked list

### 4.6 Mode 6 — Agentic

1. Spawn `zetl mcp --transport stdio` pointing at the benchmark vault
2. Connect LLM with system prompt describing available tools:
   - `search` — full-text, semantic, or hybrid search
   - `links` — forward links from a page
   - `backlinks` — backlinks to a page
   - `path` — shortest path between pages
   - `blocks` — read content blocks of a page
3. Give the LLM the question: "Find the session(s) that answer this question. Use the available tools. Return your final answer as a ranked list of session IDs."
4. Run tool loop: LLM picks tool → benchmark calls MCP → result returned to LLM → repeat
5. Cap at `--max-tool-calls` (default 10) to bound cost
6. Collect all sessions the agent referenced/returned as the ranked list

## 5. Scoring

### 5.1 Metrics

| Metric | Formula | Description |
|--------|---------|-------------|
| Recall@k | `\|retrieved ∩ ground_truth\| / \|ground_truth\|` | Did we find the answer? |
| NDCG@k | Discounted cumulative gain normalized by ideal | Does rank position matter? |
| Latency | Wall clock ms per question | Speed |
| Tool calls | Count (mode 6 only) | Agent efficiency |
| Tokens in/out | From LLM response (modes 5–6) | Cost driver |
| Cost (USD) | Estimated from tokens × provider pricing | Budget |

### 5.2 Aggregation

- **Overall** — mean across all 500 questions
- **Per question type** — 6 types from LongMemEval (knowledge_update, multi_session, temporal_reasoning, etc.)
- **Per mode** — side-by-side in `compare` output

### 5.3 JSONL Output

One JSON object per question per mode:

```json
{
  "question_id": "q_001",
  "question_type": "knowledge_update",
  "question": "What degree did I graduate with?",
  "mode": "hybrid",
  "ground_truth_sessions": ["session_042"],
  "retrieved_sessions": [
    {"id": "session_042", "score": 0.87, "rank": 1},
    {"id": "session_018", "score": 0.72, "rank": 2}
  ],
  "metrics": {
    "recall_5": 1.0,
    "recall_10": 1.0,
    "ndcg_5": 1.0,
    "ndcg_10": 1.0
  },
  "latency_ms": 12,
  "llm_stats": null
}
```

For modes 5–6, `llm_stats`:
```json
{
  "tool_calls": 4,
  "tokens_in": 2100,
  "tokens_out": 340,
  "cost_usd": 0.0002
}
```

## 6. CLI Interface

```bash
# Ingest dataset → vault
zetl-bench ingest <data.json> --vault-dir <path>

# Run benchmark (one or more modes)
zetl-bench run <vault-dir> --data <data.json> \
  --mode bm25,semantic,hybrid,graph,rerank,agentic \
  --top-k 10 \
  --limit 20 \
  --fusion-weight 0.5 \
  --max-tool-calls 10 \
  --llm-provider groq \
  --llm-model llama-3.1-8b-instant \
  --output results.jsonl

# Compare results across modes
zetl-bench compare <file1.jsonl> <file2.jsonl> ...
```

### 6.1 Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--mode` | `bm25` | Comma-separated modes to run |
| `--top-k` | `10` | Number of results to retrieve |
| `--limit` | all | Run on first N questions only |
| `--fusion-weight` | `0.5` | Hybrid mode BM25/semantic balance |
| `--max-tool-calls` | `10` | Agentic mode tool call cap |
| `--llm-provider` | `groq` | LLM provider for modes 5–6 |
| `--llm-model` | `llama-3.1-8b-instant` | Model name |
| `--llm-base-url` | provider default | Custom endpoint URL |
| `--output` | auto-named | JSONL output path |
| `--verbose` | off | Print per-question results |

### 6.2 Console Output

```
═══════════════════════════════════════════════════════
  zetl-bench — LongMemEval (hybrid, 500 questions)
═══════════════════════════════════════════════════════
  Time: 42.3s (84ms/question)

  Recall@5:  0.952    NDCG@5:  0.931
  Recall@10: 0.978    NDCG@10: 0.948

  Per-type breakdown:
    knowledge_update     R@10=1.000  (n=78)
    multi_session        R@10=0.985  (n=133)
    temporal_reasoning   R@10=0.962  (n=133)
    ...

  Results: results_hybrid_20260408_1423.jsonl
═══════════════════════════════════════════════════════
```

### 6.3 Compare Output

```
Mode        R@5    R@10   NDCG@10  Avg ms  Cost/q
─────────────────────────────────────────────────────
bm25        0.712  0.789  0.701    3ms     $0
semantic    0.934  0.961  0.912    8ms     $0
hybrid      0.952  0.978  0.948    11ms    $0
graph       0.968  0.984  0.959    15ms    $0
rerank      0.994  0.998  0.991    210ms   $0.0001
agentic     0.998  1.000  0.997    1.2s    $0.0008
```

## 7. Dependencies

### 7.1 New Crate Dependencies (zetl-bench only)

| Crate | Purpose |
|-------|---------|
| `reqwest` | HTTP client for LLM APIs |
| `clap` | CLI argument parsing (already in zetl) |
| `tokio` | Async runtime (already in zetl) |
| `serde` / `serde_json` | JSON parsing (already in zetl) |
| `indicatif` | Progress bars for long benchmark runs |

### 7.2 zetl Features Required

- Base zetl: modes 1, 5, 6
- `--features semantic`: modes 2, 3, 4 (graph-augmented uses hybrid as its initial retrieval)
- `--features mcp`: mode 6

### 7.3 External Requirements

- LongMemEval dataset (~300 MB JSON download)
- API key for modes 5–6 (`GROQ_API_KEY` or `ANTHROPIC_API_KEY`)
- No GPU required

## 8. Non-Goals

- **Other benchmarks** — LoCoMo, ConvoMem, MemBench are future work using the same harness.
- **LLM-assisted ingestion** — Entity extraction is deterministic heuristics only.
- **Web UI for results** — Console + JSONL is sufficient. Visualization can be added later.
- **Benchmark-specific optimizations to zetl** — The benchmark measures zetl as-is. Improvements go through normal specs.
