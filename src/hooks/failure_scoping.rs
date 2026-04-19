//! Failure scoping and pipeline continuation (SPEC-032 REQ-3207 / CON-3207).
//!
//! When a hook `H_k` in the ordered stage pipeline fails — non-zero exit,
//! timeout, memory overrun, malformed output — zetl:
//!
//! - Discards `H_k`'s output.
//! - Records a [`FailureRecord`] with `plugin_id`, `stage`, `page_slug`,
//!   `reason`, `detail`, `duration_ms`, `at`.
//! - Passes `H_k`'s **input** (i.e. the output of `H_{k-1}`, or the stage
//!   input when `k == 0`) to `H_{k+1}`.
//! - Continues the pipeline. One hook's failure never cascades.
//!
//! The pipeline driver in [`crate::hooks::pipeline::run_page`] implements
//! this contract. Per-build collection, the on-disk
//! `hook-diagnostics.json` file, and the `--hook-fail-on` exit policy are
//! implemented here.
//!
//! The five-part `HookDiagnostic` rendering surface (CON-3225) lives in
//! [`crate::hooks::diagnostic`]; this module owns the structured record
//! schema (CON-3207) and the policy knob.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::hooks::pipeline::{HookError, Stage};

/// `zetl build --hook-fail-on` policy knob (REQ-3207).
///
/// Default is [`HookFailOn::Never`] — hook failures are recorded to
/// `hook-diagnostics.json` but the build exits zero. [`HookFailOn::Error`]
/// makes any recorded failure fail the build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HookFailOn {
    #[default]
    Never,
    Error,
}

impl HookFailOn {
    pub fn as_str(self) -> &'static str {
        match self {
            HookFailOn::Never => "never",
            HookFailOn::Error => "error",
        }
    }

    /// Parse the CLI flag value. Returns `None` for unrecognised inputs so
    /// the caller can surface a typed error with the allowed values.
    pub fn from_flag(value: &str) -> Option<Self> {
        match value {
            "never" => Some(HookFailOn::Never),
            "error" => Some(HookFailOn::Error),
            _ => None,
        }
    }
}

impl std::fmt::Display for HookFailOn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Failure category recorded in [`FailureRecord::reason`] (CON-3207 /
/// OBS-3205).
///
/// Matches the `reason` dimension of OBS-3205's
/// `zetl_hook_failure_total{reason, ...}` counter — machine-filterable in
/// logs without pulling the free-form `detail` string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureReason {
    NonZeroExit,
    Timeout,
    MemoryExceeded,
    MalformedOutput,
    Crash,
    ContractViolation,
    /// Fallback when the underlying error reason doesn't map to any of the
    /// named categories. Specific call sites (protocol, contract enforcement)
    /// should set the category explicitly via [`FailureRecord::with_reason`].
    Other,
}

impl FailureReason {
    pub fn as_str(self) -> &'static str {
        match self {
            FailureReason::NonZeroExit => "non_zero_exit",
            FailureReason::Timeout => "timeout",
            FailureReason::MemoryExceeded => "memory_exceeded",
            FailureReason::MalformedOutput => "malformed_output",
            FailureReason::Crash => "crash",
            FailureReason::ContractViolation => "contract_violation",
            FailureReason::Other => "other",
        }
    }

    /// Best-effort classification of a free-form hook error message into a
    /// REQ-3207 category. Used when a `HookError` arrives without an
    /// explicit category hint — e.g. from a trait impl that only knows
    /// the hook's message. Specific call sites (protocol framing,
    /// contract enforcement) override with [`FailureRecord::with_reason`].
    pub fn classify(reason: &str) -> FailureReason {
        let lower = reason.to_ascii_lowercase();
        if lower.contains("timeout") || lower.contains("timed out") || lower.contains("deadline") {
            FailureReason::Timeout
        } else if lower.contains("memory") || lower.contains("oom") {
            FailureReason::MemoryExceeded
        } else if lower.contains("contract") {
            FailureReason::ContractViolation
        } else if lower.contains("malformed")
            || lower.contains("invalid json")
            || lower.contains("schema")
            || lower.contains("parse error")
        {
            FailureReason::MalformedOutput
        } else if lower.contains("crash") || lower.contains("signal") || lower.contains("sigkill")
        {
            FailureReason::Crash
        } else if lower.contains("non-zero") || lower.contains("exit code") {
            FailureReason::NonZeroExit
        } else {
            FailureReason::Other
        }
    }
}

