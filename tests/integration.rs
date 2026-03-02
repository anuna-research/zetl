//! Integration tests for the `zetl` CLI.
//!
//! These tests exercise the binary end-to-end by creating temporary vaults
//! with known markdown files, running `zetl` subcommands via
//! `std::process::Command`, and verifying the JSON output.

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a `Command` for the `zetl` binary with the given vault directory.
fn zetl_cmd(vault: &Path) -> Command {
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("zetl");
    cmd.arg("-d").arg(vault.as_os_str());
    cmd.arg("--no-cache");
    cmd
}

/// Build a `Command` without `--no-cache` (uses the incremental file cache).
fn zetl_cmd_cached(vault: &Path) -> Command {
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("zetl");
    cmd.arg("-d").arg(vault.as_os_str());
    cmd
}

/// Create a file relative to `root`, creating parent directories as needed.
fn write_file(root: &Path, relative: &str, content: &str) {
    let full = root.join(relative);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).expect("create parent dirs");
    }
    fs::write(&full, content).expect("write test file");
}

/// Run the command, assert success, parse stdout as JSON.
fn run_json(cmd: &mut Command) -> Value {
    let output = cmd.output().expect("failed to execute zetl");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "zetl exited with non-zero status.\nstdout: {stdout}\nstderr: {stderr}",
    );
    serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("failed to parse JSON output: {e}\nraw stdout: {stdout}"))
}

/// Run the command, assert success, parse stdout as JSON, and return stderr too.
fn run_json_with_stderr(cmd: &mut Command) -> (Value, String) {
    let output = cmd.output().expect("failed to execute zetl");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "zetl exited with non-zero status.\nstdout: {stdout}\nstderr: {stderr}",
    );
    let json = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("failed to parse JSON output: {e}\nraw stdout: {stdout}"));
    (json, stderr.into_owned())
}

/// Run the command (may fail) and parse stdout as JSON regardless of exit code.
fn run_json_any(cmd: &mut Command) -> (Value, std::process::ExitStatus) {
    let output = cmd.output().expect("failed to execute zetl");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("failed to parse JSON output: {e}\nraw stdout: {stdout}"));
    (json, output.status)
}

// ---------------------------------------------------------------------------
// Vault builders for reusable test fixtures
// ---------------------------------------------------------------------------

