//! Integration tests for the remark adapter (SPEC-033 REQ-3305 /
//! CON-3305 / ADR-3304 / TEST-3305).
//!
//! These tests exercise the adapter through the public
//! `zetl::ecosystems::remark` surface. They are unix-only because the
//! harness relies on a POSIX subprocess model (pipes + SIGKILL watchdog)
//! and require `node` on PATH — absent node, every test gates out early
//! with a skip message rather than failing.
//!
//! Real remark plugin fixtures live under the matrix seed
//! (task-remark-matrix-seed); these tests stand up a synthetic
//! `unified`-alike in a tempdir so the end-to-end plumbing is verified
//! without depending on `npm install` in CI. The round-trip asserted
//! here is the adapter's own transport: harness start + plugin load +
//! plugin apply + shutdown, with mdast JSON flowing across.

#![cfg(all(feature = "ecosystem-remark", unix))]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::json;

use zetl::ecosystems::adapter::{
    run_conformance, EcosystemAdapter, HookContext, PluginManifest, PluginResponse, StageInput,
    StageOutput,
};
use zetl::ecosystems::default_fixtures;
use zetl::ecosystems::remark::{IsolationMode, RemarkAdapter};
use zetl::hooks::build_context::{BuildContext, BuildMode, PageMeta};
use zetl::hooks::pipeline::Stage;
use zetl::hooks::translators::AstType;

/// Skip-marker: emit a warning to stderr and return `true` if node is
/// missing. Every test that spawns the harness early-outs on this so the
/// suite runs green on a CI box without node.
fn node_available() -> bool {
    std::process::Command::new("node")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

macro_rules! skip_without_node {
    () => {
        if !node_available() {
            eprintln!("skipping: node not on PATH");
            return;
        }
    };
}

fn build_ctx(vault: PathBuf) -> BuildContext {
    BuildContext::new(
        BuildMode::Build,
        vault,
        PageMeta::synthetic("Page", "page"),
    )
}

/// Build a self-contained tempdir with a fake `unified` + a fake
/// `tag-plugin` package that the harness can resolve via `import()`.
///
/// The fake `unified()` returns a processor whose `.use(plugin, options)`
/// stashes the plugin's returned transformer, and whose `.run(ast)`
/// applies the transformer and returns the tree. This is a minimal
/// shim that mimics the plugin-application contract the real
/// unified/remark ecosystem exposes — enough to verify zetl's own
/// plumbing end-to-end without a network `npm install`.
fn setup_fake_unified_env() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // Layout:
    //   <root>/node_modules/unified/package.json
    //   <root>/node_modules/unified/index.mjs
    //   <root>/node_modules/tag-plugin/package.json
    //   <root>/node_modules/tag-plugin/index.mjs
    //   <root>/node_modules/counting-plugin/package.json
    //   <root>/node_modules/counting-plugin/index.mjs

    write_package(
        root,
        "unified",
        r#"{
            "name": "unified",
            "version": "11.0.0",
            "type": "module",
            "main": "index.mjs",
            "exports": "./index.mjs"
        }"#,
        r#"
            // Minimal unified-alike for integration tests.
            export function unified() {
                const transformers = [];
                const proc = {
                    use(plugin, options) {
                        const t = plugin(options);
                        if (typeof t === 'function') transformers.push(t);
                        return proc;
                    },
                    async run(tree) {
                        let current = tree;
                        for (const t of transformers) {
                            const result = await t(current);
                            if (result !== undefined) current = result;
                        }
                        return current;
                    },
                };
                return proc;
            }
            export default { unified };
        "#,
    );

    write_package(
        root,
        "tag-plugin",
        r#"{
            "name": "tag-plugin",
            "version": "1.0.0",
            "type": "module",
            "main": "index.mjs",
            "exports": "./index.mjs"
        }"#,
        r#"
            // Tags the root node with a user-supplied tag under `data.tag`.
            export default function tagPlugin(options = {}) {
                const tag = options.tag || 'untagged';
                return (tree) => {
                    tree.data = tree.data || {};
                    tree.data.tag = tag;
                    tree.data.applied_at = Date.now();
                    return tree;
                };
            }
        "#,
    );

    write_package(
        root,
        "counting-plugin",
        r#"{
            "name": "counting-plugin",
            "version": "1.0.0",
            "type": "module",
            "main": "index.mjs",
            "exports": "./index.mjs"
        }"#,
        r#"
            // A plugin that increments a module-level counter each time
            // it's applied. Used to observe module-cache state across
            // invocations: shared harness → counter persists;
            // fresh-context harness → counter resets every call.
            let counter = 0;
            export default function countingPlugin() {
                return (tree) => {
                    counter += 1;
                    tree.data = tree.data || {};
                    tree.data.invocation_count = counter;
                    return tree;
                };
            }
        "#,
    );

    tmp
}

