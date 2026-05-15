//! Shared EdDSA-JWT recogniser (SPEC-041 REQ-4120, ADR-4109).
//!
//! LangSec: recognition — the three-segment base64url grammar plus Ed25519
//! signature verification — is separated from interpretation — the claims.
//! One recogniser serves every EdDSA-JWT call site in zetl: the SPEC-020
//! invitation token (`src/user/invite.rs`) and the SPEC-041 capability token
//! (`src/web/auth/capability_url.rs`, later in this plan).
//!
//! The agent token (`src/user/agent_token.rs`) is deliberately **not** routed
//! through here — despite the IMPL-041 task note, it is not a JWT. It uses a
//! bespoke `base64url(user_id || generation || ed25519_sig)` binary format
//! with its own recogniser. "One parser per language" (REQ-4120) means one
//! parser per *format*; the agent token is a different format.
//!
//! Grammar recognised by [`recognise`]:
//!
//! ```text
//! jwt        = segment "." segment "." signature
//! segment    = 1*( base64url-char )         ; header / payload, no padding
//! signature  = 1*( base64url-char )         ; decodes to exactly 64 bytes
//! ```

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::de::DeserializeOwned;
use serde::Serialize;

/// JWT header — `{"alg":"EdDSA","typ":"JWT"}`. The only header zetl produces;
/// callers that recognise tokens do not inspect the header beyond the
/// signature it commits to.
#[derive(Serialize)]
struct JwtHeader {
    alg: &'static str,
    typ: &'static str,
}

/// Why an EdDSA-JWT failed recognition or claim extraction.
///
/// `Structure` and `Signature` are *recognition* failures — the input is not
/// a well-formed, authentic token. `Payload` and `WrongSubject` are
/// *interpretation* failures — the token is authentic, but does not carry the
/// claims the caller required.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TokenError {
    /// Not three base64url segments, or the signature segment does not decode
    /// to exactly 64 bytes.
    Structure(String),
    /// The Ed25519 signature did not verify against the supplied key.
    Signature,
    /// The payload segment is not valid JSON for the expected claims type.
    Payload(String),
    /// The `sub` claim did not equal the value the caller required. `sub` is
    /// the token-type discriminator (REQ-4120, ADR-4109).
    WrongSubject {
        expected: &'static str,
        found: String,
    },
}

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokenError::Structure(d) => write!(f, "malformed JWT: {d}"),
            TokenError::Signature => write!(f, "JWT signature verification failed"),
            TokenError::Payload(d) => write!(f, "invalid JWT payload: {d}"),
            TokenError::WrongSubject { expected, found } => {
                write!(f, "JWT sub mismatch: expected {expected:?}, found {found:?}")
            }
        }
    }
}

impl std::error::Error for TokenError {}

/// A structurally-recognised, signature-verified EdDSA JWT.
///
/// Holding one of these proves the three-segment grammar held and the
/// signature verified against a known key. It proves *nothing* about the
/// claims — call [`EdDsaJwt::claims`] to interpret them.
#[derive(Debug)]
pub(crate) struct EdDsaJwt {
    /// Decoded payload bytes — a JSON object, not yet interpreted.
    payload: Vec<u8>,
}

/// Recognise and signature-verify an EdDSA JWT against `key`.
///
/// Performs *only* recognition: the three-segment grammar, base64url decoding,
/// the 64-byte signature-length check, and `verify_strict`. No claim is
/// interpreted here — recognise before acting (REQ-4120).
pub(crate) fn recognise(token: &str, key: &VerifyingKey) -> Result<EdDsaJwt, TokenError> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(TokenError::Structure(format!(
            "expected 3 segments, got {}",
            parts.len()
        )));
    }

    let signing_input = format!("{}.{}", parts[0], parts[1]);
    let sig_bytes = URL_SAFE_NO_PAD
        .decode(parts[2])
        .map_err(|e| TokenError::Structure(format!("signature not base64url: {e}")))?;
    let sig_arr: [u8; 64] = sig_bytes.as_slice().try_into().map_err(|_| {
        TokenError::Structure(format!(
            "signature is {} bytes, expected 64",
            sig_bytes.len()
        ))
    })?;
    let signature = Signature::from_bytes(&sig_arr);
    key.verify_strict(signing_input.as_bytes(), &signature)
        .map_err(|_| TokenError::Signature)?;

    let payload = URL_SAFE_NO_PAD
        .decode(parts[1])
        .map_err(|e| TokenError::Structure(format!("payload not base64url: {e}")))?;
    Ok(EdDsaJwt { payload })
}

impl EdDsaJwt {
    /// Interpret the payload as `T`, but only after the `sub` claim equals
    /// `expected_sub`.
    ///
    /// `sub` is the token-type discriminator: it is read and checked *before*
    /// the payload is deserialised into `T`, so a recogniser shared between
    /// token types (invite, capability) can never hand a caller claims from
    /// the wrong token type (REQ-4120, ADR-4109).
    ///
    /// `expected_sub` is `&'static str` because every call site is a token-type
    /// literal (`"zetl-invite"`, `"zetl-capability"`); this also lets
    /// [`TokenError::WrongSubject`] carry it without allocating.
    pub(crate) fn claims<T: DeserializeOwned>(
        &self,
        expected_sub: &'static str,
    ) -> Result<T, TokenError> {
        // Read just the discriminator first, without committing to `T`'s shape.
        #[derive(serde::Deserialize)]
        struct SubOnly {
            sub: String,
        }
        let probe: SubOnly = serde_json::from_slice(&self.payload)
            .map_err(|e| TokenError::Payload(format!("not a claims object: {e}")))?;
        if probe.sub != expected_sub {
            return Err(TokenError::WrongSubject {
                expected: expected_sub,
                found: probe.sub,
            });
        }
        serde_json::from_slice(&self.payload).map_err(|e| TokenError::Payload(e.to_string()))
    }
}

