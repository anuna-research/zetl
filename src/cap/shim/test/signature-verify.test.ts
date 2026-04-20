// TEST-3427 (browser side): Ed25519 signature verification in the shim.
//
// SPEC-034 REQ-3427 / CON-3411 / NFR-3413. The shim MUST verify the
// Ed25519 signature over the age ciphertext BEFORE any other
// processing — in particular BEFORE `credentials.get/create` and
// BEFORE decryption (CON-3408 pipeline order).
//
// Cases:
//   1. Positive   — valid signature → pipeline proceeds past
//                   Phase.SignatureVerified and renders the plaintext.
//   2. Invalid    — single-bit-flip in the signature → error page,
//                   IdentityAcquired NEVER reached, decryption NEVER
//                   attempted.
//   3. Mismatched — shim bundle holds a different pubkey than the
//                   signer used (simulated SRI-would-have-caught).
//   4. Truncated  — ciphertext lost its tail → signature fails
//                   (envelope header is unchanged so this is a pure
//                   signature-verify rejection, not an envelope parse
//                   rejection).

import { strict as assert } from "node:assert";
import { test, before } from "node:test";

import * as ed from "@noble/ed25519";
import { sha512 } from "@noble/hashes/sha512";
import { Encrypter, generateIdentity, identityToRecipient } from "age-encryption";
import { Window } from "happy-dom";

import { Phase, runPipeline } from "../pipeline.ts";
import { encodeAgeSecretKey } from "../decrypt.ts";

// Sync SHA-512 shim for @noble/ed25519 v2. Also set in the production
// entry point; duplicated here because the test file imports
// `signature.ts` indirectly and we want the hook installed before the
// first call.
ed.etc.sha512Sync = (...messages: Uint8Array[]) => {
  let total = 0;
  for (const m of messages) total += m.length;
  const concat = new Uint8Array(total);
  let off = 0;
  for (const m of messages) {
    concat.set(m, off);
    off += m.length;
  }
  return sha512(concat);
};

before(() => {
  // Install happy-dom globals so `document`/`DOMParser` resolve to a
  // live DOM while the pipeline runs under node:test. `navigator` is a
  // host getter on Node's globalThis and can't be assigned directly —
  // we skip it because the pipeline's two navigator surfaces
  // (serviceWorker purge, navigator.locks) are dependency-injected in
  // the test fixture.
  const win = new Window({ url: "https://cap.example/c/onboarding.html" });
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const g = globalThis as any;
  g.window = win;
  g.document = win.document;
  g.DOMParser = win.DOMParser;
  document.body.innerHTML =
    `<main data-zetl-capability></main>`;
});

// ── Fixture builders ───────────────────────────────────────────────────

interface BuiltFixture {
  vaultPub: Uint8Array;
  vaultPriv: Uint8Array;
  envelopeBytes: Uint8Array;
  ciphertext: Uint8Array;
  fragmentKey: string;
  plaintext: string;
}

function b64urlEncode(bytes: Uint8Array): string {
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "");
}

async function buildFixture(opts: {
  plaintext?: string;
  tamperSignature?: boolean;
  truncateCiphertext?: boolean;
} = {}): Promise<BuiltFixture> {
  const plaintext = opts.plaintext
    ?? "<p>hello from the signed envelope</p>";

  // Age identity → recipient.
  const identity = await generateIdentity();
  const recipient = await identityToRecipient(identity);

  // Encrypt plaintext.
  const enc = new Encrypter();
  enc.addRecipient(recipient);
  const fullCiphertext = await enc.encrypt(plaintext);

  // Generate Ed25519 vault-signing key + sign the *full* ciphertext.
  // The signed envelope then carries an optionally-truncated body, so
  // the signature no longer matches what transits — the reader MUST
  // reject it at the signature check rather than during age decrypt.
  const vaultPriv = ed.utils.randomPrivateKey();
  const vaultPub = ed.getPublicKey(vaultPriv);
  let signature = await ed.signAsync(fullCiphertext, vaultPriv);
  if (opts.tamperSignature) {
    signature = signature.slice();
    signature[0] = signature[0]! ^ 0x01;
  }

  const ciphertext = opts.truncateCiphertext
    ? fullCiphertext.slice(0, fullCiphertext.length - 32)
    : fullCiphertext;

  const sigB64 = b64urlEncode(signature);

  const headerText =
    "Zetl-Schema: v4\n" +
    "Zetl-Cohort-Id: engineering\n" +
    "Zetl-Cohort-Mode: delegated-url\n" +
    "Zetl-Slug: onboarding\n" +
    "Zetl-Build-Epoch: 2026-04-20T10:15:00Z\n" +
    `Zetl-Signature: ${sigB64}\n`;
  const headerBytes = new TextEncoder().encode(headerText);
  const envelope = new Uint8Array(headerBytes.length + 1 + ciphertext.length);
  envelope.set(headerBytes, 0);
  envelope[headerBytes.length] = 0x0a;
  envelope.set(ciphertext, headerBytes.length + 1);

  // Decode AGE-SECRET-KEY-1 back to the 32-byte scalar. We don't have a
  // decoder; simplest route is to generate the scalar first and encode
  // both forms. Swap strategy: regenerate 32 bytes, encode as
  // AGE-SECRET-KEY-1, use that as the identity from the start.
  const priv32 = identityToRaw(identity);
  const fragmentKey = b64urlEncode(priv32);

  return {
    vaultPub,
    vaultPriv,
    envelopeBytes: envelope,
    ciphertext,
    fragmentKey,
    plaintext,
  };
}

