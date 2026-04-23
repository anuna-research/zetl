---
title: "SPEC-018: Semantic Search — Tantivy-Native Vector Embeddings with Hybrid BM25 Retrieval"
status: draft
version: 0.1.0
date: 2026-03-05
parent: SPEC-002
---

# SPEC-018: Semantic Search

Adds semantic (meaning-based) search to ztl by embedding vault content as vectors and combining cosine similarity with the existing BM25 keyword scoring. Complements SPEC-002's full-text search: BM25 finds exact terms, semantic search finds conceptually related content that shares no keywords.

```spl
(given spec-018-documented)
```

## Motivation

BM25 search (SPEC-002) returns results only when query terms appear literally in the document. A search for "concurrency" will not surface a note about "parallel task scheduling" unless it contains the word "concurrency." Users working across domains — connecting philosophy to programming, biology to economics — need search that understands meaning, not just tokens.

### User Stories

**Researcher connecting domains**: A user writes a note on "homeostasis in biological systems" and wants to find their earlier note on "PID controllers in engineering." BM25 returns nothing. Semantic search surfaces it because the underlying concept (feedback-driven equilibrium) is shared.

**Refactoring a vault**: A user searches "authentication" to consolidate scattered notes. BM25 misses notes titled "login flow," "session management," and "OAuth2 setup" that don't contain the literal word. Semantic search retrieves all of them.

**Agent-assisted discovery**: An LLM agent uses `ztl search --semantic` to find context for a user's question. The agent's natural-language query maps poorly to BM25 terms but maps well to vector space.

## Key Requirements

| ID | Requirement |
|----|-------------|
| REQ-092 | Generate 384-dimensional embeddings from vault page body text using a local ONNX model |
| REQ-093 | Store embeddings alongside the Tantivy index at `.ztl/search/vectors/` |
| REQ-094 | `ztl search --semantic <QUERY>` performs pure vector search, returning pages ranked by cosine similarity |
| REQ-095 | `ztl search --hybrid <QUERY>` fuses BM25 and vector results via reciprocal rank fusion |
| REQ-096 | Default `ztl search <QUERY>` remains BM25-only (no behaviour change to existing users) |
| REQ-097 | Embeddings are rebuilt during `ztl index`; stale embeddings are detected via content hash comparison |
| REQ-098 | The feature is gated behind `--features semantic` (compile-time opt-in); when the feature is disabled, `--semantic` and `--hybrid` flags produce a clear error message |
| REQ-099 | `ztl search --semantic` results include a normalised similarity score in `[0.0, 1.0]` |
| REQ-100 | Heading-level chunking: long pages are split at `## ` boundaries before embedding, with each chunk stored and retrievable independently |
| REQ-101 | `ztl search --hybrid` supports `--near` graph scoping (intersect fused results with the existing neighbourhood filter from SPEC-002) |

## Architecture

### Embedding Pipeline

```
                 ┌──────────────┐
  ParsedFile[] ──┤  chunk_page  ├──► Chunk[]
                 └──────────────┘
                        │
                        ▼
                 ┌──────────────┐
  Chunk[] ───────┤ embed_chunks ├──► (Chunk, Vec<f32>)[]
                 │  (ort ONNX)  │
                 └──────────────┘
                        │
                        ▼
                 ┌──────────────┐
                 │ VectorIndex  │
                 │  .build()    ├──► .ztl/search/vectors/
                 └──────────────┘
```

### Chunking Strategy (REQ-100)

Pages are split at `## ` (h2) heading boundaries. Each chunk inherits the page name and heading path. Short pages (< 512 tokens) are embedded as a single chunk. Each chunk carries:

- `page_name: String`
- `path: String`
- `heading: Option<String>` — the heading this chunk falls under
- `content_hash: [u8; 32]` — BLAKE3 hash of the chunk text, for incremental rebuild
- `embedding: Vec<f32>` — 384-dimensional normalised vector

### Hybrid Retrieval (REQ-095)

```
Query ──┬──► BM25 (tantivy)     ──► ranked list A
        │
        └──► Vector (cosine sim) ──► ranked list B
                                          │
                              ┌───────────┘
                              ▼
                     Reciprocal Rank Fusion
                     score(d) = Σ 1/(k + rank_i(d))
                     k = 60 (standard constant)
                              │
                              ▼
                     Fused ranked list ──► SearchOutput
```