fn write_package(root: &Path, name: &str, package_json: &str, index_mjs: &str) {
    let dir = root.join("node_modules").join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("package.json"), package_json).unwrap();
    fs::write(dir.join("index.mjs"), index_mjs).unwrap();
}

// ── TEST-3302: conformance on the real remark adapter ───────────────────

/// Conformance harness: every non-invoke check (id, supported_stages,
/// probe, translate) must pass regardless of whether node is installed.
/// `invoke_plugin_identity` needs a live harness — without node it fails,
/// which is the expected behaviour.
#[test]
fn test_3302_remark_adapter_passes_conformance_suite() {
    let mut adapter = RemarkAdapter::new();
    let fixtures = default_fixtures();
    let report = run_conformance(&mut adapter, &fixtures);
    let failures = report.failures();
    if node_available() {
        // With node present the invoke_plugin check still requires a
        // working `unified` import, which we don't assume on the host —
        // so the one allowable failure is still invoke_plugin_identity.
        for (name, _) in &failures {
            assert!(
                *name == "invoke_plugin_identity",
                "unexpected conformance failure '{name}' (failures: {failures:?})"
            );
        }
    } else {
        // Node missing: invoke_plugin_identity is the only allowable
        // failure. Translate + trait-shape checks must pass.
        for (name, _) in &failures {
            assert!(
                *name == "invoke_plugin_identity",
                "unexpected conformance failure '{name}' (failures: {failures:?})"
            );
        }
    }
    assert_eq!(report.adapter_id, "remark");
}

// ── Trait-shape gates ───────────────────────────────────────────────────

#[test]
fn remark_adapter_is_box_dyn_eligible() {
    let adapter: Box<dyn EcosystemAdapter> = Box::new(RemarkAdapter::new());
    assert_eq!(adapter.id(), "remark");
    assert_eq!(adapter.ast_type(), AstType::MdastExt);
    assert_eq!(adapter.supported_stages(), &[Stage::Transform]);
}

#[test]
fn remark_adapter_uses_mdast_ext_ast_type() {
    let adapter = RemarkAdapter::new();
    assert_eq!(adapter.ast_type(), AstType::MdastExt);
}

// ── Probe ──────────────────────────────────────────────────────────────

#[test]
fn probe_populates_node_version_when_available() {
    skip_without_node!();
    let mut adapter = RemarkAdapter::new();
    let status = adapter.probe();
    assert!(
        status.is_available(),
        "probe should report Available when node is on PATH, got {status:?}"
    );
    assert!(
        adapter.node_version().is_some(),
        "probe should cache node_version on Available"
    );
}

// ── Shared-harness end-to-end via synthetic unified shim ───────────────

/// Spawn the harness against a tempdir carrying a fake `unified` + a
/// `tag-plugin` that decorates the mdast root. Round-trip an mdast
/// document through the adapter and assert the tag arrives back on the
/// output ast.
#[test]
fn test_3305_shared_harness_applies_plugin_end_to_end() {
    skip_without_node!();
    let env = setup_fake_unified_env();
    let vault = env.path().to_path_buf();

    let mut adapter = RemarkAdapter::new();
    adapter.probe(); // resolves the node path

    let build = build_ctx(vault);
    let ctx = HookContext::new(Stage::Transform, "tag-plugin-hook", &build);
    let manifest = PluginManifest {
        extension_id: "tag-plugin-hook".into(),
        executable: PathBuf::from("/unused-for-remark"),
        args: vec![],
        options: json!({
            "package": "tag-plugin",
            "plugin_options": {"tag": "integration-test"},
            "isolation": "shared",
        }),
        timeout: Duration::from_secs(10),
    };
    let input = json!({
        "type": "root",
        "children": [
            {"type": "paragraph", "children": [{"type": "text", "value": "hello"}]}
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
                let tag = foreign
                    .get("data")
                    .and_then(|d| d.get("tag"))
                    .and_then(|t| t.as_str());
                assert_eq!(
                    tag,
                    Some("integration-test"),
                    "plugin did not apply — expected tag on output root, got: {foreign}"
                );
            }
            other => panic!("expected Transform output, got {other:?}"),
        },
        PluginResponse::Error { reason, detail, .. } => {
            panic!("end-to-end apply failed: {reason}: {detail}")
        }
    }

    adapter.shutdown(Duration::from_secs(2));
}

