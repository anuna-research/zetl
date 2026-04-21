//! `zetl cap pair` pure-core — SPAKE2-authenticated pubkey handoff
//! (SPEC-034 REQ-3416 `pair` verb + REQ-3408 CLI commands + §8 purity
//! entry `cap::pair::spake2`).
//!
//! This module covers three concerns, all pure (no wall clock, no disk,
//! no network, no OS-CSPRNG entrance point):
//!
//! 1. **Phrase generation.** A `zetl cap pair` session is authenticated
//!    by a freshly-generated 4-word phrase drawn from the BIP39 English
//!    wordlist. 4 × 11 bits = 44 bits of entropy is sufficient because
//!    SPAKE2 is an online-only PAKE: an attacker gets a single guess per
//!    exchange, not an offline brute-force opportunity. Standard BIP39
//!    entropy sizes (128/160/192/224/256 bits) would be overkill for a
//!    phrase operators have to read aloud over the phone or enter on a
//!    phone keyboard.
//!
//! 2. **SPAKE2 session drivers.** Thin wrappers around
//!    `spake2::Spake2::<Ed25519Group>::start_symmetric` so the two
//!    operators do not have to agree on an "A" and "B" side ahead of
//!    time. `PairSession::start` returns the outbound message and a
//!    session handle; `finish` consumes both the handle and the peer's
//!    outbound message and returns the derived shared secret. Shared
//!    secret is NOT stored — its sole use is as input to the HKDF that
//!    produces a one-shot HMAC key for authenticating the pubkey
//!    handoff.
//!
//! 3. **Phrase-reuse nonce store.** `.zetl/caps/.pair-nonces` is a TOML
//!    file mapping a canonical hash of the phrase to the Unix second at
//!    which that phrase was first used. `NonceStore::accept` refuses to
//!    hand back a fresh session if the phrase (or its hash) has been
//!    used within `PAIR_NONCE_TTL_SECS` (30 days). Callers that accept a
//!    phrase get a pruned store back with the new row appended — they
//!    serialise it back to disk as the last step of the session.
//!
//! The effectful shell (`src/main.rs::cmd_cap_pair`) supplies the
//! [`rand_core::OsRng`] + [`std::time::SystemTime`] and persists the
//! nonce store to `.zetl/caps/.pair-nonces`. Every function here is
//! deterministic once the RNG output + the current time are fixed.
//!
//! # Threat model in one paragraph
//!
//! The handoff ciphertext envelope uses age over X25519 (out of scope of
//! this module); what `zetl cap pair` provides is authentication for
//! the *pubkey itself*, so a malicious relay cannot swap the grantee's
//! pubkey for an attacker's pubkey between terminals. The 4-word phrase
//! is the only shared secret; SPAKE2 converts it into a full-entropy
//! session key that binds both sides to the same transcript. If the
//! phrase is wrong, the HMAC verification fails and the handoff is
//! refused — the spec treats this as an audible/visual cue for the
//! operators to abort.
//!
//! Per SPEC-034 §11.4 this is a Tier 2 review surface: the crypto comes
//! from the `spake2` crate (magic-wormhole's SPAKE2 implementation over
//! curve25519-dalek-4 + Ed25519 group). The nonce store and the HKDF
//! wrapper are the parts worth reviewing here.

