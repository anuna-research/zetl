// Unit tests for the TOFU branch of the capability-mode shim
// (SPEC-034 REQ-3405 / REQ-3414 / REQ-3428 / REQ-3429 / CON-3409 /
// ADR-3414).
//
// These exercise the pure-core TOFU orchestrator (tofu.ts) +
// identity-dispatch integration (identity.ts) under a fake IDB and
// injectable `credentials.create` + SubtleCrypto surfaces. The
// Playwright suite in `tests/nfr/` will run the full navigator.locks
// + virtual-authenticator matrix; these tests pin the wire-format
// and state-machine contracts without needing a real browser.

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
  TofuError,
  performTofu,
  type TofuDeps,
} from "../tofu.ts";
import { computePrfSalt, tofuAad } from "../prf_salt.ts";
import {
  clearAllBindings,
  readBindingRecord,
  writeBindingRecord,
  type TofuBinding,
} from "../storage.ts";
import { acquireIdentity, IdentityError } from "../identity.ts";

const ORIGIN = "https://wiki.example.test";
const COHORT = "engineering";

before(() => {
  // Install happy-dom globals; unit tests don't exercise the DOM
  // but a `location` shim keeps `resolveOrigin()` deterministic.
  const win = new Window({ url: `${ORIGIN}/c/onboarding.html` });
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const g = globalThis as any;
  g.window = win;
  g.document = win.document;
});

beforeEach(async () => {
  // Reset the fake IDB between tests so cohort bindings don't leak.
  await clearAllBindings();
});

// Deterministic RNG for byte-stable assertions. Returns an incrementing
// pattern per call — enough for IV + challenge uniqueness within a test.
function seededRng(seed: number): (n: number) => Uint8Array {
  let counter = seed;
  return (n: number) => {
    const out = new Uint8Array(n);
    for (let i = 0; i < n; i++) out[i] = (counter + i) & 0xff;
    counter = (counter + n) & 0xff;
    return out;
  };
}

interface FakeCredential {
  rawId: ArrayBuffer;
  prfFirst: ArrayBuffer;
  getClientExtensionResults: () => { prf: { results: { first: ArrayBuffer } } };
}

function fakeAuthenticator(prfOutputByte = 0xab): {
  createCredential: TofuDeps["createCredential"];
  calls: Array<PublicKeyCredentialCreationOptions>;
  capturedSalt: () => Uint8Array | null;
} {
  const calls: Array<PublicKeyCredentialCreationOptions> = [];
  let capturedPrfSalt: Uint8Array | null = null;
  const prfOutput = new Uint8Array(PRF_OUTPUT_LEN).fill(prfOutputByte);
  const rawId = new Uint8Array(16).fill(0x77).buffer;
  const createCredential = async (
    opts: PublicKeyCredentialCreationOptions,
  ) => {
    calls.push(opts);
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const salt = (opts as any).extensions?.prf?.eval?.first;
    if (salt instanceof Uint8Array) capturedPrfSalt = salt;
    const cred: FakeCredential = {
      rawId,
      prfFirst: prfOutput.buffer,
      getClientExtensionResults: () => ({
        prf: { results: { first: prfOutput.buffer } },
      }),
    };
    return cred as unknown as PublicKeyCredential;
  };
  return {
    createCredential,
    calls,
    capturedSalt: () => capturedPrfSalt,
  };
}

function baseDeps(rng = seededRng(0x10)): TofuDeps {
  return {
    createCredential: fakeAuthenticator().createCredential,
    subtle: webcrypto.subtle as SubtleCrypto,
    randomBytes: rng,
    now: () => 1_700_000_000_000,
  };
}

// ── tofu.ts direct tests ───────────────────────────────────────────────

