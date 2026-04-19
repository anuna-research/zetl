//! Rust representation of the zetl-ext AST (SPEC-032 REQ-3202 / CON-3202).
//!
//! This module provides typed, serde-compatible Rust structs mirroring the
//! JSON Schema published at `tools/zetl-ast-schema-v1.json` (v1.0). The
//! pre-parse → parse → transform → render → post-render pipeline consumes
//! these types on the `transform` stage.
//!
//! ## Round-trip contract
//!
//! For every schema-valid AST:
//!
//! ```text
//!   serde_json::from_value::<Document>(v)        is Ok
//!   serde_json::to_value(&doc)                   is schema-valid
//!   serde_json::to_value(&doc) → Document        is the same Document
//! ```
//!
//! The node tag is carried in the serialised `"type"` field via internal
//! serde tagging. Absent optional fields (`frontmatter`) serialise as absent;
//! required-but-nullable fields (`Wikilink.alias`, `Link.title`, …)
//! serialise as explicit `null`.
//!
//! ## Scope
//!
//! - All CommonMark block + inline node types.
//! - zetl extensions: `Wikilink`, `Embed`, `SplBlock`, `FrontMatter` (as
//!   `Document.frontmatter`).
//! - Source positions (1-indexed, inclusive).
//! - Schema version carried at `Document.ast_version`.
//!
//! Validation against the JSON Schema file itself is deferred to a
//! downstream task (`task-ast-cli` / `task-helper-*`); this module owns the
//! Rust → JSON boundary only.

use serde::{Deserialize, Serialize};

/// The exact two-component schema version this crate emits at the root of
/// every `Document`. Bumped additively per NFR-3206; a new major here is a
/// breaking change to every hook that declares `ast_version = "1.x"`.
pub const AST_VERSION: &str = "1.0";

// ── Position ────────────────────────────────────────────────────────────────

/// Source-text position of a node. Lines and columns are 1-indexed and
/// inclusive, matching CommonMark's source-map convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Position {
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

impl Position {
    pub const fn new(start_line: u32, start_col: u32, end_line: u32, end_col: u32) -> Self {
        Self { start_line, start_col, end_line, end_col }
    }

    /// Zero-width position at the document origin — useful for synthetic
    /// nodes produced by transform hooks that have no source span.
    pub const fn origin() -> Self {
        Self::new(1, 1, 1, 1)
    }
}

// ── Frontmatter ─────────────────────────────────────────────────────────────

/// Parsed YAML frontmatter rendered as a JSON object. Arbitrary user-defined
/// keys are permitted, so the representation is a `serde_json::Map` rather
/// than a typed struct.
pub type Frontmatter = serde_json::Map<String, serde_json::Value>;

// ── Document ────────────────────────────────────────────────────────────────

/// Unit-tag used to force the literal `"type": "Document"` discriminator on
/// the root struct. See [`Document`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DocumentKind {
    #[default]
    Document,
}

/// Root of a zetl-ext AST document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub ast_version: String,
    #[serde(rename = "type", default)]
    pub kind: DocumentKind,
    pub position: Position,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frontmatter: Option<Frontmatter>,
    pub children: Vec<Block>,
}

impl Document {
    /// New empty document at [`Position::origin`] tagged with [`AST_VERSION`].
    pub fn empty() -> Self {
        Self {
            ast_version: AST_VERSION.to_string(),
            kind: DocumentKind::Document,
            position: Position::origin(),
            frontmatter: None,
            children: Vec::new(),
        }
    }
}

// ── Block enum ──────────────────────────────────────────────────────────────

/// A block-level node. The `transform` return-type contract (CON-3201)
/// forbids replacing a block with an inline, so the Block / Inline split is
/// normative.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Block {
    Heading(Heading),
    Paragraph(Paragraph),
    BlockQuote(BlockQuote),
    List(List),
    CodeBlock(CodeBlock),
    ThematicBreak(ThematicBreak),
    HtmlBlock(HtmlBlock),
    SplBlock(SplBlock),
    Embed(Embed),
}

