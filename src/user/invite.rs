//! Invitation token generation for multi-user collaborative editing (REQ-020-006).
//!
//! Generates JWT invitation tokens signed with EdDSA (ed25519). The server's
//! signing key is stored at `.ztl/collab/server.key` and created on first use.
//! Nonces are tracked in `.ztl/collab/used-nonces.json` to enforce single-use.

use anyhow::{Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const COLLAB_DIR: &str = ".ztl/collab";
const SERVER_KEY_FILE: &str = "server.key";
const USED_NONCES_FILE: &str = "used-nonces.json";
const PENDING_INVITES_FILE: &str = "pending-invites.json";

/// Default invitation expiry: 72 hours.
const DEFAULT_EXPIRY_SECS: u64 = 72 * 60 * 60;

/// JWT header for EdDSA-signed invitation tokens (CON-020-004).
#[derive(Serialize)]
struct JwtHeader {
    alg: &'static str,
    typ: &'static str,
}

/// JWT payload for invitation tokens (CON-020-004).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteClaims {
    /// Inviter user ID.
    pub iss: String,
    /// Always "ztl-invite".
    pub sub: String,
    /// Role for the invitee: reader, editor, or admin.
    pub role: String,
    /// Optional glob pattern constraining the invitee's initial page scope.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pages: Option<String>,
    /// Expiry as Unix timestamp.
    pub exp: u64,
    /// 128-bit random nonce (hex-encoded) for single-use enforcement.
    pub nonce: String,
}

/// A used nonce entry with its expiry timestamp.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct UsedNonce {
    nonce: String,
    exp: u64,
}

/// Return the collab directory path for a vault.
fn collab_dir(vault_root: &Path) -> PathBuf {
    vault_root.join(COLLAB_DIR)
}

/// Return the server key file path.
fn server_key_path(vault_root: &Path) -> PathBuf {
    collab_dir(vault_root).join(SERVER_KEY_FILE)
}

/// Return the used-nonces file path.
fn used_nonces_path(vault_root: &Path) -> PathBuf {
    collab_dir(vault_root).join(USED_NONCES_FILE)
}

/// Load or create the server's ed25519 signing key.
///
/// On first call, generates a new key and writes it to `.ztl/collab/server.key`
/// with 0600 permissions. Subsequent calls load the existing key.
pub fn load_or_create_server_key(vault_root: &Path) -> Result<SigningKey> {
    let path = server_key_path(vault_root);

    if path.exists() {
        // Verify file permissions are not too permissive (REQ-020 server key protection)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::metadata(&path)
                .with_context(|| format!("failed to read metadata: {}", path.display()))?
                .permissions();
            let mode = perms.mode() & 0o777;
            if mode & 0o077 != 0 {
                anyhow::bail!(
                    "server key {} has insecure permissions {:04o} (expected 0600). \
                     Fix with: chmod 600 {}",
                    path.display(),
                    mode,
                    path.display()
                );
            }
        }

        let bytes = fs::read(&path)
            .with_context(|| format!("failed to read server key: {}", path.display()))?;
        if bytes.len() != 32 {
            anyhow::bail!(
                "server key has invalid length ({} bytes, expected 32): {}",
                bytes.len(),
                path.display()
            );
        }
        let mut key_bytes = [0u8; 32];
        key_bytes.copy_from_slice(&bytes);
        Ok(SigningKey::from_bytes(&key_bytes))
    } else {
        // Generate a new key
        let dir = collab_dir(vault_root);
        fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create collab directory: {}", dir.display()))?;

        let mut key_bytes = [0u8; 32];
        super::getrandom(&mut key_bytes);
        let key = SigningKey::from_bytes(&key_bytes);

        fs::write(&path, key_bytes)
            .with_context(|| format!("failed to write server key: {}", path.display()))?;

        // Set file permissions to 0600 (owner read/write only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
                .with_context(|| format!("failed to set permissions on: {}", path.display()))?;
        }

        Ok(key)
    }
}

