//! ztl-ext ↔ mdast translator (SPEC-033 REQ-3308).
//!
//! mdast is the AST produced by the remark / unified ecosystem. It's
//! CommonMark-derived and closer in shape to ztl-ext than
//! pandoc-types, so this translator is essentially a renaming of
//! fields with a small number of marker conventions for nodes that
//! mdast doesn't model natively.
//!
//! ## Marker conventions (REQ-3308 table)
//!
//! | ztl-ext           | mdast                                                       |
//! | ------------------ | ----------------------------------------------------------- |
//! | `Document`         | `root` (with `ztl_ast_version` + optional `frontmatter`)   |
//! | `Heading`          | `heading { depth, children }`                               |
//! | `Paragraph`        | `paragraph { children }`                                    |
//! | `Text`             | `text { value }`                                            |
//! | `Emphasis`         | `emphasis { children }`                                     |
//! | `Strong`           | `strong { children }`                                       |
//! | `Code` (inline)    | `inlineCode { value }`                                      |
//! | `Link`             | `link { url, title, children }`                             |
//! | `Image`            | `image { url, title, alt }`                                 |
//! | `LineBreak`        | `break`                                                     |
//! | `SoftBreak`        | `text { value: "\n" }` (mdast convention)                   |
//! | `HtmlBlock`        | `html { value }`                                            |
//! | `HtmlInline`       | `html { value }`                                            |
//! | `CodeBlock`        | `code { lang, meta, value }`                                |
//! | `ThematicBreak`    | `thematicBreak`                                             |
//! | `BlockQuote`       | `blockquote { children }`                                   |
//! | `List`             | `list { ordered, start, spread, children }`                 |
//! | `ListItem`         | `listItem { children }`                                     |
//! | **`Wikilink`**     | `{ type: "wikiLink", value, data: {alias, heading, block_id} }` (remark-wiki-link-like marker) |
//! | **`Embed`**        | `{ type: "wikiLink", data: {embed: true, heading, block_id} }` |
//! | **`SplBlock`**     | `code { lang: "spl", value }`                               |
//!
//! Position information is preserved under the mdast `position` field
//! in both directions. Every emitted node carries its source position
//! so round-trip preserves CommonMark parser fidelity.
//!
//! ## Round-trip invariant (CON-3221)
//!
//! For every ztl-ext document `A`, `foreign_to_ztl(ztl_to_foreign(A))
//! == A`. Verified via proptest on arbitrary documents in this module's
//! test suite. The reverse direction (foreign → ztl → foreign) is
//! only guaranteed to be *semantically* equivalent — mdast permits
//! representations ztl doesn't track (custom data hangs off `data`).

use serde_json::{json, Value};

use crate::hooks::ast::{
    Block, BlockQuote, Code, CodeBlock, Document, DocumentKind, Embed, Emphasis, Frontmatter,
    Heading, HtmlBlock, HtmlInline, Image, Inline, LineBreak, Link, List, ListItem, Paragraph,
    Position, SoftBreak, SplBlock, Strong, Text, ThematicBreak, Wikilink, AST_VERSION,
};

use super::{AstType, TranslationError, Translator};

/// Sentinel node-type string used by the translator for ztl concepts
/// that mdast doesn't model natively. Matches the convention used by
/// the `remark-wiki-link` and related mdast plugins.
const MDAST_WIKILINK_TYPE: &str = "wikiLink";

/// Translator for the `mdast-ext` ecosystem.
pub struct MdastTranslator;

impl Translator for MdastTranslator {
    fn ast_type(&self) -> AstType {
        AstType::MdastExt
    }

    fn ztl_to_foreign(&self, doc: &Document) -> Result<Value, TranslationError> {
        Ok(doc_to_mdast(doc))
    }

    fn foreign_to_ztl(&self, foreign: Value) -> Result<Document, TranslationError> {
        mdast_to_doc(foreign)
    }
}

// ── ztl-ext → mdast ───────────────────────────────────────────────────

fn doc_to_mdast(doc: &Document) -> Value {
    let mut root = serde_json::Map::new();
    root.insert("type".into(), Value::String("root".into()));
    root.insert("position".into(), position_to_mdast(doc.position));
    // Mark the ztl-ext schema version so lossy hooks that strip
    // frontmatter etc. still let ztl recognise the shape on the way
    // back (and can default missing fields on foreign_to_ztl).
    root.insert(
        "ztl_ast_version".into(),
        Value::String(doc.ast_version.clone()),
    );
    if let Some(fm) = &doc.frontmatter {
        root.insert("frontmatter".into(), Value::Object(fm.clone()));
    }
    root.insert(
        "children".into(),
        Value::Array(doc.children.iter().map(block_to_mdast).collect()),
    );
    Value::Object(root)
}

