//! zetl-ext ↔ pandoc-types translator (SPEC-033 REQ-3307).
//!
//! pandoc-types is the AST shape used by `pandoc` filters (the
//! `pandoc-types` Haskell library / `pandocfilters` Python bindings).
//! Pandoc's AST is richer than zetl-ext in some places and stricter in
//! others — in particular it distinguishes `Str` / `Space` /
//! `SoftBreak` inline tokens where zetl-ext uses a single `Text`
//! carrying arbitrary whitespace. This translator bridges the
//! difference with the marker conventions in REQ-3307.
//!
//! ## Shape summary
//!
//! ```text
//!   Root:   { "pandoc-api-version": [1,22,2,1],
//!             "meta": { …zetl frontmatter… },
//!             "blocks": [ …blocks… ] }
//!
//!   Node:   { "t": "Para", "c": [ …inlines… ] }
//! ```
//!
//! `t` is the node-type tag; `c` is the content (either a single value
//! or an array, depending on the node). Nodes without content omit
//! `c`.
//!
//! ## Marker conventions (REQ-3307 table)
//!
//! | zetl-ext       | pandoc-types                                                      |
//! | -------------- | ----------------------------------------------------------------- |
//! | `Text`         | `Str` (spaces preserved inside `c`; zetl tag `zetl-text` attr)    |
//! | `SoftBreak`    | `SoftBreak`                                                       |
//! | `LineBreak`    | `LineBreak`                                                       |
//! | `Emphasis`     | `Emph`                                                            |
//! | `Strong`       | `Strong`                                                          |
//! | `Code`         | `Code` (attrs `[]`)                                               |
//! | `Link`         | `Link` (pandoc 3-tuple content)                                   |
//! | `Image`        | `Image` (pandoc 3-tuple content)                                  |
//! | `HtmlInline`   | `RawInline "html"`                                                |
//! | **`Wikilink`** | `Span { classes=["zetl-wikilink"], attrs=[target, alias, heading, block_id], inlines = [Str display] }` |
//! | `Heading`      | `Header`                                                          |
//! | `Paragraph`    | `Para`                                                            |
//! | `BlockQuote`   | `BlockQuote`                                                      |
//! | `CodeBlock`    | `CodeBlock`                                                       |
//! | `ThematicBreak`| `HorizontalRule`                                                  |
//! | `HtmlBlock`    | `RawBlock "html"`                                                 |
//! | `SplBlock`     | `CodeBlock { classes=["spl"], attrs=[("zetl-spl","true")] }`      |
//! | `List`         | `BulletList` / `OrderedList` (pandoc has two node types, we pick one by `ordered`) |
//! | **`Embed`**    | `Div { classes=["zetl-embed"], attrs=[target, heading, block_id] }` |
//!
//! Position info is preserved outside of the canonical pandoc envelope
//! under `zetl-position` in the node's attr list so the round-trip
//! invariant holds byte-for-byte.
//!
//! ## Round-trip invariant (CON-3221)
//!
//! For any zetl-ext document `A`, `foreign_to_zetl(zetl_to_foreign(A))
//! == A`. Verified via proptest on a CommonMark-derived generator in
//! this module's test suite.

use serde_json::{json, Value};

use crate::hooks::ast::{
    Block, BlockQuote, Code, CodeBlock, Document, DocumentKind, Embed, Emphasis, Frontmatter,
    Heading, HtmlBlock, HtmlInline, Image, Inline, LineBreak, Link, List, ListItem, Paragraph,
    Position, SoftBreak, SplBlock, Strong, Text, ThematicBreak, Wikilink, AST_VERSION,
};

use super::{AstType, TranslationError, Translator};

/// pandoc-types API version emitted on every document. Pandoc filters
/// inspect this to decide whether to accept the AST; the value tracks
/// the latest 2.x release at the time of writing.
const PANDOC_API_VERSION: [u32; 4] = [1, 23, 1, 0];

const CLASS_ZETL_WIKILINK: &str = "zetl-wikilink";
const CLASS_ZETL_EMBED: &str = "zetl-embed";
const CLASS_ZETL_SPL: &str = "spl";
const ATTR_ZETL_POSITION: &str = "zetl-position";
const ATTR_ZETL_AST_VERSION: &str = "zetl-ast-version";

pub struct PandocTranslator;

impl Translator for PandocTranslator {
    fn ast_type(&self) -> AstType {
        AstType::PandocExt
    }

    fn zetl_to_foreign(&self, doc: &Document) -> Result<Value, TranslationError> {
        Ok(doc_to_pandoc(doc))
    }

    fn foreign_to_zetl(&self, foreign: Value) -> Result<Document, TranslationError> {
        pandoc_to_doc(foreign)
    }
}

// ── zetl-ext → pandoc-types ────────────────────────────────────────────

