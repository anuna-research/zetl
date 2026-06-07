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
use zetl::scanner::{scan_vault, ScanOptions};

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

/// Make `root` a git repo. The `ignore` crate only applies positive
/// `.gitignore` patterns inside a git repository (`require_git` default),
/// so SPEC-043 tests that exercise `.gitignore` *hiding* paths must init
/// one — otherwise the gitignore is silently a no-op and the test is vacuous.
fn git_init(root: &Path) {
    let ok = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(root)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    assert!(ok, "git init failed (is git on PATH?)");
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
        !result
            .iter()
            .any(|p| p == "session" || p.contains("claude")),
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

/// Defect 1 (review): `--exclude '!pattern'` must produce a clear error,
/// not silently break by becoming `!!pattern` in the override stack.
#[test]
fn test_cli_exclude_negation_is_rejected() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    std::fs::write(root.join("a.md"), "# A").unwrap();

    let opts = ScanOptions::default().with_exclude_patterns(vec!["!.archive/".into()]);
    let err = scan_vault(root, &opts).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("--exclude") && msg.contains("!.archive/") && msg.contains(".zetlignore"),
        "expected diagnostic naming the offending pattern and pointing at .zetlignore: {msg}"
    );
}

/// Defect 3 (review) / REQ-205 level 3: a `.gitignore` with `!.archive/`
/// must re-include the dotdir even with no `.zetlignore`.
#[test]
fn test_gitignore_negation_overrides_dotdir_default() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write(root, "notes/a.md", "# A");
    write(root, ".archive/old.md", "# Old");
    write(root, ".gitignore", "!.archive/\n");

    let result = pages(root, &ScanOptions::default());
    assert!(result.contains("a"));
    assert!(
        result.contains("old"),
        ".gitignore !.archive/ should re-include the dotdir: {result:?}"
    );
}

/// Defect 4 (review) / REQ-205 level 1: `--exclude` patterns that touch
/// `.git/`, `.zetl/`, or `node_modules/` must not let the walker descend
/// into those directories. The hardcoded force-ignores are added to the
/// Override AFTER user patterns precisely so they win last-match-wins.
#[test]
fn test_user_exclude_cannot_unlock_force_ignored_dirs() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write(root, "notes/a.md", "# A");
    write(root, ".git/HEAD-fake.md", "# git");
    write(root, ".zetl/cache.md", "# zetl");
    write(root, "node_modules/lib.md", "# nm");

    // A user pattern that lexically overlaps with .git/ subpaths must
    // not cause the override stack to whitelist .git/ traversal.
    let opts = ScanOptions::default()
        .with_exclude_patterns(vec![".git/HEAD-fake".into(), ".zetl/cache".into()]);
    let result = pages(root, &opts);
    assert!(result.contains("a"));
    for page in &result {
        assert!(
            !page.contains("HEAD-fake") && !page.contains("cache") && !page.contains("lib"),
            "force-ignored dir leaked: {result:?}"
        );
    }
}

/// Risk (review): a vault rooted at e.g. `/Users/foo/.config/vault/`
/// has `file_name() == ".config"` (or similar dot-prefixed name) at
/// walker depth 0. The filter_entry must accept the root unconditionally
/// or the walk terminates immediately.
#[test]
fn test_dot_rooted_vault_is_walked() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join(".config-vault");
    std::fs::create_dir(&root).unwrap();
    write(&root, "notes/a.md", "# A");
    write(&root, "b.md", "# B");

    let result = pages(&root, &ScanOptions::default());
    assert!(
        result.contains("a") && result.contains("b"),
        "vault rooted at a dot-prefixed dir must still be walked: {result:?}"
    );
}

