//! TEST-3312 — ecosystem-specific manifest parsing.
//!
//! Exercises [`ztl::hooks::manifest::parse_manifest`] against the
//! TEST-3312 parse-error matrix (SPEC-033 §7):
//!
//! - `package = "..."` on `ecosystem = "pandoc"` manifest (remark-only field).
//! - Missing `exec` on pandoc manifest.
//! - Unknown `ecosystem` value.
//! - Both `exec` and `lua_filter` on pandoc manifest.
//!
//! Plus the happy paths that TEST-3312 gestures at by calling out the
//! valid examples alongside the invalid ones.

use ztl::ecosystems::{
    EcosystemSpecific, MdbookManifestFields, MdbookScope, PandocManifestFields,
    RemarkManifestFields,
};
use ztl::hooks::manifest::{parse_manifest, ManifestError};
use ztl::hooks::pipeline::Stage;

/// Assert the parse fails with `ManifestError::Parse` whose message
/// contains every substring in `needles`.
#[track_caller]
fn assert_parse_error_contains(
    result: Result<impl std::fmt::Debug, ManifestError>,
    needles: &[&str],
) {
    let err = result.expect_err("expected a parse error");
    match err {
        ManifestError::Parse { message, .. } => {
            for needle in needles {
                assert!(
                    message.contains(needle),
                    "expected error message to contain {needle:?}, got: {message}"
                );
            }
        }
        other => panic!("expected ManifestError::Parse, got {other:?}"),
    }
}

// ── Valid manifests ─────────────────────────────────────────────────────────

#[test]
fn pandoc_filter_manifest_parses() {
    let text = r#"
stage = "transform"
mode = "persistent"
timeout_ms = 500
ast_type = "pandoc-ext"
ecosystem = "pandoc"
exec = "pandoc-crossref"
args = ["--csl", "apa.csl"]

[select]
include = ["posts/**/*.md"]
"#;
    let m = parse_manifest(text, None).expect("valid pandoc manifest");
    assert_eq!(m.stage, Some(Stage::Transform));
    assert_eq!(m.timeout_ms, 500);
    assert_eq!(m.select.include, vec!["posts/**/*.md".to_string()]);
    match m.extra.expect("ecosystem block present") {
        EcosystemSpecific::Pandoc(PandocManifestFields {
            exec,
            args,
            lua_filter,
        }) => {
            assert_eq!(exec.as_deref(), Some("pandoc-crossref"));
            assert_eq!(args, vec!["--csl".to_string(), "apa.csl".to_string()]);
            assert!(lua_filter.is_none());
        }
        other => panic!("expected Pandoc variant, got {other:?}"),
    }
}

#[test]
fn pandoc_lua_filter_manifest_parses() {
    let text = r#"
ecosystem = "pandoc"
lua_filter = "filters/counter.lua"
"#;
    let m = parse_manifest(text, None).expect("valid pandoc lua manifest");
    match m.extra.expect("ecosystem block present") {
        EcosystemSpecific::Pandoc(p) => {
            assert_eq!(
                p.lua_filter.as_deref(),
                Some(std::path::Path::new("filters/counter.lua"))
            );
            assert!(p.exec.is_none());
        }
        other => panic!("expected Pandoc variant, got {other:?}"),
    }
}

#[test]
fn mdbook_manifest_parses_with_default_scope() {
    let text = r#"
stage = "pre-parse"
ecosystem = "mdbook"
exec = "mdbook-mermaid"
"#;
    let m = parse_manifest(text, None).expect("valid mdbook manifest");
    assert_eq!(m.stage, Some(Stage::PreParse));
    match m.extra.expect("ecosystem block present") {
        EcosystemSpecific::Mdbook(MdbookManifestFields { exec, scope }) => {
            assert_eq!(exec, "mdbook-mermaid");
            assert_eq!(scope, MdbookScope::Page);
        }
        other => panic!("expected Mdbook variant, got {other:?}"),
    }
}

#[test]
fn mdbook_manifest_parses_with_vault_scope() {
    let text = r#"
ecosystem = "mdbook"
exec = "mdbook-toc"
scope = "vault"
"#;
    let m = parse_manifest(text, None).expect("valid mdbook manifest");
    match m.extra.expect("ecosystem block present") {
        EcosystemSpecific::Mdbook(MdbookManifestFields { scope, .. }) => {
            assert_eq!(scope, MdbookScope::Vault);
        }
        other => panic!("expected Mdbook variant, got {other:?}"),
    }
}

#[test]
fn remark_manifest_parses_with_options() {
    let text = r#"
ecosystem = "remark"
package = "remark-gfm"
version = ">=3.0 <4"

[options]
singleTilde = false
firstLineBlank = true
"#;
    let m = parse_manifest(text, None).expect("valid remark manifest");
    match m.extra.expect("ecosystem block present") {
        EcosystemSpecific::Remark(RemarkManifestFields {
            package,
            version,
            options,
        }) => {
            assert_eq!(package, "remark-gfm");
            assert_eq!(version.as_deref(), Some(">=3.0 <4"));
            assert_eq!(
                options.get("singleTilde").and_then(|v| v.as_bool()),
                Some(false)
            );
            assert_eq!(
                options.get("firstLineBlank").and_then(|v| v.as_bool()),
                Some(true)
            );
        }
        other => panic!("expected Remark variant, got {other:?}"),
    }
}

