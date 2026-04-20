// Unit tests for the REQ-3425 TOFU-collision UI.
//
// Exercises the pure-core `resolveCollision` (with fake-indexeddb +
// injected prompt stubs), the identity-dispatcher integration path,
// the audit-log round-trip, and the DOM default prompt's wireframe
// shape + focus behaviour + rationale validation.

import { strict as assert } from "node:assert";
import { test, before, beforeEach } from "node:test";
import { webcrypto } from "node:crypto";
import { Buffer } from "node:buffer";

import "fake-indexeddb/auto";
import { Window } from "happy-dom";

import {
  CollisionError,
  COLLISION_BUTTON_KEEP,
  COLLISION_BUTTON_ADD,
  COLLISION_BUTTON_REPLACE,
  COLLISION_DEFAULT_NOTE,
  COLLISION_TITLE,
  MAX_RATIONALE_LEN,
  renderCollisionPrompt,
  resolveCollision,
  validateDecision,
  type CollisionDecision,
  type CollisionPrompt,
} from "../collision.ts";
import { tofuAad } from "../prf_salt.ts";
import {
  appendAuditEntry,
  clearAllBindings,
  readAuditLog,
  readBindingRecord,
  writeBindingRecord,
  type TofuBinding,
} from "../storage.ts";
import { acquireIdentity, IdentityError } from "../identity.ts";
import { IV_LEN, PRF_OUTPUT_LEN, type TofuDeps } from "../tofu.ts";

const ORIGIN = "https://wiki.example.test";
const COHORT = "engineering";

before(() => {
  const win = new Window({ url: `${ORIGIN}/c/onboarding.html` });
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const g = globalThis as any;
  g.window = win;
  g.document = win.document;
  g.HTMLElement = win.HTMLElement;
  g.HTMLButtonElement = win.HTMLButtonElement;
  g.Event = win.Event;
});

beforeEach(async () => {
  await clearAllBindings();
});

function makeBinding(cohortId: string, createdAt: number): TofuBinding {
  return {
    origin: ORIGIN,
    cohortId,
    credentialId: new Uint8Array(16).fill(0x77),
    prfSalt: new Uint8Array(32).fill(0xaa),
    iv: new Uint8Array(IV_LEN).fill(0xbb),
    aad: tofuAad(ORIGIN, cohortId),
    ciphertext: new Uint8Array(48).fill(0xcc),
    createdAt,
  };
}

// ── validateDecision ───────────────────────────────────────────────────

test("validateDecision passes keep + add through unchanged", () => {
  assert.deepEqual(validateDecision({ choice: "keep" }), { choice: "keep" });
  assert.deepEqual(validateDecision({ choice: "add" }), { choice: "add" });
});

test("validateDecision trims whitespace around replace rationale", () => {
  const out = validateDecision({
    choice: "replace",
    rationale: "  lost old device\n",
  });
  assert.deepEqual(out, { choice: "replace", rationale: "lost old device" });
});

test("validateDecision rejects empty replace rationale with rationale-required", () => {
  assert.throws(
    () => validateDecision({ choice: "replace", rationale: "   " }),
    (err: unknown) =>
      err instanceof CollisionError && err.kind === "rationale-required",
  );
});

test("validateDecision rejects overlong replace rationale with rationale-too-long", () => {
  assert.throws(
    () =>
      validateDecision({
        choice: "replace",
        rationale: "x".repeat(MAX_RATIONALE_LEN + 1),
      }),
    (err: unknown) =>
      err instanceof CollisionError && err.kind === "rationale-too-long",
  );
});

// ── resolveCollision — storage + audit ─────────────────────────────────

test("resolveCollision KEEP preserves binding and writes no audit entry", async () => {
  const existing = makeBinding(COHORT, 1_700_000_000_000);
  await writeBindingRecord(existing);

  const prompt: CollisionPrompt = async () => ({ choice: "keep" });
  const outcome = await resolveCollision(existing, prompt, {
    now: () => 1_700_000_100_000,
  });

  assert.equal(outcome.decision.choice, "keep");
  assert.equal(outcome.shouldWrap, false);
  const preserved = await readBindingRecord(COHORT);
  assert.ok(preserved, "binding preserved after KEEP");
  assert.equal(preserved!.createdAt, 1_700_000_000_000);
  const audit = await readAuditLog();
  assert.equal(audit.length, 0, "KEEP does not write audit entries");
});

