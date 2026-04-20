//! Pipeline hook observability (SPEC-032 §9 — OBS-3204 / OBS-3206).
//!
//! Every hook invocation driven by [`crate::hooks::pipeline::run_page`]
//! produces a [`HookInvocationEvent`]. Callers register a [`HookObserver`]
//! to consume those events — under `--verbose`, [`StderrObserver`] emits
//! the canonical log line
//!
//! ```text
//!   [zetl] hook: stage=<stage> id=<hook-id> page=<page-slug> duration_ms=<N>
//! ```
//!
//! and failures are tagged with `status=failed reason=<reason>` so a single
//! grep surfaces both success and failure cases from a build log.
//!
//! A [`HookStats`] aggregator rolls events up across every page in a build:
//!
//! - **per-stage wall time** — time spent running hooks at each of the three
//!   stages (matches [`crate::hooks::pipeline::PipelineStats::stage`]).
//! - **per-hook wall time** — summed duration of each unique `(stage, id)`
//!   pair; the "is this hook earning its place?" signal from §9.
//! - **pages matched** — how many unique page slugs each hook ran against,
//!   which feeds the coverage report (REQ-3208).
//! - **diagnostics emitted** — count of [`FailureRecord`] entries produced
//!   (matches `failures=<K>` in the OBS-3206 build-summary line).
//!
//! The module does not own the render pipeline itself; it is a passive
//! surface that the pipeline writes to. Keeping the aggregator separate from
//! [`PipelineStats`] lets the pipeline stay pure (`Copy` per-page stats)
//! while this module absorbs the unbounded-size book-keeping.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::time::Duration;

use crate::hooks::failure_scoping::FailureRecord;
use crate::hooks::pipeline::Stage;

/// A single hook invocation observed by the pipeline.
///
/// Emitted once per hook per page, regardless of success — callers inspect
/// [`Self::status`] to branch on failure. The event carries enough detail
/// to render the OBS-3204 log line without a back-reference into the
/// pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookInvocationEvent {
    pub stage: Stage,
    pub hook_id: String,
    pub page_slug: String,
    pub duration: Duration,
    pub status: HookInvocationStatus,
}

/// Success / failure classification for a [`HookInvocationEvent`].
///
/// On failure the raw [`crate::hooks::failure_scoping::FailureReason`] tag
/// is copied into `reason` as a string so the observer can render it
/// without pulling the enum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookInvocationStatus {
    Ok,
    Failed { reason: String },
}

impl HookInvocationStatus {
    pub fn is_ok(&self) -> bool {
        matches!(self, HookInvocationStatus::Ok)
    }
}

/// Receiver for pipeline hook-invocation events.
///
/// Implementations must be `Send + Sync` so pages may be rendered in
/// parallel; all event handlers see every event regardless of the
/// rendering thread. The pipeline calls `on_hook_invocation` exactly
/// once per hook per page — after failure scoping has decided whether to
/// revert the stage output.
pub trait HookObserver: Send + Sync {
    fn on_hook_invocation(&self, event: &HookInvocationEvent);
}

/// Observer that discards every event. Default for build paths that do
/// not enable `--verbose` and do not want to populate a [`HookStats`].
pub struct NoopObserver;

impl HookObserver for NoopObserver {
    fn on_hook_invocation(&self, _: &HookInvocationEvent) {}
}

/// Observer that writes OBS-3204 log lines to stderr. Set `verbose=false`
/// to suppress successful invocations while still surfacing failures —
/// matching the spec's "log line per failure" surface from OBS-3205.
pub struct StderrObserver {
    pub verbose: bool,
}

impl StderrObserver {
    pub fn new(verbose: bool) -> Self {
        Self { verbose }
    }
}

impl HookObserver for StderrObserver {
    fn on_hook_invocation(&self, event: &HookInvocationEvent) {
        match &event.status {
            HookInvocationStatus::Ok if self.verbose => {
                eprintln!(
                    "[zetl] hook: stage={} id={} page={} duration_ms={}",
                    event.stage,
                    event.hook_id,
                    event.page_slug,
                    event.duration.as_millis()
                );
            }
            HookInvocationStatus::Failed { reason } => {
                // Failures emit at any verbosity — OBS-3205's "log line per
                // failure" contract is independent of --verbose.
                eprintln!(
                    "[zetl] hook: stage={} id={} page={} duration_ms={} status=failed reason={}",
                    event.stage,
                    event.hook_id,
                    event.page_slug,
                    event.duration.as_millis(),
                    reason
                );
            }
            HookInvocationStatus::Ok => {}
        }
    }
}

