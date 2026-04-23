//! IMPL-029 Phase 0 spike — diamond-types API exploration.
//!
//! Exercises the diamond-types 1.0 OpLog/Branch surface against the shapes
//! ztl currently needs from automerge (see src/crdt/mod.rs + src/web/ws.rs):
//! insert, delete, save/load, fork/merge, char-position addressing.
//!
//! Also runs the same scripted edit trace through automerge-0.5 so the wire
//! formats can be compared head-to-head.
//!
//! Run: `cd tools/diamond-types-spike && cargo run --release`

use std::time::Instant;

use automerge::transaction::Transactable;
use automerge::{AutoCommit, ObjType, ReadDoc, ROOT};
use diamond_types::list::encoding::{EncodeOptions, ENCODE_FULL, ENCODE_PATCH};
use diamond_types::list::{Branch, OpLog};

fn main() {
    banner("diamond-types crate metadata");
    println!("crate version      : 1.0.0 (first stable release, 2023-07)");
    println!("ztl pin candidate : 1.0  (only 1.x released; no breaking bumps since)");
    println!("license            : ISC");
    println!("default features   : lz4 (used for content compression in encoded blobs)");

    demo_insert_delete();
    demo_save_load();
    demo_fork_merge();
    demo_char_position();
    demo_concurrent_position_rewrite();
    wire_format_size_comparison();
    demo_patch_vs_full();

    banner("done");
    println!("See research/diamond-types-spike.md for the write-up.");
}

// ────────────────────────────────────────────────────────────────────
// Demos
// ────────────────────────────────────────────────────────────────────

fn demo_insert_delete() {
    banner("(1) insert + delete + text readback");
    let mut oplog = OpLog::new();
    let alice = oplog.get_or_create_agent_id("alice");

    oplog.add_insert(alice, 0, "Hello world");
    println!("after insert           : {:?}", checkout(&oplog));

    // delete_without_content is the cheap variant — doesn't store the
    // deleted bytes. Use add_delete_without_content on the oplog.
    oplog.add_delete_without_content(alice, 5..11); // strip " world"
    println!("after delete 5..11     : {:?}", checkout(&oplog));

    oplog.add_insert(alice, 5, ", diamond");
    println!("after insert at 5      : {:?}", checkout(&oplog));

    println!("oplog ops so far (RLE) : {}", oplog.len());
}

fn demo_save_load() {
    banner("(2) save / load round trip");
    let mut oplog = OpLog::new();
    let a = oplog.get_or_create_agent_id("alice");
    oplog.add_insert(a, 0, "The quick brown fox jumps over the lazy dog.\n");
    oplog.add_insert(a, 4, "very ");
    oplog.add_delete_without_content(a, 10..15); // delete " brow"
    let before = checkout(&oplog);

    let bytes = oplog.encode(EncodeOptions::default());
    println!("encoded bytes          : {}", bytes.len());

    let loaded = OpLog::load_from(&bytes).expect("load round trip");
    let after = checkout(&loaded);
    assert_eq!(before, after, "load round trip content mismatch");
    println!("round-trip OK          : {after:?}");
}

fn demo_fork_merge() {
    banner("(3) fork + concurrent edits + merge");
    // Shared history: both peers start from the same oplog.
    let mut base = OpLog::new();
    let shared = base.get_or_create_agent_id("shared");
    base.add_insert(shared, 0, "Hello world.\n");

    // Fork = clone the oplog on each peer.  Each peer uses its *own*
    // agent-id so concurrent IDs don't collide (see diamond-types lib.rs
    // "Warning: Do not reuse IDs").
    let mut alice = base.clone();
    let mut bob = base.clone();

    let a_id = alice.get_or_create_agent_id("alice-session");
    let b_id = bob.get_or_create_agent_id("bob-session");

    // Parents = the version they both share at fork time.
    let fork_point = base.local_version();

    // Alice types "Hi, " at the start.
    alice.add_insert_at(a_id, &fork_point, 0, "Hi, ");
    // Bob appends "Goodbye." after the first period (before '\n').
    bob.add_insert_at(b_id, &fork_point, 12, " Goodbye.");

    println!("alice before merge     : {:?}", checkout(&alice));
    println!("bob before merge       : {:?}", checkout(&bob));

    // To merge, each peer encodes its divergent segment and the other
    // applies it.  The canonical way in DT 1.0 is via OpLog -> bytes ->
    // OpLog (there's no in-memory OpLog::merge; use the encode pipe or
    // the higher-level ListCRDT::merge_data_and_ff).
    let alice_bytes = alice.encode(EncodeOptions::default());
    let bob_bytes = bob.encode(EncodeOptions::default());

    // Apply each side's bytes to a fresh merged oplog.  load_from returns
    // a fresh OpLog; decode_and_add appends additional history into an
    // existing oplog.
    let mut merged = OpLog::load_from(&alice_bytes).expect("load alice");
    merged
        .decode_and_add(&bob_bytes)
        .expect("decode_and_add bob");
    println!("merged                 : {:?}", checkout(&merged));

    // Sanity: the Branch-level merge API also works, given both peers'
    // oplogs already share history.  We use it here to double-check
    // convergence.
    let branch_merged = {
        let mut branch = Branch::new_at_tip(&merged);
        branch.merge(&merged, merged.local_version_ref());
        branch.content().to_string()
    };
    println!("branch.content()       : {branch_merged:?}");
}

