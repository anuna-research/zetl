// Wire-format parser for CON-3404 envelopes.
// Byte-for-byte inverse of `src/cap/sign.rs::build_envelope`:
//
//   ztl-Schema: v4\n
//   ztl-Cohort-Id: <id>\n
//   ztl-Cohort-Mode: delegated-url|webauthn-prf\n
//   ztl-Slug: <slug>\n
//   ztl-Build-Epoch: <rfc3339>\n
//   ztl-Signature: <b64url>\n
//   \n
//   <age v1 ciphertext bytes>

export const ENVELOPE_SCHEMA = "v4";
export const SIGNATURE_LEN = 64;

export type CohortMode = "delegated-url" | "webauthn-prf";

export interface EnvelopeHeader {
  cohortId: string;
  cohortMode: CohortMode;
  slug: string;
  buildEpoch: string;
}

export interface ParsedEnvelope {
  header: EnvelopeHeader;
  signature: Uint8Array;
  ciphertext: Uint8Array;
}

export class EnvelopeParseError extends Error {
  override readonly name = "EnvelopeParseError";
  constructor(
    readonly kind:
      | "missing-separator"
      | "header-encoding"
      | "malformed-header"
      | "missing-header"
      | "schema-mismatch"
      | "unknown-cohort-mode"
      | "signature-base64"
      | "signature-length",
    message: string,
  ) {
    super(message);
  }
}

export function parseEnvelope(bytes: Uint8Array): ParsedEnvelope {
  const sepIdx = findSeparator(bytes);
  if (sepIdx < 0) {
    throw new EnvelopeParseError(
      "missing-separator",
      "envelope is missing the \\n\\n separator between headers and ciphertext",
    );
  }

  let headerText: string;
  try {
    headerText = new TextDecoder("utf-8", { fatal: true }).decode(
      bytes.subarray(0, sepIdx),
    );
  } catch (err) {
    throw new EnvelopeParseError(
      "header-encoding",
      `envelope header is not valid UTF-8: ${(err as Error).message}`,
    );
  }

  let schema: string | undefined;
  let cohortId: string | undefined;
  let cohortModeRaw: string | undefined;
  let slug: string | undefined;
  let buildEpoch: string | undefined;
  let signatureB64: string | undefined;

  for (const line of headerText.split("\n")) {
    if (line.length === 0) continue;
    const delim = line.indexOf(": ");
    if (delim < 0) {
      throw new EnvelopeParseError(
        "malformed-header",
        `envelope header line ${JSON.stringify(line)} is missing a ": " delimiter`,
      );
    }
    const name = line.slice(0, delim);
    const value = line.slice(delim + 2);
    switch (name) {
      case "ztl-Schema":
        schema = value;
        break;
      case "ztl-Cohort-Id":
        cohortId = value;
        break;
      case "ztl-Cohort-Mode":
        cohortModeRaw = value;
        break;
      case "ztl-Slug":
        slug = value;
        break;
      case "ztl-Build-Epoch":
        buildEpoch = value;
        break;
      case "ztl-Signature":
        signatureB64 = value;
        break;
      // Forward-compat: unknown headers are tolerated. A wire break
      // bumps ENVELOPE_SCHEMA and fires schema-mismatch first.
    }
  }

  if (schema === undefined) {
    throw new EnvelopeParseError(
      "missing-header",
      "required header ztl-Schema is missing",
    );
  }
  if (schema !== ENVELOPE_SCHEMA) {
    throw new EnvelopeParseError(
      "schema-mismatch",
      `ztl-Schema is ${JSON.stringify(schema)}; expected ${JSON.stringify(ENVELOPE_SCHEMA)} — shim is too old for this envelope`,
    );
  }
  if (cohortId === undefined) {
    throw new EnvelopeParseError(
      "missing-header",
      "required header ztl-Cohort-Id is missing",
    );
  }
  if (cohortModeRaw === undefined) {
    throw new EnvelopeParseError(
      "missing-header",
      "required header ztl-Cohort-Mode is missing",
    );
  }
  let cohortMode: CohortMode;
  if (cohortModeRaw === "delegated-url" || cohortModeRaw === "webauthn-prf") {
    cohortMode = cohortModeRaw;
  } else {
    throw new EnvelopeParseError(
      "unknown-cohort-mode",
      `ztl-Cohort-Mode is ${JSON.stringify(cohortModeRaw)}; expected "delegated-url" or "webauthn-prf"`,
    );
  }
  if (slug === undefined) {
    throw new EnvelopeParseError(
      "missing-header",
      "required header ztl-Slug is missing",
    );
  }
  if (buildEpoch === undefined) {
    throw new EnvelopeParseError(
      "missing-header",
      "required header ztl-Build-Epoch is missing",
    );
  }
  if (signatureB64 === undefined) {
    throw new EnvelopeParseError(
      "missing-header",
      "required header ztl-Signature is missing",
    );
  }

  let signature: Uint8Array;
  try {
    signature = base64UrlDecode(signatureB64);
  } catch (err) {
    throw new EnvelopeParseError(
      "signature-base64",
      `ztl-Signature is not valid base64url: ${(err as Error).message}`,
    );
  }
  if (signature.length !== SIGNATURE_LEN) {
    throw new EnvelopeParseError(
      "signature-length",
      `ztl-Signature decodes to ${signature.length} bytes; Ed25519 signatures are ${SIGNATURE_LEN}`,
    );
  }

  const ciphertext = bytes.slice(sepIdx + 2);

  return {
    header: { cohortId, cohortMode, slug, buildEpoch },
    signature,
    ciphertext,
  };
}

function findSeparator(bytes: Uint8Array): number {
  for (let i = 0; i + 1 < bytes.length; i++) {
    if (bytes[i] === 0x0a && bytes[i + 1] === 0x0a) return i;
  }
  return -1;
}

export function base64UrlDecode(s: string): Uint8Array {
  // RFC 4648 §5 url-safe alphabet, unpadded. Accept stray padding for
  // robustness but never emit it.
  const normalised = s.replace(/-/g, "+").replace(/_/g, "/");
  const padLen = (4 - (normalised.length % 4)) % 4;
  const padded = normalised + "=".repeat(padLen);
  const bin = atob(padded);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}