/// Observer that captures every event into a shared buffer. Useful for
/// tests and for downstream aggregation paths that want a raw event
/// log alongside the rolled-up [`HookStats`].
#[derive(Default)]
pub struct CapturingObserver {
    events: Mutex<Vec<HookInvocationEvent>>,
}

impl CapturingObserver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn events(&self) -> Vec<HookInvocationEvent> {
        self.events.lock().unwrap().clone()
    }

    pub fn len(&self) -> usize {
        self.events.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.lock().unwrap().is_empty()
    }
}

impl HookObserver for CapturingObserver {
    fn on_hook_invocation(&self, event: &HookInvocationEvent) {
        self.events.lock().unwrap().push(event.clone());
    }
}

/// Aggregated per-hook invocation accounting.
///
/// One entry per unique `(stage, hook_id)` pair in a [`HookStats`]
/// rollup. `pages` uses a [`BTreeSet`] so the count reflects unique
/// slugs (the §9 "pages matched" signal), not raw invocations.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HookAccum {
    pub duration: Duration,
    pub invocations: usize,
    pub failures: usize,
    pub pages: BTreeSet<String>,
}

impl HookAccum {
    /// Number of unique pages this hook ran against.
    pub fn pages_matched(&self) -> usize {
        self.pages.len()
    }
}

/// Key for the [`HookStats::per_hook`] map: stage + hook id.
///
/// Ordered by `(stage, hook_id)` so iteration gives a deterministic,
/// pipeline-order listing for the stats block formatter.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct HookKey {
    pub stage_index: u8,
    pub stage: String,
    pub hook_id: String,
}

impl HookKey {
    pub fn new(stage: Stage, hook_id: impl Into<String>) -> Self {
        Self {
            stage_index: stage_to_index(stage),
            stage: stage.as_str().to_string(),
            hook_id: hook_id.into(),
        }
    }
}

/// Build-wide observability aggregator.
///
/// Roll up invocations from every page render into a single summary
/// struct. Use [`HookStats::record`] to feed individual events (as from
/// an observer) and [`HookStats::record_failures`] after each page to
/// bump the `diagnostics_emitted` counter in lockstep with the failure
/// buffer populated by [`crate::hooks::pipeline::run_page`].
///
/// Implements [`HookObserver`] behind a mutex so it can be registered
/// directly with the pipeline; alternatively drive it from the caller
/// side if the caller wants to avoid the mutex.
#[derive(Debug, Default)]
pub struct HookStats {
    per_stage: [Duration; 3],
    per_hook: BTreeMap<HookKey, HookAccum>,
    diagnostics_emitted: usize,
    total_invocations: usize,
}

impl HookStats {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a single invocation. The `failures` counter on the matching
    /// [`HookAccum`] is incremented only when `event.status` is `Failed` —
    /// diagnostic records produced by the failure-scoping layer are counted
    /// separately via [`Self::record_failures`] so this method stays
    /// decoupled from the on-disk schema.
    pub fn record(&mut self, event: &HookInvocationEvent) {
        let stage = stage_from_str(event.stage.as_str())
            .expect("Stage::as_str round-trips to a valid Stage");
        self.per_stage[stage_to_index(stage) as usize] += event.duration;
        self.total_invocations += 1;

        let key = HookKey::new(stage, &event.hook_id);
        let accum = self.per_hook.entry(key).or_default();
        accum.duration += event.duration;
        accum.invocations += 1;
        accum.pages.insert(event.page_slug.clone());
        if !event.status.is_ok() {
            accum.failures += 1;
        }
    }

    /// Merge the `Vec<FailureRecord>` produced by a single
    /// `run_page` call into this aggregator. Only `diagnostics_emitted` is
    /// affected — per-hook failure counts are tracked via observer events,
    /// which land even when the hook aborts before producing a record.
    pub fn record_failures(&mut self, records: &[FailureRecord]) {
        self.diagnostics_emitted += records.len();
    }

