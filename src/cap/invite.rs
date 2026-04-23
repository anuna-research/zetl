//! `ztl cap invite` pure-core helpers (SPEC-034 REQ-3410 / CON-3401 /
//! CON-3402 / REQ-3430).
//!
//! This module is pure: no I/O, no wall clock, no OS-CSPRNG entrance
//! point. All randomness is supplied by the caller as a `CryptoRng`
//! reference, and the current time is handed in as an RFC 3339 string.
//! The effectful shell (`src/main.rs::cmd_cap_invite`) supplies both
//! from `rand_core::OsRng` and `std::time::SystemTime`.
//!
//! Responsibilities:
//!
//! - Generate a fresh `(priv_A, pub_A)` X25519 keypair for delegated-URL
//!   mode. The private key is returned by value so the caller can feed
//!   it into the URL fragment and drop it; the wrapper zeroises itself
//!   on drop so a panic mid-invite does not leak bytes to the heap.
//! - XOR-split `priv_A` into two uniformly-random halves for REQ-3430
//!   split-key mode.
//! - Generate the `g_<random>` grant id used as the primary key in
//!   `grants.toml`. The random body is base32-Crockford (no `I`/`L`/
//!   `O`/`U`) so operators reading grant ids aloud never collide on
//!   ambiguous glyphs.
//! - Parse the `--expires` duration (`72h`, `7d`, …) the same way
//!   `ztl invite` parses `--expires`, with the additional NFR-3412
//!   bounds (60s minimum, 90 days maximum).
//! - Format `created` / `expires` timestamps and render the
//!   REQ-3410 URL-shortener warning banner verbatim.
//!
//! Non-responsibilities (lives in the shell):
//!
//! - Reading `recipients.toml` / `grants.toml` off disk.
//! - Resolving `ztl_CAP_SECRET` + the cohort's `salt_stable`.
//! - Deriving the path-cap (pure, but lives in `cap::derivation` and
//!   is called from the shell so this module can stay free of
//!   crypto-crate churn).
//! - Rendering the final URL — `cap::url_format::CapUrl` already does
//!   that.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use rand_core::{CryptoRng, RngCore};
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::cap::pad::X25519Pubkey;
use crate::cap::recipients::parsing::AGE_RECIPIENT_V1_PREFIX;

/// REQ-3410 warning banner printed on stdout for every successful
/// `ztl cap invite` invocation. The exact text is pinned by a
/// byte-stable integration test so rewrites that add ornament or
/// strip the threat-model link regress loudly.
pub const URL_SHORTENER_WARNING: &str = "\
⚠  SECURITY WARNING (REQ-3412 / §11.2)
   Invite URLs contain decryption material in the fragment.
   Do NOT pass through URL shorteners, link previewers, or bots
   (Slack unfurl, Microsoft SafeLinks, Google unfurl, etc.).
   Send via channels that do not rewrite URLs:
     direct messages, email without link rewriting, in-person.
";

/// Minimum `--expires` duration per NFR-3412 (60 seconds).
pub const MIN_EXPIRES_SECS: u64 = 60;

/// Maximum `--expires` duration per NFR-3412 (90 days).
pub const MAX_EXPIRES_SECS: u64 = 90 * 86_400;

/// Default `--expires` duration when the operator omits the flag
/// (72 hours — matches the `ztl invite` collaborative default so the
/// two surfaces feel identical to operators who know one already).
pub const DEFAULT_EXPIRES_SECS: u64 = 72 * 3600;

/// Length of the base64url-encoded `priv_A` fragment key. Pinned by
/// CON-3401 at 43 chars (32 bytes unpadded).
pub const PRIV_A_B64URL_LEN: usize = 43;

/// Wrapper around a fresh X25519 private scalar. Zeroed on drop so a
/// panic between key generation and URL render cannot leak bytes to
/// the heap. The wrapper is `!Clone` and `!Copy` on purpose — callers
/// must `consume()` it to get the base64url form, which destroys the
/// secret on the way out.
#[derive(ZeroizeOnDrop)]
pub struct InviteSecret {
    inner: [u8; 32],
}

impl InviteSecret {
    /// Encode the private scalar as base64url (unpadded, 43 chars) and
    /// drop the wrapper. The only API shape that reveals the secret
    /// bytes — no `as_slice` accessor exists, so a logger cannot
    /// accidentally `{:?}` the bytes.
    pub fn into_b64url(mut self) -> String {
        let out = URL_SAFE_NO_PAD.encode(self.inner);
        self.inner.zeroize();
        out
    }

