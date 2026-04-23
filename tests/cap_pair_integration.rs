//! Integration tests for `ztl cap pair` (SPEC-034 REQ-3408 CLI,
//! REQ-3416 `pair` verb, TEST-3408 "pair variant").
//!
//! Coverage:
//!
//!   * Two-party convergence end-to-end: grantor runs interactively,
//!     grantee runs one-shot, grantor verifies the HMAC over the
//!     grantee's pubkey (TEST-3408 acceptance spine).
//!   * Wrong phrase on the grantee side yields HMAC mismatch + exit 1
//!     on the grantor.
//!   * Phrase reuse within 30 days is refused (`.ztl/caps/.pair-nonces`).
//!   * `--grantor --phrase <P>` (test hook) writes the phrase into the
//!     nonce store so later reuse is gated even without RNG draws.
//!   * Pubkey length validation surfaces on stderr.

use assert_cmd::cargo::cargo_bin_cmd;
use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine as _};
use predicates::prelude::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};

/// Canonical 32-byte pubkey (base64, no padding) used for every test.
/// Content is arbitrary — we only care that the same bytes survive
/// the SPAKE2 + HMAC round trip.
const GRANTEE_PUBKEY_B64: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";

fn fresh_vault(name: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(name)
        .tempdir()
        .expect("tempdir")
}

/// Spawn `ztl cap pair --grantor --phrase <phrase> --json` with piped
/// stdin/stdout, read the `"phase":"prompt"` JSON line, return
/// `(child, handshake_b64)`. Caller writes the three response lines to
/// `child.stdin`, then waits for the child's `"phase":"verified"` /
/// error exit.
fn spawn_grantor(
    vault: &std::path::Path,
    phrase: &str,
) -> (
    std::process::Child,
    BufReader<std::process::ChildStdout>,
    String,
) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ztl"));
    cmd.args([
        "-d",
        vault.to_str().unwrap(),
        "--json",
        "cap",
        "pair",
        "--grantor",
        "--phrase",
        phrase,
    ]);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn grantor");
    let stdout = child.stdout.take().expect("grantor stdout");
    let mut reader = BufReader::new(stdout);

    // Read the first JSON line (the prompt phase). Protocol has at
    // most two JSON lines in `--json` mode so we know the first one
    // is the handshake.
    let mut line = String::new();
    reader.read_line(&mut line).expect("grantor prompt line");
    let v: serde_json::Value = serde_json::from_str(line.trim())
        .unwrap_or_else(|e| panic!("grantor prompt not JSON: {line}\n{e}"));
    assert_eq!(
        v["phase"].as_str(),
        Some("prompt"),
        "unexpected phase: {line}"
    );
    let handshake = v["handshake_b64"].as_str().unwrap().to_string();
    (child, reader, handshake)
}

/// Run `ztl cap pair --grantee` one-shot. Returns parsed JSON output
/// or panics with stderr on failure.
fn run_grantee(
    vault: &std::path::Path,
    peer_handshake: &str,
    phrase: &str,
    pubkey_b64: &str,
) -> serde_json::Value {
    let out = cargo_bin_cmd!("ztl")
        .args([
            "-d",
            vault.to_str().unwrap(),
            "--json",
            "cap",
            "pair",
            "--grantee",
            "--peer",
            peer_handshake,
            "--phrase",
            phrase,
            "--pubkey",
            pubkey_b64,
        ])
        .assert()
        .success();
    serde_json::from_slice(&out.get_output().stdout).expect("grantee stdout is valid JSON")
}

#[test]
fn grantor_and_grantee_converge_and_verify_pubkey() {
    // TEST-3408 pair variant — the spine of the acceptance criterion.
    // Two separate vaults; the phrase is the only shared state.
    let grantor_vault = fresh_vault("ztl-cap-pair-grantor");
    let grantee_vault = fresh_vault("ztl-cap-pair-grantee");

    // Pin the phrase via --phrase so the test is deterministic. The
    // RNG path is covered by the pure-core test suite in
    // `src/cap/pair.rs`.
    let phrase = "abandon ability able about";

    let (mut grantor, mut grantor_stdout, grantor_handshake) =
        spawn_grantor(grantor_vault.path(), phrase);

    // Grantee replies.
    let grantee = run_grantee(
        grantee_vault.path(),
        &grantor_handshake,
        phrase,
        GRANTEE_PUBKEY_B64,
    );
    let grantee_handshake = grantee["handshake_b64"].as_str().unwrap();
    let grantee_hmac = grantee["hmac_b64"].as_str().unwrap();

    // Relay three lines into the grantor's stdin.
    {
        let stdin = grantor.stdin.as_mut().expect("grantor stdin");
        writeln!(stdin, "{grantee_handshake}").unwrap();
        writeln!(stdin, "{GRANTEE_PUBKEY_B64}").unwrap();
        writeln!(stdin, "{grantee_hmac}").unwrap();
    }

    // Second JSON line = verification verdict.
    let mut verdict = String::new();
    grantor_stdout
        .read_line(&mut verdict)
        .expect("grantor verdict line");
    let v: serde_json::Value = serde_json::from_str(verdict.trim())
        .unwrap_or_else(|e| panic!("verdict not JSON: {verdict}\n{e}"));
    assert_eq!(v["phase"].as_str(), Some("verified"), "verdict: {verdict}");
    assert_eq!(v["pubkey_b64"].as_str(), Some(GRANTEE_PUBKEY_B64));

    let status = grantor.wait().expect("grantor exit");
    assert!(status.success(), "grantor exited non-zero");
}

