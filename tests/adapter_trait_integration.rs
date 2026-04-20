//! Integration gate for the `EcosystemAdapter` trait (SPEC-033
//! REQ-3302 / CON-3302 / TEST-3302).
//!
//! This file is the public-facing conformance harness — it exercises
//! the trait through the `zetl::ecosystems` surface rather than the
//! module-local imports used by the unit tests next to
//! `src/ecosystems/adapter.rs`. The harness here is the pattern
//! downstream adapter tasks follow (task-pandoc-adapter,
//! task-mdbook-adapter, task-remark-harness) — swap
//! [`MockEcosystemAdapter`] for the real adapter, same assertions pass.
//!
//! TEST-3302 acceptance: trait-conformance passes on a mock adapter.

use std::path::PathBuf;
use std::time::Duration;

use zetl::ecosystems::{
    adapter::{
        run_conformance, CheckOutcome, EcosystemAdapter, HookContext, MockEcosystemAdapter,
        PluginManifest, PluginResponse, RuntimeStatus, StageInput, StageOutput,
    },
    default_fixtures,
};
use zetl::hooks::ast::Document;
use zetl::hooks::build_context::{BuildContext, BuildMode, PageMeta};
use zetl::hooks::pipeline::Stage;
use zetl::hooks::translators::{AstType, TranslationError};

fn mock_build_context() -> BuildContext {
    BuildContext::new(
        BuildMode::Build,
        PathBuf::from("/tmp/adapter-conformance"),
        PageMeta::synthetic("Conformance", "conformance"),
    )
}

// ── TEST-3302: adapter-trait conformance on the mock ────────────────────

#[test]
fn test_3302_mock_adapter_passes_conformance_suite() {
    let mut adapter = MockEcosystemAdapter::default();
    let fixtures = default_fixtures();
    let report = run_conformance(&mut adapter, &fixtures);

    assert!(
        report.is_ok(),
        "mock adapter should pass TEST-3302 conformance; failures: {:?}",
        report.failures()
    );
    assert_eq!(report.adapter_id, "mock");

    // Every check must be recorded — the per-check granularity is the
    // artifact downstream tasks consume when diagnosing a broken real
    // adapter.
    assert!(report.checks.contains_key("id_non_empty"));
    assert!(report.checks.contains_key("supported_stages_non_empty"));
    assert!(report.checks.contains_key("probe_callable"));
    assert!(report.checks.contains_key("translate_round_trip_identity"));
    assert!(report.checks.contains_key("invoke_plugin_identity"));
    for (name, outcome) in &report.checks {
        assert!(
            matches!(outcome, CheckOutcome::Pass),
            "check '{name}' should pass for the mock adapter, got {outcome:?}"
        );
    }
}

#[test]
fn test_3302_conformance_covers_every_canonical_fixture() {
    // The default corpus is the zetl-ext baseline every adapter must
    // carry across the foreign boundary (empty / paragraph / wikilink).
    // Pin the shape so downstream contributors adding a fixture here
    // notice that every adapter's round-trip logic gets re-exercised.
    let fixtures = default_fixtures();
    let names: Vec<&str> = fixtures.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["empty", "plain-paragraph", "paragraph-with-wikilink"],
        "default_fixtures corpus changed; update downstream adapter \
         tests that pin this list"
    );
}

// ── Trait-shape gate (CON-3302 signature) ──────────────────────────────

#[test]
fn mock_adapter_exposes_con_3302_surface() {
    // Each method is called through the trait once, so if any signature
    // drifts at compile time this test fails to link.
    let mut adapter: Box<dyn EcosystemAdapter> = Box::new(MockEcosystemAdapter::default());
    assert_eq!(adapter.id(), "mock");
    assert_eq!(adapter.ast_type(), AstType::ZetlExt);
    assert!(adapter.probe().is_available());
    assert_eq!(adapter.supported_stages(), &[Stage::Transform]);

    let doc = Document {
        ast_version: "1.0".into(),
        kind: zetl::hooks::ast::DocumentKind::Document,
        position: zetl::hooks::ast::Position::origin(),
        frontmatter: None,
        children: vec![],
    };
    let foreign = adapter
        .translate_to_foreign(&doc)
        .expect("translate_to_foreign");
    let back = adapter
        .translate_from_foreign(foreign)
        .expect("translate_from_foreign");
    assert_eq!(back, doc);

    let build = mock_build_context();
    let ctx = HookContext::new(Stage::Transform, "mock", &build);
    let manifest = PluginManifest {
        extension_id: "mock".into(),
        executable: PathBuf::from("/nonexistent/mock"),
        args: Vec::new(),
        options: serde_json::Value::Null,
        timeout: Duration::from_secs(1),
    };
    let input = StageInput::Transform {
        foreign: serde_json::json!({"ping": 1}),
    };
    let response = adapter.invoke_plugin(&manifest, input, &ctx);
    match response {
        PluginResponse::Success {
            output, duration, ..
        } => {
            // Identity adapter returns its input untouched.
            assert_eq!(
                output,
                StageOutput::Transform {
                    foreign: serde_json::json!({"ping": 1}),
                }
            );
            // duration is measured by the adapter; always >= 0 is the
            // only contract we can assert portably.
            let _ = duration;
        }
        other => panic!("expected Success, got {other:?}"),
    }
}

