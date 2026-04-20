// Unit tests for the REQ-3430 split-key path of the identity module.
// These exercise `parseFragment` + the `#k1=` branch of
// `acquireIdentity`: XOR reconstruction, cancellation handling,
// malformed half2 diagnostics, and the "shim compiled without
// split-key support" guard.

import { strict as assert } from "node:assert";
import { test } from "node:test";

import { Buffer } from "node:buffer";

import {
  acquireIdentity,
  IdentityError,
  PRIV_A_RAW_LEN,
  parseFragment,
  type SplitKeyPrompt,
} from "../identity.ts";

/// Local base64url encoder. The shim's runtime dependency graph only
/// needs `decode`; the test suite needs `encode` to mint synthetic
/// fragment payloads without pulling a bigger dep in.
function base64UrlEncode(bytes: Uint8Array): string {
  return Buffer.from(bytes).toString("base64url");
}

const ORIGIN = "https://wiki.example.test";
const COHORT = "engineering";

function makePrivA(seed: number): Uint8Array {
  const out = new Uint8Array(PRIV_A_RAW_LEN);
  for (let i = 0; i < PRIV_A_RAW_LEN; i++) out[i] = (seed + i) & 0xff;
  return out;
}

function xor(a: Uint8Array, b: Uint8Array): Uint8Array {
  const out = new Uint8Array(PRIV_A_RAW_LEN);
  for (let i = 0; i < PRIV_A_RAW_LEN; i++) out[i] = a[i]! ^ b[i]!;
  return out;
}

test("parseFragment distinguishes #k= and #k1= prefixes", () => {
  const priv = makePrivA(1);
  const b64 = base64UrlEncode(priv);
  const delegated = parseFragment(`#k=${b64}`);
  if (delegated === null) throw new Error("delegated fragment should parse");
  assert.equal(delegated.kind, "delegated");
  assert.deepEqual(delegated.raw, priv);

  const split = parseFragment(`#k1=${b64}`);
  if (split === null) throw new Error("split-key fragment should parse");
  assert.equal(split.kind, "split-key-half1");
  assert.deepEqual(split.raw, priv);
});

test("parseFragment returns null on empty fragment", () => {
  assert.equal(parseFragment(""), null);
  assert.equal(parseFragment("#"), null);
});

test("parseFragment rejects unknown prefixes with IdentityError", () => {
  assert.throws(
    () => parseFragment("#key=abc"),
    (err: unknown) =>
      err instanceof IdentityError && err.kind === "malformed-fragment",
  );
});

test("REQ-3430 acquireIdentity reconstructs priv_A from half1 XOR half2", async () => {
  // Author-side: pick a priv_A, split it via XOR into (half1, half2).
  const priv = makePrivA(7);
  const half2 = makePrivA(200);
  const half1 = xor(priv, half2);

  const prompt: SplitKeyPrompt = async (factor) => {
    assert.equal(factor, "spoken-phrase");
    return base64UrlEncode(half2);
  };

  const result = await acquireIdentity({
    cohortId: COHORT,
    cohortMode: "delegated-url",
    locationHash: `#k1=${base64UrlEncode(half1)}`,
    origin: ORIGIN,
    // Disable TOFU so we don't need the WebAuthn surface.
    idbFactory: null,
    promptHalf2: prompt,
    splitKeySecondFactor: "spoken-phrase",
  });
  assert.deepEqual(result, priv);
});

test("REQ-3430 acquireIdentity trims whitespace around typed half2", async () => {
  const priv = makePrivA(12);
  const half2 = makePrivA(99);
  const half1 = xor(priv, half2);

  const prompt: SplitKeyPrompt = async () =>
    `  ${base64UrlEncode(half2)}\n`;

  const result = await acquireIdentity({
    cohortId: COHORT,
    cohortMode: "delegated-url",
    locationHash: `#k1=${base64UrlEncode(half1)}`,
    origin: ORIGIN,
    idbFactory: null,
    promptHalf2: prompt,
    splitKeySecondFactor: "spoken-phrase",
  });
  assert.deepEqual(result, priv);
});

test("acquireIdentity surfaces mode-not-supported when prompt omitted on #k1= URL", async () => {
  const half1 = makePrivA(33);
  await assert.rejects(
    acquireIdentity({
      cohortId: COHORT,
      cohortMode: "delegated-url",
      locationHash: `#k1=${base64UrlEncode(half1)}`,
      origin: ORIGIN,
      idbFactory: null,
      // No promptHalf2 / splitKeySecondFactor — simulates a shim
      // bundle built without REQ-3430 opt-in.
    }),
    (err: unknown) =>
      err instanceof IdentityError && err.kind === "mode-not-supported",
  );
});

test("acquireIdentity reclassifies cancelled prompt as split-key-cancelled", async () => {
  const half1 = makePrivA(44);
  const prompt: SplitKeyPrompt = async () => {
    throw new Error("reader cancelled");
  };
  await assert.rejects(
    acquireIdentity({
      cohortId: COHORT,
      cohortMode: "delegated-url",
      locationHash: `#k1=${base64UrlEncode(half1)}`,
      origin: ORIGIN,
      idbFactory: null,
      promptHalf2: prompt,
      splitKeySecondFactor: "spoken-phrase",
    }),
    (err: unknown) =>
      err instanceof IdentityError && err.kind === "split-key-cancelled",
  );
});

test("acquireIdentity reclassifies malformed half2 as split-key-invalid-half2", async () => {
  const half1 = makePrivA(55);
  // Too short — 10 chars < 43-char base64url requirement.
  const prompt: SplitKeyPrompt = async () => "AAAAAAAAAA";
  await assert.rejects(
    acquireIdentity({
      cohortId: COHORT,
      cohortMode: "delegated-url",
      locationHash: `#k1=${base64UrlEncode(half1)}`,
      origin: ORIGIN,
      idbFactory: null,
      promptHalf2: prompt,
      splitKeySecondFactor: "spoken-phrase",
    }),
    (err: unknown) =>
      err instanceof IdentityError &&
      err.kind === "split-key-invalid-half2",
  );
});

test("acquireIdentity rejects empty-string prompt return as split-key-cancelled", async () => {
  const half1 = makePrivA(66);
  const prompt: SplitKeyPrompt = async () => "   \n";
  await assert.rejects(
    acquireIdentity({
      cohortId: COHORT,
      cohortMode: "delegated-url",
      locationHash: `#k1=${base64UrlEncode(half1)}`,
      origin: ORIGIN,
      idbFactory: null,
      promptHalf2: prompt,
      splitKeySecondFactor: "spoken-phrase",
    }),
    (err: unknown) =>
      err instanceof IdentityError && err.kind === "split-key-cancelled",
  );
});
