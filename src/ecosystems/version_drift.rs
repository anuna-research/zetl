//! Plugin-version drift detection (SPEC-033 REQ-3314).
//!
//! REQ-3314 extends the REQ-3313 runtime-detection contract from
//! "is the ecosystem's interpreter on PATH?" to "does the *plugin* on
//! PATH match the version we tested against?". Matrix entries
//! (REQ-3311) pin a `version_range` per (ecosystem, plugin) tuple; at
//! probe time, zetl invokes each configured plugin's `--version`
//! (Pandoc filters and mdBook preprocessors) or reads its
//! `package.json#version` (remark plugins), parses the observed
//! version, and classifies it against the matrix range:
//!
//! - **[`PluginVersionDrift::Exact`]** — observed equals the matrix-
//!   tested version component-wise. Silent.
//! - **[`PluginVersionDrift::MinorDrift`]** — same major, observed
//!   minor/patch is newer than tested, still within the range. Emits
//!   one `[zetl] ecosystem <eco>: <plugin> v<observed> is newer than
//!   last-tested v<tested>; proceeding` log line per session; the hook
//!   runs.
//! - **[`PluginVersionDrift::Incompatible`]** — different major, or
//!   observed below/above the range. The hook is disabled and a
//!   [`HookDiagnostic`] with class [`DiagnosticClass::RuntimeAbsence`]
//!   and a typed `plugin_version_incompatible` summary surfaces through
//!   the standard [`HookDiagnostic`] five-part shape (CON-3225). An
//!   actionable hint points the user at the matrix entry's range.
//!
//! The "tested" version shown in logs and diagnostics is the lower
//! bound of the `version_range` — that is the version the matrix row
//! has been verified against. Range parsing accepts the same
//! npm-style syntax that `ecosystem_matrix_integration.rs` validators
//! enforce: `>=<VER>` / `<=<VER>` / `><VER>` / `<<VER>` / `=<VER>` /
//! `^<VER>` / `~<VER>`, with one or two space-separated predicates
//! (e.g. `>=0.3.14 <0.4`, `^1.2`, `>=4.0 <5`). No pre-release or
//! build-metadata support in v1 — the matrix intentionally stays
//! conservative.
//!
//! The wiring into per-ecosystem adapters (`probe()` returning plugin
//! versions via [`EcosystemAdapter`][crate::ecosystems::EcosystemAdapter]
//! and feeding them through [`classify`]) is downstream of this module.
//! This task delivers: the data types, the classifier, the probe helper
//! that shells out to `<binary> --version`, the CON-3225 diagnostic
//! formatter for the incompatible case, and the log-line formatter for
//! the minor-drift warning.

use std::fmt;
use std::process::{Command, Stdio};

use crate::ecosystems::detection::parse_version;
use crate::hooks::diagnostic::{DiagnosticClass, HookDiagnostic};

// ── Types ───────────────────────────────────────────────────────────────────

/// Parsed dotted-numeric version. Component ordering is the obvious
/// `(major, minor, patch)` tuple comparison — matches the rest of
/// `src/ecosystems/detection.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u32, pub u32, pub u32);

impl Version {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Version(major, minor, patch)
    }

    pub fn major(self) -> u32 {
        self.0
    }

    pub fn minor(self) -> u32 {
        self.1
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

impl From<(u32, u32, u32)> for Version {
    fn from(v: (u32, u32, u32)) -> Self {
        Version(v.0, v.1, v.2)
    }
}

/// One operator in a `version_range` predicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeOp {
    /// `>=`
    Gte,
    /// `>`
    Gt,
    /// `<=`
    Lte,
    /// `<`
    Lt,
    /// `=` (or bare numeric)
    Eq,
    /// `^` — compatible with major (npm-style).
    Caret,
    /// `~` — compatible with minor (npm-style).
    Tilde,
}

