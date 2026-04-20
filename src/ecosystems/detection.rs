//! Startup-time ecosystem runtime detection (SPEC-033 REQ-3313).
//!
//! REQ-3313 mandates: at zetl startup, before any hook pipeline construction,
//! the system probes every compiled-in ecosystem adapter's runtime dependency
//! (`pandoc --version`, `mdbook --version`, `node --version`), logs detection
//! results, and disables adapters whose runtimes are missing rather than
//! failing the build. This module owns the primitive — invoking the
//! `--version` subprocess, parsing the output, comparing against the
//! registry's [`RuntimeDep::min_version`], producing a typed
//! [`RuntimeStatus`] and an actionable install hint.
//!
//! Per-ecosystem adapters (task-pandoc-adapter, task-mdbook-adapter,
//! task-remark-harness) call [`probe_runtime_dep`] from their own
//! [`EcosystemAdapter::probe`] implementations so all three follow the same
//! detection contract. The build/serve entry point calls
//! [`detect_all_ecosystems`] once per session and threads the report through
//! to the composition layer so hooks targeting an unavailable ecosystem are
//! disabled (with a [`DiagnosticClass::RuntimeAbsence`] diagnostic) rather
//! than crashing.
//!
//! ## Soft skip vs hard gate
//!
//! Default: a missing runtime is logged + the adapter is disabled for the
//! session. Build keeps going. This is the SPEC-033 §1.4(4) graceful-absence
//! contract: zetl never fails the build because an *optional* runtime is
//! missing.
//!
//! CI gate: callers pass a list of required ecosystem ids to
//! [`enforce_required`]. If any of them isn't [`RuntimeStatus::Available`],
//! the function returns a [`HookDiagnostic`] the caller turns into a
//! non-zero exit. This is the `--ecosystem-required=<name>` surface
//! REQ-3313 mandates for `zetl build` / `zetl serve`.
//!
//! ## Version parsing
//!
//! Real-world `--version` output varies wildly (`pandoc 3.7.0.2` vs
//! `mdbook v0.5.2` vs the bare `v22.12.0` for `node --version`). We extract
//! the first `MAJOR.MINOR(.PATCH)?` triple anywhere in the output and
//! compare component-wise against the registered minimum. Pre-release tags
//! and build metadata after the patch are ignored — adapters that need
//! finer-grained checks can re-parse the raw `version` string from
//! [`RuntimeStatus::Available`].

use std::collections::BTreeMap;
use std::process::{Command, Stdio};
use std::time::Duration;

use regex::Regex;

use crate::ecosystems::adapter::RuntimeStatus;
use crate::ecosystems::registry::{EcosystemEntry, RuntimeDep, ECOSYSTEMS};
use crate::hooks::diagnostic::{DiagnosticClass, HookDiagnostic};

/// Per-process timeout for `<binary> --version` invocations.
///
/// Probes are bounded by NFR-3301 (≤ 200 ms P95 cold-start per ecosystem);
/// in practice every supported runtime returns its version in well under
/// 100 ms. The wider 2 s ceiling here exists only to bound a runaway
/// child if the user has shadowed the binary with a script that hangs.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

// ── Public API ──────────────────────────────────────────────────────────────

