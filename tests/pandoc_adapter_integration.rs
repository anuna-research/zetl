//! Integration tests for the Pandoc adapter (SPEC-033 REQ-3303 /
//! CON-3303 / ADR-3302 / TEST-3303).
//!
//! These tests exercise the adapter through the public
//! `zetl::ecosystems::pandoc` surface — the path downstream build/serve
//! wiring will use when invoking a real pandoc filter. The key shapes
//! validated here:
//!
//! - TEST-3302 conformance (round-trip + invoke-identity) passes on the
//!   Pandoc adapter wired with [`PandocAdapter`].
//! - Filter-mode dispatch — argv[1] = target format, CON-3303 env vars
//!   set, pandoc-types JSON on stdin/stdout — observed end-to-end via
//!   a shell-script identity shim (no pandoc install required).
//! - CON-3303 env-var payload end-to-end: the shim echoes its env into
//!   stderr so the test can assert every required variable arrived.
//! - Mode resolution: `pandoc-citeproc → native` auto-promotion
//!   surfaces the deprecation diagnostic in the success response.
//!
//! Tests are unix-only because the `#!/bin/sh` identity-shim pattern
//! doesn't translate cleanly to Windows; Windows support for the
//! adapter is covered by the unit tests in `src/ecosystems/pandoc.rs`
//! which don't spawn subprocesses.

#![cfg(all(feature = "ecosystem-pandoc", unix))]

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::json;

use zetl::ecosystems::adapter::{
    run_conformance, EcosystemAdapter, HookContext, PluginManifest, PluginResponse, StageInput,
    StageOutput,
};
use zetl::ecosystems::default_fixtures;
use zetl::ecosystems::pandoc::{resolve_mode, PandocAdapter, PandocMode, PandocOptions};
use zetl::hooks::build_context::{BuildContext, BuildMode, PageMeta};
use zetl::hooks::pipeline::Stage;
use zetl::hooks::translators::AstType;

fn build_ctx() -> BuildContext {
    BuildContext::new(
        BuildMode::Build,
        PathBuf::from("/tmp/pandoc-integration"),
        PageMeta::synthetic("Page", "page"),
    )
}