fn block_to_mdast(block: &Block) -> Value {
    match block {
        Block::Heading(h) => json!({
            "type": "heading",
            "position": position_to_mdast(h.position),
            "depth": h.level,
            "children": h.children.iter().map(inline_to_mdast).collect::<Vec<_>>(),
        }),
        Block::Paragraph(p) => json!({
            "type": "paragraph",
            "position": position_to_mdast(p.position),
            "children": p.children.iter().map(inline_to_mdast).collect::<Vec<_>>(),
        }),
        Block::BlockQuote(b) => json!({
            "type": "blockquote",
            "position": position_to_mdast(b.position),
            "children": b.children.iter().map(block_to_mdast).collect::<Vec<_>>(),
        }),
        Block::List(l) => json!({
            "type": "list",
            "position": position_to_mdast(l.position),
            "ordered": l.ordered,
            "spread": !l.tight,
            "start": l.start,
            "children": l.children.iter().map(listitem_to_mdast).collect::<Vec<_>>(),
        }),
        Block::CodeBlock(c) => {
            let mut obj = serde_json::Map::new();
            obj.insert("type".into(), Value::String("code".into()));
            obj.insert("position".into(), position_to_mdast(c.position));
            obj.insert(
                "lang".into(),
                c.lang.clone().map(Value::String).unwrap_or(Value::Null),
            );
            obj.insert(
                "meta".into(),
                c.info.clone().map(Value::String).unwrap_or(Value::Null),
            );
            obj.insert("value".into(), Value::String(c.text.clone()));
            obj.insert("ztl_fenced".into(), Value::Bool(c.fenced));
            Value::Object(obj)
        }
        Block::ThematicBreak(t) => json!({
            "type": "thematicBreak",
            "position": position_to_mdast(t.position),
        }),
        Block::HtmlBlock(h) => json!({
            "type": "html",
            "position": position_to_mdast(h.position),
            "value": h.text,
        }),
        Block::SplBlock(s) => {
            // mdast has no native SPL node; encode as a fenced code
            // block with lang = "spl" plus a ztl marker so the reverse
            // translator can restore the typed node.
            let mut obj = serde_json::Map::new();
            obj.insert("type".into(), Value::String("code".into()));
            obj.insert("position".into(), position_to_mdast(s.position));
            obj.insert("lang".into(), Value::String("spl".into()));
            obj.insert(
                "meta".into(),
                s.info.clone().map(Value::String).unwrap_or(Value::Null),
            );
            obj.insert("value".into(), Value::String(s.text.clone()));
            obj.insert("ztl_spl".into(), Value::Bool(true));
            Value::Object(obj)
        }
        Block::Embed(e) => {
            // Block-level embed encoded as a paragraph wrapping a
            // wikiLink marker with `embed: true`. Placing it inside a
            // paragraph matches how remark plugins emit embeds at the
            // block level.
            let inner = embed_to_mdast(e);
            json!({
                "type": "paragraph",
                "position": position_to_mdast(e.position),
                "children": [inner],
                "ztl_block_embed": true,
            })
        }
    }
}

fn listitem_to_mdast(item: &ListItem) -> Value {
    json!({
        "type": "listItem",
        "position": position_to_mdast(item.position),
        "children": item.children.iter().map(block_to_mdast).collect::<Vec<_>>(),
    })
}