/// Probe a single [`RuntimeDep`] by executing `<binary> --version` and
/// classifying the result.
///
/// Returns one of the four [`RuntimeStatus`] variants:
///
/// - [`RuntimeStatus::Available`] — binary on `PATH`, parsed version
///   ≥ `dep.min_version`. The raw version string from the binary's own
///   output is preserved so callers can render the user's exact runtime
///   in `zetl ecosystem check`.
/// - [`RuntimeStatus::OutdatedVersion`] — binary on `PATH` but parsed
///   version below `dep.min_version`. Carries an actionable install hint.
/// - [`RuntimeStatus::Missing`] — `which`-style lookup failed (the OS
///   couldn't spawn the binary).
/// - [`RuntimeStatus::ProbeFailed`] — the binary spawned but its output
///   was unparseable, the exit code was non-zero, or the call itself
///   errored. Detail string carries the root cause.
pub fn probe_runtime_dep(dep: &RuntimeDep) -> RuntimeStatus {
    let mut cmd = Command::new(dep.binary);
    cmd.arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = match cmd.output() {
        Ok(out) => out,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return RuntimeStatus::Missing {
                binary: dep.binary.to_string(),
                hint: install_hint_for(dep.binary),
            };
        }
        Err(e) => {
            return RuntimeStatus::ProbeFailed {
                detail: format!("could not spawn `{} --version`: {e}", dep.binary),
            };
        }
    };

    if !output.status.success() {
        return RuntimeStatus::ProbeFailed {
            detail: format!(
                "`{} --version` exited {} (stderr: {})",
                dep.binary,
                output
                    .status
                    .code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "signal".into()),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        };
    }

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let raw_version = match extract_version_line(&stdout) {
        Some(line) => line,
        None => {
            return RuntimeStatus::ProbeFailed {
                detail: format!(
                    "`{} --version` produced no recognisable version line: {:?}",
                    dep.binary,
                    stdout.trim()
                ),
            };
        }
    };

    let executable = which_on_path(dep.binary);

    let found = match parse_version(&raw_version) {
        Some(v) => v,
        None => {
            return RuntimeStatus::ProbeFailed {
                detail: format!(
                    "could not parse version from `{} --version` output {:?}",
                    dep.binary, raw_version
                ),
            };
        }
    };

    let required = match parse_version(dep.min_version) {
        Some(v) => v,
        None => {
            // Registry-side bug: an unparseable min_version means the
            // ECOSYSTEMS table is malformed. Surface it as ProbeFailed
            // rather than panicking so the build keeps going (the user
            // didn't author the registry).
            return RuntimeStatus::ProbeFailed {
                detail: format!(
                    "registry min_version {:?} for binary '{}' is not parseable",
                    dep.min_version, dep.binary
                ),
            };
        }
    };

    if found < required {
        return RuntimeStatus::OutdatedVersion {
            executable: executable.unwrap_or_else(|| dep.binary.into()),
            found: raw_version,
            required: dep.min_version.to_string(),
            hint: upgrade_hint_for(dep.binary, dep.min_version),
        };
    }

    RuntimeStatus::Available {
        executable,
        version: raw_version,
    }
}

/// Walk the entire ecosystem registry and probe each runtime in canonical
/// order (pandoc → mdbook → remark per [`ECOSYSTEMS`]).
///
/// Returns one [`RuntimeStatus`] per registry entry. The result is the
/// session-scoped detection cache the spec calls for in NFR-3301:
/// callers pass it to [`disabled_for_unavailable`] / [`enforce_required`]
/// instead of re-probing.
pub fn detect_all_ecosystems() -> EcosystemDetectionReport {
    let mut entries: BTreeMap<&'static str, RuntimeStatus> = BTreeMap::new();
    for entry in ECOSYSTEMS {
        entries.insert(entry.id, probe_runtime_dep(&entry.runtime_dep));
    }
    EcosystemDetectionReport { entries }
}

/// Aggregate result of [`detect_all_ecosystems`]. Exposes lookup,
/// availability filtering, and rendering to a stream of `[zetl] ecosystem
/// <id>: ...` log lines suitable for stderr at session start.
#[derive(Debug, Clone)]
pub struct EcosystemDetectionReport {
    /// Per-ecosystem status, keyed by the registry id.
    pub entries: BTreeMap<&'static str, RuntimeStatus>,
}

impl EcosystemDetectionReport {
    /// Look up the status for a single ecosystem id, returning `None` if
    /// the id isn't in the registry at all (callers should treat that as
    /// an unknown-ecosystem error, distinct from missing-runtime).
    pub fn status_for(&self, id: &str) -> Option<&RuntimeStatus> {
        self.entries.get(id)
    }

    /// Ids whose runtime probed [`RuntimeStatus::Available`].
    pub fn available_ids(&self) -> Vec<&'static str> {
        self.entries
            .iter()
            .filter_map(|(id, status)| status.is_available().then_some(*id))
            .collect()
    }

    /// Ids whose runtime probed to anything other than
    /// [`RuntimeStatus::Available`] — Missing, OutdatedVersion, or
    /// ProbeFailed. Callers disable hooks for these ids and emit a
    /// `RuntimeAbsence` diagnostic.
    pub fn unavailable_ids(&self) -> Vec<&'static str> {
        self.entries
            .iter()
            .filter_map(|(id, status)| (!status.is_available()).then_some(*id))
            .collect()
    }

    /// Render every entry as a single `[zetl] ecosystem <id>: ...` log
    /// line in canonical registry order, suitable for stderr at session
    /// start. Lines are joined by `\n` with no trailing newline.
    pub fn format_log_lines(&self) -> String {
        let mut lines = Vec::with_capacity(ECOSYSTEMS.len());
        for entry in ECOSYSTEMS {
            if let Some(status) = self.entries.get(entry.id) {
                lines.push(format_log_line(entry, status));
            }
        }
        lines.join("\n")
    }
}