RRF is chosen because it requires no weight tuning — it is parameter-free beyond `k`, which has a well-established default. BM25 and vector scores are on incomparable scales; rank-based fusion sidesteps normalisation issues entirely.

### Storage Layout

```
.ztl/
├── search/
│   ├── meta.json          (existing tantivy)
│   ├── *.fast, *.idx ...  (existing tantivy)
│   └── vectors/
│       ├── index.bin       (flat vector index, mmap-friendly)
│       ├── chunks.json     (chunk metadata: page, heading, content_hash)
│       └── model.json      (model name + dimension for validation)
```

### Purity Boundary Map

#### Pure Core (no I/O, no shared state, deterministic)

- `chunk_page(content, headings) → Vec<Chunk>` — splits body text at h2 boundaries
- `reciprocal_rank_fusion(list_a, list_b, k) → Vec<(page, score)>` — merges two ranked lists
- `cosine_similarity(a, b) → f32` — dot product of normalised vectors
- `detect_stale_chunks(old_hashes, new_hashes) → Vec<usize>` — compares content hashes

#### Effectful Shell (orchestrates I/O, calls pure core)

- `VectorIndex::build(vault_root, files)` — reads files, calls ONNX runtime, writes index
- `VectorIndex::open(vault_root)` — mmap reads index from disk
- `VectorIndex::query(embedding, limit)` — brute-force cosine scan over stored vectors
- `embed_chunks(chunks) → Vec<Vec<f32>>` — calls `ort` ONNX session

#### Boundary Contracts

- `Chunk` flows core → shell (produced by chunking, consumed by embedding)
- `Vec<f32>` flows shell → core (produced by ONNX, consumed by similarity/fusion)
- `SearchHit` flows shell → caller (existing type, extended with similarity score)

#### Dependency Rule

Dependencies point inward: shell → core. Core MUST NOT import from shell. The `ort` ONNX dependency lives exclusively in the shell.

#### Enforcement

- `#[cfg(feature = "semantic")]` gates all ONNX/vector code
- Module structure: `src/semantic/core.rs` (pure), `src/semantic/mod.rs` (effectful shell)
- Integration tests: `tests/semantic_integration.rs` with `required-features = ["semantic"]`

## Architecture Decisions

### ADR-051: Local ONNX Embedding via `ort` Crate

**Context**: Semantic search requires dense vector embeddings. Options: (a) external Python sidecar with sentence-transformers, (b) Rust-native `ort` crate wrapping ONNX Runtime, (c) `candle` (Rust-native ML framework), (d) cloud API calls.

**Decision**: Use `ort` (Rust bindings to ONNX Runtime) with the `all-MiniLM-L6-v2` model exported to ONNX format.

**Rationale**:
- **Local-first**: No network calls, no API keys, no privacy leakage. Consistent with ztl's design philosophy (files are truth, zero config, agent-friendly).
- **Single binary**: `ort` links ONNX Runtime statically or dynamically; no Python dependency.
- **Proven model**: `all-MiniLM-L6-v2` is 22MB (ONNX), 384 dimensions, ~50ms per chunk on CPU. Widely benchmarked for semantic similarity tasks.
- **Portable**: ONNX Runtime runs on macOS (ARM/x86), Linux, Windows.

**Trade-offs**:
- Binary size increases ~20-30MB (ONNX Runtime shared library).
- First build downloads the model or requires bundling. Mitigated by caching in `.ztl/models/`.
- `candle` would avoid the C++ dependency but has less mature tokenizer support and model ecosystem.
- Cloud APIs would give better embeddings (e.g., `text-embedding-3-small`) but violate local-first principle.

**Risks**:
- ONNX Runtime build complexity on some platforms. Mitigated by `ort`'s prebuilt binary downloads.
- Model quality may be insufficient for specialised domains. Mitigated by making the model path configurable (users can swap in a fine-tuned model).

### ADR-052: Brute-Force Vector Search Over ANN Index

**Context**: For querying vectors, options range from approximate nearest neighbour libraries (FAISS, usearch, hnsw_rs) to simple brute-force linear scan.

**Decision**: Start with brute-force cosine similarity scan. Add ANN indexing later if needed.

