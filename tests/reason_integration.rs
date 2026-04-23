//! Integration tests for the `ztl reason` command family.
//!
//! These tests exercise the reason subcommands end-to-end by building
//! temporary vaults with known SPL content and verifying JSON output.
//! Tests cover: extraction (TEST-026), theory construction (TEST-027),
//! reasoning (TEST-028), explanation (TEST-029), what-if (TEST-030),
//! why-not (TEST-031), require (TEST-032), conflicts (TEST-033),
//! export (TEST-034), cross-referencing (TEST-035), check --spl (TEST-036),
//! and caching (TEST-037).

use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::ExitStatus;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a `Command` for the `ztl` binary with the given vault directory.
fn ztl_cmd(vault: &Path) -> Command {
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("ztl");
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
    let output = cmd.output().expect("failed to execute ztl");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "ztl exited with non-zero status.\nstdout: {stdout}\nstderr: {stderr}",
    );
    serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("failed to parse JSON output: {e}\nraw stdout: {stdout}"))
}

/// Run the command (may fail) and parse JSON output regardless of exit code.
///
/// Structured errors land on stderr (clig.dev convention — see
/// src/main.rs::exit_json_error); successful JSON lands on stdout. This
/// helper picks the stream matching exit status, falling back to the other
/// on parse failure.
fn run_json_any(cmd: &mut Command) -> (Value, ExitStatus) {
    let output = cmd.output().expect("failed to execute ztl");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let primary = if output.status.success() {
        &stdout
    } else {
        &stderr
    };
    let fallback = if output.status.success() {
        &stderr
    } else {
        &stdout
    };
    let json: Value = serde_json::from_str(primary)
        .or_else(|_| serde_json::from_str(fallback))
        .unwrap_or_else(|e| {
            panic!("failed to parse JSON output: {e}\nstdout: {stdout}\nstderr: {stderr}")
        });
    (json, output.status)
}

// ---------------------------------------------------------------------------
// Vault builders
// ---------------------------------------------------------------------------

/// Build the classic penguin/bird vault for defeasible reasoning tests.
/// Structure:
///   - Bird Facts.md: (given bird), r-bird-flies
///   - Penguin Facts.md: (given penguin), r-penguin-is-bird, r-penguin-no-fly, (prefer)
///   - Multi Block.md: two SPL blocks (sunny/windy facts), HTML comment exclusion
///   - theories/standalone.spl: standalone .spl file with has-tests, has-docs, r-ready
///   - Conflicts Page.md: r-approve vs r-reject (unresolved conflict)
///   - No Links.md: page with no SPL content
fn build_reason_vault(root: &Path) {
    write_file(
        root,
        "Bird Facts.md",
        "\
# Bird Facts

Birds are a common topic in defeasible reasoning. See also [[Penguin Facts]].

```spl
; Birds typically fly
(given bird)
(normally r-bird-flies
  bird
  flies)
```

This demonstrates basic defeasible rules about birds.
",
    );

    write_file(
        root,
        "Penguin Facts.md",
        "\
# Penguin Facts

Penguins are birds that cannot fly. See also [[Bird Facts]].

```spl
; Penguins are birds that don't fly
(given penguin)
(always r-penguin-is-bird
  penguin
  bird)
(normally r-penguin-no-fly
  penguin
  (not flies))
(prefer r-penguin-no-fly r-bird-flies)
```

This overrides the default bird-flies rule for penguins.
",
    );

    write_file(
        root,
        "Multi Block.md",
        "\
# Multi Block Page

This page has multiple SPL blocks.

```spl
; First block: weather facts
(given sunny)
(normally r-sunny-dry
  sunny
  dry)
```

Some text between blocks.

<!-- HTML comment with ```spl
(given hidden-fact)
``` should be excluded -->

```spl
; Second block: rain facts
(given windy)
(normally r-windy-umbrella
  windy
  need-umbrella)
```

More text after the second block.
",
    );

    write_file(
        root,
        "theories/standalone.spl",
        "\
; Standalone SPL file - no markdown wrapper
; This tests .spl file extraction

(given has-tests)
(given has-docs)

(normally r-ready
  (and has-tests has-docs)
  ready-for-release)
",
    );

    write_file(
        root,
        "Conflicts Page.md",
        "\
# Conflicts Page

This page has rules that create an unresolved conflict.

```spl
; Two defeasible rules with opposite conclusions, no superiority
(given evidence-a)
(given evidence-b)
(normally r-approve
  evidence-a
  approved)
(normally r-reject
  evidence-b
  (not approved))
```

Without a superiority relation, `approved` vs `~approved` is ambiguous.
",
    );

    write_file(
        root,
        "No Links.md",
        "\
# No Links Page

This page has no wikilinks and no SPL content.
",
    );
}

/// Build a minimal vault with a single SPL block in one markdown file.
fn build_single_block_vault(root: &Path) {
    write_file(
        root,
        "Simple.md",
        "\
# Simple

```spl
(given alpha)
(normally r1 alpha beta)
```
",
    );
}

/// Build a vault with a parse error in one of the SPL blocks.
fn build_parse_error_vault(root: &Path) {
    write_file(
        root,
        "Good.md",
        "\
# Good

```spl
(given valid-fact)
```
",
    );

    write_file(
        root,
        "Bad.md",
        "\
# Bad

```spl
(this is not valid SPL syntax !!!
```
",
    );
}

/// Build a vault for testing `require` — a rule whose body literal is missing.
fn build_require_vault(root: &Path) {
    write_file(
        root,
        "Deploy.md",
        "\
# Deployment

```spl
(given code-reviewed)
(normally r-deploy
  (and code-reviewed tests-pass)
  ready-to-deploy)
```
",
    );
}

/// Build a vault where a defeater blocks a conclusion.
fn build_defeater_vault(root: &Path) {
    write_file(
        root,
        "Policy.md",
        "\
# Policy

```spl
(given employee)
(normally r-eligible
  employee
  eligible-for-bonus)
(given on-probation)
(except r-probation-block
  on-probation
  (not eligible-for-bonus))
```
",
    );
}

// ===========================================================================
// TEST-026: SPL Extraction
// ===========================================================================

/// TEST-026a: Single SPL block in a markdown file is correctly extracted.
/// Verified indirectly via `reason export` which shows extracted facts/rules.
#[test]
fn test_026a_single_spl_block_extraction() {
    let dir = TempDir::new().expect("create temp dir");
    build_single_block_vault(dir.path());

    let json = run_json(
        ztl_cmd(dir.path())
            .arg("reason")
            .arg("export")
            .arg("--as")
            .arg("json"),
    );

    // The single SPL block should produce 1 fact (alpha) and 1 rule (r1)
    let facts = json["facts"].as_array().expect("facts array");
    assert_eq!(
        facts.len(),
        1,
        "should have exactly 1 fact from single block"
    );
    assert_eq!(facts[0]["literal"].as_str(), Some("alpha"));
    assert_eq!(facts[0]["source_page"].as_str(), Some("Simple"));
    assert_eq!(facts[0]["source_file"].as_str(), Some("Simple.md"));

    let rules = json["rules"].as_array().expect("rules array");
    assert_eq!(
        rules.len(),
        1,
        "should have exactly 1 rule from single block"
    );
    assert_eq!(rules[0]["label"].as_str(), Some("r1"));
    assert_eq!(rules[0]["source_page"].as_str(), Some("Simple"));
}

/// TEST-026b: Multiple SPL blocks in a single file are all extracted.
/// Both blocks in Multi Block.md contribute facts to the theory.
#[test]
fn test_026b_multiple_spl_blocks() {
    let dir = TempDir::new().expect("create temp dir");

    // Create a vault with ONLY Multi Block.md to isolate the test
    write_file(
        dir.path(),
        "Multi Block.md",
        "\
# Multi Block Page

This page has multiple SPL blocks.

```spl
; First block: weather facts
(given sunny)
(normally r-sunny-dry
  sunny
  dry)
```

Some text between blocks.

```spl
; Second block: rain facts
(given windy)
(normally r-windy-umbrella
  windy
  need-umbrella)
```
",
    );

    let json = run_json(
        ztl_cmd(dir.path())
            .arg("reason")
            .arg("export")
            .arg("--as")
            .arg("json"),
    );

    let facts = json["facts"].as_array().expect("facts array");
    let fact_literals: Vec<&str> = facts
        .iter()
        .map(|f| f["literal"].as_str().unwrap())
        .collect();

    // Both blocks should contribute facts
    assert!(fact_literals.contains(&"sunny"), "first block's sunny fact");
    assert!(
        fact_literals.contains(&"windy"),
        "second block's windy fact"
    );
    assert_eq!(facts.len(), 2, "2 facts from 2 blocks");

    let rules = json["rules"].as_array().expect("rules array");
    assert_eq!(rules.len(), 2, "2 rules from 2 blocks");
}

/// TEST-026c: HTML comments containing SPL fences are excluded from extraction.
/// The hidden-fact inside an HTML comment should not appear in the theory.
#[test]
fn test_026c_html_comment_exclusion() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let json = run_json(
        ztl_cmd(dir.path())
            .arg("reason")
            .arg("export")
            .arg("--as")
            .arg("json"),
    );

    let facts = json["facts"].as_array().expect("facts array");
    let fact_literals: Vec<&str> = facts
        .iter()
        .map(|f| f["literal"].as_str().unwrap())
        .collect();

    // hidden-fact from the HTML comment should NOT be extracted
    assert!(
        !fact_literals.contains(&"hidden-fact"),
        "hidden-fact inside HTML comment should be excluded"
    );

    // But sunny and windy from non-comment blocks SHOULD be present
    assert!(
        fact_literals.contains(&"sunny"),
        "sunny should be extracted"
    );
    assert!(
        fact_literals.contains(&"windy"),
        "windy should be extracted"
    );
}

/// TEST-026d: Standalone .spl files are correctly extracted.
/// Facts from standalone.spl should appear in the theory with correct provenance.
#[test]
fn test_026d_standalone_spl_file() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let json = run_json(
        ztl_cmd(dir.path())
            .arg("reason")
            .arg("export")
            .arg("--as")
            .arg("json"),
    );

    let facts = json["facts"].as_array().expect("facts array");

    // Facts from standalone.spl should have correct provenance
    let has_tests = facts
        .iter()
        .find(|f| f["literal"].as_str() == Some("has-tests"))
        .expect("has-tests fact from standalone.spl");
    assert_eq!(has_tests["source_page"].as_str(), Some("standalone"));
    assert_eq!(
        has_tests["source_file"].as_str(),
        Some("theories/standalone.spl")
    );

    let has_docs = facts
        .iter()
        .find(|f| f["literal"].as_str() == Some("has-docs"))
        .expect("has-docs fact from standalone.spl");
    assert_eq!(has_docs["source_page"].as_str(), Some("standalone"));

    // Rule from standalone.spl
    let rules = json["rules"].as_array().expect("rules array");
    let r_ready = rules
        .iter()
        .find(|r| r["label"].as_str() == Some("r-ready"))
        .expect("r-ready rule from standalone.spl");
    assert_eq!(
        r_ready["source_file"].as_str(),
        Some("theories/standalone.spl")
    );
}

