//! Integration coverage for SPEC-032 §9 observability.
//!
//! Drives [`zetl::hooks::pipeline::run_page_with_observer`] with an
//! in-process [`CapturingObserver`] and asserts the event stream + the
//! rolled-up [`HookStats`] block match the OBS-3204 log-line shape and
//! the "per-stage time, per-hook time, pages matched, diagnostics
//! emitted" block the build command renders.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use zetl::hooks::ast::{
    Block, Document, DocumentKind, Inline, Paragraph, Position, Text, AST_VERSION,
};
use zetl::hooks::build_context::{BuildContext, BuildMode, PageMeta};
use zetl::hooks::observability::{
    CapturingObserver, HookInvocationStatus, HookKey, HookObserver, HookStats,
};
use zetl::hooks::pipeline::{
    run_page_with_observer, AstDocument, HookError, HookPipeline, PostRenderHook, PreParseHook,
    Stage, TransformHook,
};

// ── Fixture hooks ────────────────────────────────────────────────────────

struct TagPreParse {
    id: &'static str,
    marker: &'static str,
    calls: Arc<AtomicUsize>,
}

impl PreParseHook for TagPreParse {
    fn id(&self) -> &str {
        self.id
    }
    fn run(&self, input: String, _ctx: &BuildContext) -> Result<String, HookError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(format!("{input}{}", self.marker))
    }
}

struct TagTransform {
    id: &'static str,
    marker: &'static str,
}

impl TransformHook for TagTransform {
    fn id(&self) -> &str {
        self.id
    }
    fn run(
        &self,
        mut input: AstDocument,
        _ctx: &BuildContext,
    ) -> Result<AstDocument, HookError> {
        if let Some(Block::Paragraph(p)) =
            input.children.iter_mut().find(|b| matches!(b, Block::Paragraph(_)))
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
    fn run(
        &self,
        _input: AstDocument,
        _ctx: &BuildContext,
    ) -> Result<AstDocument, HookError> {
        Err(HookError::new(Stage::Transform, self.id, self.reason))
    }
}

struct TagPostRender {
    id: &'static str,
    suffix: &'static str,
}

impl PostRenderHook for TagPostRender {
    fn id(&self) -> &str {
        self.id
    }
    fn run(&self, input: String, _ctx: &BuildContext) -> Result<String, HookError> {
        Ok(format!("{input}{}", self.suffix))
    }
}

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

// ── Tests ─────────────────────────────────────────────────────────────────

/// The observer receives one event per hook per page, in stage order,
/// with the hook id, stage, page slug, and a non-decreasing duration.
#[test]
fn observer_sees_one_event_per_hook_per_page() {
    let pipe = HookPipeline::new()
        .with_pre_parse(TagPreParse {
            id: "prehead",
            marker: " PRE",
            calls: Arc::new(AtomicUsize::new(0)),
        })
        .with_transform(TagTransform {
            id: "callouts",
            marker: "TF",
        })
        .with_post_render(TagPostRender {
            id: "admonition",
            suffix: "<!--POST-->",
        });

    let observer = CapturingObserver::new();
    let ctx = ctx_for("projects/q2");
    let (_html, _, failures) =
        run_page_with_observer(&pipe, "x".into(), &ctx, parse, render, &observer);
    assert!(failures.is_empty());

    let events = observer.events();
    assert_eq!(events.len(), 3);

    assert_eq!(events[0].stage, Stage::PreParse);
    assert_eq!(events[0].hook_id, "prehead");
    assert_eq!(events[0].page_slug, "projects/q2");
    assert!(matches!(events[0].status, HookInvocationStatus::Ok));

    assert_eq!(events[1].stage, Stage::Transform);
    assert_eq!(events[1].hook_id, "callouts");
    assert_eq!(events[1].page_slug, "projects/q2");

    assert_eq!(events[2].stage, Stage::PostRender);
    assert_eq!(events[2].hook_id, "admonition");
}

/// A failing hook emits a `Failed` event carrying the classified reason
/// tag — the same string that lands in the `FailureRecord`.
#[test]
fn failed_hook_emits_failure_event_with_classified_reason() {
    let pipe = HookPipeline::new().with_transform(FailingTransform {
        id: "callouts",
        reason: "rigged timeout exceeded deadline_ms=100",
    });

    let observer = CapturingObserver::new();
    let ctx = ctx_for("daily/today");
    let (_html, _, failures) =
        run_page_with_observer(&pipe, "x".into(), &ctx, parse, render, &observer);

    let events = observer.events();
    assert_eq!(events.len(), 1);
    let ev = &events[0];
    assert_eq!(ev.hook_id, "callouts");
    match &ev.status {
        HookInvocationStatus::Failed { reason } => {
            assert_eq!(reason, "timeout", "classified from 'timeout exceeded …'");
        }
        HookInvocationStatus::Ok => panic!("expected Failed, got Ok"),
    }

    // Matches the reason recorded by the failure-scoping layer.
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].reason, "timeout");
}

