//! Smoke test for the `gen-vault` synthetic-vault generator
//! (PERF-BUILD-2026-05-12 task `bench-fixture`).
//!
//! Confirms two acceptance properties:
//!   1. The generator produces the requested number of `.md` files.
//!   2. The same `--seed` + same `--pages` + same `--avg-links`
//!      yields **byte-identical** output across runs (determinism).

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use assert_cmd::prelude::*;
use std::process::Command;
use tempfile::TempDir;

/// Walk `root` and collect (relative-path, file-bytes) for every `.md`
/// file. Sorted by path so two collected snapshots compare deterministically.
fn collect_md_files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let ft = entry.file_type().unwrap();
        if ft.is_dir() {
            walk(root, &path, out);
        } else if ft.is_file() && path.extension().and_then(|e| e.to_str()) == Some("md") {
            let rel = path.strip_prefix(root).unwrap().to_string_lossy().to_string();
            let bytes = fs::read(&path).unwrap();
            out.insert(rel, bytes);
        }
    }
}

#[test]
fn gen_vault_is_deterministic_for_same_seed() {
    let tmp_a = TempDir::new().unwrap();
    let tmp_b = TempDir::new().unwrap();

    // Run #1
    Command::cargo_bin("gen-vault")
        .unwrap()
        .args([
            "--pages",
            "50",
            "--avg-links",
            "8",
            "--seed",
            "42",
            "--out",
        ])
        .arg(tmp_a.path())
        .arg("--force")
        .assert()
        .success();

    // Run #2 — separate tempdir, same seed/flags.
    Command::cargo_bin("gen-vault")
        .unwrap()
        .args([
            "--pages",
            "50",
            "--avg-links",
            "8",
            "--seed",
            "42",
            "--out",
        ])
        .arg(tmp_b.path())
        .arg("--force")
        .assert()
        .success();

    let snap_a = collect_md_files(tmp_a.path());
    let snap_b = collect_md_files(tmp_b.path());

    assert_eq!(
        snap_a.len(),
        50,
        "expected 50 .md files, got {}",
        snap_a.len()
    );
    assert_eq!(
        snap_b.len(),
        50,
        "expected 50 .md files, got {}",
        snap_b.len()
    );
    assert_eq!(
        snap_a.keys().collect::<Vec<_>>(),
        snap_b.keys().collect::<Vec<_>>(),
        "directory layouts diverged between runs with the same seed"
    );
    for (k, va) in &snap_a {
        let vb = snap_b.get(k).unwrap();
        assert_eq!(
            va,
            vb,
            "content for {} differs across runs with the same seed",
            k
        );
    }
}

#[test]
fn gen_vault_refuses_non_empty_dir_without_force() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("squatter.txt"), b"hi").unwrap();

    Command::cargo_bin("gen-vault")
        .unwrap()
        .args(["--pages", "5", "--seed", "1", "--out"])
        .arg(tmp.path())
        .assert()
        .failure();
}
