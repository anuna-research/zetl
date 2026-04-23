//! End-to-end coverage for SPEC-034 REQ-3427 / CON-3411 / ADR-3412
//! — Ed25519 content signing (TEST-3427).
//!
//! The unit tests in `src/cap/sign.rs` pin internal invariants
//! (header layout, pure-core determinism, parser). This integration
//! file exercises the public API through the `ztl::cap::sign` path
//! the build driver will import, and composes the signing layer with
//! a real age v1 ciphertext produced by `cap::age_encrypt` — the
//! same pair the shim will see on the wire.
//!
//! TEST-3427 acceptance (SPEC-034 §9):
//!
//! 1. Positive: build a signed envelope; verify with the embedded
//!    pubkey; assert pass.
//! 2. Tamper with the signature; assert verification fails.
//! 3. Tamper with the ciphertext; assert verification fails.
//! 4. Swap in a different vault's signing pubkey; assert verification
//!    fails.
//!
//! Plus the CON-3404 "envelope header is not authenticated"
//! invariant: header mutation must leave the signature intact (the
//! spec reasons about this as "headers exist for dispatch, not
//! authentication; changing the ciphertext invalidates the
//! signature, changing headers does not"). The tighter end-to-end
//! test is that a header-mutated envelope still verifies and still
//! decrypts; the ciphertext-mutated envelope does not.

use age::x25519;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

use ztl::cap::age_encrypt::encrypt_to_cohort_with_rng;
use ztl::cap::genkey::{SIGNING_KEY_LEN, ztl_CAP_SIGNING_KEY_ENV};
use ztl::cap::pad::X25519Pubkey;
use ztl::cap::recipients::parsing::CohortMode;
use ztl::cap::sign::{
    build_envelope, parse_envelope, sign_and_build_envelope, sign_ciphertext, verify_ciphertext,
    EnvelopeHeader, EnvelopeParseError, KeyLoadError, VaultSigningKey, VerifyError,
    ENVELOPE_SCHEMA, HEADER_SCHEMA, HEADER_SIGNATURE, SIGNATURE_LEN,
};

fn pubkey_from_age_string(s: &str) -> X25519Pubkey {
    let (hrp, data) = bech32::decode(s).expect("valid age1 bech32");
    assert_eq!(hrp.as_str(), "age");
    let mut pk = [0u8; 32];
    assert_eq!(data.len(), 32);
    pk.copy_from_slice(&data);
    pk
}

fn fresh_age_identity() -> (x25519::Identity, X25519Pubkey) {
    let id = x25519::Identity::generate();
    let pk = pubkey_from_age_string(&id.to_public().to_string());
    (id, pk)
}

fn sample_signing_key() -> VaultSigningKey {
    // Deterministic 32-byte seed; any bytes are a valid Ed25519 seed.
    VaultSigningKey::from_bytes(&[0x27u8; SIGNING_KEY_LEN])
}

fn other_signing_key() -> VaultSigningKey {
    VaultSigningKey::from_bytes(&[0x91u8; SIGNING_KEY_LEN])
}

fn sample_header() -> EnvelopeHeader {
    EnvelopeHeader {
        cohort_id: "engineering".to_string(),
        cohort_mode: CohortMode::DelegatedUrl,
        slug: "onboarding".to_string(),
        build_epoch: "2026-04-20T10:15:00Z".to_string(),
    }
}

fn age_ciphertext_for(plaintext: &[u8], recipient: X25519Pubkey) -> Vec<u8> {
    // Use a deterministic RNG so the padding count is stable across
    // runs; the age-crate CSPRNG still makes the *ciphertext bytes*
    // non-deterministic, which is fine — the signature is over
    // whatever bytes were produced.
    encrypt_to_cohort_with_rng(
        plaintext,
        &[recipient],
        &mut ChaCha20Rng::seed_from_u64(0x2026),
    )
    .expect("age encrypt")
    .bytes
}