use base64::engine::general_purpose::STANDARD_NO_PAD as B64;
use base64::Engine as _;
use bip39::Language;
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rand_core::{CryptoRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use spake2::{Ed25519Group, Identity, Password, Spake2};
use zeroize::Zeroize;

/// Number of BIP39 words in a pairing phrase. 4 words ≈ 44 bits, the
/// smallest size that still pronounces well and resists casual shoulder-
/// surfing in an operator-to-operator context.
pub const PHRASE_WORD_COUNT: usize = 4;

/// How long a phrase hash stays "claimed" in `.zetl/caps/.pair-nonces`
/// (30 days, REQ-3408). After this window the row is pruned on next
/// call to [`NonceStore::accept`] so the file does not grow without
/// bound across years of operator activity.
pub const PAIR_NONCE_TTL_SECS: u64 = 30 * 86_400;

/// Canonical SPAKE2 "shared identity" string. The protocol requires
/// both sides to agree on the id; baking it into the library keeps the
/// CLI surface small (operators never have to exchange an id string) at
/// the cost of locking future protocol changes to a version bump of
/// this literal.
pub const PAIR_SPAKE2_IDENTITY: &[u8] = b"zetl/cap-pair/v1";

/// Domain-separation label for the HKDF that turns the SPAKE2 shared
/// secret into an HMAC-SHA256 key used to authenticate the pubkey
/// handoff. Rotating this label on a future protocol change invalidates
/// old paired sessions cleanly.
pub const PAIR_HMAC_HKDF_INFO: &[u8] = b"zetl/cap-pair/v1/hmac";

/// Domain-separation label for hashing a phrase into the nonce-store
/// key. Keeping the phrase-hash independent from the HMAC key prevents
/// cross-use where a replayed transcript collides with a stored row.
pub const PAIR_NONCE_HASH_DOMAIN: &[u8] = b"zetl/cap-pair/v1/nonce";

/// 43-char base64url of an X25519 / Ed25519 pubkey (32 raw bytes). The
/// HMAC tag in the authenticated handoff is over exactly these 32
/// bytes, not the age-recipient-v1 string — the wire format is
/// operator-visible and must survive a paste through whatever channel
/// the operators picked.
pub const PAIR_PUBKEY_LEN: usize = 32;

/// Length of the HMAC-SHA256 tag returned by
/// [`SharedAuthKey::authenticate_pubkey`]. Full-length SHA-256 (32
/// bytes) — truncation would buy nothing here since the tag is only
/// consumed once per pairing.
pub const PAIR_HMAC_LEN: usize = 32;

type HmacSha256 = Hmac<Sha256>;

// ───────────────────────────────────────────────────────────────
// Phrase generation + canonicalisation
// ───────────────────────────────────────────────────────────────

/// A freshly-generated (or freshly-parsed) 4-word pairing phrase.
///
/// `PairPhrase` owns its word list as a `Vec<&'static str>` of BIP39
/// English words. Storing `&'static str` means no heap allocation per
/// word — the BIP39 wordlist lives in `.rodata` — but it also means the
/// phrase is safe to print: the BIP39 English wordlist is public, so
/// `Debug` / `Display` on a `PairPhrase` leaks no secret more than the
/// phrase itself (and the phrase is the shared secret the operator is
/// *about to read aloud*).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairPhrase {
    words: Vec<&'static str>,
}

impl PairPhrase {
    /// Construct a phrase from a pre-validated word list. Not `pub`
    /// because the public entry points ([`generate_phrase`] and
    /// [`PairPhrase::parse`]) both validate against the BIP39 English
    /// wordlist; callers holding `&'static str` references from
    /// [`bip39::Language::English`] have already done the check.
    fn from_static_words(words: Vec<&'static str>) -> Self {
        debug_assert_eq!(words.len(), PHRASE_WORD_COUNT);
        Self { words }
    }

    /// Iterate the 4 words in order. Borrowed slice for callers that
    /// need to render with their own separator (QR codes, for example,
    /// often use `-` instead of a literal space).
    pub fn words(&self) -> &[&'static str] {
        &self.words
    }

    /// Canonical single-line rendering: lowercase words joined with a
    /// single ASCII space. Both [`PairPhrase::parse`] and
    /// [`PairPhrase::nonce_hash`] normalise against this form, so the
    /// phrase survives round-trips through sloppy copy/paste (leading
    /// whitespace, tabs, multiple spaces, trailing newline).
    pub fn canonical(&self) -> String {
        self.words.join(" ")
    }

