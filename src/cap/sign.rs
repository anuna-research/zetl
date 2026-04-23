//! Ed25519 content signing layer (SPEC-034 REQ-3427 / CON-3411 /
//! ADR-3412 — resolves BUG-004).
//!
//! Composes the per-page wire format:
//!
//! ```text
//! ztl-Schema: v4\n
//! ztl-Cohort-Id: <id>\n
//! ztl-Cohort-Mode: delegated-url|webauthn-prf\n
//! ztl-Slug: <slug>\n
//! ztl-Build-Epoch: <rfc3339>\n
//! ztl-Signature: <b64url-ed25519-sig>\n
//! \n
//! <age v1 ciphertext bytes>
//! ```
//!
//! Per CON-3404 the Ed25519 signature covers *only* the age ciphertext
//! bytes. Envelope headers exist for client-side dispatch and are
//! intentionally unauthenticated; tampering with headers invalidates
//! no signature but also cannot substitute ciphertext. The shim's
//! render pipeline is **verify → decrypt → sanitise → render**
//! (CON-3408 §STEP 1).
//!
//! # Purity boundary (SPEC-034 §8)
//!
//! - **Pure core:** [`EnvelopeHeader`] construction, header
//!   serialisation, signing ([`sign_ciphertext`]), verification
//!   ([`verify_ciphertext`]), envelope assembly ([`build_envelope`]),
//!   envelope parsing ([`parse_envelope`]), and key-material parsing
//!   (`parse_signing_key_*` / `parse_signing_pubkey_*`). Ed25519 itself
//!   is deterministic (RFC 8032 §5.1.6) — identical inputs produce
//!   identical signatures, which is what makes REQ-3420 determinism
//!   survive across the signing step.
//! - **Effectful shell:** [`load_signing_key_from_env`] reads
//!   [`ztl_CAP_SIGNING_KEY_ENV`] and is the *only* surface that
//!   touches process state. The signing-key material NEVER touches
//!   disk (SPEC-034 §8 "Signing-key material NEVER crosses into shim
//!   JS — only the public key does").
//!
//! # Zeroisation
//!
//! [`VaultSigningKey`] wraps [`ed25519_dalek::SigningKey`], which
//! implements [`zeroize::ZeroizeOnDrop`] under the default feature
//! set. The wrapper deliberately withholds `Debug`/`Display`/`Clone`
//! so a stray `{:?}` or accidental copy cannot leak the private
//! scalar into a log line or a stack frame.

use std::env;

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine as _;
use ed25519_dalek::{Signature, SigningKey, VerifyingKey};

use crate::cap::genkey::{SIGNING_KEY_LEN, ztl_CAP_SIGNING_KEY_ENV};
use crate::cap::recipients::parsing::{CohortMode, ED25519_PUBKEY_PREFIX};

/// Envelope schema tag pinned by CON-3404. Any future wire-format
/// change MUST bump this so older shims refuse to interpret newer
/// envelopes rather than misread the bytes.
pub const ENVELOPE_SCHEMA: &str = "v4";

/// Length of an Ed25519 public key (RFC 8032). Exposed so callers
/// can size buffers without pulling in `ed25519-dalek` transitively.
pub const SIGNING_PUBKEY_LEN: usize = 32;

/// Length of an Ed25519 signature (RFC 8032).
pub const SIGNATURE_LEN: usize = 64;

/// Header field names. Kept as named constants so the byte-stable
/// integration tests and the shim JS stay in lock-step.
pub const HEADER_SCHEMA: &str = "ztl-Schema";
pub const HEADER_COHORT_ID: &str = "ztl-Cohort-Id";
pub const HEADER_COHORT_MODE: &str = "ztl-Cohort-Mode";
pub const HEADER_SLUG: &str = "ztl-Slug";
pub const HEADER_BUILD_EPOCH: &str = "ztl-Build-Epoch";
pub const HEADER_SIGNATURE: &str = "ztl-Signature";

/// Plaintext envelope header fields (CON-3404). Deterministic given
/// inputs; feeding the same values to [`build_envelope`] always
/// produces byte-identical header bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvelopeHeader {
    /// Cohort identifier (matches `[[cohort]].id` in recipients.toml).
    pub cohort_id: String,
    /// Cohort auth mode (delegated-url or webauthn-prf).
    pub cohort_mode: CohortMode,
    /// Page slug under the path-cap directory.
    pub slug: String,
    /// RFC 3339 timestamp when this envelope was built. Supplied by
    /// the build driver so this module stays free of wall-clock I/O.
    pub build_epoch: String,
}

/// Wrapper around an [`ed25519_dalek::SigningKey`]. Deliberately
/// withholds `Debug`/`Clone` so a stray log line cannot render the
/// private scalar; the inner key is dropped via
/// `SigningKey::ZeroizeOnDrop`.
pub struct VaultSigningKey {
    inner: SigningKey,
}

