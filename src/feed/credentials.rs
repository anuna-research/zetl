//! `.zetl/credentials.toml` store per CON-3812 + ADR-3810.
//!
//! Storage shape (single-file form):
//!
//! ```toml
//! [<subscription-id>]
//! auth_type = "basic" | "bearer" | "query_param"
//!
//! # basic
//! username = "..."
//! password = "..."
//!
//! # bearer
//! token = "..."
//!
//! # query_param (one of):
//! url_with_token = "https://feed.example/?api_key=secret"
//! # OR:
//! token_param = "api_key"
//! token_value = "secret"
//! ```
//!
//! Per-subscription-file form (`.zetl/credentials/<id>.toml`) is also
//! accepted; the inner shape is just the `auth_type=...` block without
//! the `[<id>]` heading.
//!
//! Required behaviours:
//!
//!   * REQ-3825 mode-0600 enforcement on read; structured error naming
//!     the file + expected mode when looser.
//!   * REQ-3825 atomic write-then-rename with mode 0600.
//!   * REQ-3825 hard refusal of credential-shaped keys in
//!     `.zetl/config.toml` (validation surface; the file-IO is at the
//!     shell layer that owns the config-load pass).
//!   * REQ-3826 `.gitignore` augmentation hooks for `zetl init`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// REQ-3825 owner-only mode for `.zetl/credentials.toml`. Enforced on
/// every read; set on every write.
pub const REQUIRED_MODE: u32 = 0o600;
/// Mask of permission bits we care about (rwxrwxrwx); ignores SUID
/// bits and file-type bits when comparing against [`REQUIRED_MODE`].
pub const MODE_MASK: u32 = 0o777;

/// One subscription's credential record. Tagged-union over auth scheme.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "auth_type", rename_all = "snake_case")]
pub enum Credential {
    /// HTTP Basic (Authorization: Basic base64(user:pass)).
    Basic { username: String, password: String },
    /// Bearer token (Authorization: Bearer <token>).
    Bearer { token: String },
    /// Query-param token. Either `url_with_token` is supplied as a
    /// pre-built URL OR `token_param`/`token_value` are supplied
    /// separately so the fetcher can splice the param onto the
    /// configured `source` URL.
    QueryParam {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url_with_token: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token_param: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token_value: Option<String>,
    },
}

impl Credential {
    /// Validate per-variant invariants (e.g. query_param needs at
    /// least one of url_with_token or token_param/token_value).
    pub fn validate(&self) -> Result<(), CredentialError> {
        match self {
            Credential::Basic { username, password } => {
                if username.is_empty() {
                    return Err(CredentialError::MissingField {
                        scheme: "basic",
                        field: "username",
                    });
                }
                if password.is_empty() {
                    return Err(CredentialError::MissingField {
                        scheme: "basic",
                        field: "password",
                    });
                }
                Ok(())
            }
            Credential::Bearer { token } => {
                if token.is_empty() {
                    return Err(CredentialError::MissingField {
                        scheme: "bearer",
                        field: "token",
                    });
                }
                Ok(())
            }
            Credential::QueryParam {
                url_with_token,
                token_param,
                token_value,
            } => {
                let url_form = url_with_token.is_some();
                let pair_form = token_param.is_some() && token_value.is_some();
                if !url_form && !pair_form {
                    return Err(CredentialError::MissingField {
                        scheme: "query_param",
                        field: "url_with_token OR (token_param + token_value)",
                    });
                }
                Ok(())
            }
        }
    }
}

/// In-memory credentials store. Keyed by subscription id (or
/// `credentials_ref` override per CON-3811).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CredentialsFile(pub BTreeMap<String, Credential>);

impl CredentialsFile {
    /// Look up a credential by subscription id / ref. Validates the
    /// per-variant invariants on demand so callers never have to
    /// re-check after construction.
    pub fn get(&self, key: &str) -> Result<Option<&Credential>, CredentialError> {
        let Some(c) = self.0.get(key) else {
            return Ok(None);
        };
        c.validate()?;
        Ok(Some(c))
    }
}

/// Parse the single-file form (`.zetl/credentials.toml`).
pub fn parse_credentials_file(body: &str) -> Result<CredentialsFile, CredentialError> {
    let raw: BTreeMap<String, Credential> =
        toml::from_str(body).map_err(|e| CredentialError::Toml(e.to_string()))?;
    for (id, cred) in &raw {
        cred.validate().map_err(|e| match e {
            CredentialError::MissingField { scheme, field } => CredentialError::SubscriptionMissingField {
                subscription_id: id.clone(),
                scheme,
                field,
            },
            other => other,
        })?;
    }
    Ok(CredentialsFile(raw))
}

