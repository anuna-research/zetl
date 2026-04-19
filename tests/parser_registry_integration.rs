//! Integration tests for the parser registry (SPEC-033 REQ-3306 /
//! CON-3306 / TEST-3306).
//!
//! The unit tests beside the module in `src/parsers/mod.rs` cover
//! individual function contracts (config parsing, resolver precedence,
//! registry lookup); this file is the public-facing gate that exercises
//! the full `zetl::parsers` surface as downstream code would — loading
//! `.zetl/config.toml` from a real tempdir, extracting frontmatter, and
//! walking the TEST-3306 matrix end-to-end.

use std::path::Path;

use serde_json::json;
use tempfile::TempDir;

use zetl::hooks::ast::Frontmatter;
use zetl::parsers::{
    select_parser_name, ParseConfig, ParseError, ParserRegistry, DEFAULT_PARSER,
};

/// Build a tempdir-backed vault with an optional `.zetl/config.toml`.
fn make_vault(config_toml: Option<&str>) -> TempDir {
    let tmp = TempDir::new().expect("tempdir");
    if let Some(body) = config_toml {
        let dir = tmp.path().join(".zetl");
        std::fs::create_dir_all(&dir).expect("mkdir .zetl");
        std::fs::write(dir.join("config.toml"), body).expect("write config.toml");
    }
    tmp
}

fn fm(value: serde_json::Value) -> Frontmatter {
    match value {
        serde_json::Value::Object(m) => m,
        _ => panic!("test helper expects an object"),
    }
}

// ── TEST-3306 matrix ─────────────────────────────────────────────────────────
//
// | Setup                                                  | Expected parser    |
// | No frontmatter, no rule, no vault default              | commonmark         |
// | [parse] default = "pandoc"                             | pandoc             |
// | [[parse.rule]] pattern="papers/**" parser="pandoc",    |                    |
// |   page in papers/                                      | pandoc             |
// | Page frontmatter parser:commonmark, vault default pandoc | commonmark       |
// | Page frontmatter parser:djot (unknown)                 | parse error + skip |

#[test]
fn test_3306_row1_no_config_no_frontmatter_defaults_to_commonmark() {
    let vault = make_vault(None);
    let cfg = ParseConfig::load_from_vault(vault.path())
        .unwrap()
        .compile()
        .unwrap();
    let name = select_parser_name(None, Path::new("index.md"), &cfg);
    assert_eq!(name, DEFAULT_PARSER);
    assert_eq!(name, "commonmark");

    let reg = ParserRegistry::with_builtins();
    assert!(reg.require(&name).is_ok());
}

#[test]
fn test_3306_row2_vault_default_pandoc_selects_pandoc() {
    let vault = make_vault(Some(
        r#"
[parse]
default = "pandoc"
"#,
    ));
    let cfg = ParseConfig::load_from_vault(vault.path())
        .unwrap()
        .compile()
        .unwrap();
    let name = select_parser_name(None, Path::new("index.md"), &cfg);
    assert_eq!(name, "pandoc");
}

#[test]
fn test_3306_row3_rule_glob_overrides_default_for_matching_page() {
    let vault = make_vault(Some(
        r#"
[parse]
default = "commonmark"

[[parse.rule]]
pattern = "papers/**"
parser = "pandoc"
"#,
    ));
    let cfg = ParseConfig::load_from_vault(vault.path())
        .unwrap()
        .compile()
        .unwrap();

    let name = select_parser_name(None, Path::new("papers/whitepaper.md"), &cfg);
    assert_eq!(name, "pandoc");

    // Pages outside the glob fall through to the vault default.
    let name = select_parser_name(None, Path::new("notes/journal.md"), &cfg);
    assert_eq!(name, "commonmark");
}

#[test]
fn test_3306_row4_frontmatter_parser_commonmark_beats_vault_default_pandoc() {
    let vault = make_vault(Some(
        r#"
[parse]
default = "pandoc"

[[parse.rule]]
pattern = "papers/**"
parser = "pandoc"
"#,
    ));
    let cfg = ParseConfig::load_from_vault(vault.path())
        .unwrap()
        .compile()
        .unwrap();

    let frontmatter = fm(json!({ "parser": "commonmark" }));
    let name = select_parser_name(
        Some(&frontmatter),
        Path::new("papers/whitepaper.md"),
        &cfg,
    );
    assert_eq!(name, "commonmark");
}

