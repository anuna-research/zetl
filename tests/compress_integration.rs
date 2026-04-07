//! Integration tests for AST-aware compact export (SPEC-026).

use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a zetl command with text (table) output format.
fn zetl_text(vault: &Path) -> Command {
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("zetl");
    cmd.arg("-d").arg(vault.as_os_str());
    cmd.arg("--no-cache");
    cmd.arg("-f").arg("table");
    cmd
}

/// Build a zetl command with JSON output format.
fn zetl_json(vault: &Path) -> Command {
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("zetl");
    cmd.arg("-d").arg(vault.as_os_str());
    cmd.arg("--no-cache");
    cmd.arg("-f").arg("json");
    cmd
}

fn write_file(root: &Path, relative: &str, content: &str) {
    let full = root.join(relative);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).expect("create parent dirs");
    }
    fs::write(&full, content).expect("write test file");
}

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

fn run_text(cmd: &mut Command) -> String {
    let output = cmd.output().expect("failed to execute zetl");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "zetl exited with non-zero status.\nstdout: {stdout}\nstderr: {stderr}",
    );
    stdout
}

fn setup_vault() -> TempDir {
    let tmp = TempDir::new().expect("create temp dir");
    write_file(
        tmp.path(),
        "Auth Design.md",
        "---\ntags: [auth, security]\naliases: [authn]\n---\n\n## Authentication Design\n\nWe decided to use Clerk because of pricing and developer experience.\nCheck [[OAuth Flow]] for the token exchange details.\nSee [[Session Management]] for stateful client handling.\n\n### Token Strategy\n\n- JWT for API\n- session cookie for web\n- refresh token rotation\n\n```rust\nfn issue_token(claims: &Claims) -> Result<String> {\n    // implementation\n}\n```\n\n### Open Questions\n\n- [ ] CSRF protection strategy\n- [x] provider selection\n\n> This is an important design note. It has multiple sentences for testing.\n",
    );

    write_file(
        tmp.path(),
        "OAuth Flow.md",
        "# OAuth Flow\n\nHandles [[Auth Design]] token exchange.\nUses PKCE for public clients.\n",
    );

    write_file(
        tmp.path(),
        "Session Management.md",
        "# Session Management\n\nManages sessions for [[Auth Design]].\nUses Redis for session storage.\n",
    );

    write_file(
        tmp.path(),
        "Unrelated.md",
        "# Unrelated\n\nThis page has no links to auth.\n",
    );

    tmp
}

// ---------------------------------------------------------------------------
// TEST-144: Single-Page Compression (text output)
// ---------------------------------------------------------------------------

#[test]
fn compact_single_page_text() {
    let vault = setup_vault();
    let mut cmd = zetl_text(vault.path());
    cmd.args(["export", "--compact", "Auth Design"]);
    let text = run_text(&mut cmd);

    // Headings preserved
    assert!(text.contains("## Authentication Design"), "heading missing");
    assert!(text.contains("### Token Strategy"), "sub-heading missing");

    // Wikilinks emitted as → target
    assert!(text.contains("→ OAuth Flow"), "wikilink to OAuth Flow missing");
    assert!(
        text.contains("→ Session Management"),
        "wikilink to Session Management missing"
    );

    // Code block compressed
    assert!(text.contains("⌜rust:"), "code block language missing");
    assert!(
        text.contains("fn issue_token"),
        "code block first line missing"
    );

    // List flattened
    assert!(text.contains("•"), "bullet missing");
    assert!(text.contains("JWT for API"), "list item missing");

    // Task list
    assert!(text.contains("☐"), "unchecked task marker missing");
    assert!(text.contains("☑"), "checked task marker missing");

    // Frontmatter
    assert!(
        text.contains("@ tags: auth, security"),
        "frontmatter tags missing"
    );
    assert!(
        text.contains("@ aliases: authn"),
        "frontmatter aliases missing"
    );

    // Blockquote (first sentence only)
    assert!(
        text.contains("> This is an important design note."),
        "blockquote first sentence missing"
    );
}

// ---------------------------------------------------------------------------
// TEST-145: Neighbourhood Compression
// ---------------------------------------------------------------------------

