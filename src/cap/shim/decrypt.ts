// age v1 decryption via typage (`age-encryption` npm).
//
// typage takes identities in bech32 AGE-SECRET-KEY-1… form. Our priv_A
// arrives as 32 raw bytes (from the fragment or from IDB unwrap), so we
// bech32-encode the scalar before handing it to typage.

import { Decrypter } from "age-encryption";

const AGE_HRP = "age-secret-key-";

export class DecryptError extends Error {
  override readonly name = "DecryptError";
  constructor(
    readonly kind: "not-a-recipient" | "malformed-ciphertext" | "identity-encode",
    message: string,
  ) {
    super(message);
  }
}

export async function ageDecrypt(
  ciphertext: Uint8Array,
  privA: Uint8Array,
): Promise<Uint8Array> {
  if (privA.length !== 32) {
    throw new DecryptError(
      "identity-encode",
      `priv_A is ${privA.length} bytes; X25519 scalars are 32`,
    );
  }
  let identity: string;
  try {
    identity = encodeAgeSecretKey(privA);
  } catch (err) {
    throw new DecryptError(
      "identity-encode",
      `could not encode priv_A as AGE-SECRET-KEY-1: ${(err as Error).message}`,
    );
  }
  const d = new Decrypter();
  d.addIdentity(identity);
  try {
    return await d.decrypt(ciphertext, "uint8array");
  } catch (err) {
    // typage surfaces "no identity matched" as a thrown Error. We fold
    // malformed-header errors (magic/version/recipient list) under
    // `malformed-ciphertext` and mis-recipient errors under
    // `not-a-recipient` based on the message. The caller only cares
    // "did it decrypt?" — precise classification is for diagnostics.
    const msg = (err as Error).message ?? String(err);
    if (/no identity|no matching/i.test(msg)) {
      throw new DecryptError(
        "not-a-recipient",
        `priv_A is not a recipient for this ciphertext (${msg}) — grant may have been rotated or revoked`,
      );
    }
    throw new DecryptError(
      "malformed-ciphertext",
      `age ciphertext failed to parse: ${msg}`,
    );
  }
}

/// Bech32 encoder specialised to the age HRP + 32-byte secret key body.
/// age v1 uses lowercase HRP `age-secret-key-` and a 6-char checksum.
/// Public spec: https://github.com/C2SP/C2SP/blob/main/age.md
export function encodeAgeSecretKey(bytes: Uint8Array): string {
  if (bytes.length !== 32) {
    throw new Error(`secret key must be 32 bytes, got ${bytes.length}`);
  }
  const data = convertBits(bytes, 8, 5, true);
  const checksum = bech32Checksum(AGE_HRP, data);
  const combined = new Uint8Array(data.length + checksum.length);
  combined.set(data, 0);
  combined.set(checksum, data.length);
  let s = AGE_HRP + "1";
  for (const v of combined) s += BECH32_CHARSET[v];
  // Bech32 requires entirely-uppercase or entirely-lowercase strings;
  // age v1 canonicalises to uppercase (typage's `generateIdentity`
  // emits `AGE-SECRET-KEY-1…` all-caps).
  return s.toUpperCase();
}

const BECH32_CHARSET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";

function convertBits(
  data: Uint8Array,
  fromBits: number,
  toBits: number,
  pad: boolean,
): Uint8Array {
  let acc = 0;
  let bits = 0;
  const out: number[] = [];
  const maxv = (1 << toBits) - 1;
  for (const value of data) {
    acc = (acc << fromBits) | value;
    bits += fromBits;
    while (bits >= toBits) {
      bits -= toBits;
      out.push((acc >> bits) & maxv);
    }
  }
  if (pad && bits > 0) out.push((acc << (toBits - bits)) & maxv);
  return Uint8Array.from(out);
}

function bech32Polymod(values: Uint8Array): number {
  const GEN = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3];
  let chk = 1;
  for (const v of values) {
    const b = chk >>> 25;
    chk = ((chk & 0x1ffffff) << 5) ^ v;
    for (let i = 0; i < 5; i++) {
      if ((b >> i) & 1) chk ^= GEN[i]!;
    }
  }
  return chk >>> 0;
}

function hrpExpand(hrp: string): Uint8Array {
  const out = new Uint8Array(hrp.length * 2 + 1);
  for (let i = 0; i < hrp.length; i++) out[i] = hrp.charCodeAt(i) >> 5;
  out[hrp.length] = 0;
  for (let i = 0; i < hrp.length; i++)
    out[hrp.length + 1 + i] = hrp.charCodeAt(i) & 31;
  return out;
}

function bech32Checksum(hrp: string, data: Uint8Array): Uint8Array {
  const values = new Uint8Array(hrpExpand(hrp).length + data.length + 6);
  const hrpPart = hrpExpand(hrp);
  values.set(hrpPart, 0);
  values.set(data, hrpPart.length);
  // trailing 6 bytes already zero (checksum slot)
  const polymod = bech32Polymod(values) ^ 1;
  const out = new Uint8Array(6);
  for (let i = 0; i < 6; i++) {
    out[i] = (polymod >> (5 * (5 - i))) & 31;
  }
  return out;
}
