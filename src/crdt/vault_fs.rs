//! Materialised-export bridge between the canonical [[Loro]] store and the
//! vault's Markdown files (SPEC-047 ADR-470, REQ-472/473, IMPL-047 T3).
//!
//! [[Loro]] is canonical; the on-disk Markdown is a deterministic *export*.
//! [`export_vault`] writes every note the manifest names to its vault-relative
//! path. Paths from the (replicated, hence untrusted) manifest are recognised
//! before use — a path that escapes the vault root is rejected, never written
//! (LangSec / REQ-483 spirit): a malicious peer must not be able to make a
//! reconcile write outside the vault.

use super::export_state::ExportState;
use super::loro_store::{DocId, LoroStore};
use super::manifest::Manifest;
use anyhow::{Context, Result};
use std::path::{Component, Path, PathBuf};

/// Resolve a vault-relative manifest path against the vault root, rejecting
/// (fail closed) any path that is absolute, escapes the root via `..`, or
/// enters a dot-directory/file. The dot rule reserves the vault's internal
/// directories — a replicated path like `.zetl/loro/manifest.loro` or
/// `.git/config` must never be materialisable — and mirrors the import-side
/// skip rule, so import and export agree on the namespace.
pub fn safe_join(vault_root: &Path, rel: &str) -> Result<PathBuf> {
    let rel_path = Path::new(rel);
    anyhow::ensure!(
        rel_path.is_relative(),
        "manifest path is not relative: {rel:?}"
    );
    let mut out = vault_root.to_path_buf();
    for comp in rel_path.components() {
        match comp {
            Component::Normal(seg) => {
                if seg.to_string_lossy().starts_with('.') {
                    anyhow::bail!("manifest path enters a reserved dot-path: {rel:?}");
                }
                out.push(seg);
            }
            // Reject anything that could traverse: `..`, `/`, drive prefixes.
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                anyhow::bail!("manifest path escapes the vault: {rel:?}")
            }
            Component::CurDir => {}
        }
    }
    Ok(out)
}

/// Verify that `path`'s parent, with every symlink resolved, still lies under
/// the (resolved) vault root — a lexically-safe path like `link/note.md` must
/// not write outside the vault when `link` is a symlink elsewhere. Called
/// after the parent exists (post `create_dir_all`), before any write.
fn verify_parent_within(vault_root: &Path, path: &Path) -> Result<()> {
    let parent = path.parent().unwrap_or(vault_root);
    let canon_parent = parent
        .canonicalize()
        .with_context(|| format!("resolve {}", parent.display()))?;
    let canon_root = vault_root
        .canonicalize()
        .with_context(|| format!("resolve vault root {}", vault_root.display()))?;
    anyhow::ensure!(
        canon_parent.starts_with(&canon_root),
        "write target {} resolves outside the vault (symlinked parent?)",
        path.display()
    );
    Ok(())
}

/// Export every note named by `manifest` to its path under `vault_root`,
/// materialising each from the canonical store (REQ-472/473). Atomic per file
/// (tmp + rename). Records each write in the export state (feeding the
/// guarded-import decision), and removes files at paths a rename or delete
/// made obsolete — but only when their on-disk content still matches our last
/// export (an externally modified orphan is never destroyed). Returns the
/// number of files written.
pub fn export_vault(vault_root: &Path, manifest: &Manifest, store: &LoroStore) -> Result<usize> {
    let mut state = ExportState::load(vault_root)?;
    let resolved = manifest.resolve();
    // Snapshot the recorded (id → old path, old hash) *before* this export
    // pass overwrites the records, so renames still see the path they vacate.
    let prior: Vec<(String, String, String)> = state
        .records()
        .iter()
        .map(|(id, r)| (id.clone(), r.exported_path.clone(), r.exported_hash.clone()))
        .collect();
    let mut written = 0;
    for (id, rel) in &resolved {
        let path = safe_join(vault_root, rel)?;
        let content = store
            .load_or_create(id)
            .with_context(|| format!("load note {id}"))?
            .materialise();
        write_checked(vault_root, &path, content.as_bytes())?;
        state.record_export(id, rel, &content);
        written += 1;
    }
    remove_obsolete_exports(vault_root, &resolved, &prior, &mut state)?;
    state.save(vault_root)?;
    Ok(written)
}

