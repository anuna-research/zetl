//! `EcosystemAdapter` trait (SPEC-033 REQ-3302 / CON-3302).
//!
//! Every plugin ecosystem zetl supports (Pandoc filters, mdBook
//! preprocessors, remark plugins, and any future additions) is reached
//! through a single uniform interface defined here. An adapter owns:
//!
//! - **Runtime detection** — [`EcosystemAdapter::probe`] asks "is this
//!   ecosystem's toolchain present on the host?", returning a
//!   [`RuntimeStatus`] that the build pipeline consults before dispatch
//!   (REQ-3313). A missing runtime surfaces an actionable diagnostic and
//!   disables the adapter for the session rather than aborting the build.
//!
//! - **AST translation** — [`EcosystemAdapter::translate_to_foreign`] and
//!   [`EcosystemAdapter::translate_from_foreign`] bridge zetl-ext
//!   ([`crate::hooks::ast::Document`]) and the ecosystem's native AST
//!   shape (pandoc-types, mdast, etc.). Round-trip fidelity is the
//!   property-test target (NFR-3305): every v1 ecosystem's adapter must
//!   survive `foreign ← zetl → foreign' → zetl'` with canonical
//!   equivalence.
//!
//! - **Invocation** — [`EcosystemAdapter::invoke_plugin`] applies the
//!   ecosystem's protocol conventions (env vars, argv, stdin/stdout
//!   shape) and returns the plugin's output as a [`PluginResponse`].
//!   The adapter is the only code path in zetl that understands how
//!   pandoc filters vs. mdbook preprocessors vs. remark plugins actually
//!   get called — the SPEC-032 hook runtime below delegates all of that
//!   transport detail here.
//!
//! - **Capability declaration** — [`EcosystemAdapter::supported_stages`]
//!   lists which SPEC-032 stages the ecosystem can participate in
//!   (Pandoc filters: `transform` only; mdBook preprocessors: `pre-parse`
//!   only; remark plugins: `transform` only). The composition layer
//!   rejects a manifest at load time that declares a stage outside this
//!   list.
//!
//! ## `ForeignAst` is `serde_json::Value` at the trait boundary
//!
//! CON-3302 specifies `ForeignAst` as "an ecosystem-specific type —
//! `PandocTypesDocument`, `MdastDocument`, `MdBookBook` — adapters
//! define their own." In the Rust layer we erase that to
//! [`serde_json::Value`] at the trait signature because:
//!
//! 1. CON-3301 stores `adapter_ctor: fn() -> Box<dyn EcosystemAdapter>`
//!    in the registry, which requires trait-object safety — the literal
//!    associated-type form of CON-3302 isn't dyn-compatible.
//! 2. Every v1 transport already serialises to JSON at the process
//!    boundary (persistent-mode protocol, pandoc filter stdin/stdout,
//!    remark Node subprocess), so the adapter's internal typed AST
//!    always round-trips through JSON anyway.
//! 3. The existing [`crate::hooks::translators::Translator`] pair makes
//!    the same choice for exactly the same reasons; this module's
//!    translate methods are a strict superset of that trait's surface
//!    (a `Translator` is enough to implement the translate half of an
//!    `EcosystemAdapter`).
//!
//! Concrete adapters (task-pandoc-adapter, task-mdbook-adapter,
//! task-remark-harness) are free to define typed interior ASTs and
//! convert to/from JSON at the trait boundary — the mock adapter in
//! this module's tests exercises that pattern end-to-end.
//!
//! ## TEST-3302 conformance harness
//!
//! [`run_conformance`] is a parameterised suite that exercises every
//! trait method on an arbitrary adapter. Downstream adapter tasks call
//! it from their integration tests so the adapter-trait contract is
//! enforced identically across mock, pandoc, mdbook, and remark.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::hooks::ast::Document;
use crate::hooks::build_context::BuildContext;
use crate::hooks::pipeline::Stage;
use crate::hooks::translators::{AstType, TranslationError};

// ── Trait surface (CON-3302) ────────────────────────────────────────────────

/// Uniform interface for every plugin ecosystem zetl can drive
/// (SPEC-033 REQ-3302 / CON-3302).
///
/// Concrete implementations live in `src/ecosystems/<name>/adapter.rs`
/// and are reached through the [`EcosystemEntry::adapter_ctor`][ctor]
/// constructor in the registry.
///
/// See the module docs for the `ForeignAst = serde_json::Value`
/// decision and the TEST-3302 conformance harness.
///
/// [ctor]: crate::ecosystems::EcosystemEntry
pub trait EcosystemAdapter: Send + Sync {
    /// Stable registry id for this adapter, matching
    /// [`crate::ecosystems::EcosystemEntry::id`].
    ///
    /// Used in diagnostics, the `zetl ecosystem check` report, and the
    /// adapter-trait conformance harness's failure messages.
    fn id(&self) -> &str;

