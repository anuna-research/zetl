//! `recipients.toml` schema + parsing (SPEC-034 REQ-3409 / CON-3403).
//!
//! ```toml
//! version = 1
//!
//! [vault]
//! signing_pubkey = "ed25519:<b64url>"        # embedded in shim bundle
//!
//! [[cohort]]
//! id   = "engineering"
//! name = "Engineering Team"
//! mode = "delegated-url"                      # or "webauthn-prf"
//! pubkeys = [
//!   "age-recipient-v1:<b64url>",
//!   ...
//! ]
//! ```
//!
//! REQ-3409 restricts pubkey entries to the `age-recipient-v1:` prefix;
//! other age recipient types (scrypt, ssh-rsa, ssh-ed25519) are
//! rejected up-front so a misconfigured operator cannot paste a
//! password-recipient (which would weaken the offline-brute-force
//! posture in NFR-3404) or an ssh recipient (which lacks the constant-
//! time X25519 guarantees the spec relies on).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// Recipient-entry prefix (REQ-3409). Changing this constant is a
/// wire-format break and should be coupled with a `version` bump.
pub const AGE_RECIPIENT_V1_PREFIX: &str = "age-recipient-v1:";

/// Vault-signing pubkey prefix (CON-3403).
pub const ED25519_PUBKEY_PREFIX: &str = "ed25519:";