#[test]
fn adapter_trait_is_object_safe() {
    // CON-3301 requires `fn() -> Box<dyn EcosystemAdapter>` for the
    // registry's `adapter_ctor` field. Object safety is an API-shape
    // invariant — lose it and the registry can't store a ctor. Pin it
    // here so downstream changes to the trait catch this regression
    // before the registry's adapter_ctor field is wired.
    fn take_boxed(_: Box<dyn EcosystemAdapter>) {}
    take_boxed(Box::new(MockEcosystemAdapter::default()));
}

#[test]
fn mock_adapter_configurable_stages_round_trip() {
    // Vary the declared stages and confirm the invoke path respects
    // them. Downstream adapters (pandoc: transform, mdbook: pre-parse,
    // remark: transform) rely on this stage-scoping behaviour.
    let mut adapter =
        MockEcosystemAdapter::default().with_stages(vec![Stage::PreParse, Stage::PostRender]);
    let build = mock_build_context();

    // PreParse should succeed — it's in the stages list.
    let ctx = HookContext::new(Stage::PreParse, "mock", &build);
    let manifest = PluginManifest {
        extension_id: "mock".into(),
        executable: PathBuf::from("/nonexistent"),
        args: vec![],
        options: serde_json::Value::Null,
        timeout: Duration::from_secs(1),
    };
    let input = StageInput::PreParse {
        markdown: "# heading".into(),
    };
    match adapter.invoke_plugin(&manifest, input, &ctx) {
        PluginResponse::Success { output, .. } => assert_eq!(
            output,
            StageOutput::PreParse {
                markdown: "# heading".into()
            }
        ),
        other => panic!("expected Success on supported stage, got {other:?}"),
    }

    // Transform is NOT in the stages list — the adapter returns
    // `unsupported_stage` per the trait contract.
    let ctx = HookContext::new(Stage::Transform, "mock", &build);
    let input = StageInput::Transform {
        foreign: serde_json::json!({}),
    };
    match adapter.invoke_plugin(&manifest, input, &ctx) {
        PluginResponse::Error { reason, .. } => assert_eq!(reason, "unsupported_stage"),
        other => panic!("expected unsupported_stage Error, got {other:?}"),
    }
}

// ── Custom-adapter conformance ─────────────────────────────────────────

/// A thin custom adapter used to show the conformance harness generalises
/// beyond the mock. This is the exact pattern task-pandoc-adapter et al.
/// will follow: wrap whatever transport the adapter owns (subprocess
/// spawn, in-process harness, …) behind the trait and call
/// `run_conformance`.
struct IdentityZetlAdapter {
    stages: Vec<Stage>,
}

impl EcosystemAdapter for IdentityZetlAdapter {
    fn id(&self) -> &str {
        "identity-zetl"
    }

    fn ast_type(&self) -> AstType {
        AstType::ZetlExt
    }

    fn probe(&mut self) -> RuntimeStatus {
        RuntimeStatus::Available {
            executable: None,
            version: "test-identity-1".into(),
        }
    }

    fn supported_stages(&self) -> &[Stage] {
        &self.stages
    }

    fn translate_to_foreign(&self, doc: &Document) -> Result<serde_json::Value, TranslationError> {
        serde_json::to_value(doc)
            .map_err(|e| TranslationError::to_foreign(AstType::ZetlExt, e.to_string()))
    }

    fn translate_from_foreign(
        &self,
        foreign: serde_json::Value,
    ) -> Result<Document, TranslationError> {
        serde_json::from_value(foreign)
            .map_err(|e| TranslationError::from_foreign(AstType::ZetlExt, e.to_string()))
    }

    fn invoke_plugin(
        &mut self,
        _manifest: &PluginManifest,
        input: StageInput,
        _ctx: &HookContext<'_>,
    ) -> PluginResponse {
        let output = match input {
            StageInput::PreParse { markdown } => StageOutput::PreParse { markdown },
            StageInput::Transform { foreign } => StageOutput::Transform { foreign },
            StageInput::PostRender { html } => StageOutput::PostRender { html },
        };
        PluginResponse::success(output, Duration::from_millis(0))
    }
}

#[test]
fn custom_identity_adapter_passes_conformance() {
    let mut adapter = IdentityZetlAdapter {
        stages: vec![Stage::Transform],
    };
    let fixtures = default_fixtures();
    let report = run_conformance(&mut adapter, &fixtures);
    assert!(
        report.is_ok(),
        "identity adapter should pass conformance; failures: {:?}",
        report.failures()
    );
    assert_eq!(report.adapter_id, "identity-zetl");
}
