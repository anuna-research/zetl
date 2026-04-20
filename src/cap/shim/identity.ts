// Identity acquisition (CON-3408 STEP 2). Reads priv_A from either the
// URL fragment (delegated-URL mode first-visit) or IndexedDB (subsequent
// visits). On a first-visit with a fragment this module also drives the
// TOFU wrap — see `tofu.ts` — so priv_A can be recovered on the next
// visit without the fragment (CON-3409).

import { base64UrlDecode, type CohortMode } from "./envelope.ts";
import { readBindingRecord, type IdbFactoryLike } from "./storage.ts";
import {
  cryptoRandomBytes,
  defaultCreateCredential,
  performTofu,
  TofuError,
  type TofuDeps,
  type TofuResult,
} from "./tofu.ts";

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
      | "mode-not-supported"
      | "tofu-failed",
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
  /// Origin string passed to the TOFU salt + AAD derivations. Injected
  /// so tests don't need to stub `window.location.origin`.
  origin?: string;
  /// Dep overrides for the TOFU flow. Production leaves this undefined
  /// and the TOFU module resolves `navigator.credentials.create` +
  /// `crypto.subtle` at call time.
  tofuDeps?: Partial<TofuDeps>;
  /// Override the IDB factory — tests inject fake-indexeddb here.
  idbFactory?: IdbFactoryLike | null;
}

/// Returns the 32-byte raw X25519 private scalar (`priv_A`) used by age
/// to decrypt the cohort ciphertext. In delegated-URL mode the
/// fragment is the source of truth for the current visit; if there
/// is no existing IDB binding this call also drives the TOFU wrap so
/// the next visit can unwrap from IDB without the fragment
/// (CON-3409). Hardened (`webauthn-prf`) mode still throws
/// `"mode-not-supported"` here — `task-cap-shim-unwrap` lands that
/// branch.
export async function acquireIdentity(
  ctx: IdentityContext,
): Promise<Uint8Array> {
  if (ctx.cohortMode === "webauthn-prf") {
    throw new IdentityError(
      "mode-not-supported",
      "hardened mode (webauthn-prf) requires the subsequent-visit unwrap branch — not yet wired in v1 core",
    );
  }

  const fromFragment = readFragmentKey(ctx.locationHash);
  if (fromFragment !== null) {
    // First-visit path: try to persist a passkey-wrapped copy of
    // priv_A so subsequent visits can unwrap without the fragment
    // (CON-3409 TOFU). This is best-effort: if the runtime has no
    // WebAuthn / IndexedDB / SubtleCrypto surface (REQ-3412
    // non-PRF fallback environment, Node-under-test without
    // polyfills, etc.) the call silently returns. The fragment
    // is still usable for *this* visit; the reader just won't
    // get persistence. Passkey-creation failures while the
    // runtime *does* expose the APIs still propagate so the
    // reader learns their authenticator refused.
    await maybeBindFragment(ctx, fromFragment);
    return fromFragment;
  }

  const fromBinding = await readBinding(ctx.cohortId, ctx.idbFactory);
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

/// Read a persisted TOFU binding. Returns the raw priv_A when the
/// record unwraps successfully, `null` when no record is present.
/// The unwrap side (AES-GCM + `credentials.get()`) is owned by the
/// subsequent-visit task; v1 of this module reports "record exists"
/// via the storage layer for the TOFU idempotency check but cannot
/// yet recover priv_A without the fragment.
export async function readBinding(
  cohortId: string,
  factory?: IdbFactoryLike | null,
): Promise<Uint8Array | null> {
  const record = await readBindingRecord(cohortId, factory ?? undefined);
  if (record === null) return null;
  // Presence without unwrap capability means we can't return a
  // priv_A — the fragment path is the only v1 source. Returning
  // null lets the pipeline fall through to `need-invite`, which is
  // the correct diagnostic until `task-cap-shim-unwrap` lands.
  return null;
}

async function maybeBindFragment(
  ctx: IdentityContext,
  privA: Uint8Array,
): Promise<TofuResult | null> {
  const origin = ctx.origin ?? resolveOrigin();
  const createCredential = ctx.tofuDeps?.createCredential
    ?? resolveCreateCredential();
  const subtle = ctx.tofuDeps?.subtle ?? resolveSubtle();
  const randomBytes = ctx.tofuDeps?.randomBytes ?? cryptoRandomBytes;
  // An explicitly-`null` idbFactory disables TOFU (REQ-3412
  // fallback); `undefined` means "use the runtime default".
  const idbFactory = resolveFactoryOverride(ctx);

  // Runtime without any of the three surfaces → REQ-3412 fallback:
  // skip TOFU silently, let the caller use the fragment for this
  // visit only. The dedicated fallback task layers on the UX
  // notice.
  if (!createCredential || !subtle || idbFactory === null) {
    return null;
  }

  const deps: TofuDeps = {
    createCredential,
    subtle,
    randomBytes,
    now: ctx.tofuDeps?.now ?? (() => Date.now()),
    idbFactory,
  };
  try {
    return await performTofu(
      { cohortId: ctx.cohortId, origin, privA },
      deps,
    );
  } catch (err) {
    if (err instanceof TofuError) {
      throw new IdentityError(
        "tofu-failed",
        `TOFU binding failed (${err.kind}): ${err.message}`,
      );
    }
    throw err;
  }
}

function resolveCreateCredential():
  | ((o: PublicKeyCredentialCreationOptions) => Promise<PublicKeyCredential | null>)
  | null {
  if (typeof navigator === "undefined") return null;
  if (!navigator.credentials || typeof navigator.credentials.create !== "function") {
    return null;
  }
  return defaultCreateCredential;
}

function resolveSubtle(): SubtleCrypto | null {
  if (typeof crypto === "undefined") return null;
  return crypto.subtle ?? null;
}

function resolveIdbFactory(): IdbFactoryLike | null {
  if (typeof indexedDB !== "undefined") return indexedDB;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const g = globalThis as any;
  if (g.indexedDB) return g.indexedDB as IdbFactoryLike;
  return null;
}

/// Distinguish "caller did not set `idbFactory`" (fall back to the
/// runtime default) from "caller set `idbFactory: null`" (disable
/// TOFU entirely — REQ-3412 fragment-only fallback). The nullish-
/// coalescing chain in `maybeBindFragment` collapses the two cases;
/// this helper inspects the raw property presence instead.
function resolveFactoryOverride(
  ctx: IdentityContext,
): IdbFactoryLike | null {
  const tofuOverride = ctx.tofuDeps;
  if (tofuOverride && "idbFactory" in tofuOverride) {
    return tofuOverride.idbFactory ?? null;
  }
  if ("idbFactory" in ctx) {
    return ctx.idbFactory ?? null;
  }
  return resolveIdbFactory();
}

function resolveOrigin(): string {
  if (typeof location !== "undefined" && typeof location.origin === "string") {
    return location.origin;
  }
  // Degenerate case — happy-dom / node environments without a
  // synthesised location. Callers under test should pass
  // `ctx.origin` explicitly.
  return "http://localhost";
}