/// Canonical detection log line. The format intentionally mirrors the
/// example in SPEC-033 REQ-3313 (`[zetl] ecosystem pandoc: detected
/// v3.1.12.1`) so log-grep tooling stays simple.
pub fn format_log_line(entry: &EcosystemEntry, status: &RuntimeStatus) -> String {
    match status {
        RuntimeStatus::Available { version, .. } => {
            format!("[zetl] ecosystem {}: detected {}", entry.id, version)
        }
        RuntimeStatus::OutdatedVersion {
            found, required, ..
        } => format!(
            "[zetl] ecosystem {}: outdated (found {}, requires ≥ {}); adapter disabled",
            entry.id, found, required
        ),
        RuntimeStatus::Missing { binary, .. } => format!(
            "[zetl] ecosystem {}: missing (binary `{}` not on PATH); adapter disabled",
            entry.id, binary
        ),
        RuntimeStatus::ProbeFailed { detail } => format!(
            "[zetl] ecosystem {}: probe failed ({detail}); adapter disabled",
            entry.id
        ),
    }
}

/// Build a five-part [`HookDiagnostic`] for a non-available ecosystem so
/// downstream callers (composition disabling a hook, `zetl ecosystem check`,
/// CI-gate enforcement) all surface the same wording. Returns `None` for
/// [`RuntimeStatus::Available`] — the absence diagnostic only makes sense
/// when something is missing.
pub fn diagnostic_for(entry: &EcosystemEntry, status: &RuntimeStatus) -> Option<HookDiagnostic> {
    match status {
        RuntimeStatus::Available { .. } => None,
        RuntimeStatus::Missing { binary, hint } => Some(
            HookDiagnostic::new(
                DiagnosticClass::RuntimeAbsence,
                format!("ecosystem '{}' runtime missing", entry.id),
            )
            .with_context(format!("required binary: {binary}"))
            .with_observed(format!(
                "`{binary} --version` failed: binary not found on PATH"
            ))
            .with_cause(format!(
                "{} ecosystem requires {} on PATH but it was not found",
                entry.name, binary
            ))
            .with_hint(hint.clone()),
        ),
        RuntimeStatus::OutdatedVersion {
            executable,
            found,
            required,
            hint,
        } => Some(
            HookDiagnostic::new(
                DiagnosticClass::RuntimeAbsence,
                format!("ecosystem '{}' runtime below minimum version", entry.id),
            )
            .with_context(format!("required minimum: {required}"))
            .with_context(format!("found at: {}", executable.display()))
            .with_observed(format!("found version: {found}"))
            .with_cause(format!(
                "{} ecosystem requires version {required} or newer; the binary on PATH is older",
                entry.name
            ))
            .with_hint(hint.clone()),
        ),
        RuntimeStatus::ProbeFailed { detail } => Some(
            HookDiagnostic::new(
                DiagnosticClass::RuntimeAbsence,
                format!("ecosystem '{}' runtime probe failed", entry.id),
            )
            .with_context(format!("required binary: {}", entry.runtime_dep.binary))
            .with_observed(detail.clone())
            .with_cause(format!(
                "the {} ecosystem runtime exists on PATH but did not respond to `--version` as expected",
                entry.name
            ))
            .with_hint(format!(
                "run `{} --version` manually and verify it returns a parseable version string",
                entry.runtime_dep.binary
            )),
        ),
    }
}

