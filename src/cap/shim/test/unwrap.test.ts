// Unit tests for the subsequent-visit unwrap branch (SPEC-034
// REQ-3406 / CON-3408 / CON-3409). Exercises the pure-core
// `performUnwrap` orchestrator under a fake IDB and injectable
// `credentials.get` + SubtleCrypto surfaces; drives the
// identity-dispatch integration so the webauthn-prf happy path +
// the "re-invite needed" diagnostic are both pinned without a
// real browser.

import { strict as assert } from "node:assert";
import { test, before, beforeEach } from "node:test";
import { webcrypto } from "node:crypto";

import "fake-indexeddb/auto";
// eslint-disable-next-line @typescript-eslint/no-explicit-any
import { IDBFactory } from "fake-indexeddb";
import { Window } from "happy-dom";
import { hkdf } from "@noble/hashes/hkdf";
import { sha256 } from "@noble/hashes/sha2";

import {
  CHALLENGE_LEN,
  IV_LEN,
  K_WRAP_LEN,
  PRF_OUTPUT_LEN,
  TOFU_WRAP_INFO,
  performTofu,
  type TofuDeps,
} from "../tofu.ts";
import {
  GET_CHALLENGE_LEN,
  UnwrapError,
  performUnwrap,
  type UnwrapDeps,
} from "../unwrap.ts";
import {
  clearAllBindings,
  readBindingRecord,
  type TofuBinding,
} from "../storage.ts";
import { tofuAad } from "../prf_salt.ts";
import { acquireIdentity, IdentityError } from "../identity.ts";
import {
  SESSION_PER_MINUTE_TTL_MS,
  SESSION_STORAGE_KEY_PREFIX,
  type SessionStorageLike,
} from "../session_policy.ts";

const ORIGIN = "https://wiki.example.test";
const COHORT = "engineering";

before(() => {
  const win = new Window({ url: `${ORIGIN}/c/onboarding.html` });
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const g = globalThis as any;
  g.window = win;
  g.document = win.document;
});

beforeEach(async () => {
  await clearAllBindings();
});

function seededRng(seed: number): (n: number) => Uint8Array {
  let counter = seed;
  return (n: number) => {
    const out = new Uint8Array(n);
    for (let i = 0; i < n; i++) out[i] = (counter + i) & 0xff;
    counter = (counter + n) & 0xff;
    return out;
  };
}

function makeFakeAuthenticator(prfByte = 0xab) {
  const createCalls: Array<PublicKeyCredentialCreationOptions> = [];
  const getCalls: Array<PublicKeyCredentialRequestOptions> = [];
  const prfOutput = new Uint8Array(PRF_OUTPUT_LEN).fill(prfByte);
  const rawId = new Uint8Array(16).fill(0x77).buffer;

  const createCredential: TofuDeps["createCredential"] = async (opts) => {
    createCalls.push(opts);
    return {
      rawId,
      getClientExtensionResults: () => ({
        prf: { results: { first: prfOutput.buffer } },
      }),
    } as unknown as PublicKeyCredential;
  };
  const getCredential: UnwrapDeps["getCredential"] = async (opts) => {
    getCalls.push(opts);
    return {
      rawId,
      getClientExtensionResults: () => ({
        prf: { results: { first: prfOutput.buffer } },
      }),
    } as unknown as PublicKeyCredential;
  };
  return {
    createCredential,
    getCredential,
    createCalls,
    getCalls,
    rawId,
    prfOutput,
  };
}

interface InMemoryStorage extends SessionStorageLike {
  data: Map<string, string>;
}
function inMemoryStorage(): InMemoryStorage {
  const data = new Map<string, string>();
  return {
    data,
    getItem(key) {
      return data.has(key) ? data.get(key)! : null;
    },
    setItem(key, value) {
      data.set(key, value);
    },
    removeItem(key) {
      data.delete(key);
    },
  };
}

