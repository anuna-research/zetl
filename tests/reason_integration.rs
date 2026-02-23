//! Integration tests for the `zetl reason` command family.
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

/// Run the command (may fail) and parse stdout as JSON regardless of exit code.
fn run_json_any(cmd: &mut Command) -> (Value, ExitStatus) {
    let output = cmd.output().expect("failed to execute zetl");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("failed to parse JSON output: {e}\nraw stdout: {stdout}"));
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
        zetl_cmd(dir.path())
            .arg("reason")
            .arg("export")
            .arg("--format")
            .arg("json"),
    );

    // The single SPL block should produce 1 fact (alpha) and 1 rule (r1)
    let facts = json["facts"].as_array().expect("facts array");
    assert_eq!(facts.len(), 1, "should have exactly 1 fact from single block");
    assert_eq!(facts[0]["literal"].as_str(), Some("alpha"));
    assert_eq!(facts[0]["source_page"].as_str(), Some("Simple"));
    assert_eq!(facts[0]["source_file"].as_str(), Some("Simple.md"));

    let rules = json["rules"].as_array().expect("rules array");
    assert_eq!(rules.len(), 1, "should have exactly 1 rule from single block");
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
        zetl_cmd(dir.path())
            .arg("reason")
            .arg("export")
            .arg("--format")
            .arg("json"),
    );

    let facts = json["facts"].as_array().expect("facts array");
    let fact_literals: Vec<&str> = facts
        .iter()
        .map(|f| f["literal"].as_str().unwrap())
        .collect();

    // Both blocks should contribute facts
    assert!(fact_literals.contains(&"sunny"), "first block's sunny fact");
    assert!(fact_literals.contains(&"windy"), "second block's windy fact");
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
        zetl_cmd(dir.path())
            .arg("reason")
            .arg("export")
            .arg("--format")
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
    assert!(fact_literals.contains(&"sunny"), "sunny should be extracted");
    assert!(fact_literals.contains(&"windy"), "windy should be extracted");
}

/// TEST-026d: Standalone .spl files are correctly extracted.
/// Facts from standalone.spl should appear in the theory with correct provenance.
#[test]
fn test_026d_standalone_spl_file() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let json = run_json(
        zetl_cmd(dir.path())
            .arg("reason")
            .arg("export")
            .arg("--format")
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

    let json = run_json(zetl_cmd(dir.path()).arg("reason").arg("status"));

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

    let (json, status) = run_json_any(zetl_cmd(dir.path()).arg("reason").arg("status"));

    // Should exit non-zero (exit code 2 for parse errors)
    assert!(!status.success(), "should fail on parse errors");

    // Diagnostics should contain the parse error
    let diagnostics = json["diagnostics"].as_array().expect("diagnostics array");
    assert!(
        !diagnostics.is_empty(),
        "should have at least one diagnostic for parse error"
    );

    let has_parse_error = diagnostics
        .iter()
        .any(|d| d["message"].as_str().unwrap_or("").contains("SPL parse error"));
    assert!(has_parse_error, "should have SPL parse error diagnostic");
}

/// TEST-027c: Provenance metadata is correct for facts and rules.
#[test]
fn test_027c_provenance_metadata() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let json = run_json(
        zetl_cmd(dir.path())
            .arg("reason")
            .arg("export")
            .arg("--format")
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

    let json = run_json(zetl_cmd(dir.path()).arg("reason").arg("status"));

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
    assert!(bird_definite, "bird should be +D (definitively provable via strict rule)");
}