/// One predicate in a `version_range`. `^1.2` parses to a single
/// [`RangePredicate`] with `op = Caret, version = 1.2.0` — callers
/// consult [`VersionRange::satisfied_by`] rather than dealing with
/// the predicate shape directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RangePredicate {
    pub op: RangeOp,
    pub version: Version,
}

/// A parsed `version_range` string. Carries the original text for
/// log/diagnostic rendering and a normalised predicate list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionRange {
    pub original: String,
    pub predicates: Vec<RangePredicate>,
}

impl VersionRange {
    /// Does `v` satisfy every predicate in the range?
    pub fn satisfied_by(&self, v: Version) -> bool {
        self.predicates.iter().all(|p| predicate_satisfied(p, v))
    }

    /// The "tested" version — the tightest lower-bound anchor in the
    /// range. Defined as the [`RangePredicate::version`] of the first
    /// lower-bound-shaped predicate (`>=`, `>`, `=`, `^`, `~`), or the
    /// first predicate's version if none is lower-bound-shaped. This
    /// is the version rendered as `last-tested v<X>` in REQ-3314's
    /// minor-drift log line.
    pub fn tested_version(&self) -> Option<Version> {
        self.predicates
            .iter()
            .find(|p| {
                matches!(
                    p.op,
                    RangeOp::Gte | RangeOp::Gt | RangeOp::Eq | RangeOp::Caret | RangeOp::Tilde
                )
            })
            .map(|p| p.version)
            .or_else(|| self.predicates.first().map(|p| p.version))
    }
}

impl fmt::Display for VersionRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.original)
    }
}

fn predicate_satisfied(p: &RangePredicate, v: Version) -> bool {
    match p.op {
        RangeOp::Gte => v >= p.version,
        RangeOp::Gt => v > p.version,
        RangeOp::Lte => v <= p.version,
        RangeOp::Lt => v < p.version,
        RangeOp::Eq => v == p.version,
        // `^X.Y.Z` allows >=X.Y.Z <(X+1).0.0 when X > 0, or >=0.Y.Z <0.(Y+1).0
        // when X == 0 (npm semantics).
        RangeOp::Caret => caret_satisfied(p.version, v),
        // `~X.Y.Z` allows >=X.Y.Z <X.(Y+1).0.
        RangeOp::Tilde => tilde_satisfied(p.version, v),
    }
}

fn caret_satisfied(anchor: Version, v: Version) -> bool {
    if v < anchor {
        return false;
    }
    if anchor.0 > 0 {
        v.0 == anchor.0
    } else if anchor.1 > 0 {
        v.0 == 0 && v.1 == anchor.1
    } else {
        v.0 == 0 && v.1 == 0 && v.2 == anchor.2
    }
}

fn tilde_satisfied(anchor: Version, v: Version) -> bool {
    v >= anchor && v.0 == anchor.0 && v.1 == anchor.1
}

/// Parse errors surfaced by [`parse_range`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionRangeParseError {
    pub input: String,
    pub reason: String,
}

impl fmt::Display for VersionRangeParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid version_range {:?}: {}", self.input, self.reason)
    }
}

impl std::error::Error for VersionRangeParseError {}

/// Parse an npm-style range string into a [`VersionRange`].
///
/// Accepts one or more space-separated predicates. Each predicate is
/// `<OP><NUMERIC>` where OP is one of `>=|<=|>|<|=|^|~` and NUMERIC is a
/// dotted-numeric version with 1–3 components. A bare numeric version
/// is implicit `=`.
pub fn parse_range(input: &str) -> Result<VersionRange, VersionRangeParseError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(VersionRangeParseError {
            input: input.to_string(),
            reason: "empty range".into(),
        });
    }
    let mut predicates = Vec::new();
    for part in trimmed.split_whitespace() {
        predicates.push(parse_predicate(part).map_err(|reason| VersionRangeParseError {
            input: input.to_string(),
            reason,
        })?);
    }
    Ok(VersionRange {
        original: trimmed.to_string(),
        predicates,
    })
}

