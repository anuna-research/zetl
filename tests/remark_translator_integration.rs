//! SPEC-033 REQ-3308 / TEST-3308 integration coverage for the ztl-ext ↔
//! mdast translator (task-remark-translator).
//!
//! The translator itself lives in `src/hooks/translators/mdast.rs`, which
//! ships with module-local unit tests + a proptest round-trip. This file
//! pins TEST-3308's hand-crafted edge-case fixtures called out in the
//! spec (§TEST-3307, TEST-3308: Translation Correctness) and the
//! "marker-table" instances of the slimmed REQ-3308 model:
//!
//! - Wikilinks inside a BlockQuote.
//! - Embed with both `heading` and `block_id`.
//! - SPL block with trailing whitespace.
//! - Frontmatter with nested structures.
//! - Position fidelity across translation.
//! - mdast extension attrs (data.* on the `wikiLink` marker) preserved
//!   across the round-trip.
//!
//! The proptest fidelity gate (NFR-3305 canonical-form equivalence) lives
//! in `src/hooks/translators/roundtrip.rs` — this file complements it
//! with named, human-readable fixtures whose failure modes are easy to
//! triage when something regresses.

use serde_json::{json, Value};

use ztl::hooks::ast::{
    Block, BlockQuote, Document, DocumentKind, Embed, Frontmatter, Heading, Inline, ListItem,
    Paragraph, Position, SplBlock, Text, Wikilink, AST_VERSION,
};
use ztl::hooks::translators::mdast::MdastTranslator;
use ztl::hooks::translators::{AstType, Translator};

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

// ── TEST-3308 fixture: Wikilinks inside a BlockQuote ─────────────────────

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

    let t = MdastTranslator;
    let mdast = t.ztl_to_foreign(&input).unwrap();

    // Shape probe: blockquote → paragraph → [text, wikiLink, text, wikiLink, text].
    assert_eq!(mdast["children"][0]["type"], "blockquote");
    let para = &mdast["children"][0]["children"][0];
    assert_eq!(para["type"], "paragraph");
    let kids = &para["children"];
    assert_eq!(kids[1]["type"], "wikiLink");
    assert_eq!(kids[1]["value"], "Other");
    assert_eq!(kids[1]["data"]["alias"], "alt");
    assert_eq!(kids[1]["data"]["heading"], "Sec");
    assert_eq!(kids[1]["data"]["block_id"], "blk");
    assert_eq!(kids[3]["type"], "wikiLink");
    assert_eq!(kids[3]["value"], "Another");

    let back = t.foreign_to_ztl(mdast).unwrap();
    assert_eq!(back, input);
}

// ── TEST-3308 fixture: Embed with heading AND block_id ───────────────────

#[test]
fn embed_with_both_heading_and_block_id_round_trips() {
    let input = doc(vec![Block::Embed(Embed {
        position: Position::origin(),
        target: "Daily".into(),
        heading: Some("Morning".into()),
        block_id: Some("abc123".into()),
    })]);

    let t = MdastTranslator;
    let mdast = t.ztl_to_foreign(&input).unwrap();

    // Embed encodes as a paragraph wrapping a wikiLink marker with
    // data.embed = true plus `ztl_block_embed` on the carrier paragraph.
    let para = &mdast["children"][0];
    assert_eq!(para["type"], "paragraph");
    assert_eq!(para["ztl_block_embed"], true);
    let inner = &para["children"][0];
    assert_eq!(inner["type"], "wikiLink");
    assert_eq!(inner["value"], "Daily");
    assert_eq!(inner["data"]["embed"], true);
    assert_eq!(inner["data"]["heading"], "Morning");
    assert_eq!(inner["data"]["block_id"], "abc123");

    let back = t.foreign_to_ztl(mdast).unwrap();
    assert_eq!(back, input);
}

// ── TEST-3308 fixture: SPL block with trailing whitespace ────────────────

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

    let t = MdastTranslator;
    let mdast = t.ztl_to_foreign(&input).unwrap();
    let node = &mdast["children"][0];
    assert_eq!(node["type"], "code");
    assert_eq!(node["lang"], "spl");
    assert_eq!(node["meta"], "extra");
    assert_eq!(node["value"], spl_text);
    assert_eq!(node["ztl_spl"], true);

    let back = t.foreign_to_ztl(mdast).unwrap();
    assert_eq!(back, input);
    // And specifically that the trailing whitespace survived.
    if let Block::SplBlock(s) = &back.children[0] {
        assert_eq!(s.text, spl_text);
    } else {
        panic!("expected SplBlock after round-trip");
    }
}

// ── TEST-3308 fixture: Frontmatter with nested structures ────────────────

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

    let t = MdastTranslator;
    let mdast = t.ztl_to_foreign(&input).unwrap();

    // Frontmatter lands on the mdast root as an object so downstream
    // consumers don't need to re-parse YAML.
    let fm_out = &mdast["frontmatter"];
    assert_eq!(fm_out["title"], "Q2");
    assert_eq!(fm_out["tags"][0], "a");
    assert_eq!(fm_out["author"]["aliases"][1], "A.");
    assert_eq!(fm_out["metrics"]["subsections"][1]["heading"], "Body");

    let back = t.foreign_to_ztl(mdast).unwrap();
    assert_eq!(back, input);
}