    /// AST ecosystem this adapter reads/emits at the translate boundary.
    /// The composition layer uses this to pair a hook's declared
    /// `ast_type` with the correct adapter at dispatch time (REQ-3221).
    fn ast_type(&self) -> AstType;

    /// Detect the ecosystem's runtime dependency (REQ-3313). Called once
    /// per session at pipeline init; the result is cached by the caller.
    ///
    /// Takes `&mut self` so adapters may cache transport state across
    /// probe calls (e.g. a `pandoc --version` lookup that also primes
    /// the `PANDOC_API_VERSION` cache for later [`Self::invoke_plugin`]
    /// calls — CON-3303).
    fn probe(&mut self) -> RuntimeStatus;

    /// Stages this adapter's plugins may participate in. Must be a
    /// non-empty subset of the registry's `supported_stages` for the
    /// same ecosystem id (TEST-3301 asserts the two stay in sync).
    fn supported_stages(&self) -> &[Stage];

    /// Serialise a zetl-ext document into the ecosystem's native AST
    /// shape, as JSON at the trait boundary (see module docs).
    ///
    /// Returning `Ok` does *not* guarantee the hook will accept the
    /// payload — it means zetl has produced a shape the ecosystem's
    /// toolchain recognises. Plugin-specific validation failures
    /// surface later through [`Self::invoke_plugin`]'s
    /// [`PluginResponse::Error`].
    fn translate_to_foreign(&self, doc: &Document) -> Result<Value, TranslationError>;

    /// Deserialise a foreign AST response back into zetl-ext. Lossy
    /// conversions are permitted per CON-3221 — the round-trip contract
    /// is semantic equivalence under
    /// [`crate::hooks::contract::canonicalise`], not byte equality.
    fn translate_from_foreign(&self, foreign: Value) -> Result<Document, TranslationError>;

    /// Invoke a single plugin end-to-end using this ecosystem's
    /// conventions. The adapter owns the full transport: spawning the
    /// plugin process, setting env vars / argv, serialising the
    /// `input`, reading the plugin's response, and surfacing diagnostics.
    ///
    /// Never panics — all failures are encoded in
    /// [`PluginResponse::Error`] so the SPEC-032 REQ-3207 failure-scoped
    /// revert path can run without catching unwinds.
    fn invoke_plugin(
        &mut self,
        manifest: &PluginManifest,
        input: StageInput,
        context: &HookContext<'_>,
    ) -> PluginResponse;
}

// ── Supporting types ────────────────────────────────────────────────────────

/// Outcome of [`EcosystemAdapter::probe`]. Reported to the user via
/// `zetl ecosystem check` and consulted by the pipeline before enabling
/// any hook backed by this adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum RuntimeStatus {
    /// The ecosystem's binary is on `PATH` at an acceptable version.
    Available {
        /// Path to the resolved executable (if the adapter inspected
        /// one). Empty when the runtime is intrinsic to the zetl
        /// binary (e.g. a pure-Rust ecosystem).
        #[serde(default)]
        executable: Option<PathBuf>,
        /// Version string as reported by the runtime itself — whatever
        /// format that ecosystem uses (`pandoc 3.1.12.1`, `v18.19.0`,
        /// `mdbook 0.4.40`, …). Kept as an opaque string so adapters
        /// don't have to agree on a normalised form.
        version: String,
    },
    /// The ecosystem's binary was found but is below the registered
    /// minimum version.
    OutdatedVersion {
        executable: PathBuf,
        found: String,
        required: String,
        /// Actionable hint — e.g. `upgrade pandoc to ≥ 2.11 via your
        /// package manager`. REQ-3313 requires all diagnostics to carry
        /// a concrete next step.
        hint: String,
    },
    /// The ecosystem's binary isn't on `PATH`. The adapter is disabled
    /// for this session; any hooks declaring this ecosystem are
    /// likewise disabled with a propagated diagnostic.
    Missing {
        /// Binary name that was looked up on `PATH` and not found.
        binary: String,
        /// Actionable hint explaining how the user can install it.
        hint: String,
    },
    /// The probe itself failed — adapter couldn't execute the version
    /// check (permission denied, I/O error, malformed output, …). Same
    /// user-visible behaviour as `Missing` (disable + diagnose) but
    /// kept as a separate variant so the diagnostic carries the root
    /// cause, not a misleading "binary not found".
    ProbeFailed { detail: String },
}

impl RuntimeStatus {
    /// `true` when the adapter should be considered live for this
    /// session. Every non-[`RuntimeStatus::Available`] variant evaluates
    /// to `false`.
    pub fn is_available(&self) -> bool {
        matches!(self, RuntimeStatus::Available { .. })
    }

    /// Short axis tag for table rendering (`zetl ecosystem check`).
    pub fn tag(&self) -> &'static str {
        match self {
            RuntimeStatus::Available { .. } => "available",
            RuntimeStatus::OutdatedVersion { .. } => "outdated",
            RuntimeStatus::Missing { .. } => "missing",
            RuntimeStatus::ProbeFailed { .. } => "probe-failed",
        }
    }
}