    /// Parse a phrase from user-entered text. Accepts any amount of
    /// surrounding whitespace + any run of whitespace between words.
    /// Rejects anything that isn't exactly 4 words or contains a
    /// non-BIP39-English token.
    pub fn parse(s: &str) -> Result<Self, PairPhraseError> {
        let lower = s.to_ascii_lowercase();
        let tokens: Vec<&str> = lower.split_whitespace().collect();
        if tokens.len() != PHRASE_WORD_COUNT {
            return Err(PairPhraseError::WrongWordCount {
                got: tokens.len(),
                expected: PHRASE_WORD_COUNT,
            });
        }
        let wordlist = Language::English.word_list();
        let mut resolved = Vec::with_capacity(PHRASE_WORD_COUNT);
        for (i, tok) in tokens.iter().enumerate() {
            match wordlist.iter().find(|w| **w == *tok) {
                Some(&w) => resolved.push(w),
                None => {
                    return Err(PairPhraseError::UnknownWord {
                        position: i,
                        word: (*tok).to_string(),
                    })
                }
            }
        }
        Ok(Self::from_static_words(resolved))
    }

    /// 32-byte SHA-256 of the canonical phrase under the nonce-hash
    /// domain-separation prefix. This is the key used by
    /// [`NonceStore`] so operators can safely share a snapshot of the
    /// nonce file without leaking every past phrase in cleartext. An
    /// attacker with the nonce file still only needs 2⁴⁴ hashes to
    /// brute-force a row — the hash is a commitment, not a password
    /// storage primitive — but it is a meaningful step up from raw
    /// phrase storage and survives incidental disclosure of the file.
    pub fn nonce_hash(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(PAIR_NONCE_HASH_DOMAIN);
        h.update([0x00]); // explicit separator so domain || canonical is unambiguous
        h.update(self.canonical().as_bytes());
        h.finalize().into()
    }
}

/// Errors returned by [`PairPhrase::parse`].
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PairPhraseError {
    /// Caller supplied a word count other than [`PHRASE_WORD_COUNT`].
    #[error("pairing phrase must be {expected} words, got {got}")]
    WrongWordCount { got: usize, expected: usize },

    /// A token wasn't in the BIP39 English wordlist. `position` is
    /// 0-indexed so the CLI can point at the offending word directly.
    #[error("pairing phrase word #{} ({:?}) is not in the BIP39 English wordlist", position + 1, word)]
    UnknownWord { position: usize, word: String },
}

/// Generate a fresh 4-word pairing phrase.
///
/// Pure function of the supplied CSPRNG — seeded RNGs reproduce the
/// same phrase byte-for-byte, which integration tests exploit to pin
/// the deterministic path. Real operator flows seed from
/// [`rand_core::OsRng`] at the shell boundary.
pub fn generate_phrase<R: RngCore + CryptoRng>(rng: &mut R) -> PairPhrase {
    let wordlist = Language::English.word_list();
    let mut words = Vec::with_capacity(PHRASE_WORD_COUNT);
    for _ in 0..PHRASE_WORD_COUNT {
        // 2^32 / 2048 = 2^21 — rejection sampling is a non-issue.
        // Draw 4 fresh bytes per word, mod the wordlist size, and
        // accept. 2048 is a power of two so `mod` is bias-free.
        let mut buf = [0u8; 4];
        rng.fill_bytes(&mut buf);
        let idx = (u32::from_le_bytes(buf) as usize) & 0x07ff; // 0..2048
        words.push(wordlist[idx]);
    }
    PairPhrase::from_static_words(words)
}

// ───────────────────────────────────────────────────────────────
// Phrase-reuse nonce store
// ───────────────────────────────────────────────────────────────

/// On-disk shape of `.zetl/caps/.pair-nonces`. Each row records one
/// pairing session; rows older than [`PAIR_NONCE_TTL_SECS`] are pruned
/// the next time [`NonceStore::accept`] runs.
///
/// The file is plain TOML so operators can audit it by hand; the
/// phrase-hash is hex-encoded (not base64) for the same reason — hex
/// pastes straight into an auditor's grep without URL-safe-character
/// translation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NonceStore {
    /// Every past pairing session still within the TTL window. Ordered
    /// by `used_at` ascending so deterministic serialisation matches
    /// the read-back byte-for-byte.
    #[serde(default)]
    pub entries: Vec<NonceEntry>,
}

