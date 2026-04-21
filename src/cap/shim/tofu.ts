// TOFU (trust-on-first-use) branch of the capability-mode shim.
// SPEC-034 REQ-3405, REQ-3414, REQ-3428, REQ-3429 / CON-3409 /
// ADR-3414.
//
// Runs on the reader's first visit with a cap-invite URL. The
// fragment priv_A is already in hand (identity.ts:readFragmentKey);
// TOFU's job is to bind priv_A to a fresh WebAuthn passkey so that
// subsequent visits can drop the fragment and unwrap from
// IndexedDB via `navigator.credentials.get()` + the authenticator's
// PRF extension.
//
// CON-3409 protocol:
//
//     challenge  = crypto.getRandomValues(32)                   // BUG-005
//     prf_salt   = SHA-256("zetl/webauthn-prf/v1/" || origin
//                          || "/" || cohort_id)                 // REQ-3414
//     credential = navigator.credentials.create({
//                    publicKey: { challenge, rp, user,
//                                 pubKeyCredParams,
//                                 authenticatorSelection,
//                                 extensions: { prf: {
//                                   eval: { first: prf_salt } }}}})
//     prf_output = credential.getClientExtensionResults().prf.results.first
//     K_wrap     = HKDF-SHA256(prf_output, "", "zetl/tofu-wrap/v1", 32)
//     iv         = crypto.getRandomValues(12)
//     aad        = utf8(origin || "/" || cohort_id)
//     wrap       = AES-256-GCM(K_wrap, iv, aad, priv_A)
//     await idb.put({ origin, cohortId, credentialId, prfSalt,
//                     iv, aad, ciphertext: wrap, createdAt })
//
// Idempotency: if a binding already exists for `cohortId`, the
// TOFU flow skips (KEEP default per REQ-3425). Collision-UI is a
// later task and will layer on top of this.

import { hkdf } from "@noble/hashes/hkdf";
import { sha256 } from "@noble/hashes/sha2";

import { computePrfSalt, tofuAad } from "./prf_salt.ts";
import {
  readBindingRecord,
  writeBindingRecord,
  type IdbFactoryLike,
  type TofuBinding,
} from "./storage.ts";

/// HKDF info string pinning the TOFU wrap-key derivation. A change
/// here would desync with every reader already bound on the
/// existing deployment.
export const TOFU_WRAP_INFO = "zetl/tofu-wrap/v1";

/// Relying-party display name used in the WebAuthn `rp.name`
/// field. Not security-critical (RP identity is pinned by `rp.id`)
/// but kept as a constant so test assertions stay stable.
export const RP_NAME = "Zetl Wiki";

/// Binary lengths pinned by CON-3409.
export const CHALLENGE_LEN = 32;
export const IV_LEN = 12;
export const K_WRAP_LEN = 32;
export const PRF_OUTPUT_LEN = 32;
export const USER_HANDLE_LEN = 16;

export class TofuError extends Error {
  override readonly name = "TofuError";
  constructor(
    readonly kind:
      | "create-rejected"
      | "no-prf"
      | "wrap-failed"
      | "storage-failed"
      | "subtle-unavailable",
    message: string,
  ) {
    super(message);
  }
}

/// Injection surface — keeps the flow pure-testable under Node's
/// test runner. Production path resolves each one to the browser
/// built-in.
export interface TofuDeps {
  /// `navigator.credentials.create` (or a moniker thereof). Tests
  /// inject a fake that returns a `PublicKeyCredential`-shaped
  /// object with a `getClientExtensionResults` function yielding
  /// `{ prf: { results: { first: <bytes> } } }`.
  createCredential: (
    opts: PublicKeyCredentialCreationOptions,
  ) => Promise<PublicKeyCredential | null>;
  /// SubtleCrypto surface for AES-GCM encryption. Production uses
  /// `crypto.subtle`; tests can override with a webcrypto polyfill.
  subtle: SubtleCrypto;
  /// 32-byte challenge + 12-byte IV RNG. Tests inject deterministic
  /// fixtures so wrap ciphertext becomes byte-stable.
  randomBytes: (n: number) => Uint8Array;
  /// Clock source for `createdAt`. Defaults to `Date.now`.
  now: () => number;
  /// Override the IDB factory — tests use `fake-indexeddb`.
  idbFactory?: IdbFactoryLike | null;
}

export interface TofuInput {
  cohortId: string;
  origin: string;
  privA: Uint8Array;
}

export interface TofuResult {
  /// `"bound"` — a fresh binding was written to IDB.
  /// `"already-bound"` — pre-existing binding for this cohort; no
  /// passkey prompt was raised. The KEEP policy of REQ-3425.
  outcome: "bound" | "already-bound";
  binding?: TofuBinding;
}