impl VaultSigningKey {
    /// Construct from raw 32-byte private scalar. Infallible — any
    /// 32-byte value is a valid Ed25519 seed.
    pub fn from_bytes(bytes: &[u8; SIGNING_KEY_LEN]) -> Self {
        Self {
            inner: SigningKey::from_bytes(bytes),
        }
    }

    /// Derived public key. Cheap (a scalar multiplication per call,
    /// already cached inside `SigningKey`).
    pub fn verifying_key(&self) -> VaultSigningPubkey {
        VaultSigningPubkey {
            inner: self.inner.verifying_key(),
        }
    }

    /// Sign the exact byte range of an age ciphertext. Per CON-3404
    /// the caller MUST hand in the raw age v1 ciphertext bytes; the
    /// envelope header is NOT signed.
    pub fn sign_ciphertext(&self, ciphertext: &[u8]) -> [u8; SIGNATURE_LEN] {
        // ed25519-dalek's `Signer::sign` is infallible and constant-
        // time (RFC 8032 §5.1.6). `Signature::to_bytes` returns the
        // 64-byte R||S layout.
        use ed25519_dalek::Signer;
        self.inner.sign(ciphertext).to_bytes()
    }
}

/// Wrapper around an [`ed25519_dalek::VerifyingKey`]. Always safe to
/// expose via `Debug`/`Clone` (a public key is not a secret).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultSigningPubkey {
    inner: VerifyingKey,
}

impl VaultSigningPubkey {
    /// Raw 32-byte public key. The build driver embeds these bytes
    /// into `/assets/shim.js` at a documented offset (REQ-3427) so
    /// the shim's SHA-384 SRI hash covers the pubkey.
    pub fn to_bytes(&self) -> [u8; SIGNING_PUBKEY_LEN] {
        self.inner.to_bytes()
    }

    /// Base64url-encoded (unpadded) public key. Matches the
    /// CON-3403 recipients.toml `signing_pubkey = "ed25519:<b64url>"`
    /// payload.
    pub fn to_b64url(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.to_bytes())
    }

    /// Standard-alphabet base64 (padded). Matches the output of
    /// `ztl cap genkey` so operators can cross-check the pubkey
    /// shown in the genkey banner against `recipients.toml`.
    pub fn to_b64_standard(&self) -> String {
        STANDARD.encode(self.to_bytes())
    }

    /// Verify a signature against the given ciphertext. `verify_strict`
    /// rejects non-canonical R/S encodings (RFC 8032 §8.4); the
    /// capability-mode shim uses `Ed25519.verify` (noble-ed25519)
    /// which enforces the same canonical check.
    pub fn verify_ciphertext(
        &self,
        ciphertext: &[u8],
        signature: &[u8; SIGNATURE_LEN],
    ) -> Result<(), VerifyError> {
        let sig = Signature::from_bytes(signature);
        self.inner
            .verify_strict(ciphertext, &sig)
            .map_err(|e| VerifyError::Invalid(e.to_string()))
    }
}

/// Errors loading a signing key from env or raw bytes.
#[derive(Debug, thiserror::Error)]
pub enum KeyLoadError {
    #[error(
        "{env} is not set — run `ztl cap genkey` to generate one, then export it in the build shell"
    )]
    EnvMissing { env: &'static str },
    #[error("{env} is set but empty — re-run `ztl cap genkey`")]
    EnvEmpty { env: &'static str },
    #[error(
        "{env} does not parse as base64 ({detail}) — re-run `ztl cap genkey` and paste the output verbatim"
    )]
    Base64 { env: &'static str, detail: String },
    #[error("{env} decodes to {got} bytes but {expected} are required — re-run `ztl cap genkey`")]
    Length {
        env: &'static str,
        got: usize,
        expected: usize,
    },
    #[error("{env} is missing the `{prefix}` prefix")]
    MissingPrefix {
        env: &'static str,
        prefix: &'static str,
    },
}

/// Errors verifying a signature.
#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    #[error("Ed25519 signature verification failed: {0}")]
    Invalid(String),
}

/// Errors parsing an envelope emitted by [`build_envelope`].
#[derive(Debug, thiserror::Error)]
pub enum EnvelopeParseError {
    #[error("envelope is missing the blank-line separator between headers and ciphertext")]
    MissingSeparator,
    #[error("envelope header is not valid UTF-8: {0}")]
    HeaderEncoding(String),
    #[error("envelope header line {0:?} is missing a `: ` delimiter")]
    MalformedHeaderLine(String),
    #[error("required header {0:?} is missing")]
    MissingHeader(&'static str),
    #[error(
        "header `{header}` has value {got:?} but schema requires {expected:?} — this shim is too old; rebuild with a matching ztl binary"
    )]
    SchemaMismatch {
        header: &'static str,
        got: String,
        expected: &'static str,
    },
    #[error("header `{HEADER_COHORT_MODE}` has value {0:?} (not `delegated-url`/`webauthn-prf`)")]
    UnknownCohortMode(String),
    #[error("header `{HEADER_SIGNATURE}` does not parse as base64url: {0}")]
    SignatureBase64(String),
    #[error(
        "header `{HEADER_SIGNATURE}` decodes to {got} bytes but {expected} are required (Ed25519 signatures are 64 bytes)"
    )]
    SignatureLength { got: usize, expected: usize },
}

