//! Per-page age v1 ciphertext emission for capability-URL mode
//! (SPEC-034 REQ-3403, CON-3404, ADR-3403).
//!
//! This is the effectful shell that turns `(plaintext, real_recipients,
//! padding_tier)` into the age ciphertext bytes the build driver writes
//! to `dist/c/<path-cap>/<slug>.html`. Padding pubkeys are produced by
//! [`crate::cap::pad`] (pure-core once the RNG is supplied) and handed
//! to age in the same bucket as real recipients — both render as
//! `-> X25519 <ephemeral-share>` stanzas, satisfying REQ-3422
//! acceptance #3 ("no recipient-type distinguisher").
//!
//! # Boundary
//!
//! SPEC-034 §8 places `cap::build::age_encrypt` in the **effectful
//! shell**: the age file-key that keys the actual STREAM/ChaCha20-Poly1305
//! encryption is sampled inside the `age` crate from its own CSPRNG,
//! so the ciphertext bytes are non-deterministic across builds even
//! for identical input. Padding is deterministic once its RNG is
//! supplied (pure-core contract), but this module's output is not.
//!
//! # API shape
//!
//! - [`encrypt_to_cohort`] — OS CSPRNG for padding; the normal build
//!   entry point.
//! - [`encrypt_to_cohort_with_rng`] — seeded RNG for the padding side.
//!   Tests pin padding to a known ChaCha20 seed so the recipient
//!   count + tier invariants (REQ-3422 #1, #2) are checkable at the
//!   header level without depending on OS-entropy.
//!
//! Both return an [`AgeCiphertext`] that reports `{ bytes, real_count,
//! padding_count, tier }` so the build driver can feed OBS-3410
//! telemetry without re-parsing the age header.
//!
//! # Why bech32 round-trip?
//!
//! `age::x25519::Recipient` exposes no public constructor from a raw
//! `[u8; 32]` in 0.11 — only `FromStr` on the `"age1…"` bech32
//! string. We therefore encode each pubkey (real or padding) into the
//! canonical age1 form with the standard `Bech32` variant (HRP `"age"`)
//! and `parse::<age::x25519::Recipient>()` it. If that parse ever
//! fails it signals a programming error (we produced the encoding
//! ourselves), not bad external input — surfaced as
//! [`AgeEncryptError::RecipientParse`] so a regression is visible in
//! build logs rather than a panic.

use std::io::Write;

use bech32::{Bech32, Hrp};
use rand_core::{CryptoRng, OsRng, RngCore};

use crate::cap::pad::{self, PadError, X25519Pubkey, TIERS};

