//! Integration tests for the mdBook adapter (SPEC-033 REQ-3304 /
//! CON-3304 / ADR-3303 / TEST-3304 + REQ-3309 / TEST-3309).
//!
//! These tests exercise the adapter through the public
//! `zetl::ecosystems::mdbook` surface — the path downstream build/serve
//! wiring will use when invoking a real preprocessor. The shapes
//! validated here:
//!
//! - TEST-3302 conformance passes on the mdBook adapter wired via
//!   [`MdbookAdapter`].
//! - CON-3304 two-call protocol: `<exec> supports html` probe followed
//!   by a real-run `<exec>` invocation with stdin = `[Context, Book]`
//!   JSON and stdout = transformed `Book` JSON.
//! - REQ-3309 envelope shape: synthetic `[Context, Book]` array with
//!   every required field; the chapter path/source_path/name fields
//!   carry the page's slug + name.
//! - `supports html` exit 0 gates the real run; exit 1 surfaces a
//!   `contract_violation` error with no real-run spawned.
//! - Per-page vs per-vault scope: `scope = "vault"` surfaces a
//!   warning diagnostic (v1 runs per page; vault batching is a later
//!   phase).
//!
//! Tests are unix-only because they rely on `#!/bin/sh` shims to stand
//! in for real preprocessors; Windows support for the adapter's pure
//! path (envelope construction, option parsing, probe classification)
//! is covered by the unit tests in `src/ecosystems/mdbook.rs`.

#![cfg(all(feature = "ecosystem-mdbook", unix))]

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};

use zetl::ecosystems::adapter::{
    run_conformance, EcosystemAdapter, HookContext, PluginManifest, PluginResponse, StageInput,
    StageOutput,
};
use zetl::ecosystems::default_fixtures;
use zetl::ecosystems::manifest::MdbookScope;
use zetl::ecosystems::mdbook::{
    build_envelope_for_page, extract_chapter_content, mdbook_adapter_ctor, validate_envelope,
    MdbookAdapter, MdbookOptions, ENVELOPE_SCHEMA_PATH, MDBOOK_PROTOCOL_VERSION,
};
use zetl::hooks::build_context::{BuildContext, BuildMode, PageMeta};
use zetl::hooks::pipeline::Stage;
use zetl::hooks::translators::AstType;

fn build_ctx() -> BuildContext {
    BuildContext::new(
        BuildMode::Build,
        PathBuf::from("/tmp/mdbook-integration"),
        PageMeta::synthetic("Chapter One", "chapter-one"),
    )
}