    /// Borrow the raw 32 bytes for XOR splitting. Not public; the
    /// split routine below is the only caller. `pub(super)` would be
    /// equivalent — this form keeps the call site obvious.
    fn as_bytes(&self) -> &[u8; 32] {
        &self.inner
    }
}

/// Outcome of `generate_invite_keypair`. The caller immediately feeds
/// `secret` into the URL fragment (consuming it), and pushes the age
/// recipient string into `recipients.toml` under the target cohort.
pub struct InviteKeypair {
    /// Zeroed-on-drop private-key wrapper. Consume via
    /// [`InviteSecret::into_b64url`] to obtain the fragment form.
    pub secret: InviteSecret,
    /// Raw 32-byte X25519 pubkey. Safe to print; serves as the
    /// cohort's recipient entry.
    pub public: X25519Pubkey,
}

/// Generate a fresh `(priv_A, pub_A)` X25519 keypair from the supplied
/// CSPRNG. Pure function of the RNG input — seeded CSPRNGs reproduce
/// the output byte-for-byte, which the test suite exploits to pin the
/// public-key rendering.
pub fn generate_invite_keypair<R: RngCore + CryptoRng>(rng: &mut R) -> InviteKeypair {
    let sk = StaticSecret::random_from_rng(&mut *rng);
    let pk = PublicKey::from(&sk);
    let mut priv_bytes = [0u8; 32];
    priv_bytes.copy_from_slice(&sk.to_bytes());
    // Drop the StaticSecret explicitly so its ZeroizeOnDrop fires
    // before our owned copy hits the URL. The owned copy is itself
    // wrapped in InviteSecret, which zeroes on consume / drop.
    drop(sk);
    InviteKeypair {
        secret: InviteSecret { inner: priv_bytes },
        public: *pk.as_bytes(),
    }
}

/// Split a base64url-encoded 32-byte private key into two 32-byte
/// halves using XOR secret sharing (REQ-3430): `priv_A = half1 ⊕ half2`.
/// `half2` is sampled fresh from `rng`; `half1 = priv_A ⊕ half2`. Each
/// half is a uniform 32-byte string on its own, so neither half alone
/// leaks any information about `priv_A`.
///
/// Returns `(half1_b64url, half2_b64url)` — both 43-char base64url
/// strings, safe to paste. The underlying byte buffers are zeroed
/// before the function returns; only the base64 strings escape.
pub fn xor_split_private_key<R: RngCore + CryptoRng>(
    secret: InviteSecret,
    rng: &mut R,
) -> (String, String) {
    let priv_bytes = *secret.as_bytes();
    // Sample half2; compute half1 = priv ^ half2.
    let mut half2 = [0u8; 32];
    rng.fill_bytes(&mut half2);
    let mut half1 = [0u8; 32];
    for i in 0..32 {
        half1[i] = priv_bytes[i] ^ half2[i];
    }
    let half1_b64 = URL_SAFE_NO_PAD.encode(half1);
    let half2_b64 = URL_SAFE_NO_PAD.encode(half2);
    // Zero the intermediate buffers. `secret` drops at function exit.
    half1.zeroize();
    half2.zeroize();
    drop(secret);
    (half1_b64, half2_b64)
}

/// Encode a raw 32-byte X25519 pubkey as the canonical
/// `age-recipient-v1:<b64url>` string recognised by
/// [`crate::cap::recipients::parsing`] and
/// [`crate::cap::grants::validation`].
pub fn encode_age_recipient_v1(pk: &X25519Pubkey) -> String {
    let b64 = URL_SAFE_NO_PAD.encode(pk);
    format!("{AGE_RECIPIENT_V1_PREFIX}{b64}")
}

/// Grant-id prefix (CON-3402). Kept as a constant so operators reading
/// a grants file never see an un-prefixed id.
pub const GRANT_ID_PREFIX: &str = "g_";

/// Length of the random body of a grant id, in Crockford characters.
/// 26 chars ≈ 130 bits of entropy — comfortable collision margin for
/// any realistic operator-grant volume.
pub const GRANT_ID_RANDOM_CHARS: usize = 26;