/// CON-3207 failure diagnostic record — the on-disk schema of
/// `hook-diagnostics.json` and the in-memory entries aggregated across
/// a build.
///
/// Field order is stable — downstream tooling (coverage report, CI
/// linters) consumes this as JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureRecord {
    /// Plugin identifier (manifest filename without `\d+-` prefix and
    /// extension — see `composition::default_extension_id`).
    pub hook: String,
    /// Stage name: `"pre-parse" | "transform" | "post-render"`.
    pub stage: String,
    /// Page slug — the page whose render was in progress when the hook
    /// failed.
    pub page_slug: String,
    /// Failure category — see [`FailureReason::as_str`].
    pub reason: String,
    /// Free-form detail — typically the hook's own error message. Kept
    /// alongside the categorised `reason` so humans get the story,
    /// tooling gets the filter dimension.
    pub detail: String,
    /// Wall-clock time spent in the failing hook invocation, in
    /// milliseconds. Zero when the failure was detected before the hook
    /// was dispatched (e.g. manifest parse, schema mismatch).
    pub duration_ms: u64,
    /// ISO 8601 UTC timestamp of when the failure was recorded.
    pub at: String,
}

impl FailureRecord {
    pub fn new(
        hook: impl Into<String>,
        stage: Stage,
        page_slug: impl Into<String>,
        reason: FailureReason,
        detail: impl Into<String>,
        duration: Duration,
        at: impl Into<String>,
    ) -> Self {
        Self {
            hook: hook.into(),
            stage: stage.as_str().to_string(),
            page_slug: page_slug.into(),
            reason: reason.as_str().to_string(),
            detail: detail.into(),
            duration_ms: duration.as_millis() as u64,
            at: at.into(),
        }
    }

    /// Construct a record from a pipeline [`HookError`]. The reason is
    /// classified from the error message; call [`Self::with_reason`] to
    /// override when a more specific category is known.
    pub fn from_hook_error(
        err: &HookError,
        page_slug: impl Into<String>,
        duration: Duration,
    ) -> Self {
        let reason = FailureReason::classify(&err.reason);
        Self::new(
            err.hook_id.clone(),
            err.stage,
            page_slug,
            reason,
            err.reason.clone(),
            duration,
            now_iso8601(),
        )
    }

    /// Override the categorised `reason` — used by call sites that have
    /// stronger classification information than the message string
    /// heuristic.
    pub fn with_reason(mut self, reason: FailureReason) -> Self {
        self.reason = reason.as_str().to_string();
        self
    }
}

/// Compute the process exit code from a collected failure buffer and the
/// `--hook-fail-on` policy.
///
/// Returns `0` when there are no failures OR the policy is
/// [`HookFailOn::Never`]; `1` when the policy is [`HookFailOn::Error`] and
/// at least one failure was recorded.
pub fn exit_code_for(records: &[FailureRecord], policy: HookFailOn) -> i32 {
    if matches!(policy, HookFailOn::Error) && !records.is_empty() {
        1
    } else {
        0
    }
}

/// Write `hook-diagnostics.json` to `<out_dir>/hook-diagnostics.json`.
///
/// The file is replaced, not merged (REQ-3208 persistence semantics):
/// each `zetl build` records only its own pass. Empty input still writes
/// `[]` so CI can check the presence of the artefact.
pub fn write_diagnostics_json(
    out_dir: &Path,
    records: &[FailureRecord],
) -> std::io::Result<PathBuf> {
    let path = out_dir.join("hook-diagnostics.json");
    let body = serde_json::to_string_pretty(records)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(&path, body)?;
    Ok(path)
}

