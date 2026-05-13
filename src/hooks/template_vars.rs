//! Template variable publishing for render-pipeline hooks
//! (SPEC-032 REQ-3214 / CON-3214).
//!
//! Every hook response MAY include an optional `template_vars` value;
//! this module owns the machinery that accumulates those emissions
//! into the minijinja render context at `page.ext.<extension_id>` and,
//! at end of build, at `vault.ext.<extension_id>`. Hooks never write
//! to `page.ext` or `vault.ext` directly — the pipeline keys each
//! emission on the invoked hook's `extension_id`, so cross-namespace
//! writes are impossible by construction (REQ-3214).
//!
//! ## Namespacing
//!
//! - `page.ext` is reserved for extensions; zetl does not write into
//!   it. Two hooks that share an `extension_id` coalesce into the same
//!   bucket — pipeline-order wins and a warning is logged (REQ-3214
//!   cross-extension coordination).
//! - `vault.ext` is a build-scoped aggregate: each page's accepted
//!   entries are merged into the vault aggregator on page completion.
//!   Release-order-dependent last-write-wins at the
//!   `(extension_id, key)` level, matching the `build_data` semantics
//!   (CON-3219).
//!
//! ## Three-stage publishing
//!
//! §13 Q11 resolves to "allow at all three stages"; the accumulator
//! accepts emissions from `pre-parse`, `transform`, and `post-render`
//! with identical semantics. When the same hook emits at two stages,
//! the later stage's value wholly replaces the earlier one
//! (CON-3214: entire replacement, not recursive merge) and an
//! `overwritten_by_later_stage` warning is recorded (OBS-3209).
//!
//! ## Size cap (1 MiB per hook per page)
//!
//! CON-3214 caps each emission at 1 MiB, measured at the JSON
//! boundary. Oversize emissions are dropped with a warning; the
//! AST/HTML payload of the response is still used. The cap is per
//! emission, not cumulative — a hook that emits 800 KiB at
//! `transform` and 800 KiB at `post-render` is accepted twice (the
//! second overwriting the first) but would be rejected once at
//! 1.1 MiB.
//!
//! ## Warnings (OBS-3209)
//!
//! Each classification of a non-simple emit path records a
//! [`TemplateVarWarning`] with the hook id, stage, and detail. The
//! build orchestrator drains these via [`PageTemplateVars::drain_warnings`]
//! to emit the OBS-3209 log lines.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::hooks::pipeline::Stage;

/// Per-emission byte cap for `template_vars` (REQ-3214 / CON-3214).
///
/// 1 MiB, measured at the JSON boundary via [`serde_json::to_vec`].
pub const TEMPLATE_VARS_SIZE_CAP_BYTES: usize = 1024 * 1024;

/// Outcome of a single [`PageTemplateVars::emit`] call.
///
/// Every arm carries the bytes the caller tried to emit so the
/// orchestrator can surface the value in OBS-3209 log lines without
/// re-serialising.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmitOutcome {
    /// First-time emission for this `hook_id`; value recorded.
    Emitted { bytes: usize, top_level_keys: usize },
    /// Same `hook_id` had a prior emission at `prior_stage`; the new
    /// value replaces the old wholesale (CON-3214). A warning is
    /// also recorded.
    OverwrittenByLaterStage {
        prior_stage: Stage,
        bytes: usize,
        top_level_keys: usize,
    },
    /// Value exceeded [`TEMPLATE_VARS_SIZE_CAP_BYTES`] and was dropped.
    /// The AST/HTML payload on the hook response is unaffected.
    DroppedOversize { bytes: usize, cap: usize },
    /// Value was JSON `null` (or the field was absent). `page.ext.<id>`
    /// is left absent. Per REQ-3214 this is a valid "hook opted not
    /// to publish this time" signal, not a warning.
    Skipped,
}

impl EmitOutcome {
    /// Whether the emission was recorded (first-time or overwriting).
    pub fn is_accepted(&self) -> bool {
        matches!(
            self,
            EmitOutcome::Emitted { .. } | EmitOutcome::OverwrittenByLaterStage { .. }
        )
    }
}

