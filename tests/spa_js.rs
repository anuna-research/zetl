//! Runs the #70 browser-runtime regression test for
//! `themes/default/static/spa.js` under node.
//!
//! The SPA shell keeps the sidebar persistent across pushState navigations, so
//! its page-relative `root_path` links must be frozen to absolute URLs on mount
//! — otherwise they compound the URL path on the next click (#70). The actual
//! assertions live in `tests/js/spa_navigation.test.mjs`; this wrapper invokes
//! node so the checks run as part of `cargo test`. If node is not installed the
//! test is skipped (logged), not failed — keeping the Rust-only build green on
//! machines without node.

use std::process::Command;

/// #73: on a host with SPA-style 404 fallback (200 + homepage HTML for any
/// unknown path), navigate() must detect that the fetched document is not
/// the page the URL points at and hard-navigate instead of swapping.
#[test]
fn spa_js_fallback_host_masking_detected() {
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("skip: `node` not available — spa.js fallback tests not run");
        return;
    }
    let status = Command::new("node")
        .arg("tests/js/spa_fallback.test.mjs")
        .status()
        .expect("spawn node");
    assert!(
        status.success(),
        "spa.js 404-fallback tests failed (run `node tests/js/spa_fallback.test.mjs`)"
    );
}

#[test]
fn spa_js_navigation_no_url_compounding() {
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("skip: `node` not available — spa.js navigation tests not run");
        return;
    }
    let status = Command::new("node")
        .arg("tests/js/spa_navigation.test.mjs")
        .status()
        .expect("spawn node");
    assert!(
        status.success(),
        "spa.js navigation tests failed (run `node tests/js/spa_navigation.test.mjs`)"
    );
}
