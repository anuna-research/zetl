// TEST-3412 / OBS-3412: fragment-required fallback when the reader's
// browser does not advertise WebAuthn PRF.
//
// SPEC-034 REQ-3412 / CON-3408. The shim detects PRF availability via
// `PublicKeyCredential.getClientCapabilities()` (WebAuthn L3) or a
// feature-probe. When unavailable:
//
//   * `acquireIdentity` must receive `idbFactory: null` so TOFU is
//     skipped silently (the reader uses priv_A directly from the
//     URL fragment)
//   * `history.replaceState` MUST NOT run — the fragment must stay
//     in the URL so the reader can bookmark / reload
//   * a `[data-zetl-fallback]` banner is appended to the capability
//     host with a deep-link to `/reader.html#fallback-prf-unavailable`
//   * `performance.mark("zetl:cap:fallback-prf-unavailable")` fires
//     (OBS-3412)
//
// These tests exercise the pipeline under an injected probe stub so
// happy-dom doesn't need a real `PublicKeyCredential` surface; the
// `detectPrfAvailability` pure-core function is tested separately.

import { strict as assert } from "node:assert";
import { test, before, beforeEach } from "node:test";

import * as ed from "@noble/ed25519";
import { sha512 } from "@noble/hashes/sha512";
import { Encrypter, generateIdentity, identityToRecipient } from "age-encryption";
import { Window } from "happy-dom";

import { Phase, runPipeline } from "../pipeline.ts";
import {
  FALLBACK_COPY,
  FALLBACK_DOC_ANCHOR,
  FALLBACK_PERF_MARK,
  detectPrfAvailability,
  emitFallbackMark,
  renderFallbackNotice,
  type PrfAvailability,
} from "../fallback.ts";

// Sync SHA-512 shim for @noble/ed25519 v2.
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

// ── Shared DOM ─────────────────────────────────────────────────────────

before(() => {
  const win = new Window({ url: "https://cap.example/c/onboarding.html#k=deadbeef" });
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const g = globalThis as any;
  g.window = win;
  g.document = win.document;
  g.DOMParser = win.DOMParser;
  // `history` + `location` are not auto-mirrored onto Node's global
  // — wire happy-dom's copies so `render.ts::scrubFragment()` finds
  // a live `history.replaceState` (spied on below). Node exposes
  // `navigator` as a read-only host getter, so we skip mirroring
  // it; `detectPrfAvailability` tests that need a credentials
  // surface patch `globalThis.PublicKeyCredential` directly.
  g.history = win.history;
  g.location = win.location;
});

beforeEach(() => {
  document.body.innerHTML = `<main data-zetl-capability></main>`;
  // Clear any perf marks left over from earlier tests so
  // `getEntriesByName` assertions stay local.
  if (typeof performance !== "undefined" && typeof performance.clearMarks === "function") {
    performance.clearMarks(FALLBACK_PERF_MARK);
  }
});

// ── Envelope fixture ───────────────────────────────────────────────────

interface BuiltFixture {
  vaultPub: Uint8Array;
  envelopeBytes: Uint8Array;
  fragmentKey: string;
  plaintext: string;
}

function b64urlEncode(bytes: Uint8Array): string {
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "");
}

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
  const payload5 = values.slice(0, values.length - 6);
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

async function buildFixture(): Promise<BuiltFixture> {
  const plaintext = "<p>hello from fallback mode</p>";
  const identity = await generateIdentity();
  const recipient = await identityToRecipient(identity);
  const enc = new Encrypter();
  enc.addRecipient(recipient);
  const ciphertext = await enc.encrypt(plaintext);

  const vaultPriv = ed.utils.randomPrivateKey();
  const vaultPub = ed.getPublicKey(vaultPriv);
  const signature = await ed.signAsync(ciphertext, vaultPriv);
  const sigB64 = b64urlEncode(signature);

  const headerText =
    "Zetl-Schema: v4\n" +
    "Zetl-Cohort-Id: engineering\n" +
    "Zetl-Cohort-Mode: delegated-url\n" +
    "Zetl-Slug: onboarding\n" +
    "Zetl-Build-Epoch: 2026-04-21T10:15:00Z\n" +
    `Zetl-Signature: ${sigB64}\n`;
  const headerBytes = new TextEncoder().encode(headerText);
  const envelope = new Uint8Array(headerBytes.length + 1 + ciphertext.length);
  envelope.set(headerBytes, 0);
  envelope[headerBytes.length] = 0x0a;
  envelope.set(ciphertext, headerBytes.length + 1);

  const priv32 = identityToRaw(identity);
  const fragmentKey = b64urlEncode(priv32);
  return { vaultPub, envelopeBytes: envelope, fragmentKey, plaintext };
}