/// Classification for an [`TemplateVarWarning`]; lines up with
/// OBS-3209's `outcome` label axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarningKind {
    /// Emission exceeded the 1 MiB cap.
    DroppedOversize,
    /// A later stage replaced an earlier stage's emission from the
    /// same hook.
    OverwrittenByLaterStage,
    /// Two distinct hooks share the same `extension_id` — logged
    /// when the composition layer detects the collision at load
    /// time. Recorded on the page accumulator so the per-page
    /// diagnostic surface is single-sourced.
    ExtensionIdCollision,
}

impl WarningKind {
    /// Label matching OBS-3209's `outcome` axis.
    pub fn as_str(self) -> &'static str {
        match self {
            WarningKind::DroppedOversize => "dropped_oversize",
            WarningKind::OverwrittenByLaterStage => "overwritten_by_later_stage",
            WarningKind::ExtensionIdCollision => "extension_id_collision",
        }
    }
}

impl std::fmt::Display for WarningKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One diagnostic about a `template_vars` emission. Surfaced via
/// [`PageTemplateVars::drain_warnings`] for the OBS-3209 log line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateVarWarning {
    pub hook_id: String,
    pub stage: Stage,
    pub kind: WarningKind,
    pub detail: String,
}

/// Single accumulated entry under `page.ext[<extension_id>]`.
#[derive(Debug, Clone)]
struct PageEntry {
    vars: Value,
    stage: Stage,
    bytes: usize,
}

/// Per-page accumulator for `page.ext.<extension_id>` emissions.
///
/// One of these is owned by each page render. Hook emissions feed
/// through [`Self::emit`]; the final minijinja context is
/// materialised via [`Self::to_page_ext`] once the pipeline finishes.
/// The accumulator is *not* `Send + Sync` by design — per-page state
/// is single-threaded (CON-3217 intra-page serial execution) so the
/// interior mutability of e.g. `build_data` is not warranted here.
#[derive(Debug, Default)]
pub struct PageTemplateVars {
    entries: BTreeMap<String, PageEntry>,
    warnings: Vec<TemplateVarWarning>,
    cap_bytes: usize,
}

impl PageTemplateVars {
    /// Create an accumulator with the spec-mandated 1 MiB cap.
    pub fn new() -> Self {
        Self::with_cap(TEMPLATE_VARS_SIZE_CAP_BYTES)
    }