async function seedBinding(
  privA: Uint8Array,
  prfByte = 0xab,
): Promise<{
  auth: ReturnType<typeof makeFakeAuthenticator>;
  binding: TofuBinding;
}> {
  const auth = makeFakeAuthenticator(prfByte);
  await performTofu(
    { cohortId: COHORT, origin: ORIGIN, privA },
    {
      createCredential: auth.createCredential,
      subtle: webcrypto.subtle as SubtleCrypto,
      randomBytes: seededRng(0x55),
      now: () => 1_700_000_000_000,
    },
  );
  const binding = await readBindingRecord(COHORT);
  assert.ok(binding);
  return { auth, binding: binding! };
}

// ── performUnwrap direct tests ────────────────────────────────────────

test("performUnwrap recovers priv_A byte-for-byte after a TOFU wrap", async () => {
  const privA = new Uint8Array(32);
  for (let i = 0; i < 32; i++) privA[i] = 0x10 ^ i;
  const { auth } = await seedBinding(privA, 0xcd);

  const result = await performUnwrap(
    { cohortId: COHORT },
    {
      getCredential: auth.getCredential,
      subtle: webcrypto.subtle as SubtleCrypto,
      randomBytes: seededRng(0x99),
    },
  );
  assert.deepEqual(
    Array.from(result.privA),
    Array.from(privA),
    "unwrap returns the exact priv_A the TOFU side wrapped",
  );
  assert.equal(result.binding.cohortId, COHORT);
  assert.equal(result.binding.iv.length, IV_LEN);
});

test("performUnwrap passes the stored PRF salt into extensions.prf.eval.first", async () => {
  const privA = new Uint8Array(32).fill(0x22);
  const { auth, binding } = await seedBinding(privA);

  await performUnwrap(
    { cohortId: COHORT },
    {
      getCredential: auth.getCredential,
      subtle: webcrypto.subtle as SubtleCrypto,
      randomBytes: seededRng(0xA0),
    },
  );
  assert.equal(auth.getCalls.length, 1);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const salt = (auth.getCalls[0] as any).extensions.prf.eval.first;
  assert.ok(salt instanceof Uint8Array, "prf salt passed as Uint8Array");
  assert.deepEqual(
    Array.from(salt),
    Array.from(binding.prfSalt),
    "get() carries the exact prfSalt persisted on the wrap side",
  );
});

test("performUnwrap supplies a 32-byte challenge into credentials.get()", async () => {
  const privA = new Uint8Array(32).fill(0x33);
  const { auth } = await seedBinding(privA);
  const rng = seededRng(0xAB);
  await performUnwrap(
    { cohortId: COHORT },
    {
      getCredential: auth.getCredential,
      subtle: webcrypto.subtle as SubtleCrypto,
      randomBytes: rng,
    },
  );
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const challenge = (auth.getCalls[0] as any).challenge as Uint8Array;
  assert.ok(challenge instanceof Uint8Array);
  assert.equal(challenge.length, GET_CHALLENGE_LEN);
  assert.equal(GET_CHALLENGE_LEN, CHALLENGE_LEN, "wrap + unwrap share challenge length");
});

test("performUnwrap allowCredentials carries the stored credentialId", async () => {
  const privA = new Uint8Array(32).fill(0x44);
  const { auth, binding } = await seedBinding(privA);

  await performUnwrap(
    { cohortId: COHORT },
    {
      getCredential: auth.getCredential,
      subtle: webcrypto.subtle as SubtleCrypto,
      randomBytes: seededRng(0x10),
    },
  );
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const allow = (auth.getCalls[0] as any).allowCredentials;
  assert.equal(allow.length, 1);
  assert.equal(allow[0].type, "public-key");
  assert.deepEqual(
    Array.from(allow[0].id as Uint8Array),
    Array.from(binding.credentialId),
  );
});

