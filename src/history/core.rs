//! Pure temporal functions for SPEC-017.
//!
//! This module provides [`parse_time_expr`] and [`resolve_snapshot`]:
//!
//! - **[`parse_time_expr`]** — pure parser; no I/O, no VCS calls.
//!   Recognises ISO 8601 dates/datetimes, relative natural-language
//!   expressions, and git-style refs (`HEAD~N`, change-ID prefixes).
//!
//! - **[`resolve_snapshot`]** — walk a snapshot list (newest-first) and
//!   return the most recent entry at or before the resolved time.

use anyhow::{anyhow, bail};
use chrono::{DateTime, Datelike as _, Duration, FixedOffset, NaiveDate, TimeZone as _, Weekday};

use crate::history::jj_backend::ChangeInfo;

// ─── Public types ─────────────────────────────────────────────────────────────

/// The result of parsing a time expression.
///
/// Variants are evaluated against a snapshot list by [`resolve_snapshot`].
#[derive(Debug, Clone, PartialEq)]
pub enum TimeExpr {
    /// An absolute point in time derived from ISO 8601 or relative NL.
    Absolute(DateTime<FixedOffset>),
    /// The Nth ancestor of the most recent snapshot (`HEAD~N`; `HEAD` = `HEAD~0`).
    HeadOffset(usize),
    /// A jj change-ID prefix or bookmark/branch name.
    Ref(String),
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Parse a time expression string into a [`TimeExpr`].
///
/// `now` is used as the reference point for relative expressions.
///
/// # Supported forms
///
/// | Form | Example |
/// |---|---|
/// | ISO 8601 date | `2024-01-15` |
/// | ISO 8601 datetime | `2024-01-15T14:30:00Z` |
/// | ISO 8601 datetime with offset | `2024-01-15T14:30:00+05:30` |
/// | Today | `today` |
/// | Yesterday | `yesterday` |
/// | N units ago | `3 days ago`, `2 weeks ago`, `1 hour ago` |
/// | Last weekday | `last monday`, `last friday` |
/// | Last week | `last week` |
/// | HEAD | `HEAD`, `HEAD~0` |
/// | HEAD ancestor | `HEAD~1`, `HEAD~7` |
/// | VCS ref | `zkpqwxlmnop` (change-ID prefix), `main`, `dev` |
pub fn parse_time_expr(expr: &str, now: DateTime<FixedOffset>) -> anyhow::Result<TimeExpr> {
    let s = expr.trim();

    if s.is_empty() {
        bail!("empty time expression");
    }

    // HEAD or HEAD~N
    if let Some(offset) = parse_head_offset(s) {
        return Ok(TimeExpr::HeadOffset(offset));
    }

    // ISO 8601 datetime with timezone
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(TimeExpr::Absolute(dt));
    }

    // ISO 8601 date only (YYYY-MM-DD) → end of that day (23:59:59) so that
    // all snapshots taken during the day are included in a ≤ comparison.
    if let Ok(nd) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let dt = nd
            .and_hms_opt(23, 59, 59)
            .and_then(|ndt| now.timezone().from_local_datetime(&ndt).single())
            .ok_or_else(|| anyhow!("could not convert date {s} to datetime"))?;
        return Ok(TimeExpr::Absolute(dt));
    }

    // Relative natural-language
    if let Some(dt) = parse_relative(s, now) {
        return Ok(TimeExpr::Absolute(dt));
    }

    // Fall through: treat as a VCS ref (change-ID prefix or bookmark name)
    Ok(TimeExpr::Ref(s.to_owned()))
}