/// Parse a base64-encoded Ed25519 private scalar. Accepts the
/// STANDARD alphabet (what `ztl cap genkey` emits) with or without
/// padding.
pub fn parse_signing_key_b64(env: &'static str, s: &str) -> Result<VaultSigningKey, KeyLoadError> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err(KeyLoadError::EnvEmpty { env });
    }
    let raw = STANDARD.decode(trimmed).map_err(|e| KeyLoadError::Base64 {
        env,
        detail: e.to_string(),
    })?;
    if raw.len() != SIGNING_KEY_LEN {
        return Err(KeyLoadError::Length {
            env,
            got: raw.len(),
            expected: SIGNING_KEY_LEN,
        });
    }
    let mut bytes = [0u8; SIGNING_KEY_LEN];
    bytes.copy_from_slice(&raw);
    Ok(VaultSigningKey::from_bytes(&bytes))
}

/// Parse a base64-encoded Ed25519 public key. The CON-3403
/// recipients.toml form is `ed25519:<b64url>`; pass the raw payload
/// (without prefix) — see [`parse_signing_pubkey_recipients_toml`]
/// for the prefixed form.
///
/// Accepts either the URL_SAFE_NO_PAD alphabet (the spec's canonical
/// form) or the STANDARD alphabet (what `ztl cap genkey` prints
/// next to the shell-export block). Operators who copy either form
/// into `recipients.toml` should not be greeted with a parser
/// surprise.
pub fn parse_signing_pubkey_b64(s: &str) -> Result<VaultSigningPubkey, KeyLoadError> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err(KeyLoadError::EnvEmpty {
            env: "signing_pubkey",
        });
    }
    let raw = URL_SAFE_NO_PAD
        .decode(trimmed)
        .or_else(|_| STANDARD.decode(trimmed))
        .map_err(|e| KeyLoadError::Base64 {
            env: "signing_pubkey",
            detail: e.to_string(),
        })?;
    if raw.len() != SIGNING_PUBKEY_LEN {
        return Err(KeyLoadError::Length {
            env: "signing_pubkey",
            got: raw.len(),
            expected: SIGNING_PUBKEY_LEN,
        });
    }
    let mut bytes = [0u8; SIGNING_PUBKEY_LEN];
    bytes.copy_from_slice(&raw);
    let vk = VerifyingKey::from_bytes(&bytes).map_err(|e| KeyLoadError::Base64 {
        env: "signing_pubkey",
        detail: format!("not a valid Ed25519 point: {e}"),
    })?;
    Ok(VaultSigningPubkey { inner: vk })
}

/// Parse the full `recipients.toml` `ed25519:<b64>` form.
pub fn parse_signing_pubkey_recipients_toml(s: &str) -> Result<VaultSigningPubkey, KeyLoadError> {
    let payload = s
        .strip_prefix(ED25519_PUBKEY_PREFIX)
        .ok_or(KeyLoadError::MissingPrefix {
            env: "signing_pubkey",
            prefix: ED25519_PUBKEY_PREFIX,
        })?;
    parse_signing_pubkey_b64(payload)
}

/// Effectful-shell entry point: read [`ztl_CAP_SIGNING_KEY_ENV`]
/// from the process environment and parse it. The only function in
/// this module that touches `std::env`. SPEC-034 §8 forbids the
/// private key from *ever* touching disk — this is the single
/// authorised channel.
pub fn load_signing_key_from_env() -> Result<VaultSigningKey, KeyLoadError> {
    let raw = env::var(ztl_CAP_SIGNING_KEY_ENV).map_err(|_| KeyLoadError::EnvMissing {
        env: ztl_CAP_SIGNING_KEY_ENV,
    })?;
    parse_signing_key_b64(ztl_CAP_SIGNING_KEY_ENV, &raw)
}

/// Ed25519-sign the given ciphertext with the vault-signing key.
/// Pure function of its inputs (ed25519-dalek uses deterministic
/// signatures per RFC 8032 §5.1.6 — identical inputs produce
/// byte-identical signatures).
pub fn sign_ciphertext(key: &VaultSigningKey, ciphertext: &[u8]) -> [u8; SIGNATURE_LEN] {
    key.sign_ciphertext(ciphertext)
}