/// Per-plugin manifest data the adapter needs to actually spawn the
/// plugin (REQ-3312). The full on-disk manifest grammar lives in
/// [`crate::hooks::manifest`] / downstream `task-eco-manifest`; this is
/// the subset the adapter layer consumes.
///
/// Adapters that need ecosystem-specific fields read them out of
/// [`Self::options`] (a free-form JSON object) — e.g. Pandoc's
/// `mode = "filter" | "native"` (CON-3303), mdBook's preprocessor
/// `supports-html` probe, remark's `package` + `options` block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Stable extension id (manifest `extension_id =` override or
    /// filename fallback). Used in log lines, diagnostics, and the
    /// template-vars namespace.
    pub extension_id: String,
    /// Absolute path to the plugin executable the adapter will spawn.
    pub executable: PathBuf,
    /// argv tail appended after the adapter's own conventions (e.g. a
    /// Pandoc filter gets `[exec, "html"]` by default; any user-supplied
    /// args here go after the `"html"`).
    #[serde(default)]
    pub args: Vec<String>,
    /// Ecosystem-specific options parsed from the manifest's
    /// `[ecosystem]` block. The adapter knows how to interpret its own
    /// subtree; unknown keys are the adapter's problem, not the caller's.
    #[serde(default)]
    pub options: Value,
    /// Hard deadline for a single invocation (REQ-3207 failure scoping).
    /// Adapters that spawn subprocesses enforce this as a kill timer.
    #[serde(default = "PluginManifest::default_timeout")]
    pub timeout: Duration,
}

impl PluginManifest {
    fn default_timeout() -> Duration {
        Duration::from_secs(30)
    }
}

/// Payload handed to [`EcosystemAdapter::invoke_plugin`]. Shape follows
/// the SPEC-032 stage the plugin is running in:
///
/// - `pre-parse` → [`StageInput::Markdown`]
/// - `transform` → [`StageInput::ForeignAst`] (already translated)
/// - `post-render` → [`StageInput::Html`]
///
/// Adapters whose `supported_stages` doesn't include the incoming
/// variant MUST return [`PluginResponse::Error`] with reason
/// `"unsupported_stage"` rather than panicking.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "stage", rename_all = "kebab-case")]
pub enum StageInput {
    /// Raw Markdown text — SPEC-032 `pre-parse` stage.
    PreParse { markdown: String },
    /// Already-translated foreign AST (JSON form) — SPEC-032
    /// `transform` stage. The caller has already applied
    /// [`EcosystemAdapter::translate_to_foreign`] to reach this shape.
    Transform { foreign: Value },
    /// Rendered HTML fragment — SPEC-032 `post-render` stage.
    PostRender { html: String },
}

impl StageInput {
    /// The [`Stage`] this input corresponds to.
    pub fn stage(&self) -> Stage {
        match self {
            StageInput::PreParse { .. } => Stage::PreParse,
            StageInput::Transform { .. } => Stage::Transform,
            StageInput::PostRender { .. } => Stage::PostRender,
        }
    }
}

/// Plugin output. Mirrors [`StageInput`]'s stage axis so the pipeline
/// can pattern-match on either end.
///
/// A correct adapter returns the same variant as the input stage
/// (pre-parse plugins return Markdown, transform plugins return foreign
/// AST JSON, post-render plugins return HTML). The conformance harness
/// asserts this invariant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "stage", rename_all = "kebab-case")]
pub enum StageOutput {
    PreParse { markdown: String },
    Transform { foreign: Value },
    PostRender { html: String },
}

impl StageOutput {
    /// The [`Stage`] this output corresponds to.
    pub fn stage(&self) -> Stage {
        match self {
            StageOutput::PreParse { .. } => Stage::PreParse,
            StageOutput::Transform { .. } => Stage::Transform,
            StageOutput::PostRender { .. } => Stage::PostRender,
        }
    }
}

/// Outcome of [`EcosystemAdapter::invoke_plugin`] (CON-3302's
/// `PluginResponse`).
///
/// Kept as an enum rather than `Result<StageOutput, InvokeError>` so
/// successful invocations can still surface non-fatal `diagnostics` and
/// captured `stderr` alongside the output — matching what the
/// persistent-mode protocol already emits via [`HookMessage::Result`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum PluginResponse {
    /// Plugin ran to completion and returned a well-formed stage output.
    Success {
        output: StageOutput,
        /// Structured advisory messages the plugin emitted. Empty is
        /// the common case; each entry surfaces as a `[zetl] ...` log
        /// line at the pipeline level.
        #[serde(default)]
        diagnostics: Vec<Diagnostic>,
        /// Captured stderr. Surfaced only under `--verbose`; REQ-3207
        /// attaches it to the failure record when the hook later fails.
        #[serde(default)]
        stderr: String,
        /// Wall-clock duration the adapter observed for this
        /// invocation. Feeds the OBS-3204 per-hook timing block.
        #[serde(default)]
        duration: Duration,
    },
    /// Plugin failed. The SPEC-032 REQ-3207 policy decides whether to
    /// revert this page, continue, or abort the build.
    Error {
        /// Short classification — maps 1:1 to
        /// [`crate::hooks::failure_scoping::FailureReason`] tags
        /// (`"timeout"`, `"non_zero_exit"`, `"contract_violation"`, …).
        reason: String,
        /// Free-form human-readable detail. Appended to the failure
        /// record.
        detail: String,
        /// Captured stderr at failure time.
        #[serde(default)]
        stderr: String,
        /// Wall-clock duration observed before the error was detected.
        #[serde(default)]
        duration: Duration,
    },
}