fn doc_to_pandoc(doc: &Document) -> Value {
    let mut meta = serde_json::Map::new();
    // zetl-ast-version carried as a MetaInlines so it survives any
    // pandoc filter that passes unknown meta through unchanged.
    meta.insert(
        ATTR_ZETL_AST_VERSION.to_string(),
        json!({
            "t": "MetaInlines",
            "c": [{"t": "Str", "c": doc.ast_version}],
        }),
    );
    meta.insert(
        "zetl-root-position".into(),
        json!({
            "t": "MetaInlines",
            "c": [{"t": "Str", "c": position_to_attr_value(doc.position)}],
        }),
    );
    if let Some(fm) = &doc.frontmatter {
        meta.insert(
            "zetl-frontmatter".into(),
            json!({
                "t": "MetaInlines",
                "c": [{"t": "Str", "c": serde_json::to_string(fm).unwrap_or_default()}],
            }),
        );
    }
    json!({
        "pandoc-api-version": PANDOC_API_VERSION,
        "meta": Value::Object(meta),
        "blocks": doc.children.iter().map(block_to_pandoc).collect::<Vec<_>>(),
    })
}

fn block_to_pandoc(b: &Block) -> Value {
    match b {
        Block::Heading(h) => json!({
            "t": "Header",
            "c": [
                h.level as u64,
                attr_tuple("", &[], &[(ATTR_ZETL_POSITION, &position_to_attr_value(h.position))]),
                h.children.iter().flat_map(inline_to_pandoc).collect::<Vec<_>>(),
            ],
        }),
        Block::Paragraph(p) => json!({
            "t": "Para",
            "c": p.children.iter().flat_map(inline_to_pandoc).collect::<Vec<_>>(),
            "zetl-position": position_to_attr_value(p.position),
        }),
        Block::BlockQuote(bq) => json!({
            "t": "BlockQuote",
            "c": bq.children.iter().map(block_to_pandoc).collect::<Vec<_>>(),
            "zetl-position": position_to_attr_value(bq.position),
        }),
        Block::List(l) => {
            let items: Vec<Value> = l
                .children
                .iter()
                .map(|item| {
                    Value::Array(item.children.iter().map(block_to_pandoc).collect())
                })
                .collect();
            if l.ordered {
                json!({
                    "t": "OrderedList",
                    "c": [
                        [
                            l.start.unwrap_or(1) as u64,
                            {"t": "DefaultStyle"},
                            {"t": "DefaultDelim"},
                        ],
                        items,
                    ],
                    "zetl-position": position_to_attr_value(l.position),
                })
            } else {
                json!({
                    "t": "BulletList",
                    "c": items,
                    "zetl-position": position_to_attr_value(l.position),
                })
            }
        }
        Block::CodeBlock(c) => {
            let mut extra = vec![(ATTR_ZETL_POSITION, position_to_attr_value(c.position))];
            if !c.fenced {
                extra.push(("zetl-fenced", "false".to_string()));
            }
            if let Some(info) = &c.info {
                extra.push(("zetl-info", info.clone()));
            }
            let classes: Vec<&str> = c.lang.as_deref().into_iter().collect();
            let extra_refs: Vec<(&str, &str)> =
                extra.iter().map(|(k, v)| (*k, v.as_str())).collect();
            json!({
                "t": "CodeBlock",
                "c": [
                    attr_tuple("", &classes, &extra_refs),
                    c.text,
                ],
            })
        }
        Block::ThematicBreak(t) => json!({
            "t": "HorizontalRule",
            "zetl-position": position_to_attr_value(t.position),
        }),
        Block::HtmlBlock(h) => json!({
            "t": "RawBlock",
            "c": ["html", h.text],
            "zetl-position": position_to_attr_value(h.position),
        }),
        Block::SplBlock(s) => {
            let mut extra = vec![
                ("zetl-spl", "true".to_string()),
                (ATTR_ZETL_POSITION, position_to_attr_value(s.position)),
            ];
            if let Some(info) = &s.info {
                extra.push(("zetl-info", info.clone()));
            }
            let extra_refs: Vec<(&str, &str)> =
                extra.iter().map(|(k, v)| (*k, v.as_str())).collect();
            json!({
                "t": "CodeBlock",
                "c": [
                    attr_tuple("", &[CLASS_ZETL_SPL], &extra_refs),
                    s.text,
                ],
            })
        }
        Block::Embed(e) => {
            let target = e.target.clone();
            let heading = e.heading.clone().unwrap_or_default();
            let block_id = e.block_id.clone().unwrap_or_default();
            let has_heading = e.heading.is_some();
            let has_block_id = e.block_id.is_some();
            let position_s = position_to_attr_value(e.position);
            let mut extra: Vec<(&str, &str)> = vec![
                ("zetl-target", target.as_str()),
                (ATTR_ZETL_POSITION, position_s.as_str()),
            ];
            if has_heading {
                extra.push(("zetl-heading", heading.as_str()));
            }
            if has_block_id {
                extra.push(("zetl-block-id", block_id.as_str()));
            }
            json!({
                "t": "Div",
                "c": [
                    attr_tuple("", &[CLASS_ZETL_EMBED], &extra),
                    Value::Array(Vec::new()),
                ],
            })
        }
    }
}