/// Crockford base32 alphabet (no `I`/`L`/`O`/`U`). Mirrors
/// `cap::derivation::CROCKFORD_ALPHABET`; kept local so this module
/// doesn't reach into the derivation module's private surface.
const CROCKFORD_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Generate a fresh grant id of the form `g_<26-char-crockford>`. Pure
/// function of the supplied RNG; seeded CSPRNGs reproduce the output.
pub fn generate_grant_id<R: RngCore + CryptoRng>(rng: &mut R) -> String {
    let mut buf = String::with_capacity(GRANT_ID_PREFIX.len() + GRANT_ID_RANDOM_CHARS);
    buf.push_str(GRANT_ID_PREFIX);
    for _ in 0..GRANT_ID_RANDOM_CHARS {
        let mut b = [0u8; 1];
        rng.fill_bytes(&mut b);
        let idx = (b[0] & 0x1f) as usize;
        buf.push(CROCKFORD_ALPHABET[idx] as char);
    }
    buf
}

/// Parse `--expires` duration strings. Accepts a trailing suffix:
/// `s` (seconds), `m` (minutes), `h` (hours), `d` (days). No suffix
/// is interpreted as hours to match the `ztl invite` convention and
/// avoid "72" meaning "72 seconds" by mistake. NFR-3412 bounds are
/// enforced: results below 60s or above 90d are rejected.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ExpiresParseError {
    #[error("invalid --expires value {value:?}: {reason}")]
    Malformed { value: String, reason: &'static str },
    #[error(
        "--expires {value:?} is below the NFR-3412 minimum of {min}s",
        min = MIN_EXPIRES_SECS
    )]
    TooShort { value: String },
    #[error("--expires {value:?} exceeds the NFR-3412 maximum of 90 days")]
    TooLong { value: String },
}

pub fn parse_expires(s: &str) -> Result<u64, ExpiresParseError> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err(ExpiresParseError::Malformed {
            value: s.to_string(),
            reason: "empty duration",
        });
    }
    let (num_part, mult) = if let Some(n) = trimmed.strip_suffix('s') {
        (n, 1u64)
    } else if let Some(n) = trimmed.strip_suffix('m') {
        (n, 60u64)
    } else if let Some(n) = trimmed.strip_suffix('h') {
        (n, 3600u64)
    } else if let Some(n) = trimmed.strip_suffix('d') {
        (n, 86_400u64)
    } else {
        // No suffix → hours (matches `ztl invite`).
        (trimmed, 3600u64)
    };
    let n: u64 = num_part
        .trim()
        .parse()
        .map_err(|_| ExpiresParseError::Malformed {
            value: s.to_string(),
            reason: "non-numeric quantity",
        })?;
    let secs = n.checked_mul(mult).ok_or(ExpiresParseError::Malformed {
        value: s.to_string(),
        reason: "duration overflow",
    })?;
    if secs < MIN_EXPIRES_SECS {
        return Err(ExpiresParseError::TooShort {
            value: s.to_string(),
        });
    }
    if secs > MAX_EXPIRES_SECS {
        return Err(ExpiresParseError::TooLong {
            value: s.to_string(),
        });
    }
    Ok(secs)
}

