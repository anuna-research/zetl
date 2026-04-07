---
title: "SPEC-024: LLM-Augmented Hybrid Search — Query Expansion, Reranking, and Graph Boosting"
version: 0.1.0
status: draft
date: 2026-04-07
audience: agent, human
parent: SPEC-018
related:
  - SPEC-013
  - SPEC-002
  - SPEC-023
dependencies:
  - llama-cpp-2 (local LLM inference via llama.cpp Rust bindings)
  - tantivy (existing)
  - ort (existing, SPEC-018)
---

| Field        | Value                                                                       |
|--------------|-----------------------------------------------------------------------------|
| Document     | SPEC-024                                                                    |
| Title        | LLM-Augmented Hybrid Search — Query Expansion, Reranking, and Graph Boosting |
| Version      | 0.1.0                                                                       |
| Status       | Draft                                                                       |
| Author       | Agent (USDD Protocol v1.3.0)                                                |
| Date         | 2026-04-07                                                                  |
| Audience     | agent, human                                                                |
| Trace        | USDD §2 (Vision → Specification)                                            |
| Parent       | SPEC-018: Semantic Search                                                   |
| Related      | SPEC-013, SPEC-002, SPEC-023                                                |
| Feature Gate | `--features llm-search` (implies `semantic`)                                |

---

## 1. Overview

### 1.1 Problem

SPEC-018 introduced hybrid BM25+vector search with reciprocal rank fusion. This works well when the user's query closely matches the vocabulary or semantic space of vault content. But three failure modes persist:

1. **Vocabulary mismatch.** A query for "distributed consensus" fails to retrieve a note titled "Paxos and Raft" because BM25 requires lexical overlap and the embedding model's 384 dimensions cannot always bridge the gap for domain-specific terminology.

2. **Rank fusion ceiling.** RRF treats BM25 and vector signals as equally reliable. In practice, some queries are better served by lexical matching (precise technical terms) while others need semantic understanding (natural-language questions). A flat fusion with no query-level adaptation leaves relevance on the table.

3. **No graph awareness in ranking.** zetl's distinguishing feature is the link graph, but the search pipeline ignores it entirely during ranking. A note two hops from the user's current focus page should rank higher than an equally relevant note in an unconnected cluster — this is the Zettelkasten insight that proximity in the graph reflects conceptual relevance.

### 1.2 Core Insight

A small local LLM (1-2B parameters, GGUF format, CPU inference) can serve three roles in the search pipeline without requiring network access or GPU hardware:

- **Query expander**: Generate lexical, semantic, and hypothetical-document (HyDE) reformulations of the original query, casting a wider retrieval net.
- **Reranker**: Score candidate results with a cross-encoder-style yes/no relevance judgment, using logprob confidence as a continuous score.
- **Graph-aware booster**: When combined with zetl's link graph, reranking scores can be blended with graph-distance signals to produce a ranking that respects both content relevance and structural proximity.

The key architectural constraint is that all inference runs locally via in-process Rust bindings to llama.cpp. No external API calls, no Python sidecar, no GPU requirement. This preserves zetl's local-first, single-binary, zero-config philosophy.

### 1.3 Design Philosophy