#[test]
fn manifest_without_ecosystem_key_has_no_extra_block() {
    // SPEC-032 ztl-native path: no `ecosystem` key at all, no `extra`.
    let text = r#"
stage = "transform"
timeout_ms = 75

[select]
include = ["**/*.md"]
"#;
    let m = parse_manifest(text, None).expect("base-only manifest parses");
    assert!(m.extra.is_none());
    assert_eq!(m.timeout_ms, 75);
}

// ── TEST-3312 invalid cases (one assertion per spec bullet) ────────────────

#[test]
fn test_3312_package_on_pandoc_manifest_rejected() {
    // Bullet 1: `package = "..."` on `ecosystem = "pandoc"` manifest
    // (remark-only field).
    let text = r#"
ecosystem = "pandoc"
exec = "pandoc-crossref"
package = "remark-gfm"
"#;
    assert_parse_error_contains(parse_manifest(text, None), &["package"]);
}

#[test]
fn test_3312_missing_exec_on_pandoc_manifest_rejected() {
    // Bullet 2: missing `exec` on a pandoc manifest. Our reading of
    // the spec: pandoc requires at least one of exec / lua_filter —
    // declaring neither is the parse error.
    let text = r#"
ecosystem = "pandoc"
args = ["--csl", "apa.csl"]
"#;
    assert_parse_error_contains(parse_manifest(text, None), &["exec", "lua_filter"]);
}

#[test]
fn test_3312_unknown_ecosystem_value_rejected() {
    // Bullet 3: unknown `ecosystem` value. Different serde releases
    // phrase the error differently; accept any of the canonical
    // substrings.
    let result = parse_manifest(r#"ecosystem = "djot""#, None);
    let err = result.expect_err("expected parse error for unknown ecosystem");
    match err {
        ManifestError::Parse { message, .. } => {
            assert!(
                message.contains("djot")
                    || message.contains("unknown variant")
                    || message.contains("ecosystem"),
                "expected unknown-variant error, got: {message}"
            );
        }
        other => panic!("expected ManifestError::Parse, got {other:?}"),
    }
}

#[test]
fn test_3312_exec_and_lua_filter_both_rejected() {
    // Bullet 4: both `exec` and `lua_filter` on a pandoc manifest.
    let text = r#"
ecosystem = "pandoc"
exec = "pandoc-crossref"
lua_filter = "filters/counter.lua"
"#;
    assert_parse_error_contains(parse_manifest(text, None), &["exec", "lua_filter"]);
}

// ── Cross-ecosystem field leakage ──────────────────────────────────────────

#[test]
fn scope_on_pandoc_manifest_rejected() {
    // `scope` is mdBook-only.
    let text = r#"
ecosystem = "pandoc"
exec = "pandoc-crossref"
scope = "page"
"#;
    assert_parse_error_contains(parse_manifest(text, None), &["scope"]);
}

#[test]
fn exec_on_remark_manifest_rejected() {
    let text = r#"
ecosystem = "remark"
package = "remark-gfm"
exec = "pandoc-crossref"
"#;
    assert_parse_error_contains(parse_manifest(text, None), &["exec"]);
}

#[test]
fn package_on_mdbook_manifest_rejected() {
    let text = r#"
ecosystem = "mdbook"
exec = "mdbook-mermaid"
package = "remark-gfm"
"#;
    assert_parse_error_contains(parse_manifest(text, None), &["package"]);
}

#[test]
fn args_on_remark_manifest_rejected() {
    // `args` is Pandoc-only; remark plugin arguments live in `options`.
    let text = r#"
ecosystem = "remark"
package = "remark-gfm"
args = ["--verbose"]
"#;
    assert_parse_error_contains(parse_manifest(text, None), &["args"]);
}

// ── Required-field coverage ────────────────────────────────────────────────

#[test]
fn mdbook_without_exec_rejected() {
    let text = r#"ecosystem = "mdbook""#;
    assert_parse_error_contains(parse_manifest(text, None), &["exec"]);
}

#[test]
fn remark_without_package_rejected() {
    let text = r#"ecosystem = "remark""#;
    assert_parse_error_contains(parse_manifest(text, None), &["package"]);
}

// ── ztl-native explicit tag ───────────────────────────────────────────────

#[test]
fn ztl_native_explicit_tag_composes_with_base_fields() {
    let text = r#"
ecosystem = "ztl-native"
stage = "transform"
timeout_ms = 120
"#;
    let m = parse_manifest(text, None).expect("ztl-native manifest parses");
    assert_eq!(m.stage, Some(Stage::Transform));
    assert_eq!(m.timeout_ms, 120);
    assert!(matches!(m.extra, Some(EcosystemSpecific::ztlNative(_))));
}

#[test]
fn ztl_native_rejects_ecosystem_specific_fields() {
    // A manifest declaring `ecosystem = "ztl-native"` must NOT also
    // carry pandoc/mdbook/remark fields — the explicit tag means no
    // adapter, so those fields would be meaningless.
    let text = r#"
ecosystem = "ztl-native"
exec = "pandoc-crossref"
"#;
    assert_parse_error_contains(parse_manifest(text, None), &["exec"]);
}

// ── Invalid scope enum value ───────────────────────────────────────────────

#[test]
fn mdbook_invalid_scope_rejected() {
    let text = r#"
ecosystem = "mdbook"
exec = "mdbook-mermaid"
scope = "sideways"
"#;
    let err = parse_manifest(text, None).expect_err("invalid scope must error");
    match err {
        ManifestError::Parse { .. } => {}
        other => panic!("expected ManifestError::Parse, got {other:?}"),
    }
}
