//! User profile storage for multi-user collaborative editing (SPEC-020).
//!
//! Profiles are stored at `.zetl/users/<user-id>/profile.json` following
//! the CON-020-001 schema.

pub mod agent_token;
pub mod invite;
pub mod passkey;
pub mod recovery;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const USERS_DIR: &str = ".zetl/users";

/// Hardcoded role levels for collab authorization (placeholder for SPL rules).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    /// Read-only access to vault content.
    Reader,
    /// Can read and edit pages.
    Editor,
    /// Full control: edit, manage users, configure vault.
    Admin,
}

impl Role {
    /// Determine the role for a user profile.
    ///
    /// Hardcoded placeholder: owner → Admin, invited users → Editor.
    /// Will be replaced by SPL-driven authorization.
    pub fn for_profile(profile: &UserProfile) -> Self {
        if profile.owner {
            Role::Admin
        } else {
            Role::Editor
        }
    }
}

impl std::str::FromStr for Role {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "reader" => Ok(Role::Reader),
            "editor" => Ok(Role::Editor),
            "admin" => Ok(Role::Admin),
            _ => Err(anyhow::anyhow!(
                "invalid role '{}': expected reader, editor, or admin",
                s
            )),
        }
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Role::Reader => write!(f, "reader"),
            Role::Editor => write!(f, "editor"),
            Role::Admin => write!(f, "admin"),
        }
    }
}

/// A WebAuthn credential stored in the user profile (CON-020-001).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WebAuthnCredential {
    /// Base64url-encoded credential ID from the authenticator.
    pub credential_id: String,
    /// Base64url-encoded COSE public key.
    pub public_key: String,
    /// Signature counter for clone detection.
    pub sign_count: u32,
    /// ISO-8601 timestamp of credential registration.
    pub created_at: String,
    /// Human-readable label (e.g. "MacBook Pro").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// A user profile following CON-020-001.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UserProfile {
    /// User ID: slugified display name + 8 random hex chars (e.g. "alice-a1b2c3d4").
    pub id: String,
    /// Display name.
    pub name: String,
    /// ISO-8601 timestamp of account creation.
    pub created_at: String,
    /// User ID of the inviter, or `None` for the bootstrap owner.
    pub invited_by: Option<String>,
    /// Whether this user is the vault owner.
    pub owner: bool,
    /// WebAuthn passkey credentials (supports multiple).
    pub credentials: Vec<WebAuthnCredential>,
    /// Base64url-encoded ed25519 public key derived from BIP39 mnemonic.
    pub recovery_pubkey: String,
    /// Agent token generation counter for token rotation (REQ-020-055).
    #[serde(default)]
    pub agent_token_generation: u8,
}

/// Generate a user ID from a display name: slugify + 8 random hex chars.
pub fn generate_user_id(name: &str) -> String {
    let slug: String = name
        .chars()
        .filter_map(|c| {
            if c.is_ascii_alphanumeric() {
                Some(c.to_ascii_lowercase())
            } else if c == ' ' || c == '-' || c == '_' {
                Some('-')
            } else {
                None
            }
        })
        .collect();

    // Trim leading/trailing dashes and collapse runs
    let slug = slug.trim_matches('-').to_string();
    let slug = collapse_dashes(&slug);

    let hex: String = (0..4)
        .map(|_| format!("{:02x}", rand_byte()))
        .collect();

    if slug.is_empty() {
        hex
    } else {
        format!("{slug}-{hex}")
    }
}

fn collapse_dashes(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut prev_dash = false;
    for c in s.chars() {
        if c == '-' {
            if !prev_dash {
                result.push('-');
            }
            prev_dash = true;
        } else {
            result.push(c);
            prev_dash = false;
        }
    }
    result
}

/// Simple random byte using getrandom via std.
fn rand_byte() -> u8 {
    let mut buf = [0u8; 1];
    // Use getrandom for cryptographic randomness
    getrandom(&mut buf);
    buf[0]
}

pub(crate) fn getrandom(buf: &mut [u8]) {
    use std::io::Read;
    let mut f = std::fs::File::open("/dev/urandom").expect("open /dev/urandom");
    f.read_exact(buf).expect("read /dev/urandom");
}

