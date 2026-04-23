//! Property tests for the diamond-types marks layer (IMPL-029 Phase 5).
//!
//! Covers the four convergence invariants called out in `task-diamond-marks-layer`:
//!   (a) marks survive save/load
//!   (b) inclusive marks grow at boundary inserts
//!   (c) non-growing marks do NOT grow at boundary inserts
//!   (d) concurrent splice / mark / unmark ops converge across replicas
//!       with marks intact
//!
//! `DiamondCrdtDocument` became the unconditional CRDT backend in IMPL-029
//! Phase 7, so these tests run on every build.

use proptest::prelude::*;
use ztl::crdt::backend::CrdtBackend;
use ztl::crdt::diamond::DiamondCrdtDocument;
use ztl::crdt::marks::{Mark, MarkType};

fn sort_marks(mut ms: Vec<Mark>) -> Vec<(String, usize, usize)> {
    ms.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then(a.start.cmp(&b.start))
            .then(a.end.cmp(&b.end))
    });
    ms.into_iter().map(|m| (m.name, m.start, m.end)).collect()
}

// -- (a) marks survive save/load --------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    /// Property (a): an arbitrary mark over a random subrange of a random
    /// plain-ASCII document round-trips through `save` → `load` with the same
    /// materialised span.
    #[test]
    fn marks_round_trip_save_load(
        text in "[a-zA-Z0-9 ]{1,40}",
        start_pick in 0usize..40,
        len_pick in 1usize..40,
        mark_idx in 0u8..8,
    ) {
        let text_len = text.chars().count();
        if text_len == 0 {
            return Ok(());
        }
        let start = start_pick % text_len;
        let end = (start + 1 + (len_pick % text_len.max(1))).min(text_len);
        if start >= end {
            return Ok(());
        }

        let mark = pick_mark(mark_idx);
        let mut doc = DiamondCrdtDocument::from_markdown(&text).unwrap();
        doc.mark(&mark, start, end).unwrap();
        let before = sort_marks(doc.marks().unwrap());

        let bytes = doc.save();
        let loaded = DiamondCrdtDocument::load(&bytes).unwrap();
        let after = sort_marks(loaded.marks().unwrap());

        prop_assert_eq!(before, after);
    }
}

// -- (b) inclusive marks grow at boundary ----------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    /// Property (b): an inclusive mark (`ExpandMark::Both`) over `[start, end)`
    /// followed by an insert of `k` chars at position `end` grows to end at
    /// `end + k`.
    #[test]
    fn inclusive_marks_grow_at_end_boundary(
        doc_len in 4usize..30,
        start in 0usize..30,
        len in 1usize..30,
        insert in "[a-z]{1,5}",
        inclusive_idx in 0u8..4,
    ) {
        let start = start % doc_len;
        let end = (start + 1 + (len % doc_len.max(1))).min(doc_len);
        if start >= end {
            return Ok(());
        }

        let text: String = "abcdefghijklmnopqrstuvwxyzABCDE".chars().take(doc_len).collect();
        let mark = pick_inclusive_mark(inclusive_idx);

        let mut doc = DiamondCrdtDocument::from_markdown(&text).unwrap();
        doc.mark(&mark, start, end).unwrap();
        doc.splice_text(end, 0, &insert).unwrap();

        let k = insert.chars().count();
        let marks = doc.marks().unwrap();
        let span = marks.iter().find(|m| m.name == mark.name())
            .expect("marked span present");
        prop_assert_eq!(span.end, end + k, "inclusive mark must grow by {}", k);
        prop_assert_eq!(span.start, start);
    }
}

// -- (c) non-growing marks do not grow at boundary -------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    /// Property (c): a non-growing mark (`ExpandMark::None`) over
    /// `[start, end)` followed by an insert at position `end` leaves the span
    /// end at `end` (content after it shifts right by `k`, but the mark does
    /// not expand to cover the new chars).
    #[test]
    fn non_growing_marks_do_not_grow_at_end_boundary(
        doc_len in 4usize..30,
        start in 0usize..30,
        len in 1usize..30,
        insert in "[a-z]{1,5}",
        non_growing_idx in 0u8..4,
    ) {
        let start = start % doc_len;
        let end = (start + 1 + (len % doc_len.max(1))).min(doc_len);
        if start >= end {
            return Ok(());
        }

        let text: String = "abcdefghijklmnopqrstuvwxyzABCDE".chars().take(doc_len).collect();
        let mark = pick_non_growing_mark(non_growing_idx);

        let mut doc = DiamondCrdtDocument::from_markdown(&text).unwrap();
        doc.mark(&mark, start, end).unwrap();
        doc.splice_text(end, 0, &insert).unwrap();

        let marks = doc.marks().unwrap();
        let span = marks.iter().find(|m| m.name == mark.name())
            .expect("marked span present");
        prop_assert_eq!(span.end, end, "non-growing mark must NOT grow");
        prop_assert_eq!(span.start, start);
    }
}

