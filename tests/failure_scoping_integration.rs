//! Integration coverage for SPEC-032 REQ-3207 / CON-3207 — failure
//! scoping and pipeline continuation.
//!
//! The test mirrors TEST-3207's canonical fixture
//! (`[10-a.py, 20-fail.py, 30-b.py]`) using in-process hook stubs: when
//! the middle hook throws, downstream hooks still run and receive the
//! failing hook's *input*, not the stage input. The failure is recorded,
//! the page render completes, and the `--hook-fail-on` policy translates
//! a non-empty failure buffer into a non-zero exit.
//!
//! Staying in-process keeps the REQ-3207 semantics under test separate
//! from the persistent-mode subprocess dance (covered by
//! `persistent_protocol_integration.rs`). Both converge on the same
//! [`FailureRecord`] schema.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

use ztl::hooks::ast::{
    Block, Document, DocumentKind, Inline, Paragraph, Position, Text, AST_VERSION,
};
use ztl::hooks::build_context::{BuildContext, BuildMode, PageMeta};
use ztl::hooks::failure_scoping::{
    exit_code_for, write_diagnostics_json, FailureRecord, HookFailOn,
};
use ztl::hooks::pipeline::{
    run_page, AstDocument, HookError, HookPipeline, PostRenderHook, PreParseHook, Stage,
    TransformHook,
};

// ── Fixture hooks ────────────────────────────────────────────────────────

/// Pre-parse hook that appends a single letter to its input; records
/// the input it saw into a shared trace.
struct TagPreParse {
    id: &'static str,
    letter: &'static str,
    calls: Arc<AtomicUsize>,
    seen: Arc<Mutex<Vec<String>>>,
}

impl PreParseHook for TagPreParse {
    fn id(&self) -> &str {
        self.id
    }
    fn run(&self, input: String, _ctx: &BuildContext) -> Result<String, HookError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().unwrap().push(input.clone());
        Ok(format!("{input}{}", self.letter))
    }
}

/// Pre-parse hook that always fails with a specific error string.
struct FailingPreParse {
    id: &'static str,
    reason: &'static str,
    calls: Arc<AtomicUsize>,
}

impl PreParseHook for FailingPreParse {
    fn id(&self) -> &str {
        self.id
    }
    fn run(&self, _input: String, _ctx: &BuildContext) -> Result<String, HookError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(HookError::new(Stage::PreParse, self.id, self.reason))
    }
}

/// Post-render hook that appends a suffix; records what it saw.
struct TagPostRender {
    id: &'static str,
    suffix: &'static str,
    calls: Arc<AtomicUsize>,
    seen: Arc<Mutex<Vec<String>>>,
}

impl PostRenderHook for TagPostRender {
    fn id(&self) -> &str {
        self.id
    }
    fn run(&self, input: String, _ctx: &BuildContext) -> Result<String, HookError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().unwrap().push(input.clone());
        Ok(format!("{input}{}", self.suffix))
    }
}

struct FailingPostRender {
    id: &'static str,
    reason: &'static str,
}

impl PostRenderHook for FailingPostRender {
    fn id(&self) -> &str {
        self.id
    }
    fn run(&self, _input: String, _ctx: &BuildContext) -> Result<String, HookError> {
        Err(HookError::new(Stage::PostRender, self.id, self.reason))
    }
}

/// Transform hook that wraps the first paragraph's text with a marker.
struct TagTransform {
    id: &'static str,
    marker: &'static str,
    calls: Arc<AtomicUsize>,
}

impl TransformHook for TagTransform {
    fn id(&self) -> &str {
        self.id
    }
    fn run(&self, mut input: AstDocument, _ctx: &BuildContext) -> Result<AstDocument, HookError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(Block::Paragraph(p)) = input
            .children
            .iter_mut()
            .find(|b| matches!(b, Block::Paragraph(_)))
        {
            if let Some(Inline::Text(t)) = p.children.first_mut() {
                t.text = format!("{}|{}", t.text, self.marker);
            }
        }
        Ok(input)
    }
}

struct FailingTransform {
    id: &'static str,
    reason: &'static str,
}

impl TransformHook for FailingTransform {
    fn id(&self) -> &str {
        self.id
    }
    fn run(&self, _input: AstDocument, _ctx: &BuildContext) -> Result<AstDocument, HookError> {
        Err(HookError::new(Stage::Transform, self.id, self.reason))
    }
}

// ── Parse / render fixtures ──────────────────────────────────────────────

fn parse(text: &str) -> AstDocument {
    Document {
        ast_version: AST_VERSION.to_string(),
        kind: DocumentKind::Document,
        position: Position::origin(),
        frontmatter: None,
        children: vec![Block::Paragraph(Paragraph {
            position: Position::origin(),
            children: vec![Inline::Text(Text {
                position: Position::origin(),
                text: text.to_string(),
            })],
        })],
    }
}

