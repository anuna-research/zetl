//! SPEC-033 NFR-3301..NFR-3308 — ecosystem performance + lifecycle gates.
//!
//! Each NFR has its own `#[test]` so a regression names the breached
//! NFR in CI's failure summary. Mirrors the SPEC-032 split in
//! `tests/nfr_gates_integration.rs`:
//!
//! - **Default-on** — cheap, host-stable invariants (pinned constants,
//!   spec-derived lookup tables, schema scope). Run on every
//!   `cargo test`.
//! - **Coarse default-on** — exercise the gate's helper surface against
//!   a small hand-crafted workload; aggregate budget checks rather than
//!   strict P95 so a slow CI worker doesn't flake them.
//! - **`#[ignore]`-gated** — full-spec budgets that need a stable host or
//!   a built release artefact (cold-start probe of every runtime,
//!   binary-size delta vs the no-feature baseline, combined-ecosystem
//!   build comparison). Run manually via:
//!
//!   ```text
//!   cargo test --release --test nfr_gates_033_integration -- --ignored
//!   ```
//!
//! Module-level matrix:
//!
//! | NFR      | Default test                                                | Strict (`--ignored`)                                  |
//! |----------|-------------------------------------------------------------|-------------------------------------------------------|
//! | NFR-3301 | `nfr_3301_cold_start_budget_pinned`                         | `nfr_3301_cold_start_strict`                          |
//! | NFR-3302 | `nfr_3302_pipe_round_trip_budget_pinned`                    | —                                                     |
//! | NFR-3303 | `nfr_3303_node_harness_round_trip_budget_pinned`            | —                                                     |
//! | NFR-3304 | `nfr_3304_size_budget_pinned`                               | `nfr_3304_release_binary_under_size_budget`           |
//! | NFR-3305 | `nfr_3305_round_trip_fidelity_holds_for_known_docs`         | —                                                     |
//! | NFR-3306 | `nfr_3306_determinism_canonicalise_is_idempotent`           | —                                                     |
//! | NFR-3307 | `nfr_3307_process_lifecycle_table_matches_registry`         | —                                                     |
//! | NFR-3308 | `nfr_3308_combined_multiplier_pinned_and_helper_works`      | —                                                     |

use std::path::PathBuf;
use std::time::{Duration, Instant};

use zetl::ecosystems::nfr_gates::{
    combined_multiplier, combined_within_budget, lifecycle_for, round_trip_budget_for,
    round_trip_fidelity_holds, size_delta_within_budget, COLD_START_BUDGET,
    COMBINED_BUILD_MULTIPLIER, DETERMINISM_SCOPE, NODE_HARNESS_ROUND_TRIP_BUDGET,
    PER_FEATURE_SIZE_BUDGET_BYTES, PIPE_ROUND_TRIP_BUDGET, PROCESS_MEMORY_CEILINGS,
    ROUND_TRIP_FIDELITY_CORPUS,
};
use zetl::ecosystems::registry::{Ecosystem, ECOSYSTEMS};
use zetl::ecosystems::{detect_all_ecosystems, probe_runtime_dep};
use zetl::hooks::ast::{
    Block, Document, DocumentKind, Heading, Paragraph, Position, Text, Wikilink, AST_VERSION,
};
use zetl::hooks::nfr_gates::p95;
use zetl::hooks::translators::canonicalise::canonicalise;
use zetl::hooks::translators::AstType;

// ── shared fixtures ─────────────────────────────────────────────────────────

fn pos() -> Position {
    Position::origin()
}

