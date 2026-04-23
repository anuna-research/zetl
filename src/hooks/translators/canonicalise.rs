//! Per-ast_type canonical-form normaliser (SPEC-033 NFR-3305).
//!
//! Strict byte-level AST equality across a round-trip through a
//! foreign AST is not achievable in general: pandoc-types especially
//! admits multiple semantically-equivalent representations of the same
//! content (e.g. `[Str "hello", Space, Str "world"]` vs
//! `[Str "hello world"]`, or `Span [] inlines` wrapping a single
//! element). NFR-3305 therefore states the round-trip property as
//! canonical-form equivalence:
//!
//! ```text
//!   ∀ A ∈ ztl-ext:
//!     canonicalise(foreign_to_ztl(ztl_to_foreign(A))) == canonicalise(A)
//! ```
//!
//! This module defines `canonicalise` as a per-ast_type ztl-ext
//! normaliser. It is applied to *both* sides of the equation, so the
//! foreign AST's representation-level degrees of freedom show up as
//! differences that survive [`Translator::foreign_to_ztl`] but get
//! collapsed by `canonicalise` before comparison.
//!
//! ## Applied transformations
//!
//! | Transformation                           | ztl-ext | mdast-ext | pandoc-ext |
//! | ---------------------------------------- | -------- | --------- | ---------- |
//! | Strip source positions                   |   ✓      |    ✓      |    ✓       |
//! | Merge adjacent `Text` nodes              |   —      |    —      |    ✓       |
//! | Normalise ordered `List.start` missing→1 |   —      |    —      |    ✓       |
//!
//! Text-merging is pandoc-specific: a `[Str "a", Space, Str "b"]`
//! sequence round-trips through our translator as three separate
//! `Text` nodes (`"a"`, `" "`, `"b"`), but a third-party pandoc filter
//! may coalesce them into a single `Str "a b"` token. Both forms are
//! semantically identical, so the canonicaliser merges adjacent
//! `Text` runs inside every inline container. Mdast and ztl-ext have
//! no such ambiguity in v1 — their translators emit one `Text` per
//! ztl-ext Text node.
//!
//! Ordered-list `start` is similarly representation-level: pandoc's
//! `OrderedList` attr triple always carries an integer start number,
//! so ztl's `start: None` is serialised as `1` and comes back as
//! `Some(1)`. Under canonical equivalence `None` and `Some(1)` are
//! the same ordered list.
//!
//! ## Relation to [`crate::hooks::contract::canonicalise`]
//!
//! The contract-module canonicaliser is a single function that strips
//! positions and nothing else; it is the NFR-3305 equivalence the
//! idempotence validator uses (REQ-3224). This module adds per-ast_type
//! transformations on top, so foreign-ecosystem round-trips can use a
//! canonical form tailored to the specific representation-level slack
//! of each ecosystem. The core "strip positions" step is shared.

use crate::hooks::ast::{Block, Document, Inline, ListItem, Position};

use super::AstType;

/// Canonicalise `doc` under the equivalence relation defined for
/// `ast_type`. See the module docs for the per-ast_type table.
pub fn canonicalise(ast_type: AstType, doc: &Document) -> Document {
    let mut out = doc.clone();
    canonicalise_in_place(ast_type, &mut out);
    out
}

fn canonicalise_in_place(ast_type: AstType, doc: &mut Document) {
    doc.position = Position::origin();
    for block in &mut doc.children {
        canonicalise_block(ast_type, block);
    }
}

fn canonicalise_block(ast_type: AstType, block: &mut Block) {
    match block {
        Block::Heading(n) => {
            n.position = Position::origin();
            canonicalise_inlines(ast_type, &mut n.children);
        }
        Block::Paragraph(n) => {
            n.position = Position::origin();
            canonicalise_inlines(ast_type, &mut n.children);
        }
        Block::BlockQuote(n) => {
            n.position = Position::origin();
            for c in &mut n.children {
                canonicalise_block(ast_type, c);
            }
        }
        Block::List(n) => {
            n.position = Position::origin();
            if ast_type_normalises_ordered_start(ast_type) && n.ordered && n.start.is_none() {
                // pandoc's OrderedList attr triple always carries an
                // integer — `None` is not representable on the wire.
                n.start = Some(1);
            }
            for item in &mut n.children {
                canonicalise_list_item(ast_type, item);
            }
        }
        Block::CodeBlock(n) => n.position = Position::origin(),
        Block::ThematicBreak(n) => n.position = Position::origin(),
        Block::HtmlBlock(n) => n.position = Position::origin(),
        Block::SplBlock(n) => n.position = Position::origin(),
        Block::Embed(n) => n.position = Position::origin(),
    }
}

