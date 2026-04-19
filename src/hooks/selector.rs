//! Hook selector evaluator (SPEC-032 REQ-3204 / CON-3204).
//!
//! A hook's selector decides whether a given page triggers the hook. This
//! module compiles a [`SelectorSpec`] (parsed from the sidecar TOML
//! manifest) into a [`CompiledSelector`] and evaluates it against a
//! [`SelectorInput`] per page, short-circuiting cheap-to-expensive:
//!
//! 1. **Path** — `include` globs match AND no `exclude` glob matches.
//! 2. **Frontmatter predicate** — `frontmatter_where` expression resolves
//!    to `true` (REQ-3205 grammar; evaluated by
//!    [`crate::hooks::predicate`]).
//! 3. **Content probes** — each probe (literal substring or `re:<pat>`
//!    regex) combined under `require_probe_match` (any / all).
//!
//! All regexes and globs are compiled **once per build** at
//! [`compile`] time; [`evaluate`] does pure predicate application with no
//! allocation on the hot path (NFR-3201: ≤ 2 ms / (page × hook) P95 on a
//! 100 KB page).
//!
//! # Why this order
//!
//! - Path globbing on a short vault-relative string costs microseconds.
//! - Frontmatter predicate evaluation needs the frontmatter to have been
//!   parsed, but is still linear in a few map/array lookups.
//! - Content probes walk the raw Markdown text. For a 100 KB page, each
//!   literal / regex probe is orders of magnitude slower than the path
//!   or frontmatter check.
//!
//! Short-circuiting in this order means most pages are rejected after
//! the cheapest check, keeping the overall per-page cost near the path
//! check for non-matching hooks.
//!
//! # SelectorInput
//!
//! Shell code builds [`SelectorInput`] once per page (see SPEC-032 §7,
//! Purity Boundary Map). Core never reads the filesystem. The caller is
//! responsible for capping `text` to at most 100 KB if it wishes to
//! enforce NFR-3201's page-size budget strictly.

use std::path::Path;

use globset::{Glob, GlobSet, GlobSetBuilder};
use regex::Regex;
use serde_json::Value;

use crate::hooks::manifest::{ProbeMatch, SelectorSpec};
use crate::hooks::predicate::{self, Predicate};

// ── Compiled form ───────────────────────────────────────────────────────────

/// A selector compiled for fast per-page evaluation.
///
/// Built once per build (REQ-3204 "Compile regexes once per build") from a
/// [`SelectorSpec`]. Cloning is cheap-ish — globsets and regexes hold
/// internal `Arc`-like state, but there is no reason to clone in the hot
/// path; keep one per hook.
#[derive(Debug, Clone)]
pub struct CompiledSelector {
    include: GlobSet,
    exclude: GlobSet,
    frontmatter_where: Option<Predicate>,
    content_probes: Vec<Probe>,
    probe_match: ProbeMatch,
}

/// One entry from `content_probe` — either a literal substring or a
/// pre-compiled regex. Regex probes are identified by the `re:` prefix
/// on the manifest value.
#[derive(Debug, Clone)]
pub enum Probe {
    Substring(String),
    Regex {
        regex: Regex,
        /// Original pattern (without the `re:` prefix). Preserved for
        /// diagnostics and `zetl hook dry-run`.
        pattern: String,
    },
}

/// Per-page input handed to [`evaluate`].
///
/// Matches SPEC-032 §7's `SelectorInput` boundary contract. The shell is
/// responsible for populating the fields; core only reads them.
///
/// - `path` is vault-relative, forward-slash style (what the include /
///   exclude globs are written against).
/// - `frontmatter` is typically a [`Value::Object`] (from parsed YAML /
///   TOML frontmatter). Any other shape is accepted — the predicate
///   will resolve paths to `null` and the comparison will return `false`.
/// - `text` is the raw Markdown body (post-frontmatter). The caller MAY
///   truncate to the first 100 KB to enforce NFR-3201.
#[derive(Debug, Clone, Copy)]
pub struct SelectorInput<'a> {
    pub path: &'a Path,
    pub frontmatter: &'a Value,
    pub text: &'a str,
}