/// Return the users directory path for a vault root.
pub fn users_dir(vault_root: &Path) -> PathBuf {
    vault_root.join(USERS_DIR)
}

/// Return the profile directory path for a specific user.
pub fn user_dir(vault_root: &Path, user_id: &str) -> PathBuf {
    users_dir(vault_root).join(user_id)
}

/// Return the profile.json path for a specific user.
pub fn profile_path(vault_root: &Path, user_id: &str) -> PathBuf {
    user_dir(vault_root, user_id).join("profile.json")
}

/// Save a user profile to `.zetl/users/<id>/profile.json`.
pub fn save_profile(vault_root: &Path, profile: &UserProfile) -> Result<()> {
    let dir = user_dir(vault_root, &profile.id);
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create user directory: {}", dir.display()))?;

    let path = dir.join("profile.json");
    let json = serde_json::to_string_pretty(profile)
        .context("failed to serialize user profile")?;
    fs::write(&path, json)
        .with_context(|| format!("failed to write profile: {}", path.display()))?;

    Ok(())
}

/// Load a user profile from `.zetl/users/<id>/profile.json`.
pub fn load_profile(vault_root: &Path, user_id: &str) -> Result<Option<UserProfile>> {
    let path = profile_path(vault_root, user_id);
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read profile: {}", path.display()))?;
    let profile: UserProfile = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse profile: {}", path.display()))?;
    Ok(Some(profile))
}

/// List all user profiles in the vault.
pub fn list_profiles(vault_root: &Path) -> Result<Vec<UserProfile>> {
    let dir = users_dir(vault_root);
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut profiles = Vec::new();
    for entry in fs::read_dir(&dir)
        .with_context(|| format!("failed to read users directory: {}", dir.display()))?
    {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let profile_file = entry.path().join("profile.json");
        if profile_file.exists() {
            let content = fs::read_to_string(&profile_file)?;
            match serde_json::from_str::<UserProfile>(&content) {
                Ok(profile) => profiles.push(profile),
                Err(e) => {
                    eprintln!(
                        "WARN: skipping malformed profile {}: {e}",
                        profile_file.display()
                    );
                }
            }
        }
    }

    profiles.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    Ok(profiles)
}

/// Delete a user profile and its directory.
pub fn delete_profile(vault_root: &Path, user_id: &str) -> Result<()> {
    let dir = user_dir(vault_root, user_id);
    if dir.exists() {
        fs::remove_dir_all(&dir)
            .with_context(|| format!("failed to remove user directory: {}", dir.display()))?;
    }
    Ok(())
}

/// Check if a vault owner already exists.
pub fn owner_exists(vault_root: &Path) -> Result<bool> {
    let profiles = list_profiles(vault_root)?;
    Ok(profiles.iter().any(|p| p.owner))
}

/// Find a user profile by display name (case-insensitive).
pub fn find_by_name(vault_root: &Path, name: &str) -> Result<Option<UserProfile>> {
    let profiles = list_profiles(vault_root)?;
    let lower = name.to_lowercase();
    Ok(profiles.into_iter().find(|p| p.name.to_lowercase() == lower))
}