fn render(ast: &AstDocument) -> String {
    let Some(Block::Paragraph(p)) = ast
        .children
        .iter()
        .find(|b| matches!(b, Block::Paragraph(_)))
    else {
        return String::from("<p></p>");
    };
    let text = p
        .children
        .iter()
        .filter_map(|c| match c {
            Inline::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("");
    format!("<p>{text}</p>")
}

fn ctx_for(slug: &str) -> BuildContext {
    BuildContext::new(
        BuildMode::Build,
        "/vault",
        PageMeta::synthetic("Page", slug),
    )
}

// ── TEST-3207 ────────────────────────────────────────────────────────────

/// REQ-3207 canonical fixture: `[a, fail, b]` at the same stage.
///
/// `b` MUST observe `a`'s output (the stage input was `"x"`, `a` appends
/// `"A"`, so `b` sees `"xA"` — NOT `"x"`). The failing hook's output is
/// discarded, a [`FailureRecord`] is emitted, the pipeline continues.
#[test]
fn test_3207_middle_hook_fails_next_receives_previous_output() {
    let calls_a = Arc::new(AtomicUsize::new(0));
    let calls_fail = Arc::new(AtomicUsize::new(0));
    let calls_b = Arc::new(AtomicUsize::new(0));
    let seen_a = Arc::new(Mutex::new(Vec::new()));
    let seen_b = Arc::new(Mutex::new(Vec::new()));

    let pipe = HookPipeline::new()
        .with_pre_parse(TagPreParse {
            id: "10-a",
            letter: "A",
            calls: calls_a.clone(),
            seen: seen_a.clone(),
        })
        .with_pre_parse(FailingPreParse {
            id: "20-fail",
            reason: "rigged timeout exceeded deadline_ms=100",
            calls: calls_fail.clone(),
        })
        .with_pre_parse(TagPreParse {
            id: "30-b",
            letter: "B",
            calls: calls_b.clone(),
            seen: seen_b.clone(),
        });

    let ctx = ctx_for("projects/q2");
    let (html, _, failures) = run_page(&pipe, "x".into(), &ctx, parse, render);

    // All three hooks were dispatched — none were skipped.
    assert_eq!(calls_a.load(Ordering::SeqCst), 1);
    assert_eq!(calls_fail.load(Ordering::SeqCst), 1);
    assert_eq!(calls_b.load(Ordering::SeqCst), 1);

    // a received the stage input "x"; b received a's output "xA" —
    // NOT the stage input "x". (The failing hook's would-be output was
    // discarded and b got a's output instead.)
    assert_eq!(*seen_a.lock().unwrap(), vec!["x".to_string()]);
    assert_eq!(*seen_b.lock().unwrap(), vec!["xA".to_string()]);

    // Final HTML includes both succeeding markers but not the failing
    // hook's "nothing" — because 30-b appended B after seeing "xA".
    assert_eq!(html, "<p>xAB</p>");

    // Exactly one failure was recorded, with CON-3207 fields.
    assert_eq!(failures.len(), 1);
    let rec = &failures[0];
    assert_eq!(rec.hook, "20-fail");
    assert_eq!(rec.stage, "pre-parse");
    assert_eq!(rec.page_slug, "projects/q2");
    assert_eq!(rec.reason, "timeout"); // classified from "timeout exceeded" in detail
    assert!(rec.detail.contains("rigged timeout exceeded"));
    assert_eq!(rec.at.len(), 20); // ISO 8601 shape
}

/// Failure in one stage does not cascade: downstream stages still receive
/// the stage-input-before-failure and run their own hooks normally.
#[test]
fn failure_does_not_cascade_across_stages() {
    let tf_calls = Arc::new(AtomicUsize::new(0));
    let post_calls = Arc::new(AtomicUsize::new(0));
    let post_seen = Arc::new(Mutex::new(Vec::new()));

    let pipe = HookPipeline::new()
        .with_transform(FailingTransform {
            id: "bad-tf",
            reason: "boom",
        })
        .with_transform(TagTransform {
            id: "good-tf",
            marker: "TF",
            calls: tf_calls.clone(),
        })
        .with_post_render(TagPostRender {
            id: "good-post",
            suffix: "<!--POST-->",
            calls: post_calls.clone(),
            seen: post_seen.clone(),
        });

    let ctx = ctx_for("p");
    let (html, _, failures) = run_page(&pipe, "hello".into(), &ctx, parse, render);

    // Good transform still ran and successfully appended its marker.
    assert_eq!(tf_calls.load(Ordering::SeqCst), 1);
    // Post-render saw the *non*-reverted render output.
    assert_eq!(post_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        *post_seen.lock().unwrap(),
        vec!["<p>hello|TF</p>".to_string()]
    );
    assert_eq!(html, "<p>hello|TF</p><!--POST-->");

    // The single failure was pinned to the transform stage.
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].hook, "bad-tf");
    assert_eq!(failures[0].stage, "transform");
}

