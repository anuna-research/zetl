---
id: BUG-026
title: WebSocket splice offsets are UTF-16 code units client-side but unicode scalars server-side — positions drift after astral characters
status: confirmed
severity: S3
priority: P3
detection-method: code inspection during the BUG-025 root-cause analysis (not yet observed live)
date: 2026-07-20
binary: zetl 0.9.3, branch `spec/047-loro-p2p` — but the mismatch predates the branch (diamond-types also spliced by unicode scalar)
vault: any note containing characters outside the Basic Multilingual Plane (emoji, many CJK extensions) before the edit position
affects:
  - "[[WebSocket]] live editing of notes containing astral-plane characters: every such character before a splice position shifts the client offset (+1 UTF-16 code unit) relative to the server's scalar index — edits land one-or-more positions off, or out of range"
not-affected:
  - notes whose content is entirely BMP (the two unit systems agree) — the overwhelmingly common case for prose vaults, hence S3
  - the explicit Save path (HTTP PUT of the full buffer)
---

# BUG-026: WS splice offsets — UTF-16 (client) vs unicode scalar (server)

## Specification Reference

- **Violates:** [[SPEC-047-loro-p2p-realtime-sync#REQ-507]]'s coordinate-fidelity intent in its astral-character corner; the declared unit of `pos`/`del` in the op message was never specified in [[SPEC-020#REQ-020-028]] — partially a **specification gap**
- **Related:** [[BUG-025-ws-live-editing-coordinate-regression]] (the dominant coordinate defect, fixed by [[SPEC-047-loro-p2p-realtime-sync#ADR-483]]); this residue survives that fix

## Analysis

[[CodeMirror]] documents index by UTF-16 code units (`fromA`/`toA` in
`iterChanges`), while `LoroText::insert`/`delete` (and previously
diamond-types' rope splice) index by unicode scalar values. `"🦀x"` is 3
code units client-side but 2 scalars server-side: a client splice "after
the x" arrives one past the server's end. Consequences range from
misplaced characters to rejected ops, growing with the number of astral
characters preceding the edit.

Pre-existing (the diamond engine had the same unit): **not** a
regression of the §9 engine swap, and not fixed by
[[SPEC-047-loro-p2p-realtime-sync#ADR-483]], which aligns the *text
content* but not the *index unit*.

## Proposed Resolution (deferred — owner: M2 WebSocket-attachment slice / HOC)

Declare the op-message coordinate unit explicitly in the CON for the WS
editing protocol (the LangSec grammar for `Splice` should state the
unit), then either (a) convert UTF-16 → scalar server-side against the
session text before applying (single recogniser, no client change), or
(b) adopt the loro-wasm client in the M2 attachment slice, which
carries its own consistent indexing. Option (a) is the rung-5 fix if
this is prioritised before M2.

Deferral rationale: prose vaults are BMP-dominated; BUG-025's fix
restores correct behaviour for the common case; the M2 slice will
rework this exact boundary. Recorded here so the ceiling is visible
debt, not invisible debt.

## AI Detection Context

- **Detecting model:** Claude Opus 4.8 (1M context)
- **Detection method:** static analysis during BUG-025 root-cause work
- **Confidence:** medium — inferred from documented API semantics (CodeMirror UTF-16, Loro scalar); no live repro run yet
