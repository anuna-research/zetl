//! Recorded per-document export state (SPEC-047 REQ-484, ADR-471, F59).
//!
//! The guarded-import decision ([`super::guarded_import::decide`]) must be a
//! function of *recorded state*, never guessed from content. This module is
//! that record: for every document the daemon has materialised, the content
//! hash and path it last wrote (or acknowledged from an external fold), plus
//! whether any export since the last external event changed the file — the
//! fact that makes an *edited* save's base ambiguous.
//!
//! Persisted as `.zetl/loro/export-state.json` alongside the snapshots, with
//! the same tmp + rename + directory-fsync durability discipline.

use super::loro_store::DocId;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

const STATE_FILE: &str = "export-state.json";

/// What the daemon knows about one document's materialised file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocExportRecord {
    /// blake3 (hex) of the content this daemon last wrote to the exported
    /// path, or acknowledged there via an external fold. An on-disk file
    /// hashing to this is an *unchanged* save — its base is known.
    pub exported_hash: String,
    /// Vault-relative path last exported — tracked so a rename or delete can
    /// remove the obsolete file during the next materialisation.
    pub exported_path: String,
    /// True when an export since the last external event on this document
    /// wrote *different* content than the generation before it. An edited
    /// save arriving after that has an ambiguous base (the editor may have
    /// opened the older generation) and must stage (F59).
    pub changed_since_external: bool,
}

/// The vault's export-state table, keyed by DocId.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ExportState {
    docs: BTreeMap<String, DocExportRecord>,
}

fn hash_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

impl ExportState {
    fn path(vault_root: &Path) -> std::path::PathBuf {
        vault_root.join(".zetl").join("loro").join(STATE_FILE)
    }

    /// Load the vault's export state; a missing file is an empty table (every
    /// decision then falls back to its conservative — staging — default).
    pub fn load(vault_root: &Path) -> Result<ExportState> {
        let path = Self::path(vault_root);
        match std::fs::read(&path) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ExportState::default()),
            Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
        }
    }

    /// Persist atomically (tmp + rename + directory fsync).
    pub fn save(&self, vault_root: &Path) -> Result<()> {
        use std::io::Write as _;
        let path = Self::path(vault_root);
        let dir = path.parent().expect("state file has a parent");
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        let tmp = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(self).context("serialise export state")?;
        let mut f =
            std::fs::File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
        f.write_all(&bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, &path)?;
        super::fsync_dir(dir)?;
        Ok(())
    }

    pub fn get(&self, id: &DocId) -> Option<&DocExportRecord> {
        self.docs.get(id.as_str())
    }

    /// All records, keyed by DocId string (for obsolete-path cleanup).
    pub fn records(&self) -> &BTreeMap<String, DocExportRecord> {
        &self.docs
    }

    /// Record that the daemon exported `content` to `path` for `id`. Sets the
    /// ambiguity flag when this export changed the file relative to the
    /// previous recorded generation.
    pub fn record_export(&mut self, id: &DocId, path: &str, content: &str) {
        let hash = hash_hex(content.as_bytes());
        let changed = match self.docs.get(id.as_str()) {
            Some(prev) => prev.changed_since_external || prev.exported_hash != hash,
            // No recorded generation at all → conservative: an edited save
            // against this document has an unknowable base.
            None => true,
        };
        self.docs.insert(
            id.as_str().to_string(),
            DocExportRecord {
                exported_hash: hash,
                exported_path: path.to_string(),
                changed_since_external: changed,
            },
        );
    }

    /// Record an external event on `id` (a fold, or observing the file equal
    /// to canonical): the editorial world is now based on `canonical`, so the
    /// ambiguity flag clears.
    pub fn record_external(&mut self, id: &DocId, path: &str, canonical: &str) {
        self.docs.insert(
            id.as_str().to_string(),
            DocExportRecord {
                exported_hash: hash_hex(canonical.as_bytes()),
                exported_path: path.to_string(),
                changed_since_external: false,
            },
        );
    }

    /// Drop the record for a document (its manifest entry is gone and its
    /// exported file has been handled).
    pub fn remove(&mut self, id_str: &str) {
        self.docs.remove(id_str);
    }

    /// Whether `bytes` hash to the recorded export for `id` (an unchanged
    /// save — the base is known).
    pub fn matches_export(&self, id: &DocId, bytes: &[u8]) -> bool {
        self.get(id)
            .is_some_and(|r| r.exported_hash == hash_hex(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_tracks_change_generations() {
        let tmp = tempfile::tempdir().unwrap();
        let id = DocId::parse(&"d".repeat(32)).unwrap();
        let mut st = ExportState::load(tmp.path()).unwrap();
        assert!(st.get(&id).is_none());

        // External bootstrap: base known, no ambiguity.
        st.record_external(&id, "a.md", "one\n");
        assert!(!st.get(&id).unwrap().changed_since_external);
        assert!(st.matches_export(&id, b"one\n"));

        // Re-exporting the same content does not create ambiguity…
        st.record_export(&id, "a.md", "one\n");
        assert!(!st.get(&id).unwrap().changed_since_external);
        // …but exporting *changed* content does, until the next external event.
        st.record_export(&id, "a.md", "two\n");
        assert!(st.get(&id).unwrap().changed_since_external);
        st.record_export(&id, "a.md", "two\n");
        assert!(
            st.get(&id).unwrap().changed_since_external,
            "flag is sticky"
        );
        st.record_external(&id, "a.md", "three\n");
        assert!(
            !st.get(&id).unwrap().changed_since_external,
            "external event clears"
        );

        st.save(tmp.path()).unwrap();
        let reloaded = ExportState::load(tmp.path()).unwrap();
        assert_eq!(reloaded.get(&id), st.get(&id));
    }
}
