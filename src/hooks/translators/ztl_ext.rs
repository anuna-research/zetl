//! Identity translator for the `ztl-ext` canonical AST.
//!
//! Implements [`Translator`] for [`AstType::ztlExt`] by round-tripping
//! the document through [`serde_json::to_value`] /
//! [`serde_json::from_value`]. The translation is byte-identical (up to
//! JSON object-key ordering); CON-3221's round-trip invariant therefore
//! holds trivially for this ast_type.

use crate::hooks::ast::Document;

use super::{AstType, TranslationError, Translator};

/// Identity translator for [`AstType::ztlExt`]. The canonical AST
/// shape *is* the wire shape, so both directions are a direct serde
/// conversion.
pub struct ztlExtTranslator;

impl Translator for ztlExtTranslator {
    fn ast_type(&self) -> AstType {
        AstType::ztlExt
    }

    fn ztl_to_foreign(&self, doc: &Document) -> Result<serde_json::Value, TranslationError> {
        serde_json::to_value(doc)
            .map_err(|e| TranslationError::to_foreign(AstType::ztlExt, e.to_string()))
    }

    fn foreign_to_ztl(&self, foreign: serde_json::Value) -> Result<Document, TranslationError> {
        serde_json::from_value(foreign)
            .map_err(|e| TranslationError::from_foreign(AstType::ztlExt, e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::ast::{
        Block, Document, DocumentKind, Heading, Inline, Paragraph, Position, Text, Wikilink,
        AST_VERSION,
    };

    fn sample_doc() -> Document {
        Document {
            ast_version: AST_VERSION.to_string(),
            kind: DocumentKind::Document,
            position: Position::origin(),
            frontmatter: None,
            children: vec![
                Block::Heading(Heading {
                    position: Position::origin(),
                    level: 1,
                    children: vec![Inline::Text(Text {
                        position: Position::origin(),
                        text: "Title".into(),
                    })],
                }),
                Block::Paragraph(Paragraph {
                    position: Position::origin(),
                    children: vec![
                        Inline::Text(Text {
                            position: Position::origin(),
                            text: "see ".into(),
                        }),
                        Inline::Wikilink(Wikilink {
                            position: Position::origin(),
                            target: "Other".into(),
                            alias: Some("alt".into()),
                            heading: None,
                            block_id: None,
                        }),
                    ],
                }),
            ],
        }
    }

    #[test]
    fn ast_type_is_ztl_ext() {
        assert_eq!(ztlExtTranslator.ast_type(), AstType::ztlExt);
    }

    #[test]
    fn ztl_to_foreign_emits_canonical_tag() {
        let t = ztlExtTranslator;
        let v = t.ztl_to_foreign(&sample_doc()).unwrap();
        assert_eq!(v["type"], "Document");
        assert_eq!(v["ast_version"], "1.0");
        assert_eq!(v["children"][0]["type"], "Heading");
        assert_eq!(v["children"][1]["children"][1]["type"], "Wikilink");
    }

    #[test]
    fn round_trip_is_byte_equivalent() {
        let t = ztlExtTranslator;
        let doc = sample_doc();
        let v = t.ztl_to_foreign(&doc).unwrap();
        let back = t.foreign_to_ztl(v).unwrap();
        assert_eq!(back, doc);
    }

    #[test]
    fn malformed_value_surfaces_translation_error() {
        let t = ztlExtTranslator;
        let bogus = serde_json::json!({"type": "Document", "children": "not an array"});
        let err = t.foreign_to_ztl(bogus).unwrap_err();
        assert_eq!(err.ast_type, AstType::ztlExt);
        assert_eq!(
            err.direction,
            super::super::TranslationDirection::ForeignToztl
        );
    }
}