fn parse_predicate(part: &str) -> Result<RangePredicate, String> {
    let (op, rest) = split_op(part)
        .ok_or_else(|| format!("predicate {part:?} does not start with a known operator"))?;
    let version = parse_numeric_version(rest)
        .ok_or_else(|| format!("predicate {part:?} tail {rest:?} is not dotted-numeric"))?;
    Ok(RangePredicate { op, version })
}

fn split_op(s: &str) -> Option<(RangeOp, &str)> {
    // Two-char operators first so `>=` doesn't parse as `>`.
    for (prefix, op) in [
        (">=", RangeOp::Gte),
        ("<=", RangeOp::Lte),
        ("==", RangeOp::Eq),
    ] {
        if let Some(rest) = s.strip_prefix(prefix) {
            return Some((op, rest));
        }
    }
    for (prefix, op) in [
        (">", RangeOp::Gt),
        ("<", RangeOp::Lt),
        ("=", RangeOp::Eq),
        ("^", RangeOp::Caret),
        ("~", RangeOp::Tilde),
    ] {
        if let Some(rest) = s.strip_prefix(prefix) {
            return Some((op, rest));
        }
    }
    // Bare numeric is implicit equality.
    if s.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return Some((RangeOp::Eq, s));
    }
    None
}

fn parse_numeric_version(s: &str) -> Option<Version> {
    if s.is_empty() {
        return None;
    }
    let parts: Vec<&str> = s.split('.').collect();
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    let major: u32 = parts[0].parse().ok()?;
    let minor: u32 = parts.get(1).map(|p| p.parse().ok()).unwrap_or(Some(0))?;
    let patch: u32 = parts.get(2).map(|p| p.parse().ok()).unwrap_or(Some(0))?;
    Some(Version(major, minor, patch))
}

/// Why a plugin was classified as [`PluginVersionDrift::Incompatible`].
/// Callers render a different hint per reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncompatReason {
    /// Observed major differs from tested major (breaking change).
    MajorMismatch,
    /// Observed version is strictly less than the tested lower bound.
    BelowRange,
    /// Observed version is at or above the range's upper bound.
    AboveRange,
}

impl IncompatReason {
    /// One-sentence cause for the [`HookDiagnostic`] `Likely cause:` line.
    pub fn cause_for(self) -> &'static str {
        match self {
            IncompatReason::MajorMismatch => {
                "the installed plugin's major version differs from the tested major \
                 version; breaking changes are likely"
            }
            IncompatReason::BelowRange => {
                "the installed plugin is older than the minimum version the matrix \
                 has been tested against"
            }
            IncompatReason::AboveRange => {
                "the installed plugin is newer than the maximum version the matrix \
                 has been tested against"
            }
        }
    }
}

/// Drift classification for one (ecosystem, plugin, observed version,
/// matrix range) tuple.
///
/// Construct via [`classify`]; render the minor-drift log line via
/// [`format_minor_drift_log`] and the incompatible diagnostic via
/// [`diagnostic_for_incompatible`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginVersionDrift {
    /// `observed == tested` component-wise. Silent.
    Exact {
        observed: Version,
        tested: Version,
    },
    /// Same major as tested, observed is newer, still within the
    /// matrix range. Warn once per session; the hook runs.
    MinorDrift {
        observed: Version,
        tested: Version,
        range: String,
    },
    /// Different major, or observed outside the matrix range.
    /// The hook SHALL be disabled.
    Incompatible {
        observed: Version,
        tested: Version,
        range: String,
        reason: IncompatReason,
    },
}

impl PluginVersionDrift {
    /// `true` for [`PluginVersionDrift::Incompatible`]. Convenience for
    /// the composition layer that disables incompatible hooks.
    pub fn is_incompatible(&self) -> bool {
        matches!(self, PluginVersionDrift::Incompatible { .. })
    }

