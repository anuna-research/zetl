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
    let mut cmd = Command::cargo_bin("zetl").expect("binary should be built");
    cmd.arg("-d").arg(vault.as_os_str());
    cmd.arg("--no-cache");
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
    let output = cmd
        .output()
        .expect("failed to execute zetl");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "zetl exited with non-zero status.\nstdout: {}\nstderr: {}",
        stdout,
        stderr,
    );
    serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("failed to parse JSON output: {}\nraw stdout: {}", e, stdout))
}

/// Run the command (may fail) and parse stdout as JSON regardless of exit code.
fn run_json_any(cmd: &mut Command) -> (Value, std::process::ExitStatus) {
    let output = cmd
        .output()
        .expect("failed to execute zetl");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("failed to parse JSON output: {}\nraw stdout: {}", e, stdout));
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
    write_file(
        root,
        "Note 1.md",
        "# Note 1\n\nRelated to [[Concept X]].\n",
    );
    write_file(
        root,
        "Note 2.md",
        "# Note 2\n\nAlso about [[Concept X]] and [[Note 1]].\n",
    );
    write_file(
        root,
        "Note 3.md",
        "# Note 3\n\nSee [[Concept X]].\n",
    );
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
        "# Zettelkasten Method\n\nContent.\n",
    );
    write_file(
        root,
        "Zettelkasten History.md",
        "# Zettelkasten History\n\nContent.\n",
    );
    write_file(
        root,
        "Rust Programming.md",
        "# Rust Programming\n\nContent.\n",
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
        "expected 5 files scanned, got: {}",
        json
    );

    // Verify links_found = 12 (only body text links, excluding code/comments)
    assert_eq!(
        json["links_found"].as_u64(),
        Some(12),
        "expected 12 body-text links found, got: {}",
        json
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
        "A should have 2 forward links, got: {:?}",
        links_a
    );

    // Check forward links from B: should link to C
    let json_b = run_json(zetl_cmd(dir.path()).arg("links").arg("B"));
    let links_b = json_b["links"].as_array().expect("links should be array");
    assert_eq!(
        links_b.len(),
        1,
        "B should have 1 forward link, got: {:?}",
        links_b
    );

    // Check forward links from D: should have 0 links
    let json_d = run_json(zetl_cmd(dir.path()).arg("links").arg("D"));
    let links_d = json_d["links"].as_array().expect("links should be array");
    assert_eq!(
        links_d.len(),
        0,
        "D should have 0 forward links, got: {:?}",
        links_d
    );

    // Check backlinks for C: should be linked from A and B
    let json_c_bl = run_json(zetl_cmd(dir.path()).arg("backlinks").arg("C"));
    let backlinks_c = json_c_bl["backlinks"]
        .as_array()
        .expect("backlinks should be array");
    assert_eq!(
        backlinks_c.len(),
        2,
        "C should have 2 backlinks, got: {:?}",
        backlinks_c
    );
    let sources: Vec<&str> = backlinks_c
        .iter()
        .filter_map(|bl| bl["source"].as_str())
        .collect();
    assert!(
        sources.contains(&"A"),
        "C should have backlink from A, sources: {:?}",
        sources
    );
    assert!(
        sources.contains(&"B"),
        "C should have backlink from B, sources: {:?}",
        sources
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
            l["target"].as_str() == Some("Page A")
                || l["target_page"].as_str() == Some("Page A")
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
            l["target"].as_str() == Some("Page B")
                || l["target_page"].as_str() == Some("Page B")
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

    let json = run_json(
        zetl_cmd(dir.path())
            .arg("backlinks")
            .arg("Concept X"),
    );

    // Verify backlinks count
    let backlinks = json["backlinks"]
        .as_array()
        .expect("backlinks should be array");
    assert_eq!(
        backlinks.len(),
        3,
        "expected 3 backlinks to 'Concept X', got: {:?}",
        backlinks
    );

    // Verify source files are present
    let sources: Vec<&str> = backlinks
        .iter()
        .filter_map(|bl| bl["source"].as_str())
        .collect();
    assert!(
        sources.contains(&"Note 1"),
        "should have backlink from Note 1, sources: {:?}",
        sources
    );
    assert!(
        sources.contains(&"Note 2"),
        "should have backlink from Note 2, sources: {:?}",
        sources
    );
    assert!(
        sources.contains(&"Note 3"),
        "should have backlink from Note 3, sources: {:?}",
        sources
    );

    // Each backlink should have a line number
    for bl in backlinks {
        assert!(
            bl.get("line").is_some() && bl["line"].as_u64().is_some(),
            "each backlink should have a line number, got: {:?}",
            bl
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

    let (json, status) = run_json_any(
        zetl_cmd(dir.path())
            .arg("check")
            .arg("--dead-links"),
    );

    // Should report dead links
    let dead_links = json["dead_links"]
        .as_array()
        .expect("dead_links should be array");

    // There should be exactly 1 dead link: Ghost Page
    assert_eq!(
        dead_links.len(),
        1,
        "expected 1 dead link, got: {:?}",
        dead_links
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

    let (json, _status) = run_json_any(
        zetl_cmd(dir.path())
            .arg("check")
            .arg("--orphans"),
    );

    let orphans = json["orphans"]
        .as_array()
        .expect("orphans should be array");

    // Extract orphan page names
    let orphan_pages: Vec<&str> = orphans
        .iter()
        .filter_map(|o| o["page"].as_str())
        .collect();

    // D should be an orphan (never linked to)
    assert!(
        orphan_pages.contains(&"D"),
        "D should be detected as an orphan, orphans: {:?}",
        orphan_pages
    );

    // A is also an orphan (nobody links to A in this vault)
    assert!(
        orphan_pages.contains(&"A"),
        "A should be detected as an orphan (no incoming links), orphans: {:?}",
        orphan_pages
    );

    // B should NOT be an orphan (A links to B)
    assert!(
        !orphan_pages.contains(&"B"),
        "B should NOT be an orphan, orphans: {:?}",
        orphan_pages
    );

    // C should NOT be an orphan (B links to C)
    assert!(
        !orphan_pages.contains(&"C"),
        "C should NOT be an orphan, orphans: {:?}",
        orphan_pages
    );
}

// ===========================================================================
// TEST-007: Syntax Validation
// ===========================================================================

#[test]
fn test_007_syntax_validation() {
    let dir = TempDir::new().expect("create temp dir");
    build_test007_vault(dir.path());

    let (json, _status) = run_json_any(
        zetl_cmd(dir.path())
            .arg("check")
            .arg("--syntax"),
    );

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
    let has_unclosed = syntax_errors
        .iter()
        .any(|e| {
            e["message"]
                .as_str()
                .map(|m| m.to_lowercase().contains("unclosed"))
                .unwrap_or(false)
        });
    assert!(has_unclosed, "should detect unclosed wikilink syntax");

    // Check that we have an empty wikilink diagnostic
    let has_empty = syntax_errors
        .iter()
        .any(|e| {
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
            "syntax error should have file, got: {:?}",
            err
        );
        assert!(
            err.get("line").is_some(),
            "syntax error should have line, got: {:?}",
            err
        );
        assert!(
            err.get("message").is_some(),
            "syntax error should have message, got: {:?}",
            err
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

    let results = json["results"]
        .as_array()
        .expect("results should be array");

    let page_names: Vec<&str> = results
        .iter()
        .filter_map(|r| r["page"].as_str())
        .collect();

    // Similar Zettelkasten pages should appear
    assert!(
        page_names
            .iter()
            .any(|name| name.to_lowercase().contains("zettelkasten")),
        "should find at least one Zettelkasten page, got: {:?}",
        page_names
    );

    // Each result should have a distance field
    for result in results {
        assert!(
            result.get("distance").is_some(),
            "each result should have a distance field, got: {:?}",
            result
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
            "results should be sorted by distance ascending, got: {:?}",
            distances
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
        "expected 4 pages, got: {}",
        json
    );

    // 3 links: A->B, A->C, B->C
    assert_eq!(
        json["links"].as_u64(),
        Some(3),
        "expected 3 links, got: {}",
        json
    );

    // Dead links: 0 (all targets exist)
    assert_eq!(
        json["dead_links"].as_u64(),
        Some(0),
        "expected 0 dead links, got: {}",
        json
    );

    // Orphans: A and D have no incoming links
    assert_eq!(
        json["orphans"].as_u64(),
        Some(2),
        "expected 2 orphans (A and D), got: {}",
        json
    );

    // Connected components: 2 (the {A,B,C} cluster and {D} isolated)
    // petgraph connected_components treats graph as undirected, so {A,B,C} is one component
    // and {D} is another.
    assert_eq!(
        json["connected_components"].as_u64(),
        Some(2),
        "expected 2 connected components, got: {}",
        json
    );

    // most_linked should be present and sorted descending by backlink_count
    let most_linked = json["most_linked"]
        .as_array()
        .expect("most_linked should be array");
    assert!(
        !most_linked.is_empty(),
        "most_linked should not be empty"
    );

    // C should be the most linked (2 incoming: from A and B)
    assert_eq!(
        most_linked[0]["page"].as_str(),
        Some("C"),
        "most linked page should be C, got: {:?}",
        most_linked
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
            "most_linked should be sorted descending, got: {:?}",
            backlink_counts
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

    let json = run_json(
        zetl_cmd(dir.path()).arg("path").arg("A").arg("D"),
    );

    // Verify path: A -> B -> C -> D
    assert_eq!(json["from"].as_str(), Some("A"));
    assert_eq!(json["to"].as_str(), Some("D"));
    assert_eq!(
        json["hops"].as_u64(),
        Some(3),
        "expected 3 hops, got: {}",
        json
    );

    let path = json["path"]
        .as_array()
        .expect("path should be array");
    let path_names: Vec<&str> = path
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
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
        "expected 2 files scanned (drafts/ should be ignored), got: {}",
        json
    );

    // Verify with stats that only 2 pages exist
    let stats = run_json(zetl_cmd(dir.path()).arg("stats"));
    assert_eq!(
        stats["pages"].as_u64(),
        Some(2),
        "expected 2 pages in stats (drafts excluded), got: {}",
        stats
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
    write_file(
        dir.path(),
        ".zetl/index.md",
        "# Index\n\n[[Link]].\n",
    );
    // This one should be scanned
    write_file(dir.path(), "Real Note.md", "# Real Note\n\nContent.\n");

    let json = run_json(zetl_cmd(dir.path()).arg("index"));

    assert_eq!(
        json["files_scanned"].as_u64(),
        Some(1),
        "only Real Note.md should be scanned (default ignores for .git, node_modules, .zetl), got: {}",
        json
    );
}

#[test]
fn test_version_flag() {
    let mut cmd = Command::cargo_bin("zetl").expect("binary should be built");
    cmd.arg("--version");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("zetl"));
}

#[test]
fn test_help_flag() {
    let mut cmd = Command::cargo_bin("zetl").expect("binary should be built");
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
    let json = run_json(
        zetl_cmd(dir.path()).arg("links").arg("my page"),
    );

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
    write_file(
        dir.path(),
        "Orphan.md",
        "# Orphan\n\nNobody links here.\n",
    );
    write_file(
        dir.path(),
        "Broken.md",
        "# Broken\n\nUnclosed [[link\n",
    );

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