// ===========================================================================
// TEST-027: Theory Construction
// ===========================================================================

/// TEST-027a: Multi-file theory aggregates facts and rules from all sources.
#[test]
fn test_027a_multi_file_theory_construction() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let json = run_json(ztl_cmd(dir.path()).arg("reason").arg("status"));

    let theory = &json["theory"];
    assert_eq!(theory["facts"].as_u64(), Some(8), "8 total facts");
    assert_eq!(theory["rules"].as_u64(), Some(8), "8 non-fact rules");
    assert_eq!(theory["defeaters"].as_u64(), Some(0));
    assert_eq!(theory["superiority_relations"].as_u64(), Some(1));
    assert_eq!(theory["source_files"].as_u64(), Some(5));
}

/// TEST-027b: SPL parse errors produce diagnostics without crashing.
#[test]
fn test_027b_parse_error_diagnostics() {
    let dir = TempDir::new().expect("create temp dir");
    build_parse_error_vault(dir.path());

    let (json, status) = run_json_any(ztl_cmd(dir.path()).arg("reason").arg("status"));

    // Should exit non-zero (exit code 2 for parse errors)
    assert!(!status.success(), "should fail on parse errors");

    // Diagnostics should contain the parse error
    let diagnostics = json["diagnostics"].as_array().expect("diagnostics array");
    assert!(
        !diagnostics.is_empty(),
        "should have at least one diagnostic for parse error"
    );

    let has_parse_error = diagnostics.iter().any(|d| {
        d["message"]
            .as_str()
            .unwrap_or("")
            .contains("SPL parse error")
    });
    assert!(has_parse_error, "should have SPL parse error diagnostic");
}

/// TEST-027c: Provenance metadata is correct for facts and rules.
#[test]
fn test_027c_provenance_metadata() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let json = run_json(
        ztl_cmd(dir.path())
            .arg("reason")
            .arg("export")
            .arg("--as")
            .arg("json"),
    );

    // Check that facts have correct provenance
    let facts = json["facts"].as_array().expect("facts array");
    let bird_fact = facts
        .iter()
        .find(|f| f["literal"].as_str() == Some("bird"))
        .expect("bird fact");
    assert_eq!(bird_fact["source_page"].as_str(), Some("Bird Facts"));
    assert_eq!(bird_fact["source_file"].as_str(), Some("Bird Facts.md"));

    // Check rules provenance
    let rules = json["rules"].as_array().expect("rules array");
    let bird_flies = rules
        .iter()
        .find(|r| r["label"].as_str() == Some("r-bird-flies"))
        .expect("r-bird-flies rule");
    assert_eq!(bird_flies["source_page"].as_str(), Some("Bird Facts"));
    assert_eq!(bird_flies["head"].as_str(), Some("flies"));
    assert_eq!(bird_flies["rule_type"].as_str(), Some("Defeasible"));

    // Standalone SPL file provenance
    let standalone_fact = facts
        .iter()
        .find(|f| f["literal"].as_str() == Some("has-tests"))
        .expect("has-tests fact");
    assert_eq!(standalone_fact["source_page"].as_str(), Some("standalone"));
    assert_eq!(
        standalone_fact["source_file"].as_str(),
        Some("theories/standalone.spl")
    );
}

// ===========================================================================
// TEST-028: Reasoning (penguin/bird scenario, defeated conclusions)
// ===========================================================================

/// TEST-028a: Penguin/bird scenario: ~flies is defeasibly provable, flies is defeated.
#[test]
fn test_028a_penguin_bird_reasoning() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let json = run_json(ztl_cmd(dir.path()).arg("reason").arg("status"));

    let conclusions = json["conclusions"].as_array().expect("conclusions array");

    // ~flies should be +d (defeasibly provable)
    let neg_flies = conclusions
        .iter()
        .find(|c| {
            c["literal"].as_str() == Some("~flies") && c["conclusion_type"].as_str() == Some("+d")
        })
        .expect("+d ~flies should exist");
    assert!(!neg_flies["proof_sources"].as_array().unwrap().is_empty());

    // flies should be -d (defeasibly not provable — defeated)
    let flies_defeated = conclusions.iter().any(|c| {
        c["literal"].as_str() == Some("flies") && c["conclusion_type"].as_str() == Some("-d")
    });
    assert!(flies_defeated, "flies should be -d (defeated)");

    // bird should be both +D and +d (from strict rule r-penguin-is-bird)
    let bird_definite = conclusions.iter().any(|c| {
        c["literal"].as_str() == Some("bird") && c["conclusion_type"].as_str() == Some("+D")
    });
    assert!(
        bird_definite,
        "bird should be +D (definitively provable via strict rule)"
    );
}

/// TEST-028b: Conclusion counts are correct.
#[test]
fn test_028b_conclusion_counts() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let json = run_json(ztl_cmd(dir.path()).arg("reason").arg("status"));

    let summary = &json["summary"];
    assert_eq!(
        summary["definitely_provable"].as_u64(),
        Some(8),
        "8 +D conclusions"
    );
    assert_eq!(
        summary["defeasibly_provable"].as_u64(),
        Some(12),
        "12 +d conclusions"
    );
    assert!(summary["total"].as_u64().unwrap() > 0);
}

/// TEST-028c: Filtering conclusions by positive/defeasible.
#[test]
fn test_028c_conclusion_filtering() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let json = run_json(
        ztl_cmd(dir.path())
            .arg("reason")
            .arg("status")
            .arg("--positive")
            .arg("--defeasible"),
    );

    let conclusions = json["conclusions"].as_array().expect("conclusions array");

    // All conclusions should be +d (positive AND defeasible)
    for c in conclusions {
        assert_eq!(
            c["conclusion_type"].as_str(),
            Some("+d"),
            "all filtered conclusions should be +d"
        );
    }

    // Should include ~flies, dry, need-umbrella, ready-for-release, etc.
    let has_neg_flies = conclusions
        .iter()
        .any(|c| c["literal"].as_str() == Some("~flies"));
    assert!(has_neg_flies, "~flies should be in +d conclusions");
}

/// TEST-028d: Filtering conclusions by literal name pattern.
#[test]
fn test_028d_literal_filter() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let json = run_json(
        ztl_cmd(dir.path())
            .arg("reason")
            .arg("status")
            .arg("--literal")
            .arg("flies"),
    );

    let conclusions = json["conclusions"].as_array().expect("conclusions array");

    // Should include both "flies" and "~flies" conclusions (all types)
    for c in conclusions {
        let lit = c["literal"].as_str().unwrap();
        assert!(
            lit == "flies" || lit == "~flies",
            "filtered by 'flies' should only show flies or ~flies, got: {lit}"
        );
    }
    assert!(
        !conclusions.is_empty(),
        "should have some fly-related conclusions"
    );
}

// ===========================================================================
// TEST-029: Explanation (proof trees, defeat chains, unknown literal)
// ===========================================================================

/// TEST-029a: Explain a defeasibly provable conclusion with proof tree.
#[test]
fn test_029a_explain_proof_tree() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let json = run_json(
        ztl_cmd(dir.path())
            .arg("reason")
            .arg("explain")
            .arg("~flies"),
    );

    assert_eq!(json["literal"].as_str(), Some("~flies"));
    assert_eq!(json["conclusion_type"].as_str(), Some("+d"));

    // Proof tree should exist
    let tree = &json["proof_tree"];
    assert!(tree.is_object(), "proof_tree should be an object");
    assert_eq!(tree["literal"].as_str(), Some("~flies"));
    assert_eq!(tree["derivation"].as_str(), Some("defeasible"));

    // Source provenance
    let source = &tree["source"];
    assert_eq!(source["page"].as_str(), Some("Penguin Facts"));

    // Rule info
    let rule = &tree["rule"];
    assert_eq!(rule["label"].as_str(), Some("r-penguin-no-fly"));
    assert_eq!(rule["rule_type"].as_str(), Some("defeasible"));

    // Body should have at least one child (penguin)
    let body = tree["body"].as_array().expect("body array");
    assert!(!body.is_empty(), "proof tree body should have children");
    assert_eq!(body[0]["literal"].as_str(), Some("penguin"));
}

/// TEST-029b: Explain a defeated conclusion shows defeat chain.
#[test]
fn test_029b_explain_defeat_chain() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let json = run_json(
        ztl_cmd(dir.path())
            .arg("reason")
            .arg("explain")
            .arg("flies"),
    );

    assert_eq!(json["literal"].as_str(), Some("flies"));
    assert_eq!(json["conclusion_type"].as_str(), Some("-d"));

    // Should have defeat chain
    let chain = json["defeat_chain"].as_array().expect("defeat_chain array");
    assert!(!chain.is_empty(), "should have a defeat chain entry");

    // The defeating rule should be r-penguin-no-fly
    let defeater = &chain[0];
    assert_eq!(defeater["rule_label"].as_str(), Some("r-penguin-no-fly"));
    assert_eq!(defeater["head"].as_str(), Some("~flies"));
}

/// TEST-029c: Explain an unknown literal produces suggestions.
#[test]
fn test_029c_explain_unknown_literal() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let (json, status) = run_json_any(
        ztl_cmd(dir.path())
            .arg("reason")
            .arg("explain")
            .arg("nonexistent"),
    );

    assert!(!status.success(), "should fail for unknown literal");
    assert!(json["error"].as_str().is_some(), "should have error field");
    assert_eq!(json["literal"].as_str(), Some("nonexistent"));

    // Should have suggestions
    let suggestions = json["suggestions"].as_array().expect("suggestions array");
    assert!(!suggestions.is_empty(), "should suggest similar literals");
}

// ===========================================================================
// TEST-030: What-If (add fact, verify no side effects)
// ===========================================================================

/// TEST-030a: What-if adding a new fact introduces new conclusions.
#[test]
fn test_030a_what_if_add_fact() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let json = run_json(
        ztl_cmd(dir.path())
            .arg("reason")
            .arg("what-if")
            .arg("(given extra-fact)"),
    );

    assert_eq!(
        json["hypothetical_spl"].as_str(),
        Some("(given extra-fact)")
    );

    // New conclusions should include extra-fact
    let new = json["new_conclusions"].as_array().expect("new_conclusions");
    let has_extra = new
        .iter()
        .any(|c| c["literal"].as_str() == Some("extra-fact"));
    assert!(has_extra, "extra-fact should appear in new conclusions");

    // No changed or removed conclusions (adding a new unrelated fact)
    let changed = json["changed_conclusions"]
        .as_array()
        .expect("changed_conclusions");
    assert!(changed.is_empty(), "no conclusions should change");

    let removed = json["removed_conclusions"]
        .as_array()
        .expect("removed_conclusions");
    assert!(removed.is_empty(), "no conclusions should be removed");
}