1. **Local-first LLM.** All models run in-process via `llama-cpp-2` Rust bindings. No network calls at query time. Models are GGUF files cached in `~/.cache/zetl/models/`.
2. **Graceful degradation.** Every LLM-augmented stage is optional. `--no-rerank` skips the reranker. If models are missing, fall back to SPEC-018 hybrid search with a warning. The pipeline never fails because a model is unavailable.
3. **Latency budget.** The full pipeline (expand + multi-retrieve + rerank) must complete within 2 seconds on CPU for a 5,000-page vault. Each stage has an independent timeout; if a stage exceeds its budget, its output is discarded and the pipeline proceeds with what it has.
4. **Graph is a first-class signal.** Link-graph distance is not a filter (that is SPEC-013's `--near`) but a continuous boost factor blended into the final score. Graph proximity and content relevance reinforce each other.
5. **Explain everything.** The `--explain` flag exposes the full score decomposition for every result: BM25 rank, vector rank, RRF score, reranker score, graph boost, final blended score. This makes the pipeline debuggable and trustworthy.

### 1.4 Scope

**In scope:**

- Local LLM inference via `llama-cpp-2` Rust bindings (GGUF models)
- Query expansion: lexical, vector, and HyDE query variants
- Multi-query reciprocal rank fusion extending SPEC-018
- LLM reranking with logprob-based confidence scoring
- Position-aware blending of RRF and reranker scores
- Graph-distance boosting relative to a focus page
- Model auto-download to `~/.cache/zetl/models/`
- `zetl search --augmented` as the top-tier search mode
- `--no-rerank` flag for fast mode
- `--explain` flag for score trace output
- Feature gating behind `--features llm-search`

**Out of scope:**

- Fine-tuning or training models (users supply pre-trained GGUF files)
- GPU acceleration (CPU-only; GPU support may come later via llama.cpp's CUDA/Metal backends)
- Streaming or interactive search (batch query-response only)
- Modifying SPEC-018's existing `--hybrid` behaviour (that remains unchanged)
- Cloud/API-based LLM inference

---

## 2. User Profiles

### 2.1 Power Researcher

A knowledge worker with a 3,000-page vault spanning multiple disciplines. They search for "systems that maintain stability through feedback" and expect to find notes on homeostasis, PID controllers, market equilibria, and thermostat design — even though none share vocabulary with the query. Today's hybrid search finds the PID controller note (semantic match) but misses the market equilibria note (domain-specific terminology). With query expansion generating a HyDE passage about "negative feedback loops in complex systems," the expanded retrieval net catches all four.

### 2.2 Agent Pipeline

An LLM agent uses `zetl search --augmented` to gather context for answering a user's question. The agent's natural-language query benefits from HyDE expansion (the LLM generates a hypothetical answer passage, which embeds closer to actual vault content than the question does). The agent also passes `--focus economics/monetary-policy` to boost results near the user's current working context in the graph.

### 2.3 Fast-Mode User

A user who values speed over exhaustiveness. They use `zetl search --augmented --no-rerank` to get the benefits of query expansion without the latency cost of the reranker. On their older laptop, the reranker adds 800ms; skipping it keeps total latency under 500ms.

---

## 3. Requirements

| ID      | Requirement |
|---------|-------------|
| REQ-135 | `zetl search --augmented <QUERY>` runs the full LLM-augmented pipeline: query expansion → multi-query retrieval → RRF fusion → LLM reranking → graph boosting → position-aware blending |
| REQ-136 | Query expansion generates three variant queries from the original: (a) a lexical reformulation emphasising keywords and synonyms, (b) a semantic reformulation as a natural-language statement, (c) a HyDE (Hypothetical Document Embedding) passage that imagines a document answering the query |
| REQ-137 | Multi-query fusion runs the original query (2x weight) and all expanded queries through both BM25 and vector retrieval in parallel, then fuses via RRF with k=60 |
| REQ-138 | LLM reranking scores the top 30 RRF candidates by prompting a reranker model with a yes/no relevance judgment; the score is derived from the logprob of the "yes" token |
| REQ-139 | Position-aware blending combines RRF rank score and reranker score with position-dependent weights: ranks 1-3 = 75% RRF / 25% reranker; ranks 4-10 = 60% RRF / 40% reranker; ranks 11+ = 40% RRF / 60% reranker |
| REQ-140 | `--focus <PAGE>` enables graph boosting: results within N hops of the focus page receive a multiplicative boost factor that decays with graph distance (boost = 1.0 + 0.3 / distance; distance 0 = same page = 1.3x, distance 1 = 1.3x, distance 2 = 1.15x, distance 3+ = 1.0x) |
| REQ-141 | `--no-rerank` skips the LLM reranker and graph boosting stages, returning RRF-fused results from the multi-query expansion (fast mode) |
| REQ-142 | `--explain` includes a score trace object on each result: `{ bm25_rank, vector_rank, rrf_score, reranker_score, graph_distance, graph_boost, final_score }` |
| REQ-143 | Models are auto-downloaded to `~/.cache/zetl/models/` on first use, with SHA-256 validation, progress reporting on stderr, and a `--model-dir` override |
| REQ-144 | The feature is gated behind `--features llm-search`; this feature implies `semantic`. When the feature is not compiled, `--augmented` prints a clear error message |
| REQ-145 | If no LLM models are available at query time (not downloaded, path invalid), the pipeline falls back to SPEC-018 `--hybrid` behaviour with a warning on stderr |

---

## 4. Architecture

### 4.1 Pipeline Overview

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                        LLM-Augmented Search Pipeline                        │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  ┌──────────┐    ┌───────────────────┐                                      │
│  │  Query Q  ├───►│  Query Expansion  │ (expansion model, ~1.7B GGUF)       │
│  └──────────┘    │  via local LLM    │                                      │
│                  └─────────┬─────────┘                                      │
│                            │                                                │
│               ┌────────────┼────────────┐                                   │
│               ▼            ▼            ▼                                    │
│          Q_original   Q_lex + Q_vec   Q_hyde                                │
│          (weight 2x)  (weight 1x)    (weight 1x)                           │
│               │            │            │                                   │
│        ┌──────┴──────┬─────┴──────┬─────┴──────┐                            │
│        ▼             ▼            ▼            ▼                             │
│   ┌─────────┐  ┌─────────┐ ┌─────────┐  ┌─────────┐                        │
│   │  BM25   │  │  BM25   │ │ Vector  │  │ Vector  │  ... (8 retrievals)    │
│   │  (Q_o)  │  │  (Q_l)  │ │ (Q_o)   │  │ (Q_h)   │                        │
│   └────┬────┘  └────┬────┘ └────┬────┘  └────┬────┘                        │
│        │            │           │            │                              │
│        └────────────┴───────────┴────────────┘                              │
│                            │                                                │
│                            ▼                                                │
│                  ┌──────────────────┐                                        │
│                  │  Multi-Query RRF │  k=60, original queries get 2x weight │
│                  │  (pure function) │                                        │
│                  └────────┬─────────┘                                        │
│                           │                                                 │
│                           ▼  Top 30 candidates                              │
│                  ┌──────────────────┐                                        │
│                  │  LLM Reranker    │  (reranker model, ~0.6B GGUF)         │
│                  │  yes/no logprobs │                                        │
│                  └────────┬─────────┘                                        │
│                           │                                                 │
│                           ▼                                                 │
│                  ┌──────────────────┐                                        │
│                  │  Graph Boosting  │  (if --focus PAGE)                     │
│                  │  distance decay  │                                        │
│                  └────────┬─────────┘                                        │
│                           │                                                 │
│                           ▼                                                 │
│                  ┌──────────────────┐                                        │
│                  │ Position-Aware   │                                        │
│                  │   Blending       │                                        │
│                  └────────┬─────────┘                                        │
│                           │                                                 │
│                           ▼                                                 │
│                  Final ranked results                                        │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 4.2 Stage Detail

#### Stage 1: Query Expansion (REQ-136)

The expansion model receives a structured prompt and generates three query variants in a single inference call:

```
System: You are a search query expansion assistant. Given a search query,
generate three variants to improve retrieval.

User: Query: "{original_query}"

Generate exactly three lines:
LEX: <keyword-focused reformulation with synonyms and related terms>
VEC: <natural-language statement capturing the query's meaning>
HYDE: <a short paragraph that a relevant document might contain>
```

**Parsing**: The output is parsed line-by-line. If parsing fails for any variant, that variant is silently dropped and the pipeline continues with the successfully parsed variants plus the original query.

**Timeout**: 500ms. If the LLM does not complete within 500ms, expansion is skipped entirely and the pipeline falls back to single-query hybrid search.

#### Stage 2: Multi-Query Retrieval and RRF (REQ-137)

Each query variant is run through both BM25 (Tantivy) and vector (SPEC-018 VectorIndex) retrieval, producing up to 8 ranked lists:

| Query      | BM25         | Vector       |
|------------|--------------|--------------|
| Q_original | list_bm25_o  | list_vec_o   |
| Q_lex      | list_bm25_l  | list_vec_l   |
| Q_vec      | list_bm25_v  | list_vec_v   |
| Q_hyde     | list_bm25_h  | list_vec_h   |

Each retrieval returns the top 50 results. Lists from Q_original are counted twice in the RRF fusion (equivalent to 2x weight):

```
score(d) = Σ_i  w_i / (k + rank_i(d))

where:
  k = 60
  w_i = 2 for lists derived from Q_original
  w_i = 1 for lists derived from expanded queries
```

This extends SPEC-018's existing `reciprocal_rank_fusion` function to accept weighted lists.

#### Stage 3: LLM Reranking (REQ-138)

The top 30 candidates from Stage 2 are reranked by a small cross-encoder-style model. For each candidate, the reranker is prompted:

```
Document: "{title}: {first_512_chars_of_content}"
Query: "{original_query}"
Is this document relevant to the query? Answer yes or no.
```

The reranker score is the logprob of the "yes" token, normalised to [0.0, 1.0] via sigmoid. If the model does not support logprob extraction, fall back to: output starts with "yes" → score 1.0, "no" → score 0.0.

**Batching**: Candidates are processed sequentially (llama.cpp single-context). Total timeout for reranking: 1200ms. If the timeout is reached, only candidates scored so far are reranked; remaining candidates retain their RRF rank.

#### Stage 4: Graph Boosting (REQ-140)

When `--focus <PAGE>` is provided, the link graph (from SPEC-001) is used to compute the shortest-path distance from the focus page to each candidate's page. The boost factor is:

```
graph_boost(d) = 1.0 + 0.3 / max(distance(focus, d), 1)

  distance 0 (same page):    not applicable (would not appear in results)
  distance 1 (direct link):  1.0 + 0.3/1 = 1.30
  distance 2 (2 hops):       1.0 + 0.3/2 = 1.15
  distance 3 (3 hops):       1.0 + 0.3/3 = 1.10
  distance ≥ 4 or no path:   1.0 (no boost)
```

Graph distances are computed via BFS from the focus page, capped at depth 3. Pages not reachable within 3 hops receive no boost. The boost is multiplicative on the blended score.

#### Stage 5: Position-Aware Blending (REQ-139)

The final score for each candidate combines the RRF score and the reranker score with position-dependent weights, then applies the graph boost:

```
Given:
  rrf_score    = normalised RRF score (0.0–1.0, via min-max over the candidate set)
  rerank_score = reranker logprob score (0.0–1.0)
  rank         = position in the RRF-sorted list (1-indexed)

Blend weights:
  if rank ∈ [1, 3]:   α = 0.75, β = 0.25
  if rank ∈ [4, 10]:  α = 0.60, β = 0.40
  if rank ≥ 11:       α = 0.40, β = 0.60

blended_score = α * rrf_score + β * rerank_score
final_score   = blended_score * graph_boost(d)
```

**Rationale for position-aware weights**: Top RRF results are already high-confidence — the reranker should not displace them unless it strongly disagrees. Lower-ranked results benefit more from the reranker's deeper understanding because the RRF signal is weaker at lower ranks.

### 4.3 Storage Layout

```
~/.cache/zetl/models/
├── manifest.json              (model registry: name, sha256, size, url)
├── zetl-expand-1.7b-q4.gguf  (query expansion model)
└── zetl-rerank-0.6b-q4.gguf  (reranker model)
```

Models are stored in the user-level cache, not the vault, because they are large (0.5–1.5GB) and shared across vaults.

### 4.4 Purity Boundary Map

#### Pure Core (no I/O, no shared state, deterministic)

- `weighted_rrf(lists: &[(Vec<RankedHit>, f64)], k: f64) → Vec<(String, f64)>` — multi-query weighted reciprocal rank fusion
- `position_aware_blend(rrf_score: f64, rerank_score: f64, rank: usize) → f64` — applies position-dependent weights
- `graph_boost(distance: Option<u32>) → f64` — computes multiplicative boost from graph distance
- `normalise_rrf_scores(scores: &mut [(String, f64)])` — min-max normalisation to [0.0, 1.0]
- `parse_expansion_output(raw: &str) → QueryExpansion` — parses LEX/VEC/HYDE lines from LLM output
- `reranker_score_from_logprob(logprob: f32) → f32` — sigmoid normalisation of yes-token logprob

#### Effectful Shell (orchestrates I/O, calls pure core)

- `LlmEngine::new(model_path: &Path, n_ctx: u32) → Result<Self>` — loads GGUF model via llama-cpp-2
- `LlmEngine::expand_query(query: &str) → Result<QueryExpansion>` — runs expansion prompt, parses output
- `LlmEngine::rerank(query: &str, candidates: &[Candidate]) → Result<Vec<(usize, f32)>>` — scores each candidate
- `ModelManager::ensure_model(name: &str) → Result<PathBuf>` — downloads model if absent, validates SHA-256
- `AugmentedSearch::run(query: &str, opts: &SearchOpts) → Result<Vec<AugmentedHit>>` — orchestrates full pipeline

#### Boundary Contracts

- `QueryExpansion` flows shell → core (produced by LLM, consumed by retrieval orchestrator)
- `Vec<RankedHit>` flows shell → core (produced by Tantivy/VectorIndex, consumed by weighted_rrf)
- `AugmentedHit` flows shell → caller (final output type with optional score trace)

#### Dependency Rule

Dependencies point inward: shell → core. Core MUST NOT import from shell. The `llama-cpp-2` dependency lives exclusively in the shell.

#### Enforcement

- `#[cfg(feature = "llm-search")]` gates all LLM inference code
- Module structure: `src/llm_search/core.rs` (pure), `src/llm_search/engine.rs` (LLM shell), `src/llm_search/models.rs` (model management), `src/llm_search/mod.rs` (pipeline orchestrator)
- Integration tests: `tests/llm_search_integration.rs` with `required-features = ["llm-search"]`

---

## 5. Contracts

### CON-039: `zetl search --augmented` CLI Contract

```
Endpoint: CLI subcommand
Command: zetl search --augmented <QUERY> [--limit N] [--path GLOB]
         [--focus PAGE] [--no-rerank] [--explain] [--model-dir PATH]

Pre-conditions:
  - QUERY is non-empty
  - Tantivy index and vector index exist at .zetl/search/ (built by `zetl index`)
  - Binary compiled with --features llm-search

Post-conditions:
  - Returns SearchOutput JSON with results ranked by the full augmented pipeline
  - Each result includes: page, path, heading (if chunk), score (final blended score)
  - If --explain: each result additionally includes score_trace object
  - If --no-rerank: reranker and graph boost stages are skipped
  - If --focus: graph boost is applied using the specified page as origin
  - Results capped at --limit (default 20)

Fallback behaviour:
  - Missing LLM models → falls back to --hybrid with warning on stderr
  - Expansion timeout → single-query hybrid (no expansion)
  - Reranker timeout (partial) → scored candidates reranked, remainder keeps RRF rank

Error model:
  - Missing search index → "Search index not found. Run `zetl index` first."
  - Feature not compiled → "Augmented search requires --features llm-search"
  - Empty query → "Empty search query"

Implements:
  - REQ-135
  - REQ-141
  - REQ-142
  - REQ-144
  - REQ-145

Verified by:
  - TEST-158
  - TEST-159
  - TEST-160
  - TEST-163
```

### CON-040: `AugmentedSearch` Internal API

```
Interface: src/llm_search/mod.rs

pub struct AugmentedSearch {
    expander: Option<LlmEngine>,
    reranker: Option<LlmEngine>,
    vector_index: VectorIndex,
    tantivy_index: TantivyIndex,
    graph: LinkGraph,
}

pub struct SearchOpts {
    pub limit: usize,
    pub focus: Option<String>,
    pub no_rerank: bool,
    pub explain: bool,
    pub path_glob: Option<String>,
}

pub struct AugmentedHit {
    pub page_name: String,
    pub path: String,
    pub heading: Option<String>,
    pub score: f64,
    pub score_trace: Option<ScoreTrace>,
}

pub struct ScoreTrace {
    pub bm25_rank: Option<u32>,
    pub vector_rank: Option<u32>,
    pub rrf_score: f64,
    pub reranker_score: Option<f64>,
    pub graph_distance: Option<u32>,
    pub graph_boost: f64,
    pub final_score: f64,
}

pub struct QueryExpansion {
    pub lex: Option<String>,
    pub vec: Option<String>,
    pub hyde: Option<String>,
}

impl AugmentedSearch {
    /// Build from existing indices and optional LLM engines.
    /// If LLM engines are None, pipeline falls back to hybrid-only.
    pub fn new(
        vector_index: VectorIndex,
        tantivy_index: TantivyIndex,
        graph: LinkGraph,
        model_dir: &Path,
    ) -> Result<Self>;

    /// Run the full augmented search pipeline.
    pub fn run(&self, query: &str, opts: &SearchOpts) -> Result<Vec<AugmentedHit>>;
}

Pre-conditions:
  - vector_index and tantivy_index are valid, open indices
  - graph is built from the current vault state
  - query is non-empty
  - opts.limit > 0

Post-conditions:
  - Returns ≤ opts.limit results sorted by descending final_score
  - If opts.explain is true, every result has Some(score_trace)
  - If opts.no_rerank is true, reranker_score is None in all traces
  - If opts.focus is Some, graph_distance and graph_boost are populated
  - If LLM engines failed to load, behaves identically to --hybrid

Implements:
  - REQ-135
  - REQ-136
  - REQ-137
  - REQ-138
  - REQ-139
  - REQ-140

Verified by:
  - TEST-161
  - TEST-162
  - TEST-164
```

---

## 6. Non-Functional Requirements

| ID      | Attribute              | Criterion |
|---------|------------------------|-----------|
| NFR-051 | End-to-end latency     | ≤ 2000ms for the full pipeline (expand + retrieve + rerank + blend) on a 5,000-page vault, single-threaded, CPU, Apple M-series |
| NFR-052 | Expansion latency      | ≤ 500ms for query expansion (3 variants from a single LLM call) |
| NFR-053 | Reranker latency       | ≤ 1200ms for reranking 30 candidates sequentially |
| NFR-054 | Model memory footprint | ≤ 2GB total RSS for both models loaded simultaneously (expansion ~1.2GB + reranker ~0.5GB at Q4 quantisation) |

### Latency Budget Breakdown

```
Stage                    Budget     Notes
─────────────────────────────────────────────────────
Query expansion          500ms      Single LLM call, ~100 output tokens
Multi-query retrieval    200ms      8 parallel retrievals (4 BM25 + 4 vector)
Multi-query RRF          5ms        Pure computation
LLM reranking            1200ms     30 candidates × ~40ms each
Graph BFS                5ms        BFS to depth 3, cached adjacency lists
Position-aware blend     1ms        Pure computation
─────────────────────────────────────────────────────
Total                    ~1911ms    Under 2000ms budget
```

---

## 7. Observability

| ID      | Signal | Type | Condition |
|---------|--------|------|-----------|
| OBS-030 | `[zetl] augmented-search: expansion_ms=N retrieval_ms=M rerank_ms=K total_ms=T` | Log (stderr) | Always when `--augmented` is used |
| OBS-031 | `[zetl] query-expansion: variants=N model=MODEL timeout=BOOL` | Log (stderr) | When `--verbose` flag is set |
| OBS-032 | `[zetl] reranker: scored=N/30 model=MODEL timeout=BOOL` | Log (stderr) | When `--verbose` flag is set |
| OBS-033 | `[zetl] graph-boost: focus=PAGE distances=[d1,d2,...] boosted=N` | Log (stderr) | When `--verbose` and `--focus` are both set |
| OBS-034 | `[zetl] model-download: name=NAME size_mb=S duration_s=D` | Log (stderr) | During model auto-download |

---

## 8. Architecture Decisions

### ADR-062: llama-cpp-2 Rust Bindings for Local LLM Inference

**Context**: The augmented search pipeline requires local LLM inference for query expansion and reranking. Options:

| Option | Description | Pros | Cons |
|--------|-------------|------|------|
| (a) `llama-cpp-2` | Rust bindings to llama.cpp via C FFI | Mature ecosystem, GGUF native, battle-tested quantisation, active maintenance, broad model support | C++ build dependency, binary size (~5-10MB for libllama) |
| (b) `candle` | Pure Rust ML framework (HuggingFace) | No C++ dependency, pure Rust, smaller binary | Less mature GGUF support, fewer quantisation options, slower inference for small models, no logprob extraction API at time of writing |
| (c) `mistral.rs` | Rust inference engine | Good performance, native Rust | Smaller community, fewer supported model architectures, heavier dependency |
| (d) External process | Spawn `llama-server` or `ollama` as a subprocess | No in-process dependency, model management handled externally | Violates single-binary principle, requires user to install external software, IPC overhead, process lifecycle management |

**Decision**: Use `llama-cpp-2` (Rust bindings to llama.cpp).

**Rationale**:

- **GGUF native**: llama.cpp is the reference implementation for GGUF model loading. The format is the de facto standard for local LLM inference, with thousands of quantised models available.
- **Quantisation quality**: llama.cpp supports Q4_K_M, Q5_K_M, Q8_0 and other quantisation levels with well-characterised quality/speed trade-offs. For our use case (short prompts, constrained output), Q4_K_M provides sufficient quality at ~40% of FP16 memory.
- **Logprob extraction**: llama.cpp exposes per-token logprobs, which is essential for the reranker's confidence scoring (REQ-138).
- **Battle-tested**: llama.cpp powers llama-server, ollama, LM Studio, and dozens of production systems. Performance bugs are caught quickly.
- **In-process**: The model loads into the Rust process via FFI. No IPC, no subprocess management, no port conflicts.
- **CPU performance**: llama.cpp includes hand-tuned SIMD kernels (AVX2, ARM NEON) for CPU inference. A 0.6B Q4 model runs at ~100 tokens/second on Apple M1, sufficient for our latency budgets.

**Trade-offs**:

- C++ build dependency via `cmake`. Mitigated by `llama-cpp-2`'s vendored build (automatic cmake invocation during `cargo build`).
- Binary size increases ~5-10MB. Acceptable given the existing ONNX Runtime dependency (~20-30MB) from SPEC-018.
- Model files are large (0.5-1.5GB). Mitigated by lazy download and user-level cache in `~/.cache/zetl/models/`.

**Risks**:

- llama.cpp API is not stable; breaking changes in upstream may require `llama-cpp-2` updates. Mitigated by pinning the `llama-cpp-2` version and wrapping all FFI calls behind an internal `LlmEngine` abstraction.
- Memory pressure from loading two models simultaneously. Mitigated by NFR-054 budget and the option to use a single model for both roles (see ADR-064).

### ADR-063: Model Selection — Separate Expander and Reranker

**Context**: The pipeline requires two capabilities: (1) text generation for query expansion, and (2) relevance scoring for reranking. Options:

| Option | Description | Trade-off |
|--------|-------------|-----------|
| (a) Single general model for both | Use one ~1.7B model for expansion and reranking | Simpler model management, but general models are weaker rerankers |
| (b) Two specialised models | Dedicated expander (~1.7B) + dedicated reranker (~0.6B) | Better quality per task, but higher memory and more models to manage |
| (c) Reranker-only (no expansion) | Skip expansion, use a reranker on top of SPEC-018 hybrid | Simpler pipeline, but misses the retrieval recall improvement from expansion |

**Decision**: Two specialised models (option b) as the default configuration, with fallback to single-model mode (option a) for memory-constrained environments.

**Default models**:

- **Expander**: A general-purpose instruction-following model, ~1.7B parameters, Q4_K_M quantisation (~1.0GB). Candidate: SmolLM2-1.7B-Instruct or Qwen2.5-1.5B-Instruct in GGUF format.
- **Reranker**: A model fine-tuned for relevance scoring, ~0.6B parameters, Q4_K_M quantisation (~0.4GB). Candidate: Qwen3-Reranker-0.6B in GGUF format (same architecture used by qmd).

**Rationale**:

- Reranker-specific models produce calibrated relevance scores that general models cannot match. The fine-tuning signal (relevance judgment pairs) is fundamentally different from general instruction-following.
- The 0.6B reranker is fast enough (~30-40ms per candidate) to score 30 candidates within the 1200ms budget.
- The 1.7B expander produces diverse, high-quality query reformulations while remaining CPU-feasible within the 500ms expansion budget.
- Combined memory (~1.4GB at Q4) fits within the 2GB budget with headroom for KV cache.

**Configuration override**: Users can specify alternative models via environment variables:

```
ZETL_EXPAND_MODEL=~/.cache/zetl/models/custom-expand.gguf
ZETL_RERANK_MODEL=~/.cache/zetl/models/custom-rerank.gguf
```

### ADR-064: Graceful Degradation and the Fallback Ladder

**Context**: The augmented pipeline has multiple points of failure: models not downloaded, inference timeout, parse failure. The system must never fail to return search results because of an LLM issue.

**Decision**: Implement a fallback ladder where each stage degrades independently:

```
Full pipeline (--augmented)
  │
  ├─ Expansion fails/times out
  │   └─► Single-query hybrid (SPEC-018 --hybrid)
  │        with reranking still applied to results
  │
  ├─ Reranker fails/times out (partial)
  │   └─► Scored candidates are reranked; remainder keeps RRF order
  │
  ├─ Reranker model not available
  │   └─► Multi-query RRF results without reranking
  │        (equivalent to --no-rerank)
  │
  ├─ Expansion model not available
  │   └─► Single-query hybrid + reranking
  │
  ├─ Both models not available
  │   └─► SPEC-018 --hybrid (pure RRF, no LLM)
  │        with warning: "LLM models not found. Falling back to hybrid search."
  │
  └─ Vector index not available
      └─► Error: "Search index not found. Run `zetl index` first."
```

**Rationale**: Users should never be surprised by a search failure caused by optional LLM components. The pipeline degrades in quality, not in availability. Warnings on stderr inform the user about degraded mode without polluting stdout (which carries JSON results for machine consumption).

---

## 9. Model Management

### 9.1 Model Manifest

A `manifest.json` file in the model directory tracks available models:

```json
{
  "models": [
    {
      "name": "zetl-expand-1.7b-q4",
      "filename": "zetl-expand-1.7b-q4.gguf",
      "sha256": "abc123...",
      "size_bytes": 1073741824,
      "url": "https://huggingface.co/zetl/zetl-expand-1.7b-q4/resolve/main/model.gguf",
      "role": "expander"
    },
    {
      "name": "zetl-rerank-0.6b-q4",
      "filename": "zetl-rerank-0.6b-q4.gguf",
      "sha256": "def456...",
      "size_bytes": 419430400,
      "url": "https://huggingface.co/zetl/zetl-rerank-0.6b-q4/resolve/main/model.gguf",
      "role": "reranker"
    }
  ]
}
```

### 9.2 Download Flow

```
zetl search --augmented "query"
  │
  ├─ ModelManager::ensure_model("zetl-expand-1.7b-q4")
  │   ├─ Check ~/.cache/zetl/models/zetl-expand-1.7b-q4.gguf exists
  │   ├─ If exists: validate SHA-256 → return path
  │   ├─ If missing: print "Downloading expansion model (1.0 GB)..." on stderr
  │   │              download with progress bar → validate SHA-256 → return path
  │   └─ If download fails: return None (triggers fallback per ADR-064)
  │
  └─ ModelManager::ensure_model("zetl-rerank-0.6b-q4")
      └─ (same flow)
```

### 9.3 Offline Mode

`ZETL_OFFLINE=1` disables all network access for model downloads. If a model is not already cached, the pipeline falls back per ADR-064 without attempting a download.

---

## 10. Test Specifications

| ID       | Description | Type | Traces |
|----------|-------------|------|--------|
| TEST-158 | `--augmented` returns results; top result for a conceptual query ("systems maintaining stability through feedback") matches across vocabulary boundaries where `--hybrid` alone does not surface all relevant pages | Integration | REQ-135, REQ-136 |
| TEST-159 | `--augmented --no-rerank` skips the reranker; results are ordered by multi-query RRF only; latency is measurably lower than with reranking | Integration | REQ-141 |
| TEST-160 | `--augmented --explain` includes a `score_trace` object on every result with all expected fields populated | Integration | REQ-142 |
| TEST-161 | `weighted_rrf` produces correct ordering for known inputs; original-query lists contribute 2x weight; all items from all input lists appear in output | Unit (property) | REQ-137 |
| TEST-162 | `position_aware_blend` applies correct weights at rank boundaries: rank 1 → 75/25, rank 5 → 60/40, rank 15 → 40/60 | Unit | REQ-139 |
| TEST-163 | `--augmented` with missing LLM models falls back to `--hybrid` behaviour and prints warning on stderr | Integration | REQ-145 |
| TEST-164 | `graph_boost` returns 1.3 for distance 1, 1.15 for distance 2, 1.1 for distance 3, 1.0 for distance ≥ 4 or None | Unit | REQ-140 |
| TEST-165 | `parse_expansion_output` correctly parses well-formed LEX/VEC/HYDE output; returns partial result for malformed output; returns empty for garbage | Unit (property) | REQ-136 |
| TEST-166 | `reranker_score_from_logprob` maps logprob 0.0 → ~0.5 (sigmoid midpoint), large positive → ~1.0, large negative → ~0.0 | Unit | REQ-138 |
| TEST-167 | Model download validates SHA-256; a file with wrong hash is rejected and re-downloaded | Integration | REQ-143 |
| TEST-168 | Compiling without `--features llm-search` produces no additional binary size from LLM dependencies; `--augmented` flag prints feature-not-enabled error | Build + Integration | REQ-144 |

### Verification Strategy

| System characteristic          | Technique |
|--------------------------------|-----------|
| Pure fusion/blend/boost functions | Property-based testing (TEST-161, TEST-162, TEST-164, TEST-165, TEST-166) |
| LLM expansion pipeline        | Integration testing with real model (TEST-158) |
| LLM reranker pipeline         | Integration testing with real model (TEST-158) |
| Fallback ladder                | Integration testing with model paths removed (TEST-163) |
| CLI contracts                  | Integration testing via `assert_cmd` (TEST-158–TEST-160) |
| Feature gating                 | Build-level verification (TEST-168) |
| Model management               | Integration testing with mock HTTP (TEST-167) |

---

## 11. Dependencies

### New Crate Dependencies (all behind `features = ["llm-search"]`)

| Crate | Purpose | Version |
|-------|---------|---------|
| `llama-cpp-2` | Rust bindings to llama.cpp for GGUF model inference | `0.1` |
| `reqwest` | HTTP client for model downloads (blocking, rustls) | `0.12` |
| `sha2` | SHA-256 validation of downloaded model files | `0.10` |
| `indicatif` | Progress bar for model downloads on stderr | `0.17` |

### Inherited Dependencies (from `semantic` feature, SPEC-018)

| Crate | Purpose |
|-------|---------|
| `ort` | ONNX Runtime for vector embeddings |
| `tokenizers` | HuggingFace tokenizer |
| `ndarray` | Tensor manipulation |

---

## 12. Web Integration

### Serve Mode (`zetl serve`)

- `GET /api/search?q=QUERY&mode=augmented&limit=N&focus=PAGE&no_rerank=BOOL` — augmented search
- `GET /api/search?q=QUERY&mode=augmented&explain=true` — with score traces
- Existing `mode=hybrid` and `mode=semantic` endpoints unchanged

### Build Mode (`zetl build`)

- Augmented search is not available in static builds (requires LLM inference at query time)
- A `llm_search_available: bool` template variable indicates whether augmented search was available at build time

---

## 13. Future Considerations

- **GPU acceleration**: llama.cpp supports CUDA and Metal backends. A future `--gpu` flag could enable GPU inference for faster reranking, making the pipeline viable for larger candidate sets (100+ candidates).
- **Adaptive query routing**: Use a lightweight classifier to predict whether a query benefits more from BM25, semantic, or augmented search, routing to the appropriate pipeline automatically.
- **Fine-tuned expansion model**: Train a zetl-specific expansion model on the user's vault content, producing reformulations that use the vault's actual vocabulary.
- **Cross-encoder reranking**: Replace the generative yes/no prompt with a true cross-encoder architecture (e.g., via ONNX export) for faster and more accurate reranking.
- **Shared KV cache**: For the reranker, the query portion of the prompt is identical across all 30 candidates. A shared KV cache for the query prefix would reduce reranking latency by ~30%.
- **Streaming results**: Return initial RRF results immediately, then update rankings as the reranker scores arrive, providing progressive refinement in the TUI.

---

## 14. Relationship to Existing Specs

- **SPEC-018** (Semantic Search): This spec extends SPEC-018's hybrid pipeline with LLM-augmented stages. The existing `--hybrid` mode is unchanged. `VectorIndex` and `reciprocal_rank_fusion` are reused directly.
- **SPEC-013** (Tantivy Search): BM25 retrieval and the Tantivy index are reused. Graph-scoped search via `--near` is orthogonal to `--focus` graph boosting — they can be combined (`--augmented --near cluster-root --focus current-page`).
- **SPEC-002** (Full-Text Search): The `SearchOutput` type is extended with optional `score_trace` field.
- **SPEC-001** (Link Graph): Graph boosting reuses `LinkGraph::bfs_neighbourhood` for distance computation.
- **SPEC-023** (Model Management): Model download and caching logic defined here may be extracted into SPEC-023 if other features (e.g., LLM-assisted tagging) also need local model inference.

See also: [[Spec Index]], [[Search Command]], [[Augmented Search]], [[ADR-062 LLM Inference Backend]]