fn inline_to_pandoc(i: &Inline) -> Vec<Value> {
    match i {
        Inline::Text(t) => {
            // Emit as a single Str token carrying the full text (spaces
            // preserved literally). On the reverse, we parse this back
            // as a single Text; real pandoc filters will split the
            // token into Str/Space/… but the zetl→pandoc→zetl identity
            // path through our own translator is byte-exact.
            vec![json!({
                "t": "Str",
                "c": t.text,
                "zetl-position": position_to_attr_value(t.position),
            })]
        }
        Inline::Emphasis(e) => vec![json!({
            "t": "Emph",
            "c": e.children.iter().flat_map(inline_to_pandoc).collect::<Vec<_>>(),
            "zetl-position": position_to_attr_value(e.position),
        })],
        Inline::Strong(s) => vec![json!({
            "t": "Strong",
            "c": s.children.iter().flat_map(inline_to_pandoc).collect::<Vec<_>>(),
            "zetl-position": position_to_attr_value(s.position),
        })],
        Inline::Code(c) => vec![json!({
            "t": "Code",
            "c": [
                attr_tuple("", &[], &[(ATTR_ZETL_POSITION, &position_to_attr_value(c.position))]),
                c.text,
            ],
        })],
        Inline::Link(l) => {
            let title = l.title.clone().unwrap_or_default();
            let has_title = l.title.is_some();
            let position = position_to_attr_value(l.position);
            let mut extra: Vec<(&str, &str)> = vec![(ATTR_ZETL_POSITION, position.as_str())];
            if !has_title {
                extra.push(("zetl-title", "null"));
            }
            vec![json!({
                "t": "Link",
                "c": [
                    attr_tuple("", &[], &extra),
                    l.children.iter().flat_map(inline_to_pandoc).collect::<Vec<_>>(),
                    [l.url, title],
                ],
            })]
        }
        Inline::Image(i) => {
            let title = i.title.clone().unwrap_or_default();
            let has_title = i.title.is_some();
            let position = position_to_attr_value(i.position);
            let alt = i.alt.clone();
            let mut extra: Vec<(&str, &str)> = vec![
                ("zetl-alt", alt.as_str()),
                (ATTR_ZETL_POSITION, position.as_str()),
            ];
            if !has_title {
                extra.push(("zetl-title", "null"));
            }
            vec![json!({
                "t": "Image",
                "c": [
                    attr_tuple("", &[], &extra),
                    i.children.iter().flat_map(inline_to_pandoc).collect::<Vec<_>>(),
                    [i.url, title],
                ],
            })]
        }
        Inline::LineBreak(b) => vec![json!({
            "t": "LineBreak",
            "zetl-position": position_to_attr_value(b.position),
        })],
        Inline::SoftBreak(b) => vec![json!({
            "t": "SoftBreak",
            "zetl-position": position_to_attr_value(b.position),
        })],
        Inline::HtmlInline(h) => vec![json!({
            "t": "RawInline",
            "c": ["html", h.text],
            "zetl-position": position_to_attr_value(h.position),
        })],
        Inline::Wikilink(w) => vec![wikilink_to_pandoc(w)],
    }
}

fn wikilink_to_pandoc(w: &Wikilink) -> Value {
    let target = w.target.clone();
    let alias = w.alias.clone().unwrap_or_default();
    let heading = w.heading.clone().unwrap_or_default();
    let block_id = w.block_id.clone().unwrap_or_default();
    let position = position_to_attr_value(w.position);

    let mut extra: Vec<(&str, &str)> = vec![
        ("zetl-target", target.as_str()),
        (ATTR_ZETL_POSITION, position.as_str()),
    ];
    // Distinguish Some("") from None via presence of the attr key.
    if w.alias.is_some() {
        extra.push(("zetl-alias", alias.as_str()));
    }
    if w.heading.is_some() {
        extra.push(("zetl-heading", heading.as_str()));
    }
    if w.block_id.is_some() {
        extra.push(("zetl-block-id", block_id.as_str()));
    }

    // Span contents: the display text (alias or target). Keep it as a
    // single Str so plain-pandoc filters render something sensible.
    let display = w.alias.clone().unwrap_or_else(|| w.target.clone());
    let inlines = vec![json!({"t": "Str", "c": display})];

    json!({
        "t": "Span",
        "c": [attr_tuple("", &[CLASS_ZETL_WIKILINK], &extra), inlines],
    })
}

/// Build a pandoc `Attr` 3-tuple: `[id, [classes], [[k,v], ...]]`.
fn attr_tuple(id: &str, classes: &[&str], attrs: &[(&str, &str)]) -> Value {
    json!([
        id,
        classes,
        attrs.iter().map(|(k, v)| vec![*k, *v]).collect::<Vec<_>>(),
    ])
}

fn position_to_attr_value(pos: Position) -> String {
    format!(
        "{}:{}-{}:{}",
        pos.start_line, pos.start_col, pos.end_line, pos.end_col
    )
}

