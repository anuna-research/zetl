//! Materialised-export bridge between the canonical [[Loro]] store and the
//! vault's Markdown files (SPEC-047 ADR-470, REQ-472/473, IMPL-047 T3).
//!
//! [[Loro]] is canonical; the on-disk Markdown is a deterministic *export*.
//! [`export_vault`] writes every note the manifest names to its vault-relative
//! path. Paths from the (replicated, hence untrusted) manifest are recognised
//! before use — a path that escapes the vault root is rejected, never written
//! (LangSec / REQ-483 spirit): a malicious peer must not be able to make a
//! reconcile write outside the vault.

use super::loro_store::LoroStore;
use super::manifest::Manifest;
use anyhow::{Context, Result};
use std::path::{Component, Path, PathBuf};

/// Resolve a vault-relative manifest path against the vault root, rejecting any
/// path that is absolute or escapes the root via `..` (fail closed).
pub fn safe_join(vault_root: &Path, rel: &str) -> Result<PathBuf> {
    let rel_path = Path::new(rel);
    anyhow::ensure!(rel_path.is_relative(), "manifest path is not relative: {rel:?}");
    let mut out = vault_root.to_path_buf();
    for comp in rel_path.components() {
        match comp {
            Component::Normal(seg) => out.push(seg),
            // Reject anything that could traverse: `..`, `/`, drive prefixes.
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                anyhow::bail!("manifest path escapes the vault: {rel:?}")
            }
            Component::CurDir => {}
        }
    }
    Ok(out)
}

/// Export every note named by `manifest` to its path under `vault_root`,
/// materialising each from the canonical store (REQ-472/473). Atomic per file
/// (tmp + rename). Returns the number of files written.
pub fn export_vault(vault_root: &Path, manifest: &Manifest, store: &LoroStore) -> Result<usize> {
    let mut written = 0;
    for (id, rel) in manifest.resolve() {
        let path = safe_join(vault_root, &rel)?;
        let content = store
            .load_or_create(&id)
            .with_context(|| format!("load note {id}"))?
            .materialise();
        write_atomic(&path, content.as_bytes())?;
        written += 1;
    }
    Ok(written)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write as _;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create dir {}", parent.display()))?;
    }
    let tmp = path.with_extension("md.tmp");
    let mut f = std::fs::File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
    f.write_all(bytes)?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::loro_store::{DocId, NoteDoc};
    use super::*;

    // REQ-472/473: export writes each note to its manifest path with the
    // materialised content.
    #[test]
    fn export_writes_notes_to_manifest_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();
        let store = LoroStore::open(vault);
        let manifest = Manifest::new();

        let a = DocId::parse("a1").unwrap();
        let b = DocId::parse("b2").unwrap();
        manifest.create(&a, "notes/alpha.md").unwrap();
        manifest.create(&b, "beta.md").unwrap();
        let mut na = NoteDoc::new();
        na.set_content("# Alpha\n\nbody").unwrap();
        store.persist(&a, &na).unwrap();
        let mut nb = NoteDoc::new();
        nb.set_content("beta content").unwrap();
        store.persist(&b, &nb).unwrap();

        let written = export_vault(vault, &manifest, &store).unwrap();
        assert_eq!(written, 2);
        assert_eq!(
            std::fs::read_to_string(vault.join("notes/alpha.md")).unwrap(),
            "# Alpha\n\nbody"
        );
        assert_eq!(
            std::fs::read_to_string(vault.join("beta.md")).unwrap(),
            "beta content"
        );
    }

    // LangSec: a manifest path that escapes the vault is rejected, never
    // written — a malicious peer cannot make a reconcile write outside.
    #[test]
    fn traversal_paths_are_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(safe_join(tmp.path(), "../evil.md").is_err());
        assert!(safe_join(tmp.path(), "a/../../evil.md").is_err());
        assert!(safe_join(tmp.path(), "/etc/passwd").is_err());
        assert!(safe_join(tmp.path(), "ok/nested.md").is_ok());
    }

    // An export refuses to write a note whose manifest path escapes the vault
    // (fails closed rather than writing any file).
    #[test]
    fn export_fails_closed_on_escaping_path() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();
        let store = LoroStore::open(vault);
        let manifest = Manifest::new();
        let evil = DocId::parse("evil").unwrap();
        manifest.create(&evil, "../escape.md").unwrap();
        let mut n = NoteDoc::new();
        n.set_content("x").unwrap();
        store.persist(&evil, &n).unwrap();

        assert!(export_vault(vault, &manifest, &store).is_err());
        assert!(!tmp.path().parent().unwrap().join("escape.md").exists());
    }
}
