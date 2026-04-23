//! Project-owned mark types for the Peritext CRDT engine.
//!
//! The `Mark` / `Scalar` / `ExpandMark` types here form the engine-agnostic
//! surface [`crate::crdt::CrdtBackend`] hands back to callers, so no
//! third-party CRDT types leak into the trait.
//!
//! `Scalar` is intentionally the narrow subset of scalar values ztl ever
//! stores on a mark (bool / string, plus reserved int/null for forward
//! compatibility) — not counters, timestamps, or bytes. This keeps
//! (de)serialisation cheap and the wire format stable.
//!
//! This module also houses the markdown ⇄ (text + marks) parser and
//! serializer shared by every CRDT backend — see [`parse_inline_marks`] and
//! [`serialize_to_markdown`].

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
/// Kept to the subset ztl actually stores so (de)serialisation is cheap and
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
    /// Unknown / non-scalar JSON values collapse to `Scalar::Bool(true)` —
    /// the presence-sentinel semantics the WAL relies on for round-tripping
    /// mark values through prior wire-format revisions.
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
/// Returned from [`crate::crdt::CrdtBackend::marks`] so every backend hands
/// back the same owned struct regardless of its internal storage format.
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

// ── Markdown ⇄ (text + marks) ─────────────────────────────────────────
//
// These helpers are engine-agnostic: `parse_inline_marks` turns a line of
// markdown into plain text + inline-mark ranges without touching the CRDT;
// `serialize_to_markdown` is its inverse, emitting canonical markdown from
// text + marks per REQ-020-027. The diamond backend drives them via
// `CrdtBackend::splice_text` / `mark` / `unmark`.

/// Plain text extracted from a line of markdown along with the inline marks
/// discovered while parsing it.
pub struct ParsedInline {
    pub plain_text: String,
    pub marks: Vec<InlineMark>,
}

/// A single inline mark range mapped onto the plain-text offsets produced by
/// [`parse_inline_marks`]. Both ends are counted in `chars`, matching how
/// the CRDT text indexes positions.
pub struct InlineMark {
    pub start: usize,
    pub end: usize,
    pub mark_type: MarkType,
}

/// Parse inline markdown text, extracting formatting marks and producing
/// the plain text content with mark positions mapped to the plain text.
///
/// Block-level tokens (headings, list markers, code fences, frontmatter)
/// are NOT handled here — see [`BlockToken::parse_line_prefix`]; callers
/// strip the block prefix before passing the remaining inline text to
/// this function.
pub fn parse_inline_marks(text: &str) -> ParsedInline {
    let mut plain = String::with_capacity(text.len());
    let mut marks: Vec<InlineMark> = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    // Stack for tracking open marks
    let mut open_marks: Vec<(MarkType, usize)> = Vec::new(); // (type, start_in_plain)

    while i < len {
        // Wikilinks: [[target]] or [[target|alias]]
        if i + 1 < len && chars[i] == '[' && chars[i + 1] == '[' {
            if let Some((target, alias, end_idx)) = parse_wikilink(&chars, i) {
                let start = plain.chars().count();
                let display = alias.as_deref().unwrap_or(&target);
                plain.push_str(display);
                marks.push(InlineMark {
                    start,
                    end: plain.chars().count(),
                    mark_type: MarkType::Wikilink { target, alias },
                });
                i = end_idx;
                continue;
            }
        }

        // Markdown links: [text](url)
        if chars[i] == '[' {
            if let Some((link_text, url, end_idx)) = parse_md_link(&chars, i) {
                let start = plain.chars().count();
                plain.push_str(&link_text);
                marks.push(InlineMark {
                    start,
                    end: plain.chars().count(),
                    mark_type: MarkType::Link { url },
                });
                i = end_idx;
                continue;
            }
        }

        // Strikethrough: ~~text~~
        if i + 1 < len && chars[i] == '~' && chars[i + 1] == '~' {
            if let Some(close_idx) = find_closing(&chars, i + 2, &['~', '~']) {
                handle_delimited_mark(
                    &chars,
                    i + 2,
                    close_idx,
                    MarkType::Strikethrough,
                    &mut plain,
                    &mut marks,
                );
                i = close_idx + 2;
                continue;
            }
        }

        // Bold: **text**
        if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
            if let Some(close_idx) = find_closing(&chars, i + 2, &['*', '*']) {
                handle_delimited_mark(
                    &chars,
                    i + 2,
                    close_idx,
                    MarkType::Bold,
                    &mut plain,
                    &mut marks,
                );
                i = close_idx + 2;
                continue;
            }
        }

        // Highlight: ==text==
        if i + 1 < len && chars[i] == '=' && chars[i + 1] == '=' {
            if let Some(close_idx) = find_closing(&chars, i + 2, &['=', '=']) {
                handle_delimited_mark(
                    &chars,
                    i + 2,
                    close_idx,
                    MarkType::Highlight,
                    &mut plain,
                    &mut marks,
                );
                i = close_idx + 2;
                continue;
            }
        }

        // Comment: %%text%%
        if i + 1 < len && chars[i] == '%' && chars[i + 1] == '%' {
            if let Some(close_idx) = find_closing(&chars, i + 2, &['%', '%']) {
                handle_delimited_mark(
                    &chars,
                    i + 2,
                    close_idx,
                    MarkType::Comment,
                    &mut plain,
                    &mut marks,
                );
                i = close_idx + 2;
                continue;
            }
        }

        // Italic: *text* (single asterisk, not preceded by *)
        if chars[i] == '*' && (i + 1 >= len || chars[i + 1] != '*') {
            // Check if this is opening or closing
            if let Some(pos) = find_open_mark_idx(&open_marks, "italic") {
                // Closing
                let (_, start) = open_marks.remove(pos);
                marks.push(InlineMark {
                    start,
                    end: plain.chars().count(),
                    mark_type: MarkType::Italic,
                });
                i += 1;
                continue;
            } else {
                // Opening
                open_marks.push((MarkType::Italic, plain.chars().count()));
                i += 1;
                continue;
            }
        }

        // Code: `text`
        if chars[i] == '`' {
            if let Some(close_idx) = find_single_closing(&chars, i + 1, '`') {
                let start = plain.chars().count();
                for &c in &chars[i + 1..close_idx] {
                    plain.push(c);
                }
                marks.push(InlineMark {
                    start,
                    end: plain.chars().count(),
                    mark_type: MarkType::Code,
                });
                i = close_idx + 1;
                continue;
            }
        }

        plain.push(chars[i]);
        i += 1;
    }

    ParsedInline {
        plain_text: plain,
        marks,
    }
}

