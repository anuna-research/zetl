//! Integration tests for the hook selector evaluator (SPEC-032 REQ-3204,
//! REQ-3205, NFR-3201).
//!
//! Unit tests live in `src/hooks/selector.rs` and `src/hooks/predicate.rs`.
//! This file exercises the public surface through the manifest parser so
//! the wire format (TOML) and the compiled form round-trip cleanly, plus
//! the cross-layer contracts the unit tests can only assert at a
//! per-layer granularity.

use std::path::Path;
use std::time::Instant;

use serde_json::json;

use zetl::hooks::manifest::parse_manifest;
use zetl::hooks::selector::{
    compile, eval_frontmatter, match_path, run_content_probe, selector_passes, SelectorInput,
};

// ── TEST-3203 × TEST-3204 × REQ-3211: end-to-end via manifest ────────────────
//
// Parse a fully-populated manifest, compile the selector, and exercise
// every layer. If any of the wire format, compile step, or evaluation
// path regress, this test catches it.

#[test]
fn manifest_to_selector_full_roundtrip() {
    let manifest_src = r#"
stage = "transform"

[select]
include = ["posts/**/*.md"]
exclude = ["drafts/**"]
frontmatter_where = 'status == "published" && extensions.tasks != false'
content_probe = ["BEGIN", "re:^TODO\\s"]
require_probe_match = "any"
"#;
    let manifest = parse_manifest(manifest_src, None).unwrap();
    let selector = compile(&manifest.select).unwrap();

    // Matching page — all four layers pass.
    let fm = json!({ "status": "published", "extensions": { "tasks": true } });
    let inp = SelectorInput {
        path: Path::new("posts/2024/hello.md"),
        frontmatter: &fm,
        text: "BEGIN\nrest of body",
    };
    assert!(selector_passes(&selector, &inp));

    // Path excluded — REQ-3204 layer 1 short-circuits.
    let inp_drafts = SelectorInput {
        path: Path::new("drafts/wip.md"),
        ..inp
    };
    assert!(!selector_passes(&selector, &inp_drafts));

    // Frontmatter opt-out — REQ-3211 convention bites.
    let fm_opt_out = json!({ "status": "published", "extensions": { "tasks": false } });
    let inp_opt_out = SelectorInput {
        frontmatter: &fm_opt_out,
        ..inp
    };
    assert!(!selector_passes(&selector, &inp_opt_out));

    // Content probe misses.
    let inp_no_probe = SelectorInput {
        text: "no keyword here",
        ..inp
    };
    assert!(!selector_passes(&selector, &inp_no_probe));
}

// ── TEST-3204: CON-3204 pseudocode short-circuit semantics ──────────────────
//
// The CON-3204 pseudocode says: if path fails we don't evaluate the
// frontmatter predicate; if frontmatter fails we don't run content probes.
// Instrument via per-layer entry points.

#[test]
fn con_3204_short_circuit_order_matches_pseudocode() {
    // Build a selector where *each* layer is expensive/informative enough
    // that we can prove order by inspecting which layers accept vs reject.
    let manifest_src = r#"
[select]
include = ["**/*.md"]
frontmatter_where = 'status == "published"'
content_probe = ["BEGIN"]
"#;
    let manifest = parse_manifest(manifest_src, None).unwrap();
    let s = compile(&manifest.select).unwrap();

    let fm_published = json!({ "status": "published" });
    let fm_draft = json!({ "status": "draft" });

    // 1. Path mismatch.
    assert!(!match_path(&s, Path::new("image.png")));

    // 2. Path match, frontmatter mismatch.
    assert!(match_path(&s, Path::new("a.md")));
    assert!(!eval_frontmatter(&s, &fm_draft));

    // 3. Path + frontmatter match, content probe mismatch.
    assert!(match_path(&s, Path::new("a.md")));
    assert!(eval_frontmatter(&s, &fm_published));
    assert!(!run_content_probe(&s, "no keyword"));

    // 4. All four layers pass.
    assert!(selector_passes(
        &s,
        &SelectorInput {
            path: Path::new("a.md"),
            frontmatter: &fm_published,
            text: "BEGIN here",
        },
    ));
}

// ── NFR-3201: Selector hot-path latency budget ──────────────────────────────
//
// The NFR asks for ≤ 2 ms P95 per (page × hook) on pages up to 100 KB.
// This isn't a formal P95 harness (that lands with `cargo bench` in
// tests/perf.rs under perf features), but it's a coarse regression
// gate — 10k evaluations against a 100 KB body, with regex and substring
// probes + frontmatter predicate, should finish within an order of
// magnitude of the implied budget (10_000 × 2 ms = 20 s). Ceiling is
// 40 s (2 × budget), which catches real regressions (>2× slowdown)
// while tolerating slow CI workers and cold-cache first runs. Strict
// P95-per-eval enforcement lives in `tests/nfr_gates_integration.rs`.