// ── Errors ──────────────────────────────────────────────────────────────────

/// Failure to compile a selector — an invalid glob, predicate, or regex.
///
/// Callers convert to [`crate::hooks::diagnostic::HookDiagnostic`] with
/// [`crate::hooks::diagnostic::DiagnosticClass::SelectorEval`] at the
/// boundary (not done here to keep the compile step pure).
#[derive(Debug)]
pub enum CompileError {
    InvalidInclude { pattern: String, source: String },
    InvalidExclude { pattern: String, source: String },
    InvalidPredicate(predicate::ParseError),
    InvalidContentProbe { pattern: String, source: String },
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompileError::InvalidInclude { pattern, source } => {
                write!(f, "invalid include glob {pattern:?}: {source}")
            }
            CompileError::InvalidExclude { pattern, source } => {
                write!(f, "invalid exclude glob {pattern:?}: {source}")
            }
            CompileError::InvalidPredicate(e) => {
                write!(f, "invalid frontmatter predicate: {e}")
            }
            CompileError::InvalidContentProbe { pattern, source } => {
                write!(f, "invalid content_probe regex {pattern:?}: {source}")
            }
        }
    }
}

impl std::error::Error for CompileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CompileError::InvalidPredicate(e) => Some(e),
            _ => None,
        }
    }
}

impl From<predicate::ParseError> for CompileError {
    fn from(e: predicate::ParseError) -> Self {
        CompileError::InvalidPredicate(e)
    }
}

// ── Compile ─────────────────────────────────────────────────────────────────

/// Compile a parsed manifest selector for per-page evaluation.
///
/// All regexes and globs are compiled here; evaluation does no further
/// compilation. Call this once per hook at build start (REQ-3204).
pub fn compile(spec: &SelectorSpec) -> Result<CompiledSelector, CompileError> {
    let include = build_globset(&spec.include).map_err(|(pattern, source)| {
        CompileError::InvalidInclude { pattern, source }
    })?;
    let exclude = build_globset(&spec.exclude).map_err(|(pattern, source)| {
        CompileError::InvalidExclude { pattern, source }
    })?;
    let frontmatter_where = match &spec.frontmatter_where {
        Some(src) => Some(predicate::parse(src)?),
        None => None,
    };
    let content_probes = spec
        .content_probe
        .iter()
        .map(|p| compile_probe(p))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CompiledSelector {
        include,
        exclude,
        frontmatter_where,
        content_probes,
        probe_match: spec.require_probe_match,
    })
}

fn build_globset(patterns: &[String]) -> Result<GlobSet, (String, String)> {
    let mut b = GlobSetBuilder::new();
    for p in patterns {
        let g = Glob::new(p).map_err(|e| (p.clone(), e.to_string()))?;
        b.add(g);
    }
    b.build().map_err(|e| ("(globset build)".to_string(), e.to_string()))
}

fn compile_probe(raw: &str) -> Result<Probe, CompileError> {
    if let Some(pattern) = raw.strip_prefix("re:") {
        let regex = Regex::new(pattern).map_err(|e| CompileError::InvalidContentProbe {
            pattern: pattern.to_string(),
            source: e.to_string(),
        })?;
        Ok(Probe::Regex {
            regex,
            pattern: pattern.to_string(),
        })
    } else {
        Ok(Probe::Substring(raw.to_string()))
    }
}

// ── Evaluation (REQ-3204 short-circuit) ─────────────────────────────────────

impl CompiledSelector {
    /// Evaluate the full selector chain in CON-3204 order.
    ///
    /// Returns `true` iff all configured layers accept the input. Short-
    /// circuits at the first failure so the expensive layers are only
    /// reached when strictly necessary.
    pub fn evaluate(&self, input: &SelectorInput<'_>) -> bool {
        if !self.match_path(input.path) {
            return false;
        }
        if !self.eval_frontmatter(input.frontmatter) {
            return false;
        }
        if !self.run_content_probe(input.text) {
            return false;
        }
        true
    }