/// Verify a signature against a ciphertext using the vault-signing
/// public key. Rejects non-canonical R/S encodings (REQ-3427 parity
/// with the shim's noble-ed25519 verifier).
pub fn verify_ciphertext(
    pubkey: &VaultSigningPubkey,
    ciphertext: &[u8],
    signature: &[u8; SIGNATURE_LEN],
) -> Result<(), VerifyError> {
    pubkey.verify_ciphertext(ciphertext, signature)
}

/// Serialise an [`EnvelopeHeader`] + signature + ciphertext into the
/// CON-3404 wire bytes. Pure function of its inputs.
///
/// The resulting byte layout is:
///
/// ```text
/// ztl-Schema: v4\n
/// ztl-Cohort-Id: <id>\n
/// ztl-Cohort-Mode: delegated-url|webauthn-prf\n
/// ztl-Slug: <slug>\n
/// ztl-Build-Epoch: <rfc3339>\n
/// ztl-Signature: <b64url>\n
/// \n
/// <ciphertext bytes verbatim>
/// ```
pub fn build_envelope(
    header: &EnvelopeHeader,
    signature: &[u8; SIGNATURE_LEN],
    ciphertext: &[u8],
) -> Vec<u8> {
    let sig_b64 = URL_SAFE_NO_PAD.encode(signature);
    let header_text = render_header(header, &sig_b64);
    let mut out = Vec::with_capacity(header_text.len() + 1 + ciphertext.len());
    out.extend_from_slice(header_text.as_bytes());
    out.push(b'\n');
    out.extend_from_slice(ciphertext);
    out
}

/// Convenience: sign the ciphertext with `key` and assemble the
/// envelope in one call.
pub fn sign_and_build_envelope(
    key: &VaultSigningKey,
    header: &EnvelopeHeader,
    ciphertext: &[u8],
) -> Vec<u8> {
    let sig = sign_ciphertext(key, ciphertext);
    build_envelope(header, &sig, ciphertext)
}

fn render_header(header: &EnvelopeHeader, sig_b64: &str) -> String {
    let mode = match header.cohort_mode {
        CohortMode::DelegatedUrl => "delegated-url",
        CohortMode::WebauthnPrf => "webauthn-prf",
    };
    let mut s = String::with_capacity(256);
    s.push_str(HEADER_SCHEMA);
    s.push_str(": ");
    s.push_str(ENVELOPE_SCHEMA);
    s.push('\n');
    s.push_str(HEADER_COHORT_ID);
    s.push_str(": ");
    s.push_str(&header.cohort_id);
    s.push('\n');
    s.push_str(HEADER_COHORT_MODE);
    s.push_str(": ");
    s.push_str(mode);
    s.push('\n');
    s.push_str(HEADER_SLUG);
    s.push_str(": ");
    s.push_str(&header.slug);
    s.push('\n');
    s.push_str(HEADER_BUILD_EPOCH);
    s.push_str(": ");
    s.push_str(&header.build_epoch);
    s.push('\n');
    s.push_str(HEADER_SIGNATURE);
    s.push_str(": ");
    s.push_str(sig_b64);
    s.push('\n');
    s
}

/// Parsed view of an envelope emitted by [`build_envelope`]. Exposed
/// so tests + future shim-parity audits can walk the byte range the
/// signature covers without re-implementing the split.
#[derive(Debug, Clone)]
pub struct ParsedEnvelope {
    pub header: EnvelopeHeader,
    pub signature: [u8; SIGNATURE_LEN],
    pub ciphertext: Vec<u8>,
}

