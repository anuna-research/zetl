//! Pure-core helpers for the revocation + lifecycle verbs
//! (SPEC-034 REQ-3416 / REQ-3426 / REQ-3408 §9).
//!
//! Every helper in this module operates on in-memory views of the
//! capability-mode configuration files. No filesystem reads, no wall
//! clock, no OS-CSPRNG entrance point. The effectful shell
//! (`src/main.rs::cmd_cap_revoke` and siblings) supplies the clock
//! (as an RFC 3339 string) and — where rotation is involved —
//! freshly-sampled randomness, then writes the mutated files back
//! atomically.
//!
//! Covered verbs:
//!
//! - [`revoke_grant`] — set `revoked=true` on a grant by id
//!   (REQ-3416 `ztl cap revoke`).
//! - [`finalise_grant`] — set `bound=true` on a grant
//!   (REQ-3416 / REQ-3426 `ztl cap finalise`). The `--rotate-grant`
//!   branch (reissuing priv_A) is orchestrated in the shell because it
//!   needs both randomness and cohort-pubkey updates; the shell calls
//!   [`replace_grant_recipient`] to perform the in-memory swap.
//! - [`sweep_expired`] — mark every past-expires grant revoked
//!   (REQ-3416 `ztl cap sweep`).
//! - [`check_grants`] — produce a report for the stale-grant audit
//!   (REQ-3416 `ztl cap check`; exits 1 in the shell when any grant
//!   has expired since the last build).
//! - [`rotate_cohort_salt`] — record a new content-key salt on a
//!   cohort without touching `salt_stable` (REQ-3402 / BUG-023 URL
//!   stability; `ztl cap rotate --cohort`).
//! - [`replace_vault_signing_pubkey`] — write a new Ed25519 pubkey to
//!   `recipients.toml::[vault].signing_pubkey` (REQ-3427
//!   `ztl cap rotate-signing-key`).
//!
//! The "is this RFC 3339 timestamp in the past?" check is lexicographic
//! over `Z`-suffixed strings — exactly the same comparison the build
//! driver uses in [`crate::cap::build::driver::run_capability_build`].
//! The grants-file validator already rejects non-`Z` offsets, so byte-
//! ordering matches calendar-ordering for every timestamp this module
//! ever sees.

use crate::cap::grants::validation::{Grant, GrantsFile};
use crate::cap::recipients::parsing::{
    Cohort, RecipientsFile, AGE_RECIPIENT_V1_PREFIX, ED25519_PUBKEY_PREFIX,
};

/// Errors returned by the pure-core revocation helpers. Variants are
/// typed (no `anyhow`) so the shell can route each to an exit code and
/// a remediation line — CI loops in particular want `GrantNotFound`
/// and `CohortNotFound` to be distinguishable from a malformed TOML.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RevocationError {
    #[error("grant {0:?} not found in grants.toml")]
    GrantNotFound(String),
    #[error("cohort {0:?} not found in recipients.toml")]
    CohortNotFound(String),
    #[error(
        "recipient pubkey {recipient:?} for grant {grant_id:?} is not present in cohort \
         {cohort:?}; re-issue the grant via `ztl cap invite`"
    )]
    RecipientNotInCohort {
        grant_id: String,
        cohort: String,
        recipient: String,
    },
    #[error(
        "signing-pubkey {0:?} does not carry the `{prefix}` prefix",
        prefix = ED25519_PUBKEY_PREFIX
    )]
    BadSigningPubkey(String),
}

/// Outcome of a revoke. The shell reads `already_revoked` to pick
/// between the "done" and "no-op" exit paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevokeOutcome {
    pub grant_id: String,
    pub cohort: String,
    pub recipient: String,
    pub already_revoked: bool,
}

