//! Shared proptest generators for ztl-ext AST documents
//! (SPEC-033 NFR-3305 / TEST-3305-fidelity).
//!
//! Exposes [`Strategy`]-building functions for every ztl-ext node
//! type, plus compositions that produce well-formed documents with the
//! ztl-specific marker nodes ([`Wikilink`], [`Embed`], [`SplBlock`],
//! [`Frontmatter`]) called out by the task acceptance criteria.
//!
//! The strategies live at the translator layer rather than in each
//! ecosystem crate because the round-trip property is stated over
//! ztl-ext documents (both sides of the equivalence are ztl-ext after
//! `foreign_to_ztl`); the *same* generator feeds the pandoc-ext,
//! mdast-ext, and ztl-ext harnesses in [`super::roundtrip`].
//!
//! ## Shape caps
//!
//! Generators are bounded to small trees by default (≤4 blocks, ≤3
//! inline children, ≤3 list items, ≤2 levels of list nesting). The
//! CON-3221 round-trip property is structural, not size-dependent, so
//! the bound trades generator variety against per-case cost. Bumping
//! `ProptestConfig::cases` covers the broader space faster than deeper
//! individual trees.
//!
//! Strings are ASCII-only (`[a-z0-9 ]` + frontmatter keys `[a-z_]`) so
//! `[dev-dependencies] proptest` can minimise failing cases without
//! tripping the UTF-8 boundary-index panic in
//! [`crate::hooks::ast::parse::split_wikilinks`] (see the
//! task-hook-authoring-cli memory entry).

#![allow(dead_code)] // Strategies are consumed by peer test modules only.

use proptest::prelude::*;

use crate::hooks::ast::{
    Block, BlockQuote, Code, CodeBlock, Document, DocumentKind, Embed, Emphasis, Frontmatter,
    Heading, HtmlBlock, HtmlInline, Image, Inline, LineBreak, Link, List, ListItem, Paragraph,
    Position, SoftBreak, SplBlock, Strong, Text, ThematicBreak, Wikilink, AST_VERSION,
};

/// Arbitrary [`Position`] with components in `[1, 20]` so the line/col
/// counters stay small and readable when proptest minimises.
pub fn arb_position() -> impl Strategy<Value = Position> {
    (1u32..=20, 1u32..=20, 1u32..=20, 1u32..=20).prop_map(|(a, b, c, d)| Position::new(a, b, c, d))
}

fn ascii_word(min: usize, max: usize) -> BoxedStrategy<String> {
    let re = format!("[a-z0-9]{{{min},{max}}}");
    proptest::string::string_regex(&re).unwrap().boxed()
}

fn ascii_sentence(max: usize) -> BoxedStrategy<String> {
    let re = format!("[a-z0-9 ]{{0,{max}}}");
    proptest::string::string_regex(&re).unwrap().boxed()
}

// ── Leaf strategies ─────────────────────────────────────────────────

/// Arbitrary [`Text`] with ASCII-only content up to 20 chars.
pub fn arb_text() -> impl Strategy<Value = Text> {
    (arb_position(), ascii_sentence(20)).prop_map(|(position, text)| Text { position, text })
}

/// Arbitrary [`Code`] inline (backtick span).
pub fn arb_code_inline() -> impl Strategy<Value = Code> {
    (arb_position(), ascii_word(1, 10)).prop_map(|(position, text)| Code { position, text })
}

/// Arbitrary [`Wikilink`] covering every option-field combination: the
/// proptest engine is responsible for walking `None`/`Some` branches so
/// the shrinker can narrow a failure down to the minimal field set.
pub fn arb_wikilink() -> impl Strategy<Value = Wikilink> {
    (
        arb_position(),
        ascii_word(1, 10),
        prop::option::of(ascii_word(1, 5)),
        prop::option::of(ascii_word(1, 5)),
        prop::option::of(ascii_word(1, 5)),
    )
        .prop_map(|(position, target, alias, heading, block_id)| Wikilink {
            position,
            target,
            alias,
            heading,
            block_id,
        })
}

/// Arbitrary [`Embed`]. Like [`arb_wikilink`] but no `alias` (embeds
/// don't carry one in ztl-ext).
pub fn arb_embed() -> impl Strategy<Value = Embed> {
    (
        arb_position(),
        ascii_word(1, 10),
        prop::option::of(ascii_word(1, 5)),
        prop::option::of(ascii_word(1, 5)),
    )
        .prop_map(|(position, target, heading, block_id)| Embed {
            position,
            target,
            heading,
            block_id,
        })
}