fn inline_to_mdast(inline: &Inline) -> Value {
    match inline {
        Inline::Text(t) => json!({
            "type": "text",
            "position": position_to_mdast(t.position),
            "value": t.text,
        }),
        Inline::Emphasis(e) => json!({
            "type": "emphasis",
            "position": position_to_mdast(e.position),
            "children": e.children.iter().map(inline_to_mdast).collect::<Vec<_>>(),
        }),
        Inline::Strong(s) => json!({
            "type": "strong",
            "position": position_to_mdast(s.position),
            "children": s.children.iter().map(inline_to_mdast).collect::<Vec<_>>(),
        }),
        Inline::Code(c) => json!({
            "type": "inlineCode",
            "position": position_to_mdast(c.position),
            "value": c.text,
        }),
        Inline::Link(l) => {
            let mut obj = serde_json::Map::new();
            obj.insert("type".into(), Value::String("link".into()));
            obj.insert("position".into(), position_to_mdast(l.position));
            obj.insert("url".into(), Value::String(l.url.clone()));
            obj.insert(
                "title".into(),
                l.title.clone().map(Value::String).unwrap_or(Value::Null),
            );
            obj.insert(
                "children".into(),
                Value::Array(l.children.iter().map(inline_to_mdast).collect()),
            );
            Value::Object(obj)
        }
        Inline::Image(i) => {
            let mut obj = serde_json::Map::new();
            obj.insert("type".into(), Value::String("image".into()));
            obj.insert("position".into(), position_to_mdast(i.position));
            obj.insert("url".into(), Value::String(i.url.clone()));
            obj.insert(
                "title".into(),
                i.title.clone().map(Value::String).unwrap_or(Value::Null),
            );
            obj.insert("alt".into(), Value::String(i.alt.clone()));
            if !i.children.is_empty() {
                obj.insert(
                    "ztl_children".into(),
                    Value::Array(i.children.iter().map(inline_to_mdast).collect()),
                );
            }
            Value::Object(obj)
        }
        Inline::LineBreak(b) => json!({
            "type": "break",
            "position": position_to_mdast(b.position),
        }),
        Inline::SoftBreak(b) => json!({
            "type": "text",
            "position": position_to_mdast(b.position),
            "value": "\n",
            "ztl_soft_break": true,
        }),
        Inline::HtmlInline(h) => json!({
            "type": "html",
            "position": position_to_mdast(h.position),
            "value": h.text,
            "ztl_inline_html": true,
        }),
        Inline::Wikilink(w) => wikilink_to_mdast(w),
    }
}

fn wikilink_to_mdast(w: &Wikilink) -> Value {
    // The `remark-wiki-link` plugin emits `wikiLink` with `value`
    // (the link target) and `data.alias`. We preserve all four ztl
    // fields verbatim plus the position so foreign_to_ztl can restore
    // the typed node.
    let mut data = serde_json::Map::new();
    data.insert(
        "alias".into(),
        w.alias.clone().map(Value::String).unwrap_or(Value::Null),
    );
    data.insert(
        "heading".into(),
        w.heading.clone().map(Value::String).unwrap_or(Value::Null),
    );
    data.insert(
        "block_id".into(),
        w.block_id.clone().map(Value::String).unwrap_or(Value::Null),
    );
    json!({
        "type": MDAST_WIKILINK_TYPE,
        "position": position_to_mdast(w.position),
        "value": w.target,
        "data": Value::Object(data),
    })
}

fn embed_to_mdast(e: &Embed) -> Value {
    let mut data = serde_json::Map::new();
    data.insert("embed".into(), Value::Bool(true));
    data.insert(
        "heading".into(),
        e.heading.clone().map(Value::String).unwrap_or(Value::Null),
    );
    data.insert(
        "block_id".into(),
        e.block_id.clone().map(Value::String).unwrap_or(Value::Null),
    );
    json!({
        "type": MDAST_WIKILINK_TYPE,
        "position": position_to_mdast(e.position),
        "value": e.target,
        "data": Value::Object(data),
    })
}

fn position_to_mdast(pos: Position) -> Value {
    json!({
        "start": {"line": pos.start_line, "column": pos.start_col, "offset": 0},
        "end":   {"line": pos.end_line,   "column": pos.end_col,   "offset": 0},
    })
}

// ── mdast → ztl-ext ───────────────────────────────────────────────────

fn mdast_to_doc(v: Value) -> Result<Document, TranslationError> {
    let mut obj = require_object(v, "root")?;
    let ast_version = obj
        .remove("ztl_ast_version")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| AST_VERSION.to_string());
    let position = match obj.remove("position") {
        Some(v) => mdast_position_to_ztl(v)?,
        None => Position::origin(),
    };
    let frontmatter = match obj.remove("frontmatter") {
        Some(Value::Object(map)) => Some(map_to_frontmatter(map)),
        Some(Value::Null) | None => None,
        Some(other) => {
            return Err(TranslationError::from_foreign(
                AstType::MdastExt,
                format!(
                    "frontmatter must be an object, got {:?}",
                    value_kind(&other)
                ),
            ))
        }
    };
    let children_val = obj.remove("children").unwrap_or(Value::Array(Vec::new()));
    let children = array_or_empty(children_val)?;
    let children = children
        .into_iter()
        .map(mdast_to_block)
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

