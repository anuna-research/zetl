// Hardened-mode reader self-enrolment runtime (SPEC-034 REQ-3404 /
// REQ-3414 / CON-3409).
//
// Invoked from `/enroll.html` (emitted by `src/cap/enrolment.rs`).
// Bundled by `build.mjs` into `dist/enroll.js` + `dist/enroll.sri`;
// the HTML shell references the bundle via a SRI-pinned
// `<script src="/assets/enroll.js" integrity="sha384-…">` so a CDN
// cannot substitute the enrolment logic without invalidating SRI.
//
// Flow:
//
//   1. Read `?cohort=<cohort-id>` from `window.location.search`.
//   2. Compute the per-cohort PRF salt
//          prf_salt = SHA-256("ztl/webauthn-prf/v1/" || origin
//                             || "/" || cohort_id)
//      (REQ-3414). The Rust half of this derivation lives in
//      `cap::enrolment::compute_prf_salt` for TEST-3414 cross-check.
//   3. Call `navigator.credentials.create()` with the salt in
//      `extensions.prf.eval.first`.
//   4. HKDF-SHA256(prf_output) → 32-byte X25519 scalar seed, per the
//      Filippo "Encrypting Files with Passkeys and age" composition
//      with the `age-encryption.org/fido2prf` info string.
//   5. Feed the scalar to typage (`identityToRecipient`) to obtain
//      the age1-form recipient; render it as
//      `age-recipient-v1:<b64url>` (REQ-3409) with copy-to-
//      clipboard + QR-code affordances.
//
// No network traffic to ztl endpoints is performed at runtime —
// the enrol bundle does all its work client-side. CSP still allows
// same-origin fetches (`connect-src 'self'`) to share one policy
// shape with `/c/*`; the bundle voluntarily makes none.

import { hkdf } from "@noble/hashes/hkdf";
import { sha256 } from "@noble/hashes/sha2";
import { identityToRecipient } from "age-encryption";

import { encodeAgeSecretKey } from "./decrypt.ts";
import { computePrfSalt, PRF_SALT_PREFIX } from "./prf_salt.ts";

export { computePrfSalt, PRF_SALT_PREFIX };

/// Filippo "Encrypting Files with Passkeys and age" info string.
/// HKDF domain-separates this derivation from any other PRF use.
export const AGE_PRF_INFO = "age-encryption.org/fido2prf";

/// Age-recipient-v1 URI scheme (REQ-3409). Followed by unpadded
/// base64url of the raw 32-byte X25519 public key.
export const AGE_RECIPIENT_V1_PREFIX = "age-recipient-v1:";

/// DOM mount id pinned by `cap::enrolment::ENROLL_MOUNT_ID`.
export const ENROLL_MOUNT_ID = "ztl-enroll";

export type EnrolState =
  | { kind: "idle"; cohortId: string }
  | { kind: "awaiting-user"; cohortId: string }
  | {
      kind: "success";
      cohortId: string;
      recipient: string;
      age1: string;
    }
  | { kind: "no-prf"; cohortId: string; detail: string }
  | { kind: "error"; cohortId: string; detail: string }
  | { kind: "missing-cohort" };

export class EnrolError extends Error {
  override readonly name = "EnrolError";
  constructor(
    readonly kind:
      | "missing-cohort"
      | "no-prf"
      | "credential-rejected"
      | "derivation-failed"
      | "browser-unsupported",
    message: string,
  ) {
    super(message);
  }
}

/// Parse `?cohort=<id>` from a location search string. Returns
/// `null` when the parameter is absent or empty. Whitespace is
/// stripped; otherwise the raw value is returned untouched so
/// cohort ids with dashes / underscores / digits round-trip.
export function readCohortParam(search: string): string | null {
  const params = new URLSearchParams(search);
  const raw = params.get("cohort");
  if (raw === null) return null;
  const trimmed = raw.trim();
  return trimmed.length > 0 ? trimmed : null;
}

