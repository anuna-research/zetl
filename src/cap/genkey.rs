//! `ztl cap genkey` core — generate + parse the two secrets that power
//! capability-mode builds (SPEC-034 REQ-3419).
//!
//! This module emits BOTH:
//!   * `ztl_CAP_SECRET` — 48 bytes total, base64-encoded:
//!     `[version:1][random:32][checksum:15]`
//!     The checksum is a truncated HMAC-SHA256 over the version byte and
//!     the 32 random bytes, keyed with a fixed domain-separation string.
//!   * `ztl_CAP_SIGNING_KEY` — 32-byte Ed25519 private scalar, base64.
//!
//! # Security framing (BUG-017)
//!
//! The 15-byte checksum is deliberately framed as a **UX safeguard**,
//! not as a security control. An attacker who can tamper with the
//! operator's env var can also derive a matching checksum — the HMAC
//! key is fixed and public. Its only purpose is to detect
//! paste-corruption and copy-truncation at build time and emit a
//! remediation message pointing at `ztl cap genkey`. The secret's
//! real security rests on the 256 bits of OS-CSPRNG randomness in the
//! 32-byte body.
//!
//! # Purity
//!
//! `generate_with_rng` is a pure function of the `CryptoRng` input.
//! `encode_secret`, `decode_secret`, and `compute_checksum` have no
//! side effects. The effectful shell (`src/main.rs::cmd_cap_genkey`)
//! supplies [`rand_core::OsRng`] via the convenience wrapper
//! [`generate`].

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use ed25519_dalek::SigningKey;
use hmac::{Hmac, Mac};
use rand_core::{CryptoRng, OsRng, RngCore};
use sha2::Sha256;
use zeroize::Zeroize;

type HmacSha256 = Hmac<Sha256>;

/// Version byte pinning the wire format of `ztl_CAP_SECRET`. Any
/// future change to the layout (e.g. different checksum length or a
/// post-quantum checksum construction) SHALL bump this byte so older
/// builds reject the new format with a clear diagnostic rather than
/// misinterpreting the bytes.
pub const SECRET_VERSION_V1: u8 = 0x01;

/// Length of the OS-CSPRNG random body embedded in a `ztl_CAP_SECRET`.
/// 32 bytes ≈ 256 bits — the cohort-secret strength SPEC-034 §8 assumes
/// as IKM for every HKDF derivation.
pub const SECRET_RANDOM_LEN: usize = 32;

/// Length of the truncated HMAC-SHA256 checksum appended to the secret.
/// 15 bytes (120 bits) is the SPEC-034 REQ-3419 layout; truncation of
/// HMAC-SHA256 to any prefix is well-defined (RFC 2104 §5).
pub const SECRET_CHECKSUM_LEN: usize = 15;

/// Total on-the-wire length of the decoded `ztl_CAP_SECRET` — 1 + 32 + 15 = 48.
pub const SECRET_TOTAL_LEN: usize = 1 + SECRET_RANDOM_LEN + SECRET_CHECKSUM_LEN;

/// Length of an Ed25519 private scalar. Public constant so callers can
/// size buffers without pulling in `ed25519-dalek` transitively.
pub const SIGNING_KEY_LEN: usize = 32;

/// Domain-separation key for the UX safeguard checksum. The operator
/// does NOT need to protect this — the checksum is not a security
/// control; see module docs. Pinned so every ztl binary computes the
/// same checksum for the same random body, letting a build reject a
/// paste-mangled secret with a targeted hint.
const CHECKSUM_HMAC_KEY: &[u8] = b"ztl/cap-secret-checksum/v1";

/// Env-var name for the encryption secret. Exported as a constant so
/// tests, docs, and `ztl cap genkey` output stay in lock-step.
pub const ztl_CAP_SECRET_ENV: &str = "ztl_CAP_SECRET";

/// Env-var name for the Ed25519 signing private key.
pub const ztl_CAP_SIGNING_KEY_ENV: &str = "ztl_CAP_SIGNING_KEY";

