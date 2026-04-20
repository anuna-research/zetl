//! Schema-validation tests for the Rust AST (SPEC-032 REQ-3202 / TEST-3202).
//!
//! Closes the contract between `src/hooks/ast/` and
//! `tools/zetl-ast-schema-v1.json`: every Rust-constructed document
//! SHALL serialise to JSON that passes the schema, and every schema-valid
//! example SHALL deserialise back into an equal Rust value.

use serde_json::{json, Value};
use zetl::hooks::ast::{
    Block, BlockQuote, Code, CodeBlock, Document, DocumentKind, Embed, Emphasis, Frontmatter,
    Heading, HtmlBlock, HtmlInline, Image, Inline, LineBreak, Link, List, ListItem, Paragraph,
    Position, SoftBreak, SplBlock, Strong, Text, ThematicBreak, Wikilink, AST_VERSION,
};

const SCHEMA_PATH: &str = "tools/zetl-ast-schema-v1.json";

fn load_schema() -> jsonschema::Validator {
    let bytes =
        std::fs::read(SCHEMA_PATH).unwrap_or_else(|e| panic!("cannot read {SCHEMA_PATH}: {e}"));
    let schema: Value = serde_json::from_slice(&bytes).expect("schema is valid JSON");
    jsonschema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .build(&schema)
        .expect("schema compiles")
}

fn assert_valid(validator: &jsonschema::Validator, value: &Value, label: &str) {
    if let Err(err) = validator.validate(value) {
        let pretty = serde_json::to_string_pretty(value).unwrap_or_default();
        panic!("{label} failed schema validation: {err}\nvalue:\n{pretty}");
    }
}

fn pos(a: u32, b: u32, c: u32, d: u32) -> Position {
    Position::new(a, b, c, d)
}

