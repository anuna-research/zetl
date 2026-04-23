#!/usr/bin/env node
// Bundler driver for the capability-mode shim.
//
// Emits `dist/shim.js` (esbuild IIFE, minified) and `dist/shim.sri`
// (SHA-384 SRI hash over the bundle bytes). REQ-3421: the HTML shell
// references the bundle as
//
//     <script src="/assets/shim.js"
//             integrity="sha384-<hash>"
//             crossorigin="anonymous">
//
// so any tamper — including to the embedded vault-signing pubkey —
// fails SRI before the shim runs (REQ-3427).
//
// The operator supplies the pubkey via `ztl_CAP_SIGNING_PUBKEY_B64URL`
// (unpadded base64url of the 32-byte Ed25519 public key). In local
// dev without a key set, we substitute a 32-zero-byte placeholder so
// the bundle still builds; the Rust build driver
// (`src/cap/build/driver.rs`) re-bundles with the real key before
// deploy.

import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(__dirname);
const OUT_DIR = join(ROOT, "dist");
const BUNDLE_PATH = join(OUT_DIR, "shim.js");
const SRI_PATH = join(OUT_DIR, "shim.sri");
const MANIFEST_PATH = join(OUT_DIR, "shim.manifest.json");
// SPEC-034 REQ-3404 hardened-mode reader self-enrolment bundle.
// Bundled from the same tsc/esbuild pipeline as the main shim so
// the two share the `age-encryption` + `@noble/*` vendoring.
const ENROLL_BUNDLE_PATH = join(OUT_DIR, "enroll.js");
const ENROLL_SRI_PATH = join(OUT_DIR, "enroll.sri");

const DEV_PLACEHOLDER_PUBKEY_B64URL = "A".repeat(43);

async function main() {
  const pubkey = process.env.ztl_CAP_SIGNING_PUBKEY_B64URL
    ?? DEV_PLACEHOLDER_PUBKEY_B64URL;

  if (pubkey.length !== 43) {
    console.error(
      `ztl_CAP_SIGNING_PUBKEY_B64URL is ${pubkey.length} chars; ` +
        `expected 43 (unpadded base64url of a 32-byte Ed25519 pubkey).`,
    );
    process.exit(2);
  }

  // REQ-3430 opt-in — mirrors `[access.split_key] second_factor` in
  // the operator's `.ztl/config.toml`. When unset (or empty) the
  // shim bundle refuses `#k1=` URLs with `mode-not-supported`. When
  // set, the shim wires a `window.prompt` (for `spoken-phrase`) or
  // a camera-scanner hook (for `qr`) into the identity branch.
  const splitKeyFactor = process.env.ztl_CAP_SPLIT_KEY_SECOND_FACTOR ?? "";
  if (
    splitKeyFactor !== ""
    && splitKeyFactor !== "spoken-phrase"
    && splitKeyFactor !== "qr"
  ) {
    console.error(
      `ztl_CAP_SPLIT_KEY_SECOND_FACTOR ${JSON.stringify(splitKeyFactor)} is not one of ""/"spoken-phrase"/"qr"`,
    );
    process.exit(2);
  }

  await mkdir(OUT_DIR, { recursive: true });

  await build({
    entryPoints: [join(ROOT, "index.ts")],
    outfile: BUNDLE_PATH,
    bundle: true,
    format: "iife",
    target: "es2022",
    platform: "browser",
    minify: true,
    sourcemap: false,
    legalComments: "none",
    define: {
      __VAULT_SIGNING_PUBKEY_B64URL__: JSON.stringify(pubkey),
      __SPLIT_KEY_SECOND_FACTOR__: JSON.stringify(splitKeyFactor),
    },
    logLevel: "warning",
  });

  const bundleBytes = await readFile(BUNDLE_PATH);
  const sriHash = "sha384-" +
    createHash("sha384").update(bundleBytes).digest("base64");
  await writeFile(SRI_PATH, sriHash + "\n", "utf8");

  // SPEC-034 REQ-3404 / REQ-3414: bundle the hardened-mode reader
  // self-enrolment entry point into `dist/enroll.js` + emit its
  // SHA-384 SRI hash. The bundle is self-contained (no network to
  // ztl endpoints at runtime) and is referenced from
  // `/enroll.html` (emitted by `src/cap/enrolment.rs`) with the
  // SRI token below.
  await build({
    entryPoints: [join(ROOT, "enroll.ts")],
    outfile: ENROLL_BUNDLE_PATH,
    bundle: true,
    format: "iife",
    target: "es2022",
    platform: "browser",
    minify: true,
    sourcemap: false,
    legalComments: "none",
    logLevel: "warning",
  });
  const enrollBytes = await readFile(ENROLL_BUNDLE_PATH);
  const enrollSri = "sha384-" +
    createHash("sha384").update(enrollBytes).digest("base64");
  await writeFile(ENROLL_SRI_PATH, enrollSri + "\n", "utf8");

  const manifest = {
    bundle: "shim.js",
    bytes: bundleBytes.length,
    integrity: sriHash,
    signingPubkeyB64url: pubkey,
    placeholderPubkey: pubkey === DEV_PLACEHOLDER_PUBKEY_B64URL,
    splitKeySecondFactor: splitKeyFactor === "" ? null : splitKeyFactor,
    enroll: {
      bundle: "enroll.js",
      bytes: enrollBytes.length,
      integrity: enrollSri,
    },
  };
  await writeFile(MANIFEST_PATH, JSON.stringify(manifest, null, 2) + "\n", "utf8");

  const banner = manifest.placeholderPubkey ? " [DEV PLACEHOLDER PUBKEY]" : "";
  console.error(
    `[ztl] cap shim: bundle=${BUNDLE_PATH} bytes=${bundleBytes.length} integrity=${sriHash}${banner}`,
  );
  console.error(
    `[ztl] cap enroll: bundle=${ENROLL_BUNDLE_PATH} bytes=${enrollBytes.length} integrity=${enrollSri}`,
  );
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
