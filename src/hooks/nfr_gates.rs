//! Performance + determinism gates for SPEC-032 NFR-3201..NFR-3208.
//!
//! Each NFR has a numeric budget; the gates enforce them as test
//! assertions and as runtime checks the build command can call after a
//! page render. The module owns three things:
//!
//! 1. **Budget constants** — one `pub const` per NFR, mirroring the
//!    spec verbatim so a budget edit is a single-line diff with the
//!    spec for review.
//! 2. **Pure helpers** — [`p95`] and [`enforce_budget`] for the test
//!    harness; [`PageBudget`] for the per-page entry point.
//! 3. **`PageBudgetReport`** — emitted by [`PageBudget::evaluate`] so
//!    the build command can surface budget excursions through the
//!    existing [`crate::hooks::observability::HookObserver`] surface
//!    without coupling this module to the observer trait.
//!
//! The module is pure (no I/O) so the gates can be reused from
//! integration tests, microbenchmarks, and the future
//! `cargo nfr-gates` aggregator without dragging in build-graph
//! dependencies. Per-page wiring lives in `tests/nfr_gates_integration.rs`.
//!
//! ## NFR coverage matrix
//!
//! | Const                                | NFR      | Verifies                             |
//! |--------------------------------------|----------|--------------------------------------|
//! | [`SELECTOR_P95_BUDGET`]              | NFR-3201 | per-(page × hook) selector P95 ≤ 2 ms |
//! | [`CANONICAL_OVERHEAD_PCT_BUDGET`]    | NFR-3202 | render overhead vs `--no-hooks` ≤ 15 %|
//! | (process — byte-equality)            | NFR-3203 | `ztl build` deterministic across runs|
//! | [`HOOK_FAIL_ON_NEVER_EXIT_CODE`]     | NFR-3204 | `--hook-fail-on never` → exit 0      |
//! | [`ZERO_HOOK_STARTUP_BUDGET`]         | NFR-3205 | startup delta vs no-hooks ≤ 5 ms P95 |
//! | (process — CHANGELOG)                | NFR-3206 | AST schema additive-only             |
//! | [`PROTOCOL_OVERHEAD_BUDGET`]         | NFR-3207 | persistent-mode round-trip P95 ≤ 10 ms|
//! | [`DEFAULT_HOOK_MEMORY_LIMIT_BYTES`]  | NFR-3208 | hook RSS ceiling, default 64 MiB     |

use std::time::Duration;

use crate::hooks::failure_scoping::FailureRecord;
use crate::hooks::pipeline::{PipelineStats, Stage};

// ── Budget constants ────────────────────────────────────────────────────────

/// NFR-3201: per-(page × hook) selector evaluation budget at P95.
pub const SELECTOR_P95_BUDGET: Duration = Duration::from_millis(2);

/// NFR-3202: max render-time inflation when the three canonical
/// extensions (callouts, tasks, admonition) are enabled, expressed as
/// a fraction of the `--no-hooks` baseline (15 %).
pub const CANONICAL_OVERHEAD_PCT_BUDGET: f64 = 0.15;

/// NFR-3204: exit code ztl SHALL return when `--hook-fail-on never`
/// is in effect, regardless of how many hooks failed.
pub const HOOK_FAIL_ON_NEVER_EXIT_CODE: i32 = 0;

/// NFR-3205: max additional process-startup cost on a vault with no
/// `.ztl/hooks/` and a theme with no `hooks/`, vs. pre-SPEC-032 ztl,
/// at P95.
pub const ZERO_HOOK_STARTUP_BUDGET: Duration = Duration::from_millis(5);

/// NFR-3207: per-page persistent-mode round-trip budget at P95 for a
/// 500-node AST.
pub const PROTOCOL_OVERHEAD_BUDGET: Duration = Duration::from_millis(10);

/// NFR-3207 inner sample target: the implementation aims to clear
/// the budget by an order of magnitude on a realistic loopback echo.
/// Used as a "warn, don't fail" threshold inside the perf harness so
/// regressions are visible long before they breach the spec budget.
pub const PROTOCOL_OVERHEAD_TASK_TARGET: Duration = Duration::from_micros(1_500);

