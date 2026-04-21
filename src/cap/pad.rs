//! X25519 ephemeral padding for age recipient lists
//! (SPEC-034 REQ-3422 + ADR-3413 — BUG-001 resolution).
//!
//! Pads each cohort's recipient list to the smallest tier ∈
//! `{10, 30, 100, 300, 1000}` ≥ `real_count` with ephemeral X25519
//! public keys whose corresponding private keys are zeroized before
//! the pubkey is returned. In the age header every recipient stanza
//! is rendered as `-> X25519 <ephemeral-share>` over a 32-byte pubkey,
//! so real and padding entries are byte-identical in shape — no
//! recipient-type distinguisher (REQ-3422 acceptance #3).
//!
//! # Purity and determinism
//!
//! [`pad_to_tier_with_rng`] is a pure function of `(real_count, rng)`:
//! passing a seeded CSPRNG reproduces the output byte-for-byte, which
//! is what SPEC-034 §8 "Purity Boundary Map" credits for
//! `cap::pad::x25519_padding_construct` — deterministic once the seed
//! is supplied. The effectful shell supplies [`rand_core::OsRng`] via
//! the convenience wrapper [`pad_to_tier`].
//!
//! # Private-key discipline (ADR-3413)
//!
//! Each padding scalar lives inside [`EphemeralPaddingSecret`], a
//! newtype that wraps [`StaticSecret`] and derives [`ZeroizeOnDrop`]
//! (the `zeroize` feature on `x25519-dalek` gives `StaticSecret` a
//! `Zeroize` impl; the derive composes that into a Drop that zeroes on
//! scope exit). After [`PublicKey`] is computed from the wrapped
//! scalar, the wrapper is dropped before the next iteration allocates.
//! `ephemeral_padding_secret_is_zeroize_on_drop` is a compile-time
//! bound that fails to build if the marker ever regresses — that is
//! the "verifiable discard" ADR-3413 asks for.
//!
//! # Acknowledged limit (REQ-3422 acceptance #5)
//!
//! An observer holding any single valid cohort private key can
//! identify their own stanza and subtract, revealing "at most
//! (tier − 1)" real recipients. SPEC-034 §11.2 documents this; no
//! claim of indistinguishability against insider observers.

use rand_core::{CryptoRng, OsRng, RngCore};
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::ZeroizeOnDrop;

/// Canonical tier ladder. REQ-3422 pins these values exactly — the
/// observable-recipient-count set is an external invariant tracked by
/// OBS-3410, so any change here is a wire-format break.
pub const TIERS: [usize; 5] = [10, 30, 100, 300, 1000];

/// Largest `real_count` that can be padded to a defined tier. A cohort
/// that exceeds this needs an out-of-band tier-ladder expansion
/// (§12 open question #3); [`pad_to_tier_with_rng`] surfaces it as
/// [`PadError::Overflow`] rather than silently rounding.
pub const MAX_REAL_RECIPIENTS: usize = 1000;

/// Serialised X25519 public key as used in an age recipient stanza.
/// Same 32-byte shape as a real recipient, which is what makes
/// REQ-3422 "structurally indistinguishable" hold.
pub type X25519Pubkey = [u8; 32];

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PadError {
    /// `real_count` exceeded [`MAX_REAL_RECIPIENTS`]; no tier exists to
    /// accommodate it. The caller must rotate the cohort or the
    /// operator must extend the tier ladder.
    #[error(
        "real recipient count {got} exceeds the largest tier ({max}); \
         rotate cohort or expand the tier ladder",
        max = MAX_REAL_RECIPIENTS
    )]
    Overflow { got: usize },
}

/// Return the smallest tier ≥ `real_count`. `0 → 10` (empty cohorts
/// still pad to the first tier so an observer cannot distinguish
/// "no readers" from "a few readers").
pub fn tier_for(real_count: usize) -> Result<usize, PadError> {
    TIERS
        .iter()
        .copied()
        .find(|&t| t >= real_count)
        .ok_or(PadError::Overflow { got: real_count })
}