/// Set `revoked=true` on the grant whose id matches `grant_id`. No-op
/// if the grant was already revoked; the caller can still detect that
/// via [`RevokeOutcome::already_revoked`].
pub fn revoke_grant(
    grants: &mut GrantsFile,
    grant_id: &str,
) -> Result<RevokeOutcome, RevocationError> {
    let g = grants
        .grants
        .iter_mut()
        .find(|g| g.id == grant_id)
        .ok_or_else(|| RevocationError::GrantNotFound(grant_id.to_string()))?;
    let already_revoked = g.revoked;
    g.revoked = true;
    Ok(RevokeOutcome {
        grant_id: g.id.clone(),
        cohort: g.cohort.clone(),
        recipient: g.recipient.clone(),
        already_revoked,
    })
}

/// Outcome of a finalise call. The shell prints both fields so the
/// operator sees whether their intent (confirm TOFU) had already taken
/// effect in a prior run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinaliseOutcome {
    pub grant_id: String,
    pub cohort: String,
    pub recipient: String,
    /// `true` iff the grant already carried `bound=true` on entry.
    pub already_bound: bool,
}

/// Set `bound=true` on the grant whose id matches `grant_id`. The
/// `--rotate-grant` branch is orchestrated from the shell because it
/// needs both randomness and cohort-pubkey updates; this helper
/// handles only the `bound=true` transition.
pub fn finalise_grant(
    grants: &mut GrantsFile,
    grant_id: &str,
) -> Result<FinaliseOutcome, RevocationError> {
    let g = grants
        .grants
        .iter_mut()
        .find(|g| g.id == grant_id)
        .ok_or_else(|| RevocationError::GrantNotFound(grant_id.to_string()))?;
    let already_bound = g.bound;
    g.bound = true;
    Ok(FinaliseOutcome {
        grant_id: g.id.clone(),
        cohort: g.cohort.clone(),
        recipient: g.recipient.clone(),
        already_bound,
    })
}

/// Rotate the recipient pubkey on a grant. Used by
/// `ztl cap finalise --rotate-grant`: the shell generates a fresh
/// (priv_A, pub_A) keypair, swaps the cohort's old pubkey for the new
/// one in `recipients.toml`, and calls this to update the grant row.
///
/// The grant's `bound` flag is reset to `false` — a rotation retires
/// the old TOFU binding, so the reader has not bound on the new URL
/// yet (REQ-3426 documents finalise-with-rotate-grant as a reissue).
pub fn replace_grant_recipient<'a>(
    grants: &'a mut GrantsFile,
    grant_id: &str,
    new_recipient: String,
) -> Result<&'a Grant, RevocationError> {
    let g = grants
        .grants
        .iter_mut()
        .find(|g| g.id == grant_id)
        .ok_or_else(|| RevocationError::GrantNotFound(grant_id.to_string()))?;
    g.recipient = new_recipient;
    g.bound = false;
    Ok(g)
}

/// Replace a cohort's pubkey entry. Used by the `--rotate-grant`
/// branch of finalise: drop the old `age-recipient-v1:<pub_A_old>`
/// and add `<pub_A_new>`. Returns [`RevocationError::CohortNotFound`]
/// when the cohort id isn't present and
/// [`RevocationError::RecipientNotInCohort`] when the old pubkey isn't
/// in the cohort's `pubkeys` array.
pub fn swap_cohort_pubkey(
    recipients: &mut RecipientsFile,
    cohort_id: &str,
    old_recipient: &str,
    new_recipient: String,
    grant_id: &str,
) -> Result<(), RevocationError> {
    let cohort = recipients
        .cohorts
        .iter_mut()
        .find(|c| c.id == cohort_id)
        .ok_or_else(|| RevocationError::CohortNotFound(cohort_id.to_string()))?;
    let idx = cohort
        .pubkeys
        .iter()
        .position(|p| p == old_recipient)
        .ok_or_else(|| RevocationError::RecipientNotInCohort {
            grant_id: grant_id.to_string(),
            cohort: cohort_id.to_string(),
            recipient: old_recipient.to_string(),
        })?;
    cohort.pubkeys[idx] = new_recipient;
    Ok(())
}