/// Load the server key from a BIP39 seed phrase, or fall back to file-based
/// load/create. When a seed is provided, the key is also written to disk so
/// that code paths using `load_or_create_server_key` directly (e.g. comment
/// verification in routes) find a consistent key.
pub fn load_or_derive_server_key(
    vault_root: &Path,
    seed_phrase: Option<&str>,
) -> Result<SigningKey> {
    match seed_phrase {
        Some(mnemonic) => {
            let key = super::recovery::derive_server_key_from_mnemonic(mnemonic)?;

            // Write to disk so other code paths find the same key
            let dir = collab_dir(vault_root);
            fs::create_dir_all(&dir)
                .with_context(|| format!("failed to create collab directory: {}", dir.display()))?;
            let path = server_key_path(vault_root);
            fs::write(&path, key.to_bytes())
                .with_context(|| format!("failed to write server key: {}", path.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
            }

            Ok(key)
        }
        None => load_or_create_server_key(vault_root),
    }
}

/// Get the server's public verifying key.
pub fn server_verifying_key(vault_root: &Path) -> Result<VerifyingKey> {
    let signing_key = load_or_create_server_key(vault_root)?;
    Ok(VerifyingKey::from(&signing_key))
}

/// Generate a 128-bit random nonce as a 32-character hex string.
fn generate_nonce() -> String {
    let mut bytes = [0u8; 16];
    super::getrandom(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Generate an invitation JWT token (REQ-020-006, CON-020-004).
///
/// Returns the signed JWT string and the nonce used.
pub fn generate_invitation(
    vault_root: &Path,
    inviter_id: &str,
    role: &str,
    pages: Option<&str>,
    expires_secs: Option<u64>,
) -> Result<(String, String)> {
    let signing_key = load_or_create_server_key(vault_root)?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let expiry = expires_secs.unwrap_or(DEFAULT_EXPIRY_SECS);
    let exp = now + expiry;
    let nonce = generate_nonce();

    let claims = InviteClaims {
        iss: inviter_id.to_string(),
        sub: "ztl-invite".to_string(),
        role: role.to_string(),
        pages: pages.map(|s| s.to_string()),
        exp,
        nonce: nonce.clone(),
    };

    let jwt = encode_jwt(&signing_key, &claims)?;

    // NOTE: Do NOT record the nonce here — nonces are only marked as used
    // when the invitation is accepted (mark_nonce_used).  Recording at
    // generation time would make every fresh invite appear "already used".

    Ok((jwt, nonce))
}

/// Encode a JWT with EdDSA signature.
fn encode_jwt(key: &SigningKey, claims: &InviteClaims) -> Result<String> {
    let header = JwtHeader {
        alg: "EdDSA",
        typ: "JWT",
    };

    let header_json = serde_json::to_vec(&header).context("failed to serialize JWT header")?;
    let payload_json = serde_json::to_vec(claims).context("failed to serialize JWT payload")?;

    let header_b64 = URL_SAFE_NO_PAD.encode(&header_json);
    let payload_b64 = URL_SAFE_NO_PAD.encode(&payload_json);

    let signing_input = format!("{header_b64}.{payload_b64}");
    let signature = key.sign(signing_input.as_bytes());
    let sig_b64 = URL_SAFE_NO_PAD.encode(signature.to_bytes());

    Ok(format!("{signing_input}.{sig_b64}"))
}

/// Decode and verify a JWT, returning the claims if valid.
pub fn decode_jwt(vault_root: &Path, token: &str) -> Result<InviteClaims> {
    let verifying_key = server_verifying_key(vault_root)?;
    decode_jwt_with_key(&verifying_key, token)
}

/// Decode and verify a JWT with a given verifying key.
pub fn decode_jwt_with_key(key: &VerifyingKey, token: &str) -> Result<InviteClaims> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        anyhow::bail!("invalid JWT: expected 3 parts, got {}", parts.len());
    }

    let signing_input = format!("{}.{}", parts[0], parts[1]);
    let sig_bytes = URL_SAFE_NO_PAD
        .decode(parts[2])
        .context("invalid JWT signature encoding")?;

    if sig_bytes.len() != 64 {
        anyhow::bail!(
            "invalid JWT signature length ({} bytes, expected 64)",
            sig_bytes.len()
        );
    }

    let mut sig_arr = [0u8; 64];
    sig_arr.copy_from_slice(&sig_bytes);
    let signature = ed25519_dalek::Signature::from_bytes(&sig_arr);

    key.verify_strict(signing_input.as_bytes(), &signature)
        .map_err(|_| anyhow::anyhow!("JWT signature verification failed"))?;

    let payload_json = URL_SAFE_NO_PAD
        .decode(parts[1])
        .context("invalid JWT payload encoding")?;

    let claims: InviteClaims =
        serde_json::from_slice(&payload_json).context("invalid JWT payload")?;

    // Verify it's an invitation token
    if claims.sub != "ztl-invite" {
        anyhow::bail!("JWT is not an invitation token (sub={})", claims.sub);
    }

    // Check expiry
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if now > claims.exp {
        anyhow::bail!("invitation token has expired");
    }

    Ok(claims)
}

/// Record a nonce in `.ztl/collab/used-nonces.json`.
fn record_nonce(vault_root: &Path, nonce: &str, exp: u64) -> Result<()> {
    let path = used_nonces_path(vault_root);
    let mut nonces = load_used_nonces(vault_root)?;

    nonces.push(UsedNonce {
        nonce: nonce.to_string(),
        exp,
    });

    let json = serde_json::to_string_pretty(&nonces).context("failed to serialize nonces")?;
    fs::write(&path, json)
        .with_context(|| format!("failed to write nonces: {}", path.display()))?;

    Ok(())
}

/// Check if a nonce has already been used.
pub fn is_nonce_used(vault_root: &Path, nonce: &str) -> Result<bool> {
    let nonces = load_used_nonces(vault_root)?;
    Ok(nonces
        .iter()
        .any(|n| super::constant_time_eq(n.nonce.as_bytes(), nonce.as_bytes())))
}

/// Mark a nonce as used (for invitation acceptance).
pub fn mark_nonce_used(vault_root: &Path, nonce: &str, exp: u64) -> Result<()> {
    record_nonce(vault_root, nonce, exp)
}

/// Atomically check whether a nonce has been used and mark it as used if not.
///
/// Returns `Ok(true)` if the nonce was freshly marked (i.e. it was NOT already used).
/// Returns `Ok(false)` if the nonce was already used (caller should reject).
///
/// Uses a file lock to prevent TOCTOU races between concurrent requests.
pub fn try_consume_nonce(vault_root: &Path, nonce: &str, exp: u64) -> Result<bool> {
    let dir = collab_dir(vault_root);
    std::fs::create_dir_all(&dir)?;

    // Acquire an advisory file lock on the nonces file to serialize access.
    let lock_path = dir.join("used-nonces.lock");
    let lock_file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("failed to open lock file: {}", lock_path.display()))?;
    fs_lock(&lock_file)?;

    // Under the lock: check + mark atomically.
    let nonces = load_used_nonces(vault_root)?;
    if nonces
        .iter()
        .any(|n| super::constant_time_eq(n.nonce.as_bytes(), nonce.as_bytes()))
    {
        // Already used — unlock and return false.
        fs_unlock(&lock_file)?;
        return Ok(false);
    }

    // Not used yet — record it.
    let mut nonces = nonces;
    nonces.push(UsedNonce {
        nonce: nonce.to_string(),
        exp,
    });
    let path = used_nonces_path(vault_root);
    let json = serde_json::to_string_pretty(&nonces).context("failed to serialize nonces")?;
    std::fs::write(&path, json)
        .with_context(|| format!("failed to write nonces: {}", path.display()))?;

    fs_unlock(&lock_file)?;
    Ok(true)
}