test("resolveCollision ADD clears binding and writes audit entry with no rationale", async () => {
  const existing = makeBinding(COHORT, 1_700_000_000_000);
  await writeBindingRecord(existing);

  const prompt: CollisionPrompt = async () => ({ choice: "add" });
  const outcome = await resolveCollision(existing, prompt, {
    now: () => 1_700_000_200_000,
  });

  assert.equal(outcome.decision.choice, "add");
  assert.equal(outcome.shouldWrap, true);
  assert.equal(
    await readBindingRecord(COHORT),
    null,
    "binding row cleared so subsequent performTofu writes fresh",
  );
  const audit = await readAuditLog();
  assert.equal(audit.length, 1);
  assert.equal(audit[0]!.choice, "add");
  assert.equal(audit[0]!.cohortId, COHORT);
  assert.equal(audit[0]!.origin, ORIGIN);
  assert.equal(audit[0]!.rationale, undefined);
  assert.equal(audit[0]!.at, 1_700_000_200_000);
  assert.equal(audit[0]!.existingBindingCreatedAt, 1_700_000_000_000);
});

test("resolveCollision REPLACE clears binding and writes audit entry with rationale", async () => {
  const existing = makeBinding(COHORT, 1_700_000_000_000);
  await writeBindingRecord(existing);

  const prompt: CollisionPrompt = async () => ({
    choice: "replace",
    rationale: "old phone stolen",
  });
  const outcome = await resolveCollision(existing, prompt, {
    now: () => 1_700_000_300_000,
  });

  assert.equal(outcome.decision.choice, "replace");
  assert.equal(outcome.shouldWrap, true);
  assert.equal(await readBindingRecord(COHORT), null);
  const audit = await readAuditLog();
  assert.equal(audit.length, 1);
  assert.equal(audit[0]!.choice, "replace");
  assert.equal(audit[0]!.rationale, "old phone stolen");
});

test("resolveCollision REPLACE with empty rationale throws CollisionError before any IDB writes", async () => {
  const existing = makeBinding(COHORT, 1_700_000_000_000);
  await writeBindingRecord(existing);

  const prompt: CollisionPrompt = async () => ({
    choice: "replace",
    rationale: "   ",
  });
  await assert.rejects(
    resolveCollision(existing, prompt),
    (err: unknown) =>
      err instanceof CollisionError && err.kind === "rationale-required",
  );
  const preserved = await readBindingRecord(COHORT);
  assert.ok(preserved, "invalid decision did not clobber the binding");
  const audit = await readAuditLog();
  assert.equal(audit.length, 0, "no audit entry on invalid decision");
});

// ── Identity dispatcher integration ────────────────────────────────────

test("REQ-3425 acquireIdentity with KEEP returns fragment priv_A and preserves binding", async () => {
  const privA = new Uint8Array(32);
  for (let i = 0; i < 32; i++) privA[i] = i;
  const fragment = b64urlEncode(privA);

  const existing = makeBinding(COHORT, 1_700_000_000_000);
  await writeBindingRecord(existing);

  let promptCalls = 0;
  const promptCollision: CollisionPrompt = async (ctx) => {
    promptCalls++;
    assert.equal(ctx.cohortId, COHORT);
    assert.equal(ctx.origin, ORIGIN);
    assert.equal(ctx.existingBindingCreatedAt, 1_700_000_000_000);
    return { choice: "keep" };
  };

  const rejectingCreate: TofuDeps["createCredential"] = async () => {
    throw new Error(
      "navigator.credentials.create must not be invoked on KEEP",
    );
  };

  const priv = await acquireIdentity({
    cohortId: COHORT,
    cohortMode: "delegated-url",
    locationHash: `#k=${fragment}`,
    origin: ORIGIN,
    tofuDeps: {
      createCredential: rejectingCreate,
      subtle: webcrypto.subtle as SubtleCrypto,
      randomBytes: (n) => new Uint8Array(n),
    },
    promptCollision,
  });

  assert.equal(promptCalls, 1, "prompt called once");
  assert.deepEqual(Array.from(priv), Array.from(privA),
    "fragment priv_A returned unchanged");
  const preserved = await readBindingRecord(COHORT);
  assert.ok(preserved, "binding preserved on KEEP");
  assert.equal(preserved!.createdAt, 1_700_000_000_000);
});

