// Identity acquisition (CON-3408 STEP 2). Reads priv_A from either the
// URL fragment (delegated-URL mode first-visit) or IndexedDB (subsequent
// visits). Branch-specific work — generating a passkey via
// `credentials.create()` on TOFU, unwrapping via `credentials.get()` on
// subsequent visits — lives in the companion tasks `task-cap-shim-tofu`
// and `task-cap-shim-unwrap`. This module owns the dispatcher and the
// `need-invite` error path the core pipeline must return.

import { base64UrlDecode, type CohortMode } from "./envelope.ts";

/// Fragment prefix pinned by CON-3401: `#k=<43-char-base64url>`.
export const FRAGMENT_PREFIX = "k=";
export const PRIV_A_B64URL_LEN = 43;
export const PRIV_A_RAW_LEN = 32;

export class IdentityError extends Error {
  override readonly name = "IdentityError";
  constructor(
    readonly kind:
      | "need-invite"
      | "malformed-fragment"
      | "fragment-length"
      | "fragment-base64"
      | "mode-not-supported",
    message: string,
  ) {
    super(message);
  }
}

export interface IdentityContext {
  cohortId: string;
  cohortMode: CohortMode;
  /// Page location — injected so tests can simulate fragments without
  /// stubbing `window.location`.
  locationHash: string;
}

/// Returns the 32-byte raw X25519 private scalar (`priv_A`) used by age
/// to decrypt the cohort ciphertext. In v1 core this resolves the
/// delegated-URL branch (fragment or existing IDB binding); the TOFU
/// `create()` path and the unwrap `get()` path are implemented in
/// downstream tasks which call into this module.
export async function acquireIdentity(
  ctx: IdentityContext,
): Promise<Uint8Array> {
  if (ctx.cohortMode === "webauthn-prf") {
    // Hardened mode: no fragment, identity lives in a per-cohort PRF
    // binding. `task-cap-shim-unwrap` + `task-cap-enrolment-page` own
    // this branch; v1 core refuses it cleanly so partial deploys fail
    // loudly rather than silently rendering blank content.
    throw new IdentityError(
      "mode-not-supported",
      "hardened mode (webauthn-prf) requires the TOFU/unwrap branch — not yet wired in v1 core",
    );
  }

  const fromFragment = readFragmentKey(ctx.locationHash);
  if (fromFragment !== null) return fromFragment;

  const fromBinding = await readBinding(ctx.cohortId);
  if (fromBinding !== null) return fromBinding;

  throw new IdentityError(
    "need-invite",
    "no invite fragment in URL and no stored binding on this device — ask your wiki operator for a fresh invite URL",
  );
}

/// Parse `#k=<43 b64url chars>` to a 32-byte raw scalar. Returns `null`
/// when the fragment is absent (no error — the caller falls back to an
/// IDB lookup). Throws on a present-but-malformed fragment so
/// typo/tampering is surfaced rather than silently ignored.
export function readFragmentKey(locationHash: string): Uint8Array | null {
  let hash = locationHash;
  if (hash.startsWith("#")) hash = hash.slice(1);
  if (hash.length === 0) return null;

  if (!hash.startsWith(FRAGMENT_PREFIX)) {
    throw new IdentityError(
      "malformed-fragment",
      `URL fragment does not begin with ${FRAGMENT_PREFIX} — expected #k=<priv_A>`,
    );
  }
  const payload = hash.slice(FRAGMENT_PREFIX.length);
  if (payload.length !== PRIV_A_B64URL_LEN) {
    throw new IdentityError(
      "fragment-length",
      `fragment key is ${payload.length} chars; expected ${PRIV_A_B64URL_LEN}`,
    );
  }
  if (!/^[A-Za-z0-9_-]+$/.test(payload)) {
    throw new IdentityError(
      "fragment-base64",
      "fragment contains non-base64url characters",
    );
  }
  let raw: Uint8Array;
  try {
    raw = base64UrlDecode(payload);
  } catch (err) {
    throw new IdentityError(
      "fragment-base64",
      `fragment is not valid base64url: ${(err as Error).message}`,
    );
  }
  if (raw.length !== PRIV_A_RAW_LEN) {
    throw new IdentityError(
      "fragment-length",
      `fragment decodes to ${raw.length} bytes; expected ${PRIV_A_RAW_LEN}`,
    );
  }
  return raw;
}

/// Stub for the subsequent-visit IDB read. Returns `null` — the TOFU /
/// unwrap tasks replace the body with a real `navigator.credentials.get`
/// + AES-GCM unwrap flow. Kept as a function boundary so downstream
/// tasks can monkey-patch via module swap during their test harnesses.
export async function readBinding(
  _cohortId: string,
): Promise<Uint8Array | null> {
  return null;
}
