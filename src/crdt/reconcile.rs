//! Version-vector anti-entropy reconciliation (SPEC-047 REQ-486, IMPL-047 T6).
//!
//! The convergence core of the sync engine, built on the [`NoteDoc`] delta
//! primitives (`oplog_vv` / `export_updates_since` / `import_updates`). Two
//! peers exchange exactly the [[Loro]] ops each lacks and **loop** until their
//! op-log [[Version Vector]]s are equal — a single exchange is not assumed to
//! be completion (F65: an edit landing mid-reconcile, or the mixed
//! root-mismatch case, needs another round).
//!
//! This is the transport-agnostic, in-memory core. The [[Merkle DAG]] coarse
//! "what changed" filter (ADR-478 — descend the DAG to localise differing
//! docs before exchanging ops) is a later T6 slice layered *on top* of this:
//! the version-vector exchange already guarantees convergence; the Merkle
//! layer only makes "are we synced?" cheap for a large, mostly-quiescent
//! vault. Also independent of the group-key/roster layer (M2) — a reconcile
//! runs only after the roster gate has admitted the peer.

use super::loro_store::{DocId, LoroStore, NoteDoc};
use anyhow::{Context, Result};

/// Safety bound: reconcile between two quiescent peers converges in one round;
/// concurrent mid-reconcile edits add rounds. A run exceeding this is a bug,
/// not slow convergence.
const MAX_ROUNDS: usize = 128;

/// Reconcile two note documents in place by exchanging the deltas each lacks,
/// looping until their op-log version vectors are equal (REQ-486, F65).
/// Returns the number of exchange rounds performed (0 if already converged).
pub fn reconcile_pair(a: &mut NoteDoc, b: &mut NoteDoc) -> Result<usize> {
    let mut rounds = 0;
    loop {
        let a_vv = a.oplog_vv();
        let b_vv = b.oplog_vv();
        if a_vv == b_vv {
            return Ok(rounds);
        }
        // Each side exports exactly what the other is missing, then imports.
        let a_to_b = a.export_updates_since(&b_vv)?;
        let b_to_a = b.export_updates_since(&a_vv)?;
        b.import_updates(&a_to_b)?;
        a.import_updates(&b_to_a)?;
        rounds += 1;
        anyhow::ensure!(
            rounds <= MAX_ROUNDS,
            "reconcile did not converge within {MAX_ROUNDS} rounds"
        );
    }
}

/// Reconcile a set of note documents between two local stores (the vault-level
/// step). Each doc is loaded from both stores, reconciled, and the merged
/// result persisted back to both — so after this call both stores hold
/// identical state for every named doc. Missing docs are treated as empty
/// (a peer that lacks a note learns it via the exchange).
///
/// Returns the total rounds across all docs (a coarse work metric).
pub fn reconcile_stores(a: &LoroStore, b: &LoroStore, docs: &[DocId]) -> Result<usize> {
    let mut total = 0;
    for id in docs {
        let mut da = a
            .load_or_create(id)
            .with_context(|| format!("load {id} from store A"))?;
        let mut db = b
            .load_or_create(id)
            .with_context(|| format!("load {id} from store B"))?;
        total += reconcile_pair(&mut da, &mut db)?;
        a.persist(id, &da)?;
        b.persist(id, &db)?;
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    // REQ-486: already-converged peers exchange nothing (0 rounds).
    #[test]
    fn converged_peers_do_no_work() {
        let mut a = NoteDoc::new();
        a.set_content("same").unwrap();
        let mut b = NoteDoc::from_snapshot(&a.snapshot().unwrap()).unwrap();
        assert_eq!(reconcile_pair(&mut a, &mut b).unwrap(), 0);
    }

    // REQ-486/474: concurrent divergent edits converge, in a bounded number of
    // rounds, to byte-identical state.
    #[test]
    fn divergent_peers_converge_in_one_round() {
        let mut a = NoteDoc::new();
        a.set_content("base").unwrap();
        let mut b = NoteDoc::from_snapshot(&a.snapshot().unwrap()).unwrap();

        a.insert(4, " A").unwrap();
        b.insert(0, "B ").unwrap();

        let rounds = reconcile_pair(&mut a, &mut b).unwrap();
        assert_eq!(a.materialise(), b.materialise(), "must converge");
        assert_eq!(a.oplog_vv(), b.oplog_vv(), "version vectors equal");
        assert_eq!(rounds, 1, "two quiescent peers converge in one exchange");
    }

    // A peer that has never seen a note learns it whole via reconcile.
    #[test]
    fn peer_learns_unknown_note() {
        let mut have = NoteDoc::new();
        have.set_content("# Note\n\ncontent").unwrap();
        let mut lacks = NoteDoc::new();

        reconcile_pair(&mut have, &mut lacks).unwrap();
        assert_eq!(lacks.materialise(), "# Note\n\ncontent");
    }

    // Vault-level: reconcile several docs across two on-disk stores; both
    // stores end identical for every doc, including ones only one side had.
    #[test]
    fn store_level_reconcile_converges_all_docs() {
        let ta = tempfile::tempdir().unwrap();
        let tb = tempfile::tempdir().unwrap();
        let sa = LoroStore::open(ta.path());
        let sb = LoroStore::open(tb.path());

        let shared = DocId::parse("shared").unwrap();
        let only_a = DocId::parse("only-a").unwrap();
        let only_b = DocId::parse("only-b").unwrap();

        // A has `shared` (v1) and `only-a`; B has `shared` (concurrent edit)
        // and `only-b`.
        let mut a_shared = NoteDoc::new();
        a_shared.set_content("shared-base").unwrap();
        sa.persist(&shared, &a_shared).unwrap();
        let mut b_shared = NoteDoc::from_snapshot(&a_shared.snapshot().unwrap()).unwrap();
        b_shared.insert(0, "B:").unwrap();
        sb.persist(&shared, &b_shared).unwrap();

        let mut a_only = NoteDoc::new();
        a_only.set_content("A private").unwrap();
        sa.persist(&only_a, &a_only).unwrap();
        let mut b_only = NoteDoc::new();
        b_only.set_content("B private").unwrap();
        sb.persist(&only_b, &b_only).unwrap();

        let docs = [shared.clone(), only_a.clone(), only_b.clone()];
        reconcile_stores(&sa, &sb, &docs).unwrap();

        // Both stores now agree on every doc.
        for id in &docs {
            let da = sa.load_or_create(id).unwrap();
            let db = sb.load_or_create(id).unwrap();
            assert_eq!(da.materialise(), db.materialise(), "doc {id} diverged");
        }
        // The shared doc converged the concurrent edit; the private docs
        // propagated whole.
        assert_eq!(sa.load_or_create(&only_b).unwrap().materialise(), "B private");
        assert_eq!(sb.load_or_create(&only_a).unwrap().materialise(), "A private");
    }
}
