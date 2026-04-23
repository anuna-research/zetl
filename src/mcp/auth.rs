//! JWT verification, DelegateContext, and auth middleware for HTTP transport.

use crate::mcp::types::{DelegateClaims, DelegateContext, ToolError};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ed25519_dalek::{Signature, VerifyingKey};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// JWT verification
// ---------------------------------------------------------------------------

/// Decode and verify a compact JWT (header.payload.signature).
///
/// - Requires `alg: "EdDSA"` in the header.
/// - Looks up the issuer's base64url-encoded ed25519 public key from
///   `allowed_issuers` (issuer_id → base64url pubkey).
/// - Verifies the ed25519 signature over the raw `"header.payload"` bytes.
/// - Checks expiry: `exp == 0` means no expiry; otherwise rejects tokens
///   whose `exp` is in the past.
pub fn verify_jwt(
    token: &str,
    allowed_issuers: &HashMap<String, String>,
) -> Result<DelegateClaims, ToolError> {
    // 1. Split into three parts.
    let parts: Vec<&str> = token.splitn(3, '.').collect();
    if parts.len() != 3 {
        return Err(ToolError::AccessDenied(
            "malformed JWT: expected header.payload.signature".into(),
        ));
    }
    let (header_b64, payload_b64, sig_b64) = (parts[0], parts[1], parts[2]);

    // 2. Decode and validate header — must have alg: "EdDSA".
    let header_bytes = URL_SAFE_NO_PAD
        .decode(header_b64)
        .map_err(|e| ToolError::AccessDenied(format!("JWT header base64 error: {e}")))?;
    let header: serde_json::Value = serde_json::from_slice(&header_bytes)
        .map_err(|e| ToolError::AccessDenied(format!("JWT header JSON error: {e}")))?;
    let alg = header.get("alg").and_then(|v| v.as_str()).unwrap_or("");
    if alg != "EdDSA" {
        return Err(ToolError::AccessDenied(format!(
            "JWT alg must be EdDSA, got {alg:?}"
        )));
    }

    // 3. Decode payload as DelegateClaims.
    let payload_bytes = URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|e| ToolError::AccessDenied(format!("JWT payload base64 error: {e}")))?;
    let claims: DelegateClaims = serde_json::from_slice(&payload_bytes)
        .map_err(|e| ToolError::AccessDenied(format!("JWT payload JSON error: {e}")))?;

    // 4. Look up issuer's public key.
    let pubkey_b64 = allowed_issuers
        .get(&claims.iss)
        .ok_or_else(|| ToolError::AccessDenied(format!("unknown issuer: {}", claims.iss)))?;
    let pubkey_bytes = URL_SAFE_NO_PAD
        .decode(pubkey_b64)
        .map_err(|e| ToolError::AccessDenied(format!("issuer pubkey base64 error: {e}")))?;
    let pubkey_array: [u8; 32] = pubkey_bytes
        .try_into()
        .map_err(|_| ToolError::AccessDenied("issuer pubkey must be 32 bytes".into()))?;
    let verifying_key = VerifyingKey::from_bytes(&pubkey_array)
        .map_err(|e| ToolError::AccessDenied(format!("invalid ed25519 pubkey: {e}")))?;

    // 5. Verify signature over "header.payload" (the raw ASCII bytes).
    let signed_input = format!("{header_b64}.{payload_b64}");
    let sig_bytes = URL_SAFE_NO_PAD
        .decode(sig_b64)
        .map_err(|e| ToolError::AccessDenied(format!("JWT signature base64 error: {e}")))?;
    let sig_array: [u8; 64] = sig_bytes
        .try_into()
        .map_err(|_| ToolError::AccessDenied("JWT signature must be 64 bytes".into()))?;
    let signature = Signature::from_bytes(&sig_array);
    verifying_key
        .verify_strict(signed_input.as_bytes(), &signature)
        .map_err(|e| ToolError::AccessDenied(format!("JWT signature verification failed: {e}")))?;

    // 6. Check expiry.
    if claims.exp != 0 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if claims.exp < now {
            return Err(ToolError::AccessDenied(format!(
                "JWT expired at {}",
                claims.exp
            )));
        }
    }

    Ok(claims)
}