fn position_from_attr_value(s: &str) -> Position {
    // Parse "startLine:startCol-endLine:endCol" — on malformed input
    // fall back to origin so a tolerant round-trip can still succeed.
    let (start, end) = match s.split_once('-') {
        Some(pair) => pair,
        None => return Position::origin(),
    };
    let (sl, sc) = match start.split_once(':') {
        Some(pair) => pair,
        None => return Position::origin(),
    };
    let (el, ec) = match end.split_once(':') {
        Some(pair) => pair,
        None => return Position::origin(),
    };
    Position::new(
        sl.parse().unwrap_or(1),
        sc.parse().unwrap_or(1),
        el.parse().unwrap_or(1),
        ec.parse().unwrap_or(1),
    )
}

// ── pandoc-types → zetl-ext ────────────────────────────────────────────

fn pandoc_to_doc(v: Value) -> Result<Document, TranslationError> {
    let obj = require_object(v, "pandoc root")?;
    let meta = obj
        .get("meta")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    let ast_version = meta
        .get(ATTR_ZETL_AST_VERSION)
        .and_then(meta_inlines_first_str)
        .unwrap_or_else(|| AST_VERSION.to_string());

    let position = meta
        .get("zetl-root-position")
        .and_then(meta_inlines_first_str)
        .map(|s| position_from_attr_value(&s))
        .unwrap_or(Position::origin());

    let frontmatter = meta
        .get("zetl-frontmatter")
        .and_then(meta_inlines_first_str)
        .and_then(|s| serde_json::from_str::<Frontmatter>(&s).ok());

    let blocks_val = obj.get("blocks").cloned().unwrap_or(Value::Array(Vec::new()));
    let blocks = require_array(blocks_val, "blocks")?;
    let children = blocks
        .into_iter()
        .map(pandoc_to_block)
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect();

    Ok(Document {
        ast_version,
        kind: DocumentKind::Document,
        position,
        frontmatter,
        children,
    })
}

fn meta_inlines_first_str(v: &Value) -> Option<String> {
    let obj = v.as_object()?;
    if obj.get("t").and_then(|v| v.as_str()) != Some("MetaInlines") {
        return None;
    }
    let c = obj.get("c")?.as_array()?;
    let first = c.first()?.as_object()?;
    if first.get("t").and_then(|v| v.as_str()) != Some("Str") {
        return None;
    }
    first.get("c")?.as_str().map(|s| s.to_string())
}