impl PluginResponse {
    /// Convenience helper for adapter internals: wrap a `StageOutput`
    /// into a `Success` with empty diagnostics and stderr.
    pub fn success(output: StageOutput, duration: Duration) -> Self {
        PluginResponse::Success {
            output,
            diagnostics: Vec::new(),
            stderr: String::new(),
            duration,
        }
    }

    /// Convenience helper for adapter internals: build an `Error`
    /// variant from the three most-used fields.
    pub fn error(reason: impl Into<String>, detail: impl Into<String>, duration: Duration) -> Self {
        PluginResponse::Error {
            reason: reason.into(),
            detail: detail.into(),
            stderr: String::new(),
            duration,
        }
    }

    /// `true` for the [`PluginResponse::Success`] variant.
    pub fn is_success(&self) -> bool {
        matches!(self, PluginResponse::Success { .. })
    }
}

/// Non-fatal plugin diagnostic — mirrors the persistent-protocol
/// [`crate::hooks::persistent::Diagnostic`] shape so adapter and direct
/// subprocess hooks surface identical log lines.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub severity: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_slug: Option<String>,
}

/// Borrowed context handed to every [`EcosystemAdapter::invoke_plugin`]
/// call.
///
/// Wraps [`BuildContext`] (the REQ-3220 payload) plus the stage and
/// extension-id axes the adapter needs to scope its diagnostics.
/// Held by reference for the duration of the call so adapters never
/// clone the full per-build ctx on the hot path.
#[derive(Debug, Clone, Copy)]
pub struct HookContext<'a> {
    /// The composed stage this invocation belongs to. Redundant with
    /// `input.stage()` at the call site but handed in separately so
    /// adapters can cheaply validate `stage == input.stage()` without
    /// matching on the input variant.
    pub stage: Stage,
    /// Extension id the adapter should attribute log lines / failures
    /// to. Usually matches `manifest.extension_id`; kept separate so
    /// the pipeline's in-flight scoping always wins.
    pub extension_id: &'a str,
    /// Full REQ-3220 build context.
    pub build: &'a BuildContext,
}

impl<'a> HookContext<'a> {
    /// Build a [`HookContext`] from its three fields. Primarily exposed
    /// for tests and conformance harness fixtures.
    pub fn new(stage: Stage, extension_id: &'a str, build: &'a BuildContext) -> Self {
        Self {
            stage,
            extension_id,
            build,
        }
    }
}

// ── Conformance harness (TEST-3302) ─────────────────────────────────────────

/// Result of [`run_conformance`] — one row per trait-method check.
#[derive(Debug, Clone, PartialEq)]
pub struct ConformanceReport {
    pub adapter_id: String,
    pub checks: BTreeMap<String, CheckOutcome>,
}

impl ConformanceReport {
    /// `true` when every check passed.
    pub fn is_ok(&self) -> bool {
        self.checks
            .values()
            .all(|c| matches!(c, CheckOutcome::Pass))
    }

    /// Iterator over failing checks — primarily used by the caller's
    /// `assert_*!` line so the failure message names the check.
    pub fn failures(&self) -> Vec<(&str, &str)> {
        self.checks
            .iter()
            .filter_map(|(name, outcome)| match outcome {
                CheckOutcome::Fail(reason) => Some((name.as_str(), reason.as_str())),
                CheckOutcome::Pass => None,
            })
            .collect()
    }
}

/// Per-check outcome inside a [`ConformanceReport`].
#[derive(Debug, Clone, PartialEq)]
pub enum CheckOutcome {
    Pass,
    Fail(String),
}

