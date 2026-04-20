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
// The operator supplies the pubkey via `ZETL_CAP_SIGNING_PUBKEY_B64URL`
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

const DEV_PLACEHOLDER_PUBKEY_B64URL = "A".repeat(43);

async function main() {
  const pubkey = process.env.ZETL_CAP_SIGNING_PUBKEY_B64URL
    ?? DEV_PLACEHOLDER_PUBKEY_B64URL;

  if (pubkey.length !== 43) {
    console.error(
      `ZETL_CAP_SIGNING_PUBKEY_B64URL is ${pubkey.length} chars; ` +
        `expected 43 (unpadded base64url of a 32-byte Ed25519 pubkey).`,
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
    },
    logLevel: "warning",
  });

  const bundleBytes = await readFile(BUNDLE_PATH);
  const sriHash = "sha384-" +
    createHash("sha384").update(bundleBytes).digest("base64");
  await writeFile(SRI_PATH, sriHash + "\n", "utf8");

  const manifest = {
    bundle: "shim.js",
    bytes: bundleBytes.length,
    integrity: sriHash,
    signingPubkeyB64url: pubkey,
    placeholderPubkey: pubkey === DEV_PLACEHOLDER_PUBKEY_B64URL,
  };
  await writeFile(MANIFEST_PATH, JSON.stringify(manifest, null, 2) + "\n", "utf8");

  const banner = manifest.placeholderPubkey ? " [DEV PLACEHOLDER PUBKEY]" : "";
  console.error(
    `[zetl] cap shim: bundle=${BUNDLE_PATH} bytes=${bundleBytes.length} integrity=${sriHash}${banner}`,
  );
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