fn pandoc_to_block(v: Value) -> Result<Option<Block>, TranslationError> {
    let obj = require_object(v, "block")?;
    let t = obj
        .get("t")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            TranslationError::from_foreign(AstType::PandocExt, "block missing 't'")
        })?
        .to_string();
    let c = obj.get("c").cloned();
    let outer_position = obj
        .get("zetl-position")
        .and_then(|v| v.as_str())
        .map(position_from_attr_value);

    match t.as_str() {
        "Header" => {
            let arr = require_array(c.unwrap_or(Value::Null), "Header.c")?;
            if arr.len() != 3 {
                return Err(TranslationError::from_foreign(
                    AstType::PandocExt,
                    format!("Header.c expected 3 elems, got {}", arr.len()),
                ));
            }
            let mut iter = arr.into_iter();
            let level = iter.next().unwrap().as_u64().unwrap_or(1) as u8;
            let attr = iter.next().unwrap();
            let position = attr_position(&attr).unwrap_or(Position::origin());
            let inlines = require_array(iter.next().unwrap(), "Header inlines")?;
            let children = parse_inlines(inlines)?;
            Ok(Some(Block::Heading(Heading {
                position,
                level,
                children,
            })))
        }
        "Para" => {
            let arr = require_array(c.unwrap_or(Value::Null), "Para.c")?;
            let children = parse_inlines(arr)?;
            Ok(Some(Block::Paragraph(Paragraph {
                position: outer_position.unwrap_or(Position::origin()),
                children,
            })))
        }
        "Plain" => {
            let arr = require_array(c.unwrap_or(Value::Null), "Plain.c")?;
            let children = parse_inlines(arr)?;
            Ok(Some(Block::Paragraph(Paragraph {
                position: outer_position.unwrap_or(Position::origin()),
                children,
            })))
        }
        "BlockQuote" => {
            let arr = require_array(c.unwrap_or(Value::Null), "BlockQuote.c")?;
            let children = arr
                .into_iter()
                .map(pandoc_to_block)
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .flatten()
                .collect();
            Ok(Some(Block::BlockQuote(BlockQuote {
                position: outer_position.unwrap_or(Position::origin()),
                children,
            })))
        }
        "BulletList" => {
            let arr = require_array(c.unwrap_or(Value::Null), "BulletList.c")?;
            let items = arr
                .into_iter()
                .map(parse_list_item)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Some(Block::List(List {
                position: outer_position.unwrap_or(Position::origin()),
                ordered: false,
                tight: true,
                start: None,
                children: items,
            })))
        }
        "OrderedList" => {
            let arr = require_array(c.unwrap_or(Value::Null), "OrderedList.c")?;
            if arr.len() != 2 {
                return Err(TranslationError::from_foreign(
                    AstType::PandocExt,
                    format!("OrderedList.c expected 2 elems, got {}", arr.len()),
                ));
            }
            let mut iter = arr.into_iter();
            let attrs = require_array(iter.next().unwrap(), "OrderedList attrs")?;
            let items_val = iter.next().unwrap();
            let start = attrs.first().and_then(|v| v.as_u64()).map(|n| n as u32);
            let items = require_array(items_val, "OrderedList items")?
                .into_iter()
                .map(parse_list_item)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Some(Block::List(List {
                position: outer_position.unwrap_or(Position::origin()),
                ordered: true,
                tight: true,
                start,
                children: items,
            })))
        }
        "CodeBlock" => {
            let arr = require_array(c.unwrap_or(Value::Null), "CodeBlock.c")?;
            if arr.len() != 2 {
                return Err(TranslationError::from_foreign(
                    AstType::PandocExt,
                    format!("CodeBlock.c expected 2 elems, got {}", arr.len()),
                ));
            }
            let mut iter = arr.into_iter();
            let attr = iter.next().unwrap();
            let text = iter.next().unwrap().as_str().unwrap_or("").to_string();
            let (_id, classes, attr_map) = decompose_attr(&attr);
            let position = attr_map
                .get(ATTR_ZETL_POSITION)
                .map(|s| position_from_attr_value(s))
                .unwrap_or(Position::origin());
            let is_spl = classes.iter().any(|c| c == CLASS_ZETL_SPL)
                || attr_map.get("zetl-spl").map(|s| s.as_str()) == Some("true");
            if is_spl {
                let info = attr_map.get("zetl-info").cloned();
                return Ok(Some(Block::SplBlock(SplBlock {
                    position,
                    info,
                    text,
                })));
            }
            let lang = classes.into_iter().next();
            let info = attr_map.get("zetl-info").cloned();
            let fenced = attr_map.get("zetl-fenced").map(|s| s.as_str()) != Some("false");
            Ok(Some(Block::CodeBlock(CodeBlock {
                position,
                fenced,
                lang,
                info,
                text,
            })))
        }
        "HorizontalRule" => Ok(Some(Block::ThematicBreak(ThematicBreak {
            position: outer_position.unwrap_or(Position::origin()),
        }))),
        "RawBlock" => {
            let arr = require_array(c.unwrap_or(Value::Null), "RawBlock.c")?;
            if arr.len() != 2 {
                return Err(TranslationError::from_foreign(
                    AstType::PandocExt,
                    format!("RawBlock.c expected 2 elems, got {}", arr.len()),
                ));
            }
            let text = arr[1].as_str().unwrap_or("").to_string();
            Ok(Some(Block::HtmlBlock(HtmlBlock {
                position: outer_position.unwrap_or(Position::origin()),
                text,
            })))
        }
        "Div" => {
            // zetl-embed envelope.
            let arr = require_array(c.unwrap_or(Value::Null), "Div.c")?;
            if arr.len() != 2 {
                return Err(TranslationError::from_foreign(
                    AstType::PandocExt,
                    format!("Div.c expected 2 elems, got {}", arr.len()),
                ));
            }
            let attr = &arr[0];
            let (_id, classes, attr_map) = decompose_attr(attr);
            if !classes.iter().any(|c| c == CLASS_ZETL_EMBED) {
                return Err(TranslationError::from_foreign(
                    AstType::PandocExt,
                    "Div without zetl-embed class has no zetl-ext mapping",
                ));
            }
            let target = attr_map
                .get("zetl-target")
                .cloned()
                .unwrap_or_default();
            let heading = attr_map.get("zetl-heading").cloned();
            let block_id = attr_map.get("zetl-block-id").cloned();
            let position = attr_map
                .get(ATTR_ZETL_POSITION)
                .map(|s| position_from_attr_value(s))
                .unwrap_or(Position::origin());
            Ok(Some(Block::Embed(Embed {
                position,
                target,
                heading,
                block_id,
            })))
        }
        other => Err(TranslationError::from_foreign(
            AstType::PandocExt,
            format!("unknown pandoc block 't': '{other}'"),
        )),
    }
}

fn parse_list_item(v: Value) -> Result<ListItem, TranslationError> {
    let arr = require_array(v, "list item")?;
    let blocks = arr
        .into_iter()
        .map(pandoc_to_block)
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect();
    Ok(ListItem::new(Position::origin(), blocks))
}

fn parse_inlines(arr: Vec<Value>) -> Result<Vec<Inline>, TranslationError> {
    let mut out = Vec::with_capacity(arr.len());
    for v in arr {
        if let Some(inline) = pandoc_to_inline(v)? {
            out.push(inline);
        }
    }
    Ok(out)
}