/// Outcome of a sweep: every grant newly marked revoked in this call,
/// plus the ids that were already revoked on entry (surfaced so the
/// shell can emit an "already revoked; no change" line without
/// claiming a false positive).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SweepOutcome {
    /// Grants that were past-expires and not revoked on entry, now
    /// flipped to `revoked=true` by this call.
    pub newly_revoked: Vec<String>,
    /// Grants that were past-expires AND already revoked — included so
    /// `ztl cap sweep --json` can report the full expired-set.
    pub already_revoked_expired: Vec<String>,
    /// Grants whose `expires` is in the future (or absent) — untouched.
    pub active: usize,
}

/// Mark every grant whose `expires <= now` as revoked. Idempotent;
/// running sweep twice has no additional effect.
pub fn sweep_expired(grants: &mut GrantsFile, now: &str) -> SweepOutcome {
    let mut out = SweepOutcome::default();
    for g in grants.grants.iter_mut() {
        if is_expired(g, now) {
            if g.revoked {
                out.already_revoked_expired.push(g.id.clone());
            } else {
                g.revoked = true;
                out.newly_revoked.push(g.id.clone());
            }
        } else {
            out.active += 1;
        }
    }
    out
}

/// Per-grant record used by [`check_grants`]. Surfacing the cohort +
/// expires timestamp lets CI pipelines group-by cohort without a
/// second parse of grants.toml.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpiredGrantRecord {
    pub grant_id: String,
    pub cohort: String,
    pub expires: String,
    pub revoked: bool,
}

/// Report produced by `ztl cap check`. The shell exits 1 when
/// `expired_unrevoked` is non-empty — that's the CI gate REQ-3416
/// alludes to ("exits 1 if any grant has expired since last build").
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CheckReport {
    /// Grants whose `expires <= now` AND `revoked=false`. These are
    /// the audit-failures that should block a CI job.
    pub expired_unrevoked: Vec<ExpiredGrantRecord>,
    /// Grants whose `expires <= now` AND `revoked=true` — informational.
    /// The shell prints them under a "cleanup OK" heading so operators
    /// can distinguish "needs action" from "already handled".
    pub expired_revoked: Vec<ExpiredGrantRecord>,
    /// Grants whose `expires` is in the future (or absent).
    pub active: usize,
}

impl CheckReport {
    /// Does the report warrant a non-zero exit code?
    pub fn is_failure(&self) -> bool {
        !self.expired_unrevoked.is_empty()
    }
}

/// Walk the grants and categorise each against `now`. Pure function of
/// the input arguments.
pub fn check_grants(grants: &GrantsFile, now: &str) -> CheckReport {
    let mut out = CheckReport::default();
    for g in &grants.grants {
        if is_expired(g, now) {
            let rec = ExpiredGrantRecord {
                grant_id: g.id.clone(),
                cohort: g.cohort.clone(),
                expires: g.expires.clone().unwrap_or_default(),
                revoked: g.revoked,
            };
            if g.revoked {
                out.expired_revoked.push(rec);
            } else {
                out.expired_unrevoked.push(rec);
            }
        } else {
            out.active += 1;
        }
    }
    out
}

/// Record a cohort rotation: set `salt_rotated` to `new_salt_b64url`
/// and `last_rotated` to `now_rfc3339`. Returns
/// [`RevocationError::CohortNotFound`] when `cohort_id` isn't present.
///
/// Pure: the caller (shell) samples `new_salt_b64url` from OsRng and
/// reads `now_rfc3339` from `SystemTime::now()`. The field is stored
/// as base64url verbatim — this module does NOT encode/decode base64,
/// so the shell can pick whichever encoding it prefers (the existing
/// `salt_stable` convention is base64url-unpadded).
pub fn rotate_cohort_salt<'a>(
    recipients: &'a mut RecipientsFile,
    cohort_id: &str,
    new_salt_b64url: String,
    now_rfc3339: String,
) -> Result<&'a Cohort, RevocationError> {
    let cohort = recipients
        .cohorts
        .iter_mut()
        .find(|c| c.id == cohort_id)
        .ok_or_else(|| RevocationError::CohortNotFound(cohort_id.to_string()))?;
    cohort.salt_rotated = Some(new_salt_b64url);
    cohort.last_rotated = Some(now_rfc3339);
    Ok(cohort)
}

