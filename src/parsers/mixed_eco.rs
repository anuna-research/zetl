//! Mixed-parser diagnostic (SPEC-033 REQ-3315 / TEST-3315).
//!
//! A page can declare its Markdown parser via frontmatter
//! (`parser: pandoc`), a `.ztl/config.toml` `[[parse.rule]]` glob, or
//! the vault-wide `[parse] default`. Independently, hooks can declare
//! an ecosystem (`ecosystem = "mdbook"`). Each ecosystem implicitly
//! expects pages to have been produced by a specific parser:
//!
//! - `pandoc` ecosystem filters operate on pandoc-types AST — they
//!   expect pages parsed by the `pandoc` parser.
//! - `mdbook` preprocessors and `remark` plugins operate on
//!   CommonMark-derived shapes — they expect pages parsed by the
//!   `commonmark` parser.
//!
//! When a hook's selector matches a page AND the page's resolved
//! parser is not what the hook's ecosystem expects, the resulting
//! HTML is undefined: the hook sees an AST that doesn't match the
//! semantics of its ecosystem. Rather than silently producing wrong
//! output, ztl detects the mismatch and:
//!
//! 1. emits a five-part [`HookDiagnostic`] citing both the page's
//!    parser and the hook's ecosystem, naming the resolution;
//! 2. refuses to run that hook on that page (the caller drops the
//!    violating (page, hook) pair before dispatch);
//! 3. under `ztl build --strict-parsers`, escalates the warning
//!    to a build failure — the CI gate for mixed-parser vaults.
//!
//! ## Config-time vs build-time
//!
//! The detector is pure: it takes a list of pages (with their
//! resolved parser) and a list of composed hooks (with their
//! compiled selectors), and returns the set of (page, hook)
//! violations. Both `ztl build` (build-time) and
//! `ztl ecosystem check` (config-time) call into the same entry
//! point so the two surfaces never drift.

use std::path::{Path, PathBuf};

use crate::hooks::composition::ComposedHook;
use crate::hooks::diagnostic::{DiagnosticClass, HookDiagnostic};
use crate::hooks::pipeline::Stage;
use crate::hooks::selector::{CompiledSelector, SelectorInput};

/// The parser an ecosystem adapter implicitly expects its pages to
/// have been produced by. Returns `None` for unknown or adapter-less
/// ecosystem ids (those are surfaced as separate "unknown ecosystem"
/// diagnostics elsewhere).
///
/// The mapping is fixed by what each ecosystem's plugins consume:
/// - Pandoc filters consume pandoc-types JSON → the `pandoc` parser
///   is the only one that produces it faithfully.
/// - mdBook preprocessors consume CommonMark book chapters → the
///   `commonmark` parser is the only one that preserves CommonMark
///   semantics (fenced divs, attribute blocks, etc. mean different
///   things under Pandoc).
/// - remark plugins consume mdast, which is a CommonMark-derived AST
///   → same reasoning as mdBook.
pub fn ecosystem_expected_parser(ecosystem_id: &str) -> Option<&'static str> {
    match ecosystem_id {
        "pandoc" => Some("pandoc"),
        "mdbook" => Some("commonmark"),
        "remark" => Some("commonmark"),
        _ => None,
    }
}

/// One (page, hook) pair where the page's resolved parser does not
/// match the parser the hook's ecosystem expects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MixedParserViolation {
    /// Vault-relative page path.
    pub page_path: PathBuf,
    /// Parser id resolved for this page (frontmatter / rule / default).
    pub page_parser: String,
    /// Hook's pipeline stage (for diagnostic context).
    pub stage: Stage,
    /// Hook's extension id (for diagnostic context and logs).
    pub hook_id: String,
    /// Hook's ecosystem id (always `Some` by construction — a hook
    /// with no ecosystem can never produce a mixed-parser violation).
    pub hook_ecosystem: String,
    /// Parser the hook's ecosystem expects (from
    /// [`ecosystem_expected_parser`]).
    pub expected_parser: String,
}