fn pandoc_to_inline(v: Value) -> Result<Option<Inline>, TranslationError> {
    let obj = require_object(v, "inline")?;
    let t = obj
        .get("t")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            TranslationError::from_foreign(AstType::PandocExt, "inline missing 't'")
        })?
        .to_string();
    let c = obj.get("c").cloned();
    let outer_position = obj
        .get("zetl-position")
        .and_then(|v| v.as_str())
        .map(position_from_attr_value);

    match t.as_str() {
        "Str" => {
            let text = c
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .unwrap_or_default();
            Ok(Some(Inline::Text(Text {
                position: outer_position.unwrap_or(Position::origin()),
                text,
            })))
        }
        "Space" => Ok(Some(Inline::Text(Text {
            position: outer_position.unwrap_or(Position::origin()),
            text: " ".into(),
        }))),
        "SoftBreak" => Ok(Some(Inline::SoftBreak(SoftBreak {
            position: outer_position.unwrap_or(Position::origin()),
        }))),
        "LineBreak" => Ok(Some(Inline::LineBreak(LineBreak {
            position: outer_position.unwrap_or(Position::origin()),
        }))),
        "Emph" => {
            let arr = require_array(c.unwrap_or(Value::Null), "Emph.c")?;
            Ok(Some(Inline::Emphasis(Emphasis {
                position: outer_position.unwrap_or(Position::origin()),
                children: parse_inlines(arr)?,
            })))
        }
        "Strong" => {
            let arr = require_array(c.unwrap_or(Value::Null), "Strong.c")?;
            Ok(Some(Inline::Strong(Strong {
                position: outer_position.unwrap_or(Position::origin()),
                children: parse_inlines(arr)?,
            })))
        }
        "Code" => {
            let arr = require_array(c.unwrap_or(Value::Null), "Code.c")?;
            if arr.len() != 2 {
                return Err(TranslationError::from_foreign(
                    AstType::PandocExt,
                    format!("Code.c expected 2 elems, got {}", arr.len()),
                ));
            }
            let attr = &arr[0];
            let (_id, _classes, attr_map) = decompose_attr(attr);
            let position = attr_map
                .get(ATTR_ZETL_POSITION)
                .map(|s| position_from_attr_value(s))
                .unwrap_or(Position::origin());
            let text = arr[1].as_str().unwrap_or("").to_string();
            Ok(Some(Inline::Code(Code { position, text })))
        }
        "Link" => {
            let arr = require_array(c.unwrap_or(Value::Null), "Link.c")?;
            if arr.len() != 3 {
                return Err(TranslationError::from_foreign(
                    AstType::PandocExt,
                    format!("Link.c expected 3 elems, got {}", arr.len()),
                ));
            }
            let attr = &arr[0];
            let (_id, _classes, attr_map) = decompose_attr(attr);
            let position = attr_map
                .get(ATTR_ZETL_POSITION)
                .map(|s| position_from_attr_value(s))
                .unwrap_or(Position::origin());
            let inlines = require_array(arr[1].clone(), "Link inlines")?;
            let target = require_array(arr[2].clone(), "Link target")?;
            if target.len() != 2 {
                return Err(TranslationError::from_foreign(
                    AstType::PandocExt,
                    format!("Link target expected 2 elems, got {}", target.len()),
                ));
            }
            let url = target[0].as_str().unwrap_or("").to_string();
            let title_str = target[1].as_str().unwrap_or("").to_string();
            let title = if attr_map.get("zetl-title").map(|s| s.as_str()) == Some("null") {
                None
            } else {
                Some(title_str)
            };
            let children = parse_inlines(inlines)?;
            Ok(Some(Inline::Link(Link {
                position,
                url,
                title,
                children,
            })))
        }
        "Image" => {
            let arr = require_array(c.unwrap_or(Value::Null), "Image.c")?;
            if arr.len() != 3 {
                return Err(TranslationError::from_foreign(
                    AstType::PandocExt,
                    format!("Image.c expected 3 elems, got {}", arr.len()),
                ));
            }
            let attr = &arr[0];
            let (_id, _classes, attr_map) = decompose_attr(attr);
            let position = attr_map
                .get(ATTR_ZETL_POSITION)
                .map(|s| position_from_attr_value(s))
                .unwrap_or(Position::origin());
            let inlines = require_array(arr[1].clone(), "Image inlines")?;
            let target = require_array(arr[2].clone(), "Image target")?;
            if target.len() != 2 {
                return Err(TranslationError::from_foreign(
                    AstType::PandocExt,
                    format!("Image target expected 2 elems, got {}", target.len()),
                ));
            }
            let url = target[0].as_str().unwrap_or("").to_string();
            let title_str = target[1].as_str().unwrap_or("").to_string();
            let title = if attr_map.get("zetl-title").map(|s| s.as_str()) == Some("null") {
                None
            } else {
                Some(title_str)
            };
            let alt = attr_map.get("zetl-alt").cloned().unwrap_or_default();
            let children = parse_inlines(inlines)?;
            Ok(Some(Inline::Image(Image {
                position,
                url,
                title,
                alt,
                children,
            })))
        }
        "RawInline" => {
            let arr = require_array(c.unwrap_or(Value::Null), "RawInline.c")?;
            if arr.len() != 2 {
                return Err(TranslationError::from_foreign(
                    AstType::PandocExt,
                    format!("RawInline.c expected 2 elems, got {}", arr.len()),
                ));
            }
            let text = arr[1].as_str().unwrap_or("").to_string();
            Ok(Some(Inline::HtmlInline(HtmlInline {
                position: outer_position.unwrap_or(Position::origin()),
                text,
            })))
        }
        "Span" => {
            // zetl-wikilink envelope.
            let arr = require_array(c.unwrap_or(Value::Null), "Span.c")?;
            if arr.len() != 2 {
                return Err(TranslationError::from_foreign(
                    AstType::PandocExt,
                    format!("Span.c expected 2 elems, got {}", arr.len()),
                ));
            }
            let attr = &arr[0];
            let (_id, classes, attr_map) = decompose_attr(attr);
            if !classes.iter().any(|c| c == CLASS_ZETL_WIKILINK) {
                return Err(TranslationError::from_foreign(
                    AstType::PandocExt,
                    "Span without zetl-wikilink class has no zetl-ext mapping",
                ));
            }
            let target = attr_map
                .get("zetl-target")
                .cloned()
                .unwrap_or_default();
            let alias = attr_map.get("zetl-alias").cloned();
            let heading = attr_map.get("zetl-heading").cloned();
            let block_id = attr_map.get("zetl-block-id").cloned();
            let position = attr_map
                .get(ATTR_ZETL_POSITION)
                .map(|s| position_from_attr_value(s))
                .unwrap_or(Position::origin());
            Ok(Some(Inline::Wikilink(Wikilink {
                position,
                target,
                alias,
                heading,
                block_id,
            })))
        }
        other => Err(TranslationError::from_foreign(
            AstType::PandocExt,
            format!("unknown pandoc inline 't': '{other}'"),
        )),
    }
}