/// Human-readable summary emitted to stderr by `zetl build` when any hook
/// failed. Lines are grouped by `(stage, hook)` and ordered by stage ID.
///
/// Under `--hook-fail-on error` this summary is followed by exit(1); under
/// `--hook-fail-on never` it's an informational suffix to the build log.
pub fn format_summary(records: &[FailureRecord]) -> String {
    if records.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    out.push_str(&format!(
        "[zetl] hook failures: {} record{} across {} page{}\n",
        records.len(),
        if records.len() == 1 { "" } else { "s" },
        unique_pages(records),
        if unique_pages(records) == 1 { "" } else { "s" },
    ));
    for r in records {
        out.push_str(&format!(
            "  - {} ({}) on {}: {} — {}\n",
            r.hook, r.stage, r.page_slug, r.reason, r.detail,
        ));
    }
    out
}

fn unique_pages(records: &[FailureRecord]) -> usize {
    let mut seen: Vec<&str> = Vec::with_capacity(records.len());
    for r in records {
        if !seen.contains(&r.page_slug.as_str()) {
            seen.push(&r.page_slug);
        }
    }
    seen.len()
}

/// Current UTC wall-clock time as an ISO 8601 string
/// (`"2026-04-20T10:32:17Z"`). Pure-std; follows the same Hinnant-derived
/// algorithm as the rest of the codebase.
pub fn now_iso8601() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    unix_secs_to_iso8601(secs)
}

fn unix_secs_to_iso8601(secs: u64) -> String {
    let tod = secs % 86400;
    let (h, m, s) = (tod / 3600, (tod % 3600) / 60, tod % 60);
    let (year, month, day) = days_to_ymd((secs / 86400) as i64);
    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}Z")
}