/// OBS-200: verbose mode emits `[zetl] scan: skipped <path> reason=<r>`
/// lines on stderr for each excluded entry. The reason taxonomy is part
/// of the contract — verify a representative sample.
#[test]
fn test_obs_200_verbose_emits_skip_lines() {
    use std::process::Command;

    // Build a tiny vault and invoke the binary with --verbose so we can
    // capture real stderr. (Calling scan_vault in-process can't easily
    // capture eprintln! output across thread boundaries.)
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write(root, "notes/a.md", "# A");
    write(root, ".claude/leak.md", "# leak");
    write(root, ".zetl/internal.md", "# zetl");

    let out_dir = tmp.path().join("dist");
    let bin = env!("CARGO_BIN_EXE_zetl");
    let output = Command::new(bin)
        .args(["-d", root.to_str().unwrap(), "--verbose", "build", "-o"])
        .arg(&out_dir)
        .output()
        .expect("failed to execute zetl binary");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "build failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        stderr
    );
    // The .claude/ directory exits via the dotdir branch of filter_entry.
    // (The hardcoded force-ignores .git/ / .zetl/ / node_modules/ are
    // removed by the WalkBuilder Override before filter_entry runs, so
    // they don't emit OBS-200 lines from the scanner; the watcher path
    // does emit "hardcoded" via classify_entry_os.)
    assert!(
        stderr.contains("[zetl] scan: skipped")
            && stderr.contains(".claude")
            && stderr.contains("reason=dotdir"),
        "expected OBS-200 dotdir skip line for .claude in stderr; got: {stderr}"
    );
}

/// REQ-207 (call-site audit): no production code path may pass
/// `&ScanOptions::default()` directly to `scan_vault` without an
/// `// SCAN-OPTS: intentional` annotation. The legitimate sites are
/// (a) the `cli.rs` fallback and (b) the legacy `web::reindex` shim.
#[test]
fn test_207_no_unannotated_default_scan_options_in_production() {
    use std::path::Path;

    fn walk(dir: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().and_then(|e| e.to_str()) == Some("rs") {
                if let Ok(text) = std::fs::read_to_string(&p) {
                    out.push(format!("{}\n{text}", p.display()));
                }
            }
        }
    }

    let mut sources = Vec::new();
    walk(Path::new("src"), &mut sources);

    let mut violations = Vec::new();
    for blob in &sources {
        let mut iter = blob.lines();
        let header = iter.next().unwrap_or("");
        let lines: Vec<&str> = iter.collect();
        let mut in_test = false;
        let mut depth = 0usize;
        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("#[cfg(test)]") || trimmed.starts_with("#[test]") {
                in_test = true;
            }
            if in_test {
                depth += line.matches('{').count();
                if depth > 0 {
                    depth -= line.matches('}').count();
                    if depth == 0 && line.contains('}') {
                        in_test = false;
                    }
                }
            }
            if in_test {
                continue;
            }
            let is_call = (line.contains("scan_vault(") || line.contains("reindex_with("))
                && line.contains("ScanOptions::default()");
            if !is_call {
                continue;
            }
            // Annotation may be on this line or the line immediately above.
            let prev = i.checked_sub(1).map(|j| lines[j]).unwrap_or("");
            if line.contains("SCAN-OPTS: intentional") || prev.contains("SCAN-OPTS: intentional") {
                continue;
            }
            violations.push(format!("{header}:{}: {}", i + 1, line.trim()));
        }
    }
    assert!(
        violations.is_empty(),
        "REQ-207: unannotated ScanOptions::default() in production scan_vault/reindex_with calls:\n{}",
        violations.join("\n")
    );
}

/// TEST-210 (SPEC-043): the minimal unlock. A directory gitignored by name
/// (exactly the `traces/` case) is invisible by default and revealed by
/// `--no-gitignore`. The corpus boundary and the git-tracking boundary are
/// different cuts of the same tree.
#[test]
fn test_210_no_gitignore_reveals_a_gitignored_dir() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    git_init(root);
    write(root, ".gitignore", "corpus/\n"); // gitignored, like traces/
    write(root, "a.md", "# A (tracked)\n");
    write(root, "corpus/note.md", "# note (gitignored corpus material)\n");

    // Default: git's opinion hides the corpus.
    let default = pages(root, &ScanOptions::default());
    assert_eq!(
        default,
        BTreeSet::from(["a".to_string()]),
        "default: gitignored corpus/ should be hidden: {default:?}"
    );

    // --no-gitignore: git's opinion is removed; the corpus is visible.
    let no_gi = pages(root, &ScanOptions::default().with_no_gitignore(true));
    assert_eq!(
        no_gi,
        BTreeSet::from(["a".to_string(), "note".to_string()]),
        "--no-gitignore: gitignored corpus/ should be revealed: {no_gi:?}"
    );
}