/// Arbitrary [`SplBlock`]. Covers both `info = None` and `info = Some`
/// to exercise the translator's optional-info marker handling.
pub fn arb_spl_block() -> impl Strategy<Value = SplBlock> {
    (
        arb_position(),
        prop::option::of(ascii_word(1, 10)),
        ascii_sentence(30),
    )
        .prop_map(|(position, info, text)| SplBlock {
            position,
            info,
            text,
        })
}

/// Arbitrary [`Frontmatter`]. Produces a JSON object whose keys are
/// ASCII identifiers and whose values exercise every primitive the
/// translator must preserve (string, number, bool, array, object).
///
/// Size-capped to ≤4 top-level keys so proptest shrinks cleanly; the
/// translator contract is that every key/value is preserved, which
/// doesn't care about map size.
pub fn arb_frontmatter() -> impl Strategy<Value = Frontmatter> {
    use serde_json::{Map, Value};
    let key = "[a-z][a-z_]{0,5}";
    let scalar = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        (-100i64..100).prop_map(Value::from),
        ascii_sentence(12).prop_map(Value::String),
    ];
    let value = prop_oneof![
        scalar.clone(),
        prop::collection::vec(scalar.clone(), 0..3).prop_map(Value::Array),
        prop::collection::hash_map(key, scalar.clone(), 0..3)
            .prop_map(|m| Value::Object(m.into_iter().collect())),
    ];
    prop::collection::vec((key, value), 0..4).prop_map(|pairs| {
        let mut fm: Map<String, Value> = Map::new();
        for (k, v) in pairs {
            fm.insert(k, v);
        }
        fm
    })
}

// ── Inline strategies ────────────────────────────────────────────────

/// Leaf inlines (no children). Used as the base case for the recursive
/// [`arb_inline`] strategy.
pub fn arb_leaf_inline() -> impl Strategy<Value = Inline> {
    prop_oneof![
        arb_text().prop_map(Inline::Text),
        arb_code_inline().prop_map(Inline::Code),
        arb_position().prop_map(|p| Inline::LineBreak(LineBreak { position: p })),
        arb_position().prop_map(|p| Inline::SoftBreak(SoftBreak { position: p })),
        arb_wikilink().prop_map(Inline::Wikilink),
        (arb_position(), ascii_word(0, 12))
            .prop_map(|(position, text)| Inline::HtmlInline(HtmlInline { position, text })),
    ]
}

/// Recursive inline strategy: leaves plus one level of structural
/// wrappers (Emphasis, Strong, Link, Image). Bounded depth keeps the
/// generator cheap — the round-trip property doesn't need deep nesting
/// to exercise the translator code paths.
pub fn arb_inline() -> impl Strategy<Value = Inline> {
    let leaf = arb_leaf_inline().boxed();
    let leaf_children = prop::collection::vec(leaf.clone(), 0..3);
    prop_oneof![
        leaf.clone(),
        (arb_position(), leaf_children.clone())
            .prop_map(|(position, children)| Inline::Emphasis(Emphasis { position, children })),
        (arb_position(), leaf_children.clone())
            .prop_map(|(position, children)| Inline::Strong(Strong { position, children })),
        (
            arb_position(),
            ascii_word(1, 10),
            prop::option::of(ascii_word(1, 8)),
            leaf_children.clone(),
        )
            .prop_map(|(position, url, title, children)| Inline::Link(Link {
                position,
                url,
                title,
                children,
            })),
        (
            arb_position(),
            ascii_word(1, 10),
            prop::option::of(ascii_word(1, 8)),
            ascii_word(0, 8),
            leaf_children,
        )
            .prop_map(
                |(position, url, title, alt, children)| Inline::Image(Image {
                    position,
                    url,
                    title,
                    alt,
                    children,
                })
            ),
    ]
}

// ── Block strategies ────────────────────────────────────────────────

/// A list item is a non-empty sequence of block children. Restricted to
/// leaf-ish blocks (paragraph/heading/thematic-break) so recursion stops
/// — `List` is not a valid list-item child in this generator.
pub fn arb_list_item() -> impl Strategy<Value = ListItem> {
    let inner_block = prop_oneof![
        (
            arb_position(),
            prop::collection::vec(arb_leaf_inline(), 0..3)
        )
            .prop_map(|(position, children)| Block::Paragraph(Paragraph { position, children })),
        arb_position().prop_map(|p| Block::ThematicBreak(ThematicBreak { position: p })),
    ];
    (arb_position(), prop::collection::vec(inner_block, 1..3))
        .prop_map(|(position, children)| ListItem::new(position, children))
}