/// TEST-028b: Conclusion counts are correct.
#[test]
fn test_028b_conclusion_counts() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let json = run_json(zetl_cmd(dir.path()).arg("reason").arg("status"));

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
        zetl_cmd(dir.path())
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
        zetl_cmd(dir.path())
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
    assert!(!conclusions.is_empty(), "should have some fly-related conclusions");
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
        zetl_cmd(dir.path())
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
        zetl_cmd(dir.path())
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
        zetl_cmd(dir.path())
            .arg("reason")
            .arg("explain")
            .arg("nonexistent"),
    );

    assert!(!status.success(), "should fail for unknown literal");
    assert!(json["error"].as_str().is_some(), "should have error field");
    assert_eq!(json["literal"].as_str(), Some("nonexistent"));

    // Should have suggestions
    let suggestions = json["suggestions"].as_array().expect("suggestions array");
    assert!(
        !suggestions.is_empty(),
        "should suggest similar literals"
    );
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
        zetl_cmd(dir.path())
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
        zetl_cmd(dir.path())
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
        zetl_cmd(dir.path())
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
        zetl_cmd(dir.path())
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
    assert!(
        defeated,
        "r-bird-flies should be blocked by defeat"
    );
}

/// TEST-031b: Why-not for a provable literal exits with error and hint.
#[test]
fn test_031b_why_not_already_provable() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let (json, status) = run_json_any(
        zetl_cmd(dir.path())
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
        zetl_cmd(dir.path())
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
        zetl_cmd(dir.path())
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
        zetl_cmd(dir.path())
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
        zetl_cmd(dir.path())
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
        zetl_cmd(dir.path())
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

    let json = run_json(
        zetl_cmd(dir.path()).arg("reason").arg("conflicts"),
    );

    let conflicts = json["conflicts"].as_array().expect("conflicts array");
    assert_eq!(
        json["conflict_count"].as_u64(),
        Some(1),
        "should have exactly 1 conflict"
    );

    let conflict = &conflicts[0];
    assert_eq!(conflict["literal"].as_str(), Some("approved"));
    assert_eq!(
        conflict["positive_literal"].as_str(),
        Some("approved")
    );
    assert_eq!(
        conflict["negative_literal"].as_str(),
        Some("~approved")
    );
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

    let json = run_json(
        zetl_cmd(dir.path()).arg("reason").arg("conflicts"),
    );

    let conflicts = json["conflicts"].as_array().expect("conflicts array");
    assert!(
        conflicts.is_empty(),
        "resolved conflict should not appear"
    );
    assert_eq!(json["conflict_count"].as_u64(), Some(0));
}

/// TEST-033c: Conflicts with --suggest produces resolution suggestions.
#[test]
fn test_033c_conflict_suggestions() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let json = run_json(
        zetl_cmd(dir.path())
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
    let has_prefer_suggestion = suggestions.iter().any(|s| {
        s.as_str()
            .unwrap_or("")
            .contains("prefer")
    });
    assert!(
        has_prefer_suggestion,
        "should suggest superiority relation"
    );
}

/// TEST-033d: Conflicts with --fail-on-conflicts exits non-zero when conflicts exist.
#[test]
fn test_033d_fail_on_conflicts() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let (json, status) = run_json_any(
        zetl_cmd(dir.path())
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
        zetl_cmd(dir.path())
            .arg("reason")
            .arg("export")
            .arg("--format")
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
        zetl_cmd(dir.path())
            .arg("reason")
            .arg("export")
            .arg("--format")
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

    let output = zetl_cmd(dir.path())
        .arg("reason")
        .arg("export")
        .arg("--format")
        .arg("spl")
        .output()
        .expect("failed to execute zetl");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should have provenance comments
    assert!(stdout.contains("; --- From:"), "should have provenance comments");

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
        zetl_cmd(dir.path())
            .arg("reason")
            .arg("export")
            .arg("--format")
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
        zetl_cmd(dir.path())
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
    assert!(
        has_neg_flies,
        "Penguin Facts should contribute to ~flies"
    );
}