#[test]
fn wrong_phrase_on_grantee_rejects_handoff() {
    // If the grantee's phrase differs, the two SPAKE2 sessions derive
    // different keys and the HMAC tag fails verification at the
    // grantor. Exit 1, with a diagnostic on stderr.
    let grantor_vault = fresh_vault("ztl-cap-pair-wrong-g");
    let grantee_vault = fresh_vault("ztl-cap-pair-wrong-gt");

    let grantor_phrase = "abandon ability able about";
    let grantee_phrase = "abandon ability able above"; // one-word swap

    let (mut grantor, _grantor_stdout, grantor_handshake) =
        spawn_grantor(grantor_vault.path(), grantor_phrase);

    let grantee = run_grantee(
        grantee_vault.path(),
        &grantor_handshake,
        grantee_phrase,
        GRANTEE_PUBKEY_B64,
    );
    let grantee_handshake = grantee["handshake_b64"].as_str().unwrap();
    let grantee_hmac = grantee["hmac_b64"].as_str().unwrap();

    {
        let stdin = grantor.stdin.as_mut().expect("grantor stdin");
        writeln!(stdin, "{grantee_handshake}").unwrap();
        writeln!(stdin, "{GRANTEE_PUBKEY_B64}").unwrap();
        writeln!(stdin, "{grantee_hmac}").unwrap();
    }

    let status = grantor.wait().expect("grantor exit");
    assert!(
        !status.success(),
        "grantor exited 0 despite phrase mismatch",
    );
}

#[test]
fn grantor_phrase_reuse_within_ttl_is_refused() {
    // REQ-3408 nonce-store gate: the same phrase cannot be reused at
    // the grantor within 30 days. We probe by passing the same
    // pinned phrase twice in a row.
    let vault = fresh_vault("ztl-cap-pair-reuse-grantor");
    let phrase = "abandon ability able about";

    // First run — it'll block on stdin after writing the prompt; we
    // close stdin immediately so SPAKE2 fails with EOF, but the
    // nonce-store row is already persisted before we start the
    // session.
    {
        let (mut child, _stdout, _hs) = spawn_grantor(vault.path(), phrase);
        drop(child.stdin.take()); // EOF → grantor errors after prompt
        let _ = child.wait();
    }

    // Second run with the same phrase → refused before we ever get
    // to the prompt. Exit non-zero, stderr mentions the window.
    let out = cargo_bin_cmd!("ztl")
        .args([
            "-d",
            vault.path().to_str().unwrap(),
            "cap",
            "pair",
            "--grantor",
            "--phrase",
            phrase,
        ])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(
        stderr.contains("reuse window"),
        "stderr missing reuse-window hint: {stderr}",
    );
}