/// Parameterised TEST-3302 conformance suite.
///
/// Exercises the adapter trait end-to-end against a corpus of
/// zetl-ext fixtures. Downstream adapter tasks call this from their
/// own integration tests to confirm identity of behaviour across
/// mock / pandoc / mdbook / remark. The harness performs:
///
/// 1. **Identity translation** — for each fixture document, round-trip
///    `doc → translate_to_foreign → translate_from_foreign` and assert
///    the canonicalised output equals the canonicalised input. (NFR-3305
///    canonicalisation is done via
///    [`crate::hooks::contract::canonicalise`].)
/// 2. **`id`/`ast_type` non-empty** — trait-level hygiene.
/// 3. **`supported_stages` non-empty** — CON-3302 + CON-3301 consistency.
/// 4. **Probe is callable** — [`EcosystemAdapter::probe`] returns
///    without panicking. Result variant is not asserted (some
///    ecosystems' runtimes won't be on CI paths).
/// 5. **Invocation identity** — for each fixture, build a stage input
///    matching the adapter's first `supported_stages()` entry, invoke
///    the plugin with an identity-manifest, and assert the output
///    matches. The adapter is expected to ship an identity-echo mode
///    for test-only plugins — the mock adapter here is the canonical
///    example.
///
/// The harness never spawns real subprocesses — that's the domain of
/// the per-ecosystem `TEST-330X` golden-HTML integration tests. This
/// one is a pure trait-shape gate.
pub fn run_conformance<A: EcosystemAdapter>(
    adapter: &mut A,
    fixtures: &[ConformanceFixture],
) -> ConformanceReport {
    use crate::hooks::contract::canonicalise;

    let mut checks: BTreeMap<String, CheckOutcome> = BTreeMap::new();
    let adapter_id = adapter.id().to_string();

    // 1. id + ast_type sanity.
    let id_check = if adapter_id.is_empty() {
        CheckOutcome::Fail("adapter id is empty".into())
    } else {
        CheckOutcome::Pass
    };
    checks.insert("id_non_empty".into(), id_check);

    // 2. supported_stages non-empty. Copy the list off the borrow so
    //    the immutable borrow doesn't conflict with the &mut probe()
    //    call below.
    let stages: Vec<Stage> = adapter.supported_stages().to_vec();
    let stages_check = if stages.is_empty() {
        CheckOutcome::Fail("supported_stages is empty".into())
    } else {
        CheckOutcome::Pass
    };
    checks.insert("supported_stages_non_empty".into(), stages_check);

    // 3. probe() is callable without panic — this is a smoke check, not
    //    an availability assertion.
    let _ = adapter.probe();
    checks.insert("probe_callable".into(), CheckOutcome::Pass);

    // 4. Translation round-trip across the fixture corpus.
    let mut translate_fail: Option<String> = None;
    for fx in fixtures {
        match adapter.translate_to_foreign(&fx.document) {
            Ok(foreign) => match adapter.translate_from_foreign(foreign) {
                Ok(back) => {
                    if canonicalise(&fx.document) != canonicalise(&back) {
                        translate_fail = Some(format!(
                            "fixture '{}' did not round-trip under canonicalisation",
                            fx.name
                        ));
                        break;
                    }
                }
                Err(e) => {
                    translate_fail = Some(format!(
                        "fixture '{}' failed translate_from_foreign: {e}",
                        fx.name
                    ));
                    break;
                }
            },
            Err(e) => {
                translate_fail = Some(format!(
                    "fixture '{}' failed translate_to_foreign: {e}",
                    fx.name
                ));
                break;
            }
        }
    }
    let translate_check = match translate_fail {
        Some(reason) => CheckOutcome::Fail(reason),
        None => CheckOutcome::Pass,
    };
    checks.insert("translate_round_trip_identity".into(), translate_check);

    // 5. invoke_plugin identity round-trip on the adapter's first
    //    supported stage.
    let invoke_check = match stages.first().copied() {
        None => CheckOutcome::Fail("no supported_stages to exercise invoke_plugin on".into()),
        Some(stage) => run_invoke_conformance(adapter, stage, fixtures),
    };
    checks.insert("invoke_plugin_identity".into(), invoke_check);

    ConformanceReport { adapter_id, checks }
}

fn run_invoke_conformance<A: EcosystemAdapter>(
    adapter: &mut A,
    stage: Stage,
    fixtures: &[ConformanceFixture],
) -> CheckOutcome {
    use crate::hooks::build_context::{BuildMode, PageMeta};

    let fixture = match fixtures.first() {
        Some(fx) => fx,
        None => return CheckOutcome::Pass,
    };

    let page = PageMeta::synthetic("Conformance", "conformance");
    let build = BuildContext::new(BuildMode::Build, PathBuf::from("/tmp/conformance"), page);
    let ctx = HookContext::new(stage, "conformance-mock", &build);

    let input = match stage {
        Stage::PreParse => StageInput::PreParse {
            markdown: fixture.sample_markdown.clone(),
        },
        Stage::Transform => match adapter.translate_to_foreign(&fixture.document) {
            Ok(foreign) => StageInput::Transform { foreign },
            Err(e) => {
                return CheckOutcome::Fail(format!(
                    "could not prepare Transform input for fixture '{}': {e}",
                    fixture.name
                ));
            }
        },
        Stage::PostRender => StageInput::PostRender {
            html: fixture.sample_html.clone(),
        },
    };

    let manifest = PluginManifest {
        extension_id: "conformance-mock".into(),
        executable: PathBuf::from("/nonexistent/conformance-mock"),
        args: Vec::new(),
        options: Value::Null,
        timeout: Duration::from_secs(5),
    };

    let expected_stage = input.stage();
    match adapter.invoke_plugin(&manifest, input, &ctx) {
        PluginResponse::Success { output, .. } => {
            if output.stage() != expected_stage {
                CheckOutcome::Fail(format!(
                    "invoke_plugin returned stage {} for input stage {}",
                    output.stage(),
                    expected_stage
                ))
            } else {
                CheckOutcome::Pass
            }
        }
        PluginResponse::Error { reason, detail, .. } => CheckOutcome::Fail(format!(
            "invoke_plugin returned Error (reason={reason}, detail={detail})"
        )),
    }
}

