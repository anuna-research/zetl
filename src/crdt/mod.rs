//! Peritext CRDT engine for collaborative editing (REQ-020-024 through REQ-020-027).
//!
//! Integrates `automerge` RichText with Peritext mark types mapped to/from
//! markdown syntax. Block-level structure is handled as indivisible atomic
//! tokens; inline formatting uses Peritext growth behavior.

pub mod blocks;
pub mod marks;

use anyhow::{Context, Result};
use automerge::marks::{ExpandMark, Mark};
use automerge::transaction::Transactable;
use automerge::{AutoCommit, ObjType, ReadDoc, ROOT};

use blocks::BlockToken;
use marks::MarkType;

/// A CRDT document wrapping an automerge `AutoCommit` with Peritext-style
/// rich text. The text object lives at `root.content` as an automerge Text.
pub struct CrdtDocument {
    doc: AutoCommit,
    text_id: automerge::ObjId,
}

impl CrdtDocument {
    /// Create a new empty CRDT document.
    pub fn new() -> Result<Self> {
        let mut doc = AutoCommit::new();
        let text_id = doc
            .put_object(ROOT, "content", ObjType::Text)
            .context("failed to create text object")?;
        Ok(Self {
            text_id: text_id.into(),
            doc,
        })
    }

    /// Load a CRDT document from markdown text. Parses the markdown into
    /// the automerge text object with appropriate marks (REQ-020-024 load flow).
    pub fn from_markdown(markdown: &str) -> Result<Self> {
        let mut crdt = Self::new()?;
        crdt.load_markdown(markdown)?;
        Ok(crdt)
    }

    /// Load markdown content into this (empty) document.
    fn load_markdown(&mut self, markdown: &str) -> Result<()> {
        let lines: Vec<&str> = markdown.lines().collect();
        let mut pos: usize = 0;
        let mut in_code_fence = false;
        let mut in_frontmatter = false;
        let mut is_first_line = true;

        for (line_idx, line) in lines.iter().enumerate() {
            // Add newline between lines (not before the first)
            if line_idx > 0 {
                self.doc
                    .splice_text(&self.text_id, pos, 0, "\n")
                    .context("splice newline")?;
                pos += 1;
            }

            // Check for frontmatter boundaries
            if *line == "---" {
                if is_first_line || in_frontmatter {
                    // Frontmatter boundary — insert as atomic token
                    self.doc
                        .splice_text(&self.text_id, pos, 0, line)
                        .context("splice frontmatter")?;
                    pos += line.len();
                    in_frontmatter = is_first_line;
                    is_first_line = false;
                    continue;
                }
            }
            is_first_line = false;

            // Inside frontmatter: plain text, no marks
            if in_frontmatter {
                self.doc
                    .splice_text(&self.text_id, pos, 0, line)
                    .context("splice frontmatter content")?;
                pos += line.len();
                continue;
            }

            // Check for code fence boundaries
            if line.starts_with("```") {
                self.doc
                    .splice_text(&self.text_id, pos, 0, line)
                    .context("splice code fence")?;
                pos += line.len();
                in_code_fence = !in_code_fence;
                continue;
            }

            // Inside code fence: plain text, no marks
            if in_code_fence {
                self.doc
                    .splice_text(&self.text_id, pos, 0, line)
                    .context("splice code content")?;
                pos += line.len();
                continue;
            }

            // Empty line
            if line.is_empty() {
                continue;
            }

            // Check for block-level prefix (heading, list marker)
            if let Some((block_token, content)) = BlockToken::parse_line_prefix(line) {
                let prefix = block_token.to_markdown();
                // Insert the atomic block prefix
                self.doc
                    .splice_text(&self.text_id, pos, 0, &prefix)
                    .context("splice block prefix")?;
                pos += prefix.len();

                // Insert inline content with formatting marks
                pos = self.insert_inline_with_marks(content, pos)?;
            } else {
                // Plain paragraph — insert inline content with formatting marks
                pos = self.insert_inline_with_marks(line, pos)?;
            }
        }

        // Ensure trailing newline
        let text = self.text()?;
        if !text.is_empty() && !text.ends_with('\n') {
            self.doc
                .splice_text(&self.text_id, pos, 0, "\n")
                .context("splice trailing newline")?;
        }

        Ok(())
    }

