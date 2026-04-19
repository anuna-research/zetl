//! CON-3212 canonical-extension golden-HTML gate.
//!
//! Walks `tests/extension-fixtures/` and, for each fixture directory,
//! looks up the registered runner, feeds `input.md` through it, and
//! compares the output against `expected.html` after normalisation.
//!
//! The runner registry is shared with `tools/xtask` via a `#[path]`
//! include so `cargo xtask update-golden` and this gate produce
//! bit-for-bit identical output — the only path by which the expected
//! file gets (re)written is through the same runner the test asserts
//! on.
//!
//! The gate also carries a drift-detection canary: a tampered
//! `expected.html` feeds through the comparator in-memory and asserts a
//! diff is produced. That sanity-checks the rig itself so a future
//! refactor can't silently make every comparison pass.

use std::fs;
use std::path::PathBuf;

use zetl::extensions::golden::{compare, load_fixtures, normalise_html, render_diff};

#[path = "../tools/xtask/src/runners.rs"]
mod runners;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixtures_root() -> PathBuf {
    manifest_dir().join("tests/extension-fixtures")
}

#[test]
fn extension_fixtures_match_expected_html() {
    let root = fixtures_root();
    let fixtures =
        load_fixtures(&root).unwrap_or_else(|e| panic!("load_fixtures({}): {e}", root.display()));
    assert!(
        !fixtures.is_empty(),
        "no fixtures found under {}; the ext-golden-html gate needs at least \
         one canary fixture to stay meaningful",
        root.display()
    );

    let mut failures: Vec<String> = Vec::new();
    for fx in &fixtures {
        let runner = runners::resolve(&fx.name).unwrap_or_else(|| {
            panic!(
                "fixture '{}' has no registered runner — add it to \
                 tools/xtask/src/runners.rs so `cargo xtask update-golden` \
                 and this gate stay in lockstep",
                fx.name
            )
        });
        let input = fx
            .read_input()
            .unwrap_or_else(|e| panic!("read input for '{}': {e}", fx.name));
        let actual = runner(&input);
        let expected = match fx.read_expected() {
            Ok(Some(s)) => s,
            Ok(None) => {
                failures.push(format!(
                    "fixture '{}': expected.html is missing. Run `cargo xtask \
                     update-golden {}` after reviewing the rendered output.",
                    fx.name, fx.name
                ));
                continue;
            }
            Err(e) => panic!("read expected for '{}': {e}", fx.name),
        };
        let cmp = compare(&expected, &actual);
        if !cmp.matched {
            failures.push(format!(
                "fixture '{}' drifted from expected.html:\n{}\n\n\
                 To accept the change: `cargo xtask update-golden {}`\n\
                 (Review the diff above carefully — a surprising change often \
                 indicates a regression, not a fixture refresh.)",
                fx.name, cmp.diff, fx.name
            ));
        }
    }

    if !failures.is_empty() {
        panic!(
            "{} fixture{} drifted:\n\n{}",
            failures.len(),
            if failures.len() == 1 { "" } else { "s" },
            failures.join("\n\n")
        );
    }
}

#[test]
fn drift_canary_triggers_on_mismatch() {
    // The gate is only useful if it actually detects drift. Run a known
    // drifted comparison in-memory and assert the diff is produced. This
    // protects against refactors that accidentally no-op the comparator
    // (e.g. normalising `actual` away or swapping `==` for `true`).
    let expected = "<p>hello</p>\n<ul><li>a</li><li>b</li></ul>";
    let drifted = "<p>hello</p>\n<ul><li>a</li><li>c</li></ul>";
    let cmp = compare(expected, drifted);
    assert!(!cmp.matched, "comparator failed to flag a real drift");
    assert!(
        cmp.diff.contains("- <li>b</li>")
            || cmp.diff.contains("- b")
            || cmp.diff.contains("b</li>"),
        "diff output did not surface the drifted token:\n{}",
        cmp.diff
    );
}

#[test]
fn drift_canary_update_flow_writes_and_reads_round_trip() {
    // Mirrors what `cargo xtask update-golden` does: write the runner's
    // output, read it back, confirm round-trip equality. Runs in a tempdir
    // copy so the committed fixture never mutates under test.
    let tmp = tempfile::TempDir::new().unwrap();
    let fixture_dir = tmp.path().join("canary");
    fs::create_dir_all(&fixture_dir).unwrap();
    fs::write(fixture_dir.join("input.md"), "# Heading\n\nA paragraph.\n").unwrap();

    let fixtures = load_fixtures(tmp.path()).unwrap();
    assert_eq!(fixtures.len(), 1);
    let fx = &fixtures[0];
    let html = runners::baseline_markdown_runner(&fx.read_input().unwrap());
    fx.write_expected(&html).unwrap();
    let round_tripped = fx.read_expected().unwrap().unwrap();
    assert_eq!(round_tripped, html);

    // Second run with the same input should compare clean.
    let cmp = compare(&round_tripped, &html);
    assert!(
        cmp.matched,
        "expected round-tripped HTML to compare clean, got diff:\n{}",
        cmp.diff
    );
}

#[test]
fn normalisation_is_idempotent() {
    // Useful smoke for `compare`: running normalisation twice must be a
    // no-op. If this ever fires, the comparator is leaking non-canonical
    // state across calls and the gate's byte-for-byte guarantee breaks.
    let sample = "<a  class=\"x\"  id=\"y\">\n  hi\n</a>";
    let once = normalise_html(sample);
    let twice = normalise_html(&once);
    assert_eq!(once, twice);
}

#[test]
fn render_diff_labels_are_correct() {
    let d = render_diff("before", "a\nb\n", "after", "a\nc\n");
    assert!(d.starts_with("--- before\n+++ after\n"), "diff:\n{d}");
}
