//! Capability probe (SPEC-032 REQ-3216).
//!
//! At pipeline initialisation, every composed hook is invoked in
//! "probe mode" to declare which stages and AST types it supports,
//! which build modes it applies to, and whether it is ready to run.
//! This mirrors mdBook's `supports <renderer>` pattern: hooks can
//! self-disable cheaply without the cost of a full-pipeline invocation.
//!
//! The probe is a protocol-layer concern — [`PersistentHook::probe`]
//! exchanges a single JSON-lines round-trip. This module sits above it
//! and classifies the outcome against the hook's composed metadata
//! (expected stage, declared `ast_type`), emits `HookDiagnostic`s for
//! failures, and gives the caller a [`CapabilityReport`] per hook.
//!
//! Failure taxonomy:
//!
//! - [`ProbeOutcome::Ok`] — hook responded, declared capabilities line
//!   up with the manifest, `ready: true`.
//! - [`ProbeOutcome::Declined`] — hook responded `ready: false`; the
//!   session silently skips it and the caller logs a diagnostic.
//! - [`ProbeOutcome::StageMismatch`] — hook's `stages` omit the stage
//!   it was composed into. Disables the hook with a diagnostic.
//! - [`ProbeOutcome::AstVersionMismatch`] — the hook's declared
//!   `zetl_ast` is not compatible with the zetl AST schema version.
//! - [`ProbeOutcome::ProbeError`] — transport-level or protocol error
//!   (timeout, non-zero exit, malformed JSON). Disables the hook.

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use serde::Serialize;

use crate::hooks::ast::AST_VERSION;
use crate::hooks::composition::ComposedHook;
use crate::hooks::persistent::{
    PersistentHook, ProbeResult, ProtocolError, SecurityPolicy, DEFAULT_HANDSHAKE_TIMEOUT,
    DEFAULT_SHUTDOWN_GRACE,
};
use crate::hooks::pipeline::Stage;

/// Default probe deadline (SPEC-032 REQ-3216: "timeout > 5s").
pub const DEFAULT_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Outcome of probing a single composed hook.
#[derive(Debug, Clone)]
pub enum ProbeOutcome {
    /// Probe succeeded and declared capabilities are compatible with
    /// the composed hook's stage and AST type.
    Ok(ProbeResult),
    /// Probe succeeded but the hook returned `ready: false`, optionally
    /// with a human-readable `reason`. Not an error — the hook simply
    /// opted out for this session.
    Declined {
        result: ProbeResult,
        reason: Option<String>,
    },
    /// Probe succeeded but the hook's `stages` list does not include
    /// the stage it was composed into. The hook is disabled with a
    /// manifest/probe-mismatch diagnostic.
    StageMismatch {
        result: ProbeResult,
        expected: Stage,
        declared: Vec<String>,
    },
    /// Probe succeeded but the hook's declared `zetl_ast` is not
    /// compatible with the running zetl's AST schema version.
    AstVersionMismatch {
        result: ProbeResult,
        declared: String,
        expected: String,
    },
    /// Probe failed at the transport layer (timeout, non-zero exit,
    /// malformed JSON, or a protocol-level error). Hook is disabled
    /// for the session.
    ProbeError(String),
}

impl ProbeOutcome {
    /// Short axis tag for table rendering / diagnostic classification.
    pub fn status_tag(&self) -> &'static str {
        match self {
            ProbeOutcome::Ok(_) => "ok",
            ProbeOutcome::Declined { .. } => "declined",
            ProbeOutcome::StageMismatch { .. } => "stage-mismatch",
            ProbeOutcome::AstVersionMismatch { .. } => "ast-version-mismatch",
            ProbeOutcome::ProbeError(_) => "error",
        }
    }

    /// `true` when the hook remains enabled for the session.
    /// `Declined` counts as disabled — hook opted out.
    pub fn is_enabled(&self) -> bool {
        matches!(self, ProbeOutcome::Ok(_))
    }

    /// The underlying probe result, when the hook responded at all.
    pub fn result(&self) -> Option<&ProbeResult> {
        match self {
            ProbeOutcome::Ok(r) => Some(r),
            ProbeOutcome::Declined { result, .. } => Some(result),
            ProbeOutcome::StageMismatch { result, .. } => Some(result),
            ProbeOutcome::AstVersionMismatch { result, .. } => Some(result),
            ProbeOutcome::ProbeError(_) => None,
        }
    }
}

