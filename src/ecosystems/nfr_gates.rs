//! Performance + lifecycle gates for SPEC-033 NFR-3301..NFR-3308.
//!
//! Mirrors the SPEC-032 [`crate::hooks::nfr_gates`] surface but covers
//! the ecosystem-layer budgets: cold-start (NFR-3301), per-page round
//! trip latency for native-pipe (NFR-3302) and Node-harness (NFR-3303)
//! ecosystems, the per-feature binary-size budget (NFR-3304), the
//! canonical-form fidelity scope (NFR-3305), determinism scope
//! (NFR-3306), per-ecosystem process-lifecycle ceilings (NFR-3307),
//! and the combined-ecosystem build-wall-time multiplier (NFR-3308).
//!
//! Each constant pins a spec number verbatim so a budget edit lands as
//! a single-line diff that a reviewer can compare against the spec; each
//! helper exposes a pure, transport-free entry point so the test
//! harness, microbenchmarks, and any future `cargo nfr-gates`
//! aggregator can re-use them without dragging the build graph in.
//!
//! ## NFR coverage matrix
//!
//! | Const                                | NFR      | Verifies                                          |
//! |--------------------------------------|----------|---------------------------------------------------|
//! | [`COLD_START_BUDGET`]                | NFR-3301 | per-ecosystem cold-start activation ≤ 200 ms P95  |
//! | [`PIPE_ROUND_TRIP_BUDGET`]           | NFR-3302 | persistent Pandoc/mdBook round-trip ≤ 15 ms P95   |
//! | [`NODE_HARNESS_ROUND_TRIP_BUDGET`]   | NFR-3303 | remark Node harness round-trip ≤ 30 ms P95        |
//! | [`PER_FEATURE_SIZE_BUDGET_BYTES`]    | NFR-3304 | each ecosystem feature flag adds ≤ 2 MiB          |
//! | [`ROUND_TRIP_FIDELITY_CORPUS`]       | NFR-3305 | canonical-form equivalence corpus size            |
//! | [`DETERMINISM_SCOPE`]                | NFR-3306 | adapter+translator layer determinism scope        |
//! | [`PROCESS_MEMORY_CEILINGS`]          | NFR-3307 | per-ecosystem process resident memory ceilings    |
//! | [`COMBINED_BUILD_MULTIPLIER`]        | NFR-3308 | all-ecosystems build wall-time ≤ 3× baseline      |

use std::time::Duration;

use crate::ecosystems::registry::Ecosystem;
use crate::hooks::ast::Document;
use crate::hooks::translators::canonicalise::canonicalise;
use crate::hooks::translators::AstType;

// ── NFR-3301: Cold-start budget ─────────────────────────────────────────────

/// NFR-3301: per-ecosystem cold-start budget (binary probe + adapter
/// activation) measured at P95 over 100 runs. The 200 ms ceiling is
/// the single number every adapter must clear.
pub const COLD_START_BUDGET: Duration = Duration::from_millis(200);

// ── NFR-3302 / NFR-3303: per-page round-trip budgets ────────────────────────

/// NFR-3302: per-page persistent-mode round-trip budget for OS-pipe
/// adapters (Pandoc, mdBook). Tighter than SPEC-032 NFR-3207's 10 ms
/// because the persistent transport amortises spawn cost; the 15 ms
/// here is the spec's round-trip ceiling on a 500-node AST.
pub const PIPE_ROUND_TRIP_BUDGET: Duration = Duration::from_millis(15);

/// NFR-3303: per-page round-trip budget for the remark Node harness
/// (one-plugin invocation on a 500-node mdast). Looser than NFR-3302
/// because JSON marshalling across the Node boundary costs more than
/// pipe-serialised AST exchange.
pub const NODE_HARNESS_ROUND_TRIP_BUDGET: Duration = Duration::from_millis(30);

