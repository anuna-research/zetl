//! The replicated vault namespace manifest (SPEC-047 REQ-504, IMPL-047 T5).
//!
//! Per Q3, a vault is a keyed collection of per-note [[Loro]] documents; the
//! manifest is the one vault-level document that maps each stable [[DocId]] to
//! its vault-relative path. It is itself a [[Loro]] map CRDT, so create /
//! rename / delete converge conflict-free (REQ-504) — a rename is a value
//! change on a *stable* DocId key, so it preserves the document's identity and
//! history rather than being a delete+create.
//!
//! Q12 / adr-namespace decisions realised here:
//! - **DocId** is a minted, opaque, path-independent id ([`DocId::mint`]) —
//!   renames and moves keep identity.
//! - **Manifest shape** is a [[Loro]] map `DocId → path`.
//! - **Collision rule** ([`Manifest::resolve`]): when two DocIds map to the
//!   same folded path, the lexicographically-smaller DocId keeps the path and
//!   the others get a deterministic disambiguated path — never a silent
//!   overwrite.
//
// SIMPLIFY: path folding is ASCII/`to_lowercase` only, not full Unicode NFC +
// locale-independent case-fold. Ceiling: case-insensitive filesystems with
// non-ASCII note names; upgrade path: fold under NFC + Unicode default
// case-fold (trace: SPEC-047 REQ-473 / REQ-504, Q12).

use super::loro_store::DocId;
use anyhow::{Context, Result};
use loro::{ExportMode, LoroDoc, LoroValue};
use std::collections::BTreeMap;
use std::path::Path;

const PATHS_MAP: &str = "paths";

/// The vault namespace manifest — a Loro map `DocId → vault-relative path`.
pub struct Manifest {
    doc: LoroDoc,
}

impl Default for Manifest {
    fn default() -> Self {
        Self::new()
    }
}

impl Manifest {
    pub fn new() -> Manifest {
        Manifest {
            doc: LoroDoc::new(),
        }
    }

    pub fn from_snapshot(bytes: &[u8]) -> Result<Manifest> {
        let doc = LoroDoc::new();
        doc.import(bytes).context("import manifest snapshot")?;
        Ok(Manifest { doc })
    }

    fn put(&self, id: &DocId, path: &str) -> Result<()> {
        self.doc
            .get_map(PATHS_MAP)
            .insert(id.as_str(), path)
            .context("manifest map insert")?;
        self.doc.commit();
        Ok(())
    }

    /// Register a new note at `path` under a fresh identity.
    pub fn create(&self, id: &DocId, path: &str) -> Result<()> {
        self.put(id, path)
    }

    /// Move a note to `new_path`, preserving its DocId (identity + history).
    pub fn rename(&self, id: &DocId, new_path: &str) -> Result<()> {
        self.put(id, new_path)
    }

    /// Tombstone a note (remove its manifest entry). The per-note Loro document
    /// is retained; only the namespace mapping is dropped.
    pub fn delete(&self, id: &DocId) -> Result<()> {
        self.doc
            .get_map(PATHS_MAP)
            .delete(id.as_str())
            .context("manifest map delete")?;
        self.doc.commit();
        Ok(())
    }

    /// The raw `DocId → path` entries, exactly as stored (before collision
    /// disambiguation).
    pub fn raw_entries(&self) -> BTreeMap<DocId, String> {
        let mut out = BTreeMap::new();
        if let LoroValue::Map(map) = self.doc.get_map(PATHS_MAP).get_value() {
            for (k, v) in map.iter() {
                if let (Ok(id), LoroValue::String(s)) = (DocId::parse(k), v) {
                    out.insert(id, s.to_string());
                }
            }
        }
        out
    }

    /// Path for a DocId, as stored.
    pub fn raw_path(&self, id: &DocId) -> Option<String> {
        self.raw_entries().get(id).cloned()
    }