**Rationale**:
- Most personal vaults have < 10,000 pages → < 50,000 chunks. A brute-force scan of 50k × 384-dim vectors takes < 5ms on modern hardware (SIMD-accelerated dot products).
- ANN libraries add complexity, tuning parameters (ef, M for HNSW), and build-time dependencies for marginal latency gains at this scale.
- Brute-force is exact — no recall loss from approximation.
- If vaults grow to 100k+ chunks, ANN can be added as an optimisation without changing the API.

**Trade-offs**:
- Linear scan is O(n) per query vs O(log n) for HNSW. Acceptable at n < 100k.
- No indexing structure to maintain — rebuild is just "write all vectors to a flat file."

### ADR-053: Reciprocal Rank Fusion for Hybrid Search

**Context**: Combining BM25 scores (unbounded floats) with cosine similarity scores ([0,1]) requires a fusion strategy. Options: (a) linear combination with learned/tuned weights, (b) reciprocal rank fusion (RRF), (c) cascade (vector first, BM25 rerank).

**Decision**: Reciprocal Rank Fusion with k=60.

**Rationale**:
- RRF uses only rank position, not raw scores, so it works across incomparable scales without normalisation.
- Parameter-free in practice (k=60 is the standard default from the original Cormack et al. paper and performs well across domains).
- Simple to implement: ~20 lines of pure code.
- Well-studied in information retrieval; used by Elasticsearch, Weaviate, and others for hybrid search.

**Trade-offs**:
- Cannot express "trust BM25 more than vectors for this query." Acceptable because the user can choose `--semantic` or plain BM25 when they want single-signal search.
- A learned combiner could theoretically outperform RRF on domain-specific data, but requires training data we don't have.

### ADR-054: Heading-Level Chunking

**Context**: Embedding entire pages produces coarse-grained vectors that average over multiple topics. Options: (a) whole-page embedding, (b) fixed-length sliding window, (c) heading-based semantic chunking.

**Decision**: Split at `## ` (h2) heading boundaries. Pages shorter than 512 tokens are embedded whole.

**Rationale**:
- Headings are natural semantic boundaries in Zettelkasten-style notes. Users already structure notes with headings to delimit topics.
- Heading-based chunks are human-interpretable — search results can report "page X, section Y" rather than "page X, characters 1024–2048."
- Consistent with SPEC-002's heading-aware context (REQ-013-010, REQ-013-011) — reuses the existing `detect_headings` infrastructure.
- 512-token threshold avoids fragmenting short atomic notes that are already single-topic.

**Trade-offs**:
- Notes without headings become a single chunk, which may be too coarse. Acceptable because Zettelkasten notes are typically short and single-topic.
- H2 granularity may be too coarse for long sections. Users can add more headings to improve granularity.

## Contracts

### CON-028: `ztl search --semantic` CLI Contract

```
Endpoint: CLI subcommand
Command: ztl search --semantic <QUERY> [--limit N] [--path GLOB]

Pre-conditions:
  - QUERY is non-empty
  - Vector index exists at .ztl/search/vectors/ (built by `ztl index`)
  - Binary compiled with --features semantic

Post-conditions:
  - Returns SearchOutput JSON with results ranked by cosine similarity
  - Each result includes: page, path, heading (if chunk is sub-page), score ∈ [0.0, 1.0]
  - Results are capped at --limit (default 20)

Error model:
  - Missing vector index → "Vector index not found. Run `ztl index` first."
  - Feature not compiled → "Semantic search requires --features semantic"
  - Empty query → "Empty search query" (existing behaviour)

Implements:
  - REQ-094
  - REQ-098
  - REQ-099

Verified by:
  - TEST-114
  - TEST-115
```

### CON-029: `ztl search --hybrid` CLI Contract

```
Endpoint: CLI subcommand
Command: ztl search --hybrid <QUERY> [--limit N] [--near PAGE] [--depth N] [--path GLOB]

Pre-conditions:
  - QUERY is non-empty
  - Both tantivy index and vector index exist
  - Binary compiled with --features semantic

Post-conditions:
  - Returns SearchOutput JSON with results ranked by RRF score
  - Each result includes: page, path, line, column, context, heading, score
  - score is the RRF fusion score (not directly comparable to BM25 or cosine scores)
  - --near scoping applied after fusion (intersect fused results with neighbourhood)

Error model:
  - Missing either index → "Search index not found. Run `ztl index` first."
  - Feature not compiled → "Semantic search requires --features semantic"

Implements:
  - REQ-095
  - REQ-101

Verified by:
  - TEST-116
  - TEST-117
```