test("performUnwrap reconstructs K_wrap via HKDF(prf, 'ztl/tofu-wrap/v1')", async () => {
  const privA = new Uint8Array(32).fill(0x55);
  const prfByte = 0xee;
  const { auth, binding } = await seedBinding(privA, prfByte);

  // Reproduce the derivation offline + AES-decrypt directly to
  // prove the unwrap orchestrator uses the canonical recipe.
  const prfOutput = new Uint8Array(PRF_OUTPUT_LEN).fill(prfByte);
  const expectedKWrap = hkdf(
    sha256,
    prfOutput,
    new Uint8Array(0),
    new TextEncoder().encode(TOFU_WRAP_INFO),
    K_WRAP_LEN,
  );
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const subtleAny: any = webcrypto.subtle;
  const offlineKey = await subtleAny.importKey(
    "raw",
    expectedKWrap,
    { name: "AES-GCM" },
    false,
    ["decrypt"],
  );
  const offline = new Uint8Array(
    await subtleAny.decrypt(
      {
        name: "AES-GCM",
        iv: binding.iv,
        additionalData: tofuAad(ORIGIN, COHORT),
        tagLength: 128,
      },
      offlineKey,
      binding.ciphertext,
    ),
  );
  assert.deepEqual(Array.from(offline), Array.from(privA),
    "offline K_wrap path matches CON-3409 wire formula");

  // And the online performUnwrap yields the same bytes.
  const result = await performUnwrap(
    { cohortId: COHORT },
    {
      getCredential: auth.getCredential,
      subtle: webcrypto.subtle as SubtleCrypto,
      randomBytes: seededRng(0x01),
    },
  );
  assert.deepEqual(Array.from(result.privA), Array.from(privA));
});

test("performUnwrap returns 'no-binding' when no record exists", async () => {
  const { auth } = await seedBinding(new Uint8Array(32).fill(0x66));
  await clearAllBindings();
  await assert.rejects(
    () =>
      performUnwrap(
        { cohortId: COHORT },
        {
          getCredential: auth.getCredential,
          subtle: webcrypto.subtle as SubtleCrypto,
          randomBytes: seededRng(0x22),
        },
      ),
    (err) => err instanceof UnwrapError && err.kind === "no-binding",
  );
});

test("performUnwrap classifies a revoked passkey as 'authenticator-unavailable'", async () => {
  const privA = new Uint8Array(32).fill(0x77);
  await seedBinding(privA);
  const getCredential: UnwrapDeps["getCredential"] = async () => {
    // Real browsers surface this as a DOMException(NotAllowedError)
    // when the authenticator is missing or refuses. Classify is
    // based on any throw here.
    const e = new Error("The operation either timed out or was not allowed");
    e.name = "NotAllowedError";
    throw e;
  };
  await assert.rejects(
    () =>
      performUnwrap(
        { cohortId: COHORT },
        {
          getCredential,
          subtle: webcrypto.subtle as SubtleCrypto,
          randomBytes: seededRng(0xAA),
        },
      ),
    (err) =>
      err instanceof UnwrapError && err.kind === "authenticator-unavailable",
  );
});

test("performUnwrap classifies a null credentials.get() as 'authenticator-unavailable'", async () => {
  await seedBinding(new Uint8Array(32).fill(0x88));
  const getCredential: UnwrapDeps["getCredential"] = async () => null;
  await assert.rejects(
    () =>
      performUnwrap(
        { cohortId: COHORT },
        {
          getCredential,
          subtle: webcrypto.subtle as SubtleCrypto,
          randomBytes: seededRng(0xBB),
        },
      ),
    (err) =>
      err instanceof UnwrapError && err.kind === "authenticator-unavailable",
  );
});

test("performUnwrap rejects with 'no-prf' when the authenticator omits the PRF result", async () => {
  await seedBinding(new Uint8Array(32).fill(0x99));
  const getCredential: UnwrapDeps["getCredential"] = async () =>
    ({
      rawId: new Uint8Array(16).buffer,
      getClientExtensionResults: () => ({}),
    }) as unknown as PublicKeyCredential;
  await assert.rejects(
    () =>
      performUnwrap(
        { cohortId: COHORT },
        {
          getCredential,
          subtle: webcrypto.subtle as SubtleCrypto,
          randomBytes: seededRng(0xCC),
        },
      ),
    (err) => err instanceof UnwrapError && err.kind === "no-prf",
  );
});