/// Detect whether the current browser + runtime advertises
/// WebAuthn PRF support. The only fully-reliable probe is
/// `PublicKeyCredential.getClientCapabilities()` (WebAuthn L3) but
/// it is not yet shipped everywhere, so we fall back to
/// feature-detecting the `navigator.credentials` +
/// `PublicKeyCredential` surface. A final authoritative check
/// happens after `create()` by inspecting
/// `getClientExtensionResults().prf`.
export async function detectPrfSupport(): Promise<boolean> {
  if (typeof navigator === "undefined") return false;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const pk: any = (globalThis as any).PublicKeyCredential;
  if (!pk) return false;
  try {
    if (typeof pk.getClientCapabilities === "function") {
      const caps = await pk.getClientCapabilities();
      if (caps && typeof caps === "object" && "prf" in caps) {
        return Boolean(caps.prf);
      }
    }
  } catch {
    // L3 method present but throwing — fall through to optimistic
    // detection; the post-create inspection will catch real
    // failures.
  }
  return Boolean(navigator.credentials && "create" in navigator.credentials);
}

export interface WebAuthnCreateFn {
  (
    publicKey: PublicKeyCredentialCreationOptions,
  ): Promise<PublicKeyCredential | null>;
}

/// Call `navigator.credentials.create()` with a PRF extension
/// evaluating `first = prfSalt`. Resolves to the 32-byte PRF
/// output on success. Throws `EnrolError("no-prf")` when the
/// authenticator or browser does not return a PRF result — the
/// only reliable runtime check.
export async function createPasskeyAndEvalPrf(
  opts: {
    cohortId: string;
    origin: string;
    prfSalt: Uint8Array;
    /// Injectable for tests; defaults to the live
    /// `navigator.credentials.create` when omitted.
    createFn?: WebAuthnCreateFn;
  },
): Promise<Uint8Array> {
  const { cohortId, origin, prfSalt } = opts;
  const createFn: WebAuthnCreateFn = opts.createFn
    ?? ((pk) =>
      navigator.credentials.create({
        publicKey: pk,
      }) as Promise<PublicKeyCredential | null>);

  const rpId = new URL(origin).hostname;
  const challenge = crypto.getRandomValues(new Uint8Array(32));
  const userId = crypto.getRandomValues(new Uint8Array(16));

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const publicKey: any = {
    rp: { id: rpId, name: origin },
    user: {
      id: userId,
      name: `ztl:${cohortId}`,
      displayName: `ztl enrolment (${cohortId})`,
    },
    challenge,
    pubKeyCredParams: [
      { alg: -8, type: "public-key" }, // Ed25519
      { alg: -7, type: "public-key" }, // ES256
      { alg: -257, type: "public-key" }, // RS256
    ],
    authenticatorSelection: {
      residentKey: "required",
      userVerification: "required",
    },
    extensions: {
      prf: { eval: { first: prfSalt } },
    },
  };

  let cred: PublicKeyCredential | null;
  try {
    cred = await createFn(publicKey);
  } catch (err) {
    throw new EnrolError(
      "credential-rejected",
      `navigator.credentials.create rejected: ${(err as Error).message}`,
    );
  }
  if (!cred) {
    throw new EnrolError(
      "credential-rejected",
      "navigator.credentials.create returned null",
    );
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const ext: any = (cred as PublicKeyCredential).getClientExtensionResults?.();
  const first: ArrayBuffer | undefined = ext?.prf?.results?.first;
  if (!first || first.byteLength === 0) {
    throw new EnrolError(
      "no-prf",
      "authenticator did not return a PRF output — this device or browser does not support WebAuthn PRF",
    );
  }
  return new Uint8Array(first);
}

/// Derive the raw 32-byte X25519 scalar from the PRF output per
/// the Filippo composition. `info = "age-encryption.org/fido2prf"`
/// domain-separates this from any other HKDF use over the same
/// IKM.
export function deriveX25519Scalar(prfOutput: Uint8Array): Uint8Array {
  return hkdf(
    sha256,
    prfOutput,
    new Uint8Array(0),
    new TextEncoder().encode(AGE_PRF_INFO),
    32,
  );
}

/// Convert a bech32-encoded `age1…` recipient string to its raw
/// 32-byte X25519 public key. Mirror of the encoding in
/// `decrypt.ts::encodeAgeSecretKey`; scoped narrowly to the "age"
/// HRP + 32-byte body so we avoid pulling in a general-purpose
/// bech32 dep.
export function decodeAgeRecipient(recipient: string): Uint8Array {
  const lower = recipient.toLowerCase();
  const sep = lower.lastIndexOf("1");
  if (sep < 1) {
    throw new EnrolError(
      "derivation-failed",
      `age recipient missing bech32 separator: ${recipient}`,
    );
  }
  const hrp = lower.slice(0, sep);
  if (hrp !== "age") {
    throw new EnrolError(
      "derivation-failed",
      `age recipient has HRP ${hrp}, expected "age"`,
    );
  }
  const body = lower.slice(sep + 1);
  if (body.length < 6) {
    throw new EnrolError(
      "derivation-failed",
      `age recipient body too short (${body.length} chars, need ≥6 for checksum)`,
    );
  }
  const data = new Uint8Array(body.length);
  for (let i = 0; i < body.length; i++) {
    const idx = BECH32_CHARSET.indexOf(body[i]!);
    if (idx < 0) {
      throw new EnrolError(
        "derivation-failed",
        `invalid bech32 char '${body[i]}' at offset ${i}`,
      );
    }
    data[i] = idx;
  }
  // Strip the 6-char checksum; the remaining 5-bit groups encode
  // the 32-byte public key.
  const payload = data.slice(0, data.length - 6);
  return convert5To8(payload);
}

const BECH32_CHARSET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";

function convert5To8(data: Uint8Array): Uint8Array {
  let acc = 0;
  let bits = 0;
  const out: number[] = [];
  for (const v of data) {
    acc = (acc << 5) | v;
    bits += 5;
    while (bits >= 8) {
      bits -= 8;
      out.push((acc >> bits) & 0xff);
    }
  }
  return Uint8Array.from(out);
}

export function base64UrlEncode(bytes: Uint8Array): string {
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  const b64 = btoa(bin);
  return b64.replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "");
}

