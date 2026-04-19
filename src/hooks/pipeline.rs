//! Three-stage render-hook pipeline scaffolding (SPEC-032 REQ-3201 / ADR-3205).
//!
//! Stage ordering within a single page render is fixed:
//!
//! ```text
//!   pre-parse  →  parse  →  transform  →  render  →  post-render
//! ```
//!
//! - **`pre-parse`** — raw Markdown text in, raw Markdown text out.
//! - **`transform`** — zetl-ext AST in, zetl-ext AST out.
//! - **`post-render`** — HTML fragment in, HTML fragment out.
//!
//! Each stage's registered hooks run sequentially; the output of hook *N*
//! becomes the input of hook *N+1*. Pages may be processed concurrently by
//! the caller — the pipeline and all hook trait objects require `Send + Sync`.
//!
//! This module is the scaffolding only. Downstream tasks layer on:
//! selector evaluation (REQ-3204), persistent-mode protocol (CON-3201),
//! failure scoping (REQ-3207), template-variable publishing (REQ-3214),
//! and observability (SPEC-032 §9).
//!
//! The `AstDocument` type used by the `transform` stage is a
//! `serde_json::Value` placeholder until `task-ast-types-rust` lands the
//! Rust representation of the zetl-ext schema (tools/zetl-ast-schema-v1.json).

use std::time::{Duration, Instant};

/// The three hook stages per REQ-3201.
///
/// Ordering is fixed: [`Stage::PreParse`] runs before the Markdown parser,
/// [`Stage::Transform`] runs on the parsed AST, and [`Stage::PostRender`]
/// runs on the rendered HTML fragment before template composition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stage {
    PreParse,
    Transform,
    PostRender,
}

impl Stage {
    /// The stable, on-disk name for this stage — matches the `<stage>.d/`
    /// directory name and the `stage = "..."` value in a hook manifest
    /// (REQ-3203).
    pub fn as_str(self) -> &'static str {
        match self {
            Stage::PreParse => "pre-parse",
            Stage::Transform => "transform",
            Stage::PostRender => "post-render",
        }
    }

    /// All stages in pipeline execution order.
    pub fn all() -> [Stage; 3] {
        [Stage::PreParse, Stage::Transform, Stage::PostRender]
    }
}

impl std::fmt::Display for Stage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Placeholder AST type for the `transform` stage.
///
/// Swapped to the typed AST representation produced by `task-ast-types-rust`
/// once that scaffolding lands. Keeping the boundary as `serde_json::Value`
/// lets this task compile independently of the AST-types branch.
pub type AstDocument = serde_json::Value;

/// Error raised by a single hook invocation.
///
/// Failure-scoping behaviour (REQ-3207 / CON-3207) — whether to revert the
/// page fragment, record a diagnostic, or abort the build — is owned by
/// `task-failure-scoping`. This scaffolding surfaces the error; the caller
/// decides the recovery policy.
#[derive(Debug, Clone)]
pub struct HookError {
    pub stage: Stage,
    pub hook_id: String,
    pub reason: String,
}

impl HookError {
    pub fn new(stage: Stage, hook_id: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            stage,
            hook_id: hook_id.into(),
            reason: reason.into(),
        }
    }
}

impl std::fmt::Display for HookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "hook '{}' ({}) failed: {}", self.hook_id, self.stage, self.reason)
    }
}

impl std::error::Error for HookError {}

/// A `pre-parse` hook: raw Markdown in, raw Markdown out.
pub trait PreParseHook: Send + Sync {
    fn id(&self) -> &str;
    fn run(&self, input: String) -> Result<String, HookError>;
}

/// A `transform` hook: zetl-ext AST in, zetl-ext AST out.
pub trait TransformHook: Send + Sync {
    fn id(&self) -> &str;
    fn run(&self, input: AstDocument) -> Result<AstDocument, HookError>;
}

/// A `post-render` hook: HTML fragment in, HTML fragment out.
pub trait PostRenderHook: Send + Sync {
    fn id(&self) -> &str;
    fn run(&self, input: String) -> Result<String, HookError>;
}