/// Newtype over [`StaticSecret`] that derives [`ZeroizeOnDrop`]. The
/// inner `StaticSecret` implements `Zeroize` via x25519-dalek's
/// `zeroize` feature; the derive composes that into an explicit
/// zeroing-Drop and, crucially, ships the [`ZeroizeOnDrop`] marker so
/// the test module can assert it.
#[derive(ZeroizeOnDrop)]
struct EphemeralPaddingSecret(StaticSecret);

impl EphemeralPaddingSecret {
    fn random<R: RngCore + CryptoRng>(rng: &mut R) -> Self {
        Self(StaticSecret::random_from_rng(rng))
    }

    fn public_key(&self) -> X25519Pubkey {
        PublicKey::from(&self.0).to_bytes()
    }
}

/// Generate exactly `tier_for(real_count) − real_count` X25519 padding
/// pubkeys from `rng`. Each iteration constructs a fresh
/// [`EphemeralPaddingSecret`], derives the public key, and drops the
/// wrapper — its [`ZeroizeOnDrop`] impl zeroes the scalar before the
/// next iteration allocates.
///
/// `rng` must be a [`CryptoRng`]: passing a predictable (e.g.
/// linear-congruential) RNG would let an attacker recover the padding
/// scalars and distinguish real from padding entries after the fact.
/// The seeded-for-tests path uses `ChaCha20Rng`, which is a
/// `CryptoRng` — the trait bound is what keeps that door closed.
pub fn pad_to_tier_with_rng<R: RngCore + CryptoRng>(
    real_count: usize,
    rng: &mut R,
) -> Result<Vec<X25519Pubkey>, PadError> {
    let tier = tier_for(real_count)?;
    let needed = tier - real_count;
    let mut out = Vec::with_capacity(needed);
    for _ in 0..needed {
        let secret = EphemeralPaddingSecret::random(rng);
        out.push(secret.public_key());
        // Explicit drop so zeroize fires before the next iteration
        // allocates, not at end-of-loop scope.
        drop(secret);
    }
    Ok(out)
}

/// Convenience wrapper around [`pad_to_tier_with_rng`] that sources
/// entropy from the OS CSPRNG. Only callable from the effectful shell;
/// pure-core call sites must use [`pad_to_tier_with_rng`] with an
/// explicit RNG so the output is a function of its inputs (SPEC-034
/// §8).
pub fn pad_to_tier(real_count: usize) -> Result<Vec<X25519Pubkey>, PadError> {
    pad_to_tier_with_rng(real_count, &mut OsRng)
}

#[cfg(test)]
mod tests {
    use super::*;

    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    /// Compile-time bound: if [`EphemeralPaddingSecret`] ever loses
    /// its [`ZeroizeOnDrop`] marker (derive removed, inner type
    /// dropped its `Zeroize` impl, dep-feature flip, …), this helper
    /// stops type-checking and the test fails at build time — exactly
    /// the "verifiable discard" ADR-3413 requires.
    fn assert_zeroize_on_drop<T: ZeroizeOnDrop>() {}

    #[test]
    fn ephemeral_padding_secret_is_zeroize_on_drop() {
        assert_zeroize_on_drop::<EphemeralPaddingSecret>();
    }

    #[test]
    fn tier_for_covers_each_bucket_boundary() {
        // 0 → 10 (empty cohort still pads)
        assert_eq!(tier_for(0).unwrap(), 10);
        assert_eq!(tier_for(1).unwrap(), 10);
        assert_eq!(tier_for(10).unwrap(), 10);
        // Just past each tier rounds up.
        assert_eq!(tier_for(11).unwrap(), 30);
        assert_eq!(tier_for(30).unwrap(), 30);
        assert_eq!(tier_for(31).unwrap(), 100);
        assert_eq!(tier_for(100).unwrap(), 100);
        assert_eq!(tier_for(101).unwrap(), 300);
        assert_eq!(tier_for(300).unwrap(), 300);
        assert_eq!(tier_for(301).unwrap(), 1000);
        assert_eq!(tier_for(1000).unwrap(), 1000);
    }