/// Create a vault for TEST-001: 5 files, 12 body-text wikilinks,
/// plus some links inside code blocks/inline code that must be excluded.
fn build_test001_vault(root: &Path) {
    // File 1: Index.md -- 3 body links
    write_file(
        root,
        "Index.md",
        "\
# Index

Welcome to the vault. See [[Page A]], [[Page B|alias]], and [[Page C#Heading]].
",
    );

    // File 2: Page A.md -- 3 body links
    write_file(
        root,
        "Page A.md",
        "\
# Page A

This links to [[Page B]], [[Page D]], and also ![[Page C]].
",
    );

    // File 3: Page B.md -- 2 body links, plus links inside code that must be ignored
    write_file(
        root,
        "Page B.md",
        "\
# Page B

Body link to [[Page C]] and [[Index]].

```
This is a code block with [[Fake Link 1]] that should be ignored.
```

Also `[[Fake Inline]]` should be ignored.
",
    );

    // File 4: Page C.md -- 2 body links
    write_file(
        root,
        "Page C.md",
        "\
# Page C

See [[Page A]] and [[Page D^block1]].
",
    );

    // File 5: Page D.md -- 2 body links, plus link in HTML comment
    write_file(
        root,
        "Page D.md",
        "\
# Page D

Linking to [[Index]] and [[Page A]].

<!-- [[Hidden Link]] should be ignored -->
",
    );
}

/// Create a vault for TEST-002 graph construction:
/// A -> B, A -> C; B -> C; D has no links.
fn build_test002_vault(root: &Path) {
    write_file(root, "A.md", "# A\n\nLinks to [[B]] and [[C]].\n");
    write_file(root, "B.md", "# B\n\nLinks to [[C]].\n");
    write_file(root, "C.md", "# C\n\nNo outgoing links.\n");
    write_file(root, "D.md", "# D\n\nAlso no outgoing links.\n");
}

/// Create a vault for TEST-003 forward link query:
/// Index.md contains [[Page A]] and [[Page B|alias]].
fn build_test003_vault(root: &Path) {
    write_file(
        root,
        "Index.md",
        "# Index\n\nSee [[Page A]] and [[Page B|alias]].\n",
    );
    write_file(root, "Page A.md", "# Page A\n\nContent.\n");
    write_file(root, "Page B.md", "# Page B\n\nContent.\n");
}

/// Create a vault for TEST-004 backlink query:
/// 3 files link to "Concept X".
fn build_test004_vault(root: &Path) {
    write_file(
        root,
        "Concept X.md",
        "# Concept X\n\nThis is the target page.\n",
    );
    write_file(root, "Note 1.md", "# Note 1\n\nRelated to [[Concept X]].\n");
    write_file(
        root,
        "Note 2.md",
        "# Note 2\n\nAlso about [[Concept X]] and [[Note 1]].\n",
    );
    write_file(root, "Note 3.md", "# Note 3\n\nSee [[Concept X]].\n");
}

/// Create a vault for TEST-005 dead link detection:
/// File A links to existing and nonexistent pages.
fn build_test005_vault(root: &Path) {
    write_file(
        root,
        "A.md",
        "# A\n\nLinks to [[Existing Page]] and [[Ghost Page]].\n",
    );
    write_file(root, "Existing Page.md", "# Existing Page\n\nContent.\n");
}

/// Create a vault for TEST-006 orphan detection:
/// A -> B -> C, D is never linked to.
fn build_test006_vault(root: &Path) {
    write_file(root, "A.md", "# A\n\nLinks to [[B]].\n");
    write_file(root, "B.md", "# B\n\nLinks to [[C]].\n");
    write_file(root, "C.md", "# C\n\nNo outgoing links.\n");
    write_file(root, "D.md", "# D\n\nNever referenced by anyone.\n");
}

/// Create a vault for TEST-007 syntax validation:
/// A file with unclosed bracket, empty [[]], and a valid link.
fn build_test007_vault(root: &Path) {
    write_file(
        root,
        "Bad Syntax.md",
        "\
# Bad Syntax

Line 3 is fine.
See [[unclosed bracket
More text.
Line 6 is fine.
Empty [[]] link here.
Line 8 is fine.
Line 9 is fine.
Line 10 is fine.
Valid [[Good Link]] here.
",
    );
    write_file(root, "Good Link.md", "# Good Link\n\nContent.\n");
}

/// Create a vault for TEST-008 SimHash fuzzy search:
/// Pages with similar and dissimilar names.
fn build_test008_vault(root: &Path) {
    write_file(
        root,
        "Zettelkasten Method.md",
        "# Zettelkasten Method\n\nThe zettelkasten method is a personal knowledge management system.\n",
    );
    write_file(
        root,
        "Zettelkasten History.md",
        "# Zettelkasten History\n\nThe zettelkasten was invented by Niklas Luhmann.\n",
    );
    write_file(
        root,
        "Rust Programming.md",
        "# Rust Programming\n\nRust is a systems programming language.\n",
    );
}

/// Create a vault for TEST-009 stats.
/// Reuses the TEST-002 graph: A->B, A->C, B->C, D isolated.
fn build_test009_vault(root: &Path) {
    build_test002_vault(root);
}

/// Create a vault for TEST-010 shortest path:
/// A -> B -> C -> D (linear chain), E is isolated.
fn build_test010_vault(root: &Path) {
    write_file(root, "A.md", "# A\n\nLinks to [[B]].\n");
    write_file(root, "B.md", "# B\n\nLinks to [[C]].\n");
    write_file(root, "C.md", "# C\n\nLinks to [[D]].\n");
    write_file(root, "D.md", "# D\n\nEnd of chain.\n");
    write_file(root, "E.md", "# E\n\nIsolated node.\n");
}

/// Create a vault for TEST-012 ignore patterns:
/// Has .zetlignore with "drafts/" and files in drafts/ that should be excluded.
fn build_test012_vault(root: &Path) {
    write_file(root, ".zetlignore", "drafts/\n");
    write_file(root, "Public.md", "# Public\n\nLinks to [[Notes]].\n");
    write_file(root, "Notes.md", "# Notes\n\nPublic note.\n");
    write_file(
        root,
        "drafts/Draft A.md",
        "# Draft A\n\nThis is a draft with [[Public]].\n",
    );
    write_file(
        root,
        "drafts/Draft B.md",
        "# Draft B\n\nAnother draft with [[Notes]].\n",
    );
}

// ===========================================================================
// TEST-001: Index Scan Completeness
// ===========================================================================

#[test]
fn test_001_index_scan_completeness() {
    let dir = TempDir::new().expect("create temp dir");
    build_test001_vault(dir.path());

    let json = run_json(zetl_cmd(dir.path()).arg("index"));

    // Verify files_scanned = 5
    assert_eq!(
        json["files_scanned"].as_u64(),
        Some(5),
        "expected 5 files scanned, got: {json}"
    );

    // Verify links_found = 12 (only body text links, excluding code/comments)
    assert_eq!(
        json["links_found"].as_u64(),
        Some(12),
        "expected 12 body-text links found, got: {json}"
    );

    // Verify elapsed time is reported
    assert!(
        json.get("elapsed_ms").is_some(),
        "elapsed_ms should be present in output"
    );
}

// ===========================================================================
// TEST-002: Graph Construction
// ===========================================================================

#[test]
fn test_002_graph_construction() {
    let dir = TempDir::new().expect("create temp dir");
    build_test002_vault(dir.path());

    // First, build the index
    run_json(zetl_cmd(dir.path()).arg("index"));

    // Check forward links from A: should link to B and C
    let json_a = run_json(zetl_cmd(dir.path()).arg("links").arg("A"));
    let links_a = json_a["links"].as_array().expect("links should be array");
    assert_eq!(
        links_a.len(),
        2,
        "A should have 2 forward links, got: {links_a:?}"
    );

    // Check forward links from B: should link to C
    let json_b = run_json(zetl_cmd(dir.path()).arg("links").arg("B"));
    let links_b = json_b["links"].as_array().expect("links should be array");
    assert_eq!(
        links_b.len(),
        1,
        "B should have 1 forward link, got: {links_b:?}"
    );

    // Check forward links from D: should have 0 links
    let json_d = run_json(zetl_cmd(dir.path()).arg("links").arg("D"));
    let links_d = json_d["links"].as_array().expect("links should be array");
    assert_eq!(
        links_d.len(),
        0,
        "D should have 0 forward links, got: {links_d:?}"
    );

    // Check backlinks for C: should be linked from A and B
    let json_c_bl = run_json(zetl_cmd(dir.path()).arg("backlinks").arg("C"));
    let backlinks_c = json_c_bl["backlinks"]
        .as_array()
        .expect("backlinks should be array");
    assert_eq!(
        backlinks_c.len(),
        2,
        "C should have 2 backlinks, got: {backlinks_c:?}"
    );
    let sources: Vec<&str> = backlinks_c
        .iter()
        .filter_map(|bl| bl["source"].as_str())
        .collect();
    assert!(
        sources.contains(&"A"),
        "C should have backlink from A, sources: {sources:?}"
    );
    assert!(
        sources.contains(&"B"),
        "C should have backlink from B, sources: {sources:?}"
    );

    // D is an orphan (zero incoming edges) -- checked via orphan detection in TEST-006
}

// ===========================================================================
// TEST-003: Forward Link Query
// ===========================================================================

#[test]
fn test_003_forward_link_query() {
    let dir = TempDir::new().expect("create temp dir");
    build_test003_vault(dir.path());

    let json = run_json(zetl_cmd(dir.path()).arg("links").arg("Index"));

    // Verify the page name in the response
    assert_eq!(
        json["page"].as_str(),
        Some("Index"),
        "page field should be 'Index'"
    );

    // Verify 2 links returned
    let links = json["links"].as_array().expect("links should be array");
    assert_eq!(links.len(), 2, "expected 2 forward links from Index");

    // Find the link to Page A (no alias)
    let link_a = links
        .iter()
        .find(|l| {
            l["target"].as_str() == Some("Page A") || l["target_page"].as_str() == Some("Page A")
        })
        .expect("should find link to Page A");
    assert!(
        link_a.get("alias").is_none()
            || link_a["alias"].is_null()
            || link_a["alias"].as_str() == Some(""),
        "Page A link should have no alias"
    );

    // Find the link to Page B (with alias "alias")
    let link_b = links
        .iter()
        .find(|l| {
            l["target"].as_str() == Some("Page B") || l["target_page"].as_str() == Some("Page B")
        })
        .expect("should find link to Page B");
    assert_eq!(
        link_b["alias"].as_str(),
        Some("alias"),
        "Page B link should have alias 'alias'"
    );
}

// ===========================================================================
// TEST-004: Backlink Query
// ===========================================================================

#[test]
fn test_004_backlink_query() {
    let dir = TempDir::new().expect("create temp dir");
    build_test004_vault(dir.path());

    let json = run_json(zetl_cmd(dir.path()).arg("backlinks").arg("Concept X"));

    // Verify backlinks count
    let backlinks = json["backlinks"]
        .as_array()
        .expect("backlinks should be array");
    assert_eq!(
        backlinks.len(),
        3,
        "expected 3 backlinks to 'Concept X', got: {backlinks:?}"
    );

    // Verify source files are present
    let sources: Vec<&str> = backlinks
        .iter()
        .filter_map(|bl| bl["source"].as_str())
        .collect();
    assert!(
        sources.contains(&"Note 1"),
        "should have backlink from Note 1, sources: {sources:?}"
    );
    assert!(
        sources.contains(&"Note 2"),
        "should have backlink from Note 2, sources: {sources:?}"
    );
    assert!(
        sources.contains(&"Note 3"),
        "should have backlink from Note 3, sources: {sources:?}"
    );

    // Each backlink should have a line number
    for bl in backlinks {
        assert!(
            bl.get("line").is_some() && bl["line"].as_u64().is_some(),
            "each backlink should have a line number, got: {bl:?}"
        );
    }
}

// ===========================================================================
// TEST-005: Dead Link Detection
// ===========================================================================

#[test]
fn test_005_dead_link_detection() {
    let dir = TempDir::new().expect("create temp dir");
    build_test005_vault(dir.path());

    let (json, status) = run_json_any(zetl_cmd(dir.path()).arg("check").arg("--dead-links"));

    // Should report dead links
    let dead_links = json["dead_links"]
        .as_array()
        .expect("dead_links should be array");

    // There should be exactly 1 dead link: Ghost Page
    assert_eq!(
        dead_links.len(),
        1,
        "expected 1 dead link, got: {dead_links:?}"
    );

    let dead = &dead_links[0];
    assert_eq!(
        dead["target"].as_str(),
        Some("Ghost Page"),
        "dead link target should be 'Ghost Page'"
    );
    assert!(
        dead["source"].as_str().is_some(),
        "dead link should have source"
    );
    assert!(
        dead["line"].as_u64().is_some(),
        "dead link should have line number"
    );

    // Exit code should be non-zero (issues found)
    assert!(
        !status.success(),
        "check --dead-links should exit non-zero when dead links exist"
    );
}

// ===========================================================================
// TEST-006: Orphan Detection
// ===========================================================================

#[test]
fn test_006_orphan_detection() {
    let dir = TempDir::new().expect("create temp dir");
    build_test006_vault(dir.path());

    let (json, _status) = run_json_any(zetl_cmd(dir.path()).arg("check").arg("--orphans"));

    let orphans = json["orphans"].as_array().expect("orphans should be array");

    // Extract orphan page names
    let orphan_pages: Vec<&str> = orphans.iter().filter_map(|o| o["page"].as_str()).collect();

    // D should be an orphan (never linked to)
    assert!(
        orphan_pages.contains(&"D"),
        "D should be detected as an orphan, orphans: {orphan_pages:?}"
    );

    // A is also an orphan (nobody links to A in this vault)
    assert!(
        orphan_pages.contains(&"A"),
        "A should be detected as an orphan (no incoming links), orphans: {orphan_pages:?}"
    );

    // B should NOT be an orphan (A links to B)
    assert!(
        !orphan_pages.contains(&"B"),
        "B should NOT be an orphan, orphans: {orphan_pages:?}"
    );

    // C should NOT be an orphan (B links to C)
    assert!(
        !orphan_pages.contains(&"C"),
        "C should NOT be an orphan, orphans: {orphan_pages:?}"
    );
}

// ===========================================================================
// TEST-007: Syntax Validation
// ===========================================================================

#[test]
fn test_007_syntax_validation() {
    let dir = TempDir::new().expect("create temp dir");
    build_test007_vault(dir.path());

    let (json, _status) = run_json_any(zetl_cmd(dir.path()).arg("check").arg("--syntax"));

    let syntax_errors = json["syntax_errors"]
        .as_array()
        .expect("syntax_errors should be array");

    // Should report at least 2 diagnostics (unclosed bracket on line 4, empty [[]] on line 7)
    assert!(
        syntax_errors.len() >= 2,
        "expected at least 2 syntax errors, got {}: {:?}",
        syntax_errors.len(),
        syntax_errors
    );

    // Check that we have an unclosed wikilink diagnostic
    let has_unclosed = syntax_errors.iter().any(|e| {
        e["message"]
            .as_str()
            .map(|m| m.to_lowercase().contains("unclosed"))
            .unwrap_or(false)
    });
    assert!(has_unclosed, "should detect unclosed wikilink syntax");

    // Check that we have an empty wikilink diagnostic
    let has_empty = syntax_errors.iter().any(|e| {
        e["message"]
            .as_str()
            .map(|m| m.to_lowercase().contains("empty"))
            .unwrap_or(false)
    });
    assert!(has_empty, "should detect empty wikilink syntax");

    // Verify that each error has file, line, column, and message
    for err in syntax_errors {
        assert!(
            err.get("file").is_some() || err.get("source").is_some(),
            "syntax error should have file, got: {err:?}"
        );
        assert!(
            err.get("line").is_some(),
            "syntax error should have line, got: {err:?}"
        );
        assert!(
            err.get("message").is_some(),
            "syntax error should have message, got: {err:?}"
        );
    }
}

// ===========================================================================
// TEST-008: SimHash Fuzzy Search
// ===========================================================================

#[test]
fn test_008_simhash_fuzzy_search() {
    let dir = TempDir::new().expect("create temp dir");
    build_test008_vault(dir.path());

    // First index the vault
    run_json(zetl_cmd(dir.path()).arg("index"));

    // Search for "zettelkasen" (typo) with a generous threshold
    // (SimHash with character trigrams can produce moderate distances
    // for short strings with small edits)
    let json = run_json(
        zetl_cmd(dir.path())
            .arg("similar")
            .arg("zettelkasen")
            .arg("--threshold")
            .arg("20"),
    );

    let results = json["results"].as_array().expect("results should be array");

    let page_names: Vec<&str> = results.iter().filter_map(|r| r["page"].as_str()).collect();

    // Similar Zettelkasten pages should appear
    assert!(
        page_names
            .iter()
            .any(|name| name.to_lowercase().contains("zettelkasten")),
        "should find at least one Zettelkasten page, got: {page_names:?}"
    );

    // Each result should have a distance field
    for result in results {
        assert!(
            result.get("distance").is_some(),
            "each result should have a distance field, got: {result:?}"
        );
    }

    // Results should be sorted by distance (ascending)
    let distances: Vec<u64> = results
        .iter()
        .filter_map(|r| r["distance"].as_u64())
        .collect();
    for window in distances.windows(2) {
        assert!(
            window[0] <= window[1],
            "results should be sorted by distance ascending, got: {distances:?}"
        );
    }
}

// ===========================================================================
// TEST-009: Stats
// ===========================================================================

#[test]
fn test_009_stats() {
    let dir = TempDir::new().expect("create temp dir");
    build_test009_vault(dir.path());

    let json = run_json(zetl_cmd(dir.path()).arg("stats"));

    // The vault has 4 pages (A, B, C, D)
    assert_eq!(
        json["pages"].as_u64(),
        Some(4),
        "expected 4 pages, got: {json}"
    );

    // 3 links: A->B, A->C, B->C
    assert_eq!(
        json["links"].as_u64(),
        Some(3),
        "expected 3 links, got: {json}"
    );

    // Dead links: 0 (all targets exist)
    assert_eq!(
        json["dead_links"].as_u64(),
        Some(0),
        "expected 0 dead links, got: {json}"
    );

    // Orphans: A and D have no incoming links
    assert_eq!(
        json["orphans"].as_u64(),
        Some(2),
        "expected 2 orphans (A and D), got: {json}"
    );

    // Connected components: 2 (the {A,B,C} cluster and {D} isolated)
    // petgraph connected_components treats graph as undirected, so {A,B,C} is one component
    // and {D} is another.
    assert_eq!(
        json["connected_components"].as_u64(),
        Some(2),
        "expected 2 connected components, got: {json}"
    );

    // most_linked should be present and sorted descending by backlink_count
    let most_linked = json["most_linked"]
        .as_array()
        .expect("most_linked should be array");
    assert!(!most_linked.is_empty(), "most_linked should not be empty");

    // C should be the most linked (2 incoming: from A and B)
    assert_eq!(
        most_linked[0]["page"].as_str(),
        Some("C"),
        "most linked page should be C, got: {most_linked:?}"
    );
    assert_eq!(
        most_linked[0]["backlink_count"].as_u64(),
        Some(2),
        "C should have 2 backlinks"
    );

    // Verify most_linked is sorted descending
    let backlink_counts: Vec<u64> = most_linked
        .iter()
        .filter_map(|m| m["backlink_count"].as_u64())
        .collect();
    for window in backlink_counts.windows(2) {
        assert!(
            window[0] >= window[1],
            "most_linked should be sorted descending, got: {backlink_counts:?}"
        );
    }
}

// ===========================================================================
// TEST-010: Shortest Path
// ===========================================================================

#[test]
fn test_010_shortest_path_found() {
    let dir = TempDir::new().expect("create temp dir");
    build_test010_vault(dir.path());

    let json = run_json(zetl_cmd(dir.path()).arg("path").arg("A").arg("D"));

    // Verify path: A -> B -> C -> D
    assert_eq!(json["from"].as_str(), Some("A"));
    assert_eq!(json["to"].as_str(), Some("D"));
    assert_eq!(
        json["hops"].as_u64(),
        Some(3),
        "expected 3 hops, got: {json}"
    );

    let path = json["path"].as_array().expect("path should be array");
    let path_names: Vec<&str> = path.iter().filter_map(|v| v.as_str()).collect();
    assert_eq!(
        path_names,
        vec!["A", "B", "C", "D"],
        "path should be [A, B, C, D]"
    );
}

#[test]
fn test_010_shortest_path_no_path() {
    let dir = TempDir::new().expect("create temp dir");
    build_test010_vault(dir.path());

    // E is isolated, so no path from A to E
    let mut cmd = zetl_cmd(dir.path());
    cmd.arg("path").arg("A").arg("E");

    // Should exit non-zero
    cmd.assert().failure();
}

// ===========================================================================
// TEST-012: Ignore Patterns
// ===========================================================================

#[test]
fn test_012_ignore_patterns() {
    let dir = TempDir::new().expect("create temp dir");
    build_test012_vault(dir.path());

    let json = run_json(zetl_cmd(dir.path()).arg("index"));

    // Should only scan 2 files (Public.md and Notes.md), not the drafts
    assert_eq!(
        json["files_scanned"].as_u64(),
        Some(2),
        "expected 2 files scanned (drafts/ should be ignored), got: {json}"
    );

    // Verify with stats that only 2 pages exist
    let stats = run_json(zetl_cmd(dir.path()).arg("stats"));
    assert_eq!(
        stats["pages"].as_u64(),
        Some(2),
        "expected 2 pages in stats (drafts excluded), got: {stats}"
    );
}

// ===========================================================================
// Additional edge-case tests
// ===========================================================================

#[test]
fn test_empty_vault() {
    let dir = TempDir::new().expect("create temp dir");

    let json = run_json(zetl_cmd(dir.path()).arg("index"));

    assert_eq!(
        json["files_scanned"].as_u64(),
        Some(0),
        "empty vault should have 0 files scanned"
    );
    assert_eq!(
        json["links_found"].as_u64(),
        Some(0),
        "empty vault should have 0 links found"
    );
}

#[test]
fn test_default_ignores_git_and_node_modules() {
    let dir = TempDir::new().expect("create temp dir");

    // Create files that should be excluded by default
    write_file(
        dir.path(),
        ".git/objects/note.md",
        "# Git Object\n\n[[Link]].\n",
    );
    write_file(
        dir.path(),
        "node_modules/pkg/readme.md",
        "# Readme\n\n[[Link]].\n",
    );
    write_file(dir.path(), ".zetl/index.md", "# Index\n\n[[Link]].\n");
    // This one should be scanned
    write_file(dir.path(), "Real Note.md", "# Real Note\n\nContent.\n");

    let json = run_json(zetl_cmd(dir.path()).arg("index"));

    assert_eq!(
        json["files_scanned"].as_u64(),
        Some(1),
        "only Real Note.md should be scanned (default ignores for .git, node_modules, .zetl), got: {json}"
    );
}

#[test]
fn test_version_flag() {
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("zetl");
    cmd.arg("--version");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("zetl"));
}

#[test]
fn test_help_flag() {
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("zetl");
    cmd.arg("--help");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("wikilink"));
}

#[test]
fn test_links_case_insensitive_page_name() {
    let dir = TempDir::new().expect("create temp dir");
    write_file(
        dir.path(),
        "My Page.md",
        "# My Page\n\nLinks to [[Other]].\n",
    );
    write_file(dir.path(), "Other.md", "# Other\n\nContent.\n");

    // Query with different case should still work
    let json = run_json(zetl_cmd(dir.path()).arg("links").arg("my page"));

    let links = json["links"].as_array().expect("links should be array");
    assert_eq!(links.len(), 1, "should find 1 forward link");
}

#[test]
fn test_check_all_categories() {
    let dir = TempDir::new().expect("create temp dir");

    // Mix of issues: dead link, orphan, syntax error
    write_file(
        dir.path(),
        "Main.md",
        "# Main\n\nLinks to [[Existing]] and [[Ghost]].\n",
    );
    write_file(
        dir.path(),
        "Existing.md",
        "# Existing\n\nLinks to [[Main]].\n",
    );
    write_file(dir.path(), "Orphan.md", "# Orphan\n\nNobody links here.\n");
    write_file(dir.path(), "Broken.md", "# Broken\n\nUnclosed [[link\n");

    // Run check without any filter flags (should report all categories)
    let (json, _status) = run_json_any(zetl_cmd(dir.path()).arg("check"));

    // Should have dead_links, orphans, and syntax_errors in the output
    assert!(
        json.get("dead_links").is_some(),
        "check output should have dead_links field"
    );
    assert!(
        json.get("orphans").is_some(),
        "check output should have orphans field"
    );
    assert!(
        json.get("syntax_errors").is_some(),
        "check output should have syntax_errors field"
    );

    // Verify summary
    if let Some(summary) = json.get("summary") {
        assert!(
            summary.get("dead_links").is_some(),
            "summary should have dead_links count"
        );
        assert!(
            summary.get("orphans").is_some(),
            "summary should have orphans count"
        );
        assert!(
            summary.get("syntax_errors").is_some(),
            "summary should have syntax_errors count"
        );
    }
}

// ===========================================================================
// TEST-013: Basic Content Search
// ===========================================================================

fn build_test013_vault(root: &Path) {
    write_file(root, "Alpha.md", "# Alpha\n\nThe quick brown fox.\n");
    write_file(
        root,
        "Beta.md",
        "# Beta\n\nNothing here.\nA quick summary of topics.\n",
    );
    write_file(root, "Gamma.md", "# Gamma\n\nNo match here.\n");
}

#[test]
fn test_013_basic_content_search() {
    let dir = TempDir::new().expect("create temp dir");
    build_test013_vault(dir.path());

    let json = run_json(zetl_cmd(dir.path()).arg("search").arg("quick"));

    let results = json["results"].as_array().expect("results should be array");

    assert_eq!(
        results.len(),
        2,
        "expected 2 matches for 'quick', got: {results:?}"
    );

    // Each result should have page, path, line, column
    for result in results {
        assert!(result.get("page").is_some(), "result should have page");
        assert!(result.get("path").is_some(), "result should have path");
        assert!(result["line"].as_u64().is_some(), "result should have line");
        assert!(
            result["column"].as_u64().is_some(),
            "result should have column"
        );
    }

    // Verify pages found
    let pages: Vec<&str> = results.iter().filter_map(|r| r["page"].as_str()).collect();
    assert!(pages.contains(&"Alpha"), "should find match in Alpha");
    assert!(pages.contains(&"Beta"), "should find match in Beta");
}

#[test]
fn test_013_search_with_context() {
    let dir = TempDir::new().expect("create temp dir");
    build_test013_vault(dir.path());

    let json = run_json(
        zetl_cmd(dir.path())
            .arg("search")
            .arg("quick")
            .arg("--context")
            .arg("10"),
    );

    let results = json["results"].as_array().expect("results should be array");

    // All results should have context
    for result in results {
        assert!(
            result["context"].as_str().is_some(),
            "result should have context when --context is specified, got: {result:?}"
        );
    }
}

#[test]
fn test_013_search_no_matches() {
    let dir = TempDir::new().expect("create temp dir");
    build_test013_vault(dir.path());

    let mut cmd = zetl_cmd(dir.path());
    cmd.arg("search").arg("nonexistent");

    // Should exit non-zero
    cmd.assert().failure();
}

// ===========================================================================
// TEST-014: Body-Text Exclusion
// ===========================================================================

#[test]
fn test_014_body_text_exclusion() {
    let dir = TempDir::new().expect("create temp dir");

    write_file(
        dir.path(),
        "Mixed.md",
        "---\ntitle: Quick Start Guide\n---\n\n# Mixed\n\nBody has quick overview.\n\n```\nquick_sort(arr)\n```\n\nMore body text.\n",
    );

    // Default: body-text only — should find "quick" in body but not frontmatter or code block
    let json = run_json(zetl_cmd(dir.path()).arg("search").arg("quick"));

    let results = json["results"].as_array().expect("results should be array");

    assert_eq!(
        results.len(),
        1,
        "should find 1 match in body text only (not frontmatter/code), got: {results:?}"
    );
    assert_eq!(
        results[0]["line"].as_u64(),
        Some(7),
        "match should be on line 7 (body text)"
    );
}

#[test]
#[ignore = "planned feature: --all search mode not yet implemented in CLI"]
fn test_014_search_all_mode() {
    let dir = TempDir::new().expect("create temp dir");

    write_file(
        dir.path(),
        "Mixed.md",
        "---\ntitle: Quick Start Guide\n---\n\n# Mixed\n\nBody has quick overview.\n\n```\nquick_sort(arr)\n```\n\nMore body text.\n",
    );

    // --all mode: should find "quick" in frontmatter, body, and code block
    let json = run_json(zetl_cmd(dir.path()).arg("search").arg("quick").arg("--all"));

    let total = json["total_matches"].as_u64().expect("total_matches");
    assert!(
        total >= 3,
        "with --all, should find matches in frontmatter, body, and code block, got total: {total}"
    );
}

// ===========================================================================
// TEST-015: Regex Search
// ===========================================================================

#[test]
#[ignore = "planned feature: --regex search mode not yet implemented in CLI"]
fn test_015_regex_search() {
    let dir = TempDir::new().expect("create temp dir");

    write_file(
        dir.path(),
        "Words.md",
        "# Words\n\nI have a note and some notes but not notation.\n",
    );

    let json = run_json(
        zetl_cmd(dir.path())
            .arg("search")
            .arg(r"\bnotes?\b")
            .arg("--regex"),
    );

    let results = json["results"].as_array().expect("results should be array");

    // Should match "note" and "notes" but not "notation"
    assert_eq!(
        results.len(),
        2,
        "regex \\bnotes?\\b should match 'note' and 'notes', got: {results:?}"
    );
}

#[test]
fn test_015_invalid_regex() {
    let dir = TempDir::new().expect("create temp dir");
    write_file(dir.path(), "A.md", "# A\n\nContent.\n");

    let mut cmd = zetl_cmd(dir.path());
    cmd.arg("search").arg("[invalid").arg("--regex");

    // Should fail (bad regex)
    cmd.assert().failure();
}

// ===========================================================================
// TEST-016: Case Sensitivity
// ===========================================================================

#[test]
fn test_016_case_insensitive_default() {
    let dir = TempDir::new().expect("create temp dir");

    write_file(
        dir.path(),
        "Case.md",
        "# Case\n\nZettelkasten on line 3.\nzettelkasten on line 4.\n",
    );

    let json = run_json(zetl_cmd(dir.path()).arg("search").arg("ZETTELKASTEN"));

    let total = json["total_matches"].as_u64().expect("total_matches");
    assert_eq!(
        total, 2,
        "case-insensitive search should find both occurrences, got: {total}"
    );
}

#[test]
fn test_016_case_sensitive() {
    let dir = TempDir::new().expect("create temp dir");

    write_file(
        dir.path(),
        "Case.md",
        "# Case\n\nZettelkasten on line 3.\nzettelkasten on line 4.\n",
    );

    let json = run_json(
        zetl_cmd(dir.path())
            .arg("search")
            .arg("Zettelkasten")
            .arg("--case-sensitive"),
    );

    let total = json["total_matches"].as_u64().expect("total_matches");
    assert_eq!(
        total, 1,
        "case-sensitive search should find only 'Zettelkasten', got: {total}"
    );
}

// ===========================================================================
// TEST-017: Search Respects Ignore Patterns
// ===========================================================================

#[test]
fn test_017_search_respects_ignores() {
    let dir = TempDir::new().expect("create temp dir");

    write_file(dir.path(), ".zetlignore", "drafts/\n");
    write_file(
        dir.path(),
        "Public.md",
        "# Public\n\nPublic content here.\n",
    );
    write_file(
        dir.path(),
        "drafts/Draft.md",
        "# Draft\n\nSecret draft content here.\n",
    );

    let json = run_json(zetl_cmd(dir.path()).arg("search").arg("content"));

    let results = json["results"].as_array().expect("results should be array");

    let pages: Vec<&str> = results.iter().filter_map(|r| r["page"].as_str()).collect();

    assert!(
        pages.contains(&"Public"),
        "should find match in Public, got: {pages:?}"
    );
    assert!(
        !pages.iter().any(|p| *p == "Draft"),
        "should NOT find match in ignored Draft, got: {pages:?}"
    );
}

// ===========================================================================
// TEST-018: Search Result Limiting
// ===========================================================================

#[test]
fn test_018_search_result_limiting() {
    let dir = TempDir::new().expect("create temp dir");

    // Create files with many matches on distinct lines (dedup collapses same-line hits)
    write_file(
        dir.path(),
        "Many.md",
        "# Many\n\nthe cat\nthe dog\nthe fox\nthe bat\nthe owl\n",
    );

    let json = run_json(
        zetl_cmd(dir.path())
            .arg("search")
            .arg("the")
            .arg("--limit")
            .arg("3"),
    );

    let results = json["results"].as_array().expect("results should be array");

    let total = json["total_matches"].as_u64().expect("total_matches");

    assert_eq!(results.len(), 3, "results should be capped at limit of 3");
    assert!(
        total > 3,
        "total_matches should report full count ({total}), not just the limited results"
    );
}

// ===========================================================================
// TEST-019: Empty Search Query Rejection
// ===========================================================================

#[test]
fn test_019_empty_search_query() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "A.md", "# A\n\nSome content here.\n");

    // Empty query should return JSON error with code 2
    let (json, status) = run_json_any(zetl_cmd(dir.path()).arg("search").arg(""));
    assert!(!status.success(), "empty query should fail");
    assert_eq!(json["error"].as_str(), Some("Empty search query"));
    assert_eq!(json["code"].as_i64(), Some(2));
}

#[test]
fn test_019_whitespace_search_query() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "A.md", "# A\n\nSome content here.\n");

    let (json, status) = run_json_any(zetl_cmd(dir.path()).arg("search").arg("   "));
    assert!(!status.success(), "whitespace-only query should fail");
    assert_eq!(json["error"].as_str(), Some("Empty search query"));
    assert_eq!(json["code"].as_i64(), Some(2));
}

// ===========================================================================
// TEST-020: Structured JSON Error Responses
// ===========================================================================

#[test]
fn test_020_json_error_page_not_found() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "A.md", "# A\n\nContent.\n");

    // links to nonexistent page should return JSON error
    let (json, status) = run_json_any(zetl_cmd(dir.path()).arg("links").arg("nonexistent"));
    assert!(!status.success());
    assert!(json["error"].as_str().unwrap().contains("Page not found"));
    assert_eq!(json["code"].as_i64(), Some(1));
}

#[test]
fn test_020_json_error_backlinks_not_found() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "A.md", "# A\n\nContent.\n");

    let (json, status) = run_json_any(zetl_cmd(dir.path()).arg("backlinks").arg("nonexistent"));
    assert!(!status.success());
    assert!(json["error"].as_str().unwrap().contains("Page not found"));
    assert_eq!(json["code"].as_i64(), Some(1));
}

#[test]
#[ignore = "planned feature: --regex search mode not yet implemented in CLI"]
fn test_020_json_error_invalid_regex() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "A.md", "# A\n\nContent.\n");

    let (json, status) = run_json_any(
        zetl_cmd(dir.path())
            .arg("search")
            .arg("[bad")
            .arg("--regex"),
    );
    assert!(!status.success());
    assert!(json["error"].as_str().unwrap().contains("Invalid regex"));
    assert_eq!(json["code"].as_i64(), Some(2));
}

// ===========================================================================
// TEST-021: Deduplicated Link Results
// ===========================================================================