fn canonicalise_list_item(ast_type: AstType, item: &mut ListItem) {
    item.position = Position::origin();
    for c in &mut item.children {
        canonicalise_block(ast_type, c);
    }
}

/// Recursively canonicalise every inline, then (for ast_types whose
/// foreign-form admits adjacent-Text ambiguity) merge adjacent `Text`
/// siblings in place.
fn canonicalise_inlines(ast_type: AstType, inlines: &mut Vec<Inline>) {
    for i in inlines.iter_mut() {
        canonicalise_inline(ast_type, i);
    }
    if ast_type_merges_adjacent_text(ast_type) {
        merge_adjacent_text(inlines);
    }
}

fn canonicalise_inline(ast_type: AstType, inline: &mut Inline) {
    match inline {
        Inline::Text(n) => n.position = Position::origin(),
        Inline::Code(n) => n.position = Position::origin(),
        Inline::LineBreak(n) => n.position = Position::origin(),
        Inline::SoftBreak(n) => n.position = Position::origin(),
        Inline::HtmlInline(n) => n.position = Position::origin(),
        Inline::Wikilink(n) => n.position = Position::origin(),
        Inline::Emphasis(n) => {
            n.position = Position::origin();
            canonicalise_inlines(ast_type, &mut n.children);
        }
        Inline::Strong(n) => {
            n.position = Position::origin();
            canonicalise_inlines(ast_type, &mut n.children);
        }
        Inline::Link(n) => {
            n.position = Position::origin();
            canonicalise_inlines(ast_type, &mut n.children);
        }
        Inline::Image(n) => {
            n.position = Position::origin();
            canonicalise_inlines(ast_type, &mut n.children);
        }
    }
}

/// Whether `ast_type` collapses `[Text "a", Text "b"]` into
/// `[Text "ab"]` under its canonical-form equivalence. Pandoc does
/// (third-party filters freely rewrite `Str`/`Space` runs); mdast and
/// ztl-ext do not (each emits one `Text` per ztl-ext Text node and
/// has no wire-level splitting convention).
fn ast_type_merges_adjacent_text(ast_type: AstType) -> bool {
    matches!(ast_type, AstType::PandocExt)
}

/// Whether `ast_type`'s wire form always carries an ordered-list
/// start integer (so `List.start = None` on an ordered list must
/// canonicalise to `Some(1)`). Pandoc does; mdast represents start
/// as `null | number` natively; ztl-ext is the reference shape.
fn ast_type_normalises_ordered_start(ast_type: AstType) -> bool {
    matches!(ast_type, AstType::PandocExt)
}

fn merge_adjacent_text(inlines: &mut Vec<Inline>) {
    if inlines.len() < 2 {
        return;
    }
    let mut out: Vec<Inline> = Vec::with_capacity(inlines.len());
    for inline in inlines.drain(..) {
        match (out.last_mut(), inline) {
            (Some(Inline::Text(prev)), Inline::Text(next)) => {
                prev.text.push_str(&next.text);
            }
            (_, inline) => out.push(inline),
        }
    }
    *inlines = out;
}