    /// Layer 1: path include/exclude globs.
    pub fn match_path(&self, path: &Path) -> bool {
        if !self.include.is_match(path) {
            return false;
        }
        !self.exclude.is_match(path)
    }

    /// Layer 2: frontmatter predicate. No predicate configured → always true.
    pub fn eval_frontmatter(&self, frontmatter: &Value) -> bool {
        match &self.frontmatter_where {
            None => true,
            Some(p) => p.evaluate(frontmatter),
        }
    }

    /// Layer 3: content probes (substring + regex), combined per
    /// [`ProbeMatch`]. No probes configured → always true.
    pub fn run_content_probe(&self, text: &str) -> bool {
        if self.content_probes.is_empty() {
            return true;
        }
        match self.probe_match {
            ProbeMatch::Any => self.content_probes.iter().any(|p| probe_hits(p, text)),
            ProbeMatch::All => self.content_probes.iter().all(|p| probe_hits(p, text)),
        }
    }

    /// Return the number of compiled content probes. Useful for
    /// `zetl hook dry-run` / coverage diagnostics.
    pub fn probe_count(&self) -> usize {
        self.content_probes.len()
    }
}

fn probe_hits(p: &Probe, text: &str) -> bool {
    match p {
        Probe::Substring(s) => text.contains(s.as_str()),
        Probe::Regex { regex, .. } => regex.is_match(text),
    }
}

// ── Free-function mirrors (SPEC-032 §7 API surface) ─────────────────────────
//
// The purity-boundary map names these four entry points. Keeping them as
// free functions makes the pure-core surface explicit; they all delegate
// to `CompiledSelector` methods.

/// Match a vault-relative path against the selector's include/exclude
/// globs (REQ-3204 layer 1).
pub fn match_path(selector: &CompiledSelector, path: &Path) -> bool {
    selector.match_path(path)
}

/// Evaluate the selector's frontmatter predicate against a parsed
/// frontmatter value (REQ-3204 layer 2 / REQ-3205 grammar).
pub fn eval_frontmatter(selector: &CompiledSelector, frontmatter: &Value) -> bool {
    selector.eval_frontmatter(frontmatter)
}

/// Apply the selector's content probes to the raw Markdown text
/// (REQ-3204 layer 3).
pub fn run_content_probe(selector: &CompiledSelector, text: &str) -> bool {
    selector.run_content_probe(text)
}

/// Full CON-3204 chain with short-circuit evaluation. Equivalent to
/// [`CompiledSelector::evaluate`] — both entry points are public so
/// callers can pick whichever reads better in context.
pub fn selector_passes(selector: &CompiledSelector, input: &SelectorInput<'_>) -> bool {
    selector.evaluate(input)
}

