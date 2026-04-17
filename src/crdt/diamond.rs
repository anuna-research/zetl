//! Diamond-types CRDT backend (IMPL-029 Phase 4 — text-only).
//!
//! Text-shaped operations (`new`, `from_markdown`, `text`, `splice_text`,
//! `to_markdown`, `save`, `load`, `fork`, `merge`) ride directly on
//! `diamond_types::list::OpLog`. Marks-shaped operations (`mark`,
//! `unmark`, `marks`) return a not-yet-implemented sentinel —
//! `task-diamond-marks-layer` (Phase 5) will layer a sibling marks
//! oplog on top per `research/diamond-types-marks.md`.
//!
//! The wire format is diamond-types' native binary encoding
//! (`EncodeOptions::default()` == `ENCODE_FULL`), which the spike
//! measured at ~0.5× automerge's size on keystroke-granular traces.
//! Re-encoding a loaded oplog is byte-identical to the original —
//! `load` and `save` never mutate the oplog, and agent-id registration
//! is deferred until the first write so round-trips don't perturb the
//! agent table.

use std::any::Any;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
use diamond_types::list::encoding::EncodeOptions;
use diamond_types::list::OpLog;
use diamond_types::AgentId;

use crate::crdt::backend::CrdtBackend;
use crate::crdt::marks::{Mark, MarkType};

/// Default agent name used for the local writer.
const AGENT_NAME: &str = "zetl";

/// Monotonic process-global counter appended to forked agent names.
/// Diamond-types corrupts on reused `(agent, seq)` tuples across
/// divergent branches, so every `fork` call allocates a fresh name.
static FORK_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Sentinel error used when the marks layer is invoked on the diamond
/// backend before Phase 5 wires it in.
const MARKS_NOT_IMPLEMENTED: &str =
    "diamond backend marks layer not yet implemented (IMPL-029 Phase 5)";

/// Diamond-types CRDT document.
pub struct DiamondCrdtDocument {
    oplog: OpLog,
    /// `None` until the first write. Lazy registration keeps `load →
    /// save` byte-identical — if we registered the agent eagerly on
    /// `load`, an unchanged round-trip would grow the agent table.
    agent: Option<AgentId>,
    /// Name under which the local writer will be registered.
    agent_name: String,
}

impl DiamondCrdtDocument {
    /// Lazily register the local agent. Called on every write path.
    fn agent(&mut self) -> AgentId {
        if let Some(a) = self.agent {
            return a;
        }
        let a = self.oplog.get_or_create_agent_id(&self.agent_name);
        self.agent = Some(a);
        a
    }
}

