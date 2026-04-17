//! Sibling marks oplog for the diamond-types CRDT backend (IMPL-029 Phase 5).
//!
//! `MarksDoc` wraps a second `diamond_types::list::OpLog` that stores span-level
//! edits (`SpanOp::{Mark, Unmark, Shift}`) as newline-delimited JSON. Every
//! write path on [`crate::crdt::diamond::DiamondCrdtDocument`] that touches
//! text also touches this doc atomically — splicing text emits a paired
//! `Shift` so mark positions stay consistent across replicas.
//!
//! Materialising the current `Vec<Mark>` replays the marks oplog in
//! diamond-types' canonical merge order. That's what preserves Peritext
//! expand semantics across concurrent edits: each peer's ops fold in at a
//! deterministic point in the replay, so inclusive marks at a boundary grow
//! or not depending on local `ExpandMark` — independent of arrival order.
//!
//! Design spec: `research/diamond-types-marks.md`.
//!
//! Why store ops as JSON-in-text instead of using a richer DT structure:
//! diamond-types 1.0 only exposes a list-of-chars API. Riding that gives us
//! RLE packing, agent/seq ordering, and binary wire format for free. Each
//! `SpanOp` is a single `add_insert` call of one JSON line + `\n`; DT packs
//! consecutive lines into a single RLE run.

use anyhow::{Context, Result};
use diamond_types::list::encoding::EncodeOptions;
use diamond_types::list::OpLog;
use diamond_types::AgentId;
use serde::{Deserialize, Serialize};

use crate::crdt::marks::{ExpandMark, Mark, MarkType, Scalar};

/// Span-level op stored in the sibling marks oplog.
///
/// One entry per `mark` / `unmark` / `shift` call on [`MarksDoc`]. Serialised
/// as a single JSON line appended to the marks oplog.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum SpanOp {
    /// Apply a mark over `[start, end)`. `expand` is decided by the caller's
    /// `MarkType` and replayed on every peer.
    Mark {
        name: String,
        value: Scalar,
        start: usize,
        end: usize,
        expand: ExpandMark,
    },
    /// Clear any mark named `name` overlapping `[start, end)`.
    ///
    /// `value` is `None` for "unmark any" (how `CrdtBackend::unmark` is called
    /// today); `Some(_)` is reserved for targeted unmark use cases not
    /// exercised yet.
    Unmark {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<Scalar>,
        start: usize,
        end: usize,
        expand: ExpandMark,
    },
    /// Shift every open span's endpoint(s) to track a text splice.
    /// Positive delta = `delta` chars inserted at `pos`; negative = deleted.
    Shift { pos: usize, delta: i64 },
}

/// Sibling diamond-types oplog storing [`SpanOp`]s.
///
/// Shares the text oplog's agent *name* (both register the same name on their
/// own `AgentId` tables) so concurrent edits from the same peer get merged
/// under one identity. Forks must allocate a fresh agent name — see
/// [`MarksDoc::clone_with_agent`].
pub struct MarksDoc {
    oplog: OpLog,
    agent_name: String,
    agent: Option<AgentId>,
}

impl MarksDoc {
    /// Create an empty marks oplog under `agent_name`.
    pub fn new(agent_name: String) -> Self {
        Self {
            oplog: OpLog::new(),
            agent_name,
            agent: None,
        }
    }

    /// Load an existing marks oplog from encoded bytes.
    pub fn load_from(data: &[u8], agent_name: String) -> Result<Self> {
        let oplog =
            OpLog::load_from(data).map_err(|e| anyhow::anyhow!("load marks oplog: {e:?}"))?;
        Ok(Self {
            oplog,
            agent_name,
            agent: None,
        })
    }

    /// Lazily register the local agent. Called on every write path so that
    /// a load → save round-trip is byte-identical (no agent-table growth).
    fn agent(&mut self) -> AgentId {
        if let Some(a) = self.agent {
            return a;
        }
        let a = self.oplog.get_or_create_agent_id(&self.agent_name);
        self.agent = Some(a);
        a
    }

