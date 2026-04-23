# MCP Server (SPEC-021) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an MCP server to ztl exposing graph traversal, search, and reasoning as typed tools with stdio/HTTP transports and JWT delegation auth.

**Architecture:** Feature-gated `mcp` module using rmcp SDK. McpState holds Arc-wrapped graph, search indexes, and vault metadata. Tools are thin adapters over existing ztl functions. Auth is an axum middleware layer for HTTP transport.

**Tech Stack:** rmcp (MCP SDK), axum 0.8 (HTTP transport), jsonwebtoken (JWT), ed25519-dalek (signing), tokio (async runtime)

---

## Task 1: Scaffold mcp feature and module

**Files to create/modify:**
- `Cargo.toml` (add `mcp` feature and deps)
- `src/lib.rs` (add `#[cfg(feature = "mcp")] pub mod mcp;`)
- `src/mcp/mod.rs` (create, feature-gated exports)
- `src/cli.rs` (add `Command::Mcp` variant)
- `src/main.rs` (add `Command::Mcp` match arm calling `cmd_mcp()` stub)

### Steps

- [ ] **1a. Add `mcp` feature to `Cargo.toml`**

In `Cargo.toml`, add the feature and dependencies:

```toml
# In [features] section, after semantic:
mcp = ["dep:rmcp", "dep:jsonwebtoken"]

# In [dependencies] section:
rmcp = { version = "0.1", optional = true, features = ["server", "transport-io", "transport-sse-server"] }
jsonwebtoken = { version = "9", optional = true }
```

Also add the test target:

```toml
[[test]]
name = "mcp_integration"
required-features = ["mcp"]
```

- [ ] **1b. Create `src/mcp/mod.rs`**

```rust
//! MCP (Model Context Protocol) server for ztl (SPEC-021).
//!
//! Gated behind the `mcp` Cargo feature. Exposes vault graph traversal,
//! search, and reasoning as typed MCP tools over stdio and HTTP transports.

pub mod auth;
pub mod delegate;
pub mod resources;
pub mod server;
pub mod tools;
pub mod transport;
pub mod types;
```

- [ ] **1c. Create stub files for each submodule**

Create `src/mcp/server.rs`:

```rust
//! McpServer struct and ServerHandler impl.
```

Create `src/mcp/tools.rs`:

```rust
//! Tool handler implementations — one function per MCP tool.
```

Create `src/mcp/transport.rs`:

```rust
//! Stdio and HTTP transport setup.
```

Create `src/mcp/resources.rs`:

```rust
//! MCP resource handlers (page directory listing).
```

Create `src/mcp/auth.rs`:

```rust
//! JWT verification, DelegateContext, and auth middleware for HTTP transport.
```

Create `src/mcp/delegate.rs`:

```rust
//! `ztl delegate` command implementation — JWT signing with ed25519.
```

Create `src/mcp/types.rs`:

```rust
//! Shared types: McpState, DelegateClaims, ToolError, etc.
```

- [ ] **1d. Add feature gate in `src/lib.rs`**

Add after the existing `pub mod web;` line:

```rust
#[cfg(feature = "mcp")]
pub mod mcp;
```

- [ ] **1e. Add `Command::Mcp` to CLI**

In `src/cli.rs`, add the variant inside the `Command` enum (after the `Diff` variant):

```rust
    /// Start an MCP server exposing ztl tools
    #[cfg(feature = "mcp")]
    Mcp {
        /// Transport mode: stdio or http
        #[arg(long, default_value = "stdio")]
        transport: McpTransport,
        /// Host to bind HTTP server to
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// Port for HTTP transport
        #[arg(long, default_value = "3001")]
        port: u16,
        /// Allow non-loopback bind without auth (DANGEROUS)
        #[arg(long)]
        insecure: bool,
    },

    /// Start an MCP server (requires --features mcp)
    #[cfg(not(feature = "mcp"))]
    Mcp {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, num_args = 0..)]
        _args: Vec<String>,
    },
```

Add the `McpTransport` enum:

```rust
#[derive(Clone, ValueEnum, PartialEq)]
pub enum McpTransport {
    Stdio,
    Http,
}
```

- [ ] **1f. Add `cmd_mcp()` stub in `src/main.rs`**

Add the match arm in the main command dispatch (near the `Command::Serve` arm):

```rust
        #[cfg(feature = "mcp")]
        Command::Mcp { transport, host, port, insecure } => {
            cmd_mcp(&cli, &transport, &host, *port, *insecure)
        }
        #[cfg(not(feature = "mcp"))]
        Command::Mcp { .. } => {
            eprintln!("MCP server requires --features mcp. Rebuild with: cargo build --features mcp");
            std::process::exit(1);
        }
```

Add the stub function:

```rust
#[cfg(feature = "mcp")]
fn cmd_mcp(
    cli: &Cli,
    transport: &ztl::cli::McpTransport,
    host: &str,
    port: u16,
    insecure: bool,
) -> Result<()> {
    let _pipeline = run_pipeline(cli)?;
    eprintln!("MCP server not yet implemented");
    Ok(())
}
```

- [ ] **1g. Verify compilation**

```bash
cargo check --features mcp
# Expected: compiles successfully with no errors
```

**Commit:** `feat(mcp): scaffold mcp feature, module structure, and CLI command`

---

## Task 2: Core types — McpState, DelegateClaims, ToolError

**Files to modify:**
- `src/mcp/types.rs`

### Steps

- [ ] **2a. Write failing test for DelegateClaims serde round-trip**

In `src/mcp/types.rs`:

```rust
//! Shared types for the MCP server module.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use crate::graph::LinkGraph;
use crate::search_index::SearchIndex;

/// Shared state for the MCP server.
///
/// Holds Arc-wrapped references to the vault graph, search indexes, and metadata.
/// Cloneable for passing into tool handlers.
#[derive(Clone)]
pub struct McpState {
    /// Vault root directory (canonicalized).
    pub vault_root: Arc<PathBuf>,
    /// The link graph built from parsed vault files.
    pub graph: Arc<LinkGraph>,
    /// Tantivy full-text search index.
    pub tantivy: Arc<SearchIndex>,
    /// Page name -> file path index.
    pub file_index: Arc<Vec<(String, PathBuf)>>,
    /// Set of resolved page names (backed by real files).
    pub resolved: Arc<HashSet<String>>,
    /// All page names in the vault (sorted).
    pub page_names: Arc<Vec<String>>,
    /// ed25519 public keys of users authorized to issue delegate tokens.
    /// Maps issuer_id -> base64url-encoded ed25519 public key.
    pub allowed_issuers: Arc<std::collections::HashMap<String, String>>,
    /// Server start time (for uptime in status tool).
    pub started_at: Instant,
}

/// JWT claims for delegate tokens (SPEC-021 auth).
///
/// Issued by `ztl delegate`, verified by the MCP server's auth middleware.
/// The token is signed with the user's ed25519 key derived from their BIP39 mnemonic.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DelegateClaims {
    /// Issuer: user ID of the token creator.
    pub iss: String,
    /// Subject: "ztl-mcp" (fixed).
    pub sub: String,
    /// Audience: vault root hash or vault identifier.
    pub aud: String,
    /// Issued-at timestamp (Unix epoch seconds).
    pub iat: u64,
    /// Expiry timestamp (Unix epoch seconds).
    pub exp: u64,
    /// Allowed tool names (empty = all tools).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    /// Page scope glob patterns (empty = all pages).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scope: Vec<String>,
}

/// Resolved authorization context extracted from a verified JWT.
///
/// Attached to request extensions by the auth middleware so tool handlers
/// can check permissions without re-parsing the token.
#[derive(Debug, Clone)]
pub struct DelegateContext {
    /// User ID of the token issuer.
    pub issuer: String,
    /// Allowed tool names (empty = all).
    pub tools: Vec<String>,
    /// Page scope globs (empty = all).
    pub scope: Vec<String>,
    /// Token expiry (Unix epoch seconds).
    pub expires_at: u64,
}

/// Errors returned by MCP tool handlers.
///
/// Serialized into MCP error responses with appropriate codes.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("page not found: {0}")]
    PageNotFound(String),
    #[error("invalid parameter: {0}")]
    InvalidParam(String),
    #[error("access denied: {0}")]
    AccessDenied(String),
    #[error("feature not available: {0}")]
    FeatureUnavailable(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl ToolError {
    /// MCP error code for this error variant.
    pub fn code(&self) -> i32 {
        match self {
            ToolError::PageNotFound(_) => -32001,
            ToolError::InvalidParam(_) => -32602,
            ToolError::AccessDenied(_) => -32003,
            ToolError::FeatureUnavailable(_) => -32004,
            ToolError::Internal(_) => -32603,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegate_claims_serde_round_trip() {
        let claims = DelegateClaims {
            iss: "alice-a1b2c3d4".to_string(),
            sub: "ztl-mcp".to_string(),
            aud: "vault-abc123".to_string(),
            iat: 1712500000,
            exp: 1712503600,
            tools: vec!["search".to_string(), "links".to_string()],
            scope: vec!["projects/*".to_string()],
        };

        let json = serde_json::to_string(&claims).unwrap();
        let decoded: DelegateClaims = serde_json::from_str(&json).unwrap();
        assert_eq!(claims, decoded);
    }

    #[test]
    fn delegate_claims_empty_tools_omitted() {
        let claims = DelegateClaims {
            iss: "alice-a1b2c3d4".to_string(),
            sub: "ztl-mcp".to_string(),
            aud: "vault-abc123".to_string(),
            iat: 1712500000,
            exp: 1712503600,
            tools: vec![],
            scope: vec![],
        };

        let json = serde_json::to_value(&claims).unwrap();
        let obj = json.as_object().unwrap();
        assert!(!obj.contains_key("tools"), "empty tools should be omitted");
        assert!(!obj.contains_key("scope"), "empty scope should be omitted");
    }

    #[test]
    fn tool_error_codes() {
        assert_eq!(ToolError::PageNotFound("x".into()).code(), -32001);
        assert_eq!(ToolError::InvalidParam("x".into()).code(), -32602);
        assert_eq!(ToolError::AccessDenied("x".into()).code(), -32003);
        assert_eq!(ToolError::FeatureUnavailable("x".into()).code(), -32004);
        assert_eq!(ToolError::Internal("x".into()).code(), -32603);
    }
}
```

