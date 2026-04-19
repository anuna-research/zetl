//! Integration coverage for SPEC-032 REQ-3224 / CON-3224 — behavioural
//! contract declarations and their enforcement.
//!
//! Validates TEST-3224's preservation-diagnostic matrix, TEST-3222's
//! `may_restructure` block-tree diff, and TEST-3224-idempotent's CI
//! double-run check by wiring the [`zetl::hooks::contract`] validators
//! around fixture hooks. The property-test harness is exercised at the
//! end as the user-facing entry point.

use zetl::hooks::ast::{
    Block, BlockQuote, Document, DocumentKind, Embed, Inline, Paragraph, Position, Text,
    Wikilink, AST_VERSION,
};
use zetl::hooks::contract::{
    canonicalise, run_property_test, validate_idempotence, validate_may_restructure,
    validate_preserves, ContractSubReason, PropertyTestCase,
};
use zetl::hooks::failure_scoping::FailureReason;
use zetl::hooks::manifest::ContractDecl;
use zetl::hooks::pipeline::Stage;

// ── Fixture builders ────────────────────────────────────────────────────

fn doc(children: Vec<Block>) -> Document {
    Document {
        ast_version: AST_VERSION.to_string(),
        kind: DocumentKind::Document,
        position: Position::origin(),
        frontmatter: None,
        children,
    }
}

fn para(children: Vec<Inline>) -> Block {
    Block::Paragraph(Paragraph {
        position: Position::origin(),
        children,
    })
}

fn text(s: &str) -> Inline {
    Inline::Text(Text {
        position: Position::origin(),
        text: s.to_string(),
    })
}

fn wikilink(target: &str) -> Inline {
    Inline::Wikilink(Wikilink {
        position: Position::origin(),
        target: target.to_string(),
        alias: None,
        heading: None,
        block_id: None,
    })
}

fn embed(target: &str) -> Block {
    Block::Embed(Embed {
        position: Position::origin(),
        target: target.to_string(),
        heading: None,
        block_id: None,
    })
}

// ── TEST-3224: preservation diagnostic matrix ───────────────────────────

/// Matrix row: empty preserves list → no enforcement, even when the
/// hook strips every wikilink.
#[test]
fn test_3224_empty_preserves_no_enforcement() {
    let input = doc(vec![para(vec![wikilink("A"), wikilink("B")])]);
    let output = doc(vec![para(vec![text("x")])]);
    let v = validate_preserves(Stage::Transform, "stripper", "p", &[], &input, &output);
    assert!(v.is_empty());
}

/// Matrix row: single preserved type (Wikilink) — hook that drops
/// wikilinks produces a `contract_violation:preserves` finding
/// enumerating count delta and page slug.
#[test]
fn test_3224_single_preserved_type_reports_count_delta() {
    let input = doc(vec![para(vec![
        wikilink("A"),
        text(" "),
        wikilink("B"),
        text(" "),
        wikilink("C"),
    ])]);
    let output = doc(vec![para(vec![text("x y z")])]);

    let v = validate_preserves(
        Stage::Transform,
        "callouts",
        "projects/q2-review",
        &["Wikilink".to_string()],
        &input,
        &output,
    );
    assert_eq!(v.len(), 1);
    let viol = &v[0];
    assert_eq!(viol.sub_reason, ContractSubReason::Preserves);
    assert_eq!(viol.stage, Stage::Transform);
    assert_eq!(viol.hook_id, "callouts");
    assert_eq!(viol.page_slug, "projects/q2-review");
    assert_eq!(viol.detail, "-3 Wikilink");
    // CON-3225 five-part body lands the observed block.
    assert!(viol
        .observed
        .iter()
        .any(|o| o.contains("input:  3 Wikilink")));
    assert!(viol
        .observed
        .iter()
        .any(|o| o.contains("output: 0 Wikilink")));

    // Converts into a FailureRecord with the contract_violation category
    // and a sub-reason-prefixed detail — both CON-3207 wire shapes.
    let rec = viol.to_failure_record(std::time::Duration::from_millis(1));
    assert_eq!(rec.reason, FailureReason::ContractViolation.as_str());
    assert!(rec.detail.starts_with("preserves: "));
    assert!(rec.detail.contains("-3 Wikilink"));
}