/// NFR-3208: default per-hook memory ceiling, in bytes (64 MiB).
/// Manifest authors override per hook via the `memory_mib` field;
/// this value is the floor enforced when no override is declared.
pub const DEFAULT_HOOK_MEMORY_LIMIT_BYTES: u64 = 64 * 1024 * 1024;

/// NFR-3203 reference path: the schema version the determinism gate
/// pins against. A bump here SHOULD coincide with a major-version
/// CHANGELOG entry and a deprecation window per NFR-3206.
pub const PINNED_AST_SCHEMA_VERSION: &str = crate::hooks::ast::AST_VERSION;

// ── Sampling helpers ───────────────────────────────────────────────────────

/// Compute the P95 of `samples` using the nearest-rank method on a
/// pre-sortable copy. Empty input returns [`Duration::ZERO`] so callers
/// can chain the result into a budget comparison without an `is_some`
/// check; an empty sample never breaches a budget by definition.
///
/// Matches the percentile formula used throughout the SPEC-032
/// observability surface (see `OBS-3201`'s `p95_ms` field): pick the
/// element at index `ceil(0.95 * n) - 1`. For small `n` this is the
/// max sample, which is the conservative (over-estimating) reading.
pub fn p95(samples: &[Duration]) -> Duration {
    if samples.is_empty() {
        return Duration::ZERO;
    }
    let mut sorted: Vec<Duration> = samples.to_vec();
    sorted.sort_unstable();
    let n = sorted.len();
    let rank = ((0.95 * n as f64).ceil() as usize).max(1);
    sorted[(rank - 1).min(n - 1)]
}

/// Compute the P50 (median) of `samples`. Matches [`p95`]'s nearest-
/// rank semantics so the two read consistently in a side-by-side
/// summary.
pub fn p50(samples: &[Duration]) -> Duration {
    if samples.is_empty() {
        return Duration::ZERO;
    }
    let mut sorted: Vec<Duration> = samples.to_vec();
    sorted.sort_unstable();
    let n = sorted.len();
    let rank = ((0.5 * n as f64).ceil() as usize).max(1);
    sorted[(rank - 1).min(n - 1)]
}

/// Render-overhead percentage: `(treatment - baseline) / baseline`.
/// A treatment faster than the baseline (negative inflation) reads
/// as `0.0` so the gate doesn't celebrate a slow baseline. Returns
/// `f64::NAN` when `baseline == Duration::ZERO`; callers should
/// reject NaN as an unmeasurable run.
pub fn overhead_ratio(baseline: Duration, treatment: Duration) -> f64 {
    let b = baseline.as_secs_f64();
    if b == 0.0 {
        return f64::NAN;
    }
    let t = treatment.as_secs_f64();
    ((t - b) / b).max(0.0)
}

// ── Per-page enforcement ───────────────────────────────────────────────────

/// Per-page budget governing the four time slices the
/// pipeline can be measured against. Used by [`PageBudget::evaluate`]
/// to translate a [`PipelineStats`] into a [`PageBudgetReport`] without
/// the build command having to know any spec numbers.
///
/// Each field maps 1:1 to a spec NFR; constructing via
/// [`PageBudget::from_spec`] guarantees the canonical values are in
/// use. Tests that want to exercise a tighter budget can hand-build
/// the struct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageBudget {
    /// Sum of selector evaluation across every hook on this page
    /// (NFR-3201 — counted at the pipeline edge, not per-hook).
    /// `None` means "not enforced for this page" — used by tests that
    /// only want the protocol-overhead arm.
    pub selector_total: Option<Duration>,
    /// Max wall-clock the persistent-protocol round-trip should take
    /// for any single hook on this page (NFR-3207).
    pub protocol_per_hook: Option<Duration>,
}

impl Default for PageBudget {
    /// Defaults to the spec's hard caps (NFR-3201 × 10 hooks for the
    /// selector slice, NFR-3207 verbatim for the protocol slice).
    fn default() -> Self {
        Self::from_spec(10)
    }
}