test("REQ-3425 acquireIdentity with REPLACE clears old binding and writes new via TOFU", async () => {
  const privA = new Uint8Array(32);
  for (let i = 0; i < 32; i++) privA[i] = 0x40 + i;
  const fragment = b64urlEncode(privA);

  const existing = makeBinding(COHORT, 1_700_000_000_000);
  await writeBindingRecord(existing);

  const promptCollision: CollisionPrompt = async () => ({
    choice: "replace",
    rationale: "old laptop stolen",
  });

  const createCalls: Array<PublicKeyCredentialCreationOptions> = [];
  const prfOutput = new Uint8Array(PRF_OUTPUT_LEN).fill(0xab);
  const rawId = new Uint8Array(16).fill(0x55).buffer;
  const createCredential: TofuDeps["createCredential"] = async (opts) => {
    createCalls.push(opts);
    return {
      rawId,
      getClientExtensionResults: () => ({
        prf: { results: { first: prfOutput.buffer } },
      }),
    } as unknown as PublicKeyCredential;
  };

  let counter = 0x10;
  const randomBytes = (n: number) => {
    const out = new Uint8Array(n);
    for (let i = 0; i < n; i++) out[i] = (counter + i) & 0xff;
    counter = (counter + n) & 0xff;
    return out;
  };

  const priv = await acquireIdentity({
    cohortId: COHORT,
    cohortMode: "delegated-url",
    locationHash: `#k=${fragment}`,
    origin: ORIGIN,
    tofuDeps: {
      createCredential,
      subtle: webcrypto.subtle as SubtleCrypto,
      randomBytes,
      now: () => 1_700_000_500_000,
    },
    promptCollision,
  });

  assert.deepEqual(Array.from(priv), Array.from(privA));
  assert.equal(createCalls.length, 1, "TOFU write ran once on REPLACE");
  const fresh = await readBindingRecord(COHORT);
  assert.ok(fresh, "fresh binding written");
  assert.equal(fresh!.createdAt, 1_700_000_500_000);
  assert.notDeepEqual(
    Array.from(fresh!.credentialId),
    Array.from(existing.credentialId),
    "new credentialId persisted",
  );

  const audit = await readAuditLog();
  assert.equal(audit.length, 1);
  assert.equal(audit[0]!.choice, "replace");
  assert.equal(audit[0]!.rationale, "old laptop stolen");
});

test("REQ-3425 acquireIdentity with ADD behaves like REPLACE at storage layer but audits 'add'", async () => {
  const privA = new Uint8Array(32).fill(0x7a);
  const fragment = b64urlEncode(privA);

  const existing = makeBinding(COHORT, 1_700_000_000_000);
  await writeBindingRecord(existing);

  const promptCollision: CollisionPrompt = async () => ({ choice: "add" });

  const prfOutput = new Uint8Array(PRF_OUTPUT_LEN).fill(0xcd);
  const rawId = new Uint8Array(16).fill(0x44).buffer;
  const createCredential: TofuDeps["createCredential"] = async () =>
    ({
      rawId,
      getClientExtensionResults: () => ({
        prf: { results: { first: prfOutput.buffer } },
      }),
    }) as unknown as PublicKeyCredential;

  await acquireIdentity({
    cohortId: COHORT,
    cohortMode: "delegated-url",
    locationHash: `#k=${fragment}`,
    origin: ORIGIN,
    tofuDeps: {
      createCredential,
      subtle: webcrypto.subtle as SubtleCrypto,
      randomBytes: (n) => new Uint8Array(n).fill(0x33),
      now: () => 1_700_000_600_000,
    },
    promptCollision,
  });

  const fresh = await readBindingRecord(COHORT);
  assert.ok(fresh);
  assert.equal(fresh!.createdAt, 1_700_000_600_000);
  const audit = await readAuditLog();
  assert.equal(audit.length, 1);
  assert.equal(audit[0]!.choice, "add");
});