/// Replace `[vault].signing_pubkey` with `new_pubkey`. The supplied
/// string must already carry the `ed25519:` prefix (REQ-3409); callers
/// that hold a raw 32-byte pubkey should format it themselves.
pub fn replace_vault_signing_pubkey(
    recipients: &mut RecipientsFile,
    new_pubkey: String,
) -> Result<(), RevocationError> {
    if !new_pubkey.starts_with(ED25519_PUBKEY_PREFIX) {
        return Err(RevocationError::BadSigningPubkey(new_pubkey));
    }
    recipients.vault.signing_pubkey = new_pubkey;
    Ok(())
}

/// Format a raw 32-byte Ed25519 pubkey as the canonical
/// `ed25519:<base64url>` wire string expected by [`replace_vault_signing_pubkey`].
pub fn encode_signing_pubkey(raw: &[u8; 32]) -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;
    format!("{ED25519_PUBKEY_PREFIX}{}", URL_SAFE_NO_PAD.encode(raw))
}

/// Format a raw 32-byte X25519 pubkey as `age-recipient-v1:<b64url>`.
/// Re-exported here so the shell can build the replacement recipient
/// string without reaching into `cap::invite` (which owns the private-
/// key wrapper — a different abstraction layer).
pub fn encode_age_recipient(raw: &[u8; 32]) -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;
    format!("{AGE_RECIPIENT_V1_PREFIX}{}", URL_SAFE_NO_PAD.encode(raw))
}