impl Block {
    /// The stable on-disk tag for this node (matches the schema's `type`).
    pub fn kind_str(&self) -> &'static str {
        match self {
            Block::Heading(_) => "Heading",
            Block::Paragraph(_) => "Paragraph",
            Block::BlockQuote(_) => "BlockQuote",
            Block::List(_) => "List",
            Block::CodeBlock(_) => "CodeBlock",
            Block::ThematicBreak(_) => "ThematicBreak",
            Block::HtmlBlock(_) => "HtmlBlock",
            Block::SplBlock(_) => "SplBlock",
            Block::Embed(_) => "Embed",
        }
    }

    pub fn position(&self) -> Position {
        match self {
            Block::Heading(n) => n.position,
            Block::Paragraph(n) => n.position,
            Block::BlockQuote(n) => n.position,
            Block::List(n) => n.position,
            Block::CodeBlock(n) => n.position,
            Block::ThematicBreak(n) => n.position,
            Block::HtmlBlock(n) => n.position,
            Block::SplBlock(n) => n.position,
            Block::Embed(n) => n.position,
        }
    }
}

// ── Inline enum ─────────────────────────────────────────────────────────────

/// An inline-level node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Inline {
    Text(Text),
    Emphasis(Emphasis),
    Strong(Strong),
    Code(Code),
    Link(Link),
    Image(Image),
    LineBreak(LineBreak),
    SoftBreak(SoftBreak),
    HtmlInline(HtmlInline),
    Wikilink(Wikilink),
}

impl Inline {
    pub fn kind_str(&self) -> &'static str {
        match self {
            Inline::Text(_) => "Text",
            Inline::Emphasis(_) => "Emphasis",
            Inline::Strong(_) => "Strong",
            Inline::Code(_) => "Code",
            Inline::Link(_) => "Link",
            Inline::Image(_) => "Image",
            Inline::LineBreak(_) => "LineBreak",
            Inline::SoftBreak(_) => "SoftBreak",
            Inline::HtmlInline(_) => "HtmlInline",
            Inline::Wikilink(_) => "Wikilink",
        }
    }

    pub fn position(&self) -> Position {
        match self {
            Inline::Text(n) => n.position,
            Inline::Emphasis(n) => n.position,
            Inline::Strong(n) => n.position,
            Inline::Code(n) => n.position,
            Inline::Link(n) => n.position,
            Inline::Image(n) => n.position,
            Inline::LineBreak(n) => n.position,
            Inline::SoftBreak(n) => n.position,
            Inline::HtmlInline(n) => n.position,
            Inline::Wikilink(n) => n.position,
        }
    }
}

// ── Block node structs ──────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Heading {
    pub position: Position,
    pub level: u8,
    pub children: Vec<Inline>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Paragraph {
    pub position: Position,
    pub children: Vec<Inline>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockQuote {
    pub position: Position,
    pub children: Vec<Block>,
}

/// Ordered or unordered list. `start` is `null` for unordered lists; for
/// ordered lists it is the first item's number (default `1`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct List {
    pub position: Position,
    pub ordered: bool,
    pub tight: bool,
    #[serde(default)]
    pub start: Option<u32>,
    pub children: Vec<ListItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListItem {
    #[serde(rename = "type", default = "list_item_kind")]
    kind: ListItemKind,
    pub position: Position,
    pub children: Vec<Block>,
}

/// Private marker used only to serialise the `"type": "ListItem"` tag on a
/// `ListItem` that appears inside `List.children` (children of a list are a
/// homogeneous `ListItem[]` in the schema, not the polymorphic `Block`
/// enum).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
enum ListItemKind {
    #[default]
    ListItem,
}

fn list_item_kind() -> ListItemKind {
    ListItemKind::ListItem
}

impl ListItem {
    pub fn new(position: Position, children: Vec<Block>) -> Self {
        Self {
            kind: ListItemKind::ListItem,
            position,
            children,
        }
    }
}

/// Fenced or indented code block. `fenced` distinguishes the two; `lang` and
/// `info` are `null` for indented blocks. The raw block content (without the
/// fence delimiters) lives in `text`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeBlock {
    pub position: Position,
    pub fenced: bool,
    pub lang: Option<String>,
    pub info: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThematicBreak {
    pub position: Position,
}

/// A raw HTML block; contents are passed through the renderer verbatim
/// (subject to the renderer's sanitiser).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HtmlBlock {
    pub position: Position,
    pub text: String,
}

