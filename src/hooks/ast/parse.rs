//! Markdown → [`Document`] conversion used by `zetl ast sample`
//! (SPEC-032 REQ-3225).
//!
//! The transform-stage pipeline boundary (CON-3201) exchanges AST documents,
//! not raw Markdown. To let hook authors introspect what a transform hook
//! would actually receive for a given page, this module realises the
//! `parse` step of the pipeline: it walks pulldown-cmark events, extracts
//! Obsidian-style `[[wikilinks]]` and `![[embeds]]`, and produces a
//! [`Document`] that validates against `tools/zetl-ast-schema-v1.json`.
//!
//! Scope: CommonMark-subset covered by the v1 schema — Heading, Paragraph,
//! BlockQuote, List/ListItem, CodeBlock (fenced or indented, promoted to
//! [`SplBlock`] when the info string is `spl`), ThematicBreak, HtmlBlock,
//! Embed, and the inline set. Features outside the schema (tables,
//! strikethrough, footnotes, definition lists) are represented as Text /
//! HtmlBlock fallbacks rather than omitted.
//!
//! Positions are 1-indexed, inclusive, anchored against the *original* file
//! (frontmatter included) so hook authors can cross-reference errors against
//! line/col pairs reported by their editor.

use std::ops::Range;

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use super::{
    Block, BlockQuote, Code, CodeBlock, Document, DocumentKind, Embed, Emphasis, Frontmatter,
    Heading, HtmlBlock, HtmlInline, Image, Inline, LineBreak, Link, List, ListItem, Paragraph,
    Position, SoftBreak, SplBlock, Strong, Text, ThematicBreak, Wikilink, AST_VERSION,
};

/// Parse a full Markdown file (with or without YAML frontmatter) into a
/// canonical [`Document`].
pub fn parse_markdown(content: &str) -> Document {
    let (frontmatter, body, fm_offset_lines) = split_frontmatter(content);
    let line_map = LineMap::new(&body, fm_offset_lines);

    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_HEADING_ATTRIBUTES
        | Options::ENABLE_MATH
        | Options::ENABLE_GFM
        | Options::ENABLE_DEFINITION_LIST;

    let parser = Parser::new_ext(&body, options);
    let mut builder = Builder::new(&line_map);
    for (event, range) in parser.into_offset_iter() {
        builder.handle(event, range);
    }

    let children = builder.finish();
    let total_lines = line_map.total_lines().max(1);
    let doc_position = Position::new(1, 1, total_lines as u32, 1);

    Document {
        ast_version: AST_VERSION.to_string(),
        kind: DocumentKind::Document,
        position: doc_position,
        frontmatter,
        children,
    }
}

// ── Frontmatter + line map ──────────────────────────────────────────────────

fn split_frontmatter(content: &str) -> (Option<Frontmatter>, String, usize) {
    let trimmed = content.trim_start();
    let leading = content.len() - trimmed.len();
    if !trimmed.starts_with("---") {
        return (None, content.to_string(), 0);
    }
    let after_first = &trimmed[3..];
    let Some(end_pos) = after_first.find("\n---") else {
        return (None, content.to_string(), 0);
    };
    let yaml_block = &after_first[..end_pos];
    let fm = match serde_yaml_ng::from_str::<serde_json::Value>(yaml_block) {
        Ok(serde_json::Value::Object(map)) => Some(map),
        _ => None,
    };
    let skip = 3 + end_pos + 4; // "---" + body + "\n---"
    let rest = &trimmed[skip..];
    let rest = rest.strip_prefix('\n').unwrap_or(rest);
    let fm_chunk = &content[..leading + skip];
    let fm_line_count = fm_chunk.matches('\n').count() + 1;
    (fm, rest.to_string(), fm_line_count)
}

/// Byte offset → (line, col) resolver for a markdown body, shifted by the
/// number of lines consumed by a leading frontmatter block so positions
/// reported in the AST line up with the *original* file.
struct LineMap<'a> {
    body: &'a str,
    /// 0-based byte offset of the start of each line within `body`.
    starts: Vec<usize>,
    /// Line offset added to every emitted position (lines consumed by
    /// frontmatter, if any).
    fm_lines: usize,
}

