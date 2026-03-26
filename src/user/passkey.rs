//! WebAuthn passkey registration and authentication (SPEC-020, REQ-020-002).
//!
//! Uses `webauthn-rs` for challenge generation and verification, with credential
//! storage in `.zetl/users/<id>/passkeys.json` alongside the user profile.

use anyhow::{anyhow, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use url::Url;
use uuid::Uuid;
use webauthn_rs::prelude::*;
use webauthn_rs::WebauthnBuilder;

use super::WebAuthnCredential;

/// Default challenge timeout: 5 minutes (WebAuthn spec recommendation).
const CHALLENGE_TIMEOUT_SECS: u64 = 300;

/// Filename for stored webauthn-rs Passkey credentials.
const PASSKEYS_FILE: &str = "passkeys.json";

/// Manages WebAuthn passkey registration and authentication flows.
///
/// Holds the configured `Webauthn` relying party instance and in-flight
/// challenge state for registration and authentication ceremonies.
pub struct PasskeyManager {
    webauthn: Webauthn,
    reg_state: Mutex<HashMap<String, (PasskeyRegistration, Instant)>>,
    auth_state: Mutex<HashMap<String, (PasskeyAuthentication, Instant)>>,
    challenge_timeout: Duration,
}

/// Container for serialized passkey credentials on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredPasskeys {
    pub passkeys: Vec<Passkey>,
}

impl PasskeyManager {
    /// Create a new PasskeyManager for the given relying party.
    ///
    /// - `rp_id`: The relying party identifier (e.g. "localhost" or a domain).
    /// - `rp_origin`: The origin URL (e.g. "http://localhost:3000").
    /// - `rp_name`: Human-readable relying party name (e.g. "zetl vault").
    pub fn new(rp_id: &str, rp_origin: &str, rp_name: &str) -> Result<Self> {
        let origin =
            Url::parse(rp_origin).with_context(|| format!("invalid rp_origin URL: {rp_origin}"))?;

        let builder = WebauthnBuilder::new(rp_id, &origin)
            .map_err(|e| anyhow!("WebauthnBuilder error: {e}"))?
            .rp_name(rp_name);

        let webauthn = builder
            .build()
            .map_err(|e| anyhow!("Webauthn build error: {e}"))?;

        Ok(Self {
            webauthn,
            reg_state: Mutex::new(HashMap::new()),
            auth_state: Mutex::new(HashMap::new()),
            challenge_timeout: Duration::from_secs(CHALLENGE_TIMEOUT_SECS),
        })
    }

    /// Begin passkey registration for a user.
    ///
    /// Returns the `CreationChallengeResponse` to send to the client's WebAuthn
    /// API. The challenge state is held in memory until `finish_registration` is
    /// called or it expires.
    pub fn start_registration(
        &self,
        user_id: &str,
        user_name: &str,
        existing_passkeys: &[Passkey],
    ) -> Result<CreationChallengeResponse> {
        let uuid = user_id_to_uuid(user_id);

        let exclude: Option<Vec<CredentialID>> = if existing_passkeys.is_empty() {
            None
        } else {
            Some(
                existing_passkeys
                    .iter()
                    .map(|pk| pk.cred_id().clone())
                    .collect(),
            )
        };

        let (ccr, reg_state) = self
            .webauthn
            .start_passkey_registration(uuid, user_name, user_name, exclude)
            .map_err(|e| anyhow!("start_passkey_registration failed: {e}"))?;

        self.reg_state
            .lock()
            .map_err(|_| anyhow!("registration state lock poisoned"))?
            .insert(user_id.to_string(), (reg_state, Instant::now()));

        self.purge_expired_reg();

        Ok(ccr)
    }

    /// Complete passkey registration with the authenticator's response.
    ///
    /// Returns the new `Passkey` credential to be stored. The caller should
    /// persist this via `save_passkeys` and update the user profile's
    /// `credentials` list.
    pub fn finish_registration(
        &self,
        user_id: &str,
        response: &RegisterPublicKeyCredential,
    ) -> Result<Passkey> {
        let (reg_state, created_at) = self
            .reg_state
            .lock()
            .map_err(|_| anyhow!("registration state lock poisoned"))?
            .remove(user_id)
            .ok_or_else(|| anyhow!("no pending registration for user {user_id}"))?;

        if created_at.elapsed() > self.challenge_timeout {
            return Err(anyhow!("registration challenge expired"));
        }

        let passkey = self
            .webauthn
            .finish_passkey_registration(response, &reg_state)
            .map_err(|e| anyhow!("finish_passkey_registration failed: {e}"))?;

        Ok(passkey)
    }

