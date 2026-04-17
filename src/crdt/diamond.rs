//! Diamond-types CRDT backend (IMPL-029 Phase 5).
//!
//! Text lives on a `diamond_types::list::OpLog`; marks live on a sibling
//! [`MarksDoc`] oplog that stores span-level ops as newline-delimited JSON
//! (see `research/diamond-types-marks.md`). `splice_text` updates both
//! atomically — every text splice emits a paired `Shift` on the marks oplog
//! so concurrent mark / text edits converge across replicas.
//!
//! Save format: framed `u32 text_len + text_bytes + marks_bytes`, both halves
//! in diamond-types' native binary encoding. Fork/merge cover both halves
//! symmetrically; every fork allocates a fresh agent name because
//! diamond-types corrupts on reused `(agent, seq)` tuples across divergent
//! branches.

use std::any::Any;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
use diamond_types::list::encoding::EncodeOptions;
use diamond_types::list::OpLog;
use diamond_types::AgentId;

use crate::crdt::backend::CrdtBackend;
use crate::crdt::blocks::BlockToken;
use crate::crdt::marks::{parse_inline_marks, serialize_to_markdown, ExpandMark, Mark, MarkType};
use crate::crdt::marks_doc::MarksDoc;

/// Default agent name used for the local writer.
const AGENT_NAME: &str = "zetl";

/// Monotonic process-global counter appended to forked agent names.
/// Diamond-types corrupts on reused `(agent, seq)` tuples across divergent
/// branches, so every `fork` call allocates a fresh name.
static FORK_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Diamond-types CRDT document.
pub struct DiamondCrdtDocument {
    oplog: OpLog,
    marks: MarksDoc,
    /// `None` until the first write. Lazy registration keeps `load → save`
    /// byte-identical — if we registered the agent eagerly on `load`, an
    /// unchanged round-trip would grow the agent table.
    agent: Option<AgentId>,
    /// Name under which the local writer will be registered on the text
    /// oplog. [`MarksDoc`] is constructed with the same name and registers
    /// independently on its own oplog.
    agent_name: String,
}

impl DiamondCrdtDocument {
    fn agent(&mut self) -> AgentId {
        if let Some(a) = self.agent {
            return a;
        }
        let a = self.oplog.get_or_create_agent_id(&self.agent_name);
        self.agent = Some(a);
        a
    }

    /// Ingest a markdown document: insert every line's plain text (with
    /// block prefixes preserved) and then apply inline marks.
    ///
    /// Text is inserted first and marks are applied afterwards so that
    /// inclusive marks at line boundaries never grow to absorb the
    /// structural newlines we inject between lines.
    fn load_markdown(&mut self, markdown: &str) -> Result<()> {
        let lines: Vec<&str> = markdown.lines().collect();
        let mut pending_marks: Vec<(MarkType, usize, usize, ExpandMark)> = Vec::new();
        let mut pos: usize = 0;
        let mut in_code_fence = false;
        let mut in_frontmatter = false;
        let mut is_first_line = true;

        for (line_idx, line) in lines.iter().enumerate() {
            if line_idx > 0 {
                <Self as CrdtBackend>::splice_text(self, pos, 0, "\n")?;
                pos += 1;
            }

            // Frontmatter boundary — atomic token, no marks.
            if *line == "---" && (is_first_line || in_frontmatter) {
                <Self as CrdtBackend>::splice_text(self, pos, 0, line)?;
                pos += line.chars().count();
                in_frontmatter = is_first_line;
                is_first_line = false;
                continue;
            }
            is_first_line = false;

            if in_frontmatter {
                <Self as CrdtBackend>::splice_text(self, pos, 0, line)?;
                pos += line.chars().count();
                continue;
            }

            // Code fence boundary toggles opaque mode.
            if line.starts_with("```") {
                <Self as CrdtBackend>::splice_text(self, pos, 0, line)?;
                pos += line.chars().count();
                in_code_fence = !in_code_fence;
                continue;
            }

            if in_code_fence {
                <Self as CrdtBackend>::splice_text(self, pos, 0, line)?;
                pos += line.chars().count();
                continue;
            }

            if line.is_empty() {
                continue;
            }

            if let Some((block_token, content)) = BlockToken::parse_line_prefix(line) {
                let prefix = block_token.to_markdown();
                <Self as CrdtBackend>::splice_text(self, pos, 0, &prefix)?;
                pos += prefix.chars().count();
                pos = self.insert_inline(content, pos, &mut pending_marks)?;
            } else {
                pos = self.insert_inline(line, pos, &mut pending_marks)?;
            }
        }

        // Trailing newline invariant (REQ-020-027).
        let text = <Self as CrdtBackend>::text(self)?;
        if !text.is_empty() && !text.ends_with('\n') {
            <Self as CrdtBackend>::splice_text(self, pos, 0, "\n")?;
        }

        for (mt, start, end, _expand) in pending_marks {
            <Self as CrdtBackend>::mark(self, &mt, start, end)?;
        }

        Ok(())
    }

