// Unit tests for the hardened-mode enrolment runtime
// (SPEC-034 REQ-3404 / REQ-3414 / CON-3409). Pairs with the Rust
// integration tests in `tests/cap_enrolment_page_integration.rs`
// — the PRF-salt test here uses the same inputs + expected
// SHA-256 digest, so a drift between the Rust and TS halves would
// break one suite or the other.

import { strict as assert } from "node:assert";
import { createHash } from "node:crypto";
import { test, before } from "node:test";

import { Window } from "happy-dom";

import {
  AGE_RECIPIENT_V1_PREFIX,
  base64UrlEncode,
  computePrfSalt,
  decodeAgeRecipient,
  deriveX25519Scalar,
  encodeQr,
  mount,
  PRF_SALT_PREFIX,
  readCohortParam,
  renderQrCanvas,
} from "../enroll.ts";

before(() => {
  const win = new Window({ url: "https://example.test/enroll.html" });
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const g = globalThis as any;
  g.window = win;
  g.document = win.document;
  g.DOMParser = win.DOMParser;
  g.HTMLElement = win.HTMLElement;
});

test("readCohortParam returns the cohort id when present", () => {
  assert.equal(readCohortParam("?cohort=engineering"), "engineering");
  assert.equal(readCohortParam("?cohort=eng&foo=bar"), "eng");
  assert.equal(readCohortParam("?foo=bar&cohort=ops"), "ops");
});

test("readCohortParam returns null when missing or empty", () => {
  assert.equal(readCohortParam(""), null);
  assert.equal(readCohortParam("?foo=bar"), null);
  assert.equal(readCohortParam("?cohort="), null);
  assert.equal(readCohortParam("?cohort=   "), null);
});

test("computePrfSalt matches REQ-3414 formula byte-for-byte", () => {
  // Mirror of `cap::enrolment::tests::prf_salt_matches_spec_formula`.
  // Compute SHA-256 externally via node:crypto and compare.
  const origin = "https://wiki.example.org";
  const cohort = "engineering";
  const h = createHash("sha256");
  h.update(PRF_SALT_PREFIX);
  h.update(origin);
  h.update("/");
  h.update(cohort);
  const expected = new Uint8Array(h.digest());

  const got = computePrfSalt(origin, cohort);
  assert.equal(got.length, 32);
  assert.deepEqual(Array.from(got), Array.from(expected));
});

test("computePrfSalt cross-cohort unlinkable (REQ-3414)", () => {
  const origin = "https://wiki.example.org";
  const eng = computePrfSalt(origin, "engineering");
  const ops = computePrfSalt(origin, "ops");
  assert.notDeepEqual(Array.from(eng), Array.from(ops));
});

test("computePrfSalt cross-origin unlinkable (REQ-3414)", () => {
  const cohort = "engineering";
  const real = computePrfSalt("https://wiki.example.org", cohort);
  const fake = computePrfSalt("https://attacker.example", cohort);
  assert.notDeepEqual(Array.from(real), Array.from(fake));
});

test("deriveX25519Scalar produces a 32-byte output", () => {
  const prf = new Uint8Array(32).fill(0x42);
  const scalar = deriveX25519Scalar(prf);
  assert.equal(scalar.length, 32);
});

test("deriveX25519Scalar is deterministic", () => {
  const prf = new Uint8Array(32).fill(0x42);
  const a = deriveX25519Scalar(prf);
  const b = deriveX25519Scalar(prf);
  assert.deepEqual(Array.from(a), Array.from(b));
});

test("deriveX25519Scalar differs for different PRF outputs", () => {
  const a = deriveX25519Scalar(new Uint8Array(32).fill(0x01));
  const b = deriveX25519Scalar(new Uint8Array(32).fill(0x02));
  assert.notDeepEqual(Array.from(a), Array.from(b));
});

test("base64UrlEncode is url-safe + unpadded", () => {
  const bytes = new Uint8Array([0xff, 0xff, 0xff]);
  const out = base64UrlEncode(bytes);
  assert.equal(out, "____");
  const zeros = new Uint8Array([0, 0]);
  assert.equal(base64UrlEncode(zeros), "AAA");
});

test("decodeAgeRecipient round-trips a known typage age1 value", async () => {
  // Use typage end-to-end: generate an identity, export its
  // recipient, ensure decodeAgeRecipient returns exactly 32 bytes.
  const { generateIdentity, identityToRecipient } = await import("age-encryption");
  const id = await generateIdentity();
  const age1 = await identityToRecipient(id);
  const raw = decodeAgeRecipient(age1);
  assert.equal(raw.length, 32);
});

test("decodeAgeRecipient rejects non-age HRPs", () => {
  assert.throws(() => decodeAgeRecipient("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"));
});

test("encodeQr produces a square boolean matrix larger than 20×20", () => {
  // Short age-recipient-v1 string fits easily; matrix side ≥ 21.
  const matrix = encodeQr(`${AGE_RECIPIENT_V1_PREFIX}${"A".repeat(43)}`);
  assert.ok(matrix.length >= 21);
  for (const row of matrix) {
    assert.equal(row.length, matrix.length);
    for (const cell of row) assert.equal(typeof cell, "boolean");
  }
});

test("renderQrCanvas emits a canvas sized to cellPx × matrix-side", () => {
  const matrix = encodeQr("zetl");
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const document = (globalThis as any).document;
  const canvas = renderQrCanvas(matrix, 3, document);
  assert.equal(canvas.width, matrix.length * 3);
  assert.equal(canvas.height, matrix.length * 3);
});

function mountPointDocument(url: string): { doc: Document; loc: Location } {
  const win = new Window({ url });
  win.document.body.innerHTML = `<main id="zetl-enroll"></main>`;
  return {
    doc: win.document as unknown as Document,
    loc: win.location as unknown as Location,
  };
}

test("mount with missing cohort renders the missing-cohort diagnostic", async () => {
  const { doc, loc } = mountPointDocument("https://example.test/enroll.html");
  await mount({ document: doc, location: loc });
  const root = doc.getElementById("zetl-enroll");
  assert.ok(root, "mount point must exist");
  assert.equal(root!.getAttribute("data-state"), "missing-cohort");
  assert.match(root!.textContent ?? "", /Missing \?cohort=/);
});

test("mount with no-PRF authenticator renders the no-prf diagnostic", async () => {
  const { doc, loc } = mountPointDocument(
    "https://example.test/enroll.html?cohort=engineering",
  );
  const createFn = async () => ({
    getClientExtensionResults: () => ({}),
  } as unknown as PublicKeyCredential);
  await mount({ document: doc, location: loc, createFn });
  const root = doc.getElementById("zetl-enroll");
  assert.ok(root);
  const state = root!.getAttribute("data-state");
  assert.ok(
    state === "no-prf" || state === "error",
    `unexpected state ${state}`,
  );
});
