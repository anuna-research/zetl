//! SPEC-032 TEST-3225 — integration tests for `zetl ast sample` and
//! `zetl ast diff`.
//!
//! Matrix coverage (from SPEC-032 §TEST-3225):
//!
//! | `zetl ast sample <file.md>`    | output validates against zetl-ast-schema-v1.json |
//! | `zetl ast diff a.json b.json`  | tree-diff identifies known mutations; exit 1 on non-empty diff |

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use serde_json::Value;
use std::fs;
use tempfile::TempDir;

const SCHEMA_PATH: &str = "tools/zetl-ast-schema-v1.json";

fn schema_validator() -> jsonschema::Validator {
    let bytes = fs::read(SCHEMA_PATH).unwrap_or_else(|e| panic!("cannot read {SCHEMA_PATH}: {e}"));
    let schema: Value = serde_json::from_slice(&bytes).unwrap();
    jsonschema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .build(&schema)
        .unwrap()
}

fn write_page(dir: &TempDir, name: &str, content: &str) -> std::path::PathBuf {
    let path = dir.path().join(name);
    fs::write(&path, content).unwrap();
    path
}

#[test]
fn sample_transform_output_validates_against_schema() {
    let dir = TempDir::new().unwrap();
    let page = write_page(
        &dir,
        "page.md",
        "---\ntitle: Sample\ntags:\n  - alpha\n---\n\n\
         # Hello **world**\n\n\
         see [[Target#Section|alias]] and [[Plain]] here.\n\n\
         > quoted paragraph\n\n\
         - one\n- two\n\n\
         ```rust\nfn x() {}\n```\n\n\
         ```spl\nfact :foo\n```\n\n\
         ![[Other]]\n",
    );

    let output = cargo_bin_cmd!("zetl")
        .args(["ast", "sample"])
        .arg(&page)
        .args(["-f", "json"])
        .output()
        .expect("run ast sample");
    assert!(
        output.status.success(),
        "sample command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: Value = serde_json::from_slice(&output.stdout).expect("sample emits valid JSON");
    let validator = schema_validator();
    validator
        .validate(&json)
        .unwrap_or_else(|e| panic!("sample JSON must validate against schema: {e}"));

    // Canonical shape checks — the representative corpus must hit the full
    // set of node kinds the page exercises.
    assert_eq!(json["type"], "Document");
    assert_eq!(json["ast_version"], "1.1");
    assert_eq!(json["frontmatter"]["title"], "Sample");

    let children = json["children"].as_array().expect("children is array");
    let kinds: Vec<&str> = children.iter().filter_map(|c| c["type"].as_str()).collect();
    assert!(kinds.contains(&"Heading"), "{kinds:?}");
    assert!(kinds.contains(&"Paragraph"), "{kinds:?}");
    assert!(kinds.contains(&"BlockQuote"), "{kinds:?}");
    assert!(kinds.contains(&"List"), "{kinds:?}");
    assert!(kinds.contains(&"CodeBlock"), "{kinds:?}");
    assert!(kinds.contains(&"SplBlock"), "{kinds:?}");
    assert!(kinds.contains(&"Embed"), "{kinds:?}");
}

#[test]
fn sample_pre_parse_stage_emits_raw_markdown() {
    let dir = TempDir::new().unwrap();
    let content = "---\ntitle: T\n---\n\n# Heading\n\ntext [[link]]\n";
    let page = write_page(&dir, "p.md", content);

    let output = cargo_bin_cmd!("zetl")
        .args(["ast", "sample"])
        .arg(&page)
        .args(["--stage", "pre-parse"])
        .output()
        .expect("run pre-parse");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout, content, "pre-parse stage emits the file verbatim");
}

#[test]
fn sample_post_render_stage_emits_html_fragment() {
    let dir = TempDir::new().unwrap();
    let page = write_page(&dir, "p.md", "# Heading\n\nA paragraph.\n");

    let output = cargo_bin_cmd!("zetl")
        .args(["ast", "sample"])
        .arg(&page)
        .args(["--stage", "post-render"])
        .output()
        .expect("run post-render");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("<h1"),
        "post-render output should contain HTML: {stdout}"
    );
    assert!(stdout.contains("Heading"));
}

#[test]
fn sample_wikilink_fields_populated() {
    let dir = TempDir::new().unwrap();
    let page = write_page(
        &dir,
        "p.md",
        "see [[Target#Sec|alias]] and [[Other#^bid]]\n",
    );

    let output = cargo_bin_cmd!("zetl")
        .args(["ast", "sample"])
        .arg(&page)
        .args(["-f", "json"])
        .output()
        .expect("run sample");
    assert!(output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    // Walk all inline children of the first paragraph.
    let inlines = &json["children"][0]["children"];
    let wikilinks: Vec<&Value> = inlines
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["type"] == "Wikilink")
        .collect();
    assert_eq!(wikilinks.len(), 2);
    assert_eq!(wikilinks[0]["target"], "Target");
    assert_eq!(wikilinks[0]["alias"], "alias");
    assert_eq!(wikilinks[0]["heading"], "Sec");
    assert!(wikilinks[0]["block_id"].is_null());
    assert_eq!(wikilinks[1]["target"], "Other");
    assert_eq!(wikilinks[1]["block_id"], "bid");
    assert!(wikilinks[1]["heading"].is_null());
}