    /// Insert inline text, detecting and applying markdown formatting marks.
    /// Returns the new position after insertion.
    fn insert_inline_with_marks(&mut self, text: &str, start_pos: usize) -> Result<usize> {
        // First pass: find all inline marks and their positions in the source text
        let parsed = parse_inline_marks(text);

        // Insert the plain text (marks stripped)
        self.doc
            .splice_text(&self.text_id, start_pos, 0, &parsed.plain_text)
            .context("splice inline text")?;

        // Apply marks
        for inline_mark in &parsed.marks {
            let mark_start = start_pos + inline_mark.start;
            let mark_end = start_pos + inline_mark.end;
            if mark_start < mark_end {
                self.doc
                    .mark(
                        &self.text_id,
                        Mark::new(
                            inline_mark.mark_type.name().to_string(),
                            inline_mark.mark_type.scalar_value(),
                            mark_start,
                            mark_end,
                        ),
                        inline_mark.mark_type.expand(),
                    )
                    .context("apply mark")?;
            }
        }

        Ok(start_pos + parsed.plain_text.len())
    }

    /// Get the raw text content of the document.
    pub fn text(&self) -> Result<String> {
        self.doc.text(&self.text_id).context("read text")
    }

    /// Get all marks on the document's text object.
    pub fn marks(&self) -> Result<Vec<Mark<'_>>> {
        self.doc.marks(&self.text_id).context("read marks")
    }

    /// Insert text at a position.
    pub fn splice_text(&mut self, pos: usize, del: isize, text: &str) -> Result<()> {
        self.doc
            .splice_text(&self.text_id, pos, del, text)
            .context("splice_text")
    }

    /// Apply a mark to a range.
    pub fn mark(&mut self, mark_type: &MarkType, start: usize, end: usize) -> Result<()> {
        self.doc
            .mark(
                &self.text_id,
                Mark::new(
                    mark_type.name().to_string(),
                    mark_type.scalar_value(),
                    start,
                    end,
                ),
                mark_type.expand(),
            )
            .context("mark")
    }

    /// Remove a mark from a range.
    pub fn unmark(&mut self, mark_type: &MarkType, start: usize, end: usize) -> Result<()> {
        self.doc
            .unmark(
                &self.text_id,
                mark_type.name(),
                start,
                end,
                mark_type.expand(),
            )
            .context("unmark")
    }

    /// Serialize the CRDT state to canonical markdown (REQ-020-027).
    pub fn to_markdown(&self) -> Result<String> {
        let text = self.text()?;
        let marks = self.marks()?;

        if text.is_empty() {
            return Ok(String::new());
        }

        serialize_to_markdown(&text, &marks)
    }

    /// Save the automerge document to bytes.
    pub fn save(&mut self) -> Vec<u8> {
        self.doc.save()
    }

    /// Load a CRDT document from saved automerge bytes.
    pub fn load(data: &[u8]) -> Result<Self> {
        let doc = AutoCommit::load(data).context("load automerge doc")?;
        // Find the text object at root.content
        let text_id = doc
            .get(ROOT, "content")
            .context("get content")?
            .map(|(_, id)| id.into())
            .context("content not found")?;
        Ok(Self { doc, text_id })
    }

    /// Fork the document for concurrent editing simulation.
    pub fn fork(&mut self) -> Self {
        let forked = self.doc.fork();
        Self {
            doc: forked,
            text_id: self.text_id.clone(),
        }
    }

    /// Merge another document's changes into this one.
    pub fn merge(&mut self, other: &mut Self) -> Result<()> {
        self.doc.merge(&mut other.doc).context("merge")?;
        Ok(())
    }
}

/// Parsed inline content: plain text with mark annotations.
struct ParsedInline {
    plain_text: String,
    marks: Vec<InlineMark>,
}

struct InlineMark {
    start: usize,
    end: usize,
    mark_type: MarkType,
}

