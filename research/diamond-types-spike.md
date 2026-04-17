# diamond-types spike (IMPL-029 / Phase 0)

Captures what the Phase 0 spike in `tools/diamond-types-spike/` uncovered
about the [diamond-types](https://crates.io/crates/diamond-types) 1.0 CRDT,
its fit for zetl's collaborative-editing surface (`src/crdt/`,
`src/web/ws.rs`), and the gaps that the rest of IMPL-029 has to close.

Run the spike:

```bash
cd tools/diamond-types-spike
cargo run --release
```

The spike is a standalone Cargo package (not a workspace member) so it
doesn't perturb the main crate's dependency graph. It pulls in both
`diamond-types = "1.0"` and `automerge = "0.5"` so the same scripted
edit traces can be measured on each backend head-to-head.

## 1. Does diamond-types cover what zetl needs?

The table below maps zetl's current automerge surface (as used by
`src/crdt/mod.rs`'s `CrdtDocument` and by `src/web/ws.rs`) onto
diamond-types 1.0.

| zetl/automerge today                                | diamond-types 1.0 equivalent                                                             | Notes                                                                                                                                |
| --------------------------------------------------- | ---------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| `AutoCommit::new()` + `put_object(ROOT, content, Text)` | `OpLog::new()`                                                                       | DT is text-only; the document *is* the oplog. No root/object-map concept.                                                            |
| `doc.splice_text(obj, pos, del, text)`              | `oplog.add_insert(agent, pos, text)` + `oplog.add_delete_without_content(agent, range)` | Insert and delete are separate ops. Positions are **char** indices, matching automerge.                                             |
| `doc.text(obj) -> String`                           | `oplog.checkout_tip().content().to_string()`                                             | Materialisation produces a `Branch`, whose `content` is a [JumpRope](https://crates.io/crates/jumprope).                             |
| `doc.marks(obj)`, `doc.mark(...)`, `doc.unmark(...)`| **missing**                                                                              | DT is plain text only. Marks must be layered on top (see §4).                                                                        |
| `doc.save() -> Vec<u8>`                             | `oplog.encode(EncodeOptions::default())` (ENCODE_FULL)                                   | Binary, with optional lz4 compression of inserted-content payload.                                                                   |
| `AutoCommit::load(&bytes)`                          | `OpLog::load_from(&bytes)`                                                               | Single-shot decode.                                                                                                                  |
| `doc.fork()`                                        | `oplog.clone()`                                                                          | DT's OpLog implements `Clone`. Each peer *must* use a distinct agent id after forking — see the warning in `diamond-types::lib.rs`. |
| `doc.merge(&mut other)`                             | `merged.decode_and_add(&other.encode(...))` (bytes round-trip)                           | There is no in-memory `OpLog::merge`; the canonical path is encode → `decode_and_add`. `Branch::merge(&oplog, frontier)` advances a checkout to a given tip. |

All of the text-shaped requirements in `CrdtDocument` are covered, and
the spike's demos (1)–(5) demonstrate insert, delete, save/load,
fork+merge, char-position addressing and concurrent-position rewrite on
multi-byte text. The only real gap is **marks**, and that is Phase 1's
subject.

### Minimum supported diamond-types version

**`diamond-types = "1.0"`.** There is only one `1.x` release on crates.io
(`1.0.0`, ISC licence, MSRV compatible with `rustc 1.88` per the Cargo
resolver). The crate has been dormant since the 1.0 tag — the author's
[README](https://github.com/josephg/diamond-types) flags a `more_types`
branch for future work, but 1.0.0 is what we can depend on today.
Pin style: `diamond-types = "1.0"` to match `automerge = "0.5"`.

Transitive tree is modest (~15 crates: `jumprope`, `smartstring`,
`smallvec`, `rle`, `content-tree`, `lz4_flex`, `humansize`, plus small
RNG/tracing deps). No `reqwest`, no `tokio`, no heavy FFI. First compile
of the spike on a cold cache takes ~18 s release.

## 2. Wire-format size vs automerge

Measured by the spike on identical edit traces replayed against both
backends (demo 6). All numbers from a release build on Apple Silicon.

| Trace                        | Plain text | diamond-types (ENCODE_FULL) | automerge (save) | DT ÷ automerge |
| ---------------------------- | ---------: | --------------------------: | ---------------: | -------------: |
| Scripted meeting-notes (12 ops, run-inserts) | 344 B | 413 B (1.20× plain) | 466 B (1.35× plain) | **0.89×** |
| Synthesised 1 049-op keystroke trace        | 853 B | 404 B (0.47× plain) | 776 B (0.91× plain) | **0.52×** |

Two headline points:

1. **Diamond-types is smaller in both regimes, and decisively smaller
   under keystroke-granular edit traces.** Its aggressive run-length
   encoding collapses runs of single-char inserts into a single
   `(agent, start_seq, len, pos, content)` tuple; automerge stores one
   op per keystroke even after column-compression. A realistic editing
   session (thousands of keystrokes, a handful of corrections) should
   see ~2× savings on the initial-sync payload.
2. **Diamond-types decodes ~80–130× faster.** The spike's cold-load
   benchmark averages 4.3 µs for DT and 338 µs for automerge on the
   small trace; 4.7 µs vs 599 µs on the synthesised one. That matters
   because `ws.rs::handle_socket` loads the full doc for every WebSocket
   reconnect — the decode cost was buried inside the broadcast relay
   and will disappear once we swap.

`EncodeOptions::ENCODE_FULL` (the `Default`) stores the start-branch
content; `ENCODE_PATCH` does not. On the traces the spike runs, both
encode modes happen to produce the same bytes because the start branch
is ROOT (empty). In the real protocol, `ENCODE_PATCH` becomes smaller
than `ENCODE_FULL` only when the receiver already knows some prefix of
history — i.e. for incremental `ServerMsg::Op` broadcasts. See §5.

### JSON-friendliness

Diamond-types' on-wire format is **binary**, not JSON. `ws.rs` already
base64-encodes `automerge` bytes for `ServerMsg::Sync { doc }` — the
same approach carries over unchanged. Diamond-types has an optional
`serde` feature, but it exposes the in-memory DAG for debugging rather
than a stable wire format; we should not use it. Stick with
`oplog.encode(EncodeOptions::default())` + base64.

## 3. Gaps vs the current automerge usage

These are the concrete places in `src/crdt/` and `src/web/ws.rs` that
the migration has to touch. Flagged per IMPL-029 phase so later tasks
can lift them verbatim.

### Gap 1 — no built-in rich-text marks layer

`src/crdt/mod.rs` leans on `automerge::marks::{Mark, ExpandMark}` and
`AutoCommit::{mark, unmark, marks, splice_text}` to implement Peritext
semantics (REQ-020-024 … REQ-020-027). `src/crdt/marks.rs` encodes
`MarkType → (name, ScalarValue, ExpandMark)` directly into automerge.
Diamond-types has **no equivalent**: it is a plain-text list CRDT.

*Affects:* every call site that uses `self.doc.mark(...)`,
`self.doc.unmark(...)`, `self.doc.marks()` (lines 142–193, 207, 221,
237 of `src/crdt/mod.rs`); the mark-name serialisation in
`src/crdt/marks.rs`; and `json_to_scalar` in `src/web/ws.rs`.

*Resolution:* §4 below.

### Gap 2 — `automerge::ScalarValue` leaks into the wire layer

`src/web/ws.rs::json_to_scalar` converts `serde_json::Value` →
`automerge::ScalarValue` inside the WebSocket handler, because
`MarkType::from_mark(name, &value)` takes `&ScalarValue`. Once automerge
is gone we need a project-owned `Scalar` enum (planned in Phase 6 of
IMPL-029) so `ws.rs` stops importing from `automerge::*`.

*Affects:* `src/web/ws.rs` lines 319–334 (the `json_to_scalar` helper
and its call site in `apply_ops`), and `MarkType::from_mark` +
`MarkType::scalar_value` in `src/crdt/marks.rs`.

### Gap 3 — structural-newline unmark dance

The `insert_structural_newline` helper (`src/crdt/mod.rs:136-164`)
relies on automerge's **after-the-fact** `unmark` plus `ExpandMark`
semantics: it inserts a `\n`, then walks the mark list and unmarks any
inclusive mark that just absorbed the newline. The diamond-types marks
layer will have to replicate that same two-step behaviour (insert with
whatever the mark's `expand` says, then clamp the mark span). If the
sibling-doc approach in §4 is taken, clamping is a trivial span edit
rather than an automerge-style `unmark` command.

### Gap 4 — fork semantics and agent-id management

Automerge forks generate a new actor-id internally. Diamond-types does
**not** — `oplog.clone()` produces a clone with the same agent ids.
Reusing an agent id after a fork corrupts the document (see the loud
warning in `diamond-types::lib.rs`: "Do not reuse IDs 💣"). Every code
path that currently calls `CrdtDocument::fork` must ensure the forked
copy then calls `get_or_create_agent_id(...)` with a **new** session id
before writing. That includes:

- `src/crdt/mod.rs:275-282` (`CrdtDocument::fork`)
- `src/web/ws.rs:594-605` (`CrdtDocStore::serialize_for_flush` — forks
  for safe serialisation outside the lock)

The serialize-for-flush path is read-only so is actually fine; but the
pattern elsewhere needs auditing.

### Gap 5 — no `ObjType::Map` / `ROOT` root container

`CrdtDocument::load` looks up `root.content` (line 268). Under DT the
oplog *is* the document — there's no ROOT container or key lookup.
Remove that indirection during Phase 3's trait extraction.

### Gap 6 — `automerge::marks::Mark<'_>` leaks into public signatures

`CrdtDocument::marks() -> Vec<Mark<'_>>` exposes automerge in the
public API (line 206). The Phase 3 trait has to return a
project-owned `Mark` struct so the marks-layer can be any backend.

### Non-gap — positions are already char-indexed

Both automerge and diamond-types index text by **Unicode code point
(char)**, not byte. The REQ-020 position semantics carry over
unchanged. The spike verifies this on `"café — 🌊"` (demo 4).

## 4. Recommended marks-layering strategy

The plan already scopes this to Phase 1 (`task-marks-layer-design`)
and asks for a written design note. The spike's recommendation to
feed into that design note is a **sibling diamond-types OpLog per
mark name** plus a tiny shift-on-splice adapter layer.

### Shape

- One `OpLog` holds the canonical text — this is what
  `splice_text`/`text` reads and what goes into `ServerMsg::Sync`
  today.
- Alongside it, a `MarksDoc` struct holds a `Vec<MarkSpan>` keyed by
  name, where `MarkSpan { name, value, start, end, expand }`. The
  span set is itself a CRDT — diamond-types does not ship a range
  CRDT, so we implement marks as append-only **span operations**
  layered on a second DT oplog that stores JSON-encoded
  `SpanOp::{Mark,Unmark}` entries (RLE-compressible; DT eats them
  efficiently). Alternatively, if the `more_types` branch lands in
  time, use its JSON map directly — but we should not block on that.
- Every text splice emits a companion `SpanOp::Shift(range, delta)`
  into the marks oplog so that loaded marks adjust to the current
  text positions. Because text+marks ops share agent-id + sequence
  numbers, they converge consistently under merge.

### Preserving Peritext expand semantics

Per-mark expand behaviour (`src/crdt/marks.rs:44-51`) stays in
`MarkType::expand()` — it is a property of the mark type, not of the
storage. The shift adapter consults it:

- `ExpandMark::Both` (bold, italic, strikethrough, highlight) →
  inserts at either boundary extend the span.
- `ExpandMark::None` (code, wikilink, link, comment) → inserts at the
  boundary do not extend.

The existing `insert_structural_newline` unmark dance becomes a pure
span-edit op: after inserting `\n` at `pos`, any span with
`expand=Both` whose new `end == pos+1` gets its `end` clamped back to
`pos`. No automerge-style round-trip needed.

### What disappears from the automerge surface

After migration, these automerge symbols have **no replacement** in
the marks layer (they all live in `src/crdt/mod.rs` /
`src/crdt/marks.rs` / `src/web/ws.rs`):

- `automerge::marks::{Mark, ExpandMark}`
- `automerge::transaction::Transactable`
- `automerge::{AutoCommit, ObjType, ReadDoc, ROOT, ScalarValue, ObjId}`

Replaced by:

- `diamond_types::list::{OpLog, Branch}` for text
- project-owned `crdt::marks::{Mark, ExpandMark, Scalar}`
- a `crdt::marks::MarksDoc` that wraps its own DT oplog

### Conflict resolution behaviour

Two editors setting the same exclusive mark (`Wikilink`, `Link`) over
overlapping ranges converge to a **last-writer-wins** result keyed by
(agent, seq) ordering — same as automerge's last-write semantics for
scalar mark values. Overlay marks (`Bold`, `Italic`, etc.) coexist;
the serialisation step already handles nested output.

Concurrent inserts at a mark boundary converge per `expand`: demo 5
confirms DT's char-position transforms produce a deterministic
merged order (`"XABZ"` from `X[A]Z` vs `X[B]Z`), so overlapping edits
*inside* a bold region remain inside the clamped span.

### Migration path (informs IMPL-029 phases 3–7)

1. **Phase 3** — Extract a `CrdtBackend` trait in `src/crdt/backend.rs`.
   Return project-owned `Mark` / `Scalar` from the trait; keep the
   existing automerge impl satisfying it. `src/web/ws.rs` migrates
   from `CrdtDocument` to `Box<dyn CrdtBackend>` here. **No marks
   behaviour change.**
2. **Phase 4** — `DiamondCrdtDocument::new/from_markdown/text/
   splice_text/save/load/fork/merge` — text-only, marks methods stub
   to a `not-yet-implemented` sentinel.
3. **Phase 5** — implement `MarksDoc` as the sibling-oplog described
   above; plumb through every `mark`/`unmark`/`marks` call.
4. **Phase 6** — swap `ws.rs`'s base64 payload from automerge bytes to
   DT bytes; retire `json_to_scalar`.
5. **Phase 7** — delete the automerge impl, remove from `Cargo.toml`.

### Known risks to carry into Phase 1

- **Mark compaction.** If `MarksDoc` is a raw append-only oplog, a
  long-lived document accrues span history indefinitely. Diamond-types
  offers no log-pruning story in 1.0. Mitigation: on quiescence
  flush, snapshot the *effective* mark set as the baseline and
  restart the marks oplog. The text oplog stays append-only; the
  marks oplog is rebuildable from the materialised marks at any
  time.
- **Dormant upstream.** Diamond-types 1.0.0 has been stable for years
  but the repo is quiet. If a bug surfaces we may have to maintain a
  fork. `diamond-types-extended = "0.2"` exists as a third-party fork
  but is less battle-tested — prefer upstream.
- **No SPEC-020-029 CRDT conformance test harness yet.** Phase 5 has
  to generate that (property tests for concurrent-splice-at-boundary
  for each `MarkType` variant) from scratch.

## 5. Acceptance checklist (matches `task-spike-diamond-types`)

- [x] Does diamond-types cover splice_text/save/load/fork/merge?
      — **Yes, via `OpLog::{add_insert, add_delete_without_content,
      encode, load_from, clone, decode_and_add}`** (demos 1–5).
- [x] What does the wire format look like (size, JSON-friendliness)?
      — **Binary, base64 on the wire. 0.52× automerge on keystroke
      traces, 0.89× on chunked run-inserts. ~80–130× faster to
      decode.**
- [x] Can marks be layered as a sibling structure, or do we need a
      different crate?
      — **Sibling diamond-types oplog per doc, carrying
      `SpanOp::{Mark,Unmark,Shift}` entries. No extra crate required.
      `diamond-types-extended` is an option but not needed.**
- [x] Recommended migration path
      — **See §4.** Phases 3–7 of IMPL-029 already line up.
- [x] Spike compiles and runs end-to-end
      — `cd tools/diamond-types-spike && cargo run --release` prints
      the seven demo sections cleanly.
