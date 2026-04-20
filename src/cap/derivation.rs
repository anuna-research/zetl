//! Pure-core HKDF derivations for capability-mode builds (SPEC-034 §8).
//!
//! Deterministic, no I/O, no randomness, no wall clock. The only inputs
//! are caller-supplied byte strings; identical inputs always produce
//! identical outputs. These functions are the cryptographic heart of
//! CON-3401 (path-cap derivation), CON-3409 (TOFU-wrap-key derivation),
//! and the hardened-mode identity-seed derivation recommended by the
//! Tier-2 review (S1-03).
//!
//! All key-separation strings use an explicit `info` parameter so the
//! same input-keying-material can never be reused across purposes —
//! NFR-3403 "KDF discipline." The stable-salt/rotatable-salt split is
//! CON-3401's: changing `cohort_salt_stable` rotates every path-cap in
//! the cohort; changing only the content-key salt leaves URLs stable
//! (BUG-023 resolution).
//!
//! # Crockford base32
//!
//! Path-caps are rendered in Crockford base32 (alphabet
//! `0123456789ABCDEFGHJKMNPQRSTVWXYZ`, no `I`/`L`/`O`/`U`), per CON-3401.
//! Encoding is big-endian 5-bits-at-a-time; any tail bits short of a
//! full 5-bit group are padded with zero bits (there is no `=`
//! padding character). For a 64-bit path-cap the result is 13 characters;
//! for 48 bits it is 10 characters; for 128 bits it is 26 characters.

use hkdf::Hkdf;
use sha2::Sha256;

/// The smallest path-cap width permitted by NFR-3401 (48 bits).
pub const PATH_CAP_MIN_BITS: u32 = 48;
/// The default path-cap width per REQ-3402 (64 bits).
pub const PATH_CAP_DEFAULT_BITS: u32 = 64;
/// The largest path-cap width permitted by REQ-3402 (128 bits).
pub const PATH_CAP_MAX_BITS: u32 = 128;

/// Fixed HKDF `info` prefix for path-cap derivation. CON-3401 pins the
/// exact string, including the version marker; any change to this
/// constant is a wire-format break.
pub const PATH_CAP_INFO_PREFIX: &str = "zetl/path-cap/v1/";

/// Fixed HKDF `info` for the TOFU wrap-key (CON-3409).
pub const TOFU_WRAP_INFO: &[u8] = b"zetl/tofu-wrap/v1";

/// Fixed HKDF `info` for the hardened-mode X25519 identity seed
/// (Tier-2 review S1-03 recommendation).
pub const HARDENED_IDENTITY_INFO: &[u8] = b"zetl/hardened-identity/v1";

/// Errors returned by the derivation routines. The enum is
/// intentionally small: the only way to misuse a pure HKDF routine is
/// to pass an out-of-range length or an invalid path-cap width.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DerivationError {
    /// `path_cap_bits` was outside [PATH_CAP_MIN_BITS, PATH_CAP_MAX_BITS]
    /// or was not a multiple of 8. CON-3401 takes a byte-granular
    /// truncation, so non-byte widths are rejected up-front.
    #[error(
        "path-cap width {bits} bits out of range; must be a multiple of 8 in [{min}, {max}]",
        min = PATH_CAP_MIN_BITS,
        max = PATH_CAP_MAX_BITS
    )]
    PathCapWidth { bits: u32 },
    /// HKDF-Expand was asked for more than 255 * HashLen bytes (RFC 5869
    /// §2.3). Not reachable with the fixed derivations below but
    /// defensive callers may want the typed error.
    #[error("HKDF expand length {0} exceeds 255 * 32")]
    ExpandTooLong(usize),
}

