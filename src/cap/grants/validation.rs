//! `grants.toml` schema + structural invariants (SPEC-034 CON-3402).
//!
//! ```toml
//! [[grant]]
//! id         = "g_01JABC..."
//! cohort     = "engineering"
//! recipient  = "age-recipient-v1:<b64url-X25519-pubkey>"
//! mode       = "delegated-url"                 # or "webauthn-prf"
//! bound      = false                            # flipped by `ztl cap finalise`
//! name       = "Alice Jones"                   # optional
//! created    = "2026-04-20T14:22:00Z"
//! expires    = "2026-10-20T00:00:00Z"          # optional
//! revoked    = false
//! pages      = "*"
//! ```
//!
//! This module enforces:
//!
//! - `deny_unknown_fields` on every struct (typo-catches are security
//!   controls: a mistyped `revoked = ture` that silently defaults to
//!   `false` would leave revoked readers decrypting).
//! - Unique grant IDs (required; a duplicate id shadowing downstream
//!   state is a silent revocation hole).
//! - Grant `cohort` is present in the caller-supplied cohort set (pure
//!   core takes the set as an argument; the file read lives in the
//!   shell).
//! - `recipient` carries the REQ-3409 `age-recipient-v1:` prefix.
//! - `mode` is one of `delegated-url` / `webauthn-prf`.
//! - `created` / `expires` parse as RFC 3339 (we do not pull in
//!   `chrono` just for validation; a hand-written check is enough to
//!   reject malformed strings).
//! - `expires > created` when `expires` is present.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::cap::recipients::parsing::AGE_RECIPIENT_V1_PREFIX;

/// Top-level shape of `grants.toml`. TOML rendering puts the grant
/// array under `[[grant]]` tables.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GrantsFile {
    #[serde(default)]
    pub version: Option<u32>,
    #[serde(default, rename = "grant")]
    pub grants: Vec<Grant>,
}

/// One row in `grants.toml`. Field order follows CON-3402 so git diffs
/// show related fields together.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub id: String,
    pub cohort: String,
    pub recipient: String,
    pub mode: GrantMode,
    #[serde(default)]
    pub bound: bool,
    #[serde(default)]
    pub name: Option<String>,
    pub created: String,
    #[serde(default)]
    pub expires: Option<String>,
    #[serde(default)]
    pub revoked: bool,
    #[serde(default = "default_pages")]
    pub pages: String,
}

fn default_pages() -> String {
    "*".to_string()
}

/// The two cohort operating modes (REQ-3404). Serialised as kebab-case
/// on the wire.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum GrantMode {
    DelegatedUrl,
    WebauthnPrf,
}

/// Validation errors. Carry enough context for a CLI renderer to
/// point at the offending grant.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ValidationError {
    #[error("grant id {0:?} is empty or whitespace")]
    EmptyId(String),
    #[error("duplicate grant id {0:?}")]
    DuplicateId(String),
    #[error("grant {grant_id:?} references unknown cohort {cohort:?}")]
    UnknownCohort { grant_id: String, cohort: String },
    #[error(
        "grant {grant_id:?} recipient does not carry the `{prefix}` prefix",
        prefix = AGE_RECIPIENT_V1_PREFIX
    )]
    RecipientPrefix { grant_id: String },
    #[error("grant {grant_id:?} recipient payload is empty")]
    RecipientEmpty { grant_id: String },
    #[error("grant {grant_id:?} recipient payload contains non-base64url character {ch:?}")]
    RecipientCharset { grant_id: String, ch: char },
    #[error("grant {grant_id:?} `created` is not a valid RFC 3339 timestamp")]
    BadCreated { grant_id: String },
    #[error("grant {grant_id:?} `expires` is not a valid RFC 3339 timestamp")]
    BadExpires { grant_id: String },
    #[error("grant {grant_id:?} `expires` is not after `created`")]
    ExpiresBeforeCreated { grant_id: String },
    #[error("grant {grant_id:?} `pages` glob is empty")]
    EmptyPages { grant_id: String },
}

impl GrantsFile {
    /// Validate structural invariants given the set of cohort ids that
    /// exist in `recipients.toml`. Passing an empty set skips the
    /// cross-file check (useful in focused unit tests); full-build
    /// validation always supplies the real cohort set.
    pub fn validate(&self, known_cohorts: &BTreeSet<String>) -> Result<(), ValidationError> {
        let mut seen_ids: BTreeSet<&str> = BTreeSet::new();
        for g in &self.grants {
            g.validate_self()?;
            if !seen_ids.insert(g.id.as_str()) {
                return Err(ValidationError::DuplicateId(g.id.clone()));
            }
            if !known_cohorts.is_empty() && !known_cohorts.contains(&g.cohort) {
                return Err(ValidationError::UnknownCohort {
                    grant_id: g.id.clone(),
                    cohort: g.cohort.clone(),
                });
            }
        }
        Ok(())
    }