/// Return the most recent [`ChangeInfo`] at or before the time described by
/// `expr`, or an error if no matching snapshot exists.
///
/// `snapshots` must be ordered **newest-first** (as returned by
/// [`VcsBackend::list_changes`]).
///
/// # Errors
///
/// Returns `SNAPSHOT_NOT_FOUND` when:
/// - `snapshots` is empty, or
/// - no snapshot timestamp is ≤ the resolved absolute time, or
/// - a `HEAD~N` offset exceeds the number of available snapshots, or
/// - no snapshot matches the supplied ref string.
pub fn resolve_snapshot<'a>(
    expr: &str,
    now: DateTime<FixedOffset>,
    snapshots: &'a [ChangeInfo],
) -> anyhow::Result<&'a ChangeInfo> {
    if snapshots.is_empty() {
        bail!("SNAPSHOT_NOT_FOUND: no snapshots available");
    }

    let time_expr = parse_time_expr(expr, now)?;

    match time_expr {
        TimeExpr::Absolute(target) => snapshots
            .iter()
            .find(|c| c.timestamp <= target)
            .ok_or_else(|| {
                anyhow!("SNAPSHOT_NOT_FOUND: no snapshot at or before {target}")
            }),

        TimeExpr::HeadOffset(n) => snapshots.get(n).ok_or_else(|| {
            anyhow!(
                "SNAPSHOT_NOT_FOUND: HEAD~{n} is out of range (only {} snapshot(s))",
                snapshots.len()
            )
        }),

        TimeExpr::Ref(ref ref_str) => snapshots
            .iter()
            .find(|c| c.change_id.starts_with(ref_str.as_str()) || c.description == *ref_str)
            .ok_or_else(|| {
                anyhow!("SNAPSHOT_NOT_FOUND: no snapshot matching ref {ref_str:?}")
            }),
    }
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

/// Parse `HEAD` → `Some(0)` and `HEAD~N` → `Some(N)`.
fn parse_head_offset(s: &str) -> Option<usize> {
    if s.eq_ignore_ascii_case("head") {
        return Some(0);
    }
    let rest = s.strip_prefix("HEAD~").or_else(|| s.strip_prefix("head~"))?;
    rest.parse::<usize>().ok()
}

/// Parse relative natural-language time expressions.
///
/// Returns `None` when `s` is not a recognised relative form.
fn parse_relative(s: &str, now: DateTime<FixedOffset>) -> Option<DateTime<FixedOffset>> {
    let lower = s.to_lowercase();

    if lower == "now" {
        return Some(now);
    }

    if lower == "today" {
        return Some(start_of_day(now));
    }

    if lower == "yesterday" {
        return Some(start_of_day(now) - Duration::days(1));
    }

    // "last week" → start of Monday of the previous calendar week
    if lower == "last week" {
        let today = now.date_naive();
        let days_since_monday = today.weekday().num_days_from_monday() as i64;
        let last_monday = today - Duration::days(days_since_monday + 7);
        return last_monday
            .and_hms_opt(0, 0, 0)
            .and_then(|ndt| now.timezone().from_local_datetime(&ndt).single());
    }

    // "last <weekday>"
    if let Some(rest) = lower.strip_prefix("last ") {
        if let Some(wd) = parse_weekday(rest) {
            return Some(last_weekday(now, wd));
        }
    }

    // "N unit(s) ago"  — must have at least 3 whitespace-separated tokens
    let words: Vec<&str> = lower.split_whitespace().collect();
    if words.len() >= 3 && words[words.len() - 1] == "ago" {
        if let Ok(n) = words[0].parse::<i64>() {
            let unit = words[1].trim_end_matches('s');
            let delta = match unit {
                "second" => Duration::seconds(n),
                "minute" => Duration::minutes(n),
                "hour" => Duration::hours(n),
                "day" => Duration::days(n),
                "week" => Duration::weeks(n),
                "month" => Duration::days(n * 30),
                "year" => Duration::days(n * 365),
                _ => return None,
            };
            return Some(now - delta);
        }
    }

    None
}

/// Return midnight of the day containing `dt`, preserving the fixed offset.
fn start_of_day(dt: DateTime<FixedOffset>) -> DateTime<FixedOffset> {
    dt.date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|ndt| dt.timezone().from_local_datetime(&ndt).single())
        .unwrap_or(dt)
}

/// Return the most recent past occurrence of `weekday` before `now`
/// (never today itself: at least 1 day back, at most 7 days back).
fn last_weekday(now: DateTime<FixedOffset>, weekday: Weekday) -> DateTime<FixedOffset> {
    let today = now.date_naive();
    let target_num = weekday.num_days_from_monday() as i64;
    let today_num = today.weekday().num_days_from_monday() as i64;
    let mut days_back = today_num - target_num;
    if days_back <= 0 {
        days_back += 7;
    }
    let past = today - Duration::days(days_back);
    past.and_hms_opt(0, 0, 0)
        .and_then(|ndt| now.timezone().from_local_datetime(&ndt).single())
        .unwrap_or_else(|| now - Duration::days(days_back))
}