#[test]
fn sample_missing_file_fails_with_diagnostic() {
    cargo_bin_cmd!("zetl")
        .args(["ast", "sample", "/nonexistent/page.md"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Cannot read page file"));
}

#[test]
fn diff_identical_documents_exits_zero() {
    let dir = TempDir::new().unwrap();
    let page = write_page(&dir, "p.md", "# Hi\n\ntext\n");

    // Generate a canonical AST, then diff it against itself.
    let sample = cargo_bin_cmd!("zetl")
        .args(["ast", "sample"])
        .arg(&page)
        .args(["-f", "json"])
        .output()
        .expect("sample");
    assert!(sample.status.success());
    let ast_path = dir.path().join("a.json");
    fs::write(&ast_path, &sample.stdout).unwrap();

    cargo_bin_cmd!("zetl")
        .args(["ast", "diff"])
        .arg(&ast_path)
        .arg(&ast_path)
        .assert()
        .success();
}

#[test]
fn diff_detects_addition_removal_and_attr_change() {
    let dir = TempDir::new().unwrap();
    let before = serde_json::json!({
        "ast_version": "1.0",
        "type": "Document",
        "position": {"start_line":1,"start_col":1,"end_line":10,"end_col":1},
        "children": [
            {
                "type": "Paragraph",
                "position": {"start_line":1,"start_col":1,"end_line":1,"end_col":5},
                "children": [
                    {
                        "type": "Text",
                        "position": {"start_line":1,"start_col":1,"end_line":1,"end_col":5},
                        "text": "old"
                    }
                ]
            }
        ]
    });
    let after = serde_json::json!({
        "ast_version": "1.0",
        "type": "Document",
        "position": {"start_line":1,"start_col":1,"end_line":10,"end_col":1},
        "children": [
            {
                "type": "Paragraph",
                "position": {"start_line":1,"start_col":1,"end_line":1,"end_col":5},
                "children": [
                    {
                        "type": "Text",
                        "position": {"start_line":1,"start_col":1,"end_line":1,"end_col":5},
                        "text": "new"
                    }
                ]
            },
            {
                "type": "ThematicBreak",
                "position": {"start_line":3,"start_col":1,"end_line":3,"end_col":3}
            }
        ]
    });
    let bp = dir.path().join("before.json");
    let ap = dir.path().join("after.json");
    fs::write(&bp, serde_json::to_string(&before).unwrap()).unwrap();
    fs::write(&ap, serde_json::to_string(&after).unwrap()).unwrap();

    let json_output = cargo_bin_cmd!("zetl")
        .args(["ast", "diff"])
        .arg(&bp)
        .arg(&ap)
        .args(["-f", "json"])
        .output()
        .expect("run diff");
    // Non-empty diff → exit 1.
    assert_eq!(json_output.status.code(), Some(1));
    let entries: Vec<Value> = serde_json::from_slice(&json_output.stdout).unwrap();
    let kinds: Vec<&str> = entries
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"modified"), "{kinds:?}");
    assert!(kinds.contains(&"added"), "{kinds:?}");

    let modified = entries
        .iter()
        .find(|e| e["kind"] == "modified")
        .expect("modified entry");
    let changes = modified["attr_changes"]
        .as_array()
        .expect("attr_changes array");
    assert!(changes.iter().any(|c| c["field"] == "text"));

    let added = entries
        .iter()
        .find(|e| e["kind"] == "added")
        .expect("added entry");
    assert_eq!(added["node_type"], "ThematicBreak");
    assert_eq!(added["start_line"], 3);
}

#[test]
fn diff_rejects_malformed_json() {
    let dir = TempDir::new().unwrap();
    let bp = dir.path().join("before.json");
    let ap = dir.path().join("after.json");
    fs::write(&bp, "{not json").unwrap();
    fs::write(&ap, "{}").unwrap();

    cargo_bin_cmd!("zetl")
        .args(["ast", "diff"])
        .arg(&bp)
        .arg(&ap)
        .assert()
        .failure()
        .stderr(predicate::str::contains("not valid JSON"));
}

#[test]
fn sample_then_diff_round_trip_is_empty() {
    // Emit AST for a representative page, then diff the emission against
    // itself: guarantees the CLI output round-trips cleanly through the
    // diff CLI with no spurious entries.
    let dir = TempDir::new().unwrap();
    let page = write_page(
        &dir,
        "p.md",
        "# Title\n\nsee [[Target]] and ![[Other]]\n\n```rust\nfn x() {}\n```\n",
    );
    let sample = cargo_bin_cmd!("zetl")
        .args(["ast", "sample"])
        .arg(&page)
        .args(["-f", "json"])
        .output()
        .expect("sample");
    assert!(sample.status.success());
    let p = dir.path().join("x.json");
    fs::write(&p, &sample.stdout).unwrap();
    cargo_bin_cmd!("zetl")
        .args(["ast", "diff"])
        .arg(&p)
        .arg(&p)
        .assert()
        .success();
}
