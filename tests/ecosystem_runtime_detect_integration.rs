//! End-to-end coverage of SPEC-033 REQ-3313 graceful-absence behaviour.
//!
//! These tests exercise the [`ztl::ecosystems::detection`] surface from
//! outside the crate (the way `ztl build` / `ztl serve` will use it),
//! pinning the contract that:
//!
//! - probing each registered ecosystem at startup yields one
//!   [`RuntimeStatus`] entry per ecosystem and never panics, regardless
//!   of which runtimes are or aren't installed on the test host;
//! - missing runtimes degrade gracefully (no crash, log line stays a
//!   `[ztl] ecosystem ...` info line, diagnostic carries an actionable
//!   install hint);
//! - the `--ecosystem-required=<name>` CI gate (REQ-3313 final paragraph)
//!   surfaces a five-part [`HookDiagnostic`] of class
//!   [`DiagnosticClass::RuntimeAbsence`] when an explicitly-required
//!   ecosystem isn't available;
//! - the canonical log-line format matches the SPEC-033 example shape.

use std::collections::BTreeMap;

use ztl::ecosystems::detection::{
    detect_all_ecosystems, diagnostic_for, enforce_required, format_log_line, install_hint_for,
    parse_version, probe_runtime_dep, EcosystemDetectionReport,
};
use ztl::ecosystems::registry::{by_id, Ecosystem, RuntimeDep, ECOSYSTEMS};
use ztl::ecosystems::RuntimeStatus;
use ztl::hooks::diagnostic::DiagnosticClass;

// ── REQ-3313: detection must never panic ─────────────────────────────────

#[test]
fn detect_all_ecosystems_never_panics_regardless_of_host_state() {
    // The build/serve entry point invokes this at startup; CI hosts may
    // or may not have pandoc / mdbook / node installed. The contract is
    // that every registered ecosystem appears in the report exactly
    // once, with *some* RuntimeStatus.
    let report = detect_all_ecosystems();
    assert_eq!(report.entries.len(), ECOSYSTEMS.len());
    for entry in ECOSYSTEMS {
        let status = report.status_for(entry.id).unwrap_or_else(|| {
            panic!(
                "detection report missing entry for ecosystem '{}'",
                entry.id
            )
        });
        // Hygiene: the status's tag is one of four canonical values.
        assert!(
            ["available", "outdated", "missing", "probe-failed"].contains(&status.tag()),
            "ecosystem {} got unknown status tag {:?}",
            entry.id,
            status.tag()
        );
    }
}

#[test]
fn detect_all_ecosystems_log_lines_in_canonical_order() {
    // SPEC-033 §6.1: every detection line is `[ztl] ecosystem <id>: ...`
    // and the canonical ordering surfaces in the same registry order
    // (pandoc → mdbook → remark) so log-grep tooling can rely on the
    // shape.
    let report = detect_all_ecosystems();
    let text = report.format_log_lines();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), ECOSYSTEMS.len());
    for (line, entry) in lines.iter().zip(ECOSYSTEMS.iter()) {
        let prefix = format!("[ztl] ecosystem {}:", entry.id);
        assert!(
            line.starts_with(&prefix),
            "expected log line to start with {prefix:?}, got {line:?}"
        );
    }
}

// ── REQ-3313: missing runtime is a soft skip, not a crash ───────────────

#[test]
fn missing_runtime_is_classified_not_thrown() {
    // The probe primitive must never panic on a missing binary — the
    // failure surfaces as a typed `RuntimeStatus::Missing` so the
    // pipeline can disable the adapter and continue.
    let dep = RuntimeDep {
        binary: "ztl-test-this-binary-does-not-exist-anywhere",
        min_version: "0.0.0",
    };
    let status = probe_runtime_dep(&dep);
    match status {
        RuntimeStatus::Missing { binary, hint } => {
            assert_eq!(binary, "ztl-test-this-binary-does-not-exist-anywhere");
            assert!(!hint.is_empty(), "missing runtime hint must not be empty");
        }
        other => panic!("expected Missing, got {other:?}"),
    }
}

#[test]
fn missing_runtime_yields_runtime_absence_diagnostic_with_hint() {
    // Composition's hook-disabling path turns `RuntimeStatus::Missing`
    // into a `HookDiagnostic{class: RuntimeAbsence}`. The five-part
    // shape (summary / context / observed / cause / hint) is mandatory
    // per CON-3225.
    let entry = Ecosystem::Pandoc.entry();
    let status = RuntimeStatus::Missing {
        binary: "pandoc".into(),
        hint: install_hint_for("pandoc"),
    };
    let diag = diagnostic_for(entry, &status).expect("missing → Some diagnostic");
    assert_eq!(diag.class, DiagnosticClass::RuntimeAbsence);
    assert!(diag.summary.contains("pandoc"));
    assert!(!diag.context.is_empty(), "context must name the binary");
    assert!(
        !diag.observed.is_empty(),
        "observed must include probe outcome"
    );
    assert!(diag.cause.is_some(), "cause line is mandatory");
    let hint = diag.hint.as_deref().unwrap_or("");
    assert!(hint.contains("pandoc"), "hint must reference the binary");

    // Render at default verbosity and assert the wire shape includes
    // the canonical labels.
    let rendered = diag.to_string();
    assert!(rendered.starts_with("[ztl] "));
    assert!(rendered.contains("Likely cause: "));
    assert!(rendered.contains("Hint: "));
}

