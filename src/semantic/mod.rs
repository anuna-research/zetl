//! Semantic search: ONNX-backed vector embeddings and hybrid BM25 retrieval (SPEC-018).
//!
//! This module is compiled only when `--features semantic` is active (REQ-098, NFR-041).
//! All ONNX/vector dependencies (`ort`, `tokenizers`, `ndarray`) are declared as optional
//! in `Cargo.toml` and gated behind this feature, ensuring zero binary size increase
//! and zero additional dependencies when the feature is absent.
//!
//! ## Module layout
//!
//! - `core` — pure functions: chunking, cosine similarity, RRF, stale-chunk detection
//! - `mod`  — effectful shell: `VectorIndex` (ONNX I/O, disk persistence)

pub mod core;

use std::cell::RefCell;
use std::path::Path;

use anyhow::Result;
use ort::session::{Session, builder::GraphOptimizationLevel};
use ort::value::TensorRef;
use tokenizers::Tokenizer;

pub use core::{Chunk, VectorHit};

/// Embedding dimension for `all-MiniLM-L6-v2`. REQ-092.
pub const EMBEDDING_DIM: usize = 384;

/// Default chunking threshold (bytes). Pages smaller than this are not split.
/// Approximates 512 tokens at ~4 bytes/token. ADR-054.
pub const CHUNK_THRESHOLD: usize = 512 * 4;

/// RRF constant `k`. ADR-053.
pub const RRF_K: usize = 60;

/// On-disk layout under `.zetl/search/vectors/`. REQ-093.
pub const VECTORS_DIR: &str = ".zetl/search/vectors";
pub const INDEX_FILE: &str = "index.bin";
pub const CHUNKS_FILE: &str = "chunks.json";
pub const MODEL_FILE: &str = "model.json";

/// HuggingFace model identifier used for download and validation. ADR-051.
pub const MODEL_NAME: &str = "all-MiniLM-L6-v2";

/// ONNX-backed vector index for semantic search.
///
/// The index stores 384-dimensional normalised embeddings produced by
/// `all-MiniLM-L6-v2` alongside chunk metadata. Queries are resolved via
/// brute-force cosine similarity scan (ADR-052).
pub struct VectorIndex {
    /// Flat vector store: `embeddings[i]` is the 384-dim embedding for `chunks[i]`.
    embeddings: Vec<[f32; EMBEDDING_DIM]>,
    /// Chunk metadata parallel to `embeddings`.
    chunks: Vec<ChunkMeta>,
    /// ONNX inference session (shared across queries). RefCell for interior mutability
    /// since ort v2 `Session::run` requires `&mut self`.
    session: RefCell<Session>,
    /// HuggingFace tokenizer.
    tokenizer: Tokenizer,
}

/// Serialisable chunk metadata stored in `chunks.json`. REQ-093.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChunkMeta {
    pub page_name: String,
    pub path: String,
    pub heading: Option<String>,
    pub content_hash: [u8; 32],
}

impl VectorIndex {
    /// Build a vector index from a set of parsed files.
    ///
    /// Loads or downloads the ONNX model, embeds all chunks, and writes the index
    /// to `.zetl/search/vectors/`. REQ-092, REQ-093, REQ-097.
    pub fn build(vault_root: &Path, files: &[crate::types::ParsedFile]) -> Result<Self> {
        let vectors_dir = vault_root.join(VECTORS_DIR);
        std::fs::create_dir_all(&vectors_dir)?;

        let (session, tokenizer) = load_model(vault_root)?;
        let session = RefCell::new(session);

        let mut all_embeddings: Vec<[f32; EMBEDDING_DIM]> = Vec::new();
        let mut all_meta: Vec<ChunkMeta> = Vec::new();

        let start = std::time::Instant::now();
        let mut chunk_count = 0usize;

        for file in files {
            let file_path = vault_root.join(&file.path);
            let content = std::fs::read_to_string(&file_path)?;
            let path_str = file.path.to_string_lossy().into_owned();

            let headings: Vec<(usize, u8, String)> = {
                use crate::scanner::body_text_ranges;
                use crate::search::detect_headings;
                let ranges = body_text_ranges(&content);
                detect_headings(&content, &ranges)
                    .into_iter()
                    .map(|h| (h.byte_offset, h.level, h.text))
                    .collect()
            };

            let chunks = core::chunk_page(
                &file.page_name,
                &path_str,
                &content,
                &headings,
                CHUNK_THRESHOLD,
            );

            for chunk in &chunks {
                let embedding = embed_text(&session, &tokenizer, &chunk.text)?;
                all_meta.push(ChunkMeta {
                    page_name: chunk.page_name.clone(),
                    path: chunk.path.clone(),
                    heading: chunk.heading.clone(),
                    content_hash: chunk.content_hash,
                });
                all_embeddings.push(embedding);
                chunk_count += 1;
            }
        }

        let duration_ms = start.elapsed().as_millis();
        eprintln!(
            "[zetl] embed: chunks={chunk_count} duration_ms={duration_ms} model={MODEL_NAME}"
        );

        // Persist index.
        let index_path = vectors_dir.join(INDEX_FILE);
        let raw: Vec<u8> = all_embeddings
            .iter()
            .flat_map(|e| e.iter().flat_map(|f| f.to_le_bytes()))
            .collect();
        std::fs::write(&index_path, &raw)?;

        let chunks_path = vectors_dir.join(CHUNKS_FILE);
        std::fs::write(&chunks_path, serde_json::to_string_pretty(&all_meta)?)?;

        let model_path = vectors_dir.join(MODEL_FILE);
        std::fs::write(
            &model_path,
            serde_json::json!({ "model": MODEL_NAME, "dim": EMBEDDING_DIM }).to_string(),
        )?;

        Ok(VectorIndex {
            embeddings: all_embeddings,
            chunks: all_meta,
            session,
            tokenizer,
        })
    }