    /// Parse a TOML string into `GrantsFile`. A thin wrapper so the
    /// shell caller gets a consistent error surface.
    pub fn from_toml(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }
}

impl Grant {
    fn validate_self(&self) -> Result<(), ValidationError> {
        if self.id.trim().is_empty() {
            return Err(ValidationError::EmptyId(self.id.clone()));
        }
        validate_recipient(&self.id, &self.recipient)?;
        if !is_rfc3339(&self.created) {
            return Err(ValidationError::BadCreated {
                grant_id: self.id.clone(),
            });
        }
        if let Some(exp) = self.expires.as_deref() {
            if !is_rfc3339(exp) {
                return Err(ValidationError::BadExpires {
                    grant_id: self.id.clone(),
                });
            }
            // Lexicographic comparison is correct for normalised RFC 3339
            // timestamps because the format sorts correctly as a string
            // (year-first, fixed-width). We enforce the `Z` suffix below
            // so there's no offset-driven ambiguity.
            if exp.as_bytes() <= self.created.as_bytes() {
                return Err(ValidationError::ExpiresBeforeCreated {
                    grant_id: self.id.clone(),
                });
            }
        }
        if self.pages.trim().is_empty() {
            return Err(ValidationError::EmptyPages {
                grant_id: self.id.clone(),
            });
        }
        Ok(())
    }
}

fn validate_recipient(grant_id: &str, s: &str) -> Result<(), ValidationError> {
    let payload = s.strip_prefix(AGE_RECIPIENT_V1_PREFIX).ok_or_else(|| {
        ValidationError::RecipientPrefix {
            grant_id: grant_id.to_string(),
        }
    })?;
    if payload.is_empty() {
        return Err(ValidationError::RecipientEmpty {
            grant_id: grant_id.to_string(),
        });
    }
    for c in payload.chars() {
        if !is_base64url(c) {
            return Err(ValidationError::RecipientCharset {
                grant_id: grant_id.to_string(),
                ch: c,
            });
        }
    }
    Ok(())
}

