//! SPEC-033 REQ-3307 / TEST-3307 integration coverage for the zetl-ext ↔
//! pandoc-types translator (task-pandoc-translator).
//!
//! The translator itself lives in `src/hooks/translators/pandoc.rs`, which
//! ships with module-local unit tests + a proptest round-trip. This file
//! pins TEST-3307's hand-crafted edge-case fixtures called out in the
//! spec (§TEST-3307, TEST-3308: Translation Correctness) and the
//! "marker-table" instances of the slimmed REQ-3307 model:
//!
//! - Wikilinks inside a BlockQuote.
//! - Embed with both `heading` and `block_id`.
//! - SPL block with trailing whitespace.
//! - Frontmatter with nested structures.
//! - Position fidelity across translation.
//! - Pandoc marker-table attrs (Span class `zetl-wikilink` / Div class
//!   `zetl-embed` / CodeBlock class `spl`) preserved across round-trip.
//!
//! The proptest fidelity gate (NFR-3305 canonical-form equivalence) lives
//! in `src/hooks/translators/roundtrip.rs` — this file complements it
//! with named, human-readable fixtures whose failure modes are easy to
//! triage when something regresses.

use serde_json::json;

use zetl::hooks::ast::{
    Block, BlockQuote, Document, DocumentKind, Embed, Frontmatter, Heading, Inline, ListItem,
    Paragraph, Position, SplBlock, Text, Wikilink, AST_VERSION,
};
use zetl::hooks::translators::pandoc::PandocTranslator;
use zetl::hooks::translators::{AstType, Translator};

// ── Helpers ──────────────────────────────────────────────────────────────

fn doc(children: Vec<Block>) -> Document {
    Document {
        ast_version: AST_VERSION.to_string(),
        kind: DocumentKind::Document,
        position: Position::origin(),
        frontmatter: None,
        children,
    }
}

fn doc_with_frontmatter(fm: Frontmatter, children: Vec<Block>) -> Document {
    Document {
        ast_version: AST_VERSION.to_string(),
        kind: DocumentKind::Document,
        position: Position::origin(),
        frontmatter: Some(fm),
        children,
    }
}

fn text(s: &str) -> Inline {
    Inline::Text(Text {
        position: Position::origin(),
        text: s.to_string(),
    })
}

fn text_at(pos: Position, s: &str) -> Inline {
    Inline::Text(Text {
        position: pos,
        text: s.to_string(),
    })
}

fn wikilink(target: &str) -> Inline {
    Inline::Wikilink(Wikilink {
        position: Position::origin(),
        target: target.to_string(),
        alias: None,
        heading: None,
        block_id: None,
    })
}

fn wikilink_full(target: &str, alias: &str, heading: &str, block_id: &str) -> Inline {
    Inline::Wikilink(Wikilink {
        position: Position::origin(),
        target: target.to_string(),
        alias: Some(alias.to_string()),
        heading: Some(heading.to_string()),
        block_id: Some(block_id.to_string()),
    })
}

fn paragraph(children: Vec<Inline>) -> Block {
    Block::Paragraph(Paragraph {
        position: Position::origin(),
        children,
    })
}

/// Look up a pandoc Attr 3-tuple's attr map by key. The Attr shape is
/// `[id, [classes...], [[k,v], ...]]`.
fn attr_get<'a>(attr: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    attr.get(2)?.as_array()?.iter().find_map(|pair| {
        let arr = pair.as_array()?;
        (arr.first()?.as_str()? == key).then_some(arr.get(1)?.as_str()?)
    })
}

/// Extract the `classes` array from an Attr 3-tuple.
fn attr_classes(attr: &serde_json::Value) -> Vec<&str> {
    attr.get(1)
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default()
}

// ── TEST-3307 fixture: Wikilinks inside a BlockQuote ─────────────────────

