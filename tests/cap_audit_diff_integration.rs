//! Integration tests for `zetl cap audit-diff` (SPEC-034 REQ-3424,
//! ADR-3410). Drives the full CLI against `tools/audit-diff-corpus/`
//! to verify:
//!
//! - Every corpus fixture's expected-marker set fires — the
//!   `audit-corpus` CI gate's contract.
//! - Negative-case fixtures (`011-known-domain-ok`) produce no findings.
//! - Git-backed mode classifies an unseen domain in a fresh vault.
//! - `--corpus <dir>` single-fixture mode exits 1 if expected markers
//!   are missed.
//!
//! The corpus runner is re-invoked here via `cargo run --bin zetl`
//! rather than the pure library so CLI wiring drift is caught.

use std::path::{Path, PathBuf};
use std::process::Command;

use zetl::cap::audit_diff::{scan_diff, Page};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn corpus_root() -> PathBuf {
    repo_root().join("tools").join("audit-diff-corpus")
}

fn read_md_tree(dir: &Path) -> Vec<(PathBuf, String)> {
    if !dir.exists() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for entry in walkdir(dir) {
        if entry.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        let rel = entry.strip_prefix(dir).unwrap_or(&entry).to_path_buf();
        let body = std::fs::read_to_string(&entry).unwrap();
        out.push((rel, body));
    }
    out.sort();
    out
}

fn walkdir(root: &Path) -> Vec<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    let mut out = Vec::new();
    while let Some(p) = stack.pop() {
        if p.is_dir() {
            for e in std::fs::read_dir(&p).unwrap().flatten() {
                stack.push(e.path());
            }
        } else {
            out.push(p);
        }
    }
    out
}

fn fixtures() -> Vec<PathBuf> {
    let fx_dir = corpus_root().join("fixtures");
    let mut out: Vec<PathBuf> = std::fs::read_dir(&fx_dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    out.sort();
    out
}

/// REQ-3424 acceptance — every fixture's `expected.txt` marker set
/// must fire in the scan output.
#[test]
fn every_corpus_fixture_fires_its_expected_markers() {
    for fx in fixtures() {
        let name = fx.file_name().unwrap().to_string_lossy().into_owned();
        let baseline = read_md_tree(&fx.join("baseline"));
        let new = read_md_tree(&fx.join("new"));

        let baseline_pages: Vec<Page> = baseline
            .iter()
            .map(|(p, c)| Page {
                path: p.as_path(),
                contents: c.as_str(),
            })
            .collect();
        let new_pages: Vec<Page> = new
            .iter()
            .map(|(p, c)| Page {
                path: p.as_path(),
                contents: c.as_str(),
            })
            .collect();
        let findings = scan_diff(&baseline_pages, &new_pages);
        let got_tags: Vec<&str> = findings.iter().map(|f| f.kind.tag()).collect();

        let expected_path = fx.join("expected.txt");
        assert!(
            expected_path.exists(),
            "fixture {name} missing expected.txt",
        );
        let raw = std::fs::read_to_string(&expected_path).unwrap();
        for line in raw.lines() {
            let wanted = line.trim();
            if wanted.is_empty() || wanted.starts_with('#') {
                continue;
            }
            assert!(
                got_tags.contains(&wanted),
                "fixture {name}: expected finding kind `{wanted}` not detected; got {got_tags:?}",
            );
        }
    }
}

/// Negative-case regression — the `011-known-domain-ok` fixture must
/// produce zero findings; a false positive here means the domain
/// baseline or `www.` normalisation regressed.
#[test]
fn known_domain_ok_produces_no_findings() {
    let fx = corpus_root().join("fixtures").join("011-known-domain-ok");
    let baseline = read_md_tree(&fx.join("baseline"));
    let new = read_md_tree(&fx.join("new"));

    let baseline_pages: Vec<Page> = baseline
        .iter()
        .map(|(p, c)| Page {
            path: p.as_path(),
            contents: c.as_str(),
        })
        .collect();
    let new_pages: Vec<Page> = new
        .iter()
        .map(|(p, c)| Page {
            path: p.as_path(),
            contents: c.as_str(),
        })
        .collect();
    let findings = scan_diff(&baseline_pages, &new_pages);
    assert!(
        findings.is_empty(),
        "known-domain-ok fixture regressed: {findings:?}",
    );
}

/// End-to-end CLI: `zetl cap audit-diff --corpus-root` exits 0 on a
/// clean corpus run. Re-invokes the binary to catch clap-plumbing drift.
#[test]
fn cli_corpus_root_mode_exits_zero_on_clean_run() {
    let zetl_bin = zetl_bin();
    let out = Command::new(&zetl_bin)
        .args([
            "cap",
            "audit-diff",
            "--corpus-root",
            corpus_root().to_str().unwrap(),
        ])
        .output()
        .expect("spawning zetl");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "audit-diff --corpus-root exited non-zero — stdout:\n{stdout}\nstderr:\n{stderr}",
    );
    assert!(
        stdout.contains("fixture(s) OK"),
        "expected 'fixture(s) OK' in output, got:\n{stdout}",
    );
}