/// Per-hook capability report surfaced by `zetl hook capabilities` and
/// by pipeline-init probe wiring.
#[derive(Debug, Clone, Serialize)]
pub struct CapabilityReport {
    /// Composed hook's extension id.
    pub extension_id: String,
    /// Stage the hook was composed into.
    pub stage: String,
    /// Hook source file (for the diagnostic surface).
    pub path: PathBuf,
    /// Sidecar manifest path, if any.
    pub manifest_path: Option<PathBuf>,
    /// Probe outcome, pre-serialised into a stable shape.
    pub outcome: CapabilityOutcome,
}

/// Serialisable projection of [`ProbeOutcome`]. The wire form is a
/// tagged union so `zetl hook capabilities --json` round-trips cleanly
/// into tooling.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum CapabilityOutcome {
    Ok {
        result: ProbeResult,
    },
    Declined {
        result: ProbeResult,
        reason: Option<String>,
    },
    StageMismatch {
        result: ProbeResult,
        expected: String,
        declared: Vec<String>,
    },
    AstVersionMismatch {
        result: ProbeResult,
        declared: String,
        expected: String,
    },
    Error {
        message: String,
    },
}

impl From<ProbeOutcome> for CapabilityOutcome {
    fn from(o: ProbeOutcome) -> Self {
        match o {
            ProbeOutcome::Ok(r) => CapabilityOutcome::Ok { result: r },
            ProbeOutcome::Declined { result, reason } => {
                CapabilityOutcome::Declined { result, reason }
            }
            ProbeOutcome::StageMismatch {
                result,
                expected,
                declared,
            } => CapabilityOutcome::StageMismatch {
                result,
                expected: expected.to_string(),
                declared,
            },
            ProbeOutcome::AstVersionMismatch {
                result,
                declared,
                expected,
            } => CapabilityOutcome::AstVersionMismatch {
                result,
                declared,
                expected,
            },
            ProbeOutcome::ProbeError(m) => CapabilityOutcome::Error { message: m },
        }
    }
}

/// Probe a single composed hook.
///
/// Spawns the hook as a persistent process, waits for its startup
/// handshake, sends `{"type":"probe"}`, reads the `probe_result` line,
/// then shuts the process down cleanly. On any failure (timeout,
/// malformed response, non-zero exit, ast/stage mismatch) the child
/// is killed and the outcome is classified accordingly.
///
/// `mode` is echoed into the probe body and surfaces to hooks via the
/// `applies_when.modes` check — pass `None` for out-of-build probes
/// (e.g. the CLI subcommand) where no build mode is active.
///
/// `deadline` bounds the total time spent inside [`PersistentHook::probe`]
/// (the handshake has its own [`DEFAULT_HANDSHAKE_TIMEOUT`]).
pub fn probe_hook(hook: &ComposedHook, mode: Option<&str>, deadline: Duration) -> ProbeOutcome {
    let cmd = Command::new(&hook.path);
    let mut persistent = match PersistentHook::spawn_with_policy(
        cmd,
        hook.extension_id.clone(),
        hook.stage,
        DEFAULT_HANDSHAKE_TIMEOUT,
        DEFAULT_SHUTDOWN_GRACE,
        SecurityPolicy::default(),
    ) {
        Ok(p) => p,
        Err(e) => return ProbeOutcome::ProbeError(format!("spawn: {e}")),
    };

    let probe_result = match persistent.probe(mode, deadline.as_millis() as u64) {
        Ok(r) => r,
        Err(ProtocolError::Timeout { deadline }) => {
            let stderr = persistent.drain_stderr();
            let _ = persistent.shutdown();
            let detail = if stderr.is_empty() {
                String::new()
            } else {
                format!("; stderr: {}", stderr.trim())
            };
            return ProbeOutcome::ProbeError(format!(
                "probe exceeded deadline of {deadline:?}{detail}"
            ));
        }
        Err(e) => {
            let stderr = persistent.drain_stderr();
            let _ = persistent.shutdown();
            let detail = if stderr.is_empty() {
                String::new()
            } else {
                format!("; stderr: {}", stderr.trim())
            };
            return ProbeOutcome::ProbeError(format!("{e}{detail}"));
        }
    };

    // Shut down cleanly — the probe exchange is the full lifecycle of
    // the probing process (no init / run / finalise). Drop would also
    // kill, but a clean shutdown keeps per-test stderr noise down.
    let _ = persistent.shutdown();

    classify(probe_result, hook.stage)
}