/// TEST-035b: Provenance command traces sources and cross-references.
#[test]
fn test_035b_provenance_trace() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let json = run_json(
        zetl_cmd(dir.path())
            .arg("reason")
            .arg("provenance")
            .arg("bird"),
    );

    assert_eq!(json["literal"].as_str(), Some("bird"));

    let conclusions = json["conclusions"]
        .as_array()
        .expect("conclusions array");
    assert!(!conclusions.is_empty());

    // Should have +D and +d entries
    let types: Vec<&str> = conclusions
        .iter()
        .map(|c| c["conclusion_type"].as_str().unwrap())
        .collect();
    assert!(types.contains(&"+D"), "should have +D conclusion");
    assert!(types.contains(&"+d"), "should have +d conclusion");

    // Source pages
    let source_pages = json["source_pages"]
        .as_array()
        .expect("source_pages array");
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
        zetl_cmd(dir.path())
            .arg("reason")
            .arg("provenance")
            .arg("~flies"),
    );

    assert_eq!(json["literal"].as_str(), Some("~flies"));

    // Source pages should include Penguin Facts
    let source_pages = json["source_pages"]
        .as_array()
        .expect("source_pages");
    let pages: Vec<&str> = source_pages
        .iter()
        .map(|p| p.as_str().unwrap())
        .collect();
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
        zetl_cmd(dir.path())
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

    let json = run_json(
        zetl_cmd(dir.path()).arg("check").arg("--spl"),
    );

    let spl_diags = json["spl_diagnostics"]
        .as_array()
        .expect("spl_diagnostics");
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
    let (json, status) = run_json_any(
        zetl_cmd(dir.path()).arg("check").arg("--spl"),
    );

    assert!(!status.success(), "check --spl should fail on parse errors");

    let spl_diags = json["spl_diagnostics"]
        .as_array()
        .expect("spl_diagnostics");
    assert!(
        !spl_diags.is_empty(),
        "should report SPL parse errors"
    );

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
    let mut cmd1 = Command::cargo_bin("zetl").expect("binary should be built");
    cmd1.arg("-d").arg(dir.path().as_os_str());
    cmd1.arg("reason").arg("status");
    let json1 = run_json(&mut cmd1);

    // Second run: should use cache
    let mut cmd2 = Command::cargo_bin("zetl").expect("binary should be built");
    cmd2.arg("-d").arg(dir.path().as_os_str());
    cmd2.arg("reason").arg("status");
    let json2 = run_json(&mut cmd2);

    // Conclusions should match
    assert_eq!(
        json1["summary"]["total"],
        json2["summary"]["total"],
        "cached and uncached should produce same total conclusions"
    );
    assert_eq!(
        json1["theory"]["facts"],
        json2["theory"]["facts"],
        "cached and uncached should produce same fact count"
    );
    assert_eq!(
        json1["theory"]["rules"],
        json2["theory"]["rules"],
        "cached and uncached should produce same rule count"
    );
}

/// TEST-037b: Modifying an SPL file invalidates the theory cache.
#[test]
fn test_037b_cache_invalidation() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    // First run: build cache (without --no-cache)
    let mut cmd1 = Command::cargo_bin("zetl").expect("binary should be built");
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
    let mut cmd2 = Command::cargo_bin("zetl").expect("binary should be built");
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
    let mut cmd1 = Command::cargo_bin("zetl").expect("binary should be built");
    cmd1.arg("-d").arg(dir.path().as_os_str());
    cmd1.arg("reason").arg("status");
    let json1 = run_json(&mut cmd1);

    // Second run with --no-cache: should produce identical results
    let json2 = run_json(zetl_cmd(dir.path()).arg("reason").arg("status"));

    assert_eq!(
        json1["summary"]["total"],
        json2["summary"]["total"],
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

    let json = run_json(zetl_cmd(dir.path()).arg("reason").arg("status"));

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

    let json = run_json(zetl_cmd(dir.path()).arg("reason").arg("status"));
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
    write_file(
        hyp_dir.path(),
        "hypothetical.spl",
        "(given new-evidence)\n",
    );

    let json = run_json(
        zetl_cmd(vault_dir.path())
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

    let (json, status) = run_json_any(
        zetl_cmd(dir.path()).arg("reason").arg("status"),
    );

    assert!(!status.success());
    assert!(json["error"].as_str().is_some());
}

/// Multi-file theory has correct source_file_count.
#[test]
fn test_source_file_count() {
    let dir = TempDir::new().expect("create temp dir");
    build_reason_vault(dir.path());

    let json = run_json(
        zetl_cmd(dir.path())
            .arg("reason")
            .arg("export")
            .arg("--format")
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
        zetl_cmd(dir.path())
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
        zetl_cmd(dir.path())
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
