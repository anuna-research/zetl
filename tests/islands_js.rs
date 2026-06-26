//! Runs the SPEC-050 browser-runtime tests for `src/web/assets/islands.js` under node.
//!
//! `islands.js` is the trusted host / reference monitor for the untrusted-Worker boundary;
//! its security-critical primitives (the CON-5005 value recogniser and the CON-5007
//! controlled-element renderer) need executable coverage. The actual assertions live in
//! `tests/js/islands_runtime.test.mjs`; this wrapper invokes node so the checks run as
//! part of `cargo test`. If node is not installed, the test is skipped (logged), not
//! failed — keeping the Rust-only build green on machines without node.

use std::process::Command;

#[test]
fn islands_js_runtime_security() {
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("skip: `node` not available — islands.js runtime tests not run");
        return;
    }
    let status = Command::new("node")
        .arg("tests/js/islands_runtime.test.mjs")
        .status()
        .expect("spawn node");
    assert!(
        status.success(),
        "islands.js runtime security tests failed (run `node tests/js/islands_runtime.test.mjs`)"
    );
}