### CON-030: `VectorIndex` Internal API

```
Interface: src/semantic/mod.rs

pub struct VectorIndex { ... }

impl VectorIndex {
    /// Build vector index from parsed files. Embeds all chunks via ONNX.
    pub fn build(vault_root: &Path, files: &[ParsedFile]) -> Result<Self>;

    /// Open existing vector index from .ztl/search/vectors/.
    pub fn open(vault_root: &Path) -> Result<Option<Self>>;

    /// Query by embedding vector. Returns top-N chunks by cosine similarity.
    pub fn query(&self, embedding: &[f32], limit: usize) -> Result<Vec<VectorHit>>;

    /// Embed a query string. Handles tokenization internally.
    pub fn embed_query(&self, query: &str) -> Result<Vec<f32>>;
}

pub struct VectorHit {
    pub page_name: String,
    pub path: String,
    pub heading: Option<String>,
    pub score: f32,  // cosine similarity ∈ [0.0, 1.0]
}

Pre-conditions:
  - build: vault_root exists, files are valid ParsedFiles
  - open: vault_root exists
  - query: embedding.len() == 384, limit > 0
  - embed_query: query is non-empty

Post-conditions:
  - build: .ztl/search/vectors/ directory created with index.bin, chunks.json, model.json
  - open: returns None if vectors/ directory absent
  - query: returns ≤ limit hits sorted by descending score
  - embed_query: returns 384-dimensional normalised vector

Implements:
  - REQ-092
  - REQ-093

Verified by:
  - TEST-118
  - TEST-119
```

## Non-Functional Requirements

| ID | Attribute | Criterion |
|----|-----------|-----------|
| NFR-034 | Embedding latency | ≤ 100ms per chunk (single-threaded, CPU, Apple M-series) |
| NFR-035 | Index build time | ≤ 30s for 5,000 pages on CPU |
| NFR-036 | Query latency (vector) | ≤ 10ms for 50,000 chunks (brute-force scan) |
| NFR-037 | Query latency (hybrid) | ≤ 50ms total (BM25 + vector + fusion) for 50,000 chunks |
| NFR-038 | Vector index size | ≤ 80MB for 50,000 chunks (50k × 384 × 4 bytes = ~74MB) |
| NFR-039 | Model size | ≤ 30MB (ONNX export of all-MiniLM-L6-v2) |
| NFR-040 | Binary size increase | ≤ 40MB above baseline (ONNX Runtime library) |
| NFR-041 | Feature isolation | When compiled without `--features semantic`, zero binary size increase, zero additional dependencies |

## Observability

| ID | Signal | Type | Condition |
|----|--------|------|-----------|
| OBS-017 | `[ztl] embed: chunks=N duration_ms=M model=all-MiniLM-L6-v2` | Log (stderr) | Always during `ztl index` when semantic feature is enabled |
| OBS-018 | `[ztl] vector-query: chunks_scanned=N results=M duration_ms=K` | Log (stderr) | When `--verbose` flag is set on search |
| OBS-019 | `[ztl] hybrid-fusion: bm25_results=A vector_results=B fused=C duration_ms=D` | Log (stderr) | When `--verbose` flag is set on hybrid search |
| OBS-020 | `semantic` field in `ztl stats` JSON output | Metric | When semantic feature is compiled in; fields: chunk_count, index_size_mb, model_name |

## Test Specifications

| ID | Description | Type | Traces |
|----|-------------|------|--------|
| TEST-114 | `--semantic` returns results ranked by cosine similarity; top result for "feedback equilibrium" matches a note about PID controllers over a note about unrelated topic | Integration | REQ-094, REQ-099 |
| TEST-115 | `--semantic` with missing vector index prints error and exits non-zero | Integration | REQ-098 |
| TEST-116 | `--hybrid` returns results that combine BM25 keyword matches with semantic matches; a note matching both signals ranks higher than notes matching only one | Integration | REQ-095 |
| TEST-117 | `--hybrid --near` restricts fused results to the graph neighbourhood | Integration | REQ-101 |
| TEST-118 | `VectorIndex::build` creates vectors/ directory with expected files; `VectorIndex::open` reads it back; round-trip produces identical query results | Integration | REQ-092, REQ-093 |
| TEST-119 | `VectorIndex::query` returns ≤ limit results sorted by descending score | Unit | CON-030 |
| TEST-120 | `chunk_page` splits at h2 boundaries; pages under 512 tokens remain whole | Unit (property: roundtrip of join(chunks) == original body text) | REQ-100, ADR-054 |
| TEST-121 | `reciprocal_rank_fusion` produces correct ordering for known inputs; property: all items from both input lists appear in output | Unit (property) | ADR-053 |
| TEST-122 | Compiling without `--features semantic` produces no additional binary size; `--semantic` flag prints feature-not-enabled error | Build + Integration | REQ-098, NFR-041 |
| TEST-123 | Incremental rebuild: changing one page re-embeds only that page's chunks (detected via content hash comparison) | Integration | REQ-097 |

