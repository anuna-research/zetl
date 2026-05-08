//! Republication-eligibility decision per REQ-3815 + REQ-3820 +
//! ADR-3809.
//!
//! Pure: walks the eligibility table from REQ-3820 against the resolved
//! [`License`], the per-subscription operator settings, and the
//! receiving vault's `[wiki].self_license` / `is_commercial` declaration.
//! Returns a [`RepublicationDecision`] carrying both the mode (`Deny` /
//! `ExcerptOnly` / `FullAllowed`) and a structured rationale naming the
//! REQ-3820 row that fired so audits + observability can attribute every
//! decision back to the spec.

use crate::feed::types::{
    License, RepublicationDecision, RepublicationMode, RepublicationRationale,
};

/// Per-subscription operator settings consumed by [`republication_eligible`].
/// Mirrors the relevant fields of `[[subscriptions]]` per CON-3811.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionPolicy {
    /// `republish: bool`. When `false`, every decision is `Deny`
    /// regardless of license.
    pub republish: bool,
    /// `republish_mode: full | excerpt`. Operator preference; the table
    /// may downgrade `Full` to `ExcerptOnly` when license rules require.
    pub mode: RepublicationMode,
    /// `i_have_permission: bool`. When the source declares no
    /// recognisable license, only `true` here unlocks any republication
    /// at all (REQ-3820 last row, ADR-3809 accepted-risk).
    pub i_have_permission: bool,
}

