//! Materialised-export bridge between the canonical [[Loro]] store and the
//! vault's Markdown files (SPEC-047 ADR-470, REQ-472/473, IMPL-047 T3).
//!
//! [[Loro]] is canonical; the on-disk Markdown is a deterministic *export*.
//! [`export_vault`] writes every note the manifest names to its vault-relative
//! path. Paths from the (replicated, hence untrusted) manifest are recognised
//! before use — a path that escapes the vault root is rejected, never written
//! (LangSec / REQ-483 spirit): a malicious peer must not be able to make a
//! reconcile write outside the vault.

use super::loro_store::{DocId, LoroStore, NoteDoc};
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

/// Bootstrap the canonical store from a vault's existing Markdown files
/// (REQ-470: the daemon becomes the single owner of vault state). One
/// [`NoteDoc`] per `.md` file holding its content, a minted [`DocId`], and a
/// manifest entry mapping DocId → vault-relative path. The `.zetl/` runtime
/// directory is skipped. Intended as a one-time bootstrap when the store is
/// empty; the persisted manifest is authoritative on subsequent starts.
/// Returns the number of notes imported.
pub fn import_vault(vault_root: &Path, store: &LoroStore, manifest: &Manifest) -> Result<usize> {
    let mut count = 0;
    for rel in markdown_files(vault_root)? {
        let content = std::fs::read_to_string(vault_root.join(&rel))
            .with_context(|| format!("read {}", rel.display()))?;
        let id = DocId::mint();
        let mut note = NoteDoc::new();
        note.set_content(&content)?;
        store.persist(&id, &note)?;
        manifest.create(&id, &path_to_rel_str(&rel))?;
        count += 1;
    }
    manifest.save(vault_root)?;
    Ok(count)
}

/// Vault-relative `.md` paths (forward-slash), skipping `.zetl/` and any
/// dot-directory. Deterministic order (sorted).
fn markdown_files(vault_root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    collect_md(vault_root, vault_root, &mut out)?;
    out.sort();
    Ok(out)
}

fn collect_md(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).with_context(|| format!("read dir {}", dir.display())),
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // Skip hidden entries (including .zetl and .git).
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let ft = entry.file_type()?;
        if ft.is_dir() {
            collect_md(root, &path, out)?;
        } else if ft.is_file() && path.extension().is_some_and(|e| e == "md") {
            if let Ok(rel) = path.strip_prefix(root) {
                out.push(rel.to_path_buf());
            }
        }
    }
    Ok(())
}

fn path_to_rel_str(rel: &Path) -> String {
    rel.to_string_lossy().replace('\\', "/")
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
        na.set_content("# Alpha\n\nbody\n").unwrap();
        store.persist(&a, &na).unwrap();
        let mut nb = NoteDoc::new();
        nb.set_content("beta content").unwrap();
        store.persist(&b, &nb).unwrap();

        let written = export_vault(vault, &manifest, &store).unwrap();
        assert_eq!(written, 2);
        assert_eq!(
            std::fs::read_to_string(vault.join("notes/alpha.md")).unwrap(),
            "# Alpha\n\nbody\n"
        );
        assert_eq!(
            std::fs::read_to_string(vault.join("beta.md")).unwrap(),
            "beta content\n"
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

    // REQ-470: import a vault's Markdown into the store, then export it back —
    // a full round-trip through the canonical Loro store preserving content
    // and paths (including nested files).
    #[test]
    fn import_then_export_round_trips_the_vault() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();
        std::fs::create_dir_all(vault.join("sub")).unwrap();
        std::fs::write(vault.join("top.md"), "# Top\n\nbody\n").unwrap();
        std::fs::write(vault.join("sub/nested.md"), "nested content").unwrap();
        // Non-markdown and hidden files are ignored.
        std::fs::write(vault.join("image.png"), b"binary").unwrap();
        std::fs::create_dir_all(vault.join(".zetl")).unwrap();
        std::fs::write(vault.join(".zetl/ignored.md"), "should be skipped").unwrap();

        let store = LoroStore::open(vault);
        let manifest = Manifest::new();
        let n = import_vault(vault, &store, &manifest).unwrap();
        assert_eq!(n, 2, "two markdown files imported, .zetl and .png skipped");

        // The manifest maps both notes to their vault-relative paths.
        let paths: Vec<String> = manifest.resolve().into_values().collect();
        assert!(paths.contains(&"top.md".to_string()));
        assert!(paths.contains(&"sub/nested.md".to_string()));

        // Persisted manifest reloads identically.
        manifest.save(vault).unwrap();
        let reloaded = Manifest::load(vault).unwrap();
        assert_eq!(reloaded.resolve(), manifest.resolve());

        // Export into a fresh directory reproduces the files byte-for-byte.
        let out = tempfile::tempdir().unwrap();
        export_vault(out.path(), &manifest, &store).unwrap();
        assert_eq!(
            std::fs::read_to_string(out.path().join("top.md")).unwrap(),
            "# Top\n\nbody\n"
        );
        assert_eq!(
            std::fs::read_to_string(out.path().join("sub/nested.md")).unwrap(),
            "nested content\n"
        );
    }
}