test("performTofu writes a binding keyed by cohortId", async () => {
  const rng = seededRng(0x42);
  const auth = fakeAuthenticator();
  const deps: TofuDeps = {
    ...baseDeps(rng),
    createCredential: auth.createCredential,
    randomBytes: rng,
  };
  const privA = new Uint8Array(32).fill(0x11);

  const result = await performTofu(
    { cohortId: COHORT, origin: ORIGIN, privA },
    deps,
  );

  assert.equal(result.outcome, "bound");
  assert.equal(auth.calls.length, 1, "exactly one credentials.create call");
  const record = await readBindingRecord(COHORT);
  assert.ok(record, "binding persisted to IDB");
  assert.equal(record!.origin, ORIGIN);
  assert.equal(record!.cohortId, COHORT);
  assert.equal(record!.iv.length, IV_LEN);
  assert.equal(record!.prfSalt.length, 32);
  assert.deepEqual(
    Array.from(record!.prfSalt),
    Array.from(computePrfSalt(ORIGIN, COHORT)),
    "prfSalt matches REQ-3414 formula",
  );
  assert.deepEqual(
    Array.from(record!.aad),
    Array.from(tofuAad(ORIGIN, COHORT)),
    "AAD binds origin + cohortId per CON-3409",
  );
});

test("performTofu supplies a 32-byte random challenge (CON-3409 / BUG-005)", async () => {
  const auth = fakeAuthenticator();
  const rng = seededRng(0xA0);
  await performTofu(
    { cohortId: COHORT, origin: ORIGIN, privA: new Uint8Array(32).fill(0x22) },
    { ...baseDeps(rng), createCredential: auth.createCredential, randomBytes: rng },
  );
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const challenge = (auth.calls[0] as any).challenge;
  assert.ok(challenge instanceof Uint8Array, "challenge is Uint8Array");
  assert.equal(challenge.length, CHALLENGE_LEN, "challenge is 32 bytes");
});

test("performTofu passes the per-cohort PRF salt into extensions.prf.eval.first", async () => {
  const auth = fakeAuthenticator();
  await performTofu(
    { cohortId: COHORT, origin: ORIGIN, privA: new Uint8Array(32).fill(0x33) },
    { ...baseDeps(), createCredential: auth.createCredential },
  );
  const salt = auth.capturedSalt();
  assert.ok(salt, "prf salt captured");
  assert.deepEqual(
    Array.from(salt!),
    Array.from(computePrfSalt(ORIGIN, COHORT)),
    "extensions.prf.eval.first matches REQ-3414 salt",
  );
});

test("performTofu is idempotent — second call skips create() with 'already-bound'", async () => {
  const rng1 = seededRng(0x01);
  const auth1 = fakeAuthenticator();
  await performTofu(
    { cohortId: COHORT, origin: ORIGIN, privA: new Uint8Array(32).fill(0x11) },
    { ...baseDeps(rng1), createCredential: auth1.createCredential, randomBytes: rng1 },
  );
  const auth2 = fakeAuthenticator();
  const result = await performTofu(
    { cohortId: COHORT, origin: ORIGIN, privA: new Uint8Array(32).fill(0x22) },
    { ...baseDeps(), createCredential: auth2.createCredential },
  );
  assert.equal(result.outcome, "already-bound");
  assert.equal(auth2.calls.length, 0, "no second create() call — KEEP per REQ-3425");
});

test("performTofu HKDF-derives K_wrap from the PRF output + 'zetl/tofu-wrap/v1'", async () => {
  // Reproduce the derivation offline and assert the wrapped
  // ciphertext decrypts with the expected K_wrap. Proves the
  // domain-separation info string + empty salt match CON-3409.
  const rng = seededRng(0x55);
  const prfByte = 0xcd;
  const auth = fakeAuthenticator(prfByte);
  const privA = new Uint8Array(32).fill(0x44);
  const deps: TofuDeps = {
    ...baseDeps(rng),
    createCredential: auth.createCredential,
    randomBytes: rng,
  };
  await performTofu(
    { cohortId: COHORT, origin: ORIGIN, privA },
    deps,
  );
  const record = await readBindingRecord(COHORT);
  assert.ok(record);

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
  const key = await subtleAny.importKey(
    "raw",
    expectedKWrap,
    { name: "AES-GCM" },
    false,
    ["decrypt"],
  );
  const recovered = new Uint8Array(
    await subtleAny.decrypt(
      {
        name: "AES-GCM",
        iv: record!.iv,
        additionalData: record!.aad,
        tagLength: 128,
      },
      key,
      record!.ciphertext,
    ),
  );
  assert.deepEqual(Array.from(recovered), Array.from(privA),
    "offline K_wrap decrypts the wrapped priv_A byte-for-byte");
});