#[test]
fn wikilinks_inside_a_blockquote_round_trip() {
    let input = doc(vec![Block::BlockQuote(BlockQuote {
        position: Position::origin(),
        children: vec![paragraph(vec![
            text("see "),
            wikilink_full("Other", "alt", "Sec", "blk"),
            text(" and "),
            wikilink("Another"),
            text("."),
        ])],
    })]);

    let t = PandocTranslator;
    let pandoc = t.zetl_to_foreign(&input).unwrap();

    // Shape probe: blocks[0] is BlockQuote containing one Para; Para's
    // inline children include two Span[zetl-wikilink] markers.
    let bq = &pandoc["blocks"][0];
    assert_eq!(bq["t"], "BlockQuote");
    let para = &bq["c"][0];
    assert_eq!(para["t"], "Para");
    let kids = &para["c"];
    assert_eq!(kids[1]["t"], "Span");
    let attr1 = &kids[1]["c"][0];
    assert!(attr_classes(attr1).contains(&"zetl-wikilink"));
    assert_eq!(attr_get(attr1, "zetl-target"), Some("Other"));
    assert_eq!(attr_get(attr1, "zetl-alias"), Some("alt"));
    assert_eq!(attr_get(attr1, "zetl-heading"), Some("Sec"));
    assert_eq!(attr_get(attr1, "zetl-block-id"), Some("blk"));
    assert_eq!(kids[3]["t"], "Span");
    let attr2 = &kids[3]["c"][0];
    assert_eq!(attr_get(attr2, "zetl-target"), Some("Another"));
    // Bare wikilink omits the optional attr keys entirely.
    assert_eq!(attr_get(attr2, "zetl-alias"), None);

    let back = t.foreign_to_zetl(pandoc).unwrap();
    assert_eq!(back, input);
}

// ── TEST-3307 fixture: Embed with heading AND block_id ───────────────────

#[test]
fn embed_with_both_heading_and_block_id_round_trips() {
    let input = doc(vec![Block::Embed(Embed {
        position: Position::origin(),
        target: "Daily".into(),
        heading: Some("Morning".into()),
        block_id: Some("abc123".into()),
    })]);

    let t = PandocTranslator;
    let pandoc = t.zetl_to_foreign(&input).unwrap();

    // Embed encodes as a Div with class `zetl-embed` and marker attrs
    // for target/heading/block_id; block-level container is required
    // because zetl-ext Embed is a block node.
    let div = &pandoc["blocks"][0];
    assert_eq!(div["t"], "Div");
    let attr = &div["c"][0];
    assert!(attr_classes(attr).contains(&"zetl-embed"));
    assert_eq!(attr_get(attr, "zetl-target"), Some("Daily"));
    assert_eq!(attr_get(attr, "zetl-heading"), Some("Morning"));
    assert_eq!(attr_get(attr, "zetl-block-id"), Some("abc123"));

    let back = t.foreign_to_zetl(pandoc).unwrap();
    assert_eq!(back, input);
}

// ── TEST-3307 fixture: SPL block with trailing whitespace ────────────────

#[test]
fn spl_block_with_trailing_whitespace_round_trips() {
    // Content intentionally carries trailing whitespace + a trailing
    // newline; the translator must not strip or collapse it.
    let spl_text = "fact :something \n  \n";
    let input = doc(vec![Block::SplBlock(SplBlock {
        position: Position::origin(),
        info: Some("extra".into()),
        text: spl_text.into(),
    })]);

    let t = PandocTranslator;
    let pandoc = t.zetl_to_foreign(&input).unwrap();
    let node = &pandoc["blocks"][0];
    assert_eq!(node["t"], "CodeBlock");
    let attr = &node["c"][0];
    assert!(attr_classes(attr).contains(&"spl"));
    assert_eq!(attr_get(attr, "zetl-spl"), Some("true"));
    assert_eq!(attr_get(attr, "zetl-info"), Some("extra"));
    assert_eq!(node["c"][1], spl_text);

    let back = t.foreign_to_zetl(pandoc).unwrap();
    assert_eq!(back, input);
    // And specifically that the trailing whitespace survived.
    if let Block::SplBlock(s) = &back.children[0] {
        assert_eq!(s.text, spl_text);
    } else {
        panic!("expected SplBlock after round-trip");
    }
}