    pub fn per_stage(&self, stage: Stage) -> Duration {
        self.per_stage[stage_to_index(stage) as usize]
    }

    pub fn per_hook(&self) -> &BTreeMap<HookKey, HookAccum> {
        &self.per_hook
    }

    pub fn total_invocations(&self) -> usize {
        self.total_invocations
    }

    pub fn diagnostics_emitted(&self) -> usize {
        self.diagnostics_emitted
    }

    /// Total wall-clock time spent in hook code across every stage. This
    /// is the `total_duration_ms` value in the OBS-3206 build-summary
    /// line.
    pub fn total_hook_duration(&self) -> Duration {
        self.per_stage.iter().copied().sum()
    }

    /// One-line OBS-3206 build-summary:
    ///
    /// ```text
    ///   [zetl] hooks: total_invocations=<N> total_duration_ms=<M> failures=<K>
    /// ```
    pub fn format_summary_line(&self) -> String {
        format!(
            "[zetl] hooks: total_invocations={} total_duration_ms={} failures={}",
            self.total_invocations,
            self.total_hook_duration().as_millis(),
            self.diagnostics_emitted,
        )
    }

    /// Multi-line human-readable stats block (§9 "per-stage time, per-hook
    /// time, pages matched, diagnostics emitted"). Every body line is
    /// indented under `[zetl] hooks:` so the block stays visually scoped.
    ///
    /// Hook rows are sorted by `(stage, hook_id)` for determinism.
    pub fn format_block(&self) -> String {
        let mut out = String::new();
        out.push_str("[zetl] hooks:\n");

        out.push_str("  stages:\n");
        for stage in Stage::all() {
            out.push_str(&format!(
                "    {:<11} duration_ms={}\n",
                stage.as_str(),
                self.per_stage(stage).as_millis(),
            ));
        }

        if self.per_hook.is_empty() {
            out.push_str("  hooks: (none)\n");
        } else {
            out.push_str("  hooks:\n");
            for (key, accum) in &self.per_hook {
                out.push_str(&format!(
                    "    stage={} id={} duration_ms={} pages_matched={} invocations={} failures={}\n",
                    key.stage,
                    key.hook_id,
                    accum.duration.as_millis(),
                    accum.pages_matched(),
                    accum.invocations,
                    accum.failures,
                ));
            }
        }

        out.push_str(&format!(
            "  totals: invocations={} duration_ms={} diagnostics_emitted={}\n",
            self.total_invocations,
            self.total_hook_duration().as_millis(),
            self.diagnostics_emitted,
        ));

        out
    }
}

impl HookObserver for Mutex<HookStats> {
    fn on_hook_invocation(&self, event: &HookInvocationEvent) {
        self.lock().unwrap().record(event);
    }
}

fn stage_to_index(stage: Stage) -> u8 {
    match stage {
        Stage::PreParse => 0,
        Stage::Transform => 1,
        Stage::PostRender => 2,
    }
}