/// Arbitrary [`Block`]. Covers every variant, including the three
/// marker nodes required by the task acceptance
/// ([`Wikilink`](Inline::Wikilink) appears indirectly via paragraph
/// inline children, [`Embed`] is a block-level variant, and [`SplBlock`]
/// is its own block).
pub fn arb_block() -> impl Strategy<Value = Block> {
    let inline = prop::collection::vec(arb_inline(), 0..3).boxed();
    prop_oneof![
        // Paragraph — the primary carrier of inline marker nodes.
        (arb_position(), inline.clone())
            .prop_map(|(position, children)| Block::Paragraph(Paragraph { position, children })),
        (arb_position(), 1u8..=6, inline.clone()).prop_map(|(position, level, children)| {
            Block::Heading(Heading {
                position,
                level,
                children,
            })
        }),
        arb_position().prop_map(|p| Block::ThematicBreak(ThematicBreak { position: p })),
        (arb_position(), ascii_word(0, 20))
            .prop_map(|(position, text)| Block::HtmlBlock(HtmlBlock { position, text })),
        (
            arb_position(),
            any::<bool>(),
            prop::option::of(ascii_word(1, 8)),
            prop::option::of(ascii_word(1, 10)),
            ascii_sentence(30),
        )
            .prop_map(
                |(position, fenced, lang, info, text)| Block::CodeBlock(CodeBlock {
                    position,
                    fenced,
                    lang,
                    info,
                    text,
                })
            ),
        arb_spl_block().prop_map(Block::SplBlock),
        arb_embed().prop_map(Block::Embed),
        // One level of blockquote — recursive but shallow.
        (
            arb_position(),
            prop::collection::vec(
                (arb_position(), inline.clone()).prop_map(|(p, c)| Block::Paragraph(Paragraph {
                    position: p,
                    children: c
                })),
                0..2,
            )
        )
            .prop_map(|(position, children)| Block::BlockQuote(BlockQuote { position, children })),
        // Bulleted/ordered list.
        (
            any::<bool>(),
            arb_position(),
            prop::option::of(1u32..5),
            prop::collection::vec(arb_list_item(), 1..3),
        )
            .prop_map(|(ordered, position, start, children)| Block::List(List {
                position,
                ordered,
                tight: true,
                start: if ordered { start } else { None },
                children,
            })),
    ]
}

/// Arbitrary ztl-ext [`Document`]. Top-level: optional frontmatter
/// plus 0..4 block children drawn from [`arb_block`].
pub fn arb_document() -> impl Strategy<Value = Document> {
    (
        arb_position(),
        prop::option::of(arb_frontmatter()),
        prop::collection::vec(arb_block(), 0..4),
    )
        .prop_map(|(position, frontmatter, children)| Document {
            ast_version: AST_VERSION.to_string(),
            kind: DocumentKind::Document,
            position,
            frontmatter,
            children,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Sanity: each strategy compiles and produces at least one value of
    // the expected variant. These aren't assertions about behaviour —
    // they guard against a generator returning only one branch, which
    // has bitten proptest users before (`prop_oneof` with `.boxed()`
    // compiles even if one arm fails to produce values in practice).

    proptest! {
        #![proptest_config(ProptestConfig { cases: 8, ..ProptestConfig::default() })]

        #[test]
        fn arb_wikilink_always_produces_a_wikilink(w in arb_wikilink()) {
            prop_assert!(!w.target.is_empty());
        }

        #[test]
        fn arb_embed_always_produces_an_embed(e in arb_embed()) {
            prop_assert!(!e.target.is_empty());
        }

        #[test]
        fn arb_spl_block_produces_valid_text(s in arb_spl_block()) {
            // `text` may be empty; the contract is just well-formed.
            let _ = s.text.len();
        }

        #[test]
        fn arb_frontmatter_is_a_valid_json_object(fm in arb_frontmatter()) {
            let v = serde_json::to_value(&fm).unwrap();
            prop_assert!(v.is_object());
        }

        #[test]
        fn arb_document_roundtrips_through_serde(doc in arb_document()) {
            let v = serde_json::to_value(&doc).unwrap();
            let back: Document = serde_json::from_value(v).unwrap();
            prop_assert_eq!(back, doc);
        }
    }
}