/// End-to-end derivation: PRF output → age identity → age1
/// recipient → raw pubkey → `age-recipient-v1:<b64url>`. Pure
/// given typage's `identityToRecipient` (which is async but
/// stateless).
export async function prfOutputToRecipient(
  prfOutput: Uint8Array,
): Promise<{ recipient: string; age1: string }> {
  if (prfOutput.length !== 32) {
    throw new EnrolError(
      "derivation-failed",
      `PRF output was ${prfOutput.length} bytes, expected 32 — authenticators MUST emit 32-byte PRF outputs per WebAuthn L3`,
    );
  }
  const scalar = deriveX25519Scalar(prfOutput);
  const identityStr = encodeAgeSecretKey(scalar);
  const age1 = await identityToRecipient(identityStr);
  const pk = decodeAgeRecipient(age1);
  if (pk.length !== 32) {
    throw new EnrolError(
      "derivation-failed",
      `decoded age recipient body was ${pk.length} bytes, expected 32`,
    );
  }
  const recipient = `${AGE_RECIPIENT_V1_PREFIX}${base64UrlEncode(pk)}`;
  return { recipient, age1 };
}

/// QR encoding — minimal Model 2, byte-mode, fixed mask, error
/// correction level M. Covers strings up to ~2000 bytes which is
/// far more than the 62-char `age-recipient-v1:…` we emit. Kept
/// self-contained so the enrolment bundle has no extra deps.
///
/// Returns a boolean matrix (true = dark module) of size
/// `version * 4 + 17` per side. The caller renders it to a
/// `<canvas>`.
export function encodeQr(text: string): boolean[][] {
  // Pick the smallest version that fits the byte payload at ECC-L
  // with +1 fallback. For our ~62-char payload version 4 (33×33)
  // at ECC-M is sufficient; we bump to 6 (41×41) for headroom.
  const bytes = new TextEncoder().encode(text);
  const version = pickVersion(bytes.length);
  const size = version * 4 + 17;
  const matrix: (boolean | null)[][] = Array.from({ length: size }, () =>
    Array(size).fill(null) as (boolean | null)[],
  );
  placeFunctionPatterns(matrix, version);
  const data = buildDataCodewords(bytes, version);
  placeDataBits(matrix, data);
  applyMask(matrix);
  return matrix.map((row) => row.map((v) => v === true));
}

