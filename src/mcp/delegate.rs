//! ztl delegate command implementation — JWT signing with ed25519.

use anyhow::{anyhow, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};

use crate::mcp::types::DelegateClaims;

/// Sign a `DelegateClaims` as a compact JWT using the given ed25519 signing key.
///
/// Produces: `base64url(header).base64url(payload).base64url(signature)`
/// where header is `{"alg":"EdDSA","typ":"JWT"}` and signature is ed25519
/// over the raw ASCII `"header.payload"` bytes.
pub fn sign_delegate_jwt(signing_key: &SigningKey, claims: &DelegateClaims) -> Result<String> {
    let header = r#"{"alg":"EdDSA","typ":"JWT"}"#;
    let payload =
        serde_json::to_vec(claims).map_err(|e| anyhow!("failed to serialize claims: {e}"))?;

    let header_b64 = URL_SAFE_NO_PAD.encode(header.as_bytes());
    let payload_b64 = URL_SAFE_NO_PAD.encode(&payload);
    let signed_input = format!("{header_b64}.{payload_b64}");

    let signature = signing_key.sign(signed_input.as_bytes());
    let sig_b64 = URL_SAFE_NO_PAD.encode(signature.to_bytes());

    Ok(format!("{signed_input}.{sig_b64}"))
}

/// Parse a human-friendly expiry duration string into a Unix timestamp.
///
/// Supported formats: `"1h"`, `"7d"`, `"30d"`, `"3600s"`.
/// Returns `now + duration` as a Unix timestamp (seconds).
pub fn parse_expiry(s: &str) -> Result<u64> {
    let s = s.trim();
    let (num_str, multiplier) = if let Some(n) = s.strip_suffix('d') {
        (n, 86_400u64)
    } else if let Some(n) = s.strip_suffix('h') {
        (n, 3_600u64)
    } else if let Some(n) = s.strip_suffix('s') {
        (n, 1u64)
    } else {
        // Assume seconds if no suffix.
        (s, 1u64)
    };

    let value: u64 = num_str
        .trim()
        .parse()
        .map_err(|_| anyhow!("invalid expiry duration: {s:?}"))?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    Ok(now + value * multiplier)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::auth::verify_jwt;
    use rand_core::OsRng;
    use std::collections::HashMap;

    #[test]
    fn sign_and_verify_roundtrip() {
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();

        let claims = DelegateClaims {
            iss: "test-user".into(),
            sub: "ztl-mcp".into(),
            aud: "test-vault".into(),
            iat: 1_000_000,
            exp: 0, // no expiry
            tools: vec!["search".into(), "get_page".into()],
            scope: vec!["notes/**".into()],
        };

        let jwt = sign_delegate_jwt(&signing_key, &claims).expect("signing should succeed");

        // Verify: build allowed_issuers map.
        let pubkey_b64 = URL_SAFE_NO_PAD.encode(verifying_key.as_bytes());
        let mut allowed_issuers = HashMap::new();
        allowed_issuers.insert("test-user".to_string(), pubkey_b64);

        let verified = verify_jwt(&jwt, &allowed_issuers).expect("verification should succeed");
        assert_eq!(verified.iss, "test-user");
        assert_eq!(verified.sub, "ztl-mcp");
        assert_eq!(
            verified.tools,
            vec!["search".to_string(), "get_page".to_string()]
        );
        assert_eq!(verified.scope, vec!["notes/**".to_string()]);
    }

    #[test]
    fn sign_empty_tools_and_scope() {
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();

        let claims = DelegateClaims {
            iss: "alice".into(),
            sub: "ztl-mcp".into(),
            aud: "vault".into(),
            iat: 500,
            exp: 0,
            tools: vec![],
            scope: vec![],
        };

        let jwt = sign_delegate_jwt(&signing_key, &claims).expect("signing should succeed");

        let pubkey_b64 = URL_SAFE_NO_PAD.encode(verifying_key.as_bytes());
        let mut allowed_issuers = HashMap::new();
        allowed_issuers.insert("alice".to_string(), pubkey_b64);

        let verified = verify_jwt(&jwt, &allowed_issuers).expect("verification should succeed");
        assert!(verified.tools.is_empty());
        assert!(verified.scope.is_empty());
    }

    #[test]
    fn parse_expiry_hours() {
        let ts = parse_expiry("1h").unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        // Should be approximately now + 3600 (within 2 seconds tolerance).
        assert!((ts as i64 - (now + 3600) as i64).unsigned_abs() < 2);
    }

    #[test]
    fn parse_expiry_days() {
        let ts = parse_expiry("7d").unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!((ts as i64 - (now + 7 * 86400) as i64).unsigned_abs() < 2);
    }

    #[test]
    fn parse_expiry_seconds() {
        let ts = parse_expiry("3600s").unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!((ts as i64 - (now + 3600) as i64).unsigned_abs() < 2);
    }

    #[test]
    fn parse_expiry_invalid() {
        assert!(parse_expiry("abc").is_err());
        assert!(parse_expiry("").is_err());
    }
}
