//! The Loro-backed rich-text editing document (SPEC-047 ADR-470/§9, IMPL-047 T3).
//!
//! The one editing engine, on [[Loro]] (replacing the former diamond-types
//! engine + `CrdtBackend` trait). Loro's rich text is natively Peritext (text
//! with expand-aware style marks), so one `LoroText` container holds both text
//! and inline marks. Block structure (headings, lists, fences) is stored as
//! literal text; only inline marks live in the style layer. The markdown
//! mapping is the shared `marks` module (`parse_inline_marks` /
//! `serialize_to_markdown`).

use std::collections::HashMap;

use anyhow::{Context, Result};
use loro::{ExpandType, ExportMode, LoroDoc, LoroValue, StyleConfig, StyleConfigMap, TextDelta};

use super::blocks::BlockToken;
use super::marks::{parse_inline_marks, serialize_to_markdown, ExpandMark, Mark, MarkType, Scalar};

/// The single text container holding a note's rich-text content.
const CONTENT: &str = "content";
/// Every mark key zetl uses, with its Peritext expand behaviour. Registered on
/// every document so marks grow (or not) at boundaries consistently.
const MARK_KEYS: &[(&str, ExpandMark)] = &[
    ("bold", ExpandMark::Both),
    ("italic", ExpandMark::Both),
    ("strikethrough", ExpandMark::Both),
    ("highlight", ExpandMark::Both),
    ("code", ExpandMark::None),
    ("wikilink", ExpandMark::None),
    ("link", ExpandMark::None),
    ("comment", ExpandMark::None),
];

fn expand_of(e: ExpandMark) -> ExpandType {
    match e {
        ExpandMark::Both => ExpandType::Both,
        ExpandMark::None => ExpandType::None,
    }
}

/// A fresh `LoroDoc` with zetl's mark styles configured (so `mark`/`unmark`
/// behave identically on every document — new, loaded, forked, merged).
fn configured_doc() -> LoroDoc {
    let doc = LoroDoc::new();
    let mut styles = StyleConfigMap::new();
    for (key, expand) in MARK_KEYS {
        styles.insert((*key).into(), StyleConfig { expand: expand_of(*expand) });
    }
    doc.config_text_style(styles);
    doc
}

fn scalar_to_loro(s: Scalar) -> LoroValue {
    match s {
        Scalar::Bool(b) => LoroValue::Bool(b),
        Scalar::Str(s) => LoroValue::String(s.into()),
        Scalar::Int(i) => LoroValue::I64(i),
        Scalar::Null => LoroValue::Null,
    }
}

fn loro_to_scalar(v: &LoroValue) -> Scalar {
    match v {
        LoroValue::Bool(b) => Scalar::Bool(*b),
        LoroValue::String(s) => Scalar::Str(s.to_string()),
        LoroValue::I64(i) => Scalar::Int(*i),
        LoroValue::Null => Scalar::Null,
        // Presence sentinel, matching Scalar::from_json's fallback.
        _ => Scalar::Bool(true),
    }
}

/// The Loro editing document.
pub struct LoroCrdtDocument {
    doc: LoroDoc,
}

impl LoroCrdtDocument {
    /// Ingest a markdown document, mirroring diamond's line-by-line ingestion:
    /// block prefixes and fenced/frontmatter lines are inserted as literal
    /// text; inline marks are collected and applied after the text exists so
    /// inclusive marks never absorb the structural newlines.
    fn load_markdown(&mut self, markdown: &str) -> Result<()> {
        let lines: Vec<&str> = markdown.lines().collect();
        let mut pending: Vec<(MarkType, usize, usize)> = Vec::new();
        let mut pos: usize = 0;
        let mut in_code_fence = false;
        let mut in_frontmatter = false;
        let mut is_first_line = true;

        for (line_idx, line) in lines.iter().enumerate() {
            if line_idx > 0 {
                self.splice_text(pos, 0, "\n")?;
                pos += 1;
            }
            if *line == "---" && (is_first_line || in_frontmatter) {
                self.splice_text(pos, 0, line)?;
                pos += line.chars().count();
                in_frontmatter = is_first_line;
                is_first_line = false;
                continue;
            }
            is_first_line = false;
            if in_frontmatter {
                self.splice_text(pos, 0, line)?;
                pos += line.chars().count();
                continue;
            }
            if line.starts_with("```") {
                self.splice_text(pos, 0, line)?;
                pos += line.chars().count();
                in_code_fence = !in_code_fence;
                continue;
            }
            if in_code_fence {
                self.splice_text(pos, 0, line)?;
                pos += line.chars().count();
                continue;
            }
            if line.is_empty() {
                continue;
            }
            if let Some((block_token, content)) = BlockToken::parse_line_prefix(line) {
                let prefix = block_token.to_markdown();
                self.splice_text(pos, 0, &prefix)?;
                pos += prefix.chars().count();
                pos = self.insert_inline(content, pos, &mut pending)?;
            } else {
                pos = self.insert_inline(line, pos, &mut pending)?;
            }
        }

        let text = self.text()?;
        if !text.is_empty() && !text.ends_with('\n') {
            self.splice_text(pos, 0, "\n")?;
        }
        for (mt, start, end) in pending {
            self.mark(&mt, start, end)?;
        }
        Ok(())
    }