/// CON-3401 path-cap derivation.
///
/// `cohort_secret` is `ZETL_CAP_SECRET` (48 bytes per REQ-3419, but the
/// derivation does not care about length — HKDF-Extract absorbs any
/// IKM). `cohort_salt_stable` is the cohort's stable path-salt; it is
/// held separate from the content-key salt so rotating content keys
/// does not change URLs (BUG-023). `path_cap_bits` picks the output
/// width from [48, 128] bits in 8-bit steps; 64 is the REQ-3402 default.
///
/// The returned string is Crockford base32 of the truncated HKDF
/// output; length is `ceil(path_cap_bits / 5)` characters.
pub fn derive_path_cap(
    cohort_secret: &[u8],
    cohort_salt_stable: &[u8],
    cohort_id: &str,
    slug: &str,
    path_cap_bits: u32,
) -> Result<String, DerivationError> {
    if !(PATH_CAP_MIN_BITS..=PATH_CAP_MAX_BITS).contains(&path_cap_bits)
        || path_cap_bits % 8 != 0
    {
        return Err(DerivationError::PathCapWidth { bits: path_cap_bits });
    }
    let byte_len = (path_cap_bits / 8) as usize;

    // Build the info string: "zetl/path-cap/v1/" || cohort_id || "/" || slug
    // matches CON-3401 exactly. Callers that pass slashes or other
    // delimiters in cohort_id/slug are responsible for escaping upstream;
    // the validator in `cap::grants::validation` restricts cohort_id to
    // `[a-z0-9][a-z0-9-]*` so no ambiguity enters here.
    let mut info = Vec::with_capacity(
        PATH_CAP_INFO_PREFIX.len() + cohort_id.len() + 1 + slug.len(),
    );
    info.extend_from_slice(PATH_CAP_INFO_PREFIX.as_bytes());
    info.extend_from_slice(cohort_id.as_bytes());
    info.push(b'/');
    info.extend_from_slice(slug.as_bytes());

    let hk = Hkdf::<Sha256>::new(Some(cohort_salt_stable), cohort_secret);
    let mut okm = [0u8; 32];
    hk.expand(&info, &mut okm)
        .expect("HKDF expand of 32 bytes never fails under SHA-256");

    Ok(crockford_encode(&okm[..byte_len]))
}

/// CON-3409 TOFU wrap-key derivation: `HKDF-SHA256(prf_output, "",
/// "zetl/tofu-wrap/v1", 32)`.
///
/// The salt is empty (HKDF treats the empty salt as `[0u8; 32]`) and
/// the output length is fixed at 32 bytes so the caller can feed the
/// result straight into AES-256-GCM. This function exists in the pure
/// core so the shim's reference vector and the Rust-side test harness
/// share a bit-exact implementation — the JS/TS shim re-implements the
/// same composition and must match our output byte-for-byte.
pub fn derive_tofu_wrap_key(prf_output: &[u8]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(None, prf_output);
    let mut okm = [0u8; 32];
    hk.expand(TOFU_WRAP_INFO, &mut okm)
        .expect("HKDF expand of 32 bytes never fails under SHA-256");
    okm
}

/// Hardened-mode X25519 identity-seed derivation, per Tier-2 review
/// S1-03: `HKDF-SHA256(prf_output, "", "zetl/hardened-identity/v1",
/// 32)`.
///
/// The result is a 32-byte seed that becomes an X25519 private key
/// after standard RFC 7748 clamping (applied by the `age` X25519
/// recipient routines in the effectful shell — the pure core does not
/// do curve arithmetic). Kept distinct from the TOFU wrap-key by an
/// `info` string so the same PRF output can serve both roles without
/// reusing key material.
pub fn derive_hardened_identity_seed(prf_output: &[u8]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(None, prf_output);
    let mut okm = [0u8; 32];
    hk.expand(HARDENED_IDENTITY_INFO, &mut okm)
        .expect("HKDF expand of 32 bytes never fails under SHA-256");
    okm
}

/// Crockford base32 alphabet. Excludes `I`, `L`, `O`, `U` to avoid
/// transcription confusion. Big-endian bit-order; no `=` padding.
const CROCKFORD_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