/// One entry in the conformance fixture corpus. Kept lightweight —
/// adapters' own integration tests extend the corpus with
/// ecosystem-specific edge cases (`pandoc-crossref` attrs, `mdast`
/// wikiLink markers, …); this module's corpus covers the zetl-ext
/// baseline every adapter must survive.
#[derive(Debug, Clone)]
pub struct ConformanceFixture {
    pub name: String,
    pub document: Document,
    pub sample_markdown: String,
    pub sample_html: String,
}

/// Default v1 conformance corpus — small, hand-picked documents that
/// exercise the zetl-ext node types every adapter is expected to carry
/// across the foreign boundary.
pub fn default_fixtures() -> Vec<ConformanceFixture> {
    use crate::hooks::ast::{
        Block, Document, DocumentKind, Paragraph, Position, Text, Wikilink, AST_VERSION,
    };

    fn doc_with(children: Vec<Block>) -> Document {
        Document {
            ast_version: AST_VERSION.to_string(),
            kind: DocumentKind::Document,
            position: Position::origin(),
            frontmatter: None,
            children,
        }
    }

    fn para(children: Vec<crate::hooks::ast::Inline>) -> Block {
        Block::Paragraph(Paragraph {
            position: Position::origin(),
            children,
        })
    }

    fn text(s: &str) -> crate::hooks::ast::Inline {
        crate::hooks::ast::Inline::Text(Text {
            position: Position::origin(),
            text: s.into(),
        })
    }

    fn wikilink(target: &str) -> crate::hooks::ast::Inline {
        crate::hooks::ast::Inline::Wikilink(Wikilink {
            position: Position::origin(),
            target: target.into(),
            alias: None,
            heading: None,
            block_id: None,
            predicates: Vec::new(),
        })
    }

    vec![
        ConformanceFixture {
            name: "empty".into(),
            document: doc_with(vec![]),
            sample_markdown: String::new(),
            sample_html: String::new(),
        },
        ConformanceFixture {
            name: "plain-paragraph".into(),
            document: doc_with(vec![para(vec![text("hello world")])]),
            sample_markdown: "hello world\n".into(),
            sample_html: "<p>hello world</p>".into(),
        },
        ConformanceFixture {
            name: "paragraph-with-wikilink".into(),
            document: doc_with(vec![para(vec![
                text("see "),
                wikilink("Other"),
                text(" for details"),
            ])]),
            sample_markdown: "see [[Other]] for details\n".into(),
            sample_html: "<p>see <a href=\"Other\">Other</a> for details</p>".into(),
        },
    ]
}

// ── MockEcosystemAdapter ────────────────────────────────────────────────────

/// Identity-echo adapter used by the TEST-3302 conformance harness and
/// by downstream composition tests that need *some* adapter wired in
/// without pulling a real ecosystem's compile-time cost.
///
/// - [`Self::probe`] always reports `RuntimeStatus::Available`.
/// - [`Self::translate_to_foreign`] / [`Self::translate_from_foreign`]
///   use `serde_json` to round-trip a zetl-ext document exactly
///   (byte-identical round trip — the mock's "foreign AST" is just
///   zetl-ext JSON).
/// - [`Self::invoke_plugin`] returns the input verbatim wrapped in
///   [`PluginResponse::Success`].
///
/// Mutability is preserved to match the trait signature, but the mock
/// holds no state beyond its declared stages.
pub struct MockEcosystemAdapter {
    id: String,
    ast_type: AstType,
    stages: Vec<Stage>,
}

impl Default for MockEcosystemAdapter {
    fn default() -> Self {
        Self {
            id: "mock".into(),
            ast_type: AstType::ZetlExt,
            stages: vec![Stage::Transform],
        }
    }
}

impl MockEcosystemAdapter {
    /// Construct a mock with a custom stage list — useful for testing
    /// the pre-parse / post-render branches of [`run_conformance`].
    pub fn with_stages(mut self, stages: Vec<Stage>) -> Self {
        self.stages = stages;
        self
    }

    /// Override the adapter id.
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    /// Override the declared AST type.
    pub fn with_ast_type(mut self, ast_type: AstType) -> Self {
        self.ast_type = ast_type;
        self
    }
}