impl MixedParserViolation {
    /// Render the violation as a [`HookDiagnostic`]. The diagnostic
    /// class is [`DiagnosticClass::ContractViolation`] — a mixed-parser
    /// config violates an implicit contract between the page and the
    /// ecosystem adapter.
    ///
    /// The rendered output cites both parsers and names the
    /// resolution (pick one parser, or move the page out of the
    /// hook's selector scope).
    pub fn to_diagnostic(&self) -> HookDiagnostic {
        HookDiagnostic::new(
            DiagnosticClass::ContractViolation,
            format!("mixed-parser configuration on {}", self.page_path.display()),
        )
        .with_context(format!(
            "page parser: {} (resolved from frontmatter / [parse] config)",
            self.page_parser
        ))
        .with_context(format!(
            "hook '{}' ({}): ecosystem = \"{}\" (expects parser \"{}\")",
            self.hook_id, self.stage, self.hook_ecosystem, self.expected_parser
        ))
        .with_observed(format!(
            "selected parser \"{}\" ≠ expected parser \"{}\"",
            self.page_parser, self.expected_parser
        ))
        .with_cause(
            "ecosystem adapters operate on their native AST shape; running them \n\
             on a page produced by a different parser yields undefined output.",
        )
        .with_hint(format!(
            "pick one: set `parser: {expected}` in the page's frontmatter, \n\
             move the page out of the hook's selector scope, \n\
             or remove / disable the '{hook}' hook for pages using \"{actual}\". \n\
             Under `ztl build --strict-parsers` this warning is fatal.",
            expected = self.expected_parser,
            hook = self.hook_id,
            actual = self.page_parser,
        ))
    }
}

/// Summary of a `detect_mixed_parsers` pass.
///
/// Violations are yielded in a byte-stable order: by vault-relative
/// page path, then by stage, then by hook id. That lets integration
/// tests snapshot the output.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MixedParserReport {
    pub violations: Vec<MixedParserViolation>,
}

impl MixedParserReport {
    pub fn is_empty(&self) -> bool {
        self.violations.is_empty()
    }
}

/// A page paired with its resolved parser and the inputs a
/// [`CompiledSelector`] needs to evaluate (frontmatter JSON value,
/// raw body text).
///
/// The caller builds this once per vault scan; the detector consumes
/// it without further I/O.
pub struct PageForDetection<'a> {
    pub path: &'a Path,
    pub parser: String,
    pub frontmatter: &'a serde_json::Value,
    pub body: &'a str,
}

/// A composed hook with its selector pre-compiled.
///
/// The caller compiles the selector once per hook (REQ-3204 "compile
/// once per build") before passing it in.
pub struct HookForDetection<'a> {
    pub stage: Stage,
    pub hook_id: &'a str,
    pub ecosystem: &'a str,
    pub selector: &'a CompiledSelector,
}

/// Walk every (page, hook) pair and record violations where the
/// hook's ecosystem expects a different parser than the page
/// resolved to, and the hook's selector matches the page.
///
/// Hooks whose ecosystem id is unknown to [`ecosystem_expected_parser`]
/// are skipped — they are not mixed-parser violations, just plain
/// unknown ecosystems (surfaced elsewhere at dispatch time).
pub fn detect_mixed_parsers(
    pages: &[PageForDetection<'_>],
    hooks: &[HookForDetection<'_>],
) -> MixedParserReport {
    let mut violations = Vec::new();

    for page in pages {
        for hook in hooks {
            let Some(expected) = ecosystem_expected_parser(hook.ecosystem) else {
                continue;
            };
            if expected == page.parser {
                continue;
            }
            let input = SelectorInput {
                path: page.path,
                frontmatter: page.frontmatter,
                text: page.body,
            };
            if !hook.selector.evaluate(&input) {
                continue;
            }
            violations.push(MixedParserViolation {
                page_path: page.path.to_path_buf(),
                page_parser: page.parser.clone(),
                stage: hook.stage,
                hook_id: hook.hook_id.to_string(),
                hook_ecosystem: hook.ecosystem.to_string(),
                expected_parser: expected.to_string(),
            });
        }
    }

    violations.sort_by(|a, b| {
        a.page_path
            .cmp(&b.page_path)
            .then_with(|| format!("{}", a.stage).cmp(&format!("{}", b.stage)))
            .then_with(|| a.hook_id.cmp(&b.hook_id))
    });

    MixedParserReport { violations }
}

/// Format every violation in a report as a single concatenated
/// diagnostic string (one rendered [`HookDiagnostic`] per violation,
/// separated by blank lines). Default verbosity.
pub fn format_report(report: &MixedParserReport) -> String {
    let mut out = String::new();
    for (i, v) in report.violations.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&v.to_diagnostic().to_string());
    }
    out
}