fn days_to_ymd(mut days: i64) -> (i32, u32, u32) {
    let mut year = 1970i32;
    loop {
        let dy = if is_leap_year(year) { 366i64 } else { 365i64 };
        if days < dy {
            break;
        }
        days -= dy;
        year += 1;
    }
    let month_days: [u32; 12] = [
        31,
        if is_leap_year(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1u32;
    for &md in &month_days {
        if days < md as i64 {
            break;
        }
        days -= md as i64;
        month += 1;
    }
    (year, month, days as u32 + 1)
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn default_policy_is_never() {
        assert_eq!(HookFailOn::default(), HookFailOn::Never);
    }

    #[test]
    fn fail_on_flag_round_trips() {
        assert_eq!(HookFailOn::from_flag("never"), Some(HookFailOn::Never));
        assert_eq!(HookFailOn::from_flag("error"), Some(HookFailOn::Error));
        assert_eq!(HookFailOn::from_flag("bogus"), None);
        assert_eq!(HookFailOn::Never.as_str(), "never");
        assert_eq!(HookFailOn::Error.as_str(), "error");
    }

    #[test]
    fn reason_classify_hits_named_categories() {
        assert_eq!(FailureReason::classify("timeout exceeded"), FailureReason::Timeout);
        assert_eq!(FailureReason::classify("hit deadline_ms"), FailureReason::Timeout);
        assert_eq!(FailureReason::classify("memory exceeded"), FailureReason::MemoryExceeded);
        assert_eq!(FailureReason::classify("OOM kill"), FailureReason::MemoryExceeded);
        assert_eq!(
            FailureReason::classify("contract violation: preserves"),
            FailureReason::ContractViolation
        );
        assert_eq!(FailureReason::classify("malformed output"), FailureReason::MalformedOutput);
        assert_eq!(FailureReason::classify("invalid json"), FailureReason::MalformedOutput);
        assert_eq!(FailureReason::classify("schema mismatch"), FailureReason::MalformedOutput);
        assert_eq!(FailureReason::classify("crashed in SIGKILL"), FailureReason::Crash);
        assert_eq!(FailureReason::classify("non-zero exit"), FailureReason::NonZeroExit);
        assert_eq!(FailureReason::classify("exit code 127"), FailureReason::NonZeroExit);
        assert_eq!(FailureReason::classify("who knows"), FailureReason::Other);
    }

    #[test]
    fn from_hook_error_populates_all_fields() {
        let err = HookError::new(Stage::Transform, "callouts", "timeout exceeded deadline_ms=100");
        let rec = FailureRecord::from_hook_error(&err, "projects/q2", Duration::from_millis(180));
        assert_eq!(rec.hook, "callouts");
        assert_eq!(rec.stage, "transform");
        assert_eq!(rec.page_slug, "projects/q2");
        assert_eq!(rec.reason, "timeout");
        assert_eq!(rec.detail, "timeout exceeded deadline_ms=100");
        assert_eq!(rec.duration_ms, 180);
        assert!(rec.at.ends_with('Z') && rec.at.len() == 20);
    }

    #[test]
    fn with_reason_overrides_classification() {
        let err = HookError::new(Stage::PostRender, "h", "some opaque message");
        let rec = FailureRecord::from_hook_error(&err, "p", Duration::ZERO)
            .with_reason(FailureReason::ContractViolation);
        assert_eq!(rec.reason, "contract_violation");
    }

    #[test]
    fn exit_code_never_is_zero_even_with_failures() {
        let recs = vec![FailureRecord::new(
            "h",
            Stage::Transform,
            "p",
            FailureReason::NonZeroExit,
            "boom",
            Duration::from_millis(1),
            "2026-04-20T00:00:00Z",
        )];
        assert_eq!(exit_code_for(&recs, HookFailOn::Never), 0);
        assert_eq!(exit_code_for(&recs, HookFailOn::Error), 1);
        assert_eq!(exit_code_for(&[], HookFailOn::Error), 0);
    }

    #[test]
    fn write_diagnostics_json_round_trips() {
        let tmp = TempDir::new().unwrap();
        let recs = vec![
            FailureRecord::new(
                "callouts",
                Stage::Transform,
                "projects/q2",
                FailureReason::Timeout,
                "Exceeded deadline_ms=100, killed at 180ms",
                Duration::from_millis(180),
                "2026-04-19T10:32:17Z",
            ),
            FailureRecord::new(
                "tasks",
                Stage::PostRender,
                "daily/today",
                FailureReason::NonZeroExit,
                "exit 2",
                Duration::from_millis(12),
                "2026-04-19T10:32:18Z",
            ),
        ];

        let path = write_diagnostics_json(tmp.path(), &recs).unwrap();
        assert_eq!(path, tmp.path().join("hook-diagnostics.json"));

        let body = std::fs::read_to_string(&path).unwrap();
        let roundtrip: Vec<FailureRecord> = serde_json::from_str(&body).unwrap();
        assert_eq!(roundtrip, recs);
    }

    #[test]
    fn write_diagnostics_json_empty_writes_empty_array() {
        let tmp = TempDir::new().unwrap();
        let path = write_diagnostics_json(tmp.path(), &[]).unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        assert_eq!(body, "[]");
    }

    #[test]
    fn summary_groups_stage_and_page_counts() {
        let recs = vec![
            FailureRecord::new(
                "h1",
                Stage::Transform,
                "p1",
                FailureReason::NonZeroExit,
                "boom",
                Duration::from_millis(1),
                "t",
            ),
            FailureRecord::new(
                "h2",
                Stage::PostRender,
                "p1",
                FailureReason::Timeout,
                "slow",
                Duration::from_millis(2),
                "t",
            ),
        ];
        let out = format_summary(&recs);
        assert!(out.contains("2 records across 1 page"));
        assert!(out.contains("h1 (transform) on p1: non_zero_exit — boom"));
        assert!(out.contains("h2 (post-render) on p1: timeout — slow"));

        assert!(format_summary(&[]).is_empty());
    }

    #[test]
    fn now_iso8601_format_shape() {
        let now = now_iso8601();
        // "YYYY-MM-DDTHH:MM:SSZ" = 20 chars
        assert_eq!(now.len(), 20, "got: {now}");
        assert!(now.ends_with('Z'));
        // Year digits are ASCII numerals.
        assert!(now.chars().take(4).all(|c| c.is_ascii_digit()));
    }
}
