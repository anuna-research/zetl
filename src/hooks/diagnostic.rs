//! Shared diagnostic type for the hook subsystem (SPEC-032 CON-3225).
//!
//! Every failure path that zetl logs from the hook subsystem — manifest
//! parse errors, selector evaluation failures, contract violations,
//! schema mismatches, protocol errors, ecosystem runtime absence —
//! renders through [`HookDiagnostic`] so all messages share one
//! five-part shape: summary, context, observed data, likely cause,
//! remediation hint.
//!
//! The shape is a quality-gate item: the three worked examples in
//! CON-3225 are constructed by [`HookDiagnostic::example_preservation`],
//! [`HookDiagnostic::example_idempotence`], and
//! [`HookDiagnostic::example_manifest_parse`], and their byte output is
//! pinned by snapshot tests.
//!
//! # Rendering
//!
//! ```text
//! [zetl] <summary-line>:
//!   <context line>
//!   <observed data line>
//!
//!   Likely cause: <one sentence naming the common cause>
//!   Hint: <link or command to run next>
//! ```
//!
//! Under [`Verbosity::Quiet`] only the summary line is emitted.
//! Under [`Verbosity::Default`] all five parts are emitted.
//! Under [`Verbosity::Verbose`] the captured hook stderr is appended
//! when the failure was a crash or timeout.

use std::fmt;

/// Which failure class this diagnostic represents. Stays machine-grep-able
/// so tooling can filter logs; the human-facing label lives in the
/// summary line the caller constructs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticClass {
    /// Sidecar TOML manifest failed to parse or has an unknown field.
    ManifestParse,
    /// Hook process exited non-zero, timed out, or crashed.
    HookFailure,
    /// `contract.*` invariant violated (preservation, idempotence, purity).
    ContractViolation,
    /// Output JSON did not match the zetl-ast schema the hook claimed.
    SchemaMismatch,
    /// Selector evaluation (path glob, frontmatter, content probe) failed.
    SelectorEval,
    /// Persistent-mode JSON-line protocol framing or handshake failure.
    ProtocolError,
    /// Ecosystem adapter's declared runtime (pandoc, node, mdbook) missing.
    RuntimeAbsence,
}

impl DiagnosticClass {
    /// Short machine-readable identifier used in log grep patterns.
    pub fn tag(self) -> &'static str {
        match self {
            DiagnosticClass::ManifestParse => "manifest-parse",
            DiagnosticClass::HookFailure => "hook-failure",
            DiagnosticClass::ContractViolation => "contract-violation",
            DiagnosticClass::SchemaMismatch => "schema-mismatch",
            DiagnosticClass::SelectorEval => "selector-eval",
            DiagnosticClass::ProtocolError => "protocol-error",
            DiagnosticClass::RuntimeAbsence => "runtime-absence",
        }
    }
}

/// How much of the diagnostic to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verbosity {
    /// Summary line only.
    Quiet,
    /// Full five-part body.
    Default,
    /// Full body plus captured stderr (when the failure was a crash/timeout).
    Verbose,
}

/// A single diagnostic emitted by the hook subsystem.
///
/// All fields except `summary` and `class` are optional so the shared
/// type fits failure classes that don't have a natural "likely cause"
/// (e.g. a TOML parse error whose location already names the cause).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookDiagnostic {
    /// Failure class. Machine-grep-able dimension for log filters.
    pub class: DiagnosticClass,
    /// First line after the `[zetl] ` prefix. May contain embedded
    /// newlines — continuation lines are rendered with a 7-space
    /// indent so they align under the first character of text.
    ///
    /// Do *not* include a trailing `:` — the renderer appends it.
    pub summary: String,
    /// Which manifest field / contract invariant / matrix entry is
    /// implicated. Each line rendered with a 2-space indent.
    pub context: Vec<String>,
    /// Concrete numbers or sample text showing what went wrong; not
    /// just "failed". Each line rendered with a 2-space indent.
    pub observed: Vec<String>,
    /// One sentence naming the most common cause of this failure class.
    /// Multi-line strings are rendered with the "Likely cause: " label
    /// on the first line and 2-space-indented continuation lines.
    pub cause: Option<String>,
    /// A link or a command the author can run next to make progress.
    /// Same multi-line rendering rule as `cause`.
    pub hint: Option<String>,
    /// Captured hook stderr — appended under [`Verbosity::Verbose`]
    /// when the failure was a crash or timeout.
    pub stderr: Option<String>,
}