test("performTofu rejects a non-32-byte priv_A", async () => {
  await assert.rejects(
    () =>
      performTofu(
        { cohortId: COHORT, origin: ORIGIN, privA: new Uint8Array(16) },
        baseDeps(),
      ),
    (err) => err instanceof TofuError && err.kind === "wrap-failed",
  );
});

test("performTofu surfaces 'no-prf' when the authenticator omits the PRF result", async () => {
  const createCredential: TofuDeps["createCredential"] = async () => {
    return {
      rawId: new Uint8Array(16).buffer,
      getClientExtensionResults: () => ({}),
    } as unknown as PublicKeyCredential;
  };
  await assert.rejects(
    () =>
      performTofu(
        { cohortId: COHORT, origin: ORIGIN, privA: new Uint8Array(32).fill(0x11) },
        { ...baseDeps(), createCredential },
      ),
    (err) => err instanceof TofuError && err.kind === "no-prf",
  );
});

test("performTofu surfaces 'create-rejected' when the authenticator throws", async () => {
  const createCredential: TofuDeps["createCredential"] = async () => {
    throw new Error("NotAllowedError: user cancelled");
  };
  await assert.rejects(
    () =>
      performTofu(
        { cohortId: COHORT, origin: ORIGIN, privA: new Uint8Array(32).fill(0x11) },
        { ...baseDeps(), createCredential },
      ),
    (err) => err instanceof TofuError && err.kind === "create-rejected",
  );
});

test("cross-cohort wraps are unlinkable — distinct salts produce distinct PRF slots (REQ-3414)", async () => {
  // Same origin, two cohorts, same priv_A: the ciphertexts must
  // differ (different IVs too) and the salts captured by the
  // authenticator must differ.
  const privA = new Uint8Array(32).fill(0x66);
  const auth = fakeAuthenticator();
  await performTofu(
    { cohortId: "engineering", origin: ORIGIN, privA },
    { ...baseDeps(seededRng(0x01)), createCredential: auth.createCredential },
  );
  await performTofu(
    { cohortId: "ops", origin: ORIGIN, privA },
    { ...baseDeps(seededRng(0x02)), createCredential: auth.createCredential },
  );
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const eng = (auth.calls[0] as any).extensions.prf.eval.first as Uint8Array;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const ops = (auth.calls[1] as any).extensions.prf.eval.first as Uint8Array;
  assert.notDeepEqual(Array.from(eng), Array.from(ops),
    "different cohorts derive different PRF salts");
});

// ── storage.ts round-trip ──────────────────────────────────────────────

test("storage round-trips every TofuBinding field", async () => {
  const binding: TofuBinding = {
    origin: ORIGIN,
    cohortId: "round-trip",
    credentialId: new Uint8Array([1, 2, 3, 4]),
    prfSalt: new Uint8Array(32).fill(0xaa),
    iv: new Uint8Array(IV_LEN).fill(0xbb),
    aad: tofuAad(ORIGIN, "round-trip"),
    ciphertext: new Uint8Array([9, 8, 7, 6, 5]),
    createdAt: 1_700_000_001_000,
  };
  await writeBindingRecord(binding);
  const got = await readBindingRecord("round-trip");
  assert.ok(got);
  assert.equal(got!.origin, binding.origin);
  assert.deepEqual(Array.from(got!.credentialId), [1, 2, 3, 4]);
  assert.deepEqual(Array.from(got!.prfSalt), Array.from(binding.prfSalt));
  assert.deepEqual(Array.from(got!.iv), Array.from(binding.iv));
  assert.deepEqual(Array.from(got!.aad), Array.from(binding.aad));
  assert.deepEqual(Array.from(got!.ciphertext), [9, 8, 7, 6, 5]);
  assert.equal(got!.createdAt, 1_700_000_001_000);
});