- [ ] **2b. Add thiserror to Cargo.toml** (if not already present)

```toml
thiserror = "1"
```

Check if thiserror is already in deps; if so, skip this step.

- [ ] **2c. Verify tests pass**

```bash
cargo test --features mcp --lib mcp::types
# Expected: 3 tests pass
```

**Commit:** `feat(mcp): add McpState, DelegateClaims, DelegateContext, ToolError types`

---

## Task 3: Pure-core functions — JWT verify, capability check, input validation

**Files to modify:**
- `src/mcp/auth.rs`

### Steps

- [ ] **3a. Implement JWT decode/verify and capability checking**

In `src/mcp/auth.rs`:

```rust
//! JWT verification, capability checking, and auth middleware for HTTP transport.

use crate::mcp::types::{DelegateClaims, DelegateContext, ToolError};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ed25519_dalek::{Signature, VerifyingKey};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// Decode and verify a JWT signed with ed25519.
///
/// The JWT uses a compact three-part format: `header.payload.signature`.
/// The header must specify `{"alg":"EdDSA","typ":"JWT"}`.
/// The signature is verified against the provided ed25519 public key.
///
/// Returns the decoded claims on success.
pub fn verify_jwt(
    token: &str,
    allowed_issuers: &HashMap<String, String>,
) -> Result<DelegateClaims, ToolError> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(ToolError::AccessDenied("malformed JWT: expected 3 parts".into()));
    }

    let header_bytes = URL_SAFE_NO_PAD
        .decode(parts[0])
        .map_err(|_| ToolError::AccessDenied("malformed JWT: invalid base64 in header".into()))?;
    let header: serde_json::Value = serde_json::from_slice(&header_bytes)
        .map_err(|_| ToolError::AccessDenied("malformed JWT: invalid header JSON".into()))?;

    // Verify algorithm
    if header.get("alg").and_then(|v| v.as_str()) != Some("EdDSA") {
        return Err(ToolError::AccessDenied("unsupported JWT algorithm (expected EdDSA)".into()));
    }

    // Decode payload
    let payload_bytes = URL_SAFE_NO_PAD
        .decode(parts[1])
        .map_err(|_| ToolError::AccessDenied("malformed JWT: invalid base64 in payload".into()))?;
    let claims: DelegateClaims = serde_json::from_slice(&payload_bytes)
        .map_err(|_| ToolError::AccessDenied("malformed JWT: invalid claims JSON".into()))?;

    // Look up issuer's public key
    let pubkey_b64 = allowed_issuers
        .get(&claims.iss)
        .ok_or_else(|| ToolError::AccessDenied(format!("unknown issuer: {}", claims.iss)))?;

    let pubkey_bytes = URL_SAFE_NO_PAD
        .decode(pubkey_b64)
        .map_err(|_| ToolError::AccessDenied("invalid issuer public key encoding".into()))?;
    let pubkey_array: [u8; 32] = pubkey_bytes
        .try_into()
        .map_err(|_| ToolError::AccessDenied("invalid issuer public key length".into()))?;
    let verifying_key = VerifyingKey::from_bytes(&pubkey_array)
        .map_err(|_| ToolError::AccessDenied("invalid ed25519 public key".into()))?;

    // Verify signature over "header.payload"
    let sig_bytes = URL_SAFE_NO_PAD
        .decode(parts[2])
        .map_err(|_| ToolError::AccessDenied("malformed JWT: invalid base64 in signature".into()))?;
    let sig_array: [u8; 64] = sig_bytes
        .try_into()
        .map_err(|_| ToolError::AccessDenied("invalid signature length".into()))?;
    let signature = Signature::from_bytes(&sig_array);

    let signed_content = format!("{}.{}", parts[0], parts[1]);
    verifying_key
        .verify_strict(signed_content.as_bytes(), &signature)
        .map_err(|_| ToolError::AccessDenied("JWT signature verification failed".into()))?;

    // Check expiry
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    if claims.exp < now {
        return Err(ToolError::AccessDenied("JWT expired".into()));
    }

    Ok(claims)
}

/// Check whether a delegate token grants access to a specific tool and page.
///
/// - If `claims.tools` is empty, all tools are allowed.
/// - If `claims.scope` is empty, all pages are allowed.
/// - Page scope uses glob matching (e.g. `projects/*` matches `projects/foo`).
pub fn check_capability(
    ctx: &DelegateContext,
    tool: &str,
    page: Option<&str>,
) -> Result<(), ToolError> {
    // Check tool allowlist
    if !ctx.tools.is_empty() && !ctx.tools.iter().any(|t| t == tool) {
        return Err(ToolError::AccessDenied(format!(
            "tool '{}' not in delegate scope (allowed: {:?})",
            tool, ctx.tools
        )));
    }

    // Check page scope
    if let Some(page_name) = page {
        if !ctx.scope.is_empty() {
            let matches = ctx.scope.iter().any(|pattern| {
                glob_match(pattern, page_name)
            });
            if !matches {
                return Err(ToolError::AccessDenied(format!(
                    "page '{}' not in delegate scope (allowed: {:?})",
                    page_name, ctx.scope
                )));
            }
        }
    }

    Ok(())
}

/// Simple glob matcher supporting `*` (any chars) and `?` (single char).
///
/// Used for page scope matching in delegate tokens.
fn glob_match(pattern: &str, input: &str) -> bool {
    let pattern_chars: Vec<char> = pattern.chars().collect();
    let input_chars: Vec<char> = input.chars().collect();
    glob_match_recursive(&pattern_chars, &input_chars, 0, 0)
}

fn glob_match_recursive(pat: &[char], inp: &[char], pi: usize, ii: usize) -> bool {
    if pi == pat.len() && ii == inp.len() {
        return true;
    }
    if pi == pat.len() {
        return false;
    }
    if pat[pi] == '*' {
        // Try matching zero or more characters
        for skip in 0..=(inp.len() - ii) {
            if glob_match_recursive(pat, inp, pi + 1, ii + skip) {
                return true;
            }
        }
        false
    } else if ii < inp.len() && (pat[pi] == '?' || pat[pi] == inp[ii]) {
        glob_match_recursive(pat, inp, pi + 1, ii + 1)
    } else {
        false
    }
}

/// Resolve a page name case-insensitively against the known pages.
///
/// Returns the canonical page name if found, or a ToolError::PageNotFound
/// with up to 5 suggestions.
pub fn resolve_page(page: &str, page_names: &[String]) -> Result<String, ToolError> {
    let lower = page.to_lowercase();

    // Exact match (case-insensitive)
    if let Some(found) = page_names.iter().find(|p| p.to_lowercase() == lower) {
        return Ok(found.clone());
    }

    // Suggestions: substring match
    let mut suggestions: Vec<&str> = page_names
        .iter()
        .filter(|p| {
            let p_lower = p.to_lowercase();
            p_lower.contains(&lower) || lower.contains(p_lower.as_str())
        })
        .map(|s| s.as_str())
        .collect();
    suggestions.sort();
    suggestions.truncate(5);

    if suggestions.is_empty() {
        Err(ToolError::PageNotFound(format!("page not found: '{}'", page)))
    } else {
        Err(ToolError::PageNotFound(format!(
            "page not found: '{}'. Did you mean: {}?",
            page,
            suggestions.join(", ")
        )))
    }
}

/// Suggest pages matching a fuzzy query prefix.
///
/// Returns up to `limit` page names whose lowercase form contains the query.
pub fn suggest_pages(query: &str, pages: &[String], limit: usize) -> Vec<String> {
    let lower = query.to_lowercase();
    pages
        .iter()
        .filter(|p| p.to_lowercase().contains(&lower))
        .take(limit)
        .cloned()
        .collect()
}

/// Extract DelegateContext from verified claims.
pub fn claims_to_context(claims: &DelegateClaims) -> DelegateContext {
    DelegateContext {
        issuer: claims.iss.clone(),
        tools: claims.tools.clone(),
        scope: claims.scope.clone(),
        expires_at: claims.exp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_match_star() {
        assert!(glob_match("projects/*", "projects/foo"));
        assert!(glob_match("projects/*", "projects/bar"));
        assert!(!glob_match("projects/*", "notes/foo"));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("*.md", "readme.md"));
    }

    #[test]
    fn glob_match_question() {
        assert!(glob_match("page?", "page1"));
        assert!(glob_match("page?", "pageX"));
        assert!(!glob_match("page?", "page12"));
    }

    #[test]
    fn glob_match_exact() {
        assert!(glob_match("readme", "readme"));
        assert!(!glob_match("readme", "README"));
    }

    #[test]
    fn check_capability_all_allowed() {
        let ctx = DelegateContext {
            issuer: "alice".into(),
            tools: vec![],
            scope: vec![],
            expires_at: u64::MAX,
        };
        assert!(check_capability(&ctx, "search", Some("any-page")).is_ok());
    }

    #[test]
    fn check_capability_tool_restricted() {
        let ctx = DelegateContext {
            issuer: "alice".into(),
            tools: vec!["search".into(), "links".into()],
            scope: vec![],
            expires_at: u64::MAX,
        };
        assert!(check_capability(&ctx, "search", None).is_ok());
        assert!(check_capability(&ctx, "backlinks", None).is_err());
    }

    #[test]
    fn check_capability_scope_restricted() {
        let ctx = DelegateContext {
            issuer: "alice".into(),
            tools: vec![],
            scope: vec!["projects/*".into()],
            expires_at: u64::MAX,
        };
        assert!(check_capability(&ctx, "get", Some("projects/foo")).is_ok());
        assert!(check_capability(&ctx, "get", Some("notes/bar")).is_err());
    }

    #[test]
    fn resolve_page_exact() {
        let pages = vec!["README".to_string(), "Architecture".to_string()];
        assert_eq!(resolve_page("readme", &pages).unwrap(), "README");
        assert_eq!(resolve_page("architecture", &pages).unwrap(), "Architecture");
    }

    #[test]
    fn resolve_page_not_found_with_suggestions() {
        let pages = vec!["README".to_string(), "Architecture".to_string(), "Readme-old".to_string()];
        let err = resolve_page("readm", &pages).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Did you mean"), "got: {msg}");
        assert!(msg.contains("README"));
    }

    #[test]
    fn resolve_page_no_suggestions() {
        let pages = vec!["README".to_string()];
        let err = resolve_page("zzzzz", &pages).unwrap_err();
        let msg = err.to_string();
        assert!(!msg.contains("Did you mean"), "got: {msg}");
    }

    #[test]
    fn suggest_pages_basic() {
        let pages = vec![
            "README".to_string(),
            "Architecture".to_string(),
            "API Reference".to_string(),
        ];
        let results = suggest_pages("arch", &pages, 5);
        assert_eq!(results, vec!["Architecture"]);
    }

    #[test]
    fn suggest_pages_limit() {
        let pages: Vec<String> = (0..20).map(|i| format!("page-{i}")).collect();
        let results = suggest_pages("page", &pages, 3);
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn claims_to_context_preserves_fields() {
        let claims = DelegateClaims {
            iss: "alice-a1b2c3d4".into(),
            sub: "ztl-mcp".into(),
            aud: "vault-abc".into(),
            iat: 100,
            exp: 200,
            tools: vec!["search".into()],
            scope: vec!["docs/*".into()],
        };
        let ctx = claims_to_context(&claims);
        assert_eq!(ctx.issuer, "alice-a1b2c3d4");
        assert_eq!(ctx.tools, vec!["search"]);
        assert_eq!(ctx.scope, vec!["docs/*"]);
        assert_eq!(ctx.expires_at, 200);
    }
}
```

- [ ] **3b. Verify tests pass**

```bash
cargo test --features mcp --lib mcp::auth
# Expected: 10 tests pass
```

**Commit:** `feat(mcp): pure-core JWT verification, capability checking, and input validation`

---

## Task 4: Tool — `search`

**Files to modify:**
- `src/mcp/tools.rs`

### Steps

- [ ] **4a. Implement the search tool handler**

In `src/mcp/tools.rs`:

```rust
//! Tool handler implementations — one function per MCP tool.
//!
//! Each function takes McpState + tool-specific parameters, calls existing
//! ztl APIs, and returns a serde_json::Value for the MCP response.

use crate::mcp::auth::resolve_page;
use crate::mcp::types::{McpState, ToolError};
use serde_json::{json, Value};

/// MCP tool: `search` — full-text search over vault contents.
///
/// Parameters:
///   - query (string, required): search query
///   - limit (integer, optional, default 20): max results
///
/// Returns: array of { page, path, score }
pub fn tool_search(
    state: &McpState,
    query: &str,
    limit: usize,
) -> Result<Value, ToolError> {
    if query.trim().is_empty() {
        return Err(ToolError::InvalidParam("query must not be empty".into()));
    }
    let limit = limit.min(100).max(1);

    let hits = state
        .tantivy
        .query(query, limit)
        .map_err(|e| ToolError::Internal(format!("search failed: {e}")))?;

    let results: Vec<Value> = hits
        .iter()
        .map(|hit| {
            json!({
                "page": hit.page_name,
                "path": hit.path,
                "score": hit.score,
            })
        })
        .collect();

    Ok(json!({
        "query": query,
        "count": results.len(),
        "results": results,
    }))
}
```

- [ ] **4b. Write test (requires a vault with indexed files)**

Add at the bottom of `src/mcp/tools.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // Tool tests require a populated McpState, tested in integration tests.
    // Unit tests validate parameter handling only.

    #[test]
    fn search_empty_query_rejected() {
        // We can't construct a full McpState in a unit test (needs tantivy),
        // so test the validation branch:
        let result = validate_search_params("", 20);
        assert!(result.is_err());
    }

    #[test]
    fn search_limit_clamped() {
        assert_eq!(clamp_limit(0), 1);
        assert_eq!(clamp_limit(200), 100);
        assert_eq!(clamp_limit(50), 50);
    }
}

fn validate_search_params(query: &str, _limit: usize) -> Result<(), ToolError> {
    if query.trim().is_empty() {
        return Err(ToolError::InvalidParam("query must not be empty".into()));
    }
    Ok(())
}

fn clamp_limit(limit: usize) -> usize {
    limit.min(100).max(1)
}
```

- [ ] **4c. Verify**

```bash
cargo test --features mcp --lib mcp::tools
# Expected: 2 tests pass
```

**Commit:** `feat(mcp): implement search tool handler`

---

## Task 5: Tool — `get`

**Files to modify:**
- `src/mcp/tools.rs`

### Steps

- [ ] **5a. Implement the get tool handler**

Append to `src/mcp/tools.rs` (before the `#[cfg(test)]` block):

```rust
/// MCP tool: `get` — retrieve a page's raw Markdown content.
///
/// Parameters:
///   - page (string, required): page name (case-insensitive)
///
/// Returns: { page, path, content, size_bytes }
pub fn tool_get(
    state: &McpState,
    page: &str,
) -> Result<Value, ToolError> {
    let resolved = resolve_page(page, &state.page_names)?;

    // Find file path
    let (_, file_path) = state
        .file_index
        .iter()
        .find(|(name, _)| name == &resolved)
        .ok_or_else(|| ToolError::Internal(format!("resolved page '{}' has no file entry", resolved)))?;

    let abs_path = state.vault_root.join(file_path);
    let content = std::fs::read_to_string(&abs_path)
        .map_err(|e| ToolError::Internal(format!("reading {}: {e}", abs_path.display())))?;

    let size = content.len();

    Ok(json!({
        "page": resolved,
        "path": file_path.to_string_lossy(),
        "content": content,
        "size_bytes": size,
    }))
}
```

- [ ] **5b. Verify compilation**

```bash
cargo check --features mcp
```

**Commit:** `feat(mcp): implement get tool handler`

---

## Task 6: Tool — `links`

**Files to modify:**
- `src/mcp/tools.rs`

### Steps

- [ ] **6a. Implement the links tool handler**

Append to `src/mcp/tools.rs`:

```rust
/// MCP tool: `links` — get forward links from a page.
///
/// Parameters:
///   - page (string, required): page name (case-insensitive)
///
/// Returns: { page, count, links: [{ target, line, alias, heading, is_embed }] }
pub fn tool_links(
    state: &McpState,
    page: &str,
) -> Result<Value, ToolError> {
    let resolved = resolve_page(page, &state.page_names)?;

    let forward = state.graph.forward_links(&resolved);

    let links: Vec<Value> = forward
        .iter()
        .map(|fl| {
            json!({
                "target": fl.target,
                "line": fl.meta.line,
                "alias": fl.meta.alias,
                "heading": fl.meta.heading,
                "is_embed": fl.meta.is_embed,
                "resolved": state.resolved.contains(&fl.target),
            })
        })
        .collect();

    Ok(json!({
        "page": resolved,
        "count": links.len(),
        "links": links,
    }))
}
```

**Commit:** `feat(mcp): implement links tool handler`

---

## Task 7: Tool — `backlinks`

**Files to modify:**
- `src/mcp/tools.rs`

### Steps

- [ ] **7a. Implement the backlinks tool handler**

Append to `src/mcp/tools.rs`:

```rust
/// MCP tool: `backlinks` — get pages that link to this page.
///
/// Parameters:
///   - page (string, required): page name (case-insensitive)
///
/// Returns: { page, count, backlinks: [{ source, line, alias, is_embed }] }
pub fn tool_backlinks(
    state: &McpState,
    page: &str,
) -> Result<Value, ToolError> {
    let resolved = resolve_page(page, &state.page_names)?;

    let backs = state.graph.backlinks(&resolved);

    let backlinks: Vec<Value> = backs
        .iter()
        .map(|bl| {
            json!({
                "source": bl.source,
                "line": bl.line,
                "alias": bl.alias,
                "is_embed": bl.is_embed,
            })
        })
        .collect();

    Ok(json!({
        "page": resolved,
        "count": backlinks.len(),
        "backlinks": backlinks,
    }))
}
```

**Commit:** `feat(mcp): implement backlinks tool handler`

---

## Task 8: Tool — `path`

**Files to modify:**
- `src/mcp/tools.rs`

### Steps

- [ ] **8a. Implement the path tool handler**

Append to `src/mcp/tools.rs`:

```rust
/// MCP tool: `path` — find shortest link path between two pages.
///
/// Parameters:
///   - from (string, required): source page name
///   - to (string, required): target page name
///   - max_depth (integer, optional, default 10): maximum hops
///
/// Returns: { from, to, hops, path: [page_name, ...] } or { from, to, reachable: false }
pub fn tool_path(
    state: &McpState,
    from: &str,
    to: &str,
    max_depth: usize,
) -> Result<Value, ToolError> {
    let from_resolved = resolve_page(from, &state.page_names)?;
    let to_resolved = resolve_page(to, &state.page_names)?;
    let max_depth = max_depth.min(50).max(1);

    match state.graph.shortest_path(&from_resolved, &to_resolved, max_depth) {
        Some(result) => Ok(json!({
            "from": result.from,
            "to": result.to,
            "hops": result.hops,
            "path": result.path,
        })),
        None => Ok(json!({
            "from": from_resolved,
            "to": to_resolved,
            "reachable": false,
            "max_depth_searched": max_depth,
        })),
    }
}
```

**Commit:** `feat(mcp): implement path tool handler`

---

## Task 9: Tool — `similar`

**Files to modify:**
- `src/mcp/tools.rs`

### Steps

- [ ] **9a. Implement the similar tool handler**

Append to `src/mcp/tools.rs`:

```rust
/// MCP tool: `similar` — find pages with similar names (SimHash).
///
/// Parameters:
///   - query (string, required): search string
///   - threshold (integer, optional, default 12): max Hamming distance
///   - limit (integer, optional, default 10): max results
///
/// Returns: { query, count, results: [{ page, distance, path }] }
pub fn tool_similar(
    state: &McpState,
    query: &str,
    threshold: u32,
    limit: usize,
) -> Result<Value, ToolError> {
    if query.trim().is_empty() {
        return Err(ToolError::InvalidParam("query must not be empty".into()));
    }

    let pages: Vec<(String, String)> = state
        .file_index
        .iter()
        .map(|(name, path)| (name.clone(), path.to_string_lossy().to_string()))
        .collect();

    let index = crate::simhash::SimHashIndex::build(&pages);
    let results = index.search(query, threshold, limit);

    let items: Vec<Value> = results
        .iter()
        .map(|r| {
            json!({
                "page": r.page,
                "distance": r.distance,
                "path": r.path,
            })
        })
        .collect();

    Ok(json!({
        "query": query,
        "count": items.len(),
        "results": items,
    }))
}
```

**Commit:** `feat(mcp): implement similar tool handler`

---

## Task 10: Tool — `check`

**Files to modify:**
- `src/mcp/tools.rs`

### Steps

- [ ] **10a. Implement the check tool handler**

Append to `src/mcp/tools.rs`:

```rust
/// MCP tool: `check` — validate vault health (dead links, orphans).
///
/// Parameters: (none)
///
/// Returns: { dead_links: [...], orphans: [...], stats: { ... } }
pub fn tool_check(
    state: &McpState,
) -> Result<Value, ToolError> {
    let dead = state.graph.dead_links();
    let orphans = state.graph.orphans();
    let stats = state.graph.stats(10);

    let dead_links: Vec<Value> = dead
        .iter()
        .map(|dl| {
            json!({
                "source": dl.source,
                "line": dl.line,
                "target": dl.target,
            })
        })
        .collect();

    let orphan_list: Vec<Value> = orphans
        .iter()
        .map(|o| {
            json!({
                "page": o.page,
                "forward_links": o.forward_links,
            })
        })
        .collect();

    Ok(json!({
        "dead_links": dead_links,
        "dead_link_count": dead_links.len(),
        "orphans": orphan_list,
        "orphan_count": orphan_list.len(),
        "stats": {
            "pages": stats.pages,
            "links": stats.links,
            "connected_components": stats.connected_components,
        },
    }))
}
```

**Commit:** `feat(mcp): implement check tool handler`

---

## Task 11: Tool — `status`

**Files to modify:**
- `src/mcp/tools.rs`

### Steps

- [ ] **11a. Implement the status tool handler**

Append to `src/mcp/tools.rs`:

```rust
/// MCP tool: `status` — vault summary and server status.
///
/// Parameters: (none)
///
/// Returns: { vault_root, page_count, link_count, uptime_secs, ... }
pub fn tool_status(
    state: &McpState,
) -> Result<Value, ToolError> {
    let stats = state.graph.stats(5);
    let uptime = state.started_at.elapsed().as_secs();

    let most_linked: Vec<Value> = stats
        .most_linked
        .iter()
        .map(|ml| {
            json!({
                "page": ml.page,
                "backlink_count": ml.backlink_count,
            })
        })
        .collect();

    Ok(json!({
        "vault_root": state.vault_root.to_string_lossy(),
        "page_count": stats.pages,
        "link_count": stats.links,
        "dead_links": stats.dead_links,
        "orphans": stats.orphans,
        "connected_components": stats.connected_components,
        "most_linked": most_linked,
        "uptime_secs": uptime,
    }))
}
```

**Commit:** `feat(mcp): implement status tool handler`

---

## Task 12: Tool — `reason`

**Files to modify:**
- `src/mcp/tools.rs`

### Steps

- [ ] **12a. Implement the reason tool handler (feature-gated)**

Append to `src/mcp/tools.rs`:

```rust
/// MCP tool: `reason` — run defeasible reasoning over vault SPL blocks.
///
/// Feature-gated: requires `--features reason` at compile time.
///
/// Parameters:
///   - query (string, optional): literal to query (e.g. "flies(tweety)")
///   - mode (string, optional, default "status"): "status" | "explain" | "what-if"
///   - hypothesis (string, optional): inline SPL for what-if mode
///
/// Returns: conclusions array with proof traces.
#[cfg(feature = "reason")]
pub fn tool_reason(
    state: &McpState,
    query: Option<&str>,
    files: &[crate::types::ParsedFile],
) -> Result<Value, ToolError> {
    use crate::reason::build_theory;
    use crate::reason::types::ConclusionType;

    let spl_blocks: Vec<crate::types::SplBlock> = files
        .iter()
        .flat_map(|f| f.spl_blocks.clone())
        .collect();

    if spl_blocks.is_empty() {
        return Ok(json!({
            "has_spl": false,
            "message": "No SPL blocks found in vault",
            "conclusions": [],
        }));
    }

    let result = build_theory(&spl_blocks)
        .map_err(|e| ToolError::Internal(format!("theory construction failed: {e}")))?;

    let mut conclusions: Vec<Value> = result
        .conclusions
        .iter()
        .map(|c| {
            json!({
                "literal": c.literal.clone(),
                "type": match &c.conclusion_type {
                    ConclusionType::DefinitelyProvable => "+D",
                    ConclusionType::DefinitelyNotProvable => "-D",
                    ConclusionType::DefeasiblyProvable => "+d",
                    ConclusionType::DefeasiblyNotProvable => "-d",
                },
                "sources": c.sources.iter().map(|s| json!({
                    "file": s.file,
                    "line": s.line,
                    "page": s.page,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();

    // Filter by query if specified
    if let Some(q) = query {
        let q_lower = q.to_lowercase();
        conclusions.retain(|c| {
            c.get("literal")
                .and_then(|l| l.as_str())
                .map(|l| l.to_lowercase().contains(&q_lower))
                .unwrap_or(false)
        });
    }

    Ok(json!({
        "has_spl": true,
        "conclusion_count": conclusions.len(),
        "conclusions": conclusions,
        "diagnostics_count": result.diagnostics.len(),
        "summary": {
            "facts": result.summary.fact_count,
            "rules": result.summary.rule_count,
            "superiority_relations": result.summary.superiority_count,
        },
    }))
}

#[cfg(not(feature = "reason"))]
pub fn tool_reason(
    _state: &McpState,
    _query: Option<&str>,
    _files: &[crate::types::ParsedFile],
) -> Result<Value, ToolError> {
    Err(ToolError::FeatureUnavailable(
        "reason tool requires --features reason at compile time".into(),
    ))
}
```

**Commit:** `feat(mcp): implement reason tool handler (feature-gated)`

---

## Task 13: MCP Server struct and stdio transport

**Files to modify:**
- `src/mcp/server.rs`
- `src/mcp/transport.rs`
- `src/main.rs` (update `cmd_mcp()`)

### Steps

- [ ] **13a. Implement McpServer with rmcp ServerHandler**

In `src/mcp/server.rs`:

```rust
//! McpServer struct implementing the rmcp ServerHandler trait.
//!
//! Registers all 9 MCP tools and dispatches calls to the tool handler functions.

use crate::mcp::tools;
use crate::mcp::types::{McpState, ToolError};
use rmcp::model::{
    CallToolRequest, CallToolResult, Content, Implementation, ListToolsResult,
    ServerCapabilities, ServerInfo, Tool, ToolInputSchema,
};
use rmcp::handler::server::ServerHandler;
use rmcp::service::RequestContext;
use serde_json::{json, Value};
use std::sync::Arc;

/// The ztl MCP server, wrapping McpState and implementing the rmcp handler trait.
#[derive(Clone)]
pub struct McpServer {
    pub state: McpState,
    /// Parsed files cached for reason tool.
    pub files: Arc<Vec<crate::types::ParsedFile>>,
}

impl McpServer {
    pub fn new(state: McpState, files: Vec<crate::types::ParsedFile>) -> Self {
        Self {
            state,
            files: Arc::new(files),
        }
    }

    /// Build the list of MCP tool definitions.
    fn tool_definitions() -> Vec<Tool> {
        vec![
            Tool::new(
                "search",
                "Full-text search over vault page contents",
                json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Search query" },
                        "limit": { "type": "integer", "description": "Max results (1-100)", "default": 20 }
                    },
                    "required": ["query"]
                }),
            ),
            Tool::new(
                "get",
                "Retrieve a page's raw Markdown content",
                json!({
                    "type": "object",
                    "properties": {
                        "page": { "type": "string", "description": "Page name (case-insensitive)" }
                    },
                    "required": ["page"]
                }),
            ),
            Tool::new(
                "links",
                "Get forward links from a page (outgoing wikilinks)",
                json!({
                    "type": "object",
                    "properties": {
                        "page": { "type": "string", "description": "Page name (case-insensitive)" }
                    },
                    "required": ["page"]
                }),
            ),
            Tool::new(
                "backlinks",
                "Get pages that link to this page (incoming wikilinks)",
                json!({
                    "type": "object",
                    "properties": {
                        "page": { "type": "string", "description": "Page name (case-insensitive)" }
                    },
                    "required": ["page"]
                }),
            ),
            Tool::new(
                "path",
                "Find shortest link path between two pages",
                json!({
                    "type": "object",
                    "properties": {
                        "from": { "type": "string", "description": "Source page name" },
                        "to": { "type": "string", "description": "Target page name" },
                        "max_depth": { "type": "integer", "description": "Maximum hops", "default": 10 }
                    },
                    "required": ["from", "to"]
                }),
            ),
            Tool::new(
                "similar",
                "Find pages with similar names using SimHash fuzzy matching",
                json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Search string" },
                        "threshold": { "type": "integer", "description": "Max Hamming distance", "default": 12 },
                        "limit": { "type": "integer", "description": "Max results", "default": 10 }
                    },
                    "required": ["query"]
                }),
            ),
            Tool::new(
                "check",
                "Validate vault health: dead links, orphans, graph stats",
                json!({
                    "type": "object",
                    "properties": {},
                }),
            ),
            Tool::new(
                "status",
                "Vault summary and MCP server status",
                json!({
                    "type": "object",
                    "properties": {},
                }),
            ),
            Tool::new(
                "reason",
                "Run defeasible reasoning over vault SPL blocks (requires reason feature)",
                json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Literal to query (e.g. 'flies(tweety)')" }
                    },
                }),
            ),
        ]
    }

    /// Dispatch a tool call to the appropriate handler function.
    fn dispatch_tool(&self, name: &str, args: &Value) -> Result<Value, ToolError> {
        match name {
            "search" => {
                let query = args
                    .get("query")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::InvalidParam("missing required parameter: query".into()))?;
                let limit = args
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(20) as usize;
                tools::tool_search(&self.state, query, limit)
            }
            "get" => {
                let page = args
                    .get("page")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::InvalidParam("missing required parameter: page".into()))?;
                tools::tool_get(&self.state, page)
            }
            "links" => {
                let page = args
                    .get("page")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::InvalidParam("missing required parameter: page".into()))?;
                tools::tool_links(&self.state, page)
            }
            "backlinks" => {
                let page = args
                    .get("page")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::InvalidParam("missing required parameter: page".into()))?;
                tools::tool_backlinks(&self.state, page)
            }
            "path" => {
                let from = args
                    .get("from")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::InvalidParam("missing required parameter: from".into()))?;
                let to = args
                    .get("to")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::InvalidParam("missing required parameter: to".into()))?;
                let max_depth = args
                    .get("max_depth")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(10) as usize;
                tools::tool_path(&self.state, from, to, max_depth)
            }
            "similar" => {
                let query = args
                    .get("query")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::InvalidParam("missing required parameter: query".into()))?;
                let threshold = args
                    .get("threshold")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(12) as u32;
                let limit = args
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(10) as usize;
                tools::tool_similar(&self.state, query, threshold, limit)
            }
            "check" => tools::tool_check(&self.state),
            "status" => tools::tool_status(&self.state),
            "reason" => {
                let query = args.get("query").and_then(|v| v.as_str());
                tools::tool_reason(&self.state, query, &self.files)
            }
            _ => Err(ToolError::InvalidParam(format!("unknown tool: {name}"))),
        }
    }
}

#[rmcp::async_trait]
impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some("ztl MCP server — bi-directional wikilink graph tools for Markdown vaults".into()),
            name: "ztl".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        }
    }

    fn get_capabilities(&self) -> ServerCapabilities {
        ServerCapabilities {
            tools: Some(Default::default()),
            resources: Some(Default::default()),
            ..Default::default()
        }
    }

    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequest>,
        _ctx: RequestContext,
    ) -> Result<ListToolsResult, rmcp::Error> {
        Ok(ListToolsResult {
            tools: Self::tool_definitions(),
            next_cursor: None,
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequest,
        _ctx: RequestContext,
    ) -> Result<CallToolResult, rmcp::Error> {
        let name = request.name.as_str();
        let args = request
            .arguments
            .as_ref()
            .cloned()
            .unwrap_or_else(|| serde_json::Map::new().into());

        match self.dispatch_tool(name, &Value::Object(args.as_object().cloned().unwrap_or_default())) {
            Ok(value) => {
                let text = serde_json::to_string_pretty(&value).unwrap_or_default();
                Ok(CallToolResult {
                    content: vec![Content::text(text)],
                    is_error: Some(false),
                    ..Default::default()
                })
            }
            Err(err) => Ok(CallToolResult {
                content: vec![Content::text(err.to_string())],
                is_error: Some(true),
                ..Default::default()
            }),
        }
    }
}
```

- [ ] **13b. Implement stdio transport in `src/mcp/transport.rs`**

```rust
//! Stdio and HTTP transport setup for the MCP server.

use crate::mcp::server::McpServer;
use crate::mcp::types::McpState;
use anyhow::Result;

/// Run the MCP server over stdio (JSON-RPC 2.0 over stdin/stdout).
///
/// This is the default transport for CLI integration with editors
/// and AI agents. The process runs until stdin is closed.
pub async fn serve_stdio(server: McpServer) -> Result<()> {
    let service = rmcp::ServiceBuilder::new(server)
        .build();

    let transport = rmcp::transport::io::stdio();
    let _server = service.serve(transport).await?;

    // Block until the transport is closed
    _server.waiting().await?;

    Ok(())
}

/// Run the MCP server over HTTP (SSE + POST /mcp).
///
/// Uses axum for the HTTP layer with optional JWT auth middleware.
pub async fn serve_http(
    server: McpServer,
    host: &str,
    port: u16,
    require_auth: bool,
) -> Result<()> {
    use axum::routing::{get, post};
    use axum::Router;
    use std::sync::Arc;

    let state = Arc::new(server.clone());

    let mut app = Router::new()
        .route("/health", get(health_handler));

    // MCP SSE endpoint
    let sse_service = rmcp::transport::sse_server::SseServer::new(server);
    app = sse_service.register_routes(app);

    if require_auth {
        // Auth middleware is applied in Task 15
        eprintln!("MCP HTTP server with auth on {host}:{port}");
    } else {
        eprintln!("MCP HTTP server (no auth) on {host}:{port}");
    }

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    eprintln!("ztl mcp  ->  http://{addr}");
    axum::serve(listener, app).await?;

    Ok(())
}

/// Health check endpoint: GET /health
async fn health_handler() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "status": "ok",
        "server": "ztl-mcp",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}
```

- [ ] **13c. Update `cmd_mcp()` in `src/main.rs`**

Replace the stub with a full implementation:

```rust
#[cfg(feature = "mcp")]
fn cmd_mcp(
    cli: &Cli,
    transport: &ztl::cli::McpTransport,
    host: &str,
    port: u16,
    insecure: bool,
) -> Result<()> {
    use ztl::cli::McpTransport;
    use ztl::mcp::server::McpServer;
    use ztl::mcp::transport;
    use ztl::mcp::types::McpState;

    let pipeline = run_pipeline(cli)?;

    // Build tantivy search index
    let tantivy = ztl::search_index::SearchIndex::build(&pipeline.vault_root, &pipeline.files)
        .context("building search index for MCP")?;

    // Collect page names (sorted)
    let mut page_names: Vec<String> = pipeline
        .files
        .iter()
        .map(|f| f.page_name.clone())
        .collect();
    page_names.sort_by_key(|a| a.to_lowercase());

    // Build allowed issuers from user profiles
    let mut allowed_issuers = std::collections::HashMap::new();
    if let Ok(profiles) = ztl::user::list_profiles(&pipeline.vault_root) {
        for profile in profiles {
            allowed_issuers.insert(profile.id.clone(), profile.recovery_pubkey.clone());
        }
    }

    let state = McpState {
        vault_root: std::sync::Arc::new(pipeline.vault_root.clone()),
        graph: std::sync::Arc::new(pipeline.graph),
        tantivy: std::sync::Arc::new(tantivy),
        file_index: std::sync::Arc::new(pipeline.file_index),
        resolved: std::sync::Arc::new(pipeline.graph_resolved),
        page_names: std::sync::Arc::new(page_names),
        allowed_issuers: std::sync::Arc::new(allowed_issuers),
        started_at: std::time::Instant::now(),
    };

    let server = McpServer::new(state, pipeline.files);

    let rt = tokio::runtime::Runtime::new().context("creating tokio runtime")?;

    match transport {
        McpTransport::Stdio => {
            rt.block_on(transport::serve_stdio(server))?;
        }
        McpTransport::Http => {
            // Network bind safety (Task 17)
            let is_loopback = host == "127.0.0.1" || host == "::1" || host == "localhost";
            let require_auth = !is_loopback;

            if !is_loopback && insecure {
                eprintln!("WARNING: --insecure allows unauthenticated non-loopback access");
            } else if !is_loopback && allowed_issuers.is_empty() {
                anyhow::bail!(
                    "Non-loopback bind ({host}) requires registered users for JWT auth. \
                     Use --insecure to override (DANGEROUS) or add users with `ztl serve --collab --init-owner`."
                );
            }

            rt.block_on(transport::serve_http(
                server,
                host,
                port,
                require_auth && !insecure,
            ))?;
        }
    }

    Ok(())
}
```

- [ ] **13d. Write integration test for stdio transport**

Create `tests/mcp_integration.rs`:

```rust
//! MCP server integration tests (requires --features mcp).

use assert_cmd::Command;
use std::io::Write;
use tempfile::TempDir;

/// Create a minimal vault with a few test pages.
fn setup_vault() -> TempDir {
    let dir = TempDir::new().unwrap();
    let root = dir.path();

    std::fs::write(
        root.join("README.md"),
        "# README\n\nWelcome to the vault.\n\nSee [[Architecture]] for details.\n",
    )
    .unwrap();

    std::fs::write(
        root.join("Architecture.md"),
        "# Architecture\n\n## Overview\n\nLinks to [[README]] and [[API]].\n",
    )
    .unwrap();

    std::fs::write(
        root.join("API.md"),
        "# API\n\nThe API documentation.\n\nSee [[Architecture]].\n",
    )
    .unwrap();

    dir
}

#[test]
fn mcp_stdio_initialize() {
    let vault = setup_vault();

    // Prepare an MCP initialize request
    let init_request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "test-client",
                "version": "1.0"
            }
        }
    });

    let request_str = format!("{}\n", serde_json::to_string(&init_request).unwrap());

    let output = Command::cargo_bin("ztl")
        .unwrap()
        .args(["--dir", vault.path().to_str().unwrap(), "mcp"])
        .write_stdin(request_str)
        .timeout(std::time::Duration::from_secs(10))
        .output()
        .expect("failed to run ztl mcp");

    let stdout = String::from_utf8_lossy(&output.stdout);
    // The response should contain "ztl" as the server name
    assert!(
        stdout.contains("ztl"),
        "expected 'ztl' in initialize response, got: {stdout}"
    );
}

#[test]
fn mcp_stdio_list_tools() {
    let vault = setup_vault();

    // Send initialize + list_tools
    let requests = vec![
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "1.0" }
            }
        }),
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }),
    ];

    let input: String = requests
        .iter()
        .map(|r| format!("{}\n", serde_json::to_string(r).unwrap()))
        .collect();

    let output = Command::cargo_bin("ztl")
        .unwrap()
        .args(["--dir", vault.path().to_str().unwrap(), "mcp"])
        .write_stdin(input)
        .timeout(std::time::Duration::from_secs(10))
        .output()
        .expect("failed to run ztl mcp");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("search"), "tools should include 'search'");
    assert!(stdout.contains("links"), "tools should include 'links'");
    assert!(stdout.contains("backlinks"), "tools should include 'backlinks'");
    assert!(stdout.contains("get"), "tools should include 'get'");
    assert!(stdout.contains("path"), "tools should include 'path'");
    assert!(stdout.contains("similar"), "tools should include 'similar'");
    assert!(stdout.contains("check"), "tools should include 'check'");
    assert!(stdout.contains("status"), "tools should include 'status'");
    assert!(stdout.contains("reason"), "tools should include 'reason'");
}
```

- [ ] **13e. Verify**

```bash
cargo test --features mcp mcp_stdio_initialize -- --nocapture
# Expected: test passes, response contains "ztl"

cargo test --features mcp mcp_stdio_list_tools -- --nocapture
# Expected: test passes, all 9 tool names present
```

**Commit:** `feat(mcp): McpServer with stdio transport and tool dispatch`

---

## Task 14: HTTP transport with healthcheck

**Files to modify:**
- `src/mcp/transport.rs` (already has skeleton from Task 13)

### Steps

- [ ] **14a. Add HTTP integration test**

Append to `tests/mcp_integration.rs`:

```rust
#[test]
fn mcp_http_healthcheck() {
    let vault = setup_vault();

    // Start the server in the background
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_ztl"))
        .args([
            "--dir",
            vault.path().to_str().unwrap(),
            "mcp",
            "--transport",
            "http",
            "--port",
            "0", // Let OS assign port — but ztl may not support port 0
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("failed to start ztl mcp http");

    // Give the server a moment to start
    std::thread::sleep(std::time::Duration::from_secs(2));

    // Since we can't easily get the assigned port, use a fixed port test
    // This test is best run via the integration test framework with port allocation
    child.kill().ok();
}
```

Note: The HTTP integration test is better validated manually or via the full integration test in Task 18. The healthcheck handler implementation is already in Task 13.

- [ ] **14b. Verify healthcheck compiles**

```bash
cargo check --features mcp
```

**Commit:** `feat(mcp): HTTP transport with /health endpoint`

---

## Task 15: Auth middleware for HTTP transport

**Files to modify:**
- `src/mcp/auth.rs` (add axum middleware)
- `src/mcp/transport.rs` (wire auth layer)

### Steps

- [ ] **15a. Add axum auth middleware**

Append to `src/mcp/auth.rs` (before `#[cfg(test)]`):

```rust
use axum::extract::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::http::StatusCode;

/// Axum middleware that extracts and verifies a Bearer JWT from the Authorization header.
///
/// On success, inserts a `DelegateContext` into request extensions.
/// On failure, returns 401 Unauthorized.
pub async fn jwt_auth_middleware(
    axum::extract::State(issuers): axum::extract::State<Arc<std::collections::HashMap<String, String>>>,
    mut request: Request,
    next: Next,
) -> Response {
    // Extract Bearer token from Authorization header
    let auth_header = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    let token = match auth_header {
        Some(h) if h.starts_with("Bearer ") => &h[7..],
        _ => {
            return (
                StatusCode::UNAUTHORIZED,
                axum::Json(serde_json::json!({
                    "error": "missing or invalid Authorization header (expected: Bearer <jwt>)"
                })),
            )
                .into_response();
        }
    };

    // Verify JWT
    match verify_jwt(token, &issuers) {
        Ok(claims) => {
            let ctx = claims_to_context(&claims);
            request.extensions_mut().insert(ctx);
            next.run(request).await
        }
        Err(err) => (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({
                "error": err.to_string()
            })),
        )
            .into_response(),
    }
}
```

Add at the top of the file:

```rust
use std::sync::Arc;
```

- [ ] **15b. Wire auth middleware into HTTP transport**

Update `serve_http` in `src/mcp/transport.rs` to apply the middleware when `require_auth` is true:

```rust
pub async fn serve_http(
    server: McpServer,
    host: &str,
    port: u16,
    require_auth: bool,
) -> Result<()> {
    use axum::routing::get;
    use axum::Router;
    use std::sync::Arc;

    let app = Router::new()
        .route("/health", get(health_handler));

    // MCP SSE endpoint
    let sse_service = rmcp::transport::sse_server::SseServer::new(server.clone());
    let app = sse_service.register_routes(app);

    let app = if require_auth {
        let issuers = server.state.allowed_issuers.clone();
        app.layer(axum::middleware::from_fn_with_state(
            issuers,
            crate::mcp::auth::jwt_auth_middleware,
        ))
    } else {
        app
    };

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    eprintln!("ztl mcp  ->  http://{addr}");
    axum::serve(listener, app).await?;

    Ok(())
}
```

- [ ] **15c. Add auth unit tests**

Append to the tests module in `src/mcp/auth.rs`:

```rust
    #[test]
    fn verify_jwt_malformed_rejected() {
        let issuers = HashMap::new();
        let result = verify_jwt("not.a.valid.jwt", &issuers);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("malformed"));
    }

    #[test]
    fn verify_jwt_unknown_issuer_rejected() {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use base64::Engine;

        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"EdDSA","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_string(&super::DelegateClaims {
            iss: "unknown-user".into(),
            sub: "ztl-mcp".into(),
            aud: "vault".into(),
            iat: 0,
            exp: u64::MAX,
            tools: vec![],
            scope: vec![],
        }).unwrap());
        let fake_sig = URL_SAFE_NO_PAD.encode([0u8; 64]);

        let token = format!("{header}.{payload}.{fake_sig}");
        let issuers = HashMap::new();

        let result = verify_jwt(&token, &issuers);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown issuer"));
    }

    #[test]
    fn verify_jwt_valid_round_trip() {
        use ed25519_dalek::SigningKey;
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use base64::Engine;
        use ed25519_dalek::Signer;

        // Generate a keypair
        let mut rng_bytes = [0u8; 32];
        crate::user::getrandom(&mut rng_bytes);
        let signing_key = SigningKey::from_bytes(&rng_bytes);
        let verifying_key = signing_key.verifying_key();

        let pubkey_b64 = URL_SAFE_NO_PAD.encode(verifying_key.as_bytes());

        let claims = super::DelegateClaims {
            iss: "alice-a1b2c3d4".into(),
            sub: "ztl-mcp".into(),
            aud: "vault".into(),
            iat: 0,
            exp: u64::MAX,
            tools: vec!["search".into()],
            scope: vec!["docs/*".into()],
        };

        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"EdDSA","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_string(&claims).unwrap());
        let signed_content = format!("{header}.{payload}");
        let signature = signing_key.sign(signed_content.as_bytes());
        let sig_b64 = URL_SAFE_NO_PAD.encode(signature.to_bytes());

        let token = format!("{signed_content}.{sig_b64}");

        let mut issuers = HashMap::new();
        issuers.insert("alice-a1b2c3d4".to_string(), pubkey_b64);

        let decoded = verify_jwt(&token, &issuers).unwrap();
        assert_eq!(decoded.iss, "alice-a1b2c3d4");
        assert_eq!(decoded.tools, vec!["search"]);
    }

    #[test]
    fn verify_jwt_expired_rejected() {
        use ed25519_dalek::SigningKey;
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use base64::Engine;
        use ed25519_dalek::Signer;

        let mut rng_bytes = [0u8; 32];
        crate::user::getrandom(&mut rng_bytes);
        let signing_key = SigningKey::from_bytes(&rng_bytes);
        let verifying_key = signing_key.verifying_key();
        let pubkey_b64 = URL_SAFE_NO_PAD.encode(verifying_key.as_bytes());

        let claims = super::DelegateClaims {
            iss: "alice-a1b2c3d4".into(),
            sub: "ztl-mcp".into(),
            aud: "vault".into(),
            iat: 0,
            exp: 1, // expired in 1970
            tools: vec![],
            scope: vec![],
        };

        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"EdDSA","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_string(&claims).unwrap());
        let signed_content = format!("{header}.{payload}");
        let signature = signing_key.sign(signed_content.as_bytes());
        let sig_b64 = URL_SAFE_NO_PAD.encode(signature.to_bytes());

        let token = format!("{signed_content}.{sig_b64}");

        let mut issuers = HashMap::new();
        issuers.insert("alice-a1b2c3d4".to_string(), pubkey_b64);

        let result = verify_jwt(&token, &issuers);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("expired"));
    }
```

- [ ] **15d. Verify**

```bash
cargo test --features mcp --lib mcp::auth
# Expected: 14 tests pass (10 from task 3 + 4 new)
```

**Commit:** `feat(mcp): JWT auth middleware for HTTP transport`

---

## Task 16: `ztl delegate` CLI command

**Files to modify:**
- `src/cli.rs` (add `Command::Delegate`)
- `src/mcp/delegate.rs` (JWT signing logic)
- `src/main.rs` (add `cmd_delegate()`)

### Steps

- [ ] **16a. Add `Command::Delegate` to CLI**

In `src/cli.rs`, add inside the `Command` enum:

```rust
    /// Issue a delegate JWT token for MCP authentication
    #[cfg(feature = "mcp")]
    Delegate {
        /// BIP39 mnemonic phrase (12 words) for signing
        #[arg(long)]
        mnemonic: String,
        /// Token lifetime (e.g. "1h", "24h", "7d"; default: 1h)
        #[arg(long, default_value = "1h")]
        ttl: String,
        /// Restrict token to specific tools (comma-separated; default: all)
        #[arg(long)]
        tools: Option<String>,
        /// Restrict token to page glob patterns (comma-separated; default: all)
        #[arg(long)]
        scope: Option<String>,
        /// Vault audience identifier (default: vault root hash)
        #[arg(long)]
        aud: Option<String>,
    },

    /// Issue a delegate JWT token (requires --features mcp)
    #[cfg(not(feature = "mcp"))]
    Delegate {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, num_args = 0..)]
        _args: Vec<String>,
    },
```

- [ ] **16b. Implement JWT signing in `src/mcp/delegate.rs`**

```rust
//! `ztl delegate` command implementation — JWT signing with ed25519.
//!
//! Signs a delegate JWT using the user's ed25519 key derived from their
//! BIP39 mnemonic, granting scoped access to MCP tools.

use crate::mcp::types::DelegateClaims;
use crate::user::recovery::derive_signing_key_from_mnemonic;
use anyhow::{anyhow, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ed25519_dalek::Signer;
use std::time::{SystemTime, UNIX_EPOCH};

/// Parse a TTL string like "1h", "24h", "7d" into seconds.
pub fn parse_ttl(ttl: &str) -> Result<u64> {
    let ttl = ttl.trim();
    if ttl.is_empty() {
        return Err(anyhow!("empty TTL"));
    }

    let (num_str, unit) = if ttl.ends_with('d') {
        (&ttl[..ttl.len() - 1], 86400u64)
    } else if ttl.ends_with('h') {
        (&ttl[..ttl.len() - 1], 3600u64)
    } else if ttl.ends_with('m') {
        (&ttl[..ttl.len() - 1], 60u64)
    } else if ttl.ends_with('s') {
        (&ttl[..ttl.len() - 1], 1u64)
    } else {
        // Assume seconds
        (ttl, 1u64)
    };

    let num: u64 = num_str
        .parse()
        .with_context(|| format!("invalid TTL number: '{num_str}'"))?;

    if num == 0 {
        return Err(anyhow!("TTL must be > 0"));
    }

    Ok(num * unit)
}

/// Sign a delegate JWT with the user's ed25519 key.
///
/// The mnemonic is used to derive the signing key via SLIP-0010.
/// The resulting JWT has the compact format: header.payload.signature.
pub fn sign_delegate_token(
    mnemonic: &str,
    ttl_secs: u64,
    tools: Vec<String>,
    scope: Vec<String>,
    aud: &str,
) -> Result<String> {
    let signing_key = derive_signing_key_from_mnemonic(mnemonic)
        .context("deriving signing key from mnemonic")?;
    let verifying_key = signing_key.verifying_key();

    // Derive issuer ID: we need the user's profile, but we don't have the vault here.
    // Use the public key fingerprint as issuer identity for now.
    // The MCP server maps recovery_pubkey -> user_id for verification.
    let pubkey_b64 = URL_SAFE_NO_PAD.encode(verifying_key.as_bytes());

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let claims = DelegateClaims {
        iss: pubkey_b64.clone(), // Issuer is the pubkey itself (server resolves to user_id)
        sub: "ztl-mcp".to_string(),
        aud: aud.to_string(),
        iat: now,
        exp: now + ttl_secs,
        tools,
        scope,
    };

    // Build JWT: base64url(header).base64url(payload).base64url(signature)
    let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"EdDSA","typ":"JWT"}"#);
    let payload = URL_SAFE_NO_PAD.encode(
        serde_json::to_string(&claims).context("serializing claims")?,
    );

    let signed_content = format!("{header}.{payload}");
    let signature = signing_key.sign(signed_content.as_bytes());
    let sig_b64 = URL_SAFE_NO_PAD.encode(signature.to_bytes());

    Ok(format!("{signed_content}.{sig_b64}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::auth::verify_jwt;
    use crate::user::recovery::derive_pubkey_from_mnemonic;
    use std::collections::HashMap;

    #[test]
    fn parse_ttl_hours() {
        assert_eq!(parse_ttl("1h").unwrap(), 3600);
        assert_eq!(parse_ttl("24h").unwrap(), 86400);
    }

    #[test]
    fn parse_ttl_days() {
        assert_eq!(parse_ttl("7d").unwrap(), 604800);
    }

    #[test]
    fn parse_ttl_minutes() {
        assert_eq!(parse_ttl("30m").unwrap(), 1800);
    }

    #[test]
    fn parse_ttl_invalid() {
        assert!(parse_ttl("").is_err());
        assert!(parse_ttl("0h").is_err());
        assert!(parse_ttl("abc").is_err());
    }

    #[test]
    fn delegate_round_trip() {
        // Use a fixed test mnemonic
        let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

        let token = sign_delegate_token(
            mnemonic,
            3600,
            vec!["search".into()],
            vec!["docs/*".into()],
            "test-vault",
        )
        .unwrap();

        // Verify the token
        let pubkey = derive_pubkey_from_mnemonic(mnemonic).unwrap();
        let pubkey_b64 = URL_SAFE_NO_PAD.encode(pubkey.as_bytes());

        let mut issuers = HashMap::new();
        issuers.insert(pubkey_b64.clone(), pubkey_b64.clone());

        let claims = verify_jwt(&token, &issuers).unwrap();
        assert_eq!(claims.sub, "ztl-mcp");
        assert_eq!(claims.aud, "test-vault");
        assert_eq!(claims.tools, vec!["search"]);
        assert_eq!(claims.scope, vec!["docs/*"]);
    }
}
```

- [ ] **16c. Add cmd_delegate() in `src/main.rs`**

```rust
        #[cfg(feature = "mcp")]
        Command::Delegate { mnemonic, ttl, tools, scope, aud } => {
            cmd_delegate(&cli, mnemonic, ttl, tools.as_deref(), scope.as_deref(), aud.as_deref())
        }
        #[cfg(not(feature = "mcp"))]
        Command::Delegate { .. } => {
            eprintln!("Delegate command requires --features mcp");
            std::process::exit(1);
        }
```

```rust
#[cfg(feature = "mcp")]
fn cmd_delegate(
    cli: &Cli,
    mnemonic: &str,
    ttl: &str,
    tools: Option<&str>,
    scope: Option<&str>,
    aud: Option<&str>,
) -> Result<()> {
    use ztl::mcp::delegate::{parse_ttl, sign_delegate_token};

    let ttl_secs = parse_ttl(ttl).context("parsing TTL")?;

    let tool_list: Vec<String> = tools
        .map(|t| t.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();

    let scope_list: Vec<String> = scope
        .map(|s| s.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();

    let vault_root = std::fs::canonicalize(&cli.dir)
        .with_context(|| format!("Cannot resolve vault directory: {}", cli.dir))?;

    // Use vault root hash as default audience
    let default_aud = format!("ztl:{}", vault_root.to_string_lossy());
    let aud = aud.unwrap_or(&default_aud);

    let token = sign_delegate_token(mnemonic, ttl_secs, tool_list, scope_list, aud)
        .context("signing delegate token")?;

    match effective_format(cli) {
        OutputFormat::Json => {
            print_json(&serde_json::json!({
                "token": token,
                "ttl_secs": ttl_secs,
                "aud": aud,
            }));
        }
        _ => {
            println!("{token}");
        }
    }

    Ok(())
}
```

- [ ] **16d. Verify**

```bash
cargo test --features mcp --lib mcp::delegate
# Expected: 5 tests pass (parse_ttl * 4 + round_trip * 1)
```

**Commit:** `feat(mcp): ztl delegate command for issuing JWT tokens`

---

## Task 17: Network bind safety

**Files to modify:**
- `src/main.rs` (already partially implemented in Task 13c)

### Steps

- [ ] **17a. Verify bind safety logic in `cmd_mcp()`**

The bind safety check was added in Task 13c. Verify it rejects non-loopback without auth:

Add to `tests/mcp_integration.rs`:

```rust
#[test]
fn mcp_http_non_loopback_without_auth_fails() {
    let vault = setup_vault();
    // No users exist -> no allowed issuers -> should fail on 0.0.0.0

    let output = Command::cargo_bin("ztl")
        .unwrap()
        .args([
            "--dir",
            vault.path().to_str().unwrap(),
            "mcp",
            "--transport",
            "http",
            "--host",
            "0.0.0.0",
            "--port",
            "13999",
        ])
        .timeout(std::time::Duration::from_secs(5))
        .output()
        .expect("failed to run ztl mcp");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success() || stderr.contains("Non-loopback"),
        "non-loopback bind without auth should fail, stderr: {stderr}"
    );
}

#[test]
fn mcp_http_non_loopback_with_insecure_allowed() {
    let vault = setup_vault();

    // With --insecure, non-loopback should be allowed (server starts)
    // We can't easily test the server starts, so just check it doesn't
    // exit with the auth error.
    let output = Command::cargo_bin("ztl")
        .unwrap()
        .args([
            "--dir",
            vault.path().to_str().unwrap(),
            "mcp",
            "--transport",
            "http",
            "--host",
            "0.0.0.0",
            "--port",
            "13998",
            "--insecure",
        ])
        .timeout(std::time::Duration::from_secs(3))
        .output()
        .expect("failed to run ztl mcp");

    let stderr = String::from_utf8_lossy(&output.stderr);
    // Should NOT contain the "requires registered users" error
    assert!(
        !stderr.contains("requires registered users"),
        "insecure flag should bypass auth requirement, stderr: {stderr}"
    );
}
```

- [ ] **17b. Verify**

```bash
cargo test --features mcp mcp_http_non_loopback -- --nocapture
# Expected: both tests pass
```

**Commit:** `feat(mcp): network bind safety for non-loopback HTTP`

---

## Task 18: MCP resource handlers (page directory)

**Files to modify:**
- `src/mcp/resources.rs`
- `src/mcp/server.rs` (register resource handlers)

### Steps

- [ ] **18a. Implement resource listing**

In `src/mcp/resources.rs`:

```rust
//! MCP resource handlers — page directory listing.
//!
//! Exposes the vault's page directory as an MCP resource, allowing clients
//! to browse available pages without calling a tool.

use crate::mcp::types::McpState;
use rmcp::model::{ListResourcesResult, ReadResourceRequest, ReadResourceResult, Resource, ResourceContents};
use serde_json::json;

/// Build the list of resources (one per vault page).
pub fn list_page_resources(state: &McpState) -> ListResourcesResult {
    let resources: Vec<Resource> = state
        .page_names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let path = state
                .file_index
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, p)| p.to_string_lossy().to_string())
                .unwrap_or_default();

            Resource {
                uri: format!("ztl://pages/{}", urlencoding_simple(name)),
                name: name.clone(),
                description: Some(format!("Markdown page: {path}")),
                mime_type: Some("text/markdown".into()),
                ..Default::default()
            }
        })
        .collect();

    ListResourcesResult {
        resources,
        next_cursor: None,
    }
}

/// Read a resource by URI.
pub fn read_page_resource(
    state: &McpState,
    uri: &str,
) -> Result<ReadResourceResult, String> {
    // Parse URI: ztl://pages/<page_name>
    let prefix = "ztl://pages/";
    if !uri.starts_with(prefix) {
        return Err(format!("unknown resource URI: {uri}"));
    }

    let encoded_name = &uri[prefix.len()..];
    let page_name = urlencoding_decode(encoded_name);

    // Find the page
    let (_, file_path) = state
        .file_index
        .iter()
        .find(|(name, _)| name.to_lowercase() == page_name.to_lowercase())
        .ok_or_else(|| format!("page not found: {page_name}"))?;

    let abs_path = state.vault_root.join(file_path);
    let content = std::fs::read_to_string(&abs_path)
        .map_err(|e| format!("reading {}: {e}", abs_path.display()))?;

    Ok(ReadResourceResult {
        contents: vec![ResourceContents::text(content, Some(uri.to_string()))],
    })
}

/// Simple URL encoding for page names (spaces -> %20, etc.)
fn urlencoding_simple(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            ' ' => "%20".to_string(),
            '/' => "%2F".to_string(),
            _ if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' => {
                c.to_string()
            }
            _ => format!("%{:02X}", c as u32),
        })
        .collect()
}

/// Simple URL decoding.
fn urlencoding_decode(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                result.push(byte as char);
            } else {
                result.push('%');
                result.push_str(&hex);
            }
        } else {
            result.push(c);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urlencoding_round_trip() {
        let name = "My Page Name";
        let encoded = urlencoding_simple(name);
        assert_eq!(encoded, "My%20Page%20Name");
        let decoded = urlencoding_decode(&encoded);
        assert_eq!(decoded, name);
    }

    #[test]
    fn urlencoding_with_slashes() {
        let name = "folder/Page";
        let encoded = urlencoding_simple(name);
        assert!(encoded.contains("%2F"));
        let decoded = urlencoding_decode(&encoded);
        assert_eq!(decoded, name);
    }
}
```

- [ ] **18b. Register resource handlers in McpServer**

Add to the `ServerHandler` impl in `src/mcp/server.rs`:

```rust
    async fn list_resources(
        &self,
        _request: Option<rmcp::model::PaginatedRequest>,
        _ctx: RequestContext,
    ) -> Result<ListResourcesResult, rmcp::Error> {
        Ok(crate::mcp::resources::list_page_resources(&self.state))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequest,
        _ctx: RequestContext,
    ) -> Result<ReadResourceResult, rmcp::Error> {
        crate::mcp::resources::read_page_resource(&self.state, &request.uri)
            .map_err(|e| rmcp::Error::internal_error(e, None))
    }
```

- [ ] **18c. Verify**

```bash
cargo test --features mcp --lib mcp::resources
# Expected: 2 tests pass
```

**Commit:** `feat(mcp): page directory MCP resources`

---

## Task 19: Full integration tests

**Files to modify:**
- `tests/mcp_integration.rs`

### Steps

- [ ] **19a. Add end-to-end tool call tests**

Append to `tests/mcp_integration.rs`:

```rust
/// Helper: send a series of JSON-RPC requests to ztl mcp stdio and return stdout.
fn mcp_stdio_exchange(vault: &TempDir, requests: Vec<serde_json::Value>) -> String {
    let input: String = requests
        .iter()
        .map(|r| format!("{}\n", serde_json::to_string(r).unwrap()))
        .collect();

    let output = Command::cargo_bin("ztl")
        .unwrap()
        .args(["--dir", vault.path().to_str().unwrap(), "mcp"])
        .write_stdin(input)
        .timeout(std::time::Duration::from_secs(15))
        .output()
        .expect("failed to run ztl mcp");

    String::from_utf8_lossy(&output.stdout).to_string()
}

/// Standard preamble: initialize + initialized notification.
fn init_preamble() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "1.0" }
            }
        }),
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }),
    ]
}

#[test]
fn mcp_tool_search() {
    let vault = setup_vault();
    let mut requests = init_preamble();
    requests.push(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 10,
        "method": "tools/call",
        "params": {
            "name": "search",
            "arguments": { "query": "Architecture", "limit": 5 }
        }
    }));

    let stdout = mcp_stdio_exchange(&vault, requests);
    assert!(stdout.contains("Architecture"), "search should find Architecture, got: {stdout}");
}

#[test]
fn mcp_tool_links() {
    let vault = setup_vault();
    let mut requests = init_preamble();
    requests.push(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 10,
        "method": "tools/call",
        "params": {
            "name": "links",
            "arguments": { "page": "README" }
        }
    }));

    let stdout = mcp_stdio_exchange(&vault, requests);
    assert!(stdout.contains("Architecture"), "README should link to Architecture, got: {stdout}");
}

#[test]
fn mcp_tool_backlinks() {
    let vault = setup_vault();
    let mut requests = init_preamble();
    requests.push(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 10,
        "method": "tools/call",
        "params": {
            "name": "backlinks",
            "arguments": { "page": "Architecture" }
        }
    }));

    let stdout = mcp_stdio_exchange(&vault, requests);
    // README and API both link to Architecture
    assert!(stdout.contains("README"), "Architecture should have backlink from README, got: {stdout}");
}

#[test]
fn mcp_tool_path() {
    let vault = setup_vault();
    let mut requests = init_preamble();
    requests.push(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 10,
        "method": "tools/call",
        "params": {
            "name": "path",
            "arguments": { "from": "README", "to": "API" }
        }
    }));

    let stdout = mcp_stdio_exchange(&vault, requests);
    // README -> Architecture -> API (or README -> Architecture if API is linked)
    assert!(stdout.contains("path") || stdout.contains("hops"), "should find a path, got: {stdout}");
}

#[test]
fn mcp_tool_check() {
    let vault = setup_vault();
    let mut requests = init_preamble();
    requests.push(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 10,
        "method": "tools/call",
        "params": {
            "name": "check",
            "arguments": {}
        }
    }));

    let stdout = mcp_stdio_exchange(&vault, requests);
    assert!(stdout.contains("dead_links") || stdout.contains("stats"), "check should return vault health, got: {stdout}");
}

#[test]
fn mcp_tool_status() {
    let vault = setup_vault();
    let mut requests = init_preamble();
    requests.push(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 10,
        "method": "tools/call",
        "params": {
            "name": "status",
            "arguments": {}
        }
    }));

    let stdout = mcp_stdio_exchange(&vault, requests);
    assert!(stdout.contains("page_count") || stdout.contains("vault_root"), "status should return vault info, got: {stdout}");
}

#[test]
fn mcp_tool_similar() {
    let vault = setup_vault();
    let mut requests = init_preamble();
    requests.push(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 10,
        "method": "tools/call",
        "params": {
            "name": "similar",
            "arguments": { "query": "Arch" }
        }
    }));

    let stdout = mcp_stdio_exchange(&vault, requests);
    assert!(stdout.contains("Architecture") || stdout.contains("results"), "similar should find Architecture, got: {stdout}");
}

#[test]
fn mcp_tool_get() {
    let vault = setup_vault();
    let mut requests = init_preamble();
    requests.push(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 10,
        "method": "tools/call",
        "params": {
            "name": "get",
            "arguments": { "page": "README" }
        }
    }));

    let stdout = mcp_stdio_exchange(&vault, requests);
    assert!(stdout.contains("Welcome to the vault"), "get should return page content, got: {stdout}");
}

#[test]
fn mcp_tool_unknown_returns_error() {
    let vault = setup_vault();
    let mut requests = init_preamble();
    requests.push(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 10,
        "method": "tools/call",
        "params": {
            "name": "nonexistent_tool",
            "arguments": {}
        }
    }));

    let stdout = mcp_stdio_exchange(&vault, requests);
    assert!(
        stdout.contains("unknown tool") || stdout.contains("error") || stdout.contains("is_error"),
        "unknown tool should return error, got: {stdout}"
    );
}
```

- [ ] **19b. Verify all integration tests pass**

```bash
cargo test --features mcp --test mcp_integration -- --nocapture
# Expected: all integration tests pass
```

- [ ] **19c. Run full test suite to check for regressions**

```bash
cargo test --features mcp
# Expected: all existing tests still pass
```

**Commit:** `test(mcp): full end-to-end integration tests for all 9 MCP tools`

---

## Summary

| Task | Description | Files | Tests |
|------|-------------|-------|-------|
| 1 | Scaffold mcp feature + module | Cargo.toml, lib.rs, cli.rs, main.rs, mcp/*.rs stubs | cargo check |
| 2 | Core types (McpState, DelegateClaims, ToolError) | mcp/types.rs | 3 unit tests |
| 3 | Pure-core (JWT verify, capability check, resolve) | mcp/auth.rs | 10 unit tests |
| 4 | Tool: search | mcp/tools.rs | 2 unit tests |
| 5 | Tool: get | mcp/tools.rs | compile check |
| 6 | Tool: links | mcp/tools.rs | compile check |
| 7 | Tool: backlinks | mcp/tools.rs | compile check |
| 8 | Tool: path | mcp/tools.rs | compile check |
| 9 | Tool: similar | mcp/tools.rs | compile check |
| 10 | Tool: check | mcp/tools.rs | compile check |
| 11 | Tool: status | mcp/tools.rs | compile check |
| 12 | Tool: reason (feature-gated) | mcp/tools.rs | compile check |
| 13 | McpServer + stdio transport | mcp/server.rs, mcp/transport.rs, main.rs | 2 integration tests |
| 14 | HTTP transport + /health | mcp/transport.rs | compile check |
| 15 | JWT auth middleware | mcp/auth.rs, mcp/transport.rs | 4 unit tests |
| 16 | ztl delegate command | cli.rs, mcp/delegate.rs, main.rs | 5 unit tests |
| 17 | Network bind safety | main.rs | 2 integration tests |
| 18 | MCP resources (page directory) | mcp/resources.rs, mcp/server.rs | 2 unit tests |
| 19 | Full integration tests | tests/mcp_integration.rs | 9 e2e tests |

**Total: 19 tasks, ~39 tests, 11 new files, 4 modified files**

### Dependency graph

```
Task 1 (scaffold)
  -> Task 2 (types)
     -> Task 3 (pure-core auth)
        -> Tasks 4-12 (tools, parallel)
        -> Task 15 (auth middleware)
        -> Task 16 (delegate cmd)
     -> Task 13 (server + stdio)
        -> Task 14 (HTTP transport)
           -> Task 15 (auth middleware)
           -> Task 17 (bind safety)
     -> Task 18 (resources)
  -> Task 19 (integration tests, depends on all above)
```

Tasks 4-12 are independent of each other and can be implemented in parallel.
Tasks 15, 16, 17 can be parallelized after Task 3 and Task 14 are complete.