impl HookDiagnostic {
    /// Minimal constructor. Callers append context/observed/etc.
    pub fn new(class: DiagnosticClass, summary: impl Into<String>) -> Self {
        Self {
            class,
            summary: summary.into(),
            context: Vec::new(),
            observed: Vec::new(),
            cause: None,
            hint: None,
            stderr: None,
        }
    }

    pub fn with_context(mut self, line: impl Into<String>) -> Self {
        self.context.push(line.into());
        self
    }

    pub fn with_observed(mut self, line: impl Into<String>) -> Self {
        self.observed.push(line.into());
        self
    }

    pub fn with_cause(mut self, cause: impl Into<String>) -> Self {
        self.cause = Some(cause.into());
        self
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn with_stderr(mut self, stderr: impl Into<String>) -> Self {
        self.stderr = Some(stderr.into());
        self
    }

    /// Render the diagnostic at the requested verbosity and return the
    /// resulting text. Output never contains ANSI escape sequences, so
    /// `NO_COLOR` and `--no-color` are honoured trivially.
    pub fn render(&self, verbosity: Verbosity) -> String {
        let mut buf = String::new();
        self.render_into(verbosity, &mut buf);
        buf
    }

    fn render_into(&self, verbosity: Verbosity, buf: &mut String) {
        // Summary line(s): "[zetl] " on the first, 7-space indent on
        // continuations, trailing ":" always.
        let mut summary_lines = self.summary.lines();
        if let Some(first) = summary_lines.next() {
            buf.push_str("[zetl] ");
            buf.push_str(first);
        }
        for line in summary_lines {
            buf.push('\n');
            buf.push_str("       ");
            buf.push_str(line);
        }
        buf.push_str(":\n");

        if verbosity == Verbosity::Quiet {
            return;
        }

        for line in &self.context {
            buf.push_str("  ");
            buf.push_str(line);
            buf.push('\n');
        }
        for line in &self.observed {
            buf.push_str("  ");
            buf.push_str(line);
            buf.push('\n');
        }

        let has_cause_or_hint = self.cause.is_some() || self.hint.is_some();
        let has_body_above = !self.context.is_empty() || !self.observed.is_empty();
        if has_cause_or_hint && has_body_above {
            buf.push('\n');
        }

        render_labelled(buf, "Likely cause: ", self.cause.as_deref());
        render_labelled(buf, "Hint: ", self.hint.as_deref());

        if verbosity == Verbosity::Verbose {
            if let Some(stderr) = &self.stderr {
                let trimmed = stderr.trim_end_matches('\n');
                if !trimmed.is_empty() {
                    buf.push('\n');
                    buf.push_str("  stderr:\n");
                    for line in trimmed.lines() {
                        buf.push_str("    ");
                        buf.push_str(line);
                        buf.push('\n');
                    }
                }
            }
        }
    }
}

impl fmt::Display for HookDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render(Verbosity::Default))
    }
}

impl std::error::Error for HookDiagnostic {}

fn render_labelled(buf: &mut String, label: &str, body: Option<&str>) {
    let Some(body) = body else { return };
    let mut lines = body.lines();
    let Some(first) = lines.next() else { return };
    buf.push_str("  ");
    buf.push_str(label);
    buf.push_str(first);
    buf.push('\n');
    for line in lines {
        buf.push_str("  ");
        buf.push_str(line);
        buf.push('\n');
    }
}

// ── CON-3225 worked-example constructors ─────────────────────────────────
//
// These reproduce the three worked examples in SPEC-032 §CON-3225
// verbatim. They exist as constructors (not just test fixtures) so
// downstream callers can use them as starting points when shaping new
// contract-violation or manifest-parse diagnostics — and so a single
// snapshot test pins their byte output across releases (acceptance
// gate for task-diagnostic-design).