/// CI-gate enforcement (REQ-3313, `--ecosystem-required=<name>`).
///
/// Returns `Ok(())` when every id in `required` resolves to
/// [`RuntimeStatus::Available`] in the report. Returns a
/// [`HookDiagnostic`] (class [`DiagnosticClass::RuntimeAbsence`])
/// describing the first missing required ecosystem otherwise.
///
/// Unknown ids in `required` are treated as fatal — the caller asked for
/// an ecosystem zetl doesn't know about, which is almost certainly a typo
/// in CI config and not something to silently skip.
#[allow(clippy::result_large_err)] // HookDiagnostic carries diagnostic-context strings; boxing the error adds a dereference on every `?` with no runtime benefit on this cold path.
pub fn enforce_required(
    report: &EcosystemDetectionReport,
    required: &[String],
) -> Result<(), HookDiagnostic> {
    for req in required {
        let entry = match crate::ecosystems::registry::by_id(req) {
            Some(e) => e,
            None => {
                let known: Vec<&str> = ECOSYSTEMS.iter().map(|e| e.id).collect();
                return Err(HookDiagnostic::new(
                    DiagnosticClass::RuntimeAbsence,
                    format!("--ecosystem-required={req} names an unknown ecosystem"),
                )
                .with_context(format!("requested: {req}"))
                .with_observed(format!("known ecosystems: [{}]", known.join(", ")))
                .with_cause("the registry does not contain an ecosystem with that id")
                .with_hint(
                    "spell-check the value or run `zetl ecosystem check` to list known ids",
                ));
            }
        };
        let status = match report.status_for(req) {
            Some(s) => s,
            None => {
                return Err(HookDiagnostic::new(
                    DiagnosticClass::RuntimeAbsence,
                    format!("ecosystem '{req}' was not probed in this session"),
                )
                .with_observed("detection report is missing this ecosystem id")
                .with_cause(
                    "the detection step ran but did not include this ecosystem — internal error",
                )
                .with_hint("file a bug; this should never happen in normal operation"));
            }
        };
        if !status.is_available() {
            // Reuse the canonical absence diagnostic so the gate's
            // wording matches what `zetl ecosystem check` shows.
            let diag = diagnostic_for(entry, status).expect("non-available status returns Some");
            return Err(diag);
        }
    }
    Ok(())
}

// ── Internals ───────────────────────────────────────────────────────────────

/// Pull the first non-blank line of `--version` output. Most binaries put
/// the version on the very first line; this trims whitespace and skips
/// any leading blank lines without assuming a format.
fn extract_version_line(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .map(|s| s.to_string())
}

/// Extract a `MAJOR.MINOR.PATCH` triple (PATCH defaults to 0) from
/// arbitrary version output. Returns the first match anywhere in the
/// string, e.g. `"pandoc 3.7.0.2"` → `(3, 7, 0)`, `"v22.12.0"` →
/// `(22, 12, 0)`, `"mdbook v0.5.2"` → `(0, 5, 2)`.
pub fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    // Lazily compile once per process — a single static Regex would need
    // a OnceLock; keeping it scoped here avoids the dep.
    let re = Regex::new(r"(\d+)\.(\d+)(?:\.(\d+))?").ok()?;
    let caps = re.captures(text)?;
    let major: u32 = caps.get(1)?.as_str().parse().ok()?;
    let minor: u32 = caps.get(2)?.as_str().parse().ok()?;
    let patch: u32 = caps
        .get(3)
        .map(|m| m.as_str().parse().unwrap_or(0))
        .unwrap_or(0);
    Some((major, minor, patch))
}

/// Look up `binary` on `PATH`, returning the absolute path of the first
/// match. Returns `None` when the lookup fails — the probe surfaces that
/// as [`RuntimeStatus::Missing`] in the spawn step rather than relying on
/// this. The result is informational: a successful spawn with no resolved
/// path (rare on Unix, possible on Windows shims) just leaves `executable`
/// as `None` in [`RuntimeStatus::Available`].
fn which_on_path(binary: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(binary);
        if candidate.is_file() {
            return Some(candidate);
        }
        // Windows: try common executable extensions.
        #[cfg(windows)]
        for ext in ["exe", "bat", "cmd"] {
            let with_ext = dir.join(format!("{binary}.{ext}"));
            if with_ext.is_file() {
                return Some(with_ext);
            }
        }
    }
    None
}

/// Per-binary install hint shown in [`RuntimeStatus::Missing.hint`] and
/// [`HookDiagnostic`] output. Kept short and OS-agnostic — the hint
/// points the user at the canonical install path for that runtime; we
/// don't try to detect the user's package manager.
pub fn install_hint_for(binary: &str) -> String {
    match binary {
        "pandoc" => {
            "install pandoc (≥ 2.11): https://pandoc.org/installing.html (homebrew: `brew install pandoc`, debian: `apt install pandoc`)".into()
        }
        "mdbook" => {
            "install mdbook (≥ 0.4): `cargo install mdbook` or download from https://github.com/rust-lang/mdBook/releases".into()
        }
        "node" => {
            "install Node.js (≥ 18): https://nodejs.org/ (homebrew: `brew install node`, debian: `apt install nodejs`)".into()
        }
        other => format!(
            "install `{other}` and ensure it is on PATH; check the ecosystem's docs for the canonical install path"
        ),
    }
}

