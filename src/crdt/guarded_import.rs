//! Guarded import of external Markdown edits (SPEC-047 REQ-484, ADR-471,
//! CON-471, IMPL-047 T4).
//!
//! When something *other than the daemon* writes a note's Markdown file, the
//! daemon must decide whether to **fold** that edit into the canonical [[Loro]]
//! store or **stage** it to a conflict area — never silently discarding either
//! side. The decision is made from *recorded state* over [[Loro]] logical time,
//! never guessed from content (F59): folding a stale-based edit would convert
//! the daemon's materialised edits into deletions.
//!
//! This module is the pure decision core ([`decide`]) plus a thin orchestrator
//! ([`import_external`]). The decision is the load-bearing, LangSec-adjacent
//! part; it is exhaustively unit-tested. Delete/rename folding as manifest ops
//! (F38/F64) belongs to the namespace manifest (T5) and is out of scope here.

use super::loro_store::NoteDoc;
use anyhow::Result;
use std::path::PathBuf;

/// Whether the external save's bytes match a retained export generation
/// exactly (an *unchanged* save — its base is known) or match none (an
/// *edited* save — its base must be inferred from recorded state). Content
/// hashing can only recognise an unchanged save; an edited buffer matches no
/// retained hash, so "which export did these bytes derive from" is unknowable
/// from content alone (F59).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalWrite {
    /// Content hash equals a retained export generation — the base is known.
    Unchanged,
    /// Content hash matches no retained export — an edited buffer.
    Edited,
}

/// Recorded state the fold/stage decision is a function of (CON-471 C2:
/// decided from recorded state, never guessed from content).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportState {
    /// The write matched a retained export hash, or not.
    pub write: ExternalWrite,
    /// A daemon op exists for this document since `last_export` that has not
    /// been re-materialised — so the editor's base is stale.
    pub unmaterialised_daemon_op: bool,
    /// An export generation was materialised for this document since the last
    /// external event on it (the editor may have opened E0 while the daemon
    /// later wrote E1) — so an *edited* save's base is ambiguous.
    pub intervening_export: bool,
}

/// The fold/stage decision (CON-471 C5: `Folded | Staged`, discards neither).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportDecision {
    /// Safe to fold: the external base is unambiguous.
    Fold,
    /// Stage to the conflict area: folding could lose data. Carries the reason.
    Stage(&'static str),
}

/// The pure fold/stage decision (REQ-484, F59). Conservative: it folds only
/// when exactly one base is *possible*, and over-stages the ambiguous
/// mixed-editing case (a false positive — never data loss).
pub fn decide(state: ImportState) -> ImportDecision {
    // An unmaterialised daemon op means the file on disk is not the current
    // canonical base; the external editor necessarily worked from a stale
    // export. Fold would drop the daemon's ops — stage.
    if state.unmaterialised_daemon_op {
        return ImportDecision::Stage("unmaterialised daemon op — external base is stale");
    }
    match state.write {
        // An unchanged save's base is exactly the retained export it hashes to;
        // folding it is safe (at worst a no-op).
        ExternalWrite::Unchanged => ImportDecision::Fold,
        // An edited save's base is knowable only if nothing re-exported since
        // the last external event; otherwise the base is ambiguous — stage.
        ExternalWrite::Edited => {
            if state.intervening_export {
                ImportDecision::Stage("intervening export — edited-save base is ambiguous")
            } else {
                ImportDecision::Fold
            }
        }
    }
}

/// The outcome of importing an external write (CON-471 C5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportOutcome {
    /// Folded into the canonical store.
    Folded,
    /// Staged to the conflict area at this path (bytes preserved).
    Staged(PathBuf),
}

/// Orchestrate a guarded import: decide, then either fold the Markdown into
/// `note` or hand back a `Staged` outcome for the caller to write to the
/// conflict area. `stage_path` is where a staged copy would live (the caller
/// performs the write, keeping this function free of the conflict-area I/O so
/// the decision stays testable).
///
/// The Markdown → Loro fold replaces the note body via the canonical rich-text
/// ingestion (`NoteDoc::set_content`).
pub fn import_external(
    note: &mut NoteDoc,
    markdown: &str,
    state: ImportState,
    stage_path: impl Into<PathBuf>,
) -> Result<ImportOutcome> {
    match decide(state) {
        ImportDecision::Fold => {
            note.set_content(markdown)?;
            Ok(ImportOutcome::Folded)
        }
        ImportDecision::Stage(_reason) => Ok(ImportOutcome::Staged(stage_path.into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(write: ExternalWrite, unmat: bool, intervening: bool) -> ImportState {
        ImportState {
            write,
            unmaterialised_daemon_op: unmat,
            intervening_export: intervening,
        }
    }

    // TEST-484a (positive): a clean external edit with a known base folds.
    #[test]
    fn clean_edited_save_folds() {
        assert_eq!(
            decide(state(ExternalWrite::Edited, false, false)),
            ImportDecision::Fold
        );
    }

    // TEST-484b (positive): an unchanged save always folds (base known).
    #[test]
    fn unchanged_save_folds_even_with_intervening_export() {
        // An unchanged save hashes to a retained export, so its base is known
        // regardless of intervening exports.
        assert_eq!(
            decide(state(ExternalWrite::Unchanged, false, true)),
            ImportDecision::Fold
        );
    }

    // TEST-484c (neg-input): an edited save with an intervening export has an
    // ambiguous base and is staged (F59) — never folded to a deletion.
    #[test]
    fn edited_save_with_intervening_export_stages() {
        assert!(matches!(
            decide(state(ExternalWrite::Edited, false, true)),
            ImportDecision::Stage(_)
        ));
    }

    // TEST-484d (neg-input): any unmaterialised daemon op stages — the on-disk
    // file is not the canonical base.
    #[test]
    fn unmaterialised_daemon_op_stages_regardless() {
        for write in [ExternalWrite::Unchanged, ExternalWrite::Edited] {
            for interv in [false, true] {
                assert!(
                    matches!(decide(state(write, true, interv)), ImportDecision::Stage(_)),
                    "unmaterialised daemon op must stage ({write:?}, interv={interv})"
                );
            }
        }
    }

    // TEST-484e: the orchestrator folds into the note on a fold decision and
    // preserves the staged path (bytes never discarded) on a stage decision.
    #[test]
    fn orchestrator_folds_or_stages() {
        let mut note = NoteDoc::new();
        note.set_content("original").unwrap();

        // Fold path: body is updated.
        let out = import_external(
            &mut note,
            "edited body",
            state(ExternalWrite::Edited, false, false),
            "/conflict/x.md",
        )
        .unwrap();
        assert_eq!(out, ImportOutcome::Folded);
        assert_eq!(note.materialise(), "edited body\n");

        // Stage path: note untouched, staged path returned.
        let out = import_external(
            &mut note,
            "stale edit",
            state(ExternalWrite::Edited, true, false),
            "/conflict/x.md",
        )
        .unwrap();
        assert_eq!(out, ImportOutcome::Staged(PathBuf::from("/conflict/x.md")));
        assert_eq!(
            note.materialise(),
            "edited body\n",
            "staged write must not fold"
        );
    }
}