#[test]
fn grantee_phrase_reuse_within_ttl_is_refused() {
    // Parallel gate on the grantee side — the same vault cannot
    // --grantee with the same phrase twice inside the TTL. Each
    // --grantee run accepts the phrase into the local nonce store.
    let vault = fresh_vault("ztl-cap-pair-reuse-grantee");
    let phrase = "abandon ability able about";

    // Fabricate a minimal valid SPAKE2 handshake for the grantee to
    // consume. SPAKE2 messages here are 1 side-byte + element-length
    // — the exact values don't matter because the nonce-store gate
    // fires before the SPAKE2 step.

    // Actually the nonce-store gate fires AFTER the handshake decode
    // but BEFORE the SPAKE2 finish, so garbage-but-decodable base64
    // is fine. Use an empty string — it'll decode to zero bytes and
    // fail the base64 / length checks, but the first run still
    // persists a nonce row if we supply a passable handshake.

    // To reliably persist a row on the first run we need the nonce
    // accept to succeed. That requires only the phrase hash path;
    // the SPAKE2 step failing is fine (the row is written before
    // SPAKE2 starts).

    // Use a real grantor handshake from a throwaway spawn.
    let throwaway = fresh_vault("ztl-cap-pair-reuse-grantee-probe");
    let (mut g, _s, hs) = spawn_grantor(throwaway.path(), "abandon ability able above");
    drop(g.stdin.take());
    let _ = g.wait();

    // First grantee run — persists the phrase into the grantee's
    // nonce store.
    let _first = cargo_bin_cmd!("ztl")
        .args([
            "-d",
            vault.path().to_str().unwrap(),
            "cap",
            "pair",
            "--grantee",
            "--peer",
            &hs,
            "--phrase",
            phrase,
            "--pubkey",
            GRANTEE_PUBKEY_B64,
        ])
        .assert();

    // Second grantee run with SAME phrase → refused.
    let out = cargo_bin_cmd!("ztl")
        .args([
            "-d",
            vault.path().to_str().unwrap(),
            "cap",
            "pair",
            "--grantee",
            "--peer",
            &hs,
            "--phrase",
            phrase,
            "--pubkey",
            GRANTEE_PUBKEY_B64,
        ])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(
        stderr.contains("reuse window"),
        "stderr missing reuse-window hint: {stderr}",
    );
}

#[test]
fn grantee_rejects_short_pubkey() {
    // 31-byte pubkey — short by one. Surface the length diagnostic.
    let vault = fresh_vault("ztl-cap-pair-bad-pubkey");
    let phrase = "abandon ability able about";
    // Fake peer handshake — base64 of 33 bytes (1 side-byte + 32).
    let fake_peer = STANDARD_NO_PAD.encode(vec![0x53u8; 33]);
    let short_pubkey = STANDARD_NO_PAD.encode([0u8; 31]);

    cargo_bin_cmd!("ztl")
        .args([
            "-d",
            vault.path().to_str().unwrap(),
            "cap",
            "pair",
            "--grantee",
            "--peer",
            &fake_peer,
            "--phrase",
            phrase,
            "--pubkey",
            &short_pubkey,
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("must decode to"));
}

#[test]
fn grantor_auto_generates_phrase_when_flag_omitted() {
    // Without `--phrase` the grantor draws a fresh phrase from OsRng
    // and prints it as part of the prompt line. Sanity-check: phrase
    // is 4 ASCII-letter tokens and the handshake survives base64.
    let vault = fresh_vault("ztl-cap-pair-fresh-phrase");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ztl"));
    cmd.args([
        "-d",
        vault.path().to_str().unwrap(),
        "--json",
        "cap",
        "pair",
        "--grantor",
    ]);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn grantor");
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).expect("prompt line");
    let v: serde_json::Value = serde_json::from_str(line.trim()).expect("valid json");
    let phrase = v["phrase"].as_str().unwrap();
    let handshake = v["handshake_b64"].as_str().unwrap();

    assert_eq!(
        phrase.split_whitespace().count(),
        4,
        "expected 4 words, got {phrase:?}",
    );
    assert!(
        STANDARD_NO_PAD.decode(handshake.as_bytes()).is_ok(),
        "handshake not valid base64: {handshake}"
    );

    // Close stdin so the child errors out cleanly instead of hanging
    // the test harness waiting for three response lines.
    drop(child.stdin.take());
    let _ = child.wait();

    // Drain stdout / stderr to prevent deadlock on child cleanup.
    let mut rest = String::new();
    let _ = reader.read_to_string(&mut rest);
}

#[test]
fn human_output_contains_security_warning() {
    // In non-JSON mode the grantor prints an explicit instruction to
    // share the phrase over a trusted channel. Pin the string so a
    // rewrite that drops the advisory regresses.
    let vault = fresh_vault("ztl-cap-pair-human");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ztl"));
    cmd.args([
        "-d",
        vault.path().to_str().unwrap(),
        "-f",
        "table",
        "cap",
        "pair",
        "--grantor",
        "--phrase",
        "abandon ability able about",
    ]);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn grantor");
    // Close stdin immediately — we'll error after the prompt, but
    // we're probing the prompt body, not the SPAKE2 finish.
    drop(child.stdin.take());

    let mut out = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    let _ = child.wait();

    assert!(
        out.contains("TRUSTED channel"),
        "human output missing trust advisory:\n{out}",
    );
    assert!(
        out.contains("Pairing phrase"),
        "human output missing phrase header:\n{out}",
    );
    assert!(
        out.contains("Handshake message"),
        "human output missing handshake header:\n{out}",
    );
}