/// Representative corpus: one example per schema node type, composed into a
/// single Document so structural validation covers every variant in one
/// pass. Each construction is deliberate — a bug in any single node struct
/// surfaces as a validation failure against the JSON Schema.
fn representative_document() -> Document {
    let mut fm = Frontmatter::new();
    fm.insert("title".into(), json!("Corpus"));
    fm.insert("tags".into(), json!(["alpha", "beta"]));
    fm.insert("draft".into(), json!(false));

    Document {
        ast_version: AST_VERSION.into(),
        kind: DocumentKind::Document,
        position: pos(1, 1, 80, 1),
        frontmatter: Some(fm),
        children: vec![
            Block::Heading(Heading {
                position: pos(1, 1, 1, 20),
                level: 1,
                children: vec![
                    Inline::Text(Text {
                        position: pos(1, 3, 1, 10),
                        text: "Header ".into(),
                    }),
                    Inline::Strong(Strong {
                        position: pos(1, 10, 1, 20),
                        children: vec![Inline::Emphasis(Emphasis {
                            position: pos(1, 12, 1, 18),
                            children: vec![Inline::Text(Text {
                                position: pos(1, 13, 1, 17),
                                text: "bold-em".into(),
                            })],
                        })],
                    }),
                ],
            }),
            Block::Paragraph(Paragraph {
                position: pos(3, 1, 3, 40),
                children: vec![
                    Inline::Text(Text {
                        position: pos(3, 1, 3, 6),
                        text: "see ".into(),
                    }),
                    Inline::Link(Link {
                        position: pos(3, 5, 3, 20),
                        url: "https://example.com".into(),
                        title: Some("Home".into()),
                        children: vec![Inline::Text(Text {
                            position: pos(3, 7, 3, 13),
                            text: "home".into(),
                        })],
                    }),
                    Inline::SoftBreak(SoftBreak {
                        position: pos(3, 20, 3, 21),
                    }),
                    Inline::Image(Image {
                        position: pos(3, 22, 3, 40),
                        url: "cat.png".into(),
                        title: None,
                        alt: "cat".into(),
                        children: vec![],
                    }),
                    Inline::LineBreak(LineBreak {
                        position: pos(3, 40, 3, 41),
                    }),
                    Inline::Code(Code {
                        position: pos(3, 42, 3, 50),
                        text: "fn x()".into(),
                    }),
                    Inline::HtmlInline(HtmlInline {
                        position: pos(3, 52, 3, 56),
                        text: "<br>".into(),
                    }),
                    Inline::Wikilink(Wikilink {
                        position: pos(3, 58, 3, 78),
                        target: "Target".into(),
                        alias: Some("click".into()),
                        heading: Some("Section".into()),
                        block_id: Some("abc123".into()),
                    }),
                    Inline::Wikilink(Wikilink {
                        position: pos(3, 80, 3, 90),
                        target: "Bare".into(),
                        alias: None,
                        heading: None,
                        block_id: None,
                    }),
                ],
            }),
            Block::BlockQuote(BlockQuote {
                position: pos(5, 1, 9, 1),
                children: vec![
                    Block::Paragraph(Paragraph {
                        position: pos(5, 3, 5, 30),
                        children: vec![Inline::Text(Text {
                            position: pos(5, 3, 5, 30),
                            text: "[!note] Important".into(),
                        })],
                    }),
                    Block::List(List {
                        position: pos(7, 1, 9, 1),
                        ordered: true,
                        tight: true,
                        start: Some(2),
                        children: vec![
                            ListItem::new(
                                pos(7, 1, 7, 10),
                                vec![Block::Paragraph(Paragraph {
                                    position: pos(7, 4, 7, 10),
                                    children: vec![Inline::Text(Text {
                                        position: pos(7, 4, 7, 10),
                                        text: "one".into(),
                                    })],
                                })],
                            ),
                            ListItem::new(
                                pos(8, 1, 8, 10),
                                vec![Block::Paragraph(Paragraph {
                                    position: pos(8, 4, 8, 10),
                                    children: vec![Inline::Text(Text {
                                        position: pos(8, 4, 8, 10),
                                        text: "two".into(),
                                    })],
                                })],
                            ),
                        ],
                    }),
                    Block::List(List {
                        position: pos(9, 1, 9, 20),
                        ordered: false,
                        tight: false,
                        start: None,
                        children: vec![ListItem::new(pos(9, 1, 9, 20), vec![])],
                    }),
                ],
            }),
            Block::CodeBlock(CodeBlock {
                position: pos(11, 1, 13, 3),
                fenced: true,
                lang: Some("rust".into()),
                info: Some("rust ignore".into()),
                text: "fn main() {}\n".into(),
            }),
            Block::CodeBlock(CodeBlock {
                position: pos(15, 1, 17, 1),
                fenced: false,
                lang: None,
                info: None,
                text: "    indented\n".into(),
            }),
            Block::ThematicBreak(ThematicBreak {
                position: pos(19, 1, 19, 3),
            }),
            Block::HtmlBlock(HtmlBlock {
                position: pos(21, 1, 21, 20),
                text: "<div>raw</div>".into(),
            }),
            Block::SplBlock(SplBlock {
                position: pos(23, 1, 25, 3),
                info: Some("spl".into()),
                text: "fact :foo bar".into(),
            }),
            Block::SplBlock(SplBlock {
                position: pos(27, 1, 29, 3),
                info: None,
                text: "rule :x".into(),
            }),
            Block::Embed(Embed {
                position: pos(31, 1, 31, 18),
                target: "Projects".into(),
                heading: Some("Q2".into()),
                block_id: None,
            }),
            Block::Embed(Embed {
                position: pos(33, 1, 33, 18),
                target: "Archive".into(),
                heading: None,
                block_id: Some("xyz".into()),
            }),
            Block::Embed(Embed {
                position: pos(35, 1, 35, 15),
                target: "Bare".into(),
                heading: None,
                block_id: None,
            }),
        ],
    }
}

#[test]
fn representative_corpus_validates_against_schema() {
    let validator = load_schema();
    let doc = representative_document();
    let json = serde_json::to_value(&doc).unwrap();
    assert_valid(&validator, &json, "representative corpus");
}

#[test]
fn empty_document_validates() {
    let validator = load_schema();
    let doc = Document::empty();
    let json = serde_json::to_value(&doc).unwrap();
    assert_valid(&validator, &json, "empty document");
}