    fn insert_inline(
        &mut self,
        line: &str,
        start_pos: usize,
        pending: &mut Vec<(MarkType, usize, usize)>,
    ) -> Result<usize> {
        let parsed = parse_inline_marks(line);
        self.splice_text(start_pos, 0, &parsed.plain_text)?;
        for m in parsed.marks {
            pending.push((m.mark_type, start_pos + m.start, start_pos + m.end));
        }
        Ok(start_pos + parsed.plain_text.chars().count())
    }

    fn text_handle(&self) -> loro::LoroText {
        self.doc.get_text(CONTENT)
    }
}

/// The editing surface (formerly the `CrdtBackend` trait, now inherent — one
/// engine, no indirection, per SPEC-047 §9).
impl LoroCrdtDocument {
    pub fn new() -> Result<Self> {
        Ok(Self { doc: configured_doc() })
    }

    pub fn from_markdown(markdown: &str) -> Result<Self> {
        let mut this = Self::new()?;
        this.load_markdown(markdown)?;
        Ok(this)
    }

    pub fn load(data: &[u8]) -> Result<Self> {
        let doc = configured_doc();
        doc.import(data).context("import loro editing snapshot")?;
        Ok(Self { doc })
    }

    pub fn text(&self) -> Result<String> {
        Ok(self.text_handle().to_string())
    }

