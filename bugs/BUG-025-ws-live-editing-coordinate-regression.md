---
id: BUG-025
title: Live collab editing silently fails on any note containing mark syntax (splice coordinates rejected server-side)
status: resolved
severity: S2
priority: P1
detection-method: live playtest — ar-crawl session driving the web editor against `zetl serve` + `zetld` on a scratch vault (SPEC-047 canonical-store workflow verification)
date: 2026-07-20
binary: zetl 0.9.3, branch `spec/047-loro-p2p` (post-review-fix commits 34f4366/78badcb; regression introduced earlier, by the SPEC-047 §9 engine swap)
vault: 2-note scratch vault; `Home.md` contains one `[[wikilink]]`
affects:
  - "[[WebSocket]] live editing (`/ws/edit/{slug}`) of any note whose Markdown contains inline-mark syntax: `[[wikilink]]`, `**bold**, *italic*`, `` `code` ``, links — every per-keystroke `Splice` op is rejected server-side"
  - quiescence auto-flush of such notes — the server doc never becomes dirty with the user's edits, and a later flush/merge can persist the stale server text (silent loss of everything typed since the last explicit Save)
not-affected:
  - notes containing no inline-mark syntax (control vault verified clean, including quiescence auto-flush)
  - the explicit Save button — `saveDoc()` is a plain HTTP `PUT` of the full buffer, bypassing the CRDT op stream entirely (which is why the loss is *silent*: the user's escape hatch works, masking the broken path)
  - the canonical-store path (`zetld` fold/materialise) — [[SPEC-047-loro-p2p-realtime-sync#REQ-484]] guarded import operates on whole files, not coordinates
---

# BUG-025: Live collab editing silently fails on notes containing mark syntax

## Specification Reference

- **Violates:** [[SPEC-020#REQ-020-028]] (WebSocket Editing Protocol — client ops must apply), [[SPEC-020#REQ-020-029]] (CRDT State Management — server state reflects client edits)
- **Resolved by:** [[SPEC-047-loro-p2p-realtime-sync#REQ-507]] / [[SPEC-047-loro-p2p-realtime-sync#ADR-483]] (F72)
- **Related:** [[SPEC-047-loro-p2p-realtime-sync#TEST-507a]] is the regression test that should have existed when the §9 engine swap changed the session document's text representation. [[BUG-026-ws-op-offsets-utf16-vs-scalar]] is the related latent coordinate defect.

## Environment

- macOS (Darwin 25.5.0), `zetl serve -d <vault> -p 3111` + `zetld` daemon on the same vault
- Browser: Chromium via `ar-crawl session` (Playwright); WebSocket status "connected"

## Steps to Reproduce

1. Vault with `Home.md` = `# Home\n\nWelcome to [[Second Note]].\n`
2. `zetl serve -d <vault>`; open `/edit/home` in a browser (WS connects)
3. Type any character anywhere in the buffer
4. Observe: server stderr prints `warning: CRDT op apply failed for home: loro text insert` for **every keystroke** (20/20 in the playtest); the on-disk file never receives the edits on quiescence flush; only the explicit Save button (HTTP PUT) persists them

## Expected Behaviour

Per [[SPEC-020#REQ-020-028]], client `op` messages apply to the server document; per [[SPEC-020#REQ-020-029]], the server CRDT state reflects the live edits and the quiescence flush persists them.

## Actual Behaviour

Every `Splice` op on a marked note is rejected. The client buffer shows the text; the server document never advances; `record_edit` still marks the doc dirty, so a quiescence flush can write the **stale server text over the file** — silently discarding everything typed since the last explicit Save.

## Evidence

- Playtest log: 20/20 keystroke ops → `CRDT op apply failed for home: loro text insert`; control vault (no mark syntax) 0 failures over the identical flow, both with and without the daemon
- Red-gate test failure output (before fix): server text `"# Home\n\nWelcome to Second Note.\n"` vs source `"# Home\n\nWelcome to [[Second Note]].\n"` — the 4 stripped bracket chars are exactly the coordinate offset

## Root Cause

- **Category:** design-error (with a test-gap accomplice)
- **Analysis:** The [[SPEC-047-loro-p2p-realtime-sync]] §9 engine swap replaced the session document's representation: diamond-types held the **raw Markdown source** as text; the rich-text [[Loro]] document stores **syntax-stripped plain text** with marks in the style layer. The deployed web client is [[CodeMirror]] over the raw source and sends splice positions in **source coordinates** (`update.changes.iterChanges → pos: fromA`). The implicit client/server coordinate contract — "server text ≡ the source the client edits" — was never written down as a requirement, so nothing red-flagged the swap: unit tests covered round-trips and merges, but no test asserted that a *source-coordinate op on a marked note* applies. The gap was only observable by driving the real client against the real server (found by playtest, not by the suite).
- **Why silent:** `apply_ops` failures are logged-and-dropped (`eprintln!` + the op is still relayed to other clients, who apply it fine since they share the client's coordinate frame) — so browsers stay mutually consistent while the server diverges.

## Resolution

- **Fix:** `LoroCrdtDocument::from_source` — the session document's text container holds the raw Markdown source verbatim (no mark parsing, no block tokens). All three `zetl serve` session-ingestion sites (initial load, clean fs-watch reload, dirty external merge) switched from `from_markdown` to `from_source`. Rich-text ingestion is confined to the canonical-store boundary (`set_content` / guarded import / materialise), per [[SPEC-047-loro-p2p-realtime-sync#ADR-483]].
- **Verified by:** `ws_doc_applies_source_coordinate_ops_on_marked_notes` ([[SPEC-047-loro-p2p-realtime-sync#TEST-507a]], observed red before the fix, green after), plus TEST-507b/c companions; live re-playtest: typing on the wikilink note auto-flushes on quiescence with zero op failures, and the session survives a daemon `materialise` (byte-identical exports are skipped).
- **Regression test added:** yes — `src/web/ws.rs::ws_doc_applies_source_coordinate_ops_on_marked_notes` and `src/crdt/loro_backend.rs` TEST-507b/c companions.

## AI Detection Context

- **Detecting model:** Claude Opus 4.8 (1M context)
- **Detection method:** live playtest (`ar-crawl session` driving the real editor), then controlled isolation (control vault without mark syntax; wikilink-free note on the daemon vault)
- **Confidence:** high — directly observed, minimal repro, red-gated
- **Session context:** SPEC-047 playtest session, 2026-07-20 (see memory `spec-047-loro-p2p`)