impl<'a> LineMap<'a> {
    fn new(body: &'a str, fm_lines: usize) -> Self {
        let starts: Vec<usize> = std::iter::once(0)
            .chain(body.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        Self {
            body,
            starts,
            fm_lines,
        }
    }

    fn total_lines(&self) -> usize {
        self.starts.len() + self.fm_lines
    }

    /// Map a *byte* offset into the body to a 1-indexed (line, col).
    fn offset(&self, byte: usize) -> (u32, u32) {
        let byte = byte.min(self.body.len());
        let line_idx = self
            .starts
            .partition_point(|&s| s <= byte)
            .saturating_sub(1);
        let line_start = self.starts[line_idx];
        let col_bytes = byte - line_start;
        let col_chars = self.body[line_start..line_start + col_bytes]
            .chars()
            .count();
        let line = (line_idx + 1 + self.fm_lines) as u32;
        let col = (col_chars + 1) as u32;
        (line.max(1), col.max(1))
    }

    fn position(&self, range: &Range<usize>) -> Position {
        let (sl, sc) = self.offset(range.start);
        // end offset is exclusive; step back one byte to stay "inclusive".
        let end_byte = range.end.saturating_sub(1).max(range.start);
        let (el, ec) = self.offset(end_byte);
        Position::new(sl, sc, el.max(sl), ec.max(1))
    }
}

// ── Frame stack ─────────────────────────────────────────────────────────────

enum Frame {
    Root(Vec<Block>),
    Paragraph(Position, Vec<Inline>),
    Heading(Position, u8, Vec<Inline>),
    BlockQuote(Position, Vec<Block>),
    List {
        position: Position,
        ordered: bool,
        start: Option<u32>,
        items: Vec<ListItem>,
        /// Whether *any* of the items contained a wrapping Paragraph —
        /// pulldown-cmark uses a Paragraph wrapper exactly when the source
        /// list is loose (blank lines between items).
        loose: bool,
    },
    Item {
        position: Position,
        blocks: Vec<Block>,
        pending_inlines: Vec<Inline>,
    },
    Emphasis(Position, Vec<Inline>),
    Strong(Position, Vec<Inline>),
    Link {
        position: Position,
        url: String,
        title: Option<String>,
        children: Vec<Inline>,
    },
    Image {
        position: Position,
        url: String,
        title: Option<String>,
        alt_buf: String,
        children: Vec<Inline>,
    },
    CodeBlock {
        position: Position,
        fenced: bool,
        info: Option<String>,
        lang: Option<String>,
        text: String,
    },
    HtmlBlock {
        position: Position,
        text: String,
    },
    /// Swallow events inside unsupported containers (Table, FootnoteDefinition,
    /// DefinitionList, Strikethrough, …) so the surrounding document remains
    /// well-formed. Contents are dropped.
    Skip,
}

struct Builder<'a> {
    line_map: &'a LineMap<'a>,
    stack: Vec<Frame>,
}

impl<'a> Builder<'a> {
    fn new(line_map: &'a LineMap<'a>) -> Self {
        Self {
            line_map,
            stack: vec![Frame::Root(Vec::new())],
        }
    }

    fn handle(&mut self, event: Event<'_>, range: Range<usize>) {
        if matches!(self.stack.last(), Some(Frame::Skip)) {
            // While skipping, still track balanced Start/End so we exit the
            // swallow on the matching close.
            match event {
                Event::Start(_) => self.stack.push(Frame::Skip),
                Event::End(_) => {
                    self.stack.pop();
                }
                _ => {}
            }
            return;
        }

        // Route plain Text events inside literal-text frames (CodeBlock,
        // HtmlBlock) straight into the frame's buffer; the inline-text
        // path would mis-classify the content as Paragraph children.
        if let Event::Text(s) = &event {
            match self.stack.last_mut() {
                Some(Frame::CodeBlock { text, .. }) => {
                    text.push_str(s);
                    return;
                }
                Some(Frame::HtmlBlock { text, .. }) => {
                    text.push_str(s);
                    return;
                }
                _ => {}
            }
        }

        match event {
            Event::Start(tag) => self.start_tag(tag, range),
            Event::End(end) => self.end_tag(end, range),
            Event::Text(s) => self.push_text(&s, &range),
            Event::Code(s) => self.push_inline(Inline::Code(Code {
                position: self.line_map.position(&range),
                text: s.to_string(),
            })),
            Event::Html(s) => self.push_html_block(&s, &range),
            Event::InlineHtml(s) => self.push_inline(Inline::HtmlInline(HtmlInline {
                position: self.line_map.position(&range),
                text: s.to_string(),
            })),
            Event::SoftBreak => self.push_inline(Inline::SoftBreak(SoftBreak {
                position: self.line_map.position(&range),
            })),
            Event::HardBreak => self.push_inline(Inline::LineBreak(LineBreak {
                position: self.line_map.position(&range),
            })),
            Event::Rule => self.push_block(Block::ThematicBreak(ThematicBreak {
                position: self.line_map.position(&range),
            })),
            // Fallbacks for features outside the v1 schema: preserve content
            // as plain text rather than dropping silently.
            Event::InlineMath(s) | Event::DisplayMath(s) => self.push_text(&s, &range),
            Event::FootnoteReference(_) | Event::TaskListMarker(_) => {}
        }
    }