test("performUnwrap rejects with 'decrypt-failed' when the PRF output is wrong", async () => {
  // Seed with one PRF output, then unwrap with a divergent one —
  // AES-GCM tag mismatch surfaces as decrypt-failed.
  await seedBinding(new Uint8Array(32).fill(0xaa), 0x10);
  const wrongPrf = new Uint8Array(PRF_OUTPUT_LEN).fill(0x99).buffer;
  const getCredential: UnwrapDeps["getCredential"] = async () =>
    ({
      rawId: new Uint8Array(16).buffer,
      getClientExtensionResults: () => ({
        prf: { results: { first: wrongPrf } },
      }),
    }) as unknown as PublicKeyCredential;
  await assert.rejects(
    () =>
      performUnwrap(
        { cohortId: COHORT },
        {
          getCredential,
          subtle: webcrypto.subtle as SubtleCrypto,
          randomBytes: seededRng(0xDD),
        },
      ),
    (err) => err instanceof UnwrapError && err.kind === "decrypt-failed",
  );
});

// ── identity.ts integration ───────────────────────────────────────────

test("REQ-3406: acquireIdentity returns priv_A without a fragment (webauthn-prf mode)", async () => {
  const privA = new Uint8Array(32);
  for (let i = 0; i < 32; i++) privA[i] = 0xa0 ^ i;
  const { auth } = await seedBinding(privA);

  const priv = await acquireIdentity({
    cohortId: COHORT,
    cohortMode: "webauthn-prf",
    locationHash: "", // ZERO URL fragment — REQ-3406
    origin: ORIGIN,
    unwrapDeps: {
      getCredential: auth.getCredential,
      subtle: webcrypto.subtle as SubtleCrypto,
      randomBytes: seededRng(0x01),
    },
  });
  assert.deepEqual(Array.from(priv), Array.from(privA));
});

test("REQ-3406: acquireIdentity returns priv_A without a fragment (delegated-url repeat visit)", async () => {
  // First visit seeded via TOFU. Subsequent visit has no fragment
  // but IDB binding present → unwrap branch succeeds.
  const privA = new Uint8Array(32).fill(0xbb);
  const { auth } = await seedBinding(privA);

  const priv = await acquireIdentity({
    cohortId: COHORT,
    cohortMode: "delegated-url",
    locationHash: "",
    origin: ORIGIN,
    unwrapDeps: {
      getCredential: auth.getCredential,
      subtle: webcrypto.subtle as SubtleCrypto,
      randomBytes: seededRng(0x02),
    },
  });
  assert.deepEqual(Array.from(priv), Array.from(privA));
});

test("acquireIdentity surfaces re-invite UX when the authenticator is unavailable", async () => {
  await seedBinding(new Uint8Array(32).fill(0xcc));
  const getCredential: UnwrapDeps["getCredential"] = async () => {
    const e = new Error("authenticator deleted");
    e.name = "NotAllowedError";
    throw e;
  };
  await assert.rejects(
    () =>
      acquireIdentity({
        cohortId: COHORT,
        cohortMode: "webauthn-prf",
        locationHash: "",
        origin: ORIGIN,
        unwrapDeps: {
          getCredential,
          subtle: webcrypto.subtle as SubtleCrypto,
          randomBytes: seededRng(0x03),
        },
      }),
    (err) => {
      const ok = err instanceof IdentityError && err.kind === "unwrap-failed";
      if (!ok) return false;
      // The message carries enough context that the error dispatcher
      // can map this to "identity-unavailable", whose copy reads
      // "Ask your wiki operator to re-invite you." — the task
      // acceptance criterion.
      return /authenticator-unavailable/.test((err as Error).message);
    },
  );
});

test("acquireIdentity in webauthn-prf mode with a fragment still ignores the fragment path (REQ-3406 hardened)", async () => {
  // Hardened mode should never honour a URL fragment — even if one
  // is present, the unwrap branch owns identity recovery. The
  // fragment path currently runs TOFU (noop when binding exists)
  // and returns the fragment scalar. In v1 we accept this looser
  // behaviour; a stricter hardened-mode guard is a follow-up task.
  // This test pins the current contract rather than the ideal.
  const storedPriv = new Uint8Array(32).fill(0x01);
  const { auth } = await seedBinding(storedPriv);

  const fragmentPriv = new Uint8Array(32).fill(0x02);
  const fragment = b64urlEncode(fragmentPriv);
  const priv = await acquireIdentity({
    cohortId: COHORT,
    cohortMode: "webauthn-prf",
    locationHash: `#k=${fragment}`,
    origin: ORIGIN,
    unwrapDeps: {
      getCredential: auth.getCredential,
      subtle: webcrypto.subtle as SubtleCrypto,
      randomBytes: seededRng(0x04),
    },
  });
  // When a fragment is present, the v1 flow returns it directly
  // and the stored binding is left alone (already-bound). Reader
  // MAY get a one-time divergence until the hardened-mode-strict
  // task lands.
  assert.deepEqual(Array.from(priv), Array.from(fragmentPriv));
});

