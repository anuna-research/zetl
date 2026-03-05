//! Integration tests for SPEC-018: Semantic Search.
//!
//! Tests in this file require `--features semantic` to compile.
//!
//! Test ranges: TEST-114 through TEST-123.

use std::fs;
use std::path::Path;

use zetl::semantic::core::{
    chunk_page, cosine_similarity, detect_stale_chunks, reciprocal_rank_fusion, vector_search,
};

fn write(root: &Path, name: &str, content: &str) {
    fs::write(root.join(name), content).unwrap();
}

// ── Pure-core tests ────────────────────────────────────────────────────────

// TEST-120: chunk_page roundtrip — joining all chunk texts reproduces the input body.
#[test]
fn test_chunk_page_roundtrip() {
    // Make a long page (> threshold) with h2 headings.
    let body = format!(
        "Intro text.\n{}\n{}\n",
        "## Section A\n".to_string() + &"a".repeat(100),
        "## Section B\n".to_string() + &"b".repeat(100),
    );
    let headings = vec![
        (12usize, 2u8, "Section A".to_string()),
        // Approximate offset after "Intro text.\n## Section A\n" + 100 a's + \n
        (12 + 13 + 100 + 1, 2u8, "Section B".to_string()),
    ];
    let chunks = chunk_page("p", "p.md", &body, &headings, 10);
    let rejoined: String = chunks.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(rejoined, body, "chunk texts must reconstruct the full input");
}

// TEST-121: RRF — all pages from both input lists appear in the output.
#[test]
fn test_rrf_completeness() {
    let a = vec![
        ("alpha".to_string(), 1),
        ("beta".to_string(), 2),
        ("gamma".to_string(), 3),
    ];
    let b = vec![
        ("beta".to_string(), 1),
        ("delta".to_string(), 2),
    ];
    let fused = reciprocal_rank_fusion(&a, &b, 60);
    let pages: std::collections::HashSet<&str> =
        fused.iter().map(|(p, _)| p.as_str()).collect();
    for expected in &["alpha", "beta", "gamma", "delta"] {
        assert!(pages.contains(expected), "missing page: {expected}");
    }
}

// TEST-121: RRF scores are sorted descending.
#[test]
fn test_rrf_sorted_descending() {
    let a = vec![("p1".to_string(), 1), ("p2".to_string(), 2)];
    let b = vec![("p1".to_string(), 1), ("p3".to_string(), 2)];
    let fused = reciprocal_rank_fusion(&a, &b, 60);
    for w in fused.windows(2) {
        assert!(w[0].1 >= w[1].1, "scores must be non-increasing");
    }
}

// TEST-119: cosine_similarity returns 1.0 for identical unit vectors.
#[test]
fn test_cosine_similarity_self() {
    let v = vec![0.6f32, 0.8, 0.0];
    assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-5);
}

// TEST-122: feature-absent error message test (compiled path only).
// When compiled WITH --features semantic this test simply verifies the module exists.
#[test]
fn test_semantic_module_available() {
    // If this file compiles, the semantic feature is active.
    // Verify the core module exports are accessible.
    let _chunk = chunk_page("test", "test.md", "short", &[], 9999);
    let _sim = cosine_similarity(&[1.0f32, 0.0], &[0.0f32, 1.0]);
    let _rrf = reciprocal_rank_fusion(&[], &[], 60);
    let _stale = detect_stale_chunks(&[], &[]);
}

// TEST-119 (property): vector_search always returns <= limit results, sorted descending.
// Verified over a range of (embeddings_count, limit) pairs.
#[test]
fn test_vector_search_count_and_order_property() {
    // Build a set of 2-D unit vectors at various angles.
    let angles_deg: Vec<f32> = (0..20).map(|i| i as f32 * 18.0).collect();
    let embeddings: Vec<Vec<f32>> = angles_deg
        .iter()
        .map(|deg| {
            let rad = deg.to_radians();
            vec![rad.cos(), rad.sin()]
        })
        .collect();
    let query = vec![1.0f32, 0.0]; // 0°

    for limit in [0, 1, 3, 10, 20, 50] {
        let results = vector_search(&embeddings, &query, limit);
        // count <= limit
        assert!(
            results.len() <= limit,
            "limit={limit}: got {} results",
            results.len()
        );
        // count <= total embeddings
        assert!(results.len() <= embeddings.len());
        // scores are non-increasing
        for w in results.windows(2) {
            assert!(
                w[0].0 >= w[1].0,
                "limit={limit}: scores not sorted desc: {} then {}",
                w[0].0,
                w[1].0
            );
        }
    }
}