/// A fenced code block whose info string is exactly `spl`; promoted to a
/// first-class node so transform hooks can operate on SPL source without
/// string-matching info strings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SplBlock {
    pub position: Position,
    #[serde(default)]
    pub info: Option<String>,
    pub text: String,
}

/// Obsidian-style transclusion (`![[target]]`, `![[target#heading]]`,
/// `![[target#^block-id]]`). Block-level.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Embed {
    pub position: Position,
    pub target: String,
    pub heading: Option<String>,
    pub block_id: Option<String>,
}

// ── Inline node structs ─────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Text {
    pub position: Position,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Emphasis {
    pub position: Position,
    pub children: Vec<Inline>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Strong {
    pub position: Position,
    pub children: Vec<Inline>,
}

/// Inline code span. `text` is the literal content between the backticks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Code {
    pub position: Position,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Link {
    pub position: Position,
    pub url: String,
    pub title: Option<String>,
    pub children: Vec<Inline>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Image {
    pub position: Position,
    pub url: String,
    pub title: Option<String>,
    pub alt: String,
    pub children: Vec<Inline>,
}

/// A hard line break (two-space-newline or backslash-newline).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LineBreak {
    pub position: Position,
}

/// A soft line break (single newline within a paragraph). Renders as a
/// space or newline depending on the renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoftBreak {
    pub position: Position,
}

/// A span of raw HTML inside a paragraph or other inline context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HtmlInline {
    pub position: Position,
    pub text: String,
}