/// Hand-crafted ~50-node zetl-ext document — enough breadth for
/// canonicalise + dispatch helpers without dragging proptest into the
/// integration crate.
fn fixture_document() -> Document {
    let mut children: Vec<Block> = Vec::new();
    children.push(Block::Heading(Heading {
        position: pos(),
        level: 1,
        children: vec![text("Title")],
    }));
    for i in 0..8 {
        children.push(Block::Paragraph(Paragraph {
            position: pos(),
            children: vec![
                text(&format!("para {i} preface ")),
                wikilink(&format!("note-{i}")),
                text(" — body."),
            ],
        }));
    }
    Document {
        ast_version: AST_VERSION.to_string(),
        kind: DocumentKind::Document,
        position: pos(),
        frontmatter: None,
        children,
    }
}

fn text(s: &str) -> zetl::hooks::ast::Inline {
    zetl::hooks::ast::Inline::Text(Text {
        position: pos(),
        text: s.to_string(),
    })
}

fn wikilink(target: &str) -> zetl::hooks::ast::Inline {
    zetl::hooks::ast::Inline::Wikilink(Wikilink {
        position: pos(),
        target: target.to_string(),
        alias: None,
        heading: None,
        block_id: None,
    })
}

// ── NFR-3301: Cold-start budget ─────────────────────────────────────────────

/// Spec-pin gate: NFR-3301's 200 ms ceiling is encoded as a constant.
/// Drift requires a CHANGELOG entry — the constant lives in
/// `src/ecosystems/nfr_gates.rs` so a budget edit is a single-line diff.
#[test]
fn nfr_3301_cold_start_budget_pinned() {
    assert_eq!(
        COLD_START_BUDGET,
        Duration::from_millis(200),
        "NFR-3301: cold-start budget drifted from spec value (200 ms)"
    );
}

/// Strict gate: time the registry-wide runtime probe and assert the
/// per-ecosystem cold-start budget. Marked `#[ignore]` because the
/// timing is host-dependent and needs the runtimes available — local
/// dev machines usually have at least pandoc + node + mdbook installed,
/// CI runners often don't.
#[test]
#[ignore]
fn nfr_3301_cold_start_strict() {
    let mut samples: Vec<(Ecosystem, Duration)> = Vec::new();
    for entry in ECOSYSTEMS {
        let t0 = Instant::now();
        // The probe is what NFR-3301 measures — a single
        // `<binary> --version` invocation per ecosystem.
        let _status = probe_runtime_dep(&entry.runtime_dep);
        samples.push((entry.ecosystem, t0.elapsed()));
    }

    for (eco, dur) in &samples {
        // Use a single-sample assertion; the spec's "P95 over 100 runs"
        // is honoured by the bench harness — at integration-test scope
        // a single probe is the available signal.
        eprintln!("NFR-3301 cold-start: {eco} → {dur:?} (budget {COLD_START_BUDGET:?})");
        assert!(
            *dur <= COLD_START_BUDGET,
            "NFR-3301: ecosystem {eco} cold-start {dur:?} exceeds budget {COLD_START_BUDGET:?}"
        );
    }
}

/// Smoke gate: aggregate the registry's runtime detection sweep
/// — the entry point the build command pays at startup. We don't
/// enforce a budget on each runtime (some runners genuinely lack
/// `pandoc` etc.); we assert the probe call itself is fast for
/// the ecosystems whose binaries *are* present.
#[test]
fn nfr_3301_detect_all_ecosystems_returns_within_aggregate_window() {
    let t0 = Instant::now();
    let report = detect_all_ecosystems();
    let elapsed = t0.elapsed();

    // Generous aggregate ceiling: 3 ecosystems × probe-timeout (2 s).
    // The real budget is 200 ms each, but this gate is the smoke test
    // that runs on every CI pipeline regardless of which runtimes are
    // installed — the strict per-ecosystem assertion lives in the
    // `--ignored` arm above.
    let smoke_ceiling = Duration::from_secs(8);
    assert!(
        elapsed <= smoke_ceiling,
        "NFR-3301 smoke: detect_all_ecosystems took {elapsed:?} (smoke ceiling {smoke_ceiling:?})"
    );
    assert_eq!(
        report.entries.len(),
        ECOSYSTEMS.len(),
        "NFR-3301: detection must report one entry per registered ecosystem"
    );
}