impl HookDiagnostic {
    /// Worked example 1 from CON-3225 — contract preservation violation.
    pub fn example_preservation() -> Self {
        HookDiagnostic::new(
            DiagnosticClass::ContractViolation,
            "hook 'callouts' contract violation on projects/q2-review.md",
        )
        .with_context(r#"contract.preserves = ["Wikilink", "Embed"]"#)
        .with_observed("observed in input:  12 Wikilink, 3 Embed")
        .with_observed("observed in output:  8 Wikilink, 3 Embed")
        .with_observed("net change: -4 Wikilink")
        .with_cause(
            "the transform strips unrecognised Span classes or\n\
             drops inline nodes whose type it doesn't match.",
        )
        .with_hint(
            "run `zetl ast diff <before.json> <after.json>` on the fixture\n\
             input to locate the removed nodes; see:\n  \
             https://zetl.codeberg.page/docs/hook-authoring/preservation",
        )
    }

    /// Worked example 2 from CON-3225 — idempotence violation (CI double-run).
    pub fn example_idempotence() -> Self {
        HookDiagnostic::new(
            DiagnosticClass::ContractViolation,
            "hook 'tasks' contract violation in CI double-run on\n\
             tests/hook-fixtures/tasks/input.md",
        )
        .with_context("contract.idempotent = true (declared)")
        .with_context("observed: canonicalise(f(f(x))) != canonicalise(f(x))")
        .with_observed("first run:  3 tasks blocks rendered, 0 nested")
        .with_observed("second run: 3 tasks blocks rendered, 3 nested \u{2190} likely bug")
        .with_cause(
            "the transform runs over its own output, wrapping\n\
             already-rendered blocks a second time.",
        )
        .with_hint(
            "detect already-rendered output (e.g. presence of\n\
             `class=\"zetl-tasks-rendered\"`) and short-circuit, OR declare\n\
             contract.idempotent = false if nested output is intended.",
        )
    }

    /// Worked example 3 from CON-3225 — manifest parse error.
    pub fn example_manifest_parse() -> Self {
        HookDiagnostic::new(
            DiagnosticClass::ManifestParse,
            "hook manifest error at .zetl/hooks/transform.d/foo.toml",
        )
        .with_context("unknown field 'ecosystm' (did you mean 'ecosystem'?)")
        .with_observed("line 3, column 1")
        .with_hint(
            "valid top-level fields: stage, mode, timeout_ms, memory_mib,\n\
             ast_type, ast_version, ecosystem, select, before, after, optional,\n\
             extension_id, contract. See:\n  \
             https://zetl.codeberg.page/docs/hook-authoring/manifest-fields",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_tag_stable() {
        assert_eq!(DiagnosticClass::ManifestParse.tag(), "manifest-parse");
        assert_eq!(DiagnosticClass::ContractViolation.tag(), "contract-violation");
        assert_eq!(DiagnosticClass::HookFailure.tag(), "hook-failure");
        assert_eq!(DiagnosticClass::SchemaMismatch.tag(), "schema-mismatch");
    }

    #[test]
    fn quiet_renders_summary_only() {
        let d = HookDiagnostic::new(DiagnosticClass::HookFailure, "hook 'foo' crashed")
            .with_context("ignored in quiet")
            .with_observed("also ignored")
            .with_cause("and this")
            .with_hint("and this too");
        assert_eq!(d.render(Verbosity::Quiet), "[zetl] hook 'foo' crashed:\n");
    }

    #[test]
    fn default_renders_full_body() {
        let d = HookDiagnostic::new(DiagnosticClass::HookFailure, "hook 'foo' crashed")
            .with_context("field = value")
            .with_observed("observed: 1 != 2")
            .with_cause("the transform mishandles X.")
            .with_hint("run `zetl hook test foo`.");
        let expected = "\
[zetl] hook 'foo' crashed:
  field = value
  observed: 1 != 2

  Likely cause: the transform mishandles X.
  Hint: run `zetl hook test foo`.
";
        assert_eq!(d.render(Verbosity::Default), expected);
    }

    #[test]
    fn multi_line_summary_indents_continuation() {
        let d = HookDiagnostic::new(
            DiagnosticClass::ContractViolation,
            "hook 'tasks' violated contract on\nfixtures/a/b.md",
        );
        let rendered = d.render(Verbosity::Quiet);
        assert_eq!(
            rendered,
            "[zetl] hook 'tasks' violated contract on\n       fixtures/a/b.md:\n"
        );
    }

    #[test]
    fn missing_cause_still_renders_hint_with_separator() {
        let d = HookDiagnostic::new(DiagnosticClass::ManifestParse, "hook manifest error")
            .with_context("unknown field 'x'")
            .with_observed("line 3, column 1")
            .with_hint("valid fields: stage, mode, ...");
        let expected = "\
[zetl] hook manifest error:
  unknown field 'x'
  line 3, column 1

  Hint: valid fields: stage, mode, ...
";
        assert_eq!(d.render(Verbosity::Default), expected);
    }

    #[test]
    fn summary_only_diagnostic_omits_separator() {
        // No context, no observed, no cause, no hint — summary only.
        let d = HookDiagnostic::new(DiagnosticClass::HookFailure, "hook 'foo' exited 1");
        assert_eq!(d.render(Verbosity::Default), "[zetl] hook 'foo' exited 1:\n");
    }

    #[test]
    fn verbose_appends_stderr_when_present() {
        let d = HookDiagnostic::new(DiagnosticClass::HookFailure, "hook 'foo' crashed")
            .with_cause("the process aborted.")
            .with_hint("run `zetl hook test foo --verbose`.")
            .with_stderr("Traceback (most recent call last):\n  File \"foo.py\"\nNameError: x\n");
        let rendered = d.render(Verbosity::Verbose);
        assert!(rendered.contains("  stderr:\n"));
        assert!(rendered.contains("    Traceback (most recent call last):\n"));
        assert!(rendered.contains("    NameError: x\n"));
    }

    #[test]
    fn verbose_omits_stderr_when_empty() {
        let d = HookDiagnostic::new(DiagnosticClass::HookFailure, "hook 'foo' crashed")
            .with_stderr("");
        let rendered = d.render(Verbosity::Verbose);
        assert!(!rendered.contains("stderr"));
    }

    #[test]
    fn default_verbosity_omits_stderr() {
        let d = HookDiagnostic::new(DiagnosticClass::HookFailure, "hook 'foo' crashed")
            .with_stderr("panic: everything is broken\n");
        let rendered = d.render(Verbosity::Default);
        assert!(!rendered.contains("panic"));
        assert!(!rendered.contains("stderr"));
    }

    #[test]
    fn display_trait_renders_default_verbosity() {
        let d = HookDiagnostic::new(DiagnosticClass::HookFailure, "hook 'foo' crashed")
            .with_context("field = value");
        assert_eq!(format!("{d}"), d.render(Verbosity::Default));
    }

    // ── CON-3225 byte-stable snapshots ──────────────────────────────────
    //
    // These tests are the regression gate for task-diagnostic-design:
    // "snapshot-test of the three CON-3225 worked examples stays
    // byte-stable across releases." Any diff here is intentional and
    // must be paired with a CON-3225 update in SPEC-032.

    const PRESERVATION_SNAPSHOT: &str = "\
[zetl] hook 'callouts' contract violation on projects/q2-review.md:
  contract.preserves = [\"Wikilink\", \"Embed\"]
  observed in input:  12 Wikilink, 3 Embed
  observed in output:  8 Wikilink, 3 Embed
  net change: -4 Wikilink

  Likely cause: the transform strips unrecognised Span classes or
  drops inline nodes whose type it doesn't match.
  Hint: run `zetl ast diff <before.json> <after.json>` on the fixture
  input to locate the removed nodes; see:
    https://zetl.codeberg.page/docs/hook-authoring/preservation
";

    const IDEMPOTENCE_SNAPSHOT: &str = "\
[zetl] hook 'tasks' contract violation in CI double-run on
       tests/hook-fixtures/tasks/input.md:
  contract.idempotent = true (declared)
  observed: canonicalise(f(f(x))) != canonicalise(f(x))
  first run:  3 tasks blocks rendered, 0 nested
  second run: 3 tasks blocks rendered, 3 nested \u{2190} likely bug

  Likely cause: the transform runs over its own output, wrapping
  already-rendered blocks a second time.
  Hint: detect already-rendered output (e.g. presence of
  `class=\"zetl-tasks-rendered\"`) and short-circuit, OR declare
  contract.idempotent = false if nested output is intended.
";

    const MANIFEST_PARSE_SNAPSHOT: &str = "\
[zetl] hook manifest error at .zetl/hooks/transform.d/foo.toml:
  unknown field 'ecosystm' (did you mean 'ecosystem'?)
  line 3, column 1

  Hint: valid top-level fields: stage, mode, timeout_ms, memory_mib,
  ast_type, ast_version, ecosystem, select, before, after, optional,
  extension_id, contract. See:
    https://zetl.codeberg.page/docs/hook-authoring/manifest-fields
";

    #[test]
    fn snapshot_con_3225_preservation() {
        assert_eq!(
            HookDiagnostic::example_preservation().render(Verbosity::Default),
            PRESERVATION_SNAPSHOT,
        );
    }

    #[test]
    fn snapshot_con_3225_idempotence() {
        assert_eq!(
            HookDiagnostic::example_idempotence().render(Verbosity::Default),
            IDEMPOTENCE_SNAPSHOT,
        );
    }

    #[test]
    fn snapshot_con_3225_manifest_parse() {
        assert_eq!(
            HookDiagnostic::example_manifest_parse().render(Verbosity::Default),
            MANIFEST_PARSE_SNAPSHOT,
        );
    }

    #[test]
    fn worked_example_classes_match_con_3225() {
        assert_eq!(
            HookDiagnostic::example_preservation().class,
            DiagnosticClass::ContractViolation
        );
        assert_eq!(
            HookDiagnostic::example_idempotence().class,
            DiagnosticClass::ContractViolation
        );
        assert_eq!(
            HookDiagnostic::example_manifest_parse().class,
            DiagnosticClass::ManifestParse
        );
    }
}