/// Validate that a user ID matches the expected format.
pub fn validate_user_id(id: &str) -> bool {
    // Must be non-empty, contain only lowercase alphanumeric and dashes,
    // and end with 8 hex chars.
    if id.is_empty() || id.len() < 8 {
        return false;
    }

    // All chars must be lowercase alphanumeric or dash
    if !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return false;
    }

    // Last 8 chars (after the final dash) should be hex
    let hex_suffix = if let Some(pos) = id.rfind('-') {
        &id[pos + 1..]
    } else {
        id
    };

    hex_suffix.len() == 8 && hex_suffix.chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_profile(name: &str, owner: bool) -> UserProfile {
        UserProfile {
            id: generate_user_id(name),
            name: name.to_string(),
            created_at: "2026-03-18T10:00:00Z".to_string(),
            invited_by: None,
            owner,
            credentials: vec![],
            recovery_pubkey: "dGVzdC1wdWJrZXk".to_string(),
            agent_token_generation: 0,
        }
    }

    #[test]
    fn test_generate_user_id_format() {
        let id = generate_user_id("Alice");
        assert!(id.starts_with("alice-"), "got: {id}");
        // slug + dash + 8 hex chars
        let hex_part = id.rsplit('-').next().unwrap();
        assert_eq!(hex_part.len(), 8);
        assert!(hex_part.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_generate_user_id_special_chars() {
        let id = generate_user_id("Bob O'Brien");
        assert!(id.starts_with("bob-obrien-"), "got: {id}");
        assert!(validate_user_id(&id));
    }

    #[test]
    fn test_generate_user_id_empty_name() {
        let id = generate_user_id("");
        // Should be just 8 hex chars
        assert_eq!(id.len(), 8);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_generate_user_id_uniqueness() {
        let id1 = generate_user_id("Alice");
        let id2 = generate_user_id("Alice");
        assert_ne!(id1, id2, "IDs should be unique due to random suffix");
    }

    #[test]
    fn test_validate_user_id() {
        assert!(validate_user_id("alice-a1b2c3d4"));
        assert!(validate_user_id("bob-obrien-12345678"));
        assert!(validate_user_id("abcd1234")); // no slug, just hex
        assert!(!validate_user_id("")); // empty
        assert!(!validate_user_id("short")); // too short hex
        assert!(!validate_user_id("Alice-a1b2c3d4")); // uppercase
    }

    #[test]
    fn test_save_and_load_profile() {
        let tmp = TempDir::new().unwrap();
        let profile = make_profile("Alice", true);

        save_profile(tmp.path(), &profile).unwrap();
        let loaded = load_profile(tmp.path(), &profile.id).unwrap();
        assert_eq!(loaded, Some(profile));
    }

    #[test]
    fn test_load_nonexistent_profile() {
        let tmp = TempDir::new().unwrap();
        let loaded = load_profile(tmp.path(), "no-such-user").unwrap();
        assert_eq!(loaded, None);
    }

    #[test]
    fn test_list_profiles() {
        let tmp = TempDir::new().unwrap();
        let alice = make_profile("Alice", true);
        let mut bob = make_profile("Bob", false);
        bob.created_at = "2026-03-19T10:00:00Z".to_string();
        bob.invited_by = Some(alice.id.clone());

        save_profile(tmp.path(), &alice).unwrap();
        save_profile(tmp.path(), &bob).unwrap();

        let profiles = list_profiles(tmp.path()).unwrap();
        assert_eq!(profiles.len(), 2);
        // Sorted by created_at, alice first
        assert_eq!(profiles[0].name, "Alice");
        assert_eq!(profiles[1].name, "Bob");
    }

    #[test]
    fn test_list_profiles_empty_vault() {
        let tmp = TempDir::new().unwrap();
        let profiles = list_profiles(tmp.path()).unwrap();
        assert!(profiles.is_empty());
    }

    #[test]
    fn test_delete_profile() {
        let tmp = TempDir::new().unwrap();
        let profile = make_profile("Alice", true);

        save_profile(tmp.path(), &profile).unwrap();
        assert!(load_profile(tmp.path(), &profile.id).unwrap().is_some());

        delete_profile(tmp.path(), &profile.id).unwrap();
        assert!(load_profile(tmp.path(), &profile.id).unwrap().is_none());
    }

    #[test]
    fn test_owner_exists() {
        let tmp = TempDir::new().unwrap();
        assert!(!owner_exists(tmp.path()).unwrap());

        let owner = make_profile("Alice", true);
        save_profile(tmp.path(), &owner).unwrap();
        assert!(owner_exists(tmp.path()).unwrap());
    }

    #[test]
    fn test_find_by_name() {
        let tmp = TempDir::new().unwrap();
        let alice = make_profile("Alice", true);
        save_profile(tmp.path(), &alice).unwrap();

        let found = find_by_name(tmp.path(), "alice").unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, alice.id);

        let not_found = find_by_name(tmp.path(), "bob").unwrap();
        assert!(not_found.is_none());
    }

    #[test]
    fn test_profile_serialization_matches_schema() {
        let profile = UserProfile {
            id: "alice-a1b2c3d4".to_string(),
            name: "Alice".to_string(),
            created_at: "2026-03-18T10:00:00Z".to_string(),
            invited_by: None,
            owner: true,
            credentials: vec![WebAuthnCredential {
                credential_id: "Y3JlZC1pZA".to_string(),
                public_key: "cHViLWtleQ".to_string(),
                sign_count: 42,
                created_at: "2026-03-18T10:00:00Z".to_string(),
                label: Some("MacBook Pro".to_string()),
            }],
            recovery_pubkey: "cmVjb3ZlcnktcHVia2V5".to_string(),
            agent_token_generation: 0,
        };

        let json = serde_json::to_value(&profile).unwrap();
        // Validate all CON-020-001 required fields
        assert_eq!(json["id"], "alice-a1b2c3d4");
        assert_eq!(json["name"], "Alice");
        assert_eq!(json["created_at"], "2026-03-18T10:00:00Z");
        assert!(json["invited_by"].is_null());
        assert_eq!(json["owner"], true);
        assert!(json["credentials"].is_array());
        assert_eq!(json["credentials"][0]["credential_id"], "Y3JlZC1pZA");
        assert_eq!(json["credentials"][0]["public_key"], "cHViLWtleQ");
        assert_eq!(json["credentials"][0]["sign_count"], 42);
        assert_eq!(json["credentials"][0]["label"], "MacBook Pro");
        assert_eq!(json["recovery_pubkey"], "cmVjb3ZlcnktcHVia2V5");
    }

    #[test]
    fn test_credential_label_omitted_when_none() {
        let cred = WebAuthnCredential {
            credential_id: "Y3JlZC1pZA".to_string(),
            public_key: "cHViLWtleQ".to_string(),
            sign_count: 0,
            created_at: "2026-03-18T10:00:00Z".to_string(),
            label: None,
        };
        let json = serde_json::to_value(&cred).unwrap();
        assert!(!json.as_object().unwrap().contains_key("label"));
    }

    #[test]
    fn test_profile_with_multiple_credentials() {
        let tmp = TempDir::new().unwrap();
        let mut profile = make_profile("Alice", true);
        profile.credentials = vec![
            WebAuthnCredential {
                credential_id: "cred1".to_string(),
                public_key: "pk1".to_string(),
                sign_count: 10,
                created_at: "2026-03-18T10:00:00Z".to_string(),
                label: Some("MacBook Pro".to_string()),
            },
            WebAuthnCredential {
                credential_id: "cred2".to_string(),
                public_key: "pk2".to_string(),
                sign_count: 5,
                created_at: "2026-03-18T11:00:00Z".to_string(),
                label: Some("iPhone".to_string()),
            },
        ];

        save_profile(tmp.path(), &profile).unwrap();
        let loaded = load_profile(tmp.path(), &profile.id).unwrap().unwrap();
        assert_eq!(loaded.credentials.len(), 2);
        assert_eq!(loaded.credentials[0].label.as_deref(), Some("MacBook Pro"));
        assert_eq!(loaded.credentials[1].label.as_deref(), Some("iPhone"));
    }

    #[test]
    fn test_profile_directory_structure() {
        let tmp = TempDir::new().unwrap();
        let profile = make_profile("Alice", true);

        save_profile(tmp.path(), &profile).unwrap();

        // Verify the directory structure matches spec
        let expected_dir = tmp.path().join(".zetl/users").join(&profile.id);
        assert!(expected_dir.is_dir());
        assert!(expected_dir.join("profile.json").is_file());
    }

    #[test]
    fn test_role_for_owner_is_admin() {
        let profile = make_profile("Alice", true);
        assert_eq!(Role::for_profile(&profile), Role::Admin);
    }

    #[test]
    fn test_role_for_non_owner_is_editor() {
        let profile = make_profile("Bob", false);
        assert_eq!(Role::for_profile(&profile), Role::Editor);
    }

    #[test]
    fn test_role_ordering() {
        assert!(Role::Admin > Role::Editor);
        assert!(Role::Editor > Role::Reader);
        assert!(Role::Reader < Role::Admin);
    }

    #[test]
    fn test_role_display() {
        assert_eq!(Role::Admin.to_string(), "admin");
        assert_eq!(Role::Editor.to_string(), "editor");
        assert_eq!(Role::Reader.to_string(), "reader");
    }
}