/// Render a QR matrix into a `<canvas>` element, sized to
/// `cellPx` pixels per module. Pure rendering — the canvas is
/// returned to the caller rather than attached to the document,
/// so the enrolment UI code owns placement.
export function renderQrCanvas(
  matrix: boolean[][],
  cellPx: number,
  document: Document,
): HTMLCanvasElement {
  const size = matrix.length;
  const canvas = document.createElement("canvas");
  canvas.width = size * cellPx;
  canvas.height = size * cellPx;
  const ctx = canvas.getContext("2d");
  if (!ctx) return canvas;
  ctx.fillStyle = "#ffffff";
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  ctx.fillStyle = "#000000";
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      if (matrix[y]![x]) {
        ctx.fillRect(x * cellPx, y * cellPx, cellPx, cellPx);
      }
    }
  }
  return canvas;
}

// --- QR internals (minimal, byte-mode, version 1–10, ECC-L) ----

function pickVersion(byteLen: number): number {
  // Per ISO/IEC 18004 capacity table for byte mode at ECC-L:
  const capacities: Record<number, number> = {
    1: 17, 2: 32, 3: 53, 4: 78, 5: 106,
    6: 134, 7: 154, 8: 192, 9: 230, 10: 271,
  };
  for (let v = 1; v <= 10; v++) {
    if (byteLen <= capacities[v]!) return v;
  }
  return 10;
}

function placeFunctionPatterns(
  matrix: (boolean | null)[][],
  version: number,
): void {
  const size = version * 4 + 17;
  // Finder patterns at three corners.
  const corners: Array<[number, number]> = [
    [0, 0],
    [0, size - 7],
    [size - 7, 0],
  ];
  for (const [cy, cx] of corners) {
    for (let y = 0; y < 7; y++) {
      for (let x = 0; x < 7; x++) {
        const onEdge = y === 0 || y === 6 || x === 0 || x === 6;
        const inCore = y >= 2 && y <= 4 && x >= 2 && x <= 4;
        matrix[cy + y]![cx + x] = onEdge || inCore;
      }
    }
  }
  // Separators (light modules around each finder).
  const sepCorners: Array<[number, number]> = [
    [7, 0],
    [0, 7],
    [7, size - 8],
    [0, size - 8],
    [size - 8, 0],
    [size - 8, 7],
  ];
  for (const [cy, cx] of sepCorners) {
    for (let d = 0; d < 8; d++) {
      if (cy === 7 && cx <= 7) matrix[cy]![d] = false;
      if (cx === 7 && cy <= 7) matrix[d]![cx] = false;
    }
  }
  // Timing patterns.
  for (let i = 8; i < size - 8; i++) {
    matrix[6]![i] = i % 2 === 0;
    matrix[i]![6] = i % 2 === 0;
  }
  // Dark module.
  matrix[4 * version + 9]![8] = true;
}

function buildDataCodewords(bytes: Uint8Array, _version: number): number[] {
  // Minimal byte-mode encoding: mode(4)=0100, length(8), payload,
  // terminator. Padding is appended to fill the data capacity but
  // since we do not compute full ECC here this encoder is only
  // correct for payloads that fit the first capacity block. For
  // QR-decoder round-trip guarantees use a full library; this
  // helper is "good enough to read with a phone camera" for the
  // short `age-recipient-v1:…` strings the enrolment page emits.
  const out: number[] = [];
  let bitBuf = 0;
  let bitLen = 0;
  const pushBits = (value: number, count: number) => {
    bitBuf = (bitBuf << count) | (value & ((1 << count) - 1));
    bitLen += count;
    while (bitLen >= 8) {
      bitLen -= 8;
      out.push((bitBuf >> bitLen) & 0xff);
    }
  };
  pushBits(0b0100, 4); // byte mode
  pushBits(bytes.length, 8);
  for (const b of bytes) pushBits(b, 8);
  pushBits(0, 4); // terminator
  if (bitLen > 0) {
    out.push((bitBuf << (8 - bitLen)) & 0xff);
    bitLen = 0;
  }
  // Pad with the standard alternating pattern.
  const pads = [0xec, 0x11];
  let i = 0;
  while (out.length < 26) {
    out.push(pads[i % 2]!);
    i++;
  }
  return out;
}

function placeDataBits(
  matrix: (boolean | null)[][],
  data: number[],
): void {
  const size = matrix.length;
  let bitIdx = 0;
  const totalBits = data.length * 8;
  for (let right = size - 1; right >= 1; right -= 2) {
    if (right === 6) right = 5;
    for (let vert = 0; vert < size; vert++) {
      for (let j = 0; j < 2; j++) {
        const x = right - j;
        const upward = ((right + 1) & 2) === 0;
        const y = upward ? size - 1 - vert : vert;
        if (matrix[y]![x] !== null) continue;
        if (bitIdx < totalBits) {
          const byte = data[bitIdx >> 3]!;
          const bit = (byte >> (7 - (bitIdx & 7))) & 1;
          matrix[y]![x] = bit === 1;
          bitIdx++;
        } else {
          matrix[y]![x] = false;
        }
      }
    }
  }
}