impl EcosystemAdapter for MockEcosystemAdapter {
    fn id(&self) -> &str {
        &self.id
    }

    fn ast_type(&self) -> AstType {
        self.ast_type
    }

    fn probe(&mut self) -> RuntimeStatus {
        RuntimeStatus::Available {
            executable: None,
            version: "mock-1.0.0".into(),
        }
    }

    fn supported_stages(&self) -> &[Stage] {
        &self.stages
    }

    fn translate_to_foreign(&self, doc: &Document) -> Result<Value, TranslationError> {
        serde_json::to_value(doc)
            .map_err(|e| TranslationError::to_foreign(self.ast_type, e.to_string()))
    }

    fn translate_from_foreign(&self, foreign: Value) -> Result<Document, TranslationError> {
        serde_json::from_value(foreign)
            .map_err(|e| TranslationError::from_foreign(self.ast_type, e.to_string()))
    }

    fn invoke_plugin(
        &mut self,
        manifest: &PluginManifest,
        input: StageInput,
        context: &HookContext<'_>,
    ) -> PluginResponse {
        let start = std::time::Instant::now();
        // Reject stages outside the adapter's declared support list —
        // the trait contract says `unsupported_stage` rather than
        // panicking so REQ-3207 can record a failure record.
        if !self.stages.contains(&context.stage) {
            return PluginResponse::error(
                "unsupported_stage",
                format!(
                    "mock adapter '{}' does not support stage {} (supports: {:?})",
                    manifest.extension_id, context.stage, self.stages
                ),
                start.elapsed(),
            );
        }
        let output = match input {
            StageInput::PreParse { markdown } => StageOutput::PreParse { markdown },
            StageInput::Transform { foreign } => StageOutput::Transform { foreign },
            StageInput::PostRender { html } => StageOutput::PostRender { html },
        };
        PluginResponse::success(output, start.elapsed())
    }
}

impl std::fmt::Debug for MockEcosystemAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockEcosystemAdapter")
            .field("id", &self.id)
            .field("ast_type", &self.ast_type)
            .field("stages", &self.stages)
            .finish()
    }
}

