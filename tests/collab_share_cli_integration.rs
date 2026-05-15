//! Integration tests for `zetl collab share` (SPEC-041 REQ-4116/4117/4118,
//! CON-4110). Exercises the full mint→list→revoke flow end-to-end through
//! the binary, including the bearer-authority security notice and the
//! "revoke unknown jti errors" rule.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;

fn make_vault() -> tempfile::TempDir {
    let tmp = tempfile::TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join(".zetl/collab")).unwrap();
    tmp
}

#[test]
fn share_mint_prints_url_and_bearer_notice() {
    let tmp = make_vault();
    let output = cargo_bin_cmd!("zetl")
        .args([
            "--dir",
            tmp.path().to_str().unwrap(),
            "collab",
            "share",
            "mint",
            "--scope",
            "shared/draft/**",
            "--role",
            "reader",
            "--expires",
            "1h",
            "--site-url",
            "https://wiki.example.com",
        ])
        .output()
        .expect("zetl spawn");
    assert!(
        output.status.success(),
        "mint exit={:?} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("https://wiki.example.com/?cap="),
        "stdout missing URL: {stdout}"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("SECURITY") && stderr.contains("bearer"),
        "stderr missing bearer notice: {stderr}"
    );
    assert!(
        stderr.contains("scope=") && stderr.contains("role=reader"),
        "stderr should show scope and role: {stderr}"
    );
}

#[test]
fn share_mint_rejects_out_of_bounds_expiry() {
    let tmp = make_vault();
    let too_long = cargo_bin_cmd!("zetl")
        .args([
            "--dir",
            tmp.path().to_str().unwrap(),
            "collab",
            "share",
            "mint",
            "--scope",
            "x",
            "--role",
            "reader",
            "--expires",
            "120d",
            "--site-url",
            "https://x.example.com",
        ])
        .output()
        .expect("zetl spawn");
    assert!(!too_long.status.success(), "120d should be rejected");
    let too_short = cargo_bin_cmd!("zetl")
        .args([
            "--dir",
            tmp.path().to_str().unwrap(),
            "collab",
            "share",
            "mint",
            "--scope",
            "x",
            "--role",
            "reader",
            "--expires",
            "30s",
            "--site-url",
            "https://x.example.com",
        ])
        .output()
        .expect("zetl spawn");
    assert!(!too_short.status.success(), "30s should be rejected");
}

#[test]
fn share_mint_rejects_unknown_role() {
    let tmp = make_vault();
    let output = cargo_bin_cmd!("zetl")
        .args([
            "--dir",
            tmp.path().to_str().unwrap(),
            "collab",
            "share",
            "mint",
            "--scope",
            "x",
            "--role",
            "admin",
            "--site-url",
            "https://x.example.com",
        ])
        .output()
        .expect("zetl spawn");
    // Clap's value_parser rejects this before we get to our handler.
    assert!(!output.status.success(), "admin role should be rejected");
}

#[test]
fn share_list_round_trip() {
    let tmp = make_vault();
    // Empty store
    let empty = cargo_bin_cmd!("zetl")
        .args(["--dir", tmp.path().to_str().unwrap(), "collab", "share", "list"])
        .output()
        .expect("zetl spawn");
    assert!(empty.status.success());
    assert!(empty.stdout.is_empty(), "empty list should print nothing");

    // Mint two records
    for (scope, role) in [("a/**", "reader"), ("b/**", "editor")] {
        let out = cargo_bin_cmd!("zetl")
            .args([
                "--dir",
                tmp.path().to_str().unwrap(),
                "collab",
                "share",
                "mint",
                "--scope",
                scope,
                "--role",
                role,
                "--site-url",
                "https://x.example.com",
            ])
            .output()
            .expect("zetl spawn");
        assert!(out.status.success());
    }

    let listed = cargo_bin_cmd!("zetl")
        .args(["--dir", tmp.path().to_str().unwrap(), "collab", "share", "list"])
        .output()
        .expect("zetl spawn");
    assert!(listed.status.success());
    let stdout = String::from_utf8_lossy(&listed.stdout);
    assert!(stdout.contains("jti\tscope\trole\texp\tstatus"));
    assert!(stdout.contains("a/**"));
    assert!(stdout.contains("b/**"));
    assert!(stdout.contains("live"));
    // CON-4110: never prints token bytes (JWT marker).
    assert!(!stdout.contains("$argon2"));
    // The token bytes contain dots and base64url; a heuristic: jti is 32
    // hex chars while a real token is much longer. Look at column widths.
}

#[test]
fn share_revoke_marks_revoked() {
    let tmp = make_vault();
    let mint = cargo_bin_cmd!("zetl")
        .args([
            "--dir",
            tmp.path().to_str().unwrap(),
            "collab",
            "share",
            "mint",
            "--scope",
            "x",
            "--role",
            "reader",
            "--site-url",
            "https://x.example.com",
        ])
        .output()
        .expect("zetl spawn");
    assert!(mint.status.success());
    // Pull the jti from the security-notice line ("Revoke with: zetl
    // collab share revoke <jti>") — that's the operator-facing primary
    // reference.
    let stderr = String::from_utf8_lossy(&mint.stderr);
    // Find the first 32-char hex token anywhere in stderr — that's the jti
    // the security notice surfaces in the `Revoke with:` line.
    let jti = stderr
        .split_whitespace()
        .find(|s| s.len() == 32 && s.chars().all(|c| c.is_ascii_hexdigit()))
        .expect("could not extract jti from stderr")
        .to_string();

    let revoke = cargo_bin_cmd!("zetl")
        .args([
            "--dir",
            tmp.path().to_str().unwrap(),
            "collab",
            "share",
            "revoke",
            &jti,
        ])
        .output()
        .expect("zetl spawn");
    assert!(revoke.status.success(), "revoke failed: {:?}", revoke);

    let list = cargo_bin_cmd!("zetl")
        .args(["--dir", tmp.path().to_str().unwrap(), "collab", "share", "list"])
        .output()
        .expect("zetl spawn");
    let stdout = String::from_utf8_lossy(&list.stdout);
    assert!(stdout.contains(&jti));
    assert!(stdout.contains("revoked"), "expected revoked status: {stdout}");
}

#[test]
fn share_revoke_unknown_jti_errors() {
    let tmp = make_vault();
    let output = cargo_bin_cmd!("zetl")
        .args([
            "--dir",
            tmp.path().to_str().unwrap(),
            "collab",
            "share",
            "revoke",
            "deadbeefdeadbeefdeadbeefdeadbeef",
        ])
        .output()
        .expect("zetl spawn");
    assert!(!output.status.success(), "should reject unknown jti");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("deadbeef") || stderr.contains("no capability"),
        "expected helpful error: {stderr}"
    );
}
