// Subsequent-visit unwrap branch (SPEC-034 REQ-3406 / CON-3408
// STEP 2 / CON-3409). Inverse of `tofu.ts::performTofu`:
//
//     binding    = await idb.get(cohortId)
//     credential = await navigator.credentials.get({
//                    publicKey: { challenge, allowCredentials: [...binding.credentialId],
//                                 extensions: { prf: { eval: { first: binding.prfSalt } } } } })
//     prf_output = credential.getClientExtensionResults().prf.results.first
//     K_wrap     = HKDF-SHA256(prf_output, "", "zetl/tofu-wrap/v1", 32)
//     priv_A     = AES-256-GCM-decrypt(K_wrap, binding.iv, binding.aad, binding.ciphertext)
//
// Per REQ-3406 this path is fragment-free: priv_A never transits the
// URL on a subsequent visit. If the authenticator has been deleted /
// revoked, the unwrap surfaces a structured `authenticator-unavailable`
// diagnostic that `errors.ts` maps to the "re-invite needed" copy.

import { hkdf } from "@noble/hashes/hkdf";
import { sha256 } from "@noble/hashes/sha2";

import { tofuAad } from "./prf_salt.ts";
import { readBindingRecord, type IdbFactoryLike, type TofuBinding } from "./storage.ts";
import { cryptoRandomBytes, K_WRAP_LEN, PRF_OUTPUT_LEN, TOFU_WRAP_INFO } from "./tofu.ts";

/// Challenge length matches the TOFU wrap side — 32 bytes is the
/// WebAuthn recommendation (not a security floor in the unwrap
/// direction, since `prf.eval.first` is the actual cryptographic
/// input).
export const GET_CHALLENGE_LEN = 32;

export class UnwrapError extends Error {
  override readonly name = "UnwrapError";
  constructor(
    readonly kind:
      | "no-binding"
      | "authenticator-unavailable"
      | "get-rejected"
      | "no-prf"
      | "decrypt-failed"
      | "subtle-unavailable"
      | "storage-failed",
    message: string,
  ) {
    super(message);
  }
}

/// Injection surface — mirror of `TofuDeps`. Production callers
/// resolve each to the browser built-in; tests inject fakes to
/// simulate revoked authenticators, missing PRF output, and
/// tampered ciphertexts.
export interface UnwrapDeps {
  /// `navigator.credentials.get` (or a moniker). Must return a
  /// credential whose `getClientExtensionResults()` carries
  /// `{ prf: { results: { first: ArrayBuffer } } }`. A revoked
  /// authenticator is surfaced by throwing — the unwrap orchestrator
  /// catches and reclassifies as `authenticator-unavailable`.
  getCredential: (
    opts: PublicKeyCredentialRequestOptions,
  ) => Promise<PublicKeyCredential | null>;
  /// SubtleCrypto surface for AES-GCM decrypt.
  subtle: SubtleCrypto;
  /// 32-byte challenge RNG. Tests inject deterministic fixtures.
  randomBytes: (n: number) => Uint8Array;
  /// Override the IDB factory — tests use `fake-indexeddb`.
  idbFactory?: IdbFactoryLike | null;
}

export interface UnwrapInput {
  cohortId: string;
}

export interface UnwrapResult {
  /// The recovered 32-byte X25519 scalar.
  privA: Uint8Array;
  /// The binding record used — surfaced for the session-policy
  /// cache which keys on origin+cohortId+createdAt so replacement
  /// of the record (e.g. after operator re-issue) invalidates the
  /// cache.
  binding: TofuBinding;
}

