//! Agent token derivation and verification (REQ-020-004, REQ-020-019, CON-020-003).
//!
//! Token format: `base64url(user_id_bytes || generation_byte || ed25519_signature)`
//! where signature covers `b"zetl-agent-v1-" || user_id || generation`.
//!
//! Size: up to 64 bytes (user_id padded/truncated to 64) + 1 byte (generation) + 64 bytes (sig)
//! = 129 bytes → 172 base64url characters.
//!
//! The user_id is stored as UTF-8 bytes zero-padded to 64 bytes so the server
//! can extract it at a fixed offset. The generation counter enables token
//! rotation without changing the recovery mnemonic.

use anyhow::{anyhow, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ed25519_dalek::{Signer, Verifier};

/// Fixed size for the user_id field in the token (zero-padded UTF-8).
const USER_ID_LEN: usize = 64;

/// Domain separator prefix for the signed message.
const DOMAIN_PREFIX: &[u8] = b"zetl-agent-v1-";

/// Generate an agent token from a BIP39 mnemonic, user ID, and generation counter.
///
/// Returns the base64url-encoded token string suitable for `Authorization: Bearer <token>`.
pub fn generate_agent_token(mnemonic: &str, user_id: &str, generation: u8) -> Result<String> {
    let signing_key = super::recovery::derive_signing_key_from_mnemonic(mnemonic)?;

    let message = build_sign_message(user_id, generation);
    let signature = signing_key.sign(&message);

    // Assemble token: user_id_bytes (64) || generation (1) || signature (64)
    let mut token_bytes = Vec::with_capacity(USER_ID_LEN + 1 + 64);
    let mut id_buf = [0u8; USER_ID_LEN];
    let id_bytes = user_id.as_bytes();
    let copy_len = id_bytes.len().min(USER_ID_LEN);
    id_buf[..copy_len].copy_from_slice(&id_bytes[..copy_len]);
    token_bytes.extend_from_slice(&id_buf);
    token_bytes.push(generation);
    token_bytes.extend_from_slice(&signature.to_bytes());

    Ok(URL_SAFE_NO_PAD.encode(&token_bytes))
}

/// Decode and verify an agent token against a stored recovery public key.
///
/// Returns `Ok(TokenClaims)` if the token is valid, or an error describing why not.
pub fn verify_agent_token(
    token_b64: &str,
    recovery_pubkey_b64: &str,
    expected_generation: u8,
) -> Result<TokenClaims> {
    let token_bytes = URL_SAFE_NO_PAD
        .decode(token_b64)
        .map_err(|e| anyhow!("invalid base64url token: {e}"))?;

    let expected_len = USER_ID_LEN + 1 + 64;
    if token_bytes.len() != expected_len {
        return Err(anyhow!(
            "invalid token length: expected {expected_len}, got {}",
            token_bytes.len()
        ));
    }

    // Extract fields
    let id_buf = &token_bytes[..USER_ID_LEN];
    let generation = token_bytes[USER_ID_LEN];
    let sig_bytes = &token_bytes[USER_ID_LEN + 1..];

    // Recover user_id (strip trailing zero padding)
    let user_id = std::str::from_utf8(id_buf)
        .map_err(|e| anyhow!("invalid UTF-8 in user_id: {e}"))?
        .trim_end_matches('\0')
        .to_string();

    if user_id.is_empty() {
        return Err(anyhow!("empty user_id in token"));
    }

    // Check generation counter
    if generation != expected_generation {
        return Err(anyhow!(
            "token generation mismatch: token has {generation}, expected {expected_generation}"
        ));
    }

    // Decode public key
    let pubkey_bytes = URL_SAFE_NO_PAD
        .decode(recovery_pubkey_b64)
        .map_err(|e| anyhow!("invalid base64url pubkey: {e}"))?;

    if pubkey_bytes.len() != 32 {
        return Err(anyhow!(
            "invalid pubkey length: expected 32, got {}",
            pubkey_bytes.len()
        ));
    }

    let mut key_arr = [0u8; 32];
    key_arr.copy_from_slice(&pubkey_bytes);
    let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(&key_arr)
        .map_err(|e| anyhow!("invalid ed25519 public key: {e}"))?;

    // Verify signature
    let mut sig_arr = [0u8; 64];
    sig_arr.copy_from_slice(sig_bytes);
    let signature = ed25519_dalek::Signature::from_bytes(&sig_arr);

    let message = build_sign_message(&user_id, generation);
    verifying_key
        .verify(&message, &signature)
        .map_err(|_| anyhow!("invalid token signature"))?;

    Ok(TokenClaims { user_id, generation })
}

/// Extract the user_id from a token without verifying the signature.
///
/// Used by the middleware to look up the user profile before verification.
pub fn extract_user_id(token_b64: &str) -> Result<String> {
    let token_bytes = URL_SAFE_NO_PAD
        .decode(token_b64)
        .map_err(|e| anyhow!("invalid base64url token: {e}"))?;

    let expected_len = USER_ID_LEN + 1 + 64;
    if token_bytes.len() != expected_len {
        return Err(anyhow!("invalid token length"));
    }

    let id_buf = &token_bytes[..USER_ID_LEN];
    let user_id = std::str::from_utf8(id_buf)
        .map_err(|e| anyhow!("invalid UTF-8 in user_id: {e}"))?
        .trim_end_matches('\0')
        .to_string();

    if user_id.is_empty() {
        return Err(anyhow!("empty user_id in token"));
    }

    Ok(user_id)
}

/// Claims extracted from a verified agent token.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenClaims {
    pub user_id: String,
    pub generation: u8,
}