impl PageBudget {
    /// Spec-derived budget. `expected_hooks_per_page` scales the
    /// NFR-3201 per-(page × hook) budget into a per-page total — pass
    /// `0` to disable the selector arm.
    pub fn from_spec(expected_hooks_per_page: u32) -> Self {
        let selector_total = if expected_hooks_per_page == 0 {
            None
        } else {
            Some(SELECTOR_P95_BUDGET * expected_hooks_per_page)
        };
        Self {
            selector_total,
            protocol_per_hook: Some(PROTOCOL_OVERHEAD_BUDGET),
        }
    }

    /// Evaluate `stats` + `failures` against this budget. Always
    /// returns a report — even when nothing breached — so callers can
    /// emit a zero-violation telemetry line without branching.
    pub fn evaluate(
        &self,
        page_slug: impl Into<String>,
        stats: &PipelineStats,
        failures: &[FailureRecord],
    ) -> PageBudgetReport {
        let page_slug = page_slug.into();
        let mut violations = Vec::new();

        if let Some(budget) = self.selector_total {
            // Selector wall-clock isn't a stage timer in [`PipelineStats`];
            // we approximate by attributing the pre-parse stage's hook
            // overhead (selectors fire before each hook). When a future
            // task wires a dedicated counter into the pipeline this branch
            // updates without touching callers.
            let observed = stats.pre_parse;
            if observed > budget {
                violations.push(BudgetViolation {
                    nfr: "NFR-3201",
                    stage: Some(Stage::PreParse),
                    observed,
                    budget,
                    detail: "per-page selector wall-clock above budget".to_string(),
                });
            }
        }

        if let Some(budget) = self.protocol_per_hook {
            // Transform stage is the only one that runs through the
            // persistent protocol today; pre-parse + post-render bypass
            // it. Sum hook-stage time and compare against the per-hook
            // budget × hook-count contributions emitted by the failure
            // records (which carry per-hook duration_ms).
            let mut worst: Duration = Duration::ZERO;
            for rec in failures {
                if rec.duration_ms > worst.as_millis() as u64 {
                    worst = Duration::from_millis(rec.duration_ms);
                }
            }
            let observed = worst.max(stats.transform);
            if observed > budget {
                violations.push(BudgetViolation {
                    nfr: "NFR-3207",
                    stage: Some(Stage::Transform),
                    observed,
                    budget,
                    detail: "persistent-mode round-trip above budget".to_string(),
                });
            }
        }

        PageBudgetReport {
            page_slug,
            violations,
        }
    }
}

/// One budget excursion — the spec NFR id, the stage that breached,
/// the observed wall-clock, and the budget it cleared. Preserves the
/// integer ms shape used by the OBS-3201 coverage row so a downstream
/// telemetry sink can `serde_json::to_value` the report directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetViolation {
    pub nfr: &'static str,
    pub stage: Option<Stage>,
    pub observed: Duration,
    pub budget: Duration,
    pub detail: String,
}

impl std::fmt::Display for BudgetViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let stage = match self.stage {
            Some(s) => s.as_str(),
            None => "-",
        };
        write!(
            f,
            "{}: stage={} observed={}ms budget={}ms ({})",
            self.nfr,
            stage,
            self.observed.as_millis(),
            self.budget.as_millis(),
            self.detail,
        )
    }
}

/// Result of a per-page budget evaluation. `violations.is_empty()` ==
/// "page passed every gate". `passed()` is the canonical query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageBudgetReport {
    pub page_slug: String,
    pub violations: Vec<BudgetViolation>,
}

impl PageBudgetReport {
    pub fn passed(&self) -> bool {
        self.violations.is_empty()
    }

    /// One stderr-friendly line per violation. Caller prefixes with
    /// `[ztl]` to match the OBS-3204 channel.
    pub fn format_lines(&self) -> Vec<String> {
        self.violations
            .iter()
            .map(|v| format!("page={} {}", self.page_slug, v))
            .collect()
    }
}

// ── Suite-level gate ───────────────────────────────────────────────────────