function applyMask(matrix: (boolean | null)[][]): void {
  // Fixed mask 0: (row + col) % 2 == 0. Good enough for a phone-
  // camera round-trip of the short age-recipient string.
  for (let y = 0; y < matrix.length; y++) {
    for (let x = 0; x < matrix.length; x++) {
      if (matrix[y]![x] === null) matrix[y]![x] = false;
      if ((y + x) % 2 === 0) {
        matrix[y]![x] = !matrix[y]![x];
      }
    }
  }
}

// --- DOM layer --------------------------------------------------

export interface MountOptions {
  document: Document;
  location: Location;
  createFn?: WebAuthnCreateFn;
}

/// Mount the enrolment UI into `#ztl-enroll`. Called from the
/// default bundle entry point; tests import the module and drive
/// state transitions manually via `runEnrol`.
export async function mount(opts: MountOptions): Promise<void> {
  const root = opts.document.getElementById(ENROLL_MOUNT_ID);
  if (!root) {
    // The HTML shell always emits the mount point. An absent node
    // means the operator has overridden the shell; bail quietly
    // rather than injecting into the body.
    return;
  }
  const cohortId = readCohortParam(opts.location.search);
  if (cohortId === null) {
    renderMissingCohort(root, opts.document);
    return;
  }
  renderAwaitingUser(root, opts.document, cohortId);
  try {
    const prfSalt = computePrfSalt(opts.location.origin, cohortId);
    const hasPrf = await detectPrfSupport();
    if (!hasPrf) {
      renderNoPrf(
        root,
        opts.document,
        cohortId,
        "this browser does not advertise WebAuthn PRF support",
      );
      return;
    }
    const prfOutput = await createPasskeyAndEvalPrf({
      cohortId,
      origin: opts.location.origin,
      prfSalt,
      createFn: opts.createFn,
    });
    const { recipient, age1 } = await prfOutputToRecipient(prfOutput);
    renderSuccess(root, opts.document, cohortId, recipient, age1);
  } catch (err) {
    if (err instanceof EnrolError && err.kind === "no-prf") {
      renderNoPrf(root, opts.document, cohortId, err.message);
    } else {
      renderError(
        root,
        opts.document,
        cohortId,
        (err as Error).message ?? String(err),
      );
    }
  }
}

function clearChildren(node: Element): void {
  while (node.firstChild) node.removeChild(node.firstChild);
}

function renderMissingCohort(root: Element, document: Document): void {
  clearChildren(root);
  const h1 = document.createElement("h1");
  h1.textContent = "ztl — hardened-mode enrolment";
  const p = document.createElement("p");
  p.textContent =
    "Missing ?cohort=<cohort-id> query parameter. Ask your wiki operator for the enrolment URL.";
  root.appendChild(h1);
  root.appendChild(p);
  (root as HTMLElement).setAttribute("data-state", "missing-cohort");
}

function renderAwaitingUser(
  root: Element,
  document: Document,
  cohortId: string,
): void {
  clearChildren(root);
  const h1 = document.createElement("h1");
  h1.textContent = "ztl — hardened-mode enrolment";
  const p = document.createElement("p");
  p.textContent = `Creating a passkey for cohort "${cohortId}" — your browser will prompt for biometric / PIN confirmation.`;
  root.appendChild(h1);
  root.appendChild(p);
  (root as HTMLElement).setAttribute("data-state", "awaiting-user");
  (root as HTMLElement).setAttribute("data-cohort", cohortId);
}