    /// Append a [`SpanOp`] as a JSON line at the tail of the marks oplog.
    fn append(&mut self, op: &SpanOp) -> Result<()> {
        let mut json = serde_json::to_string(op).context("serialize SpanOp")?;
        json.push('\n');
        let agent = self.agent();
        let pos = self.oplog.len();
        self.oplog.add_insert(agent, pos, &json);
        Ok(())
    }

    /// Record a `Mark` op for `[start, end)`.
    pub fn mark(
        &mut self,
        name: &str,
        value: Scalar,
        start: usize,
        end: usize,
        expand: ExpandMark,
    ) -> Result<()> {
        if start >= end {
            return Ok(());
        }
        self.append(&SpanOp::Mark {
            name: name.to_string(),
            value,
            start,
            end,
            expand,
        })
    }

    /// Record an `Unmark` op for `[start, end)`.
    pub fn unmark(
        &mut self,
        name: &str,
        start: usize,
        end: usize,
        expand: ExpandMark,
    ) -> Result<()> {
        if start >= end {
            return Ok(());
        }
        self.append(&SpanOp::Unmark {
            name: name.to_string(),
            value: None,
            start,
            end,
            expand,
        })
    }

    /// Record a `Shift` op for a paired text splice.
    ///
    /// `delta > 0` = chars inserted at `pos`; `delta < 0` = chars deleted.
    pub fn shift(&mut self, pos: usize, delta: i64) -> Result<()> {
        if delta == 0 {
            return Ok(());
        }
        self.append(&SpanOp::Shift { pos, delta })
    }

    /// Encode the marks oplog to bytes for persistence / wire transfer.
    pub fn encode(&self) -> Vec<u8> {
        self.oplog.encode(EncodeOptions::default())
    }

    /// Merge another encoded marks oplog into this one.
    pub fn decode_and_add(&mut self, data: &[u8]) -> Result<()> {
        self.oplog
            .decode_and_add(data)
            .map_err(|e| anyhow::anyhow!("merge marks oplog: {e:?}"))?;
        Ok(())
    }

    /// Clone the marks oplog under a new agent name.
    ///
    /// Diamond-types corrupts on reused `(agent, seq)` tuples across divergent
    /// branches, so every `fork` call must allocate a fresh name.
    pub fn clone_with_agent(&self, agent_name: String) -> Self {
        Self {
            oplog: self.oplog.clone(),
            agent_name,
            agent: None,
        }
    }

    /// Replay the marks oplog in canonical merge order and materialise the
    /// current mark set.
    pub fn materialise(&self) -> Vec<Mark> {
        let text = self.oplog.checkout_tip().content().to_string();
        let ops = parse_span_ops(&text);
        materialise_spans(&ops)
    }
}

/// Parse a newline-delimited stream of JSON [`SpanOp`]s.
///
/// Corrupt / partial lines are skipped — the marks oplog is append-only, so
/// malformed records can only come from a bug (not a downgrade), and the
/// correct behaviour is to log-and-skip rather than fail the whole document.
fn parse_span_ops(text: &str) -> Vec<SpanOp> {
    text.split('\n')
        .filter(|line| !line.is_empty())
        .filter_map(|line| serde_json::from_str::<SpanOp>(line).ok())
        .collect()
}

/// Materialise a span-set by folding a stream of [`SpanOp`]s in order.
///
/// Implements the Peritext expand / shift / carve-out rules from
/// `research/diamond-types-marks.md` §4.
fn materialise_spans(ops: &[SpanOp]) -> Vec<Mark> {
    let mut spans: Vec<MarkSpan> = Vec::new();
    for op in ops {
        match op {
            SpanOp::Shift { pos, delta } => apply_shift(&mut spans, *pos, *delta),
            SpanOp::Mark {
                name,
                value,
                start,
                end,
                expand,
            } => apply_mark(&mut spans, name, value, *start, *end, *expand),
            SpanOp::Unmark {
                name,
                value,
                start,
                end,
                ..
            } => apply_unmark(&mut spans, name, value.as_ref(), *start, *end),
        }
    }
    spans
        .into_iter()
        .map(|s| Mark {
            name: s.name,
            value: s.value,
            start: s.start,
            end: s.end,
        })
        .collect()
}