/// Write an executable shim at `path` with the given `#!/bin/sh` body.
fn write_shim(path: &Path, body: &str) {
    let mut f = std::fs::File::create(path).unwrap();
    writeln!(f, "#!/bin/sh").unwrap();
    f.write_all(body.as_bytes()).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

// ── TEST-3302: conformance on the real mdbook adapter ───────────────────

/// Conformance harness: every non-invoke check (id, supported_stages,
/// probe, translate) must pass regardless of whether mdbook or any
/// preprocessors are installed. `invoke_plugin_identity` needs a live
/// binary to probe, so it's the expected failure without one.
#[test]
fn test_3302_mdbook_adapter_passes_conformance_suite() {
    let mut adapter = MdbookAdapter::new();
    let fixtures = default_fixtures();
    let report = run_conformance(&mut adapter, &fixtures);
    let failures = report.failures();
    for (name, _) in &failures {
        assert!(
            *name == "invoke_plugin_identity",
            "unexpected conformance failure '{name}' (failures: {failures:?})"
        );
    }
    assert_eq!(report.adapter_id, "mdbook");
}

// ── Trait-shape gates ───────────────────────────────────────────────────

#[test]
fn mdbook_adapter_is_box_dyn_eligible() {
    let adapter: Box<dyn EcosystemAdapter> = Box::new(MdbookAdapter::new());
    assert_eq!(adapter.id(), "mdbook");
    assert_eq!(adapter.ast_type(), AstType::ZetlExt);
    assert_eq!(adapter.supported_stages(), &[Stage::PreParse]);
}

#[test]
fn mdbook_adapter_ctor_builds_live_adapter() {
    let adapter: Box<dyn EcosystemAdapter> = mdbook_adapter_ctor();
    assert_eq!(adapter.id(), "mdbook");
    assert_eq!(adapter.supported_stages(), &[Stage::PreParse]);
}

// ── TEST-3304: real-run round-trip via identity shim ────────────────────

/// Identity preprocessor: passes the `supports html` probe (exit 0) and,
/// on the real run, reads the full `[ctx, book]` JSON from stdin and
/// re-emits the book element untouched. Round-tripping the envelope
/// through a cat-style shim verifies the adapter's envelope construction
/// and response parsing end-to-end.
#[test]
fn test_3304_identity_preprocessor_round_trips_pre_parse_markdown() {
    let tmp = tempfile::tempdir().unwrap();
    let shim = tmp.path().join("mdbook-identity");

    // A preprocessor that:
    //   - `supports <renderer>` → exit 0
    //   - (no argv) → read JSON envelope from stdin, write Book element
    //                 (index 1 of the input array) to stdout
    // Using `python3 -c` for the JSON slice so we don't depend on jq.
    write_shim(
        &shim,
        concat!(
            "if [ \"$1\" = \"supports\" ]; then exit 0; fi\n",
            "python3 -c 'import json,sys; env=json.load(sys.stdin); sys.stdout.write(json.dumps(env[1]))'\n",
        ),
    );

    // Python3 is required for the envelope slice. If it's missing,
    // running the pre-parse hook won't produce JSON on stdout; skip.
    if std::process::Command::new("python3")
        .arg("--version")
        .output()
        .map(|o| !o.status.success())
        .unwrap_or(true)
    {
        eprintln!("skipping: python3 not on PATH");
        return;
    }

    let mut adapter = MdbookAdapter::new();
    let ctx_build = build_ctx();
    let ctx = HookContext::new(Stage::PreParse, "identity-mdbook", &ctx_build);
    let manifest = PluginManifest {
        extension_id: "identity-mdbook".into(),
        executable: shim,
        args: vec![],
        options: Value::Null,
        timeout: Duration::from_secs(10),
    };
    let body = "# Heading\n\nbody text";
    let input = StageInput::PreParse {
        markdown: body.into(),
    };
    let response = adapter.invoke_plugin(&manifest, input, &ctx);
    match response {
        PluginResponse::Success { output, .. } => match output {
            StageOutput::PreParse { markdown } => {
                assert_eq!(
                    markdown, body,
                    "identity preprocessor should round-trip the chapter.content verbatim"
                );
            }
            other => panic!("expected PreParse output, got {other:?}"),
        },
        PluginResponse::Error { reason, detail, .. } => {
            panic!("identity round-trip failed: {reason}: {detail}");
        }
    }
}

/// CON-3304 `supports html` probe: a preprocessor that exits 1 for
/// `supports <renderer>` is flagged as `contract_violation` and the
/// real run is never spawned. The shim increments a counter on each
/// invocation so the test can assert the real run never ran.
#[test]
fn test_3304_supports_html_gates_invocation() {
    let tmp = tempfile::tempdir().unwrap();
    let shim = tmp.path().join("mdbook-rejects-html");
    let counter = tmp.path().join("invocations.txt");

    write_shim(
        &shim,
        &format!(
            concat!(
                "# record every call, then branch on argv\n",
                "echo \"call $1\" >> {counter}\n",
                "if [ \"$1\" = \"supports\" ]; then exit 1; fi\n",
                "cat\n",
            ),
            counter = counter.display(),
        ),
    );

    let mut adapter = MdbookAdapter::new();
    let ctx_build = build_ctx();
    let ctx = HookContext::new(Stage::PreParse, "reject-html", &ctx_build);
    let manifest = PluginManifest {
        extension_id: "reject-html".into(),
        executable: shim,
        args: vec![],
        options: Value::Null,
        timeout: Duration::from_secs(5),
    };
    let input = StageInput::PreParse {
        markdown: "body".into(),
    };
    let response = adapter.invoke_plugin(&manifest, input, &ctx);
    match response {
        PluginResponse::Error { reason, detail, .. } => {
            assert_eq!(reason, "contract_violation");
            assert!(
                detail.contains("supports html") || detail.contains("html renderer"),
                "expected supports-html-related detail, got: {detail}"
            );
        }
        PluginResponse::Success { .. } => {
            panic!("preprocessor that rejects html should not run the real invocation");
        }
    }

    // Probe ran once; real run did not.
    let calls = std::fs::read_to_string(&counter).unwrap_or_default();
    let probe_calls = calls.matches("call supports").count();
    let real_calls = calls
        .lines()
        .filter(|l| !l.starts_with("call supports"))
        .count();
    assert_eq!(
        probe_calls, 1,
        "probe should fire exactly once (got {probe_calls})"
    );
    assert_eq!(
        real_calls, 0,
        "real run must not fire when probe rejects html"
    );
}

/// A preprocessor that exits non-zero on the real run surfaces as a
/// `non_zero_exit` failure, with captured stderr.
#[test]
fn test_3304_non_zero_exit_on_real_run_is_classified() {
    let tmp = tempfile::tempdir().unwrap();
    let shim = tmp.path().join("mdbook-fails");

    write_shim(
        &shim,
        concat!(
            "if [ \"$1\" = \"supports\" ]; then exit 0; fi\n",
            "echo 'intentional failure' 1>&2\n",
            "exit 2\n",
        ),
    );

    let mut adapter = MdbookAdapter::new();
    let ctx_build = build_ctx();
    let ctx = HookContext::new(Stage::PreParse, "fails-on-real-run", &ctx_build);
    let manifest = PluginManifest {
        extension_id: "fails-on-real-run".into(),
        executable: shim,
        args: vec![],
        options: Value::Null,
        timeout: Duration::from_secs(5),
    };
    let input = StageInput::PreParse {
        markdown: "body".into(),
    };
    let response = adapter.invoke_plugin(&manifest, input, &ctx);
    match response {
        PluginResponse::Error { reason, stderr, .. } => {
            assert_eq!(reason, "non_zero_exit");
            assert!(
                stderr.contains("intentional failure"),
                "expected captured stderr to include the preprocessor's message, got: {stderr}"
            );
        }
        PluginResponse::Success { .. } => {
            panic!("failing preprocessor should surface as an Error");
        }
    }
}

/// A preprocessor that writes garbage to stdout (not valid JSON)
/// surfaces as `malformed_output`.
#[test]
fn test_3304_malformed_stdout_is_classified() {
    let tmp = tempfile::tempdir().unwrap();
    let shim = tmp.path().join("mdbook-garbage");

    write_shim(
        &shim,
        concat!(
            "if [ \"$1\" = \"supports\" ]; then exit 0; fi\n",
            "printf 'this is not json\\n'\n",
        ),
    );

    let mut adapter = MdbookAdapter::new();
    let ctx_build = build_ctx();
    let ctx = HookContext::new(Stage::PreParse, "garbage-stdout", &ctx_build);
    let manifest = PluginManifest {
        extension_id: "garbage-stdout".into(),
        executable: shim,
        args: vec![],
        options: Value::Null,
        timeout: Duration::from_secs(5),
    };
    let input = StageInput::PreParse {
        markdown: "body".into(),
    };
    let response = adapter.invoke_plugin(&manifest, input, &ctx);
    match response {
        PluginResponse::Error { reason, detail, .. } => {
            assert_eq!(reason, "malformed_output");
            assert!(
                detail.contains("non-JSON") || detail.contains("JSON"),
                "expected JSON-parse detail, got: {detail}"
            );
        }
        PluginResponse::Success { .. } => {
            panic!("non-JSON stdout should surface as malformed_output");
        }
    }
}

/// A preprocessor that returns a JSON Book whose first section isn't a
/// Chapter (e.g. a PartTitle) surfaces as `malformed_output` with a
/// "wrong shape" detail — the adapter cannot extract `Chapter.content`.
#[test]
fn test_3304_unexpected_envelope_shape_is_classified() {
    let tmp = tempfile::tempdir().unwrap();
    let shim = tmp.path().join("mdbook-wrong-shape");

    write_shim(
        &shim,
        concat!(
            "if [ \"$1\" = \"supports\" ]; then exit 0; fi\n",
            "printf '%s' '{\"sections\": [{\"PartTitle\": \"Part One\"}]}'\n",
        ),
    );

    let mut adapter = MdbookAdapter::new();
    let ctx_build = build_ctx();
    let ctx = HookContext::new(Stage::PreParse, "wrong-shape", &ctx_build);
    let manifest = PluginManifest {
        extension_id: "wrong-shape".into(),
        executable: shim,
        args: vec![],
        options: Value::Null,
        timeout: Duration::from_secs(5),
    };
    let input = StageInput::PreParse {
        markdown: "body".into(),
    };
    let response = adapter.invoke_plugin(&manifest, input, &ctx);
    match response {
        PluginResponse::Error { reason, detail, .. } => {
            assert_eq!(reason, "malformed_output");
            assert!(
                detail.contains("invalid shape") || detail.contains("Chapter"),
                "expected envelope-shape detail, got: {detail}"
            );
        }
        PluginResponse::Success { .. } => {
            panic!("wrong-shape envelope should surface as malformed_output");
        }
    }
}

// ── REQ-3309: envelope shape visible to the preprocessor ─────────────────

/// The preprocessor receives the synthetic envelope on stdin. Run a
/// shim that echoes its stdin to stderr so the test can assert the
/// CON-3304 `[Context, Book]` shape arrived exactly as constructed by
/// the adapter.
#[test]
fn test_3309_envelope_carries_required_fields_on_stdin() {
    let tmp = tempfile::tempdir().unwrap();
    let shim = tmp.path().join("mdbook-stdin-spy");

    // Echo stdin to stderr for the test to read, then emit a valid
    // identity Book so the adapter returns Success.
    write_shim(
        &shim,
        concat!(
            "if [ \"$1\" = \"supports\" ]; then exit 0; fi\n",
            "tee /dev/stderr | python3 -c 'import json,sys; env=json.load(sys.stdin); sys.stdout.write(json.dumps(env[1]))'\n",
        ),
    );

    // Python3 is required for the JSON slice.
    if std::process::Command::new("python3")
        .arg("--version")
        .output()
        .map(|o| !o.status.success())
        .unwrap_or(true)
    {
        eprintln!("skipping: python3 not on PATH");
        return;
    }

    let mut adapter = MdbookAdapter::new();
    let ctx_build = build_ctx();
    let ctx = HookContext::new(Stage::PreParse, "stdin-spy", &ctx_build);
    let manifest = PluginManifest {
        extension_id: "stdin-spy".into(),
        executable: shim,
        args: vec![],
        options: Value::Null,
        timeout: Duration::from_secs(10),
    };
    let body = "Hello\n\nworld.";
    let input = StageInput::PreParse {
        markdown: body.into(),
    };
    let response = adapter.invoke_plugin(&manifest, input, &ctx);
    let stderr = match response {
        PluginResponse::Success { stderr, .. } => stderr,
        PluginResponse::Error { reason, detail, .. } => {
            panic!("stdin-spy invocation failed: {reason}: {detail}");
        }
    };

    // The shim's stderr is a copy of the JSON envelope the preprocessor
    // received. Parse it and assert every REQ-3309 field.
    let seen: Value = serde_json::from_str(stderr.trim()).unwrap_or_else(|e| {
        panic!("couldn't parse recorded envelope stderr: {e}; stderr was: {stderr:?}")
    });

    // Top-level shape: 2-element array.
    let arr = seen.as_array().expect("envelope must be a JSON array");
    assert_eq!(arr.len(), 2, "envelope is [Context, Book]");

    // Context fields.
    assert_eq!(arr[0]["root"], "/tmp/mdbook-integration");
    assert_eq!(arr[0]["renderer"], "html");
    assert_eq!(arr[0]["mdbook_version"], MDBOOK_PROTOCOL_VERSION);
    assert_eq!(arr[0]["config"]["book"]["title"], "Chapter One");
    assert_eq!(arr[0]["config"]["book"]["src"], ".");

    // Book fields.
    let sections = arr[1]["sections"].as_array().unwrap();
    assert_eq!(sections.len(), 1);
    let ch = &sections[0]["Chapter"];
    assert_eq!(ch["name"], "Chapter One");
    assert_eq!(ch["content"], body);
    assert_eq!(ch["path"], "chapter-one.md");
    assert_eq!(ch["source_path"], "chapter-one.md");
    assert!(ch["number"].is_null());
    assert!(arr[1]["__non_exhaustive"].is_null());
}

// ── Vault scope diagnostic (REQ-3304) ───────────────────────────────────

/// `scope = "vault"` is accepted but surfaces a warning diagnostic so
/// users see that v1 still invokes per-page. (Whole-vault batching is a
/// future phase; the adapter preserves the scope field so the pipeline
/// can key off it later without a manifest change.)
#[test]
fn test_3304_vault_scope_emits_warning_diagnostic() {
    let tmp = tempfile::tempdir().unwrap();
    let shim = tmp.path().join("mdbook-vault-noop");

    // Identity: passes supports, echoes the Book untouched.
    write_shim(
        &shim,
        concat!(
            "if [ \"$1\" = \"supports\" ]; then exit 0; fi\n",
            "python3 -c 'import json,sys; env=json.load(sys.stdin); sys.stdout.write(json.dumps(env[1]))'\n",
        ),
    );

    if std::process::Command::new("python3")
        .arg("--version")
        .output()
        .map(|o| !o.status.success())
        .unwrap_or(true)
    {
        eprintln!("skipping: python3 not on PATH");
        return;
    }

    let mut adapter = MdbookAdapter::new();
    let ctx_build = build_ctx();
    let ctx = HookContext::new(Stage::PreParse, "vault-scope", &ctx_build);
    let manifest = PluginManifest {
        extension_id: "vault-scope".into(),
        executable: shim,
        args: vec![],
        options: json!({"scope": "vault"}),
        timeout: Duration::from_secs(10),
    };
    let input = StageInput::PreParse {
        markdown: "body".into(),
    };
    let response = adapter.invoke_plugin(&manifest, input, &ctx);
    match response {
        PluginResponse::Success { diagnostics, .. } => {
            assert!(
                diagnostics
                    .iter()
                    .any(|d| d.severity == "warning" && d.message.contains("scope=\"vault\"")),
                "expected vault-scope warning diagnostic, got: {diagnostics:?}"
            );
        }
        PluginResponse::Error { reason, detail, .. } => {
            panic!("vault-scope identity run failed: {reason}: {detail}");
        }
    }
}

// ── Pure envelope construction (REQ-3309) ───────────────────────────────

/// The envelope builder is a pure function — assert the CON-3304 top-level
/// array shape and REQ-3309 required fields without spawning anything.
#[test]
fn envelope_builder_matches_req_3309_shape() {
    let opts = MdbookOptions::default();
    let env = build_envelope_for_page(
        Path::new("/vault/root"),
        "My Vault",
        &opts,
        "Page One",
        "page-one",
        "raw markdown body",
    );

    let arr = env.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["root"], "/vault/root");
    assert_eq!(arr[0]["config"]["book"]["title"], "My Vault");
    assert_eq!(arr[0]["renderer"], "html");
    assert_eq!(arr[0]["mdbook_version"], MDBOOK_PROTOCOL_VERSION);

    let ch = &arr[1]["sections"][0]["Chapter"];
    assert_eq!(ch["name"], "Page One");
    assert_eq!(ch["content"], "raw markdown body");
    assert_eq!(ch["path"], "page-one.md");
    assert_eq!(ch["source_path"], "page-one.md");

    // Round-trip: extract_chapter_content on the envelope's Book element
    // returns the exact input body.
    let book = env[1].clone();
    assert_eq!(extract_chapter_content(&book).unwrap(), "raw markdown body");
}