/// Matrix row: multiple preserved types (Wikilink + Embed + CodeBlock).
/// A hook that drops two kinds and preserves the third reports exactly
/// two findings.
#[test]
fn test_3224_multiple_preserved_types_only_flags_the_dropped_ones() {
    let input = doc(vec![
        para(vec![wikilink("A"), wikilink("B")]),
        embed("X"),
        Block::CodeBlock(zetl::hooks::ast::CodeBlock {
            position: Position::origin(),
            fenced: true,
            lang: Some("rust".into()),
            info: None,
            text: "let x = 1;".into(),
        }),
    ]);
    // Hook drops the wikilinks and the embed; preserves the code block.
    let output = doc(vec![Block::CodeBlock(zetl::hooks::ast::CodeBlock {
        position: Position::origin(),
        fenced: true,
        lang: Some("rust".into()),
        info: None,
        text: "let x = 1;".into(),
    })]);
    let v = validate_preserves(
        Stage::Transform,
        "lossy",
        "notes/page",
        &[
            "Wikilink".to_string(),
            "Embed".to_string(),
            "CodeBlock".to_string(),
        ],
        &input,
        &output,
    );
    let kinds: Vec<String> = v.iter().map(|x| x.detail.clone()).collect();
    assert_eq!(v.len(), 2, "got {kinds:?}");
    assert!(kinds.iter().any(|k| k == "-2 Wikilink"));
    assert!(kinds.iter().any(|k| k == "-1 Embed"));
    // CodeBlock was preserved → no record mentions it.
    assert!(!kinds.iter().any(|k| k.contains("CodeBlock")));
}

// ── TEST-3222: pre-parse block-tree diff ────────────────────────────────

/// REQ-3222 scenario: pre-parse hook wraps every paragraph in a
/// `<div>`-shaped BlockQuote → kind swap at index 0. Manifest with
/// `may_restructure = false` (default) sees a violation.
#[test]
fn test_3222_may_restructure_false_detects_block_tree_change() {
    let before = doc(vec![
        para(vec![text("one")]),
        para(vec![text("two")]),
    ]);
    let after = doc(vec![
        Block::BlockQuote(BlockQuote {
            position: Position::origin(),
            children: vec![para(vec![text("one")])],
        }),
        Block::BlockQuote(BlockQuote {
            position: Position::origin(),
            children: vec![para(vec![text("two")])],
        }),
    ]);
    let v = validate_may_restructure(Stage::PreParse, "divver", "p", &before, &after)
        .expect("kind swap must trip the default may_restructure=false");
    assert_eq!(v.sub_reason, ContractSubReason::MayRestructure);
    assert!(v
        .observed
        .iter()
        .any(|o| o.contains("Paragraph") && o.contains("BlockQuote")));
}

/// REQ-3222 scenario: flipping the manifest to `may_restructure = true`
/// opts out — the same hook output MUST NOT produce a violation. The
/// property-test harness models this by honouring the ContractDecl
/// directly.
#[test]
fn test_3222_may_restructure_true_opts_out() {
    let before = doc(vec![para(vec![text("one")])]);
    let after = doc(vec![Block::BlockQuote(BlockQuote {
        position: Position::origin(),
        children: vec![para(vec![text("one")])],
    })]);
    let case = PropertyTestCase {
        name: "opted-out".into(),
        contract: ContractDecl {
            preserves: vec![],
            idempotent: false,
            may_restructure: true, // <-- opt-out
            pure: false,
            expansion_bound: None,
        },
        stage: Stage::PreParse,
        input: before,
        input_size: 0,
    };
    let report = run_property_test(&case, "divver", "p", move |_| Ok(after.clone())).unwrap();
    assert!(
        report.passed(),
        "may_restructure=true must suppress the check, got {report:?}"
    );
}

// ── TEST-3224-idempotent: CI double-run check ───────────────────────────

/// A hook that wraps its output in a new paragraph on every call
/// breaks idempotence — `f(f(x))` accumulates two wrapper paragraphs
/// while `f(x)` has one. Canonicalisation strips positions so this
/// test is robust to synthetic-node position drift.
#[test]
fn test_3224_idempotent_double_run_detects_non_idempotence() {
    let input = doc(vec![para(vec![text("x")])]);
    let wrapper = |d: Document| {
        let mut prefix = vec![para(vec![text("WRAPPED")])];
        prefix.extend(d.children);
        Ok(Document {
            children: prefix,
            ..d
        })
    };
    let v = validate_idempotence(Stage::Transform, "tasks", "daily/today", input, wrapper)
        .unwrap()
        .expect("expected an idempotence violation");
    assert_eq!(v.sub_reason, ContractSubReason::Idempotent);
    assert_eq!(v.hook_id, "tasks");
    assert_eq!(v.page_slug, "daily/today");
    assert!(v.observed.iter().any(|o| o.contains("canonicalise")));
}