fn stage_from_str(s: &str) -> Option<Stage> {
    match s {
        "pre-parse" => Some(Stage::PreParse),
        "transform" => Some(Stage::Transform),
        "post-render" => Some(Stage::PostRender),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::failure_scoping::{FailureReason, FailureRecord};

    fn ok_event(stage: Stage, id: &str, page: &str, ms: u64) -> HookInvocationEvent {
        HookInvocationEvent {
            stage,
            hook_id: id.into(),
            page_slug: page.into(),
            duration: Duration::from_millis(ms),
            status: HookInvocationStatus::Ok,
        }
    }

    fn fail_event(
        stage: Stage,
        id: &str,
        page: &str,
        ms: u64,
        reason: &str,
    ) -> HookInvocationEvent {
        HookInvocationEvent {
            stage,
            hook_id: id.into(),
            page_slug: page.into(),
            duration: Duration::from_millis(ms),
            status: HookInvocationStatus::Failed {
                reason: reason.into(),
            },
        }
    }

    #[test]
    fn capturing_observer_records_every_event() {
        let obs = CapturingObserver::new();
        obs.on_hook_invocation(&ok_event(Stage::Transform, "callouts", "daily", 3));
        obs.on_hook_invocation(&ok_event(Stage::Transform, "tasks", "daily", 5));
        assert_eq!(obs.len(), 2);
        let evs = obs.events();
        assert_eq!(evs[0].hook_id, "callouts");
        assert_eq!(evs[1].hook_id, "tasks");
    }

    #[test]
    fn hook_stats_aggregates_per_stage_and_per_hook() {
        let mut s = HookStats::new();
        s.record(&ok_event(Stage::PreParse, "a", "page-1", 2));
        s.record(&ok_event(Stage::PreParse, "a", "page-2", 3));
        s.record(&ok_event(Stage::Transform, "callouts", "page-1", 10));
        s.record(&ok_event(Stage::Transform, "callouts", "page-2", 15));
        s.record(&ok_event(Stage::Transform, "tasks", "page-1", 7));
        s.record(&fail_event(
            Stage::PostRender,
            "admonition",
            "page-1",
            4,
            "timeout",
        ));

        assert_eq!(s.per_stage(Stage::PreParse), Duration::from_millis(5));
        assert_eq!(s.per_stage(Stage::Transform), Duration::from_millis(32));
        assert_eq!(s.per_stage(Stage::PostRender), Duration::from_millis(4));

        let a = s
            .per_hook()
            .get(&HookKey::new(Stage::PreParse, "a"))
            .unwrap();
        assert_eq!(a.duration, Duration::from_millis(5));
        assert_eq!(a.pages_matched(), 2);
        assert_eq!(a.invocations, 2);
        assert_eq!(a.failures, 0);

        let callouts = s
            .per_hook()
            .get(&HookKey::new(Stage::Transform, "callouts"))
            .unwrap();
        assert_eq!(callouts.duration, Duration::from_millis(25));
        assert_eq!(callouts.pages_matched(), 2);
        assert_eq!(callouts.invocations, 2);

        let admonition = s
            .per_hook()
            .get(&HookKey::new(Stage::PostRender, "admonition"))
            .unwrap();
        assert_eq!(admonition.failures, 1);

        assert_eq!(s.total_invocations(), 6);
    }

    #[test]
    fn record_failures_increments_diagnostics_counter() {
        let mut s = HookStats::new();
        assert_eq!(s.diagnostics_emitted(), 0);

        let recs = vec![
            FailureRecord::new(
                "callouts",
                Stage::Transform,
                "daily",
                FailureReason::Timeout,
                "boom",
                Duration::from_millis(1),
                "t",
            ),
            FailureRecord::new(
                "tasks",
                Stage::PostRender,
                "daily",
                FailureReason::NonZeroExit,
                "exit 2",
                Duration::from_millis(1),
                "t",
            ),
        ];
        s.record_failures(&recs);
        assert_eq!(s.diagnostics_emitted(), 2);

        s.record_failures(&[]);
        assert_eq!(s.diagnostics_emitted(), 2, "empty records don't bump");
    }

    #[test]
    fn pages_matched_dedupes_repeats() {
        let mut s = HookStats::new();
        for _ in 0..5 {
            s.record(&ok_event(Stage::Transform, "callouts", "daily", 1));
        }
        let a = s
            .per_hook()
            .get(&HookKey::new(Stage::Transform, "callouts"))
            .unwrap();
        assert_eq!(a.invocations, 5);
        assert_eq!(
            a.pages_matched(),
            1,
            "5 invocations on same page = 1 page matched"
        );
    }

    #[test]
    fn format_summary_line_matches_obs_3206_shape() {
        let mut s = HookStats::new();
        s.record(&ok_event(Stage::Transform, "callouts", "daily", 10));
        s.record(&ok_event(Stage::Transform, "tasks", "daily", 20));
        s.record_failures(&[FailureRecord::new(
            "tasks",
            Stage::Transform,
            "daily",
            FailureReason::NonZeroExit,
            "boom",
            Duration::from_millis(1),
            "t",
        )]);

        let line = s.format_summary_line();
        assert_eq!(
            line,
            "[zetl] hooks: total_invocations=2 total_duration_ms=30 failures=1"
        );
    }

    #[test]
    fn format_block_lists_stages_and_hooks_deterministically() {
        let mut s = HookStats::new();
        // Register hooks out of stage order — output must still list
        // pre-parse → transform → post-render.
        s.record(&ok_event(Stage::PostRender, "admonition", "p1", 4));
        s.record(&ok_event(Stage::PreParse, "first", "p1", 2));
        s.record(&ok_event(Stage::Transform, "tasks", "p1", 7));
        s.record(&ok_event(Stage::Transform, "callouts", "p1", 10));
        s.record(&ok_event(Stage::Transform, "callouts", "p2", 15));
        s.record_failures(&[FailureRecord::new(
            "tasks",
            Stage::Transform,
            "p1",
            FailureReason::NonZeroExit,
            "boom",
            Duration::from_millis(1),
            "t",
        )]);

        let block = s.format_block();
        // Shape: header then three stage rows then the hooks section.
        assert!(block.starts_with("[zetl] hooks:\n"));
        assert!(block.contains("    pre-parse   duration_ms=2\n"));
        assert!(block.contains("    transform   duration_ms=32\n"));
        assert!(block.contains("    post-render duration_ms=4\n"));

        // Hook lines appear grouped and alphabetised within a stage.
        let idx_first = block.find("id=first").unwrap();
        let idx_callouts = block.find("id=callouts").unwrap();
        let idx_tasks = block.find("id=tasks").unwrap();
        let idx_admonition = block.find("id=admonition").unwrap();
        assert!(
            idx_first < idx_callouts,
            "pre-parse hook must appear before transform hook"
        );
        assert!(
            idx_callouts < idx_tasks,
            "transform hooks ordered alphabetically: callouts < tasks"
        );
        assert!(
            idx_tasks < idx_admonition,
            "transform hooks must appear before post-render hooks"
        );

        // pages_matched dedups pages.
        assert!(block.contains("id=callouts duration_ms=25 pages_matched=2"));
        // totals roll-up line mirrors the OBS-3206 summary.
        assert!(block.contains("  totals: invocations=5 duration_ms=38 diagnostics_emitted=1\n"));
    }

    #[test]
    fn format_block_handles_no_hooks() {
        let s = HookStats::new();
        let block = s.format_block();
        assert!(block.contains("  hooks: (none)\n"));
        assert!(block.contains("    pre-parse   duration_ms=0\n"));
    }

    #[test]
    fn mutex_hook_stats_is_an_observer() {
        let stats = Mutex::new(HookStats::new());
        let observer: &dyn HookObserver = &stats;
        observer.on_hook_invocation(&ok_event(Stage::Transform, "callouts", "p1", 5));
        observer.on_hook_invocation(&ok_event(Stage::Transform, "callouts", "p2", 7));

        let s = stats.lock().unwrap();
        assert_eq!(s.total_invocations(), 2);
        assert_eq!(
            s.per_hook()
                .get(&HookKey::new(Stage::Transform, "callouts"))
                .unwrap()
                .pages_matched(),
            2
        );
    }

    #[test]
    fn stderr_observer_is_quiet_on_success_without_verbose() {
        // Smoke test: the quiet observer does not panic and does not
        // mutate any shared state. We can't assert on stderr output
        // without redirecting it; the behaviour is validated via
        // `run_page` integration in `tests/observability_integration.rs`.
        let obs = StderrObserver::new(false);
        obs.on_hook_invocation(&ok_event(Stage::Transform, "callouts", "p", 1));
    }

    #[test]
    fn noop_observer_accepts_all_events() {
        let obs = NoopObserver;
        obs.on_hook_invocation(&ok_event(Stage::PreParse, "a", "p", 0));
        obs.on_hook_invocation(&fail_event(Stage::Transform, "b", "p", 0, "boom"));
    }

    #[test]
    fn hook_key_orders_by_stage_then_id() {
        let mut keys = vec![
            HookKey::new(Stage::PostRender, "z"),
            HookKey::new(Stage::PreParse, "z"),
            HookKey::new(Stage::Transform, "b"),
            HookKey::new(Stage::Transform, "a"),
            HookKey::new(Stage::PreParse, "a"),
        ];
        keys.sort();
        let rendered: Vec<String> = keys
            .iter()
            .map(|k| format!("{}:{}", k.stage, k.hook_id))
            .collect();
        assert_eq!(
            rendered,
            vec![
                "pre-parse:a",
                "pre-parse:z",
                "transform:a",
                "transform:b",
                "post-render:z",
            ]
        );
    }
}