/// MdbookScope wire form is stable ("page" / "vault"). Manifests depend
/// on these strings staying stable, so a serde rename would be breaking.
#[test]
fn scope_wire_form_is_lowercase() {
    assert_eq!(MdbookScope::Page.as_str(), "page");
    assert_eq!(MdbookScope::Vault.as_str(), "vault");
}

// ── REQ-3309 / CON-3309: schema validation + round-trip fidelity ─────────

/// Load the canonical envelope JSON Schema. The path is pinned so the
/// schema file can't be silently moved or renamed.
fn envelope_schema_validator() -> jsonschema::Validator {
    let bytes = std::fs::read(ENVELOPE_SCHEMA_PATH)
        .unwrap_or_else(|e| panic!("cannot read {ENVELOPE_SCHEMA_PATH}: {e}"));
    let schema: Value = serde_json::from_slice(&bytes).expect("envelope schema is valid JSON");
    jsonschema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .build(&schema)
        .expect("envelope schema compiles")
}

/// REQ-3309 / CON-3309: the envelope zetl constructs MUST validate
/// against `tools/zetl-mdbook-envelope-schema-v1.json`. Walks the
/// default conformance corpus so every canonical page content shape
/// (empty, plain paragraph, paragraph with wikilink) is covered.
#[test]
fn test_3309_every_fixture_page_envelope_passes_schema_validation() {
    let validator = envelope_schema_validator();
    let opts = MdbookOptions::default();

    for fx in default_fixtures() {
        let env = build_envelope_for_page(
            Path::new("/vault/root"),
            "My Vault",
            &opts,
            &fx.name,
            &fx.name,
            &fx.sample_markdown,
        );

        if let Err(err) = validator.validate(&env) {
            let pretty = serde_json::to_string_pretty(&env).unwrap_or_default();
            panic!(
                "fixture {:?} envelope failed schema validation: {err}\nenvelope:\n{pretty}",
                fx.name
            );
        }

        // Hand-rolled validator must agree with the JSON Schema — drift
        // between the two is a bug in either direction.
        validate_envelope(&env).unwrap_or_else(|e| {
            panic!("fixture {:?} hand-rolled validator rejected: {e}", fx.name)
        });
    }
}