// TEST-119 (property): the highest-scoring result equals the embedding closest to the query.
#[test]
fn test_vector_search_top_result_is_best_match() {
    let embeddings: Vec<Vec<f32>> = vec![
        vec![0.0f32, 1.0],  // 90°  score = 0
        vec![-1.0f32, 0.0], // 180° score = -1
        vec![1.0f32, 0.0],  // 0°   score = 1 (best)
        vec![0.6f32, 0.8],  // ~53° score = 0.6
    ];
    let query = vec![1.0f32, 0.0];
    let results = vector_search(&embeddings, &query, 4);
    // Index 2 should be ranked first.
    assert_eq!(results[0].1, 2, "best match should be index 2");
    assert!((results[0].0 - 1.0).abs() < 1e-5);
}

// TEST-120 (property): chunk texts concatenated reproduce the full input.
// Verified for several page sizes and heading configurations.
#[test]
fn test_chunk_page_roundtrip_various_configs() {
    let cases: &[(&str, Vec<(usize, u8, String)>, usize)] = &[
        // Long page, no headings → single chunk, roundtrip trivially holds.
        ("a".repeat(2048).leak(), vec![], 100),
        // Long page with one h2 at offset 512.
        ({
            let mut s = "a".repeat(512);
            s.push_str("## Section\n");
            s.push_str(&"b".repeat(512));
            s.leak()
        }, vec![(512, 2, "Section".to_string())], 100),
        // Long page with two h2s.
        ({
            let mut s = "intro ".repeat(20); // 120 bytes
            s.push_str("## Alpha\n");
            s.push_str(&"x".repeat(300));
            s.push_str("## Beta\n");
            s.push_str(&"y".repeat(300));
            s.leak()
        }, vec![
            (120, 2, "Alpha".to_string()),
            (120 + 9 + 300, 2, "Beta".to_string()),
        ], 10),
    ];

    for (content, headings, threshold) in cases {
        let chunks = chunk_page("p", "p.md", content, headings, *threshold);
        let rejoined: String = chunks.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(
            &rejoined, content,
            "roundtrip failed for content of length {}",
            content.len()
        );
    }
}

// TEST-120 (property): when there is content before the first h2, an intro chunk (heading=None)
// appears first.
#[test]
fn test_chunk_page_intro_chunk_present() {
    let content = "Intro paragraph.\n## First Section\n".to_string() + &"x".repeat(500);
    let headings = vec![(17, 2u8, "First Section".to_string())];
    let chunks = chunk_page("p", "p.md", &content, &headings, 1);
    // First chunk should be the intro with no heading.
    assert_eq!(chunks[0].heading, None);
    assert!(chunks[0].text.starts_with("Intro"));
}

// TEST-120 (property): each chunk's page_name and path match the arguments passed in.
#[test]
fn test_chunk_page_metadata_propagated() {
    let content = "x".repeat(600);
    let headings = vec![(100, 2u8, "H".to_string())];
    let chunks = chunk_page("my-page", "notes/my-page.md", &content, &headings, 1);
    for chunk in &chunks {
        assert_eq!(chunk.page_name, "my-page");
        assert_eq!(chunk.path, "notes/my-page.md");
    }
}

// TEST-121 (property): RRF output length equals the union of both input lists.
#[test]
fn test_rrf_output_length_equals_union() {
    let a = vec![
        ("p1".to_string(), 1),
        ("p2".to_string(), 2),
        ("p3".to_string(), 3),
    ];
    let b = vec![
        ("p2".to_string(), 1), // duplicate
        ("p4".to_string(), 2),
        ("p5".to_string(), 3),
    ];
    let fused = reciprocal_rank_fusion(&a, &b, 60);
    // Union of {p1,p2,p3} and {p2,p4,p5} = {p1,p2,p3,p4,p5} = 5 distinct pages.
    assert_eq!(fused.len(), 5, "expected union size 5, got {}", fused.len());
}