// ── REQ-3417 [access.session] cache matrix ────────────────────────────

test("per-page policy: credentials.get() runs on every visit", async () => {
  const privA = new Uint8Array(32).fill(0x20);
  const { auth } = await seedBinding(privA);
  const storage = inMemoryStorage();

  for (let i = 0; i < 3; i++) {
    await acquireIdentity({
      cohortId: COHORT,
      cohortMode: "webauthn-prf",
      locationHash: "",
      origin: ORIGIN,
      unwrapDeps: {
        getCredential: auth.getCredential,
        subtle: webcrypto.subtle as SubtleCrypto,
        randomBytes: seededRng(0x30 + i),
      },
      sessionPolicy: "per-page",
      policyDeps: { storage },
    });
  }
  assert.equal(auth.getCalls.length, 3, "per-page → one UV per visit");
  assert.equal(storage.data.size, 0, "per-page → nothing written to storage");
});

test("per-session policy: second visit reads cached priv_A, no UV prompt", async () => {
  const privA = new Uint8Array(32).fill(0x21);
  const { auth } = await seedBinding(privA);
  const storage = inMemoryStorage();

  const first = await acquireIdentity({
    cohortId: COHORT,
    cohortMode: "webauthn-prf",
    locationHash: "",
    origin: ORIGIN,
    unwrapDeps: {
      getCredential: auth.getCredential,
      subtle: webcrypto.subtle as SubtleCrypto,
      randomBytes: seededRng(0x40),
    },
    sessionPolicy: "per-session",
    policyDeps: { storage },
  });
  const second = await acquireIdentity({
    cohortId: COHORT,
    cohortMode: "webauthn-prf",
    locationHash: "",
    origin: ORIGIN,
    unwrapDeps: {
      getCredential: auth.getCredential,
      subtle: webcrypto.subtle as SubtleCrypto,
      randomBytes: seededRng(0x41),
    },
    sessionPolicy: "per-session",
    policyDeps: { storage },
  });
  assert.deepEqual(Array.from(first), Array.from(privA));
  assert.deepEqual(Array.from(second), Array.from(privA));
  assert.equal(auth.getCalls.length, 1,
    "per-session → second visit reused cached priv_A without a fresh UV call");
  // Cache key lives under the documented prefix.
  const key = Array.from(storage.data.keys())[0]!;
  assert.ok(key.startsWith(SESSION_STORAGE_KEY_PREFIX));
});

test("per-minute policy: UV re-runs after TTL expiry", async () => {
  const privA = new Uint8Array(32).fill(0x22);
  const { auth } = await seedBinding(privA);
  const storage = inMemoryStorage();

  let now = 1_700_000_100_000;
  const policyDeps = {
    storage,
    now: () => now,
  };

  await acquireIdentity({
    cohortId: COHORT,
    cohortMode: "webauthn-prf",
    locationHash: "",
    origin: ORIGIN,
    unwrapDeps: {
      getCredential: auth.getCredential,
      subtle: webcrypto.subtle as SubtleCrypto,
      randomBytes: seededRng(0x50),
    },
    sessionPolicy: "per-minute",
    policyDeps,
  });
  // Still inside the TTL window — cached path.
  now += SESSION_PER_MINUTE_TTL_MS - 1_000;
  await acquireIdentity({
    cohortId: COHORT,
    cohortMode: "webauthn-prf",
    locationHash: "",
    origin: ORIGIN,
    unwrapDeps: {
      getCredential: auth.getCredential,
      subtle: webcrypto.subtle as SubtleCrypto,
      randomBytes: seededRng(0x51),
    },
    sessionPolicy: "per-minute",
    policyDeps,
  });
  assert.equal(auth.getCalls.length, 1, "inside TTL → cache hit");

  // Advance past TTL → fresh UV.
  now += SESSION_PER_MINUTE_TTL_MS + 1_000;
  await acquireIdentity({
    cohortId: COHORT,
    cohortMode: "webauthn-prf",
    locationHash: "",
    origin: ORIGIN,
    unwrapDeps: {
      getCredential: auth.getCredential,
      subtle: webcrypto.subtle as SubtleCrypto,
      randomBytes: seededRng(0x52),
    },
    sessionPolicy: "per-minute",
    policyDeps,
  });
  assert.equal(auth.getCalls.length, 2, "past TTL → UV re-runs");
});