#[test]
fn test_3306_row5_unknown_frontmatter_parser_is_registry_error() {
    let vault = make_vault(None);
    let cfg = ParseConfig::load_from_vault(vault.path())
        .unwrap()
        .compile()
        .unwrap();
    let frontmatter = fm(json!({ "parser": "djot" }));
    let name = select_parser_name(Some(&frontmatter), Path::new("index.md"), &cfg);
    assert_eq!(name, "djot");

    let reg = ParserRegistry::with_builtins();
    let err = reg
        .require(&name)
        .err()
        .expect("unknown parser must error");
    match err {
        ParseError::UnknownParser { name, known } => {
            assert_eq!(name, "djot");
            // Known list surfaces registered ids so the user can triage.
            assert!(known.contains(&"commonmark".to_string()));
            assert!(known.contains(&"pandoc".to_string()));
        }
        other => panic!("expected UnknownParser, got {other:?}"),
    }
}

// ── End-to-end: selection + parse round trip ───────────────────────────────

#[test]
fn selecting_commonmark_produces_a_valid_document() {
    let vault = make_vault(None);
    let cfg = ParseConfig::load_from_vault(vault.path())
        .unwrap()
        .compile()
        .unwrap();
    let name = select_parser_name(None, Path::new("index.md"), &cfg);

    let reg = ParserRegistry::with_builtins();
    let parser = reg.require(&name).unwrap();
    let doc = parser
        .parse("---\ntitle: Hello\n---\n\n# Greetings\n\nHi there.\n")
        .unwrap();
    assert_eq!(doc.ast_version, zetl::hooks::ast::AST_VERSION);
    assert!(doc.frontmatter.is_some(), "frontmatter should be parsed");
    assert!(!doc.children.is_empty());
}

#[test]
fn selecting_pandoc_stub_surfaces_runtime_unavailable_hint() {
    // The pandoc adapter isn't wired yet, so selecting pandoc must
    // return a typed RuntimeUnavailable error pointing at `zetl
    // ecosystem check` — per REQ-3306 the registry slot exists so
    // selection resolves, but invocation surfaces the actionable
    // diagnostic until task-pandoc-adapter lands.
    let vault = make_vault(Some(
        r#"
[parse]
default = "pandoc"
"#,
    ));
    let cfg = ParseConfig::load_from_vault(vault.path())
        .unwrap()
        .compile()
        .unwrap();
    let name = select_parser_name(None, Path::new("index.md"), &cfg);
    assert_eq!(name, "pandoc");

    let reg = ParserRegistry::with_builtins();
    let parser = reg.require(&name).unwrap();
    let err = parser
        .parse("# anything")
        .err()
        .expect("pandoc stub must error");
    match err {
        ParseError::RuntimeUnavailable { parser, hint } => {
            assert_eq!(parser, "pandoc");
            assert!(hint.contains("zetl ecosystem check"));
        }
        other => panic!("expected RuntimeUnavailable, got {other:?}"),
    }
}

// ── Config loading edges ────────────────────────────────────────────────────

#[test]
fn vault_without_zetl_dir_yields_empty_config() {
    let vault = make_vault(None);
    let cfg = ParseConfig::load_from_vault(vault.path()).unwrap();
    assert!(cfg.default.is_none());
    assert!(cfg.rule.is_empty());
}

#[test]
fn config_round_trip_preserves_rule_order() {
    let vault = make_vault(Some(
        r#"
[parse]
default = "commonmark"

[[parse.rule]]
pattern = "a/**"
parser = "pandoc"

[[parse.rule]]
pattern = "b/**"
parser = "commonmark"

[[parse.rule]]
pattern = "c/**"
parser = "pandoc"
"#,
    ));
    let cfg = ParseConfig::load_from_vault(vault.path()).unwrap();
    let patterns: Vec<_> = cfg.rule.iter().map(|r| r.pattern.as_str()).collect();
    assert_eq!(patterns, vec!["a/**", "b/**", "c/**"]);
}

#[test]
fn matching_pandoc_rule_routes_through_full_pipeline() {
    // End-to-end: load config → compile → select → require → parse
    // (stub) — verifies the full surface works as a cohesive pipeline
    // with the pandoc-stub error surface as the terminal.
    let vault = make_vault(Some(
        r#"
[parse]
default = "commonmark"

[[parse.rule]]
pattern = "papers/**"
parser = "pandoc"
"#,
    ));
    let cfg = ParseConfig::load_from_vault(vault.path())
        .unwrap()
        .compile()
        .unwrap();

    let reg = ParserRegistry::with_builtins();
    let name = select_parser_name(None, Path::new("papers/foo.md"), &cfg);
    let err = reg
        .require(&name)
        .unwrap()
        .parse("# paper body")
        .err()
        .expect("pandoc stub errors until adapter lands");
    assert!(matches!(err, ParseError::RuntimeUnavailable { .. }));
}