/// Look up the per-page round-trip budget for an [`Ecosystem`].
///
/// Pandoc + mdBook map to [`PIPE_ROUND_TRIP_BUDGET`]; remark maps to
/// [`NODE_HARNESS_ROUND_TRIP_BUDGET`]. Adding a new ecosystem means
/// adding a row here — the function is exhaustive on purpose so the
/// compiler reminds the author when they forget.
pub const fn round_trip_budget_for(ecosystem: Ecosystem) -> Duration {
    match ecosystem {
        Ecosystem::Pandoc | Ecosystem::Mdbook => PIPE_ROUND_TRIP_BUDGET,
        Ecosystem::Remark => NODE_HARNESS_ROUND_TRIP_BUDGET,
    }
}

// ── NFR-3304: Binary-size budget per feature ────────────────────────────────

/// NFR-3304: each ecosystem feature flag adds at most 2 MiB to the
/// release binary's stripped size. Measured via
/// `cargo build --release --features ecosystem-<id>` minus the no-
/// feature baseline; the test harness reads the resulting artefact
/// from `target/release/ztl` and compares against this constant.
pub const PER_FEATURE_SIZE_BUDGET_BYTES: u64 = 2 * 1024 * 1024;

/// Returns `true` iff `delta_bytes` clears the NFR-3304 ceiling. Pure
/// helper so a future bench harness can short-circuit reporting.
pub const fn size_delta_within_budget(delta_bytes: u64) -> bool {
    delta_bytes <= PER_FEATURE_SIZE_BUDGET_BYTES
}

// ── NFR-3305: Translation round-trip fidelity ───────────────────────────────

/// NFR-3305: release-gate corpus size for the canonical-form
/// equivalence sweep. Per-translator integration covers 64 cases on
/// every push (cheap CI gate); this is the nightly / release sweep
/// target the property harness honours when `PROPTEST_CASES` is set.
pub const ROUND_TRIP_FIDELITY_CORPUS: u32 = 10_000;

/// Pure NFR-3305 equivalence check: `lhs` and `rhs` agree under the
/// per-`ast_type` canonical-form normaliser ([`canonicalise`]).
///
/// Wraps the existing translator helper so callers in test harnesses
/// don't have to reach across module boundaries — the gate is a
/// one-liner: `assert!(round_trip_fidelity_holds(...))`.
pub fn round_trip_fidelity_holds(ast_type: AstType, lhs: &Document, rhs: &Document) -> bool {
    canonicalise(ast_type, lhs) == canonicalise(ast_type, rhs)
}

// ── NFR-3306: Determinism scope ─────────────────────────────────────────────

/// NFR-3306: spec scope tag — `adapter-and-translator-layer`.
///
/// Pinned as a constant so a downstream module that surfaces the gate
/// in `ztl ecosystem check` (or a future `--strict-determinism`
/// mode) reads the same scope-string the spec quotes. Plugin-internal
/// non-determinism is explicitly out of scope per spec; users wanting
/// stricter end-to-end determinism are routed to the matrix-pin /
/// post-process-normaliser pattern.
pub const DETERMINISM_SCOPE: &str = "adapter-and-translator-layer";

// ── NFR-3307: Process lifecycle ceilings ────────────────────────────────────

/// NFR-3307: ecosystem runtime process-lifecycle bounds.
///
/// One row per ecosystem, ordered to match the spec's bullet list. The
/// resident-memory cap is the headline ceiling each ecosystem's adapter
/// enforces (Pandoc filters per persistent process, mdBook
/// preprocessors per spawn, the remark harness as one process per ztl
/// run).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessLifecycle {
    pub ecosystem: Ecosystem,
    /// Per-process resident-memory ceiling, in bytes. Adapters respawn
    /// the runtime when this limit is breached.
    pub max_resident_bytes: u64,
    /// Whether the ecosystem's adapter holds the runtime open across
    /// invocations (`true` — long-lived) or re-spawns per invocation
    /// (`false` — one-shot). mdBook is one-shot by default with an
    /// opt-in to persistent.
    pub persistent_default: bool,
}

