// Session-policy caching for the subsequent-visit unwrap branch
// (SPEC-034 REQ-3417 `[access.session]`). The policy controls how
// often the reader must re-authenticate via `credentials.get()` on
// this origin:
//
//   * per-page    — default. Every page load calls credentials.get().
//                   No cache. Matches the un-cached shim behaviour.
//   * per-session — cache priv_A in sessionStorage; valid until the
//                   tab closes. A reader navigating across intra-wiki
//                   pages inside a single tab re-uses priv_A without
//                   a fresh UV prompt.
//   * per-minute  — cache priv_A in sessionStorage with a 60-second
//                   wall-clock expiry; subsequent visits within the
//                   window skip UV.
//
// sessionStorage is origin- and tab-scoped by the browser. Storing
// the 32-byte priv_A there is an explicit trade-off operators opt
// into when they pick `per-session` or `per-minute`: tab crash
// clears the cache, cross-tab does not share it, extensions with
// full storage access can read it. The default `per-page` keeps
// priv_A in memory only, matching v0.4.0 behaviour.

export type SessionPolicy = "per-page" | "per-session" | "per-minute";
export const SESSION_POLICY_DEFAULT: SessionPolicy = "per-page";
export const SESSION_PER_MINUTE_TTL_MS = 60_000;
export const SESSION_STORAGE_KEY_PREFIX = "zetl:cap:uv:";

export interface CacheKey {
  /// origin string (defence-in-depth; sessionStorage is already
  /// origin-keyed by the browser).
  origin: string;
  cohortId: string;
  /// `createdAt` from the backing TOFU binding. When the operator
  /// re-issues a binding the createdAt changes → the cache key
  /// changes → any stale cached priv_A is ignored rather than
  /// shadowing the new record.
  bindingCreatedAt: number;
}

/// Minimal sessionStorage surface — tests inject an in-memory fake
/// so we don't depend on happy-dom's storage polyfill or risk
/// leaking keys across test files.
export interface SessionStorageLike {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

export interface PolicyDeps {
  /// Policy knob. Defaults to `per-page` when absent.
  policy?: SessionPolicy;
  /// sessionStorage surface. Resolved from `globalThis.sessionStorage`
  /// when omitted; tests inject a fake.
  storage?: SessionStorageLike | null;
  /// Clock source. Defaults to `Date.now`.
  now?: () => number;
}

interface CachedEntry {
  privAB64Url: string;
  bindingCreatedAt: number;
  /// Absolute wall-clock expiry (ms since epoch). Omitted for
  /// `per-session` entries — those live until sessionStorage is
  /// cleared by the browser.
  expiresAt?: number;
}

/// Look up a cached priv_A for the given cache key. Returns `null`
/// when the policy is `per-page`, when no cache entry exists, when
/// the entry is expired, or when the stored binding timestamp does
/// not match the current record (operator re-bind invalidates).
export function lookupCachedPrivA(
  key: CacheKey,
  deps: PolicyDeps = {},
): Uint8Array | null {
  const policy = deps.policy ?? SESSION_POLICY_DEFAULT;
  if (policy === "per-page") return null;

  const storage = resolveStorage(deps.storage);
  if (storage === null) return null;

  const raw = storage.getItem(storageKey(key));
  if (raw === null) return null;

  let entry: CachedEntry;
  try {
    entry = JSON.parse(raw) as CachedEntry;
  } catch {
    // Malformed cache entry — discard so a subsequent write replaces
    // it with a valid record.
    storage.removeItem(storageKey(key));
    return null;
  }

  if (entry.bindingCreatedAt !== key.bindingCreatedAt) {
    storage.removeItem(storageKey(key));
    return null;
  }

  if (entry.expiresAt !== undefined) {
    const now = (deps.now ?? (() => Date.now()))();
    if (now >= entry.expiresAt) {
      storage.removeItem(storageKey(key));
      return null;
    }
  }

  try {
    return base64UrlDecode(entry.privAB64Url);
  } catch {
    storage.removeItem(storageKey(key));
    return null;
  }
}

/// Persist `privA` under the policy. No-op when the policy is
/// `per-page` or sessionStorage is unavailable. Idempotent: writes
/// replace rather than append.
export function storeCachedPrivA(
  key: CacheKey,
  privA: Uint8Array,
  deps: PolicyDeps = {},
): void {
  const policy = deps.policy ?? SESSION_POLICY_DEFAULT;
  if (policy === "per-page") return;
  if (privA.length !== 32) return;

  const storage = resolveStorage(deps.storage);
  if (storage === null) return;

  const entry: CachedEntry = {
    privAB64Url: base64UrlEncode(privA),
    bindingCreatedAt: key.bindingCreatedAt,
  };
  if (policy === "per-minute") {
    const now = (deps.now ?? (() => Date.now()))();
    entry.expiresAt = now + SESSION_PER_MINUTE_TTL_MS;
  }
  try {
    storage.setItem(storageKey(key), JSON.stringify(entry));
  } catch {
    // Quota exhaustion / disabled storage — drop the cache rather
    // than surfacing. Caller re-runs UV on the next visit.
  }
}

/// Delete the cache entry for a key. Used when unwrap fails so
/// stale priv_A doesn't shadow a freshly-issued binding.
export function clearCachedPrivA(key: CacheKey, deps: PolicyDeps = {}): void {
  const storage = resolveStorage(deps.storage);
  if (storage === null) return;
  storage.removeItem(storageKey(key));
}

/// Parse the `[access.session]` TOML knob. Accepts the three
/// documented values; every other input (including `undefined`)
/// falls back to the default. Exposed separately so the build-side
/// TOML parser can normalise the string before threading it into the
/// shim bundle.
export function parseSessionPolicy(raw: unknown): SessionPolicy {
  if (raw === "per-session" || raw === "per-minute" || raw === "per-page") {
    return raw;
  }
  return SESSION_POLICY_DEFAULT;
}

function storageKey(key: CacheKey): string {
  return `${SESSION_STORAGE_KEY_PREFIX}${key.origin}:${key.cohortId}`;
}

function resolveStorage(
  override: SessionStorageLike | null | undefined,
): SessionStorageLike | null {
  if (override === null) return null;
  if (override !== undefined) return override;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const g = globalThis as any;
  if (g.sessionStorage) return g.sessionStorage as SessionStorageLike;
  return null;
}

function base64UrlEncode(bytes: Uint8Array): string {
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "");
}

function base64UrlDecode(s: string): Uint8Array {
  const normalised = s.replace(/-/g, "+").replace(/_/g, "/");
  const padLen = (4 - (normalised.length % 4)) % 4;
  const padded = normalised + "=".repeat(padLen);
  const bin = atob(padded);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}