/// Convenience constructor for [`crate::ecosystems::EcosystemEntry`]'s
/// `adapter_ctor` field: returns a boxed default [`MockEcosystemAdapter`].
/// Per-ecosystem tasks will replace this with their own ctor.
pub fn mock_adapter_ctor() -> Box<dyn EcosystemAdapter> {
    Box::new(MockEcosystemAdapter::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::build_context::{BuildMode, PageMeta};

    fn mock_ctx(stage: Stage) -> (BuildContext, HookContext<'static>) {
        // HookContext borrows BuildContext — the test-only helper leaks
        // the BuildContext behind a Box to get a 'static borrow so each
        // test is self-contained.
        let page = PageMeta::synthetic("MockPage", "mock-page");
        let build = Box::leak(Box::new(BuildContext::new(
            BuildMode::Build,
            PathBuf::from("/tmp/mock"),
            page,
        )));
        let ctx = HookContext::new(stage, "mock", build);
        (build.clone(), ctx)
    }

    #[test]
    fn mock_adapter_passes_default_conformance_suite() {
        let mut adapter = MockEcosystemAdapter::default();
        let fixtures = default_fixtures();
        let report = run_conformance(&mut adapter, &fixtures);
        assert!(
            report.is_ok(),
            "mock adapter conformance failed: {:?}",
            report.failures()
        );
        assert_eq!(report.adapter_id, "mock");
    }

    #[test]
    fn mock_adapter_probe_is_available() {
        let mut adapter = MockEcosystemAdapter::default();
        let status = adapter.probe();
        assert!(status.is_available());
        assert_eq!(status.tag(), "available");
    }

    #[test]
    fn mock_adapter_reports_its_stages() {
        let adapter = MockEcosystemAdapter::default();
        assert_eq!(adapter.supported_stages(), &[Stage::Transform]);
    }

    #[test]
    fn mock_adapter_translate_round_trips() {
        let adapter = MockEcosystemAdapter::default();
        let fixtures = default_fixtures();
        for fx in &fixtures {
            let foreign = adapter.translate_to_foreign(&fx.document).unwrap();
            let back = adapter.translate_from_foreign(foreign).unwrap();
            assert_eq!(
                back, fx.document,
                "round-trip failed for fixture '{}'",
                fx.name
            );
        }
    }

    #[test]
    fn mock_adapter_invoke_returns_identity() {
        let mut adapter = MockEcosystemAdapter::default();
        let (_build, ctx) = mock_ctx(Stage::Transform);
        let manifest = PluginManifest {
            extension_id: "mock".into(),
            executable: PathBuf::from("/nonexistent"),
            args: Vec::new(),
            options: Value::Null,
            timeout: Duration::from_secs(1),
        };
        let input = StageInput::Transform {
            foreign: serde_json::json!({"hello": "world"}),
        };
        let response = adapter.invoke_plugin(&manifest, input, &ctx);
        match response {
            PluginResponse::Success { output, .. } => match output {
                StageOutput::Transform { foreign } => {
                    assert_eq!(foreign, serde_json::json!({"hello": "world"}));
                }
                other => panic!("expected Transform output, got {other:?}"),
            },
            PluginResponse::Error { reason, detail, .. } => {
                panic!("expected Success, got Error(reason={reason}, detail={detail})");
            }
        }
    }

    #[test]
    fn mock_adapter_rejects_unsupported_stage() {
        let mut adapter = MockEcosystemAdapter::default().with_stages(vec![Stage::Transform]);
        let (_build, ctx) = mock_ctx(Stage::PreParse);
        let manifest = PluginManifest {
            extension_id: "mock".into(),
            executable: PathBuf::from("/nonexistent"),
            args: Vec::new(),
            options: Value::Null,
            timeout: Duration::from_secs(1),
        };
        let input = StageInput::PreParse {
            markdown: "body".into(),
        };
        let response = adapter.invoke_plugin(&manifest, input, &ctx);
        match response {
            PluginResponse::Error { reason, .. } => {
                assert_eq!(reason, "unsupported_stage");
            }
            other => panic!("expected unsupported_stage Error, got {other:?}"),
        }
    }

    #[test]
    fn runtime_status_tags_are_canonical() {
        let available = RuntimeStatus::Available {
            executable: None,
            version: "1".into(),
        };
        let outdated = RuntimeStatus::OutdatedVersion {
            executable: PathBuf::from("/bin/x"),
            found: "0.1".into(),
            required: "1.0".into(),
            hint: "upgrade".into(),
        };
        let missing = RuntimeStatus::Missing {
            binary: "x".into(),
            hint: "install".into(),
        };
        let failed = RuntimeStatus::ProbeFailed {
            detail: "eperm".into(),
        };
        assert_eq!(available.tag(), "available");
        assert_eq!(outdated.tag(), "outdated");
        assert_eq!(missing.tag(), "missing");
        assert_eq!(failed.tag(), "probe-failed");
        assert!(available.is_available());
        assert!(!outdated.is_available());
        assert!(!missing.is_available());
        assert!(!failed.is_available());
    }

    #[test]
    fn stage_input_and_output_stage_helpers_agree() {
        let input = StageInput::Transform {
            foreign: Value::Null,
        };
        assert_eq!(input.stage(), Stage::Transform);
        let output = StageOutput::PreParse {
            markdown: "x".into(),
        };
        assert_eq!(output.stage(), Stage::PreParse);
    }

    #[test]
    fn conformance_report_flags_failures() {
        // Build a deliberately-broken adapter that fails translate.
        struct BrokenAdapter;
        impl EcosystemAdapter for BrokenAdapter {
            fn id(&self) -> &str {
                "broken"
            }
            fn ast_type(&self) -> AstType {
                AstType::ZetlExt
            }
            fn probe(&mut self) -> RuntimeStatus {
                RuntimeStatus::Missing {
                    binary: "none".into(),
                    hint: "".into(),
                }
            }
            fn supported_stages(&self) -> &[Stage] {
                &[Stage::Transform]
            }
            fn translate_to_foreign(&self, _doc: &Document) -> Result<Value, TranslationError> {
                Err(TranslationError::to_foreign(AstType::ZetlExt, "boom"))
            }
            fn translate_from_foreign(
                &self,
                _foreign: Value,
            ) -> Result<Document, TranslationError> {
                Err(TranslationError::from_foreign(AstType::ZetlExt, "boom"))
            }
            fn invoke_plugin(
                &mut self,
                _m: &PluginManifest,
                _i: StageInput,
                _c: &HookContext<'_>,
            ) -> PluginResponse {
                PluginResponse::error("broken", "boom", Duration::default())
            }
        }

        let mut adapter = BrokenAdapter;
        let fixtures = default_fixtures();
        let report = run_conformance(&mut adapter, &fixtures);
        assert!(!report.is_ok());
        let failures = report.failures();
        assert!(
            failures
                .iter()
                .any(|(name, _)| *name == "translate_round_trip_identity"),
            "expected translate_round_trip_identity failure, got {failures:?}"
        );
        assert!(
            failures
                .iter()
                .any(|(name, _)| *name == "invoke_plugin_identity"),
            "expected invoke_plugin_identity failure, got {failures:?}"
        );
    }

    #[test]
    fn plugin_manifest_default_timeout_is_thirty_seconds() {
        assert_eq!(PluginManifest::default_timeout(), Duration::from_secs(30));
    }

    #[test]
    fn mock_adapter_ctor_returns_box_dyn() {
        let adapter: Box<dyn EcosystemAdapter> = mock_adapter_ctor();
        assert_eq!(adapter.id(), "mock");
        assert_eq!(adapter.ast_type(), AstType::ZetlExt);
    }
}