#[test]
fn canonical_example_validates_without_wikilink_at_block_level() {
    // The CON-3202 example shows a Wikilink at the root — but the schema's
    // Document.children is block-only. This test constructs the intended
    // variant (Wikilink wrapped in a Paragraph) and confirms both that the
    // shape matches the schema AND that round-tripping preserves identity.
    let validator = load_schema();
    let doc = Document {
        ast_version: "1.0".into(),
        kind: DocumentKind::Document,
        position: pos(1, 1, 42, 1),
        frontmatter: Some({
            let mut fm = Frontmatter::new();
            fm.insert("title".into(), json!("Q2 Review"));
            fm
        }),
        children: vec![Block::Paragraph(Paragraph {
            position: pos(5, 1, 5, 20),
            children: vec![Inline::Wikilink(Wikilink {
                position: pos(5, 1, 5, 20),
                target: "Projects Index".into(),
                alias: None,
                heading: None,
                block_id: None,
            })],
        })],
    };
    let json = serde_json::to_value(&doc).unwrap();
    assert_valid(&validator, &json, "canonical with wikilink wrapped");
    let back: Document = serde_json::from_value(json).unwrap();
    assert_eq!(back, doc);
}

#[test]
fn roundtrip_through_schema_preserves_document() {
    let validator = load_schema();
    let original = representative_document();

    // Rust → JSON → schema check → JSON → Rust.
    let v1 = serde_json::to_value(&original).unwrap();
    assert_valid(&validator, &v1, "first pass");
    let back: Document = serde_json::from_value(v1).unwrap();
    let v2 = serde_json::to_value(&back).unwrap();
    assert_valid(&validator, &v2, "second pass");
    assert_eq!(back, original);
}

/// Guard: schema rejects malformed JSON (e.g. wrong tag value) — this
/// verifies our validator is actually doing work rather than passing
/// everything, which is the usual failure mode of "forgot to register
/// Draft 2020-12 dialect".
#[test]
fn schema_validator_rejects_unknown_node_type() {
    let validator = load_schema();
    let bad = json!({
        "ast_version": "1.0",
        "type": "Document",
        "position": {"start_line":1,"start_col":1,"end_line":1,"end_col":1},
        "children": [
            {
                "type": "NotARealNode",
                "position": {"start_line":1,"start_col":1,"end_line":1,"end_col":1}
            }
        ]
    });
    assert!(
        validator.validate(&bad).is_err(),
        "schema should reject unknown node type"
    );
}

#[test]
fn schema_validator_rejects_bad_position() {
    let validator = load_schema();
    let bad = json!({
        "ast_version": "1.0",
        "type": "Document",
        "position": {"start_line": 0, "start_col": 1, "end_line": 1, "end_col": 1},
        "children": []
    });
    assert!(
        validator.validate(&bad).is_err(),
        "schema should reject position with start_line < 1"
    );
}

/// Fixture-corpus parse stability: every JSON we emit deserialises back
/// into the same Rust value. Guards against silently lossy fields.
#[test]
fn fixture_corpus_parse_stable() {
    let cases: Vec<Value> = vec![
        // Canonical example from CON-3202 (adapted — Wikilink moved inside
        // a Paragraph per block-only Document.children constraint).
        serde_json::to_value(representative_document()).unwrap(),
        // Empty document.
        serde_json::to_value(Document::empty()).unwrap(),
        // Document with frontmatter-only (no children).
        json!({
            "ast_version": "1.0",
            "type": "Document",
            "position": {"start_line":1,"start_col":1,"end_line":1,"end_col":1},
            "frontmatter": {"title": "t"},
            "children": []
        }),
    ];
    let validator = load_schema();
    for (i, v) in cases.iter().enumerate() {
        assert_valid(&validator, v, &format!("case {i}"));
        let d: Document = serde_json::from_value(v.clone())
            .unwrap_or_else(|e| panic!("case {i} deserialises: {e}"));
        let re = serde_json::to_value(&d).unwrap();
        assert_valid(&validator, &re, &format!("case {i} (re-serialised)"));
        let d2: Document = serde_json::from_value(re.clone()).unwrap();
        assert_eq!(d, d2, "case {i} round-trip identity");
    }
}