    /// Create an accumulator with an explicit cap (tests).
    pub fn with_cap(cap_bytes: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            warnings: Vec::new(),
            cap_bytes,
        }
    }

    /// Register an emission from `hook_id` at `stage`.
    ///
    /// Rules, per REQ-3214 / CON-3214:
    /// - `Value::Null` is treated as "no emission this time";
    ///   `page.ext.<id>` is left untouched.
    /// - Values serialising to > [`Self::cap_bytes`] are dropped with
    ///   a warning.
    /// - A subsequent emission from the same `hook_id` replaces the
    ///   prior value wholesale and records an overwrite warning.
    pub fn emit(&mut self, hook_id: &str, stage: Stage, vars: Value) -> EmitOutcome {
        if vars.is_null() {
            return EmitOutcome::Skipped;
        }
        let bytes = match serde_json::to_vec(&vars) {
            Ok(v) => v.len(),
            Err(_) => 0,
        };
        if bytes > self.cap_bytes {
            self.warnings.push(TemplateVarWarning {
                hook_id: hook_id.to_string(),
                stage,
                kind: WarningKind::DroppedOversize,
                detail: format!("{bytes} bytes exceeds {cap}-byte cap", cap = self.cap_bytes),
            });
            return EmitOutcome::DroppedOversize {
                bytes,
                cap: self.cap_bytes,
            };
        }
        let top_level_keys = count_top_level_keys(&vars);

        match self.entries.get(hook_id) {
            Some(prior) => {
                let prior_stage = prior.stage;
                self.warnings.push(TemplateVarWarning {
                    hook_id: hook_id.to_string(),
                    stage,
                    kind: WarningKind::OverwrittenByLaterStage,
                    detail: format!("replaces emission from stage '{prior_stage}'"),
                });
                self.entries
                    .insert(hook_id.to_string(), PageEntry { vars, stage, bytes });
                EmitOutcome::OverwrittenByLaterStage {
                    prior_stage,
                    bytes,
                    top_level_keys,
                }
            }
            None => {
                self.entries
                    .insert(hook_id.to_string(), PageEntry { vars, stage, bytes });
                EmitOutcome::Emitted {
                    bytes,
                    top_level_keys,
                }
            }
        }
    }

    /// Record an extension-id collision detected by the composition
    /// layer.
    ///
    /// Two distinct hooks declaring the same `extension_id` is valid
    /// — they coalesce into one `page.ext.<id>` bucket per REQ-3214
    /// cross-extension coordination. The warning is informational so
    /// vault/theme authors can see the coupling.
    pub fn note_extension_collision(&mut self, hook_id: &str, stage: Stage, other_hook: &str) {
        self.warnings.push(TemplateVarWarning {
            hook_id: hook_id.to_string(),
            stage,
            kind: WarningKind::ExtensionIdCollision,
            detail: format!("shares extension_id with '{other_hook}'"),
        });
    }

    /// Materialise the `page.ext` JSON object for minijinja.
    ///
    /// Shape: `{"<extension_id>": <vars>, ...}`. Hooks that emitted
    /// only `null` / omitted the field are absent from the object
    /// (template `if page.ext.<id>` resolves false).
    pub fn to_page_ext(&self) -> Value {
        let mut map = serde_json::Map::new();
        for (id, entry) in &self.entries {
            map.insert(id.clone(), entry.vars.clone());
        }
        Value::Object(map)
    }

    /// Inspect the accumulator's warnings without draining.
    pub fn warnings(&self) -> &[TemplateVarWarning] {
        &self.warnings
    }

    /// Pop the accumulator's warnings. The build orchestrator drains
    /// these to emit the OBS-3209 log lines and increment the
    /// `zetl_hook_template_vars_total{outcome}` counter.
    pub fn drain_warnings(&mut self) -> Vec<TemplateVarWarning> {
        std::mem::take(&mut self.warnings)
    }

    /// Number of distinct extension-ids recorded.
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Whether any hook has emitted.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Total bytes across accepted entries.
    pub fn total_bytes(&self) -> usize {
        self.entries.values().map(|e| e.bytes).sum()
    }

    /// Direct read of a single hook's emitted value.
    pub fn get(&self, hook_id: &str) -> Option<&Value> {
        self.entries.get(hook_id).map(|e| &e.vars)
    }

    /// Stage at which `hook_id`'s current value was emitted.
    pub fn stage_for(&self, hook_id: &str) -> Option<Stage> {
        self.entries.get(hook_id).map(|e| e.stage)
    }

    /// Fold accepted entries into the vault-level aggregator.
    ///
    /// Called by the build orchestrator when a page render completes.
    /// Release-order-dependent last-write-wins at the `extension_id`
    /// level — matches `build_data`'s CON-3219 semantics.
    pub fn merge_into_vault(&self, vault: &VaultTemplateVars) {
        let mut guard = vault.inner.lock().unwrap();
        for (id, entry) in &self.entries {
            guard.entries.insert(id.clone(), entry.vars.clone());
        }
    }
}

fn count_top_level_keys(vars: &Value) -> usize {
    match vars {
        Value::Object(m) => m.len(),
        _ => 0,
    }
}

// ── Vault-level aggregator ─────────────────────────────────────────────

/// Build-scoped aggregator for `vault.ext.<extension_id>`.
///
/// Pages contribute via [`PageTemplateVars::merge_into_vault`] as
/// they complete. The aggregator is `Send + Sync` (via
/// [`Arc<Mutex<_>>`]) so concurrent page renders can merge without
/// the orchestrator serialising them. Cross-page write visibility is
/// release-order-dependent — the last merger wins for a given
/// `extension_id`, matching `build_data` (CON-3219).
///
/// Per SPEC-032 §13 Q9 the spec text defers this surface to v1.1;
/// the plan task (`task-template-vars`) commits to it up front so
/// themes can consume `vault.ext` for cross-page aggregates (tag
/// clouds, dead-link totals, task roll-ups) without a later schema
/// change.
#[derive(Clone)]
pub struct VaultTemplateVars {
    inner: Arc<Mutex<VaultInner>>,
}