    /// Begin passkey authentication for a user.
    ///
    /// Returns the `RequestChallengeResponse` to send to the client. The user
    /// must have at least one registered passkey.
    pub fn start_authentication(
        &self,
        user_id: &str,
        passkeys: &[Passkey],
    ) -> Result<RequestChallengeResponse> {
        if passkeys.is_empty() {
            return Err(anyhow!("user {user_id} has no registered passkeys"));
        }

        let (rcr, auth_state) = self
            .webauthn
            .start_passkey_authentication(passkeys)
            .map_err(|e| anyhow!("start_passkey_authentication failed: {e}"))?;

        self.auth_state
            .lock()
            .map_err(|_| anyhow!("authentication state lock poisoned"))?
            .insert(user_id.to_string(), (auth_state, Instant::now()));

        self.purge_expired_auth();

        Ok(rcr)
    }

    /// Complete passkey authentication with the authenticator's response.
    ///
    /// Returns the `AuthenticationResult` which includes the updated sign
    /// counter. The caller should update the stored passkey via
    /// `Passkey::update_credential`.
    pub fn finish_authentication(
        &self,
        user_id: &str,
        response: &PublicKeyCredential,
    ) -> Result<AuthenticationResult> {
        let (auth_state, created_at) = self
            .auth_state
            .lock()
            .map_err(|_| anyhow!("authentication state lock poisoned"))?
            .remove(user_id)
            .ok_or_else(|| anyhow!("no pending authentication for user {user_id}"))?;

        if created_at.elapsed() > self.challenge_timeout {
            return Err(anyhow!("authentication challenge expired"));
        }

        let auth_result = self
            .webauthn
            .finish_passkey_authentication(response, &auth_state)
            .map_err(|e| anyhow!("finish_passkey_authentication failed: {e}"))?;

        Ok(auth_result)
    }

    fn purge_expired_reg(&self) {
        if let Ok(mut state) = self.reg_state.lock() {
            state.retain(|_, (_, t)| t.elapsed() < self.challenge_timeout);
        }
    }

    fn purge_expired_auth(&self) {
        if let Ok(mut state) = self.auth_state.lock() {
            state.retain(|_, (_, t)| t.elapsed() < self.challenge_timeout);
        }
    }
}

// -- UUID derivation --