/// Classify a probe response against the composed hook's expectations.
/// Pulled out of [`probe_hook`] so tests can exercise the matrix
/// without spawning real child processes.
pub fn classify(result: ProbeResult, expected: Stage) -> ProbeOutcome {
    if !result.ready {
        let reason = result.reason.clone();
        return ProbeOutcome::Declined { result, reason };
    }

    if !ast_schema_compatible(&result.zetl_ast, AST_VERSION) {
        let declared = result.zetl_ast.clone();
        return ProbeOutcome::AstVersionMismatch {
            result,
            declared,
            expected: AST_VERSION.to_string(),
        };
    }

    // `stages` empty is also a mismatch — the hook effectively declared
    // "I handle no stages".
    let declared_stages = result.stages.clone();
    if !declared_stages.iter().any(|s| s == expected.as_str()) {
        return ProbeOutcome::StageMismatch {
            result,
            expected,
            declared: declared_stages,
        };
    }

    ProbeOutcome::Ok(result)
}

/// Compatibility check for the hook's declared `zetl_ast` against the
/// running zetl's AST schema version.
///
/// Semver rules apply: MAJOR must match exactly; MINOR and PATCH are
/// forward-compatible (the hook may declare a lower minor than zetl
/// is running; zetl is responsible for preserving older minor shapes).
/// Free-form strings that don't look like semver fall back to exact
/// equality.
pub fn ast_schema_compatible(declared: &str, running: &str) -> bool {
    let d = parse_ver(declared);
    let r = parse_ver(running);
    match (d, r) {
        (Some((dmaj, dmin, _dpat)), Some((rmaj, rmin, _rpat))) => {
            if dmaj != rmaj {
                return false;
            }
            dmin <= rmin
        }
        _ => declared == running,
    }
}

fn parse_ver(s: &str) -> Option<(u32, u32, u32)> {
    let mut parts = s.split('.');
    let maj: u32 = parts.next()?.parse().ok()?;
    let min: u32 = parts.next().unwrap_or("0").parse().ok()?;
    let pat: u32 = parts.next().unwrap_or("0").parse().ok()?;
    Some((maj, min, pat))
}

/// Probe every enabled hook in a composed stage pipeline and collect
/// per-hook reports. Intended to be called once at pipeline init so
/// the caller can filter out probe-failed hooks before the build runs.
///
/// The function never short-circuits — every hook is probed so the
/// caller has a complete diagnostic surface. Failed hooks surface
/// their outcome via [`CapabilityOutcome::Error`] / `StageMismatch` /
/// `AstVersionMismatch` / `Declined` and the caller decides whether
/// to disable them.
pub fn probe_stage_pipeline(
    hooks: &[ComposedHook],
    mode: Option<&str>,
    deadline: Duration,
) -> Vec<CapabilityReport> {
    hooks
        .iter()
        .map(|h| {
            let outcome = probe_hook(h, mode, deadline);
            CapabilityReport {
                extension_id: h.extension_id.clone(),
                stage: h.stage.to_string(),
                path: h.path.clone(),
                manifest_path: h.manifest_path.clone(),
                outcome: outcome.into(),
            }
        })
        .collect()
}