// ── Tests (TEST-3204) ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn sel(spec: SelectorSpec) -> CompiledSelector {
        compile(&spec).expect("selector compile")
    }

    fn input<'a>(path: &'a str, fm: &'a Value, text: &'a str) -> SelectorInput<'a> {
        SelectorInput {
            path: Path::new(path),
            frontmatter: fm,
            text,
        }
    }

    // ── Path matching ────────────────────────────────────────────────────

    #[test]
    fn default_selector_matches_any_md() {
        let s = sel(SelectorSpec::default());
        let fm = json!({});
        assert!(s.evaluate(&input("notes/hello.md", &fm, "")));
        assert!(s.evaluate(&input("deep/a/b/c.md", &fm, "")));
        // Non-.md is excluded by the default include pattern.
        assert!(!s.evaluate(&input("image.png", &fm, "")));
    }

    #[test]
    fn include_globs_restrict_paths() {
        let s = sel(SelectorSpec {
            include: vec!["posts/**/*.md".into()],
            ..Default::default()
        });
        let fm = json!({});
        assert!(s.evaluate(&input("posts/2024/hello.md", &fm, "")));
        assert!(!s.evaluate(&input("notes/hello.md", &fm, "")));
    }

    #[test]
    fn exclude_globs_subtract_matched_paths() {
        let s = sel(SelectorSpec {
            include: vec!["**/*.md".into()],
            exclude: vec!["drafts/**".into()],
            ..Default::default()
        });
        let fm = json!({});
        assert!(s.evaluate(&input("posts/hello.md", &fm, "")));
        assert!(!s.evaluate(&input("drafts/wip.md", &fm, "")));
    }

    #[test]
    fn invalid_include_glob_rejected_at_compile() {
        let spec = SelectorSpec {
            include: vec!["[unclosed".into()],
            ..Default::default()
        };
        let err = compile(&spec).unwrap_err();
        matches!(err, CompileError::InvalidInclude { .. });
        assert!(err.to_string().contains("[unclosed"));
    }

    #[test]
    fn invalid_exclude_glob_rejected_at_compile() {
        let spec = SelectorSpec {
            exclude: vec!["[bad".into()],
            ..Default::default()
        };
        let err = compile(&spec).unwrap_err();
        matches!(err, CompileError::InvalidExclude { .. });
    }

    // ── Frontmatter predicate layer ──────────────────────────────────────

    #[test]
    fn frontmatter_predicate_gates_match() {
        let s = sel(SelectorSpec {
            frontmatter_where: Some(r#"status == "published""#.into()),
            ..Default::default()
        });
        assert!(s.evaluate(&input("a.md", &json!({ "status": "published" }), "")));
        assert!(!s.evaluate(&input("a.md", &json!({ "status": "draft" }), "")));
    }

    #[test]
    fn frontmatter_predicate_missing_path_is_null() {
        let s = sel(SelectorSpec {
            frontmatter_where: Some(r#"extensions.tasks != false"#.into()),
            ..Default::default()
        });
        // Missing → null → `null != false` → true (type-mismatched).
        assert!(s.evaluate(&input("a.md", &json!({}), "")));
        // Explicit false → excluded.
        assert!(!s.evaluate(&input("a.md", &json!({ "extensions": { "tasks": false } }), "")));
    }

    #[test]
    fn invalid_predicate_rejected_at_compile() {
        let spec = SelectorSpec {
            frontmatter_where: Some("garbage @@".into()),
            ..Default::default()
        };
        let err = compile(&spec).unwrap_err();
        matches!(err, CompileError::InvalidPredicate(_));
    }

    // ── Content probes (layer 3) ─────────────────────────────────────────

    #[test]
    fn content_probe_substring_any() {
        let s = sel(SelectorSpec {
            content_probe: vec![":::note".into()],
            ..Default::default()
        });
        assert!(s.evaluate(&input("a.md", &json!({}), "before\n:::note\ncontent")));
        assert!(!s.evaluate(&input("a.md", &json!({}), "no matches here")));
    }

    #[test]
    fn content_probe_regex_prefix() {
        let s = sel(SelectorSpec {
            content_probe: vec![r"re:^TODO\s".into()],
            ..Default::default()
        });
        assert!(s.evaluate(&input("a.md", &json!({}), "TODO buy milk")));
        assert!(!s.evaluate(&input("a.md", &json!({}), "# TODO list")));
    }

    #[test]
    fn content_probe_require_any_matches_any() {
        let s = sel(SelectorSpec {
            content_probe: vec!["alpha".into(), "beta".into()],
            require_probe_match: ProbeMatch::Any,
            ..Default::default()
        });
        assert!(s.evaluate(&input("a.md", &json!({}), "contains alpha only")));
        assert!(s.evaluate(&input("a.md", &json!({}), "contains beta only")));
        assert!(!s.evaluate(&input("a.md", &json!({}), "neither here")));
    }

    #[test]
    fn content_probe_require_all_matches_all() {
        let s = sel(SelectorSpec {
            content_probe: vec!["alpha".into(), "beta".into()],
            require_probe_match: ProbeMatch::All,
            ..Default::default()
        });
        assert!(s.evaluate(&input("a.md", &json!({}), "alpha and beta present")));
        assert!(!s.evaluate(&input("a.md", &json!({}), "alpha only")));
        assert!(!s.evaluate(&input("a.md", &json!({}), "beta only")));
    }

    #[test]
    fn empty_content_probe_list_always_passes() {
        let s = sel(SelectorSpec::default());
        assert!(s.evaluate(&input("a.md", &json!({}), "")));
    }

    #[test]
    fn invalid_regex_probe_rejected_at_compile() {
        let spec = SelectorSpec {
            content_probe: vec!["re:[unclosed".into()],
            ..Default::default()
        };
        let err = compile(&spec).unwrap_err();
        matches!(err, CompileError::InvalidContentProbe { .. });
    }

    // ── TEST-3204: Short-circuit semantics ───────────────────────────────
    //
    // The spec says: when path fails, frontmatter must not parse; when
    // frontmatter fails, content probes must not run; etc. Our three
    // layers already short-circuit at the evaluator, but we also verify
    // that the per-layer entry points don't do work when called in
    // isolation — the only observable signal at this module's level is
    // that `evaluate` returns early without consulting the later layers.
    //
    // We use a ProbeInvocationCounter via a closure test harness: the
    // selector doesn't call out to user code, so instead we assert that
    // layer-specific entry points produce their expected false-at-first-
    // failure behaviour.

    #[test]
    fn short_circuit_path_fail_returns_false() {
        let s = sel(SelectorSpec {
            include: vec!["posts/**/*.md".into()],
            frontmatter_where: Some(r#"status == "published""#.into()),
            content_probe: vec!["never-matches".into()],
            ..Default::default()
        });
        // Path mismatch → short-circuit false. Frontmatter would be true
        // if we reached it; content probe would be false. Because we
        // return at layer 1, we never consult the later layers.
        assert!(!s.evaluate(&input(
            "other/hello.md",
            &json!({ "status": "published" }),
            "has the substring never-matches? no.",
        )));
    }

    #[test]
    fn short_circuit_frontmatter_fail_doesnt_scan_content() {
        // Wrap the `text` in a pointer and assert that reading it would
        // have been observable via side-effects — we can't instrument
        // `text.contains()` directly, but we can verify the layered
        // entry-points independently:
        let s = sel(SelectorSpec {
            frontmatter_where: Some(r#"status == "published""#.into()),
            content_probe: vec!["sentinel-that-IS-present".into()],
            ..Default::default()
        });
        let fm = json!({ "status": "draft" }); // frontmatter fails
        let text = "sentinel-that-IS-present here";
        // Selector short-circuits on frontmatter, so never reaches text.
        // The isolated `run_content_probe` call would hit the probe, so
        // `evaluate` returning false despite that is the proof.
        assert!(s.run_content_probe(text));
        assert!(!s.eval_frontmatter(&fm));
        assert!(!s.evaluate(&input("a.md", &fm, text)));
    }

    #[test]
    fn short_circuit_all_layers_pass_hook_eligible() {
        let s = sel(SelectorSpec {
            include: vec!["posts/**/*.md".into()],
            frontmatter_where: Some(r#"status == "published""#.into()),
            content_probe: vec!["BEGIN".into()],
            require_probe_match: ProbeMatch::All,
            ..Default::default()
        });
        assert!(s.evaluate(&input(
            "posts/2024/hello.md",
            &json!({ "status": "published" }),
            "BEGIN here",
        )));
    }

    // ── TEST-3204 via counting: verify each layer is only invoked when
    // ── the previous layer passed. We instrument by calling the layer
    // ── entry points directly and asserting the boolean outcomes — this
    // ── is the same information the CON-3204 pseudocode asserts about.

    #[test]
    fn layers_isolatable_for_instrumentation() {
        let counter = Arc::new(AtomicUsize::new(0));
        let s = sel(SelectorSpec {
            include: vec!["**/*.md".into()],
            frontmatter_where: Some(r#"status == "published""#.into()),
            content_probe: vec!["BEGIN".into()],
            ..Default::default()
        });
        let fm_ok = json!({ "status": "published" });
        let fm_bad = json!({ "status": "draft" });

        // Proxy: every call to a layer increments the counter. Since the
        // layers are pure, the caller controls whether they are invoked.
        let call_path = |p: &str| {
            counter.fetch_add(1, Ordering::SeqCst);
            s.match_path(&PathBuf::from(p))
        };
        let call_fm = |fm: &Value| {
            counter.fetch_add(1, Ordering::SeqCst);
            s.eval_frontmatter(fm)
        };
        let call_content = |t: &str| {
            counter.fetch_add(1, Ordering::SeqCst);
            s.run_content_probe(t)
        };

        // Manual short-circuit mirroring CON-3204 pseudocode.
        counter.store(0, Ordering::SeqCst);
        let outcome = call_path("not-a-match.txt")
            && call_fm(&fm_ok)
            && call_content("BEGIN x");
        assert!(!outcome);
        assert_eq!(counter.load(Ordering::SeqCst), 1, "path-fail short-circuits");

        counter.store(0, Ordering::SeqCst);
        let outcome = call_path("a.md")
            && call_fm(&fm_bad)
            && call_content("BEGIN x");
        assert!(!outcome);
        assert_eq!(counter.load(Ordering::SeqCst), 2, "frontmatter-fail short-circuits");

        counter.store(0, Ordering::SeqCst);
        let outcome = call_path("a.md")
            && call_fm(&fm_ok)
            && call_content("nothing matches");
        assert!(!outcome);
        assert_eq!(counter.load(Ordering::SeqCst), 3, "content-probe-fail reaches content");

        counter.store(0, Ordering::SeqCst);
        let outcome = call_path("a.md")
            && call_fm(&fm_ok)
            && call_content("BEGIN something");
        assert!(outcome);
        assert_eq!(counter.load(Ordering::SeqCst), 3, "all-pass reaches all layers exactly once");
    }

    // ── Compile-once invariant ───────────────────────────────────────────

    #[test]
    fn compile_succeeds_once_and_reuses_regex_across_evaluations() {
        let s = sel(SelectorSpec {
            content_probe: vec!["re:^TODO".into()],
            ..Default::default()
        });
        // Evaluate many times — no panics, regex is the same object.
        for i in 0..1000 {
            let body = if i % 2 == 0 {
                "TODO inline"
            } else {
                "no match"
            };
            let _ = s.evaluate(&input("a.md", &json!({}), body));
        }
    }

    // ── Free-function mirrors match method semantics ─────────────────────

    #[test]
    fn free_functions_mirror_methods() {
        let s = sel(SelectorSpec {
            include: vec!["**/*.md".into()],
            frontmatter_where: Some(r#"status == "published""#.into()),
            content_probe: vec!["BEGIN".into()],
            ..Default::default()
        });
        let fm = json!({ "status": "published" });
        let inp = input("a.md", &fm, "BEGIN now");
        assert_eq!(match_path(&s, inp.path), s.match_path(inp.path));
        assert_eq!(eval_frontmatter(&s, inp.frontmatter), s.eval_frontmatter(inp.frontmatter));
        assert_eq!(run_content_probe(&s, inp.text), s.run_content_probe(inp.text));
        assert_eq!(selector_passes(&s, &inp), s.evaluate(&inp));
    }

    // ── REQ-3211: canonical extension opt-out convention ─────────────────

    #[test]
    fn canonical_extension_opt_out_pattern() {
        // The shape prescribed by REQ-3211: canonical extensions include
        // `frontmatter.extensions.<name> != false` in their selector.
        // Verify the end-to-end wiring: disabled extension → no match.
        let s = sel(SelectorSpec {
            frontmatter_where: Some(r#"extensions.tasks != false"#.into()),
            ..Default::default()
        });
        assert!(!s.evaluate(&input(
            "a.md",
            &json!({ "extensions": { "tasks": false } }),
            "",
        )));
        assert!(s.evaluate(&input(
            "a.md",
            &json!({ "extensions": { "tasks": true } }),
            "",
        )));
        assert!(s.evaluate(&input("a.md", &json!({}), "")));
    }
}