    /// Open an existing vector index from `.zetl/search/vectors/`.
    ///
    /// Returns `None` if the vectors directory does not exist. REQ-093.
    pub fn open(vault_root: &Path) -> Result<Option<Self>> {
        let vectors_dir = vault_root.join(VECTORS_DIR);
        if !vectors_dir.exists() {
            return Ok(None);
        }

        let index_path = vectors_dir.join(INDEX_FILE);
        let chunks_path = vectors_dir.join(CHUNKS_FILE);
        if !index_path.exists() || !chunks_path.exists() {
            return Ok(None);
        }

        let raw = std::fs::read(&index_path)?;
        let chunk_count = raw.len() / (EMBEDDING_DIM * 4);
        let mut embeddings = Vec::with_capacity(chunk_count);
        for i in 0..chunk_count {
            let mut arr = [0f32; EMBEDDING_DIM];
            for (j, f) in arr.iter_mut().enumerate() {
                let off = (i * EMBEDDING_DIM + j) * 4;
                *f = f32::from_le_bytes(raw[off..off + 4].try_into()?);
            }
            embeddings.push(arr);
        }

        let chunks: Vec<ChunkMeta> =
            serde_json::from_str(&std::fs::read_to_string(&chunks_path)?)?;

        let (session, tokenizer) = load_model(vault_root)?;

        Ok(Some(VectorIndex {
            embeddings,
            chunks,
            session: RefCell::new(session),
            tokenizer,
        }))
    }

    /// Query the index with a string. Embeds the query and returns top-N hits by cosine similarity.
    ///
    /// REQ-094, REQ-099, CON-030.
    pub fn query_text(&self, query: &str, limit: usize) -> Result<Vec<VectorHit>> {
        let q_emb = embed_text(&self.session, &self.tokenizer, query)?;
        self.query(&q_emb, limit)
    }


    /// Query by a pre-computed embedding vector. Returns top-N chunks by cosine similarity.
    ///
    /// CON-030.
    pub fn query(&self, embedding: &[f32; EMBEDDING_DIM], limit: usize) -> Result<Vec<VectorHit>> {
        let start = std::time::Instant::now();

        let mut scored: Vec<(f32, usize)> = self
            .embeddings
            .iter()
            .enumerate()
            .map(|(i, e)| (core::cosine_similarity(e, embedding), i))
            .collect();

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);

        let results: Vec<VectorHit> = scored
            .into_iter()
            .map(|(score, i)| {
                let meta = &self.chunks[i];
                VectorHit {
                    page_name: meta.page_name.clone(),
                    path: meta.path.clone(),
                    heading: meta.heading.clone(),
                    score,
                }
            })
            .collect();

        let duration_ms = start.elapsed().as_millis();
        // OBS-018: emitted when --verbose; caller checks verbosity.
        let _ = (duration_ms, self.embeddings.len()); // used in verbose path