/// Split a wire envelope back into `(header, signature, ciphertext)`.
/// Pure; the inverse of [`build_envelope`] up to header-ordering
/// (the parser accepts any order, the builder pins CON-3404 order).
pub fn parse_envelope(bytes: &[u8]) -> Result<ParsedEnvelope, EnvelopeParseError> {
    // Find the `\n\n` separator between headers and ciphertext.
    let sep = bytes
        .windows(2)
        .position(|w| w == b"\n\n")
        .ok_or(EnvelopeParseError::MissingSeparator)?;
    let header_bytes = &bytes[..sep];
    let ciphertext = bytes[sep + 2..].to_vec();

    let header_text = std::str::from_utf8(header_bytes)
        .map_err(|e| EnvelopeParseError::HeaderEncoding(e.to_string()))?;

    let mut schema: Option<&str> = None;
    let mut cohort_id: Option<&str> = None;
    let mut cohort_mode: Option<&str> = None;
    let mut slug: Option<&str> = None;
    let mut build_epoch: Option<&str> = None;
    let mut signature_b64: Option<&str> = None;

    for line in header_text.split('\n') {
        if line.is_empty() {
            continue;
        }
        let (name, value) = line
            .split_once(": ")
            .ok_or_else(|| EnvelopeParseError::MalformedHeaderLine(line.to_string()))?;
        match name {
            HEADER_SCHEMA => schema = Some(value),
            HEADER_COHORT_ID => cohort_id = Some(value),
            HEADER_COHORT_MODE => cohort_mode = Some(value),
            HEADER_SLUG => slug = Some(value),
            HEADER_BUILD_EPOCH => build_epoch = Some(value),
            HEADER_SIGNATURE => signature_b64 = Some(value),
            _ => {
                // Forward-compat: unknown headers are tolerated so a
                // newer builder adding a hint-header does not break
                // older verifiers. A wire-break SHALL bump the
                // schema tag (ENVELOPE_SCHEMA) so the SchemaMismatch
                // path fires first.
            }
        }
    }

    let schema_val = schema.ok_or(EnvelopeParseError::MissingHeader(HEADER_SCHEMA))?;
    if schema_val != ENVELOPE_SCHEMA {
        return Err(EnvelopeParseError::SchemaMismatch {
            header: HEADER_SCHEMA,
            got: schema_val.to_string(),
            expected: ENVELOPE_SCHEMA,
        });
    }
    let cohort_id = cohort_id
        .ok_or(EnvelopeParseError::MissingHeader(HEADER_COHORT_ID))?
        .to_string();
    let mode_raw = cohort_mode.ok_or(EnvelopeParseError::MissingHeader(HEADER_COHORT_MODE))?;
    let cohort_mode = match mode_raw {
        "delegated-url" => CohortMode::DelegatedUrl,
        "webauthn-prf" => CohortMode::WebauthnPrf,
        other => return Err(EnvelopeParseError::UnknownCohortMode(other.to_string())),
    };
    let slug = slug
        .ok_or(EnvelopeParseError::MissingHeader(HEADER_SLUG))?
        .to_string();
    let build_epoch = build_epoch
        .ok_or(EnvelopeParseError::MissingHeader(HEADER_BUILD_EPOCH))?
        .to_string();
    let sig_b64 = signature_b64.ok_or(EnvelopeParseError::MissingHeader(HEADER_SIGNATURE))?;

    let sig_raw = URL_SAFE_NO_PAD
        .decode(sig_b64)
        .map_err(|e| EnvelopeParseError::SignatureBase64(e.to_string()))?;
    if sig_raw.len() != SIGNATURE_LEN {
        return Err(EnvelopeParseError::SignatureLength {
            got: sig_raw.len(),
            expected: SIGNATURE_LEN,
        });
    }
    let mut signature = [0u8; SIGNATURE_LEN];
    signature.copy_from_slice(&sig_raw);

    Ok(ParsedEnvelope {
        header: EnvelopeHeader {
            cohort_id,
            cohort_mode,
            slug,
            build_epoch,
        },
        signature,
        ciphertext,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_key() -> VaultSigningKey {
        // Deterministic 32-byte seed; any bytes are a valid Ed25519
        // seed so we don't need a genkey round-trip here.
        VaultSigningKey::from_bytes(&[7u8; SIGNING_KEY_LEN])
    }

    fn sample_header() -> EnvelopeHeader {
        EnvelopeHeader {
            cohort_id: "engineering".to_string(),
            cohort_mode: CohortMode::DelegatedUrl,
            slug: "onboarding".to_string(),
            build_epoch: "2026-04-20T10:15:00Z".to_string(),
        }
    }

    #[test]
    fn sign_and_verify_round_trip() {
        let key = sample_key();
        let pub_ = key.verifying_key();
        let ciphertext = b"age-v1-ciphertext-bytes";
        let sig = sign_ciphertext(&key, ciphertext);
        verify_ciphertext(&pub_, ciphertext, &sig).expect("signature verifies");
    }

    #[test]
    fn signatures_are_deterministic() {
        // RFC 8032 Ed25519 is deterministic: identical (key, message)
        // always produce identical signatures. REQ-3420 relies on
        // this property for reproducible builds.
        let key = sample_key();
        let ciphertext = b"determinism-probe";
        let sig_a = sign_ciphertext(&key, ciphertext);
        let sig_b = sign_ciphertext(&key, ciphertext);
        assert_eq!(sig_a, sig_b);
    }

    #[test]
    fn verify_rejects_tampered_signature() {
        let key = sample_key();
        let pub_ = key.verifying_key();
        let ciphertext = b"age-ciphertext";
        let mut sig = sign_ciphertext(&key, ciphertext);
        sig[0] ^= 0x01;
        assert!(matches!(
            verify_ciphertext(&pub_, ciphertext, &sig),
            Err(VerifyError::Invalid(_))
        ));
    }

    #[test]
    fn verify_rejects_tampered_ciphertext() {
        let key = sample_key();
        let pub_ = key.verifying_key();
        let ciphertext = b"age-ciphertext";
        let sig = sign_ciphertext(&key, ciphertext);
        let mut tampered = ciphertext.to_vec();
        tampered[0] ^= 0x01;
        assert!(matches!(
            verify_ciphertext(&pub_, &tampered, &sig),
            Err(VerifyError::Invalid(_))
        ));
    }

    #[test]
    fn verify_rejects_wrong_pubkey() {
        let key_a = VaultSigningKey::from_bytes(&[1u8; SIGNING_KEY_LEN]);
        let key_b = VaultSigningKey::from_bytes(&[2u8; SIGNING_KEY_LEN]);
        let pub_b = key_b.verifying_key();
        let ciphertext = b"body";
        let sig_a = sign_ciphertext(&key_a, ciphertext);
        assert!(matches!(
            verify_ciphertext(&pub_b, ciphertext, &sig_a),
            Err(VerifyError::Invalid(_))
        ));
    }

    #[test]
    fn envelope_round_trip() {
        let key = sample_key();
        let header = sample_header();
        let ciphertext = b"age-v1-ciphertext-payload";
        let bytes = sign_and_build_envelope(&key, &header, ciphertext);
        let parsed = parse_envelope(&bytes).expect("parse");
        assert_eq!(parsed.header, header);
        assert_eq!(&parsed.ciphertext[..], ciphertext);
        verify_ciphertext(&key.verifying_key(), &parsed.ciphertext, &parsed.signature)
            .expect("round-trip signature verifies");
    }

    #[test]
    fn envelope_is_byte_stable_under_fixed_inputs() {
        // CON-3411 + REQ-3420: for a fixed key, header, and ciphertext,
        // the envelope bytes must be identical across runs. Ed25519 is
        // deterministic; the builder adds no randomness of its own.
        let key = sample_key();
        let header = sample_header();
        let ciphertext = b"fixed-ciphertext";
        let a = sign_and_build_envelope(&key, &header, ciphertext);
        let b = sign_and_build_envelope(&key, &header, ciphertext);
        assert_eq!(a, b);
    }

    #[test]
    fn header_order_matches_con_3404() {
        // Wire format pins the order: Schema, Cohort-Id, Cohort-Mode,
        // Slug, Build-Epoch, Signature.
        let key = sample_key();
        let header = sample_header();
        let bytes = sign_and_build_envelope(&key, &header, b"x");
        let sep = bytes
            .windows(2)
            .position(|w| w == b"\n\n")
            .expect("separator");
        let text = std::str::from_utf8(&bytes[..sep]).expect("header is ascii");
        let schema_pos = text.find(HEADER_SCHEMA).expect("schema present");
        let cohort_id_pos = text.find(HEADER_COHORT_ID).expect("cohort id present");
        let cohort_mode_pos = text.find(HEADER_COHORT_MODE).expect("cohort mode present");
        let slug_pos = text.find(HEADER_SLUG).expect("slug present");
        let build_epoch_pos = text.find(HEADER_BUILD_EPOCH).expect("build epoch present");
        let signature_pos = text.find(HEADER_SIGNATURE).expect("signature present");
        assert!(schema_pos < cohort_id_pos);
        assert!(cohort_id_pos < cohort_mode_pos);
        assert!(cohort_mode_pos < slug_pos);
        assert!(slug_pos < build_epoch_pos);
        assert!(build_epoch_pos < signature_pos);
    }

    #[test]
    fn header_tampering_does_not_invalidate_signature() {
        // CON-3404: "Changing any envelope header invalidates NO
        // signature; changing ciphertext bytes invalidates the
        // signature." Headers exist for dispatch, not authentication.
        let key = sample_key();
        let header = sample_header();
        let ciphertext = b"real-ciphertext";
        let bytes = sign_and_build_envelope(&key, &header, ciphertext);

        // Locate the cohort-id value and overwrite it in place.
        // Both values are 11 chars so the envelope byte length is
        // preserved and the `\n\n` separator stays at its original
        // offset.
        let needle = format!("{HEADER_COHORT_ID}: engineering");
        let replacement = format!("{HEADER_COHORT_ID}: attackerxxx");
        assert_eq!(
            needle.len(),
            replacement.len(),
            "len must match for in-place edit"
        );
        let at = bytes
            .windows(needle.len())
            .position(|w| w == needle.as_bytes())
            .expect("cohort-id header present");
        let mut tampered = bytes.clone();
        tampered[at..at + needle.len()].copy_from_slice(replacement.as_bytes());

        let parsed = parse_envelope(&tampered).expect("parses — headers unauthenticated");
        assert_eq!(parsed.header.cohort_id, "attackerxxx");
        // Signature still verifies over the original ciphertext bytes.
        verify_ciphertext(&key.verifying_key(), &parsed.ciphertext, &parsed.signature)
            .expect("header tamper leaves signature intact");
    }

    #[test]
    fn ciphertext_tampering_invalidates_envelope_signature() {
        let key = sample_key();
        let header = sample_header();
        let ciphertext = b"hello-age-ciphertext-bytes";
        let mut bytes = sign_and_build_envelope(&key, &header, ciphertext);

        // Flip a bit in the ciphertext portion only (after the
        // `\n\n` separator).
        let sep = bytes
            .windows(2)
            .position(|w| w == b"\n\n")
            .expect("separator");
        let target = sep + 2;
        bytes[target] ^= 0x01;

        let parsed = parse_envelope(&bytes).expect("parse");
        assert!(matches!(
            verify_ciphertext(&key.verifying_key(), &parsed.ciphertext, &parsed.signature),
            Err(VerifyError::Invalid(_))
        ));
    }

    #[test]
    fn schema_mismatch_is_rejected() {
        let mut bytes = b"ztl-Schema: v99\n".to_vec();
        bytes.extend_from_slice(b"ztl-Cohort-Id: eng\n");
        bytes.extend_from_slice(b"ztl-Cohort-Mode: delegated-url\n");
        bytes.extend_from_slice(b"ztl-Slug: s\n");
        bytes.extend_from_slice(b"ztl-Build-Epoch: 2026-04-20T00:00:00Z\n");
        bytes.extend_from_slice(b"ztl-Signature: AAAA\n");
        bytes.extend_from_slice(b"\nbody");
        let err = parse_envelope(&bytes).expect_err("schema v99 is rejected");
        assert!(matches!(err, EnvelopeParseError::SchemaMismatch { .. }));
    }

    #[test]
    fn missing_separator_is_rejected() {
        let bytes = b"ztl-Schema: v4\nztl-Cohort-Id: x\n".to_vec();
        assert!(matches!(
            parse_envelope(&bytes),
            Err(EnvelopeParseError::MissingSeparator)
        ));
    }

    #[test]
    fn unknown_headers_are_tolerated_forward_compat() {
        let key = sample_key();
        let header = sample_header();
        let sig = sign_ciphertext(&key, b"body");
        let sig_b64 = URL_SAFE_NO_PAD.encode(sig);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"ztl-Schema: v4\n");
        bytes.extend_from_slice(b"ztl-Cohort-Id: engineering\n");
        bytes.extend_from_slice(b"ztl-Cohort-Mode: delegated-url\n");
        bytes.extend_from_slice(b"ztl-Slug: onboarding\n");
        bytes.extend_from_slice(b"ztl-Build-Epoch: 2026-04-20T10:15:00Z\n");
        bytes.extend_from_slice(format!("ztl-Signature: {sig_b64}\n").as_bytes());
        bytes.extend_from_slice(b"ztl-Future-Hint: reserved-for-v0.5.x\n");
        bytes.extend_from_slice(b"\nbody");
        let parsed = parse_envelope(&bytes).expect("unknown headers tolerated");
        assert_eq!(parsed.header, header);
        verify_ciphertext(&key.verifying_key(), &parsed.ciphertext, &parsed.signature)
            .expect("signature intact");
    }

    #[test]
    fn parse_signing_key_round_trips_genkey_standard_alphabet() {
        // `ztl cap genkey` emits STANDARD base64; parser must accept
        // that verbatim as the env-var payload.
        let raw = [9u8; SIGNING_KEY_LEN];
        let encoded = STANDARD.encode(raw);
        let parsed =
            parse_signing_key_b64("ztl_CAP_SIGNING_KEY", &encoded).expect("parse STANDARD");
        assert_eq!(
            parsed.verifying_key().to_bytes(),
            VaultSigningKey::from_bytes(&raw).verifying_key().to_bytes()
        );
    }

    #[test]
    fn parse_signing_key_rejects_wrong_length() {
        let short = STANDARD.encode([0u8; SIGNING_KEY_LEN - 1]);
        match parse_signing_key_b64("ztl_CAP_SIGNING_KEY", &short) {
            Err(KeyLoadError::Length { got, .. }) => assert_eq!(got, SIGNING_KEY_LEN - 1),
            Err(other) => panic!("expected Length, got {other:?}"),
            Ok(_) => panic!("short key must be rejected"),
        }
    }

    #[test]
    fn parse_signing_key_rejects_empty() {
        match parse_signing_key_b64("ztl_CAP_SIGNING_KEY", "   \n ") {
            Err(KeyLoadError::EnvEmpty { .. }) => {}
            Err(other) => panic!("expected EnvEmpty, got {other:?}"),
            Ok(_) => panic!("empty key must be rejected"),
        }
    }

    #[test]
    fn parse_signing_pubkey_accepts_both_alphabets() {
        let key = sample_key();
        let pub_ = key.verifying_key();

        let url = pub_.to_b64url();
        let std_ = pub_.to_b64_standard();
        let from_url = parse_signing_pubkey_b64(&url).expect("url-safe parses");
        let from_std = parse_signing_pubkey_b64(&std_).expect("standard parses");
        assert_eq!(from_url.to_bytes(), pub_.to_bytes());
        assert_eq!(from_std.to_bytes(), pub_.to_bytes());
    }

    #[test]
    fn parse_signing_pubkey_recipients_toml_form() {
        let key = sample_key();
        let pub_ = key.verifying_key();
        let toml_form = format!("{ED25519_PUBKEY_PREFIX}{}", pub_.to_b64url());
        let parsed =
            parse_signing_pubkey_recipients_toml(&toml_form).expect("prefixed form parses");
        assert_eq!(parsed.to_bytes(), pub_.to_bytes());
    }

    #[test]
    fn parse_signing_pubkey_requires_prefix_in_recipients_toml() {
        let key = sample_key();
        let err = parse_signing_pubkey_recipients_toml(&key.verifying_key().to_b64url())
            .expect_err("unprefixed form rejected");
        assert!(matches!(err, KeyLoadError::MissingPrefix { .. }));
    }

    #[test]
    fn verify_ciphertext_rejects_non_canonical_signature() {
        // `verify_strict` rejects non-canonical S (RFC 8032 §8.4).
        // An all-zero signature is not canonical; this probe pins
        // that we picked `verify_strict` and not `verify`.
        let pub_ = sample_key().verifying_key();
        let zeros = [0u8; SIGNATURE_LEN];
        assert!(matches!(
            verify_ciphertext(&pub_, b"whatever", &zeros),
            Err(VerifyError::Invalid(_))
        ));
    }

    #[test]
    fn webauthn_prf_cohort_mode_serialises_kebab_case() {
        let key = sample_key();
        let header = EnvelopeHeader {
            cohort_id: "secret-team".to_string(),
            cohort_mode: CohortMode::WebauthnPrf,
            slug: "x".to_string(),
            build_epoch: "t".to_string(),
        };
        let bytes = sign_and_build_envelope(&key, &header, b"y");
        let text = std::str::from_utf8(&bytes[..160]).unwrap_or("");
        assert!(text.contains(&format!("{HEADER_COHORT_MODE}: webauthn-prf\n")));
        let parsed = parse_envelope(&bytes).expect("parse");
        assert_eq!(parsed.header.cohort_mode, CohortMode::WebauthnPrf);
    }

    #[test]
    fn load_from_env_reads_ztl_cap_signing_key() {
        // The effectful shell path. Uses an in-process env mutation
        // guarded by a serialisation to avoid cross-test interleaving.
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());

        let raw = [3u8; SIGNING_KEY_LEN];
        let encoded = STANDARD.encode(raw);
        let prev = env::var(ztl_CAP_SIGNING_KEY_ENV).ok();
        // SAFETY: test-only env mutation; guarded by LOCK above.
        unsafe { env::set_var(ztl_CAP_SIGNING_KEY_ENV, &encoded) };
        let loaded = load_signing_key_from_env().expect("loads");
        assert_eq!(
            loaded.verifying_key().to_bytes(),
            VaultSigningKey::from_bytes(&raw).verifying_key().to_bytes()
        );
        match prev {
            Some(v) => unsafe { env::set_var(ztl_CAP_SIGNING_KEY_ENV, v) },
            None => unsafe { env::remove_var(ztl_CAP_SIGNING_KEY_ENV) },
        }
    }

    #[test]
    fn load_from_env_reports_missing_env_with_remediation() {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());

        let prev = env::var(ztl_CAP_SIGNING_KEY_ENV).ok();
        unsafe { env::remove_var(ztl_CAP_SIGNING_KEY_ENV) };
        let err = match load_signing_key_from_env() {
            Ok(_) => panic!("missing env must be rejected"),
            Err(e) => e,
        };
        let rendered = format!("{err}");
        assert!(
            rendered.contains("ztl cap genkey"),
            "remediation hint missing: {rendered}"
        );
        assert!(matches!(err, KeyLoadError::EnvMissing { .. }));
        if let Some(v) = prev {
            unsafe { env::set_var(ztl_CAP_SIGNING_KEY_ENV, v) };
        }
    }

    #[test]
    fn signature_is_base64url_not_standard_alphabet() {
        // The envelope's `ztl-Signature` header is base64url
        // (URL_SAFE_NO_PAD). A signature that happened to contain
        // `+` or `/` (i.e. STANDARD alphabet) would be wrong.
        let key = sample_key();
        let bytes = sign_and_build_envelope(&key, &sample_header(), b"probe");
        let header_slice = &bytes[..bytes.windows(2).position(|w| w == b"\n\n").unwrap()];
        let header_text = std::str::from_utf8(header_slice).unwrap();
        let sig_line = header_text
            .lines()
            .find(|l| l.starts_with(HEADER_SIGNATURE))
            .expect("signature present");
        assert!(
            !sig_line.contains('+'),
            "base64url must not emit `+`: {sig_line}"
        );
        assert!(
            !sig_line.contains('/'),
            "base64url must not emit `/`: {sig_line}"
        );
        assert!(
            !sig_line.contains('='),
            "unpadded base64url must not emit `=`: {sig_line}"
        );
    }
}