/// Write an executable shim at `path` with the given `#!/bin/sh` body.
fn write_shim(path: &Path, body: &str) {
    let mut f = std::fs::File::create(path).unwrap();
    writeln!(f, "#!/bin/sh").unwrap();
    f.write_all(body.as_bytes()).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

// ── TEST-3302: conformance on the real pandoc adapter ───────────────────

#[test]
fn test_3302_pandoc_adapter_passes_conformance_suite() {
    let mut adapter = PandocAdapter::new();
    let fixtures = default_fixtures();
    let report = run_conformance(&mut adapter, &fixtures);
    // `invoke_plugin_identity` needs a real binary to round-trip through
    // — without pandoc the conformance harness will flag that check as
    // failing. Every other check (id/stages/probe/translate) must pass
    // regardless of runtime availability.
    let failures = report.failures();
    for (name, _) in &failures {
        assert!(
            *name == "invoke_plugin_identity",
            "unexpected conformance failure '{name}' (failures: {failures:?})"
        );
    }
    assert_eq!(report.adapter_id, "pandoc");
}

// ── TEST-3303: filter-mode end-to-end via identity shim ─────────────────

/// Filter-mode sanity check: adapter serialises pandoc-types JSON, the
/// shim cats it back verbatim, adapter parses it and returns it in a
/// `StageOutput::Transform`.
#[test]
fn test_3303_filter_mode_round_trips_through_identity_shim() {
    let tmp = tempfile::tempdir().unwrap();
    let shim = tmp.path().join("pandoc-identity-filter");
    // argv[1] is target format ("html"); filter reads stdin and writes
    // it back on stdout. This is the "do-nothing filter" pattern.
    write_shim(&shim, "cat\n");

    let mut adapter = PandocAdapter::new();
    let ctx_build = build_ctx();
    let ctx = HookContext::new(Stage::Transform, "identity-filter", &ctx_build);
    let manifest = PluginManifest {
        extension_id: "identity-filter".into(),
        executable: shim,
        args: vec![],
        options: json!({"mode": "filter"}),
        timeout: Duration::from_secs(5),
    };
    let input = json!({
        "pandoc-api-version": [1,23,1,0],
        "meta": {"zetl-ast-version": {"t": "MetaInlines", "c": [{"t": "Str", "c": "1.0"}]}},
        "blocks": [
            {"t": "Para", "c": [{"t": "Str", "c": "hello"}]}
        ]
    });
    let response = adapter.invoke_plugin(
        &manifest,
        StageInput::Transform {
            foreign: input.clone(),
        },
        &ctx,
    );
    match response {
        PluginResponse::Success { output, .. } => match output {
            StageOutput::Transform { foreign } => {
                assert_eq!(
                    foreign, input,
                    "identity filter must round-trip pandoc-types JSON unchanged"
                );
            }
            other => panic!("expected Transform output, got {other:?}"),
        },
        PluginResponse::Error { reason, detail, .. } => {
            panic!("filter-mode round-trip failed: {reason}: {detail}");
        }
    }
}

/// CON-3303 env-var payload: the shim captures its env into stderr so
/// we can assert every variable in the CON-3303 table arrived.
#[test]
fn test_3303_filter_mode_sets_con_3303_env_vars() {
    let tmp = tempfile::tempdir().unwrap();
    let shim = tmp.path().join("pandoc-env-spy-filter");
    // Echo the five CON-3303 variables to stderr for the test to
    // assert on, then cat stdin through unchanged so the adapter's JSON
    // parse still succeeds.
    write_shim(
        &shim,
        concat!(
            "echo \"PANDOC_VERSION=${PANDOC_VERSION}\" 1>&2\n",
            "echo \"PANDOC_API_VERSION=${PANDOC_API_VERSION}\" 1>&2\n",
            "echo \"PANDOC_READER_OPTIONS=${PANDOC_READER_OPTIONS}\" 1>&2\n",
            "echo \"PANDOC_WRITER_OPTIONS=${PANDOC_WRITER_OPTIONS}\" 1>&2\n",
            "echo \"PANDOC_SCRIPT_FILE=${PANDOC_SCRIPT_FILE}\" 1>&2\n",
            "echo \"ARGV1=$1\" 1>&2\n",
            "cat\n",
        ),
    );

    let mut adapter = PandocAdapter::new();
    let ctx_build = build_ctx();
    let ctx = HookContext::new(Stage::Transform, "env-spy", &ctx_build);
    let manifest = PluginManifest {
        extension_id: "env-spy".into(),
        executable: shim.clone(),
        args: vec![],
        options: json!({"mode": "filter", "target_format": "html"}),
        timeout: Duration::from_secs(5),
    };
    let input = json!({
        "pandoc-api-version": [1,23,1,0],
        "meta": {},
        "blocks": []
    });
    let response = adapter.invoke_plugin(
        &manifest,
        StageInput::Transform {
            foreign: input.clone(),
        },
        &ctx,
    );
    let (output, stderr) = match response {
        PluginResponse::Success { output, stderr, .. } => (output, stderr),
        PluginResponse::Error { reason, detail, .. } => {
            panic!("env-spy invocation failed: {reason}: {detail}");
        }
    };
    // Every CON-3303 variable must appear in the shim's stderr.
    for required in [
        "PANDOC_VERSION=",
        "PANDOC_API_VERSION=",
        "PANDOC_READER_OPTIONS=",
        "PANDOC_WRITER_OPTIONS=",
        "PANDOC_SCRIPT_FILE=",
    ] {
        assert!(
            stderr.contains(required),
            "missing CON-3303 variable prefix '{required}' in captured stderr: {stderr}"
        );
    }
    // PANDOC_SCRIPT_FILE must be the absolute path we handed to the
    // adapter.
    let want = format!("PANDOC_SCRIPT_FILE={}", shim.display());
    assert!(
        stderr.contains(&want),
        "PANDOC_SCRIPT_FILE not propagated — expected '{want}' in stderr: {stderr}"
    );
    // argv[1] must equal the target_format from options.
    assert!(
        stderr.contains("ARGV1=html"),
        "argv[1] should be target_format='html', stderr: {stderr}"
    );
    // And the cat-through must still have produced a valid round-trip.
    match output {
        StageOutput::Transform { foreign } => assert_eq!(foreign, input),
        other => panic!("expected Transform output, got {other:?}"),
    }
}

// ── TEST-3303n: native mode (exercising resolve_mode wiring) ────────────

#[test]
fn test_3303n_native_mode_diagnostic_on_pandoc_citeproc_promotion() {
    // mode is unset + exec is pandoc-citeproc → native + --citeproc +
    // a deprecation diagnostic. This exercise the resolver only; the
    // actual subprocess dispatch needs pandoc installed which we don't
    // assume. The surrounding `invoke_plugin` either succeeds (pandoc
    // is present) or errors with `runtime_absence` — both encode the
    // correct diagnostic-carrying path.
    let manifest = PluginManifest {
        extension_id: "citeproc-legacy".into(),
        executable: PathBuf::from("/nonexistent/pandoc-citeproc"),
        args: vec![],
        options: serde_json::Value::Null,
        timeout: Duration::from_secs(1),
    };
    let invocation = resolve_mode(
        &manifest,
        &PandocOptions::default(),
        Path::new("/nonexistent/pandoc"),
    );
    assert_eq!(invocation.mode, PandocMode::Native);
    assert!(
        invocation.argv.iter().any(|a| a == "--citeproc"),
        "expected --citeproc injected into native argv, got {:?}",
        invocation.argv
    );
    assert!(
        invocation.argv.windows(2).any(|w| w
            == ["--from".to_string(), "json".to_string()]),
        "native mode must pipe pandoc-types JSON in/out, got {:?}",
        invocation.argv
    );
    assert_eq!(
        invocation.diagnostics.len(),
        1,
        "one deprecation warning expected, got: {:?}",
        invocation.diagnostics
    );
    assert!(invocation.diagnostics[0]
        .message
        .contains("pandoc-citeproc is deprecated"));
}

// ── Trait-shape gates ───────────────────────────────────────────────────

#[test]
fn pandoc_adapter_is_box_dyn_eligible() {
    let adapter: Box<dyn EcosystemAdapter> = Box::new(PandocAdapter::new());
    assert_eq!(adapter.id(), "pandoc");
    assert_eq!(adapter.ast_type(), AstType::PandocExt);
    assert_eq!(adapter.supported_stages(), &[Stage::Transform]);
}

#[test]
fn pandoc_adapter_uses_pandoc_ext_ast_type() {
    // The dispatch layer (SPEC-032 REQ-3221) pairs a manifest's
    // `ast_type` with the adapter's declared `ast_type`. Locking this
    // pin prevents a silent regression where the adapter's declared
    // type drifts away from its translator's output shape.
    let adapter = PandocAdapter::new();
    assert_eq!(adapter.ast_type(), AstType::PandocExt);
}