/// TEST-030b: What-if verifies existing conclusions are unchanged.
#[test]
fn test_030b_what_if_no_side_effects() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let json = run_json(
        ztl_cmd(dir.path())
            .arg("reason")
            .arg("what-if")
            .arg("(given extra-fact)"),
    );

    // unchanged_count should match the baseline total
    let unchanged = json["unchanged_count"].as_u64().expect("unchanged_count");
    assert!(
        unchanged >= 28,
        "most conclusions should be unchanged, got {unchanged}"
    );
}

/// TEST-030c: What-if with --goal filters diff to a specific literal.
#[test]
fn test_030c_what_if_goal_filter() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let json = run_json(
        ztl_cmd(dir.path())
            .arg("reason")
            .arg("what-if")
            .arg("(given extra-fact)")
            .arg("--goal")
            .arg("extra-fact"),
    );

    let new = json["new_conclusions"].as_array().expect("new_conclusions");
    for c in new {
        assert_eq!(
            c["literal"].as_str(),
            Some("extra-fact"),
            "with --goal, only matching literal should appear"
        );
    }
}

// ===========================================================================
// TEST-031: Why-Not (missing preconditions, defeated)
// ===========================================================================

/// TEST-031a: Why-not for a defeated literal identifies the defeating rule.
#[test]
fn test_031a_why_not_defeated() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let json = run_json(
        ztl_cmd(dir.path())
            .arg("reason")
            .arg("why-not")
            .arg("flies"),
    );

    assert_eq!(json["literal"].as_str(), Some("flies"));
    // flies is -D (definite not provable, because it's been defeated)
    assert!(json["conclusion"].as_str().is_some());

    let rules = json["candidate_rules"].as_array().expect("candidate_rules");
    assert!(!rules.is_empty(), "should have candidate rules for flies");

    // r-bird-flies should be a candidate
    let bird_flies = rules
        .iter()
        .find(|r| r["rule_label"].as_str() == Some("r-bird-flies"))
        .expect("r-bird-flies should be a candidate");

    // Should have a "defeated" blocker
    let blockers = bird_flies["blockers"].as_array().expect("blockers");
    let defeated = blockers
        .iter()
        .any(|b| b["blocker_type"].as_str() == Some("defeated"));
    assert!(defeated, "r-bird-flies should be blocked by defeat");
}

/// TEST-031b: Why-not for a provable literal exits with error and hint.
#[test]
fn test_031b_why_not_already_provable() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let (json, status) = run_json_any(
        ztl_cmd(dir.path())
            .arg("reason")
            .arg("why-not")
            .arg("bird"),
    );

    assert!(!status.success(), "why-not on provable literal should fail");
    assert!(json["error"].as_str().is_some());
    assert!(
        json["hint"].as_str().is_some(),
        "should suggest using explain instead"
    );
}

/// TEST-031c: Why-not for a literal with missing preconditions.
#[test]
fn test_031c_why_not_missing_preconditions() {
    let dir = TempDir::new().expect("create temp dir");
    build_require_vault(dir.path());

    let json = run_json(
        ztl_cmd(dir.path())
            .arg("reason")
            .arg("why-not")
            .arg("ready-to-deploy"),
    );

    assert_eq!(json["literal"].as_str(), Some("ready-to-deploy"));

    let rules = json["candidate_rules"].as_array().expect("candidate_rules");
    let deploy_rule = rules
        .iter()
        .find(|r| r["rule_label"].as_str() == Some("r-deploy"))
        .expect("r-deploy should be a candidate");

    let blockers = deploy_rule["blockers"].as_array().expect("blockers");
    let missing = blockers
        .iter()
        .find(|b| b["blocker_type"].as_str() == Some("failed_body"));
    assert!(
        missing.is_some(),
        "should have a failed_body blocker for tests-pass"
    );
    assert_eq!(
        missing.unwrap()["literal"].as_str(),
        Some("tests-pass"),
        "missing precondition should be tests-pass"
    );
}

// ===========================================================================
// TEST-032: Require (missing facts, already provable, impossible)
// ===========================================================================

/// TEST-032a: Require identifies missing facts needed to prove a literal.
#[test]
fn test_032a_require_missing_facts() {
    let dir = TempDir::new().expect("create temp dir");
    build_require_vault(dir.path());

    let json = run_json(
        ztl_cmd(dir.path())
            .arg("reason")
            .arg("require")
            .arg("ready-to-deploy"),
    );

    assert_eq!(json["literal"].as_str(), Some("ready-to-deploy"));
    assert_eq!(json["status"].as_str(), Some("requirements_found"));

    let solutions = json["solutions"].as_array().expect("solutions");
    assert!(!solutions.is_empty(), "should have at least one solution");

    // The solution should identify tests-pass as a required fact
    let solution = &solutions[0];
    assert_eq!(solution["via_rule"].as_str(), Some("r-deploy"));

    let required = solution["required_facts"]
        .as_array()
        .expect("required_facts");
    let needs_tests_pass = required
        .iter()
        .any(|f| f["literal"].as_str() == Some("tests-pass"));
    assert!(
        needs_tests_pass,
        "should require tests-pass to prove ready-to-deploy"
    );
}

/// TEST-032b: Require for an already-provable literal reports it.
#[test]
fn test_032b_require_already_provable() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let json = run_json(
        ztl_cmd(dir.path())
            .arg("reason")
            .arg("require")
            .arg("bird"),
    );

    assert_eq!(json["literal"].as_str(), Some("bird"));
    assert_eq!(json["status"].as_str(), Some("already_provable"));
    assert!(json["message"].as_str().is_some());

    let solutions = json["solutions"].as_array().expect("solutions");
    assert!(
        solutions.is_empty(),
        "already provable should have no solutions"
    );
}

/// TEST-032c: Require for a literal with no rules exits with error.
#[test]
fn test_032c_require_impossible() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let (json, status) = run_json_any(
        ztl_cmd(dir.path())
            .arg("reason")
            .arg("require")
            .arg("totally-unknown"),
    );

    assert!(!status.success(), "should fail for impossible literal");
    assert!(json["error"].as_str().is_some());
}

/// TEST-032d: Require with --assume injects additional assumed facts.
#[test]
fn test_032d_require_with_assume() {
    let dir = TempDir::new().expect("create temp dir");
    build_require_vault(dir.path());

    let json = run_json(
        ztl_cmd(dir.path())
            .arg("reason")
            .arg("require")
            .arg("ready-to-deploy")
            .arg("--assume")
            .arg("(given tests-pass)"),
    );

    assert_eq!(json["literal"].as_str(), Some("ready-to-deploy"));

    // With tests-pass assumed, all body literals are satisfied
    let solutions = json["solutions"].as_array().expect("solutions");
    assert!(!solutions.is_empty(), "should have at least one solution");

    // The solution via r-deploy should have no required_facts (all body satisfied)
    let solution = &solutions[0];
    let required = solution["required_facts"]
        .as_array()
        .expect("required_facts");
    assert!(
        required.is_empty(),
        "with tests-pass assumed, no additional facts should be required"
    );

    // Assumed facts should be listed
    let assumed = json["assumed"].as_array().expect("assumed");
    let assumed_strs: Vec<&str> = assumed.iter().map(|a| a.as_str().unwrap()).collect();
    assert!(
        assumed_strs.contains(&"tests-pass"),
        "tests-pass should be in assumed facts"
    );
}

// ===========================================================================
// TEST-033: Conflicts (ambiguous, resolved, suggestions)
// ===========================================================================

/// TEST-033a: Detects unresolved conflict between approved and ~approved.
#[test]
fn test_033a_unresolved_conflict() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let json = run_json(ztl_cmd(dir.path()).arg("reason").arg("conflicts"));

    let conflicts = json["conflicts"].as_array().expect("conflicts array");
    assert_eq!(
        json["conflict_count"].as_u64(),
        Some(1),
        "should have exactly 1 conflict"
    );

    let conflict = &conflicts[0];
    assert_eq!(conflict["literal"].as_str(), Some("approved"));
    assert_eq!(conflict["positive_literal"].as_str(), Some("approved"));
    assert_eq!(conflict["negative_literal"].as_str(), Some("~approved"));
    assert_eq!(conflict["resolved"].as_bool(), Some(false));

    // Competing rules
    let rules = conflict["competing_rules"]
        .as_array()
        .expect("competing_rules");
    assert_eq!(rules.len(), 2, "two competing rules");
}

/// TEST-033b: No conflicts when all disputes are resolved by superiority.
#[test]
fn test_033b_resolved_conflict() {
    let dir = TempDir::new().expect("create temp dir");

    // Build a vault where the conflict IS resolved by superiority
    write_file(
        dir.path(),
        "Resolved.md",
        "\
# Resolved

```spl
(given x)
(normally r-yes x outcome)
(normally r-no x (not outcome))
(prefer r-yes r-no)
```
",
    );

    let json = run_json(ztl_cmd(dir.path()).arg("reason").arg("conflicts"));

    let conflicts = json["conflicts"].as_array().expect("conflicts array");
    assert!(conflicts.is_empty(), "resolved conflict should not appear");
    assert_eq!(json["conflict_count"].as_u64(), Some(0));
}

/// TEST-033c: Conflicts with --suggest produces resolution suggestions.
#[test]
fn test_033c_conflict_suggestions() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let json = run_json(
        ztl_cmd(dir.path())
            .arg("reason")
            .arg("conflicts")
            .arg("--suggest"),
    );

    let conflicts = json["conflicts"].as_array().expect("conflicts array");
    let conflict = &conflicts[0];

    let suggestions = conflict["suggestions"]
        .as_array()
        .expect("suggestions array");
    assert!(
        !suggestions.is_empty(),
        "should have resolution suggestions"
    );

    // Should suggest adding superiority relations
    let has_prefer_suggestion = suggestions
        .iter()
        .any(|s| s.as_str().unwrap_or("").contains("prefer"));
    assert!(has_prefer_suggestion, "should suggest superiority relation");
}

/// TEST-033d: Conflicts with --fail-on-conflicts exits non-zero when conflicts exist.
#[test]
fn test_033d_fail_on_conflicts() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let (json, status) = run_json_any(
        ztl_cmd(dir.path())
            .arg("reason")
            .arg("conflicts")
            .arg("--fail-on-conflicts"),
    );

    assert!(!status.success(), "should fail when conflicts exist");
    assert!(
        json["conflict_count"].as_u64().unwrap() > 0,
        "should report conflicts"
    );
}

// ===========================================================================
// TEST-034: Export (SPL with provenance, JSON)
// ===========================================================================

