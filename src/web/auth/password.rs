//! Static password store + argon2id verify (SPEC-041 REQ-4107, REQ-4108,
//! NFR-4103, CON-4107, ADR-4106).
//!
//! Phase-3 scope (task-auth-password-store): the persistence layer and the
//! pure verify primitive. The `PasswordAuthenticator` and `/auth/password`
//! routes land in task-auth-password-impl; the `zetl collab passwd` CLI
//! lands in task-auth-passwd-cli — both build on this file.
//!
//! On-disk format (CON-4107):
//!
//! ```text
//! .zetl/collab/passwords.json   mode 0600
//! [ { "user_id": "<id>",
//!     "phc":     "$argon2id$v=19$m=<m>,t=<t>,p=<p>$<salt>$<hash>" } ]
//! ```
//!
//! The argon2id parameters are embedded in the PHC string, so [`NFR-4103`]
//! costs can be raised later without invalidating existing credentials.

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use serde::{Deserialize, Serialize};
use thiserror::Error;

const PASSWORDS_DIR: &str = ".zetl/collab";
const PASSWORDS_FILE: &str = "passwords.json";

/// One stored password record (CON-4107).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PasswordRecord {
    pub user_id: String,
    /// The argon2id PHC string — `"$argon2id$v=19$m=...,t=...,p=...$salt$hash"`.
    /// Parameters are embedded so the operator can raise the cost (NFR-4103)
    /// without invalidating existing credentials.
    pub phc: String,
}

/// Why a password operation failed.
#[derive(Debug, Error)]
pub(crate) enum PasswordError {
    #[error("password store I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("password store JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("password hashing: {0}")]
    Hash(String),
    /// Permissions on `passwords.json` are wider than 0600 (unix only).
    #[error("password store {path} has insecure permissions {mode:04o} (expected 0600). Fix with: chmod 600 {path}")]
    InsecurePermissions { path: String, mode: u32 },
    /// `passwd remove <user>` named a user not present in the store.
    #[error("no password record for {0:?}")]
    UnknownUser(String),
}

/// Return the directory holding `passwords.json` for a vault.
fn passwords_dir(vault_root: &Path) -> PathBuf {
    vault_root.join(PASSWORDS_DIR)
}

/// Return the path to `passwords.json`.
pub(crate) fn passwords_path(vault_root: &Path) -> PathBuf {
    passwords_dir(vault_root).join(PASSWORDS_FILE)
}

/// Hash a password with argon2id, returning the PHC string.
///
/// Uses [`Argon2::default()`] — the crate's recommended argon2id v19 settings.
/// Parameters are embedded in the returned PHC string per ADR-4106 so they
/// can be raised later without invalidating existing hashes.
pub(crate) fn hash(password: &str) -> Result<String, PasswordError> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    argon2
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| PasswordError::Hash(e.to_string()))
}

/// Verify `password` against an argon2id PHC string in constant time.
///
/// Returns `false` for a wrong password, an unparseable PHC string, or any
/// other internal failure — REQ-4107 cause-indistinguishability. Callers
/// MUST NOT branch on the reason for a false result.
pub(crate) fn verify(password: &str, phc: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(phc) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

/// Load `passwords.json`, enforcing 0600 permissions (CON-4107, ADR-4106).
///
/// An absent file is **not** an error — it represents the empty store, which
/// is the on-disk state before any password has been set. Insecure
/// permissions ARE an error: the same 0600 enforcement that `server.key`
/// uses in `src/user/invite.rs`.
pub(crate) fn load_store(vault_root: &Path) -> Result<Vec<PasswordRecord>, PasswordError> {
    let path = passwords_path(vault_root);
    if !path.exists() {
        return Ok(Vec::new());
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::metadata(&path)?.permissions();
        let mode = perms.mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(PasswordError::InsecurePermissions {
                path: path.display().to_string(),
                mode,
            });
        }
    }

    let body = fs::read_to_string(&path)?;
    let records: Vec<PasswordRecord> = serde_json::from_str(&body)?;
    Ok(records)
}