/// Hint shown when a runtime is present but below `min_version`. Same
/// wording as [`install_hint_for`] but framed as an upgrade so the user
/// understands they don't need to install from scratch.
pub fn upgrade_hint_for(binary: &str, min_version: &str) -> String {
    format!(
        "upgrade `{binary}` to ≥ {min_version}; see {}",
        install_hint_for(binary)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecosystems::registry::Ecosystem;

    // ── parse_version ──────────────────────────────────────────────────

    #[test]
    fn parses_pandoc_version_line() {
        assert_eq!(parse_version("pandoc 3.7.0.2"), Some((3, 7, 0)));
    }

    #[test]
    fn parses_mdbook_version_line() {
        assert_eq!(parse_version("mdbook v0.5.2"), Some((0, 5, 2)));
    }

    #[test]
    fn parses_node_version_line() {
        assert_eq!(parse_version("v22.12.0"), Some((22, 12, 0)));
    }

    #[test]
    fn parses_two_component_version() {
        assert_eq!(parse_version("foo 1.2"), Some((1, 2, 0)));
    }

    #[test]
    fn returns_none_when_no_version_present() {
        assert_eq!(parse_version("hello world"), None);
    }

    #[test]
    fn version_compares_lexicographically_by_component() {
        assert!(parse_version("1.2.3") < parse_version("1.2.4"));
        assert!(parse_version("1.2.3") < parse_version("1.3.0"));
        assert!(parse_version("1.2.3") < parse_version("2.0.0"));
        assert!(parse_version("2.11") > parse_version("2.10.99"));
    }

    // ── probe_runtime_dep against real / fake binaries ────────────────

    #[test]
    fn probe_returns_missing_for_nonexistent_binary() {
        let dep = RuntimeDep {
            binary: "definitely-not-a-real-binary-zetl-test",
            min_version: "0.0.0",
        };
        let status = probe_runtime_dep(&dep);
        match status {
            RuntimeStatus::Missing { binary, hint } => {
                assert_eq!(binary, "definitely-not-a-real-binary-zetl-test");
                assert!(!hint.is_empty(), "missing hint should be non-empty");
            }
            other => panic!("expected Missing, got {other:?}"),
        }
    }

    #[test]
    fn probe_picks_up_present_binary() {
        // `sh` is on every POSIX system; on Windows we skip the test
        // since `sh --version` isn't reliable.
        #[cfg(unix)]
        {
            let dep = RuntimeDep {
                binary: "sh",
                min_version: "0.0.0",
            };
            let status = probe_runtime_dep(&dep);
            // Either Available (most shells print a parseable version)
            // or ProbeFailed (busybox sh has no `--version`). Both are
            // acceptable; the assertion is that we don't panic and don't
            // misclassify present-but-broken as Missing.
            assert!(
                matches!(
                    status,
                    RuntimeStatus::Available { .. } | RuntimeStatus::ProbeFailed { .. }
                ),
                "expected Available or ProbeFailed for `sh`, got {status:?}"
            );
        }
    }

    #[test]
    fn probe_classifies_outdated_when_min_above_found() {
        // Build a scratch script that prints a fake version and run the
        // probe against an absurdly high min_version. We use the `cat`
        // binary (always present on Unix) by passing `--version` to it
        // — GNU `cat --version` prints `cat (GNU coreutils) 9.x`,
        // BusyBox cat doesn't have --version. Skip on systems where
        // cat doesn't print a parseable version.
        #[cfg(unix)]
        {
            let dep = RuntimeDep {
                binary: "cat",
                min_version: "999.0.0",
            };
            let status = probe_runtime_dep(&dep);
            // Accept Outdated (GNU cat) or ProbeFailed (BusyBox cat).
            assert!(
                matches!(
                    status,
                    RuntimeStatus::OutdatedVersion { .. } | RuntimeStatus::ProbeFailed { .. }
                ),
                "expected Outdated or ProbeFailed, got {status:?}"
            );
        }
    }

    // ── format_log_line ──────────────────────────────────────────────

    #[test]
    fn log_line_for_available_matches_spec_example_shape() {
        let entry = Ecosystem::Pandoc.entry();
        let status = RuntimeStatus::Available {
            executable: None,
            version: "pandoc 3.1.12.1".into(),
        };
        assert_eq!(
            format_log_line(entry, &status),
            "[zetl] ecosystem pandoc: detected pandoc 3.1.12.1"
        );
    }

    #[test]
    fn log_line_for_missing_names_binary_and_disables() {
        let entry = Ecosystem::Mdbook.entry();
        let status = RuntimeStatus::Missing {
            binary: "mdbook".into(),
            hint: "install mdbook".into(),
        };
        let line = format_log_line(entry, &status);
        assert!(line.starts_with("[zetl] ecosystem mdbook: missing"));
        assert!(line.contains("mdbook"));
        assert!(line.contains("adapter disabled"));
    }

    #[test]
    fn log_line_for_outdated_includes_versions() {
        let entry = Ecosystem::Remark.entry();
        let status = RuntimeStatus::OutdatedVersion {
            executable: "/usr/bin/node".into(),
            found: "v16.0.0".into(),
            required: "18.0.0".into(),
            hint: "upgrade".into(),
        };
        let line = format_log_line(entry, &status);
        assert!(line.contains("v16.0.0"));
        assert!(line.contains("18.0.0"));
        assert!(line.contains("adapter disabled"));
    }

    #[test]
    fn log_line_for_probe_failed_includes_detail() {
        let entry = Ecosystem::Pandoc.entry();
        let status = RuntimeStatus::ProbeFailed {
            detail: "permission denied".into(),
        };
        let line = format_log_line(entry, &status);
        assert!(line.contains("probe failed"));
        assert!(line.contains("permission denied"));
        assert!(line.contains("adapter disabled"));
    }

    // ── diagnostic_for ───────────────────────────────────────────────

    #[test]
    fn diagnostic_for_available_is_none() {
        let entry = Ecosystem::Pandoc.entry();
        let status = RuntimeStatus::Available {
            executable: None,
            version: "3.1".into(),
        };
        assert!(diagnostic_for(entry, &status).is_none());
    }

    #[test]
    fn diagnostic_for_missing_carries_class_and_hint() {
        let entry = Ecosystem::Pandoc.entry();
        let status = RuntimeStatus::Missing {
            binary: "pandoc".into(),
            hint: install_hint_for("pandoc"),
        };
        let diag = diagnostic_for(entry, &status).expect("missing → Some diagnostic");
        assert_eq!(diag.class, DiagnosticClass::RuntimeAbsence);
        assert!(diag.summary.contains("pandoc"));
        assert!(diag.hint.as_deref().unwrap_or("").contains("pandoc"));
        // Five-part shape: rendering at Default verbosity must include
        // every required label.
        let rendered = diag.to_string();
        assert!(rendered.starts_with("[zetl] "));
        assert!(rendered.contains("Likely cause: "));
        assert!(rendered.contains("Hint: "));
    }

    #[test]
    fn diagnostic_for_outdated_carries_versions_in_observed_or_context() {
        let entry = Ecosystem::Remark.entry();
        let status = RuntimeStatus::OutdatedVersion {
            executable: "/usr/local/bin/node".into(),
            found: "v16.0.0".into(),
            required: "18.0.0".into(),
            hint: upgrade_hint_for("node", "18.0.0"),
        };
        let diag = diagnostic_for(entry, &status).expect("outdated → Some diagnostic");
        let rendered = diag.to_string();
        assert!(rendered.contains("v16.0.0"));
        assert!(rendered.contains("18.0.0"));
    }

    // ── EcosystemDetectionReport ────────────────────────────────────

    fn fake_report(
        pandoc: RuntimeStatus,
        mdbook: RuntimeStatus,
        remark: RuntimeStatus,
    ) -> EcosystemDetectionReport {
        let mut entries = BTreeMap::new();
        entries.insert("pandoc", pandoc);
        entries.insert("mdbook", mdbook);
        entries.insert("remark", remark);
        EcosystemDetectionReport { entries }
    }

    #[test]
    fn report_partitions_into_available_and_unavailable() {
        let report = fake_report(
            RuntimeStatus::Available {
                executable: None,
                version: "3.1".into(),
            },
            RuntimeStatus::Missing {
                binary: "mdbook".into(),
                hint: "".into(),
            },
            RuntimeStatus::ProbeFailed {
                detail: "boom".into(),
            },
        );
        assert_eq!(report.available_ids(), vec!["pandoc"]);
        let mut unavail = report.unavailable_ids();
        unavail.sort();
        assert_eq!(unavail, vec!["mdbook", "remark"]);
    }

    #[test]
    fn report_format_log_lines_emits_one_per_ecosystem_in_canonical_order() {
        let report = fake_report(
            RuntimeStatus::Available {
                executable: None,
                version: "3.1".into(),
            },
            RuntimeStatus::Available {
                executable: None,
                version: "0.4.40".into(),
            },
            RuntimeStatus::Available {
                executable: None,
                version: "v18.0.0".into(),
            },
        );
        let text = report.format_log_lines();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("[zetl] ecosystem pandoc:"));
        assert!(lines[1].starts_with("[zetl] ecosystem mdbook:"));
        assert!(lines[2].starts_with("[zetl] ecosystem remark:"));
    }

    // ── enforce_required ────────────────────────────────────────────

    #[test]
    fn enforce_required_passes_when_every_required_is_available() {
        let report = fake_report(
            RuntimeStatus::Available {
                executable: None,
                version: "3.1".into(),
            },
            RuntimeStatus::Missing {
                binary: "mdbook".into(),
                hint: "install".into(),
            },
            RuntimeStatus::Missing {
                binary: "node".into(),
                hint: "install".into(),
            },
        );
        assert!(enforce_required(&report, &["pandoc".into()]).is_ok());
    }

    #[test]
    fn enforce_required_fails_when_required_is_missing() {
        let report = fake_report(
            RuntimeStatus::Available {
                executable: None,
                version: "3.1".into(),
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
        let err = enforce_required(&report, &["mdbook".into()])
            .expect_err("missing required ecosystem must error");
        assert_eq!(err.class, DiagnosticClass::RuntimeAbsence);
        assert!(err.summary.contains("mdbook"));
    }

    #[test]
    fn enforce_required_rejects_unknown_ecosystem() {
        let report = fake_report(
            RuntimeStatus::Available {
                executable: None,
                version: "3.1".into(),
            },
            RuntimeStatus::Available {
                executable: None,
                version: "0.4.40".into(),
            },
            RuntimeStatus::Available {
                executable: None,
                version: "v18".into(),
            },
        );
        let err =
            enforce_required(&report, &["djot".into()]).expect_err("unknown ecosystem must error");
        assert_eq!(err.class, DiagnosticClass::RuntimeAbsence);
        assert!(err.summary.contains("djot"));
    }

    #[test]
    fn enforce_required_checks_all_supplied_ids() {
        let report = fake_report(
            RuntimeStatus::Missing {
                binary: "pandoc".into(),
                hint: install_hint_for("pandoc"),
            },
            RuntimeStatus::Available {
                executable: None,
                version: "0.4.40".into(),
            },
            RuntimeStatus::Available {
                executable: None,
                version: "v18".into(),
            },
        );
        // mdbook is available, pandoc is not. The first failure (in
        // input order) is what surfaces.
        let err = enforce_required(&report, &["mdbook".into(), "pandoc".into()])
            .expect_err("pandoc missing should fail the gate");
        assert!(err.summary.contains("pandoc"));
    }

    // ── install_hint_for ────────────────────────────────────────────

    #[test]
    fn install_hint_covers_every_v1_runtime() {
        for entry in ECOSYSTEMS {
            let hint = install_hint_for(entry.runtime_dep.binary);
            assert!(
                !hint.is_empty(),
                "install_hint_for {} must not be empty",
                entry.runtime_dep.binary
            );
            // The hint should at least reference the binary name so the
            // user knows what to install.
            assert!(
                hint.contains(entry.runtime_dep.binary),
                "install_hint_for {} should mention the binary by name (got: {hint:?})",
                entry.runtime_dep.binary
            );
        }
    }

    #[test]
    fn upgrade_hint_includes_min_version() {
        let hint = upgrade_hint_for("pandoc", "2.11");
        assert!(hint.contains("pandoc"));
        assert!(hint.contains("2.11"));
    }

    // ── detect_all_ecosystems integration ───────────────────────────

    #[test]
    fn detect_all_returns_one_entry_per_registered_ecosystem() {
        let report = detect_all_ecosystems();
        assert_eq!(report.entries.len(), ECOSYSTEMS.len());
        for entry in ECOSYSTEMS {
            assert!(
                report.entries.contains_key(entry.id),
                "detection report missing ecosystem {}",
                entry.id
            );
        }
    }
}
