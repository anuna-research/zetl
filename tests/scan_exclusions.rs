//! Integration tests for SPEC-026 vault scan exclusions.
//!
//! Implements TEST-200 through TEST-205 against the public scanner API.
//! Each test builds a synthetic vault layout in a tempdir, invokes
//! `scan_vault` with appropriate `ScanOptions`, and asserts on the set of
//! page names returned.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use tempfile::TempDir;
use zetl::scanner::{ScanOptions, scan_vault};

fn write(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn pages(root: &Path, opts: &ScanOptions) -> BTreeSet<String> {
    scan_vault(root, opts)
        .unwrap()
        .into_iter()
        .map(|f| f.page_name)
        .collect()
}

/// TEST-200: a vault with `notes/a.md` and `.claude/session.md` produces
/// only the `a` page when scanned with default options.
#[test]
fn test_200_dotdir_excluded_by_default() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write(root, "notes/a.md", "# A\n");
    write(root, ".claude/session.md", "# leak\n");

    let result = pages(root, &ScanOptions::default());

    assert!(
        result.contains("a"),
        "expected page 'a' to be present: {result:?}"
    );
    assert!(
        !result.iter().any(|p| p == "session" || p.contains("claude")),
        ".claude/ pages must not appear under default options: {result:?}"
    );
}

/// TEST-201: dotfiles directly at the vault root are still walked.
/// (Spec leaves the precise behaviour to clarify-during-implementation;
/// we pin "dotfiles at root are scanned" as the answer.)
#[test]
fn test_201_root_dotfile_is_scanned() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write(root, "notes/a.md", "# A\n");
    write(root, ".hidden-note.md", "# Hidden\n");

    let result = pages(root, &ScanOptions::default());

    assert!(result.contains("a"), "regular page missing: {result:?}");
    assert!(
        result.contains(".hidden-note") || result.contains("hidden-note"),
        "root dotfile should be walked: {result:?}"
    );
}

/// TEST-202: a `.zetlignore` containing `!.archive/` re-includes the
/// `.archive/` dir even though dotdirs are excluded by default.
#[test]
fn test_202_zetlignore_negation_overrides_default() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write(root, "notes/a.md", "# A\n");
    write(root, ".archive/old.md", "# Old\n");
    write(root, ".zetlignore", "!.archive/\n");

    let result = pages(root, &ScanOptions::default());

    assert!(result.contains("a"));
    assert!(
        result.contains("old"),
        ".zetlignore !pattern should re-include .archive/: {result:?}"
    );
}

/// TEST-203: --exclude PATTERN skips the matching directory; subsequent
/// scans without the flag include it again (i.e. flag is ephemeral).
#[test]
fn test_203_exclude_pattern_honoured() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write(root, "drafts/d.md", "# D\n");
    write(root, "notes/a.md", "# A\n");

    let with_exclude = pages(
        root,
        &ScanOptions::default().with_exclude_patterns(vec!["drafts/".into()]),
    );
    assert!(
        !with_exclude.contains("d"),
        "--exclude 'drafts/' should drop drafts: {with_exclude:?}"
    );
    assert!(with_exclude.contains("a"));

    let without = pages(root, &ScanOptions::default());
    assert!(
        without.contains("d"),
        "without --exclude, drafts must reappear: {without:?}"
    );
}

/// TEST-204: --include-hidden re-enables dotdir traversal, but the level-1
/// hardcoded `.zetl/` force-ignore is still applied.
#[test]
fn test_204_include_hidden_restores_walk_but_keeps_zetl_force_ignore() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write(root, "notes/a.md", "# A\n");
    write(root, ".claude/x.md", "# X\n");
    write(root, ".zetl/cache/y.md", "# Should never appear\n");

    let result = pages(root, &ScanOptions::default().with_include_hidden(true));

    assert!(
        result.contains("x"),
        "--include-hidden must walk .claude/: {result:?}"
    );
    assert!(
        !result.contains("y"),
        ".zetl/ must be force-ignored even with --include-hidden: {result:?}"
    );
}

/// TEST-205: precedence — CLI --exclude overrides .zetlignore negation.
#[test]
fn test_205_cli_exclude_overrides_zetlignore_negation() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write(root, ".foo/a.md", "# A\n");
    write(root, ".zetlignore", "!.foo/\n");

    // .zetlignore alone re-includes .foo/.
    let with_zetlignore_only = pages(root, &ScanOptions::default());
    assert!(
        with_zetlignore_only.contains("a"),
        ".zetlignore negation should re-include .foo/: {with_zetlignore_only:?}"
    );

    // Adding CLI --exclude '.foo/' wins.
    let with_cli_exclude = pages(
        root,
        &ScanOptions::default().with_exclude_patterns(vec![".foo/".into()]),
    );
    assert!(
        !with_cli_exclude.contains("a"),
        "CLI --exclude should beat .zetlignore re-include: {with_cli_exclude:?}"
    );
}

/// Bonus: explicit guard that nothing under `.git/`, `.zetl/`, or
/// `node_modules/` ever appears, regardless of options.
#[test]
fn test_force_ignored_dirs_never_scanned() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write(root, "notes/a.md", "# A\n");
    write(root, ".git/HEAD-fake.md", "# git\n");
    write(root, ".zetl/cache.md", "# zetl\n");
    write(root, "node_modules/lib.md", "# nm\n");

    for opts in [
        ScanOptions::default(),
        ScanOptions::default().with_include_hidden(true),
        ScanOptions::default().with_exclude_patterns(vec!["!.git/".into()]),
    ] {
        let result = pages(root, &opts);
        assert!(
            result.contains("a"),
            "regular page missing under opts {opts:?}: {result:?}"
        );
        for page in &result {
            assert!(
                !page.contains("HEAD-fake") && !page.contains("cache") && !page.contains("lib"),
                "force-ignored dir leaked under opts {opts:?}: {result:?}"
            );
        }
    }
}