// ---------------------------------------------------------------------------
// Capability checking
// ---------------------------------------------------------------------------

/// Check that `ctx` permits `tool` to operate on `page`.
///
/// - Empty `ctx.tools` → all tools allowed.
/// - Non-empty `ctx.tools` → tool must be in the list OR list contains `"*"`.
/// - Empty `ctx.scope`  → all pages allowed.
/// - Non-empty `ctx.scope` + `page = Some(p)` → `p` must match at least one
///   glob pattern in `ctx.scope`.
/// - `page = None` → scope is not checked.
pub fn check_capability(
    ctx: &DelegateContext,
    tool: &str,
    page: Option<&str>,
) -> Result<(), ToolError> {
    // Tool check.
    if !ctx.tools.is_empty() {
        let allowed = ctx.tools.iter().any(|t| t == "*" || t == tool);
        if !allowed {
            return Err(ToolError::AccessDenied(format!(
                "tool {tool:?} not permitted for this token"
            )));
        }
    }

    // Scope check.
    if !ctx.scope.is_empty() {
        if let Some(p) = page {
            let allowed = ctx.scope.iter().any(|pat| glob_match(pat, p));
            if !allowed {
                return Err(ToolError::AccessDenied(format!(
                    "page {p:?} not within token scope"
                )));
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Glob matching
// ---------------------------------------------------------------------------

/// Simple glob matching.
///
/// - `*`  matches any characters within a single path segment (no `/`).
/// - `**` matches any characters including `/` (i.e. across segments).
/// - `**` alone matches everything.
///
/// Examples:
/// - `projects/*`  matches `projects/foo` but not `projects/foo/bar`
/// - `projects/**` matches `projects/foo` and `projects/foo/bar`
/// - `**`          matches anything
pub fn glob_match(pattern: &str, input: &str) -> bool {
    glob_match_inner(pattern.as_bytes(), input.as_bytes())
}

fn glob_match_inner(pat: &[u8], input: &[u8]) -> bool {
    let mut pi = 0usize; // pattern index
    let mut ii = 0usize; // input index

    while pi < pat.len() {
        // Check for `**`
        if pat[pi] == b'*' && pi + 1 < pat.len() && pat[pi + 1] == b'*' {
            // Consume `**` (and optional surrounding `/`)
            pi += 2;
            // Skip a leading `/` after `**` in pattern if present.
            if pi < pat.len() && pat[pi] == b'/' {
                pi += 1;
            }
            // If `**` is at the end of the pattern, it matches everything remaining.
            if pi == pat.len() {
                return true;
            }
            // Try matching the rest of the pattern against every possible suffix of input.
            for start in ii..=input.len() {
                if glob_match_inner(&pat[pi..], &input[start..]) {
                    return true;
                }
            }
            return false;
        }

        // Single `*` — matches any chars except `/`
        if pat[pi] == b'*' {
            pi += 1;
            // If `*` is at the end, match everything up to the next `/` (if any).
            if pi == pat.len() {
                // Must not have any `/` left in input.
                return !input[ii..].contains(&b'/');
            }
            // Try every possible non-`/` prefix.
            let mut j = ii;
            loop {
                if glob_match_inner(&pat[pi..], &input[j..]) {
                    return true;
                }
                if j >= input.len() || input[j] == b'/' {
                    break;
                }
                j += 1;
            }
            return false;
        }

        // Literal character match.
        if ii >= input.len() || pat[pi] != input[ii] {
            return false;
        }
        pi += 1;
        ii += 1;
    }

    // Pattern exhausted — input must also be exhausted.
    ii == input.len()
}

// ---------------------------------------------------------------------------
// Page resolution
// ---------------------------------------------------------------------------

/// Resolve a page name from a list of known page names.
///
/// 1. Case-insensitive exact match first.
/// 2. On miss, return `PageNotFound` with up to 5 substring suggestions.
pub fn resolve_page(page: &str, page_names: &[String]) -> Result<String, ToolError> {
    let lower = page.to_lowercase();

    // Case-insensitive exact match.
    for name in page_names {
        if name.to_lowercase() == lower {
            return Ok(name.clone());
        }
    }

    // Not found — collect suggestions.
    let suggestions = suggest_pages(page, page_names, 5);
    let msg = if suggestions.is_empty() {
        format!("{page:?} (no suggestions)")
    } else {
        format!("{page:?}; did you mean: {}", suggestions.join(", "))
    };
    Err(ToolError::PageNotFound(msg))
}

/// Return pages containing `query` as a substring (case-insensitive), up to `limit`.
pub fn suggest_pages(query: &str, pages: &[String], limit: usize) -> Vec<String> {
    let lower = query.to_lowercase();
    pages
        .iter()
        .filter(|p| p.to_lowercase().contains(&lower))
        .take(limit)
        .cloned()
        .collect()
}

// ---------------------------------------------------------------------------
// Claims → context
// ---------------------------------------------------------------------------

/// Convert verified `DelegateClaims` into a `DelegateContext`.
pub fn claims_to_context(claims: DelegateClaims) -> DelegateContext {
    DelegateContext {
        issuer: claims.iss,
        tools: claims.tools,
        scope: claims.scope,
        expires_at: claims.exp,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::types::DelegateClaims;

    // -- glob_match ----------------------------------------------------------

    #[test]
    fn glob_match_star() {
        // `*` matches within a single segment.
        assert!(glob_match("projects/*", "projects/foo"));
        assert!(glob_match("projects/*", "projects/bar"));
        // `*` must not cross `/`.
        assert!(!glob_match("projects/*", "projects/foo/bar"));
        // `*` at beginning of pattern.
        assert!(glob_match("*.md", "readme.md"));
        assert!(!glob_match("*.md", "src/readme.md"));
    }

    #[test]
    fn glob_match_double_star() {
        // `**` matches across segments.
        assert!(glob_match("projects/**", "projects/foo"));
        assert!(glob_match("projects/**", "projects/foo/bar"));
        assert!(glob_match("projects/**", "projects/a/b/c"));
        // `**` alone matches everything.
        assert!(glob_match("**", "anything/goes/here"));
        assert!(glob_match("**", ""));
        // Must still respect literal prefix.
        assert!(!glob_match("other/**", "projects/foo"));
    }

    #[test]
    fn glob_match_exact() {
        assert!(glob_match("notes/foo", "notes/foo"));
        assert!(!glob_match("notes/foo", "notes/bar"));
        assert!(!glob_match("notes/foo", "notes/foo/extra"));
    }

    // -- check_capability ----------------------------------------------------

    fn make_ctx(tools: &[&str], scope: &[&str]) -> DelegateContext {
        DelegateContext {
            issuer: "alice".into(),
            tools: tools.iter().map(|s| s.to_string()).collect(),
            scope: scope.iter().map(|s| s.to_string()).collect(),
            expires_at: 0,
        }
    }

    #[test]
    fn check_capability_all_allowed() {
        // Empty tools + empty scope = everything allowed.
        let ctx = make_ctx(&[], &[]);
        assert!(check_capability(&ctx, "any_tool", Some("any/page")).is_ok());
        assert!(check_capability(&ctx, "any_tool", None).is_ok());
    }

    #[test]
    fn check_capability_tool_restricted() {
        let ctx = make_ctx(&["get_page", "search"], &[]);
        assert!(check_capability(&ctx, "get_page", None).is_ok());
        assert!(check_capability(&ctx, "search", None).is_ok());
        assert!(check_capability(&ctx, "delete_page", None).is_err());
    }

    #[test]
    fn check_capability_wildcard_tool() {
        let ctx = make_ctx(&["*"], &[]);
        assert!(check_capability(&ctx, "anything", None).is_ok());
        assert!(check_capability(&ctx, "other_tool", Some("page")).is_ok());
    }

    #[test]
    fn check_capability_scope_restricted() {
        let ctx = make_ctx(&[], &["notes/**"]);
        assert!(check_capability(&ctx, "get_page", Some("notes/foo")).is_ok());
        assert!(check_capability(&ctx, "get_page", Some("notes/foo/bar")).is_ok());
        assert!(check_capability(&ctx, "get_page", Some("other/page")).is_err());
        // page = None → scope not checked.
        assert!(check_capability(&ctx, "get_page", None).is_ok());
    }

    // -- resolve_page / suggest_pages ----------------------------------------

    fn page_list() -> Vec<String> {
        vec![
            "Notes/Foo".into(),
            "Notes/Bar".into(),
            "Projects/Alpha".into(),
            "Projects/Beta".into(),
            "Journal/2024-01".into(),
        ]
    }

    #[test]
    fn resolve_page_exact() {
        let pages = page_list();
        // Exact case.
        assert_eq!(resolve_page("Notes/Foo", &pages).unwrap(), "Notes/Foo");
        // Case-insensitive.
        assert_eq!(resolve_page("notes/foo", &pages).unwrap(), "Notes/Foo");
        assert_eq!(
            resolve_page("PROJECTS/ALPHA", &pages).unwrap(),
            "Projects/Alpha"
        );
    }

    #[test]
    fn resolve_page_suggestions() {
        let pages = page_list();
        // Query "Notes" won't match any page exactly (they're "Notes/Foo", "Notes/Bar", etc.)
        // but "Notes/Foo" and "Notes/Bar" both contain "notes" as a substring.
        let err = resolve_page("Notes", &pages).unwrap_err();
        let msg = err.to_string();
        // Should say page not found.
        assert!(
            msg.contains("page not found"),
            "expected 'page not found' in: {msg}"
        );
        // Should include at least one suggestion from pages containing "Notes".
        assert!(
            msg.contains("Notes/Foo") || msg.contains("Notes/Bar"),
            "expected Notes/Foo or Notes/Bar in suggestions: {msg}"
        );
    }

    #[test]
    fn resolve_page_no_match() {
        let pages = page_list();
        let err = resolve_page("zzz-nonexistent", &pages).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("page not found"));
        assert!(msg.contains("no suggestions"));
    }

    // -- verify_jwt roundtrip ------------------------------------------------

    #[test]
    fn verify_jwt_roundtrip() {
        use ed25519_dalek::Signer;
        use ed25519_dalek::SigningKey;
        use rand_core::OsRng;

        // Generate a fresh keypair.
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();

        // Build a JWT manually.
        let header = r#"{"alg":"EdDSA","typ":"JWT"}"#;
        let claims = DelegateClaims {
            iss: "test-issuer".into(),
            sub: "ztl-mcp".into(),
            aud: "my-vault".into(),
            iat: 1_000_000,
            exp: 0, // no expiry
            tools: vec!["get_page".into()],
            scope: vec![],
        };
        let payload = serde_json::to_string(&claims).expect("serialize claims");

        let header_b64 = URL_SAFE_NO_PAD.encode(header.as_bytes());
        let payload_b64 = URL_SAFE_NO_PAD.encode(payload.as_bytes());
        let signed_input = format!("{header_b64}.{payload_b64}");

        let signature = signing_key.sign(signed_input.as_bytes());
        let sig_b64 = URL_SAFE_NO_PAD.encode(signature.to_bytes());

        let token = format!("{signed_input}.{sig_b64}");

        // Build allowed_issuers map.
        let pubkey_b64 = URL_SAFE_NO_PAD.encode(verifying_key.as_bytes());
        let mut allowed_issuers = HashMap::new();
        allowed_issuers.insert("test-issuer".to_string(), pubkey_b64);

        // Verify — must succeed and return matching claims.
        let returned = verify_jwt(&token, &allowed_issuers).expect("verify_jwt failed");
        assert_eq!(returned.iss, "test-issuer");
        assert_eq!(returned.tools, vec!["get_page".to_string()]);

        // Tampered token must fail.
        let tampered = format!("{signed_input}x.{sig_b64}");
        assert!(verify_jwt(&tampered, &allowed_issuers).is_err());

        // Wrong issuer must fail.
        let mut wrong_issuers = HashMap::new();
        wrong_issuers.insert(
            "other-issuer".to_string(),
            URL_SAFE_NO_PAD.encode(verifying_key.as_bytes()),
        );
        assert!(verify_jwt(&token, &wrong_issuers).is_err());
    }
}