test("clearAllBindings wipes the database", async () => {
  await writeBindingRecord({
    origin: ORIGIN,
    cohortId: "to-be-wiped",
    credentialId: new Uint8Array(16),
    prfSalt: new Uint8Array(32),
    iv: new Uint8Array(IV_LEN),
    aad: tofuAad(ORIGIN, "to-be-wiped"),
    ciphertext: new Uint8Array(48),
    createdAt: 1_700_000_002_000,
  });
  await clearAllBindings();
  assert.equal(await readBindingRecord("to-be-wiped"), null);
});

// ── identity.ts integration — fragment + TOFU ─────────────────────────

test("acquireIdentity returns the fragment scalar AND persists a TOFU binding", async () => {
  // Pick a 32-byte priv_A with a known base64url encoding so we
  // can assemble `#k=…` deterministically.
  const privA = new Uint8Array(32);
  for (let i = 0; i < 32; i++) privA[i] = i;
  const fragment = b64urlEncode(privA);
  assert.equal(fragment.length, 43);

  const auth = fakeAuthenticator();
  const priv = await acquireIdentity({
    cohortId: COHORT,
    cohortMode: "delegated-url",
    locationHash: `#k=${fragment}`,
    origin: ORIGIN,
    tofuDeps: {
      createCredential: auth.createCredential,
      subtle: webcrypto.subtle as SubtleCrypto,
      randomBytes: seededRng(0x33),
    },
  });
  assert.deepEqual(Array.from(priv), Array.from(privA),
    "acquireIdentity returns the fragment scalar untouched");
  const record = await readBindingRecord(COHORT);
  assert.ok(record, "TOFU binding created on first visit");
  assert.equal(record!.cohortId, COHORT);
});

test("acquireIdentity falls through to need-invite when no fragment + no binding", async () => {
  await assert.rejects(
    () =>
      acquireIdentity({
        cohortId: "no-record",
        cohortMode: "delegated-url",
        locationHash: "",
        origin: ORIGIN,
      }),
    (err) => err instanceof IdentityError && err.kind === "need-invite",
  );
});

test("acquireIdentity in webauthn-prf mode falls through to need-invite when no binding exists", async () => {
  // With task-cap-shim-unwrap landed, hardened mode no longer
  // throws `mode-not-supported` — it goes straight to the unwrap
  // branch. A cohort without a persisted binding surfaces
  // `need-invite` so the reader is directed to request a fresh
  // invite URL.
  await assert.rejects(
    () =>
      acquireIdentity({
        cohortId: COHORT,
        cohortMode: "webauthn-prf",
        locationHash: "",
        origin: ORIGIN,
      }),
    (err) => err instanceof IdentityError && err.kind === "need-invite",
  );
});

test("acquireIdentity skips TOFU silently when no IDB factory is available (REQ-3412 fallback)", async () => {
  const privA = new Uint8Array(32);
  for (let i = 0; i < 32; i++) privA[i] = i;
  const fragment = b64urlEncode(privA);
  const auth = fakeAuthenticator();
  const priv = await acquireIdentity({
    cohortId: COHORT,
    cohortMode: "delegated-url",
    locationHash: `#k=${fragment}`,
    origin: ORIGIN,
    tofuDeps: {
      createCredential: auth.createCredential,
      subtle: webcrypto.subtle as SubtleCrypto,
      randomBytes: seededRng(0x99),
    },
    idbFactory: null,
  });
  assert.deepEqual(Array.from(priv), Array.from(privA));
  assert.equal(auth.calls.length, 0,
    "TOFU did not run — fragment-only mode per REQ-3412");
});

// ── helpers ────────────────────────────────────────────────────────────

function b64urlEncode(bytes: Uint8Array): string {
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "");
}

// Suppress lint for unused imports when the file is parsed but not
// all surfaces are exercised.
void IDBFactory;
