//! Project-owned mark types for the Peritext CRDT engine.
//!
//! The `Mark` / `Scalar` / `ExpandMark` types here replace automerge's borrowed
//! `automerge::marks::Mark<'_>`, `automerge::ScalarValue`, and
//! `automerge::marks::ExpandMark` so that alternative backends (diamond-types)
//! can satisfy the [`crate::crdt::CrdtBackend`] trait without leaking
//! third-party types.
//!
//! `Scalar` is intentionally the narrow subset of automerge scalar values
//! zetl ever stored on a mark (bool / string, plus reserved int/null for
//! forward compatibility) — not counters, timestamps, or bytes. This keeps
//! (de)serialisation cheap and the wire format wire-stable.
//!
//! `Scalar::Bool(true)` decodes cleanly from WAL entries written by the
//! automerge backend, so `.zetl/wal/` payloads stay readable across the
//! backend cut-over.

use serde::{Deserialize, Serialize};

/// Peritext expand behaviour for a mark span.
///
/// - `Both`: text inserted at either boundary inherits the mark (bold, italic,
///   strikethrough, highlight).
/// - `None`: text inserted at either boundary does NOT inherit the mark (code,
///   wikilink, link, comment).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpandMark {
    Both,
    None,
}

/// Project-owned scalar value carried on a mark.
///
/// Kept to the subset zetl actually stores so (de)serialisation is cheap and
/// wire-stable. `Int` / `Null` are reserved for future mark types (e.g. a
/// severity-carrying callout mark) so adding them doesn't require a wire bump.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Scalar {
    Bool(bool),
    Str(String),
    Int(i64),
    Null,
}

impl Scalar {
    /// Construct a `Scalar` from a `serde_json::Value`, matching the
    /// wire-level encoding used by `OpEntry::Mark.value`.
    ///
    /// Unknown / non-scalar JSON values collapse to `Scalar::Bool(true)` to
    /// match the automerge backend's historical behaviour.
    pub fn from_json(v: &serde_json::Value) -> Self {
        match v {
            serde_json::Value::Bool(b) => Self::Bool(*b),
            serde_json::Value::String(s) => Self::Str(s.clone()),
            serde_json::Value::Null => Self::Null,
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Self::Int(i)
                } else {
                    // Fall back to the presence-sentinel rather than invent a
                    // lossy i64 from a float; no MarkType today emits floats.
                    Self::Bool(true)
                }
            }
            _ => Self::Bool(true),
        }
    }
}

/// Project-owned CRDT mark span.
///
/// Returned from [`crate::crdt::CrdtBackend::marks`] instead of
/// `automerge::marks::Mark<'_>` so alternative backends (e.g. diamond-types)
/// can satisfy the trait without leaking automerge's borrowed mark type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    pub name: String,
    pub value: Scalar,
    pub start: usize,
    pub end: usize,
}

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
    /// The mark name used as the key in the CRDT.
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

    /// The scalar value stored on the CRDT for this mark.
    pub fn scalar_value(&self) -> Scalar {
        match self {
            Self::Bold | Self::Italic | Self::Code | Self::Strikethrough | Self::Highlight => {
                Scalar::Bool(true)
            }
            Self::Wikilink { target, alias } => match alias {
                Some(a) => Scalar::Str(format!("{target}|{a}")),
                None => Scalar::Str(target.clone()),
            },
            Self::Link { url } => Scalar::Str(url.clone()),
            Self::Comment => Scalar::Bool(true),
        }
    }

    /// Reconstruct a MarkType from a mark name and scalar value.
    pub fn from_mark(name: &str, value: &Scalar) -> Option<Self> {
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

fn scalar_to_string(v: &Scalar) -> Option<String> {
    match v {
        Scalar::Str(s) => Some(s.clone()),
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

    #[test]
    fn scalar_from_json_roundtrip_wire_stable() {
        // WAL entries written by the automerge backend encode mark values as
        // JSON via serde_json::Value — `Scalar::from_json` must decode them
        // to the same `Scalar` that a fresh MarkType would produce.
        assert_eq!(
            Scalar::from_json(&serde_json::json!(true)),
            Scalar::Bool(true)
        );
        assert_eq!(
            Scalar::from_json(&serde_json::json!("Project X")),
            Scalar::Str("Project X".into())
        );
        assert_eq!(Scalar::from_json(&serde_json::json!(42)), Scalar::Int(42));
        assert_eq!(Scalar::from_json(&serde_json::json!(null)), Scalar::Null);
    }
}