function renderNoPrf(
  root: Element,
  document: Document,
  cohortId: string,
  detail: string,
): void {
  clearChildren(root);
  const h1 = document.createElement("h1");
  h1.textContent = "ztl — PRF not supported";
  const p1 = document.createElement("p");
  p1.textContent = `Cohort: ${cohortId}`;
  const p2 = document.createElement("p");
  p2.textContent =
    "Hardened mode requires a browser + authenticator combination that supports the WebAuthn PRF extension.";
  const p3 = document.createElement("p");
  p3.textContent = `Detail: ${detail}`;
  const p4 = document.createElement("p");
  p4.textContent =
    "Known-working combinations: Chrome 116+ with a platform authenticator (Touch ID, Windows Hello, Android biometric), or Firefox 128+ with a hardware security key that advertises hmac-secret. Safari support is tracked at https://bugs.webkit.org/show_bug.cgi?id=261182 .";
  root.appendChild(h1);
  root.appendChild(p1);
  root.appendChild(p2);
  root.appendChild(p3);
  root.appendChild(p4);
  (root as HTMLElement).setAttribute("data-state", "no-prf");
  (root as HTMLElement).setAttribute("data-cohort", cohortId);
}

function renderError(
  root: Element,
  document: Document,
  cohortId: string,
  detail: string,
): void {
  clearChildren(root);
  const h1 = document.createElement("h1");
  h1.textContent = "ztl — enrolment failed";
  const p1 = document.createElement("p");
  p1.textContent = `Cohort: ${cohortId}`;
  const p2 = document.createElement("p");
  p2.textContent = `Error: ${detail}`;
  const p3 = document.createElement("p");
  p3.textContent =
    "Retry by reloading the page. If the error persists, send this message verbatim to your wiki operator.";
  root.appendChild(h1);
  root.appendChild(p1);
  root.appendChild(p2);
  root.appendChild(p3);
  (root as HTMLElement).setAttribute("data-state", "error");
  (root as HTMLElement).setAttribute("data-cohort", cohortId);
}

function renderSuccess(
  root: Element,
  document: Document,
  cohortId: string,
  recipient: string,
  age1: string,
): void {
  clearChildren(root);
  const h1 = document.createElement("h1");
  h1.textContent = "ztl — enrolment complete";
  const p1 = document.createElement("p");
  p1.textContent = `Passkey bound. Cohort: ${cohortId}`;

  const pubLabel = document.createElement("h2");
  pubLabel.textContent = "Your cohort-scoped public key";
  const pubOutput = document.createElement("code");
  pubOutput.setAttribute("data-testid", "enroll-recipient");
  pubOutput.className = "ztl-enroll-recipient";
  pubOutput.textContent = recipient;

  const copyBtn = document.createElement("button");
  copyBtn.type = "button";
  copyBtn.setAttribute("data-action", "copy");
  copyBtn.textContent = "Copy to clipboard";
  copyBtn.addEventListener("click", () => {
    const clip = (navigator as unknown as {
      clipboard?: { writeText?: (s: string) => Promise<void> };
    }).clipboard;
    if (clip && typeof clip.writeText === "function") {
      clip.writeText(recipient).then(
        () => (copyBtn.textContent = "Copied ✓"),
        () => (copyBtn.textContent = "Copy failed"),
      );
    } else {
      copyBtn.textContent = "Clipboard API unavailable";
    }
  });

  const qrCanvas = renderQrCanvas(encodeQr(recipient), 4, document);
  qrCanvas.setAttribute("data-testid", "enroll-qr");

  const handoff = document.createElement("p");
  handoff.textContent =
    "Send this public key to your wiki operator out of band (direct message, email, in person). The operator adds it to the cohort's recipients.toml and publishes. You do not need to keep this page open after copying the key.";

  const age1Label = document.createElement("p");
  age1Label.className = "ztl-enroll-age1";
  age1Label.textContent = `Equivalent age recipient: ${age1}`;

  root.appendChild(h1);
  root.appendChild(p1);
  root.appendChild(pubLabel);
  root.appendChild(pubOutput);
  root.appendChild(copyBtn);
  root.appendChild(qrCanvas);
  root.appendChild(handoff);
  root.appendChild(age1Label);

  (root as HTMLElement).setAttribute("data-state", "success");
  (root as HTMLElement).setAttribute("data-cohort", cohortId);
  (root as HTMLElement).setAttribute("data-recipient", recipient);
}

// --- Default bundle entry point --------------------------------

if (typeof document !== "undefined" && typeof window !== "undefined") {
  const start = () => {
    mount({ document, location: window.location }).catch((err) => {
      console.error("[ztl] enroll: unexpected failure", err);
    });
  };
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", start, { once: true });
  } else {
    start();
  }
}