/// Run the subsequent-visit unwrap. Caller holds the
/// `navigator.locks` exclusive lock (pipeline.ts) — the unwrap does
/// not re-acquire it. Throws `UnwrapError` on every failure path so
/// the identity dispatcher can map the error to a reader-facing
/// diagnostic.
export async function performUnwrap(
  input: UnwrapInput,
  deps: UnwrapDeps,
): Promise<UnwrapResult> {
  let binding: TofuBinding | null;
  try {
    binding = await readBindingRecord(input.cohortId, deps.idbFactory);
  } catch (err) {
    throw new UnwrapError(
      "storage-failed",
      `IDB read for cohort ${JSON.stringify(input.cohortId)} failed: ${(err as Error).message}`,
    );
  }
  if (binding === null) {
    throw new UnwrapError(
      "no-binding",
      `no stored TOFU binding for cohort ${JSON.stringify(input.cohortId)} on this device`,
    );
  }

  if (!deps.subtle) {
    throw new UnwrapError(
      "subtle-unavailable",
      "SubtleCrypto is unavailable — the unwrap branch cannot run",
    );
  }

  const challenge = deps.randomBytes(GET_CHALLENGE_LEN);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const requestOptions: any = {
    challenge,
    allowCredentials: [
      {
        id: binding.credentialId,
        type: "public-key",
        // Authenticator transports are deployment-dependent; omitting
        // the hint lets the platform surface any bound device.
      },
    ],
    userVerification: "required",
    extensions: {
      prf: { eval: { first: binding.prfSalt } },
    },
  };

  let credential: PublicKeyCredential | null;
  try {
    credential = await deps.getCredential(
      requestOptions as PublicKeyCredentialRequestOptions,
    );
  } catch (err) {
    // A revoked / deleted passkey surfaces here as a thrown
    // DOMException (NotAllowedError, InvalidStateError, etc.). We
    // classify all of them as `authenticator-unavailable` so the
    // reader is shown the "re-invite needed" copy rather than a
    // generic failure.
    throw new UnwrapError(
      "authenticator-unavailable",
      `navigator.credentials.get rejected: ${(err as Error).message}`,
    );
  }
  if (!credential) {
    throw new UnwrapError(
      "authenticator-unavailable",
      "navigator.credentials.get returned null — the passkey is no longer available on this device",
    );
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const ext: any = (credential as PublicKeyCredential)
    .getClientExtensionResults?.();
  const firstBuf: ArrayBuffer | undefined = ext?.prf?.results?.first;
  if (!firstBuf || firstBuf.byteLength === 0) {
    throw new UnwrapError(
      "no-prf",
      "authenticator did not return a PRF output during get() — the browser or device may have dropped WebAuthn PRF support",
    );
  }
  const prfOutput = new Uint8Array(firstBuf);
  if (prfOutput.length !== PRF_OUTPUT_LEN) {
    throw new UnwrapError(
      "no-prf",
      `PRF output was ${prfOutput.length} bytes; CON-3409 expects ${PRF_OUTPUT_LEN}`,
    );
  }

  const kWrap = hkdf(
    sha256,
    prfOutput,
    new Uint8Array(0),
    new TextEncoder().encode(TOFU_WRAP_INFO),
    K_WRAP_LEN,
  );

  // Reconstruct the AAD from binding.origin + binding.cohortId. The
  // wrap side stored the exact AAD bytes it used, but re-deriving
  // means a tampered IDB record with a mismatched AAD field is
  // caught by the GCM tag check — defence-in-depth against
  // record-field tampering.
  const aad = tofuAad(binding.origin, binding.cohortId);

  let plaintext: Uint8Array;
  try {
    const key = await deps.subtle.importKey(
      "raw",
      asBufferSource(kWrap),
      { name: "AES-GCM" },
      false,
      ["decrypt"],
    );
    const buf = await deps.subtle.decrypt(
      {
        name: "AES-GCM",
        iv: asBufferSource(binding.iv),
        additionalData: asBufferSource(aad),
        tagLength: 128,
      },
      key,
      asBufferSource(binding.ciphertext),
    );
    plaintext = new Uint8Array(buf);
  } catch (err) {
    throw new UnwrapError(
      "decrypt-failed",
      `AES-256-GCM unwrap failed: ${(err as Error).message}`,
    );
  }

  if (plaintext.length !== 32) {
    throw new UnwrapError(
      "decrypt-failed",
      `unwrapped priv_A is ${plaintext.length} bytes; expected 32`,
    );
  }

  return { privA: plaintext, binding };
}

/// Production default: `navigator.credentials.get` wrapped so the
/// identity dispatcher and tests can share the same symbol. Pairs
/// with `defaultCreateCredential` from tofu.ts.
export function defaultGetCredential(
  opts: PublicKeyCredentialRequestOptions,
): Promise<PublicKeyCredential | null> {
  return navigator.credentials.get({
    publicKey: opts,
  }) as Promise<PublicKeyCredential | null>;
}

/// Resolve the production `getCredential` binding. Returns `null`
/// when `navigator.credentials.get` is absent so the dispatcher can
/// short-circuit to `authenticator-unavailable` rather than throwing
/// an opaque `TypeError`.
export function resolveGetCredential():
  | ((o: PublicKeyCredentialRequestOptions) => Promise<PublicKeyCredential | null>)
  | null {
  if (typeof navigator === "undefined") return null;
  if (!navigator.credentials || typeof navigator.credentials.get !== "function") {
    return null;
  }
  return defaultGetCredential;
}

/// Re-export the RNG default so dispatchers don't need to pick
/// between importing from tofu.ts vs unwrap.ts.
export { cryptoRandomBytes };

/// SubtleCrypto typings in TS 5.7+ reject `Uint8Array<ArrayBufferLike>`
/// because `ArrayBufferLike` includes `SharedArrayBuffer`. Widen via a
/// local helper rather than sprinkling casts at every call site —
/// matches the identical helper in tofu.ts.
function asBufferSource(bytes: Uint8Array): BufferSource {
  return bytes as unknown as BufferSource;
}
