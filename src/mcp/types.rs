//! Shared types: McpState, DelegateClaims, ToolError, etc.

use crate::graph::LinkGraph;
use crate::search_index::SearchIndex;
use crate::simhash::SimHashIndex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

/// Shared server state, cheaply cloneable via Arc internals.
#[derive(Clone)]
pub struct McpState {
    pub vault_root: Arc<PathBuf>,
    pub graph: Arc<LinkGraph>,
    pub tantivy: Arc<SearchIndex>,
    pub simhash: Arc<SimHashIndex>,
    pub file_index: Arc<Vec<(String, PathBuf)>>,
    pub resolved: Arc<HashSet<String>>,
    pub page_names: Arc<Vec<String>>,
    pub allowed_issuers: Arc<HashMap<String, String>>,
    pub started_at: Instant,
    /// Transport mode: "stdio" or "http"
    pub transport: Arc<String>,
}

/// JWT payload for delegate tokens.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DelegateClaims {
    /// User / issuer identifier.
    pub iss: String,
    /// Subject — always `"zetl-mcp"`.
    pub sub: String,
    /// Vault identifier (audience).
    pub aud: String,
    /// Issued-at (Unix timestamp seconds).
    pub iat: u64,
    /// Expiry (Unix timestamp seconds). `0` means no expiry.
    pub exp: u64,
    /// Allowed tool names. Empty list means all tools are allowed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    /// Page glob patterns. Empty list means all pages are accessible.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scope: Vec<String>,
}

/// Resolved authentication context derived from a verified `DelegateClaims`.
#[derive(Debug, Clone)]
pub struct DelegateContext {
    pub issuer: String,
    pub tools: Vec<String>,
    pub scope: Vec<String>,
    pub expires_at: u64,
}

/// Typed errors returned by MCP tool handlers.
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
    /// JSON-RPC error code for this variant.
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

    fn sample_claims() -> DelegateClaims {
        DelegateClaims {
            iss: "alice".to_string(),
            sub: "zetl-mcp".to_string(),
            aud: "my-vault".to_string(),
            iat: 1_000_000,
            exp: 2_000_000,
            tools: vec!["get".to_string(), "search".to_string()],
            scope: vec!["notes/**".to_string()],
        }
    }

    #[test]
    fn delegate_claims_serde_round_trip() {
        let original = sample_claims();
        let json = serde_json::to_string(&original).expect("serialize");
        let recovered: DelegateClaims = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn delegate_claims_empty_tools_omitted() {
        let claims = DelegateClaims {
            iss: "bob".to_string(),
            sub: "zetl-mcp".to_string(),
            aud: "vault-x".to_string(),
            iat: 500,
            exp: 0,
            tools: vec![],
            scope: vec![],
        };
        let json = serde_json::to_string(&claims).expect("serialize");
        // Neither "tools" nor "scope" keys should appear in the JSON output.
        assert!(
            !json.contains("\"tools\""),
            "\"tools\" key present but should be omitted: {json}"
        );
        assert!(
            !json.contains("\"scope\""),
            "\"scope\" key present but should be omitted: {json}"
        );
        // Round-trip should still work correctly.
        let recovered: DelegateClaims = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(recovered.tools, Vec::<String>::new());
        assert_eq!(recovered.scope, Vec::<String>::new());
    }

    #[test]
    fn tool_error_codes() {
        assert_eq!(ToolError::PageNotFound("x".into()).code(), -32001);
        assert_eq!(ToolError::InvalidParam("p".into()).code(), -32602);
        assert_eq!(ToolError::AccessDenied("a".into()).code(), -32003);
        assert_eq!(ToolError::FeatureUnavailable("f".into()).code(), -32004);
        assert_eq!(ToolError::Internal("e".into()).code(), -32603);
    }
}