/// Helper for test authors: quickly compare two documents under an
/// ast_type's canonical equivalence relation. Returns `true` iff the
/// canonicalised forms are structurally equal.
pub fn canonical_eq(ast_type: AstType, a: &Document, b: &Document) -> bool {
    canonicalise(ast_type, a) == canonicalise(ast_type, b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::ast::{
        Block, Document, DocumentKind, Inline, Paragraph, Position, Text, AST_VERSION,
    };

    fn doc_with(children: Vec<Block>) -> Document {
        Document {
            ast_version: AST_VERSION.to_string(),
            kind: DocumentKind::Document,
            position: Position::new(5, 1, 10, 80),
            frontmatter: None,
            children,
        }
    }

    fn text(pos: Position, s: &str) -> Inline {
        Inline::Text(Text {
            position: pos,
            text: s.to_string(),
        })
    }

    #[test]
    fn positions_are_stripped_for_every_ast_type() {
        let doc = doc_with(vec![Block::Paragraph(Paragraph {
            position: Position::new(3, 1, 3, 80),
            children: vec![text(Position::new(3, 1, 3, 5), "hi")],
        })]);
        for t in [AstType::ztlExt, AstType::MdastExt, AstType::PandocExt] {
            let c = canonicalise(t, &doc);
            assert_eq!(c.position, Position::origin());
            if let Block::Paragraph(p) = &c.children[0] {
                assert_eq!(p.position, Position::origin());
                if let Inline::Text(t2) = &p.children[0] {
                    assert_eq!(t2.position, Position::origin());
                } else {
                    panic!("expected Text child");
                }
            }
        }
    }

    #[test]
    fn pandoc_canonical_merges_adjacent_text_nodes() {
        // Three consecutive Text nodes — the pandoc canonicalised form
        // collapses them into one; mdast/ztl-ext leave them alone.
        let doc = doc_with(vec![Block::Paragraph(Paragraph {
            position: Position::origin(),
            children: vec![
                text(Position::origin(), "hello"),
                text(Position::origin(), " "),
                text(Position::origin(), "world"),
            ],
        })]);
        let c_pandoc = canonicalise(AstType::PandocExt, &doc);
        if let Block::Paragraph(p) = &c_pandoc.children[0] {
            assert_eq!(p.children.len(), 1);
            if let Inline::Text(t) = &p.children[0] {
                assert_eq!(t.text, "hello world");
            } else {
                panic!("expected single Text");
            }
        }
        let c_mdast = canonicalise(AstType::MdastExt, &doc);
        if let Block::Paragraph(p) = &c_mdast.children[0] {
            assert_eq!(p.children.len(), 3, "mdast canonical must NOT merge Text");
        }
    }

    #[test]
    fn canonical_eq_treats_positionally_different_docs_as_equal() {
        let a = doc_with(vec![Block::Paragraph(Paragraph {
            position: Position::new(5, 1, 5, 5),
            children: vec![text(Position::new(5, 1, 5, 5), "hi")],
        })]);
        let b = doc_with(vec![Block::Paragraph(Paragraph {
            position: Position::new(99, 1, 99, 99),
            children: vec![text(Position::new(99, 9, 99, 11), "hi")],
        })]);
        assert!(canonical_eq(AstType::ztlExt, &a, &b));
        assert!(canonical_eq(AstType::MdastExt, &a, &b));
        assert!(canonical_eq(AstType::PandocExt, &a, &b));
    }

    #[test]
    fn merge_adjacent_text_is_idempotent() {
        let doc = doc_with(vec![Block::Paragraph(Paragraph {
            position: Position::origin(),
            children: vec![
                text(Position::origin(), "a"),
                text(Position::origin(), "b"),
                text(Position::origin(), "c"),
            ],
        })]);
        let once = canonicalise(AstType::PandocExt, &doc);
        let twice = canonicalise(AstType::PandocExt, &once);
        assert_eq!(once, twice);
    }

    #[test]
    fn text_merge_does_not_cross_non_text_siblings() {
        let doc = doc_with(vec![Block::Paragraph(Paragraph {
            position: Position::origin(),
            children: vec![
                text(Position::origin(), "a"),
                Inline::Code(crate::hooks::ast::Code {
                    position: Position::origin(),
                    text: "x".into(),
                }),
                text(Position::origin(), "b"),
                text(Position::origin(), "c"),
            ],
        })]);
        let c = canonicalise(AstType::PandocExt, &doc);
        if let Block::Paragraph(p) = &c.children[0] {
            // ["a", Code, "bc"] — Code interrupts the run.
            assert_eq!(p.children.len(), 3);
            if let Inline::Text(t) = &p.children[2] {
                assert_eq!(t.text, "bc");
            } else {
                panic!("expected merged Text after Code");
            }
        }
    }
}
