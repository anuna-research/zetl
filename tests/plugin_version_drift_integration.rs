//! SPEC-033 REQ-3314 / TEST-3314 plugin-version drift detection.
//!
//! REQ-3314 extends REQ-3313's "is the ecosystem's interpreter present?"
//! contract to "does the *plugin* on PATH match what the matrix was
//! tested against?". These tests exercise the public
//! [`ztl::ecosystems::version_drift`] surface from outside the crate
//! (the way the composition layer will use it at probe time), pinning:
//!
//! - the REQ-3314 decision tree (Exact / MinorDrift / Incompatible) for
//!   every branch, including a mocked `pandoc-crossref --version` script
//!   the probe-helper executes through the OS — TEST-3314's
//!   "passes with mocked pandoc `--version`" acceptance gate;
//! - the minor-drift log line matches REQ-3314's prose verbatim;
//! - the incompatible diagnostic carries the five-part CON-3225 shape
//!   and a `plugin_version_incompatible` summary;
//! - the remark-flavoured `package.json` fallback parses correctly;
//! - every live matrix row's `version_range` parses — i.e. the seeded
//!   ranges in `tools/ztl-ecosystem-matrix.toml` drive live drift
//!   classification without tripping the parser.

#![cfg(unix)]

use std::collections::BTreeMap;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ztl::ecosystems::version_drift::{
    classify, diagnostic_for_incompatible, format_minor_drift_log, parse_range,
    probe_node_package_version, probe_plugin_version, IncompatReason, PluginProbeError,
    PluginVersionDrift, Version,
};
use ztl::hooks::diagnostic::{DiagnosticClass, Verbosity};

/// Retry `probe_plugin_version` on the Linux `ETXTBSY` (errno 26, "Text
/// file busy") race, where a sibling test thread's fork holds an fd to
/// our just-written mock script between write and exec. Matches on the
/// stringified error detail because the production probe wraps the
/// underlying `io::Error` as `ProbeFailed { detail: format!("…: {e}") }`
/// rather than exposing the raw errno. 25 ms backoff, 3 s deadline —
/// mirrors the `spawn_or_retry` helpers in `src/hooks/persistent.rs`
/// and `tests/persistent_protocol_integration.rs`.
fn probe_with_retry(binary: &str) -> Version {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match probe_plugin_version(binary) {
            Err(PluginProbeError::ProbeFailed { detail, .. })
                if detail.contains("os error 26") && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(25));
            }
            result => return result.expect("probe succeeds"),
        }
    }
}

// ── Mocked `<plugin> --version` scripts ───────────────────────────────────

/// Write a tiny shell script to `dir` named `name` that echoes the
/// supplied `version_output` text on `--version` and returns 0. Returns
/// the absolute path to the script. The whole thing gets +x so `Command`
/// can spawn it directly.
fn write_mock_version_script(dir: &Path, name: &str, version_output: &str) -> PathBuf {
    let script_path = dir.join(name);
    let mut f = std::fs::File::create(&script_path).expect("create mock script");
    writeln!(f, "#!/bin/sh").unwrap();
    writeln!(f, "if [ \"$1\" = \"--version\" ]; then").unwrap();
    // Single-quoted heredoc to keep the output literal — no shell
    // expansion of the mocked version text.
    writeln!(f, "  cat <<'EOF'").unwrap();
    writeln!(f, "{version_output}").unwrap();
    writeln!(f, "EOF").unwrap();
    writeln!(f, "  exit 0").unwrap();
    writeln!(f, "fi").unwrap();
    writeln!(f, "exit 1").unwrap();
    drop(f);
    let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&script_path, perms).unwrap();
    script_path
}

// ── Decision-tree matrix against mocked pandoc-crossref ──────────────────

#[test]
fn test_3314_exact_match_via_mocked_probe_is_silent() {
    // Matrix range `>=0.3.14 <0.4`; probe reports the exact lower bound.
    let tmp = tempfile::tempdir().unwrap();
    let path = write_mock_version_script(tmp.path(), "pandoc-crossref", "pandoc-crossref 0.3.14");
    let observed = probe_with_retry(path.to_str().unwrap());
    assert_eq!(observed, Version(0, 3, 14));

    let drift = classify(">=0.3.14 <0.4", observed).expect("classify");
    assert!(drift.is_exact(), "exact match expected, got {drift:?}");
}