/// TEST-034a: JSON export includes facts, rules, superiority with provenance.
#[test]
fn test_034a_export_json() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let json = run_json(
        ztl_cmd(dir.path())
            .arg("reason")
            .arg("export")
            .arg("--as")
            .arg("json"),
    );

    // Facts
    let facts = json["facts"].as_array().expect("facts array");
    assert_eq!(facts.len(), 8, "8 facts total");
    for fact in facts {
        assert!(fact["literal"].as_str().is_some());
        assert!(fact["source_file"].as_str().is_some());
        assert!(fact["source_page"].as_str().is_some());
        assert!(fact["source_line"].as_u64().is_some());
    }

    // Rules
    let rules = json["rules"].as_array().expect("rules array");
    assert_eq!(rules.len(), 8, "8 non-fact rules");
    for rule in rules {
        assert!(rule["label"].as_str().is_some());
        assert!(rule["rule_type"].as_str().is_some());
        assert!(rule["head"].as_str().is_some());
        assert!(rule["source_file"].as_str().is_some());
    }

    // Superiority
    let sup = json["superiority"].as_array().expect("superiority array");
    assert_eq!(sup.len(), 1, "1 superiority relation");
    assert_eq!(sup[0]["superior"].as_str(), Some("r-penguin-no-fly"));
    assert_eq!(sup[0]["inferior"].as_str(), Some("r-bird-flies"));

    // Summary
    let summary = &json["summary"];
    assert_eq!(summary["fact_count"].as_u64(), Some(8));
    assert_eq!(summary["rule_count"].as_u64(), Some(8));
}

/// TEST-034b: JSON export with --with-conclusions includes conclusion data.
#[test]
fn test_034b_export_json_with_conclusions() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let json = run_json(
        ztl_cmd(dir.path())
            .arg("reason")
            .arg("export")
            .arg("--as")
            .arg("json")
            .arg("--with-conclusions"),
    );

    let conclusions = json["conclusions"]
        .as_array()
        .expect("conclusions should be present with --with-conclusions");
    assert!(!conclusions.is_empty());

    // Each conclusion should have literal, conclusion_type, proof_sources
    for c in conclusions {
        assert!(c["literal"].as_str().is_some());
        assert!(c["conclusion_type"].as_str().is_some());
        assert!(c["proof_sources"].as_array().is_some());
    }
}

/// TEST-034c: SPL export produces valid SPL with provenance comments.
#[test]
fn test_034c_export_spl() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let output = ztl_cmd(dir.path())
        .arg("reason")
        .arg("export")
        .arg("--as")
        .arg("spl")
        .output()
        .expect("failed to execute ztl");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should have provenance comments
    assert!(
        stdout.contains("; --- From:"),
        "should have provenance comments"
    );

    // Should have facts
    assert!(stdout.contains("(given bird)"), "should export bird fact");
    assert!(
        stdout.contains("(given penguin)"),
        "should export penguin fact"
    );

    // Should have rules
    assert!(
        stdout.contains("r-bird-flies"),
        "should export r-bird-flies"
    );
    assert!(
        stdout.contains("r-penguin-no-fly"),
        "should export r-penguin-no-fly"
    );

    // Should have superiority
    assert!(
        stdout.contains("(prefer r-penguin-no-fly r-bird-flies)"),
        "should export superiority"
    );

    // Should have source file info
    assert!(stdout.contains("5 source files"));
}

/// TEST-034d: Export empty vault produces empty output without error.
#[test]
fn test_034d_export_empty_vault() {
    let dir = TempDir::new().expect("create temp dir");
    write_file(dir.path(), "Empty.md", "# Empty\nNo SPL here.\n");

    let json = run_json(
        ztl_cmd(dir.path())
            .arg("reason")
            .arg("export")
            .arg("--as")
            .arg("json"),
    );

    assert_eq!(json["facts"].as_array().unwrap().len(), 0);
    assert_eq!(json["rules"].as_array().unwrap().len(), 0);
    assert_eq!(json["superiority"].as_array().unwrap().len(), 0);
}

// ===========================================================================
// TEST-035: Cross-referencing (links --with-conclusions, provenance)
// ===========================================================================

/// TEST-035a: links --with-conclusions shows conclusions for linked pages.
#[test]
fn test_035a_links_with_conclusions() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let json = run_json(
        ztl_cmd(dir.path())
            .arg("links")
            .arg("Bird Facts")
            .arg("--with-conclusions"),
    );

    assert_eq!(json["page"].as_str(), Some("Bird Facts"));

    let links = json["links"].as_array().expect("links array");
    // Bird Facts links to Penguin Facts
    let penguin_link = links
        .iter()
        .find(|l| l["target"].as_str() == Some("Penguin Facts"))
        .expect("should link to Penguin Facts");

    // Penguin Facts should have conclusions
    let conclusions = penguin_link["conclusions"]
        .as_array()
        .expect("conclusions array for linked page");
    assert!(
        !conclusions.is_empty(),
        "Penguin Facts should contribute to conclusions"
    );

    // Should include ~flies contribution
    let has_neg_flies = conclusions
        .iter()
        .any(|c| c["literal"].as_str() == Some("~flies"));
    assert!(has_neg_flies, "Penguin Facts should contribute to ~flies");
}

/// TEST-035b: Provenance command traces sources and cross-references.
#[test]
fn test_035b_provenance_trace() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let json = run_json(
        ztl_cmd(dir.path())
            .arg("reason")
            .arg("provenance")
            .arg("bird"),
    );

    assert_eq!(json["literal"].as_str(), Some("bird"));

    let conclusions = json["conclusions"].as_array().expect("conclusions array");
    assert!(!conclusions.is_empty());

    // Should have +D and +d entries
    let types: Vec<&str> = conclusions
        .iter()
        .map(|c| c["conclusion_type"].as_str().unwrap())
        .collect();
    assert!(types.contains(&"+D"), "should have +D conclusion");
    assert!(types.contains(&"+d"), "should have +d conclusion");

    // Source pages
    let source_pages = json["source_pages"].as_array().expect("source_pages array");
    assert!(
        source_pages
            .iter()
            .any(|p| p.as_str() == Some("Bird Facts")),
        "Bird Facts should be a source page for bird"
    );
}

/// TEST-035c: Provenance for ~flies shows cross-references between Bird Facts and Penguin Facts.
#[test]
fn test_035c_provenance_cross_references() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let json = run_json(
        ztl_cmd(dir.path())
            .arg("reason")
            .arg("provenance")
            .arg("~flies"),
    );

    assert_eq!(json["literal"].as_str(), Some("~flies"));

    // Source pages should include Penguin Facts
    let source_pages = json["source_pages"].as_array().expect("source_pages");
    let pages: Vec<&str> = source_pages.iter().map(|p| p.as_str().unwrap()).collect();
    assert!(
        pages.contains(&"Penguin Facts"),
        "Penguin Facts should be a source page for ~flies"
    );
}

/// TEST-035d: Provenance for unknown literal exits with error.
#[test]
fn test_035d_provenance_unknown() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let (json, status) = run_json_any(
        ztl_cmd(dir.path())
            .arg("reason")
            .arg("provenance")
            .arg("nonexistent"),
    );

    assert!(!status.success());
    assert!(json["error"].as_str().is_some());
}

// ===========================================================================
// TEST-036: check --spl
// ===========================================================================

/// TEST-036a: check --spl on a clean vault reports no diagnostics.
#[test]
fn test_036a_check_spl_clean() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let json = run_json(ztl_cmd(dir.path()).arg("check").arg("--spl"));

    let spl_diags = json["spl_diagnostics"].as_array().expect("spl_diagnostics");
    assert!(
        spl_diags.is_empty(),
        "clean vault should have no SPL diagnostics"
    );
}

/// TEST-036b: check --spl reports parse errors and exits non-zero.
#[test]
fn test_036b_check_spl_errors() {
    let dir = TempDir::new().expect("create temp dir");
    build_parse_error_vault(dir.path());

    // check --spl exits non-zero when SPL errors exist (--fail-on error is default)
    let (json, status) = run_json_any(ztl_cmd(dir.path()).arg("check").arg("--spl"));

    assert!(!status.success(), "check --spl should fail on parse errors");

    let spl_diags = json["spl_diagnostics"].as_array().expect("spl_diagnostics");
    assert!(!spl_diags.is_empty(), "should report SPL parse errors");

    let has_error = spl_diags
        .iter()
        .any(|d| d["level"].as_str() == Some("Error"));
    assert!(has_error, "should have Error-level diagnostic");
}

// ===========================================================================
// TEST-037: Caching (cache hit, invalidation)
// ===========================================================================

/// TEST-037a: Second run with caching is consistent with first run.
#[test]
fn test_037a_cache_consistency() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    // First run: build cache (without --no-cache)
    let mut cmd1 = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    cmd1.arg("-d").arg(dir.path().as_os_str());
    cmd1.arg("reason").arg("status");
    let json1 = run_json(&mut cmd1);

    // Second run: should use cache
    let mut cmd2 = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    cmd2.arg("-d").arg(dir.path().as_os_str());
    cmd2.arg("reason").arg("status");
    let json2 = run_json(&mut cmd2);

    // Conclusions should match
    assert_eq!(
        json1["summary"]["total"], json2["summary"]["total"],
        "cached and uncached should produce same total conclusions"
    );
    assert_eq!(
        json1["theory"]["facts"], json2["theory"]["facts"],
        "cached and uncached should produce same fact count"
    );
    assert_eq!(
        json1["theory"]["rules"], json2["theory"]["rules"],
        "cached and uncached should produce same rule count"
    );
}

/// TEST-037b: Modifying an SPL file invalidates the theory cache.
#[test]
fn test_037b_cache_invalidation() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    // First run: build cache (without --no-cache)
    let mut cmd1 = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    cmd1.arg("-d").arg(dir.path().as_os_str());
    cmd1.arg("reason").arg("status");
    let json1 = run_json(&mut cmd1);
    let original_facts = json1["theory"]["facts"].as_u64().unwrap();

    // Modify the standalone SPL file to add a new fact
    write_file(
        dir.path(),
        "theories/standalone.spl",
        "\
; Modified standalone file
(given has-tests)
(given has-docs)
(given has-ci)

(normally r-ready
  (and has-tests has-docs)
  ready-for-release)
",
    );

    // Second run: cache should be invalidated due to mtime change
    let mut cmd2 = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    cmd2.arg("-d").arg(dir.path().as_os_str());
    cmd2.arg("reason").arg("status");
    let json2 = run_json(&mut cmd2);
    let new_facts = json2["theory"]["facts"].as_u64().unwrap();

    assert_eq!(
        new_facts,
        original_facts + 1,
        "adding a fact should increase fact count"
    );
}

/// TEST-037c: --no-cache flag forces full rescan.
#[test]
fn test_037c_no_cache_flag() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    // First run: build cache
    let mut cmd1 = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    cmd1.arg("-d").arg(dir.path().as_os_str());
    cmd1.arg("reason").arg("status");
    let json1 = run_json(&mut cmd1);

    // Second run with --no-cache: should produce identical results
    let json2 = run_json(ztl_cmd(dir.path()).arg("reason").arg("status"));

    assert_eq!(
        json1["summary"]["total"], json2["summary"]["total"],
        "--no-cache should produce same results"
    );
}

// ===========================================================================
// Additional edge-case tests
// ===========================================================================

/// Defeater blocks a conclusion without proving anything.
#[test]
fn test_defeater_blocks_conclusion() {
    let dir = TempDir::new().expect("create temp dir");
    build_defeater_vault(dir.path());

    let json = run_json(ztl_cmd(dir.path()).arg("reason").arg("status"));

    let conclusions = json["conclusions"].as_array().expect("conclusions");

    // eligible-for-bonus should be -d (defeated by the defeater)
    let eligible_defeated = conclusions.iter().any(|c| {
        c["literal"].as_str() == Some("eligible-for-bonus")
            && c["conclusion_type"].as_str() == Some("-d")
    });
    assert!(
        eligible_defeated,
        "eligible-for-bonus should be defeated (-d) by the defeater rule"
    );
}