/// The current schema version. CON-3403 fixes this at `1`; readers
/// that encounter a higher version should error so they don't
/// half-parse a newer format.
pub const CURRENT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RecipientsFile {
    pub version: u32,
    pub vault: VaultSection,
    #[serde(default, rename = "cohort")]
    pub cohorts: Vec<Cohort>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VaultSection {
    pub signing_pubkey: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Cohort {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub mode: CohortMode,
    #[serde(default)]
    pub pubkeys: Vec<String>,
    /// Optional glob; when set, only pages matching it are encrypted
    /// for this cohort. Pure core does not compile the glob here; that
    /// happens in `cap::scoping::cohort_index`.
    #[serde(default)]
    pub pages: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CohortMode {
    DelegatedUrl,
    WebauthnPrf,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("unsupported recipients-file version {0} (current: {CURRENT_VERSION})")]
    UnsupportedVersion(u32),
    #[error("vault.signing_pubkey is missing the `{ED25519_PUBKEY_PREFIX}` prefix")]
    SigningPubkeyPrefix,
    #[error("vault.signing_pubkey payload is empty")]
    SigningPubkeyEmpty,
    #[error("vault.signing_pubkey payload contains non-base64url character {0:?}")]
    SigningPubkeyCharset(char),
    #[error("cohort id {0:?} is empty or not a valid identifier")]
    InvalidCohortId(String),
    #[error("duplicate cohort id {0:?}")]
    DuplicateCohortId(String),
    #[error(
        "cohort {cohort:?} recipient {idx} is missing the `{prefix}` prefix",
        prefix = AGE_RECIPIENT_V1_PREFIX
    )]
    RecipientPrefix { cohort: String, idx: usize },
    #[error("cohort {cohort:?} recipient {idx} payload is empty")]
    RecipientEmpty { cohort: String, idx: usize },
    #[error("cohort {cohort:?} recipient {idx} payload contains non-base64url character {ch:?}")]
    RecipientCharset {
        cohort: String,
        idx: usize,
        ch: char,
    },
    #[error("cohort {cohort:?} contains a duplicate recipient pubkey")]
    DuplicateRecipient { cohort: String },
    #[error("cohort {0:?} has an empty pages glob")]
    EmptyPages(String),
    #[error(transparent)]
    Toml(#[from] toml::de::Error),
}

impl RecipientsFile {
    /// Parse a TOML string and validate all invariants in one go.
    pub fn parse(s: &str) -> Result<Self, ParseError> {
        let f: Self = toml::from_str(s)?;
        f.validate()?;
        Ok(f)
    }

    pub fn validate(&self) -> Result<(), ParseError> {
        if self.version != CURRENT_VERSION {
            return Err(ParseError::UnsupportedVersion(self.version));
        }
        validate_signing_pubkey(&self.vault.signing_pubkey)?;

        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for c in &self.cohorts {
            if !is_valid_cohort_id(&c.id) {
                return Err(ParseError::InvalidCohortId(c.id.clone()));
            }
            if !seen.insert(c.id.as_str()) {
                return Err(ParseError::DuplicateCohortId(c.id.clone()));
            }
            validate_cohort_pubkeys(c)?;
            if let Some(p) = c.pages.as_deref() {
                if p.trim().is_empty() {
                    return Err(ParseError::EmptyPages(c.id.clone()));
                }
            }
        }
        Ok(())
    }

    /// Return the set of cohort ids — the cross-reference set used by
    /// `cap::grants::validation::GrantsFile::validate`.
    pub fn cohort_ids(&self) -> BTreeSet<String> {
        self.cohorts.iter().map(|c| c.id.clone()).collect()
    }
}

fn validate_signing_pubkey(s: &str) -> Result<(), ParseError> {
    let payload = s
        .strip_prefix(ED25519_PUBKEY_PREFIX)
        .ok_or(ParseError::SigningPubkeyPrefix)?;
    if payload.is_empty() {
        return Err(ParseError::SigningPubkeyEmpty);
    }
    for c in payload.chars() {
        if !is_base64url(c) {
            return Err(ParseError::SigningPubkeyCharset(c));
        }
    }
    Ok(())
}

fn validate_cohort_pubkeys(c: &Cohort) -> Result<(), ParseError> {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for (idx, pk) in c.pubkeys.iter().enumerate() {
        let payload =
            pk.strip_prefix(AGE_RECIPIENT_V1_PREFIX)
                .ok_or(ParseError::RecipientPrefix {
                    cohort: c.id.clone(),
                    idx,
                })?;
        if payload.is_empty() {
            return Err(ParseError::RecipientEmpty {
                cohort: c.id.clone(),
                idx,
            });
        }
        for ch in payload.chars() {
            if !is_base64url(ch) {
                return Err(ParseError::RecipientCharset {
                    cohort: c.id.clone(),
                    idx,
                    ch,
                });
            }
        }
        if !seen.insert(pk.as_str()) {
            return Err(ParseError::DuplicateRecipient {
                cohort: c.id.clone(),
            });
        }
    }
    Ok(())
}

fn is_base64url(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

/// Cohort ids are stable URL-derivation inputs (CON-3401 feeds them
/// into HKDF `info`). Pinning them to `[a-z0-9][a-z0-9-]*` matches the
/// Tier-2 review S3-01 recommendation and sidesteps any salt-
/// concatenation ambiguity: `/` is forbidden, so `"a/b"` cannot
/// collide with `"a"` + `"/b"` across cohort/slug boundaries.
fn is_valid_cohort_id(s: &str) -> bool {
    let mut it = s.chars();
    match it.next() {
        Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit() => {}
        _ => return false,
    }
    it.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD_SIGNING_PUBKEY: &str = "ed25519:Abc123_-xyz";

    fn sample() -> RecipientsFile {
        RecipientsFile {
            version: 1,
            vault: VaultSection {
                signing_pubkey: GOOD_SIGNING_PUBKEY.to_string(),
            },
            cohorts: vec![Cohort {
                id: "engineering".to_string(),
                name: Some("Engineering".to_string()),
                mode: CohortMode::DelegatedUrl,
                pubkeys: vec![
                    format!("{AGE_RECIPIENT_V1_PREFIX}abc123"),
                    format!("{AGE_RECIPIENT_V1_PREFIX}def-456_"),
                ],
                pages: None,
            }],
        }
    }

    #[test]
    fn sample_parses_and_validates() {
        sample().validate().unwrap();
    }

    #[test]
    fn parse_from_toml_round_trip() {
        let f = sample();
        let s = toml::to_string(&f).unwrap();
        let back = RecipientsFile::parse(&s).unwrap();
        assert_eq!(f, back);
    }

    #[test]
    fn rejects_unsupported_version() {
        let mut f = sample();
        f.version = 2;
        assert!(matches!(
            f.validate(),
            Err(ParseError::UnsupportedVersion(2))
        ));
    }

    #[test]
    fn rejects_bad_signing_pubkey_prefix() {
        let mut f = sample();
        f.vault.signing_pubkey = "rsa:abc".to_string();
        assert!(matches!(f.validate(), Err(ParseError::SigningPubkeyPrefix)));
    }

    #[test]
    fn rejects_non_age_recipient_v1() {
        // REQ-3409: only `age-recipient-v1:` is allowed. This rules
        // out scrypt / ssh-ed25519 / ssh-rsa recipients even though
        // `age` itself would accept them.
        let mut f = sample();
        f.cohorts[0].pubkeys[0] = "age-scrypt:deadbeef".to_string();
        assert!(matches!(
            f.validate(),
            Err(ParseError::RecipientPrefix { .. })
        ));
        f.cohorts[0].pubkeys[0] = "ssh-ed25519:AAAAC3Nza".to_string();
        assert!(matches!(
            f.validate(),
            Err(ParseError::RecipientPrefix { .. })
        ));
    }

    #[test]
    fn duplicate_cohort_id_fails() {
        let mut f = sample();
        f.cohorts.push(f.cohorts[0].clone());
        assert!(matches!(
            f.validate(),
            Err(ParseError::DuplicateCohortId(_))
        ));
    }

    #[test]
    fn duplicate_pubkey_within_cohort_fails() {
        let mut f = sample();
        let dup = f.cohorts[0].pubkeys[0].clone();
        f.cohorts[0].pubkeys.push(dup);
        assert!(matches!(
            f.validate(),
            Err(ParseError::DuplicateRecipient { .. })
        ));
    }

    #[test]
    fn invalid_cohort_id_fails() {
        let mut f = sample();
        f.cohorts[0].id = "Has/Slash".to_string();
        assert!(matches!(f.validate(), Err(ParseError::InvalidCohortId(_))));
        f.cohorts[0].id = "-leading-dash".to_string();
        assert!(matches!(f.validate(), Err(ParseError::InvalidCohortId(_))));
        f.cohorts[0].id = "UPPER".to_string();
        assert!(matches!(f.validate(), Err(ParseError::InvalidCohortId(_))));
        f.cohorts[0].id = "".to_string();
        assert!(matches!(f.validate(), Err(ParseError::InvalidCohortId(_))));
    }

    #[test]
    fn empty_pages_glob_fails() {
        let mut f = sample();
        f.cohorts[0].pages = Some("   ".to_string());
        assert!(matches!(f.validate(), Err(ParseError::EmptyPages(_))));
    }

    #[test]
    fn unknown_field_rejected() {
        let bad = format!(
            r#"
            version = 1
            [vault]
            signing_pubkey = "{GOOD_SIGNING_PUBKEY}"
            weird = 1
            "#
        );
        let err = RecipientsFile::parse(&bad).unwrap_err();
        assert!(matches!(err, ParseError::Toml(_)), "err was: {err:?}");
    }

    #[test]
    fn cohort_ids_returns_set() {
        let mut f = sample();
        f.cohorts.push(Cohort {
            id: "ops".to_string(),
            name: None,
            mode: CohortMode::WebauthnPrf,
            pubkeys: vec![],
            pages: None,
        });
        let ids = f.cohort_ids();
        assert!(ids.contains("engineering"));
        assert!(ids.contains("ops"));
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn recipient_charset_rejects_bad_char() {
        let mut f = sample();
        f.cohorts[0].pubkeys[0] = format!("{AGE_RECIPIENT_V1_PREFIX}abc!def");
        assert!(matches!(
            f.validate(),
            Err(ParseError::RecipientCharset { ch: '!', .. })
        ));
    }
}
