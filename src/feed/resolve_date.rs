//! Date-resolution pure function per REQ-3804.
//!
//! Walks a documented fallback chain for the published date of a page:
//!
//! ```text
//! frontmatter.published
//!   > frontmatter.date
//!   > frontmatter.created
//!   > git first-commit date    (passed in via PageDates; pure core
//!                              never invokes git itself)
//!   > git last-commit date
//!   > <hard error>
//! ```
//!
//! Filesystem mtime is **explicitly NOT** in the chain because it is
//! non-deterministic across hosts — the same vault checked out on two
//! machines can produce different mtimes (NFR-3804 stability).
//!
//! Output is RFC 3339 (ISO 8601 with `T` separator and `Z` / numeric tz
//! offset). The function rejects timezone-naive strings (no `Z`,
//! offset, or named tz suffix) so feed readers always see explicit
//! offsets per the spec's "no implicit UTC" rule.
//!
//! No `chrono::Utc::now()` is called — the function is pure. The
//! current build time, when needed for the future-date sanity bound,
//! is passed in as `build_time_ceiling` so callers can clock-skew test.

use serde::{Deserialize, Serialize};

/// Per-page date inputs the shell hands to this function. The shell
/// reads frontmatter / git metadata, performs no parsing, and passes
/// the strings as-is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageDates {
    /// `frontmatter.published`.
    pub frontmatter_published: Option<String>,
    /// `frontmatter.date`.
    pub frontmatter_date: Option<String>,
    /// `frontmatter.created`.
    pub frontmatter_created: Option<String>,
    /// First git-commit author-date. RFC 3339 format expected; the
    /// shell is responsible for asking git for the right format
    /// (`%aI`).
    pub git_first_commit: Option<String>,
    /// Last git-commit author-date.
    pub git_last_commit: Option<String>,
}

/// Outcome of [`resolve_date`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedDate {
    /// RFC 3339 string. Always carries an explicit timezone (`Z` or a
    /// numeric `+HH:MM` / `-HH:MM` offset).
    pub rfc3339: String,
    /// Which fallback rung produced the value, for observability.
    pub source: DateSource,
}

/// Provenance for [`ResolvedDate::rfc3339`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateSource {
    FrontmatterPublished,
    FrontmatterDate,
    FrontmatterCreated,
    GitFirstCommit,
    GitLastCommit,
}

/// Hard-error cases for [`resolve_date`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResolveDateError {
    /// Every source returned `None` — page has no resolvable date.
    /// REQ-3804 mandates a structured missing-date error rather than
    /// silently substituting the build time.
    #[error("no date resolvable for page (frontmatter + git both empty)")]
    Missing,
    /// A source returned a string that isn't RFC 3339 or that we
    /// reject (timezone-naive, future-dated, pre-1970).
    #[error("malformed date {value:?} from {from:?}: {reason}")]
    Malformed {
        from: DateSource,
        value: String,
        reason: String,
    },
}

/// Resolve a page's published date. See module docs for the fallback
/// chain. `build_time_ceiling` is the maximum date we accept (default:
/// build_time + 1 year); pass `None` to disable the future-date check.
pub fn resolve_date(
    dates: &PageDates,
    build_time_ceiling: Option<&str>,
) -> Result<ResolvedDate, ResolveDateError> {
    let chain = [
        (
            DateSource::FrontmatterPublished,
            dates.frontmatter_published.as_deref(),
        ),
        (
            DateSource::FrontmatterDate,
            dates.frontmatter_date.as_deref(),
        ),
        (
            DateSource::FrontmatterCreated,
            dates.frontmatter_created.as_deref(),
        ),
        (
            DateSource::GitFirstCommit,
            dates.git_first_commit.as_deref(),
        ),
        (DateSource::GitLastCommit, dates.git_last_commit.as_deref()),
    ];
    for (source, value) in chain {
        if let Some(raw) = value {
            let raw = raw.trim();
            if raw.is_empty() {
                continue;
            }
            // Parse + normalise. Bare YYYY-MM-DD is allowed and
            // interpreted as midnight UTC; everything else must carry
            // an explicit timezone marker.
            let canonical =
                canonicalise_rfc3339(raw).map_err(|reason| ResolveDateError::Malformed {
                    from: source,
                    value: raw.to_string(),
                    reason,
                })?;
            if let Some(ceiling) = build_time_ceiling {
                if canonical.as_str() > ceiling {
                    return Err(ResolveDateError::Malformed {
                        from: source,
                        value: raw.to_string(),
                        reason: format!("date is in the future (>{ceiling})"),
                    });
                }
            }
            if canonical.starts_with("19") && canonical.as_str() < "1970-01-01T00:00:00Z" {
                return Err(ResolveDateError::Malformed {
                    from: source,
                    value: raw.to_string(),
                    reason: "date pre-1970".to_string(),
                });
            }
            return Ok(ResolvedDate {
                rfc3339: canonical,
                source,
            });
        }
    }
    Err(ResolveDateError::Missing)
}

