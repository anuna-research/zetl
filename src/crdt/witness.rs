//! The Merkle vault-root convergence witness (SPEC-047 REQ-485, IMPL-047 T6).
//!
//! A cheap, independent "are two peers in the same state?" check layered over
//! [[Loro]] (ADR-478): compute a single [[Merkle Vault Root]] over a store's
//! materialised notes. Two peers that have converged derive the **identical**
//! root; a mismatch between peers that [[Loro]] reports as converged is an
//! integrity alarm, not something to silently tolerate. This reuses the
//! existing [[SPEC-006]] `merkle::compute_vault_root` primitive rather than a
//! new hasher (Simplicity Ladder rung 4).
//
// SIMPLIFY: each note contributes a whole-content BLAKE3 hash, not its
// SPEC-006 block-leaf file-root. Ceiling: dovetailing byte-for-byte with the
// jj `vault_root` snapshot and enabling the Merkle DAG *descent* to localise
// differing blocks within a note; upgrade path: chunk each note's materialised
// Markdown into MerkleLeaf blocks via the existing parser and use
// `compute_file_root` (trace: SPEC-047 REQ-485 / REQ-486 / SPEC-006).

use super::loro_store::LoroStore;
use super::manifest::Manifest;
use crate::merkle::compute_vault_root;
use crate::types::ContentHash;
use anyhow::Result;
use std::path::{Path, PathBuf};

/// The vault-root witness over a store's notes as named by `manifest`
/// (REQ-485). Deterministic: identical converged state → identical root.
pub fn vault_witness(manifest: &Manifest, store: &LoroStore) -> Result<ContentHash> {
    let resolved = manifest.resolve();
    let mut pairs: Vec<(PathBuf, ContentHash)> = Vec::with_capacity(resolved.len());
    for (id, path) in resolved {
        let content = store.load_or_create(&id)?.materialise();
        let hash = *blake3::hash(content.as_bytes()).as_bytes();
        pairs.push((PathBuf::from(path), hash));
    }
    let refs: Vec<(&Path, ContentHash)> = pairs.iter().map(|(p, h)| (p.as_path(), *h)).collect();
    Ok(compute_vault_root(&refs))
}

/// Whether two peers' vaults witness the same state (REQ-485). A cheap
/// heartbeat: equal witnesses mean the materialised state agrees. (Equal
/// witnesses do *not* by themselves prove equal [[Loro]] op state — roots can
/// lag unmaterialised ops, F33 — so reconciliation still checks version
/// vectors; this is the coarse filter, not the completion condition.)
pub fn witnesses_agree(
    a_manifest: &Manifest,
    a_store: &LoroStore,
    b_manifest: &Manifest,
    b_store: &LoroStore,
) -> Result<bool> {
    Ok(vault_witness(a_manifest, a_store)? == vault_witness(b_manifest, b_store)?)
}

#[cfg(test)]
mod tests {
    use super::super::loro_store::{DocId, NoteDoc};
    use super::super::reconcile::reconcile_vault;
    use super::*;

    fn seed(store: &LoroStore, manifest: &Manifest, id: &str, path: &str, body: &str) {
        // Widen the short test tag to the fixed 32-lowercase-hex DocId grammar.
        let did = DocId::parse(&format!("{:0>32}", id.replace('n', "d"))).unwrap();
        manifest.create(&did, path).unwrap();
        let mut note = NoteDoc::new();
        note.set_content(body).unwrap();
        store.persist(&did, &note).unwrap();
    }

    // REQ-485: an empty vault has the canonical empty root; identical content
    // yields identical witnesses regardless of which store computed them.
    #[test]
    fn identical_vaults_witness_equally() {
        let ta = tempfile::tempdir().unwrap();
        let tb = tempfile::tempdir().unwrap();
        let (sa, sb) = (LoroStore::open(ta.path()), LoroStore::open(tb.path()));
        let (ma, mb) = (Manifest::new(), Manifest::new());

        assert_eq!(
            vault_witness(&ma, &sa).unwrap(),
            vault_witness(&mb, &sb).unwrap()
        );

        seed(&sa, &ma, "n1", "a.md", "hello");
        seed(&sb, &mb, "n1", "a.md", "hello");
        assert!(witnesses_agree(&ma, &sa, &mb, &sb).unwrap());
    }

    // REQ-485: divergent content witnesses differently.
    #[test]
    fn divergent_vaults_witness_differently() {
        let ta = tempfile::tempdir().unwrap();
        let tb = tempfile::tempdir().unwrap();
        let (sa, sb) = (LoroStore::open(ta.path()), LoroStore::open(tb.path()));
        let (ma, mb) = (Manifest::new(), Manifest::new());
        seed(&sa, &ma, "n1", "a.md", "one");
        seed(&sb, &mb, "n1", "a.md", "two");
        assert!(!witnesses_agree(&ma, &sa, &mb, &sb).unwrap());
    }

    // REQ-485/486: after reconciling two divergent vaults, their witnesses
    // agree — the witness confirms the reconcile converged.
    #[test]
    fn witness_agrees_after_reconcile() {
        let ta = tempfile::tempdir().unwrap();
        let tb = tempfile::tempdir().unwrap();
        let (sa, sb) = (LoroStore::open(ta.path()), LoroStore::open(tb.path()));
        let (ma, mb) = (Manifest::new(), Manifest::new());
        seed(&sa, &ma, "n1", "a.md", "A note");
        seed(&sb, &mb, "n2", "b.md", "B note");
        assert!(!witnesses_agree(&ma, &sa, &mb, &sb).unwrap());

        reconcile_vault(&sa, &ma, &sb, &mb).unwrap();
        assert!(
            witnesses_agree(&ma, &sa, &mb, &sb).unwrap(),
            "witnesses must agree after a successful reconcile"
        );
    }
}