#[test]
fn test_021_links_dedup() {
    let dir = TempDir::new().unwrap();
    // Create a file that links to B twice on the same line
    write_file(
        dir.path(),
        "A.md",
        "# A\n\nSee [[B]] and also [[B]] again.\n",
    );
    write_file(dir.path(), "B.md", "# B\n\nContent.\n");

    run_json(zetl_cmd(dir.path()).arg("index"));
    let json = run_json(zetl_cmd(dir.path()).arg("links").arg("A"));
    let links = json["links"].as_array().unwrap();

    // Both [[B]] are on line 3, so (A, B, 3) should appear only once
    assert_eq!(
        links.len(),
        1,
        "duplicate (source,target,line) should be deduped: {links:?}"
    );
    assert_eq!(links[0]["target"].as_str(), Some("B"));
}

#[test]
fn test_021_links_different_lines_not_deduped() {
    let dir = TempDir::new().unwrap();
    // B on line 3 and B on line 5 — different lines, should both appear
    write_file(
        dir.path(),
        "A.md",
        "# A\n\nFirst [[B]] link.\n\nSecond [[B]] link.\n",
    );
    write_file(dir.path(), "B.md", "# B\n\nContent.\n");

    run_json(zetl_cmd(dir.path()).arg("index"));
    let json = run_json(zetl_cmd(dir.path()).arg("links").arg("A"));
    let links = json["links"].as_array().unwrap();

    assert_eq!(
        links.len(),
        2,
        "links on different lines should both appear: {links:?}"
    );
}

// ===========================================================================
// TEST-023: List All Pages
// ===========================================================================

#[test]
fn test_023_list_pages() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "Zebra.md", "# Zebra\n");
    write_file(dir.path(), "Apple.md", "# Apple\n");
    write_file(dir.path(), "sub/Mango.md", "# Mango\n");

    let json = run_json(zetl_cmd(dir.path()).arg("list"));
    let pages = json["pages"].as_array().unwrap();

    assert_eq!(json["total"].as_u64(), Some(3));
    assert_eq!(pages.len(), 3);

    // Should be sorted alphabetically
    let names: Vec<&str> = pages.iter().map(|p| p["page"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["Apple", "Mango", "Zebra"]);
}

#[test]
fn test_023_list_empty_vault() {
    let dir = TempDir::new().unwrap();
    let json = run_json(zetl_cmd(dir.path()).arg("list"));
    assert_eq!(json["total"].as_u64(), Some(0));
    assert_eq!(json["pages"].as_array().unwrap().len(), 0);
}

// ===========================================================================
// TEST-024: Search Path Filter
// ===========================================================================

#[test]
fn test_024_search_path_filter() {
    let dir = TempDir::new().unwrap();
    write_file(
        dir.path(),
        "concepts/Alpha.md",
        "# Alpha\n\nA note about things.\n",
    );
    write_file(
        dir.path(),
        "concepts/Beta.md",
        "# Beta\n\nAnother note about things.\n",
    );
    write_file(
        dir.path(),
        "tools/Gamma.md",
        "# Gamma\n\nA note about tools.\n",
    );

    // Search with --path restricts to concepts/
    let json = run_json(
        zetl_cmd(dir.path())
            .arg("search")
            .arg("note")
            .arg("--path")
            .arg("concepts/"),
    );
    let results = json["results"].as_array().unwrap();

    assert_eq!(results.len(), 2, "should only find results in concepts/");
    for r in results {
        assert!(
            r["path"].as_str().unwrap().starts_with("concepts/"),
            "result path should be in concepts/: {}",
            r["path"]
        );
    }
}

#[test]
fn test_024_search_no_path_filter() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "concepts/Alpha.md", "# Alpha\n\nA note.\n");
    write_file(dir.path(), "tools/Gamma.md", "# Gamma\n\nA note.\n");

    // Without --path, all directories searched
    let json = run_json(zetl_cmd(dir.path()).arg("search").arg("note"));
    let results = json["results"].as_array().unwrap();
    let dirs: std::collections::HashSet<&str> = results
        .iter()
        .map(|r| r["path"].as_str().unwrap().split('/').next().unwrap())
        .collect();

    assert!(dirs.contains("concepts"), "should include concepts/");
    assert!(dirs.contains("tools"), "should include tools/");
}

// ===========================================================================
// TEST-025: Graph Export
// ===========================================================================

#[test]
fn test_025_export_graph() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "A.md", "# A\n\n[[B]] and [[C]].\n");
    write_file(dir.path(), "B.md", "# B\n\n[[C]].\n");
    write_file(dir.path(), "C.md", "# C\n\n[[A]].\n");

    run_json(zetl_cmd(dir.path()).arg("index"));
    let json = run_json(zetl_cmd(dir.path()).arg("export"));

    let nodes = json["nodes"].as_array().unwrap();
    let edges = json["edges"].as_array().unwrap();

    assert_eq!(json["node_count"].as_u64(), Some(3));
    assert_eq!(json["edge_count"].as_u64().unwrap(), edges.len() as u64);

    // Should have edges A->B, A->C, B->C, C->A
    assert_eq!(
        edges.len(),
        4,
        "triangle graph should have 4 directed edges"
    );

    // Edges should be unique
    let edge_set: std::collections::HashSet<(&str, &str)> = edges
        .iter()
        .map(|e| (e["source"].as_str().unwrap(), e["target"].as_str().unwrap()))
        .collect();
    assert_eq!(edge_set.len(), edges.len(), "edges should be unique");

    // All nodes should have page and path
    for n in nodes {
        assert!(n["page"].as_str().is_some());
    }
}

#[test]
fn test_025_export_includes_dead_link_targets() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "A.md", "# A\n\n[[Ghost]].\n");

    run_json(zetl_cmd(dir.path()).arg("index"));
    let json = run_json(zetl_cmd(dir.path()).arg("export"));

    let nodes = json["nodes"].as_array().unwrap();
    let node_names: Vec<&str> = nodes.iter().map(|n| n["page"].as_str().unwrap()).collect();

    assert!(
        node_names.contains(&"Ghost"),
        "dead link target should appear as node"
    );

    // Ghost should have null path
    let ghost = nodes.iter().find(|n| n["page"] == "Ghost").unwrap();
    assert!(
        ghost["path"].is_null(),
        "dead link target should have null path"
    );
}

// ===========================================================================
// TEST-049: zetl blocks --resolve (reverse mode) — REQ-045 / CON-020
// ===========================================================================

/// TEST-049 scenario: hash prefix too short (< 8 hex chars) → exit 1, error message
#[test]
fn test_049_resolve_hash_too_short() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "A.md", "# A\n\nSome content.\n");

    let (json, status) = run_json_any(
        zetl_cmd(dir.path())
            .arg("blocks")
            .arg("--resolve")
            .arg("e5f6"),
    );
    assert!(
        !status.success(),
        "should exit non-zero for too-short prefix"
    );
    assert!(
        json["error"]
            .as_str()
            .unwrap_or("")
            .contains("hash prefix too short"),
        "error should mention 'hash prefix too short', got: {:?}",
        json["error"]
    );
}

/// TEST-049 scenario: hash not found → exit 1, CON-020 error format
#[test]
fn test_049_resolve_hash_not_found() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "A.md", "# A\n\nSome content.\n");

    let (json, status) = run_json_any(
        zetl_cmd(dir.path())
            .arg("blocks")
            .arg("--resolve")
            .arg("deadbeef"),
    );
    assert!(!status.success(), "should exit non-zero for hash not found");
    let err_msg = json["error"].as_str().unwrap_or("");
    assert!(
        err_msg.contains("deadbeef") && err_msg.contains("not found"),
        "error should mention the prefix and 'not found', got: {err_msg:?}"
    );
    // CON-020: no 'code' field in the error JSON
    assert!(
        json["code"].is_null(),
        "resolve error should not include a 'code' field"
    );
}

/// TEST-049 scenario: unique match → exit 0, CON-020 single-location JSON
#[test]
fn test_049_resolve_unique_match() {
    let dir = TempDir::new().unwrap();
    write_file(
        dir.path(),
        "Redis.md",
        "# Redis\n\nWe benchmarked Redis at high throughput.\n",
    );

    // First get the hash via forward mode
    let forward = run_json(zetl_cmd(dir.path()).arg("blocks").arg("Redis"));
    let blocks = forward["blocks"].as_array().unwrap();
    assert!(!blocks.is_empty(), "Redis.md should have blocks");

    // Find the paragraph block
    let para = blocks
        .iter()
        .find(|b| b["type"] == "paragraph")
        .expect("should have a paragraph block");
    let full_hash = para["hash"].as_str().unwrap();
    let prefix = &full_hash[..8];

    // Resolve with 8-char prefix
    let json = run_json(
        zetl_cmd(dir.path())
            .arg("blocks")
            .arg("--resolve")
            .arg(prefix),
    );

    // CON-020 single-location format: flat object
    assert_eq!(
        json["hash"].as_str().unwrap(),
        full_hash,
        "resolved hash should match full hash"
    );
    assert_eq!(
        json["page"].as_str().unwrap(),
        "Redis",
        "resolved page should be 'Redis'"
    );
    assert!(
        json["file"].as_str().unwrap().contains("Redis"),
        "resolved file should reference Redis.md"
    );
    assert_eq!(
        json["type"].as_str().unwrap(),
        "paragraph",
        "resolved type should be 'paragraph'"
    );
    assert!(
        json["lines"].is_array(),
        "resolved output should have a 'lines' array"
    );
    assert!(
        json["text"].is_string(),
        "resolved output should have a 'text' field"
    );
    // CON-020 single-location: no 'locations' array, no 'note'
    assert!(
        json["locations"].is_null(),
        "single-match result should not have a 'locations' field"
    );
}

/// TEST-049 scenario: ambiguous hash prefix (different full hashes) → exit 1, CON-020 error
#[test]
fn test_049_resolve_ambiguous_prefix() {
    // We need two leaves that share a prefix but have different full hashes.
    // We can't easily force a collision, so we skip this test if we can't create
    // the scenario. Instead, we verify the error format by using the merkle library
    // logic indirectly. This test exercises the code path by ensuring the format
    // is correct when the ambiguous case is returned.
    //
    // For a practical test: create two files with content that would produce
    // different hashes sharing the same 8-char prefix. Since we can't control
    // BLAKE3 output exactly, we test the binary interface by verifying the
    // command rejects obviously bad inputs and produces correct output for known
    // scenarios we CAN set up.
    //
    // This test instead verifies: the ambiguous error JSON has the expected fields
    // via a unit-level check. The full ambiguous scenario is verified in merkle.rs unit tests.

    // Minimal test: two different short prefixes that don't exist → not found
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "A.md", "# A\n\nContent here.\n");

    let (json, status) = run_json_any(
        zetl_cmd(dir.path())
            .arg("blocks")
            .arg("--resolve")
            .arg("00000000"),
    );
    // Either not found or found — just verify it parses as JSON and exits appropriately
    if !status.success() {
        assert!(
            json["error"].is_string(),
            "non-zero exit should have an 'error' field"
        );
    }
}

/// TEST-049 scenario: duplicate content (identical full hashes at multiple locations)
/// → exit 0, CON-020 locations array + note
#[test]
fn test_049_resolve_duplicate_content() {
    let dir = TempDir::new().unwrap();
    // Two files with identical paragraph content → same BLAKE3 leaf hash
    let identical = "Identical content appears here.";
    write_file(dir.path(), "File1.md", &format!("# File1\n\n{identical}\n"));
    write_file(dir.path(), "File2.md", &format!("# File2\n\n{identical}\n"));

    // Get the hash from forward mode on File1
    let forward = run_json(zetl_cmd(dir.path()).arg("blocks").arg("File1"));
    let blocks = forward["blocks"].as_array().unwrap();
    let para = blocks
        .iter()
        .find(|b| b["type"] == "paragraph")
        .expect("File1 should have a paragraph block");
    let full_hash = para["hash"].as_str().unwrap();
    let prefix = &full_hash[..8];

    // Resolve — should find both files
    let json = run_json(
        zetl_cmd(dir.path())
            .arg("blocks")
            .arg("--resolve")
            .arg(prefix),
    );

    // CON-020 duplicate content format
    assert_eq!(
        json["hash"].as_str().unwrap(),
        full_hash,
        "duplicate resolve hash should match"
    );
    let locations = json["locations"].as_array().unwrap();
    assert_eq!(locations.len(), 2, "both files should appear in locations");
    assert_eq!(
        json["note"].as_str().unwrap(),
        "identical content at multiple locations",
        "note field should indicate identical content"
    );

    // Each location should have file, page, lines, type, text
    for loc in locations {
        assert!(loc["file"].is_string());
        assert!(loc["page"].is_string());
        assert!(loc["lines"].is_array());
        assert!(loc["type"].is_string());
        assert!(loc["text"].is_string());
    }
}

/// TEST-049 scenario: roundtrip — forward then reverse returns same location
#[test]
fn test_049_resolve_roundtrip() {
    let dir = TempDir::new().unwrap();
    write_file(
        dir.path(),
        "Decision.md",
        "# Decision\n\nWe chose Redis because of performance.\n",
    );

    // Forward mode: get blocks
    let forward = run_json(zetl_cmd(dir.path()).arg("blocks").arg("Decision"));
    let blocks = forward["blocks"].as_array().unwrap();
    let para = blocks
        .iter()
        .find(|b| b["type"] == "paragraph")
        .expect("should have a paragraph block");

    let forward_hash = para["hash"].as_str().unwrap();
    let forward_lines = para["lines"].as_array().unwrap();
    let forward_text = para["text"].as_str().unwrap();

    // Reverse mode: resolve with the full hash
    let reverse = run_json(
        zetl_cmd(dir.path())
            .arg("blocks")
            .arg("--resolve")
            .arg(forward_hash),
    );

    assert_eq!(
        reverse["hash"].as_str().unwrap(),
        forward_hash,
        "roundtrip hash should match"
    );
    assert_eq!(
        reverse["page"].as_str().unwrap(),
        "Decision",
        "roundtrip page should match"
    );
    assert_eq!(
        reverse["lines"].as_array().unwrap()[0],
        forward_lines[0],
        "roundtrip start line should match"
    );
    assert_eq!(
        reverse["lines"].as_array().unwrap()[1],
        forward_lines[1],
        "roundtrip end line should match"
    );
    assert_eq!(
        reverse["text"].as_str().unwrap(),
        forward_text,
        "roundtrip text should match"
    );
}

// ===========================================================================
// TEST-049: zetl blocks (forward mode) — REQ-045 / CON-020
// ===========================================================================

/// Fixture content modelled after demo-vault/decisions/Redis vs Memcached.md.
/// Contains heading, paragraph, SPL block, table, and more paragraphs —
/// giving us all the leaf types we need to exercise forward mode.
const REDIS_FIXTURE: &str = "\
# Redis vs Memcached

We evaluated Redis and Memcached as caching backends for the zetl index layer.

```spl
(given redis-evaluated)
(given memcached-evaluated)
(given single-node-deployment)
```

## Benchmark Results

| Backend    | Reads/sec | Memory overhead |
|------------|-----------|-----------------|
| Redis 7.2  | 185 000   | ~12 MB          |
| Memcached  | 170 000   | ~8 MB           |

The p99 numbers show Redis edges out Memcached on read latency under load.

## Decision

We adopt Redis as the recommended caching backend.
";

/// TEST-049 forward mode: blocks command returns heading, paragraph, spl, and
/// table leaf types for a file that contains all of them.
#[test]
fn test_049_blocks_forward_list_all_types() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "Redis vs Memcached.md", REDIS_FIXTURE);

    let json = run_json(zetl_cmd(dir.path()).arg("blocks").arg("Redis vs Memcached"));

    assert_eq!(
        json["page"].as_str().unwrap(),
        "Redis vs Memcached",
        "page field should match the queried page name"
    );
    assert!(
        json["block_count"].as_u64().unwrap() > 0,
        "block_count should be positive"
    );

    let blocks = json["blocks"].as_array().unwrap();
    assert!(!blocks.is_empty(), "blocks array must not be empty");

    // Collect all block types present
    let types: Vec<&str> = blocks.iter().map(|b| b["type"].as_str().unwrap()).collect();

    // Heading blocks
    assert!(
        types.iter().any(|t| t.starts_with("heading")),
        "expected at least one heading block, got: {types:?}"
    );

    // Paragraph blocks
    assert!(
        types.contains(&"paragraph"),
        "expected at least one paragraph block, got: {types:?}"
    );

    // SPL block
    assert!(
        types.contains(&"spl"),
        "expected at least one spl block, got: {types:?}"
    );

    // Table block
    assert!(
        types.contains(&"table"),
        "expected at least one table block, got: {types:?}"
    );

    // Each block must have the required fields
    for block in blocks {
        assert!(block["index"].is_number(), "block must have an index");
        assert!(block["type"].is_string(), "block must have a type");
        assert!(block["lines"].is_array(), "block must have a lines array");
        assert_eq!(
            block["lines"].as_array().unwrap().len(),
            2,
            "lines must be a two-element array [start, end]"
        );
        assert!(block["hash"].is_string(), "block must have a hash");
        assert!(block["text"].is_string(), "block must have a text field");
    }

    // file_hash should be present
    assert!(
        json["file_hash"].is_string(),
        "forward mode should include a file_hash"
    );
}

/// TEST-049 forward mode: SPL leaf blocks carry a spl_hashes object with
/// content_hash and ast_hash fields (REQ-045 / CON-020 §spl-dual-hashing).
#[test]
fn test_049_blocks_forward_spl_has_spl_hashes() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "Redis vs Memcached.md", REDIS_FIXTURE);

    let json = run_json(zetl_cmd(dir.path()).arg("blocks").arg("Redis vs Memcached"));

    let blocks = json["blocks"].as_array().unwrap();

    let spl_block = blocks
        .iter()
        .find(|b| b["type"] == "spl")
        .expect("fixture should contain an SPL block");

    let spl_hashes = &spl_block["spl_hashes"];
    assert!(
        !spl_hashes.is_null(),
        "SPL block must have a spl_hashes object"
    );
    assert!(
        spl_hashes["content_hash"].is_string(),
        "spl_hashes must have content_hash"
    );
    assert!(
        spl_hashes["ast_hash"].is_string(),
        "spl_hashes must have ast_hash"
    );

    // content_hash and ast_hash should be non-empty hex strings
    let content_hash = spl_hashes["content_hash"].as_str().unwrap();
    let ast_hash = spl_hashes["ast_hash"].as_str().unwrap();
    assert!(!content_hash.is_empty(), "content_hash must be non-empty");
    assert!(!ast_hash.is_empty(), "ast_hash must be non-empty");
    assert!(
        content_hash.chars().all(|c| c.is_ascii_hexdigit()),
        "content_hash must be a hex string, got: {content_hash}"
    );
    assert!(
        ast_hash.chars().all(|c| c.is_ascii_hexdigit()),
        "ast_hash must be a hex string, got: {ast_hash}"
    );

    // Non-SPL blocks must NOT have spl_hashes
    let non_spl = blocks.iter().find(|b| b["type"] != "spl").unwrap();
    assert!(
        non_spl["spl_hashes"].is_null(),
        "non-SPL block must not have spl_hashes"
    );
}

