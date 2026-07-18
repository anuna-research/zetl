//! The [[Loro]] canonical store (SPEC-047 REQ-472/473, CON-471, IMPL-047 T3).
//!
//! Per Q3 ([[SPEC-047]] ADR-478), a vault is **one `LoroDoc` per note**, keyed
//! by a stable [`DocId`] and persisted as a snapshot (full history + state)
//! under `<vault>/.zetl/loro/<DocId>.loro`. This module is introduced
//! *additively*: the diamond-types editing engine and the WebSocket layer are
//! untouched in this slice. Later IMPL-047 T3 slices layer block-structured
//! materialisation (SPEC-006 AST) on top and migrate the editing layer.
//!
//! Purity: [`NoteDoc::materialise`] is deterministic and I/O-free (REQ-473 C3).
//! The effectful shell ([`LoroStore`]) performs snapshot persistence.
//!
//! A [`NoteDoc`] *is* a rich-text [`LoroCrdtDocument`] (the editing engine):
//! store and editor share one Loro model (SPEC-047 §9). `materialise` returns
//! canonical Markdown via `to_markdown`.

use super::loro_backend::LoroCrdtDocument;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Subdirectory of `.zetl/` holding per-note Loro snapshots.
const LORO_SUBDIR: &str = "loro";
/// Snapshot file extension.
const SNAPSHOT_EXT: &str = "loro";

/// A stable identifier for one note document (Q3 / Q12). The exact scheme is
/// `[Provisional — DESIGN-047 task adr-namespace]`; until it is fixed, a DocId
/// is an opaque, filesystem-safe slug. Recognised (LangSec) before it is ever
/// used to build a path — a DocId that could escape the store directory is
/// rejected, never sanitised.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DocId(String);