    fn insert_inline(
        &mut self,
        line: &str,
        start_pos: usize,
        pending: &mut Vec<(MarkType, usize, usize, ExpandMark)>,
    ) -> Result<usize> {
        let parsed = parse_inline_marks(line);
        <Self as CrdtBackend>::splice_text(self, start_pos, 0, &parsed.plain_text)?;
        for m in parsed.marks {
            let expand = m.mark_type.expand();
            pending.push((m.mark_type, start_pos + m.start, start_pos + m.end, expand));
        }
        Ok(start_pos + parsed.plain_text.chars().count())
    }
}

/// Zip the text + marks oplog encodings into a single framed blob.
///
/// Wire format: `u32 (little-endian) text_len | text_bytes | marks_bytes`.
/// The text length is the only frame delimiter we need — the remainder of
/// the buffer is the marks oplog.
fn encode_framed(text: &[u8], marks: &[u8]) -> Vec<u8> {
    let text_len = text.len();
    assert!(
        text_len <= u32::MAX as usize,
        "text oplog exceeds u32 frame (got {text_len} bytes)"
    );
    let mut out = Vec::with_capacity(4 + text_len + marks.len());
    out.extend_from_slice(&(text_len as u32).to_le_bytes());
    out.extend_from_slice(text);
    out.extend_from_slice(marks);
    out
}

fn decode_framed(data: &[u8]) -> Result<(&[u8], &[u8])> {
    if data.len() < 4 {
        anyhow::bail!("diamond framed blob too short ({}B)", data.len());
    }
    let text_len = u32::from_le_bytes(data[0..4].try_into().expect("4-byte slice")) as usize;
    if 4 + text_len > data.len() {
        anyhow::bail!(
            "diamond framed blob: text_len {text_len} exceeds payload {}B",
            data.len() - 4
        );
    }
    Ok((&data[4..4 + text_len], &data[4 + text_len..]))
}

impl CrdtBackend for DiamondCrdtDocument {
    fn new() -> Result<Self> {
        Ok(Self {
            oplog: OpLog::new(),
            marks: MarksDoc::new(AGENT_NAME.to_string()),
            agent: None,
            agent_name: AGENT_NAME.to_string(),
        })
    }

    fn from_markdown(markdown: &str) -> Result<Self> {
        let mut this = <Self as CrdtBackend>::new()?;
        this.load_markdown(markdown)?;
        Ok(this)
    }

    fn load(data: &[u8]) -> Result<Self> {
        let (text_bytes, marks_bytes) = decode_framed(data)?;
        let oplog = OpLog::load_from(text_bytes)
            .map_err(|e| anyhow::anyhow!("load diamond oplog: {e:?}"))?;
        let marks = MarksDoc::load_from(marks_bytes, AGENT_NAME.to_string())?;
        Ok(Self {
            oplog,
            marks,
            agent: None,
            agent_name: AGENT_NAME.to_string(),
        })
    }

    fn text(&self) -> Result<String> {
        Ok(self.oplog.checkout_tip().content().to_string())
    }