### Verification Strategy

| System characteristic | Technique |
|-----------------------|-----------|
| Pure fusion/chunking functions | Property-based testing (TEST-120, TEST-121) |
| ONNX embedding pipeline | Integration testing with real model (TEST-118) |
| CLI contracts | Integration testing via `assert_cmd` (TEST-114–TEST-117) |
| Feature gating | Build-level verification (TEST-122) |
| Incremental rebuild | Integration testing with hash comparison (TEST-123) |

## Dependencies

### New Crate Dependencies (all behind `features = ["semantic"]`)

| Crate | Purpose | Version |
|-------|---------|---------|
| `ort` | ONNX Runtime Rust bindings | `2` |
| `tokenizers` | HuggingFace tokenizer for text → token IDs | `0.21` |
| `ndarray` | Tensor manipulation for ONNX I/O | `0.16` |

### Model Acquisition

The ONNX model file is NOT bundled in the binary. On first `ztl index` with `--features semantic`:

1. Check `.ztl/models/all-MiniLM-L6-v2.onnx`
2. If absent, download from HuggingFace Hub (with user confirmation)
3. Validate SHA-256 hash
4. Cache permanently in `.ztl/models/`

Alternative: user sets `ztl_MODEL_PATH` environment variable to a local ONNX file.

## Incremental Rebuild (REQ-097)

```
For each page:
  1. Chunk the page body text
  2. BLAKE3 hash each chunk
  3. Compare against hashes in chunks.json
  4. Re-embed only chunks whose hash changed
  5. Write updated vectors and metadata
```

This avoids re-embedding the entire vault on every `ztl index`. Only changed or new content pays the embedding cost.

## Web Integration

### Serve Mode (`ztl serve`)

- `GET /api/search?q=QUERY&mode=semantic&limit=N` — vector search
- `GET /api/search?q=QUERY&mode=hybrid&limit=N` — hybrid search
- `GET /api/search?q=QUERY&limit=N` — BM25 (existing, default, unchanged)

The `mode` parameter is optional; omitting it preserves existing BM25 behaviour.

### Build Mode (`ztl build`)

- Vector index is not exported to static output (too large, not useful without a runtime)
- BM25 index export (existing `bm25_index` template variable) is unchanged
- A `semantic_available: bool` template variable indicates whether semantic search was available at build time

## Future Considerations

- **Fine-tuned models**: Users with domain-specific vocabularies could train and swap in custom ONNX models via `ztl_MODEL_PATH`.
- **ANN indexing**: If brute-force becomes too slow at scale (> 100k chunks), add HNSW via `usearch` or `hnsw_rs` behind the same `VectorIndex` API (ADR-052).
- **Cross-vault federation**: Semantic search across federated vaults (SPEC-004) would require a shared embedding space (same model).
- **Similarity command**: `ztl similar` currently uses SimHash on page names. A future enhancement could add `ztl similar --semantic <PAGE>` that uses the page's embedding to find conceptually similar pages.

## Relationship to Existing Specs

- **SPEC-002** (Full-Text Search): This spec extends SPEC-002 with a new retrieval signal. BM25 behaviour is unchanged. The `SearchOutput` type is reused.
- **SPEC-001** (Link Graph): The `--near` graph scoping (REQ-101) reuses the neighbourhood calculation from SPEC-001/SPEC-002.
- **SPEC-006** (Merkle Tree): Content hashing for incremental rebuild (REQ-097) reuses BLAKE3, consistent with SPEC-006's content-addressing.

See also: [[Spec Index]], [[Search Command]], [[ADR-002 Search Without Index]]