#[test]
fn test_3427_positive_envelope_verifies_end_to_end() {
    let key = sample_signing_key();
    let (_id, pk) = fresh_age_identity();
    let ciphertext = age_ciphertext_for(b"<html>secret</html>", pk);

    let envelope = sign_and_build_envelope(&key, &sample_header(), &ciphertext);
    let parsed = parse_envelope(&envelope).expect("envelope parses");

    // Signature must verify against the same pubkey the shim embeds.
    verify_ciphertext(&key.verifying_key(), &parsed.ciphertext, &parsed.signature)
        .expect("TEST-3427: positive envelope verifies");

    // The ciphertext surface is unchanged — the envelope carries it
    // byte-for-byte so the shim's `ageDecrypt` call receives exactly
    // what `cap::age_encrypt` emitted.
    assert_eq!(parsed.ciphertext, ciphertext);
}

#[test]
fn test_3427_negative_signature_tamper_rejects() {
    let key = sample_signing_key();
    let (_id, pk) = fresh_age_identity();
    let ciphertext = age_ciphertext_for(b"body", pk);

    let envelope = sign_and_build_envelope(&key, &sample_header(), &ciphertext);
    let mut parsed = parse_envelope(&envelope).expect("parse");
    // Flip a single bit in the signature.
    parsed.signature[0] ^= 0x01;

    assert!(matches!(
        verify_ciphertext(&key.verifying_key(), &parsed.ciphertext, &parsed.signature),
        Err(VerifyError::Invalid(_))
    ));
}

#[test]
fn test_3427_negative_ciphertext_tamper_rejects() {
    let key = sample_signing_key();
    let (_id, pk) = fresh_age_identity();
    let ciphertext = age_ciphertext_for(b"payload", pk);

    let mut envelope = sign_and_build_envelope(&key, &sample_header(), &ciphertext);
    // Locate the `\n\n` separator and flip a bit in the ciphertext body.
    let sep = envelope
        .windows(2)
        .position(|w| w == b"\n\n")
        .expect("envelope has separator");
    // The bit we flip must be in the age v1 header region — ChaCha20-
    // Poly1305 would fail the MAC even without a signature, but the
    // REQ-3427 guarantee is that the *shim* rejects before trying to
    // decrypt. That's what we assert: signature fails first.
    let body_start = sep + 2;
    let target = body_start + 16;
    assert!(
        target < envelope.len(),
        "envelope must have a ciphertext body"
    );
    envelope[target] ^= 0x01;

    let parsed = parse_envelope(&envelope).expect("envelope still parses");
    assert!(matches!(
        verify_ciphertext(&key.verifying_key(), &parsed.ciphertext, &parsed.signature),
        Err(VerifyError::Invalid(_))
    ));
}

#[test]
fn test_3427_negative_wrong_vault_pubkey_rejects() {
    let key = sample_signing_key();
    let other = other_signing_key();
    let (_id, pk) = fresh_age_identity();
    let ciphertext = age_ciphertext_for(b"cross-vault probe", pk);

    let envelope = sign_and_build_envelope(&key, &sample_header(), &ciphertext);
    let parsed = parse_envelope(&envelope).expect("parse");

    // Build was signed by `key`; shim tries `other` (different vault).
    // REQ-3427: verification MUST fail.
    assert!(matches!(
        verify_ciphertext(
            &other.verifying_key(),
            &parsed.ciphertext,
            &parsed.signature
        ),
        Err(VerifyError::Invalid(_))
    ));
}

#[test]
fn test_3427_signature_covers_ciphertext_only_headers_untrusted() {
    // CON-3404: "The Ed25519 signature covers the bytes of the age
    // ciphertext only (excluding the envelope headers)."
    //
    // A header mutation leaves the signature intact. The invariant
    // the spec encodes is that the attack surface is *separable*:
    // headers give up NO authentication to an attacker (a header-
    // swapped envelope parses, but it still holds the original
    // ciphertext, which was meant for the cohort the signature
    // vouches for — so a CDN that rewrites headers cannot present
    // "attacker-controlled content" to the shim).
    let key = sample_signing_key();
    let (_id, pk) = fresh_age_identity();
    let ciphertext = age_ciphertext_for(b"body", pk);

    let envelope = sign_and_build_envelope(&key, &sample_header(), &ciphertext);

    // In-place overwrite of the slug header — same byte length.
    let needle = b"ztl-Slug: onboarding";
    let replace = b"ztl-Slug: ATTACKER-X";
    assert_eq!(needle.len(), replace.len());
    let at = envelope
        .windows(needle.len())
        .position(|w| w == needle)
        .expect("slug header present");
    let mut tampered = envelope.clone();
    tampered[at..at + needle.len()].copy_from_slice(replace);

    let parsed = parse_envelope(&tampered).expect("header-mutated envelope still parses");
    // Signature still verifies: headers are not part of the signed
    // byte range.
    verify_ciphertext(&key.verifying_key(), &parsed.ciphertext, &parsed.signature)
        .expect("header-only tamper must not invalidate the signature");
    // But the slug is now the attacker's string — proof that
    // headers are not authenticated and therefore must not be
    // trusted for security decisions.
    assert_eq!(parsed.header.slug, "ATTACKER-X");
}

