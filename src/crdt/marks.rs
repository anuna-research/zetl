use automerge::marks::ExpandMark;
use automerge::ScalarValue;

/// Mark types supported by the Peritext CRDT engine (REQ-020-025).
///
/// Each variant maps to a markdown syntax and carries Peritext growth behavior
/// that determines whether text inserted at mark boundaries inherits the mark.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MarkType {
    Bold,
    Italic,
    Code,
    Strikethrough,
    Wikilink {
        target: String,
        alias: Option<String>,
    },
    Link {
        url: String,
    },
    Highlight,
    Comment,
}

impl MarkType {
    /// The automerge mark name used as the key in the CRDT.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Bold => "bold",
            Self::Italic => "italic",
            Self::Code => "code",
            Self::Strikethrough => "strikethrough",
            Self::Wikilink { .. } => "wikilink",
            Self::Link { .. } => "link",
            Self::Highlight => "highlight",
            Self::Comment => "comment",
        }
    }

    /// Peritext growth behavior for this mark type.
    ///
    /// - Inclusive (`Both`): text at boundary inherits formatting
    /// - Non-growing (`None`): text at boundary does NOT inherit
    pub fn expand(&self) -> ExpandMark {
        match self {
            Self::Bold | Self::Italic | Self::Strikethrough | Self::Highlight => ExpandMark::Both,
            Self::Code | Self::Wikilink { .. } | Self::Link { .. } | Self::Comment => {
                ExpandMark::None
            }
        }
    }

    /// Whether this mark type uses inclusive (growing) behavior.
    pub fn is_inclusive(&self) -> bool {
        matches!(self.expand(), ExpandMark::Both)
    }

    /// The scalar value stored in automerge for this mark.
    pub fn scalar_value(&self) -> ScalarValue {
        match self {
            Self::Bold | Self::Italic | Self::Code | Self::Strikethrough | Self::Highlight => {
                ScalarValue::from(true)
            }
            Self::Wikilink { target, alias } => {
                // Encode as "target" or "target|alias"
                match alias {
                    Some(a) => ScalarValue::from(format!("{target}|{a}")),
                    None => ScalarValue::from(target.clone()),
                }
            }
            Self::Link { url } => ScalarValue::from(url.clone()),
            Self::Comment => ScalarValue::from(true),
        }
    }

    /// Reconstruct a MarkType from an automerge mark name and scalar value.
    pub fn from_mark(name: &str, value: &ScalarValue) -> Option<Self> {
        match name {
            "bold" => Some(Self::Bold),
            "italic" => Some(Self::Italic),
            "code" => Some(Self::Code),
            "strikethrough" => Some(Self::Strikethrough),
            "highlight" => Some(Self::Highlight),
            "comment" => Some(Self::Comment),
            "wikilink" => {
                let s = scalar_to_string(value)?;
                if let Some((target, alias)) = s.split_once('|') {
                    Some(Self::Wikilink {
                        target: target.to_string(),
                        alias: Some(alias.to_string()),
                    })
                } else {
                    Some(Self::Wikilink {
                        target: s,
                        alias: None,
                    })
                }
            }
            "link" => {
                let url = scalar_to_string(value)?;
                Some(Self::Link { url })
            }
            _ => None,
        }
    }

    /// Reconstruct a simple (non-parameterized) MarkType from just a name.
    /// Used for `unmark` operations where no value is provided.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "bold" => Some(Self::Bold),
            "italic" => Some(Self::Italic),
            "code" => Some(Self::Code),
            "strikethrough" => Some(Self::Strikethrough),
            "highlight" => Some(Self::Highlight),
            "comment" => Some(Self::Comment),
            _ => None,
        }
    }

    /// Conflict resolution strategy.
    ///
    /// Returns `true` if this mark type uses last-write-wins (exclusive),
    /// `false` if marks coexist (overlay).
    pub fn is_exclusive(&self) -> bool {
        matches!(self, Self::Wikilink { .. } | Self::Link { .. })
    }

    /// Canonical nesting order for serialization (outermost → innermost).
    /// Lower number = more outer. Per REQ-020-027:
    /// strikethrough > bold > italic > code > highlight
    pub fn nesting_order(&self) -> u8 {
        match self {
            Self::Strikethrough => 0,
            Self::Bold => 1,
            Self::Italic => 2,
            Self::Code => 3,
            Self::Highlight => 4,
            Self::Comment => 5,
            Self::Link { .. } => 6,
            Self::Wikilink { .. } => 7,
        }
    }
}

fn scalar_to_string(v: &ScalarValue) -> Option<String> {
    match v {
        ScalarValue::Str(s) => Some(s.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inclusive_marks_have_both_expand() {
        assert_eq!(MarkType::Bold.expand(), ExpandMark::Both);
        assert_eq!(MarkType::Italic.expand(), ExpandMark::Both);
        assert_eq!(MarkType::Strikethrough.expand(), ExpandMark::Both);
        assert_eq!(MarkType::Highlight.expand(), ExpandMark::Both);
    }

    #[test]
    fn non_growing_marks_have_none_expand() {
        assert_eq!(MarkType::Code.expand(), ExpandMark::None);
        assert_eq!(
            MarkType::Wikilink {
                target: "x".into(),
                alias: None
            }
            .expand(),
            ExpandMark::None
        );
        assert_eq!(
            MarkType::Link {
                url: "http://x".into()
            }
            .expand(),
            ExpandMark::None
        );
        assert_eq!(MarkType::Comment.expand(), ExpandMark::None);
    }

    #[test]
    fn round_trip_mark_type() {
        let cases = vec![
            MarkType::Bold,
            MarkType::Italic,
            MarkType::Code,
            MarkType::Strikethrough,
            MarkType::Highlight,
            MarkType::Comment,
            MarkType::Wikilink {
                target: "Project X".into(),
                alias: None,
            },
            MarkType::Wikilink {
                target: "Project X".into(),
                alias: Some("the project".into()),
            },
            MarkType::Link {
                url: "https://example.com".into(),
            },
        ];
        for mt in cases {
            let name = mt.name();
            let value = mt.scalar_value();
            let reconstructed = MarkType::from_mark(name, &value).unwrap();
            assert_eq!(mt, reconstructed);
        }
    }

    #[test]
    fn nesting_order_is_correct() {
        assert!(MarkType::Strikethrough.nesting_order() < MarkType::Bold.nesting_order());
        assert!(MarkType::Bold.nesting_order() < MarkType::Italic.nesting_order());
        assert!(MarkType::Italic.nesting_order() < MarkType::Code.nesting_order());
        assert!(MarkType::Code.nesting_order() < MarkType::Highlight.nesting_order());
    }
}