#[test]
fn test_3314_minor_drift_via_mocked_probe_warns_and_continues() {
    // pandoc-crossref 0.3.16 → newer patch within the tested range.
    let tmp = tempfile::tempdir().unwrap();
    let path = write_mock_version_script(tmp.path(), "pandoc-crossref", "pandoc-crossref 0.3.16");
    let observed = probe_with_retry(path.to_str().unwrap());
    assert_eq!(observed, Version(0, 3, 16));

    let drift = classify(">=0.3.14 <0.4", observed).expect("classify");
    match &drift {
        PluginVersionDrift::MinorDrift {
            observed: o,
            tested: t,
            ..
        } => {
            assert_eq!(*o, Version(0, 3, 16));
            assert_eq!(*t, Version(0, 3, 14));
        }
        other => panic!("expected MinorDrift, got {other:?}"),
    }

    // Log line matches REQ-3314 prose verbatim.
    let log = format_minor_drift_log(
        "pandoc",
        "pandoc-crossref",
        Version(0, 3, 16),
        Version(0, 3, 14),
    );
    assert_eq!(
        log,
        "[ztl] ecosystem pandoc: pandoc-crossref v0.3.16 is newer than last-tested v0.3.14; proceeding"
    );
}

#[test]
fn test_3314_incompatible_major_mismatch_via_mocked_probe_disables_hook() {
    // pandoc-crossref 1.0.0 — major bump from tested 0.3.14.
    let tmp = tempfile::tempdir().unwrap();
    let path = write_mock_version_script(tmp.path(), "pandoc-crossref", "pandoc-crossref 1.0.0");
    let observed = probe_with_retry(path.to_str().unwrap());
    assert_eq!(observed, Version(1, 0, 0));

    let drift = classify(">=0.3.14 <0.4", observed).expect("classify");
    assert!(
        drift.is_incompatible(),
        "incompatible expected, got {drift:?}"
    );
    match drift {
        PluginVersionDrift::Incompatible { reason, .. } => {
            assert_eq!(reason, IncompatReason::MajorMismatch);
        }
        other => panic!("unexpected classification {other:?}"),
    }
}

#[test]
fn test_3314_incompatible_below_range_via_mocked_probe() {
    // pandoc-crossref 0.3.10 — older than the tested lower bound 0.3.14
    // (same major 0 but patch below).
    let tmp = tempfile::tempdir().unwrap();
    let path = write_mock_version_script(tmp.path(), "pandoc-crossref", "pandoc-crossref 0.3.10");
    let observed = probe_with_retry(path.to_str().unwrap());
    assert_eq!(observed, Version(0, 3, 10));

    let drift = classify(">=0.3.14 <0.4", observed).expect("classify");
    match drift {
        PluginVersionDrift::Incompatible { reason, .. } => {
            assert_eq!(reason, IncompatReason::BelowRange);
        }
        other => panic!("expected BelowRange incompatible, got {other:?}"),
    }
}

#[test]
fn test_3314_incompatible_above_range_via_mocked_probe() {
    // Same major (0), observed 0.4.0 hits the exclusive upper bound.
    let tmp = tempfile::tempdir().unwrap();
    let path = write_mock_version_script(tmp.path(), "pandoc-crossref", "pandoc-crossref 0.4.0");
    let observed = probe_with_retry(path.to_str().unwrap());
    assert_eq!(observed, Version(0, 4, 0));

    let drift = classify(">=0.3.14 <0.4", observed).expect("classify");
    match drift {
        PluginVersionDrift::Incompatible { reason, .. } => {
            assert_eq!(reason, IncompatReason::AboveRange);
        }
        other => panic!("expected AboveRange incompatible, got {other:?}"),
    }
}

// ── CON-3225 five-part shape on incompatibility diagnostic ────────────────

#[test]
fn test_3314_incompatible_diagnostic_renders_five_parts() {
    let diag = diagnostic_for_incompatible(
        "pandoc",
        "pandoc-crossref",
        Version(1, 0, 0),
        Version(0, 3, 14),
        ">=0.3.14 <0.4",
        IncompatReason::MajorMismatch,
    );
    // Typed class + machine-grep summary tag.
    assert_eq!(diag.class, DiagnosticClass::RuntimeAbsence);
    assert!(
        diag.summary.contains("plugin_version_incompatible"),
        "summary should lead with the REQ-3314 typed error name; got {:?}",
        diag.summary
    );

    // Render at default verbosity and assert every CON-3225 part is present.
    let rendered = diag.render(Verbosity::Default);
    let lines: Vec<&str> = rendered.lines().collect();
    // (1) Summary with `[ztl] ` prefix and `:` terminator.
    assert!(lines.first().unwrap().starts_with("[ztl] "));
    assert!(lines.first().unwrap().ends_with(":"));
    // (2) Context lines (at least the version_range pointer).
    assert!(rendered.contains("version_range"));
    assert!(rendered.contains("last-tested v0.3.14"));
    // (3) Observed (concrete found-version line).
    assert!(rendered.contains("found v1.0.0"));
    // (4) Likely cause: exactly one.
    assert!(rendered.contains("Likely cause: "));
    // (5) Hint: actionable, naming both the range and the check cmd.
    assert!(rendered.contains("Hint: "));
    assert!(rendered.contains(">=0.3.14 <0.4"));
    assert!(rendered.contains("ztl ecosystem check"));
}