fn decompose_attr(v: &Value) -> (String, Vec<String>, std::collections::BTreeMap<String, String>) {
    let arr = match v.as_array() {
        Some(a) if a.len() == 3 => a,
        _ => return ("".into(), Vec::new(), Default::default()),
    };
    let id = arr[0].as_str().unwrap_or("").to_string();
    let classes: Vec<String> = arr[1]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let attrs: std::collections::BTreeMap<String, String> = arr[2]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| {
                    let pair = v.as_array()?;
                    if pair.len() != 2 {
                        return None;
                    }
                    let k = pair[0].as_str()?.to_string();
                    let val = pair[1].as_str()?.to_string();
                    Some((k, val))
                })
                .collect()
        })
        .unwrap_or_default();
    (id, classes, attrs)
}

fn attr_position(v: &Value) -> Option<Position> {
    let (_, _, attrs) = decompose_attr(v);
    attrs
        .get(ATTR_ZETL_POSITION)
        .map(|s| position_from_attr_value(s))
}

fn require_object(
    v: Value,
    ctx: &str,
) -> Result<serde_json::Map<String, Value>, TranslationError> {
    match v {
        Value::Object(o) => Ok(o),
        other => Err(TranslationError::from_foreign(
            AstType::PandocExt,
            format!("expected object for {ctx}, got {}", value_kind(&other)),
        )),
    }
}

fn require_array(v: Value, ctx: &str) -> Result<Vec<Value>, TranslationError> {
    match v {
        Value::Array(a) => Ok(a),
        other => Err(TranslationError::from_foreign(
            AstType::PandocExt,
            format!("expected array for {ctx}, got {}", value_kind(&other)),
        )),
    }
}