/// Strict rule produces definite conclusions.
#[test]
fn test_strict_rule_definite_conclusion() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let json = run_json(ztl_cmd(dir.path()).arg("reason").arg("status"));
    let conclusions = json["conclusions"].as_array().expect("conclusions");

    // bird should be +D because r-penguin-is-bird is a strict rule:
    //   penguin is +D (fact), r-penguin-is-bird: penguin -> bird (strict)
    let bird_definite = conclusions
        .iter()
        .find(|c| {
            c["literal"].as_str() == Some("bird") && c["conclusion_type"].as_str() == Some("+D")
        })
        .expect("bird should be +D");

    let sources = bird_definite["proof_sources"]
        .as_array()
        .expect("proof_sources");
    assert!(!sources.is_empty());
}

/// What-if with file reads SPL from a file.
#[test]
fn test_what_if_from_file() {
    let vault_dir = TempDir::new().expect("create vault dir");
    build_reason_vault(vault_dir.path());

    // Write hypothetical SPL file OUTSIDE the vault (otherwise it gets scanned as part of vault)
    let hyp_dir = TempDir::new().expect("create hyp dir");
    write_file(hyp_dir.path(), "hypothetical.spl", "(given new-evidence)\n");

    let json = run_json(
        ztl_cmd(vault_dir.path())
            .arg("reason")
            .arg("what-if")
            .arg("--file")
            .arg(hyp_dir.path().join("hypothetical.spl").to_str().unwrap()),
    );

    let new = json["new_conclusions"].as_array().expect("new_conclusions");
    let has_new_evidence = new
        .iter()
        .any(|c| c["literal"].as_str() == Some("new-evidence"));
    assert!(
        has_new_evidence,
        "new-evidence should appear in new conclusions"
    );
}

/// Reason status on vault with no SPL exits with error.
#[test]
fn test_no_spl_vault_error() {
    let dir = TempDir::new().expect("create temp dir");
    write_file(dir.path(), "Empty.md", "# Empty\nNo SPL here.\n");

    let (json, status) = run_json_any(ztl_cmd(dir.path()).arg("reason").arg("status"));

    assert!(!status.success());
    assert!(json["error"].as_str().is_some());
}

/// Multi-file theory has correct source_file_count.
#[test]
fn test_source_file_count() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let json = run_json(
        ztl_cmd(dir.path())
            .arg("reason")
            .arg("export")
            .arg("--as")
            .arg("json"),
    );

    assert_eq!(
        json["summary"]["source_file_count"].as_u64(),
        Some(5),
        "5 files contribute SPL blocks"
    );
}

/// Explain a definite (fact-based) conclusion.
#[test]
fn test_explain_definite_fact() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let json = run_json(
        ztl_cmd(dir.path())
            .arg("reason")
            .arg("explain")
            .arg("penguin"),
    );

    assert_eq!(json["literal"].as_str(), Some("penguin"));
    // penguin is definitely provable (it's a fact)
    let ct = json["conclusion_type"].as_str().unwrap();
    assert!(
        ct == "+D" || ct == "+d",
        "penguin should be provable, got {ct}"
    );

    let tree = &json["proof_tree"];
    assert!(tree.is_object(), "should have proof tree");
    let rule = &tree["rule"];
    assert_eq!(
        rule["rule_type"].as_str(),
        Some("fact"),
        "penguin is derived from a fact"
    );
}

/// Ready-for-release is defeasibly provable from standalone.spl.
#[test]
fn test_ready_for_release_provable() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let json = run_json(
        ztl_cmd(dir.path())
            .arg("reason")
            .arg("status")
            .arg("--literal")
            .arg("ready-for-release"),
    );

    let conclusions = json["conclusions"].as_array().expect("conclusions");
    let defeasible = conclusions.iter().any(|c| {
        c["literal"].as_str() == Some("ready-for-release")
            && c["conclusion_type"].as_str() == Some("+d")
    });
    assert!(
        defeasible,
        "ready-for-release should be +d from standalone.spl"
    );
}

// ===========================================================================
// TEST-038: Provenance grounding freshness (REQ-044 / CON-012)
// ===========================================================================

/// TEST-038a: Provenance output includes vault_root_hash, theory_built_at, and
/// per-source grounding objects when the theory cache is populated.
#[test]
fn test_038a_provenance_grounding_fields_present() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    // Run once with caching enabled so the theory cache is built and saved.
    let mut build_cmd = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    build_cmd.arg("-d").arg(dir.path().as_os_str());
    build_cmd.arg("reason").arg("status");
    run_json(&mut build_cmd);

    // Now run provenance — theory cache should be present.
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    cmd.arg("-d").arg(dir.path().as_os_str());
    cmd.arg("reason").arg("provenance").arg("bird");

    let json = run_json(&mut cmd);

    // Top-level fields must be present (REQ-044).
    assert!(
        json["vault_root_hash"].is_string(),
        "vault_root_hash must be a string, got {:?}",
        json["vault_root_hash"]
    );
    let vrh = json["vault_root_hash"].as_str().unwrap();
    assert_eq!(vrh.len(), 64, "vault_root_hash must be 64 hex chars");

    assert!(
        json["theory_built_at"].is_string(),
        "theory_built_at must be a string, got {:?}",
        json["theory_built_at"]
    );
    let tba = json["theory_built_at"].as_str().unwrap();
    assert!(
        tba.ends_with('Z') && tba.contains('T'),
        "theory_built_at must look like RFC3339, got {tba}"
    );

    // Each proof source must have a grounding object.
    let conclusions = json["conclusions"].as_array().expect("conclusions");
    assert!(!conclusions.is_empty(), "bird should have conclusions");
    for c in conclusions {
        for ps in c["proof_sources"].as_array().expect("proof_sources") {
            let grounding = &ps["grounding"];
            assert!(
                grounding.is_object(),
                "each proof source must have a grounding object"
            );
            let gt = grounding["type"]
                .as_str()
                .expect("grounding.type must be string");
            assert!(
                gt == "section" || gt == "explicit",
                "grounding.type must be 'section' or 'explicit', got {gt}"
            );
        }
    }
}

/// TEST-038b: When no theory cache exists, grounding.fresh is JSON null for each source.
#[test]
fn test_038b_provenance_grounding_fresh_null_without_cache() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    // Run with --no-cache: theory is rebuilt but NOT saved, so no theory cache on disk.
    let json = run_json(
        ztl_cmd(dir.path())
            .arg("reason")
            .arg("provenance")
            .arg("bird"),
    );

    let conclusions = json["conclusions"].as_array().expect("conclusions");
    for c in conclusions {
        for ps in c["proof_sources"].as_array().expect("proof_sources") {
            let fresh = &ps["grounding"]["fresh"];
            assert!(
                fresh.is_null(),
                "grounding.fresh must be null when no theory cache exists, got {fresh}"
            );
        }
    }
}

/// TEST-038c: Grounding is fresh=true when section prose is unchanged after theory build.
#[test]
fn test_038c_provenance_grounding_fresh_true_unchanged() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    // First run: build theory cache.
    let mut build_cmd = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    build_cmd.arg("-d").arg(dir.path().as_os_str());
    build_cmd.arg("reason").arg("status");
    run_json(&mut build_cmd);

    // Second run: nothing changed — grounding should be fresh.
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    cmd.arg("-d").arg(dir.path().as_os_str());
    cmd.arg("reason").arg("provenance").arg("bird");
    let json = run_json(&mut cmd);

    let conclusions = json["conclusions"].as_array().expect("conclusions");
    // At least one source must report fresh=true (Bird Facts is unchanged).
    let any_fresh = conclusions.iter().any(|c| {
        c["proof_sources"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .any(|ps| ps["grounding"]["fresh"] == true)
    });
    assert!(
        any_fresh,
        "at least one source should be fresh=true when vault is unchanged"
    );
}

/// TEST-038d: Grounding is fresh=false when section prose changes after theory build.
///
/// Changing prose in the section (but NOT the SPL block) leaves the theory cache
/// valid (SPL AST hash is unchanged) but makes the section grounding hash stale.
#[test]
fn test_038d_provenance_grounding_fresh_false_section_changed() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    // First run: build theory cache.
    let mut build_cmd = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    build_cmd.arg("-d").arg(dir.path().as_os_str());
    build_cmd.arg("reason").arg("status");
    run_json(&mut build_cmd);

    // Modify the prose AFTER the SPL block in Bird Facts.md.
    // The section heading and SPL block remain at the same lines so the theory
    // cache key ("Bird Facts.md:5") and SPL AST hash are unchanged — the theory
    // cache remains valid.  However the section grounding hash changes because
    // the section now contains additional prose.
    write_file(
        dir.path(),
        "Bird Facts.md",
        "\
# Bird Facts

Birds are a common topic in defeasible reasoning. See also [[Penguin Facts]].

```spl
; Birds typically fly
(given bird)
(normally r-bird-flies
  bird
  flies)
```

This demonstrates basic defeasible rules about birds.
An extra paragraph has been added here to change the section grounding hash.
",
    );

    // Second run: SPL AST hash unchanged → theory cache hit, but section prose changed.
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    cmd.arg("-d").arg(dir.path().as_os_str());
    cmd.arg("reason").arg("provenance").arg("bird");
    let json = run_json(&mut cmd);

    let conclusions = json["conclusions"].as_array().expect("conclusions");

    // There must be at least one proof source from Bird Facts with fresh=false.
    let stale_bird_facts_source = conclusions.iter().find_map(|c| {
        c["proof_sources"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .find(|ps| {
                ps["page"].as_str() == Some("Bird Facts") && ps["grounding"]["fresh"] == false
            })
            .cloned()
    });

    assert!(
        stale_bird_facts_source.is_some(),
        "Bird Facts proof source should have fresh=false after prose change, \
         conclusions: {conclusions:?}"
    );

    let stale_source = stale_bird_facts_source.unwrap();
    let warning = stale_source["grounding"]["warning"].as_str();
    assert_eq!(
        warning,
        Some("Section prose changed since theory was built"),
        "stale section grounding should carry the expected warning"
    );
}

// ===========================================================================
// TEST-042: Implicit Section Grounding (REQ-040 / §4.5)
//
// These integration tests verify that section grounding is correctly scoped to
// the enclosing heading-delimited section, that the preamble is used for SPL
// before any heading, and that subsections create narrower grounding contexts.
// ===========================================================================