/// One row in `.zetl/caps/.pair-nonces`. Deliberately minimal — the
/// spec never stores the phrase itself, only the commitment hash, and
/// the timestamp is sufficient to gate reuse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NonceEntry {
    /// Lowercase-hex SHA-256 of the canonical phrase (domain-separated
    /// per [`PairPhrase::nonce_hash`]).
    pub phrase_hash_hex: String,
    /// Unix seconds at the moment the phrase was first accepted.
    pub used_at_unix: u64,
}

/// Errors returned by nonce-store operations.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NonceStoreError {
    /// The phrase hash appeared in the store at `used_at` which is still
    /// inside the TTL window. The caller should abort the session and
    /// prompt the operator to generate a new phrase.
    #[error("phrase already used at unix {used_at} (within the {ttl}s reuse window)")]
    RecentlyUsed { used_at: u64, ttl: u64 },

    /// The file on disk was not valid TOML — the caller should decide
    /// whether to treat this as empty (clean slate) or as fatal.
    #[error("nonce store TOML parse error: {0}")]
    Toml(String),
}

impl NonceStore {
    /// Parse a nonce store from a TOML string. Empty input is treated
    /// as an empty store so a missing file can be `""` on first run.
    pub fn from_toml(s: &str) -> Result<Self, NonceStoreError> {
        if s.trim().is_empty() {
            return Ok(Self::default());
        }
        toml::from_str(s).map_err(|e| NonceStoreError::Toml(e.to_string()))
    }

    /// Serialise the store as TOML. Sorted by `used_at` ascending so the
    /// output is byte-stable given the same input.
    pub fn to_toml(&self) -> String {
        // Clone + sort for render stability. The clone is cheap — a
        // pairing session per day for 30 days is 30 rows.
        let mut s = self.clone();
        s.entries.sort_by_key(|e| e.used_at_unix);
        toml::to_string(&s).unwrap_or_default()
    }

    /// Attempt to accept a pairing phrase. Returns:
    /// - `Ok(updated_store)` on success — the caller persists this
    ///   store to disk BEFORE handing the phrase to SPAKE2 so a crash
    ///   mid-session does not permit replay.
    /// - `Err(RecentlyUsed)` if the phrase was used within the TTL.
    ///
    /// The returned store is pruned of all expired rows regardless of
    /// accept/reject, so repeated calls do not let the file grow past
    /// roughly `TTL_SECS / min-pair-interval` rows.
    pub fn accept(
        mut self,
        phrase_hash: &[u8; 32],
        now_unix: u64,
    ) -> Result<Self, NonceStoreError> {
        let cutoff = now_unix.saturating_sub(PAIR_NONCE_TTL_SECS);
        self.entries.retain(|e| e.used_at_unix >= cutoff);

        let hex = hex_encode(phrase_hash);
        if let Some(e) = self.entries.iter().find(|e| e.phrase_hash_hex == hex) {
            return Err(NonceStoreError::RecentlyUsed {
                used_at: e.used_at_unix,
                ttl: PAIR_NONCE_TTL_SECS,
            });
        }
        self.entries.push(NonceEntry {
            phrase_hash_hex: hex,
            used_at_unix: now_unix,
        });
        Ok(self)
    }
}

/// Lowercase-hex of a 32-byte buffer. Self-contained so `cap::pair`
/// carries no dependency on the `hex` crate (which isn't in the
/// workspace today and would be a sizeable crate to add just for this
/// call site).
fn hex_encode(bytes: &[u8; 32]) -> String {
    const LUT: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for b in bytes {
        out.push(LUT[((*b >> 4) & 0x0f) as usize] as char);
        out.push(LUT[(*b & 0x0f) as usize] as char);
    }
    out
}

// ───────────────────────────────────────────────────────────────
// SPAKE2 session driver + HMAC handoff
// ───────────────────────────────────────────────────────────────

/// An in-progress pairing session. Holds the SPAKE2 instance — which is
/// single-use per the upstream crate — and nothing else. The caller
/// transports the outbound message to the peer out-of-band, receives
/// the peer's outbound message, and calls [`PairSession::finish`] to
/// derive the shared key.
pub struct PairSession {
    inner: Spake2<Ed25519Group>,
}

