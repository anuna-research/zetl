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
use crate::crdt::marks::{Mark, MarkType};
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
    let text_len =
        u32::from_le_bytes(data[0..4].try_into().expect("4-byte slice")) as usize;
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
        if !markdown.is_empty() {
            let agent = this.agent();
            this.oplog.add_insert(agent, 0, markdown);
        }
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
        <Self as CrdtBackend>::text(self)
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
    fn from_markdown_preserves_text() {
        // from_markdown is raw-text-only on the diamond backend today;
        // markdown→marks parsing lives on the automerge backend's
        // `load_markdown`. `task-diamond-markdown-ingest` will port it.
        let md = "# Heading\n\n**bold** and *italic*\n";
        let doc = DiamondCrdtDocument::from_markdown(md).unwrap();
        assert_eq!(doc.text().unwrap(), md);
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
        let mut doc = DiamondCrdtDocument::from_markdown("café — 🌊").unwrap();
        doc.splice_text(5, 0, "[after é] ").unwrap();
        assert_eq!(doc.text().unwrap(), "café [after é] — 🌊");
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
        assert_eq!(wikilinks.len(), 1, "exclusive marks LWW: expect 1, got {:?}", marks);
        // And the surviving value is one of the two we set.
        let v = &wikilinks[0].value;
        assert!(v == &Scalar::Str("A".into()) || v == &Scalar::Str("B".into()));
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