/// A legitimately-idempotent hook (wraps unrendered paragraphs only,
/// short-circuits on already-wrapped input) must pass the double-run
/// check.
#[test]
fn test_3224_idempotent_double_run_passes_for_short_circuiting_hook() {
    let input = doc(vec![para(vec![text("x")])]);
    // Wrap only paragraphs without the marker; re-entry is a no-op.
    let short_circuit = |d: Document| {
        const MARKER: &str = "__zetl_wrapped__";
        let children = d
            .children
            .into_iter()
            .map(|b| match b {
                Block::Paragraph(p) => {
                    let already = p.children.iter().any(|c| match c {
                        Inline::Text(t) => t.text.contains(MARKER),
                        _ => false,
                    });
                    if already {
                        Block::Paragraph(p)
                    } else {
                        Block::Paragraph(Paragraph {
                            position: Position::origin(),
                            children: {
                                let mut c = p.children;
                                c.insert(
                                    0,
                                    Inline::Text(Text {
                                        position: Position::origin(),
                                        text: MARKER.to_string(),
                                    }),
                                );
                                c
                            },
                        })
                    }
                }
                other => other,
            })
            .collect();
        Ok(Document { children, ..d })
    };
    let v = validate_idempotence(
        Stage::Transform,
        "safe-tasks",
        "daily/today",
        input,
        short_circuit,
    )
    .unwrap();
    assert!(v.is_none());
}

// ── canonical-form equivalence (NFR-3305) ───────────────────────────────

#[test]
fn canonicalise_equates_docs_that_differ_only_in_positions() {
    let a = doc(vec![para(vec![text("hello")])]);
    let b = Document {
        ast_version: AST_VERSION.to_string(),
        kind: DocumentKind::Document,
        position: Position::new(10, 1, 10, 5),
        frontmatter: None,
        children: vec![Block::Paragraph(Paragraph {
            position: Position::new(10, 1, 10, 5),
            children: vec![Inline::Text(Text {
                position: Position::new(10, 1, 10, 5),
                text: "hello".into(),
            })],
        })],
    };
    assert_ne!(a, b, "raw docs differ in positions");
    assert_eq!(canonicalise(&a), canonicalise(&b));
}

// ── Property-test harness end-to-end ────────────────────────────────────

/// A single case drives every tier-1 validator. The hook here:
///   - preserves wikilinks (passes preserves check), BUT
///   - is not idempotent (breaks idempotent check), AND
///   - runs at the transform stage so `may_restructure` is skipped.
/// The report must flag exactly the idempotence violation.
#[test]
fn property_test_harness_end_to_end_reports_only_observed_violations() {
    let case = PropertyTestCase {
        name: "non-idempotent-preserving".into(),
        contract: ContractDecl {
            preserves: vec!["Wikilink".into()],
            idempotent: true,
            may_restructure: false,
            pure: false,
            expansion_bound: None,
        },
        stage: Stage::Transform,
        input: doc(vec![para(vec![wikilink("A"), text(" and "), wikilink("B")])]),
        input_size: 0,
    };
    // f: prepend a paragraph each call; never strips wikilinks.
    let report = run_property_test(&case, "h", "p", |d: Document| {
        let mut children = vec![para(vec![text("HEADER")])];
        children.extend(d.children);
        Ok(Document { children, ..d })
    })
    .unwrap();
    assert!(!report.passed());
    let subs: Vec<_> = report.violations.iter().map(|v| v.sub_reason).collect();
    assert!(subs.contains(&ContractSubReason::Idempotent));
    assert!(!subs.contains(&ContractSubReason::Preserves));
    assert!(!subs.contains(&ContractSubReason::MayRestructure));
}

/// When every declared contract holds the report is empty and
/// [`PropertyTestReport::passed`] is true — callers use this as the CI
/// gate predicate.
#[test]
fn property_test_harness_identity_hook_passes_every_tier1_check() {
    let case = PropertyTestCase {
        name: "identity-passes".into(),
        contract: ContractDecl {
            preserves: vec!["Wikilink".into(), "Embed".into()],
            idempotent: true,
            may_restructure: false,
            pure: true, // tier-2 advisory; v1 surfaces but doesn't gate
            expansion_bound: Some(10.0),
        },
        stage: Stage::Transform,
        input: doc(vec![
            para(vec![wikilink("A"), wikilink("B")]),
            embed("X"),
        ]),
        input_size: 1000,
    };
    let report = run_property_test(&case, "identity", "page", |d| Ok(d)).unwrap();
    assert!(
        report.passed(),
        "expected all-pass report, got {:?}",
        report.violations
    );
    assert_eq!(report.case_name, "identity-passes");
}