// ── TEST-3307 fixture: Frontmatter with nested structures ────────────────

#[test]
fn frontmatter_with_nested_structures_round_trips() {
    let mut fm = Frontmatter::new();
    fm.insert("title".into(), json!("Q2"));
    fm.insert("tags".into(), json!(["a", "b", "c"]));
    fm.insert(
        "author".into(),
        json!({
            "name": "A",
            "aliases": ["Alpha", "A."]
        }),
    );
    fm.insert(
        "metrics".into(),
        json!({
            "words": 1024,
            "subsections": [
                { "heading": "Intro", "count": 12 },
                { "heading": "Body",  "count": 30 }
            ]
        }),
    );

    let input = doc_with_frontmatter(fm, vec![paragraph(vec![text("body")])]);

    let t = PandocTranslator;
    let pandoc = t.zetl_to_foreign(&input).unwrap();

    // Frontmatter lands in pandoc's Meta map under `zetl-frontmatter` as
    // a MetaInlines-encoded JSON string — preserves arbitrary nested
    // structure even when pandoc filters don't know how to represent it
    // natively. Round-trip brings it back byte-identical.
    let meta = &pandoc["meta"];
    assert!(meta.is_object(), "meta must be a pandoc Meta map");
    assert_eq!(meta["zetl-frontmatter"]["t"], "MetaInlines");
    let serialised = meta["zetl-frontmatter"]["c"][0]["c"]
        .as_str()
        .expect("frontmatter json string");
    let parsed: serde_json::Value = serde_json::from_str(serialised).unwrap();
    assert_eq!(parsed["title"], "Q2");
    assert_eq!(parsed["tags"][0], "a");
    assert_eq!(parsed["author"]["aliases"][1], "A.");
    assert_eq!(parsed["metrics"]["subsections"][1]["heading"], "Body");

    let back = t.foreign_to_zetl(pandoc).unwrap();
    assert_eq!(back, input);
}

// ── TEST-3307 fixture: Position fidelity across translation ──────────────

#[test]
fn position_information_is_preserved_across_translation() {
    let doc_pos = Position::new(1, 1, 7, 1);
    let hd_pos = Position::new(1, 1, 1, 9);
    let para_pos = Position::new(3, 1, 5, 12);
    let txt_pos = Position::new(3, 1, 3, 6);
    let wl_pos = Position::new(3, 7, 3, 20);

    let input = Document {
        ast_version: AST_VERSION.to_string(),
        kind: DocumentKind::Document,
        position: doc_pos,
        frontmatter: None,
        children: vec![
            Block::Heading(Heading {
                position: hd_pos,
                level: 2,
                children: vec![text_at(hd_pos, "Title")],
            }),
            Block::Paragraph(Paragraph {
                position: para_pos,
                children: vec![
                    text_at(txt_pos, "see "),
                    Inline::Wikilink(Wikilink {
                        position: wl_pos,
                        target: "Target".into(),
                        alias: None,
                        heading: None,
                        block_id: None,
                    }),
                ],
            }),
        ],
    };

    let t = PandocTranslator;
    let pandoc = t.zetl_to_foreign(&input).unwrap();

    // Pandoc has no native position concept; zetl-position rides as an
    // attr on every node that has an Attr tuple (Header, Span, …) and as
    // a top-level extra key on node shapes that don't (Para, Str).
    let header = &pandoc["blocks"][0];
    assert_eq!(header["t"], "Header");
    let hd_attr = &header["c"][1];
    assert_eq!(attr_get(hd_attr, "zetl-position"), Some("1:1-1:9"));

    let para = &pandoc["blocks"][1];
    assert_eq!(para["t"], "Para");
    assert_eq!(para["zetl-position"], "3:1-5:12");

    // The wikilink Span lives inside Para's inline children.
    let span = &para["c"][1];
    assert_eq!(span["t"], "Span");
    let wl_attr = &span["c"][0];
    assert_eq!(attr_get(wl_attr, "zetl-position"), Some("3:7-3:20"));

    let back = t.foreign_to_zetl(pandoc).unwrap();
    assert_eq!(back, input);

    // And a direct positional probe on the rehydrated doc — the header
    // and the wikilink should have survived byte-for-byte.
    if let Block::Heading(h) = &back.children[0] {
        assert_eq!(h.position, hd_pos);
    } else {
        panic!("expected Heading");
    }
    if let Block::Paragraph(p) = &back.children[1] {
        if let Inline::Wikilink(w) = &p.children[1] {
            assert_eq!(w.position, wl_pos);
        } else {
            panic!("expected Wikilink");
        }
    } else {
        panic!("expected Paragraph");
    }
}