// ── NFR-3302 / 3303: Per-page round-trip budgets ────────────────────────────

/// NFR-3302: pipe-adapter (Pandoc, mdBook) round-trip budget pin.
#[test]
fn nfr_3302_pipe_round_trip_budget_pinned() {
    assert_eq!(
        PIPE_ROUND_TRIP_BUDGET,
        Duration::from_millis(15),
        "NFR-3302: pipe round-trip budget drifted from 15 ms"
    );
    assert_eq!(
        round_trip_budget_for(Ecosystem::Pandoc),
        PIPE_ROUND_TRIP_BUDGET
    );
    assert_eq!(
        round_trip_budget_for(Ecosystem::Mdbook),
        PIPE_ROUND_TRIP_BUDGET
    );
}

/// NFR-3303: Node harness round-trip budget pin (remark).
#[test]
fn nfr_3303_node_harness_round_trip_budget_pinned() {
    assert_eq!(
        NODE_HARNESS_ROUND_TRIP_BUDGET,
        Duration::from_millis(30),
        "NFR-3303: node-harness round-trip budget drifted from 30 ms"
    );
    assert_eq!(
        round_trip_budget_for(Ecosystem::Remark),
        NODE_HARNESS_ROUND_TRIP_BUDGET,
        "NFR-3303: remark must use the Node-harness budget, not the pipe budget"
    );
}

/// Coarse gate: ensure the in-process translation step (the part of the
/// round-trip zetl owns; the rest is the foreign runtime's cost) is
/// orders-of-magnitude inside the per-page budget. We translate the
/// 25-node fixture 1,000 times through every translator and assert the
/// aggregate clears the per-call budget × sample count.
#[test]
fn nfr_3302_3303_translation_layer_is_well_under_budget() {
    use zetl::hooks::translators::TranslatorRegistry;

    let reg = TranslatorRegistry::all_v1();
    let doc = fixture_document();

    for ast_type in [AstType::ZetlExt, AstType::MdastExt, AstType::PandocExt] {
        let translator = reg.get(ast_type).expect("translator registered");
        let mut samples = Vec::with_capacity(1_000);
        for _ in 0..1_000 {
            let t0 = Instant::now();
            let foreign = translator
                .zetl_to_foreign(&doc)
                .expect("translate to foreign");
            let _back = translator
                .foreign_to_zetl(foreign)
                .expect("translate from foreign");
            samples.push(t0.elapsed());
        }

        let total: Duration = samples.iter().sum();
        let per_call_budget = round_trip_budget_for(match ast_type {
            AstType::ZetlExt | AstType::MdastExt => Ecosystem::Remark, // mdast → remark; zetl-ext gated by remark for headroom
            AstType::PandocExt => Ecosystem::Pandoc,
        });
        let aggregate_budget = per_call_budget * 1_000;
        assert!(
            total < aggregate_budget,
            "NFR-3302/3303: {ast_type} 1k round-trips took {total:?}, aggregate budget {aggregate_budget:?}"
        );

        let observed_p95 = p95(&samples);
        eprintln!(
            "NFR-3302/3303 {ast_type}: n=1000 p95={observed_p95:?} per-call budget {per_call_budget:?}"
        );
    }
}

// ── NFR-3304: Per-feature binary-size budget ────────────────────────────────

/// Spec-pin gate: NFR-3304's 2 MiB ceiling.
#[test]
fn nfr_3304_size_budget_pinned() {
    assert_eq!(
        PER_FEATURE_SIZE_BUDGET_BYTES,
        2 * 1024 * 1024,
        "NFR-3304: per-feature size budget drifted from 2 MiB"
    );
    assert!(size_delta_within_budget(0));
    assert!(size_delta_within_budget(PER_FEATURE_SIZE_BUDGET_BYTES));
    assert!(!size_delta_within_budget(PER_FEATURE_SIZE_BUDGET_BYTES + 1));
}