/// An ordered collection of hooks for each of the three stages.
///
/// Hooks are stored in the order they should execute. Composition and
/// precedence (vault > theme, filename sort) are determined by
/// `task-composition`; this scaffolding consumes the already-ordered list.
#[derive(Default)]
pub struct HookPipeline {
    pre_parse: Vec<Box<dyn PreParseHook>>,
    transform: Vec<Box<dyn TransformHook>>,
    post_render: Vec<Box<dyn PostRenderHook>>,
}

impl HookPipeline {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_pre_parse<H: PreParseHook + 'static>(&mut self, hook: H) {
        self.pre_parse.push(Box::new(hook));
    }

    pub fn push_transform<H: TransformHook + 'static>(&mut self, hook: H) {
        self.transform.push(Box::new(hook));
    }

    pub fn push_post_render<H: PostRenderHook + 'static>(&mut self, hook: H) {
        self.post_render.push(Box::new(hook));
    }

    /// Builder-style variant of [`Self::push_pre_parse`].
    pub fn with_pre_parse<H: PreParseHook + 'static>(mut self, hook: H) -> Self {
        self.push_pre_parse(hook);
        self
    }

    /// Builder-style variant of [`Self::push_transform`].
    pub fn with_transform<H: TransformHook + 'static>(mut self, hook: H) -> Self {
        self.push_transform(hook);
        self
    }

    /// Builder-style variant of [`Self::push_post_render`].
    pub fn with_post_render<H: PostRenderHook + 'static>(mut self, hook: H) -> Self {
        self.push_post_render(hook);
        self
    }

    pub fn pre_parse_len(&self) -> usize {
        self.pre_parse.len()
    }
    pub fn transform_len(&self) -> usize {
        self.transform.len()
    }
    pub fn post_render_len(&self) -> usize {
        self.post_render.len()
    }

    /// Number of hooks registered at `stage`.
    pub fn stage_len(&self, stage: Stage) -> usize {
        match stage {
            Stage::PreParse => self.pre_parse.len(),
            Stage::Transform => self.transform.len(),
            Stage::PostRender => self.post_render.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.pre_parse.is_empty() && self.transform.is_empty() && self.post_render.is_empty()
    }
}

/// Per-stage wall-clock accounting for a single page render.
///
/// Surfaced by [`run_page`] and aggregated by the build command into the
/// stats block (SPEC-032 §9 OBS-3201). `parse` and `render` time — the
/// zetl-owned conversions between the stage boundaries — are recorded
/// alongside the hook-stage times so the whole pipeline is measurable from
/// a single struct.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PipelineStats {
    pub pre_parse: Duration,
    pub parse: Duration,
    pub transform: Duration,
    pub render: Duration,
    pub post_render: Duration,
}

impl PipelineStats {
    /// Wall-clock time summed across every stage (hooks + parse + render).
    pub fn total(&self) -> Duration {
        self.pre_parse + self.parse + self.transform + self.render + self.post_render
    }

    /// Duration spent running hooks at `stage` (excludes `parse` / `render`).
    pub fn stage(&self, stage: Stage) -> Duration {
        match stage {
            Stage::PreParse => self.pre_parse,
            Stage::Transform => self.transform,
            Stage::PostRender => self.post_render,
        }
    }

    /// Sum this page's durations into `total` (used by the build command to
    /// aggregate across pages when rendering concurrently).
    pub fn accumulate_into(&self, total: &mut PipelineStats) {
        total.pre_parse += self.pre_parse;
        total.parse += self.parse;
        total.transform += self.transform;
        total.render += self.render;
        total.post_render += self.post_render;
    }
}