/// TEST-212 (SPEC-043): `.zetlignore` as sole authority, **blacklist style**.
/// Under `--no-gitignore`, git's `*`-whitelist is removed and `.zetlignore`
/// scopes the vault by *excluding* what the corpus view should not show.
/// This is the supported scoping idiom for the flag (cf. TEST-213 for the
/// whitelist-style limitation).
#[test]
fn test_212_no_gitignore_lets_zetlignore_blacklist_scope_the_vault() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    git_init(root);
    write(root, ".gitignore", "*\n!tracked.md\n"); // git: only tracked.md
    write(root, ".zetlignore", "secret/\n"); // corpus view: hide secret/
    write(root, "tracked.md", "# tracked (git-visible)\n");
    write(root, "corpus/note.md", "# note (gitignored corpus material)\n");
    write(root, "secret/leak.md", "# leak (blacklisted)\n");

    // git would show only `tracked`; --no-gitignore + blacklist shows
    // everything except the blacklisted subtree.
    let no_gi = pages(root, &ScanOptions::default().with_no_gitignore(true));
    assert_eq!(
        no_gi,
        BTreeSet::from(["tracked".to_string(), "note".to_string()]),
        "--no-gitignore: .zetlignore blacklist should hide only secret/: {no_gi:?}"
    );
}

/// TEST-213 (SPEC-043): the `*`-then-re-include *whitelist* idiom now works in
/// `.zetlignore`. zetl loads it via `add_custom_ignore_filename`, giving it the
/// `ignore` crate's full per-directory semantics — `*` excludes everything,
/// then `!corpus/` + `!corpus/**` re-includes the subtree. This is the
/// ergonomic corpus-view configuration: scope to exactly the roots you want.
#[test]
fn test_213_zetlignore_whitelist_idiom_scopes_the_vault() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    git_init(root);
    write(root, ".zetlignore", "*\n!corpus/\n!corpus/**\n");
    write(root, "tracked.md", "# tracked\n");
    write(root, "corpus/note.md", "# note\n");

    let no_gi = pages(root, &ScanOptions::default().with_no_gitignore(true));
    assert_eq!(
        no_gi,
        BTreeSet::from(["note".to_string()]),
        "whitelist .zetlignore should scope to corpus/: {no_gi:?}"
    );
}

/// TEST-214 (SPEC-043): promoting `.zetlignore` to a custom ignore filename
/// makes it outrank `.gitignore` — the precedence Configuration.md §REQ-205
/// has always documented. A `.zetlignore` negation re-includes a path that
/// `.gitignore` excludes, with git's ignore handling fully active (default,
/// not `--no-gitignore`).
#[test]
fn test_214_zetlignore_overrides_gitignore() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    git_init(root);
    write(root, ".gitignore", "corpus/\n"); // git hides corpus/
    write(root, ".zetlignore", "!corpus/\n!corpus/**\n"); // zetl re-includes it
    write(root, "a.md", "# A\n");
    write(root, "corpus/note.md", "# note\n");

    // Default options: .gitignore is active, but .zetlignore outranks it.
    let result = pages(root, &ScanOptions::default());
    assert!(
        result.contains("note"),
        ".zetlignore negation should override .gitignore exclusion: {result:?}"
    );
}

/// TEST-211 (SPEC-043): with no `.zetlignore` present, `--no-gitignore`
/// surfaces everything the dotdir/force-ignore defaults still permit —
/// i.e. git's opinion is removed but levels 1–2 hold. A `*`-whitelist
/// `.gitignore` would otherwise hide both regular files.
#[test]
fn test_211_no_gitignore_without_zetlignore_keeps_level_1_2_defaults() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    git_init(root);
    write(root, ".gitignore", "*\n"); // hide everything from git's view
    write(root, "a.md", "# A\n");
    write(root, "sub/b.md", "# B\n");
    write(root, ".claude/session.md", "# dotdir\n"); // still excluded (level 2)
    write(root, "node_modules/lib.md", "# nm\n"); // still excluded (level 1)

    let no_gi = pages(root, &ScanOptions::default().with_no_gitignore(true));
    assert!(
        no_gi.contains("a") && no_gi.contains("b"),
        "--no-gitignore should surface files hidden only by .gitignore: {no_gi:?}"
    );
    assert!(
        !no_gi.contains("session"),
        "level-2 dotdir default must still hold under --no-gitignore: {no_gi:?}"
    );
    assert!(
        !no_gi.contains("lib"),
        "level-1 force-ignore must still hold under --no-gitignore: {no_gi:?}"
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
        ScanOptions::default().with_exclude_patterns(vec!["unrelated/".into()]),
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