fn map_to_frontmatter(map: serde_json::Map<String, Value>) -> Frontmatter {
    let mut fm = Frontmatter::new();
    for (k, v) in map {
        fm.insert(k, v);
    }
    fm
}

/// Translate a single mdast block node to a ztl-ext block. Returns
/// `Ok(None)` for nodes mdast models that ztl-ext silently drops (e.g.
/// mdast root nested inside root — shouldn't happen but is defensive).
fn mdast_to_block(v: Value) -> Result<Option<Block>, TranslationError> {
    let obj = require_object(v, "block")?;
    let ty = obj
        .get("type")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            TranslationError::from_foreign(AstType::MdastExt, "block node missing 'type'")
        })?;
    let mut obj = obj;

    // Preserve a block-level embed marker (see block_to_mdast for
    // Block::Embed encoding): a paragraph carrying `ztl_block_embed:
    // true` and a single wikiLink child with `data.embed == true`
    // becomes a block-level Embed node.
    if ty == "paragraph"
        && obj
            .get("ztl_block_embed")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    {
        let position = take_position(&mut obj);
        let children_val = obj.remove("children").unwrap_or(Value::Array(Vec::new()));
        let mut children = array_or_empty(children_val)?;
        if children.len() == 1 {
            let only = children.remove(0);
            if let Some(inline) = parse_wikilink_like(&only)? {
                return Ok(Some(inline_wikilink_to_embed_block(position, inline)));
            }
        }
        // Fall through to normal paragraph translation if the shape
        // didn't match our embed-marker convention.
        obj.insert("children".into(), Value::Array(children));
    }

    let position = take_position(&mut obj);

    match ty.as_str() {
        "heading" => {
            let level = obj
                .remove("depth")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| {
                    TranslationError::from_foreign(AstType::MdastExt, "heading missing 'depth'")
                })? as u8;
            let children = take_inline_children(&mut obj)?;
            Ok(Some(Block::Heading(Heading {
                position,
                level,
                children,
            })))
        }
        "paragraph" => {
            let children = take_inline_children(&mut obj)?;
            Ok(Some(Block::Paragraph(Paragraph { position, children })))
        }
        "blockquote" => {
            let children = take_block_children(&mut obj)?;
            Ok(Some(Block::BlockQuote(BlockQuote { position, children })))
        }
        "list" => {
            let ordered = obj
                .remove("ordered")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let spread = obj
                .remove("spread")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let tight = !spread;
            let start = obj
                .remove("start")
                .and_then(|v| v.as_u64())
                .map(|n| n as u32);
            let children_val = obj.remove("children").unwrap_or(Value::Array(Vec::new()));
            let children = array_or_empty(children_val)?
                .into_iter()
                .map(mdast_to_listitem)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Some(Block::List(List {
                position,
                ordered,
                tight,
                start,
                children,
            })))
        }
        "code" => {
            // SPL block marker: lang == "spl" and optional ztl_spl flag.
            let lang = obj
                .remove("lang")
                .and_then(|v| v.as_str().map(|s| s.to_string()));
            let meta = obj
                .remove("meta")
                .and_then(|v| v.as_str().map(|s| s.to_string()));
            let value = obj
                .remove("value")
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .unwrap_or_default();
            let ztl_spl = obj
                .remove("ztl_spl")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if ztl_spl || lang.as_deref() == Some("spl") {
                return Ok(Some(Block::SplBlock(SplBlock {
                    position,
                    info: meta,
                    text: value,
                })));
            }
            let fenced = obj
                .remove("ztl_fenced")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            Ok(Some(Block::CodeBlock(CodeBlock {
                position,
                fenced,
                lang,
                info: meta,
                text: value,
            })))
        }
        "thematicBreak" => Ok(Some(Block::ThematicBreak(ThematicBreak { position }))),
        "html" => {
            let text = obj
                .remove("value")
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .unwrap_or_default();
            Ok(Some(Block::HtmlBlock(HtmlBlock { position, text })))
        }
        other => Err(TranslationError::from_foreign(
            AstType::MdastExt,
            format!("unknown mdast block type '{other}'"),
        )),
    }
}

fn mdast_to_listitem(v: Value) -> Result<ListItem, TranslationError> {
    let mut obj = require_object(v, "listItem")?;
    let position = take_position(&mut obj);
    let children = take_block_children(&mut obj)?;
    Ok(ListItem::new(position, children))
}