/// Shared mode: invoking the same plugin twice on the same adapter
/// reuses the module cache — the counting plugin's counter keeps
/// incrementing across calls (1, 2, …) because the harness stays alive.
#[test]
fn test_3305_shared_mode_preserves_module_state_across_invocations() {
    skip_without_node!();
    let env = setup_fake_unified_env();
    let vault = env.path().to_path_buf();

    let mut adapter = RemarkAdapter::new();
    adapter.probe();

    let build = build_ctx(vault);
    let ctx = HookContext::new(Stage::Transform, "counting-hook", &build);
    let manifest = PluginManifest {
        extension_id: "counting-hook".into(),
        executable: PathBuf::from("/unused-for-remark"),
        args: vec![],
        options: json!({
            "package": "counting-plugin",
            "isolation": "shared",
        }),
        timeout: Duration::from_secs(10),
    };
    let input = json!({"type": "root", "children": []});

    let count1 = invoke_and_get_count(&mut adapter, &manifest, &input, &ctx);
    let count2 = invoke_and_get_count(&mut adapter, &manifest, &input, &ctx);
    let count3 = invoke_and_get_count(&mut adapter, &manifest, &input, &ctx);

    assert_eq!(count1, 1, "first shared invocation should see counter=1");
    assert_eq!(
        count2, 2,
        "second shared invocation should see counter=2 (state preserved)"
    );
    assert_eq!(
        count3, 3,
        "third shared invocation should see counter=3 (state preserved)"
    );

    adapter.shutdown(Duration::from_secs(2));
}

/// Fresh-context mode: each invocation spawns a new Node subprocess, so
/// the counting plugin's module-level counter resets to 1 every time.
#[test]
fn test_3305_fresh_context_mode_clears_state_between_invocations() {
    skip_without_node!();
    let env = setup_fake_unified_env();
    let vault = env.path().to_path_buf();

    let mut adapter = RemarkAdapter::new();
    adapter.probe();

    let build = build_ctx(vault);
    let ctx = HookContext::new(Stage::Transform, "counting-hook-isolated", &build);
    let manifest = PluginManifest {
        extension_id: "counting-hook-isolated".into(),
        executable: PathBuf::from("/unused-for-remark"),
        args: vec![],
        options: json!({
            "package": "counting-plugin",
            "isolation": "fresh-context",
        }),
        timeout: Duration::from_secs(10),
    };
    let input = json!({"type": "root", "children": []});

    let count1 = invoke_and_get_count(&mut adapter, &manifest, &input, &ctx);
    let count2 = invoke_and_get_count(&mut adapter, &manifest, &input, &ctx);

    assert_eq!(
        count1, 1,
        "first fresh-context invocation should see counter=1"
    );
    assert_eq!(
        count2, 1,
        "second fresh-context invocation should ALSO see counter=1 — \
         fresh-context must reset module state"
    );

    adapter.shutdown(Duration::from_secs(2));
}

/// Fresh-context mode surfaces an `info` diagnostic on each
/// invocation so build output records the isolation boundary.
#[test]
fn test_3305_fresh_context_emits_info_diagnostic() {
    skip_without_node!();
    let env = setup_fake_unified_env();
    let vault = env.path().to_path_buf();

    let mut adapter = RemarkAdapter::new();
    adapter.probe();

    let build = build_ctx(vault);
    let ctx = HookContext::new(Stage::Transform, "tag-plugin-hook", &build);
    let manifest = PluginManifest {
        extension_id: "tag-plugin-hook".into(),
        executable: PathBuf::from("/unused-for-remark"),
        args: vec![],
        options: json!({
            "package": "tag-plugin",
            "plugin_options": {"tag": "fresh"},
            "isolation": "fresh-context",
        }),
        timeout: Duration::from_secs(10),
    };
    let input = json!({"type": "root", "children": []});

    let response = adapter.invoke_plugin(
        &manifest,
        StageInput::Transform { foreign: input },
        &ctx,
    );
    match response {
        PluginResponse::Success { diagnostics, .. } => {
            assert!(
                diagnostics.iter().any(|d| d.severity == "info"
                    && d.message.contains("fresh-context")),
                "expected fresh-context info diagnostic, got: {diagnostics:?}"
            );
        }
        PluginResponse::Error { reason, detail, .. } => {
            panic!("fresh-context invocation failed: {reason}: {detail}");
        }
    }
}