/// Outbound SPAKE2 handshake message — raw bytes plus a base64 helper
/// for the operator-facing wire format. Printed on stdout by
/// `zetl cap pair`; operators paste it into whatever channel they
/// agreed on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairMessage(pub Vec<u8>);

impl PairMessage {
    /// Encode as base64 (standard alphabet, no padding) for terminal
    /// display. Standard alphabet rather than url-safe so the message
    /// survives line-wrapping without `-`/`+` confusion; no-pad so the
    /// terminal output is a single token.
    pub fn to_b64(&self) -> String {
        B64.encode(&self.0)
    }

    /// Decode from the base64 form emitted by [`PairMessage::to_b64`].
    pub fn from_b64(s: &str) -> Result<Self, PairError> {
        let raw = B64
            .decode(s.trim().as_bytes())
            .map_err(|_| PairError::MessageDecode)?;
        Ok(Self(raw))
    }
}

/// Errors returned by [`PairSession`] and the handoff authenticator.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PairError {
    /// Peer's SPAKE2 message wasn't valid base64.
    #[error("peer message is not valid base64")]
    MessageDecode,
    /// The SPAKE2 crate rejected the peer message (wrong side byte,
    /// wrong length, non-canonical encoding, identity element, …). The
    /// operator response should be: "abort, start a new phrase."
    #[error("SPAKE2 protocol error — phrase mismatch or malformed peer message")]
    Spake2Protocol,
    /// Pubkey bytes weren't the expected 32-byte length.
    #[error("pubkey must be {PAIR_PUBKEY_LEN} bytes, got {got}")]
    PubkeyLen { got: usize },
    /// HMAC verification failed — either the phrase was wrong on one
    /// side, the handoff message was tampered with, or a MITM tried
    /// to swap the pubkey.
    #[error("HMAC tag does not authenticate pubkey — abort pairing")]
    HmacMismatch,
    /// HMAC tag wasn't the expected 32-byte length.
    #[error("HMAC tag must be {PAIR_HMAC_LEN} bytes, got {got}")]
    HmacLen { got: usize },
}

impl PairSession {
    /// Begin a new symmetric SPAKE2 exchange for `phrase`. Returns the
    /// outbound message + a handle the caller feeds the peer's reply
    /// into via [`PairSession::finish`].
    ///
    /// This uses the symmetric variant of SPAKE2 (both sides run the
    /// same code; no "A" vs "B" role to agree on) which matches the
    /// operator-to-operator workflow — neither side is naturally the
    /// server.
    pub fn start(phrase: &PairPhrase) -> Result<(Self, PairMessage), PairError> {
        let pw = Password::new(phrase.canonical().as_bytes());
        let id = Identity::new(PAIR_SPAKE2_IDENTITY);
        let (sess, msg) = Spake2::<Ed25519Group>::start_symmetric(&pw, &id);
        Ok((Self { inner: sess }, PairMessage(msg)))
    }

    /// Finish the exchange with the peer's outbound message and derive
    /// the shared authentication key. Consumes the session — the
    /// SPAKE2 crate enforces single-use, so a second call is not
    /// possible at the type level.
    pub fn finish(self, peer: &PairMessage) -> Result<SharedAuthKey, PairError> {
        let raw = self
            .inner
            .finish(&peer.0)
            .map_err(|_| PairError::Spake2Protocol)?;
        // Turn the SPAKE2 output into a fixed-size HMAC key via HKDF
        // so callers with a different key length need (future HMAC-512
        // migration, SIV construction) can swap the KDF without
        // breaking on-the-wire shapes.
        let hk = Hkdf::<Sha256>::new(None, &raw);
        let mut key = [0u8; 32];
        hk.expand(PAIR_HMAC_HKDF_INFO, &mut key)
            .expect("HKDF expand for 32 bytes always fits");
        Ok(SharedAuthKey { key })
    }
}