    #[test]
    fn tier_for_rejects_overflow() {
        assert!(matches!(
            tier_for(1001),
            Err(PadError::Overflow { got: 1001 })
        ));
        assert!(matches!(
            tier_for(usize::MAX),
            Err(PadError::Overflow { .. })
        ));
    }

    /// REQ-3422 acceptance #1: real + padding = tier value exactly.
    /// Exercises every tier boundary plus representative interiors.
    #[test]
    fn real_plus_padding_equals_tier() {
        let mut rng = ChaCha20Rng::seed_from_u64(0xC0FFEE);
        for real in [
            0usize, 1, 5, 10, 11, 29, 30, 31, 77, 100, 101, 299, 300, 301, 777, 1000,
        ] {
            let padding = pad_to_tier_with_rng(real, &mut rng).unwrap();
            let tier = tier_for(real).unwrap();
            assert_eq!(
                real + padding.len(),
                tier,
                "real={real} + padding.len()={} != tier={tier}",
                padding.len()
            );
        }
    }

    #[test]
    fn padding_entries_are_32_byte_nonzero_pubkeys() {
        let padding = pad_to_tier_with_rng(3, &mut ChaCha20Rng::seed_from_u64(1)).unwrap();
        assert_eq!(padding.len(), 7); // tier 10 − 3 real
        for pk in &padding {
            // Fixed-size return type, but assert the length invariant
            // at the value-level too so a refactor that widens the
            // alias is caught.
            assert_eq!(pk.len(), 32);
            // A scalar sampled from a CSPRNG produces an all-zero
            // pubkey only with negligible probability; a non-zero
            // check catches a regression that returned an
            // uninitialised buffer.
            assert!(pk.iter().any(|&b| b != 0));
        }
    }

    #[test]
    fn padding_entries_are_distinct() {
        // tier 10 with 0 real → 10 padding entries; two colliding
        // X25519 pubkeys under 256-bit birthday is ≈ 2⁻²⁴⁰, so a dup
        // indicates a bug (e.g. RNG reuse), not bad luck.
        let padding = pad_to_tier_with_rng(0, &mut ChaCha20Rng::seed_from_u64(2)).unwrap();
        let mut sorted = padding.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), padding.len());
    }

    /// SPEC-034 §8: `cap::pad::x25519_padding_construct` is deterministic
    /// once the seed is supplied. Same seed → same bytes; different
    /// seed → different bytes.
    #[test]
    fn deterministic_with_seeded_rng() {
        let a = pad_to_tier_with_rng(7, &mut ChaCha20Rng::seed_from_u64(42)).unwrap();
        let b = pad_to_tier_with_rng(7, &mut ChaCha20Rng::seed_from_u64(42)).unwrap();
        assert_eq!(a, b, "identical seeds must produce identical padding");

        let c = pad_to_tier_with_rng(7, &mut ChaCha20Rng::seed_from_u64(43)).unwrap();
        assert_ne!(a, c, "different seeds must produce different padding");
    }

    #[test]
    fn overflow_surfaces_from_pad_fn() {
        let err = pad_to_tier_with_rng(1001, &mut ChaCha20Rng::seed_from_u64(0)).unwrap_err();
        assert_eq!(err, PadError::Overflow { got: 1001 });
    }

    /// OsRng path smoke-tests (can't be seeded, so we only check shape
    /// + size). Keeps the effectful wrapper covered so a future refactor
    /// doesn't silently break it.
    #[test]
    fn os_rng_wrapper_produces_tier_sized_list() {
        let padding = pad_to_tier(5).unwrap();
        assert_eq!(padding.len(), 5); // tier 10 − 5 real
    }
}