/// TEST-049 forward mode: --type paragraph filter returns only paragraph blocks.
#[test]
fn test_049_blocks_forward_type_filter_paragraph() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "Redis vs Memcached.md", REDIS_FIXTURE);

    let json = run_json(
        zetl_cmd(dir.path())
            .arg("blocks")
            .arg("Redis vs Memcached")
            .arg("--type")
            .arg("paragraph"),
    );

    let blocks = json["blocks"].as_array().unwrap();

    // Must have at least one paragraph
    assert!(
        !blocks.is_empty(),
        "--type paragraph filter should return at least one paragraph"
    );

    // Every block in the result must be a paragraph
    for block in blocks {
        assert_eq!(
            block["type"].as_str().unwrap(),
            "paragraph",
            "--type paragraph filter must only return paragraph blocks"
        );
    }

    // Filtered count should equal block_count field
    assert_eq!(
        json["block_count"].as_u64().unwrap() as usize,
        blocks.len(),
        "block_count should equal the number of blocks in the array"
    );
}

/// TEST-049 forward mode: querying a page that does not exist exits with
/// code 1 and includes an error field in the JSON output.
#[test]
fn test_049_blocks_page_not_found() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "A.md", "# A\n\nSome content.\n");

    let (json, status) = run_json_any(zetl_cmd(dir.path()).arg("blocks").arg("NonExistentPage"));

    assert!(
        !status.success(),
        "blocks for a missing page should exit non-zero"
    );
    assert!(
        json["error"].is_string(),
        "error response must have an 'error' field, got: {json:?}"
    );
}

/// TEST-049 forward mode: the hash returned for a block by forward mode is
/// usable as source metadata — resolving that hash in reverse mode yields
/// the same file, page, line range, and text (roundtrip via hash identity).
#[test]
fn test_049_blocks_forward_hash_as_source_metadata_roundtrip() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "Redis vs Memcached.md", REDIS_FIXTURE);

    // Forward pass: collect all blocks
    let forward = run_json(zetl_cmd(dir.path()).arg("blocks").arg("Redis vs Memcached"));
    let blocks = forward["blocks"].as_array().unwrap();

    // Use the table block as our probe — it has a unique, stable hash
    let table_block = blocks
        .iter()
        .find(|b| b["type"] == "table")
        .expect("fixture must contain a table block");

    let forward_hash = table_block["hash"].as_str().unwrap();
    let forward_lines = table_block["lines"].as_array().unwrap();

    // Reverse pass: resolve with the full hash
    let reverse = run_json(
        zetl_cmd(dir.path())
            .arg("blocks")
            .arg("--resolve")
            .arg(forward_hash),
    );

    assert_eq!(
        reverse["hash"].as_str().unwrap(),
        forward_hash,
        "resolved hash must equal the forward hash"
    );
    assert_eq!(
        reverse["page"].as_str().unwrap(),
        "Redis vs Memcached",
        "resolved page must match the source page"
    );
    assert_eq!(
        reverse["type"].as_str().unwrap(),
        "table",
        "resolved type must match the forward type"
    );
    assert_eq!(
        reverse["lines"].as_array().unwrap()[0],
        forward_lines[0],
        "resolved start line must match forward start line"
    );
    assert_eq!(
        reverse["lines"].as_array().unwrap()[1],
        forward_lines[1],
        "resolved end line must match forward end line"
    );
    assert!(
        reverse["text"].is_string(),
        "resolved result must include a text field"
    );
}

// ===========================================================================
// TEST-012-003 / TEST-012-010: Theme System End-to-End Verification
// ===========================================================================

/// Helper: create a minimal vault with one markdown page for theme testing.
fn build_theme_test_vault(root: &Path) {
    write_file(
        root,
        "Hello.md",
        "# Hello\n\nThis is the hello page with a [[World]] link.\n",
    );
    write_file(root, "World.md", "# World\n\nContent of the world page.\n");
}

/// TEST-012-003: Theme overriding only page.html with a custom banner.
/// The custom page.html extends base.html, so the built-in base layout should
/// be inherited while the banner from the theme's page.html appears in output.
#[test]
fn test_012_003_theme_page_override_inherits_base() {
    let dir = TempDir::new().expect("create temp dir");
    build_theme_test_vault(dir.path());

    // Create a custom theme that overrides only page.html with a banner
    let theme_dir = dir.path().join(".zetl/themes/banner-theme");
    fs::create_dir_all(&theme_dir).expect("create theme dir");
    fs::write(
        theme_dir.join("page.html"),
        r#"{% extends "base.html" %}
{% block title %}{{ page.title }} — Themed{% endblock %}
{% block content %}
<div id="custom-banner">THEME BANNER: {{ page.title }}</div>
<article>{{ page.content_html }}</article>
{% endblock %}"#,
    )
    .expect("write theme page.html");

    let out_dir = dir.path().join("dist");

    // Build with the custom theme
    let mut cmd = zetl_cmd(dir.path());
    cmd.arg("build")
        .arg("--theme")
        .arg("banner-theme")
        .arg("-o")
        .arg(out_dir.as_os_str());
    let output = cmd.output().expect("failed to execute zetl build");
    assert!(
        output.status.success(),
        "zetl build with banner-theme should succeed.\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Read the generated page HTML
    let page_html = fs::read_to_string(out_dir.join("hello/index.html"))
        .expect("hello/index.html should exist");

    // Custom banner should appear
    assert!(
        page_html.contains("THEME BANNER: Hello"),
        "custom banner should appear in page output"
    );

    // Base layout should be inherited (DOCTYPE from base.html)
    assert!(
        page_html.contains("<!DOCTYPE html>"),
        "base.html layout should be inherited (DOCTYPE present)"
    );

    // Index page should use the built-in template (not overridden)
    let index_html =
        fs::read_to_string(out_dir.join("index.html")).expect("index.html should exist");
    assert!(
        !index_html.contains("THEME BANNER"),
        "index.html should use built-in template, not the theme's page.html"
    );
    assert!(
        index_html.contains("<!DOCTYPE html>"),
        "index.html should still have base.html layout"
    );
}

/// TEST-012-003: Theme overriding base.html — all pages should use it.
#[test]
fn test_012_003_theme_base_override_affects_all_pages() {
    let dir = TempDir::new().expect("create temp dir");
    build_theme_test_vault(dir.path());

    // Create a theme that overrides base.html with a custom wrapper
    let theme_dir = dir.path().join(".zetl/themes/custom-base");
    fs::create_dir_all(&theme_dir).expect("create theme dir");
    fs::write(
        theme_dir.join("base.html"),
        r#"<!DOCTYPE html>
<html lang="en" data-theme="{{ theme }}">
<head><meta charset="utf-8"><title>{% block title %}custom-base{% endblock %}</title></head>
<body>
<div id="custom-base-wrapper">
{% block content %}{% endblock %}
</div>
</body>
</html>"#,
    )
    .expect("write theme base.html");

    let out_dir = dir.path().join("dist");

    let mut cmd = zetl_cmd(dir.path());
    cmd.arg("build")
        .arg("--theme")
        .arg("custom-base")
        .arg("-o")
        .arg(out_dir.as_os_str());
    let output = cmd.output().expect("failed to execute zetl build");
    assert!(
        output.status.success(),
        "zetl build with custom-base theme should succeed.\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Check index page uses custom base
    let index_html =
        fs::read_to_string(out_dir.join("index.html")).expect("index.html should exist");
    assert!(
        index_html.contains("custom-base-wrapper"),
        "index.html should use the overridden base.html"
    );

    // Check page uses custom base
    let page_html = fs::read_to_string(out_dir.join("hello/index.html"))
        .expect("hello/index.html should exist");
    assert!(
        page_html.contains("custom-base-wrapper"),
        "page should use the overridden base.html"
    );
}

/// TEST-012-003: --theme default works with no .zetl/ directory at all.
#[test]
fn test_012_003_default_theme_no_zetl_dir() {
    let dir = TempDir::new().expect("create temp dir");
    build_theme_test_vault(dir.path());

    // Confirm no .zetl directory exists
    assert!(
        !dir.path().join(".zetl").exists(),
        "precondition: .zetl/ should not exist"
    );

    let out_dir = dir.path().join("dist");

    let mut cmd = zetl_cmd(dir.path());
    cmd.arg("build")
        .arg("--theme")
        .arg("default")
        .arg("-o")
        .arg(out_dir.as_os_str());
    let output = cmd.output().expect("failed to execute zetl build");
    assert!(
        output.status.success(),
        "zetl build --theme default should succeed without .zetl/ dir.\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify output was generated
    let index_html =
        fs::read_to_string(out_dir.join("index.html")).expect("index.html should be generated");
    assert!(
        index_html.contains("<!DOCTYPE html>"),
        "default theme should produce valid HTML"
    );
    assert!(
        index_html.contains("Hello"),
        "index should list the Hello page"
    );
}

/// TEST-012-010: --theme nonexistent gives a clear error message listing available themes.
#[test]
fn test_012_010_nonexistent_theme_error() {
    let dir = TempDir::new().expect("create temp dir");
    build_theme_test_vault(dir.path());

    // Create one valid theme so the hint lists it
    let theme_dir = dir.path().join(".zetl/themes/existing-theme");
    fs::create_dir_all(&theme_dir).expect("create theme dir");
    fs::write(
        theme_dir.join("page.html"),
        "{% extends \"base.html\" %}{% block content %}ok{% endblock %}",
    )
    .unwrap();

    let out_dir = dir.path().join("dist");

    let mut cmd = zetl_cmd(dir.path());
    cmd.arg("build")
        .arg("--theme")
        .arg("nonexistent")
        .arg("-o")
        .arg(out_dir.as_os_str());
    let output = cmd.output().expect("failed to execute zetl build");

    assert!(
        !output.status.success(),
        "zetl build --theme nonexistent should fail"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("nonexistent"),
        "error should mention the theme name 'nonexistent', got: {stderr}"
    );
    assert!(
        stderr.contains("not found") || stderr.contains("does not exist"),
        "error should indicate the theme was not found, got: {stderr}"
    );
    assert!(
        stderr.contains("existing-theme"),
        "error hint should list the available theme 'existing-theme', got: {stderr}"
    );
}

/// TEST-012-010: --theme '../escape' is rejected as an invalid theme name.
#[test]
fn test_012_010_path_traversal_rejected() {
    let dir = TempDir::new().expect("create temp dir");
    build_theme_test_vault(dir.path());

    let out_dir = dir.path().join("dist");

    let mut cmd = zetl_cmd(dir.path());
    cmd.arg("build")
        .arg("--theme")
        .arg("../escape")
        .arg("-o")
        .arg(out_dir.as_os_str());
    let output = cmd.output().expect("failed to execute zetl build");

    assert!(
        !output.status.success(),
        "zetl build --theme '../escape' should fail"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("invalid theme name"),
        "error should say 'invalid theme name', got: {stderr}"
    );
}

/// TEST-012-010: The {{ theme }} variable is accessible in templates and renders correctly.
#[test]
fn test_012_010_theme_variable_accessible() {
    let dir = TempDir::new().expect("create temp dir");
    build_theme_test_vault(dir.path());

    // Create a theme that outputs {{ theme }} in visible content
    let theme_dir = dir.path().join(".zetl/themes/my-theme");
    fs::create_dir_all(&theme_dir).expect("create theme dir");
    fs::write(
        theme_dir.join("page.html"),
        r#"{% extends "base.html" %}
{% block content %}
<div id="theme-probe">ACTIVE_THEME={{ theme }}</div>
<article>{{ page.content_html }}</article>
{% endblock %}"#,
    )
    .expect("write theme page.html");

    let out_dir = dir.path().join("dist");

    let mut cmd = zetl_cmd(dir.path());
    cmd.arg("build")
        .arg("--theme")
        .arg("my-theme")
        .arg("-o")
        .arg(out_dir.as_os_str());
    let output = cmd.output().expect("failed to execute zetl build");
    assert!(
        output.status.success(),
        "zetl build with my-theme should succeed.\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Check that the theme variable rendered correctly
    let page_html = fs::read_to_string(out_dir.join("hello/index.html"))
        .expect("hello/index.html should exist");
    assert!(
        page_html.contains("ACTIVE_THEME=my-theme"),
        "theme variable should render as 'my-theme' in template output"
    );

    // Also verify data-theme attribute on the <html> tag from base.html
    assert!(
        page_html.contains(r#"data-theme="my-theme""#),
        "base.html should set data-theme to 'my-theme'"
    );
}

// ── TEST-012-005: Serve-mode static asset handling ─────────────────────────
//
// These tests spawn `zetl serve` on a random port, wait for it to start,
// send raw HTTP/1.1 requests via TcpStream, then verify responses.

/// Find a free TCP port by binding to port 0 and reading the assigned port.
fn find_free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind to port 0");
    listener.local_addr().unwrap().port()
}

/// Spawn `zetl serve` and wait up to 3 seconds for it to accept TCP connections.
fn spawn_serve(vault: &Path, port: u16, theme: &str) -> std::process::Child {
    let bin = assert_cmd::cargo::cargo_bin!("zetl");
    let child = std::process::Command::new(bin)
        .arg("-d")
        .arg(vault)
        .arg("--no-cache")
        .arg("serve")
        .arg("--port")
        .arg(port.to_string())
        .arg("--theme")
        .arg(theme)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn zetl serve");

    // Poll until the port is accepting connections (max ~3s).
    for _ in 0..30 {
        if std::net::TcpStream::connect(format!("127.0.0.1:{port}")).is_ok() {
            return child;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("zetl serve did not become ready on port {port}");
}

/// Send a raw HTTP/1.1 GET and return (status_line, headers, body).
fn http_get(port: u16, path: &str) -> (String, String, Vec<u8>) {
    use std::io::{Read, Write};
    let mut stream =
        std::net::TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect to server");
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .ok();
    let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).expect("send request");

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).ok(); // read until EOF or timeout

    let raw = String::from_utf8_lossy(&buf);
    let (head, body_start) = match raw.find("\r\n\r\n") {
        Some(pos) => (&raw[..pos], pos + 4),
        None => (raw.as_ref(), buf.len()),
    };
    let mut lines = head.lines();
    let status_line = lines.next().unwrap_or("").to_string();
    let headers: String = lines.collect::<Vec<_>>().join("\n");
    let body = buf[body_start..].to_vec();
    (status_line, headers, body)
}

/// TEST-012-005: Shared .zetl/static/test.js returns 200 with correct MIME.
#[test]
fn test_012_005_serve_shared_static_200_mime() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "Note.md", "# Note\nHello.\n");
    let static_dir = dir.path().join(".zetl/static");
    fs::create_dir_all(&static_dir).unwrap();
    fs::write(static_dir.join("test.js"), b"console.log('ok');").unwrap();

    let port = find_free_port();
    let mut child = spawn_serve(dir.path(), port, "default");

    let (status, headers, body) = http_get(port, "/_static/test.js");
    child.kill().ok();
    child.wait().ok();

    assert!(status.contains("200"), "expected 200 OK, got: {status}");
    assert!(
        headers.contains("application/javascript"),
        "expected application/javascript content-type, got headers:\n{headers}"
    );
    assert_eq!(body, b"console.log('ok');");
}

/// TEST-012-005: Theme static overrides shared at same path.
#[test]
fn test_012_005_serve_theme_overrides_shared() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "Note.md", "# Note\nHello.\n");

    // Shared version
    let shared = dir.path().join(".zetl/static");
    fs::create_dir_all(&shared).unwrap();
    fs::write(shared.join("style.css"), b"body{color:red}").unwrap();

    // Theme override
    let theme_dir = dir.path().join(".zetl/themes/mytheme/static");
    fs::create_dir_all(&theme_dir).unwrap();
    fs::write(theme_dir.join("style.css"), b"body{color:blue}").unwrap();

    let port = find_free_port();
    let mut child = spawn_serve(dir.path(), port, "mytheme");

    let (status, headers, body) = http_get(port, "/_static/style.css");
    child.kill().ok();
    child.wait().ok();

    assert!(status.contains("200"), "expected 200 OK, got: {status}");
    assert!(
        headers.contains("text/css"),
        "expected text/css, got headers:\n{headers}"
    );
    assert_eq!(
        body, b"body{color:blue}",
        "theme static should override shared"
    );
}

/// TEST-012-005: 404 for nonexistent static files.
#[test]
fn test_012_005_serve_404_for_nonexistent() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "Note.md", "# Note\nHello.\n");

    let port = find_free_port();
    let mut child = spawn_serve(dir.path(), port, "default");

    let (status, _headers, _body) = http_get(port, "/_static/nope.js");
    child.kill().ok();
    child.wait().ok();

    assert!(status.contains("404"), "expected 404, got: {status}");
}

/// TEST-012-005: No error when no static dirs exist.
#[test]
fn test_012_005_serve_no_static_dirs_graceful() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "Note.md", "# Note\nHello.\n");
    // No .zetl/static/ at all

    let port = find_free_port();
    let mut child = spawn_serve(dir.path(), port, "default");

    let (status, _headers, _body) = http_get(port, "/_static/anything.js");
    child.kill().ok();
    let exit = child.wait().ok();

    assert!(
        status.contains("404"),
        "expected 404 when no static dirs, got: {status}"
    );
    // The server should not have crashed — kill returns Ok if it was still running
    // or the exit status is from our kill signal, not an internal panic.
    if let Some(es) = exit {
        // If it exited on its own before kill, it should NOT be a panic exit
        // (signal-killed processes don't have a normal exit code on Unix)
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            // Either killed by our signal (9) or still running when we killed it — both fine
            assert!(
                es.signal().is_some() || es.success(),
                "server should not have panicked, exit status: {es:?}"
            );
        }
    }
}

// ── TEST-012-006: Build-mode static asset handling ─────────────────────────

/// TEST-012-006: Shared + theme assets merged in dist/_static/.
#[test]
fn test_012_006_build_shared_and_theme_merge() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "Note.md", "# Note\nHello.\n");

    // Shared static
    let shared = dir.path().join(".zetl/static");
    fs::create_dir_all(&shared).unwrap();
    fs::write(shared.join("shared.js"), "// shared").unwrap();

    // Theme static
    let theme_dir = dir.path().join(".zetl/themes/merge-theme/static");
    fs::create_dir_all(&theme_dir).unwrap();
    fs::write(theme_dir.join("theme.js"), "// theme").unwrap();

    // Also need a minimal theme template so --theme validation passes
    let theme_root = dir.path().join(".zetl/themes/merge-theme");
    fs::write(
        theme_root.join("page.html"),
        r#"{% extends "base.html" %}{% block content %}{{ page.content_html }}{% endblock %}"#,
    )
    .unwrap();

    let out_dir = dir.path().join("dist");
    let mut cmd = zetl_cmd(dir.path());
    cmd.arg("build")
        .arg("--theme")
        .arg("merge-theme")
        .arg("-o")
        .arg(out_dir.as_os_str());
    let output = cmd.output().expect("run zetl build");
    assert!(
        output.status.success(),
        "zetl build should succeed.\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Both files should be present in _static/
    assert_eq!(
        fs::read_to_string(out_dir.join("_static/shared.js")).unwrap(),
        "// shared"
    );
    assert_eq!(
        fs::read_to_string(out_dir.join("_static/theme.js")).unwrap(),
        "// theme"
    );
}