/// Parse a weekday name (long or three-letter abbreviation).
fn parse_weekday(s: &str) -> Option<Weekday> {
    match s {
        "monday" | "mon" => Some(Weekday::Mon),
        "tuesday" | "tue" => Some(Weekday::Tue),
        "wednesday" | "wed" => Some(Weekday::Wed),
        "thursday" | "thu" => Some(Weekday::Thu),
        "friday" | "fri" => Some(Weekday::Fri),
        "saturday" | "sat" => Some(Weekday::Sat),
        "sunday" | "sun" => Some(Weekday::Sun),
        _ => None,
    }
}

// ─── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod time_expression_parser {
    use chrono::{FixedOffset, TimeZone as _, Timelike as _};

    use super::*;

    /// A fixed "now" for deterministic tests: 2026-03-04 15:30:00 UTC.
    fn now_utc() -> DateTime<FixedOffset> {
        FixedOffset::east_opt(0)
            .unwrap()
            .with_ymd_and_hms(2026, 3, 4, 15, 30, 0)
            .unwrap()
    }

    fn make_snapshot(change_id: &str, description: &str, ts: DateTime<FixedOffset>) -> ChangeInfo {
        ChangeInfo {
            change_id: change_id.to_owned(),
            commit_id: "deadbeef0000".to_owned(),
            timestamp: ts,
            description: description.to_owned(),
        }
    }

    fn ts(y: i32, m: u32, d: u32, h: u32, min: u32, s: u32) -> DateTime<FixedOffset> {
        FixedOffset::east_opt(0)
            .unwrap()
            .with_ymd_and_hms(y, m, d, h, min, s)
            .unwrap()
    }

    // ── ISO 8601 ────────────────────────────────────────────────────────────

    #[test]
    fn iso_date_only() {
        // Date-only expressions resolve to end-of-day so all snapshots on
        // that day are included in a ≤ comparison.
        let expr = parse_time_expr("2024-01-15", now_utc()).unwrap();
        let TimeExpr::Absolute(dt) = expr else {
            panic!("expected Absolute, got {expr:?}");
        };
        assert_eq!(dt.year(), 2024);
        assert_eq!(dt.month(), 1);
        assert_eq!(dt.day(), 15);
        assert_eq!(dt.hour(), 23);
        assert_eq!(dt.minute(), 59);
    }

    #[test]
    fn iso_datetime_utc() {
        let expr = parse_time_expr("2024-06-01T12:00:00Z", now_utc()).unwrap();
        let TimeExpr::Absolute(dt) = expr else {
            panic!("expected Absolute, got {expr:?}");
        };
        assert_eq!(dt.year(), 2024);
        assert_eq!(dt.month(), 6);
        assert_eq!(dt.day(), 1);
        assert_eq!(dt.hour(), 12);
    }

    #[test]
    fn iso_datetime_with_offset() {
        let expr = parse_time_expr("2024-06-01T14:30:00+05:30", now_utc()).unwrap();
        let TimeExpr::Absolute(dt) = expr else {
            panic!("expected Absolute, got {expr:?}");
        };
        // Stored with the +05:30 offset, hour = 14.
        assert_eq!(dt.hour(), 14);
    }

    // ── Relative natural language ────────────────────────────────────────────

    #[test]
    fn relative_today() {
        let now = now_utc(); // 2026-03-04 15:30:00
        let expr = parse_time_expr("today", now).unwrap();
        let TimeExpr::Absolute(dt) = expr else {
            panic!("expected Absolute");
        };
        assert_eq!((dt.year(), dt.month(), dt.day()), (2026, 3, 4));
        assert_eq!(dt.hour(), 0);
    }

    #[test]
    fn relative_yesterday() {
        let now = now_utc(); // 2026-03-04
        let expr = parse_time_expr("yesterday", now).unwrap();
        let TimeExpr::Absolute(dt) = expr else {
            panic!("expected Absolute");
        };
        assert_eq!((dt.year(), dt.month(), dt.day()), (2026, 3, 3));
    }

    #[test]
    fn relative_n_days_ago() {
        let now = now_utc(); // 2026-03-04 15:30:00
        let expr = parse_time_expr("3 days ago", now).unwrap();
        let TimeExpr::Absolute(dt) = expr else {
            panic!("expected Absolute");
        };
        // 3 days before 2026-03-04 15:30:00 = 2026-03-01 15:30:00
        assert_eq!((dt.year(), dt.month(), dt.day()), (2026, 3, 1));
    }

    #[test]
    fn relative_2_weeks_ago() {
        let now = now_utc(); // 2026-03-04
        let expr = parse_time_expr("2 weeks ago", now).unwrap();
        let TimeExpr::Absolute(dt) = expr else {
            panic!("expected Absolute");
        };
        // 14 days before 2026-03-04 = 2026-02-18
        assert_eq!((dt.year(), dt.month(), dt.day()), (2026, 2, 18));
    }

    #[test]
    fn relative_1_hour_ago() {
        let now = now_utc(); // 15:30:00
        let expr = parse_time_expr("1 hour ago", now).unwrap();
        let TimeExpr::Absolute(dt) = expr else {
            panic!("expected Absolute");
        };
        assert_eq!(dt.hour(), 14);
        assert_eq!(dt.minute(), 30);
    }

    #[test]
    fn relative_last_monday() {
        // 2026-03-04 is a Wednesday.
        let now = now_utc();
        let expr = parse_time_expr("last monday", now).unwrap();
        let TimeExpr::Absolute(dt) = expr else {
            panic!("expected Absolute");
        };
        // Most recent Monday before Wednesday 2026-03-04 = 2026-03-02.
        assert_eq!((dt.year(), dt.month(), dt.day()), (2026, 3, 2));
    }

    #[test]
    fn relative_last_friday() {
        // 2026-03-04 is a Wednesday; last Friday = 2026-02-27.
        let now = now_utc();
        let expr = parse_time_expr("last friday", now).unwrap();
        let TimeExpr::Absolute(dt) = expr else {
            panic!("expected Absolute");
        };
        assert_eq!((dt.year(), dt.month(), dt.day()), (2026, 2, 27));
    }

    #[test]
    fn relative_last_week() {
        // 2026-03-04 (Wednesday): current week starts 2026-03-02 (Mon),
        // so last week starts 2026-02-23 (Mon).
        let now = now_utc();
        let expr = parse_time_expr("last week", now).unwrap();
        let TimeExpr::Absolute(dt) = expr else {
            panic!("expected Absolute");
        };
        assert_eq!((dt.year(), dt.month(), dt.day()), (2026, 2, 23));
    }

    // ── HEAD refs ────────────────────────────────────────────────────────────

    #[test]
    fn head_ref() {
        let expr = parse_time_expr("HEAD", now_utc()).unwrap();
        assert_eq!(expr, TimeExpr::HeadOffset(0));
    }

    #[test]
    fn head_tilde_zero() {
        let expr = parse_time_expr("HEAD~0", now_utc()).unwrap();
        assert_eq!(expr, TimeExpr::HeadOffset(0));
    }

    #[test]
    fn head_tilde_n() {
        let expr = parse_time_expr("HEAD~3", now_utc()).unwrap();
        assert_eq!(expr, TimeExpr::HeadOffset(3));
    }

    #[test]
    fn head_lowercase() {
        let expr = parse_time_expr("head~5", now_utc()).unwrap();
        assert_eq!(expr, TimeExpr::HeadOffset(5));
    }

    // ── VCS ref passthrough ──────────────────────────────────────────────────

    #[test]
    fn vcs_ref_change_id_prefix() {
        let expr = parse_time_expr("zkpqwxlmnopq", now_utc()).unwrap();
        assert_eq!(expr, TimeExpr::Ref("zkpqwxlmnopq".to_owned()));
    }

    #[test]
    fn vcs_ref_branch_name() {
        let expr = parse_time_expr("main", now_utc()).unwrap();
        assert_eq!(expr, TimeExpr::Ref("main".to_owned()));
    }

    // ── resolve_snapshot ────────────────────────────────────────────────────

    /// Build a small snapshot list (newest-first) for resolver tests.
    fn sample_snapshots() -> Vec<ChangeInfo> {
        vec![
            make_snapshot("aaaaaaaaaaaa", "snap-c", ts(2026, 3, 4, 12, 0, 0)),
            make_snapshot("bbbbbbbbbbbb", "snap-b", ts(2026, 3, 2, 8, 0, 0)),
            make_snapshot("cccccccccccc", "snap-a", ts(2026, 3, 1, 0, 0, 0)),
        ]
    }

    #[test]
    fn resolve_iso_date_exact_match() {
        let snaps = sample_snapshots();
        let result = resolve_snapshot("2026-03-04", now_utc(), &snaps).unwrap();
        assert_eq!(result.description, "snap-c");
    }

    #[test]
    fn resolve_iso_date_before_newest() {
        let snaps = sample_snapshots();
        // 2026-03-03 is after snap-b (03-02) but before snap-c (03-04 12:00).
        let result = resolve_snapshot("2026-03-03", now_utc(), &snaps).unwrap();
        assert_eq!(result.description, "snap-b");
    }

    #[test]
    fn resolve_iso_before_all_returns_error() {
        let snaps = sample_snapshots();
        let err = resolve_snapshot("2025-01-01", now_utc(), &snaps).unwrap_err();
        assert!(
            err.to_string().contains("SNAPSHOT_NOT_FOUND"),
            "expected SNAPSHOT_NOT_FOUND, got {err}"
        );
    }

    #[test]
    fn resolve_head_zero_is_newest() {
        let snaps = sample_snapshots();
        let result = resolve_snapshot("HEAD", now_utc(), &snaps).unwrap();
        assert_eq!(result.description, "snap-c");
    }

    #[test]
    fn resolve_head_tilde_1() {
        let snaps = sample_snapshots();
        let result = resolve_snapshot("HEAD~1", now_utc(), &snaps).unwrap();
        assert_eq!(result.description, "snap-b");
    }

    #[test]
    fn resolve_head_out_of_range() {
        let snaps = sample_snapshots();
        let err = resolve_snapshot("HEAD~10", now_utc(), &snaps).unwrap_err();
        assert!(
            err.to_string().contains("SNAPSHOT_NOT_FOUND"),
            "expected SNAPSHOT_NOT_FOUND, got {err}"
        );
    }

    #[test]
    fn resolve_empty_snapshots_errors() {
        let err = resolve_snapshot("yesterday", now_utc(), &[]).unwrap_err();
        assert!(
            err.to_string().contains("SNAPSHOT_NOT_FOUND"),
            "expected SNAPSHOT_NOT_FOUND, got {err}"
        );
    }

    #[test]
    fn resolve_ref_by_change_id_prefix() {
        let snaps = sample_snapshots();
        let result = resolve_snapshot("bbbb", now_utc(), &snaps).unwrap();
        assert_eq!(result.description, "snap-b");
    }

    #[test]
    fn resolve_ref_by_description() {
        let snaps = sample_snapshots();
        let result = resolve_snapshot("snap-a", now_utc(), &snaps).unwrap();
        assert_eq!(result.description, "snap-a");
    }

    #[test]
    fn resolve_ref_no_match_errors() {
        let snaps = sample_snapshots();
        let err = resolve_snapshot("unknown-ref", now_utc(), &snaps).unwrap_err();
        assert!(
            err.to_string().contains("SNAPSHOT_NOT_FOUND"),
            "expected SNAPSHOT_NOT_FOUND, got {err}"
        );
    }

    #[test]
    fn error_on_empty_expr() {
        let err = parse_time_expr("", now_utc()).unwrap_err();
        assert!(err.to_string().contains("empty time expression"));
    }

    #[test]
    fn whitespace_trimmed() {
        let expr = parse_time_expr("  HEAD~2  ", now_utc()).unwrap();
        assert_eq!(expr, TimeExpr::HeadOffset(2));
    }
}
