//! Chrono-free epoch / RFC 3339 helpers shared across the feed
//! module. Hoisted from `feed::retention` and `web::build` where
//! they were duplicated. Kept chrono-free so the leaf modules
//! remain feature-flag-independent.

/// Convert a `SystemTime` to an RFC 3339 UTC string. Returns
/// `None` for times before the Unix epoch (only happens on systems
/// with skewed clocks; we'd rather fail closed than emit a future
/// date).
pub fn system_time_to_rfc3339(t: std::time::SystemTime) -> Option<String> {
    let dur = t.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(epoch_secs_to_rfc3339(dur.as_secs() as i64))
}

/// Render an integer second offset from the Unix epoch as
/// `YYYY-MM-DDTHH:MM:SSZ`. Negative inputs clamp to the epoch.
pub fn epoch_secs_to_rfc3339(total: i64) -> String {
    if total < 0 {
        return "1970-01-01T00:00:00Z".to_string();
    }
    let days = total / 86_400;
    let rem = total - days * 86_400;
    let (h, m, s) = (
        (rem / 3600) as u32,
        ((rem % 3600) / 60) as u32,
        (rem % 60) as u32,
    );
    let (y, mo, d) = epoch_days_to_ymd(days);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

/// Subtract a positive number of seconds from an RFC 3339 string.
/// Used by retention pruning to compute the cutoff date for a
/// `Duration` policy. Input must already be in canonical
/// `YYYY-MM-DDTHH:MM:SSZ` form (the form `feed::resolve_date`
/// produces); other shapes pass through unchanged.
pub fn subtract_seconds_from_rfc3339(rfc3339: &str, seconds: i64) -> String {
    if let Some((y, m, d, h, mi, se)) = parse_components(rfc3339) {
        let total = days_since_epoch(y, m, d) * 86_400
            + (h as i64) * 3600
            + (mi as i64) * 60
            + (se as i64);
        return epoch_secs_to_rfc3339(total - seconds);
    }
    rfc3339.to_string()
}

fn parse_components(s: &str) -> Option<(i32, u32, u32, u32, u32, u32)> {
    if s.len() < 19 {
        return None;
    }
    Some((
        s[0..4].parse().ok()?,
        s[5..7].parse().ok()?,
        s[8..10].parse().ok()?,
        s[11..13].parse().ok()?,
        s[14..16].parse().ok()?,
        s[17..19].parse().ok()?,
    ))
}

fn days_since_epoch(year: i32, month: u32, day: u32) -> i64 {
    let mut days: i64 = 0;
    for y in 1970..year {
        days += if is_leap(y) { 366 } else { 365 };
    }
    let dim = days_in_months(year);
    for m in 1..month as usize {
        days += dim[m - 1] as i64;
    }
    days + (day as i64) - 1
}

/// Convert epoch-day-offset to (year, month, day). Public for
/// retention's date arithmetic + the build-wire's mtime fallback.
pub fn epoch_days_to_ymd(mut days: i64) -> (i32, u32, u32) {
    let mut year: i32 = 1970;
    loop {
        let dy = if is_leap(year) { 366 } else { 365 };
        if days < dy {
            break;
        }
        days -= dy;
        year += 1;
    }
    let dim = days_in_months(year);
    let mut month: u32 = 1;
    for &d in &dim {
        if days < d as i64 {
            break;
        }
        days -= d as i64;
        month += 1;
    }
    (year, month, (days + 1) as u32)
}

/// Proleptic Gregorian leap-year rule.
pub fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_months(y: i32) -> [u8; 12] {
    let feb = if is_leap(y) { 29 } else { 28 };
    [31, feb, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_zero_is_unix_epoch() {
        assert_eq!(epoch_secs_to_rfc3339(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn negative_clamps() {
        assert_eq!(epoch_secs_to_rfc3339(-1), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn round_trip_year_2026() {
        // 2026-05-08T11:25:24Z = epoch 1_778_239_524
        let s = epoch_secs_to_rfc3339(1_778_239_524);
        assert_eq!(s, "2026-05-08T11:25:24Z");
    }

    #[test]
    fn leap_year_rule() {
        assert!(is_leap(2024));
        assert!(!is_leap(2023));
        assert!(!is_leap(2100));
        assert!(is_leap(2000));
    }

    #[test]
    fn subtract_handles_leap() {
        // 2024-03-01 minus 1 day = 2024-02-29 (leap year).
        assert_eq!(
            subtract_seconds_from_rfc3339("2024-03-01T00:00:00Z", 86_400),
            "2024-02-29T00:00:00Z",
        );
    }
}