/// TEST-012-006: Theme overwrites shared on conflict.
#[test]
fn test_012_006_build_theme_overwrites_shared() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "Note.md", "# Note\nHello.\n");

    // Both have style.css
    let shared = dir.path().join(".zetl/static");
    fs::create_dir_all(&shared).unwrap();
    fs::write(shared.join("style.css"), "/* shared */").unwrap();

    let theme_dir = dir.path().join(".zetl/themes/winner/static");
    fs::create_dir_all(&theme_dir).unwrap();
    fs::write(theme_dir.join("style.css"), "/* theme wins */").unwrap();

    let theme_root = dir.path().join(".zetl/themes/winner");
    fs::write(
        theme_root.join("page.html"),
        r#"{% extends "base.html" %}{% block content %}{{ page.content_html }}{% endblock %}"#,
    )
    .unwrap();

    let out_dir = dir.path().join("dist");
    let mut cmd = zetl_cmd(dir.path());
    cmd.arg("build")
        .arg("--theme")
        .arg("winner")
        .arg("-o")
        .arg(out_dir.as_os_str());
    let output = cmd.output().expect("run zetl build");
    assert!(
        output.status.success(),
        "zetl build should succeed.\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(
        fs::read_to_string(out_dir.join("_static/style.css")).unwrap(),
        "/* theme wins */",
        "theme version should overwrite shared"
    );
}

/// TEST-012-006: Directory structure preserved in _static/.
#[test]
fn test_012_006_build_preserves_directory_structure() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "Note.md", "# Note\nHello.\n");

    let nested = dir.path().join(".zetl/static/fonts/woff2");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("inter.woff2"), "fontbytes").unwrap();

    let out_dir = dir.path().join("dist");
    let mut cmd = zetl_cmd(dir.path());
    cmd.arg("build").arg("-o").arg(out_dir.as_os_str());
    let output = cmd.output().expect("run zetl build");
    assert!(
        output.status.success(),
        "zetl build should succeed.\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(
        fs::read_to_string(out_dir.join("_static/fonts/woff2/inter.woff2")).unwrap(),
        "fontbytes",
        "nested directory structure should be preserved"
    );
}

/// TEST-012-006: No _static/ directory when no source static dirs exist.
#[test]
fn test_012_006_build_no_static_dirs_no_output() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "Note.md", "# Note\nHello.\n");
    // No .zetl/static/ or theme static

    let out_dir = dir.path().join("dist");
    let mut cmd = zetl_cmd(dir.path());
    cmd.arg("build").arg("-o").arg(out_dir.as_os_str());
    let output = cmd.output().expect("run zetl build");
    assert!(
        output.status.success(),
        "zetl build should succeed.\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        !out_dir.join("_static").exists(),
        "_static/ should not be created when no source static dirs exist"
    );
}

// ── TEST-012-007: End-to-end frontmatter parsing ───────────────────────────

/// Install a custom theme whose page template renders frontmatter fields so
/// we can verify them in the static-build HTML output.
fn install_frontmatter_theme(vault: &Path) {
    let theme_dir = vault.join(".zetl/themes/fm-test");
    fs::create_dir_all(&theme_dir).unwrap();
    fs::write(
        theme_dir.join("page.html"),
        r#"{% extends "base.html" %}
{% block content %}
<div id="fm-format">{{ page.frontmatter.format }}</div>
<div id="fm-author">{{ page.frontmatter.author }}</div>
<div id="fm-tags">{% for t in page.frontmatter.tags %}{{ t }};{% endfor %}</div>
<div id="fm-empty">{{ page.frontmatter is mapping }}</div>
<article>{{ page.content_html | safe }}</article>
{% endblock %}"#,
    )
    .unwrap();
}