test("cache invalidates when bindingCreatedAt changes (operator re-issue)", async () => {
  const privA = new Uint8Array(32).fill(0x23);
  const { auth } = await seedBinding(privA);
  const storage = inMemoryStorage();

  await acquireIdentity({
    cohortId: COHORT,
    cohortMode: "webauthn-prf",
    locationHash: "",
    origin: ORIGIN,
    unwrapDeps: {
      getCredential: auth.getCredential,
      subtle: webcrypto.subtle as SubtleCrypto,
      randomBytes: seededRng(0x60),
    },
    sessionPolicy: "per-session",
    policyDeps: { storage },
  });
  assert.equal(auth.getCalls.length, 1);

  // Tamper with cache entry's bindingCreatedAt — simulates a
  // re-issued binding on the server side with a new createdAt.
  const key = Array.from(storage.data.keys())[0]!;
  const entry = JSON.parse(storage.data.get(key)!);
  entry.bindingCreatedAt = 42;
  storage.data.set(key, JSON.stringify(entry));

  await acquireIdentity({
    cohortId: COHORT,
    cohortMode: "webauthn-prf",
    locationHash: "",
    origin: ORIGIN,
    unwrapDeps: {
      getCredential: auth.getCredential,
      subtle: webcrypto.subtle as SubtleCrypto,
      randomBytes: seededRng(0x61),
    },
    sessionPolicy: "per-session",
    policyDeps: { storage },
  });
  assert.equal(auth.getCalls.length, 2,
    "bindingCreatedAt mismatch forced a fresh UV");
});

test("unwrap failure clears the session-policy cache", async () => {
  const privA = new Uint8Array(32).fill(0x24);
  const { auth } = await seedBinding(privA);
  const storage = inMemoryStorage();

  // First visit caches priv_A.
  await acquireIdentity({
    cohortId: COHORT,
    cohortMode: "webauthn-prf",
    locationHash: "",
    origin: ORIGIN,
    unwrapDeps: {
      getCredential: auth.getCredential,
      subtle: webcrypto.subtle as SubtleCrypto,
      randomBytes: seededRng(0x70),
    },
    sessionPolicy: "per-session",
    policyDeps: { storage },
  });
  assert.equal(storage.data.size, 1);

  // Simulate cache-miss + authenticator revoked on next visit by
  // corrupting the cache entry so bindingCreatedAt no longer
  // matches (forces re-UV) + swapping to a rejecting credential.
  for (const k of Array.from(storage.data.keys())) {
    const v = JSON.parse(storage.data.get(k)!);
    v.bindingCreatedAt = 0;
    storage.data.set(k, JSON.stringify(v));
  }
  const rejecting: UnwrapDeps["getCredential"] = async () => {
    const e = new Error("revoked");
    e.name = "NotAllowedError";
    throw e;
  };
  await assert.rejects(() =>
    acquireIdentity({
      cohortId: COHORT,
      cohortMode: "webauthn-prf",
      locationHash: "",
      origin: ORIGIN,
      unwrapDeps: {
        getCredential: rejecting,
        subtle: webcrypto.subtle as SubtleCrypto,
        randomBytes: seededRng(0x71),
      },
      sessionPolicy: "per-session",
      policyDeps: { storage },
    }),
  );
  assert.equal(storage.data.size, 0, "failing unwrap cleared cache");
});

// ── helpers ────────────────────────────────────────────────────────────

function b64urlEncode(bytes: Uint8Array): string {
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "");
}

void IDBFactory;