/// Result of generating both capability-mode secrets. Consumers should
/// treat this struct as transient: encode it, print it, and drop it.
/// No `Debug` impl on purpose — printing a `GenkeyOutput` should always
/// route through [`GenkeyOutput::render_human`] / the JSON helper so
/// the audit trail never leaks half-rendered debug output.
#[derive(Clone)]
pub struct GenkeyOutput {
    /// Decoded 48-byte layout. Exposed so callers can re-verify before
    /// printing.
    pub secret_bytes: [u8; SECRET_TOTAL_LEN],
    /// Base64-encoded `ztl_CAP_SECRET` — exactly what the operator
    /// pastes into their password manager.
    pub secret_b64: String,
    /// Base64-encoded Ed25519 private scalar.
    pub signing_key_b64: String,
    /// Base64-encoded Ed25519 public key — not a secret; safe to print
    /// in a follow-up line so operators can cross-check it against
    /// `recipients.toml::[vault].signing_pubkey` after a rebuild.
    pub signing_pubkey_b64: String,
}

impl Drop for GenkeyOutput {
    fn drop(&mut self) {
        // Best-effort zeroize. Strings are left alone — `String::zeroize`
        // only overwrites the current buffer and has no guarantee
        // against compiler elision; the operator is expected to store
        // `secret_b64` / `signing_key_b64` in their password manager
        // and drop the process immediately afterwards.
        self.secret_bytes.zeroize();
    }
}

/// Errors returned by [`decode_secret`]. Every variant carries enough
/// context that the build's `cap::build` entry point can surface a
/// remediation message without a secondary lookup.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SecretParseError {
    /// Base64 decode failed (bad character, bad padding, etc).
    #[error(
        "{env} does not parse as base64 ({detail}) — re-run `ztl cap genkey` and paste the output verbatim"
    )]
    Base64 { env: &'static str, detail: String },
    /// Decoded length mismatch — likely copy-truncation.
    #[error(
        "{env} decodes to {got} bytes but {expected} are required ({layout}) — re-run `ztl cap genkey`"
    )]
    Length {
        env: &'static str,
        got: usize,
        expected: usize,
        layout: &'static str,
    },
    /// Version byte is not one this binary understands.
    #[error(
        "{env} has version byte 0x{got:02x}; this binary only understands 0x{supported:02x} — upgrade ztl or re-run `ztl cap genkey`"
    )]
    Version {
        env: &'static str,
        got: u8,
        supported: u8,
    },
    /// Checksum mismatch — the secret was mangled between `genkey` and here.
    #[error(
        "{env} checksum mismatch — the secret appears corrupted (paste truncation, extra whitespace, or substitution); re-run `ztl cap genkey` and paste the output verbatim. See SPEC-034 REQ-3419 for the expected layout"
    )]
    Checksum { env: &'static str },
}

/// Decoded + checksum-verified view of `ztl_CAP_SECRET`.
#[derive(Clone)]
pub struct ParsedSecret {
    /// Raw 48-byte layout.
    pub bytes: [u8; SECRET_TOTAL_LEN],
}

impl std::fmt::Debug for ParsedSecret {
    /// Redacted `Debug`: callers that accidentally `{:?}` a decoded
    /// secret (in a test harness, a panic payload, a logging layer
    /// someone plumbed in later) see only the version byte and the
    /// total length. The 32-byte random body and 15-byte checksum
    /// never render. `Display` is intentionally not implemented.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParsedSecret")
            .field("version", &self.version())
            .field("len", &SECRET_TOTAL_LEN)
            .field("random_body", &"<redacted 32 bytes>")
            .field("checksum", &"<redacted 15 bytes>")
            .finish()
    }
}

impl ParsedSecret {
    /// Version byte (always [`SECRET_VERSION_V1`] for a successfully
    /// parsed secret in this binary).
    pub fn version(&self) -> u8 {
        self.bytes[0]
    }