/// Is this grant expired relative to `now`?
///
/// RFC 3339 `Z`-suffixed strings sort correctly as bytes, so this is a
/// valid comparison under the format pinned by
/// `cap::grants::validation::is_rfc3339`. A grant with no `expires`
/// never expires.
fn is_expired(g: &Grant, now: &str) -> bool {
    match g.expires.as_deref() {
        Some(exp) => exp.as_bytes() <= now.as_bytes(),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cap::grants::validation::{Grant, GrantMode, GrantsFile};
    use crate::cap::recipients::parsing::{Cohort, CohortMode, RecipientsFile, VaultSection};

    fn gk(id: &str, cohort: &str, expires: Option<&str>, revoked: bool) -> Grant {
        Grant {
            id: id.to_string(),
            cohort: cohort.to_string(),
            recipient: format!("{AGE_RECIPIENT_V1_PREFIX}AAAA"),
            mode: GrantMode::DelegatedUrl,
            bound: false,
            name: None,
            created: "2026-01-01T00:00:00Z".to_string(),
            expires: expires.map(String::from),
            revoked,
            pages: "*".to_string(),
        }
    }

    fn sample_grants() -> GrantsFile {
        GrantsFile {
            version: Some(1),
            grants: vec![
                gk("g_a", "eng", Some("2026-12-01T00:00:00Z"), false),
                gk("g_b", "eng", Some("2025-06-01T00:00:00Z"), false),
                gk("g_c", "ops", Some("2025-06-01T00:00:00Z"), true),
                gk("g_d", "eng", None, false),
            ],
        }
    }

    fn sample_recipients() -> RecipientsFile {
        RecipientsFile {
            version: 1,
            vault: VaultSection {
                signing_pubkey: "ed25519:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
            },
            cohorts: vec![Cohort {
                id: "eng".to_string(),
                name: None,
                mode: CohortMode::DelegatedUrl,
                pubkeys: vec![format!("{AGE_RECIPIENT_V1_PREFIX}oldkey")],
                pages: None,
                salt_stable: None,
                salt_rotated: None,
                last_rotated: None,
            }],
        }
    }

    #[test]
    fn revoke_sets_flag_and_is_idempotent() {
        let mut g = sample_grants();
        let out = revoke_grant(&mut g, "g_a").unwrap();
        assert!(!out.already_revoked);
        assert!(g.grants[0].revoked);

        let out2 = revoke_grant(&mut g, "g_a").unwrap();
        assert!(out2.already_revoked);
        assert!(g.grants[0].revoked);
    }

    #[test]
    fn revoke_unknown_id_errors() {
        let mut g = sample_grants();
        let err = revoke_grant(&mut g, "g_zzz").unwrap_err();
        assert!(matches!(err, RevocationError::GrantNotFound(_)));
    }

    #[test]
    fn finalise_sets_bound_and_is_idempotent() {
        let mut g = sample_grants();
        let out = finalise_grant(&mut g, "g_a").unwrap();
        assert!(!out.already_bound);
        assert!(g.grants[0].bound);

        let out2 = finalise_grant(&mut g, "g_a").unwrap();
        assert!(out2.already_bound);
    }

    #[test]
    fn replace_grant_recipient_resets_bound() {
        let mut g = sample_grants();
        // Pre-finalise so we can watch rotate reset the bit.
        finalise_grant(&mut g, "g_a").unwrap();
        assert!(g.grants[0].bound);

        let new = format!("{AGE_RECIPIENT_V1_PREFIX}newkey");
        let _ = replace_grant_recipient(&mut g, "g_a", new.clone()).unwrap();
        assert_eq!(g.grants[0].recipient, new);
        assert!(!g.grants[0].bound, "rotate-grant must reset bound=false");
    }

    #[test]
    fn swap_cohort_pubkey_swaps() {
        let mut r = sample_recipients();
        let old = format!("{AGE_RECIPIENT_V1_PREFIX}oldkey");
        let new = format!("{AGE_RECIPIENT_V1_PREFIX}newkey");
        swap_cohort_pubkey(&mut r, "eng", &old, new.clone(), "g_a").unwrap();
        assert_eq!(r.cohorts[0].pubkeys, vec![new]);
    }

    #[test]
    fn swap_cohort_pubkey_rejects_unknown_cohort() {
        let mut r = sample_recipients();
        let err =
            swap_cohort_pubkey(&mut r, "nonexistent", "x", "y".to_string(), "g_a").unwrap_err();
        assert!(matches!(err, RevocationError::CohortNotFound(_)));
    }

    #[test]
    fn swap_cohort_pubkey_rejects_unknown_recipient() {
        let mut r = sample_recipients();
        let err =
            swap_cohort_pubkey(&mut r, "eng", "missing", "new".to_string(), "g_a").unwrap_err();
        assert!(matches!(err, RevocationError::RecipientNotInCohort { .. }));
    }

    #[test]
    fn sweep_marks_past_expires_revoked() {
        let mut g = sample_grants();
        let out = sweep_expired(&mut g, "2026-04-20T00:00:00Z");
        assert_eq!(out.newly_revoked, vec!["g_b"]);
        assert_eq!(out.already_revoked_expired, vec!["g_c"]);
        assert_eq!(out.active, 2); // g_a (future) + g_d (no expires)
        assert!(g.grants[1].revoked);
        assert!(g.grants[2].revoked); // already was
        assert!(!g.grants[0].revoked);
        assert!(!g.grants[3].revoked);
    }

    #[test]
    fn sweep_is_idempotent() {
        let mut g = sample_grants();
        sweep_expired(&mut g, "2026-04-20T00:00:00Z");
        let out2 = sweep_expired(&mut g, "2026-04-20T00:00:00Z");
        assert!(out2.newly_revoked.is_empty());
        assert_eq!(out2.already_revoked_expired, vec!["g_b", "g_c"]);
    }

    #[test]
    fn check_report_partitions_correctly() {
        let grants = sample_grants();
        let report = check_grants(&grants, "2026-04-20T00:00:00Z");
        assert_eq!(report.expired_unrevoked.len(), 1);
        assert_eq!(report.expired_unrevoked[0].grant_id, "g_b");
        assert_eq!(report.expired_revoked.len(), 1);
        assert_eq!(report.expired_revoked[0].grant_id, "g_c");
        assert_eq!(report.active, 2);
        assert!(report.is_failure());
    }

    #[test]
    fn check_report_is_clean_when_no_expires_in_past() {
        let grants = sample_grants();
        let report = check_grants(&grants, "2024-01-01T00:00:00Z");
        assert_eq!(report.expired_unrevoked.len(), 0);
        assert_eq!(report.expired_revoked.len(), 0);
        assert!(!report.is_failure());
    }

    #[test]
    fn rotate_cohort_salt_records_fields_without_touching_salt_stable() {
        let mut r = sample_recipients();
        let before = r.cohorts[0].salt_stable.clone();
        let out = rotate_cohort_salt(
            &mut r,
            "eng",
            "newsalt_b64url".to_string(),
            "2026-04-21T00:00:00Z".to_string(),
        )
        .unwrap();
        assert_eq!(out.salt_rotated.as_deref(), Some("newsalt_b64url"));
        assert_eq!(out.last_rotated.as_deref(), Some("2026-04-21T00:00:00Z"));
        // REQ-3402 / BUG-023: salt_stable must be untouched so URLs stay
        // stable across rotations.
        assert_eq!(r.cohorts[0].salt_stable, before);
    }

    #[test]
    fn rotate_cohort_salt_rejects_unknown_cohort() {
        let mut r = sample_recipients();
        let err = rotate_cohort_salt(
            &mut r,
            "no-such-cohort",
            "x".to_string(),
            "2026-01-01T00:00:00Z".to_string(),
        )
        .unwrap_err();
        assert!(matches!(err, RevocationError::CohortNotFound(_)));
    }

    #[test]
    fn replace_vault_signing_pubkey_overwrites() {
        let mut r = sample_recipients();
        let new = "ed25519:ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ".to_string();
        replace_vault_signing_pubkey(&mut r, new.clone()).unwrap();
        assert_eq!(r.vault.signing_pubkey, new);
    }

    #[test]
    fn replace_vault_signing_pubkey_rejects_bare_pubkey() {
        let mut r = sample_recipients();
        let err = replace_vault_signing_pubkey(&mut r, "not-prefixed".to_string()).unwrap_err();
        assert!(matches!(err, RevocationError::BadSigningPubkey(_)));
    }

    #[test]
    fn encode_signing_pubkey_carries_prefix() {
        let raw = [0u8; 32];
        let s = encode_signing_pubkey(&raw);
        assert!(s.starts_with(ED25519_PUBKEY_PREFIX));
        assert_eq!(s.len(), ED25519_PUBKEY_PREFIX.len() + 43);
    }

    #[test]
    fn encode_age_recipient_carries_prefix() {
        let raw = [1u8; 32];
        let s = encode_age_recipient(&raw);
        assert!(s.starts_with(AGE_RECIPIENT_V1_PREFIX));
    }

    #[test]
    fn is_expired_handles_missing_and_boundary() {
        let g_never = gk("g_never", "eng", None, false);
        assert!(!is_expired(&g_never, "2099-12-31T23:59:59Z"));
        let g_past = gk("g_past", "eng", Some("2024-01-01T00:00:00Z"), false);
        assert!(is_expired(&g_past, "2024-01-01T00:00:01Z"));
        // equality counts as expired (lexicographic <=)
        assert!(is_expired(&g_past, "2024-01-01T00:00:00Z"));
    }
}