/// Strict gate: read the release binary's stripped size from
/// `target/release/zetl` (present after `cargo build --release`) and
/// emit it as telemetry. We do NOT compare against a no-feature
/// baseline here — that needs a side-by-side build the CI nightly
/// pipeline runs separately. This gate exists to catch regressions
/// when the release binary itself overshoots a sanity ceiling.
#[test]
#[ignore]
fn nfr_3304_release_binary_under_size_budget() {
    let target = release_binary_path();
    if !target.is_file() {
        eprintln!(
            "NFR-3304: skipping — release binary not present at {} (run `cargo build --release` first)",
            target.display()
        );
        return;
    }
    let size = std::fs::metadata(&target).unwrap().len();
    eprintln!(
        "NFR-3304: release binary at {} = {} bytes ({:.1} MiB)",
        target.display(),
        size,
        size as f64 / (1024.0 * 1024.0)
    );

    // Sanity ceiling — the no-feature baseline today is well under 100 MiB.
    // The strict per-feature delta gate is the bench harness's job;
    // here we just guard against a runaway binary.
    let sanity_ceiling = 200 * 1024 * 1024;
    assert!(
        size < sanity_ceiling,
        "NFR-3304: release binary is {size} bytes, over sanity ceiling {sanity_ceiling}"
    );
}

fn release_binary_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("release")
        .join("zetl")
}

// ── NFR-3305: Translation round-trip fidelity ───────────────────────────────

/// Coarse gate: the canonical-form equivalence helper agrees with
/// itself on a hand-crafted document for every ast_type. The strict
/// 10k-case sweep lives in `tests/translators/roundtrip.rs` (and is
/// surfaced as `make translator-roundtrip`); this gate covers the
/// equivalence-helper API the integration code uses.
#[test]
fn nfr_3305_round_trip_fidelity_holds_for_known_docs() {
    let doc = fixture_document();
    for ast_type in [AstType::ZetlExt, AstType::MdastExt, AstType::PandocExt] {
        assert!(
            round_trip_fidelity_holds(ast_type, &doc, &doc),
            "NFR-3305: identity must satisfy canonical-form equivalence for {ast_type}"
        );

        // A single position-only difference must canonicalise away
        // — this is the slack canonicalise() collapses on every ast_type.
        let mut shifted = doc.clone();
        shifted.position = Position::new(99, 1, 99, 99);
        assert!(
            round_trip_fidelity_holds(ast_type, &doc, &shifted),
            "NFR-3305: position-only diff must canonicalise away for {ast_type}"
        );
    }
}

#[test]
fn nfr_3305_round_trip_fidelity_corpus_pinned() {
    assert_eq!(
        ROUND_TRIP_FIDELITY_CORPUS, 10_000,
        "NFR-3305: release-gate corpus size drifted from spec (10,000)"
    );
}

// ── NFR-3306: Determinism (adapter+translator scope) ────────────────────────

/// The canonical-form normaliser MUST be idempotent — applying it
/// twice yields the same document as applying it once. This is the
/// determinism contract the adapter layer rests on; if canonicalise
/// becomes order-dependent, every cross-platform fidelity test breaks
/// silently.
#[test]
fn nfr_3306_determinism_canonicalise_is_idempotent() {
    let doc = fixture_document();
    for ast_type in [AstType::ZetlExt, AstType::MdastExt, AstType::PandocExt] {
        let once = canonicalise(ast_type, &doc);
        let twice = canonicalise(ast_type, &once);
        assert_eq!(
            once, twice,
            "NFR-3306: canonicalise({ast_type}) must be idempotent"
        );
    }
}

#[test]
fn nfr_3306_determinism_scope_pinned_to_adapter_layer() {
    assert_eq!(
        DETERMINISM_SCOPE, "adapter-and-translator-layer",
        "NFR-3306: determinism scope drifted; spec restricts to adapter+translator layer"
    );
}