// -- (d) concurrent replicas converge with marks intact ---------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// Property (d): two replicas forked from the same baseline, each applying
    /// a random sequence of splice / mark / unmark ops, produce identical
    /// materialised mark sets after symmetric merge. The text content also
    /// agrees (diamond-types guarantees this; we re-assert for regression
    /// safety).
    #[test]
    fn concurrent_replicas_converge(
        base in "[a-zA-Z]{10,25}",
        alice_ops in prop::collection::vec(any_op_strategy(), 1..6),
        bob_ops in prop::collection::vec(any_op_strategy(), 1..6),
    ) {
        let mut alice = DiamondCrdtDocument::from_markdown(&base).unwrap();
        let mut bob = alice.fork();

        apply_ops(&mut alice, &alice_ops);
        apply_ops(&mut *bob, &bob_ops);

        // Symmetric merge: alice ← bob's updates, bob ← alice's updates.
        // Clone into fresh forks to avoid disturbing the originals when
        // extracting patches.
        let mut alice_copy = alice.fork();
        let mut bob_copy = bob.fork();
        alice.merge(&mut *bob_copy).unwrap();
        bob.merge(&mut *alice_copy).unwrap();

        let a_text = alice.text().unwrap();
        let b_text = bob.text().unwrap();
        prop_assert_eq!(&a_text, &b_text, "text must converge");

        let a_marks = sort_marks(alice.marks().unwrap());
        let b_marks = sort_marks(bob.marks().unwrap());
        prop_assert_eq!(a_marks, b_marks, "marks must converge");
    }
}

// -- op generation for property (d) -----------------------------------------

/// A random op applied during property (d)'s concurrent-replicas test.
#[derive(Debug, Clone)]
enum RandomOp {
    Splice {
        pos_pick: usize,
        del_pick: usize,
        text: String,
    },
    Mark {
        mark_idx: u8,
        start_pick: usize,
        end_pick: usize,
    },
    Unmark {
        mark_idx: u8,
        start_pick: usize,
        end_pick: usize,
    },
}

fn any_op_strategy() -> impl Strategy<Value = RandomOp> {
    prop_oneof![
        (0usize..50, 0usize..5, "[a-zA-Z]{0,4}").prop_map(|(pos_pick, del_pick, text)| {
            RandomOp::Splice {
                pos_pick,
                del_pick,
                text,
            }
        }),
        (0u8..8, 0usize..50, 0usize..50).prop_map(|(mark_idx, start_pick, end_pick)| {
            RandomOp::Mark {
                mark_idx,
                start_pick,
                end_pick,
            }
        }),
        (0u8..6, 0usize..50, 0usize..50).prop_map(|(mark_idx, start_pick, end_pick)| {
            RandomOp::Unmark {
                mark_idx,
                start_pick,
                end_pick,
            }
        }),
    ]
}

fn apply_ops(doc: &mut dyn CrdtBackend, ops: &[RandomOp]) {
    for op in ops {
        let len = doc.text().map(|t| t.chars().count()).unwrap_or(0);
        if len == 0 {
            // Only splice-insert is legal on an empty doc.
            if let RandomOp::Splice { text, .. } = op {
                if !text.is_empty() {
                    let _ = doc.splice_text(0, 0, text);
                }
            }
            continue;
        }
        match op {
            RandomOp::Splice {
                pos_pick,
                del_pick,
                text,
            } => {
                let pos = pos_pick % (len + 1);
                let max_del = len.saturating_sub(pos);
                let del = (*del_pick).min(max_del) as isize;
                let _ = doc.splice_text(pos, del, text);
            }
            RandomOp::Mark {
                mark_idx,
                start_pick,
                end_pick,
            } => {
                let mut start = start_pick % len;
                let mut end = end_pick % (len + 1);
                if end <= start {
                    end = (start + 1).min(len);
                }
                if start >= end {
                    start = end.saturating_sub(1);
                }
                if start < end {
                    let _ = doc.mark(&pick_mark(*mark_idx), start, end);
                }
            }
            RandomOp::Unmark {
                mark_idx,
                start_pick,
                end_pick,
            } => {
                let mut start = start_pick % len;
                let mut end = end_pick % (len + 1);
                if end <= start {
                    end = (start + 1).min(len);
                }
                if start >= end {
                    start = end.saturating_sub(1);
                }
                if start < end {
                    let _ = doc.unmark(&pick_simple_unmark(*mark_idx), start, end);
                }
            }
        }
    }
}

// -- MarkType fixtures ------------------------------------------------------

fn pick_mark(idx: u8) -> MarkType {
    match idx % 8 {
        0 => MarkType::Bold,
        1 => MarkType::Italic,
        2 => MarkType::Strikethrough,
        3 => MarkType::Highlight,
        4 => MarkType::Code,
        5 => MarkType::Comment,
        6 => MarkType::Wikilink {
            target: "Target".into(),
            alias: None,
        },
        _ => MarkType::Link {
            url: "https://example.com".into(),
        },
    }
}

fn pick_inclusive_mark(idx: u8) -> MarkType {
    match idx % 4 {
        0 => MarkType::Bold,
        1 => MarkType::Italic,
        2 => MarkType::Strikethrough,
        _ => MarkType::Highlight,
    }
}

fn pick_non_growing_mark(idx: u8) -> MarkType {
    match idx % 4 {
        0 => MarkType::Code,
        1 => MarkType::Comment,
        2 => MarkType::Wikilink {
            target: "T".into(),
            alias: None,
        },
        _ => MarkType::Link {
            url: "https://x".into(),
        },
    }
}

/// Mark types `unmark` accepts — only the simple (non-parameterized) ones
/// since `MarkType::from_name` rejects wikilink/link.
fn pick_simple_unmark(idx: u8) -> MarkType {
    match idx % 6 {
        0 => MarkType::Bold,
        1 => MarkType::Italic,
        2 => MarkType::Code,
        3 => MarkType::Strikethrough,
        4 => MarkType::Highlight,
        _ => MarkType::Comment,
    }
}