/// Acquire an exclusive advisory lock on a file descriptor.
fn fs_lock(file: &std::fs::File) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        extern "C" {
            fn flock(
                fd: std::os::raw::c_int,
                operation: std::os::raw::c_int,
            ) -> std::os::raw::c_int;
        }
        const LOCK_EX: std::os::raw::c_int = 2;
        let ret = unsafe { flock(file.as_raw_fd(), LOCK_EX) };
        if ret != 0 {
            anyhow::bail!("flock(LOCK_EX) failed: {}", std::io::Error::last_os_error());
        }
    }
    #[cfg(not(unix))]
    {
        let _ = file;
    }
    Ok(())
}

/// Release an advisory lock on a file descriptor.
fn fs_unlock(file: &std::fs::File) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        extern "C" {
            fn flock(
                fd: std::os::raw::c_int,
                operation: std::os::raw::c_int,
            ) -> std::os::raw::c_int;
        }
        const LOCK_UN: std::os::raw::c_int = 8;
        let ret = unsafe { flock(file.as_raw_fd(), LOCK_UN) };
        if ret != 0 {
            anyhow::bail!("flock(LOCK_UN) failed: {}", std::io::Error::last_os_error());
        }
    }
    #[cfg(not(unix))]
    {
        let _ = file;
    }
    Ok(())
}

