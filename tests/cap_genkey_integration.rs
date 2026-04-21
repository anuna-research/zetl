//! Integration tests for `zetl cap genkey` (SPEC-034 REQ-3419, TEST-3419).
//!
//! Covers:
//!
//! * The command exits 0 and prints BOTH env-var assignments to stdout.
//! * The printed `ZETL_CAP_SECRET` round-trips through the pure-core
//!   decoder (i.e. the base64 + checksum we ship is what the build-time
//!   parser accepts).
//! * A tampered checksum fails with a `Checksum`-class parse error and
//!   the remediation text ("re-run `zetl cap genkey`") is surfaced.
//! * `--json` produces a parseable JSON body.
//! * Neither secret ever reaches stderr — the acceptance criterion for
//!   the "secret substring never appears in any stderr/logs" guarantee.
//! * Two back-to-back invocations produce different secrets (no
//!   accidental deterministic seed wired into the production path).

use assert_cmd::cargo::cargo_bin_cmd;
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;

fn run_genkey(extra_args: &[&str]) -> (std::process::Output, String, String) {
    let mut args = vec!["cap", "genkey"];
    args.extend_from_slice(extra_args);
    let assert = cargo_bin_cmd!("zetl").args(&args).assert().success();
    let out = assert.get_output().clone();
    let stdout = String::from_utf8(out.stdout.clone()).expect("stdout is utf8");
    let stderr = String::from_utf8(out.stderr.clone()).expect("stderr is utf8");
    (out, stdout, stderr)
}

/// Pull the base64 value from a `export NAME='...'` line in `render_human`.
fn extract_export(stdout: &str, name: &str) -> String {
    let needle = format!("export {name}='");
    let start = stdout
        .find(&needle)
        .unwrap_or_else(|| panic!("missing `export {name}=` line:\n{stdout}"))
        + needle.len();
    let end = stdout[start..]
        .find('\'')
        .unwrap_or_else(|| panic!("unterminated `export {name}=` value:\n{stdout}"))
        + start;
    stdout[start..end].to_string()
}

#[test]
fn genkey_prints_both_env_vars_and_succeeds() {
    let (_out, stdout, _stderr) = run_genkey(&[]);
    assert!(
        stdout.contains("export ZETL_CAP_SECRET="),
        "missing ZETL_CAP_SECRET export line:\n{stdout}",
    );
    assert!(
        stdout.contains("export ZETL_CAP_SIGNING_KEY="),
        "missing ZETL_CAP_SIGNING_KEY export line:\n{stdout}",
    );
}

#[test]
fn genkey_output_parses_through_pure_core() {
    let (_out, stdout, _stderr) = run_genkey(&[]);
    let secret_b64 = extract_export(&stdout, "ZETL_CAP_SECRET");
    // Must round-trip the checksum check that the build uses at startup.
    let parsed = zetl::cap::genkey::decode_secret(&secret_b64)
        .expect("genkey output must parse under pure-core decoder");
    assert_eq!(parsed.version(), zetl::cap::genkey::SECRET_VERSION_V1);
    assert_eq!(
        parsed.random_body().len(),
        zetl::cap::genkey::SECRET_RANDOM_LEN
    );

    // Signing key must be 32 base64-decoded bytes.
    let sk_b64 = extract_export(&stdout, "ZETL_CAP_SIGNING_KEY");
    let sk = STANDARD.decode(&sk_b64).expect("signing key is base64");
    assert_eq!(sk.len(), zetl::cap::genkey::SIGNING_KEY_LEN);
}

#[test]
fn tampered_checksum_yields_remediation_error() {
    let (_out, stdout, _stderr) = run_genkey(&[]);
    let secret_b64 = extract_export(&stdout, "ZETL_CAP_SECRET");

    // Flip a bit in the last checksum byte.
    let mut raw = STANDARD
        .decode(&secret_b64)
        .expect("produced secret is base64");
    let last = raw.len() - 1;
    raw[last] ^= 0x01;
    let mangled = STANDARD.encode(&raw);

    let err = match zetl::cap::genkey::decode_secret(&mangled) {
        Ok(_) => panic!("tampered checksum must be rejected"),
        Err(e) => e,
    };
    assert!(
        matches!(err, zetl::cap::genkey::SecretParseError::Checksum { .. }),
        "expected Checksum variant, got {err:?}",
    );
    let rendered = format!("{err}");
    assert!(
        rendered.to_lowercase().contains("re-run `zetl cap genkey`"),
        "error missing remediation text:\n{rendered}",
    );
}

#[test]
fn genkey_never_logs_secret_to_stderr() {
    // BUG-017 + REQ-3419 acceptance: the secret is printed exactly once,
    // on stdout. Any appearance on stderr would mean the value also
    // reached every log sink that tails stderr (systemd-journald, CI
    // log aggregators, etc.) — which defeats the "password-manager
    // only" storage guidance.
    let (_out, stdout, stderr) = run_genkey(&[]);
    let secret_b64 = extract_export(&stdout, "ZETL_CAP_SECRET");
    let signing_b64 = extract_export(&stdout, "ZETL_CAP_SIGNING_KEY");
    assert!(
        !stderr.contains(&secret_b64),
        "ZETL_CAP_SECRET leaked to stderr:\n{stderr}",
    );
    assert!(
        !stderr.contains(&signing_b64),
        "ZETL_CAP_SIGNING_KEY leaked to stderr:\n{stderr}",
    );
}

#[test]
fn genkey_json_mode_is_parseable() {
    let assert = cargo_bin_cmd!("zetl")
        .args(["--json", "cap", "genkey"])
        .assert()
        .success();
    let out = assert.get_output();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let j: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout must be valid JSON under --json");
    assert_eq!(j["command"], "zetl cap genkey");
    assert_eq!(j["secret"]["env"], "ZETL_CAP_SECRET");
    assert_eq!(j["signing_key"]["env"], "ZETL_CAP_SIGNING_KEY");
    // Round-trip the embedded secret through the pure-core decoder.
    let secret_b64 = j["secret"]["value"]
        .as_str()
        .expect("secret.value is a string");
    zetl::cap::genkey::decode_secret(secret_b64).expect("emitted secret parses");
}

#[test]
fn two_invocations_produce_distinct_secrets() {
    // Guard against the failure mode of a refactor wiring a constant
    // seed into the production generate() path.
    let (_a_out, a_stdout, _a_err) = run_genkey(&[]);
    let (_b_out, b_stdout, _b_err) = run_genkey(&[]);
    let a_secret = extract_export(&a_stdout, "ZETL_CAP_SECRET");
    let b_secret = extract_export(&b_stdout, "ZETL_CAP_SECRET");
    assert_ne!(a_secret, b_secret);
    let a_sk = extract_export(&a_stdout, "ZETL_CAP_SIGNING_KEY");
    let b_sk = extract_export(&b_stdout, "ZETL_CAP_SIGNING_KEY");
    assert_ne!(a_sk, b_sk);
}

#[test]
fn genkey_output_includes_bug017_safeguard_framing() {
    let (_out, stdout, _stderr) = run_genkey(&[]);
    let lower = stdout.to_lowercase();
    assert!(
        lower.contains("ux safeguard") && lower.contains("not a security control"),
        "BUG-017 safeguard framing missing from genkey banner:\n{stdout}",
    );
}