    fn marks(&self) -> Result<Vec<Mark>> {
        Ok(self.marks.materialise())
    }

    fn splice_text(&mut self, pos: usize, del: isize, text: &str) -> Result<()> {
        if del < 0 {
            anyhow::bail!("splice_text: del must be non-negative (got {del})");
        }
        let del_len = del as usize;
        if del_len == 0 && text.is_empty() {
            return Ok(());
        }
        let agent = self.agent();
        if del_len > 0 {
            self.oplog
                .add_delete_without_content(agent, pos..pos + del_len);
            self.marks.shift(pos, -(del_len as i64))?;
        }
        if !text.is_empty() {
            self.oplog.add_insert(agent, pos, text);
            let n = text.chars().count() as i64;
            self.marks.shift(pos, n)?;
        }
        Ok(())
    }

    fn mark(&mut self, mark_type: &MarkType, start: usize, end: usize) -> Result<()> {
        self.marks.mark(
            mark_type.name(),
            mark_type.scalar_value(),
            start,
            end,
            mark_type.expand(),
        )
    }

    fn unmark(&mut self, mark_type: &MarkType, start: usize, end: usize) -> Result<()> {
        self.marks
            .unmark(mark_type.name(), start, end, mark_type.expand())
    }

    fn to_markdown(&self) -> Result<String> {
        let text = <Self as CrdtBackend>::text(self)?;
        if text.is_empty() {
            return Ok(String::new());
        }
        let marks = <Self as CrdtBackend>::marks(self)?;
        Ok(serialize_to_markdown(&text, &marks))
    }

    fn save(&mut self) -> Vec<u8> {
        let text = self.oplog.encode(EncodeOptions::default());
        let marks = self.marks.encode();
        encode_framed(&text, &marks)
    }

    fn fork(&mut self) -> Box<dyn CrdtBackend> {
        let seq = FORK_COUNTER.fetch_add(1, Ordering::Relaxed);
        let name = format!("{AGENT_NAME}-fork-{seq:x}");
        Box::new(Self {
            oplog: self.oplog.clone(),
            marks: self.marks.clone_with_agent(name.clone()),
            agent: None,
            agent_name: name,
        })
    }