/// Deterministically derive a UUID from a zetl user ID using BLAKE3.
///
/// WebAuthn requires a stable `Uuid` for each user. We hash the user ID
/// to produce one, setting version-4 and variant-1 bits for format compliance.
pub fn user_id_to_uuid(user_id: &str) -> Uuid {
    let hash = blake3::hash(user_id.as_bytes());
    let bytes = hash.as_bytes();
    let mut uuid_bytes = [0u8; 16];
    uuid_bytes.copy_from_slice(&bytes[..16]);
    // Set UUID version 4 and variant 1 bits
    uuid_bytes[6] = (uuid_bytes[6] & 0x0f) | 0x40;
    uuid_bytes[8] = (uuid_bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(uuid_bytes)
}

// -- Credential storage --

/// Load stored passkeys for a user from `.zetl/users/<id>/passkeys.json`.
pub fn load_passkeys(vault_root: &Path, user_id: &str) -> Result<Vec<Passkey>> {
    let path = super::user_dir(vault_root, user_id).join(PASSKEYS_FILE);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read passkeys: {}", path.display()))?;
    let stored: StoredPasskeys = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse passkeys: {}", path.display()))?;
    Ok(stored.passkeys)
}

/// Save passkeys for a user to `.zetl/users/<id>/passkeys.json`.
pub fn save_passkeys(vault_root: &Path, user_id: &str, passkeys: &[Passkey]) -> Result<()> {
    let dir = super::user_dir(vault_root, user_id);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create user directory: {}", dir.display()))?;

    let stored = StoredPasskeys {
        passkeys: passkeys.to_vec(),
    };
    let json = serde_json::to_string_pretty(&stored).context("failed to serialize passkeys")?;

    let path = dir.join(PASSKEYS_FILE);
    std::fs::write(&path, &json)
        .with_context(|| format!("failed to write passkeys: {}", path.display()))?;

    Ok(())
}

/// Convert a webauthn-rs `Passkey` to our profile-level `WebAuthnCredential`.
///
/// Extracts the credential ID and public key fingerprint from the serialized
/// passkey data for the human-readable profile summary.
pub fn passkey_to_credential(
    passkey: &Passkey,
    label: Option<String>,
    created_at: &str,
) -> WebAuthnCredential {
    // Credential ID: base64url-encode the raw bytes
    let cred_id_str = URL_SAFE_NO_PAD.encode(passkey.cred_id());

    // Public key: BLAKE3 fingerprint of the serialized passkey (the full COSE
    // key lives in passkeys.json; this is a display-only summary).
    let pk_fingerprint = match serde_json::to_vec(passkey) {
        Ok(bytes) => {
            let hash = blake3::hash(&bytes);
            hash.to_hex().as_str()[..32].to_string()
        }
        Err(_) => "unknown".to_string(),
    };

    // Extract sign_count from the serialized representation
    let sign_count = serde_json::to_value(passkey)
        .ok()
        .and_then(|v| v.get("counter").and_then(|c| c.as_u64()))
        .unwrap_or(0) as u32;

    WebAuthnCredential {
        credential_id: cred_id_str,
        public_key: pk_fingerprint,
        sign_count,
        created_at: created_at.to_string(),
        label,
    }
}

/// Update the sign counter on stored passkeys after a successful authentication.
///
/// Applies the `AuthenticationResult` to each matching passkey and persists
/// the updated credentials. Returns `true` if the counter was updated
/// normally, `false` if a possible credential clone was detected.
pub fn update_passkey_after_auth(
    vault_root: &Path,
    user_id: &str,
    auth_result: &AuthenticationResult,
) -> Result<bool> {
    let mut passkeys = load_passkeys(vault_root, user_id)?;
    let mut counter_ok = true;

    for pk in &mut passkeys {
        match pk.update_credential(auth_result) {
            Some(true) => {} // counter updated normally
            Some(false) => {
                counter_ok = false; // possible clone
            }
            None => {} // credential didn't match this result
        }
    }

    save_passkeys(vault_root, user_id, &passkeys)?;
    Ok(counter_ok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_user_id_to_uuid_deterministic() {
        let uuid1 = user_id_to_uuid("alice-a1b2c3d4");
        let uuid2 = user_id_to_uuid("alice-a1b2c3d4");
        assert_eq!(uuid1, uuid2, "same user ID should produce same UUID");
    }

    #[test]
    fn test_user_id_to_uuid_unique() {
        let uuid1 = user_id_to_uuid("alice-a1b2c3d4");
        let uuid2 = user_id_to_uuid("bob-e5f6g7h8");
        assert_ne!(
            uuid1, uuid2,
            "different user IDs should produce different UUIDs"
        );
    }

    #[test]
    fn test_user_id_to_uuid_version_variant() {
        let uuid = user_id_to_uuid("test-user-12345678");
        let bytes = uuid.as_bytes();
        // Version 4: byte 6 high nibble is 0x4
        assert_eq!(bytes[6] >> 4, 4);
        // Variant 1: byte 8 high 2 bits are 0b10
        assert_eq!(bytes[8] >> 6, 2);
    }

    #[test]
    fn test_passkey_manager_creation() {
        let mgr = PasskeyManager::new("localhost", "http://localhost:3000", "zetl vault");
        assert!(mgr.is_ok(), "PasskeyManager should be constructible");
    }

    #[test]
    fn test_passkey_manager_invalid_origin() {
        let mgr = PasskeyManager::new("localhost", "not-a-url", "zetl vault");
        assert!(mgr.is_err(), "invalid origin should fail");
    }

    #[test]
    fn test_start_registration() {
        let mgr = PasskeyManager::new("localhost", "http://localhost:3000", "zetl vault").unwrap();
        let ccr = mgr.start_registration("alice-a1b2c3d4", "Alice", &[]);
        assert!(ccr.is_ok(), "start_registration should succeed: {ccr:?}");
    }

    #[test]
    fn test_finish_registration_no_pending() {
        let mgr = PasskeyManager::new("localhost", "http://localhost:3000", "zetl vault").unwrap();

        // Create a dummy RegisterPublicKeyCredential
        let response: RegisterPublicKeyCredential = serde_json::from_str(DUMMY_REG_RESPONSE)
            .unwrap_or_else(|_| {
                // If we can't parse, just test that finish_registration fails with no pending state
                panic!("dummy response parse failed — test needs valid JSON fixture");
            });

        let result = mgr.finish_registration("alice-a1b2c3d4", &response);
        assert!(result.is_err(), "should fail with no pending registration");
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("no pending registration"),);
    }

    #[test]
    fn test_start_authentication_no_passkeys() {
        let mgr = PasskeyManager::new("localhost", "http://localhost:3000", "zetl vault").unwrap();
        let result = mgr.start_authentication("alice-a1b2c3d4", &[]);
        assert!(result.is_err(), "should fail with no passkeys");
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("no registered passkeys"),);
    }

    #[test]
    fn test_finish_authentication_no_pending() {
        let mgr = PasskeyManager::new("localhost", "http://localhost:3000", "zetl vault").unwrap();

        let response: PublicKeyCredential = serde_json::from_str(DUMMY_AUTH_RESPONSE)
            .unwrap_or_else(|_| {
                panic!("dummy response parse failed — test needs valid JSON fixture");
            });

        let result = mgr.finish_authentication("alice-a1b2c3d4", &response);
        assert!(
            result.is_err(),
            "should fail with no pending authentication"
        );
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("no pending authentication"),);
    }

    #[test]
    fn test_save_and_load_passkeys_empty() {
        let tmp = TempDir::new().unwrap();
        let user_id = "alice-a1b2c3d4";

        // No passkeys file yet
        let loaded = load_passkeys(tmp.path(), user_id).unwrap();
        assert!(loaded.is_empty());

        // Save empty list
        save_passkeys(tmp.path(), user_id, &[]).unwrap();
        let loaded = load_passkeys(tmp.path(), user_id).unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn test_stored_passkeys_serialization() {
        let stored = StoredPasskeys { passkeys: vec![] };
        let json = serde_json::to_string(&stored).unwrap();
        assert!(json.contains("\"passkeys\":[]"));

        let back: StoredPasskeys = serde_json::from_str(&json).unwrap();
        assert!(back.passkeys.is_empty());
    }

    // Minimal JSON fixtures for type-level testing (not cryptographically valid).
    const DUMMY_REG_RESPONSE: &str = r#"{
        "id": "AAAA",
        "rawId": "AAAA",
        "type": "public-key",
        "response": {
            "attestationObject": "o2NmbXRkbm9uZWdhdHRTdG10oGhhdXRoRGF0YVikSZYN5YgOjGh0NBcPZHZgW4_krrmihjLHmVzzuoMdl2NFAAAAAAAAAAAAAAAAAAAAAAAAAAAAIQAB",
            "clientDataJSON": "eyJ0eXBlIjoid2ViYXV0aG4uY3JlYXRlIiwiY2hhbGxlbmdlIjoiQUFBQSIsIm9yaWdpbiI6Imh0dHA6Ly9sb2NhbGhvc3Q6MzAwMCJ9"
        }
    }"#;

    const DUMMY_AUTH_RESPONSE: &str = r#"{
        "id": "AAAA",
        "rawId": "AAAA",
        "type": "public-key",
        "response": {
            "authenticatorData": "SZYN5YgOjGh0NBcPZHZgW4_krrmihjLHmVzzuoMdl2MFAAAAAA",
            "clientDataJSON": "eyJ0eXBlIjoid2ViYXV0aG4uZ2V0IiwiY2hhbGxlbmdlIjoiQUFBQSIsIm9yaWdpbiI6Imh0dHA6Ly9sb2NhbGhvc3Q6MzAwMCJ9",
            "signature": "AAAA",
            "userHandle": "AAAA"
        }
    }"#;
}