/// Parse inline markdown text, extracting formatting marks and producing
/// the plain text content with mark positions mapped to the plain text.
fn parse_inline_marks(text: &str) -> ParsedInline {
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
                let start = plain.len();
                let display = alias.as_deref().unwrap_or(&target);
                plain.push_str(display);
                marks.push(InlineMark {
                    start,
                    end: plain.len(),
                    mark_type: MarkType::Wikilink { target, alias },
                });
                i = end_idx;
                continue;
            }
        }

        // Markdown links: [text](url)
        if chars[i] == '[' {
            if let Some((link_text, url, end_idx)) = parse_md_link(&chars, i) {
                let start = plain.len();
                plain.push_str(&link_text);
                marks.push(InlineMark {
                    start,
                    end: plain.len(),
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
                    2,
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
                    2,
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
                    2,
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
                    2,
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
                    end: plain.len(),
                    mark_type: MarkType::Italic,
                });
                i += 1;
                continue;
            } else {
                // Opening
                open_marks.push((MarkType::Italic, plain.len()));
                i += 1;
                continue;
            }
        }

        // Code: `text`
        if chars[i] == '`' {
            if let Some(close_idx) = find_single_closing(&chars, i + 1, '`') {
                let start = plain.len();
                for &c in &chars[i + 1..close_idx] {
                    plain.push(c);
                }
                marks.push(InlineMark {
                    start,
                    end: plain.len(),
                    mark_type: MarkType::Code,
                });
                i = close_idx + 1;
                continue;
            }
        }

        plain.push(chars[i]);
        i += 1;
    }

    // Any unclosed italic marks — treat the * as literal
    // We need to re-insert the asterisks. This is a rare edge case.
    // For now, leave the text as-is (the asterisks were consumed).

    ParsedInline {
        plain_text: plain,
        marks,
    }
}

/// Handle a delimited inline mark (like ~~text~~). Recursively parses
/// the inner content for nested marks.
fn handle_delimited_mark(
    chars: &[char],
    inner_start: usize,
    inner_end: usize,
    _delim_len: usize,
    mark_type: MarkType,
    plain: &mut String,
    marks: &mut Vec<InlineMark>,
) {
    let inner: String = chars[inner_start..inner_end].iter().collect();
    let outer_start = plain.len();
    let inner_parsed = parse_inline_marks(&inner);
    plain.push_str(&inner_parsed.plain_text);
    let outer_end = plain.len();

    // Offset inner marks
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
    open_marks
        .iter()
        .rposition(|(mt, _)| mt.name() == name)
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
    chars[start..].iter().position(|&c| c == delim).map(|p| start + p)
}