fn is_base64url(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

/// Strict RFC 3339 check: `YYYY-MM-DDThh:mm:ssZ` or with
/// `.fractional` seconds. We reject numeric offsets (`+05:00`) — the
/// wire format pins `Z` (UTC) so string comparison is safe in
/// `validate_self`. Not a full RFC 3339 implementation; enough to
/// reject obviously malformed input without pulling in chrono.
fn is_rfc3339(s: &str) -> bool {
    let bytes = s.as_bytes();
    // Minimum is `2026-04-20T00:00:00Z` = 20 chars.
    if bytes.len() < 20 {
        return false;
    }
    let digit = |b: u8| b.is_ascii_digit();
    let is = |i: usize, c: u8| bytes.get(i).copied() == Some(c);
    if !(digit(bytes[0])
        && digit(bytes[1])
        && digit(bytes[2])
        && digit(bytes[3])
        && is(4, b'-')
        && digit(bytes[5])
        && digit(bytes[6])
        && is(7, b'-')
        && digit(bytes[8])
        && digit(bytes[9])
        && is(10, b'T')
        && digit(bytes[11])
        && digit(bytes[12])
        && is(13, b':')
        && digit(bytes[14])
        && digit(bytes[15])
        && is(16, b':')
        && digit(bytes[17])
        && digit(bytes[18]))
    {
        return false;
    }
    // Tail: optional `.<digits>` then `Z`.
    let mut i = 19;
    if bytes.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while bytes.get(i).copied().is_some_and(digit) {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    bytes.get(i) == Some(&b'Z') && i + 1 == bytes.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cohort_set(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    fn good_grant(id: &str) -> Grant {
        Grant {
            id: id.to_string(),
            cohort: "engineering".to_string(),
            recipient: format!("{AGE_RECIPIENT_V1_PREFIX}Alice0_-base64url"),
            mode: GrantMode::DelegatedUrl,
            bound: false,
            name: Some("Alice".to_string()),
            created: "2026-04-20T00:00:00Z".to_string(),
            expires: Some("2026-10-20T00:00:00Z".to_string()),
            revoked: false,
            pages: "*".to_string(),
        }
    }

    #[test]
    fn valid_grant_passes() {
        let f = GrantsFile {
            version: Some(1),
            grants: vec![good_grant("g1")],
        };
        f.validate(&cohort_set(&["engineering"])).unwrap();
    }

    #[test]
    fn duplicate_id_fails() {
        let f = GrantsFile {
            version: None,
            grants: vec![good_grant("g1"), good_grant("g1")],
        };
        let err = f.validate(&cohort_set(&["engineering"])).unwrap_err();
        assert!(matches!(err, ValidationError::DuplicateId(id) if id == "g1"));
    }

    #[test]
    fn unknown_cohort_fails() {
        let f = GrantsFile {
            version: None,
            grants: vec![good_grant("g1")],
        };
        let err = f.validate(&cohort_set(&["other"])).unwrap_err();
        assert!(matches!(err, ValidationError::UnknownCohort { .. }));
    }

    #[test]
    fn empty_cohort_set_skips_cross_check() {
        let f = GrantsFile {
            version: None,
            grants: vec![good_grant("g1")],
        };
        f.validate(&BTreeSet::new()).unwrap();
    }

    #[test]
    fn recipient_missing_prefix_fails() {
        let mut g = good_grant("g1");
        g.recipient = "ssh-ed25519:abc".to_string();
        let f = GrantsFile {
            version: None,
            grants: vec![g],
        };
        assert!(matches!(
            f.validate(&BTreeSet::new()),
            Err(ValidationError::RecipientPrefix { .. })
        ));
    }

    #[test]
    fn recipient_empty_payload_fails() {
        let mut g = good_grant("g1");
        g.recipient = AGE_RECIPIENT_V1_PREFIX.to_string();
        let f = GrantsFile {
            version: None,
            grants: vec![g],
        };
        assert!(matches!(
            f.validate(&BTreeSet::new()),
            Err(ValidationError::RecipientEmpty { .. })
        ));
    }

    #[test]
    fn recipient_bad_char_fails() {
        let mut g = good_grant("g1");
        g.recipient = format!("{AGE_RECIPIENT_V1_PREFIX}abc!");
        let f = GrantsFile {
            version: None,
            grants: vec![g],
        };
        assert!(matches!(
            f.validate(&BTreeSet::new()),
            Err(ValidationError::RecipientCharset { ch: '!', .. })
        ));
    }

    #[test]
    fn bad_created_fails() {
        let mut g = good_grant("g1");
        g.created = "not-a-date".to_string();
        let f = GrantsFile {
            version: None,
            grants: vec![g],
        };
        assert!(matches!(
            f.validate(&BTreeSet::new()),
            Err(ValidationError::BadCreated { .. })
        ));
    }

    #[test]
    fn bad_expires_fails() {
        let mut g = good_grant("g1");
        g.expires = Some("2026/10/20".to_string());
        let f = GrantsFile {
            version: None,
            grants: vec![g],
        };
        assert!(matches!(
            f.validate(&BTreeSet::new()),
            Err(ValidationError::BadExpires { .. })
        ));
    }

    #[test]
    fn expires_before_created_fails() {
        let mut g = good_grant("g1");
        g.expires = Some("2026-01-01T00:00:00Z".to_string());
        let f = GrantsFile {
            version: None,
            grants: vec![g],
        };
        assert!(matches!(
            f.validate(&BTreeSet::new()),
            Err(ValidationError::ExpiresBeforeCreated { .. })
        ));
    }

    #[test]
    fn empty_pages_glob_fails() {
        let mut g = good_grant("g1");
        g.pages = "   ".to_string();
        let f = GrantsFile {
            version: None,
            grants: vec![g],
        };
        assert!(matches!(
            f.validate(&BTreeSet::new()),
            Err(ValidationError::EmptyPages { .. })
        ));
    }

    #[test]
    fn unknown_field_rejected_by_deny_unknown_fields() {
        let bad = r#"
            [[grant]]
            id = "g1"
            cohort = "eng"
            recipient = "age-recipient-v1:abc"
            mode = "delegated-url"
            created = "2026-04-20T00:00:00Z"
            sneaky = true
        "#;
        let err = GrantsFile::from_toml(bad).unwrap_err();
        assert!(err.to_string().contains("sneaky"), "error was: {err}");
    }

    #[test]
    fn fractional_seconds_accepted() {
        assert!(is_rfc3339("2026-04-20T00:00:00.123Z"));
        assert!(is_rfc3339("2026-04-20T00:00:00Z"));
        assert!(!is_rfc3339("2026-04-20T00:00:00"));
        assert!(!is_rfc3339("2026-04-20T00:00:00+05:00"));
        assert!(!is_rfc3339("2026-04-20 00:00:00Z"));
    }

    #[test]
    fn serde_round_trip() {
        let f = GrantsFile {
            version: Some(1),
            grants: vec![good_grant("g1"), good_grant("g2")],
        };
        let s = toml::to_string(&f).unwrap();
        let back = GrantsFile::from_toml(&s).unwrap();
        assert_eq!(f, back);
    }

    #[test]
    fn default_pages_is_star() {
        let s = r#"
            [[grant]]
            id = "g1"
            cohort = "eng"
            recipient = "age-recipient-v1:abc"
            mode = "delegated-url"
            created = "2026-04-20T00:00:00Z"
        "#;
        let f = GrantsFile::from_toml(s).unwrap();
        assert_eq!(f.grants[0].pages, "*");
    }
}
