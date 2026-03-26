//! BIP39 mnemonic generation and SLIP-0010 ed25519 key derivation for account
//! recovery (REQ-020-002, CON-020-002).
//!
//! The recovery flow:
//! 1. On account creation, generate a 12-word BIP39 mnemonic (128-bit entropy).
//! 2. Derive an ed25519 keypair via SLIP-0010 at path `m/44'/0'/0'`.
//! 3. Store only the public key in the user profile; display mnemonic once.
//! 4. Recovery: server issues a 256-bit challenge, client signs it with the
//!    derived private key, server verifies against stored pubkey.

use anyhow::{anyhow, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ed25519_dalek::{Signature, SigningKey, VerifyingKey};
use hmac::{Hmac, Mac};
use sha2_crate::Sha512;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Challenge timeout: 5 minutes (REQ-020-062).
const CHALLENGE_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Maximum active challenges per user (REQ-020-062).
const MAX_CHALLENGES_PER_USER: usize = 3;

type HmacSha512 = Hmac<Sha512>;

/// Result of BIP39 mnemonic generation: the mnemonic phrase and the derived
/// ed25519 public key (base64url-encoded, for storage in profile.json).
pub struct RecoveryKeypair {
    /// 12-word BIP39 mnemonic — display once, never store server-side.
    pub mnemonic: String,
    /// Base64url-encoded ed25519 public key.
    pub recovery_pubkey: String,
}

impl Drop for RecoveryKeypair {
    fn drop(&mut self) {
        // Zeroize the mnemonic in-place to avoid leaving secrets in memory.
        // SAFETY: we overwrite every byte of the String's buffer with zeros.
        // The String is left in a valid (all-NUL) state before deallocation.
        let bytes = unsafe { self.mnemonic.as_mut_vec() };
        for b in bytes.iter_mut() {
            // Use write_volatile to prevent the compiler from optimising this out.
            unsafe { std::ptr::write_volatile(b, 0) };
        }
    }
}

/// Generate a 12-word BIP39 mnemonic and derive the ed25519 keypair via
/// SLIP-0010 at path `m/44'/0'/0'`.
pub fn generate_recovery_keypair() -> Result<RecoveryKeypair> {
    let mnemonic = bip39::Mnemonic::generate(12)
        .map_err(|e| anyhow!("BIP39 generation failed: {e}"))?;

    let mnemonic_phrase = mnemonic.to_string();
    let pubkey = derive_pubkey_from_mnemonic(&mnemonic_phrase)?;
    let recovery_pubkey = URL_SAFE_NO_PAD.encode(pubkey.as_bytes());

    Ok(RecoveryKeypair {
        mnemonic: mnemonic_phrase,
        recovery_pubkey,
    })
}

/// Derive the ed25519 public key from a BIP39 mnemonic via SLIP-0010.
pub fn derive_pubkey_from_mnemonic(mnemonic_phrase: &str) -> Result<VerifyingKey> {
    let signing_key = derive_signing_key_from_mnemonic(mnemonic_phrase)?;
    Ok(signing_key.verifying_key())
}

/// Derive the ed25519 signing key from a BIP39 mnemonic via SLIP-0010
/// at path `m/44'/0'/0'`.
pub(crate) fn derive_signing_key_from_mnemonic(mnemonic_phrase: &str) -> Result<SigningKey> {
    let mnemonic: bip39::Mnemonic = mnemonic_phrase
        .parse()
        .map_err(|e| anyhow!("invalid BIP39 mnemonic: {e}"))?;

    // BIP39: mnemonic → seed (no passphrase)
    let seed = mnemonic.to_seed("");

    // SLIP-0010 ed25519 master key derivation
    let (master_key, master_chain) = slip0010_master_key(&seed)?;

    // Derive m/44'/0'/0' (three hardened children)
    let (key1, chain1) = slip0010_derive_child(&master_key, &master_chain, 44 + 0x80000000)?;
    let (key2, chain2) = slip0010_derive_child(&key1, &chain1, 0 + 0x80000000)?;
    let (key3, _) = slip0010_derive_child(&key2, &chain2, 0 + 0x80000000)?;

    let signing_key = SigningKey::from_bytes(&key3);
    Ok(signing_key)
}

/// SLIP-0010: Derive master key and chain code from seed.
///
/// `I = HMAC-SHA512(Key = "ed25519 seed", Data = seed)`
/// Master secret key = I_L (left 32 bytes), chain code = I_R (right 32 bytes).
fn slip0010_master_key(seed: &[u8]) -> Result<([u8; 32], [u8; 32])> {
    let mut mac = HmacSha512::new_from_slice(b"ed25519 seed")
        .map_err(|_| anyhow!("HMAC key error"))?;
    mac.update(seed);
    let result = mac.finalize().into_bytes();

    let mut key = [0u8; 32];
    let mut chain = [0u8; 32];
    key.copy_from_slice(&result[..32]);
    chain.copy_from_slice(&result[32..]);
    Ok((key, chain))
}

/// SLIP-0010: Derive a hardened child key.
///
/// For ed25519, only hardened derivation is defined:
/// `I = HMAC-SHA512(Key = chain_code, Data = 0x00 || key || ser32(index))`
fn slip0010_derive_child(
    key: &[u8; 32],
    chain_code: &[u8; 32],
    index: u32,
) -> Result<([u8; 32], [u8; 32])> {
    let mut mac = HmacSha512::new_from_slice(chain_code)
        .map_err(|_| anyhow!("HMAC key error"))?;
    mac.update(&[0x00]);
    mac.update(key);
    mac.update(&index.to_be_bytes());
    let result = mac.finalize().into_bytes();

    let mut child_key = [0u8; 32];
    let mut child_chain = [0u8; 32];
    child_key.copy_from_slice(&result[..32]);
    child_chain.copy_from_slice(&result[32..]);
    Ok((child_key, child_chain))
}

/// Sign a challenge with the ed25519 key derived from a mnemonic.
///
/// Used client-side: the user provides their mnemonic, we derive the key and
/// sign the server's challenge.
pub fn sign_challenge(mnemonic_phrase: &str, challenge: &[u8]) -> Result<Vec<u8>> {
    use ed25519_dalek::Signer;
    let signing_key = derive_signing_key_from_mnemonic(mnemonic_phrase)?;
    let signature = signing_key.sign(challenge);
    Ok(signature.to_bytes().to_vec())
}

/// Verify a signed challenge against a stored recovery public key.
///
/// - `recovery_pubkey_b64`: base64url-encoded ed25519 public key from profile.
/// - `challenge`: the original 256-bit nonce.
/// - `signature_bytes`: the 64-byte ed25519 signature.
pub fn verify_challenge(
    recovery_pubkey_b64: &str,
    challenge: &[u8],
    signature_bytes: &[u8],
) -> Result<bool> {
    use ed25519_dalek::Verifier;

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
    let verifying_key = VerifyingKey::from_bytes(&key_arr)
        .map_err(|e| anyhow!("invalid ed25519 public key: {e}"))?;

    if signature_bytes.len() != 64 {
        return Ok(false);
    }

    let mut sig_arr = [0u8; 64];
    sig_arr.copy_from_slice(signature_bytes);
    let signature = Signature::from_bytes(&sig_arr);

    Ok(verifying_key.verify(challenge, &signature).is_ok())
}

// -- Recovery Challenge Store --

/// A pending recovery challenge.
struct PendingChallenge {
    challenge: [u8; 32],
    issued_at: Instant,
}

/// In-memory store for recovery challenges (CON-020-002, REQ-020-062).
pub struct RecoveryChallengeStore {
    /// Maps user_id → list of pending challenges.
    challenges: Mutex<HashMap<String, Vec<PendingChallenge>>>,
}

impl RecoveryChallengeStore {
    pub fn new() -> Self {
        Self {
            challenges: Mutex::new(HashMap::new()),
        }
    }

    /// Issue a new 256-bit challenge for the given user.
    ///
    /// Returns the challenge bytes. Enforces the max-3-per-user limit and
    /// prunes expired challenges lazily.
    pub fn issue_challenge(&self, user_id: &str) -> Result<[u8; 32]> {
        let mut store = self
            .challenges
            .lock()
            .map_err(|_| anyhow!("challenge store lock poisoned"))?;

        // Prune expired challenges for this user
        let now = Instant::now();
        if let Some(challenges) = store.get_mut(user_id) {
            challenges.retain(|c| now.duration_since(c.issued_at) < CHALLENGE_TIMEOUT);
        }

        // Enforce max active challenges per user
        let count = store.get(user_id).map_or(0, |v| v.len());
        if count >= MAX_CHALLENGES_PER_USER {
            return Err(anyhow!("too many active recovery challenges for user"));
        }

        // Generate 256-bit challenge
        let mut challenge = [0u8; 32];
        crate::user::getrandom(&mut challenge);

        store
            .entry(user_id.to_string())
            .or_default()
            .push(PendingChallenge {
                challenge,
                issued_at: Instant::now(),
            });

        Ok(challenge)
    }

    /// Consume and validate a challenge for the given user.
    ///
    /// Returns `true` if a matching, non-expired challenge was found and removed.
    /// Returns 410 Gone equivalent if expired.
    pub fn consume_challenge(
        &self,
        user_id: &str,
        challenge: &[u8; 32],
    ) -> Result<ChallengeResult> {
        let mut store = self
            .challenges
            .lock()
            .map_err(|_| anyhow!("challenge store lock poisoned"))?;

        let challenges = match store.get_mut(user_id) {
            Some(c) => c,
            None => return Ok(ChallengeResult::NotFound),
        };

        // Find matching challenge
        let pos = challenges.iter().position(|c| super::constant_time_eq(&c.challenge, challenge));
        match pos {
            Some(idx) => {
                let pending = challenges.remove(idx);
                if Instant::now().duration_since(pending.issued_at) >= CHALLENGE_TIMEOUT {
                    Ok(ChallengeResult::Expired)
                } else {
                    Ok(ChallengeResult::Valid)
                }
            }
            None => Ok(ChallengeResult::NotFound),
        }
    }
}

/// Outcome of consuming a recovery challenge.
#[derive(Debug, PartialEq)]
pub enum ChallengeResult {
    Valid,
    Expired,
    NotFound,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_recovery_keypair() {
        let kp = generate_recovery_keypair().unwrap();
        // 12-word mnemonic
        assert_eq!(kp.mnemonic.split_whitespace().count(), 12);
        // pubkey is valid base64url
        let decoded = URL_SAFE_NO_PAD.decode(&kp.recovery_pubkey).unwrap();
        assert_eq!(decoded.len(), 32);
    }

    #[test]
    fn test_mnemonic_derives_consistent_pubkey() {
        let kp = generate_recovery_keypair().unwrap();
        let pubkey1 = derive_pubkey_from_mnemonic(&kp.mnemonic).unwrap();
        let pubkey2 = derive_pubkey_from_mnemonic(&kp.mnemonic).unwrap();
        assert_eq!(pubkey1.as_bytes(), pubkey2.as_bytes());
    }

    #[test]
    fn test_sign_and_verify_challenge() {
        let kp = generate_recovery_keypair().unwrap();
        let challenge = b"test-challenge-256-bits-padding!!";

        let signature = sign_challenge(&kp.mnemonic, challenge).unwrap();
        let valid =
            verify_challenge(&kp.recovery_pubkey, challenge, &signature).unwrap();
        assert!(valid, "signature should verify against recovery pubkey");
    }

    #[test]
    fn test_verify_wrong_challenge() {
        let kp = generate_recovery_keypair().unwrap();
        let challenge = b"test-challenge-256-bits-padding!!";
        let wrong = b"wrong-challenge-256-bits-paddin!!";

        let signature = sign_challenge(&kp.mnemonic, challenge).unwrap();
        let valid = verify_challenge(&kp.recovery_pubkey, wrong, &signature).unwrap();
        assert!(!valid, "signature should not verify against wrong challenge");
    }

    #[test]
    fn test_verify_wrong_key() {
        let kp1 = generate_recovery_keypair().unwrap();
        let kp2 = generate_recovery_keypair().unwrap();
        let challenge = b"test-challenge-256-bits-padding!!";

        let signature = sign_challenge(&kp1.mnemonic, challenge).unwrap();
        let valid =
            verify_challenge(&kp2.recovery_pubkey, challenge, &signature).unwrap();
        assert!(!valid, "signature should not verify against different key");
    }

    #[test]
    fn test_challenge_store_issue_and_consume() {
        let store = RecoveryChallengeStore::new();
        let challenge = store.issue_challenge("alice").unwrap();
        assert_eq!(challenge.len(), 32);

        let result = store.consume_challenge("alice", &challenge).unwrap();
        assert_eq!(result, ChallengeResult::Valid);

        // Consuming again should fail (challenge removed)
        let result = store.consume_challenge("alice", &challenge).unwrap();
        assert_eq!(result, ChallengeResult::NotFound);
    }

    #[test]
    fn test_challenge_store_max_per_user() {
        let store = RecoveryChallengeStore::new();
        store.issue_challenge("alice").unwrap();
        store.issue_challenge("alice").unwrap();
        store.issue_challenge("alice").unwrap();
        // 4th should fail
        let err = store.issue_challenge("alice");
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("too many"));
    }

    #[test]
    fn test_challenge_store_different_users_independent() {
        let store = RecoveryChallengeStore::new();
        let c1 = store.issue_challenge("alice").unwrap();
        let c2 = store.issue_challenge("bob").unwrap();

        assert_eq!(
            store.consume_challenge("alice", &c1).unwrap(),
            ChallengeResult::Valid
        );
        assert_eq!(
            store.consume_challenge("bob", &c2).unwrap(),
            ChallengeResult::Valid
        );
    }

    #[test]
    fn test_challenge_not_found_for_wrong_user() {
        let store = RecoveryChallengeStore::new();
        let challenge = store.issue_challenge("alice").unwrap();
        let result = store.consume_challenge("bob", &challenge).unwrap();
        assert_eq!(result, ChallengeResult::NotFound);
    }

    #[test]
    fn test_invalid_mnemonic_rejected() {
        let result = derive_pubkey_from_mnemonic("not a valid mnemonic phrase");
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_bad_signature_length() {
        let kp = generate_recovery_keypair().unwrap();
        let challenge = b"test-challenge-256-bits-padding!!";
        // Too short
        let valid = verify_challenge(&kp.recovery_pubkey, challenge, &[0u8; 32]).unwrap();
        assert!(!valid);
    }

    #[test]
    fn test_known_mnemonic_deterministic() {
        // Using a fixed mnemonic to verify deterministic derivation
        let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let pubkey1 = derive_pubkey_from_mnemonic(mnemonic).unwrap();
        let pubkey2 = derive_pubkey_from_mnemonic(mnemonic).unwrap();
        assert_eq!(
            pubkey1.as_bytes(),
            pubkey2.as_bytes(),
            "same mnemonic must produce same key"
        );
    }
}