/// Persist the store atomically — write to a temp file in the same directory,
/// chmod 0600, then rename (rename is atomic within the same filesystem).
fn write_store_atomic(
    vault_root: &Path,
    records: &[PasswordRecord],
) -> Result<(), PasswordError> {
    let dir = passwords_dir(vault_root);
    fs::create_dir_all(&dir)?;
    let final_path = passwords_path(vault_root);

    // Temp file in the SAME directory so rename is atomic.
    let mut tmp_path = final_path.clone();
    let suffix = format!(".tmp.{}", uuid::Uuid::new_v4());
    tmp_path.set_extension(format!("json{suffix}"));

    {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp_path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        let body = serde_json::to_string_pretty(records)?;
        file.write_all(body.as_bytes())?;
        file.sync_all()?;
    }
    fs::rename(&tmp_path, &final_path)?;
    Ok(())
}

/// Add (or overwrite) the password record for `user_id`, hashing the password
/// with argon2id (NFR-4103 cost). Returns the PHC string written.
pub(crate) fn upsert(
    vault_root: &Path,
    user_id: &str,
    password: &str,
) -> Result<String, PasswordError> {
    let phc = hash(password)?;
    let mut records = load_store(vault_root)?;
    records.retain(|r| r.user_id != user_id);
    records.push(PasswordRecord {
        user_id: user_id.to_string(),
        phc: phc.clone(),
    });
    write_store_atomic(vault_root, &records)?;
    Ok(phc)
}

/// Remove the password record for `user_id`. Returns
/// [`PasswordError::UnknownUser`] if there was no record to remove (so the
/// CLI can give a clear "not found" exit, per CON-4106).
pub(crate) fn remove(vault_root: &Path, user_id: &str) -> Result<(), PasswordError> {
    let mut records = load_store(vault_root)?;
    let before = records.len();
    records.retain(|r| r.user_id != user_id);
    if records.len() == before {
        return Err(PasswordError::UnknownUser(user_id.to_string()));
    }
    write_store_atomic(vault_root, &records)?;
    Ok(())
}

/// List the user_ids that have a password record. Never returns PHC strings
/// or hash bytes — `list` is for operator visibility, not credential
/// inspection (CON-4106).
pub(crate) fn list(vault_root: &Path) -> Result<Vec<String>, PasswordError> {
    let records = load_store(vault_root)?;
    Ok(records.into_iter().map(|r| r.user_id).collect())
}

/// Look up the record for `user_id`. Returns `None` if no record exists.
pub(crate) fn lookup(
    vault_root: &Path,
    user_id: &str,
) -> Result<Option<PasswordRecord>, PasswordError> {
    let records = load_store(vault_root)?;
    Ok(records.into_iter().find(|r| r.user_id == user_id))
}

