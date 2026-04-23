//! End-to-end round-trips for SPEC-034 REQ-3403 / CON-3404 / ADR-3403
//! age-v1 ciphertext emission.
//!
//! The unit tests in `src/cap/age_encrypt.rs` pin internal invariants
//! (tier accounting, header-level "no recipient-type distinguisher",
//! bech32 canonicalisation). This integration file exercises the
//! public API surface that the build driver (`task-cap-build-driver`)
//! will call, through the same `ztl::cap::age_encrypt` module path
//! that ships to consumers.
//!
//! Coverage:
//!
//! - Rust-encrypt → age-crate decrypt (reference implementation)
//!   with a real cohort identity succeeds and returns the exact
//!   plaintext.
//! - Decrypt with an identity outside the real recipient set fails
//!   — this is the "padding-slot key fails" test in the task
//!   description. Since `cap::pad` zeroises padding scalars on drop
//!   (ADR-3413 compile-time guard), no padding-slot private key is
//!   retrievable; a fresh unrelated identity is the closest thing
//!   any observer could bring, and it must not decrypt.
//! - Tier structure observable through the returned `AgeCiphertext`
//!   metadata is internally consistent with the tier ladder
//!   (REQ-3422).
//!
//! TEST-3403 (Rust side).

use std::io::Read;

use age::x25519;
use bech32::{Bech32, Hrp};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

use ztl::cap::age_encrypt::{encrypt_to_cohort_with_rng, AgeEncryptError};
use ztl::cap::pad::{self, X25519Pubkey};

/// Decode an `age1...` bech32 pubkey string (as produced by
/// `x25519::Identity::to_public().to_string()`) back to the raw
/// 32-byte public key the `cap::age_encrypt` API expects.
fn pubkey_from_age_string(s: &str) -> X25519Pubkey {
    let (hrp, data) = bech32::decode(s).expect("valid age1 bech32");
    assert_eq!(hrp.as_str(), "age");
    let mut pk = [0u8; 32];
    assert_eq!(data.len(), 32);
    pk.copy_from_slice(&data);
    pk
}

fn fresh_identity_pair() -> (x25519::Identity, X25519Pubkey) {
    let id = x25519::Identity::generate();
    let pk = pubkey_from_age_string(&id.to_public().to_string());
    (id, pk)
}

fn try_decrypt(ciphertext: &[u8], id: &x25519::Identity) -> Option<Vec<u8>> {
    let decryptor = age::Decryptor::new(ciphertext).ok()?;
    let mut reader = decryptor
        .decrypt(std::iter::once(id as &dyn age::Identity))
        .ok()?;
    let mut out = Vec::new();
    reader.read_to_end(&mut out).ok()?;
    Some(out)
}

#[test]
fn req_3403_round_trip_single_cohort_recipient() {
    let (alice, alice_pk) = fresh_identity_pair();
    let plaintext = b"<html><body>REQ-3403 sanitised page body</body></html>";

    let ct = encrypt_to_cohort_with_rng(
        plaintext,
        &[alice_pk],
        &mut ChaCha20Rng::seed_from_u64(0xAAAA),
    )
    .expect("encrypt succeeds");

    // Bookkeeping matches the canonical tier ladder.
    assert_eq!(ct.real_count, 1);
    assert_eq!(ct.tier, 10);
    assert_eq!(ct.padding_count, 9);
    assert_eq!(ct.real_count + ct.padding_count, ct.tier);

    let decrypted = try_decrypt(&ct.bytes, &alice).expect("alice decrypts");
    assert_eq!(decrypted, plaintext);
}

#[test]
fn req_3403_round_trip_multi_recipient_cohort() {
    // A typical small-cohort case — every listed reader must be able
    // to decrypt independently.
    let readers: Vec<(x25519::Identity, X25519Pubkey)> =
        (0..5).map(|_| fresh_identity_pair()).collect();
    let pubkeys: Vec<X25519Pubkey> = readers.iter().map(|(_, pk)| *pk).collect();

    let plaintext = b"shared cohort page";
    let ct =
        encrypt_to_cohort_with_rng(plaintext, &pubkeys, &mut ChaCha20Rng::seed_from_u64(0x5555))
            .expect("encrypt succeeds");

    assert_eq!(ct.real_count, 5);
    assert_eq!(ct.tier, 10);
    assert_eq!(ct.padding_count, 5);

    for (id, _) in &readers {
        let decrypted = try_decrypt(&ct.bytes, id).expect("each reader decrypts");
        assert_eq!(decrypted, plaintext);
    }
}

/// Task description: "decrypt with a padding-slot key fails". Padding
/// scalars are zeroised in `cap::pad`, so no padding-slot private key
/// is ever retained — we stand in for that with a fresh unrelated
/// identity, which is exactly the capability level an outside
/// observer (or an attacker attempting to forge a padding-slot key)
/// would bring. The age reference implementation must refuse to
/// decrypt.
#[test]
fn req_3422_decrypt_with_non_recipient_key_fails() {
    let (_alice, alice_pk) = fresh_identity_pair();
    let ct = encrypt_to_cohort_with_rng(
        b"cohort-only",
        &[alice_pk],
        &mut ChaCha20Rng::seed_from_u64(0xB0B),
    )
    .expect("encrypt succeeds");

    let bystander = x25519::Identity::generate();
    assert!(
        try_decrypt(&ct.bytes, &bystander).is_none(),
        "only cohort recipients may decrypt"
    );
}

/// REQ-3422 acceptance #3 cross-check via public API only: the age
/// header contains exactly `tier` `"-> X25519 "` stanza prefixes and
/// no other recipient-type tag. Real and padding entries are
/// byte-indistinguishable in the header.
#[test]
fn req_3422_no_recipient_type_distinguisher_in_header() {
    let (_id, pk) = fresh_identity_pair();
    let ct =
        encrypt_to_cohort_with_rng(b"body", &[pk; 11], &mut ChaCha20Rng::seed_from_u64(0xC0DE))
            .expect("encrypt succeeds");

    // 11 real pubkeys → tier 30 → 19 padding entries.
    assert_eq!(ct.tier, 30);
    assert_eq!(ct.padding_count, 19);

    let header_ascii_end = ct
        .bytes
        .windows(4)
        .position(|w| w == b"\n---")
        .expect("v1 has a `--- <mac>` terminator");
    let line_end = ct.bytes[header_ascii_end + 1..]
        .iter()
        .position(|&b| b == b'\n')
        .expect("mac line is newline-terminated");
    let header =
        std::str::from_utf8(&ct.bytes[..header_ascii_end + 1 + line_end]).expect("ascii header");

    let x25519_count = header.matches("-> X25519 ").count();
    assert_eq!(x25519_count, ct.tier);
    assert!(!header.contains("-> scrypt "));
    assert!(!header.contains("-> ssh-"));
}

#[test]
fn oversized_cohort_surfaces_padding_overflow() {
    let hrp = Hrp::parse("age").unwrap();
    let pk_bytes = [0x42u8; 32];
    // Raw pubkey irrelevant — the error fires before age is called.
    let _encoded = bech32::encode::<Bech32>(hrp, &pk_bytes).unwrap();

    let recipients = vec![pk_bytes; pad::MAX_REAL_RECIPIENTS + 1];
    let err = encrypt_to_cohort_with_rng(b"x", &recipients, &mut ChaCha20Rng::seed_from_u64(0))
        .expect_err("overflow");
    assert!(matches!(err, AgeEncryptError::Padding(_)));
}