#[derive(Default)]
struct VaultInner {
    entries: BTreeMap<String, Value>,
}

impl VaultTemplateVars {
    /// Empty aggregator. One of these exists per build.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(VaultInner::default())),
        }
    }

    /// Materialise the `vault.ext` JSON object for minijinja.
    pub fn to_vault_ext(&self) -> Value {
        let guard = self.inner.lock().unwrap();
        let mut map = serde_json::Map::new();
        for (id, vars) in &guard.entries {
            map.insert(id.clone(), vars.clone());
        }
        Value::Object(map)
    }

    /// Whether any page has merged into the aggregator.
    pub fn is_empty(&self) -> bool {
        self.inner.lock().unwrap().entries.is_empty()
    }

    /// Distinct `extension_id` count.
    pub fn entry_count(&self) -> usize {
        self.inner.lock().unwrap().entries.len()
    }

    /// Direct read of the aggregated value for one extension.
    pub fn get(&self, hook_id: &str) -> Option<Value> {
        self.inner.lock().unwrap().entries.get(hook_id).cloned()
    }

    /// Reset the aggregator — called at the top of `zetl serve`
    /// renders so each build starts clean (CON-3219 parity).
    pub fn clear(&self) {
        self.inner.lock().unwrap().entries.clear();
    }
}

impl Default for VaultTemplateVars {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for VaultTemplateVars {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let guard = self.inner.lock().unwrap();
        f.debug_struct("VaultTemplateVars")
            .field("entry_count", &guard.entries.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── TEST-3214 matrix: per-page accumulator ────────────────────────────

    /// Row 1: hook emits `template_vars: {total: 12}` →
    /// `page.ext.<hook_id>.total == 12`.
    #[test]
    fn single_hook_single_emission_lands_under_hook_id() {
        let mut page = PageTemplateVars::new();
        let outcome = page.emit("tasks", Stage::Transform, json!({"total": 12}));
        assert!(matches!(outcome, EmitOutcome::Emitted { .. }));
        assert!(outcome.is_accepted());

        let root = page.to_page_ext();
        assert_eq!(root["tasks"]["total"], json!(12));
    }

    /// Row 2: two hooks emit disjoint vars → both visible at their
    /// respective `page.ext.<id>` paths.
    #[test]
    fn two_hooks_land_under_distinct_extension_ids() {
        let mut page = PageTemplateVars::new();
        page.emit("tasks", Stage::Transform, json!({"total": 12}));
        page.emit("callouts", Stage::Transform, json!({"count": 3}));

        let root = page.to_page_ext();
        assert_eq!(root["tasks"]["total"], json!(12));
        assert_eq!(root["callouts"]["count"], json!(3));
        assert_eq!(page.entry_count(), 2);
    }

    /// Row 3: same hook emits at `transform` then `post-render` →
    /// later wins and a warning is recorded.
    #[test]
    fn same_hook_later_stage_wins_and_warns() {
        let mut page = PageTemplateVars::new();
        let first = page.emit("tasks", Stage::Transform, json!({"total": 12}));
        assert!(matches!(first, EmitOutcome::Emitted { .. }));

        let second = page.emit("tasks", Stage::PostRender, json!({"total": 99}));
        match second {
            EmitOutcome::OverwrittenByLaterStage { prior_stage, .. } => {
                assert_eq!(prior_stage, Stage::Transform);
            }
            other => panic!("expected OverwrittenByLaterStage, got {other:?}"),
        }

        let root = page.to_page_ext();
        assert_eq!(root["tasks"]["total"], json!(99));

        let warnings = page.drain_warnings();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].hook_id, "tasks");
        assert_eq!(warnings[0].stage, Stage::PostRender);
        assert_eq!(warnings[0].kind, WarningKind::OverwrittenByLaterStage);
        assert!(warnings[0].detail.contains("transform"));
    }