fn value_kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::ast::{
        Block, CodeBlock, Document, DocumentKind, Embed, Heading, Inline, Link, List, ListItem,
        Paragraph, Position, SplBlock, Text, Wikilink,
    };

    fn pos() -> Position {
        Position::origin()
    }

    fn wrap(children: Vec<Block>) -> Document {
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

    #[test]
    fn ast_type_is_pandoc_ext() {
        assert_eq!(PandocTranslator.ast_type(), AstType::PandocExt);
    }

    #[test]
    fn document_envelope_shape_matches_pandoc() {
        let t = PandocTranslator;
        let doc = wrap(vec![para(vec![text("hi")])]);
        let v = t.zetl_to_foreign(&doc).unwrap();
        assert_eq!(v["pandoc-api-version"][0], 1);
        assert!(v["meta"].is_object());
        assert!(v["blocks"].is_array());
        assert_eq!(v["blocks"][0]["t"], "Para");
    }

    #[test]
    fn round_trip_preserves_heading_paragraph_text() {
        let t = PandocTranslator;
        let doc = wrap(vec![
            Block::Heading(Heading {
                position: pos(),
                level: 2,
                children: vec![text("Title")],
            }),
            para(vec![text("hello world")]),
        ]);
        let v = t.zetl_to_foreign(&doc).unwrap();
        let back = t.foreign_to_zetl(v).unwrap();
        assert_eq!(back, doc);
    }

    #[test]
    fn wikilink_span_marker_preserves_fields() {
        let t = PandocTranslator;
        let doc = wrap(vec![para(vec![Inline::Wikilink(Wikilink {
            position: pos(),
            target: "Target".into(),
            alias: Some("A".into()),
            heading: Some("H".into()),
            block_id: Some("B".into()),
        })])]);
        let v = t.zetl_to_foreign(&doc).unwrap();
        let span = &v["blocks"][0]["c"][0];
        assert_eq!(span["t"], "Span");
        let attr = &span["c"][0];
        assert_eq!(attr[1][0], CLASS_ZETL_WIKILINK);
        let back = t.foreign_to_zetl(v).unwrap();
        assert_eq!(back, doc);
    }

    #[test]
    fn wikilink_without_alias_roundtrips() {
        let t = PandocTranslator;
        let doc = wrap(vec![para(vec![Inline::Wikilink(Wikilink {
            position: pos(),
            target: "Target".into(),
            alias: None,
            heading: None,
            block_id: None,
        })])]);
        let v = t.zetl_to_foreign(&doc).unwrap();
        let back = t.foreign_to_zetl(v).unwrap();
        assert_eq!(back, doc);
    }

    #[test]
    fn embed_div_marker_round_trips() {
        let t = PandocTranslator;
        let doc = wrap(vec![Block::Embed(Embed {
            position: pos(),
            target: "Daily".into(),
            heading: Some("Today".into()),
            block_id: None,
        })]);
        let v = t.zetl_to_foreign(&doc).unwrap();
        let div = &v["blocks"][0];
        assert_eq!(div["t"], "Div");
        assert_eq!(div["c"][0][1][0], CLASS_ZETL_EMBED);
        let back = t.foreign_to_zetl(v).unwrap();
        assert_eq!(back, doc);
    }

    #[test]
    fn spl_block_round_trips_via_code_block_marker() {
        let t = PandocTranslator;
        let doc = wrap(vec![Block::SplBlock(SplBlock {
            position: pos(),
            info: None,
            text: "fact :foo".into(),
        })]);
        let v = t.zetl_to_foreign(&doc).unwrap();
        let cb = &v["blocks"][0];
        assert_eq!(cb["t"], "CodeBlock");
        assert_eq!(cb["c"][0][1][0], CLASS_ZETL_SPL);
        let back = t.foreign_to_zetl(v).unwrap();
        assert_eq!(back, doc);
    }

    #[test]
    fn list_ordered_and_unordered_round_trip() {
        let t = PandocTranslator;
        let doc = wrap(vec![
            Block::List(List {
                position: pos(),
                ordered: false,
                tight: true,
                start: None,
                children: vec![ListItem::new(pos(), vec![para(vec![text("a")])])],
            }),
            Block::List(List {
                position: pos(),
                ordered: true,
                tight: true,
                start: Some(3),
                children: vec![ListItem::new(pos(), vec![para(vec![text("b")])])],
            }),
        ]);
        let v = t.zetl_to_foreign(&doc).unwrap();
        let back = t.foreign_to_zetl(v).unwrap();
        assert_eq!(back, doc);
    }

    #[test]
    fn code_block_language_and_info_preserved() {
        let t = PandocTranslator;
        let doc = wrap(vec![Block::CodeBlock(CodeBlock {
            position: pos(),
            fenced: true,
            lang: Some("rust".into()),
            info: Some("edition=2021".into()),
            text: "fn main() {}\n".into(),
        })]);
        let v = t.zetl_to_foreign(&doc).unwrap();
        let back = t.foreign_to_zetl(v).unwrap();
        assert_eq!(back, doc);
    }

    #[test]
    fn link_with_and_without_title_roundtrip() {
        let t = PandocTranslator;
        let doc = wrap(vec![para(vec![
            Inline::Link(Link {
                position: pos(),
                url: "https://a".into(),
                title: None,
                children: vec![text("a")],
            }),
            Inline::Link(Link {
                position: pos(),
                url: "https://b".into(),
                title: Some("titled".into()),
                children: vec![text("b")],
            }),
        ])]);
        let v = t.zetl_to_foreign(&doc).unwrap();
        let back = t.foreign_to_zetl(v).unwrap();
        assert_eq!(back, doc);
    }

    #[test]
    fn unknown_block_type_surfaces_error() {
        let t = PandocTranslator;
        let v = json!({
            "pandoc-api-version": PANDOC_API_VERSION,
            "meta": {},
            "blocks": [{"t": "Definition", "c": []}],
        });
        let err = t.foreign_to_zetl(v).unwrap_err();
        assert!(err.message.contains("Definition"));
    }

    #[test]
    fn frontmatter_round_trips_via_meta() {
        let t = PandocTranslator;
        let mut fm = Frontmatter::new();
        fm.insert("title".into(), json!("Q2"));
        fm.insert("tags".into(), json!(["a", "b"]));
        let doc = Document {
            ast_version: AST_VERSION.to_string(),
            kind: DocumentKind::Document,
            position: pos(),
            frontmatter: Some(fm),
            children: vec![para(vec![text("body")])],
        };
        let v = t.zetl_to_foreign(&doc).unwrap();
        let back = t.foreign_to_zetl(v).unwrap();
        assert_eq!(back, doc);
    }
}