// ── NFR-3307: Process-lifecycle ceilings ────────────────────────────────────

/// The lifecycle table must cover every ecosystem the registry knows
/// about. Adding an ecosystem without a memory ceiling is a
/// process-lifecycle hole; this gate catches it before a real adapter
/// goes unbounded in production.
#[test]
fn nfr_3307_process_lifecycle_table_matches_registry() {
    for entry in ECOSYSTEMS {
        let row = lifecycle_for(entry.ecosystem).unwrap_or_else(|| {
            panic!(
                "NFR-3307: registry has '{}' but lifecycle table has no row",
                entry.id
            )
        });
        assert!(
            row.max_resident_bytes > 0,
            "NFR-3307: ecosystem '{}' has zero memory ceiling — adapter would never respawn",
            entry.id
        );
    }
    assert_eq!(
        PROCESS_MEMORY_CEILINGS.len(),
        ECOSYSTEMS.len(),
        "NFR-3307: lifecycle table size differs from registry size"
    );
}

#[test]
fn nfr_3307_individual_ceilings_match_spec() {
    // Pandoc filters: ≤ 50 MiB resident per filter.
    let pandoc = lifecycle_for(Ecosystem::Pandoc).unwrap();
    assert_eq!(pandoc.max_resident_bytes, 50 * 1024 * 1024);
    assert!(pandoc.persistent_default);

    // mdBook preprocessors: one-shot per invocation by default.
    let mdbook = lifecycle_for(Ecosystem::Mdbook).unwrap();
    assert!(!mdbook.persistent_default);

    // remark Node harness: one harness per zetl process; ≤ 256 MiB ceiling.
    let remark = lifecycle_for(Ecosystem::Remark).unwrap();
    assert_eq!(remark.max_resident_bytes, 256 * 1024 * 1024);
    assert!(remark.persistent_default);
}

// ── NFR-3308: Combined-ecosystem build budget ───────────────────────────────

#[test]
fn nfr_3308_combined_multiplier_pinned_and_helper_works() {
    assert!(
        (COMBINED_BUILD_MULTIPLIER - 3.0).abs() < 1e-9,
        "NFR-3308: combined-build multiplier drifted from 3×"
    );

    let baseline = Duration::from_millis(1_000);
    // Edge: 3× is the spec's upper bound (≤), so equality passes.
    assert!(combined_within_budget(
        baseline,
        Duration::from_millis(3_000)
    ));
    // Just over: regression.
    assert!(!combined_within_budget(
        baseline,
        Duration::from_millis(3_001)
    ));
    // 2× is comfortably under.
    assert_eq!(
        combined_multiplier(baseline, Duration::from_millis(2_000)),
        2.0
    );
}

/// Coarse synthetic gate: simulate a baseline + treatment build pair
/// and assert the multiplier helper agrees with hand math. The full
/// `--no-default-features` vs all-three-ecosystems build comparison
/// (previously `--features ecosystems-v1`; the umbrella was retired
/// in SPEC-033 §12 Phase F) is a release-gate bench — it requires
/// building two binaries on the same vault — and lives in the bench
/// harness, not here.
#[test]
fn nfr_3308_synthetic_build_pair_within_budget() {
    let baseline = Duration::from_millis(2_500);
    let treatment_ok = Duration::from_millis(7_000); // 2.8×
    let treatment_bad = Duration::from_millis(7_600); // 3.04×

    assert!(combined_within_budget(baseline, treatment_ok));
    assert!(!combined_within_budget(baseline, treatment_bad));

    eprintln!(
        "NFR-3308 synthetic: baseline={baseline:?} treatment_ok={treatment_ok:?} \
         multiplier={:.3} (cap {COMBINED_BUILD_MULTIPLIER:.1}×)",
        combined_multiplier(baseline, treatment_ok)
    );
}
