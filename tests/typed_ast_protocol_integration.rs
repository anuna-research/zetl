//! Integration coverage for SPEC-032 REQ-3221 / CON-3221 — the typed
//! AST-protocol dispatch surface.
//!
//! These tests exercise the cross-module wire-up:
//!
//! - Hook manifests declare `ast_type` + `ast_version` and composition
//!   plumbs them to [`ComposedHook`] (REQ-3221).
//! - [`dispatch_transform`] routes AST bytes through the right
//!   translator based on the hook's declared ast_type.
//! - A mixed pipeline — two transform hooks at the same stage with
//!   different ast_types — keeps each hook's view in its declared
//!   format while the pipeline always holds canonical zetl-ext between
//!   hooks (REQ-3221 mixed-pipeline paragraph).
//! - Marker-strip detection surfaces the REQ-3221 warning when a
//!   foreign-ext hook drops a preserved node type.
//! - Unknown ast_type in a manifest is a parse error at load-time
//!   (REQ-3313 ecosystem-not-compiled path — covered via the
//!   `zetl_only` registry).
//!
//! Transport is stubbed via a closure: the real persistent-mode
//! protocol (SPEC-032 CON-3201) is covered by the separate
//! `persistent_protocol_integration.rs` suite. Keeping the dispatch-
//! surface tests transport-free makes them deterministic and cheap.

use serde_json::Value;

use zetl::hooks::ast::{
    Block, Document, DocumentKind, Inline, Paragraph, Position, Text, Wikilink, AST_VERSION,
};
use zetl::hooks::composition::compose_stage;
use zetl::hooks::pipeline::Stage;
use zetl::hooks::translators::{
    dispatch_transform, AstType, DispatchError, TranslatorRegistry,
};

fn pos() -> Position {
    Position::origin()
}

fn doc_with(children: Vec<Block>) -> Document {
    Document {
        ast_version: AST_VERSION.to_string(),
        kind: DocumentKind::Document,
        position: pos(),
        frontmatter: None,
        children,
    }
}

fn para(children: Vec<Inline>) -> Block {
    Block::Paragraph(Paragraph {
        position: pos(),
        children,
    })
}

fn text(s: &str) -> Inline {
    Inline::Text(Text {
        position: pos(),
        text: s.to_string(),
    })
}

fn wikilink(target: &str) -> Inline {
    Inline::Wikilink(Wikilink {
        position: pos(),
        target: target.to_string(),
        alias: None,
        heading: None,
        block_id: None,
    })
}

// ── Composition → dispatch integration ────────────────────────────────────