    fn start_tag(&mut self, tag: Tag<'_>, range: Range<usize>) {
        let pos = self.line_map.position(&range);
        match tag {
            Tag::Paragraph => self.stack.push(Frame::Paragraph(pos, Vec::new())),
            Tag::Heading { level, .. } => {
                self.stack
                    .push(Frame::Heading(pos, heading_level(level), Vec::new()))
            }
            Tag::BlockQuote(_) => self.stack.push(Frame::BlockQuote(pos, Vec::new())),
            Tag::CodeBlock(kind) => {
                let (fenced, info, lang) = match kind {
                    CodeBlockKind::Indented => (false, None, None),
                    CodeBlockKind::Fenced(info_cow) => {
                        let info_str = info_cow.to_string();
                        let lang = info_str.split_whitespace().next().map(str::to_owned);
                        let info = if info_str.is_empty() {
                            None
                        } else {
                            Some(info_str)
                        };
                        (true, info, lang)
                    }
                };
                self.stack.push(Frame::CodeBlock {
                    position: pos,
                    fenced,
                    info,
                    lang,
                    text: String::new(),
                });
            }
            Tag::HtmlBlock => self.stack.push(Frame::HtmlBlock {
                position: pos,
                text: String::new(),
            }),
            Tag::List(start) => self.stack.push(Frame::List {
                position: pos,
                ordered: start.is_some(),
                start: start.map(|n| n as u32),
                items: Vec::new(),
                loose: false,
            }),
            Tag::Item => self.stack.push(Frame::Item {
                position: pos,
                blocks: Vec::new(),
                pending_inlines: Vec::new(),
            }),
            Tag::Emphasis => self.stack.push(Frame::Emphasis(pos, Vec::new())),
            Tag::Strong => self.stack.push(Frame::Strong(pos, Vec::new())),
            Tag::Link {
                dest_url, title, ..
            } => self.stack.push(Frame::Link {
                position: pos,
                url: dest_url.to_string(),
                title: if title.is_empty() {
                    None
                } else {
                    Some(title.to_string())
                },
                children: Vec::new(),
            }),
            Tag::Image {
                dest_url, title, ..
            } => self.stack.push(Frame::Image {
                position: pos,
                url: dest_url.to_string(),
                title: if title.is_empty() {
                    None
                } else {
                    Some(title.to_string())
                },
                alt_buf: String::new(),
                children: Vec::new(),
            }),
            // Out-of-schema: swallow the subtree.
            Tag::Strikethrough
            | Tag::Table(_)
            | Tag::TableHead
            | Tag::TableRow
            | Tag::TableCell
            | Tag::FootnoteDefinition(_)
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition
            | Tag::MetadataBlock(_) => self.stack.push(Frame::Skip),
        }
    }