/// Decide republication eligibility against REQ-3820's table.
///
/// Behavioural summary (one row per [`License`] variant):
///   * `Cc0_1_0`            -> `FullAllowed`
///   * `CcBy*` (any version)
///       -> `Full` if operator opted into Full, else `ExcerptOnly`
///   * `CcBySa4_0`          -> `Full` only if vault_self_license is
///                              compatible CC-BY-SA, else `ExcerptOnly`
///   * `CcByNc4_0`          -> `Full` only if vault is non-commercial,
///                              else `ExcerptOnly`
///   * `CcByNd4_0`          -> `ExcerptOnly` always (derivatives blocked,
///                              and wikilink-rewriting modifies bodies)
///   * `Other(_)` / `Unknown`
///       -> `Deny` unless operator set `i_have_permission=true`, then
///          `Full` if operator chose Full else `ExcerptOnly`
///
/// `republish=false` short-circuits to `Deny` first; downstream test
/// matrix in [`crate::feed::tests`] (Phase 2 task-tests-pure-core)
/// covers every row.
pub fn republication_eligible(
    license: &License,
    policy: &SubscriptionPolicy,
    vault_self_license: Option<&License>,
    vault_is_commercial: bool,
) -> RepublicationDecision {
    if !policy.republish {
        return RepublicationDecision {
            mode: RepublicationMode::Deny,
            rationale: RepublicationRationale::UnknownDefaultDeny,
        };
    }

    match license {
        License::Cc0_1_0 => RepublicationDecision {
            mode: RepublicationMode::FullAllowed,
            rationale: RepublicationRationale::PublicDomain,
        },
        License::CcBy4_0 | License::CcBy3_0 => match policy.mode {
            RepublicationMode::FullAllowed => RepublicationDecision {
                mode: RepublicationMode::FullAllowed,
                rationale: RepublicationRationale::CcByFull,
            },
            _ => RepublicationDecision {
                mode: RepublicationMode::ExcerptOnly,
                rationale: RepublicationRationale::CcByExcerpt,
            },
        },
        License::CcBySa4_0 => {
            let compatible = matches!(vault_self_license, Some(License::CcBySa4_0));
            if compatible && policy.mode == RepublicationMode::FullAllowed {
                RepublicationDecision {
                    mode: RepublicationMode::FullAllowed,
                    rationale: RepublicationRationale::CcBySaCompatible,
                }
            } else {
                RepublicationDecision {
                    mode: RepublicationMode::ExcerptOnly,
                    rationale: RepublicationRationale::CcBySaIncompatible,
                }
            }
        }
        License::CcByNc4_0 => {
            if !vault_is_commercial && policy.mode == RepublicationMode::FullAllowed {
                RepublicationDecision {
                    mode: RepublicationMode::FullAllowed,
                    rationale: RepublicationRationale::CcByNcNonCommercial,
                }
            } else {
                RepublicationDecision {
                    mode: RepublicationMode::ExcerptOnly,
                    rationale: RepublicationRationale::CcByNcCommercial,
                }
            }
        }
        License::CcByNd4_0 => RepublicationDecision {
            mode: RepublicationMode::ExcerptOnly,
            rationale: RepublicationRationale::CcByNdExcerptOnly,
        },
        License::Other(_) | License::Unknown => {
            if policy.i_have_permission {
                let mode = if policy.mode == RepublicationMode::FullAllowed {
                    RepublicationMode::FullAllowed
                } else {
                    RepublicationMode::ExcerptOnly
                };
                RepublicationDecision {
                    mode,
                    rationale: RepublicationRationale::UnknownOperatorPermitted,
                }
            } else {
                RepublicationDecision {
                    mode: RepublicationMode::Deny,
                    rationale: RepublicationRationale::UnknownDefaultDeny,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_policy() -> SubscriptionPolicy {
        SubscriptionPolicy {
            republish: true,
            mode: RepublicationMode::FullAllowed,
            i_have_permission: false,
        }
    }
    fn excerpt_policy() -> SubscriptionPolicy {
        SubscriptionPolicy {
            republish: true,
            mode: RepublicationMode::ExcerptOnly,
            i_have_permission: false,
        }
    }

    #[test]
    fn cc0_full_always() {
        let d = republication_eligible(&License::Cc0_1_0, &full_policy(), None, false);
        assert_eq!(d.mode, RepublicationMode::FullAllowed);
        assert_eq!(d.rationale, RepublicationRationale::PublicDomain);
    }

    #[test]
    fn ccby_respects_operator_mode() {
        let d_full = republication_eligible(&License::CcBy4_0, &full_policy(), None, false);
        assert_eq!(d_full.mode, RepublicationMode::FullAllowed);
        assert_eq!(d_full.rationale, RepublicationRationale::CcByFull);
        let d_excerpt = republication_eligible(&License::CcBy4_0, &excerpt_policy(), None, false);
        assert_eq!(d_excerpt.mode, RepublicationMode::ExcerptOnly);
    }

    #[test]
    fn ccbysa_full_only_when_self_license_compatible() {
        // Compatible vault.
        let d = republication_eligible(
            &License::CcBySa4_0,
            &full_policy(),
            Some(&License::CcBySa4_0),
            false,
        );
        assert_eq!(d.mode, RepublicationMode::FullAllowed);
        assert_eq!(d.rationale, RepublicationRationale::CcBySaCompatible);
        // Incompatible vault (declares CC-BY).
        let d = republication_eligible(
            &License::CcBySa4_0,
            &full_policy(),
            Some(&License::CcBy4_0),
            false,
        );
        assert_eq!(d.mode, RepublicationMode::ExcerptOnly);
        // No declared self_license.
        let d = republication_eligible(&License::CcBySa4_0, &full_policy(), None, false);
        assert_eq!(d.mode, RepublicationMode::ExcerptOnly);
    }

    #[test]
    fn ccbync_full_only_when_non_commercial() {
        let d = republication_eligible(&License::CcByNc4_0, &full_policy(), None, false);
        assert_eq!(d.mode, RepublicationMode::FullAllowed);
        assert_eq!(d.rationale, RepublicationRationale::CcByNcNonCommercial);
        let d = republication_eligible(&License::CcByNc4_0, &full_policy(), None, true);
        assert_eq!(d.mode, RepublicationMode::ExcerptOnly);
        assert_eq!(d.rationale, RepublicationRationale::CcByNcCommercial);
    }

    #[test]
    fn ccbynd_excerpt_only_always() {
        for is_commercial in [false, true] {
            let d = republication_eligible(
                &License::CcByNd4_0,
                &full_policy(),
                None,
                is_commercial,
            );
            assert_eq!(d.mode, RepublicationMode::ExcerptOnly);
            assert_eq!(d.rationale, RepublicationRationale::CcByNdExcerptOnly);
        }
    }

    #[test]
    fn unknown_default_deny() {
        let d = republication_eligible(&License::Unknown, &full_policy(), None, false);
        assert_eq!(d.mode, RepublicationMode::Deny);
        assert_eq!(d.rationale, RepublicationRationale::UnknownDefaultDeny);
    }

    #[test]
    fn unknown_with_operator_permission_allows_excerpt_or_full() {
        let mut policy = excerpt_policy();
        policy.i_have_permission = true;
        let d = republication_eligible(&License::Unknown, &policy, None, false);
        assert_eq!(d.mode, RepublicationMode::ExcerptOnly);
        assert_eq!(d.rationale, RepublicationRationale::UnknownOperatorPermitted);
        policy.mode = RepublicationMode::FullAllowed;
        let d = republication_eligible(&License::Unknown, &policy, None, false);
        assert_eq!(d.mode, RepublicationMode::FullAllowed);
    }

    #[test]
    fn republish_false_short_circuits_to_deny() {
        let mut policy = full_policy();
        policy.republish = false;
        let d = republication_eligible(&License::Cc0_1_0, &policy, None, false);
        assert_eq!(d.mode, RepublicationMode::Deny);
    }

    #[test]
    fn vault_is_commercial_only_changes_ccbync() {
        // For every license that isn't CC-BY-NC, flipping is_commercial
        // should not change the mode (REQ-3820 §non-commercial column).
        for lic in [
            License::Cc0_1_0,
            License::CcBy4_0,
            License::CcBy3_0,
            License::CcBySa4_0,
            License::CcByNd4_0,
        ] {
            let a = republication_eligible(&lic, &full_policy(), Some(&License::CcBySa4_0), false);
            let b = republication_eligible(&lic, &full_policy(), Some(&License::CcBySa4_0), true);
            assert_eq!(a.mode, b.mode, "license {lic:?} commercial-flip");
        }
    }
}