// ── Probe failure modes ────────────────────────────────────────────────────

#[test]
fn test_3314_probe_missing_binary_surfaces_not_found() {
    let err = probe_plugin_version("definitely-no-plugin-ztl-test-3314").unwrap_err();
    match err {
        PluginProbeError::NotFound { binary } => {
            assert_eq!(binary, "definitely-no-plugin-ztl-test-3314");
        }
        other => panic!("expected NotFound, got {other:?}"),
    }
}

#[test]
fn test_3314_remark_package_json_probe_round_trips() {
    // remark plugins are probed via package.json#version rather than
    // a CLI --version — exercise that branch end-to-end.
    let tmp = tempfile::tempdir().unwrap();
    let pkg_dir = tmp.path().join("remark-gfm");
    std::fs::create_dir_all(&pkg_dir).unwrap();
    std::fs::write(
        pkg_dir.join("package.json"),
        r#"{"name": "remark-gfm", "version": "4.0.1"}"#,
    )
    .unwrap();
    let observed = probe_node_package_version(tmp.path(), "remark-gfm").expect("probe succeeds");
    assert_eq!(observed, Version(4, 0, 1));

    // Matrix seeds `>=4.0 <5` for remark-gfm. 4.0.1 is a minor drift.
    let drift = classify(">=4.0 <5", observed).expect("classify");
    match drift {
        PluginVersionDrift::MinorDrift { .. } => {}
        other => panic!("expected MinorDrift for 4.0.1 against >=4.0 <5, got {other:?}"),
    }
}

// ── Live matrix parse gate ────────────────────────────────────────────────

#[test]
fn test_3314_every_live_matrix_version_range_parses_and_classifies_its_own_lower_bound_as_exact() {
    // Every `version_range` in tools/ztl-ecosystem-matrix.toml must
    // parse under the REQ-3314 parser. As a sanity check, feed the
    // parser's `tested_version()` back through `classify` — it should
    // always come out as Exact (fixed-point property of the tested
    // anchor).
    let matrix_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tools/ztl-ecosystem-matrix.toml");
    let body = std::fs::read_to_string(&matrix_path).expect("read matrix");
    let parsed: toml::Value = toml::from_str(&body).expect("parse matrix toml");
    let eco_table = parsed
        .get("ecosystem")
        .and_then(|v| v.as_table())
        .expect("ecosystem table");

    let mut seen = 0usize;
    let mut per_eco: BTreeMap<String, usize> = BTreeMap::new();
    for (eco_id, eco_val) in eco_table {
        let rows = eco_val
            .get("plugins")
            .and_then(|v| v.as_array())
            .expect("plugins array");
        for plug in rows {
            let t = plug.as_table().unwrap();
            let name = t.get("name").and_then(|v| v.as_str()).unwrap();
            let range_str = t.get("version_range").and_then(|v| v.as_str()).unwrap();
            let range = parse_range(range_str).unwrap_or_else(|e| {
                panic!(
                    "matrix row {eco_id}/{name}: version_range {range_str:?} failed to parse: {e}"
                )
            });
            let tested = range
                .tested_version()
                .unwrap_or_else(|| panic!("matrix row {eco_id}/{name}: no tested anchor"));
            let drift = classify(range_str, tested)
                .unwrap_or_else(|e| panic!("classify failed for {eco_id}/{name}: {e}"));
            assert!(
                drift.is_exact(),
                "tested anchor should classify as Exact against its own range; \
                 row {eco_id}/{name} range {range_str:?} tested {tested:?} → {drift:?}"
            );
            seen += 1;
            *per_eco.entry(eco_id.clone()).or_default() += 1;
        }
    }
    assert!(seen > 0, "matrix should have at least one plugin row");
    for (eco, n) in &per_eco {
        println!("drift-matrix-parse: {n} rows for ecosystem {eco}");
    }
}