    /// `true` for [`PluginVersionDrift::Exact`]. Convenience for log
    /// suppression (exact matches stay silent).
    pub fn is_exact(&self) -> bool {
        matches!(self, PluginVersionDrift::Exact { .. })
    }
}

/// Classify `observed` against `version_range` per REQ-3314.
///
/// Exact match wins over minor-drift (same major + in-range). Anything
/// outside the range — below, above, or different-major — is
/// [`PluginVersionDrift::Incompatible`]. `AboveRange` fires when the
/// range's explicit `<X`/`<=X` upper bound excludes the observed
/// version; different-major observed with no explicit upper bound is
/// classified as `MajorMismatch`, not `AboveRange`, so the hint can
/// lead with "breaking change".
pub fn classify(
    version_range: &str,
    observed: Version,
) -> Result<PluginVersionDrift, VersionRangeParseError> {
    let range = parse_range(version_range)?;
    let tested = range.tested_version().ok_or_else(|| VersionRangeParseError {
        input: version_range.to_string(),
        reason: "range has no predicates".into(),
    })?;

    if observed == tested {
        return Ok(PluginVersionDrift::Exact { observed, tested });
    }

    if observed.major() != tested.major() {
        return Ok(PluginVersionDrift::Incompatible {
            observed,
            tested,
            range: range.original,
            reason: IncompatReason::MajorMismatch,
        });
    }

    if observed < tested {
        return Ok(PluginVersionDrift::Incompatible {
            observed,
            tested,
            range: range.original,
            reason: IncompatReason::BelowRange,
        });
    }

    if !range.satisfied_by(observed) {
        return Ok(PluginVersionDrift::Incompatible {
            observed,
            tested,
            range: range.original,
            reason: IncompatReason::AboveRange,
        });
    }

    Ok(PluginVersionDrift::MinorDrift {
        observed,
        tested,
        range: range.original,
    })
}

// ── Rendering (logs + diagnostics) ──────────────────────────────────────────

/// Canonical minor-drift log line matching REQ-3314 verbatim.
///
/// Example: `[zetl] ecosystem pandoc: pandoc-crossref v0.3.16 is newer
/// than last-tested v0.3.14; proceeding`.
///
/// Emit once per session per (ecosystem, plugin) pair — caller
/// deduplicates.
pub fn format_minor_drift_log(
    ecosystem: &str,
    plugin: &str,
    observed: Version,
    tested: Version,
) -> String {
    format!(
        "[zetl] ecosystem {ecosystem}: {plugin} v{observed} is newer than last-tested v{tested}; proceeding"
    )
}

/// Build a five-part [`HookDiagnostic`] for a
/// [`PluginVersionDrift::Incompatible`] classification. The summary is
/// the typed `plugin_version_incompatible` string REQ-3314 names so
/// log-grep tooling can pick these out; the hint directs the user at
/// the matrix range and the `zetl ecosystem check` subcommand.
pub fn diagnostic_for_incompatible(
    ecosystem: &str,
    plugin: &str,
    observed: Version,
    tested: Version,
    range: &str,
    reason: IncompatReason,
) -> HookDiagnostic {
    HookDiagnostic::new(
        DiagnosticClass::RuntimeAbsence,
        format!("plugin_version_incompatible: {ecosystem}/{plugin} v{observed} outside tested range"),
    )
    .with_context(format!("matrix version_range = {range:?}"))
    .with_context(format!("last-tested v{tested}"))
    .with_observed(format!("found v{observed} on host"))
    .with_cause(reason.cause_for())
    .with_hint(format!(
        "install a version of '{plugin}' matching {range:?}, or update the matrix \
         entry after re-testing; `zetl ecosystem check` lists every configured plugin \
         and its observed version"
    ))
}

// ── Probe (invoke `<binary> --version`) ─────────────────────────────────────