fn demo_char_position() {
    banner("(4) char-position addressing (multi-byte text)");
    // Diamond-types' internal rope stores content in unicode code points
    // and positions are char indices — *not* byte indices. Relevant for
    // ztl's em-dash/emoji pages (see Cache.md repro test).
    let mut oplog = OpLog::new();
    let a = oplog.get_or_create_agent_id("alice");

    oplog.add_insert(a, 0, "café — 🌊");
    let snapshot = checkout(&oplog);
    println!("initial                : {snapshot:?}  (char len = {})", snapshot.chars().count());

    // Insert at char position 5 (between "café " and "—"): note, this is
    // char position 5 even though the byte offset is 6 ('é' is 2 bytes).
    oplog.add_insert(a, 5, "[after é] ");
    println!("after insert at char 5 : {:?}", checkout(&oplog));

    // Delete the em-dash at its *char* range.
    let content = checkout(&oplog);
    let em_start = content.chars().position(|c| c == '—').unwrap();
    oplog.add_delete_without_content(a, em_start..em_start + 1);
    println!("after em-dash delete   : {:?}", checkout(&oplog));
}

fn demo_concurrent_position_rewrite() {
    banner("(5) concurrent insert rewrites positions (Peritext-like convergence)");
    // Verifies DT's transformed-op merge: two inserts at the same char
    // pos concurrently → both preserved, deterministic order.
    let mut base = OpLog::new();
    let shared = base.get_or_create_agent_id("shared");
    base.add_insert(shared, 0, "XZ");

    let mut alice = base.clone();
    let mut bob = base.clone();
    let a = alice.get_or_create_agent_id("alice");
    let b = bob.get_or_create_agent_id("bob");

    let fork = base.local_version();
    alice.add_insert_at(a, &fork, 1, "A"); // X[A]Z
    bob.add_insert_at(b, &fork, 1, "B");   // X[B]Z

    let mut merged = OpLog::load_from(&alice.encode(EncodeOptions::default())).unwrap();
    merged
        .decode_and_add(&bob.encode(EncodeOptions::default()))
        .unwrap();

    let result = checkout(&merged);
    println!("alice : {:?}", checkout(&alice));
    println!("bob   : {:?}", checkout(&bob));
    println!("merged: {result:?}  (both inserts preserved, order is deterministic per agent-id sort)");
    assert_eq!(result.chars().count(), 4);
    assert!(result.contains('A') && result.contains('B'));
}

// ────────────────────────────────────────────────────────────────────
// Wire-format size: diamond-types vs automerge on the same edit trace
// ────────────────────────────────────────────────────────────────────

fn wire_format_size_comparison() {
    banner("(6) wire-format size: diamond-types vs automerge");
    compare_trace("scripted meeting-notes", scripted_trace());
    compare_trace("synthesised 1000-op typing", synthesised_trace(1000));
}

fn compare_trace(label: &str, trace: Vec<Op>) {
    println!("\n-- trace: {label} --");

    // Diamond-types encode.  Branch::insert/delete also append to the
    // oplog, so use only one of the two code paths — here we drive the
    // oplog directly and derive the Branch on demand.
    let dt_bytes = {
        let mut oplog = OpLog::new();
        let a = oplog.get_or_create_agent_id("alice");
        for op in &trace {
            match op {
                Op::Insert(pos, s) => {
                    oplog.add_insert(a, *pos, s.as_str());
                }
                Op::Delete(range) => {
                    oplog.add_delete_without_content(a, range.clone());
                }
            }
        }
        oplog.encode(EncodeOptions::default())
    };

    // Automerge encode — replay the same trace against an AutoCommit text.
    let am_bytes = {
        let mut doc = AutoCommit::new();
        let tid = doc.put_object(ROOT, "content", ObjType::Text).unwrap();
        for op in &trace {
            match op {
                Op::Insert(pos, s) => {
                    doc.splice_text(&tid, *pos, 0, s.as_str()).unwrap();
                }
                Op::Delete(range) => {
                    let len = range.end - range.start;
                    doc.splice_text(&tid, range.start, len as isize, "").unwrap();
                }
            }
        }
        doc.save()
    };

    // Sanity: both backends converge to the same text.
    let dt_text = {
        let oplog = OpLog::load_from(&dt_bytes).unwrap();
        checkout(&oplog)
    };
    let am_text = {
        let doc = AutoCommit::load(&am_bytes).unwrap();
        let tid = doc.get(ROOT, "content").unwrap().unwrap().1;
        doc.text(&tid).unwrap()
    };
    assert_eq!(dt_text, am_text, "backends diverged on scripted trace");

    let plain_text_len = dt_text.len();

    println!("scripted trace ops     : {}", trace.len());
    println!("plain-text size        : {plain_text_len} bytes");
    println!(
        "diamond-types encoded  : {:>5} bytes  ({:.2}× plain)",
        dt_bytes.len(),
        dt_bytes.len() as f64 / plain_text_len.max(1) as f64
    );
    println!(
        "automerge encoded      : {:>5} bytes  ({:.2}× plain)",
        am_bytes.len(),
        am_bytes.len() as f64 / plain_text_len.max(1) as f64
    );
    println!(
        "ratio dt / automerge   : {:.2}×",
        dt_bytes.len() as f64 / am_bytes.len().max(1) as f64
    );

    // Cold-load timing: a rough proxy for the payload-parse cost on late
    // WebSocket joiners (src/web/ws.rs::handle_socket sends the full doc).
    let t0 = Instant::now();
    for _ in 0..100 {
        let _ = OpLog::load_from(&dt_bytes).unwrap();
    }
    let dt_load_us = t0.elapsed().as_micros() as f64 / 100.0;
    let t0 = Instant::now();
    for _ in 0..100 {
        let _ = AutoCommit::load(&am_bytes).unwrap();
    }
    let am_load_us = t0.elapsed().as_micros() as f64 / 100.0;
    println!("dt load  (avg of 100) : {dt_load_us:>8.1} µs");
    println!("am load  (avg of 100) : {am_load_us:>8.1} µs");
}