    fn merge(&mut self, other: &mut dyn CrdtBackend) -> Result<()> {
        let other = other
            .as_any_mut()
            .downcast_mut::<DiamondCrdtDocument>()
            .context("merge: incompatible CRDT backend")?;
        let text_patch = other.oplog.encode(EncodeOptions::default());
        self.oplog
            .decode_and_add(&text_patch)
            .map_err(|e| anyhow::anyhow!("merge diamond oplog: {e:?}"))?;
        let marks_patch = other.marks.encode();
        self.marks.decode_and_add(&marks_patch)?;
        Ok(())
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crdt::marks::Scalar;

    #[test]
    fn empty_document() {
        let doc = DiamondCrdtDocument::new().unwrap();
        assert_eq!(doc.text().unwrap(), "");
        assert_eq!(doc.to_markdown().unwrap(), "");
        assert!(doc.marks().unwrap().is_empty());
    }

    #[test]
    fn from_markdown_round_trips_with_marks() {
        let md = "# Heading\n\n**bold** and *italic*\n";
        let doc = DiamondCrdtDocument::from_markdown(md).unwrap();
        // Raw text has the inline mark syntax stripped.
        assert_eq!(doc.text().unwrap(), "# Heading\n\nbold and italic\n");
        // Marks were extracted while parsing.
        let marks = doc.marks().unwrap();
        let mark_names: Vec<&str> = marks.iter().map(|m| m.name.as_str()).collect();
        assert!(mark_names.contains(&"bold"));
        assert!(mark_names.contains(&"italic"));
        // Serializing back re-emits the original markdown.
        assert_eq!(doc.to_markdown().unwrap(), md);
    }

    #[test]
    fn splice_inserts_and_deletes() {
        let mut doc = DiamondCrdtDocument::new().unwrap();
        doc.splice_text(0, 0, "Hello world").unwrap();
        assert_eq!(doc.text().unwrap(), "Hello world");

        doc.splice_text(5, 6, "").unwrap();
        assert_eq!(doc.text().unwrap(), "Hello");

        doc.splice_text(2, 3, "lp").unwrap();
        assert_eq!(doc.text().unwrap(), "Help");
    }

    #[test]
    fn splice_rejects_negative_delete() {
        let mut doc = DiamondCrdtDocument::new().unwrap();
        let err = doc.splice_text(0, -1, "x").unwrap_err();
        assert!(err.to_string().contains("non-negative"));
    }

    #[test]
    fn multibyte_char_positions() {
        // `from_markdown` also enforces the trailing-newline invariant
        // (REQ-020-027), so the loaded text ends with `\n`.
        let mut doc = DiamondCrdtDocument::from_markdown("café — 🌊").unwrap();
        doc.splice_text(5, 0, "[after é] ").unwrap();
        assert_eq!(doc.text().unwrap(), "café [after é] — 🌊\n");
    }

    #[test]
    fn save_load_content_equivalent() {
        let md = "Hello world\nSecond line\n";
        let mut doc = DiamondCrdtDocument::from_markdown(md).unwrap();
        let bytes = doc.save();

        let loaded = DiamondCrdtDocument::load(&bytes).unwrap();
        assert_eq!(loaded.text().unwrap(), md);
        assert_eq!(loaded.to_markdown().unwrap(), md);
    }

    #[test]
    fn save_load_byte_identical_empty() {
        let mut doc = DiamondCrdtDocument::new().unwrap();
        let bytes1 = doc.save();

        let mut loaded = DiamondCrdtDocument::load(&bytes1).unwrap();
        let bytes2 = loaded.save();
        assert_eq!(bytes1, bytes2, "save(load(empty)) must be byte-identical");
    }

    #[test]
    fn save_load_byte_identical_with_content() {
        let md = "The quick brown fox jumps over the lazy dog.\n";
        let mut doc = DiamondCrdtDocument::from_markdown(md).unwrap();
        doc.splice_text(10, 5, "red").unwrap();
        let bytes1 = doc.save();

        let mut loaded = DiamondCrdtDocument::load(&bytes1).unwrap();
        let bytes2 = loaded.save();
        assert_eq!(bytes1, bytes2);
        assert_eq!(
            loaded.text().unwrap(),
            "The quick red fox jumps over the lazy dog.\n"
        );
    }

    #[test]
    fn marks_survive_save_load() {
        let mut doc = DiamondCrdtDocument::from_markdown("Hello world").unwrap();
        doc.mark(&MarkType::Bold, 0, 5).unwrap();
        doc.mark(
            &MarkType::Wikilink {
                target: "Target".into(),
                alias: None,
            },
            6,
            11,
        )
        .unwrap();

        let bytes = doc.save();
        let loaded = DiamondCrdtDocument::load(&bytes).unwrap();
        let marks = loaded.marks().unwrap();
        assert_eq!(marks.len(), 2);
        assert!(marks.iter().any(|m| m.name == "bold"));
        assert!(marks.iter().any(|m| m.name == "wikilink"));
    }

    #[test]
    fn inclusive_mark_grows_at_end_boundary() {
        let mut doc = DiamondCrdtDocument::from_markdown("Hello").unwrap();
        doc.mark(&MarkType::Bold, 0, 5).unwrap();
        doc.splice_text(5, 0, "!").unwrap();

        let marks = doc.marks().unwrap();
        let bold = marks.iter().find(|m| m.name == "bold").unwrap();
        assert_eq!(bold.end, 6, "inclusive bold should grow to cover `!`");
    }

    #[test]
    fn non_growing_mark_preserved_at_end_boundary() {
        // "Hello World" — wikilink over "World" (chars 6..11); typing " foo" at
        // 11 must NOT extend the wikilink mark.
        let mut doc = DiamondCrdtDocument::from_markdown("Hello World").unwrap();
        doc.mark(
            &MarkType::Wikilink {
                target: "World".into(),
                alias: None,
            },
            6,
            11,
        )
        .unwrap();
        doc.splice_text(11, 0, " foo").unwrap();

        let marks = doc.marks().unwrap();
        let wl = marks.iter().find(|m| m.name == "wikilink").unwrap();
        assert_eq!(wl.end, 11, "non-growing wikilink must stay at 11");
    }

    #[test]
    fn unmark_removes_mark() {
        let mut doc = DiamondCrdtDocument::from_markdown("Hello").unwrap();
        doc.mark(&MarkType::Bold, 0, 5).unwrap();
        assert_eq!(doc.marks().unwrap().len(), 1);

        doc.unmark(&MarkType::Bold, 0, 5).unwrap();
        assert!(doc.marks().unwrap().is_empty());
    }

    #[test]
    fn fork_diverges_independently() {
        let mut doc = DiamondCrdtDocument::from_markdown("Hello world\n").unwrap();
        let mut fork = doc.fork();

        doc.splice_text(5, 0, " there").unwrap();
        fork.splice_text(11, 0, "!").unwrap();

        assert_eq!(doc.text().unwrap(), "Hello there world\n");
        assert_eq!(fork.text().unwrap(), "Hello world!\n");
    }

    #[test]
    fn merge_concurrent_edits() {
        let mut doc = DiamondCrdtDocument::from_markdown("Hello world\n").unwrap();
        let mut fork = doc.fork();

        doc.splice_text(0, 0, "Hi, ").unwrap();
        fork.splice_text(11, 0, "!").unwrap();

        doc.merge(&mut *fork).unwrap();

        let out = doc.text().unwrap();
        assert!(out.starts_with("Hi, Hello world"), "got: {out:?}");
        assert!(out.contains('!'), "got: {out:?}");
    }

    #[test]
    fn merge_preserves_marks_from_both_sides() {
        let mut doc = DiamondCrdtDocument::from_markdown("Hello world\n").unwrap();
        let mut fork = doc.fork();

        doc.mark(&MarkType::Bold, 0, 5).unwrap();
        fork.mark(&MarkType::Italic, 6, 11).unwrap();

        doc.merge(&mut *fork).unwrap();

        let marks = doc.marks().unwrap();
        assert!(marks.iter().any(|m| m.name == "bold"));
        assert!(marks.iter().any(|m| m.name == "italic"));
    }

    #[test]
    fn same_type_merge_is_symmetric() {
        let mut a = DiamondCrdtDocument::from_markdown("a").unwrap();
        let mut b = DiamondCrdtDocument::from_markdown("b").unwrap();
        a.merge(&mut b as &mut dyn CrdtBackend).unwrap();
    }

    #[test]
    fn exclusive_marks_lww_on_concurrent_overlap() {
        let mut doc = DiamondCrdtDocument::from_markdown("Project X").unwrap();
        let mut fork = doc.fork();

        doc.mark(
            &MarkType::Wikilink {
                target: "A".into(),
                alias: None,
            },
            0,
            9,
        )
        .unwrap();
        fork.mark(
            &MarkType::Wikilink {
                target: "B".into(),
                alias: None,
            },
            0,
            9,
        )
        .unwrap();

        doc.merge(&mut *fork).unwrap();

        let marks = doc.marks().unwrap();
        // Exactly one wikilink survives — the LWW winner in canonical replay
        // order. Which wins is deterministic but depends on DT's agent
        // ordering; we only assert the count here.
        let wikilinks: Vec<_> = marks.iter().filter(|m| m.name == "wikilink").collect();
        assert_eq!(
            wikilinks.len(),
            1,
            "exclusive marks LWW: expect 1, got {marks:?}",
        );
        // And the surviving value is one of the two we set.
        let v = &wikilinks[0].value;
        assert!(v == &Scalar::Str("A".into()) || v == &Scalar::Str("B".into()));
    }

    // ── SPEC-020 markdown conformance (REQ-020-024 … REQ-020-029) ────
    //
    // Markdown ingestion, inline-mark round-trip, block-token round-trip,
    // and multi-client convergence — exercised against the diamond-types
    // backend as the only implementation after IMPL-029 Phase 7.

    #[test]
    fn plain_text_round_trip() {
        let md = "Hello world\n";
        let doc = DiamondCrdtDocument::from_markdown(md).unwrap();
        assert_eq!(doc.to_markdown().unwrap(), md);
    }

    #[test]
    fn bold_round_trip() {
        let md = "Some **bold** text\n";
        let doc = DiamondCrdtDocument::from_markdown(md).unwrap();
        assert_eq!(doc.to_markdown().unwrap(), md);
    }

    #[test]
    fn italic_round_trip() {
        let md = "Some *italic* text\n";
        let doc = DiamondCrdtDocument::from_markdown(md).unwrap();
        assert_eq!(doc.to_markdown().unwrap(), md);
    }

    #[test]
    fn code_round_trip() {
        let md = "Some `code` text\n";
        let doc = DiamondCrdtDocument::from_markdown(md).unwrap();
        assert_eq!(doc.to_markdown().unwrap(), md);
    }

    #[test]
    fn wikilink_round_trip() {
        let md = "See [[Project X]] for details\n";
        let doc = DiamondCrdtDocument::from_markdown(md).unwrap();
        assert_eq!(doc.to_markdown().unwrap(), md);
    }

    #[test]
    fn wikilink_with_alias_round_trip() {
        let md = "See [[Project X|the project]] for details\n";
        let doc = DiamondCrdtDocument::from_markdown(md).unwrap();
        assert_eq!(doc.to_markdown().unwrap(), md);
    }

    #[test]
    fn strikethrough_round_trip() {
        let md = "Some ~~deleted~~ text\n";
        let doc = DiamondCrdtDocument::from_markdown(md).unwrap();
        assert_eq!(doc.to_markdown().unwrap(), md);
    }

    #[test]
    fn highlight_round_trip() {
        let md = "Some ==highlighted== text\n";
        let doc = DiamondCrdtDocument::from_markdown(md).unwrap();
        assert_eq!(doc.to_markdown().unwrap(), md);
    }

    #[test]
    fn comment_round_trip() {
        let md = "Some %%hidden%% text\n";
        let doc = DiamondCrdtDocument::from_markdown(md).unwrap();
        assert_eq!(doc.to_markdown().unwrap(), md);
    }

    #[test]
    fn md_link_round_trip() {
        let md = "Click [here](https://example.com) now\n";
        let doc = DiamondCrdtDocument::from_markdown(md).unwrap();
        assert_eq!(doc.to_markdown().unwrap(), md);
    }

    #[test]
    fn heading_round_trip() {
        let md = "## My Heading\n";
        let doc = DiamondCrdtDocument::from_markdown(md).unwrap();
        assert_eq!(doc.to_markdown().unwrap(), md);
    }

    #[test]
    fn code_fence_round_trip() {
        let md = "```spl\naccess(alice, read).\n```\n";
        let doc = DiamondCrdtDocument::from_markdown(md).unwrap();
        assert_eq!(doc.to_markdown().unwrap(), md);
    }

    #[test]
    fn list_item_round_trip() {
        let md = "- first item\n- second item\n";
        let doc = DiamondCrdtDocument::from_markdown(md).unwrap();
        assert_eq!(doc.to_markdown().unwrap(), md);
    }

    #[test]
    fn ordered_list_round_trip() {
        let md = "1. first\n2. second\n3. third\n";
        let doc = DiamondCrdtDocument::from_markdown(md).unwrap();
        assert_eq!(doc.to_markdown().unwrap(), md);
    }

    #[test]
    fn frontmatter_round_trip() {
        let md = "---\ntitle: Test\ntags: [a, b]\n---\n\n## Content\n";
        let doc = DiamondCrdtDocument::from_markdown(md).unwrap();
        assert_eq!(doc.to_markdown().unwrap(), md);
    }

    #[test]
    fn mixed_marks_round_trip() {
        // TEST-020-025: bold, italic, wikilink
        let md = "**bold** and *italic* and [[Link]]\n";
        let doc = DiamondCrdtDocument::from_markdown(md).unwrap();

        let marks = doc.marks().unwrap();
        let mark_names: Vec<&str> = marks.iter().map(|m| m.name.as_str()).collect();
        assert!(mark_names.contains(&"bold"));
        assert!(mark_names.contains(&"italic"));
        assert!(mark_names.contains(&"wikilink"));

        let bold_mark = marks.iter().find(|m| m.name == "bold").unwrap();
        let mt = MarkType::from_mark(&bold_mark.name, &bold_mark.value).unwrap();
        assert!(mt.is_inclusive());

        let wikilink_mark = marks.iter().find(|m| m.name == "wikilink").unwrap();
        let mt = MarkType::from_mark(&wikilink_mark.name, &wikilink_mark.value).unwrap();
        assert!(!mt.is_inclusive());

        assert_eq!(doc.to_markdown().unwrap(), md);
    }

    #[test]
    fn code_mark_is_non_growing() {
        let md = "Use `code` here\n";
        let doc = DiamondCrdtDocument::from_markdown(md).unwrap();

        let marks = doc.marks().unwrap();
        let code_mark = marks.iter().find(|m| m.name == "code").unwrap();
        let mt = MarkType::from_mark(&code_mark.name, &code_mark.value).unwrap();
        assert_eq!(mt.expand(), ExpandMark::None);
    }

    #[test]
    fn block_heading_is_atomic() {
        // TEST-020-026: heading prefix is atomic
        let md = "## Heading\n\nParagraph text\n";
        let doc = DiamondCrdtDocument::from_markdown(md).unwrap();
        let text = doc.text().unwrap();
        assert!(text.starts_with("## Heading"));
        assert_eq!(doc.to_markdown().unwrap(), md);
    }

    #[test]
    fn code_fence_is_opaque() {
        // TEST-020-026: code fence content is plain text, no formatting
        let md = "```spl\n**not bold** and *not italic*\n```\n";
        let doc = DiamondCrdtDocument::from_markdown(md).unwrap();

        let marks = doc.marks().unwrap();
        assert!(
            marks.is_empty(),
            "Code fence content should have no marks, got: {:?}",
            marks.iter().map(|m| m.name.as_str()).collect::<Vec<_>>()
        );

        assert_eq!(doc.to_markdown().unwrap(), md);
    }

    #[test]
    fn deterministic_serialization() {
        // TEST-020-027: same state always produces byte-identical output
        let md = "**bold** and *italic* text\n";
        let doc = DiamondCrdtDocument::from_markdown(md).unwrap();
        assert_eq!(doc.to_markdown().unwrap(), doc.to_markdown().unwrap());
    }

    #[test]
    fn concurrent_edit_merge() {
        let md = "Hello world\n";
        let mut doc = DiamondCrdtDocument::from_markdown(md).unwrap();
        let mut fork = doc.fork();

        // Alice bolds "Hello" (chars 0..5)
        doc.mark(&MarkType::Bold, 0, 5).unwrap();
        // Bob italicizes "world" (chars 6..11)
        fork.mark(&MarkType::Italic, 6, 11).unwrap();

        doc.merge(&mut *fork).unwrap();

        let out = doc.to_markdown().unwrap();
        assert!(out.contains("**Hello**"), "got: {out:?}");
        assert!(out.contains("*world*"), "got: {out:?}");
    }

    #[test]
    fn wikilink_non_growing_behavior() {
        // TEST-020-024: typing after ]] is plain text
        let md = "See [[Project X]] here\n";
        let mut doc = DiamondCrdtDocument::from_markdown(md).unwrap();

        let marks = doc.marks().unwrap();
        let wl = marks.iter().find(|m| m.name == "wikilink").unwrap();
        let wl_end = wl.end;
        drop(marks);

        doc.splice_text(wl_end, 0, " is great").unwrap();

        let marks_after = doc.marks().unwrap();
        let wl_after = marks_after.iter().find(|m| m.name == "wikilink").unwrap();
        assert_eq!(
            wl_after.end, wl_end,
            "Wikilink should not grow when text is inserted at boundary"
        );
    }

    #[test]
    fn save_and_load_round_trip() {
        let md = "**bold** and [[Link]]\n";
        let mut doc = DiamondCrdtDocument::from_markdown(md).unwrap();
        let bytes = doc.save();

        let loaded = DiamondCrdtDocument::load(&bytes).unwrap();
        assert_eq!(loaded.to_markdown().unwrap(), md);
    }

    #[test]
    fn test_020_027_canonical_serialization_after_concurrent_edits() {
        // CRDT text: "Hello world\n\nSome text here\n"
        //             0123456789012 3456789012345678
        // "Some" = 13..17, "text" = 18..22
        let md = "Hello world\n\nSome text here\n";
        let mut doc = DiamondCrdtDocument::from_markdown(md).unwrap();
        let mut fork = doc.fork();

        // Alice bolds "Hello" (0..5)
        doc.mark(&MarkType::Bold, 0, 5).unwrap();

        // Bob italicizes "text" (18..22) and strikethroughs "Some" (13..17)
        fork.mark(&MarkType::Italic, 18, 22).unwrap();
        fork.mark(&MarkType::Strikethrough, 13, 17).unwrap();

        doc.merge(&mut *fork).unwrap();

        let out1 = doc.to_markdown().unwrap();
        let out2 = doc.to_markdown().unwrap();
        assert_eq!(out1, out2, "serialization must be deterministic");

        assert!(out1.contains("**Hello**"), "got: {out1:?}");
        assert!(out1.contains("*text*"), "got: {out1:?}");
        assert!(out1.contains("~~Some~~"), "got: {out1:?}");
    }

    #[test]
    fn parse_serialize_round_trip_equivalence() {
        let md =
            "## Heading\n\n**bold** and *italic* with [[Link]] and `code`\n\n- list ~~item~~\n";
        let doc = DiamondCrdtDocument::from_markdown(md).unwrap();

        let serialized = doc.to_markdown().unwrap();
        let doc2 = DiamondCrdtDocument::from_markdown(&serialized).unwrap();
        let serialized2 = doc2.to_markdown().unwrap();
        assert_eq!(
            serialized, serialized2,
            "parse(serialize(state)) must round-trip"
        );
    }

    #[test]
    fn nested_marks_canonical_order() {
        // strikethrough > bold > italic (outermost → innermost)
        let md = "~~**text**~~\n";
        let doc = DiamondCrdtDocument::from_markdown(md).unwrap();
        assert_eq!(doc.to_markdown().unwrap(), md);
    }

    #[test]
    fn cache_md_loads_with_multibyte_chars() {
        // Regression guard: embedded em-dash used to trip up char- vs
        // byte-indexed splices.
        let content = std::fs::read_to_string("demo-vault/architecture/Cache.md").unwrap();
        let doc = DiamondCrdtDocument::from_markdown(&content);
        assert!(doc.is_ok(), "Cache.md should load: {:?}", doc.err());
    }

    #[test]
    fn save_bounded_for_keystroke_traces() {
        // Coarse sanity-check: 10x-of-plaintext bound covers the marks oplog
        // overhead (one Shift op per keystroke as JSON). Tighter bounds would
        // require the compaction pass deferred to Phase 6.
        let mut doc = DiamondCrdtDocument::new().unwrap();
        let filler = "The sixteenth-century Venetian typographer Aldus Manutius set \
                      small italic forms to fit more words on a page; we just keep typing.";
        for (i, ch) in filler.chars().enumerate() {
            doc.splice_text(i, 0, &ch.to_string()).unwrap();
        }
        let bytes = doc.save();
        assert!(
            bytes.len() < filler.len() * 10,
            "diamond save was {} bytes for {} plain chars",
            bytes.len(),
            filler.chars().count()
        );
    }
}