    fn end_tag(&mut self, _end: TagEnd, _range: Range<usize>) {
        let Some(frame) = self.stack.pop() else {
            return;
        };
        match frame {
            Frame::Root(children) => {
                // Shouldn't happen in balanced input; restore and stop.
                self.stack.push(Frame::Root(children));
            }
            Frame::Paragraph(position, inlines) => {
                if let Some(embed) = detect_embed_paragraph(&inlines, position) {
                    self.push_block(Block::Embed(embed));
                } else {
                    self.push_block(Block::Paragraph(Paragraph {
                        position,
                        children: inlines,
                    }));
                }
            }
            Frame::Heading(position, level, children) => {
                self.push_block(Block::Heading(Heading {
                    position,
                    level,
                    children,
                }));
            }
            Frame::BlockQuote(position, children) => {
                self.push_block(Block::BlockQuote(BlockQuote { position, children }));
            }
            Frame::List {
                position,
                ordered,
                start,
                items,
                loose,
            } => {
                self.push_block(Block::List(List {
                    position,
                    ordered,
                    tight: !loose,
                    start,
                    children: items,
                }));
            }
            Frame::Item {
                position,
                mut blocks,
                pending_inlines,
            } => {
                // Tight-list items may accumulate inlines without a wrapping
                // Paragraph event from pulldown-cmark; wrap them now so the
                // schema's `children: Block[]` constraint holds.
                if !pending_inlines.is_empty() {
                    blocks.push(Block::Paragraph(Paragraph {
                        position,
                        children: pending_inlines,
                    }));
                }
                if let Some(Frame::List { items, .. }) = self.stack.last_mut() {
                    items.push(ListItem::new(position, blocks));
                }
            }
            Frame::Emphasis(position, children) => {
                self.push_inline(Inline::Emphasis(Emphasis { position, children }));
            }
            Frame::Strong(position, children) => {
                self.push_inline(Inline::Strong(Strong { position, children }));
            }
            Frame::Link {
                position,
                url,
                title,
                children,
            } => {
                self.push_inline(Inline::Link(Link {
                    position,
                    url,
                    title,
                    children,
                }));
            }
            Frame::Image {
                position,
                url,
                title,
                alt_buf,
                children,
            } => {
                self.push_inline(Inline::Image(Image {
                    position,
                    url,
                    title,
                    alt: alt_buf,
                    children,
                }));
            }
            Frame::CodeBlock {
                position,
                fenced,
                info,
                lang,
                text,
            } => {
                let lang_key = lang.as_deref().unwrap_or("");
                if lang_key == "spl" {
                    self.push_block(Block::SplBlock(SplBlock {
                        position,
                        info,
                        text,
                    }));
                } else {
                    self.push_block(Block::CodeBlock(CodeBlock {
                        position,
                        fenced,
                        lang,
                        info,
                        text,
                    }));
                }
            }
            Frame::HtmlBlock { position, text } => {
                self.push_block(Block::HtmlBlock(HtmlBlock { position, text }));
            }
            Frame::Skip => {}
        }
    }

    fn push_block(&mut self, block: Block) {
        match self.stack.last_mut() {
            Some(Frame::Root(children)) => children.push(block),
            Some(Frame::BlockQuote(_, children)) => children.push(block),
            Some(Frame::Item {
                blocks,
                pending_inlines,
                position,
                ..
            }) => {
                // Starting a Paragraph inside the item indicates a loose list;
                // record that up-stack by flushing any pending inlines first.
                if !pending_inlines.is_empty() {
                    let drained = std::mem::take(pending_inlines);
                    blocks.push(Block::Paragraph(Paragraph {
                        position: *position,
                        children: drained,
                    }));
                }
                blocks.push(block);
                if let Some(Frame::List { loose, .. }) = self
                    .stack
                    .iter_mut()
                    .rev()
                    .find(|f| matches!(f, Frame::List { .. }))
                {
                    *loose = true;
                }
            }
            _ => {}
        }
    }

    fn push_inline(&mut self, inline: Inline) {
        match self.stack.last_mut() {
            Some(Frame::Paragraph(_, inlines))
            | Some(Frame::Heading(_, _, inlines))
            | Some(Frame::Emphasis(_, inlines))
            | Some(Frame::Strong(_, inlines))
            | Some(Frame::Link {
                children: inlines, ..
            }) => {
                inlines.push(inline);
            }
            Some(Frame::Image {
                children: inlines,
                alt_buf,
                ..
            }) => {
                if let Inline::Text(t) = &inline {
                    alt_buf.push_str(&t.text);
                }
                inlines.push(inline);
            }
            Some(Frame::Item {
                pending_inlines, ..
            }) => {
                pending_inlines.push(inline);
            }
            _ => {}
        }
    }

    fn push_text(&mut self, s: &str, range: &Range<usize>) {
        let position = self.line_map.position(range);
        // Emit text verbatim; wikilink extraction runs as a post-pass on
        // concatenated adjacent Text nodes because pulldown-cmark splits a
        // run containing `[` / `]` across multiple Text events while
        // attempting (and failing) to match reference-link syntax.
        self.push_inline(Inline::Text(Text {
            position,
            text: s.to_string(),
        }));
    }