/// NFR-3307 lifecycle table — one entry per ecosystem.
///
/// Adding a new ecosystem requires extending this slice; the test
/// harness asserts every registry entry has a matching row.
pub const PROCESS_MEMORY_CEILINGS: &[ProcessLifecycle] = &[
    ProcessLifecycle {
        ecosystem: Ecosystem::Pandoc,
        max_resident_bytes: 50 * 1024 * 1024,
        persistent_default: true,
    },
    ProcessLifecycle {
        ecosystem: Ecosystem::Mdbook,
        // mdBook preprocessors are one-shot by default per spec; the
        // resident-memory ceiling matches Pandoc's persistent row to
        // keep behaviour comparable for users who opt in.
        max_resident_bytes: 50 * 1024 * 1024,
        persistent_default: false,
    },
    ProcessLifecycle {
        ecosystem: Ecosystem::Remark,
        // Single Node harness per ztl process — the larger ceiling
        // reflects V8 + harness + plugin module footprints.
        max_resident_bytes: 256 * 1024 * 1024,
        persistent_default: true,
    },
];

/// Look up the [`ProcessLifecycle`] row for an [`Ecosystem`]. Returns
/// `None` only if the table drifted out of sync with the registry —
/// the test harness asserts this never happens, so callers in CI code
/// can `.expect("registry sync")`.
pub fn lifecycle_for(ecosystem: Ecosystem) -> Option<&'static ProcessLifecycle> {
    PROCESS_MEMORY_CEILINGS
        .iter()
        .find(|r| r.ecosystem == ecosystem)
}

// ── NFR-3308: Combined-ecosystem build budget ───────────────────────────────

/// NFR-3308: maximum allowed wall-time multiplier when every v1
/// ecosystem is enabled simultaneously, relative to the
/// `--no-ecosystems` baseline. The 3× ceiling is the spec's "honest
/// upper bound" — individual workloads should sit below it; this is
/// the gate that fails CI if real-world enablement regresses past
/// pathological.
pub const COMBINED_BUILD_MULTIPLIER: f64 = 3.0;

/// Compute the combined-ecosystem build multiplier from a (baseline,
/// treatment) wall-clock pair. Mirrors the
/// [`crate::hooks::nfr_gates::overhead_ratio`] semantics but reports
/// the raw multiplier rather than the (treatment-baseline)/baseline
/// inflation, because the NFR-3308 budget is expressed as "≤ 3×",
/// not "≤ 200 % over".
pub fn combined_multiplier(baseline: Duration, treatment: Duration) -> f64 {
    let b = baseline.as_secs_f64();
    if b == 0.0 {
        return f64::NAN;
    }
    treatment.as_secs_f64() / b
}