/// Handle a delimited inline mark (like ~~text~~). Recursively parses the
/// inner content for nested marks.
fn handle_delimited_mark(
    chars: &[char],
    inner_start: usize,
    inner_end: usize,
    mark_type: MarkType,
    plain: &mut String,
    marks: &mut Vec<InlineMark>,
) {
    let inner: String = chars[inner_start..inner_end].iter().collect();
    let outer_start = plain.chars().count();
    let inner_parsed = parse_inline_marks(&inner);
    plain.push_str(&inner_parsed.plain_text);
    let outer_end = plain.chars().count();

    for mut m in inner_parsed.marks {
        m.start += outer_start;
        m.end += outer_start;
        marks.push(m);
    }

    marks.push(InlineMark {
        start: outer_start,
        end: outer_end,
        mark_type,
    });
}

fn find_open_mark_idx(open_marks: &[(MarkType, usize)], name: &str) -> Option<usize> {
    open_marks.iter().rposition(|(mt, _)| mt.name() == name)
}

fn find_closing(chars: &[char], start: usize, delim: &[char]) -> Option<usize> {
    let dlen = delim.len();
    if start + dlen > chars.len() {
        return None;
    }
    let mut i = start;
    while i + dlen <= chars.len() {
        if &chars[i..i + dlen] == delim {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_single_closing(chars: &[char], start: usize, delim: char) -> Option<usize> {
    chars[start..]
        .iter()
        .position(|&c| c == delim)
        .map(|p| start + p)
}

fn parse_wikilink(chars: &[char], start: usize) -> Option<(String, Option<String>, usize)> {
    let mut i = start + 2;
    let mut content = String::new();
    while i + 1 < chars.len() {
        if chars[i] == ']' && chars[i + 1] == ']' {
            if let Some(pipe_pos) = content.find('|') {
                let target = content[..pipe_pos].to_string();
                let alias = content[pipe_pos + 1..].to_string();
                return Some((target, Some(alias), i + 2));
            } else {
                return Some((content, None, i + 2));
            }
        }
        content.push(chars[i]);
        i += 1;
    }
    None
}

fn parse_md_link(chars: &[char], start: usize) -> Option<(String, String, usize)> {
    let mut i = start + 1;
    let mut link_text = String::new();
    while i < chars.len() && chars[i] != ']' {
        link_text.push(chars[i]);
        i += 1;
    }
    if i >= chars.len() || chars[i] != ']' {
        return None;
    }
    i += 1;
    if i >= chars.len() || chars[i] != '(' {
        return None;
    }
    i += 1;
    let mut url = String::new();
    while i < chars.len() && chars[i] != ')' {
        url.push(chars[i]);
        i += 1;
    }
    if i >= chars.len() || chars[i] != ')' {
        return None;
    }
    Some((link_text, url, i + 1))
}

/// Serialize CRDT text + marks to canonical markdown (REQ-020-027).
///
/// Mark nesting order (outermost → innermost):
/// strikethrough > bold > italic > code > highlight
pub fn serialize_to_markdown(text: &str, marks: &[Mark]) -> String {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();

    let mut typed_marks: Vec<(MarkType, usize, usize)> = Vec::new();
    for m in marks {
        if let Some(mt) = MarkType::from_mark(&m.name, &m.value) {
            typed_marks.push((mt, m.start, m.end));
        }
    }

    typed_marks.sort_by(|a, b| {
        a.1.cmp(&b.1)
            .then(a.0.nesting_order().cmp(&b.0.nesting_order()))
    });

    let mut opens: Vec<Vec<usize>> = vec![Vec::new(); len + 1];
    let mut closes: Vec<Vec<usize>> = vec![Vec::new(); len + 1];

    for (idx, (_, start, end)) in typed_marks.iter().enumerate() {
        if *start < len + 1 {
            opens[*start].push(idx);
        }
        if *end <= len {
            closes[*end].push(idx);
        }
    }

    let mut out = String::with_capacity(text.len() * 2);

    for pos in 0..=len {
        // Close marks (innermost closes first)
        let mut closing = closes[pos].clone();
        closing.sort_by(|a, b| {
            typed_marks[*b]
                .0
                .nesting_order()
                .cmp(&typed_marks[*a].0.nesting_order())
        });
        for idx in &closing {
            write_mark_close(&typed_marks[*idx].0, &mut out);
        }

        // Open marks (outermost opens first)
        let mut opening = opens[pos].clone();
        opening.sort_by(|a, b| {
            typed_marks[*a]
                .0
                .nesting_order()
                .cmp(&typed_marks[*b].0.nesting_order())
        });
        for idx in &opening {
            write_mark_open(&typed_marks[*idx].0, &mut out);
        }

        if pos < len {
            out.push(chars[pos]);
        }
    }

    normalize_whitespace(&out)
}

fn write_mark_open(mark: &MarkType, out: &mut String) {
    match mark {
        MarkType::Bold => out.push_str("**"),
        MarkType::Italic => out.push('*'),
        MarkType::Code => out.push('`'),
        MarkType::Strikethrough => out.push_str("~~"),
        MarkType::Highlight => out.push_str("=="),
        MarkType::Comment => out.push_str("%%"),
        MarkType::Wikilink { target, alias } => {
            out.push_str("[[");
            // Aliased: [[target| … and the alias is plain text.
            // Unaliased: [[ … and the target is plain text.
            if alias.is_some() {
                out.push_str(target);
                out.push('|');
            }
        }
        MarkType::Link { .. } => {
            out.push('[');
        }
    }
}

fn write_mark_close(mark: &MarkType, out: &mut String) {
    match mark {
        MarkType::Bold => out.push_str("**"),
        MarkType::Italic => out.push('*'),
        MarkType::Code => out.push('`'),
        MarkType::Strikethrough => out.push_str("~~"),
        MarkType::Highlight => out.push_str("=="),
        MarkType::Comment => out.push_str("%%"),
        MarkType::Wikilink { .. } => {
            out.push_str("]]");
        }
        MarkType::Link { url } => {
            out.push_str("](");
            out.push_str(url);
            out.push(')');
        }
    }
}

/// Normalize whitespace per REQ-020-027: trim trailing space per line,
/// collapse consecutive blank lines, ensure a single trailing newline.
pub fn normalize_whitespace(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();

    let trimmed: Vec<String> = lines.iter().map(|l| l.trim_end().to_string()).collect();

    let mut result = String::with_capacity(text.len());
    let mut prev_blank = false;
    for line in &trimmed {
        let is_blank = line.is_empty();
        if is_blank && prev_blank {
            continue;
        }
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(line);
        prev_blank = is_blank;
    }

    if !result.is_empty() && !result.ends_with('\n') {
        result.push('\n');
    }

    result
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
    fn normalize_whitespace_trims_and_collapses() {
        let text = "line one  \n\n\n\nline two";
        assert_eq!(normalize_whitespace(text), "line one\n\nline two\n");
    }

    #[test]
    fn parse_inline_marks_extracts_bold() {
        let out = parse_inline_marks("hi **bold** there");
        assert_eq!(out.plain_text, "hi bold there");
        let bold = out
            .marks
            .iter()
            .find(|m| m.mark_type == MarkType::Bold)
            .expect("bold mark");
        assert_eq!(bold.start, 3);
        assert_eq!(bold.end, 7);
    }

    #[test]
    fn serialize_to_markdown_round_trips_bold() {
        let marks = vec![Mark {
            name: "bold".into(),
            value: Scalar::Bool(true),
            start: 0,
            end: 4,
        }];
        assert_eq!(serialize_to_markdown("bold", &marks), "**bold**\n");
    }

    #[test]
    fn scalar_from_json_roundtrip_wire_stable() {
        // WAL entries encode mark values as JSON via serde_json::Value —
        // `Scalar::from_json` must decode them to the same `Scalar` that a
        // fresh MarkType would produce.
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