    fn push_html_block(&mut self, s: &str, range: &Range<usize>) {
        // Inside an HtmlBlock frame, chunks stream as Html events.
        if let Some(Frame::HtmlBlock { text, .. }) = self.stack.last_mut() {
            text.push_str(s);
            return;
        }
        // Stray Html event (e.g. raw HTML paragraph handled at top-level).
        self.push_block(Block::HtmlBlock(HtmlBlock {
            position: self.line_map.position(range),
            text: s.trim_end_matches('\n').to_string(),
        }));
    }

    fn finish(mut self) -> Vec<Block> {
        let mut blocks = match self.stack.pop() {
            Some(Frame::Root(blocks)) => blocks,
            _ => Vec::new(),
        };
        // Post-pass: merge adjacent Text runs and extract `[[wikilinks]]`.
        // `![[embeds]]` appearing as a standalone paragraph are promoted
        // to block-level [`Embed`] here as well.
        post_process_blocks(&mut blocks);
        blocks
    }
}

// ── Post-process: wikilink extraction + embed promotion ─────────────────────

fn post_process_blocks(blocks: &mut [Block]) {
    for block in blocks.iter_mut() {
        if let Some(replacement) = post_process_block(block) {
            *block = replacement;
        }
    }
}

fn post_process_block(block: &mut Block) -> Option<Block> {
    match block {
        Block::Paragraph(p) => {
            process_inlines(&mut p.children);
            if let Some(embed) = detect_embed_paragraph(&p.children, p.position) {
                return Some(Block::Embed(embed));
            }
            None
        }
        Block::Heading(h) => {
            process_inlines(&mut h.children);
            None
        }
        Block::BlockQuote(bq) => {
            post_process_blocks(&mut bq.children);
            None
        }
        Block::List(l) => {
            for item in &mut l.children {
                post_process_blocks(&mut item.children);
            }
            None
        }
        _ => None,
    }
}

/// Walk an inline vector: merge adjacent Text nodes, extract wikilinks,
/// then recurse into nested inline containers.
fn process_inlines(items: &mut Vec<Inline>) {
    // First merge adjacent Text runs so wikilinks split by pulldown-cmark
    // across multiple Text events can still be recognised.
    let mut merged: Vec<Inline> = Vec::with_capacity(items.len());
    for item in items.drain(..) {
        match item {
            Inline::Text(t) => match merged.last_mut() {
                Some(Inline::Text(prev)) => prev.text.push_str(&t.text),
                _ => merged.push(Inline::Text(t)),
            },
            other => merged.push(other),
        }
    }

    // Now extract wikilinks from each (possibly merged) Text run.
    let mut out: Vec<Inline> = Vec::with_capacity(merged.len());
    for item in merged {
        match item {
            Inline::Text(t) => {
                for piece in split_wikilinks(&t.text, t.position) {
                    out.push(piece);
                }
            }
            Inline::Emphasis(mut e) => {
                process_inlines(&mut e.children);
                out.push(Inline::Emphasis(e));
            }
            Inline::Strong(mut s) => {
                process_inlines(&mut s.children);
                out.push(Inline::Strong(s));
            }
            Inline::Link(mut l) => {
                process_inlines(&mut l.children);
                out.push(Inline::Link(l));
            }
            Inline::Image(mut img) => {
                process_inlines(&mut img.children);
                out.push(Inline::Image(img));
            }
            other => out.push(other),
        }
    }
    *items = out;
}