async function runWithProbe(
  fx: BuiltFixture,
  probe: () => Promise<PrfAvailability>,
  extra: { captureMark?: (reason?: string) => void } = {},
) {
  // Point the happy-dom location to an invite URL so the hash is
  // readable. Route replaceState through a spy so we can assert it
  // DOES or DOES NOT run.
  const replaceStateCalls: Array<[unknown, string, string]> = [];
  const origReplace = history.replaceState.bind(history);
  history.replaceState = ((s: unknown, t: string, u: string) => {
    replaceStateCalls.push([s, t, u]);
    return origReplace(s as never, t, u);
  }) as typeof history.replaceState;
  try {
    const trace = await runPipeline({
      vaultSigningPubkey: fx.vaultPub,
      fetchEnvelope: async () => fx.envelopeBytes,
      purgeServiceWorkers: async () => {},
      withLock: async (body) => body(),
      locationHash: `#k=${fx.fragmentKey}`,
      probePrf: probe,
      emitFallbackMark: extra.captureMark,
    });
    return { trace, replaceStateCalls };
  } finally {
    history.replaceState = origReplace;
  }
}

// ── TEST-3412 positive: PRF available → happy path unchanged ──────────

test("TEST-3412 PRF available → no banner, fragment scrubbed, no OBS-3412 mark", async () => {
  const fx = await buildFixture();
  const captures: Array<string | undefined> = [];
  const { trace, replaceStateCalls } = await runWithProbe(
    fx,
    async () => ({ available: true }),
    { captureMark: (r) => captures.push(r) },
  );

  assert.equal(trace.errorKind, undefined);
  assert.equal(trace.fallbackActive, false);
  assert.ok(trace.phases.includes(Phase.Rendered));

  const host = document.querySelector("main[data-zetl-capability]");
  assert.ok(host);
  assert.equal(
    host!.querySelector("[data-zetl-fallback]"),
    null,
    "banner MUST NOT be rendered on the happy path",
  );
  assert.ok(
    replaceStateCalls.length === 1,
    "scrubFragment must run exactly once on the happy path",
  );
  assert.equal(captures.length, 0, "no OBS-3412 mark on the happy path");
});

// ── TEST-3412 fallback: PRF unavailable → banner + no scrub + mark ────

test("TEST-3412 PRF unavailable → banner rendered, fragment NOT scrubbed, OBS-3412 mark fires", async () => {
  const fx = await buildFixture();
  const captures: Array<string | undefined> = [];
  const { trace, replaceStateCalls } = await runWithProbe(
    fx,
    async () => ({ available: false, reason: "caps-prf-false" }),
    { captureMark: (r) => captures.push(r) },
  );

  assert.equal(trace.errorKind, undefined);
  assert.equal(trace.fallbackActive, true);
  assert.equal(trace.fallbackReason, "caps-prf-false");
  assert.ok(trace.phases.includes(Phase.Rendered));

  // Banner is inside the capability host and carries the exact copy.
  const host = document.querySelector("main[data-zetl-capability]");
  const banner = host!.querySelector("[data-zetl-fallback]");
  assert.ok(banner, "fallback banner is present");
  assert.equal(banner!.getAttribute("data-zetl-fallback-reason"), "caps-prf-false");
  assert.equal(banner!.getAttribute("role"), "status");
  const summary = banner!.querySelector("[data-zetl-fallback-summary]");
  assert.equal(summary!.textContent, FALLBACK_COPY);
  const help = banner!.querySelector("[data-zetl-fallback-help] a");
  assert.equal(help!.getAttribute("href"), `/reader.html#${FALLBACK_DOC_ANCHOR}`);
  assert.equal(help!.getAttribute("rel"), "noopener noreferrer");

  // Fragment scrub MUST NOT have run — the reader needs the fragment
  // on every visit.
  assert.equal(
    replaceStateCalls.length,
    0,
    "REQ-3412: replaceState MUST NOT be called when in fragment-required fallback",
  );

  // OBS-3412 observability mark fired with the reason string.
  assert.deepEqual(captures, ["caps-prf-false"]);

  // The decrypted body is still present alongside the banner.
  assert.match(host!.innerHTML, /hello from fallback mode/);
});

// ── TEST-3412 fallback: TOFU skipped even when IDB factory was set ────