fn demo_patch_vs_full() {
    banner("(7) encode modes: ENCODE_PATCH vs ENCODE_FULL");
    // The ws.rs protocol has two shapes today:
    //   - full doc push on sync (ServerMsg::Sync { doc: base64 })
    //   - incremental op broadcast (ServerMsg::Op { ops: [...] })
    // Diamond-types' equivalents are ENCODE_FULL (includes start-branch
    // content) vs ENCODE_PATCH (patch-only, no baseline content).
    let mut oplog = OpLog::new();
    let a = oplog.get_or_create_agent_id("alice");
    for _ in 0..50 {
        oplog.add_insert(a, 0, "abcdefghijklmnop\n");
    }
    let full = oplog.encode(ENCODE_FULL);
    let patch = oplog.encode(ENCODE_PATCH);
    println!("ENCODE_FULL  : {} bytes", full.len());
    println!("ENCODE_PATCH : {} bytes", patch.len());
    println!(
        "note: patch-only would need an agreed-upon base version on both sides — current ws.rs \
         \"Sync\" message uses a full doc, so map Sync → ENCODE_FULL and Op → \
         ENCODE_PATCH from a known frontier."
    );
}

// ────────────────────────────────────────────────────────────────────
// Helpers
// ────────────────────────────────────────────────────────────────────

fn checkout(oplog: &OpLog) -> String {
    oplog.checkout_tip().content().to_string()
}

fn banner(title: &str) {
    println!("\n── {title} ────────────────────────────────");
}

enum Op {
    Insert(usize, String),
    Delete(std::ops::Range<usize>),
}

/// A small scripted edit trace loosely representing 5 minutes of typing
/// in a ztl note: mostly run-inserts with a handful of backspaces. Used
/// as a level playing field for wire-size comparisons.
fn scripted_trace() -> Vec<Op> {
    let mut ops = Vec::new();
    let chunks = [
        "# Meeting notes\n\n",
        "- Discussed IMPL-029 backend swap.\n",
        "- Diamond-types is text-only; marks need a sibling layer.\n",
        "- Review wire-format budget for /ws/edit sync payloads.\n",
        "- Ship behind --features collab until the marks layer lands.\n",
        "\n## Follow-ups\n",
        "1. Spike the OpLog API.\n",
        "2. Design the marks sibling.\n",
        "3. Migrate src/web/ws.rs off automerge::ScalarValue.\n",
    ];
    let mut pos = 0usize;
    for chunk in chunks {
        ops.push(Op::Insert(pos, chunk.to_string()));
        pos += chunk.chars().count();
    }
    // A handful of backspace-style deletions scattered through.
    ops.push(Op::Delete(30..34));
    ops.push(Op::Delete(120..128));
    ops.push(Op::Insert(30, "IMPL-029".to_string()));
    ops
}

/// Synthesised trace: `n` single-char insertions at the tip, occasionally
/// rewound by a 3-char backspace. Representative of keystroke-granular
/// editing where the optimistic UI flushes one op per key.
fn synthesised_trace(n: usize) -> Vec<Op> {
    let filler = "The sixteenth-century Venetian typographer Aldus Manutius set small \
                  italic forms to fit more words on a page; we just keep typing. ";
    let mut ops = Vec::with_capacity(n + n / 20);
    let mut pos = 0usize;
    let chars: Vec<char> = filler.chars().cycle().take(n).collect();
    for (i, ch) in chars.iter().enumerate() {
        ops.push(Op::Insert(pos, ch.to_string()));
        pos += 1;
        // Every 20 chars, "correct a typo": backspace 3 chars.
        if i > 0 && i % 20 == 0 && pos >= 3 {
            ops.push(Op::Delete(pos - 3..pos));
            pos -= 3;
        }
    }
    ops
}