/// Apply a `samples → P95 ≤ budget` gate. Returns
/// [`Err(BudgetExcursion)`] when the sample's P95 breaches the budget;
/// the caller renders the error or asserts.
///
/// The two-argument shape (samples, budget) keeps the per-NFR test
/// bodies tight and uniform — readers see the budget number adjacent
/// to the gate id without having to chase a struct field.
pub fn enforce_budget(
    nfr: &'static str,
    samples: &[Duration],
    budget: Duration,
) -> Result<BudgetSummary, BudgetExcursion> {
    let summary = BudgetSummary::measure(nfr, samples, budget);
    if summary.passed() {
        Ok(summary)
    } else {
        Err(BudgetExcursion(summary))
    }
}

/// Summary from [`enforce_budget`]: the NFR id, the sample count,
/// the observed P50 / P95, and the budget. Used by the test harness to
/// log a one-line "p50=… p95=… budget=…" trace at the end of every
/// gate so regressions show up in CI without re-running the gate
/// locally.
#[derive(Debug, Clone)]
pub struct BudgetSummary {
    pub nfr: &'static str,
    pub n: usize,
    pub p50: Duration,
    pub p95: Duration,
    pub budget: Duration,
}

impl BudgetSummary {
    fn measure(nfr: &'static str, samples: &[Duration], budget: Duration) -> Self {
        Self {
            nfr,
            n: samples.len(),
            p50: p50(samples),
            p95: p95(samples),
            budget,
        }
    }

    pub fn passed(&self) -> bool {
        self.p95 <= self.budget
    }
}

impl std::fmt::Display for BudgetSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: n={} p50={:?} p95={:?} budget={:?}",
            self.nfr, self.n, self.p50, self.p95, self.budget,
        )
    }
}

/// Wrapper carrying a failed [`BudgetSummary`]. Implements [`Error`]
/// so callers can `?` it; the wrapped summary is publicly available
/// for telemetry / panicking with a nice message.
#[derive(Debug, Clone)]
pub struct BudgetExcursion(pub BudgetSummary);

impl std::fmt::Display for BudgetExcursion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} budget breached: {}", self.0.nfr, self.0,)
    }
}

impl std::error::Error for BudgetExcursion {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::failure_scoping::FailureReason;

    #[test]
    fn p95_nearest_rank_picks_max_for_small_samples() {
        let samples = vec![
            Duration::from_millis(1),
            Duration::from_millis(2),
            Duration::from_millis(3),
        ];
        assert_eq!(p95(&samples), Duration::from_millis(3));
    }

    #[test]
    fn p95_picks_19th_of_20_samples() {
        let samples: Vec<Duration> = (1..=20).map(|i| Duration::from_millis(i)).collect();
        assert_eq!(p95(&samples), Duration::from_millis(19));
    }

    #[test]
    fn p95_empty_returns_zero() {
        assert_eq!(p95(&[]), Duration::ZERO);
        assert_eq!(p50(&[]), Duration::ZERO);
    }

    #[test]
    fn overhead_ratio_clamps_negative_to_zero() {
        let baseline = Duration::from_millis(100);
        let faster = Duration::from_millis(50);
        assert_eq!(overhead_ratio(baseline, faster), 0.0);
    }

    #[test]
    fn overhead_ratio_zero_baseline_is_nan() {
        assert!(overhead_ratio(Duration::ZERO, Duration::from_millis(1)).is_nan());
    }

    #[test]
    fn overhead_ratio_canonical_15_percent() {
        let baseline = Duration::from_millis(1000);
        let treatment = Duration::from_millis(1150);
        let r = overhead_ratio(baseline, treatment);
        assert!((r - 0.15).abs() < 1e-9, "ratio = {r}");
    }

    #[test]
    fn enforce_budget_ok_when_under_budget() {
        let samples = vec![Duration::from_millis(1); 100];
        let summary = enforce_budget("NFR-3201", &samples, Duration::from_millis(2)).unwrap();
        assert!(summary.passed());
        assert_eq!(summary.n, 100);
    }