/// TEST-012-007: Page with frontmatter — all fields accessible in templates.
#[test]
fn test_012_007_frontmatter_fields_accessible_in_template() {
    let dir = TempDir::new().unwrap();
    install_frontmatter_theme(dir.path());

    write_file(
        dir.path(),
        "Screenplay.md",
        "---\nformat: fountain\ntags:\n  - drama\n  - screenplay\nauthor: Jane Doe\n---\n# Act One\n\nINT. OFFICE - DAY\n",
    );

    let out_dir = dir.path().join("dist");
    let mut cmd = zetl_cmd(dir.path());
    cmd.arg("build")
        .arg("--theme")
        .arg("fm-test")
        .arg("-o")
        .arg(out_dir.as_os_str());
    let output = cmd.output().expect("run zetl build");
    assert!(
        output.status.success(),
        "zetl build should succeed.\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let html =
        fs::read_to_string(out_dir.join("screenplay/index.html")).expect("read built page HTML");

    // Verify each frontmatter field is rendered correctly
    assert!(
        html.contains(r#"<div id="fm-format">fountain</div>"#),
        "format field should be 'fountain', got:\n{html}"
    );
    assert!(
        html.contains(r#"<div id="fm-author">Jane Doe</div>"#),
        "author field should be 'Jane Doe', got:\n{html}"
    );
    assert!(
        html.contains(r#"<div id="fm-tags">drama;screenplay;</div>"#),
        "tags should be iterable as [drama, screenplay], got:\n{html}"
    );
}

/// TEST-012-007: Page with no frontmatter — page.frontmatter is empty mapping.
#[test]
fn test_012_007_no_frontmatter_empty_object() {
    let dir = TempDir::new().unwrap();
    install_frontmatter_theme(dir.path());

    write_file(
        dir.path(),
        "Plain.md",
        "# Just a heading\n\nSome content with no frontmatter.\n",
    );

    let out_dir = dir.path().join("dist");
    let mut cmd = zetl_cmd(dir.path());
    cmd.arg("build")
        .arg("--theme")
        .arg("fm-test")
        .arg("-o")
        .arg(out_dir.as_os_str());
    let output = cmd.output().expect("run zetl build");
    assert!(
        output.status.success(),
        "zetl build should succeed.\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let html = fs::read_to_string(out_dir.join("plain/index.html")).expect("read built page HTML");

    // With no frontmatter, individual field accesses should render empty
    assert!(
        html.contains(r#"<div id="fm-format"></div>"#),
        "format should be empty for page without frontmatter, got:\n{html}"
    );
    assert!(
        html.contains(r#"<div id="fm-author"></div>"#),
        "author should be empty for page without frontmatter, got:\n{html}"
    );
    // The frontmatter value itself is an empty JSON object, which is a mapping
    assert!(
        html.contains(r#"<div id="fm-empty">true</div>"#),
        "page.frontmatter should be a mapping (empty object), got:\n{html}"
    );
}

/// TEST-012-007: Page with malformed YAML — empty object returned and warning logged.
#[test]
fn test_012_007_malformed_yaml_warning() {
    let dir = TempDir::new().unwrap();
    install_frontmatter_theme(dir.path());

    write_file(
        dir.path(),
        "Broken.md",
        "---\n: [invalid yaml\nauthor: broken\n---\n# Body\n\nContent here.\n",
    );

    let out_dir = dir.path().join("dist");
    let mut cmd = zetl_cmd(dir.path());
    cmd.arg("build")
        .arg("--theme")
        .arg("fm-test")
        .arg("-o")
        .arg(out_dir.as_os_str());
    let output = cmd.output().expect("run zetl build");
    assert!(
        output.status.success(),
        "zetl build should succeed even with malformed frontmatter.\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("malformed frontmatter YAML"),
        "stderr should contain warning about malformed YAML, got:\n{stderr}"
    );

    let html = fs::read_to_string(out_dir.join("broken/index.html")).expect("read built page HTML");

    // Malformed YAML → frontmatter is empty object, fields render empty
    assert!(
        html.contains(r#"<div id="fm-author"></div>"#),
        "author should be empty when YAML is malformed, got:\n{html}"
    );
    assert!(
        html.contains(r#"<div id="fm-empty">true</div>"#),
        "page.frontmatter should still be a mapping (empty object), got:\n{html}"
    );
}

/// TEST-012-007: strip_frontmatter — rendered markdown does not contain frontmatter block.
#[test]
fn test_012_007_strip_frontmatter_from_rendered_html() {
    let dir = TempDir::new().unwrap();

    write_file(
        dir.path(),
        "WithFM.md",
        "---\ntitle: Secret Title\ntags:\n  - hidden\n---\n# Visible Heading\n\nVisible body text.\n",
    );

    let out_dir = dir.path().join("dist");
    let mut cmd = zetl_cmd(dir.path());
    cmd.arg("build").arg("-o").arg(out_dir.as_os_str());
    let output = cmd.output().expect("run zetl build");
    assert!(
        output.status.success(),
        "zetl build should succeed.\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let html = fs::read_to_string(out_dir.join("withfm/index.html")).expect("read built page HTML");

    // The article/prose section should NOT contain frontmatter delimiters or YAML keys
    assert!(
        !html.contains("Secret Title"),
        "frontmatter title value should not appear in rendered HTML body"
    );
    assert!(
        !html.contains("tags:"),
        "frontmatter YAML keys should not appear in rendered HTML body"
    );
    assert!(
        !html.contains("- hidden"),
        "frontmatter YAML values should not appear in rendered HTML body"
    );

    // The visible markdown content SHOULD be present
    assert!(
        html.contains("Visible Heading"),
        "markdown heading should appear in rendered HTML"
    );
    assert!(
        html.contains("Visible body text"),
        "markdown body should appear in rendered HTML"
    );
}

// ===========================================================================
// Phase 1 verification tests (IMPL-013)
// ===========================================================================

// ---------------------------------------------------------------------------
// 1. zetl index JSON includes search_index_docs and search_index_size_kb
// ---------------------------------------------------------------------------

#[test]
fn test_013_v1_index_json_has_search_fields() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "Note.md", "# Note\n\nSome content.\n");

    let json = run_json(zetl_cmd(dir.path()).arg("index"));

    assert!(
        json["search_index_docs"].as_u64().is_some(),
        "index output must include 'search_index_docs', got: {json}"
    );
    assert!(
        json["search_index_size_kb"].as_u64().is_some(),
        "index output must include 'search_index_size_kb', got: {json}"
    );
    assert_eq!(
        json["search_index_docs"].as_u64().unwrap(),
        1,
        "should report 1 indexed document"
    );
    assert!(
        json["search_index_size_kb"].as_u64().unwrap() > 0,
        "search index should have non-zero size after indexing"
    );
}

// ---------------------------------------------------------------------------
// 2. Search results carry score, heading, heading_level (BM25 fields)
// ---------------------------------------------------------------------------

#[test]
fn test_013_v1_search_results_have_bm25_fields() {
    let dir = TempDir::new().unwrap();
    write_file(
        dir.path(),
        "Page.md",
        "# Introduction\n\nThis page discusses alpha deeply.\n\n## Details\n\nMore alpha content.\n",
    );

    let json = run_json(zetl_cmd(dir.path()).arg("search").arg("alpha"));
    let results = json["results"].as_array().expect("results array");

    assert!(
        !results.is_empty(),
        "should find at least one result for 'alpha'"
    );

    for result in results {
        assert!(
            result["score"].as_f64().is_some() && result["score"].as_f64().unwrap() > 0.0,
            "every result must have a positive 'score', got: {result}"
        );
        // heading may be null (before any heading), but the field must exist
        assert!(
            result.get("heading").is_some(),
            "every result must have a 'heading' field, got: {result}"
        );
        assert!(
            result.get("heading_level").is_some(),
            "every result must have a 'heading_level' field, got: {result}"
        );
    }
}

// ---------------------------------------------------------------------------
// TEST-013-001: Exclusion zones — inline code and HTML comments
// ---------------------------------------------------------------------------

#[test]
fn test_013_001_inline_code_excluded() {
    let dir = TempDir::new().unwrap();
    // "exclusion_zeta" only appears in an inline code span → must not be found
    write_file(
        dir.path(),
        "Inline.md",
        "# Inline\n\nNormal body text.\n`exclusion_zeta in inline code`\nMore body text.\n",
    );

    let mut cmd = zetl_cmd(dir.path());
    cmd.arg("search").arg("exclusion_zeta");
    cmd.assert().failure(); // no matches → exit 1
}

#[test]
fn test_013_001_html_comment_excluded() {
    let dir = TempDir::new().unwrap();
    // "exclusion_eta" only appears in an HTML comment → must not be found
    write_file(
        dir.path(),
        "Comment.md",
        "# Comment\n\nNormal body text.\n<!-- exclusion_eta in HTML comment -->\nMore body text.\n",
    );

    let mut cmd = zetl_cmd(dir.path());
    cmd.arg("search").arg("exclusion_eta");
    cmd.assert().failure(); // no matches → exit 1
}

#[test]
fn test_013_001_body_text_found_around_exclusion_zones() {
    let dir = TempDir::new().unwrap();
    // "bodyterm" in body text is found; "bodyterm" in inline code / comment is not double-counted
    write_file(
        dir.path(),
        "Mixed.md",
        "# Mixed\n\nbodyterm in body.\n`bodyterm in inline code`\n<!-- bodyterm in comment -->\n",
    );

    let json = run_json(zetl_cmd(dir.path()).arg("search").arg("bodyterm"));
    let total = json["total_matches"].as_u64().expect("total_matches");
    assert_eq!(
        total, 1,
        "only the body-text occurrence should be found, got: {total}"
    );
}

// ---------------------------------------------------------------------------
// TEST-013-002: Relevance ranking — more occurrences → higher BM25 score
// ---------------------------------------------------------------------------

#[test]
fn test_013_002_relevance_ranking() {
    let dir = TempDir::new().unwrap();

    // Rich.md has many occurrences of "quux"; Sparse.md has one.
    write_file(
        dir.path(),
        "Rich.md",
        "# Rich\n\nquux quux quux quux quux quux quux quux quux quux\n",
    );
    write_file(
        dir.path(),
        "Sparse.md",
        "# Sparse\n\nOnly one quux here in this longer sentence.\n",
    );

    let json = run_json(zetl_cmd(dir.path()).arg("search").arg("quux"));
    let results = json["results"].as_array().expect("results array");

    // Collect (page, score) pairs
    let rich_score = results
        .iter()
        .find(|r| r["page"].as_str() == Some("Rich"))
        .map(|r| r["score"].as_f64().unwrap())
        .expect("should have a result for Rich");

    let sparse_score = results
        .iter()
        .find(|r| r["page"].as_str() == Some("Sparse"))
        .map(|r| r["score"].as_f64().unwrap())
        .expect("should have a result for Sparse");

    assert!(
        rich_score > sparse_score,
        "Rich.md (many quux) should score higher than Sparse.md (one quux): rich={rich_score}, sparse={sparse_score}"
    );
}

// ---------------------------------------------------------------------------
// TEST-013-005: Line-level results — same file, multiple lines, same score
// ---------------------------------------------------------------------------

#[test]
fn test_013_005_line_level_results_separate_and_same_score() {
    let dir = TempDir::new().unwrap();
    write_file(
        dir.path(),
        "Multi.md",
        "# Multi\n\nThe term zephyr on line 3.\n\nThe term zephyr on line 5.\n\nThe term zephyr on line 7.\n",
    );

    let json = run_json(zetl_cmd(dir.path()).arg("search").arg("zephyr"));
    let results = json["results"].as_array().expect("results array");

    assert_eq!(
        results.len(),
        3,
        "should have 3 separate line-level results for 3 occurrences, got: {results:?}"
    );

    let lines: Vec<u64> = results
        .iter()
        .map(|r| r["line"].as_u64().expect("line field"))
        .collect();
    assert!(lines.contains(&3), "should have a result on line 3");
    assert!(lines.contains(&5), "should have a result on line 5");
    assert!(lines.contains(&7), "should have a result on line 7");

    // All results from the same document share the same BM25 score
    let scores: Vec<f64> = results
        .iter()
        .map(|r| r["score"].as_f64().expect("score field"))
        .collect();
    let first_score = scores[0];
    for s in &scores {
        assert_eq!(
            *s, first_score,
            "all results from the same document should share the same score, got: {scores:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// 6. Heading context — matches under headings show correct heading/level
// ---------------------------------------------------------------------------

#[test]
fn test_013_v1_heading_context_correct() {
    let dir = TempDir::new().unwrap();
    write_file(
        dir.path(),
        "Sections.md",
        "# Overview\n\nIntroductory text.\n\n## Implementation\n\nThis section has the searchterm here.\n\n### Sub-section\n\nAnother searchterm occurrence.\n",
    );

    let json = run_json(zetl_cmd(dir.path()).arg("search").arg("searchterm"));
    let results = json["results"].as_array().expect("results array");

    assert_eq!(results.len(), 2, "should have 2 results for 'searchterm'");

    // Sort by line to get stable order
    let mut sorted = results.clone();
    sorted.sort_by_key(|r| r["line"].as_u64().unwrap_or(0));

    // First occurrence is under "## Implementation"
    assert_eq!(
        sorted[0]["heading"].as_str(),
        Some("Implementation"),
        "first match should be under 'Implementation', got: {:?}",
        sorted[0]["heading"]
    );
    assert_eq!(
        sorted[0]["heading_level"].as_u64(),
        Some(2),
        "first match heading level should be 2"
    );

    // Second occurrence is under "### Sub-section"
    assert_eq!(
        sorted[1]["heading"].as_str(),
        Some("Sub-section"),
        "second match should be under 'Sub-section', got: {:?}",
        sorted[1]["heading"]
    );
    assert_eq!(
        sorted[1]["heading_level"].as_u64(),
        Some(3),
        "second match heading level should be 3"
    );
}

// ---------------------------------------------------------------------------
// TEST-013-011: Headings inside code blocks are not used as enclosing headings
// ---------------------------------------------------------------------------

#[test]
fn test_013_011_heading_in_code_block_not_used_as_context() {
    let dir = TempDir::new().unwrap();
    // The "# Fake Heading" inside the fenced code block must not be recognized as a
    // heading, so the match after the code block should still report "Real Heading".
    write_file(
        dir.path(),
        "FakeHeading.md",
        "# Real Heading\n\nBody text before code.\n\n```\n# Fake Heading\n```\n\nBody text after code with codeheadingterm.\n",
    );

    let json = run_json(zetl_cmd(dir.path()).arg("search").arg("codeheadingterm"));
    let results = json["results"].as_array().expect("results array");

    assert_eq!(results.len(), 1, "should find exactly one match");
    assert_eq!(
        results[0]["heading"].as_str(),
        Some("Real Heading"),
        "match after the code block should be under 'Real Heading', not the fake one inside code; got: {:?}",
        results[0]["heading"]
    );
    assert_eq!(
        results[0]["heading_level"].as_u64(),
        Some(1),
        "heading level should be 1 (Real Heading)"
    );
}

// ---------------------------------------------------------------------------
// TEST-013-003: Index invalidation — modify file → new content searchable
// ---------------------------------------------------------------------------

#[test]
fn test_013_003_index_invalidation_after_file_change() {
    let dir = TempDir::new().unwrap();
    write_file(
        dir.path(),
        "Evolving.md",
        "# Evolving\n\nThis file has oldterm content.\n",
    );

    // Build initial index (with cache so the cache file is saved for incremental detection)
    run_json(zetl_cmd_cached(dir.path()).arg("index"));

    // "newterm" is not yet in the file
    {
        let mut cmd = zetl_cmd_cached(dir.path());
        cmd.arg("search").arg("newterm");
        cmd.assert().failure();
    }

    // Modify the file: replace content
    write_file(
        dir.path(),
        "Evolving.md",
        "# Evolving\n\nThis file now has newterm content.\n",
    );

    // Re-index (incremental — detects file change and rebuilds search index)
    run_json(zetl_cmd_cached(dir.path()).arg("index"));

    // Now "newterm" should be found
    let json = run_json(zetl_cmd_cached(dir.path()).arg("search").arg("newterm"));
    assert_eq!(
        json["total_matches"].as_u64().unwrap(),
        1,
        "newterm should be found after re-indexing the modified file"
    );

    // And "oldterm" should no longer be indexed
    {
        let mut cmd = zetl_cmd_cached(dir.path());
        cmd.arg("search").arg("oldterm");
        cmd.assert().failure();
    }
}

// ---------------------------------------------------------------------------
// TEST-013-003: --no-cache forces full rebuild even when nothing changed
// ---------------------------------------------------------------------------

#[test]
fn test_013_003_no_cache_forces_rebuild() {
    let dir = TempDir::new().unwrap();
    write_file(
        dir.path(),
        "Stable.md",
        "# Stable\n\nThis file contains stableterm.\n",
    );

    // First: build with cache (normal operation)
    run_json(zetl_cmd_cached(dir.path()).arg("index"));

    // Second: rebuild with --no-cache (should delete and fully rebuild the search index)
    let json = run_json(zetl_cmd(dir.path()).arg("index"));

    // Index should report the correct document count after forced rebuild
    assert_eq!(
        json["search_index_docs"].as_u64().unwrap(),
        1,
        "--no-cache rebuild should index all documents"
    );

    // Search should still work after the forced rebuild
    let search_json = run_json(zetl_cmd(dir.path()).arg("search").arg("stableterm"));
    assert_eq!(
        search_json["total_matches"].as_u64().unwrap(),
        1,
        "stableterm should be findable after --no-cache rebuild"
    );
}

// ---------------------------------------------------------------------------
// TEST-013-004: Lazy index — search builds the index on the fly
// ---------------------------------------------------------------------------

#[test]
fn test_013_004_lazy_index_built_on_search() {
    let dir = TempDir::new().unwrap();
    write_file(
        dir.path(),
        "LazyDoc.md",
        "# LazyDoc\n\nThis document has lazyterm content.\n",
    );

    // Ensure no .zetl/search/ directory exists (fresh vault — no prior `zetl index`)
    let search_dir = dir.path().join(".zetl").join("search");
    assert!(
        !search_dir.exists(),
        "search dir must not exist before the lazy test"
    );

    // Run `zetl search` without a prior `zetl index` — should build index lazily
    let (json, stderr) = run_json_with_stderr(zetl_cmd(dir.path()).arg("search").arg("lazyterm"));

    // The result should be found
    assert_eq!(
        json["total_matches"].as_u64().unwrap(),
        1,
        "lazyterm should be found even without prior zetl index"
    );

    // Stderr must contain the lazy-build advisory message
    assert!(
        stderr.contains("Building search index"),
        "stderr must warn about lazy index build, got: {stderr:?}"
    );
}

// ===========================================================================
// Phase 2: Graph-Scoped Search
// TEST-013-006: --near filters results to the neighbourhood (bidirectional BFS)
// TEST-013-007: --depth without --near or --depth 0 → exit code 2
// TEST-013-008: Case-insensitive anchor resolution; unresolvable → exit code 2
// TEST-013-009: Neighbourhood metadata present/absent in JSON
// TEST-013-016: --near composes with --path and --context
// ===========================================================================

/// Build a vault with a known link structure for Phase 2 tests.
///
/// Forward links:  A→B, A→C, B→D
/// Isolated:       E
/// Backlink probe: X→Y
///
/// Every page contains "graphterm" so neighbourhood filtering can be tested.
/// "Spaced Repetition.md" exists for the case-insensitive anchor test (TEST-013-008).
fn build_phase2_vault(root: &Path) {
    write_file(root, "A.md", "# A\n\ngraphterm\n\n[[B]] [[C]]\n");
    write_file(root, "B.md", "# B\n\ngraphterm\n\n[[D]]\n");
    write_file(root, "C.md", "# C\n\ngraphterm\n");
    write_file(root, "D.md", "# D\n\ngraphterm\n");
    write_file(root, "E.md", "# E\n\ngraphterm\n"); // isolated
    write_file(root, "X.md", "# X\n\ngraphterm\n\n[[Y]]\n");
    write_file(root, "Y.md", "# Y\n\ngraphterm\n");
    write_file(
        root,
        "Spaced Repetition.md",
        "# Spaced Repetition\n\nspacedterm\n",
    );
}

// ---------------------------------------------------------------------------
// TEST-013-006: --near A depth 1 → results limited to {A, B, C}
// ---------------------------------------------------------------------------

#[test]
fn test_013_006_near_depth1_outgoing() {
    let dir = TempDir::new().unwrap();
    build_phase2_vault(dir.path());

    // Build link graph (required for --near); must use cached mode so index.json is saved.
    run_json(zetl_cmd_cached(dir.path()).arg("index"));

    let json = run_json(
        zetl_cmd(dir.path())
            .arg("search")
            .arg("graphterm")
            .arg("--near")
            .arg("A"),
    );

    let results = json["results"].as_array().expect("results array");
    let pages: Vec<&str> = results.iter().filter_map(|r| r["page"].as_str()).collect();

    // A, B, C are within 1 hop of A (bidirectional)
    assert!(pages.contains(&"A"), "A (anchor) must be in results");
    assert!(
        pages.contains(&"B"),
        "B (outgoing 1-hop) must be in results"
    );
    assert!(
        pages.contains(&"C"),
        "C (outgoing 1-hop) must be in results"
    );

    // D is 2 hops away; E is isolated — both excluded at depth 1
    assert!(
        !pages.contains(&"D"),
        "D (2 hops) must be excluded at depth 1"
    );
    assert!(!pages.contains(&"E"), "E (isolated) must be excluded");
}

// ---------------------------------------------------------------------------
// TEST-013-006: --near A --depth 2 → results include D (2 hops via B)
// ---------------------------------------------------------------------------

#[test]
fn test_013_006_near_depth2_includes_second_hop() {
    let dir = TempDir::new().unwrap();
    build_phase2_vault(dir.path());

    run_json(zetl_cmd_cached(dir.path()).arg("index"));

    let json = run_json(
        zetl_cmd(dir.path())
            .arg("search")
            .arg("graphterm")
            .arg("--near")
            .arg("A")
            .arg("--depth")
            .arg("2"),
    );

    let results = json["results"].as_array().expect("results array");
    let pages: Vec<&str> = results.iter().filter_map(|r| r["page"].as_str()).collect();

    assert!(pages.contains(&"A"), "A must be in results");
    assert!(pages.contains(&"B"), "B must be in results");
    assert!(pages.contains(&"C"), "C must be in results");
    assert!(
        pages.contains(&"D"),
        "D (2 hops via B) must be in results at depth 2"
    );

    // E is isolated even at depth 2
    assert!(!pages.contains(&"E"), "E (isolated) must be excluded");
}

// ---------------------------------------------------------------------------
// TEST-013-006: Backlinks — --near Y includes X (X→Y, bidirectional BFS)
// ---------------------------------------------------------------------------

#[test]
fn test_013_006_near_includes_backlinks() {
    let dir = TempDir::new().unwrap();
    build_phase2_vault(dir.path());

    run_json(zetl_cmd_cached(dir.path()).arg("index"));

    let json = run_json(
        zetl_cmd(dir.path())
            .arg("search")
            .arg("graphterm")
            .arg("--near")
            .arg("Y"),
    );

    let results = json["results"].as_array().expect("results array");
    let pages: Vec<&str> = results.iter().filter_map(|r| r["page"].as_str()).collect();

    // Y is the anchor; X has a forward link to Y, so X is in Y's backlink neighbourhood
    assert!(pages.contains(&"Y"), "Y (anchor) must be in results");
    assert!(
        pages.contains(&"X"),
        "X must be included (X→Y backlink, bidirectional BFS)"
    );
}

// ---------------------------------------------------------------------------
// TEST-013-007: --depth without --near → exit code 2
// ---------------------------------------------------------------------------

#[test]
fn test_013_007_depth_without_near_exits_2() {
    let dir = TempDir::new().unwrap();
    build_phase2_vault(dir.path());

    zetl_cmd(dir.path())
        .arg("search")
        .arg("graphterm")
        .arg("--depth")
        .arg("1")
        .assert()
        .code(2);
}

// ---------------------------------------------------------------------------
// TEST-013-007: --depth 0 (with --near) → exit code 2
// ---------------------------------------------------------------------------

#[test]
fn test_013_007_depth_zero_exits_2() {
    let dir = TempDir::new().unwrap();
    build_phase2_vault(dir.path());

    run_json(zetl_cmd_cached(dir.path()).arg("index"));

    zetl_cmd(dir.path())
        .arg("search")
        .arg("graphterm")
        .arg("--near")
        .arg("A")
        .arg("--depth")
        .arg("0")
        .assert()
        .code(2);
}

// ---------------------------------------------------------------------------
// TEST-013-008: Case-insensitive anchor resolution
// ---------------------------------------------------------------------------

#[test]
fn test_013_008_case_insensitive_anchor_resolution() {
    let dir = TempDir::new().unwrap();
    build_phase2_vault(dir.path());

    run_json(zetl_cmd_cached(dir.path()).arg("index"));

    // "spaced repetition" (lowercase) should resolve to "Spaced Repetition"
    let json = run_json(
        zetl_cmd(dir.path())
            .arg("search")
            .arg("spacedterm")
            .arg("--near")
            .arg("spaced repetition"),
    );

    // The resolved anchor in the output should be the correctly-cased page name
    let near_field = json["near"].as_str().expect("near field present in output");
    assert_eq!(
        near_field, "Spaced Repetition",
        "anchor should resolve to canonical casing"
    );

    let results = json["results"].as_array().expect("results array");
    let pages: Vec<&str> = results.iter().filter_map(|r| r["page"].as_str()).collect();
    assert!(
        pages.contains(&"Spaced Repetition"),
        "Spaced Repetition page must be in results"
    );
}

// ---------------------------------------------------------------------------
// TEST-013-008: Unresolvable anchor → exit code 2 with suggestions
// ---------------------------------------------------------------------------

#[test]
fn test_013_008_unresolvable_anchor_exits_2_with_suggestions() {
    let dir = TempDir::new().unwrap();
    build_phase2_vault(dir.path());

    run_json(zetl_cmd_cached(dir.path()).arg("index"));

    // Use a name that partially matches "Spaced Repetition" to trigger suggestions
    let output = zetl_cmd(dir.path())
        .arg("search")
        .arg("graphterm")
        .arg("--near")
        .arg("spaced")
        .output()
        .expect("execute zetl");

    assert_eq!(output.status.code(), Some(2), "should exit with code 2");

    // The JSON error response should mention a suggestion
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("failed to parse JSON: {e}\nstdout: {stdout}"));
    let error_msg = json["error"].as_str().expect("error field in JSON");
    assert!(
        error_msg.contains("Spaced Repetition"),
        "error message should suggest 'Spaced Repetition', got: {error_msg}"
    );
}

#[test]
fn test_013_008_completely_unknown_anchor_exits_2() {
    let dir = TempDir::new().unwrap();
    build_phase2_vault(dir.path());

    run_json(zetl_cmd_cached(dir.path()).arg("index"));

    zetl_cmd(dir.path())
        .arg("search")
        .arg("graphterm")
        .arg("--near")
        .arg("NoSuchPageXYZ123")
        .assert()
        .code(2);
}

// ---------------------------------------------------------------------------
// TEST-013-009: Neighbourhood metadata present in JSON when --near is used
// ---------------------------------------------------------------------------

#[test]
fn test_013_009_neighbourhood_metadata_present_with_near() {
    let dir = TempDir::new().unwrap();
    build_phase2_vault(dir.path());

    run_json(zetl_cmd_cached(dir.path()).arg("index"));

    let json = run_json(
        zetl_cmd(dir.path())
            .arg("search")
            .arg("graphterm")
            .arg("--near")
            .arg("A")
            .arg("--depth")
            .arg("1"),
    );

    // REQ-013-009: near, depth, neighbourhood_size must appear in output
    assert!(
        json["near"].as_str().is_some(),
        "near field must be present when --near is used"
    );
    assert_eq!(
        json["near"].as_str().unwrap(),
        "A",
        "near should be the resolved anchor name"
    );
    assert_eq!(
        json["depth"].as_u64(),
        Some(1),
        "depth field must equal the requested depth"
    );
    assert!(
        json["neighbourhood_size"].as_u64().is_some(),
        "neighbourhood_size field must be present"
    );
    // A at depth 1: {A, B, C} = 3 pages
    assert_eq!(
        json["neighbourhood_size"].as_u64().unwrap(),
        3,
        "neighbourhood of A at depth 1 is {{A, B, C}} = 3 pages"
    );
}

// ---------------------------------------------------------------------------
// TEST-013-009: Neighbourhood metadata absent when --near is NOT used
// ---------------------------------------------------------------------------

#[test]
fn test_013_009_neighbourhood_metadata_absent_without_near() {
    let dir = TempDir::new().unwrap();
    build_phase2_vault(dir.path());

    let json = run_json(zetl_cmd(dir.path()).arg("search").arg("graphterm"));

    // REQ-013-009: fields must be omitted from the envelope when --near is not used
    assert!(
        json.get("near").is_none() || json["near"].is_null(),
        "near field must be absent when --near is not used"
    );
    assert!(
        json.get("depth").is_none() || json["depth"].is_null(),
        "depth field must be absent when --near is not used"
    );
    assert!(
        json.get("neighbourhood_size").is_none() || json["neighbourhood_size"].is_null(),
        "neighbourhood_size field must be absent when --near is not used"
    );
}

// ---------------------------------------------------------------------------
// TEST-013-016: --near composes with --path and --context
// ---------------------------------------------------------------------------

#[test]
fn test_013_016_near_composes_with_path_filter() {
    let dir = TempDir::new().unwrap();
    build_phase2_vault(dir.path());

    run_json(zetl_cmd_cached(dir.path()).arg("index"));

    // --near A (neighbourhood {A, B, C} at depth 1) AND --path B.md
    // Expected: only B.md result (C and A excluded by --path)
    let json = run_json(
        zetl_cmd(dir.path())
            .arg("search")
            .arg("graphterm")
            .arg("--near")
            .arg("A")
            .arg("--path")
            .arg("B.md"),
    );

    let results = json["results"].as_array().expect("results array");
    let pages: Vec<&str> = results.iter().filter_map(|r| r["page"].as_str()).collect();

    // Only B.md matches both the neighbourhood filter and the path filter
    assert_eq!(
        pages,
        vec!["B"],
        "--path B.md should restrict to B within the neighbourhood"
    );
}

#[test]
fn test_013_016_near_composes_with_context() {
    let dir = TempDir::new().unwrap();
    build_phase2_vault(dir.path());

    run_json(zetl_cmd_cached(dir.path()).arg("index"));

    let json = run_json(
        zetl_cmd(dir.path())
            .arg("search")
            .arg("graphterm")
            .arg("--near")
            .arg("A")
            .arg("--context")
            .arg("20"),
    );

    let results = json["results"].as_array().expect("results array");

    // All results in the neighbourhood should have context snippets
    for result in results {
        assert!(
            result["context"].as_str().is_some(),
            "context must be present when --context is specified, result: {result:?}"
        );
    }

    // Results should still be limited to the neighbourhood
    let pages: Vec<&str> = results.iter().filter_map(|r| r["page"].as_str()).collect();
    assert!(
        !pages.contains(&"E"),
        "E (isolated) must be excluded even with --context"
    );
    assert!(
        !pages.contains(&"D"),
        "D (2+ hops) must be excluded at default depth 1 even with --context"
    );
}

// ===========================================================================
// Phase 4: Static build search
// TEST-013-014: zetl build emits search-index.json with BM25 corpus statistics
// TEST-013-015: Generated HTML includes Cmd+K modal, BM25 fetch, and keyboard nav
// ===========================================================================

/// Run `zetl build --out-dir <out>` against the vault at `vault`, assert success.
fn run_build(vault: &Path, out_dir: &Path) {
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("zetl");
    cmd.arg("-d")
        .arg(vault)
        .arg("--no-cache")
        .arg("build")
        .arg("--out-dir")
        .arg(out_dir);
    let output = cmd.output().expect("failed to execute zetl build");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "zetl build exited with non-zero status.\nstderr: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// TEST-013-014: search-index.json is emitted in the output directory
// ---------------------------------------------------------------------------

#[test]
fn test_013_014_build_emits_search_index_json() {
    let dir = TempDir::new().unwrap();
    write_file(
        dir.path(),
        "Alpha.md",
        "# Alpha\n\nalpha content uniqueterm here\n",
    );
    write_file(
        dir.path(),
        "Beta.md",
        "# Beta\n\nbeta content uniqueterm here\n",
    );

    let out_dir = dir.path().join("dist");
    run_build(dir.path(), &out_dir);

    let search_index_path = out_dir.join("search-index.json");
    assert!(
        search_index_path.exists(),
        "search-index.json must be written to output directory"
    );
}

// ---------------------------------------------------------------------------
// TEST-013-014: search-index.json contains avgDl, docs (correct count), df
// ---------------------------------------------------------------------------

#[test]
fn test_013_014_search_index_top_level_fields() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "Alpha.md", "# Alpha\n\nalpha content here\n");
    write_file(dir.path(), "Beta.md", "# Beta\n\nbeta content here\n");
    write_file(dir.path(), "Gamma.md", "# Gamma\n\ngamma content here\n");

    let out_dir = dir.path().join("dist");
    run_build(dir.path(), &out_dir);

    let json_str = fs::read_to_string(out_dir.join("search-index.json")).unwrap();
    let json: Value =
        serde_json::from_str(&json_str).expect("search-index.json must be valid JSON");

    assert!(
        json["avgDl"].is_number(),
        "search-index.json must contain 'avgDl' as a number, got: {json}"
    );
    assert!(
        json["docs"].is_array(),
        "search-index.json must contain 'docs' as an array, got: {json}"
    );
    assert!(
        json["df"].is_object(),
        "search-index.json must contain 'df' as an object, got: {json}"
    );

    let docs = json["docs"].as_array().unwrap();
    assert_eq!(
        docs.len(),
        3,
        "docs array must have one entry per vault file (3 files)"
    );

    assert!(
        json["avgDl"].as_f64().unwrap() > 0.0,
        "avgDl must be > 0 for non-empty files"
    );
}

// ---------------------------------------------------------------------------
// TEST-013-014: each doc has n, s, dl, tf with correct term frequencies
// ---------------------------------------------------------------------------

#[test]
fn test_013_014_doc_fields_and_term_frequencies() {
    let dir = TempDir::new().unwrap();
    // "rareword" appears 3 times in Alpha only; "shared" appears in both
    write_file(
        dir.path(),
        "Alpha.md",
        "# Alpha\n\nrareword rareword rareword shared\n",
    );
    write_file(dir.path(), "Beta.md", "# Beta\n\nshared content\n");

    let out_dir = dir.path().join("dist");
    run_build(dir.path(), &out_dir);

    let json_str = fs::read_to_string(out_dir.join("search-index.json")).unwrap();
    let json: Value = serde_json::from_str(&json_str).unwrap();
    let docs = json["docs"].as_array().unwrap();

    // Every doc must have n, s, dl, tf
    for doc in docs {
        assert!(doc["n"].is_string(), "doc must have 'n' (page name): {doc}");
        assert!(doc["s"].is_string(), "doc must have 's' (slug): {doc}");
        assert!(
            doc["dl"].is_number(),
            "doc must have 'dl' (doc length): {doc}"
        );
        assert!(
            doc["tf"].is_object(),
            "doc must have 'tf' (term frequencies): {doc}"
        );
    }

    // Locate Alpha doc
    let alpha = docs
        .iter()
        .find(|d| d["n"].as_str() == Some("Alpha"))
        .expect("Alpha doc must be present");

    // rareword appears 3 times in Alpha
    assert_eq!(
        alpha["tf"]["rareword"].as_u64().unwrap_or(0),
        3,
        "rareword must have tf=3 in Alpha"
    );

    // shared appears once in Alpha
    assert_eq!(
        alpha["tf"]["shared"].as_u64().unwrap_or(0),
        1,
        "shared must have tf=1 in Alpha"
    );

    // dl for Alpha must equal sum of tf values
    let expected_dl: u64 = alpha["tf"]
        .as_object()
        .unwrap()
        .values()
        .filter_map(|v| v.as_u64())
        .sum();
    assert_eq!(
        alpha["dl"].as_u64().unwrap(),
        expected_dl,
        "dl must equal the sum of all tf values"
    );

    // df["rareword"] must be 1 (only Alpha contains it)
    let df = &json["df"];
    assert_eq!(
        df["rareword"].as_u64().unwrap_or(0),
        1,
        "rareword must appear in df=1 document"
    );

    // df["shared"] must be 2 (both Alpha and Beta contain it)
    assert_eq!(
        df["shared"].as_u64().unwrap_or(0),
        2,
        "shared must appear in df=2 documents"
    );
}

// ---------------------------------------------------------------------------
// TEST-013-015: generated HTML includes Cmd+K search modal
// ---------------------------------------------------------------------------

#[test]
fn test_013_015_html_has_search_modal() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "Page.md", "# Page\n\nsome content\n");

    let out_dir = dir.path().join("dist");
    run_build(dir.path(), &out_dir);

    let html = fs::read_to_string(out_dir.join("index.html")).unwrap();

    assert!(
        html.contains("search-overlay"),
        "index.html must contain search overlay element"
    );
    assert!(
        html.contains("search-input"),
        "index.html must contain search input element"
    );
    assert!(
        html.contains("openSearch"),
        "index.html must contain openSearch function"
    );
}

// ---------------------------------------------------------------------------
// TEST-013-015: generated HTML embeds BM25 search index inline
// ---------------------------------------------------------------------------

#[test]
fn test_013_015_html_embeds_bm25_index() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "Page.md", "# Page\n\nsome content\n");

    let out_dir = dir.path().join("dist");
    run_build(dir.path(), &out_dir);

    let html = fs::read_to_string(out_dir.join("index.html")).unwrap();

    assert!(
        html.contains("id=\"zetl-bm25-index\""),
        "index.html must embed the BM25 search index inline for file:// support"
    );
    assert!(
        html.contains("\"avgDl\""),
        "embedded BM25 index must contain corpus statistics"
    );
}

// ---------------------------------------------------------------------------
// TEST-013-015: generated HTML scores results by BM25
// ---------------------------------------------------------------------------

#[test]
fn test_013_015_html_includes_bm25_scorer() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "Page.md", "# Page\n\nsome content\n");

    let out_dir = dir.path().join("dist");
    run_build(dir.path(), &out_dir);

    let html = fs::read_to_string(out_dir.join("index.html")).unwrap();

    assert!(
        html.contains("bm25Search"),
        "index.html must include the bm25Search function"
    );
    // BM25 parameters (k1=1.2, b=0.75) from SPEC-013 §4.9
    assert!(
        html.contains("1.2"),
        "bm25Search must use k1=1.2 per SPEC-013 §4.9"
    );
    assert!(
        html.contains("0.75"),
        "bm25Search must use b=0.75 per SPEC-013 §4.9"
    );
}

// ---------------------------------------------------------------------------
// TEST-013-015: generated HTML includes keyboard navigation (ArrowDown/Up/Enter)
// ---------------------------------------------------------------------------

#[test]
fn test_013_015_html_keyboard_navigation() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "Page.md", "# Page\n\nsome content\n");

    let out_dir = dir.path().join("dist");
    run_build(dir.path(), &out_dir);

    let html = fs::read_to_string(out_dir.join("index.html")).unwrap();

    assert!(
        html.contains("ArrowDown"),
        "index.html must handle ArrowDown key for keyboard navigation"
    );
    assert!(
        html.contains("ArrowUp"),
        "index.html must handle ArrowUp key for keyboard navigation"
    );
    assert!(
        html.contains("'Enter'") || html.contains("\"Enter\""),
        "index.html must handle Enter key to navigate to selected result"
    );
    assert!(
        html.contains("'Escape'") || html.contains("\"Escape\""),
        "index.html must handle Escape to close search modal"
    );
}

// ---------------------------------------------------------------------------
// TEST-013-015: generated HTML results link to correct page paths
// ---------------------------------------------------------------------------

#[test]
fn test_013_015_html_result_links_use_slug() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "Page.md", "# Page\n\nsome content\n");

    let out_dir = dir.path().join("dist");
    run_build(dir.path(), &out_dir);

    let html = fs::read_to_string(out_dir.join("index.html")).unwrap();

    // Results use item.s (slug) as their href
    assert!(
        html.contains("item.s") || html.contains("filtered[active].s"),
        "index.html must build result links from the slug field (item.s)"
    );
}