impl DocId {
    /// Recognise a DocId: non-empty, `[A-Za-z0-9._-]+`, and neither `.` nor
    /// `..` (which would be path components, not ids). Rejects anything that
    /// could traverse out of the store directory (fail closed).
    pub fn parse(s: &str) -> Result<DocId> {
        if s.is_empty() || s == "." || s == ".." {
            anyhow::bail!("invalid DocId {s:?}: empty or a path component");
        }
        if !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        {
            anyhow::bail!("invalid DocId {s:?}: only [A-Za-z0-9._-] allowed");
        }
        Ok(DocId(s.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Mint a fresh, stable DocId: 32 lowercase hex chars over 128 random bits
    /// (Q12 / adr-namespace). Opaque and decoupled from the path, so a rename
    /// or move preserves the document's identity and history
    /// ([[REQ-504]]). Collision probability is negligible (128-bit).
    pub fn mint() -> DocId {
        use rand_core::RngCore as _;
        let mut bytes = [0u8; 16];
        rand_core::OsRng.fill_bytes(&mut bytes);
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        DocId(hex)
    }
}

impl std::fmt::Display for DocId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One note's canonical CRDT document. Unified with the editing engine
/// (SPEC-047 ADR-470/§9): a `NoteDoc` *is* a rich-text
/// [`LoroCrdtDocument`], so the note the store persists and syncs is the same
/// representation the editor edits — one model, not two. Content in and out is
/// canonical Markdown ([`materialise`](Self::materialise) = `to_markdown`).
pub struct NoteDoc {
    inner: LoroCrdtDocument,
}

impl Default for NoteDoc {
    fn default() -> Self {
        Self::new()
    }
}

impl NoteDoc {
    /// A fresh, empty note document.
    pub fn new() -> NoteDoc {
        NoteDoc {
            inner: LoroCrdtDocument::new().expect("empty loro document is infallible"),
        }
    }

    /// Bind this replica's edit-attribution actor id to a stable device identity
    /// ([`crate::p2p::identity::DeviceIdentity::loro_peer`]). Set before editing.
    pub fn set_actor(&self, peer: u64) -> Result<()> {
        self.inner.set_actor(peer)
    }

    /// This replica's current Loro actor id (who its edits attribute to).
    pub fn actor(&self) -> u64 {
        self.inner.actor()
    }

    /// Load a note document from a persisted snapshot (REQ-472 C4: restart
    /// reloads canonical state *with* causal history — a snapshot carries the
    /// full oplog, so subsequent concurrent edits still merge).
    pub fn from_snapshot(bytes: &[u8]) -> Result<NoteDoc> {
        Ok(NoteDoc {
            inner: LoroCrdtDocument::load(bytes)?,
        })
    }

    /// Replace the note's content with canonical Markdown (parsed into the
    /// rich-text model; exact bytes preserved — no editor newline normalising).
    pub fn set_content(&mut self, markdown: &str) -> Result<()> {
        self.inner.set_markdown(markdown)
    }

    /// Insert `text` at unicode index `pos`.
    pub fn insert(&mut self, pos: usize, text: &str) -> Result<()> {
        self.inner.splice_text(pos, 0, text)
    }

    /// Deterministic materialisation to canonical Markdown (REQ-473 C3:
    /// referentially transparent — identical Loro state yields identical bytes).
    pub fn materialise(&self) -> String {
        self.inner.to_markdown().unwrap_or_default()
    }

    /// The note's text length in unicode scalars (distinct from the canonical
    /// materialised length, which may add a trailing newline). Callers indexing
    /// into the text (e.g. [`insert`](Self::insert)) clamp against this.
    pub fn len(&self) -> usize {
        self.inner.text().map(|t| t.chars().count()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Full snapshot (history + state) for persistence.
    pub fn snapshot(&self) -> Result<Vec<u8>> {
        Ok(self.inner.save())
    }

    /// Op-log version vector — the per-document sync cursor (REQ-486).
    pub fn oplog_vv(&self) -> loro::VersionVector {
        self.inner.oplog_vv()
    }

    /// Export the ops this document has that a peer at `remote` lacks (REQ-486).
    pub fn export_updates_since(&self, remote: &loro::VersionVector) -> Result<Vec<u8>> {
        self.inner.export_updates_since(remote)
    }

    /// Merge a peer's exported updates into this document (conflict-free —
    /// REQ-474). `&self` — Loro is interior-mutable — composing with the
    /// [`crate::crdt::reconcile::Syncable`] abstraction.
    pub fn import_updates(&self, bytes: &[u8]) -> Result<()> {
        self.inner.import_updates(bytes)
    }
}

/// The vault's keyed collection of per-note Loro documents (effectful shell).
/// Persists snapshots under `<vault>/.zetl/loro/`.
pub struct LoroStore {
    dir: PathBuf,
}

impl LoroStore {
    /// Open (creating the directory lazily on first persist) the store for a
    /// vault root.
    pub fn open(vault_root: &Path) -> LoroStore {
        LoroStore {
            dir: vault_root.join(".zetl").join(LORO_SUBDIR),
        }
    }

    fn snapshot_path(&self, id: &DocId) -> PathBuf {
        self.dir.join(format!("{}.{SNAPSHOT_EXT}", id.as_str()))
    }

    /// Load a note document from disk, or return a fresh one if none exists.
    pub fn load_or_create(&self, id: &DocId) -> Result<NoteDoc> {
        let path = self.snapshot_path(id);
        match std::fs::read(&path) {
            Ok(bytes) => NoteDoc::from_snapshot(&bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(NoteDoc::new()),
            Err(e) => Err(e).with_context(|| format!("read snapshot {}", path.display())),
        }
    }

    /// Persist a note's snapshot atomically (tmp + rename), 0600.
    pub fn persist(&self, id: &DocId, note: &NoteDoc) -> Result<()> {
        use std::io::Write as _;
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("create loro dir {}", self.dir.display()))?;
        let bytes = note.snapshot()?;
        let path = self.snapshot_path(id);
        let tmp = path.with_extension("loro.tmp");
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp).with_context(|| format!("open {}", tmp.display()))?;
        f.write_all(&bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Whether a note is persisted.
    pub fn exists(&self, id: &DocId) -> bool {
        self.snapshot_path(id).exists()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // TEST (REQ-472/473): snapshot persistence roundtrips through disk with
    // content intact.
    #[test]
    fn persist_and_reload_roundtrips() {
        let tmp = tempfile::tempdir().unwrap();
        let store = LoroStore::open(tmp.path());
        let id = DocId::parse("note-alpha").unwrap();

        let mut note = store.load_or_create(&id).unwrap();
        assert_eq!(note.materialise(), "");
        note.set_content("# Title\n\nbody").unwrap();
        store.persist(&id, &note).unwrap();
        assert!(store.exists(&id));

        // Reopen from disk (fresh store) — canonical state reloads.
        let store2 = LoroStore::open(tmp.path());
        let reloaded = store2.load_or_create(&id).unwrap();
        assert_eq!(reloaded.materialise(), "# Title\n\nbody\n");
    }

    // TEST-473: materialise is referentially transparent — identical state,
    // identical bytes, regardless of how the state was reached.
    #[test]
    fn materialise_is_deterministic_across_edit_orders() {
        let mut a = NoteDoc::new();
        a.insert(0, "world").unwrap();
        a.insert(0, "hello ").unwrap();

        let mut b = NoteDoc::new();
        b.insert(0, "hello ").unwrap();
        b.insert(6, "world").unwrap();

        assert_eq!(a.materialise(), "hello world\n");
        assert_eq!(a.materialise(), b.materialise());
        // Referential transparency: repeated calls agree.
        assert_eq!(a.materialise(), a.materialise());
    }

    // TEST-474: two peers exchanging updates converge conflict-free.
    #[test]
    fn peers_converge_via_update_exchange() {
        let mut a = NoteDoc::new();
        let mut b = NoteDoc::new();
        a.set_content("shared").unwrap();

        // b learns a's ops (b had none), then both edit concurrently.
        let empty = loro::VersionVector::default();
        b.import_updates(&a.export_updates_since(&empty).unwrap()).unwrap();
        assert_eq!(b.materialise(), "shared\n");

        a.insert(6, " by A").unwrap();
        b.insert(0, "X ").unwrap();

        // Exchange the deltas each lacks.
        let a_vv = a.oplog_vv();
        let b_vv = b.oplog_vv();
        let a_to_b = a.export_updates_since(&b_vv).unwrap();
        let b_to_a = b.export_updates_since(&a_vv).unwrap();
        a.import_updates(&b_to_a).unwrap();
        b.import_updates(&a_to_b).unwrap();

        // Conflict-free convergence to identical bytes (REQ-474 + REQ-485).
        assert_eq!(a.materialise(), b.materialise());
    }

    // TEST (REQ-483 spirit / LangSec): a DocId that could traverse the store
    // directory is rejected, never sanitised.
    #[test]
    fn docid_rejects_traversal_and_bad_chars() {
        assert!(DocId::parse("").is_err());
        assert!(DocId::parse(".").is_err());
        assert!(DocId::parse("..").is_err());
        assert!(DocId::parse("../secret").is_err());
        assert!(DocId::parse("a/b").is_err());
        assert!(DocId::parse("a b").is_err());
        assert!(DocId::parse("note-1_v.2").is_ok());
    }

    // Property tests for the pure CRDT core (SPEC-047 §10: property-based
    // testing for the Loro core — roundtrip + convergence invariants).
    use proptest::prelude::*;

    /// Apply a sequence of inserts, clamping each position into range so every
    /// generated edit is valid.
    fn apply_inserts(note: &mut NoteDoc, edits: &[(usize, String)]) {
        for (p, t) in edits {
            let len = note.len();
            let pos = if len == 0 { 0 } else { p % (len + 1) };
            note.insert(pos, t).unwrap();
        }
    }

    proptest! {
        // REQ-472/473: persisting then reloading any content preserves it,
        // and materialise is a referentially-transparent function of state.
        #[test]
        fn prop_snapshot_roundtrip_preserves_content(s in "[a-zA-Z0-9 \n#*_-]{0,60}") {
            let tmp = tempfile::tempdir().unwrap();
            let store = LoroStore::open(tmp.path());
            let id = DocId::parse("p").unwrap();
            let mut note = store.load_or_create(&id).unwrap();
            note.set_content(&s).unwrap();
            store.persist(&id, &note).unwrap();

            let reloaded = LoroStore::open(tmp.path()).load_or_create(&id).unwrap();
            prop_assert_eq!(reloaded.materialise(), note.materialise());
        }

        // REQ-474/485: two peers that start from a shared base, edit
        // concurrently, and exchange the deltas each lacks converge to
        // byte-identical state — conflict-free, order-independent.
        #[test]
        fn prop_two_peers_converge(
            base in "[a-z ]{0,20}",
            a_edits in prop::collection::vec((0usize..200, "[a-z]{1,4}"), 0..6),
            b_edits in prop::collection::vec((0usize..200, "[A-Z]{1,4}"), 0..6),
        ) {
            let mut a = NoteDoc::new();
            let mut b = NoteDoc::new();
            a.set_content(&base).unwrap();

            // b learns the shared base from a.
            let empty = loro::VersionVector::default();
            b.import_updates(&a.export_updates_since(&empty).unwrap()).unwrap();

            // Concurrent divergent edits.
            apply_inserts(&mut a, &a_edits);
            apply_inserts(&mut b, &b_edits);

            // Each exports exactly the ops the other lacks; both import.
            let a_vv = a.oplog_vv();
            let b_vv = b.oplog_vv();
            let a_to_b = a.export_updates_since(&b_vv).unwrap();
            let b_to_a = b.export_updates_since(&a_vv).unwrap();
            a.import_updates(&b_to_a).unwrap();
            b.import_updates(&a_to_b).unwrap();

            prop_assert_eq!(a.materialise(), b.materialise());
        }
    }
}
