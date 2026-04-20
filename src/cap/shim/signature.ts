// Ed25519 signature verification (REQ-3427 / NFR-3413).
//
// Verification runs BEFORE any other processing (CON-3408 STEP 1). On
// failure the pipeline MUST NOT invoke `credentials.get/create`, MUST NOT
// attempt decryption, and MUST surface a user-visible error.

import * as ed from "@noble/ed25519";
import { sha512 } from "@noble/hashes/sha512";

// `@noble/ed25519` v2 is synchronous only when a SHA-512 implementation
// is injected (it ships sync-SHA-512 off by default). `sha512` from
// `@noble/hashes` satisfies that.
ed.etc.sha512Sync = (...messages: Uint8Array[]) => {
  const concat = concatBytes(messages);
  return sha512(concat);
};

/// Verify an Ed25519 signature over the given message using the 32-byte
/// public key. Throws on malformed inputs; returns `false` on a
/// cryptographically invalid signature.
export async function verifyEd25519(
  publicKey: Uint8Array,
  message: Uint8Array,
  signature: Uint8Array,
): Promise<boolean> {
  if (publicKey.length !== 32) {
    throw new Error(
      `invalid Ed25519 public key length: ${publicKey.length} bytes (expected 32)`,
    );
  }
  if (signature.length !== 64) {
    throw new Error(
      `invalid Ed25519 signature length: ${signature.length} bytes (expected 64)`,
    );
  }
  try {
    // `verifyAsync` rejects non-canonical R/S encodings per RFC 8032
    // §8.4 (parity with `ed25519-dalek::verify_strict`).
    return await ed.verifyAsync(signature, message, publicKey);
  } catch {
    return false;
  }
}

function concatBytes(chunks: Uint8Array[]): Uint8Array {
  let total = 0;
  for (const c of chunks) total += c.length;
  const out = new Uint8Array(total);
  let off = 0;
  for (const c of chunks) {
    out.set(c, off);
    off += c.length;
  }
  return out;
}