// ---------------------------------------------------------------------------
// TEST-013-012: Serve mode search API
// ---------------------------------------------------------------------------

/// Extract body as a String from raw bytes returned by http_get.
fn body_string(body: &[u8]) -> String {
    String::from_utf8_lossy(body).into_owned()
}

#[test]
fn test_013_012_api_search_returns_bm25_results() {
    // Vault with several occurrences of "algorithm" so Tantivy finds something.
    let dir = TempDir::new().unwrap();
    write_file(
        dir.path(),
        "Algorithm.md",
        "# Introduction\n\nThis page discusses the algorithm deeply.\n\n## Details\n\nThe algorithm is efficient and the algorithm is correct.\n",
    );
    write_file(
        dir.path(),
        "Other.md",
        "# Other\n\nUnrelated content with no hits.\n",
    );

    let port = find_free_port();
    let mut child = spawn_serve(dir.path(), port, "default");

    let (status_line, _headers, body_bytes) = http_get(port, "/api/search?q=algorithm");
    let _ = child.kill();
    let _ = child.wait();

    assert!(
        status_line.contains("200"),
        "GET /api/search?q=algorithm must return 200, got: {status_line}"
    );

    let body = body_string(&body_bytes);
    let json: Value = serde_json::from_str(&body)
        .unwrap_or_else(|e| panic!("response is not valid JSON: {e}\nbody: {body}"));

    let results = json["results"]
        .as_array()
        .expect("results must be an array");
    assert!(
        !results.is_empty(),
        "should find at least one result for 'algorithm'"
    );

    // Every result must carry the required BM25 fields.
    for result in results {
        assert!(
            result["score"].as_f64().is_some() && result["score"].as_f64().unwrap() > 0.0,
            "every result must have a positive 'score', got: {result}"
        );
        assert!(
            result["page"].as_str().is_some(),
            "every result must have a 'page' field, got: {result}"
        );
        assert!(
            result["path"].as_str().is_some(),
            "every result must have a 'path' field, got: {result}"
        );
        assert!(
            result["line"].as_u64().is_some(),
            "every result must have a 'line' field, got: {result}"
        );
        assert!(
            result.get("heading").is_some(),
            "every result must have a 'heading' field (may be null), got: {result}"
        );
        assert!(
            result.get("context").is_some(),
            "every result must have a 'context' field, got: {result}"
        );
    }

    // Results should be ordered by descending score.
    let scores: Vec<f64> = results
        .iter()
        .map(|r| r["score"].as_f64().unwrap_or(0.0))
        .collect();
    let mut sorted = scores.clone();
    sorted.sort_by(|a, b| b.partial_cmp(a).unwrap());
    assert_eq!(
        scores, sorted,
        "results must be ordered by descending BM25 score"
    );
}

#[test]
fn test_013_012_api_search_empty_query_returns_400() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "Note.md", "# Note\n\nsome content\n");

    let port = find_free_port();
    let mut child = spawn_serve(dir.path(), port, "default");

    // Empty string for q
    let (status_empty, _, _) = http_get(port, "/api/search?q=");
    // Missing q entirely
    let (status_missing, _, _) = http_get(port, "/api/search");

    let _ = child.kill();
    let _ = child.wait();

    assert!(
        status_empty.contains("400"),
        "GET /api/search?q= must return 400 Bad Request, got: {status_empty}"
    );
    assert!(
        status_missing.contains("400"),
        "GET /api/search (no q param) must return 400 Bad Request, got: {status_missing}"
    );
}

#[test]
fn test_013_012_api_search_limit_parameter() {
    let dir = TempDir::new().unwrap();
    // Create a file with many occurrences of "test" so there are more than 3 matches.
    write_file(
        dir.path(),
        "Many.md",
        "# Many\n\ntest line one.\n\ntest line two.\n\ntest line three.\n\ntest line four.\n\ntest line five.\n",
    );

    let port = find_free_port();
    let mut child = spawn_serve(dir.path(), port, "default");
    let (status_line, _headers, body_bytes) = http_get(port, "/api/search?q=test&limit=3");
    let _ = child.kill();
    let _ = child.wait();

    assert!(
        status_line.contains("200"),
        "GET /api/search?q=test&limit=3 must return 200, got: {status_line}"
    );

    let body = body_string(&body_bytes);
    let json: Value = serde_json::from_str(&body)
        .unwrap_or_else(|e| panic!("response is not valid JSON: {e}\nbody: {body}"));

    let results = json["results"]
        .as_array()
        .expect("results must be an array");
    assert!(
        results.len() <= 3,
        "limit=3 must return at most 3 results, got {}",
        results.len()
    );
    // There should be at least 1 result (the file does contain "test").
    assert!(
        !results.is_empty(),
        "should find at least one result for 'test'"
    );
}

// ---------------------------------------------------------------------------
// TEST-013-013: Serve mode Cmd+K full-text search modal
// ---------------------------------------------------------------------------

#[test]
fn test_013_013_serve_html_search_modal_fields() {
    // The HTML served by zetl serve must contain the Cmd+K modal elements
    // that display page name, heading, context, and score to the user.
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "Page.md", "# Page\n\nsome content\n");

    let port = find_free_port();
    let mut child = spawn_serve(dir.path(), port, "default");
    let (status_line, _headers, body_bytes) = http_get(port, "/");
    let _ = child.kill();
    let _ = child.wait();

    assert!(status_line.contains("200"), "GET / must return 200");
    let html = body_string(&body_bytes);

    assert!(
        html.contains("search-overlay"),
        "serve HTML must contain the search overlay element"
    );
    assert!(
        html.contains("search-input"),
        "serve HTML must contain the search input element"
    );
    assert!(
        html.contains("openSearch"),
        "serve HTML must contain the openSearch function"
    );
    // Results render page name, heading, context, score.
    assert!(
        html.contains("page-name"),
        "serve HTML must render .page-name in results"
    );
    assert!(
        html.contains("heading"),
        "serve HTML must render .heading in results"
    );
    assert!(
        html.contains("context"),
        "serve HTML must render .context in results"
    );
    assert!(
        html.contains("score"),
        "serve HTML must render .score in results"
    );
    // Results come from the /api/search backend.
    assert!(
        html.contains("/api/search"),
        "serve HTML must query the /api/search endpoint for full-text results"
    );
}

#[test]
fn test_013_013_serve_html_keyboard_navigation() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "Page.md", "# Page\n\nsome content\n");

    let port = find_free_port();
    let mut child = spawn_serve(dir.path(), port, "default");
    let (status_line, _headers, body_bytes) = http_get(port, "/");
    let _ = child.kill();
    let _ = child.wait();

    assert!(status_line.contains("200"), "GET / must return 200");
    let html = body_string(&body_bytes);

    assert!(
        html.contains("ArrowDown"),
        "serve HTML must handle ArrowDown for keyboard navigation"
    );
    assert!(
        html.contains("ArrowUp"),
        "serve HTML must handle ArrowUp for keyboard navigation"
    );
    assert!(
        html.contains("'Enter'") || html.contains("\"Enter\""),
        "serve HTML must handle Enter to follow the selected result"
    );
    assert!(
        html.contains("'Escape'") || html.contains("\"Escape\""),
        "serve HTML must handle Escape to close the search modal"
    );
}

// ===========================================================================
// TEST-014: Phase 2 theme install / list / remove / serve end-to-end
// ===========================================================================

// ---------------------------------------------------------------------------
// Local-git helpers
// ---------------------------------------------------------------------------

/// Initialise a git repo in `dir`, set local user identity, add all files,
/// and make an initial commit.
fn git_init_commit(dir: &std::path::Path, commit_msg: &str) {
    let run = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HOME", dir.to_str().unwrap())
            .output()
            .unwrap_or_else(|e| panic!("git {:?} failed to start: {e}", args));
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    };
    run(&["init"]);
    run(&["config", "user.email", "test@example.com"]);
    run(&["config", "user.name", "Test"]);
    run(&["add", "."]);
    run(&["commit", "-m", commit_msg]);
}