    /// The resolved, collision-free `DocId → path` mapping every converged peer
    /// derives identically (REQ-504). When several DocIds collide on a folded
    /// path, the smallest DocId keeps it; the rest get generated names that are
    /// reserved against the **entire** resolved namespace — a generated name
    /// colliding with another group's stored path (or another generated name)
    /// is extended deterministically, never silently overwritten.
    pub fn resolve(&self) -> BTreeMap<DocId, String> {
        use std::collections::BTreeSet;
        let raw = self.raw_entries();
        let mut by_fold: BTreeMap<String, Vec<(DocId, String)>> = BTreeMap::new();
        for (id, path) in raw {
            by_fold
                .entry(fold_path(&path))
                .or_default()
                .push((id, path));
        }
        // Every group's rank-0 entry keeps its stored path; those folded names
        // are exactly the group keys, hence mutually distinct. Reserving them
        // all up front means a generated name can never shadow a kept path in
        // any group — not just its own.
        let mut taken: BTreeSet<String> = by_fold.keys().cloned().collect();
        let mut out = BTreeMap::new();
        for group in by_fold.values() {
            let mut group = group.clone();
            group.sort_by(|a, b| a.0.cmp(&b.0));
            for (rank, (id, path)) in group.into_iter().enumerate() {
                let resolved = if rank == 0 {
                    path
                } else {
                    let candidate = disambiguation_candidates(&path, &id)
                        .find(|c| !taken.contains(&fold_path(c)))
                        .expect("candidate sequence is unbounded");
                    taken.insert(fold_path(&candidate));
                    candidate
                };
                out.insert(id, resolved);
            }
        }
        out
    }

    pub fn snapshot(&self) -> Result<Vec<u8>> {
        self.doc
            .export(ExportMode::snapshot())
            .context("export manifest snapshot")
    }

    pub fn oplog_vv(&self) -> loro::VersionVector {
        self.doc.oplog_vv()
    }

    pub fn export_updates_since(&self, remote: &loro::VersionVector) -> Result<Vec<u8>> {
        self.doc
            .export(ExportMode::updates(remote))
            .context("export manifest updates")
    }

    pub fn import_updates(&self, bytes: &[u8]) -> Result<()> {
        self.doc.import(bytes).context("import manifest updates")?;
        Ok(())
    }

    /// Persist the manifest snapshot atomically under `.zetl/loro/manifest.loro`
    /// (0600), alongside the per-note snapshots.
    pub fn save(&self, vault_root: &Path) -> Result<()> {
        self.save_to_dir(&vault_root.join(".zetl").join("loro"))
    }

    /// Persist the manifest snapshot into an explicit snapshot directory
    /// (tmp + rename + directory fsync — same durability discipline as note
    /// snapshots). [`crate::crdt::loro_store::LoroStore::persist_manifest`]
    /// routes here so store and manifest agree on the directory.
    pub(crate) fn save_to_dir(&self, dir: &Path) -> Result<()> {
        use std::io::Write as _;
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        let path = dir.join("manifest.loro");
        let bytes = self
            .doc
            .export(ExportMode::snapshot())
            .context("export manifest")?;
        let tmp = path.with_extension("loro.tmp");
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            opts.mode(0o600);
        }
        let mut f = opts
            .open(&tmp)
            .with_context(|| format!("open {}", tmp.display()))?;
        f.write_all(&bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, &path)?;
        super::fsync_dir(dir)?;
        Ok(())
    }

    /// Whether a persisted manifest snapshot exists for this vault. This — not
    /// the entry count — is the bootstrap signal: a persisted manifest with
    /// zero entries is a valid state (every note deleted), and re-importing
    /// stray Markdown over it would resurrect deleted content under fresh
    /// DocIds.
    pub fn snapshot_exists(vault_root: &Path) -> bool {
        vault_root
            .join(".zetl")
            .join("loro")
            .join("manifest.loro")
            .exists()
    }

    /// Load a vault's manifest, or a fresh empty one if none is persisted.
    pub fn load(vault_root: &Path) -> Result<Manifest> {
        let path = vault_root.join(".zetl").join("loro").join("manifest.loro");
        match std::fs::read(&path) {
            Ok(bytes) => Manifest::from_snapshot(&bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Manifest::new()),
            Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
        }
    }
}

/// Fold a path for collision comparison (case-insensitive filesystems).
fn fold_path(path: &str) -> String {
    path.to_lowercase()
}