/// Load used nonces from disk, pruning expired entries (24h past expiry).
fn load_used_nonces(vault_root: &Path) -> Result<Vec<UsedNonce>> {
    let path = used_nonces_path(vault_root);
    if !path.exists() {
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read nonces: {}", path.display()))?;
    let nonces: Vec<UsedNonce> =
        serde_json::from_str(&content).context("failed to parse used-nonces.json")?;

    // Prune nonces whose exp is more than 24 hours in the past (REQ-020-006)
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let cutoff = now.saturating_sub(24 * 60 * 60);

    Ok(nonces.into_iter().filter(|n| n.exp > cutoff).collect())
}

/// Build a full invitation URL.
pub fn invitation_url(host: &str, port: u16, token: &str) -> String {
    format!("http://{host}:{port}/auth/accept?token={token}")
}

// ── Pending invitations ──────────────────────────────────────────────────

/// A pending invitation record stored in `.ztl/collab/pending-invites.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingInvitation {
    /// The nonce (unique identifier for this invitation).
    pub nonce: String,
    /// The JWT token string.
    pub token: String,
    /// Inviter user ID.
    pub inviter_id: String,
    /// Inviter display name.
    pub inviter_name: String,
    /// Granted role.
    pub role: String,
    /// Optional page scope.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pages: Option<String>,
    /// Expiry as Unix timestamp.
    pub exp: u64,
    /// Whether this invitation has been revoked.
    #[serde(default)]
    pub revoked: bool,
}

/// Return the pending-invites file path.
fn pending_invites_path(vault_root: &Path) -> PathBuf {
    collab_dir(vault_root).join(PENDING_INVITES_FILE)
}

/// Save a pending invitation record.
pub fn save_pending_invitation(vault_root: &Path, invite: &PendingInvitation) -> Result<()> {
    let mut invites = load_pending_invitations(vault_root)?;
    invites.push(invite.clone());
    write_pending_invitations(vault_root, &invites)
}