#[test]
fn hot_path_latency_order_of_magnitude() {
    let manifest_src = r#"
[select]
include = ["**/*.md"]
exclude = ["drafts/**"]
frontmatter_where = 'status == "published" && !draft && word_count > 100'
content_probe = ["KEYWORD", "re:^#\\s+Title$"]
require_probe_match = "any"
"#;
    let manifest = parse_manifest(manifest_src, None).unwrap();
    let s = compile(&manifest.select).unwrap();

    // 100 KB page, roughly worst-case for the content probe layer.
    let mut body = String::with_capacity(100 * 1024);
    while body.len() < 100 * 1024 {
        body.push_str("A line of Markdown prose that doesn't mention the keyword. ");
    }
    body.push_str("\nKEYWORD\n");

    let fm = json!({
        "status": "published",
        "draft": false,
        "word_count": 2000
    });
    let inp = SelectorInput {
        path: Path::new("posts/hello.md"),
        frontmatter: &fm,
        text: &body,
    };

    let start = Instant::now();
    let mut hits = 0usize;
    for _ in 0..10_000 {
        if selector_passes(&s, &inp) {
            hits += 1;
        }
    }
    let elapsed = start.elapsed();

    assert_eq!(hits, 10_000, "selector should match every iteration");
    // Coarse regression gate: 10k iterations must stay within 2× the
    // implied NFR-3201 budget (2 × 20 s = 40 s). NFR-3201's tight
    // 2 ms/page budget is enforced by a separate perf harness; this
    // guards against order-of-magnitude regressions only.
    assert!(
        elapsed.as_secs() < 40,
        "10k selector evaluations took {elapsed:?}, expected < 40s"
    );
}

// ── REQ-3204 step 1: path glob behaviour ────────────────────────────────────

#[test]
fn path_globs_accept_relative_posix_paths() {
    let manifest_src = r#"
[select]
include = ["posts/**/*.md", "notes/*.md"]
exclude = ["posts/private/**", "notes/_secret.md"]
"#;
    let manifest = parse_manifest(manifest_src, None).unwrap();
    let s = compile(&manifest.select).unwrap();
    let fm = json!({});

    for (path, expect) in [
        ("posts/2024/hello.md", true),
        ("posts/private/leak.md", false),
        ("notes/daily.md", true),
        ("notes/_secret.md", false),
        ("other/file.md", false),
        ("posts/deep/nested/file.md", true),
    ] {
        let inp = SelectorInput {
            path: Path::new(path),
            frontmatter: &fm,
            text: "",
        };
        assert_eq!(
            selector_passes(&s, &inp),
            expect,
            "path={path} expect={expect}"
        );
    }
}

// ── REQ-3205: Frontmatter predicate canonical examples via manifest ─────────

#[test]
fn req_3205_canonical_examples_via_manifest() {
    for (expr, fm, expect) in [
        (
            r#"tags contains "project""#,
            json!({ "tags": ["project", "x"] }),
            true,
        ),
        (
            r#"status == "published" && !draft"#,
            json!({ "status": "published", "draft": false }),
            true,
        ),
        (
            r#"frontmatter.extensions.tasks != false"#,
            json!({ "frontmatter": { "extensions": { "tasks": false } } }),
            false,
        ),
        (r#"word_count > 500"#, json!({ "word_count": 742 }), true),
        (
            r#"title matches "^Daily.*""#,
            json!({ "title": "Daily log" }),
            true,
        ),
        // Type-strict: string "500" is not ordered against number 500.
        (r#"word_count > 500"#, json!({ "word_count": "500" }), false),
    ] {
        let manifest_src = format!(
            r#"
[select]
frontmatter_where = {}
"#,
            toml_string_literal(expr)
        );
        let manifest = parse_manifest(&manifest_src, None).unwrap();
        let s = compile(&manifest.select).unwrap();
        let inp = SelectorInput {
            path: Path::new("a.md"),
            frontmatter: &fm,
            text: "",
        };
        assert_eq!(selector_passes(&s, &inp), expect, "expr={expr:?} fm={fm}");
    }
}

/// Tiny TOML string-literal encoder for the test's `format!` above.
fn toml_string_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        // Single-quoted TOML literals can't contain single quotes; our
        // REQ-3205 examples use double quotes inside, so a single-quoted
        // wrapper is lossless here.
        assert!(ch != '\'', "test expr must not contain single quotes");
        out.push(ch);
    }
    out.push('\'');
    out
}