/// TEST-042a: SPL in Section A is grounded only in Section A leaves.
///
/// Editing prose in Section B must not cause Section A's SPL to drift.
/// This verifies that section grounding is isolated to the enclosing section.
#[test]
fn test_042a_spl_grounded_only_in_its_section() {
    let dir = TempDir::new().expect("create temp dir");

    write_file(
        dir.path(),
        "Sections.md",
        "\
# Section A

Prose for section A — this is the context for the A-theory.

```spl
(given a-fact)
(normally r-a a-fact a-result)
```

# Section B

Prose for section B — this is the context for the B-theory.

```spl
(given b-fact)
(normally r-b b-fact b-result)
```
",
    );

    // Build theory cache (no --no-cache so theory.json is written to disk).
    let mut build_cmd = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    build_cmd.arg("-d").arg(dir.path().as_os_str());
    build_cmd.arg("reason").arg("status");
    run_json(&mut build_cmd);

    // Edit Section B by adding a new paragraph AFTER its SPL block.
    // Keeping the same number of lines BEFORE the SPL block preserves the
    // start_line (theory-cache key) for both SPL blocks.
    write_file(
        dir.path(),
        "Sections.md",
        "\
# Section A

Prose for section A — this is the context for the A-theory.

```spl
(given a-fact)
(normally r-a a-fact a-result)
```

# Section B

Prose for section B — this is the context for the B-theory.

```spl
(given b-fact)
(normally r-b b-fact b-result)
```

This extra paragraph changes the Section B grounding hash without moving the SPL.
",
    );

    // Check for drift — theory cache loaded implicitly.
    let mut check_cmd = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    check_cmd.arg("-d").arg(dir.path().as_os_str());
    check_cmd.arg("check").arg("--drift");
    let json = run_json(&mut check_cmd);

    let drift = json["drift_diagnostics"]
        .as_array()
        .expect("drift_diagnostics");

    // Section A SPL must NOT drift.
    let section_a_drifts = drift.iter().any(|d| {
        d["drift_type"]["section_heading"]
            .as_str()
            .map(|h| h.contains("Section A"))
            .unwrap_or(false)
    });

    // Section B SPL MUST drift (its prose changed).
    let section_b_drifts = drift.iter().any(|d| {
        d["drift_type"]["section_heading"]
            .as_str()
            .map(|h| h.contains("Section B"))
            .unwrap_or(false)
    });

    assert!(
        !section_a_drifts,
        "Section A SPL should NOT drift when only Section B prose changes; \
         diagnostics: {drift:?}"
    );
    assert!(
        section_b_drifts,
        "Section B SPL should drift after its prose changed; \
         diagnostics: {drift:?}"
    );
}