#[test]
fn test_3427_envelope_byte_layout_matches_con_3404() {
    // The wire envelope starts with `ztl-Schema: v4\n`. Pin the
    // first bytes so a builder refactor that accidentally reorders
    // headers is caught here, not by the shim.
    let key = sample_signing_key();
    let ciphertext = b"deterministic body";
    let sig = sign_ciphertext(&key, ciphertext);
    let envelope = build_envelope(&sample_header(), &sig, ciphertext);

    let prefix = format!("{HEADER_SCHEMA}: {ENVELOPE_SCHEMA}\n");
    assert!(
        envelope.starts_with(prefix.as_bytes()),
        "envelope must begin with `{prefix}`; got {:?}",
        std::str::from_utf8(&envelope[..prefix.len().min(envelope.len())])
    );

    let sep = envelope
        .windows(2)
        .position(|w| w == b"\n\n")
        .expect("separator");
    let header = std::str::from_utf8(&envelope[..sep]).expect("header is ascii");

    // Signature header present with URL_SAFE_NO_PAD base64.
    let sig_line = header
        .lines()
        .find(|l| l.starts_with(HEADER_SIGNATURE))
        .expect("signature header present");
    let b64_payload = sig_line.trim_start_matches(&format!("{HEADER_SIGNATURE}: "));
    assert_eq!(
        b64_payload.len(),
        86,
        "b64url-unpadded(64 bytes) == 86 chars; got {} ({b64_payload:?})",
        b64_payload.len()
    );
}

#[test]
fn test_3427_unsigned_envelope_is_rejected_by_parser() {
    // A short blob with no envelope structure must not parse as a
    // valid envelope. REQ-3427 requires the shim to reject before
    // any other processing; in Rust terms the parser never hands
    // the verifier bytes that don't carry the required headers.
    let blob = b"not an envelope";
    let err = parse_envelope(blob).expect_err("non-envelope blob must not parse");
    assert!(matches!(err, EnvelopeParseError::MissingSeparator));

    // A blob that happens to contain a stray `\n\n` still fails
    // because the mandatory `ztl-Schema` header is absent.
    let with_sep = b"garbage\n\nmore garbage";
    let err = parse_envelope(with_sep).expect_err("must not parse");
    assert!(
        matches!(
            err,
            EnvelopeParseError::MissingHeader(_) | EnvelopeParseError::MalformedHeaderLine(_)
        ),
        "expected MissingHeader or MalformedHeaderLine, got {err:?}"
    );
}

#[test]
fn test_3427_key_load_surfaces_missing_env_with_remediation_hint() {
    // TEST-3427 adjacent: if the build can't find ztl_CAP_SIGNING_KEY,
    // the operator must be pointed at `ztl cap genkey`. We don't
    // mutate the real env from an integration test; instead we
    // exercise the parser with an empty string, which surfaces the
    // same EnvEmpty variant the env-missing path would.
    match ztl::cap::sign::parse_signing_key_b64(ztl_CAP_SIGNING_KEY_ENV, "") {
        Err(KeyLoadError::EnvEmpty { env }) => {
            assert_eq!(env, ztl_CAP_SIGNING_KEY_ENV);
        }
        Err(other) => panic!("expected EnvEmpty variant, got {other:?}"),
        Ok(_) => panic!("empty key string must be rejected"),
    }
}

#[test]
fn test_3427_signature_length_is_exactly_64_bytes() {
    // Canonical RFC 8032 Ed25519 signature length. Used by the shim's
    // fixed-width `extractSignature` call; if this ever changes the
    // shim code breaks, so pin it here as a regression gate.
    let key = sample_signing_key();
    let sig = sign_ciphertext(&key, b"probe");
    assert_eq!(sig.len(), SIGNATURE_LEN);
    assert_eq!(SIGNATURE_LEN, 64);
}