/// Negative CLI path: a synthesised fixture whose expected marker
/// cannot be detected must make the CLI exit 1. Proves the regression
/// gate actually bites on detector regressions.
#[test]
fn cli_corpus_single_fixture_fails_on_missed_marker() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = tmp.path().join("bogus");
    std::fs::create_dir_all(fx.join("new")).unwrap();
    std::fs::write(
        fx.join("new").join("page.md"),
        "# innocuous\n\nno findings here.\n",
    )
    .unwrap();
    std::fs::write(fx.join("expected.txt"), "unseen-domain\n").unwrap();

    let zetl_bin = zetl_bin();
    let out = Command::new(&zetl_bin)
        .args(["cap", "audit-diff", "--corpus", fx.to_str().unwrap()])
        .output()
        .expect("spawning zetl");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "expected non-zero exit on missed marker, stdout:\n{stdout}",
    );
    assert!(
        stderr.contains("CORPUS MISS"),
        "expected CORPUS MISS diagnostic in stderr, got:\n{stderr}",
    );
}

/// Git-mode smoke test — build a scratch repo, commit a clean page,
/// then stage a page introducing an unseen domain, and drive
/// `cap audit-diff HEAD~1 HEAD`. Validates the git plumbing without
/// relying on the host repo's state.
#[test]
fn cli_git_mode_detects_unseen_domain_between_refs() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    git(repo, &["init", "-q", "-b", "main"]);
    git(repo, &["config", "user.email", "t@t"]);
    git(repo, &["config", "user.name", "t"]);
    git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::write(repo.join("a.md"), "# a\n\n[ok](https://example.com/)\n").unwrap();
    git(repo, &["add", "a.md"]);
    git(repo, &["commit", "-q", "-m", "baseline"]);

    std::fs::write(repo.join("b.md"), "# b\n\n[bad](https://evil.test/drop)\n").unwrap();
    git(repo, &["add", "b.md"]);
    git(repo, &["commit", "-q", "-m", "add evil link"]);

    let zetl_bin = zetl_bin();
    let out = Command::new(&zetl_bin)
        .args([
            "-d",
            repo.to_str().unwrap(),
            "cap",
            "audit-diff",
            "HEAD~1",
            "HEAD",
        ])
        .output()
        .expect("spawning zetl");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    // Findings → exit 1 by design.
    assert!(
        !out.status.success(),
        "audit-diff should exit 1 when findings present; stdout:\n{stdout}\nstderr:\n{stderr}",
    );
    assert!(
        stdout.contains("unseen-domain") && stdout.contains("evil.test"),
        "expected unseen-domain/evil.test in output, got:\n{stdout}",
    );
}

fn zetl_bin() -> PathBuf {
    // `CARGO_BIN_EXE_zetl` is set by cargo for integration tests when
    // the package has a `[[bin]]` named `zetl`. Falls back to the
    // debug build path for IDE runners that skip the env-var.
    if let Some(p) = option_env!("CARGO_BIN_EXE_zetl") {
        return PathBuf::from(p);
    }
    repo_root().join("target").join("debug").join("zetl")
}

fn git(repo: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .expect("spawning git");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr),
    );
}
