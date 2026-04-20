// Identity acquisition (CON-3408 STEP 2). Reads priv_A from either the
// URL fragment (delegated-URL mode first-visit) or IndexedDB (subsequent
// visits). On a first-visit with a fragment this module also drives the
// TOFU wrap — see `tofu.ts` — so priv_A can be recovered on the next
// visit without the fragment (CON-3409). On a subsequent visit with
// no fragment (delegated-URL repeat OR hardened webauthn-prf mode)
// the unwrap branch (`unwrap.ts`) recovers priv_A from the persisted
// TOFU binding via a WebAuthn PRF ceremony.

import { base64UrlDecode, type CohortMode } from "./envelope.ts";
import {
  clearCachedPrivA,
  lookupCachedPrivA,
  storeCachedPrivA,
  type PolicyDeps,
  type SessionPolicy,
} from "./session_policy.ts";
import { readBindingRecord, type IdbFactoryLike } from "./storage.ts";
import {
  cryptoRandomBytes,
  defaultCreateCredential,
  performTofu,
  TofuError,
  type TofuDeps,
  type TofuResult,
} from "./tofu.ts";
import {
  performUnwrap,
  resolveGetCredential,
  UnwrapError,
  type UnwrapDeps,
} from "./unwrap.ts";

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
      | "tofu-failed"
      | "unwrap-failed",
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
  /// Dep overrides for the subsequent-visit unwrap flow. Production
  /// leaves this undefined and `unwrap.ts` resolves
  /// `navigator.credentials.get` + `crypto.subtle` at call time.
  unwrapDeps?: Partial<UnwrapDeps>;
  /// Override the IDB factory — tests inject fake-indexeddb here.
  idbFactory?: IdbFactoryLike | null;
  /// REQ-3417 `[access.session]` — gates how often
  /// `navigator.credentials.get()` is re-run. Defaults to `per-page`
  /// when omitted.
  sessionPolicy?: SessionPolicy;
  /// Dep overrides for the session-policy cache. Tests inject a
  /// fake sessionStorage + clock.
  policyDeps?: Omit<PolicyDeps, "policy">;
}

/// Returns the 32-byte raw X25519 private scalar (`priv_A`) used by age
/// to decrypt the cohort ciphertext. Three source branches:
///
///   1. URL fragment present (first-visit delegated-URL): decode
///      `#k=<b64url>`, run TOFU wrap best-effort, return the
///      fragment scalar.
///   2. URL fragment absent, IDB binding present (subsequent visit
///      in either delegated-URL or hardened mode): unwrap via
///      `navigator.credentials.get` + AES-GCM. REQ-3406 mandates
///      this branch is fragment-free.
///   3. Neither source available: `need-invite`.
///
/// REQ-3417 session-policy caching short-circuits branch 2 when a
/// cached priv_A is still fresh under the active policy; a stale or
/// absent cache entry triggers the full UV ceremony.
export async function acquireIdentity(
  ctx: IdentityContext,
): Promise<Uint8Array> {
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

  const unwrapped = await tryUnwrapBinding(ctx);
  if (unwrapped !== null) return unwrapped;

  throw new IdentityError(
    "need-invite",
    ctx.cohortMode === "webauthn-prf"
      ? "no stored passkey binding for this cohort on this device — ask your wiki operator for a fresh invite URL"
      : "no invite fragment in URL and no stored binding on this device — ask your wiki operator for a fresh invite URL",
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

/// Attempt to recover priv_A from a persisted TOFU binding via the
/// WebAuthn PRF unwrap ceremony. Returns `null` when no binding
/// exists (caller falls through to `need-invite`). Every other
/// failure — revoked authenticator, absent WebAuthn API, decrypt
/// mismatch — is reclassified into an `IdentityError` so the
/// pipeline renders a structured diagnostic.
///
/// On success, caches priv_A under the active
/// `[access.session]` policy so subsequent intra-session visits
/// can skip the UV prompt.
async function tryUnwrapBinding(
  ctx: IdentityContext,
): Promise<Uint8Array | null> {
  const factory = resolveFactoryOverride(ctx);
  if (factory === null) return null;
  const record = await readBindingRecord(ctx.cohortId, factory);
  if (record === null) return null;

  const origin = ctx.origin ?? resolveOrigin();
  const policyDeps = {
    ...(ctx.policyDeps ?? {}),
    policy: ctx.sessionPolicy,
  };
  const cacheKey = {
    origin,
    cohortId: ctx.cohortId,
    bindingCreatedAt: record.createdAt,
  };
  const cached = lookupCachedPrivA(cacheKey, policyDeps);
  if (cached !== null) return cached;

  const getCredential =
    ctx.unwrapDeps?.getCredential ?? resolveGetCredential();
  const subtle = ctx.unwrapDeps?.subtle ?? resolveSubtle();
  const randomBytes = ctx.unwrapDeps?.randomBytes ?? cryptoRandomBytes;

  if (!getCredential) {
    // REQ-3406 path requires WebAuthn. A runtime without
    // `credentials.get` means the reader's browser cannot unwrap
    // the stored binding — surface `need-invite` rather than
    // silently falling through to a misleading diagnostic. We
    // still clear any cached priv_A so a later browser upgrade
    // doesn't read a stale entry.
    clearCachedPrivA(cacheKey, policyDeps);
    throw new IdentityError(
      "unwrap-failed",
      "this browser does not expose navigator.credentials.get — ask your wiki operator for a fresh invite URL",
    );
  }
  if (!subtle) {
    throw new IdentityError(
      "unwrap-failed",
      "SubtleCrypto is unavailable — cannot unwrap the stored binding",
    );
  }

  const deps: UnwrapDeps = {
    getCredential,
    subtle,
    randomBytes,
    idbFactory: factory,
  };
  try {
    const result = await performUnwrap({ cohortId: ctx.cohortId }, deps);
    storeCachedPrivA(cacheKey, result.privA, policyDeps);
    return result.privA;
  } catch (err) {
    // Any unwrap failure invalidates the session-policy cache so a
    // subsequent visit re-runs the UV ceremony rather than reading
    // a stale priv_A.
    clearCachedPrivA(cacheKey, policyDeps);
    if (err instanceof UnwrapError) {
      throw new IdentityError(
        "unwrap-failed",
        `subsequent-visit unwrap failed (${err.kind}): ${err.message}`,
      );
    }
    throw err;
  }
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