/// Validate + normalise a date string into RFC 3339 form. Accepts:
///
///   * `YYYY-MM-DD`                     -> normalised to `YYYY-MM-DDT00:00:00Z`
///   * `YYYY-MM-DDTHH:MM:SS<tz>`        with `tz` ∈ `Z`, `+HH:MM`, `-HH:MM`
///   * `YYYY-MM-DDTHH:MM:SS.fff<tz>`    fractional seconds passed through
///
/// Rejects everything else. Lexicographic comparison on the canonical
/// form matches chronological order which is what
/// `build_time_ceiling` checking depends on.
fn canonicalise_rfc3339(s: &str) -> Result<String, String> {
    let bytes = s.as_bytes();
    // Bare date.
    if bytes.len() == 10 {
        validate_ymd(s)?;
        return Ok(format!("{s}T00:00:00Z"));
    }
    if bytes.len() < 19 {
        return Err("too short for RFC 3339".to_string());
    }
    // `YYYY-MM-DD` prefix.
    validate_ymd(&s[..10])?;
    if !matches!(bytes[10], b'T' | b' ') {
        return Err("missing T separator".to_string());
    }
    // `HH:MM:SS` (treat space the same as T but normalise to T).
    if bytes[13] != b':' || bytes[16] != b':' {
        return Err("malformed HH:MM:SS".to_string());
    }
    for &b in &bytes[11..13] {
        if !b.is_ascii_digit() {
            return Err("non-digit in hour".to_string());
        }
    }
    for &b in &bytes[14..16] {
        if !b.is_ascii_digit() {
            return Err("non-digit in minute".to_string());
        }
    }
    for &b in &bytes[17..19] {
        if !b.is_ascii_digit() {
            return Err("non-digit in second".to_string());
        }
    }
    // Optional fractional seconds.
    let mut tail_start = 19;
    if tail_start < bytes.len() && bytes[tail_start] == b'.' {
        let mut p = tail_start + 1;
        while p < bytes.len() && bytes[p].is_ascii_digit() {
            p += 1;
        }
        tail_start = p;
    }
    let tz = &s[tail_start..];
    if tz.is_empty() {
        return Err("timezone-naive (no Z or offset)".to_string());
    }
    if tz == "Z" || tz == "z" {
        // ok
    } else if tz.len() == 6 && (bytes[tail_start] == b'+' || bytes[tail_start] == b'-') {
        if bytes[tail_start + 3] != b':' {
            return Err("malformed tz offset".to_string());
        }
    } else {
        return Err(format!("malformed timezone: {tz:?}"));
    }
    let mut out = String::with_capacity(s.len() + 1);
    out.push_str(&s[..10]);
    out.push('T');
    out.push_str(&s[11..tail_start]);
    if tz == "z" {
        out.push('Z');
    } else {
        out.push_str(tz);
    }
    Ok(out)
}

