//! Integration tests for SPEC-018: Semantic Search.
//!
//! Tests in this file require `--features semantic` to compile.
//!
//! Test ranges: TEST-114 through TEST-123.

use std::fs;
use std::path::Path;

use zetl::semantic::core::{
    chunk_page, cosine_similarity, detect_stale_chunks, reciprocal_rank_fusion,
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

// TEST-122 (CLI path): when compiled WITHOUT the feature, --semantic / --hybrid print an
// error and exit non-zero. That path is tested in the standard integration suite
// (tests/integration.rs) via assert_cmd, since those tests run without --features semantic.