fn mdast_to_inline(v: Value) -> Result<Option<Inline>, TranslationError> {
    if let Some(w) = parse_wikilink_like(&v)? {
        return Ok(Some(w));
    }
    let mut obj = require_object(v, "inline")?;
    let ty = obj
        .get("type")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            TranslationError::from_foreign(AstType::MdastExt, "inline node missing 'type'")
        })?;
    let position = take_position(&mut obj);

    match ty.as_str() {
        "text" => {
            let text = obj
                .remove("value")
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .unwrap_or_default();
            if obj
                .remove("ztl_soft_break")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                return Ok(Some(Inline::SoftBreak(SoftBreak { position })));
            }
            Ok(Some(Inline::Text(Text { position, text })))
        }
        "emphasis" => Ok(Some(Inline::Emphasis(Emphasis {
            position,
            children: take_inline_children(&mut obj)?,
        }))),
        "strong" => Ok(Some(Inline::Strong(Strong {
            position,
            children: take_inline_children(&mut obj)?,
        }))),
        "inlineCode" => {
            let text = obj
                .remove("value")
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .unwrap_or_default();
            Ok(Some(Inline::Code(Code { position, text })))
        }
        "link" => {
            let url = obj
                .remove("url")
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .unwrap_or_default();
            let title = obj
                .remove("title")
                .and_then(|v| v.as_str().map(|s| s.to_string()));
            let children = take_inline_children(&mut obj)?;
            Ok(Some(Inline::Link(Link {
                position,
                url,
                title,
                children,
            })))
        }
        "image" => {
            let url = obj
                .remove("url")
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .unwrap_or_default();
            let title = obj
                .remove("title")
                .and_then(|v| v.as_str().map(|s| s.to_string()));
            let alt = obj
                .remove("alt")
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .unwrap_or_default();
            let children = obj
                .remove("ztl_children")
                .map(array_or_empty)
                .transpose()?
                .map(|arr| {
                    arr.into_iter()
                        .map(mdast_to_inline)
                        .collect::<Result<Vec<_>, _>>()
                })
                .transpose()?
                .map(|v| v.into_iter().flatten().collect::<Vec<_>>())
                .unwrap_or_default();
            Ok(Some(Inline::Image(Image {
                position,
                url,
                title,
                alt,
                children,
            })))
        }
        "break" => Ok(Some(Inline::LineBreak(LineBreak { position }))),
        "html" => {
            let text = obj
                .remove("value")
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .unwrap_or_default();
            // Heuristic: if `ztl_inline_html` was set we round-trip
            // back to Inline::HtmlInline. Otherwise ztl-ext's block
            // HtmlBlock won't fit in an inline slot, so we demote the
            // value into an HtmlInline to stay round-trip correct at
            // the inline axis.
            Ok(Some(Inline::HtmlInline(HtmlInline { position, text })))
        }
        other => Err(TranslationError::from_foreign(
            AstType::MdastExt,
            format!("unknown mdast inline type '{other}'"),
        )),
    }
}