/// Reject duplicate user_ids in a store (defence-in-depth: the writers above
/// enforce uniqueness, but a hand-edited file might not).
pub(crate) fn check_unique_user_ids(records: &[PasswordRecord]) -> Result<(), PasswordError> {
    let mut seen = HashSet::new();
    for r in records {
        if !seen.insert(&r.user_id) {
            return Err(PasswordError::Hash(format!(
                "duplicate user_id {:?} in passwords.json",
                r.user_id
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// REQ-4107: argon2id roundtrip — a freshly-hashed password verifies; a
    /// different password does not.
    #[test]
    fn hash_and_verify_roundtrip() {
        let phc = hash("correct horse battery staple").unwrap();
        assert!(verify("correct horse battery staple", &phc));
        assert!(!verify("wrong password", &phc));
        assert!(!verify("", &phc));
    }

    /// REQ-4107: each hash uses a fresh salt, so two hashes of the same
    /// password differ.
    #[test]
    fn salts_are_random() {
        let a = hash("password").unwrap();
        let b = hash("password").unwrap();
        assert_ne!(a, b);
        assert!(verify("password", &a));
        assert!(verify("password", &b));
    }

    /// REQ-4107: a malformed PHC string verifies as `false` — never
    /// surfaces a parse error to the caller.
    #[test]
    fn malformed_phc_verifies_false() {
        assert!(!verify("anything", "not a phc string"));
        assert!(!verify("anything", "$argon2id$wrong"));
        assert!(!verify("anything", ""));
    }

    /// ADR-4106: PHC string embeds `argon2id` algorithm + version + params,
    /// so a future cost upgrade can run alongside existing hashes.
    #[test]
    fn phc_string_embeds_argon2id() {
        let phc = hash("x").unwrap();
        assert!(phc.starts_with("$argon2id$"));
        assert!(phc.contains("v=19"));
        assert!(phc.contains("m="));
        assert!(phc.contains("t="));
        assert!(phc.contains("p="));
    }

    /// CON-4107: an absent file is the empty store, not an error.
    #[test]
    fn load_store_absent_file_is_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let records = load_store(tmp.path()).unwrap();
        assert!(records.is_empty());
    }

    /// CON-4106 / CON-4107: upsert creates the file and the record;
    /// load reads it back.
    #[test]
    fn upsert_persists_record() {
        let tmp = tempfile::TempDir::new().unwrap();
        upsert(tmp.path(), "alice", "hunter2").unwrap();
        let records = load_store(tmp.path()).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].user_id, "alice");
        assert!(verify("hunter2", &records[0].phc));
    }

    /// CON-4106: upsert overwrites an existing record rather than duplicating.
    #[test]
    fn upsert_overwrites_existing() {
        let tmp = tempfile::TempDir::new().unwrap();
        upsert(tmp.path(), "alice", "old").unwrap();
        upsert(tmp.path(), "alice", "new").unwrap();
        let records = load_store(tmp.path()).unwrap();
        assert_eq!(records.len(), 1);
        assert!(verify("new", &records[0].phc));
        assert!(!verify("old", &records[0].phc));
    }

    /// CON-4106: remove deletes; remove of unknown user errors clearly.
    #[test]
    fn remove_and_unknown_user() {
        let tmp = tempfile::TempDir::new().unwrap();
        upsert(tmp.path(), "alice", "pw").unwrap();
        remove(tmp.path(), "alice").unwrap();
        assert!(load_store(tmp.path()).unwrap().is_empty());

        let err = remove(tmp.path(), "alice").unwrap_err();
        assert!(matches!(err, PasswordError::UnknownUser(_)));
    }

    /// CON-4106: list emits user_ids only — never PHC strings.
    #[test]
    fn list_returns_user_ids_only() {
        let tmp = tempfile::TempDir::new().unwrap();
        upsert(tmp.path(), "alice", "pw").unwrap();
        upsert(tmp.path(), "bob", "pw").unwrap();
        let ids = list(tmp.path()).unwrap();
        assert!(ids.contains(&"alice".to_string()));
        assert!(ids.contains(&"bob".to_string()));
        // A grep-based check: the returned strings contain no PHC marker.
        for id in &ids {
            assert!(!id.contains("$argon2"));
        }
    }

    /// CON-4107: lookup returns the record on hit, None on miss.
    #[test]
    fn lookup_hit_and_miss() {
        let tmp = tempfile::TempDir::new().unwrap();
        upsert(tmp.path(), "alice", "pw").unwrap();
        assert!(lookup(tmp.path(), "alice").unwrap().is_some());
        assert!(lookup(tmp.path(), "ghost").unwrap().is_none());
    }

    /// CON-4107: writes set mode 0600 (unix).
    #[cfg(unix)]
    #[test]
    fn upsert_sets_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::TempDir::new().unwrap();
        upsert(tmp.path(), "alice", "pw").unwrap();
        let mode = fs::metadata(passwords_path(tmp.path()))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    /// CON-4107 + ADR-4106: reads refuse a file with insecure permissions,
    /// matching the `server.key` enforcement in src/user/invite.rs.
    #[cfg(unix)]
    #[test]
    fn load_store_rejects_insecure_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::TempDir::new().unwrap();
        upsert(tmp.path(), "alice", "pw").unwrap();
        let path = passwords_path(tmp.path());
        // Widen permissions to group-readable.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let err = load_store(tmp.path()).unwrap_err();
        assert!(matches!(err, PasswordError::InsecurePermissions { .. }));
    }

    /// `check_unique_user_ids` catches hand-edited duplicates.
    #[test]
    fn duplicate_user_ids_rejected() {
        let phc = hash("x").unwrap();
        let records = vec![
            PasswordRecord {
                user_id: "alice".to_string(),
                phc: phc.clone(),
            },
            PasswordRecord {
                user_id: "alice".to_string(),
                phc,
            },
        ];
        let err = check_unique_user_ids(&records).unwrap_err();
        assert!(err.to_string().contains("duplicate"));
    }
}