/// Load all pending invitations, pruning expired entries (24h past expiry).
pub fn load_pending_invitations(vault_root: &Path) -> Result<Vec<PendingInvitation>> {
    let path = pending_invites_path(vault_root);
    if !path.exists() {
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read pending invites: {}", path.display()))?;
    let invites: Vec<PendingInvitation> =
        serde_json::from_str(&content).context("failed to parse pending-invites.json")?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let cutoff = now.saturating_sub(24 * 60 * 60);

    Ok(invites.into_iter().filter(|i| i.exp > cutoff).collect())
}

/// Write pending invitations to disk.
fn write_pending_invitations(vault_root: &Path, invites: &[PendingInvitation]) -> Result<()> {
    let dir = collab_dir(vault_root);
    fs::create_dir_all(&dir)?;
    let path = pending_invites_path(vault_root);
    let json =
        serde_json::to_string_pretty(invites).context("failed to serialize pending invites")?;
    fs::write(&path, json)
        .with_context(|| format!("failed to write pending invites: {}", path.display()))?;
    Ok(())
}

/// Revoke a pending invitation by nonce. Returns true if found and revoked.
pub fn revoke_invitation(vault_root: &Path, nonce: &str) -> Result<bool> {
    let mut invites = load_pending_invitations(vault_root)?;
    let mut found = false;
    for invite in &mut invites {
        if super::constant_time_eq(invite.nonce.as_bytes(), nonce.as_bytes()) && !invite.revoked {
            invite.revoked = true;
            found = true;
            break;
        }
    }
    if found {
        write_pending_invitations(vault_root, &invites)?;
        // Also mark the nonce as used so the token can't be accepted
        let exp = invites
            .iter()
            .find(|i| super::constant_time_eq(i.nonce.as_bytes(), nonce.as_bytes()))
            .map(|i| i.exp)
            .unwrap_or(0);
        mark_nonce_used(vault_root, nonce, exp)?;
    }
    Ok(found)
}

/// Mark a pending invitation as consumed (accepted). Returns true if found.
pub fn mark_invitation_consumed(vault_root: &Path, nonce: &str) -> Result<bool> {
    let mut invites = load_pending_invitations(vault_root)?;
    let pos = invites
        .iter()
        .position(|i| super::constant_time_eq(i.nonce.as_bytes(), nonce.as_bytes()));
    if let Some(idx) = pos {
        invites.remove(idx);
        write_pending_invitations(vault_root, &invites)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_generate_nonce_length() {
        let nonce = generate_nonce();
        assert_eq!(nonce.len(), 32, "nonce should be 32 hex chars (128 bits)");
        assert!(nonce.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_generate_nonce_uniqueness() {
        let n1 = generate_nonce();
        let n2 = generate_nonce();
        assert_ne!(n1, n2);
    }

    #[test]
    fn test_server_key_create_and_load() {
        let tmp = TempDir::new().unwrap();
        let key1 = load_or_create_server_key(tmp.path()).unwrap();
        let key2 = load_or_create_server_key(tmp.path()).unwrap();
        assert_eq!(key1.to_bytes(), key2.to_bytes(), "key should persist");
    }

    #[test]
    fn test_server_key_permissions() {
        let tmp = TempDir::new().unwrap();
        load_or_create_server_key(tmp.path()).unwrap();

        let path = server_key_path(tmp.path());
        assert!(path.exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::metadata(&path).unwrap().permissions();
            assert_eq!(perms.mode() & 0o777, 0o600);
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_server_key_rejects_insecure_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = TempDir::new().unwrap();
        // Create the key first with correct permissions
        load_or_create_server_key(tmp.path()).unwrap();

        // Widen permissions to group-readable
        let path = server_key_path(tmp.path());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        let result = load_or_create_server_key(tmp.path());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("insecure permissions"),
            "expected insecure permissions error, got: {err}"
        );
    }

    #[test]
    fn test_generate_and_verify_jwt() {
        let tmp = TempDir::new().unwrap();
        let (token, nonce) = generate_invitation(
            tmp.path(),
            "alice-a1b2c3d4",
            "editor",
            Some("projects/*"),
            Some(3600),
        )
        .unwrap();

        // Verify the token
        let claims = decode_jwt(tmp.path(), &token).unwrap();
        assert_eq!(claims.iss, "alice-a1b2c3d4");
        assert_eq!(claims.sub, "ztl-invite");
        assert_eq!(claims.role, "editor");
        assert_eq!(claims.pages.as_deref(), Some("projects/*"));
        assert_eq!(claims.nonce, nonce);
    }

    #[test]
    fn test_jwt_structure() {
        let tmp = TempDir::new().unwrap();
        let (token, _) =
            generate_invitation(tmp.path(), "alice-a1b2c3d4", "editor", None, Some(3600)).unwrap();

        // JWT should have 3 base64url-encoded parts
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3);

        // Decode header
        let header_json = URL_SAFE_NO_PAD.decode(parts[0]).unwrap();
        let header: serde_json::Value = serde_json::from_slice(&header_json).unwrap();
        assert_eq!(header["alg"], "EdDSA");
        assert_eq!(header["typ"], "JWT");
    }

    #[test]
    fn test_jwt_without_pages() {
        let tmp = TempDir::new().unwrap();
        let (token, _) =
            generate_invitation(tmp.path(), "alice-a1b2c3d4", "reader", None, Some(3600)).unwrap();

        let claims = decode_jwt(tmp.path(), &token).unwrap();
        assert!(claims.pages.is_none());
    }

    #[test]
    fn test_jwt_invalid_signature() {
        let tmp = TempDir::new().unwrap();
        let (token, _) =
            generate_invitation(tmp.path(), "alice-a1b2c3d4", "editor", None, Some(3600)).unwrap();

        // Tamper with the token
        let parts: Vec<&str> = token.split('.').collect();
        let tampered = format!("{}x", parts[2]);
        let tampered_token = format!("{}.{}.{}", parts[0], parts[1], tampered);

        let result = decode_jwt(tmp.path(), &tampered_token);
        assert!(result.is_err());
    }

    #[test]
    fn test_jwt_expired() {
        let tmp = TempDir::new().unwrap();
        let signing_key = load_or_create_server_key(tmp.path()).unwrap();

        // Create a token that's already expired
        let claims = InviteClaims {
            iss: "alice-a1b2c3d4".to_string(),
            sub: "ztl-invite".to_string(),
            role: "editor".to_string(),
            pages: None,
            exp: 1, // Unix epoch + 1 second (way in the past)
            nonce: generate_nonce(),
        };

        let token = encode_jwt(&signing_key, &claims).unwrap();
        let result = decode_jwt(tmp.path(), &token);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("expired"),
            "should report expiry"
        );
    }

    #[test]
    fn test_nonce_tracking() {
        let tmp = TempDir::new().unwrap();

        // Generate invitation does NOT record the nonce (nonces are only
        // marked as used when the invitation is accepted).
        let (_, nonce) =
            generate_invitation(tmp.path(), "alice-a1b2c3d4", "editor", None, Some(3600)).unwrap();

        // Nonce should not be recorded at generation time.
        assert!(!is_nonce_used(tmp.path(), &nonce).unwrap());
        assert!(!is_nonce_used(tmp.path(), "nonexistent").unwrap());

        // Only after explicit mark_nonce_used should it be flagged.
        mark_nonce_used(tmp.path(), &nonce, 9999999999).unwrap();
        assert!(is_nonce_used(tmp.path(), &nonce).unwrap());
    }

    #[test]
    fn test_nonce_pruning() {
        let tmp = TempDir::new().unwrap();

        // Manually write a nonce that expired >24h ago
        let dir = collab_dir(tmp.path());
        fs::create_dir_all(&dir).unwrap();
        let old_nonce = UsedNonce {
            nonce: "old-nonce".to_string(),
            exp: 1, // way in the past
        };
        let json = serde_json::to_string_pretty(&vec![old_nonce]).unwrap();
        fs::write(used_nonces_path(tmp.path()), json).unwrap();

        // Loading should prune it
        let nonces = load_used_nonces(tmp.path()).unwrap();
        assert!(nonces.is_empty(), "expired nonce should be pruned");
    }

    #[test]
    fn test_invitation_url_format() {
        let url = invitation_url("localhost", 3000, "abc.def.ghi");
        assert_eq!(url, "http://localhost:3000/auth/accept?token=abc.def.ghi");
    }

    #[test]
    fn test_multiple_invitations_different_nonces() {
        let tmp = TempDir::new().unwrap();
        let (_, n1) =
            generate_invitation(tmp.path(), "alice-a1b2c3d4", "editor", None, Some(3600)).unwrap();
        let (_, n2) =
            generate_invitation(tmp.path(), "alice-a1b2c3d4", "reader", None, Some(3600)).unwrap();
        assert_ne!(n1, n2);

        // Nonces are NOT tracked at generation time — only at acceptance.
        assert!(!is_nonce_used(tmp.path(), &n1).unwrap());
        assert!(!is_nonce_used(tmp.path(), &n2).unwrap());
    }
}