/// Helper for callers that already hold a [`ComposedHook`]: extract
/// the hook's ecosystem id (or `None` for ztl-native hooks).
pub fn hook_ecosystem(hook: &ComposedHook) -> Option<&str> {
    hook.ecosystem.as_deref()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::manifest::{ProbeMatch, SelectorSpec};
    use crate::hooks::selector::compile;
    use serde_json::json;
    use std::path::PathBuf;

    fn make_selector(include: &[&str]) -> CompiledSelector {
        let spec = SelectorSpec {
            include: include.iter().map(|s| s.to_string()).collect(),
            exclude: vec![],
            frontmatter_where: None,
            content_probe: vec![],
            require_probe_match: ProbeMatch::Any,
        };
        compile(&spec).unwrap()
    }

    #[test]
    fn mapping_covers_all_v1_ecosystems() {
        assert_eq!(ecosystem_expected_parser("pandoc"), Some("pandoc"));
        assert_eq!(ecosystem_expected_parser("mdbook"), Some("commonmark"));
        assert_eq!(ecosystem_expected_parser("remark"), Some("commonmark"));
        assert_eq!(ecosystem_expected_parser("djot"), None);
        assert_eq!(ecosystem_expected_parser(""), None);
    }

    #[test]
    fn commonmark_page_with_pandoc_hook_is_a_violation() {
        let selector = make_selector(&["**/*.md"]);
        let fm = json!({});
        let page = PageForDetection {
            path: Path::new("notes/page.md"),
            parser: "commonmark".to_string(),
            frontmatter: &fm,
            body: "# hi",
        };
        let hook = HookForDetection {
            stage: Stage::Transform,
            hook_id: "my-filter",
            ecosystem: "pandoc",
            selector: &selector,
        };
        let report = detect_mixed_parsers(&[page], &[hook]);
        assert_eq!(report.violations.len(), 1);
        let v = &report.violations[0];
        assert_eq!(v.page_parser, "commonmark");
        assert_eq!(v.expected_parser, "pandoc");
        assert_eq!(v.hook_ecosystem, "pandoc");
    }

    #[test]
    fn pandoc_page_with_mdbook_hook_is_a_violation_acceptance_case() {
        // TEST-3315 acceptance example: page with `parser: pandoc`
        // and an ecosystem-mdbook hook selector matching → diagnostic
        // cites both and names the resolution.
        let selector = make_selector(&["**/*.md"]);
        let fm = json!({ "parser": "pandoc" });
        let page = PageForDetection {
            path: Path::new("papers/whitepaper.md"),
            parser: "pandoc".to_string(),
            frontmatter: &fm,
            body: "body",
        };
        let hook = HookForDetection {
            stage: Stage::PreParse,
            hook_id: "book-index",
            ecosystem: "mdbook",
            selector: &selector,
        };
        let report = detect_mixed_parsers(&[page], &[hook]);
        assert_eq!(report.violations.len(), 1);

        let rendered = format_report(&report);
        // Cites page parser, hook ecosystem, expected parser.
        assert!(rendered.contains("papers/whitepaper.md"), "{rendered}");
        assert!(rendered.contains("pandoc"), "{rendered}");
        assert!(rendered.contains("mdbook"), "{rendered}");
        assert!(rendered.contains("commonmark"), "{rendered}");
        // Names the resolution.
        assert!(rendered.contains("Hint:"), "{rendered}");
        assert!(rendered.contains("strict-parsers"), "{rendered}");
    }

    #[test]
    fn matching_parser_emits_no_violation() {
        let selector = make_selector(&["**/*.md"]);
        let fm = json!({ "parser": "pandoc" });
        let page = PageForDetection {
            path: Path::new("papers/x.md"),
            parser: "pandoc".to_string(),
            frontmatter: &fm,
            body: "",
        };
        let hook = HookForDetection {
            stage: Stage::Transform,
            hook_id: "pd",
            ecosystem: "pandoc",
            selector: &selector,
        };
        let report = detect_mixed_parsers(&[page], &[hook]);
        assert!(report.is_empty());
    }

    #[test]
    fn selector_miss_suppresses_violation() {
        let selector = make_selector(&["papers/**"]);
        let fm = json!({});
        let page = PageForDetection {
            path: Path::new("notes/x.md"),
            parser: "commonmark".to_string(),
            frontmatter: &fm,
            body: "",
        };
        let hook = HookForDetection {
            stage: Stage::Transform,
            hook_id: "pd",
            ecosystem: "pandoc",
            selector: &selector,
        };
        let report = detect_mixed_parsers(&[page], &[hook]);
        assert!(report.is_empty());
    }

    #[test]
    fn unknown_ecosystem_is_skipped_not_reported() {
        let selector = make_selector(&["**/*.md"]);
        let fm = json!({});
        let page = PageForDetection {
            path: Path::new("x.md"),
            parser: "commonmark".to_string(),
            frontmatter: &fm,
            body: "",
        };
        let hook = HookForDetection {
            stage: Stage::Transform,
            hook_id: "djot-hook",
            ecosystem: "djot", // not in the registry
            selector: &selector,
        };
        let report = detect_mixed_parsers(&[page], &[hook]);
        assert!(report.is_empty());
    }

    #[test]
    fn violations_are_sorted_byte_stably() {
        let selector = make_selector(&["**/*.md"]);
        let fm = json!({});
        let pages = vec![
            PageForDetection {
                path: Path::new("z.md"),
                parser: "pandoc".to_string(),
                frontmatter: &fm,
                body: "",
            },
            PageForDetection {
                path: Path::new("a.md"),
                parser: "pandoc".to_string(),
                frontmatter: &fm,
                body: "",
            },
        ];
        let hooks = vec![
            HookForDetection {
                stage: Stage::Transform,
                hook_id: "zz",
                ecosystem: "mdbook",
                selector: &selector,
            },
            HookForDetection {
                stage: Stage::Transform,
                hook_id: "aa",
                ecosystem: "mdbook",
                selector: &selector,
            },
        ];
        let report = detect_mixed_parsers(&pages, &hooks);
        let paths: Vec<PathBuf> = report
            .violations
            .iter()
            .map(|v| v.page_path.clone())
            .collect();
        assert_eq!(
            paths,
            vec![
                PathBuf::from("a.md"),
                PathBuf::from("a.md"),
                PathBuf::from("z.md"),
                PathBuf::from("z.md")
            ]
        );
        // Within same path, sorted by hook_id.
        assert_eq!(report.violations[0].hook_id, "aa");
        assert_eq!(report.violations[1].hook_id, "zz");
    }

    #[test]
    fn hook_ecosystem_extracts_from_composed_hook() {
        use crate::hooks::composition::{ComposedHook, CompositionSource};
        use crate::hooks::translators::AstType;

        let h = ComposedHook {
            stage: Stage::Transform,
            filename: "f.py".to_string(),
            extension_id: "f".to_string(),
            path: PathBuf::from("/nowhere"),
            manifest_path: None,
            source: CompositionSource::Vault,
            before: vec![],
            after: vec![],
            optional: false,
            ast_type: AstType::default(),
            ast_version: None,
            preserves: vec![],
            ecosystem: Some("mdbook".to_string()),
            disabled: None,
        };
        assert_eq!(hook_ecosystem(&h), Some("mdbook"));

        let h2 = ComposedHook {
            ecosystem: None,
            ..h
        };
        assert_eq!(hook_ecosystem(&h2), None);
    }
}