    /// Row 4: hook emits > 1 MiB → vars dropped with a warning; the
    /// AST/HTML payload is unaffected (we only test the dropping +
    /// warning contract; payload handling lives elsewhere).
    #[test]
    fn oversize_emission_dropped_with_warning() {
        let cap = 1024; // 1 KiB for a fast test
        let mut page = PageTemplateVars::with_cap(cap);

        let big = "x".repeat(cap * 2);
        let outcome = page.emit("bloat", Stage::Transform, json!({"data": big}));
        match outcome {
            EmitOutcome::DroppedOversize { bytes, cap: c } => {
                assert!(bytes > c, "bytes={bytes} cap={c}");
                assert_eq!(c, cap);
            }
            other => panic!("expected DroppedOversize, got {other:?}"),
        }

        // page.ext.<id> absent — the oversize emission did not land.
        let root = page.to_page_ext();
        assert!(root.get("bloat").is_none());

        let warnings = page.drain_warnings();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].kind, WarningKind::DroppedOversize);
        assert_eq!(warnings[0].hook_id, "bloat");
    }

    /// Row 5: hook emits `template_vars: null` (or omits the field)
    /// → `page.ext.<id>` absent, no warning.
    #[test]
    fn null_emission_is_a_noop() {
        let mut page = PageTemplateVars::new();
        let outcome = page.emit("silent", Stage::PreParse, Value::Null);
        assert_eq!(outcome, EmitOutcome::Skipped);

        let root = page.to_page_ext();
        assert!(root.get("silent").is_none());
        assert!(page.drain_warnings().is_empty());
        assert!(page.is_empty());
    }

    /// Three-stage publishing (§13 Q11): all three stages may emit.
    #[test]
    fn all_three_stages_may_publish() {
        let mut page = PageTemplateVars::new();
        assert!(page
            .emit("a", Stage::PreParse, json!({"where": "pre"}))
            .is_accepted());
        assert!(page
            .emit("b", Stage::Transform, json!({"where": "tf"}))
            .is_accepted());
        assert!(page
            .emit("c", Stage::PostRender, json!({"where": "post"}))
            .is_accepted());

        let root = page.to_page_ext();
        assert_eq!(root["a"]["where"], "pre");
        assert_eq!(root["b"]["where"], "tf");
        assert_eq!(root["c"]["where"], "post");
    }

    /// Multi-stage emission by a single hook: each later emission
    /// replaces the prior one wholesale (CON-3214 "entire
    /// replacement, not recursive merge").
    #[test]
    fn multi_stage_replacement_is_wholesale_not_recursive() {
        let mut page = PageTemplateVars::new();
        page.emit(
            "tasks",
            Stage::Transform,
            json!({"total": 12, "by_project": {"a": 5}}),
        );
        page.emit("tasks", Stage::PostRender, json!({"total": 13}));

        let vars = page.get("tasks").unwrap();
        // `by_project` is gone — not recursively merged.
        assert_eq!(vars, &json!({"total": 13}));
    }

    /// Non-object scalar values are accepted (CON-3214 "any valid
    /// JSON value"). top_level_keys is reported as 0.
    #[test]
    fn scalar_and_array_values_are_accepted() {
        let mut page = PageTemplateVars::new();
        let o_num = page.emit("n", Stage::Transform, json!(42));
        let o_arr = page.emit("a", Stage::Transform, json!(["x", "y"]));
        let o_str = page.emit("s", Stage::Transform, json!("hello"));

        for (o, hook) in [(o_num, "n"), (o_arr, "a"), (o_str, "s")] {
            match o {
                EmitOutcome::Emitted { top_level_keys, .. } => {
                    assert_eq!(top_level_keys, 0, "hook={hook}");
                }
                other => panic!("hook={hook} unexpected {other:?}"),
            }
        }
        let root = page.to_page_ext();
        assert_eq!(root["n"], 42);
        assert_eq!(root["a"], json!(["x", "y"]));
        assert_eq!(root["s"], "hello");
    }

    /// Extension-id collision: two distinct hooks share an
    /// `extension_id`; the composition layer notes it; the page
    /// accumulator surfaces the warning alongside any emissions.
    #[test]
    fn extension_id_collision_recorded_as_warning() {
        let mut page = PageTemplateVars::new();
        page.note_extension_collision("tasks", Stage::Transform, "vault-tasks");
        let warnings = page.drain_warnings();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].kind, WarningKind::ExtensionIdCollision);
        assert!(warnings[0].detail.contains("vault-tasks"));
    }

    #[test]
    fn emit_outcome_is_accepted_semantics() {
        assert!(EmitOutcome::Emitted {
            bytes: 0,
            top_level_keys: 0
        }
        .is_accepted());
        assert!(EmitOutcome::OverwrittenByLaterStage {
            prior_stage: Stage::Transform,
            bytes: 0,
            top_level_keys: 0
        }
        .is_accepted());
        assert!(!EmitOutcome::DroppedOversize { bytes: 0, cap: 0 }.is_accepted());
        assert!(!EmitOutcome::Skipped.is_accepted());
    }

    #[test]
    fn size_cap_constant_is_one_mib() {
        assert_eq!(TEMPLATE_VARS_SIZE_CAP_BYTES, 1024 * 1024);
    }

    #[test]
    fn warning_kind_strings_match_obs_3209_axis() {
        assert_eq!(WarningKind::DroppedOversize.as_str(), "dropped_oversize");
        assert_eq!(
            WarningKind::OverwrittenByLaterStage.as_str(),
            "overwritten_by_later_stage"
        );
        assert_eq!(
            WarningKind::ExtensionIdCollision.as_str(),
            "extension_id_collision"
        );
    }

    #[test]
    fn total_bytes_tracks_accepted_entries() {
        let mut page = PageTemplateVars::new();
        page.emit("a", Stage::Transform, json!({"x": "hello"}));
        let after_one = page.total_bytes();
        assert!(after_one > 0);
        page.emit("b", Stage::Transform, json!({"y": "world"}));
        assert!(page.total_bytes() > after_one);
    }

    #[test]
    fn stage_for_returns_latest_stage() {
        let mut page = PageTemplateVars::new();
        page.emit("a", Stage::PreParse, json!({"v": 1}));
        assert_eq!(page.stage_for("a"), Some(Stage::PreParse));
        page.emit("a", Stage::PostRender, json!({"v": 2}));
        assert_eq!(page.stage_for("a"), Some(Stage::PostRender));
        assert_eq!(page.stage_for("missing"), None);
    }

    // ── Vault-level aggregator ────────────────────────────────────────────

    #[test]
    fn vault_aggregator_starts_empty() {
        let v = VaultTemplateVars::new();
        assert!(v.is_empty());
        assert_eq!(v.entry_count(), 0);
        assert_eq!(v.to_vault_ext(), json!({}));
    }

    #[test]
    fn page_merge_populates_vault_ext() {
        let mut page = PageTemplateVars::new();
        page.emit("tasks", Stage::Transform, json!({"total": 12}));
        page.emit("callouts", Stage::PostRender, json!({"count": 3}));

        let vault = VaultTemplateVars::new();
        page.merge_into_vault(&vault);

        let root = vault.to_vault_ext();
        assert_eq!(root["tasks"]["total"], 12);
        assert_eq!(root["callouts"]["count"], 3);
        assert_eq!(vault.entry_count(), 2);
    }

    #[test]
    fn vault_aggregator_is_release_order_last_write_wins() {
        let vault = VaultTemplateVars::new();

        // Page A contributes first.
        let mut page_a = PageTemplateVars::new();
        page_a.emit("tasks", Stage::Transform, json!({"total": 1}));
        page_a.merge_into_vault(&vault);

        // Page B contributes after; wins.
        let mut page_b = PageTemplateVars::new();
        page_b.emit("tasks", Stage::Transform, json!({"total": 99}));
        page_b.merge_into_vault(&vault);

        assert_eq!(vault.get("tasks"), Some(json!({"total": 99})));
    }

    #[test]
    fn vault_is_cloneable_and_shares_state() {
        let v1 = VaultTemplateVars::new();
        let v2 = v1.clone();

        let mut page = PageTemplateVars::new();
        page.emit("tasks", Stage::Transform, json!({"x": 1}));
        page.merge_into_vault(&v1);

        // Clone sees the merge because both share the Arc<Mutex<_>>.
        assert_eq!(v2.get("tasks"), Some(json!({"x": 1})));
    }

    #[test]
    fn vault_clear_empties_aggregator() {
        let vault = VaultTemplateVars::new();
        let mut page = PageTemplateVars::new();
        page.emit("tasks", Stage::Transform, json!({"x": 1}));
        page.merge_into_vault(&vault);
        assert!(!vault.is_empty());
        vault.clear();
        assert!(vault.is_empty());
    }

    #[test]
    fn vault_aggregator_is_send_and_sync() {
        // Compile-time assertion: VaultTemplateVars must be Send + Sync
        // so the build orchestrator can merge across threads.
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<VaultTemplateVars>();
    }

    // ── Integration: minijinja autoescape ───────────────────────────────

    /// TEST-3214 row 7: string value containing `<script>` is
    /// autoescaped when rendered through minijinja's standard HTML
    /// autoescape path — no new XSS surface (CON-3214).
    #[test]
    fn string_values_autoescape_through_minijinja() {
        let mut page = PageTemplateVars::new();
        page.emit(
            "xss",
            Stage::Transform,
            json!({"note": "<script>alert(1)</script>"}),
        );

        let mut env = minijinja::Environment::new();
        // Match the global autoescape rule zetl uses for .html templates.
        env.set_auto_escape_callback(|_| minijinja::AutoEscape::Html);
        env.add_template("t.html", "{{ page.ext.xss.note }}")
            .unwrap();

        let rendered = env
            .get_template("t.html")
            .unwrap()
            .render(minijinja::context! {
                page => minijinja::context! { ext => page.to_page_ext() }
            })
            .unwrap();

        // Autoescape fires: raw `<` / `>` are entity-encoded, so the
        // payload cannot execute. (minijinja also escapes `/` — the
        // exact output of a safe string is renderer-defined; we assert
        // the dangerous tokens are gone.)
        assert!(!rendered.contains("<script>"), "rendered: {rendered}");
        assert!(!rendered.contains("</script>"), "rendered: {rendered}");
        assert!(rendered.contains("&lt;script&gt;"), "rendered: {rendered}");
    }

    /// TEST-3214 row 6: `{{ page.ext.unknown.field }}` on an absent
    /// extension renders empty (minijinja's default) rather than
    /// raising. zetl's render env uses `Chainable` undefined
    /// behaviour so deeply-chained access on a missing extension
    /// keeps returning undefined.
    #[test]
    fn absent_extension_path_renders_empty_under_minijinja() {
        let page = PageTemplateVars::new();

        let mut env = minijinja::Environment::new();
        env.set_undefined_behavior(minijinja::UndefinedBehavior::Chainable);
        env.add_template("t.html", "before|{{ page.ext.unknown.field }}|after")
            .unwrap();

        let rendered = env
            .get_template("t.html")
            .unwrap()
            .render(minijinja::context! {
                page => minijinja::context! { ext => page.to_page_ext() }
            })
            .unwrap();

        assert_eq!(rendered, "before||after");
    }

    /// Template `if page.ext.<hook_id>` correctly reads false when
    /// the hook emitted `null` / did not emit.
    #[test]
    fn absent_extension_is_falsy_under_minijinja_if() {
        let mut page = PageTemplateVars::new();
        page.emit("emitted", Stage::Transform, json!({"ok": true}));
        // `skipped` hook never populates the map.
        page.emit("skipped", Stage::Transform, Value::Null);

        let mut env = minijinja::Environment::new();
        env.add_template(
            "t.html",
            "{% if page.ext.emitted %}E{% endif %}{% if page.ext.skipped %}S{% endif %}",
        )
        .unwrap();

        let rendered = env
            .get_template("t.html")
            .unwrap()
            .render(minijinja::context! {
                page => minijinja::context! { ext => page.to_page_ext() }
            })
            .unwrap();

        assert_eq!(rendered, "E");
    }
}