/// Encode `claims` as an EdDSA JWT signed by `key`.
///
/// The single JWT-producing path in zetl — REQ-4120 is "one parser per
/// language", and the symmetric obligation is one serialiser.
pub(crate) fn encode<T: Serialize>(key: &SigningKey, claims: &T) -> Result<String, TokenError> {
    let header = JwtHeader {
        alg: "EdDSA",
        typ: "JWT",
    };
    let header_json = serde_json::to_vec(&header)
        .map_err(|e| TokenError::Payload(format!("header serialise: {e}")))?;
    let payload_json = serde_json::to_vec(claims)
        .map_err(|e| TokenError::Payload(format!("payload serialise: {e}")))?;

    let header_b64 = URL_SAFE_NO_PAD.encode(&header_json);
    let payload_b64 = URL_SAFE_NO_PAD.encode(&payload_json);
    let signing_input = format!("{header_b64}.{payload_b64}");
    let signature = key.sign(signing_input.as_bytes());
    let sig_b64 = URL_SAFE_NO_PAD.encode(signature.to_bytes());
    Ok(format!("{signing_input}.{sig_b64}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    fn test_key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    #[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
    struct DemoClaims {
        sub: String,
        scope: String,
        n: u64,
    }

    /// TEST-4120 (token portion), roundtrip property: `recognise ∘ encode`
    /// recovers the original claims for any well-formed input.
    #[test]
    fn encode_recognise_roundtrip() {
        let key = test_key();
        let claims = DemoClaims {
            sub: "zetl-demo".to_string(),
            scope: "a/b/**".to_string(),
            n: 42,
        };
        let token = encode(&key, &claims).unwrap();
        let jwt = recognise(&token, &key.verifying_key()).unwrap();
        let back: DemoClaims = jwt.claims("zetl-demo").unwrap();
        assert_eq!(back, claims);
    }

    /// TEST-4120: a token not in three segments is a Structure error.
    #[test]
    fn rejects_wrong_segment_count() {
        let key = test_key();
        let err = recognise("only.two", &key.verifying_key()).unwrap_err();
        assert!(matches!(err, TokenError::Structure(_)));
    }

    /// TEST-4120: a signature segment that does not decode to 64 bytes is a
    /// Structure error, surfaced before any verification is attempted.
    #[test]
    fn rejects_bad_signature_length() {
        let key = test_key();
        // header.payload.<short sig>
        let err = recognise("aGVhZGVy.cGF5bG9hZA.c2hvcnQ", &key.verifying_key()).unwrap_err();
        assert!(matches!(err, TokenError::Structure(_)));
    }

    /// TEST-4120: a token signed by a different key fails signature
    /// verification.
    #[test]
    fn rejects_wrong_key() {
        let signer = test_key();
        let other = SigningKey::from_bytes(&[9u8; 32]);
        let claims = DemoClaims {
            sub: "zetl-demo".to_string(),
            scope: "x".to_string(),
            n: 1,
        };
        let token = encode(&signer, &claims).unwrap();
        let err = recognise(&token, &other.verifying_key()).unwrap_err();
        assert_eq!(err, TokenError::Signature);
    }

    /// TEST-4120: the `sub` discriminator is checked before claims of the
    /// caller's type are trusted.
    #[test]
    fn claims_rejects_wrong_subject() {
        let key = test_key();
        let claims = DemoClaims {
            sub: "zetl-invite".to_string(),
            scope: "x".to_string(),
            n: 1,
        };
        let token = encode(&key, &claims).unwrap();
        let jwt = recognise(&token, &key.verifying_key()).unwrap();
        let err = jwt.claims::<DemoClaims>("zetl-capability").unwrap_err();
        assert_eq!(
            err,
            TokenError::WrongSubject {
                expected: "zetl-capability",
                found: "zetl-invite".to_string(),
            }
        );
    }

    /// TEST-4120: a tampered payload segment (re-encoded after signing)
    /// fails signature verification — the signature commits to the payload.
    #[test]
    fn rejects_tampered_payload() {
        let key = test_key();
        let claims = DemoClaims {
            sub: "zetl-demo".to_string(),
            scope: "x".to_string(),
            n: 1,
        };
        let token = encode(&key, &claims).unwrap();
        let mut parts: Vec<&str> = token.split('.').collect();
        let forged_payload = URL_SAFE_NO_PAD.encode(br#"{"sub":"zetl-demo","scope":"x","n":999}"#);
        parts[1] = &forged_payload;
        let tampered = parts.join(".");
        let err = recognise(&tampered, &key.verifying_key()).unwrap_err();
        assert_eq!(err, TokenError::Signature);
    }
}