test("REQ-3412: pipeline forces TOFU off in fallback mode regardless of idbFactory override", async () => {
  // A probe stub that reports "unavailable". Even if the caller
  // (production `index.ts`) had supplied `idbFactory: indexedDB`, the
  // pipeline must override it to null. We verify via the fake-
  // authenticator observer in tofu.test.ts in the full matrix; here
  // we assert only the end-to-end visible behaviour: no error
  // (TOFU didn't try to bind and throw) and the pipeline reaches
  // IdentityAcquired.
  const fx = await buildFixture();
  const trace = (await runWithProbe(fx, async () => ({
    available: false,
    reason: "no-public-key-credential",
  }))).trace;
  assert.equal(trace.errorKind, undefined);
  assert.ok(trace.phases.includes(Phase.IdentityAcquired),
    "identity must still resolve via the fragment");
});

// ── detectPrfAvailability probe matrix ────────────────────────────────

test("detectPrfAvailability: no PublicKeyCredential → unavailable", async () => {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const g = globalThis as any;
  const saved = g.PublicKeyCredential;
  delete g.PublicKeyCredential;
  try {
    const out = await detectPrfAvailability();
    assert.equal(out.available, false);
    assert.equal(out.reason, "no-public-key-credential");
  } finally {
    if (saved !== undefined) g.PublicKeyCredential = saved;
  }
});

test("detectPrfAvailability: getClientCapabilities returns {prf: false} → unavailable caps-prf-false", async () => {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const g = globalThis as any;
  const saved = g.PublicKeyCredential;
  g.PublicKeyCredential = {
    getClientCapabilities: async () => ({ prf: false }),
  };
  try {
    const out = await detectPrfAvailability();
    assert.equal(out.available, false);
    assert.equal(out.reason, "caps-prf-false");
  } finally {
    if (saved === undefined) delete g.PublicKeyCredential;
    else g.PublicKeyCredential = saved;
  }
});

test("detectPrfAvailability: getClientCapabilities returns {prf: true} → available", async () => {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const g = globalThis as any;
  const saved = g.PublicKeyCredential;
  g.PublicKeyCredential = {
    getClientCapabilities: async () => ({ prf: true }),
  };
  try {
    const out = await detectPrfAvailability();
    assert.equal(out.available, true);
    assert.equal(out.reason, undefined);
  } finally {
    if (saved === undefined) delete g.PublicKeyCredential;
    else g.PublicKeyCredential = saved;
  }
});

test("detectPrfAvailability: L3 method throws → feature-probe fallback runs", async () => {
  // When `getClientCapabilities` throws, the probe falls through to
  // feature-detecting `navigator.credentials.create`. In Node's
  // test environment `navigator` exists as a host getter but has
  // no `credentials` surface, so the probe reports
  // `no-webauthn-create`. The contract we pin here is that the
  // probe never throws the L3 exception through and classifies
  // the failure as a structured reason tag.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const g = globalThis as any;
  const savedPk = g.PublicKeyCredential;
  g.PublicKeyCredential = {
    getClientCapabilities: async () => {
      throw new Error("not yet implemented in this build");
    },
  };
  try {
    const out = await detectPrfAvailability();
    // On a runtime without `navigator.credentials.create` we end
    // up here; on a real browser with create() we'd see
    // `{ available: true }`. Either outcome is valid — what
    // matters is no exception propagates.
    if (!out.available) {
      assert.equal(out.reason, "no-webauthn-create");
    }
  } finally {
    if (savedPk === undefined) delete g.PublicKeyCredential;
    else g.PublicKeyCredential = savedPk;
  }
});

// ── emitFallbackMark actually writes to performance ───────────────────

test("emitFallbackMark writes to performance.mark with detail", () => {
  emitFallbackMark("caps-prf-false");
  const entries = performance.getEntriesByName(FALLBACK_PERF_MARK);
  assert.equal(entries.length, 1);
  // happy-dom's PerformanceMark exposes `detail`; real browsers do
  // too per the Performance Timeline spec.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  assert.equal((entries[0] as any).detail, "caps-prf-false");
});

test("emitFallbackMark without reason still writes the mark name", () => {
  emitFallbackMark();
  const entries = performance.getEntriesByName(FALLBACK_PERF_MARK);
  assert.equal(entries.length, 1);
});

// ── renderFallbackNotice is idempotent ────────────────────────────────

test("renderFallbackNotice replaces rather than stacks on repeat calls", () => {
  const host = document.querySelector("main[data-zetl-capability]")!;
  renderFallbackNotice(host, "caps-prf-false");
  renderFallbackNotice(host, "no-public-key-credential");
  const banners = host.querySelectorAll("[data-zetl-fallback]");
  assert.equal(banners.length, 1, "only one banner at a time");
  assert.equal(
    banners[0]!.getAttribute("data-zetl-fallback-reason"),
    "no-public-key-credential",
    "second call's reason wins",
  );
});
