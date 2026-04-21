//! Semantic search: ONNX-backed vector embeddings and hybrid BM25 retrieval (SPEC-018).
//!
//! This module is compiled only when `--features semantic` is active (REQ-098, NFR-041).
//! All ONNX/vector dependencies (`ort`, `tokenizers`, `ndarray`) are declared as optional
//! in `Cargo.toml` and gated behind this feature, ensuring zero binary size increase
//! and zero additional dependencies when the feature is absent.
//!
//! ## Module layout
//!
//! - `acquisition` — model download from HuggingFace Hub with SHA-256 validation
//! - `core` — pure functions: chunking, cosine similarity, RRF, stale-chunk detection
//! - `mod`  — effectful shell: `VectorIndex` (ONNX I/O, disk persistence)

pub mod acquisition;
pub mod core;

use std::cell::RefCell;
use std::path::Path;

use anyhow::Result;
use ort::session::{builder::GraphOptimizationLevel, Session};
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
    /// to `.zetl/search/vectors/`. Reuses embeddings from any existing index for
    /// chunks whose BLAKE3 content hash is unchanged (REQ-097 incremental rebuild).
    ///
    /// REQ-092, REQ-093, REQ-097.
    pub fn build(vault_root: &Path, files: &[crate::types::ParsedFile]) -> Result<Self> {
        let vectors_dir = vault_root.join(VECTORS_DIR);
        std::fs::create_dir_all(&vectors_dir)?;

        // Load the existing embedding cache before initialising the ONNX session so that
        // builds with no stale chunks avoid loading the model entirely (fast path).
        let cache = load_embedding_cache(&vectors_dir);

        let (session, tokenizer) = load_model(vault_root)?;
        let session = RefCell::new(session);

        let mut all_embeddings: Vec<[f32; EMBEDDING_DIM]> = Vec::new();
        let mut all_meta: Vec<ChunkMeta> = Vec::new();

        let start = std::time::Instant::now();
        let mut embedded_count = 0usize;
        let mut reused_count = 0usize;

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
                let embedding = if let Some(&cached) = cache.get(&chunk.content_hash) {
                    // Chunk content unchanged — reuse stored embedding. REQ-097.
                    reused_count += 1;
                    cached
                } else {
                    // Chunk is new or stale — embed via ONNX.
                    embedded_count += 1;
                    embed_text(&session, &tokenizer, &chunk.text)?
                };
                all_meta.push(ChunkMeta {
                    page_name: chunk.page_name.clone(),
                    path: chunk.path.clone(),
                    heading: chunk.heading.clone(),
                    content_hash: chunk.content_hash,
                });
                all_embeddings.push(embedding);
            }
        }

        let duration_ms = start.elapsed().as_millis();
        eprintln!(
            "[zetl] embed: chunks={} reused={reused_count} embedded={embedded_count} duration_ms={duration_ms} model={MODEL_NAME}",
            all_embeddings.len(),
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

        let chunks: Vec<ChunkMeta> = serde_json::from_str(&std::fs::read_to_string(&chunks_path)?)?;

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
        let scored = core::vector_search(&self.embeddings, embedding, limit);

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

        Ok(results)
    }

    /// Embed a query string and return its normalised 384-dimensional vector.
    ///
    /// Useful when the caller needs the raw embedding (e.g. for caching or
    /// passing to `query` directly). REQ-099.
    pub fn embed_query(&self, query: &str) -> Result<[f32; EMBEDDING_DIM]> {
        embed_text(&self.session, &self.tokenizer, query)
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

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// TEST-114: index.bin round-trip — write N embeddings as little-endian f32, read back
    /// identical values. Verifies the flat-binary layout used by `VectorIndex::build`/`open`.
    #[test]
    fn test_index_bin_roundtrip() {
        let tmp = tempfile::TempDir::new().unwrap();
        let vectors_dir = tmp.path().join(VECTORS_DIR);
        fs::create_dir_all(&vectors_dir).unwrap();

        // Build 3 embeddings with distinct values.
        let mut embeddings: Vec<[f32; EMBEDDING_DIM]> = Vec::new();
        for i in 0..3usize {
            let mut arr = [0.0f32; EMBEDDING_DIM];
            arr[0] = i as f32 + 0.5;
            arr[EMBEDDING_DIM - 1] = -(i as f32);
            embeddings.push(arr);
        }

        // Write (same code path as VectorIndex::build).
        let raw: Vec<u8> = embeddings
            .iter()
            .flat_map(|e| e.iter().flat_map(|f| f.to_le_bytes()))
            .collect();
        let index_path = vectors_dir.join(INDEX_FILE);
        fs::write(&index_path, &raw).unwrap();

        // Read back (same code path as VectorIndex::open).
        let read_raw = fs::read(&index_path).unwrap();
        let chunk_count = read_raw.len() / (EMBEDDING_DIM * 4);
        assert_eq!(chunk_count, 3, "expected 3 chunks");

        let mut read_embeddings: Vec<[f32; EMBEDDING_DIM]> = Vec::with_capacity(chunk_count);
        for i in 0..chunk_count {
            let mut arr = [0f32; EMBEDDING_DIM];
            for (j, f) in arr.iter_mut().enumerate() {
                let off = (i * EMBEDDING_DIM + j) * 4;
                *f = f32::from_le_bytes(read_raw[off..off + 4].try_into().unwrap());
            }
            read_embeddings.push(arr);
        }

        for (orig, back) in embeddings.iter().zip(read_embeddings.iter()) {
            for (a, b) in orig.iter().zip(back.iter()) {
                assert!(
                    (a - b).abs() < f32::EPSILON,
                    "f32 value mismatch: {a} vs {b}"
                );
            }
        }
    }

    /// TEST-115: chunks.json round-trip — serialise a `Vec<ChunkMeta>`, write to disk,
    /// deserialise, and verify all fields (including optional heading and content_hash).
    #[test]
    fn test_chunks_json_roundtrip() {
        let tmp = tempfile::TempDir::new().unwrap();
        let vectors_dir = tmp.path().join(VECTORS_DIR);
        fs::create_dir_all(&vectors_dir).unwrap();

        let chunks = vec![
            ChunkMeta {
                page_name: "page-alpha".to_string(),
                path: "notes/page-alpha.md".to_string(),
                heading: Some("Overview".to_string()),
                content_hash: [42u8; 32],
            },
            ChunkMeta {
                page_name: "page-beta".to_string(),
                path: "notes/page-beta.md".to_string(),
                heading: None,
                content_hash: [7u8; 32],
            },
        ];

        let chunks_path = vectors_dir.join(CHUNKS_FILE);
        fs::write(&chunks_path, serde_json::to_string_pretty(&chunks).unwrap()).unwrap();

        let read: Vec<ChunkMeta> =
            serde_json::from_str(&fs::read_to_string(&chunks_path).unwrap()).unwrap();

        assert_eq!(read.len(), 2);
        assert_eq!(read[0].page_name, "page-alpha");
        assert_eq!(read[0].path, "notes/page-alpha.md");
        assert_eq!(read[0].heading, Some("Overview".to_string()));
        assert_eq!(read[0].content_hash, [42u8; 32]);
        assert_eq!(read[1].page_name, "page-beta");
        assert_eq!(read[1].heading, None);
        assert_eq!(read[1].content_hash, [7u8; 32]);
    }

    /// TEST-116: model.json round-trip — write `{ "model": MODEL_NAME, "dim": EMBEDDING_DIM }`,
    /// read back and verify both fields are preserved correctly.
    #[test]
    fn test_model_json_roundtrip() {
        let tmp = tempfile::TempDir::new().unwrap();
        let vectors_dir = tmp.path().join(VECTORS_DIR);
        fs::create_dir_all(&vectors_dir).unwrap();

        let model_path = vectors_dir.join(MODEL_FILE);
        let json = serde_json::json!({ "model": MODEL_NAME, "dim": EMBEDDING_DIM });
        fs::write(&model_path, json.to_string()).unwrap();

        let read: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&model_path).unwrap()).unwrap();
        assert_eq!(read["model"].as_str().unwrap(), MODEL_NAME);
        assert_eq!(read["dim"].as_u64().unwrap(), EMBEDDING_DIM as u64);
    }

    /// TEST-117: `VectorIndex::open` returns `None` when the vectors directory is absent.
    /// Exercises the fast-path early exit before any file I/O or model loading.
    #[test]
    fn test_open_none_when_dir_absent() {
        let tmp = tempfile::TempDir::new().unwrap();
        // No .zetl/search/vectors/ created.
        let result = VectorIndex::open(tmp.path()).unwrap();
        assert!(
            result.is_none(),
            "expected None when vectors dir is missing"
        );
    }

    /// TEST-117: `VectorIndex::open` returns `None` when `index.bin` is absent even if
    /// `chunks.json` exists. Both files are required for a valid index.
    #[test]
    fn test_open_none_when_index_bin_absent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let vectors_dir = tmp.path().join(VECTORS_DIR);
        fs::create_dir_all(&vectors_dir).unwrap();
        // Write chunks.json but not index.bin.
        let empty: Vec<ChunkMeta> = vec![];
        fs::write(
            vectors_dir.join(CHUNKS_FILE),
            serde_json::to_string_pretty(&empty).unwrap(),
        )
        .unwrap();

        let result = VectorIndex::open(tmp.path()).unwrap();
        assert!(result.is_none(), "expected None when index.bin is missing");
    }

    /// TEST-123: incremental rebuild — `load_embedding_cache` returns a hash→embedding map
    /// from an existing on-disk index. Chunks whose BLAKE3 hash is present in the cache are
    /// reused without re-embedding; absent hashes produce no entry (caller embeds them fresh).
    ///
    /// This test verifies the detection mechanism (REQ-097) independently of the ONNX runtime.
    #[test]
    fn test_load_embedding_cache_hit_and_miss() {
        let tmp = tempfile::TempDir::new().unwrap();
        let vectors_dir = tmp.path().join(VECTORS_DIR);
        fs::create_dir_all(&vectors_dir).unwrap();

        let hash_a = [0xAAu8; 32];
        let hash_b = [0xBBu8; 32];

        let mut emb_a = [0.0f32; EMBEDDING_DIM];
        emb_a[0] = 1.0;
        let mut emb_b = [0.0f32; EMBEDDING_DIM];
        emb_b[0] = 2.0;

        let embeddings = vec![emb_a, emb_b];
        let chunks = vec![
            ChunkMeta {
                page_name: "page-a".to_string(),
                path: "page-a.md".to_string(),
                heading: None,
                content_hash: hash_a,
            },
            ChunkMeta {
                page_name: "page-b".to_string(),
                path: "page-b.md".to_string(),
                heading: Some("Section".to_string()),
                content_hash: hash_b,
            },
        ];

        let raw: Vec<u8> = embeddings
            .iter()
            .flat_map(|e| e.iter().flat_map(|f| f.to_le_bytes()))
            .collect();
        fs::write(vectors_dir.join(INDEX_FILE), &raw).unwrap();
        fs::write(
            vectors_dir.join(CHUNKS_FILE),
            serde_json::to_string_pretty(&chunks).unwrap(),
        )
        .unwrap();

        let cache = load_embedding_cache(&vectors_dir);

        assert!(cache.contains_key(&hash_a), "hash_a should be in cache");
        assert!(cache.contains_key(&hash_b), "hash_b should be in cache");
        assert!((cache[&hash_a][0] - 1.0).abs() < f32::EPSILON);
        assert!((cache[&hash_b][0] - 2.0).abs() < f32::EPSILON);

        let hash_c = [0xCCu8; 32];
        assert!(
            !cache.contains_key(&hash_c),
            "unknown hash should not be in cache"
        );
    }

    /// TEST-123 (edge case): `load_embedding_cache` returns an empty map when files are absent.
    #[test]
    fn test_load_embedding_cache_empty_when_absent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let vectors_dir = tmp.path().join(VECTORS_DIR);
        fs::create_dir_all(&vectors_dir).unwrap();
        let cache = load_embedding_cache(&vectors_dir);
        assert!(
            cache.is_empty(),
            "cache must be empty when files are missing"
        );
    }

    /// TEST-123 (edge case): `load_embedding_cache` returns an empty map when chunk count
    /// does not match embedding count (corrupt index). Caller will re-embed everything.
    #[test]
    fn test_load_embedding_cache_corrupt_returns_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let vectors_dir = tmp.path().join(VECTORS_DIR);
        fs::create_dir_all(&vectors_dir).unwrap();

        // Write 2 embeddings but only 1 ChunkMeta — mismatch.
        let embeddings: Vec<[f32; EMBEDDING_DIM]> = vec![[0.0f32; EMBEDDING_DIM]; 2];
        let raw: Vec<u8> = embeddings
            .iter()
            .flat_map(|e| e.iter().flat_map(|f| f.to_le_bytes()))
            .collect();
        fs::write(vectors_dir.join(INDEX_FILE), &raw).unwrap();

        let chunks = vec![ChunkMeta {
            page_name: "only".to_string(),
            path: "only.md".to_string(),
            heading: None,
            content_hash: [0u8; 32],
        }];
        fs::write(
            vectors_dir.join(CHUNKS_FILE),
            serde_json::to_string(&chunks).unwrap(),
        )
        .unwrap();

        let cache = load_embedding_cache(&vectors_dir);
        assert!(
            cache.is_empty(),
            "mismatched file sizes must produce an empty cache"
        );
    }

    /// TEST-117: `VectorIndex::open` returns `None` when `chunks.json` is absent even if
    /// `index.bin` exists.
    #[test]
    fn test_open_none_when_chunks_json_absent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let vectors_dir = tmp.path().join(VECTORS_DIR);
        fs::create_dir_all(&vectors_dir).unwrap();
        // Write index.bin but not chunks.json.
        fs::write(vectors_dir.join(INDEX_FILE), b"").unwrap();

        let result = VectorIndex::open(tmp.path()).unwrap();
        assert!(
            result.is_none(),
            "expected None when chunks.json is missing"
        );
    }

    /// TEST-118: index.bin byte layout — file size equals `chunk_count * EMBEDDING_DIM * 4`.
    /// Ensures the flat-array format has no padding or headers.
    #[test]
    fn test_index_bin_size_invariant() {
        let tmp = tempfile::TempDir::new().unwrap();
        let vectors_dir = tmp.path().join(VECTORS_DIR);
        fs::create_dir_all(&vectors_dir).unwrap();

        for chunk_count in [0usize, 1, 5, 10] {
            let embeddings: Vec<[f32; EMBEDDING_DIM]> = vec![[0.0f32; EMBEDDING_DIM]; chunk_count];
            let raw: Vec<u8> = embeddings
                .iter()
                .flat_map(|e| e.iter().flat_map(|f| f.to_le_bytes()))
                .collect();
            let index_path = vectors_dir.join(INDEX_FILE);
            fs::write(&index_path, &raw).unwrap();

            let on_disk = fs::metadata(&index_path).unwrap().len() as usize;
            let expected = chunk_count * EMBEDDING_DIM * 4;
            assert_eq!(
                on_disk, expected,
                "chunk_count={chunk_count}: file size {on_disk} != {expected}"
            );
        }
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Build a content-hash → embedding lookup table from an existing on-disk index.
///
/// Returns an empty map when the index files are absent or unreadable (graceful
/// degradation — the caller will embed all chunks from scratch).
///
/// Used by `VectorIndex::build` for incremental rebuild (REQ-097): chunks whose
/// BLAKE3 hash matches an existing entry are skipped and their stored embedding
/// is reused without invoking the ONNX runtime.
pub(crate) fn load_embedding_cache(
    vectors_dir: &Path,
) -> std::collections::HashMap<[u8; 32], [f32; EMBEDDING_DIM]> {
    let index_path = vectors_dir.join(INDEX_FILE);
    let chunks_path = vectors_dir.join(CHUNKS_FILE);

    let (raw, chunks): (Vec<u8>, Vec<ChunkMeta>) = match (
        std::fs::read(&index_path),
        std::fs::read_to_string(&chunks_path).and_then(|s| {
            serde_json::from_str::<Vec<ChunkMeta>>(&s)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
        }),
    ) {
        (Ok(r), Ok(c)) => (r, c),
        _ => return std::collections::HashMap::new(),
    };

    let chunk_count = raw.len() / (EMBEDDING_DIM * 4);
    if chunk_count != chunks.len() {
        // Corrupt or mismatched files — start fresh.
        return std::collections::HashMap::new();
    }

    let mut map = std::collections::HashMap::with_capacity(chunk_count);
    for (i, meta) in chunks.iter().enumerate() {
        let mut arr = [0f32; EMBEDDING_DIM];
        for (j, f) in arr.iter_mut().enumerate() {
            let off = (i * EMBEDDING_DIM + j) * 4;
            if let Ok(bytes) = raw[off..off + 4].try_into() {
                *f = f32::from_le_bytes(bytes);
            }
        }
        map.insert(meta.content_hash, arr);
    }
    map
}

/// Load the ONNX session and HuggingFace tokenizer.
///
/// Delegates to `acquisition::ensure_model` which handles:
/// - `ZETL_MODEL_PATH` env var override
/// - automatic download with user confirmation when files are absent
/// - SHA-256 integrity validation via sidecar files
fn load_model(vault_root: &Path) -> Result<(Session, Tokenizer)> {
    let (model_path, tokenizer_path) = acquisition::ensure_model(vault_root)?;

    let session = Session::builder()?
        .with_optimization_level(GraphOptimizationLevel::Level3)?
        .commit_from_file(&model_path)?;

    let tokenizer = Tokenizer::from_file(&tokenizer_path).map_err(|e| {
        anyhow::anyhow!(
            "Failed to load tokenizer from {}: {e}",
            tokenizer_path.display()
        )
    })?;

    Ok((session, tokenizer))
}

/// Tokenize `text` and run a single ONNX inference pass, returning a
/// normalised 384-dimensional embedding.
/// Maximum sequence length for all-MiniLM-L6-v2 (positional embedding limit).
const MAX_SEQ_LEN: usize = 256;

fn embed_text(
    session: &RefCell<Session>,
    tokenizer: &Tokenizer,
    text: &str,
) -> Result<[f32; EMBEDDING_DIM]> {
    let encoding = tokenizer
        .encode(text, true)
        .map_err(|e| anyhow::anyhow!("Tokenization failed: {e}"))?;

    // Truncate to model's max sequence length, then pad to MAX_SEQ_LEN.
    // This matches ChromaDB's approach: truncate + pad to fixed length.
    let raw_ids = encoding.get_ids();
    let raw_mask = encoding.get_attention_mask();
    let raw_type = encoding.get_type_ids();
    let len = raw_ids.len().min(MAX_SEQ_LEN);

    let mut ids = vec![0i64; MAX_SEQ_LEN];
    let mut mask = vec![0i64; MAX_SEQ_LEN];
    let mut type_ids = vec![0i64; MAX_SEQ_LEN];
    for i in 0..len {
        ids[i] = raw_ids[i] as i64;
        mask[i] = raw_mask[i] as i64;
        type_ids[i] = raw_type[i] as i64;
    }

    let shape = [1i64, MAX_SEQ_LEN as i64];
    let t_ids = TensorRef::<i64>::from_array_view((shape, ids.as_slice()))
        .map_err(|e| anyhow::anyhow!("Failed to create input_ids tensor: {e}"))?;
    let t_mask = TensorRef::<i64>::from_array_view((shape, mask.as_slice()))
        .map_err(|e| anyhow::anyhow!("Failed to create attention_mask tensor: {e}"))?;
    let t_type = TensorRef::<i64>::from_array_view((shape, type_ids.as_slice()))
        .map_err(|e| anyhow::anyhow!("Failed to create token_type_ids tensor: {e}"))?;

    let mut session_guard = session.borrow_mut();
    let outputs = session_guard.run(ort::inputs![t_ids, t_mask, t_type])?;

    // The first output is the last hidden state.
    // Shape: (1, MAX_SEQ_LEN, 384).
    // Perform attention-weighted mean pooling: only average over non-padding tokens.
    let (out_shape, data) = outputs[0]
        .try_extract_tensor::<f32>()
        .map_err(|e| anyhow::anyhow!("Failed to extract tensor: {e}"))?;
    let dims: &[i64] = &out_shape;
    anyhow::ensure!(dims.len() == 3, "expected 3-D output, got {}D", dims.len());
    let hidden_dim = dims[2] as usize;
    anyhow::ensure!(
        hidden_dim == EMBEDDING_DIM,
        "unexpected embedding dim {hidden_dim}"
    );

    // Attention-weighted mean pooling: sum(hidden * mask) / sum(mask)
    let mut embedding = [0f32; EMBEDDING_DIM];
    let mask_sum: f32 = mask.iter().map(|&m| m as f32).sum();
    let mask_sum = if mask_sum > 0.0 { mask_sum } else { 1.0 };

    for j in 0..EMBEDDING_DIM {
        let weighted_sum: f32 = (0..MAX_SEQ_LEN)
            .map(|t| data[t * EMBEDDING_DIM + j] * mask[t] as f32)
            .sum();
        embedding[j] = weighted_sum / mask_sum;
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