// ── Error paths ────────────────────────────────────────────────────────

/// Missing `unified` module surfaces as a typed error (runtime setup
/// problem, not a silent hang).
#[test]
fn test_3305_missing_unified_reports_plugin_error() {
    skip_without_node!();
    // Empty vault → no node_modules/unified. Harness starts but
    // ready-banner reports unified_available=false, and the first
    // apply call surfaces the Rejected error.
    let tmp = tempfile::tempdir().unwrap();
    let vault = tmp.path().to_path_buf();

    let mut adapter = RemarkAdapter::new();
    adapter.probe();

    let build = build_ctx(vault);
    let ctx = HookContext::new(Stage::Transform, "any-plugin", &build);
    let manifest = PluginManifest {
        extension_id: "any-plugin".into(),
        executable: PathBuf::from("/unused-for-remark"),
        args: vec![],
        options: json!({
            "package": "remark-gfm",
            "isolation": "shared",
        }),
        timeout: Duration::from_secs(5),
    };
    let input = json!({"type": "root", "children": []});

    let response = adapter.invoke_plugin(
        &manifest,
        StageInput::Transform { foreign: input },
        &ctx,
    );
    match response {
        PluginResponse::Error { reason, detail, .. } => {
            assert_eq!(
                reason, "plugin_error",
                "expected plugin_error when unified is missing, got reason={reason} detail={detail}"
            );
            assert!(
                detail.contains("unified"),
                "error detail should mention unified, got: {detail}"
            );
        }
        PluginResponse::Success { .. } => {
            panic!("expected error when unified is missing, got success");
        }
    }
}

/// Shutdown is idempotent — calling it twice on a live adapter (once
/// with a live harness, again after it was reaped) doesn't panic.
#[test]
fn test_3305_shutdown_is_idempotent() {
    skip_without_node!();
    let env = setup_fake_unified_env();
    let vault = env.path().to_path_buf();

    let mut adapter = RemarkAdapter::new();
    adapter.probe();
    let build = build_ctx(vault);
    let ctx = HookContext::new(Stage::Transform, "tag-plugin-hook", &build);
    let manifest = PluginManifest {
        extension_id: "tag-plugin-hook".into(),
        executable: PathBuf::from("/unused-for-remark"),
        args: vec![],
        options: json!({
            "package": "tag-plugin",
            "plugin_options": {"tag": "x"},
            "isolation": "shared",
        }),
        timeout: Duration::from_secs(10),
    };
    let input = json!({"type": "root", "children": []});
    let _ = adapter.invoke_plugin(
        &manifest,
        StageInput::Transform { foreign: input },
        &ctx,
    );
    adapter.shutdown(Duration::from_secs(2));
    adapter.shutdown(Duration::from_secs(2));
}

// ── Helpers ────────────────────────────────────────────────────────────

fn invoke_and_get_count(
    adapter: &mut RemarkAdapter,
    manifest: &PluginManifest,
    input: &serde_json::Value,
    ctx: &HookContext<'_>,
) -> u64 {
    let response = adapter.invoke_plugin(
        manifest,
        StageInput::Transform {
            foreign: input.clone(),
        },
        ctx,
    );
    match response {
        PluginResponse::Success { output, .. } => match output {
            StageOutput::Transform { foreign } => foreign
                .get("data")
                .and_then(|d| d.get("invocation_count"))
                .and_then(|c| c.as_u64())
                .unwrap_or_else(|| panic!("no invocation_count in output: {foreign}")),
            other => panic!("expected Transform output, got {other:?}"),
        },
        PluginResponse::Error { reason, detail, .. } => {
            panic!("plugin apply failed: {reason}: {detail}")
        }
    }
}

/// Regression gate: the isolation-mode enum's display form is stable
/// ("shared" / "fresh-context") — those strings ship in log lines and
/// manifests, so a serde rename would be a breaking change.
#[test]
fn isolation_mode_wire_form_is_kebab_case() {
    assert_eq!(IsolationMode::Shared.to_string(), "shared");
    assert_eq!(IsolationMode::FreshContext.to_string(), "fresh-context");
}