test("REQ-3425 acquireIdentity without promptCollision preserves pre-REQ-3425 already-bound behaviour", async () => {
  // No prompt → silent idempotent short-circuit in performTofu.
  // This is the back-compat branch; existing unit suites exercise
  // it directly. Verify collisions are not prompted when the
  // operator hasn't wired the callback through.
  const privA = new Uint8Array(32).fill(0x01);
  const fragment = b64urlEncode(privA);

  const existing = makeBinding(COHORT, 1_700_000_000_000);
  await writeBindingRecord(existing);

  const rejectingCreate: TofuDeps["createCredential"] = async () => {
    throw new Error(
      "navigator.credentials.create must not run — binding already present",
    );
  };

  const priv = await acquireIdentity({
    cohortId: COHORT,
    cohortMode: "delegated-url",
    locationHash: `#k=${fragment}`,
    origin: ORIGIN,
    tofuDeps: {
      createCredential: rejectingCreate,
      subtle: webcrypto.subtle as SubtleCrypto,
      randomBytes: (n) => new Uint8Array(n),
    },
  });
  assert.deepEqual(Array.from(priv), Array.from(privA));
  const audit = await readAuditLog();
  assert.equal(audit.length, 0);
});

test("REQ-3425 acquireIdentity skips collision prompt when no existing binding", async () => {
  const privA = new Uint8Array(32).fill(0x02);
  const fragment = b64urlEncode(privA);

  let promptCalls = 0;
  const promptCollision: CollisionPrompt = async () => {
    promptCalls++;
    return { choice: "keep" };
  };

  const prfOutput = new Uint8Array(PRF_OUTPUT_LEN).fill(0x12);
  const createCredential: TofuDeps["createCredential"] = async () =>
    ({
      rawId: new Uint8Array(16).buffer,
      getClientExtensionResults: () => ({
        prf: { results: { first: prfOutput.buffer } },
      }),
    }) as unknown as PublicKeyCredential;

  await acquireIdentity({
    cohortId: "brand-new",
    cohortMode: "delegated-url",
    locationHash: `#k=${fragment}`,
    origin: ORIGIN,
    tofuDeps: {
      createCredential,
      subtle: webcrypto.subtle as SubtleCrypto,
      randomBytes: (n) => new Uint8Array(n).fill(0x22),
      now: () => 1_700_000_700_000,
    },
    promptCollision,
  });

  assert.equal(promptCalls, 0, "no prompt on fresh cohort");
});

test("REQ-3425 collision-failed surfaces as IdentityError when prompt rejects", async () => {
  const existing = makeBinding(COHORT, 1_700_000_000_000);
  await writeBindingRecord(existing);

  const privA = new Uint8Array(32).fill(0x03);
  const fragment = b64urlEncode(privA);

  const promptCollision: CollisionPrompt = async () => {
    throw new Error("reader closed the dialog");
  };

  await assert.rejects(
    acquireIdentity({
      cohortId: COHORT,
      cohortMode: "delegated-url",
      locationHash: `#k=${fragment}`,
      origin: ORIGIN,
      tofuDeps: {
        createCredential: async () => {
          throw new Error("should not reach create()");
        },
        subtle: webcrypto.subtle as SubtleCrypto,
        randomBytes: (n) => new Uint8Array(n),
      },
      promptCollision,
    }),
    (err: unknown) =>
      err instanceof IdentityError && err.kind === "collision-failed",
  );
});

// ── DOM default prompt ─────────────────────────────────────────────────

