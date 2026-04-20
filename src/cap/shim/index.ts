// Capability-mode browser shim — CON-3408 entry point.
//
// The bundled build (esbuild) replaces `__VAULT_SIGNING_PUBKEY_B64URL__`
// with the operator's Ed25519 vault-signing public key encoded as
// base64url. The replacement is string-literal, so the SHA-384 SRI hash
// over the emitted `shim.js` covers the pubkey: a CDN that tampers with
// the pubkey invalidates SRI (REQ-3427).

import { base64UrlDecode } from "./envelope.ts";
import { renderError } from "./errors.ts";
import {
  defaultFetchEnvelope,
  defaultPurgeServiceWorkers,
  defaultWithLock,
  LockUnavailableError,
  runPipeline,
  type PipelineDeps,
  type PipelineTrace,
} from "./pipeline.ts";

// `declare const` + esbuild `define` swaps this for the literal b64url
// pubkey string at build time. Keeping it a plain const means the
// emitted bundle has a single exact byte range to audit.
declare const __VAULT_SIGNING_PUBKEY_B64URL__: string;

export interface ShimOptions {
  /// Override dependency injection points — used by the Playwright
  /// pipeline-order tests.
  deps?: Partial<PipelineDeps>;
}

export async function renderCurrentPage(
  opts: ShimOptions = {},
): Promise<PipelineTrace> {
  let vaultSigningPubkey: Uint8Array;
  try {
    vaultSigningPubkey = base64UrlDecode(__VAULT_SIGNING_PUBKEY_B64URL__);
  } catch (err) {
    renderError(
      "internal",
      `embedded vault-signing pubkey is not valid base64url: ${(err as Error).message}`,
    );
    return { phases: [], errorKind: "internal", errorDetail: (err as Error).message };
  }
  if (vaultSigningPubkey.length !== 32) {
    renderError(
      "internal",
      `embedded vault-signing pubkey is ${vaultSigningPubkey.length} bytes; expected 32`,
    );
    return { phases: [], errorKind: "internal" };
  }

  const deps: PipelineDeps = {
    vaultSigningPubkey,
    fetchEnvelope: opts.deps?.fetchEnvelope ?? defaultFetchEnvelope,
    purgeServiceWorkers:
      opts.deps?.purgeServiceWorkers ?? defaultPurgeServiceWorkers,
    withLock: opts.deps?.withLock ?? defaultWithLock,
    locationHash: opts.deps?.locationHash ?? location.hash,
  };

  try {
    return await runPipeline(deps);
  } catch (err) {
    if (err instanceof LockUnavailableError) {
      renderError("lock-unavailable", err.message);
      return { phases: [], errorKind: "lock-unavailable", errorDetail: err.message };
    }
    throw err;
  }
}

/// Mirror of CON-3408's `forgetBinding()` surface. v1 core ships a stub
/// that wipes any future IndexedDB entries the TOFU task creates; until
/// that task lands there are none, so this is a no-op.
export async function forgetBinding(): Promise<void> {
  // `task-cap-shim-tofu` adds the concrete IDB object store and updates
  // this to `idb.deleteDatabase(...)`.
}

declare global {
  interface Window {
    __zetlCapShim?: {
      renderCurrentPage: typeof renderCurrentPage;
      forgetBinding: typeof forgetBinding;
    };
  }
}

if (typeof window !== "undefined") {
  window.__zetlCapShim = { renderCurrentPage, forgetBinding };
  const start = () => {
    void renderCurrentPage();
  };
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", start, { once: true });
  } else {
    start();
  }
}