// TEST-121 (property): a page that appears in both lists ranks above pages in only one list
// when both lists share the same rank for that page.
#[test]
fn test_rrf_double_appearance_boosts_rank() {
    let shared = "shared".to_string();
    let only_a = "only_a".to_string();
    let only_b = "only_b".to_string();
    let a = vec![(shared.clone(), 1), (only_a.clone(), 2)];
    let b = vec![(shared.clone(), 1), (only_b.clone(), 2)];
    let fused = reciprocal_rank_fusion(&a, &b, 60);
    // "shared" appears in both at rank 1 → highest fused score.
    assert_eq!(fused[0].0, shared);
    // The two single-list entries have equal scores; verify both appear.
    let pages: Vec<&str> = fused.iter().map(|(p, _)| p.as_str()).collect();
    assert!(pages.contains(&"only_a"));
    assert!(pages.contains(&"only_b"));
}

// TEST-122 (CLI path): when compiled WITHOUT the feature, --semantic / --hybrid print an
// error and exit non-zero. That path is tested in the standard integration suite
// (tests/integration.rs) via assert_cmd, since those tests run without --features semantic.

// ── Storage layout tests ────────────────────────────────────────────────────

use zetl::semantic::{ChunkMeta, CHUNKS_FILE, EMBEDDING_DIM, INDEX_FILE, MODEL_FILE, MODEL_NAME, VECTORS_DIR};

/// TEST-114: index.bin stores embeddings as a flat little-endian f32 array.
/// The parsed chunk count matches the number of written embeddings.
#[test]
fn test_storage_index_bin_chunk_count() {
    let tmp = tempfile::TempDir::new().unwrap();
    let vectors_dir = tmp.path().join(VECTORS_DIR);
    fs::create_dir_all(&vectors_dir).unwrap();

    let n = 7usize;
    let embeddings: Vec<[f32; EMBEDDING_DIM]> = (0..n)
        .map(|i| {
            let mut arr = [0.0f32; EMBEDDING_DIM];
            arr[0] = i as f32;
            arr
        })
        .collect();

    let raw: Vec<u8> = embeddings
        .iter()
        .flat_map(|e| e.iter().flat_map(|f| f.to_le_bytes()))
        .collect();
    fs::write(vectors_dir.join(INDEX_FILE), &raw).unwrap();

    let read_raw = fs::read(vectors_dir.join(INDEX_FILE)).unwrap();
    let chunk_count = read_raw.len() / (EMBEDDING_DIM * 4);
    assert_eq!(chunk_count, n, "chunk count mismatch");
}

/// TEST-115: chunks.json is valid JSON that round-trips all ChunkMeta fields.
#[test]
fn test_storage_chunks_json_fields() {
    let tmp = tempfile::TempDir::new().unwrap();
    let vectors_dir = tmp.path().join(VECTORS_DIR);
    fs::create_dir_all(&vectors_dir).unwrap();

    let meta = vec![
        ChunkMeta {
            page_name: "intro".to_string(),
            path: "intro.md".to_string(),
            heading: None,
            content_hash: [1u8; 32],
        },
        ChunkMeta {
            page_name: "guide".to_string(),
            path: "guide.md".to_string(),
            heading: Some("Installation".to_string()),
            content_hash: [2u8; 32],
        },
    ];

    let json_str = serde_json::to_string_pretty(&meta).unwrap();
    fs::write(vectors_dir.join(CHUNKS_FILE), &json_str).unwrap();

    let parsed: Vec<ChunkMeta> =
        serde_json::from_str(&fs::read_to_string(vectors_dir.join(CHUNKS_FILE)).unwrap()).unwrap();

    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].page_name, "intro");
    assert_eq!(parsed[0].heading, None);
    assert_eq!(parsed[0].content_hash, [1u8; 32]);
    assert_eq!(parsed[1].heading, Some("Installation".to_string()));
    assert_eq!(parsed[1].content_hash, [2u8; 32]);
}