// ── Pandoc marker-table attrs preserved across round-trip ────────────────

/// The four marker attrs that zetl stores on the `zetl-wikilink` Span's
/// Attr map (`target`, `alias`, `heading`, `block_id`) are the REQ-3307
/// slimmed-model marker convention. This test pins the acceptance
/// criterion "pandoc marker attrs preserved across round-trip" — a
/// zetl-ext document carrying every combination round-trips through the
/// pandoc adapter without any attr going missing.
#[test]
fn pandoc_wikilink_marker_attrs_all_survive_round_trip() {
    let input = doc(vec![paragraph(vec![
        // Bare wikilink (no optional attrs).
        wikilink("Bare"),
        // Alias only.
        Inline::Wikilink(Wikilink {
            position: Position::origin(),
            target: "Aliased".into(),
            alias: Some("display".into()),
            heading: None,
            block_id: None,
        }),
        // Heading only.
        Inline::Wikilink(Wikilink {
            position: Position::origin(),
            target: "Heading".into(),
            alias: None,
            heading: Some("Section".into()),
            block_id: None,
        }),
        // Block-id only.
        Inline::Wikilink(Wikilink {
            position: Position::origin(),
            target: "Anchored".into(),
            alias: None,
            heading: None,
            block_id: Some("blk42".into()),
        }),
        // All three set together.
        wikilink_full("All", "x", "y", "z"),
    ])]);

    let t = PandocTranslator;
    let pandoc = t.zetl_to_foreign(&input).unwrap();

    // Verify every wikilink kept its Attr map shape on the wire; absent
    // optionals do not appear in the attr list at all (distinguishing
    // `None` from `Some("")`).
    let kids = &pandoc["blocks"][0]["c"];
    let a0 = &kids[0]["c"][0];
    assert_eq!(attr_get(a0, "zetl-target"), Some("Bare"));
    assert_eq!(attr_get(a0, "zetl-alias"), None);
    let a1 = &kids[1]["c"][0];
    assert_eq!(attr_get(a1, "zetl-alias"), Some("display"));
    let a2 = &kids[2]["c"][0];
    assert_eq!(attr_get(a2, "zetl-heading"), Some("Section"));
    let a3 = &kids[3]["c"][0];
    assert_eq!(attr_get(a3, "zetl-block-id"), Some("blk42"));
    let a4 = &kids[4]["c"][0];
    assert_eq!(attr_get(a4, "zetl-alias"), Some("x"));
    assert_eq!(attr_get(a4, "zetl-heading"), Some("y"));
    assert_eq!(attr_get(a4, "zetl-block-id"), Some("z"));

    let back = t.foreign_to_zetl(pandoc).unwrap();
    assert_eq!(back, input);
}