/// Validate that the operator has not embedded credentials in the
/// non-protected `.zetl/config.toml`. REQ-3825 is enforced at this
/// boundary so a misplaced password produces a hard error before the
/// shell ever issues a request.
///
/// `config_body` is the raw `.zetl/config.toml` body. We surface every
/// disallowed key found rather than short-circuiting; the caller emits
/// them as a single batched error.
pub fn config_credential_leak_scan(config_body: &str) -> Result<(), CredentialError> {
    let parsed: toml::Value = toml::from_str(config_body)
        .map_err(|e| CredentialError::Toml(format!("config.toml unreadable: {e}")))?;
    let mut leaks: Vec<String> = Vec::new();
    walk_for_creds(&parsed, "".to_string(), &mut leaks);
    if leaks.is_empty() {
        Ok(())
    } else {
        Err(CredentialError::ConfigLeak(leaks))
    }
}

/// Key names that flag a credential leak when found in
/// `.zetl/config.toml`. Covers every variant of [`Credential`] plus
/// the wider set of secret-shaped query-param keys recognised by
/// [`crate::feed::auth::SECRET_PARAM_KEYS`].
pub const LEAK_SCAN_KEYS: &[&str] = &[
    // Credential variants from this module.
    "password",
    "token",
    "token_value",
    "token_param",
    "url_with_token",
    // Generic secret-shaped keys (mirrors auth::SECRET_PARAM_KEYS).
    "secret",
    "auth",
    "api_key",
    "apikey",
    "access_token",
];

fn walk_for_creds(value: &toml::Value, path: String, out: &mut Vec<String>) {
    match value {
        toml::Value::Table(t) => {
            for (k, v) in t {
                let here = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                if LEAK_SCAN_KEYS.iter().any(|key| k.eq_ignore_ascii_case(key)) {
                    out.push(here.clone());
                }
                walk_for_creds(v, here, out);
            }
        }
        toml::Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                walk_for_creds(v, format!("{path}[{i}]"), out);
            }
        }
        _ => {}
    }
}

/// Serialise a [`CredentialsFile`] back to TOML for writing. Produces
/// stable, deterministic output (BTreeMap iteration order).
pub fn serialize_credentials_file(file: &CredentialsFile) -> Result<String, CredentialError> {
    toml::to_string_pretty(&file.0).map_err(|e| CredentialError::Toml(e.to_string()))
}

/// REQ-3825 read path: load `.zetl/credentials.toml` from disk after
/// verifying the file's permission bits match [`REQUIRED_MODE`].
/// Refuses to load with [`CredentialError::LooseMode`] when the file
/// is readable by group or other (or any non-owner bit). On non-Unix
/// platforms the mode check is skipped — Windows ACLs are out of
/// scope for v1; the shell layer documents this in its operator
/// guidance.
pub fn load_credentials_file(path: &Path) -> Result<CredentialsFile, CredentialError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = std::fs::metadata(path).map_err(|e| CredentialError::Io {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;
        let mode = metadata.permissions().mode() & MODE_MASK;
        if mode != REQUIRED_MODE {
            return Err(CredentialError::LooseMode {
                path: path.to_path_buf(),
                actual: mode,
                required: REQUIRED_MODE,
            });
        }
    }
    let body = std::fs::read_to_string(path).map_err(|e| CredentialError::Io {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;
    parse_credentials_file(&body)
}

/// REQ-3825 write path: serialise + atomic write-then-rename with
/// mode-0600 set before the rename. The temp file lives next to the
/// target so the rename is in the same filesystem (atomic on POSIX
/// + NTFS). On Unix, the temp file is created with mode-0600 from
/// the start so the secret is never visible at a looser mode even
/// transiently.
pub fn save_credentials_file(
    path: &Path,
    file: &CredentialsFile,
) -> Result<(), CredentialError> {
    let body = serialize_credentials_file(file)?;
    let tmp_path = sibling_tmp_path(path);
    write_owner_only(&tmp_path, body.as_bytes()).map_err(|e| CredentialError::Io {
        path: tmp_path.clone(),
        message: e.to_string(),
    })?;
    std::fs::rename(&tmp_path, path).map_err(|e| {
        // Best-effort tmp cleanup; preserve the original error.
        let _ = std::fs::remove_file(&tmp_path);
        CredentialError::Io {
            path: path.to_path_buf(),
            message: format!("rename {} -> {}: {e}", tmp_path.display(), path.display()),
        }
    })?;
    Ok(())
}

fn sibling_tmp_path(target: &Path) -> PathBuf {
    let mut name = target
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from("credentials"));
    name.push(".tmp");
    target
        .parent()
        .map(|p| p.join(&name))
        .unwrap_or_else(|| PathBuf::from(&name))
}

#[cfg(unix)]
fn write_owner_only(path: &Path, body: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(REQUIRED_MODE)
        .open(path)?;
    f.write_all(body)?;
    f.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn write_owner_only(path: &Path, body: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, body)
}

/// Errors at the credentials-store boundary.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CredentialError {
    #[error("malformed credentials file: {0}")]
    Toml(String),
    #[error("[{scheme}] credential missing field: {field}")]
    MissingField { scheme: &'static str, field: &'static str },
    #[error("[{subscription_id}] {scheme} credential missing field: {field}")]
    SubscriptionMissingField {
        subscription_id: String,
        scheme: &'static str,
        field: &'static str,
    },
    /// REQ-3825: credentials present in `.zetl/config.toml`. Carries
    /// every offending key path (e.g. `subscriptions.0.password`) so
    /// the shell can produce a complete remediation message.
    #[error(
        "credentials in .zetl/config.toml (REQ-3825 + ADR-3810): {0:?}; \
         move them to .zetl/credentials.toml (mode 0600)"
    )]
    ConfigLeak(Vec<String>),
    /// REQ-3825: file mode is looser than 0600 on read.
    #[error("{path:?} mode {actual:o} differs from required {required:o} (REQ-3825); chmod 600 the file before retrying")]
    LooseMode {
        path: PathBuf,
        actual: u32,
        required: u32,
    },
    /// Filesystem error during read or write.
    #[error("io on {path:?}: {message}")]
    Io { path: PathBuf, message: String },
}