// Decode an AGE-SECRET-KEY-1… bech32 string back to the 32 raw bytes.
// Inverse of `encodeAgeSecretKey` in decrypt.ts.
function identityToRaw(identity: string): Uint8Array {
  const CHARSET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";
  const lower = identity.toLowerCase();
  const sepIdx = lower.lastIndexOf("1");
  if (sepIdx < 0) throw new Error("bech32: no separator");
  const data = lower.slice(sepIdx + 1);
  const values: number[] = [];
  for (const ch of data) {
    const v = CHARSET.indexOf(ch);
    if (v < 0) throw new Error(`bech32: invalid char ${ch}`);
    values.push(v);
  }
  // Strip the 6-char checksum.
  const payload5 = values.slice(0, values.length - 6);
  // Convert 5-bit groups → 8-bit bytes.
  const out: number[] = [];
  let acc = 0;
  let bits = 0;
  for (const v of payload5) {
    acc = (acc << 5) | v;
    bits += 5;
    if (bits >= 8) {
      bits -= 8;
      out.push((acc >> bits) & 0xff);
    }
  }
  if (out.length !== 32) throw new Error(`bech32 decoded to ${out.length} bytes`);
  return Uint8Array.from(out);
}

// Round-trip sanity check so a silent bech32 drift doesn't masquerade
// as a pipeline bug.
test("age-secret bech32 round-trip", async () => {
  const identity = await generateIdentity();
  const raw = identityToRaw(identity);
  assert.equal(encodeAgeSecretKey(raw), identity);
});

// ── runPipeline helper ─────────────────────────────────────────────────

async function runWithFixture(
  fx: BuiltFixture,
  overrides: { vaultPub?: Uint8Array; locationHash?: string } = {},
) {
  return runPipeline({
    vaultSigningPubkey: overrides.vaultPub ?? fx.vaultPub,
    fetchEnvelope: async () => fx.envelopeBytes,
    purgeServiceWorkers: async () => {},
    withLock: async (body) => body(),
    locationHash: overrides.locationHash ?? `#k=${fx.fragmentKey}`,
  });
}

// ── TEST-3427 positive ─────────────────────────────────────────────────

test("TEST-3427 positive: valid signature → decrypt + render", async () => {
  const fx = await buildFixture();
  const trace = await runWithFixture(fx);

  assert.equal(trace.errorKind, undefined);
  assert.ok(trace.phases.includes(Phase.SignatureVerified),
    "must reach SignatureVerified on valid sig");
  assert.ok(trace.phases.includes(Phase.Rendered),
    "positive path renders through to Phase.Rendered");

  const host = document.querySelector("main[data-zetl-capability]");
  assert.ok(host, "capability host element present");
  const html = host!.innerHTML;
  assert.match(html, /hello from the signed envelope/);
  // No error sentinel should be stamped on the success path.
  assert.equal(host!.getAttribute("data-zetl-error"), null);
});

// ── TEST-3427 negative (a): invalid signature ──────────────────────────

test("TEST-3427 negative (a): invalid signature → error page, no decrypt", async () => {
  const fx = await buildFixture({ tamperSignature: true });
  const trace = await runWithFixture(fx);

  assert.equal(trace.errorKind, "signature-failed");
  assert.ok(trace.phases.includes(Phase.EnvelopeFetched),
    "envelope parsing still succeeds; the signature check is what rejects");
  assert.ok(!trace.phases.includes(Phase.SignatureVerified),
    "SignatureVerified MUST NOT be recorded on tampered sig");
  assert.ok(!trace.phases.includes(Phase.IdentityAcquired),
    "REQ-3427: MUST NOT call credentials.get/create");
  assert.ok(!trace.phases.includes(Phase.Decrypted),
    "REQ-3427: MUST NOT attempt decryption");
  assert.ok(trace.phases.at(-1) === Phase.Errored,
    "terminal phase is Errored");

  const host = document.querySelector("main[data-zetl-capability]");
  const summary = host!.querySelector("[data-zetl-error-summary]");
  // Copy pinned by SPEC-034 REQ-3427 — byte-stable, no trailing period.
  assert.equal(
    summary!.textContent,
    "This page's signature did not verify — possible tampering; contact your wiki operator",
  );
  assert.equal(host!.getAttribute("data-zetl-error"), "signature-failed");
});

// ── TEST-3427 negative (b): mismatched embedded pubkey ────────────────

test("TEST-3427 negative (b): mismatched embedded pubkey → error page", async () => {
  const fx = await buildFixture();
  // Build the envelope with the real signing key, but simulate the
  // shim bundle holding a *different* vault-signing pubkey (e.g. the
  // CDN swapped the bundle and SRI would have caught it — we mimic
  // the downstream behaviour if SRI were bypassed).
  const otherPriv = ed.utils.randomPrivateKey();
  const otherPub = ed.getPublicKey(otherPriv);
  const trace = await runWithFixture(fx, { vaultPub: otherPub });

  assert.equal(trace.errorKind, "signature-failed");
  assert.ok(!trace.phases.includes(Phase.SignatureVerified));
  assert.ok(!trace.phases.includes(Phase.IdentityAcquired));
  assert.ok(!trace.phases.includes(Phase.Decrypted));
});

// ── TEST-3427 negative (c): truncated ciphertext ──────────────────────

test("TEST-3427 negative (c): truncated ciphertext → error page, no decrypt", async () => {
  const fx = await buildFixture({ truncateCiphertext: true });
  const trace = await runWithFixture(fx);

  assert.equal(trace.errorKind, "signature-failed");
  assert.ok(trace.phases.includes(Phase.EnvelopeFetched),
    "envelope header is intact — parse still succeeds");
  assert.ok(!trace.phases.includes(Phase.SignatureVerified));
  assert.ok(!trace.phases.includes(Phase.IdentityAcquired));
  assert.ok(!trace.phases.includes(Phase.Decrypted));
});
