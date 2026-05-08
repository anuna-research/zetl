//! Retention pruning per REQ-3832 + REQ-3833 + REQ-3835 + ADR-3812 +
//! CON-3814 + OBS-3810.
//!
//! Pure: takes a list of inbox items + retention policy + current
//! time and produces a sequence of [`PruneAction`]s. Caller performs
//! the file-system effects (move into archived/, delete, etc).

use crate::feed::types::RetentionPolicy;
use serde::{Deserialize, Serialize};

/// One inbox item under retention review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxItem {
    /// Vault-relative path under `.zetl/feeds/<sub>/inbox/`.
    pub path: String,
    /// RFC 3339 ingest / publish timestamp.
    pub date_published: String,
    /// Object identity (for OBS-3810 labels).
    pub guid: String,
}

/// Retention mode: archive (default per ADR-3812) or delete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RetentionMode {
    Archive,
    Delete,
}

impl Default for RetentionMode {
    fn default() -> Self {
        RetentionMode::Archive
    }
}

impl RetentionMode {
    pub fn parse(s: Option<&str>) -> Self {
        match s {
            Some("delete") => RetentionMode::Delete,
            _ => RetentionMode::Archive,
        }
    }
}

/// Action the shell should take on a single inbox item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PruneAction {
    /// Move the file from inbox/ to archived/.
    Archive { path: String, reason: PruneReason },
    /// Delete outright (only when retention_mode = delete).
    Delete { path: String, reason: PruneReason },
    /// Keep — within policy.
    Keep { path: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PruneReason {
    /// Item is older than the policy's duration window.
    Age,
    /// Total inbox count exceeds the policy's `Count` cap; this item
    /// is one of the dropped ones.
    Count,
    /// Forgotten via `zetl feed forget`.
    UserRequest,
    /// Pruned because the source-side retraction propagated and we
    /// already removed it from the public republished feed.
    RepublishCoupled,
}

/// Plan retention actions. Pure — sorts items by date desc, applies
/// the policy, returns one [`PruneAction`] per item. The caller owns
/// the file-system effects.
pub fn plan_retention(
    items: &[InboxItem],
    policy: &RetentionPolicy,
    mode: RetentionMode,
    now_rfc3339: &str,
) -> Vec<PruneAction> {
    if items.is_empty() {
        return Vec::new();
    }
    let mut sorted: Vec<&InboxItem> = items.iter().collect();
    sorted.sort_by(|a, b| b.date_published.cmp(&a.date_published));

    let mut out = Vec::with_capacity(sorted.len());
    match policy {
        RetentionPolicy::Forever => {
            for item in &sorted {
                out.push(PruneAction::Keep {
                    path: item.path.clone(),
                });
            }
        }
        RetentionPolicy::Duration { seconds } => {
            let cutoff = subtract_seconds(now_rfc3339, *seconds);
            for item in &sorted {
                if item.date_published.as_str() < cutoff.as_str() {
                    out.push(make_action(&item.path, mode, PruneReason::Age));
                } else {
                    out.push(PruneAction::Keep {
                        path: item.path.clone(),
                    });
                }
            }
        }
        RetentionPolicy::Count { count } => {
            for (idx, item) in sorted.iter().enumerate() {
                if idx < *count {
                    out.push(PruneAction::Keep {
                        path: item.path.clone(),
                    });
                } else {
                    out.push(make_action(&item.path, mode, PruneReason::Count));
                }
            }
        }
    }
    out
}

fn make_action(path: &str, mode: RetentionMode, reason: PruneReason) -> PruneAction {
    match mode {
        RetentionMode::Archive => PruneAction::Archive {
            path: path.to_string(),
            reason,
        },
        RetentionMode::Delete => PruneAction::Delete {
            path: path.to_string(),
            reason,
        },
    }
}

/// RFC 3339 subtraction. Re-exports
/// [`crate::feed::datetime::subtract_seconds_from_rfc3339`] under the
/// retention-local name the rest of the module already uses.
fn subtract_seconds(rfc3339: &str, seconds: i64) -> String {
    crate::feed::datetime::subtract_seconds_from_rfc3339(rfc3339, seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn it(path: &str, date: &str) -> InboxItem {
        InboxItem {
            path: path.to_string(),
            date_published: date.to_string(),
            guid: path.to_string(),
        }
    }

    #[test]
    fn forever_keeps_everything() {
        let items = vec![it("a", "2026-01-01T00:00:00Z"), it("b", "2025-01-01T00:00:00Z")];
        let actions = plan_retention(&items, &RetentionPolicy::Forever, RetentionMode::Archive, "2026-05-08T00:00:00Z");
        for a in &actions {
            assert!(matches!(a, PruneAction::Keep { .. }));
        }
    }

    #[test]
    fn duration_archives_old_items() {
        // 90-day cutoff at 2026-05-08 ≈ 2026-02-07.
        let items = vec![
            it("recent", "2026-04-01T00:00:00Z"),
            it("old", "2025-12-01T00:00:00Z"),
        ];
        let actions = plan_retention(
            &items,
            &RetentionPolicy::Duration {
                seconds: 90 * 24 * 60 * 60,
            },
            RetentionMode::Archive,
            "2026-05-08T00:00:00Z",
        );
        assert!(matches!(actions[0], PruneAction::Keep { .. }));
        assert!(matches!(
            actions[1],
            PruneAction::Archive {
                reason: PruneReason::Age,
                ..
            }
        ));
    }

    #[test]
    fn count_archives_overflow() {
        let items: Vec<InboxItem> = (0..5)
            .map(|i| it(&format!("p{i}"), &format!("2026-05-{:02}T00:00:00Z", i + 1)))
            .collect();
        let actions = plan_retention(
            &items,
            &RetentionPolicy::Count { count: 3 },
            RetentionMode::Archive,
            "2026-05-08T00:00:00Z",
        );
        let kept = actions.iter().filter(|a| matches!(a, PruneAction::Keep { .. })).count();
        let archived = actions.iter().filter(|a| matches!(a, PruneAction::Archive { reason: PruneReason::Count, .. })).count();
        assert_eq!(kept, 3);
        assert_eq!(archived, 2);
    }

    #[test]
    fn delete_mode_produces_delete_actions() {
        let items = vec![it("old", "2025-01-01T00:00:00Z")];
        let actions = plan_retention(
            &items,
            &RetentionPolicy::Duration {
                seconds: 30 * 24 * 60 * 60,
            },
            RetentionMode::Delete,
            "2026-05-08T00:00:00Z",
        );
        assert!(matches!(actions[0], PruneAction::Delete { .. }));
    }

    #[test]
    fn empty_input_yields_empty_plan() {
        let actions = plan_retention(
            &[],
            &RetentionPolicy::Forever,
            RetentionMode::Archive,
            "2026-05-08T00:00:00Z",
        );
        assert!(actions.is_empty());
    }

    #[test]
    fn retention_mode_parse_defaults_to_archive() {
        assert_eq!(RetentionMode::parse(None), RetentionMode::Archive);
        assert_eq!(RetentionMode::parse(Some("archive")), RetentionMode::Archive);
        assert_eq!(RetentionMode::parse(Some("delete")), RetentionMode::Delete);
        assert_eq!(RetentionMode::parse(Some("garbage")), RetentionMode::Archive);
    }

    #[test]
    fn date_subtraction_roundtrips_within_a_year() {
        let result = subtract_seconds("2026-05-08T12:30:45Z", 86_400);
        assert_eq!(result, "2026-05-07T12:30:45Z");
    }

    #[test]
    fn date_subtraction_handles_leap_year() {
        // 2024 is a leap year; subtract 1 day across Feb 29 boundary.
        let result = subtract_seconds("2024-03-01T00:00:00Z", 86_400);
        assert_eq!(result, "2024-02-29T00:00:00Z");
    }
}