/// End-to-end integration: a transform hook declares
/// `ast_type = "mdast-ext"` in its sidecar manifest, composition emits
/// the right [`AstType`] + preserves list, and [`dispatch_transform`]
/// routes the AST through the mdast translator so the hook sees
/// mdast-shape JSON.
///
/// This is the acceptance test for `task-typed-ast-protocol`:
/// > Hook with ast_type=mdast executes against a translated AST;
/// > translators registered; dispatch table covered by TEST-3221.
#[test]
#[cfg(unix)]
fn hook_with_ast_type_mdast_sees_translated_ast() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    let tmp = TempDir::new().unwrap();
    let vault_root = tmp.path().join("vault");
    let tx = vault_root.join(".zetl").join("hooks").join("transform.d");
    fs::create_dir_all(&tx).unwrap();

    // Hook stub (never actually spawned — we use dispatch_transform
    // with a fake run closure) plus its sidecar manifest.
    let hook_path = tx.join("10-callouts.py");
    fs::write(&hook_path, "#!/bin/sh\ntrue\n").unwrap();
    let mut perms = fs::metadata(&hook_path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&hook_path, perms).unwrap();

    fs::write(
        tx.join("10-callouts.py.toml"),
        r#"ast_type = "mdast-ext"
ast_version = ">=1.0 <2"

[contract]
preserves = ["Wikilink"]
"#,
    )
    .unwrap();

    let pipe = compose_stage(&vault_root, None, Stage::Transform).unwrap();
    assert_eq!(pipe.hooks.len(), 1);
    let hook = &pipe.hooks[0];
    assert_eq!(hook.ast_type, AstType::MdastExt);
    assert_eq!(hook.preserves, vec!["Wikilink".to_string()]);

    let registry = TranslatorRegistry::all_v1();
    let input = doc_with(vec![para(vec![text("see "), wikilink("Target")])]);

    // The run closure simulates a mdast-filter hook: it receives the
    // mdast-shape JSON, asserts the shape, and echoes it back. This
    // proves the dispatch surface is actually translating (not just
    // passing through zetl-ext bytes).
    let received: std::cell::RefCell<Option<Value>> = Default::default();
    let outcome = dispatch_transform(
        &registry,
        hook.ast_type,
        &hook.extension_id,
        &hook.preserves,
        input.clone(),
        |foreign| {
            // The hook sees mdast, not zetl-ext — `root` not `Document`.
            assert_eq!(foreign["type"], "root");
            assert_eq!(foreign["children"][0]["type"], "paragraph");
            assert_eq!(foreign["children"][0]["children"][1]["type"], "wikiLink");
            *received.borrow_mut() = Some(foreign.clone());
            Ok(foreign)
        },
    )
    .unwrap();

    // And the pipeline retrieves canonical zetl-ext bytes back out.
    assert_eq!(outcome.output, input);
    assert!(outcome.warnings.is_empty());
    assert!(received.borrow().is_some(), "run closure was not invoked");
}

// ── Mixed-pipeline round-trip (REQ-3221) ─────────────────────────────────

/// REQ-3221: two hooks at the same stage may declare different
/// `ast_type`s. Each hook sees its own declared shape; between hooks
/// the pipeline holds canonical zetl-ext.
///
/// Simulates a transform-stage pipeline with a `mdast-ext` hook
/// followed by a `pandoc-ext` hook. The fake hook closures assert the
/// node-tag of the top-level block in the payload they receive —
/// `paragraph` (mdast) and `Para` (pandoc). If the dispatch surface
/// leaked one hook's format into the other, one assertion would fail.
#[test]
fn mixed_pipeline_each_hook_sees_declared_format() {
    let registry = TranslatorRegistry::all_v1();
    let mut doc = doc_with(vec![para(vec![text("hello")])]);

    // Hook 1 (mdast).
    let outcome1 = dispatch_transform(
        &registry,
        AstType::MdastExt,
        "mdast-hook",
        &[],
        doc.clone(),
        |foreign| {
            assert_eq!(foreign["type"], "root", "mdast hook saw non-mdast shape");
            assert_eq!(foreign["children"][0]["type"], "paragraph");
            Ok(foreign)
        },
    )
    .unwrap();
    doc = outcome1.output;

    // Hook 2 (pandoc).
    let outcome2 = dispatch_transform(
        &registry,
        AstType::PandocExt,
        "pandoc-hook",
        &[],
        doc.clone(),
        |foreign| {
            assert!(
                foreign.get("pandoc-api-version").is_some(),
                "pandoc hook saw non-pandoc shape"
            );
            assert_eq!(foreign["blocks"][0]["t"], "Para");
            Ok(foreign)
        },
    )
    .unwrap();

    // End-to-end identity: the pipeline's canonical form is zetl-ext,
    // and both hooks were identity operators, so the original document
    // round-trips through both translators unchanged.
    let expected = doc_with(vec![para(vec![text("hello")])]);
    assert_eq!(outcome2.output, expected);
    assert!(outcome1.warnings.is_empty());
    assert!(outcome2.warnings.is_empty());
}

// ── Marker-strip detection (REQ-3221 / CON-3221) ─────────────────────────