/// Multiple failing hooks accumulate into the buffer in stage order; each
/// record carries its own stage and hook id.
#[test]
fn multiple_failures_accumulate_in_order() {
    let pipe = HookPipeline::new()
        .with_transform(FailingTransform {
            id: "tf-fail",
            reason: "contract violation: preserves",
        })
        .with_post_render(FailingPostRender {
            id: "post-fail",
            reason: "exit code 127",
        });

    let ctx = ctx_for("daily/today");
    let (_html, _, failures) = run_page(&pipe, "hi".into(), &ctx, parse, render);

    assert_eq!(failures.len(), 2);
    assert_eq!(failures[0].stage, "transform");
    assert_eq!(failures[0].hook, "tf-fail");
    assert_eq!(failures[0].reason, "contract_violation");
    assert_eq!(failures[1].stage, "post-render");
    assert_eq!(failures[1].hook, "post-fail");
    assert_eq!(failures[1].reason, "non_zero_exit");

    // Both records carry the same page slug — REQ-3207 `page_slug` is the
    // page context, not the hook origin.
    assert!(failures.iter().all(|r| r.page_slug == "daily/today"));
}

/// `--hook-fail-on` translates a non-empty failure buffer into a non-zero
/// exit; HTML output is still written (REQ-3207: "failure is reported,
/// not build-aborting for partial output").
#[test]
fn hook_fail_on_error_exits_nonzero_but_html_is_produced() {
    let pipe = HookPipeline::new().with_transform(FailingTransform {
        id: "tf",
        reason: "boom",
    });

    let ctx = ctx_for("p");
    let (html, _, failures) = run_page(&pipe, "x".into(), &ctx, parse, render);

    // Pipeline produced output normally — the render ran against the
    // reverted AST (identical to the stage input).
    assert_eq!(html, "<p>x</p>");

    // Default policy: build continues zero even with failures.
    assert_eq!(exit_code_for(&failures, HookFailOn::Never), 0);
    // `--hook-fail-on error`: any failure flips the exit to 1.
    assert_eq!(exit_code_for(&failures, HookFailOn::Error), 1);
}

/// Per-build `hook-diagnostics.json` captures the CON-3207 record schema
/// byte-for-byte round-trip.
#[test]
fn hook_diagnostics_json_written_on_build() {
    let pipe = HookPipeline::new()
        .with_transform(FailingTransform {
            id: "callouts",
            reason: "timeout exceeded deadline_ms=100",
        })
        .with_post_render(FailingPostRender {
            id: "tasks",
            reason: "exit code 2",
        });

    let ctx = ctx_for("projects/q2");
    let (_html, _, failures) = run_page(&pipe, "x".into(), &ctx, parse, render);
    assert_eq!(failures.len(), 2);

    let out = TempDir::new().unwrap();
    let path = write_diagnostics_json(out.path(), &failures).unwrap();
    assert_eq!(path, out.path().join("hook-diagnostics.json"));

    let body = std::fs::read_to_string(&path).unwrap();
    let roundtrip: Vec<FailureRecord> = serde_json::from_str(&body).unwrap();
    assert_eq!(roundtrip, failures);

    // The schema contains every CON-3207 field.
    assert!(body.contains("\"hook\": \"callouts\""));
    assert!(body.contains("\"stage\": \"transform\""));
    assert!(body.contains("\"page_slug\": \"projects/q2\""));
    assert!(body.contains("\"reason\": \"timeout\""));
    assert!(body.contains("\"duration_ms\":"));
    assert!(body.contains("\"at\":"));
}

/// A hook that succeeds after a neighbouring failure runs unaffected —
/// proving the error path doesn't taint the buffer shared with other
/// hooks (no accidental Arc / Mutex poisoning).
#[test]
fn successful_hook_after_failure_is_unaffected() {
    let good_calls = Arc::new(AtomicUsize::new(0));
    let good_seen = Arc::new(Mutex::new(Vec::new()));

    let pipe = HookPipeline::new()
        .with_pre_parse(FailingPreParse {
            id: "fail-first",
            reason: "boom",
            calls: Arc::new(AtomicUsize::new(0)),
        })
        .with_pre_parse(TagPreParse {
            id: "good-second",
            letter: "G",
            calls: good_calls.clone(),
            seen: good_seen.clone(),
        });

    let ctx = ctx_for("p");
    let (html, _, failures) = run_page(&pipe, "x".into(), &ctx, parse, render);

    assert_eq!(failures.len(), 1);
    assert_eq!(good_calls.load(Ordering::SeqCst), 1);
    assert_eq!(*good_seen.lock().unwrap(), vec!["x".to_string()]);
    assert_eq!(html, "<p>xG</p>");
}