/// Envelope round-trip fidelity (REQ-3309 acceptance): constructing
/// the envelope from a body, slicing out the Book, and extracting
/// `Chapter.content` MUST return the body verbatim. Covers the
/// preprocessor-identity case — every preprocessor that doesn't rewrite
/// content preserves it exactly.
#[test]
fn test_3309_envelope_content_round_trip_is_byte_identical() {
    let opts = MdbookOptions::default();
    let corpus = [
        "",
        "plain ascii body\n",
        "# Heading\n\nwith a [[Wikilink]] and [external](https://example.com)\n",
        "```\nfenced code — with em dash\n```\n",
        // Multi-byte + control-adjacent content; UTF-8 must survive.
        "α β γ\n\r\ntab\there\n",
        // A full CON-3309 payload exemplifying the SPEC snippet shape.
        "---\ntitle: Example\n---\n\nBody with $math$ and `code`.\n",
    ];

    for body in corpus {
        let env = build_envelope_for_page(
            Path::new("/vault/root"),
            "Vault",
            &opts,
            "Page",
            "page",
            body,
        );
        let book = env[1].clone();
        let back = extract_chapter_content(&book).unwrap();
        assert_eq!(
            back, body,
            "envelope round-trip dropped bytes for input {body:?}"
        );
    }
}

/// Counter-test: a malformed envelope (missing required Context field)
/// fails both the JSON Schema and the hand-rolled validator. Ensures the
/// happy-path test isn't silently accepting garbage.
#[test]
fn envelope_schema_rejects_missing_required_context_field() {
    let validator = envelope_schema_validator();
    let bad = json!([
        {
            // "root" is deliberately absent.
            "config": {"book": {"title": "T", "authors": [], "src": "."}, "preprocessor": {}},
            "renderer": "html",
            "mdbook_version": "0.4.40"
        },
        {"sections": [], "__non_exhaustive": null}
    ]);
    assert!(
        validator.validate(&bad).is_err(),
        "schema should reject envelope missing context.root"
    );
    assert!(
        validate_envelope(&bad).is_err(),
        "hand-rolled validator should reject envelope missing context.root"
    );
}