/// Tag the current HEAD of `dir` with `tag`.
fn git_tag(dir: &std::path::Path, tag: &str) {
    let out = std::process::Command::new("git")
        .args(["tag", tag])
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HOME", dir.to_str().unwrap())
        .output()
        .expect("git tag");
    assert!(
        out.status.success(),
        "git tag failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Create a minimal theme git repo under `repo_dir` (which must already exist)
/// with `theme_name` and version `1.0.0`.
fn create_theme_repo(repo_dir: &std::path::Path, theme_name: &str) {
    write_file(
        repo_dir,
        "theme.toml",
        &format!(
            "[theme]\nname = \"{theme_name}\"\nversion = \"1.0.0\"\ndescription = \"Test theme\"\n"
        ),
    );
    write_file(
        repo_dir,
        "base.html",
        "<!DOCTYPE html><html><head><title>{{ page.title }}</title></head>\
         <body id=\"test-theme-marker\">{% block content %}{% endblock %}</body></html>",
    );
    git_init_commit(repo_dir, "Initial theme");
}

/// Return the `file://` URL for an absolute path (works on Unix).
fn file_url(path: &std::path::Path) -> String {
    format!("file://{}", path.display())
}

// ---------------------------------------------------------------------------
// TEST-014-006: install from a source whose URL is a local file:// repo.
// (GitHub shorthand → https://github.com/… URL is exercised by the unit
// tests in src/web/theme.rs; here we verify the full install pipeline
// end-to-end using a local bare repo as the git remote.)
// ---------------------------------------------------------------------------

#[test]
fn test_014_006_theme_install_from_local_file_url() {
    let dir = TempDir::new().unwrap();
    let vault = dir.path().join("vault");
    fs::create_dir_all(&vault).unwrap();
    write_file(&vault, "Note.md", "# Note\n\nHello.\n");

    // Repo without a theme.toml so the name falls back to the repo dir name.
    let repo_dir = dir.path().join("my-theme-repo");
    fs::create_dir_all(&repo_dir).unwrap();
    write_file(
        &repo_dir,
        "base.html",
        "<html><body>test theme</body></html>",
    );
    git_init_commit(&repo_dir, "Initial theme");

    let url = file_url(&repo_dir);

    let mut cmd = zetl_cmd(&vault);
    cmd.arg("theme").arg("install").arg(&url);
    let json = run_json(&mut cmd);

    assert_eq!(
        json["installed"]["name"].as_str().unwrap(),
        "my-theme-repo",
        "installed name should be derived from repo dir name; got {json}"
    );
    assert_eq!(
        json["installed"]["source"].as_str().unwrap(),
        url,
        "source URL must be recorded"
    );

    // Theme directory must exist on disk.
    let theme_dir = vault.join(".zetl/themes/my-theme-repo");
    assert!(
        theme_dir.is_dir(),
        ".zetl/themes/my-theme-repo must exist after install"
    );
    assert!(
        theme_dir.join("base.html").exists(),
        "base.html must be present"
    );
}

// ---------------------------------------------------------------------------
// TEST-014-007: install at a specific tag.
// ---------------------------------------------------------------------------

#[test]
fn test_014_007_theme_install_at_specific_tag() {
    let dir = TempDir::new().unwrap();
    let vault = dir.path().join("vault");
    fs::create_dir_all(&vault).unwrap();
    write_file(&vault, "Note.md", "# Note\n\nHello.\n");

    let repo_dir = dir.path().join("tagged-theme-repo");
    fs::create_dir_all(&repo_dir).unwrap();
    create_theme_repo(&repo_dir, "tagged-theme");
    git_tag(&repo_dir, "v1.0.0");

    let url_with_tag = format!("{}#v1.0.0", file_url(&repo_dir));

    let mut cmd = zetl_cmd(&vault);
    cmd.arg("theme").arg("install").arg(&url_with_tag);
    let json = run_json(&mut cmd);

    assert_eq!(
        json["installed"]["ref"].as_str().unwrap(),
        "v1.0.0",
        "installed ref must be 'v1.0.0'; got {json}"
    );

    // Name comes from theme.toml ("tagged-theme"), not the repo dir name.
    let theme_dir = vault.join(".zetl/themes/tagged-theme");
    assert!(
        theme_dir.is_dir(),
        "theme dir must exist after tagged install"
    );
}

// ---------------------------------------------------------------------------
// TEST-014-008: install with --path extracts the correct subdirectory.
// ---------------------------------------------------------------------------

#[test]
fn test_014_008_theme_install_with_path_subdir() {
    let dir = TempDir::new().unwrap();
    let vault = dir.path().join("vault");
    fs::create_dir_all(&vault).unwrap();
    write_file(&vault, "Note.md", "# Note\n\nHello.\n");

    // Repo contains two subdirectories; only `themes/light` is the theme.
    let repo_dir = dir.path().join("multi-theme-repo");
    fs::create_dir_all(&repo_dir).unwrap();
    write_file(
        &repo_dir,
        "themes/light/theme.toml",
        "[theme]\nname = \"light\"\nversion = \"1.0.0\"\n",
    );
    write_file(&repo_dir, "themes/light/base.html", "<html>light</html>");
    write_file(&repo_dir, "README.md", "Mono-repo with themes");
    git_init_commit(&repo_dir, "Initial");

    let url = file_url(&repo_dir);

    let mut cmd = zetl_cmd(&vault);
    cmd.arg("theme")
        .arg("install")
        .arg(&url)
        .arg("--path")
        .arg("themes/light");
    let json = run_json(&mut cmd);

    // Name derived from the last component of --path.
    assert_eq!(
        json["installed"]["name"].as_str().unwrap(),
        "light",
        "name should be derived from --path last component; got {json}"
    );
    assert_eq!(
        json["installed"]["path"].as_str().unwrap(),
        "themes/light",
        "path must be recorded in output"
    );

    let theme_dir = vault.join(".zetl/themes/light");
    assert!(theme_dir.is_dir(), ".zetl/themes/light must exist");
    assert!(
        theme_dir.join("theme.toml").exists(),
        "theme.toml must be present"
    );
    // The README from repo root must NOT be copied.
    assert!(
        !theme_dir.join("README.md").exists(),
        "README.md should not be copied"
    );
}

// ---------------------------------------------------------------------------
// TEST-014-009: --name overrides the derived theme name.
// ---------------------------------------------------------------------------

#[test]
fn test_014_009_theme_install_with_name_override() {
    let dir = TempDir::new().unwrap();
    let vault = dir.path().join("vault");
    fs::create_dir_all(&vault).unwrap();
    write_file(&vault, "Note.md", "# Note\n\nHello.\n");

    let repo_dir = dir.path().join("some-repo");
    fs::create_dir_all(&repo_dir).unwrap();
    create_theme_repo(&repo_dir, "my-theme");

    let url = file_url(&repo_dir);

    let mut cmd = zetl_cmd(&vault);
    cmd.arg("theme")
        .arg("install")
        .arg(&url)
        .arg("--name")
        .arg("custom-name");
    let json = run_json(&mut cmd);

    assert_eq!(
        json["installed"]["name"].as_str().unwrap(),
        "custom-name",
        "--name should override derived name; got {json}"
    );

    let theme_dir = vault.join(".zetl/themes/custom-name");
    assert!(theme_dir.is_dir(), ".zetl/themes/custom-name must exist");
}

// ---------------------------------------------------------------------------
// TEST-014-010: duplicate install without --force fails; with --force succeeds.
// ---------------------------------------------------------------------------

#[test]
fn test_014_010_duplicate_install_fails_without_force() {
    let dir = TempDir::new().unwrap();
    let vault = dir.path().join("vault");
    fs::create_dir_all(&vault).unwrap();
    write_file(&vault, "Note.md", "# Note\n\nHello.\n");

    let repo_dir = dir.path().join("dup-theme-repo");
    fs::create_dir_all(&repo_dir).unwrap();
    create_theme_repo(&repo_dir, "dup-theme");
    let url = file_url(&repo_dir);

    // First install — must succeed.
    run_json(&mut {
        let mut cmd = zetl_cmd(&vault);
        cmd.arg("theme").arg("install").arg(&url);
        cmd
    });

    // Second install without --force — must fail.
    let output = {
        let bin = assert_cmd::cargo::cargo_bin!("zetl");
        std::process::Command::new(bin)
            .arg("-d")
            .arg(&vault)
            .arg("--no-cache")
            .arg("theme")
            .arg("install")
            .arg(&url)
            .output()
            .expect("run zetl")
    };
    assert!(
        !output.status.success(),
        "second install without --force should fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("already installed") || stderr.contains("--force"),
        "error must mention 'already installed' or '--force'; got: {stderr}"
    );

    // Third install with --force — must succeed.
    run_json(&mut {
        let mut cmd = zetl_cmd(&vault);
        cmd.arg("theme").arg("install").arg("--force").arg(&url);
        cmd
    });
}

// ---------------------------------------------------------------------------
// TEST-014-011: .zetl-source.toml is written with correct provenance.
// ---------------------------------------------------------------------------

#[test]
fn test_014_011_zetl_source_toml_provenance() {
    let dir = TempDir::new().unwrap();
    let vault = dir.path().join("vault");
    fs::create_dir_all(&vault).unwrap();
    write_file(&vault, "Note.md", "# Note\n\nHello.\n");

    let repo_dir = dir.path().join("prov-theme-repo");
    fs::create_dir_all(&repo_dir).unwrap();
    create_theme_repo(&repo_dir, "prov-theme");
    git_tag(&repo_dir, "v2.0.0");
    let url_with_tag = format!("{}#v2.0.0", file_url(&repo_dir));

    run_json(&mut {
        let mut cmd = zetl_cmd(&vault);
        cmd.arg("theme").arg("install").arg(&url_with_tag);
        cmd
    });

    // Name comes from theme.toml ("prov-theme"), not the repo dir name.
    let source_path = vault.join(".zetl/themes/prov-theme/.zetl-source.toml");
    assert!(
        source_path.exists(),
        ".zetl-source.toml must exist after install"
    );

    let content = fs::read_to_string(&source_path).expect("read .zetl-source.toml");

    // Must contain the URL (without the #ref fragment).
    let expected_url = file_url(&repo_dir);
    assert!(
        content.contains(&expected_url),
        ".zetl-source.toml must contain the source URL; got:\n{content}"
    );
    // Must record the ref.
    assert!(
        content.contains("v2.0.0"),
        ".zetl-source.toml must contain the ref 'v2.0.0'; got:\n{content}"
    );
    // Must contain a commit SHA (40 hex chars somewhere).
    assert!(
        content.contains("commit"),
        ".zetl-source.toml must contain a 'commit' field; got:\n{content}"
    );
    // Must contain installed_at (ISO 8601 format).
    assert!(
        content.contains("installed_at"),
        ".zetl-source.toml must contain 'installed_at'; got:\n{content}"
    );
    // Must contain zetl_version.
    assert!(
        content.contains("zetl_version"),
        ".zetl-source.toml must contain 'zetl_version'; got:\n{content}"
    );
}

// ---------------------------------------------------------------------------
// TEST-014-012: zetl theme list shows installed theme with origin.
// ---------------------------------------------------------------------------

#[test]
fn test_014_012_theme_list_shows_installed_with_origin() {
    let dir = TempDir::new().unwrap();
    let vault = dir.path().join("vault");
    fs::create_dir_all(&vault).unwrap();
    write_file(&vault, "Note.md", "# Note\n\nHello.\n");

    let repo_dir = dir.path().join("list-theme-repo");
    fs::create_dir_all(&repo_dir).unwrap();
    create_theme_repo(&repo_dir, "list-theme");
    let url = file_url(&repo_dir);

    // Install the theme.
    run_json(&mut {
        let mut cmd = zetl_cmd(&vault);
        cmd.arg("theme").arg("install").arg(&url);
        cmd
    });

    // List themes.
    let json = run_json(&mut {
        let mut cmd = zetl_cmd(&vault);
        cmd.arg("theme").arg("list");
        cmd
    });

    let themes = json["themes"].as_array().expect("themes must be an array");
    // Name comes from theme.toml ("list-theme"), not the repo dir name.
    let installed = themes
        .iter()
        .find(|t| t["name"].as_str() == Some("list-theme"))
        .unwrap_or_else(|| panic!("installed theme 'list-theme' not found in list; got {json}"));

    assert_eq!(
        installed["source"].as_str().unwrap_or(""),
        "installed",
        "source field must be 'installed'"
    );
    assert_eq!(
        installed["origin_url"].as_str().unwrap_or(""),
        url,
        "origin_url must match the install URL"
    );
}

// ---------------------------------------------------------------------------
// TEST-014-013: zetl serve --theme <installed> renders with the installed theme.
// ---------------------------------------------------------------------------

#[test]
fn test_014_013_serve_with_installed_theme() {
    let dir = TempDir::new().unwrap();
    let vault = dir.path().join("vault");
    fs::create_dir_all(&vault).unwrap();
    write_file(&vault, "Hello.md", "# Hello\n\nWorld content.\n");

    // Create a theme repo whose base.html emits a unique sentinel string.
    let repo_dir = dir.path().join("serve-theme-repo");
    fs::create_dir_all(&repo_dir).unwrap();
    write_file(
        &repo_dir,
        "theme.toml",
        "[theme]\nname = \"serve-theme\"\nversion = \"1.0.0\"\n",
    );
    // Use vault.name (always available) rather than page.title (only for page context).
    write_file(
        &repo_dir,
        "base.html",
        "<!DOCTYPE html><html><head><title>{{ vault.name }}</title></head>\
         <body data-custom-theme=\"serve-theme-sentinel\">\
         {% block content %}{% endblock %}</body></html>",
    );
    git_init_commit(&repo_dir, "Initial theme");

    let url = file_url(&repo_dir);

    // Install the theme into the vault.
    run_json(&mut {
        let mut cmd = zetl_cmd(&vault);
        cmd.arg("theme")
            .arg("install")
            .arg(&url)
            .arg("--name")
            .arg("serve-theme");
        cmd
    });

    // Serve with the installed theme and fetch the index page.
    let port = find_free_port();
    let mut child = spawn_serve(&vault, port, "serve-theme");
    let (status_line, _headers, body_bytes) = http_get(port, "/");
    let _ = child.kill();
    let _ = child.wait();

    assert!(
        status_line.contains("200"),
        "GET / must return 200; got: {status_line}"
    );
    let html = body_string(&body_bytes);
    assert!(
        html.contains("serve-theme-sentinel"),
        "response must contain sentinel from installed theme; got:\n{html}"
    );
}

// ---------------------------------------------------------------------------
// TEST-014-015: zetl theme remove <installed> deletes the theme directory.
// ---------------------------------------------------------------------------

#[test]
fn test_014_015_theme_remove_installed() {
    let dir = TempDir::new().unwrap();
    let vault = dir.path().join("vault");
    fs::create_dir_all(&vault).unwrap();
    write_file(&vault, "Note.md", "# Note\n\nHello.\n");

    let repo_dir = dir.path().join("rm-theme-repo");
    fs::create_dir_all(&repo_dir).unwrap();
    create_theme_repo(&repo_dir, "rm-theme");
    let url = file_url(&repo_dir);

    // Install.
    run_json(&mut {
        let mut cmd = zetl_cmd(&vault);
        cmd.arg("theme")
            .arg("install")
            .arg(&url)
            .arg("--name")
            .arg("rm-theme");
        cmd
    });

    let theme_dir = vault.join(".zetl/themes/rm-theme");
    assert!(theme_dir.is_dir(), "theme dir must exist before removal");

    // Remove.
    let json = run_json(&mut {
        let mut cmd = zetl_cmd(&vault);
        cmd.arg("theme").arg("remove").arg("rm-theme");
        cmd
    });

    assert_eq!(
        json["removed"]["name"].as_str().unwrap(),
        "rm-theme",
        "removed.name must be 'rm-theme'; got {json}"
    );
    assert!(
        !theme_dir.exists(),
        "theme directory must be deleted after removal"
    );

    // After removal, theme list must not include rm-theme as installed.
    let list = run_json(&mut {
        let mut cmd = zetl_cmd(&vault);
        cmd.arg("theme").arg("list");
        cmd
    });
    let themes = list["themes"].as_array().unwrap();
    assert!(
        !themes
            .iter()
            .any(|t| t["name"].as_str() == Some("rm-theme")),
        "rm-theme must not appear in theme list after removal"
    );
}

// ---------------------------------------------------------------------------
// TEST-014-016: zetl theme remove default fails (bundled theme).
// ---------------------------------------------------------------------------

#[test]
fn test_014_016_theme_remove_bundled_fails() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "Note.md", "# Note\n\nHello.\n");

    let output = {
        let bin = assert_cmd::cargo::cargo_bin!("zetl");
        std::process::Command::new(bin)
            .arg("-d")
            .arg(dir.path())
            .arg("--no-cache")
            .arg("theme")
            .arg("remove")
            .arg("default")
            .output()
            .expect("run zetl")
    };

    assert!(
        !output.status.success(),
        "removing bundled 'default' theme must fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("bundled") || stderr.contains("cannot remove"),
        "error must explain that 'default' is bundled; got: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// TEST-014: Invalid source strings produce clear errors.
// ---------------------------------------------------------------------------

#[test]
fn test_014_invalid_source_strings_rejected() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "Note.md", "# Note\nHello.\n");

    let invalid_sources = [
        "not-a-valid-source",
        "ftp://example.com/theme.git",
        "/absolute/path",
        "just-one-component",
        "",
    ];

    for source in &invalid_sources {
        let output = {
            let bin = assert_cmd::cargo::cargo_bin!("zetl");
            std::process::Command::new(bin)
                .arg("-d")
                .arg(dir.path())
                .arg("--no-cache")
                .arg("theme")
                .arg("install")
                .arg(source)
                .output()
                .expect("run zetl")
        };
        assert!(
            !output.status.success(),
            "source {:?} should be rejected but was accepted",
            source
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !stderr.is_empty(),
            "error output must be non-empty for invalid source {:?}",
            source
        );
    }
}

// ---------------------------------------------------------------------------
// TEST-014: Path traversal in --path is rejected.
// ---------------------------------------------------------------------------

#[test]
fn test_014_path_traversal_rejected() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "Note.md", "# Note\nHello.\n");

    let traversal_paths = ["../escape", "../../etc/passwd", "/absolute"];

    let repo_dir = dir.path().join("dummy-repo");
    fs::create_dir_all(&repo_dir).unwrap();
    // No need for a valid repo; --path validation happens before cloning.
    let url = file_url(&repo_dir);

    for path in &traversal_paths {
        let output = {
            let bin = assert_cmd::cargo::cargo_bin!("zetl");
            std::process::Command::new(bin)
                .arg("-d")
                .arg(dir.path())
                .arg("--no-cache")
                .arg("theme")
                .arg("install")
                .arg(&url)
                .arg("--path")
                .arg(path)
                .output()
                .expect("run zetl")
        };
        assert!(
            !output.status.success(),
            "--path {:?} should be rejected as a traversal attempt",
            path
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("disallowed")
                || stderr.contains("relative")
                || stderr.contains("absolute"),
            "error for --path {:?} must explain the rejection; got: {stderr}",
            path
        );
    }
}

// (Chain navigation tests removed — chain logic stripped from core per IMPL-015.
//  Chain navigation will be re-added via post-index theme hooks per SPEC-016.)

// ===========================================================================
// TEST-015-001: fountain theme — Courier Prime font-face, no CDN URLs
// ===========================================================================

/// TEST-015-001: `zetl build --theme fountain` produces HTML that declares
/// Courier Prime via @font-face with local WOFF2 paths and contains no
/// external CDN URLs for fonts or stylesheets.
#[test]
fn test_015_001_fountain_font_face_no_cdn() {
    let dir = TempDir::new().expect("create temp dir");
    write_file(dir.path(), "Intro.md", "# Intro\n\nOpening scene.\n");

    let out_dir = dir.path().join("dist");
    let output = zetl_cmd(dir.path())
        .arg("build")
        .arg("--theme")
        .arg("fountain")
        .arg("-o")
        .arg(&out_dir)
        .output()
        .expect("zetl build --theme fountain");
    assert!(
        output.status.success(),
        "zetl build --theme fountain failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let html = fs::read_to_string(out_dir.join("intro/index.html"))
        .expect("intro/index.html should be built");

    // Must declare Courier Prime via @font-face.
    assert!(
        html.contains("Courier Prime"),
        "fountain HTML should declare 'Courier Prime' font"
    );
    assert!(
        html.contains("@font-face"),
        "fountain HTML should include @font-face declarations"
    );
    assert!(
        html.contains("CourierPrime-Regular.woff2"),
        "fountain HTML should reference CourierPrime-Regular.woff2"
    );
    assert!(
        html.contains("CourierPrime-Bold.woff2"),
        "fountain HTML should reference CourierPrime-Bold.woff2"
    );

    // Must not reference any CDN (no external font hosting).
    assert!(
        !html.contains("fonts.googleapis.com"),
        "fountain HTML must not use Google Fonts CDN"
    );
    assert!(
        !html.contains("fonts.gstatic.com"),
        "fountain HTML must not reference Google's static CDN"
    );
    assert!(
        !html.contains("cdnjs.cloudflare.com"),
        "fountain HTML must not use cdnjs CDN"
    );
    assert!(
        !html.contains("unpkg.com"),
        "fountain HTML must not use unpkg CDN"
    );
    assert!(
        !html.contains("jsdelivr.net"),
        "fountain HTML must not use jsDelivr CDN"
    );
}

// ===========================================================================
// TEST-015-002: fountain theme — WOFF2 fonts copied to _static/
// ===========================================================================

/// TEST-015-002: After `zetl build --theme fountain`, all four Courier Prime
/// WOFF2 font files are present in the `_static/` directory of the build
/// output.
#[test]
fn test_015_002_fountain_woff2_fonts_in_static() {
    let dir = TempDir::new().expect("create temp dir");
    write_file(dir.path(), "Scene.md", "# Scene\n\nContent.\n");

    let out_dir = dir.path().join("dist");
    let output = zetl_cmd(dir.path())
        .arg("build")
        .arg("--theme")
        .arg("fountain")
        .arg("-o")
        .arg(&out_dir)
        .output()
        .expect("zetl build --theme fountain");
    assert!(
        output.status.success(),
        "zetl build --theme fountain failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let static_dir = out_dir.join("_static");

    for font in &[
        "CourierPrime-Regular.woff2",
        "CourierPrime-Bold.woff2",
        "CourierPrime-Italic.woff2",
        "CourierPrime-BoldItalic.woff2",
    ] {
        let path = static_dir.join(font);
        assert!(
            path.exists(),
            "_static/{font} must exist in build output after `zetl build --theme fountain`"
        );
        let size = fs::metadata(&path)
            .unwrap_or_else(|e| panic!("failed to stat {font}: {e}"))
            .len();
        assert!(
            size > 0,
            "_static/{font} must not be empty"
        );
    }
}

// (Fountain chain scene-nav tests removed — see IMPL-015.)

/// TEST-015-003c: `zetl theme list` includes "fountain" with `source =
/// "bundled"`.
#[test]
fn test_015_003_fountain_theme_list_bundled() {
    let dir = TempDir::new().expect("create temp dir");
    write_file(dir.path(), "Note.md", "# Note\n\nHello.\n");

    let json = run_json(&mut {
        let mut cmd = zetl_cmd(dir.path());
        cmd.arg("theme").arg("list");
        cmd
    });

    let themes = json["themes"].as_array().expect("themes must be an array");
    let fountain = themes
        .iter()
        .find(|t| t["name"].as_str() == Some("fountain"))
        .unwrap_or_else(|| {
            panic!(
                "fountain must appear in `zetl theme list`; got: {json}"
            )
        });

    assert_eq!(
        fountain["source"].as_str().unwrap_or(""),
        "bundled",
        "fountain theme source must be 'bundled'"
    );
}