fn validate_ymd(s: &str) -> Result<(), String> {
    if s.len() != 10 {
        return Err("YYYY-MM-DD must be 10 chars".to_string());
    }
    let bytes = s.as_bytes();
    if bytes[4] != b'-' || bytes[7] != b'-' {
        return Err("missing - separators in date".to_string());
    }
    for &b in &bytes[..4] {
        if !b.is_ascii_digit() {
            return Err("non-digit in year".to_string());
        }
    }
    for &b in &bytes[5..7] {
        if !b.is_ascii_digit() {
            return Err("non-digit in month".to_string());
        }
    }
    for &b in &bytes[8..10] {
        if !b.is_ascii_digit() {
            return Err("non-digit in day".to_string());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_chain_picks_first_present() {
        let dates = PageDates {
            frontmatter_published: Some("2025-01-01T00:00:00Z".to_string()),
            frontmatter_date: Some("2024-01-01T00:00:00Z".to_string()),
            ..Default::default()
        };
        let r = resolve_date(&dates, None).unwrap();
        assert_eq!(r.rfc3339, "2025-01-01T00:00:00Z");
        assert_eq!(r.source, DateSource::FrontmatterPublished);
    }

    #[test]
    fn fallback_to_git_first_commit() {
        let dates = PageDates {
            git_first_commit: Some("2024-06-01T12:30:45+02:00".to_string()),
            ..Default::default()
        };
        let r = resolve_date(&dates, None).unwrap();
        assert_eq!(r.source, DateSource::GitFirstCommit);
    }

    #[test]
    fn missing_when_all_none() {
        let r = resolve_date(&PageDates::default(), None);
        assert_eq!(r, Err(ResolveDateError::Missing));
    }

    #[test]
    fn bare_date_normalised_to_midnight_utc() {
        let dates = PageDates {
            frontmatter_date: Some("2026-05-08".to_string()),
            ..Default::default()
        };
        let r = resolve_date(&dates, None).unwrap();
        assert_eq!(r.rfc3339, "2026-05-08T00:00:00Z");
    }

    #[test]
    fn timezone_naive_rejected() {
        let dates = PageDates {
            frontmatter_date: Some("2026-05-08T12:00:00".to_string()),
            ..Default::default()
        };
        let err = resolve_date(&dates, None).unwrap_err();
        match err {
            ResolveDateError::Malformed { reason, .. } => {
                assert!(reason.contains("timezone-naive"), "got {reason}")
            }
            _ => panic!("expected malformed"),
        }
    }

    #[test]
    fn future_date_rejected_against_ceiling() {
        let dates = PageDates {
            frontmatter_date: Some("2099-01-01T00:00:00Z".to_string()),
            ..Default::default()
        };
        let err = resolve_date(&dates, Some("2027-01-01T00:00:00Z")).unwrap_err();
        match err {
            ResolveDateError::Malformed { reason, .. } => {
                assert!(reason.contains("future"), "got {reason}")
            }
            _ => panic!("expected malformed"),
        }
    }

    #[test]
    fn empty_strings_skipped_in_chain() {
        let dates = PageDates {
            frontmatter_published: Some("   ".to_string()),
            frontmatter_date: Some("2026-01-01".to_string()),
            ..Default::default()
        };
        let r = resolve_date(&dates, None).unwrap();
        assert_eq!(r.source, DateSource::FrontmatterDate);
    }

    #[test]
    fn deterministic_for_identical_inputs() {
        let dates = PageDates {
            frontmatter_date: Some("2026-05-08T12:00:00Z".to_string()),
            ..Default::default()
        };
        let a = resolve_date(&dates, None).unwrap();
        let b = resolve_date(&dates, None).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn fractional_seconds_preserved() {
        let dates = PageDates {
            frontmatter_date: Some("2026-05-08T12:00:00.123Z".to_string()),
            ..Default::default()
        };
        let r = resolve_date(&dates, None).unwrap();
        assert_eq!(r.rfc3339, "2026-05-08T12:00:00.123Z");
    }

    #[test]
    fn space_separator_normalised_to_t() {
        let dates = PageDates {
            frontmatter_date: Some("2026-05-08 12:00:00Z".to_string()),
            ..Default::default()
        };
        let r = resolve_date(&dates, None).unwrap();
        assert_eq!(r.rfc3339, "2026-05-08T12:00:00Z");
    }

    #[test]
    fn lowercase_z_normalised_to_uppercase() {
        let dates = PageDates {
            frontmatter_date: Some("2026-05-08T12:00:00z".to_string()),
            ..Default::default()
        };
        let r = resolve_date(&dates, None).unwrap();
        assert_eq!(r.rfc3339, "2026-05-08T12:00:00Z");
    }

    #[test]
    fn malformed_garbage_rejected() {
        let dates = PageDates {
            frontmatter_date: Some("yesterday".to_string()),
            ..Default::default()
        };
        assert!(resolve_date(&dates, None).is_err());
    }
}