/// Summarise a single capability report into a one-line stderr
/// diagnostic (`[zetl] hook probe: ...`). Used by both the init-time
/// probe wiring and the `zetl hook capabilities` pretty-printer.
pub fn format_diagnostic(report: &CapabilityReport) -> String {
    match &report.outcome {
        CapabilityOutcome::Ok { result } => format!(
            "[zetl] hook probe ok: stage={} id={} version={} zetl_ast={}",
            report.stage, report.extension_id, result.version, result.zetl_ast
        ),
        CapabilityOutcome::Declined { reason, .. } => {
            let r = reason.as_deref().unwrap_or("(no reason given)");
            format!(
                "[zetl] hook probe: stage={} id={} declined: {r}",
                report.stage, report.extension_id
            )
        }
        CapabilityOutcome::StageMismatch {
            expected, declared, ..
        } => format!(
            "[zetl] hook probe: stage={} id={} manifest/probe mismatch: expected stage {expected}, declared {declared:?}",
            report.stage, report.extension_id
        ),
        CapabilityOutcome::AstVersionMismatch {
            declared, expected, ..
        } => format!(
            "[zetl] hook probe: stage={} id={} ast version mismatch: hook declared {declared}, zetl speaks {expected}",
            report.stage, report.extension_id
        ),
        CapabilityOutcome::Error { message } => format!(
            "[zetl] hook probe: stage={} id={} error: {message}",
            report.stage, report.extension_id
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::persistent::AppliesWhen;

    fn sample_result(stages: &[&str], ready: bool) -> ProbeResult {
        ProbeResult {
            zetl_ast: AST_VERSION.to_string(),
            hook: "sample".into(),
            version: "0.1.0".into(),
            stages: stages.iter().map(|s| s.to_string()).collect(),
            ast_types: Some(vec!["zetl-ext".into()]),
            applies_when: Some(AppliesWhen::default()),
            ready,
            reason: None,
        }
    }

    #[test]
    fn classify_ok_when_stage_matches() {
        let r = sample_result(&["transform"], true);
        assert!(matches!(classify(r, Stage::Transform), ProbeOutcome::Ok(_)));
    }

    #[test]
    fn classify_stage_mismatch_disables_hook() {
        let r = sample_result(&["post-render"], true);
        match classify(r, Stage::Transform) {
            ProbeOutcome::StageMismatch {
                expected, declared, ..
            } => {
                assert_eq!(expected, Stage::Transform);
                assert_eq!(declared, vec!["post-render"]);
            }
            other => panic!("expected StageMismatch, got {other:?}"),
        }
    }

    #[test]
    fn classify_ready_false_is_declined_not_error() {
        let mut r = sample_result(&["transform"], false);
        r.reason = Some("build-only hook skipped under serve".into());
        match classify(r, Stage::Transform) {
            ProbeOutcome::Declined { reason, .. } => {
                assert_eq!(
                    reason.as_deref(),
                    Some("build-only hook skipped under serve")
                );
            }
            other => panic!("expected Declined, got {other:?}"),
        }
    }

    #[test]
    fn classify_ast_major_bump_is_mismatch() {
        let mut r = sample_result(&["transform"], true);
        r.zetl_ast = "2.0".into();
        assert!(matches!(
            classify(r, Stage::Transform),
            ProbeOutcome::AstVersionMismatch { .. }
        ));
    }

    #[test]
    fn ast_schema_compatibility_accepts_minor_lower() {
        // Hook pinned to 1.0 runs on zetl 1.3.
        assert!(ast_schema_compatible("1.0", "1.3"));
        assert!(ast_schema_compatible("1.2", "1.2.7"));
    }

    #[test]
    fn ast_schema_compatibility_rejects_higher_minor() {
        // Hook built for zetl 1.5 won't run on zetl 1.3 — the hook
        // expects a shape that older zetl doesn't emit.
        assert!(!ast_schema_compatible("1.5", "1.3"));
    }

    #[test]
    fn ast_schema_compatibility_rejects_major_mismatch() {
        assert!(!ast_schema_compatible("2.0", "1.0"));
        assert!(!ast_schema_compatible("1.0", "2.0"));
    }

    #[test]
    fn empty_stage_list_is_mismatch() {
        let r = sample_result(&[], true);
        match classify(r, Stage::Transform) {
            ProbeOutcome::StageMismatch { declared, .. } => assert!(declared.is_empty()),
            other => panic!("expected StageMismatch, got {other:?}"),
        }
    }

    #[test]
    fn applies_when_permits_mode_defaults_allow() {
        let a = AppliesWhen::default();
        assert!(a.permits_mode("build"));
        assert!(a.permits_mode("serve"));
    }

    #[test]
    fn applies_when_permits_mode_whitelist() {
        let a = AppliesWhen {
            modes: Some(vec!["build".into()]),
            ..Default::default()
        };
        assert!(a.permits_mode("build"));
        assert!(!a.permits_mode("serve"));
    }
}