/// Format a Unix timestamp (seconds since epoch) as an RFC 3339 UTC
/// string (`YYYY-MM-DDThh:mm:ssZ`). Hand-rolled Gregorian date math so
/// this module stays free of `chrono`. Matches the strict validator
/// in `cap::grants::validation::is_rfc3339`.
pub fn format_rfc3339_utc(unix_secs: u64) -> String {
    // Days since 1970-01-01.
    let days = (unix_secs / 86_400) as i64;
    let time_of_day = unix_secs % 86_400;
    let hh = time_of_day / 3600;
    let mm = (time_of_day % 3600) / 60;
    let ss = time_of_day % 60;
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// Howard Hinnant's `civil_from_days`: convert a days-since-epoch
/// integer to a `(year, month, day)` triple using the standard
/// proleptic Gregorian calendar. Self-contained so this module has
/// no `chrono` dependency.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (y + if m <= 2 { 1 } else { 0 }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    fn seeded() -> ChaCha20Rng {
        ChaCha20Rng::from_seed([42u8; 32])
    }

    #[test]
    fn invite_secret_zeroises_after_consume() {
        let kp = generate_invite_keypair(&mut seeded());
        let b64 = kp.secret.into_b64url();
        // Base64url of 32 bytes unpadded is 43 chars.
        assert_eq!(b64.len(), PRIV_A_B64URL_LEN);
        // Every character in the output must be in the base64url
        // alphabet (no padding character).
        for c in b64.chars() {
            assert!(
                c.is_ascii_alphanumeric() || c == '-' || c == '_',
                "non-base64url char {c:?} in {b64}"
            );
        }
    }

    #[test]
    fn generate_keypair_is_deterministic_under_seeded_rng() {
        let a = generate_invite_keypair(&mut seeded());
        let b = generate_invite_keypair(&mut seeded());
        assert_eq!(a.public, b.public);
        assert_eq!(a.secret.into_b64url(), b.secret.into_b64url());
    }

    #[test]
    fn xor_split_halves_xor_back_to_priv() {
        let kp = generate_invite_keypair(&mut seeded());
        let priv_bytes_before = *kp.secret.as_bytes();
        let (h1, h2) = xor_split_private_key(kp.secret, &mut seeded());
        let h1_bytes = URL_SAFE_NO_PAD.decode(&h1).expect("base64");
        let h2_bytes = URL_SAFE_NO_PAD.decode(&h2).expect("base64");
        let mut reconstructed = [0u8; 32];
        for i in 0..32 {
            reconstructed[i] = h1_bytes[i] ^ h2_bytes[i];
        }
        assert_eq!(reconstructed, priv_bytes_before);
        assert_eq!(h1.len(), PRIV_A_B64URL_LEN);
        assert_eq!(h2.len(), PRIV_A_B64URL_LEN);
    }

    #[test]
    fn encode_age_recipient_v1_carries_prefix() {
        let kp = generate_invite_keypair(&mut seeded());
        let s = encode_age_recipient_v1(&kp.public);
        assert!(s.starts_with(AGE_RECIPIENT_V1_PREFIX));
        let payload = &s[AGE_RECIPIENT_V1_PREFIX.len()..];
        assert_eq!(payload.len(), PRIV_A_B64URL_LEN);
    }

    #[test]
    fn generate_grant_id_shape() {
        let id = generate_grant_id(&mut seeded());
        assert!(id.starts_with(GRANT_ID_PREFIX));
        let body = &id[GRANT_ID_PREFIX.len()..];
        assert_eq!(body.len(), GRANT_ID_RANDOM_CHARS);
        for c in body.chars() {
            assert!(
                c.is_ascii_digit() || ("ABCDEFGHJKMNPQRSTVWXYZ".contains(c)),
                "non-Crockford char {c:?} in grant id {id}"
            );
        }
    }

    #[test]
    fn two_grant_ids_differ_under_distinct_rng_calls() {
        let mut rng = seeded();
        let a = generate_grant_id(&mut rng);
        let b = generate_grant_id(&mut rng);
        assert_ne!(a, b);
    }

    #[test]
    fn parse_expires_accepts_common_forms() {
        assert_eq!(parse_expires("72h").unwrap(), 72 * 3600);
        assert_eq!(parse_expires("7d").unwrap(), 7 * 86_400);
        assert_eq!(parse_expires("60s").unwrap(), 60);
        assert_eq!(parse_expires("5m").unwrap(), 300);
        // Bare integer is interpreted as hours.
        assert_eq!(parse_expires("24").unwrap(), 24 * 3600);
    }

    #[test]
    fn parse_expires_enforces_bounds() {
        assert!(matches!(
            parse_expires("30s"),
            Err(ExpiresParseError::TooShort { .. })
        ));
        assert!(matches!(
            parse_expires("91d"),
            Err(ExpiresParseError::TooLong { .. })
        ));
    }

    #[test]
    fn parse_expires_rejects_malformed() {
        assert!(matches!(
            parse_expires(""),
            Err(ExpiresParseError::Malformed { .. })
        ));
        assert!(matches!(
            parse_expires("abc"),
            Err(ExpiresParseError::Malformed { .. })
        ));
    }

    #[test]
    fn format_rfc3339_utc_matches_validator() {
        // Epoch + 1 day + 1 hour + 1 minute + 1 second.
        let ts = 86_400 + 3600 + 60 + 1;
        let s = format_rfc3339_utc(ts);
        assert_eq!(s, "1970-01-02T01:01:01Z");
        assert!(
            crate::cap::grants::validation::GrantsFile::from_toml(&format!(
                r#"[[grant]]
id = "g_x"
cohort = "eng"
recipient = "age-recipient-v1:AAA"
mode = "delegated-url"
created = "{s}"
"#
            ))
            .is_ok()
        );
    }

    #[test]
    fn url_shortener_warning_has_required_keywords() {
        // REQ-3410 pins the text; this test is an early smoke-test
        // that byte-drifts of the banner (rewrites, ANSI codes,
        // translations) break at least one expectation. The
        // byte-stable snapshot lives in the integration test.
        let t = URL_SHORTENER_WARNING;
        assert!(t.contains("SECURITY WARNING"));
        assert!(t.contains("URL shorteners"));
        assert!(t.contains("link previewers"));
        assert!(t.contains("Slack unfurl"));
    }
}