/// Errors surfaced by [`probe_plugin_version`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginProbeError {
    /// The binary isn't on PATH (OS `ENOENT` on spawn).
    NotFound { binary: String },
    /// The binary spawned but didn't respond to `--version` the way
    /// we expected (non-zero exit, unparseable output, unreadable
    /// stdout). `detail` carries the root cause.
    ProbeFailed { binary: String, detail: String },
}

impl fmt::Display for PluginProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PluginProbeError::NotFound { binary } => {
                write!(f, "plugin binary '{binary}' not found on PATH")
            }
            PluginProbeError::ProbeFailed { binary, detail } => {
                write!(f, "`{binary} --version` probe failed: {detail}")
            }
        }
    }
}

impl std::error::Error for PluginProbeError {}

/// Invoke `<binary> --version` and parse the first dotted-numeric
/// version found in its output.
///
/// Mirrors the parser conventions of [`crate::ecosystems::detection::probe_runtime_dep`]
/// — we extract the first `MAJOR.MINOR(.PATCH)?` triple anywhere in
/// stdout. Pandoc filters and mdBook preprocessors all print a
/// version line on `--version`; remark plugins are JS packages and
/// are probed separately through [`probe_node_package_version`].
pub fn probe_plugin_version(binary: &str) -> Result<Version, PluginProbeError> {
    let mut cmd = Command::new(binary);
    cmd.arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = match cmd.output() {
        Ok(out) => out,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(PluginProbeError::NotFound {
                binary: binary.to_string(),
            });
        }
        Err(e) => {
            return Err(PluginProbeError::ProbeFailed {
                binary: binary.to_string(),
                detail: format!("could not spawn: {e}"),
            });
        }
    };
    if !output.status.success() {
        return Err(PluginProbeError::ProbeFailed {
            binary: binary.to_string(),
            detail: format!(
                "`{binary} --version` exited {} (stderr: {})",
                output
                    .status
                    .code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "signal".into()),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        });
    }
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    parse_version(&stdout)
        .map(Version::from)
        .ok_or_else(|| PluginProbeError::ProbeFailed {
            binary: binary.to_string(),
            detail: format!(
                "could not parse version from `{binary} --version` output {:?}",
                stdout.trim()
            ),
        })
}