/// Deterministic, unbounded sequence of disambiguated paths for a colliding
/// DocId: progressively longer DocId prefixes before the extension (8, 16, 24,
/// then the full 32), then the full id with a numeric suffix. Stable across
/// peers, so the first *globally free* candidate is the same everywhere.
fn disambiguation_candidates<'a>(
    path: &'a str,
    id: &'a DocId,
) -> impl Iterator<Item = String> + 'a {
    let full = id.as_str();
    (1u32..).map(move |k| {
        let tag = match k {
            1..=4 => full[..(k as usize * 8).min(full.len())].to_string(),
            n => format!("{full}-{}", n - 4),
        };
        match path.rfind('.') {
            Some(dot) if dot > 0 => format!("{} ({}){}", &path[..dot], tag, &path[dot..]),
            _ => format!("{path} ({tag})"),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // REQ-504: create/rename/delete over stable DocIds; rename preserves
    // identity (same DocId, new path), delete tombstones.
    #[test]
    fn create_rename_delete() {
        let m = Manifest::new();
        let id = DocId::parse(&"a".repeat(32)).unwrap();
        m.create(&id, "notes/a.md").unwrap();
        assert_eq!(m.raw_path(&id).as_deref(), Some("notes/a.md"));

        m.rename(&id, "archive/a.md").unwrap();
        assert_eq!(m.raw_path(&id).as_deref(), Some("archive/a.md"));
        // Identity preserved: still exactly one entry, same DocId.
        assert_eq!(m.raw_entries().len(), 1);

        m.delete(&id).unwrap();
        assert!(m.raw_path(&id).is_none());
    }

    // Minted DocIds are distinct, opaque, and parse-valid.
    #[test]
    fn minted_docids_are_distinct_and_valid() {
        let a = DocId::mint();
        let b = DocId::mint();
        assert_ne!(a, b);
        assert!(DocId::parse(a.as_str()).is_ok());
        assert_eq!(a.as_str().len(), 32);
    }

    // TEST-504a: concurrent create/rename on disconnected peers → identical
    // resolved mapping after merge (conflict-free convergence).
    #[test]
    fn concurrent_edits_converge() {
        let a = Manifest::new();
        let b = Manifest::new();
        let id1 = DocId::parse(&"1".repeat(32)).unwrap();
        let id2 = DocId::parse(&"2".repeat(32)).unwrap();

        // A creates id1, B creates id2 — concurrently, distinct paths.
        a.create(&id1, "one.md").unwrap();
        b.create(&id2, "two.md").unwrap();

        // Exchange updates both ways.
        let a_vv = a.oplog_vv();
        let b_vv = b.oplog_vv();
        let a_to_b = a.export_updates_since(&b_vv).unwrap();
        let b_to_a = b.export_updates_since(&a_vv).unwrap();
        b.import_updates(&a_to_b).unwrap();
        a.import_updates(&b_to_a).unwrap();

        assert_eq!(a.resolve(), b.resolve());
        assert_eq!(a.resolve().len(), 2);
    }

    // TEST-504b: two DocIds colliding on the same (case-folded) path resolve
    // deterministically — the smaller DocId keeps the path, the larger is
    // disambiguated; never a silent overwrite.
    #[test]
    fn colliding_paths_disambiguate_deterministically() {
        let m = Manifest::new();
        let lo = DocId::parse(&"aaaa1111".repeat(4)).unwrap();
        let hi = DocId::parse(&"bbbb2222".repeat(4)).unwrap();
        // Same path modulo case → collision.
        m.create(&lo, "Note.md").unwrap();
        m.create(&hi, "note.md").unwrap();

        let resolved = m.resolve();
        assert_eq!(resolved.len(), 2, "both entries survive");
        // Smaller DocId keeps its path; the other is disambiguated, distinct.
        assert_eq!(resolved[&lo], "Note.md");
        assert_ne!(resolved[&hi], "note.md");
        assert_ne!(resolved[&lo].to_lowercase(), resolved[&hi].to_lowercase());
        assert!(resolved[&hi].contains("bbbb2222"));
    }

    // Regression: a *generated* collision name must be reserved against the
    // whole namespace. Here a third note (its own fold group) already occupies
    // the name the second colliding note would generate — resolution must not
    // silently assign two DocIds the same path.
    #[test]
    fn generated_names_are_reserved_globally() {
        let m = Manifest::new();
        let lo = DocId::parse(&"aaaa1111".repeat(4)).unwrap();
        let hi = DocId::parse(&"bbbb2222".repeat(4)).unwrap();
        let squatter = DocId::parse(&"cccc3333".repeat(4)).unwrap();
        m.create(&lo, "note.md").unwrap();
        m.create(&hi, "Note.md").unwrap();
        // Occupies exactly hi's first-choice generated name "note (bbbb2222).md".
        m.create(&squatter, "note (bbbb2222).md").unwrap();

        let resolved = m.resolve();
        assert_eq!(resolved.len(), 3, "all entries survive");
        let mut folded: Vec<String> = resolved.values().map(|p| p.to_lowercase()).collect();
        folded.sort();
        folded.dedup();
        assert_eq!(
            folded.len(),
            3,
            "no two DocIds share a resolved path: {resolved:?}"
        );
        // The squatter keeps its stored path; hi is pushed to a longer tag.
        assert_eq!(resolved[&squatter], "note (bbbb2222).md");
        assert!(
            resolved[&hi].contains("bbbb2222bbbb2222"),
            "extended tag: {}",
            resolved[&hi]
        );
    }
}
