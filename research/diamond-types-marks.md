# Marks-layer design (IMPL-029 / Phase 1)

This note is the input to Phases 3–5 of IMPL-029. It specifies the
replacement for automerge's `RichText` marks under the diamond-types
backend selected in [`diamond-types-spike.md`](diamond-types-spike.md).
The spike already concluded that a sibling diamond-types oplog is the
right shape; this note pins down the concrete types, the op model, the
per-mark-type payload, the Peritext (expand) semantics, the
merge/conflict rules, and the splice → mark-span shift algorithm.

It also enumerates every `automerge::*` symbol that disappears and what
replaces it, so the actual code-move tasks downstream can lift the
answer without re-deriving it.

Status: draft for review by hugo. Sign-off is recorded by
`hence task assert (given decided-marks-layer-design)`.

## 1. Decision

- **Shape**: a project-owned `MarksDoc` that wraps **a second
  `diamond_types::list::OpLog`**. The text oplog and the marks oplog
  share agent ids but are independent CRDT documents. Every write-path
  that touches text also touches marks atomically inside
  `DiamondCrdtDocument`.
- **Representation on the marks oplog**: each span edit is a
  JSON-encoded `SpanOp` (`Mark`, `Unmark`, `Shift`) appended as an
  `add_insert` at the oplog tail. Diamond-types then gets to do all
  the RLE / agent-ordering / merge bookkeeping for free.
- **Materialisation**: to read the current mark set, replay the marks
  oplog in DT's canonical order (the same order a `Branch::merge` to
  tip would produce) and fold the ops into a `Vec<Mark>`.
- **No extra crate.** We do not depend on `diamond-types-extended`, a
  new interval-CRDT crate, or a hand-rolled Lamport store. Everything
  rides on the same `diamond-types = "1.0"` pin Phase 2 adds.