/// Open-span tracking struct used during materialisation. Carries `expand`
/// alongside the range so subsequent `Shift` ops can grow / skip boundaries
/// correctly; the emitted [`Mark`] drops it (not needed on the wire).
#[derive(Debug, Clone)]
struct MarkSpan {
    name: String,
    value: Scalar,
    start: usize,
    end: usize,
    expand: ExpandMark,
}

/// Apply a text-position shift to every open span.
///
/// Splits into insert-shift (delta > 0) and delete-shift (delta < 0). An
/// insert at a span boundary grows the span only when `ExpandMark::Both`; a
/// delete always clamps to the surviving range, dropping the span if fully
/// covered.
fn apply_shift(spans: &mut Vec<MarkSpan>, pos: usize, delta: i64) {
    if delta == 0 {
        return;
    }
    if delta > 0 {
        let d = delta as usize;
        for span in spans.iter_mut() {
            if pos > span.end {
                // entirely after the span — no change
            } else if pos < span.start {
                span.start += d;
                span.end += d;
            } else if pos == span.start {
                match span.expand {
                    ExpandMark::Both => span.end += d,
                    ExpandMark::None => {
                        span.start += d;
                        span.end += d;
                    }
                }
            } else if pos == span.end {
                match span.expand {
                    ExpandMark::Both => span.end += d,
                    ExpandMark::None => {}
                }
            } else {
                // strictly inside — always grow
                span.end += d;
            }
        }
    } else {
        let d = (-delta) as usize;
        let del_start = pos;
        let del_end = pos + d;
        let mut out = Vec::with_capacity(spans.len());
        for span in spans.drain(..) {
            if del_end <= span.start {
                out.push(MarkSpan {
                    start: span.start - d,
                    end: span.end - d,
                    ..span
                });
            } else if del_start >= span.end {
                out.push(span);
            } else {
                let new_start = span.start.min(del_start);
                let surviving_end = span.end.max(del_end).saturating_sub(d);
                let new_end = surviving_end.max(new_start);
                if new_start < new_end {
                    out.push(MarkSpan {
                        start: new_start,
                        end: new_end,
                        ..span
                    });
                }
            }
        }
        *spans = out;
    }
}

/// Apply a `Mark` op, handling LWW-per-span for exclusive mark types and
/// overlay (accumulate) for the rest.
fn apply_mark(
    spans: &mut Vec<MarkSpan>,
    name: &str,
    value: &Scalar,
    start: usize,
    end: usize,
    expand: ExpandMark,
) {
    if start >= end {
        return;
    }
    let exclusive = MarkType::from_name(name)
        .map(|mt| mt.is_exclusive())
        .unwrap_or_else(|| {
            // Named marks not in `from_name` (wikilink / link) still resolve
            // via `from_mark`, which carries the scalar payload.
            MarkType::from_mark(name, value)
                .map(|mt| mt.is_exclusive())
                .unwrap_or(false)
        });
    if exclusive {
        spans.retain(|s| !(s.name == name && ranges_overlap(s.start, s.end, start, end)));
    }
    spans.push(MarkSpan {
        name: name.to_string(),
        value: value.clone(),
        start,
        end,
        expand,
    });
}