/// Read the `version` field from `<node_modules_path>/<package>/package.json`.
///
/// remark plugins ship as npm packages and don't have a `--version`
/// CLI; their version lives on the package manifest. Callers resolve
/// `node_modules_path` (from remark harness init state) and pass it in
/// so this function stays pure.
pub fn probe_node_package_version(
    node_modules_path: &std::path::Path,
    package: &str,
) -> Result<Version, PluginProbeError> {
    let pkg_json = node_modules_path.join(package).join("package.json");
    let body = std::fs::read_to_string(&pkg_json).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            PluginProbeError::NotFound {
                binary: format!("{}/{package}/package.json", node_modules_path.display()),
            }
        } else {
            PluginProbeError::ProbeFailed {
                binary: format!("{}/{package}/package.json", node_modules_path.display()),
                detail: format!("read error: {e}"),
            }
        }
    })?;
    let parsed: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| PluginProbeError::ProbeFailed {
            binary: format!("{}/{package}/package.json", node_modules_path.display()),
            detail: format!("not valid JSON: {e}"),
        })?;
    let version_str = parsed.get("version").and_then(|v| v.as_str()).ok_or_else(|| {
        PluginProbeError::ProbeFailed {
            binary: format!("{}/{package}/package.json", node_modules_path.display()),
            detail: "`version` field missing or not a string".into(),
        }
    })?;
    parse_version(version_str)
        .map(Version::from)
        .ok_or_else(|| PluginProbeError::ProbeFailed {
            binary: format!("{}/{package}/package.json", node_modules_path.display()),
            detail: format!("unparseable version field {version_str:?}"),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Version + parse_numeric_version ────────────────────────────────

    #[test]
    fn numeric_version_parses_one_two_three_components() {
        assert_eq!(parse_numeric_version("1"), Some(Version(1, 0, 0)));
        assert_eq!(parse_numeric_version("1.2"), Some(Version(1, 2, 0)));
        assert_eq!(parse_numeric_version("1.2.3"), Some(Version(1, 2, 3)));
        assert!(parse_numeric_version("").is_none());
        assert!(parse_numeric_version("1.2.3.4").is_none());
        assert!(parse_numeric_version("1.a").is_none());
    }

    #[test]
    fn version_display_always_three_components() {
        assert_eq!(Version(1, 2, 3).to_string(), "1.2.3");
        assert_eq!(Version(0, 0, 0).to_string(), "0.0.0");
    }

    #[test]
    fn version_compare_is_tuple_order() {
        assert!(Version(1, 2, 3) < Version(1, 2, 4));
        assert!(Version(1, 2, 3) < Version(1, 3, 0));
        assert!(Version(1, 2, 3) < Version(2, 0, 0));
    }

    // ── parse_range ───────────────────────────────────────────────────

    #[test]
    fn parse_range_two_predicate_form() {
        let r = parse_range(">=0.3.14 <0.4").unwrap();
        assert_eq!(r.predicates.len(), 2);
        assert_eq!(
            r.predicates[0],
            RangePredicate {
                op: RangeOp::Gte,
                version: Version(0, 3, 14)
            }
        );
        assert_eq!(
            r.predicates[1],
            RangePredicate {
                op: RangeOp::Lt,
                version: Version(0, 4, 0)
            }
        );
    }

    #[test]
    fn parse_range_accepts_caret_and_tilde() {
        let r = parse_range("^1.2.3").unwrap();
        assert_eq!(r.predicates[0].op, RangeOp::Caret);
        let r = parse_range("~1.2").unwrap();
        assert_eq!(r.predicates[0].op, RangeOp::Tilde);
    }

    #[test]
    fn parse_range_accepts_bare_numeric_as_equality() {
        let r = parse_range("1.2.3").unwrap();
        assert_eq!(r.predicates[0].op, RangeOp::Eq);
        assert_eq!(r.predicates[0].version, Version(1, 2, 3));
    }

    #[test]
    fn parse_range_rejects_empty_and_bogus() {
        assert!(parse_range("").is_err());
        assert!(parse_range("  ").is_err());
        assert!(parse_range("bogus").is_err());
        assert!(parse_range(">=notaversion").is_err());
    }

    // ── satisfied_by + tested_version ─────────────────────────────────

    #[test]
    fn satisfied_by_matches_two_predicate_range() {
        let r = parse_range(">=0.3.14 <0.4").unwrap();
        assert!(r.satisfied_by(Version(0, 3, 14)));
        assert!(r.satisfied_by(Version(0, 3, 99)));
        assert!(!r.satisfied_by(Version(0, 3, 13)));
        assert!(!r.satisfied_by(Version(0, 4, 0)));
        assert!(!r.satisfied_by(Version(1, 0, 0)));
    }

    #[test]
    fn caret_respects_zero_major_npm_semantics() {
        let r = parse_range("^0.3.14").unwrap();
        assert!(r.satisfied_by(Version(0, 3, 14)));
        assert!(r.satisfied_by(Version(0, 3, 99)));
        // ^0.3.14 does NOT admit 0.4.0 (npm treats 0.x as "every minor is breaking").
        assert!(!r.satisfied_by(Version(0, 4, 0)));
    }

    #[test]
    fn caret_nonzero_major_allows_minor_patch() {
        let r = parse_range("^1.2.3").unwrap();
        assert!(r.satisfied_by(Version(1, 2, 3)));
        assert!(r.satisfied_by(Version(1, 9, 99)));
        assert!(!r.satisfied_by(Version(2, 0, 0)));
        assert!(!r.satisfied_by(Version(1, 2, 2)));
    }

    #[test]
    fn tilde_allows_patch_only() {
        let r = parse_range("~1.2.3").unwrap();
        assert!(r.satisfied_by(Version(1, 2, 3)));
        assert!(r.satisfied_by(Version(1, 2, 99)));
        assert!(!r.satisfied_by(Version(1, 3, 0)));
    }

    #[test]
    fn tested_version_picks_lower_bound() {
        assert_eq!(
            parse_range(">=0.3.14 <0.4").unwrap().tested_version(),
            Some(Version(0, 3, 14))
        );
        assert_eq!(
            parse_range("^1.2.3").unwrap().tested_version(),
            Some(Version(1, 2, 3))
        );
        // Pure upper bound: falls back to the first predicate.
        assert_eq!(
            parse_range("<2.0.0").unwrap().tested_version(),
            Some(Version(2, 0, 0))
        );
    }

    // ── classify (REQ-3314 decision tree) ─────────────────────────────

    #[test]
    fn classify_exact_is_silent_match() {
        let d = classify(">=0.3.14 <0.4", Version(0, 3, 14)).unwrap();
        assert!(d.is_exact());
        match d {
            PluginVersionDrift::Exact { observed, tested } => {
                assert_eq!(observed, Version(0, 3, 14));
                assert_eq!(tested, Version(0, 3, 14));
            }
            other => panic!("expected Exact, got {other:?}"),
        }
    }

    #[test]
    fn classify_same_major_newer_in_range_is_minor_drift() {
        // >=0.3.14 <0.4: observed 0.3.16 is newer + in range.
        let d = classify(">=0.3.14 <0.4", Version(0, 3, 16)).unwrap();
        match d {
            PluginVersionDrift::MinorDrift {
                observed, tested, ..
            } => {
                assert_eq!(observed, Version(0, 3, 16));
                assert_eq!(tested, Version(0, 3, 14));
            }
            other => panic!("expected MinorDrift, got {other:?}"),
        }
    }

    #[test]
    fn classify_different_major_is_incompatible_major_mismatch() {
        let d = classify(">=4.0 <5", Version(5, 0, 0)).unwrap();
        assert!(d.is_incompatible());
        match d {
            PluginVersionDrift::Incompatible { reason, .. } => {
                assert_eq!(reason, IncompatReason::MajorMismatch);
            }
            other => panic!("expected Incompatible, got {other:?}"),
        }
    }

    #[test]
    fn classify_below_lower_bound_is_incompatible_below_range() {
        let d = classify(">=0.3.14 <0.4", Version(0, 3, 10)).unwrap();
        match d {
            PluginVersionDrift::Incompatible { reason, .. } => {
                assert_eq!(reason, IncompatReason::BelowRange);
            }
            other => panic!("expected Incompatible, got {other:?}"),
        }
    }

    #[test]
    fn classify_at_or_above_upper_bound_is_incompatible_above_range() {
        // Same major as tested (0), observed 0.4.0 hits the exclusive upper bound.
        let d = classify(">=0.3.14 <0.4", Version(0, 4, 0)).unwrap();
        match d {
            PluginVersionDrift::Incompatible { reason, .. } => {
                assert_eq!(reason, IncompatReason::AboveRange);
            }
            other => panic!("expected Incompatible, got {other:?}"),
        }
    }

    #[test]
    fn classify_rejects_malformed_range() {
        assert!(classify("bogus-range", Version(1, 0, 0)).is_err());
    }

    // ── Rendering: minor-drift log + incompatible diagnostic ──────────

    #[test]
    fn minor_drift_log_matches_req_3314_verbatim() {
        let line = format_minor_drift_log(
            "pandoc",
            "pandoc-crossref",
            Version(0, 3, 16),
            Version(0, 3, 14),
        );
        assert_eq!(
            line,
            "[zetl] ecosystem pandoc: pandoc-crossref v0.3.16 is newer than last-tested v0.3.14; proceeding"
        );
    }

    #[test]
    fn incompatible_diagnostic_has_five_part_shape() {
        let diag = diagnostic_for_incompatible(
            "pandoc",
            "pandoc-crossref",
            Version(1, 0, 0),
            Version(0, 3, 14),
            ">=0.3.14 <0.4",
            IncompatReason::MajorMismatch,
        );
        // Class + machine-grep tag.
        assert_eq!(diag.class, DiagnosticClass::RuntimeAbsence);
        assert!(diag.summary.contains("plugin_version_incompatible"));
        // Rendered body carries all five parts.
        let rendered = diag.to_string();
        assert!(rendered.starts_with("[zetl] "));
        assert!(rendered.contains("plugin_version_incompatible"));
        assert!(rendered.contains("version_range"));
        assert!(rendered.contains("v1.0.0"));
        assert!(rendered.contains("v0.3.14"));
        assert!(rendered.contains("Likely cause: "));
        assert!(rendered.contains("Hint: "));
        assert!(rendered.contains("zetl ecosystem check"));
    }

    #[test]
    fn incompatible_hint_suggests_range_and_ecosystem_check() {
        let diag = diagnostic_for_incompatible(
            "mdbook",
            "mdbook-mermaid",
            Version(0, 13, 0),
            Version(0, 14, 0),
            ">=0.14 <0.16",
            IncompatReason::BelowRange,
        );
        let hint = diag.hint.as_deref().unwrap_or_default();
        assert!(hint.contains(">=0.14 <0.16"));
        assert!(hint.contains("mdbook-mermaid"));
        assert!(hint.contains("zetl ecosystem check"));
    }

    // ── probe_plugin_version: NotFound ────────────────────────────────

    #[test]
    fn probe_plugin_returns_not_found_for_bogus_binary() {
        let err = probe_plugin_version("definitely-not-a-real-plugin-zetl-test").unwrap_err();
        match err {
            PluginProbeError::NotFound { binary } => {
                assert_eq!(binary, "definitely-not-a-real-plugin-zetl-test");
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    // ── probe_node_package_version against a tempdir fixture ──────────

    #[test]
    fn probe_node_package_reads_version_from_package_json() {
        let tmp = tempfile::tempdir().unwrap();
        let pkg_dir = tmp.path().join("remark-gfm");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(
            pkg_dir.join("package.json"),
            r#"{"name": "remark-gfm", "version": "4.0.1"}"#,
        )
        .unwrap();
        let v = probe_node_package_version(tmp.path(), "remark-gfm").unwrap();
        assert_eq!(v, Version(4, 0, 1));
    }

    #[test]
    fn probe_node_package_missing_returns_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let err = probe_node_package_version(tmp.path(), "remark-ghost").unwrap_err();
        match err {
            PluginProbeError::NotFound { .. } => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn probe_node_package_bad_json_returns_probe_failed() {
        let tmp = tempfile::tempdir().unwrap();
        let pkg_dir = tmp.path().join("remark-broken");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join("package.json"), "not json {{").unwrap();
        let err = probe_node_package_version(tmp.path(), "remark-broken").unwrap_err();
        match err {
            PluginProbeError::ProbeFailed { detail, .. } => {
                assert!(detail.contains("not valid JSON"), "got {detail:?}");
            }
            other => panic!("expected ProbeFailed, got {other:?}"),
        }
    }

    #[test]
    fn probe_node_package_missing_version_field_returns_probe_failed() {
        let tmp = tempfile::tempdir().unwrap();
        let pkg_dir = tmp.path().join("remark-noversion");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join("package.json"), r#"{"name":"x"}"#).unwrap();
        let err = probe_node_package_version(tmp.path(), "remark-noversion").unwrap_err();
        match err {
            PluginProbeError::ProbeFailed { detail, .. } => {
                assert!(detail.contains("version"));
            }
            other => panic!("expected ProbeFailed, got {other:?}"),
        }
    }
}
