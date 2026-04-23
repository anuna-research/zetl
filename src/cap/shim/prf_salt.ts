// REQ-3414 per-cohort PRF salt derivation — kept in its own module so
// the TOFU branch (identity.ts / tofu.ts) can import the salt helper
// without also pulling in typage's `identityToRecipient` + the QR /
// DOM surface that live in enroll.ts.
//
// Byte-identical to `cap::enrolment::compute_prf_salt` on the Rust
// side (TEST-3414 cross-check) and to `computePrfSalt` re-exported
// from enroll.ts for the reader self-enrolment page.

import { sha256 } from "@noble/hashes/sha2";

/// REQ-3414 PRF salt prefix. Must stay byte-identical to
/// `cap::enrolment::PRF_SALT_PREFIX` on the Rust side.
export const PRF_SALT_PREFIX = "ztl/webauthn-prf/v1/";

/// Compute the REQ-3414 per-cohort PRF salt
///     SHA-256("ztl/webauthn-prf/v1/" || origin || "/" || cohort_id)
/// Pure function; deterministic given (origin, cohort_id).
export function computePrfSalt(origin: string, cohortId: string): Uint8Array {
  const enc = new TextEncoder();
  const prefix = enc.encode(PRF_SALT_PREFIX);
  const originBytes = enc.encode(origin);
  const slash = enc.encode("/");
  const cohortBytes = enc.encode(cohortId);
  const buf = new Uint8Array(
    prefix.length + originBytes.length + slash.length + cohortBytes.length,
  );
  let off = 0;
  buf.set(prefix, off);
  off += prefix.length;
  buf.set(originBytes, off);
  off += originBytes.length;
  buf.set(slash, off);
  off += slash.length;
  buf.set(cohortBytes, off);
  return sha256(buf);
}

/// Construct the TOFU wrap's AES-GCM AAD per CON-3409:
///     aad = utf8(origin || "/" || cohort_id)
/// Exported separately so the unwrap branch can reconstruct the
/// exact same AAD bytes when calling AES-GCM decrypt.
export function tofuAad(origin: string, cohortId: string): Uint8Array {
  const enc = new TextEncoder();
  return enc.encode(`${origin}/${cohortId}`);
}