The alternatives considered and rejected are in [§10](#10-alternatives-considered).

## 2. Data model

All types live in `src/crdt/marks.rs` (existing `MarkType` stays
unchanged — only its `scalar_value`/`from_mark` plumbing retargets).

```rust
// src/crdt/marks.rs

/// Project-owned mark returned from CrdtBackend::marks().
/// Replaces automerge::marks::Mark<'_>.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    pub name: String,
    pub value: Scalar,
    pub start: usize,   // char index in the text doc at current frontier
    pub end: usize,     // exclusive
    pub expand: ExpandMark,
}

/// Project-owned scalar. Replaces automerge::ScalarValue inside
/// src/crdt/ and src/web/ws.rs.  Keeps the subset we actually store
/// — no counters, timestamps, or bytes — so (de)serialisation is
/// cheap and wire-stable.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Scalar {
    Bool(bool),
    Str(String),
    // Reserved; not emitted by any MarkType today but cheap to carry
    // so future marks (e.g. callout severity) don't need a wire bump.
    Int(i64),
    Null,
}

/// Project-owned expand enum. Replaces automerge::marks::ExpandMark.
/// Kept to exactly the two variants zetl emits today — adding
/// Before/After is a one-line change if a future MarkType needs them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpandMark {
    /// Text inserted at either boundary inherits the mark.
    Both,
    /// Text inserted at either boundary does NOT inherit the mark.
    None,
}
```

```rust
// src/crdt/marks_doc.rs  (new)

/// Span-level op stored in the sibling oplog. One entry per
/// Mark/Unmark/Shift call.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum SpanOp {
    /// Set a mark over [start, end). `expand` is decided by the
    /// caller's MarkType (not stored on the text oplog itself).
    Mark {
        name: String,
        value: Scalar,
        start: usize,
        end: usize,
        expand: ExpandMark,
    },
    /// Clear any mark named `name` overlapping [start, end).
    /// `value` is None for "unmark all" (how `unmark()` is called
    /// today); Some(_) when the caller wants to unmark one specific
    /// value (not used in zetl today but kept for forward compat).
    Unmark {
        name: String,
        value: Option<Scalar>,
        start: usize,
        end: usize,
        expand: ExpandMark,
    },
    /// Shift every open span's endpoint(s) to track a text splice.
    /// Emitted automatically by DiamondCrdtDocument::splice_text.
    Shift {
        /// Char position in the text-at-authoring-frontier.
        pos: usize,
        /// Chars inserted (> 0) or deleted (< 0) at `pos`.
        delta: i64,
    },
}

pub struct MarksDoc {
    oplog: diamond_types::list::OpLog,
    agent: diamond_types::AgentId,
}
```

### On-the-wire

Each `SpanOp` is serialised with `serde_json::to_string` and then
passed to `oplog.add_insert(agent, oplog.len_chars(), &json + "\n")`.
That is: the marks oplog is literally a newline-delimited JSON stream
stored inside a DT text document. Diamond-types does the heavy
lifting:

- RLE packing of appended runs (each `SpanOp` is one line → one run,
  not one char-op).
- Agent-id / seq-number ordering for merge.
- Cheap binary wire-format via `encode(EncodeOptions::default())`.

We never call `.content().to_string()` on the marks oplog in
production. We iterate operations via `oplog.iter()`, collect the
per-op inserted strings, parse them as JSON, and fold into spans.

### Why not store the Vec<Mark> directly in the text oplog as a prefix

Tempting — but the text oplog is what clients diff-sync via
`ServerMsg::Op`, and splicing marks into its character stream would
(a) corrupt offsets for text splices and (b) complicate serialise →
markdown. Two parallel documents is cleaner.

## 3. Per-`MarkType` payload mapping

The `MarkType` variants in `src/crdt/marks.rs` stay byte-identical on
the user-visible surface. What changes is the backing scalar type —
every call that built an `automerge::ScalarValue` now builds a
`crdt::marks::Scalar`. The name, growth behaviour, nesting order, and
conflict mode are unchanged.

| `MarkType` variant      | mark name       | `Scalar` payload                | `ExpandMark` | Conflict      | Nesting |
| ----------------------- | --------------- | ------------------------------- | ------------ | ------------- | ------- |
| `Bold`                  | `"bold"`        | `Scalar::Bool(true)`            | `Both`       | Overlay       | 1       |
| `Italic`                | `"italic"`      | `Scalar::Bool(true)`            | `Both`       | Overlay       | 2       |
| `Strikethrough`         | `"strikethrough"` | `Scalar::Bool(true)`          | `Both`       | Overlay       | 0       |
| `Highlight`             | `"highlight"`   | `Scalar::Bool(true)`            | `Both`       | Overlay       | 4       |
| `Code`                  | `"code"`        | `Scalar::Bool(true)`            | `None`       | Overlay       | 3       |
| `Comment`               | `"comment"`     | `Scalar::Bool(true)`            | `None`       | Overlay       | 5       |
| `Link { url }`          | `"link"`        | `Scalar::Str(url)`              | `None`       | LWW per span  | 6       |
| `Wikilink { target, alias: None }` | `"wikilink"` | `Scalar::Str(target)`      | `None`       | LWW per span  | 7       |
| `Wikilink { target, alias: Some(a) }` | `"wikilink"` | `Scalar::Str("{target}|{alias}")` | `None` | LWW per span  | 7       |

Notes:

- **Payload stability.** The `Bool(true)` sentinel for presence-only
  marks (bold, italic, code, strikethrough, highlight, comment) matches
  the existing automerge encoding — so WAL entries serialised by the
  automerge backend before the swap can still be deserialised into
  `Scalar::Bool` with no format migration (the wire format today is
  already JSON in `OpEntry::Mark.value`; see `src/web/ws.rs:145-162`).
  That keeps `.zetl/wal/` readable across the cut-over.
- **Wikilink alias encoding** keeps the `"target|alias"` string form
  from `MarkType::scalar_value()` unchanged. Any future schema change
  (e.g. structured payloads) is a separate task.
- **`link` payload** is the URL string, matching today's behaviour.
  Malformed URLs are not validated at the CRDT layer; the markdown
  serialiser handles escaping.
- **Unmark** today is called with a name + range only (no value). That
  stays true: `SpanOp::Unmark.value` is always `None` in the caller
  path today. The `Some(_)` slot exists so a future "unmark this
  specific link" use case doesn't need a format bump.

## 4. Peritext expand semantics preservation

Expand behaviour is a property of the `MarkType`, not of the storage —
the same `MarkType::expand()` method that drives automerge today
drives the diamond-types backend. The reason Peritext semantics
survive is that **we replay the marks oplog in DT's canonical merge
order, carrying `expand` on each Mark/Unmark op**, and apply each
`Shift` with per-span growth awareness.

### Rules the materialiser applies

Given a current mark set `S = [MarkSpan { name, value, start, end, expand }, …]`
and an incoming op:

**Insert shift (`SpanOp::Shift { pos, delta > 0 }`)** — a text insert
of `delta` chars at `pos`:

```
for span in S:
    if pos > span.end:
        pass                           # entirely after the span; no change
    elif pos < span.start:
        span.start += delta            # entirely before; shift right
        span.end   += delta
    elif pos == span.start:
        if span.expand == Both:
            span.end += delta          # boundary-at-start: grow
        else:
            span.start += delta        # non-growing: push off
            span.end   += delta
    elif pos == span.end:
        if span.expand == Both:
            span.end += delta          # boundary-at-end: grow
        else:
            pass                       # non-growing: leave behind
    else:                              # pos strictly inside
        span.end += delta              # grow regardless of expand
```

**Delete shift (`SpanOp::Shift { pos, delta < 0 }`)** — `|delta|`
chars deleted starting at `pos`:

```
let del_start = pos
let del_end   = pos + |delta|
for span in S:
    if del_end <= span.start:
        span.start -= |delta|
        span.end   -= |delta|
    elif del_start >= span.end:
        pass
    else:                              # overlap
        # clamp the span to the surviving characters
        new_start = min(span.start, del_start)
        new_end   = max(span.end, del_end) - |delta|
        new_end   = max(new_end, new_start)
        span.start = new_start
        span.end   = new_end
        if span.start == span.end:
            drop span                  # span fully covered by delete
```

**Mark op (`SpanOp::Mark { name, value, start, end, expand }`)**:

```
if MarkType::from_name(name).is_exclusive():
    # LWW: drop any prior span with the same name whose range
    # overlaps [start, end). Ordering is decided by DT's merge order,
    # so "last" = "later in the canonical replay", which is exactly
    # what DT gives us for free.
    S.retain(|s| !(s.name == name && overlaps(s, start..end)))
S.push(MarkSpan { name, value, start, end, expand })
# overlay marks just accumulate; dedup happens at serialise time
```

**Unmark op (`SpanOp::Unmark { name, start, end, expand }`)**:

```
# Carve any span with the same name out of [start, end).
# If value is Some(_), only remove spans whose value matches.
new_spans = []
for s in S:
    if s.name != name || (value.is_some() && s.value != value):
        new_spans.push(s); continue
    # Subtract the unmark range from this span
    if end <= s.start || start >= s.end:
        new_spans.push(s)              # disjoint
    elif start <= s.start && end >= s.end:
        continue                       # fully covered → drop
    elif start > s.start && end < s.end:
        new_spans.push(split_left(s, start))
        new_spans.push(split_right(s, end))
    elif start <= s.start:
        new_spans.push(clip_left(s, end))
    else:
        new_spans.push(clip_right(s, start))
S = new_spans
```

This mirrors automerge's `unmark` carve-out behaviour exactly, and it
is what `insert_structural_newline` in `src/crdt/mod.rs:136` depends
on (the after-the-insert unmark that strips inclusive marks from the
`\n`).

### `insert_structural_newline` becomes a local span edit

Today:
1. `splice_text(pos, 0, "\n")`.
2. Query `doc.marks()`, find inclusive marks whose `end == pos+1 &&
   start <= pos`.
3. Emit one `doc.unmark(name, pos, pos+1, expand)` per hit.

After migration, step 2 is a span scan over `materialised_marks` and
step 3 is a single `SpanOp::Unmark` appended to the marks oplog. No
round-trip through automerge's query layer; no `ExpandMark` nuance on
the unmark call (we pass the mark type's expand through for
symmetry).

## 5. Splice → mark-span shift algorithm

The critical path — this is what every keystroke hits.

```rust
// src/crdt/diamond.rs (Phase 5 sketch)
impl DiamondCrdtDocument {
    pub fn splice_text(&mut self, pos: usize, del: isize, text: &str)
        -> Result<()>
    {
        // 1. Apply to the text oplog.
        if del > 0 {
            self.text.add_delete_without_content(self.agent, pos..pos+del as usize);
            self.marks.append(SpanOp::Shift { pos, delta: -(del as i64) })?;
        }
        if !text.is_empty() {
            self.text.add_insert(self.agent, pos, text);
            let n = text.chars().count() as i64;
            self.marks.append(SpanOp::Shift { pos, delta: n })?;
        }
        Ok(())
    }
}
```

Two properties this buys:

1. **Local edits are cheap.** A splice appends one Shift to the marks
   oplog and DT coalesces successive shifts into a single RLE run
   (the marks oplog is itself a text stream of JSON lines; consecutive
   Shift lines pack well).
2. **Concurrent shifts compose on merge.** When Alice's `Shift(5, +3)`
   and Bob's `Mark("bold", 10, 15)` arrive on a third peer, DT merges
   the marks oplog into a canonical order. The materialiser plays the
   shifts and marks in that order — if Alice's shift sorts before
   Bob's mark, Bob's mark appears at [13, 18) (shifted); if it sorts
   after, Bob's mark appears at [10, 15) (unshifted). Either outcome
   is a valid Peritext convergence, and it is deterministic across
   all peers.

### Concurrent insert at a mark boundary

Consider the canonical Peritext case: **Alice has `[[Project X]]` at
chars [5, 14), a wikilink with `ExpandMark::None`. Bob types ` foo` at
char 14 (right after the `]]`).**

Serialised events after merge:
- `SpanOp::Mark { name: "wikilink", value: "Project X", start: 5,
  end: 14, expand: None }`  ← Alice
- `SpanOp::Shift { pos: 14, delta: +4 }`  ← Bob

Replay in DT order: Mark first (Alice's agent seq pre-dates Bob's edit
if she inserted the wikilink before Bob opened the page), then Shift.
Applying Shift at `pos == span.end` with `span.expand == None` — no
change (rule `elif pos == span.end: if Both: grow; else: pass`).
Bob's text lands at [14, 18) outside the wikilink. Correct.

Bold case instead: **Alice `**Hello**` at [0, 5) with `ExpandMark::Both`;
Bob types `!` at char 5.** Replay: Mark, then Shift(5, +1). At
`pos == span.end && span.expand == Both`: grow → span becomes [0, 6).
Bob's `!` is bolded. Correct.

Non-boundary insert case: **Alice `**Hello**` at [0, 5); Bob types `X`
at char 3.** Shift(3, +1) with `pos strictly inside span` → grow →
span becomes [0, 6). Bob's `X` sits inside bold. Correct regardless
of expand — both Peritext and automerge agree here.

### Concurrent insert BETWEEN two ops on the same agent

A known hazard: if Alice locally does
`splice_text(0, 0, "Hi ")` *then* `mark("bold", 3, 8)`, the `3` and
`8` are Alice-local char positions. If Bob concurrently inserted `X`
at position 0 on his replica, after merge Alice's "3" no longer
refers to the same character. **The Shift ops Alice emitted fix this
at replay time** — Bob's `Shift(0, +1)` and Alice's `Mark(…, 3, 8)`
merge into the marks oplog; whichever sorts first dictates the
outcome, both are legal Peritext convergences.

The failure mode to avoid: **applying Alice's Mark op to the text-tip
positions instead of to the materialised mark set at the op's place
in the replay**. The implementation must materialise-by-replay, not
materialise-against-tip.

## 6. Conflict resolution behaviour

Per REQ-020-025 there are two conflict classes, both handled at
materialisation time:

### Overlay marks (bold, italic, strikethrough, highlight, code, comment)

Concurrent marks coexist. Two bolds over overlapping ranges merge
into their set union; the serialiser emits one `**…**` per contiguous
bold run. The replayer just `push`es; the serialiser already does
run-merging when it walks active marks at each char position
(`serialize_to_markdown` in `src/crdt/mod.rs:578`).

### Exclusive marks (`wikilink`, `link`)

Last-write-wins per span. If Alice and Bob both mark chars [5, 14) as
different wikilinks, the one whose `SpanOp::Mark` sorts later in DT's
canonical order survives. DT's canonical order is Lamport-total — so
this matches the "Lamport timestamp ordering of the opId" contract in
REQ-020-025.

The `is_exclusive()` check on the materialiser is the only place that
needs to distinguish. The `MarkType::is_exclusive` method already
exists (`src/crdt/marks.rs:125`) and stays verbatim.

### Mark vs Unmark race

If Alice marks [5, 14) as bold and Bob concurrently unmarks [0, 20)
of the same name, the one that sorts later wins — same rule. Small
subtlety: if Alice's mark sorts later, her [5, 14) survives because
the unmark only saw a pre-existing mark set that didn't yet contain
[5, 14). Applied in replay order this falls out naturally.

## 7. Conformance to SPEC-020

REQ-020-024 (Peritext CRDT editing layer): stays satisfied — the
marks layer implements Peritext's inclusive/non-growing span
semantics, now over a DT text oplog instead of an automerge Text.

REQ-020-025 (Peritext mark types): the 8 variants of `MarkType` are
unchanged, and §3's table shows payload/expand/conflict mode
preserved 1:1.

REQ-020-026 (block-level structure): orthogonal — block tokens live
in the text oplog as ordinary chars; the marks layer never touches
them.

REQ-020-027 (canonical serialisation): serialiser signature becomes
`fn serialize_to_markdown(text: &str, marks: &[Mark]) -> Result<String>`
— same body, just the `Mark<'_>` input type swaps to the project-owned
`Mark`. Nesting order is already a `MarkType::nesting_order`
property, which doesn't move.

REQ-020-028 (WebSocket protocol): the `OpEntry::{Splice,Mark,Unmark}`
JSON shape in `src/web/ws.rs:142-162` stays byte-identical on the
wire. `json_to_scalar` changes its return type from
`automerge::ScalarValue` to `crdt::marks::Scalar`; no wire-format
migration needed.

## 8. `automerge::*` symbol retirement

Every `automerge::*` import in `src/crdt/` or `src/web/` disappears
after Phase 7. Below is the full replacement map. Call sites are the
ones to audit during Phases 3–7.

| automerge symbol (today)                        | Call sites                                                                                              | Replacement                                                           |
| ----------------------------------------------- | ------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------- |
| `automerge::AutoCommit`                         | `src/crdt/mod.rs:28, 265, 277`                                                                          | `diamond_types::list::OpLog` (text) + `MarksDoc` (marks)              |
| `automerge::ObjType::Text`                      | `src/crdt/mod.rs:30`                                                                                    | *(none — DT oplog has no root container)*                             |
| `automerge::ObjId`                              | `src/crdt/mod.rs:22, 280`                                                                               | *(none — DT is a single-rooted document)*                             |
| `automerge::ROOT`                               | `src/crdt/mod.rs:30, 268`                                                                               | *(none)*                                                              |
| `automerge::ReadDoc` (trait)                    | `src/crdt/mod.rs:13`                                                                                    | *(none — replaced by `Branch::content()` / `OpLog::checkout_tip`)*    |
| `automerge::transaction::Transactable` (trait)  | `src/crdt/mod.rs:12`                                                                                    | *(none — DT ops are free fns on `OpLog`)*                             |
| `automerge::marks::Mark<'_>`                    | `src/crdt/mod.rs:11, 206, 221, 236, 578, 586, 599` and every test using `doc.marks()`                   | `crdt::marks::Mark` (owned)                                           |
| `automerge::marks::ExpandMark`                  | `src/crdt/mod.rs:11, 142, 160, 191, 228, 241`; `src/crdt/marks.rs:1, 44-51, 159-183`                    | `crdt::marks::ExpandMark`                                             |
| `automerge::ScalarValue`                        | `src/crdt/marks.rs:2, 59, 77, 146`; `src/web/ws.rs:319-334`                                             | `crdt::marks::Scalar`                                                 |
| `AutoCommit::new()`                             | `src/crdt/mod.rs:28`                                                                                    | `OpLog::new()`                                                        |
| `AutoCommit::load(&bytes)`                      | `src/crdt/mod.rs:265`                                                                                   | `OpLog::load_from(&bytes)`                                            |
| `doc.put_object(ROOT, "content", Text)`         | `src/crdt/mod.rs:30`                                                                                    | *(deleted)*                                                           |
| `doc.get(ROOT, "content")`                      | `src/crdt/mod.rs:268`                                                                                   | *(deleted)*                                                           |
| `doc.splice_text(&tid, pos, del, text)`         | `src/crdt/mod.rs:62, 74, 83, 93, 109, 138, 174, 213`                                                    | `oplog.add_insert(agent, pos, text)` + `oplog.add_delete_without_content(agent, pos..pos+del)` + `MarksDoc::shift(pos, delta)` |
| `doc.text(&tid)`                                | `src/crdt/mod.rs:202`                                                                                   | `oplog.checkout_tip().content().to_string()`                          |
| `doc.mark(&tid, Mark::new(...), expand)`        | `src/crdt/mod.rs:183, 220`                                                                              | `MarksDoc::mark(SpanOp::Mark { … })`                                  |
| `doc.unmark(&tid, name, start, end, expand)`    | `src/crdt/mod.rs:159, 236`                                                                              | `MarksDoc::unmark(SpanOp::Unmark { … })`                              |
| `doc.marks(&tid)`                               | `src/crdt/mod.rs:143, 207, 249`                                                                         | `MarksDoc::materialise()` → `Vec<Mark>`                               |
| `doc.save()`                                    | `src/crdt/mod.rs:260`                                                                                   | `(text_oplog.encode(ENCODE_FULL), marks_oplog.encode(ENCODE_FULL))` zipped into a single framed blob (format: u32 text_len, text_bytes, marks_bytes) |
| `doc.fork()`                                    | `src/crdt/mod.rs:277`; `src/web/ws.rs:601`                                                              | `text_oplog.clone()` + `marks_oplog.clone()` + **mandatory fresh agent id on each clone** (see Phase-0 gap 4) |
| `doc.merge(&mut other)`                         | `src/crdt/mod.rs:286`                                                                                   | `merged.text.decode_and_add(&other.text.encode(..))` + same for marks |
| `Mark::new(name, value, start, end)`            | `src/crdt/mod.rs:184, 222`                                                                              | `crdt::marks::Mark { name, value, start, end, expand }` struct literal |
| `ScalarValue::from(bool)` / `ScalarValue::from(String)` / `ScalarValue::from(i64)` / `ScalarValue::from(f64)` | `src/crdt/marks.rs:62-73`; `src/web/ws.rs:321-332`                        | `Scalar::Bool` / `Scalar::Str` / `Scalar::Int` / *(`f64` path removed — no MarkType produces it)* |
| `ScalarValue::Str(smol) => smol.to_string()` pattern in `scalar_to_string` | `src/crdt/marks.rs:146-150`                                                  | `match s { Scalar::Str(s) => Some(s.clone()), _ => None }`            |

Net surface after Phase 7:

- **In `src/crdt/`** only `diamond_types::list::{OpLog, Branch}` and
  `diamond_types::AgentId` are imported from third-party CRDT crates.
- **In `src/web/ws.rs`** there is no CRDT-crate import at all — every
  type that crosses the ws boundary is `crdt::marks::{Mark, Scalar,
  ExpandMark}` or `crdt::backend::CrdtBackend`. This is the goal
  `task-ws-wire-format` codifies.

## 9. Known risks / open questions

These are the things that need hugo's sign-off before implementation
begins, or that I expect to learn something about during Phase 5.

1. **Mark-op ordering correctness under deep concurrent branches.**
   The materialiser's "replay in DT's canonical order" relies on DT
   producing a total order that agrees between peers. The spike
   verifies this for simple 2-peer splice concurrency (demo 5).
   *Unverified*: 3-way merges with interleaved mark + shift ops. The
   Phase 5 property-test suite (proptest, 4 replicas × random
   interleaved mark/splice ops, assert convergence) is where this
   gets proven.
2. **Mark-oplog compaction.** A long-lived doc accrues SpanOps
   forever. Mitigation (carried over from the spike note): on
   quiescence flush, snapshot the materialised `Vec<Mark>` as a
   rebase baseline and restart the marks oplog from a single
   "sync-all" SpanOp variant. Text oplog stays append-only; marks
   oplog is rebuildable at any time. Not required for Phase 5
   correctness but should ship by Phase 6 to keep WAL size bounded.
3. **Shift vs. frontier-stamped positions.** The design uses
   position-based Shift ops. A strictly more robust alternative would
   anchor marks to DT character IDs (Peritext's original approach),
   but DT 1.0 does not cleanly expose char-id handles to third-party
   code. If Phase 5's property tests uncover convergence bugs that
   Shift-based replay can't fix, the escape hatch is the
   frontier-stamping variant outlined in §10.
4. **Unmark payload.** Today's `CrdtDocument::unmark` never passes a
   value; `SpanOp::Unmark.value` is forever `None`. I kept the slot
   open for forward compat but we could delete it if hugo prefers a
   tighter schema.
5. **`Scalar::Int` / `Scalar::Null`.** No current `MarkType` emits
   these. They exist so future marks (callouts, tags-with-colour) do
   not need a wire-format bump. Open for deletion if hugo wants the
   enum minimal.
6. **WAL compatibility.** §3 claims `Scalar::Bool(true)` decodes
   cleanly from today's JSON WAL payloads because both backends
   already serialise marks as `serde_json::Value`. This is a claim
   that `task-ws-wire-format` in Phase 6 should assert on with a
   fixture test; if it turns out false, `.zetl/wal/` clears on
   upgrade — that's already an accepted outcome in the plan
   (`task-ws-wire-format.description`, "WAL format change is
   acceptable").

## 10. Alternatives considered

**Extend `src/crdt/mod.rs` with a hand-rolled mark log over a `Vec`
+ `Lamport` timestamps.** Rejected — we'd be reimplementing
agent/seq/RLE merging that DT already solves. Every bug DT has fixed
over its life we'd re-introduce.

**Use `diamond-types-extended = "0.2"`.** It has a JSON object model
that would naturally fit `Mark` records. Rejected — third-party fork
of DT, less battle-tested, introduces a second upstream dependency.
If DT 1.0 proves insufficient we can revisit.

**Bring in `crdt-marks` / `automerge` peritext as an isolated
crate.** There is no such crate on crates.io with a Rust-native
Peritext implementation independent of automerge. Would need to write
one — that is exactly what this task is avoiding by riding DT.

**Use an interval CRDT (Yjs-style `Y.Map` of ranges).** No Rust
interval-CRDT crate is maintained enough to depend on (`y-crdt` ships
Map support but not a range-backed formatting layer). Building one is
larger than the sibling-oplog approach.

**Frontier-stamped positions instead of Shift ops.** Each Mark op
carries the text `Frontier` at the time it was authored; replay
transforms positions through DT's version-vector machinery. More
Peritext-correct (the transform is implicit from CRDT structure), but
DT 1.0's public API does not expose position-transform against an
arbitrary prior frontier. Workable only if we vendor a small patch to
DT. Held in reserve — §9-3.

**Inline marks as characters in the text oplog.** Encode mark
open/close as invisible control characters. Rejected — breaks char
positions for text splices, complicates markdown serialisation, and
doesn't interact well with inclusive/non-growing boundaries.

## 11. Phase-5 implementation checklist

A concrete list for when `task-diamond-marks-layer` picks this up:

- [ ] `src/crdt/marks.rs`: replace `use automerge::*` with the
      project-owned `Scalar` and `ExpandMark` types defined in §2.
- [ ] `src/crdt/marks.rs`: retarget `scalar_value()` to return
      `Scalar`; retarget `from_mark(&Scalar)` to match.
- [ ] `src/crdt/marks_doc.rs` (new): implement `MarksDoc` per §2 with
      `mark`, `unmark`, `shift`, `materialise`, `encode`, `load_from`,
      `clone`, `decode_and_add`.
- [ ] `src/crdt/diamond.rs`: wire `MarksDoc` into
      `DiamondCrdtDocument`; `splice_text` emits paired Shift ops per
      §5.
- [ ] Materialiser: implement §4's six rules (insert/delete × overlap
      classes; mark with LWW; unmark with carve-out; no-op
      everywhere else).
- [ ] `src/crdt/mod.rs`'s `insert_structural_newline`: rewrite to use
      the materialised mark set + one `SpanOp::Unmark` per hit.
- [ ] Property tests (`proptest`, cfg-gated): 4 replicas, random
      interleaved {splice, mark, unmark} ops, assert
      (a) convergence, (b) inclusive-boundary growth, (c)
      non-growing-boundary preservation, (d) save/load identity.
- [ ] Conformance: run the existing `src/crdt/mod.rs` test module
      verbatim against the diamond backend; all 20+ round-trip tests
      must pass unchanged.