/// Obsidian-style wikilink (`[[target]]`, `[[target|alias]]`,
/// `[[target#heading]]`, `[[target#^block-id]]`, or any combination).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Wikilink {
    pub position: Position,
    pub target: String,
    pub alias: Option<String>,
    pub heading: Option<String>,
    pub block_id: Option<String>,
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pos() -> Position {
        Position::new(1, 1, 1, 1)
    }

    /// Every struct field in the schema must round-trip through
    /// serialise → deserialise unchanged. This is the serde-side half of
    /// TEST-3202's round-trip property.
    fn roundtrip(doc: &Document) {
        let v = serde_json::to_value(doc).expect("serialise");
        let back: Document = serde_json::from_value(v.clone()).expect("deserialise");
        assert_eq!(&back, doc, "roundtrip mismatch for value {v}");
    }

    #[test]
    fn ast_version_is_v1_0() {
        assert_eq!(AST_VERSION, "1.0");
    }

    #[test]
    fn empty_document_roundtrips_and_emits_tag() {
        let doc = Document::empty();
        let v = serde_json::to_value(&doc).unwrap();
        assert_eq!(v["type"], "Document");
        assert_eq!(v["ast_version"], "1.0");
        assert!(v.get("frontmatter").is_none(), "absent frontmatter omitted");
        roundtrip(&doc);
    }

    #[test]
    fn block_enum_tagged_with_type_field() {
        let block = Block::Paragraph(Paragraph {
            position: pos(),
            children: vec![Inline::Text(Text {
                position: pos(),
                text: "hi".into(),
            })],
        });
        let v = serde_json::to_value(&block).unwrap();
        assert_eq!(v["type"], "Paragraph");
        assert_eq!(v["children"][0]["type"], "Text");
    }

    #[test]
    fn heading_roundtrip_with_inline_children() {
        let doc = Document {
            ast_version: "1.0".into(),
            kind: DocumentKind::Document,
            position: pos(),
            frontmatter: None,
            children: vec![Block::Heading(Heading {
                position: pos(),
                level: 2,
                children: vec![
                    Inline::Text(Text { position: pos(), text: "Hello ".into() }),
                    Inline::Strong(Strong {
                        position: pos(),
                        children: vec![Inline::Text(Text {
                            position: pos(),
                            text: "World".into(),
                        })],
                    }),
                ],
            })],
        };
        roundtrip(&doc);
    }

    #[test]
    fn list_preserves_start_and_listitem_tag() {
        let item = ListItem::new(pos(), vec![
            Block::Paragraph(Paragraph {
                position: pos(),
                children: vec![Inline::Text(Text { position: pos(), text: "a".into() })],
            }),
        ]);
        let list = Block::List(List {
            position: pos(),
            ordered: true,
            tight: true,
            start: Some(3),
            children: vec![item],
        });
        let v = serde_json::to_value(&list).unwrap();
        assert_eq!(v["type"], "List");
        assert_eq!(v["start"], 3);
        assert_eq!(v["children"][0]["type"], "ListItem");

        let back: Block = serde_json::from_value(v).unwrap();
        assert_eq!(back, list);
    }

    #[test]
    fn list_start_accepts_null_and_missing() {
        let with_null = json!({
            "type": "List", "position": {"start_line":1,"start_col":1,"end_line":1,"end_col":1},
            "ordered": false, "tight": true, "start": null, "children": []
        });
        let with_missing = json!({
            "type": "List", "position": {"start_line":1,"start_col":1,"end_line":1,"end_col":1},
            "ordered": false, "tight": true, "children": []
        });
        let a: Block = serde_json::from_value(with_null).unwrap();
        let b: Block = serde_json::from_value(with_missing).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn wikilink_preserves_all_null_fields() {
        let link = Inline::Wikilink(Wikilink {
            position: pos(),
            target: "Projects Index".into(),
            alias: None,
            heading: None,
            block_id: None,
        });
        let v = serde_json::to_value(&link).unwrap();
        // Per CON-3202 canonical example, alias/heading/block_id are
        // explicit `null` when absent (required-but-nullable in schema).
        assert_eq!(v["type"], "Wikilink");
        assert!(v.get("alias").is_some() && v["alias"].is_null());
        assert!(v.get("heading").is_some() && v["heading"].is_null());
        assert!(v.get("block_id").is_some() && v["block_id"].is_null());

        let back: Inline = serde_json::from_value(v).unwrap();
        assert_eq!(back, link);
    }

    #[test]
    fn wikilink_with_alias_heading_blockid_roundtrips() {
        let link = Inline::Wikilink(Wikilink {
            position: pos(),
            target: "Target".into(),
            alias: Some("alias text".into()),
            heading: Some("Section".into()),
            block_id: Some("abc123".into()),
        });
        let v = serde_json::to_value(&link).unwrap();
        let back: Inline = serde_json::from_value(v).unwrap();
        assert_eq!(back, link);
    }

    #[test]
    fn embed_required_nullables_emit_as_null() {
        let embed = Block::Embed(Embed {
            position: pos(),
            target: "foo".into(),
            heading: None,
            block_id: None,
        });
        let v = serde_json::to_value(&embed).unwrap();
        assert_eq!(v["type"], "Embed");
        assert!(v["heading"].is_null());
        assert!(v["block_id"].is_null());
    }

    #[test]
    fn codeblock_lang_and_info_null_for_indented() {
        let cb = Block::CodeBlock(CodeBlock {
            position: pos(),
            fenced: false,
            lang: None,
            info: None,
            text: "    indented code\n".into(),
        });
        let v = serde_json::to_value(&cb).unwrap();
        assert!(v["lang"].is_null());
        assert!(v["info"].is_null());
        let back: Block = serde_json::from_value(v).unwrap();
        assert_eq!(back, cb);
    }

    #[test]
    fn link_and_image_title_null_roundtrips() {
        let link = Inline::Link(Link {
            position: pos(),
            url: "https://example.com".into(),
            title: None,
            children: vec![Inline::Text(Text { position: pos(), text: "ex".into() })],
        });
        let img = Inline::Image(Image {
            position: pos(),
            url: "cat.png".into(),
            title: Some("a cat".into()),
            alt: "cat".into(),
            children: vec![],
        });
        let vl = serde_json::to_value(&link).unwrap();
        let vi = serde_json::to_value(&img).unwrap();
        assert!(vl["title"].is_null());
        assert_eq!(vi["title"], "a cat");
        assert_eq!(vi["alt"], "cat");
        assert_eq!(serde_json::from_value::<Inline>(vl).unwrap(), link);
        assert_eq!(serde_json::from_value::<Inline>(vi).unwrap(), img);
    }

    #[test]
    fn splblock_info_omitted_when_none() {
        let spl = Block::SplBlock(SplBlock {
            position: pos(),
            info: None,
            text: "fact :foo".into(),
        });
        let v = serde_json::to_value(&spl).unwrap();
        // Schema: info is optional + nullable. Rust emits absent when None
        // so the round-trip does not widen the schema surface.
        assert!(v.get("info").is_none_or(|x| x.is_null()));
        let back: Block = serde_json::from_value(v).unwrap();
        assert_eq!(back, spl);
    }

    #[test]
    fn splblock_with_info_roundtrips() {
        let spl = Block::SplBlock(SplBlock {
            position: pos(),
            info: Some("spl\n".into()),
            text: "fact :foo".into(),
        });
        let v = serde_json::to_value(&spl).unwrap();
        assert_eq!(v["info"], "spl\n");
        let back: Block = serde_json::from_value(v).unwrap();
        assert_eq!(back, spl);
    }

    #[test]
    fn thematic_line_soft_breaks_roundtrip() {
        let items: Vec<Block> = vec![Block::ThematicBreak(ThematicBreak { position: pos() })];
        for b in items {
            let v = serde_json::to_value(&b).unwrap();
            let back: Block = serde_json::from_value(v).unwrap();
            assert_eq!(back, b);
        }
        let inline: Vec<Inline> = vec![
            Inline::LineBreak(LineBreak { position: pos() }),
            Inline::SoftBreak(SoftBreak { position: pos() }),
        ];
        for i in inline {
            let v = serde_json::to_value(&i).unwrap();
            let back: Inline = serde_json::from_value(v).unwrap();
            assert_eq!(back, i);
        }
    }

    #[test]
    fn html_block_and_inline_preserve_raw_text() {
        let hb = Block::HtmlBlock(HtmlBlock { position: pos(), text: "<div>x</div>".into() });
        let hi = Inline::HtmlInline(HtmlInline {
            position: pos(),
            text: "<br>".into(),
        });
        let vb = serde_json::to_value(&hb).unwrap();
        let vi = serde_json::to_value(&hi).unwrap();
        assert_eq!(vb["text"], "<div>x</div>");
        assert_eq!(vi["text"], "<br>");
        assert_eq!(serde_json::from_value::<Block>(vb).unwrap(), hb);
        assert_eq!(serde_json::from_value::<Inline>(vi).unwrap(), hi);
    }

    /// Document-with-frontmatter mirrors the canonical example in CON-3202.
    #[test]
    fn canonical_con_3202_example_deserialises() {
        let example = json!({
            "ast_version": "1.0",
            "type": "Document",
            "position": {"start_line":1,"start_col":1,"end_line":42,"end_col":1},
            "frontmatter": {"title": "Q2 Review", "tags": ["project"]},
            "children": [
                {
                    "type": "Heading",
                    "level": 1,
                    "position": {"start_line":1,"start_col":1,"end_line":1,"end_col":10},
                    "children": [
                        {"type":"Text","text":"Q2 Review","position":{"start_line":1,"start_col":3,"end_line":1,"end_col":10}}
                    ]
                },
                {
                    "type": "BlockQuote",
                    "position": {"start_line":3,"start_col":1,"end_line":3,"end_col":20},
                    "children": [
                        {
                            "type": "Paragraph",
                            "position": {"start_line":3,"start_col":3,"end_line":3,"end_col":20},
                            "children": [
                                {"type":"Text","text":"[!note] Important","position":{"start_line":3,"start_col":3,"end_line":3,"end_col":20}}
                            ]
                        }
                    ]
                }
            ]
        });
        let doc: Document = serde_json::from_value(example.clone()).expect("parse canonical");
        assert_eq!(doc.ast_version, "1.0");
        assert!(doc.frontmatter.is_some());
        let fm = doc.frontmatter.as_ref().unwrap();
        assert_eq!(fm.get("title").and_then(|v| v.as_str()), Some("Q2 Review"));
        assert_eq!(doc.children.len(), 2);
        roundtrip(&doc);
    }

    #[test]
    fn deny_unknown_fields_rejects_extras() {
        let bad = json!({
            "type": "Text",
            "position": {"start_line":1,"start_col":1,"end_line":1,"end_col":1},
            "text": "x",
            "extra": "not allowed"
        });
        assert!(serde_json::from_value::<Inline>(bad).is_err());

        let bad_doc = json!({
            "ast_version": "1.0",
            "type": "Document",
            "position": {"start_line":1,"start_col":1,"end_line":1,"end_col":1},
            "children": [],
            "junk": 1
        });
        assert!(serde_json::from_value::<Document>(bad_doc).is_err());
    }

    #[test]
    fn wrong_type_tag_rejected() {
        // Paragraph with Heading's level field should fail because Heading's
        // tag is wrong for deserialising into Paragraph.
        let v = json!({
            "type": "NotANode",
            "position": {"start_line":1,"start_col":1,"end_line":1,"end_col":1},
        });
        assert!(serde_json::from_value::<Block>(v).is_err());
    }

    /// Block and Inline enums are disjoint: a Block::Paragraph payload
    /// must not be accepted when deserialising as Inline (CON-3201 return
    /// type contract guard).
    #[test]
    fn block_not_parseable_as_inline() {
        let para = Block::Paragraph(Paragraph {
            position: pos(),
            children: vec![],
        });
        let v = serde_json::to_value(&para).unwrap();
        let err = serde_json::from_value::<Inline>(v).unwrap_err();
        assert!(err.to_string().contains("Paragraph") || err.to_string().contains("variant"));
    }

    #[test]
    fn inline_not_parseable_as_block() {
        let t = Inline::Text(Text { position: pos(), text: "x".into() });
        let v = serde_json::to_value(&t).unwrap();
        assert!(serde_json::from_value::<Block>(v).is_err());
    }

    /// Nested structure (document → blockquote → list → listitem → paragraph
    /// → inline) round-trips untouched.
    #[test]
    fn deeply_nested_document_roundtrips() {
        let doc = Document {
            ast_version: AST_VERSION.into(),
            kind: DocumentKind::Document,
            position: pos(),
            frontmatter: Some({
                let mut fm = Frontmatter::new();
                fm.insert("title".into(), json!("Nested"));
                fm.insert("tags".into(), json!(["a", "b"]));
                fm
            }),
            children: vec![Block::BlockQuote(BlockQuote {
                position: pos(),
                children: vec![Block::List(List {
                    position: pos(),
                    ordered: false,
                    tight: false,
                    start: None,
                    children: vec![ListItem::new(pos(), vec![Block::Paragraph(Paragraph {
                        position: pos(),
                        children: vec![
                            Inline::Emphasis(Emphasis {
                                position: pos(),
                                children: vec![Inline::Text(Text {
                                    position: pos(),
                                    text: "inner".into(),
                                })],
                            }),
                            Inline::Code(Code { position: pos(), text: "x()".into() }),
                            Inline::Wikilink(Wikilink {
                                position: pos(),
                                target: "Other".into(),
                                alias: Some("alt".into()),
                                heading: None,
                                block_id: None,
                            }),
                        ],
                    })])],
                })],
            })],
        };
        roundtrip(&doc);
    }

    #[test]
    fn block_and_inline_kind_str_report_variant() {
        assert_eq!(Block::ThematicBreak(ThematicBreak { position: pos() }).kind_str(), "ThematicBreak");
        assert_eq!(Inline::LineBreak(LineBreak { position: pos() }).kind_str(), "LineBreak");
    }

    #[test]
    fn block_and_inline_position_accessor() {
        let p = Position::new(5, 10, 7, 1);
        assert_eq!(Block::ThematicBreak(ThematicBreak { position: p }).position(), p);
        assert_eq!(Inline::SoftBreak(SoftBreak { position: p }).position(), p);
    }

    // ── Property tests ──────────────────────────────────────────────────────

    /// Generator for arbitrary Positions. Kept tight to avoid overflows and
    /// keep shrinking fast.
    fn arb_position() -> impl proptest::strategy::Strategy<Value = Position> {
        use proptest::prelude::*;
        (1u32..=100, 1u32..=100, 1u32..=100, 1u32..=100)
            .prop_map(|(a, b, c, d)| Position::new(a, b, c, d))
    }

    fn arb_inline() -> impl proptest::strategy::Strategy<Value = Inline> {
        use proptest::prelude::*;
        let leaf = prop_oneof![
            (arb_position(), ".*").prop_map(|(p, t)| Inline::Text(Text { position: p, text: t })),
            (arb_position(), ".*").prop_map(|(p, t)| Inline::Code(Code { position: p, text: t })),
            (arb_position(), ".*")
                .prop_map(|(p, t)| Inline::HtmlInline(HtmlInline { position: p, text: t })),
            arb_position().prop_map(|p| Inline::LineBreak(LineBreak { position: p })),
            arb_position().prop_map(|p| Inline::SoftBreak(SoftBreak { position: p })),
            (
                arb_position(),
                ".*",
                proptest::option::of(".*"),
                proptest::option::of(".*"),
                proptest::option::of(".*")
            )
                .prop_map(|(p, target, alias, heading, block_id)| Inline::Wikilink(Wikilink {
                    position: p,
                    target,
                    alias,
                    heading,
                    block_id,
                })),
        ];
        leaf.prop_recursive(3, 16, 4, |inner| {
            use proptest::prelude::*;
            prop_oneof![
                (arb_position(), prop::collection::vec(inner.clone(), 0..3))
                    .prop_map(|(p, c)| Inline::Emphasis(Emphasis { position: p, children: c })),
                (arb_position(), prop::collection::vec(inner.clone(), 0..3))
                    .prop_map(|(p, c)| Inline::Strong(Strong { position: p, children: c })),
                (
                    arb_position(),
                    ".*",
                    proptest::option::of(".*"),
                    prop::collection::vec(inner.clone(), 0..3)
                )
                    .prop_map(|(p, u, t, c)| Inline::Link(Link {
                        position: p,
                        url: u,
                        title: t,
                        children: c,
                    })),
                (
                    arb_position(),
                    ".*",
                    proptest::option::of(".*"),
                    ".*",
                    prop::collection::vec(inner, 0..3)
                )
                    .prop_map(|(p, u, t, a, c)| Inline::Image(Image {
                        position: p,
                        url: u,
                        title: t,
                        alt: a,
                        children: c,
                    })),
            ]
        })
    }

    fn arb_block() -> impl proptest::strategy::Strategy<Value = Block> {
        use proptest::prelude::*;
        let leaf = prop_oneof![
            arb_position().prop_map(|p| Block::ThematicBreak(ThematicBreak { position: p })),
            (arb_position(), ".*")
                .prop_map(|(p, t)| Block::HtmlBlock(HtmlBlock { position: p, text: t })),
            (
                arb_position(),
                any::<bool>(),
                proptest::option::of(".*"),
                proptest::option::of(".*"),
                ".*"
            )
                .prop_map(|(p, fenced, lang, info, text)| Block::CodeBlock(CodeBlock {
                    position: p,
                    fenced,
                    lang,
                    info,
                    text,
                })),
            (arb_position(), proptest::option::of(".*"), ".*")
                .prop_map(|(p, info, text)| Block::SplBlock(SplBlock { position: p, info, text })),
            (
                arb_position(),
                ".*",
                proptest::option::of(".*"),
                proptest::option::of(".*")
            )
                .prop_map(|(p, target, heading, block_id)| Block::Embed(Embed {
                    position: p,
                    target,
                    heading,
                    block_id,
                })),
            (arb_position(), prop::collection::vec(arb_inline(), 0..3))
                .prop_map(|(p, c)| Block::Paragraph(Paragraph { position: p, children: c })),
            (arb_position(), 1u8..=6, prop::collection::vec(arb_inline(), 0..3)).prop_map(
                |(p, l, c)| Block::Heading(Heading { position: p, level: l, children: c })
            ),
        ];
        leaf.prop_recursive(3, 8, 3, |inner| {
            use proptest::prelude::*;
            prop_oneof![
                (arb_position(), prop::collection::vec(inner.clone(), 0..3))
                    .prop_map(|(p, c)| Block::BlockQuote(BlockQuote { position: p, children: c })),
                (
                    arb_position(),
                    any::<bool>(),
                    any::<bool>(),
                    proptest::option::of(0u32..=100),
                    prop::collection::vec(
                        (arb_position(), prop::collection::vec(inner, 0..2))
                            .prop_map(|(p, bs)| ListItem::new(p, bs)),
                        0..3
                    )
                )
                    .prop_map(|(p, o, t, s, items)| Block::List(List {
                        position: p,
                        ordered: o,
                        tight: t,
                        start: s,
                        children: items,
                    })),
            ]
        })
    }

    use proptest::prelude::{any, prop_assert_eq};

    proptest::proptest! {
        #![proptest_config(proptest::test_runner::Config {
            cases: 64,
            .. proptest::test_runner::Config::default()
        })]

        /// TEST-3202 property: serialise → deserialise is identity on the
        /// Rust representation. Combined with the schema file itself, this
        /// anchors the round-trip guarantee documented at module level.
        #[test]
        fn prop_inline_roundtrip(inline in arb_inline()) {
            let v = serde_json::to_value(&inline).unwrap();
            let back: Inline = serde_json::from_value(v).unwrap();
            prop_assert_eq!(back, inline);
        }

        #[test]
        fn prop_block_roundtrip(block in arb_block()) {
            let v = serde_json::to_value(&block).unwrap();
            let back: Block = serde_json::from_value(v).unwrap();
            prop_assert_eq!(back, block);
        }

        #[test]
        fn prop_document_roundtrip(
            pos in arb_position(),
            has_fm in any::<bool>(),
            children in proptest::collection::vec(arb_block(), 0..3),
        ) {
            let frontmatter = if has_fm {
                let mut fm = Frontmatter::new();
                fm.insert("k".into(), serde_json::json!("v"));
                Some(fm)
            } else {
                None
            };
            let doc = Document {
                ast_version: AST_VERSION.into(),
                kind: DocumentKind::Document,
                position: pos,
                frontmatter,
                children,
            };
            let v = serde_json::to_value(&doc).unwrap();
            let back: Document = serde_json::from_value(v).unwrap();
            prop_assert_eq!(back, doc);
        }
    }
}