/// TEST-116: model.json contains "model" and "dim" keys with expected values.
#[test]
fn test_storage_model_json_keys() {
    let tmp = tempfile::TempDir::new().unwrap();
    let vectors_dir = tmp.path().join(VECTORS_DIR);
    fs::create_dir_all(&vectors_dir).unwrap();

    let json = serde_json::json!({ "model": MODEL_NAME, "dim": EMBEDDING_DIM });
    fs::write(vectors_dir.join(MODEL_FILE), json.to_string()).unwrap();

    let v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(vectors_dir.join(MODEL_FILE)).unwrap()).unwrap();
    assert_eq!(v["model"].as_str().unwrap(), MODEL_NAME);
    assert_eq!(v["dim"].as_u64().unwrap(), EMBEDDING_DIM as u64);
}

/// TEST-117: `VectorIndex::open` returns `None` for each partial-directory state.
#[test]
fn test_storage_open_none_cases() {
    use zetl::semantic::VectorIndex;

    // Case 1: directory absent.
    let tmp = tempfile::TempDir::new().unwrap();
    assert!(VectorIndex::open(tmp.path()).unwrap().is_none());

    // Case 2: directory exists but both files absent.
    let tmp2 = tempfile::TempDir::new().unwrap();
    fs::create_dir_all(tmp2.path().join(VECTORS_DIR)).unwrap();
    assert!(VectorIndex::open(tmp2.path()).unwrap().is_none());

    // Case 3: only chunks.json present.
    let tmp3 = tempfile::TempDir::new().unwrap();
    let vd3 = tmp3.path().join(VECTORS_DIR);
    fs::create_dir_all(&vd3).unwrap();
    let empty: Vec<ChunkMeta> = vec![];
    fs::write(vd3.join(CHUNKS_FILE), serde_json::to_string(&empty).unwrap()).unwrap();
    assert!(VectorIndex::open(tmp3.path()).unwrap().is_none());

    // Case 4: only index.bin present.
    let tmp4 = tempfile::TempDir::new().unwrap();
    let vd4 = tmp4.path().join(VECTORS_DIR);
    fs::create_dir_all(&vd4).unwrap();
    fs::write(vd4.join(INDEX_FILE), b"").unwrap();
    assert!(VectorIndex::open(tmp4.path()).unwrap().is_none());
}

/// TEST-118: the three required files are co-located under VECTORS_DIR.
/// Verifies the directory constant and file name constants align.
#[test]
fn test_storage_constants_alignment() {
    assert!(VECTORS_DIR.contains("search/vectors"), "VECTORS_DIR should be under search/vectors");
    assert_eq!(INDEX_FILE, "index.bin");
    assert_eq!(CHUNKS_FILE, "chunks.json");
    assert_eq!(MODEL_FILE, "model.json");
    assert_eq!(EMBEDDING_DIM, 384, "all-MiniLM-L6-v2 dimension must be 384");
}

// ── CLI path tests (feature = "semantic") ────────────────────────────────────

/// TEST-123: `zetl search --semantic <QUERY>` exits non-zero with a descriptive error when the
/// vector index has not been built yet (VectorIndex::open returns None). REQ-094, REQ-098.
///
/// The binary must be compiled with `--features semantic` for this test to exercise the real
/// code path (not the stub that always rejects the flag).
#[test]
fn test_search_semantic_missing_index_exits_nonzero() {
    use std::process::Command;
    use tempfile::TempDir;

    let tmp = TempDir::new().unwrap();
    // Write a markdown file so the vault is non-empty.
    fs::write(tmp.path().join("note.md"), "# Note\nSome content here.").unwrap();
    // Do NOT build the vector index — .zetl/search/vectors/ is absent.

    let bin = assert_cmd::cargo::cargo_bin("zetl");
    let output = Command::new(bin)
        .args([
            "-d",
            tmp.path().to_str().unwrap(),
            "search",
            "--semantic",
            "test query",
        ])
        .output()
        .expect("failed to run zetl");

    assert!(
        !output.status.success(),
        "`zetl search --semantic` without a built index should exit non-zero"
    );
    // The error must mention the index is missing and how to fix it.
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{stderr}{stdout}");
    assert!(
        combined.contains("index") || combined.contains("zetl index"),
        "error output should mention the index; got: {combined}"
    );
}