// ── REQ-3313: --ecosystem-required CI gate ──────────────────────────────

#[test]
fn enforce_required_passes_when_required_runtime_is_available() {
    // Build a synthetic report so the test doesn't depend on host
    // installs. `pandoc` available → `--ecosystem-required=pandoc`
    // succeeds.
    let report = synthetic_report(
        RuntimeStatus::Available {
            executable: None,
            version: "pandoc 3.1.12.1".into(),
        },
        RuntimeStatus::Missing {
            binary: "mdbook".into(),
            hint: install_hint_for("mdbook"),
        },
        RuntimeStatus::Missing {
            binary: "node".into(),
            hint: install_hint_for("node"),
        },
    );
    enforce_required(&report, &["pandoc".into()]).expect("pandoc available → gate passes");
}

#[test]
fn enforce_required_fails_with_runtime_absence_when_required_runtime_missing() {
    // The CI gate wraps the same diagnostic shape the soft-skip path
    // uses — same class, same hint, same wording — so authors don't see
    // two parallel error styles.
    let report = synthetic_report(
        RuntimeStatus::Available {
            executable: None,
            version: "pandoc 3.1".into(),
        },
        RuntimeStatus::Missing {
            binary: "mdbook".into(),
            hint: install_hint_for("mdbook"),
        },
        RuntimeStatus::Missing {
            binary: "node".into(),
            hint: install_hint_for("node"),
        },
    );
    let diag = enforce_required(&report, &["mdbook".into()])
        .expect_err("missing required ecosystem must error");
    assert_eq!(diag.class, DiagnosticClass::RuntimeAbsence);
    assert!(diag.summary.contains("mdbook"));
    let rendered = diag.to_string();
    assert!(rendered.contains("Hint: "), "CI-gate diag must keep hint");
}

#[test]
fn enforce_required_rejects_unknown_ecosystem_id() {
    let report = synthetic_report(
        RuntimeStatus::Available {
            executable: None,
            version: "pandoc 3.1".into(),
        },
        RuntimeStatus::Available {
            executable: None,
            version: "mdbook 0.4.40".into(),
        },
        RuntimeStatus::Available {
            executable: None,
            version: "v18.0.0".into(),
        },
    );
    let diag =
        enforce_required(&report, &["djot".into()]).expect_err("unknown ecosystem id must error");
    assert_eq!(diag.class, DiagnosticClass::RuntimeAbsence);
    assert!(diag.summary.contains("djot"));
}

// ── REQ-3313 example: log-line shape stability ──────────────────────────

#[test]
fn available_log_line_matches_spec_example_format() {
    // SPEC-033 REQ-3313's example text is `[ztl] ecosystem pandoc:
    // detected v3.1.12.1`. The exact substring after `:` is implementation
    // detail (we preserve the binary's own version output), but the
    // prefix shape is part of the contract.
    let entry = Ecosystem::Pandoc.entry();
    let status = RuntimeStatus::Available {
        executable: None,
        version: "v3.1.12.1".into(),
    };
    let line = format_log_line(entry, &status);
    assert_eq!(line, "[ztl] ecosystem pandoc: detected v3.1.12.1");
}

// ── parse_version covers every v1 ecosystem's real --version output ────

#[test]
fn parse_version_recognises_every_v1_runtime_format() {
    // These three lines are byte-for-byte samples from real
    // `<binary> --version` runs as captured during task development.
    // Locking them in here means a future regression in version
    // parsing surfaces with a precise failure rather than a generic
    // probe failure on the test host.
    assert_eq!(parse_version("pandoc 3.7.0.2"), Some((3, 7, 0)));
    assert_eq!(parse_version("mdbook v0.5.2"), Some((0, 5, 2)));
    assert_eq!(parse_version("v22.12.0"), Some((22, 12, 0)));
}

// ── helper ──────────────────────────────────────────────────────────────

fn synthetic_report(
    pandoc: RuntimeStatus,
    mdbook: RuntimeStatus,
    remark: RuntimeStatus,
) -> EcosystemDetectionReport {
    let mut entries: BTreeMap<&'static str, RuntimeStatus> = BTreeMap::new();
    // Look the ids up via `by_id` so the test breaks loudly if the
    // registry's canonical id strings drift.
    entries.insert(by_id("pandoc").expect("registry has pandoc").id, pandoc);
    entries.insert(by_id("mdbook").expect("registry has mdbook").id, mdbook);
    entries.insert(by_id("remark").expect("registry has remark").id, remark);
    EcosystemDetectionReport { entries }
}
