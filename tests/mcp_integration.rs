//! Integration tests for MCP server (SPEC-021).
//! Requires: cargo test --features mcp --test mcp_integration

#![cfg(feature = "mcp")]

use serde_json::{json, Value};
use std::time::Duration;
use tempfile::TempDir;

/// Create a temporary vault with a few interlinked pages.
fn setup_vault() -> TempDir {
    let dir = TempDir::new().unwrap();
    let root = dir.path();

    std::fs::write(
        root.join("README.md"),
        "# README\n\nWelcome. See [[Architecture]] for details.\n",
    )
    .unwrap();
    std::fs::write(
        root.join("Architecture.md"),
        "# Architecture\n\nLinks to [[README]] and [[API Design]].\n",
    )
    .unwrap();
    std::fs::write(
        root.join("API Design.md"),
        "# API Design\n\nThe API docs. See [[Architecture]].\n",
    )
    .unwrap();

    dir
}

/// Build the newline-delimited JSON payload for a sequence of JSON-RPC messages.
fn build_stdin(messages: &[Value]) -> String {
    messages
        .iter()
        .map(|m| serde_json::to_string(m).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

/// Parse newline-delimited JSON output, returning all parsed objects.
fn parse_ndjson(output: &str) -> Vec<Value> {
    output
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// Find a JSON-RPC response with the given id.
fn find_response(responses: &[Value], id: &Value) -> Option<Value> {
    responses.iter().find(|r| r.get("id") == Some(id)).cloned()
}

/// Standard initialize request.
fn initialize_request(id: u64) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {
                "name": "zetl-test",
                "version": "0.1.0"
            }
        }
    })
}

/// Initialized notification (no response expected).
fn initialized_notification() -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    })
}

/// Run zetl mcp with the given messages, return parsed NDJSON responses.
fn run_mcp(vault: &TempDir, messages: &[Value]) -> Vec<Value> {
    let stdin_data = build_stdin(messages);

    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("zetl");
    let output = cmd
        .args([
            "-d",
            vault.path().to_str().unwrap(),
            "mcp",
            "--transport",
            "stdio",
        ])
        .write_stdin(stdin_data)
        .timeout(Duration::from_secs(15))
        .output()
        .expect("failed to run zetl mcp");

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_ndjson(&stdout)
}

/// Helper: run the init handshake + additional messages, return all responses.
fn run_mcp_with_handshake(vault: &TempDir, extra: &[Value]) -> Vec<Value> {
    let mut messages = vec![initialize_request(1), initialized_notification()];
    messages.extend_from_slice(extra);
    run_mcp(vault, &messages)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn mcp_stdio_initialize() {
    let vault = setup_vault();
    let responses = run_mcp(&vault, &[initialize_request(1)]);

    let resp = find_response(&responses, &json!(1)).expect("no response for initialize");
    let result = resp.get("result").expect("initialize has no result");
    let server_info = result.get("serverInfo").expect("no serverInfo");
    let name = server_info["name"].as_str().unwrap();
    assert!(
        name.contains("zetl"),
        "server name should contain 'zetl', got: {name}"
    );
}

#[test]
fn mcp_stdio_list_tools() {
    let vault = setup_vault();
    let responses = run_mcp_with_handshake(
        &vault,
        &[json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        })],
    );

    let resp = find_response(&responses, &json!(2)).expect("no response for tools/list");
    let result = resp.get("result").expect("tools/list has no result");
    let tools = result.get("tools").expect("no tools array");
    let tools_arr = tools.as_array().unwrap();

    let tool_names: Vec<&str> = tools_arr
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();

    // Core tools are always expected regardless of feature flags.
    let expected_core = [
        "get",
        "search",
        "links",
        "backlinks",
        "path",
        "similar",
        "check",
        "status",
    ];
    for name in &expected_core {
        assert!(
            tool_names.contains(name),
            "missing tool: {name}. Found: {tool_names:?}"
        );
    }

    // "reason" is only registered when built with --features reason.
    #[cfg(feature = "reason")]
    assert!(
        tool_names.contains(&"reason"),
        "missing tool: reason (expected with --features reason). Found: {tool_names:?}"
    );
}