/// Remove files left at previously-exported paths that no current manifest
/// entry names (renamed or deleted notes), when — and only when — the file is
/// byte-identical to what we last exported there. A file an external editor
/// changed since is left in place: never destroy the only copy of unseen data.
fn remove_obsolete_exports(
    vault_root: &Path,
    resolved: &std::collections::BTreeMap<DocId, String>,
    prior: &[(String, String, String)],
    state: &mut ExportState,
) -> Result<()> {
    let current_paths: std::collections::BTreeSet<&str> =
        resolved.values().map(String::as_str).collect();
    for (id_str, old_rel, old_hash) in prior {
        if current_paths.contains(old_rel.as_str()) {
            continue; // path still (or newly) owned by some note
        }
        let Ok(old_path) = safe_join(vault_root, old_rel) else {
            continue;
        };
        match std::fs::read(&old_path) {
            Ok(bytes) if blake3::hash(&bytes).to_hex().to_string() == *old_hash => {
                std::fs::remove_file(&old_path)
                    .with_context(|| format!("remove obsolete export {}", old_path.display()))?;
            }
            // Modified since our export, or already gone → leave it be.
            _ => {}
        }
        // Ids no longer in the manifest lose their record entirely; renamed
        // ids already got a fresh record from the export pass above.
        if DocId::parse(id_str)
            .ok()
            .is_none_or(|id| !resolved.contains_key(&id))
        {
            state.remove(id_str);
        }
    }
    Ok(())
}

/// Bootstrap the canonical store from a vault's existing Markdown files
/// (REQ-470: the daemon becomes the single owner of vault state). One
/// [`NoteDoc`] per `.md` file holding its content, a minted [`DocId`], and a
/// manifest entry mapping DocId → vault-relative path. The `.zetl/` runtime
/// directory is skipped. Intended as a one-time bootstrap when the store is
/// empty; the persisted manifest is authoritative on subsequent starts.
/// Returns the number of notes imported.
pub fn import_vault(vault_root: &Path, store: &LoroStore, manifest: &Manifest) -> Result<usize> {
    let mut state = ExportState::load(vault_root)?;
    let mut count = 0;
    for rel in markdown_files(vault_root)? {
        let content = std::fs::read_to_string(vault_root.join(&rel))
            .with_context(|| format!("read {}", rel.display()))?;
        let id = DocId::mint();
        let mut note = store.load_or_create(&id)?;
        note.set_content(&content)?;
        store.persist(&id, &note)?;
        let rel_str = path_to_rel_str(&rel);
        manifest.create(&id, &rel_str)?;
        // The import IS an external event: the editorial world and the store
        // agree on this base, so a follow-up save folds cleanly.
        state.record_external(&id, &rel_str, &note.materialise());
        count += 1;
    }
    manifest.save(vault_root)?;
    state.save(vault_root)?;
    Ok(count)
}