/// Run the full three-stage pipeline for a single page.
///
/// Sequence, per REQ-3201:
///
/// 1. Apply every `pre-parse` hook to the raw Markdown in registration order.
/// 2. Call `parse` on the post-hook text to produce an AST.
/// 3. Apply every `transform` hook to the AST in registration order.
/// 4. Call `render` on the post-hook AST to produce an HTML fragment.
/// 5. Apply every `post-render` hook to the HTML in registration order.
///
/// `parse` and `render` are injected by the caller — the pipeline is
/// agnostic to the specific Markdown parser (pulldown-cmark, pandoc-types,
/// mdast, …) and HTML renderer. This also keeps the scaffolding pure: no
/// concrete dependency on the render pipeline in `src/web/`.
///
/// Concurrency: the function is pure with respect to the pipeline (takes
/// `&HookPipeline`) and the trait-object bounds are `Send + Sync`, so
/// callers may invoke `run_page` from multiple threads concurrently for
/// different pages. Within a single page the stages remain sequential.
pub fn run_page<ParseFn, RenderFn>(
    pipeline: &HookPipeline,
    raw_markdown: String,
    parse: ParseFn,
    render: RenderFn,
) -> Result<(String, PipelineStats), HookError>
where
    ParseFn: FnOnce(&str) -> AstDocument,
    RenderFn: FnOnce(&AstDocument) -> String,
{
    let mut stats = PipelineStats::default();

    // Stage 1: pre-parse (raw text → raw text).
    let mut text = raw_markdown;
    if !pipeline.pre_parse.is_empty() {
        let t0 = Instant::now();
        for hook in &pipeline.pre_parse {
            text = hook.run(text)?;
        }
        stats.pre_parse = t0.elapsed();
    }

    // Parse (zetl-owned; not a hook).
    let t0 = Instant::now();
    let mut ast = parse(&text);
    stats.parse = t0.elapsed();

    // Stage 2: transform (AST → AST).
    if !pipeline.transform.is_empty() {
        let t0 = Instant::now();
        for hook in &pipeline.transform {
            ast = hook.run(ast)?;
        }
        stats.transform = t0.elapsed();
    }

    // Render (zetl-owned; not a hook).
    let t0 = Instant::now();
    let mut html = render(&ast);
    stats.render = t0.elapsed();

    // Stage 3: post-render (HTML → HTML).
    if !pipeline.post_render.is_empty() {
        let t0 = Instant::now();
        for hook in &pipeline.post_render {
            html = hook.run(html)?;
        }
        stats.post_render = t0.elapsed();
    }

    Ok((html, stats))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    // ── Fixture hooks ──────────────────────────────────────────────────────

    /// pre-parse hook: appends a trailing marker to the raw Markdown.
    struct TagPreParse {
        id: String,
        marker: &'static str,
        calls: Arc<AtomicUsize>,
        trace: Arc<Mutex<Vec<String>>>,
    }

    impl PreParseHook for TagPreParse {
        fn id(&self) -> &str {
            &self.id
        }
        fn run(&self, input: String) -> Result<String, HookError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.trace.lock().unwrap().push(format!("pre:{}", self.id));
            Ok(format!("{input} {}", self.marker))
        }
    }

    /// transform hook: pushes a marker into the AST's `marks` array.
    struct TagTransform {
        id: String,
        marker: &'static str,
        calls: Arc<AtomicUsize>,
        trace: Arc<Mutex<Vec<String>>>,
    }

    impl TransformHook for TagTransform {
        fn id(&self) -> &str {
            &self.id
        }
        fn run(&self, mut input: AstDocument) -> Result<AstDocument, HookError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.trace.lock().unwrap().push(format!("tf:{}", self.id));
            let marks = input
                .get_mut("marks")
                .and_then(|m| m.as_array_mut())
                .expect("fixture AST must carry a `marks` array");
            marks.push(serde_json::Value::String(self.marker.to_string()));
            Ok(input)
        }
    }

    /// post-render hook: appends a trailing marker to the HTML fragment.
    struct TagPostRender {
        id: String,
        marker: &'static str,
        calls: Arc<AtomicUsize>,
        trace: Arc<Mutex<Vec<String>>>,
    }

    impl PostRenderHook for TagPostRender {
        fn id(&self) -> &str {
            &self.id
        }
        fn run(&self, input: String) -> Result<String, HookError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.trace.lock().unwrap().push(format!("post:{}", self.id));
            Ok(format!("{input}<!--{}-->", self.marker))
        }
    }

    /// Fixture parse: wraps the incoming text as the AST's `raw` field plus
    /// an empty `marks` array that transform hooks append to.
    fn parse(text: &str) -> AstDocument {
        serde_json::json!({
            "ast_version": "1.0",
            "type": "Document",
            "raw": text,
            "marks": [],
        })
    }

    /// Fixture render: serialises the AST's raw text + marks into a string
    /// so downstream assertions can prove transform hooks ran before render.
    fn render(ast: &AstDocument) -> String {
        let raw = ast.get("raw").and_then(|v| v.as_str()).unwrap_or("");
        let marks = ast
            .get("marks")
            .and_then(|m| m.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default();
        format!("<p>{raw} |marks={marks}</p>")
    }

    // ── Tests ───────────────────────────────────────────────────────────────

    #[test]
    fn stage_as_str_matches_spec() {
        assert_eq!(Stage::PreParse.as_str(), "pre-parse");
        assert_eq!(Stage::Transform.as_str(), "transform");
        assert_eq!(Stage::PostRender.as_str(), "post-render");
    }

    #[test]
    fn stage_all_is_pipeline_order() {
        assert_eq!(
            Stage::all(),
            [Stage::PreParse, Stage::Transform, Stage::PostRender]
        );
    }

    #[test]
    fn empty_pipeline_passes_through() {
        let pipe = HookPipeline::new();
        assert!(pipe.is_empty());
        let (html, stats) = run_page(&pipe, "hello".into(), parse, render).unwrap();
        assert_eq!(html, "<p>hello |marks=</p>");
        // Parse and render still ran; hook stages did not.
        assert_eq!(stats.pre_parse, Duration::ZERO);
        assert_eq!(stats.transform, Duration::ZERO);
        assert_eq!(stats.post_render, Duration::ZERO);
    }

    /// TEST-3201 — three-stage execution order + data flow.
    ///
    /// Registers one hook per stage with a sentinel marker and asserts
    /// (a) each hook ran exactly once, (b) the order was
    /// `pre-parse → transform → post-render`, and (c) each stage's mutation
    /// lands in the correct data shape:
    ///   - pre-parse's marker appears in the HTML's text (proves it was fed
    ///     into `parse`),
    ///   - transform's marker appears inside `|marks=…|` (proves it was fed
    ///     into `render`),
    ///   - post-render's marker appears as an HTML comment suffix (proves
    ///     it ran *after* `render`).
    #[test]
    fn three_stage_execution_order() {
        let calls_pre = Arc::new(AtomicUsize::new(0));
        let calls_tf = Arc::new(AtomicUsize::new(0));
        let calls_post = Arc::new(AtomicUsize::new(0));
        let trace = Arc::new(Mutex::new(Vec::new()));

        let pipe = HookPipeline::new()
            .with_pre_parse(TagPreParse {
                id: "preparser".into(),
                marker: "PRE",
                calls: calls_pre.clone(),
                trace: trace.clone(),
            })
            .with_transform(TagTransform {
                id: "transformer".into(),
                marker: "TF",
                calls: calls_tf.clone(),
                trace: trace.clone(),
            })
            .with_post_render(TagPostRender {
                id: "postrenderer".into(),
                marker: "POST",
                calls: calls_post.clone(),
                trace: trace.clone(),
            });

        assert_eq!(pipe.pre_parse_len(), 1);
        assert_eq!(pipe.transform_len(), 1);
        assert_eq!(pipe.post_render_len(), 1);

        let (html, stats) = run_page(&pipe, "hello".into(), parse, render).unwrap();

        // Each hook ran exactly once.
        assert_eq!(calls_pre.load(Ordering::SeqCst), 1);
        assert_eq!(calls_tf.load(Ordering::SeqCst), 1);
        assert_eq!(calls_post.load(Ordering::SeqCst), 1);

        // Execution order was pre-parse → transform → post-render.
        let order = trace.lock().unwrap().clone();
        assert_eq!(
            order,
            vec!["pre:preparser", "tf:transformer", "post:postrenderer"]
        );

        // Each stage's marker lands in the right pass:
        //   - PRE is in the parsed text (pre-parse fed into parse)
        //   - TF is in the marks array (transform fed into render)
        //   - POST is an HTML suffix (post-render ran last)
        assert_eq!(html, "<p>hello PRE |marks=TF</p><!--POST-->");

        // Per-stage time accounting recorded non-zero-or-zero durations for
        // every slot (hooks may be fast enough for a zero on some clocks,
        // but the fields must exist and be <= the total).
        assert!(stats.total() >= stats.pre_parse);
        assert!(stats.total() >= stats.transform);
        assert!(stats.total() >= stats.post_render);
        assert_eq!(stats.stage(Stage::PreParse), stats.pre_parse);
        assert_eq!(stats.stage(Stage::Transform), stats.transform);
        assert_eq!(stats.stage(Stage::PostRender), stats.post_render);
    }

    /// Multiple hooks at the same stage run sequentially in registration
    /// order — the scaffolding's "sequential per stage" contract.
    #[test]
    fn sequential_order_within_stage() {
        let trace = Arc::new(Mutex::new(Vec::new()));
        let calls = Arc::new(AtomicUsize::new(0));

        let mut pipe = HookPipeline::new();
        pipe.push_pre_parse(TagPreParse {
            id: "a".into(),
            marker: "A",
            calls: calls.clone(),
            trace: trace.clone(),
        });
        pipe.push_pre_parse(TagPreParse {
            id: "b".into(),
            marker: "B",
            calls: calls.clone(),
            trace: trace.clone(),
        });
        pipe.push_transform(TagTransform {
            id: "t1".into(),
            marker: "T1",
            calls: calls.clone(),
            trace: trace.clone(),
        });
        pipe.push_transform(TagTransform {
            id: "t2".into(),
            marker: "T2",
            calls: calls.clone(),
            trace: trace.clone(),
        });

        let (html, _) = run_page(&pipe, "x".into(), parse, render).unwrap();
        assert_eq!(html, "<p>x A B |marks=T1,T2</p>");
        assert_eq!(
            *trace.lock().unwrap(),
            vec!["pre:a", "pre:b", "tf:t1", "tf:t2"]
        );
    }

    /// A hook error short-circuits the pipeline; later hooks do not run.
    /// Failure-recovery policy (revert vs. abort) is the concern of
    /// task-failure-scoping — this scaffolding just propagates the error.
    #[test]
    fn hook_error_short_circuits() {
        struct FailingTransform;
        impl TransformHook for FailingTransform {
            fn id(&self) -> &str {
                "boom"
            }
            fn run(&self, _input: AstDocument) -> Result<AstDocument, HookError> {
                Err(HookError::new(Stage::Transform, "boom", "rigged"))
            }
        }

        let post_calls = Arc::new(AtomicUsize::new(0));
        let post_trace = Arc::new(Mutex::new(Vec::new()));
        let pipe = HookPipeline::new()
            .with_transform(FailingTransform)
            .with_post_render(TagPostRender {
                id: "after".into(),
                marker: "AFTER",
                calls: post_calls.clone(),
                trace: post_trace.clone(),
            });

        let err = run_page(&pipe, "x".into(), parse, render).unwrap_err();
        assert_eq!(err.stage, Stage::Transform);
        assert_eq!(err.hook_id, "boom");
        // Downstream hook was never reached.
        assert_eq!(post_calls.load(Ordering::SeqCst), 0);
    }

    /// Parse and render closures run exactly once per page and only if every
    /// earlier stage succeeded. Counting via `Cell`-like state verifies the
    /// FnOnce contract implicitly (compiler) plus the control flow.
    #[test]
    fn parse_and_render_run_once_each() {
        let parse_count = Arc::new(AtomicUsize::new(0));
        let render_count = Arc::new(AtomicUsize::new(0));
        let parse_c = parse_count.clone();
        let render_c = render_count.clone();

        let parse_fn = move |text: &str| {
            parse_c.fetch_add(1, Ordering::SeqCst);
            parse(text)
        };
        let render_fn = move |ast: &AstDocument| {
            render_c.fetch_add(1, Ordering::SeqCst);
            render(ast)
        };

        let pipe = HookPipeline::new();
        let _ = run_page(&pipe, "y".into(), parse_fn, render_fn).unwrap();
        assert_eq!(parse_count.load(Ordering::SeqCst), 1);
        assert_eq!(render_count.load(Ordering::SeqCst), 1);
    }

    /// Hook trait objects are `Send + Sync` so pages may be processed
    /// concurrently by the caller — this test actually runs two page
    /// renders across threads sharing the same pipeline.
    #[test]
    fn pipeline_is_shareable_across_threads() {
        let trace = Arc::new(Mutex::new(Vec::new()));
        let calls = Arc::new(AtomicUsize::new(0));
        let pipe = Arc::new(
            HookPipeline::new().with_pre_parse(TagPreParse {
                id: "a".into(),
                marker: "A",
                calls: calls.clone(),
                trace: trace.clone(),
            }),
        );

        let mut handles = Vec::new();
        for i in 0..4 {
            let p = pipe.clone();
            handles.push(std::thread::spawn(move || {
                run_page(&p, format!("page{i}"), parse, render).unwrap().0
            }));
        }
        let outputs: Vec<String> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        // Each page saw the single pre-parse hook.
        assert_eq!(calls.load(Ordering::SeqCst), 4);
        // All four page outputs are present.
        for i in 0..4 {
            let expected = format!("<p>page{i} A |marks=</p>");
            assert!(outputs.contains(&expected), "missing output for page{i}");
        }
    }

    #[test]
    fn pipeline_stats_accumulates() {
        let mut total = PipelineStats::default();
        let a = PipelineStats {
            pre_parse: Duration::from_millis(1),
            parse: Duration::from_millis(2),
            transform: Duration::from_millis(3),
            render: Duration::from_millis(4),
            post_render: Duration::from_millis(5),
        };
        let b = PipelineStats {
            pre_parse: Duration::from_millis(10),
            parse: Duration::from_millis(20),
            transform: Duration::from_millis(30),
            render: Duration::from_millis(40),
            post_render: Duration::from_millis(50),
        };
        a.accumulate_into(&mut total);
        b.accumulate_into(&mut total);
        assert_eq!(total.pre_parse, Duration::from_millis(11));
        assert_eq!(total.parse, Duration::from_millis(22));
        assert_eq!(total.transform, Duration::from_millis(33));
        assert_eq!(total.render, Duration::from_millis(44));
        assert_eq!(total.post_render, Duration::from_millis(55));
        assert_eq!(total.total(), Duration::from_millis(11 + 22 + 33 + 44 + 55));
    }

    #[test]
    fn stage_len_reports_per_stage_counts() {
        let pipe = HookPipeline::new()
            .with_pre_parse(TagPreParse {
                id: "p".into(),
                marker: "P",
                calls: Arc::new(AtomicUsize::new(0)),
                trace: Arc::new(Mutex::new(Vec::new())),
            })
            .with_transform(TagTransform {
                id: "t".into(),
                marker: "T",
                calls: Arc::new(AtomicUsize::new(0)),
                trace: Arc::new(Mutex::new(Vec::new())),
            });
        assert_eq!(pipe.stage_len(Stage::PreParse), 1);
        assert_eq!(pipe.stage_len(Stage::Transform), 1);
        assert_eq!(pipe.stage_len(Stage::PostRender), 0);
        assert!(!pipe.is_empty());
    }
}