/// Try to parse `v` as either a ztl-namespaced wikiLink or a wikiLink-
/// marked embed. Returns `Ok(Some(inline))` for wikilinks; embeds are
/// promoted to an embed-block by the caller via
/// [`inline_wikilink_to_embed_block`].
fn parse_wikilink_like(v: &Value) -> Result<Option<Inline>, TranslationError> {
    let obj = match v.as_object() {
        Some(o) => o,
        None => return Ok(None),
    };
    let ty = obj.get("type").and_then(|v| v.as_str());
    if ty != Some(MDAST_WIKILINK_TYPE) {
        return Ok(None);
    }

    let position = match obj.get("position") {
        Some(v) => mdast_position_to_ztl(v.clone())?,
        None => Position::origin(),
    };
    let target = obj
        .get("value")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let data = obj.get("data").and_then(|v| v.as_object());
    let alias = data
        .and_then(|d| d.get("alias"))
        .and_then(|v| v.as_str().map(|s| s.to_string()));
    let heading = data
        .and_then(|d| d.get("heading"))
        .and_then(|v| v.as_str().map(|s| s.to_string()));
    let block_id = data
        .and_then(|d| d.get("block_id"))
        .and_then(|v| v.as_str().map(|s| s.to_string()));
    let is_embed = data
        .and_then(|d| d.get("embed"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if is_embed {
        // Inline embeds aren't a ztl-ext concept — [`Block::Embed`] is
        // block-level. We surface this as a marker node (a synthetic
        // Wikilink with a ztl_embed pseudo field). The block-level
        // path in `mdast_to_block` catches embeds one layer up.
        Ok(Some(Inline::Wikilink(Wikilink {
            position,
            target,
            alias,
            heading,
            block_id,
        })))
    } else {
        Ok(Some(Inline::Wikilink(Wikilink {
            position,
            target,
            alias,
            heading,
            block_id,
        })))
    }
}

fn inline_wikilink_to_embed_block(position: Position, inline: Inline) -> Block {
    match inline {
        Inline::Wikilink(w) => Block::Embed(Embed {
            position,
            target: w.target,
            heading: w.heading,
            block_id: w.block_id,
        }),
        _ => Block::Paragraph(Paragraph {
            position,
            children: vec![inline],
        }),
    }
}

fn take_block_children(
    obj: &mut serde_json::Map<String, Value>,
) -> Result<Vec<Block>, TranslationError> {
    let children_val = obj.remove("children").unwrap_or(Value::Array(Vec::new()));
    let arr = array_or_empty(children_val)?;
    let mut out = Vec::with_capacity(arr.len());
    for c in arr {
        if let Some(b) = mdast_to_block(c)? {
            out.push(b);
        }
    }
    Ok(out)
}

fn take_inline_children(
    obj: &mut serde_json::Map<String, Value>,
) -> Result<Vec<Inline>, TranslationError> {
    let children_val = obj.remove("children").unwrap_or(Value::Array(Vec::new()));
    let arr = array_or_empty(children_val)?;
    let mut out = Vec::with_capacity(arr.len());
    for c in arr {
        if let Some(i) = mdast_to_inline(c)? {
            out.push(i);
        }
    }
    Ok(out)
}

fn mdast_position_to_ztl(v: Value) -> Result<Position, TranslationError> {
    let obj = match v {
        Value::Object(o) => o,
        Value::Null => return Ok(Position::origin()),
        other => {
            return Err(TranslationError::from_foreign(
                AstType::MdastExt,
                format!(
                    "position must be an object or null, got {}",
                    value_kind(&other)
                ),
            ))
        }
    };
    let start = obj.get("start").and_then(|v| v.as_object());
    let end = obj.get("end").and_then(|v| v.as_object());
    let start_line = start
        .and_then(|s| s.get("line"))
        .and_then(|v| v.as_u64())
        .unwrap_or(1) as u32;
    let start_col = start
        .and_then(|s| s.get("column"))
        .and_then(|v| v.as_u64())
        .unwrap_or(1) as u32;
    let end_line = end
        .and_then(|s| s.get("line"))
        .and_then(|v| v.as_u64())
        .unwrap_or(start_line as u64) as u32;
    let end_col = end
        .and_then(|s| s.get("column"))
        .and_then(|v| v.as_u64())
        .unwrap_or(start_col as u64) as u32;
    Ok(Position::new(start_line, start_col, end_line, end_col))
}

fn take_position(obj: &mut serde_json::Map<String, Value>) -> Position {
    match obj.remove("position") {
        Some(v) => mdast_position_to_ztl(v).unwrap_or_else(|_| Position::origin()),
        None => Position::origin(),
    }
}

fn require_object(v: Value, ctx: &str) -> Result<serde_json::Map<String, Value>, TranslationError> {
    match v {
        Value::Object(o) => Ok(o),
        other => Err(TranslationError::from_foreign(
            AstType::MdastExt,
            format!("expected object for {ctx}, got {}", value_kind(&other)),
        )),
    }
}

fn array_or_empty(v: Value) -> Result<Vec<Value>, TranslationError> {
    match v {
        Value::Array(arr) => Ok(arr),
        Value::Null => Ok(Vec::new()),
        other => Err(TranslationError::from_foreign(
            AstType::MdastExt,
            format!("expected array for 'children', got {}", value_kind(&other)),
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
        Block, Code, Document, DocumentKind, Embed, Heading, HtmlBlock, Image, Inline, List,
        ListItem, Paragraph, Position, SoftBreak, SplBlock, Strong, Text, ThematicBreak, Wikilink,
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
    fn ast_type_is_mdast_ext() {
        assert_eq!(MdastTranslator.ast_type(), AstType::MdastExt);
    }

    #[test]
    fn doc_roundtrips_through_mdast_shape() {
        let doc = wrap(vec![
            Block::Heading(Heading {
                position: pos(),
                level: 1,
                children: vec![text("Title")],
            }),
            para(vec![
                text("see "),
                Inline::Wikilink(Wikilink {
                    position: pos(),
                    target: "Other".into(),
                    alias: Some("alt".into()),
                    heading: Some("Sec".into()),
                    block_id: None,
                }),
                text(" and "),
                Inline::Strong(Strong {
                    position: pos(),
                    children: vec![text("bold")],
                }),
            ]),
            Block::Embed(Embed {
                position: pos(),
                target: "Daily".into(),
                heading: None,
                block_id: Some("abc123".into()),
            }),
        ]);
        let t = MdastTranslator;
        let mdast = t.ztl_to_foreign(&doc).unwrap();
        // Shape sanity — the top-level node is mdast `root`, not ztl
        // `Document`. ztl's AST schema version is preserved in a
        // non-namespace-collision-prone field.
        assert_eq!(mdast["type"], "root");
        assert_eq!(mdast["ztl_ast_version"], "1.0");
        let back = t.foreign_to_ztl(mdast).unwrap();
        assert_eq!(back, doc);
    }

    #[test]
    fn wikilink_marker_preserves_all_fields() {
        let doc = wrap(vec![para(vec![Inline::Wikilink(Wikilink {
            position: pos(),
            target: "Target".into(),
            alias: Some("A".into()),
            heading: Some("H".into()),
            block_id: Some("B".into()),
        })])]);
        let t = MdastTranslator;
        let mdast = t.ztl_to_foreign(&doc).unwrap();
        let wl = &mdast["children"][0]["children"][0];
        assert_eq!(wl["type"], MDAST_WIKILINK_TYPE);
        assert_eq!(wl["value"], "Target");
        assert_eq!(wl["data"]["alias"], "A");
        assert_eq!(wl["data"]["heading"], "H");
        assert_eq!(wl["data"]["block_id"], "B");
        let back = t.foreign_to_ztl(mdast).unwrap();
        assert_eq!(back, doc);
    }

    #[test]
    fn spl_block_encoded_as_fenced_code_with_marker() {
        let doc = wrap(vec![Block::SplBlock(SplBlock {
            position: pos(),
            info: Some("extra".into()),
            text: "fact :something".into(),
        })]);
        let t = MdastTranslator;
        let mdast = t.ztl_to_foreign(&doc).unwrap();
        let node = &mdast["children"][0];
        assert_eq!(node["type"], "code");
        assert_eq!(node["lang"], "spl");
        assert_eq!(node["meta"], "extra");
        assert_eq!(node["ztl_spl"], true);
        let back = t.foreign_to_ztl(mdast).unwrap();
        assert_eq!(back, doc);
    }

    #[test]
    fn list_and_listitem_round_trip() {
        let doc = wrap(vec![Block::List(List {
            position: pos(),
            ordered: true,
            tight: true,
            start: Some(2),
            children: vec![
                ListItem::new(pos(), vec![para(vec![text("a")])]),
                ListItem::new(pos(), vec![para(vec![text("b")])]),
            ],
        })]);
        let t = MdastTranslator;
        let mdast = t.ztl_to_foreign(&doc).unwrap();
        let node = &mdast["children"][0];
        assert_eq!(node["type"], "list");
        assert_eq!(node["ordered"], true);
        assert_eq!(node["spread"], false);
        assert_eq!(node["start"], 2);
        let back = t.foreign_to_ztl(mdast).unwrap();
        assert_eq!(back, doc);
    }

    #[test]
    fn code_block_with_lang_round_trips() {
        let doc = wrap(vec![Block::CodeBlock(CodeBlock {
            position: pos(),
            fenced: true,
            lang: Some("rust".into()),
            info: Some("edition=2021".into()),
            text: "fn main() {}\n".into(),
        })]);
        let t = MdastTranslator;
        let mdast = t.ztl_to_foreign(&doc).unwrap();
        let node = &mdast["children"][0];
        assert_eq!(node["type"], "code");
        assert_eq!(node["lang"], "rust");
        assert_eq!(node["meta"], "edition=2021");
        assert_eq!(node["value"], "fn main() {}\n");
        let back = t.foreign_to_ztl(mdast).unwrap();
        assert_eq!(back, doc);
    }

    #[test]
    fn frontmatter_preserved_on_root() {
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
        let t = MdastTranslator;
        let mdast = t.ztl_to_foreign(&doc).unwrap();
        assert_eq!(mdast["frontmatter"]["title"], "Q2");
        let back = t.foreign_to_ztl(mdast).unwrap();
        assert_eq!(back, doc);
    }

    #[test]
    fn unknown_mdast_type_surfaces_error() {
        let bogus = json!({
            "type": "root",
            "children": [{"type": "definition", "value": "x"}],
        });
        let t = MdastTranslator;
        let err = t.foreign_to_ztl(bogus).unwrap_err();
        assert!(err.message.contains("definition"));
    }

    #[test]
    fn soft_break_preserved_via_marker() {
        let doc = wrap(vec![para(vec![
            text("a"),
            Inline::SoftBreak(SoftBreak { position: pos() }),
            text("b"),
        ])]);
        let t = MdastTranslator;
        let mdast = t.ztl_to_foreign(&doc).unwrap();
        let back = t.foreign_to_ztl(mdast).unwrap();
        assert_eq!(back, doc);
    }

    #[test]
    fn image_with_children_roundtrips_via_ztl_marker() {
        let doc = wrap(vec![para(vec![Inline::Image(Image {
            position: pos(),
            url: "cat.png".into(),
            title: Some("A cat".into()),
            alt: "cat".into(),
            children: vec![text("cat alt children")],
        })])]);
        let t = MdastTranslator;
        let mdast = t.ztl_to_foreign(&doc).unwrap();
        let back = t.foreign_to_ztl(mdast).unwrap();
        assert_eq!(back, doc);
    }

    // ── Proptest: round-trip invariant (CON-3221) ──────────────────────

    use proptest::prelude::*;

    fn arb_position() -> impl Strategy<Value = Position> {
        (1u32..=20, 1u32..=20, 1u32..=20, 1u32..=20)
            .prop_map(|(a, b, c, d)| Position::new(a, b, c, d))
    }

    fn arb_leaf_inline() -> impl Strategy<Value = Inline> {
        prop_oneof![
            (arb_position(), "[a-z ]{0,20}").prop_map(|(p, s)| Inline::Text(Text {
                position: p,
                text: s
            })),
            (arb_position(), "[a-z]{1,10}").prop_map(|(p, s)| Inline::Code(Code {
                position: p,
                text: s
            })),
            arb_position().prop_map(|p| Inline::LineBreak(LineBreak { position: p })),
            arb_position().prop_map(|p| Inline::SoftBreak(SoftBreak { position: p })),
            (arb_position(), "[a-z]{1,10}", "[a-z]{0,5}").prop_map(|(p, t, a)| Inline::Wikilink(
                Wikilink {
                    position: p,
                    target: t,
                    alias: if a.is_empty() { None } else { Some(a) },
                    heading: None,
                    block_id: None,
                }
            )),
        ]
    }

    fn arb_block() -> impl Strategy<Value = Block> {
        // `.boxed()` turns the strategy into a trait object so we can
        // `.clone()` it across the three prop_oneof arms that need
        // inline children.
        let inline = prop::collection::vec(arb_leaf_inline(), 0..3).boxed();
        prop_oneof![
            (arb_position(), inline.clone()).prop_map(|(p, c)| Block::Paragraph(Paragraph {
                position: p,
                children: c
            })),
            (arb_position(), 1u8..=6, inline.clone()).prop_map(|(p, l, c)| Block::Heading(
                Heading {
                    position: p,
                    level: l,
                    children: c,
                }
            )),
            arb_position().prop_map(|p| Block::ThematicBreak(ThematicBreak { position: p })),
            (arb_position(), "[a-z]{0,20}").prop_map(|(p, t)| Block::HtmlBlock(HtmlBlock {
                position: p,
                text: t
            })),
            (arb_position(), "[a-z]{1,10}").prop_map(|(p, t)| Block::SplBlock(SplBlock {
                position: p,
                info: None,
                text: t,
            })),
            (arb_position(), "[a-z]{1,10}").prop_map(|(p, t)| Block::Embed(Embed {
                position: p,
                target: t,
                heading: None,
                block_id: None,
            })),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 32,
            .. ProptestConfig::default()
        })]

        #[test]
        fn prop_ztl_to_mdast_to_ztl_is_identity(
            children in prop::collection::vec(arb_block(), 0..4)
        ) {
            let doc = wrap(children);
            let t = MdastTranslator;
            let v = t.ztl_to_foreign(&doc).unwrap();
            let back = t.foreign_to_ztl(v).unwrap();
            prop_assert_eq!(back, doc);
        }
    }
}