    /// The 32-byte random body. HKDF consumers (`cap::derivation`) use
    /// this directly as IKM.
    pub fn random_body(&self) -> &[u8; SECRET_RANDOM_LEN] {
        // Safe: layout is pinned by SECRET_TOTAL_LEN.
        (&self.bytes[1..1 + SECRET_RANDOM_LEN])
            .try_into()
            .expect("layout pinned by SECRET_TOTAL_LEN")
    }

    /// The 15-byte checksum. Only useful for tests re-round-tripping.
    pub fn checksum(&self) -> &[u8; SECRET_CHECKSUM_LEN] {
        (&self.bytes[1 + SECRET_RANDOM_LEN..])
            .try_into()
            .expect("layout pinned by SECRET_TOTAL_LEN")
    }
}

impl Drop for ParsedSecret {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

/// Compute the 15-byte UX-safeguard checksum over `[version || random]`.
/// Exposed for tests and for the parser; effectful callers should use
/// [`generate_with_rng`] which wires this in.
pub fn compute_checksum(
    version: u8,
    random: &[u8; SECRET_RANDOM_LEN],
) -> [u8; SECRET_CHECKSUM_LEN] {
    let mut mac =
        HmacSha256::new_from_slice(CHECKSUM_HMAC_KEY).expect("HMAC-SHA256 accepts any key length");
    mac.update(&[version]);
    mac.update(random);
    let full = mac.finalize().into_bytes();
    let mut out = [0u8; SECRET_CHECKSUM_LEN];
    out.copy_from_slice(&full[..SECRET_CHECKSUM_LEN]);
    out
}

/// Pack a version + random body into the 48-byte wire layout, computing
/// the checksum on the way.
pub fn build_secret(version: u8, random: &[u8; SECRET_RANDOM_LEN]) -> [u8; SECRET_TOTAL_LEN] {
    let mut out = [0u8; SECRET_TOTAL_LEN];
    out[0] = version;
    out[1..1 + SECRET_RANDOM_LEN].copy_from_slice(random);
    let cksum = compute_checksum(version, random);
    out[1 + SECRET_RANDOM_LEN..].copy_from_slice(&cksum);
    out
}

/// Base64-encode a 48-byte decoded secret. STANDARD alphabet with
/// padding — the resulting 64-character string is a single token an
/// operator can select-and-copy from any terminal.
pub fn encode_secret(bytes: &[u8; SECRET_TOTAL_LEN]) -> String {
    STANDARD.encode(bytes)
}

/// Parse + checksum-verify an encoded `ztl_CAP_SECRET`. Whitespace
/// around the input is trimmed so callers can pass the raw env-var
/// value without an extra `.trim()` call. This is the single entry
/// point the build-time secret parser should use.
pub fn decode_secret(encoded: &str) -> Result<ParsedSecret, SecretParseError> {
    let trimmed = encoded.trim();
    let raw = STANDARD
        .decode(trimmed)
        .map_err(|e| SecretParseError::Base64 {
            env: ztl_CAP_SECRET_ENV,
            detail: e.to_string(),
        })?;
    if raw.len() != SECRET_TOTAL_LEN {
        return Err(SecretParseError::Length {
            env: ztl_CAP_SECRET_ENV,
            got: raw.len(),
            expected: SECRET_TOTAL_LEN,
            layout: "1-byte version || 32-byte random || 15-byte checksum",
        });
    }
    if raw[0] != SECRET_VERSION_V1 {
        return Err(SecretParseError::Version {
            env: ztl_CAP_SECRET_ENV,
            got: raw[0],
            supported: SECRET_VERSION_V1,
        });
    }

    let mut bytes = [0u8; SECRET_TOTAL_LEN];
    bytes.copy_from_slice(&raw);
    let mut random = [0u8; SECRET_RANDOM_LEN];
    random.copy_from_slice(&bytes[1..1 + SECRET_RANDOM_LEN]);
    let expected = compute_checksum(bytes[0], &random);
    // Constant-time compare even though the checksum isn't a security
    // control — removes a category of "early-abort" paste-confusion
    // heuristics an operator might build on top of the error surface.
    let mut diff: u8 = 0;
    for i in 0..SECRET_CHECKSUM_LEN {
        diff |= bytes[1 + SECRET_RANDOM_LEN + i] ^ expected[i];
    }
    if diff != 0 {
        return Err(SecretParseError::Checksum {
            env: ztl_CAP_SECRET_ENV,
        });
    }

    Ok(ParsedSecret { bytes })
}

/// Generate both secrets from the supplied CSPRNG. Pure function of
/// the RNG input — seeded CSPRNGs reproduce the output byte-for-byte,
/// which is what the tests exploit to pin checksum and Ed25519
/// derivation under a known vector.
pub fn generate_with_rng<R: RngCore + CryptoRng>(rng: &mut R) -> GenkeyOutput {
    let mut random = [0u8; SECRET_RANDOM_LEN];
    rng.fill_bytes(&mut random);

    let secret_bytes = build_secret(SECRET_VERSION_V1, &random);
    let secret_b64 = encode_secret(&secret_bytes);
    // Clear the temporary random buffer; it's already inside secret_bytes.
    random.zeroize();

    let signing_key = SigningKey::generate(rng);
    let signing_pubkey = signing_key.verifying_key();
    let signing_key_b64 = STANDARD.encode(signing_key.to_bytes());
    let signing_pubkey_b64 = STANDARD.encode(signing_pubkey.to_bytes());

    GenkeyOutput {
        secret_bytes,
        secret_b64,
        signing_key_b64,
        signing_pubkey_b64,
    }
}

/// Convenience wrapper that pulls from [`rand_core::OsRng`]. This is
/// the effectful-shell entry point — `ztl cap genkey` calls this once
/// per invocation.
pub fn generate() -> GenkeyOutput {
    generate_with_rng(&mut OsRng)
}

/// Render the full operator-facing banner. Emitted exactly once to
/// stdout by `ztl cap genkey`. Output is deterministic given the
/// inputs (no timestamps, no hostnames) so tests can pin it with a
/// byte-stable snapshot.
///
/// Format: two `export VAR=...` lines operators can paste into their
/// shell rc or a password-manager entry, followed by the public-key
/// fingerprint and a storage-guidance footer that points at SPEC-034.
pub fn render_human(out: &GenkeyOutput) -> String {
    let mut s = String::new();
    s.push_str("# ztl cap genkey — capability-mode secrets (SPEC-034 REQ-3419)\n");
    s.push_str("#\n");
    s.push_str("# Store BOTH values in a password manager (1Password, Bitwarden,\n");
    s.push_str("# macOS Keychain, pass, etc). They are printed to this terminal\n");
    s.push_str("# exactly once; ztl does not write them to any file and does not\n");
    s.push_str("# log them. If you lose them, you will need to rotate the cohort\n");
    s.push_str("# (see SPEC-034 §9 rotation workflow).\n");
    s.push_str("#\n");
    s.push_str("# The 15-byte checksum embedded in ztl_CAP_SECRET is a UX safeguard\n");
    s.push_str("# against paste-corruption — NOT a security control. An attacker who\n");
    s.push_str("# compromises the env var can produce a valid checksum. Protect the\n");
    s.push_str("# secret itself, not the checksum.\n");
    s.push('\n');
    s.push_str(&format!(
        "export {}='{}'\n",
        ztl_CAP_SECRET_ENV, out.secret_b64
    ));
    s.push_str(&format!(
        "export {}='{}'\n",
        ztl_CAP_SIGNING_KEY_ENV, out.signing_key_b64
    ));
    s.push('\n');
    s.push_str("# Vault-signing PUBLIC key (not a secret — embed in recipients.toml\n");
    s.push_str("# under [vault].signing_pubkey; the build will verify on first run):\n");
    s.push_str(&format!(
        "#   signing_pubkey = \"{}\"\n",
        out.signing_pubkey_b64
    ));
    s
}

/// JSON mirror of `render_human` for scripted callers (`--json`).
/// Keeps the same field names as the shell-export form so an automated
/// deploy can `jq -r .secret` without reinventing a parser.
pub fn render_json(out: &GenkeyOutput) -> serde_json::Value {
    serde_json::json!({
        "command": "ztl cap genkey",
        "spec": "SPEC-034 REQ-3419",
        "secret": {
            "env": ztl_CAP_SECRET_ENV,
            "value": out.secret_b64,
            "version": SECRET_VERSION_V1,
            "layout": "version:1 || random:32 || checksum:15 (HMAC-SHA256, truncated)",
            "checksum_framing": "UX safeguard (paste-corruption detection), not a security control",
        },
        "signing_key": {
            "env": ztl_CAP_SIGNING_KEY_ENV,
            "value": out.signing_key_b64,
            "algorithm": "Ed25519 (RFC 8032)",
            "public_key": out.signing_pubkey_b64,
        },
        "storage": {
            "instructions": "Store both values in a password manager; ztl prints them once and never writes them to disk.",
            "rotation_workflow": "SPEC-034 §9",
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    fn seeded_rng() -> ChaCha20Rng {
        // Deterministic seed; changing it is a test-data break only.
        ChaCha20Rng::from_seed([7u8; 32])
    }

    #[test]
    fn generate_produces_48_byte_secret_and_32_byte_signing_key() {
        let out = generate_with_rng(&mut seeded_rng());
        assert_eq!(out.secret_bytes.len(), SECRET_TOTAL_LEN);
        assert_eq!(out.secret_bytes[0], SECRET_VERSION_V1);

        // Decoded signing key is 32 bytes.
        let decoded = STANDARD.decode(&out.signing_key_b64).unwrap();
        assert_eq!(decoded.len(), SIGNING_KEY_LEN);
        let decoded_pub = STANDARD.decode(&out.signing_pubkey_b64).unwrap();
        assert_eq!(decoded_pub.len(), 32);
    }

    #[test]
    fn roundtrip_encode_decode_succeeds() {
        let out = generate_with_rng(&mut seeded_rng());
        let parsed = decode_secret(&out.secret_b64).expect("valid secret parses");
        assert_eq!(parsed.version(), SECRET_VERSION_V1);
        assert_eq!(parsed.bytes, out.secret_bytes);
    }

    #[test]
    fn roundtrip_tolerates_surrounding_whitespace() {
        let out = generate_with_rng(&mut seeded_rng());
        let padded = format!("  \n\t{}\n ", out.secret_b64);
        decode_secret(&padded).expect("whitespace is trimmed");
    }

    #[test]
    fn checksum_tampering_is_detected() {
        let out = generate_with_rng(&mut seeded_rng());
        let mut tampered = out.secret_bytes;
        // Flip a bit in the checksum region (last 15 bytes).
        let idx = SECRET_TOTAL_LEN - 1;
        tampered[idx] ^= 0x01;
        let encoded = encode_secret(&tampered);
        let err = decode_secret(&encoded).expect_err("tampered checksum must not parse");
        assert!(
            matches!(err, SecretParseError::Checksum { .. }),
            "expected Checksum variant, got {err:?}",
        );
        let rendered = format!("{err}").to_lowercase();
        assert!(
            rendered.contains("re-run `ztl cap genkey`"),
            "remediation text missing from error: {rendered}",
        );
    }

    #[test]
    fn random_body_tampering_invalidates_checksum() {
        let out = generate_with_rng(&mut seeded_rng());
        let mut tampered = out.secret_bytes;
        tampered[5] ^= 0xff;
        let encoded = encode_secret(&tampered);
        let err = decode_secret(&encoded).expect_err("random-body edit invalidates checksum");
        assert!(matches!(err, SecretParseError::Checksum { .. }));
    }

    #[test]
    fn wrong_length_is_rejected() {
        let short = STANDARD.encode([0u8; SECRET_TOTAL_LEN - 1]);
        let err = decode_secret(&short).expect_err("short secret must not parse");
        match err {
            SecretParseError::Length { got, expected, .. } => {
                assert_eq!(got, SECRET_TOTAL_LEN - 1);
                assert_eq!(expected, SECRET_TOTAL_LEN);
            }
            other => panic!("expected Length variant, got {other:?}"),
        }
    }

    #[test]
    fn wrong_version_is_rejected() {
        let mut bad = [0u8; SECRET_TOTAL_LEN];
        bad[0] = 0xff;
        // Checksum is wrong too, but the parser checks version before
        // checksum — this test pins that ordering.
        let encoded = STANDARD.encode(bad);
        let err = decode_secret(&encoded).expect_err("bad version must not parse");
        assert!(matches!(err, SecretParseError::Version { .. }));
    }

    #[test]
    fn bad_base64_is_rejected_with_hint() {
        let err = decode_secret("!!!not-base64!!!").expect_err("non-base64 must not parse");
        assert!(matches!(err, SecretParseError::Base64 { .. }));
        let rendered = format!("{err}");
        assert!(rendered.contains("re-run `ztl cap genkey`"));
    }

    #[test]
    fn two_calls_produce_distinct_secrets() {
        // Under OsRng (real CSPRNG), two back-to-back generations must
        // never collide. This catches the failure mode of accidentally
        // wiring a constant seed into the production path.
        let a = generate();
        let b = generate();
        assert_ne!(a.secret_b64, b.secret_b64);
        assert_ne!(a.signing_key_b64, b.signing_key_b64);
    }

    #[test]
    fn seeded_generation_is_reproducible() {
        let a = generate_with_rng(&mut seeded_rng());
        let b = generate_with_rng(&mut seeded_rng());
        assert_eq!(a.secret_b64, b.secret_b64);
        assert_eq!(a.signing_key_b64, b.signing_key_b64);
        assert_eq!(a.signing_pubkey_b64, b.signing_pubkey_b64);
    }

    #[test]
    fn render_human_includes_both_env_vars_and_safeguard_framing() {
        let out = generate_with_rng(&mut seeded_rng());
        let text = render_human(&out);
        assert!(text.contains(&format!("export {ztl_CAP_SECRET_ENV}=")));
        assert!(text.contains(&format!("export {ztl_CAP_SIGNING_KEY_ENV}=")));
        assert!(text.contains(&out.secret_b64));
        assert!(text.contains(&out.signing_key_b64));
        assert!(text.contains(&out.signing_pubkey_b64));
        // BUG-017 framing: checksum must be described as a UX safeguard,
        // not a security control.
        assert!(
            text.to_lowercase().contains("ux safeguard"),
            "render_human is missing the BUG-017 safeguard framing:\n{text}",
        );
        assert!(
            text.to_lowercase().contains("not a security control")
                || text.to_lowercase().contains("not— a security control"),
            "render_human should explicitly state the checksum is not a security control:\n{text}",
        );
    }

    #[test]
    fn render_json_is_parseable_and_mirrors_fields() {
        let out = generate_with_rng(&mut seeded_rng());
        let j = render_json(&out);
        assert_eq!(j["command"], "ztl cap genkey");
        assert_eq!(j["secret"]["env"], ztl_CAP_SECRET_ENV);
        assert_eq!(j["secret"]["value"], out.secret_b64);
        assert_eq!(j["signing_key"]["env"], ztl_CAP_SIGNING_KEY_ENV);
        assert_eq!(j["signing_key"]["value"], out.signing_key_b64);
        assert_eq!(j["signing_key"]["public_key"], out.signing_pubkey_b64);
    }

    #[test]
    fn parsed_secret_random_body_is_ikm_length() {
        // The downstream HKDF derivation in `cap::derivation` consumes
        // `ztl_CAP_SECRET` as IKM. Pin the slice length so a refactor
        // that accidentally narrows the random body breaks this test.
        let out = generate_with_rng(&mut seeded_rng());
        let parsed = decode_secret(&out.secret_b64).unwrap();
        assert_eq!(parsed.random_body().len(), SECRET_RANDOM_LEN);
    }
}