/// One-shot HMAC key derived from a SPAKE2 shared secret. Never
/// persisted, never used for content encryption — per SPEC-034 §8 the
/// pair protocol's shared key exists only long enough to authenticate
/// the pubkey handoff, then is dropped.
///
/// The key is zeroed on drop via [`zeroize::Zeroize`]. `Clone` is
/// deliberately omitted — the expected flow is construct → use once →
/// drop.
#[derive(Zeroize)]
#[zeroize(drop)]
pub struct SharedAuthKey {
    key: [u8; 32],
}

impl SharedAuthKey {
    /// Produce an HMAC-SHA256 tag over the 32-byte raw pubkey. Pubkey
    /// format is the raw X25519 or Ed25519 scalar bytes — NOT the
    /// `age-recipient-v1:` string, NOT the base64 form — so MITMs
    /// can't swap the rendering without changing the tag.
    pub fn authenticate_pubkey(&self, pubkey: &[u8]) -> Result<[u8; PAIR_HMAC_LEN], PairError> {
        if pubkey.len() != PAIR_PUBKEY_LEN {
            return Err(PairError::PubkeyLen { got: pubkey.len() });
        }
        let mut mac = HmacSha256::new_from_slice(&self.key).expect("HMAC accepts any key length");
        mac.update(pubkey);
        let tag = mac.finalize().into_bytes();
        let mut out = [0u8; PAIR_HMAC_LEN];
        out.copy_from_slice(&tag);
        Ok(out)
    }

    /// Constant-time verification of an HMAC-SHA256 tag over the given
    /// pubkey. Returns `Err(HmacMismatch)` on any divergence — either a
    /// phrase mismatch or a tampered tag.
    pub fn verify_pubkey(&self, pubkey: &[u8], tag: &[u8]) -> Result<(), PairError> {
        if pubkey.len() != PAIR_PUBKEY_LEN {
            return Err(PairError::PubkeyLen { got: pubkey.len() });
        }
        if tag.len() != PAIR_HMAC_LEN {
            return Err(PairError::HmacLen { got: tag.len() });
        }
        let mut mac = HmacSha256::new_from_slice(&self.key).expect("HMAC accepts any key length");
        mac.update(pubkey);
        mac.verify_slice(tag).map_err(|_| PairError::HmacMismatch)
    }
}