fn parse_wikilink(chars: &[char], start: usize) -> Option<(String, Option<String>, usize)> {
    // start points to first [
    // Find closing ]]
    let mut i = start + 2;
    let mut content = String::new();
    while i + 1 < chars.len() {
        if chars[i] == ']' && chars[i + 1] == ']' {
            // Parse target|alias
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
    // [text](url)
    let mut i = start + 1;
    let mut link_text = String::new();
    // Find closing ]
    while i < chars.len() && chars[i] != ']' {
        link_text.push(chars[i]);
        i += 1;
    }
    if i >= chars.len() || chars[i] != ']' {
        return None;
    }
    i += 1; // skip ]
    if i >= chars.len() || chars[i] != '(' {
        return None;
    }
    i += 1; // skip (
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
fn serialize_to_markdown(text: &str, marks: &[Mark<'_>]) -> Result<String> {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();

    // Build a list of active mark types at each character position
    let mut typed_marks: Vec<(MarkType, usize, usize)> = Vec::new();
    for m in marks {
        if let Some(mt) = MarkType::from_mark(m.name(), m.value()) {
            typed_marks.push((mt, m.start, m.end));
        }
    }

    // Sort by start position, then by nesting order (outermost first)
    typed_marks.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.nesting_order().cmp(&b.0.nesting_order())));

    // Build events: mark-open and mark-close at each position
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
        // Close marks (in reverse nesting order — innermost closes first)
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

        // Open marks (in nesting order — outermost opens first)
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

        // Write the character
        if pos < len {
            out.push(chars[pos]);
        }
    }

    // Normalize whitespace per REQ-020-027:
    // - single blank line between blocks
    // - no trailing whitespace
    // - trailing newline at EOF
    let normalized = normalize_whitespace(&out);
    Ok(normalized)
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
            // For aliased wikilinks, write [[target| — the alias is in plain text
            // For unaliased wikilinks, write [[ — the target is in plain text
            if alias.is_some() {
                out.push_str(target);
                out.push('|');
            }
        }
        MarkType::Link { url } => {
            out.push('[');
            // text content follows, then we close with ](url)
            let _ = url; // used in close
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

/// Normalize whitespace per REQ-020-027.
fn normalize_whitespace(text: &str) -> String {
    let mut lines: Vec<&str> = text.lines().collect();

    // Remove trailing whitespace from each line
    let trimmed: Vec<String> = lines.iter().map(|l| l.trim_end().to_string()).collect();

    // Collapse multiple blank lines into single blank lines
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

    // Trailing newline at EOF
    if !result.is_empty() && !result.ends_with('\n') {
        result.push('\n');
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_document() {
        let doc = CrdtDocument::new().unwrap();
        assert_eq!(doc.text().unwrap(), "");
    }

    #[test]
    fn plain_text_round_trip() {
        let md = "Hello world\n";
        let doc = CrdtDocument::from_markdown(md).unwrap();
        let out = doc.to_markdown().unwrap();
        assert_eq!(out, md);
    }

    #[test]
    fn bold_round_trip() {
        let md = "Some **bold** text\n";
        let doc = CrdtDocument::from_markdown(md).unwrap();
        let out = doc.to_markdown().unwrap();
        assert_eq!(out, md);
    }

    #[test]
    fn italic_round_trip() {
        let md = "Some *italic* text\n";
        let doc = CrdtDocument::from_markdown(md).unwrap();
        let out = doc.to_markdown().unwrap();
        assert_eq!(out, md);
    }

    #[test]
    fn code_round_trip() {
        let md = "Some `code` text\n";
        let doc = CrdtDocument::from_markdown(md).unwrap();
        let out = doc.to_markdown().unwrap();
        assert_eq!(out, md);
    }

    #[test]
    fn wikilink_round_trip() {
        let md = "See [[Project X]] for details\n";
        let doc = CrdtDocument::from_markdown(md).unwrap();
        let out = doc.to_markdown().unwrap();
        assert_eq!(out, md);
    }

    #[test]
    fn wikilink_with_alias_round_trip() {
        let md = "See [[Project X|the project]] for details\n";
        let doc = CrdtDocument::from_markdown(md).unwrap();
        let out = doc.to_markdown().unwrap();
        assert_eq!(out, md);
    }

    #[test]
    fn strikethrough_round_trip() {
        let md = "Some ~~deleted~~ text\n";
        let doc = CrdtDocument::from_markdown(md).unwrap();
        let out = doc.to_markdown().unwrap();
        assert_eq!(out, md);
    }

    #[test]
    fn highlight_round_trip() {
        let md = "Some ==highlighted== text\n";
        let doc = CrdtDocument::from_markdown(md).unwrap();
        let out = doc.to_markdown().unwrap();
        assert_eq!(out, md);
    }

    #[test]
    fn comment_round_trip() {
        let md = "Some %%hidden%% text\n";
        let doc = CrdtDocument::from_markdown(md).unwrap();
        let out = doc.to_markdown().unwrap();
        assert_eq!(out, md);
    }

    #[test]
    fn md_link_round_trip() {
        let md = "Click [here](https://example.com) now\n";
        let doc = CrdtDocument::from_markdown(md).unwrap();
        let out = doc.to_markdown().unwrap();
        assert_eq!(out, md);
    }

    #[test]
    fn heading_round_trip() {
        let md = "## My Heading\n";
        let doc = CrdtDocument::from_markdown(md).unwrap();
        let out = doc.to_markdown().unwrap();
        assert_eq!(out, md);
    }

    #[test]
    fn code_fence_round_trip() {
        let md = "```spl\naccess(alice, read).\n```\n";
        let doc = CrdtDocument::from_markdown(md).unwrap();
        let out = doc.to_markdown().unwrap();
        assert_eq!(out, md);
    }

    #[test]
    fn mixed_marks_round_trip() {
        // TEST-020-025: bold, italic, wikilink
        let md = "**bold** and *italic* and [[Link]]\n";
        let doc = CrdtDocument::from_markdown(md).unwrap();

        // Verify mark types
        let marks = doc.marks().unwrap();
        let mark_names: Vec<&str> = marks.iter().map(|m| m.name()).collect();
        assert!(mark_names.contains(&"bold"));
        assert!(mark_names.contains(&"italic"));
        assert!(mark_names.contains(&"wikilink"));

        // Verify growth behavior
        let bold_mark = marks.iter().find(|m| m.name() == "bold").unwrap();
        let mt = MarkType::from_mark(bold_mark.name(), bold_mark.value()).unwrap();
        assert!(mt.is_inclusive()); // bold is inclusive

        let wikilink_mark = marks.iter().find(|m| m.name() == "wikilink").unwrap();
        let mt = MarkType::from_mark(wikilink_mark.name(), wikilink_mark.value()).unwrap();
        assert!(!mt.is_inclusive()); // wikilink is non-growing

        // Serialize back — byte-identical
        let out = doc.to_markdown().unwrap();
        assert_eq!(out, md);
    }

    #[test]
    fn code_mark_is_non_growing() {
        let md = "Use `code` here\n";
        let doc = CrdtDocument::from_markdown(md).unwrap();

        // The code mark should be non-growing
        let marks = doc.marks().unwrap();
        let code_mark = marks.iter().find(|m| m.name() == "code").unwrap();
        let mt = MarkType::from_mark(code_mark.name(), code_mark.value()).unwrap();
        assert_eq!(mt.expand(), ExpandMark::None);
    }

    #[test]
    fn block_heading_is_atomic() {
        // TEST-020-026: heading prefix is atomic
        let md = "## Heading\n\nParagraph text\n";
        let doc = CrdtDocument::from_markdown(md).unwrap();
        let text = doc.text().unwrap();

        // The heading prefix "## " is present as a unit in the text
        assert!(text.starts_with("## Heading"));

        // Round-trip preserves block structure
        let out = doc.to_markdown().unwrap();
        assert_eq!(out, md);
    }

    #[test]
    fn code_fence_is_opaque() {
        // TEST-020-026: code fence content is plain text, no formatting
        let md = "```spl\n**not bold** and *not italic*\n```\n";
        let doc = CrdtDocument::from_markdown(md).unwrap();

        // No formatting marks should be applied inside code fences
        let marks = doc.marks().unwrap();
        assert!(
            marks.is_empty(),
            "Code fence content should have no marks, got: {:?}",
            marks.iter().map(|m| m.name()).collect::<Vec<_>>()
        );

        let out = doc.to_markdown().unwrap();
        assert_eq!(out, md);
    }

    #[test]
    fn deterministic_serialization() {
        // TEST-020-027: same state always produces byte-identical output
        let md = "**bold** and *italic* text\n";
        let doc = CrdtDocument::from_markdown(md).unwrap();
        let out1 = doc.to_markdown().unwrap();
        let out2 = doc.to_markdown().unwrap();
        assert_eq!(out1, out2);
    }

    #[test]
    fn concurrent_edit_merge() {
        let md = "Hello world\n";
        let mut doc = CrdtDocument::from_markdown(md).unwrap();
        let mut fork = doc.fork();

        // Alice bolds "Hello" (chars 0..5)
        doc.mark(&MarkType::Bold, 0, 5).unwrap();

        // Bob italicizes "world" (chars 6..11)
        fork.mark(&MarkType::Italic, 6, 11).unwrap();

        // Merge
        doc.merge(&mut fork).unwrap();

        let out = doc.to_markdown().unwrap();
        assert!(out.contains("**Hello**"));
        assert!(out.contains("*world*"));
    }

    #[test]
    fn wikilink_non_growing_behavior() {
        // TEST-020-024: typing after ]] is plain text
        let md = "See [[Project X]] here\n";
        let mut doc = CrdtDocument::from_markdown(md).unwrap();

        // The wikilink mark covers "Project X" in the plain text
        let marks = doc.marks().unwrap();
        let wl = marks.iter().find(|m| m.name() == "wikilink").unwrap();
        let wl_end = wl.end;

        // Insert text right after the wikilink end position
        // With ExpandMark::None, this text should NOT inherit the wikilink mark
        drop(marks);
        doc.splice_text(wl_end, 0, " is great").unwrap();

        // Verify the wikilink mark did not expand
        let marks_after = doc.marks().unwrap();
        let wl_after = marks_after
            .iter()
            .find(|m| m.name() == "wikilink")
            .unwrap();
        assert_eq!(
            wl_after.end, wl_end,
            "Wikilink should not grow when text is inserted at boundary"
        );
    }

    #[test]
    fn save_and_load_round_trip() {
        let md = "**bold** and [[Link]]\n";
        let mut doc = CrdtDocument::from_markdown(md).unwrap();
        let bytes = doc.save();

        let loaded = CrdtDocument::load(&bytes).unwrap();
        let out = loaded.to_markdown().unwrap();
        assert_eq!(out, md);
    }

    #[test]
    fn whitespace_normalization() {
        let text = "line one  \n\n\n\nline two";
        let normalized = normalize_whitespace(text);
        assert_eq!(normalized, "line one\n\nline two\n");
    }

    #[test]
    fn list_item_round_trip() {
        let md = "- first item\n- second item\n";
        let doc = CrdtDocument::from_markdown(md).unwrap();
        let out = doc.to_markdown().unwrap();
        assert_eq!(out, md);
    }
}