/// Lines added to `.gitignore` by `zetl init` per REQ-3825's secrets-
/// hygiene mandate. Returned as a slice so the shell is responsible
/// for the actual file write.
pub const GITIGNORE_ENTRIES: &[&str] = &[".zetl/credentials.toml", ".zetl/credentials/"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_scheme() {
        let body = r#"
            [upstream]
            auth_type = "basic"
            username = "alice"
            password = "hunter2"
        "#;
        let file = parse_credentials_file(body).unwrap();
        let cred = file.get("upstream").unwrap().unwrap();
        match cred {
            Credential::Basic { username, password } => {
                assert_eq!(username, "alice");
                assert_eq!(password, "hunter2");
            }
            other => panic!("expected Basic, got {other:?}"),
        }
    }

    #[test]
    fn parses_bearer_scheme() {
        let body = r#"
            [upstream]
            auth_type = "bearer"
            token = "abc123"
        "#;
        let file = parse_credentials_file(body).unwrap();
        let cred = file.get("upstream").unwrap().unwrap();
        assert!(matches!(cred, Credential::Bearer { token } if token == "abc123"));
    }

    #[test]
    fn parses_query_param_url_form() {
        let body = r#"
            [upstream]
            auth_type = "query_param"
            url_with_token = "https://example.com/feed?api_key=secret"
        "#;
        let file = parse_credentials_file(body).unwrap();
        let cred = file.get("upstream").unwrap().unwrap();
        assert!(matches!(
            cred,
            Credential::QueryParam {
                url_with_token: Some(_),
                token_param: None,
                token_value: None,
            }
        ));
    }

    #[test]
    fn parses_query_param_pair_form() {
        let body = r#"
            [upstream]
            auth_type = "query_param"
            token_param = "api_key"
            token_value = "secret"
        "#;
        let file = parse_credentials_file(body).unwrap();
        assert!(matches!(
            file.get("upstream").unwrap().unwrap(),
            Credential::QueryParam {
                url_with_token: None,
                token_param: Some(_),
                token_value: Some(_),
            }
        ));
    }

    #[test]
    fn missing_required_field_surfaces_per_subscription() {
        let body = r#"
            [upstream]
            auth_type = "basic"
            username = "alice"
        "#;
        let err = parse_credentials_file(body).unwrap_err();
        // Will fail to parse (serde rejects missing required field
        // on the Basic variant first) — but if we constructed the
        // file in-memory, validate() would catch it.
        assert!(matches!(err, CredentialError::Toml(_)));
    }

    #[test]
    fn query_param_needs_at_least_one_form() {
        let cred = Credential::QueryParam {
            url_with_token: None,
            token_param: None,
            token_value: None,
        };
        let err = cred.validate().unwrap_err();
        assert!(matches!(err, CredentialError::MissingField { scheme: "query_param", .. }));
    }

    #[test]
    fn config_leak_scan_finds_password() {
        let body = r#"
            [feed]
            base_url = "https://example.com"

            [[subscriptions]]
            id = "x"
            source = "https://upstream/feed"
            password = "leaked"
        "#;
        let err = config_credential_leak_scan(body).unwrap_err();
        match err {
            CredentialError::ConfigLeak(paths) => {
                assert!(paths.iter().any(|p| p.ends_with(".password")), "got {paths:?}");
            }
            other => panic!("expected ConfigLeak, got {other:?}"),
        }
    }

    #[test]
    fn config_leak_scan_finds_token_under_array() {
        let body = r#"
            [[subscriptions]]
            id = "a"
            source = "x"
            token = "xxx"
        "#;
        let err = config_credential_leak_scan(body).unwrap_err();
        assert!(matches!(err, CredentialError::ConfigLeak(_)));
    }

    #[test]
    fn config_leak_scan_clean_config_passes() {
        let body = r#"
            [feed]
            base_url = "https://example.com"
            [[subscriptions]]
            id = "x"
            source = "https://upstream/feed"
            credentials_ref = "x"
        "#;
        config_credential_leak_scan(body).unwrap();
    }

    #[test]
    fn round_trip_serialise_deserialise() {
        let body = r#"
            [a]
            auth_type = "basic"
            username = "u"
            password = "p"
        "#;
        let file = parse_credentials_file(body).unwrap();
        let out = serialize_credentials_file(&file).unwrap();
        let back = parse_credentials_file(&out).unwrap();
        assert_eq!(file, back);
    }

    #[test]
    fn unknown_subscription_returns_none() {
        let file = CredentialsFile::default();
        assert!(file.get("nope").unwrap().is_none());
    }

    #[test]
    fn gitignore_entries_cover_both_storage_shapes() {
        assert!(GITIGNORE_ENTRIES.contains(&".zetl/credentials.toml"));
        assert!(GITIGNORE_ENTRIES.contains(&".zetl/credentials/"));
    }

    #[test]
    fn config_leak_scan_finds_query_param_token_value() {
        // REQ-3825: an operator pasting a query_param block into
        // .zetl/config.toml must be warned about every secret-shaped
        // key, not just `token` / `password` / `url_with_token`.
        let body = r#"
            [[subscriptions]]
            id = "x"
            source = "https://upstream/feed"
            token_param = "api_key"
            token_value = "secret"
        "#;
        let err = config_credential_leak_scan(body).unwrap_err();
        match err {
            CredentialError::ConfigLeak(paths) => {
                assert!(paths.iter().any(|p| p.ends_with(".token_value")), "got {paths:?}");
                assert!(paths.iter().any(|p| p.ends_with(".token_param")), "got {paths:?}");
            }
            other => panic!("expected ConfigLeak, got {other:?}"),
        }
    }

    #[test]
    fn config_leak_scan_finds_generic_secret_keys() {
        for key in ["api_key", "apikey", "secret", "auth", "access_token"] {
            let body = format!(
                r#"
                [[subscriptions]]
                id = "x"
                source = "y"
                {key} = "leaked"
                "#
            );
            let err = config_credential_leak_scan(&body).unwrap_err();
            match err {
                CredentialError::ConfigLeak(paths) => {
                    assert!(paths.iter().any(|p| p.ends_with(&format!(".{key}"))), "key {key} not flagged: {paths:?}");
                }
                other => panic!("expected ConfigLeak for {key}, got {other:?}"),
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn load_credentials_file_rejects_loose_mode() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!(
            "zetl-cred-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("credentials.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"[x]\nauth_type=\"bearer\"\ntoken=\"t\"\n").unwrap();
        // Looser-than-0600 (group + other can read).
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = load_credentials_file(&path).unwrap_err();
        match err {
            CredentialError::LooseMode { actual, required, .. } => {
                assert_eq!(actual, 0o644);
                assert_eq!(required, 0o600);
            }
            other => panic!("expected LooseMode, got {other:?}"),
        }
        // chmod to 0600 and confirm load succeeds.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let file = load_credentials_file(&path).unwrap();
        assert!(file.get("x").unwrap().is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn save_credentials_file_writes_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!(
            "zetl-cred-write-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("credentials.toml");
        let mut file = CredentialsFile::default();
        file.0.insert(
            "x".to_string(),
            Credential::Bearer {
                token: "secret".to_string(),
            },
        );
        save_credentials_file(&path, &file).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "save did not set 0600 (got {mode:o})");
        // Round-trip: load it back via the protected reader.
        let back = load_credentials_file(&path).unwrap();
        assert_eq!(file, back);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