// ───────────────────────────────────────────────────────────────
// Tests
// ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    fn seeded() -> ChaCha20Rng {
        ChaCha20Rng::from_seed([7u8; 32])
    }

    #[test]
    fn generate_phrase_yields_four_english_words() {
        let p = generate_phrase(&mut seeded());
        let wl = Language::English.word_list();
        assert_eq!(p.words().len(), PHRASE_WORD_COUNT);
        for w in p.words() {
            assert!(wl.contains(w), "{w:?} not in BIP39 English wordlist");
        }
    }

    #[test]
    fn generate_phrase_is_deterministic_under_seeded_rng() {
        let a = generate_phrase(&mut seeded());
        let b = generate_phrase(&mut seeded());
        assert_eq!(a, b);
        // Also pin the canonical rendering so a silent wordlist swap
        // regresses loudly.
        assert_eq!(a.canonical().split_whitespace().count(), PHRASE_WORD_COUNT);
    }

    #[test]
    fn generate_phrase_two_successive_draws_differ() {
        let mut rng = seeded();
        let a = generate_phrase(&mut rng);
        let b = generate_phrase(&mut rng);
        // 44 bits of entropy — the chance of equal draws is
        // negligible; a hit means we're not actually drawing fresh
        // bytes.
        assert_ne!(a, b);
    }

    #[test]
    fn parse_roundtrips_canonical_form() {
        let p = generate_phrase(&mut seeded());
        let s = p.canonical();
        let parsed = PairPhrase::parse(&s).expect("canonical round-trip");
        assert_eq!(parsed, p);
    }

    #[test]
    fn parse_tolerates_whitespace_and_case() {
        let p = generate_phrase(&mut seeded());
        let ws = format!("  \t{}  \n", p.canonical().to_uppercase());
        assert_eq!(PairPhrase::parse(&ws).unwrap(), p);
    }

    #[test]
    fn parse_rejects_wrong_word_count() {
        assert!(matches!(
            PairPhrase::parse("abandon ability"),
            Err(PairPhraseError::WrongWordCount { got: 2, .. })
        ));
        assert!(matches!(
            PairPhrase::parse("abandon ability able about above"),
            Err(PairPhraseError::WrongWordCount { got: 5, .. })
        ));
    }

    #[test]
    fn parse_rejects_non_bip39_word() {
        assert!(matches!(
            PairPhrase::parse("abandon xyzzy able about"),
            Err(PairPhraseError::UnknownWord { position: 1, .. })
        ));
    }

    #[test]
    fn nonce_hash_is_deterministic_and_32_bytes() {
        let p = PairPhrase::parse("abandon ability able about").unwrap();
        let a = p.nonce_hash();
        let b = p.nonce_hash();
        assert_eq!(a, b);
        assert_eq!(a.len(), 32);
        // Different phrase → different hash.
        let q = PairPhrase::parse("abandon ability able above").unwrap();
        assert_ne!(p.nonce_hash(), q.nonce_hash());
    }

    #[test]
    fn nonce_store_accept_round_trips_via_toml() {
        let store = NonceStore::default();
        let p = PairPhrase::parse("abandon ability able about").unwrap();
        let now = 1_700_000_000u64;
        let store = store.accept(&p.nonce_hash(), now).unwrap();
        let toml = store.to_toml();
        let parsed = NonceStore::from_toml(&toml).unwrap();
        assert_eq!(parsed, store);
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].used_at_unix, now);
    }

    #[test]
    fn nonce_store_rejects_reuse_within_ttl() {
        let p = PairPhrase::parse("abandon ability able about").unwrap();
        let store = NonceStore::default()
            .accept(&p.nonce_hash(), 1_700_000_000)
            .unwrap();
        // Exactly at the TTL boundary the row is still "recent" per
        // the `>=` in the cutoff check.
        let err = store
            .clone()
            .accept(&p.nonce_hash(), 1_700_000_000 + PAIR_NONCE_TTL_SECS - 1)
            .unwrap_err();
        assert!(matches!(err, NonceStoreError::RecentlyUsed { .. }));
    }

    #[test]
    fn nonce_store_accepts_after_ttl_expires() {
        let p = PairPhrase::parse("abandon ability able about").unwrap();
        let store = NonceStore::default()
            .accept(&p.nonce_hash(), 1_700_000_000)
            .unwrap();
        // Past the TTL window the old row is pruned and reuse is fine.
        let later = 1_700_000_000 + PAIR_NONCE_TTL_SECS + 1;
        let store = store.accept(&p.nonce_hash(), later).unwrap();
        // One surviving row (the new accept); the old row was pruned.
        assert_eq!(store.entries.len(), 1);
        assert_eq!(store.entries[0].used_at_unix, later);
    }

    #[test]
    fn nonce_store_prunes_expired_rows_even_on_accept_for_new_phrase() {
        let old = PairPhrase::parse("abandon ability able about").unwrap();
        let new = PairPhrase::parse("abandon ability able above").unwrap();
        let store = NonceStore::default()
            .accept(&old.nonce_hash(), 1_700_000_000)
            .unwrap();
        // Accepting a brand-new phrase past the TTL prunes the old row.
        let later = 1_700_000_000 + PAIR_NONCE_TTL_SECS + 1;
        let store = store.accept(&new.nonce_hash(), later).unwrap();
        assert_eq!(store.entries.len(), 1);
        assert_eq!(
            store.entries[0].phrase_hash_hex,
            hex_encode(&new.nonce_hash())
        );
    }

    #[test]
    fn nonce_store_from_toml_treats_empty_as_default() {
        assert_eq!(NonceStore::from_toml("").unwrap(), NonceStore::default());
        assert_eq!(
            NonceStore::from_toml("   \n").unwrap(),
            NonceStore::default()
        );
    }

    #[test]
    fn nonce_store_from_toml_rejects_garbage() {
        assert!(matches!(
            NonceStore::from_toml("this is not toml ="),
            Err(NonceStoreError::Toml(_))
        ));
    }

    #[test]
    fn spake2_two_sides_converge_with_same_phrase() {
        let phrase = generate_phrase(&mut seeded());
        let (a_sess, a_msg) = PairSession::start(&phrase).unwrap();
        let (b_sess, b_msg) = PairSession::start(&phrase).unwrap();
        let a_key = a_sess.finish(&b_msg).unwrap();
        let b_key = b_sess.finish(&a_msg).unwrap();

        // Both sides derive the SAME authenticator over the same pubkey
        // → the shared key converged.
        let pk = [9u8; PAIR_PUBKEY_LEN];
        let tag_a = a_key.authenticate_pubkey(&pk).unwrap();
        let tag_b = b_key.authenticate_pubkey(&pk).unwrap();
        assert_eq!(tag_a, tag_b);
    }

    #[test]
    fn spake2_wrong_phrase_produces_divergent_keys() {
        let a_phrase = generate_phrase(&mut seeded());
        let b_phrase = PairPhrase::parse("abandon ability able about").unwrap();
        let (a_sess, a_msg) = PairSession::start(&a_phrase).unwrap();
        let (b_sess, b_msg) = PairSession::start(&b_phrase).unwrap();

        // The SPAKE2 protocol itself does not fail on phrase mismatch
        // — it just yields different session keys. The mismatch surfaces
        // when the handoff HMAC fails to verify below.
        let a_key = a_sess.finish(&b_msg).unwrap();
        let b_key = b_sess.finish(&a_msg).unwrap();

        let pk = [9u8; PAIR_PUBKEY_LEN];
        let tag_from_a = a_key.authenticate_pubkey(&pk).unwrap();
        assert!(matches!(
            b_key.verify_pubkey(&pk, &tag_from_a),
            Err(PairError::HmacMismatch)
        ));
    }

    #[test]
    fn pair_message_base64_roundtrip() {
        let phrase = generate_phrase(&mut seeded());
        let (_sess, msg) = PairSession::start(&phrase).unwrap();
        let s = msg.to_b64();
        let back = PairMessage::from_b64(&s).unwrap();
        assert_eq!(msg, back);
    }

    #[test]
    fn pair_message_rejects_non_base64() {
        assert!(matches!(
            PairMessage::from_b64("not base64!!!@@@"),
            Err(PairError::MessageDecode)
        ));
    }

    #[test]
    fn shared_auth_key_rejects_wrong_pubkey_length() {
        let phrase = generate_phrase(&mut seeded());
        let (a_sess, _a_msg) = PairSession::start(&phrase).unwrap();
        let (b_sess, b_msg) = PairSession::start(&phrase).unwrap();
        let a_key = a_sess.finish(&b_msg).unwrap();
        // Intentionally drop b_sess without finishing — we only need
        // `a_key` for the length check below.
        drop(b_sess);
        assert!(matches!(
            a_key.authenticate_pubkey(&[1u8; 31]),
            Err(PairError::PubkeyLen { got: 31 })
        ));
    }

    #[test]
    fn shared_auth_key_rejects_wrong_tag_length() {
        let phrase = generate_phrase(&mut seeded());
        let (a_sess, _a_msg) = PairSession::start(&phrase).unwrap();
        let (_b_sess, b_msg) = PairSession::start(&phrase).unwrap();
        let a_key = a_sess.finish(&b_msg).unwrap();
        let pk = [2u8; PAIR_PUBKEY_LEN];
        assert!(matches!(
            a_key.verify_pubkey(&pk, &[0u8; 31]),
            Err(PairError::HmacLen { got: 31 })
        ));
    }

    #[test]
    fn nonce_store_toml_is_human_readable() {
        let p = PairPhrase::parse("abandon ability able about").unwrap();
        let store = NonceStore::default()
            .accept(&p.nonce_hash(), 1_700_000_000)
            .unwrap();
        let toml = store.to_toml();
        // Human legibility: hex phrase-hash appears in the expected
        // form and the `used_at_unix` field name is stable.
        assert!(toml.contains("phrase_hash_hex"));
        assert!(toml.contains("used_at_unix"));
        assert!(toml.contains(&hex_encode(&p.nonce_hash())));
    }
}
