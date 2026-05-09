//! `zetl feed validate` JSON Feed conformance regression coverage.
//!
//! The validator must reject feeds missing required fields per JSON
//! Feed v1.1 (top-level `title` + `items` array; per-item `id` and one
//! of `content_html`/`content_text`). Earlier behaviour accepted
//! `{"version": "..."}` as valid and silently ignored every other
//! requirement.

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;

fn run_validate(body: &str) -> assert_cmd::assert::Assert {
    cargo_bin_cmd!("zetl")
        .args(["feed", "validate", "--feed-format", "jsonfeed"])
        .write_stdin(body.to_string())
        .assert()
}

#[test]
fn jsonfeed_validator_rejects_version_only_body() {
    // The body the reviewer flagged: just a `version` key. Previously
    // exited 0; must now fail because `title` and `items` are missing.
    let body = r#"{"version":"https://jsonfeed.org/version/1.1"}"#;
    run_validate(body)
        .failure()
        .stderr(predicate::str::contains("title").or(predicate::str::contains("items")));
}

#[test]
fn jsonfeed_validator_rejects_missing_items_array() {
    let body = r#"{"version":"https://jsonfeed.org/version/1.1","title":"X"}"#;
    run_validate(body)
        .failure()
        .stderr(predicate::str::contains("items"));
}

#[test]
fn jsonfeed_validator_rejects_item_without_id() {
    let body = r#"{"version":"https://jsonfeed.org/version/1.1","title":"X","items":[{"content_text":"hi"}]}"#;
    run_validate(body)
        .failure()
        .stderr(predicate::str::contains("id"));
}

#[test]
fn jsonfeed_validator_rejects_item_without_content() {
    let body = r#"{"version":"https://jsonfeed.org/version/1.1","title":"X","items":[{"id":"1"}]}"#;
    run_validate(body).failure().stderr(
        predicate::str::contains("content_html").or(predicate::str::contains("content_text")),
    );
}

#[test]
fn jsonfeed_validator_accepts_minimal_valid_feed() {
    // version + title + one item with id + content_text. Should pass.
    let body = r#"{"version":"https://jsonfeed.org/version/1.1","title":"X","items":[{"id":"urn:1","url":"https://example.com/1","content_text":"hi"}]}"#;
    run_validate(body).success();
}