test("renderCollisionPrompt renders the REQ-3425 wireframe with KEEP default-focused", async () => {
  const win = new Window({ url: `${ORIGIN}/c/onboarding.html` });
  const doc = win.document;
  const main = doc.createElement("main");
  main.setAttribute("data-zetl-capability", "");
  doc.body.appendChild(main);

  const pending = renderCollisionPrompt(
    {
      cohortId: "engineering",
      origin: ORIGIN,
      existingBindingCreatedAt: 1_700_000_000_000,
    },
    doc as unknown as Document,
  );

  const panel = doc.querySelector("[data-zetl-collision]");
  assert.ok(panel, "collision panel mounted");
  assert.ok(
    doc.body.textContent?.includes(COLLISION_TITLE),
    "wireframe title rendered",
  );
  assert.ok(
    doc.body.textContent?.includes("(for cohort: engineering)"),
    "cohort interpolated into body",
  );
  assert.ok(
    doc.body.textContent?.includes(COLLISION_DEFAULT_NOTE),
    "default-keep note rendered",
  );
  const keep = doc.querySelector('[data-zetl-collision-choice="keep"]');
  const add = doc.querySelector('[data-zetl-collision-choice="add"]');
  const replace = doc.querySelector('[data-zetl-collision-choice="replace"]');
  assert.ok(keep && add && replace, "three buttons present");
  assert.equal(keep!.textContent, COLLISION_BUTTON_KEEP);
  assert.equal(add!.textContent, COLLISION_BUTTON_ADD);
  assert.equal(replace!.textContent, COLLISION_BUTTON_REPLACE);
  assert.equal(
    doc.activeElement,
    keep,
    "REQ-3425 acceptance: default focus on KEEP",
  );

  // Click KEEP and verify the promise resolves with the right choice.
  (keep as unknown as HTMLButtonElement).click();
  const decision: CollisionDecision = await pending;
  assert.deepEqual(decision, { choice: "keep" });
});

test("renderCollisionPrompt REPLACE path requires non-empty rationale before resolving", async () => {
  const win = new Window({ url: `${ORIGIN}/c/onboarding.html` });
  const doc = win.document;
  const main = doc.createElement("main");
  main.setAttribute("data-zetl-capability", "");
  doc.body.appendChild(main);

  const pending = renderCollisionPrompt(
    {
      cohortId: COHORT,
      origin: ORIGIN,
      existingBindingCreatedAt: 1_700_000_000_000,
    },
    doc as unknown as Document,
  );

  const replace = doc.querySelector(
    '[data-zetl-collision-choice="replace"]',
  ) as unknown as HTMLButtonElement;
  const rationale = doc.querySelector(
    "[data-zetl-collision-rationale]",
  ) as unknown as HTMLInputElement;
  assert.ok(replace && rationale);

  // First click: no rationale. Must not resolve; error must surface.
  replace.click();
  await new Promise((r) => setTimeout(r, 0));
  const errEl = doc.querySelector("[data-zetl-collision-error]");
  assert.ok(errEl, "error element present");
  assert.equal(errEl!.hasAttribute("hidden"), false);

  // Second click: rationale provided → promise resolves.
  rationale.value = "test rationale";
  replace.click();
  const decision = await pending;
  assert.deepEqual(decision, {
    choice: "replace",
    rationale: "test rationale",
  });
});

test("renderCollisionPrompt ADD resolves without rationale", async () => {
  const win = new Window({ url: `${ORIGIN}/c/onboarding.html` });
  const doc = win.document;
  const main = doc.createElement("main");
  main.setAttribute("data-zetl-capability", "");
  doc.body.appendChild(main);

  const pending = renderCollisionPrompt(
    {
      cohortId: COHORT,
      origin: ORIGIN,
      existingBindingCreatedAt: 1_700_000_000_000,
    },
    doc as unknown as Document,
  );

  const add = doc.querySelector(
    '[data-zetl-collision-choice="add"]',
  ) as unknown as HTMLButtonElement;
  add.click();
  const decision = await pending;
  assert.deepEqual(decision, { choice: "add" });
});

// ── Audit-log round-trip ───────────────────────────────────────────────

test("audit log preserves insertion order across multiple entries", async () => {
  await appendAuditEntry({
    at: 1,
    origin: ORIGIN,
    cohortId: "a",
    choice: "add",
    existingBindingCreatedAt: 100,
  });
  await appendAuditEntry({
    at: 2,
    origin: ORIGIN,
    cohortId: "b",
    choice: "replace",
    rationale: "two",
    existingBindingCreatedAt: 101,
  });
  await appendAuditEntry({
    at: 3,
    origin: ORIGIN,
    cohortId: "c",
    choice: "add",
    existingBindingCreatedAt: 102,
  });

  const log = await readAuditLog();
  assert.equal(log.length, 3);
  assert.deepEqual(log.map((e) => e.cohortId), ["a", "b", "c"]);
  assert.equal(log[1]!.rationale, "two");
});

// ── helpers ────────────────────────────────────────────────────────────

function b64urlEncode(bytes: Uint8Array): string {
  return Buffer.from(bytes).toString("base64url");
}