/// A foreign-ext hook that silently strips wikilinks produces a
/// [`StripWarning`]. CON-3221 failure table — pipeline CONTINUES; the
/// warning is advisory (not a failure).
#[test]
fn dropped_wikilinks_surface_as_warnings_not_errors() {
    let registry = TranslatorRegistry::all_v1();
    let input = doc_with(vec![para(vec![
        text("before "),
        wikilink("A"),
        text(" and "),
        wikilink("B"),
    ])]);

    // Pandoc-filter that returns a document with no Span (zetl-wikilink)
    // nodes. Post-translation this lands as a paragraph with just
    // text and the translator's zetl-wikilink inlines gone.
    let run = |_foreign: Value| -> Result<Value, DispatchError> {
        Ok(serde_json::json!({
            "pandoc-api-version": [1, 23, 1, 0],
            "meta": {"zetl-ast-version": {"t":"MetaInlines","c":[{"t":"Str","c":"1.0"}]}},
            "blocks": [
                {
                    "t": "Para",
                    "c": [
                        {"t": "Str", "c": "before  and "}
                    ],
                }
            ],
        }))
    };

    let preserves = vec!["Wikilink".to_string(), "Embed".to_string()];
    let outcome = dispatch_transform(
        &registry,
        AstType::PandocExt,
        "lossy-filter",
        &preserves,
        input,
        run,
    )
    .unwrap();

    // Exactly one warning — the Wikilink count dropped 2 → 0.
    assert_eq!(outcome.warnings.len(), 1);
    let w = &outcome.warnings[0];
    assert_eq!(w.hook_id, "lossy-filter");
    assert_eq!(w.node_type, "Wikilink");
    assert_eq!(w.before, 2);
    assert_eq!(w.after, 0);
    assert_eq!(w.log_line(), "lossy-filter dropped 2 Wikilink");
}

// ── REQ-3313 ecosystem-not-compiled path ─────────────────────────────────

/// A v1 binary registering only `zetl-ext` rejects a hook manifest
/// declaring a foreign `ast_type` at load-time (REQ-3313). The typed
/// error carries the actionable hint pointing at
/// `zetl ecosystem check`.
#[test]
fn zetl_only_binary_rejects_foreign_ast_type_with_hint() {
    let registry = TranslatorRegistry::zetl_only();
    let err = dispatch_transform(
        &registry,
        AstType::PandocExt,
        "needs-pandoc",
        &[],
        doc_with(vec![]),
        |v| Ok(v),
    )
    .unwrap_err();

    match err {
        DispatchError::EcosystemNotCompiled {
            ast_type,
            hook_id,
            hint,
        } => {
            assert_eq!(ast_type, AstType::PandocExt);
            assert_eq!(hook_id, "needs-pandoc");
            assert!(hint.contains("zetl ecosystem check"));
            // The hint names the registered ast_types so users know
            // which ecosystems their binary CAN speak.
            assert!(hint.contains("zetl-ext"));
        }
        other => panic!("expected EcosystemNotCompiled, got {other}"),
    }
}

// ── Translator responses that fail to parse (CON-3221 failure table) ────

/// CON-3221 row "Hook returns foreign AST that fails to translate back":
/// a malformed mdast response surfaces as a [`DispatchError::Translation`]
/// (REQ-3207 failure; the caller reverts the page fragment).
#[test]
fn malformed_foreign_response_surfaces_translation_error() {
    let registry = TranslatorRegistry::all_v1();
    let doc = doc_with(vec![para(vec![text("x")])]);

    let run = |_foreign: Value| -> Result<Value, DispatchError> {
        // mdast parser will reject the `children` shape — it expects
        // an array, not a string.
        Ok(serde_json::json!({
            "type": "root",
            "children": "oops not an array",
        }))
    };

    let err = dispatch_transform(
        &registry,
        AstType::MdastExt,
        "malformed",
        &[],
        doc,
        run,
    )
    .unwrap_err();

    match err {
        DispatchError::Translation(t) => {
            assert_eq!(t.ast_type, AstType::MdastExt);
        }
        other => panic!("expected Translation, got {other}"),
    }
}