#[test]
fn compact_neighbourhood() {
    let vault = setup_vault();
    let mut cmd = zetl_text(vault.path());
    cmd.args(["export", "--compact", "Auth Design", "--neighbours"]);
    let text = run_text(&mut cmd);

    // Should have page delimiters for all 3 connected pages
    assert!(
        text.contains("=== Auth Design ==="),
        "Auth Design delimiter missing"
    );
    assert!(
        text.contains("=== OAuth Flow ==="),
        "OAuth Flow delimiter missing"
    );
    assert!(
        text.contains("=== Session Management ==="),
        "Session Management delimiter missing"
    );

    // Should NOT contain Unrelated (not a neighbour)
    assert!(
        !text.contains("=== Unrelated ==="),
        "Unrelated should not be in neighbourhood"
    );

    // Graph summary present
    assert!(text.contains("=== GRAPH ("), "graph summary missing");
    assert!(text.contains("→"), "forward link arrow in graph missing");
}

// ---------------------------------------------------------------------------
// TEST-146: Full Vault Compression
// ---------------------------------------------------------------------------

#[test]
fn compact_all() {
    let vault = setup_vault();
    let mut cmd = zetl_text(vault.path());
    cmd.args(["export", "--compact", "--all"]);
    let text = run_text(&mut cmd);

    // Should contain all 4 pages
    assert!(text.contains("=== Auth Design ==="), "Auth Design missing");
    assert!(text.contains("=== OAuth Flow ==="), "OAuth Flow missing");
    assert!(
        text.contains("=== Session Management ==="),
        "Session Management missing"
    );
    assert!(text.contains("=== Unrelated ==="), "Unrelated missing");

    // Graph summary with correct page count
    assert!(
        text.contains("=== GRAPH (4 pages,"),
        "graph summary count wrong"
    );
}

// ---------------------------------------------------------------------------
// TEST-148: Wikilink Preservation
// ---------------------------------------------------------------------------

#[test]
fn compact_wikilink_preservation() {
    let vault = TempDir::new().expect("temp dir");
    write_file(
        vault.path(),
        "Test.md",
        "Check [[OAuth Flow]] and [[Session Management|sessions]] for details.\n",
    );
    write_file(vault.path(), "OAuth Flow.md", "# OAuth Flow\n");
    write_file(
        vault.path(),
        "Session Management.md",
        "# Session Mgmt\n",
    );

    let mut cmd = zetl_text(vault.path());
    cmd.args(["export", "--compact", "Test"]);
    let text = run_text(&mut cmd);

    assert!(text.contains("→ OAuth Flow"), "OAuth Flow link missing");
    assert!(
        text.contains("→ Session Management"),
        "Session Management link missing"
    );
}

// ---------------------------------------------------------------------------
// TEST-149: Dead Link Marking
// ---------------------------------------------------------------------------

#[test]
fn compact_dead_link_marking() {
    let vault = TempDir::new().expect("temp dir");
    write_file(
        vault.path(),
        "Test.md",
        "See [[Nonexistent Page]] for info.\n",
    );

    let mut cmd = zetl_text(vault.path());
    cmd.args(["export", "--compact", "Test"]);
    let text = run_text(&mut cmd);

    assert!(
        text.contains("→ ✗ Nonexistent Page"),
        "dead link marker missing; got: {}",
        text
    );
}

// ---------------------------------------------------------------------------
// TEST-153: JSON Envelope
// ---------------------------------------------------------------------------

#[test]
fn compact_json_envelope() {
    let vault = setup_vault();
    let mut cmd = zetl_json(vault.path());
    cmd.args(["export", "--compact", "Auth Design"]);
    let json = run_json(&mut cmd);

    // Verify JSON structure per CON-037
    assert!(json["pages"].is_array(), "pages should be array");
    let page = &json["pages"][0];
    assert_eq!(page["name"], "Auth Design", "page name wrong");
    assert!(
        page["compressed"].is_string(),
        "compressed should be string"
    );
    assert!(
        !page["compressed"].as_str().unwrap().is_empty(),
        "compressed should not be empty"
    );
    assert!(
        page["token_estimate"].is_u64(),
        "token_estimate should be integer"
    );
    assert!(
        page["raw_token_estimate"].is_u64(),
        "raw_token_estimate should be integer"
    );
    assert!(
        json["total_token_estimate"].is_u64(),
        "total_token_estimate should be integer"
    );
}

// ---------------------------------------------------------------------------
// TEST-151: Determinism
// ---------------------------------------------------------------------------

#[test]
fn compact_determinism() {
    let vault = setup_vault();

    let run_once = || {
        let mut cmd = zetl_text(vault.path());
        cmd.args(["export", "--compact", "Auth Design"]);
        run_text(&mut cmd)
    };

    let a = run_once();
    let b = run_once();
    assert_eq!(a, b, "compression must be deterministic");
}