    pub fn marks(&self) -> Result<Vec<Mark>> {
        // Reconstruct contiguous mark spans by walking the rich-text delta,
        // tracking each style key's open span until it lapses or changes value.
        let mut open: HashMap<String, (Scalar, usize)> = HashMap::new();
        let mut out: Vec<Mark> = Vec::new();
        let mut pos: usize = 0;

        for d in self.text_handle().to_delta() {
            let TextDelta::Insert { insert, attributes } = d else {
                continue;
            };
            let len = insert.chars().count();
            let active: HashMap<String, Scalar> = attributes
                .unwrap_or_default()
                .into_iter()
                .filter_map(|(k, v)| {
                    let s = loro_to_scalar(&v);
                    // A style set to null is an erased mark, not an active one.
                    (!matches!(s, Scalar::Null)).then_some((k, s))
                })
                .collect();

            // Close every open key that is now absent or whose value changed.
            let to_close: Vec<String> = open
                .iter()
                .filter(|(k, (val, _))| active.get(*k) != Some(val))
                .map(|(k, _)| k.clone())
                .collect();
            for k in to_close {
                let (val, start) = open.remove(&k).expect("key was open");
                out.push(Mark { name: k, value: val, start, end: pos });
            }
            // Open any active key not already open.
            for (k, v) in active {
                open.entry(k).or_insert((v, pos));
            }
            pos += len;
        }
        for (k, (val, start)) in open {
            out.push(Mark { name: k, value: val, start, end: pos });
        }
        // Deterministic order: by span start, then nesting, then name.
        out.sort_by(|a, b| {
            a.start
                .cmp(&b.start)
                .then_with(|| nesting(&a.name).cmp(&nesting(&b.name)))
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(out)
    }

    pub fn splice_text(&mut self, pos: usize, del: isize, text: &str) -> Result<()> {
        if del < 0 {
            anyhow::bail!("splice_text: del must be non-negative (got {del})");
        }
        let del_len = del as usize;
        if del_len == 0 && text.is_empty() {
            return Ok(());
        }
        let handle = self.text_handle();
        if del_len > 0 {
            handle.delete(pos, del_len).context("loro text delete")?;
        }
        if !text.is_empty() {
            handle.insert(pos, text).context("loro text insert")?;
        }
        self.doc.commit();
        Ok(())
    }

    pub fn mark(&mut self, mark_type: &MarkType, start: usize, end: usize) -> Result<()> {
        self.text_handle()
            .mark(start..end, mark_type.name(), scalar_to_loro(mark_type.scalar_value()))
            .with_context(|| format!("loro mark {}", mark_type.name()))?;
        self.doc.commit();
        Ok(())
    }

    pub fn unmark(&mut self, mark_type: &MarkType, start: usize, end: usize) -> Result<()> {
        self.text_handle()
            .unmark(start..end, mark_type.name())
            .with_context(|| format!("loro unmark {}", mark_type.name()))?;
        self.doc.commit();
        Ok(())
    }

    pub fn to_markdown(&self) -> Result<String> {
        let text = self.text()?;
        if text.is_empty() {
            return Ok(String::new());
        }
        Ok(serialize_to_markdown(&text, &self.marks()?))
    }

    pub fn save(&mut self) -> Vec<u8> {
        self.doc
            .export(ExportMode::snapshot())
            .expect("export loro snapshot")
    }

    /// Fork into an independent document with the same history (snapshot
    /// round-trip).
    pub fn fork(&mut self) -> LoroCrdtDocument {
        let bytes = self.save();
        Self::load(&bytes).expect("fork via snapshot")
    }

    /// Merge another document's ops into this one (conflict-free).
    pub fn merge(&mut self, other: &LoroCrdtDocument) -> Result<()> {
        let patch = other
            .doc
            .export(ExportMode::all_updates())
            .context("export loro updates for merge")?;
        self.doc.import(&patch).context("import loro updates on merge")?;
        Ok(())
    }
}

fn nesting(name: &str) -> u8 {
    MarkType::from_name(name)
        .map(|m| m.nesting_order())
        .unwrap_or(u8::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(markdown: &str) -> String {
        LoroCrdtDocument::from_markdown(markdown)
            .unwrap()
            .to_markdown()
            .unwrap()
    }

    #[test]
    fn empty_document() {
        let d = LoroCrdtDocument::new().unwrap();
        assert_eq!(d.text().unwrap(), "");
        assert_eq!(d.to_markdown().unwrap(), "");
    }

    #[test]
    fn plain_text_round_trip() {
        assert_eq!(md("Hello world\n"), "Hello world\n");
    }

    #[test]
    fn bold_round_trip() {
        assert_eq!(md("This is **bold** text\n"), "This is **bold** text\n");
    }

    #[test]
    fn italic_and_code_round_trip() {
        assert_eq!(md("*em* and `code`\n"), "*em* and `code`\n");
    }

    #[test]
    fn wikilink_round_trip() {
        assert_eq!(md("see [[Target|alias]] here\n"), "see [[Target|alias]] here\n");
    }

    #[test]
    fn headings_and_lists_are_literal_text() {
        let src = "# Title\n\n- one\n- two\n";
        assert_eq!(md(src), src);
    }

    #[test]
    fn splice_inserts_and_deletes() {
        let mut d = LoroCrdtDocument::from_markdown("Hello world\n").unwrap();
        d.splice_text(5, 0, ",").unwrap();
        assert_eq!(d.text().unwrap(), "Hello, world\n");
        d.splice_text(0, 5, "Howdy").unwrap();
        assert_eq!(d.text().unwrap(), "Howdy, world\n");
    }

    #[test]
    fn splice_rejects_negative_delete() {
        let mut d = LoroCrdtDocument::new().unwrap();
        assert!(d.splice_text(0, -1, "x").is_err());
    }

    #[test]
    fn unmark_removes_mark() {
        let mut d = LoroCrdtDocument::from_markdown("bold text\n").unwrap();
        d.mark(&MarkType::Bold, 0, 4).unwrap();
        assert_eq!(d.to_markdown().unwrap(), "**bold** text\n");
        d.unmark(&MarkType::Bold, 0, 4).unwrap();
        assert_eq!(d.to_markdown().unwrap(), "bold text\n");
    }

    #[test]
    fn save_load_content_equivalent() {
        let mut a = LoroCrdtDocument::from_markdown("**bold** and *em*\n").unwrap();
        let bytes = a.save();
        let b = LoroCrdtDocument::load(&bytes).unwrap();
        assert_eq!(a.text().unwrap(), b.text().unwrap());
        assert_eq!(a.to_markdown().unwrap(), b.to_markdown().unwrap());
    }

    #[test]
    fn marks_survive_save_load() {
        let mut a = LoroCrdtDocument::from_markdown("**bold** text\n").unwrap();
        let bytes = a.save();
        let b = LoroCrdtDocument::load(&bytes).unwrap();
        assert_eq!(a.marks().unwrap(), b.marks().unwrap());
        assert_eq!(b.to_markdown().unwrap(), "**bold** text\n");
    }

    #[test]
    fn fork_diverges_independently() {
        let mut a = LoroCrdtDocument::from_markdown("shared\n").unwrap();
        let mut b = a.fork();
        a.splice_text(0, 0, "A ").unwrap();
        b.splice_text(0, 0, "B ").unwrap();
        assert_eq!(a.text().unwrap(), "A shared\n");
        assert_eq!(b.text().unwrap(), "B shared\n");
    }

    #[test]
    fn merge_concurrent_edits_converges() {
        let mut a = LoroCrdtDocument::from_markdown("base\n").unwrap();
        let mut b = a.fork();
        a.splice_text(4, 0, " A").unwrap();
        b.splice_text(0, 0, "B ").unwrap();
        a.merge(&b).unwrap();
        b.merge(&a).unwrap();
        assert_eq!(a.text().unwrap(), b.text().unwrap());
    }

    #[test]
    fn merge_preserves_marks_from_both_sides() {
        let mut a = LoroCrdtDocument::from_markdown("hello world\n").unwrap();
        let mut b = a.fork();
        a.mark(&MarkType::Bold, 0, 5).unwrap();
        b.mark(&MarkType::Italic, 6, 11).unwrap();
        a.merge(&b).unwrap();
        let out = a.to_markdown().unwrap();
        assert!(out.contains("**hello**"), "bold from A: {out}");
        assert!(out.contains("*world*"), "italic from B: {out}");
    }
}