/// Run the TOFU flow. Caller holds the `navigator.locks` exclusive
/// lock (see pipeline.ts) and has already purged SWs — those
/// preconditions are NOT rechecked here so unit tests can drive
/// this module directly with injected deps.
export async function performTofu(
  input: TofuInput,
  deps: TofuDeps,
): Promise<TofuResult> {
  const existing = await readBindingRecord(input.cohortId, deps.idbFactory);
  if (existing !== null) {
    return { outcome: "already-bound", binding: existing };
  }

  if (input.privA.length !== 32) {
    throw new TofuError(
      "wrap-failed",
      `priv_A is ${input.privA.length} bytes; TOFU wrap requires a 32-byte X25519 scalar`,
    );
  }

  const prfSalt = computePrfSalt(input.origin, input.cohortId);
  const challenge = deps.randomBytes(CHALLENGE_LEN);
  const userHandle = deps.randomBytes(USER_HANDLE_LEN);

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const creationOptions: any = {
    rp: { id: deriveRpId(input.origin), name: RP_NAME },
    user: {
      id: userHandle,
      name: `zetl:${input.cohortId}`,
      displayName: `zetl reader (${input.cohortId})`,
    },
    challenge,
    pubKeyCredParams: [
      { alg: -8, type: "public-key" }, // Ed25519
      { alg: -7, type: "public-key" }, // ES256
    ],
    authenticatorSelection: {
      residentKey: "preferred",
      userVerification: "required",
    },
    extensions: {
      prf: { eval: { first: prfSalt } },
    },
  };

  let credential: PublicKeyCredential | null;
  try {
    credential = await deps.createCredential(
      creationOptions as PublicKeyCredentialCreationOptions,
    );
  } catch (err) {
    throw new TofuError(
      "create-rejected",
      `navigator.credentials.create rejected: ${(err as Error).message}`,
    );
  }
  if (!credential) {
    throw new TofuError(
      "create-rejected",
      "navigator.credentials.create returned null",
    );
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const ext: any = (credential as PublicKeyCredential)
    .getClientExtensionResults?.();
  const firstBuf: ArrayBuffer | undefined = ext?.prf?.results?.first;
  if (!firstBuf || firstBuf.byteLength === 0) {
    throw new TofuError(
      "no-prf",
      "authenticator did not return a PRF output — this device or browser does not support WebAuthn PRF",
    );
  }
  const prfOutput = new Uint8Array(firstBuf);
  if (prfOutput.length !== PRF_OUTPUT_LEN) {
    throw new TofuError(
      "no-prf",
      `PRF output was ${prfOutput.length} bytes; CON-3409 expects ${PRF_OUTPUT_LEN}`,
    );
  }

  // Derive K_wrap per CON-3409.
  const kWrap = hkdf(
    sha256,
    prfOutput,
    new Uint8Array(0),
    new TextEncoder().encode(TOFU_WRAP_INFO),
    K_WRAP_LEN,
  );

  // AES-256-GCM(K_wrap, iv, aad, priv_A). AAD binds the wrap to
  // origin + cohort_id so ciphertexts copied across cohorts fail
  // the GCM tag check on unwrap.
  if (!deps.subtle) {
    throw new TofuError(
      "subtle-unavailable",
      "SubtleCrypto is unavailable — the TOFU wrap cannot run",
    );
  }
  const aad = tofuAad(input.origin, input.cohortId);
  const iv = deps.randomBytes(IV_LEN);
  let ciphertext: Uint8Array;
  try {
    const key = await deps.subtle.importKey(
      "raw",
      asBufferSource(kWrap),
      { name: "AES-GCM" },
      false,
      ["encrypt"],
    );
    const buf = await deps.subtle.encrypt(
      {
        name: "AES-GCM",
        iv: asBufferSource(iv),
        additionalData: asBufferSource(aad),
        tagLength: 128,
      },
      key,
      asBufferSource(input.privA),
    );
    ciphertext = new Uint8Array(buf);
  } catch (err) {
    throw new TofuError(
      "wrap-failed",
      `AES-256-GCM wrap failed: ${(err as Error).message}`,
    );
  }

  const credentialId = new Uint8Array(
    (credential as PublicKeyCredential).rawId ?? new ArrayBuffer(0),
  );
  const binding: TofuBinding = {
    origin: input.origin,
    cohortId: input.cohortId,
    credentialId,
    prfSalt,
    iv,
    aad,
    ciphertext,
    createdAt: deps.now(),
  };

  try {
    await writeBindingRecord(binding, deps.idbFactory);
  } catch (err) {
    throw new TofuError(
      "storage-failed",
      `IDB write for cohort ${JSON.stringify(input.cohortId)} failed: ${(err as Error).message}`,
    );
  }

  return { outcome: "bound", binding };
}

/// Extract the WebAuthn RP id from the page origin. RP id must be
/// a registrable-domain suffix of the page origin's host; for our
/// all-static deployment the page host is exactly the RP id.
export function deriveRpId(origin: string): string {
  try {
    return new URL(origin).hostname;
  } catch {
    // Degenerate fallback for tests that pass a bare host instead
    // of a full URL.
    return origin;
  }
}

/// Production default: use `crypto.getRandomValues` for the
/// challenge + IV. Exposed so the identity dispatcher and tests
/// can share the same symbol.
export function cryptoRandomBytes(n: number): Uint8Array {
  return crypto.getRandomValues(new Uint8Array(n));
}

/// SubtleCrypto typings in TypeScript 5.7+ reject
/// `Uint8Array<ArrayBufferLike>` because `ArrayBufferLike` includes
/// `SharedArrayBuffer`. The shim never uses SharedArrayBuffer, so
/// we widen via a local helper rather than sprinkling casts across
/// every call site.
function asBufferSource(bytes: Uint8Array): BufferSource {
  return bytes as unknown as BufferSource;
}

/// Production default: wrap `navigator.credentials.create`.
export function defaultCreateCredential(
  opts: PublicKeyCredentialCreationOptions,
): Promise<PublicKeyCredential | null> {
  return navigator.credentials.create({
    publicKey: opts,
  }) as Promise<PublicKeyCredential | null>;
}