/// Rolled-up [`HookStats`] across two pages: per-hook durations sum,
/// `pages_matched` reports unique slugs, and `diagnostics_emitted`
/// matches the returned failure buffer length.
#[test]
fn hook_stats_rolls_up_across_pages() {
    let pipe = HookPipeline::new()
        .with_transform(TagTransform {
            id: "callouts",
            marker: "C",
        })
        .with_transform(TagTransform {
            id: "tasks",
            marker: "T",
        })
        .with_transform(FailingTransform {
            id: "brokey",
            reason: "boom contract violation",
        });

    let stats = Mutex::new(HookStats::new());
    for slug in ["daily/2026-04-19", "daily/2026-04-20", "projects/q2"] {
        let ctx = ctx_for(slug);
        let (_html, _, fails) =
            run_page_with_observer(&pipe, "hi".into(), &ctx, parse, render, &stats);
        stats.lock().unwrap().record_failures(&fails);
    }

    let s = stats.lock().unwrap();
    assert_eq!(s.total_invocations(), 3 /*hooks*/ * 3 /*pages*/);
    assert_eq!(s.diagnostics_emitted(), 3, "one failure per page");

    let callouts = s
        .per_hook()
        .get(&HookKey::new(Stage::Transform, "callouts"))
        .expect("callouts recorded");
    assert_eq!(callouts.pages_matched(), 3);
    assert_eq!(callouts.invocations, 3);
    assert_eq!(callouts.failures, 0);

    let brokey = s
        .per_hook()
        .get(&HookKey::new(Stage::Transform, "brokey"))
        .expect("brokey recorded");
    assert_eq!(brokey.pages_matched(), 3);
    assert_eq!(brokey.failures, 3);
}

/// The formatted stats block contains the per-stage times, the
/// per-hook rows with pages_matched, and the diagnostics_emitted total
/// — the four signals §9 names for the block.
#[test]
fn stats_block_contains_every_spec_signal() {
    let pipe = HookPipeline::new()
        .with_pre_parse(TagPreParse {
            id: "a",
            marker: "",
            calls: Arc::new(AtomicUsize::new(0)),
        })
        .with_transform(TagTransform {
            id: "callouts",
            marker: "C",
        })
        .with_transform(FailingTransform {
            id: "brokey",
            reason: "boom",
        });

    let stats = Mutex::new(HookStats::new());
    for slug in ["p1", "p2"] {
        let ctx = ctx_for(slug);
        let (_html, _, fails) =
            run_page_with_observer(&pipe, "hi".into(), &ctx, parse, render, &stats);
        stats.lock().unwrap().record_failures(&fails);
    }

    let block = stats.lock().unwrap().format_block();
    // per-stage time
    assert!(block.contains("    pre-parse   duration_ms="));
    assert!(block.contains("    transform   duration_ms="));
    assert!(block.contains("    post-render duration_ms="));
    // per-hook rows (id + pages_matched on each row)
    assert!(block.contains("id=a") && block.contains("pages_matched=2"));
    assert!(block.contains("id=callouts") && block.contains("invocations=2"));
    assert!(block.contains("id=brokey") && block.contains("failures=2"));
    // diagnostics_emitted in the totals line
    assert!(block.contains("diagnostics_emitted=2"));
}

/// OBS-3206 summary line renders the canonical shape verbatim.
#[test]
fn summary_line_obs_3206_shape() {
    let pipe = HookPipeline::new().with_transform(TagTransform {
        id: "callouts",
        marker: "C",
    });

    let stats = Mutex::new(HookStats::new());
    for slug in ["p1", "p2", "p3"] {
        let ctx = ctx_for(slug);
        let (_html, _, fails) =
            run_page_with_observer(&pipe, "hi".into(), &ctx, parse, render, &stats);
        stats.lock().unwrap().record_failures(&fails);
    }

    let line = stats.lock().unwrap().format_summary_line();
    assert!(
        line.starts_with("[zetl] hooks: total_invocations=3 total_duration_ms="),
        "got: {line}"
    );
    assert!(line.ends_with(" failures=0"), "got: {line}");
}

/// The observer trait is `Send + Sync`, so the observer works with the
/// pipeline's concurrent-render contract. This test runs two page
/// renders across threads and asserts both events land in the shared
/// capturing observer.
#[test]
fn observer_sees_events_from_concurrent_renders() {
    let pipe = Arc::new(HookPipeline::new().with_transform(TagTransform {
        id: "callouts",
        marker: "C",
    }));
    let observer = Arc::new(CapturingObserver::new());

    let mut handles = Vec::new();
    for i in 0..4 {
        let p = pipe.clone();
        let o = observer.clone();
        handles.push(std::thread::spawn(move || {
            let ctx = ctx_for(&format!("page{i}"));
            let _ = run_page_with_observer(&p, "x".into(), &ctx, parse, render, o.as_ref());
        }));
    }
    for h in handles {
        h.join().unwrap();
    }

    assert_eq!(observer.len(), 4);
    let slugs: Vec<String> = observer
        .events()
        .into_iter()
        .map(|e| e.page_slug)
        .collect();
    for i in 0..4 {
        assert!(slugs.iter().any(|s| s == &format!("page{i}")));
    }
}

/// Driving a [`HookStats`] via its `HookObserver` impl (behind a
/// `Mutex`) lets the pipeline populate it without the caller threading
/// events through an intermediate buffer.
#[test]
fn hook_stats_usable_as_observer_directly() {
    let pipe = HookPipeline::new().with_transform(TagTransform {
        id: "callouts",
        marker: "C",
    });

    let stats_cell = Mutex::new(HookStats::new());
    // `&Mutex<HookStats>` coerces to `&dyn HookObserver` — see the impl.
    let obs: &dyn HookObserver = &stats_cell;
    let ctx = ctx_for("daily/today");
    run_page_with_observer(&pipe, "hi".into(), &ctx, parse, render, obs);

    let s = stats_cell.lock().unwrap();
    assert_eq!(s.total_invocations(), 1);
    assert_eq!(
        s.per_hook()
            .get(&HookKey::new(Stage::Transform, "callouts"))
            .unwrap()
            .invocations,
        1
    );
}