/// Build the message that gets signed: domain prefix + user_id + generation byte.
fn build_sign_message(user_id: &str, generation: u8) -> Vec<u8> {
    let mut msg = Vec::with_capacity(DOMAIN_PREFIX.len() + user_id.len() + 1);
    msg.extend_from_slice(DOMAIN_PREFIX);
    msg.extend_from_slice(user_id.as_bytes());
    msg.push(generation);
    msg
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::user::recovery::generate_recovery_keypair;

    #[test]
    fn generate_and_verify_roundtrip() {
        let kp = generate_recovery_keypair().unwrap();
        let user_id = "alice-a1b2c3d4";
        let generation = 0u8;

        let token = generate_agent_token(&kp.mnemonic, user_id, generation).unwrap();
        let claims =
            verify_agent_token(&token, &kp.recovery_pubkey, generation).unwrap();

        assert_eq!(claims.user_id, user_id);
        assert_eq!(claims.generation, generation);
    }

    #[test]
    fn wrong_generation_rejected() {
        let kp = generate_recovery_keypair().unwrap();
        let token = generate_agent_token(&kp.mnemonic, "alice-a1b2c3d4", 0).unwrap();

        let result = verify_agent_token(&token, &kp.recovery_pubkey, 1);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("generation mismatch"));
    }

    #[test]
    fn wrong_key_rejected() {
        let kp1 = generate_recovery_keypair().unwrap();
        let kp2 = generate_recovery_keypair().unwrap();
        let token = generate_agent_token(&kp1.mnemonic, "alice-a1b2c3d4", 0).unwrap();

        let result = verify_agent_token(&token, &kp2.recovery_pubkey, 0);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid token signature"));
    }

    #[test]
    fn extract_user_id_works() {
        let kp = generate_recovery_keypair().unwrap();
        let token = generate_agent_token(&kp.mnemonic, "bob-12345678", 0).unwrap();

        let user_id = extract_user_id(&token).unwrap();
        assert_eq!(user_id, "bob-12345678");
    }

    #[test]
    fn invalid_base64_rejected() {
        let result = verify_agent_token("not-valid-base64!!!", "dGVzdA", 0);
        assert!(result.is_err());
    }

    #[test]
    fn truncated_token_rejected() {
        let result = verify_agent_token("AAAA", "dGVzdA", 0);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid token length"));
    }

    #[test]
    fn generation_counter_rotation() {
        let kp = generate_recovery_keypair().unwrap();
        let user_id = "alice-a1b2c3d4";

        // Token at generation 0
        let token_g0 = generate_agent_token(&kp.mnemonic, user_id, 0).unwrap();
        // Token at generation 1
        let token_g1 = generate_agent_token(&kp.mnemonic, user_id, 1).unwrap();

        // Both are different tokens
        assert_ne!(token_g0, token_g1);

        // Each verifies only at its own generation
        assert!(verify_agent_token(&token_g0, &kp.recovery_pubkey, 0).is_ok());
        assert!(verify_agent_token(&token_g0, &kp.recovery_pubkey, 1).is_err());
        assert!(verify_agent_token(&token_g1, &kp.recovery_pubkey, 1).is_ok());
        assert!(verify_agent_token(&token_g1, &kp.recovery_pubkey, 0).is_err());
    }

    #[test]
    fn deterministic_token_generation() {
        let kp = generate_recovery_keypair().unwrap();
        let t1 = generate_agent_token(&kp.mnemonic, "alice-a1b2c3d4", 0).unwrap();
        let t2 = generate_agent_token(&kp.mnemonic, "alice-a1b2c3d4", 0).unwrap();
        assert_eq!(t1, t2, "same inputs must produce identical tokens");
    }
}