/// Re-import external Markdown edits back into the canonical store (REQ-484,
/// ADR-471). For each note the manifest names, compares the on-disk file to
/// the store's canonical materialisation; a difference is an external write,
/// routed through the guarded-import decision ([`super::guarded_import`])
/// **from recorded state** ([`ExportState`], F59): whether the bytes match
/// the retained export generation, whether the store holds ops not yet
/// materialised (the editor's base is stale), and whether an export changed
/// the file since the last external event (an edited save's base is
/// ambiguous). Stale or ambiguous saves stage; only saves with an
/// unambiguous base fold.
///
/// A file that is not valid UTF-8 (an external tool wrote binary) is staged
/// to the conflict area as raw bytes rather than aborting the pass — a later
/// materialisation may overwrite the file, and the staged copy must survive.
/// Returns (folded, staged) counts. A missing file is skipped (a delete is a
/// manifest op, F38 — not handled here).
pub fn reimport_vault(
    vault_root: &Path,
    store: &LoroStore,
    manifest: &Manifest,
) -> Result<(usize, usize)> {
    use super::guarded_import::{decide, ExternalWrite, ImportDecision, ImportState};
    let mut export_state = ExportState::load(vault_root)?;
    let (mut folded, mut staged) = (0, 0);
    for (id, rel) in manifest.resolve() {
        let path = safe_join(vault_root, &rel)?;
        let raw = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
        };
        let Ok(disk) = String::from_utf8(raw.clone()) else {
            // Not Markdown we can ingest — preserve the bytes, keep going.
            stage_conflict(vault_root, &rel, &raw)?;
            staged += 1;
            continue;
        };
        let note = store.load_or_create(&id)?;
        let canonical = note.materialise();
        if disk == canonical {
            // Converged: disk is the canonical export. Record it so the next
            // edited save has a known base even if the state file was lost.
            export_state.record_external(&id, &rel, &canonical);
            continue;
        }
        let rec = export_state.get(&id);
        let state = ImportState {
            write: if export_state.matches_export(&id, disk.as_bytes()) {
                ExternalWrite::Unchanged
            } else {
                ExternalWrite::Edited
            },
            // The store's canonical content differs from what we last wrote to
            // disk → ops exist that the editor's file never showed. No record
            // at all is the conservative same (base unknowable).
            unmaterialised_daemon_op: rec.is_none_or(|r| {
                r.exported_hash != blake3::hash(canonical.as_bytes()).to_hex().to_string()
            }),
            intervening_export: rec.is_none_or(|r| r.changed_since_external),
        };
        match decide(state) {
            ImportDecision::Fold => {
                let mut n = store.load_or_create(&id)?;
                n.set_content(&disk)?;
                store.persist(&id, &n)?;
                export_state.record_external(&id, &rel, &n.materialise());
                folded += 1;
            }
            ImportDecision::Stage(_) => {
                stage_conflict(vault_root, &rel, disk.as_bytes())?;
                staged += 1;
            }
        }
    }
    export_state.save(vault_root)?;
    Ok((folded, staged))
}