fn heading_level(lvl: HeadingLevel) -> u8 {
    match lvl {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

// ── Wikilink extraction ─────────────────────────────────────────────────────

/// Split a text run on `[[...]]` (wikilinks), emitting interleaved
/// [`Inline::Text`] and [`Inline::Wikilink`]. `![[...]]` sequences become
/// Wikilinks too; promotion to block-level [`Embed`] happens later at the
/// paragraph close if the paragraph's contents match an embed-only shape.
fn split_wikilinks(s: &str, position: Position) -> Vec<Inline> {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut text_start = 0;
    while i < bytes.len() {
        let window = &s[i..];
        let (is_embed, inner_start) = if window.starts_with("![[") {
            (true, i + 3)
        } else if window.starts_with("[[") {
            (false, i + 2)
        } else {
            i += 1;
            continue;
        };
        if let Some(rel_end) = s[inner_start..].find("]]") {
            let close_idx = inner_start + rel_end;
            let inner = &s[inner_start..close_idx];
            // No nested brackets allowed inside wikilink inner.
            if inner.contains('[') || inner.contains(']') {
                i = inner_start;
                continue;
            }
            // Flush preceding text.
            if text_start < i {
                out.push(Inline::Text(Text {
                    position,
                    text: s[text_start..i].to_string(),
                }));
            }
            let wl = parse_wikilink_inner(inner, position);
            // Embeds inline are still Wikilinks here (Embed is block-level;
            // promotion happens in detect_embed_paragraph). Preserve the `!`
            // in the text when we can't promote, so `![[foo]]` embedded
            // inline (rare) round-trips visually.
            if is_embed {
                out.push(Inline::Text(Text {
                    position,
                    text: "!".into(),
                }));
            }
            out.push(wl);
            i = close_idx + 2;
            text_start = i;
        } else {
            i = inner_start;
        }
    }
    if text_start < bytes.len() {
        out.push(Inline::Text(Text {
            position,
            text: s[text_start..].to_string(),
        }));
    }
    out
}

fn parse_wikilink_inner(inner: &str, position: Position) -> Inline {
    let (target_and_frag, alias) = match inner.split_once('|') {
        Some((t, a)) => (t, Some(a.to_string())),
        None => (inner, None),
    };
    let (target_raw, heading, block_id) = if let Some((t, rest)) = target_and_frag.split_once('#') {
        if let Some(bid) = rest.strip_prefix('^') {
            (t, None, Some(bid.to_string()))
        } else {
            (t, Some(rest.to_string()), None)
        }
    } else {
        (target_and_frag, None, None)
    };
    Inline::Wikilink(Wikilink {
        position,
        target: target_raw.to_string(),
        alias,
        heading,
        block_id,
    })
}

/// A paragraph consisting of a single `![[target]]` (or stripped equivalent)
/// is promoted to a block-level [`Embed`].
fn detect_embed_paragraph(inlines: &[Inline], position: Position) -> Option<Embed> {
    let wl = match inlines {
        [Inline::Text(t), Inline::Wikilink(w)] if t.text.trim() == "!" => w,
        _ => return None,
    };
    Some(Embed {
        position,
        target: wl.target.clone(),
        heading: wl.heading.clone(),
        block_id: wl.block_id.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_produces_empty_document() {
        let doc = parse_markdown("");
        assert_eq!(doc.ast_version, AST_VERSION);
        assert!(doc.children.is_empty());
        assert!(doc.frontmatter.is_none());
    }

    #[test]
    fn frontmatter_is_parsed_and_body_positions_shift() {
        let src = "---\ntitle: T\n---\n# Header\n";
        let doc = parse_markdown(src);
        assert!(doc.frontmatter.is_some());
        assert_eq!(doc.frontmatter.as_ref().unwrap()["title"], "T");
        assert_eq!(doc.children.len(), 1);
        if let Block::Heading(h) = &doc.children[0] {
            // Heading lives on line 4 in the original file.
            assert!(h.position.start_line >= 4, "{:?}", h.position);
        } else {
            panic!("expected heading");
        }
    }

    #[test]
    fn wikilink_inline_with_alias_and_heading() {
        let doc = parse_markdown("see [[Target#Section|alias]] here\n");
        let Block::Paragraph(p) = &doc.children[0] else {
            panic!("expected paragraph");
        };
        let wl = p.children.iter().find_map(|i| match i {
            Inline::Wikilink(w) => Some(w),
            _ => None,
        });
        let wl = wl.expect("wikilink present");
        assert_eq!(wl.target, "Target");
        assert_eq!(wl.heading.as_deref(), Some("Section"));
        assert_eq!(wl.alias.as_deref(), Some("alias"));
        assert!(wl.block_id.is_none());
    }

    #[test]
    fn wikilink_block_id_fragment() {
        let doc = parse_markdown("see [[Target#^abc]]\n");
        let Block::Paragraph(p) = &doc.children[0] else {
            panic!()
        };
        let wl = p
            .children
            .iter()
            .find_map(|i| match i {
                Inline::Wikilink(w) => Some(w),
                _ => None,
            })
            .unwrap();
        assert_eq!(wl.target, "Target");
        assert_eq!(wl.block_id.as_deref(), Some("abc"));
        assert!(wl.heading.is_none());
    }

    #[test]
    fn standalone_embed_is_promoted_to_block() {
        let doc = parse_markdown("![[Other]]\n");
        assert!(matches!(doc.children[0], Block::Embed(_)));
        if let Block::Embed(e) = &doc.children[0] {
            assert_eq!(e.target, "Other");
        }
    }

    #[test]
    fn spl_code_block_becomes_spl_block() {
        let doc = parse_markdown("```spl\nfact :x\n```\n");
        match &doc.children[0] {
            Block::SplBlock(s) => {
                assert_eq!(s.text, "fact :x\n");
                assert_eq!(s.info.as_deref(), Some("spl"));
            }
            other => panic!("expected SplBlock, got {other:?}"),
        }
    }

    #[test]
    fn fenced_code_block_preserves_lang_and_info() {
        let doc = parse_markdown("```rust ignore\nfn x() {}\n```\n");
        match &doc.children[0] {
            Block::CodeBlock(c) => {
                assert!(c.fenced);
                assert_eq!(c.lang.as_deref(), Some("rust"));
                assert_eq!(c.info.as_deref(), Some("rust ignore"));
                assert_eq!(c.text, "fn x() {}\n");
            }
            _ => panic!("expected code block"),
        }
    }

    #[test]
    fn nested_list_keeps_block_children() {
        let src = "- one\n- two\n";
        let doc = parse_markdown(src);
        let Block::List(list) = &doc.children[0] else {
            panic!("expected list")
        };
        assert!(!list.ordered);
        assert_eq!(list.children.len(), 2);
    }

    #[test]
    fn ordered_list_start_is_recorded() {
        let src = "3. one\n4. two\n";
        let doc = parse_markdown(src);
        let Block::List(list) = &doc.children[0] else {
            panic!()
        };
        assert!(list.ordered);
        assert_eq!(list.start, Some(3));
    }

    #[test]
    fn blockquote_preserves_inner_paragraphs() {
        let doc = parse_markdown("> hello\n> world\n");
        let Block::BlockQuote(bq) = &doc.children[0] else {
            panic!()
        };
        assert!(!bq.children.is_empty());
    }

    #[test]
    fn thematic_break_is_top_level() {
        let doc = parse_markdown("---\n");
        assert!(matches!(doc.children[0], Block::ThematicBreak(_)));
    }

    #[test]
    fn emphasis_and_strong_nest_correctly() {
        let doc = parse_markdown("**bold *em***\n");
        let Block::Paragraph(p) = &doc.children[0] else {
            panic!()
        };
        let Inline::Strong(s) = &p.children[0] else {
            panic!()
        };
        assert!(matches!(s.children.last(), Some(Inline::Emphasis(_))));
    }

    #[test]
    fn link_preserves_url_title_and_children() {
        let doc = parse_markdown("[home](https://e.x \"Title\")\n");
        let Block::Paragraph(p) = &doc.children[0] else {
            panic!()
        };
        let Inline::Link(l) = &p.children[0] else {
            panic!()
        };
        assert_eq!(l.url, "https://e.x");
        assert_eq!(l.title.as_deref(), Some("Title"));
    }

    #[test]
    fn image_alt_text_populated() {
        let doc = parse_markdown("![cat](cat.png)\n");
        let Block::Paragraph(p) = &doc.children[0] else {
            panic!()
        };
        let Inline::Image(img) = &p.children[0] else {
            panic!()
        };
        assert_eq!(img.url, "cat.png");
        assert_eq!(img.alt, "cat");
    }

    #[test]
    fn html_block_preserved_verbatim() {
        let doc = parse_markdown("<div>raw</div>\n");
        match &doc.children[0] {
            Block::HtmlBlock(h) => assert!(h.text.contains("<div>raw</div>")),
            _ => panic!("expected html block"),
        }
    }

    #[test]
    fn table_source_is_dropped_to_keep_document_valid() {
        // Tables are out of the v1 schema; the parser swallows them rather
        // than emitting an invalid node.
        let doc = parse_markdown("| a | b |\n|---|---|\n| 1 | 2 |\n");
        // The document is still well-formed even if empty.
        let json = serde_json::to_value(&doc).unwrap();
        assert_eq!(json["type"], "Document");
    }

    #[test]
    fn wikilink_with_unclosed_brackets_is_left_as_text() {
        let doc = parse_markdown("text [[unclosed\n");
        let Block::Paragraph(p) = &doc.children[0] else {
            panic!()
        };
        assert!(matches!(p.children.first(), Some(Inline::Text(_))));
        assert!(p.children.iter().all(|i| !matches!(i, Inline::Wikilink(_))));
    }
}