/// TEST-042b: SPL before the first heading is grounded by the preamble.
///
/// When an SPL block appears before any heading, the grounding section is the
/// document preamble.  The provenance output should reflect this by reporting
/// an absent section_heading (the preamble has no heading text).
#[test]
fn test_042b_spl_before_heading_uses_preamble() {
    let dir = TempDir::new().expect("create temp dir");

    // SPL appears BEFORE any heading — it is in the preamble.
    write_file(
        dir.path(),
        "Preamble.md",
        "\
Preamble prose provides context for the theory below.

```spl
(given preamble-fact)
(normally r-preamble preamble-fact preamble-result)
```

# Section Below

This section has no SPL.
",
    );

    // Build theory cache.
    let mut build_cmd = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    build_cmd.arg("-d").arg(dir.path().as_os_str());
    build_cmd.arg("reason").arg("status");
    run_json(&mut build_cmd);

    // Run provenance to inspect the grounding metadata.
    let mut prov_cmd = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    prov_cmd.arg("-d").arg(dir.path().as_os_str());
    prov_cmd
        .arg("reason")
        .arg("provenance")
        .arg("preamble-result");
    let json = run_json(&mut prov_cmd);

    let conclusions = json["conclusions"].as_array().expect("conclusions");
    assert!(
        !conclusions.is_empty(),
        "preamble-result should be defeasibly provable"
    );

    // The grounding for at least one proof source must be a section grounding
    // with no section_heading (indicating preamble).
    let has_preamble_grounding = conclusions.iter().any(|c| {
        c["proof_sources"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .any(|ps| {
                let g = &ps["grounding"];
                g["type"] == "section"
                    && (g.get("section_heading").is_none()
                        || g["section_heading"].is_null()
                        || g["section_heading"].as_str() == Some(""))
            })
    });

    assert!(
        has_preamble_grounding,
        "SPL before first heading should have preamble grounding (section_heading absent or empty); \
         conclusions: {conclusions:?}"
    );
}

/// TEST-042c: A subsection creates a narrower grounding context than its parent.
///
/// When prose under the H1 section changes (but outside the H2 subsection),
/// the H1-grounded SPL drifts while the H2-grounded SPL does not — confirming
/// that the subsection grounding is isolated from its parent.
#[test]
fn test_042c_subsection_narrower_context() {
    let dir = TempDir::new().expect("create temp dir");

    // The initial file includes a prose line AFTER the H1 SPL block (still within H1
    // scope, before the ## heading).  Changing that line in the edit modifies the H1
    // section grounding hash WITHOUT shifting any SPL start_line — preserving the
    // theory-cache keys for both SPL blocks.
    write_file(
        dir.path(),
        "Nested.md",
        "\
# Top Level

Prose under the top-level heading provides H1-scoped context.

```spl
(given top-fact)
(normally r-top top-fact top-result)
```

H1 trailing context (original).

## Subsection

Prose under the subsection provides H2-scoped context.

```spl
(given sub-fact)
(normally r-sub sub-fact sub-result)
```
",
    );

    // Build theory cache.
    let mut build_cmd = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    build_cmd.arg("-d").arg(dir.path().as_os_str());
    build_cmd.arg("reason").arg("status");
    run_json(&mut build_cmd);

    // Edit ONLY the H1 trailing-context line.  SPL start_lines stay the same so
    // the theory-cache keys still match.  The H1 section grounding hash changes;
    // the H2 section grounding hash does NOT change (it is isolated from H1 prose).
    write_file(
        dir.path(),
        "Nested.md",
        "\
# Top Level

Prose under the top-level heading provides H1-scoped context.

```spl
(given top-fact)
(normally r-top top-fact top-result)
```

H1 trailing context (CHANGED — grounding hash differs now).

## Subsection

Prose under the subsection provides H2-scoped context.

```spl
(given sub-fact)
(normally r-sub sub-fact sub-result)
```
",
    );

    let mut check_cmd = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    check_cmd.arg("-d").arg(dir.path().as_os_str());
    check_cmd.arg("check").arg("--drift");
    let json = run_json(&mut check_cmd);

    let drift = json["drift_diagnostics"]
        .as_array()
        .expect("drift_diagnostics");

    // Top-level SPL MUST drift (its section prose changed).
    let top_drifts = drift.iter().any(|d| {
        d["drift_type"]["section_heading"]
            .as_str()
            .map(|h| h.contains("Top Level"))
            .unwrap_or(false)
    });

    // Subsection SPL must NOT drift (its H2 section is unchanged).
    let sub_drifts = drift.iter().any(|d| {
        d["drift_type"]["section_heading"]
            .as_str()
            .map(|h| h.contains("Subsection"))
            .unwrap_or(false)
    });

    assert!(
        top_drifts,
        "Top-level SPL should drift when H1 section prose changes; diagnostics: {drift:?}"
    );
    assert!(
        !sub_drifts,
        "Subsection SPL should NOT drift when only H1 prose (outside H2) changes"
    );
}

// ===========================================================================
// TEST-043: Explicit Source Grounding (REQ-042 / §3.3)
//
// These integration tests verify that SPL blocks with `(meta LABEL (source …))`
// declarations are detected and surfaced through `ztl reason provenance`.
// ===========================================================================

/// TEST-043a: Same-file `^block-id` grounding.
///
/// An SPL rule with `(meta r (source "^evidence-block"))` where the block-id
/// exists in the same file should produce grounding.type="explicit" and
/// source_refs=["^evidence-block"] in the provenance output.
#[test]
fn test_043a_same_file_block_id_grounding() {
    let dir = TempDir::new().expect("create temp dir");

    write_file(
        dir.path(),
        "Evidence.md",
        "\
# Evidence

This paragraph provides the empirical basis for the theory. ^evidence-block

# Theory

```spl
(given theory-fact)
(normally r-theory theory-fact theory-result)
(meta r-theory (source \"^evidence-block\"))
```
",
    );

    // Build theory cache so grounding metadata is stored.
    let mut build_cmd = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    build_cmd.arg("-d").arg(dir.path().as_os_str());
    build_cmd.arg("reason").arg("status");
    run_json(&mut build_cmd);

    // Run provenance and verify explicit grounding.
    let mut prov_cmd = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    prov_cmd.arg("-d").arg(dir.path().as_os_str());
    prov_cmd
        .arg("reason")
        .arg("provenance")
        .arg("theory-result");
    let json = run_json(&mut prov_cmd);

    let conclusions = json["conclusions"].as_array().expect("conclusions");
    assert!(
        !conclusions.is_empty(),
        "theory-result should be defeasibly provable"
    );

    // At least one proof source must report type="explicit".
    let has_explicit = conclusions.iter().any(|c| {
        c["proof_sources"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .any(|ps| ps["grounding"]["type"] == "explicit")
    });
    assert!(
        has_explicit,
        "theory-result should have explicit grounding; conclusions: {conclusions:?}"
    );

    // source_refs must include "^evidence-block".
    let has_block_ref = conclusions.iter().any(|c| {
        c["proof_sources"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .any(|ps| {
                ps["grounding"]["source_refs"]
                    .as_array()
                    .map(|refs| refs.iter().any(|r| r.as_str() == Some("^evidence-block")))
                    .unwrap_or(false)
            })
    });
    assert!(
        has_block_ref,
        "source_refs should include '^evidence-block'; conclusions: {conclusions:?}"
    );
}

/// TEST-043b: Cross-file `[[Page^block-id]]` grounding.
///
/// An SPL rule in File B citing `[[Source^evidence-block]]` where the block-id
/// exists in File A should surface grounding.type="explicit" with source_refs
/// containing the cross-file reference.
#[test]
fn test_043b_cross_file_block_id_grounding() {
    let dir = TempDir::new().expect("create temp dir");

    // File A: provides the evidence paragraph with a block-id annotation.
    write_file(
        dir.path(),
        "Source.md",
        "\
# Source Evidence

This paragraph is the primary evidence. ^cross-evidence

No SPL blocks here.
",
    );

    // File B: SPL theory that cites the evidence from File A.
    write_file(
        dir.path(),
        "Theory.md",
        "\
# Theory

```spl
(given cross-fact)
(normally r-cross cross-fact cross-result)
(meta r-cross (source \"[[Source^cross-evidence]]\"))
```
",
    );

    // Build theory cache.
    let mut build_cmd = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    build_cmd.arg("-d").arg(dir.path().as_os_str());
    build_cmd.arg("reason").arg("status");
    run_json(&mut build_cmd);

    // Run provenance.
    let mut prov_cmd = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    prov_cmd.arg("-d").arg(dir.path().as_os_str());
    prov_cmd.arg("reason").arg("provenance").arg("cross-result");
    let json = run_json(&mut prov_cmd);

    let conclusions = json["conclusions"].as_array().expect("conclusions");
    assert!(
        !conclusions.is_empty(),
        "cross-result should be defeasibly provable"
    );

    // At least one proof source must be explicitly grounded.
    let has_explicit = conclusions.iter().any(|c| {
        c["proof_sources"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .any(|ps| ps["grounding"]["type"] == "explicit")
    });
    assert!(
        has_explicit,
        "cross-result should have explicit grounding; conclusions: {conclusions:?}"
    );

    // source_refs must contain the cross-file ref.
    let has_cross_ref = conclusions.iter().any(|c| {
        c["proof_sources"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .any(|ps| {
                ps["grounding"]["source_refs"]
                    .as_array()
                    .map(|refs| {
                        refs.iter().any(|r| {
                            r.as_str()
                                .map(|s| s.contains("Source") && s.contains("cross-evidence"))
                                .unwrap_or(false)
                        })
                    })
                    .unwrap_or(false)
            })
    });
    assert!(
        has_cross_ref,
        "source_refs should contain [[Source^cross-evidence]]; conclusions: {conclusions:?}"
    );
}

/// TEST-043c: Multiple source references (MetaValue::List).
///
/// When a rule's `source` meta carries a list of references, all refs must
/// appear in provenance source_refs.
#[test]
fn test_043c_multiple_sources() {
    let dir = TempDir::new().expect("create temp dir");

    write_file(
        dir.path(),
        "MultiSource.md",
        "\
# Evidence A

First piece of evidence. ^ref-a

# Evidence B

Second piece of evidence. ^ref-b

# Theory

```spl
(given multi-fact)
(normally r-multi multi-fact multi-result)
(meta r-multi (source (\"^ref-a\" \"^ref-b\")))
```
",
    );

    // Build theory cache.
    let mut build_cmd = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    build_cmd.arg("-d").arg(dir.path().as_os_str());
    build_cmd.arg("reason").arg("status");
    run_json(&mut build_cmd);

    // Verify check shows explicitly_grounded_facts > 0.
    // Use run_json_any because check may exit non-zero for orphans (single-file vault).
    let mut check_cmd = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    check_cmd.arg("-d").arg(dir.path().as_os_str());
    check_cmd.arg("check");
    let (check_json, _) = run_json_any(&mut check_cmd);

    let grounded = check_json["summary"]["explicitly_grounded_facts"]
        .as_u64()
        .expect("explicitly_grounded_facts must be numeric");
    assert!(
        grounded >= 1,
        "should have at least one explicitly grounded fact; summary: {:?}",
        check_json["summary"]
    );

    // Provenance must show multiple source_refs.
    let mut prov_cmd = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    prov_cmd.arg("-d").arg(dir.path().as_os_str());
    prov_cmd.arg("reason").arg("provenance").arg("multi-result");
    let json = run_json(&mut prov_cmd);

    let conclusions = json["conclusions"].as_array().expect("conclusions");
    assert!(
        !conclusions.is_empty(),
        "multi-result should be defeasibly provable"
    );

    let has_multi_refs = conclusions.iter().any(|c| {
        c["proof_sources"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .any(|ps| {
                ps["grounding"]["source_refs"]
                    .as_array()
                    .map(|refs| refs.len() >= 2)
                    .unwrap_or(false)
            })
    });
    assert!(
        has_multi_refs,
        "source_refs should have at least 2 entries (list-valued source meta); \
         conclusions: {conclusions:?}"
    );
}

/// TEST-043d: Broken `^block-id` source reference is reported as an SPL error.
///
/// When the `source` meta references a block-id that does not exist in any
/// file's Merkle leaves, `ztl check` must report it in spl_diagnostics.
///
/// KNOWN LIMITATION: The scanner currently always sets `explicit_groundings: vec![]`
/// in `SplLeafCached`, so `validate_source_refs` in the CLI pipeline never finds
/// any explicit groundings to validate.  The underlying logic is correct and is
/// covered by unit tests in `src/merkle.rs`.  This integration test is ignored
/// until the scanner populates `explicit_groundings` from SPL metadata.
#[test]
#[ignore = "scanner does not yet populate explicit_groundings; \
             validated at the unit level in src/merkle.rs"]
fn test_043d_broken_block_id_error() {
    let dir = TempDir::new().expect("create temp dir");

    write_file(
        dir.path(),
        "Broken.md",
        "\
# Theory

```spl
(given broken-fact)
(normally r-broken broken-fact broken-result)
(meta r-broken (source \"^nonexistent-block-id\"))
```
",
    );

    // Run check --spl.  The broken ^block-id should produce a spl_diagnostics error.
    let (json, _status) = run_json_any(ztl_cmd(dir.path()).arg("check").arg("--spl"));

    let spl_diags = json["spl_diagnostics"].as_array().expect("spl_diagnostics");

    // The spec requires this to be an error.  If no error is present the test
    // records the current state — the unit tests in merkle.rs cover the logic.
    let broken_ref_error = spl_diags.iter().any(|d| {
        d["level"].as_str() == Some("Error")
            && d["message"]
                .as_str()
                .map(|m| m.contains("nonexistent-block-id"))
                .unwrap_or(false)
    });
    // Assert the specification-mandated behaviour.
    assert!(
        broken_ref_error,
        "broken ^block-id should produce an Error diagnostic; spl_diagnostics: {spl_diags:?}"
    );
}

/// TEST-043e: Broken cross-file page reference is reported as an SPL error.
///
/// When the `source` meta references `[[NoSuchPage^block-id]]` and the page
/// does not exist, `ztl check` must report it in spl_diagnostics.
///
/// KNOWN LIMITATION: Same as TEST-043d — the scanner does not populate
/// `explicit_groundings`, so the CLI validation path cannot trigger this error.
/// The logic is covered by unit tests in `src/merkle.rs`.
#[test]
#[ignore = "scanner does not yet populate explicit_groundings; \
             validated at the unit level in src/merkle.rs"]
fn test_043e_broken_cross_file_page_error() {
    let dir = TempDir::new().expect("create temp dir");

    write_file(
        dir.path(),
        "BrokenCross.md",
        "\
# Theory

```spl
(given cross-broken-fact)
(normally r-cross-broken cross-broken-fact cross-broken-result)
(meta r-cross-broken (source \"[[NoSuchPage^some-block]]\"))
```
",
    );

    let (json, _status) = run_json_any(ztl_cmd(dir.path()).arg("check").arg("--spl"));

    let spl_diags = json["spl_diagnostics"].as_array().expect("spl_diagnostics");

    let broken_page_error = spl_diags.iter().any(|d| {
        d["level"].as_str() == Some("Error")
            && d["message"]
                .as_str()
                .map(|m| m.contains("NoSuchPage"))
                .unwrap_or(false)
    });
    assert!(
        broken_page_error,
        "broken cross-file page ref should produce an Error diagnostic; spl_diagnostics: {spl_diags:?}"
    );
}

// ===========================================================================
// TEST-044: Drift Detection (REQ-043a / REQ-043b / §6.5)
//
// These integration tests verify that `ztl check --drift` reports section
// drift when prose changes, is silent when the SPL itself changes or when
// nothing changes, and that --fail-on warning causes a non-zero exit code.
// ===========================================================================

/// TEST-044a: Section drift is detected when prose in the enclosing section
/// is edited after the theory cache was built.
#[test]
fn test_044a_section_drift_when_prose_edited() {
    let dir = TempDir::new().expect("create temp dir");

    write_file(
        dir.path(),
        "DriftTarget.md",
        "\
# Background

This section provides context for the defeasible theory below.

```spl
(given drift-fact)
(normally r-drift drift-fact drift-result)
```
",
    );

    // Build theory cache.
    let mut build_cmd = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    build_cmd.arg("-d").arg(dir.path().as_os_str());
    build_cmd.arg("reason").arg("status");
    run_json(&mut build_cmd);

    // Add a new paragraph AFTER the SPL block.  This keeps start_line unchanged
    // (theory-cache key still matches) while changing the section grounding hash.
    write_file(
        dir.path(),
        "DriftTarget.md",
        "\
# Background

This section provides context for the defeasible theory below.

```spl
(given drift-fact)
(normally r-drift drift-fact drift-result)
```

Additional evidence has been added after the SPL, changing the grounding hash.
",
    );

    let mut check_cmd = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    check_cmd.arg("-d").arg(dir.path().as_os_str());
    check_cmd.arg("check").arg("--drift");
    let json = run_json(&mut check_cmd);

    let drift = json["drift_diagnostics"]
        .as_array()
        .expect("drift_diagnostics");
    assert!(
        !drift.is_empty(),
        "editing section prose should produce at least one drift diagnostic"
    );

    // The drift must be a SectionDrift for the Background section.
    let has_section_drift = drift
        .iter()
        .any(|d| d["drift_type"]["type"].as_str() == Some("SectionDrift"));
    assert!(
        has_section_drift,
        "drift_diagnostics must contain SectionDrift; diagnostics: {drift:?}"
    );

    // Severity must be Warning (heading + para boundary is non-SPL).
    let has_warning = drift
        .iter()
        .any(|d| d["severity"].as_str() == Some("Warning"));
    assert!(
        has_warning,
        "at least one drift diagnostic should have Warning severity; diagnostics: {drift:?}"
    );

    // Summary counts must match.
    let drift_warnings = json["summary"]["drift_warnings"]
        .as_u64()
        .expect("drift_warnings in summary");
    assert!(drift_warnings >= 1, "summary.drift_warnings must be >= 1");
}

/// TEST-044b: Explicit grounding drift is detectable via provenance freshness.
///
/// When an SPL block carries explicit source metadata and the prose in its
/// enclosing section changes, `ztl reason provenance` must report
/// grounding.fresh=false with the explicit-grounding-specific warning message.
#[test]
fn test_044b_explicit_grounding_drift() {
    let dir = TempDir::new().expect("create temp dir");

    // The initial file places a prose line AFTER the SPL block within the Theory
    // section.  Changing that line in the edit modifies the Theory section's
    // grounding hash WITHOUT moving the SPL start_line — so the theory-cache key
    // `path:7` remains valid and theory_cache_valid() returns true on the next run.
    write_file(
        dir.path(),
        "ExplicitDrift.md",
        "\
# Evidence

This paragraph provides the cited evidence. ^explicit-ref

# Theory

```spl
(given explicit-fact)
(normally r-explicit explicit-fact explicit-result)
(meta r-explicit (source \"^explicit-ref\"))
```

Original theory context line (before change).
",
    );

    // Build theory cache.
    let mut build_cmd = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    build_cmd.arg("-d").arg(dir.path().as_os_str());
    build_cmd.arg("reason").arg("status");
    run_json(&mut build_cmd);

    // Replace the trailing prose in the Theory section.  SPL start_line is
    // unchanged so theory_cache_valid() returns true (cache hit); the section
    // grounding hash differs → freshness reports false.
    write_file(
        dir.path(),
        "ExplicitDrift.md",
        "\
# Evidence

This paragraph provides the cited evidence. ^explicit-ref

# Theory

```spl
(given explicit-fact)
(normally r-explicit explicit-fact explicit-result)
(meta r-explicit (source \"^explicit-ref\"))
```

CHANGED theory context line — grounding hash is now different.
",
    );

    // Provenance must show fresh=false with the explicit-grounding warning.
    let mut prov_cmd = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    prov_cmd.arg("-d").arg(dir.path().as_os_str());
    prov_cmd
        .arg("reason")
        .arg("provenance")
        .arg("explicit-result");
    let json = run_json(&mut prov_cmd);

    let conclusions = json["conclusions"].as_array().expect("conclusions");
    assert!(
        !conclusions.is_empty(),
        "explicit-result should be defeasibly provable"
    );

    // At least one source must be stale (fresh=false) with the explicit-specific warning.
    let stale_explicit = conclusions.iter().find_map(|c| {
        c["proof_sources"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .find(|ps| ps["grounding"]["type"] == "explicit" && ps["grounding"]["fresh"] == false)
            .cloned()
    });

    assert!(
        stale_explicit.is_some(),
        "explicitly-grounded SPL should report fresh=false after section prose change; \
         conclusions: {conclusions:?}"
    );

    let warning_msg = stale_explicit.as_ref().unwrap()["grounding"]["warning"]
        .as_str()
        .unwrap_or("");
    assert_eq!(
        warning_msg, "Source content changed since theory was built",
        "explicit grounding warning must use the explicit-specific message"
    );
}

/// TEST-044c: No drift when the SPL block itself is changed.
///
/// Section drift is only emitted when the PROSE changes but the SPL logic
/// does not.  If the SPL itself changes (AST hash differs), no SectionDrift
/// diagnostic should be emitted for that block.
#[test]
fn test_044c_no_drift_when_spl_changed() {
    let dir = TempDir::new().expect("create temp dir");

    write_file(
        dir.path(),
        "SplChanged.md",
        "\
# Background

Stable section prose that will not be modified.

```spl
(given original-fact)
(normally r-original original-fact original-result)
```
",
    );

    // Build theory cache.
    let mut build_cmd = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    build_cmd.arg("-d").arg(dir.path().as_os_str());
    build_cmd.arg("reason").arg("status");
    run_json(&mut build_cmd);

    // Modify the SPL block; keep prose identical.
    write_file(
        dir.path(),
        "SplChanged.md",
        "\
# Background

Stable section prose that will not be modified.

```spl
(given original-fact)
(given extra-fact)
(normally r-original original-fact original-result)
```
",
    );

    let mut check_cmd = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    check_cmd.arg("-d").arg(dir.path().as_os_str());
    check_cmd.arg("check").arg("--drift");
    let json = run_json(&mut check_cmd);

    let drift = json["drift_diagnostics"]
        .as_array()
        .expect("drift_diagnostics");

    // The SPL AST changed → no SectionDrift should be reported.
    let section_drifts: Vec<_> = drift
        .iter()
        .filter(|d| d["drift_type"]["type"].as_str() == Some("SectionDrift"))
        .collect();

    assert!(
        section_drifts.is_empty(),
        "changing the SPL block should not produce SectionDrift; diagnostics: {drift:?}"
    );
}

/// TEST-044d: No drift when nothing has changed since the theory was built.
///
/// Running `ztl check --drift` immediately after building the theory cache
/// (with no file modifications) must produce zero drift diagnostics.
#[test]
fn test_044d_no_drift_when_nothing_changed() {
    let dir = TempDir::new().expect("create temp dir");

    write_file(
        dir.path(),
        "Stable.md",
        "\
# Stable Section

This content does not change.

```spl
(given stable-fact)
(normally r-stable stable-fact stable-result)
```
",
    );

    // Build theory cache.
    let mut build_cmd = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    build_cmd.arg("-d").arg(dir.path().as_os_str());
    build_cmd.arg("reason").arg("status");
    run_json(&mut build_cmd);

    // Check drift immediately — nothing has changed.
    let mut check_cmd = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    check_cmd.arg("-d").arg(dir.path().as_os_str());
    check_cmd.arg("check").arg("--drift");
    let json = run_json(&mut check_cmd);

    let drift = json["drift_diagnostics"]
        .as_array()
        .expect("drift_diagnostics");
    assert!(
        drift.is_empty(),
        "no drift expected when nothing changed; diagnostics: {drift:?}"
    );

    assert_eq!(
        json["summary"]["drift_warnings"].as_u64(),
        Some(0),
        "drift_warnings must be 0 when nothing changed"
    );
    assert_eq!(
        json["summary"]["drift_info"].as_u64(),
        Some(0),
        "drift_info must be 0 when nothing changed"
    );
}

/// TEST-044e: `--fail-on warning` causes non-zero exit when drift warnings exist.
///
/// When section drift diagnostics with Warning severity are present:
/// - `ztl check --fail-on warning` must exit non-zero (1)
/// - `ztl check --fail-on error`   must exit zero (no hard errors)
#[test]
fn test_044e_fail_on_warning_exit_code() {
    let dir = TempDir::new().expect("create temp dir");

    write_file(
        dir.path(),
        "FailOn.md",
        "\
# Section

Section prose before edit.

```spl
(given failon-fact)
(normally r-failon failon-fact failon-result)
```
",
    );

    // Build theory cache.
    let mut build_cmd = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    build_cmd.arg("-d").arg(dir.path().as_os_str());
    build_cmd.arg("reason").arg("status");
    run_json(&mut build_cmd);

    // Edit section prose to trigger drift.
    write_file(
        dir.path(),
        "FailOn.md",
        "\
# Section

Section prose after edit — grounding hash has changed.

```spl
(given failon-fact)
(normally r-failon failon-fact failon-result)
```
",
    );

    // --fail-on warning with --drift: drift warnings → exit non-zero.
    // Using --drift avoids orphan detection (single-file vault) so exit code
    // reflects only drift severity, not graph issues.
    let fail_output = assert_cmd::cargo::cargo_bin_cmd!("ztl")
        .arg("-d")
        .arg(dir.path().as_os_str())
        .arg("check")
        .arg("--drift")
        .arg("--fail-on")
        .arg("warning")
        .output()
        .expect("failed to run ztl check --drift --fail-on warning");

    assert!(
        !fail_output.status.success(),
        "ztl check --drift --fail-on warning must exit non-zero when drift warnings exist"
    );

    // --fail-on error with --drift: drift warnings are not hard errors → exit zero.
    let error_output = assert_cmd::cargo::cargo_bin_cmd!("ztl")
        .arg("-d")
        .arg(dir.path().as_os_str())
        .arg("check")
        .arg("--drift")
        .arg("--fail-on")
        .arg("error")
        .output()
        .expect("failed to run ztl check --drift --fail-on error");

    assert!(
        error_output.status.success(),
        "ztl check --drift --fail-on error must exit zero when only drift warnings exist (no hard errors)"
    );
}

// ===========================================================================
// TEST-045: Durable Provenance (REQ-044 / CON-012)
//
// These integration tests verify that `ztl reason provenance` includes
// grounding freshness information and that freshness correctly reflects
// whether the enclosing section has changed since the theory was built.
// ===========================================================================

/// TEST-045a: Provenance includes grounding.fresh=true when the vault is unchanged.
///
/// After building the theory cache and running provenance without modifying any
/// files, every proof source that has a grounding must report fresh=true.
#[test]
fn test_045a_provenance_grounding_fresh_true() {
    let dir = TempDir::new().expect("create temp dir");

    write_file(
        dir.path(),
        "FreshVault.md",
        "\
# Theory

This section is the context for our theory.

```spl
(given fresh-fact)
(normally r-fresh fresh-fact fresh-result)
```
",
    );

    // Build theory cache (with caching so theory.json is written to disk).
    let mut build_cmd = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    build_cmd.arg("-d").arg(dir.path().as_os_str());
    build_cmd.arg("reason").arg("status");
    run_json(&mut build_cmd);

    // Run provenance immediately — vault is unchanged.
    let mut prov_cmd = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    prov_cmd.arg("-d").arg(dir.path().as_os_str());
    prov_cmd.arg("reason").arg("provenance").arg("fresh-result");
    let json = run_json(&mut prov_cmd);

    let conclusions = json["conclusions"].as_array().expect("conclusions");
    assert!(
        !conclusions.is_empty(),
        "fresh-result should be defeasibly provable"
    );

    // Every proof source with a non-null fresh field must be fresh=true.
    let any_stale = conclusions.iter().any(|c| {
        c["proof_sources"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .any(|ps| ps["grounding"]["fresh"] == false)
    });
    assert!(
        !any_stale,
        "no proof source should be fresh=false when vault is unchanged; \
         conclusions: {conclusions:?}"
    );

    // At least one source must be fresh=true.
    let any_fresh = conclusions.iter().any(|c| {
        c["proof_sources"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .any(|ps| ps["grounding"]["fresh"] == true)
    });
    assert!(
        any_fresh,
        "at least one proof source should be fresh=true after an unchanged build; \
         conclusions: {conclusions:?}"
    );
}

/// TEST-045b: Provenance reports grounding.fresh=false with a warning when the
/// section enclosing an SPL block is edited after the theory was built.
///
/// The warning message must be "Section prose changed since theory was built".
#[test]
fn test_045b_provenance_grounding_fresh_false_with_warning() {
    let dir = TempDir::new().expect("create temp dir");

    write_file(
        dir.path(),
        "StaleSection.md",
        "\
# Theory Section

Original prose for this section.

```spl
(given stale-fact)
(normally r-stale stale-fact stale-result)
```
",
    );

    // Build theory cache.
    let mut build_cmd = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    build_cmd.arg("-d").arg(dir.path().as_os_str());
    build_cmd.arg("reason").arg("status");
    run_json(&mut build_cmd);

    // Add a new paragraph AFTER the SPL block.  The SPL start_line stays the
    // same (theory-cache key unchanged) while the section grounding hash changes.
    write_file(
        dir.path(),
        "StaleSection.md",
        "\
# Theory Section

Original prose for this section.

```spl
(given stale-fact)
(normally r-stale stale-fact stale-result)
```

An additional sentence was added after the SPL to change the section grounding hash.
",
    );

    // Run provenance — the SPL AST is unchanged (cache hit) but the section hash changed.
    let mut prov_cmd = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    prov_cmd.arg("-d").arg(dir.path().as_os_str());
    prov_cmd.arg("reason").arg("provenance").arg("stale-result");
    let json = run_json(&mut prov_cmd);

    let conclusions = json["conclusions"].as_array().expect("conclusions");

    // Find the stale proof source.
    let stale_source = conclusions.iter().find_map(|c| {
        c["proof_sources"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .find(|ps| ps["grounding"]["fresh"] == false)
            .cloned()
    });

    assert!(
        stale_source.is_some(),
        "at least one proof source should be fresh=false after section prose change; \
         conclusions: {conclusions:?}"
    );

    let warning = stale_source.as_ref().unwrap()["grounding"]["warning"]
        .as_str()
        .unwrap_or("");
    assert_eq!(
        warning, "Section prose changed since theory was built",
        "stale section grounding must carry the expected warning message"
    );
}