        Ok(results)
    }

    /// Emit OBS-018 timing line to stderr.
    pub fn log_query_stats(&self, results: usize, duration_ms: u128) {
        eprintln!(
            "[zetl] vector-query: chunks_scanned={} results={results} duration_ms={duration_ms}",
            self.embeddings.len()
        );
    }

    /// Number of chunks stored in the index.
    pub fn chunk_count(&self) -> usize {
        self.embeddings.len()
    }

    /// Return per-chunk content hashes for incremental rebuild (REQ-097).
    pub fn content_hashes(&self) -> Vec<[u8; 32]> {
        self.chunks.iter().map(|c| c.content_hash).collect()
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Load the ONNX session and HuggingFace tokenizer.
///
/// The model is expected at `.zetl/models/all-MiniLM-L6-v2.onnx`. If absent,
/// an informative error is returned (download is the caller's responsibility).
fn load_model(vault_root: &Path) -> Result<(Session, Tokenizer)> {
    let model_path = vault_root
        .join(".zetl")
        .join("models")
        .join(format!("{MODEL_NAME}.onnx"));

    if !model_path.exists() {
        anyhow::bail!(
            "ONNX model not found at {}. \
             Download it with: zetl index (will prompt to download automatically).",
            model_path.display()
        );
    }

    let session = Session::builder()?
        .with_optimization_level(GraphOptimizationLevel::Level3)?
        .commit_from_file(&model_path)?;

    let tokenizer_path = vault_root
        .join(".zetl")
        .join("models")
        .join(format!("{MODEL_NAME}-tokenizer.json"));

    let tokenizer = if tokenizer_path.exists() {
        Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| anyhow::anyhow!("Failed to load tokenizer: {e}"))?
    } else {
        anyhow::bail!(
            "Tokenizer not found at {}. Run `zetl index` to download it.",
            tokenizer_path.display()
        );
    };

    Ok((session, tokenizer))
}

/// Tokenize `text` and run a single ONNX inference pass, returning a
/// normalised 384-dimensional embedding.
fn embed_text(
    session: &RefCell<Session>,
    tokenizer: &Tokenizer,
    text: &str,
) -> Result<[f32; EMBEDDING_DIM]> {
    let encoding = tokenizer
        .encode(text, true)
        .map_err(|e| anyhow::anyhow!("Tokenization failed: {e}"))?;

    let ids: Vec<i64> = encoding.get_ids().iter().map(|&x| x as i64).collect();
    let mask: Vec<i64> = encoding
        .get_attention_mask()
        .iter()
        .map(|&x| x as i64)
        .collect();
    let type_ids: Vec<i64> = encoding
        .get_type_ids()
        .iter()
        .map(|&x| x as i64)
        .collect();
    let seq_len = ids.len();

    let shape = [1i64, seq_len as i64];
    let t_ids = TensorRef::<i64>::from_array_view((shape, ids.as_slice()))
        .map_err(|e| anyhow::anyhow!("Failed to create input_ids tensor: {e}"))?;
    let t_mask = TensorRef::<i64>::from_array_view((shape, mask.as_slice()))
        .map_err(|e| anyhow::anyhow!("Failed to create attention_mask tensor: {e}"))?;
    let t_type = TensorRef::<i64>::from_array_view((shape, type_ids.as_slice()))
        .map_err(|e| anyhow::anyhow!("Failed to create token_type_ids tensor: {e}"))?;

    let mut session_guard = session.borrow_mut();
    let outputs = session_guard.run(ort::inputs![t_ids, t_mask, t_type])?;

    // The first output is the last hidden state; mean-pool across the sequence dimension.
    let (out_shape, data) = outputs[0]
        .try_extract_tensor::<f32>()
        .map_err(|e| anyhow::anyhow!("Failed to extract tensor: {e}"))?;
    // Shape: (1, seq_len, 384) → mean over dim 1.
    let dims: &[i64] = &out_shape;
    anyhow::ensure!(dims.len() == 3, "expected 3-D output, got {}D", dims.len());
    let hidden_dim = dims[2] as usize;
    anyhow::ensure!(hidden_dim == EMBEDDING_DIM, "unexpected embedding dim {hidden_dim}");

    let mut embedding = [0f32; EMBEDDING_DIM];
    for j in 0..EMBEDDING_DIM {
        let sum: f32 = (0..seq_len).map(|t| data[t * EMBEDDING_DIM + j]).sum();
        embedding[j] = sum / seq_len as f32;
    }

    // L2-normalise so cosine similarity = dot product.
    let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in embedding.iter_mut() {
            *x /= norm;
        }
    }

    Ok(embedding)
}