/// Embed carries `zetl-target` + optional `zetl-heading` / `zetl-block-id`
/// marker attrs on its `zetl-embed` Div. This test pins that the
/// block-level embed marker round-trips regardless of which combination
/// of optional attrs is present.
#[test]
fn pandoc_embed_marker_attrs_all_survive_round_trip() {
    let input = doc(vec![
        Block::Embed(Embed {
            position: Position::origin(),
            target: "Bare".into(),
            heading: None,
            block_id: None,
        }),
        Block::Embed(Embed {
            position: Position::origin(),
            target: "Anchored".into(),
            heading: None,
            block_id: Some("blk".into()),
        }),
        Block::Embed(Embed {
            position: Position::origin(),
            target: "Sectioned".into(),
            heading: Some("Section".into()),
            block_id: None,
        }),
        Block::Embed(Embed {
            position: Position::origin(),
            target: "Both".into(),
            heading: Some("Section".into()),
            block_id: Some("blk".into()),
        }),
    ]);

    let t = PandocTranslator;
    let pandoc = t.zetl_to_foreign(&input).unwrap();

    for i in 0..4 {
        let div = &pandoc["blocks"][i];
        assert_eq!(div["t"], "Div");
        let attr = &div["c"][0];
        assert!(attr_classes(attr).contains(&"zetl-embed"));
    }

    let back = t.foreign_to_zetl(pandoc).unwrap();
    assert_eq!(back, input);
}

// ── Slimmed-model marker-table sanity ────────────────────────────────────

/// REQ-3307 slimmed model: the `pandoc-ext` translator emits a canonical
/// pandoc-types document envelope (`pandoc-api-version` + `meta` +
/// `blocks`) and carries the `zetl-ast-version` marker in `meta` so
/// lossy foreign hooks can still be round-tripped through
/// `foreign_to_zetl` without losing the schema pin.
#[test]
fn pandoc_root_envelope_carries_zetl_ast_version_marker() {
    let input = doc(vec![paragraph(vec![text("x")])]);
    let t = PandocTranslator;
    let pandoc = t.zetl_to_foreign(&input).unwrap();
    assert!(pandoc["pandoc-api-version"].is_array());
    assert!(pandoc["meta"].is_object());
    assert!(pandoc["blocks"].is_array());
    assert_eq!(pandoc["meta"]["zetl-ast-version"]["t"], "MetaInlines");
    assert_eq!(pandoc["meta"]["zetl-ast-version"]["c"][0]["c"], AST_VERSION);
}

/// Per REQ-3307, the translator declares `AstType::PandocExt` — a
/// regression floor: renaming the ast_type would break every
/// composition-level manifest that declares `ast_type = "pandoc-ext"`.
#[test]
fn translator_reports_pandoc_ext_ast_type() {
    assert_eq!(PandocTranslator.ast_type(), AstType::PandocExt);
}

// ── List + ListItem under pandoc shape (slimmed-model completeness) ──────

/// A nested ordered list inside a bullet list — a common CommonMark
/// fixture that tests the translator's `BulletList` / `OrderedList`
/// branching. Pandoc distinguishes the two via different `t` tags; a
/// regression would show as a round-trip mismatch on the `ordered` flag
/// or on the nested children.
#[test]
fn nested_mixed_lists_round_trip() {
    use zetl::hooks::ast::List;
    let input = doc(vec![Block::List(List {
        position: Position::origin(),
        ordered: false,
        tight: true,
        start: None,
        children: vec![
            ListItem::new(Position::origin(), vec![paragraph(vec![text("outer 1")])]),
            ListItem::new(
                Position::origin(),
                vec![
                    paragraph(vec![text("outer 2")]),
                    Block::List(List {
                        position: Position::origin(),
                        ordered: true,
                        tight: true,
                        start: Some(1),
                        children: vec![ListItem::new(
                            Position::origin(),
                            vec![paragraph(vec![text("inner 1")])],
                        )],
                    }),
                ],
            ),
        ],
    })]);

    let t = PandocTranslator;
    let pandoc = t.zetl_to_foreign(&input).unwrap();
    let outer = &pandoc["blocks"][0];
    assert_eq!(outer["t"], "BulletList");
    // Inner list lives in item[1]'s block-children (index 1 after the
    // outer-2 paragraph).
    let inner = &outer["c"][1][1];
    assert_eq!(inner["t"], "OrderedList");
    // OrderedList.c = [[start, style, delim], items]; start is the first
    // element of the attr triple.
    assert_eq!(inner["c"][0][0], 1);

    let back = t.foreign_to_zetl(pandoc).unwrap();
    assert_eq!(back, input);
}