fn crockford_encode(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    let total_bits = bytes.len() * 8;
    let char_count = total_bits.div_ceil(5);
    let mut out = String::with_capacity(char_count);

    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    for &b in bytes {
        buf = (buf << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            let idx = ((buf >> bits) & 0x1f) as usize;
            out.push(CROCKFORD_ALPHABET[idx] as char);
        }
    }
    if bits > 0 {
        let idx = ((buf << (5 - bits)) & 0x1f) as usize;
        out.push(CROCKFORD_ALPHABET[idx] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_path_cap_deterministic() {
        let a = derive_path_cap(b"secret", b"salt", "eng", "page/a", 64).unwrap();
        let b = derive_path_cap(b"secret", b"salt", "eng", "page/a", 64).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn derive_path_cap_changes_with_slug() {
        let a = derive_path_cap(b"s", b"sa", "eng", "page/a", 64).unwrap();
        let b = derive_path_cap(b"s", b"sa", "eng", "page/b", 64).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn derive_path_cap_changes_with_cohort() {
        let a = derive_path_cap(b"s", b"sa", "eng", "page/a", 64).unwrap();
        let b = derive_path_cap(b"s", b"sa", "ops", "page/a", 64).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn derive_path_cap_changes_with_secret() {
        let a = derive_path_cap(b"secret1", b"sa", "eng", "a", 64).unwrap();
        let b = derive_path_cap(b"secret2", b"sa", "eng", "a", 64).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn derive_path_cap_changes_with_stable_salt() {
        let a = derive_path_cap(b"s", b"salt1", "eng", "a", 64).unwrap();
        let b = derive_path_cap(b"s", b"salt2", "eng", "a", 64).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn derive_path_cap_widths_match_expected_length() {
        // 48 bits = 6 bytes → ceil(48/5) = 10 chars
        let a = derive_path_cap(b"s", b"sa", "eng", "a", 48).unwrap();
        assert_eq!(a.len(), 10);
        // 64 bits = 8 bytes → ceil(64/5) = 13 chars
        let b = derive_path_cap(b"s", b"sa", "eng", "a", 64).unwrap();
        assert_eq!(b.len(), 13);
        // 128 bits = 16 bytes → ceil(128/5) = 26 chars
        let c = derive_path_cap(b"s", b"sa", "eng", "a", 128).unwrap();
        assert_eq!(c.len(), 26);
    }

    #[test]
    fn derive_path_cap_rejects_bad_width() {
        assert!(matches!(
            derive_path_cap(b"s", b"sa", "eng", "a", 47),
            Err(DerivationError::PathCapWidth { bits: 47 })
        ));
        assert!(matches!(
            derive_path_cap(b"s", b"sa", "eng", "a", 129),
            Err(DerivationError::PathCapWidth { bits: 129 })
        ));
        assert!(matches!(
            derive_path_cap(b"s", b"sa", "eng", "a", 60),
            Err(DerivationError::PathCapWidth { bits: 60 })
        ));
    }

    #[test]
    fn derive_path_cap_output_uses_only_crockford_alphabet() {
        let out = derive_path_cap(b"s", b"sa", "eng", "a", 64).unwrap();
        for c in out.chars() {
            assert!(
                CROCKFORD_ALPHABET.contains(&(c as u8)),
                "char {c:?} not in Crockford alphabet"
            );
        }
    }

    #[test]
    fn wrap_key_deterministic_and_distinct_from_identity() {
        let a = derive_tofu_wrap_key(b"prf_output");
        let b = derive_tofu_wrap_key(b"prf_output");
        assert_eq!(a, b);
        let c = derive_hardened_identity_seed(b"prf_output");
        assert_ne!(a, c, "info-string separation must force distinct outputs");
    }

    #[test]
    fn wrap_key_changes_with_prf_output() {
        let a = derive_tofu_wrap_key(b"prf1");
        let b = derive_tofu_wrap_key(b"prf2");
        assert_ne!(a, b);
    }

    #[test]
    fn hardened_identity_seed_changes_with_prf_output() {
        let a = derive_hardened_identity_seed(b"prf1");
        let b = derive_hardened_identity_seed(b"prf2");
        assert_ne!(a, b);
    }

    #[test]
    fn crockford_rfc_like_vectors() {
        // 0xFF = 11111111 → "ZW" (111_11 + 111_00)
        assert_eq!(crockford_encode(&[0xff]), "ZW");
        // 0x00 = 00000000 → "00" (0 + 0-padded trailing)
        assert_eq!(crockford_encode(&[0x00]), "00");
        // "Hello World" → fixed output; just assert non-empty + alphabet
        let s = crockford_encode(b"Hello World");
        assert!(!s.is_empty());
        assert!(s.chars().all(|c| CROCKFORD_ALPHABET.contains(&(c as u8))));
        // Empty input → empty output
        assert_eq!(crockford_encode(&[]), "");
    }

    #[test]
    fn hkdf_matches_rfc_5869_test_case_1() {
        // RFC 5869 Appendix A.1 test vector: SHA-256, 22-byte IKM,
        // 13-byte salt, 10-byte info, 42-byte OKM. We exercise the
        // same underlying hkdf crate used throughout this module so
        // any future version bump that changes semantics is caught.
        let ikm = unhex("0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b");
        let salt = unhex("000102030405060708090a0b0c");
        let info = unhex("f0f1f2f3f4f5f6f7f8f9");
        let expected_okm = unhex(
            "3cb25f25faacd57a90434f64d0362f2a\
             2d2d0a90cf1a5a4c5db02d56ecc4c5bf\
             34007208d5b887185865",
        );
        let hk = Hkdf::<Sha256>::new(Some(&salt), &ikm);
        let mut okm = [0u8; 42];
        hk.expand(&info, &mut okm).unwrap();
        assert_eq!(&okm[..], &expected_okm[..]);
    }

    fn unhex(s: &str) -> Vec<u8> {
        let filtered: String = s.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(filtered.len() % 2 == 0, "odd-length hex string");
        (0..filtered.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&filtered[i..i + 2], 16).unwrap())
            .collect()
    }
}