// ── TEST-3308 fixture: Position fidelity across translation ──────────────

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

    let t = MdastTranslator;
    let mdast = t.ztl_to_foreign(&input).unwrap();

    // mdast native `position` carries {start.line, start.column,
    // end.line, end.column}. Spot-check a couple of nodes.
    let hd = &mdast["children"][0];
    assert_eq!(hd["type"], "heading");
    assert_eq!(hd["position"]["start"]["line"], 1);
    assert_eq!(hd["position"]["end"]["line"], 1);
    assert_eq!(hd["position"]["end"]["column"], 9);

    let wl = &mdast["children"][1]["children"][1];
    assert_eq!(wl["position"]["start"]["line"], 3);
    assert_eq!(wl["position"]["start"]["column"], 7);
    assert_eq!(wl["position"]["end"]["column"], 20);

    let back = t.foreign_to_ztl(mdast).unwrap();
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

// ── mdast extension attrs preserved across round-trip ────────────────────

/// The four marker attrs that ztl stores under the `wikiLink` marker's
/// `data` object (`alias`, `heading`, `block_id`, `embed`) are what the
/// remark-wiki-link plugin convention names as the wiki-link extension's
/// own data schema. This test pins the acceptance criterion "mdast
/// extension attrs preserved across round-trip" — a ztl-ext document
/// carrying every combination round-trips through the mdast adapter
/// without any attr going missing.
#[test]
fn mdast_wikilink_extension_attrs_all_survive_round_trip() {
    let input = doc(vec![paragraph(vec![
        // Bare wikilink (no extensions).
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

    let t = MdastTranslator;
    let mdast = t.ztl_to_foreign(&input).unwrap();

    // Verify every wikilink kept its data-map shape on the wire.
    let kids = &mdast["children"][0]["children"];
    assert_eq!(kids[0]["data"]["alias"], Value::Null);
    assert_eq!(kids[1]["data"]["alias"], "display");
    assert_eq!(kids[2]["data"]["heading"], "Section");
    assert_eq!(kids[3]["data"]["block_id"], "blk42");
    assert_eq!(kids[4]["data"]["alias"], "x");
    assert_eq!(kids[4]["data"]["heading"], "y");
    assert_eq!(kids[4]["data"]["block_id"], "z");

    let back = t.foreign_to_ztl(mdast).unwrap();
    assert_eq!(back, input);
}

/// Embed carries a `data.embed: true` marker plus optional `heading` /
/// `block_id` extension attrs. The paragraph carrier tags itself with
/// `ztl_block_embed: true`. This test pins that the block-level embed
/// marker round-trips regardless of which combination of optional attrs
/// is present.
#[test]
fn mdast_embed_extension_attrs_all_survive_round_trip() {
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

    let t = MdastTranslator;
    let mdast = t.ztl_to_foreign(&input).unwrap();

    for i in 0..4 {
        let para = &mdast["children"][i];
        assert_eq!(para["type"], "paragraph");
        assert_eq!(para["ztl_block_embed"], true);
        let inner = &para["children"][0];
        assert_eq!(inner["type"], "wikiLink");
        assert_eq!(inner["data"]["embed"], true);
    }

    let back = t.foreign_to_ztl(mdast).unwrap();
    assert_eq!(back, input);
}

// ── Slimmed-model marker-table sanity ────────────────────────────────────

/// REQ-3308 slimmed model: the `mdast-ext` translator emits a `root`
/// node (not a ztl `Document` tag) and explicitly carries the
/// `ztl_ast_version` attr so lossy foreign hooks can still be
/// round-tripped through `foreign_to_ztl` without losing the schema
/// pin.
#[test]
fn mdast_root_carries_ztl_ast_version_marker() {
    let input = doc(vec![paragraph(vec![text("x")])]);
    let t = MdastTranslator;
    let mdast = t.ztl_to_foreign(&input).unwrap();
    assert_eq!(mdast["type"], "root");
    assert_eq!(mdast["ztl_ast_version"], AST_VERSION);
}

/// Per REQ-3308, the translator declares `AstType::MdastExt` — a
/// regression floor: renaming the ast_type would break every
/// composition-level manifest that declares `ast_type = "mdast-ext"`.
#[test]
fn translator_reports_mdast_ext_ast_type() {
    assert_eq!(MdastTranslator.ast_type(), AstType::MdastExt);
}

// ── List + ListItem under mdast shape (slimmed-model completeness) ───────

/// A nested ordered list inside a bullet list — a common CommonMark
/// fixture that tests the translator's `listItem`-as-block-container
/// handling (ztl-ext permits any Block child; mdast's `listItem` does
/// too, so this is 1-to-1 renaming, but a regression would show as a
/// round-trip mismatch on nested children).
#[test]
fn nested_mixed_lists_round_trip() {
    use ztl::hooks::ast::List;
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

    let t = MdastTranslator;
    let mdast = t.ztl_to_foreign(&input).unwrap();
    let outer = &mdast["children"][0];
    assert_eq!(outer["type"], "list");
    assert_eq!(outer["ordered"], false);
    let inner = &outer["children"][1]["children"][1];
    assert_eq!(inner["type"], "list");
    assert_eq!(inner["ordered"], true);
    assert_eq!(inner["start"], 1);

    let back = t.foreign_to_ztl(mdast).unwrap();
    assert_eq!(back, input);
}