/// Apply an `Unmark` op — carve `[start, end)` out of any matching span.
///
/// Behaviour matches automerge's `unmark`: disjoint spans pass through,
/// fully-covered spans drop, partial overlaps clip / split.
fn apply_unmark(
    spans: &mut Vec<MarkSpan>,
    name: &str,
    value: Option<&Scalar>,
    start: usize,
    end: usize,
) {
    if start >= end {
        return;
    }
    let mut out = Vec::with_capacity(spans.len());
    for s in spans.drain(..) {
        let name_matches = s.name == name;
        let value_matches = value.map_or(true, |v| &s.value == v);
        if !(name_matches && value_matches) {
            out.push(s);
            continue;
        }
        if end <= s.start || start >= s.end {
            // disjoint
            out.push(s);
        } else if start <= s.start && end >= s.end {
            // fully covered — drop
        } else if start > s.start && end < s.end {
            // unmark range strictly inside — split
            out.push(MarkSpan {
                end: start,
                ..s.clone()
            });
            out.push(MarkSpan { start: end, ..s });
        } else if start <= s.start {
            // clip left
            out.push(MarkSpan { start: end, ..s });
        } else {
            // clip right
            out.push(MarkSpan { end: start, ..s });
        }
    }
    *spans = out;
}

fn ranges_overlap(a_start: usize, a_end: usize, b_start: usize, b_end: usize) -> bool {
    a_start < b_end && b_start < a_end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_doc_has_no_marks() {
        let doc = MarksDoc::new("alice".into());
        assert!(doc.materialise().is_empty());
    }

    #[test]
    fn single_mark_survives_save_load() {
        let mut doc = MarksDoc::new("alice".into());
        doc.mark("bold", Scalar::Bool(true), 0, 5, ExpandMark::Both)
            .unwrap();
        let bytes = doc.encode();

        let loaded = MarksDoc::load_from(&bytes, "alice".into()).unwrap();
        let marks = loaded.materialise();
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].name, "bold");
        assert_eq!(marks[0].start, 0);
        assert_eq!(marks[0].end, 5);
    }

    #[test]
    fn inclusive_mark_grows_at_end_boundary() {
        let mut doc = MarksDoc::new("alice".into());
        doc.mark("bold", Scalar::Bool(true), 0, 5, ExpandMark::Both)
            .unwrap();
        doc.shift(5, 3).unwrap();

        let marks = doc.materialise();
        assert_eq!(marks[0].end, 8, "inclusive mark must grow at end boundary");
        assert_eq!(marks[0].start, 0);
    }

    #[test]
    fn non_growing_mark_preserved_at_end_boundary() {
        let mut doc = MarksDoc::new("alice".into());
        doc.mark("wikilink", Scalar::Str("X".into()), 5, 14, ExpandMark::None)
            .unwrap();
        doc.shift(14, 4).unwrap();

        let marks = doc.materialise();
        assert_eq!(
            marks[0].end, 14,
            "non-growing mark must NOT grow at end boundary"
        );
    }

    #[test]
    fn insert_inside_span_always_grows() {
        let mut doc = MarksDoc::new("alice".into());
        doc.mark("code", Scalar::Bool(true), 0, 5, ExpandMark::None)
            .unwrap();
        doc.shift(3, 2).unwrap();

        let marks = doc.materialise();
        assert_eq!(marks[0].end, 7, "mid-span insert always grows span");
    }

    #[test]
    fn insert_before_span_shifts_right() {
        let mut doc = MarksDoc::new("alice".into());
        doc.mark("bold", Scalar::Bool(true), 5, 10, ExpandMark::Both)
            .unwrap();
        doc.shift(0, 3).unwrap();

        let marks = doc.materialise();
        assert_eq!(marks[0].start, 8);
        assert_eq!(marks[0].end, 13);
    }

    #[test]
    fn delete_fully_covering_span_drops_it() {
        let mut doc = MarksDoc::new("alice".into());
        doc.mark("bold", Scalar::Bool(true), 3, 7, ExpandMark::Both)
            .unwrap();
        doc.shift(0, -10).unwrap();

        assert!(doc.materialise().is_empty());
    }

    #[test]
    fn delete_clamps_partial_overlap() {
        let mut doc = MarksDoc::new("alice".into());
        doc.mark("bold", Scalar::Bool(true), 2, 10, ExpandMark::Both)
            .unwrap();
        // Delete chars [5, 8) — 3 chars
        doc.shift(5, -3).unwrap();

        let marks = doc.materialise();
        assert_eq!(marks[0].start, 2);
        assert_eq!(marks[0].end, 7);
    }

    #[test]
    fn unmark_carves_out_middle() {
        let mut doc = MarksDoc::new("alice".into());
        doc.mark("bold", Scalar::Bool(true), 0, 10, ExpandMark::Both)
            .unwrap();
        doc.unmark("bold", 3, 7, ExpandMark::Both).unwrap();

        let marks = doc.materialise();
        assert_eq!(marks.len(), 2, "unmark in middle splits into two spans");
        assert!(marks.iter().any(|m| m.start == 0 && m.end == 3));
        assert!(marks.iter().any(|m| m.start == 7 && m.end == 10));
    }

    #[test]
    fn unmark_fully_covers_drops_span() {
        let mut doc = MarksDoc::new("alice".into());
        doc.mark("bold", Scalar::Bool(true), 2, 5, ExpandMark::Both)
            .unwrap();
        doc.unmark("bold", 0, 10, ExpandMark::Both).unwrap();

        assert!(doc.materialise().is_empty());
    }

    #[test]
    fn exclusive_mark_lww_on_overlap() {
        let mut doc = MarksDoc::new("alice".into());
        doc.mark(
            "wikilink",
            Scalar::Str("Target A".into()),
            0,
            5,
            ExpandMark::None,
        )
        .unwrap();
        // Overlapping wikilink mark — should replace the earlier one (LWW).
        doc.mark(
            "wikilink",
            Scalar::Str("Target B".into()),
            2,
            8,
            ExpandMark::None,
        )
        .unwrap();

        let marks = doc.materialise();
        assert_eq!(marks.len(), 1, "exclusive marks LWW per overlap");
        assert_eq!(marks[0].value, Scalar::Str("Target B".into()));
    }

    #[test]
    fn overlay_marks_coexist() {
        let mut doc = MarksDoc::new("alice".into());
        doc.mark("bold", Scalar::Bool(true), 0, 5, ExpandMark::Both)
            .unwrap();
        // Another bold over the same range — both survive (overlay), serialiser
        // dedups at render time.
        doc.mark("bold", Scalar::Bool(true), 3, 8, ExpandMark::Both)
            .unwrap();

        let marks = doc.materialise();
        assert_eq!(marks.len(), 2, "overlay marks accumulate");
    }

    #[test]
    fn concurrent_merge_preserves_marks() {
        let mut alice = MarksDoc::new("alice".into());
        let mut bob = MarksDoc::new("bob".into());

        alice
            .mark("bold", Scalar::Bool(true), 0, 5, ExpandMark::Both)
            .unwrap();
        bob.mark("italic", Scalar::Bool(true), 10, 15, ExpandMark::Both)
            .unwrap();

        // Symmetric merge.
        let alice_bytes = alice.encode();
        let bob_bytes = bob.encode();
        alice.decode_and_add(&bob_bytes).unwrap();
        bob.decode_and_add(&alice_bytes).unwrap();

        let a_marks = alice.materialise();
        let b_marks = bob.materialise();
        assert_eq!(a_marks.len(), 2);
        assert_eq!(b_marks.len(), 2);

        // Both replicas see the same mark set (order-independent).
        let mut a_names: Vec<_> = a_marks.iter().map(|m| m.name.as_str()).collect();
        let mut b_names: Vec<_> = b_marks.iter().map(|m| m.name.as_str()).collect();
        a_names.sort();
        b_names.sort();
        assert_eq!(a_names, b_names);
    }
}