/// Preserve an external write in the conflict area (CON-471 C5: bytes are
/// never discarded), mirroring the note's vault-relative path.
fn stage_conflict(vault_root: &Path, rel: &str, bytes: &[u8]) -> Result<()> {
    let conflict = vault_root.join(".zetl").join("conflicts").join(rel);
    write_checked(vault_root, &conflict, bytes)
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

/// Atomic write (tmp + rename + directory fsync) with the symlink-escape
/// check: after the parent exists, its *resolved* location must still be
/// under the vault root, or nothing is written.
fn write_checked(vault_root: &Path, path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write as _;
    let parent = path.parent().unwrap_or(vault_root).to_path_buf();
    std::fs::create_dir_all(&parent).with_context(|| format!("create dir {}", parent.display()))?;
    verify_parent_within(vault_root, path)?;
    let tmp = path.with_extension("md.tmp");
    let mut f = std::fs::File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
    f.write_all(bytes)?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)?;
    super::fsync_dir(&parent)?;
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

        let a = DocId::parse(&"a1".repeat(16)).unwrap();
        let b = DocId::parse(&"b2".repeat(16)).unwrap();
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

    // LangSec: replicated paths must not reach the vault's internal state —
    // `.zetl/loro/manifest.loro`, `.git/config`, or anything dot-prefixed
    // would let a peer's manifest entry overwrite canonical metadata.
    #[test]
    fn reserved_dot_paths_are_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(safe_join(tmp.path(), ".zetl/loro/manifest.loro").is_err());
        assert!(safe_join(tmp.path(), ".git/config").is_err());
        assert!(safe_join(tmp.path(), "a/.hidden/b.md").is_err());
        assert!(safe_join(tmp.path(), ".dotfile.md").is_err());
        assert!(safe_join(tmp.path(), "dotted.name/ok.md").is_ok());
    }

    // LangSec: a lexically-safe path whose parent is a symlink out of the
    // vault must not be written through — the resolved target is checked.
    #[cfg(unix)]
    #[test]
    fn symlinked_parent_cannot_escape_the_vault() {
        let vault_dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let vault = vault_dir.path();
        std::os::unix::fs::symlink(outside.path(), vault.join("link")).unwrap();

        let store = LoroStore::open(vault);
        let manifest = Manifest::new();
        let id = DocId::parse(&"d".repeat(32)).unwrap();
        manifest.create(&id, "link/note.md").unwrap();
        let mut n = NoteDoc::new();
        n.set_content("payload").unwrap();
        store.persist(&id, &n).unwrap();

        assert!(export_vault(vault, &manifest, &store).is_err());
        assert!(
            !outside.path().join("note.md").exists(),
            "nothing written outside"
        );
    }

    // A rename or delete removes the file at the vacated path on the next
    // materialisation — but an externally modified orphan is preserved.
    #[test]
    fn materialisation_removes_obsolete_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();
        std::fs::write(vault.join("a.md"), "content\n").unwrap();
        let store = LoroStore::open(vault);
        let manifest = Manifest::new();
        import_vault(vault, &store, &manifest).unwrap();
        export_vault(vault, &manifest, &store).unwrap();
        let id = manifest.resolve().into_iter().next().unwrap().0;

        // Rename: the old path disappears, the new one exists.
        manifest.rename(&id, "b.md").unwrap();
        export_vault(vault, &manifest, &store).unwrap();
        assert!(!vault.join("a.md").exists(), "renamed-away file removed");
        assert_eq!(
            std::fs::read_to_string(vault.join("b.md")).unwrap(),
            "content\n"
        );

        // Externally modified orphan: rename again, but the vacated file has
        // unseen edits — it must be left in place.
        std::fs::write(vault.join("b.md"), "externally edited\n").unwrap();
        manifest.rename(&id, "c.md").unwrap();
        export_vault(vault, &manifest, &store).unwrap();
        assert!(vault.join("b.md").exists(), "modified orphan preserved");
        assert!(vault.join("c.md").exists());

        // Delete: the manifest entry goes; the (unchanged) export goes too.
        manifest.delete(&id).unwrap();
        export_vault(vault, &manifest, &store).unwrap();
        assert!(!vault.join("c.md").exists(), "deleted note's file removed");
    }

    // A deleted note must NOT be re-imported on a daemon restart: bootstrap
    // keys off the persisted manifest snapshot, not the entry count.
    #[test]
    fn empty_persisted_manifest_is_not_a_first_run() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();
        std::fs::write(vault.join("a.md"), "content\n").unwrap();
        let store = LoroStore::open(vault);
        let manifest = Manifest::new();
        import_vault(vault, &store, &manifest).unwrap();
        let id = manifest.resolve().into_iter().next().unwrap().0;
        manifest.delete(&id).unwrap();
        manifest.save(vault).unwrap();

        // The persisted manifest exists and is empty — that is a valid state.
        assert!(Manifest::snapshot_exists(vault));
        assert!(Manifest::load(vault).unwrap().resolve().is_empty());
    }

    // A non-UTF-8 external write is staged (bytes preserved), not fatal to
    // the whole reimport pass.
    #[test]
    fn non_utf8_external_write_is_staged() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();
        std::fs::write(vault.join("note.md"), "text\n").unwrap();
        let store = LoroStore::open(vault);
        let manifest = Manifest::new();
        import_vault(vault, &store, &manifest).unwrap();
        export_vault(vault, &manifest, &store).unwrap();

        let binary = [0xff, 0xfe, 0x00, 0x42];
        std::fs::write(vault.join("note.md"), binary).unwrap();
        let (folded, staged) = reimport_vault(vault, &store, &manifest).unwrap();
        assert_eq!((folded, staged), (0, 1), "binary write stages");
        assert_eq!(
            std::fs::read(vault.join(".zetl/conflicts/note.md")).unwrap(),
            binary,
            "the exact bytes are preserved"
        );
    }

    // F59: an external save whose base is stale — the store holds an
    // unmaterialised op — must stage, not fold (folding would convert the
    // daemon's changes into deletions).
    #[test]
    fn stale_based_save_stages_instead_of_folding() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();
        std::fs::write(vault.join("note.md"), "E0\n").unwrap();
        let store = LoroStore::open(vault);
        let manifest = Manifest::new();
        import_vault(vault, &store, &manifest).unwrap();
        export_vault(vault, &manifest, &store).unwrap();
        let id = manifest.resolve().into_iter().next().unwrap().0;

        // A daemon-side op (e.g. a merged sync delta) not yet materialised.
        let mut n = store.load_or_create(&id).unwrap();
        n.set_content("E1 from sync\n").unwrap();
        store.persist(&id, &n).unwrap();

        // The editor, still holding E0, saves an edit.
        std::fs::write(vault.join("note.md"), "E0 plus my edit\n").unwrap();
        let (folded, staged) = reimport_vault(vault, &store, &manifest).unwrap();
        assert_eq!((folded, staged), (0, 1), "stale base must stage");
        // The daemon's op survives in canonical; the editor's bytes survive
        // in the conflict area (CON-471 C5: neither side discarded).
        assert_eq!(
            store.load_or_create(&id).unwrap().materialise(),
            "E1 from sync\n"
        );
        assert_eq!(
            std::fs::read_to_string(vault.join(".zetl/conflicts/note.md")).unwrap(),
            "E0 plus my edit\n"
        );
    }

    // F59: after an export *changed* the file (E0 → E1), an edited save may
    // still derive from E0 — its base is ambiguous, so it stages even though
    // everything is materialised.
    #[test]
    fn edited_save_after_intervening_export_stages() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();
        std::fs::write(vault.join("note.md"), "E0\n").unwrap();
        let store = LoroStore::open(vault);
        let manifest = Manifest::new();
        import_vault(vault, &store, &manifest).unwrap();
        export_vault(vault, &manifest, &store).unwrap();
        let id = manifest.resolve().into_iter().next().unwrap().0;

        // Daemon op + materialisation: the file on disk is now E1.
        let mut n = store.load_or_create(&id).unwrap();
        n.set_content("E1 from sync\n").unwrap();
        store.persist(&id, &n).unwrap();
        export_vault(vault, &manifest, &store).unwrap();

        // The editor's buffer predates E1; its save silently overwrites E1.
        std::fs::write(vault.join("note.md"), "E0 plus my edit\n").unwrap();
        let (folded, staged) = reimport_vault(vault, &store, &manifest).unwrap();
        assert_eq!((folded, staged), (0, 1), "ambiguous base must stage");
        assert_eq!(
            store.load_or_create(&id).unwrap().materialise(),
            "E1 from sync\n"
        );

        // Once the user reconciles (file matches canonical again), the next
        // genuine edit folds normally — the ambiguity flag clears.
        export_vault(vault, &manifest, &store).unwrap();
        reimport_vault(vault, &store, &manifest).unwrap();
        std::fs::write(vault.join("note.md"), "E1 from sync, edited\n").unwrap();
        let (folded, staged) = reimport_vault(vault, &store, &manifest).unwrap();
        assert_eq!((folded, staged), (1, 0), "clean edit folds again");
    }

    // An export refuses to write a note whose manifest path escapes the vault
    // (fails closed rather than writing any file).
    #[test]
    fn export_fails_closed_on_escaping_path() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();
        let store = LoroStore::open(vault);
        let manifest = Manifest::new();
        let evil = DocId::parse(&"e".repeat(32)).unwrap();
        manifest.create(&evil, "../escape.md").unwrap();
        let mut n = NoteDoc::new();
        n.set_content("x").unwrap();
        store.persist(&evil, &n).unwrap();

        assert!(export_vault(vault, &manifest, &store).is_err());
        assert!(!tmp.path().parent().unwrap().join("escape.md").exists());
    }

    // REQ-484: an external edit to a note's Markdown file folds back into the
    // canonical store; an unchanged file is a no-op.
    #[test]
    fn reimport_folds_external_edits() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();
        std::fs::write(vault.join("note.md"), "original\n").unwrap();
        let store = LoroStore::open(vault);
        let manifest = Manifest::new();
        import_vault(vault, &store, &manifest).unwrap();
        export_vault(vault, &manifest, &store).unwrap();

        // No external change → nothing folded.
        let (folded, staged) = reimport_vault(vault, &store, &manifest).unwrap();
        assert_eq!((folded, staged), (0, 0), "unchanged file is a no-op");

        // An external editor changes the file.
        std::fs::write(vault.join("note.md"), "edited by hand\n").unwrap();
        let (folded, staged) = reimport_vault(vault, &store, &manifest).unwrap();
        assert_eq!((folded, staged), (1, 0), "external edit folds");

        // The canonical store now reflects the edit.
        let id = manifest
            .resolve()
            .into_iter()
            .find(|(_, p)| p == "note.md")
            .unwrap()
            .0;
        assert_eq!(
            store.load_or_create(&id).unwrap().materialise(),
            "edited by hand\n"
        );
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