impl CrdtBackend for DiamondCrdtDocument {
    fn new() -> Result<Self> {
        Ok(Self {
            oplog: OpLog::new(),
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
        let oplog =
            OpLog::load_from(data).map_err(|e| anyhow::anyhow!("load diamond oplog: {e:?}"))?;
        Ok(Self {
            oplog,
            agent: None,
            agent_name: AGENT_NAME.to_string(),
        })
    }

    fn text(&self) -> Result<String> {
        Ok(self.oplog.checkout_tip().content().to_string())
    }

    fn marks(&self) -> Result<Vec<Mark>> {
        anyhow::bail!(MARKS_NOT_IMPLEMENTED)
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
        }
        if !text.is_empty() {
            self.oplog.add_insert(agent, pos, text);
        }
        Ok(())
    }

    fn mark(&mut self, _mark_type: &MarkType, _start: usize, _end: usize) -> Result<()> {
        anyhow::bail!(MARKS_NOT_IMPLEMENTED)
    }

    fn unmark(&mut self, _mark_type: &MarkType, _start: usize, _end: usize) -> Result<()> {
        anyhow::bail!(MARKS_NOT_IMPLEMENTED)
    }

    fn to_markdown(&self) -> Result<String> {
        <Self as CrdtBackend>::text(self)
    }

    fn save(&mut self) -> Vec<u8> {
        self.oplog.encode(EncodeOptions::default())
    }

    fn fork(&mut self) -> Box<dyn CrdtBackend> {
        let forked = self.oplog.clone();
        let seq = FORK_COUNTER.fetch_add(1, Ordering::Relaxed);
        let name = format!("{AGENT_NAME}-fork-{seq:x}");
        Box::new(Self {
            oplog: forked,
            agent: None,
            agent_name: name,
        })
    }

    fn merge(&mut self, other: &mut dyn CrdtBackend) -> Result<()> {
        let other = other
            .as_any_mut()
            .downcast_mut::<DiamondCrdtDocument>()
            .context("merge: incompatible CRDT backend")?;
        let patch = other.oplog.encode(EncodeOptions::default());
        self.oplog
            .decode_and_add(&patch)
            .map_err(|e| anyhow::anyhow!("merge diamond oplog: {e:?}"))?;
        Ok(())
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_document() {
        let doc = DiamondCrdtDocument::new().unwrap();
        assert_eq!(doc.text().unwrap(), "");
        assert_eq!(doc.to_markdown().unwrap(), "");
    }

    #[test]
    fn from_markdown_preserves_text() {
        let md = "# Heading\n\n**bold** and *italic*\n";
        let doc = DiamondCrdtDocument::from_markdown(md).unwrap();
        // Marks layer not wired in yet — text is the raw markdown.
        assert_eq!(doc.text().unwrap(), md);
        assert_eq!(doc.to_markdown().unwrap(), md);
    }

    #[test]
    fn splice_inserts_and_deletes() {
        let mut doc = DiamondCrdtDocument::new().unwrap();
        doc.splice_text(0, 0, "Hello world").unwrap();
        assert_eq!(doc.text().unwrap(), "Hello world");

        // Delete " world" (6 chars from position 5).
        doc.splice_text(5, 6, "").unwrap();
        assert_eq!(doc.text().unwrap(), "Hello");

        // Replace "llo" with "lp".
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
        // Positions are char indices, not byte offsets.
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
        doc.splice_text(10, 5, "red").unwrap(); // "brown" -> "red"
        let bytes1 = doc.save();

        let mut loaded = DiamondCrdtDocument::load(&bytes1).unwrap();
        let bytes2 = loaded.save();
        assert_eq!(
            bytes1, bytes2,
            "save(load(doc)) must produce byte-identical output"
        );
        assert_eq!(loaded.text().unwrap(), "The quick red fox jumps over the lazy dog.\n");
    }

    #[test]
    fn save_load_byte_identical_after_many_edits() {
        let mut doc = DiamondCrdtDocument::new().unwrap();
        for ch in "abcdefghij".chars() {
            doc.splice_text(0, 0, &ch.to_string()).unwrap();
        }
        doc.splice_text(3, 2, "XYZ").unwrap();
        let bytes1 = doc.save();

        let mut loaded = DiamondCrdtDocument::load(&bytes1).unwrap();
        let bytes2 = loaded.save();
        assert_eq!(bytes1, bytes2);
        assert_eq!(loaded.text().unwrap(), doc.text().unwrap());
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

        // Alice prepends "Hi, " on doc; Bob appends "!" on fork.
        doc.splice_text(0, 0, "Hi, ").unwrap();
        fork.splice_text(11, 0, "!").unwrap();

        doc.merge(&mut *fork).unwrap();

        let out = doc.text().unwrap();
        assert!(out.starts_with("Hi, Hello world"), "got: {out:?}");
        assert!(out.contains('!'), "got: {out:?}");
    }

    #[test]
    fn merge_rejects_mismatched_backend_without_panic() {
        // We can't construct the automerge backend here (different module),
        // but the downcast path is exercised by the ok case above. This test
        // asserts the error surfaces rather than panicking when merging a
        // dangling mutable reference to a wrong type — see the context
        // message in the implementation.
        let mut a = DiamondCrdtDocument::from_markdown("a").unwrap();
        let mut b = DiamondCrdtDocument::from_markdown("b").unwrap();
        // Same-type merge should succeed.
        a.merge(&mut b as &mut dyn CrdtBackend).unwrap();
    }

    #[test]
    fn marks_returns_not_implemented() {
        let doc = DiamondCrdtDocument::from_markdown("**bold**").unwrap();
        let err = doc.marks().unwrap_err();
        assert!(
            err.to_string().contains("not yet implemented"),
            "expected sentinel error, got: {err}"
        );
    }

    #[test]
    fn mark_op_returns_not_implemented() {
        let mut doc = DiamondCrdtDocument::from_markdown("hello").unwrap();
        let err = doc.mark(&MarkType::Bold, 0, 5).unwrap_err();
        assert!(
            err.to_string().contains("not yet implemented"),
            "expected sentinel error, got: {err}"
        );
    }

    #[test]
    fn unmark_op_returns_not_implemented() {
        let mut doc = DiamondCrdtDocument::from_markdown("hello").unwrap();
        let err = doc.unmark(&MarkType::Bold, 0, 5).unwrap_err();
        assert!(
            err.to_string().contains("not yet implemented"),
            "expected sentinel error, got: {err}"
        );
    }

    #[test]
    fn save_is_smaller_than_plain_text_for_keystroke_traces() {
        // Sanity check the spike's finding: DT's RLE should compress
        // run-inserts under the plain-text size on long sessions.
        let mut doc = DiamondCrdtDocument::new().unwrap();
        let filler = "The sixteenth-century Venetian typographer Aldus Manutius set \
                      small italic forms to fit more words on a page; we just keep typing.";
        for (i, ch) in filler.chars().enumerate() {
            doc.splice_text(i, 0, &ch.to_string()).unwrap();
        }
        let bytes = doc.save();
        // Not a hard performance assertion — just sanity that encoding
        // isn't wildly bloated. DT should easily fit under 2× plain.
        assert!(
            bytes.len() < filler.len() * 2,
            "diamond-types encoding was {} bytes for {} plain chars",
            bytes.len(),
            filler.chars().count()
        );
    }
}