#[test]
fn mcp_stdio_call_search() {
    let vault = setup_vault();
    let responses = run_mcp_with_handshake(
        &vault,
        &[json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "search",
                "arguments": {
                    "query": "architecture"
                }
            }
        })],
    );

    let resp = find_response(&responses, &json!(3)).expect("no response for tools/call search");
    let result = resp.get("result").expect("tools/call has no result");
    let content = result.get("content").expect("no content");
    let text = content[0]["text"].as_str().unwrap();
    assert!(
        text.to_lowercase().contains("architecture"),
        "search results should mention architecture, got: {text}"
    );
}

#[test]
fn mcp_stdio_call_links() {
    let vault = setup_vault();
    let responses = run_mcp_with_handshake(
        &vault,
        &[json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "links",
                "arguments": {
                    "page": "README"
                }
            }
        })],
    );

    let resp = find_response(&responses, &json!(4)).expect("no response for tools/call links");
    let result = resp.get("result").expect("tools/call has no result");
    let content = result.get("content").expect("no content");
    let text = content[0]["text"].as_str().unwrap();
    assert!(
        text.contains("Architecture"),
        "README forward links should include Architecture, got: {text}"
    );
}

#[test]
fn mcp_stdio_call_backlinks() {
    let vault = setup_vault();
    let responses = run_mcp_with_handshake(
        &vault,
        &[json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "backlinks",
                "arguments": {
                    "page": "Architecture"
                }
            }
        })],
    );

    let resp = find_response(&responses, &json!(5)).expect("no response for tools/call backlinks");
    let result = resp.get("result").expect("tools/call has no result");
    let content = result.get("content").expect("no content");
    let text = content[0]["text"].as_str().unwrap();
    assert!(
        text.contains("README"),
        "Architecture backlinks should include README, got: {text}"
    );
}

#[test]
fn mcp_stdio_call_get() {
    let vault = setup_vault();
    let responses = run_mcp_with_handshake(
        &vault,
        &[json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {
                "name": "get",
                "arguments": {
                    "page": "README"
                }
            }
        })],
    );

    let resp = find_response(&responses, &json!(6)).expect("no response for tools/call get");
    let result = resp.get("result").expect("tools/call has no result");
    let content = result.get("content").expect("no content");
    let text = content[0]["text"].as_str().unwrap();
    assert!(
        text.contains("Welcome") || text.contains("README"),
        "get should return page content, got: {text}"
    );
}

#[test]
fn mcp_stdio_call_check() {
    let vault = setup_vault();
    let responses = run_mcp_with_handshake(
        &vault,
        &[json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": {
                "name": "check",
                "arguments": {}
            }
        })],
    );

    let resp = find_response(&responses, &json!(7)).expect("no response for tools/call check");
    let result = resp.get("result").expect("tools/call has no result");
    let content = result.get("content").expect("no content");
    let text = content[0]["text"].as_str().unwrap();
    // The check tool returns health information — it should at least be valid JSON.
    let parsed: Value = serde_json::from_str(text).expect("check output should be valid JSON");
    assert!(
        parsed.get("dead_links").is_some()
            || parsed.get("orphans").is_some()
            || parsed.get("page_count").is_some(),
        "check result should contain health info, got: {text}"
    );
}

#[test]
fn mcp_stdio_call_status() {
    let vault = setup_vault();
    let responses = run_mcp_with_handshake(
        &vault,
        &[json!({
            "jsonrpc": "2.0",
            "id": 8,
            "method": "tools/call",
            "params": {
                "name": "status",
                "arguments": {}
            }
        })],
    );

    let resp = find_response(&responses, &json!(8)).expect("no response for tools/call status");
    let result = resp.get("result").expect("tools/call has no result");
    let content = result.get("content").expect("no content");
    let text = content[0]["text"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(text).expect("status output should be valid JSON");
    let page_count = parsed
        .get("page_count")
        .and_then(|v| v.as_u64())
        .expect("status should have page_count");
    assert!(
        page_count >= 3,
        "vault has 3 pages, got page_count: {page_count}"
    );
}