/// Returns `true` iff `treatment / baseline ≤ 3.0`. The harness's
/// canonical gate; broken out so the failure case can format a
/// reviewer-friendly message without re-deriving the multiplier.
pub fn combined_within_budget(baseline: Duration, treatment: Duration) -> bool {
    let m = combined_multiplier(baseline, treatment);
    m.is_finite() && m <= COMBINED_BUILD_MULTIPLIER
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Cold-start gate ────────────────────────────────────────────────

    #[test]
    fn cold_start_budget_pinned_to_spec() {
        assert_eq!(COLD_START_BUDGET, Duration::from_millis(200));
    }

    // ── Per-page round-trip lookup ─────────────────────────────────────

    #[test]
    fn round_trip_budget_for_each_ecosystem() {
        assert_eq!(
            round_trip_budget_for(Ecosystem::Pandoc),
            PIPE_ROUND_TRIP_BUDGET
        );
        assert_eq!(
            round_trip_budget_for(Ecosystem::Mdbook),
            PIPE_ROUND_TRIP_BUDGET
        );
        assert_eq!(
            round_trip_budget_for(Ecosystem::Remark),
            NODE_HARNESS_ROUND_TRIP_BUDGET
        );
    }

    #[test]
    fn round_trip_budget_constants_pin_to_spec() {
        assert_eq!(PIPE_ROUND_TRIP_BUDGET, Duration::from_millis(15));
        assert_eq!(NODE_HARNESS_ROUND_TRIP_BUDGET, Duration::from_millis(30));
    }

    // ── Size budget ────────────────────────────────────────────────────

    #[test]
    fn size_budget_pinned_to_2mib() {
        assert_eq!(PER_FEATURE_SIZE_BUDGET_BYTES, 2 * 1024 * 1024);
    }

    #[test]
    fn size_delta_within_budget_boundaries() {
        assert!(size_delta_within_budget(0));
        assert!(size_delta_within_budget(PER_FEATURE_SIZE_BUDGET_BYTES));
        assert!(!size_delta_within_budget(PER_FEATURE_SIZE_BUDGET_BYTES + 1));
    }

    // ── Round-trip fidelity wrapper ────────────────────────────────────

    #[test]
    fn round_trip_fidelity_holds_for_identical_documents() {
        use crate::hooks::ast::{Document, DocumentKind, Position, AST_VERSION};
        let doc = Document {
            ast_version: AST_VERSION.to_string(),
            kind: DocumentKind::Document,
            position: Position::origin(),
            frontmatter: None,
            children: vec![],
        };
        for ast_type in [AstType::ztlExt, AstType::MdastExt, AstType::PandocExt] {
            assert!(
                round_trip_fidelity_holds(ast_type, &doc, &doc),
                "identity must always satisfy NFR-3305 for {ast_type}"
            );
        }
    }

    #[test]
    fn round_trip_fidelity_corpus_pinned() {
        // Bump intentionally — paired with a CHANGELOG entry — when the
        // release-gate corpus widens.
        assert_eq!(ROUND_TRIP_FIDELITY_CORPUS, 10_000);
    }

    // ── Determinism scope tag ──────────────────────────────────────────

    #[test]
    fn determinism_scope_pinned_to_adapter_and_translator() {
        assert_eq!(DETERMINISM_SCOPE, "adapter-and-translator-layer");
    }

    // ── Process lifecycle table ────────────────────────────────────────

    #[test]
    fn process_lifecycle_table_has_row_for_every_ecosystem() {
        use crate::ecosystems::registry::ECOSYSTEMS;
        for entry in ECOSYSTEMS {
            assert!(
                lifecycle_for(entry.ecosystem).is_some(),
                "NFR-3307: registry has '{}' but lifecycle table doesn't",
                entry.id
            );
        }
        assert_eq!(PROCESS_MEMORY_CEILINGS.len(), ECOSYSTEMS.len());
    }

    #[test]
    fn process_lifecycle_pins_per_ecosystem_ceilings() {
        let pandoc = lifecycle_for(Ecosystem::Pandoc).unwrap();
        assert_eq!(pandoc.max_resident_bytes, 50 * 1024 * 1024);
        assert!(pandoc.persistent_default);

        let mdbook = lifecycle_for(Ecosystem::Mdbook).unwrap();
        assert_eq!(mdbook.max_resident_bytes, 50 * 1024 * 1024);
        assert!(
            !mdbook.persistent_default,
            "NFR-3307: mdBook preprocessors are one-shot by default"
        );

        let remark = lifecycle_for(Ecosystem::Remark).unwrap();
        assert_eq!(remark.max_resident_bytes, 256 * 1024 * 1024);
        assert!(remark.persistent_default);
    }

    // ── Combined-ecosystem multiplier ──────────────────────────────────

    #[test]
    fn combined_multiplier_basic_ratios() {
        assert_eq!(
            combined_multiplier(Duration::from_millis(1000), Duration::from_millis(2000)),
            2.0
        );
        assert_eq!(
            combined_multiplier(Duration::from_millis(1000), Duration::from_millis(3000)),
            3.0
        );
    }

    #[test]
    fn combined_multiplier_zero_baseline_is_nan() {
        assert!(combined_multiplier(Duration::ZERO, Duration::from_millis(1)).is_nan());
    }

    #[test]
    fn combined_within_budget_at_3x_passes_just_over_fails() {
        let baseline = Duration::from_millis(1_000);
        assert!(combined_within_budget(
            baseline,
            Duration::from_millis(3_000)
        ));
        assert!(!combined_within_budget(
            baseline,
            Duration::from_millis(3_001)
        ));
        // Zero baseline reads as NaN → fail (we won't certify a build
        // we can't measure).
        assert!(!combined_within_budget(
            Duration::ZERO,
            Duration::from_millis(1)
        ));
    }

    #[test]
    fn combined_multiplier_pinned_to_3x() {
        assert!((COMBINED_BUILD_MULTIPLIER - 3.0).abs() < 1e-9);
    }
}