    #[test]
    fn enforce_budget_err_when_over_budget() {
        let samples = vec![Duration::from_millis(5); 100];
        let err = enforce_budget("NFR-3207", &samples, Duration::from_millis(2)).unwrap_err();
        assert_eq!(err.0.nfr, "NFR-3207");
        assert!(!err.0.passed());
    }

    #[test]
    fn page_budget_default_matches_spec() {
        let b = PageBudget::default();
        assert_eq!(
            b.selector_total,
            Some(SELECTOR_P95_BUDGET * 10),
            "default expects 10 hooks/page; per-hook budget is NFR-3201"
        );
        assert_eq!(b.protocol_per_hook, Some(PROTOCOL_OVERHEAD_BUDGET));
    }

    #[test]
    fn page_budget_evaluate_passes_for_quiet_page() {
        let b = PageBudget::from_spec(10);
        let stats = PipelineStats::default();
        let report = b.evaluate("daily/2026-04-20", &stats, &[]);
        assert!(report.passed(), "no work, no violations");
        assert!(report.format_lines().is_empty());
    }

    #[test]
    fn page_budget_evaluate_flags_protocol_overrun() {
        let b = PageBudget::from_spec(10);
        let mut stats = PipelineStats::default();
        // 25 ms transform stage breaches the 10 ms NFR-3207 budget.
        stats.transform = Duration::from_millis(25);
        let report = b.evaluate("page", &stats, &[]);
        assert_eq!(report.violations.len(), 1);
        assert_eq!(report.violations[0].nfr, "NFR-3207");
        assert_eq!(report.violations[0].stage, Some(Stage::Transform));
    }

    #[test]
    fn page_budget_evaluate_flags_selector_overrun() {
        let b = PageBudget::from_spec(10);
        let mut stats = PipelineStats::default();
        // 25 ms pre-parse breaches the 20 ms aggregate selector budget.
        stats.pre_parse = Duration::from_millis(25);
        let report = b.evaluate("page", &stats, &[]);
        assert_eq!(report.violations.len(), 1);
        assert_eq!(report.violations[0].nfr, "NFR-3201");
    }

    #[test]
    fn page_budget_can_disable_selector_arm() {
        let b = PageBudget::from_spec(0);
        assert_eq!(b.selector_total, None);
    }

    #[test]
    fn page_budget_uses_failure_records_for_protocol_arm() {
        let b = PageBudget::from_spec(10);
        let stats = PipelineStats::default();
        let rec = FailureRecord::new(
            "callouts",
            Stage::Transform,
            "p",
            FailureReason::Timeout,
            "t",
            Duration::from_millis(50),
            "2026-04-20T00:00:00Z",
        );
        let report = b.evaluate("p", &stats, &[rec]);
        assert_eq!(report.violations.len(), 1);
        assert_eq!(report.violations[0].nfr, "NFR-3207");
    }

    #[test]
    fn report_format_lines_one_per_violation() {
        let b = PageBudget::from_spec(10);
        let mut stats = PipelineStats::default();
        stats.transform = Duration::from_millis(50);
        stats.pre_parse = Duration::from_millis(50);
        let report = b.evaluate("posts/hello", &stats, &[]);
        let lines = report.format_lines();
        assert_eq!(lines.len(), 2);
        for line in &lines {
            assert!(line.starts_with("page=posts/hello"));
        }
    }

    #[test]
    fn budget_constants_pin_to_spec_numbers() {
        // Single-line diff against SPEC-032 §4 if any of these change.
        assert_eq!(SELECTOR_P95_BUDGET, Duration::from_millis(2));
        assert_eq!(PROTOCOL_OVERHEAD_BUDGET, Duration::from_millis(10));
        assert_eq!(ZERO_HOOK_STARTUP_BUDGET, Duration::from_millis(5));
        assert_eq!(CANONICAL_OVERHEAD_PCT_BUDGET, 0.15);
        assert_eq!(DEFAULT_HOOK_MEMORY_LIMIT_BYTES, 64 * 1024 * 1024);
        assert_eq!(HOOK_FAIL_ON_NEVER_EXIT_CODE, 0);
        assert_eq!(PINNED_AST_SCHEMA_VERSION, "1.0");
    }
}