/// Result of encrypting one page to a cohort.
///
/// The counts are reported for OBS-3410 telemetry (tier distribution
/// per build) without forcing callers to re-parse the age header.
#[derive(Debug, Clone)]
pub struct AgeCiphertext {
    /// Raw age v1 ciphertext bytes. Signed verbatim by the content-
    /// signing layer (REQ-3427); the Ed25519 signature covers *these*
    /// bytes, not the envelope header.
    pub bytes: Vec<u8>,
    /// Number of real cohort recipients whose pubkeys were wrapped.
    pub real_count: usize,
    /// Number of ephemeral padding pubkeys the age header carries.
    /// `real_count + padding_count == tier`.
    pub padding_count: usize,
    /// Canonical tier ∈ [`TIERS`] (= smallest tier ≥ `real_count`).
    pub tier: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum AgeEncryptError {
    /// Padding-layer error (most notably [`PadError::Overflow`] when
    /// the real cohort exceeds the largest tier).
    #[error("padding generation failed: {0}")]
    Padding(#[from] PadError),

    /// A recipient pubkey we produced ourselves failed to re-parse as
    /// `age::x25519::Recipient`. Only reachable on a programming bug
    /// (wrong HRP, wrong variant, ad-hoc length), not on bad caller
    /// input — there is no caller-provided string here. `reason` is
    /// the `&'static str` the age crate returns from `FromStr` —
    /// kept as owned text so the error can cross thread boundaries.
    #[error("age recipient parse failed for recipient {index}: {reason}")]
    RecipientParse { index: usize, reason: String },

    /// `age::Encryptor::with_recipients` rejected the recipient set.
    /// In 0.11 this is only hit for an empty recipient list, which we
    /// prevent by construction — any tier pads the set to ≥ 10 — so
    /// this variant is a belt-and-braces surfacing of an age
    /// upstream change rather than an expected runtime error.
    #[error("age Encryptor construction failed: {0}")]
    Encrypt(String),

    /// I/O error while streaming plaintext through the age writer or
    /// flushing its `finish()`. age v1 is finalised by a mandatory
    /// `finish()` call; forgetting it produces a truncated ciphertext
    /// that refuses to decrypt, so we propagate errors from both
    /// `write_all` and `finish`.
    #[error("i/o during age ciphertext write: {0}")]
    Io(#[from] std::io::Error),
}

/// Encrypt `plaintext` to the union of `real_recipients` + X25519
/// padding, sized to `pad::tier_for(real_recipients.len())`. Padding
/// entropy comes from [`OsRng`]; the padding scalars are zeroized
/// inside [`pad::pad_to_tier`] before this function returns.
///
/// Errors: see [`AgeEncryptError`]. The common failure mode is a
/// cohort larger than [`pad::MAX_REAL_RECIPIENTS`], which surfaces as
/// [`AgeEncryptError::Padding`] wrapping [`PadError::Overflow`].
pub fn encrypt_to_cohort(
    plaintext: &[u8],
    real_recipients: &[X25519Pubkey],
) -> Result<AgeCiphertext, AgeEncryptError> {
    encrypt_to_cohort_with_rng(plaintext, real_recipients, &mut OsRng)
}

/// Seeded variant of [`encrypt_to_cohort`]. `rng` feeds the padding
/// side only — the age *file-key* is still sampled by the `age` crate
/// from its own CSPRNG, so ciphertext bytes are non-deterministic even
/// under a pinned `rng`. The recipient *count*, tier, and padding
/// pubkeys are deterministic, which is what the header-level tests
/// pin.
pub fn encrypt_to_cohort_with_rng<R: RngCore + CryptoRng>(
    plaintext: &[u8],
    real_recipients: &[X25519Pubkey],
    rng: &mut R,
) -> Result<AgeCiphertext, AgeEncryptError> {
    let real_count = real_recipients.len();
    let tier = pad::tier_for(real_count)?;
    let padding = pad::pad_to_tier_with_rng(real_count, rng)?;
    let padding_count = padding.len();
    debug_assert_eq!(real_count + padding_count, tier);
    debug_assert!(TIERS.contains(&tier));

    // Owned recipients — age 0.11 `with_recipients` takes
    // `impl Iterator<Item = &'a dyn Recipient>`, so we hold the
    // `x25519::Recipient` values here and hand out references below.
    let mut owned: Vec<age::x25519::Recipient> = Vec::with_capacity(tier);
    for (idx, pk) in real_recipients.iter().chain(padding.iter()).enumerate() {
        let encoded = encode_age_recipient(pk);
        let parsed: age::x25519::Recipient =
            encoded
                .parse()
                .map_err(|e: &str| AgeEncryptError::RecipientParse {
                    index: idx,
                    reason: e.to_string(),
                })?;
        owned.push(parsed);
    }

    let recipients_iter = owned.iter().map(|r| r as &dyn age::Recipient);
    let encryptor = age::Encryptor::with_recipients(recipients_iter)
        .map_err(|e| AgeEncryptError::Encrypt(e.to_string()))?;

    let mut bytes = Vec::with_capacity(plaintext.len() + 512);
    let mut writer = encryptor
        .wrap_output(&mut bytes)
        .map_err(|e| AgeEncryptError::Encrypt(e.to_string()))?;
    writer.write_all(plaintext)?;
    writer.finish()?;

    Ok(AgeCiphertext {
        bytes,
        real_count,
        padding_count,
        tier,
    })
}

/// Encode a raw 32-byte X25519 pubkey as the canonical age recipient
/// string `age1<bech32(pubkey)>` (standard Bech32 variant, HRP `"age"`).
/// This is the same format `age::x25519::Recipient::to_string()`
/// emits, which is what makes the `parse()` round-trip safe.
fn encode_age_recipient(pk: &X25519Pubkey) -> String {
    let hrp = Hrp::parse("age").expect("hrp \"age\" is a valid bech32 hrp");
    bech32::encode::<Bech32>(hrp, pk).expect("32 bytes fits inside bech32 length limit")
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Read;

    use age::x25519;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    fn fresh_identity() -> (x25519::Identity, X25519Pubkey) {
        let id = x25519::Identity::generate();
        let pk_string = id.to_public().to_string();
        let pk_bytes = decode_age_pubkey_str(&pk_string);
        (id, pk_bytes)
    }

    /// Decode a `to_public().to_string()` value (canonical "age1…"
    /// bech32) back to raw 32-byte pubkey. Test-only inverse of
    /// [`encode_age_recipient`]; used so we can feed the age crate's
    /// own generated identities into the module under test.
    fn decode_age_pubkey_str(s: &str) -> X25519Pubkey {
        let (hrp, data) = bech32::decode(s).expect("valid bech32");
        assert_eq!(hrp.as_str(), "age");
        let mut out = [0u8; 32];
        assert_eq!(data.len(), 32, "age pubkey must be 32 bytes");
        out.copy_from_slice(&data);
        out
    }

    fn decrypt_with(ciphertext: &[u8], id: &x25519::Identity) -> Option<Vec<u8>> {
        let decryptor = age::Decryptor::new(ciphertext).ok()?;
        let iter = std::iter::once(id as &dyn age::Identity);
        let mut reader = decryptor.decrypt(iter).ok()?;
        let mut out = Vec::new();
        reader.read_to_end(&mut out).ok()?;
        Some(out)
    }

    #[test]
    fn round_trip_with_real_recipient_succeeds() {
        let (id, pk) = fresh_identity();
        let plaintext = b"REQ-3403: page-body bytes under age + Ed25519 envelope";
        let out =
            encrypt_to_cohort_with_rng(plaintext, &[pk], &mut ChaCha20Rng::seed_from_u64(0xA6E))
                .expect("encrypt");
        assert_eq!(out.real_count, 1);
        assert_eq!(out.tier, 10);
        assert_eq!(out.padding_count, 9);
        let decrypted = decrypt_with(&out.bytes, &id).expect("decrypt");
        assert_eq!(decrypted, plaintext);
    }

    /// REQ-3422 acceptance #5 (observer model): a reader who does NOT
    /// hold any recipient private key cannot decrypt. Since padding
    /// scalars are zeroized inside `cap::pad`, nobody on the network
    /// ever holds a "padding-slot key" — this test exercises that
    /// invariant through the age reference implementation: an
    /// unrelated identity (functionally equivalent to a padding-slot
    /// identity the attacker would need to forge) cannot unwrap the
    /// file key.
    #[test]
    fn decrypt_with_padding_slot_key_fails() {
        let (_real_id, real_pk) = fresh_identity();
        let out = encrypt_to_cohort_with_rng(
            b"secret",
            &[real_pk],
            &mut ChaCha20Rng::seed_from_u64(0xA6E),
        )
        .expect("encrypt");

        // A freshly-sampled identity is indistinguishable from a
        // padding-slot identity the attacker would need to forge to
        // impersonate one of the tier-10 dummy stanzas.
        let bystander = x25519::Identity::generate();
        assert!(
            decrypt_with(&out.bytes, &bystander).is_none(),
            "non-recipient identity must not decrypt"
        );
    }

    /// REQ-3422 acceptance #1: real + padding = tier value exactly,
    /// verified at the [`AgeCiphertext`] metadata level for every
    /// tier boundary + interior.
    #[test]
    fn real_plus_padding_equals_tier_across_ladder() {
        let mut rng = ChaCha20Rng::seed_from_u64(0xBEEF);
        // A handful of real pubkeys; we only care about counts here
        // so a single fresh identity cloned into the list suffices.
        let (_id, pk) = fresh_identity();
        for real in [0usize, 1, 10, 11, 30, 100, 300, 999, 1000] {
            let recipients = vec![pk; real];
            let out = encrypt_to_cohort_with_rng(b"x", &recipients, &mut rng).expect("encrypt");
            let tier = pad::tier_for(real).unwrap();
            assert_eq!(out.real_count, real);
            assert_eq!(out.tier, tier);
            assert_eq!(out.real_count + out.padding_count, tier);
        }
    }

    #[test]
    fn overflow_surfaces_from_padding_layer() {
        let (_id, pk) = fresh_identity();
        let recipients = vec![pk; pad::MAX_REAL_RECIPIENTS + 1];
        let err = encrypt_to_cohort(b"body", &recipients).unwrap_err();
        assert!(matches!(
            err,
            AgeEncryptError::Padding(PadError::Overflow { got }) if got == pad::MAX_REAL_RECIPIENTS + 1
        ));
    }

    /// REQ-3422 acceptance #3: no recipient-type distinguisher. The
    /// age header for real + padding uses the same `-> X25519` stanza
    /// prefix for every entry; a simple byte scan over the ASCII
    /// header confirms that exactly `tier` X25519 stanzas are present
    /// and no other recipient type (scrypt/ssh/…) appears. Byte-level
    /// rather than library-level so a refactor that accidentally
    /// switches recipient type is caught in this test, not in a
    /// downstream release.
    #[test]
    fn header_stanzas_are_all_x25519() {
        let (_id, pk) = fresh_identity();
        let out = encrypt_to_cohort_with_rng(b"body", &[pk], &mut ChaCha20Rng::seed_from_u64(1))
            .expect("encrypt");

        // age v1 closes its ASCII header with a `--- <b64>\n` MAC
        // line; everything before that newline is ASCII and safe to
        // inspect as UTF-8. Slice on the LF after `--- ` so the MAC
        // line itself is included in the scan (it's part of the
        // "no scrypt" surface).
        let mac_idx = out
            .bytes
            .windows(4)
            .position(|w| w == b"\n---")
            .expect("age v1 header has a `--- <mac>` terminator line");
        let mac_end = out.bytes[mac_idx + 1..]
            .iter()
            .position(|&b| b == b'\n')
            .expect("mac line is newline-terminated");
        let header = std::str::from_utf8(&out.bytes[..mac_idx + 1 + mac_end])
            .expect("age v1 header is ASCII up to the MAC line");

        let x25519_stanzas = header.matches("-> X25519 ").count();
        assert_eq!(
            x25519_stanzas, out.tier,
            "header must contain exactly tier X25519 stanzas (no type distinguisher)"
        );
        assert!(
            !header.contains("-> scrypt "),
            "no scrypt stanza may appear (BUG-001: scrypt padding is forbidden)"
        );
        assert!(
            !header.contains("-> ssh-"),
            "no ssh stanza may appear in cohort ciphertexts"
        );
    }

    #[test]
    fn encode_age_recipient_matches_age_crate_bech32() {
        // Round-trip: if our encoder agrees with the age crate's
        // canonical formatting, parsing our output must produce an
        // `x25519::Recipient` with the same Display form the age
        // crate would have emitted for the identity we started from.
        let id = x25519::Identity::generate();
        let canonical = id.to_public().to_string();
        let raw = decode_age_pubkey_str(&canonical);
        let ours = encode_age_recipient(&raw);
        assert_eq!(ours, canonical);
    }
}
